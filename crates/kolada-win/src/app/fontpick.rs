//! Settings › Font › Other on Windows: the finder of the machine's fonts
//! (`kolada_ui::fontfind`), its catalogue read on a worker the first time it
//! opens, and a family chosen from it set as the note's font.

use std::collections::HashSet;

use kolada_text::installed::Family;
use kolada_ui::fontfind::{Catalogue, Finder, Key, Outcome};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_UP};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use super::*;

/// The finder, the machine's fonts once read, and the faces loaded to draw
/// their names (by their place in the catalogue).
#[derive(Default)]
pub struct FontPick {
    pub finder: Option<Finder>,
    catalogue: Option<Catalogue>,
    reading: bool,
    loaded: HashSet<usize>,
    /// The machine's family the note is set in, by a name kept for the run.
    pub name: Option<&'static str>,
}

impl FontPick {
    pub fn new(session: &Session) -> FontPick {
        let name = (session.font == kolada_core::session::INSTALLED_FONT && !session.font_family.is_empty()).then(|| &*Box::leak(session.font_family.clone().into_boxed_str()));
        FontPick { name, ..FontPick::default() }
    }
}

impl App {
    /// Other was pressed: the finder opens (its fonts read the first time),
    /// or, open, it is put away.
    pub(super) fn font_other(&mut self) {
        if self.font.finder.take().is_none() || !self.ui.options_open {
            let mut finder = Finder::new();
            finder.refresh(self.font.catalogue.as_ref());
            self.font.finder = Some(finder);
            self.font_faces_shown();
            self.read_fonts();
        }
        self.request_frame();
    }

    pub(super) fn close_font_finder(&mut self) {
        if self.font.finder.take().is_some() {
            self.request_frame();
        }
    }

    /// The machine's fonts, read on a worker below the app: posted back as
    /// `WM_APP_FONTS`.
    fn read_fonts(&mut self) {
        if self.font.catalogue.is_some() || self.font.reading {
            return;
        }
        self.font.reading = true;
        let hwnd = crate::Handoff(self.hwnd);
        std::thread::spawn(move || {
            let hwnd = hwnd;
            crate::background_priority();
            let families = kolada_text::installed::families(&kolada_text::installed::files_in(&crate::fonts::installed_dirs()));
            let found = Box::into_raw(Box::new(families));
            if unsafe { PostMessageW(Some(hwnd.0), crate::window::WM_APP_FONTS, WPARAM(0), LPARAM(found as isize)) }.is_err() {
                drop(unsafe { Box::from_raw(found) });
            }
        });
    }

    /// The machine's fonts arrived.
    pub(super) fn fonts_read(&mut self, families: Vec<Family>) {
        self.font.reading = false;
        self.font.catalogue = Some(Catalogue::new(families));
        if let Some(f) = &mut self.font.finder {
            f.refresh(self.font.catalogue.as_ref());
        }
        self.font_faces_shown();
        self.request_frame();
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
    fn choose_installed(&mut self, i: usize) {
        let Some(c) = &self.font.catalogue else { return };
        let (name, files) = (c.names[i], c.files[i].clone());
        self.eng.borrow_mut().load_files(&files);
        self.font.loaded.insert(i);
        self.font.name = Some(name);
        self.font.finder = None;
        self.session.font_family = name.to_string();
        self.session.font_files = files.iter().map(|f| f.to_string_lossy().into_owned()).collect();
        self.choose(Setting::Font, kolada_core::session::INSTALLED_FONT);
    }

    fn font_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Same => {}
            Outcome::Changed => {
                self.font_faces_shown();
                self.request_frame();
            }
            Outcome::Chose(i) => self.choose_installed(i),
            Outcome::Close => self.close_font_finder(),
        }
    }

    /// A family shown in the finder, clicked.
    pub(super) fn font_clicked(&mut self, row: usize) {
        if let Some(outcome) = self.font.finder.as_ref().map(|f| f.clicked(row)) {
            self.font_outcome(outcome);
        }
    }

    /// Typed while the finder is open: true if it took it.
    pub(super) fn font_char(&mut self, s: &str) -> bool {
        if !self.ui.options_open {
            return false;
        }
        let Some(f) = &mut self.font.finder else { return false };
        let outcome = f.typed(s, self.font.catalogue.as_ref());
        self.font_outcome(outcome);
        true
    }

    /// A key while the finder is open: true if it was its.
    pub(super) fn font_key(&mut self, vk: u16) -> bool {
        if !self.ui.options_open || self.font.finder.is_none() {
            return false;
        }
        let key = match vk {
            v if v == VK_UP.0 => Key::Up,
            v if v == VK_DOWN.0 => Key::Down,
            v if v == VK_RETURN.0 => Key::Enter,
            v if v == VK_ESCAPE.0 => Key::Escape,
            v if v == VK_BACK.0 => Key::Back,
            v if v == VK_DELETE.0 => Key::Delete,
            v if v == VK_LEFT.0 => Key::Left,
            v if v == VK_RIGHT.0 => Key::Right,
            v if v == VK_HOME.0 => Key::Home,
            v if v == VK_END.0 => Key::End,
            _ => return false,
        };
        let outcome = self.font.finder.as_mut().map_or(Outcome::Same, |f| f.key(key, self.font.catalogue.as_ref()));
        self.font_outcome(outcome);
        true
    }
}
