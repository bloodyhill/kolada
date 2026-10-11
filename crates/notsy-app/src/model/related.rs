//! Magnetic memory, as on Windows (`notsy-win` `app.rs`): the notes related
//! to what is being written drift into the margin beside the caret's line,
//! follow it, and fade out when they no longer relate. Resting on one peeks
//! at its note; a click opens it over the page; dragged into the text it
//! becomes a link there. A note row carried from the file tree links the
//! same way, or, let go outside the window, opens in a window of its own.

use std::time::{Duration, Instant};

use notsy_core::related::{Corpus, Kind, Related};
use notsy_markdown::{self as md, Block};
use notsy_ui::scene::{CHIP_GAP, CHIP_HEIGHT, Chip, Peek};

use super::share::Effect;
use super::*;

/// Related notes shown at most.
const CHIP_LIMIT: usize = 4;
/// A pause this long in writing, or a move to another paragraph, finds the
/// related notes again.
const RELATED_DELAY: Duration = Duration::from_millis(350);
/// Resting on a chip this long peeks at its note; the peek fades in over this.
const PEEK_DELAY: Duration = Duration::from_millis(280);
const PEEK_FADE: Duration = Duration::from_millis(130);
/// A press moved this far is a drag, DIPs.
const DRAG_SLOP: f32 = 4.0;

/// A related note in the margin, and its springs.
pub(super) struct ChipState {
    rel: Related,
    /// Top edge (NaN until first placed) and its velocity; how far it still
    /// has to drift in, and that velocity; opacity.
    y: f32,
    vy: f32,
    dx: f32,
    vdx: f32,
    alpha: f32,
    /// No longer related: fading out where it is.
    leaving: bool,
}

pub(super) struct PeekState {
    chip: usize,
    since: Instant,
    title: String,
    body: String,
}

#[derive(Default)]
pub(super) struct Margin {
    /// What every note is about, built with the index.
    pub corpus: std::sync::Arc<std::sync::Mutex<Option<Corpus>>>,
    chips: Vec<ChipState>,
    moving: bool,
    /// The note version, paragraph and note the margin was found for, and
    /// when to look again.
    key: (u64, usize, String),
    due: Option<Instant>,
    /// A peek, and the chip waiting to be peeked at, since when.
    peek: Option<PeekState>,
    peek_pending: Option<(usize, Instant)>,
    /// A chip pressed (and where), and the pointer while it is dragged.
    chip_press: Option<(usize, f32, f32)>,
    chip_drag: Option<(f32, f32)>,
    /// A note row pressed in the file tree (and where), and the pointer
    /// while it is carried.
    row_press: Option<(usize, f32, f32)>,
    row_drag: Option<(f32, f32)>,
    /// Where in the text a dragged chip or row would land as a link.
    drop_at: Option<usize>,
}

/// The margin as the scene takes it: the chips, the one dragged and where,
/// where it would drop in the text, and the peek.
pub(super) type MarginView<'a> = (Vec<Chip<'a>>, Option<(usize, f32, f32)>, Option<usize>, Option<Peek<'a>>);

/// Moves `x` toward `target` as a lightly damped spring; false once there.
fn drift(x: &mut f32, v: &mut f32, target: f32, dt: f32) -> bool {
    const OMEGA: f32 = 11.0;
    const ZETA: f32 = 0.82;
    let a = OMEGA * OMEGA * (target - *x) - 2.0 * ZETA * OMEGA * *v;
    *v += a * dt;
    *x += *v * dt;
    if (target - *x).abs() < 0.1 && v.abs() < 2.0 {
        *x = target;
        *v = 0.0;
        return false;
    }
    true
}

/// A note's opening, as a peek shows it: its first lines as they read,
/// without frontmatter or Markdown marks.
fn opening(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for l in md::lines(text).filter(|l| !matches!(l.block, Block::Frontmatter | Block::Fence)) {
        let plain = md::plain(&text[l.start..l.end]);
        let plain = plain.trim();
        if plain.is_empty() {
            continue;
        }
        let cut: String = plain.chars().take(160).collect();
        out.push(if plain.chars().count() > 160 { format!("{cut}\u{2026}") } else { cut });
        if out.len() == 7 {
            break;
        }
    }
    out.join("\n")
}

impl Model {
    /// The margin is shown: the option, a full window, a page that is written.
    fn margin_on(&self) -> bool {
        self.session.related == 0 && !self.session.minimal && !self.compact && !self.settings.read_only
    }

    /// The text or the caret's paragraph changed: the margin is found again
    /// after a pause.
    pub(super) fn related_soon(&mut self) {
        let head = self.editor.selection().head;
        let key = (self.editor.version(), self.editor.line_at(head.min(self.editor.text().len())).start, self.session.note.clone());
        if key != self.margin.key {
            self.margin.key = key;
            self.margin.due = Some(Instant::now() + RELATED_DELAY);
        }
    }

    /// Finds the notes related to what is being written (most to the
    /// paragraph at the caret and its neighbours). Chips still related stay
    /// and move to their new places, new ones drift in, the rest fade out.
    pub fn update_related(&mut self) -> bool {
        self.margin.due = None;
        let found = {
            let corpus = self.margin.corpus.lock().unwrap();
            let e = &self.editor;
            match corpus.as_ref() {
                Some(c) if !c.is_empty() && !e.text().trim().is_empty() && self.margin_on() => {
                    let head = e.selection().head;
                    let lines = e.outline();
                    let i = lines.partition_point(|l| l.start <= head).max(1) - 1;
                    let from = lines[i.saturating_sub(1)].start;
                    let to = lines[(i + 1).min(lines.len() - 1)].end;
                    c.related(e.text(), &e.text()[from..to], &self.session.note, CHIP_LIMIT)
                }
                _ => Vec::new(),
            }
        };
        let mut next = Vec::with_capacity(found.len() + self.margin.chips.len());
        for rel in found {
            let key = rel.key();
            match self.margin.chips.iter().position(|c| c.rel.key() == key) {
                Some(i) => {
                    let mut chip = self.margin.chips.remove(i);
                    chip.rel = rel;
                    chip.leaving = false;
                    next.push(chip);
                }
                None => next.push(ChipState { rel, y: f32::NAN, vy: 0.0, dx: 28.0, vdx: 0.0, alpha: if self.ui.animate { 0.0 } else { 1.0 }, leaving: false }),
            }
        }
        for mut chip in self.margin.chips.drain(..) {
            chip.leaving = true;
            next.push(chip);
        }
        self.margin.chips = next;
        self.margin.moving = true;
        // A peek belongs to a chip that may have moved.
        self.margin.peek = None;
        if !self.ui.animate {
            // Tests and dumps: where they go, at once.
            self.step_chips(10.0);
        }
        true
    }

    /// The margin moves, or waits on the clock.
    pub(super) fn chips_moving(&self) -> bool {
        let m = &self.margin;
        m.moving || m.peek.as_ref().is_some_and(|p| p.since.elapsed() < PEEK_FADE + Duration::from_millis(20))
    }

    /// When the margin is next due to be found, or a chip peeked at.
    pub(super) fn chips_due(&self) -> Option<Instant> {
        let m = &self.margin;
        [m.due, m.peek_pending.map(|(_, t)| t + PEEK_DELAY)].into_iter().flatten().min()
    }

    /// A frame of the margin: a due search, a due peek, the chips' springs.
    pub(super) fn step_chips(&mut self, dt: f32) -> bool {
        let mut changed = false;
        if self.margin.due.is_some_and(|t| Instant::now() >= t) {
            changed |= self.update_related();
        }
        if let Some((i, since)) = self.margin.peek_pending
            && since.elapsed() >= PEEK_DELAY
        {
            self.margin.peek_pending = None;
            changed |= self.show_peek(i);
        }
        if self.margin.peek.as_ref().is_some_and(|p| p.since.elapsed() < PEEK_FADE + Duration::from_millis(20)) {
            changed = true;
        }
        if !self.margin.moving {
            return changed;
        }
        let anchor = self.ui.caret_target.map_or(self.ui.column.top, |c| c.top);
        let bottom = self.viewport.1 - 16.0;
        let shown = self.margin.chips.iter().filter(|c| !c.leaving).count() as f32;
        let stack = shown * (CHIP_HEIGHT + CHIP_GAP) - CHIP_GAP;
        let top = (anchor - 14.0).min(bottom - stack).max(self.ui.column.top);
        let mut moving = false;
        let mut k = 0.0;
        // Without animation (tests, dumps) everything is where it goes at once.
        let instant = !self.ui.animate;
        for c in &mut self.margin.chips {
            if !c.leaving {
                let target = top + k * (CHIP_HEIGHT + CHIP_GAP);
                k += 1.0;
                if c.y.is_nan() || instant {
                    c.y = target;
                }
                moving |= drift(&mut c.y, &mut c.vy, target, dt);
            }
            let dx = if c.leaving { 16.0 } else { 0.0 };
            if instant {
                c.dx = dx;
            }
            moving |= drift(&mut c.dx, &mut c.vdx, dx, dt);
            let goal = if c.leaving { 0.0 } else { 1.0 };
            c.alpha = if instant { goal } else if c.alpha < goal { (c.alpha + dt * 5.0).min(goal) } else { (c.alpha - dt * 5.0).max(goal) };
            moving |= c.alpha != goal;
        }
        self.margin.chips.retain(|c| !(c.leaving && c.alpha <= 0.0));
        self.margin.moving = moving && !instant;
        true
    }

    /// The margin as the scene draws it: the chips, one dragged, where it
    /// would drop, the peek.
    pub(super) fn margin_view(&self) -> MarginView<'_> {
        let chips = self
            .margin
            .chips
            .iter()
            .filter(|c| !c.y.is_nan())
            .map(|c| Chip { name: &c.rel.name, detail: &c.rel.detail, kind: c.rel.kind, strength: (c.rel.score * 2.0).min(1.0), y: c.y, dx: c.dx, alpha: c.alpha })
            .collect();
        let dragged = match (self.margin.chip_press, self.margin.chip_drag) {
            (Some((i, _, _)), Some((x, y))) => Some((i, x, y)),
            _ => None,
        };
        let peek = self.margin.peek.as_ref().filter(|_| dragged.is_none()).map(|p| Peek {
            chip: p.chip,
            title: &p.title,
            body: &p.body,
            alpha: if self.ui.animate { chrome::ease(p.since.elapsed().as_secs_f32() / PEEK_FADE.as_secs_f32()) } else { 1.0 },
        });
        (chips, dragged, self.margin.drop_at, peek)
    }

    /// A note row being carried, as the scene draws it under the pointer.
    pub(super) fn carried(&self) -> Option<(&str, f32, f32)> {
        match (self.margin.row_press, self.margin.row_drag) {
            (Some((i, _, _)), Some((x, y))) => self.session.sidebar.get(i).map(|r| (r.name.as_str(), x, y)),
            _ => None,
        }
    }

    /// What every note is about is read (for a dump).
    pub fn corpus_ready(&self) -> bool {
        self.margin.corpus.lock().unwrap().is_some()
    }

    /// The pointer rests on the first chip and it is peeked at (for a dump).
    pub fn peek_first(&mut self) {
        if let Some((r, _)) = self.hits.iter().find(|(_, t)| *t == Target::Chip(0)).copied() {
            self.pointer_moved(r.left + 10.0, r.top + 10.0);
            self.show_peek(0);
        }
    }

    /// The chip the pointer is known to rest on (peeked or waiting to be).
    pub(super) fn margin_hovered(&self) -> Option<usize> {
        self.margin.peek.as_ref().map(|p| p.chip).or(self.margin.peek_pending.map(|(i, _)| i))
    }

    /// The button let go at (x, y), with what it ends: a drag, a carried
    /// row or chip, a selection. What the view must do outside comes back.
    pub fn pointer_up_at(&mut self, x: f32, y: f32) -> (bool, Vec<Effect>) {
        if let Some(done) = self.margin_up(x, y) {
            return done;
        }
        if let Some(done) = self.popup_dropped(x, y) {
            return done;
        }
        (self.pointer_up(), Vec::new())
    }

    /// The pointer rests on chip `i` (or on none): after a moment, a peek.
    pub(super) fn hover_chip(&mut self, chip: Option<usize>) {
        if self.margin.peek.as_ref().map(|p| p.chip) == chip && chip.is_some() {
            return;
        }
        self.margin.peek = None;
        self.margin.peek_pending = chip.map(|i| (i, Instant::now()));
    }

    fn show_peek(&mut self, i: usize) -> bool {
        if self.hover != Some(Target::Chip(i)) || self.margin.chip_drag.is_some() {
            return false;
        }
        let Some(chip) = self.margin.chips.get(i) else { return false };
        let body = {
            let index = self.index.lock().unwrap();
            let Some(note) = index.as_ref().and_then(|all| all.iter().find(|n| n.path == chip.rel.path)) else { return false };
            opening(note.text())
        };
        self.margin.peek = Some(PeekState { chip: i, since: Instant::now(), title: chip.rel.name.clone(), body });
        true
    }

    /// A chip clicked: its note opens over the page.
    fn open_chip(&mut self, i: usize) -> bool {
        let Some(chip) = self.margin.chips.get(i) else { return false };
        let (rel, passage) = (chip.rel.path.clone(), (chip.rel.kind == Kind::Highlight).then(|| chip.rel.detail.clone()));
        if !self.open_over_page(&rel) {
            return false;
        }
        // A marked passage's chip opens at the passage.
        if let (Some(p), Some(popup)) = (passage, self.popup_editor_mut())
            && let Some(at) = popup.text().find(&p)
        {
            popup.set_selection(Selection::caret(at));
        }
        true
    }

    /// A `[[link]]` to the note `name` at `at`, spaced from the words around it.
    fn insert_link(&mut self, name: &str, at: usize) -> bool {
        if self.settings.read_only {
            return false;
        }
        let text = self.editor.text();
        let mut at = at.min(text.len());
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        let mut link = String::new();
        if text[..at].chars().next_back().is_some_and(|c| !c.is_whitespace()) {
            link.push(' ');
        }
        link.push_str(&format!("[[{name}]]"));
        if text[at..].chars().next().is_some_and(|c| !c.is_whitespace()) {
            link.push(' ');
        }
        self.editor.set_selection(Selection::caret(at));
        self.edit(|e| e.insert(&link))
    }

    /// The button went down on a chip, or on a note row (which may be carried).
    pub(super) fn margin_down(&mut self, target: Option<Target>, x: f32, y: f32) -> Option<bool> {
        match target? {
            Target::Chip(i) => {
                self.margin.chip_press = Some((i, x, y));
                Some(true)
            }
            Target::Row(i) if self.session.sidebar.get(i).is_some_and(|r| r.kind == RowKind::Note) && self.search.is_none() && self.trash.is_none() => {
                // A click, unless it moves: the row's note opens on release.
                self.margin.row_press = Some((i, x, y));
                Some(false)
            }
            _ => None,
        }
    }

    /// The pointer moved with the button held: a chip or a row that has
    /// moved past the slop is dragged toward the text. None if neither.
    pub(super) fn margin_dragged(&mut self, x: f32, y: f32) -> Option<bool> {
        let moved = |(px, py): (f32, f32)| (x - px).abs() + (y - py).abs() > DRAG_SLOP;
        let over_text = scene::hit(&self.hits, x, y) == Some(Target::Note);
        if let Some((_, px, py)) = self.margin.chip_press {
            if self.margin.chip_drag.is_some() || moved((px, py)) {
                self.margin.chip_drag = Some((x, y));
                self.margin.peek = None;
                self.margin.drop_at = over_text.then(|| self.offset_at(x, y));
            }
            return Some(true);
        }
        if let Some((_, px, py)) = self.margin.row_press {
            if self.margin.row_drag.is_some() || moved((px, py)) {
                self.margin.row_drag = Some((x, y));
                self.margin.drop_at = over_text.then(|| self.offset_at(x, y));
            }
            return Some(true);
        }
        None
    }

    /// The button let go at (x, y): a chip opens (clicked) or links where it
    /// was dropped; a row opens (clicked), links where it was dropped, or
    /// outside the window opens in a window of its own. None if neither.
    pub(super) fn margin_up(&mut self, x: f32, y: f32) -> Option<(bool, Vec<Effect>)> {
        if let Some((i, _, _)) = self.margin.chip_press.take() {
            let dragged = self.margin.chip_drag.take().is_some();
            let changed = match self.margin.drop_at.take() {
                Some(at) if dragged => {
                    let name = self.margin.chips.get(i).map(|c| c.rel.name.clone());
                    name.is_some_and(|n| self.insert_link(&n, at))
                }
                _ if !dragged => self.open_chip(i),
                _ => true,
            };
            return Some((changed, Vec::new()));
        }
        let (i, _, _) = self.margin.row_press.take()?;
        let dragged = self.margin.row_drag.take().is_some();
        let drop = self.margin.drop_at.take();
        let Some(row) = self.session.sidebar.get(i).cloned() else { return Some((false, Vec::new())) };
        let rel = format!("{}.md", vault::row_paths(&self.session.sidebar)[i]);
        if !dragged {
            return Some((self.click_row(i), Vec::new()));
        }
        if let Some(at) = drop {
            let changed = rel != self.session.note && self.insert_link(&row.name, at);
            return Some((changed, Vec::new()));
        }
        let (w, h) = self.viewport;
        if x < 0.0 || y < 0.0 || x >= w || y >= h {
            self.save_flush();
            return Some((true, vec![Effect::Window(self.vault.join(rel))]));
        }
        Some((true, Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(test: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("notsy-app-related-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Garden.md"), "# Garden\n\nTomatoes need sun and water. Basil grows beside the tomatoes in the garden bed.\n").unwrap();
        std::fs::write(dir.join("Taxes.md"), "# Taxes\n\nReceipts and forms for the accountant, due in April.\n").unwrap();
        std::fs::write(dir.join("Today.md"), "# Today\n\nPlant more tomatoes and basil in the garden bed this week.\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.open("Today.md");
        // What every note is about, read as the app does on a thread.
        m.start_index();
        for _ in 0..400 {
            if m.margin.corpus.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        m.go(Motion::DocEnd, false);
        m.frame(2400, 1440, 2.0);
        (m, dir)
    }

    #[test]
    fn notes_about_the_same_thing_drift_into_the_margin_and_a_click_opens_one_over_the_page() {
        let (mut m, dir) = model("chips");
        assert!(m.update_related());
        m.frame(2400, 1440, 2.0);
        let names: Vec<String> = m.margin.chips.iter().map(|c| c.rel.name.clone()).collect();
        assert_eq!(names.first().map(String::as_str), Some("Garden"), "{names:?}");
        assert!(!names.contains(&"Taxes".to_string()), "unrelated notes stay out: {names:?}");
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::Chip(0)).expect("the chip is in the margin");
        assert!(m.pointer_down(r.left + 10.0, r.top + 10.0, 1, false));
        m.pointer_up_at(r.left + 10.0, r.top + 10.0);
        assert_eq!(m.popup_note(), Some("Garden.md"), "its note opens over the page");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_chip_dragged_into_the_text_becomes_a_link_there() {
        let (mut m, dir) = model("drag");
        m.update_related();
        m.frame(2400, 1440, 2.0);
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::Chip(0)).unwrap();
        let (n, _) = *m.hits.iter().find(|(_, t)| *t == Target::Note).unwrap();
        m.pointer_down(r.left + 10.0, r.top + 10.0, 1, false);
        let line = m.lines.iter().find(|l| m.text()[l.para.start..].starts_with("Plant")).unwrap();
        let (x, y) = (line.x + 2.0, line.y + 4.0);
        assert!(n.left < x);
        m.pointer_dragged(x, y);
        assert!(m.margin.drop_at.is_some(), "it would land in the text");
        m.pointer_up_at(x, y);
        assert!(m.text().contains("[[Garden]] Plant"), "{:?}", m.text());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_note_row_carried_into_the_text_links_it_and_a_click_still_opens_it() {
        let (mut m, dir) = model("row");
        let row = |m: &Model, name: &str| m.hits.iter().find(|(_, t)| matches!(t, Target::Row(i) if m.session.sidebar[*i].name == name)).map(|(r, _)| *r).unwrap();
        let r = row(&m, "Taxes");
        let line = m.lines.iter().find(|l| m.text()[l.para.start..].starts_with("Plant")).unwrap();
        let (x, y) = (line.x + 2.0, line.y + 4.0);
        m.pointer_down(r.left + 20.0, r.top + 5.0, 1, false);
        m.pointer_dragged(x, y);
        m.frame(2400, 1440, 2.0);
        m.pointer_up_at(x, y);
        assert!(m.text().contains("[[Taxes]] Plant"), "{:?}", m.text());
        assert_eq!(m.session.note, "Today.md");
        // Without moving: a click, the note opens.
        let r = row(&m, "Taxes");
        m.pointer_down(r.left + 20.0, r.top + 5.0, 1, false);
        m.pointer_up_at(r.left + 20.0, r.top + 5.0);
        assert_eq!(m.session.note, "Taxes.md");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_margin_is_off_when_the_option_says_so() {
        let (mut m, dir) = model("off");
        m.choose(notsy_ui::scene::Setting::Related, 1);
        m.update_related();
        assert!(m.margin.chips.iter().all(|c| c.leaving));
        let _ = std::fs::remove_dir_all(dir);
    }
}
