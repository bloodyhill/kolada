//! The command palette: Ctrl+K (at a bare caret: with text selected it is
//! Link) or Ctrl+P drops a panel from the top of the note column with a
//! field to type a command in and what matches, the keys that do each
//! beside it, so everything is a few letters away and the keys are learned
//! on the way (`kolada_ui::palette` draws it). `>` in the search field
//! lists the same. The commands are the shared list
//! (`kolada_ui::commands`), with what only Windows has.

use kolada_ui::commands::{self, Id};
use kolada_ui::scene::Platform;

use super::menu::Action;
use super::share::Agent;
use super::*;
use crate::scene::{MenuItem, Transfer};

/// What a command does.
#[derive(Clone)]
pub(super) enum Run {
    /// What a click on this does.
    Click(Target),
    /// What this menu item does.
    Act(Action),
    Choose(Setting, u8),
    /// Back or forward through the notes visited.
    Step(bool),
    /// The find bar, with replace or without.
    Find(bool),
    /// Back to finding notes.
    Search,
    Today,
    /// The welcome screen, again.
    Welcome,
    /// The note's past.
    History,
    /// A move on the note's lines, carets or sections.
    Edit(Id),
    Social(kolada_core::social::Platform),
    Quit,
}

pub(super) struct Command {
    pub item: MenuItem,
    /// Other words it is found by.
    words: &'static str,
    pub run: Run,
    /// The shared command it is, if it is one (see `kolada_ui::commands`).
    pub id: Option<Id>,
}

fn command(icon: &'static str, label: &str, keys: &'static str, words: &'static str, run: Run) -> Command {
    Command { item: MenuItem { icon, label: label.to_string(), keys, ..MenuItem::default() }, words, run, id: None }
}

/// Segoe Fluent Icons, as the rest of the app uses them.
mod icon {
    pub const ADD: &str = "\u{E710}";
    pub const MINIMAL: &str = "\u{E73F}";
    pub const TO_MAIN: &str = "\u{E740}";
    pub const SEARCH: &str = "\u{E721}";
    pub const TODAY: &str = "\u{E787}";
    pub const REPLACE: &str = "\u{E8AB}";
    pub const BACK: &str = "\u{E72B}";
    pub const FORWARD: &str = "\u{E72A}";
    pub const HIGHLIGHT: &str = "\u{E7E6}";
    pub const WINDOW: &str = "\u{E8A7}";
    pub const RENAME: &str = "\u{E8AC}";
    pub const COPY: &str = "\u{E8C8}";
    pub const LINK: &str = "\u{E71B}";
    pub const FOLDER: &str = "\u{E838}";
    pub const DELETE: &str = "\u{E74D}";
    pub const HEADING: &str = "\u{E8D2}";
    pub const BOLD: &str = "\u{E8DD}";
    pub const ITALIC: &str = "\u{E8DB}";
    pub const STRIKE: &str = "\u{EDE0}";
    pub const LIST: &str = "\u{E8FD}";
    pub const CHECKLIST: &str = "\u{E9D5}";
    pub const QUOTE: &str = "\u{E9B2}";
    pub const CODE: &str = "\u{E943}";
    pub const EXPORT: &str = "\u{EDE1}";
    pub const IMPORT: &str = "\u{E8B5}";
    pub const PASTE: &str = "\u{E77F}";
    pub const SUN: &str = "\u{E706}";
    pub const MOON: &str = "\u{E708}";
    pub const SYSTEM: &str = "\u{E7A1}";
    pub const LARGER: &str = "\u{E8E8}";
    pub const SMALLER: &str = "\u{E8E7}";
    pub const WIDTH: &str = "\u{E9A6}";
    pub const PANEL: &str = "\u{E8A0}";
    pub const GRID: &str = "\u{E80A}";
    pub const TOOLBAR: &str = "\u{E70E}";
    pub const SPELL: &str = "\u{F87B}";
    pub const WRITING: &str = "\u{E70F}";
    pub const RELATED: &str = "\u{E71B}";
    pub const QUICK: &str = "\u{E765}";
    pub const SOCIAL: &str = "\u{E8BD}";
    pub const VAULT: &str = "\u{E8B7}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const QUIT: &str = "\u{E7E8}";
    pub const HISTORY: &str = "\u{E81C}";
    pub const UP: &str = "\u{E70E}";
    pub const DOWN: &str = "\u{E70D}";
    pub const SORT: &str = "\u{E8FD}";
    pub const CARETS: &str = "\u{E721}";
    pub const FOLD: &str = "\u{E76C}";
    pub const UNFOLD: &str = "\u{E70D}";
}

/// The commands for a query, best first (see `commands::ranked`); with no
/// query, what ran last, then the rest.
pub(super) fn ranked(all: &[Command], query: &str, recent: &[Id]) -> Vec<usize> {
    if query.trim().is_empty() {
        let ids: Vec<Option<Id>> = all.iter().map(|c| c.id).collect();
        return commands::at_rest(&ids, recent);
    }
    let entries: Vec<(&str, &str)> = all.iter().map(|c| (c.item.label.as_str(), c.words)).collect();
    commands::ranked(&entries, query)
}

impl App {
    /// What the shared command `id` does here, and its icon; None where it
    /// does not apply now (no note open, a page that cannot be edited).
    fn run_of(&self, id: Id) -> Option<(&'static str, Run)> {
        let note = !self.session.note.is_empty();
        if (commands::needs_note(id) && !note) || (commands::edits(id) && self.read_only) {
            return None;
        }
        let path = || self.note_path();
        Some(match id {
            Id::NewNote => (icon::ADD, Run::Click(Target::NewNote)),
            Id::Today => (icon::TODAY, Run::Today),
            Id::Due => (icon::TODAY, Run::Click(Target::DueDot)),
            Id::SearchNotes => (icon::SEARCH, Run::Search),
            Id::Find => (icon::SEARCH, Run::Find(false)),
            Id::Replace => (icon::REPLACE, Run::Find(true)),
            Id::Back => (icon::BACK, Run::Step(false)),
            Id::Forward => (icon::FORWARD, Run::Step(true)),
            Id::Highlights => (icon::HIGHLIGHT, Run::Click(Target::Highlights)),
            Id::History => (icon::HISTORY, Run::History),
            Id::OpenWindow if self.compact => return None,
            Id::OpenWindow => (icon::WINDOW, Run::Act(Action::OpenWindow(path()))),
            Id::Rename => (icon::RENAME, Run::Act(Action::Rename(path()))),
            Id::DuplicateNote => (icon::COPY, Run::Act(Action::Duplicate(path()))),
            Id::CopyLink => (icon::LINK, Run::Act(Action::CopyLink(stem(&path())))),
            Id::TrashNote => (icon::DELETE, Run::Act(Action::Delete(self.session.note.clone()))),
            Id::Trash => (icon::DELETE, Run::Click(Target::Trash)),
            Id::Heading => (icon::HEADING, Run::Click(Target::Tool(Tool::Heading))),
            Id::Bold => (icon::BOLD, Run::Click(Target::Tool(Tool::Bold))),
            Id::Italic => (icon::ITALIC, Run::Click(Target::Tool(Tool::Italic))),
            Id::Strike => (icon::STRIKE, Run::Click(Target::Tool(Tool::Strike))),
            Id::Highlight => (icon::HIGHLIGHT, Run::Act(Action::Highlight)),
            Id::List => (icon::LIST, Run::Click(Target::Tool(Tool::List))),
            Id::Checklist => (icon::CHECKLIST, Run::Click(Target::Tool(Tool::Checklist))),
            Id::Quote => (icon::QUOTE, Run::Click(Target::Tool(Tool::Quote))),
            Id::Code => (icon::CODE, Run::Click(Target::Tool(Tool::Code))),
            Id::Link => (icon::LINK, Run::Click(Target::Tool(Tool::Link))),
            Id::MoveLineUp => (icon::UP, Run::Edit(id)),
            Id::MoveLineDown => (icon::DOWN, Run::Edit(id)),
            Id::DuplicateLine => (icon::COPY, Run::Edit(id)),
            Id::SortLines => (icon::SORT, Run::Edit(id)),
            Id::NextMatch => (icon::CARETS, Run::Edit(id)),
            Id::Fold => (icon::FOLD, Run::Edit(id)),
            Id::Unfold => (icon::UNFOLD, Run::Edit(id)),
            Id::CopyForAi => (icon::COPY, Run::Act(Action::CopyMarkdown)),
            Id::CopyRich => (icon::COPY, Run::Act(Action::CopyRich)),
            Id::ExportMarkdown => (icon::EXPORT, Run::Click(Target::Transfer(Transfer::Markdown))),
            Id::ExportPage => (icon::EXPORT, Run::Click(Target::Transfer(Transfer::Page))),
            Id::ExportPdf => (icon::EXPORT, Run::Click(Target::Transfer(Transfer::Pdf))),
            Id::Import => (icon::IMPORT, Run::Click(Target::Transfer(Transfer::Files))),
            Id::ImportFolder => (icon::IMPORT, Run::Click(Target::Transfer(Transfer::Folder))),
            Id::FromClipboard => (icon::PASTE, Run::Click(Target::Transfer(Transfer::Clipboard))),
            Id::Welcome => (icon::IMPORT, Run::Welcome),
            Id::LargerText => (icon::LARGER, Run::Click(Target::TextSize(true))),
            Id::SmallerText => (icon::SMALLER, Run::Click(Target::TextSize(false))),
            Id::LineWidth => (icon::WIDTH, Run::Click(Target::LineWidth)),
            Id::OpenVault => (icon::VAULT, Run::Click(Target::VaultOpen)),
            Id::Settings => (icon::SETTINGS, Run::Click(Target::Options)),
            Id::Quit => (icon::QUIT, Run::Quit),
        })
    }

    /// Everything the palette offers now: the shared commands that apply
    /// (see `run_of`), then what only Windows has: the agents installed,
    /// importing from the Notes app, and settings by what they would change
    /// to.
    pub(super) fn commands(&self) -> Vec<Command> {
        let mut out: Vec<Command> = commands::ALL
            .iter()
            .filter_map(|c| {
                let (glyph, run) = self.run_of(c.id)?;
                let mut cmd = command(glyph, c.name, c.keys(Platform::Windows), c.words, run);
                cmd.item.danger = c.id == Id::TrashNote;
                cmd.id = Some(c.id);
                Some(cmd)
            })
            .collect();
        for p in kolada_core::social::PLATFORMS {
            out.push(command(icon::SOCIAL, &format!("Preview for {}", p.name()), "", "social post caption length see more", Run::Social(p)));
        }
        for agent in Agent::detect() {
            // As the title bar's Send menu has them: where each opens.
            out.push(command(agent.icon(), agent.label(), agent.place(), "send ai chat ask agent share", Run::Act(Action::SendTo(agent))));
        }
        if !self.session.note.is_empty() {
            out.push(command(icon::FOLDER, "Show in Explorer", "", "folder file reveal", Run::Act(Action::Reveal(self.note_path()))));
        }
        out.push(command(icon::IMPORT, "Import from Notes (.nbk)\u{2026}", "", "nbk export backup foss notes app", Run::Click(Target::Transfer(Transfer::NotesApp))));
        // The themes it is not in.
        for (value, icon, label) in [(1, icon::SUN, "Light theme"), (2, icon::MOON, "Dark theme"), (0, icon::SYSTEM, "Theme like Windows")] {
            if self.session.theme != value {
                out.push(command(icon, label, "", "appearance mode color", Run::Choose(Setting::Theme, value)));
            }
        }
        // The window minimal, or full again (a note's own window has its
        // own way back).
        if !self.compact {
            let minimal = self.session.minimal;
            let (glyph, label, target) =
                if minimal { (icon::TO_MAIN, "Full window", Target::ToMain) } else { (icon::MINIMAL, "Minimal window", Target::Minimal) };
            out.push(command(glyph, label, "", "focus small compact mini popup note alone", Run::Click(target)));
        }
        // The view, as its button says: the one a press switches to.
        let grid = self.grid.is_some();
        out.push(command(icon::GRID, if grid { "Show as list" } else { "Show as grid" }, "", "view notes boxes cards layout side menu", Run::Click(Target::ViewToggle)));
        let (sidebar, toolbar) = (self.session.sidebar_collapsed, self.session.toolbar_collapsed);
        let on_off = |on: bool, what: &str| format!("Turn {what} {}", if on { "off" } else { "on" });
        // The side menu's own commands: not while the grid is the view.
        if !grid {
            out.push(command(icon::PANEL, if sidebar { "Keep the side menu open" } else { "Tuck the side menu away" }, "Ctrl+\\", "sidebar panel focus", Run::Click(Target::ToggleSidebar)));
        }
        out.extend([
            command(icon::TOOLBAR, if toolbar { "Keep the toolbar open" } else { "Tuck the toolbar away" }, "Ctrl+Shift+\\", "format bar focus", Run::Click(Target::ToggleToolbar)),
            command(icon::SPELL, &on_off(self.session.spell == 0, "spell check"), "", "spelling dictionary", Run::Choose(Setting::Spell, if self.session.spell == 0 { 1 } else { 0 })),
            command(icon::WRITING, &on_off(self.session.assist == 0, "writing help"), "", "grammar fix lists gist summary ai model assistant", Run::Choose(Setting::Assist, if self.session.assist == 0 { kolada_ui::modelcard::OFF } else { kolada_ui::modelcard::setting_value(false, super::assist::chosen_index(&self.session)) })),
            command(icon::QUICK, &on_off(self.session.hotkey == 0, "the Win+Alt+N quick note"), "", "hotkey shortcut global capture", Run::Choose(Setting::Hotkey, if self.session.hotkey == 0 { 1 } else { 0 })),
            command(icon::RELATED, &on_off(self.session.related == 0, "related notes"), "", "margin memory magnet chips", Run::Choose(Setting::Related, if self.session.related == 0 { 1 } else { 0 })),
            command(icon::FOLDER, "Show vault in Explorer", "", "folder", Run::Click(Target::VaultShow)),
        ]);
        out
    }

    /// A line, caret or section move from the palette.
    fn run_edit(&mut self, id: Id) {
        match id {
            Id::Fold => self.fold_here(true),
            Id::Unfold => self.fold_here(false),
            _ => self.edit(|e| {
                match id {
                    Id::MoveLineUp => e.move_lines(true),
                    Id::MoveLineDown => e.move_lines(false),
                    Id::DuplicateLine => {
                        e.duplicate_lines(false);
                        true
                    }
                    Id::SortLines => e.sort_lines(),
                    _ => e.add_next_match(),
                };
            }),
        }
    }

    /// Ctrl+K and Ctrl+P: the palette drops in, its field empty, the
    /// commands run last under it; again, it goes.
    pub(super) fn open_palette(&mut self) {
        if self.palette.as_ref().is_some_and(|p| !p.closing) {
            return self.close_palette();
        }
        self.close_search();
        self.hide_link_card();
        let mut p = Palette::new(self.commands());
        p.refilter(self.recent.ids());
        self.palette = Some(p);
        self.request_frame();
    }

    /// Esc, a click beside it, or a command run: it springs back up.
    pub(super) fn close_palette(&mut self) {
        if let Some(p) = &mut self.palette {
            p.closing = true;
            p.drop.to = 0.0;
        }
        self.request_frame();
    }

    /// The palette's springs, a frame on; false once they rest (a closed
    /// palette is gone then).
    pub(super) fn step_palette(&mut self, dt: f32) -> bool {
        let Some(p) = &mut self.palette else { return false };
        let moving = p.step(dt);
        if !moving && p.closing {
            self.palette = None;
        }
        moving
    }

    /// The palette as the scene draws it.
    pub(super) fn palette_view(&self) -> Option<kolada_ui::palette::PaletteView<'_>> {
        let p = self.palette.as_ref()?;
        Some(kolada_ui::palette::PaletteView {
            query: p.query.text(),
            caret: p.query.selection().head,
            rows: &p.shown,
            first: p.first,
            chosen: p.chosen,
            drop: p.drop.at,
            height: p.height.at,
            fill: p.fill.at - p.first as f32,
        })
    }

    /// A key while the palette is open: it has them all.
    pub(super) fn palette_key(&mut self, vk: u16, ctrl: bool, shift: bool) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_TAB, VK_UP};
        let Some(p) = &mut self.palette else { return false };
        let k = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| key.0 == vk;
        let n = p.shown.len();
        let field = |p: &mut Palette, f: &dyn Fn(&mut Editor)| {
            f(&mut p.query);
        };
        if k(VK_ESCAPE) {
            self.close_palette();
            return true;
        }
        if k(VK_RETURN) {
            let chosen = p.chosen;
            self.run_palette_row(chosen);
            return true;
        }
        if k(VK_DOWN) || (k(VK_TAB) && !shift) {
            p.choose(if n == 0 { 0 } else { (p.chosen + 1) % n });
        } else if k(VK_UP) || (k(VK_TAB) && shift) {
            p.choose(if n == 0 { 0 } else { (p.chosen + n - 1) % n });
        } else if k(VK_NEXT) {
            p.choose((p.chosen + kolada_ui::palette::SHOWN).min(n.saturating_sub(1)));
        } else if k(VK_PRIOR) {
            p.choose(p.chosen.saturating_sub(kolada_ui::palette::SHOWN));
        } else if k(VK_BACK) {
            field(p, &|e| e.backspace(ctrl));
            let recent = self.recent.ids().to_vec();
            if let Some(p) = &mut self.palette {
                p.refilter(&recent);
            }
        } else if k(VK_DELETE) {
            field(p, &|e| e.delete(ctrl));
            let recent = self.recent.ids().to_vec();
            if let Some(p) = &mut self.palette {
                p.refilter(&recent);
            }
        } else if k(VK_LEFT) || k(VK_RIGHT) || k(VK_HOME) || k(VK_END) {
            let head = p.query.selection().head;
            let text = p.query.text();
            let at = if k(VK_HOME) {
                0
            } else if k(VK_END) {
                text.len()
            } else if k(VK_LEFT) {
                text[..head].char_indices().next_back().map_or(0, |(i, _)| i)
            } else {
                text[head..].chars().next().map_or(head, |c| head + c.len_utf8())
            };
            p.query.set_selection(Selection::caret(at));
        } else if ctrl && vk == b'A' as u16 {
            p.query.select_all();
        } else if ctrl && (vk == b'K' as u16 || vk == b'P' as u16) {
            self.close_palette();
        } else if ctrl && vk == b'V' as u16 {
            let pasted = crate::clipboard::get(self.hwnd).unwrap_or_default();
            let line = pasted.lines().next().unwrap_or("").to_string();
            self.palette_typed(&line);
        } else {
            // Other shortcuts wait until it is closed.
            return true;
        }
        self.request_frame();
        true
    }

    /// Text typed into the palette's field.
    pub(super) fn palette_typed(&mut self, s: &str) {
        let recent = self.recent.ids().to_vec();
        if let Some(p) = &mut self.palette {
            p.query.insert(s);
            p.refilter(&recent);
        }
        self.request_frame();
    }

    /// The pointer came over a row: it is the chosen one.
    pub(super) fn palette_hover(&mut self, row: usize) {
        if let Some(p) = &mut self.palette
            && p.chosen != row
        {
            p.choose(row);
            self.request_frame();
        }
    }

    /// Runs the palette's `i`th row: it closes as the command runs.
    pub(super) fn run_palette_row(&mut self, i: usize) {
        let Some(p) = &self.palette else { return };
        let Some(&k) = p.order.get(i) else { return };
        let (run, id) = (p.all[k].run.clone(), p.all[k].id);
        if let Some(id) = id {
            self.recent.ran(id);
        }
        self.close_palette();
        self.run(run);
    }

    /// Runs the `i`th command shown in the search's `>` list.
    pub(super) fn run_command(&mut self, i: usize) {
        let Some(run) = self.search.as_ref().and_then(|s| s.runs.get(i)).cloned() else { return };
        if let Some(id) = self.search.as_ref().and_then(|s| s.ids.get(i).copied().flatten()) {
            self.recent.ran(id);
        }
        if let Run::Search = run {
            if let Some(s) = &mut self.search {
                s.query = Editor::field("");
            }
            return self.run_search();
        }
        self.close_search();
        self.run(run);
    }

    fn run(&mut self, run: Run) {
        match run {
            Run::Click(target) => self.activate(target),
            Run::Act(action) => self.run_action(action, false),
            Run::Choose(setting, value) => self.choose(setting, value),
            Run::Step(forward) => self.step(forward),
            Run::Find(replace) => self.open_find(replace),
            Run::Search => self.open_search(),
            Run::Today => self.open_today(),
            Run::Welcome => self.show_welcome(),
            Run::History => self.toggle_scrub(),
            Run::Edit(id) => self.run_edit(id),
            Run::Social(p) => self.show_social(Some(p)),
            Run::Quit => self.quit(),
        }
    }
}

/// A value moving to `to` as a spring: a little overshoot, then settled.
#[derive(Clone, Copy, Debug)]
pub(super) struct Spring {
    pub at: f32,
    pub to: f32,
    v: f32,
}

impl Spring {
    fn new(at: f32) -> Spring {
        Spring { at, to: at, v: 0.0 }
    }

    /// A frame on (`dt` seconds, in small steps so a slow frame stays
    /// stable); false once it rests.
    fn step(&mut self, dt: f32, stiffness: f32, damping: f32) -> bool {
        let mut left = dt.min(0.1);
        while left > 0.0 {
            let h = left.min(1.0 / 240.0);
            let a = -stiffness * (self.at - self.to) - damping * self.v;
            self.v += a * h;
            self.at += self.v * h;
            left -= h;
        }
        if (self.at - self.to).abs() < 0.002 && self.v.abs() < 0.02 {
            self.at = self.to;
            self.v = 0.0;
            return false;
        }
        true
    }
}

/// The palette while it shows: what is typed, every command it can run,
/// those that match (best first) and the one chosen, and its springs.
pub(super) struct Palette {
    query: Editor,
    all: Vec<Command>,
    /// The matches, by index into `all`, and their rows as drawn.
    order: Vec<usize>,
    shown: Vec<MenuItem>,
    chosen: usize,
    first: usize,
    /// How far it has dropped in (0..1), the rows' room (in rows), and
    /// where the chosen row's fill is (in rows).
    drop: Spring,
    height: Spring,
    fill: Spring,
    pub closing: bool,
}

impl Palette {
    fn new(all: Vec<Command>) -> Palette {
        Palette {
            query: Editor::field(""),
            all,
            order: Vec::new(),
            shown: Vec::new(),
            chosen: 0,
            first: 0,
            drop: Spring { at: 0.0, to: 1.0, v: 0.0 },
            height: Spring::new(0.0),
            fill: Spring::new(0.0),
            closing: false,
        }
    }

    /// What matches what is typed now; with nothing typed, the commands
    /// run last (`recent`, at most five), or none. The first is chosen.
    fn refilter(&mut self, recent: &[Id]) {
        let query = self.query.text().trim().to_string();
        self.order = if query.is_empty() {
            recent.iter().filter_map(|&id| self.all.iter().position(|c| c.id == Some(id))).take(commands::Recent::KEPT).collect()
        } else {
            let entries: Vec<(&str, &str)> = self.all.iter().map(|c| (c.item.label.as_str(), c.words)).collect();
            commands::ranked(&entries, &query)
        };
        self.shown = self.order.iter().map(|&i| self.all[i].item.clone()).collect();
        self.first = 0;
        self.chosen = 0;
        self.fill = Spring::new(0.0);
        self.height.to = kolada_ui::palette::rows_for(self.shown.len());
    }

    /// Row `i` is the chosen one: its fill glides there, and the list
    /// scrolls to keep it in view.
    fn choose(&mut self, i: usize) {
        if self.shown.is_empty() {
            return;
        }
        let i = i.min(self.shown.len() - 1);
        let shown = kolada_ui::palette::SHOWN;
        if i < self.first {
            self.first = i;
        } else if i >= self.first + shown {
            self.first = i + 1 - shown;
        }
        // In rows from the list's top: the rows scroll, and it with them.
        self.chosen = i;
        self.fill.to = i as f32;
    }

    fn step(&mut self, dt: f32) -> bool {
        let a = self.drop.step(dt, 420.0, 32.0);
        let b = self.height.step(dt, 520.0, 42.0);
        let c = self.fill.step(dt, 700.0, 46.0);
        a || b || c
    }

    pub(super) fn moving(&self) -> bool {
        [self.drop, self.height, self.fill].iter().any(|s| s.at != s.to || s.v != 0.0)
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    fn palette() -> Palette {
        let all = vec![
            Command { id: Some(Id::NewNote), ..command("", "New note", "Ctrl+N", "create page", Run::Search) },
            Command { id: Some(Id::MoveLineUp), ..command("", "Move line up", "Alt+\u{2191}", "shift", Run::Search) },
            Command { id: Some(Id::MoveLineDown), ..command("", "Move line down", "Alt+\u{2193}", "shift", Run::Search) },
            Command { id: Some(Id::TrashNote), ..command("", "Move note to trash", "", "delete", Run::Search) },
            command("", "Dark theme", "", "appearance", Run::Search),
        ];
        Palette::new(all)
    }

    fn rows(p: &Palette) -> Vec<&str> {
        p.shown.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn opened_with_nothing_typed_it_offers_what_ran_last_or_nothing() {
        let mut p = palette();
        p.refilter(&[]);
        assert!(rows(&p).is_empty(), "nothing run yet: the field alone");
        p.refilter(&[Id::TrashNote, Id::NewNote]);
        assert_eq!(rows(&p), ["Move note to trash", "New note"]);
        assert_eq!(p.height.to, 2.0, "its room grows to fit them");
    }

    #[test]
    fn typing_filters_and_the_first_match_is_chosen() {
        let mut p = palette();
        p.query.insert("move");
        p.refilter(&[]);
        assert_eq!(rows(&p), ["Move line up", "Move line down", "Move note to trash"]);
        assert_eq!(p.chosen, 0);
        p.choose(2);
        assert_eq!((p.chosen, p.fill.to), (2, 2.0), "the fill glides to the chosen row");
        p.query.insert("zz");
        p.refilter(&[]);
        assert!(rows(&p).is_empty());
        assert_eq!(p.height.to, 0.0);
    }

    #[test]
    fn it_drops_in_on_a_spring_and_comes_to_rest() {
        let mut p = palette();
        p.query.insert("move");
        p.refilter(&[]);
        let mut frames = 0;
        let mut most: f32 = 0.0;
        while p.step(1.0 / 60.0) {
            most = most.max(p.drop.at);
            frames += 1;
            assert!(frames < 120, "it rests within two seconds");
        }
        assert!(most > 1.0 && most < 1.08, "a little overshoot: {most}");
        assert_eq!((p.drop.at, p.height.at), (1.0, 3.0));
        assert!(!p.moving(), "no frames at rest");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(label: &str, words: &'static str, id: Option<Id>) -> Command {
        Command { id, ..command("", label, "", words, Run::Search) }
    }

    fn names(all: &[Command], q: &str, recent: &[Id]) -> Vec<String> {
        ranked(all, q, recent).into_iter().map(|i| all[i].item.label.clone()).collect()
    }

    #[test]
    fn finds_commands_by_name_and_offers_what_ran_last_first() {
        let all = vec![
            named("New note", "create page", Some(Id::NewNote)),
            named("Dark theme", "appearance mode", None),
            named("Sort lines", "order", Some(Id::SortLines)),
            named("Move note to trash", "delete remove", Some(Id::TrashNote)),
        ];
        assert_eq!(names(&all, "note", &[]), ["New note", "Move note to trash"]);
        assert_eq!(names(&all, "delete", &[]), ["Move note to trash"]);
        assert_eq!(names(&all, "", &[]), ["New note", "Dark theme", "Sort lines", "Move note to trash"]);
        assert_eq!(names(&all, "", &[Id::SortLines])[0], "Sort lines", "what ran last first");
    }
}
