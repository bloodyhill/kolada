//! The command palette: `>` in the search field (or Ctrl+P) lists what
//! Kolada can do, by name, with the keys that do it too, so everything is a
//! few letters away and the keys are learned on the way.

use kolada_core::search;

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
    Social(kolada_core::social::Platform),
    Quit,
}

pub(super) struct Command {
    pub item: MenuItem,
    /// Other words it is found by.
    words: &'static str,
    pub run: Run,
}

fn command(icon: &'static str, label: &str, keys: &'static str, words: &'static str, run: Run) -> Command {
    Command { item: MenuItem { icon, label: label.to_string(), keys, ..MenuItem::default() }, words, run }
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
    pub const TRASH: &str = "\u{E74D}";
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
    pub const TOOLBAR: &str = "\u{E70E}";
    pub const SPELL: &str = "\u{F87B}";
    pub const WRITING: &str = "\u{E70F}";
    pub const RELATED: &str = "\u{E71B}";
    pub const QUICK: &str = "\u{E765}";
    pub const SOCIAL: &str = "\u{E8BD}";
    pub const VAULT: &str = "\u{E8B7}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const QUIT: &str = "\u{E7E8}";
}

/// The commands for a query, best first: a name that starts with it or has
/// a word that does, then the other words a command answers to, then names
/// that merely contain it or (from three letters) have its letters in
/// order; in the list's own order when they match as well.
pub(super) fn ranked(commands: &[Command], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<(u8, usize)> = commands
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            if q.is_empty() {
                return Some((0, i));
            }
            let by_name = search::rank(&c.item.label.to_lowercase(), &q).filter(|&r| r < 5 || q.chars().count() >= 3);
            let by_words = c.words.split(' ').filter_map(|w| search::rank(w, &q).filter(|&r| r <= 2)).min();
            let rank = match (by_name, by_words) {
                (Some(r), _) if r <= 3 => r,
                (_, Some(r)) => 4 + r,
                (Some(r), None) => 3 + r,
                (None, None) => return None,
            };
            Some((rank, i))
        })
        .collect();
    out.sort();
    out.into_iter().map(|(_, i)| i).collect()
}

impl App {
    /// Everything the palette offers now: the note's commands only with a
    /// note open, the agents that are installed, settings by what they
    /// would change to.
    pub(super) fn commands(&self) -> Vec<Command> {
        let mut out = vec![
            command(icon::ADD, "New note", "Ctrl+N", "create page fresh blank", Run::Click(Target::NewNote)),
            command(icon::TODAY, "Today's note", "", "daily journal diary date", Run::Today),
            command(icon::TODAY, "What's due", "Ctrl+T", "today tasks due reminders agenda late", Run::Click(Target::DueDot)),
            command(icon::SEARCH, "Search notes", "Ctrl+O", "find open go switch", Run::Search),
            command(icon::SEARCH, "Find in note", "Ctrl+F", "search text", Run::Find(false)),
            command(icon::REPLACE, "Replace in note", "Ctrl+H", "find change", Run::Find(true)),
            command(icon::BACK, "Go back", "Alt+\u{2190}", "previous history", Run::Step(false)),
            command(icon::FORWARD, "Go forward", "Alt+\u{2192}", "next history", Run::Step(true)),
            command(icon::HIGHLIGHT, "Highlights", "", "marked gathered", Run::Click(Target::Highlights)),
        ];
        if !self.session.note.is_empty() {
            let path = self.note_path();
            if !self.compact {
                out.push(command(icon::WINDOW, "Open in a new window", "", "separate pop out", Run::Act(Action::OpenWindow(path.clone()))));
            }
            out.extend([
                command(icon::RENAME, "Rename note", "", "title name", Run::Act(Action::Rename(path.clone()))),
                command(icon::COPY, "Duplicate note", "", "copy clone", Run::Act(Action::Duplicate(path.clone()))),
                command(icon::LINK, "Copy link to note", "", "wikilink", Run::Act(Action::CopyLink(stem(&path)))),
                command(icon::FOLDER, "Show in Explorer", "", "folder file reveal", Run::Act(Action::Reveal(path))),
            ]);
            let mut delete = command(icon::DELETE, "Move note to trash", "", "delete remove", Run::Act(Action::Delete(self.session.note.clone())));
            delete.item.danger = true;
            out.push(delete);
        }
        out.push(command(icon::TRASH, "Trash", "", "deleted restore bin recycle", Run::Click(Target::Trash)));
        if !self.read_only {
            out.extend([
                command(icon::HEADING, "Heading", "Ctrl+1", "title h1 h2 h3", Run::Click(Target::Tool(Tool::Heading))),
                command(icon::BOLD, "Bold", "Ctrl+B", "strong format", Run::Click(Target::Tool(Tool::Bold))),
                command(icon::ITALIC, "Italic", "Ctrl+I", "emphasis format", Run::Click(Target::Tool(Tool::Italic))),
                command(icon::STRIKE, "Strikethrough", "Ctrl+Shift+X", "format cross", Run::Click(Target::Tool(Tool::Strike))),
                command(icon::HIGHLIGHT, "Highlight", "", "mark format", Run::Act(Action::Highlight)),
                command(icon::LIST, "Bulleted list", "Ctrl+Shift+8", "bullets", Run::Click(Target::Tool(Tool::List))),
                command(icon::CHECKLIST, "Checklist", "Ctrl+L", "task todo", Run::Click(Target::Tool(Tool::Checklist))),
                command(icon::QUOTE, "Quote", "", "blockquote", Run::Click(Target::Tool(Tool::Quote))),
                command(icon::CODE, "Code", "Ctrl+E", "monospace", Run::Click(Target::Tool(Tool::Code))),
                command(icon::LINK, "Link", "Ctrl+K", "url", Run::Click(Target::Tool(Tool::Link))),
            ]);
        }
        for p in kolada_core::social::PLATFORMS {
            out.push(command(icon::SOCIAL, &format!("Preview for {}", p.name()), "", "social post caption length see more", Run::Social(p)));
        }
        for agent in Agent::detect() {
            // As the title bar's Send menu has them: where each opens.
            out.push(command(agent.icon(), agent.label(), agent.place(), "send ai chat ask agent share", Run::Act(Action::SendTo(agent))));
        }
        out.extend([
            command(icon::COPY, "Copy for any AI", "", "markdown prompt chat", Run::Act(Action::CopyMarkdown)),
            command(icon::COPY, "Copy formatted", "", "rich html email", Run::Act(Action::CopyRich)),
            command(icon::EXPORT, "Export as Markdown\u{2026}", "", "save share md", Run::Click(Target::Transfer(Transfer::Markdown))),
            command(icon::EXPORT, "Export as web page\u{2026}", "", "save share html", Run::Click(Target::Transfer(Transfer::Page))),
            command(icon::EXPORT, "Export as PDF\u{2026}", "", "save share print", Run::Click(Target::Transfer(Transfer::Pdf))),
            command(icon::IMPORT, "Import notes\u{2026}", "", "files open markdown docx word pdf html web page document convert evernote enex notion zip export", Run::Click(Target::Transfer(Transfer::Files))),
            command(icon::IMPORT, "Import from Notes (.nbk)\u{2026}", "", "nbk export backup foss notes app", Run::Click(Target::Transfer(Transfer::NotesApp))),
            command(icon::IMPORT, "Import a folder\u{2026}", "", "vault obsidian", Run::Click(Target::Transfer(Transfer::Folder))),
            command(icon::PASTE, "New note from clipboard", "", "paste import", Run::Click(Target::Transfer(Transfer::Clipboard))),
        ]);
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
        let (sidebar, toolbar) = (self.session.sidebar_collapsed, self.session.toolbar_collapsed);
        let on_off = |on: bool, what: &str| format!("Turn {what} {}", if on { "off" } else { "on" });
        out.extend([
            command(icon::LARGER, "Larger text", "", "font size zoom bigger", Run::Click(Target::TextSize(true))),
            command(icon::SMALLER, "Smaller text", "", "font size zoom", Run::Click(Target::TextSize(false))),
            command(icon::WIDTH, "Change line width", "", "column narrow wide", Run::Click(Target::LineWidth)),
            command(icon::PANEL, if sidebar { "Keep the side menu open" } else { "Tuck the side menu away" }, "", "sidebar panel focus", Run::Click(Target::ToggleSidebar)),
            command(icon::TOOLBAR, if toolbar { "Keep the toolbar open" } else { "Tuck the toolbar away" }, "", "format bar focus", Run::Click(Target::ToggleToolbar)),
            command(icon::SPELL, &on_off(self.session.spell == 0, "spell check"), "", "spelling dictionary", Run::Choose(Setting::Spell, if self.session.spell == 0 { 1 } else { 0 })),
            command(icon::WRITING, &on_off(self.session.assist == 0, "writing help"), "", "grammar fix lists gist summary ai model assistant", Run::Choose(Setting::Assist, if self.session.assist == 0 { 1 } else { 0 })),
            command(icon::QUICK, &on_off(self.session.hotkey == 0, "the Win+Alt+N quick note"), "", "hotkey shortcut global capture", Run::Choose(Setting::Hotkey, if self.session.hotkey == 0 { 1 } else { 0 })),
            command(icon::RELATED, &on_off(self.session.related == 0, "related notes"), "", "margin memory magnet chips", Run::Choose(Setting::Related, if self.session.related == 0 { 1 } else { 0 })),
            command(icon::VAULT, "Open a vault\u{2026}", "", "folder obsidian switch", Run::Click(Target::VaultOpen)),
            command(icon::FOLDER, "Show vault in Explorer", "", "folder", Run::Click(Target::VaultShow)),
            command(icon::SETTINGS, "Settings", "", "options preferences", Run::Click(Target::Options)),
            command(icon::QUIT, "Quit Kolada", "", "exit close", Run::Quit),
        ]);
        out
    }

    /// Ctrl+P: the search field, ready for a command.
    pub(super) fn open_palette(&mut self) {
        self.open_search();
        if let Some(s) = &mut self.search {
            s.query = Editor::new(">");
            s.query.set_selection(Selection::caret(1));
        }
        self.run_search();
    }

    /// Runs the `i`th command shown.
    pub(super) fn run_command(&mut self, i: usize) {
        let Some(run) = self.search.as_ref().and_then(|s| s.runs.get(i)).cloned() else { return };
        if let Run::Search = run {
            if let Some(s) = &mut self.search {
                s.query = Editor::new("");
            }
            return self.run_search();
        }
        self.close_search();
        match run {
            Run::Click(target) => self.activate(target),
            Run::Act(action) => self.run_action(action, false),
            Run::Choose(setting, value) => self.choose(setting, value),
            Run::Step(forward) => self.step(forward),
            Run::Find(replace) => self.open_find(replace),
            Run::Search => {}
            Run::Today => self.open_today(),
            Run::Social(p) => self.show_social(Some(p)),
            Run::Quit => self.quit(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(commands: &[Command], q: &str) -> Vec<String> {
        ranked(commands, q).into_iter().map(|i| commands[i].item.label.clone()).collect()
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
        assert_eq!(names(&commands, "dark"), ["Dark theme"]);
        // Not in any name: the words it answers to.
        assert_eq!(names(&commands, "delete"), ["Move note to trash"]);
        assert_eq!(names(&commands, "print"), ["Export as PDF\u{2026}"]);
        assert_eq!(names(&commands, "zzz"), Vec::<String>::new());
    }

    #[test]
    fn short_queries_skip_letters_in_order() {
        let commands = vec![
            command("", "Replace in note", "", "find change", Run::Search),
            command("", "Italic", "", "emphasis", Run::Search),
            command("", "New chat in Claude", "", "send ai chat", Run::Search),
            command("", "Copy for any AI", "", "markdown prompt", Run::Search),
        ];
        assert_eq!(names(&commands, "ai"), ["Copy for any AI", "New chat in Claude"]);
        // From three letters, letters in order count, after the rest.
        assert_eq!(names(&commands, "rpl"), ["Replace in note"]);
    }
}
