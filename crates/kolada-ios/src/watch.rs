//! Following what other apps do to the vault while Kolada is in front (an
//! iPad's Split View beside the Files app or another Markdown app): iOS has
//! no FSEvents for an app, so a thread looks at the vault's notes and
//! folders every few seconds while the app is active, and reports what was
//! made, removed or changed (a removal and a making in one look are taken
//! for a move, as FSEvents reports one, so the open note is followed by its
//! file id). The longer a look takes, the longer it waits before the next,
//! so a vault of a hundred thousand notes costs no more than a small one.

use std::collections::HashMap;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

use kolada_app::services::{Change, ITEM_CREATED, ITEM_REMOVED, ITEM_RENAMED, Watcher};

use crate::KoladaApp;

/// Whether the app is in front (looks are made only then).
static ACTIVE: AtomicBool = AtomicBool::new(true);

/// The app came to the front (true) or left it.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_set_active(_h: *mut KoladaApp, active: bool) {

    ACTIVE.store(active, Ordering::Relaxed);
}

/// What a look saw: each note's and folder's change time and size.
pub type Seen = HashMap<PathBuf, (SystemTime, u64)>;

/// The vault's notes and folders, hidden ones (`.trash`, `.obsidian`) left out.
pub fn look(vault: &Path) -> Seen {
    let mut seen = Seen::new();
    let mut dirs = vec![vault.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let name = e.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            let path = e.path();
            if meta.is_dir() {
                dirs.push(path.clone());
            } else if !kolada_app::model::arrive::is_note_file(&path) {
                continue;
            }
            seen.insert(path, (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len()));
        }
    }
    seen
}

/// What changed between two looks, as the model takes changes.
pub fn changes(before: &Seen, now: &Seen) -> Vec<Change> {
    let gone: Vec<&PathBuf> = before.keys().filter(|p| !now.contains_key(*p)).collect();
    let made: Vec<&PathBuf> = now.keys().filter(|p| !before.contains_key(*p)).collect();
    // Both in one look: a move or a rename, as FSEvents says it.
    let moved = !gone.is_empty() && !made.is_empty();
    let mut out: Vec<Change> = Vec::new();
    for p in &gone {
        out.push(Change { path: (*p).clone(), flags: if moved { ITEM_RENAMED } else { ITEM_REMOVED } });
    }
    for p in &made {
        out.push(Change { path: (*p).clone(), flags: if moved { ITEM_RENAMED } else { ITEM_CREATED } });
    }
    for (p, was) in before {
        if now.get(p).is_some_and(|n| n != was) {
            out.push(Change { path: p.clone(), flags: ITEM_CREATED });
        }
    }
    out
}

/// Looks every few seconds while the app is in front.
pub struct PollWatcher {
    pub every: Duration,
}

impl Watcher for PollWatcher {
    fn watch(&self, vault: &Path, wake: Box<dyn Fn() + Send>) -> Receiver<Change> {
        let (tx, rx) = channel();
        let (vault, every) = (vault.to_path_buf(), self.every);
        std::thread::spawn(move || {
            let mut seen = look(&vault);
            let mut wait = every;
            loop {
                std::thread::sleep(wait);
                if !ACTIVE.load(Ordering::Relaxed) {
                    continue;
                }
                let t = Instant::now();
                let now = look(&vault);
                // A look that takes long waits longer: at most a twentieth of the time spent looking.
                wait = every.max(t.elapsed() * 20);
                let found = changes(&seen, &now);
                seen = now;
                if found.is_empty() {
                    continue;
                }
                for c in found {
                    if tx.send(c).is_err() {
                        return;
                    }
                }
                wake();
            }
        });
        rx
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_made_changed_moved_and_removed_by_another_app_are_reported() {
        let v = std::env::temp_dir().join(format!("kolada-ios-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&v);
        std::fs::create_dir_all(v.join("Projects")).unwrap();
        std::fs::create_dir_all(v.join(".trash")).unwrap();
        std::fs::write(v.join("Plan.md"), "# Plan\n").unwrap();
        std::fs::write(v.join(".trash").join("Old.md"), "x").unwrap();
        std::fs::write(v.join("bird.png"), "x").unwrap();
        let first = look(&v);
        assert!(first.contains_key(&v.join("Plan.md")) && first.contains_key(&v.join("Projects")));
        assert!(!first.contains_key(&v.join(".trash").join("Old.md")) && !first.contains_key(&v.join("bird.png")), "notes and folders only, hidden ones left out");
        // Made, and changed.
        std::fs::write(v.join("Projects").join("New.md"), "# New\n").unwrap();
        std::fs::write(v.join("Plan.md"), "# Plan\n\nlonger now\n").unwrap();
        let second = look(&v);
        let c = changes(&first, &second);
        assert!(c.iter().any(|c| c.path == v.join("Projects").join("New.md") && c.flags == ITEM_CREATED));
        assert!(c.iter().any(|c| c.path == v.join("Plan.md") && c.flags == ITEM_CREATED), "changed: {c:?}");
        // Moved: both ends, flagged renamed, as FSEvents says it.
        std::fs::rename(v.join("Plan.md"), v.join("Projects").join("Plan.md")).unwrap();
        let third = look(&v);
        let c = changes(&second, &third);
        assert!(c.iter().filter(|c| c.flags == ITEM_RENAMED).count() == 2, "{c:?}");
        // Removed.
        std::fs::remove_file(v.join("Projects").join("New.md")).unwrap();
        let c = changes(&third, &look(&v));
        // (Its folder changed with it, where the file system says so.)
        assert!(c.contains(&Change { path: v.join("Projects").join("New.md"), flags: ITEM_REMOVED }), "{c:?}");
        assert!(c.iter().all(|c| c.path.starts_with(v.join("Projects"))), "{c:?}");
        assert!(changes(&third, &third).is_empty());
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn the_watcher_wakes_the_app_when_a_note_arrives() {
        let v = std::env::temp_dir().join(format!("kolada-ios-watching-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&v);
        std::fs::create_dir_all(&v).unwrap();
        let (woke, rx_woke) = channel();
        let rx = PollWatcher { every: Duration::from_millis(30) }.watch(&v, Box::new(move || { let _ = woke.send(()); }));
        std::thread::sleep(Duration::from_millis(60));
        std::fs::write(v.join("Arrived.md"), "# Arrived\n").unwrap();
        rx_woke.recv_timeout(Duration::from_secs(5)).expect("woken");
        assert_eq!(rx.try_recv().unwrap().path, v.join("Arrived.md"));
        let _ = std::fs::remove_dir_all(v);
    }
}
