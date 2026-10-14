//! Linking as you write, as on Windows (`kolada-win` `app/suggest.rs`): after
//! `[[`, the notes whose names fit what follows show under the caret. Up and
//! Down pick, Return or Tab links it, Escape lets the brackets be. With no
//! note by that name, the last row makes one when the link is followed.

use kolada_core::search;
use kolada_ui::links::link_at_caret;

use super::*;

/// Names shown at most.
const SHOWN: usize = 6;

pub(super) struct Suggest {
    /// Where the name being typed starts (just after `[[`).
    start: usize,
    pub names: Vec<(String, String)>,
    pub selected: usize,
    /// The name as typed, when no note has it.
    pub create: Option<String>,
}

#[derive(Default)]
pub(super) struct Suggesting {
    pub now: Option<Suggest>,
    /// Put away with Escape for the link starting here: it stays away
    /// until the caret leaves it.
    dismissed: Option<usize>,
}

impl Model {
    /// Finds suggestions for the link at the caret, or puts them away.
    pub(super) fn update_suggestions(&mut self) {
        let head = self.editor.selection().head;
        let found = if self.settings.read_only || !self.editor.selection().is_caret() || self.popup.is_some() || self.scrubbing() {
            None
        } else {
            link_at_caret(self.editor.text(), head).map(|(start, typed)| (start, typed.to_string()))
        };
        let Some((start, typed)) = found else {
            self.suggest.dismissed = None;
            self.suggest.now = None;
            return;
        };
        if self.suggest.dismissed == Some(start) {
            return;
        }
        self.start_index();
        let names: Vec<(String, String)> = match self.index.lock().unwrap().as_ref() {
            Some(all) => search::complete(all, &typed, SHOWN)
                .into_iter()
                .map(|n| (n.name.clone(), n.path.rsplit_once('/').map_or(String::new(), |(f, _)| f.to_string())))
                .collect(),
            None => Vec::new(),
        };
        let exact = names.iter().any(|(n, _)| n.to_lowercase() == typed.trim().to_lowercase());
        let create = (!typed.trim().is_empty() && !exact).then(|| typed.trim().to_string());
        let selected = self.suggest.now.as_ref().filter(|s| s.start == start).map_or(0, |s| s.selected);
        let rows = names.len() + create.is_some() as usize;
        self.suggest.now = (rows > 0).then(|| Suggest { start, names, selected: selected.min(rows.saturating_sub(1)), create });
    }

    /// The suggestions as the scene draws them.
    pub(super) fn suggestions_view(&self) -> Option<scene::Suggestions<'_>> {
        self.suggest.now.as_ref().map(|s| scene::Suggestions { names: &s.names, selected: s.selected, create: s.create.as_deref() })
    }

    /// Links the `i`th suggestion: the name typed so far becomes it, and the
    /// link closes.
    pub(super) fn accept_suggestion(&mut self, i: usize) -> bool {
        let Some(s) = self.suggest.now.take() else { return false };
        let name = match s.names.get(i) {
            Some((name, _)) => name.clone(),
            None => match s.create {
                Some(name) => name,
                None => return false,
            },
        };
        let head = self.editor.selection().head;
        // Brackets typed already stay; otherwise they close the link.
        let closed = self.editor.text()[head..].starts_with("]]");
        let insert = if closed { name } else { format!("{name}]]") };
        self.editor.set_selection(Selection { anchor: s.start, head });
        self.edit(|e| {
            e.insert(&insert);
            if closed {
                let at = e.selection().head + 2;
                e.set_selection(Selection::caret(at));
            }
        });
        self.suggest.dismissed = None;
        self.suggest.now = None;
        true
    }

    /// A key while suggestions show: Up, Down, Return, Tab or Escape. None
    /// for keys that are not theirs.
    pub(super) fn suggest_key(&mut self, key: SuggestKey) -> Option<bool> {
        let s = self.suggest.now.as_mut()?;
        let rows = s.names.len() + s.create.is_some() as usize;
        match key {
            SuggestKey::Down => s.selected = (s.selected + 1) % rows,
            SuggestKey::Up => s.selected = (s.selected + rows - 1) % rows,
            SuggestKey::Pick => {
                let i = s.selected;
                return Some(self.accept_suggestion(i));
            }
            SuggestKey::Away => {
                self.suggest.dismissed = Some(s.start);
                self.suggest.now = None;
            }
        }
        Some(true)
    }
}

/// The keys suggestions take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SuggestKey {
    Up,
    Down,
    Pick,
    Away,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(test: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-app-suggest-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Work")).unwrap();
        std::fs::write(dir.join("Page.md"), "# Page\n\n").unwrap();
        std::fs::write(dir.join("Pricing.md"), "# Pricing\n").unwrap();
        std::fs::write(dir.join("Work").join("Price list.md"), "# Price list\n").unwrap();
        std::fs::write(dir.join("ملاحظة عربية.md"), "# ملاحظة عربية\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.open("Page.md");
        // The index is read on a thread: wait for it.
        m.start_index();
        for _ in 0..200 {
            if m.index.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        m.go(Motion::DocEnd, false);
        (m, dir)
    }

    #[test]
    fn typing_a_link_suggests_notes_and_return_links_the_chosen_one() {
        let (mut m, dir) = vault("pick");
        m.insert("See [[Pri");
        let names: Vec<String> = m.suggest.now.as_ref().unwrap().names.iter().map(|n| n.0.clone()).collect();
        assert!(names.contains(&"Pricing".to_string()) && names.contains(&"Price list".to_string()), "{names:?}");
        assert_eq!(m.suggest.now.as_ref().unwrap().create.as_deref(), Some("Pri"), "and a new note named as typed");
        m.frame(2000, 1440, 2.0);
        assert!(m.hits.iter().any(|(_, t)| matches!(t, Target::Suggestion(_))), "drawn under the caret");
        // Down, then Return links it and closes the brackets.
        let second = names[1].clone();
        assert!(m.go(Motion::Down, false));
        assert!(m.newline(false));
        assert!(m.text().ends_with(&format!("See [[{second}]]")), "{:?}", m.text());
        assert!(m.suggest.now.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn escape_lets_the_brackets_be_until_the_link_is_left() {
        let (mut m, dir) = vault("away");
        m.insert("[[Pri");
        assert!(m.escape());
        assert!(m.suggest.now.is_none());
        m.insert("c");
        assert!(m.suggest.now.is_none(), "still away for this link");
        m.insert("]] and [[ملا");
        let s = m.suggest.now.as_ref().expect("a new link suggests again");
        assert_eq!(s.names[0].0, "ملاحظة عربية");
        // Tab links it too.
        assert!(m.tab(false));
        assert!(m.text().ends_with("[[ملاحظة عربية]]"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_click_on_a_suggestion_links_it() {
        let (mut m, dir) = vault("click");
        m.insert("[[Pricing");
        m.frame(2000, 1440, 2.0);
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::Suggestion(0)).unwrap();
        assert!(m.pointer_down(r.left + 4.0, r.top + 4.0, 1, false));
        assert!(m.text().ends_with("[[Pricing]]"), "{:?}", m.text());
        let _ = std::fs::remove_dir_all(dir);
    }
}
