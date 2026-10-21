//! The app's commands, by name, and the iPad keyboard's shortcuts for them:
//! the Mac's menu bar keys (`kolada-mac` `app.rs`), so a hardware keyboard
//! does on an iPad what it does on a Mac. The shell builds its key commands
//! and the iPadOS menu bar from `kolada_commands`, and sends the names back
//! through `kolada_do`; the table is tested here against what `run` knows.

use std::ffi::{CString, c_char};

use kolada_app::Model;
use kolada_app::model::menu::MenuAction;
use kolada_app::model::share::Effect;
use kolada_editor::BlockKind;
use kolada_ui::scene::{Panel, Tool};

/// A shortcut: the menu it is in, its title, its key and modifiers
/// ("cmd", "cmd shift", "cmd alt"), and the command it sends.
pub struct Key {
    pub menu: &'static str,
    pub title: &'static str,
    pub key: &'static str,
    pub mods: &'static str,
    pub what: &'static str,
}

const fn k(menu: &'static str, title: &'static str, key: &'static str, mods: &'static str, what: &'static str) -> Key {
    Key { menu, title, key, mods, what }
}

/// Every shortcut, as the Mac's menus have them. Cmd+H, Cmd+Space and
/// Cmd+Tab are the system's on an iPad: Heading cycles from the menu and
/// the toolbar only. `paste-plain` is the shell's (it reads the pasteboard).
pub const KEYS: &[Key] = &[
    k("File", "New Note", "n", "cmd", "new"),
    k("File", "Today's Note", "d", "cmd shift", "today"),
    k("File", "What's Due", "t", "cmd", "due"),
    k("File", "Search Notes", "o", "cmd", "search"),
    k("File", "Command Palette", "p", "cmd", "palette"),
    k("File", "Find", "f", "cmd", "find"),
    k("File", "Find and Replace", "f", "cmd alt", "replace"),
    k("File", "Find Next", "g", "cmd", "find-next"),
    k("File", "Find Previous", "g", "cmd shift", "find-previous"),
    k("File", "Note History", "h", "cmd shift", "past"),
    k("File", "Share Link\u{2026}", "", "", "share"),
    k("File", "Export Note\u{2026}", "e", "cmd shift", "export"),
    k("File", "Copy Note", "c", "cmd shift", "copy-note"),
    k("File", "Move to Trash", "\u{8}", "cmd", "trash"),
    k("File", "Settings", ",", "cmd", "settings"),
    k("Edit", "Undo", "z", "cmd", "undo"),
    k("Edit", "Redo", "z", "cmd shift", "redo"),
    k("Edit", "Paste as Plain Text", "v", "cmd shift", "paste-plain"),
    k("Format", "Bold", "b", "cmd", "bold"),
    k("Format", "Italic", "i", "cmd", "italic"),
    k("Format", "Strikethrough", "x", "cmd shift", "strike"),
    k("Format", "Code", "e", "cmd", "code"),
    k("Format", "Link", "k", "cmd", "link"),
    k("Format", "Bulleted List", "8", "cmd", "list"),
    k("Format", "Checklist", "l", "cmd", "checklist"),
    k("Format", "Check Off Task", "\r", "cmd", "task"),
    k("Format", "Heading", "", "", "heading"),
    k("Format", "Heading 1", "1", "cmd", "h1"),
    k("Format", "Heading 2", "2", "cmd", "h2"),
    k("Format", "Heading 3", "3", "cmd", "h3"),
    k("Format", "Body Text", "0", "cmd", "body"),
    k("Format", "Highlight", "u", "cmd shift", "highlight"),
    k("View", "Side Menu", "\\", "cmd", "menu"),
    k("View", "Toolbar", "\\", "cmd shift", "toolbar"),
    k("View", "Highlights", "", "", "highlights"),
    k("Go", "Back", "[", "cmd", "back"),
    k("Go", "Forward", "]", "cmd", "forward"),
];

/// Commands the shell does itself.
pub const SHELLS: &[&str] = &["paste-plain"];

/// Does the command `what`: None when there is no such command; else
/// whether the screen changes, and what the OS must do after.
pub fn run(m: &mut Model, what: &str) -> Option<(bool, Vec<Effect>)> {
    let done = |changed: bool| Some((changed, Vec::new()));
    match what {
        "new" => done(m.new_note()),
        "search" => done(m.open_search()),
        "palette" => done(m.open_palette()),
        "today" => done(m.open_today()),
        "due" => done(m.toggle_agenda()),
        "past" => done(m.toggle_scrub()),
        "find" => done(m.open_find(false)),
        "replace" => done(m.open_find(true)),
        "find-next" => done(m.find_again(false)),
        "find-previous" => done(m.find_again(true)),
        "share" => Some(m.run_menu(MenuAction::ShareLink)),
        "export" => Some(m.run_menu(MenuAction::ExportNote)),
        "copy-note" => Some(m.run_menu(MenuAction::CopyRich)),
        "trash" => done(m.trash_open_note()),
        "settings" => done(m.toggle_options()),
        "highlights" => done(m.open_highlights()),
        "menu" => done(m.toggle_panel(Panel::Sidebar)),
        "reveal-menu" => done(m.reveal_side_menu()),
        "toolbar" => done(m.toggle_panel(Panel::Toolbar)),
        "back" => done(m.step(false)),
        "forward" => done(m.step(true)),
        "undo" => done(m.undo()),
        "redo" => done(m.redo()),
        "bold" => done(m.tool(Tool::Bold)),
        "italic" => done(m.tool(Tool::Italic)),
        "strike" => done(m.tool(Tool::Strike)),
        "code" => done(m.tool(Tool::Code)),
        "link" => done(m.tool(Tool::Link)),
        "list" => done(m.tool(Tool::List)),
        "checklist" => done(m.tool(Tool::Checklist)),
        "heading" => done(m.tool(Tool::Heading)),
        "highlight" => Some(m.run_menu(MenuAction::Highlight)),
        "task" => done(m.toggle_task_here()),
        "h1" => done(m.block(BlockKind::Heading(1))),
        "h2" => done(m.block(BlockKind::Heading(2))),
        "h3" => done(m.block(BlockKind::Heading(3))),
        "body" => done(m.block(BlockKind::Body)),
        // As the keyboard's Escape: the Today card and the past take it first.
        "escape" => done(m.command("cancelOperation:")),
        // The note written and named for its title, and the session: as the app leaves the front.
        "save" => done(m.save_flush()),
        // A moment after an edit, as the Mac does: the note only, no rename.
        "autosave" => done(m.save()),
        // The vault changed with no watcher to say so (iCloud downloads, the Files app).
        "refresh" => done(m.refresh()),
        _ => {
            None
        }
    }
}

/// The shortcuts, a line each: `menu	title	key	mods	command`.
pub fn lines() -> String {
    KEYS.iter().map(|k| format!("{}\t{}\t{}\t{}\t{}\n", k.menu, k.title, k.key, k.mods, k.what)).collect()
}

/// The shortcuts and the iPadOS menu bar's items (see `lines`), as a C
/// string to free with `kolada_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_commands() -> *mut c_char {
    CString::new(lines()).map_or(std::ptr::null_mut(), CString::into_raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_runs_a_command_and_no_two_share_a_key() {
        let dir = std::env::temp_dir().join(format!("kolada-ios-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), "# Plan\n\nsome words\n").unwrap();
        for key in KEYS {
            if SHELLS.contains(&key.what) {
                continue;
            }
            let mut m = Model::new(dir.clone(), true);
            m.open("Plan.md");
            m.frame(1640, 2360, 2.0);
            assert!(run(&mut m, key.what).is_some(), "{} sends {:?}, which the app knows", key.title, key.what);
        }
        let mut seen = std::collections::HashSet::new();
        for key in KEYS.iter().filter(|k| !k.key.is_empty()) {
            assert!(seen.insert((key.key, key.mods)), "{} takes {} {:?} again", key.title, key.mods, key.key);
            assert!(key.mods.split(' ').all(|m| ["cmd", "shift", "alt", "ctrl"].contains(&m)), "{}: {}", key.title, key.mods);
            assert!(key.mods.contains("cmd"), "{} has Command: an iPad keeps the rest for typing", key.title);
            assert!(!(key.key == "h" && key.mods == "cmd"), "Cmd+H is the iPad's Home");
        }
        assert!(run(&mut Model::new(dir.clone(), true), "no-such-command").is_none());
        assert_eq!(lines().lines().count(), KEYS.len());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bold_from_the_keyboard_wraps_the_selection_as_one_undo_step() {
        let dir = std::env::temp_dir().join(format!("kolada-ios-bold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), "# Plan\n\nsome words\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.open("Plan.md");
        m.frame(1640, 2360, 2.0);
        m.select_utf16(8, 4);
        assert!(matches!(run(&mut m, "bold"), Some((true, e)) if e.is_empty()));
        assert!(m.access_text().contains("**some** words"));
        run(&mut m, "undo");
        assert!(m.access_text().contains("\nsome words"));
        run(&mut m, "h2");
        assert!(m.access_text().contains("## some words"), "the caret's line a heading");
        let _ = std::fs::remove_dir_all(dir);
    }
}
