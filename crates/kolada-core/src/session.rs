//! The session snapshot: everything the first frame needs, in one small file.
//!
//! It is read before the window exists, so it must be tiny and quick to
//! decode. It is written atomically (temp file, then rename) and
//! checksummed. A damaged or old-format snapshot is ignored and the app
//! starts fresh; it is a cache, never the source of truth.

use std::fs;
use std::io;
use std::path::Path;

const MAGIC: [u8; 4] = *b"NSES";
const VERSION: u32 = 16;
/// Recent notes kept: the side menu shows up to five besides the open one.
pub const RECENT: usize = 6;
/// Favorites kept at most.
pub const FAVORITES: usize = 40;
/// Bits of `Session::folded`: a side menu section shut.
pub const FOLD_FAVORITES: u8 = 1;
pub const FOLD_RECENT: u8 = 2;
/// Recent hidden altogether (the side menu's right-click menu).
pub const HIDE_RECENT: u8 = 4;
/// The vault has more than `RECENT_FROM` notes, as last counted: Recent
/// shows then. Kept so the first frame knows before the vault is read.
pub const MANY_NOTES: u8 = 8;
/// Recent shows only in a vault of more notes than this; in a smaller one
/// the tree, newest edited first, is enough.
pub const RECENT_FROM: usize = 25;

/// Whether the side menu shows Recent, by `Session::folded`.
pub fn shows_recent(folded: u8) -> bool {
    folded & MANY_NOTES != 0 && folded & HIDE_RECENT == 0
}

/// File tree rows kept for the first frame: more than the tallest window
/// shows. The rest of the tree comes back from `expanded` once the vault is
/// listed. (A long list made the snapshot big, and a big file just written
/// is scanned by the antivirus on its first read: ~8 ms for 120 KB, at
/// every launch.)
const FIRST_ROWS: usize = 160;
const HEADER: usize = 4 + 4 + 4 + 8;

/// Smart resume: a note active this recently reopens where it was left;
/// after longer, Kolada opens on a fresh page.
pub const RESUME_WINDOW_SECS: u64 = 60 * 60;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub window: Placement,
    pub dark: bool,
    /// Absolute vault path.
    pub vault: String,
    /// Open note, relative to the vault, with forward slashes.
    pub note: String,
    /// Byte offset of the first line at the top of the viewport.
    pub scroll_line: u64,
    /// How far that line is scrolled past the top, in DIPs.
    pub scroll_px: f32,
    /// Caret byte offset in the note.
    pub caret: u64,
    /// The sidebar rows visible last time, so the first frame can draw the
    /// file tree without listing the vault. Saved, only its first rows.
    pub sidebar: Vec<SidebarRow>,
    /// Folders open in the file tree (vault-relative) as last saved, beyond
    /// what the saved rows say; empty once the vault is listed again.
    pub expanded: Vec<String>,
    /// The side menu is collapsed to its thin pill.
    pub sidebar_collapsed: bool,
    /// The toolbar is collapsed to its thin pill.
    pub toolbar_collapsed: bool,
    /// When the note was last used (typed in, opened, or left), in seconds
    /// since the Unix epoch.
    pub last_active: u64,
    /// The note was created by typing on a fresh page, so its file name
    /// still follows its first line.
    pub captured: bool,
    /// Options. Appearance: 0 follows the system, 1 light, 2 dark.
    pub theme: u8,
    /// Text size and line width: 0 is the default, 1 smaller or narrower,
    /// 2 larger or wider.
    pub text_size: u8,
    pub line_width: u8,
    /// The note's typeface: 0 sans serif, 1 serif, 2 monospace.
    pub font: u8,
    /// What a launch opens: 0 the last note if it was in use within the
    /// hour, else a fresh page; 1 always the last note; 2 always a fresh page.
    pub launch: u8,
    /// Related notes in the margin: 0 shown, 1 hidden.
    pub related: u8,
    /// Spelling marked: 0 yes, 1 no.
    pub spell: u8,
    /// The notification area: 0 an icon there, 1 none, 2 an icon, and
    /// closing the window keeps the app running there.
    pub tray: u8,
    /// Win+Alt+N from any app for a fresh page: 0 yes, 1 no.
    pub hotkey: u8,
    /// The side menu's social drawer: 2 open; folded until it is opened
    /// (0, as it starts, or 1, folded again).
    pub social: u8,
    /// The side menu's width as set by dragging its edge, DIPs; 0 for the
    /// default.
    pub sidebar_width: u16,
    /// What a launch opens: 0 the full window, 1 the minimal one (the note
    /// alone, small).
    pub window_mode: u8,
    /// The window is minimal now, at `minimal_window`; `window` is where
    /// the full one was.
    pub minimal: bool,
    pub minimal_window: Placement,
    /// Writing help from the model on this PC: 0 automatic, 1 off.
    pub assist: u8,
    /// Reminders of tasks due: 0 a nudge in the morning and at a task's
    /// time, 1 none.
    pub remind: u8,
    /// The notes opened last in this vault, newest first (vault-relative,
    /// with `.md`): the side menu's Recent.
    pub recent: Vec<String>,
    /// Notes and folders kept at hand above Recent (vault-relative; a
    /// folder ends with `/`, a note with `.md`).
    pub favorites: Vec<String>,
    /// Side menu sections shut (`FOLD_FAVORITES`, `FOLD_RECENT`).
    pub folded: u8,
    /// Checking for updates, once a day, outside a store: 0 yes, 1 no.
    pub updates: u8,
}

/// Restored window position, in physical pixels at `dpi`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub dpi: u32,
    pub maximized: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SidebarRow {
    pub depth: u8,
    pub kind: RowKind,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RowKind {
    Folder { expanded: bool },
    Note,
}

impl Session {
    /// A first run's settings: every feature on (as `Default` has them),
    /// and closing the window keeps Kolada running in the background, beside
    /// the clock, so it comes back at once.
    pub fn first_run() -> Session {
        Session { tray: 2, ..Session::default() }
    }

    /// The vault was counted: `notes` in it. True if that changes whether
    /// Recent can show.
    pub fn counted(&mut self, notes: usize) -> bool {
        let before = self.folded;
        if notes > RECENT_FROM {
            self.folded |= MANY_NOTES;
        } else {
            self.folded &= !MANY_NOTES;
        }
        before != self.folded
    }

    pub fn has_window(&self) -> bool {
        self.window.width > 0 && self.window.height > 0 && self.window.dpi > 0
    }

    /// Whether launching at `now` should reopen the last note rather than
    /// open a fresh page.
    pub fn resumes(&self, now: u64) -> bool {
        !self.note.is_empty()
            && match self.launch {
                1 => true,
                2 => false,
                _ => now.saturating_sub(self.last_active) < RESUME_WINDOW_SECS,
            }
    }

    /// The note at `rel` was opened: first among the recent ones.
    pub fn remember(&mut self, rel: &str) {
        if rel.is_empty() {
            return;
        }
        self.recent.retain(|n| n != rel);
        self.recent.insert(0, rel.to_string());
        self.recent.truncate(RECENT);
    }

    /// A note was renamed or moved: Recent and Favorites follow it.
    pub fn renamed(&mut self, from: &str, to: &str) {
        for n in self.recent.iter_mut().chain(self.favorites.iter_mut()).filter(|n| *n == from) {
            *n = to.to_string();
        }
        self.recent.dedup();
    }

    /// Adds `path` (a note's, with `.md`, or a folder's, ending in `/`) to
    /// the favorites, or takes it off them. Whether it is one now.
    pub fn toggle_favorite(&mut self, path: &str) -> bool {
        if let Some(i) = self.favorites.iter().position(|f| f == path) {
            self.favorites.remove(i);
            return false;
        }
        if self.favorites.len() >= FAVORITES || path.is_empty() {
            return false;
        }
        self.favorites.push(path.to_string());
        true
    }

    /// Leaves the note for a fresh page, keeping the vault and the window.
    pub fn start_fresh(&mut self) {
        self.note.clear();
        self.scroll_line = 0;
        self.scroll_px = 0.0;
        self.caret = 0;
        self.captured = false;
    }
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn load(path: &Path) -> Option<Session> {
    decode(&fs::read(path).ok()?)
}

pub fn save(path: &Path, session: &Session) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, encode(session))?;
    // Replaces the old snapshot atomically on NTFS.
    fs::rename(&tmp, path)
}

pub fn encode(s: &Session) -> Vec<u8> {
    let mut p = Vec::with_capacity(256);
    let w = &s.window;
    for v in [w.x, w.y, w.width, w.height] {
        p.extend_from_slice(&v.to_le_bytes());
    }
    p.extend_from_slice(&w.dpi.to_le_bytes());
    p.push(w.maximized as u8);
    p.push(s.dark as u8);
    p.push(s.sidebar_collapsed as u8);
    p.push(s.toolbar_collapsed as u8);
    put_str(&mut p, &s.vault);
    put_str(&mut p, &s.note);
    p.extend_from_slice(&s.scroll_line.to_le_bytes());
    p.extend_from_slice(&s.scroll_px.to_le_bytes());
    p.extend_from_slice(&s.caret.to_le_bytes());
    let rows = &s.sidebar[..s.sidebar.len().min(FIRST_ROWS)];
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for row in rows {
        p.push(row.depth);
        p.push(match row.kind {
            RowKind::Note => 0,
            RowKind::Folder { expanded: false } => 1,
            RowKind::Folder { expanded: true } => 2,
        });
        put_str(&mut p, &row.name);
    }
    p.extend_from_slice(&s.last_active.to_le_bytes());
    p.push(s.captured as u8);
    p.extend_from_slice(&[s.theme, s.text_size, s.line_width]);
    p.extend_from_slice(&[s.font, s.launch, s.related]);
    p.extend_from_slice(&[s.spell, s.tray]);
    p.push(s.hotkey);
    p.push(s.social);
    p.extend_from_slice(&s.sidebar_width.to_le_bytes());
    p.push(s.window_mode);
    p.push(s.minimal as u8);
    let m = &s.minimal_window;
    for v in [m.x, m.y, m.width, m.height] {
        p.extend_from_slice(&v.to_le_bytes());
    }
    p.extend_from_slice(&m.dpi.to_le_bytes());
    p.push(m.maximized as u8);
    let mut open: Vec<String> = crate::vault::expanded_folders(&s.sidebar).into_iter().chain(s.expanded.iter().cloned()).collect();
    open.sort();
    open.dedup();
    p.extend_from_slice(&(open.len() as u32).to_le_bytes());
    for folder in &open {
        put_str(&mut p, folder);
    }
    p.push(s.assist);
    p.push(s.remind);
    let recent = &s.recent[..s.recent.len().min(RECENT)];
    p.extend_from_slice(&(recent.len() as u32).to_le_bytes());
    for note in recent {
        put_str(&mut p, note);
    }
    let favorites = &s.favorites[..s.favorites.len().min(FAVORITES)];
    p.extend_from_slice(&(favorites.len() as u32).to_le_bytes());
    for f in favorites {
        put_str(&mut p, f);
    }
    p.push(s.folded);
    p.push(s.updates);

    let mut out = Vec::with_capacity(HEADER + p.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(p.len() as u32).to_le_bytes());
    out.extend_from_slice(&fnv1a(&p).to_le_bytes());
    out.extend_from_slice(&p);
    out
}

pub fn decode(bytes: &[u8]) -> Option<Session> {
    if bytes.len() < HEADER || bytes[..4] != MAGIC {
        return None;
    }
    let mut r = Reader { b: &bytes[4..] };
    // Older versions are this one without some fields at the end.
    let version = r.u32()?;
    if !(2..=VERSION).contains(&version) {
        return None;
    }
    let len = r.u32()? as usize;
    let sum = r.u64()?;
    let payload = r.b;
    if payload.len() != len || fnv1a(payload) != sum {
        return None;
    }

    let mut r = Reader { b: payload };
    let window = Placement {
        x: r.i32()?,
        y: r.i32()?,
        width: r.i32()?,
        height: r.i32()?,
        dpi: r.u32()?,
        maximized: r.u8()? != 0,
    };
    let dark = r.u8()? != 0;
    let sidebar_collapsed = r.u8()? != 0;
    let toolbar_collapsed = r.u8()? != 0;
    let vault = r.str()?;
    let note = r.str()?;
    let scroll_line = r.u64()?;
    let scroll_px = f32::from_le_bytes(r.take(4)?.try_into().ok()?);
    let caret = r.u64()?;
    let rows = r.u32()? as usize;
    let mut sidebar = Vec::with_capacity(rows.min(4096));
    for _ in 0..rows {
        let depth = r.u8()?;
        let kind = match r.u8()? {
            0 => RowKind::Note,
            1 => RowKind::Folder { expanded: false },
            2 => RowKind::Folder { expanded: true },
            _ => return None,
        };
        sidebar.push(SidebarRow { depth, kind, name: r.str()? });
    }
    let (last_active, captured) = if version >= 3 { (r.u64()?, r.u8()? != 0) } else { (0, false) };
    let (theme, text_size, line_width) = if version >= 4 { (r.u8()?, r.u8()?, r.u8()?) } else { (0, 0, 0) };
    let (font, launch, related) = if version >= 5 { (r.u8()?, r.u8()?, r.u8()?) } else { (0, 0, 0) };
    let (spell, tray) = if version >= 6 { (r.u8()?, r.u8()?) } else { (0, 0) };
    let hotkey = if version >= 7 { r.u8()? } else { 0 };
    let social = if version >= 8 { r.u8()? } else { 0 };
    let sidebar_width = if version >= 9 { r.u16()? } else { 0 };
    let (window_mode, minimal, minimal_window) = if version >= 10 {
        let (mode, minimal) = (r.u8()?, r.u8()? != 0);
        let placed = Placement { x: r.i32()?, y: r.i32()?, width: r.i32()?, height: r.i32()?, dpi: r.u32()?, maximized: r.u8()? != 0 };
        (mode, minimal, placed)
    } else {
        (0, false, Placement::default())
    };
    let expanded = if version >= 11 {
        let n = r.u32()? as usize;
        let mut open = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            open.push(r.str()?);
        }
        open
    } else {
        Vec::new()
    };
    let assist = if version >= 12 { r.u8()? } else { 0 };
    let remind = if version >= 13 { r.u8()? } else { 0 };
    let recent = if version >= 14 {
        let n = r.u32()? as usize;
        if n > RECENT {
            return None;
        }
        (0..n).map(|_| r.str()).collect::<Option<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let (favorites, folded) = if version >= 15 {
        let n = r.u32()? as usize;
        if n > FAVORITES {
            return None;
        }
        ((0..n).map(|_| r.str()).collect::<Option<Vec<_>>>()?, r.u8()?)
    } else {
        (Vec::new(), 0)
    };
    let updates = if version >= 16 { r.u8()? } else { 0 };
    if !r.b.is_empty() {
        return None;
    }
    Some(Session {
        window,
        dark,
        vault,
        note,
        scroll_line,
        scroll_px,
        caret,
        sidebar,
        expanded,
        sidebar_collapsed,
        toolbar_collapsed,
        last_active,
        captured,
        theme,
        text_size,
        line_width,
        font,
        launch,
        related,
        spell,
        tray,
        hotkey,
        social,
        sidebar_width,
        window_mode,
        minimal,
        minimal_window,
        assist,
        remind,
        recent,
        favorites,
        folded,
        updates,
    })
}

fn put_str(p: &mut Vec<u8>, s: &str) {
    p.extend_from_slice(&(s.len() as u32).to_le_bytes());
    p.extend_from_slice(s.as_bytes());
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.b.len() < n {
            return None;
        }
        let (head, tail) = self.b.split_at(n);
        self.b = tail;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn sample() -> Session {
        Session {
            window: Placement { x: -8, y: 10, width: 2400, height: 1600, dpi: 288, maximized: true },
            dark: true,
            vault: r"C:\Users\me\my notes".into(),
            note: "Folder/ملاحظة 1.md".into(),
            scroll_line: 1234,
            scroll_px: 7.5,
            caret: 1300,
            sidebar: vec![
                SidebarRow { depth: 0, kind: RowKind::Folder { expanded: true }, name: "Folder".into() },
                SidebarRow { depth: 1, kind: RowKind::Note, name: "ملاحظة 1".into() },
                SidebarRow { depth: 0, kind: RowKind::Folder { expanded: false }, name: "Archive".into() },
            ],
            expanded: vec!["Folder".into()],
            sidebar_collapsed: true,
            toolbar_collapsed: false,
            last_active: 1_790_000_000,
            captured: true,
            theme: 2,
            text_size: 1,
            line_width: 2,
            font: 1,
            launch: 2,
            related: 1,
            spell: 1,
            tray: 2,
            hotkey: 1,
            social: 1,
            sidebar_width: 312,
            window_mode: 1,
            minimal: true,
            minimal_window: Placement { x: 1900, y: 60, width: 1740, height: 2160, dpi: 288, maximized: false },
            assist: 1,
            remind: 1,
            recent: vec!["Plans/Weekly plan.md".into(), "Moving flat.md".into()],
            favorites: vec!["Work/".into(), "Plans/Weekly plan.md".into()],
            folded: FOLD_RECENT,
            updates: 1,
        }
    }

    #[test]
    fn roundtrips() {
        let s = sample();
        assert_eq!(decode(&encode(&s)), Some(s));
        assert_eq!(decode(&encode(&Session::default())), Some(Session::default()));
    }

    #[test]
    fn rejects_damage() {
        let bytes = encode(&sample());
        for i in 0..bytes.len() {
            let mut bad = bytes.clone();
            bad[i] ^= 0x40;
            assert_eq!(decode(&bad), None, "flipped byte {i} was accepted");
        }
        for n in 0..bytes.len() {
            assert_eq!(decode(&bytes[..n]), None, "truncated to {n} was accepted");
        }
    }

    #[test]
    fn recent_notes_come_newest_first_follow_renames_and_stay_few() {
        let mut s = Session::default();
        for n in ["a.md", "b.md", "a.md", "c.md", "", "d.md", "e.md", "f.md", "g.md"] {
            s.remember(n);
        }
        assert_eq!(s.recent, ["g.md", "f.md", "e.md", "d.md", "c.md", "a.md"]);
        s.renamed("c.md", "Work/c2.md");
        assert_eq!(s.recent[4], "Work/c2.md");
        assert_eq!(decode(&encode(&s)).unwrap().recent, s.recent);
    }

    #[test]
    fn favorites_are_kept_in_order_toggle_and_follow_renames() {
        let mut s = Session::default();
        assert!(s.toggle_favorite("Work/") && s.toggle_favorite("Plan.md") && s.toggle_favorite("Ideas.md"));
        assert!(!s.toggle_favorite("Plan.md"), "a second time takes it off");
        assert_eq!(s.favorites, ["Work/", "Ideas.md"]);
        s.renamed("Ideas.md", "Later/Ideas.md");
        assert_eq!(s.favorites[1], "Later/Ideas.md");
        s.folded = FOLD_FAVORITES;
        let back = decode(&encode(&s)).unwrap();
        assert_eq!((&back.favorites, back.folded), (&s.favorites, FOLD_FAVORITES));
        for i in 0..FAVORITES + 5 {
            s.toggle_favorite(&format!("{i}.md"));
        }
        assert_eq!(s.favorites.len(), FAVORITES, "a few dozen at most");
    }

    #[test]
    fn reads_version_2_snapshots() {
        let mut bytes = encode(&sample());
        // Drop the version 3 to 16 fields and fix up the header.
        let recent: usize = sample().recent.iter().chain(&sample().favorites).map(|n| 4 + n.len()).sum::<usize>() + 4 + 4 + 1 + 1;
        let payload_len = bytes.len() - HEADER - 60 - recent;
        bytes.truncate(HEADER + payload_len);
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&(payload_len as u32).to_le_bytes());
        let sum = fnv1a(&bytes[HEADER..]);
        bytes[12..20].copy_from_slice(&sum.to_le_bytes());
        let s = decode(&bytes).expect("version 2 still loads");
        assert_eq!((s.last_active, s.captured), (0, false));
        assert_eq!(s.note, sample().note);
    }

    #[test]
    fn recent_shows_past_25_notes_unless_hidden() {
        let mut s = Session::default();
        assert!(!shows_recent(s.folded), "not before the vault is counted");
        assert!(!s.counted(RECENT_FROM) && !shows_recent(s.folded), "25 notes: the tree is enough");
        assert!(s.counted(RECENT_FROM + 1) && shows_recent(s.folded));
        s.folded |= HIDE_RECENT;
        assert!(!shows_recent(s.folded), "hidden from the side menu's right-click menu");
        s.folded &= !HIDE_RECENT;
        s.folded |= FOLD_RECENT;
        assert!(shows_recent(s.folded), "folded shut is still shown, as its heading");
        assert!(s.counted(3) && !shows_recent(s.folded), "notes deleted down to a few");
        let back = decode(&encode(&Session { folded: MANY_NOTES | HIDE_RECENT, ..s })).unwrap();
        assert_eq!(back.folded, MANY_NOTES | HIDE_RECENT, "kept in the snapshot");
    }

    #[test]
    fn a_first_run_has_everything_on_and_keeps_running_in_the_background() {
        let s = Session::first_run();
        assert_eq!(s.tray, 2, "closing keeps Kolada running");
        // 0 is on for each of these.
        assert_eq!((s.spell, s.hotkey, s.assist, s.remind, s.related, s.updates, s.launch), (0, 0, 0, 0, 0, 0, 0));
        assert_eq!(decode(&encode(&s)), Some(s));
    }

    #[test]
    fn reads_version_15_snapshots_with_updates_on() {
        let mut bytes = encode(&sample());
        // Version 15 had no update setting: the last byte.
        bytes.pop();
        bytes[4..8].copy_from_slice(&15u32.to_le_bytes());
        let len = (bytes.len() - HEADER) as u32;
        bytes[8..12].copy_from_slice(&len.to_le_bytes());
        let sum = fnv1a(&bytes[HEADER..]);
        bytes[12..20].copy_from_slice(&sum.to_le_bytes());
        let s = decode(&bytes).expect("version 15 still loads");
        assert_eq!(s.updates, 0, "checking for updates starts on");
        assert_eq!(Session { updates: 1, ..s }, sample());
    }

    #[test]
    fn resumes_only_a_recent_note() {
        let mut s = Session { launch: 0, ..sample() };
        let t = s.last_active;
        assert!(s.resumes(t + 60));
        assert!(s.resumes(t + RESUME_WINDOW_SECS - 1));
        assert!(!s.resumes(t + RESUME_WINDOW_SECS));
        assert!(s.resumes(t - 5), "a clock that went back still resumes");
        s.start_fresh();
        assert!(!s.resumes(t) && s.note.is_empty() && s.caret == 0 && !s.captured);
        assert_eq!(s.vault, sample().vault);
    }

    #[test]
    fn launch_option_can_always_or_never_resume() {
        let t = sample().last_active;
        let always = Session { launch: 1, ..sample() };
        assert!(always.resumes(t + 30 * 24 * 3600));
        let never = Session { launch: 2, ..sample() };
        assert!(!never.resumes(t + 1));
    }

    #[test]
    fn a_big_tree_saves_small_and_opens_as_it_was() {
        // Two thousand rows, folders open deep in the list.
        let mut sidebar = Vec::new();
        for f in 0..200 {
            sidebar.push(SidebarRow { depth: 0, kind: RowKind::Folder { expanded: f % 3 == 0 }, name: format!("Folder {f}") });
            if f % 3 == 0 {
                sidebar.push(SidebarRow { depth: 1, kind: RowKind::Folder { expanded: true }, name: "Inner".into() });
                sidebar.extend((0..8).map(|n| SidebarRow { depth: 2, kind: RowKind::Note, name: format!("A note with a longish name {n}") }));
            }
        }
        let s = Session { sidebar, ..sample() };
        let bytes = encode(&s);
        assert!(bytes.len() < 16 * 1024, "the snapshot is {} bytes", bytes.len());
        let back = decode(&bytes).unwrap();
        assert_eq!(back.sidebar[..], s.sidebar[..FIRST_ROWS], "the first rows, as they were");
        let mut open: HashSet<String> = back.expanded.iter().cloned().collect();
        open.extend(crate::vault::expanded_folders(&back.sidebar));
        let mut want = crate::vault::expanded_folders(&s.sidebar);
        want.insert("Folder".into());
        assert_eq!(open, want, "every open folder, even far down");
    }

    /// Times loading a real snapshot (KOLADA_SESSION=file).
    #[test]
    #[ignore = "reads the snapshot named by KOLADA_SESSION"]
    fn time_a_real_snapshot() {
        let path = std::path::PathBuf::from(std::env::var("KOLADA_SESSION").unwrap());
        let bytes = fs::read(&path).unwrap();
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let s = decode(&bytes).unwrap();
            let decoded = t.elapsed();
            let t = std::time::Instant::now();
            let read = fs::read(&path).unwrap();
            println!("decode {decoded:?} ({} rows, {} bytes), read {:?}", s.sidebar.len(), read.len(), t.elapsed());
        }
    }

    #[test]
    fn saves_atomically_and_loads() {
        let dir = std::env::temp_dir().join(format!("kolada-session-test-{}", std::process::id()));
        let path = dir.join("session.bin");
        save(&path, &sample()).unwrap();
        save(&path, &sample()).unwrap(); // replaces an existing file
        assert_eq!(load(&path), Some(sample()));
        assert!(!path.with_extension("tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
