//! The scroll bars of the page and of the file tree, and the page's map of
//! headings, as on Windows (`kolada-win` `app/scroll.rs`).
//!
//! The page lays out only what is on screen, so its scroll bar cannot know
//! every paragraph's height: they are estimated from their lengths (cheap
//! enough to redo on every keystroke), and only the note's end is measured.

use std::time::{Duration, Instant};

use kolada_markdown::{self as md, Block};

use super::*;

/// Estimated heights, each kept with what it was measured for.
#[derive(Default)]
pub struct Heights {
    /// Text version, column width, text size.
    key: (u64, u32, u32),
    /// Height above each line, then the whole note's.
    sums: Vec<f32>,
    /// The headings for the scroll bar's map, and what they are for.
    map_key: Option<((u64, u32, u32), u32)>,
    map: Vec<(String, u8, usize, f32)>,
    /// Where the page's scroll bar was and how long, when last shown: a
    /// hidden bar keeps them rather than measure the whole note each key.
    bar: (f32, f32),
}

/// A scroll bar being dragged (the page's or the file tree's), and where on
/// the thumb it was grabbed (DIPs from its top).
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub bar: Target,
    grab: f32,
}

impl Drag {
    /// A bar held by the middle of its thumb.
    pub fn new(bar: Target) -> Drag {
        Drag { bar, grab: 0.0 }
    }
}

/// A scroll bar shows this long after scrolling, then fades out over the second.
pub const SHOWN: Duration = Duration::from_millis(900);
pub const FADE: Duration = Duration::from_millis(350);

/// Pointer this close to the window's right edge brings the page's scroll
/// bar out, DIPs.
const NEAR_EDGE: f32 = 48.0;
/// Room kept between the caret and the bottom of the window, DIPs.
const CARET_MARGIN: f32 = 40.0;
/// Rough line height, DIPs.
const PAGE_LINE: f32 = 24.0;

/// How visible a scroll bar is after scrolling at `t`: fully for a moment,
/// then fading.
fn after_scroll(t: Option<Instant>) -> f32 {
    let Some(t) = t else { return 0.0 };
    let e = t.elapsed();
    if e < SHOWN { 1.0 } else { 1.0 - ((e - SHOWN).as_secs_f32() / FADE.as_secs_f32()).min(1.0) }
}

impl Model {
    fn heights_key(&self) -> (u64, u32, u32) {
        (self.editor.version(), self.column_width.to_bits(), self.fonts.body.size.to_bits())
    }

    /// Each line's height, estimated from its length and block, summed.
    fn estimate(&self) -> Vec<f32> {
        let size = self.fonts.body.size;
        let width = self.column_width.max(80.0);
        let text = self.editor.text();
        let lines = self.editor.outline();
        let title = title_line(text, lines);
        let mut sums = Vec::with_capacity(lines.len() + 1);
        let mut y = 0.0;
        sums.push(y);
        for (i, l) in lines.iter().enumerate() {
            let (s, spacing, pad) = match l.block {
                _ if Some(i) == title => (size * scene::HEADING_SCALE[0], 1.3, 0.0),
                Block::Heading(n) => (size * scene::HEADING_SCALE[(n.clamp(1, 6) - 1) as usize], 1.3, [18.0, 14.0].get(n as usize - 1).copied().unwrap_or(10.0)),
                // A folded fence is a sliver.
                Block::Fence => (1.0, 8.0, 0.0),
                Block::Code | Block::Frontmatter => (size * 0.875, 1.5, 0.0),
                _ => (size, 1.5, 0.0),
            };
            let chars = text[l.content().min(l.end)..l.end].chars().count().max(1) as f32;
            // About half an em per character in these faces.
            let per_row = (width / (s * 0.52)).max(8.0);
            y += pad + (chars / per_row).ceil().max(1.0) * s * spacing;
            sums.push(y);
        }
        sums
    }

    pub(super) fn estimated(&self) -> std::cell::Ref<'_, Vec<f32>> {
        let key = self.heights_key();
        if self.ui.heights.borrow().key != key || self.ui.heights.borrow().sums.is_empty() {
            let sums = self.estimate();
            let mut h = self.ui.heights.borrow_mut();
            h.key = key;
            h.sums = sums;
        }
        std::cell::Ref::map(self.ui.heights.borrow(), |h| &h.sums)
    }

    /// The room the note has in the window, DIPs.
    fn room(&self) -> f32 {
        (self.viewport.1 - self.ui.column.top - CARET_MARGIN).max(PAGE_LINE)
    }

    /// The page's scroll bar, if the note does not fit the window.
    pub(super) fn scroll_view(&self) -> Option<scene::ScrollView> {
        if self.scrub_page().is_some() || self.settings.read_only && self.lines.is_empty() {
            return None;
        }
        let text = self.editor.text();
        let (width, view) = self.viewport;
        let at_top = self.scroll_line == 0 && self.scroll <= 0.0;
        // The last frame showed the end of the note inside the window: at
        // the end (or, from the top, it all fits).
        let end_shown = self.lines.last().is_some_and(|l| l.para.end >= text.len() && l.y + l.height <= view - CARET_MARGIN + 1.0);
        if at_top && end_shown {
            return None;
        }
        let near = self.ui.pointer.is_some_and(|(x, y)| x > width - NEAR_EDGE && y > self.ui.column.top);
        let dragged = self.ui.scroll_drag.is_some_and(|d| d.bar == Target::Scrollbar);
        let alpha = if near || dragged { 1.0 } else { after_scroll(self.ui.scrolled_at) };
        // Hidden (while typing): where it was, without measuring the note.
        let (last_at, last_size) = self.ui.heights.borrow().bar;
        if alpha <= 0.0 && last_size > 0.0 {
            return Some(scene::ScrollView { at: last_at, size: last_size, alpha });
        }
        let sums = self.estimated();
        let lines = self.editor.outline();
        let total = sums.last().copied().unwrap_or(0.0).max(1.0);
        let room = self.room();
        if total <= room && at_top && self.lines.is_empty() {
            return None;
        }
        let i = lines.partition_point(|l| l.start < self.scroll_line);
        let top = sums.get(i).copied().unwrap_or(0.0) + self.scroll;
        let max = (total - room).max(1.0);
        let at = if end_shown { 1.0 } else { (top / max).min(0.99) };
        let size = (room / total).clamp(0.04, 1.0);
        drop(sums);
        self.ui.heights.borrow_mut().bar = (at, size);
        Some(scene::ScrollView { at, size, alpha })
    }

    /// The page's scroll bar track, thumb position and size, if it has one.
    fn page_bar(&self) -> Option<(Rect, f32, f32)> {
        let bar = self.scroll_view()?;
        Some((chrome::scroll_track(self.ui.column, self.viewport.0, self.viewport.1), bar.at, bar.size))
    }

    /// The file tree's scroll bar: its track, where it is, how much shows.
    fn tree_bar(&self) -> Option<(Rect, f32, f32)> {
        let track = self.ui.tree_track?;
        let (rows, fit) = (self.session.sidebar.len(), self.ui.tree_rows);
        (rows > fit && fit > 0).then(|| (track, self.ui.tree_top as f32 / (rows - fit) as f32, fit as f32 / rows as f32))
    }

    fn bar(&self, bar: Target) -> Option<(Rect, f32, f32)> {
        match bar {
            Target::TreeScrollbar => self.tree_bar(),
            _ => self.page_bar(),
        }
    }

    /// The button went down on a scroll bar at `y` DIPs: on the thumb it is
    /// grabbed; elsewhere on the track, the thumb jumps there first.
    pub(super) fn scroll_press(&mut self, bar: Target, y: f32) -> bool {
        let Some((track, at, size)) = self.bar(bar) else { return false };
        let (top, h) = chrome::thumb(track, at, size);
        let grab = if (top..top + h).contains(&y) { y - top } else { h / 2.0 };
        self.ui.scroll_drag = Some(Drag { bar, grab });
        self.scroll_drag_to(y);
        true
    }

    /// A grabbed thumb follows the pointer at `y` DIPs.
    pub(super) fn scroll_drag_to(&mut self, y: f32) -> bool {
        let Some(drag) = self.ui.scroll_drag else { return false };
        let Some((track, at, size)) = self.bar(drag.bar) else { return false };
        let (_, h) = chrome::thumb(track, at, size);
        let f = ((y - drag.grab - track.top) / (track.height() - h).max(1.0)).clamp(0.0, 1.0);
        match drag.bar {
            Target::TreeScrollbar => {
                let max = self.session.sidebar.len().saturating_sub(self.ui.tree_rows);
                self.ui.tree_top = (f * max as f32).round() as usize;
            }
            _ => self.scroll_to_fraction(f),
        }
        true
    }

    /// Scrolls the page to `f` of the way from its top (0) to its end (1).
    fn scroll_to_fraction(&mut self, f: f32) {
        self.ui.scroll_pending = 0.0;
        let (line, px) = {
            let sums = self.estimated();
            let lines = self.editor.outline();
            let target = f * (sums.last().copied().unwrap_or(0.0) - self.room()).max(0.0);
            let i = sums.partition_point(|&s| s <= target).saturating_sub(1).min(lines.len().saturating_sub(1));
            (lines.get(i).map_or(0, |l| l.start), target - sums.get(i).copied().unwrap_or(0.0))
        };
        self.scroll_line = line;
        self.scroll = 0.0;
        // Walks the rest exactly, and stops at the end.
        self.shift_scroll(if f >= 1.0 { f32::MAX / 4.0 } else { px });
    }

    /// The note's headings for the scroll bar's map: text, level, the line
    /// each starts, and where scrolling puts it at the top (0..1).
    fn headings(&self) -> Vec<(String, u8, usize, f32)> {
        let lines = self.editor.outline();
        let text = self.editor.text();
        let sums = self.estimated();
        let max = (sums.last().copied().unwrap_or(0.0) - self.room()).max(1.0);
        let title = title_line(text, lines);
        lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| {
                let level = match l.block {
                    Block::Heading(n) if Some(i) != title => n,
                    _ => return None,
                };
                let words = md::plain(&text[l.start..l.end]);
                let words = words.trim();
                (!words.is_empty()).then(|| (words.to_string(), level, l.start, (sums[i] / max).min(1.0)))
            })
            .collect()
    }

    /// `headings`, kept for the text version, width and view they are for.
    pub(super) fn headings_for_map(&self) -> std::cell::Ref<'_, Vec<(String, u8, usize, f32)>> {
        let key = (self.heights_key(), self.viewport.1.to_bits());
        if self.ui.heights.borrow().map_key != Some(key) {
            let found = self.headings();
            let mut h = self.ui.heights.borrow_mut();
            h.map_key = Some(key);
            h.map = found;
        }
        std::cell::Ref::map(self.ui.heights.borrow(), |h| &h.map)
    }

    /// Jumps to the heading on the line starting at `line`: its line at the
    /// top of the view (or as near as the note's end allows), the caret on it.
    pub(super) fn jump_to(&mut self, line: usize) -> bool {
        self.ui.scroll_pending = 0.0;
        self.scroll_line = line;
        self.scroll = 0.0;
        // Pulled back from past the end by a walk to it.
        self.shift_scroll(0.0);
        self.editor.set_selection(Selection::caret(line));
        self.caret_moved(false);
        self.reveal = false;
        self.ui.scrolled_at = Some(Instant::now());
        true
    }

    /// The headings' line starts, by `Target::Heading` index, as last mapped.
    pub(super) fn heading_line(&self, i: usize) -> Option<usize> {
        self.ui.heights.borrow().map.get(i).map(|h| h.2)
    }

    /// The wheel or trackpad: over the side menu it scrolls the file tree a
    /// row at a time, elsewhere the page. `precise` deltas (a trackpad) move
    /// at once, a mouse wheel's notches ease in.
    pub fn wheel(&mut self, dy: f32, precise: bool) -> bool {
        // Scrolling sends the selection bar away.
        self.dismiss_selbar();
        if self.settings.open {
            return self.scroll_options(dy);
        }
        self.wheel_agenda();
        if let Some(done) = self.social_wheel(dy) {
            return done;
        }
        if let Some(done) = self.popup_wheel(dy) {
            return done;
        }
        if self.hover.and_then(Target::panel) == Some(scene::Panel::Sidebar) {
            if self.search.is_some() || self.trash.is_some() {
                return false;
            }
            self.ui.tree_scroll += dy / PAGE_LINE;
            let rows = self.ui.tree_scroll.trunc();
            self.ui.tree_scroll -= rows;
            let max = self.session.sidebar.len().saturating_sub(self.ui.tree_rows) as f32;
            let top = (self.ui.tree_top as f32 + rows).clamp(0.0, max) as usize;
            return std::mem::replace(&mut self.ui.tree_top, top) != top;
        }
        self.ui.scrolled_at = Some(Instant::now());
        if precise {
            self.ui.scroll_pending = 0.0;
            self.scroll_by(dy);
        } else {
            self.ui.scroll_pending += dy;
        }
        true
    }

    /// Keeps the file tree's first row in range after the tree changed.
    pub(super) fn clamp_tree(&mut self) {
        let max = self.session.sidebar.len().saturating_sub(self.ui.tree_rows.max(1));
        self.ui.tree_top = self.ui.tree_top.min(max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long(test: &str) -> (PathBuf, Model) {
        let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut body = String::from("# Long\n\n");
        for s in 0..8 {
            body += &format!("## Part {s}\n");
            body += &(0..40).map(|i| format!("Paragraph {i} of a part that goes on.\n")).collect::<String>();
        }
        std::fs::write(dir.join("Long.md"), body).unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        (dir, m)
    }

    #[test]
    fn a_long_note_has_a_scroll_bar_that_shows_after_scrolling_and_drags_the_view() {
        let (dir, mut m) = long("bar");
        let bar = m.scroll_view().expect("a long note has a scroll bar");
        assert_eq!(bar.alpha, 0.0, "resting: out of sight");
        assert!(bar.size < 0.5 && bar.at == 0.0, "{} {}", bar.size, bar.at);
        assert!(m.wheel(400.0, true));
        m.frame(2000, 1440, 2.0);
        let bar = m.scroll_view().unwrap();
        assert_eq!(bar.alpha, 1.0, "shown after scrolling");
        assert!(bar.at > 0.0);
        // Dragged to the bottom of its track: the end of the note.
        let track = chrome::scroll_track(m.ui.column, m.viewport.0, m.viewport.1);
        assert!(m.scroll_press(Target::Scrollbar, track.top + 5.0));
        assert!(m.scroll_drag_to(track.bottom + 100.0));
        m.pointer_up();
        m.frame(2000, 1440, 2.0);
        assert!(m.at_end, "the end of the note is in view");
        assert!(m.scroll_view().unwrap().at >= 0.99);
        // And to the top again.
        m.scroll_press(Target::Scrollbar, track.top - 100.0);
        m.pointer_up();
        assert_eq!((m.scroll_line, m.scroll), (0, 0.0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_short_note_has_no_scroll_bar() {
        let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-short-bar", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Short.md"), "# Short\n\nA line.\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        assert!(m.scroll_view().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_scroll_bars_map_lists_the_headings_and_a_click_jumps_to_one() {
        let (dir, mut m) = long("map");
        let map: Vec<(String, u8)> = m.headings_for_map().iter().map(|h| (h.0.clone(), h.1)).collect();
        assert_eq!(map.len(), 8, "every heading but the title: {map:?}");
        assert_eq!(map[3], ("Part 3".to_string(), 2));
        let line = m.heading_line(5).unwrap();
        assert!(m.jump_to(line));
        m.frame(2000, 1440, 2.0);
        assert_eq!(m.scroll_line, line, "the heading is at the top of the view");
        assert_eq!(m.selection().head, m.editor.line_at(line).content(), "and the caret at its text");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_mouse_wheel_eases_in_and_a_trackpad_moves_at_once() {
        let (dir, mut m) = long("ease");
        m.wheel(300.0, false);
        assert_eq!((m.scroll_line, m.scroll), (0, 0.0), "a notch is not there at once");
        assert!(m.wants_frames());
        for _ in 0..200 {
            if !m.wants_frames() {
                break;
            }
            m.tick();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(m.ui.scroll_pending, 0.0, "it gets there");
        assert!(m.scroll_line > 0, "having moved the view");
        let there = (m.scroll_line, m.scroll);
        m.wheel(-100.0, true);
        assert_ne!((m.scroll_line, m.scroll), there, "a trackpad's move is there at once");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_wheel_over_the_side_menu_scrolls_the_file_tree() {
        let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-tree-wheel", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..80 {
            std::fs::write(dir.join(format!("Note {i:02}.md")), format!("# Note {i:02}\n")).unwrap();
        }
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        assert!(m.ui.tree_rows > 0 && m.ui.tree_rows < 80, "{}", m.ui.tree_rows);
        // The pointer over the side menu.
        let (r, _) = *m.hits.iter().find(|(_, t)| matches!(t, Target::Row(_))).unwrap();
        m.pointer_moved(r.left + 10.0, r.top + 5.0);
        assert!(m.wheel(24.0 * 5.0, true));
        assert_eq!(m.ui.tree_top, 5);
        m.frame(2000, 1440, 2.0);
        assert!(m.tree_bar().is_some(), "with a scroll bar of its own");
        assert!(m.wheel(24.0 * 500.0, true));
        assert_eq!(m.ui.tree_top, 80 - m.ui.tree_rows, "and stops at its end");
        let _ = std::fs::remove_dir_all(dir);
    }
}
