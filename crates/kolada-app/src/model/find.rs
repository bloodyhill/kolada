//! Finding (and replacing) in the note: Cmd+F. Matches are found as the query
//! is typed; the one that is on is the selection, so the page scrolls to it.

use kolada_editor::find;
use kolada_ui::scene::FindView;

use super::*;

/// Which field of the find bar has the keys.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Focus {
    /// The page does: the bar stays, its matches too.
    Page,
    Query,
    Replace,
}

pub(super) struct Find {
    pub query: Editor,
    pub replace: Option<Editor>,
    pub focus: Focus,
    pub matches: Vec<(usize, usize)>,
    /// Where along the note each match is (0..1), for marks beside the scroll bar.
    pub ticks: Vec<f32>,
    pub current: Option<usize>,
    /// The text version and query the matches are for.
    found_for: (u64, String),
}

impl Find {
    pub fn view(&self) -> FindView<'_> {
        FindView {
            query: self.query.text(),
            query_caret: self.query.selection().head,
            replace: self.replace.as_ref().map(|r| (r.text(), r.selection().head)),
            focus: match self.focus {
                Focus::Page => 0,
                Focus::Query => 1,
                Focus::Replace => 2,
            },
            count: self.matches.len(),
            current: self.current,
        }
    }

    fn field(&mut self) -> Option<&mut Editor> {
        match self.focus {
            Focus::Query => Some(&mut self.query),
            Focus::Replace => self.replace.as_mut(),
            Focus::Page => None,
        }
    }
}

impl Model {
    /// A field of the find bar has the keys.
    pub(super) fn find_focused(&self) -> bool {
        self.find.as_ref().is_some_and(|f| f.focus != Focus::Page)
    }

    /// Opens the find bar (with `replace`, its second row), seeded with the
    /// selection if it is a few words on one line, and gives it the keys.
    pub fn open_find(&mut self, replace: bool) -> bool {
        self.search = None;
        self.trash = None;
        let selected = self.editor.selected_text();
        let seed = (!selected.is_empty() && !selected.contains('\n') && selected.len() < 200).then(|| selected.to_string());
        let find = self.find.get_or_insert_with(|| Find {
            query: Editor::new(""),
            replace: None,
            focus: Focus::Query,
            matches: Vec::new(),
            ticks: Vec::new(),
            current: None,
            found_for: (u64::MAX, String::new()),
        });
        if let Some(seed) = seed {
            find.query = Editor::new(seed);
        }
        find.query.select_all();
        if replace && find.replace.is_none() {
            find.replace = Some(Editor::new(""));
        }
        find.focus = if replace && !find.query.text().is_empty() { Focus::Replace } else { Focus::Query };
        self.refresh_find(false);
        true
    }

    pub fn close_find(&mut self) -> bool {
        self.find.take().is_some()
    }

    /// Finds again if the text or the query changed since. With `jump`, the
    /// match nearest the caret is selected (as the query is typed).
    pub(super) fn refresh_find(&mut self, jump: bool) {
        let Some(f) = &mut self.find else { return };
        let key = (self.editor.version(), f.query.text().to_string());
        if f.found_for == key && !jump {
            return;
        }
        f.matches = find::find_all(self.editor.text(), f.query.text());
        f.found_for = key;
        let at = self.editor.selection().start();
        f.current = if jump { find::nearest(&f.matches, at) } else { f.current.filter(|&i| i < f.matches.len()) };
        let matches = f.matches.clone();
        let ticks = {
            let sums = self.estimated();
            let lines = self.editor.outline();
            let total = sums.last().copied().unwrap_or(0.0).max(1.0);
            matches
                .iter()
                .map(|&(a, _)| {
                    let i = lines.partition_point(|l| l.start <= a).max(1) - 1;
                    sums.get(i).copied().unwrap_or(0.0) / total
                })
                .collect()
        };
        if let Some(f) = &mut self.find {
            f.ticks = ticks;
        }
        if jump {
            self.select_match();
        }
    }

    /// Selects the match that is on, bringing it into view.
    fn select_match(&mut self) {
        let Some(f) = &self.find else { return };
        if let Some(&(a, b)) = f.current.and_then(|i| f.matches.get(i)) {
            self.editor.set_selection(Selection { anchor: a, head: b });
            self.reveal = true;
        }
    }

    /// Cmd+G: the next match (Shift: the one before); the find bar opens
    /// first if it is closed, with the query it had.
    pub fn find_again(&mut self, back: bool) -> bool {
        if self.find.is_none() {
            return self.open_find(false);
        }
        self.find_step(back)
    }

    /// The next match (or, `back`, the one before), around the note.
    pub(super) fn find_step(&mut self, back: bool) -> bool {
        self.refresh_find(false);
        let Some(f) = &mut self.find else { return false };
        let n = f.matches.len();
        if n == 0 {
            return true;
        }
        let at = self.editor.selection();
        f.current = Some(match f.current {
            // Still on the selected match: step from it.
            Some(i) if f.matches[i] == (at.start(), at.end()) => {
                if back { (i + n - 1) % n } else { (i + 1) % n }
            }
            // The caret moved: from where it is now.
            _ => {
                let i = find::nearest(&f.matches, at.start()).unwrap_or(0);
                if back { (i + n - 1) % n } else { i }
            }
        });
        self.select_match();
        true
    }

    /// Replaces the match that is on (if it is the selection), then goes on
    /// to the next.
    pub(super) fn replace_one(&mut self) -> bool {
        self.refresh_find(false);
        let Some(f) = &self.find else { return false };
        let with = f.replace.as_ref().map_or(String::new(), |r| r.text().to_string());
        let s = self.editor.selection();
        let on = f.current.and_then(|i| f.matches.get(i)).is_some_and(|&m| m == (s.start(), s.end()));
        if on {
            self.edit(|e| e.insert(&with));
            self.refresh_find(false);
        }
        self.find_step(false)
    }

    /// Replaces every match, as one step undo takes back.
    pub(super) fn replace_all(&mut self) -> bool {
        self.refresh_find(false);
        let Some(f) = &self.find else { return false };
        let with = f.replace.as_ref().map_or(String::new(), |r| r.text().to_string());
        let changes: Vec<(usize, usize, String)> = f.matches.iter().map(|&(a, b)| (a, b, with.clone())).collect();
        if changes.is_empty() {
            return true;
        }
        self.edit(|e| e.replace_ranges(&changes));
        self.refresh_find(false);
        true
    }

    /// Runs an editing command on the find field that has the keys.
    pub(super) fn find_field(&mut self, f: impl FnOnce(&mut Editor)) -> bool {
        let Some(find) = &mut self.find else { return false };
        let query = find.focus == Focus::Query;
        let before = find.query.version();
        match find.field() {
            Some(field) => f(field),
            None => return false,
        }
        if query && find.query.version() != before {
            self.refresh_find(true);
        }
        true
    }

    /// Return in a find field: the next match (Shift: the one before), or replace.
    pub(super) fn find_return(&mut self, back: bool) -> bool {
        if self.find.as_ref().is_some_and(|f| f.focus == Focus::Replace) { self.replace_one() } else { self.find_step(back) }
    }

    /// Tab between the query and the replacement.
    pub(super) fn find_tab(&mut self) -> bool {
        if let Some(f) = &mut self.find {
            if f.replace.is_some() {
                f.focus = if f.focus == Focus::Replace { Focus::Query } else { Focus::Replace };
            }
        }
        true
    }

    /// A click on the find bar.
    pub(super) fn find_click(&mut self, target: Target) -> bool {
        let Some(f) = &mut self.find else { return false };
        match target {
            Target::FindField => f.focus = Focus::Query,
            Target::ReplaceField => f.focus = Focus::Replace,
            Target::FindPrev => return self.find_step(true),
            Target::FindNext => return self.find_step(false),
            Target::FindReplace => {
                if f.replace.take().is_none() {
                    f.replace = Some(Editor::new(""));
                    f.focus = Focus::Replace;
                } else if f.focus == Focus::Replace {
                    f.focus = Focus::Query;
                }
            }
            Target::FindClose => return self.close_find(),
            Target::ReplaceOne => return self.replace_one(),
            Target::ReplaceAll => return self.replace_all(),
            _ => return false,
        }
        true
    }
}
