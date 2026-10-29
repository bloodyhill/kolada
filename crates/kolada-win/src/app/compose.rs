//! Text composed with an input method (Japanese, Chinese, Korean; the emoji
//! panel; see `ime`): shown where it is going as marked text, underlined,
//! and settled into the field as typing once the input method is done. The
//! note, a note open over it and search take it this way; elsewhere (the
//! find bar) the input method's own window shows it, and what it settles on
//! arrives as typed characters.

use crate::ime;

use super::*;

/// Where composed text goes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Field {
    Note,
    Popup,
    Search,
}

impl App {
    /// The field an input method composes into, if it shows marked text.
    pub(super) fn compose_field(&mut self) -> Option<Field> {
        // The past is looked at, not typed into.
        if self.scrub_page().is_some() {
            return None;
        }
        if self.find.as_mut().is_some_and(|f| f.field().is_some()) {
            return None;
        }
        if self.popup.is_some() {
            return Some(Field::Popup);
        }
        if self.search_has_keys() {
            return Some(Field::Search);
        }
        (!self.read_only).then_some(Field::Note)
    }

    fn composed_editor(&mut self, field: Field) -> Option<&mut Editor> {
        match field {
            Field::Note => Some(&mut self.editor),
            Field::Popup => self.popup.as_mut().map(|p| &mut p.editor),
            Field::Search => self.search.as_mut().map(|s| &mut s.query),
        }
    }

    /// The input method's text changed: `result` is settled, `composing`
    /// still being written. False if no field here shows marked text (the
    /// input method's window does).
    pub(super) fn ime_composed(&mut self, result: Option<String>, composing: Option<ime::Composing>) -> bool {
        let Some(field) = self.compose_field() else { return false };
        if let Some(s) = result {
            self.ime_target = None;
            match field {
                Field::Note => {
                    self.leave_picture_for_text();
                    self.edit(|e| e.commit_marked(&s));
                }
                Field::Popup => self.popup_edit(|e| e.commit_marked(&s)),
                Field::Search => {
                    if let Some(search) = &mut self.search {
                        search.query.commit_marked(&s);
                    }
                    self.run_search();
                }
            }
        }
        if let Some(c) = composing {
            if field == Field::Note && self.editor.marked().is_none() {
                self.leave_picture_for_text();
            }
            self.ime_target = c.target;
            if let Some(e) = self.composed_editor(field) {
                e.set_marked(&c.text, c.cursor);
            }
            match field {
                Field::Note => self.caret_moved(false),
                Field::Popup => {
                    if let Some(p) = &mut self.popup {
                        p.reveal = true;
                    }
                }
                Field::Search => self.run_search(),
            }
        }
        self.request_frame();
        true
    }

    /// The input method is done composing. Text it never settled (the
    /// composition was cancelled, or emptied) goes, and what it replaced
    /// comes back.
    pub(super) fn ime_ended(&mut self) {
        self.ime_target = None;
        let mut changed = false;
        for field in [Field::Note, Field::Popup, Field::Search] {
            if let Some(e) = self.composed_editor(field)
                && e.marked().is_some()
            {
                e.cancel_marked();
                changed = true;
            }
        }
        if changed {
            if self.search.is_some() {
                self.run_search();
            }
            self.request_frame();
        }
    }

    /// Whether anything is being composed, in any field.
    pub(super) fn composing(&self) -> bool {
        self.editor.marked().is_some() || self.popup.as_ref().is_some_and(|p| p.editor.marked().is_some()) || self.search.as_ref().is_some_and(|s| s.query.marked().is_some())
    }

    /// Settles a composition as it stands, before something else happens
    /// to the text (a click elsewhere, another note): its text is committed
    /// here, then the input method is told to drop it. (Told to complete
    /// instead, it would send the text back while this is still running,
    /// and the message would be lost.)
    pub(super) fn settle_composition(&mut self) {
        if !self.composing() {
            return;
        }
        self.ime_target = None;
        for field in [Field::Note, Field::Popup, Field::Search] {
            if let Some(e) = self.composed_editor(field)
                && let Some((a, b)) = e.marked()
            {
                let text = e.text()[a..b].to_string();
                e.commit_marked(&text);
            }
        }
        ime::cancel(self.hwnd);
        self.request_frame();
    }

    /// After a frame: the input method's candidate list goes beside the
    /// caret of the field being composed in.
    pub(super) fn place_candidates(&mut self) {
        if !self.composing() {
            return;
        }
        let caret = match self.compose_field() {
            Some(Field::Note) => self.caret_target,
            Some(Field::Popup) => self.popup.as_ref().and_then(|p| p.drawn.as_ref()).and_then(|d| d.caret),
            Some(Field::Search) => self.hits.iter().find(|(_, t)| *t == Target::Search).map(|(r, _)| *r),
            None => None,
        };
        let Some(c) = caret else { return };
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let px = |v: f32| (v * scale).round() as i32;
        ime::place(self.hwnd, windows::Win32::Foundation::RECT { left: px(c.left), top: px(c.top), right: px(c.right), bottom: px(c.bottom) });
    }

    /// What the page underlines: the text being composed in the note, and
    /// the part of it being converted.
    pub(super) fn note_marked(&self) -> Option<scene::Composed> {
        let (start, end) = self.editor.marked()?;
        Some(((start, end), self.ime_target.map(|(a, b)| (start + a, (start + b).min(end)))))
    }
}
