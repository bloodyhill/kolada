//! What a key does on Linux: the keys and shortcuts Windows has (Ctrl is
//! the command key), turned into the model's calls. The text system's keys
//! become the commands the model takes by AppKit's names (`moveWordLeft:`,
//! `deleteBackward:`...: `kolada-app` `model/commands.rs`), the rest the
//! app's own actions. Kept free of winit so it is tested as plain data.

use kolada_ui::scene::Tool;

/// The keys that are not text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Backspace,
    Delete,
    Enter,
    Tab,
    Escape,
    Insert,
    F3,
    F10,
    /// The menu key.
    Menu,
}

/// A key as pressed: a named key, or the character it types without
/// modifiers (`a` for Ctrl+Shift+A, `8` for Ctrl+Shift+8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key<'a> {
    Named(Named),
    Char(&'a str),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// The app's own actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Do {
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    PastePlain,
    NewNote,
    Search,
    Palette,
    Find,
    Replace,
    FindNext,
    FindPrevious,
    Today,
    Past,
    SideMenu,
    Toolbar,
    Settings,
    Back,
    Forward,
    /// Ctrl+Enter: the line's task checked off, or the line made a task.
    Task,
    Save,
    Quit,
    /// The menu key, or Shift+F10: the text's menu at the caret.
    Menu,
}

/// What a key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// A text-system command, by AppKit's name.
    Command(&'static str),
    Do(Do),
    Tool(Tool),
    /// A heading of this level; 0 for body text.
    Heading(u8),
}

/// A move, or with Shift the same move extending the selection.
fn moving(plain: &'static str, extend: &'static str, shift: bool) -> Action {
    Action::Command(if shift { extend } else { plain })
}

/// What `key` with `mods` does; None when it is no shortcut (its text, if
/// any, is typed).
pub fn action(key: Key, m: Mods) -> Option<Action> {
    use Named::*;
    let (ctrl, shift, alt) = (m.ctrl, m.shift, m.alt);
    Some(match key {
        Key::Named(Left) if alt && !ctrl => Action::Do(Do::Back),
        Key::Named(Right) if alt && !ctrl => Action::Do(Do::Forward),
        Key::Named(Left) if ctrl => moving("moveWordLeft:", "moveWordLeftAndModifySelection:", shift),
        Key::Named(Left) => moving("moveLeft:", "moveLeftAndModifySelection:", shift),
        Key::Named(Right) if ctrl => moving("moveWordRight:", "moveWordRightAndModifySelection:", shift),
        Key::Named(Right) => moving("moveRight:", "moveRightAndModifySelection:", shift),
        Key::Named(Up) => moving("moveUp:", "moveUpAndModifySelection:", shift),
        Key::Named(Down) => moving("moveDown:", "moveDownAndModifySelection:", shift),
        Key::Named(Home) if ctrl => moving("moveToBeginningOfDocument:", "moveToBeginningOfDocumentAndModifySelection:", shift),
        Key::Named(Home) => moving("moveToBeginningOfLine:", "moveToBeginningOfLineAndModifySelection:", shift),
        Key::Named(End) if ctrl => moving("moveToEndOfDocument:", "moveToEndOfDocumentAndModifySelection:", shift),
        Key::Named(End) => moving("moveToEndOfLine:", "moveToEndOfLineAndModifySelection:", shift),
        Key::Named(PageUp) => moving("pageUp:", "pageUpAndModifySelection:", shift),
        Key::Named(PageDown) => moving("pageDown:", "pageDownAndModifySelection:", shift),
        Key::Named(Backspace) => Action::Command(if ctrl { "deleteWordBackward:" } else { "deleteBackward:" }),
        Key::Named(Delete) if shift && !ctrl => Action::Do(Do::Cut),
        Key::Named(Delete) => Action::Command(if ctrl { "deleteWordForward:" } else { "deleteForward:" }),
        Key::Named(Enter) if ctrl => Action::Do(Do::Task),
        Key::Named(Enter) => Action::Command(if shift { "insertLineBreak:" } else { "insertNewline:" }),
        Key::Named(Tab) if ctrl => return None,
        Key::Named(Tab) => Action::Command(if shift { "insertBacktab:" } else { "insertTab:" }),
        Key::Named(Escape) => Action::Command("cancelOperation:"),
        Key::Named(Insert) if ctrl => Action::Do(Do::Copy),
        Key::Named(Insert) if shift => Action::Do(Do::Paste),
        Key::Named(Insert) => return None,
        Key::Named(F3) => Action::Do(if shift { Do::FindPrevious } else { Do::FindNext }),
        Key::Named(Menu) => Action::Do(Do::Menu),
        Key::Named(F10) if shift && !ctrl && !alt => Action::Do(Do::Menu),
        Key::Named(F10) => return None,
        Key::Char(c) if ctrl && !alt => match (c.to_lowercase().as_str(), shift) {
            ("a", false) => Action::Command("selectAll:"),
            ("b", false) => Action::Tool(Tool::Bold),
            ("i", false) => Action::Tool(Tool::Italic),
            ("e", false) => Action::Tool(Tool::Code),
            ("k", false) => Action::Tool(Tool::Link),
            ("l", false) => Action::Tool(Tool::Checklist),
            ("x", true) => Action::Tool(Tool::Strike),
            ("8", true) | ("*", _) => Action::Tool(Tool::List),
            ("1", false) => Action::Heading(1),
            ("2", false) => Action::Heading(2),
            ("3", false) => Action::Heading(3),
            ("0", false) => Action::Heading(0),
            ("z", true) | ("y", false) => Action::Do(Do::Redo),
            ("z", false) => Action::Do(Do::Undo),
            ("c", false) => Action::Do(Do::Copy),
            ("x", false) => Action::Do(Do::Cut),
            ("v", true) => Action::Do(Do::PastePlain),
            ("v", false) => Action::Do(Do::Paste),
            ("n", false) => Action::Do(Do::NewNote),
            ("o", false) | ("f", true) => Action::Do(Do::Search),
            ("p", false) => Action::Do(Do::Palette),
            ("f", false) => Action::Do(Do::Find),
            ("h", false) => Action::Do(Do::Replace),
            ("h", true) => Action::Do(Do::Past),
            ("t", false) => Action::Do(Do::Today),
            ("\\", false) => Action::Do(Do::SideMenu),
            ("\\", true) | ("|", _) => Action::Do(Do::Toolbar),
            (",", false) => Action::Do(Do::Settings),
            ("s", false) => Action::Do(Do::Save),
            ("q", false) => Action::Do(Do::Quit),
            _ => return None,
        },
        Key::Char(_) => return None,
    })
}

/// Whether `text` from a key is typed: not with Ctrl or Alt held (those are
/// shortcuts), and never control characters.
pub fn types(text: &str, m: Mods) -> bool {
    !m.ctrl && !m.alt && !text.is_empty() && !text.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { ctrl: false, shift: false, alt: false };
    const CTRL: Mods = Mods { ctrl: true, shift: false, alt: false };
    const SHIFT: Mods = Mods { ctrl: false, shift: true, alt: false };
    const CTRL_SHIFT: Mods = Mods { ctrl: true, shift: true, alt: false };
    const ALT: Mods = Mods { ctrl: false, shift: false, alt: true };

    #[test]
    fn the_text_keys_are_the_text_systems_commands_with_shift_extending() {
        let n = |k| action(Key::Named(k), NONE);
        assert_eq!(n(Named::Left), Some(Action::Command("moveLeft:")));
        assert_eq!(action(Key::Named(Named::Left), CTRL), Some(Action::Command("moveWordLeft:")));
        assert_eq!(action(Key::Named(Named::Right), CTRL_SHIFT), Some(Action::Command("moveWordRightAndModifySelection:")));
        assert_eq!(action(Key::Named(Named::Home), CTRL_SHIFT), Some(Action::Command("moveToBeginningOfDocumentAndModifySelection:")));
        assert_eq!(n(Named::End), Some(Action::Command("moveToEndOfLine:")));
        assert_eq!(action(Key::Named(Named::Down), SHIFT), Some(Action::Command("moveDownAndModifySelection:")));
        assert_eq!(action(Key::Named(Named::Backspace), CTRL), Some(Action::Command("deleteWordBackward:")));
        assert_eq!(n(Named::Delete), Some(Action::Command("deleteForward:")));
        assert_eq!(action(Key::Named(Named::Delete), SHIFT), Some(Action::Do(Do::Cut)), "Shift+Delete cuts, as on Windows");
        assert_eq!(n(Named::Enter), Some(Action::Command("insertNewline:")));
        assert_eq!(action(Key::Named(Named::Enter), SHIFT), Some(Action::Command("insertLineBreak:")), "a line break that ends no list");
        assert_eq!(action(Key::Named(Named::Enter), CTRL), Some(Action::Do(Do::Task)));
        assert_eq!(action(Key::Named(Named::Tab), SHIFT), Some(Action::Command("insertBacktab:")));
        assert_eq!(n(Named::Escape), Some(Action::Command("cancelOperation:")));
        assert_eq!(n(Named::PageDown), Some(Action::Command("pageDown:")));
        assert_eq!(action(Key::Named(Named::Left), ALT), Some(Action::Do(Do::Back)));
        assert_eq!(n(Named::F3), Some(Action::Do(Do::FindNext)));
        assert_eq!(action(Key::Named(Named::F3), SHIFT), Some(Action::Do(Do::FindPrevious)));
        assert_eq!(n(Named::Menu), Some(Action::Do(Do::Menu)), "the menu key opens the text's menu");
        assert_eq!(action(Key::Named(Named::F10), SHIFT), Some(Action::Do(Do::Menu)), "as Shift+F10 does");
        assert_eq!(n(Named::F10), None);
    }

    #[test]
    fn ctrl_is_the_command_key_with_the_shortcuts_windows_has() {
        let c = |s| action(Key::Char(s), CTRL);
        let cs = |s| action(Key::Char(s), CTRL_SHIFT);
        assert_eq!(c("b"), Some(Action::Tool(Tool::Bold)));
        assert_eq!(c("i"), Some(Action::Tool(Tool::Italic)));
        assert_eq!(c("e"), Some(Action::Tool(Tool::Code)));
        assert_eq!(c("k"), Some(Action::Tool(Tool::Link)));
        assert_eq!(c("l"), Some(Action::Tool(Tool::Checklist)));
        assert_eq!(cs("x"), Some(Action::Tool(Tool::Strike)));
        assert_eq!(cs("8"), Some(Action::Tool(Tool::List)));
        assert_eq!(c("2"), Some(Action::Heading(2)));
        assert_eq!(c("0"), Some(Action::Heading(0)));
        assert_eq!(c("z"), Some(Action::Do(Do::Undo)));
        assert_eq!(cs("z"), Some(Action::Do(Do::Redo)));
        assert_eq!(c("y"), Some(Action::Do(Do::Redo)));
        assert_eq!(c("c"), Some(Action::Do(Do::Copy)));
        assert_eq!(c("x"), Some(Action::Do(Do::Cut)));
        assert_eq!(c("v"), Some(Action::Do(Do::Paste)));
        assert_eq!(cs("v"), Some(Action::Do(Do::PastePlain)));
        assert_eq!(c("a"), Some(Action::Command("selectAll:")));
        assert_eq!(c("n"), Some(Action::Do(Do::NewNote)));
        assert_eq!(c("o"), Some(Action::Do(Do::Search)));
        assert_eq!(cs("f"), Some(Action::Do(Do::Search)));
        assert_eq!(c("p"), Some(Action::Do(Do::Palette)));
        assert_eq!(c("f"), Some(Action::Do(Do::Find)));
        assert_eq!(c("h"), Some(Action::Do(Do::Replace)));
        assert_eq!(cs("h"), Some(Action::Do(Do::Past)));
        assert_eq!(c("t"), Some(Action::Do(Do::Today)));
        assert_eq!(c("\\"), Some(Action::Do(Do::SideMenu)));
        assert_eq!(c(","), Some(Action::Do(Do::Settings)));
        assert_eq!(c("q"), Some(Action::Do(Do::Quit)));
        assert_eq!(action(Key::Char("B"), CTRL_SHIFT), None, "Ctrl+Shift+B is nothing");
        assert_eq!(action(Key::Char("b"), NONE), None, "a letter alone is text");
        assert_eq!(c("j"), None);
    }

    #[test]
    fn text_is_typed_without_ctrl_or_alt_and_never_control_characters() {
        assert!(types("a", NONE));
        assert!(types("A", SHIFT));
        assert!(types("\u{645}", NONE), "Arabic from its layout");
        assert!(!types("\u{2}", CTRL), "Ctrl+B's control character");
        assert!(!types("b", CTRL));
        assert!(!types("x", ALT));
        assert!(!types("\r", NONE), "Enter is a command, not text");
        assert!(!types("\u{8}", NONE) && !types("\u{7f}", NONE) && !types("\t", NONE));
        assert!(!types("", NONE));
    }
}
