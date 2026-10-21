//! Linux's calendar and clock, for the model's `Calendar` service: the local
//! calendar from the C library (the zone in `TZ` or `/etc/localtime`), clock
//! times on the 24-hour clock. The words that say when ("5 min ago",
//! "Yesterday") are shared (`kolada_ui::when`).

use std::time::{SystemTime, UNIX_EPOCH};

use kolada_app::services::{Calendar, Local};

pub struct LinuxCalendar;

impl Calendar for LinuxCalendar {
    fn local(&self, t: SystemTime) -> Option<Local> {
        let secs = match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        };
        let at = secs as libc::time_t;
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
        self.local(t).map(|l| format!("{:02}:{:02}", l.hour, l.minute)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_local_calendar_and_clock() {
        let l = LinuxCalendar.local(SystemTime::now()).unwrap();
        assert!(l.year >= 2026 && (1..=12).contains(&l.month) && (1..=31).contains(&l.day) && l.weekday < 7 && l.hour < 24);
        assert_eq!(LinuxCalendar.clock(SystemTime::now()).len(), 5);
        // A known moment lands on its day, give or take the zone.
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_791_553_507);
        let l = LinuxCalendar.local(t).unwrap();
        assert_eq!((l.year, l.month), (2026, 10));
        assert!((8..=10).contains(&l.day));
    }
}
