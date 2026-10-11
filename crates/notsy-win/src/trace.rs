//! Startup markers in milliseconds since the process was created, so the
//! numbers include everything before `main` (loader, CRT, DLL imports).

use std::fmt::Write as _;
use std::sync::Mutex;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

static MARKS: Mutex<Vec<(&'static str, f64)>> = Mutex::new(Vec::new());
static INFO: Mutex<String> = Mutex::new(String::new());

/// Extra context for the trace line, e.g. which GPU was picked.
pub fn info(key: &str, value: &str) {
    if let Ok(mut info) = INFO.lock() {
        let _ = write!(info, " {key}={value:?}");
    }
}

fn ticks(t: FILETIME) -> u64 {
    (t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64
}

/// Milliseconds since this process was created.
pub fn since_start() -> f64 {
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    let ok = unsafe {
        GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user)
    };
    if ok.is_err() {
        return 0.0;
    }
    let now = unsafe { GetSystemTimePreciseAsFileTime() };
    ticks(now).saturating_sub(ticks(created)) as f64 / 10_000.0
}

pub fn mark(name: &'static str) {
    let t = since_start();
    if let Ok(mut marks) = MARKS.lock() {
        marks.push((name, t));
    }
}

/// When the marker `name` was reached, ms since the process was created.
pub fn at(name: &str) -> Option<f64> {
    MARKS.lock().ok()?.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// One line: `name=ms name=ms ...`.
pub fn report() -> String {
    let mut out = String::new();
    if let Ok(marks) = MARKS.lock() {
        for (name, t) in marks.iter() {
            let _ = write!(out, "{name}={t:.2} ");
        }
    }
    let mut out = out.trim_end().to_string();
    if let Ok(info) = INFO.lock() {
        out.push_str(&info);
    }
    out
}
