//! More than one caret: Alt+click adds one, Ctrl+D selects the next match
//! of the selection as well. Typing, Backspace, Delete, Enter and the
//! arrows act at every caret, one undo step for all of them; anything else
//! (a click, a format, a paste with structure, Esc) goes back to one caret,
//! the main one: the last added.

use super::*;

impl Editor {
    /// Every selection, the main one among them, in text order.
    pub fn selections(&self) -> Vec<Selection> {
        let mut all = self.extra.clone();
        all.push(self.selection);
        all.sort_by_key(|s| s.start());
        all
    }

    /// Whether there is more than one caret.
    pub fn has_carets(&self) -> bool {
        !self.extra.is_empty()
    }

    /// Back to the main caret alone. False if it was alone already.
    pub fn single_caret(&mut self) -> bool {
        let had = !self.extra.is_empty();
        self.extra.clear();
        had
    }

    /// Alt+click: a caret at `at` as well, which becomes the main one. On a
    /// caret already there (one of several), that caret goes instead.
    pub fn add_caret(&mut self, at: usize) {
        self.settle_marked();
        let at = self.rest(snap(&self.text, at), true, false);
        let all = self.selections();
        if all.len() > 1 && let Some(i) = all.iter().position(|s| s.is_caret() && s.head == at) {
            let mut rest = all;
            rest.remove(i);
            self.selection = rest.pop().expect("one was left");
            self.extra = rest;
            return;
        }
        self.extra.push(self.selection);
        self.selection = Selection::caret(at);
        self.goal_x = None;
        self.history.seal();
        self.merge_carets();
    }

    /// Ctrl+D: at a bare caret, its word is selected; with text selected,
    /// its next match (after the main selection, around past the end) is
    /// selected as well and becomes the main selection. False if there is
    /// no word, or no other match.
    pub fn add_next_match(&mut self) -> bool {
        self.settle_marked();
        let s = self.selection;
        if s.is_caret() {
            let extra = std::mem::take(&mut self.extra);
            self.select_word_at(s.head);
            self.extra = extra;
            self.merge_carets();
            return !self.selection.is_caret();
        }
        let needle = self.selected_text().to_string();
        let taken = self.selections();
        let free = |at: usize| taken.iter().all(|t| at + needle.len() <= t.start() || at >= t.end());
        let after = self.text[s.end()..].match_indices(&needle).map(|(i, _)| s.end() + i);
        let before = self.text[..s.start()].match_indices(&needle).map(|(i, _)| i);
        let Some(at) = after.chain(before).find(|&at| free(at)) else { return false };
        self.extra.push(s);
        self.selection = Selection { anchor: at, head: at + needle.len() };
        self.goal_x = None;
        self.history.seal();
        true
    }

    /// Does `f` (an edit made for one caret) at every caret, from the last
    /// in the text to the first so the earlier ones' offsets hold, as one
    /// undo step. A run of typed letters (`typing`) stays one step.
    pub(crate) fn at_each(&mut self, typing: bool, mut f: impl FnMut(&mut Editor)) {
        self.settle_marked();
        let all = self.selections();
        let main = all.iter().position(|s| *s == self.selection).unwrap_or(0);
        let grow = typing && self.history.multi_open;
        self.extra.clear();
        let steps = self.history.undo.len();
        // Placed after the caret being edited, in offsets now.
        let mut done: Vec<Selection> = Vec::with_capacity(all.len());
        for s in all.iter().rev() {
            self.selection = *s;
            self.goal_x = None;
            self.history.open = false;
            let len = self.text.len();
            f(self);
            let delta = self.text.len() as isize - len as isize;
            let shift = |at: usize| (at as isize + delta).max(0) as usize;
            for d in &mut done {
                *d = Selection { anchor: shift(d.anchor), head: shift(d.head) };
            }
            done.push(self.selection);
        }
        done.reverse();
        let changes: Vec<Change> = self.history.undo.drain(steps..).flat_map(|(_, c)| c).collect();
        if !changes.is_empty() {
            match self.history.undo.last_mut() {
                Some((Kind::Other, last)) if grow => last.extend(changes),
                _ => self.history.undo.push((Kind::Other, changes)),
            }
            self.history.redo.clear();
        }
        self.history.open = false;
        self.history.multi_open = typing;
        self.selection = done[main];
        self.extra = done;
        self.extra.remove(main);
        self.merge_carets();
    }

    /// Carets that met become one; the main one stays main.
    pub(crate) fn merge_carets(&mut self) {
        if self.extra.is_empty() {
            return;
        }
        let main = self.selection;
        let mut all = self.selections();
        all.dedup_by(|b, a| {
            let meets = b.start() < a.end() || (b.start() == a.end() && (a.is_caret() || b.is_caret()));
            if meets {
                let (start, end) = (a.start(), a.end().max(b.end()));
                *a = if a.anchor <= a.head { Selection { anchor: start, head: end } } else { Selection { anchor: end, head: start } };
            }
            meets
        });
        let i = all.iter().position(|s| s.start() <= main.start() && main.end() <= s.end()).unwrap_or(all.len() - 1);
        self.selection = all.remove(i);
        self.extra = all;
    }
}
