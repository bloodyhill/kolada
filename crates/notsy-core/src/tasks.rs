//! Tasks with dates: `- [ ] Pay rent @fri`. What is due is read from the
//! notes themselves, so the vault stays plain Markdown any app can read.
//! Ticking a repeating task (`@every monday`) keeps it open and writes a
//! ticked copy above it, dated: `- [x] Water plants ✓ 2026-10-12`. Its next
//! turn counts from the last such copy.

use crate::when::{self, Day, Repeat};

/// A task due on a day, found in a note.
#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    /// The note's vault-relative path.
    pub path: String,
    /// Where its line starts in the note's text.
    pub line: usize,
    /// What it says, without its checkbox and dates.
    pub text: String,
    pub day: Day,
    pub time: Option<(u32, u32)>,
    pub repeat: Option<Repeat>,
}

/// Marks a ticked copy of a repeating task, before the day it was done.
const DONE_ON: &str = "\u{2713} ";

/// A task line: where its text starts (after the box), and whether it is ticked.
fn task_line(line: &str) -> Option<(usize, bool)> {
    let rest = line.trim_start();
    let indent = line.len() - rest.len();
    let after_mark = if let Some(r) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")).or_else(|| rest.strip_prefix("+ ")) {
        r
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        rest[digits..].strip_prefix(". ").or_else(|| rest[digits..].strip_prefix(") ")).filter(|_| digits > 0)?
    };
    let done = match after_mark.get(..3)? {
        "[ ]" => false,
        "[x]" | "[X]" => true,
        _ => return None,
    };
    let text = indent + (rest.len() - after_mark.len()) + 3;
    Some((text + usize::from(line[text..].starts_with(' ')), done))
}

/// A task's words without its dates and done mark, spaces closed up.
fn words_of(text: &str, today: Day) -> String {
    let mut out = String::new();
    let mut at = 0;
    for (i, due) in when::find_all(text, today) {
        out.push_str(&text[at..i]);
        at = i + 1 + due.len;
    }
    out.push_str(&text[at..]);
    let out = out.split(DONE_ON.trim_end()).next().unwrap_or("");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The open tasks with a date in note `path`'s `text`, each on its next turn.
pub fn in_note(path: &str, text: &str, today: Day) -> Vec<Task> {
    let mut tasks = Vec::new();
    let mut fenced = false;
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let at = start;
        start += line.len();
        let line = line.trim_end_matches(['\n', '\r']);
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let Some((body, false)) = task_line(line).filter(|_| !fenced) else { continue };
        let Some((_, due)) = when::find_all(&line[body..], today).into_iter().next() else { continue };
        let words = words_of(&line[body..], today);
        let day = match due.repeat {
            Some(r) => last_done(text, &words, today).map_or(due.day, |d| r.after(d)),
            None => due.day,
        };
        tasks.push(Task { path: path.to_string(), line: at, text: words, day, time: due.time, repeat: due.repeat });
    }
    tasks
}

/// The last day a repeating task with these words was ticked, from its
/// ticked copies in the note.
fn last_done(text: &str, words: &str, today: Day) -> Option<Day> {
    text.lines()
        .filter_map(|l| {
            let (body, true) = task_line(l)? else { return None };
            let (said, on) = l[body..].rsplit_once(DONE_ON.trim_end())?;
            let on = on.trim();
            let day = Day::new(on.get(..4)?.parse().ok()?, on.get(5..7)?.parse().ok()?, on.get(8..10)?.parse().ok()?)?;
            (words_of(said, today) == words).then_some(day)
        })
        .max()
}

/// What is due, as a Today sheet shows it: late, today (by time, then as
/// written), and the coming week.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Agenda {
    pub late: Vec<Task>,
    pub today: Vec<Task>,
    pub coming: Vec<Task>,
}

/// Days ahead the agenda looks.
pub const COMING_DAYS: i64 = 7;

impl Agenda {
    pub fn of(tasks: impl IntoIterator<Item = Task>, today: Day) -> Agenda {
        let mut a = Agenda::default();
        let last = today.plus_days(COMING_DAYS);
        for t in tasks {
            match t.day {
                d if d < today => a.late.push(t),
                d if d == today => a.today.push(t),
                d if d <= last => a.coming.push(t),
                _ => {}
            }
        }
        let order = |a: &Task, b: &Task| (a.day, a.time.is_none(), a.time).cmp(&(b.day, b.time.is_none(), b.time));
        a.late.sort_by(order);
        a.today.sort_by(order);
        a.coming.sort_by(order);
        a
    }

    /// How many are late or due today.
    pub fn now(&self) -> usize {
        self.late.len() + self.today.len()
    }
}

/// The edits that tick the task whose line starts at `line` in `text`
/// (start, end, replacement): its box, or for a repeating task a ticked
/// copy written above it with today's date, the task kept open for its next
/// turn. None when no open task starts there.
pub fn tick(text: &str, line: usize, today: Day) -> Option<Vec<(usize, usize, String)>> {
    let end = text[line..].find('\n').map_or(text.len(), |e| line + e);
    let this = text.get(line..end)?;
    let (body, false) = task_line(this)? else { return None };
    let open = this[..body].rfind("[ ]")?;
    let repeats = when::find_all(&this[body..], today).iter().any(|(_, d)| d.repeat.is_some());
    if !repeats {
        return Some(vec![(line + open + 1, line + open + 2, "x".to_string())]);
    }
    let copy = format!("{}[x] {} {DONE_ON}{}\n", &this[..open], words_of(&this[body..], today), today.iso());
    Some(vec![(line, line, copy)])
}

/// The edits that write each open task's date down for good (see
/// `when::written`), so `@fri` still means this Friday once Friday is gone
/// and a missed task shows as late. The line starting at `skip` (the one
/// being typed in) is left for now. Repeating dates stay as they are.
pub fn settle(text: &str, today: Day, skip: Option<usize>) -> Vec<(usize, usize, String)> {
    let mut edits = Vec::new();
    let mut fenced = false;
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let at = start;
        start += line.len();
        let line = line.trim_end_matches(['\n', '\r']);
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if fenced || Some(at) == skip {
            continue;
        }
        let Some((body, false)) = task_line(line) else { continue };
        for (i, due) in when::find_all(&line[body..], today) {
            let words = &line[body + i + 1..body + i + 1 + due.len];
            if due.repeat.is_none() && !when::is_fixed(words, today) {
                let from = at + body + i + 1;
                edits.push((from, from + due.len, when::written(due.day, due.time)));
            }
        }
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    // Monday 2026-10-05
    const TODAY: Day = Day { year: 2026, month: 10, day: 5 };

    fn day(s: &str) -> Day {
        Day::new(s[..4].parse().unwrap(), s[5..7].parse().unwrap(), s[8..].parse().unwrap()).unwrap()
    }

    fn apply(text: &str, edits: &[(usize, usize, String)]) -> String {
        let mut out = text.to_string();
        for (a, z, s) in edits.iter().rev() {
            out.replace_range(*a..*z, s);
        }
        out
    }

    const NOTE: &str = "# Moving\n- [x] Book the van @oct 1\n- [ ] Pay deposit @today\n- [ ] Call the landlord @fri 4pm\n* [ ] Water plants @every monday\n1. [ ] Old bill @oct 1\n- [ ] No date here\n- [ ] Far off @dec 25\n```\n- [ ] in code @today\n```\nSay hi @today\n";

    #[test]
    fn open_tasks_with_dates_are_found_with_their_words() {
        let tasks = in_note("Moving.md", NOTE, TODAY);
        let got: Vec<(&str, Day, Option<(u32, u32)>)> = tasks.iter().map(|t| (t.text.as_str(), t.day, t.time)).collect();
        assert_eq!(
            got,
            [
                ("Pay deposit", TODAY, None),
                ("Call the landlord", day("2026-10-09"), Some((16, 0))),
                ("Water plants", TODAY, None),
                ("Old bill", day("2027-10-01"), None),
                ("Far off", day("2026-12-25"), None),
            ]
        );
        assert_eq!(&NOTE[tasks[0].line..tasks[0].line + 5], "- [ ]");
    }

    #[test]
    fn the_agenda_sorts_late_today_and_the_coming_week() {
        let mut tasks = in_note("Moving.md", NOTE, TODAY);
        tasks.push(Task { path: "Work.md".into(), line: 0, text: "Send invoice".into(), day: day("2026-10-03"), time: None, repeat: None });
        tasks.push(Task { path: "Health.md".into(), line: 0, text: "Dentist".into(), day: TODAY, time: Some((9, 0)), repeat: None });
        let a = Agenda::of(tasks, TODAY);
        let names = |v: &[Task]| v.iter().map(|t| t.text.clone()).collect::<Vec<_>>();
        assert_eq!(names(&a.late), ["Send invoice"]);
        assert_eq!(names(&a.today), ["Dentist", "Pay deposit", "Water plants"], "a time first");
        assert_eq!(names(&a.coming), ["Call the landlord"], "not December");
        assert_eq!(a.now(), 4);
    }

    #[test]
    fn ticking_a_task_ticks_its_box() {
        let line = NOTE.find("- [ ] Pay").unwrap();
        let after = apply(NOTE, &tick(NOTE, line, TODAY).unwrap());
        assert!(after.contains("- [x] Pay deposit @today\n"));
        assert!(in_note("m.md", &after, TODAY).iter().all(|t| t.text != "Pay deposit"));
        assert_eq!(tick(NOTE, NOTE.find("- [x] Book").unwrap(), TODAY), None, "already ticked");
        assert_eq!(tick(NOTE, 3, TODAY), None, "not a line's start");
    }

    #[test]
    fn a_repeating_task_stays_open_and_its_next_turn_follows_the_tick() {
        let line = NOTE.find("* [ ] Water").unwrap();
        let after = apply(NOTE, &tick(NOTE, line, TODAY).unwrap());
        assert!(after.contains("* [x] Water plants \u{2713} 2026-10-05\n* [ ] Water plants @every monday\n"), "{after}");
        let water = in_note("m.md", &after, TODAY).into_iter().find(|t| t.text == "Water plants").unwrap();
        assert_eq!(water.day, day("2026-10-12"), "done today: next Monday");
        // Not ticked for two weeks: it is late from its last turn.
        let later = in_note("m.md", &after, day("2026-10-27")).into_iter().find(|t| t.text == "Water plants").unwrap();
        assert_eq!(later.day, day("2026-10-12"));
        let daily = "- [x] Stretch \u{2713} 2026-10-04\n- [ ] Stretch @every day\n";
        assert_eq!(in_note("d.md", daily, TODAY)[0].day, TODAY);
    }

    #[test]
    fn dates_are_written_down_for_good_once_their_line_is_left() {
        let note = "- [ ] Pay @fri, then rest\n- [ ] Call @today 4pm\n- [ ] Plants @every monday\n- [ ] Fixed @Fri 9 Oct 2026\n- [x] Done @fri\nSay @fri\n";
        let typing = note.find("- [ ] Call").unwrap();
        let after = apply(note, &settle(note, TODAY, Some(typing)));
        assert_eq!(after, note.replacen("@fri,", "@Fri 9 Oct 2026,", 1), "the line being typed in waits");
        let after = apply(note, &settle(note, TODAY, None));
        assert!(after.contains("- [ ] Call @Mon 5 Oct 2026 4pm\n") && after.contains("@every monday") && after.contains("- [x] Done @fri\nSay @fri\n"), "{after}");
        assert!(settle(&after, TODAY, None).is_empty(), "once is enough");
        // A week later the missed one is late, not next Friday's.
        let late = in_note("m.md", &after, day("2026-10-16"));
        assert_eq!(late[0].day, day("2026-10-09"));
    }

    #[test]
    fn task_lines_are_known_by_their_marks() {
        assert_eq!(task_line("- [ ] a"), Some((6, false)));
        assert_eq!(task_line("\t- [X] a"), Some((7, true)));
        assert_eq!(task_line("12. [ ] a"), Some((8, false)));
        assert_eq!(task_line("- [ ]"), Some((5, false)));
        for no in ["- a", "[ ] a", "-[ ] a", "x. [ ] a", "- [-] a"] {
            assert_eq!(task_line(no), None, "{no}");
        }
    }
}
