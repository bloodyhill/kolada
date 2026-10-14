//! Deleting notes and bringing them back. A deleted note goes to the vault's
//! trash (see `kolada_core::trash`): Ctrl+Z right after brings it back, and
//! the trash, in place of the file tree, lists every deleted note to restore
//! or remove for good (to the Windows Recycle Bin, never beyond).

use kolada_core::trash::{self, Trashed};

use super::*;
use crate::scene::TrashEntry;

/// The trash as the side menu shows it.
pub struct TrashList {
    items: Vec<Trashed>,
    pub entries: Vec<TrashEntry>,
}

/// Where the app notes what each deleted note was.
fn log() -> PathBuf {
    kolada_core::app_data_dir().unwrap_or_default().join("trash.tsv")
}

/// How long ago `at` was, in a word or two.
pub fn when(at: u64, now: u64) -> String {
    let s = now.saturating_sub(at);
    match s {
        0..60 => "Just now".to_string(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        86_400..172_800 => "Yesterday".to_string(),
        _ => format!("{} days ago", s / 86_400),
    }
}

impl App {
    /// Moves the note at `rel` (in the vault) to the trash. The page's own
    /// note leaves a fresh page behind; one open over the page closes.
    pub(super) fn trash_note(&mut self, rel: &str) {
        if self.session.vault.is_empty() || rel.is_empty() {
            return;
        }
        let vault = PathBuf::from(&self.session.vault);
        let mut rel = rel.to_string();
        let page = self.session.note == rel;
        if page {
            // What is written so far goes with it, so it comes back whole.
            // (Saving may rename it after its title.)
            self.save_note(true);
            rel = self.session.note.clone();
            // It was blank, and is gone already.
            if rel.is_empty() {
                return;
            }
        }
        if self.popup.as_ref().is_some_and(|p| p.path == vault.join(&rel)) {
            self.close_popup();
        }
        let item = match trash::trash(&vault, &rel, &log(), session::now()) {
            Ok(item) => item,
            Err(_) => return self.whisper("Could not delete that note".to_string()),
        };
        let name = item.name();
        if page {
            self.stop_timer(AUTOSAVE_TIMER);
            self.session.start_fresh();
            self.load(Opened::default(), 0, true);
            self.update_related();
        }
        self.undo_delete.push((item, self.editor.version()));
        self.refresh_tree();
        self.start_index();
        if self.trash.is_some() {
            self.open_trash();
        }
        self.whisper(format!("Deleted {name} \u{00B7} Ctrl+Z brings it back"));
    }

    /// Ctrl+Z right after deleting a note: it comes back, and opens.
    pub(super) fn undo_last_delete(&mut self) {
        let Some((item, _)) = self.undo_delete.pop() else { return };
        self.bring_back(&item);
    }

    fn bring_back(&mut self, item: &Trashed) {
        let vault = PathBuf::from(&self.session.vault);
        match trash::restore(&vault, item, &log()) {
            Ok(path) => {
                self.open(&path);
                self.refresh_tree();
                self.start_index();
                if self.trash.is_some() {
                    self.open_trash();
                    // Nothing left in it: back to the tree.
                    if self.trash.as_ref().is_some_and(|t| t.items.is_empty()) {
                        self.close_trash();
                    }
                }
                self.whisper(format!("Brought back {}", stem(&path)));
            }
            Err(_) => self.whisper(format!("Could not bring back {}", item.name())),
        }
    }

    /// The trash, in place of the file tree (again, to refresh it).
    pub(super) fn open_trash(&mut self) {
        if self.search.is_some() {
            self.close_search();
        }
        self.ui.options_open = false;
        let items = trash::list(Path::new(&self.session.vault), &log());
        let now = session::now();
        let entries = items.iter().map(|t| TrashEntry { name: t.name(), when: when(t.at, now) }).collect();
        self.trash = Some(TrashList { items, entries });
        self.request_frame();
    }

    pub(super) fn close_trash(&mut self) {
        if self.trash.take().is_some() {
            self.request_frame();
            self.update_reveal();
        }
    }

    /// The deleted note at `i` in the trash.
    pub(super) fn trashed(&self, i: usize) -> Option<Trashed> {
        self.trash.as_ref()?.items.get(i).cloned()
    }

    /// Brings back the `i`th note in the trash, and opens it.
    pub(super) fn restore(&mut self, i: usize) {
        if let Some(item) = self.trashed(i) {
            self.bring_back(&item);
        }
    }

    /// The `i`th note in the trash goes to the Windows Recycle Bin.
    pub(super) fn delete_for_good(&mut self, i: usize) {
        let Some(item) = self.trashed(i) else { return };
        if crate::recycle(self.hwnd, &item.file) {
            trash::forget(Path::new(&self.session.vault), &item, &log());
            self.undo_delete.retain(|(t, _)| t.file != item.file);
            self.whisper(format!("{} is in the Recycle Bin now", item.name()));
        }
        self.open_trash();
    }

    /// Every note in the trash goes to the Windows Recycle Bin.
    pub(super) fn empty_trash(&mut self) {
        let Some(list) = self.trash.take() else { return };
        let vault = PathBuf::from(&self.session.vault);
        let mut moved = 0;
        for item in &list.items {
            if crate::recycle(self.hwnd, &item.file) {
                trash::forget(&vault, item, &log());
                moved += 1;
            }
        }
        self.undo_delete.clear();
        self.open_trash();
        if moved > 0 {
            self.whisper(format!("Emptied the trash \u{00B7} {moved} in the Recycle Bin"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::when;

    #[test]
    fn says_when_in_a_word_or_two() {
        let now = 1_800_000_000;
        assert_eq!(when(now - 5, now), "Just now");
        assert_eq!(when(now - 600, now), "10 min ago");
        assert_eq!(when(now - 7200, now), "2 h ago");
        assert_eq!(when(now - 90_000, now), "Yesterday");
        assert_eq!(when(now - 5 * 86_400, now), "5 days ago");
    }
}
