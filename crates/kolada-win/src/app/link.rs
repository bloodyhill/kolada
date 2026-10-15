//! Links without seeing addresses (the shared behaviour is
//! `kolada_ui::linkfield`): Ctrl+K and the toolbar's Link open a small
//! field under the selection. A note's name (its suggestions under it, Up
//! and Down to pick) or a web address, then Enter, links the selected
//! words; on a link it opens filled, with Remove. Esc puts it away. Resting
//! the pointer on a link for 400 ms shows where it goes in a faint card.

use kolada_ui::linkfield::{self, LinkField};

use super::*;

/// Notes suggested at most.
const SHOWN: usize = 5;

/// The timer that shows the address card once the pointer has rested.
pub(super) const LINK_TIMER: usize = 52;

pub struct LinkState {
    pub field: LinkField,
    /// Notes whose names fit what is typed: name and folder.
    pub names: Vec<(String, String)>,
}

impl LinkState {
    pub fn view(&self) -> scene::LinkFieldView<'_> {
        scene::LinkFieldView {
            query: self.field.query.text(),
            caret: self.field.query.selection().head,
            names: &self.names,
            picked: self.field.picked,
            existing: self.field.existing,
        }
    }
}

/// Where the pointer rests on a link: what the link leads to, and since
/// when it showed (None until the timer says so).
pub struct LinkHover {
    pub address: String,
    pub shown: bool,
    pub at: (f32, f32),
}

impl App {
    /// Opens the link field for the selection (Ctrl+K, the toolbar's Link).
    pub(super) fn open_link_field(&mut self) {
        if self.read_only || self.scrub_page().is_some() {
            return;
        }
        self.close_find();
        self.close_menu();
        self.suggest = None;
        self.link_hover = None;
        self.link = Some(LinkState { field: LinkField::open(&self.editor), names: Vec::new() });
        self.refresh_link_names();
        self.request_frame();
    }

    pub(super) fn close_link_field(&mut self) {
        if self.link.take().is_some() {
            self.request_frame();
        }
    }

    /// The notes that fit what is typed (none for a web address).
    fn refresh_link_names(&mut self) {
        let Some(l) = &self.link else { return };
        let names: Vec<(String, String)> = match l.field.note_typed() {
            Some(typed) if !typed.is_empty() => search::complete(&self.index, typed, SHOWN)
                .into_iter()
                .map(|n| {
                    let folder = n.path.rsplit_once('/').map_or(String::new(), |(f, _)| f.to_string());
                    (n.name.clone(), folder)
                })
                .collect(),
            _ => Vec::new(),
        };
        if let Some(l) = &mut self.link {
            l.field.picked = l.field.picked.filter(|&i| i < names.len());
            l.names = names;
        }
    }

    /// Enter, or a suggestion clicked: the link is written.
    fn link_commit(&mut self) {
        let Some(l) = self.link.take() else { return };
        let to = match l.field.picked.and_then(|i| l.names.get(i)) {
            Some((name, _)) => name.clone(),
            None => l.field.query.text().to_string(),
        };
        if to.trim().is_empty() {
            // Nothing typed: on a link, Enter leaves it as it is.
            self.request_frame();
            return;
        }
        self.edit(|e| {
            l.field.apply(e, &to);
        });
        self.request_frame();
    }

    /// Keys while the link field is open; false for keys it leaves alone.
    pub(super) fn link_key(&mut self, vk: u16, ctrl: bool, shift: bool) -> bool {
        let k = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| key.0 == vk;
        let Some(l) = &mut self.link else { return false };
        let rows = l.names.len();
        if k(VK_ESCAPE) || (ctrl && vk == b'K' as u16) {
            self.close_link_field();
            return true;
        }
        if k(VK_RETURN) || (k(VK_TAB) && l.field.picked.is_some()) {
            self.link_commit();
            return true;
        }
        if (k(VK_DOWN) || k(VK_UP)) && rows > 0 {
            l.field.picked = Some(match (l.field.picked, k(VK_DOWN)) {
                (None, true) => 0,
                (None, false) => rows - 1,
                (Some(i), true) => (i + 1) % rows,
                (Some(i), false) => (i + rows - 1) % rows,
            });
            self.request_frame();
            return true;
        }
        let before = l.field.query.version();
        let q = &mut l.field.query;
        let motion = |m: Motion, e: &mut Editor| {
            let plan = e.plan(m, shift, &OneLine(e.text()));
            e.apply(plan);
        };
        if k(VK_LEFT) {
            motion(if ctrl { Motion::WordLeft } else { Motion::Left }, q);
        } else if k(VK_RIGHT) {
            motion(if ctrl { Motion::WordRight } else { Motion::Right }, q);
        } else if k(VK_HOME) {
            motion(Motion::DocStart, q);
        } else if k(VK_END) {
            motion(Motion::DocEnd, q);
        } else if k(VK_BACK) {
            q.backspace(ctrl);
        } else if k(VK_DELETE) {
            q.delete(ctrl);
        } else if ctrl && vk == b'A' as u16 {
            q.select_all();
        } else if ctrl && vk == b'V' as u16 {
            if let Some(text) = clipboard::get(self.hwnd) {
                let line = text.trim().lines().next().unwrap_or("").to_string();
                if let Some(l) = &mut self.link {
                    l.field.query.insert(&line);
                }
            }
        } else {
            // Typing comes as characters. The page's shortcuts wait until
            // the field is put away: an undo under it would move the words
            // it links.
            return true;
        }
        if self.link.as_ref().is_some_and(|l| l.field.query.version() != before) {
            if let Some(l) = &mut self.link {
                l.field.picked = None;
            }
            self.refresh_link_names();
        }
        self.request_frame();
        true
    }

    /// A character typed into the link field; false when it is not open.
    pub(super) fn link_typed(&mut self, s: &str) -> bool {
        let Some(l) = &mut self.link else { return false };
        l.field.query.insert(s);
        l.field.picked = None;
        self.refresh_link_names();
        self.request_frame();
        true
    }

    /// A click on the link field's card.
    pub(super) fn link_click(&mut self, target: Target) {
        match target {
            Target::LinkRemove => {
                if let Some(l) = self.link.take() {
                    self.edit(|e| {
                        l.field.remove(e);
                    });
                }
            }
            Target::LinkSuggestion(i) => {
                if let Some(l) = &mut self.link {
                    l.field.picked = Some(i);
                }
                self.link_commit();
            }
            _ => {}
        }
        self.request_frame();
    }

    /// The pointer moved, over the note's text (`over_note`) at `p` DIPs:
    /// resting on a link starts the wait for its address card; leaving it
    /// puts the card away.
    pub(super) fn link_hover_moved(&mut self, over_note: bool, p: (f32, f32)) {
        let address = if over_note && self.link.is_none() && self.popup.is_none() && self.menu.is_none() && self.scrub_page().is_none() {
            let at = self.offset_at(p.0, p.1);
            // Past the end of a line that ends in a link is not on it: the
            // offset there stays put as the pointer moves left.
            let line_end = self.editor.line_at(at).end;
            let past = at == line_end && self.offset_at(p.0 - 6.0, p.1) == at;
            if past { None } else { linkfield::address_at(self.editor.text(), at) }
        } else {
            None
        };
        let same = match (&self.link_hover, &address) {
            (Some(h), Some(a)) => h.address == *a,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        if self.link_hover.as_ref().is_some_and(|h| h.shown) {
            self.request_frame();
        }
        self.link_hover = address.map(|address| LinkHover { address, shown: false, at: p });
        if self.link_hover.is_some() {
            self.start_timer(LINK_TIMER, linkfield::ADDRESS_DELAY_MS);
        } else {
            self.stop_timer(LINK_TIMER);
        }
    }

    /// The pointer has rested on a link: its card shows, where the pointer
    /// is now.
    pub(super) fn link_hover_rested(&mut self) {
        self.stop_timer(LINK_TIMER);
        let pointer = self.ui.pointer;
        if let Some(h) = &mut self.link_hover {
            h.shown = true;
            if let Some(p) = pointer {
                h.at = p;
            }
            self.request_frame();
        }
    }

    /// Typing, clicking or leaving puts the card away.
    pub(super) fn hide_link_card(&mut self) {
        if let Some(h) = self.link_hover.take() {
            self.stop_timer(LINK_TIMER);
            if h.shown {
                self.request_frame();
            }
        }
    }
}
