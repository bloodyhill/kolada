//! The share card: Send's "Share link…" opens it under the button, in the
//! menu's card style and with its fade. Three pills choose how long the
//! link lives (7 days unless chosen otherwise, and the choice is kept), one
//! teal button makes it, and once made its address shows large, already
//! copied. Below, this install's live links (three at most), the open
//! note's first, each with Copy and Delete; Delete asks "Delete?" on the
//! button itself and a second click confirms. Nothing in it is red.
//!
//! What the card holds and does is `Card` (plain data, every shell's); the
//! shell sends and receives on a thread of its own and tells the card what
//! came back. `draw` puts it on the screen from a `ShareView`.

use kolada_core::share::{Expiry, Link, MOST_LIVE, Problem, short};
use kolada_text::{Align, Format};
use windows_core::Result;

use crate::geom::{Fill, Ink, Rect};
use crate::scene::{self, Eng, Fonts, Scene, Target, Text};

/// What a click on the card is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    /// Anywhere outside it: it closes, as a menu does.
    Shade,
    Card,
    Expiry(Expiry),
    Create,
    /// Copy the link just made, again.
    CopyNew,
    /// A listed link's Copy and Delete, by its place in the list.
    Copy(usize),
    Delete(usize),
}

/// Where the card is.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Choosing how long the link lives.
    Choose,
    /// The note going up.
    Creating,
    /// Made: its address (on the clipboard already).
    Ready { url: String },
    /// It could not be made (or a link deleted): what to say.
    Failed(String),
}

/// What was copied last: the link just made, or a listed one (its id).
#[derive(Clone, Debug, PartialEq)]
pub enum Copied {
    New,
    Link(String),
}

/// What the shell does after a click.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Nothing,
    Close,
    /// Make a link for the open note, living this long.
    Create(Expiry),
    /// This address on the clipboard.
    Copy(String),
    /// Delete the link with this id.
    Delete(String),
}

/// The card's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub expiry: Expiry,
    pub phase: Phase,
    /// This install's live links, the open note's first, then the newest.
    pub links: Vec<Link>,
    /// The open note (vault-relative, with `.md`).
    pub note: String,
    /// The link whose Delete asks "Delete?".
    pub confirming: Option<String>,
    /// Links being deleted: faded, their buttons gone.
    pub deleting: Vec<String>,
    pub copied: Option<Copied>,
}

impl Card {
    /// Opened for `note`, with the links known so far (kept here, shown at
    /// once while the server's list comes) and the expiry chosen last.
    pub fn new(expiry: Expiry, note: &str, links: &[Link], now: u64) -> Card {
        let mut c = Card { expiry, phase: Phase::Choose, links: Vec::new(), note: note.to_string(), confirming: None, deleting: Vec::new(), copied: None };
        c.set_links(links, now);
        c
    }

    fn set_links(&mut self, links: &[Link], now: u64) {
        let mut links: Vec<Link> = links.iter().filter(|l| l.expires > now).cloned().collect();
        let note = self.note.clone();
        links.sort_by(|a, b| (b.note == note && !note.is_empty()).cmp(&(a.note == note && !note.is_empty())).then(b.created.cmp(&a.created)));
        self.links = links;
    }

    /// As many links are live as an install may have.
    pub fn at_limit(&self) -> bool {
        self.links.len() >= MOST_LIVE
    }

    /// Whether Create makes a link now.
    pub fn can_create(&self) -> bool {
        !self.at_limit() && matches!(self.phase, Phase::Choose | Phase::Failed(_))
    }

    /// What to say above Create, if anything: why it failed, or why it
    /// cannot be pressed.
    pub fn message(&self) -> Option<String> {
        match &self.phase {
            Phase::Failed(m) => Some(m.clone()),
            Phase::Choose if self.at_limit() => Some(Problem::Limit { live: self.links.len() }.message()),
            _ => None,
        }
    }

    /// A click on the card (`None`: on nothing of the card's).
    pub fn click(&mut self, hit: Hit) -> Command {
        let confirming = self.confirming.take();
        match hit {
            Hit::Shade => Command::Close,
            Hit::Card => Command::Nothing,
            Hit::Expiry(e) => {
                if matches!(self.phase, Phase::Choose | Phase::Failed(_)) {
                    self.expiry = e;
                }
                Command::Nothing
            }
            Hit::Create if self.can_create() => {
                self.phase = Phase::Creating;
                Command::Create(self.expiry)
            }
            Hit::Create => Command::Nothing,
            Hit::CopyNew => match &self.phase {
                Phase::Ready { url } => {
                    self.copied = Some(Copied::New);
                    Command::Copy(url.clone())
                }
                _ => Command::Nothing,
            },
            Hit::Copy(i) => match self.links.get(i) {
                Some(l) if !self.deleting.contains(&l.id) => {
                    self.copied = Some(Copied::Link(l.id.clone()));
                    Command::Copy(l.url())
                }
                _ => Command::Nothing,
            },
            Hit::Delete(i) => match self.links.get(i).map(|l| l.id.clone()) {
                Some(id) if self.deleting.contains(&id) => Command::Nothing,
                Some(id) if confirming.as_ref() == Some(&id) => {
                    self.deleting.push(id.clone());
                    Command::Delete(id)
                }
                Some(id) => {
                    self.confirming = Some(id);
                    Command::Nothing
                }
                None => Command::Nothing,
            },
        }
    }

    /// The link was made: its address shows, copied, and it joins the list.
    pub fn created(&mut self, link: Link, now: u64) {
        self.phase = Phase::Ready { url: link.url() };
        self.copied = Some(Copied::New);
        let mut links = self.links.clone();
        links.retain(|l| l.id != link.id);
        links.push(link);
        self.set_links(&links, now);
    }

    /// It could not be made.
    pub fn failed(&mut self, problem: &Problem) {
        self.phase = Phase::Failed(problem.message());
    }

    /// The server's list arrived (merged with what is kept here).
    pub fn listed(&mut self, links: &[Link], now: u64) {
        self.set_links(links, now);
        self.deleting.retain(|id| self.links.iter().any(|l| &l.id == id));
    }

    /// A delete came back: gone, or why not.
    pub fn deleted(&mut self, id: &str, result: std::result::Result<(), Problem>, now: u64) {
        self.deleting.retain(|d| d != id);
        match result {
            Ok(()) => {
                let links: Vec<Link> = self.links.iter().filter(|l| l.id != id).cloned().collect();
                self.set_links(&links, now);
                if self.copied == Some(Copied::Link(id.to_string())) {
                    self.copied = None;
                }
            }
            Err(p) if !matches!(self.phase, Phase::Creating) => self.phase = Phase::Failed(p.message()),
            Err(_) => {}
        }
    }
}

/// How long a link has left, in a few words.
pub fn left_words(expires: u64, now: u64) -> String {
    let left = expires.saturating_sub(now);
    let (days, hours) = (left / 86_400, left / 3_600);
    match (days, hours) {
        (0, 0) => "Under an hour left".into(),
        (0, 1) => "1 hour left".into(),
        (0, h) => format!("{h} hours left"),
        (1, _) => "1 day left".into(),
        (d, _) => format!("{d} days left"),
    }
}

/// The card as the view shows it: its state, how far it has faded in, now
/// (Unix seconds, for what is left of each link).
pub struct ShareView<'a> {
    pub card: &'a Card,
    pub alpha: f32,
    pub now: u64,
}

/// The card's width, its padding, a row's height.
const W: f32 = 340.0;
const PAD: f32 = 10.0;
const ROW: f32 = 32.0;
const PILLS: f32 = 32.0;
const BUTTON: f32 = 36.0;
const URL: f32 = 40.0;
const EDGE: f32 = 8.0;

/// Where the card goes under the Send button (`send`), and its width: its
/// right edge at the button's, inside the window.
fn place(send: Option<Rect>, width: f32) -> (f32, f32, f32) {
    let w = W.min(width - 2.0 * EDGE);
    let (right, top) = send.map_or((width - EDGE, 46.0), |b| (b.right, b.bottom + 6.0));
    let left = (right.min(width - EDGE) - w).max(EDGE);
    (left, top, w)
}

pub fn draw(eng: &Eng, fonts: &Fonts, view: &ShareView, hover: Option<Target>, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    let a = view.alpha.clamp(0.0, 1.0);
    if a <= 0.001 {
        return Ok(());
    }
    let card = view.card;
    let send = scene.hits.iter().rev().find(|(_, t)| *t == Target::SendTo).map(|(r, _)| *r);
    scene.overlay(None);
    let (fills_from, texts_from) = (scene.fills.len(), scene.texts.len());
    scene.hits.push((Rect::new(0.0, 0.0, width, height), Target::Share(Hit::Shade)));
    let (left, top, w) = place(send, width);
    let inner = w - 2.0 * PAD;
    let small = fonts.small.clone();
    let label = fonts.label.clone();
    let choice = fonts.choice.clone();
    let wrapped = Format { wrap: true, center_y: false, ..fonts.peek.clone() };

    // The top: the pills, what to say, Create; or the link made.
    let ready = match &card.phase {
        Phase::Ready { url } => Some(url.as_str()),
        _ => None,
    };
    let message = card.message().map(|m| scene::layout(eng, &m, &wrapped, inner, 200.0));
    let said = message.as_ref().map_or(0.0, |l| l.measure().height + 8.0);
    let head = if ready.is_some() { PAD + URL + ROW + PAD } else { PAD + PILLS + 8.0 + said + BUTTON + PAD };
    // Then the links that fit; the card moves up where the window ends.
    let list_top = if card.links.is_empty() { 0.0 } else { 9.0 };
    let wanted = head + list_top + card.links.len() as f32 * ROW + if card.links.is_empty() { 0.0 } else { 6.0 };
    let top = if top + wanted > height - EDGE { (height - EDGE - wanted).max(EDGE) } else { top };
    let room = height - EDGE - top - head - list_top - 6.0;
    let shown = if card.links.is_empty() { 0 } else { ((room / ROW).floor().max(0.0) as usize).min(card.links.len()) };
    let h = head + if shown > 0 { list_top + shown as f32 * ROW + 6.0 } else { 0.0 };
    let r = Rect::new(left, top, left + w, top + h);
    scene.fills.push(Fill::shadow(r.offset(0.0, 6.0), 12.0, 36.0, 2.0));
    scene.fills.push(Fill::solid(r, 12.0, Ink::Border));
    scene.fills.push(Fill::solid(r.inset(1.0), 11.0, Ink::Panel));
    scene.hits.push((r, Target::Share(Hit::Card)));
    let on = |t: Hit| hover == Some(Target::Share(t));
    let x0 = r.left + PAD;
    let mut y = r.top + PAD;
    let text = |scene: &mut Scene, s: &str, f: &Format, b: Rect, ink: Ink| {
        let l = scene::single_line(eng, s, f, b.width().max(1.0), b.height());
        scene.texts.push(Text { x: b.left, y: b.top, layout: l, ink, alpha: 1.0 });
    };
    match ready {
        Some(url) => {
            let big = Format { size: 20.0, weight: 600, line: Some(26.0), ..label.clone() };
            text(scene, short(url), &big, Rect::new(x0 + 2.0, y, x0 + inner, y + URL), Ink::Text);
            y += URL;
            let copy = Rect::new(r.right - PAD - 72.0, y + 2.0, r.right - PAD, y + ROW - 2.0);
            if card.copied == Some(Copied::New) {
                text(scene, "\u{E73E}", &fonts.icon_small, Rect::new(x0, y, x0 + 18.0, y + ROW), Ink::Accent);
                text(scene, "Copied", &small, Rect::new(x0 + 20.0, y, copy.left - 8.0, y + ROW), Ink::Accent);
            }
            scene.fills.push(Fill::solid(copy, 7.0, if on(Hit::CopyNew) { Ink::Hover } else { Ink::Field }));
            scene.hits.push((copy, Target::Share(Hit::CopyNew)));
            let l = scene::single_line(eng, "Copy", &choice, copy.width(), copy.height());
            scene.texts.push(Text { x: copy.left, y: copy.top, layout: l, ink: if on(Hit::CopyNew) { Ink::Text } else { Ink::Muted }, alpha: 1.0 });
            y += ROW;
        }
        None => {
            // How long it lives: one of three, as the options choose.
            let bar = Rect::new(x0, y, x0 + inner, y + PILLS);
            scene.fills.push(Fill::solid(bar, 8.0, Ink::Field));
            let pw = inner / Expiry::ALL.len() as f32;
            for (k, e) in Expiry::ALL.into_iter().enumerate() {
                let p = Rect::new(bar.left + k as f32 * pw, bar.top, bar.left + (k + 1) as f32 * pw, bar.bottom).inset(2.0);
                let chosen = card.expiry == e;
                if chosen {
                    scene.fills.push(Fill::shadow(p.offset(0.0, 1.0), 6.0, 4.0, 0.8));
                    scene.fills.push(Fill::solid(p, 6.0, Ink::Raised));
                } else if on(Hit::Expiry(e)) {
                    scene.fills.push(Fill::solid(p, 6.0, Ink::Hover));
                }
                scene.hits.push((p, Target::Share(Hit::Expiry(e))));
                let l = scene::single_line(eng, e.label(), &choice, p.width(), p.height());
                scene.texts.push(Text { x: p.left, y: p.top, layout: l, ink: if chosen || on(Hit::Expiry(e)) { Ink::Text } else { Ink::Muted }, alpha: 1.0 });
            }
            y += PILLS + 8.0;
            if let Some(l) = message {
                scene.texts.push(Text { x: x0 + 2.0, y, layout: l, ink: Ink::Muted, alpha: 1.0 });
                y += said;
            }
            let b = Rect::new(x0, y, x0 + inner, y + BUTTON);
            let creating = card.phase == Phase::Creating;
            let (fill, ink, alpha) = match (card.can_create(), creating) {
                (_, true) => (Ink::Accent, Ink::OnAccent, 0.6),
                (true, _) => (Ink::Accent, Ink::OnAccent, if on(Hit::Create) { 0.88 } else { 1.0 }),
                (false, _) => (Ink::Field, Ink::Faint, 1.0),
            };
            scene.fills.push(Fill::faded(b, 8.0, fill, alpha));
            scene.hits.push((b, Target::Share(Hit::Create)));
            let words = if creating { "Creating link\u{2026}" } else { "Create link" };
            let l = scene::single_line(eng, words, &Format { weight: 600, ..choice.clone() }, b.width(), b.height());
            scene.texts.push(Text { x: b.left, y: b.top, layout: l, ink, alpha: 1.0 });
            y += BUTTON;
        }
    }
    y += PAD;
    // The live links.
    if shown > 0 {
        scene.fills.push(Fill::solid(Rect::new(r.left + 12.0, y + 4.0, r.right - 12.0, y + 5.0), 0.0, Ink::Border));
        y += list_top;
    }
    for (i, link) in card.links.iter().take(shown).enumerate() {
        let row = Rect::new(r.left + 4.0, y, r.right - 4.0, y + ROW);
        let going = card.deleting.contains(&link.id);
        let asking = card.confirming.as_ref() == Some(&link.id);
        let del_w = if asking { 66.0 } else { 28.0 };
        let del = Rect::new(row.right - 4.0 - del_w, y + 2.0, row.right - 4.0, y + ROW - 2.0);
        let copy = Rect::new(del.left - 30.0, y + 2.0, del.left - 2.0, y + ROW - 2.0);
        let left_w = 92.0;
        let when = Rect::new(copy.left - 6.0 - left_w, y, copy.left - 6.0, y + ROW);
        let name = if link.title.is_empty() { short_url(&link.url()) } else { link.title.clone() };
        let texts_at = scene.texts.len();
        text(scene, &name, &Format { ellipsis: true, ..label.clone() }, Rect::new(row.left + 6.0, y, when.left - 6.0, y + ROW), Ink::Text);
        text(scene, &left_words(link.expires, view.now), &Format { align: Align::End, ..small.clone() }, when, Ink::Faint);
        if going {
            scene.texts[texts_at..].iter_mut().for_each(|t| t.alpha *= 0.4);
        } else {
            let copied = card.copied == Some(Copied::Link(link.id.clone()));
            if on(Hit::Copy(i)) {
                scene.fills.push(Fill::solid(copy, 6.0, Ink::Hover));
            }
            scene.hits.push((copy, Target::Share(Hit::Copy(i))));
            let (glyph, ink) = if copied { ("\u{E73E}", Ink::Accent) } else { ("\u{E8C8}", if on(Hit::Copy(i)) { Ink::Text } else { Ink::Muted }) };
            text(scene, glyph, &fonts.icon_small, copy, ink);
            if asking {
                scene.fills.push(Fill::solid(del, 6.0, Ink::Selection));
                let l = scene::single_line(eng, "Delete?", &choice, del.width(), del.height());
                scene.texts.push(Text { x: del.left, y: del.top, layout: l, ink: Ink::Text, alpha: 1.0 });
            } else {
                if on(Hit::Delete(i)) {
                    scene.fills.push(Fill::solid(del, 6.0, Ink::Hover));
                }
                text(scene, "\u{E74D}", &fonts.icon_small, del, if on(Hit::Delete(i)) { Ink::Text } else { Ink::Muted });
            }
            scene.hits.push((del, Target::Share(Hit::Delete(i))));
        }
        y += ROW;
    }
    if a < 1.0 {
        scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= a);
        scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= a);
    }
    Ok(())
}

fn short_url(url: &str) -> String {
    short(url).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_791_000_000;
    const DAY: u64 = 86_400;

    fn link(id: &str, note: &str, created: u64, days: u64) -> Link {
        Link { id: id.into(), created, expires: NOW + days * DAY - 60, title: format!("Note {id}"), note: note.into() }
    }

    fn open(card: &Card, width: f32, height: f32, hover: Option<Target>) -> Scene {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let mut scene = Scene::default();
        let send = Rect::new(width - 230.0, 5.0, width - 184.0, 37.0);
        scene.hits.push((send, Target::SendTo));
        draw(&eng, &fonts, &ShareView { card, alpha: 1.0, now: NOW }, hover, width, height, &mut scene).unwrap();
        scene
    }

    fn rect(scene: &Scene, hit: Hit) -> Option<Rect> {
        scene.hits.iter().find(|(_, t)| *t == Target::Share(hit)).map(|(r, _)| *r)
    }

    #[test]
    fn the_share_card_walks_from_choose_to_ready() {
        let mut card = Card::new(Expiry::Week, "Trips/Coast.md", &[link("Old1", "Other.md", NOW - 9 * DAY, 2)], NOW);
        assert_eq!(card.phase, Phase::Choose);
        let s = open(&card, 1200.0, 800.0, None);
        assert!(rect(&s, Hit::Expiry(Expiry::Day)).is_some() && rect(&s, Hit::Create).is_some(), "the pills and Create");
        assert!(!s.fills.iter().any(|f| f.ink == Ink::Danger), "nothing red");
        // Chosen, kept: 30 days.
        assert_eq!(card.click(Hit::Expiry(Expiry::Month)), Command::Nothing);
        assert_eq!(card.expiry, Expiry::Month);
        // Create: the note goes up; a second press does nothing.
        assert_eq!(card.click(Hit::Create), Command::Create(Expiry::Month));
        assert_eq!(card.phase, Phase::Creating);
        assert_eq!(card.click(Hit::Create), Command::Nothing);
        assert_eq!(card.click(Hit::Expiry(Expiry::Day)), Command::Nothing);
        assert_eq!(card.expiry, Expiry::Month, "not while it is being made");
        // Made: its address shows, already copied, and it heads the list.
        card.created(Link { id: "Y5tq".into(), created: NOW, expires: NOW + 30 * DAY, title: "Coast".into(), note: "Trips/Coast.md".into() }, NOW);
        assert_eq!(card.phase, Phase::Ready { url: "https://notsy.co/Y5tq".into() });
        assert_eq!(card.copied, Some(Copied::New));
        assert_eq!(card.links.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["Y5tq", "Old1"], "this note's first");
        let s = open(&card, 1200.0, 800.0, None);
        assert!(rect(&s, Hit::Create).is_none() && rect(&s, Hit::CopyNew).is_some());
        let big = s.texts.iter().map(|t| t.layout.measure().height).fold(0.0, f32::max);
        assert!(big >= 20.0, "the address large");
        assert_eq!(card.click(Hit::CopyNew), Command::Copy("https://notsy.co/Y5tq".into()));
        assert_eq!(card.click(Hit::Copy(1)), Command::Copy("https://notsy.co/Old1".into()));
        assert_eq!(card.copied, Some(Copied::Link("Old1".into())));
        assert_eq!(card.click(Hit::Shade), Command::Close);
        // A failure says why, in the card, and Create tries again.
        let mut card = Card::new(Expiry::Week, "A.md", &[], NOW);
        card.click(Hit::Create);
        card.failed(&Problem::Offline);
        assert_eq!(card.message().as_deref(), Some("Couldn't reach notsy.co. Check the connection and try again."));
        assert_eq!(card.click(Hit::Create), Command::Create(Expiry::Week));
        assert_eq!(left_words(NOW + 6 * DAY + 3600, NOW), "6 days left");
        assert_eq!(left_words(NOW + DAY + 5, NOW), "1 day left");
        assert_eq!(left_words(NOW + 5 * 3600, NOW), "5 hours left");
        assert_eq!(left_words(NOW + 60, NOW), "Under an hour left");
    }

    #[test]
    fn three_live_links_disable_create_and_list_them() {
        let links = [link("Aaaa", "A.md", NOW - 3 * DAY, 4), link("Bbbb", "Here.md", NOW - 5 * DAY, 2), link("Cccc", "C.md", NOW - DAY, 6), link("Gone", "D.md", NOW - 30 * DAY, 0)];
        let mut card = Card::new(Expiry::Week, "Here.md", &links, NOW);
        assert_eq!(card.links.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["Bbbb", "Cccc", "Aaaa"], "live only: this note's, then the newest");
        assert!(card.at_limit() && !card.can_create());
        assert_eq!(card.message().as_deref(), Some("3 links are live. Delete one to share another."));
        assert_eq!(card.click(Hit::Create), Command::Nothing);
        let s = open(&card, 1200.0, 800.0, Some(Target::Share(Hit::Create)));
        for i in 0..3 {
            assert!(rect(&s, Hit::Copy(i)).is_some() && rect(&s, Hit::Delete(i)).is_some(), "row {i}'s buttons");
        }
        assert!(!s.fills.iter().any(|f| f.ink == Ink::Accent), "Create greyed");
        // Delete asks on the button, then the second click deletes.
        assert_eq!(card.click(Hit::Delete(1)), Command::Nothing);
        assert_eq!(card.confirming.as_deref(), Some("Cccc"));
        let asking = open(&card, 1200.0, 800.0, None);
        assert!(rect(&asking, Hit::Delete(1)).unwrap().width() > rect(&s, Hit::Delete(1)).unwrap().width(), "it widens to ask");
        assert!(!asking.fills.iter().any(|f| f.ink == Ink::Danger), "nothing red");
        // A click elsewhere lets the question go.
        card.click(Hit::Card);
        assert_eq!(card.confirming, None);
        card.click(Hit::Delete(1));
        assert_eq!(card.click(Hit::Delete(1)), Command::Delete("Cccc".into()));
        assert!(rect(&open(&card, 1200.0, 800.0, None), Hit::Delete(1)).is_none(), "no buttons while it goes");
        assert_eq!(card.click(Hit::Delete(1)), Command::Nothing);
        card.deleted("Cccc", Ok(()), NOW);
        assert_eq!(card.links.len(), 2);
        assert!(card.can_create() && card.message().is_none());
        assert_eq!(card.click(Hit::Create), Command::Create(Expiry::Week));
        // The server's list replaces what was kept.
        card.listed(&links[..1], NOW);
        assert_eq!(card.links.len(), 1);
        // A delete that fails says so, and the link stays.
        let mut card = Card::new(Expiry::Week, "Here.md", &links, NOW);
        card.click(Hit::Delete(0));
        card.click(Hit::Delete(0));
        card.deleted("Bbbb", Err(Problem::Offline), NOW);
        assert_eq!(card.links.len(), 3);
        assert!(matches!(card.phase, Phase::Failed(_)));
    }

    #[test]
    fn the_card_stays_in_small_windows() {
        let links = [link("Aaaa", "A.md", NOW - 3 * DAY, 4), link("Bbbb", "Here.md", NOW - 5 * DAY, 2), link("Cccc", "C.md", NOW - DAY, 6)];
        let card = Card::new(Expiry::Week, "Here.md", &links, NOW);
        for (w, h) in [(360.0, 320.0), (420.0, 240.0), (1200.0, 800.0)] {
            let s = open(&card, w, h, None);
            let c = rect(&s, Hit::Card).unwrap();
            assert!(c.left >= 0.0 && c.right <= w && c.top >= 0.0 && c.bottom <= h, "{w}x{h}: {c:?}");
            assert!(rect(&s, Hit::Create).is_some_and(|b| b.bottom <= c.bottom), "{w}x{h}: Create in reach");
        }
    }
}
