//! Settings › Font › Other, the same on every OS: a field to type a font's
//! name in, the machine's families that match (each drawn in its own face,
//! `chrome::font_finder`), Up and Down to pick, Enter or a click to choose,
//! Esc to put it away. The machine's fonts are read on a worker when it
//! first opens (`kolada_text::installed`); the shell hands them over as a
//! `Catalogue` and loads the faces of the names shown.

use std::path::PathBuf;

use kolada_text::installed::{self, Family};

use crate::chrome::FINDER_ROWS;
use crate::scene::FontFinder;

/// The machine's families, read once a run: their names (kept for the run,
/// so a frame can name a face by them) and their files.
pub struct Catalogue {
    pub names: Vec<&'static str>,
    pub files: Vec<Vec<PathBuf>>,
}

impl Catalogue {
    pub fn new(families: Vec<Family>) -> Catalogue {
        let (mut names, mut files) = (Vec::with_capacity(families.len()), Vec::with_capacity(families.len()));
        for f in families {
            names.push(&*Box::leak(f.name.into_boxed_str()));
            files.push(f.files);
        }
        Catalogue { names, files }
    }
}

/// A key the finder takes while it is open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Escape,
    Back,
    Delete,
    Left,
    Right,
    Home,
    End,
}

/// What a key or a click came to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// Nothing changed.
    Same,
    /// What shows changed.
    Changed,
    /// The family at this place in the catalogue was chosen.
    Chose(usize),
    /// Put away without choosing.
    Close,
}

/// The finder while it is open.
#[derive(Default)]
pub struct Finder {
    pub query: String,
    /// Where the caret is in `query`, in bytes.
    pub caret: usize,
    /// The row Up and Down picked.
    pub picked: Option<usize>,
    /// The catalogue's places of every family that matches, best first, and
    /// the names of the first few (what shows).
    found: Vec<usize>,
    shown: Vec<&'static str>,
    /// The catalogue was there for the last look.
    read: bool,
}

impl Finder {
    pub fn new() -> Finder {
        Finder::default()
    }

    /// Looks through the catalogue (None while it is being read) for what
    /// is typed.
    pub fn refresh(&mut self, catalogue: Option<&Catalogue>) {
        self.read = catalogue.is_some();
        self.found = catalogue.map_or(Vec::new(), |c| installed::matching(&c.names, &self.query));
        self.shown = catalogue.map_or(Vec::new(), |c| self.found.iter().take(FINDER_ROWS).map(|&i| c.names[i]).collect());
        self.picked = self.picked.filter(|&p| p < self.shown.len());
    }

    /// The catalogue's places of the families shown: the faces the shell
    /// loads to draw them.
    pub fn shown(&self) -> &[usize] {
        &self.found[..self.found.len().min(FINDER_ROWS)]
    }

    pub fn view(&self) -> FontFinder<'_> {
        FontFinder { query: &self.query, caret: self.caret, names: &self.shown, picked: self.picked, total: if self.read { self.found.len().max(1) } else { 0 } }
    }

    /// Text typed into the field.
    pub fn typed(&mut self, s: &str, catalogue: Option<&Catalogue>) -> Outcome {
        if s.chars().any(char::is_control) {
            return Outcome::Same;
        }
        self.query.insert_str(self.caret, s);
        self.caret += s.len();
        self.picked = None;
        self.refresh(catalogue);
        Outcome::Changed
    }

    pub fn key(&mut self, key: Key, catalogue: Option<&Catalogue>) -> Outcome {
        let prev = |q: &str, at: usize| q[..at].char_indices().next_back().map_or(0, |(i, _)| i);
        let next = |q: &str, at: usize| q[at..].chars().next().map_or(at, |c| at + c.len_utf8());
        match key {
            Key::Escape => return Outcome::Close,
            Key::Enter => {
                let row = self.picked.unwrap_or(0);
                return match self.found.get(row).filter(|_| row < FINDER_ROWS) {
                    Some(&i) => Outcome::Chose(i),
                    None => Outcome::Same,
                };
            }
            Key::Down if !self.shown.is_empty() => self.picked = Some(self.picked.map_or(0, |p| (p + 1).min(self.shown.len() - 1))),
            Key::Up if !self.shown.is_empty() => self.picked = Some(self.picked.map_or(0, |p| p.saturating_sub(1))),
            Key::Back if self.caret > 0 => {
                let at = prev(&self.query, self.caret);
                self.query.replace_range(at..self.caret, "");
                self.caret = at;
                self.picked = None;
                self.refresh(catalogue);
            }
            Key::Delete if self.caret < self.query.len() => {
                let to = next(&self.query, self.caret);
                self.query.replace_range(self.caret..to, "");
                self.picked = None;
                self.refresh(catalogue);
            }
            Key::Left => self.caret = prev(&self.query, self.caret),
            Key::Right => self.caret = next(&self.query, self.caret),
            Key::Home => self.caret = 0,
            Key::End => self.caret = self.query.len(),
            _ => return Outcome::Same,
        }
        Outcome::Changed
    }

    /// A click on the row shown at `row`.
    pub fn clicked(&self, row: usize) -> Outcome {
        self.found.get(row).filter(|_| row < FINDER_ROWS).map_or(Outcome::Same, |&i| Outcome::Chose(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue() -> Catalogue {
        let fam = |n: &str| Family { name: n.into(), files: vec![PathBuf::from(format!("{n}.ttf"))] };
        Catalogue::new(["Bahnschrift", "Calibri", "Cambria", "Candara", "Cascadia Code", "Constantia", "Corbel", "Courier New", "Palatino Linotype"].map(fam).to_vec())
    }

    #[test]
    fn typing_narrows_the_fonts_and_enter_chooses_the_one_picked() {
        let c = catalogue();
        let mut f = Finder::new();
        f.refresh(None);
        assert_eq!(f.view().total, 0, "still reading the machine's fonts");
        f.refresh(Some(&c));
        assert_eq!(f.view().names.len(), FINDER_ROWS, "a few at once");
        assert_eq!(f.typed("ca", Some(&c)), Outcome::Changed);
        assert_eq!(f.view().names, ["Calibri", "Cambria", "Candara", "Cascadia Code"]);
        assert_eq!(f.key(Key::Enter, Some(&c)), Outcome::Chose(1), "Enter takes the first");
        f.key(Key::Down, Some(&c));
        f.key(Key::Down, Some(&c));
        assert_eq!(f.view().picked, Some(1));
        assert_eq!(f.key(Key::Enter, Some(&c)), Outcome::Chose(2), "Cambria");
        assert_eq!(f.clicked(3), Outcome::Chose(4), "a click on Cascadia Code");
        f.key(Key::Back, Some(&c));
        assert_eq!((f.query.as_str(), f.caret, f.view().picked), ("c", 1, None), "typing again lets the pick go");
        f.typed("zz", Some(&c));
        assert!(f.view().names.is_empty() && f.view().total == 1, "none match: it says so");
        assert_eq!(f.key(Key::Enter, Some(&c)), Outcome::Same);
        assert_eq!(f.key(Key::Escape, Some(&c)), Outcome::Close);
    }

    #[test]
    fn the_caret_moves_by_whole_letters() {
        let c = catalogue();
        let mut f = Finder::new();
        f.typed("Café", Some(&c));
        assert_eq!(f.caret, 5);
        f.key(Key::Left, Some(&c));
        assert_eq!(f.caret, 3, "back over é, two bytes");
        f.key(Key::Delete, Some(&c));
        assert_eq!(f.query, "Caf");
        f.key(Key::Home, Some(&c));
        f.key(Key::Back, Some(&c));
        assert_eq!((f.query.as_str(), f.caret), ("Caf", 0), "nothing before the start");
        f.key(Key::End, Some(&c));
        assert_eq!(f.caret, 3);
        assert_eq!(f.typed("\n", Some(&c)), Outcome::Same, "no line breaks in a name");
    }
}
