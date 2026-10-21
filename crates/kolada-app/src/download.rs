//! The writing model the person chose, downloaded into the app's data as
//! on Windows (`kolada-win` `assist.rs` `Download`): on a thread of its own
//! at background priority, through the OS's HTTP (`Web::get_from`), by the
//! shared `kolada_core::fetch` (a part file that resumes, checked by its
//! SHA-256 before it is used). Dropping it stops the download; what came is
//! kept and the next one resumes from there.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use kolada_assist::models;
use kolada_core::fetch;

use crate::services::{Priority, Web};

/// What a model's download says as it goes.
pub enum Fetched {
    /// Bytes in so far, of all of them: at most a thousand times in all.
    Progress(u64, u64),
    Ended(fetch::Outcome),
}

pub struct Download {
    pub id: &'static str,
    pub events: Receiver<Fetched>,
    cancel: Arc<AtomicBool>,
}

/// The OS's HTTP as the shared download asks for it.
struct Through<'a>(&'a dyn Web);

impl fetch::Source for Through<'_> {
    fn get_from(&self, url: &str, from: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16> {
        self.0.get_from(url, from, body)
    }
}

impl Download {
    /// The model `id` downloading into `dir`; `wake` is called after each
    /// thing it says. None when `id` is no model in the catalogue, or it has
    /// nowhere to come from yet (`models::HUB`, `fetch::address`).
    pub fn start(web: Arc<dyn Web>, priority: Arc<dyn Priority>, dir: &Path, id: &str, wake: impl Fn() + Send + 'static) -> Option<Download> {
        let m = models::by_id(id)?;
        let url = fetch::address(models::url(m), m.file)?;
        let dir = dir.to_path_buf();
        let cancel = Arc::new(AtomicBool::new(false));
        let (out, events) = channel();
        let stop = cancel.clone();
        std::thread::spawn(move || {
            priority.lower_thread();
            let want = fetch::Want { url: &url, file: m.file, bytes: m.bytes, sha256: m.sha256 };
            fetch_into(&dir, &want, &Through(&*web), &stop, &out, &wake);
        });
        Some(Download { id: m.id, events, cancel })
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// The download's thread: `want` from `source` into `dir`, telling `out`
/// how it goes (a thousandth at a time at most) and how it ended, with a
/// `wake` after each.
pub(crate) fn fetch_into(dir: &Path, want: &fetch::Want, source: &dyn fetch::Source, cancel: &AtomicBool, out: &Sender<Fetched>, wake: &dyn Fn()) {
    let mut shown = None;
    let mut progress = |done: u64, total: u64| {
        let step = done.saturating_mul(1000) / total.max(1);
        if shown.replace(step) != Some(step) && out.send(Fetched::Progress(done, total)).is_ok() {
            wake();
        }
    };
    let outcome = match std::fs::create_dir_all(dir) {
        Ok(()) => fetch::download(dir, want, source, &mut progress, cancel),
        Err(_) => fetch::Outcome::Failed(fetch::Failure::Disk),
    };
    let _ = out.send(Fetched::Ended(outcome));
    wake();
}

/// Once `keep` is in place in `dir`: the catalogue's other models, and what
/// came of their downloads, go, so switching sizes frees the space.
pub fn remove_other_models(dir: &Path, keep: &str) {
    let files: Vec<&str> = models::MODELS.iter().map(|m| m.file).collect();
    fetch::remove_others(dir, Some(keep), &files);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A server in memory, answering every range in small pieces.
    struct Memory(Vec<u8>);

    impl fetch::Source for Memory {
        fn get_from(&self, _url: &str, from: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16> {
            for piece in self.0[from as usize..].chunks(100) {
                if !body(piece) {
                    break;
                }
            }
            Some(if from > 0 { 206 } else { 200 })
        }
    }

    #[test]
    fn a_model_downloads_telling_the_window_how_it_goes() {
        let root = std::env::temp_dir().join(format!("kolada-app-download-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("models");
        let file: Vec<u8> = (0..250_000u32).map(|i| (i % 253) as u8).collect();
        let sha = kolada_core::update::hex(&kolada_core::update::sha256(&file));
        let want = fetch::Want { url: "https://huggingface.co/x", file: "Kolada-writer-mini-v1.gguf", bytes: file.len() as u64, sha256: &sha };
        let (out, events) = channel();
        let wakes = std::cell::Cell::new(0);
        fetch_into(&dir, &want, &Memory(file.clone()), &AtomicBool::new(false), &out, &|| wakes.set(wakes.get() + 1));
        let said: Vec<Fetched> = events.try_iter().collect();
        assert_eq!(wakes.get(), said.len(), "a wake for each");
        let steps: Vec<(u64, u64)> = said.iter().filter_map(|s| if let Fetched::Progress(d, t) = s { Some((*d, *t)) } else { None }).collect();
        // 2,500 pieces came, the window heard of at most a thousandth each.
        assert!(steps.len() > 10 && steps.len() <= 1001, "{} steps", steps.len());
        assert!(steps.windows(2).all(|w| w[0].0 < w[1].0) && steps.iter().all(|&(_, t)| t == file.len() as u64));
        assert!(matches!(said.last(), Some(Fetched::Ended(fetch::Outcome::Done(p))) if *p == dir.join(want.file)));
        assert!(fs::read(dir.join(want.file)).unwrap() == file);
        // Stopped before it starts: it says so, and nothing is in place.
        let other = fetch::Want { file: "Kolada-writer-med-v1.gguf", ..want };
        fetch_into(&dir, &other, &Memory(file), &AtomicBool::new(true), &out, &|| {});
        assert!(matches!(events.try_iter().last(), Some(Fetched::Ended(fetch::Outcome::Cancelled))));
        assert!(!dir.join(other.file).exists());
        // Switching sizes: the other models go, the one kept stays.
        let light = models::by_id("light").unwrap();
        let full = models::by_id("full").unwrap();
        fs::write(dir.join(light.file), b"x").unwrap();
        fs::write(dir.join(format!("{}.part", light.file)), b"x").unwrap();
        fs::write(dir.join(full.file), b"x").unwrap();
        remove_other_models(&dir, full.file);
        assert!(!dir.join(light.file).exists() && !dir.join(format!("{}.part", light.file)).exists());
        assert!(dir.join(full.file).exists() && dir.join(want.file).exists(), "files not in the catalogue are left alone");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_model_with_nowhere_to_come_from_does_not_start() {
        let web: Arc<dyn Web> = Arc::new(crate::services::NoServices);
        let priority: Arc<dyn Priority> = Arc::new(crate::services::NoServices);
        let dir = std::env::temp_dir().join(format!("kolada-app-download-none-{}", std::process::id()));
        assert!(Download::start(web.clone(), priority.clone(), &dir, "nonsense", || {}).is_none(), "no such model");
        // The catalogue is not published yet, and this is no test instance
        // with a local server: nowhere to download from.
        if models::HUB.is_none() && !kolada_core::testing() {
            assert!(Download::start(web, priority, &dir, "light", || {}).is_none());
        }
        assert!(!dir.exists(), "nothing written");
    }
}
