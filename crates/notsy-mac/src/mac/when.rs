//! The Mac's calendar and clock, for the model's `Calendar` service: the local
//! calendar, and clock times that follow the user's 12 or 24 hour setting.
//! The words that say when ("5 min ago", "Yesterday") are shared
//! (`notsy_ui::when`, used by `notsy-app`).

use std::time::{SystemTime, UNIX_EPOCH};

use notsy_app::services::{Calendar, Local};
use objc2_foundation::{NSDate, NSDateFormatter, NSDateFormatterStyle};

fn secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// The Mac's calendar and clock.
pub struct Mac;

impl Calendar for Mac {
    fn local(&self, t: SystemTime) -> Option<Local> {
        let at: libc::time_t = secs(t) as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if unsafe { libc::localtime_r(&at, &mut tm) }.is_null() {
            return None;
        }
        Some(Local {
            year: tm.tm_year + 1900,
            month: tm.tm_mon as u32 + 1,
            day: tm.tm_mday as u32,
            weekday: tm.tm_wday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
            second: tm.tm_sec as u32,
        })
    }

    fn clock(&self, t: SystemTime) -> String {
        let formatter = NSDateFormatter::new();
        formatter.setDateStyle(NSDateFormatterStyle::NoStyle);
        formatter.setTimeStyle(NSDateFormatterStyle::ShortStyle);
        let date = NSDate::dateWithTimeIntervalSince1970(secs(t) as f64);
        // The narrow no-break space before AM and PM is not in every font.
        formatter.stringFromDate(&date).to_string().replace(['\u{202F}', '\u{A0}'], " ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_local_calendar_and_clock() {
        let l = Mac.local(SystemTime::now()).unwrap();
        assert!(l.year >= 2026 && (1..=12).contains(&l.month) && (1..=31).contains(&l.day) && l.weekday < 7 && l.hour < 24);
        assert!(!Mac.clock(SystemTime::now()).is_empty());
    }
}
