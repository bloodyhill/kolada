//! The menus, as on Windows (`kolada-win` `app/menu.rs`), shown as the Mac's
//! own: right-click the note (a link, a table, the clipboard, formatting,
//! the note), a file tree row or its three dots, a deleted note; and the
//! title bar's send button. The model says what a menu holds and does what
//! an item picks; the view only shows it.

use std::path::PathBuf;

use kolada_editor::{Inline, Selection, TableEdit};
use kolada_markdown as md;
use kolada_ui::links::{self, Link};
use kolada_ui::scene::{Target, Tool};

use super::share::{Agent, Effect};
use super::*;

/// What an item does.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuAction {
    Cut,
    Copy,
    Paste,
    /// The copied text without its formatting.
    PastePlain,
    SelectAll,
    /// A table's rows, columns and alignment, where the caret is.
    Table(TableEdit),
    /// The `i`th fix offered for a misspelled word (`spelling.rs`).
    Spell(usize),
    /// A deleted note, by index in the trash: bring it back, or remove it to
    /// the Mac's Trash.
    Restore(usize),
    DeleteForGood(usize),
    Format(Tool),
    Highlight,
    /// Follow the link at a text offset.
    FollowLink(usize),
    /// Notes, by vault-relative path (with `.md`): open, open in a window
    /// of its own, rename, duplicate, delete.
    Open(String),
    OpenWindow(String),
    Rename(String),
    Duplicate(String),
    Delete(String),
    /// A `[[link]]` to the note of this name, on the clipboard.
    CopyLink(String),
    /// A note (with `.md`) or folder (with `/`) into the favorites, or out.
    Favorite(String),
    /// A file or folder in the Finder.
    Reveal(PathBuf),
    /// A fresh page that saves into this folder (vault-relative).
    NewNoteIn(String),
    /// The send menu: the note shared as a link (the share card), copied
    /// with its formatting, exported, or handed to an AI app.
    ShareLink,
    CopyRich,
    ExportNote,
    SendTo(Agent),
    /// The side menu's Recent hidden, or back (`session::HIDE_RECENT`).
    ToggleRecent,
}

/// One line of a menu: its label, the key that does the same (shown at its
/// side: a letter with Command, an upper-case one adds Shift), and whether a
/// new group starts here.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub key: &'static str,
    pub action: MenuAction,
    pub divider: bool,
}

fn item(label: &str, action: MenuAction) -> MenuItem {
    MenuItem { label: label.to_string(), key: "", action, divider: false }
}

fn keyed(label: &str, key: &'static str, action: MenuAction) -> MenuItem {
    MenuItem { label: label.to_string(), key, action, divider: false }
}

fn after_divider(mut i: MenuItem) -> MenuItem {
    i.divider = true;
    i
}

/// A menu the model opened itself (a row's three dots, the send button): its
/// items, and where it goes, DIPs (its top left).
#[derive(Clone, Debug)]
pub struct MenuRequest {
    pub x: f32,
    pub y: f32,
    pub items: Vec<MenuItem>,
}

/// A menu starts with no divider.
fn tidy(mut items: Vec<MenuItem>) -> Vec<MenuItem> {
    if let Some(first) = items.first_mut() {
        first.divider = false;
    }
    items
}

impl Model {
    /// A right-click at (x, y) DIPs: the menu for what is there, or empty.
    /// On the note, a click outside the selection moves the caret there
    /// first, so the items are for the place clicked. `has_html`: the
    /// pasteboard holds formatted text (so "Paste as Plain Text" means something).
    pub fn context_menu(&mut self, x: f32, y: f32, has_html: bool) -> Vec<MenuItem> {
        match kolada_ui::scene::hit(&self.hits, x, y) {
            // A deleted note in the trash: bring it back, or remove it for good.
            Some(Target::TrashRow(i) | Target::TrashRestore(i)) if self.trash.is_some() => {
                vec![item("Bring Back", MenuAction::Restore(i)), after_divider(item("Move to Trash", MenuAction::DeleteForGood(i)))]
            }
            Some(Target::Row(i) | Target::RowMenu(i)) if self.search.is_none() && self.trash.is_none() => self.row_menu(i),
            Some(t @ (Target::Recent(_) | Target::Favorite(_))) if self.search.is_none() && self.trash.is_none() => self.kept_menu(t),
            // The side menu's background or a section's heading: Recent hidden or back.
            Some(Target::SidebarPanel | Target::FoldRecent | Target::FoldFavorites) if self.search.is_none() && self.trash.is_none() => self.recent_toggle().into_iter().collect(),
            Some(Target::Note) if !self.search_has_keys() && self.trash.is_none() && self.scrub_page().is_none() => {
                let at = self.offset_at(x, y);
                let s = self.editor.selection();
                if s.is_caret() || at < s.start() || at > s.end() {
                    self.editor.set_selection(Selection::caret(at));
                    self.caret_moved(false);
                }
                let link = links::link_at(self.editor.text(), at).map(|_| at);
                self.text_menu(link, has_html)
            }
            _ => Vec::new(),
        }
    }

    /// The items for the caret and selection as they are (no link under the
    /// click), as the menu key would show them.
    #[cfg(test)]
    pub fn context_items(&self, has_html: bool) -> Vec<MenuItem> {
        self.text_menu(None, has_html)
    }

    /// The text's menu: a link, a table's items, the clipboard, formatting,
    /// Select All, and the note.
    pub(super) fn text_menu(&self, link: Option<usize>, has_html: bool) -> Vec<MenuItem> {
        let selected = !self.editor.selection().is_caret();
        let editable = !self.settings.read_only;
        let mut items = Vec::new();
        if let Some(at) = link {
            items.push(item("Open Link", MenuAction::FollowLink(at)));
        }
        // In a table: its rows, columns and alignment, the clipboard and
        // Select All, and nothing else, so the menu stays short.
        let place = self.editor.table_place().filter(|_| editable);
        if let Some(place) = place {
            items.push(after_divider(item("Add Row Below", MenuAction::Table(TableEdit::RowBelow))));
            if !place.header {
                items.push(item("Add Row Above", MenuAction::Table(TableEdit::RowAbove)));
            }
            items.push(item("Add Column Right", MenuAction::Table(TableEdit::ColumnRight)));
            items.push(item("Add Column Left", MenuAction::Table(TableEdit::ColumnLeft)));
            let aligns = [(md::Align::Left, "Align Left"), (md::Align::Center, "Align Center"), (md::Align::Right, "Align Right")];
            for (n, (align, label)) in aligns.into_iter().filter(|(a, _)| *a != place.align).enumerate() {
                let action = MenuAction::Table(TableEdit::Align(align));
                items.push(if n == 0 { after_divider(item(label, action)) } else { item(label, action) });
            }
            let mut deletes = Vec::new();
            if !place.header {
                deletes.push(item("Delete Row", MenuAction::Table(TableEdit::DeleteRow)));
            }
            if place.columns > 1 {
                deletes.push(item("Delete Column", MenuAction::Table(TableEdit::DeleteColumn)));
            }
            if let Some(first) = deletes.first_mut() {
                first.divider = true;
            }
            items.extend(deletes);
        }
        if selected && editable {
            items.push(after_divider(keyed("Cut", "x", MenuAction::Cut)));
        }
        if selected {
            let copy = keyed("Copy", "c", MenuAction::Copy);
            items.push(if editable { copy } else { after_divider(copy) });
        }
        if editable {
            let paste = keyed("Paste", "v", MenuAction::Paste);
            items.push(if selected { paste } else { after_divider(paste) });
            if has_html {
                items.push(keyed("Paste as Plain Text", "V", MenuAction::PastePlain));
            }
        }
        if editable && place.is_none() {
            // The whole note, formatted, for mail and documents.
            if !self.editor.text().trim().is_empty() {
                items.push(item("Copy Note as Rich Text", MenuAction::CopyRich));
            }
            items.push(after_divider(keyed("Bold", "b", MenuAction::Format(Tool::Bold))));
            items.push(keyed("Italic", "i", MenuAction::Format(Tool::Italic)));
            items.push(item("Highlight", MenuAction::Highlight));
            items.push(keyed("Link", "k", MenuAction::Format(Tool::Link)));
        }
        items.push(after_divider(keyed("Select All", "a", MenuAction::SelectAll)));
        // The note itself.
        if place.is_none() && editable && !self.session.note.is_empty() && self.path.is_some() {
            let name = self.path.as_deref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            items.push(after_divider(item("Copy Link to Note", MenuAction::CopyLink(name))));
            items.push(item("Delete Note", MenuAction::Delete(self.session.note.clone())));
        }
        tidy(items)
    }

    /// A file tree row's menu.
    pub fn row_menu(&self, i: usize) -> Vec<MenuItem> {
        let Some(row) = self.session.sidebar.get(i) else { return Vec::new() };
        let rel = vault::row_paths(&self.session.sidebar)[i].clone();
        match row.kind {
            RowKind::Note => {
                let note = format!("{rel}.md");
                vec![
                    item("Open", MenuAction::Open(note.clone())),
                    item("Open in New Window", MenuAction::OpenWindow(note.clone())),
                    after_divider(item("Rename", MenuAction::Rename(note.clone()))),
                    item("Duplicate", MenuAction::Duplicate(note.clone())),
                    item("Copy Link to Note", MenuAction::CopyLink(row.name.clone())),
                    self.favorite_item(note.clone()),
                    item(&format!("Show in {}", self.file_manager()), MenuAction::Reveal(self.vault.join(&note))),
                    after_divider(item("Delete", MenuAction::Delete(note))),
                ]
            }
            RowKind::Folder { .. } => vec![
                item("New Note Here", MenuAction::NewNoteIn(rel.clone())),
                self.favorite_item(format!("{rel}/")),
                item(&format!("Show in {}", self.file_manager()), MenuAction::Reveal(self.vault.join(&rel))),
            ],
        }
    }

    /// The send button's menu: the note shared as a link; the AI apps found
    /// here; then the note exported to a file, or copied.
    pub fn send_menu(&self) -> Vec<MenuItem> {
        self.send_menu_with(self.agents())
    }

    /// The AI apps and tools the OS has (asked of `services.installed`).
    pub(super) fn agents(&self) -> Vec<Agent> {
        let installed = &self.services.installed;
        Agent::detect_with(|app| installed.app(app), |tool| installed.tool(tool).is_some())
    }

    pub fn send_menu_with(&self, agents: Vec<Agent>) -> Vec<MenuItem> {
        let mut items = vec![item("Share Link\u{2026}", MenuAction::ShareLink)];
        for (k, a) in agents.into_iter().enumerate() {
            let i = item(a.label(), MenuAction::SendTo(a));
            items.push(if k == 0 { after_divider(i) } else { i });
        }
        items.push(after_divider(item("Export Note\u{2026}", MenuAction::ExportNote)));
        items.push(item("Copy", MenuAction::CopyRich));
        items
    }

    /// Opens a menu from a button: below it, its left edge at the button's
    /// (`right`: its right edge at the button's).
    pub(super) fn request_menu(&mut self, button: Target, items: Vec<MenuItem>, right: bool) -> bool {
        let Some(r) = self.hits.iter().find(|(_, t)| *t == button).map(|(r, _)| *r) else { return false };
        if items.is_empty() {
            return false;
        }
        // The view puts the menu's top left here; a right-aligned one is
        // placed by the view from the button's right edge.
        let x = if right { r.right } else { r.left };
        // Drawn by the model (Linux): under the button, its row kept lit.
        if self.menus.drawn.on {
            let row = match button {
                Target::RowMenu(i) => Some(i),
                _ => None,
            };
            return self.show_menu((x, r.bottom + 4.0), Vec::new(), items, row, Some(button), right);
        }
        self.menus.request = Some(MenuRequest { x, y: r.bottom + 4.0, items });
        true
    }

    /// The menu the model asked for since this was last asked, and whether
    /// it sits to the left of its point.
    pub fn take_menu_request(&mut self) -> Option<MenuRequest> {
        self.menus.request.take()
    }

    /// A table item picked: rows, columns or alignment, as one undo step.
    pub fn table(&mut self, edit: TableEdit) -> bool {
        self.edit(|e| {
            e.edit_table(edit);
        })
    }

    /// Does what a menu item does (the clipboard's items and spelling are
    /// the view's own). What the view must do outside the app comes back.
    pub fn run_menu(&mut self, action: MenuAction) -> (bool, Vec<Effect>) {
        let none = Vec::new();
        match action {
            MenuAction::Table(edit) => (self.table(edit), none),
            MenuAction::Format(tool) => (self.tool(tool), none),
            MenuAction::Highlight => (self.edit(|e| e.toggle_inline(Inline::Highlight)), none),
            MenuAction::Restore(_) | MenuAction::DeleteForGood(_) => {
                let i = match action {
                    MenuAction::Restore(i) | MenuAction::DeleteForGood(i) => i,
                    _ => unreachable!(),
                };
                (self.trash_pick(action, i), none)
            }
            MenuAction::FollowLink(at) => self.follow_link(at),
            MenuAction::Open(rel) => {
                self.trash = None;
                self.search = None;
                self.open(&rel);
                (true, none)
            }
            MenuAction::OpenWindow(rel) => {
                self.save_flush();
                (false, vec![Effect::Window(self.vault.join(rel))])
            }
            MenuAction::Rename(rel) => (self.rename_by_title(&rel), none),
            MenuAction::Duplicate(rel) => (self.duplicate(&rel), none),
            MenuAction::Delete(rel) => (self.trash_note(&rel), none),
            MenuAction::CopyLink(name) => {
                self.whisper(&format!("Copied a link to {name}"));
                (true, vec![Effect::Clipboard(format!("[[{name}]]"), String::new())])
            }
            MenuAction::Favorite(path) => (self.toggle_favorite(&path), none),
            MenuAction::ToggleRecent => {
                self.session.folded ^= session::HIDE_RECENT;
                self.persist_session();
                (true, none)
            }
            MenuAction::Reveal(path) => (false, vec![Effect::Reveal(path)]),
            MenuAction::NewNoteIn(dir) => {
                self.new_note();
                self.fresh_dir = Some(dir);
                (true, none)
            }
            MenuAction::ShareLink => (self.open_share(), none),
            MenuAction::CopyRich => (true, self.copy_rich().into_iter().collect()),
            MenuAction::ExportNote => (true, self.export_note().into_iter().collect()),
            MenuAction::SendTo(agent) => (true, self.send_to(agent)),
            MenuAction::Cut | MenuAction::Copy | MenuAction::Paste | MenuAction::PastePlain | MenuAction::SelectAll | MenuAction::Spell(_) => (false, none),
        }
    }

    /// The link at `at` followed: a note opens (made if it does not exist
    /// yet), a web page opens in the browser, a tag is searched for.
    pub fn follow_link(&mut self, at: usize) -> (bool, Vec<Effect>) {
        match links::link_at(self.editor.text(), at) {
            None => (false, Vec::new()),
            Some(Link::Web(url)) => (false, if links::is_web(&url) { vec![Effect::Url(url)] } else { Vec::new() }),
            Some(Link::Tag(tag)) => {
                self.open_search();
                if let Some(s) = &mut self.search {
                    s.query = Editor::field(tag);
                    let end = s.query.text().len();
                    s.query.set_selection(Selection::caret(end));
                    s.fresh = false;
                }
                (true, Vec::new())
            }
            Some(Link::Note(name)) => {
                let path = vault::find_note(&self.vault, &name).or_else(|| {
                    let path = note::unique_path(&self.vault, &note::file_stem_for(&name), None);
                    std::fs::write(&path, "").ok().map(|_| path)
                });
                let Some(rel) = path.and_then(|p| vault::relative(&self.vault, &p)) else { return (false, Vec::new()) };
                (self.open_over_page(&rel), Vec::new())
            }
        }
    }

    /// A Command-click at (x, y) DIPs: a link there is followed. None when
    /// there is no link under it (the click is an ordinary one).
    pub fn link_click(&mut self, x: f32, y: f32) -> Option<(bool, Vec<Effect>)> {
        if self.popup.is_some() || kolada_ui::scene::hit(&self.hits, x, y) != Some(Target::Note) || self.scrub_page().is_some() {
            return None;
        }
        let at = self.offset_at(x, y);
        links::link_at(self.editor.text(), at)?;
        Some(self.follow_link(at))
    }

    /// Renaming is editing the title: the note opens with its title line
    /// selected, and the file follows what is typed there.
    fn rename_by_title(&mut self, rel: &str) -> bool {
        if self.session.note != rel {
            self.open(rel);
        }
        let lines = self.editor.outline();
        if let Some(t) = title_line(self.editor.text(), lines) {
            let l = lines[t];
            self.editor.set_selection(Selection { anchor: l.content().min(l.end), head: l.end });
            self.caret_moved(false);
            self.reveal = true;
        }
        true
    }

    /// A copy of the note beside it, numbered, opened.
    fn duplicate(&mut self, rel: &str) -> bool {
        self.save_flush();
        let path = self.vault.join(rel);
        let (Some(dir), Some(stem)) = (path.parent(), path.file_stem()) else { return false };
        let copy = note::unique_path(dir, &stem.to_string_lossy(), None);
        if std::fs::copy(&path, &copy).is_err() {
            return false;
        }
        let Some(to) = vault::relative(&self.vault, &copy) else { return false };
        self.open(&to);
        self.refresh_tree();
        let name = copy.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.whisper(&format!("Duplicated as {name}"));
        true
    }

    /// The note at `rel` goes to the vault's trash: the open note, or
    /// another (from a menu, or a phone's list). Undo brings it back.
    pub fn trash_note(&mut self, rel: &str) -> bool {
        // Only a note inside the vault is ever moved.
        if !vault::is_note_path(rel) {
            return false;
        }
        if rel == self.session.note {
            return self.trash_open_note();
        }
        let Ok(item) = trash::trash(&self.vault, rel, &self.trash_log(), session::now()) else { return false };
        self.session.recent.retain(|n| n != rel);
        self.undo_delete.push((item, self.editor.version()));
        self.refresh_tree();
        if self.trash.is_some() {
            self.open_trash();
        }
        true
    }
}

/// The menus' state in the model.
#[derive(Default)]
pub struct Menus {
    /// A menu the model opened, for the view to show.
    pub request: Option<MenuRequest>,
    /// What the view must do outside the app for a command (`palette.rs`).
    pub effects: Vec<Effect>,
    /// Menus the model draws itself, where the shell asks (`drawn.rs`).
    pub drawn: super::drawn::Drawn,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const TABLE: &str = "| a | bc |\n| - | -- |\n| 1 | 2 |\n\nafter\n";

    fn model(test: &str, note: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-app-menu-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Named after its first line, as a note written in Kolada is (else
        // it would open under its file's name).
        let stem = kolada_core::note::file_stem_for(&kolada_editor::title_of(note));
        let stem = if stem.is_empty() { "Note".to_string() } else { stem };
        std::fs::write(dir.join(format!("{stem}.md")), note).unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        (m, dir)
    }

    fn labels(items: &[MenuItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    fn caret(m: &mut Model, at: usize) {
        m.editor.set_selection(Selection::caret(at));
    }

    #[test]
    fn a_table_cell_offers_rows_columns_and_alignment_before_the_clipboard() {
        let (mut m, dir) = model("table", TABLE);
        caret(&mut m, TABLE.find('1').unwrap());
        let items = m.context_items(false);
        assert_eq!(
            labels(&items),
            ["Add Row Below", "Add Row Above", "Add Column Right", "Add Column Left", "Align Center", "Align Right", "Delete Row", "Delete Column", "Paste", "Select All"]
        );
        assert!(!items[0].divider, "a menu starts with no divider");
        assert!(items.iter().filter(|i| i.divider).count() >= 3);
        // The header has no row above and nothing to delete as a row.
        caret(&mut m, 2);
        let header = m.context_items(false);
        assert_eq!(labels(&header)[..4], ["Add Row Below", "Add Column Right", "Add Column Left", "Align Center"]);
        assert!(!labels(&header).contains(&"Delete Row") && !labels(&header).contains(&"Add Row Above"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn outside_a_table_it_is_the_clipboard_formatting_and_the_note() {
        let (mut m, dir) = model("text", TABLE);
        caret(&mut m, TABLE.find("after").unwrap());
        let note = ["Copy Note as Rich Text", "Bold", "Italic", "Highlight", "Link", "Select All", "Copy Link to Note", "Delete Note"];
        assert_eq!(labels(&m.context_items(false)), [&["Paste"][..], &note].concat());
        assert_eq!(labels(&m.context_items(true)), [&["Paste", "Paste as Plain Text"][..], &note].concat());
        m.editor.set_selection(Selection { anchor: TABLE.find("after").unwrap(), head: TABLE.len() - 1 });
        assert_eq!(labels(&m.context_items(false)), [&["Cut", "Copy", "Paste"][..], &note].concat());
        // Keys are shown beside what has them.
        assert!(m.context_items(false).iter().any(|i| i.label == "Bold" && i.key == "b"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn picking_a_table_item_reshapes_the_table_and_undo_puts_it_back() {
        let (mut m, dir) = model("edit", TABLE);
        caret(&mut m, TABLE.find('1').unwrap());
        assert!(m.table(TableEdit::RowBelow));
        assert_eq!(m.text().lines().filter(|l| l.starts_with('|')).count(), 4);
        assert!(m.table(TableEdit::ColumnRight));
        assert!(m.text().lines().next().unwrap().matches('|').count() == 4, "{:?}", m.text().lines().next());
        assert!(m.table(TableEdit::Align(md::Align::Right)));
        assert!(m.text().lines().nth(1).unwrap().contains("--:"), "{:?}", m.text().lines().nth(1));
        assert!(m.table(TableEdit::DeleteColumn));
        assert!(m.table(TableEdit::DeleteRow));
        assert_eq!(m.text().lines().filter(|l| l.starts_with('|')).count(), 3);
        assert!(m.undo());
        assert_eq!(m.text().lines().filter(|l| l.starts_with('|')).count(), 4);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typing_in_a_table_tab_between_cells_and_enter_for_a_new_row() {
        let (mut m, dir) = model("typing", TABLE);
        caret(&mut m, 2);
        // Tab goes cell to cell (not a tab character), Shift+Tab back.
        assert!(m.tab(false));
        assert_eq!(m.selected_text(), "bc");
        assert!(m.tab(false));
        assert_eq!(m.selected_text(), "1");
        assert!(m.tab(true));
        assert_eq!(m.selected_text(), "bc");
        assert!(!m.text().contains('\t'));
        // Past the last cell: a new row, with the caret in it.
        m.tab(false);
        m.tab(false);
        assert_eq!(m.selected_text(), "2");
        m.tab(false);
        assert_eq!(m.text().lines().filter(|l| l.starts_with('|')).count(), 4);
        m.insert("x");
        assert!(m.text().lines().nth(3).unwrap().contains('x'));
        // Enter in a cell adds a row below, and the typed text stays in its cell.
        assert!(m.newline(false));
        assert_eq!(m.text().lines().filter(|l| l.starts_with('|')).count(), 5);
        m.insert("y");
        assert!(m.text().lines().nth(4).unwrap().contains('y'));
        // One undo takes back the typed letter, another the row.
        assert!(m.undo());
        assert!(!m.text().contains('y'));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_right_click_on_the_note_moves_the_caret_there_and_elsewhere_gives_no_menu() {
        let (mut m, dir) = model("click", TABLE);
        let (r, _) = m.hits.iter().find(|(_, t)| *t == Target::Note).copied().expect("the note is on screen");
        // Off the note and the tree (where the toolbar comes out) there is no menu.
        let (t, _) = m.hits.iter().find(|(_, t)| *t == Target::RevealToolbar).copied().unwrap();
        assert!(m.context_menu(t.left + 2.0, t.top + 2.0, false).is_empty());
        let items = m.context_menu((r.left + r.right) / 2.0, r.top + 20.0, false);
        assert!(items.iter().any(|i| i.action == MenuAction::SelectAll));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_link_under_the_right_click_comes_first_and_opens_its_note_or_page() {
        let note = "# Links\n\nSee [[Other]] and https://example.com and #idea\n";
        let (mut m, dir) = model("links", note);
        std::fs::write(dir.join("Other.md"), "# Other\n\nIts text.\n").unwrap();
        let at = note.find("Other").unwrap() + 1;
        caret(&mut m, at);
        let items = m.text_menu(Some(at), false);
        assert_eq!(items[0].label, "Open Link");
        let (changed, effects) = m.run_menu(items[0].action.clone());
        assert!(changed && effects.is_empty());
        assert_eq!(m.popup_note(), Some("Other.md"), "a linked note opens over the page");
        m.close_popup();
        // A web page goes to the browser.
        let (_, effects) = m.follow_link(note.find("example").unwrap());
        assert_eq!(effects, [Effect::Url("https://example.com".into())]);
        // A tag is searched for.
        m.follow_link(note.find("#idea").unwrap() + 1);
        assert_eq!(m.search.as_ref().map(|s| s.query.text().to_string()), Some("#idea".into()));
        // A link to a note that is not there yet makes it.
        m.close_search();
        let note2 = "# Links\n\n[[Brand new]]\n";
        m.editor = Editor::new(note2);
        m.follow_link(note2.find("Brand").unwrap());
        assert!(dir.join("Brand new.md").is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_command_click_on_a_link_follows_it_and_elsewhere_is_a_click() {
        let note = "# Links\n\nSee [[Other]] now\n";
        let (mut m, dir) = model("cmd-click", note);
        std::fs::write(dir.join("Other.md"), "# Other\n").unwrap();
        // Where the link is drawn: the paragraph's line, at its start.
        let i = m.lines.iter().position(|l| m.text()[l.para.start..].starts_with("See")).unwrap();
        let (lx, ly) = (m.lines[i].x, m.lines[i].y);
        let (x, h) = {
            let nt = m.note_text();
            let (x, _, h) = nt.caret(&m.lines[i].para, note.find("Other").unwrap() + 2).unwrap();
            (x, h)
        };
        let y = ly + h / 2.0;
        assert!(m.link_click(lx + x, y).is_some());
        assert_eq!(m.popup_note(), Some("Other.md"));
        m.close_popup();
        assert!(m.link_click(lx + 1.0, y).is_none(), "\"See\" is no link");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_rows_menu_opens_renames_duplicates_and_deletes_its_note() {
        let (mut m, dir) = model("rows", "# Note\n\nBody\n");
        std::fs::create_dir_all(dir.join("Folder")).unwrap();
        std::fs::write(dir.join("Other.md"), "# Other\n\nIts text.\n").unwrap();
        m.refresh_tree();
        m.frame(2000, 1440, 2.0);
        let row = |m: &Model, name: &str| m.session.sidebar.iter().position(|r| r.name == name).unwrap();
        let other = row(&m, "Other");
        assert_eq!(labels(&m.row_menu(other)), ["Open", "Open in New Window", "Rename", "Duplicate", "Copy Link to Note", "Add to Favorites", "Show in Finder", "Delete"]);
        assert_eq!(labels(&m.row_menu(row(&m, "Folder"))), ["New Note Here", "Add to Favorites", "Show in Finder"]);
        // Rename: the note opens with its title selected.
        m.run_menu(MenuAction::Rename("Other.md".into()));
        assert_eq!(m.session.note, "Other.md");
        assert_eq!(m.selected_text(), "Other");
        // Duplicate: a numbered copy beside it, opened.
        m.run_menu(MenuAction::Duplicate("Other.md".into()));
        assert_eq!(m.session.note, "Other 2.md");
        assert_eq!(m.whispered(), Some("Duplicated as Other 2"));
        // Delete another note: it goes to the trash, the page stays.
        assert!(m.run_menu(MenuAction::Delete("Note.md".into())).0);
        assert!(!dir.join("Note.md").exists());
        assert_eq!(m.session.note, "Other 2.md");
        // A link to it, on the clipboard.
        let (_, effects) = m.run_menu(MenuAction::CopyLink("Other".into()));
        assert!(matches!(&effects[0], Effect::Clipboard(t, _) if t == "[[Other]]"));
        // A new note in the folder saves there.
        m.run_menu(MenuAction::NewNoteIn("Folder".into()));
        m.insert("Inside the folder");
        m.save();
        assert!(dir.join("Folder").join("Inside the folder.md").is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_send_button_offers_the_apps_found_then_export_and_copy() {
        let (mut m, dir) = model("send", "# Note\n\nBody\n");
        let items = m.send_menu_with(vec![Agent::Claude, Agent::Codex]);
        assert_eq!(labels(&items), ["Share Link\u{2026}", "New Chat in Claude", "New Session in Codex", "Export Note\u{2026}", "Copy"]);
        assert!(items[1].divider && items[3].divider);
        // A click on the send button asks the view for the menu, under it.
        let (r, _) = m.hits.iter().find(|(_, t)| *t == Target::SendTo).copied().expect("the send button");
        assert!(m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false));
        let req = m.take_menu_request().expect("a menu");
        assert!(req.y > r.bottom && labels(&req.items).contains(&"Copy"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_send_button_offers_share_link_first() {
        let (mut m, dir) = model("share", "# Note\n\nBody\n");
        for agents in [vec![], vec![Agent::Claude]] {
            let items = m.send_menu_with(agents);
            assert_eq!(items[0].action, MenuAction::ShareLink);
            assert!(!items[0].divider && items[1].divider);
        }
        // Picked: the share card opens under Send.
        assert_eq!(m.run_menu(MenuAction::ShareLink), (true, vec![]));
        m.frame(2000, 1440, 2.0);
        let send = m.hits.iter().find(|(_, t)| *t == Target::SendTo).map(|(r, _)| *r).unwrap();
        let card = m.hits.iter().find(|(_, t)| *t == Target::Share(kolada_ui::share::Hit::Card)).map(|(r, _)| *r).expect("the card");
        assert!(card.top >= send.bottom);
        // Escape closes it.
        assert!(m.escape() && !m.sharing());
        let _ = std::fs::remove_dir_all(dir);
    }
}
