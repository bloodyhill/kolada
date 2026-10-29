//! Quick Send's popup in the app model (docs/specs/2026-10-26-sync-quick-send.md,
//! section 7): what it shows and what its clicks and keys do, for every shell
//! on the model. The shell keeps a small window of its own for it (made
//! once, kept hidden, shown by the keys), draws `quick_surface` there, hands
//! the clipboard over when the keys are pressed, and does the [`QuickDo`]s it
//! gets back. Windows has the same in kolada-win/src/app/quick.rs; the
//! drawing is `kolada_ui::quick` for both.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use kolada_core::note;
use kolada_sync::{Event, Kind as SendKind, Outgoing, PairState, Refusal};
use kolada_ui::pixels::Pixels;
use kolada_ui::quick::{self as q, ClipView, DeviceRow, Hit, Host, Kind, Pairing, PicView, PopupView, Screen, Staged};
use kolada_ui::raster::Surface;
use kolada_ui::scene::{self, BeamView, Target};
use kolada_ui::{display, theme};

use super::Model;
use crate::quick::{Net, now};

/// The Beam button stays faint this long after a keystroke.
const TYPING: Duration = Duration::from_millis(1500);
/// A group's beams are each within this long of the one before.
const GROUP_MS: u64 = 5 * 60_000;
/// A card's spring steps this often.
const FRAME: Duration = Duration::from_millis(16);
/// What the popup says shows this long.
const SAID: Duration = Duration::from_secs(8);
/// "Copied" shows this long on a clip's button.
const COPIED: Duration = Duration::from_millis(1500);
/// A copied password leaves the clipboard after this long, if still there.
const CLEAR: Duration = Duration::from_secs(60);
/// A wheel notch, DIPs.
const WHEEL: f32 = 48.0;
/// A thumbnail's longer side, pixels: sharp on the card at 2x.
const THUMB_EDGE: u32 = 360;
/// The largest picture file that is read to beam.
const PICTURE_MOST: u64 = kolada_sync::engine::PICTURE_MOST as u64;

/// What the shell does for the popup.
#[derive(Clone, Debug, PartialEq)]
pub enum QuickDo {
    /// Show it above the clock (the menu bar's corner on the Mac) of the
    /// screen with the pointer, `q::W` x `height` DIPs, taking the keys.
    Show { height: f32 },
    /// The same size change while it shows, without taking the keys
    /// ("Beamed").
    Shrink { height: f32 },
    Hide,
    /// Put `text` on the clipboard; `secret`: marked concealed, so
    /// clipboard managers keep no copy.
    Copy { text: String, secret: bool },
    /// Empty the clipboard if it still holds `text` (a password, a minute on).
    Clear(String),
    /// A link, in the browser.
    Url(String),
    /// The main window, forward: a clip was opened as a note in it.
    BringUp,
    /// A notification while the popup is away; a click on it calls
    /// `quick_open(false)`.
    Notify { title: String, text: String },
    /// Pairing changed whether the keys are wanted: register them again
    /// (`quick_wants_keys`).
    Keys,
    /// The file picker, for pictures to attach (`quick_attach` with what
    /// was picked).
    PickFile,
    /// A save dialog for this picture file, then write it there.
    SaveFile { name: String, bytes: Vec<u8> },
    /// Put this picture (its file's bytes, a PNG or JPEG...) on the
    /// clipboard.
    CopyPicture(Vec<u8>),
    /// Pictures beamed together, saved: a folder picker, then each written
    /// there, never over a file (`kolada_core::files::save_all`).
    SaveAll(Vec<(String, Vec<u8>)>),
    /// Pictures beamed together, on the clipboard as files where the OS
    /// takes them (Explorer, Finder, a chat app paste them all); else the
    /// first as a picture.
    CopyFiles(Vec<(String, Vec<u8>)>),
    /// Open this file in the system's viewer (a picture, full size): the
    /// shell writes it among the app's own files, never the vault.
    OpenFile { name: String, bytes: Vec<u8> },
}

/// What is on the clipboard, as the shell reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    pub text: String,
    /// The app that copied it marked it concealed (a password manager).
    pub concealed: bool,
    /// A picture on it (a screenshot), as a file's bytes: its type and them.
    pub picture: Option<(String, Vec<u8>)>,
}

/// The keys the popup takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickKey {
    Escape,
    Enter,
    Backspace,
    /// Cmd+V (Ctrl+V): a picture or a password ready to beam, or text
    /// into the field.
    Paste,
    /// Shift+Enter: a new line in the field.
    NewLine,
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum Showing {
    #[default]
    FirstUse,
    Main,
    Devices,
}

/// What was copied, ready to beam.
struct Ready {
    text: String,
    kind: Kind,
    picture: Option<Ready2>,
    /// More pictures picked with `picture` (a file name and each), to go
    /// together (`Outgoing::Pictures`).
    rest: Vec<(String, Ready2)>,
    /// Maybe a password: masked, with "Not a password".
    unsure: bool,
}

/// A picture ready to beam: its file, and its thumbnail once made.
struct Ready2 {
    bytes: Vec<u8>,
    mime: String,
    size: String,
    /// The thumbnail, as a JPEG and as pixels, and its size in pixels.
    thumb: Option<(Vec<u8>, Arc<Pixels>, u32, u32)>,
    /// Which staging this is (a thumbnail for an older one is dropped).
    n: u64,
}

/// A thumbnail made off the UI thread: for what is ready to beam (its
/// staging), or for a card.
enum Made {
    Staged { n: u64, jpeg: Vec<u8>, pixels: Arc<Pixels>, width: u32, height: u32 },
    Card { id: kolada_sync::ItemId, pixels: Arc<Pixels> },
}

/// A device as the popup knows it: id, name, how it is drawn
/// (`kolada_sync::names::look`), and whether it is this one.
type Dev = (kolada_sync::DeviceId, String, String, bool);

#[derive(Default)]
pub(super) struct Quick {
    net: Option<Net>,
    /// Cards opened by a tap: each one's spring (where it is, how fast,
    /// where it is going), by item; and when they last stepped.
    opens: Vec<(kolada_sync::ItemId, f32, f32, bool)>,
    stepped: Option<Instant>,
    /// The card whose "…" menu is open.
    more: Option<usize>,
    /// The shell shows the popup (the last `Show`, until `Hide`).
    shown: bool,
    surface: Option<Surface>,
    hits: Vec<(scene::Rect, Target)>,
    hover: Option<Target>,
    pub(super) showing: Showing,
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
    /// The shell holds Beam's keys now (Linux: where the desktop let it).
    keys_held: bool,
    typing: Option<String>,
    /// This device's name being typed (Settings > Devices' Rename).
    naming: Option<String>,
    said: Option<(String, Instant)>,
    copied: Option<(kolada_sync::ItemId, Instant)>,
    /// A password put on the clipboard, and when.
    on_clipboard: Option<(String, Instant)>,
    cards: Vec<Shown>,
    devices: Vec<Dev>,
    paired: bool,
    pair: Option<PairState>,
    /// The first use's steps were seen.
    intro_seen: bool,
    /// Clips that arrived and were not yet seen (the side menu's count).
    unseen: u32,
    /// The side menu's row's second line (`BeamView::line`).
    row_line: String,
    /// The Beam button's menu is open, for the note named so.
    menu: Option<String>,
    /// The note was clicked in or written in since launch: the button
    /// offers it.
    engaged: bool,
    /// When the note was last written in (the button fades while typing).
    typed_at: Option<Instant>,
    /// Note sync's loop (see `start_notes_sync`), and when it was last told
    /// the open note is being typed in.
    syncer: Option<kolada_sync::syncer::Syncer>,
    sync_typed: Option<Instant>,
    /// What is typed in the send area.
    draft: String,
    /// The cards' thumbnails, decoded, and the ones being decoded.
    thumbs: Vec<(kolada_sync::ItemId, Arc<Pixels>)>,
    decoding: Vec<kolada_sync::ItemId>,
    /// Thumbnails made off the UI thread come back here, then `wake`.
    made: Option<(Sender<Made>, Receiver<Made>)>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    stagings: u64,
}

/// When `ms` was, as people say it.
fn ago(m: &Model, ms: u64) -> String {
    crate::when::ago(&*m.services.calendar, SystemTime::UNIX_EPOCH + Duration::from_millis(ms))
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

/// The vault paths (from its top, `/` between folders) the watcher's
/// changes are, for note sync; None to look at the whole vault again: a
/// path with no extension is a folder, renamed or removed with every note
/// in it.
fn sync_paths(vault: &Path, changes: &[crate::services::Change]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for c in changes {
        let rel = kolada_core::vault::relative(vault, &c.path)?;
        if !rel.rsplit('/').next().unwrap_or(&rel).contains('.') {
            return None;
        }
        out.push(rel);
    }
    Some(out)
}

/// A picture file's type, from its name: None if it is not a picture.
fn picture_type(path: &Path) -> Option<&'static str> {
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

/// A file's extension for a picture type.
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

/// A decoded picture's pixels as the scene draws them (premultiplied).
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

impl Model {
    /// After the first frame: the engine, from the keys the OS keeps. It
    /// opens no connection until this device is paired. `kind` is this
    /// device's ("desktop", "ios"); `name` how the others see it; `wake` is
    /// called from another thread when there is news (`quick_events`).
    pub fn start_quick(&mut self, base: &str, kind: &str, name: &str, wake: Arc<dyn Fn() + Send + Sync>) {
        if self.compact || self.quick.net.is_some() {
            return;
        }
        let data = self.data.as_ref().map(|d| d.join("sync"));
        let net = Net::start(self.services.socket.clone(), self.services.keys.clone(), self.services.web.clone(), data, base, kind, name, wake.clone());
        self.quick.wake = Some(wake);
        self.quick_with(net);
        // On by default; `KOLADA_SYNC_NOTES=0` turns it off (tests).
        if std::env::var("KOLADA_SYNC_NOTES").as_deref() != Ok("0") {
            self.start_notes_sync();
        }
    }

    /// Note sync (spec section 8) for this vault, on a thread of its own,
    /// after the first frame. It does nothing, not even a scan, until a
    /// paired device syncs too (`Engine::sync_peers`). Its writes come back
    /// through the vault's watcher, as any outside change does.
    pub fn start_notes_sync(&mut self) {
        let (Some(net), Some(data)) = (&self.quick.net, &self.data) else { return };
        let (root, data) = (self.vault.clone(), data.clone());
        let open = move || kolada_sync::vault::Vault::open(&root, &data, true);
        // Switched on here: the group's other devices sync notes with it
        // (a phone that follows, or another desktop switched on).
        net.with(|e, _| e.set_sync(kolada_sync::SyncCap::On));
        self.quick.syncer = Some(kolada_sync::syncer::Syncer::open(open, net.engine(), net.kicker(), Arc::new(|_| {})));
    }

    /// What the vault's watcher saw, for note sync.
    pub(super) fn sync_saw(&self, changes: &[crate::services::Change]) {
        let Some(s) = &self.quick.syncer else { return };
        match sync_paths(&self.vault, changes) {
            None => s.rescan(),
            Some(paths) => paths.iter().for_each(|p| s.changed(p)),
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

    /// Quick Send on an engine already open (a shell's dumps hand in a
    /// paired one, `kolada_sync::testing`), with the services' socket and keys.
    pub fn start_quick_on(&mut self, engine: kolada_sync::Engine, wake: Arc<dyn Fn() + Send + Sync>) {
        let net = Net::with_engine(engine, self.services.socket.clone(), self.services.keys.clone(), self.services.web.clone(), wake.clone());
        self.quick.wake = Some(wake);
        self.quick_with(net);
    }

    pub(super) fn quick_with(&mut self, net: Net) {
        self.quick.net = Some(net);
        self.quick.intro_seen = self.intro_file().is_some_and(|p| p.exists());
        self.quick_refresh();
    }

    fn intro_file(&self) -> Option<PathBuf> {
        self.data.as_ref().map(|d| d.join("sync").join("intro.seen"))
    }

    /// Whether Quick Send runs here.
    pub fn quick_on(&self) -> bool {
        self.quick.net.is_some()
    }

    /// Whether the keys should be held: once paired, as Settings says
    /// (`Session::quick_keys`, 2 off). Until then the keys open the popup
    /// to pair, so they are held too.
    pub fn quick_wants_keys(&self) -> bool {
        self.quick.net.is_some() && self.session.quick_keys != 2
    }

    /// Which keys Settings has for Beam (`Session::quick_keys`: 0 the
    /// default, 1 the other, 2 off): the shell holds those.
    pub fn quick_keys_choice(&self) -> u8 {
        self.session.quick_keys
    }

    /// The shell held other keys than the ones chosen (another app had
    /// them): Settings and the popup say the ones held.
    pub fn set_quick_keys_choice(&mut self, keys: u8) {
        self.session.quick_keys = keys;
    }

    /// The Mac's keys as Settings has them: Option+Z, or the old
    /// Ctrl+Option+Shift+Z.
    fn mac_keys(&self) -> &'static str {
        if self.session.quick_keys == 1 { Host::MAC_OLD_KEYS } else { Host::MAC.keys }
    }

    /// Whether the popup shows (the shell's last `Show`, until `Hide`).
    pub fn quick_shown(&self) -> bool {
        self.quick.shown
    }

    /// The engine said something (the wake).
    pub fn quick_events(&mut self) -> Vec<QuickDo> {
        self.quick_thumbs();
        let mut dos = Vec::new();
        let Some(net) = &self.quick.net else { return dos };
        let open = self.quick.shown;
        let mut says = Vec::new();
        let mut paired_now = false;
        for e in net.take_events() {
            match e {
                Event::Arrived(id) if !open => {
                    // A picture that came with a note is told as part of the note.
                    let card = net.read(|e| e.cards().into_iter().find(|c| c.id == id).cloned()).filter(|c| c.picture.as_ref().is_none_or(|p| p.note.is_none()));
                    if let Some(c) = card {
                        let from = net.read(|e| e.devices().iter().find(|d| d.id == c.from).map(|d| d.name.clone())).unwrap_or_default();
                        let title = if from.is_empty() { "Beam".to_string() } else { format!("From {from}") };
                        let text = match c.kind {
                            SendKind::Secret => "A password: click to copy it once".to_string(),
                            SendKind::Picture => "A picture".to_string(),
                            _ => c.text.chars().take(200).collect(),
                        };
                        dos.push(QuickDo::Notify { title, text });
                    }
                }
                Event::Paired => {
                    paired_now = true;
                    says.push("Paired. Copy something and press the keys to beam it.");
                }
                Event::CodeRefused(why) => says.push(refusal(why)),
                Event::Removed => says.push("This device is no longer paired."),
                Event::Resting { .. } => says.push("Your devices reconnect tomorrow; nothing is lost."),
                Event::Sent(id) => {
                    self.quick.marks.acked(id, Instant::now());
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
        for s in says {
            self.quick_say(s);
        }
        let was = self.quick.paired;
        self.quick_refresh();
        if was != self.quick.paired {
            dos.push(QuickDo::Keys);
        }
        if paired_now && self.quick.showing == Showing::Devices {
            self.quick.showing = Showing::Main;
        }
        // What arrives while the popup shows its clips is seen there.
        if open && self.quick.showing == Showing::Main {
            self.quick_seen();
        }
        dos
    }

    /// Where thumbnails made off the UI thread are sent.
    fn quick_made(&mut self) -> Sender<Made> {
        self.quick.made.get_or_insert_with(mpsc::channel).0.clone()
    }

    /// Takes the thumbnails made since the last look: whether any came.
    fn quick_thumbs(&mut self) -> bool {
        let Some((_, rx)) = &self.quick.made else { return false };
        let made: Vec<Made> = rx.try_iter().collect();
        let any = !made.is_empty();
        for m in made {
            match m {
                Made::Card { id, pixels } => {
                    self.quick.decoding.retain(|k| *k != id);
                    self.quick.thumbs.push((id, pixels));
                }
                Made::Staged { n, jpeg, pixels, width, height } => {
                    let ready = self.quick.ready.as_mut();
                    if let Some(pic) = ready.and_then(|r| r.picture.iter_mut().chain(r.rest.iter_mut().map(|(_, p)| p)).find(|p| p.n == n)) {
                        pic.thumb = Some((jpeg, pixels, width, height));
                    }
                }
            }
        }
        any
    }

    fn quick_say(&mut self, text: &str) {
        self.quick.said = Some((text.to_string(), Instant::now()));
    }

    /// Reads the engine into what the popup shows.
    fn quick_refresh(&mut self) {
        let Some(net) = &self.quick.net else { return };
        let raw = net.read(|e| {
            let me = e.device_id();
            let name_of = |id: &kolada_sync::DeviceId| e.devices().iter().find(|d| d.id == *id).map(|d| d.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "a device".into());
            // A note's pictures, and the rest of pictures beamed together,
            // show as part of their card, not as cards of their own.
            let notes: Vec<kolada_sync::ItemId> = e.cards().iter().filter(|c| c.kind == SendKind::Text && !c.title.is_empty()).map(|c| c.id).collect();
            let ids: Vec<kolada_sync::ItemId> = e.cards().iter().map(|c| c.id).collect();
            let cards: Vec<_> = e.cards().into_iter().filter(|c| c.picture.as_ref().and_then(|p| p.note).is_none_or(|n| !ids.contains(&n))).map(|c| ((*c).clone(), name_of(&c.from))).collect();
            let with: Vec<(kolada_sync::ItemId, usize)> = notes.iter().map(|n| (*n, e.note_pictures(*n).len())).collect();
            let rest: Vec<(kolada_sync::ItemId, Vec<kolada_sync::Card>)> = cards.iter().filter(|(c, _)| c.kind == SendKind::Picture).map(|(c, _)| (c.id, e.note_pictures(c.id).into_iter().cloned().collect::<Vec<_>>())).filter(|(_, r)| !r.is_empty()).collect();
            let mut devices: Vec<Dev> = e.devices().iter().map(|d| (d.id, if d.id == me { e.name().to_string() } else { d.name.clone() }, kolada_sync::names::look(&d.kind, if d.id == me { e.model() } else { &d.model }).to_string(), d.id == me)).collect();
            devices.sort_by_key(|d| !d.3);
            (cards, devices, e.paired(), e.pair_state(), e.unread(), with, rest)
        });
        let (cards, devices, paired, pair, unseen, with, rest) = raw;
        self.quick.unseen = unseen as u32;
        let followers: Vec<&kolada_sync::Card> = rest.iter().flat_map(|(_, r)| r.iter()).collect();
        let ready: Vec<kolada_sync::ItemId> = net.read(|e| cards.iter().map(|(c, _)| c).chain(followers.iter().copied()).filter(|c| c.picture.is_some() && e.has_picture(c.id)).map(|c| c.id).collect());
        // Thumbnails not decoded yet are, off this thread.
        let todo: Vec<(kolada_sync::ItemId, Vec<u8>)> = cards
            .iter()
            .map(|(c, _)| c)
            .chain(followers.iter().copied())
            .filter_map(|c| c.picture.as_ref().map(|p| (c.id, p.thumb.clone())))
            .filter(|(id, t)| !t.is_empty() && !self.quick.thumbs.iter().any(|(k, _)| k == id) && !self.quick.decoding.contains(id))
            .collect();
        for (id, jpeg) in todo {
            self.quick.decoding.push(id);
            let images = self.services.images.clone();
            let tx = self.quick_made();
            let wake = self.quick.wake.clone();
            std::thread::spawn(move || {
                let Some(raster) = images.recode().and_then(|r| r.decode(&jpeg, THUMB_EDGE)) else { return };
                let _ = tx.send(Made::Card { id, pixels: Arc::new(pixels_of(&raster)) });
                if let Some(w) = wake {
                    w();
                }
            });
        }
        let live: Vec<kolada_sync::ItemId> = cards.iter().map(|(c, _)| c.id).chain(followers.iter().map(|c| c.id)).collect();
        self.quick.thumbs.retain(|(id, _)| live.contains(id));
        // A beam that didn't arrive shows red, after a restart too.
        for (c, _) in cards.iter().filter(|(c, _)| c.lost) {
            self.quick.marks.lost(c.id);
        }
        // The side menu's row: the newest beam not yet seen, else where the
        // last one came from and when.
        let first_line = |c: &kolada_sync::Card| match c.kind {
            SendKind::Secret => "A password".to_string(),
            SendKind::Picture => "A picture".to_string(),
            _ => c.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(80).collect(),
        };
        self.quick.row_line = match cards.iter().find(|(c, _)| !c.read && !c.mine) {
            Some((c, _)) => first_line(c),
            None => match cards.first() {
                Some((c, _)) if c.mine => format!("You beamed, {}", ago(self, c.at)),
                Some((c, from)) => format!("From {from}, {}", ago(self, c.at)),
                None => String::new(),
            },
        };
        // One faint line a group, on its first card (the oldest): who and
        // when. A group is one sender's beams, each within 5 minutes of the
        // one before.
        let metas: Vec<String> = (0..cards.len())
            .map(|i| {
                let (c, from) = &cards[i];
                let first = cards.get(i + 1).is_none_or(|(o, _)| o.mine != c.mine || o.from != c.from || c.at.saturating_sub(o.at) >= GROUP_MS);
                match (first, c.mine) {
                    (false, _) => String::new(),
                    (true, true) => format!("You \u{00B7} {}", ago(self, c.at)),
                    (true, false) => format!("{from} \u{00B7} {}", ago(self, c.at)),
                }
            })
            .collect();
        self.quick.cards = cards
            .into_iter()
            .zip(metas)
            .map(|((c, _), from)| Shown {
                id: c.id,
                from,
                goes: if c.lost { q::LOST.into() } else if c.kind == SendKind::Secret { "Goes once copied".into() } else { q::with_pictures(with.iter().find(|(n, _)| *n == c.id).map_or(0, |(_, k)| *k)) },
                kind: match c.kind {
                    SendKind::Text => Kind::Text,
                    SendKind::Link => Kind::Link,
                    SendKind::Secret => Kind::Secret,
                    SendKind::Picture => Kind::Picture,
                },
                spent: c.kind == SendKind::Secret && c.copied,
                mine: c.mine,
                picture: c.picture.as_ref().map(|p| (p.width, p.height, file_size(p.size), ready.contains(&c.id))),
                rest: rest.iter().find(|(k, _)| *k == c.id).map(|(_, r)| r.iter().filter_map(|f| f.picture.as_ref().map(|p| (f.id, (p.width, p.height, file_size(p.size), ready.contains(&f.id))))).collect()).unwrap_or_default(),
                text: c.text,
                title: c.title,
            })
            .collect();
        self.quick.devices = devices;
        self.quick.paired = paired;
        self.quick.pair = Some(pair);
        if self.quick.chosen > self.quick.devices.iter().filter(|d| !d.3).count() {
            self.quick.chosen = 0;
        }
    }

    /// Opens the popup: the first use's steps the first time, the devices
    /// while none is paired, else the clips; with `devices`, the devices.
    /// From the menu bar's (tray's) entry, Settings' Devices… and a click
    /// on a notification.
    pub fn quick_open(&mut self, devices: bool) -> Vec<QuickDo> {
        if self.quick.net.is_none() {
            return Vec::new();
        }
        self.quick_refresh();
        // Nothing paired yet (and the first use seen), or asked for: pairing,
        // here in the popup.
        self.quick.showing = if !self.quick.intro_seen {
            Showing::FirstUse
        } else if !self.quick.paired || devices {
            Showing::Devices
        } else {
            Showing::Main
        };
        self.quick.scroll = 0.0;
        if self.quick.showing == Showing::Main {
            self.quick_seen();
        }
        self.quick_show()
    }

    /// Back from the popup's Devices (its arrow, or Esc): a pairing under
    /// way here stops; the beams again, or the popup goes while nothing
    /// is paired.
    fn devices_back(&mut self) -> Vec<QuickDo> {
        self.quick.typing = None;
        self.quick.naming = None;
        if matches!(self.quick.pair, Some(PairState::Opening | PairState::Showing { .. } | PairState::Waiting))
            && let Some(net) = &self.quick.net
        {
            net.with(|e, _| e.cancel_pairing());
        }
        if self.quick.paired {
            self.quick.showing = Showing::Main;
            self.quick_show()
        } else {
            self.quick_gone()
        }
    }

    /// How many beams are not yet seen; None while Beam is off.
    pub fn quick_badge(&self) -> Option<u32> {
        self.quick.net.as_ref().map(|_| self.quick.unseen)
    }

    /// The paired devices, as Settings > Devices and the popup list them.
    pub(super) fn device_rows(&self) -> Vec<DeviceRow<'_>> {
        self.quick.devices.iter().map(|d| DeviceRow { name: &d.1, look: kolada_ui::quick::Look::of(&d.2), this: d.3 }).collect()
    }

    /// A code's time left ("4:12 left"), while one shows.
    pub(super) fn pair_left(&self) -> String {
        match &self.quick.pair {
            Some(PairState::Showing { expires, .. }) => {
                let s = expires.saturating_sub(now()) / 1000;
                format!("{}:{:02} left", s / 60, s % 60)
            }
            _ => String::new(),
        }
    }

    /// Note sync's lines for Settings > Devices, while it runs.
    pub(super) fn sync_facts(&self) -> Vec<String> {
        self.quick.syncer.as_ref().map(|s| s.state().facts()).unwrap_or_default()
    }

    /// Settings > Devices, while pairing runs here: the rows, time left and
    /// sync's lines are the caller's (`device_rows`, `pair_left`,
    /// `sync_facts`).
    pub(super) fn devices_view<'a>(&'a self, rows: &'a [DeviceRow<'a>], left: &'a str, facts: &'a [String]) -> Option<scene::DevicesView<'a>> {
        self.quick.net.as_ref()?;
        let this = match self.platform {
            scene::Platform::Mac => "Mac",
            scene::Platform::Ios => "device",
            _ => "PC",
        };
        let said = self.quick.said.as_ref().filter(|(_, at)| at.elapsed() < SAID).map(|(s, _)| s.as_str());
        Some(scene::DevicesView { devices: rows, pairing: pairing_of(self.quick.typing.as_deref(), self.quick.pair.as_ref(), left), said, facts, this, naming: self.quick.naming.as_deref() })
    }

    /// Typing while the code, or this device's name, is entered in
    /// Settings > Devices: true if it was theirs.
    pub(super) fn code_typed(&mut self, s: &str) -> bool {
        if !self.settings.open || self.quick.typing.is_none() && self.quick.naming.is_none() {
            return false;
        }
        for c in s.chars() {
            self.quick_char(c);
        }
        true
    }

    pub(super) fn code_key(&mut self, key: QuickKey) -> bool {
        if !self.settings.open || self.quick.typing.is_none() && self.quick.naming.is_none() {
            return false;
        }
        self.quick_key(key, &|| None);
        true
    }

    /// What Beam's side-menu row and window button show; None while Beam
    /// is off. Read from what the last refresh kept: a frame never waits on
    /// the engine.
    pub fn beam_view(&self) -> Option<BeamView<'_>> {
        self.quick.net.as_ref()?;
        Some(BeamView {
            unseen: self.quick.unseen,
            line: &self.quick.row_line,
            typing: self.quick.typed_at.is_some_and(|t| t.elapsed() < TYPING),
            menu: self.quick.menu.as_deref(),
            // The Mac's keys, where it holds them; none on iOS or Linux.
            keys: match self.platform {
                scene::Platform::Mac if self.quick_wants_keys() => self.mac_keys(),
                scene::Platform::Linux if self.quick.keys_held => Host::LINUX.keys,
                _ => "",
            },
            check: self.quick.marks.button(Instant::now(), self.ui.animate),
        })
    }

    /// Whether the shell holds Beam's keys now (Linux, where the desktop
    /// lets an app have keys of its own): the button and its menu say them.
    pub fn set_quick_keys_held(&mut self, held: bool) -> bool {
        std::mem::replace(&mut self.quick.keys_held, held) != held
    }

    /// A check by the window's Beam button is showing: frames are due.
    pub(super) fn beam_check_moving(&self) -> bool {
        self.quick.marks.button(Instant::now(), true).is_some()
    }

    /// The note was clicked in (or `written` in): the Beam button offers it.
    pub(super) fn note_used(&mut self, written: bool) {
        self.quick.engaged = true;
        if written {
            self.quick.typed_at = Some(Instant::now());
        }
    }

    /// A press on Beam's button or its menu: Some (what it changed) if it
    /// was about them. A press elsewhere while the menu is open closes it,
    /// and is used up.
    pub(super) fn beam_click(&mut self, target: Option<Target>) -> Option<bool> {
        if self.quick.net.is_none() {
            return None;
        }
        match target {
            Some(Target::Quick(Hit::Fab)) => {
                if self.quick.menu.take().is_some() {
                    return Some(true);
                }
                // With the note in use and something in it: this note, or a
                // new beam. Just after launch, or with nothing in it: the popup.
                let words = !self.editor.text().trim().is_empty();
                if words && self.quick.engaged {
                    let title = self.editor.title();
                    self.quick.menu = Some(if title.trim().is_empty() { "This note".into() } else { title });
                    Some(true)
                } else {
                    self.settings.requests.push(super::settings::Request::QuickOpen);
                    Some(false)
                }
            }
            Some(Target::Quick(Hit::BeamNote)) => {
                self.quick.menu = None;
                self.settings.requests.push(super::settings::Request::BeamNote);
                Some(true)
            }
            Some(Target::Quick(Hit::NewBeam)) => {
                self.quick.menu = None;
                self.settings.requests.push(super::settings::Request::QuickOpen);
                Some(true)
            }
            _ if self.quick.menu.is_some() => {
                self.quick.menu = None;
                Some(true)
            }
            _ => None,
        }
    }

    /// Beams the open note, from the Beam button's menu: its Markdown as it
    /// is, under its name, and each picture it embeds that is in the vault,
    /// so the other device saves it as the same note (`Outgoing::Note`).
    pub fn quick_beam_note(&mut self) -> Vec<QuickDo> {
        if self.quick.net.is_none() || !self.quick.paired {
            return self.quick_open(false);
        }
        let text = self.editor.text().trim_end().to_string();
        if text.trim().is_empty() {
            return Vec::new();
        }
        let title = self.path.as_deref().and_then(Path::file_stem).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| self.editor.title());
        let from = self.path.as_deref().and_then(Path::parent).map_or_else(|| self.vault.clone(), Path::to_path_buf);
        let pictures = kolada_ui::pictures::read_note_pictures(&self.vault, &from, &text, kolada_sync::engine::PICTURE_MOST)
            .into_iter()
            .map(|f| kolada_sync::NotePicture { name: f.name, mime: f.mime.into(), bytes: f.bytes, width: f.width, height: f.height, thumb: Vec::new() })
            .collect();
        let Some(net) = &self.quick.net else { return Vec::new() };
        let sent = net.with(|e, now| e.send_to(Outgoing::Note { title, text, pictures }, None, now));
        match sent {
            Some(id) => {
                let place = if self.quick.shown { q::Place::Card } else { q::Place::Button };
                self.quick.marks.sent(id, place);
                self.quick.scroll = 0.0;
            }
            None => self.quick_say("This note is too long to beam."),
        }
        self.quick_refresh();
        Vec::new()
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
        self.quick_refresh();
    }

    fn quick_show(&mut self) -> Vec<QuickDo> {
        self.quick.shown = true;
        vec![QuickDo::Show { height: q::H }]
    }

    /// The popup went (the shell hid it, or it lost the keys).
    fn quick_gone(&mut self) -> Vec<QuickDo> {
        self.quick.shown = false;
        // Put away unbeamed, what was copied is let go: the keys read the
        // clipboard again next time.
        self.quick.ready = None;
        self.quick.typing = None;
        self.quick.naming = None;
        self.quick.hover = None;
        vec![QuickDo::Hide]
    }

    /// The popup lost the keys (a click elsewhere): it goes.
    pub fn quick_deactivated(&mut self) -> Vec<QuickDo> {
        if !self.quick.shown {
            return Vec::new();
        }
        self.quick_gone()
    }

    /// "Not a password": what is ready, maybe a password, beams as the
    /// text (or link) it is. Nothing about it is remembered.
    fn quick_not_secret(&mut self) {
        if let Some(r) = self.quick.ready.as_mut().filter(|r| r.unsure && r.kind == Kind::Secret) {
            r.kind = kind_of(&r.text, false);
            r.unsure = false;
        }
    }

    /// What is on the clipboard, ready to beam: a picture, or text.
    fn quick_stage(&mut self, clip: Option<Clip>) -> bool {
        if let Some((mime, bytes)) = clip.as_ref().and_then(|c| c.picture.clone()) {
            return self.quick_stage_picture(bytes, mime, "Picture".into());
        }
        match clip.filter(|c| !c.text.trim().is_empty()) {
            Some(c) => {
                let secrecy = kolada_sync::judge_secret(&c.text, c.concealed);
                self.quick.ready = Some(Ready { kind: kind_of(&c.text, secrecy.masked()), text: c.text, picture: None, rest: Vec::new(), unsure: secrecy == kolada_sync::Secrecy::Unsure });
                true
            }
            None => {
                self.quick_say("The clipboard has nothing to beam.");
                false
            }
        }
    }

    /// A picture ready to beam; its thumbnail is made off this thread. `name`
    /// is its file's name without its extension.
    fn quick_stage_picture(&mut self, bytes: Vec<u8>, mime: String, name: String) -> bool {
        self.quick.ready = None;
        self.quick_add_picture(bytes, mime, name)
    }

    /// One more picture ready to beam, beside those already (`Ready::rest`),
    /// within what goes at once (`kolada_sync::pictures_over`).
    fn quick_add_picture(&mut self, bytes: Vec<u8>, mime: String, name: String) -> bool {
        if bytes.len() as u64 > PICTURE_MOST {
            self.quick_say("That picture is too big to beam: 20 MB at most.");
            return false;
        }
        let (count, held) = self.quick.ready.as_ref().filter(|r| r.picture.is_some()).map_or((0, 0), |r| (1 + r.rest.len(), r.picture.iter().chain(r.rest.iter().map(|(_, p)| p)).map(|p| p.bytes.len()).sum()));
        if let Some(why) = kolada_sync::pictures_over(count + 1, held + bytes.len()) {
            self.quick_say(&why);
            return false;
        }
        self.quick.stagings += 1;
        let n = self.quick.stagings;
        let size = file_size(bytes.len() as u64);
        let file = format!("{name}.{}", picture_ext(&mime));
        let (images, tx, wake, copy) = (self.services.images.clone(), self.quick_made(), self.quick.wake.clone(), bytes.clone());
        std::thread::spawn(move || {
            let Some(recode) = images.recode() else { return };
            let Some(raster) = recode.decode(&copy, THUMB_EDGE) else { return };
            let jpeg = recode.encode(&raster, 72, false).filter(|j| j.len() <= kolada_sync::engine::THUMB_MOST).unwrap_or_default();
            let _ = tx.send(Made::Staged { n, jpeg, pixels: Arc::new(pixels_of(&raster)), width: raster.width, height: raster.height });
            if let Some(w) = wake {
                w();
            }
        });
        let pic = Ready2 { bytes, mime, size, thumb: None, n };
        match self.quick.ready.as_mut().filter(|r| r.picture.is_some()) {
            Some(r) => r.rest.push((file, pic)),
            None => self.quick.ready = Some(Ready { text: file, kind: Kind::Picture, picture: Some(pic), rest: Vec::new(), unsure: false }),
        }
        true
    }

    /// One of the pictures ready to beam goes (0 is the first); the rest stay.
    fn quick_unstage(&mut self, k: usize) {
        let Some(r) = self.quick.ready.as_mut() else { return };
        if k > 0 {
            if k <= r.rest.len() {
                r.rest.remove(k - 1);
            }
        } else if r.rest.is_empty() {
            self.quick.ready = None;
        } else {
            let (text, pic) = r.rest.remove(0);
            r.text = text;
            r.picture = Some(pic);
        }
    }

    /// Pictures picked in the file picker, dropped on the popup or pasted:
    /// all of them ready to beam together, beside any already ready (up to
    /// what goes at once). Only pictures go, for now.
    pub fn quick_attach(&mut self, paths: &[PathBuf]) -> Vec<QuickDo> {
        if self.quick.net.is_none() || !self.quick.paired {
            return Vec::new();
        }
        let dos = if self.quick.shown && self.quick.showing == Showing::Main { Vec::new() } else { self.quick_open(false) };
        // What was ready that isn't a picture gives way to them.
        if self.quick.ready.as_ref().is_some_and(|r| r.picture.is_none()) {
            self.quick.ready = None;
        }
        let pictures: Vec<(&PathBuf, &str)> = paths.iter().filter_map(|p| picture_type(p).map(|m| (p, m))).collect();
        if pictures.is_empty() {
            if !paths.is_empty() {
                self.quick_say("Only pictures can be beamed for now.");
            }
            return dos;
        }
        for (path, mime) in pictures {
            if std::fs::metadata(path).is_ok_and(|m| m.len() > PICTURE_MOST) {
                self.quick_say("That picture is too big to beam: 20 MB at most.");
                continue;
            }
            match std::fs::read(path) {
                Ok(bytes) => {
                    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Picture".into());
                    if !self.quick_add_picture(bytes, mime.into(), name) {
                        break;
                    }
                }
                Err(_) => self.quick_say("That file couldn't be read."),
            }
        }
        dos
    }

    /// Beams what is ready to whoever is chosen. The popup stays (or none
    /// shows): the beam is the timeline's newest, pending until the server
    /// has it, then its check shows a moment (by the window's Beam button
    /// where no popup shows).
    fn quick_beam(&mut self) -> Vec<QuickDo> {
        self.quick_send();
        Vec::new()
    }

    /// Sends what is ready: its item, if it went.
    fn quick_send(&mut self) -> Option<kolada_sync::ItemId> {
        let ready = self.quick.ready.take()?;
        let net = self.quick.net.as_ref()?;
        let others: Vec<&Dev> = self.quick.devices.iter().filter(|d| !d.3).collect();
        let to = match self.quick.chosen {
            0 => None,
            i => others.get(i - 1).map(|d| vec![d.0]),
        };
        let as_note = |name: String, pic: Ready2| {
            let (thumb, width, height) = pic.thumb.map(|(j, _, w, h)| (j, w, h)).unwrap_or_default();
            kolada_sync::NotePicture { name, mime: pic.mime, bytes: pic.bytes, width, height, thumb }
        };
        let out = match (ready.kind, ready.picture) {
            (_, Some(pic)) if !ready.rest.is_empty() => Outgoing::Pictures(std::iter::once(as_note(ready.text, pic)).chain(ready.rest.into_iter().map(|(n, p)| as_note(n, p))).collect()),
            (_, Some(pic)) => {
                let (thumb, width, height) = pic.thumb.map(|(j, _, w, h)| (j, w, h)).unwrap_or_default();
                Outgoing::Picture { bytes: pic.bytes, mime: pic.mime, name: ready.text, width, height, thumb }
            }
            (Kind::Secret, None) => Outgoing::Secret(ready.text),
            _ => Outgoing::Text(ready.text),
        };
        let sent = net.with(|e, now| e.send_to(out, to.as_deref(), now));
        match sent {
            Some(id) => {
                let place = if self.quick.shown { q::Place::Card } else { q::Place::Button };
                self.quick.marks.sent(id, place);
                // The newest, at the foot.
                self.quick.scroll = 0.0;
            }
            None => self.quick_say("Too long to beam: a few pages at most."),
        }
        self.quick_refresh();
        sent
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
            let place = if self.quick.shown { q::Place::Card } else { q::Place::Button };
            self.quick.marks.sent(again, place);
            self.quick.scroll = 0.0;
        }
        self.quick_refresh();
    }

    /// Beams what is typed in the send area.
    fn quick_beam_draft(&mut self) -> Vec<QuickDo> {
        let text = self.quick.draft.trim_end().to_string();
        if text.trim().is_empty() {
            return Vec::new();
        }
        self.quick.ready = Some(Ready { kind: kind_of(&text, false), text, picture: None, rest: Vec::new(), unsure: false });
        if self.quick_send().is_some() {
            self.quick.draft.clear();
        }
        Vec::new()
    }

    /// The keys: what was copied, shown first (or beamed at once, as
    /// Settings says). `secret`: the password keys. `clip` reads the
    /// clipboard, only once the first use is seen and this device paired.
    pub fn quick_keys(&mut self, secret: bool, clip: &dyn Fn() -> Option<Clip>) -> Vec<QuickDo> {
        if self.quick.net.is_none() {
            return Vec::new();
        }
        if !self.quick.paired || !self.quick.intro_seen {
            return self.quick_open(false);
        }
        if self.session.quick_beam == 1 {
            if self.quick_stage(clip()) {
                if secret && let Some(r) = &mut self.quick.ready {
                    r.kind = Kind::Secret;
                    r.unsure = false;
                }
                // Maybe a password: shown, for "Not a password", not beamed.
                if !self.quick.ready.as_ref().is_some_and(|r| r.unsure) {
                    self.quick.chosen = 0;
                    return self.quick_beam();
                }
            }
            return self.quick_open(false);
        }
        let dos = self.quick_open(false);
        self.quick_stage(clip());
        if secret && let Some(r) = &mut self.quick.ready {
            r.kind = Kind::Secret;
            r.unsure = false;
        }
        dos
    }

    // Drawing.

    /// Draws the popup's frame of `w` x `h` pixels at `scale` pixels per DIP.
    pub fn quick_frame(&mut self, w: u32, h: u32, scale: f32) {
        self.quick_thumbs();
        let (wd, hd) = (w as f32 / scale, h as f32 / scale);
        let said = self.quick.said.as_ref().filter(|(_, at)| at.elapsed() < SAID).map(|(s, _)| s.clone());
        let copied = self.quick.copied.filter(|(_, at)| at.elapsed() < COPIED).map(|(id, _)| id);
        let quick = &self.quick;
        let thumb_of = |id: &kolada_sync::ItemId| quick.thumbs.iter().find(|(k, _)| k == id).map(|(_, p)| p.clone());
        let rests: Vec<Vec<PicView>> = quick.cards.iter().map(|c| c.rest.iter().map(|(id, (w, h, size, ready))| PicView { pixels: thumb_of(id), width: *w, height: *h, size, ready: *ready }).collect()).collect();
        let staged_rest: Vec<PicView> = quick.ready.iter().flat_map(|r| r.rest.iter()).map(|(_, p)| PicView { pixels: p.thumb.as_ref().map(|t| t.1.clone()), width: p.thumb.as_ref().map_or(4, |t| t.2), height: p.thumb.as_ref().map_or(3, |t| t.3), size: &p.size, ready: true }).collect();
        let clips: Vec<ClipView> = quick
            .cards
            .iter()
            .zip(&rests)
            .map(|(c, rest)| ClipView {
                sent: quick.marks.card(c.id, Instant::now(), self.ui.animate),
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
                open: quick.opens.iter().find(|o| o.0 == c.id).map_or(0.0, |o| o.1),
            })
            .collect();
        let mut to = vec!["All devices"];
        to.extend(quick.devices.iter().filter(|d| !d.3).map(|d| d.1.as_str()));
        let rows = self.device_rows();
        let left = self.pair_left();
        let facts = self.sync_facts();
        let what = |k: Kind| match k {
            Kind::Link => "a link",
            Kind::Secret => "a password",
            Kind::Text => "text",
            Kind::Picture => "a picture",
        };
        let saved = quick.saved.as_ref().filter(|(_, _, at)| at.elapsed() < SAID).map(|(n, _, _)| n.as_str());
        let screen = match quick.showing {
            Showing::FirstUse => Screen::FirstUse,
            Showing::Devices => {
                let pairing = pairing_of(quick.typing.as_deref(), quick.pair.as_ref(), &left);
                Screen::Devices { devices: &rows, pairing, facts: &facts, naming: quick.naming.as_deref() }
            }
            Showing::Main => Screen::Main {
                staged: quick.ready.as_ref().map(|r| Staged {
                    what: what(r.kind),
                    kind: r.kind,
                    text: &r.text,
                    picture: r.picture.as_ref().map(|p| PicView { pixels: p.thumb.as_ref().map(|t| t.1.clone()), width: p.thumb.as_ref().map_or(4, |t| t.2), height: p.thumb.as_ref().map_or(3, |t| t.3), size: &p.size, ready: true }),
                    more: &staged_rest,
                    unsure: r.unsure,
                }),
                to: &to,
                chosen: quick.chosen,
                clips: &clips,
                scroll: quick.scroll,
                saved,
                draft: &quick.draft,
                more: quick.more,
            },
        };
        let devices = quick.devices.iter().filter(|d| !d.3).count();
        // The Mac's words and keys; Linux has Ctrl keys, as Windows.
        let host = match self.platform {
            scene::Platform::Mac => Host { keys: self.mac_keys(), ..Host::MAC },
            scene::Platform::Ios => Host::MAC,
            scene::Platform::Linux => Host::LINUX,
            _ => Host::WINDOWS,
        };
        let view = PopupView { screen, devices, said: said.as_deref(), host };
        let Ok(built) = q::popup(&self.eng, &self.fonts, &view, quick.hover, wd, hd) else { return };
        let Ok(list) = display::record(&self.eng, &built, theme::for_mode(self.dark), scale) else { return };
        self.quick.scroll_max = built.quick_scroll_max;
        self.quick.scroll = self.quick.scroll.min(self.quick.scroll_max);
        let px = (w, h, (scale * 96.0).round() as u32);
        let surface = self.quick.surface.get_or_insert_with(|| Surface::new(px));
        surface.draw(px, &list);
        self.quick.hits = built.hits;
    }

    /// The popup's last frame, once one was drawn.
    pub fn quick_surface(&self) -> Option<&Surface> {
        self.quick.surface.as_ref()
    }

    fn quick_hit(&self, x: f32, y: f32) -> Option<Hit> {
        match scene::hit(&self.quick.hits, x, y) {
            Some(Target::Quick(hit)) if !matches!(hit, Hit::Popup | Hit::List) => Some(hit),
            _ => None,
        }
    }

    /// The pointer over the popup, DIPs: whether it changes.
    pub fn quick_pointer_moved(&mut self, x: f32, y: f32) -> bool {
        let hover = self.quick_hit(x, y).map(Target::Quick);
        let changed = hover != self.quick.hover;
        self.quick.hover = hover;
        changed
    }

    /// Whether the pointer is over text that takes typing in the popup:
    /// the shell shows the I-beam there.
    pub fn quick_takes_typing(&self) -> bool {
        matches!(self.quick.hover, Some(Target::Quick(h)) if q::takes_typing(h))
    }

    pub fn quick_pointer_left(&mut self) -> bool {
        self.quick.hover.take().is_some()
    }

    /// The wheel over the popup, DIPs down: whether it moved.
    pub fn quick_wheel(&mut self, dy: f32) -> bool {
        if self.quick.showing != Showing::Main {
            return false;
        }
        let before = self.quick.scroll;
        // The newest beams are at the foot: scrolling down goes toward them.
        self.quick.scroll = (self.quick.scroll - dy).clamp(0.0, self.quick.scroll_max);
        self.quick.scroll != before
    }

    /// A wheel notch, for shells that count notches.
    pub fn quick_notch() -> f32 {
        WHEEL
    }

    /// A click in the popup at `x`, `y` DIPs (a click never reads the
    /// clipboard: only Paste does).
    pub fn quick_click(&mut self, x: f32, y: f32) -> Vec<QuickDo> {
        match self.quick_hit(x, y) {
            Some(hit) => self.quick_press(hit),
            None => Vec::new(),
        }
    }

    pub(super) fn quick_press(&mut self, hit: Hit) -> Vec<QuickDo> {
        // Any press but the "…" itself puts its menu away.
        if !matches!(hit, Hit::More(_)) {
            self.quick.more = None;
        }
        let Some(net) = &self.quick.net else { return Vec::new() };
        let mut dos = Vec::new();
        match hit {
            Hit::Card(i) => match self.quick.cards.get(i).map(|c| (c.id, c.kind)) {
                Some((id, Kind::Text)) => self.toggle_open(id),
                Some((_, Kind::Link)) => return self.quick_press(Hit::Open(i)),
                Some((_, Kind::Picture)) => return self.quick_open_picture(i),
                _ => {}
            },
            Hit::More(i) => self.quick.more = if self.quick.more == Some(i) { None } else { Some(i) },
            Hit::ReadMore(i) => return self.quick_read(i),
            Hit::Retry(i) => self.quick_retry(i),
            Hit::Popup | Hit::List | Hit::CodeField | Hit::NameField | Hit::BeamNote => {}
            Hit::Show | Hit::Fab | Hit::NewBeam => return self.quick_open(false),
            Hit::Close => return self.quick_gone(),
            Hit::Devices => self.quick.showing = Showing::Devices,
            Hit::Back => return self.devices_back(),
            Hit::Field => {}
            Hit::Attach => dos.push(QuickDo::PickFile),
            Hit::Beam if self.quick.ready.is_none() => return self.quick_beam_draft(),
            Hit::Beam => return self.quick_beam(),
            Hit::Unstage => self.quick.ready = None,
            Hit::NotSecret => self.quick_not_secret(),
            Hit::UnstageAt(k) => self.quick_unstage(k),
            Hit::Tile(i, k) => return self.quick_open_tile(i, k),
            Hit::To(i) => self.quick.chosen = i,
            Hit::Copy(i) => dos.extend(self.quick_copy(i)),
            Hit::Open(i) => {
                if let Some(url) = self.quick.cards.get(i).map(|c| c.text.trim().to_string()) {
                    let mut dos = self.quick_gone();
                    dos.push(QuickDo::Url(url));
                    return dos;
                }
            }
            Hit::SaveNote(i) => self.quick_save(i),
            Hit::SaveFile(i) if self.quick.cards.get(i).is_some_and(|c| !c.rest.is_empty()) => {
                if let Some(files) = self.quick_group_files(i) {
                    dos.push(QuickDo::SaveAll(files));
                }
            }
            Hit::SaveFile(i) => {
                let card = self.quick.cards.get(i).map(|c| (c.id, c.text.clone()));
                if let Some((id, name)) = card {
                    match net.read(|e| e.picture(id)) {
                        Some(bytes) => dos.push(QuickDo::SaveFile { name, bytes }),
                        None => self.quick_say("The picture is still coming."),
                    }
                }
            }
            Hit::OpenNote => {
                if let Some((_, path, _)) = self.quick.saved.take() {
                    let mut dos = self.quick_gone();
                    self.open_path(&path);
                    dos.push(QuickDo::BringUp);
                    return dos;
                }
            }
            Hit::PairNow | Hit::NotNow => {
                self.quick.intro_seen = true;
                if let Some(p) = self.intro_file() {
                    let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
                    let _ = std::fs::write(p, b"1");
                }
                if hit == Hit::PairNow {
                    self.quick.showing = Showing::Devices;
                } else if self.quick.paired {
                    self.quick.showing = Showing::Main;
                } else {
                    return self.quick_gone();
                }
            }
            Hit::ShowCode => net.with(|e, _| e.show_code()),
            Hit::EnterCode => self.quick.typing = Some(String::new()),
            Hit::Pair => self.quick_pair(),
            Hit::Cancel => {
                self.quick.typing = None;
                net.with(|e, _| e.cancel_pairing());
            }
            Hit::Allow | Hit::Deny => net.with(|e, now| e.allow(hit == Hit::Allow, now)),
            Hit::Forget(i) => {
                if let Some(id) = self.quick.cards.get(i).map(|c| c.id) {
                    net.with(|e, _| e.forget(id));
                }
            }
            Hit::Remove(i) => {
                if let Some((id, ..)) = self.quick.devices.get(i).cloned() {
                    net.with(|e, _| e.remove(id));
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
        dos
    }

    /// A card's pictures beamed together, each its name and bytes, in
    /// order; None (and it says so) while any is still coming.
    fn quick_group_files(&mut self, i: usize) -> Option<Vec<(String, Vec<u8>)>> {
        let c = self.quick.cards.get(i)?;
        let ids: Vec<kolada_sync::ItemId> = std::iter::once(c.id).chain(c.rest.iter().map(|(id, _)| *id)).collect();
        let files: Option<Vec<(String, Vec<u8>)>> = self.quick.net.as_ref()?.read(|e| ids.iter().map(|id| Some((e.cards().iter().find(|k| k.id == *id)?.text.clone(), e.picture(*id)?))).collect());
        if files.is_none() {
            self.quick_say("The pictures are still coming.");
        }
        files
    }

    fn quick_copy(&mut self, i: usize) -> Vec<QuickDo> {
        if self.quick.cards.get(i).is_some_and(|c| !c.rest.is_empty()) {
            let id = self.quick.cards[i].id;
            return match self.quick_group_files(i) {
                Some(files) => {
                    self.quick.copied = Some((id, Instant::now()));
                    vec![QuickDo::CopyFiles(files)]
                }
                None => Vec::new(),
            };
        }
        let Some(c) = self.quick.cards.get(i) else { return Vec::new() };
        let (id, secret, picture) = (c.id, c.kind == Kind::Secret, c.kind == Kind::Picture);
        let Some(net) = &self.quick.net else { return Vec::new() };
        if picture {
            return match net.read(|e| e.picture(id)) {
                Some(bytes) => {
                    self.quick.copied = Some((id, Instant::now()));
                    vec![QuickDo::CopyPicture(bytes)]
                }
                None => {
                    self.quick_say("The picture is still coming.");
                    Vec::new()
                }
            };
        }
        let Some(text) = net.with(|e, _| e.copy(id)) else { return Vec::new() };
        if secret {
            self.quick.on_clipboard = Some((text.clone(), Instant::now()));
        }
        self.quick.copied = Some((id, Instant::now()));
        vec![QuickDo::Copy { text, secret }]
    }

    /// Saves a clip as a note in the vault; it leaves Quick Send.
    fn quick_save(&mut self, i: usize) {
        let Some(c) = self.quick.cards.get(i) else { return };
        if c.kind == Kind::Secret {
            return;
        }
        let id = c.id;
        if c.kind == Kind::Picture {
            let name = c.text.clone();
            return self.quick_save_picture(id, &name);
        }
        // A note: saved under its name, with the pictures that came with it.
        if c.kind == Kind::Text && !c.title.is_empty() {
            let (title, text) = (c.title.clone(), c.text.clone());
            return self.quick_save_note(id, &title, &text);
        }
        let (title, text) = match c.kind {
            Kind::Link if !c.title.is_empty() => (c.title.clone(), format!("{}\n\n{}\n", c.title, c.text.trim())),
            _ => (c.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("Beam").trim().chars().take(60).collect::<String>(), c.text.clone()),
        };
        let stem = note::file_stem_for(&title);
        let path = note::unique_path(&self.vault, &stem, None);
        if note::save(&path, &text, note::Format::default()).is_err() {
            self.quick_say("That couldn't be saved as a note.");
            return;
        }
        if let Some(net) = &self.quick.net {
            net.with(|e, _| {
                e.forget(id);
            });
        }
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.quick.saved = Some((name, path, Instant::now()));
    }

    /// A beamed note saved in the vault: its Markdown as it came, under its
    /// name, and each picture that came with it at the place the note
    /// embeds it from (one already there is kept), so it reads the same.
    fn quick_save_note(&mut self, id: kolada_sync::ItemId, title: &str, text: &str) {
        let Some(net) = &self.quick.net else { return };
        let pictures: Vec<(kolada_sync::ItemId, String, Option<Vec<u8>>)> = net.read(|e| e.note_pictures(id).into_iter().map(|c| (c.id, c.text.clone(), e.picture(c.id))).collect());
        if pictures.iter().any(|(_, _, b)| b.is_none()) {
            return self.quick_say("The note's pictures are still coming.");
        }
        let path = note::unique_path(&self.vault, &note::file_stem_for(title), None);
        if note::save(&path, text, note::Format::default()).is_err() {
            return self.quick_say("That couldn't be saved as a note.");
        }
        let dir = path.parent().map_or_else(|| self.vault.clone(), Path::to_path_buf);
        for (_, name, bytes) in &pictures {
            let (Some(to), Some(bytes)) = (kolada_ui::pictures::note_picture_path(&dir, name), bytes) else { continue };
            if !to.exists() {
                if let Some(parent) = to.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&to, bytes);
            }
        }
        net.with(|e, _| {
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
    fn quick_save_picture(&mut self, id: kolada_sync::ItemId, file: &str) {
        let Some(bytes) = self.quick.net.as_ref().and_then(|n| n.read(|e| e.picture(id))) else {
            self.quick_say("The picture is still coming.");
            return;
        };
        let path = Path::new(file);
        let stem = note::file_stem_for(&path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Picture".into()));
        let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into());
        let picture = (1..).map(|n| self.vault.join(if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} {n}.{ext}") })).find(|p| !p.exists()).unwrap_or_default();
        let note_path = note::unique_path(&self.vault, &stem, None);
        let shown = picture.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if std::fs::write(&picture, &bytes).is_err() || note::save(&note_path, &format!("![[{shown}]]\n"), note::Format::default()).is_err() {
            self.quick_say("That couldn't be saved as a note.");
            return;
        }
        if let Some(net) = &self.quick.net {
            net.with(|e, _| {
                e.forget(id);
            });
        }
        let name = note_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.quick.saved = Some((name, note_path, Instant::now()));
    }

    /// The name typed for this device: kept, sealed and sent to the group
    /// (an empty one goes back to the computer's).
    fn save_name(&mut self) {
        let (Some(name), Some(net)) = (self.quick.naming.take(), &self.quick.net) else { return };
        net.with(|e, _| e.rename_self(&name));
    }

    /// What this device is (`kolada_sync::names::MODELS`: a laptop, an
    /// iPad), as the shell knows it; told to the group.
    pub fn set_quick_model(&mut self, model: &str) {
        if let Some(net) = &self.quick.net {
            net.with(|e, _| e.set_model(model));
        }
        self.quick_refresh();
    }

    fn quick_pair(&mut self) {
        let Some(typed) = self.quick.typing.clone() else { return };
        let Some(net) = &self.quick.net else { return };
        match net.with(|e, _| e.enter_code(&typed)) {
            Ok(()) => self.quick.typing = None,
            Err(_) => self.quick_say("A code is 8 letters and digits, like F4FS-2RSA."),
        }
    }

    /// A letter typed in the popup: into the code field while pairing,
    /// else into the send area. Whether it changes.
    pub fn quick_char(&mut self, c: char) -> bool {
        if let Some(t) = &mut self.quick.naming {
            if c.is_control() || t.chars().count() >= kolada_sync::names::NAME_MOST {
                return false;
            }
            t.push(c);
            return true;
        }
        let Some(t) = &mut self.quick.typing else {
            if self.quick.showing != Showing::Main || self.quick.ready.is_some() || c.is_control() {
                return false;
            }
            self.quick.draft.push(c);
            return true;
        };
        let count = t.chars().filter(|c| *c != '-').count();
        match kolada_sync::code::normal(c) {
            Some(c) if count < 8 => {
                if count == 4 {
                    t.push('-');
                }
                t.push(c);
                true
            }
            _ => false,
        }
    }

    /// A key in the popup. `clip` reads the clipboard, only for Paste.
    pub fn quick_key(&mut self, key: QuickKey, clip: &dyn Fn() -> Option<Clip>) -> Vec<QuickDo> {
        if let Some(t) = &mut self.quick.naming {
            match key {
                QuickKey::Backspace => {
                    t.pop();
                }
                QuickKey::Enter => {
                    self.save_name();
                    self.quick_refresh();
                }
                QuickKey::Escape => self.quick.naming = None,
                QuickKey::Paste | QuickKey::NewLine => {}
            }
            return Vec::new();
        }
        if let Some(t) = &mut self.quick.typing {
            match key {
                QuickKey::Backspace => {
                    t.pop();
                    if t.ends_with('-') {
                        t.pop();
                    }
                }
                QuickKey::Enter => {
                    self.quick_pair();
                    self.quick_refresh();
                }
                QuickKey::Escape => self.quick.typing = None,
                QuickKey::Paste | QuickKey::NewLine => {}
            }
            return Vec::new();
        }
        let main = self.quick.showing == Showing::Main;
        match key {
            QuickKey::Escape if self.quick.showing == Showing::Devices => self.devices_back(),
            QuickKey::Escape => self.quick_gone(),
            QuickKey::Enter if main && self.quick.ready.is_some() => self.quick_beam(),
            QuickKey::Enter if main => self.quick_beam_draft(),
            QuickKey::NewLine if main && self.quick.ready.is_none() => {
                self.quick.draft.push('\n');
                Vec::new()
            }
            QuickKey::Backspace if main && self.quick.ready.is_none() => {
                self.quick.draft.pop();
                Vec::new()
            }
            // A picture, or a password, is shown ready to beam (a password
            // never in the field, where it would show); text goes into the
            // field.
            QuickKey::Paste if main => {
                let c = clip();
                let to_field = c.as_ref().is_some_and(|c| c.picture.is_none() && !kolada_sync::judge_secret(&c.text, c.concealed).masked());
                match c {
                    Some(c) if to_field && self.quick.ready.is_none() => self.quick.draft.push_str(&c.text),
                    c => {
                        self.quick_stage(c);
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// When the popup next changes by the clock alone: a sent beam's check,
    /// "Copied" back to Copy, a password leaving the clipboard, a code's
    /// countdown, a line said fading.
    pub fn quick_next_due(&self) -> Option<Instant> {
        let quick = &self.quick;
        let beamed = quick.marks.moving(Instant::now()).then(|| Instant::now() + FRAME);
        let copied = quick.copied.map(|(_, t)| t + COPIED).filter(|t| *t > Instant::now());
        let clear = quick.on_clipboard.as_ref().map(|(_, t)| *t + CLEAR);
        let said = quick.said.as_ref().map(|(_, t)| *t + SAID).filter(|t| *t > Instant::now());
        let count = (quick.shown && matches!(quick.pair, Some(PairState::Showing { .. }))).then(|| Instant::now() + Duration::from_secs(1));
        // A card opening or closing: the next frame.
        let moving = quick.opens.iter().any(|o| o.1 != if o.3 { 1.0 } else { 0.0 } || o.2 != 0.0).then(|| Instant::now() + FRAME);
        [beamed, copied, clear, said, count, moving].into_iter().flatten().min()
    }

    /// Steps what is due (`quick_next_due`).
    pub fn quick_tick(&mut self) -> Vec<QuickDo> {
        let mut dos = Vec::new();
        self.quick.marks.prune(Instant::now());
        if self.quick.on_clipboard.as_ref().is_some_and(|(_, t)| t.elapsed() >= CLEAR)
            && let Some((text, _)) = self.quick.on_clipboard.take()
        {
            dos.push(QuickDo::Clear(text));
        }
        if self.quick.shown && matches!(self.quick.pair, Some(PairState::Showing { .. })) {
            self.quick_refresh();
        }
        self.step_opens();
        dos
    }

    /// Steps the opening and closing cards' springs to now.
    fn step_opens(&mut self) {
        let now = Instant::now();
        let dt = self.quick.stepped.map_or(1.0 / 60.0, |t| now.duration_since(t).as_secs_f32()).min(0.05);
        self.quick.stepped = Some(now);
        for o in &mut self.quick.opens {
            (o.1, o.2) = q::spring(o.1, o.2, if o.3 { 1.0 } else { 0.0 }, dt);
        }
        self.quick.opens.retain(|o| o.3 || o.1 != 0.0 || o.2 != 0.0);
    }

    /// A tap on a text card: it opens (or closes) on its spring.
    fn toggle_open(&mut self, id: kolada_sync::ItemId) {
        match self.quick.opens.iter_mut().find(|o| o.0 == id) {
            Some(o) => o.3 = !o.3,
            None => self.quick.opens.push((id, 0.0, 0.0, true)),
        }
        self.quick.stepped = Some(Instant::now());
    }

    /// A picture card, tapped: the picture full size, in the system's viewer.
    /// A picture of a card holding several, opened (0 is the first).
    fn quick_open_tile(&mut self, i: usize, k: usize) -> Vec<QuickDo> {
        if k == 0 {
            return self.quick_open_picture(i);
        }
        let Some((id, name)) = self.quick.cards.get(i).and_then(|c| c.rest.get(k - 1)).map(|(id, _)| *id).and_then(|id| self.quick.net.as_ref().map(|n| (id, n.read(|e| e.cards().iter().find(|c| c.id == id).map(|c| c.text.clone()).unwrap_or_default())))) else { return Vec::new() };
        match self.quick.net.as_ref().and_then(|n| n.read(|e| e.picture(id))) {
            Some(bytes) => vec![QuickDo::OpenFile { name, bytes }],
            None => {
                self.quick_say("The picture is still coming.");
                Vec::new()
            }
        }
    }

    fn quick_open_picture(&mut self, i: usize) -> Vec<QuickDo> {
        let Some((id, name)) = self.quick.cards.get(i).map(|c| (c.id, c.text.clone())) else { return Vec::new() };
        match self.quick.net.as_ref().and_then(|n| n.read(|e| e.picture(id))) {
            Some(bytes) => vec![QuickDo::OpenFile { name, bytes }],
            None => {
                self.quick_say("The picture is still coming.");
                Vec::new()
            }
        }
    }

    /// "Read more": the whole beam as a fresh page in the editor, not
    /// written to the vault unless it is edited (then it is saved as any new
    /// note is) or saved as a note.
    fn quick_read(&mut self, i: usize) -> Vec<QuickDo> {
        let Some(text) = self.quick.cards.get(i).map(|c| c.text.clone()) else { return Vec::new() };
        let mut dos = self.quick_gone();
        self.open_unwritten(text);
        dos.push(QuickDo::BringUp);
        dos
    }

    /// Where the last frame drew `hit`, DIPs (for a shell's scripted checks).
    pub fn quick_place_of(&self, hit: Hit) -> Option<scene::Rect> {
        self.quick.hits.iter().find(|(_, t)| *t == Target::Quick(hit)).map(|(r, _)| *r)
    }

    /// What pairing shows: the code while one is up, whether a device asks
    /// to be allowed, whether this one is paired; and how many clips there are.
    pub fn quick_state(&self) -> (Option<String>, bool, bool, usize) {
        let code = match &self.quick.pair {
            Some(PairState::Showing { code, .. }) => Some(code.clone()),
            _ => None,
        };
        (code, matches!(self.quick.pair, Some(PairState::Asks { .. })), self.quick.paired, self.quick.cards.len())
    }

    /// The menu bar's (tray's) line for it, with its keys when held.
    pub fn quick_menu_label(&self, keys: Option<&str>) -> Option<String> {
        self.quick.net.as_ref()?;
        Some(match keys {
            Some(k) => format!("Beam\t{k}"),
            None => "Beam".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::Services;
    use kolada_sync::testing::{T, deliver, deliver_at, pair};
    use kolada_sync::Engine;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kolada-quick-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("vault")).unwrap();
        d
    }

    /// A model on a paired engine (its partner returned), the first use seen
    /// or not, data kept under `dir`.
    fn paired(dir: &PathBuf, seen: bool) -> (Model, Engine) {
        let mut a = Engine::open("https://sync.kolada.app", "desktop", "Desk PC", None, None);
        let mut b = Engine::open("https://sync.kolada.app", "desktop", "Laptop", None, None);
        pair(&mut a, "desktop", &mut b, "desktop");
        a.events();
        b.events();
        let mut m = Model::with_services(Services::none(), dir.join("vault"), true);
        m.data = Some(dir.join("data"));
        if seen {
            std::fs::create_dir_all(dir.join("data").join("sync")).unwrap();
            std::fs::write(dir.join("data").join("sync").join("intro.seen"), b"1").unwrap();
        }
        let s = Services::none();
        m.quick_with(Net::with_engine(a, s.socket.clone(), s.keys.clone(), s.web.clone(), Arc::new(|| {})));
        (m, b)
    }

    fn clip(text: &str) -> Option<Clip> {
        Some(Clip { text: text.into(), ..Default::default() })
    }

    /// "Beam this note" sends the note with each picture it embeds; the
    /// other device shows one card ("With 2 pictures"), and Save as note
    /// writes the note and its pictures where it embeds them, so it reads
    /// the same. A picture already there is kept.
    #[test]
    fn a_beamed_note_brings_its_pictures_and_saves_them_back() {
        let dir = temp("note-pictures");
        let (mut m, b) = paired(&dir, true);
        let vault = dir.join("vault");
        std::fs::create_dir_all(vault.join("attachments")).unwrap();
        std::fs::create_dir_all(vault.join("maps")).unwrap();
        // PNGs by their first bytes: the signature, then the IHDR chunk with the size.
        let png = |w: u8, h: u8, fill: u8| -> Vec<u8> { [&[0x89u8, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13][..], b"IHDR", &[0, 0, 0, w, 0, 0, 0, h], &[fill; 300]].concat() };
        let (photo, route) = (png(4, 3, 7), png(8, 6, 9));
        std::fs::write(vault.join("attachments").join("photo.png"), &photo).unwrap();
        std::fs::write(vault.join("maps").join("route.png"), &route).unwrap();
        std::fs::write(vault.join("Trip.md"), "# Trip
![[photo.png]]
![the way](maps/route.png)
").unwrap();
        m.open("Trip.md");
        assert!(m.quick_beam_note().is_empty());
        // What went: the note under its name, and its two pictures, tied to it.
        let (title, names) = m.quick.net.as_ref().unwrap().read(|e| {
            let note = e.cards().into_iter().find(|c| c.mine && c.kind == SendKind::Text).cloned().expect("the note's card");
            let names: Vec<(String, Option<Vec<u8>>)> = e.note_pictures(note.id).iter().map(|c| (c.text.clone(), e.picture(c.id))).collect();
            (note.title, names)
        });
        assert_eq!(title, "Trip");
        assert_eq!(names, vec![("photo.png".to_string(), Some(photo.clone())), ("maps/route.png".to_string(), Some(route.clone()))]);

        // The other way: a note from the paired device, with its pictures.
        let other = dir.join("other");
        let (mut r, mut b) = paired(&other, true);
        std::fs::create_dir_all(other.join("vault").join("maps")).unwrap();
        // A picture already there with the same name is kept.
        std::fs::write(other.join("vault").join("maps").join("route.png"), b"theirs").unwrap();
        let pic = |name: &str, bytes: &Vec<u8>| kolada_sync::NotePicture { name: name.into(), mime: "image/png".into(), bytes: bytes.clone(), width: 4, height: 3, thumb: vec![] };
        let text = "# Trip
![[photo.png]]
![the way](maps/route.png)
".to_string();
        b.send(Outgoing::Note { title: "Trip".into(), text, pictures: vec![pic("photo.png", &photo), pic("maps/route.png", &route)] }, T).unwrap();
        let mut seq = 0;
        let (items, up) = kolada_sync::testing::items_from(&mut b, &mut seq);
        assert_eq!(items.len(), 3, "the note and its two pictures");
        let net = r.quick.net.as_ref().unwrap();
        net.with(|e, now| {
            for f in &items {
                e.frame(kolada_sync::Channel::Group, f, now);
            }
            for (blob, body) in &up {
                e.got(*blob, Some(body.clone()));
            }
        });
        net.wait_for_events();
        r.quick_events();
        r.quick_refresh();
        assert_eq!(r.quick.cards.len(), 1, "one card: the note");
        assert_eq!(r.quick.cards[0].goes, "With 2 pictures");
        r.quick_save(0);
        let saved = other.join("vault");
        assert_eq!(std::fs::read_to_string(saved.join("Trip.md")).unwrap().trim_end(), "# Trip
![[photo.png]]
![the way](maps/route.png)");
        assert_eq!(std::fs::read(saved.join("photo.png")).unwrap(), photo, "beside the note, where ![[photo.png]] finds it");
        assert_eq!(std::fs::read(saved.join("maps").join("route.png")).unwrap(), b"theirs", "kept");
        r.quick_refresh();
        assert!(r.quick.cards.is_empty(), "the note and its pictures leave Beam once saved");
        assert!(r.quick.net.as_ref().unwrap().read(|e| e.cards().is_empty()));
    }

    /// The Mac's Beam keys are Option+Z, as Alt+Z on Windows; where
    /// another app holds them the shell falls back to the old
    /// Ctrl+Option+Shift+Z (choice 1), and the window says the ones held.
    #[test]
    fn the_macs_keys_are_option_z_or_the_ones_held() {
        let dir = temp("mac-keys");
        let (mut m, _) = paired(&dir, true);
        m.platform = scene::Platform::Mac;
        assert_eq!(m.quick_keys_choice(), 0, "Option+Z by default, Macs from before included");
        assert_eq!(m.beam_view().unwrap().keys, "\u{2325}Z");
        m.set_quick_keys_choice(1);
        assert_eq!(m.beam_view().unwrap().keys, "\u{2303}\u{2325}\u{21E7}Z");
        m.set_quick_keys_choice(2);
        assert_eq!(m.beam_view().unwrap().keys, "", "off: none shown");
    }

    /// The server says it has `id` (its "sent" frame), as it would.
    fn ack(m: &Model, id: kolada_sync::ItemId) {
        let f = kolada_sync::frame::Frame::new("sent").with("id", kolada_sync::crypto::hex(&id)).with("seq", 1).encode();
        let net = m.quick.net.as_ref().unwrap();
        net.with(|e, now| e.frame(kolada_sync::Channel::Group, &f, now));
        net.wait_for_events();
    }

    fn none() -> Option<Clip> {
        None
    }

    /// Clicks the control `hit` as last drawn.
    fn press(m: &mut Model, hit: Hit) -> Vec<QuickDo> {
        m.quick_frame(q::W as u32, q::H as u32, 1.0);
        // What a card's "…" menu holds: the pointer over the card, its "…",
        // then the item.
        if let Hit::Open(i) | Hit::SaveFile(i) | Hit::SaveNote(i) | Hit::Forget(i) = hit
            && m.quick_place_of(hit).is_none()
            && let Some(card) = m.quick_place_of(Hit::Card(i))
        {
            m.quick_pointer_moved((card.left + card.right) / 2.0, (card.top + card.bottom) / 2.0);
            m.quick_frame(q::W as u32, q::H as u32, 1.0);
            let more = m.quick_place_of(Hit::More(i)).expect("its \u{2026} under the pointer");
            m.quick_click((more.left + more.right) / 2.0, (more.top + more.bottom) / 2.0);
            m.quick_frame(q::W as u32, q::H as u32, 1.0);
        }
        let r = m.quick.hits.iter().find(|(_, t)| *t == Target::Quick(hit)).map(|(r, _)| *r).unwrap_or_else(|| panic!("{hit:?} is drawn"));
        m.quick_click((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    }

    #[test]
    fn the_keys_show_what_was_copied_and_enter_beams_it_then_it_goes() {
        let dir = temp("keys");
        let (mut m, _) = paired(&dir, true);
        assert_eq!(m.quick_keys(false, &|| clip("https://kolada.app/docs")), vec![QuickDo::Show { height: q::H }]);
        assert!(m.quick_shown());
        assert_eq!(m.quick.showing, Showing::Main);
        assert!(m.quick.ready.as_ref().is_some_and(|r| r.kind == Kind::Link), "a link, shown first");
        m.quick_frame(q::W as u32, q::H as u32, 1.0);
        assert!(m.quick.hits.iter().any(|(_, t)| *t == Target::Quick(Hit::Beam)));
        // Enter beams it to every device; the popup stays on the beams,
        // the newest at the foot.
        assert!(m.quick_key(QuickKey::Enter, &none).is_empty(), "no Beamed, nothing put away");
        assert!(m.quick_shown());
        assert_eq!(m.quick.showing, Showing::Main);
        assert!(m.quick.ready.is_none());
        assert_eq!(m.quick.cards.len(), 1);
        assert!(m.quick.cards[0].mine);
    }

    /// A beam sent from the popup: pending (faint) until the server has it,
    /// then its check, which pops in, holds and goes; frames are due only
    /// while it shows. A failure shows red, and its click sends it again.
    #[test]
    fn a_sent_beam_waits_then_shows_its_check_then_the_check_goes() {
        let dir = temp("sent");
        let (mut m, _) = paired(&dir, true);
        m.quick_open(false);
        m.quick.draft = "Gate B, row 14".into();
        m.quick_key(QuickKey::Enter, &none);
        assert!(m.quick.draft.is_empty(), "the field clears and stays");
        let id = m.quick.cards[0].id;
        let at = |m: &Model| m.quick.marks.card(id, Instant::now(), true);
        assert_eq!(at(&m), q::Sent::Pending, "queued: faint");
        assert!(m.quick_next_due().is_none_or(|t| t > Instant::now() + Duration::from_millis(100)), "nothing moves while it waits");
        ack(&m, id);
        m.quick_events();
        assert!(matches!(at(&m), q::Sent::Check { .. }), "acknowledged: the check");
        assert!(m.quick_next_due().is_some_and(|t| t <= Instant::now() + Duration::from_millis(50)), "frames while it shows");
        m.quick_frame(q::W as u32, q::H as u32, 1.0);
        // Its time over, it goes; nothing more is due for it.
        m.quick.marks.acked(id, Instant::now() - Duration::from_secs(2));
        assert_eq!(at(&m), q::Sent::None, "and gone");
        m.quick_tick();
        assert!(m.quick_next_due().is_none_or(|t| t > Instant::now() + Duration::from_millis(100)));
        // A failure: red, and a click on it beams it again.
        m.quick.draft = "Door code 4931".into();
        m.quick_key(QuickKey::Enter, &none);
        let failed = m.quick.cards.iter().find(|c| c.text == "Door code 4931").unwrap().id;
        m.quick.marks.failed(failed);
        assert_eq!(m.quick.marks.card(failed, Instant::now(), true), q::Sent::Failed);
        let i = m.quick.cards.iter().position(|c| c.id == failed).unwrap();
        m.quick_frame(q::W as u32, q::H as u32, 1.0);
        assert!(m.quick_place_of(Hit::Retry(i)).is_some(), "its red mark takes a click");
        m.quick_retry(i);
        assert!(!m.quick.cards.iter().any(|c| c.id == failed), "the failed one goes");
        let again = m.quick.cards.iter().find(|c| c.text == "Door code 4931").expect("sent again").id;
        assert_eq!(m.quick.marks.card(again, Instant::now(), true), q::Sent::Pending);
    }

    /// The server's word on a beam, as the engine takes it.
    fn server(m: &Model, f: kolada_sync::frame::Frame) {
        let net = m.quick.net.as_ref().unwrap();
        net.with(|e, now| e.frame(kolada_sync::Channel::Group, &f.encode(), now));
        net.wait_for_events();
    }

    /// A beam that expired on the server before any device had it: its card
    /// says "Didn't arrive" under a red mark, after a restart too, and a
    /// click on the mark sends it again as a new beam in its place.
    #[test]
    fn a_beam_that_didnt_arrive_says_so_and_its_mark_sends_it_again() {
        let dir = temp("lost");
        let (mut m, _) = paired(&dir, true);
        m.quick_open(false);
        m.quick.draft = "Gate B, row 14".into();
        m.quick_key(QuickKey::Enter, &none);
        let id = m.quick.cards[0].id;
        ack(&m, id);
        m.quick_events();
        server(&m, kolada_sync::frame::Frame::new("lost").with("ids", kolada_sync::crypto::hex(&id)));
        m.quick_events();
        assert_eq!(m.quick.said.as_ref().map(|(s, _)| s.as_str()), Some("A beam didn't arrive in 7 days."));
        let c = m.quick.cards.iter().find(|c| c.id == id).unwrap();
        assert_eq!(c.goes, q::LOST);
        // The mark comes from the card itself, as it would after a restart.
        m.quick.marks.forget(id);
        m.quick_refresh();
        assert_eq!(m.quick.marks.card(id, Instant::now(), true), q::Sent::Failed);
        let i = m.quick.cards.iter().position(|c| c.id == id).unwrap();
        m.quick_frame(q::W as u32, q::H as u32, 1.0);
        assert!(m.quick_place_of(Hit::Retry(i)).is_some(), "its red mark takes a click");
        m.quick_retry(i);
        assert!(!m.quick.cards.iter().any(|c| c.id == id), "the lost one goes");
        let again = m.quick.cards.iter().find(|c| c.text == "Gate B, row 14").expect("sent again");
        assert!(again.goes.is_empty());
        assert_eq!(m.quick.marks.card(again.id, Instant::now(), true), q::Sent::Pending);
    }

    /// A refusal the user can act on is said, beside the red mark.
    #[test]
    fn a_refused_beam_says_why() {
        let dir = temp("refused");
        let (mut m, _) = paired(&dir, true);
        m.quick_open(false);
        for (why, says) in [("slow", "Too many beams at once. Try again in a minute."), ("limit", "That's the most beaming for today. Try again tomorrow.")] {
            m.quick.draft = format!("Note {why}");
            m.quick_key(QuickKey::Enter, &none);
            let id = m.quick.cards.iter().find(|c| c.text == format!("Note {why}")).unwrap().id;
            server(&m, kolada_sync::frame::Frame::new("error").with("id", kolada_sync::crypto::hex(&id)).with("error", why).with("retry", 60));
            m.quick_events();
            assert_eq!(m.quick.marks.card(id, Instant::now(), true), q::Sent::Failed);
            assert_eq!(m.quick.said.as_ref().map(|(s, _)| s.as_str()), Some(says));
        }
    }

    /// Pairing or the devices only on first use, or with nothing paired;
    /// after that, and after a restart, the popup opens on the send area and
    /// the clips, with Devices behind its chip.
    #[test]
    fn once_paired_it_opens_on_the_clips_after_a_restart_too() {
        let dir = temp("restart");
        let (mut m, _) = paired(&dir, false);
        m.quick_open(false);
        assert_eq!(m.quick.showing, Showing::FirstUse, "the very first time");
        press(&mut m, Hit::NotNow);
        assert_eq!(m.quick.showing, Showing::Main);
        let keys = m.quick.net.as_ref().unwrap().read(|e| e.keys_blob());
        for _ in 0..3 {
            m.quick_key(QuickKey::Escape, &none);
            m.quick_open(false);
            assert_eq!(m.quick.showing, Showing::Main, "every time after");
        }
        // A restart: a new model on the same data, the engine from the kept keys.
        let mut again = Model::with_services(Services::none(), dir.join("vault"), true);
        again.data = Some(dir.join("data"));
        let s = Services::none();
        let engine = kolada_sync::Engine::open("https://sync.kolada.app", "desktop", "Desk PC", None, Some(&keys));
        assert!(engine.paired());
        again.quick_with(Net::with_engine(engine, s.socket.clone(), s.keys.clone(), s.web.clone(), Arc::new(|| {})));
        assert_eq!(again.quick_keys(false, &|| clip("x")), vec![QuickDo::Show { height: q::H }]);
        assert_eq!(again.quick.showing, Showing::Main, "after a restart too");
        again.quick_frame(q::W as u32, q::H as u32, 1.0);
        assert!(again.quick_place_of(Hit::Devices).is_some(), "Devices behind its chip");
        // The chip opens the devices here, in the popup; Esc comes back.
        press(&mut again, Hit::Devices);
        assert_eq!(again.quick.showing, Showing::Devices);
        assert!(!again.settings.open, "not sent off to Settings");
        again.quick_key(QuickKey::Escape, &none);
        assert_eq!(again.quick.showing, Showing::Main, "Esc: the beams again");
        again.quick_open(false);
        assert_eq!(again.quick.showing, Showing::Main, "and on the clips the next time");
    }

    #[test]
    fn escape_or_a_click_elsewhere_puts_it_away() {
        let dir = temp("away");
        let (mut m, _) = paired(&dir, true);
        m.quick_keys(false, &|| clip("Gate B, row 14"));
        assert_eq!(m.quick_key(QuickKey::Escape, &none), vec![QuickDo::Hide]);
        assert!(!m.quick_shown());
        m.quick_keys(false, &|| clip("Gate B, row 14"));
        assert_eq!(m.quick_deactivated(), vec![QuickDo::Hide]);
        // An empty clipboard says so, and nothing is staged.
        m.quick_keys(false, &|| None);
        assert!(m.quick.ready.is_none());
        assert!(m.quick.said.is_some());
    }

    #[test]
    fn beam_at_once_beams_without_showing_what_was_copied() {
        let dir = temp("once");
        let (mut m, _) = paired(&dir, true);
        m.session.quick_beam = 1;
        assert!(m.quick_keys(false, &|| clip("Door code 4931")).is_empty(), "no popup");
        assert!(!m.quick_shown());
        // Its check shows by the window's Beam button once the server has it.
        let id = m.quick.cards[0].id;
        assert!(m.beam_view().unwrap().check.is_none());
        ack(&m, id);
        m.quick_events();
        assert!(m.beam_view().unwrap().check.is_some(), "by the button");
        assert!(m.wants_frames(), "the window draws it");
    }

    #[test]
    fn the_password_keys_or_a_concealed_copy_beam_a_password() {
        let dir = temp("secret");
        let (mut m, _) = paired(&dir, true);
        m.quick_keys(true, &|| clip("plain words"));
        assert_eq!(m.quick.ready.as_ref().map(|r| r.kind), Some(Kind::Secret));
        m.quick_keys(false, &|| Some(Clip { text: "plain words".into(), concealed: true, picture: None }));
        assert_eq!(m.quick.ready.as_ref().map(|r| r.kind), Some(Kind::Secret));
        m.quick_key(QuickKey::Enter, &none);
        assert_eq!(m.quick.cards[0].kind, Kind::Secret, "it went as a password");
    }

    /// Something that may be a password shows masked with "Not a password";
    /// a click on it and it beams as the text it is. Beamed at once (as
    /// Settings may say), it waits in the popup for that choice instead.
    #[test]
    fn maybe_a_password_waits_masked_and_not_a_password_beams_it_as_text() {
        let dir = temp("unsure");
        let (mut m, _) = paired(&dir, true);
        m.quick_keys(false, &|| clip("x7kp2qm9vld8wz"));
        assert!(m.quick.ready.as_ref().is_some_and(|r| r.kind == Kind::Secret && r.unsure), "masked, unsure");
        press(&mut m, Hit::NotSecret);
        assert!(m.quick.ready.as_ref().is_some_and(|r| r.kind == Kind::Text && !r.unsure), "now text");
        m.quick_key(QuickKey::Enter, &none);
        assert_eq!(m.quick.cards[0].kind, Kind::Text, "it went as text");
        // A sure one has no way out.
        m.quick_keys(false, &|| clip("9fG!kQ2#vB7z"));
        assert!(m.quick.ready.as_ref().is_some_and(|r| r.kind == Kind::Secret && !r.unsure));
        m.quick_frame(q::W as u32, q::H as u32, 1.0);
        assert!(m.quick_place_of(Hit::NotSecret).is_none());
        // Beamed at once: an unsure one is shown, not sent.
        m.quick_gone();
        m.session.quick_beam = 1;
        let cards = m.quick.cards.len();
        m.quick_keys(false, &|| clip("x7kp2qm9vld8wz"));
        assert!(m.quick.shown && m.quick.ready.as_ref().is_some_and(|r| r.unsure), "waiting for the choice");
        assert_eq!(m.quick.cards.len(), cards, "nothing beamed");
        // Pasted, it never lands in the field, where it would show.
        m.quick_gone();
        m.session.quick_beam = 0;
        m.quick_keys(false, &none);
        m.quick.ready = None;
        m.quick_key(QuickKey::Paste, &|| clip("x7kp2qm9vld8wz"));
        assert!(m.quick.draft.is_empty() && m.quick.ready.as_ref().is_some_and(|r| r.unsure));
    }

    #[test]
    fn the_first_use_comes_first_and_not_now_goes_to_the_clips() {
        let dir = temp("first");
        let (mut m, _) = paired(&dir, false);
        let read = std::cell::Cell::new(false);
        let watched = || {
            read.set(true);
            clip("x")
        };
        assert_eq!(m.quick_keys(false, &watched), vec![QuickDo::Show { height: q::H }]);
        assert_eq!(m.quick.showing, Showing::FirstUse);
        assert!(!read.get() && m.quick.ready.is_none(), "the clipboard is not read before the steps are seen");
        press(&mut m, Hit::NotNow);
        assert_eq!(m.quick.showing, Showing::Main, "paired: the clips");
        assert!(dir.join("data").join("sync").join("intro.seen").exists(), "seen once");
        assert!(m.quick_shown());
    }

    #[test]
    fn an_unpaired_device_opens_on_pairing_and_types_a_code() {
        let dir = temp("unpaired");
        let mut m = Model::with_services(Services::none(), dir.join("vault"), true);
        m.data = Some(dir.join("data"));
        let s = Services::none();
        // A server that never answers: pairing waits, as it would for one far
        // away (a stand-in that refused at once raced the checks below).
        struct Silent;
        impl crate::services::Socket for Silent {
            fn connect(&self, _: &str, _: &[(String, String)]) -> Result<Box<dyn crate::services::Link>, (u16, String)> {
                std::thread::sleep(Duration::from_secs(3600));
                Err((0, String::new()))
            }
        }
        m.quick_with(Net::with_engine(Engine::open("https://sync.kolada.app", "desktop", "Mac", None, None), Arc::new(Silent), s.keys.clone(), s.web.clone(), Arc::new(|| {})));
        m.quick.intro_seen = true;
        // Nothing paired: the keys open the popup on its devices, where a
        // code is typed; Esc stops the pairing and the popup goes.
        assert_eq!(m.quick_keys(false, &|| clip("x")), vec![QuickDo::Show { height: q::H }]);
        assert_eq!(m.quick.showing, Showing::Devices);
        assert!(!m.settings.open);
        press(&mut m, Hit::EnterCode);
        for c in "f4fs2rsa".chars() {
            m.quick_char(c);
        }
        assert_eq!(m.quick.typing.as_deref(), Some("F4FS-2RSA"));
        m.quick_key(QuickKey::Enter, &none);
        assert_eq!(m.quick.pair, Some(PairState::Waiting), "the code goes to the engine");
        assert_eq!(m.quick_key(QuickKey::Escape, &none), vec![QuickDo::Hide]);
        m.quick_refresh();
        assert_ne!(m.quick.pair, Some(PairState::Waiting), "Esc stopped it");
        // The same pairing in Settings > Devices.
        m.settings.open = true;
        let click = |m: &mut Model, hit: Hit| {
            m.frame(1600, 1200, 1.0);
            let r = m.hits.iter().find(|(_, t)| *t == Target::Quick(hit)).map(|(r, _)| *r).unwrap_or_else(|| panic!("{hit:?} in Settings"));
            m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false);
        };
        click(&mut m, Hit::EnterCode);
        // Typed in Settings: the code, not the note.
        let note = m.editor.text().to_string();
        m.insert("f4fs2rsa!");
        assert_eq!(m.quick.typing.as_deref(), Some("F4FS-2RSA"), "upper case, the dash put in, nothing past 8");
        m.backspace(false);
        assert_eq!(m.quick.typing.as_deref(), Some("F4FS-2RS"));
        m.insert("A");
        m.newline(false);
        assert_eq!(m.editor.text(), note, "the note was not typed in");
        assert!(m.quick.typing.is_none(), "the code goes to the engine");
        assert_eq!(m.quick.pair, Some(PairState::Waiting));
        // Esc in the field leaves the field, not Settings.
        click(&mut m, Hit::Cancel);
        click(&mut m, Hit::EnterCode);
        assert!(m.escape());
        assert!(m.quick.typing.is_none() && m.settings.open);
    }

    #[test]
    fn an_arrival_is_a_notification_while_the_popup_is_away() {
        let dir = temp("arrive");
        let (mut m, mut b) = paired(&dir, true);
        let id = b.send(Outgoing::Text("Gate B, row 14".into()), T).unwrap();
        let net = m.quick.net.as_ref().unwrap();
        net.with(|e, _| {
            assert!(deliver(&mut b, e, 1));
        });
        // The thread hands what the engine says over, and wakes the shell.
        net.wait_for_events();
        let dos = m.quick_events();
        assert!(m.quick.cards.iter().any(|c| c.id == id));
        assert_eq!(dos, vec![QuickDo::Notify { title: "From Laptop".into(), text: "Gate B, row 14".into() }]);
        assert_eq!(m.quick.cards.len(), 1);
    }

    #[test]
    fn arrivals_count_on_the_side_menus_button_until_the_popup_shows_them() {
        let dir = temp("badge");
        let mut off = Model::with_services(Services::none(), dir.join("vault"), true);
        assert_eq!(off.quick_badge(), None, "no button while Beam is off");
        let (mut m, mut b) = paired(&dir, true);
        assert_eq!(m.quick_badge(), Some(0));
        let net = m.quick.net.as_ref().unwrap();
        for (seq, text) in ["Gate B, row 14", "Door code 4931"].into_iter().enumerate() {
            b.send(Outgoing::Text(text.into()), T);
            net.with(|e, _| assert!(deliver(&mut b, e, seq as u64 + 1)));
        }
        net.wait_for_events();
        m.quick_events();
        assert_eq!(m.quick_badge(), Some(2));
        // The button asks the shell to open the popup; showing the clips sees them.
        m.frame(1600, 1000, 1.0);
        let r = m.hits.iter().find(|(_, t)| *t == Target::Quick(Hit::Show)).map(|(r, _)| *r).expect("the side menu's button");
        m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false);
        assert_eq!(m.take_requests(), vec![super::super::settings::Request::QuickOpen]);
        m.quick_open(false);
        assert_eq!(m.quick_badge(), Some(0));
        off.frame(800, 600, 1.0);
        assert!(!off.hits.iter().any(|(_, t)| *t == Target::Quick(Hit::Show)));
    }

    /// What the watcher saw reaches note sync as paths in the vault; a
    /// folder (no extension), or a path outside the vault, as a fresh look.
    #[test]
    fn the_watchers_changes_reach_note_sync_as_vault_paths() {
        let v = Path::new("/notes");
        let c = |p: &str| crate::services::Change { path: PathBuf::from(p), flags: 0 };
        assert_eq!(sync_paths(v, &[c("/notes/Plan.md"), c("/notes/Work/Ideas.md")]), Some(vec!["Plan.md".to_string(), "Work/Ideas.md".to_string()]));
        assert_eq!(sync_paths(v, &[c("/notes/Plan.md"), c("/notes/Work")]), None, "a folder");
        assert_eq!(sync_paths(v, &[c("/elsewhere/x.md")]), None);
    }

    /// Clips stay until removed: a clip's x takes it from this device's
    /// history, and the others stay.
    #[test]
    fn a_clips_x_removes_it_and_the_rest_stay() {
        let dir = temp("forget");
        let (mut m, _) = paired(&dir, true);
        for text in ["one", "two"] {
            m.quick_open(false);
            m.quick.draft = text.into();
            m.quick_key(QuickKey::Enter, &none);
        }
        m.quick_open(false);
        assert_eq!(m.quick.cards.len(), 2);
        press(&mut m, Hit::Forget(0));
        assert_eq!(m.quick.cards.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["one"]);
    }

    /// A long beam: a tap opens its card on a spring (and another closes
    /// it); past its limit, Read more opens it as a fresh page in the
    /// editor, which writes nothing to the vault until it is edited.
    #[test]
    fn a_tap_opens_a_long_card_and_read_more_opens_it_unwritten() {
        let dir = temp("open");
        let (mut m, _) = paired(&dir, true);
        let long: String = (1..=40).map(|n| format!("Line {n} of a long beam, the agenda for Thursday.")).collect::<Vec<_>>().join("\n");
        m.quick_open(false);
        m.quick.draft = long.clone();
        m.quick_key(QuickKey::Enter, &none);
        m.quick_open(false);
        press(&mut m, Hit::Card(0));
        assert!(m.quick_next_due().is_some(), "it moves");
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(10));
            m.quick_tick();
        }
        assert_eq!(m.quick.opens.first().map(|o| (o.1, o.3)), Some((1.0, true)), "open, and settled");
        assert!(m.quick_next_due().is_none_or(|t| t > Instant::now() + Duration::from_millis(100)), "and still");
        let notes = |m: &Model| std::fs::read_dir(&m.vault).map(|d| d.count()).unwrap_or(0);
        let before = notes(&m);
        let dos = press(&mut m, Hit::ReadMore(0));
        assert!(dos.contains(&QuickDo::BringUp));
        assert_eq!(m.editor.text(), long, "the whole beam, in the editor");
        assert!(m.session.note.is_empty() && !m.dirty, "a fresh page, not a file");
        m.save_flush();
        assert_eq!(notes(&m), before, "nothing written to the vault");
        // A tap on it again closes it.
        m.quick_open(false);
        press(&mut m, Hit::Card(0));
        assert_eq!(m.quick.opens.first().map(|o| o.3), Some(false));
    }

    /// One faint line a group: who and when, on its first card; a group is
    /// one sender's beams, each within 5 minutes of the one before.
    #[test]
    fn who_and_when_show_once_a_group() {
        let dir = temp("groups");
        let (mut m, mut b) = paired(&dir, true);
        let net = m.quick.net.as_ref().unwrap();
        for (seq, (text, at)) in [("one", T), ("two", T + 60_000), ("three", T + 20 * 60_000)].into_iter().enumerate() {
            b.send(Outgoing::Text(text.into()), at);
            net.with(|e, _| assert!(deliver_at(&mut b, e, seq as u64 + 1, at)));
        }
        net.wait_for_events();
        m.quick_events();
        let from: Vec<(&str, bool)> = m.quick.cards.iter().map(|c| (c.text.as_str(), !c.from.is_empty())).collect();
        assert_eq!(from, [("three", true), ("two", false), ("one", true)], "newest first; 'two' joins 'one''s group");
        assert!(m.quick.cards[2].from.starts_with("Laptop \u{00B7} "));
    }

    /// The window's Beam button: just after launch it opens the popup;
    /// once the note is written in, a menu offers this note or a new beam,
    /// and "Beam this note" beams its text under its name. A press
    /// elsewhere only closes the menu; the button fades while typing.
    #[test]
    fn beams_button_offers_the_note_once_it_is_written_in() {
        let dir = temp("fab");
        let (mut m, _) = paired(&dir, true);
        let press = |m: &mut Model, hit: Hit| {
            m.frame(1600, 1000, 1.0);
            let r = m.hits.iter().find(|(_, t)| *t == Target::Quick(hit)).map(|(r, _)| *r).expect("on screen");
            m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false);
        };
        press(&mut m, Hit::Fab);
        assert_eq!(m.take_requests(), vec![super::super::settings::Request::QuickOpen], "untouched: the popup");
        assert!(!m.beam_view().unwrap().typing);
        m.insert("Trip plans

Gate B, row 14");
        assert!(m.beam_view().unwrap().typing, "fainter while typing");
        press(&mut m, Hit::Fab);
        assert!(m.take_requests().is_empty());
        let title = m.editor.title();
        assert_eq!(m.beam_view().unwrap().menu, Some(title.as_str()), "the menu names the note");
        // A press elsewhere closes it, and does nothing else.
        m.pointer_down(800.0, 500.0, 1, false);
        assert_eq!(m.beam_view().unwrap().menu, None);
        press(&mut m, Hit::Fab);
        press(&mut m, Hit::BeamNote);
        assert_eq!(m.take_requests(), vec![super::super::settings::Request::BeamNote]);
        assert_eq!(m.beam_view().unwrap().menu, None);
        assert!(m.quick_beam_note().is_empty(), "no popup; the keys stay in the note");
        assert_eq!(m.quick.cards[0].text, "Trip plans

Gate B, row 14");
        let _ = title;
        press(&mut m, Hit::Fab);
        press(&mut m, Hit::NewBeam);
        assert_eq!(m.take_requests(), vec![super::super::settings::Request::QuickOpen]);
    }

    #[test]
    fn a_clip_copies_saves_as_a_note_and_a_password_leaves_the_clipboard() {
        let dir = temp("clips");
        let (mut m, mut b) = paired(&dir, true);
        let net = m.quick.net.as_ref().unwrap();
        b.send(Outgoing::Text("Gate B, row 14\nBoarding at 19:05".into()), T);
        net.with(|e, _| assert!(deliver(&mut b, e, 1)));
        b.send(Outgoing::Secret("hunter2!Xq9".into()), T);
        net.with(|e, _| assert!(deliver(&mut b, e, 2)));
        m.quick_open(false);
        assert_eq!(m.quick.cards.len(), 2);
        let secret = m.quick.cards.iter().position(|c| c.kind == Kind::Secret).unwrap();
        let text = 1 - secret;
        // A password copies once, concealed, and leaves the clipboard a minute on.
        assert_eq!(press(&mut m, Hit::Copy(secret)), vec![QuickDo::Copy { text: "hunter2!Xq9".into(), secret: true }]);
        m.quick.on_clipboard.as_mut().unwrap().1 -= CLEAR;
        assert!(m.quick_tick().contains(&QuickDo::Clear("hunter2!Xq9".into())));
        // Text copies, then saves as a note, which takes it out of Quick Send.
        assert_eq!(press(&mut m, Hit::Copy(text)), vec![QuickDo::Copy { text: "Gate B, row 14\nBoarding at 19:05".into(), secret: false }]);
        press(&mut m, Hit::SaveNote(text));
        let (name, path, _) = m.quick.saved.clone().unwrap();
        assert_eq!(name, "Gate B, row 14");
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim_end(), "Gate B, row 14\nBoarding at 19:05");
        assert!(path.starts_with(dir.join("vault")));
        assert!(m.quick.cards.iter().all(|c| c.kind == Kind::Secret), "the saved clip left");
        // Open: the note, in the main window.
        let dos = press(&mut m, Hit::OpenNote);
        assert_eq!(dos, vec![QuickDo::Hide, QuickDo::BringUp]);
        assert_eq!(m.path.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn a_link_opens_in_the_browser() {
        let dir = temp("link");
        let (mut m, mut b) = paired(&dir, true);
        let net = m.quick.net.as_ref().unwrap();
        b.send(Outgoing::Text("https://kolada.app/docs".into()), T);
        net.with(|e, _| assert!(deliver(&mut b, e, 1)));
        m.quick_open(false);
        assert_eq!(press(&mut m, Hit::Open(0)), vec![QuickDo::Hide, QuickDo::Url("https://kolada.app/docs".into())]);
    }

    /// The send area is a text field: type, Enter beams it, Shift+Enter
    /// makes a new line; what is typed stays until it is beamed.
    #[test]
    fn the_send_area_takes_typing_and_enter_beams_it() {
        let dir = temp("type");
        let (mut m, _) = paired(&dir, true);
        m.quick_open(false);
        for c in "Gate B".chars() {
            assert!(m.quick_char(c));
        }
        m.quick_key(QuickKey::NewLine, &none);
        for c in "row 14x".chars() {
            m.quick_char(c);
        }
        m.quick_key(QuickKey::Backspace, &none);
        assert_eq!(m.quick.draft, "Gate B\nrow 14");
        assert!(!m.quick_char('\u{7}'), "no control characters");
        m.quick_frame(q::W as u32, q::H as u32, 1.0);
        assert!(m.quick_place_of(Hit::Beam).is_some(), "Beam shows once something is typed");
        // Put away, it is still there next time.
        m.quick_key(QuickKey::Escape, &none);
        m.quick_open(false);
        assert_eq!(m.quick.draft, "Gate B\nrow 14");
        assert!(m.quick_key(QuickKey::Enter, &none).is_empty());
        assert!(m.quick.draft.is_empty(), "beamed, the field is empty again");
        assert_eq!(m.quick.cards[0].text, "Gate B\nrow 14");
        // Enter on an empty field does nothing.
        m.quick_open(false);
        assert!(m.quick_key(QuickKey::Enter, &none).is_empty());
    }

    /// Paste: text into the field; a password ready to beam, masked, never
    /// in the field; a picture ready to beam with its name and size.
    #[test]
    fn paste_puts_text_in_the_field_and_a_password_or_picture_ready() {
        let dir = temp("paste");
        let (mut m, _) = paired(&dir, true);
        m.quick_open(false);
        m.quick_key(QuickKey::Paste, &|| clip("From the clipboard"));
        assert_eq!(m.quick.draft, "From the clipboard");
        assert!(m.quick.ready.is_none());
        m.quick.draft.clear();
        m.quick_key(QuickKey::Paste, &|| Some(Clip { text: "hunter2!Xq9#".into(), concealed: true, picture: None }));
        assert!(m.quick.draft.is_empty(), "a password never shows in the field");
        assert_eq!(m.quick.ready.as_ref().map(|r| r.kind), Some(Kind::Secret));
        m.quick.ready = None;
        let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
        m.quick_key(QuickKey::Paste, &|| Some(Clip { text: String::new(), concealed: false, picture: Some(("image/png".into(), png.clone())) }));
        let r = m.quick.ready.as_ref().expect("a picture ready");
        assert_eq!((r.kind, r.text.as_str()), (Kind::Picture, "Picture.png"));
        assert_eq!(r.picture.as_ref().map(|p| p.size.as_str()), Some("7 bytes"));
        // Beamed: it goes as a picture, sealed, and its card shows here.
        m.quick_key(QuickKey::Enter, &none);
        assert_eq!(m.quick.cards[0].kind, Kind::Picture);
        assert_eq!(m.quick.net.as_ref().unwrap().read(|e| e.picture(e.cards()[0].id)), Some(png), "kept here, for Copy and Save");
    }

    /// Attach: the file picker; a picture picked (or dropped) is ready to
    /// beam, anything else is refused for now.
    #[test]
    fn attach_opens_the_picker_and_takes_a_picture_only() {
        let dir = temp("attach");
        let (mut m, _) = paired(&dir, true);
        m.quick_open(false);
        assert_eq!(press(&mut m, Hit::Attach), vec![QuickDo::PickFile]);
        let pdf = dir.join("Report.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        m.quick_attach(&[pdf]);
        assert!(m.quick.ready.is_none());
        assert_eq!(m.quick.said.as_ref().map(|(s, _)| s.as_str()), Some("Only pictures can be beamed for now."));
        let shot = dir.join("Screenshot 2026-10-27.JPG");
        std::fs::write(&shot, vec![0xFF, 0xD8, 0xFF, 7]).unwrap();
        m.quick_attach(&[shot]);
        let r = m.quick.ready.as_ref().expect("ready to beam");
        assert_eq!((r.kind, r.text.as_str()), (Kind::Picture, "Screenshot 2026-10-27.jpg"));
        assert_eq!(r.picture.as_ref().map(|p| p.mime.as_str()), Some("image/jpeg"));
    }

    /// A picture card: Copy puts the picture on the clipboard, Save… saves
    /// its file, Save as note puts it in the vault with a note showing it.
    #[test]
    fn a_picture_card_copies_saves_and_becomes_a_note() {
        let dir = temp("pictures");
        let (mut m, mut b) = paired(&dir, true);
        let png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 9, 9, 9];
        let id = b.send(Outgoing::Picture { bytes: png.clone(), mime: "image/png".into(), name: "Whiteboard.png".into(), width: 40, height: 30, thumb: vec![] }, T).unwrap();
        // The sender's upload, as the server and the shell would do it.
        let t = b.outgoing().into_iter().filter_map(|(_, f)| kolada_sync::frame::Frame::parse(&f)).find(|f| f.t() == "ticket").unwrap();
        b.frame(kolada_sync::Channel::Group, &kolada_sync::frame::Frame::new("ticket").with("id", t.get("id").unwrap()).with("op", "put").with("exp", T + 1).with("sig", "s").encode(), T);
        let (blob, sealed) = match b.transfers().pop() {
            Some(kolada_sync::Transfer::Put { blob, body, .. }) => (blob, body),
            other => panic!("{other:?}"),
        };
        b.put_done(blob, 200, T);
        let net = m.quick.net.as_ref().unwrap();
        net.with(|e, _| {
            assert!(deliver(&mut b, e, 1));
            e.got(blob, Some(sealed));
        });
        m.quick_open(false);
        assert_eq!(m.quick.cards[0].kind, Kind::Picture);
        assert_eq!(m.quick.cards[0].picture.as_ref().map(|p| p.3), Some(true), "its bytes are here");
        assert_eq!(press(&mut m, Hit::Copy(0)), vec![QuickDo::CopyPicture(png.clone())]);
        // A tap on it: full size, in the system's viewer.
        assert_eq!(press(&mut m, Hit::Card(0)), vec![QuickDo::OpenFile { name: "Whiteboard.png".into(), bytes: png.clone() }]);
        assert_eq!(press(&mut m, Hit::SaveFile(0)), vec![QuickDo::SaveFile { name: "Whiteboard.png".into(), bytes: png.clone() }]);
        press(&mut m, Hit::SaveNote(0));
        assert_eq!(std::fs::read(dir.join("vault").join("Whiteboard.png")).unwrap(), png);
        assert_eq!(std::fs::read_to_string(dir.join("vault").join("Whiteboard.md")).unwrap().trim_end(), "![[Whiteboard.png]]");
        assert!(m.quick.cards.iter().all(|c| c.id != id), "a saved picture leaves Beam");
    }

    /// Several pictures picked at once are all ready to beam, each with its
    /// ×; one more beside them adds to them; past what goes at once it says
    /// so. Beamed, they go together, and come back as one card holding the
    /// rest, each picture opening on its own.
    #[test]
    fn several_pictures_go_as_one_card() {
        let dir = temp("several");
        let (mut m, _) = paired(&dir, true);
        m.quick_open(false);
        let files: Vec<PathBuf> = (1..=4).map(|k| {
            let f = dir.join(format!("Photo {k}.jpg"));
            std::fs::write(&f, vec![0xFF, 0xD8, 0xFF, k as u8]).unwrap();
            f
        }).collect();
        m.quick_attach(&files[..3]);
        let r = m.quick.ready.as_ref().expect("ready");
        assert_eq!((r.text.as_str(), r.rest.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()), ("Photo 1.jpg", vec!["Photo 2.jpg", "Photo 3.jpg"]));
        m.quick_attach(&files[3..]);
        assert_eq!(m.quick.ready.as_ref().unwrap().rest.len(), 3, "one more beside them");
        press(&mut m, Hit::UnstageAt(0));
        let r = m.quick.ready.as_ref().unwrap();
        assert_eq!((r.text.as_str(), r.rest.len()), ("Photo 2.jpg", 2), "the first's ×: the next leads");
        press(&mut m, Hit::UnstageAt(2));
        assert_eq!(m.quick.ready.as_ref().unwrap().rest.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["Photo 3.jpg"]);
        // Past what goes at once: it says so, and keeps what fits.
        let many: Vec<PathBuf> = (0..12).map(|k| {
            let f = dir.join(format!("Many {k}.png"));
            std::fs::write(&f, vec![0x89, b'P', b'N', b'G', k as u8]).unwrap();
            f
        }).collect();
        m.quick_attach(&many);
        assert_eq!(1 + m.quick.ready.as_ref().unwrap().rest.len(), kolada_sync::PICTURES_AT_ONCE);
        assert_eq!(m.quick.said.as_ref().map(|(s, _)| s.as_str()), Some("Up to 10 pictures go at once."));
        // Beamed: one card holding the rest, here and on the other device.
        m.quick.ready.as_mut().unwrap().rest.truncate(1);
        press(&mut m, Hit::Beam);
        assert!(m.quick.ready.is_none());
        assert_eq!(m.quick.cards.len(), 1, "one card");
        assert_eq!(m.quick.cards[0].rest.len(), 1, "holding the rest");
        // Its second picture opens on its own (the sender keeps a copy).
        let dos = press(&mut m, Hit::Tile(0, 1));
        assert!(matches!(dos.as_slice(), [QuickDo::OpenFile { name, .. }] if name == "Photo 3.jpg"), "{dos:?}");
        // Save all: every picture, in order, for the shell's folder; Copy:
        // every picture as files.
        let names = |files: &[(String, Vec<u8>)]| files.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        match press(&mut m, Hit::SaveFile(0)).as_slice() {
            [QuickDo::SaveAll(files)] => assert_eq!(names(files), vec!["Photo 2.jpg", "Photo 3.jpg"]),
            other => panic!("{other:?}"),
        }
        match press(&mut m, Hit::Copy(0)).as_slice() {
            [QuickDo::CopyFiles(files)] => assert_eq!((names(files), files[0].1.clone()), (vec!["Photo 2.jpg".to_string(), "Photo 3.jpg".to_string()], vec![0xFF, 0xD8, 0xFF, 2])),
            other => panic!("{other:?}"),
        }
    }
}
