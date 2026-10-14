//! Dates said in plain words: `tomorrow`, `fri`, `in 3 days`, `oct 12`,
//! `every monday`, and a time of day after one (`fri 4pm`, `at 16:30`). A
//! task writes its due date after an `@`, and [`parse`] reads the longest
//! run of words that makes a date. It works on calendar days and clock
//! times only, given today, so it needs no time zone and every shell gets
//! the same answer.

/// A calendar day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// How a task comes round again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Daily,
    Weekly,
    Monthly,
    Yearly,
    /// Every given weekday: 0 for Monday through 6 for Sunday.
    On(u32),
}

/// A date found in text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Due {
    pub day: Day,
    pub repeat: Option<Repeat>,
    /// A time of day, hours and minutes, when one is said.
    pub time: Option<(u32, u32)>,
    /// How many bytes of the text it took, counted from where the words begin.
    pub len: usize,
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

impl Day {
    pub fn new(year: i32, month: u32, day: u32) -> Option<Day> {
        let ok = (1..=9999).contains(&year) && (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month);
        ok.then_some(Day { year, month, day })
    }

    /// Days since 1970-01-01.
    pub fn number(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_number(n: i64) -> Day {
        let z = n + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Day { year, month, day }
    }

    /// 0 for Monday through 6 for Sunday.
    pub fn weekday(self) -> u32 {
        (self.number() + 3).rem_euclid(7) as u32
    }

    pub fn plus_days(self, n: i64) -> Day {
        Day::from_number(self.number() + n)
    }

    /// Months on, keeping the day of the month or the month's last day.
    pub fn plus_months(self, n: i32) -> Day {
        let total = self.year * 12 + self.month as i32 - 1 + n;
        let (year, month) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
        Day { year, month, day: self.day.min(days_in_month(year, month)) }
    }

    /// `2026-10-12`.
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        _ => 28,
    }
}

impl Repeat {
    /// The day after `from` that this repeat falls on.
    pub fn after(self, from: Day) -> Day {
        match self {
            Repeat::Daily => from.plus_days(1),
            Repeat::Weekly => from.plus_days(7),
            Repeat::Monthly => from.plus_months(1),
            Repeat::Yearly => from.plus_months(12),
            Repeat::On(wd) => coming(from, wd),
        }
    }
}

/// A date as Kolada writes one down for good (`Fri 9 Oct 2026`, with its
/// time: `Fri 9 Oct 2026 4pm`), so it means the same day tomorrow as today.
pub fn written(day: Day, time: Option<(u32, u32)>) -> String {
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTH_NAMES: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut out = format!("{} {} {} {}", NAMES[day.weekday() as usize], day.day, MONTH_NAMES[day.month as usize - 1], day.year);
    if let Some((h, m)) = time {
        let (h12, half) = (if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "am" } else { "pm" });
        out.push_str(&if m == 0 { format!(" {h12}{half}") } else { format!(" {h12}:{m:02}{half}") });
    }
    out
}

/// Whether the date `text` starts with means the same day whenever it is
/// read (`oct 12 2026`, not `fri` or `oct 12`).
pub fn is_fixed(text: &str, today: Day) -> bool {
    let (a, b) = (parse(text, today), parse(text, today.plus_days(400)));
    a.is_some() && a.map(|d| d.day) == b.map(|d| d.day)
}

/// The dates in `text` after each `@`, with where each `@` is. A date
/// someone has typed after an `@` is the only kind of date Kolada looks for.
pub fn find_all(text: &str, today: Day) -> Vec<(usize, Due)> {
    text.match_indices('@')
        .filter(|(i, _)| *i == 0 || text[..*i].ends_with(char::is_whitespace))
        .filter_map(|(i, _)| parse(&text[i + 1..], today).map(|d| (i, d)))
        .collect()
}

/// The date `text` starts with, if it starts with one, and a time of day
/// after it. Reads at most five words for the date and keeps the longest run
/// that is one; a time alone (`4pm`) is today's.
pub fn parse(text: &str, today: Day) -> Option<Due> {
    let mut spans = Vec::new(); // where each word is, without trailing punctuation
    let mut at = 0;
    for _ in 0..8 {
        let rest = &text[at..];
        let start = at + rest.len() - rest.trim_start().len();
        let end = start + text[start..].find(char::is_whitespace).unwrap_or(text.len() - start);
        if end == start {
            break;
        }
        let kept = text[start..end].trim_end_matches([',', '.', ';', ')']).len();
        spans.push((start, start + kept));
        at = end;
    }
    let words: Vec<String> = spans.iter().map(|&(s, e)| text[s..e].to_lowercase()).collect();
    let all: Vec<&str> = words.iter().map(String::as_str).collect();
    for n in (1..=all.len().min(5)).rev() {
        if let Some((day, repeat)) = read(&all[..n], today) {
            let (time, len) = match time_of(&all[n..]) {
                Some((time, k)) => (Some(time), spans[n + k - 1].1),
                None => (None, spans[n - 1].1),
            };
            return Some(Due { day, repeat, time, len });
        }
    }
    let (time, k) = time_of(&all)?;
    Some(Due { day: today, repeat: None, time: Some(time), len: spans[k - 1].1 })
}

/// A time of day the words start with (`4pm`, `4:30 pm`, `16:30`, `at
/// noon`), and how many words it took.
fn time_of(w: &[&str]) -> Option<((u32, u32), usize)> {
    let (w, skip) = match w {
        ["at", rest @ ..] => (rest, 1),
        _ => (w, 0),
    };
    let first = *w.first()?;
    if first == "noon" || first == "midday" {
        return Some(((12, 0), skip + 1));
    }
    let (clock, half, used) = match (first.strip_suffix("am").or_else(|| first.strip_suffix("pm")), w.get(1)) {
        (Some(c), _) => (c, Some(first.ends_with("pm")), 1),
        (None, Some(&h)) if h == "am" || h == "pm" => (first, Some(h == "pm"), 2),
        _ => (first, None, 1),
    };
    let (h, m) = match clock.split_once(':') {
        Some((h, m)) if m.len() == 2 => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        // A bare number is a time only with am or pm after it.
        None if half.is_some() => (clock.parse::<u32>().ok()?, 0),
        _ => return None,
    };
    let h = match half {
        Some(pm) if (1..=12).contains(&h) => h % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None => h,
    };
    (h < 24 && m < 60).then_some(((h, m), skip + used))
}

fn read(w: &[&str], today: Day) -> Option<(Day, Option<Repeat>)> {
    if let ["every", rest @ ..] = w {
        return every(rest, today);
    }
    one_day(w, today).map(|d| (d, None))
}

/// A repeat starts today, or on its weekday's next turn today included;
/// once ticked, the next turn counts from then (see `Repeat::after`).
fn every(w: &[&str], today: Day) -> Option<(Day, Option<Repeat>)> {
    match w {
        ["day"] => Some((today, Some(Repeat::Daily))),
        ["week"] => Some((today, Some(Repeat::Weekly))),
        ["month"] => Some((today, Some(Repeat::Monthly))),
        ["year"] => Some((today, Some(Repeat::Yearly))),
        [d] => weekday_of(d).map(|wd| (coming(today.plus_days(-1), wd), Some(Repeat::On(wd)))),
        _ => None,
    }
}

fn one_day(w: &[&str], today: Day) -> Option<Day> {
    match w {
        ["today"] => Some(today),
        ["tomorrow"] | ["tmrw"] => Some(today.plus_days(1)),
        ["yesterday"] => Some(today.plus_days(-1)),
        ["next", "week"] => Some(monday_after(today)),
        ["next", "month"] => Some(today.plus_months(1).with_day(1)),
        ["next", d] => weekday_of(d).map(|wd| monday_after(today).plus_days(i64::from(wd))),
        ["this", d] => weekday_of(d).map(|wd| today.plus_days((i64::from(wd) - i64::from(today.weekday())).rem_euclid(7))),
        ["in", n, unit] => {
            let n: i64 = if *n == "a" || *n == "an" { 1 } else { n.parse().ok().filter(|n| *n <= 10_000)? };
            match unit.trim_end_matches('s') {
                "day" => Some(today.plus_days(n)),
                "week" => Some(today.plus_days(7 * n)),
                "month" => Some(today.plus_months(n as i32)),
                "year" => Some(today.plus_months(12 * n as i32)),
                _ => None,
            }
        }
        [d] if weekday_of(d).is_some() => Some(coming(today, weekday_of(d)?)),
        [d] => iso(d),
        [a, b] => month_day(a, b, today),
        [a, b, c] => month_day_year(a, b, c),
        // As `written` puts it: `fri 9 oct 2026`.
        [w, a, b, c] if weekday_of(w).is_some() => month_day_year(a, b, c),
        _ => None,
    }
}

impl Day {
    fn with_day(self, day: u32) -> Day {
        Day { day, ..self }
    }
}

/// The first `wd` strictly after today.
fn coming(today: Day, wd: u32) -> Day {
    let ahead = (i64::from(wd) - i64::from(today.weekday())).rem_euclid(7);
    today.plus_days(if ahead == 0 { 7 } else { ahead })
}

fn monday_after(today: Day) -> Day {
    today.plus_days(7 - i64::from(today.weekday()))
}

fn weekday_of(w: &str) -> Option<u32> {
    let full = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    WEEKDAYS.iter().position(|d| *d == w).or_else(|| full.iter().position(|d| *d == w)).or(match w {
        "tues" => Some(1),
        "weds" => Some(2),
        "thur" | "thurs" => Some(3),
        _ => None,
    }).map(|i| i as u32)
}

fn month_of(w: &str) -> Option<u32> {
    const FULL: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    let w = if w == "sept" { "sep" } else { w };
    MONTHS.iter().position(|m| *m == w).or_else(|| FULL.iter().position(|m| *m == w)).map(|i| i as u32 + 1)
}

fn ordinal(w: &str) -> Option<u32> {
    let digits = w.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &w[digits.len()..];
    if !matches!(suffix, "" | "st" | "nd" | "rd" | "th") || digits.len() > 2 {
        return None;
    }
    digits.parse().ok()
}

/// `oct 12` or `12 oct`: the next such day, today included.
fn month_day(a: &str, b: &str, today: Day) -> Option<Day> {
    let (month, day) = match (month_of(a), ordinal(b)) {
        (Some(m), Some(d)) => (m, d),
        _ => (month_of(b)?, ordinal(a)?),
    };
    let this = Day::new(today.year, month, day)?;
    if this >= today { Some(this) } else { Day::new(today.year + 1, month, day) }
}

fn month_day_year(a: &str, b: &str, c: &str) -> Option<Day> {
    let year: i32 = c.parse().ok().filter(|y| (1000..=9999).contains(y))?;
    let (month, day) = match (month_of(a), ordinal(b)) {
        (Some(m), Some(d)) => (m, d),
        _ => (month_of(b)?, ordinal(a)?),
    };
    Day::new(year, month, day)
}

fn iso(w: &str) -> Option<Day> {
    let mut p = w.split('-');
    let (y, m, d) = (p.next()?, p.next()?, p.next()?);
    if p.next().is_some() || y.len() != 4 {
        return None;
    }
    Day::new(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Monday 2026-10-05
    const TODAY: Day = Day { year: 2026, month: 10, day: 5 };

    fn day(s: &str) -> Day {
        iso(s).unwrap()
    }

    fn on(text: &str) -> Option<Day> {
        parse(text, TODAY).map(|d| d.day)
    }

    #[test]
    fn days_count_from_the_epoch_and_come_back_the_same() {
        assert_eq!(day("1970-01-01").number(), 0);
        assert_eq!(day("2000-03-01").number(), 11_017);
        for n in [-800_000, -1, 0, 59, 11_016, 20_000, 2_000_000] {
            assert_eq!(Day::from_number(n).number(), n);
        }
        assert_eq!(TODAY.weekday(), 0, "2026-10-05 is a Monday");
        assert_eq!(day("2000-02-29").plus_days(1), day("2000-03-01"));
    }

    #[test]
    fn a_made_up_day_is_not_a_day() {
        assert_eq!(Day::new(2026, 2, 29), None);
        assert_eq!(Day::new(2028, 2, 29), Some(day("2028-02-29")));
        assert_eq!(Day::new(2026, 13, 1), None);
        assert_eq!(on("2026-02-30"), None);
    }

    #[test]
    fn today_tomorrow_and_yesterday() {
        assert_eq!(on("today"), Some(TODAY));
        assert_eq!(on("Tomorrow"), Some(day("2026-10-06")));
        assert_eq!(on("yesterday"), Some(day("2026-10-04")));
    }

    #[test]
    fn a_weekday_is_the_next_one_and_never_today() {
        assert_eq!(on("fri"), Some(day("2026-10-09")));
        assert_eq!(on("Friday"), Some(day("2026-10-09")));
        assert_eq!(on("monday"), Some(day("2026-10-12")), "said on a Monday it means next week's");
        assert_eq!(on("this monday"), Some(TODAY));
        assert_eq!(on("this sun"), Some(day("2026-10-11")));
    }

    #[test]
    fn next_means_the_following_week() {
        assert_eq!(on("next week"), Some(day("2026-10-12")));
        assert_eq!(on("next friday"), Some(day("2026-10-16")));
        assert_eq!(on("next month"), Some(day("2026-11-01")));
    }

    #[test]
    fn in_a_number_of_days_weeks_months_or_years() {
        assert_eq!(on("in 3 days"), Some(day("2026-10-08")));
        assert_eq!(on("in a week"), Some(day("2026-10-12")));
        assert_eq!(on("in 2 weeks"), Some(day("2026-10-19")));
        assert_eq!(on("in 4 months"), Some(day("2027-02-05")));
        assert_eq!(on("in 1 year"), Some(day("2027-10-05")));
        assert_eq!(on("in 3 fortnights"), None);
    }

    #[test]
    fn a_month_end_is_kept_when_months_are_added() {
        assert_eq!(day("2026-01-31").plus_months(1), day("2026-02-28"));
        assert_eq!(day("2028-01-31").plus_months(1), day("2028-02-29"));
        assert_eq!(day("2026-11-30").plus_months(3), day("2027-02-28"));
    }

    #[test]
    fn a_month_and_day_is_the_next_time_it_comes_round() {
        assert_eq!(on("oct 12"), Some(day("2026-10-12")));
        assert_eq!(on("12 October"), Some(day("2026-10-12")));
        assert_eq!(on("oct 5"), Some(TODAY), "today still counts");
        assert_eq!(on("oct 4"), Some(day("2027-10-04")), "already gone, so next year");
        assert_eq!(on("march 3rd"), Some(day("2027-03-03")));
        assert_eq!(on("sept 9"), Some(day("2027-09-09")));
        assert_eq!(on("jan 9 2030"), Some(day("2030-01-09")));
        assert_eq!(on("2026-12-25"), Some(day("2026-12-25")));
    }

    #[test]
    fn every_says_how_it_comes_round() {
        let d = parse("every monday", TODAY).unwrap();
        assert_eq!((d.day, d.repeat), (TODAY, Some(Repeat::On(0))), "said on a Monday, today is its turn");
        assert_eq!(parse("every friday", TODAY).unwrap().day, day("2026-10-09"));
        assert_eq!(parse("every day", TODAY).unwrap().repeat, Some(Repeat::Daily));
        assert_eq!(parse("every month", TODAY).unwrap().day, TODAY);
        assert_eq!(parse("every year", TODAY).unwrap().repeat, Some(Repeat::Yearly));
        assert_eq!(Repeat::Monthly.after(day("2026-01-31")), day("2026-02-28"));
        assert_eq!(Repeat::Weekly.after(day("2026-12-28")), day("2027-01-04"));
        assert_eq!(Repeat::On(0).after(day("2026-10-06")), day("2026-10-12"), "ticked late on a Tuesday, next Monday");
        assert_eq!(Repeat::On(0).after(TODAY), day("2026-10-12"));
    }

    #[test]
    fn a_date_written_down_for_good_reads_back_the_same_any_day() {
        assert_eq!(written(day("2026-10-09"), None), "Fri 9 Oct 2026");
        assert_eq!(written(TODAY, Some((16, 0))), "Mon 5 Oct 2026 4pm");
        assert_eq!(written(TODAY, Some((0, 5))), "Mon 5 Oct 2026 12:05am");
        for (d, t) in [(day("2026-10-09"), None), (TODAY, Some((16, 30))), (day("2027-01-01"), Some((12, 0)))] {
            let w = written(d, t);
            let back = parse(&w, TODAY.plus_days(900)).unwrap();
            assert_eq!((back.day, back.time, back.len), (d, t, w.len()), "{w}");
            assert!(is_fixed(&w, TODAY));
        }
        assert!(!is_fixed("fri", TODAY) && !is_fixed("oct 12", TODAY) && !is_fixed("today", TODAY));
        assert!(is_fixed("2026-12-25", TODAY) && !is_fixed("soon", TODAY));
    }

    #[test]
    fn a_time_of_day_can_follow_a_date_or_stand_alone() {
        fn at(t: &str) -> Option<(Day, Option<(u32, u32)>, &str)> {
            parse(t, TODAY).map(|d| (d.day, d.time, &t[..d.len]))
        }
        assert_eq!(at("fri 4pm call"), Some((day("2026-10-09"), Some((16, 0)), "fri 4pm")));
        assert_eq!(at("tomorrow at 16:30."), Some((day("2026-10-06"), Some((16, 30)), "tomorrow at 16:30")));
        assert_eq!(at("oct 12 9:15 am"), Some((day("2026-10-12"), Some((9, 15)), "oct 12 9:15 am")));
        assert_eq!(at("12am"), Some((TODAY, Some((0, 0)), "12am")));
        assert_eq!(at("12pm"), Some((TODAY, Some((12, 0)), "12pm")));
        assert_eq!(at("at noon"), Some((TODAY, Some((12, 0)), "at noon")));
        assert_eq!(at("every monday 8am").map(|d| d.1), Some(Some((8, 0))));
        assert_eq!(at("fri 4 apples"), Some((day("2026-10-09"), None, "fri")), "a bare number is not a time");
        for t in ["13pm", "25:00", "9:5", "at", "4"] {
            assert_eq!(parse(t, TODAY).and_then(|d| d.time), None, "{t}");
        }
    }

    #[test]
    fn only_the_date_is_taken_and_its_length_is_reported() {
        let text = "tomorrow call mum";
        assert_eq!(parse(text, TODAY).unwrap().len, "tomorrow".len());
        let text = "next friday, then";
        let due = parse(text, TODAY).unwrap();
        assert_eq!(&text[..due.len], "next friday");
        let text = "oct 12th.";
        assert_eq!(&text[..parse(text, TODAY).unwrap().len], "oct 12th");
    }

    #[test]
    fn words_that_are_not_dates_are_left_alone() {
        for t in ["", "soon", "mayor", "marching", "fridays off", "in", "every", "13 14", "oct", "may 40"] {
            assert_eq!(on(t), None, "{t:?}");
        }
    }

    #[test]
    fn a_task_line_gives_its_dates_after_the_at_sign() {
        let line = "- [ ] pay rent @fri and mail me@oct 12 then @every week";
        let found = find_all(line, TODAY);
        let at: Vec<usize> = found.iter().map(|(i, _)| *i).collect();
        assert_eq!(at, vec![line.find("@fri").unwrap(), line.find("@every").unwrap()], "an @ inside an address is not a date");
        assert_eq!(found[0].1.day, day("2026-10-09"));
        assert_eq!(found[1].1.repeat, Some(Repeat::Weekly));
    }

    #[test]
    fn non_ascii_text_cannot_split_a_character() {
        assert_eq!(on("غدا"), None);
        assert_eq!(on("tomorrow ✓"), Some(day("2026-10-06")));
        assert_eq!(find_all("@é", TODAY).len(), 0);
    }
}
