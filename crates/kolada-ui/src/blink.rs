//! The caret blinks, then rests. While the window has the keys it shows and
//! hides at the system's rate for `REST` after the last key, click, edit or
//! caret move; then it stays solid and nothing more is due, so an idle
//! window draws no frames. Every move shows it at once and starts the blink
//! over, so it is never missing right after one.
//!
//! The shell says the OS's rate (`period`) and whether the window has the
//! keys; the time is passed in, so tests need not wait. A blink is drawn by
//! leaving the caret's fill out of the frame: the surface repaints only
//! where it was.

use std::time::{Duration, Instant};

/// How long the caret blinks after the last thing done; then it rests, solid.
pub const REST: Duration = Duration::from_secs(10);

/// The rate where the OS says none: half a second shown, half hidden.
pub const DEFAULT: (Duration, Duration) = (Duration::from_millis(500), Duration::from_millis(500));

/// Whether the caret shows, and when that next changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blink {
    /// How long the caret shows, then hides, in each blink (None: it never
    /// blinks, as the OS asks).
    pub period: Option<(Duration, Duration)>,
    /// The window has the keys (an unfocused window's caret rests).
    pub focused: bool,
    /// When the caret was last shown anew (a move, an edit, the keys back):
    /// it blinks from then until `REST` later.
    pub since: Option<Instant>,
}

impl Default for Blink {
    fn default() -> Blink {
        Blink { period: Some(DEFAULT), focused: true, since: None }
    }
}

impl Blink {
    /// The OS's rate: shown for `on`, hidden for `off` (None, or either
    /// zero: it never blinks).
    pub fn set_period(&mut self, period: Option<(Duration, Duration)>) {
        self.period = period.filter(|(on, off)| !on.is_zero() && !off.is_zero());
    }

    /// The caret moved, or the text changed: it shows at once and blinks
    /// from now.
    pub fn restart(&mut self, now: Instant) {
        self.since = Some(now);
    }

    /// The window gained or lost the keys: gained, the caret shows and
    /// blinks from now; lost, it rests. Whether the caret's look changes.
    pub fn set_focused(&mut self, focused: bool, now: Instant) -> bool {
        let before = self.shown(now);
        self.focused = focused;
        self.since = focused.then_some(now);
        self.shown(now) != before
    }

    /// The time into the blink, while it blinks.
    fn blinking_for(&self, now: Instant) -> Option<(Duration, Duration, Duration)> {
        let (on, off) = self.period?;
        let since = self.since.filter(|_| self.focused)?;
        let t = now.saturating_duration_since(since);
        (t < REST).then_some((t, on, off))
    }

    /// Whether the caret is drawn at `now`.
    pub fn shown(&self, now: Instant) -> bool {
        match self.blinking_for(now) {
            Some((t, on, off)) => t.as_nanos() % (on + off).as_nanos() < on.as_nanos(),
            None => true,
        }
    }

    /// When the caret next shows or hides (None: it rests, solid, and
    /// nothing is due). Always after `now`.
    pub fn next_due(&self, now: Instant) -> Option<Instant> {
        let (t, on, off) = self.blinking_for(now)?;
        let since = self.since?;
        let cycle = (on + off).as_nanos();
        let phase = t.as_nanos() % cycle;
        let start = t.as_nanos() - phase;
        let next = Duration::from_nanos((if phase < on.as_nanos() { start + on.as_nanos() } else { start + cycle }) as u64);
        if next < REST {
            Some(since + next)
        } else if self.shown(now) {
            // Shown and resting from here on.
            None
        } else {
            // Hidden as the rest begins: shown solid then.
            Some(since + REST)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn blinking(on: u64, off: u64) -> (Blink, Instant) {
        let mut b = Blink::default();
        b.set_period(Some((ms(on), ms(off))));
        let t0 = Instant::now();
        b.restart(t0);
        (b, t0)
    }

    /// Shown then hidden at the rate given, each change due once, until it
    /// rests solid with nothing due.
    #[test]
    fn blinks_at_the_rate_given_then_rests_solid() {
        let (b, t0) = blinking(530, 530);
        assert!(b.shown(t0), "shown at once");
        assert_eq!(b.next_due(t0), Some(t0 + ms(530)));
        assert!(!b.shown(t0 + ms(530)) && b.shown(t0 + ms(1060)));
        assert_eq!(b.next_due(t0 + ms(600)), Some(t0 + ms(1060)));
        // Walk the toggles: each is due after the last, and they end.
        let (mut at, mut toggles) = (t0, 0);
        while let Some(next) = b.next_due(at) {
            assert!(next > at);
            assert_ne!(b.shown(at), b.shown(next), "each wake changes the caret");
            at = next;
            toggles += 1;
            assert!(toggles < 100);
        }
        assert!(b.shown(at), "it rests shown");
        assert!(at <= t0 + REST, "within the rest period");
        assert!(b.shown(t0 + REST + ms(1)) && b.next_due(t0 + REST).is_none());
    }

    /// Hidden as the rest period begins, the caret is due back then, solid.
    #[test]
    fn hidden_at_the_rest_comes_back_solid() {
        // 10 s is three 3 s cycles and a second more: past the 0.9 s shown.
        let (b, t0) = blinking(900, 2100);
        assert!(!b.shown(t0 + ms(9_950)));
        assert_eq!(b.next_due(t0 + ms(9_950)), Some(t0 + REST));
        assert!(b.shown(t0 + REST) && b.next_due(t0 + REST).is_none());
    }

    #[test]
    fn a_move_shows_it_at_once_and_starts_over() {
        let (mut b, t0) = blinking(500, 500);
        let later = t0 + ms(700);
        assert!(!b.shown(later));
        b.restart(later);
        assert!(b.shown(later));
        assert_eq!(b.next_due(later), Some(later + ms(500)));
    }

    #[test]
    fn without_the_keys_or_with_no_blink_it_is_solid_and_nothing_is_due() {
        let (mut b, t0) = blinking(500, 500);
        let hidden = t0 + ms(700);
        assert!(b.set_focused(false, hidden), "shown again as the keys go");
        assert!(b.shown(hidden) && b.next_due(hidden).is_none());
        b.restart(hidden);
        assert!(b.shown(hidden + ms(600)) && b.next_due(hidden).is_none(), "moves without the keys do not blink");
        assert!(!b.set_focused(true, hidden), "the keys back: shown, as it was");
        assert_eq!(b.next_due(hidden), Some(hidden + ms(500)));
        for period in [None, Some((ms(0), ms(500)))] {
            b.set_period(period);
            b.restart(t0);
            assert!(b.shown(t0 + ms(700)) && b.next_due(t0).is_none());
        }
    }
}
