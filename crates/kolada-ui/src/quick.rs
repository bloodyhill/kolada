//! Quick Send's popup (docs/specs/2026-10-26-sync-quick-send.md; the
//! redesign the user chose on 2026-10-27): a small window of its own,
//! never part of the side menu. It opens on the keys (Win+Shift+Z), from the
//! tray and from a notification; it shows what was copied, ready to beam
//! with Enter, and the clips beamed and received, kept until removed (the
//! newest 200; a password for a day).
//!
//! - The action is Beam: never the paper plane or the word Send, which are
//!   the note's (its share link and exports).
//! - A clip is not a note: Save as note turns it into one and it leaves
//!   here; its × removes it.
//! - The first time it opens, three steps say what it is for.
//!
//! Plain data in (`PopupView`), a scene out, for every shell. Fills only;
//! the one colour is the accent.

use kolada_text::{Align, Format, Layout};
use windows_core::Result;

use crate::chrome::Painter;
use crate::geom::{Fill, Ink, Rect};
use crate::scene::{self, Eng, Fonts, Scene, Target, Text};

/// The action's word, in one place: "Beam" (the user may choose another).
/// Never "Send", which is the note's.
pub const VERB: &str = "Beam";
pub const VERB_DONE: &str = "Beamed";
pub const VERB_LOWER: &str = "beam";

/// The popup's size, DIPs; and the size it shrinks to once something is beamed.
pub const W: f32 = 376.0;
pub const H: f32 = 560.0;
pub const BEAMED_H: f32 = 72.0;

/// What a click in the popup is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    /// Anywhere in it with nothing to do.
    Popup,
    /// The side menu's Beam row: opens the popup on its clips.
    Show,
    /// The window's Beam button, bottom right; its menu's two choices.
    Fab,
    BeamNote,
    NewBeam,
    /// The text field to type in (the send area), and its attach button.
    Field,
    Attach,
    Close,
    /// Opens the devices (also Settings' "Devices…" button), and back.
    Devices,
    Back,
    Beam,
    /// Puts what is ready to beam away.
    Unstage,
    /// One of the pictures ready to beam, its ×: it goes, the rest stay
    /// (0 is the first).
    UnstageAt(usize),
    /// A picture of a card holding several: card, picture (0 is the first).
    Tile(usize, usize),
    /// Who to beam to: everyone (0) or one device (its place + 1).
    To(usize),
    Copy(usize),
    Open(usize),
    SaveNote(usize),
    /// A picture saved as a file, where the user picks.
    SaveFile(usize),
    /// A clip's ×: it leaves this device's history.
    Forget(usize),
    /// A card itself: a tap opens a cut one (or a picture, or a link).
    Card(usize),
    /// A card's "…": its menu (Open, Save, Save as note, Remove).
    More(usize),
    /// "Read more" under an opened card cut past its limit: the beam as a
    /// note, opened and not written.
    ReadMore(usize),
    /// A failed beam's red mark: send it again.
    Retry(usize),
    /// The clips' list (scrolls).
    List,
    /// "Saved as a note": opens it.
    OpenNote,
    /// The first use's buttons.
    PairNow,
    NotNow,
    /// Pairing.
    ShowCode,
    EnterCode,
    CodeField,
    Pair,
    Cancel,
    Allow,
    Deny,
    Remove(usize),
    /// This device's Rename, its name being typed, Save and Cancel.
    Rename,
    NameField,
    NameSave,
    NameCancel,
    /// "Not a password", under something ready that may be one: it beams
    /// as text.
    NotSecret,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Link,
    Secret,
    Picture,
}

/// A picture, as a clip or what was copied shows it: its thumbnail once
/// decoded, its size in pixels, how big its file is ("1.2 MB"), and whether
/// its bytes are here yet (a received one comes down just after its item).
#[derive(Clone)]
pub struct PicView<'a> {
    pub pixels: Option<std::sync::Arc<crate::pixels::Pixels>>,
    pub width: u32,
    pub height: u32,
    pub size: &'a str,
    pub ready: bool,
}

impl std::fmt::Debug for PicView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PicView({}x{}, {}, ready {})", self.width, self.height, self.size, self.ready)
    }
}

impl PartialEq for PicView<'_> {
    fn eq(&self, o: &Self) -> bool {
        (self.width, self.height, self.size, self.ready) == (o.width, o.height, o.size, o.ready) && self.pixels.as_ref().map(std::sync::Arc::as_ptr) == o.pixels.as_ref().map(std::sync::Arc::as_ptr)
    }
}

/// A clip as the list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipView<'a> {
    /// Its faint line under the text, on the first card of a group (a new
    /// sender, or 5 minutes or more apart): "iPhone · 2 min ago", or the
    /// time alone for this device's own; empty on the rest.
    pub from: &'a str,
    /// "Goes once copied" for a password; empty for the rest, which stay
    /// until removed.
    pub goes: &'a str,
    pub kind: Kind,
    pub text: &'a str,
    pub title: &'a str,
    /// Copied just now.
    pub copied: bool,
    /// A password already copied: shown as gone.
    pub spent: bool,
    pub mine: bool,
    pub picture: Option<PicView<'a>>,
    /// The rest of the pictures beamed with `picture`, in order: the card
    /// shows them all, as a grid.
    pub more: &'a [PicView<'a>],
    /// How open it is, 0 (two lines) to 1 (as many as `OPEN_LINES`), as
    /// its spring has it (`spring`).
    pub open: f32,
    /// Sent from here: pending, its check, or failed (`Marks::card`).
    pub sent: Sent,
}

/// A beam sent from here, as its card shows it: waiting for the server,
/// its check (popping in and fading, as `check_shape` gives it), or failed
/// (a click retries).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Sent {
    #[default]
    None,
    Pending,
    Check { scale: f32, alpha: f32 },
    Failed,
}

/// The check's life once the server has the beam: in on a spring, held,
/// then faded (seconds).
pub const CHECK_IN: f32 = 0.3;
pub const CHECK_HOLD: f32 = 0.8;
pub const CHECK_OUT: f32 = 0.25;

/// The check `t` seconds after the server took the beam: its scale and its
/// alpha, or None once it is gone. With `animate` off (the OS asks for less
/// motion) it fades in and out at its size, without the spring.
pub fn check_shape(t: f32, animate: bool) -> Option<(f32, f32)> {
    if !(0.0..CHECK_IN + CHECK_HOLD + CHECK_OUT).contains(&t) {
        return None;
    }
    let out = if t > CHECK_IN + CHECK_HOLD { 1.0 - (t - CHECK_IN - CHECK_HOLD) / CHECK_OUT } else { 1.0 };
    if !animate {
        return Some((1.0, (t / 0.15).min(1.0).min(out)));
    }
    // A spring from nothing to whole that overshoots a little and settles.
    let (w, z) = (22.0f32, 0.5f32);
    let wd = w * (1.0 - z * z).sqrt();
    let s = 1.0 - (-z * w * t).exp() * ((wd * t).cos() + z * w / wd * (wd * t).sin());
    Some((s.max(0.0), (t / 0.08).min(1.0).min(out)))
}

/// Where a beam's check shows: beside its card, sent from the popup; or by
/// the window's Beam button, sent where no popup showed (Beam this note,
/// the keys beaming at once).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Card,
    Button,
}

/// The beams sent from here and how each stands, from sent until its check
/// is gone (a failed one until retried or removed). The shared model and
/// each shell keep one and draw from it.
#[derive(Clone, Debug, Default)]
pub struct Marks {
    list: Vec<Mark>,
}

#[derive(Clone, Debug)]
struct Mark {
    id: [u8; 16],
    place: Place,
    acked: Option<std::time::Instant>,
    failed: bool,
}

impl Marks {
    /// `id` went to the server: pending until it says it has it.
    pub fn sent(&mut self, id: [u8; 16], place: Place) {
        self.list.retain(|m| m.id != id);
        self.list.push(Mark { id, place, acked: None, failed: false });
    }

    /// The server has `id`: its check starts now. Whether it was ours.
    pub fn acked(&mut self, id: [u8; 16], now: std::time::Instant) -> bool {
        self.list.iter_mut().find(|m| m.id == id).map(|m| m.acked = Some(now)).is_some()
    }

    /// `id` didn't go: its card shows the failure. Whether it was ours.
    pub fn failed(&mut self, id: [u8; 16]) -> bool {
        self.list.iter_mut().find(|m| m.id == id).map(|m| m.failed = true).is_some()
    }

    pub fn forget(&mut self, id: [u8; 16]) {
        self.list.retain(|m| m.id != id);
    }

    /// `id` never arrived (it expired on the server): its card shows the
    /// failure, as after a restart, whether or not it was sent this run.
    pub fn lost(&mut self, id: [u8; 16]) {
        match self.list.iter_mut().find(|m| m.id == id) {
            Some(m) => m.failed = true,
            None => self.list.push(Mark { id, place: Place::Card, acked: None, failed: true }),
        }
    }

    /// How `id`'s card shows it at `now`.
    pub fn card(&self, id: [u8; 16], now: std::time::Instant, animate: bool) -> Sent {
        match self.list.iter().find(|m| m.id == id) {
            None => Sent::None,
            Some(m) if m.failed => Sent::Failed,
            Some(Mark { acked: None, .. }) => Sent::Pending,
            Some(Mark { acked: Some(t), place: Place::Card, .. }) => check_shape(now.duration_since(*t).as_secs_f32(), animate).map_or(Sent::None, |(scale, alpha)| Sent::Check { scale, alpha }),
            Some(_) => Sent::None,
        }
    }

    /// The check by the Beam button at `now`, if one shows.
    pub fn button(&self, now: std::time::Instant, animate: bool) -> Option<(f32, f32)> {
        self.list.iter().rev().filter(|m| m.place == Place::Button && !m.failed).find_map(|m| check_shape(now.duration_since(m.acked?).as_secs_f32(), animate))
    }

    /// A check is showing: frames are due until it goes.
    pub fn moving(&self, now: std::time::Instant) -> bool {
        self.list.iter().any(|m| !m.failed && m.acked.is_some_and(|t| now.duration_since(t).as_secs_f32() < CHECK_IN + CHECK_HOLD + CHECK_OUT))
    }

    /// Lets go of the checks that are gone.
    pub fn prune(&mut self, now: std::time::Instant) {
        self.list.retain(|m| m.failed || m.acked.is_none_or(|t| now.duration_since(t).as_secs_f32() < CHECK_IN + CHECK_HOLD + CHECK_OUT));
    }
}

/// What a beam's failure says, by the server's reason (`Event::SendFailed`).
pub fn failure(why: &str) -> &'static str {
    match why {
        "slow" => "Too many beams at once. Try again in a minute.",
        "limit" => "That's the most beaming for today. Try again tomorrow.",
        "full" => "Your other devices have too much waiting. Try again once they catch up.",
        "size" => "Too big to beam.",
        "pictures" => "Pictures are resting until tomorrow.",
        _ => "That didn't beam. Try again.",
    }
}

/// The faint line under a beam of this device's that expired before any
/// device had it.
pub const LOST: &str = "Didn't arrive";

/// The faint line under a note beamed with its pictures: how many came.
pub fn with_pictures(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => "With 1 picture".into(),
        n => format!("With {n} pictures"),
    }
}

/// A sent beam's mark centred at (`cx`, `cy`): faint dots while pending;
/// the check, a filled teal disc as big as the field's buttons with a white
/// check; a failure, red (an error), whose click retries.
pub(crate) fn sent_mark(p: &mut Painter, cx: f32, cy: f32, sent: Sent, retry: Option<Hit>) {
    let half = FIELD_BUTTON / 2.0;
    let around = Rect::new(cx - half, cy - half, cx + half, cy + half);
    match sent {
        Sent::None => {}
        Sent::Pending => dots(p, around, Ink::Faint),
        Sent::Check { scale, alpha } => {
            let r = half * scale;
            p.scene.fills.push(Fill { alpha, ..Fill::solid(Rect::new(cx - r, cy - r, cx + r, cy + r), r, Ink::Accent) });
            let g = 20.0 * scale;
            let layer = p.scene.layer_now();
            p.scene.vectors.push(crate::scene::VectorMark { layer, rect: Rect::new(cx - g / 2.0, cy - g / 2.0, cx + g / 2.0, cy + g / 2.0), logo: crate::vector::Logo::Check, ink: Ink::OnAccent, alpha });
        }
        Sent::Failed => {
            p.scene.fills.push(Fill::solid(around, half, Ink::Danger));
            p.scene.fills.push(Fill::solid(Rect::new(cx - 1.5, cy - 8.0, cx + 1.5, cy + 2.5), 1.5, Ink::OnDanger));
            p.scene.fills.push(Fill::solid(Rect::new(cx - 1.75, cy + 4.5, cx + 1.75, cy + 8.0), 1.75, Ink::OnDanger));
            if let Some(hit) = retry {
                p.scene.hits.push((around, Target::Quick(hit)));
            }
        }
    }
}

/// What was copied, ready to beam.
#[derive(Clone, Debug, PartialEq)]
pub struct Staged<'a> {
    /// "a link", "text", "a password", "a picture".
    pub what: &'a str,
    pub kind: Kind,
    /// The text; a picture's file name.
    pub text: &'a str,
    pub picture: Option<PicView<'a>>,
    /// The rest of the pictures picked with `picture`, each removable.
    pub more: &'a [PicView<'a>],
    /// Maybe a password: masked, with "Not a password" to click.
    pub unsure: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceRow<'a> {
    pub name: &'a str,
    pub look: Look,
    pub this: bool,
}

/// How a device is drawn: its mark in Settings > Devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Laptop,
    Desktop,
    Phone,
    Tablet,
}

impl Look {
    /// From `kolada_sync::names::look`'s word.
    pub fn of(word: &str) -> Look {
        match word {
            "desktop" => Look::Desktop,
            "phone" => Look::Phone,
            "tablet" => Look::Tablet,
            _ => Look::Laptop,
        }
    }

    pub fn logo(self) -> crate::vector::Logo {
        match self {
            Look::Laptop => crate::vector::Logo::Laptop,
            Look::Desktop => crate::vector::Logo::Desktop,
            Look::Phone => crate::vector::Logo::Phone,
            Look::Tablet => crate::vector::Logo::Tablet,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pairing<'a> {
    Idle,
    Opening,
    Showing { code: &'a str, left: &'a str },
    Asks { name: &'a str },
    Entering { typed: &'a str },
    Waiting,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Screen<'a> {
    /// The first time it opens: what it is for, in three steps.
    FirstUse,
    Main {
        staged: Option<Staged<'a>>,
        /// "All devices" first, then each other device; the one chosen.
        to: &'a [&'a str],
        chosen: usize,
        clips: &'a [ClipView<'a>],
        scroll: f32,
        /// "Saved as a note, “…”".
        saved: Option<&'a str>,
        /// What is typed in the send area so far.
        draft: &'a str,
        /// The card whose "…" menu is open.
        more: Option<usize>,
    },
    /// This device's name while it is being typed (Rename).
    Devices { devices: &'a [DeviceRow<'a>], pairing: Pairing<'a>, facts: &'a [String], naming: Option<&'a str> },
    /// Beamed: to whom, and what.
    Beamed { to: &'a str, what: &'a str },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PopupView<'a> {
    pub screen: Screen<'a>,
    pub devices: usize,
    /// A line to say (a refusal, keys already in use).
    pub said: Option<&'a str>,
    pub host: Host<'a>,
}

/// Where the popup runs: what it calls this device, its keys, and paste.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Host<'a> {
    pub this: &'a str,
    pub keys: &'a str,
    pub paste: &'a str,
}

impl Host<'static> {
    pub const WINDOWS: Host<'static> = Host { this: "PC", keys: "Alt+Z", paste: "Ctrl+V" };
    pub const MAC: Host<'static> = Host { this: "Mac", keys: "\u{2325}Z", paste: "\u{2318}V" };
    /// The Mac's keys where another app holds Option+Z.
    pub const MAC_OLD_KEYS: &'static str = "\u{2303}\u{2325}\u{21E7}Z";
    pub const LINUX: Host<'static> = Host { this: "computer", keys: "Alt+Z", paste: "Ctrl+V" };
}

// One spacing scale (4 and 8 points) and one radius family: cards 12,
// controls 8, chips round.
const PAD: f32 = 16.0;
const GAP: f32 = 8.0;
const BAR: f32 = 32.0;
const CARD_RADIUS: f32 = 12.0;
const CONTROL_RADIUS: f32 = 8.0;
/// A card's inner padding, and its who-and-when line.
const CARD_PAD: f32 = 12.0;
const META: f32 = 16.0;
/// The composer's field: a capsule at least a line and 12 above and below
/// tall, its ends round (radius `FIELD_MIN / 2`). What sits in it (the
/// beam, attach) is a circle inset `FIELD_INSET` from its top, bottom and
/// end, its radius the end's less the inset: concentric with it.
const FIELD_MIN: f32 = 44.0;
const FIELD_INSET: f32 = 6.0;
const FIELD_BUTTON: f32 = FIELD_MIN - 2.0 * FIELD_INSET;
const CHIP: f32 = 28.0;
/// A glyph button (Copy, "…", attach, beam), and the gap at a card's side.
const GLYPH: f32 = 24.0;
const GUTTER: f32 = 48.0;
/// A text card shows this many lines at rest; opened, this many at most
/// (fewer in a short popup), past which "Read more" opens it as a note.
pub const REST_LINES: usize = 2;
pub const OPEN_LINES: usize = 12;
/// A picture card's width.
const PICTURE_W: f32 = 240.0;

/// One step of a card's spring toward `to` (0 or 1), `dt` seconds on:
/// where it is and how fast. Natural, a touch of give, settled in about
/// a quarter second.
pub fn spring(x: f32, v: f32, to: f32, dt: f32) -> (f32, f32) {
    const K: f32 = 260.0;
    const C: f32 = 29.0;
    let mut x = x;
    let mut v = v;
    // Small steps: steady whatever the frame rate.
    let n = (dt / 0.004).ceil().max(1.0) as usize;
    let h = dt / n as f32;
    for _ in 0..n {
        v += (K * (to - x) - C * v) * h;
        x += v * h;
    }
    if (x - to).abs() < 0.002 && v.abs() < 0.05 { (to, 0.0) } else { (x, v) }
}

fn label(p: &Painter) -> Format {
    p.fonts.label.clone()
}

fn small(p: &Painter) -> Format {
    p.fonts.small.clone()
}

fn centred(f: &Format) -> Format {
    Format { align: Align::Center, ..f.clone() }
}

fn bold(f: &Format) -> Format {
    Format { weight: 600, ..f.clone() }
}

fn wrapped(p: &Painter) -> Format {
    Format { wrap: true, center_y: false, ..p.fonts.peek.clone() }
}

fn put(p: &mut Painter, s: &str, f: &Format, r: Rect, ink: Ink, alpha: f32) {
    let l = scene::single_line(p.eng, s, f, r.width().max(1.0), r.height());
    p.scene.texts.push(Text { x: r.left, y: r.top, layout: l, ink, alpha });
}

/// The beam mark in `r` (a square), from its outline: a dot and two
/// quarter rings, filled, crisp at every size.
pub(crate) fn beam_glyph(p: &mut Painter, r: Rect, ink: Ink) {
    beam_mark(p, r, ink, 1.0);
}

pub(crate) fn beam_mark(p: &mut Painter, r: Rect, ink: Ink, alpha: f32) {
    let layer = p.scene.layer_now();
    p.scene.vectors.push(crate::scene::VectorMark { layer, rect: r, logo: crate::vector::Logo::Beam, ink, alpha });
}

/// A button: the one asked for raised (as a chosen option is), the other
/// quiet; neither teal.
fn button(p: &mut Painter, r: Rect, word: &str, hit: Hit, strong: bool) {
    let hot = p.hover == Some(Target::Quick(hit));
    if strong {
        p.scene.fills.push(Fill::shadow(r.offset(0.0, 1.0), CONTROL_RADIUS, 4.0, 0.8));
        p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS, if hot { Ink::Hover } else { Ink::Raised }));
    } else if hot {
        p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS, Ink::Hover));
    }
    p.scene.hits.push((r, Target::Quick(hit)));
    let f = if strong { bold(&p.fonts.choice) } else { p.fonts.choice.clone() };
    put(p, word, &centred(&f), r, if strong { Ink::Text } else { Ink::Muted }, 1.0);
}

fn icon_button(p: &mut Painter, r: Rect, glyph: &str, hit: Hit) {
    let hot = p.hover == Some(Target::Quick(hit));
    if hot {
        p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS, Ink::Hover));
    }
    p.scene.hits.push((r, Target::Quick(hit)));
    let f = centred(&p.fonts.icon_small);
    put(p, glyph, &f, r, if hot { Ink::Text } else { Ink::Muted }, 1.0);
}

/// Three dots, filled: a card's "…".
fn dots(p: &mut Painter, r: Rect, ink: Ink) {
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    for dx in [-5.0, 0.0, 5.0] {
        p.scene.fills.push(Fill::solid(Rect::new(cx + dx - 1.5, cy - 1.5, cx + dx + 1.5, cy + 1.5), 1.5, ink));
    }
}

/// A glyph button in `r`: its highlight under the pointer (radius
/// `CONTROL_RADIUS`, or round in the field), its glyph in muted ink (the
/// text's under the pointer).
fn glyph(p: &mut Painter, r: Rect, g: &str, hit: Hit) {
    glyph_in(p, r, g, hit, CONTROL_RADIUS);
}

fn glyph_in(p: &mut Painter, r: Rect, g: &str, hit: Hit, radius: f32) {
    let hot = p.hover == Some(Target::Quick(hit));
    if hot {
        p.scene.fills.push(Fill::solid(r, radius, Ink::Hover));
    }
    p.scene.hits.push((r, Target::Quick(hit)));
    // Its ink in the middle, not its box: an icon's ink sits off its em.
    let f = centred(&p.fonts.icon_small.clone());
    let _ = p.centred(g, &f, r, if hot { Ink::Text } else { Ink::Muted });
}

/// The most lines the send area grows to; past them it shows the last ones.
const FIELD_LINES: usize = 4;
/// The placeholder's words, kept short.
pub const PLACEHOLDER: &str = "Type, paste or drop to beam";
/// Under something ready that may be a password.
pub const NOT_SECRET: &str = "Not a password";
/// The field's caret: as wide as the note's, and as tall for its text size
/// (1.5 times it), centred on the capitals (0.35 of the size above the
/// baseline), as the note's is.
pub const CARET_WIDTH: f32 = 2.0;
const CARET_TALL: f32 = 1.5;
const CARET_MID: f32 = 0.35;
/// The placeholder starts this far after the caret, which sits before it.
const CARET_GAP: f32 = 5.0;

/// Whether `hit` takes typing: the pointer is the text I-beam over it.
pub fn takes_typing(hit: Hit) -> bool {
    matches!(hit, Hit::Field | Hit::CodeField | Hit::NameField)
}

/// The composer, pinned at the popup's foot: one raised field. In it what
/// is typed (or the placeholder), and at its end, on its last line: who to
/// (a chip, only with two devices or more to choose from: a tap moves to
/// the next), attach, and the beam, teal once there is something to beam.
/// Something copied and ready sits in the field's top, with its x. Where
/// it starts.
#[allow(clippy::too_many_arguments)]
fn composer(p: &mut Painter, x0: f32, x1: f32, bottom: f32, ceiling: f32, staged: Option<&Staged>, draft: &str, to: &[&str], chosen: usize) -> Result<f32> {
    let body = wrapped(p);
    let line = body.line.unwrap_or(18.0);
    let typed = !draft.is_empty();
    let beamable = typed || staged.is_some();
    let f = small(p);
    // The end of the last line: the beam, attach, and who to; each a circle
    // inset alike from the field's bottom (and the beam from its end), so
    // they stay on the last line as the field grows.
    let row_top = bottom - FIELD_MIN;
    let send = Rect::new(x1 - FIELD_INSET - FIELD_BUTTON, bottom - FIELD_INSET - FIELD_BUTTON, x1 - FIELD_INSET, bottom - FIELD_INSET);
    let attach = Rect::new(send.left - FIELD_BUTTON, send.top, send.left, send.bottom);
    let mut end = attach.left;
    let chip = (to.len() > 2).then(|| {
        let name = to.get(chosen).copied().unwrap_or(to[0]);
        let w = scene::single_line(p.eng, name, &f, 200.0, 24.0).measure().width + 20.0;
        let r = Rect::new(attach.left - 4.0 - w, row_top + 10.0, attach.left - 4.0, bottom - 10.0);
        end = r.left - 4.0;
        (r, name)
    });
    let inner = end - 8.0 - (x0 + 16.0);
    let shown = if typed { draft } else if staged.is_some() { "Enter to beam it" } else { PLACEHOLDER };
    let l = scene::layout(p.eng, shown, &body, inner, f32::MAX);
    let lines = l.lines().len().max(1);
    let visible = lines.min(FIELD_LINES);
    let text_h = (visible as f32 * line + 24.0).max(FIELD_MIN).ceil();
    // What was copied, in the field's top: 56 tall, smaller where the
    // popup is short.
    let preview_h = if staged.is_some() { (bottom - text_h - ceiling - 120.0).clamp(0.0, 56.0).floor() } else { 0.0 };
    let field = Rect::new(x0, bottom - text_h - preview_h, x1, bottom);
    band(p, Rect::new(x0 - PAD, field.top - PAD * 0.75, x1 + PAD, bottom + PAD));
    p.scene.fills.push(Fill::solid(field, FIELD_MIN / 2.0, Ink::Field));
    p.scene.hits.push((Rect::new(field.left, field.top + preview_h, end, field.bottom), Target::Quick(Hit::Field)));
    if let Some(s) = staged.filter(|_| preview_h >= 40.0) {
        let r = Rect::new(field.left + 8.0, field.top + 8.0, field.right - 8.0, field.top + preview_h);
        let (lead, first, second) = match (&s.picture, s.kind) {
            // Several: a row of them, each with its ×, then how many.
            (Some(pic), _) if !s.more.is_empty() => {
                let all: Vec<&PicView> = std::iter::once(pic).chain(s.more.iter()).collect();
                let fit = (((r.width() - 32.0 - 96.0) / (STAGED_TILE + 6.0)).floor() as usize).max(2);
                let shown = if all.len() > fit { fit - 1 } else { all.len() };
                let mut x = r.left;
                let mut crosses = Vec::new();
                for (k, pic) in all.iter().take(shown).enumerate() {
                    let t = Rect::new(x, r.top, x + STAGED_TILE, r.top + STAGED_TILE);
                    tile(p, pic, t, 8.0);
                    crosses.push((k, Rect::new(t.right - 18.0, t.top - 6.0, t.right + 6.0, t.top + 18.0)));
                    x = t.right + 6.0;
                }
                // Each one's ×: a small dark disc on its corner, in a layer
                // over the pictures.
                let clip = p.scene.overlays.last().and_then(|o| o.2);
                p.scene.overlay(clip);
                for (k, cross) in crosses {
                    let disc = cross.inset(4.0);
                    let hot = p.hover == Some(Target::Quick(Hit::UnstageAt(k)));
                    p.scene.fills.push(Fill { alpha: if hot { 0.85 } else { 0.62 }, ..Fill::solid(disc, 8.0, Ink::Scrim) });
                    p.scene.hits.push((cross, Target::Quick(Hit::UnstageAt(k))));
                    put(p, "\u{E8BB}", &centred(&Format { size: 8.0, ..p.fonts.icon_small.clone() }), disc, Ink::OnAccent, 1.0);
                }
                if all.len() > shown {
                    let t = Rect::new(x, r.top, x + STAGED_TILE, r.top + STAGED_TILE);
                    p.scene.fills.push(Fill::solid(t, 8.0, Ink::Field));
                    put(p, &format!("+{}", all.len() - shown), &centred(&bold(&small(p))), t, Ink::Muted, 1.0);
                    x = t.right + 6.0;
                }
                (x - r.left + 4.0, format!("{} pictures", all.len()), "Ready to beam".to_string())
            }
            (Some(pic), _) => {
                thumb(p, pic, Rect::new(r.left, r.top, r.left + 40.0, r.top + 40.0));
                (48.0, s.text.to_string(), pic.size.to_string())
            }
            (None, Kind::Secret) => {
                put(p, "\u{E72E}", &centred(&p.fonts.icon_small.clone()), Rect::new(r.left, r.top, r.left + 40.0, r.top + 40.0), Ink::Muted, 1.0);
                (48.0, "\u{2022}".repeat(12), "A password, ready".to_string())
            }
            (None, _) => (8.0, s.text.lines().next().unwrap_or("").to_string(), format!("Ready to {VERB_LOWER} \u{00B7} {}", s.what)),
        };
        let tx = r.left + lead;
        put(p, &first, &Format { wrap: false, ellipsis: true, ..label(p) }, Rect::new(tx, r.top, r.right - 32.0, r.top + 20.0), if s.kind == Kind::Link { Ink::Link } else { Ink::Text }, 1.0);
        let under = Rect::new(tx, r.top + 20.0, r.right - 32.0, r.top + 38.0);
        if s.unsure && s.kind == Kind::Secret {
            // Maybe a password: one click and it is text.
            let w = scene::single_line(p.eng, NOT_SECRET, &f, under.width(), 24.0).measure().width.ceil();
            let hit = Rect::new(under.left, under.top, under.left + w, under.bottom);
            let hot = p.hover == Some(Target::Quick(Hit::NotSecret));
            put(p, NOT_SECRET, &f, under, if hot { Ink::Text } else { Ink::Muted }, 1.0);
            p.scene.hits.push((hit, Target::Quick(Hit::NotSecret)));
        } else {
            put(p, &second, &f, under, Ink::Muted, 1.0);
        }
        glyph(p, Rect::new(r.right - GLYPH, r.top + 8.0, r.right, r.top + 8.0 + GLYPH), "\u{E8BB}", Hit::Unstage);
    }
    // The text, its last lines when it is long, cut at the field's edge;
    // one line sits in the middle, more start 12 from the top.
    let text_top = field.top + preview_h + if visible == 1 { (text_h - line) / 2.0 } else { 12.0 };
    let shift = (lines - visible) as f32 * line;
    p.scene.overlay(Some(Rect::new(field.left, field.top + preview_h + 4.0, end, field.bottom - 4.0)));
    let text_x = if typed { x0 + 16.0 } else { x0 + 16.0 + CARET_GAP };
    let baseline = l.glyphs((0.0, 0.0), 1.0).first().map_or(line * 0.8, |g| g.y as f32);
    let (cx, cy) = if typed { let c = l.caret(draft.len()); (c.x, c.y - shift) } else { (0.0, 0.0) };
    p.scene.texts.push(Text { x: text_x, y: text_top - shift, layout: l, ink: if typed { Ink::Text } else { Ink::Faint }, alpha: 1.0 });
    // The caret: at the end of what is typed; before the placeholder,
    // which starts just after it, while nothing is (and nothing is ready).
    if staged.is_none() {
        let mid = text_top + cy + baseline - CARET_MID * body.size;
        let half = CARET_TALL * body.size / 2.0;
        let cx = x0 + 16.0 + cx;
        p.scene.fills.push(Fill::solid(Rect::new(cx - CARET_WIDTH / 2.0, mid - half, cx + CARET_WIDTH / 2.0, mid + half), CARET_WIDTH / 2.0, Ink::Accent));
    }
    p.scene.overlay(None);
    if let Some((r, name)) = chip {
        let hot = p.hover == Some(Target::Quick(Hit::To((chosen + 1) % to.len())));
        p.scene.fills.push(Fill::solid(r, r.height() / 2.0, if hot { Ink::Border } else { Ink::Hover }));
        p.scene.hits.push((r, Target::Quick(Hit::To((chosen + 1) % to.len()))));
        put(p, name, &centred(&f), r, Ink::Muted, 1.0);
    }
    glyph_in(p, attach, "\u{E723}", Hit::Attach, FIELD_BUTTON / 2.0);
    // The beam: teal with something to beam, faint without.
    if beamable {
        let hot = p.hover == Some(Target::Quick(Hit::Beam));
        if hot {
            p.scene.fills.push(Fill::solid(send, FIELD_BUTTON / 2.0, Ink::Hover));
        }
        p.scene.hits.push((send, Target::Quick(Hit::Beam)));
    }
    let g = 20.0;
    // The mark's ink spans 3.5..20.5 of its 24 across and down: its middle
    // is its box's.
    let (sx, sy) = ((send.left + send.right) / 2.0, (send.top + send.bottom) / 2.0);
    beam_glyph(p, Rect::new(sx - g / 2.0, sy - g / 2.0, sx + g / 2.0, sy + g / 2.0), if beamable { Ink::Accent } else { Ink::Faint });
    Ok(field.top)
}

/// A picture filling `r`, its shape kept, centred, on the card's own tone
/// where the shapes differ (never a box of its own), its corners `round`;
/// before its thumbnail is decoded, the picture glyph on an inset.
fn cover(p: &mut Painter, pic: &PicView, r: Rect, round: f32) {
    match &pic.pixels {
        Some(px) => {
            let (pw, ph) = (px.width.max(1) as f32, px.height.max(1) as f32);
            let s = (r.width() / pw).min(r.height() / ph);
            let (dw, dh) = (pw * s, ph * s);
            let at = Rect::new(r.left + (r.width() - dw) / 2.0, r.top + (r.height() - dh) / 2.0, r.left + (r.width() + dw) / 2.0, r.top + (r.height() + dh) / 2.0);
            let layer = p.scene.layer_now();
            p.scene.pictures.push(crate::scene::Picture { layer, rect: at, pixels: px.clone(), alpha: 1.0, round });
        }
        None => {
            p.scene.fills.push(Fill::solid(r, round, Ink::Field));
            put(p, "\u{E91B}", &centred(&p.fonts.icon_small.clone()), r, Ink::Faint, 1.0);
        }
    }
}

/// A picture's thumbnail in `r` (its height at most), as wide as `r` and as
/// tall as the picture's shape allows: its height. Before the thumbnail is
/// decoded, a quiet field with the picture glyph.
fn thumb(p: &mut Painter, pic: &PicView, r: Rect) -> f32 {
    let w = r.width();
    let aspect = if pic.width > 0 && pic.height > 0 { pic.height as f32 / pic.width as f32 } else { 0.6 };
    let h = (w * aspect).clamp(r.height().min(56.0), r.height());
    let box_ = Rect::new(r.left, r.top, r.right, r.top + h);
    match &pic.pixels {
        Some(px) => {
            // The whole picture, centred, its shape kept (letterboxed).
            let (pw, ph) = (px.width.max(1) as f32, px.height.max(1) as f32);
            let s = (w / pw).min(h / ph);
            let (dw, dh) = (pw * s, ph * s);
            let at = Rect::new(box_.left + (w - dw) / 2.0, box_.top + (h - dh) / 2.0, box_.left + (w + dw) / 2.0, box_.top + (h + dh) / 2.0);
            let layer = p.scene.layer_now();
            p.scene.pictures.push(crate::scene::Picture { layer, rect: at, pixels: px.clone(), alpha: 1.0, round: 8.0 });
        }
        None => {
            p.scene.fills.push(Fill::solid(box_, 8.0, Ink::Field));
            put(p, "\u{E91B}", &centred(&p.fonts.icon_small.clone()), box_, Ink::Faint, 1.0);
        }
    }
    h
}

/// The popup, `width` x `height` DIPs (`W` x `H`, or `BEAMED_H` tall).
pub fn popup(eng: &Eng, fonts: &Fonts, view: &PopupView, hover: Option<Target>, width: f32, height: f32) -> Result<Scene> {
    let mut scene = Scene::default();
    // Layers count from its own overlays (the clips' list is one), as the
    // chrome's do: pictures land in the layer they are drawn in.
    scene.in_chrome = true;
    let mut p = Painter { eng, fonts, hover, scene: &mut scene };
    let all = Rect::new(0.0, 0.0, width, height);
    // The well, lowest: the list's cards rise from it, between the top
    // bar's and the composer's bands (`top`, `composer`).
    p.scene.fills.push(Fill::solid(all, 0.0, Ink::Well));
    p.scene.hits.push((all, Target::Quick(Hit::Popup)));
    match &view.screen {
        Screen::Beamed { to, what } => beamed(&mut p, all, to, what),
        Screen::FirstUse => first_use(&mut p, view, all),
        Screen::Main { staged, to, chosen, clips, scroll, saved, draft, more } => main(&mut p, view, all, staged.as_ref(), to, *chosen, clips, *scroll, *saved, draft, *more),
        Screen::Devices { devices, pairing, facts, naming } => devices_screen(&mut p, view, all, devices, pairing, facts, *naming),
    }?;
    Ok(scene)
}

/// The top bar, 32 tall: the mark and name (or Back and a title), the
/// devices chip, Close. Where what is under it starts.
fn top(p: &mut Painter, all: Rect, back: Option<&str>, devices: Option<usize>) -> f32 {
    let y = all.top + PAD;
    let (x0, x1) = (all.left + PAD, all.right - PAD);
    // Its own band, on the panel's tone, lifted off the well by a shadow.
    band(p, Rect::new(all.left, all.top, all.right, y + BAR + PAD * 0.75));
    match back {
        Some(title) => {
            icon_button(p, Rect::new(x0 - 4.0, y, x0 + 28.0, y + BAR), "\u{E72B}", Hit::Back);
            put(p, title, &bold(&label(p)), Rect::new(x0 + 32.0, y, x1 - 40.0, y + BAR), Ink::Text, 1.0);
        }
        None => {
            beam_glyph(p, Rect::new(x0, y + 6.0, x0 + 20.0, y + 26.0), Ink::Text);
            put(p, "Beam", &bold(&label(p)), Rect::new(x0 + 28.0, y, x1 - 140.0, y + BAR), Ink::Text, 1.0);
        }
    }
    let close = Rect::new(x1 - BAR, y, x1, y + BAR);
    icon_button(p, close, "\u{E8BB}", Hit::Close);
    if let Some(n) = devices {
        let word = match n {
            0 => "Pair your devices".to_string(),
            1 => "1 device".to_string(),
            n => format!("{n} devices"),
        };
        let f = small(p);
        let w = scene::single_line(p.eng, &word, &f, 200.0, CHIP).measure().width + 24.0;
        let r = Rect::new(close.left - GAP - w, y + 2.0, close.left - GAP, y + 2.0 + CHIP);
        let hot = p.hover == Some(Target::Quick(Hit::Devices));
        p.scene.fills.push(Fill::solid(r, CHIP / 2.0, if hot { Ink::Hover } else { Ink::Field }));
        p.scene.hits.push((r, Target::Quick(Hit::Devices)));
        put(p, &word, &centred(&f), r, Ink::Muted, 1.0);
    }
    y + BAR + PAD * 0.75 + GAP
}

/// A bar's band across the popup (its top, its composer): the panel's
/// tone, and a soft shadow where it meets the well.
fn band(p: &mut Painter, r: Rect) {
    p.scene.fills.push(Fill::shadow(r, 0.0, 8.0, 0.9));
    p.scene.fills.push(Fill::solid(r, 0.0, Ink::Panel));
}

/// A card lifted off the well: a tight shadow under it and a soft, wide
/// one, then its fill. Never a rim.
fn lifted(p: &mut Painter, r: Rect, radius: f32) {
    p.scene.fills.push(Fill::shadow(r.offset(0.0, 1.0), radius, 2.0, 0.9));
    p.scene.fills.push(Fill::shadow(r.offset(0.0, 3.0), radius, 10.0, 1.1));
    p.scene.fills.push(Fill::solid(r, radius, Ink::Raised));
}

fn beamed(p: &mut Painter, all: Rect, to: &str, what: &str) -> Result<()> {
    let mid = (all.top + all.bottom) / 2.0;
    let x0 = all.left + PAD;
    let circle = Rect::new(x0, mid - 18.0, x0 + 36.0, mid + 18.0);
    p.scene.fills.push(Fill::solid(circle, 18.0, Ink::Field));
    put(p, "\u{E73E}", &centred(&p.fonts.icon.clone()), circle, Ink::Text, 1.0);
    let x = circle.right + 12.0;
    let line = format!("{VERB_DONE} to {to}");
    put(p, &line, &bold(&label(p)), Rect::new(x, mid - 20.0, all.right - PAD, mid + 2.0), Ink::Text, 1.0);
    put(p, what, &small(p), Rect::new(x, mid + 2.0, all.right - PAD, mid + 20.0), Ink::Muted, 1.0);
    Ok(())
}

fn first_use(p: &mut Painter, view: &PopupView, all: Rect) -> Result<()> {
    let mut y = top(p, all, None, None);
    let (x0, x1) = (all.left + PAD, all.right - PAD);
    let w = x1 - x0;
    let title_format = Format { size: 19.0, weight: 600, line: Some(25.0), wrap: true, center_y: false, ..p.fonts.label.clone() };
    let first = format!("It shows in this window. Press Enter to {VERB_LOWER} it.");
    let keys_step = format!("Copy it, then press {}", view.host.keys);
    let steps = [
        (keys_step.as_str(), first.as_str()),
        ("It arrives on your other devices", "As a notification, and in this window, with Copy."),
        ("Pair your devices once", "A code shows here: scan it with your phone, or type it on the other device."),
    ];
    let body = wrapped(p);
    let tx = x0 + 38.0;
    // Measured before anything is placed: on a short screen (a laptop at
    // 200%) the steps' second lines go first, then the title; what clips
    // are against notes always stays.
    let title_text = format!("Get something from this {} to your phone or other computer, now.", view.host.this);
    let title = scene::layout(p.eng, &title_text, &title_format, w, 80.0);
    let title_h = title.measure().height.min(80.0) + 16.0;
    let subs: Vec<std::rc::Rc<Layout>> = steps.iter().map(|(_, s)| scene::layout(p.eng, s, &body, x1 - tx, 60.0)).collect();
    let subs_h: f32 = subs.iter().map(|l| l.measure().height.min(60.0)).sum();
    let what = "For a link, a code, a password, a few lines. Your notes don't need this: they stay the same on every device by themselves.";
    let what_l = scene::layout(p.eng, what, &body, w - 24.0, 120.0);
    let box_h = what_l.measure().height.min(120.0) + 20.0;
    let room = all.bottom - PAD - 40.0 - 16.0 - y;
    let steps_h = 3.0 * (22.0 + 14.0);
    let (show_title, show_subs) = if title_h + steps_h + subs_h + box_h <= room {
        (true, true)
    } else if title_h + steps_h + box_h <= room {
        (true, false)
    } else {
        (false, false)
    };
    if show_title {
        p.scene.texts.push(Text { x: x0, y, layout: title, ink: Ink::Text, alpha: 1.0 });
        y += title_h;
    }
    for (i, (head, _)) in steps.iter().enumerate() {
        let n = Rect::new(x0, y, x0 + 26.0, y + 26.0);
        p.scene.fills.push(Fill::solid(n, 13.0, Ink::Field));
        put(p, &(i + 1).to_string(), &centred(&bold(&small(p))), n, Ink::Text, 1.0);
        put(p, head, &bold(&label(p)), Rect::new(tx, y + 2.0, x1, y + 24.0), Ink::Text, 1.0);
        let mut h = 0.0;
        if show_subs {
            h = subs[i].measure().height.min(60.0);
            p.scene.texts.push(Text { x: tx, y: y + 24.0, layout: subs[i].clone(), ink: Ink::Muted, alpha: 1.0 });
        }
        y += 22.0 + h + 14.0;
    }
    // What it is for, against the notes.
    let r = Rect::new(x0, y, x1, y + box_h);
    lifted(p, r, CARD_RADIUS);
    p.scene.texts.push(Text { x: x0 + 12.0, y: y + 10.0, layout: what_l, ink: Ink::Muted, alpha: 1.0 });
    if let Some(said) = view.said {
        put(p, said, &small(p), Rect::new(x0, all.bottom - PAD - 40.0 - GAP - 20.0, x1, all.bottom - PAD - 40.0 - GAP), Ink::Muted, 1.0);
    }
    let b = all.bottom - PAD - 40.0;
    let not_now_w = 104.0;
    button(p, Rect::new(x0, b, x1 - not_now_w - GAP, b + 40.0), "Pair your devices", Hit::PairNow, true);
    button(p, Rect::new(x1 - not_now_w, b, x1, b + 40.0), "Not now", Hit::NotNow, false);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn main(p: &mut Painter, view: &PopupView, all: Rect, staged: Option<&Staged>, to: &[&str], chosen: usize, clips: &[ClipView], scroll: f32, saved: Option<&str>, draft: &str, more: Option<usize>) -> Result<()> {
    let list_top = top(p, all, None, Some(view.devices));
    let (x0, x1) = (all.left + PAD, all.right - PAD);
    let mut y = composer(p, x0, x1, all.bottom - PAD, list_top, staged, draft, to, chosen)? - PAD * 0.75;
    // Just above the field: "Saved as a note", and what was said.
    if let Some(name) = saved {
        let r = Rect::new(x0, y - GAP - 40.0, x1, y - GAP);
        lifted(p, r, CARD_RADIUS);
        put(p, "\u{E73E}", &centred(&p.fonts.icon_small.clone()), Rect::new(x0 + 8.0, r.top, x0 + 32.0, r.bottom), Ink::Muted, 1.0);
        put(p, &format!("Saved as a note, \u{201C}{name}\u{201D}"), &Format { wrap: false, ellipsis: true, ..small(p) }, Rect::new(x0 + 36.0, r.top, x1 - 64.0, r.bottom), Ink::Text, 1.0);
        let open = Rect::new(x1 - 60.0, r.top + 6.0, x1 - 6.0, r.bottom - 6.0);
        p.button(open, Target::Quick(Hit::OpenNote), CONTROL_RADIUS);
        put(p, "Open", &centred(&bold(&small(p))), open, Ink::Text, 1.0);
        y = r.top;
    }
    if let Some(said) = view.said {
        put(p, said, &small(p), Rect::new(x0, y - GAP - 20.0, x1, y - GAP), Ink::Muted, 1.0);
        y -= GAP + 20.0;
    }
    history(p, Rect::new(x0, list_top, x1, y - GAP), clips, scroll, more)
}

/// A card's measure: its width, its text's lines in all, how many it shows
/// open, and its height as open as it is.
struct Measure {
    w: f32,
    lines: usize,
    open_lines: usize,
    text_h: f32,
    h: f32,
}

/// Lines a card opens to in a list `list_h` tall: `OPEN_LINES`, or fewer
/// where the popup is short (most of the list's height).
/// `name` as it fits in `w`: whole, or its start and end with an ellipsis
/// between ("Screens…10-27.png"), so its kind stays readable.
fn fit_middle(p: &Painter, name: &str, f: &Format, w: f32) -> String {
    let fits = |s: &str| scene::single_line(p.eng, s, f, 2000.0, 20.0).measure().width <= w;
    if fits(name) {
        return name.to_string();
    }
    let chars: Vec<char> = name.chars().collect();
    let cut = |k: usize| {
        let (a, b) = (k.div_ceil(2), k / 2);
        format!("{}\u{2026}{}", chars[..a].iter().collect::<String>(), chars[chars.len() - b..].iter().collect::<String>())
    };
    let (mut lo, mut hi) = (0, chars.len());
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if fits(&cut(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    cut(lo)
}

/// A card's quiet line: who and when (its group's first card), and a
/// password's "Goes once copied".
fn meta_line(c: &ClipView) -> String {
    match (c.from.is_empty(), c.goes.is_empty()) {
        (false, false) => format!("{} \u{00B7} {}", c.from, c.goes),
        (false, true) => c.from.to_string(),
        (true, _) => c.goes.to_string(),
    }
}

fn open_limit(list_h: f32, line: f32) -> usize {
    ((list_h * 0.7 / line).floor() as usize).clamp(REST_LINES + 1, OPEN_LINES)
}

fn measure(p: &Painter, c: &ClipView, max_w: f32, limit: usize) -> Measure {
    let body = wrapped(p);
    let line = body.line.unwrap_or(18.0);
    let meta = meta_line(c);
    let meta_h = if meta.is_empty() { 0.0 } else { 4.0 + META };
    let meta_w = if meta.is_empty() { 0.0 } else { scene::single_line(p.eng, &meta, &small(p), 400.0, META).measure().width };
    // The text's column: the card's inside, less Copy's and "…"'s room at
    // its end (kept at rest too, so nothing moves under the pointer).
    let side = 2.0 * CARD_PAD + 2.0 * GLYPH + 4.0;
    let (w, lines, open_lines, text_h) = match c.kind {
        Kind::Text => {
            let (_, lines, widest) = card_text(p, c.text, max_w - side, limit);
            let rest = lines.min(REST_LINES);
            let open_lines = lines.min(limit);
            let shown = rest as f32 + (open_lines - rest) as f32 * c.open;
            let more = if lines > limit { (4.0 + META) * c.open } else { 0.0 };
            (widest + side, lines, open_lines, shown * line + more)
        }
        Kind::Link => {
            let title = if c.title.is_empty() { c.text } else { c.title };
            let tw = scene::single_line(p.eng, title, &bold(&label(p)), 1000.0, 20.0).measure().width;
            (tw + side, 1, 1, 20.0 + META)
        }
        Kind::Secret => (200.0, 1, 1, 20.0),
        Kind::Picture => {
            // The picture the card's width, flush to its top; several, a
            // grid of them; its name, or how many, under it.
            let tw = if c.more.is_empty() { PICTURE_W } else { GRID_W }.min(max_w);
            let th = if c.more.is_empty() { c.picture.as_ref().map_or(120.0, |pic| thumb_height(pic, tw).clamp(80.0, 220.0)) } else { grid_height(1 + c.more.len(), tw) };
            (tw, 1, 1, th - CARD_PAD + 8.0 + GLYPH)
        }
    };
    let w = w.max(meta_w + 2.0 * CARD_PAD).clamp(140.0_f32.min(max_w), max_w).ceil();
    Measure { w, lines, open_lines, text_h, h: (2.0 * CARD_PAD + text_h + meta_h).ceil() }
}

/// The timeline: newest at the foot, just above the field, older ones
/// going up as a chat's do; this device's own on the right, what came in
/// on the left; each as wide as it needs. `scroll` is how far up it has
/// gone, as far as the oldest one's top and no further. It is its own
/// layer, clipped: a card half scrolled out is cut at its edge, never
/// drawn or clicked over what is around it. A card's "…" menu opens over
/// it, above everything.
fn history(p: &mut Painter, list: Rect, clips: &[ClipView], scroll: f32, more: Option<usize>) -> Result<()> {
    p.scene.hits.push((list, Target::Quick(Hit::List)));
    if clips.is_empty() {
        p.scene.quick_scroll_max = 0.0;
        put(p, &format!("What you {VERB_LOWER} or receive shows here."), &centred(&small(p)), Rect::new(list.left, list.bottom - 56.0, list.right, list.bottom - 24.0), Ink::Muted, 1.0);
        return Ok(());
    }
    let line = wrapped(p).line.unwrap_or(18.0);
    let limit = open_limit(list.height(), line);
    let max_w = list.width() - GUTTER;
    let sizes: Vec<Measure> = clips.iter().map(|c| measure(p, c, max_w, limit)).collect();
    let content = sizes.iter().map(|m| m.h).sum::<f32>() + GAP * (sizes.len() as f32 - 1.0);
    let most = (content - list.height()).max(0.0);
    p.scene.quick_scroll_max = most;
    let hits_from = p.scene.hits.len();
    p.scene.overlay(Some(list));
    let mut bottom = list.bottom + scroll.clamp(0.0, most);
    let mut menu_at = None;
    for (i, (c, m)) in clips.iter().zip(&sizes).enumerate() {
        let r = if c.mine { Rect::new(list.right - m.w, bottom - m.h, list.right, bottom) } else { Rect::new(list.left, bottom - m.h, list.left + m.w, bottom) };
        bottom = r.top - GAP;
        if r.top >= list.bottom {
            continue;
        }
        if r.bottom <= list.top {
            break;
        }
        if let Some(at) = clip(p, c, i, r, m, list, more == Some(i))? {
            menu_at = Some((i, at, c.kind, !c.more.is_empty()));
        }
    }
    p.scene.overlay(None);
    let mut kept = Vec::with_capacity(p.scene.hits.len());
    for (k, (r, t)) in p.scene.hits.drain(..).enumerate() {
        if k < hits_from || matches!(t, Target::Quick(Hit::List)) {
            kept.push((r, t));
            continue;
        }
        let cut = Rect::new(r.left.max(list.left), r.top.max(list.top), r.right.min(list.right), r.bottom.min(list.bottom));
        if cut.right > cut.left && cut.bottom > cut.top {
            kept.push((cut, t));
        }
    }
    p.scene.hits = kept;
    // The open "…" menu: a Kolada menu, under its button (over it near the
    // foot), inside the list.
    if let Some((i, at, kind, group)) = menu_at {
        let mut items: Vec<(&str, &str, Hit)> = Vec::new();
        if kind == Kind::Link {
            items.push(("\u{E8A7}", "Open link", Hit::Open(i)));
        }
        if kind == Kind::Picture {
            items.push(("\u{E896}", if group { "Save all\u{2026}" } else { "Save picture\u{2026}" }, Hit::SaveFile(i)));
        }
        if kind != Kind::Secret {
            items.push(("\u{E7C3}", "Save as note", Hit::SaveNote(i)));
        }
        items.push(("\u{E74D}", "Remove", Hit::Forget(i)));
        let w = 200.0;
        let h = 12.0 + items.len() as f32 * crate::chrome::MENU_ITEM;
        let left = (at.right - w).clamp(list.left, list.right - w);
        let top = if at.bottom + 4.0 + h <= list.bottom { at.bottom + 4.0 } else { (at.top - 4.0 - h).max(list.top) };
        let card = Rect::new(left, top, left + w, top + h);
        p.scene.overlay(None);
        p.panel(card, 1.0);
        p.scene.hits.push((card, Target::Quick(Hit::Popup)));
        for (k, (g, word, hit)) in items.iter().enumerate() {
            let row = Rect::new(card.left + 6.0, card.top + 6.0 + crate::chrome::MENU_ITEM * k as f32, card.right - 6.0, card.top + 6.0 + crate::chrome::MENU_ITEM * (k as f32 + 1.0));
            crate::chrome::menu_row(p, row, Target::Quick(*hit), crate::chrome::RowIcon::Glyph(g), word, "", 0.0, false)?;
        }
    }
    Ok(())
}

/// One paragraph of a card's text, placed: where its text goes (from the
/// column's left and the text's top), and its list marker's, if any.
struct Placed {
    x: f32,
    y: f32,
    layout: std::rc::Rc<Layout>,
    marker: Option<(f32, std::rc::Rc<Layout>)>,
}

/// A card's text as the editor sets it, in a column `w` wide: each
/// paragraph in its own direction (its first strong letter's); a list
/// item's marker at the paragraph's start (the right, in Arabic), set apart
/// from the text, its lines hung under the item's text. At most `n` lines,
/// the last cut with an ellipsis. What is placed, the lines in all (counted
/// to `n + 1` at most: enough to know it is cut), and the widest line.
fn card_text(p: &Painter, text: &str, w: f32, n: usize) -> (Vec<Placed>, usize, f32) {
    let body = wrapped(p);
    let line = body.line.unwrap_or(18.0);
    let paragraphs: Vec<_> = kolada_markdown::lines(text).collect();
    let (mut out, mut used, mut widest) = (Vec::new(), 0usize, 0.0f32);
    for (i, l) in paragraphs.iter().enumerate() {
        if used > n {
            break;
        }
        let list = l.block.is_item();
        let content = &text[if list { l.content() } else { l.start }..l.end];
        let rtl = scene::is_rtl(content);
        let f = Format { rtl: Some(rtl), ..body.clone() };
        // The marker, set left to right whatever the paragraph's direction
        // (isolated): a bullet, a task's box, or the item's number.
        let marker = list.then(|| {
            let m = match l.block {
                kolada_markdown::Block::Task { done: true } => "\u{2713}",
                kolada_markdown::Block::Task { .. } => "\u{25CB}",
                kolada_markdown::Block::Bullet => "\u{2022}",
                _ => text[l.start..l.content()].trim(),
            };
            scene::single_line(p.eng, m, &Format { rtl: Some(false), wrap: false, ..body.clone() }, 60.0, line)
        });
        let depth = if list { l.depth as f32 * 16.0 } else { 0.0 };
        let indent = depth + marker.as_ref().map_or(0.0, |m| m.measure().width + 6.0);
        let tw = (w - indent).max(40.0);
        let full = scene::layout(p.eng, content, &f, tw, f32::MAX);
        let count = full.lines().len().max(1);
        let room = n.saturating_sub(used);
        if room > 0 {
            // The last paragraph shown, with more after it: cut, and the
            // ellipsis says so.
            let more = count > room || (count == room && i + 1 < paragraphs.len());
            let layout = if more { first_lines(p, &format!("{content}\u{2026}"), &f, tw, room).0 } else { full.clone() };
            let x = if rtl { 0.0 } else { indent };
            let mx = marker.map(|m| {
                let mw = m.measure().width;
                (if rtl { w - depth - mw } else { depth }, m)
            });
            widest = widest.max(full.measure().width + indent);
            out.push(Placed { x, y: used as f32 * line, layout, marker: mx });
        }
        used += count;
    }
    (out, used.max(1), widest)
}

/// `text` in at most `n` lines of `w`, cut with an ellipsis: a clip may be
/// pages long. Its layout, and its height.
fn first_lines(p: &Painter, text: &str, f: &Format, w: f32, n: usize) -> (std::rc::Rc<Layout>, f32) {
    let line = f.line.unwrap_or(18.0);
    let l = scene::layout(p.eng, text, f, w, f32::MAX);
    if l.lines().len() <= n {
        let h = (l.lines().len().clamp(1, n) as f32) * line;
        return (l, h);
    }
    // The longest start that fits in `n` lines with an ellipsis, found by
    // halving: a line's end offsets restart in each paragraph, so they
    // can't say where the n-th line ends. Only the first n paragraphs, and
    // as many letters as n lines could hold, are looked at.
    let most = text.split_inclusive('\n').take(n).map(str::len).sum::<usize>().min(n * 400);
    let mut ends: Vec<usize> = text.char_indices().map(|(i, _)| i).take_while(|&i| i <= most).collect();
    ends.push(most.min(text.len()));
    let cut = |k: usize| {
        let mut c = text[..ends[k]].trim_end().to_string();
        c.push('\u{2026}');
        c
    };
    let fits = |k: usize| scene::layout(p.eng, &cut(k), f, w, f32::MAX).lines().len() <= n;
    let (mut lo, mut hi) = (0, ends.len() - 1);
    if fits(hi) {
        lo = hi;
    }
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let l = scene::layout(p.eng, &cut(lo), f, w, f32::MAX);
    let h = (l.lines().len().clamp(1, n) as f32) * line;
    (l, h)
}

/// A picture card's thumbnail, at most this tall.
const THUMB_H: f32 = 180.0;

/// A card holding several pictures: its width, the grid's inset from its
/// edges and gutter (so each tile's corners sit concentric in the card's),
/// and the most tiles it shows (the last says how many more).
const GRID_W: f32 = 264.0;
const GRID_INSET: f32 = 4.0;
const GRID_TILES: usize = 4;

/// A picture ready to beam, one of several: its square.
const STAGED_TILE: f32 = 40.0;

/// How tall the grid of `n` pictures is, `w` wide: two side by side; three
/// as one wide over two; four or more two by two.
fn grid_height(n: usize, w: f32) -> f32 {
    let cell = (w - 2.0 * GRID_INSET - GRID_INSET) / 2.0;
    match n {
        0 | 1 => cell,
        2 => 2.0 * GRID_INSET + cell,
        _ => 2.0 * GRID_INSET + 2.0 * cell + GRID_INSET,
    }
}

/// The grid of a card holding several pictures, in `r`: each tile its
/// picture cut to fill it, its corners concentric in the card's; past
/// `GRID_TILES`, the last says how many more. Each opens on a tap.
fn grid(p: &mut Painter, i: usize, first: &PicView, rest: &[PicView], r: Rect) {
    let all: Vec<&PicView> = std::iter::once(first).chain(rest.iter()).collect();
    let n = all.len();
    let (x0, x1, y0) = (r.left + GRID_INSET, r.right - GRID_INSET, r.top + GRID_INSET);
    let cell = (x1 - x0 - GRID_INSET) / 2.0;
    let half = |k: usize, top: f32| {
        let left = if k % 2 == 0 { x0 } else { x0 + cell + GRID_INSET };
        Rect::new(left, top, left + cell, top + cell)
    };
    let rects: Vec<Rect> = match n {
        2 => vec![half(0, y0), half(1, y0)],
        3 => vec![Rect::new(x0, y0, x1, y0 + cell), half(0, y0 + cell + GRID_INSET), half(1, y0 + cell + GRID_INSET)],
        _ => (0..GRID_TILES).map(|k| half(k, y0 + (k / 2) as f32 * (cell + GRID_INSET))).collect(),
    };
    let round = CARD_RADIUS - GRID_INSET;
    for (k, t) in rects.iter().enumerate().take(n) {
        tile(p, all[k], *t, round);
        p.scene.hits.push((*t, Target::Quick(Hit::Tile(i, k))));
    }
    if n > GRID_TILES {
        let t = rects[GRID_TILES - 1];
        // Over the pictures: a layer of its own, clipped as this one is.
        let clip = p.scene.overlays.last().and_then(|o| o.2);
        p.scene.overlay(clip);
        p.scene.fills.push(Fill { alpha: 0.5, ..Fill::solid(t, round, Ink::Scrim) });
        put(p, &format!("+{}", n - GRID_TILES + 1), &centred(&Format { size: 20.0, weight: 600, ..label(p) }), t, Ink::OnAccent, 1.0);
    }
}

/// A picture cut to fill `r`, its middle kept (`crop`), corners `round`;
/// before its thumbnail is decoded, the picture glyph on an inset.
fn tile(p: &mut Painter, pic: &PicView, r: Rect, round: f32) {
    match &pic.pixels {
        Some(px) => {
            let layer = p.scene.layer_now();
            let pixels = crop(px, r.width() / r.height().max(1.0));
            p.scene.pictures.push(crate::scene::Picture { layer, rect: r, pixels, alpha: 1.0, round });
        }
        None => {
            p.scene.fills.push(Fill::solid(r, round, Ink::Field));
            put(p, "\u{E91B}", &centred(&p.fonts.icon_small.clone()), r, Ink::Faint, 1.0);
        }
    }
}

thread_local! {
    /// Thumbnails cut to a tile's shape, by the thumbnail and the shape:
    /// made once, not each frame; the oldest go past a few dozen.
    static CROPS: std::cell::RefCell<Vec<(usize, u32, std::sync::Arc<crate::pixels::Pixels>, std::sync::Arc<crate::pixels::Pixels>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `px`'s middle at the shape `aspect` (width over height).
fn crop(px: &std::sync::Arc<crate::pixels::Pixels>, aspect: f32) -> std::sync::Arc<crate::pixels::Pixels> {
    let key = (std::sync::Arc::as_ptr(px) as usize, (aspect * 1000.0) as u32);
    if let Some(hit) = CROPS.with(|c| c.borrow().iter().find(|e| (e.0, e.1) == key && std::sync::Arc::ptr_eq(&e.2, px)).map(|e| e.3.clone())) {
        return hit;
    }
    let (w, h) = (px.width.max(1), px.height.max(1));
    let (cw, ch) = if w as f32 / h as f32 > aspect { (((h as f32) * aspect).round().max(1.0) as u32, h) } else { (w, ((w as f32) / aspect).round().max(1.0) as u32) };
    let (ox, oy) = ((w - cw.min(w)) / 2, (h - ch.min(h)) / 2);
    let mut data = Vec::with_capacity((cw * ch) as usize);
    for y in 0..ch.min(h) {
        let row = ((oy + y) * w + ox) as usize;
        data.extend_from_slice(&px.data[row..row + cw.min(w) as usize]);
    }
    let out = std::sync::Arc::new(crate::pixels::Pixels { width: cw.min(w), height: ch.min(h), data });
    CROPS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= 48 {
            c.remove(0);
        }
        c.push((key.0, key.1, px.clone(), out.clone()));
    });
    out
}

fn thumb_height(pic: &PicView, w: f32) -> f32 {
    let aspect = if pic.width > 0 && pic.height > 0 { pic.height as f32 / pic.width as f32 } else { 0.6 };
    (w * aspect).clamp(56.0, THUMB_H)
}

/// A card, one raised fill whoever sent it: what it holds (a text's first
/// lines, opening on a tap; a link's title over its site; a picture; a
/// password's dots), Copy at its top right, "…" beside it under the
/// pointer, and its group's faint line under it. Where its "…" is, when its
/// menu is open.
#[allow(clippy::too_many_arguments)]
fn clip(p: &mut Painter, c: &ClipView, i: usize, r: Rect, m: &Measure, list: Rect, menu: bool) -> Result<Option<Rect>> {
    lifted(p, r, CARD_RADIUS);
    p.scene.hits.push((r, Target::Quick(Hit::Card(i))));
    // Sent from here: its mark beside it, in the gutter, on its middle.
    if c.mine {
        sent_mark(p, r.left - GAP - FIELD_BUTTON / 2.0, (r.top + r.bottom) / 2.0, c.sent, Some(Hit::Retry(i)));
    }
    let (x0, x1) = (r.left + CARD_PAD, r.right - CARD_PAD);
    let f = small(p);
    let body = wrapped(p);
    let line = body.line.unwrap_or(18.0);
    // A picture runs to the card's edges at its top, cut to its corners;
    // what the card says goes under it.
    let pic_h = match (&c.picture, c.kind) {
        (Some(_), Kind::Picture) => m.text_h + CARD_PAD - 8.0 - GLYPH,
        _ => 0.0,
    };
    match (&c.picture, pic_h > 0.0) {
        (Some(pic), true) if !c.more.is_empty() => grid(p, i, pic, c.more, Rect::new(r.left, r.top, r.right, r.top + pic_h)),
        (Some(pic), true) => cover(p, pic, Rect::new(r.left, r.top, r.right, r.top + pic_h), CARD_RADIUS),
        _ => {}
    }
    let y = if pic_h > 0.0 { r.top + pic_h + 8.0 } else { r.top + CARD_PAD };
    // Copy, and "…" under the pointer (or while its menu is open).
    let gy = if pic_h > 0.0 { y - 2.0 } else { r.top + 6.0 };
    let copy = Rect::new(r.right - 6.0 - GLYPH, gy, r.right - 6.0, gy + GLYPH);
    let spent = c.kind == Kind::Secret && (c.spent || c.mine);
    if !spent {
        glyph(p, copy, if c.copied { "\u{E73E}" } else { "\u{E8C8}" }, Hit::Copy(i));
    }
    let hovered = matches!(p.hover, Some(Target::Quick(Hit::Card(k) | Hit::Copy(k) | Hit::More(k) | Hit::ReadMore(k))) if k == i);
    let more = Rect::new(copy.left - GLYPH, copy.top, copy.left, copy.bottom);
    if hovered || menu {
        let hot = menu || p.hover == Some(Target::Quick(Hit::More(i)));
        if hot {
            p.scene.fills.push(Fill::solid(more, CONTROL_RADIUS, Ink::Hover));
        }
        p.scene.hits.push((more, Target::Quick(Hit::More(i))));
        dots(p, more, if hot { Ink::Text } else { Ink::Muted });
    }
    let column = Rect::new(x0, y, more.left - 4.0, y + m.text_h);
    match c.kind {
        Kind::Text => {
            // Two lines at rest, cut with an ellipsis; opening, the lines up
            // to the limit come in as the card grows (clipped to it).
            let n = if c.open > 0.0 { m.open_lines } else { REST_LINES };
            let (placed, _, _) = card_text(p, c.text, column.width(), n);
            let shown = line * (m.lines.min(REST_LINES) as f32 + (m.open_lines - m.lines.min(REST_LINES)) as f32 * c.open);
            let clip = Rect::new(r.left, y - 2.0, r.right, (y + shown + 2.0).min(r.bottom));
            p.scene.overlay(Some(Rect::new(clip.left.max(list.left), clip.top.max(list.top), clip.right.min(list.right), clip.bottom.min(list.bottom))));
            for pl in placed {
                if let Some((mx, ml)) = pl.marker {
                    p.scene.texts.push(Text { x: x0 + mx, y: y + pl.y, layout: ml, ink: Ink::Muted, alpha: 1.0 });
                }
                p.scene.texts.push(Text { x: x0 + pl.x, y: y + pl.y, layout: pl.layout, ink: Ink::Text, alpha: 1.0 });
            }
            p.scene.overlay(Some(list));
            // Cut past the limit: "Read more" opens it as a note.
            if m.lines > m.open_lines && c.open > 0.5 {
                let at = y + shown + 4.0;
                let w = scene::single_line(p.eng, "Read more", &f, 200.0, META).measure().width + 8.0;
                let rr = Rect::new(x0 - 4.0, at, x0 - 4.0 + w, at + META);
                let hot = p.hover == Some(Target::Quick(Hit::ReadMore(i)));
                p.scene.hits.push((rr, Target::Quick(Hit::ReadMore(i))));
                put(p, "Read more", &centred(&f), rr, if hot { Ink::Text } else { Ink::Muted }, ((c.open - 0.5) * 2.0).min(1.0));
            }
        }
        Kind::Link => {
            let title = if c.title.is_empty() { c.text } else { c.title };
            put(p, title, &Format { wrap: false, ellipsis: true, ..bold(&label(p)) }, Rect::new(x0, y, column.right, y + 20.0), Ink::Link, 1.0);
            let domain = c.text.split("://").nth(1).unwrap_or(c.text).split('/').next().unwrap_or("");
            put(p, domain, &f, Rect::new(x0, y + 20.0, column.right, y + 20.0 + META), Ink::Muted, 1.0);
        }
        Kind::Picture if !c.more.is_empty() => {
            put(p, &format!("{} pictures", 1 + c.more.len()), &Format { wrap: false, ..f.clone() }, Rect::new(x0, y, more.left - 4.0 + 8.0, y + GLYPH), Ink::Muted, 1.0);
        }
        Kind::Picture => {
            // Its name cut in the middle where it must be; its size never.
            let size = c.picture.as_ref().map_or("", |pic| pic.size);
            let tail = if size.is_empty() { String::new() } else { format!(" \u{00B7} {size}") };
            let tail_w = scene::single_line(p.eng, &tail, &f, 400.0, GLYPH).measure().width;
            let name = fit_middle(p, c.text, &f, (more.left - 4.0 - x0 - tail_w).max(24.0));
            put(p, &format!("{name}{tail}"), &Format { wrap: false, ..f.clone() }, Rect::new(x0, y, more.left - 4.0 + 8.0, y + GLYPH), Ink::Muted, 1.0);
        }
        Kind::Secret => {
            let icon = centred(&p.fonts.icon_small.clone());
            put(p, "\u{E72E}", &icon, Rect::new(x0, y, x0 + 16.0, y + 20.0), Ink::Muted, 1.0);
            let shown = if c.spent { "Copied. Gone from your devices.".to_string() } else if c.mine { "A password you beamed".to_string() } else { "\u{2022}".repeat(12) };
            put(p, &shown, &f, Rect::new(x0 + 24.0, y, column.right, y + 20.0), Ink::Muted, 1.0);
        }
    }
    // The group's quiet line, muted (never faint): who and when; a
    // password's "Goes once copied".
    let line = meta_line(c);
    if !line.is_empty() {
        let at = r.bottom - CARD_PAD - META;
        put(p, &line, &Format { wrap: false, ellipsis: true, ..f }, Rect::new(x0, at, x1, at + META), Ink::Muted, 1.0);
    }
    Ok(menu.then_some(more))
}

fn devices_screen(p: &mut Painter, view: &PopupView, all: Rect, devices: &[DeviceRow], pairing: &Pairing, facts: &[String], naming: Option<&str>) -> Result<()> {
    let y = top(p, all, Some("Devices"), None);
    let mut v = crate::scene::DevicesView { devices, pairing: pairing.clone(), said: view.said, facts, this: view.host.this, naming };
    // A short popup, while a code shows: the code alone, the list back
    // once it goes.
    if y + devices_height(&v) > all.bottom - PAD {
        v.devices = &[];
    }
    devices_panel(p, &v, all.left + PAD, all.right - PAD, y)?;
    Ok(())
}

/// The card with a code to pair: its title row, the QR (140 DIPs with its
/// margin, for the link to pair) and its foot.
const SHOWING_H: f32 = 48.0 + 140.0 + 16.0;

/// A device's mark: the side menu's marks' size.
const DEVICE_MARK: f32 = 18.0;

/// A row of the devices panel, and a button's height in it.
const DEVICE_ROW: f32 = 40.0;
const DEVICE_BUTTON: f32 = 32.0;

/// A button's corners, as Settings' buttons have them.
const BUTTON_RADIUS: f32 = 6.0;

/// How tall `devices_panel` is for `v`.
pub fn devices_height(v: &crate::scene::DevicesView) -> f32 {
    let pairing = match v.pairing {
        Pairing::Idle | Pairing::Opening | Pairing::Waiting | Pairing::Entering { .. } => DEVICE_ROW,
        Pairing::Showing { .. } => SHOWING_H,
        Pairing::Asks { .. } => 88.0,
    };
    v.devices.len() as f32 * DEVICE_ROW + GAP + pairing + (v.said.is_some() as usize + v.facts.len()) as f32 * 24.0
}

/// A small button in the panel, as Settings' own are: the button's fill,
/// a step off what it sits on, its words in the full text ink; darker
/// under the pointer. What is asked for is the strongest: the text ink
/// itself, its words the page's, in their weight (teal is kept for the
/// caret and the beam).
fn panel_button(p: &mut Painter, r: Rect, word: &str, hit: Hit, strong: bool) {
    let hot = p.hover == Some(Target::Quick(hit));
    let (fill, ink) = if strong { (Ink::Text, Ink::Page) } else { (if hot { Ink::Pressed } else { Ink::Button }, Ink::Text) };
    p.scene.fills.push(Fill { alpha: if strong && hot { 0.85 } else { 1.0 }, ..Fill::solid(r, BUTTON_RADIUS, fill) });
    p.scene.hits.push((r, Target::Quick(hit)));
    let f = if strong { bold(&p.fonts.choice) } else { p.fonts.choice.clone() };
    put(p, word, &centred(&f), r, ink, 1.0);
}

/// A quiet button: its word alone, the hover's fill under the pointer.
fn quiet_button(p: &mut Painter, r: Rect, word: &str, hit: Hit) {
    let hot = p.hover == Some(Target::Quick(hit));
    if hot {
        p.scene.fills.push(Fill::solid(r, BUTTON_RADIUS, Ink::Hover));
    }
    p.scene.hits.push((r, Target::Quick(hit)));
    let f = centred(&p.fonts.choice);
    put(p, word, &f, r, if hot { Ink::Text } else { Ink::Muted }, 1.0);
}

/// A field being typed in, on a row: its soft fill, the text (or what it
/// shows while empty, faint) and the caret at its end.
fn typed_field(p: &mut Painter, field: Rect, typed: &str, empty: &str, hit: Hit, mono: bool) {
    p.scene.fills.push(Fill::solid(field, BUTTON_RADIUS, Ink::Field));
    p.scene.hits.push((field, Target::Quick(hit)));
    let f = if mono { Format { size: 17.0, weight: 600, line: Some(24.0), center_y: true, wrap: false, ..p.fonts.mono.clone() } } else { Format { center_y: true, wrap: false, ..label(p) } };
    let shown = if typed.is_empty() { empty } else { typed };
    let t = Rect::new(field.left + 12.0, field.top, field.right - 12.0, field.bottom);
    let l = scene::single_line(p.eng, shown, &f, t.width(), t.height());
    let cx = if typed.is_empty() { 0.0 } else { l.caret(typed.len()).x.min(t.width()) };
    p.scene.texts.push(Text { x: t.left, y: t.top, layout: l, ink: if typed.is_empty() { Ink::Faint } else { Ink::Text }, alpha: 1.0 });
    let mid = (t.top + t.bottom) / 2.0;
    p.scene.fills.push(Fill::solid(Rect::new(t.left + cx - CARET_WIDTH / 2.0, mid - 10.0, t.left + cx + CARET_WIDTH / 2.0, mid + 10.0), CARET_WIDTH / 2.0, Ink::Accent));
}

fn word_w(p: &Painter, word: &str) -> f32 {
    scene::single_line(p.eng, word, &p.fonts.choice, 300.0, DEVICE_BUTTON).measure().width + 32.0
}

/// The paired devices and pairing, from `y` across `x0`..`x1`: each device
/// on a row (its kind's icon, its name, "This one" or Remove); then what
/// pairing asks for now: Pair a device and Enter a code; the code, its time
/// left and Cancel; "X wants to pair", Allow and No; the code being typed,
/// Pair and Cancel; then what was said, and quiet facts. Settings >
/// Devices draws it, and so does the popup where a shell shows pairing
/// there. Where it ends.
pub(crate) fn devices_panel(p: &mut Painter, v: &crate::scene::DevicesView, x0: f32, x1: f32, mut y: f32) -> Result<f32> {
    let lf = label(p);
    let f = small(p);
    for (i, d) in v.devices.iter().enumerate() {
        let r = Rect::new(x0, y, x1, y + DEVICE_ROW);
        // Solid, as the side menu's marks and at their size, on the name's line.
        let (cx, cy) = (x0 + 14.0, (r.top + r.bottom) / 2.0);
        let half = DEVICE_MARK / 2.0;
        let layer = p.scene.layer_now();
        p.scene.vectors.push(crate::scene::VectorMark { layer, rect: Rect::new(cx - half, cy - half, cx + half, cy + half), logo: d.look.logo(), ink: Ink::Muted, alpha: 1.0 });
        let name = if d.name.is_empty() { "A device" } else { d.name };
        match (d.this, v.naming) {
            // This device's name being typed, then Save and Cancel.
            (true, Some(typed)) => {
                let (sw, cw) = (word_w(p, "Save"), word_w(p, "Cancel"));
                let field = Rect::new(x0 + 32.0, r.top + 4.0, x1 - sw - cw - 2.0 * GAP, r.top + 4.0 + DEVICE_BUTTON);
                typed_field(p, field, typed, name, Hit::NameField, false);
                let save = Rect::new(field.right + GAP, field.top, field.right + GAP + sw, field.bottom);
                panel_button(p, save, "Save", Hit::NameSave, true);
                panel_button(p, Rect::new(save.right + GAP, save.top, x1, save.bottom), "Cancel", Hit::NameCancel, false);
            }
            // Its name, "This one" beside it, and a quiet Rename.
            (true, None) => {
                let w = word_w(p, "Rename");
                let room = Rect::new(x0 + 32.0, y, x1 - w - GAP, r.bottom);
                let l = scene::single_line(p.eng, name, &lf, room.width().max(1.0), room.height());
                let nw = l.measure().width.min(room.width());
                p.scene.texts.push(Text { x: room.left, y: room.top, layout: l, ink: Ink::Text, alpha: 1.0 });
                put(p, "This one", &f, Rect::new(room.left + nw + 10.0, y, room.right, r.bottom), Ink::Faint, 1.0);
                quiet_button(p, Rect::new(x1 - w, y + 4.0, x1, y + 4.0 + DEVICE_BUTTON), "Rename", Hit::Rename);
            }
            (false, _) => {
                let w = word_w(p, "Remove");
                put(p, name, &lf, Rect::new(x0 + 32.0, y, x1 - w - GAP, r.bottom), Ink::Text, 1.0);
                let b = Rect::new(x1 - w, y + 4.0, x1, y + 4.0 + DEVICE_BUTTON);
                panel_button(p, b, "Remove", Hit::Remove(i), false);
            }
        }
        y = r.bottom;
    }
    y += GAP;
    let row = |y: f32| Rect::new(x0, y, x1, y + DEVICE_ROW);
    match &v.pairing {
        Pairing::Idle => {
            let r = row(y);
            let pair = Rect::new(x0, r.top + 4.0, x0 + word_w(p, "Pair a device"), r.top + 4.0 + DEVICE_BUTTON);
            panel_button(p, pair, "Pair a device", Hit::ShowCode, v.devices.len() < 2);
            let enter = Rect::new(pair.right + GAP, pair.top, pair.right + GAP + word_w(p, "Enter a code"), pair.bottom);
            panel_button(p, enter, "Enter a code", Hit::EnterCode, false);
            y = r.bottom;
        }
        Pairing::Opening | Pairing::Waiting => {
            let r = row(y);
            let line = if matches!(v.pairing, Pairing::Opening) { "Getting a code\u{2026}" } else { "Waiting for the other device\u{2026}" };
            let w = word_w(p, "Cancel");
            put(p, line, &lf, Rect::new(x0, r.top, x1 - w - GAP, r.bottom), Ink::Muted, 1.0);
            panel_button(p, Rect::new(x1 - w, r.top + 4.0, x1, r.top + 4.0 + DEVICE_BUTTON), "Cancel", Hit::Cancel, false);
            y = r.bottom;
        }
        Pairing::Showing { code, left } => {
            // The QR (the link to pair, the code in its fragment) and beside
            // it the code to type, and how long it lasts; Cancel at the
            // card's top right. Both go with the code.
            // On the page as the rows are, not on a card of its own: its
            // button keeps Settings' one fill.
            let r = Rect::new(x0, y, x1, y + SHOWING_H);
            put(p, "Pair a device", &bold(&lf), Rect::new(r.left, r.top + CARD_PAD, x1 - 96.0, r.top + CARD_PAD + 20.0), Ink::Text, 1.0);
            let w = word_w(p, "Cancel");
            panel_button(p, Rect::new(r.right - w, r.top + 8.0, r.right, r.top + 8.0 + DEVICE_BUTTON), "Cancel", Hit::Cancel, false);
            let q = crate::qr::draw(&mut p.scene.fills, r.left, r.top + 48.0, &crate::qr::pair_link(code));
            let column = Rect::new(q.right + 16.0, q.top, r.right, q.bottom);
            let mid = (column.top + column.bottom) / 2.0;
            let big = Format { size: 24.0, weight: 600, line: Some(32.0), wrap: false, ellipsis: true, ..p.fonts.mono.clone() };
            put(p, code, &big, Rect::new(column.left, mid - 40.0, column.right, mid - 8.0), Ink::Text, 1.0);
            let how = Format { wrap: true, ..f.clone() };
            let l = scene::layout(p.eng, "Scan it with your phone, or type it on the other device.", &how, column.width().max(1.0), 60.0);
            p.scene.texts.push(Text { x: column.left, y: mid, layout: l, ink: Ink::Muted, alpha: 1.0 });
            put(p, left, &f, Rect::new(column.left, column.bottom - 20.0, column.right, column.bottom), Ink::Faint, 1.0);
            y = r.bottom;
        }
        Pairing::Asks { name } => {
            let r = Rect::new(x0, y, x1, y + 88.0);
            let who = format!("{} wants to pair with this {}", if name.is_empty() { "A device" } else { name }, v.this);
            put(p, &who, &bold(&lf), Rect::new(r.left, r.top + CARD_PAD, r.right, r.top + CARD_PAD + 20.0), Ink::Text, 1.0);
            let by = r.bottom - CARD_PAD - DEVICE_BUTTON;
            let (aw, nw) = (word_w(p, "Allow"), word_w(p, "No"));
            panel_button(p, Rect::new(r.left, by, r.left + aw, by + DEVICE_BUTTON), "Allow", Hit::Allow, true);
            panel_button(p, Rect::new(r.left + aw + GAP, by, r.left + aw + GAP + nw, by + DEVICE_BUTTON), "No", Hit::Deny, false);
            y = r.bottom;
        }
        Pairing::Entering { typed } => {
            // The code being typed, then Pair and Cancel, on one row.
            let r = row(y);
            let (pw, cw) = (word_w(p, "Pair"), word_w(p, "Cancel"));
            let field = Rect::new(x0, r.top + 4.0, x1 - pw - cw - 2.0 * GAP, r.top + 4.0 + DEVICE_BUTTON);
            typed_field(p, field, typed, "XXXX-XXXX", Hit::CodeField, true);
            let pair = Rect::new(field.right + GAP, r.top + 4.0, field.right + GAP + pw, r.top + 4.0 + DEVICE_BUTTON);
            panel_button(p, pair, "Pair", Hit::Pair, true);
            panel_button(p, Rect::new(pair.right + GAP, pair.top, x1, pair.bottom), "Cancel", Hit::Cancel, false);
            y = r.bottom;
        }
    }
    for line in v.said.into_iter().chain(v.facts.iter().map(String::as_str)) {
        put(p, line, &f, Rect::new(x0, y + 4.0, x1, y + 24.0), Ink::Muted, 1.0);
        y += 24.0;
    }
    Ok(y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eng_fonts() -> (Eng, Fonts) {
        (Eng::new(kolada_text::Engine::new(crate::testing::fonts())), Fonts::new(1.0, 0))
    }

    fn rect_of(scene: &Scene, t: Hit) -> Option<Rect> {
        scene.hits.iter().find(|(_, x)| *x == Target::Quick(t)).map(|(r, _)| *r)
    }

    /// Where `t`'s glyphs put ink at `scale`, in points.
    fn ink(eng: &Eng, t: &Text, scale: f32) -> Rect {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut e = eng.borrow_mut();
        for g in t.layout.glyphs((t.x * scale, t.y * scale), scale) {
            if let Some(m) = e.mask(&g).filter(|m| m.width > 0 && m.height > 0) {
                let (left, top) = ((g.x + m.left) as f32, (g.y - m.top) as f32);
                x0 = x0.min(left);
                y0 = y0.min(top);
                x1 = x1.max(left + m.width as f32);
                y1 = y1.max(top + m.height as f32);
            }
        }
        Rect::new(x0 / scale, y0 / scale, x1 / scale, y1 / scale)
    }

    fn clips() -> Vec<ClipView<'static>> {
        let c = |kind, text, title, from| ClipView { sent: Sent::None, from, goes: "", kind, text, title, copied: false, spent: false, mine: false, picture: None, more: &[], open: 0.0 };
        vec![
            c(Kind::Text, "Gate B, row 14. Boarding at 19:05.", "", "From iPhone \u{00B7} 2 min"),
            ClipView { goes: "Goes once copied", ..c(Kind::Secret, "hunter2!Xq9", "", "From iPhone \u{00B7} 1 h") },
            c(Kind::Link, "https://developers.cloudflare.com/durable-objects/", "Durable Objects pricing", "You beamed \u{00B7} 3 h"),
            c(Kind::Text, "A long one: the agenda for Thursday, with the budget review, the Mac release and the sync design, and the notes from last week to bring along, printed, in the blue folder on the desk.", "", "From Laptop \u{00B7} Yesterday"),
            c(Kind::Text, "Door code for the studio: 4931", "", "From Laptop \u{00B7} 2 days"),
        ]
    }

    /// The field is a capsule; the beam and attach in it are circles inset
    /// alike from its top, bottom and end, their radius its end's less the
    /// inset (concentric), their glyphs in their middles. So at 100, 150
    /// and 300%, with Arabic typed, and when the field grows for lines:
    /// they stay on its last line, the same insets from its bottom.
    #[test]
    fn the_fields_buttons_are_concentric_with_its_end() {
        let (eng, _) = eng_fonts();
        let to = ["All devices", "Laptop"];
        for draft in ["Gate B", "\u{0628}\u{0648}\u{0627}\u{0628}\u{0629} \u{0628}", "Gate B, row 14\nBoarding at 19:05\nSeat 22A"] {
            for scale in [1.0, 1.5, 3.0] {
                let fonts = Fonts::new(scale, 0);
                let view = PopupView { screen: Screen::Main { staged: None, to: &to, chosen: 0, clips: &[], scroll: 0.0, saved: None, draft, more: None }, devices: 1, said: None, host: Host::WINDOWS };
                let scene = popup(&eng, &fonts, &view, None, W, H).unwrap();
                let field = scene.fills.iter().find(|f| f.ink == Ink::Field && f.rect.left <= PAD + 0.5 && f.rect.bottom > H - 80.0).expect("the field").clone();
                assert_eq!(field.radius, FIELD_MIN / 2.0, "{draft:?}: a capsule");
                let send = rect_of(&scene, Hit::Beam).expect("the beam");
                let attach = rect_of(&scene, Hit::Attach).expect("attach");
                for (name, b) in [("beam", send), ("attach", attach)] {
                    assert_eq!((b.width(), b.height()), (FIELD_BUTTON, FIELD_BUTTON), "{name}: a circle's box");
                    assert!((field.rect.bottom - b.bottom - FIELD_INSET).abs() < 0.01, "{draft:?} {name}: inset from the bottom");
                    assert!((b.width() / 2.0 - (field.radius - FIELD_INSET)).abs() < 0.01, "{name}: concentric with the end");
                    // Whole pixels at every scale: crisp, the same inset all round.
                    for v in [b.left, b.top, b.right, b.bottom] {
                        assert!((v * scale - (v * scale).round()).abs() < 0.01, "{scale} {name}: {v} on a pixel");
                    }
                }
                assert!((field.rect.right - send.right - FIELD_INSET).abs() < 0.01, "the beam inset from the end");
                if !draft.contains('\n') {
                    assert!((send.top - field.rect.top - FIELD_INSET).abs() < 0.01, "{draft:?}: one line, inset from the top too");
                } else {
                    assert!(field.rect.height() > FIELD_MIN, "the field grew");
                }
                // The beam mark (its ink square in its box) in the circle's middle.
                let mark = scene.vectors.iter().find(|v| v.logo == crate::vector::Logo::Beam && v.rect.left >= send.left && v.rect.right <= send.right).expect("the beam mark");
                let (cx, cy) = ((send.left + send.right) / 2.0, (send.top + send.bottom) / 2.0);
                assert!(((mark.rect.left + mark.rect.right) / 2.0 - cx).abs() < 0.01 && ((mark.rect.top + mark.rect.bottom) / 2.0 - cy).abs() < 0.01, "the mark centred");
                // The paperclip's ink in its circle's middle, within a pixel.
                let clip = scene.texts.iter().find(|t| t.layout.text() == "\u{E723}").expect("the paperclip");
                let i = ink(&eng, clip, scale);
                let (ax, ay) = ((attach.left + attach.right) / 2.0, (attach.top + attach.bottom) / 2.0);
                let (gx, gy) = ((i.left + i.right) / 2.0, (i.top + i.bottom) / 2.0);
                assert!((gx - ax).abs() <= 1.0 && (gy - ay).abs() <= 1.0, "{scale}: the paperclip's ink in its middle ({gx},{gy}) vs ({ax},{ay})");
            }
        }
    }

    /// A card's list as the editor sets it: in an Arabic paragraph the
    /// number sits at its start, the right, set apart from the text; an
    /// item's wrapped lines hang under its text, not under its marker.
    #[test]
    fn a_cards_list_keeps_its_markers_at_the_start_and_hangs_its_lines() {
        let (eng, fonts) = eng_fonts();
        let text_ar = "1. \u{0627}\u{0644}\u{0628}\u{0646}\u{062F} \u{0627}\u{0644}\u{0623}\u{0648}\u{0644}\n2. \u{0627}\u{0644}\u{0628}\u{0646}\u{062F} \u{0627}\u{0644}\u{062B}\u{0627}\u{0646}\u{064A}";
        let text_en = "- A long item that runs on past the card's width, so its second line has to hang under its own text and not under the bullet\n- Two";
        for (text, rtl) in [(text_ar, true), (text_en, false)] {
            let card = ClipView { sent: Sent::None, from: "", goes: "", kind: Kind::Text, text, title: "", copied: false, spent: false, mine: false, picture: None, more: &[], open: 1.0 };
            let cl = [card];
            let view = PopupView { screen: Screen::Main { staged: None, to: &["Laptop"], chosen: 0, clips: &cl, scroll: 0.0, saved: None, draft: "", more: None }, devices: 1, said: None, host: Host::WINDOWS };
            let scene = popup(&eng, &fonts, &view, None, W, H).unwrap();
            let markers: Vec<&Text> = scene.texts.iter().filter(|t| matches!(t.layout.text(), "1." | "2." | "\u{2022}")).collect();
            assert_eq!(markers.len(), 2, "{rtl}: a marker an item");
            for m in &markers {
                let item = scene.texts.iter().find(|t| (t.y - m.y).abs() < 0.5 && t.ink == Ink::Text).expect("the item's text");
                let mw = m.layout.measure().width;
                if rtl {
                    let right = item.x + item.layout.width();
                    assert!(m.x >= right, "the number at the right of its Arabic text ({} vs {right})", m.x);
                    assert!(item.layout.is_rtl(), "the item reads right to left");
                } else {
                    assert!(item.x >= m.x + mw, "the text after its bullet");
                }
            }
            if !rtl {
                let item = scene.texts.iter().find(|t| t.layout.text().starts_with("A long item")).unwrap();
                assert!(item.layout.lines().len() >= 2, "it wraps");
                // One layout at the item's indent: every line of it starts there.
                assert!(item.x > markers[0].x + markers[0].layout.measure().width, "its lines hang under its text");
            }
        }
    }

    /// A picture card's name is cut in its middle where it must be, so its
    /// kind still shows; its size is never cut.
    #[test]
    fn a_pictures_name_is_cut_in_its_middle_and_its_size_never() {
        let (eng, fonts) = eng_fonts();
        let pic = PicView { pixels: None, width: 1920, height: 1080, size: "1.2 MB", ready: true };
        let card = ClipView { sent: Sent::None, from: "", goes: "", kind: Kind::Picture, text: "Screenshot 2026-10-27 at 14.32.05 from the desk machine.png", title: "", copied: false, spent: false, mine: false, picture: Some(pic), more: &[], open: 0.0 };
        let cl = [card];
        let view = PopupView { screen: Screen::Main { staged: None, to: &["Laptop"], chosen: 0, clips: &cl, scroll: 0.0, saved: None, draft: "", more: None }, devices: 1, said: None, host: Host::WINDOWS };
        let scene = popup(&eng, &fonts, &view, None, W, H).unwrap();
        let line = scene.texts.iter().find(|t| t.layout.text().ends_with(" \u{00B7} 1.2 MB")).expect("its name and its whole size");
        let s = line.layout.text();
        assert!(s.starts_with("Screens") && s.contains('\u{2026}') && s.contains(".png \u{00B7}"), "cut in the middle: {}", s.len());
        let more = rect_of(&scene, Hit::More(0));
        if let Some(more) = more {
            assert!(line.x + line.layout.measure().width <= more.left + 0.5, "clear of its menu");
        }
    }

    /// Pairing's code shows with its QR beside it (the link to pair, the
    /// code in its fragment): in the popup, and nowhere once the code is
    /// gone (expired or used: pairing is idle again). A short popup shows
    /// the code alone, never past its foot.
    #[test]
    fn a_code_shows_with_its_qr_and_the_qr_goes_with_it() {
        let (eng, fonts) = eng_fonts();
        let devices = [DeviceRow { name: "Desk PC", look: Look::Laptop, this: true }, DeviceRow { name: "iPhone", look: Look::Phone, this: false }];
        let at = |pairing: Pairing, h: f32| {
            let view = PopupView { screen: Screen::Devices { devices: &devices, pairing, facts: &[], naming: None }, devices: 1, said: None, host: Host::WINDOWS };
            popup(&eng, &fonts, &view, None, W, h).unwrap()
        };
        let qr = |s: &Scene| s.fills.iter().filter(|f| f.ink == Ink::QrModule).count();
        let showing = at(Pairing::Showing { code: "F4FS-2RSA", left: "4:12 left" }, H);
        assert!(qr(&showing) > 100, "the QR, beside the code");
        let ground = showing.fills.iter().find(|f| f.ink == Ink::QrGround).unwrap().rect;
        let code = showing.texts.iter().find(|t| t.layout.text() == "F4FS-2RSA").expect("the code to type");
        assert!(code.x >= ground.right, "beside it");
        assert!(showing.texts.iter().any(|t| t.layout.text() == "Desk PC"), "and the list");
        for gone in [Pairing::Idle, Pairing::Waiting, Pairing::Asks { name: "Laptop" }] {
            assert_eq!(qr(&at(gone, H)), 0, "no code, no QR");
        }
        let short = at(Pairing::Showing { code: "F4FS-2RSA", left: "4:12 left" }, 330.0);
        assert!(qr(&short) > 100);
        assert!(short.fills.iter().all(|f| f.rect.bottom <= 330.0), "inside the short popup");
        assert!(!short.texts.iter().any(|t| t.layout.text() == "Desk PC"), "the code alone there");
    }

    /// Settings > Devices marks a device as the side menu marks its rows:
    /// solid, at their size, centred on the name's line.
    #[test]
    fn a_devices_mark_is_solid_and_the_side_menus_size() {
        let (eng, fonts) = eng_fonts();
        let devices = [DeviceRow { name: "Desk PC", look: Look::Laptop, this: true }, DeviceRow { name: "iPhone", look: Look::Phone, this: false }];
        let view = PopupView { screen: Screen::Devices { devices: &devices, pairing: Pairing::Idle, facts: &[], naming: None }, devices: 1, said: None, host: Host::WINDOWS };
        let scene = popup(&eng, &fonts, &view, None, W, H).unwrap();
        // Solid marks (never an outline glyph), the side menu's size.
        assert!(!scene.texts.iter().any(|t| t.layout.text() == "\u{E7F8}" || t.layout.text() == "\u{E8EA}"), "no outline glyphs");
        for (logo, name) in [(crate::vector::Logo::Laptop, "Desk PC"), (crate::vector::Logo::Phone, "iPhone")] {
            let mark = scene.vectors.iter().find(|v| v.logo == logo).unwrap_or_else(|| panic!("{name}'s mark"));
            assert_eq!((mark.rect.width(), mark.rect.height()), (DEVICE_MARK, DEVICE_MARK));
            // Its middle (its ink's, as drawn) at the middle of the name's capitals.
            let label = scene.texts.iter().find(|t| t.layout.text() == name).unwrap();
            let li = ink(&eng, label, 1.0);
            let (mid, gm) = ((li.top + li.bottom) / 2.0, (mark.rect.top + mark.rect.bottom) / 2.0);
            assert!((gm - mid).abs() <= 1.5, "{name}: on the name's line ({gm} vs {mid})");
        }
        let _ = &fonts;
    }

    /// Each kind of device has its own mark, in one family; every button
    /// in Devices reads as one: the button's fill, a step off the panel,
    /// its words in full ink, and the one asked for the strongest (the text
    /// ink itself); never a raised white pill, never a shadow, in whatever
    /// state pairing is. This device's row offers a quiet Rename, not
    /// Remove, and while its name is typed, the field, Save and Cancel.
    #[test]
    fn devices_buttons_have_settings_fill_and_this_one_renames() {
        use crate::vector::Logo;
        let (eng, fonts) = eng_fonts();
        let devices = [
            DeviceRow { name: "Windows laptop", look: Look::Laptop, this: true },
            DeviceRow { name: "iPhone", look: Look::Phone, this: false },
            DeviceRow { name: "iPad", look: Look::Tablet, this: false },
            DeviceRow { name: "Studio", look: Look::Desktop, this: false },
        ];
        let screen = |pairing, naming| PopupView { screen: Screen::Devices { devices: &devices, pairing, facts: &[], naming }, devices: 3, said: None, host: Host::WINDOWS };
        let scene = popup(&eng, &fonts, &screen(Pairing::Idle, None), None, W, H).unwrap();
        for logo in [Logo::Laptop, Logo::Phone, Logo::Tablet, Logo::Desktop] {
            assert_eq!(scene.vectors.iter().filter(|v| v.logo == logo).count(), 1, "{logo:?}");
        }
        assert!(rect_of(&scene, Hit::Rename).is_some() && rect_of(&scene, Hit::Remove(0)).is_none(), "this one renames");
        let rename = rect_of(&scene, Hit::Rename).unwrap();
        assert!(!scene.fills.iter().any(|f| f.rect == rename), "quiet: no fill at rest");
        let names: Vec<&str> = scene.texts.iter().map(|t| t.layout.text()).collect();
        assert!(names.contains(&"This one") && names.contains(&"iPad") && names.contains(&"Studio"), "{names:?}");
        for (pairing, naming) in [
            (Pairing::Idle, None),
            (Pairing::Idle, Some("Kitchen laptop")),
            (Pairing::Asks { name: "iPad" }, None),
            (Pairing::Entering { typed: "F4FS" }, None),
            (Pairing::Showing { code: "F4FS-2RSA", left: "4:12 left" }, None),
            (Pairing::Waiting, None),
        ] {
            let scene = popup(&eng, &fonts, &screen(pairing.clone(), naming), None, W, H).unwrap();
            let buttons: Vec<Rect> = scene.hits.iter().filter(|(_, t)| matches!(t, Target::Quick(Hit::Remove(_) | Hit::ShowCode | Hit::EnterCode | Hit::Pair | Hit::Cancel | Hit::Allow | Hit::Deny | Hit::NameSave | Hit::NameCancel))).map(|(r, _)| *r).collect();
            assert!(!buttons.is_empty());
            for b in buttons {
                let under: Vec<&Fill> = scene.fills.iter().filter(|f| f.rect == b).collect();
                assert!(!under.is_empty() && under.iter().all(|f| f.ink == Ink::Button || f.ink == Ink::Text), "{pairing:?}: a button's fill, or the text ink for what is asked for");
                let words: Vec<Ink> = scene.texts.iter().filter(|t| t.x >= b.left && t.x < b.right && t.y >= b.top - 1.0 && t.y < b.bottom).map(|t| t.ink).collect();
                assert!(!words.is_empty() && words.iter().all(|i| *i == Ink::Text || *i == Ink::Page), "{pairing:?}: a button's words in full ink ({words:?})");
                assert!(!scene.fills.iter().any(|f| f.rect == b.offset(0.0, 1.0)), "{pairing:?}: no shadow");
            }
            assert!(!scene.fills.iter().any(|f| f.ink == Ink::Raised), "{pairing:?}: nothing raised white");
            if naming.is_some() {
                assert!(rect_of(&scene, Hit::NameField).is_some() && rect_of(&scene, Hit::Rename).is_none());
                assert!(scene.texts.iter().any(|t| t.layout.text() == "Kitchen laptop"));
            }
        }
    }

    /// A beam's mark through its life: pending until the server has it; its
    /// check pops in on a spring (a little past whole, then whole), holds,
    /// fades and is gone; with less motion it only fades. A beam sent with
    /// no popup shows its check by the button instead. A failure stays red.
    #[test]
    fn a_beams_mark_waits_pops_holds_fades_and_goes() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        let mut m = Marks::default();
        m.sent([1; 16], Place::Card);
        assert_eq!(m.card([1; 16], at(5.0), true), Sent::Pending);
        assert!(!m.moving(at(5.0)), "waiting moves nothing");
        assert!(m.acked([1; 16], t0));
        let scale = |s: f32| match m.card([1; 16], at(s), true) {
            Sent::Check { scale, alpha } => Some((scale, alpha)),
            _ => None,
        };
        assert!(scale(0.02).is_some_and(|(s, _)| s < 0.5), "it grows");
        assert!((1..30).map(|k| scale(k as f32 / 100.0).unwrap().0).fold(0.0f32, f32::max) > 1.0, "a little past whole");
        assert!(scale(0.8).is_some_and(|(s, a)| (s - 1.0).abs() < 0.02 && a == 1.0), "held, whole");
        assert!(scale(CHECK_IN + CHECK_HOLD + 0.1).is_some_and(|(_, a)| a < 1.0), "fading");
        assert_eq!(scale(CHECK_IN + CHECK_HOLD + CHECK_OUT + 0.01), None, "gone");
        assert!(m.moving(at(0.5)) && !m.moving(at(2.0)));
        // Less motion: whole from the start, only its alpha moves.
        match m.card([1; 16], at(0.05), false) {
            Sent::Check { scale, alpha } => assert!(scale == 1.0 && alpha < 1.0),
            s => panic!("{s:?}"),
        }
        m.prune(at(2.0));
        assert_eq!(m.card([1; 16], at(0.5), true), Sent::None, "let go once gone");
        // No popup: the check by the button, not the card.
        m.sent([2; 16], Place::Button);
        m.acked([2; 16], t0);
        assert!(m.button(at(0.5), true).is_some());
        assert_eq!(m.card([2; 16], at(0.5), true), Sent::None);
        // A failure stays until retried or removed.
        m.sent([3; 16], Place::Card);
        m.failed([3; 16]);
        m.prune(at(60.0));
        assert_eq!(m.card([3; 16], at(60.0), true), Sent::Failed);
    }

    /// Every state, both themes' inks: everything inside the popup, nothing
    /// overlapping that should not, and the paper plane and Send nowhere.
    #[test]
    fn every_state_holds_together_and_never_says_send() {
        let (eng, fonts) = eng_fonts();
        let to = ["All devices", "iPhone", "Laptop"];
        let devices = [DeviceRow { name: "Desk PC", look: Look::Laptop, this: true }, DeviceRow { name: "iPhone", look: Look::Phone, this: false }];
        let cl = clips();
        let staged = Staged { what: "a link", kind: Kind::Link, text: "https://developers.cloudflare.com/durable-objects/platform/pricing/", picture: None, more: &[], unsure: false };
        // Long text of several paragraphs, right to left: shown in a few
        // lines, never over who to and Beam.
        let long = format!("{}\n\n{}", "\u{0646}\u{0635} \u{062A}\u{062C}\u{0631}\u{064A}\u{0628}\u{064A} ".repeat(30), "\u{0633}\u{0637}\u{0631} ".repeat(40));
        let long_staged = Staged { what: "text", kind: Kind::Text, text: &long, picture: None, more: &[], unsure: false };
        let mut long_cl = cl.clone();
        long_cl.insert(0, ClipView { text: &long, ..cl[0].clone() });
        // Typed, long enough to run past the field's lines; a picture ready;
        // picture cards, one still coming.
        let typed = "Gate B, row 14\nBoarding at 19:05\nSeat 22A\nBags: one\nTaxi at 17:30, from the hotel's front door, not the side one.";
        let pic = PicView { pixels: None, width: 1920, height: 1080, size: "1.2 MB", ready: true };
        let staged_picture = Staged { what: "a picture", kind: Kind::Picture, text: "Picture.png", picture: Some(pic.clone()), more: &[], unsure: false };
        let maybe = Staged { what: "a password", kind: Kind::Secret, text: "x7kp2qm9vld8wz", picture: None, more: &[], unsure: true };
        let mut picture_cl = cl.clone();
        picture_cl.insert(0, ClipView { kind: Kind::Picture, text: "Screenshot.png", picture: Some(pic.clone()), ..cl[0].clone() });
        picture_cl.insert(1, ClipView { kind: Kind::Picture, text: "Whiteboard.jpg", picture: Some(PicView { ready: false, height: 3000, ..pic.clone() }), ..cl[0].clone() });
        let screens = [
            Screen::FirstUse,
            Screen::Main { staged: None, to: &to, chosen: 0, clips: &[], scroll: 0.0, saved: None, draft: "", more: None },
            Screen::Main { staged: Some(staged.clone()), to: &to, chosen: 0, clips: &cl, scroll: 0.0, saved: None, draft: "", more: None },
            Screen::Main { staged: Some(long_staged.clone()), to: &to, chosen: 0, clips: &long_cl, scroll: 0.0, saved: None, draft: "", more: None },
            Screen::Main { staged: None, to: &to, chosen: 0, clips: &cl, scroll: 0.0, saved: None, draft: typed, more: None },
            Screen::Main { staged: Some(staged_picture.clone()), to: &to, chosen: 0, clips: &cl, scroll: 0.0, saved: None, draft: "", more: None },
            Screen::Main { staged: Some(maybe.clone()), to: &to, chosen: 0, clips: &cl, scroll: 0.0, saved: None, draft: "", more: None },
            Screen::Main { staged: None, to: &to, chosen: 0, clips: &picture_cl, scroll: 0.0, saved: None, draft: "", more: None },
            Screen::Main { staged: None, to: &to, chosen: 0, clips: &long_cl, scroll: 0.0, saved: None, draft: "", more: None },
            Screen::Main { staged: None, to: &to, chosen: 0, clips: &cl, scroll: 120.0, saved: Some("Gate B, row 14"), draft: "", more: None },
            Screen::Devices { devices: &devices, pairing: Pairing::Idle, facts: &[], naming: None },
            Screen::Devices { devices: &devices, pairing: Pairing::Showing { code: "F4FS-2RSA", left: "4:12 left" }, facts: &[], naming: None },
            Screen::Devices { devices: &devices, pairing: Pairing::Asks { name: "Laptop" }, facts: &[], naming: None },
            Screen::Devices { devices: &devices, pairing: Pairing::Entering { typed: "F4FS-2R" }, facts: &[], naming: None },
            Screen::Beamed { to: "iPhone and Laptop", what: "Durable Objects pricing" },
        ];
        // Every height a screen may leave it: a laptop at 200% leaves about
        // 330 DIPs above the taskbar.
        // On each platform's words: the Mac's never say Win, Ctrl or PC.
        for host in [Host::WINDOWS, Host::MAC] {
        for (k, screen) in screens.iter().enumerate() {
          for full in [H, 480.0, 400.0, 330.0] {
            let h = if matches!(screen, Screen::Beamed { .. }) { BEAMED_H } else { full };
            let view = PopupView { screen: screen.clone(), devices: 2, said: Some("That code didn't match"), host };
            let scene = popup(&eng, &fonts, &view, None, W, h).unwrap();
            let k = format!("{k} at {h}");
            for (r, t) in &scene.hits {
                assert!(r.left >= -0.5 && r.top >= -0.5 && r.right <= W + 0.5 && r.bottom <= h + 0.5, "state {k}: {t:?} leaves the popup ({r:?})");
            }
            // A text in a clipped layer (the clips' list) may run past it:
            // it is cut there. Every other text is inside the popup.
            let cut = |i: usize| {
                let mut clip = None;
                for o in &scene.overlays {
                    if i >= o.1 {
                        clip = o.2;
                    }
                }
                clip
            };
            for (i, t) in scene.texts.iter().enumerate() {
                match cut(i) {
                    Some(c) => assert!(c.top >= -0.5 && c.bottom <= h + 0.5, "state {k}: a clip outside the popup"),
                    None => assert!(t.y >= -0.5 && t.y <= h, "state {k}: a text outside the popup"),
                }
            }
            let controls: Vec<Rect> = scene.hits.iter().filter(|(_, t)| !matches!(t, Target::Quick(Hit::Popup | Hit::List))).map(|(r, _)| *r).collect();
            for (a, ra) in controls.iter().enumerate() {
                for rb in &controls[a + 1..] {
                    let overlap = ra.left < rb.right - 0.5 && rb.left < ra.right - 0.5 && ra.top < rb.bottom - 0.5 && rb.top < ra.bottom - 0.5;
                    let nested = (ra.left >= rb.left && ra.right <= rb.right && ra.top >= rb.top && ra.bottom <= rb.bottom) || (rb.left >= ra.left && rb.right <= ra.right && rb.top >= ra.top && rb.bottom <= ra.bottom);
                    assert!(!overlap || nested, "state {k}: controls overlap {ra:?} {rb:?}");
                }
            }
            for t in &scene.texts {
                let s = t.layout.text();
                assert!(!s.contains('\u{E724}'), "state {k}: the paper plane is the note's");
                assert!(s != "Send" && !s.starts_with("Send "), "state {k}: Send is the note's: {s}");
                assert!(!s.contains("Share"), "state {k}: no second Share");
            }
            // A text that is not on a control never runs into one (a text
            // the list cuts is checked by the scroll test).
            // An icon is placed by its ink, not its box (`Painter::centred`),
            // so its box may lean past its own control; its ink is checked
            // by the_fields_buttons_are_concentric_with_its_end.
            let icon = |s: &str| !s.is_empty() && s.chars().all(|c| ('\u{E000}'..='\u{F8FF}').contains(&c));
            for (_, t) in scene.texts.iter().enumerate().filter(|(i, t)| cut(*i).is_none() && !icon(t.layout.text())) {
                let m = t.layout.measure();
                let tr = Rect::new(t.x, t.y, t.x + m.width, t.y + m.height);
                for c in &controls {
                    let inside = tr.left >= c.left - 0.5 && tr.right <= c.right + 0.5 && tr.top >= c.top - 0.5 && tr.bottom <= c.bottom + 0.5;
                    let touches = tr.left < c.right - 0.5 && c.left < tr.right - 0.5 && tr.top < c.bottom - 0.5 && c.top < tr.bottom - 0.5;
                    assert!(!touches || inside, "state {k}: {:?} runs into a control {c:?}", t.layout.text());
                }
                let s = t.layout.text();
                if host == Host::MAC {
                    assert!(!s.contains("Win+") && !s.contains("Ctrl+") && !s.contains("this PC"), "state {k}: {s} on the Mac");
                }
            }
          }
        }
        }
    }

    /// Something that may be a password shows masked, with "Not a password"
    /// to click under its dots, clear of its x; a sure one has none.
    #[test]
    fn maybe_a_password_shows_masked_with_not_a_password() {
        let (eng, fonts) = eng_fonts();
        let at = |unsure: bool| {
            let staged = Staged { what: "a password", kind: Kind::Secret, text: "x7kp2qm9vld8wz", picture: None, more: &[], unsure };
            let view = PopupView { screen: Screen::Main { staged: Some(staged), to: &["All devices", "iPhone"], chosen: 0, clips: &[], scroll: 0.0, saved: None, draft: "", more: None }, devices: 1, said: None, host: Host::WINDOWS };
            popup(&eng, &fonts, &view, None, W, H).unwrap()
        };
        let scene = at(true);
        let not = rect_of(&scene, Hit::NotSecret).expect("Not a password, to click");
        let unstage = rect_of(&scene, Hit::Unstage).unwrap();
        let field = rect_of(&scene, Hit::Field).unwrap();
        assert!(not.right <= unstage.left && not.bottom <= field.top + 1.0, "under the dots, clear of the x, above the field's text");
        assert!(not.width() >= 40.0 && not.height() >= 16.0, "big enough to click");
        assert!(scene.texts.iter().any(|t| t.layout.text() == NOT_SECRET));
        assert!(scene.texts.iter().all(|t| !t.layout.text().contains("x7kp")), "still masked");
        assert!(rect_of(&at(false), Hit::NotSecret).is_none(), "a sure password has no way out");
    }

    /// What was copied sits in the field's top, ready, with its x; the beam
    /// in the field turns teal. Who to is a small chip in the field's row,
    /// only with two devices or more to choose from: a tap moves it on.
    #[test]
    fn what_was_copied_shows_ready_in_the_field_and_who_to_is_a_chip() {
        let (eng, fonts) = eng_fonts();
        let staged = Staged { what: "a password", kind: Kind::Secret, text: "hunter2!Xq9", picture: None, more: &[], unsure: false };
        let at = |to: &[&str], chosen: usize| {
            let view = PopupView { screen: Screen::Main { staged: Some(staged.clone()), to, chosen, clips: &[], scroll: 0.0, saved: None, draft: "", more: None }, devices: to.len() - 1, said: None, host: Host::WINDOWS };
            popup(&eng, &fonts, &view, None, W, H).unwrap()
        };
        let scene = at(&["All devices", "iPhone", "Laptop"], 0);
        let field = rect_of(&scene, Hit::Field).unwrap();
        let beam = rect_of(&scene, Hit::Beam).expect("the beam, with something to beam");
        let chip = rect_of(&scene, Hit::To(1)).expect("who to: a tap moves to the next");
        let unstage = rect_of(&scene, Hit::Unstage).unwrap();
        assert!(beam.left >= field.right - 1.0 - 80.0 && beam.bottom <= H - PAD, "in the field's row, at its end");
        assert!(chip.right <= beam.left && (chip.top + chip.bottom) / 2.0 > field.bottom - FIELD_MIN, "on the last line");
        assert!(unstage.bottom <= field.bottom - FIELD_MIN + 8.0, "in the field's top");
        assert!(scene.vectors.iter().any(|v| v.logo == crate::vector::Logo::Beam && v.ink == Ink::Accent && v.rect.left >= beam.left - 1.0), "teal with something to beam");
        assert!(rect_of(&at(&["All devices", "iPhone", "Laptop"], 2), Hit::To(0)).is_some(), "after the last, all again");
        assert!(scene.texts.iter().all(|t| !t.layout.text().contains("hunter2")), "a password never shows");
        // One other device: nothing to choose, no chip.
        let one = at(&["All devices", "iPhone"], 0);
        assert!(!one.hits.iter().any(|(_, t)| matches!(t, Target::Quick(Hit::To(_)))));
        // Nothing to beam: the beam is faint and takes no click.
        let view = PopupView { screen: Screen::Main { staged: None, to: &["All devices", "iPhone"], chosen: 0, clips: &[], scroll: 0.0, saved: None, draft: "", more: None }, devices: 1, said: None, host: Host::WINDOWS };
        let empty = popup(&eng, &fonts, &view, None, W, H).unwrap();
        assert!(rect_of(&empty, Hit::Beam).is_none());
        assert!(empty.vectors.iter().any(|v| v.logo == crate::vector::Logo::Beam && v.ink == Ink::Faint));
    }

    /// The pointer is the I-beam over the whole send field, as over the
    /// note and the search (34ce90d), and an arrow over its paperclip; the
    /// code field takes typing too.
    #[test]
    fn the_send_field_shows_the_i_beam_but_not_over_its_paperclip() {
        let (eng, fonts) = eng_fonts();
        let to = ["All devices", "Laptop"];
        for draft in ["", "Gate B, row 14"] {
            let view = PopupView { screen: Screen::Main { staged: None, to: &to, chosen: 0, clips: &[], scroll: 0.0, saved: None, draft, more: None }, devices: 1, said: None, host: Host::WINDOWS };
            let scene = popup(&eng, &fonts, &view, None, W, H).unwrap();
            let field = rect_of(&scene, Hit::Field).expect("the field");
            let attach = rect_of(&scene, Hit::Attach).expect("the paperclip");
            let over = |x: f32, y: f32| match scene::hit(&scene.hits, x, y) {
                Some(Target::Quick(h)) => takes_typing(h),
                _ => false,
            };
            for k in 0..=10 {
                let x = field.left + 1.0 + (field.width() - 2.0) * k as f32 / 10.0;
                assert!(over(x, (field.top + field.bottom) / 2.0), "the I-beam at {x} over {field:?} ({draft:?})");
            }
            assert!(!over((attach.left + attach.right) / 2.0, (attach.top + attach.bottom) / 2.0), "an arrow over the paperclip");
            assert!(attach.left >= field.right - 0.5, "the paperclip beside the field, not in it");
        }
        assert!(takes_typing(Hit::CodeField) && !takes_typing(Hit::Beam) && !takes_typing(Hit::Attach));
    }

    /// The send field's caret, at 100, 150 and 300%: with nothing typed, just
    /// before the placeholder's first letter (which starts after it); typed,
    /// just after the last letter. Centred on the capitals of its line and
    /// as tall for the text as the note's caret (its line, 1.5 times the
    /// size), never on top of a letter.
    #[test]
    fn the_fields_caret_sits_before_the_placeholder_centred_on_its_line() {
        let (eng, fonts) = eng_fonts();
        assert_eq!(fonts.body.line.unwrap() / fonts.body.size, CARET_TALL, "the note's caret is its line, 1.5 times its text size");
        let size = fonts.peek.size;
        let to = ["All devices", "Laptop"];
        for draft in ["", "Gate B", "Gate B, row 14
Boarding at 19:05"] {
            let view = PopupView { screen: Screen::Main { staged: None, to: &to, chosen: 0, clips: &[], scroll: 0.0, saved: None, draft, more: None }, devices: 1, said: None, host: Host::WINDOWS };
            let scene = popup(&eng, &fonts, &view, None, W, H).unwrap();
            let field = rect_of(&scene, Hit::Field).unwrap();
            let carets: Vec<Rect> = scene.fills.iter().filter(|f| f.ink == Ink::Accent && f.rect.width() <= CARET_WIDTH + 0.01 && f.rect.top >= field.top && f.rect.bottom <= field.bottom).map(|f| f.rect).collect();
            assert_eq!(carets.len(), 1, "{draft:?}: one caret");
            let caret = carets[0];
            let shown = if draft.is_empty() { PLACEHOLDER } else { draft };
            let text = scene.texts.iter().find(|t| t.layout.text() == shown).expect("the field's text");
            assert!((caret.height() - CARET_TALL * size).abs() < 0.01 && (caret.width() - CARET_WIDTH).abs() < 0.01, "{caret:?}");
            for scale in [1.0, 1.5, 3.0] {
                let glyphs = text.layout.glyphs((text.x * scale, text.y * scale), scale);
                let (first, last) = (glyphs.first().unwrap(), glyphs.last().unwrap());
                let at = |g: &kolada_text::Glyph| (g.x as f32 / scale, g.y as f32 / scale);
                let (gx, base) = if draft.is_empty() { at(first) } else { at(last) };
                if draft.is_empty() {
                    assert!(caret.right <= gx + 0.5 && gx - caret.right <= CARET_GAP + 1.0, "{scale}: just before the first letter ({caret:?}, {gx})");
                } else {
                    assert!(caret.left >= gx, "{scale}: after the last letter ({caret:?}, {gx})");
                }
                let mid = (caret.top + caret.bottom) / 2.0;
                assert!((mid - (base - CARET_MID * size)).abs() <= 1.0 / scale + 0.01, "{scale} {draft:?}: centred on its line ({mid}, baseline {base})");
                assert!(caret.top < base - 0.7 * size && caret.bottom > base, "{scale}: over the capitals and down past the baseline");
            }
        }
    }

    /// The beams read as a chat's: the newest at the foot, just above the
    /// field, older ones going up; what this device beamed on the right,
    /// what came in on the left. Scrolled up, the oldest one's top is as
    /// far as it goes; a card half scrolled out is cut at the list's edge,
    /// never drawn or clicked over the field (seen: it was, on 2026-10-27).
    #[test]
    fn the_beams_read_from_the_foot_up_and_scroll_as_far_as_the_oldest() {
        let (eng, fonts) = eng_fonts();
        let to = ["All devices", "Laptop"];
        let mut many = clips();
        many[2].mine = true;
        many.extend(clips());
        many.extend(clips());
        let at = |scroll: f32| {
            let view = PopupView { screen: Screen::Main { staged: None, to: &to, chosen: 0, clips: &many, scroll, saved: None, draft: "", more: None }, devices: 1, said: None, host: Host::WINDOWS };
            popup(&eng, &fonts, &view, None, W, H).unwrap()
        };
        let foot = at(0.0);
        let most = foot.quick_scroll_max;
        assert!(most > 200.0, "fifteen clips are taller than the list ({most})");
        let list = rect_of(&foot, Hit::List).unwrap();
        let field = rect_of(&foot, Hit::Field).unwrap();
        assert!(field.top >= list.bottom, "the field is below the list, at the foot");
        assert!((field.bottom - (H - PAD)).abs() < 0.01);
        // At the foot, the newest card ends at the list's foot (the card
        // itself: how tall it is depends on the fonts its words wrap in).
        let newest = rect_of(&foot, Hit::Card(0)).expect("the newest is shown");
        assert!(newest.bottom <= list.bottom + 0.5 && newest.bottom >= list.bottom - 16.0, "{newest:?} {list:?}");
        // Sent on the right, received on the left.
        let card_of = |s: &Scene, i: usize| rect_of(s, Hit::Card(i)).map(|r| r.right).unwrap();
        assert!(card_of(&foot, 2) > card_of(&foot, 1), "mine sits right");
        // Scrolled past the oldest: the same as at the oldest; below the
        // foot: the same as at the foot.
        assert_eq!(at(most).hits, at(most + 5_000.0).hits, "no scrolling beyond the oldest");
        assert_eq!(at(-300.0).hits, foot.hits, "nor below the newest");
        let oldest = at(most);
        let first = oldest.hits.iter().filter_map(|(r, t)| matches!(t, Target::Quick(Hit::Card(_))).then_some(r.top)).fold(f32::MAX, f32::min);
        assert!(first >= list.top - 0.5 && first <= list.top + 40.0, "the oldest card starts at the list's top ({first}, {list:?})");
        for scroll in [37.0, most / 2.0, most] {
            let s = at(scroll);
            assert!(s.overlays.iter().any(|o| o.2 == Some(list)), "the list is clipped to itself");
            for (r, t) in &s.hits {
                if let Target::Quick(Hit::Copy(_) | Hit::Card(_) | Hit::More(_)) = t {
                    assert!(r.top >= list.top - 0.5 && r.bottom <= list.bottom + 0.5, "{t:?} at {r:?} leaves the list at scroll {scroll}");
                }
            }
            assert_eq!(rect_of(&s, Hit::Field), Some(field), "the field stays put");
        }
    }

    /// Every card has Copy, a glyph at its top right (not a password once
    /// copied); under the pointer, "…" beside it opens a Kolada menu: Open
    /// link, Save picture…, Save as note (never for a password), Remove.
    #[test]
    fn a_card_has_copy_and_its_menu_has_the_rest() {
        let (eng, fonts) = eng_fonts();
        let cl = clips();
        let at = |hover: Option<Target>, more: Option<usize>| {
            let view = PopupView { screen: Screen::Main { staged: None, to: &["All devices"], chosen: 0, clips: &cl, scroll: 0.0, saved: None, draft: "", more }, devices: 1, said: None, host: Host::WINDOWS };
            popup(&eng, &fonts, &view, hover, W, H).unwrap()
        };
        let rest = at(None, None);
        for i in 0..3 {
            let copy = rect_of(&rest, Hit::Copy(i)).expect("Copy on every card");
            let card = rect_of(&rest, Hit::Card(i)).unwrap();
            assert!(copy.right <= card.right && copy.top >= card.top && copy.top <= card.top + 8.0, "at its top right");
            assert!(rect_of(&rest, Hit::More(i)).is_none(), "quiet at rest");
        }
        assert!(!rest.texts.iter().any(|t| t.layout.text().contains("Save as note")), "no rows of words on the cards");
        // Under the pointer, "…"; open, its menu.
        let hovered = at(Some(Target::Quick(Hit::Card(2))), None);
        assert!(rect_of(&hovered, Hit::More(2)).is_some());
        let link = at(None, Some(2));
        let words: Vec<&str> = link.texts.iter().map(|t| t.layout.text()).collect();
        assert!(words.contains(&"Open link") && words.contains(&"Save as note") && words.contains(&"Remove"), "{words:?}");
        assert!(rect_of(&link, Hit::Open(2)).is_some() && rect_of(&link, Hit::Forget(2)).is_some());
        let password = at(None, Some(1));
        assert!(rect_of(&password, Hit::SaveNote(1)).is_none() && rect_of(&password, Hit::Forget(1)).is_some(), "a password is copied, never saved");
        assert!(!rest.texts.iter().any(|t| t.layout.text().contains("hunter2")));
        // Kept until removed: no card says when it goes.
        assert!(!rest.texts.iter().any(|t| t.layout.text().contains("Goes in")));
    }

    /// Surfaces a step apart, lifted by fill and shadow, never a rim: the
    /// well lowest, the bars' bands over it, every card raised, whoever
    /// sent it; no tinted slab anywhere. Teal is the caret's, and the
    /// beam's with something to beam: nothing else.
    #[test]
    fn surfaces_step_apart_and_teal_only_where_it_means_something() {
        let (eng, fonts) = eng_fonts();
        let mut cl = clips();
        cl[2].mine = true;
        let staged = Staged { what: "a link", kind: Kind::Link, text: "https://kolada.app", picture: None, more: &[], unsure: false };
        let screens = [
            Screen::Main { staged: None, to: &["All devices", "iPhone", "Laptop"], chosen: 0, clips: &cl, scroll: 0.0, saved: Some("Plan"), draft: "Gate B", more: Some(0) },
            Screen::Main { staged: Some(staged), to: &["All devices"], chosen: 0, clips: &cl, scroll: 0.0, saved: None, draft: "", more: None },
            Screen::FirstUse,
            Screen::Beamed { to: "iPhone", what: "Plan" },
        ];
        for screen in screens {
            let view = PopupView { screen, devices: 2, said: None, host: Host::WINDOWS };
            let scene = popup(&eng, &fonts, &view, None, W, H).unwrap();
            for f in &scene.fills {
                assert!(f.ink != Ink::Mark, "no tinted slab: {f:?}");
                if f.ink == Ink::Accent {
                    assert!(f.rect.width() <= CARET_WIDTH + 0.01, "teal fills are the caret's: {f:?}");
                }
            }
            assert!(scene.texts.iter().all(|t| t.ink != Ink::Danger), "red is only for errors");
            for v in scene.vectors.iter().filter(|v| v.ink == Ink::Accent) {
                assert_eq!(v.logo, crate::vector::Logo::Beam, "teal marks are the beam's");
            }
            assert_eq!(scene.fills[0].ink, Ink::Well, "the popup's lowest surface is the well");
            assert!(scene.fills.iter().all(|f| f.ink != Ink::Border), "lifted by fill and shadow, never a rim");
        }
        // Cards: one raised fill, mine and theirs alike, a shadow under each.
        let view = PopupView { screen: Screen::Main { staged: None, to: &["All devices"], chosen: 0, clips: &cl, scroll: 0.0, saved: None, draft: "", more: None }, devices: 1, said: None, host: Host::WINDOWS };
        let scene = popup(&eng, &fonts, &view, None, W, H).unwrap();
        for i in 0..3 {
            let card = rect_of(&scene, Hit::Card(i)).unwrap();
            let fill = scene.fills.iter().find(|f| f.rect == card).expect("its fill");
            assert_eq!(fill.ink, Ink::Raised);
            assert!(scene.fills.iter().any(|f| f.ink == Ink::Shadow && f.rect.left == card.left && f.rect.top > card.top), "lifted");
        }
    }

    /// A text card shows two lines at rest. Opened (its spring at 1) it
    /// shows up to `OPEN_LINES`, growing upward so the field and the newest
    /// stay put; past the limit "Read more" opens it as a note. Halfway, it
    /// is halfway. A card that fits in two lines opens to nothing more.
    #[test]
    fn a_long_card_opens_to_its_limit_then_reads_more_as_a_note() {
        let (eng, fonts) = eng_fonts();
        let long: String = (1..=30).map(|n| format!("Line {n} of the agenda, with enough words to fill it.")).collect::<Vec<_>>().join("\n");
        let mid: String = (1..=6).map(|n| format!("Step {n}")).collect::<Vec<_>>().join("\n");
        let base = clips()[0].clone();
        let at = |open: [f32; 3]| {
            let cl = [ClipView { text: "Short one", open: open[0], ..base.clone() }, ClipView { text: &mid, open: open[1], ..base.clone() }, ClipView { text: &long, open: open[2], ..base.clone() }];
            let view = PopupView { screen: Screen::Main { staged: None, to: &["All devices"], chosen: 0, clips: &cl, scroll: 0.0, saved: None, draft: "", more: None }, devices: 1, said: None, host: Host::WINDOWS };
            popup(&eng, &fonts, &view, None, W, H).unwrap()
        };
        let line = fonts.peek.line.unwrap();
        let shut = at([0.0; 3]);
        let card = |s: &Scene, i: usize| rect_of(s, Hit::Card(i)).unwrap();
        let (a0, b0) = (card(&shut, 0), card(&shut, 1));
        assert_eq!(a0.height(), card(&at([1.0, 0.0, 0.0]), 0).height(), "two lines or fewer: nothing to open");
        assert!(rect_of(&shut, Hit::ReadMore(2)).is_none());
        // The six-line one opens to six, growing upward: the newest card and
        // the field stay where they were.
        let open = at([0.0, 1.0, 0.0]);
        assert!((card(&open, 1).height() - b0.height() - 4.0 * line).abs() < 1.0, "{:?} {:?}", card(&open, 1), b0);
        assert_eq!(card(&open, 0), a0, "the newest stays put");
        assert_eq!(rect_of(&open, Hit::Field), rect_of(&shut, Hit::Field));
        assert_eq!(card(&open, 1).bottom, b0.bottom, "it grows upward");
        let half = at([0.0, 0.5, 0.0]);
        assert!(card(&half, 1).height() > b0.height() && card(&half, 1).height() < card(&open, 1).height());
        // The thirty-line one opens to the limit, then offers Read more.
        let limit = at([0.0, 0.0, 1.0]);
        assert!(card(&limit, 2).height() <= b0.height() + (OPEN_LINES - REST_LINES) as f32 * line + 2.0 * META);
        assert!(rect_of(&limit, Hit::ReadMore(2)).is_some(), "past the limit");
        // Its lines, not an ellipsis alone (seen: a line's ends restart in
        // each paragraph, and the cut kept nothing).
        let shown: Vec<&str> = limit.texts.iter().map(|t| t.layout.text()).collect();
        assert!(shown.iter().any(|t| t.starts_with("Line 1 of")) && shown.iter().any(|t| t.starts_with("Line 2 of")), "its first lines");
        assert!(shown.iter().any(|t| t.starts_with("Line ") && t.ends_with('\u{2026}')), "the last line shown says there is more");
        assert!(rect_of(&limit, Hit::ReadMore(2)).unwrap().bottom <= card(&limit, 2).bottom);
    }

    /// The spring opens and closes a card naturally: it moves the right way,
    /// settles near the end in about a quarter second, and lands exactly.
    #[test]
    fn a_cards_spring_settles_where_it_is_going() {
        let (mut x, mut v) = (0.0, 0.0);
        let mut t = 0.0;
        while (x, v) != (1.0, 0.0) && t < 2.0 {
            (x, v) = spring(x, v, 1.0, 1.0 / 60.0);
            assert!(x >= 0.0 && x <= 1.1, "{x}");
            t += 1.0 / 60.0;
        }
        assert!(t < 0.6 && x == 1.0, "settled at {t} s");
        let (x, _) = spring(1.0, 0.0, 0.0, 0.1);
        assert!(x < 1.0, "and back");
    }
}
