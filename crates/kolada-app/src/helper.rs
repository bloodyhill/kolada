//! The writing model's own process, `kolada-ai`, beside the app, as on
//! Windows (`kolada-win` `assist.rs`). It starts a moment after the window
//! shows, at low priority, and ends with Kolada (or when writing help is
//! turned off). Requests go down its standard input as frames (see
//! `kolada_assist::wire`) from a writer thread; a reader thread takes its
//! replies off its standard output and wakes the window. Without its files
//! it stays off, without a word. Its model is the one the person chose,
//! downloaded into the app's data (`crate::download`).

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use kolada_assist::wire::{self, Reply, Request};

use crate::services::Priority;

/// The helper's name beside the app's binary, and Notsy's from before.
pub const HELPER: &str = "kolada-ai";
const HELPER_BEFORE: &str = "notsy-ai";

/// The helper's exit code when its model would not load: starting it again
/// would not help.
pub const NO_MODEL: u32 = 2;

pub enum Event {
    Reply(Reply),
    /// The helper ended, with its exit code; None if it never started (its
    /// files are missing, or it would not start).
    Ended(Option<u32>),
}

pub struct Helper {
    frames: Sender<Vec<u8>>,
    pub events: Receiver<Event>,
    /// The process, ended when the helper goes.
    child: Arc<Mutex<Option<Child>>>,
}

/// The helper and its model, if both are there; `model` is the id of the
/// model the person chose (`kolada_assist::models`). A test instance's own
/// download first, then the real app data's: a model there is shared by
/// every Kolada.
fn files(model: Option<&str>) -> Option<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let own = kolada_core::testing().then(kolada_core::app_data_dir).flatten();
    [own, kolada_core::real_data_dir()].into_iter().flatten().find_map(|data| files_in(dir, Some(&data), model)).or_else(|| files_in(dir, None, model))
}

/// The writing helper is beside the app's binary (only then is a model
/// worth offering).
pub fn helper_here() -> bool {
    std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).is_some_and(|dir| [HELPER, HELPER_BEFORE].iter().any(|n| dir.join(n).is_file()))
}

/// Where a chosen model downloads: the real app data's `models`, shared by
/// every Kolada; a test instance's own, so that a test never changes the
/// person's models.
pub fn models_dir() -> Option<PathBuf> {
    let data = if kolada_core::testing() { kolada_core::app_data_dir() } else { kolada_core::real_data_dir() };
    Some(data?.join("models"))
}

/// `files` for the binary in `dir` and the app's data in `data`, as on
/// Windows (`kolada-win` `assist.rs` `files_in`). A model trained for
/// Kolada staged with a test build comes first (in `<dir>/models`, or the
/// bundle's `Resources/models`), the chosen one if it is there; else the
/// chosen model in `<data>/models`, once all of it has downloaded. No
/// model, no helper.
fn files_in(dir: &Path, data: Option<&Path>, model: Option<&str>) -> Option<(PathBuf, PathBuf)> {
    let helper = [HELPER, HELPER_BEFORE].into_iter().map(|n| dir.join(n)).find(|h| h.is_file())?;
    let chosen = model.and_then(kolada_assist::models::by_id);
    let staged = [Some(dir.join("models")), dir.parent().map(|c| c.join("Resources").join("models"))];
    if let Some(found) = staged.iter().flatten().find_map(|d| beside(d, chosen.map(|m| m.file))) {
        return Some((helper, found));
    }
    let m = chosen?;
    let shared = data?.join("models").join(m.file);
    let whole = std::fs::metadata(&shared).is_ok_and(|f| f.is_file() && f.len() == m.bytes);
    whole.then_some((helper, shared))
}

/// A model trained for Kolada in `models`: `chosen` if it is one of them,
/// else the first by name.
fn beside(models: &Path, chosen: Option<&str>) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(models)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            kolada_assist::models::trained(name) && name.to_ascii_lowercase().ends_with(".gguf") && p.is_file()
        })
        .collect();
    found.sort();
    let pick = found.iter().position(|p| chosen.is_some_and(|c| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(c)))).unwrap_or(0);
    (!found.is_empty()).then(|| found.swap_remove(pick))
}

impl Helper {
    /// Starts the helper on a thread of its own, on the model chosen (its
    /// id, see `files`), which then writes what is sent. What it answers
    /// arrives in `events`, and `wake` is called.
    pub fn start(priority: Arc<dyn Priority>, model: Option<String>, wake: impl Fn() + Send + Sync + 'static) -> Helper {
        let (frames, pending) = channel::<Vec<u8>>();
        let (events_out, events) = channel();
        let child: Arc<Mutex<Option<Child>>> = Arc::default();
        let held = child.clone();
        let wake = Arc::new(wake);
        std::thread::spawn(move || {
            let started = files(model.as_deref()).and_then(|(helper, model)| {
                Command::new(&helper).arg("--model").arg(&model).current_dir(helper.parent()?).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()
            });
            let Some(mut process) = started else {
                let _ = events_out.send(Event::Ended(None));
                return wake();
            };
            // Below the app, as on Windows: typing comes first.
            priority.lower_process(process.id());
            let (Some(mut stdin), Some(stdout)) = (process.stdin.take(), process.stdout.take()) else {
                let _ = process.kill();
                let _ = events_out.send(Event::Ended(None));
                return wake();
            };
            *held.lock().unwrap() = Some(process);
            let (reader_wake, reader_child) = (wake.clone(), held.clone());
            std::thread::spawn(move || {
                let mut from = BufReader::new(stdout);
                while let Ok(Some(body)) = wire::read_frame(&mut from) {
                    let Some(reply) = Reply::parse(&body) else { continue };
                    if events_out.send(Event::Reply(reply)).is_err() {
                        break;
                    }
                    reader_wake();
                }
                // Its output closed: it ended, or is ending.
                let ended = reader_child.lock().unwrap().take();
                let code = ended.and_then(|mut c| c.wait().ok()).and_then(|s| s.code()).map_or(1, |c| c as u32);
                let _ = events_out.send(Event::Ended(Some(code)));
                reader_wake();
            });
            // Closing its input when `frames` goes ends the helper.
            while let Ok(frame) = pending.recv() {
                if wire::write_frame(&mut stdin, &frame).is_err() {
                    break;
                }
            }
        });
        Helper { frames, events, child }
    }

    /// A helper over channels, for tests: what is sent comes out of the
    /// first, and the second's events are answered.
    #[cfg(test)]
    pub fn over(frames: Sender<Vec<u8>>, events: Receiver<Event>) -> Helper {
        Helper { frames, events, child: Arc::default() }
    }

    /// False once the helper can no longer be written to.
    pub fn send(&self, request: &Request) -> bool {
        self.frames.send(request.frame()).is_ok()
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_assist::models;
    use std::fs;

    /// A bundle's MacOS folder with the helper in it, and an app data
    /// folder, both empty of models.
    fn folders(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("kolada-app-models-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (dir, data) = (root.join("Kolada.app").join("Contents").join("MacOS"), root.join("data"));
        fs::create_dir_all(dir.join("models")).unwrap();
        fs::create_dir_all(data.join("models")).unwrap();
        fs::write(dir.join(HELPER), b"").unwrap();
        (dir, data)
    }

    fn clean(dir: &Path) {
        let _ = fs::remove_dir_all(dir.ancestors().nth(3).unwrap());
    }

    #[test]
    fn no_model_means_no_helper() {
        let (dir, data) = folders("none");
        assert_eq!(files_in(&dir, Some(&data), None), None);
        assert_eq!(files_in(&dir, Some(&data), Some("full")), None, "chosen but not downloaded");
        // A model not trained for Kolada, a download cut short, and a
        // model's license are no model.
        fs::write(dir.join("models").join("LFM2.5-1.2B-Instruct-Q4_0.gguf"), b"x").unwrap();
        fs::write(dir.join("models").join("kolada-writer-mini-v1.gguf.part"), b"x").unwrap();
        fs::write(dir.join("models").join("LICENSE-LFM.txt"), b"x").unwrap();
        assert_eq!(files_in(&dir, Some(&data), None), None);
        clean(&dir);
    }

    #[test]
    fn a_model_staged_with_a_test_build_is_used_first() {
        let (dir, data) = folders("beside");
        let helper = dir.join(HELPER);
        // In the bundle's Resources.
        let resources = dir.parent().unwrap().join("Resources").join("models");
        fs::create_dir_all(&resources).unwrap();
        let bundled = resources.join("Kolada-writer-mini-v1.gguf");
        fs::write(&bundled, b"x").unwrap();
        assert_eq!(files_in(&dir, Some(&data), None), Some((helper.clone(), bundled.clone())), "with no choice made");
        assert_eq!(files_in(&dir, None, Some("full")), Some((helper.clone(), bundled.clone())), "over a choice not staged");
        // Beside the binary, the chosen one of two.
        let full = models::by_id("full").unwrap();
        fs::write(dir.join("models").join(full.file), b"x").unwrap();
        assert_eq!(files_in(&dir, Some(&data), Some("full")), Some((helper.clone(), dir.join("models").join(full.file))));
        // No helper, no help.
        fs::remove_file(&helper).unwrap();
        assert_eq!(files_in(&dir, Some(&data), Some("full")), None);
        clean(&dir);
    }

    #[test]
    fn the_chosen_model_comes_from_the_data_folder_by_id() {
        let (dir, data) = folders("chosen");
        let light = models::by_id("light").unwrap();
        let file = data.join("models").join(light.file);
        // Part of it: still downloading.
        fs::File::create(&file).unwrap().set_len(light.bytes - 1).unwrap();
        assert_eq!(files_in(&dir, Some(&data), Some("light")), None);
        fs::File::options().write(true).open(&file).unwrap().set_len(light.bytes).unwrap();
        assert_eq!(files_in(&dir, Some(&data), Some("light")), Some((dir.join(HELPER), file.clone())));
        // Another choice, or none, is not this file.
        assert_eq!(files_in(&dir, Some(&data), Some("full")), None);
        assert_eq!(files_in(&dir, Some(&data), None), None);
        assert_eq!(files_in(&dir, Some(&data), Some("nonsense")), None);
        // Notsy's helper, by its old name, still runs it.
        fs::rename(dir.join(HELPER), dir.join(HELPER_BEFORE)).unwrap();
        assert_eq!(files_in(&dir, Some(&data), Some("light")), Some((dir.join(HELPER_BEFORE), file)));
        clean(&dir);
    }
}
