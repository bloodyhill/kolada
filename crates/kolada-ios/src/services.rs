//! What iOS gives the model (`kolada_app::services`). The shell hands its
//! answers in once, before the first window, as a table of C functions
//! (`kolada_services`, `KoladaServices` in `apps/ios/Kolada/kolada.h`):
//! UITextChecker for spelling, URLSession for the web, ImageIO for pictures,
//! Vision for the words in them, the user's clock, and where CoreText keeps
//! the system's fonts. Rust holds no Objective-C: everything here builds and
//! is tested on any OS, with stand-in functions. What the table leaves out
//! answers as `Services::none()` does; iOS has no Trash, so a note removed
//! for good from Kolada's trash stays there (the `Bin` refuses).

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use kolada_app::services::{Bin, Calendar, Fonts, Images, Installed, Local, NoServices, Ocr, Read, Services, Spell, Web};
use kolada_core::share::{Raster, Recode};
use kolada_text::FontSet;
use kolada_ui::pictures::Decoder;
use kolada_ui::pixels::Pixels;

/// Bytes handed back to Rust: `sink(to, bytes, length)`, appended, called
/// any number of times.
pub type Sink = extern "C" fn(to: *mut c_void, bytes: *const c_void, length: usize);

pub type ColorGlyphFn = extern "C" fn(name: *const c_char, glyph: u16, size: f32, left: *mut i32, top: *mut i32, width: *mut u32, height: *mut u32, sink: Sink, to: *mut c_void) -> bool;

/// What the shell answers with (each may be null: then as `none()`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KoladaServices {
    /// The misspelled words of a line (UTF-16), written into `out` as start
    /// and end pairs (room for `most` pairs); how many pairs.
    pub spell_check: Option<extern "C" fn(units: *const u16, length: usize, out: *mut usize, most: usize) -> usize>,
    /// Learn Spelling.
    pub spell_learn: Option<extern "C" fn(word: *const c_char)>,
    /// The dictionary's guesses for `word`, one a line.
    pub spell_guesses: Option<extern "C" fn(word: *const c_char, sink: Sink, to: *mut c_void)>,
    /// A request, answered before it returns (called from Rust's own
    /// threads, never the main one): `method` on `url` with `headers`
    /// ("Name: value" a line) and `body`. The status (0: no answer), and at
    /// most `most` bytes of the answer into the sink.
    pub web: Option<extern "C" fn(method: *const c_char, url: *const c_char, headers: *const c_char, body: *const u8, body_length: usize, most: usize, sink: Sink, to: *mut c_void) -> u16>,
    /// A picture's size in pixels as stored (the file at `path`, or `bytes`
    /// when `path` is null); false when it is no picture.
    pub picture_size: Option<extern "C" fn(path: *const c_char, bytes: *const u8, length: usize, width: *mut u32, height: *mut u32) -> bool>,
    /// The picture's first frame as stored, drawn `width` x `height` into
    /// `bgra` (premultiplied, little-endian 0xAARRGGBB words, top row first).
    pub picture_draw: Option<extern "C" fn(path: *const c_char, bytes: *const u8, length: usize, width: u32, height: u32, bgra: *mut u8) -> bool>,
    /// Pixels as `picture_draw` gives them, as a JPEG at `quality` or a PNG.
    pub picture_encode: Option<extern "C" fn(bgra: *const u8, width: u32, height: u32, quality: u8, png: bool, sink: Sink, to: *mut c_void) -> bool>,
    /// The words in a picture, a line each (none is ""); false when there
    /// is no reader now (asked again later). From the index's thread.
    pub picture_text: Option<extern "C" fn(path: *const c_char, sink: Sink, to: *mut c_void) -> bool>,
    /// The time at `seconds` since 1970 as the user's clock writes it.
    pub clock: Option<extern "C" fn(seconds: i64, sink: Sink, to: *mut c_void)>,
    /// The file of the system's font named `name` (a PostScript name), as
    /// CoreText has it, when the app may read it; nothing otherwise.
    pub font_file: Option<extern "C" fn(name: *const c_char, sink: Sink, to: *mut c_void)>,
    /// A colour glyph of the font named (its id, its size in pixels) drawn
    /// by CoreText, as the text engine cannot read iOS's emoji: its
    /// picture's left and top from the glyph's origin (up is positive), its
    /// size, and its premultiplied RGBA rows into the sink.
    pub color_glyph: Option<ColorGlyphFn>,
    /// The Files app's Recently Deleted (FileManager.trashItem): a note removed
    /// for good from Kolada's trash goes there; in the app's own folder,
    /// which has none, it is removed. Whether it went.
    pub recycle: Option<extern "C" fn(path: *const c_char) -> bool>,
    /// Whether the AI app `name` ("Claude") is on this device (its link
    /// scheme opens), for the send menu.
    pub app_installed: Option<extern "C" fn(name: *const c_char) -> bool>,
    /// The fonts inside the app (`Kolada.app/Fonts`), for when the system's
    /// cannot be read.
    pub fonts_dir: *const c_char,
}

// The table is fixed once set and its functions may be called from any thread.
unsafe impl Send for KoladaServices {}
unsafe impl Sync for KoladaServices {}

/// The table, with its folder copied out.
struct Table {
    f: KoladaServices,
    fonts_dir: Option<PathBuf>,
}

static TABLE: OnceLock<Table> = OnceLock::new();

/// The shell's answers, given once before the first `kolada_open`. Later
/// calls are ignored.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_services(services: *const KoladaServices) {
    if services.is_null() {
        return;
    }
    let f = unsafe { *services };
    let fonts_dir = (!f.fonts_dir.is_null()).then(|| PathBuf::from(unsafe { CStr::from_ptr(f.fonts_dir) }.to_string_lossy().into_owned()));
    let _ = TABLE.set(Table { f: KoladaServices { fonts_dir: std::ptr::null(), ..f }, fonts_dir });
}

fn table() -> KoladaServices {
    TABLE.get().map(|t| t.f).unwrap_or_default()
}

/// The services an iOS window's model gets: the shell's, where it gave them.
pub fn ios() -> Services {
    let f = table();
    let mut s = Services { calendar: Arc::new(IosCalendar { clock: f.clock }), fonts: Arc::new(IosFonts { color: f.color_glyph, ..IosFonts::system(f.font_file, TABLE.get().and_then(|t| t.fonts_dir.clone())) }), ..Services::none() };
    if let Some(check) = f.spell_check {
        s.spell = Arc::new(IosSpell { check, learn: f.spell_learn, guesses: f.spell_guesses });
    }
    if let Some(send) = f.web {
        s.web = Arc::new(IosWeb { send });
    }
    if let (Some(size), Some(draw)) = (f.picture_size, f.picture_draw) {
        s.images = Arc::new(IosImages { size, draw, encode: f.picture_encode });
    }
    // Only the app itself watches (tests and dumps keep none: `Model::launch`).
    if TABLE.get().is_some() {
        s.watcher = Arc::new(crate::watch::PollWatcher { every: std::time::Duration::from_secs(3) });
    }
    if let Some(installed) = f.app_installed {
        s.installed = Arc::new(IosInstalled { installed });
    }
    if let Some(recycle) = f.recycle {
        s.bin = Arc::new(IosBin { recycle });
    }
    if let Some(read) = f.picture_text {
        s.ocr = Arc::new(IosOcr { read });
    }
    s
}

/// Appends what the shell hands back to the `Vec<u8>` at `to`.
pub extern "C" fn sink(to: *mut c_void, bytes: *const c_void, length: usize) {
    if to.is_null() || bytes.is_null() || length == 0 {
        return;
    }
    let out = unsafe { &mut *(to as *mut Vec<u8>) };
    out.extend_from_slice(unsafe { std::slice::from_raw_parts(bytes as *const u8, length) });
}

fn c(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

fn path_c(p: &Path) -> CString {
    c(&p.to_string_lossy())
}

/// What a call hands back through the sink, as text.
fn collect(call: impl FnOnce(Sink, *mut c_void)) -> String {
    let mut out: Vec<u8> = Vec::new();
    call(sink, &mut out as *mut Vec<u8> as *mut c_void);
    String::from_utf8_lossy(&out).into_owned()
}

fn collect_bytes(call: impl FnOnce(Sink, *mut c_void) -> bool) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    call(sink, &mut out as *mut Vec<u8> as *mut c_void).then_some(out)
}

/// The local calendar, from the C library (iOS keeps the zone as the Mac
/// does); the clock as the user's setting writes it (12 or 24 hour), from
/// the shell's `DateFormatter`.
pub struct IosCalendar {
    clock: Option<extern "C" fn(i64, Sink, *mut c_void)>,
}

fn secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

impl Calendar for IosCalendar {
    fn local(&self, t: SystemTime) -> Option<Local> {
        let secs = secs(t);
        #[cfg(unix)]
        {
            let at = secs as libc::time_t;
            let mut tm: libc::tm = unsafe { std::mem::zeroed() };
            if unsafe { libc::localtime_r(&at, &mut tm) }.is_null() {
                return None;
            }
            Some(Local {
                year: tm.tm_year + 1900,
                month: tm.tm_mon as u32 + 1,
                day: tm.tm_mday as u32,
                weekday: tm.tm_wday as u32,
                hour: tm.tm_hour as u32,
                minute: tm.tm_min as u32,
                second: tm.tm_sec as u32,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = secs;
            NoServices.local(t)
        }
    }

    fn clock(&self, t: SystemTime) -> String {
        if let Some(clock) = self.clock {
            // The narrow no-break space before AM and PM is not in every font.
            let said = collect(|s, to| clock(secs(t), s, to)).replace(['\u{202F}', '\u{A0}'], " ");
            if !said.is_empty() {
                return said;
            }
        }
        match self.local(t) {
            Some(l) => format!("{:02}:{:02}", l.hour, l.minute),
            None => NoServices.clock(t),
        }
    }
}

/// UITextChecker.
pub struct IosSpell {
    check: extern "C" fn(*const u16, usize, *mut usize, usize) -> usize,
    learn: Option<extern "C" fn(*const c_char)>,
    guesses: Option<extern "C" fn(*const c_char, Sink, *mut c_void)>,
}

/// The most misspellings a line reports.
const MOST_MISSPELLED: usize = 256;

impl Spell for IosSpell {
    fn check(&self, units: &[u16]) -> Vec<(usize, usize)> {
        let mut out = vec![0usize; MOST_MISSPELLED * 2];
        let n = (self.check)(units.as_ptr(), units.len(), out.as_mut_ptr(), MOST_MISSPELLED).min(MOST_MISSPELLED);
        out.chunks_exact(2).take(n).map(|p| (p[0].min(units.len()), p[1].min(units.len()))).filter(|(a, b)| a < b).collect()
    }

    fn learn(&self, word: &str) {
        if let Some(learn) = self.learn {
            learn(c(word).as_ptr());
        }
    }

    fn suggest(&self, word: &str) -> Vec<String> {
        let Some(guesses) = self.guesses else { return Vec::new() };
        let w = c(word);
        collect(|s, to| guesses(w.as_ptr(), s, to)).lines().map(str::trim).filter(|l| !l.is_empty()).take(5).map(String::from).collect()
    }
}

/// URLSession.
pub struct IosWeb {
    send: extern "C" fn(*const c_char, *const c_char, *const c_char, *const u8, usize, usize, Sink, *mut c_void) -> u16,
}

impl Web for IosWeb {
    fn get(&self, address: &str, most: usize) -> Option<Vec<u8>> {
        if !(address.starts_with("https://") || address.starts_with("http://")) {
            return None;
        }
        match self.send("GET", address, &[], &[], most) {
            Some((200..=299, body)) => Some(body),
            _ => None,
        }
    }

    fn send(&self, method: &str, address: &str, headers: &[(&str, &str)], body: &[u8], most: usize) -> Option<(u16, Vec<u8>)> {
        if !(address.starts_with("https://") || address.starts_with("http://")) {
            return None;
        }
        let headers: String = headers.iter().map(|(k, v)| format!("{}: {}\n", k.replace(['\r', '\n'], ""), v.replace(['\r', '\n'], ""))).collect();
        let (method, address, headers) = (c(method), c(address), c(&headers));
        let mut out: Vec<u8> = Vec::new();
        let status = (self.send)(method.as_ptr(), address.as_ptr(), headers.as_ptr(), body.as_ptr(), body.len(), most, sink, &mut out as *mut Vec<u8> as *mut c_void);
        out.truncate(most);
        (status != 0).then_some((status, out))
    }
}

/// ImageIO: pictures in notes (`Decoder`) and pictures made ready for a
/// shared link (`Recode`).
#[derive(Clone, Copy)]
pub struct IosImages {
    size: extern "C" fn(*const c_char, *const u8, usize, *mut u32, *mut u32) -> bool,
    draw: extern "C" fn(*const c_char, *const u8, usize, u32, u32, *mut u8) -> bool,
    encode: Option<extern "C" fn(*const u8, u32, u32, u8, bool, Sink, *mut c_void) -> bool>,
}

/// The largest picture drawn, a side.
const MOST_SIDE: u32 = 8192;

impl IosImages {
    fn size_of(&self, path: Option<&CStr>, bytes: &[u8]) -> Option<(u32, u32)> {
        let (mut w, mut h) = (0u32, 0u32);
        let ok = (self.size)(path.map_or(std::ptr::null(), CStr::as_ptr), bytes.as_ptr(), bytes.len(), &mut w, &mut h);
        (ok && w > 0 && h > 0).then_some((w, h))
    }

    fn draw_at(&self, path: Option<&CStr>, bytes: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
        let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
        (self.draw)(path.map_or(std::ptr::null(), CStr::as_ptr), bytes.as_ptr(), bytes.len(), width, height, bgra.as_mut_ptr()).then_some(bgra)
    }
}

impl Images for IosImages {
    fn decoder(&self) -> Option<Box<dyn Decoder>> {
        Some(Box::new(*self))
    }

    fn recode(&self) -> Option<Box<dyn Recode>> {
        self.encode.is_some().then(|| Box::new(*self) as Box<dyn Recode>)
    }
}

impl Decoder for IosImages {
    fn size(&self, path: &Path) -> Option<(u32, u32)> {
        self.size_of(Some(&path_c(path)), &[])
    }

    fn decode(&self, path: &Path, width: u32) -> Option<Pixels> {
        let p = path_c(path);
        let (w, h) = self.size_of(Some(&p), &[])?;
        let width = width.clamp(1, MOST_SIDE);
        let height = ((h as f64 * width as f64 / w as f64).round() as u32).clamp(1, MOST_SIDE * 2);
        let bgra = self.draw_at(Some(&p), &[], width, height)?;
        let data = bgra.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        Some(Pixels { width, height, data })
    }
}

impl Recode for IosImages {
    fn decode(&self, bytes: &[u8], edge: u32) -> Option<Raster> {
        let (w, h) = self.size_of(None, bytes)?;
        let s = (edge as f64 / w.max(h) as f64).min(1.0);
        let (width, height) = (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1));
        Some(Raster { width, height, bgra: self.draw_at(None, bytes, width, height)? })
    }

    fn encode(&self, raster: &Raster, quality: u8, png: bool) -> Option<Vec<u8>> {
        let encode = self.encode?;
        if raster.bgra.len() != (raster.width as usize) * (raster.height as usize) * 4 {
            return None;
        }
        collect_bytes(|s, to| encode(raster.bgra.as_ptr(), raster.width, raster.height, quality.clamp(1, 100), png, s, to)).filter(|b| !b.is_empty())
    }
}

/// The AI apps on this device, by their link schemes; no command-line tools.
pub struct IosInstalled {
    installed: extern "C" fn(*const c_char) -> bool,
}

impl Installed for IosInstalled {
    fn app(&self, name: &str) -> bool {
        (self.installed)(c(name).as_ptr())
    }

    fn tool(&self, _: &str) -> Option<PathBuf> {
        None
    }
}

/// The Files app's Recently Deleted.
pub struct IosBin {
    recycle: extern "C" fn(*const c_char) -> bool,
}

impl Bin for IosBin {
    fn recycle(&self, path: &Path) -> bool {
        (self.recycle)(path_c(path).as_ptr())
    }
}

/// Vision's text recognition.
pub struct IosOcr {
    read: extern "C" fn(*const c_char, Sink, *mut c_void) -> bool,
}

impl Ocr for IosOcr {
    fn read(&self, path: &Path) -> Read {
        let p = path_c(path);
        match collect_bytes(|s, to| (self.read)(p.as_ptr(), s, to)) {
            Some(words) => Read::Words(String::from_utf8_lossy(&words).into_owned()),
            None => Read::Missing,
        }
    }
}

/// The fonts, named one by one (never a scan): the system's own, found by
/// their PostScript names through CoreText (the shell's `font_file`), so a
/// device reads its own font files wherever its iOS keeps them and the
/// simulator its runtime's, never the Mac's folders. Where the system's
/// cannot be read, the open fonts inside the app (`Kolada.app/Fonts`: Inter,
/// Fira Mono, Noto Sans Arabic and Hebrew). The icons are in the program.
pub struct IosFonts {
    find: Box<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>,
    bundle: Option<PathBuf>,
    /// CoreText's drawing of colour glyphs (iOS's emoji).
    color: Option<ColorGlyphFn>,
}

/// The system's faces: the first screen's (sans text, code), the serif
/// option's, and the fallbacks loaded when text first needs them.
pub const SANS: &[&str] = &["HelveticaNeue"];
pub const MONO: &[&str] = &["Menlo-Regular"];
pub const SYMBOLS: &[&str] = &["AppleSymbols"];
/// What `SYMBOLS` holds that the first screen shows: the keys' signs.
pub const KEY_SIGNS: &str = "\u{2318}\u{21E7}\u{2325}\u{2303}\u{232B}\u{21A9}";
pub const SERIF: &[&str] = &["Georgia", "Georgia-Bold", "Georgia-Italic", "Georgia-BoldItalic"];
pub const FALLBACK: &[&str] = &[
    // Colour emoji; Arabic, Hebrew; Chinese, Japanese, Korean; Thai, Devanagari; symbols.
    "AppleColorEmoji",
    "GeezaPro",
    "ArialHebrew",
    "PingFangSC-Regular",
    "HiraginoSans-W3",
    "AppleSDGothicNeo-Regular",
    "Thonburi",
    "KohinoorDevanagari-Regular",
];

/// The app's own fonts (`assets/fonts/ios`, copied into `Kolada.app/Fonts`).
pub const BUNDLED_TEXT: &[&str] = &["InterVariable.ttf", "InterVariable-Italic.ttf", "FiraMono-Medium.ttf"];
pub const BUNDLED_FALLBACK: &[&str] = &["NotoSansArabic.ttf", "NotoSansHebrew.ttf"];

impl IosFonts {
    /// Fonts found by `font_file` (the shell's CoreText lookup), the app's
    /// own in `bundle` when those cannot be had.
    pub fn system(font_file: Option<extern "C" fn(*const c_char, Sink, *mut c_void)>, bundle: Option<PathBuf>) -> IosFonts {
        IosFonts::with(
            move |name: &str| {
                let file = font_file?;
                let n = c(name);
                let path = collect(|s, to| file(n.as_ptr(), s, to));
                let path = PathBuf::from(path.trim());
                (!path.as_os_str().is_empty() && path.is_file()).then_some(path)
            },
            bundle,
        )
    }

    pub fn with(find: impl Fn(&str) -> Option<PathBuf> + Send + Sync + 'static, bundle: Option<PathBuf>) -> IosFonts {
        IosFonts { find: Box::new(find), bundle, color: None }
    }

    /// Each name's file, once each (a collection holds several faces).
    fn files(&self, names: &[&str]) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for name in names {
            if let Some(p) = (self.find)(name) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }

    fn bundled(&self, names: &[&str]) -> Vec<PathBuf> {
        let Some(dir) = &self.bundle else { return Vec::new() };
        names.iter().map(|n| dir.join(n)).filter(|p| p.is_file()).collect()
    }
}

impl Fonts for IosFonts {
    fn font_set(&self, option: u8) -> FontSet {
        let icons: Vec<&'static [u8]> = vec![include_bytes!("../../../assets/fonts/KoladaIcons.ttf")];
        let (sans, mono) = (self.files(SANS), self.files(MONO));
        if sans.is_empty() {
            // The system's fonts cannot be read: the app's own.
            let mut later = self.bundled(BUNDLED_FALLBACK);
            later.extend(self.files(FALLBACK));
            return FontSet {
                files: self.bundled(BUNDLED_TEXT),
                fallback: later,
                data: icons,
                ui: "Inter Variable".into(),
                text: if option == 2 { "Fira Mono" } else { "Inter Variable" }.into(),
                code: "Fira Mono".into(),
                icon: "Kolada Icons".into(),
            };
        }
        let serif = self.files(SERIF);
        let mut now = sans;
        now.extend(mono.iter().cloned());
        // The keys' signs the first screen shows (⌘N), so drawing them loads no fallback.
        now.extend(self.files(SYMBOLS));
        if option == 1 {
            now.extend(serif.iter().cloned());
        }
        let mut later = self.files(FALLBACK);
        if option != 1 {
            later.extend(serif);
        }
        // Without Menlo, code is set in the app's own monospace face.
        if mono.is_empty() {
            later.extend(self.bundled(&["FiraMono-Medium.ttf"]));
        }
        FontSet {
            files: now,
            fallback: later,
            data: icons,
            ui: "Helvetica Neue".into(),
            text: match option {
                1 => "Georgia",
                2 => "Menlo",
                _ => "Helvetica Neue",
            }
            .into(),
            code: if mono.is_empty() { "Fira Mono" } else { "Menlo" }.into(),
            icon: "Kolada Icons".into(),
        }
    }

    fn covered(&self) -> &'static str {
        if self.files(SANS).is_empty() || self.files(SYMBOLS).is_empty() { "" } else { KEY_SIGNS }
    }

    fn color_glyphs(&self) -> Option<kolada_text::ColorGlyphs> {
        let draw = self.color?;
        Some(Arc::new(move |name: &str, glyph: u16, size: f32| {
            let n = c(name);
            let (mut left, mut top, mut width, mut height) = (0i32, 0i32, 0u32, 0u32);
            let rgba = collect_bytes(|s, to| draw(n.as_ptr(), glyph, size, &mut left, &mut top, &mut width, &mut height, s, to))?;
            (width > 0 && height > 0 && rgba.len() == (width * height * 4) as usize).then(|| kolada_text::Mask { left, top, width, height, color: true, data: rgba })
        }))
    }

    fn note_family(&self, option: u8) -> (String, Vec<PathBuf>) {
        let system = !self.files(SANS).is_empty();
        match (option, system) {
            (1, true) => ("Georgia".into(), self.files(SERIF)),
            (2, true) => ("Menlo".into(), Vec::new()),
            (_, true) => ("Helvetica Neue".into(), Vec::new()),
            (2, false) => ("Fira Mono".into(), Vec::new()),
            // No serif ships inside the app: the sans stands in.
            (_, false) => ("Inter Variable".into(), Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/ios")
    }

    #[test]
    fn reads_the_local_calendar_and_writes_the_users_clock() {
        let l = IosCalendar { clock: None }.local(SystemTime::now()).unwrap();
        assert!(l.year >= 2026 && (1..=12).contains(&l.month) && l.weekday < 7 && l.hour < 24);
        assert_eq!(IosCalendar { clock: None }.clock(SystemTime::now()).len(), 5, "24 hour with no clock from the shell");
        extern "C" fn twelve(_: i64, s: Sink, to: *mut c_void) {
            let said = "9:10\u{202F}PM";
            s(to, said.as_ptr().cast(), said.len());
        }
        assert_eq!(IosCalendar { clock: Some(twelve) }.clock(SystemTime::now()), "9:10 PM", "as the user's setting writes it, in a space every font has");
    }

    #[test]
    fn misspellings_and_guesses_come_from_the_shells_checker() {
        // A checker that marks every word starting with 'x', and guesses two words.
        extern "C" fn check(units: *const u16, n: usize, out: *mut usize, most: usize) -> usize {
            let u = unsafe { std::slice::from_raw_parts(units, n) };
            let out = unsafe { std::slice::from_raw_parts_mut(out, most * 2) };
            let mut found = 0;
            let mut i = 0;
            while i < n && found < most {
                if u[i] == 'x' as u16 && (i == 0 || u[i - 1] == ' ' as u16) {
                    let end = (i..n).find(|&j| u[j] == ' ' as u16).unwrap_or(n);
                    out[found * 2] = i;
                    out[found * 2 + 1] = end;
                    found += 1;
                    i = end;
                }
                i += 1;
            }
            found
        }
        extern "C" fn guesses(_: *const c_char, s: Sink, to: *mut c_void) {
            let g = "plan\nplans\n";
            s(to, g.as_ptr().cast(), g.len());
        }
        let spell = IosSpell { check, learn: None, guesses: Some(guesses) };
        let units: Vec<u16> = "a xplan and xlist".encode_utf16().collect();
        assert_eq!(spell.check(&units), vec![(2, 7), (12, 17)]);
        assert_eq!(spell.suggest("xplan"), vec!["plan", "plans"]);
    }

    #[test]
    fn the_web_is_asked_for_through_the_shell_with_its_headers_and_body() {
        extern "C" fn send(method: *const c_char, url: *const c_char, headers: *const c_char, body: *const u8, n: usize, most: usize, s: Sink, to: *mut c_void) -> u16 {
            let (m, u, h) = unsafe { (CStr::from_ptr(method).to_str().unwrap(), CStr::from_ptr(url).to_str().unwrap(), CStr::from_ptr(headers).to_str().unwrap()) };
            let b = unsafe { std::slice::from_raw_parts(body, n) };
            let answer = format!("{m} {u} [{}] {} {most}", h.trim_end().replace('\n', "|"), String::from_utf8_lossy(b));
            s(to, answer.as_ptr().cast(), answer.len());
            if u.ends_with("/missing") { 404 } else { 201 }
        }
        let web = IosWeb { send };
        let (status, body) = web.send("POST", "https://notsy.co/api/links", &[("X-Kolada-Key", "k"), ("Content-Type", "text/plain")], b"hello", 4096).unwrap();
        assert_eq!(status, 201);
        assert_eq!(String::from_utf8(body).unwrap(), "POST https://notsy.co/api/links [X-Kolada-Key: k|Content-Type: text/plain] hello 4096");
        assert!(web.get("https://example.com/missing", 100).is_none(), "an error page is no picture");
        assert!(web.get("https://example.com/a.png", 100).is_some());
        assert!(web.get("file:///etc/passwd", 100).is_none(), "only http and https");
        let (_, cut) = web.send("GET", "https://example.com/big", &[], &[], 3).unwrap();
        assert_eq!(cut.len(), 3, "never more than asked for");
    }

    /// A stand-in ImageIO: every picture is 40 x 20, drawn in one colour.
    extern "C" fn size(_: *const c_char, bytes: *const u8, n: usize, w: *mut u32, h: *mut u32) -> bool {
        let none = !bytes.is_null() && n > 0 && unsafe { *bytes } == b'!';
        unsafe {
            *w = 40;
            *h = 20;
        }
        !none
    }
    extern "C" fn draw(_: *const c_char, _: *const u8, _: usize, w: u32, h: u32, bgra: *mut u8) -> bool {
        let px = unsafe { std::slice::from_raw_parts_mut(bgra, (w * h * 4) as usize) };
        for p in px.chunks_exact_mut(4) {
            p.copy_from_slice(&[0x94, 0x95, 0x0E, 0xFF]);
        }
        true
    }
    extern "C" fn encode(_: *const u8, w: u32, h: u32, q: u8, png: bool, s: Sink, to: *mut c_void) -> bool {
        let said = format!("{} {w}x{h} q{q}", if png { "png" } else { "jpeg" });
        s(to, said.as_ptr().cast(), said.len());
        true
    }

    #[test]
    fn pictures_are_sized_drawn_and_made_ready_to_share_through_imageio() {
        let images = IosImages { size, draw, encode: Some(encode) };
        let d = images.decoder().unwrap();
        assert_eq!(d.size(Path::new("/v/a.png")), Some((40, 20)));
        let px = d.decode(Path::new("/v/a.png"), 20).unwrap();
        assert_eq!((px.width, px.height), (20, 10), "the shape kept");
        assert_eq!(px.data[0], 0xFF0E_9594, "little-endian words, 0xAARRGGBB");
        let r = images.recode().unwrap();
        let raster = r.decode(b"jpeg", 10).unwrap();
        assert_eq!((raster.width, raster.height, raster.bgra.len()), (10, 5, 200), "never more than the edge");
        assert!(r.decode(b"!no", 10).is_none());
        assert_eq!(r.encode(&raster, 82, false).unwrap(), b"jpeg 10x5 q82");
        assert!(IosImages { size, draw, encode: None }.recode().is_none(), "no encoder: nothing is shared with pictures");
    }

    #[test]
    fn words_in_pictures_come_from_vision_or_wait_for_it() {
        extern "C" fn read(path: *const c_char, s: Sink, to: *mut c_void) -> bool {
            let p = unsafe { CStr::from_ptr(path) }.to_str().unwrap();
            if p.ends_with("later.png") {
                return false;
            }
            let w = "Total\n42";
            s(to, w.as_ptr().cast(), w.len());
            true
        }
        let ocr = IosOcr { read };
        assert_eq!(ocr.read(Path::new("/v/receipt.png")), Read::Words("Total\n42".into()));
        assert_eq!(ocr.read(Path::new("/v/later.png")), Read::Missing);
    }

    #[test]
    fn fonts_are_the_systems_as_coretext_finds_them_and_never_the_macs_folders() {
        let dir = std::env::temp_dir().join(format!("kolada-ios-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The device's own files, wherever its iOS keeps them.
        for f in ["HelveticaNeue.ttc", "Menlo.ttc", "Georgia.ttf", "AppleColorEmoji@2x.ttc", "PingFang.ttc"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let d = dir.clone();
        let fonts = IosFonts::with(
            move |name| {
                let file = match name {
                    "HelveticaNeue" => "HelveticaNeue.ttc",
                    "Menlo-Regular" => "Menlo.ttc",
                    n if n.starts_with("Georgia") => "Georgia.ttf",
                    "AppleColorEmoji" => "AppleColorEmoji@2x.ttc",
                    "PingFangSC-Regular" => "PingFang.ttc",
                    _ => return None,
                };
                Some(d.join(file))
            },
            Some(assets()),
        );
        let set = fonts.font_set(0);
        assert_eq!(set.files, vec![dir.join("HelveticaNeue.ttc"), dir.join("Menlo.ttc")], "the first screen's: text and code");
        assert_eq!(set.fallback, vec![dir.join("AppleColorEmoji@2x.ttc"), dir.join("PingFang.ttc"), dir.join("Georgia.ttf")], "the rest when needed, Georgia's four faces in one file once");
        assert_eq!((set.ui.as_str(), set.text.as_str(), set.code.as_str(), set.icon.as_str()), ("Helvetica Neue", "Helvetica Neue", "Menlo", "Kolada Icons"));
        assert!(set.files.iter().chain(&set.fallback).all(|p| p.starts_with(&dir)), "nothing from the Mac's /System/Library/Fonts");
        let serif = fonts.font_set(1);
        assert!(serif.files.contains(&dir.join("Georgia.ttf")) && serif.text == "Georgia");
        assert_eq!(fonts.note_family(2).0, "Menlo");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn where_the_systems_fonts_cannot_be_read_the_apps_own_are_drawn_with() {
        let fonts = IosFonts::with(|_| None, Some(assets()));
        let set = fonts.font_set(0);
        assert_eq!(set.files.len(), BUNDLED_TEXT.len(), "Inter, its italic and Fira Mono are in the app: {:?}", set.files);
        assert_eq!(set.fallback.len(), BUNDLED_FALLBACK.len(), "and Arabic and Hebrew: {:?}", set.fallback);
        let e = kolada_text::Engine::new(set.clone());
        for family in [set.ui.as_str(), set.code.as_str(), set.icon.as_str()] {
            assert!(e.families().iter().any(|f| f == family), "{family} in {:?}", e.families());
        }
        assert_eq!(fonts.note_family(1).0, "Inter Variable", "no serif inside: the sans stands in");
        let none = IosFonts::with(|_| None, None).font_set(0);
        assert!(none.files.is_empty() && none.data.len() == 1, "nothing to read at all: the icons still draw");
    }

    #[test]
    fn emoji_the_text_engine_cannot_read_are_drawn_by_coretext() {
        extern "C" fn draw(name: *const c_char, glyph: u16, size: f32, left: *mut i32, top: *mut i32, w: *mut u32, h: *mut u32, s: Sink, to: *mut c_void) -> bool {
            assert_eq!(unsafe { CStr::from_ptr(name) }.to_str().unwrap(), "AppleColorEmoji");
            if glyph == 0 {
                return false;
            }
            let side = size as u32;
            unsafe {
                (*left, *top, *w, *h) = (1, side as i32 - 2, side, side);
            }
            let px = vec![0x80u8; (side * side * 4) as usize];
            s(to, px.as_ptr().cast(), px.len());
            true
        }
        let fonts = IosFonts { color: Some(draw), ..IosFonts::with(|_| None, None) };
        let glyphs = fonts.color_glyphs().expect("the shell draws them");
        let m = glyphs("AppleColorEmoji", 7, 20.0).unwrap();
        assert_eq!((m.left, m.top, m.width, m.height, m.color, m.data.len()), (1, 18, 20, 20, true, 1600));
        assert!(glyphs("AppleColorEmoji", 0, 20.0).is_none());
        assert!(IosFonts::with(|_| None, None).color_glyphs().is_none(), "nothing given: the engine's own");
    }

    #[test]
    fn a_note_removed_for_good_goes_to_the_files_apps_recently_deleted() {
        extern "C" fn recycle(path: *const c_char) -> bool {
            unsafe { CStr::from_ptr(path) }.to_str().unwrap().ends_with("Old.md")
        }
        let bin = IosBin { recycle };
        assert!(bin.recycle(Path::new("/v/.trash/Old.md")));
        assert!(!bin.recycle(Path::new("/v/.trash/Kept.md")), "where iOS cannot, the note stays in Kolada's trash");
    }

    #[test]
    fn the_send_menu_offers_the_claude_app_when_it_is_on_the_device() {
        extern "C" fn installed(name: *const c_char) -> bool {
            unsafe { CStr::from_ptr(name) }.to_str().unwrap() == "Claude"
        }
        let i = IosInstalled { installed };
        let found = kolada_app::model::share::Agent::detect_with(|a| i.app(a), |t| i.tool(t).is_some());
        assert_eq!(found, vec![kolada_app::model::share::Agent::Claude], "no command-line tools on iOS");
    }

    #[test]
    fn the_table_given_once_becomes_the_models_services() {
        // Nothing given: the calendar and the fonts still answer.
        let s = ios();
        assert!(s.calendar.local(SystemTime::now()).is_some());
        assert!(s.spell.check(&[0x78]).is_empty());
    }
}
