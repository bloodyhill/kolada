//! What Kolada can do, by name: each command's name, the other words it is
//! found by, and the keys that do it on each OS. Every shell's palette is
//! built from this one list (adding what only it has: the AI apps found,
//! themes, settings by what they would change to), so a command is named
//! and keyed alike everywhere, and a new one reaches every palette at once.

use kolada_core::search;

use crate::scene::Platform;

/// A command, whatever shell runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Id {
    NewNote,
    Today,
    Due,
    SearchNotes,
    Find,
    Replace,
    Back,
    Forward,
    Highlights,
    History,
    OpenWindow,
    Rename,
    DuplicateNote,
    CopyLink,
    TrashNote,
    Trash,
    Heading,
    Bold,
    Italic,
    Strike,
    Highlight,
    List,
    Checklist,
    Quote,
    Code,
    Link,
    MoveLineUp,
    MoveLineDown,
    DuplicateLine,
    SortLines,
    NextMatch,
    Fold,
    Unfold,
    CopyForAi,
    CopyRich,
    ExportMarkdown,
    ExportPage,
    ExportPdf,
    Import,
    ImportFolder,
    /// The welcome screen a first run opens on.
    Welcome,
    FromClipboard,
    LargerText,
    SmallerText,
    LineWidth,
    OpenVault,
    Settings,
    Quit,
}

/// The keys that do a command on each OS, as each names them; empty where
/// it has none.
#[derive(Clone, Copy, Debug)]
pub struct Keys {
    pub windows: &'static str,
    pub mac: &'static str,
    pub linux: &'static str,
    pub ios: &'static str,
}

pub struct Command {
    pub id: Id,
    pub name: &'static str,
    /// Other words it is found by.
    pub words: &'static str,
    pub keys: Keys,
}

impl Command {
    pub fn keys(&self, platform: Platform) -> &'static str {
        match platform {
            Platform::Windows => self.keys.windows,
            Platform::Mac => self.keys.mac,
            Platform::Linux => self.keys.linux,
            Platform::Ios => self.keys.ios,
            // An Android keyboard has Ctrl, and Android takes Linux's keys.
            Platform::Android => self.keys.linux,
        }
    }
}

/// Ctrl keys on Windows and Linux, Command keys on the Mac and iPad.
const fn pc_mac(pc: &'static str, mac: &'static str) -> Keys {
    Keys { windows: pc, mac, linux: pc, ios: mac }
}

const NONE: Keys = pc_mac("", "");

const fn c(id: Id, name: &'static str, words: &'static str, keys: Keys) -> Command {
    Command { id, name, words, keys }
}

/// Every command, in the order the palette lists them (after what was
/// used last).
pub const ALL: &[Command] = &[
    c(Id::NewNote, "New note", "create page fresh blank", pc_mac("Ctrl+N", "\u{2318}N")),
    c(Id::Today, "Today's note", "daily journal diary date", pc_mac("", "\u{21E7}\u{2318}D")),
    c(Id::Due, "What's due", "today tasks due reminders agenda late", pc_mac("Ctrl+T", "\u{2318}T")),
    c(Id::SearchNotes, "Search notes", "find open go switch", pc_mac("Ctrl+O", "\u{2318}O")),
    c(Id::Find, "Find in note", "search text", pc_mac("Ctrl+F", "\u{2318}F")),
    c(Id::Replace, "Replace in note", "find change", pc_mac("Ctrl+H", "")),
    c(Id::Back, "Go back", "previous history", pc_mac("Alt+\u{2190}", "\u{2318}[")),
    c(Id::Forward, "Go forward", "next history", pc_mac("Alt+\u{2192}", "\u{2318}]")),
    c(Id::Highlights, "Highlights", "marked gathered", NONE),
    c(Id::History, "Note history", "past versions time scrub restore", pc_mac("Ctrl+Shift+H", "\u{21E7}\u{2318}H")),
    c(Id::MoveLineUp, "Move line up", "shift reorder swap", Keys { windows: "Alt+\u{2191}", mac: "\u{2303}\u{2318}\u{2191}", linux: "Alt+\u{2191}", ios: "\u{2303}\u{2318}\u{2191}" }),
    c(Id::MoveLineDown, "Move line down", "shift reorder swap", Keys { windows: "Alt+\u{2193}", mac: "\u{2303}\u{2318}\u{2193}", linux: "Alt+\u{2193}", ios: "\u{2303}\u{2318}\u{2193}" }),
    c(Id::DuplicateLine, "Duplicate line", "copy clone repeat", Keys { windows: "Shift+Alt+\u{2193}", mac: "\u{21E7}\u{2325}\u{2193}", linux: "Shift+Alt+\u{2193}", ios: "\u{21E7}\u{2325}\u{2193}" }),
    c(Id::SortLines, "Sort lines", "order alphabetical abc selection list", NONE),
    c(Id::NextMatch, "Select the same word again", "next match multiple cursors carets occurrence add", pc_mac("Ctrl+D", "\u{2318}D")),
    c(Id::Fold, "Collapse section", "fold heading hide outline", pc_mac("Ctrl+Shift+[", "\u{2325}\u{2318}[")),
    c(Id::Unfold, "Expand section", "unfold heading show outline", pc_mac("Ctrl+Shift+]", "\u{2325}\u{2318}]")),
    c(Id::OpenWindow, "Open in a new window", "separate pop out", NONE),
    c(Id::Rename, "Rename note", "title name", NONE),
    c(Id::DuplicateNote, "Duplicate note", "copy clone", NONE),
    c(Id::CopyLink, "Copy link to note", "wikilink", NONE),
    c(Id::TrashNote, "Move note to trash", "delete remove", pc_mac("", "\u{2318}\u{232B}")),
    c(Id::Trash, "Trash", "deleted restore bin recycle", NONE),
    c(Id::Heading, "Heading", "title h1 h2 h3", Keys { windows: "Ctrl+1", mac: "\u{2325}\u{2318}1", linux: "Ctrl+1", ios: "\u{2318}1" }),
    c(Id::Bold, "Bold", "strong format", pc_mac("Ctrl+B", "\u{2318}B")),
    c(Id::Italic, "Italic", "emphasis format", pc_mac("Ctrl+I", "\u{2318}I")),
    c(Id::Strike, "Strikethrough", "format cross", pc_mac("Ctrl+Shift+X", "\u{21E7}\u{2318}X")),
    c(Id::Highlight, "Highlight", "mark format", pc_mac("", "\u{21E7}\u{2318}U")),
    c(Id::List, "Bulleted list", "bullets", pc_mac("Ctrl+Shift+8", "\u{2318}8")),
    c(Id::Checklist, "Checklist", "task todo", pc_mac("Ctrl+L", "\u{2318}L")),
    c(Id::Quote, "Quote", "blockquote", NONE),
    c(Id::Code, "Code", "monospace", pc_mac("Ctrl+E", "\u{2318}E")),
    c(Id::Link, "Link", "url", pc_mac("Ctrl+K", "\u{2318}K")),
    c(Id::CopyForAi, "Copy for any AI", "markdown prompt chat", NONE),
    c(Id::CopyRich, "Copy formatted", "rich html email", pc_mac("", "\u{21E7}\u{2318}C")),
    c(Id::ExportMarkdown, "Export as Markdown\u{2026}", "save share md", NONE),
    c(Id::ExportPage, "Export as web page\u{2026}", "save share html", NONE),
    c(Id::ExportPdf, "Export as PDF\u{2026}", "save share print", NONE),
    c(Id::Import, "Import notes\u{2026}", "files open markdown docx word pdf html web page document convert evernote enex notion zip nbk export google keep takeout bear bear2bk", NONE),
    c(Id::ImportFolder, "Import a folder\u{2026}", "vault obsidian", NONE),
    c(Id::Welcome, "Welcome", "start first run import bring notes obsidian evernote notion keep bear setup devices pair writing model", NONE),
    c(Id::FromClipboard, "New note from clipboard", "paste import", NONE),
    c(Id::LargerText, "Larger text", "font size zoom bigger", NONE),
    c(Id::SmallerText, "Smaller text", "font size zoom", NONE),
    c(Id::LineWidth, "Change line width", "column narrow wide", NONE),
    c(Id::OpenVault, "Open a vault\u{2026}", "folder obsidian switch", NONE),
    c(Id::Settings, "Settings", "options preferences", Keys { windows: "", mac: "\u{2318},", linux: "Ctrl+,", ios: "\u{2318}," }),
    c(Id::Quit, "Quit Kolada", "exit close", Keys { windows: "", mac: "\u{2318}Q", linux: "Ctrl+Q", ios: "" }),
];

/// The command `id`.
pub fn command(id: Id) -> &'static Command {
    ALL.iter().find(|c| c.id == id).expect("every id is listed")
}

/// What acts on the text: offered only while a note can be written.
pub fn edits(id: Id) -> bool {
    use Id::*;
    matches!(id, Heading | Bold | Italic | Strike | Highlight | List | Checklist | Quote | Code | Link | MoveLineUp | MoveLineDown | DuplicateLine | SortLines | NextMatch)
}

/// What a note must be open for.
pub fn needs_note(id: Id) -> bool {
    use Id::*;
    edits(id) || matches!(id, History | OpenWindow | Rename | DuplicateNote | CopyLink | TrashNote | Fold | Unfold)
}

/// The palette's entries for a query, best first, by index into `entries`
/// (each a name and the other words it is found by): a name that starts
/// with the query or has a word that does, then the other words, then
/// names that merely contain it or (from three letters) have its letters
/// in order; in the list's own order when they match as well. An empty
/// query keeps the list's order.
pub fn ranked(entries: &[(&str, &str)], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<(u8, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, &(name, words))| {
            if q.is_empty() {
                return Some((0, i));
            }
            let by_name = search::rank(&name.to_lowercase(), &q).filter(|&r| r < 5 || q.chars().count() >= 3);
            let by_words = words.split(' ').filter_map(|w| search::rank(w, &q).filter(|&r| r <= 2)).min();
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

/// The order of a palette with nothing typed, by index into `ids` (each
/// entry's command, None for a shell's own): what was run last (`recent`,
/// latest first), then the rest as listed.
pub fn at_rest(ids: &[Option<Id>], recent: &[Id]) -> Vec<usize> {
    let place = |i: usize| match ids[i] {
        Some(id) if recent.contains(&id) => (0, recent.iter().position(|&r| r == id).unwrap_or(0)),
        _ => (1, i),
    };
    let mut order: Vec<usize> = (0..ids.len()).collect();
    order.sort_by_key(|&i| place(i));
    order
}

/// The commands run lately, latest first: all a palette with nothing typed
/// offers.
#[derive(Clone, Debug, Default)]
pub struct Recent(Vec<Id>);

impl Recent {
    pub const KEPT: usize = 5;

    pub fn ran(&mut self, id: Id) {
        self.0.retain(|&r| r != id);
        self.0.insert(0, id);
        self.0.truncate(Self::KEPT);
    }

    pub fn ids(&self) -> &[Id] {
        &self.0
    }
}

/// Where the letters of `query` show in `name`, for drawing them in full
/// ink: the byte range of the first match at a word's start, else of the
/// first match anywhere. None if it is not in the name as typed (matched by
/// its other words, or letters skipped).
pub fn matched(name: &str, query: &str) -> Option<(usize, usize)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    let lower = name.to_lowercase();
    // Lowercasing keeps byte offsets for the names listed (ASCII and
    // punctuation); a name it would change gets no marks.
    if lower.len() != name.len() {
        return None;
    }
    let at_word = lower.match_indices(&q).map(|(i, _)| i).find(|&i| i == 0 || !lower[..i].ends_with(char::is_alphanumeric));
    let at = at_word.or_else(|| lower.find(&q))?;
    Some((at, at + q.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(entries: &[(&str, &str)], q: &str) -> Vec<String> {
        ranked(entries, q).into_iter().map(|i| entries[i].0.to_string()).collect()
    }

    #[test]
    fn finds_commands_by_name_then_by_other_words() {
        let entries =
            [("New note", "create page"), ("Find in note", "search text"), ("Export as PDF\u{2026}", "save print"), ("Dark theme", "appearance mode"), ("Move note to trash", "delete remove")];
        assert_eq!(names(&entries, "").len(), 5, "all, in order");
        assert_eq!(names(&entries, "note"), ["New note", "Find in note", "Move note to trash"]);
        assert_eq!(names(&entries, "pdf"), ["Export as PDF\u{2026}"]);
        // Not in any name: the words it answers to.
        assert_eq!(names(&entries, "delete"), ["Move note to trash"]);
        assert_eq!(names(&entries, "zzz"), Vec::<String>::new());
    }

    #[test]
    fn short_queries_skip_letters_in_order() {
        let entries = [("Replace in note", "find change"), ("Italic", "emphasis"), ("New chat in Claude", "send ai chat"), ("Copy for any AI", "markdown prompt")];
        assert_eq!(names(&entries, "ai"), ["Copy for any AI", "New chat in Claude"]);
        // From three letters, letters in order count, after the rest.
        assert_eq!(names(&entries, "rpl"), ["Replace in note"]);
    }

    #[test]
    fn every_command_is_listed_once_with_a_name_and_keys_kept_apart_by_os() {
        for (i, c) in ALL.iter().enumerate() {
            assert!(!c.name.is_empty());
            assert_eq!(command(c.id).name, c.name);
            assert!(ALL[..i].iter().all(|o| o.id != c.id && o.name != c.name), "{} is listed once", c.name);
            assert!(!c.keys.windows.contains('\u{2318}') && !c.keys.linux.contains('\u{2318}'), "{}: no Command key on a PC", c.name);
            assert!(!c.keys.mac.contains("Ctrl+"), "{}: the Mac names its keys by their signs", c.name);
        }
        // No two commands share keys on one OS.
        for p in [Platform::Windows, Platform::Mac, Platform::Linux, Platform::Ios] {
            let keys: Vec<&str> = ALL.iter().map(|c| c.keys(p)).filter(|k| !k.is_empty()).collect();
            for (i, k) in keys.iter().enumerate() {
                assert!(!keys[..i].contains(k), "{k} is used twice on {p:?}");
            }
        }
    }

    #[test]
    fn the_editors_power_moves_are_commands_with_each_oss_keys() {
        assert_eq!(command(Id::MoveLineUp).keys(Platform::Windows), "Alt+\u{2191}");
        assert_eq!(command(Id::MoveLineUp).keys(Platform::Mac), "\u{2303}\u{2318}\u{2191}", "the Mac keeps \u{2325}\u{2191} for paragraphs");
        assert_eq!(command(Id::NextMatch).keys(Platform::Linux), "Ctrl+D");
        assert_eq!(command(Id::Fold).keys(Platform::Mac), "\u{2325}\u{2318}[", "\u{2318}[ is Back on the Mac");
        assert_eq!(command(Id::SortLines).keys(Platform::Windows), "", "sorting has no keys");
        for id in [Id::MoveLineUp, Id::MoveLineDown, Id::DuplicateLine, Id::SortLines, Id::NextMatch] {
            assert!(edits(id) && needs_note(id));
        }
    }

    #[test]
    fn with_nothing_typed_what_ran_last_comes_first() {
        let ids = [Some(Id::NewNote), Some(Id::Bold), None, Some(Id::SortLines), Some(Id::Settings)];
        let mut recent = Recent::default();
        recent.ran(Id::Settings);
        recent.ran(Id::Bold);
        recent.ran(Id::Settings);
        assert_eq!(recent.ids(), [Id::Settings, Id::Bold], "latest first, once each");
        assert_eq!(at_rest(&ids, recent.ids()), [4, 1, 0, 2, 3]);
        for id in [Id::NewNote, Id::Find, Id::Bold, Id::Italic, Id::Code, Id::Quote] {
            recent.ran(id);
        }
        assert_eq!(recent.ids().len(), Recent::KEPT, "at most five");
    }

    #[test]
    fn the_letters_typed_show_where_they_are_in_the_name() {
        assert_eq!(matched("Move line up", "line"), Some((5, 9)));
        assert_eq!(matched("Move line up", "mov"), Some((0, 3)));
        assert_eq!(matched("Bulleted list", "list"), Some((9, 13)), "at a word's start");
        assert_eq!(matched("Settings", "options"), None, "found by its other words");
        assert_eq!(matched("Settings", ""), None);
    }
}
