//! The open note when another app moves or deletes its file. A move is
//! followed: the file is found at its new place (by what it is, not its
//! name) and the page goes on there, its unsaved words with it. A delete
//! leaves a fresh page, and the words that were open are kept in the vault's
//! trash, where they can be brought back.

use std::ffi::OsStr;

use super::*;
use crate::services::{Change, ITEM_CREATED, ITEM_RENAMED};

/// Which file this is, whatever it is called: its device and inode.
pub type FileId = (u64, u64);

#[cfg(unix)]
pub fn file_id(path: &Path) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some((m.dev(), m.ino()))
}

/// Where the OS has no device and inode to ask, a moved note is found by its
/// name alone (`find_moved` falls back to it when the id is not known).
#[cfg(not(unix))]
pub fn file_id(_: &Path) -> Option<FileId> {
    None
}

/// `path` as a place in the vault (the vault's own spelling of it: FSEvents
/// says `/private/var`, where the vault may say `/var`), unless it is in a
/// hidden folder or is hidden itself (`.trash`, `.obsidian`): a note moved
/// there is gone, not followed.
fn in_vault(vault: &Path, path: &Path) -> Option<PathBuf> {
    let rel = path.strip_prefix(vault).ok().map(Path::to_path_buf).or_else(|| path.strip_prefix(vault.canonicalize().ok()?).ok().map(Path::to_path_buf))?;
    if rel.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) {
        return None;
    }
    Some(vault.join(rel))
}

/// How many files a moved folder is searched through, at most.
const SEARCH_AT_MOST: usize = 20_000;

/// Where the open note went: the file with `id` (or, where that is not
/// known, the only file of the same `name`) among what `changes` say was
/// renamed or created: those files, and what is inside those folders.
pub fn find_moved(vault: &Path, id: Option<FileId>, name: Option<&OsStr>, changes: &[Change]) -> Option<PathBuf> {
    let mut by_name: Vec<PathBuf> = Vec::new();
    let mut seen = 0;
    let mut check = |file: &Path| -> Option<PathBuf> {
        if id.is_some() && file_id(file) == id {
            return Some(file.to_path_buf());
        }
        if name.is_some() && file.file_name() == name && !by_name.iter().any(|f| f == file) {
            by_name.push(file.to_path_buf());
        }
        None
    };
    for change in changes.iter().filter(|c| c.flags & (ITEM_RENAMED | ITEM_CREATED) != 0) {
        let Some(path) = in_vault(vault, &change.path) else { continue };
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.is_file() {
            if let Some(found) = check(&path) {
                return Some(found);
            }
        } else if meta.is_dir() {
            // A moved folder: its notes went with it.
            let mut stack = vec![path];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    seen += 1;
                    if seen > SEARCH_AT_MOST {
                        break;
                    }
                    if entry.file_name().to_string_lossy().starts_with('.') {
                        continue;
                    }
                    let Ok(kind) = entry.file_type() else { continue };
                    if kind.is_dir() {
                        stack.push(entry.path());
                    } else if kind.is_file()
                        && let Some(found) = check(&entry.path())
                    {
                        return Some(found);
                    }
                }
            }
        }
    }
    // Without an id to go by (or when a copy, on another volume, has a new
    // one): the one file of that name.
    if by_name.len() == 1 { by_name.pop() } else { None }
}

impl Model {
    /// The open note's file is gone from where it was: follow it, or leave.
    pub(super) fn follow_open_note(&mut self, changes: &[Change]) {
        let Some(old) = self.path.clone() else { return };
        if old.exists() {
            self.file_id = file_id(&old);
            return;
        }
        match find_moved(&self.vault, self.file_id, old.file_name(), changes) {
            Some(new) => self.moved_to(&old, new),
            None => self.deleted_outside(),
        }
    }

    /// Another app moved the open note's file: the page goes on at its new
    /// place (its past with it), unsaved words and all.
    fn moved_to(&mut self, old: &Path, new: PathBuf) {
        let (Some(from), Some(to)) = (vault::relative(&self.vault, old), vault::relative(&self.vault, &new)) else { return };
        self.history_renamed(&from, &to);
        self.session.renamed(&from, &to);
        self.session.note = to;
        self.file_id = file_id(&new);
        self.path = Some(new);
        self.read_times();
        self.refresh_tree();
    }

    /// Another app deleted the open note's file: a fresh page. What was open
    /// is kept in the vault's trash first, to be brought back from there.
    fn deleted_outside(&mut self) {
        let text = self.editor.text().to_string();
        if !text.trim().is_empty() {
            let dir = self.vault.join(trash::FOLDER);
            if std::fs::create_dir_all(&dir).is_ok() {
                let file = note::unique_path(&dir, &note::file_stem_for(&self.editor.title()), None);
                let _ = note::save(&file, &text, self.format);
            }
        }
        self.dirty = false;
        self.format = note::Format::default();
        self.session.start_fresh();
        self.show(String::new(), String::new(), None);
        self.refresh_tree();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ITEM_REMOVED;

    fn vault(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-app-outside-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Work")).unwrap();
        std::fs::write(dir.join("Plan.md"), "# Plan\n\nBody.\n").unwrap();
        std::fs::write(dir.join("Other.md"), "# Other\n").unwrap();
        dir.canonicalize().unwrap()
    }

    fn renamed(path: &Path) -> Change {
        Change { path: path.to_path_buf(), flags: ITEM_RENAMED }
    }

    fn open(v: &Path) -> Model {
        let mut m = Model::new(v.to_path_buf(), true);
        m.open("Plan.md");
        m.frame(2000, 1440, 2.0);
        m
    }

    // Needs a file's identity (device and inode), which only unix tells.
    #[test]
    #[cfg(unix)]
    fn a_note_renamed_by_another_app_is_followed_to_its_new_name() {
        let v = vault("renamed");
        let mut m = open(&v);
        std::fs::rename(v.join("Plan.md"), v.join("Launch plan.md")).unwrap();
        assert!(m.apply_outside(&[renamed(&v.join("Plan.md")), renamed(&v.join("Launch plan.md"))]));
        assert_eq!(m.session.note, "Launch plan.md");
        assert_eq!(m.path.as_deref(), Some(v.join("Launch plan.md").as_path()));
        assert!(m.text().contains("Body."), "the page goes on");
        // Typing saves to the new place; the old one is not made again.
        m.go(Motion::DocEnd, false);
        m.insert("more");
        m.save();
        assert!(std::fs::read_to_string(v.join("Launch plan.md")).unwrap().contains("more"));
        assert!(!v.join("Plan.md").exists());
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn a_note_moved_to_another_folder_or_in_a_moved_folder_is_followed() {
        let v = vault("moved");
        let mut m = open(&v);
        std::fs::rename(v.join("Plan.md"), v.join("Work").join("Plan.md")).unwrap();
        assert!(m.apply_outside(&[renamed(&v.join("Plan.md")), renamed(&v.join("Work").join("Plan.md"))]));
        assert_eq!(m.session.note, "Work/Plan.md");
        // Its folder renamed: only the folder is in the news.
        std::fs::rename(v.join("Work"), v.join("Projects")).unwrap();
        assert!(m.apply_outside(&[renamed(&v.join("Work")), renamed(&v.join("Projects"))]));
        assert_eq!(m.session.note, "Projects/Plan.md");
        assert!(m.text().contains("Body."));
        let _ = std::fs::remove_dir_all(v);
    }

    // Needs a file's identity (device and inode), which only unix tells.
    #[test]
    #[cfg(unix)]
    fn unsaved_words_go_with_a_note_that_was_renamed() {
        let v = vault("dirty");
        let mut m = open(&v);
        m.go(Motion::DocEnd, false);
        m.insert("unsaved words");
        std::fs::rename(v.join("Plan.md"), v.join("Renamed.md")).unwrap();
        m.apply_outside(&[renamed(&v.join("Plan.md")), renamed(&v.join("Renamed.md"))]);
        assert!(m.text().ends_with("unsaved words"), "not thrown away");
        assert_eq!(m.session.note, "Renamed.md");
        m.save();
        assert!(std::fs::read_to_string(v.join("Renamed.md")).unwrap().contains("unsaved words"));
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn it_is_not_followed_into_a_hidden_folder_or_by_another_files_name() {
        let v = vault("hidden");
        let id = file_id(&v.join("Plan.md"));
        std::fs::create_dir_all(v.join(".trash")).unwrap();
        std::fs::rename(v.join("Plan.md"), v.join(".trash").join("Plan.md")).unwrap();
        let news = [renamed(&v.join("Plan.md")), renamed(&v.join(".trash").join("Plan.md"))];
        assert_eq!(find_moved(&v, id, Some(OsStr::new("Plan.md")), &news), None, "moved to a hidden trash is gone");
        // Another file with another id and another name is not it.
        let news = [renamed(&v.join("Other.md"))];
        assert_eq!(find_moved(&v, id, Some(OsStr::new("Plan.md")), &news), None);
        // Without an id, a file of the same name is.
        std::fs::write(v.join("Work").join("Plan.md"), "x").unwrap();
        let news = [Change { path: v.join("Work").join("Plan.md"), flags: ITEM_CREATED }];
        assert_eq!(find_moved(&v, None, Some(OsStr::new("Plan.md")), &news), Some(v.join("Work").join("Plan.md")));
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn a_note_deleted_by_another_app_leaves_a_fresh_page_and_its_words_in_the_trash() {
        let v = vault("deleted");
        let mut m = open(&v);
        m.go(Motion::DocEnd, false);
        m.insert("last words");
        std::fs::remove_file(v.join("Plan.md")).unwrap();
        assert!(m.apply_outside(&[Change { path: v.join("Plan.md"), flags: ITEM_REMOVED }]));
        assert_eq!(m.session.note, "");
        assert!(m.path.is_none() && m.text().is_empty(), "a fresh page");
        assert!(!v.join("Plan.md").exists(), "typing is not what brings it back");
        assert!(m.open_trash());
        let (items, _) = m.trash.as_ref().unwrap();
        let kept = items.iter().find(|t| t.name() == "Plan").expect("the words are in the trash");
        assert!(std::fs::read_to_string(&kept.file).unwrap().ends_with("last words"));
        let _ = std::fs::remove_dir_all(v);
    }
}
