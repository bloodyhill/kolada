//! The selection bar: a small capsule of formatting that comes up over
//! text selected with the pointer (a drag, a double click) once the
//! button is let go, centred on the selection's first line, or under its
//! last where there is no room above. It goes on typing, on a click
//! elsewhere, on Escape, while scrolling, and when the selection collapses
//! or changes; the keys never bring it (Shift+arrows select while typing).
//!
//! The rules are kept here, in a state every shell holds the same way: it
//! says when the pointer let go of a selection (`pointer_selected`), when
//! something sends the bar away (`dismiss`), and before each frame what
//! the selection is (`follow`).

use std::time::{Duration, Instant};

use windows_core::Result;

use crate::chrome::{self, BAR, BTN, BTN_GAP, INSET, Painter};
use crate::scene::{Eng, Fonts, Input, Rect, Scene, Target, Tool};

/// Its buttons, left to right. Link opens the link field; Heading steps
/// through the heading levels.
pub const TOOLS: [Tool; 6] = [Tool::Bold, Tool::Italic, Tool::Highlight, Tool::Heading, Tool::Link, Tool::Code];
/// How long it takes to come in, rising `RISE` DIPs as it fades in.
pub const APPEAR: Duration = Duration::from_millis(120);
const RISE: f32 = 4.0;
/// Between it and the selection, DIPs.
const GAP: f32 = 8.0;
/// How far above the note column's top it may go: the dates' row and a
/// little more, below the toolbar's row. Higher, it goes under the
/// selection instead.
const ABOVE_COLUMN: f32 = 48.0;
/// The window's edges it keeps from, DIPs.
const EDGE: f32 = 8.0;

/// Its width: its buttons in a capsule.
pub const WIDTH: f32 = 2.0 * INSET + TOOLS.len() as f32 * BTN + (TOOLS.len() - 1) as f32 * BTN_GAP;

/// Whether the bar is up, over which selection, and since when.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelBar {
    over: Option<(usize, usize)>,
    since: Option<Instant>,
}

/// A selection's ends, start first.
fn ordered((a, b): (usize, usize)) -> (usize, usize) {
    (a.min(b), a.max(b))
}

impl SelBar {
    /// The pointer let go of a selection it made (`selection`: anchor and
    /// head): the bar comes up over it, unless it is empty. Whether the
    /// screen changes.
    pub fn pointer_selected(&mut self, selection: (usize, usize), now: Instant) -> bool {
        let (from, to) = ordered(selection);
        if from == to {
            return self.dismiss();
        }
        self.over = Some((from, to));
        self.since = Some(now);
        true
    }

    /// One of its buttons was used: it stays, over the selection as the
    /// change left it (marks moved it), or goes if that is empty.
    pub fn applied(&mut self, selection: (usize, usize)) -> bool {
        let (from, to) = ordered(selection);
        if self.over.is_none() {
            return false;
        }
        if from == to {
            return self.dismiss();
        }
        self.over = Some((from, to));
        true
    }

    /// Typing, a click elsewhere, Escape, a scroll: it goes. Whether it was up.
    pub fn dismiss(&mut self) -> bool {
        self.since = None;
        self.over.take().is_some()
    }

    /// The selection is `selection` now: the bar goes unless it is still the
    /// one it came up over (a key moved it, text replaced it, it collapsed).
    /// Whether it went.
    pub fn follow(&mut self, selection: (usize, usize)) -> bool {
        if self.over.is_some_and(|o| o != ordered(selection)) { self.dismiss() } else { false }
    }

    pub fn shown(&self) -> bool {
        self.over.is_some()
    }

    /// How far it has come in at `now`, 0..1 (all the way at once when
    /// motion is off); None while it is away.
    pub fn view(&self, now: Instant, animate: bool) -> Option<f32> {
        self.over?;
        let since = self.since?;
        Some(if animate { (now.saturating_duration_since(since).as_secs_f32() / APPEAR.as_secs_f32()).min(1.0) } else { 1.0 })
    }

    /// It is still coming in.
    pub fn moving(&self, now: Instant) -> bool {
        self.over.is_some() && self.since.is_some_and(|t| now.saturating_duration_since(t) < APPEAR)
    }
}

/// Where the bar goes for a selection whose first line's selected part is
/// `first` and last line's `last`, in a window `width` x `height` whose
/// note column starts at `column_top`: above the first line, centred on
/// it; under the last line where there is no room above. Inside the
/// window either way. True when above.
pub fn place(first: Rect, last: Rect, column_top: f32, width: f32, height: f32) -> (Rect, bool) {
    let above = first.top - GAP - BAR >= (column_top - ABOVE_COLUMN).max(EDGE);
    let (line, top) = if above { (first, first.top - GAP - BAR) } else { (last, last.bottom + GAP) };
    let top = top.min(height - EDGE - BAR).max(EDGE);
    let left = ((line.left + line.right) / 2.0 - WIDTH / 2.0).min(width - EDGE - WIDTH).max(EDGE);
    (Rect::new(left, top, left + WIDTH, top + BAR), above)
}

/// Draws the bar over the selection the page drew (`Scene::selection_span`),
/// as far as it has come in (`Input::selection_bar`). Not while the link
/// field, a menu or a note over the page is open.
pub fn draw(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    let Some(appear) = input.selection_bar else { return Ok(()) };
    if input.link_field.is_some() || input.menu.is_some() || input.popup.is_some() {
        return Ok(());
    }
    let Some((first, last)) = scene.selection_span else { return Ok(()) };
    let (r, _) = place(first, last, scene.column.top, width, height);
    let a = chrome::ease(appear);
    let r = r.offset(0.0, RISE * (1.0 - a));
    scene.overlay(None);
    let (fills_from, texts_from) = (scene.fills.len(), scene.texts.len());
    let mut p = Painter { eng, fonts, hover: input.hover, scene };
    p.capsule(r);
    p.scene.hits.push((r, Target::SelBarPanel));
    let mut x = r.left + INSET;
    for tool in TOOLS {
        let b = Rect::new(x, r.top + INSET, x + BTN, r.top + INSET + BTN);
        p.tool_button(b, Target::SelBar(tool), chrome::tool_glyph(tool), input.active_tools.contains(&tool))?;
        x += BTN + BTN_GAP;
    }
    if a < 1.0 {
        scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= a);
        scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= a);
    }
    scene.selbar = Some(r);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_comes_for_a_pointer_selection_and_goes_when_the_selection_changes() {
        let now = Instant::now();
        let mut bar = SelBar::default();
        assert!(!bar.pointer_selected((4, 4), now) && !bar.shown(), "nothing selected, nothing to format");
        assert!(bar.pointer_selected((9, 4), now) && bar.shown());
        assert_eq!(bar.view(now, false), Some(1.0));
        assert_eq!(bar.view(now, true), Some(0.0), "it fades in");
        assert_eq!(bar.view(now + APPEAR, true), Some(1.0));
        assert!(bar.moving(now) && !bar.moving(now + APPEAR));
        assert!(!bar.follow((4, 9)), "the same selection, either way round");
        assert!(bar.follow((4, 10)) && !bar.shown(), "Shift+Right moved it: it goes");
        // Typing replaced the selection: the caret alone is left.
        bar.pointer_selected((4, 9), now);
        assert!(bar.follow((5, 5)) && bar.view(now, true).is_none());
    }

    #[test]
    fn the_keys_never_bring_it() {
        // Only a pointer brings it: following a selection made with the
        // keys leaves it away.
        let mut bar = SelBar::default();
        for sel in [(0, 1), (0, 2), (0, 7)] {
            assert!(!bar.follow(sel));
            assert!(!bar.shown());
        }
    }

    #[test]
    fn a_button_keeps_it_over_the_selection_the_change_left() {
        let now = Instant::now();
        let mut bar = SelBar::default();
        bar.pointer_selected((4, 7), now);
        // Bold wrapped the word: its marks moved the selection along.
        assert!(bar.applied((6, 9)));
        assert!(!bar.follow((6, 9)) && bar.shown());
        assert!(bar.applied((6, 6)) && !bar.shown(), "nothing left selected");
        assert!(!SelBar::default().applied((1, 3)), "it does not come back for a button");
        assert!(bar.pointer_selected((1, 3), now) && bar.dismiss() && !bar.shown(), "Escape, a scroll, a click elsewhere");
    }

    #[test]
    fn it_sits_above_the_first_line_and_under_the_last_without_room() {
        let (column_top, width, height) = (96.0, 1000.0, 700.0);
        let first = Rect::new(300.0, 200.0, 500.0, 224.0);
        let last = Rect::new(120.0, 248.0, 260.0, 272.0);
        let (r, above) = place(first, last, column_top, width, height);
        assert!(above && r.bottom <= first.top - GAP + 0.01 && r.height() == BAR);
        assert!(((r.left + r.right) / 2.0 - 400.0).abs() < 0.01, "centred on the first line: {r:?}");
        // Scrolled to the window's top: no room above, so under the last line.
        let first = Rect::new(300.0, 60.0, 500.0, 84.0);
        let (r, above) = place(first, last, column_top, width, height);
        assert!(!above && r.top >= last.bottom + GAP - 0.01, "{r:?}");
        assert!(((r.left + r.right) / 2.0 - 190.0).abs() < 0.01, "centred on the last line: {r:?}");
        // At a window's edges it stays inside.
        let corner = Rect::new(2.0, 300.0, 30.0, 324.0);
        let (r, _) = place(corner, corner, column_top, 360.0, height);
        assert!(r.left >= EDGE && r.right <= 360.0 - EDGE);
        let foot = Rect::new(100.0, 40.0, 200.0, 64.0);
        let (r, _) = place(foot, Rect::new(100.0, 680.0, 200.0, 704.0), column_top, width, height);
        assert!(r.bottom <= height - EDGE && r.top >= EDGE);
    }
}
