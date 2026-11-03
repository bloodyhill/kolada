//! Settings › Font › Other: the finder of the machine's fonts
//! (`kolada_ui::fontfind`), its catalogue read on a worker the first time it
//! opens (`services::Fonts::installed_dirs`), and a family chosen from it
//! set as the note's font, as Windows' `app/fontpick.rs` does.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use kolada_core::session::{INSTALLED_FONT, Session};
use kolada_text::installed::Family;
use kolada_ui::fontfind::{Catalogue, Finder, Key, Outcome};
use kolada_ui::scene::{self, Setting};

use super::Model;

/// The finder, the machine's fonts once read, and the faces loaded to draw
/// their names (by their place in the catalogue).
#[derive(Default)]
pub(super) struct FontPick {
    pub(super) finder: Option<Finder>,
    catalogue: Option<Catalogue>,
    pub(super) reading: Option<Receiver<Vec<Family>>>,
    loaded: HashSet<usize>,
    /// The machine's family the note is set in, by a name kept for the run.
    pub(super) name: Option<&'static str>,
}

/// The machine's family the session sets notes in, and its files, while
/// its first file is there (gone, the note is in sans).
pub(super) fn installed(s: &Session) -> Option<(String, Vec<PathBuf>)> {
    let files: Vec<PathBuf> = s.font_files.iter().map(PathBuf::from).collect();
    (s.font == INSTALLED_FONT && !s.font_family.is_empty() && files.first().is_some_and(|f| f.is_file())).then(|| (s.font_family.clone(), files))
}

impl Model {
    /// The machine's font chosen, by a name kept for the run (the scene
    /// names a face by it): the catalogue's, or kept once from the session.
    pub(super) fn font_name(&mut self) -> Option<&'static str> {
        if self.session.font != INSTALLED_FONT || self.session.font_family.is_empty() {
            return None;
        }
        if self.font.name.is_none_or(|n| n != self.session.font_family) {
            self.font.name = Some(Box::leak(self.session.font_family.clone().into_boxed_str()));
        }
        self.font.name
    }

    pub(super) fn font_finder_view(&self) -> Option<scene::FontFinder<'_>> {
        self.font.finder.as_ref().filter(|_| self.settings.open).map(|f| f.view())
    }

    /// Other was pressed: the finder opens (its fonts read the first time),
    /// or, open, it is put away.
    pub(super) fn font_other(&mut self) -> bool {
        if self.font.finder.take().is_none() {
            let mut finder = Finder::new();
            finder.refresh(self.font.catalogue.as_ref());
            self.font.finder = Some(finder);
            self.font_faces_shown();
            self.read_fonts();
        }
        true
    }

    pub(super) fn font_finder_close(&mut self) -> bool {
        self.font.finder.take().is_some()
    }

    /// The machine's fonts, read on a worker below the app.
    fn read_fonts(&mut self) {
        if self.font.catalogue.is_some() || self.font.reading.is_some() {
            return;
        }
        let dirs = self.services.fonts.installed_dirs();
        let (to, from) = channel();
        std::thread::Builder::new()
            .name("fonts".into())
            .spawn(move || {
                let _ = to.send(kolada_text::installed::families(&kolada_text::installed::files_in(&dirs)));
            })
            .ok();
        self.font.reading = Some(from);
    }

    /// While the fonts are read, the view looks again this often.
    pub(super) fn font_due(&self) -> Option<Instant> {
        self.font.reading.as_ref().map(|_| Instant::now() + Duration::from_millis(100))
    }

    /// The machine's fonts, if they arrived: whether the screen changes.
    pub(super) fn fonts_arrived(&mut self) -> bool {
        let Some(families) = self.font.reading.as_ref().and_then(|r| r.try_recv().ok()) else { return false };
        self.font.reading = None;
        self.font.catalogue = Some(Catalogue::new(families));
        if let Some(f) = &mut self.font.finder {
            f.refresh(self.font.catalogue.as_ref());
        }
        self.font_faces_shown();
        true
    }

    /// The faces of the names the finder shows, loaded so each is drawn in
    /// its own (one file each: the regular).
    fn font_faces_shown(&mut self) {
        let (Some(f), Some(c)) = (&self.font.finder, &self.font.catalogue) else { return };
        let mut eng = self.eng.borrow_mut();
        for &i in f.shown() {
            if self.font.loaded.insert(i) {
                eng.load_files(&c.files[i][..c.files[i].len().min(1)]);
            }
        }
    }

    /// A family of the catalogue chosen: the note is set in it, and it is
    /// kept by its files.
    fn choose_installed(&mut self, i: usize) -> bool {
        let Some(c) = &self.font.catalogue else { return false };
        let (name, files) = (c.names[i], c.files[i].clone());
        self.font.loaded.insert(i);
        self.font.name = Some(name);
        self.font.finder = None;
        self.session.font_family = name.to_string();
        self.session.font_files = files.iter().map(|f| f.to_string_lossy().into_owned()).collect();
        self.choose(Setting::Font, INSTALLED_FONT)
    }

    fn font_outcome(&mut self, outcome: Outcome) -> bool {
        match outcome {
            Outcome::Same => false,
            Outcome::Changed => {
                self.font_faces_shown();
                true
            }
            Outcome::Chose(i) => self.choose_installed(i),
            Outcome::Close => self.font_finder_close(),
        }
    }

    /// A family shown in the finder, clicked.
    pub(super) fn font_clicked(&mut self, row: usize) -> bool {
        let outcome = self.font.finder.as_ref().map_or(Outcome::Same, |f| f.clicked(row));
        self.font_outcome(outcome)
    }

    /// Text typed while the finder is open: None if it is not.
    pub(super) fn font_typed(&mut self, s: &str) -> Option<bool> {
        let f = self.font.finder.as_mut().filter(|_| self.settings.open)?;
        let outcome = f.typed(&s.replace('\n', ""), self.font.catalogue.as_ref());
        Some(self.font_outcome(outcome))
    }

    /// A key while the finder is open: None if it is not.
    pub(super) fn font_key(&mut self, key: Key) -> Option<bool> {
        let f = self.font.finder.as_mut().filter(|_| self.settings.open)?;
        let outcome = f.key(key, self.font.catalogue.as_ref());
        Some(self.font_outcome(outcome))
    }
}
