//! Beam's send field: what is typed, edited as a text field is on every OS
//! (the caret anywhere, a selection by the pointer or Shift, a word by a
//! double click, all of it by a triple one, undo), and an input method's
//! composition shown in it until it settles. The shells turn their keys
//! into `Key`s, and move text between it and their clipboards.

use kolada_editor::{Editor, Layout, Motion, Selection};

/// What a key does in the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    LineStart,
    LineEnd,
    Start,
    End,
    Backspace,
    Delete,
    /// Backspace and Delete a word at a time.
    WordBackspace,
    WordDelete,
    SelectAll,
    Undo,
    Redo,
}

pub struct Draft {
    e: Editor,
    /// Where an input method's text being composed starts (it runs to the
    /// caret), until it settles.
    marked: Option<usize>,
}

impl Default for Draft {
    fn default() -> Draft {
        Draft { e: Editor::field(""), marked: None }
    }
}

impl std::fmt::Debug for Draft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.text())
    }
}

impl From<&str> for Draft {
    fn from(text: &str) -> Draft {
        let mut d = Draft { e: Editor::field(text), marked: None };
        d.e.set_selection(Selection::caret(text.len()));
        d
    }
}

impl From<String> for Draft {
    fn from(text: String) -> Draft {
        Draft::from(text.as_str())
    }
}

impl PartialEq<&str> for Draft {
    fn eq(&self, other: &&str) -> bool {
        self.text() == *other
    }
}

/// The field's lines are its own lines (a beam is a few lines at most):
/// Up and Down move between them, keeping the column.
struct Lines<'a>(&'a str);

impl Lines<'_> {
    fn line(&self, at: usize) -> (usize, usize) {
        let start = self.0[..at].rfind('\n').map_or(0, |i| i + 1);
        let end = self.0[at..].find('\n').map_or(self.0.len(), |i| at + i);
        (start, end)
    }

    /// The offset `x` characters into the line from `start` to `end`.
    fn at_x(&self, (start, end): (usize, usize), x: f32) -> usize {
        self.0[start..end].char_indices().nth(x.max(0.0) as usize).map_or(end, |(i, _)| start + i)
    }
}

impl Layout for Lines<'_> {
    fn visual_line(&self, offset: usize) -> (usize, usize) {
        self.line(offset)
    }

    fn x_of(&self, offset: usize) -> f32 {
        let (start, _) = self.line(offset);
        self.0[start..offset].chars().count() as f32
    }

    fn above(&self, offset: usize, x: f32) -> Option<usize> {
        let (start, _) = self.line(offset);
        (start > 0).then(|| self.at_x(self.line(start - 1), x))
    }

    fn below(&self, offset: usize, x: f32) -> Option<usize> {
        let (_, end) = self.line(offset);
        (end < self.0.len()).then(|| self.at_x(self.line(end + 1), x))
    }

    fn rtl(&self, offset: usize) -> bool {
        let (start, end) = self.line(offset);
        crate::scene::is_rtl(&self.0[start..end])
    }
}

impl Draft {
    pub fn text(&self) -> &str {
        self.e.text()
    }

    pub fn is_empty(&self) -> bool {
        self.e.text().is_empty()
    }

    /// The selection: its anchor and its head (the caret), byte offsets.
    pub fn selection(&self) -> (usize, usize) {
        let s = self.e.selection();
        (s.anchor, s.head)
    }

    /// Emptied (beamed): nothing to undo back into.
    pub fn clear(&mut self) {
        *self = Draft::default();
    }

    /// Text typed or pasted, over the selection (and over what was being
    /// composed, which it settles).
    pub fn insert(&mut self, text: &str) -> bool {
        self.take_marked();
        if text.is_empty() && self.e.selection().is_caret() {
            return false;
        }
        self.e.insert(text);
        true
    }

    /// A key: whether the field changed (its text or its selection).
    /// `extend`: Shift is held, so a move takes the selection along.
    pub fn key(&mut self, key: Key, extend: bool) -> bool {
        let before = (self.e.version(), self.e.selection());
        let motion = |m: Motion, d: &mut Draft| {
            let plan = d.e.plan(m, extend, &Lines(d.e.text()));
            d.e.apply(plan);
        };
        match key {
            Key::Left => motion(Motion::Left, self),
            Key::Right => motion(Motion::Right, self),
            Key::WordLeft => motion(Motion::WordLeft, self),
            Key::WordRight => motion(Motion::WordRight, self),
            Key::Up => motion(Motion::Up, self),
            Key::Down => motion(Motion::Down, self),
            Key::LineStart => motion(Motion::LineStart, self),
            Key::LineEnd => motion(Motion::LineEnd, self),
            Key::Start => motion(Motion::DocStart, self),
            Key::End => motion(Motion::DocEnd, self),
            Key::Backspace => self.e.backspace(false),
            Key::Delete => self.e.delete(false),
            Key::WordBackspace => self.e.backspace(true),
            Key::WordDelete => self.e.delete(true),
            Key::SelectAll => self.e.select_all(),
            Key::Undo => {
                self.e.undo();
            }
            Key::Redo => {
                self.e.redo();
            }
        }
        (self.e.version(), self.e.selection()) != before
    }

    /// The words selected, for the clipboard (Copy); None with none.
    pub fn copy(&self) -> Option<String> {
        let s = self.e.selected_text();
        (!s.is_empty()).then(|| s.to_string())
    }

    /// Cut: the words selected, taken out.
    pub fn cut(&mut self) -> Option<String> {
        let s = self.copy()?;
        self.e.insert("");
        Some(s)
    }

    /// The button went down at byte `at`: the caret there (with Shift, the
    /// selection to there); a double click takes its word, a triple all.
    pub fn press(&mut self, at: usize, clicks: u32, extend: bool) {
        let at = self.snap(at);
        match clicks {
            2 => self.e.select_word_at(at),
            n if n >= 3 => self.e.select_all(),
            _ if extend => {
                let anchor = self.e.selection().anchor;
                self.e.set_selection(Selection { anchor, head: at });
            }
            _ => self.e.set_selection(Selection::caret(at)),
        }
    }

    /// The pointer dragged to byte `at` with the button down: the
    /// selection runs from where it went down to there.
    pub fn drag(&mut self, at: usize) {
        let at = self.snap(at);
        let anchor = self.e.selection().anchor;
        self.e.set_selection(Selection { anchor, head: at });
    }

    fn snap(&self, at: usize) -> usize {
        let t = self.e.text();
        let mut at = at.min(t.len());
        while !t.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    /// An input method's text being composed: it takes the selection's
    /// place, and is replaced as it changes; empty, it is taken back.
    pub fn mark(&mut self, text: &str) -> bool {
        let start = match self.marked {
            Some(start) => {
                let head = self.e.selection().head;
                self.e.set_selection(Selection { anchor: start, head });
                start
            }
            None => self.e.selection().start(),
        };
        self.e.insert(text);
        self.marked = (!text.is_empty()).then_some(start);
        true
    }

    /// The input method let the composition stand as it is.
    pub fn unmark(&mut self) -> bool {
        self.marked.take().is_some()
    }

    /// What was being composed, taken out: whether there was any.
    pub fn take_marked(&mut self) -> bool {
        let Some(start) = self.marked.take() else { return false };
        let head = self.e.selection().head;
        self.e.set_selection(Selection { anchor: start, head });
        self.e.insert("");
        true
    }

    pub fn has_marked(&self) -> bool {
        self.marked.is_some()
    }

    /// The selection and what is being composed, as UTF-16 offsets and
    /// lengths (an input method's measure).
    pub fn utf16(&self) -> ((usize, usize), Option<(usize, usize)>) {
        let t = self.e.text();
        let u = |at: usize| t[..at].encode_utf16().count();
        let s = self.e.selection();
        let selected = (u(s.start()), u(s.end()) - u(s.start()));
        let marked = self.marked.map(|m| (u(m), u(s.head) - u(m)));
        (selected, marked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_moves_selects_and_deletes_as_a_text_field_does() {
        let mut d = Draft::from("Gate B, row 14");
        assert_eq!(d.selection(), (14, 14), "the caret at the end");
        assert!(d.key(Key::WordLeft, false));
        assert_eq!(d.selection(), (12, 12));
        assert!(d.key(Key::WordLeft, true) && d.copy().as_deref() == Some("row "), "{:?}", d.copy());
        assert_eq!(d.cut().as_deref(), Some("row "));
        assert_eq!(d, "Gate B, 14");
        assert!(d.key(Key::Undo, false));
        assert_eq!(d, "Gate B, row 14", "undone");
        assert!(d.key(Key::Redo, false));
        assert_eq!(d, "Gate B, 14");
        assert!(d.key(Key::SelectAll, false));
        assert!(d.insert("Seat 22A"));
        assert_eq!(d, "Seat 22A", "typing replaces the selection");
        d.key(Key::Start, false);
        assert!(d.key(Key::Delete, false));
        assert_eq!(d, "eat 22A");
        d.key(Key::End, false);
        assert!(d.key(Key::WordBackspace, false));
        assert_eq!(d, "eat ");
    }

    #[test]
    fn up_and_down_move_between_lines_keeping_the_column() {
        let mut d = Draft::from("Gate B\nrow 14\nSeat 22A");
        d.key(Key::Start, false);
        d.key(Key::Right, false);
        d.key(Key::Right, false);
        assert!(d.key(Key::Down, false));
        assert_eq!(d.selection().1, "Gate B\nro".len());
        assert!(d.key(Key::Down, true));
        assert_eq!(d.copy().as_deref(), Some("w 14\nSe"));
        assert!(d.key(Key::Up, false));
        assert!(!d.key(Key::Up, false) || d.selection().1 <= "Gate B".len());
    }

    #[test]
    fn the_pointer_places_the_caret_drags_a_selection_and_double_clicks_a_word() {
        let mut d = Draft::from("Door code 4931");
        d.press(5, 1, false);
        assert_eq!(d.selection(), (5, 5));
        d.drag(9);
        assert_eq!(d.copy().as_deref(), Some("code"));
        d.press(0, 1, true);
        assert_eq!(d.selection(), (5, 0), "Shift: from where it was");
        d.press(11, 2, false);
        assert_eq!(d.copy().as_deref(), Some("4931"));
        d.press(1, 3, false);
        assert_eq!(d.copy().as_deref(), Some("Door code 4931"));
        d.press(99, 1, false);
        assert_eq!(d.selection(), (14, 14), "past the end: the end");
    }

    #[test]
    fn an_input_method_composes_over_the_selection_and_settles_once() {
        let mut d = Draft::from("Gate X");
        d.press(5, 1, false);
        d.drag(6);
        d.mark("\u{306b}");
        d.mark("\u{306b}\u{307b}");
        assert_eq!(d, "Gate \u{306b}\u{307b}", "in the selection's place, replaced as it changes");
        assert_eq!(d.utf16().1, Some((5, 2)));
        assert!(d.insert("\u{65e5}"));
        assert_eq!(d, "Gate \u{65e5}", "settled once, in its place");
        d.mark("x");
        assert!(d.take_marked());
        assert_eq!(d, "Gate \u{65e5}", "taken back whole");
        assert!(!d.has_marked());
    }
}
