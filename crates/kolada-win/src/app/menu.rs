//! Menus, drawn by the app like the rest of its chrome: right-click the
//! text, a file tree row, a related note or a deleted one; or the three dots
//! on a row. Arrow keys move through a menu, Enter picks, Esc closes.

use std::process::Command;

use kolada_editor::TableEdit;

use super::share::Agent;
use super::*;
use crate::scene::MenuItem;

/// A menu fades in over this long.
pub const FADE: Duration = Duration::from_millis(90);

/// What an item does.
#[derive(Clone, Debug)]
pub(super) enum Action {
    Cut,
    Copy,
    Paste,
    /// The copied text without its formatting (see `App::paste`).
    PastePlain,
    SelectAll,
    Format(Tool),
    Highlight,
    /// Follow the link at a text offset.
    FollowLink(usize, f32),
    /// Notes, by path: open as the page, rename, duplicate, copy a link to
    /// it, show it in Explorer, delete.
    Open(PathBuf),
    Rename(PathBuf),
    Duplicate(PathBuf),
    CopyLink(String),
    Reveal(PathBuf),
    Delete(String),
    /// A fresh page that saves into a folder.
    NewNoteIn(PathBuf),
    /// A related note: open it over the page, or link it at the caret.
    OpenChip(usize),
    LinkChip(usize),
    /// A deleted note, by index in the trash.
    Restore(usize),
    DeleteForGood(usize),
    /// A note in a window of its own.
    OpenWindow(PathBuf),
    /// The title bar's quick actions (see `share`).
    CopyRich,
    CopyMarkdown,
    /// The open note to a file of the kind chosen (see `App::export_note`).
    ExportNote,
    SendTo(Agent),
    /// Spelling: a word's correction (its range and the word), adding the
    /// word to the dictionary, or leaving it be.
    Correct(usize, usize, String),
    LearnWord(String),
    IgnoreWord(String),
    /// A table's rows, columns and alignment, where the caret is.
    Table(TableEdit),
    /// A note (with `.md`) or folder (with `/`) into the favorites or out.
    Favorite(String),
    /// The side menu's Recent hidden, or back.
    ToggleRecent,
}

/// A menu open at a point, DIPs; the file tree row it is for; whether its
/// text actions are the popup's.
pub struct Menu {
    pub x: f32,
    pub y: f32,
    pub items: Vec<MenuItem>,
    actions: Vec<Action>,
    pub opened: Instant,
    pub row: Option<usize>,
    /// The title bar button it opened from.
    pub anchor: Option<Target>,
    popup: bool,
}

/// Menu icons (Segoe Fluent Icons).
mod icon {
    pub const CUT: &str = "\u{E8C6}";
    pub const COPY: &str = "\u{E8C8}";
    pub const PASTE: &str = "\u{E77F}";
    pub const SELECT_ALL: &str = "\u{E8B3}";
    pub const BOLD: &str = "\u{E8DD}";
    pub const ITALIC: &str = "\u{E8DB}";
    pub const HIGHLIGHT: &str = "\u{E7E6}";
    pub const LINK: &str = "\u{E71B}";
    pub const OPEN: &str = "\u{E8E5}";
    pub const OPEN_OVER: &str = "\u{E8A7}";
    pub const RENAME: &str = "\u{E8AC}";
    pub const FOLDER: &str = "\u{E838}";
    pub const DELETE: &str = "\u{E74D}";
    pub const ADD: &str = "\u{E710}";
    pub const RESTORE: &str = "\u{E7A7}";
    pub const ALIGN_LEFT: &str = "\u{E8E4}";
    pub const ALIGN_CENTER: &str = "\u{E8E3}";
    pub const ALIGN_RIGHT: &str = "\u{E8E2}";
    pub const FAVORITE: &str = "\u{E734}";
    pub const HIDE: &str = "\u{ED1A}";
    pub const SHOW: &str = "\u{E7B3}";
}

fn item(icon: &'static str, label: &str, keys: &'static str) -> MenuItem {
    MenuItem { icon, label: label.to_string(), keys, ..MenuItem::default() }
}

/// A new group starts at this item.
fn after_divider(mut i: MenuItem) -> MenuItem {
    i.divider = true;
    i
}

fn danger(mut i: MenuItem) -> MenuItem {
    i.danger = true;
    i
}

impl App {
    pub(super) fn show_menu(&mut self, (x, y): (f32, f32), entries: Vec<(MenuItem, Action)>, row: Option<usize>, popup: bool) {
        if entries.is_empty() {
            return;
        }
        let (items, actions) = entries.into_iter().unzip();
        self.menu = Some(Menu { x, y, items, actions, opened: Instant::now(), row, anchor: None, popup });
        self.ui.options_open = false;
        self.hover_chip(None);
        self.request_frame();
    }

    pub(super) fn close_menu(&mut self) {
        if self.menu.take().is_some() {
            self.request_frame();
            self.update_reveal();
        }
    }

    /// The text's menu at a point in DIPs: editing, formatting, and the note.
    fn text_menu(&mut self, at: (f32, f32), link: Option<usize>, popup: bool) {
        let selected = if popup {
            self.popup.as_ref().is_some_and(|p| !p.editor.selection().is_caret())
        } else {
            !self.editor.selection().is_caret()
        };
        let mut entries = Vec::new();
        // A misspelled word under the click: its corrections come first.
        if !popup
            && !self.read_only
            && let Some(at) = self.menu_word
            && let Some(spell) = &mut self.spell
        {
            let line = self.editor.line_at(at);
            if let Some((a, b, word)) = spell.word_at(self.editor.text(), &line, at) {
                let fixes = spell.suggest(&word);
                if fixes.is_empty() {
                    entries.push((item("", "No suggestions", ""), Action::IgnoreWord(String::new())));
                }
                for fix in fixes {
                    entries.push((item("", &fix, ""), Action::Correct(a, b, fix.clone())));
                }
                entries.push((after_divider(item("\u{E8FB}", "Add to dictionary", "")), Action::LearnWord(word.clone())));
                entries.push((item("\u{E7BA}", "Ignore", ""), Action::IgnoreWord(word)));
            }
        }
        if let Some(offset) = link {
            let open = item(icon::OPEN_OVER, "Open link", "Ctrl+Click");
            entries.push((if entries.is_empty() { open } else { after_divider(open) }, Action::FollowLink(offset, at.1)));
        }
        let editable = popup || !self.read_only;
        // In a table: its rows, columns and alignment, the clipboard and
        // Select all, and nothing else (formatting stays on the keys and the
        // toolbar), so the menu stays short.
        let place = if popup { self.popup.as_ref().and_then(|p| p.editor.table_place()) } else { self.editor.table_place() };
        let place = place.filter(|_| editable);
        if let Some(place) = place {
            entries.push((after_divider(item(icon::ADD, "Add row below", "Enter")), Action::Table(TableEdit::RowBelow)));
            if !place.header {
                entries.push((item(icon::ADD, "Add row above", ""), Action::Table(TableEdit::RowAbove)));
            }
            entries.push((item(icon::ADD, "Add column right", ""), Action::Table(TableEdit::ColumnRight)));
            entries.push((item(icon::ADD, "Add column left", ""), Action::Table(TableEdit::ColumnLeft)));
            let aligns = [(md::Align::Left, icon::ALIGN_LEFT, "Align left"), (md::Align::Center, icon::ALIGN_CENTER, "Align center"), (md::Align::Right, icon::ALIGN_RIGHT, "Align right")];
            for (n, (align, icon, label)) in aligns.into_iter().filter(|(a, _, _)| *a != place.align).enumerate() {
                let i = item(icon, label, "");
                entries.push((if n == 0 { after_divider(i) } else { i }, Action::Table(TableEdit::Align(align))));
            }
            let mut deletes = Vec::new();
            if !place.header {
                deletes.push((danger(item(icon::DELETE, "Delete row", "")), Action::Table(TableEdit::DeleteRow)));
            }
            if place.columns > 1 {
                deletes.push((danger(item(icon::DELETE, "Delete column", "")), Action::Table(TableEdit::DeleteColumn)));
            }
            if let Some(first) = deletes.first_mut() {
                first.0.divider = true;
            }
            entries.extend(deletes);
        }
        if selected && editable {
            entries.push((after_divider(item(icon::CUT, "Cut", "Ctrl+X")), Action::Cut));
        }
        if selected {
            let copy = item(icon::COPY, "Copy", "Ctrl+C");
            entries.push((if editable { copy } else { after_divider(copy) }, Action::Copy));
        }
        if editable {
            let paste = item(icon::PASTE, "Paste", "Ctrl+V");
            entries.push((if selected { paste } else { after_divider(paste) }, Action::Paste));
            // Copied with its formatting: Paste keeps it, this does not.
            if clipboard::has_html() {
                entries.push((item(icon::PASTE, "Paste as plain text", "Ctrl+Shift+V"), Action::PastePlain));
            }
        }
        if editable && place.is_none() {
            // The whole note, formatted, for mail and documents.
            if !popup && !self.editor.text().trim().is_empty() {
                entries.push((item(icon::COPY, "Copy note as rich text", ""), Action::CopyRich));
            }
            entries.push((after_divider(item(icon::BOLD, "Bold", "Ctrl+B")), Action::Format(Tool::Bold)));
            entries.push((item(icon::ITALIC, "Italic", "Ctrl+I"), Action::Format(Tool::Italic)));
            entries.push((item(icon::HIGHLIGHT, "Highlight", ""), Action::Highlight));
            entries.push((item(icon::LINK, "Link", "Ctrl+K"), Action::Format(Tool::Link)));
        }
        entries.push((after_divider(item(icon::SELECT_ALL, "Select all", "Ctrl+A")), Action::SelectAll));
        // The note itself.
        let note = if popup { self.popup.as_ref().map(|p| p.path.clone()) } else { (!self.session.note.is_empty()).then(|| self.note_path()) };
        if let Some(path) = note.filter(|_| place.is_none())
            && let Some(rel) = vault::relative(Path::new(&self.session.vault), &path)
        {
            entries.push((after_divider(item(icon::LINK, "Copy link to note", "")), Action::CopyLink(stem(&path))));
            entries.push((danger(item(icon::DELETE, "Delete note", "")), Action::Delete(rel)));
        }
        if entries.first().is_some_and(|(i, _)| i.divider) {
            entries[0].0.divider = false;
        }
        self.show_menu(at, entries, None, popup);
    }

    /// Right-click at a client point in pixels.
    pub(super) fn context_menu(&mut self, px: i32, py: i32) {
        self.close_menu();
        // The past shown has nothing to cut, paste or format.
        if self.scrub_page().is_some() && self.hit_px(px, py) == Some(Target::Note) {
            return;
        }
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let at = (px as f32 / scale, py as f32 / scale);
        match self.hit_px(px, py) {
            Some(Target::Note) => {
                if self.search.is_some() {
                    self.close_search();
                }
                self.close_popup();
                self.ensure_drawn();
                let offset = self.offset_at(at.0, at.1);
                // A click outside the selection moves the caret there first.
                let s = self.editor.selection();
                if s.is_caret() || offset < s.start() || offset > s.end() {
                    self.editor.set_selection(Selection::caret(offset));
                    self.caret_moved(false);
                }
                let link = link_at(self.editor.text(), offset).map(|_| offset);
                self.menu_word = Some(offset);
                self.text_menu(at, link, false);
                self.menu_word = None;
            }
            Some(Target::PopupText) => {
                let Some(offset) = self.popup_offset(at.0, at.1) else { return };
                if let Some(p) = &mut self.popup {
                    let s = p.editor.selection();
                    if s.is_caret() || offset < s.start() || offset > s.end() {
                        p.editor.set_selection(Selection::caret(offset));
                    }
                }
                self.text_menu(at, None, true);
            }
            Some(Target::Row(i) | Target::RowMenu(i)) => self.open_row_menu(i, Some(at)),
            Some(Target::Favorite(i)) => {
                let Some(fav) = self.session.favorites.get(i).cloned() else { return };
                let entries = vec![(item(icon::FAVORITE, "Remove from favorites", ""), Action::Favorite(fav))];
                self.show_menu(at, entries, None, false);
            }
            Some(Target::Recent(i)) => {
                let Some(note) = self.session.recent.get(i).cloned() else { return };
                let mut entries = vec![(item(icon::FAVORITE, self.favorite_label(&note), ""), Action::Favorite(note))];
                entries.extend(self.recent_toggle().map(|(i, a)| (after_divider(i), a)));
                self.show_menu(at, entries, None, false);
            }
            // Anywhere else in the side menu: Recent hidden, or back.
            Some(Target::SidebarPanel | Target::FoldRecent | Target::FoldFavorites) => {
                if let Some(entry) = self.recent_toggle() {
                    self.show_menu(at, vec![entry], None, false);
                }
            }
            Some(Target::Chip(i)) => {
                let entries = vec![
                    (item(icon::OPEN_OVER, "Open over the page", "Click"), Action::OpenChip(i)),
                    (item(icon::LINK, "Link it here", "Drag"), Action::LinkChip(i)),
                ];
                self.show_menu(at, entries, None, false);
            }
            Some(Target::TrashRow(i) | Target::TrashRestore(i)) => {
                let entries = vec![
                    (item(icon::RESTORE, "Bring back", "Click"), Action::Restore(i)),
                    (danger(item(icon::DELETE, "Move to Recycle Bin", "")), Action::DeleteForGood(i)),
                ];
                self.show_menu(at, entries, None, false);
            }
            _ => {}
        }
    }

    /// The text's menu at the caret (the menu key, or Shift+F10).
    pub(super) fn open_text_menu_at_caret(&mut self) {
        let Some(c) = self.caret_target else { return };
        let head = self.editor.selection().head;
        let link = link_at(self.editor.text(), head).map(|_| head);
        self.text_menu((c.right + 2.0, c.bottom + 2.0), link, false);
    }

    /// The menu's words for a note or folder: into the favorites, or out.
    /// "Hide Recent" or "Show Recent", where the vault is big enough to
    /// have it (see `session::shows_recent`).
    fn recent_toggle(&self) -> Option<(MenuItem, Action)> {
        if self.session.folded & session::MANY_NOTES == 0 {
            return None;
        }
        let shown = self.session.folded & session::HIDE_RECENT == 0;
        Some((if shown { item(icon::HIDE, "Hide Recent", "") } else { item(icon::SHOW, "Show Recent", "") }, Action::ToggleRecent))
    }

    fn favorite_label(&self, path: &str) -> &'static str {
        if self.session.favorites.iter().any(|f| f == path) { "Remove from favorites" } else { "Add to favorites" }
    }

    /// A file tree row's menu: at a point, or below its three dots.
    pub(super) fn open_row_menu(&mut self, i: usize, at: Option<(f32, f32)>) {
        let Some(row) = self.session.sidebar.get(i).cloned() else { return };
        let at = at.or_else(|| self.hits.iter().find(|(_, t)| *t == Target::RowMenu(i)).map(|(r, _)| (r.left, r.bottom + 4.0)));
        let Some(at) = at else { return };
        let rel = vault::row_paths(&self.session.sidebar)[i].clone();
        let vault = PathBuf::from(&self.session.vault);
        let entries = match row.kind {
            RowKind::Note => {
                let path = vault.join(format!("{rel}.md"));
                vec![
                    (item(icon::OPEN, "Open", "Click"), Action::Open(path.clone())),
                    (item(icon::OPEN_OVER, "Open in new window", ""), Action::OpenWindow(path.clone())),
                    (after_divider(item(icon::RENAME, "Rename", "")), Action::Rename(path.clone())),
                    (item(icon::COPY, "Duplicate", ""), Action::Duplicate(path.clone())),
                    (item(icon::LINK, "Copy link to note", ""), Action::CopyLink(row.name.clone())),
                    (item(icon::FAVORITE, self.favorite_label(&format!("{rel}.md")), ""), Action::Favorite(format!("{rel}.md"))),
                    (item(icon::FOLDER, "Show in folder", ""), Action::Reveal(path)),
                    (after_divider(danger(item(icon::DELETE, "Delete", ""))), Action::Delete(format!("{rel}.md"))),
                ]
            }
            RowKind::Folder { .. } => {
                let dir = vault.join(&rel);
                vec![
                    (item(icon::ADD, "New note here", ""), Action::NewNoteIn(dir.clone())),
                    (item(icon::FAVORITE, self.favorite_label(&format!("{rel}/")), ""), Action::Favorite(format!("{rel}/"))),
                    (item(icon::FOLDER, "Show in folder", ""), Action::Reveal(dir)),
                ]
            }
        };
        self.show_menu(at, entries, Some(i), false);
    }

    /// Keys while a menu is open: Up and Down move, Enter picks, Esc closes;
    /// anything else closes it and goes on as usual.
    pub(super) fn menu_key(&mut self, vk: u16) -> bool {
        let Some(m) = &self.menu else { return false };
        let n = m.items.len();
        let current = match self.ui.hover {
            Some(Target::MenuItem(i)) => Some(i),
            _ => None,
        };
        match vk {
            v if v == VK_ESCAPE.0 => self.close_menu(),
            v if v == VK_DOWN.0 => self.ui.hover = Some(Target::MenuItem(current.map_or(0, |i| (i + 1) % n))),
            v if v == VK_UP.0 => self.ui.hover = Some(Target::MenuItem(current.map_or(n - 1, |i| (i + n - 1) % n))),
            v if v == VK_RETURN.0 => {
                if let Some(i) = current {
                    self.menu_action(i);
                }
            }
            _ => {
                self.close_menu();
                return false;
            }
        }
        self.request_frame();
        true
    }

    /// The `i`th item of the open menu was picked.
    pub(super) fn menu_action(&mut self, i: usize) {
        let Some(menu) = self.menu.take() else { return };
        self.request_frame();
        self.update_reveal();
        let Some(action) = menu.actions.get(i).cloned() else { return };
        self.run_action(action, menu.popup);
    }

    /// Does what a menu item does; `popup`: to the note open over the page.
    pub(super) fn run_action(&mut self, action: Action, popup: bool) {
        let popup = popup && self.popup.is_some();
        match action {
            Action::Cut if popup => {
                if let Some(p) = &self.popup {
                    clipboard::set(self.hwnd, p.editor.selected_text());
                }
                self.popup_edit(|e| e.backspace(false));
            }
            Action::Copy if popup => {
                if let Some(p) = &self.popup {
                    clipboard::set(self.hwnd, p.editor.selected_text());
                }
            }
            Action::Paste if popup => self.popup_paste(false),
            Action::PastePlain if popup => self.popup_paste(true),
            Action::SelectAll if popup => self.popup_edit(|e| e.select_all()),
            Action::Table(edit) if popup => self.popup_edit(|e| {
                e.edit_table(edit);
            }),
            Action::Highlight if popup => self.popup_edit(|e| e.toggle_inline(Inline::Highlight)),
            Action::Cut => self.cut(),
            Action::Copy => self.copy(),
            Action::Paste => self.paste(),
            Action::PastePlain => self.paste_plain(),
            Action::SelectAll => {
                self.editor.select_all();
                self.caret_moved(false);
            }
            Action::Format(tool) => self.tool(tool),
            Action::Table(edit) => self.edit(|e| {
                e.edit_table(edit);
            }),
            Action::Highlight => self.edit(|e| e.toggle_inline(Inline::Highlight)),
            Action::FollowLink(at, y) => {
                self.follow_link(at, y);
            }
            Action::Open(path) => self.open(&path),
            Action::Rename(path) => self.rename_by_title(&path),
            Action::Duplicate(path) => self.duplicate(&path),
            Action::CopyLink(name) => {
                clipboard::set(self.hwnd, &format!("[[{name}]]"));
                self.whisper(format!("Copied a link to {name}"));
            }
            Action::Favorite(path) => self.toggle_favorite(&path),
            Action::ToggleRecent => {
                self.session.folded ^= session::HIDE_RECENT;
                self.request_frame();
            }
            Action::Reveal(path) => {
                let arg = if path.is_dir() { path.as_os_str().to_owned() } else { format!("/select,{}", path.display()).into() };
                let _ = Command::new("explorer.exe").arg(arg).spawn();
            }
            Action::Delete(rel) => self.trash_note(&rel),
            Action::NewNoteIn(dir) => {
                self.fresh_page();
                self.fresh_dir = Some(dir);
            }
            Action::OpenChip(i) => self.open_chip(i),
            Action::LinkChip(i) => {
                let at = self.editor.selection().head;
                self.link_chip(i, at);
            }
            Action::Restore(i) => self.restore(i),
            Action::DeleteForGood(i) => self.delete_for_good(i),
            Action::OpenWindow(path) => self.open_window(&path, None),
            Action::CopyRich => self.copy_rich(),
            Action::ExportNote => self.export_note(),
            Action::CopyMarkdown => self.copy_markdown(),
            Action::SendTo(agent) => self.send_to(agent),
            Action::Correct(a, b, fix) => {
                if self.editor.text().get(a..b).is_some() {
                    self.editor.set_selection(Selection { anchor: a, head: b });
                    self.edit(|e| e.insert(&fix));
                }
            }
            Action::LearnWord(word) => {
                if let Some(spell) = &mut self.spell {
                    spell.add(&word);
                }
                self.request_frame();
            }
            Action::IgnoreWord(word) => {
                if !word.is_empty()
                    && let Some(spell) = &mut self.spell
                {
                    spell.ignore(&word);
                }
                self.request_frame();
            }
        }
    }

    /// Renaming is editing the title: the note opens with its title line
    /// selected, and the file follows what is typed there.
    fn rename_by_title(&mut self, path: &Path) {
        if self.session.note.is_empty() || self.note_path() != path {
            self.open(path);
        }
        let lines = self.editor.outline();
        if let Some(t) = crate::note::title_line(self.editor.text(), lines) {
            let l = lines[t];
            self.editor.set_selection(Selection { anchor: l.content().min(l.end), head: l.end });
            self.caret_moved(false);
        }
    }

    /// A copy of the note beside it, numbered, opened.
    fn duplicate(&mut self, path: &Path) {
        let Some(dir) = path.parent() else { return };
        let copy = note::unique_path(dir, &stem(path), None);
        if std::fs::copy(path, &copy).is_ok() {
            self.open(&copy);
            self.refresh_tree();
            self.start_index();
            self.whisper(format!("Duplicated as {}", stem(&copy)));
        }
    }
}
