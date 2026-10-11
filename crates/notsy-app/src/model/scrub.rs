//! A note's past, and the date line above its title. The past is kept in the
//! app's data as the note is written (`notsy_core::history`) and shown by
//! the time scrubber (drawn by `notsy_ui::scrub`): click the note's dates
//! or press Cmd+Shift+H, drag along the track or use the arrow keys; a
//! changed line comes back on its own, Restore (Return) brings back the
//! whole version, both one undo step. Esc returns to now. The same as the
//! Windows shell's `app/scrub.rs`, whose tests this follows.

use std::cell::Cell;
use std::time::{Duration, Instant, SystemTime};

use notsy_core::history::{self, History};
use notsy_ui::scrub::{self as scrub_ui, ScrubView};

use super::*;
use crate::when;

/// While typing goes on, a version is kept at most this often (and always
/// when the note is left).
pub(super) const KEEP_EVERY_MS: u64 = 30_000;
/// How quickly the playhead arrives at a version (rad/s, critically
/// damped), and how fast the dock fades and the page goes back (per second).
const GLIDE: f32 = 24.0;
const FADE: f32 = 8.0;
/// A note this long is not counted word by word while it is typed in.
const WORDS_AT_ONCE: usize = 64 * 1024;
const WORDS_PAUSE: Duration = Duration::from_millis(400);

pub(super) struct Scrub {
    history: History,
    /// Each version, oldest first: when, and how much it changed. The last
    /// is the note as it was when the scrubber opened.
    times: Vec<u64>,
    changes: Vec<usize>,
    /// The version shown: its text and lines, which of them differ from
    /// the note now, and when it was written.
    shown: usize,
    text: String,
    lines: Vec<notsy_markdown::Line>,
    changed: Vec<bool>,
    when: String,
    ago: String,
    since: String,
    /// The playhead's distance from the version shown (in versions), and
    /// its speed.
    offset: f32,
    velocity: f32,
    /// How far into the past the page has gone, and the dock's opacity:
    /// both follow where they are headed.
    past: f32,
    alpha: f32,
    closing: bool,
    dragging: bool,
    hover_line: Option<usize>,
    /// Where the page was scrolled when it opened, to return to.
    scroll: (usize, f32),
}

/// What the model keeps about the open note's past and dates.
pub(super) struct Past {
    pub scrub: Option<Scrub>,
    /// The scrubber is in motion: frames follow until it is still.
    pub moving: bool,
    /// The note a version was last kept for, and when.
    kept: (String, u64),
    /// When the note's file was made and last written.
    times: (Option<SystemTime>, Option<SystemTime>),
    /// The words in the note at an editor version, and when they were counted.
    words: Cell<(u64, usize, Option<Instant>)>,
    /// The dates as last written out (they take a time zone to say), and when:
    /// said again each second, not each frame.
    dates: notsy_ui::when::DateCache,
}

impl Default for Past {
    fn default() -> Past {
        Past { scrub: None, moving: false, kept: (String::new(), 0), times: (None, None), words: Cell::new((u64::MAX, 0, None)), dates: Default::default() }
    }
}

/// Milliseconds since the Unix epoch.
pub(super) fn ms(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn time_of(ms: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_millis(ms)
}

/// Moves `x` toward `to` at `rate` per second; false once there.
fn toward(x: &mut f32, to: f32, rate: f32, dt: f32) -> bool {
    let step = rate * dt;
    if (to - *x).abs() <= step {
        *x = to;
        return false;
    }
    *x += step.copysign(to - *x);
    true
}

/// A critically damped spring: `x` toward 0; false once it is at rest.
fn settle(x: &mut f32, v: &mut f32, dt: f32, omega: f32) -> bool {
    let decay = (-omega * dt).exp();
    let k = *v + omega * *x;
    *x = (*x + k * dt) * decay;
    *v = (*v - omega * k * dt) * decay;
    // A quarter DIP is under a pixel at any scale: stop there rather than
    // drawing frames nobody can see.
    if x.abs() < 0.25 && v.abs() < 5.0 {
        *x = 0.0;
        *v = 0.0;
        return false;
    }
    true
}

impl Model {
    /// The note's past is open over the page (and not on its way out).
    pub(super) fn scrubbing(&self) -> bool {
        self.past.scrub.as_ref().is_some_and(|s| !s.closing)
    }

    /// The version the open scrubber shows, oldest first.
    #[cfg(test)]
    fn shown(&self) -> Option<usize> {
        self.past.scrub.as_ref().filter(|s| !s.closing).map(|s| s.shown)
    }

    /// A different note is shown: its past and dates are its own.
    pub(super) fn noted(&mut self) {
        self.past.scrub = None;
        self.past.moving = false;
        self.past.words.set((u64::MAX, 0, None));
        self.read_times();
    }

    /// When the open note's file was made and last written.
    pub(super) fn read_times(&mut self) {
        self.past.times = match &self.path {
            Some(path) => match std::fs::metadata(path) {
                Ok(m) => (m.created().ok(), m.modified().ok()),
                Err(_) => (None, None),
            },
            None => (None, None),
        };
    }

    /// The file the open note's past is kept in.
    fn history_file(&self) -> Option<PathBuf> {
        if self.session.note.is_empty() {
            return None;
        }
        let dir = history::dir(self.data.as_ref()?, &self.vault);
        Some(history::file(&dir, &self.session.note))
    }

    /// Before a note's first save in this run, what its file says is kept
    /// as a version (when it was last written), so the note as it was
    /// before these edits is always in its past.
    pub(super) fn keep_base(&mut self) {
        if self.session.note.is_empty() || self.past.kept.0 == self.session.note {
            return;
        }
        let now = ms(SystemTime::now());
        if let Some(file) = self.history_file()
            && let Some(path) = &self.path
            && let Ok((text, _)) = note::load(path)
        {
            let at = self.past.times.1.map_or(now, ms);
            let _ = history::record(&file, &text, at);
        }
        self.past.kept = (self.session.note.clone(), now);
    }

    /// After a save: the note as written becomes a version, at most every
    /// half minute while typing goes on, and always when it is left.
    pub(super) fn keep_version(&mut self, flush: bool, now: u64) {
        if !flush && self.past.kept.0 == self.session.note && now.saturating_sub(self.past.kept.1) < KEEP_EVERY_MS {
            return;
        }
        if let Some(file) = self.history_file()
            && history::record(&file, self.editor.text(), now).is_ok()
        {
            self.past.kept = (self.session.note.clone(), now);
        }
    }

    /// The note was written (or `flush`ed, as it is left): it goes to the
    /// top of its folder in the side menu, and becomes a version.
    pub(super) fn written(&mut self, wrote: bool, flush: bool) {
        if wrote {
            self.read_times();
            let note = self.session.note.clone();
            vault::written(&mut self.session.sidebar, note.strip_suffix(".md").unwrap_or(&note));
        }
        if wrote || (flush && self.past.kept.0 == self.session.note) {
            self.keep_version(flush, ms(SystemTime::now()));
        }
    }

    /// The file was renamed: its past is kept under the new name.
    pub(super) fn history_renamed(&mut self, from: &str, to: &str) {
        if let Some(data) = &self.data {
            let _ = history::rename(&history::dir(data, &self.vault), from, to);
        }
        if self.past.kept.0 == from {
            self.past.kept.0 = to.to_string();
        }
    }

    /// For `notsy-mac --dump --scrub`: a made-up past for the open note
    /// (the note grown a few lines at a time over some days, kept in a
    /// folder of its own in the temp folder, never the app's data), opened
    /// on a version in the middle. Returns whether the scrubber opened.
    pub fn demo_past(&mut self) -> bool {
        let dir = std::env::temp_dir().join(format!("notsy-app-demo-past-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        self.data = Some(dir);
        let Some(file) = self.history_file() else { return false };
        let lines: Vec<String> = self.editor.text().split('\n').map(str::to_string).collect();
        let (minute, hour, day) = (60_000u64, 3_600_000u64, 86_400_000u64);
        let now = ms(SystemTime::now());
        let n = 24;
        for k in 0..n {
            let keep = (lines.len() * (k + 1) / (n + 1)).max(1);
            // Three sittings: a first draft, a return, and today's edits.
            let at = match k {
                0..8 => now - 6 * day + k as u64 * 4 * minute,
                8..16 => now - 3 * day - 2 * hour + (k as u64 - 8) * 6 * minute,
                _ => now - 2 * hour + (k as u64 - 16) * 9 * minute,
            };
            let _ = history::record(&file, &lines[..keep].join("\n"), at);
        }
        if !self.open_scrub() {
            return false;
        }
        let len = self.past.scrub.as_ref().map_or(0, |s| s.times.len());
        self.scrub_to(len * 2 / 3);
        for _ in 0..300 {
            self.scrub_step(0.02);
        }
        true
    }

    // The date line.

    /// The note's dates and word count, small above its title: none for
    /// the welcome page.
    pub(super) fn date_line(&self) -> Option<String> {
        if self.session.note.is_empty() {
            return None;
        }
        let n = self.words_now();
        let dates = self.past.dates.get(self.past.times, || when::page_dates(&*self.services.calendar, self.past.times.0, self.past.times.1));
        dates.map(|d| d + &when::words_phrase(n))
    }

    /// The words in the note, counted again only when it changed (a long
    /// note while it is typed in, not more than every 400 ms).
    fn words_now(&self) -> usize {
        let version = self.editor.version();
        let (v, n, at) = self.past.words.get();
        if v == version {
            return n;
        }
        if self.editor.text().len() > WORDS_AT_ONCE && v != u64::MAX && at.is_some_and(|t| t.elapsed() < WORDS_PAUSE) {
            return n;
        }
        let n = when::count_words(self.editor.text());
        self.past.words.set((version, n, Some(Instant::now())));
        n
    }

    // The scrubber.

    /// The note's past opens, or closes.
    pub fn toggle_scrub(&mut self) -> bool {
        if self.scrubbing() {
            return self.close_scrub();
        }
        self.open_scrub()
    }

    fn open_scrub(&mut self) -> bool {
        // Only a note that is a file has a past; not the page being typed
        // in before it has words, nor the welcome page.
        if self.session.note.is_empty() || self.path.is_none() || self.data.is_none() {
            return false;
        }
        self.search = None;
        self.trash = None;
        self.find = None;
        self.save_flush();
        // Now is the newest version.
        self.keep_base();
        let now = ms(SystemTime::now());
        let Some(file) = self.history_file() else { return false };
        if history::record(&file, self.editor.text(), now).is_ok() {
            self.past.kept = (self.session.note.clone(), now);
        }
        let history = History::open(&file);
        let n = history.len();
        if n < 2 {
            // Said, as on Windows: a key that does nothing reads as broken.
            self.whisper("No earlier versions yet: they are kept as you write");
            return true;
        }
        let times: Vec<u64> = (0..n).map(|i| history.at(i)).collect();
        let changes = (0..n).map(|i| history.change(i)).collect();
        let since = when::day(&*self.services.calendar, time_of(times[0]));
        self.past.scrub = Some(Scrub {
            history,
            times,
            changes,
            shown: n - 1,
            text: String::new(),
            lines: Vec::new(),
            changed: Vec::new(),
            when: String::new(),
            ago: String::new(),
            since,
            offset: 0.0,
            velocity: 0.0,
            past: 0.0,
            alpha: 0.0,
            closing: false,
            dragging: false,
            hover_line: None,
            scroll: (self.scroll_line, self.scroll),
        });
        // It opens on the version before now: the playhead glides back one.
        self.scrub_to(n - 2);
        self.past.moving = true;
        true
    }

    /// Back to now, where the page was when the past opened.
    pub fn close_scrub(&mut self) -> bool {
        let Some(s) = &mut self.past.scrub else { return false };
        if s.closing {
            return false;
        }
        s.closing = true;
        s.dragging = false;
        s.hover_line = None;
        (self.scroll_line, self.scroll) = s.scroll;
        self.past.moving = true;
        true
    }

    /// Shows version `i`, the playhead gliding there from where it is.
    pub(super) fn scrub_to(&mut self, i: usize) -> bool {
        let now = self.editor.text();
        let calendar = self.services.calendar.clone();
        let Some(s) = &mut self.past.scrub else { return false };
        let i = i.min(s.times.len() - 1);
        if i == s.shown && !s.text.is_empty() {
            return false;
        }
        s.offset += s.shown as f32 - i as f32;
        s.shown = i;
        // The newest version is the note now (what was brought back since
        // included).
        s.text = if i + 1 == s.times.len() { now.to_string() } else { s.history.text(i) };
        s.lines = notsy_markdown::lines(&s.text).collect();
        s.changed = history::changed(&s.text, now);
        (s.when, s.ago) = if i + 1 == s.times.len() { ("Now".to_string(), String::new()) } else { when::moment(&*calendar, time_of(s.times[i])) };
        s.hover_line = None;
        self.past.moving = true;
        true
    }

    /// One frame of the scrubber's motion, `dt` seconds on; false once all
    /// is still.
    pub fn scrub_step(&mut self, dt: f32) -> bool {
        let Some(s) = &mut self.past.scrub else { return false };
        let gliding = settle(&mut s.offset, &mut s.velocity, dt, GLIDE);
        let in_past = !s.closing && s.shown + 1 < s.times.len();
        let going = toward(&mut s.past, if in_past { 1.0 } else { 0.0 }, FADE * 0.7, dt);
        let fading = toward(&mut s.alpha, if s.closing { 0.0 } else { 1.0 }, FADE, dt);
        if s.closing && s.alpha <= 0.0 {
            self.past.scrub = None;
            return false;
        }
        gliding || going || fading
    }

    pub(super) fn scrub_view(&self) -> Option<ScrubView<'_>> {
        self.past.scrub.as_ref().map(|s| ScrubView {
            times: &s.times,
            changes: &s.changes,
            shown: s.shown,
            head: s.shown as f32 + s.offset,
            when: &s.when,
            ago: &s.ago,
            since: &s.since,
            changed: &s.changed,
            past: s.past,
            alpha: s.alpha,
            hover_line: s.hover_line,
        })
    }

    /// The version shown, to draw in place of the note: its text and lines.
    /// None once it is closing (the note shows again as the dock fades).
    pub(super) fn scrub_page(&self) -> Option<(&str, &[notsy_markdown::Line])> {
        self.past.scrub.as_ref().filter(|s| !s.closing).map(|s| (s.text.as_str(), s.lines.as_slice()))
    }

    /// Keys while the scrubber is open, by AppKit's name for them: arrows
    /// step through versions, Home and End go to the oldest and to now,
    /// Return restores, Esc closes. Anything else is held back from the
    /// note underneath. None when it is not open.
    pub fn scrub_command(&mut self, name: &str) -> Option<bool> {
        let s = self.past.scrub.as_ref().filter(|s| !s.closing)?;
        let (shown, last) = (s.shown, s.times.len() - 1);
        Some(match name {
            "moveLeft:" | "moveUp:" | "moveBackward:" => self.scrub_to(shown.saturating_sub(1)),
            "moveRight:" | "moveDown:" | "moveForward:" => self.scrub_to(shown + 1),
            "moveToBeginningOfDocument:" | "scrollToBeginningOfDocument:" | "moveToBeginningOfLine:" | "moveToLeftEndOfLine:" => self.scrub_to(0),
            "moveToEndOfDocument:" | "scrollToEndOfDocument:" | "moveToEndOfLine:" | "moveToRightEndOfLine:" => self.scrub_to(last),
            "cancelOperation:" => self.close_scrub(),
            "insertNewline:" | "insertParagraphSeparator:" => self.restore_version(),
            _ => false,
        })
    }

    /// The note becomes the version shown: one undo step.
    pub fn restore_version(&mut self) -> bool {
        let Some(s) = self.past.scrub.as_ref().filter(|s| !s.closing) else { return false };
        if s.shown + 1 >= s.times.len() {
            return self.close_scrub();
        }
        let text = s.text.clone();
        self.close_scrub();
        self.edit(|e| {
            let len = e.text().len();
            e.replace_ranges(&[(0, len, text)]);
        });
        self.reveal = false;
        true
    }

    /// Line `i` of the version shown comes back into the note: one undo
    /// step. Its mark goes, as the note now has it.
    pub(super) fn bring_back_line(&mut self, i: usize) -> bool {
        let Some(s) = self.past.scrub.as_ref().filter(|s| !s.closing) else { return false };
        if s.changed.get(i) != Some(&true) {
            return false;
        }
        let replacement = history::bring_back(&s.text, self.editor.text(), i);
        let before = self.editor.version();
        self.editor.replace_ranges(&[replacement]);
        self.dirty |= self.editor.version() != before;
        let now = self.editor.text().to_string();
        if let Some(s) = &mut self.past.scrub {
            s.changed = history::changed(&s.text, &now);
            s.hover_line = None;
        }
        true
    }

    /// The pointer pressed at (x, y) DIPs while the past is open (or on the
    /// note's dates): what it did, if the scrubber has a say.
    pub(super) fn scrub_down(&mut self, x: f32, y: f32) -> Option<bool> {
        let target = scene::hit(&self.hits, x, y);
        if target == Some(Target::PageDate) {
            return Some(self.toggle_scrub());
        }
        if !self.scrubbing() {
            return None;
        }
        match target {
            Some(Target::ScrubClose) => Some(self.close_scrub()),
            Some(Target::ScrubRestore) => Some(self.restore_version()),
            Some(Target::BringBack(i)) => Some(self.bring_back_line(i)),
            Some(Target::ScrubTrack) => Some(self.scrub_point(x)),
            // The past is looked at: no caret, no selection, no editing.
            Some(Target::ScrubPanel | Target::Note | Target::Checkbox(_) | Target::Tool(_)) => Some(false),
            _ => None,
        }
    }

    /// The track was pressed or dragged at `x` DIPs: the version nearest
    /// there.
    fn scrub_point(&mut self, x: f32) -> bool {
        let Some(&(r, _)) = self.hits.iter().find(|(_, t)| *t == Target::ScrubTrack) else { return false };
        let Some(s) = &mut self.past.scrub else { return false };
        s.dragging = true;
        // The track's hit reaches 10 DIPs past its ends.
        let places = scrub_ui::places(&s.times, r.width() - 20.0);
        let i = scrub_ui::nearest(&places, x - r.left - 10.0);
        self.scrub_to(i);
        true
    }

    pub(super) fn scrub_dragging(&self) -> bool {
        self.past.scrub.as_ref().is_some_and(|s| s.dragging)
    }

    /// The pointer dragged to `x` DIPs: along the track while it is held.
    pub(super) fn scrub_drag(&mut self, x: f32) -> bool {
        self.scrub_point(x)
    }

    /// The button came up.
    pub fn pointer_up(&mut self) -> bool {
        if self.social_up() {
            return true;
        }
        if self.drop_sidebar_edge() {
            return true;
        }
        if self.popup_up() {
            return true;
        }
        if self.ui.scroll_drag.take().is_some() {
            self.ui.scrolled_at = Some(Instant::now());
            return true;
        }
        if self.resizing.is_some() {
            return self.end_resize();
        }
        match &mut self.past.scrub {
            Some(s) if s.dragging => {
                s.dragging = false;
                true
            }
            _ => false,
        }
    }

    /// The pointer moved over the page at (x, y) DIPs: a changed line
    /// under it offers to come back. Whether that changed.
    pub(super) fn scrub_hover(&mut self, x: f32, y: f32) -> bool {
        let over_page = matches!(scene::hit(&self.hits, x, y), Some(Target::Note | Target::BringBack(_)));
        let Some(s) = &mut self.past.scrub else { return false };
        let line = self
            .lines
            .iter()
            .find(|l| over_page && y >= l.y && y < l.y + l.height)
            .and_then(|l| s.lines.iter().position(|m| m.start == l.para.start))
            .filter(|&i| s.changed.get(i) == Some(&true));
        std::mem::replace(&mut s.hover_line, line) != line
    }
}

#[cfg(test)]
mod tests;
