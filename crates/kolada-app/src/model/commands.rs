//! The text system's commands, by the names AppKit gives them (`moveLeft:`,
//! `deleteWordBackward:`, `insertNewline:`...): the Mac's key bindings send
//! them, and iOS's hardware keyboard is mapped onto the same names, so each
//! shell turns its keys into a name and the model does the rest.

use super::*;

impl Model {
    /// Does the command `name`; whether the screen changes. Moves come in
    /// pairs: plain, and `...AndModifySelection:` with Shift.
    pub fn command(&mut self, name: &str) -> bool {
        let (name, extend) = match name.strip_suffix("AndModifySelection:") {
            Some(base) => (format!("{base}:"), true),
            None => (name.to_string(), false),
        };
        // A drawn menu, then the Today card and the note's past, when open,
        // take the keys.
        if let Some(done) = self.menu_command(&name) {
            return done;
        }
        // Beam's menu open: Esc puts it away, and only that.
        if name == "cancelOperation:" && self.quick.menu.take().is_some() {
            return true;
        }
        // Browsing the grid, Esc goes back to the note (a search typed in
        // its bar is cleared first).
        if name == "cancelOperation:" && self.search.is_none() && self.grid_escape() {
            return true;
        }
        if let Some(done) = self.agenda_command(&name) {
            return done;
        }
        if let Some(done) = self.scrub_command(&name) {
            return done;
        }
        match name.as_str() {
            "deleteBackward:" | "deleteBackwardByDecomposingPreviousCharacter:" => self.backspace(false),
            "deleteWordBackward:" => self.backspace(true),
            "deleteForward:" => self.delete(false),
            "deleteWordForward:" => self.delete(true),
            "insertNewline:" | "insertParagraphSeparator:" => self.newline(false),
            "insertLineBreak:" | "insertNewlineIgnoringFieldEditor:" => self.newline(true),
            "insertTab:" => self.tab(false),
            "insertBacktab:" => self.tab(true),
            // AppKit's "left" and "right" are the screen's; "backward",
            // "forward", "beginning" and "end" the text's order, which a
            // right-to-left line turns around.
            "moveLeft:" => self.go(Motion::Left, extend),
            "moveRight:" => self.go(Motion::Right, extend),
            "moveBackward:" => self.go(self.in_order(false, Motion::Left, Motion::Right), extend),
            "moveForward:" => self.go(self.in_order(true, Motion::Left, Motion::Right), extend),
            "moveUp:" => self.go(Motion::Up, extend),
            "moveDown:" => self.go(Motion::Down, extend),
            "moveWordLeft:" => self.go(Motion::WordLeft, extend),
            "moveWordRight:" => self.go(Motion::WordRight, extend),
            "moveWordBackward:" => self.go(self.in_order(false, Motion::WordLeft, Motion::WordRight), extend),
            "moveWordForward:" => self.go(self.in_order(true, Motion::WordLeft, Motion::WordRight), extend),
            // The visual line's ends: Cmd+Left and Cmd+Right go to the
            // screen's side, which in a right-to-left line is its end.
            "moveToBeginningOfLine:" => self.go(Motion::LineStart, extend),
            "moveToEndOfLine:" => self.go(Motion::LineEnd, extend),
            "moveToLeftEndOfLine:" => self.go(if self.caret_rtl() { Motion::LineEnd } else { Motion::LineStart }, extend),
            "moveToRightEndOfLine:" => self.go(if self.caret_rtl() { Motion::LineStart } else { Motion::LineEnd }, extend),
            // The paragraph's ends (Ctrl+A, Ctrl+E), and from one to the
            // one before or after (Option+Shift+Up and Down).
            "moveToBeginningOfParagraph:" => self.go_paragraph(false, false, extend),
            "moveToEndOfParagraph:" => self.go_paragraph(true, false, extend),
            "moveParagraphBackward:" => self.go_paragraph(false, true, extend),
            "moveParagraphForward:" => self.go_paragraph(true, true, extend),
            "moveToBeginningOfDocument:" | "scrollToBeginningOfDocument:" => self.go(Motion::DocStart, extend),
            "moveToEndOfDocument:" | "scrollToEndOfDocument:" => self.go(Motion::DocEnd, extend),
            "pageUp:" | "scrollPageUp:" => self.page(false, extend),
            "pageDown:" | "scrollPageDown:" => self.page(true, extend),
            "selectAll:" => self.select_all(),
            // Cmd+Backspace and Cmd+Delete: to the visual line's start or
            // end, deleted; Control-K and its kin: to the paragraph's.
            "deleteToEndOfLine:" => self.delete_to(false),
            "deleteToBeginningOfLine:" => self.delete_to(true),
            "deleteToEndOfParagraph:" => self.delete_to_paragraph(false),
            "deleteToBeginningOfParagraph:" => self.delete_to_paragraph(true),
            // Esc right after a replacement or a correction takes it back.
            "cancelOperation:" => self.revert_typed() || self.escape(),
            // The rest: nothing to do yet.
            _ => false,
        }
    }

    /// Whether the note's line at the caret reads right to left (a field's
    /// or a note open over the page: as left to right).
    fn caret_rtl(&self) -> bool {
        use kolada_editor::Layout;
        !self.in_field() && self.popup.is_none() && self.note_text().rtl(self.editor.selection().head)
    }

    /// The screen's way for a step in the text's order: backward is to the
    /// right in a right-to-left line.
    fn in_order(&self, forward: bool, left: Motion, right: Motion) -> Motion {
        if forward != self.caret_rtl() { right } else { left }
    }

    /// To the start or end of the paragraph at the caret; `step`, from
    /// there, to the one before or after's (as NSTextView). A field or a
    /// note over the page has its line's ends.
    fn go_paragraph(&mut self, end: bool, step: bool, extend: bool) -> bool {
        if self.in_field() || self.popup.is_some() {
            return self.go(if end { Motion::LineEnd } else { Motion::LineStart }, extend);
        }
        let s = self.editor.selection();
        let to = paragraph_bound(self.editor.text(), s.head, end, step);
        self.editor.set_selection(Selection { anchor: if extend { s.anchor } else { to }, head: to });
        self.reveal = true;
        self.caret_moved(false);
        true
    }

    /// Control-K and its kin: to the paragraph's end or start, deleted; at
    /// it already, the line break (as on the Mac).
    fn delete_to_paragraph(&mut self, back: bool) -> bool {
        if self.in_field() || self.popup.is_some() || self.scrubbing() || self.settings.read_only {
            return self.delete_to(back);
        }
        let head = self.editor.selection().head;
        let to = paragraph_bound(self.editor.text(), head, !back, false);
        if to == head {
            return if back { self.backspace(false) } else { self.delete(false) };
        }
        self.editor.set_selection(Selection { anchor: head, head: to });
        self.edit(|e| e.backspace(false))
    }
}

/// The start (or end) of the paragraph holding `at`: the text between line
/// breaks. With `step`, from its start already, the one before's start
/// (from its end, the one after's end).
fn paragraph_bound(text: &str, at: usize, end: bool, step: bool) -> usize {
    let start_of = |i: usize| text[..i].rfind('\n').map_or(0, |n| n + 1);
    let end_of = |i: usize| text[i..].find('\n').map_or(text.len(), |n| i + n);
    if end {
        let e = end_of(at);
        if step && e == at && at < text.len() { end_of(at + 1) } else { e }
    } else {
        let s = start_of(at);
        if step && s == at && at > 0 { start_of(at - 1) } else { s }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_systems_commands_move_select_and_delete_by_name() {
        let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-commands", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), "# Plan\none two three").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(1000, 700, 1.0);
        m.command("moveToEndOfDocument:");
        assert_eq!(m.selection_utf16(), (20, 0));
        m.command("moveWordLeftAndModifySelection:");
        assert_eq!(m.selection_utf16(), (15, 5), "Shift extends");
        assert!(m.command("deleteBackward:"));
        assert!(m.text().ends_with("one two "));
        m.command("deleteWordBackward:");
        assert!(m.text().ends_with("one "), "{:?}", m.text().len());
        assert!(!m.command("noSuchCommand:"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Cmd+Backspace (`deleteToBeginningOfLine:`) takes the line back to its
    /// start and no further: the lines above and the note stay, and Cmd+Z
    /// puts it back. (It once sent the whole note to the trash: Move to
    /// Trash had it as its menu key.)
    #[test]
    fn cmd_backspace_deletes_to_the_lines_start_and_cmd_z_puts_it_back() {
        let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-cmd-backspace", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let text = "# Plan\n\nFirst line of the plan.\nSecond line, the one being typed.\n";
        std::fs::write(dir.join("Plan.md"), text).unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.open("Plan.md");
        m.frame(1000, 700, 1.0);
        let end = text.find("typed.").unwrap() + "typed.".len();
        m.editor.set_selection(kolada_editor::Selection::caret(end));
        assert!(m.command("deleteToBeginningOfLine:"));
        assert_eq!(m.text(), "# Plan\n\nFirst line of the plan.\n\n", "only the line, back to its start");
        assert_eq!(m.session.note, "Plan.md", "the note is still open");
        assert!(dir.join("Plan.md").is_file(), "and still in the vault");
        assert!(m.undo());
        assert_eq!(m.text(), text, "Cmd+Z puts the line back");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// AppKit's line, paragraph and word bindings, as NSTextView has them,
    /// in a wrapped paragraph left to right and one right to left: "left"
    /// and "right" are the screen's (Cmd+Left in an Arabic line goes to its
    /// end), "backward", "forward" and the paragraph's ends the text's
    /// order; Control-K and its kin delete to the paragraph's ends, Cmd+
    /// Backspace to the visual line's; Shift extends; Cmd+Z puts back.
    #[test]
    fn line_paragraph_and_word_bindings_go_as_appkit_has_them_both_ways() {
        let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-bindings", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let long = "word ".repeat(60) + "end";
        let arabic = "\u{643}\u{644}\u{645}\u{629} ".repeat(60) + "\u{646}\u{647}\u{627}\u{64a}\u{629}";
        let text = format!("# Keys\n\n{long}\nshort line\n{arabic}\nlast\n");
        std::fs::write(dir.join("Keys.md"), &text).unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.open("Keys.md");
        m.frame(1000, 700, 1.0);
        let at = |m: &Model| m.editor.selection().head;
        let set = |m: &mut Model, i: usize| m.editor.set_selection(Selection::caret(i));
        // Where a plain motion takes the caret from `i`.
        let by = |m: &mut Model, i: usize, motion: Motion| {
            set(m, i);
            m.go(motion, false);
            at(m)
        };
        let run = |m: &mut Model, i: usize, name: &str| {
            set(m, i);
            m.command(name);
            at(m)
        };

        // Left to right: the second visual line of a wrapped paragraph.
        let (p, p_end) = (text.find(&long).unwrap(), text.find(&long).unwrap() + long.len());
        let mid = p + 5 * 40;
        let (start, end) = (by(&mut m, mid, Motion::LineStart), by(&mut m, mid, Motion::LineEnd));
        assert!(start > p && end < p_end, "the paragraph wraps: {p} {start} {end} {p_end}");
        assert_eq!(run(&mut m, mid, "moveToLeftEndOfLine:"), start, "Cmd+Left: the visual line's start");
        assert_eq!(run(&mut m, mid, "moveToRightEndOfLine:"), end);
        assert_eq!(run(&mut m, mid, "moveToBeginningOfLine:"), start);
        assert_eq!(run(&mut m, mid, "moveToEndOfLine:"), end);
        assert_eq!(run(&mut m, mid, "moveToBeginningOfParagraph:"), p, "Control-A: the paragraph's start");
        assert_eq!(run(&mut m, mid, "moveToEndOfParagraph:"), p_end);
        assert_eq!(run(&mut m, p, "moveParagraphBackward:"), text.find("\n\n").unwrap() + 1, "from its start, the one before's");
        assert_eq!(run(&mut m, p_end, "moveParagraphForward:"), text.find("short line").unwrap() + "short line".len());
        assert_eq!(run(&mut m, mid, "moveBackward:"), mid - 1);
        assert_eq!(run(&mut m, mid, "moveForward:"), mid + 1);
        assert_eq!(run(&mut m, mid, "moveLeft:"), mid - 1);
        assert_eq!(run(&mut m, mid + 2, "moveWordBackward:"), mid);
        set(&mut m, mid);
        m.command("moveToEndOfParagraphAndModifySelection:");
        assert_eq!((m.editor.selection().anchor, at(&m)), (mid, p_end), "Shift extends");
        // Deleting: to the paragraph's end (Control-K), not the next one's;
        // to the visual line's start (Cmd+Backspace), not the paragraph's.
        set(&mut m, mid);
        assert!(m.command("deleteToEndOfParagraph:"));
        assert_eq!(m.text(), format!("# Keys\n\n{}\nshort line\n{arabic}\nlast\n", &long[..mid - p]));
        assert!(m.undo());
        assert_eq!(m.text(), text);
        set(&mut m, mid);
        assert!(m.command("deleteToBeginningOfLine:"));
        assert_eq!(m.text().len(), text.len() - (mid - start), "to the visual line's start only");
        assert!(m.undo());
        set(&mut m, mid);
        assert!(m.command("deleteToBeginningOfParagraph:"));
        assert_eq!(m.text(), format!("# Keys\n\n{}\nshort line\n{arabic}\nlast\n", &long[mid - p..]));
        assert!(m.undo());
        assert_eq!(m.text(), text, "Cmd+Z puts each back");

        // Right to left: the screen's left is the line's end.
        let (r, r_end) = (text.find(&arabic).unwrap(), text.find(&arabic).unwrap() + arabic.len());
        let rmid = r + "\u{643}\u{644}\u{645}\u{629} ".len() * 40;
        let (start, end) = (by(&mut m, rmid, Motion::LineStart), by(&mut m, rmid, Motion::LineEnd));
        assert!(start > r && end < r_end, "the Arabic paragraph wraps: {r} {start} {end} {r_end}");
        assert_eq!(run(&mut m, rmid, "moveToLeftEndOfLine:"), end, "Cmd+Left in an Arabic line: its end, on the left");
        assert_eq!(run(&mut m, rmid, "moveToRightEndOfLine:"), start);
        assert_eq!(run(&mut m, rmid, "moveToBeginningOfLine:"), start, "the beginning is the text's");
        assert_eq!(run(&mut m, rmid, "moveToBeginningOfParagraph:"), r);
        assert_eq!(run(&mut m, rmid, "moveToEndOfParagraph:"), r_end);
        assert_eq!(run(&mut m, rmid, "moveBackward:"), rmid - 1, "backward: before the space, in the text");
        assert_eq!(run(&mut m, rmid, "moveForward:"), rmid + 2);
        assert_eq!(run(&mut m, rmid, "moveLeft:"), rmid + 2, "left on the screen: on in the text");
        assert_eq!(run(&mut m, rmid, "moveRight:"), rmid - 1);
        set(&mut m, rmid);
        m.command("moveToLeftEndOfLineAndModifySelection:");
        assert_eq!((m.editor.selection().anchor, at(&m)), (rmid, end), "Shift extends toward the left");
        let _ = std::fs::remove_dir_all(dir);
    }
}
