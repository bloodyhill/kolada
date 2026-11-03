//! What a phone's native shell asks of the model (the iPhone's UIKit, and
//! Android's Views): it draws its own navigation, list of notes and bars,
//! and the model draws the note alone inside them (`set_bare`). So: every
//! note in the vault, a note opened, pinned (a favorite) or trashed, and
//! where the text is, for the OS's own selection handles and loupe. Each
//! phone's binding (the iPhone's over C, Android's over JNI) is only
//! a wrapper over these, so the two phones can't drift apart.

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
    /// since 1970	1 when pinned`.
    pub fn phone_notes_text(&self) -> String {
        let clean = |s: &str| s.replace(['\t', '\n', '\0'], " ");
        self.phone_notes()
            .iter()
            .map(|n| format!("{}\t{}\t{}\t{}\n", clean(&n.path), clean(&n.name), n.written, u8::from(n.pinned)))
            .collect()
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
