//! What moves between frames, as on Windows: a wheel's scroll eases in,
//! the caret glides to where it goes and blinks for a while after, a note
//! just opened fades in, and the scroll bar shows for a moment after
//! scrolling. The view asks
//! `wants_frames` after each event and, while it is true, calls `tick` once
//! a display refresh.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use kolada_ui::blink::Blink;

use super::*;

/// A scroll eases in: what is left of it shrinks at this rate per second.
const SCROLL_RATE: f32 = 18.0;
/// Stiffness of the caret's glide after a click (rad/s), critically
/// damped: it arrives within 80 ms and never overshoots.
pub(super) const CARET_OMEGA: f32 = 140.0;
/// A note that opens fades in over this long.
pub(super) const PAGE_FADE: Duration = Duration::from_millis(170);
/// A passing word ("Copied the note") shows this long, fading in and out at
/// its ends.
const WHISPER: Duration = Duration::from_millis(2600);
const WHISPER_IN: Duration = Duration::from_millis(160);
const WHISPER_OUT: Duration = Duration::from_millis(550);

/// The window's state that is not kept between launches.
#[derive(Default)]
pub struct Ui {
    /// Things fade and glide (the app; not tests or dumps, whose frames
    /// must come out the same each time).
    pub animate: bool,
    /// Where the pointer is, DIPs (none outside the window).
    pub pointer: Option<(f32, f32)>,
    /// What the button last went down on: a drag selects text only from the page.
    pub pressed: Option<Target>,
    /// The note column in the last frame.
    pub column: Rect,
    /// When the page last scrolled: its scroll bar shows for a moment after.
    pub scrolled_at: Option<Instant>,
    /// A scroll bar being dragged.
    pub scroll_drag: Option<super::bars::Drag>,
    /// Scroll distance still to travel, DIPs (positive is down).
    pub scroll_pending: f32,
    /// Estimated heights for the scroll bar (see `bars`).
    pub heights: RefCell<super::bars::Heights>,
    /// The first file tree row shown; wheel movement over the tree not yet a
    /// whole row; how many rows fit, and the tree's scroll bar track, in the
    /// last frame.
    pub tree_top: usize,
    pub tree_scroll: f32,
    pub tree_rows: usize,
    pub tree_track: Option<Rect>,
    /// Search results that fit, for keeping the chosen one in view.
    pub result_rows: usize,
    /// The caret glides: where it belongs in the last frame, how far from
    /// there it was drawn, and how fast that gap closes; it moved since the
    /// last frame, and glides there if `glide`.
    pub caret_target: Option<Rect>,
    pub caret_offset: (f32, f32),
    pub caret_velocity: (f32, f32),
    pub caret_gliding: bool,
    pub caret_jumped: bool,
    pub glide: bool,
    /// The caret blinks for a while after it moves, while the window has
    /// the keys (see `kolada_ui::blink`); whether the last frame left it out.
    pub blink: Blink,
    pub caret_hidden: bool,
    /// When the note on screen was opened, while it fades in.
    pub page_opened: Option<Instant>,
    /// When the last frame stepped (for how far things move).
    pub last_tick: Option<Instant>,
    /// A passing word at the top of the page, and since when.
    pub whisper: Option<(String, Instant)>,
    /// When the file tree row under the pointer was written.
    pub row_date: Option<(usize, String)>,
    /// How long this launch took to its first frame, and what the last keys
    /// cost from their event to their frame (ms), for the settings' footer.
    pub opened_in: Option<f32>,
    pub key_ms: Vec<f32>,
    /// The window is maximized (where the chrome draws its own buttons).
    pub maximized: bool,
}

/// Closes `x` toward 0 with velocity `v` over `dt` seconds, critically damped
/// at `omega`; false once it is there.
pub fn settle(x: &mut f32, v: &mut f32, dt: f32, omega: f32) -> bool {
    let decay = (-omega * dt).exp();
    let k = *v + omega * *x;
    *x = (*x + k * dt) * decay;
    *v = (*v - omega * k * dt) * decay;
    // A quarter DIP is under a pixel at any scale: stop there rather than
    // drawing frames nobody can see.
    if x.abs() < 0.25 && v.abs() < 5.0 {
        *x = 0.0;
        *v = 0.0;
        return false;
    }
    true
}

impl Model {
    /// Something is moving: frames should keep coming.
    pub fn wants_frames(&self) -> bool {
        let ui = &self.ui;
        ui.scroll_pending != 0.0
            || ui.caret_gliding
            || ui.page_opened.is_some()
            // The scroll bar and a passing word move only as they fade.
            || ui.scrolled_at.is_some_and(|t| (super::bars::SHOWN..super::bars::SHOWN + super::bars::FADE + Duration::from_millis(20)).contains(&t.elapsed()))
            || self.past.moving
            || self.pictures_fading()
            || self.ui.whisper.as_ref().is_some_and(|(_, t)| {
                let e = t.elapsed();
                e < WHISPER_IN + Duration::from_millis(20) || e > WHISPER - WHISPER_OUT
            })
            || self.popup_fading()
            || self.panels_moving()
            || self.chips_moving()
            || self.arrival_moving()
            || self.assist_moving()
            || self.agenda_moving()
            || self.share_fading()
            || self.formatting_moving()
            || self.model_card_moving()
            || self.menu_fading()
            || self.beam_check_moving()
    }

    /// When something next has to happen by the clock while nothing moves
    /// (a reveal, a pause in typing, a scroll bar or a passing word starting
    /// to fade): the view wakes once then, not every frame until then.
    pub fn next_due(&self) -> Option<Instant> {
        let ui = &self.ui;
        let bar = ui.scrolled_at.map(|t| t + super::bars::SHOWN).filter(|t| *t > Instant::now());
        let whisper = ui.whisper.as_ref().map(|(_, t)| *t + WHISPER - WHISPER_OUT);
        [bar, whisper, self.blink_due(), self.panels_due(), self.chips_due(), self.assist_due(), self.formatting_due(), self.link_card_due(), self.model_card_due(), self.remind_due()].into_iter().flatten().min()
    }

    /// Steps everything that moves by the time since the last step; whether
    /// the screen changes.
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let dt = self.ui.last_tick.replace(now).map_or(1.0 / 60.0, |t| now.duration_since(t).as_secs_f32());
        // After an idle stretch, step as if one refresh passed rather than jump.
        let dt = if dt > 0.1 { 1.0 / 60.0 } else { dt.min(1.0 / 30.0) };
        let mut changed = false;
        if self.ui.scroll_pending != 0.0 {
            let rest = self.ui.scroll_pending;
            let step = if rest.abs() < 0.5 { rest } else { rest * (1.0 - (-SCROLL_RATE * dt).exp()) };
            // Stop at the top or the end instead of pushing against it.
            self.ui.scroll_pending = if self.scroll_by(step) { rest - step } else { 0.0 };
            changed = true;
        }
        if self.ui.caret_gliding {
            let (ox, oy) = &mut self.ui.caret_offset;
            let (vx, vy) = &mut self.ui.caret_velocity;
            let moving_x = settle(ox, vx, dt, CARET_OMEGA);
            let moving_y = settle(oy, vy, dt, CARET_OMEGA);
            self.ui.caret_gliding = moving_x || moving_y;
            changed = true;
        }
        if let Some(t) = self.ui.page_opened {
            if t.elapsed() >= PAGE_FADE {
                self.ui.page_opened = None;
            }
            changed = true;
        }
        if let Some(t) = self.ui.scrolled_at {
            if t.elapsed() >= super::bars::SHOWN + super::bars::FADE + Duration::from_millis(20) {
                self.ui.scrolled_at = None;
            }
            changed = true;
        }
        if self.past.moving {
            if !self.scrub_step(dt) {
                self.past.moving = false;
            }
            changed = true;
        }
        changed |= self.pictures_fading() || self.popup_fading();
        changed |= self.step_panels(dt);
        changed |= self.step_formatting();
        changed |= self.step_link_card();
        self.step_remind();
        changed |= self.step_model_card() | self.model_card_moving();
        changed |= self.formatting_moving();
        changed |= self.step_chips(dt);
        changed |= self.step_arrival(dt);
        changed |= self.menu_fading();
        changed |= self.step_assist();
        if self.agenda_moving() {
            self.step_agenda(dt);
            changed = true;
        }
        if let Some((_, t)) = &self.ui.whisper {
            if t.elapsed() >= WHISPER {
                self.ui.whisper = None;
            }
            changed = true;
        }
        // The caret shows or hides: only its sliver is drawn again.
        changed |= self.caret_shown(now) == self.ui.caret_hidden;
        if !self.wants_frames() {
            self.ui.last_tick = None;
        }
        changed
    }

    /// Whether the caret is drawn at `now`: always, unless it blinks (only
    /// where things move: not in tests or dumps, nor with less motion asked).
    pub(super) fn caret_shown(&self, now: Instant) -> bool {
        !self.ui.animate || self.ui.blink.shown(now)
    }

    /// When the caret next shows or hides by itself; none once it rests.
    fn blink_due(&self) -> Option<Instant> {
        if self.ui.animate { self.ui.blink.next_due(Instant::now()) } else { None }
    }

    /// The window gained or lost the keys: its caret blinks only while it
    /// has them, and shows solid at once either way. Whether the screen
    /// changes; the view asks `next_due` again after.
    pub fn set_focused(&mut self, focused: bool) -> bool {
        let now = Instant::now();
        let before = self.caret_shown(now);
        self.ui.blink.set_focused(focused, now);
        self.caret_shown(now) != before
    }

    /// The OS's caret blink: shown for the first, hidden for the second
    /// (None: it never blinks, a setting some people choose). Half a second
    /// each until a shell says.
    pub fn set_caret_blink(&mut self, period: Option<(Duration, Duration)>) {
        self.ui.blink.set_period(period);
    }

    /// The caret moved; it glides there unless `glide` is false (typing,
    /// where it must stay with the letters, or dragging a selection).
    pub(super) fn caret_moved(&mut self, glide: bool) {
        self.ui.caret_jumped = true;
        // Shown at once wherever it lands, and blinking from now.
        self.ui.blink.restart(Instant::now());
        self.ui.glide = glide && self.ui.animate;
        self.update_suggestions();
        self.related_soon();
    }

    /// Where the caret is drawn this frame: from where it was, closing the gap.
    pub(super) fn glide_to(&mut self, target: Option<Rect>) {
        let ui = &mut self.ui;
        if std::mem::take(&mut ui.caret_jumped) {
            match (ui.caret_target, target) {
                (Some(old), Some(new)) if ui.glide && (old.left != new.left || old.top != new.top) => {
                    ui.caret_offset.0 += old.left - new.left;
                    ui.caret_offset.1 += old.top - new.top;
                    ui.caret_gliding = true;
                }
                (_, _) if ui.glide => {}
                _ => {
                    ui.caret_offset = (0.0, 0.0);
                    ui.caret_velocity = (0.0, 0.0);
                    ui.caret_gliding = false;
                }
            }
        }
        // Gone from the screen: nothing to glide from next time.
        if target.is_none() {
            ui.caret_offset = (0.0, 0.0);
            ui.caret_velocity = (0.0, 0.0);
            ui.caret_gliding = false;
        }
        ui.caret_target = target;
    }

    /// A note just opened fades in and rises into place.
    pub(super) fn fade_in_page(&mut self) {
        if self.ui.animate {
            self.ui.page_opened = Some(Instant::now());
        }
    }

    /// A word or two at the top of the page, that fades after a moment.
    pub fn whisper(&mut self, text: &str) {
        self.ui.whisper = Some((text.to_string(), Instant::now()));
    }

    /// The passing word as the scene draws it.
    pub(super) fn whisper_view(&self) -> Option<scene::Whisper<'_>> {
        self.ui.whisper.as_ref().map(|(text, since)| {
            let t = since.elapsed();
            let alpha = if t < WHISPER_IN {
                t.as_secs_f32() / WHISPER_IN.as_secs_f32()
            } else if t > WHISPER - WHISPER_OUT {
                WHISPER.saturating_sub(t).as_secs_f32() / WHISPER_OUT.as_secs_f32()
            } else {
                1.0
            };
            scene::Whisper { text, alpha }
        })
    }

    /// What the page says in passing, if anything.
    pub fn whispered(&self) -> Option<&str> {
        self.ui.whisper.as_ref().map(|(t, _)| t.as_str())
    }

    /// The launch drew its first frame this long after the process began.
    pub fn opened_in(&mut self, ms: Option<f64>) {
        self.ui.opened_in = ms.map(|m| m as f32);
    }

    /// A key's frame is drawn, `ms` after its event.
    pub fn key_cost(&mut self, ms: f32) {
        if self.ui.key_ms.len() >= 64 {
            self.ui.key_ms.remove(0);
        }
        self.ui.key_ms.push(ms);
    }

    /// The middle of what the last keystrokes cost, ms.
    pub(super) fn keystroke_median(&self) -> Option<f32> {
        if self.ui.key_ms.len() < 5 {
            return None;
        }
        let mut ms = self.ui.key_ms.clone();
        ms.sort_by(f32::total_cmp);
        Some(ms[ms.len() / 2])
    }

    /// How far the page has faded in, 0..1.
    pub(super) fn page_fade(&self) -> f32 {
        self.ui.page_opened.map_or(1.0, |t| chrome::ease(t.elapsed().as_secs_f32() / PAGE_FADE.as_secs_f32()))
    }
}

#[cfg(test)]
mod tests {
    use super::settle;

    #[test]
    fn caret_glide_closes_the_gap_without_overshoot_or_jitter() {
        let (mut x, mut v) = (-120.0_f32, 0.0_f32);
        let mut steps = 0;
        while settle(&mut x, &mut v, 1.0 / 60.0, super::CARET_OMEGA) {
            assert!(x <= 0.0, "never past where it goes: {x}");
            steps += 1;
            assert!(steps < 120, "and it gets there");
        }
        assert_eq!((x, v), (0.0, 0.0));
        assert!(steps * 1000 / 60 <= 80, "within 80 ms: {steps} frames");
    }

    fn note(name: &str) -> (std::path::PathBuf, crate::Model) {
        let dir = std::env::temp_dir().join(format!("kolada-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let long: String = (0..200).map(|i| format!("Line {i} of the note.\n")).collect();
        std::fs::write(dir.join("Note.md"), format!("# Note\n\n{long}")).unwrap();
        let mut m = crate::Model::new(dir.clone(), false);
        m.frame(1600, 1000, 1.0);
        m.ui.animate = true;
        (dir, m)
    }

    /// Every key's step, Page Up/Down too, is drawn where it lands in the
    /// very next frame, starts no motion, and leaves no frames due.
    #[test]
    fn a_keys_step_lands_at_once_and_starts_no_motion() {
        use kolada_editor::Motion;
        let (dir, mut m) = note("caret-lands");
        let moves: [&dyn Fn(&mut crate::Model) -> bool; 8] = [
            &|m| m.go(Motion::Right, false),
            &|m| m.go(Motion::Down, false),
            &|m| m.go(Motion::WordRight, false),
            &|m| m.go(Motion::Up, false),
            &|m| m.go(Motion::LineEnd, false),
            &|m| m.go(Motion::LineStart, false),
            &|m| m.page(true, false),
            &|m| m.page(false, false),
        ];
        for (i, step) in moves.iter().enumerate() {
            let before = m.caret_rect();
            assert!(step(&mut m));
            m.frame(1600, 1000, 1.0);
            assert_ne!(m.caret_rect(), before, "move {i} moves the caret");
            assert_eq!(m.ui.caret_offset, (0.0, 0.0), "move {i} is drawn where it lands");
            assert!(!m.ui.caret_gliding && !m.wants_frames(), "move {i} starts no motion");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Arrow-downs at key-repeat pace never fall behind: after each, the
    /// caret is drawn on its own line in that frame, and nothing is left
    /// to catch up, even when they start in the middle of a click's glide.
    #[test]
    fn held_arrow_keys_never_fall_behind() {
        use kolada_editor::Motion;
        let (dir, mut m) = note("caret-repeat");
        m.pointer_down(700.0, 600.0, 1, false);
        m.frame(1600, 1000, 1.0);
        for i in 0..30 {
            let before = m.editor.selection().head;
            assert!(m.go(Motion::Down, false));
            m.frame(1600, 1000, 1.0);
            assert!(m.editor.selection().head > before, "key {i} goes down a line");
            assert!(m.caret_rect().is_some(), "key {i} keeps the caret in view");
            assert_eq!(m.ui.caret_offset, (0.0, 0.0), "key {i} is drawn on its line at once");
            assert!(!m.ui.caret_gliding && !m.wants_frames(), "key {i} leaves nothing to catch up");
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The blink as if it began `ago` (the clock is not waited on).
    fn blinking_since(m: &mut crate::Model, ago: std::time::Duration) {
        m.ui.blink.since = Some(std::time::Instant::now().checked_sub(ago).expect("the clock goes back that far"));
    }

    fn ms(n: u64) -> std::time::Duration {
        std::time::Duration::from_millis(n)
    }

    /// A move to another line also asks the margin for related notes after
    /// a short pause; that wait is not the caret's, so it is done with.
    fn margin_done(m: &mut crate::Model) {
        m.update_related();
        for _ in 0..100 {
            if !m.wants_frames() {
                return;
            }
            m.tick();
        }
        panic!("the margin settles");
    }

    /// After a move the caret blinks, one wake per show or hide and no
    /// frames in between; ten seconds on it rests, solid, with nothing due.
    #[test]
    fn the_caret_blinks_then_rests_solid_with_nothing_due() {
        use kolada_editor::Motion;
        let (dir, mut m) = note("blink-rests");
        assert!(m.go(Motion::Right, false));
        margin_done(&mut m);
        m.frame(1600, 1000, 1.0);
        assert!(!m.ui.caret_hidden && !m.wants_frames(), "shown at once, and no frames while it blinks");
        let due = m.next_due().expect("its first hide is due");
        assert!(due > std::time::Instant::now() + ms(400), "half a second shown first");
        blinking_since(&mut m, ms(600));
        assert!(m.tick(), "the hide changes the screen");
        m.frame(1600, 1000, 1.0);
        assert!(m.ui.caret_hidden && m.caret_rect().is_some(), "left out of the frame, still where it is");
        assert!(m.next_due().is_some() && !m.wants_frames());
        blinking_since(&mut m, kolada_ui::blink::REST + ms(1));
        assert!(m.tick(), "the rest shows it again");
        m.frame(1600, 1000, 1.0);
        assert!(!m.ui.caret_hidden, "solid at rest");
        assert_eq!(m.next_due(), None, "nothing due: the window is idle");
        assert!(!m.wants_frames() && !m.tick(), "and draws nothing more");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A move, an edit or a click while the caret is hidden shows it in the
    /// very next frame and starts its blink over.
    #[test]
    fn a_move_or_an_edit_shows_the_caret_at_once_and_starts_the_blink_over() {
        use kolada_editor::Motion;
        let (dir, mut m) = note("blink-restart");
        let steps: [&dyn Fn(&mut crate::Model) -> bool; 3] = [
            &|m| m.go(Motion::Down, false),
            &|m| m.insert("x"),
            &|m| m.pointer_down(700.0, 300.0, 1, false),
        ];
        for (i, step) in steps.iter().enumerate() {
            blinking_since(&mut m, ms(600));
            m.tick();
            m.frame(1600, 1000, 1.0);
            assert!(m.ui.caret_hidden, "step {i}: hidden before");
            assert!(step(&mut m));
            m.frame(1600, 1000, 1.0);
            assert!(!m.ui.caret_hidden, "step {i}: shown at once");
            let since = m.ui.blink.since.expect("blinking");
            assert!(since.elapsed() < ms(200), "step {i}: the blink starts over");
            assert!(m.ui.blink.next_due(std::time::Instant::now()).is_some_and(|t| t > std::time::Instant::now() + ms(300)), "step {i}: hidden next a full half second on");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A window without the keys never blinks: its caret is solid and
    /// nothing is due; given the keys back, it blinks from then.
    #[test]
    fn an_unfocused_window_does_not_blink_and_has_nothing_due() {
        use kolada_editor::Motion;
        let (dir, mut m) = note("blink-focus");
        assert!(m.go(Motion::Right, false));
        blinking_since(&mut m, ms(600));
        m.tick();
        m.frame(1600, 1000, 1.0);
        assert!(m.ui.caret_hidden);
        assert!(m.set_focused(false), "the keys gone while hidden: shown again");
        margin_done(&mut m);
        m.frame(1600, 1000, 1.0);
        assert!(!m.ui.caret_hidden);
        assert_eq!(m.next_due(), None);
        assert!(!m.wants_frames() && !m.tick());
        // Moves without the keys (a drop, a click-through) do not blink either.
        assert!(m.go(Motion::Down, false));
        margin_done(&mut m);
        m.frame(1600, 1000, 1.0);
        assert_eq!(m.next_due(), None);
        assert!(!m.set_focused(true), "the keys back: shown, as it was");
        assert!(m.next_due().is_some(), "and blinking from now");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// "No blink" from the OS (Windows' INFINITE, less motion on the Mac or
    /// iOS) keeps the caret solid, with nothing due after a move.
    #[test]
    fn no_blink_keeps_the_caret_solid_with_nothing_due() {
        use kolada_editor::Motion;
        let (dir, mut m) = note("blink-none");
        m.set_caret_blink(None);
        assert!(m.go(Motion::Right, false));
        margin_done(&mut m);
        m.frame(1600, 1000, 1.0);
        blinking_since(&mut m, ms(600));
        assert!(!m.tick());
        m.frame(1600, 1000, 1.0);
        assert!(!m.ui.caret_hidden && m.next_due().is_none());
        m.set_caret_blink(Some((ms(500), ms(500))));
        m.reduce_motion(true);
        assert!(m.go(Motion::Right, false));
        margin_done(&mut m);
        blinking_since(&mut m, ms(600));
        assert!(!m.tick());
        m.frame(1600, 1000, 1.0);
        assert!(!m.ui.caret_hidden && m.next_due().is_none(), "less motion asked: solid");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A blink frame repaints the caret's sliver and nothing else.
    #[test]
    fn a_blink_frame_repaints_only_the_caret() {
        use kolada_editor::Motion;
        let (dir, mut m) = note("blink-damage");
        assert!(m.go(Motion::Down, false));
        m.frame(1600, 1000, 1.0);
        m.frame(1600, 1000, 1.0);
        assert_eq!(m.surface().last_damage.as_deref(), Some(&[][..]), "settled: an unchanged frame paints nothing");
        let c = m.caret_rect().expect("a caret");
        for (ago, hidden) in [(600, true), (1100, false)] {
            blinking_since(&mut m, ms(ago));
            assert!(m.tick());
            m.frame(1600, 1000, 1.0);
            assert_eq!(m.ui.caret_hidden, hidden);
            let damage = m.surface().last_damage.clone().expect("a partial frame");
            assert_eq!(damage.len(), 1, "one rectangle: {damage:?}");
            let (l, t, r, b) = damage[0];
            // The caret is drawn snapped to whole pixels.
            assert!(l as f32 <= c.left.round() && t as f32 <= c.top.round() && r as f32 >= c.right.round() && b as f32 >= c.bottom.round(), "it covers the caret: {:?} {c:?}", damage[0]);
            assert!(((r - l) as f32) <= c.width() + 4.0 && ((b - t) as f32) <= c.height() + 4.0, "and no more: {:?} {c:?}", damage[0]);
            println!("blink frame repaints {}x{} px (caret {}x{} DIPs at 1x)", r - l, b - t, c.width(), c.height());
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

impl Model {
    /// The OS asks for less motion (the Mac's Reduce Motion, iOS's too):
    /// panels, cards, chips and the pop-up note arrive and leave at once
    /// instead of sliding and fading. The shell says so for the app's own
    /// windows, at launch and whenever the setting changes.
    pub fn reduce_motion(&mut self, reduce: bool) {
        self.ui.animate = !reduce;
    }

    /// Whether the OS draws its window buttons over the window's top left
    /// (the Mac's traffic lights; not in full screen, where they hide): our
    /// controls keep clear of them only while they are there. Whether the
    /// screen changes.
    pub fn set_os_buttons(&mut self, shown: bool) -> bool {
        std::mem::replace(&mut self.os_buttons, shown) != shown
    }

    /// The OS the shell is, for the options it offers (the Mac by default).
    pub fn set_platform(&mut self, platform: scene::Platform) {
        self.platform = platform;
        // A minimal window has no meaning on iOS: a session that kept one
        // opens full.
        if platform == scene::Platform::Ios && self.session.minimal {
            self.session.minimal = false;
            self.settle_panels();
        }
    }

    /// The note alone, with none of Kolada's own chrome: the shell draws
    /// its own around it (the iPhone's native one). Before the first frame.
    pub fn set_bare(&mut self, on: bool) {
        self.bare = on;
    }

    /// What the last frame drew at (x, y) DIPs (for a shell's tests).
    pub fn target_at(&self, x: f32, y: f32) -> Option<Target> {
        scene::hit(&self.hits, x, y)
    }

    /// Where the last frame drew `target`, DIPs (for a shell's tests).
    pub fn place_of(&self, target: Target) -> Option<Rect> {
        self.hits.iter().find(|(_, t)| *t == target).map(|(r, _)| *r)
    }

    /// Everything the last frame drew that takes the pointer, and where.
    pub fn drawn_targets(&self) -> &[(Rect, Target)] {
        &self.hits
    }

    /// The keyboard has Command (the Mac's, an iPad's): shortcuts are
    /// named with its glyphs; elsewhere with Ctrl and Alt.
    pub(super) fn apple_keys(&self) -> bool {
        matches!(self.platform, scene::Platform::Mac | scene::Platform::Ios)
    }

    /// What a file is shown in: the Finder, Explorer, or on Linux the
    /// desktop's own file manager, whichever it is ("Show in Folder").
    pub(super) fn file_manager(&self) -> &'static str {
        match self.platform {
            scene::Platform::Windows => "Explorer",
            scene::Platform::Linux => "Folder",
            scene::Platform::Mac | scene::Platform::Ios => "Finder",
        }
    }

    /// The System theme's name in the palette.
    pub(super) fn system_theme_label(&self) -> &'static str {
        match self.platform {
            scene::Platform::Windows => "Theme like Windows",
            scene::Platform::Linux => "Theme like the desktop",
            scene::Platform::Mac | scene::Platform::Ios => "Theme like macOS",
        }
    }

    /// Which of the window buttons the chrome draws itself (Minimize,
    /// Maximize, Close: drawn where the OS draws none, `set_os_buttons(false)`,
    /// as on Windows and Linux) is at (x, y) DIPs. The shell does what it
    /// says, on the press's release over the same button.
    pub fn window_button(&self, x: f32, y: f32) -> Option<WindowButton> {
        if self.os_buttons {
            return None;
        }
        match scene::hit(&self.hits, x, y) {
            Some(Target::Minimize) => Some(WindowButton::Minimize),
            Some(Target::Maximize) => Some(WindowButton::Maximize),
            Some(Target::Close) => Some(WindowButton::Close),
            _ => None,
        }
    }

    /// The window fills the screen's work area (Maximize then shows as
    /// Restore). Whether the screen changes.
    pub fn set_maximized(&mut self, on: bool) -> bool {
        std::mem::replace(&mut self.ui.maximized, on) != on
    }
}

/// A window button the chrome draws itself (see `Model::window_button`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowButton {
    Minimize,
    Maximize,
    Close,
}
