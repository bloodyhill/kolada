//! Scrolling the page: where it ends, and the scroll bars of the page and of
//! the note open over it.
//!
//! The page lays out only what is on screen, so the scroll bar cannot know
//! every paragraph's height: it estimates them from their lengths (cheap
//! enough to redo on every keystroke), and measures exactly only where the
//! note ends, which is what stops the scroll.

use super::*;

/// Estimated heights for the scroll bar, and the exact end of the note, each
/// kept with what it was measured for.
#[derive(Default)]
pub struct Heights {
    /// Text version, column width, text size.
    key: (u64, u32, u32),
    /// Height above each line, then the whole note's.
    sums: Vec<f32>,
    /// The same and the view's height.
    end_key: (u64, u32, u32, u32),
    end: (usize, f32),
    /// The headings for the scroll bar's map, and what they are for.
    map_key: Option<((u64, u32, u32), u32)>,
    map: Vec<(String, u8, usize, f32)>,
    /// Where the page's scroll bar was and how long, when last shown: a
    /// hidden bar keeps them rather than measure the whole note each key.
    bar: (f32, f32),
}

/// A scroll bar being dragged (its target: the page's, the popup's or the
/// file tree's), and where on the thumb it was grabbed (DIPs from its top).
#[derive(Clone, Copy)]
pub struct Drag {
    pub bar: Target,
    grab: f32,
}

/// A scroll bar shows this long after scrolling, then fades out over the
/// second.
pub const SHOWN: Duration = Duration::from_millis(900);
pub const FADE: Duration = Duration::from_millis(350);

/// Pointer this close to the window's right edge brings the page's scroll
/// bar out, DIPs.
const NEAR_EDGE: f32 = 48.0;

/// How visible a scroll bar is after scrolling at `t`: fully for a moment,
/// then fading.
fn after_scroll(t: Option<Instant>) -> f32 {
    let Some(t) = t else { return 0.0 };
    let e = t.elapsed();
    if e < SHOWN { 1.0 } else { 1.0 - ((e - SHOWN).as_secs_f32() / FADE.as_secs_f32()).min(1.0) }
}

impl App {
    fn heights_key(&self) -> (u64, u32, u32) {
        let size = self.fonts.body.size;
        (self.page_version(), self.column.width().to_bits(), size.to_bits())
    }

    /// Each line's height, estimated from its length and block, summed.
    fn estimate(&self) -> Vec<f32> {
        let size = self.fonts.body.size;
        let width = self.column.width().max(80.0);
        let text = self.editor.text();
        let lines = self.editor.outline();
        let title = crate::note::title_line(text, lines);
        let mut sums = Vec::with_capacity(lines.len() + 1);
        let mut y = 0.0;
        sums.push(y);
        for (i, l) in lines.iter().enumerate() {
            let (s, spacing, pad) = match l.block {
                _ if Some(i) == title => (size * scene::HEADING_SCALE[0], 1.3, 0.0),
                Block::Heading(n) => {
                    (size * scene::HEADING_SCALE[(n.clamp(1, 6) - 1) as usize], 1.3, [18.0, 14.0].get(n as usize - 1).copied().unwrap_or(10.0))
                }
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
        if self.heights.borrow().key != key || self.heights.borrow().sums.is_empty() {
            let sums = self.estimate();
            let mut h = self.heights.borrow_mut();
            h.key = key;
            h.sums = sums;
        }
        std::cell::Ref::map(self.heights.borrow(), |h| &h.sums)
    }

    /// Where the view is when the note's end sits at the bottom of the
    /// window (with room for the caret): the paragraph at the top and how
    /// far it is scrolled. The top of the note if it all fits.
    pub(super) fn scroll_end(&self) -> (usize, f32) {
        let view = self.view_height();
        let (v, w, s) = self.heights_key();
        let key = (v, w, s, view.to_bits());
        if self.heights.borrow().end_key == key {
            return self.heights.borrow().end;
        }
        let room = (view - self.column.top - CARET_MARGIN).max(PAGE_LINE);
        let nt = self.note_text();
        let mut end = (0, 0.0);
        let mut sum = 0.0;
        for i in (0..nt.lines.len()).rev() {
            let Ok(p) = nt.para(i) else { break };
            sum += nt.block_height(&p).unwrap_or(0.0);
            if sum > room {
                end = (p.start, sum - room);
                break;
            }
        }
        let mut h = self.heights.borrow_mut();
        h.end_key = key;
        h.end = end;
        end
    }

    /// Keeps the view from resting past the note's end, after the text got
    /// shorter under it. Only measures when the end is in view.
    pub(super) fn clamp_scroll(&mut self) {
        let text = self.editor.text();
        if self.lines.last().is_none_or(|l| l.para.end < text.len()) && self.session.scroll_line as usize <= text.len() {
            return;
        }
        let end = self.scroll_end();
        if (self.session.scroll_line as usize, self.session.scroll_px) > end {
            self.session.scroll_line = end.0 as u64;
            self.session.scroll_px = end.1;
        }
    }

    /// The page's scroll bar, if the note does not fit the window.
    pub(super) fn scroll_view(&self, (width, height, dpi): (u32, u32, u32)) -> Option<ScrollView> {
        let text = self.editor.text();
        let view = height as f32 * 96.0 / dpi as f32;
        let at_top = self.session.scroll_line == 0 && self.session.scroll_px <= 0.0;
        // The last frame showed the end of the note, all of it inside the
        // window: at the end (or, from the top, it all fits).
        let end_shown = self.lines.last().is_some_and(|l| l.para.end >= text.len() && l.y + l.height <= view - CARET_MARGIN + 1.0);
        if at_top && end_shown {
            return None;
        }
        let width = width as f32 * 96.0 / dpi as f32;
        let near = self.ui.pointer.is_some_and(|(x, y)| x > width - NEAR_EDGE && y > self.column.top);
        let dragged = self.scroll_drag.is_some_and(|d| d.bar == Target::Scrollbar);
        let alpha = if near || dragged { 1.0 } else if self.popup.is_none() { after_scroll(self.scrolled_at) } else { 0.0 };
        // Hidden (while typing): where it was, without measuring the note.
        let (last_at, last_size) = self.heights.borrow().bar;
        if alpha <= 0.0 && last_size > 0.0 {
            return Some(ScrollView { at: last_at, size: last_size, alpha });
        }
        let sums = self.estimated();
        let lines = self.editor.outline();
        let total = sums.last().copied().unwrap_or(0.0).max(1.0);
        let room = (view - self.column.top - CARET_MARGIN).max(PAGE_LINE);
        if total <= room && at_top && self.lines.is_empty() {
            return None;
        }
        let i = lines.partition_point(|l| l.start < self.session.scroll_line as usize);
        let top = sums.get(i).copied().unwrap_or(0.0) + self.session.scroll_px;
        let max = (total - room).max(1.0);
        let at = if end_shown { 1.0 } else { (top / max).min(0.99) };
        let size = (room / total).clamp(0.04, 1.0);
        drop(sums);
        self.heights.borrow_mut().bar = (at, size);
        Some(ScrollView { at, size, alpha })
    }

    /// How visible the popup's scroll bar is: while the pointer is on its
    /// card, and for a moment after it scrolls.
    pub(super) fn popup_bar_alpha(&self) -> f32 {
        let over = matches!(
            self.ui.hover,
            Some(Target::PopupCard | Target::PopupText | Target::PopupOpen | Target::PopupClose | Target::PopupCheckbox(_) | Target::PopupScrollbar)
        );
        if over { 1.0 } else { after_scroll(self.scrolled_at) }
    }

    /// The page's scroll bar track, thumb position and size, if it has one.
    fn page_bar(&self) -> Option<(Rect, f32, f32)> {
        let view = window::view(self.hwnd);
        let bar = self.scroll_view(view)?;
        let (w, h) = (view.0 as f32 * 96.0 / view.2 as f32, view.1 as f32 * 96.0 / view.2 as f32);
        Some((chrome::scroll_track(self.column, w, h), bar.at, bar.size))
    }

    fn popup_bar(&self) -> Option<(Rect, f32, f32)> {
        let d = self.popup.as_ref()?.drawn.as_ref()?;
        let max = d.height - d.content.height();
        (max > 1.0).then(|| (scene::popup_track(d.card, d.content), self.popup.as_ref().map_or(0.0, |p| p.scroll) / max, d.content.height() / d.height))
    }

    /// The file tree's scroll bar: its track, where it is, how much shows.
    fn tree_bar(&self) -> Option<(Rect, f32, f32)> {
        let track = self.tree_track?;
        let (rows, fit) = (self.session.sidebar.len(), self.tree_rows);
        (rows > fit && fit > 0).then(|| (track, self.ui.tree_top as f32 / (rows - fit) as f32, fit as f32 / rows as f32))
    }

    fn bar(&self, bar: Target) -> Option<(Rect, f32, f32)> {
        match bar {
            Target::PopupScrollbar => self.popup_bar(),
            Target::TreeScrollbar => self.tree_bar(),
            _ => self.page_bar(),
        }
    }

    /// The button went down on a scroll bar at `y` DIPs: on the thumb it is
    /// grabbed; elsewhere on the track, the thumb jumps there first.
    pub(super) fn scroll_press(&mut self, bar: Target, y: f32) {
        let Some((track, at, size)) = self.bar(bar) else { return };
        let (top, h) = chrome::thumb(track, at, size);
        let grab = if (top..top + h).contains(&y) { y - top } else { h / 2.0 };
        self.scroll_drag = Some(Drag { bar, grab });
        unsafe { SetCapture(self.hwnd) };
        self.scroll_drag_to(y);
    }

    /// A grabbed thumb follows the pointer at `y` DIPs.
    pub(super) fn scroll_drag_to(&mut self, y: f32) {
        let Some(drag) = self.scroll_drag else { return };
        let Some((track, at, size)) = self.bar(drag.bar) else { return };
        let (_, h) = chrome::thumb(track, at, size);
        let f = ((y - drag.grab - track.top) / (track.height() - h).max(1.0)).clamp(0.0, 1.0);
        match drag.bar {
            Target::PopupScrollbar => {
                if let Some(p) = &mut self.popup
                    && let Some(d) = &p.drawn
                {
                    p.scroll = f * (d.height - d.content.height()).max(0.0);
                }
            }
            Target::TreeScrollbar => {
                let max = self.session.sidebar.len().saturating_sub(self.tree_rows);
                self.ui.tree_top = (f * max as f32).round() as usize;
            }
            _ => self.scroll_to_fraction(f),
        }
        self.request_frame();
    }

    /// The note's headings for the scroll bar's map: text, level, the line
    /// each starts, and where scrolling puts it at the top (0..1).
    pub(super) fn headings(&self) -> Vec<(String, u8, usize, f32)> {
        let lines = self.editor.outline();
        let text = self.editor.text();
        let sums = self.estimated();
        let room = (self.view_height() - self.column.top - CARET_MARGIN).max(PAGE_LINE);
        let max = (sums.last().copied().unwrap_or(0.0) - room).max(1.0);
        let title = crate::note::title_line(text, lines);
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
        let key = (self.heights_key(), self.view_height().to_bits());
        if self.heights.borrow().map_key != Some(key) {
            let found = self.headings();
            let mut h = self.heights.borrow_mut();
            h.map_key = Some(key);
            h.map = found;
        }
        std::cell::Ref::map(self.heights.borrow(), |h| &h.map)
    }

    /// Jumps to the heading on the line starting at `line`, with its line at
    /// the top of the view (or as near as the note's end allows).
    pub(super) fn jump_to(&mut self, line: usize) {
        self.scroll_pending = 0.0;
        self.session.scroll_line = line as u64;
        self.session.scroll_px = 0.0;
        let end = self.scroll_end();
        if (line, 0.0) > end {
            self.session.scroll_line = end.0 as u64;
            self.session.scroll_px = end.1;
        }
        self.editor.set_selection(Selection::caret(line));
        self.caret_moved(false);
        self.reveal_caret = false;
        self.scrolled_at = Some(Instant::now());
        self.request_frame();
    }

    /// Scrolls the page to `f` of the way from its top (0) to its end (1).
    fn scroll_to_fraction(&mut self, f: f32) {
        self.scroll_pending = 0.0;
        if f >= 1.0 {
            let end = self.scroll_end();
            self.session.scroll_line = end.0 as u64;
            self.session.scroll_px = end.1;
            return;
        }
        let (line, px) = {
            let sums = self.estimated();
            let lines = self.editor.outline();
            let view = self.view_height();
            let room = (view - self.column.top - CARET_MARGIN).max(PAGE_LINE);
            let target = f * (sums.last().copied().unwrap_or(0.0) - room).max(0.0);
            let i = sums.partition_point(|&s| s <= target).saturating_sub(1).min(lines.len().saturating_sub(1));
            (lines.get(i).map_or(0, |l| l.start), target - sums.get(i).copied().unwrap_or(0.0))
        };
        self.session.scroll_line = line as u64;
        self.session.scroll_px = 0.0;
        // Walks the rest exactly, and stops at the end.
        self.scroll_by(px);
    }
}
