//! Back and forward through the notes visited, as on Windows (Alt+Left and
//! Alt+Right there; Cmd+[ and Cmd+] here, and the mouse's side buttons):
//! each note comes back where it was left, its view and its caret.

use super::*;

/// Steps back kept at most.
const HISTORY: usize = 64;

/// A note as it was left: its file (vault-relative), the paragraph at the
/// top of the view and how far that was scrolled, and the caret.
#[derive(Clone, Debug)]
pub(super) struct Visit {
    rel: String,
    scroll: (usize, f32),
    caret: usize,
}

#[derive(Default)]
pub(super) struct Visits {
    back: Vec<Visit>,
    ahead: Vec<Visit>,
    /// A step back or forward is under way (it is not a new visit).
    stepping: bool,
}

impl Model {
    fn visit(&self) -> Visit {
        Visit { rel: self.session.note.clone(), scroll: (self.scroll_line, self.scroll), caret: self.editor.selection().head }
    }

    /// The open note goes on the way back, as another opens (a new step
    /// forgets the way forward).
    pub(super) fn remember_visit(&mut self) {
        if self.visits.stepping || self.session.note.is_empty() || self.path.is_none() {
            return;
        }
        let visit = self.visit();
        self.visits.back.push(visit);
        if self.visits.back.len() > HISTORY {
            self.visits.back.remove(0);
        }
        self.visits.ahead.clear();
    }

    /// Back to the note left before (`forward`: the one gone back from),
    /// where it was left. Notes deleted or renamed since are skipped.
    pub fn step(&mut self, forward: bool) -> bool {
        loop {
            let next = if forward { self.visits.ahead.pop() } else { self.visits.back.pop() };
            let Some(v) = next else { return false };
            if !self.vault.join(&v.rel).is_file() {
                continue;
            }
            let here = (!self.session.note.is_empty() && self.path.is_some()).then(|| self.visit());
            self.visits.stepping = true;
            self.trash = None;
            self.search = None;
            self.open(&v.rel);
            self.visits.stepping = false;
            if let Some(here) = here {
                if forward { self.visits.back.push(here) } else { self.visits.ahead.push(here) }
            }
            let text = self.editor.text();
            let len = text.len();
            let mut caret = v.caret.min(len);
            while !text.is_char_boundary(caret) {
                caret -= 1;
            }
            self.editor.set_selection(Selection::caret(caret));
            (self.scroll_line, self.scroll) = (v.scroll.0.min(len), v.scroll.1);
            self.fix_scroll_line();
            self.reveal = false;
            return true;
        }
    }

    /// Opens a note as the page from outside the model (a note's own window
    /// handing it back, a link): whatever was over the tree closes.
    pub fn open_note(&mut self, rel: &str) -> bool {
        self.trash = None;
        self.search = None;
        self.open(rel);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_forward_return_to_each_note_where_it_was_left() {
        let dir = std::env::temp_dir().join(format!("kolada-app-visits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("A.md"), "# A\n\nFirst note, a line.\n").unwrap();
        std::fs::write(dir.join("B.md"), "# B\n\nSecond.\n").unwrap();
        std::fs::write(dir.join("C.md"), "# C\n\nThird.\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.open("A.md");
        // The note the model opened with is not part of this walk.
        m.visits = Default::default();
        m.go(Motion::DocEnd, false);
        let a_end = m.selection().head;
        m.open("B.md");
        m.open("C.md");
        assert!(m.step(false));
        assert_eq!(m.session.note, "B.md");
        assert!(m.step(false));
        assert_eq!(m.session.note, "A.md");
        assert_eq!(m.selection().head, a_end, "where the caret was left");
        assert!(!m.step(false), "nothing further back");
        assert!(m.step(true));
        assert!(m.step(true));
        assert_eq!(m.session.note, "C.md");
        assert!(!m.step(true));
        // A new visit forgets the way forward; a deleted note is skipped.
        m.step(false);
        m.open("A.md");
        assert!(!m.step(true));
        std::fs::remove_file(dir.join("B.md")).unwrap();
        assert!(m.step(false));
        // Back is now [A, B]: B is skipped, A (opened first) comes back.
        assert_eq!(m.session.note, "A.md");
        assert!(!m.step(false));
        let _ = std::fs::remove_dir_all(dir);
    }
}
