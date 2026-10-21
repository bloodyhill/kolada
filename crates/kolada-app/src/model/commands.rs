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
            "moveLeft:" | "moveBackward:" => self.go(Motion::Left, extend),
            "moveRight:" | "moveForward:" => self.go(Motion::Right, extend),
            "moveUp:" => self.go(Motion::Up, extend),
            "moveDown:" => self.go(Motion::Down, extend),
            "moveWordLeft:" | "moveWordBackward:" => self.go(Motion::WordLeft, extend),
            "moveWordRight:" | "moveWordForward:" => self.go(Motion::WordRight, extend),
            "moveToBeginningOfLine:" | "moveToLeftEndOfLine:" | "moveToBeginningOfParagraph:" => self.go(Motion::LineStart, extend),
            "moveToEndOfLine:" | "moveToRightEndOfLine:" | "moveToEndOfParagraph:" => self.go(Motion::LineEnd, extend),
            "moveToBeginningOfDocument:" | "scrollToBeginningOfDocument:" => self.go(Motion::DocStart, extend),
            "moveToEndOfDocument:" | "scrollToEndOfDocument:" => self.go(Motion::DocEnd, extend),
            "pageUp:" | "scrollPageUp:" => self.page(false, extend),
            "pageDown:" | "scrollPageDown:" => self.page(true, extend),
            "selectAll:" => self.select_all(),
            // Control-K and their kin: to the line's end or start, deleted.
            "deleteToEndOfLine:" | "deleteToEndOfParagraph:" => self.delete_to(false),
            "deleteToBeginningOfLine:" | "deleteToBeginningOfParagraph:" => self.delete_to(true),
            "cancelOperation:" => self.escape(),
            // The rest: nothing to do yet.
            _ => false,
        }
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
}
