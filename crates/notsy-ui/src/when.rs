//! When a note was written, the way people say it: "just now", "5 min ago",
//! "today, 3:42 PM", "yesterday", "12 Sep", "12 Sep 2025"; a version's
//! moment for the time scrubber; a note's dates and word count for the top
//! of its page. Every shell says it the same way: only the local calendar
//! and the clock time (which follows the user's 12 or 24 hour setting) come
//! from the OS, through [`LocalTime`].

use std::cell::RefCell;
use std::time::{Duration, Instant, SystemTime};

/// A date on the local calendar: year, month 1..=12, day 1..=31.
pub type Day = (i32, u32, u32);

/// What the OS tells: where a moment falls on the local calendar, and its
/// clock time as the user's settings write it.
pub trait LocalTime {
    fn day(&self, t: SystemTime) -> Option<Day>;
    fn clock(&self, t: SystemTime) -> String;
}

/// Days since 1970-01-01 of a calendar date (proleptic Gregorian).
pub fn day_number(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn month(m: u32) -> &'static str {
    MONTHS[(m as usize).clamp(1, 12) - 1]
}

/// How long ago something on the calendar at `then` was, from `now`:
/// `seconds` ago, with its clock time `time` for today and yesterday.
pub fn phrase(then: Day, now: Day, seconds: u64, time: &str) -> String {
    let days = day_number(now.0, now.1, now.2) - day_number(then.0, then.1, then.2);
    match (seconds, days) {
        (0..60, _) => "just now".to_string(),
        (60..3600, _) => format!("{} min ago", seconds / 60),
        (_, 0) => format!("today, {time}"),
        (_, 1) => format!("yesterday, {time}"),
        _ if then.0 == now.0 => format!("{} {}", then.2, month(then.1)),
        _ => format!("{} {} {}", then.2, month(then.1), then.0),
    }
}

/// When `t` was, from now.
pub fn ago(os: &dyn LocalTime, t: SystemTime) -> String {
    let now = SystemTime::now();
    let (Some(a), Some(b)) = (os.day(t), os.day(now)) else { return String::new() };
    let seconds = now.duration_since(t).map_or(0, |d| d.as_secs());
    // The clock time is only said for today and yesterday.
    let time = if seconds >= 3600 && day_number(b.0, b.1, b.2) - day_number(a.0, a.1, a.2) <= 1 { os.clock(t) } else { String::new() };
    phrase(a, b, seconds, &time)
}

/// A version's moment, for the time scrubber: when ("Today, 3:42 PM",
/// "Tue 12 Sep, 9:10 AM") and how long ago ("2 hours ago").
pub fn moment_parts(then: Day, now: Day, seconds: u64, time: &str) -> (String, String) {
    let (a, b) = (day_number(then.0, then.1, then.2), day_number(now.0, now.1, now.2));
    let when = match b - a {
        0 => format!("Today, {time}"),
        1 => format!("Yesterday, {time}"),
        _ if then.0 == now.0 => format!("{} {} {}, {time}", DAYS[(a + 4).rem_euclid(7) as usize], then.2, month(then.1)),
        _ => format!("{} {} {}, {time}", then.2, month(then.1), then.0),
    };
    let ago = match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..7200 => "an hour ago".to_string(),
        7200..86_400 => format!("{} hours ago", seconds / 3600),
        _ => match b - a {
            1 => "a day ago".to_string(),
            d @ 2..14 => format!("{d} days ago"),
            d @ 14..60 => format!("{} weeks ago", d / 7),
            d @ 60..730 => format!("{} months ago", d / 30),
            d => format!("{} years ago", d / 365),
        },
    };
    (when, ago)
}

/// When `t` was and how long ago, for the time scrubber's bubble.
pub fn moment(os: &dyn LocalTime, t: SystemTime) -> (String, String) {
    let now = SystemTime::now();
    let (Some(a), Some(b)) = (os.day(t), os.day(now)) else { return (String::new(), String::new()) };
    let seconds = now.duration_since(t).map_or(0, |d| d.as_secs());
    moment_parts(a, b, seconds, &os.clock(t))
}

/// The day `then` was, from `now`, short ("Today", "Yesterday", "12 Sep",
/// "12 Sep 2025").
pub fn day_phrase(then: Day, now: Day) -> String {
    match day_number(now.0, now.1, now.2) - day_number(then.0, then.1, then.2) {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        _ if then.0 == now.0 => format!("{} {}", then.2, month(then.1)),
        _ => format!("{} {} {}", then.2, month(then.1), then.0),
    }
}

/// The day of `t`, short: where the scrubber's track begins.
pub fn day(os: &dyn LocalTime, t: SystemTime) -> String {
    match (os.day(t), os.day(SystemTime::now())) {
        (Some(a), Some(b)) => day_phrase(a, b),
        _ => String::new(),
    }
}

/// How a note's dates read together: when it was edited and made, only
/// made if it has not changed since, or only edited when the file says it
/// was made later (a copy, whose "made" is when it was copied).
pub fn dates(made: Option<(SystemTime, String)>, edited: Option<(SystemTime, String)>) -> Option<String> {
    match (made, edited) {
        (Some((c, made)), Some((m, edited))) => match m.duration_since(c) {
            Ok(d) if d.as_secs() < 60 => Some(format!("Created {made}")),
            Ok(_) => Some(format!("Edited {edited} \u{00B7} Created {made}")),
            Err(_) => Some(format!("Edited {edited}")),
        },
        (None, Some((_, edited))) => Some(format!("Edited {edited}")),
        (Some((_, made)), None) => Some(format!("Created {made}")),
        (None, None) => None,
    }
}

/// A note's dates, for the top of its page.
pub fn page_dates(os: &dyn LocalTime, created: Option<SystemTime>, modified: Option<SystemTime>) -> Option<String> {
    dates(created.map(|t| (t, ago(os, t))), modified.map(|t| (t, ago(os, t))))
}

/// The page's dates as said, kept for a second: a frame asks for them every
/// time it draws, and the words only change by the minute.
#[derive(Default)]
pub struct DateCache {
    kept: RefCell<Option<((Option<SystemTime>, Option<SystemTime>), Instant, Option<String>)>>,
}

impl DateCache {
    /// The dates said for `times`: the last said, if for the same times and
    /// less than a second old; otherwise `say()`.
    pub fn get(&self, times: (Option<SystemTime>, Option<SystemTime>), say: impl FnOnce() -> Option<String>) -> Option<String> {
        let mut kept = self.kept.borrow_mut();
        if let Some((t, at, said)) = &*kept {
            if *t == times && at.elapsed() < Duration::from_secs(1) {
                return said.clone();
            }
        }
        let said = say();
        *kept = Some((times, Instant::now(), said.clone()));
        said
    }
}

/// The words in a note: not pictures' embeds, not marks like `---`.
pub fn count_words(text: &str) -> usize {
    text.lines()
        .filter(|l| crate::images::embed_in(l).is_none())
        .flat_map(str::split_whitespace)
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
}

/// A count as people write it: 1,240.
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What follows the dates: " · 61 words" (nothing for an empty note).
pub fn words_phrase(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => " \u{00B7} 1 word".to_string(),
        n => format!(" \u{00B7} {} words", thousands(n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    /// A clock that says UTC and 24 hours, for tests.
    struct Utc;
    impl LocalTime for Utc {
        fn day(&self, t: SystemTime) -> Option<Day> {
            let d = t.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64 / 86_400;
            // Inverse of day_number, by search over a few centuries.
            let mut y = 1970 + (d / 366) as i32;
            while day_number(y + 1, 1, 1) <= d {
                y += 1;
            }
            let mut m = 1;
            while m < 12 && day_number(y, m + 1, 1) <= d {
                m += 1;
            }
            Some((y, m, (d - day_number(y, m, 1)) as u32 + 1))
        }
        fn clock(&self, t: SystemTime) -> String {
            let s = t.duration_since(UNIX_EPOCH).unwrap().as_secs() % 86_400;
            format!("{}:{:02}", s / 3600, s / 60 % 60)
        }
    }

    #[test]
    fn says_when_the_way_people_do() {
        let now = (2026, 9, 29);
        assert_eq!(phrase(now, now, 12, "3:42 PM"), "just now");
        assert_eq!(phrase(now, now, 25 * 60, "3:42 PM"), "25 min ago");
        assert_eq!(phrase(now, now, 5 * 3600, "10:15 AM"), "today, 10:15 AM");
        assert_eq!(phrase((2026, 9, 28), now, 20 * 3600, "23:05"), "yesterday, 23:05");
        assert_eq!(phrase((2026, 9, 12), now, 17 * 86_400, ""), "12 Sep");
        assert_eq!(phrase((2025, 12, 31), (2026, 1, 2), 2 * 86_400, ""), "31 Dec 2025");
        // Just past midnight, yesterday evening is still yesterday.
        assert_eq!(phrase((2026, 2, 28), (2026, 3, 1), 3 * 3600, "22:00"), "yesterday, 22:00");
    }

    #[test]
    fn says_when_a_version_was_written_and_how_long_ago() {
        let now = (2026, 10, 4);
        assert_eq!(moment_parts(now, now, 30, "3:42 PM"), ("Today, 3:42 PM".into(), "just now".into()));
        assert_eq!(moment_parts(now, now, 2 * 3600 + 5, "1:40 PM"), ("Today, 1:40 PM".into(), "2 hours ago".into()));
        assert_eq!(moment_parts((2026, 10, 3), now, 20 * 3600, "7:00 PM").0, "Yesterday, 7:00 PM");
        // 2026-09-29 is a Tuesday.
        assert_eq!(moment_parts((2026, 9, 29), now, 5 * 86_400, "9:10 AM"), ("Tue 29 Sep, 9:10 AM".into(), "5 days ago".into()));
        assert_eq!(moment_parts((2025, 12, 31), now, 277 * 86_400, "11:00").0, "31 Dec 2025, 11:00");
        assert_eq!(moment_parts((2026, 7, 1), now, 95 * 86_400, "").1, "3 months ago");
        assert_eq!(day_phrase(now, now), "Today");
        assert_eq!(day_phrase((2026, 10, 3), now), "Yesterday");
        assert_eq!(day_phrase((2026, 9, 12), now), "12 Sep");
    }

    #[test]
    fn puts_a_notes_dates_together() {
        let t = |s: u64| UNIX_EPOCH + Duration::from_secs(s);
        let (made, edited) = (Some((t(1000), "12 Sep".to_string())), Some((t(9000), "today".to_string())));
        assert_eq!(dates(made.clone(), edited.clone()).as_deref(), Some("Edited today \u{00B7} Created 12 Sep"));
        assert_eq!(dates(made.clone(), Some((t(1030), "12 Sep".into()))).as_deref(), Some("Created 12 Sep"));
        // A copy: made after it was last edited.
        assert_eq!(dates(Some((t(9999), "today".into())), Some((t(1000), "12 Sep".into()))).as_deref(), Some("Edited 12 Sep"));
        assert_eq!(dates(None, None), None);
    }

    #[test]
    fn counts_days_across_months_and_years() {
        assert_eq!(day_number(1970, 1, 1), 0);
        assert_eq!(day_number(2026, 3, 1) - day_number(2026, 2, 28), 1);
        assert_eq!(day_number(2024, 3, 1) - day_number(2024, 2, 28), 2, "leap year");
        assert_eq!(day_number(2026, 1, 1) - day_number(2025, 12, 31), 1);
    }

    #[test]
    fn counts_words_and_writes_thousands() {
        assert_eq!(count_words("# Plan\n- [ ] ship **it** now\n---\nملاحظة هنا"), 6);
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("Moodboard\n![[Pasted image 1.png]]\nMore words."), 3);
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1240), "1,240");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(words_phrase(0), "");
        assert_eq!(words_phrase(1), " \u{00B7} 1 word");
        assert_eq!(words_phrase(1240), " \u{00B7} 1,240 words");
    }

    #[test]
    fn says_a_moment_through_the_os_clock() {
        let now = SystemTime::now();
        assert_eq!(ago(&Utc, now), "just now");
        assert_eq!(day(&Utc, now), "Today");
        let (when, how) = moment(&Utc, now - Duration::from_secs(90 * 60));
        assert!(how == "an hour ago" && (when.starts_with("Today, ") || when.starts_with("Yesterday, ")), "{when} / {how}");
        assert!(page_dates(&Utc, Some(now), Some(now)).unwrap().starts_with("Created "));
    }

    #[test]
    fn keeps_the_dates_said_for_a_second() {
        let cache = DateCache::default();
        let times = (Some(UNIX_EPOCH), None);
        let mut said = 0;
        for _ in 0..50 {
            let got = cache.get(times, || {
                said += 1;
                Some("Created 1 Jan 1970".to_string())
            });
            assert_eq!(got.as_deref(), Some("Created 1 Jan 1970"));
        }
        assert_eq!(said, 1, "said once, not once per frame");
        // Other times are said afresh.
        cache.get((None, None), || {
            said += 1;
            None
        });
        assert_eq!(said, 2);
    }
}
