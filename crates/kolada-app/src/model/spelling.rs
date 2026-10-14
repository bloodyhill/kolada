//! Fixes for a misspelled word, offered where the right-click is: the
//! dictionary's guesses, then Ignore Spelling and Learn Spelling. The
//! checker is passed in, so tests run without the system's (which waits on
//! a spelling service a bare SSH session has none of).
//!
//! Merge point with the right-click menu (`model/menu.rs` when it lands):
//! ask `spell_menu_items(x, y)` for the items to put at the top of that
//! menu, and send the index of the one chosen (counted among these items
//! alone) to `spell_pick`. Nothing else here depends on the menu.

use std::collections::HashSet;

use kolada_editor::Selection;
use kolada_ui::scene::MenuItem;

use super::Model;

/// What choosing an item does.
#[derive(Clone, Debug, PartialEq)]
pub enum SpellAction {
    /// The word is replaced by this one.
    Replace(String),
    /// The word is not marked again while Kolada runs.
    Ignore,
    /// The word is taught to the system's dictionary.
    Learn,
}

/// A misspelled word under the pointer, and what can be done about it.
#[derive(Clone, Debug, PartialEq)]
pub struct SpellFix {
    pub word: String,
    /// Its bytes in the note.
    pub range: (usize, usize),
    /// One entry per menu item: the action it takes.
    pub actions: Vec<SpellAction>,
}

/// What the spelling fixes remember.
#[derive(Default)]
pub struct Fixes {
    /// Words (lower case) the user said to leave alone.
    ignored: HashSet<String>,
    /// The fix the menu now open offers.
    pending: Option<SpellFix>,
}

impl Model {
    /// Whether the word in `units` was ignored.
    pub(super) fn is_ignored(&self, units: &[u16]) -> bool {
        !self.fixes.ignored.is_empty() && self.fixes.ignored.contains(&String::from_utf16_lossy(units).to_lowercase())
    }

    /// The misspelled word at point (`x`, `y`), if any, with the actions for
    /// `guesses` (the dictionary's offers for it).
    fn misspelled_at(&self, x: f32, y: f32, guesses: impl Fn(&str) -> Vec<String>) -> Option<SpellFix> {
        if self.session.spell != 0 || self.search.is_some() || self.trash.is_some() || self.settings.read_only || kolada_ui::scene::hit(&self.hits, x, y) != Some(kolada_ui::scene::Target::Note) {
            return None;
        }
        let at = self.offset_at(x, y);
        let text = self.editor.text();
        let line = self.editor.outline().iter().find(|l| l.start <= at && at <= l.end && l.end <= text.len())?;
        let marks = self.spelling.get(&kolada_ui::spell::line_key(&text[line.start..line.end]))?;
        let &(a, b) = marks.iter().find(|&&(a, b)| line.start + a as usize <= at && at <= line.start + b as usize)?;
        let range = (line.start + a as usize, line.start + b as usize);
        let word = text[range.0..range.1].to_string();
        let mut actions: Vec<SpellAction> = guesses(&word).into_iter().take(5).map(SpellAction::Replace).collect();
        actions.push(SpellAction::Ignore);
        actions.push(SpellAction::Learn);
        Some(SpellFix { word, range, actions })
    }

    /// The right-click menu's spelling items for the word at the point, and
    /// the word is selected (as in any Mac text field): None when it is not
    /// a misspelled word. `guesses` is the dictionary.
    pub fn spell_menu_items_with(&mut self, x: f32, y: f32, guesses: impl Fn(&str) -> Vec<String>) -> Option<Vec<MenuItem>> {
        let fix = self.misspelled_at(x, y, guesses)?;
        self.editor.set_selection(Selection { anchor: fix.range.0, head: fix.range.1 });
        let items: Vec<MenuItem> = fix
            .actions
            .iter()
            .map(|a| match a {
                SpellAction::Replace(w) => MenuItem { label: w.clone(), ..Default::default() },
                // A divider above the first of the two, when guesses come before.
                SpellAction::Ignore => MenuItem { label: "Ignore Spelling".into(), divider: fix.actions.len() > 2, ..Default::default() },
                SpellAction::Learn => MenuItem { label: "Learn Spelling".into(), ..Default::default() },
            })
            .collect();
        self.fixes.pending = Some(fix);
        Some(items)
    }

    /// `spell_menu_items_with` asking the system's dictionary.
    pub fn spell_menu_items(&mut self, x: f32, y: f32) -> Option<Vec<MenuItem>> {
        let spell = self.services.spell.clone();
        self.spell_menu_items_with(x, y, move |word| spell.suggest(word))
    }

    /// The item `index` of the menu `spell_menu_items` gave was chosen.
    pub fn spell_pick(&mut self, index: usize) -> bool {
        let spell = self.services.spell.clone();
        self.spell_pick_with(index, move |word| spell.learn(word))
    }

    /// `spell_pick` teaching words with `learn`.
    pub fn spell_pick_with(&mut self, index: usize, learn: impl FnOnce(&str)) -> bool {
        let Some(fix) = self.fixes.pending.take() else { return false };
        let Some(action) = fix.actions.get(index).cloned() else { return false };
        // The word may have moved or changed since the menu opened.
        if self.editor.text().get(fix.range.0..fix.range.1) != Some(fix.word.as_str()) {
            return false;
        }
        match action {
            SpellAction::Replace(w) => {
                self.editor.set_selection(Selection { anchor: fix.range.0, head: fix.range.1 });
                self.edit(|e| e.insert(&w))
            }
            SpellAction::Ignore => {
                self.fixes.ignored.insert(fix.word.to_lowercase());
                self.spelling.clear();
                true
            }
            SpellAction::Learn => {
                learn(&fix.word);
                self.spelling.clear();
                true
            }
        }
    }
}

#[cfg(test)]
mod tests;
