//! Quick Send on Windows (docs/specs/2026-10-26-sync-quick-send.md; the
//! popup the user chose on 2026-10-27): a small window of its own above the
//! clock, like the clipboard panel, never part of the side menu. It opens on
//! the keys (Win+Shift+Z by default), from the tray's menu, from Settings'
//! Devices button and from a notification. What was copied shows ready, and
//! Enter beams it. The engine and its connection are `crate::quick`; what
//! the popup shows is `kolada_ui::quick`.

use std::sync::Arc;
use std::time::SystemTime;

use kolada_sync::{Event, Kind as SendKind, Outgoing, PairState, Refusal};
use kolada_core::share::Recode;
use kolada_ui::pixels::Pixels;
use kolada_ui::quick::{self as q, ClipView, DeviceRow, Hit, Host, Kind, Pairing, PicView, PopupView, Screen, Staged};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    UnregisterHotKey, VK_BACK, VK_CONTROL, VK_ESCAPE, VK_RETURN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, GetCursorPos, IDC_ARROW, IsWindowVisible, LoadCursorW, RegisterClassExW, SW_HIDE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, WM_ACTIVATE, WM_CHAR, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_PAINT, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::w;

use super::*;
use crate::cpu::Surface;

/// The hotkey ids: beam the clipboard, and beam it as a password.
pub const QUICK_SEND: i32 = 2;
pub const QUICK_SECRET: i32 = 3;
/// A copied password leaves the clipboard after this long, if still there.
pub const CLEAR_TIMER: usize = 60;
const CLEAR_MS: u32 = 60_000;
/// While a code shows, its countdown moves once a second.
pub const COUNT_TIMER: usize = 61;
/// A group's beams are each within this long of the one before (ms).
const GROUP_MS: u64 = 5 * 60_000;
/// While a card opens or closes, its spring steps this often.
const OPEN_TIMER: usize = 65;
/// "Beamed" shows this long, then the popup goes.
pub const HIDE_TIMER: usize = 62;
/// The Beam button stays faint this long after a keystroke (63 is
/// `HIDE_TIMER + 1`).
const TYPING_TIMER: usize = 64;
const TYPING_MS: u32 = 1500;
/// What the popup says shows this long.
const SAID: Duration = Duration::from_secs(8);
/// "Copied" shows this long on a clip's button.
const COPIED: Duration = Duration::from_millis(1500);
/// The popup keeps this far from the screen's edges, DIPs.
const EDGE: f32 = 12.0;
/// A thumbnail's longer side, pixels: sharp on the card at 2x.
const THUMB_EDGE: u32 = 360;
/// The largest picture file that is read to beam.
const PICTURE_MOST: u64 = kolada_sync::engine::PICTURE_MOST as u64;

/// The keys, by `Session::quick_keys`: the beam, and its password twin.
/// Alt+Z is the user's choice (2026-10-27), as apps like Gemini take
/// Alt+Space; where another app holds it (NVIDIA's overlay does), Beam
/// takes Win+Shift+Z instead. The password twin stays Win+Ctrl+Shift+Z:
/// Alt+Shift switches the keyboard's language on Windows.
const KEYS: [(&str, HOT_KEY_MODIFIERS, HOT_KEY_MODIFIERS); 2] = [
    ("Alt+Z", HOT_KEY_MODIFIERS(MOD_ALT.0), HOT_KEY_MODIFIERS(MOD_WIN.0 | MOD_CONTROL.0 | MOD_SHIFT.0)),
    ("Win+Shift+Z", HOT_KEY_MODIFIERS(MOD_WIN.0 | MOD_SHIFT.0), HOT_KEY_MODIFIERS(MOD_WIN.0 | MOD_CONTROL.0 | MOD_SHIFT.0)),
];

/// A clip as the list shows it, kept between frames.
struct Shown {
    id: kolada_sync::ItemId,
    text: String,
    title: String,
    from: String,
    goes: String,
    kind: Kind,
    spent: bool,
    mine: bool,
    /// A picture card's size in pixels, file size, and whether its bytes
    /// are here.
    picture: Option<(u32, u32, String, bool)>,
    /// The rest of the pictures beamed with it, each as `picture` is.
    rest: Vec<(kolada_sync::ItemId, (u32, u32, String, bool))>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Showing {
    FirstUse,
    Main,
    Devices,
}

/// What was copied, ready to beam.
struct Ready {
    text: String,
    kind: Kind,
    picture: Option<ReadyPicture>,
    /// More pictures picked with `picture` (a file name and each), to go
    /// together (`Outgoing::Pictures`).
    rest: Vec<(String, ReadyPicture)>,
    /// Maybe a password: masked, with "Not a password".
    unsure: bool,
}

/// A picture ready to beam: its file (a bitmap from the clipboard becomes
/// a PNG off the UI thread), and its thumbnail once made.
struct ReadyPicture {
    bytes: Vec<u8>,
    mime: String,
    size: String,
    thumb: Option<(Vec<u8>, Arc<Pixels>, u32, u32)>,
    /// Which staging this is (what is made for an older one is dropped).
    n: u64,
}

/// What a worker thread made for the popup.
enum Made {
    Staged { n: u64, jpeg: Vec<u8>, pixels: Arc<Pixels>, width: u32, height: u32, file: Option<Vec<u8>> },
    Card { id: kolada_sync::ItemId, pixels: Arc<Pixels> },
}

/// A picture to stage: a file's bytes, or the clipboard's bitmap.
enum Source {
    File(Vec<u8>, String),
    Bitmap(u32, u32, Vec<u8>),
}

pub struct Quick {
    pub net: Option<crate::quick::Net>,
    window: Option<HWND>,
    surface: Option<Surface>,
    fonts: Option<Fonts>,
    hits: Vec<(kolada_ui::geom::Rect, Target)>,
    hover: Option<Target>,
    tracking: bool,
    showing: Showing,
    ready: Option<Ready>,
    /// Who to beam to: 0 all, then each other device.
    chosen: usize,
    scroll: f32,
    /// How far the clips can scroll, as the last frame found.
    scroll_max: f32,
    /// The note a clip was just saved as: its name and path.
    saved: Option<(String, PathBuf, Instant)>,
    /// What was sent from here and how each stands: pending, its check,
    /// or failed (`q::Marks`).
    marks: q::Marks,
    typing: Option<String>,
    /// This device's name being typed (Settings > Devices' Rename).
    naming: Option<String>,
    said: Option<(String, Instant)>,
    copied: Option<(kolada_sync::ItemId, Instant)>,
    on_clipboard: Option<String>,
    held: bool,
    cards: Vec<Shown>,
    devices: Vec<(kolada_sync::DeviceId, String, String, bool)>,
    paired: bool,
    pair: Option<PairState>,
    /// The first use's steps were seen.
    intro_seen: bool,
    /// The last notification was Quick Send's: a click on it opens the popup.
    pub balloon: bool,
    /// When the keys were pressed, until the popup shows: key to visible.
    pub pressed: Option<Instant>,
    /// What is typed in the send area.
    draft: String,
    /// The cards' thumbnails, decoded, and the ones being decoded.
    thumbs: Vec<(kolada_sync::ItemId, Arc<Pixels>)>,
    decoding: Vec<kolada_sync::ItemId>,
    made: Option<(std::sync::mpsc::Sender<Made>, std::sync::mpsc::Receiver<Made>)>,
    stagings: u64,
    /// A file dialog of the popup's is open: losing the keys to it does not
    /// put the popup away.
    picking: bool,
    /// The beam under way is what was typed (the field empties once it goes).
    beaming_draft: bool,
    /// The side menu's row's second line (`BeamView::line`).
    row_line: String,
    /// The Beam button's menu is open, for the note named so.
    menu: Option<String>,
    /// The note was clicked in or written in since launch: the button
    /// offers it.
    engaged: bool,
    /// When the note was last written in (the button fades while typing).
    typed_at: Option<Instant>,
    /// Note sync's loop, and the vault it keeps (see `start_notes_sync`).
    syncer: Option<kolada_sync::syncer::Syncer>,
    sync_vault: String,
    /// When sync was last told the open note is being typed in.
    sync_typed: Option<Instant>,
    /// Beam was asked for while a copied bitmap was still being made a
    /// PNG: it goes as soon as the PNG is ready.
    beam_waiting: bool,
    /// Which of `KEYS` is held, if any.
    in_use: Option<usize>,
    /// Cards opened by a tap: each one's spring (where it is, how fast,
    /// where it is going), by item; when they last stepped.
    opens: Vec<(kolada_sync::ItemId, f32, f32, bool)>,
    stepped: Option<Instant>,
    /// The card whose "…" menu is open.
    more: Option<usize>,
}

impl Default for Quick {
    fn default() -> Self {
        Quick {
            net: None,
            window: None,
            surface: None,
            fonts: None,
            hits: Vec::new(),
            hover: None,
            tracking: false,
            showing: Showing::FirstUse,
            ready: None,
            chosen: 0,
            scroll: 0.0,
            scroll_max: 0.0,
            saved: None,
            marks: q::Marks::default(),
            typing: None,
            naming: None,
            said: None,
            copied: None,
            on_clipboard: None,
            held: false,
            cards: Vec::new(),
            devices: Vec::new(),
            paired: false,
            pair: None,
            intro_seen: false,
            balloon: false,
            pressed: None,
            draft: String::new(),
            thumbs: Vec::new(),
            decoding: Vec::new(),
            made: None,
            stagings: 0,
            picking: false,
            beaming_draft: false,
            row_line: String::new(),
            menu: None,
            engaged: false,
            typed_at: None,
            syncer: None,
            sync_vault: String::new(),
            sync_typed: None,
            beam_waiting: false,
            in_use: None,
            opens: Vec::new(),
            stepped: None,
            more: None,
        }
    }
}

/// Pairing as Settings and the popup draw it: the code being typed, or
/// what the engine says.
fn pairing_of<'a>(typing: Option<&'a str>, pair: Option<&'a PairState>, left: &'a str) -> Pairing<'a> {
    match (typing, pair) {
        (Some(t), _) => Pairing::Entering { typed: t },
        (None, Some(PairState::Opening)) => Pairing::Opening,
        (None, Some(PairState::Showing { code, .. })) => Pairing::Showing { code, left },
        (None, Some(PairState::Asks { name })) => Pairing::Asks { name },
        (None, Some(PairState::Waiting)) => Pairing::Waiting,
        _ => Pairing::Idle,
    }
}

pub(super) fn devices_view<'a>(d: &'a super::DevicesLive<'a>) -> kolada_ui::scene::DevicesView<'a> {
    kolada_ui::scene::DevicesView { devices: &d.rows, pairing: pairing_of(d.typing, d.pair, &d.left), said: d.said, facts: &d.facts, this: "PC", naming: d.naming }
}

/// One thing the watcher saw, for note sync.
#[derive(Debug, PartialEq)]
enum SyncStep {
    Changed(String),
    Moved(String, String),
}

/// What the watcher's list means to note sync: renames paired (old name,
/// new name), the rest changed; None to look at the whole vault again (too
/// much to list, or a folder renamed or removed, which moves every note in
/// it).
fn sync_steps(list: &[(crate::watch::Action, String)], overflow: bool) -> Option<Vec<SyncStep>> {
    use crate::watch::Action;
    let folder = |p: &str| !p.rsplit('/').next().unwrap_or(p).contains('.');
    if overflow || list.iter().any(|(a, p)| *a != Action::Modified && folder(p)) {
        return None;
    }
    let mut out = Vec::new();
    let mut from: Option<&str> = None;
    for (action, path) in list {
        match action {
            Action::RenamedFrom => from = Some(path),
            Action::RenamedTo => out.push(match from.take() {
                Some(f) => SyncStep::Moved(f.to_string(), path.clone()),
                None => SyncStep::Changed(path.clone()),
            }),
            _ => out.push(SyncStep::Changed(path.clone())),
        }
    }
    Some(out)
}

fn intro_file() -> Option<PathBuf> {
    kolada_core::app_data_dir().map(|d| d.join("sync").join("intro.seen"))
}

/// When `ms` was, as people say it.
fn ago(ms: u64) -> String {
    crate::when::ago(SystemTime::UNIX_EPOCH + Duration::from_millis(ms))
}

/// A picture file's type, from its name: None if it is not a picture.
fn picture_type(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "heic" | "heif" => "image/heic",
        "tif" | "tiff" => "image/tiff",
        _ => return None,
    })
}

fn picture_ext(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/heic" => "heic",
        "image/tiff" => "tiff",
        _ => "png",
    }
}

/// How big a file is, as people say it.
fn file_size(n: u64) -> String {
    match n {
        0..1_000 => format!("{n} bytes"),
        1_000..1_000_000 => format!("{} KB", n / 1_000),
        _ => format!("{:.1} MB", n as f64 / 1_000_000.0),
    }
}

/// Decoded pixels as the scene draws them (premultiplied).
fn pixels_of(r: &kolada_core::share::Raster) -> Pixels {
    let data = r
        .bgra
        .chunks_exact(4)
        .map(|p| {
            let (b, g, red, a) = (p[0] as u32, p[1] as u32, p[2] as u32, p[3] as u32);
            let m = |c: u32| (c * a + 127) / 255;
            (a << 24) | (m(red) << 16) | (m(g) << 8) | m(b)
        })
        .collect();
    Pixels { width: r.width, height: r.height, data }
}

/// Windows' "Animation effects" (Settings, Accessibility, Visual effects):
/// off, a sent beam's check fades in and out without its spring.
fn animate() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
    let mut on: i32 = 1;
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut i32 as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    on != 0
}

fn kind_of(text: &str, secret: bool) -> Kind {
    let t = text.trim();
    if secret {
        Kind::Secret
    } else if (t.starts_with("https://") || t.starts_with("http://")) && !t.contains(char::is_whitespace) {
        Kind::Link
    } else {
        Kind::Text
    }
}

/// What this PC is, and what it is called until the user names it: a
/// laptop where it has a battery; the computer's own name, unless Windows
/// made it up (`LAPTOP-B9TBEDG7`), then "Windows laptop" or "Windows PC".
fn this_pc() -> (&'static str, String) {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows::Win32::System::SystemInformation::{ComputerNamePhysicalDnsHostname, GetComputerNameExW};
    let mut power = SYSTEM_POWER_STATUS::default();
    // 128: no system battery; 255: not known.
    let laptop = unsafe { GetSystemPowerStatus(&mut power) }.is_ok() && power.BatteryFlag & 128 == 0 && power.BatteryFlag != 255;
    let model = kolada_sync::names::desktop_model(laptop);
    // The name as typed, not COMPUTERNAME's capitals.
    let mut buf = [0u16; 256];
    let mut n = buf.len() as u32;
    let host = unsafe { GetComputerNameExW(ComputerNamePhysicalDnsHostname, Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut n) }.is_ok().then(|| String::from_utf16_lossy(&buf[..n as usize]));
    (model, kolada_sync::names::default_name(host.as_deref(), model))
}

impl App {
    /// After the first frame: the engine, from the keys kept here. It opens
    /// no connection until this device is paired.
    pub(super) fn start_quick(&mut self) {
        if self.compact {
            return;
        }
        let Some(data) = kolada_core::app_data_dir() else { return };
        let base = std::env::var("KOLADA_SYNC_BASE").unwrap_or_else(|_| kolada_sync::SERVER.to_string());
        let (model, name) = this_pc();
        let name = std::env::var("KOLADA_SYNC_NAME").unwrap_or(name);
        let net = crate::quick::Net::start(crate::Handoff(self.hwnd), data, &base, &name);
        net.with(|e, _| e.set_model(model));
        self.quick.net = Some(net);
        self.quick.intro_seen = intro_file().is_some_and(|p| p.exists());
        self.quick_refresh();
        self.update_quick_hotkey(false);
        self.start_notes_sync();
        // Made once, hidden, and drawn once, so the keys only show it: no
        // window to create, no glyphs to rasterize on the first press.
        if let Some(h) = self.quick_window() {
            self.quick_place(h, q::H);
            self.quick_render(h);
        }
    }

    /// Note sync (spec section 8), for the vault open now. Only where asked
    /// for (`KOLADA_SYNC_NOTES=1`) until the history that keeps other
    /// devices' versions has shipped a release on every platform: an older
    /// Kolada would cut those versions off. Its writes reach the open note
    /// through the vault's watcher, as any outside change does.
    fn start_notes_sync(&mut self) {
        if std::env::var("KOLADA_SYNC_NOTES").as_deref() == Ok("0") || self.session.vault.is_empty() {
            return;
        }
        let (Some(net), Some(data)) = (&self.quick.net, kolada_core::app_data_dir()) else { return };
        let root = std::path::PathBuf::from(&self.session.vault);
        let open = move || kolada_sync::vault::Vault::open(&root, &data, true);
        // Switched on here: the group's other devices sync notes with it
        // (a phone that follows, or another desktop switched on).
        net.with(|e, _| e.set_sync(kolada_sync::SyncCap::On));
        self.quick.syncer = Some(kolada_sync::syncer::Syncer::open(open, net.engine(), net.kicker(), Arc::new(|_| {})));
        self.quick.sync_vault = self.session.vault.clone();
    }

    /// What the vault's watcher saw, for note sync.
    pub(super) fn sync_saw(&self, changes: &crate::watch::Changes) {
        let Some(s) = &self.quick.syncer else { return };
        if changes.vault != Path::new(&self.quick.sync_vault) {
            return;
        }
        match sync_steps(&changes.list, changes.overflow) {
            None => s.rescan(),
            Some(steps) => {
                for step in steps {
                    match step {
                        SyncStep::Changed(p) => s.changed(&p),
                        SyncStep::Moved(from, to) => s.moved(&from, &to),
                    }
                }
            }
        }
    }

    /// The open note was typed in: sync keeps its text against what comes
    /// in (told at most once a second).
    pub(super) fn sync_typing(&mut self) {
        let Some(s) = &self.quick.syncer else { return };
        if self.quick.sync_typed.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        s.typing(&self.session.note);
        self.quick.sync_typed = Some(Instant::now());
    }

    /// The engine said something (`WM_APP_SYNC`).
    pub(super) fn quick_events(&mut self) {
        self.quick_thumbs();
        let open = self.quick_open();
        let front = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() } == self.hwnd;
        let Some(net) = &self.quick.net else { return };
        let (mut notes, mut says) = (Vec::new(), Vec::new());
        let mut paired_now = false;
        let mut acked = false;
        for e in net.take_events() {
            match e {
                Event::Arrived(id) if !open && !front => {
                    // A picture that came with a note is told as part of the note.
                    let card = net.read(|e| e.cards().into_iter().find(|c| c.id == id).cloned()).filter(|c| c.picture.as_ref().is_none_or(|p| p.note.is_none()));
                    if let Some(c) = card {
                        let from = net.read(|e| e.devices().iter().find(|d| d.id == c.from).map(|d| d.name.clone())).unwrap_or_default();
                        let title = if from.is_empty() { "Beam".to_string() } else { format!("From {from}") };
                        let body = match c.kind {
                            SendKind::Secret => "A password: click to copy it once".to_string(),
                            SendKind::Picture => "A picture".to_string(),
                            _ => c.text.chars().take(200).collect(),
                        };
                        notes.push((title, body));
                    }
                }
                Event::Paired => {
                    paired_now = true;
                    says.push("Paired. Copy something and press the keys to beam it.");
                }
                Event::CodeRefused(why) => says.push(refusal(why)),
                Event::Removed => says.push("This PC is no longer paired."),
                Event::Resting { .. } => says.push("Your devices reconnect tomorrow; nothing is lost."),
                Event::Sent(id) => {
                    if self.quick.marks.acked(id, Instant::now()) {
                        acked = true;
                    }
                }
                // A failure shows as its card's red mark, and a reason the
                // user can act on is said; a picture's card goes with its
                // upload, so its failure is said.
                Event::SendFailed(id, why) => {
                    let kept = net.read(|e| e.cards().iter().any(|c| c.id == id));
                    if kept && self.quick.marks.failed(id) {
                        if matches!(why.as_str(), "slow" | "limit" | "full" | "size") {
                            says.push(q::failure(&why));
                        }
                    } else {
                        self.quick.marks.forget(id);
                        says.push(q::failure(&why));
                    }
                }
                Event::Lost(id) => {
                    self.quick.marks.lost(id);
                    says.push("A beam didn't arrive in 7 days.");
                }
                Event::FetchFailed(_) => says.push("A picture couldn't be opened, so it was left out."),
                Event::Synced { id, from, name, batch } => {
                    if let Some(s) = &self.quick.syncer {
                        s.batch(id, from, name, batch);
                    }
                }
                _ => {}
            }
        }
        for (title, body) in notes {
            self.notify(&title, &body);
            self.quick.balloon = true;
        }
        for s in says {
            self.say(s);
        }
        if acked {
            self.marks_moving();
        }
        let was = self.quick.paired;
        self.quick_refresh();
        if was != self.quick.paired {
            self.update_quick_hotkey(false);
        }
        if paired_now && self.quick.showing == Showing::Devices {
            self.quick.showing = Showing::Main;
        }
        // What arrives while the popup shows its clips is seen there.
        if open && self.quick.showing == Showing::Main {
            self.quick_seen();
        }
        self.quick_draw();
        // The side menu's count.
        self.request_frame();
    }

    /// The paired devices, as Settings > Devices and the popup list them.
    pub(super) fn device_rows(&self) -> Vec<DeviceRow<'_>> {
        self.quick.devices.iter().map(|d| DeviceRow { name: &d.1, look: kolada_ui::quick::Look::of(&d.2), this: d.3 }).collect()
    }

    /// A code's time left ("4:12 left"), while one shows.
    pub(super) fn pair_left(&self) -> String {
        match &self.quick.pair {
            Some(PairState::Showing { expires, .. }) => {
                let s = expires.saturating_sub(crate::quick::now()) / 1000;
                format!("{}:{:02} left", s / 60, s % 60)
            }
            _ => String::new(),
        }
    }

    /// Settings > Devices, while pairing runs here.
    pub(super) fn devices_live(&self) -> Option<super::DevicesLive<'_>> {
        self.quick.net.as_ref()?;
        let said = self.quick.said.as_ref().filter(|(_, at)| at.elapsed() < SAID).map(|(s, _)| s.as_str());
        Some(super::DevicesLive { rows: self.device_rows(), typing: self.quick.typing.as_deref(), naming: self.quick.naming.as_deref(), pair: self.quick.pair.as_ref(), left: self.pair_left(), said, facts: self.quick.syncer.as_ref().map(|s| s.state().facts()).unwrap_or_default() })
    }

    /// Back from the popup's Devices (its arrow, or Esc): a pairing under
    /// way here stops; the beams again, or the popup goes while nothing
    /// is paired.
    fn devices_back(&mut self) {
        self.quick.typing = None;
        self.quick.naming = None;
        if matches!(self.quick.pair, Some(PairState::Opening | PairState::Showing { .. } | PairState::Waiting)) {
            self.with_net(|e, _| e.cancel_pairing());
        }
        if self.quick.paired {
            self.quick.showing = Showing::Main;
            self.quick_draw();
        } else {
            self.hide_quick();
        }
    }

    /// A key while the code, or this device's name, is typed in Settings >
    /// Devices: true if it was theirs (Backspace, Enter pairs or saves,
    /// Esc stops).
    pub(super) fn code_key(&mut self, vk: u16) -> bool {
        if !self.ui.options_open || self.quick.typing.is_none() && self.quick.naming.is_none() || ![VK_BACK.0, VK_RETURN.0, VK_ESCAPE.0].contains(&vk) {
            return false;
        }
        self.quick_key(vk);
        self.request_frame();
        true
    }

    pub(super) fn code_char(&mut self, unit: u16) -> bool {
        if !self.ui.options_open || self.quick.typing.is_none() && self.quick.naming.is_none() {
            return false;
        }
        self.quick_char(unit);
        self.request_frame();
        true
    }

    /// A tap on a text card: it opens (or closes) on its spring.
    fn toggle_open(&mut self, id: kolada_sync::ItemId) {
        match self.quick.opens.iter_mut().find(|o| o.0 == id) {
            Some(o) => o.3 = !o.3,
            None => self.quick.opens.push((id, 0.0, 0.0, true)),
        }
        self.quick.stepped = Some(Instant::now());
        self.start_timer(OPEN_TIMER, 16);
    }

    /// Steps the opening and closing cards' springs to now; the timer
    /// stops once they rest.
    fn step_opens(&mut self) {
        let now = Instant::now();
        let dt = self.quick.stepped.map_or(1.0 / 60.0, |t| now.duration_since(t).as_secs_f32()).min(0.05);
        self.quick.stepped = Some(now);
        for o in &mut self.quick.opens {
            (o.1, o.2) = q::spring(o.1, o.2, if o.3 { 1.0 } else { 0.0 }, dt);
        }
        self.quick.opens.retain(|o| o.3 || o.1 != 0.0 || o.2 != 0.0);
        if self.quick.opens.iter().all(|o| o.1 == if o.3 { 1.0 } else { 0.0 } && o.2 == 0.0) {
            self.stop_timer(OPEN_TIMER);
        }
    }

    /// A picture card, tapped: the picture full size, in the system's
    /// viewer, from a copy among the app's own files (never the vault).
    fn open_picture(&mut self, i: usize) {
        if let Some(id) = self.quick.cards.get(i).map(|c| c.id) {
            self.open_picture_id(id);
        }
    }

    /// A picture, full size in the system's viewer: a card's, or one of the
    /// rest beamed with it.
    fn open_picture_id(&mut self, id: kolada_sync::ItemId) {
        let Some(name) = self.quick.net.as_ref().and_then(|n| n.read(|e| e.cards().iter().find(|c| c.id == id).map(|c| c.text.clone()))) else { return };
        let Some(bytes) = self.quick.net.as_ref().and_then(|n| n.read(|e| e.picture(id))) else {
            return self.say("The picture is still coming.");
        };
        let Some(dir) = kolada_core::app_data_dir().map(|d| d.join("sync").join("open")) else { return };
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join(std::path::Path::new(&name).file_name().unwrap_or(std::ffi::OsStr::new("Picture.png")));
        if std::fs::write(&file, &bytes).is_ok() {
            self.hide_quick();
            share::open_file(&file);
        }
    }

    /// What Beam's side-menu row and window button show; None while Beam
    /// is off.
    pub(super) fn beam_view(&self) -> Option<kolada_ui::scene::BeamView<'_>> {
        let net = self.quick.net.as_ref()?;
        Some(kolada_ui::scene::BeamView {
            unseen: net.read(|e| e.unread()) as u32,
            line: &self.quick.row_line,
            typing: self.quick.typed_at.is_some_and(|t| t.elapsed().as_millis() < TYPING_MS as u128),
            menu: self.quick.menu.as_deref(),
            keys: self.beam_keys().unwrap_or(""),
            check: self.quick.marks.button(Instant::now(), animate()),
        })
    }

    /// The note was clicked in (or `written` in): the Beam button offers it.
    pub(super) fn note_used(&mut self, written: bool) {
        if self.quick.net.is_none() {
            return;
        }
        self.quick.engaged = true;
        if written {
            // Faint while typing; back once the keys rest.
            self.quick.typed_at = Some(Instant::now());
            self.start_timer(TYPING_TIMER, TYPING_MS + 50);
        }
    }

    /// A press anywhere while the Beam button's menu is open closes it:
    /// true if it did (the press is used up), unless it was on the menu.
    pub(super) fn beam_menu_away(&mut self, pressed: Option<Target>) -> bool {
        if self.quick.menu.is_none() || matches!(pressed, Some(Target::Quick(Hit::Fab | Hit::BeamNote | Hit::NewBeam))) {
            return false;
        }
        self.quick.menu = None;
        self.request_frame();
        true
    }

    /// The Beam button: with the note in use and something in it, a menu
    /// (this note, or a new beam); just after launch or with nothing in
    /// it, the popup.
    fn beam_fab(&mut self) {
        if self.quick.menu.take().is_some() {
            return self.request_frame();
        }
        let words = !self.editor.text().trim().is_empty();
        if words && self.quick.engaged {
            let title = self.editor.title();
            self.quick.menu = Some(if title.trim().is_empty() { "This note".into() } else { title });
            self.request_frame();
        } else {
            self.open_quick(false);
        }
    }

    /// Beams the open note, from the Beam button's menu: its Markdown as it
    /// is, under its name, and each picture it embeds that is in the vault,
    /// so the other device saves it as the same note (`Outgoing::Note`).
    pub(super) fn beam_note(&mut self) {
        self.quick.menu = None;
        self.request_frame();
        if !self.quick.paired || !self.quick.intro_seen {
            return self.open_quick(false);
        }
        let text = self.editor.text().trim_end().to_string();
        if text.trim().is_empty() {
            return;
        }
        let vault = PathBuf::from(&self.session.vault);
        let path = vault.join(&self.session.note);
        let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|t| !self.session.note.is_empty() && !t.is_empty()).unwrap_or_else(|| self.editor.title());
        let from = path.parent().filter(|d| d.starts_with(&vault)).map_or_else(|| vault.clone(), std::path::Path::to_path_buf);
        let pictures = kolada_ui::pictures::read_note_pictures(&vault, &from, &text, kolada_sync::engine::PICTURE_MOST)
            .into_iter()
            .map(|f| kolada_sync::NotePicture { name: f.name, mime: f.mime.into(), bytes: f.bytes, width: f.width, height: f.height, thumb: Vec::new() })
            .collect();
        let Some(net) = &self.quick.net else { return };
        match net.with(|e, now| e.send_to(Outgoing::Note { title, text, pictures }, None, now)) {
            Some(id) => {
                let place = if self.quick_open() { q::Place::Card } else { q::Place::Button };
                self.quick.marks.sent(id, place);
                self.quick.scroll = 0.0;
            }
            None => self.say("This note is too long to beam."),
        }
        self.quick_refresh();
        self.quick_draw();
    }

    /// Every clip shown is seen: the count goes.
    fn quick_seen(&mut self) {
        let Some(net) = &self.quick.net else { return };
        let unseen = net.read(|e| e.cards().iter().filter(|c| !c.read && !c.mine).map(|c| c.id).collect::<Vec<_>>());
        if unseen.is_empty() {
            return;
        }
        net.with(|e, _| {
            for id in unseen {
                e.seen(id);
            }
        });
        self.request_frame();
    }

    fn say(&mut self, text: &str) {
        self.quick.said = Some((text.to_string(), Instant::now()));
    }

    /// Reads the engine into what the popup shows.
    pub(super) fn quick_refresh(&mut self) {
        let Some(net) = &self.quick.net else { return };
        let (cards, devices, paired, pair) = net.read(|e| {
            let me = e.device_id();
            let name_of = |id: &kolada_sync::DeviceId| e.devices().iter().find(|d| d.id == *id).map(|d| d.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "a device".into());
            // A note's pictures, and the rest of pictures beamed together,
            // show as part of their card, not as cards of their own.
            let notes: Vec<kolada_sync::ItemId> = e.cards().iter().filter(|c| c.kind == SendKind::Text && !c.title.is_empty()).map(|c| c.id).collect();
            let ids: Vec<kolada_sync::ItemId> = e.cards().iter().map(|c| c.id).collect();
            let all: Vec<&kolada_sync::Card> = e.cards().into_iter().filter(|c| c.picture.as_ref().and_then(|p| p.note).is_none_or(|n| !ids.contains(&n))).collect();
            // One faint line a group, on its first card (the oldest): who and
            // when. A group is one sender's beams, each within 5 minutes of
            // the one before.
            let first = |i: usize| {
                let c = all[i];
                all.get(i + 1).is_none_or(|o| o.mine != c.mine || o.from != c.from || c.at.saturating_sub(o.at) >= GROUP_MS)
            };
            let cards: Vec<Shown> = all
                .iter()
                .enumerate()
                .map(|(i, c)| Shown {
                    id: c.id,
                    text: c.text.clone(),
                    title: c.title.clone(),
                    from: match (first(i), c.mine) {
                        (false, _) => String::new(),
                        (true, true) => format!("You \u{00B7} {}", ago(c.at)),
                        (true, false) => format!("{} \u{00B7} {}", name_of(&c.from), ago(c.at)),
                    },
                    goes: if c.lost { q::LOST.into() } else if c.kind == SendKind::Secret { "Goes once copied".into() } else { q::with_pictures(if notes.contains(&c.id) { e.note_pictures(c.id).len() } else { 0 }) },
                    kind: match c.kind {
                        SendKind::Text => Kind::Text,
                        SendKind::Link => Kind::Link,
                        SendKind::Secret => Kind::Secret,
                        SendKind::Picture => Kind::Picture,
                    },
                    spent: c.kind == SendKind::Secret && c.copied,
                    mine: c.mine,
                    picture: c.picture.as_ref().map(|p| (p.width, p.height, file_size(p.size), e.has_picture(c.id))),
                    rest: if c.kind == SendKind::Picture { e.note_pictures(c.id).into_iter().filter_map(|f| f.picture.as_ref().map(|p| (f.id, (p.width, p.height, file_size(p.size), e.has_picture(f.id))))).collect() } else { Vec::new() },
                })
                .collect();
            let mut devices: Vec<_> = e.devices().iter().map(|d| (d.id, if d.id == me { e.name().to_string() } else { d.name.clone() }, kolada_sync::names::look(&d.kind, if d.id == me { e.model() } else { &d.model }).to_string(), d.id == me)).collect();
            devices.sort_by_key(|d| !d.3);
            (cards, devices, e.paired(), e.pair_state())
        });
        // Thumbnails not decoded yet are, off the UI thread.
        let todo: Vec<(kolada_sync::ItemId, Vec<u8>)> = net.read(|e| {
            e.cards().into_iter().filter_map(|c| c.picture.as_ref().map(|p| (c.id, p.thumb.clone()))).filter(|(id, t)| !t.is_empty() && !self.quick.thumbs.iter().any(|(k, _)| k == id) && !self.quick.decoding.contains(id)).collect()
        });
        for (id, jpeg) in todo {
            self.quick.decoding.push(id);
            let (tx, hwnd) = (self.quick_made(), crate::Handoff(self.hwnd));
            std::thread::spawn(move || {
                let hwnd = hwnd;
                let Some(raster) = crate::images::Recoder::new().and_then(|r| r.decode(&jpeg, THUMB_EDGE)) else { return };
                let _ = tx.send(Made::Card { id, pixels: Arc::new(pixels_of(&raster)) });
                unsafe {
                    let _ = PostMessageW(Some(hwnd.0), crate::quick::WM_APP_SYNC, WPARAM(0), LPARAM(0));
                }
            });
        }
        self.quick.thumbs.retain(|(id, _)| cards.iter().any(|c| c.id == *id || c.rest.iter().any(|(k, _)| k == id)));
        // A beam that didn't arrive shows red, after a restart too.
        let lost = self.quick.net.as_ref().map(|n| n.read(|e| e.cards().iter().filter(|c| c.lost).map(|c| c.id).collect::<Vec<_>>()));
        for id in lost.unwrap_or_default() {
            self.quick.marks.lost(id);
        }
        // The side menu's row: the newest beam not yet seen, else where the
        // last one came from and when.
        let Some(net) = &self.quick.net else { return };
        self.quick.row_line = net.read(|e| {
            let cards = e.cards();
            let first_line = |c: &kolada_sync::Card| match c.kind {
                SendKind::Secret => "A password".to_string(),
                SendKind::Picture => "A picture".to_string(),
                _ => c.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(80).collect(),
            };
            match cards.iter().find(|c| !c.read && !c.mine) {
                Some(c) => first_line(c),
                None => match cards.first() {
                    Some(c) if c.mine => format!("You beamed, {}", ago(c.at)),
                    Some(c) => {
                        let from = e.devices().iter().find(|d| d.id == c.from).map(|d| d.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "a device".into());
                        format!("From {from}, {}", ago(c.at))
                    }
                    None => String::new(),
                },
            }
        });
        self.quick.cards = cards;
        self.quick.devices = devices;
        self.quick.paired = paired;
        self.quick.pair = Some(pair);
        if self.quick.chosen > self.quick.devices.iter().filter(|d| !d.3).count() {
            self.quick.chosen = 0;
        }
        if (self.quick_open() || self.ui.options_open) && matches!(self.quick.pair, Some(PairState::Showing { .. })) {
            self.start_timer(COUNT_TIMER, 1000);
        }
    }

    /// Where work made off the UI thread is sent.
    fn quick_made(&mut self) -> std::sync::mpsc::Sender<Made> {
        self.quick.made.get_or_insert_with(std::sync::mpsc::channel).0.clone()
    }

    /// Takes what was made off the UI thread since the last look.
    fn quick_thumbs(&mut self) {
        let Some((_, rx)) = &self.quick.made else { return };
        let made: Vec<Made> = rx.try_iter().collect();
        for m in made {
            match m {
                Made::Card { id, pixels } => {
                    self.quick.decoding.retain(|k| *k != id);
                    self.quick.thumbs.push((id, pixels));
                }
                Made::Staged { n, jpeg, pixels, width, height, file } => {
                    if let Some(pic) = self.quick.ready.as_mut().and_then(|r| r.picture.iter_mut().chain(r.rest.iter_mut().map(|(_, p)| p)).find(|p| p.n == n)) {
                        if let Some(file) = file {
                            pic.size = file_size(file.len() as u64);
                            pic.bytes = file;
                        }
                        pic.thumb = Some((jpeg, pixels, width, height));
                        if std::mem::take(&mut self.quick.beam_waiting) && !pic.bytes.is_empty() {
                            let _ = self.quick_window();
                            self.beam();
                        }
                    }
                }
            }
        }
    }

    pub(super) fn quick_open(&self) -> bool {
        self.quick.window.is_some_and(|h| unsafe { IsWindowVisible(h) }.as_bool())
    }

    // The window.

    fn quick_window(&mut self) -> Option<HWND> {
        if let Some(h) = self.quick.window {
            return Some(h);
        }
        unsafe {
            let instance = GetModuleHandleW(None).ok()?;
            let class = w!("Kolada.QuickSend");
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_DROPSHADOW,
                lpfnWndProc: Some(quick_proc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            let h = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, class, w!("Beam"), WS_POPUP, 0, 0, 400, 600, None, None, Some(instance.into()), None).ok()?;
            let round = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as *const _, size_of_val(&round) as u32);
            // Pictures dropped on it are beamed.
            windows::Win32::UI::Shell::DragAcceptFiles(h, true);
            self.quick.window = Some(h);
            Some(h)
        }
    }

    /// Places the popup above the clock on the screen with the pointer, at
    /// `height` DIPs tall.
    fn quick_place(&mut self, h: HWND, height: f32) {
        unsafe {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let work: RECT = info.rcWork;
            // That screen's scale, not the one the popup was last on.
            let (mut dx, mut dy) = (0u32, 0u32);
            let scale = match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) {
                Ok(()) if dx >= 96 => dx as f32 / 96.0,
                _ => window::dpi(h) as f32 / 96.0,
            };
            let (w, hh, edge) = ((q::W * scale) as i32, (height * scale) as i32, (EDGE * scale) as i32);
            let _ = SetWindowPos(h, None, work.right - edge - w, work.bottom - edge - hh, w, hh, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    /// Opens the popup: the first use's steps the first time, the devices
    /// while none is paired, else the clips; with `stage`, what is on the
    /// clipboard, ready to beam.
    pub(super) fn open_quick(&mut self, stage: bool) {
        if self.quick.net.is_none() {
            return;
        }
        self.quick_refresh();
        // Nothing paired yet (and the first use seen): pairing, here.
        self.quick.showing = if !self.quick.intro_seen {
            Showing::FirstUse
        } else if !self.quick.paired {
            Showing::Devices
        } else {
            Showing::Main
        };
        if stage {
            self.stage_clipboard();
        }
        self.quick.scroll = 0.0;
        if self.quick.showing == Showing::Main {
            self.quick_seen();
        }
        let Some(h) = self.quick_window() else { return };
        self.quick_place(h, q::H);
        unsafe {
            let _ = ShowWindow(h, SW_SHOW);
            let _ = SetForegroundWindow(h);
        }
        self.quick_draw();
    }

    fn hide_quick(&mut self) {
        if let Some(h) = self.quick.window {
            unsafe {
                let _ = ShowWindow(h, SW_HIDE);
            }
        }
        self.stop_timer(HIDE_TIMER);
        self.stop_timer(COUNT_TIMER);
        // Put away unbeamed, what was copied is let go: the keys read the
        // clipboard again next time.
        self.quick.ready = None;
        self.quick.typing = None;
        self.quick.naming = None;
        self.quick.hover = None;
    }

    /// What is on the clipboard, ready to beam: a picture (a screenshot),
    /// or text.
    fn stage_clipboard(&mut self) -> bool {
        // Picture files copied in Explorer: all of them, together.
        let files: Vec<PathBuf> = crate::clipboard::get_files(self.hwnd).into_iter().filter(|p| picture_type(p).is_some()).collect();
        if !files.is_empty() {
            self.quick.ready = None;
            self.add_files(&files);
            return self.quick.ready.is_some();
        }
        if let Some(picture) = crate::clipboard::get_picture(self.hwnd) {
            let source = match picture {
                crate::clipboard::Picture::Png(bytes) => Source::File(bytes, "image/png".into()),
                crate::clipboard::Picture::Pixels { width, height, bgra } => Source::Bitmap(width, height, bgra),
            };
            return self.stage_picture(source, "Picture".into());
        }
        match crate::clipboard::get(self.hwnd).filter(|t| !t.trim().is_empty()) {
            Some(text) => {
                let secrecy = kolada_sync::judge_secret(&text, crate::clipboard::concealed());
                self.quick.ready = Some(Ready { kind: kind_of(&text, secrecy.masked()), text, picture: None, rest: Vec::new(), unsure: secrecy == kolada_sync::Secrecy::Unsure });
                true
            }
            None => {
                self.say("The clipboard has nothing to beam.");
                false
            }
        }
    }

    /// A picture ready to beam; its thumbnail (and for a bitmap, its PNG) is
    /// made off the UI thread. `name`: its file's name without extension.
    fn stage_picture(&mut self, source: Source, name: String) -> bool {
        self.quick.ready = None;
        self.add_picture(source, name)
    }

    /// One more picture ready to beam, beside those already (`Ready::rest`),
    /// within what goes at once (`kolada_sync::pictures_over`).
    fn add_picture(&mut self, source: Source, name: String) -> bool {
        let (bytes, mime) = match &source {
            Source::File(bytes, mime) => (bytes.clone(), mime.clone()),
            Source::Bitmap(..) => (Vec::new(), "image/png".to_string()),
        };
        if bytes.len() as u64 > PICTURE_MOST {
            self.say("That picture is too big to beam: 20 MB at most.");
            return false;
        }
        let (count, held) = self.quick.ready.as_ref().filter(|r| r.picture.is_some()).map_or((0, 0), |r| (1 + r.rest.len(), r.picture.iter().chain(r.rest.iter().map(|(_, p)| p)).map(|p| p.bytes.len()).sum()));
        if let Some(why) = kolada_sync::pictures_over(count + 1, held + bytes.len()) {
            self.say(&why);
            return false;
        }
        self.quick.stagings += 1;
        self.quick.beam_waiting = false;
        let n = self.quick.stagings;
        let size = if bytes.is_empty() { String::new() } else { file_size(bytes.len() as u64) };
        let file = format!("{name}.{}", picture_ext(&mime));
        let (tx, hwnd) = (self.quick_made(), crate::Handoff(self.hwnd));
        std::thread::spawn(move || {
            let hwnd = hwnd;
            let Some(recode) = crate::images::Recoder::new() else { return };
            let (raster, file) = match source {
                Source::File(bytes, _) => (recode.decode(&bytes, THUMB_EDGE), None),
                Source::Bitmap(width, height, bgra) => {
                    let full = kolada_core::share::Raster { width, height, bgra };
                    let png = recode.encode(&full, 100, true);
                    (png.as_ref().and_then(|p| recode.decode(p, THUMB_EDGE)), png)
                }
            };
            let Some(raster) = raster else { return };
            let jpeg = recode.encode(&raster, 72, false).filter(|j| j.len() <= kolada_sync::engine::THUMB_MOST).unwrap_or_default();
            let _ = tx.send(Made::Staged { n, jpeg, pixels: Arc::new(pixels_of(&raster)), width: raster.width, height: raster.height, file });
            unsafe {
                let _ = PostMessageW(Some(hwnd.0), crate::quick::WM_APP_SYNC, WPARAM(0), LPARAM(0));
            }
        });
        let pic = ReadyPicture { bytes, mime, size, thumb: None, n };
        match self.quick.ready.as_mut().filter(|r| r.picture.is_some()) {
            Some(r) => r.rest.push((file, pic)),
            None => self.quick.ready = Some(Ready { text: file, kind: Kind::Picture, picture: Some(pic), rest: Vec::new(), unsure: false }),
        }
        true
    }

    /// One of the pictures ready to beam goes (0 is the first); the rest stay.
    fn unstage(&mut self, k: usize) {
        let Some(r) = self.quick.ready.as_mut() else { return };
        if k > 0 {
            if k <= r.rest.len() {
                r.rest.remove(k - 1);
            }
        } else if r.rest.is_empty() {
            self.quick.ready = None;
            self.quick.beam_waiting = false;
        } else {
            let (text, pic) = r.rest.remove(0);
            r.text = text;
            r.picture = Some(pic);
        }
    }

    /// Pictures picked, dropped on the popup or pasted as files: all of
    /// them ready to beam together, beside any already ready (up to what
    /// goes at once). Only pictures go, for now.
    pub(super) fn quick_attach(&mut self, paths: &[PathBuf]) {
        if self.quick.net.is_none() || !self.quick.paired {
            return;
        }
        if !self.quick_open() || self.quick.showing != Showing::Main {
            self.open_quick(false);
        }
        // What was ready that isn't a picture gives way to them.
        if self.quick.ready.as_ref().is_some_and(|r| r.picture.is_none()) {
            self.quick.ready = None;
        }
        self.add_files(paths);
        self.quick_draw();
    }

    /// Each picture of `paths` ready to beam, beside those already, up to
    /// what goes at once; it says what can't go.
    fn add_files(&mut self, paths: &[PathBuf]) {
        let pictures: Vec<(&PathBuf, &str)> = paths.iter().filter_map(|p| picture_type(p).map(|m| (p, m))).collect();
        if pictures.is_empty() && !paths.is_empty() {
            self.say("Only pictures can be beamed for now.");
        }
        for (path, mime) in pictures {
            if std::fs::metadata(path).is_ok_and(|m| m.len() > PICTURE_MOST) {
                self.say("That picture is too big to beam: 20 MB at most.");
                continue;
            }
            match std::fs::read(path) {
                Ok(bytes) => {
                    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Picture".into());
                    if !self.add_picture(Source::File(bytes, mime.into()), name) {
                        break;
                    }
                }
                Err(_) => self.say("That file couldn't be read."),
            }
        }
    }

    /// The file picker, owned by the popup, which stays up meanwhile.
    fn quick_pick(&mut self) {
        let Some(h) = self.quick.window else { return };
        self.quick.picking = true;
        let picked = crate::dialog::open_files(h, "Beam a picture", &[("Pictures", "*.png;*.jpg;*.jpeg;*.gif;*.webp;*.bmp;*.heic;*.tif;*.tiff")]);
        self.quick.picking = false;
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        self.quick_attach(&picked);
    }

    /// A save dialog for a picture card's file.
    fn quick_save_file(&mut self, i: usize) {
        let Some(c) = self.quick.cards.get(i) else { return };
        let (id, name) = (c.id, c.text.clone());
        let Some(bytes) = self.quick.net.as_ref().and_then(|n| n.read(|e| e.picture(id))) else {
            return self.say("The picture is still coming.");
        };
        let Some(h) = self.quick.window else { return };
        let ext = std::path::Path::new(&name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into());
        self.quick.picking = true;
        let to = crate::dialog::save_file(h, "Save the picture", &name, ("Picture", &format!("*.{ext}")), &ext);
        self.quick.picking = false;
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        if let Some(path) = to
            && std::fs::write(&path, &bytes).is_err()
        {
            self.say("The picture couldn't be saved there.");
        }
    }

    /// Beams what is ready to whoever is chosen. The popup stays (or none
    /// shows): the beam is the timeline's newest, pending until the server
    /// has it, then its check shows a moment (by the window's Beam button
    /// where no popup shows).
    fn beam(&mut self) {
        let Some(ready) = self.quick.ready.take() else { return };
        let Some(net) = &self.quick.net else { return };
        let others: Vec<_> = self.quick.devices.iter().filter(|d| !d.3).collect();
        let to = match self.quick.chosen {
            0 => None,
            i => others.get(i - 1).map(|d| vec![d.0]),
        };
        let as_note = |name: String, pic: ReadyPicture| {
            let (thumb, width, height) = pic.thumb.map(|(j, _, w, h)| (j, w, h)).unwrap_or_default();
            kolada_sync::NotePicture { name, mime: pic.mime, bytes: pic.bytes, width, height, thumb }
        };
        let out = match (ready.kind, ready.picture) {
            (_, Some(pic)) if pic.bytes.is_empty() => {
                // A bitmap still being made a PNG: it goes once it is one.
                self.quick.ready = Some(Ready { text: ready.text, kind: Kind::Picture, picture: Some(pic), rest: ready.rest, unsure: false });
                self.quick.beam_waiting = true;
                return;
            }
            (_, Some(pic)) if !ready.rest.is_empty() => Outgoing::Pictures(std::iter::once(as_note(ready.text, pic)).chain(ready.rest.into_iter().map(|(n, p)| as_note(n, p))).collect()),
            (_, Some(pic)) => {
                let (thumb, width, height) = pic.thumb.map(|(j, _, w, h)| (j, w, h)).unwrap_or_default();
                Outgoing::Picture { bytes: pic.bytes, mime: pic.mime, name: ready.text, width, height, thumb }
            }
            (Kind::Secret, None) => Outgoing::Secret(ready.text),
            _ => Outgoing::Text(ready.text),
        };
        let draft = self.quick.ready.is_none() && self.quick.beaming_draft;
        match net.with(|e, now| e.send_to(out, to.as_deref(), now)) {
            Some(id) => {
                let place = if self.quick_open() { q::Place::Card } else { q::Place::Button };
                self.quick.marks.sent(id, place);
                // The newest, at the foot.
                self.quick.scroll = 0.0;
                if draft {
                    self.quick.draft.clear();
                }
            }
            None => self.say("Too long to beam: a few pages at most."),
        }
        self.quick.beaming_draft = false;
        self.quick_refresh();
        self.quick_draw();
    }

    /// A failed or lost beam's red mark: it goes again, as a new beam in
    /// its place, to every other device (a picture from the copy kept here).
    fn quick_retry(&mut self, i: usize) {
        let Some(c) = self.quick.cards.get(i).filter(|c| c.mine) else { return };
        let id = c.id;
        let Some(net) = &self.quick.net else { return };
        let again = net.with(|e, now| e.resend(id, now));
        self.quick.marks.forget(id);
        if let Some(again) = again {
            let place = if self.quick_open() { q::Place::Card } else { q::Place::Button };
            self.quick.marks.sent(again, place);
            self.quick.scroll = 0.0;
        }
        self.quick_refresh();
        self.quick_draw();
    }

    /// The marks' clock: the cards' springs and the checks step on one
    /// timer, kept while anything moves; a check by the window's button
    /// needs its frames too.
    fn marks_moving(&mut self) {
        let now = Instant::now();
        if self.quick.marks.moving(now) {
            self.start_timer(OPEN_TIMER, 16);
        }
        if self.quick.marks.button(now, true).is_some() {
            self.request_frame();
        }
    }

    /// Beams what is typed in the send area.
    fn beam_draft(&mut self) {
        let text = self.quick.draft.trim_end().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.quick.ready = Some(Ready { kind: kind_of(&text, false), text, picture: None, rest: Vec::new(), unsure: false });
        self.quick.beaming_draft = true;
        self.beam();
    }

    /// The keys: what was copied, shown first (or beamed at once, as Settings
    /// says). `secret`: the password keys.
    pub(super) fn quick_keys(&mut self, secret: bool) {
        self.quick.pressed = Some(Instant::now());
        if !self.quick.paired || !self.quick.intro_seen {
            return self.open_quick(false);
        }
        if self.session.quick_beam == 1 {
            if self.stage_clipboard() {
                if secret && let Some(r) = &mut self.quick.ready {
                    r.kind = Kind::Secret;
                    r.unsure = false;
                }
                // Maybe a password: shown, for "Not a password", not beamed.
                if !self.quick.ready.as_ref().is_some_and(|r| r.unsure) {
                    self.quick.chosen = 0;
                    self.beam();
                    return;
                }
            }
            self.open_quick(false);
            return;
        }
        self.open_quick(true);
        if secret && let Some(r) = &mut self.quick.ready {
            r.kind = Kind::Secret;
            r.unsure = false;
        }
        self.quick_draw();
    }

    // Drawing.

    pub(super) fn quick_draw(&mut self) {
        let Some(h) = self.quick.window else { return };
        if !unsafe { IsWindowVisible(h) }.as_bool() {
            return;
        }
        self.quick_render(h);
        // Key to visible, for test instances: appended to a file beside
        // their data (Quick Send's budget is under 50 ms).
        if let Some(t) = self.quick.pressed.take()
            && kolada_core::testing()
            && let Some(dir) = kolada_core::app_data_dir()
        {
            use std::io::Write;
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("quick-open-ms.txt")) {
                let _ = writeln!(f, "{:.2}", t.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }

    /// Draws the popup's frame into its surface, and onto the window.
    fn quick_render(&mut self, h: HWND) {
        self.quick_thumbs();
        let view = window::view(h);
        let scale = view.2 as f32 / 96.0;
        let (wd, hd) = (view.0 as f32 / scale, view.1 as f32 / scale);
        let fonts = self.quick.fonts.get_or_insert_with(|| Fonts::new(1.0, 0)).clone();
        let said = self.quick.said.as_ref().filter(|(_, at)| at.elapsed() < SAID).map(|(s, _)| s.clone());
        let now = Instant::now();
        let copied = self.quick.copied.filter(|(_, at)| now.duration_since(*at) < COPIED).map(|(id, _)| id);
        let thumb_of = |id: &kolada_sync::ItemId| self.quick.thumbs.iter().find(|(k, _)| k == id).map(|(_, p)| p.clone());
        let rests: Vec<Vec<PicView>> = self.quick.cards.iter().map(|c| c.rest.iter().map(|(id, (w, h, size, ready))| PicView { pixels: thumb_of(id), width: *w, height: *h, size, ready: *ready }).collect()).collect();
        let staged_rest: Vec<PicView> = self.quick.ready.iter().flat_map(|r| r.rest.iter()).map(|(_, p)| PicView { pixels: p.thumb.as_ref().map(|t| t.1.clone()), width: p.thumb.as_ref().map_or(4, |t| t.2), height: p.thumb.as_ref().map_or(3, |t| t.3), size: &p.size, ready: !p.bytes.is_empty() }).collect();
        let clips: Vec<ClipView> = self
            .quick
            .cards
            .iter()
            .zip(&rests)
            .map(|(c, rest)| ClipView {
                sent: self.quick.marks.card(c.id, Instant::now(), animate()),
                from: &c.from,
                goes: &c.goes,
                kind: c.kind,
                text: &c.text,
                title: &c.title,
                copied: copied == Some(c.id),
                spent: c.spent,
                mine: c.mine,
                picture: c.picture.as_ref().map(|(w, h, size, ready)| PicView { pixels: thumb_of(&c.id), width: *w, height: *h, size, ready: *ready }),
                more: rest,
                open: self.quick.opens.iter().find(|o| o.0 == c.id).map_or(0.0, |o| o.1),
            })
            .collect();
        let others: Vec<&str> = self.quick.devices.iter().filter(|d| !d.3).map(|d| d.1.as_str()).collect();
        let mut to = vec!["All devices"];
        to.extend(others.iter().copied());
        let rows = self.device_rows();
        let left = self.pair_left();
        let facts = self.quick.syncer.as_ref().map(|s| s.state().facts()).unwrap_or_default();
        let what = |k: Kind| match k {
            Kind::Link => "a link",
            Kind::Secret => "a password",
            Kind::Text => "text",
            Kind::Picture => "a picture",
        };
        let saved = self.quick.saved.as_ref().filter(|(_, _, at)| at.elapsed() < SAID).map(|(n, _, _)| n.as_str());
        let screen = match self.quick.showing {
            Showing::FirstUse => Screen::FirstUse,
            Showing::Devices => Screen::Devices { devices: &rows, pairing: pairing_of(self.quick.typing.as_deref(), self.quick.pair.as_ref(), &left), facts: &facts, naming: self.quick.naming.as_deref() },
            Showing::Main => Screen::Main {
                staged: self.quick.ready.as_ref().map(|r| Staged {
                    what: what(r.kind),
                    kind: r.kind,
                    text: &r.text,
                    picture: r.picture.as_ref().map(|p| PicView { pixels: p.thumb.as_ref().map(|t| t.1.clone()), width: p.thumb.as_ref().map_or(4, |t| t.2), height: p.thumb.as_ref().map_or(3, |t| t.3), size: &p.size, ready: !p.bytes.is_empty() }),
                    more: &staged_rest,
                    unsure: r.unsure,
                }),
                to: &to,
                chosen: self.quick.chosen,
                clips: &clips,
                scroll: self.quick.scroll,
                saved,
                draft: &self.quick.draft,
                more: self.quick.more,
            },
        };
        let devices = self.quick.devices.iter().filter(|d| !d.3).count();
        let keys = self.beam_keys().unwrap_or("");
        let popup = PopupView { screen, devices, said: said.as_deref(), host: Host { keys, ..Host::WINDOWS } };
        let Ok(scene) = q::popup(&self.eng, &fonts, &popup, self.quick.hover, wd, hd) else { return };
        let Ok(list) = display::record(&self.eng, &scene, theme::for_mode(self.session.dark), scale) else { return };
        let surface = self.quick.surface.get_or_insert_with(|| Surface::new(view));
        surface.draw(view, &list);
        crate::cpu::blit_changed(surface, h);
        self.quick.hits = scene.hits;
        self.quick.scroll_max = scene.quick_scroll_max;
        self.quick.scroll = self.quick.scroll.min(self.quick.scroll_max);
    }

    fn quick_hit(&self, x: i32, y: i32) -> Option<Hit> {
        let h = self.quick.window?;
        let scale = window::dpi(h) as f32 / 96.0;
        let (x, y) = (x as f32 / scale, y as f32 / scale);
        let at = |r: &kolada_ui::geom::Rect| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
        self.quick.hits.iter().rev().find_map(|(r, t)| match t {
            Target::Quick(hit) if at(r) && !matches!(hit, Hit::Popup | Hit::List) => Some(*hit),
            _ => None,
        })
    }

    fn quick_move(&mut self, x: i32, y: i32) {
        let hover = self.quick_hit(x, y).map(Target::Quick);
        if hover != self.quick.hover {
            self.quick.hover = hover;
            self.quick_draw();
        }
    }

    fn quick_wheel(&mut self, notches: f32) {
        if self.quick.showing == Showing::Main {
            // The newest beams are at the foot: the wheel up goes back in time.
            self.quick.scroll = (self.quick.scroll + notches * WHEEL_STEP).clamp(0.0, self.quick.scroll_max);
            self.quick_draw();
        }
    }

    /// A click in the popup, or Settings' Devices button.
    pub(super) fn quick_click(&mut self, hit: Hit) {
        if self.quick.net.is_none() {
            return;
        }
        // Any press but the "…" itself puts its menu away.
        if !matches!(hit, Hit::More(_)) {
            self.quick.more = None;
        }
        match hit {
            Hit::Card(i) => match self.quick.cards.get(i).map(|c| (c.id, c.kind)) {
                Some((id, Kind::Text)) => self.toggle_open(id),
                Some((_, Kind::Link)) => return self.quick_click(Hit::Open(i)),
                Some((_, Kind::Picture)) => self.open_picture(i),
                _ => {}
            },
            Hit::More(i) => self.quick.more = if self.quick.more == Some(i) { None } else { Some(i) },
            Hit::Retry(i) => self.quick_retry(i),
            Hit::ReadMore(i) => {
                if let Some(text) = self.quick.cards.get(i).map(|c| c.text.clone()) {
                    self.hide_quick();
                    self.bring_up();
                    self.open_unwritten(text);
                    return;
                }
            }
            Hit::Popup | Hit::List | Hit::CodeField | Hit::NameField => {}
            Hit::Show | Hit::NewBeam => {
                self.quick.menu = None;
                return self.open_quick(false);
            }
            Hit::Fab => return self.beam_fab(),
            Hit::BeamNote => return self.beam_note(),
            Hit::Close => self.hide_quick(),
            Hit::Devices => self.quick.showing = Showing::Devices,
            Hit::Back => return self.devices_back(),
            Hit::Field => {}
            Hit::Attach => return self.quick_pick(),
            Hit::Beam if self.quick.ready.is_none() => return self.beam_draft(),
            Hit::Beam => return self.beam(),
            Hit::Unstage => {
                self.quick.ready = None;
                self.quick.beam_waiting = false;
            }
            // "Not a password": it beams as the text (or link) it is;
            // nothing about it is remembered.
            Hit::NotSecret => {
                if let Some(r) = self.quick.ready.as_mut().filter(|r| r.unsure && r.kind == Kind::Secret) {
                    r.kind = kind_of(&r.text, false);
                    r.unsure = false;
                }
            }
            Hit::UnstageAt(k) => self.unstage(k),
            Hit::Tile(i, k) => {
                let id = if k == 0 { self.quick.cards.get(i).map(|c| c.id) } else { self.quick.cards.get(i).and_then(|c| c.rest.get(k - 1)).map(|(id, _)| *id) };
                if let Some(id) = id {
                    self.open_picture_id(id);
                }
            }
            Hit::To(i) => self.quick.chosen = i,
            Hit::Copy(i) => self.copy_clip(i),
            Hit::Open(i) => {
                if let Some(url) = self.quick.cards.get(i).map(|c| c.text.trim().to_string()) {
                    share::open_address(&url);
                    self.hide_quick();
                    return;
                }
            }
            Hit::SaveNote(i) => self.save_clip(i),
            Hit::SaveFile(i) if self.quick.cards.get(i).is_some_and(|c| !c.rest.is_empty()) => self.quick_save_all(i),
            Hit::SaveFile(i) => self.quick_save_file(i),
            Hit::OpenNote => {
                if let Some((_, path, _)) = self.quick.saved.take() {
                    self.hide_quick();
                    self.bring_up();
                    self.open(&path);
                    return;
                }
            }
            Hit::PairNow | Hit::NotNow => {
                self.quick.intro_seen = true;
                if let Some(p) = intro_file() {
                    let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
                    let _ = std::fs::write(p, b"1");
                }
                if hit == Hit::PairNow {
                    self.quick.showing = Showing::Devices;
                } else if self.quick.paired {
                    self.quick.showing = Showing::Main;
                } else {
                    return self.hide_quick();
                }
            }
            Hit::ShowCode => self.with_net(|e, _| e.show_code()),
            Hit::EnterCode => self.quick.typing = Some(String::new()),
            Hit::Pair => self.quick_pair(),
            Hit::Cancel => {
                self.quick.typing = None;
                self.with_net(|e, _| e.cancel_pairing());
            }
            Hit::Allow | Hit::Deny => self.with_net(|e, now| e.allow(hit == Hit::Allow, now)),
            Hit::Forget(i) => {
                if let Some(id) = self.quick.cards.get(i).map(|c| c.id) {
                    self.with_net(|e, _| {
                        e.forget(id);
                    });
                }
            }
            Hit::Remove(i) => {
                if let Some((id, ..)) = self.quick.devices.get(i).cloned() {
                    self.with_net(|e, _| e.remove(id));
                }
            }
            Hit::Rename => {
                self.quick.typing = None;
                self.quick.naming = self.quick.devices.iter().find(|d| d.3).map(|d| d.1.clone());
            }
            Hit::NameSave => self.save_name(),
            Hit::NameCancel => self.quick.naming = None,
        }
        self.quick_refresh();
        self.quick_draw();
    }

    fn with_net(&self, f: impl FnOnce(&mut kolada_sync::Engine, u64)) {
        if let Some(net) = &self.quick.net {
            net.with(f);
        }
    }

    /// A card's pictures beamed together, each its name and bytes, in
    /// order; None (and it says so) while any is still coming.
    fn group_files(&mut self, i: usize) -> Option<Vec<(String, Vec<u8>)>> {
        let c = self.quick.cards.get(i)?;
        let ids: Vec<kolada_sync::ItemId> = std::iter::once(c.id).chain(c.rest.iter().map(|(id, _)| *id)).collect();
        let files: Option<Vec<(String, Vec<u8>)>> = self.quick.net.as_ref()?.read(|e| ids.iter().map(|id| Some((e.cards().iter().find(|k| k.id == *id)?.text.clone(), e.picture(*id)?))).collect());
        if files.is_none() {
            self.say("The pictures are still coming.");
        }
        files
    }

    /// "Save all…": a folder, and each picture there, never over a file.
    fn quick_save_all(&mut self, i: usize) {
        let Some(files) = self.group_files(i) else { return };
        let Some(h) = self.quick.window else { return };
        self.quick.picking = true;
        let to = crate::dialog::pick_folder(h, "Save the pictures");
        self.quick.picking = false;
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        if let Some(dir) = to
            && kolada_core::files::save_all(&dir, &files).is_err()
        {
            self.say("The pictures couldn't be saved there.");
        }
    }

    fn copy_clip(&mut self, i: usize) {
        // Several pictures: as files, which Explorer and chat apps paste whole.
        if self.quick.cards.get(i).is_some_and(|c| !c.rest.is_empty()) {
            let id = self.quick.cards[i].id;
            let Some(files) = self.group_files(i) else { return };
            let Some(dir) = kolada_core::app_data_dir().map(|d| d.join("sync").join("copied")) else { return };
            match kolada_core::files::copies(&dir, &files) {
                Ok(paths) if crate::clipboard::set_files(self.hwnd, &paths) => {
                    self.quick.copied = Some((id, Instant::now()));
                    self.start_timer(HIDE_TIMER + 1, COPIED.as_millis() as u32 + 50);
                }
                _ => self.say("The pictures couldn't be copied."),
            }
            return;
        }
        let Some(c) = self.quick.cards.get(i) else { return };
        let (id, secret, picture) = (c.id, c.kind == Kind::Secret, c.kind == Kind::Picture);
        let Some(net) = &self.quick.net else { return };
        if picture {
            let Some(bytes) = net.read(|e| e.picture(id)) else { return self.say("The picture is still coming.") };
            let decoded = crate::images::Recoder::new().and_then(|r| r.decode(&bytes, u32::MAX));
            let png = bytes.starts_with(b"\x89PNG").then_some(&bytes[..]);
            if decoded.is_some_and(|r| crate::clipboard::set_picture(self.hwnd, r.width, r.height, &r.bgra, png)) {
                self.quick.copied = Some((id, Instant::now()));
                self.start_timer(HIDE_TIMER + 1, COPIED.as_millis() as u32 + 50);
            } else {
                self.say("The picture couldn't be copied.");
            }
            return;
        }
        if let Some(text) = net.with(|e, _| e.copy(id)) {
            if secret {
                crate::clipboard::set_secret(self.hwnd, &text);
                self.quick.on_clipboard = Some(text);
                self.start_timer(CLEAR_TIMER, CLEAR_MS);
            } else {
                crate::clipboard::set(self.hwnd, &text);
            }
            self.quick.copied = Some((id, Instant::now()));
            // Back to plain Copy once "Copied" has shown.
            self.start_timer(HIDE_TIMER + 1, COPIED.as_millis() as u32 + 50);
        }
    }

    /// Saves a clip as a note in the vault; it leaves Quick Send.
    fn save_clip(&mut self, i: usize) {
        let Some(c) = self.quick.cards.get(i) else { return };
        if c.kind == Kind::Secret || self.session.vault.is_empty() {
            return;
        }
        let id = c.id;
        if c.kind == Kind::Picture {
            let file = c.text.clone();
            return self.save_picture(id, &file);
        }
        // A note: saved under its name, with the pictures that came with it.
        if c.kind == Kind::Text && !c.title.is_empty() {
            let (title, text) = (c.title.clone(), c.text.clone());
            return self.save_beamed_note(id, &title, &text);
        }
        let (title, text) = match c.kind {
            Kind::Link if !c.title.is_empty() => (c.title.clone(), format!("{}\n\n{}\n", c.title, c.text.trim())),
            _ => (c.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("Beam").trim().chars().take(60).collect::<String>(), c.text.clone()),
        };
        let dir = PathBuf::from(&self.session.vault);
        let stem = kolada_core::note::file_stem_for(&title);
        let path = kolada_core::note::unique_path(&dir, &stem, None);
        if kolada_core::note::save(&path, &text, kolada_core::note::Format::default()).is_err() {
            self.say("That couldn't be saved as a note.");
            return;
        }
        self.with_net(|e, _| {
            e.forget(id);
        });
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.quick.saved = Some((name, path, Instant::now()));
    }

    /// A beamed note saved in the vault: its Markdown as it came, under its
    /// name, and each picture that came with it at the place the note
    /// embeds it from (one already there is kept), so it reads the same.
    fn save_beamed_note(&mut self, id: kolada_sync::ItemId, title: &str, text: &str) {
        let Some(net) = &self.quick.net else { return };
        let pictures: Vec<(kolada_sync::ItemId, String, Option<Vec<u8>>)> = net.read(|e| e.note_pictures(id).into_iter().map(|c| (c.id, c.text.clone(), e.picture(c.id))).collect());
        if pictures.iter().any(|(_, _, b)| b.is_none()) {
            return self.say("The note's pictures are still coming.");
        }
        let dir = PathBuf::from(&self.session.vault);
        let path = kolada_core::note::unique_path(&dir, &kolada_core::note::file_stem_for(title), None);
        if kolada_core::note::save(&path, text, kolada_core::note::Format::default()).is_err() {
            return self.say("That couldn't be saved as a note.");
        }
        let beside = path.parent().map_or_else(|| dir.clone(), std::path::Path::to_path_buf);
        for (_, name, bytes) in &pictures {
            let (Some(to), Some(bytes)) = (kolada_ui::pictures::note_picture_path(&beside, name), bytes) else { continue };
            if !to.exists() {
                if let Some(parent) = to.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&to, bytes);
            }
        }
        self.with_net(|e, _| {
            e.forget(id);
            for (p, _, _) in &pictures {
                e.forget(*p);
            }
        });
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.quick.saved = Some((name, path, Instant::now()));
    }

    /// A picture saved as a note: the picture in the vault, and a note
    /// beside it that shows it (`![[name]]`, as Obsidian has it).
    fn save_picture(&mut self, id: kolada_sync::ItemId, file: &str) {
        let Some(bytes) = self.quick.net.as_ref().and_then(|n| n.read(|e| e.picture(id))) else {
            return self.say("The picture is still coming.");
        };
        let dir = PathBuf::from(&self.session.vault);
        let path = std::path::Path::new(file);
        let stem = kolada_core::note::file_stem_for(&path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Picture".into()));
        let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into());
        let picture = (1..).map(|n| dir.join(if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} {n}.{ext}") })).find(|p| !p.exists()).unwrap_or_default();
        let note = kolada_core::note::unique_path(&dir, &stem, None);
        let shown = picture.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if std::fs::write(&picture, &bytes).is_err() || kolada_core::note::save(&note, &format!("![[{shown}]]\n"), kolada_core::note::Format::default()).is_err() {
            return self.say("That couldn't be saved as a note.");
        }
        self.with_net(|e, _| {
            e.forget(id);
        });
        let name = note.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.quick.saved = Some((name, note, Instant::now()));
    }

    /// The name typed for this device: kept, sealed and sent to the group
    /// (an empty one goes back to the computer's).
    fn save_name(&mut self) {
        let Some(name) = self.quick.naming.take() else { return };
        self.with_net(|e, _| e.rename_self(&name));
    }

    fn quick_pair(&mut self) {
        let Some(typed) = self.quick.typing.clone() else { return };
        let Some(net) = &self.quick.net else { return };
        match net.with(|e, _| e.enter_code(&typed)) {
            Ok(()) => self.quick.typing = None,
            Err(_) => self.say("A code is 8 letters and digits, like F4FS-2RSA."),
        }
    }

    fn quick_char(&mut self, unit: u16) {
        let Some(c) = char::from_u32(unit as u32) else { return };
        if let Some(t) = &mut self.quick.naming {
            if !c.is_control() && t.chars().count() < kolada_sync::names::NAME_MOST {
                t.push(c);
                self.quick_draw();
            }
            return;
        }
        // Into the send area, when it shows and nothing else is ready.
        if self.quick.typing.is_none() {
            if self.quick.showing == Showing::Main && self.quick.ready.is_none() && !c.is_control() {
                self.quick.draft.push(c);
                self.quick_draw();
            }
            return;
        }
        if let Some(t) = &mut self.quick.typing {
            let count = t.chars().filter(|c| *c != '-').count();
            if let Some(c) = kolada_sync::code::normal(c)
                && count < 8
            {
                if count == 4 {
                    t.push('-');
                }
                t.push(c);
                self.quick_draw();
            }
        }
    }

    fn quick_key(&mut self, vk: u16) {
        let ctrl = key_down(VK_CONTROL.0);
        if let Some(t) = &mut self.quick.naming {
            if vk == VK_BACK.0 {
                t.pop();
            } else if vk == VK_RETURN.0 {
                self.save_name();
                self.quick_refresh();
            } else if vk == VK_ESCAPE.0 {
                self.quick.naming = None;
            }
            return self.quick_draw();
        }
        if let Some(t) = &mut self.quick.typing {
            if vk == VK_BACK.0 {
                t.pop();
                if t.ends_with('-') {
                    t.pop();
                }
            } else if vk == VK_RETURN.0 {
                self.quick_pair();
                self.quick_refresh();
            } else if vk == VK_ESCAPE.0 {
                self.quick.typing = None;
            }
            return self.quick_draw();
        }
        let main = self.quick.showing == Showing::Main;
        let shift = key_down(VK_SHIFT.0);
        if vk == VK_ESCAPE.0 && self.quick.showing == Showing::Devices {
            self.devices_back();
        } else if vk == VK_ESCAPE.0 {
            self.hide_quick();
        } else if vk == VK_RETURN.0 && main && self.quick.ready.is_some() {
            self.beam();
        } else if vk == VK_RETURN.0 && main && shift {
            self.quick.draft.push('\n');
            self.quick_draw();
        } else if vk == VK_RETURN.0 && main {
            self.beam_draft();
        } else if vk == VK_BACK.0 && main && self.quick.ready.is_none() {
            self.quick.draft.pop();
            self.quick_draw();
        } else if ctrl && vk == b'V' as u16 && main {
            self.quick_paste();
            self.quick_draw();
        }
    }

    /// Ctrl+V: a picture, or a password, ready to beam (a password never in
    /// the field, where it would show); text into the field.
    fn quick_paste(&mut self) {
        let text = crate::clipboard::get(self.hwnd).filter(|t| !t.trim().is_empty());
        let secret = text.as_deref().is_some_and(|t| kolada_sync::judge_secret(t, crate::clipboard::concealed()).masked());
        match text {
            Some(t) if !secret && self.quick.ready.is_none() => self.quick.draft.push_str(&t.replace("\r\n", "\n")),
            _ => {
                self.stage_clipboard();
            }
        }
    }

    /// A timer of Quick Send's: true if it was one.
    pub(super) fn quick_timer(&mut self, id: usize) -> bool {
        match id {
            HIDE_TIMER => self.stop_timer(id),
            id if id == HIDE_TIMER + 1 => {
                self.stop_timer(id);
                self.quick_draw();
            }
            TYPING_TIMER => {
                self.stop_timer(id);
                self.request_frame();
            }
            OPEN_TIMER => {
                self.step_opens();
                let now = Instant::now();
                self.quick.marks.prune(now);
                if self.quick.marks.moving(now) {
                    self.start_timer(OPEN_TIMER, 16);
                }
                if self.quick.marks.button(now, true).is_some() || self.quick.marks.moving(now) {
                    self.request_frame();
                }
                self.quick_draw();
            }
            CLEAR_TIMER => {
                self.stop_timer(id);
                if let Some(text) = self.quick.on_clipboard.take()
                    && crate::clipboard::holds(self.hwnd, &text)
                {
                    crate::clipboard::clear(self.hwnd);
                }
            }
            COUNT_TIMER => {
                if (self.quick_open() || self.ui.options_open) && matches!(self.quick.pair, Some(PairState::Showing { .. })) {
                    self.quick_refresh();
                    self.quick_draw();
                    self.request_frame();
                } else {
                    self.stop_timer(id);
                }
            }
            _ => return false,
        }
        true
    }

    /// Takes the chosen keys from the system once paired (or gives them
    /// back). A test instance takes them only when asked
    /// (`KOLADA_TEST_HOTKEY`), so the user's own Kolada keeps them.
    /// `said`: tell when another app holds them.
    pub(super) fn update_quick_hotkey(&mut self, said: bool) {
        if self.compact || (kolada_core::testing() && std::env::var_os("KOLADA_TEST_HOTKEY").is_none()) {
            return;
        }
        unsafe {
            if self.quick.held {
                let _ = UnregisterHotKey(Some(self.hwnd), QUICK_SEND);
                let _ = UnregisterHotKey(Some(self.hwnd), QUICK_SECRET);
                self.quick.held = false;
            }
            self.quick.in_use = None;
            let tries = key_tries(self.session.quick_keys);
            if tries.is_empty() || self.quick.net.is_none() {
                return;
            }
            for &i in tries {
                let (_, beam, secret) = KEYS[i];
                if RegisterHotKey(Some(self.hwnd), QUICK_SEND, beam | MOD_NOREPEAT, b'Z' as u32).is_ok() {
                    let _ = RegisterHotKey(Some(self.hwnd), QUICK_SECRET, secret | MOD_NOREPEAT, b'Z' as u32);
                    self.quick.held = true;
                    self.quick.in_use = Some(i);
                    break;
                }
            }
            if !self.quick.held && said {
                self.whisper(format!("{} is already in use by another app: choose other keys in Settings", KEYS[tries[0]].0));
            }
        }
    }

    /// The keys Beam holds now ("Alt+Z", or Win+Shift+Z in its place), if any.
    pub(super) fn beam_keys(&self) -> Option<&'static str> {
        self.quick.in_use.map(|i| KEYS[i].0)
    }

    /// Settings' Keys choice as it is in use: Win+Shift+Z where Alt+Z was
    /// chosen but another app holds it.
    pub(super) fn quick_keys_shown(&self) -> u8 {
        match self.quick.in_use {
            Some(i) => i as u8,
            None => self.session.quick_keys,
        }
    }

    /// The tray menu's line, with its keys when they are held.
    pub(super) fn quick_menu_label(&self) -> Option<String> {
        self.quick.net.as_ref()?;
        let keys = self.beam_keys();
        Some(match keys {
            Some(k) => format!("Beam\t{k}"),
            None => "Beam".to_string(),
        })
    }
}

/// The keys to try, in order, for Settings' choice: Alt+Z, then
/// Win+Shift+Z where another app holds Alt+Z; Win+Shift+Z alone; none (Off).
fn key_tries(choice: u8) -> &'static [usize] {
    match choice {
        0 => &[0, 1],
        1 => &[1],
        _ => &[],
    }
}

/// The popup's window procedure: its input goes to the app.
extern "system" fn quick_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let point = || ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32);
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut ps) };
            with_app(|a| {
                if let Some(s) = &a.quick.surface {
                    crate::cpu::blit(s, hwnd, Some(dc));
                }
            });
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            let (x, y) = point();
            with_app(|a| {
                if !a.quick.tracking {
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    a.quick.tracking = unsafe { TrackMouseEvent(&mut t) }.is_ok();
                }
                a.quick_move(x, y);
            });
            return LRESULT(0);
        }
        // The I-beam over the send area and the code field, an arrow
        // elsewhere (the class's cursor).
        WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
            let mut at = POINT::default();
            let typing = unsafe { GetCursorPos(&mut at).is_ok() && ScreenToClient(hwnd, &mut at).as_bool() } && with_app(|a| a.quick_hit(at.x, at.y).is_some_and(q::takes_typing)).unwrap_or(false);
            unsafe {
                SetCursor(LoadCursorW(None, if typing { IDC_IBEAM } else { IDC_ARROW }).ok());
            }
            return LRESULT(1);
        }
        WM_MOUSELEAVE => {
            with_app(|a| {
                a.quick.tracking = false;
                a.quick.hover = None;
                a.quick_draw();
            });
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            let (x, y) = point();
            with_app(|a| {
                if let Some(hit) = a.quick_hit(x, y) {
                    a.quick_click(hit);
                }
            });
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            let notches = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            with_app(|a| a.quick_wheel(notches));
            return LRESULT(0);
        }
        WM_KEYDOWN => {
            with_app(|a| a.quick_key(wp.0 as u16));
            return LRESULT(0);
        }
        WM_CHAR => {
            with_app(|a| a.quick_char(wp.0 as u16));
            return LRESULT(0);
        }
        // Like the clipboard panel: clicking elsewhere puts it away (not its
        // own file dialog).
        WM_ACTIVATE => {
            if wp.0 & 0xFFFF == 0 {
                with_app(|a| {
                    if !a.quick.picking {
                        a.hide_quick();
                    }
                });
            }
        }
        // A picture dropped on it, ready to beam.
        windows::Win32::UI::WindowsAndMessaging::WM_DROPFILES => {
            let drop = windows::Win32::UI::Shell::HDROP(wp.0 as *mut _);
            let files: Vec<PathBuf> = unsafe {
                use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW};
                let count = DragQueryFileW(drop, u32::MAX, None);
                let files = (0..count)
                    .filter_map(|i| {
                        let mut buf = vec![0u16; DragQueryFileW(drop, i, None) as usize + 1];
                        let n = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
                        (n > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
                    })
                    .collect();
                DragFinish(drop);
                files
            };
            with_app(|a| a.quick_attach(&files));
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn refusal(why: Refusal) -> &'static str {
    match why {
        Refusal::NoSuchCode => "No device is showing that code.",
        Refusal::Expired => "That code ran out. Show a new one.",
        Refusal::Burned => "Too many wrong tries. Show a new code.",
        Refusal::Country => "Both devices need to be in the same country.",
        Refusal::Busy => "Another device is trying that code. Wait a moment.",
        Refusal::Slow => "Too many tries. Wait a minute.",
        Refusal::Quota => "Pairing is resting until tomorrow.",
        Refusal::WrongCode => "That code didn't match.",
        Refusal::Denied => "The other device said no.",
        Refusal::Full => "Ten devices are paired already.",
        Refusal::Network => "Couldn't reach the other device.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::Action;

    /// Alt+Z by default, Win+Shift+Z when another app holds it (NVIDIA's
    /// overlay does); Win+Shift+Z alone when chosen; nothing when off.
    #[test]
    fn beam_takes_alt_z_or_win_shift_z_where_alt_z_is_held() {
        assert_eq!(KEYS[key_tries(0)[0]].0, "Alt+Z");
        assert_eq!(key_tries(0).iter().map(|&i| KEYS[i].0).collect::<Vec<_>>(), ["Alt+Z", "Win+Shift+Z"]);
        assert_eq!(key_tries(1).iter().map(|&i| KEYS[i].0).collect::<Vec<_>>(), ["Win+Shift+Z"]);
        assert!(key_tries(2).is_empty());
        assert_eq!(KEYS[0].1, HOT_KEY_MODIFIERS(MOD_ALT.0), "Alt alone, no Windows key");
    }

    /// What Windows reports goes to note sync as it was meant: a rename as
    /// one move, an edit or a new note as a change; a folder renamed, or
    /// more than the watcher could list, as a look at the whole vault.
    #[test]
    fn the_watchers_changes_reach_note_sync_as_moves_and_changes() {
        let l = |v: &[(Action, &str)]| v.iter().map(|(a, p)| (*a, p.to_string())).collect::<Vec<_>>();
        let steps = sync_steps(&l(&[(Action::Modified, "Plan.md"), (Action::RenamedFrom, "Old.md"), (Action::RenamedTo, "Work/New.md"), (Action::Added, "Fresh.md"), (Action::Removed, "Gone.md")]), false);
        assert_eq!(steps, Some(vec![SyncStep::Changed("Plan.md".into()), SyncStep::Moved("Old.md".into(), "Work/New.md".into()), SyncStep::Changed("Fresh.md".into()), SyncStep::Changed("Gone.md".into())]));
        assert_eq!(sync_steps(&l(&[(Action::RenamedFrom, "Work"), (Action::RenamedTo, "Done")]), false), None, "a folder: look again");
        assert_eq!(sync_steps(&l(&[(Action::Modified, "Plan.md")]), true), None, "overflowed");
        assert_eq!(sync_steps(&l(&[(Action::Modified, "Work")]), false), Some(vec![SyncStep::Changed("Work".into())]), "a folder's own write time is nothing to look again for");
    }
}
