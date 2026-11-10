//! A note opened from search opens on what it found (`kolada_ui::found`):
//! scrolled so its first match sits a little above the middle of the
//! window, every match tinted as marked text is. Found once as the note
//! opens; the tint goes with the note's first edit, another note opening,
//! the search being closed, or Esc, and stays through clicks and caret
//! moves. As kolada-app's `model/found.rs` has it for the Mac and Linux.

use kolada_ui::found::{self, Found};

use super::*;

/// What one Esc puts away of what a search left showing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Away {
    /// The tint in the note it opened.
    Tint,
    /// The search: typed in, or kept open beside the note.
    Search,
}

/// The tint first (the search, or the grid's boxes behind it, stay), then
/// the search; the grid's boxes with the Esc after (`grid_escape`). As
/// kolada-app's `escape` has it.
pub(super) fn esc_away(tinted: bool, searching: bool) -> Option<Away> {
    if tinted {
        Some(Away::Tint)
    } else if searching {
        Some(Away::Search)
    } else {
        None
    }
}

impl App {
    /// The note just opened came from a search for `query`: what it found
    /// there, the caret at the first match, and the view put on it as the
    /// next frame is drawn.
    pub(super) fn opened_from_search(&mut self, query: &str) {
        self.found = Found::opened(self.editor.text(), self.editor.outline(), query, self.editor.version());
        let Some(f) = &self.found else { return };
        if let Some(at) = f.caret() {
            self.editor.set_selection(Selection::caret(at));
            self.caret_moved(false);
        }
        self.reveal_caret = false;
        self.request_frame();
    }

    /// The tint goes; whether there was one.
    pub(super) fn clear_found(&mut self) -> bool {
        let had = self.found.take().is_some();
        if had {
            self.request_frame();
        }
        had
    }

    /// The matches to tint in the note as it is now (none once it changed).
    pub(super) fn found_view(&self) -> &[(usize, usize)] {
        self.found.as_ref().map_or(&[][..], |f| f.ranges(self.editor.version()))
    }

    /// The frame after the note opened from search: the view goes to its
    /// first match. Whether it moved (the scene is built again).
    pub(super) fn center_found(&mut self, scene: &Scene, (_, height, dpi): (u32, u32, u32)) -> bool {
        if self.found.as_ref().is_some_and(|f| !f.holds(self.editor.version())) {
            self.found = None;
        }
        let Some(at) = self.found.as_mut().and_then(Found::take_center) else { return false };
        let height = height as f32 * 96.0 / dpi as f32;
        let Some((line, dy)) = found::view_at(&self.note_text(), at, scene.column.top, height) else { return false };
        self.session.scroll_line = line as u64;
        self.session.scroll_px = 0.0;
        self.scroll_pending = 0.0;
        self.scroll_by(dy);
        true
    }

    /// The search's query, when the grid's boxes are its matches.
    pub(super) fn grid_query(&self) -> Option<String> {
        let searching = self.grid.as_ref().is_some_and(|g| g.notes.searching());
        self.search.as_ref().filter(|s| searching && s.commands.is_none()).map(|s| s.query.text().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{Away, esc_away};

    /// A note opened from search, tinted, with the search kept open (or the
    /// grid's boxes behind it): one Esc takes the tint, the next the search;
    /// the grid's boxes go with the Esc after (`grid_escape`).
    #[test]
    fn esc_takes_the_tint_first_then_the_search() {
        assert_eq!(esc_away(true, true), Some(Away::Tint), "the tint, the search kept");
        assert_eq!(esc_away(false, true), Some(Away::Search), "then the search");
        assert_eq!(esc_away(true, false), Some(Away::Tint), "the grid browsed: the tint before its boxes");
        assert_eq!(esc_away(false, false), None, "then the grid's own way, as before");
    }
}
