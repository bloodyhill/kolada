//! What a phone's native shell asks of the model: it draws its own
//! navigation, list of notes and bars,
//! and the model draws the note alone inside them (`set_bare`). So: every
//! note in the vault, a note opened, pinned (a favorite) or trashed, and
//! where the text is, for the OS's own selection handles and loupe. The
//! phones share these, so they can't drift apart.

use std::time::UNIX_EPOCH;

use kolada_core::vault;

use super::Model;

/// A note in the phone's list of them.
#[derive(Clone, Debug, PartialEq)]
pub struct PhoneNote {
    /// Its path in the vault.
    pub path: String,
    pub name: String,
    /// When it was written last, seconds since 1970 (0: not known).
    pub written: u64,
    /// A favorite.
    pub pinned: bool,
}

impl Model {
    /// Every note in the vault, the one written last first.
    pub fn phone_notes(&self) -> Vec<PhoneNote> {
        self.every_note()
            .into_iter()
            .map(|(n, pinned)| {
                let written = n.written.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                PhoneNote { path: n.path, name: n.name, written, pinned }
            })
            .collect()
    }

    /// `phone_notes` as the shells read it: a line each, `path	name	seconds
    /// since 1970	1 when pinned	1 when shared` (a link live).
    pub fn phone_notes_text(&self) -> String {
        let shared = self.shared_notes();
        self.phone_notes()
            .iter()
            .map(|n| format!("{}\t{}\t{}\t{}\t{}\n", clean(&n.path), clean(&n.name), n.written, u8::from(n.pinned), u8::from(shared.contains(&n.path))))
            .collect()
    }

    /// The settings this copy offers, for a phone's own settings page: a
    /// line each, `section	label	setting	value now	1 when a stepper	word=value|...`.
    pub fn phone_settings_text(&self) -> String {
        let mut out = String::new();
        for (o, now) in self.settings_offered() {
            let choices: Vec<String> = o.choices.iter().map(|(w, v)| format!("{w}={v}")).collect();
            out.push_str(&format!("{}\t{}\t{:?}\t{now}\t{}\t{}\n", o.section, o.label, o.setting, u8::from(o.stepper), choices.join("|")));
        }
        out
    }

    /// The setting named `name` (as `phone_settings_text` names it) set to
    /// `value`; false when this copy does not offer it.
    pub fn phone_choose(&mut self, name: &str, value: u8) -> bool {
        let Some((o, _)) = self.settings_offered().into_iter().find(|(o, _)| format!("{:?}", o.setting) == name) else { return false };
        self.choose(o.setting, value)
    }

    /// A settings button: `vault` (another folder), or an import of `files`,
    /// a `folder` or the `clipboard`. What it asks of the OS comes as effects.
    pub fn phone_settings_button(&mut self, what: &str) -> bool {
        use kolada_ui::scene::{Target, Transfer};
        let target = match what {
            "vault" => Target::VaultOpen,
            "files" => Target::Transfer(Transfer::Files),
            "folder" => Target::Transfer(Transfer::Folder),
            "clipboard" => Target::Transfer(Transfer::Clipboard),
            _ => return false,
        };
        self.settings_button(target)
    }

    /// Notes matching `query` in their names or words: a line each,
    /// `path	name	the words around the match`; None while every note is
    /// still being read (ask again after the index's wake).
    pub fn phone_search_text(&mut self, query: &str) -> Option<String> {
        let hits = self.search_notes(query)?;
        Some(hits.iter().map(|h| format!("{}\t{}\t{}\n", clean(&h.path), clean(&h.name), clean(h.snippet.as_deref().unwrap_or("")))).collect())
    }

    /// The notes in the trash, most recently deleted first: `name	when` a line.
    pub fn phone_trashed_text(&mut self) -> String {
        self.trashed_notes().iter().map(|(n, w)| format!("{}\t{}\n", clean(n), clean(w))).collect()
    }

    /// Whether the note at `path` has a link live, as kept here.
    pub fn phone_is_shared(&self, path: &str) -> bool {
        self.shared_notes().iter().any(|n| n == path)
    }

    /// Stops sharing the note at `path`: its live links deleted, said once
    /// done. False when it has none.
    pub fn phone_stop_sharing(&mut self, path: &str) -> bool {
        vault::is_note_path(path) && self.stop_sharing(path)
    }

    /// The note open now, its path in the vault (empty for a fresh page).
    pub fn phone_current(&self) -> String {
        self.note_rel().unwrap_or_default()
    }

    /// Opens the note at `path` (in the vault); false for what is not a note's path.
    pub fn phone_open(&mut self, path: &str) -> bool {
        vault::is_note_path(path) && {
            self.open(path);
            true
        }
    }

    /// Pins the note at `path` (a favorite), or unpins it; whether it changed.
    pub fn phone_pin(&mut self, path: &str) -> bool {
        vault::is_note_path(path) && self.toggle_favorite(path)
    }

    /// The note at `path` goes to the vault's trash; Undo brings it back.
    pub fn phone_trash(&mut self, path: &str) -> bool {
        self.trash_note(path)
    }

    /// The page's colour, 0xRRGGBB: what the shell's window shows where the
    /// note draws nothing (so a band of the frame in it need not be shown).
    pub fn page_color(&self) -> u32 {
        kolada_ui::theme::for_mode(self.dark).background
    }

    /// Where the caret goes at UTF-16 position `at`, DIPs (left, top,
    /// right, bottom), for the OS's own caret, loupe and handles; None
    /// when not drawn.
    pub fn phone_caret_at(&self, at: usize) -> Option<[f32; 4]> {
        self.caret_box_at(at).map(|r| [r.left, r.top, r.right, r.bottom])
    }

    /// The boxes the text from UTF-16 `location`, `length` long, covers:
    /// one a visual line, top to bottom.
    pub fn phone_range_rects(&self, location: usize, length: usize) -> Vec<[f32; 4]> {
        self.range_boxes(location, length).iter().map(|r| [r.left, r.top, r.right, r.bottom]).collect()
    }
}

/// A field of a line: its tabs and line breaks as spaces.
fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\0'], " ")
}

/// The vault's pictures by name, kept between boxes (`note_box`).
static KNOWN: std::sync::Mutex<Option<(std::path::PathBuf, std::collections::HashMap<String, std::path::PathBuf>)>> = std::sync::Mutex::new(None);

/// The box of the note at `path` in the folder `vault`, for a phone's grid
/// of notes (`kolada_ui::preview`); it reads the file alone, so any thread
/// may ask. A line each: its kind (`text`, `picture`, `table` or `list`),
/// its title, its first line of words; then its other lines of words
/// (`text`), a picture's file (empty when it is not found), a table's rows
/// (cells by tabs, the header first), or a list's items (`mark	depth	text`,
/// the mark a bullet, its number, `[ ]` or `[x]`). None when the note
/// cannot be read.
pub fn note_box(vault: &std::path::Path, path: &str) -> Option<String> {
    use kolada_ui::preview::{Mark, Rich, of_note};
    if !vault::is_note_path(path) {
        return None;
    }
    let mut known = KNOWN.lock().unwrap_or_else(|e| e.into_inner());
    let (p, file) = of_note(vault, path, &mut known)?;
    drop(known);
    let mut out = format!("{}\n{}\n{}\n", p.rich.as_ref().map_or("text", Rich::kind), clean(&p.title), clean(&p.line));
    match &p.rich {
        Some(Rich::Picture(_)) => out.push_str(&clean(&file.map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())),
        Some(Rich::Table(rows)) => out.push_str(&rows.iter().map(|r| r.iter().map(|c| clean(c)).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n")),
        Some(Rich::List(items)) => out.push_str(
            &items
                .iter()
                .map(|i| {
                    let mark = match &i.mark {
                        Mark::Bullet => "\u{2022}".to_string(),
                        Mark::Number(n) => clean(n),
                        Mark::Task { done } => if *done { "[x]" } else { "[ ]" }.to_string(),
                    };
                    format!("{mark}\t{}\t{}", i.depth, clean(&i.text))
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        None => out.push_str(&p.lines.iter().skip(1).map(|l| clean(l)).collect::<Vec<_>>().join("\n")),
    }
    Some(out)
}
