//! What moves between frames, as on Windows: a wheel's scroll eases in,
//! the caret glides to where it goes, a note just opened fades in, and the
//! scroll bar shows for a moment after scrolling. The view asks
//! `wants_frames` after each event and, while it is true, calls `tick` once
//! a display refresh.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use super::*;

/// A scroll eases in: what is left of it shrinks at this rate per second.
const SCROLL_RATE: f32 = 18.0;
/// Stiffness of the caret's glide (rad/s), critically damped: it arrives
/// quickly and never overshoots.
const CARET_OMEGA: f32 = 42.0;
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
    }

    /// When something next has to happen by the clock while nothing moves
    /// (a reveal, a pause in typing, a scroll bar or a passing word starting
    /// to fade): the view wakes once then, not every frame until then.
    pub fn next_due(&self) -> Option<Instant> {
        let ui = &self.ui;
        let bar = ui.scrolled_at.map(|t| t + super::bars::SHOWN).filter(|t| *t > Instant::now());
        let whisper = ui.whisper.as_ref().map(|(_, t)| *t + WHISPER - WHISPER_OUT);
        [bar, whisper, self.panels_due(), self.chips_due(), self.assist_due()].into_iter().flatten().min()
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
        changed |= self.step_chips(dt);
        changed |= self.step_arrival(dt);
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
        if !self.wants_frames() {
            self.ui.last_tick = None;
        }
        changed
    }

    /// The caret moved; it glides there unless `glide` is false (typing,
    /// where it must stay with the letters, or dragging a selection).
    pub(super) fn caret_moved(&mut self, glide: bool) {
        self.ui.caret_jumped = true;
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
    #[cfg(test)]
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
        while settle(&mut x, &mut v, 1.0 / 60.0, 42.0) {
            assert!(x <= 0.0, "never past where it goes: {x}");
            steps += 1;
            assert!(steps < 120, "and it gets there");
        }
        assert_eq!((x, v), (0.0, 0.0));
        assert!(steps < 20, "in a fraction of a second: {steps} frames");
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
    }
}
