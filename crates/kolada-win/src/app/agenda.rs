//! The Today card (see `kolada_ui::agenda`): what is due across the vault,
//! opened by the teal dot beside the note's dates, Ctrl+T or the palette, and
//! closed like a menu. Ticking a task in it ticks it in its note, open or
//! not.

use std::collections::HashMap;

use kolada_core::tasks::{self, Agenda, Task};
use kolada_ui::agenda::{AgendaView, Row};

use super::*;

/// The card's spring: quick, a touch past and back.
const OMEGA: f32 = 24.0;
const ZETA: f32 = 0.8;

#[derive(Default)]
pub(super) struct State {
    /// Tasks with dates, by note (from the index, and each note as saved).
    by_note: Due,
    /// Late or due today: the teal dot shows when there are any.
    pub(super) now: usize,
    sheet: Option<Sheet>,
    /// Timed tasks already announced today: their note, line and time.
    announced: Vec<(String, usize, (u32, u32))>,
    /// The tray icon was put there only for a notification.
    pub(super) icon_for_now: bool,
}

/// Looks at the clock once a minute for what to remind of.
pub(super) const REMIND_TIMER: usize = 48;
/// The morning nudge comes at this hour, or when Kolada first runs after it.
const MORNING: u32 = 9;
/// A timed task is announced if its time came at most this many minutes ago.
const LATE_ANNOUNCE: u32 = 15;

/// The day the morning nudge was last given (kept in the app's data, so it
/// comes once a day however often Kolada starts).
fn nudged_on() -> Option<String> {
    let path = kolada_core::app_data_dir()?.join("nudged.txt");
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn set_nudged_on(day: &str) {
    if let Some(dir) = kolada_core::app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("nudged.txt"), day);
    }
}

/// The morning nudge's words: how many, and the first few.
pub(super) fn nudge_words(names: &[&str]) -> (String, String) {
    let title = if names.len() == 1 { "1 thing today".to_string() } else { format!("{} things today", names.len()) };
    let mut text = names.iter().take(3).copied().collect::<Vec<_>>().join(", ");
    if names.len() > 3 {
        text.push_str(&format!(" and {} more", names.len() - 3));
    }
    (title, text)
}

struct Sheet {
    /// How far down (0..1, a spring may take it past), and how fast.
    open: f32,
    v: f32,
    /// Where it is going: down, or back up and away.
    down: bool,
    tasks: Vec<Task>,
    rows: Vec<Row>,
    selected: Option<usize>,
}

/// Every note's dated tasks, by note.
pub(super) type Due = HashMap<String, Vec<Task>>;

/// The dated tasks in `notes` (read on the indexing worker, not the
/// window's thread: the whole vault's text goes by).
pub(super) fn due_in(notes: &[search::Indexed]) -> Due {
    let today = daily::today();
    notes
        .iter()
        .filter(|n| search::is_note(&n.path) && n.text().contains('@'))
        .map(|n| (n.path.clone(), tasks::in_note(&n.path, n.text(), today)))
        .filter(|(_, t)| !t.is_empty())
        .collect()
}

impl App {
    /// Every note's dated tasks, from the index just read.
    pub(super) fn due_from_index(&mut self, due: Due) {
        self.agenda.by_note = due;
        self.refresh_due();
        // The clock is looked at once a minute from now on.
        if !self.compact {
            self.start_timer(REMIND_TIMER, 60_000);
            self.remind();
        }
    }

    /// Reminds of what is due, as the option allows: once in the morning
    /// (from 9:00) when anything is late or due today, and as a timed
    /// task's time comes.
    pub(super) fn remind(&mut self) {
        if self.session.remind != 0 || self.compact {
            return;
        }
        let now = daily::now();
        let (today, clock) = (daily::today(), (u32::from(now.hour), u32::from(now.minute)));
        let all: Vec<Task> = self.agenda.by_note.values().flatten().cloned().collect();
        let agenda = Agenda::of(all, today);
        // A timed task whose time has just come.
        let minutes = |(h, m): (u32, u32)| h * 60 + m;
        for t in &agenda.today {
            let Some(time) = t.time else { continue };
            let key = (t.path.clone(), t.line, time);
            let since = minutes(clock).checked_sub(minutes(time));
            if since.is_some_and(|s| s <= LATE_ANNOUNCE) && !self.agenda.announced.contains(&key) {
                self.agenda.announced.push(key);
                let note = t.path.rsplit('/').next().unwrap_or(&t.path).trim_end_matches(".md").to_string();
                let when = kolada_ui::agenda::when_words(t.day, t.time, today);
                self.notify(&t.text, &format!("{when} · {note}"));
                return;
            }
        }
        // The morning's: once a day.
        let day = today.iso();
        if u32::from(now.hour) >= MORNING && agenda.now() > 0 && nudged_on().as_deref() != Some(day.as_str()) {
            set_nudged_on(&day);
            let names: Vec<&str> = agenda.late.iter().chain(&agenda.today).map(|t| t.text.as_str()).collect();
            let (title, text) = nudge_words(&names);
            self.notify(&title, &text);
        }
    }

    /// The open note's tasks as it now reads, then the count and the sheet.
    pub(super) fn refresh_due(&mut self) {
        let today = daily::today();
        if !self.session.note.is_empty() {
            let mine = tasks::in_note(&self.session.note, self.editor.text(), today);
            if mine.is_empty() {
                self.agenda.by_note.remove(&self.session.note);
            } else {
                self.agenda.by_note.insert(self.session.note.clone(), mine);
            }
        }
        // Read again as of today: a repeat's turn and what counts as late move with the day.
        let all: Vec<Task> = self.agenda.by_note.values().flatten().cloned().collect();
        let agenda = Agenda::of(all, today);
        let now = agenda.now();
        if now != self.agenda.now {
            self.agenda.now = now;
            self.request_frame();
        }
        if let Some(s) = &mut self.agenda.sheet {
            s.rows = kolada_ui::agenda::rows(&agenda, today);
            s.tasks = agenda.late.into_iter().chain(agenda.today).chain(agenda.coming).collect();
            s.selected = s.selected.filter(|&i| i < s.rows.len()).or(if s.rows.is_empty() { None } else { Some(0) });
            self.request_frame();
        }
    }

    pub(super) fn agenda_down(&self) -> bool {
        self.agenda.sheet.as_ref().is_some_and(|s| s.down)
    }

    pub(super) fn open_agenda(&mut self) {
        if self.agenda.sheet.is_none() {
            self.agenda.sheet = Some(Sheet { open: 0.0, v: 0.0, down: true, tasks: Vec::new(), rows: Vec::new(), selected: None });
        }
        if let Some(s) = &mut self.agenda.sheet {
            s.down = true;
        }
        self.refresh_due();
        self.request_frame();
    }

    pub(super) fn close_agenda(&mut self) {
        if let Some(s) = &mut self.agenda.sheet {
            s.down = false;
        }
        self.request_frame();
    }

    pub(super) fn toggle_agenda(&mut self) {
        if self.agenda_down() { self.close_agenda() } else { self.open_agenda() }
    }

    /// The wheel turned: an open card closes, and the page scrolls on.
    pub(super) fn wheel_agenda(&mut self) {
        if self.agenda_down() {
            self.close_agenda();
        }
    }

    /// One frame of the sheet's motion; whether it is still moving.
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

    pub(super) fn agenda_moving(&self) -> bool {
        self.agenda.sheet.as_ref().is_some_and(|s| s.open != if s.down { 1.0 } else { 0.0 } || s.v != 0.0)
    }

    pub(super) fn agenda_view(&self) -> Option<AgendaView<'_>> {
        let s = self.agenda.sheet.as_ref()?;
        Some(AgendaView { open: s.open, rows: &s.rows, now: s.rows.iter().filter(|r| r.part != kolada_ui::agenda::Part::Coming).count(), selected: s.selected.filter(|_| s.down) })
    }

    /// Keys while the sheet is down: Up and Down choose, Enter opens the
    /// task's note, Ctrl+Enter or Space ticks it, Escape lets it go. Whether
    /// the key was the sheet's.
    pub(super) fn agenda_key(&mut self, vk: u16, ctrl: bool) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SPACE, VK_UP};
        if !self.agenda_down() {
            return false;
        }
        let k = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| key.0 == vk;
        let Some(s) = &mut self.agenda.sheet else { return false };
        let n = s.rows.len();
        if k(VK_ESCAPE) {
            self.close_agenda();
        } else if k(VK_DOWN) && n > 0 {
            s.selected = Some(s.selected.map_or(0, |i| (i + 1).min(n - 1)));
        } else if k(VK_UP) && n > 0 {
            s.selected = Some(s.selected.map_or(0, |i| i.saturating_sub(1)));
        } else if k(VK_RETURN) && ctrl || k(VK_SPACE) {
            if let Some(i) = s.selected {
                self.tick_due(i);
            }
        } else if k(VK_RETURN) {
            if let Some(i) = s.selected {
                self.go_to_due(i);
            }
        } else {
            // Typing lets it go, and goes to the note.
            use windows::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};
            if ![VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN].iter().any(|m| k(*m)) {
                self.close_agenda();
            }
            return false;
        }
        self.request_frame();
        true
    }

    /// Opens task `i`'s note, the caret at the end of its line.
    pub(super) fn go_to_due(&mut self, i: usize) {
        let Some(t) = self.agenda.sheet.as_ref().and_then(|s| s.tasks.get(i)).cloned() else { return };
        self.close_agenda();
        if self.session.note != t.path {
            let path = Path::new(&self.session.vault).join(&t.path);
            self.open(&path);
        }
        if self.session.note == t.path {
            let text = self.editor.text();
            let end = text.get(t.line..).and_then(|r| r.find('\n')).map_or(text.len(), |e| t.line + e);
            self.editor.set_selection(Selection::caret(end.min(text.len())));
            self.caret_moved(false);
        }
    }

    /// Ticks task `i` in its note: through the editor when it is the open
    /// note (one undo step), else in its file.
    pub(super) fn tick_due(&mut self, i: usize) {
        let Some(t) = self.agenda.sheet.as_ref().and_then(|s| s.tasks.get(i)).cloned() else { return };
        let today = daily::today();
        if t.path == self.session.note {
            match tasks::tick(self.editor.text(), t.line, today) {
                Some(edits) => self.edit(|e| e.replace_ranges(&edits)),
                None => self.whisper("That task has changed since".to_string()),
            }
            self.save_note(false);
        } else {
            let file = Path::new(&self.session.vault).join(&t.path);
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
                None => self.whisper("That task has changed since".to_string()),
            }
        }
        self.refresh_due();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_morning_nudge_says_how_many_and_names_the_first_few() {
        assert_eq!(nudge_words(&["Pay rent"]), ("1 thing today".to_string(), "Pay rent".to_string()));
        assert_eq!(
            nudge_words(&["Pay rent", "Call mum", "Dentist", "Water plants", "Invoice"]),
            ("5 things today".to_string(), "Pay rent, Call mum, Dentist and 2 more".to_string())
        );
    }
}
