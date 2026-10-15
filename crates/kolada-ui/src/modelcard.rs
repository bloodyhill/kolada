//! The writing model, offered on first run: a small card floats at the
//! bottom right of the note area. "Writing help", then a tile for each
//! model (its name over its download size, Light chosen to begin with),
//! then a quiet "Not now" and a teal Download. Downloading, the card
//! springs down into a slim strip in the same corner ("Light · 64%", a
//! small × to cancel, a thin teal bar); done, the strip fades and the
//! shell says "Writing help is ready" with a `Whisper`.
//!
//! What the card holds and does is `ModelCard` (plain data, every
//! shell's); the shell downloads on a thread of its own, tells the card
//! how far it has come (`progress`), and springs `ModelCardView::strip`
//! between the card (0) and the strip (1). `draw` puts it on the screen.

use kolada_text::{Align, Format};
use windows_core::Result;

use crate::geom::{Fill, Ink, Rect};
use crate::scene::{self, Eng, Fonts, Scene, Target, Text, VectorMark};
use crate::vector::Logo;

/// A model on offer: its id in the catalogue, the name its tile shows,
/// and how much it downloads. The shell fills these from kolada-assist's.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub id: &'static str,
    pub name: &'static str,
    pub bytes: u64,
}

/// Where the card is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Choosing a model: the tile picked (by its place in `choices`).
    Choose { picked: usize },
    /// The picked model coming down: bytes so far, of all of them.
    Downloading { picked: usize, done: u64, total: u64 },
}

/// The card's state.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCard {
    pub choices: Vec<Choice>,
    pub phase: Phase,
    /// The pick before the last change: the teal highlight glides from it.
    pub from: usize,
}

/// What a click on the card is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    /// The card itself, between its controls: the click stays on it.
    Card,
    /// A model's tile, by its place in `choices`.
    Pick(usize),
    Download,
    NotNow,
    /// The strip's ×: the download stops.
    Cancel,
}

/// What the shell does after a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    None,
    /// Download this model (its place in `choices`).
    Download(usize),
    /// Not now: the card goes, and the session remembers it was answered.
    NotNow,
    /// Stop the download; the card is back to choosing.
    Cancel,
}

impl ModelCard {
    /// The card choosing, with the first model (Light) picked.
    pub fn new(choices: Vec<Choice>) -> Self {
        ModelCard { choices, phase: Phase::Choose { picked: 0 }, from: 0 }
    }

    /// The model picked, in either phase.
    pub fn picked(&self) -> usize {
        match self.phase {
            Phase::Choose { picked } | Phase::Downloading { picked, .. } => picked,
        }
    }

    /// A click on the card.
    pub fn click(&mut self, hit: Hit) -> Command {
        match (self.phase, hit) {
            (Phase::Choose { picked }, Hit::Pick(i)) if i < self.choices.len() => {
                self.from = picked;
                self.phase = Phase::Choose { picked: i };
                Command::None
            }
            (Phase::Choose { picked }, Hit::Download) => match self.choices.get(picked) {
                Some(c) => {
                    self.phase = Phase::Downloading { picked, done: 0, total: c.bytes };
                    Command::Download(picked)
                }
                None => Command::None,
            },
            (Phase::Choose { .. }, Hit::NotNow) => Command::NotNow,
            (Phase::Downloading { picked, .. }, Hit::Cancel) => {
                self.phase = Phase::Choose { picked };
                Command::Cancel
            }
            _ => Command::None,
        }
    }

    /// How far the download has come (a total of 0 keeps the one known).
    pub fn progress(&mut self, done: u64, total: u64) {
        if let Phase::Downloading { picked, total: known, .. } = self.phase {
            let total = if total > 0 { total } else { known };
            self.phase = Phase::Downloading { picked, done, total };
        }
    }

    /// The download's share done, 0 to 100 (0 while choosing).
    pub fn percent(&self) -> u8 {
        match self.phase {
            Phase::Downloading { done, total, .. } if total > 0 => (done.min(total) as u128 * 100 / total as u128) as u8,
            _ => 0,
        }
    }
}

/// A download's size as its tile shows it: decimal megabytes, rounded
/// ("230 MB"); from a thousand of them, gigabytes ("1.2 GB").
pub fn size_label(bytes: u64) -> String {
    let mb = (bytes as f64 / 1e6).round() as u64;
    if mb >= 1000 { format!("{:.1} GB", bytes as f64 / 1e9) } else { format!("{mb} MB") }
}

/// Settings › Writing help's value: Light, Full (a place in the
/// catalogue) or Off. The shells keep it as `Session::assist` (1 off) and
/// the model's id.
pub const OFF: u8 = 2;

/// The value Settings › Writing help shows: `OFF`, or the chosen model's
/// place in the catalogue (`None`, a model it no longer has: the first).
pub fn setting_value(off: bool, model: Option<usize>) -> u8 {
    if off { OFF } else { model.unwrap_or(0).min(OFF as usize - 1) as u8 }
}

/// What a choice in Settings › Writing help sets: off, or which model.
pub fn from_setting(value: u8) -> (bool, Option<usize>) {
    if value >= OFF { (true, None) } else { (false, Some(value as usize)) }
}

/// The card as the view shows it: its state; how far it has faded in
/// (out, once done); how far it has sprung from the card (0) into the
/// strip (1), which a spring may carry a little past either end.
pub struct ModelCardView<'a> {
    pub card: &'a ModelCard,
    /// The whole of it, fading in and out.
    pub alpha: f32,
    /// Springing from the choice (0) to the strip at the note's foot (1).
    pub strip: f32,
    /// Seconds since it was first shown: its parts come in one after
    /// another.
    pub since: f32,
    /// The teal highlight gliding from the pick before (`ModelCard::from`)
    /// to this one, 0 to 1 (a spring: a little past 1 on the way).
    pub glide: f32,
}

/// The display face: Prata (SIL OFL, `assets/fonts`), built in and loaded
/// the first time the choice shows, never at launch.
const DISPLAY: &str = "Prata";
static PRATA: [&[u8]; 1] = [include_bytes!("../../../assets/fonts/Prata-Regular.ttf")];

#[cfg(test)]
thread_local! {
    /// Another face to try the choice in (`dump_model_card_faces`).
    static TRY: std::cell::RefCell<Option<(&'static str, Vec<&'static [u8]>)>> = const { std::cell::RefCell::new(None) };
}

/// The face the choice is set in, and its files.
fn display() -> (&'static str, Vec<&'static [u8]>) {
    #[cfg(test)]
    if let Some(f) = TRY.with(|t| t.borrow().clone()) {
        return f;
    }
    (DISPLAY, PRATA.to_vec())
}

/// The choice's width; the bird; the title's row; a choice's pill and the
/// gap between them; Download's capsule.
const W: f32 = 252.0;
const PAD: f32 = 16.0;
const BIRD: f32 = 32.0;
const HEAD: f32 = 31.0;
const TILE_W: f32 = 108.0;
const TILE: f32 = 30.0;
const GAP: f32 = 7.0;
const BUTTON_W: f32 = 116.0;
const BUTTON: f32 = 28.0;
const RADIUS: f32 = 14.0;
/// The strip's width and height, and its bar's height.
const STRIP_W: f32 = 228.0;
const STRIP_H: f32 = 44.0;
const BAR: f32 = 3.0;
/// The strip from the note area's foot; nothing nearer an edge than this.
const FOOT: f32 = 24.0;
const EDGE: f32 = 12.0;

/// The bird and the gap under it: left out where the window is too short.
const BIRD_H: f32 = BIRD + 13.0;
/// The choice's height, the bird included.
const CARD_H: f32 = BIRD_H + HEAD + 15.0 + TILE + 18.0 + BUTTON + 6.0 + 22.0;

/// The bird fits above the choice in a window this tall.
fn with_bird(window: Rect) -> bool {
    window.height() >= CARD_H + 2.0 * EDGE
}

/// The choice's height in `window`.
fn card_h(window: Rect) -> f32 {
    if with_bird(window) { CARD_H } else { CARD_H - BIRD_H }
}

/// Where the choice and the strip go: the choice in the middle of the
/// `window`, a little above centre; the strip at the foot of `area` (the
/// note's part of it), centred there.
fn place(window: Rect, area: Rect) -> (Rect, Rect) {
    let w = W.min(window.width() - 2.0 * EDGE).max(0.0);
    let mid = window.left + window.width() / 2.0;
    let h = card_h(window);
    let room = (window.height() - h).max(0.0);
    let top = (window.top + room * 0.42).max(window.top + EDGE.min(room / 2.0));
    let card = Rect::new(mid - w / 2.0, top, mid + w / 2.0, top + h.min(window.height()));
    let cx = area.left + area.width() / 2.0;
    let sw = STRIP_W.min(area.width() - 2.0 * EDGE).max(0.0);
    let foot = FOOT.min((area.height() - STRIP_H).max(0.0) / 2.0);
    let strip = Rect::new(cx - sw / 2.0, area.bottom - foot - STRIP_H, cx + sw / 2.0, area.bottom - foot);
    (card, strip)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Ease out (cubic): quick, then settling.
fn out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// A spring from 0 to 1 over `t` in 0..1, a little past 1 on the way.
fn spring(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    1.0 - (-6.0 * t).exp() * (8.5 * t).cos()
}

/// How far in a part that starts `delay` seconds after the choice shows
/// is, 0 to 1, over half a second.
fn arrive(since: f32, delay: f32) -> f32 {
    out((since - delay) / 0.5)
}

/// Draws the choice over the `window` (the page gone behind it), and its
/// strip at the foot of `area`, the part of the window the note has.
pub fn draw(eng: &Eng, fonts: &Fonts, view: &ModelCardView, hover: Option<Target>, window: Rect, area: Rect, scene: &mut Scene) -> Result<()> {
    let a = view.alpha.clamp(0.0, 1.0);
    if a <= 0.001 || view.card.choices.is_empty() {
        return Ok(());
    }
    let (face, files) = display();
    eng.borrow_mut().load_faces(face, &files);
    let card = view.card;
    let (big, small) = place(window, area);
    let t = view.strip.clamp(-0.15, 1.15);
    let r = Rect::new(lerp(big.left, small.left, t), lerp(big.top, small.top, t), lerp(big.right, small.right, t), lerp(big.bottom, small.bottom, t));
    scene.overlay(None);
    let (fills_from, texts_from, vectors_from) = (scene.fills.len(), scene.texts.len(), scene.vectors.len());
    let choosing = matches!(card.phase, Phase::Choose { .. });
    let since = view.since.max(0.0);
    // The page dissolves to plain paper (or graphite) behind the choice,
    // and comes back as it springs down to its strip.
    let veil = out(since / 0.35) * (1.0 - t).clamp(0.0, 1.0);
    if veil > 0.0 {
        scene.fills.push(Fill::faded(window, 0.0, Ink::Page, veil));
        if choosing {
            scene.hits.push((window, Target::Model(Hit::Card)));
        }
    }
    // The strip's panel grows in as it settles (no box while choosing).
    let boxed = ((t - 0.35) / 0.65).clamp(0.0, 1.0);
    if boxed > 0.0 {
        let round = r.height().min(r.width()) / 2.0 * boxed + RADIUS * (1.0 - boxed);
        scene.fills.push(Fill::shadow(r.offset(0.0, 4.0), round, 22.0, 1.4 * boxed));
        scene.fills.push(Fill::faded(r, round, Ink::Panel, boxed));
    }
    scene.hits.push((r, Target::Model(Hit::Card)));
    let on = |h: Hit| hover == Some(Target::Model(h));
    let text = |scene: &mut Scene, s: &str, f: &Format, b: Rect, ink: Ink, alpha: f32| {
        if alpha > 0.005 {
            let l = scene::single_line(eng, s, f, b.width().max(1.0), b.height());
            scene.texts.push(Text { x: b.left, y: b.top, layout: l, ink, alpha });
        }
    };
    let center = |f: &Format| Format { align: Align::Center, center_y: true, ..*f };
    // The choice's parts leave as it springs; the strip's come in.
    let (inside, strip_in) = ((1.0 - t / 0.45).clamp(0.0, 1.0), ((t - 0.5) / 0.5).clamp(0.0, 1.0));
    if inside > 0.0 {
        let mid = big.left + big.width() / 2.0;
        // Each part rises into place after the one before it.
        let rise = |delay: f32| {
            let e = arrive(since, delay);
            ((1.0 - e) * 10.0 - (1.0 - inside) * 13.0, e * inside)
        };
        let mut y = big.top;
        // The bird glides up and settles, teal with its white eye.
        if with_bird(window) {
            let s = spring((since - 0.05) / 0.7);
            let fade = arrive(since, 0.05);
            let lift = (1.0 - s) * 18.0 - (1.0 - inside) * 13.0;
            let bird = Rect::new(mid - BIRD / 2.0, y + lift, mid + BIRD / 2.0, y + lift + BIRD);
            for (logo, ink) in [(Logo::Bird, Ink::Accent), (Logo::BirdEye, Ink::OnAccent)] {
                scene.vectors.push(VectorMark { layer: scene.layer_now(), rect: bird, logo, ink, alpha: fade * inside });
            }
            y += BIRD_H;
        }
        // The title, in Prata.
        let (dy, k) = rise(0.16);
        let title = Format { face: Some(face), size: 24.0, weight: 400, ..center(&fonts.title) };
        text(scene, "Writing help", &title, Rect::new(big.left, y + dy, big.right, y + dy + HEAD), Ink::Text, k);
        y += HEAD + 15.0;
        // Two pills side by side, the chosen one under a teal tint that
        // glides from one to the other.
        let (dy, k) = rise(0.28);
        let n = card.choices.len() as f32;
        let tw = ((big.width() - GAP * (n - 1.0)) / n).min(TILE_W);
        let row_left = mid - (tw * n + GAP * (n - 1.0)) / 2.0;
        let pill = |i: usize| {
            let x = row_left + i as f32 * (tw + GAP);
            Rect::new(x, y + dy, x + tw, y + dy + TILE)
        };
        let round = TILE / 2.0;
        let fills_at = scene.fills.len();
        for i in 0..card.choices.len() {
            let pointed = choosing && on(Hit::Pick(i)) && card.picked() != i;
            scene.fills.push(Fill::solid(pill(i), round, if pointed { Ink::Hover } else { Ink::Field }));
            if choosing {
                scene.hits.push((pill(i), Target::Model(Hit::Pick(i))));
            }
        }
        let (from, to) = (pill(card.from.min(card.choices.len() - 1)), pill(card.picked()));
        let g = view.glide;
        let lit = Rect::new(lerp(from.left, to.left, g), to.top, lerp(from.right, to.right, g), to.bottom);
        scene.fills.push(Fill::faded(lit, round, Ink::Accent, 0.16));
        let name = Format { face: Some(face), size: 13.5, weight: 400, center_y: true, ..fonts.label };
        let size = Format { size: 11.0, center_y: true, ..fonts.label };
        for (i, c) in card.choices.iter().enumerate() {
            let p = pill(i);
            let chosen = card.picked() == i;
            let label = size_label(c.bytes);
            let nw = scene::single_line(eng, c.name, &name, tw, TILE).measure().width;
            let sw = scene::single_line(eng, &label, &size, tw, TILE).measure().width;
            let total = (nw + 5.0 + sw).min(tw - 14.0);
            let x = p.left + (tw - total) / 2.0;
            text(scene, c.name, &name, Rect::new(x, p.top, (x + nw + 1.0).min(p.right - 10.0), p.bottom), if chosen || (choosing && on(Hit::Pick(i))) { Ink::Text } else { Ink::Muted }, k);
            let sx = x + nw + 5.0;
            text(scene, &label, &size, Rect::new(sx, p.top + 1.0, (sx + sw + 1.0).min(p.right - 10.0).max(sx + 1.0), p.bottom + 1.0), if chosen { Ink::Muted } else { Ink::Faint }, k);
        }
        scene.fills[fills_at..].iter_mut().for_each(|f| f.alpha *= k);
        y += TILE + 18.0;
        // Download: a teal capsule; once pressed it becomes the progress
        // itself, filling from the left, before it springs down.
        let (dy, k) = rise(0.4);
        let down = Rect::new(mid - BUTTON_W / 2.0, y + dy, mid + BUTTON_W / 2.0, y + dy + BUTTON);
        let round = BUTTON / 2.0;
        let fills_at = scene.fills.len();
        let word = Format { size: 12.0, weight: 600, ..center(&fonts.label) };
        match card.phase {
            Phase::Choose { .. } => {
                let hot = on(Hit::Download);
                scene.fills.push(Fill::shadow(down.offset(0.0, 3.0), round, 12.0, if hot { 1.1 } else { 0.7 }));
                scene.fills.push(Fill::solid(down, round, Ink::Accent));
                if hot {
                    scene.fills.push(Fill::faded(down, round, Ink::OnAccent, 0.12));
                }
                text(scene, "Download", &word, down, Ink::OnAccent, k);
                scene.hits.push((down, Target::Model(Hit::Download)));
            }
            Phase::Downloading { .. } => {
                scene.fills.push(Fill::solid(down, round, Ink::Field));
                let w = (down.width() * card.percent() as f32 / 100.0).max(down.height());
                scene.fills.push(Fill::solid(Rect::new(down.left, down.top, down.left + w, down.bottom), round, Ink::Accent));
                let pct = format!("{}%", card.percent());
                text(scene, &pct, &word, down, if card.percent() >= 55 { Ink::OnAccent } else { Ink::Text }, k);
            }
        }
        scene.fills[fills_at..].iter_mut().for_each(|f| f.alpha *= k);
        y += BUTTON + 6.0;
        // Not now: a faint word under it.
        let (dy, k) = rise(0.5);
        let not = Rect::new(mid - 40.0, y + dy, mid + 40.0, y + dy + 22.0);
        if choosing {
            if on(Hit::NotNow) {
                let at = scene.fills.len();
                scene.fills.push(Fill::solid(not, 8.0, Ink::Hover));
                scene.fills[at].alpha *= k;
            }
            let quiet = Format { size: 11.5, ..center(&fonts.label) };
            text(scene, "Not now", &quiet, not, if on(Hit::NotNow) { Ink::Text } else { Ink::Faint }, k);
            scene.hits.push((not, Target::Model(Hit::NotNow)));
        }
    }
    // The strip's contents keep inside it.
    scene.overlay(Some(r));
    if let (Phase::Downloading { picked, .. }, true) = (card.phase, strip_in > 0.0) {
        let fill_at = scene.fills.len();
        let s = small;
        let x0 = s.left + PAD;
        let row = Rect::new(x0, s.top + 6.0, s.right - 8.0, s.top + 32.0);
        // The ×, small, at the right.
        let cancel = Rect::new(row.right - 26.0, row.top, row.right, row.bottom);
        if on(Hit::Cancel) {
            scene.fills.push(Fill::solid(cancel, 7.0, Ink::Hover));
        }
        text(scene, "\u{E8BB}", &fonts.icon_small, cancel, if on(Hit::Cancel) { Ink::Text } else { Ink::Muted }, strip_in);
        scene.hits.push((cancel, Target::Model(Hit::Cancel)));
        // "Light · 64%": the name in Prata, then how far, quieter.
        let name = card.choices.get(picked).map_or("", |c| c.name);
        let room = Rect::new(row.left + 2.0, row.top, cancel.left - 6.0, row.bottom);
        let face = Format { face: Some(face), weight: 400, size: 16.0, ..fonts.label };
        let l = scene::single_line(eng, name, &face, room.width().max(1.0), room.height());
        let nw = l.measure().width.min(room.width());
        scene.texts.push(Text { x: room.left, y: room.top, layout: l, ink: Ink::Text, alpha: strip_in });
        let pct = format!("\u{00B7}  {}%", card.percent());
        text(scene, &pct, &fonts.label, Rect::new(room.left + nw + 6.0, room.top, room.right.max(room.left + nw + 7.0), room.bottom), Ink::Muted, strip_in);
        // The bar: a teal fill on a field-coloured track.
        let track = Rect::new(x0, s.bottom - 10.0 - BAR, s.right - PAD, s.bottom - 10.0);
        scene.fills.push(Fill::solid(track, BAR / 2.0, Ink::Field));
        let w = track.width() * card.percent() as f32 / 100.0;
        if w > 0.5 {
            scene.fills.push(Fill::solid(Rect::new(track.left, track.top, track.left + w.max(BAR), track.bottom), BAR / 2.0, Ink::Accent));
        }
        scene.fills[fill_at..].iter_mut().for_each(|f| f.alpha *= strip_in);
    }
    if a < 1.0 {
        scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= a);
        scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= a);
        scene.vectors[vectors_from..].iter_mut().for_each(|v| v.alpha *= a);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue() -> Vec<Choice> {
        vec![Choice { id: "light", name: "Light", bytes: 229_600_000 }, Choice { id: "full", name: "Full", bytes: 730_400_000 }]
    }

    fn open(card: &ModelCard, strip: f32, width: f32, height: f32, hover: Option<Target>) -> Scene {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let mut scene = Scene::default();
        draw(&eng, &fonts, &ModelCardView { card, alpha: 1.0, strip, since: 10.0, glide: 1.0 }, hover, Rect::new(0.0, 0.0, width, height), Rect::new(0.0, 0.0, width, height), &mut scene).unwrap();
        scene
    }

    fn rect(scene: &Scene, hit: Hit) -> Option<Rect> {
        // The last: the veil behind the card takes the same hit.
        scene.hits.iter().rev().find(|(_, t)| *t == Target::Model(hit)).map(|(r, _)| *r)
    }

    #[test]
    fn the_model_card_walks_from_choose_to_downloading_and_back_on_cancel() {
        let mut card = ModelCard::new(catalogue());
        let s = open(&card, 0.0, 1200.0, 800.0, None);
        for h in [Hit::Pick(0), Hit::Pick(1), Hit::Download, Hit::NotNow] {
            assert!(rect(&s, h).is_some(), "{h:?} while choosing");
        }
        assert!(rect(&s, Hit::Cancel).is_none());
        let c = rect(&s, Hit::Card).unwrap();
        assert!(((c.left + c.right) / 2.0 - 600.0).abs() < 0.5 && c.top > 100.0 && c.bottom < 700.0 && c.width() >= 240.0, "in the middle: {c:?}");
        // The page behind dims and takes no clicks while choosing.
        assert!(s.fills.iter().any(|f| f.ink == Ink::Page && f.rect == Rect::new(0.0, 0.0, 1200.0, 800.0)), "a veil over the page");
        assert!(s.hits.iter().any(|(r, t)| *t == Target::Model(Hit::Card) && *r == Rect::new(0.0, 0.0, 1200.0, 800.0)));
        assert!(!s.fills.iter().any(|f| f.ink == Ink::Danger), "nothing red");
        // Full, then Download.
        assert_eq!(card.click(Hit::Pick(1)), Command::None);
        assert_eq!(card.phase, Phase::Choose { picked: 1 });
        assert_eq!(card.click(Hit::Pick(7)), Command::None, "no such tile");
        assert_eq!(card.phase, Phase::Choose { picked: 1 });
        assert_eq!(card.click(Hit::Download), Command::Download(1));
        assert_eq!(card.phase, Phase::Downloading { picked: 1, done: 0, total: 730_400_000 });
        // Picking and Download do nothing while it comes down.
        assert_eq!(card.click(Hit::Pick(0)), Command::None);
        assert_eq!(card.click(Hit::Download), Command::None);
        assert_eq!(card.click(Hit::NotNow), Command::None);
        card.progress(467_456_000, 730_400_000);
        assert_eq!(card.percent(), 64);
        card.progress(900_000_000, 0);
        assert_eq!(card.percent(), 100, "never past all of it");
        card.progress(467_456_000, 730_400_000);
        // The strip: in the same corner, smaller, with the × and the bar.
        let strip = open(&card, 1.0, 1200.0, 800.0, None);
        let r = rect(&strip, Hit::Card).unwrap();
        assert!(r.height() < c.height() / 2.0 && r.width() < c.width() && ((r.left + r.right) / 2.0 - 600.0).abs() < 0.5 && r.bottom > 700.0 && r.bottom <= 800.0 - 12.0, "a slim strip at the foot, centred: {r:?}");
        assert!(!strip.fills.iter().any(|f| f.ink == Ink::Page && f.rect == Rect::new(0.0, 0.0, 1200.0, 800.0)), "the veil lifts while it comes down");
        assert!(rect(&strip, Hit::Download).is_none() && rect(&strip, Hit::Pick(0)).is_none());
        let x = rect(&strip, Hit::Cancel).expect("the ×");
        assert!(x.right <= r.right && x.width() >= 20.0 && x.height() >= 20.0);
        let bar: Vec<&Fill> = strip.fills.iter().filter(|f| f.rect.height() <= BAR + 0.01).collect();
        assert!(bar.iter().any(|f| f.ink == Ink::Field) && bar.iter().any(|f| f.ink == Ink::Accent), "a teal fill on a field-coloured track");
        let teal = bar.iter().find(|f| f.ink == Ink::Accent).unwrap().rect.width();
        let track = bar.iter().find(|f| f.ink == Ink::Field).unwrap().rect.width();
        assert!((teal / track - 0.64).abs() < 0.01, "{teal} of {track}");
        // Cancel: back to choosing, Full still picked.
        assert_eq!(card.click(Hit::Cancel), Command::Cancel);
        assert_eq!(card.phase, Phase::Choose { picked: 1 });
        assert_eq!(card.percent(), 0);
        assert_eq!(card.click(Hit::Cancel), Command::None);
        assert_eq!(card.click(Hit::Card), Command::None);
        // Faded out, nothing is drawn or clicked.
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let mut gone = Scene::default();
        draw(&eng, &Fonts::new(1.0, 0), &ModelCardView { card: &card, alpha: 0.0, strip: 0.0, since: 10.0, glide: 1.0 }, None, Rect::new(0.0, 0.0, 800.0, 600.0), Rect::new(0.0, 0.0, 800.0, 600.0), &mut gone).unwrap();
        assert!(gone.hits.is_empty() && gone.fills.is_empty());
    }

    #[test]
    fn its_parts_come_in_one_after_another_and_the_tint_glides() {
        let mut card = ModelCard::new(catalogue());
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let at = |card: &ModelCard, since: f32, glide: f32| {
            let mut scene = Scene::default();
            let w = Rect::new(0.0, 0.0, 1200.0, 800.0);
            draw(&eng, &fonts, &ModelCardView { card, alpha: 1.0, strip: 0.0, since, glide }, None, w, w, &mut scene).unwrap();
            scene
        };
        // At first only the bird is on its way; the rest follows.
        let first = at(&card, 0.1, 1.0);
        assert!(first.vectors.iter().all(|v| v.alpha > 0.0) && first.texts.iter().all(|t| t.alpha < 0.05), "the bird first");
        let settled = at(&card, 3.0, 1.0);
        assert!(settled.texts.iter().all(|t| t.alpha > 0.99), "all of it, settled");
        // A new pick: the teal tint is between the two pills halfway.
        card.click(Hit::Pick(1));
        assert_eq!(card.from, 0);
        let tint = |s: &Scene| s.fills.iter().find(|f| f.ink == Ink::Accent && f.alpha < 0.5).map(|f| f.rect).unwrap();
        let (a, b) = (rect(&at(&card, 3.0, 0.0), Hit::Pick(0)).unwrap(), rect(&at(&card, 3.0, 0.0), Hit::Pick(1)).unwrap());
        assert!((tint(&at(&card, 3.0, 0.0)).left - a.left).abs() < 0.5);
        let half = tint(&at(&card, 3.0, 0.5));
        assert!(half.left > a.left + 1.0 && half.left < b.left - 1.0, "gliding");
        assert!((tint(&at(&card, 3.0, 1.0)).left - b.left).abs() < 0.5);
    }

    #[test]
    fn light_is_preselected_and_not_now_says_so() {
        let mut card = ModelCard::new(catalogue());
        assert_eq!(card.phase, Phase::Choose { picked: 0 });
        let s = open(&card, 0.0, 1000.0, 700.0, None);
        let (light, full) = (rect(&s, Hit::Pick(0)).unwrap(), rect(&s, Hit::Pick(1)).unwrap());
        assert!(light.right <= full.left && (light.top - full.top).abs() < 0.5, "side by side");
        let tinted = |r: Rect| s.fills.iter().any(|f| f.rect == r && f.ink == Ink::Accent);
        assert!(tinted(light) && !tinted(full), "Light is the chosen one");
        // Each tile says only its name and its size.
        let words = |r: Rect| s.texts.iter().filter(|t| r.contains(t.x + 0.5, t.y + 0.5)).count();
        assert_eq!((words(light), words(full)), (2, 2));
        let down = rect(&s, Hit::Download).unwrap();
        let not = rect(&s, Hit::NotNow).unwrap();
        let mid = |r: Rect| (r.left + r.right) / 2.0;
        assert!(not.top >= down.bottom && (mid(not) - mid(down)).abs() < 0.5 && (mid(down) - (light.left + full.right) / 2.0).abs() < 0.5, "Download centred under the choices, Not now under it");
        assert_eq!(s.vectors.iter().map(|v| v.logo).collect::<Vec<_>>(), [crate::vector::Logo::Bird, crate::vector::Logo::BirdEye], "the bird asks");
        assert!(s.fills.iter().any(|f| f.rect == down && f.ink == Ink::Accent), "Download is teal");
        assert!(!s.fills.iter().any(|f| f.rect == not), "Not now is quiet at rest");
        assert_eq!(card.click(Hit::NotNow), Command::NotNow);
        assert_eq!(card.phase, Phase::Choose { picked: 0 }, "the shell lets it go");
    }

    #[test]
    fn sizes_read_as_megabytes() {
        assert_eq!(size_label(229_600_000), "230 MB");
        assert_eq!(size_label(730_400_000), "730 MB");
        assert_eq!(size_label(230_000_000), "230 MB");
        assert_eq!(size_label(499_999), "0 MB");
        assert_eq!(size_label(1_240_000_000), "1.2 GB");
    }

    #[test]
    fn settings_writing_help_reads_light_full_or_off() {
        assert_eq!(setting_value(false, Some(0)), 0);
        assert_eq!(setting_value(false, Some(1)), 1);
        assert_eq!(setting_value(false, None), 0, "a model it no longer has reads as the first");
        assert_eq!(setting_value(true, Some(1)), OFF);
        assert_eq!(from_setting(0), (false, Some(0)));
        assert_eq!(from_setting(1), (false, Some(1)));
        assert_eq!(from_setting(OFF), (true, None));
    }

    #[test]
    #[ignore = "writes the choice in other faces (KOLADA_TRY_FONTS: Family=file,file;...) to KOLADA_DUMP_DIR"]
    fn dump_model_card_faces() {
        let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
        for spec in std::env::var("KOLADA_TRY_FONTS").unwrap().split(';').filter(|s| !s.is_empty()) {
            let (family, files) = spec.split_once('=').unwrap();
            let family: &'static str = Box::leak(family.to_string().into_boxed_str());
            let data: Vec<&'static [u8]> = files.split(',').map(|f| &*Box::leak(std::fs::read(f).unwrap().into_boxed_slice())).collect();
            TRY.with(|t| *t.borrow_mut() = Some((family, data)));
            let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
            let fonts = Fonts::new(1.0, 0);
            let card = ModelCard::new(catalogue());
            for (theme_name, theme) in [("dark", &crate::theme::DARK), ("light", &crate::theme::LIGHT)] {
                let input = scene::Input {
                    vault_name: "my notes",
                    page_fade: 1.0,
                    toolbar_dock: 1.0,
                    toolbar_show: 1.0,
                    model_card: Some(ModelCardView { card: &card, alpha: 1.0, strip: 0.0, since: 10.0, glide: 1.0 }),
                    ..Default::default()
                };
                let s = scene::build(&eng, &fonts, &input, 1000.0, 560.0).unwrap();
                let list = crate::display::record(&eng, &s, theme, 1.5).unwrap();
                crate::raster::Surface::draw_new((1500, 840, 144), &list).dump(&dir.join(format!("face-{}-{theme_name}.bmp", family.replace(' ', "-")))).unwrap();
            }
        }
        TRY.with(|t| *t.borrow_mut() = None);
    }

    #[test]
    #[ignore = "writes the model card in each phase for manual inspection (to KOLADA_DUMP_DIR)"]
    fn dump_model_card() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let text = "# Weekly plan\nShip the frameless window and the side menu, then test it on every screen size.\n\nA second paragraph that runs long enough to pass under the card at the bottom right of the page, so the card is seen over words.\n";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
        let choose = ModelCard::new(catalogue());
        let mut downloading = choose.clone();
        downloading.click(Hit::Download);
        downloading.progress(147_000_000, 229_600_000);
        let mut gliding = choose.clone();
        gliding.click(Hit::Pick(1));
        // (name, card, strip, hover, since, glide)
        let cases = [
            ("arriving", &choose, 0.0, None, 0.32, 1.0),
            ("choose", &choose, 0.0, None, 10.0, 1.0),
            ("choose-hover-full", &choose, 0.0, Some(Target::Model(Hit::Pick(1))), 10.0, 1.0),
            ("gliding", &gliding, 0.0, None, 10.0, 0.55),
            ("filling", &downloading, 0.0, None, 10.0, 1.0),
            ("springing", &downloading, 0.5, None, 10.0, 1.0),
            ("downloading", &downloading, 1.0, None, 10.0, 1.0),
        ];
        for (name, card, strip, hover, since, glide) in cases {
            for (theme_name, theme) in [("dark", &crate::theme::DARK), ("light", &crate::theme::LIGHT)] {
                let input = scene::Input {
                    vault_name: "my notes",
                    active_note: "Weekly plan",
                    text,
                    selection: (2, 2),
                    lines: &lines,
                    page_fade: 1.0,
                    toolbar_dock: 1.0,
                    toolbar_show: 1.0,
                    hover,
                    model_card: Some(ModelCardView { card, alpha: 1.0, strip, since, glide }),
                    ..Default::default()
                };
                let s = scene::build(&eng, &fonts, &input, 1000.0, 560.0).unwrap();
                let list = crate::display::record(&eng, &s, theme, 1.5).unwrap();
                crate::raster::Surface::draw_new((1500, 840, 144), &list).dump(&dir.join(format!("model-{name}-{theme_name}.bmp"))).unwrap();
            }
        }
    }

    /// Done, the shell says so with a passing word at the top of the page.
    #[test]
    fn a_whisper_can_say_the_writing_help_is_ready() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        for width in [360.0, 1200.0] {
            let input = scene::Input { page_fade: 1.0, whisper: Some(scene::Whisper { text: "Writing help is ready", alpha: 1.0 }), ..Default::default() };
            let s = scene::build(&eng, &fonts, &input, width, 600.0).unwrap();
            let (right, _) = crate::chrome::whisper_place(&input, s.column, width);
            let room = (right - s.column.left).clamp(80.0, 360.0);
            let said = scene::layout(&eng, "\u{E73E}  Writing help is ready", &fonts.small, 1000.0, 20.0).measure().width;
            assert!(said < room && right <= width, "it fits its room at {width}: {said} of {room}");
        }
    }
}
