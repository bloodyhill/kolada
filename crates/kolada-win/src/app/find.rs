//! Finding (and replacing) in the note: Ctrl+F, Ctrl+H, F3. Matches are
//! tinted on the page and marked beside the scroll bar; the one that is on
//! is the selection, so the page scrolls to it.

use kolada_editor::find;

use super::*;
use crate::scene::FindView;

/// Which field of the find bar has the keys.
#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    /// The page does: the bar stays, its matches too.
    Page,
    Query,
    Replace,
}

pub struct Find {
    query: Editor,
    replace: Option<Editor>,
    pub focus: Focus,
    pub matches: Vec<(usize, usize)>,
    pub current: Option<usize>,
    /// Where each match is along the note, 0..1.
    pub ticks: Vec<f32>,
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

    pub(super) fn field(&mut self) -> Option<&mut Editor> {
        match self.focus {
            Focus::Query => Some(&mut self.query),
            Focus::Replace => self.replace.as_mut(),
            Focus::Page => None,
        }
    }
}

impl App {
    /// Opens the find bar (with `replace`, its second row), seeded with the
    /// selection if it is a few words on one line, and gives it the keys.
    pub(super) fn open_find(&mut self, replace: bool) {
        self.close_popup();
        self.close_menu();
        let selected = self.editor.selected_text();
        let seed = (!selected.is_empty() && !selected.contains('\n') && selected.len() < 200).then(|| selected.to_string());
        let find = self.find.get_or_insert_with(|| Find {
            query: Editor::field(""),
            replace: None,
            focus: Focus::Query,
            matches: Vec::new(),
            current: None,
            ticks: Vec::new(),
            found_for: (u64::MAX, String::new()),
        });
        if let Some(seed) = seed {
            find.query = Editor::field(seed);
        }
        find.query.select_all();
        if replace && find.replace.is_none() {
            find.replace = Some(Editor::field(""));
        }
        find.focus = if replace && !find.query.text().is_empty() { Focus::Replace } else { Focus::Query };
        self.refresh_find();
        self.request_frame();
    }

    pub(super) fn close_find(&mut self) {
        if self.find.take().is_some() {
            self.request_frame();
        }
    }

    /// Finds again if the text or the query changed since. With `jump`,
    /// the match nearest the caret is selected (as the query is typed).
    pub(super) fn refresh_find_jump(&mut self, jump: bool) {
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

    pub(super) fn refresh_find(&mut self) {
        self.refresh_find_jump(false);
    }

    /// Selects the match that is on, bringing it into view.
    fn select_match(&mut self) {
        let Some(f) = &self.find else { return };
        if let Some(&(a, b)) = f.current.and_then(|i| f.matches.get(i)) {
            self.editor.set_selection(Selection { anchor: a, head: b });
            self.caret_moved(false);
        }
    }

    /// The next match (or, `back`, the one before), around the note.
    pub(super) fn find_step(&mut self, back: bool) {
        self.refresh_find();
        let Some(f) = &mut self.find else { return };
        let n = f.matches.len();
        if n == 0 {
            return;
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
    }

    /// Replaces the match that is on (if it is the selection), then goes on
    /// to the next.
    pub(super) fn replace_one(&mut self) {
        self.refresh_find();
        let Some(f) = &self.find else { return };
        let with = f.replace.as_ref().map_or(String::new(), |r| r.text().to_string());
        let s = self.editor.selection();
        let on = f.current.and_then(|i| f.matches.get(i)).is_some_and(|&m| m == (s.start(), s.end()));
        if on {
            self.edit(|e| e.insert(&with));
            self.refresh_find();
        }
        self.find_step(false);
    }

    /// Replaces every match, as one step Ctrl+Z takes back.
    pub(super) fn replace_all(&mut self) {
        self.refresh_find();
        let Some(f) = &self.find else { return };
        let with = f.replace.as_ref().map_or(String::new(), |r| r.text().to_string());
        let changes: Vec<(usize, usize, String)> = f.matches.iter().map(|&(a, b)| (a, b, with.clone())).collect();
        if changes.is_empty() {
            return;
        }
        let n = changes.len();
        self.edit(|e| e.replace_ranges(&changes));
        self.refresh_find();
        self.whisper(if n == 1 { "Replaced 1".to_string() } else { format!("Replaced {n}") });
    }

    /// Keys while a find bar field has them; false for keys it leaves to
    /// the page.
    pub(super) fn find_key(&mut self, vk: u16, ctrl: bool, shift: bool) -> bool {
        let k = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| key.0 == vk;
        let Some(f) = &mut self.find else { return false };
        if f.focus == Focus::Page {
            return false;
        }
        let replacing = f.focus == Focus::Replace;
        if k(VK_ESCAPE) {
            self.close_find();
        } else if k(VK_RETURN) {
            if replacing {
                if ctrl { self.replace_all() } else { self.replace_one() }
            } else {
                self.find_step(shift);
            }
        } else if k(VK_UP) || k(VK_DOWN) {
            self.find_step(k(VK_UP));
        } else if k(VK_TAB) {
            if f.replace.is_some() {
                f.focus = if replacing { Focus::Query } else { Focus::Replace };
            }
        } else if ctrl && vk == b'H' as u16 {
            self.open_find(true);
        } else if ctrl && !shift && vk == b'F' as u16 {
            self.open_find(false);
        } else {
            let before = f.query.version();
            let field = f.field().expect("a field has the keys");
            let motion = |m: Motion, e: &mut Editor| {
                let plan = e.plan(m, shift, &OneLine(e.text()));
                e.apply(plan);
            };
            if k(VK_LEFT) {
                motion(if ctrl { Motion::WordLeft } else { Motion::Left }, field);
            } else if k(VK_RIGHT) {
                motion(if ctrl { Motion::WordRight } else { Motion::Right }, field);
            } else if k(VK_HOME) {
                motion(Motion::DocStart, field);
            } else if k(VK_END) {
                motion(Motion::DocEnd, field);
            } else if k(VK_BACK) {
                field.backspace(ctrl);
            } else if k(VK_DELETE) {
                field.delete(ctrl);
            } else if ctrl && vk == b'A' as u16 {
                field.select_all();
            } else if ctrl && vk == b'V' as u16 {
                if let Some(text) = clipboard::get(self.hwnd) {
                    let line = text.lines().next().unwrap_or("").to_string();
                    if let Some(field) = self.find.as_mut().and_then(Find::field) {
                        field.insert(&line);
                    }
                }
            } else {
                // Other shortcuts are the page's; typing comes as characters.
                return !ctrl;
            }
            let changed = self.find.as_ref().is_some_and(|f| f.query.version() != before);
            if changed {
                self.refresh_find_jump(true);
            }
        }
        self.request_frame();
        true
    }

    /// A character typed into the find bar; false if it has no field with
    /// the keys.
    pub(super) fn find_typed(&mut self, s: &str) -> bool {
        let Some(f) = &mut self.find else { return false };
        let query = f.focus == Focus::Query;
        let Some(field) = f.field() else { return false };
        field.insert(s);
        if query {
            self.refresh_find_jump(true);
        }
        self.request_frame();
        true
    }

    /// A click on the find bar.
    pub(super) fn find_click(&mut self, target: Target) {
        let Some(f) = &mut self.find else { return };
        match target {
            Target::FindField => f.focus = Focus::Query,
            Target::ReplaceField => f.focus = Focus::Replace,
            Target::FindPrev => self.find_step(true),
            Target::FindNext => self.find_step(false),
            Target::FindReplace => {
                if f.replace.take().is_none() {
                    f.replace = Some(Editor::field(""));
                    f.focus = Focus::Replace;
                } else if f.focus == Focus::Replace {
                    f.focus = Focus::Query;
                }
            }
            Target::FindClose => self.close_find(),
            Target::ReplaceOne => self.replace_one(),
            Target::ReplaceAll => self.replace_all(),
            _ => {}
        }
        self.request_frame();
    }
}
