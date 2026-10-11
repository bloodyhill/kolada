//! The time scrubber (drawn by `notsy_ui::scrub`): the note's past, kept
//! in the app's data as it is written (`notsy_core::history`), shown version
//! by version over the page. Click the note's dates (or Ctrl+Shift+H) to
//! open it; drag along its track or use the arrow keys; a changed line
//! comes back on its own, Restore brings back the whole version. Both are
//! one undo step. Esc returns to now.

use notsy_core::history::{self, History};
use notsy_ui::scrub::{self as scrub_ui, ScrubView};

use super::*;

/// While typing goes on, a version is kept at most this often (and always
/// when the note is left).
pub(super) const KEEP_EVERY_MS: u64 = 30_000;
/// How quickly the playhead arrives at a version (rad/s, critically
/// damped), and how fast the dock fades and the page goes back (per second).
const GLIDE: f32 = 24.0;
const FADE: f32 = 8.0;

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
    lines: Vec<md::Line>,
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
    scroll: (u64, f32),
}

/// Milliseconds since the Unix epoch.
pub(super) fn ms(t: SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn time_of(ms: u64) -> SystemTime {
    std::time::UNIX_EPOCH + Duration::from_millis(ms)
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

impl App {
    /// The file the open note's past is kept in.
    fn history_file(&self) -> Option<PathBuf> {
        if self.session.note.is_empty() {
            return None;
        }
        let dir = history::dir(&notsy_core::app_data_dir()?, Path::new(&self.session.vault));
        Some(history::file(&dir, &self.session.note))
    }

    /// Before a note's first save in this run, what its file says is kept
    /// as a version (when it was last written), so the note as it was
    /// before these edits is always in its past.
    pub(super) fn keep_base(&mut self) {
        if self.session.note.is_empty() || self.kept.0 == self.session.note {
            return;
        }
        let now = ms(SystemTime::now());
        if let Some(file) = self.history_file()
            && let Ok((text, _)) = note::load(&self.note_path())
        {
            let at = self.note_times.1.map_or(now, ms);
            let _ = history::record(&file, &text, at);
        }
        self.kept = (self.session.note.clone(), now);
    }

    /// After a save: the note as written becomes a version, at most every
    /// half minute while typing goes on, and always when it is left.
    pub(super) fn keep_version(&mut self, flush: bool) {
        let now = ms(SystemTime::now());
        if !flush && now.saturating_sub(self.kept.1) < KEEP_EVERY_MS {
            return;
        }
        if let Some(file) = self.history_file()
            && history::record(&file, self.editor.text(), now).is_ok()
        {
            self.kept = (self.session.note.clone(), now);
        }
    }

    /// The note's past opens, or closes.
    pub(super) fn toggle_scrub(&mut self) {
        match &self.scrub {
            Some(s) if !s.closing => self.close_scrub(),
            _ => self.open_scrub(),
        }
    }

    fn open_scrub(&mut self) {
        if self.read_only || self.session.note.is_empty() {
            return self.whisper("A note's past is kept once it is saved".to_string());
        }
        self.close_menu();
        self.close_popup();
        self.save_note(true);
        // Now is the newest version.
        self.keep_base();
        let now = ms(SystemTime::now());
        let Some(file) = self.history_file() else { return };
        if history::record(&file, self.editor.text(), now).is_ok() {
            self.kept = (self.session.note.clone(), now);
        }
        let history = History::open(&file);
        let n = history.len();
        if n < 2 {
            return self.whisper("No earlier versions yet: they are kept as you write".to_string());
        }
        let times: Vec<u64> = (0..n).map(|i| history.at(i)).collect();
        let changes = (0..n).map(|i| history.change(i)).collect();
        let since = crate::when::day(time_of(times[0]));
        self.scrub = Some(Scrub {
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
            scroll: (self.session.scroll_line, self.session.scroll_px),
        });
        // It opens on the version before now: the playhead glides back one.
        self.scrub_to(n - 2);
        self.scrub_moving = true;
        self.request_frame();
    }

    /// Back to now, where the page was when the past opened.
    pub(super) fn close_scrub(&mut self) {
        let Some(s) = &mut self.scrub else { return };
        if s.closing {
            return;
        }
        s.closing = true;
        s.dragging = false;
        s.hover_line = None;
        (self.session.scroll_line, self.session.scroll_px) = s.scroll;
        self.fix_scroll_line();
        self.scrub_moving = true;
        self.request_frame();
    }

    /// Shows version `i`, the playhead gliding there from where it is.
    pub(super) fn scrub_to(&mut self, i: usize) {
        let now = self.editor.text();
        let Some(s) = &mut self.scrub else { return };
        let i = i.min(s.times.len() - 1);
        if i == s.shown && !s.text.is_empty() {
            return;
        }
        s.offset += s.shown as f32 - i as f32;
        s.shown = i;
        // The newest version is the note now (what was brought back since
        // included).
        s.text = if i + 1 == s.times.len() { now.to_string() } else { s.history.text(i) };
        s.lines = md::lines(&s.text).collect();
        s.changed = history::changed(&s.text, now);
        (s.when, s.ago) = if i + 1 == s.times.len() { ("Now".to_string(), String::new()) } else { crate::when::moment(time_of(s.times[i])) };
        s.hover_line = None;
        self.scrub_moving = true;
        self.request_frame();
    }

    /// One frame of the scrubber's motion; false once all is still.
    pub(super) fn step_scrub(&mut self, dt: f32) -> bool {
        let Some(s) = &mut self.scrub else { return false };
        let gliding = settle(&mut s.offset, &mut s.velocity, dt, GLIDE);
        let in_past = !s.closing && s.shown + 1 < s.times.len();
        let going = toward(&mut s.past, if in_past { 1.0 } else { 0.0 }, FADE * 0.7, dt);
        let fading = toward(&mut s.alpha, if s.closing { 0.0 } else { 1.0 }, FADE, dt);
        if s.closing && s.alpha <= 0.0 {
            self.scrub = None;
            return false;
        }
        gliding || going || fading
    }

    pub(super) fn scrub_view(&self) -> Option<ScrubView<'_>> {
        self.scrub.as_ref().map(|s| ScrubView {
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
    pub(super) fn scrub_page(&self) -> Option<(&str, &[md::Line])> {
        self.scrub.as_ref().filter(|s| !s.closing).map(|s| (s.text.as_str(), s.lines.as_slice()))
    }

    /// What the page's measured heights are kept for: the note's version,
    /// or, while scrubbing, the version shown.
    pub(super) fn page_version(&self) -> u64 {
        match self.scrub.as_ref().filter(|s| !s.closing) {
            Some(s) => (1 << 63) | (self.editor.version() << 24) ^ s.shown as u64,
            None => self.editor.version(),
        }
    }

    /// Keys while the scrubber is open: arrows step through versions, Home
    /// and End go to the oldest and to now, Enter restores, Esc closes.
    /// Anything else is held back from the note underneath. False when it
    /// is not open.
    pub(super) fn scrub_key(&mut self, vk: u16) -> bool {
        let Some(s) = &self.scrub else { return false };
        if s.closing {
            return false;
        }
        let (shown, last) = (s.shown, s.times.len() - 1);
        match vk {
            v if v == VK_LEFT.0 || v == VK_UP.0 => self.scrub_to(shown.saturating_sub(1)),
            v if v == VK_RIGHT.0 || v == VK_DOWN.0 => self.scrub_to(shown + 1),
            v if v == VK_HOME.0 => self.scrub_to(0),
            v if v == VK_END.0 => self.scrub_to(last),
            v if v == VK_ESCAPE.0 => self.close_scrub(),
            v if v == VK_RETURN.0 => self.restore_version(),
            _ => {}
        }
        true
    }

    /// The note becomes the version shown: one undo step.
    pub(super) fn restore_version(&mut self) {
        let Some(s) = &self.scrub else { return };
        if s.shown + 1 >= s.times.len() {
            return self.close_scrub();
        }
        let (text, when) = (s.text.clone(), s.when.clone());
        self.edit(|e| {
            let len = e.text().len();
            e.replace_ranges(&[(0, len, text)]);
        });
        self.close_scrub();
        self.whisper(format!("Restored the note from {when} \u{00B7} Ctrl+Z undoes it"));
    }

    /// Line `i` of the version shown comes back into the note: one undo
    /// step. Its mark goes, as the note now has it.
    pub(super) fn bring_back_line(&mut self, i: usize) {
        let Some(s) = &self.scrub else { return };
        if s.changed.get(i) != Some(&true) {
            return;
        }
        let replacement = history::bring_back(&s.text, self.editor.text(), i);
        self.edit(|e| e.replace_ranges(&[replacement]));
        let now = self.editor.text().to_string();
        if let Some(s) = &mut self.scrub {
            s.changed = history::changed(&s.text, &now);
            s.hover_line = None;
        }
        self.whisper("Brought back \u{00B7} Ctrl+Z undoes it".to_string());
        self.request_frame();
    }

    /// The track was pressed or dragged at `x` (pixels): the version
    /// nearest there.
    pub(super) fn scrub_point(&mut self, x: i32) {
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let Some(&(r, _)) = self.hits.iter().find(|(_, t)| *t == Target::ScrubTrack) else { return };
        let Some(s) = &mut self.scrub else { return };
        s.dragging = true;
        // The track's hit reaches 10 DIPs past its ends.
        let places = scrub_ui::places(&s.times, r.width() - 20.0);
        let i = scrub_ui::nearest(&places, x as f32 / scale - r.left - 10.0);
        self.scrub_to(i);
    }

    pub(super) fn scrub_dragging(&self) -> bool {
        self.scrub.as_ref().is_some_and(|s| s.dragging)
    }

    pub(super) fn scrub_release(&mut self) {
        if let Some(s) = &mut self.scrub {
            s.dragging = false;
        }
    }

    /// The pointer moved over the page at `(x, y)` pixels: a changed line
    /// under it offers to come back.
    pub(super) fn scrub_hover(&mut self, x: i32, y: i32) {
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let (x, y) = (x as f32 / scale, y as f32 / scale);
        let over_page = matches!(scene::hit(&self.hits, x, y), Some(Target::Note | Target::BringBack(_)));
        let Some(s) = &mut self.scrub else { return };
        let line = self
            .lines
            .iter()
            .find(|l| over_page && y >= l.y && y < l.y + l.height)
            .and_then(|l| s.lines.iter().position(|m| m.start == l.para.start))
            .filter(|&i| s.changed.get(i) == Some(&true));
        if line != s.hover_line {
            s.hover_line = line;
            self.request_frame();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fades_move_toward_where_they_head_and_stop_there() {
        let mut x = 0.0;
        assert!(toward(&mut x, 1.0, 8.0, 0.05));
        assert!((x - 0.4).abs() < 1e-6);
        while toward(&mut x, 1.0, 8.0, 0.05) {}
        assert_eq!(x, 1.0);
        assert!(!toward(&mut x, 1.0, 8.0, 0.05));
        assert_eq!(time_of(ms(time_of(1_234_567))), time_of(1_234_567));
    }
}
