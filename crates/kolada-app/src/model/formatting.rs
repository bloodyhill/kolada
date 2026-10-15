//! The formatting bars' moments, as on Windows (`kolada-win` `app/bars.rs`;
//! what they look like is `kolada_ui::chrome` `toolbar`, `reading`,
//! `tooltip`, and `kolada_ui::selbar`): the toolbar's Aa card, a button's
//! name after the pointer rests on it, and the selection bar over text
//! selected with the pointer, gone on typing, a click elsewhere, Escape
//! and scrolling. Timers are the frame loop's: a name waits by the clock.

use std::time::{Duration, Instant};

use kolada_ui::selbar::SelBar;

use super::*;

/// The pointer rests on a button this long before its name shows.
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);
/// The Aa card fades in over this long.
const READING_FADE: Duration = Duration::from_millis(120);

#[derive(Default)]
pub(super) struct Formatting {
    pub selbar: SelBar,
    /// The Aa card, open since then.
    reading: Option<Instant>,
    /// The button whose name shows, and the one the pointer rests on with
    /// when its name is due.
    tooltip: Option<Target>,
    pending: Option<(Target, Instant)>,
    /// The press went down on the page's text: letting go may bring the
    /// selection bar.
    pub pressed_text: bool,
}

impl Model {
    /// The selection bar, the Aa card and a button's name as the scene
    /// draws them.
    pub(super) fn formatting_view(&self) -> (Option<f32>, Option<f32>, Option<Target>) {
        let f = &self.formatting;
        let animate = self.ui.animate;
        let page = self.popup.is_none() && self.scrub_page().is_none() && !self.settings.read_only;
        let selbar = f.selbar.view(Instant::now(), animate).filter(|_| page);
        let reading = f.reading.map(|t| if animate { (t.elapsed().as_secs_f32() / READING_FADE.as_secs_f32()).min(1.0) } else { 1.0 });
        (selbar, reading, f.tooltip)
    }

    /// Before a frame: the selection bar goes once its selection is not the
    /// one there is (typed over, moved by the keys, collapsed).
    pub(super) fn follow_selection(&mut self) {
        let s = self.editor.selection();
        self.formatting.selbar.follow((s.anchor, s.head));
    }

    /// Something of theirs moves: frames keep coming.
    pub(super) fn formatting_moving(&self) -> bool {
        let f = &self.formatting;
        self.ui.animate && (f.selbar.moving(Instant::now()) || f.reading.is_some_and(|t| t.elapsed() < READING_FADE + Duration::from_millis(20)))
    }

    /// When a button's name is due.
    pub(super) fn formatting_due(&self) -> Option<Instant> {
        self.formatting.pending.map(|(_, at)| at)
    }

    /// A frame: a name due shows. Whether the screen changes.
    pub(super) fn step_formatting(&mut self) -> bool {
        let f = &mut self.formatting;
        match f.pending {
            Some((target, at)) if Instant::now() >= at => {
                f.pending = None;
                if self.hover == Some(target) {
                    f.tooltip = Some(target);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// The pointer is over `target` now: a name that showed goes, and one
    /// for a button is due after the pointer rests there.
    pub(super) fn tooltip_hover(&mut self, target: Option<Target>) {
        let named = target.filter(|t| chrome::button_name(*t, self.platform, false).is_some());
        let f = &mut self.formatting;
        if f.tooltip != named {
            f.tooltip = None;
        }
        if named != f.pending.map(|(t, _)| t) && named != f.tooltip {
            f.pending = named.map(|t| (t, Instant::now() + TOOLTIP_DELAY));
        }
    }

    /// A press, a key, the pointer leaving: no name shows, none waits.
    /// Whether one showed.
    pub(super) fn tooltip_away(&mut self) -> bool {
        self.formatting.pending = None;
        self.formatting.tooltip.take().is_some()
    }

    /// Aa: its card opens, or closes.
    pub(super) fn toggle_reading(&mut self) -> bool {
        let f = &mut self.formatting;
        f.reading = if f.reading.is_some() { None } else { Some(Instant::now()) };
        true
    }

    /// The Aa card goes. Whether it was open.
    pub(super) fn close_reading(&mut self) -> bool {
        self.formatting.reading.take().is_some()
    }

    /// A press at `target`: a name showing goes, and the selection bar and
    /// the Aa card unless it is on them. Whether the screen changes.
    pub(super) fn formatting_press(&mut self, target: Option<Target>) -> bool {
        let mut changed = self.tooltip_away();
        if !matches!(target, Some(Target::SelBar(_) | Target::SelBarPanel)) {
            changed |= self.formatting.selbar.dismiss();
        }
        if !matches!(target, Some(Target::Reading | Target::ReadingPanel | Target::TextSize(_) | Target::Width(_))) {
            changed |= self.close_reading();
        }
        self.formatting.pressed_text = target == Some(Target::Note);
        changed
    }

    /// The button came up: a selection the pointer made on the page (a
    /// drag, a double or triple click) brings the selection bar. Whether
    /// the screen changes.
    pub(super) fn formatting_release(&mut self) -> bool {
        if !std::mem::take(&mut self.formatting.pressed_text) || self.popup.is_some() || self.scrubbing() || self.settings.read_only {
            return false;
        }
        let s = self.editor.selection();
        self.formatting.selbar.pointer_selected((s.anchor, s.head), Instant::now())
    }

    /// Escape, a scroll: the selection bar goes. Whether it was up.
    pub(super) fn dismiss_selbar(&mut self) -> bool {
        self.formatting.selbar.dismiss()
    }

    /// A button on the selection bar: its formatting, one step to undo; the
    /// bar stays over what is still selected.
    pub(super) fn selbar_tool(&mut self, tool: Tool) -> bool {
        self.tooltip_away();
        let changed = self.tool(tool);
        let s = self.editor.selection();
        self.formatting.selbar.applied((s.anchor, s.head));
        changed
    }

    /// Whether the selection bar is up (for tests).
    #[cfg(test)]
    pub fn selection_bar_shown(&self) -> bool {
        self.formatting.selbar.shown()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_editor::Selection;

    /// A vault with one note, open, a frame drawn at 1000 x 700.
    fn open() -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-selbar-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let more = "More to read.\n".repeat(60);
        std::fs::write(dir.join("Plan.md"), format!("# Plan\nShip the public beta on October twelve.\nThen rest.\n{more}")).unwrap();
        let mut m = Model::new(dir.clone(), false);
        m.open("Plan.md");
        m.frame(1000, 700, 1.0);
        (m, dir)
    }

    fn rect(m: &Model, target: Target) -> Option<Rect> {
        m.hits.iter().rev().find(|(_, t)| *t == target).map(|(r, _)| *r)
    }

    /// Where the word `word` is on screen: its middle.
    fn word_at(m: &Model, word: &str) -> (f32, f32) {
        let at = m.text().find(word).unwrap();
        let line = m.lines.iter().find(|l| l.para.start <= at && at <= l.para.end).unwrap();
        let nt = m.note_text();
        let [l, t, r, b] = nt.range(&line.para, at, at + word.len()).unwrap()[0];
        (line.x + (l + r) / 2.0, line.y + (t + b) / 2.0)
    }

    /// Double-clicks `word`: it is selected, and the button comes up.
    fn double_click(m: &mut Model, word: &str) {
        let (x, y) = word_at(m, word);
        m.pointer_down(x, y, 1, false);
        m.pointer_up_at(x, y);
        m.pointer_down(x, y, 2, false);
        m.pointer_up_at(x, y);
        m.frame(1000, 700, 1.0);
    }

    #[test]
    fn it_comes_up_above_a_word_double_clicked() {
        let (mut m, dir) = open();
        double_click(&mut m, "public");
        let s = m.selection();
        assert_eq!(&m.text()[s.anchor.min(s.head)..s.anchor.max(s.head)], "public");
        assert!(m.selection_bar_shown());
        let bar = rect(&m, Target::SelBarPanel).expect("the selection bar");
        let (x, y) = word_at(&m, "public");
        assert!(bar.bottom < y - 8.0, "above the word: {bar:?} at {y}");
        assert!(((bar.left + bar.right) / 2.0 - x).abs() < 1.0 || bar.left <= 8.5, "centred on it");
        for tool in kolada_ui::selbar::TOOLS {
            let b = rect(&m, Target::SelBar(tool)).unwrap_or_else(|| panic!("{tool:?}"));
            assert!(b.left >= bar.left && b.right <= bar.right && b.top >= bar.top && b.bottom <= bar.bottom);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_drag_brings_it_and_a_click_elsewhere_sends_it_away() {
        let (mut m, dir) = open();
        let (x0, y) = word_at(&m, "Ship");
        let (x1, _) = word_at(&m, "beta");
        m.pointer_down(x0, y, 1, false);
        assert!(!m.selection_bar_shown(), "not while dragging");
        m.pointer_dragged(x1, y);
        m.pointer_up_at(x1, y);
        m.frame(1000, 700, 1.0);
        assert!(m.selection_bar_shown() && rect(&m, Target::SelBarPanel).is_some());
        // A click elsewhere on the page.
        let (x, y) = word_at(&m, "rest");
        m.pointer_down(x, y, 1, false);
        m.pointer_up_at(x, y);
        m.frame(1000, 700, 1.0);
        assert!(!m.selection_bar_shown() && rect(&m, Target::SelBarPanel).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn it_goes_below_when_there_is_no_room_above() {
        let (mut m, dir) = open();
        // The line scrolled up to the window's top: no room above it.
        let (_, y) = word_at(&m, "public");
        m.scroll_by(y - 85.0);
        m.frame(1000, 700, 1.0);
        double_click(&mut m, "public");
        let bar = rect(&m, Target::SelBarPanel).expect("the selection bar");
        let (_, y) = word_at(&m, "public");
        assert!(bar.top > y, "under the word: {bar:?} at {y}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_keys_never_bring_it() {
        let (mut m, dir) = open();
        let at = m.text().find("public").unwrap();
        m.editor.set_selection(Selection::caret(at));
        for _ in 0..6 {
            m.go(Motion::Right, true);
        }
        m.frame(1000, 700, 1.0);
        assert!(!m.editor.selection().is_caret());
        assert!(!m.selection_bar_shown() && rect(&m, Target::SelBarPanel).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn it_goes_on_typing_escape_and_scrolling() {
        let (mut m, dir) = open();
        double_click(&mut m, "public");
        m.insert("open");
        m.frame(1000, 700, 1.0);
        assert!(!m.selection_bar_shown(), "typing over the selection");
        assert!(m.text().contains("Ship the open beta"));
        double_click(&mut m, "beta");
        assert!(m.selection_bar_shown());
        m.escape();
        assert!(!m.selection_bar_shown(), "Escape");
        double_click(&mut m, "beta");
        m.wheel(40.0, true);
        assert!(!m.selection_bar_shown(), "scrolling");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn its_buttons_format_as_one_step_and_it_stays() {
        let (mut m, dir) = open();
        let before = m.text().to_string();
        double_click(&mut m, "public");
        let bold = rect(&m, Target::SelBar(Tool::Bold)).unwrap();
        m.pointer_down((bold.left + bold.right) / 2.0, (bold.top + bold.bottom) / 2.0, 1, false);
        m.frame(1000, 700, 1.0);
        assert!(m.text().contains("**public**"), "bold");
        assert!(m.selection_bar_shown(), "still over the word");
        let high = rect(&m, Target::SelBar(Tool::Highlight)).unwrap();
        m.pointer_down((high.left + high.right) / 2.0, (high.top + high.bottom) / 2.0, 1, false);
        m.frame(1000, 700, 1.0);
        assert!(m.text().contains("==public=="), "highlighted: {:?}", m.text().len());
        m.undo();
        assert!(m.text().contains("**public**") && !m.text().contains("=="), "one step back is the bold alone");
        m.undo();
        assert_eq!(m.text(), before, "and one more is the note as it was");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_button_says_its_name_after_the_pointer_rests() {
        let (mut m, dir) = open();
        m.toggle_panel(scene::Panel::Toolbar);
        m.frame(1000, 700, 1.0);
        let bold = rect(&m, Target::Tool(Tool::Bold)).expect("the pinned toolbar");
        m.pointer_moved((bold.left + bold.right) / 2.0, (bold.top + bold.bottom) / 2.0);
        assert!(!m.step_formatting(), "not at once");
        let due = m.formatting_due().expect("a name due");
        assert!(due > Instant::now() + Duration::from_millis(400));
        m.formatting.pending = m.formatting.pending.map(|(t, _)| (t, Instant::now()));
        assert!(m.step_formatting());
        m.frame(1000, 700, 1.0);
        assert_eq!(m.formatting.tooltip, Some(Target::Tool(Tool::Bold)));
        m.pointer_left();
        assert_eq!(m.formatting.tooltip, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn aa_opens_a_card_of_text_size_and_line_width() {
        let (mut m, dir) = open();
        m.toggle_panel(scene::Panel::Toolbar);
        m.frame(1000, 700, 1.0);
        let aa = rect(&m, Target::Reading).expect("Aa");
        m.pointer_down((aa.left + aa.right) / 2.0, (aa.top + aa.bottom) / 2.0, 1, false);
        m.frame(1000, 700, 1.0);
        let wide = rect(&m, Target::Width(2)).expect("the card");
        m.pointer_down((wide.left + wide.right) / 2.0, (wide.top + wide.bottom) / 2.0, 1, false);
        assert_eq!(m.session.line_width, 2);
        let larger = rect(&m, Target::TextSize(true)).unwrap();
        m.pointer_down((larger.left + larger.right) / 2.0, (larger.top + larger.bottom) / 2.0, 1, false);
        assert_eq!(m.session.text_size, 18);
        m.frame(1000, 700, 1.0);
        assert!(rect(&m, Target::ReadingPanel).is_some(), "it stays while used");
        m.escape();
        m.frame(1000, 700, 1.0);
        assert!(rect(&m, Target::ReadingPanel).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
