//! Startup timing, printed when `KOLADA_TRACE` is set: each stage's time
//! since `main`, and the wall clock (nanoseconds since the epoch) at which it
//! happened, so a script that noted the time before starting the program
//! can say how long the whole launch took, loader included
//! (`tools/speed-gate-linux.sh`).

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KOLADA_TRACE").is_some())
}

fn started() -> Instant {
    static AT: OnceLock<Instant> = OnceLock::new();
    *AT.get_or_init(Instant::now)
}

/// Milliseconds since `main` began (the first `mark`).
pub fn age_ms() -> f64 {
    started().elapsed().as_secs_f64() * 1000.0
}

/// Says that `what` has just happened.
pub fn mark(what: &str) {
    let ms = age_ms();
    if enabled() {
        let epoch = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        eprintln!("{what}: {ms:.1} ms (at {epoch} ns)");
    }
}

/// `KOLADA_EXIT_AFTER_FIRST_FRAME` is set (the speed gate sets it): the app
/// quits, by its normal quit, once the first frame is on the screen.
pub fn exit_after_first_frame() -> bool {
    std::env::var_os("KOLADA_EXIT_AFTER_FIRST_FRAME").is_some()
}
