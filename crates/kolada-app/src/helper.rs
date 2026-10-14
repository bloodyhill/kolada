//! The writing model's own process, `kolada-ai`, beside the app, as on
//! Windows (`kolada-win` `assist.rs`). It starts a moment after the window
//! shows, at low priority, and ends with Kolada (or when writing help is
//! turned off). Requests go down its standard input as frames (see
//! `kolada_assist::wire`) from a writer thread; a reader thread takes its
//! replies off its standard output and wakes the window. Without its files
//! it stays off, without a word.

use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use kolada_assist::wire::{self, Reply, Request};

use crate::services::Priority;

/// The one model, beside the app or in its data (shared by every Kolada):
/// the model trained for Kolada's fixes. A build may name another
/// (KOLADA_MODEL_FILE), as on Windows.
const MODEL: &str = match option_env!("KOLADA_MODEL_FILE") {
    Some(m) => m,
    None => kolada_core::before::MODEL,
};

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

/// The helper and its model, if both are there: `kolada-ai` beside the app's
/// binary; the model in the app's Resources/models, or in its data's models.
fn files() -> Option<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let helper = exe.with_file_name("kolada-ai");
    if !helper.is_file() {
        return None;
    }
    let bundled = exe.parent()?.parent()?.join("Resources").join("models").join(MODEL);
    if bundled.is_file() {
        return Some((helper, bundled));
    }
    let beside = exe.parent()?.join("models").join(MODEL);
    if beside.is_file() {
        return Some((helper, beside));
    }
    let shared = kolada_core::app_data_dir()?.join("models").join(MODEL);
    shared.is_file().then_some((helper, shared))
}

impl Helper {
    /// Starts the helper on a thread of its own, which then writes what is
    /// sent. What it answers arrives in `events`, and `wake` is called.
    pub fn start(priority: Arc<dyn Priority>, wake: impl Fn() + Send + Sync + 'static) -> Helper {
        let (frames, pending) = channel::<Vec<u8>>();
        let (events_out, events) = channel();
        let child: Arc<Mutex<Option<Child>>> = Arc::default();
        let held = child.clone();
        let wake = Arc::new(wake);
        std::thread::spawn(move || {
            let started = files().and_then(|(helper, model)| {
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
