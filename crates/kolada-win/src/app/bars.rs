//! The formatting bars' moments (what they look like is `kolada_ui::chrome`
//! `toolbar`, `reading`, `tooltip` and `kolada_ui::selbar`): the toolbar's
//! Aa card opening and closing, a button's name after the pointer rests on
//! it, and the selection bar over text selected with the mouse, gone on
//! typing, a click elsewhere, Escape and scrolling.

use kolada_ui::selbar::SelBar;

use super::*;

/// The timer that shows a button's name once the pointer has rested on it.
pub(super) const TOOLTIP_TIMER: usize = 53;
const TOOLTIP_DELAY_MS: u32 = 500;
/// The Aa card fades in over this long.
const READING_FADE: Duration = Duration::from_millis(120);

/// The bars' state that is not saved.
#[derive(Default)]
pub struct Bars {
    /// The selection bar.
    pub selbar: SelBar,
    /// The Aa card, open since then.
    pub reading: Option<Instant>,
    /// The button whose name shows, and the one the pointer rests on,
    /// waiting for it.
    pub tooltip: Option<Target>,
    pending: Option<Target>,
}

impl Bars {
    /// Something of theirs is moving: frames keep coming.
    pub fn moving(&self) -> bool {
        let now = Instant::now();
        self.selbar.moving(now) || self.reading.is_some_and(|t| t.elapsed() < READING_FADE)
    }

    /// How far the selection bar has come in, and the Aa card.
    pub fn views(&self) -> (Option<f32>, Option<f32>) {
        let reading = self.reading.map(|t| (t.elapsed().as_secs_f32() / READING_FADE.as_secs_f32()).min(1.0));
        (self.selbar.view(Instant::now(), true), reading)
    }
}

impl App {
    /// The pointer is over `target` now: a name that showed goes, and one
    /// for a button comes after the pointer rests there.
    pub(super) fn tooltip_hover(&mut self, target: Option<Target>) {
        if self.bars.tooltip.take().is_some() {
            self.request_frame();
        }
        let named = target.filter(|t| chrome::button_name(*t, scene::Platform::Windows, false).is_some());
        if named != self.bars.pending {
            self.bars.pending = named;
            if named.is_some() {
                self.start_timer(TOOLTIP_TIMER, TOOLTIP_DELAY_MS);
            } else {
                self.stop_timer(TOOLTIP_TIMER);
            }
        }
    }

    /// The pointer rested: its button says what it is.
    pub(super) fn tooltip_rested(&mut self) {
        self.stop_timer(TOOLTIP_TIMER);
        if self.bars.pending.is_some() && self.bars.pending == self.ui.hover {
            self.bars.tooltip = self.bars.pending;
            self.request_frame();
        }
    }

    /// A press, a key, the pointer leaving: no name shows, none waits.
    pub(super) fn tooltip_away(&mut self) {
        self.bars.pending = None;
        self.stop_timer(TOOLTIP_TIMER);
        if self.bars.tooltip.take().is_some() {
            self.request_frame();
        }
    }

    /// Aa: its card opens, or closes.
    pub(super) fn toggle_reading(&mut self) {
        self.bars.reading = if self.bars.reading.is_some() { None } else { Some(Instant::now()) };
        self.request_frame();
    }

    pub(super) fn close_reading(&mut self) {
        if self.bars.reading.take().is_some() {
            self.request_frame();
        }
    }

    /// The mouse let go of a selection it made on the page (a drag, a
    /// double or triple click): the selection bar comes up over it.
    pub(super) fn mouse_selected(&mut self) {
        if self.read_only || self.popup.is_some() || self.scrub_page().is_some() || self.link.is_some() {
            return;
        }
        let s = self.editor.selection();
        if self.bars.selbar.pointer_selected((s.anchor, s.head), Instant::now()) {
            self.request_frame();
        }
    }

    /// Escape, a scroll, a click elsewhere: the selection bar goes.
    /// Whether it was up.
    pub(super) fn dismiss_selbar(&mut self) -> bool {
        let gone = self.bars.selbar.dismiss();
        if gone {
            self.request_frame();
        }
        gone
    }

    /// A button on the selection bar: its formatting, as one step to undo;
    /// the bar stays over what is still selected. Link asks what to link
    /// to, in the link field, in the bar's place.
    pub(super) fn selbar_tool(&mut self, tool: Tool) {
        self.tooltip_away();
        if tool == Tool::Link {
            self.bars.selbar.dismiss();
            return self.open_link_field();
        }
        self.edit(|e| apply_tool(e, tool));
        let s = self.editor.selection();
        self.bars.selbar.applied((s.anchor, s.head));
        self.request_frame();
    }
}
