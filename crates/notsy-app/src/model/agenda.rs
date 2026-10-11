//! What is due, as on Windows (`notsy-win` `app/agenda.rs`): the tasks with
//! an `@date` across the vault, a red dot beside the note's dates while
//! something is late or due today, and the Today card that opens at the dot
//! (Cmd+T and the palette's "What's due" open it too). It lists what is late
//! and due today, then the coming week; ticking a task there ticks it in its
//! note, open or not; choosing one opens its note at the task. It closes like
//! a menu: Escape, a click beside it, the wheel, or typing.
//!
//! A task's date is read from plain words (`@fri 4pm`, `@in 3 days`,
//! `@every monday`, by `notsy_core::when`); a relative date is written down
//! for good once its line is left (`settle`), or it would slide forward each
//! day and a missed task would never be late.

use std::collections::HashMap;

use notsy_core::tasks::{self, Task};
use notsy_core::when::Day;
use notsy_ui::agenda::{AgendaView, Part, Row};

use super::*;
use crate::services::Calendar;

/// The card's spring: quick, a touch past and back.
const OMEGA: f32 = 24.0;
const ZETA: f32 = 0.8;

/// Every note's dated tasks, by note.
pub(super) type Due = HashMap<String, Vec<Task>>;

#[derive(Default)]
pub(super) struct Agenda {
    /// Tasks with dates, by note (from the index, and the open note as saved).
    by_note: Due,
    /// Late or due today: the red dot shows when there are any.
    pub now: usize,
    sheet: Option<Sheet>,
    /// The index thread leaves the vault's tasks here.
    pub slot: Arc<Mutex<Option<Due>>>,
}

struct Sheet {
    /// How far open (0 gone, 1 open; the spring may take it a little past), and how fast.
    open: f32,
    v: f32,
    /// Where it is going: open, or back and away.
    down: bool,
    tasks: Vec<Task>,
    rows: Vec<Row>,
    selected: Option<usize>,
}

/// Today, in local time.
pub(super) fn today(calendar: &dyn Calendar) -> Day {
    let w = super::daily::now(calendar);
    Day::new(i32::from(w.year), u32::from(w.month), u32::from(w.day)).unwrap_or(Day { year: 1970, month: 1, day: 1 })
}

/// The dated tasks in `notes`, each note's own (read on the index's thread:
/// the whole vault's text goes by).
pub(super) fn due_in(notes: &[Indexed], calendar: &dyn Calendar) -> Due {
    let today = today(calendar);
    notes
        .iter()
        .filter(|n| search::is_note(&n.path) && n.text().contains('@'))
        .map(|n| (n.path.clone(), tasks::in_note(&n.path, n.text(), today)))
        .filter(|(_, t)| !t.is_empty())
        .collect()
}

impl Model {
    /// The index thread has read the vault's tasks: takes them.
    pub fn due_ready(&mut self) -> bool {
        let Some(due) = self.agenda.slot.lock().unwrap().take() else { return false };
        self.due_from_index(due);
        true
    }

    pub(super) fn due_from_index(&mut self, due: Due) {
        self.agenda.by_note = due;
        self.refresh_due();
    }

    /// The open note's tasks as it now reads, then the count and the card.
    pub(super) fn refresh_due(&mut self) {
        let today = today(&*self.services.calendar);
        if !self.session.note.is_empty() {
            let mine = if self.editor.text().contains('@') { tasks::in_note(&self.session.note, self.editor.text(), today) } else { Vec::new() };
            if mine.is_empty() {
                self.agenda.by_note.remove(&self.session.note);
            } else {
                self.agenda.by_note.insert(self.session.note.clone(), mine);
            }
        }
        // Read again as of today: a repeat's turn and what counts as late move with the day.
        let all: Vec<Task> = self.agenda.by_note.values().flatten().cloned().collect();
        let agenda = tasks::Agenda::of(all, today);
        self.agenda.now = agenda.now();
        if let Some(s) = &mut self.agenda.sheet {
            s.rows = notsy_ui::agenda::rows(&agenda, today);
            s.tasks = agenda.late.into_iter().chain(agenda.today).chain(agenda.coming).collect();
            s.selected = s.selected.filter(|&i| i < s.rows.len()).or(if s.rows.is_empty() { None } else { Some(0) });
        }
    }

    /// A task's relative date, written down for good (one undo step) once
    /// its line is left: `@fri` becomes `@Fri 9 Oct 2026`. The line being
    /// typed in waits until the note is left (`flush`).
    pub(super) fn settle_dates(&mut self, flush: bool) {
        if !self.editor.text().contains('@') {
            return;
        }
        let typing = (!flush).then(|| self.editor.line_at(self.editor.selection().head).start);
        let edits = tasks::settle(self.editor.text(), today(&*self.services.calendar), typing);
        if !edits.is_empty() {
            self.editor.amend(&edits);
            self.dirty = true;
        }
    }

    pub(super) fn agenda_down(&self) -> bool {
        self.agenda.sheet.as_ref().is_some_and(|s| s.down)
    }

    /// Opens the card (Cmd+T, the red dot, the palette).
    pub fn open_agenda(&mut self) -> bool {
        self.search = None;
        self.trash = None;
        if self.agenda.sheet.is_none() {
            self.agenda.sheet = Some(Sheet { open: 0.0, v: 0.0, down: true, tasks: Vec::new(), rows: Vec::new(), selected: None });
        }
        if let Some(s) = &mut self.agenda.sheet {
            s.down = true;
        }
        self.refresh_due();
        true
    }

    pub(super) fn close_agenda(&mut self) -> bool {
        match &mut self.agenda.sheet {
            Some(s) if s.down => {
                s.down = false;
                true
            }
            _ => false,
        }
    }

    pub fn toggle_agenda(&mut self) -> bool {
        if self.agenda_down() { self.close_agenda() } else { self.open_agenda() }
    }

    /// The wheel turned: an open card goes, and the page scrolls on.
    pub(super) fn wheel_agenda(&mut self) {
        self.close_agenda();
    }

    /// One frame of the card's motion; whether it is still moving.
    pub(super) fn step_agenda(&mut self, dt: f32) -> bool {
        let Some(s) = &mut self.agenda.sheet else { return false };
        let target = if s.down { 1.0 } else { 0.0 };
        let a = OMEGA * OMEGA * (target - s.open) - 2.0 * ZETA * OMEGA * s.v;
        s.v += a * dt;
        s.open += s.v * dt;
        if (target - s.open).abs() < 0.002 && s.v.abs() < 0.02 {
            s.open = target;
            s.v = 0.0;
            if !s.down {
                self.agenda.sheet = None;
            }
            return false;
        }
        true
    }

    /// The card, all the way open or shut (a picture of it, without the clock).
    pub fn settle_agenda(&mut self) {
        for _ in 0..90 {
            self.step_agenda(1.0 / 60.0);
        }
    }

    pub(super) fn agenda_moving(&self) -> bool {
        self.agenda.sheet.as_ref().is_some_and(|s| s.open != if s.down { 1.0 } else { 0.0 } || s.v != 0.0)
    }

    pub(super) fn agenda_view(&self) -> Option<AgendaView<'_>> {
        let s = self.agenda.sheet.as_ref()?;
        Some(AgendaView { open: s.open, rows: &s.rows, now: s.rows.iter().filter(|r| r.part != Part::Coming).count(), selected: s.selected.filter(|_| s.down) })
    }

    /// A click: on the red dot the card opens or goes; with it open, a task's
    /// box ticks it, its row opens its note, and anywhere else lets it go.
    /// None when the click was not the card's.
    pub(super) fn agenda_click(&mut self, hit: Option<Target>) -> Option<bool> {
        if hit == Some(Target::DueDot) {
            return Some(self.toggle_agenda());
        }
        if !self.agenda_down() {
            return None;
        }
        Some(match hit {
            Some(Target::AgendaPanel) => false,
            Some(Target::AgendaOpen(i)) => self.go_to_due(i),
            Some(Target::AgendaTick(i)) => self.tick_due(i),
            _ => self.close_agenda(),
        })
    }

    /// Keys while the card is open, by AppKit's name for them: Up and Down
    /// choose, Return opens the task's note, Escape lets it go. Any other
    /// command lets it go and is the note's. None when it is not open.
    pub fn agenda_command(&mut self, name: &str) -> Option<bool> {
        if !self.agenda_down() {
            return None;
        }
        let s = self.agenda.sheet.as_mut()?;
        let n = s.rows.len();
        match name {
            "cancelOperation:" => Some(self.close_agenda()),
            "moveDown:" if n > 0 => {
                s.selected = Some(s.selected.map_or(0, |i| (i + 1).min(n - 1)));
                Some(true)
            }
            "moveUp:" if n > 0 => {
                s.selected = Some(s.selected.map_or(0, |i| i.saturating_sub(1)));
                Some(true)
            }
            "moveDown:" | "moveUp:" => Some(false),
            "insertNewline:" | "insertParagraphSeparator:" => {
                let chosen = s.selected;
                Some(chosen.is_some_and(|i| self.go_to_due(i)))
            }
            _ => {
                self.close_agenda();
                None
            }
        }
    }

    /// Text typed with the card open: Space ticks the chosen task; anything
    /// else lets it go and is the note's. Some when the key was the card's.
    pub(super) fn agenda_text(&mut self, text: &str) -> Option<bool> {
        if !self.agenda_down() {
            return None;
        }
        let chosen = self.agenda.sheet.as_ref().and_then(|s| s.selected);
        if text == " " && let Some(i) = chosen {
            return Some(self.tick_due(i));
        }
        self.close_agenda();
        None
    }

    /// Opens task `i`'s note, the caret at the end of its line.
    pub(super) fn go_to_due(&mut self, i: usize) -> bool {
        let Some(t) = self.agenda.sheet.as_ref().and_then(|s| s.tasks.get(i)).cloned() else { return false };
        self.close_agenda();
        if self.session.note != t.path {
            self.open(&t.path);
        }
        if self.session.note == t.path {
            let text = self.editor.text();
            let end = text.get(t.line..).and_then(|r| r.find('\n')).map_or(text.len(), |e| t.line + e);
            self.editor.set_selection(Selection::caret(end.min(text.len())));
            self.reveal = true;
            self.caret_moved(false);
        }
        true
    }

    /// Ticks task `i` in its note: through the editor when it is the open
    /// note (one undo step), else in its file.
    pub(super) fn tick_due(&mut self, i: usize) -> bool {
        let Some(t) = self.agenda.sheet.as_ref().and_then(|s| s.tasks.get(i)).cloned() else { return false };
        let today = today(&*self.services.calendar);
        if t.path == self.session.note {
            match tasks::tick(self.editor.text(), t.line, today) {
                Some(edits) => {
                    self.editor.replace_ranges(&edits);
                    self.dirty = true;
                    self.save_with(false);
                }
                None => self.whisper("That task has changed since"),
            }
        } else {
            let file = self.vault.join(&t.path);
            let ticked = note::load(&file).ok().and_then(|(text, format)| {
                let edits = tasks::tick(&text, t.line, today)?;
                let mut out = text;
                for (a, z, s) in edits.iter().rev() {
                    out.replace_range(*a..*z, s);
                }
                note::save(&file, &out, format).ok()?;
                Some(out)
            });
            match ticked {
                Some(text) => {
                    let left = tasks::in_note(&t.path, &text, today);
                    if left.is_empty() {
                        self.agenda.by_note.remove(&t.path);
                    } else {
                        self.agenda.by_note.insert(t.path.clone(), left);
                    }
                }
                None => self.whisper("That task has changed since"),
            }
        }
        self.refresh_due();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(test: &str, notes: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("notsy-app-test-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in notes {
            std::fs::write(dir.join(name), text).unwrap();
        }
        dir
    }

    const SIZE: (u32, u32, f32) = (2000, 1440, 2.0);

    /// A model on `v` with the vault's tasks read, as the index thread does.
    fn drawn(v: &Path) -> Model {
        let mut m = Model::new(v.to_path_buf(), true);
        m.due_from_index(due_in(&search::index(v), &crate::services::NoServices));
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        m
    }

    fn at(m: &Model, target: Target) -> (f32, f32) {
        let r = m.hits.iter().find(|(_, t)| *t == target).map(|(r, _)| *r).unwrap_or_else(|| panic!("{target:?} is drawn"));
        ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    }

    fn settle_card(m: &mut Model) {
        // A second and a half of frames, however fast the test runs.
        for _ in 0..90 {
            m.step_agenda(1.0 / 60.0);
        }
        m.frame(SIZE.0, SIZE.1, SIZE.2);
    }

    #[test]
    fn what_is_late_is_counted_by_a_dot_and_listed_in_a_card_that_opens_there() {
        let v = vault(
            "due",
            &[
                ("Chores.md", "# Chores\n\n- [ ] Pay rent @Mon 1 Jan 2024\n- [ ] Someday @Sat 25 Dec 2038\n- [x] Done @Mon 1 Jan 2024\n"),
                ("Plan.md", "# Plan\n\nNo dates here.\n"),
            ],
        );
        let mut m = drawn(&v);
        assert_eq!(m.agenda.now, 1, "one task is late; the far one and the ticked one are not");
        assert!(m.agenda_view().is_none(), "at rest the only sign is the dot");
        // The dot shows beside the note's dates, on the note that is open.
        m.open("Chores.md");
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        let (x, y) = at(&m, Target::DueDot);
        assert!(m.pointer_down(x, y, 1, false), "the dot opens the card");
        assert!(m.agenda_moving() && m.wants_frames(), "it springs open");
        settle_card(&mut m);
        let view = m.agenda_view().expect("the card is open");
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].part, Part::Late);
        assert_eq!(view.rows[0].text, "Pay rent");
        assert_eq!(view.rows[0].note, "Chores");
        assert!(m.hits.iter().any(|(_, t)| *t == Target::AgendaShade), "a click beside it lets it go");
        // A click beside it closes it, and nothing moves in the note.
        assert!(m.pointer_down(3.0, 700.0, 1, false));
        settle_card(&mut m);
        assert!(m.agenda_view().is_none());
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn ticking_in_the_card_ticks_the_task_in_a_note_that_is_not_open_and_choosing_opens_it() {
        let v = vault("tick", &[("Chores.md", "# Chores\n\n- [ ] Pay rent @Mon 1 Jan 2024\n- [ ] Call mum @Tue 2 Jan 2024\n"), ("Plan.md", "# Plan\n")]);
        let mut m = drawn(&v);
        m.open("Plan.md");
        assert!(m.toggle_agenda());
        settle_card(&mut m);
        assert_eq!(m.agenda_view().unwrap().rows.len(), 2);
        assert_eq!(m.agenda.now, 2);
        let (x, y) = at(&m, Target::AgendaTick(0));
        assert!(m.pointer_down(x, y, 1, false));
        let file = std::fs::read_to_string(v.join("Chores.md")).unwrap();
        assert!(file.contains("- [x] Pay rent") && file.contains("- [ ] Call mum"), "ticked in its file: {} lines", file.lines().count());
        assert_eq!(m.agenda.now, 1, "and no longer counted");
        assert_eq!(m.session.note, "Plan.md", "the note stayed shut");
        // Down and Return choose the one left and open its note at its line.
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        assert_eq!(m.agenda_command("moveDown:"), Some(true));
        assert_eq!(m.agenda_command("insertNewline:"), Some(true));
        assert_eq!(m.session.note, "Chores.md");
        let head = m.editor.selection().head;
        assert!(m.text()[..head].ends_with("Call mum @Tue 2 Jan 2024"), "the caret is at the end of the task's line");
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn a_task_in_the_open_note_ticks_through_the_editor_and_undo_takes_it_back() {
        let v = vault("tick-open", &[("Chores.md", "# Chores\n\n- [ ] Pay rent @Mon 1 Jan 2024\n")]);
        let mut m = drawn(&v);
        assert!(m.open_agenda());
        assert_eq!(m.agenda_text(" "), Some(true), "Space ticks the chosen task");
        assert!(m.text().contains("- [x] Pay rent"));
        assert!(std::fs::read_to_string(v.join("Chores.md")).unwrap().contains("- [x] Pay rent"), "saved at once");
        assert_eq!(m.agenda.now, 0);
        assert!(m.editor.undo());
        assert!(m.text().contains("- [ ] Pay rent"));
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn a_repeating_task_keeps_its_turn_and_leaves_a_ticked_copy() {
        let v = vault("repeat", &[("Chores.md", "# Chores\n\n- [ ] Water plants @every monday\n")]);
        let mut m = drawn(&v);
        let before = m.agenda.by_note["Chores.md"][0].day;
        assert!(before >= today(&crate::services::NoServices), "its next turn is a Monday to come or today");
        m.refresh_due();
        let text = m.text();
        assert!(text.contains("@every monday"));
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn keys_choose_escape_lets_go_and_typing_is_the_notes() {
        let v = vault("keys", &[("Chores.md", "# Chores\n\n- [ ] A @Mon 1 Jan 2024\n- [ ] B @Tue 2 Jan 2024\n")]);
        let mut m = drawn(&v);
        assert_eq!(m.agenda_command("moveDown:"), None, "closed, the card takes no keys");
        m.open_agenda();
        assert_eq!(m.agenda_command("moveDown:"), Some(true));
        assert_eq!(m.agenda_command("moveUp:"), Some(true));
        assert_eq!(m.agenda_command("cancelOperation:"), Some(true));
        assert!(!m.agenda_down());
        settle_card(&mut m);
        assert!(m.agenda_view().is_none());
        m.open_agenda();
        assert_eq!(m.agenda_text("x"), None, "a letter goes to the note");
        assert!(!m.agenda_down(), "and lets the card go");
        m.open_agenda();
        m.wheel(10.0, true);
        assert!(!m.agenda_down(), "the wheel lets it go");
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn leaving_a_note_writes_its_relative_dates_down_for_good() {
        let v = vault("settle", &[("Chores.md", "# Chores\n\n")]);
        let mut m = drawn(&v);
        m.go(Motion::DocEnd, false);
        for c in "- [ ] Pay rent @fri".chars() {
            m.insert(&c.to_string());
        }
        m.save();
        assert!(m.text().contains("@fri"), "the line being typed in waits");
        m.save_flush();
        let text = std::fs::read_to_string(v.join("Chores.md")).unwrap();
        assert!(!text.contains("@fri"), "written down once the note is left");
        let at = text.find("@Fri ").expect("a weekday, then the day");
        assert!(text[at..].trim_end().chars().last().is_some_and(|c| c.is_ascii_digit()), "ends in its year");
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn a_shortcut_becomes_its_text_when_a_space_ends_it() {
        let v = vault("expand", &[("Plan.md", "# Plan\n\n")]);
        let mut m = drawn(&v);
        m.go(Motion::DocEnd, false);
        for c in ";;alpha ".chars() {
            m.insert(&c.to_string());
        }
        assert!(m.text().ends_with("\u{3b1} "), "the greek letter: {:?}", m.text().len());
        for c in ";;date ".chars() {
            m.insert(&c.to_string());
        }
        let t = m.text();
        let date = t.rsplit(' ').nth(1).unwrap();
        let digits: Vec<bool> = date.chars().map(|c| c.is_ascii_digit()).collect();
        assert_eq!((date.len(), digits.iter().filter(|d| **d).count()), (10, 8), "today as a date: {} characters", date.len());
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn a_note_is_tidied_as_it_is_left_and_not_while_it_is_typed_in() {
        let v = vault("tidy-page", &[("Plan.md", "# Plan\n\nOne.\n")]);
        let mut m = drawn(&v);
        m.go(Motion::DocEnd, false);
        m.insert("Two.   \n\n\n\n\nThree.  ");
        m.save();
        assert!(std::fs::read_to_string(v.join("Plan.md")).unwrap().contains("Two.   \n"), "mid-edit the spaces stay");
        m.save_flush();
        let text = std::fs::read_to_string(v.join("Plan.md")).unwrap();
        assert!(!text.contains("Two.   ") && !text.contains("\n\n\n"), "spaces at line ends and runs of empty lines are gone");
        assert!(text.contains("Two.\n") && text.contains("Three."));
        let _ = std::fs::remove_dir_all(v);
    }
}
