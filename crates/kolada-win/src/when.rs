//! When a note was written, the way people say it. The words are shared
//! (`kolada_ui::when`); this is the Windows clock: the local calendar, and
//! clock times that follow the user's settings (12 or 24 hours).

use std::time::{SystemTime, UNIX_EPOCH};

use kolada_ui::when::{Day, LocalTime};
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::core::PCWSTR;

/// `t` on the local calendar and clock.
fn local(t: SystemTime) -> Option<SYSTEMTIME> {
    let ticks = t.duration_since(UNIX_EPOCH).ok()?.as_nanos() / 100 + 116_444_736_000_000_000;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut here = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&ft, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut here).ok()?;
    }
    Some(here)
}

/// The Windows calendar and clock.
pub struct Windows;

impl LocalTime for Windows {
    fn day(&self, t: SystemTime) -> Option<Day> {
        let s = local(t)?;
        Some((s.wYear as i32, s.wMonth as u32, s.wDay as u32))
    }

    fn clock(&self, t: SystemTime) -> String {
        let Some(system) = local(t) else { return String::new() };
        let mut buf = [0u16; 32];
        let n = unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&system), PCWSTR::null(), Some(&mut buf)) };
        String::from_utf16_lossy(&buf[..(n as usize).saturating_sub(1)])
    }
}

/// When `t` was, from now.
pub fn ago(t: SystemTime) -> String {
    kolada_ui::when::ago(&Windows, t)
}

/// When `t` was and how long ago, for the time scrubber's bubble.
pub fn moment(t: SystemTime) -> (String, String) {
    kolada_ui::when::moment(&Windows, t)
}

/// The day of `t`, short: where the scrubber's track begins.
pub fn day(t: SystemTime) -> String {
    kolada_ui::when::day(&Windows, t)
}

/// A note's dates, for the top of its page.
pub fn page_dates(created: Option<SystemTime>, modified: Option<SystemTime>) -> Option<String> {
    kolada_ui::when::page_dates(&Windows, created, modified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_local_clock() {
        let (y, m, d) = Windows.day(SystemTime::now()).unwrap();
        assert!(y >= 2026 && (1..=12).contains(&m) && (1..=31).contains(&d));
        assert!(!Windows.clock(SystemTime::now()).is_empty());
    }
}
