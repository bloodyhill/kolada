//! Menus the model draws itself, in the chrome like the rest of it, as
//! Windows draws its own (`kolada-win` `app/menu.rs`), for a shell with no
//! menus of its own to show them in (Linux: a toolkit's menus would mean a
//! toolkit). Opt-in: `set_drawn_menus(true)`. Elsewhere the shell shows the
//! model's items as the OS's own menus (`menu.rs`), and nothing here runs.
//!
//! What a menu holds is `menu.rs`'s, as for the Mac: a right-click on the
//! note (a misspelled word's fixes first), a file tree row, a kept note or a
//! deleted one; a row's three dots and the send button. Open, it is drawn
//! over everything and fades in; the pointer lights its items, a click picks
//! one, Up and Down walk them, Return picks, Escape closes, and a click
//! beside it, the wheel or typing put it away. The clipboard's items come
//! back to the shell (`take_menu_picks`); the rest the model does.

use std::time::{Duration, Instant};

use kolada_editor::TableEdit;
use kolada_ui::scene::{self, MenuView, Target};
use kolada_ui::chrome;

use super::menu::{MenuAction, MenuItem};
use super::share::Agent;
use super::*;

/// A menu fades in over this long.
pub const FADE: Duration = Duration::from_millis(90);

/// A menu on the screen.
pub struct Open {
    /// Its point, DIPs: its top left, or its top right when `right`.
    pub x: f32,
    pub y: f32,
    pub right: bool,
    pub items: Vec<scene::MenuItem>,
    pub actions: Vec<MenuAction>,
    pub opened: Instant,
    /// The file tree row it is for (kept lit while it is open), and the
    /// button it opened from.
    pub row: Option<usize>,
    pub anchor: Option<Target>,
    /// The item the button went down on: picked if it comes up there.
    pub pressed: Option<usize>,
}

#[derive(Default)]
pub struct Drawn {
    /// The shell has the model draw its menus.
    pub on: bool,
    pub open: Option<Open>,
    /// The clipboard's items picked, for the shell to do.
    pub picks: Vec<MenuAction>,
}

/// An item's icon (the icon font's glyphs, at the Windows app's codepoints).
fn icon(action: &MenuAction, label: &str) -> &'static str {
    match action {
        MenuAction::Cut => "\u{E8C6}",
        MenuAction::Copy | MenuAction::CopyRich | MenuAction::Duplicate(_) => "\u{E8C8}",
        MenuAction::Paste | MenuAction::PastePlain => "\u{E77F}",
        MenuAction::SelectAll => "\u{E8B3}",
        MenuAction::Format(Tool::Bold) => "\u{E8DD}",
        MenuAction::Format(Tool::Italic) => "\u{E8DB}",
        MenuAction::Format(_) | MenuAction::CopyLink(_) | MenuAction::ShareLink => "\u{E71B}",
        MenuAction::Highlight => "\u{E7E6}",
        MenuAction::FollowLink(_) | MenuAction::OpenWindow(_) => "\u{E8A7}",
        MenuAction::Open(_) => "\u{E8E5}",
        MenuAction::Rename(_) => "\u{E8AC}",
        MenuAction::Favorite(_) => "\u{E734}",
        MenuAction::Reveal(_) => "\u{E838}",
        MenuAction::Delete(_) | MenuAction::DeleteForGood(_) | MenuAction::Table(TableEdit::DeleteRow | TableEdit::DeleteColumn) => "\u{E74D}",
        MenuAction::NewNoteIn(_) | MenuAction::Table(TableEdit::RowBelow | TableEdit::RowAbove | TableEdit::ColumnRight | TableEdit::ColumnLeft) => "\u{E710}",
        MenuAction::Table(TableEdit::Align(kolada_markdown::Align::Center)) => "\u{E8E3}",
        MenuAction::Table(TableEdit::Align(kolada_markdown::Align::Right)) => "\u{E8E2}",
        MenuAction::Table(_) => "\u{E8E4}",
        MenuAction::Restore(_) => "\u{E7A7}",
        MenuAction::ExportNote => "\u{EDE1}",
        MenuAction::SendTo(Agent::Codex | Agent::Gemini) => "\u{E756}",
        MenuAction::SendTo(_) => "\u{E8BD}",
        MenuAction::ToggleRecent if label.starts_with("Hide") => "\u{ED1A}",
        MenuAction::ToggleRecent => "\u{E7B3}",
        // A spelling fix is the word itself; the dictionary's two have theirs.
        MenuAction::Spell(_) if label.starts_with("Learn") => "\u{E8FB}",
        MenuAction::Spell(_) if label.starts_with("Ignore") => "\u{E7BA}",
        MenuAction::Spell(_) => "",
    }
}

/// What destroys something reads so.
fn danger(action: &MenuAction) -> bool {
    matches!(action, MenuAction::Delete(_) | MenuAction::DeleteForGood(_) | MenuAction::Table(TableEdit::DeleteRow | TableEdit::DeleteColumn))
}

/// An item's key (`MenuItem::key`: a letter with Command, an upper-case one
/// adding Shift) as this keyboard names it.
pub fn keys(key: &str, apple: bool) -> &'static str {
    match (key, apple) {
        ("", _) => "",
        ("x", false) => "Ctrl+X",
        ("c", false) => "Ctrl+C",
        ("v", false) => "Ctrl+V",
        ("V", false) => "Ctrl+Shift+V",
        ("a", false) => "Ctrl+A",
        ("b", false) => "Ctrl+B",
        ("i", false) => "Ctrl+I",
        ("k", false) => "Ctrl+K",
        ("x", true) => "\u{2318}X",
        ("c", true) => "\u{2318}C",
        ("v", true) => "\u{2318}V",
        ("V", true) => "\u{21E7}\u{2318}V",
        ("a", true) => "\u{2318}A",
        ("b", true) => "\u{2318}B",
        ("i", true) => "\u{2318}I",
        ("k", true) => "\u{2318}K",
        _ => "",
    }
}

impl Model {
    /// The model draws its own menus (the shell has none to show them in):
    /// right-clicks and the buttons that open menus open them in the chrome.
    pub fn set_drawn_menus(&mut self, on: bool) {
        self.menus.drawn.on = on;
    }

    /// A drawn menu is open.
    pub fn menu_open(&self) -> bool {
        self.menus.drawn.open.is_some()
    }

    /// Puts an open menu away; whether there was one.
    pub fn close_menu(&mut self) -> bool {
        let closed = self.menus.drawn.open.take().is_some();
        if closed && matches!(self.hover, Some(Target::MenuItem(_) | Target::MenuPanel)) {
            self.hover = None;
        }
        closed
    }

    /// The clipboard's items picked from a drawn menu since the last look
    /// (Cut, Copy, Paste, Paste as Plain Text): the shell does them, with
    /// its clipboard.
    pub fn take_menu_picks(&mut self) -> Vec<MenuAction> {
        std::mem::take(&mut self.menus.drawn.picks)
    }

    /// Opens `items` drawn at (x, y) DIPs; spelling's fixes first. False
    /// when there is nothing to show.
    pub(super) fn show_menu(&mut self, (x, y): (f32, f32), spelling: Vec<scene::MenuItem>, items: Vec<MenuItem>, row: Option<usize>, anchor: Option<Target>, right: bool) -> bool {
        if spelling.is_empty() && items.is_empty() {
            return false;
        }
        let apple = self.apple_keys();
        let mut shown = Vec::new();
        let mut actions = Vec::new();
        for (i, s) in spelling.into_iter().enumerate() {
            let action = MenuAction::Spell(i);
            shown.push(scene::MenuItem { icon: icon(&action, &s.label), ..s });
            actions.push(action);
        }
        let after_spelling = !shown.is_empty();
        for (i, it) in items.into_iter().enumerate() {
            shown.push(scene::MenuItem {
                icon: icon(&it.action, &it.label),
                keys: keys(it.key, apple),
                danger: danger(&it.action),
                divider: it.divider || (i == 0 && after_spelling),
                label: it.label,
            });
            actions.push(it.action);
        }
        if let Some(first) = shown.first_mut() {
            first.divider = false;
        }
        // The settings panel gives way, as on Windows.
        self.settings.open = false;
        self.menus.drawn.open = Some(Open { x, y, right, items: shown, actions, opened: Instant::now(), row, anchor, pressed: None });
        true
    }

    /// A right-click at (x, y) DIPs: the menu for what is there, drawn at
    /// the pointer (a misspelled word's fixes first, the word selected).
    /// `has_html`: the clipboard holds formatted text. Whether the screen
    /// changes (the caret may move even when no menu opens).
    pub fn open_context_menu(&mut self, x: f32, y: f32, has_html: bool) -> bool {
        let had = self.close_menu();
        let before = self.editor.selection();
        let on_note = scene::hit(&self.hits, x, y) == Some(Target::Note) && self.popup.is_none();
        let spelling = if on_note && !self.settings.read_only && self.scrub_page().is_none() { self.spell_menu_items(x, y).unwrap_or_default() } else { Vec::new() };
        let items = self.context_menu(x, y, has_html);
        let row = match scene::hit(&self.hits, x, y) {
            Some(Target::Row(i) | Target::RowMenu(i)) if self.search.is_none() && self.trash.is_none() => Some(i),
            _ => None,
        };
        let opened = self.show_menu((x, y), spelling, items, row, None, false);
        opened || had || self.editor.selection() != before
    }

    /// The menu key (or Shift+F10): the text's menu at the caret.
    pub fn open_menu_at_caret(&mut self, has_html: bool) -> bool {
        self.close_menu();
        if self.search_has_keys() || self.trash.is_some() || self.popup.is_some() || self.scrub_page().is_some() {
            return false;
        }
        let Some(c) = self.caret else { return false };
        let head = self.editor.selection().head;
        let link = kolada_ui::links::link_at(self.editor.text(), head).map(|_| head);
        let items = self.text_menu(link, has_html);
        self.show_menu((c.right + 2.0, c.bottom + 2.0), Vec::new(), items, None, None, false)
    }

    /// The menu as the scene draws it, and the row it is for and the
    /// button it hangs from.
    pub(super) fn menu_view(&self) -> (Option<MenuView<'_>>, Option<usize>, Option<Target>) {
        let Some(m) = &self.menus.drawn.open else { return (None, None, None) };
        let alpha = if self.ui.animate { chrome::ease(m.opened.elapsed().as_secs_f32() / FADE.as_secs_f32()) } else { 1.0 };
        (Some(MenuView { x: m.x, y: m.y, align_right: m.right, items: &m.items, alpha }), m.row, m.anchor)
    }

    /// The menu is still fading in.
    pub(super) fn menu_fading(&self) -> bool {
        self.ui.animate && self.menus.drawn.open.as_ref().is_some_and(|m| m.opened.elapsed() < FADE + Duration::from_millis(20))
    }

    /// The button went down on `target` while a menu is open: on an item it
    /// waits for the release; beside the menu it only puts the menu away
    /// (another menu button opens its own instead). None when no menu is
    /// open, or the press goes on as usual.
    pub(super) fn menu_down(&mut self, target: Option<Target>) -> Option<bool> {
        let m = self.menus.drawn.open.as_mut()?;
        match target {
            Some(Target::MenuItem(i)) => {
                m.pressed = Some(i);
                Some(true)
            }
            Some(Target::MenuPanel) => Some(false),
            other => {
                let anchor = m.anchor;
                self.close_menu();
                match other {
                    // The same button again: just closed.
                    t if t == anchor && t.is_some() => Some(true),
                    Some(Target::RowMenu(_) | Target::SendTo) => None,
                    _ => Some(true),
                }
            }
        }
    }

    /// The button came up on `target`: the item it went down on is picked.
    /// None when no item was pressed.
    pub(super) fn menu_up(&mut self, target: Option<Target>) -> Option<bool> {
        let m = self.menus.drawn.open.as_mut()?;
        let pressed = m.pressed.take()?;
        if target != Some(Target::MenuItem(pressed)) {
            return Some(false);
        }
        Some(self.pick_menu_item(pressed))
    }

    /// The keys while a menu is open: Up and Down walk it, Return (or Tab)
    /// picks, Escape closes it; anything else puts it away and goes on as
    /// usual (None).
    pub(super) fn menu_command(&mut self, name: &str) -> Option<bool> {
        let m = self.menus.drawn.open.as_ref()?;
        let n = m.items.len();
        let current = match self.hover {
            Some(Target::MenuItem(i)) if i < n => Some(i),
            _ => None,
        };
        match name {
            "moveDown:" | "insertTab:" => self.hover = Some(Target::MenuItem(current.map_or(0, |i| (i + 1) % n))),
            "moveUp:" | "insertBacktab:" => self.hover = Some(Target::MenuItem(current.map_or(n - 1, |i| (i + n - 1) % n))),
            "insertNewline:" | "insertLineBreak:" => {
                return Some(match current {
                    Some(i) => self.pick_menu_item(i),
                    None => self.close_menu(),
                });
            }
            "cancelOperation:" => {
                self.close_menu();
            }
            _ => {
                self.close_menu();
                return None;
            }
        }
        Some(true)
    }

    /// The `i`th item of the open menu picked: the menu closes and it is
    /// done (the clipboard's by the shell). Whether the screen changes.
    pub fn pick_menu_item(&mut self, i: usize) -> bool {
        let Some(m) = self.menus.drawn.open.take() else { return false };
        if matches!(self.hover, Some(Target::MenuItem(_) | Target::MenuPanel)) {
            self.hover = None;
        }
        let Some(action) = m.actions.get(i).cloned() else { return true };
        match action {
            MenuAction::Cut | MenuAction::Copy | MenuAction::Paste | MenuAction::PastePlain => {
                self.menus.drawn.picks.push(action);
            }
            MenuAction::SelectAll => {
                self.select_all();
            }
            MenuAction::Spell(i) => {
                self.spell_pick(i);
            }
            action => {
                let (_, effects) = self.run_menu(action);
                self.menus.effects.extend(effects);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn model(test: &str, note: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-app-drawn-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Note.md"), note).unwrap();
        std::fs::write(dir.join("Other.md"), "# Other\n\nIts text.\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.set_platform(scene::Platform::Linux);
        m.set_drawn_menus(true);
        m.open("Note.md");
        m.frame(2000, 1440, 2.0);
        (m, dir)
    }

    fn centre(m: &Model, target: Target) -> (f32, f32) {
        let (r, _) = m.hits.iter().find(|(_, t)| *t == target).copied().unwrap_or_else(|| panic!("{target:?} is drawn"));
        ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    }

    fn labels(m: &Model) -> Vec<String> {
        m.menus.drawn.open.as_ref().map(|o| o.items.iter().map(|i| i.label.clone()).collect()).unwrap_or_default()
    }

    /// Where item `i` of the open menu is drawn.
    fn item(m: &Model, i: usize) -> (f32, f32) {
        centre(m, Target::MenuItem(i))
    }

    #[test]
    fn a_right_click_on_the_note_draws_its_menu_with_icons_and_ctrl_keys() {
        let (mut m, dir) = model("note", "# Note\n\nBody text here.\n");
        let (x, y) = centre(&m, Target::Note);
        assert!(m.open_context_menu(x, y, false));
        assert!(m.menu_open());
        assert!(m.take_menu_request().is_none(), "drawn, not asked of the shell");
        m.frame(2000, 1440, 2.0);
        assert!(m.hits.iter().any(|(_, t)| *t == Target::MenuPanel), "the menu is in the frame");
        let open = m.menus.drawn.open.as_ref().unwrap();
        let paste = open.items.iter().find(|i| i.label == "Paste").expect("Paste");
        assert_eq!(paste.keys, "Ctrl+V", "Linux's keys");
        assert!(!paste.icon.is_empty());
        assert!(open.items.iter().find(|i| i.label == "Delete Note").is_some_and(|i| i.danger), "deleting reads as dangerous");
        assert!(!open.items[0].divider, "a menu starts with no divider");
        // Escape closes it.
        assert!(m.command("cancelOperation:"));
        assert!(!m.menu_open());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_click_picks_an_item_on_its_release_and_the_clipboards_go_to_the_shell() {
        let (mut m, dir) = model("pick", "# Note\n\nBody text here.\n");
        let (x, y) = centre(&m, Target::Note);
        m.open_context_menu(x, y, false);
        m.frame(2000, 1440, 2.0);
        let names = labels(&m);
        let select_all = names.iter().position(|l| l == "Select All").unwrap();
        let (ix, iy) = item(&m, select_all);
        assert!(m.pointer_down(ix, iy, 1, false));
        assert!(m.menu_open(), "not on the press");
        m.pointer_up_at(ix, iy);
        assert!(!m.menu_open(), "on the release");
        assert_eq!(m.selected_text(), m.text(), "Select All was done by the model");
        // Copy comes back to the shell.
        m.frame(2000, 1440, 2.0);
        m.open_context_menu(x, y, false);
        m.frame(2000, 1440, 2.0);
        let copy = labels(&m).iter().position(|l| l == "Copy").expect("Copy, with text selected");
        let (cx, cy) = item(&m, copy);
        m.pointer_down(cx, cy, 1, false);
        m.pointer_up_at(cx, cy);
        assert_eq!(m.take_menu_picks(), [MenuAction::Copy]);
        assert!(m.take_menu_picks().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_click_beside_the_menu_only_closes_it() {
        let (mut m, dir) = model("beside", "# Note\n\nBody text here.\n");
        let (x, y) = centre(&m, Target::Note);
        m.open_context_menu(x, y, false);
        m.frame(2000, 1440, 2.0);
        let caret = m.selection();
        let (nx, ny) = centre(&m, Target::NewNote);
        assert!(m.pointer_down(nx, ny, 1, false));
        assert!(!m.menu_open());
        assert_eq!(m.session.note, "Note.md", "the click did not reach New note");
        assert_eq!(m.selection(), caret);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_keys_walk_the_menu_and_return_picks() {
        let (mut m, dir) = model("keys", "# Note\n\nBody text here.\n");
        let (x, y) = centre(&m, Target::Note);
        m.open_context_menu(x, y, false);
        let n = labels(&m).len();
        assert!(m.command("moveDown:"));
        assert_eq!(m.hover, Some(Target::MenuItem(0)));
        assert!(m.command("moveUp:"));
        assert_eq!(m.hover, Some(Target::MenuItem(n - 1)), "around to the last");
        let select_all = labels(&m).iter().position(|l| l == "Select All").unwrap();
        while m.hover != Some(Target::MenuItem(select_all)) {
            m.command("moveDown:");
        }
        assert!(m.command("insertNewline:"));
        assert!(!m.menu_open());
        assert_eq!(m.selected_text(), m.text());
        // Typing puts the menu away and types.
        m.command("moveToEndOfDocument:");
        m.frame(2000, 1440, 2.0);
        assert!(m.open_context_menu(x, y, false) && m.menu_open());
        assert!(m.commit_marked("z"));
        assert!(!m.menu_open());
        assert!(m.text().contains('z'));
        // As does an arrow, which moves the caret as usual.
        m.frame(2000, 1440, 2.0);
        assert!(m.open_context_menu(x, y, false) && m.menu_open());
        let before = m.selection().head;
        assert!(m.command("moveLeft:"));
        assert!(!m.menu_open() && m.selection().head != before);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_rows_three_dots_and_the_send_button_open_drawn_menus_under_them() {
        let (mut m, dir) = model("buttons", "# Note\n\nBody text here.\n");
        let other = m.session.sidebar.iter().position(|r| r.name == "Other").unwrap();
        m.pointer_moved(centre(&m, Target::Row(other)).0, centre(&m, Target::Row(other)).1);
        m.frame(2000, 1440, 2.0);
        let (dx, dy) = centre(&m, Target::RowMenu(other));
        assert!(m.pointer_down(dx, dy, 1, false));
        m.pointer_up_at(dx, dy);
        assert!(m.take_menu_request().is_none());
        assert_eq!(labels(&m).first().map(String::as_str), Some("Open"));
        assert!(labels(&m).contains(&"Show in Folder".to_string()));
        assert_eq!(m.menus.drawn.open.as_ref().unwrap().row, Some(other), "its row stays lit");
        // Pressing the dots again closes it.
        m.frame(2000, 1440, 2.0);
        m.pointer_down(dx, dy, 1, false);
        assert!(!m.menu_open());
        // The send button's opens at its right edge, aligned right.
        let (sx, sy) = centre(&m, Target::SendTo);
        m.pointer_down(sx, sy, 1, false);
        let open = m.menus.drawn.open.as_ref().expect("the send menu");
        assert!(open.right && open.anchor == Some(Target::SendTo));
        assert_eq!(open.items[0].label, "Share Link\u{2026}");
        // From one to the other: the row's dots open theirs at once.
        m.frame(2000, 1440, 2.0);
        m.pointer_moved(dx, dy);
        m.frame(2000, 1440, 2.0);
        m.pointer_down(dx, dy, 1, false);
        assert_eq!(m.menus.drawn.open.as_ref().map(|o| o.anchor), Some(Some(Target::RowMenu(other))));
        // Picking Open opens the note.
        m.frame(2000, 1440, 2.0);
        let (ox, oy) = item(&m, 0);
        m.pointer_down(ox, oy, 1, false);
        m.pointer_up_at(ox, oy);
        assert_eq!(m.session.note, "Other.md");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_wheel_puts_a_menu_away() {
        let (mut m, dir) = model("wheel", "# Note\n\nBody text here.\n");
        let (x, y) = centre(&m, Target::Note);
        m.open_context_menu(x, y, false);
        assert!(m.wheel(40.0, true));
        assert!(!m.menu_open());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_drawn_menus_the_shell_is_asked_as_before() {
        let (mut m, dir) = model("native", "# Note\n\nBody text here.\n");
        m.set_drawn_menus(false);
        let (sx, sy) = centre(&m, Target::SendTo);
        m.pointer_down(sx, sy, 1, false);
        assert!(!m.menu_open());
        assert!(m.take_menu_request().is_some(), "the Mac shows the OS's menu");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keys_are_named_for_the_keyboard() {
        assert_eq!(keys("V", false), "Ctrl+Shift+V");
        assert_eq!(keys("x", false), "Ctrl+X");
        assert_eq!(keys("V", true), "\u{21E7}\u{2318}V");
        assert_eq!(keys("", false), "");
    }
}
