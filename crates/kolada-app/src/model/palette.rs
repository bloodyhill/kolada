//! The command palette, as on Windows (`kolada-win` `app/palette.rs`): `>` in
//! the search field (or Cmd+P) lists what Kolada can do, by name, with the
//! keys that do it too, so everything is a few letters away and the keys are
//! learned on the way.

use kolada_core::social::Platform;
use kolada_ui::commands::{self, Id};
use kolada_ui::scene::{MenuItem, Panel, Setting, Transfer};

use super::menu::MenuAction;
use super::share::{Agent, Effect};
use super::transfer::Pick;
use super::*;

/// What a command does.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Run {
    NewNote,
    Today,
    /// The welcome screen, again.
    Welcome,
    /// What is due.
    Due,
    /// Back to finding notes.
    Search,
    /// The find bar, with replace or without.
    Find(bool),
    /// Back or forward through the notes visited.
    Step(bool),
    Highlights,
    History,
    Trash,
    Act(MenuAction),
    Tool(Tool),
    Social(Platform),
    Choose(Setting, u8),
    Panel(Panel),
    /// The notes as a grid, or as the side menu's list.
    View,
    Minimal(bool),
    TextSize(bool),
    LineWidth,
    Settings,
    Transfer(Transfer),
    Vault,
    ShowVault,
    /// A move on the note's lines, carets or sections.
    Edit(Id),
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

/// The icon font's glyphs, at the codepoints the Windows app uses.
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
    pub const QUICK: &str = "\u{E765}";
    pub const SOCIAL: &str = "\u{E8BD}";
    pub const AGENT: &str = "\u{E8BD}";
    pub const TERMINAL: &str = "\u{E756}";
    pub const VAULT: &str = "\u{E8B7}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const HISTORY: &str = "\u{E81C}";
    pub const QUIT: &str = "\u{E7E8}";
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

impl Model {
    /// Everything the palette offers now: the note's commands only with a
    /// note open, the AI apps found, settings by what they would change to.
    pub(super) fn commands(&self) -> Vec<Command> {
        self.commands_with(self.agents())
    }

    /// What the shared command `id` does here, and its icon; None where it
    /// does not apply now (no note open, a page that cannot be edited).
    fn run_of(&self, id: Id) -> Option<(&'static str, Run)> {
        let rel = self.note_rel();
        if (commands::needs_note(id) && rel.is_none()) || (commands::edits(id) && self.settings.read_only) {
            return None;
        }
        let rel = rel.unwrap_or_default();
        Some(match id {
            Id::NewNote => (icon::ADD, Run::NewNote),
            Id::Today => (icon::TODAY, Run::Today),
            Id::Due => (icon::TODAY, Run::Due),
            Id::SearchNotes => (icon::SEARCH, Run::Search),
            Id::Find => (icon::SEARCH, Run::Find(false)),
            Id::Replace => (icon::REPLACE, Run::Find(true)),
            Id::Back => (icon::BACK, Run::Step(false)),
            Id::Forward => (icon::FORWARD, Run::Step(true)),
            Id::Highlights => (icon::HIGHLIGHT, Run::Highlights),
            Id::History => (icon::HISTORY, Run::History),
            Id::OpenWindow if self.compact => return None,
            Id::OpenWindow => (icon::WINDOW, Run::Act(MenuAction::OpenWindow(rel))),
            Id::Rename => (icon::RENAME, Run::Act(MenuAction::Rename(rel))),
            Id::DuplicateNote => (icon::COPY, Run::Act(MenuAction::Duplicate(rel))),
            Id::CopyLink => {
                let name = self.path.as_deref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                (icon::LINK, Run::Act(MenuAction::CopyLink(name)))
            }
            Id::TrashNote => (icon::DELETE, Run::Act(MenuAction::Delete(rel))),
            Id::Trash => (icon::DELETE, Run::Trash),
            Id::Heading => (icon::HEADING, Run::Tool(Tool::Heading)),
            Id::Bold => (icon::BOLD, Run::Tool(Tool::Bold)),
            Id::Italic => (icon::ITALIC, Run::Tool(Tool::Italic)),
            Id::Strike => (icon::STRIKE, Run::Tool(Tool::Strike)),
            Id::Highlight => (icon::HIGHLIGHT, Run::Act(MenuAction::Highlight)),
            Id::List => (icon::LIST, Run::Tool(Tool::List)),
            Id::Checklist => (icon::CHECKLIST, Run::Tool(Tool::Checklist)),
            Id::Quote => (icon::QUOTE, Run::Tool(Tool::Quote)),
            Id::Code => (icon::CODE, Run::Tool(Tool::Code)),
            Id::Link => (icon::LINK, Run::Tool(Tool::Link)),
            Id::MoveLineUp => (icon::UP, Run::Edit(id)),
            Id::MoveLineDown => (icon::DOWN, Run::Edit(id)),
            Id::DuplicateLine => (icon::COPY, Run::Edit(id)),
            Id::SortLines => (icon::SORT, Run::Edit(id)),
            Id::NextMatch => (icon::CARETS, Run::Edit(id)),
            Id::Fold => (icon::FOLD, Run::Edit(id)),
            Id::Unfold => (icon::UNFOLD, Run::Edit(id)),
            // The Mac's Copy Note is the Markdown any AI reads.
            Id::CopyForAi => return None,
            Id::CopyRich => (icon::COPY, Run::Act(MenuAction::CopyRich)),
            Id::ExportMarkdown => (icon::EXPORT, Run::Transfer(Transfer::Markdown)),
            Id::ExportPage => (icon::EXPORT, Run::Transfer(Transfer::Page)),
            Id::ExportPdf => (icon::EXPORT, Run::Transfer(Transfer::Pdf)),
            Id::Import => (icon::IMPORT, Run::Transfer(Transfer::Files)),
            Id::ImportFolder => (icon::IMPORT, Run::Transfer(Transfer::Folder)),
            Id::FromClipboard => (icon::PASTE, Run::Transfer(Transfer::Clipboard)),
            Id::Welcome => (icon::IMPORT, Run::Welcome),
            Id::LargerText => (icon::LARGER, Run::TextSize(true)),
            Id::SmallerText => (icon::SMALLER, Run::TextSize(false)),
            Id::LineWidth => (icon::WIDTH, Run::LineWidth),
            Id::OpenVault => (icon::VAULT, Run::Vault),
            Id::Settings => (icon::SETTINGS, Run::Settings),
            Id::Quit => (icon::QUIT, Run::Quit),
        })
    }

    pub(super) fn commands_with(&self, agents: Vec<Agent>) -> Vec<Command> {
        // The keys as this keyboard names them: the Mac's Command, or Ctrl
        // and Alt on Linux (none where it binds none).
        let pc = !self.apple_keys();
        let k = |mac: &'static str, ctrl: &'static str| if pc { ctrl } else { mac };
        let mut out: Vec<Command> = commands::ALL
            .iter()
            .filter_map(|c| {
                let (glyph, run) = self.run_of(c.id)?;
                let mut cmd = command(glyph, c.name, c.keys(self.platform), c.words, run);
                cmd.item.danger = c.id == Id::TrashNote;
                cmd.id = Some(c.id);
                Some(cmd)
            })
            .collect();
        if let Some(rel) = self.note_rel() {
            out.push(command(icon::FOLDER, &format!("Show in {}", self.file_manager()), "", "folder file reveal", Run::Act(MenuAction::Reveal(self.vault.join(&rel)))));
        }
        for p in kolada_core::social::PLATFORMS.into_iter().filter(|_| self.platform != scene::Platform::Ios) {
            out.push(command(icon::SOCIAL, &format!("Preview for {}", p.name()), "", "social post caption length see more", Run::Social(p)));
        }
        for agent in agents {
            let glyph = if matches!(agent, Agent::Codex | Agent::Gemini) { icon::TERMINAL } else { icon::AGENT };
            let place = match agent {
                Agent::Claude => "App",
                Agent::Codex | Agent::Gemini => "Terminal",
                Agent::ChatGpt | Agent::Perplexity => "Browser",
            };
            out.push(command(glyph, agent.label(), place, "send ai chat ask agent share", Run::Act(MenuAction::SendTo(agent))));
        }
        // The themes it is not in.
        for (value, glyph, label) in [(1, icon::SUN, "Light theme"), (2, icon::MOON, "Dark theme"), (0, icon::SYSTEM, self.system_theme_label())] {
            if self.session.theme != value {
                out.push(command(glyph, label, "", "appearance mode color", Run::Choose(Setting::Theme, value)));
            }
        }
        if !self.compact && !self.platform.touch() {
            let minimal = self.session.minimal;
            let (glyph, label) = if minimal { (icon::TO_MAIN, "Full window") } else { (icon::MINIMAL, "Minimal window") };
            out.push(command(glyph, label, k("\u{21E7}\u{2318}M", ""), "focus small compact mini note alone", Run::Minimal(!minimal)));
        }
        // The view, as its button says: the one a press switches to (not in
        // a note's own window, nor on the phone, which has its own list).
        let grid = self.grid_shown();
        if !self.compact && self.platform != scene::Platform::Ios {
            out.push(command(icon::GRID, if grid { "Show as list" } else { "Show as grid" }, "", "view notes boxes cards layout side menu", Run::View));
        }
        let (sidebar, toolbar) = (self.session.sidebar_collapsed, self.session.toolbar_collapsed);
        let on_off = |on: bool, what: &str| format!("Turn {what} {}", if on { "off" } else { "on" });
        // The side menu's own commands: not while the grid is the view.
        if !grid {
            out.push(command(icon::PANEL, if sidebar { "Keep the side menu open" } else { "Tuck the side menu away" }, k("\u{2318}\\", "Ctrl+\\"), "sidebar panel focus", Run::Panel(Panel::Sidebar)));
        }
        out.extend([
            command(icon::TOOLBAR, if toolbar { "Keep the toolbar open" } else { "Tuck the toolbar away" }, k("\u{21E7}\u{2318}\\", "Ctrl+Shift+\\"), "format bar focus", Run::Panel(Panel::Toolbar)),
            command(icon::SPELL, &on_off(self.session.spell == 0, "spell check"), "", "spelling dictionary", Run::Choose(Setting::Spell, if self.session.spell == 0 { 1 } else { 0 })),
        ]);
        // The quick note's keys, where the shell has them (the Mac's).
        if self.platform == scene::Platform::Mac {
            out.push(command(icon::QUICK, &on_off(self.session.hotkey == 0, "the Ctrl+Option+N quick note"), "", "hotkey shortcut global capture", Run::Choose(Setting::Hotkey, if self.session.hotkey == 0 { 1 } else { 0 })));
        }
        out.push(command(icon::FOLDER, &format!("Show vault in {}", self.file_manager()), "", "folder", Run::ShowVault));
        out
    }

    /// Runs the shared command `id`, as its keys do: what each shell's key
    /// bindings call (Alt+Up, Ctrl+D...). False where it does not apply.
    pub fn run_id(&mut self, id: Id) -> bool {
        let Some((_, run)) = self.run_of(id) else { return false };
        self.recent_commands.ran(id);
        self.run(run)
    }

    /// Cmd+P: the search field, ready for a command.
    pub fn open_palette(&mut self) -> bool {
        self.open_search();
        if let Some(s) = &mut self.search {
            s.query = Editor::field(">");
            s.query.set_selection(Selection::caret(1));
            s.fresh = false;
        }
        true
    }

    /// The commands for the query, while it starts with `>`. True if the
    /// search is the palette.
    pub(super) fn run_palette(&mut self) -> bool {
        let palette = self.search.as_ref().is_some_and(|s| s.query.text().starts_with('>'));
        if !palette {
            if let Some(s) = &mut self.search {
                s.commands = None;
                s.shown.clear();
                s.runs.clear();
                s.ids.clear();
            }
            return false;
        }
        // Gathered as the palette opens (installed apps are looked up then),
        // not on every key.
        let gathered = self.search.as_ref().is_some_and(|s| s.commands.is_none()).then(|| self.commands());
        let Some(s) = &mut self.search else { return false };
        if let Some(c) = gathered {
            s.commands = Some(c);
        }
        let all = s.commands.as_deref().unwrap_or_default();
        let order = ranked(all, &s.query.text()[1..], self.recent_commands.ids());
        s.shown = order.iter().map(|&i| all[i].item.clone()).collect();
        s.runs = order.iter().map(|&i| all[i].run.clone()).collect();
        s.ids = order.iter().map(|&i| all[i].id).collect();
        s.hits.clear();
        s.selected = s.selected.min(s.shown.len().saturating_sub(1));
        s.fresh = true;
        true
    }

    /// Runs the `i`th command shown.
    pub(super) fn run_command(&mut self, i: usize) -> bool {
        let Some(run) = self.search.as_ref().and_then(|s| s.runs.get(i)).cloned() else { return false };
        if let Some(id) = self.search.as_ref().and_then(|s| s.ids.get(i).copied().flatten()) {
            self.recent_commands.ran(id);
        }
        if let Run::Search = run {
            if let Some(s) = &mut self.search {
                s.query = Editor::field("");
                s.fresh = false;
            }
            return true;
        }
        self.search = None;
        self.run(run)
    }

    fn run(&mut self, run: Run) -> bool {
        match run {
            Run::NewNote => self.new_note(),
            Run::Today => self.open_today(),
            Run::Welcome => self.show_welcome(),
            Run::Due => self.open_agenda(),
            Run::Search => self.open_search(),
            Run::Find(replace) => self.open_find(replace),
            Run::Step(forward) => self.step(forward),
            Run::Highlights => self.open_highlights(),
            Run::History => self.toggle_scrub(),
            Run::Trash => self.open_trash(),
            Run::Act(action) => {
                let (changed, effects) = self.run_menu(action);
                self.menus.effects.extend(effects);
                changed
            }
            Run::Tool(tool) => self.tool(tool),
            Run::Social(p) => self.show_social(Some(p)),
            Run::Choose(setting, value) => self.choose(setting, value),
            Run::Panel(panel) => self.toggle_panel(panel),
            Run::View => self.toggle_view(),
            Run::Minimal(on) => self.set_minimal(on),
            Run::TextSize(up) => match scene::step_text_size(self.session.text_size, up) {
                Some(next) => self.choose(Setting::TextSize, next),
                None => false,
            },
            Run::LineWidth => {
                let next = match self.session.line_width {
                    1 => 0,
                    0 => 2,
                    _ => 1,
                };
                self.choose(Setting::LineWidth, next)
            }
            Run::Settings => self.toggle_options(),
            Run::Transfer(what) => self.transfer(what),
            Run::Vault => {
                self.settings.requests.push(settings::Request::Pick(Pick::Vault));
                true
            }
            Run::ShowVault => {
                self.settings.requests.push(settings::Request::ShowVault(self.vault.clone()));
                true
            }
            Run::Quit => {
                self.settings.requests.push(settings::Request::Quit);
                true
            }
            Run::Edit(id) => match id {
                Id::Fold | Id::Unfold => {
                    let at = self.editor.selection().head;
                    if id == Id::Fold { self.editor.fold(at) } else { self.editor.unfold(at) }
                }
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
            },
        }
    }

    /// What the view must do outside the app since it last asked (a
    /// command's clipboard, Finder, browser).
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.menus.effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(commands: &[Command], q: &str) -> Vec<String> {
        ranked(commands, q, &[]).into_iter().map(|i| commands[i].item.label.clone()).collect()
    }

    #[test]
    fn finds_commands_by_name_then_by_other_words() {
        let commands = vec![
            command("", "New note", "", "create page", Run::Search),
            command("", "Find in note", "", "search text", Run::Search),
            command("", "Export as PDF\u{2026}", "", "save print", Run::Search),
            command("", "Dark theme", "", "appearance mode", Run::Search),
            command("", "Move note to trash", "", "delete remove", Run::Search),
        ];
        assert_eq!(names(&commands, "").len(), 5, "all, in order");
        assert_eq!(names(&commands, "note"), ["New note", "Find in note", "Move note to trash"]);
        assert_eq!(names(&commands, "pdf"), ["Export as PDF\u{2026}"]);
        assert_eq!(names(&commands, "delete"), ["Move note to trash"]);
        assert_eq!(names(&commands, "zzz"), Vec::<String>::new());
    }

    fn model(test: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-app-palette-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Note.md"), "# Note\n\nBody\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        (m, dir)
    }

    #[test]
    fn the_palette_names_keys_and_places_as_the_platform_does() {
        let (mut m, dir) = model("platform");
        let find = |cs: &[Command], label: &str| cs.iter().find(|c| c.item.label == label).map(|c| c.item.keys).unwrap_or_else(|| panic!("no {label}"));
        let mac = m.commands_with(Vec::new());
        assert_eq!(find(&mac, "New note"), "\u{2318}N");
        assert!(mac.iter().any(|c| c.item.label == "Show in Finder"));
        assert!(mac.iter().any(|c| c.item.label.contains("quick note")), "the Mac has its quick note keys");
        m.set_platform(scene::Platform::Linux);
        m.session.theme = 1;
        let linux = m.commands_with(Vec::new());
        assert!(linux.iter().all(|c| !c.item.keys.contains('\u{2318}') && !c.item.keys.contains('\u{21E7}')), "no Command glyphs: {:?}", linux.iter().map(|c| c.item.keys).collect::<Vec<_>>());
        assert_eq!(find(&linux, "New note"), "Ctrl+N");
        assert_eq!(find(&linux, "Replace in note"), "Ctrl+H");
        assert_eq!(find(&linux, "Go back"), "Alt+\u{2190}");
        assert_eq!(find(&linux, "Note history"), "Ctrl+Shift+H");
        assert_eq!(find(&linux, "Bulleted list"), "Ctrl+Shift+8");
        assert_eq!(find(&linux, "Settings"), "Ctrl+,");
        assert_eq!(find(&linux, "Today's note"), "", "no key where Linux binds none");
        assert!(linux.iter().any(|c| c.item.label == "Show in Folder") && linux.iter().any(|c| c.item.label == "Show vault in Folder"));
        assert!(!linux.iter().any(|c| c.item.label.contains("Finder") || c.item.label.contains("macOS")));
        assert!(!linux.iter().any(|c| c.item.label.contains("quick note")), "no quick note keys on Linux yet");
        assert!(linux.iter().any(|c| c.item.label == "Theme like the desktop"));
        let row = m.session.sidebar.iter().position(|r| r.name == "Note").unwrap();
        assert!(m.row_menu(row).iter().any(|i| i.label == "Show in Folder"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cmd_p_lists_commands_and_return_runs_the_one_chosen() {
        let (mut m, dir) = model("run");
        assert!(m.open_palette());
        m.frame(2000, 1440, 2.0);
        let shown = m.search.as_ref().unwrap().shown.len();
        assert!(shown > 30, "everything, with no query: {shown}");
        assert!(m.hits.iter().any(|(_, t)| matches!(t, Target::Result(_))), "listed in the side menu");
        m.insert("dark");
        m.frame(2000, 1440, 2.0);
        assert_eq!(m.search.as_ref().unwrap().shown[0].label, "Dark theme");
        assert!(m.newline(false));
        assert!(m.search.is_none(), "the palette closes");
        assert_eq!(m.session.theme, 2, "and the theme is dark");
        // Back to search: a query without > finds notes again.
        m.open_palette();
        m.backspace(false);
        // The notes are read on a thread of their own: wait for them.
        for _ in 0..500 {
            if m.index.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        m.insert("Note");
        m.frame(2000, 1440, 2.0);
        let s = m.search.as_ref().unwrap();
        assert!(s.commands.is_none() && s.hits.iter().any(|h| h.name == "Note"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn larger_and_smaller_text_step_through_every_size() {
        let (mut m, dir) = model("sizes");
        let run = |m: &mut Model, q: &str| {
            m.open_palette();
            m.insert(q);
            m.frame(2000, 1440, 2.0);
            m.newline(false);
            scene::text_size(m.session.text_size)
        };
        let up: Vec<u8> = (0..4).map(|_| run(&mut m, "larger text")).collect();
        assert_eq!(up, [18, 20, 23, 23], "and no larger than the largest");
        let down: Vec<u8> = (0..7).map(|_| run(&mut m, "smaller text")).collect();
        assert_eq!(down, [20, 18, 16, 15, 14, 13, 13], "and no smaller than the smallest");
        assert_eq!(m.fonts.body.size, 13.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_command_that_goes_outside_hands_the_view_its_effect() {
        let (mut m, dir) = model("effect");
        m.open_palette();
        m.insert("copy link");
        m.frame(2000, 1440, 2.0);
        assert!(m.newline(false));
        assert!(matches!(m.take_effects().as_slice(), [Effect::Clipboard(t, _)] if t == "[[Note]]"));
        m.open_palette();
        m.insert("quit");
        m.frame(2000, 1440, 2.0);
        m.newline(false);
        assert_eq!(m.take_requests(), [settings::Request::Quit]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_palette_has_the_notes_commands_only_with_a_note_and_the_apps_found() {
        let (mut m, dir) = model("only");
        let labels = |m: &Model| m.commands_with(vec![Agent::Claude]).into_iter().map(|c| c.item.label).collect::<Vec<_>>();
        assert!(labels(&m).contains(&"Rename note".to_string()) && labels(&m).contains(&"New Chat in Claude".to_string()));
        m.new_note();
        assert!(!labels(&m).contains(&"Rename note".to_string()), "a blank page has no file to rename");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod icon_font_tests {
    /// Every private-use icon the screens name is in Kolada's own icon font
    /// (`assets/icons/map.tsv`), which the Mac and iOS draw with: one that is
    /// not shows as a blank there (seen on the Mac: Note history and
    /// Spelling in the palette had no icon). Windows draws Segoe's own.
    #[test]
    fn every_icon_the_screens_name_is_in_koladas_icon_font() {
        let map = include_str!("../../../../assets/icons/map.tsv");
        let have: Vec<u32> = map.lines().filter(|l| !l.starts_with('#')).filter_map(|l| u32::from_str_radix(l.split('\t').next()?, 16).ok()).collect();
        let sources = [
            include_str!("palette.rs"),
            include_str!("../../../kolada-ui/src/chrome.rs"),
            include_str!("../../../kolada-ui/src/scene.rs"),
            include_str!("../../../kolada-ui/src/scrub.rs"),
            include_str!("../../../kolada-ui/src/social.rs"),
        ];
        let mut missing = Vec::new();
        for src in sources {
            for (i, _) in src.match_indices("\\u{") {
                let code = &src[i + 3..];
                let Some(end) = code.find('}') else { continue };
                let Ok(c) = u32::from_str_radix(&code[..end], 16) else { continue };
                // The Private Use Area, where Segoe's (and so Kolada's) icons are.
                if (0xE000..=0xF8FF).contains(&c) && !have.contains(&c) && !missing.contains(&c) {
                    missing.push(c);
                }
            }
        }
        assert!(missing.is_empty(), "icons not in the font: {:X?} (map them in assets/icons/map.tsv, then cargo run -p iconfont)", missing);
    }
}
