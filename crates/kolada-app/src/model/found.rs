//! A note opened from search opens on what it found (`kolada_ui::found`):
//! scrolled so its first match sits a little above the middle of the
//! window, every match tinted as marked text is. The matches are found once,
//! as the note opens; the tint goes with the note's first edit (any change
//! to its text: they are for the text they were found in), another note
//! opening, the search being closed, or Esc. A click in the note or the
//! caret moving keeps it. As Windows' `app.rs` has it.

use kolada_ui::found::{self, Found};

use super::*;

impl Model {
    /// The note just opened came from a search for `query`: what it found
    /// there, the caret at the first match, and the view put on it by the
    /// next frame.
    pub(super) fn opened_from_search(&mut self, query: &str) {
        self.found = Found::opened(self.editor.text(), self.editor.outline(), query, self.editor.version());
        let Some(f) = &self.found else { return };
        if let Some(at) = f.caret() {
            self.editor.set_selection(Selection::caret(at));
            self.caret_moved(false);
        }
        self.reveal = false;
    }

    /// Opens the note at `rel` (vault-relative, with `.md`) from a shell's
    /// own search for `query`: on its first
    /// match, every match tinted.
    pub fn open_from_search(&mut self, rel: &str, query: &str) {
        self.open(rel);
        self.opened_from_search(query);
    }

    /// The tint goes; whether there was one.
    pub(super) fn clear_found(&mut self) -> bool {
        self.found.take().is_some()
    }

    /// The matches to tint in the note as it is now (none once it changed).
    pub(super) fn found_view(&self) -> &[(usize, usize)] {
        self.found.as_ref().map_or(&[][..], |f| f.ranges(self.editor.version()))
    }

    /// Once written in (or read again from its file), the tint is let go.
    pub(super) fn found_follows_text(&mut self) {
        if self.found.as_ref().is_some_and(|f| !f.holds(self.editor.version())) {
            self.found = None;
        }
    }

    /// The first frame after the note opened from search: the view goes to
    /// its first match. Whether it moved (the frame is laid out again).
    pub(super) fn center_found(&mut self, column_top: f32) -> bool {
        let Some(at) = self.found.as_mut().and_then(Found::take_center) else { return false };
        let Some((line, dy)) = found::view_at(&self.note_text(), at, column_top, self.viewport.1) else { return false };
        (self.scroll_line, self.scroll) = (line, 0.0);
        self.shift_scroll(dy);
        true
    }

    /// What the note was opened from a search for, while it shows what that
    /// found (for a shell that draws its own search).
    pub fn found_query(&self) -> Option<&str> {
        self.found.as_ref().filter(|f| f.holds(self.editor.version())).map(Found::query)
    }
}

#[cfg(test)]
mod tests;
