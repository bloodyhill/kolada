//! Scrolling the note, as Windows does it: from a paragraph, not from the
//! top. The view is `scroll_line` (the start of the paragraph at its top)
//! and `scroll` (DIPs below that paragraph's start), so a frame measures
//! only what it draws; counted from the top, every frame (every keystroke
//! in a 6,000-line note) measured every paragraph above it: 3.4 ms.

use super::*;

/// The least the last paragraph keeps in view when scrolled to the end.
const END_LINE: f32 = 24.0;

impl Model {
    /// Keeps the top of the view on a paragraph start after the text changed under it.
    pub(super) fn fix_scroll_line(&mut self) {
        let text = self.editor.text();
        let mut at = self.scroll_line.min(text.len());
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        self.scroll_line = text[..at].rfind('\n').map_or(0, |i| i + 1);
    }

    /// Moves the view by `dy` DIPs, paragraph by paragraph, stopping at the
    /// top and at the last paragraph.
    pub(super) fn shift_scroll(&mut self, dy: f32) {
        self.fix_scroll_line();
        let (mut line, mut px) = (self.scroll_line, self.scroll + dy);
        {
            let nt = self.note_text();
            loop {
                if px < 0.0 {
                    if line == 0 {
                        px = 0.0;
                        break;
                    }
                    let Ok(prev) = nt.para_at(line - 1) else { break };
                    line = prev.start;
                    px += nt.block_height(&prev).unwrap_or(0.0);
                } else {
                    let Ok(p) = nt.para_at(line) else { break };
                    let h = nt.block_height(&p).unwrap_or(0.0);
                    if p.end >= nt.text.len() {
                        px = px.min((h - END_LINE).max(0.0));
                        break;
                    }
                    if px < h {
                        break;
                    }
                    px -= h;
                    line = p.end + 1;
                }
            }
        }
        (self.scroll_line, self.scroll) = (line, px);
    }

    /// Scrolls the note by `dy` DIPs (positive: further down), not past
    /// where the end of the note reaches the bottom; whether it moved.
    pub fn scroll_by(&mut self, dy: f32) -> bool {
        if self.settings.open {
            return self.scroll_options(dy);
        }
        if dy > 0.0 && self.at_end {
            return false;
        }
        let before = (self.scroll_line, self.scroll);
        self.shift_scroll(dy);
        (self.scroll_line, self.scroll) != before
    }

    /// The caret was not drawn: puts the view on its paragraph, the caret's
    /// line at the top when it is above the view and at the bottom when it is
    /// below. False if the caret has no paragraph.
    pub(super) fn scroll_to_caret(&mut self, column_top: f32, bottom: f32) -> bool {
        let head = self.editor.selection().head;
        let target = {
            let nt = self.note_text();
            let Ok(p) = nt.para_at(head) else { return false };
            let Ok((_, y, h)) = nt.caret(&p, head) else { return false };
            let above = head < self.scroll_line;
            (p.start, if above { p.pad + y } else { p.pad + y + h - (bottom - column_top) })
        };
        self.scroll_line = target.0;
        self.scroll = 0.0;
        self.shift_scroll(target.1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long(test: &str) -> (PathBuf, Model) {
        let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let body: String = (0..400).map(|i| format!("Paragraph {i} of a note that goes on.\n")).collect();
        std::fs::write(dir.join("Long.md"), body).unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        (dir, m)
    }

    #[test]
    fn the_view_rests_on_a_paragraph_and_scrolls_back_exactly() {
        let (dir, mut m) = long("anchor");
        assert!(m.scroll_by(1000.0));
        m.frame(2000, 1440, 2.0);
        assert!(m.scroll_line > 0, "the top of the view is a paragraph down the note");
        assert!(m.scroll >= 0.0 && m.scroll < 60.0, "and only part of one above it: {}", m.scroll);
        let text = m.text();
        assert!(m.scroll_line == 0 || text.as_bytes()[m.scroll_line - 1] == b'\n', "at a paragraph's start");
        assert!(m.scroll_by(-1000.0));
        assert!(m.scroll_line == 0 && m.scroll < 0.01, "back at the top: {} {}", m.scroll_line, m.scroll);
        m.scroll_by(-5000.0);
        assert_eq!((m.scroll_line, m.scroll), (0, 0.0), "and exactly there when pushed against it");
        assert!(!m.scroll_by(-30.0), "and no further");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_caret_far_below_or_above_is_brought_into_view_in_one_go() {
        let (dir, mut m) = long("reveal");
        m.go(Motion::DocEnd, false);
        m.frame(2000, 1440, 2.0);
        let c = m.caret_rect().expect("the caret is drawn at the end");
        assert!(c.top >= 0.0 && c.bottom <= m.viewport.1, "{c:?}");
        let end = (m.scroll_line, m.scroll);
        m.go(Motion::DocStart, false);
        m.frame(2000, 1440, 2.0);
        assert_eq!((m.scroll_line, m.scroll), (0, 0.0));
        m.go(Motion::DocEnd, false);
        m.frame(2000, 1440, 2.0);
        assert_eq!((m.scroll_line, m.scroll), end, "the same place each time");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_text_change_under_the_view_keeps_it_on_a_paragraph_start() {
        let (dir, mut m) = long("fix");
        m.scroll_by(1000.0);
        m.frame(2000, 1440, 2.0);
        // The offset now points mid-word.
        m.scroll_line += 3;
        m.fix_scroll_line();
        let text = m.text();
        assert!(m.scroll_line == 0 || text.as_bytes()[m.scroll_line - 1] == b'\n');
        let _ = std::fs::remove_dir_all(dir);
    }
}
