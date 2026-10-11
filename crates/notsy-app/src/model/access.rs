//! What a screen reader asks of the note: its text, its lines, what is in
//! view and where a range is drawn, in UTF-16 units as the OSes count them.
//! The Mac's NSAccessibility asks this (VoiceOver); iOS's UIAccessibility and
//! Windows' UI Automation ask the same. Lines are the note's own lines, as
//! typed: VoiceOver reads a wrapped paragraph as one line, as it does in
//! TextEdit with wrapping off.

use super::*;

impl Model {
    /// What a screen reader calls the note: its name.
    pub fn access_label(&self) -> String {
        let note = &self.session.note;
        let name = note.rsplit(['/', '\\']).next().unwrap_or(note);
        name.strip_suffix(".md").unwrap_or(name).to_string()
    }

    /// The text a screen reader reads: the note, or the one open over it.
    pub fn access_text(&self) -> &str {
        self.active().text()
    }

    /// The text's length in UTF-16 units.
    pub fn access_len(&self) -> usize {
        let text = self.access_text();
        utf16_at(text, text.len())
    }

    /// The line (from 0) that holds UTF-16 position `at`.
    pub fn access_line_of(&self, at: usize) -> usize {
        let text = self.access_text();
        let b = byte_at(text, at.min(self.access_len()));
        text.as_bytes()[..b].iter().filter(|&&c| c == b'\n').count()
    }

    /// Line `line`'s range, UTF-16 start and length, its newline included;
    /// None past the last line.
    pub fn access_line_range(&self, line: usize) -> Option<(usize, usize)> {
        let text = self.access_text();
        let mut start = 0;
        for _ in 0..line {
            start += text[start..].find('\n')? + 1;
        }
        let end = text[start..].find('\n').map_or(text.len(), |i| start + i + 1);
        let s = utf16_at(text, start);
        Some((s, utf16_at(text, end) - s))
    }

    /// The character at UTF-16 position `at`, start and length: two units
    /// for one outside the Basic Multilingual Plane (most emoji), so a
    /// screen reader never splits one.
    pub fn access_char_at(&self, at: usize) -> (usize, usize) {
        let text = self.access_text();
        let mut unit = 0;
        for c in text.chars() {
            let n = c.len_utf16();
            if at < unit + n {
                return (unit, n);
            }
            unit += n;
        }
        (unit, 0)
    }

    /// What the window shows of the note, UTF-16 start and length: the
    /// paragraphs the last frame drew.
    pub fn access_visible(&self) -> (usize, usize) {
        if self.popup.is_some() {
            return (0, self.access_len());
        }
        let text = self.editor.text();
        let (Some(first), Some(last)) = (self.lines.first(), self.lines.last()) else { return (0, 0) };
        let s = utf16_at(text, first.para.start.min(text.len()));
        (s, utf16_at(text, last.para.end.min(text.len())) - s)
    }

    /// Where a range is drawn, DIPs from the view's top left: across the
    /// column, over the paragraphs it is in (the caret's box for an empty
    /// one at the caret); None when none of it is in view.
    pub fn access_rect(&self, start: usize, len: usize) -> Option<Rect> {
        if self.popup.is_some() {
            return self.caret_rect();
        }
        let text = self.editor.text();
        let (a, b) = (byte_at(text, start), byte_at(text, start + len));
        if a == b && Some(a) == Some(self.editor.selection().head) {
            if let Some(caret) = self.caret {
                return Some(caret);
            }
        }
        self.lines
            .iter()
            .filter(|l| l.para.start <= b && a <= l.para.end)
            .map(|l| Rect::new(l.x, l.y, l.x + self.column_width, l.y + l.height))
            .reduce(|r, s| Rect::new(r.left.min(s.left), r.top.min(s.top), r.right.max(s.right), r.bottom.max(s.bottom)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(test: &str, text: &str) -> (PathBuf, Model) {
        let dir = std::env::temp_dir().join(format!("notsy-app-test-{}-access-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), text).unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        (dir, m)
    }

    #[test]
    fn a_screen_reader_reads_the_note_by_its_lines_in_utf16() {
        // "é" is one unit, the emoji two: the OS counts UTF-16, as the input methods do.
        let (dir, m) = note("lines", "# Plan\nCaf\u{E9} \u{1F426}\nLast");
        assert_eq!(m.access_label(), "Plan");
        assert_eq!(m.access_len(), 7 + 8 + 4);
        assert_eq!(m.access_line_range(0), Some((0, 7)));
        assert_eq!(m.access_line_range(1), Some((7, 8)), "the newline is the line's, the emoji two units");
        assert_eq!(m.access_line_range(2), Some((15, 4)));
        assert_eq!(m.access_line_range(3), None);
        assert_eq!((m.access_line_of(0), m.access_line_of(7), m.access_line_of(14), m.access_line_of(18), m.access_line_of(99)), (0, 1, 1, 2, 2));
        assert_eq!(m.access_text(), "# Plan\nCaf\u{E9} \u{1F426}\nLast");
        assert_eq!((m.access_char_at(11), m.access_char_at(12), m.access_char_at(13), m.access_char_at(99)), ((11, 1), (12, 2), (12, 2), (19, 0)), "the bird is one character of two units");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_screen_reader_knows_what_is_in_view_and_where_a_line_is_drawn() {
        let long: String = (0..400).map(|i| format!("Line {i}\n")).collect();
        let (dir, m) = note("view", &format!("# Plan\n{long}"));
        let (start, len) = m.access_visible();
        assert_eq!(start, 0, "the top of the note is in view");
        assert!(len > 0 && len < m.access_len(), "and not all of a long note: {len} of {}", m.access_len());
        let (l1, n1) = m.access_line_range(1).unwrap();
        let r = m.access_rect(l1, n1).expect("the first line is drawn");
        assert!(r.top >= 0.0 && r.bottom <= 720.0 && r.right > r.left, "{r:?}");
        let (far, n) = m.access_line_range(390).unwrap();
        assert_eq!(m.access_rect(far, n), None, "a line out of view has no place");
        let caret = m.access_rect(m.selection_utf16().0, 0).expect("the caret is drawn");
        assert!(caret.right - caret.left < 10.0, "the caret's own box: {caret:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
