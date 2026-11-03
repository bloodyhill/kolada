//! What a shell's own screens ask of the model, where the shell draws its
//! lists itself (the iPhone's): every note's text searched, the trash's
//! notes and bringing one back, a note duplicated, a share link made for
//! the open note. The model keeps the work; the shell only shows it.

use kolada_core::search::Hit;

use super::menu::MenuAction;
use super::*;

impl Model {
    /// Notes whose names or words match `query` (typos forgiven), best
    /// first, each with the words around its match. None while every
    /// note's text is still being read (the first time it is asked); the
    /// shell shows names meanwhile and asks again on the index's wake.
    pub fn search_notes(&mut self, query: &str) -> Option<Vec<Hit>> {
        self.start_index();
        let all = self.index.lock().unwrap();
        all.as_ref().map(|all| search::search(all, query, SEARCH_LIMIT))
    }

    /// The notes in the vault's trash, most recently deleted first: each
    /// one's name and when it went ("2 days ago").
    pub fn trashed_notes(&mut self) -> Vec<(String, String)> {
        self.open_trash();
        let entries = self.trash.as_ref().map(|(_, e)| e.iter().map(|t| (t.name.clone(), t.when.clone())).collect()).unwrap_or_default();
        // The list is the shell's to show: the model's own trash view stays shut.
        self.trash = None;
        entries
    }

    /// Brings back the `i`th note of `trashed_notes`, and opens it.
    pub fn restore_trashed(&mut self, i: usize) -> bool {
        self.open_trash();
        let back = self.trash_pick(MenuAction::Restore(i), i);
        self.trash = None;
        back
    }

    /// A copy of the note at `rel` beside it, opened.
    pub fn duplicate_note(&mut self, rel: &str) -> bool {
        vault::is_note_path(rel) && self.run_menu(MenuAction::Duplicate(rel.to_string())).0
    }

    /// The open note goes up as a short link, as the share card's Create
    /// does: the link comes back on the clipboard (an effect) once made.
    /// False when there is nothing to share.
    pub fn share_link(&mut self) -> bool {
        if self.editor.text().trim().is_empty() {
            return false;
        }
        self.share_now();
        true
    }
}
