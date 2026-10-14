//! Files handed to Kolada from outside, as on Windows (`kolada-win`
//! `main.rs`, `jump.rs`): a note opened with it (its vault becomes the one
//! open), another app's export or a document imported; and the notes
//! written last, for the Dock's menu.

use std::time::SystemTime;

use super::arrive::{is_document, is_note_file};
use super::*;

/// The notes of `vault` written most recently, newest first: `limit` of them.
pub fn recent_notes(vault: &Path, limit: usize) -> Vec<PathBuf> {
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut dirs = vec![vault.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => dirs.push(e.path()),
                Ok(_) if name.to_lowercase().ends_with(".md") => {
                    if let Ok(modified) = e.metadata().and_then(|m| m.modified()) {
                        found.push((modified, e.path()));
                    }
                }
                _ => {}
            }
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.truncate(limit);
    found.into_iter().map(|(_, p)| p).collect()
}

/// Recent notes listed at most.
const RECENT: usize = 7;

impl Model {
    /// The notes written last, for the Dock's menu: their names and paths.
    /// From a list found on a thread (the whole vault is walked once, after
    /// the first frame), kept as notes are written: the menu never waits.
    pub fn recent(&self, limit: usize) -> Vec<(String, PathBuf)> {
        let list = self.recent.lock().unwrap().clone();
        list.into_iter().filter(|p| p.is_file()).take(limit).map(|p| (p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), p)).collect()
    }

    /// Finds the notes written last, on a thread of its own.
    pub(super) fn find_recent(&mut self) {
        let (slot, vault) = (self.recent.clone(), self.vault.clone());
        std::thread::spawn(move || {
            let found = recent_notes(&vault, RECENT * 2);
            *slot.lock().unwrap() = found;
        });
    }

    /// The note at `path` was just written: first among the recent.
    pub(super) fn written_recently(&mut self, path: &Path) {
        let mut list = self.recent.lock().unwrap();
        list.retain(|p| p != path);
        list.insert(0, path.to_path_buf());
        list.truncate(RECENT * 2);
    }

    /// A file opened with Kolada (from the Finder, another app, the Dock): a
    /// note opens, its vault becoming the one open when it is outside this
    /// one; an export or a document is imported. Whether the vault changed
    /// (its files are then followed afresh).
    pub fn open_path(&mut self, path: &Path) -> bool {
        if is_document(path) {
            self.import_files(vec![path.to_path_buf()]);
            return false;
        }
        if !path.is_file() {
            return false;
        }
        // Anything else (an RTF, a picture, an app) would open as a page of
        // its raw bytes, in a vault of its folder, and be written back.
        if !is_note_file(path) {
            self.whisper("Kolada can't read this");
            return false;
        }
        if let Some(rel) = vault::relative(&self.vault, path).filter(|_| path.starts_with(&self.vault)) {
            self.open_note(&rel);
            return false;
        }
        let root = vault::root_for(path);
        let Some(rel) = vault::relative(&root, path) else { return false };
        let changed = self.switch_vault(root);
        self.open_note(&rel);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kolada-app-opening-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_note_opened_with_kolada_opens_in_its_own_vault() {
        let (a, b) = (dir("a"), dir("b"));
        std::fs::write(a.join("Home.md"), "# Home\n").unwrap();
        std::fs::create_dir_all(b.join(".obsidian")).unwrap();
        std::fs::create_dir_all(b.join("Deep")).unwrap();
        std::fs::write(b.join("Deep").join("Far.md"), "# Far\n").unwrap();
        let mut m = Model::new(a.clone(), true);
        // One in this vault just opens.
        std::fs::write(a.join("Other.md"), "# Other\n").unwrap();
        assert!(!m.open_path(&a.join("Other.md")));
        assert_eq!(m.session.note, "Other.md");
        // One elsewhere: its vault (the folder with .obsidian above it) opens.
        assert!(m.open_path(&b.join("Deep").join("Far.md")));
        assert_eq!(m.vault(), b.as_path());
        assert_eq!(m.session.note, "Deep/Far.md");
        let _ = (std::fs::remove_dir_all(a), std::fs::remove_dir_all(b));
    }

    #[test]
    fn a_file_kolada_cannot_read_is_not_opened_as_a_note() {
        let (a, b) = (dir("unreadable-a"), dir("unreadable-b"));
        std::fs::write(a.join("Home.md"), "# Home\n").unwrap();
        let rtf = "{\\rtf1\\ansi The \n\\b first \n}";
        std::fs::write(b.join("Plan.rtf"), rtf).unwrap();
        let mut m = Model::new(a.clone(), true);
        assert!(!m.open_path(&b.join("Plan.rtf")));
        assert_eq!(m.vault(), a.as_path(), "the vault stays");
        assert_eq!(m.whispered(), Some("Kolada can't read this"));
        m.new_note();
        assert_eq!(std::fs::read_to_string(b.join("Plan.rtf")).unwrap(), rtf, "never written");
        let _ = (std::fs::remove_dir_all(a), std::fs::remove_dir_all(b));
    }

    #[test]
    fn a_note_opened_with_kolada_and_left_unedited_is_not_written() {
        let (a, b) = (dir("unedited-a"), dir("unedited-b"));
        std::fs::write(a.join("Home.md"), "# Home\n").unwrap();
        // Spaces at line ends, and empty lines, that tidying would take out.
        let text = "# Far\n\nThe \nnext   \n\n\n\nend\n";
        std::fs::write(b.join("Far.md"), text).unwrap();
        let mut m = Model::new(a.clone(), true);
        // One in this vault, and one opened with Kolada from elsewhere.
        std::fs::write(a.join("Near.md"), text).unwrap();
        m.open("Near.md");
        m.open_path(&b.join("Far.md"));
        m.new_note();
        assert_eq!(std::fs::read_to_string(a.join("Near.md")).unwrap(), text, "the note was rewritten without an edit");
        assert_eq!(std::fs::read_to_string(b.join("Far.md")).unwrap(), text, "the note was rewritten without an edit");
        // An edit still tidies it as it is left.
        m.open("Far.md");
        m.insert("!");
        m.new_note();
        assert_eq!(std::fs::read_to_string(b.join("Far.md")).unwrap(), "# Far\n!\nThe\nnext\n\nend\n");
        let _ = (std::fs::remove_dir_all(a), std::fs::remove_dir_all(b));
    }

    #[test]
    fn the_dock_lists_the_notes_written_last_newest_first() {
        let v = dir("recent");
        std::fs::create_dir_all(v.join("Sub")).unwrap();
        for (i, name) in ["Old.md", "Sub/Mid.md", "New.md"].iter().enumerate() {
            let p = v.join(name);
            std::fs::write(&p, "x").unwrap();
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + i as u64 * 60);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        let mut m = Model::new(v.clone(), true);
        m.find_recent();
        for _ in 0..200 {
            if !m.recent.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let names: Vec<String> = m.recent(2).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["New", "Mid"]);
        // A note written goes first, without walking the vault again.
        m.open("Old.md");
        m.insert("more ");
        m.save();
        assert_eq!(m.recent(1)[0].0, "Old");
        let _ = std::fs::remove_dir_all(v);
    }
}
