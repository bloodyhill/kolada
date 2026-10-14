//! Something done a moment after the last of a run of events (spelling
//! resumes, the note is written): each keystroke only moves a deadline on,
//! and AppKit is asked once per quiet spell, not cancelled and asked again
//! per key (`cancelPreviousPerformRequests` cost about 0.02 ms a key each).

use std::cell::Cell;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Later {
    /// When it is due, pushed on by each event.
    due: Cell<Option<Instant>>,
    /// Whether AppKit already holds a request to call back.
    asked: Cell<bool>,
}

/// What a callback does once AppKit makes it.
#[derive(Debug, PartialEq)]
pub enum Fire {
    /// The deadline passed: do the work.
    Now,
    /// Events moved the deadline on: ask again for this many seconds.
    Again(f64),
    /// Nothing is due (it was done or called off).
    Nothing,
}

impl Later {
    /// Due `delay` seconds from `now`, sooner or later than it was. The
    /// seconds to ask AppKit for, when no request is held yet or the new
    /// deadline is sooner than the one asked for.
    pub fn push(&self, now: Instant, delay: f64) -> Option<f64> {
        let at = now + Duration::from_secs_f64(delay);
        let sooner = self.due.get().is_some_and(|due| at < due);
        self.due.set(Some(at));
        if self.asked.get() && !sooner {
            return None;
        }
        self.asked.set(true);
        Some(delay)
    }

    /// AppKit called back (a sooner push may leave an earlier request
    /// standing; it finds the deadline not reached and asks again).
    pub fn fire(&self, now: Instant) -> Fire {
        self.asked.set(false);
        match self.due.get() {
            None => Fire::Nothing,
            Some(at) if at <= now => {
                self.due.set(None);
                Fire::Now
            }
            Some(at) => {
                self.asked.set(true);
                Fire::Again((at - now).as_secs_f64().max(0.001))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_of_keystrokes_asks_appkit_once_and_the_work_waits_for_the_last() {
        let later = Later::default();
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        assert_eq!(later.push(t0, 0.4), Some(0.4));
        for n in 1..100 {
            assert_eq!(later.push(ms(n), 0.4), None, "keystroke {n} asked AppKit again");
        }
        // The first request comes back 400 ms after the first key: the last
        // came at 99 ms, so it waits until 499 ms.
        match later.fire(ms(400)) {
            Fire::Again(s) => assert!((s - 0.099).abs() < 0.002, "{s}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(later.push(ms(450), 0.4), None, "a request is held");
        assert!(matches!(later.fire(ms(499)), Fire::Again(_)));
        assert_eq!(later.fire(ms(850)), Fire::Now);
        assert_eq!(later.fire(ms(900)), Fire::Nothing, "done once");
        assert_eq!(later.push(ms(1000), 0.4), Some(0.4), "a new run asks again");
    }

    #[test]
    fn a_sooner_deadline_asks_appkit_for_the_sooner_time() {
        let later = Later::default();
        let t0 = Instant::now();
        assert_eq!(later.push(t0, 0.4), Some(0.4));
        // Spelling found more lines to check: next step in 20 ms, not 400.
        assert_eq!(later.push(t0, 0.02), Some(0.02));
        assert_eq!(later.fire(t0 + Duration::from_millis(20)), Fire::Now);
        // The 400 ms request still comes back and finds nothing due.
        assert_eq!(later.fire(t0 + Duration::from_millis(400)), Fire::Nothing);
    }
}
