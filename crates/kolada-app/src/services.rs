//! What the app asks of the operating system, and nothing else.
//!
//! The model is plain Rust: it never calls an OS API. Everything that needs
//! one (the local calendar, the spell checker, reading words in a picture,
//! the web, the OS's trash, watching the vault's folder, the system's fonts)
//! is a small trait here, and the shell hands the model a [`Services`] when it
//! builds it. [`Services::none`] answers every one of them with something
//! harmless and deterministic (UTC, no misspellings, no OCR, no network, no
//! watcher), which is what tests and dumps run on.
//!
//! The traits are `Send + Sync` because the model calls several of them from
//! threads of its own (indexing, fetching pictures, reading files).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use kolada_text::FontSet;
use kolada_ui::pictures::Decoder;
use kolada_ui::when::{Day, LocalTime};

/// A moment on the local calendar and clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    /// 1 to 31.
    pub day: u32,
    /// 0 for Sunday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// The user's calendar and clock.
pub trait Calendar: Send + Sync {
    /// Where `t` falls on the local calendar and clock (None if the OS cannot say).
    fn local(&self, t: SystemTime) -> Option<Local>;
    /// `t`'s clock time as the user's settings write it ("9:10 AM" or "09:10").
    fn clock(&self, t: SystemTime) -> String;
}

/// Now, in local time (the Unix epoch if the calendar cannot say).
pub fn now(calendar: &dyn Calendar) -> Local {
    calendar.local(SystemTime::now()).unwrap_or(Local { year: 1970, month: 1, day: 1, weekday: 4, hour: 0, minute: 0, second: 0 })
}

/// A [`Calendar`] seen as the shared words' `LocalTime`.
pub struct Words<'a>(pub &'a dyn Calendar);

impl LocalTime for Words<'_> {
    fn day(&self, t: SystemTime) -> Option<Day> {
        self.0.local(t).map(|l| (l.year, l.month, l.day))
    }

    fn clock(&self, t: SystemTime) -> String {
        self.0.clock(t)
    }
}

/// Spelling, checked by the system's own dictionaries (and the words the
/// user taught them).
pub trait Spell: Send + Sync {
    /// The misspelled words of `units` (a line's prose as UTF-16, with what
    /// is not prose blanked: see `kolada_ui::spell::prose`), as UTF-16 ranges.
    fn check(&self, units: &[u16]) -> Vec<(usize, usize)>;
    /// Teaches the system's dictionary `word` (Learn Spelling).
    fn learn(&self, word: &str);
    /// Up to five words the dictionary offers for `word`.
    fn suggest(&self, word: &str) -> Vec<String>;
}

/// What reading a picture came to.
#[derive(Debug, PartialEq)]
pub enum Read {
    /// The words, a line each (empty when the picture holds none, or the
    /// reader could not make sense of the file).
    Words(String),
    /// The OS has no reader (or not now): nothing was learned, try again later.
    Missing,
}

/// Text recognition: the words in a picture. Called from the index's thread,
/// never the UI's.
pub trait Ocr: Send + Sync {
    fn read(&self, path: &Path) -> Read;
}

/// The web: a picture fetched for a paste, and a note shared as a link
/// (called from threads of their own).
pub trait Web: Send + Sync {
    /// The bytes at `address` (http or https only), if they come and are no
    /// more than `most`.
    fn get(&self, address: &str, most: usize) -> Option<Vec<u8>>;

    /// A request (`method` on `address`, with `headers` and `body`) and
    /// whatever the server answers, any status: it and at most `most` bytes
    /// of the body. None when no answer comes. For notsy.co links.
    fn send(&self, method: &str, address: &str, headers: &[(&str, &str)], body: &[u8], most: usize) -> Option<(u16, Vec<u8>)> {
        let _ = (method, address, headers, body, most);
        None
    }

    /// 32 random bytes from the OS, for the install's key (made the first
    /// time a link is shared). Where there is `/dev/urandom` it is read.
    fn random(&self) -> Option<[u8; 32]> {
        use std::io::Read;
        let mut b = [0u8; 32];
        std::fs::File::open("/dev/urandom").ok()?.read_exact(&mut b).ok()?;
        Some(b)
    }
}

/// The OS's own trash, where a note removed for good from Kolada's trash goes
/// (never beyond), so the OS can still put it back.
pub trait Bin: Send + Sync {
    /// Moves the file at `path` there: whether it went.
    fn recycle(&self, path: &Path) -> bool;
}

/// `Change::flags`: the path was created, removed, or renamed (a move is two
/// changes, both renamed). A watcher translates what its OS says into these.
pub const ITEM_CREATED: u32 = 0x0100;
pub const ITEM_REMOVED: u32 = 0x0200;
pub const ITEM_RENAMED: u32 = 0x0800;

/// A path that changed, and what happened to it (`ITEM_*`). A move is two
/// changes, both flagged renamed: the old path (no longer there) and the new,
/// in the order they happened.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub path: PathBuf,
    pub flags: u32,
}

/// Following what other apps do to the vault.
pub trait Watcher: Send + Sync {
    /// Watches `vault` on a thread of its own: the paths that change arrive
    /// in the receiver, and `wake` is called (from that thread) after each batch.
    fn watch(&self, vault: &Path, wake: Box<dyn Fn() + Send>) -> Receiver<Change>;
}

/// Where the OS lets a thread or a child process run below the window's.
pub trait Priority: Send + Sync {
    /// The calling thread goes to background priority (the index's thread).
    fn lower_thread(&self);
    /// The process `pid` (the writing model's helper) runs below the app.
    fn lower_process(&self, pid: u32);
}

/// The fonts the app draws with.
pub trait Fonts: Send + Sync {
    /// The font set for the Font option (0 sans serif, 1 serif, 2
    /// monospace): a core set for the first screen, and fallbacks loaded
    /// when text first needs them. Name files, never scan.
    fn font_set(&self, option: u8) -> FontSet;
    /// The family notes are set in for a font option, and the files that
    /// hold it when `font_set` did not load them (a face the option moves to).
    fn note_family(&self, option: u8) -> (String, Vec<PathBuf>);
}

/// Picture decoding: the OS's half of `kolada_ui::pictures`. Called on the
/// pictures' worker thread.
pub trait Images: Send + Sync {
    fn decoder(&self) -> Option<Box<dyn Decoder>>;

    /// Pictures decoded from their bytes and encoded again (JPEG or PNG),
    /// for a note shared as a link; on the thread that shares.
    fn recode(&self) -> Option<Box<dyn kolada_core::share::Recode>> {
        None
    }
}

/// What is installed on this machine, for the send menu's AI apps.
pub trait Installed: Send + Sync {
    /// Whether the app `name` ("Claude") is installed where the OS keeps apps.
    fn app(&self, name: &str) -> bool;
    /// Where the command-line tool `name` ("codex") is, if it can be found
    /// (an app opened from the shell's own menu has no PATH to look in).
    fn tool(&self, name: &str) -> Option<PathBuf>;
}

/// Everything the model needs from the OS. Cheap to clone (each is shared).
#[derive(Clone)]
pub struct Services {
    pub calendar: Arc<dyn Calendar>,
    pub spell: Arc<dyn Spell>,
    pub ocr: Arc<dyn Ocr>,
    pub web: Arc<dyn Web>,
    /// The OS's trash. The model only uses it once `Model::launch` or
    /// `Model::note_window` makes it the app's (tests and dumps keep none).
    pub bin: Arc<dyn Bin>,
    pub watcher: Arc<dyn Watcher>,
    pub priority: Arc<dyn Priority>,
    pub fonts: Arc<dyn Fonts>,
    pub images: Arc<dyn Images>,
    pub installed: Arc<dyn Installed>,
}

impl Services {
    /// Harmless stand-ins for all of them ([`NoServices`]): for tests, dumps
    /// and a shell's first steps. Replace the fields a shell has.
    pub fn none() -> Services {
        let n = Arc::new(NoServices);
        Services { calendar: n.clone(), spell: n.clone(), ocr: n.clone(), web: n.clone(), bin: n.clone(), watcher: n.clone(), priority: n.clone(), fonts: n.clone(), images: n.clone(), installed: n }
    }
}

impl Default for Services {
    fn default() -> Services {
        Services::none()
    }
}

/// The stand-in for every service: UTC and 24 hour clock, nothing
/// misspelled, no words in pictures, no network, a trash that refuses, a
/// watcher that never reports, no priorities, the test fonts, no picture decoder.
pub struct NoServices;

fn secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

impl Calendar for NoServices {
    fn local(&self, t: SystemTime) -> Option<Local> {
        let s = secs(t);
        let (days, rest) = (s.div_euclid(86_400), s.rem_euclid(86_400));
        // Days since 1970-01-01 to a civil date (proleptic Gregorian).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Some(Local { year, month, day, weekday: (days + 4).rem_euclid(7) as u32, hour: (rest / 3600) as u32, minute: (rest % 3600 / 60) as u32, second: (rest % 60) as u32 })
    }

    fn clock(&self, t: SystemTime) -> String {
        self.local(t).map(|l| format!("{:02}:{:02}", l.hour, l.minute)).unwrap_or_default()
    }
}

impl Spell for NoServices {
    fn check(&self, _: &[u16]) -> Vec<(usize, usize)> {
        Vec::new()
    }
    fn learn(&self, _: &str) {}
    fn suggest(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

impl Ocr for NoServices {
    fn read(&self, _: &Path) -> Read {
        Read::Missing
    }
}

impl Web for NoServices {
    fn get(&self, _: &str, _: usize) -> Option<Vec<u8>> {
        None
    }

    fn random(&self) -> Option<[u8; 32]> {
        None
    }
}

impl Bin for NoServices {
    fn recycle(&self, _: &Path) -> bool {
        false
    }
}

impl Watcher for NoServices {
    fn watch(&self, _: &Path, _: Box<dyn Fn() + Send>) -> Receiver<Change> {
        channel().1
    }
}

impl Priority for NoServices {
    fn lower_thread(&self) {}
    fn lower_process(&self, _: u32) {}
}

impl Fonts for NoServices {
    fn font_set(&self, _: u8) -> FontSet {
        kolada_ui::testing::fonts()
    }

    fn note_family(&self, option: u8) -> (String, Vec<PathBuf>) {
        let set = kolada_ui::testing::fonts();
        (if option == 2 { set.code } else { set.text }, Vec::new())
    }
}

impl Installed for NoServices {
    fn app(&self, _: &str) -> bool {
        false
    }
    fn tool(&self, _: &str) -> Option<PathBuf> {
        None
    }
}

impl Images for NoServices {
    fn decoder(&self) -> Option<Box<dyn Decoder>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stand_in_calendar_reads_utc() {
        // 2026-10-09 (a Friday) 13:45:07 UTC.
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_791_553_507);
        let l = NoServices.local(t).unwrap();
        assert_eq!((l.year, l.month, l.day, l.weekday, l.hour, l.minute, l.second), (2026, 10, 9, 5, 13, 45, 7));
        assert_eq!(NoServices.clock(t), "13:45");
        let before = UNIX_EPOCH + std::time::Duration::from_secs(951_782_400);
        assert_eq!(NoServices.local(before).map(|l| (l.year, l.month, l.day)), Some((2000, 2, 29)));
    }
}
