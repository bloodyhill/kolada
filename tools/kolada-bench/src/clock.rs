//! QueryPerformanceCounter time, the same clock DXGI stamps frames with.

use std::sync::OnceLock;

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub fn now() -> i64 {
    let mut t = 0;
    // Cannot fail on Windows XP and later.
    let _ = unsafe { QueryPerformanceCounter(&mut t) };
    t
}

fn frequency() -> i64 {
    static FREQ: OnceLock<i64> = OnceLock::new();
    *FREQ.get_or_init(|| {
        let mut f = 0;
        let _ = unsafe { QueryPerformanceFrequency(&mut f) };
        f
    })
}

pub fn ms_between(from: i64, to: i64) -> f64 {
    (to - from) as f64 * 1000.0 / frequency() as f64
}

pub fn ticks(ms: f64) -> i64 {
    (ms * frequency() as f64 / 1000.0) as i64
}
