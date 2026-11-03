//! Pictures in the open note: saved where the vault keeps its attachments
//! (Obsidian's setting) and embedded on a line of their own, the way
//! Obsidian names and embeds them; and resized by their right edge. A
//! screenshot on the pasteboard, pictures dropped on the window and pictures
//! that came with a paste all end up here (`paste.rs`, `arrive.rs`).
//! Reference: kolada-win app/pictures.rs and convert.rs (save_pasted).

use std::path::{Path, PathBuf};

use kolada_editor::Selection;
use kolada_ui::images::{self, Lookup};
use kolada_ui::note::{PICTURE_LEAST, fit_picture};
use kolada_ui::pictures::{attachment_dir, attachment_setting};

use super::Model;
use crate::services::{self, Calendar};

/// What a drawing made here is named before its stamp.
const DRAWING: &str = "Drawing";

/// Whether the picture `name` is a drawing made here (`Drawing <stamp>.png`).
pub fn is_drawing(name: &str) -> bool {
    let file = name.rsplit('/').next().unwrap_or(name);
    file.strip_prefix(DRAWING).is_some_and(|r| r.starts_with(' ')) && file.ends_with(".png")
}

/// `dir/name`, or `dir/name 1.ext`, `dir/name 2.ext`... if taken.
pub fn free_name(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = name.rsplit_once('.').map_or((name, ""), |(s, e)| (s, e));
    (0..)
        .map(|n| {
            let file = match (n, ext) {
                (0, _) => name.to_string(),
                (n, "") => format!("{stem} {n}"),
                (n, ext) => format!("{stem} {n}.{ext}"),
            };
            dir.join(file)
        })
        .find(|p| !p.exists())
        .expect("a free name exists")
}

/// A file's name with its extension.
pub fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What `;;name` expands to in a note: the built-in set, and `;;date` as today.
pub fn snippets(calendar: &std::sync::Arc<dyn Calendar>) -> kolada_editor::Snippets {
    let calendar = calendar.clone();
    kolada_editor::Snippets::standard().with_today(move || {
        let t = services::now(&*calendar);
        format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)
    })
}

/// When it is now, as Obsidian stamps pasted pictures: `20261005183012`.
pub fn stamp(calendar: &dyn Calendar) -> String {
    let t = services::now(calendar);
    format!("{:04}{:02}{:02}{:02}{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

/// Whether `bytes` are a picture Kolada shows (PNG, JPEG, GIF, WebP, BMP), by
/// how they start: a page's error answered in a picture's place is not.
pub fn is_picture(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
        || bytes.starts_with(b"BM")
}

/// What a paste brought, saved in `dir` as `Pasted image <stamp>.<ext>`:
/// those inside it, and those Word or Outlook keep on this machine, at once;
/// for each on the web, an empty file holds its name until it is fetched.
/// The Markdown with each picture embedded by its saved name (one that
/// cannot be kept, or with no `dir`, left out), and the pictures to fetch.
pub fn save_pasted(p: kolada_convert::Pasted, dir: Option<&Path>, stamp: &str) -> (String, Vec<(PathBuf, String)>) {
    let mut text = p.markdown;
    let mut fetch = Vec::new();
    let ready = dir.is_some_and(|d| std::fs::create_dir_all(d).is_ok());
    let place = |name: &str, save: &mut dyn FnMut(&Path) -> bool| -> Option<PathBuf> {
        let dir = dir.filter(|_| ready)?;
        let ext = name.rsplit_once('.').map_or("png", |(_, e)| e);
        let to = free_name(dir, &format!("Pasted image {stamp}.{ext}"));
        save(&to).then_some(to)
    };
    /// Where a picture of the paste is.
    enum Source<'a> {
        Inside(&'a [u8]),
        File(&'a str),
        Web(&'a str),
    }
    let mut all: Vec<(&str, Source)> = p.pictures.iter().map(|pic| (pic.name.as_str(), Source::Inside(&pic.data))).collect();
    all.extend(p.fetch.iter().map(|(name, s)| (name.as_str(), s.strip_prefix("file:").map_or(Source::Web(s), Source::File))));
    // Named in the order the note shows them, the first the plain name.
    all.sort_by_key(|(name, _)| text.find(&format!("![[{name}]]")).unwrap_or(usize::MAX));
    let mut kept: Vec<(String, Option<PathBuf>)> = Vec::new();
    for (name, source) in all {
        let saved = match source {
            Source::Inside(data) => place(name, &mut |to| std::fs::write(to, data).is_ok()),
            Source::File(path) => place(name, &mut |to| std::fs::copy(path, to).is_ok()),
            Source::Web(url) => {
                let saved = place(name, &mut |to| std::fs::File::create(to).is_ok());
                if let Some(to) = &saved {
                    fetch.push((to.clone(), url.to_string()));
                }
                saved
            }
        };
        kept.push((name.to_string(), saved));
    }
    for (name, saved) in kept {
        let now = saved.as_deref().map_or_else(String::new, |to| format!("![[{}]]", name_of(to)));
        text = text.replace(&format!("![[{name}]]"), &now);
    }
    // Where a picture was left out, one blank line, not more.
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    (text.trim().to_string(), fetch)
}

/// A picture being resized by its right edge: its line, where the drag began
/// and the width then, its width now, the widest it can be, and its own
/// width (which a drag back to it returns to).
pub struct Resizing {
    pub line: usize,
    from_x: f32,
    start_width: f32,
    pub width: f32,
    most: f32,
    natural: f32,
}

impl Model {
    /// Where pictures for the open note go, as the vault's settings say (the
    /// folder is made when the first picture is saved).
    pub(super) fn attachment_path(&self) -> PathBuf {
        let setting = std::fs::read_to_string(self.vault.join(".obsidian").join("app.json")).ok().and_then(|j| attachment_setting(&j));
        let note_dir = self.path.as_deref().and_then(Path::parent).map_or_else(|| self.vault.clone(), Path::to_path_buf);
        attachment_dir(&self.vault, &note_dir, setting.as_deref().unwrap_or(""))
    }

    /// The attachments folder, made.
    pub(super) fn attachment_dir(&self) -> Option<PathBuf> {
        let dir = self.attachment_path();
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    /// Embeds `names` at `at`, each on a line of its own, and puts the caret
    /// on the line after them. Whether the screen changes.
    pub(super) fn embed_pictures(&mut self, names: &[String], at: usize) -> bool {
        if names.is_empty() || self.in_field() {
            return false;
        }
        let text = self.editor.text();
        let line = self.editor.line_at(at.min(text.len()));
        // On a blank line, there; otherwise after the line's words.
        let blank = text[line.start..line.end].trim().is_empty();
        let at = if blank { line.start } else { line.end };
        let mut block = String::new();
        if !blank {
            block.push('\n');
        }
        for name in names {
            block.push_str(&format!("![[{name}]]\n"));
        }
        self.editor.set_selection(Selection::caret(at));
        let done = self.edit(|e| e.insert(&block));
        self.pictures.borrow_mut().forget_places();
        done
    }

    /// Cmd+V with a picture (and no text) on the pasteboard: `png` is saved
    /// with the vault's attachments as `Pasted image <stamp>.png` and
    /// embedded at the caret. Whether the screen changes.
    pub fn paste_picture(&mut self, png: &[u8]) -> bool {
        self.paste_picture_stamped(png, &stamp(&*self.services.calendar))
    }

    pub(super) fn paste_picture_stamped(&mut self, png: &[u8], stamp: &str) -> bool {
        let Some(path) = self.save_picture(png, &format!("Pasted image {stamp}.png")) else { return false };
        let at = self.editor.selection().head;
        self.embed_pictures(&[name_of(&path)], at)
    }

    /// A drawing made with a touch screen's pad, embedded at the caret as
    /// `Drawing <stamp>.png`: named so, it can be opened and drawn on again.
    pub fn insert_drawing(&mut self, png: &[u8]) -> bool {
        let Some(path) = self.save_picture(png, &format!("{DRAWING} {}.png", stamp(&*self.services.calendar))) else { return false };
        let at = self.editor.selection().head;
        self.embed_pictures(&[name_of(&path)], at)
    }

    /// `png` saved with the vault's attachments as `name` (or a free name
    /// like it), where the note may be edited.
    fn save_picture(&self, png: &[u8], name: &str) -> Option<PathBuf> {
        // Under the welcome, nothing is written (the page takes nothing).
        if self.in_field() || self.scrubbing() || self.settings.read_only || self.welcome_covers() {
            return None;
        }
        let path = free_name(&self.attachment_dir()?, name);
        std::fs::write(&path, png).ok()?;
        Some(path)
    }

    // A picture chosen on a touch screen: the shell draws its handles and
    // its menu (sizes, replace, draw on, delete) over the model's frame.

    /// The picture chosen whole, as a touch screen's shell shows it: where
    /// it is drawn (DIPs), and whether it is a drawing made here.
    pub fn chosen_picture(&self) -> Option<(kolada_ui::scene::Rect, bool)> {
        let (a, b) = self.selected_picture()?;
        let place = self.place_of(kolada_ui::scene::Target::Picture(a))?;
        let name = images::embed_in(&self.editor.text()[a..b])?;
        Some((place, is_drawing(name)))
    }

    /// The chosen picture's right edge held by a shell's handle: pressed
    /// (`phase` 0) at `x` DIPs, moved (1), let go (2), as a pointer drags
    /// it. Whether the screen changes.
    pub fn drag_picture_edge(&mut self, phase: u8, x: f32) -> bool {
        match phase {
            0 => self.selected_picture().is_some_and(|(a, _)| self.begin_resize(a, x)),
            1 => self.resize_to(x),
            _ => self.end_resize(),
        }
    }

    /// The chosen picture `share` of the column wide (a menu's Small,
    /// Medium, Large), or its own size (None), as a drag would leave it.
    pub fn size_chosen_picture(&mut self, share: Option<f32>) -> bool {
        let Some((a, _)) = self.selected_picture() else { return false };
        if !self.begin_resize(a, 0.0) {
            return false;
        }
        if let Some(r) = &mut self.resizing {
            r.width = share.map_or(r.natural, |s| (r.most * s).clamp(PICTURE_LEAST, r.most.max(PICTURE_LEAST)));
            // The line says whether this changes it, not the last frame.
            r.start_width = f32::NAN;
        }
        self.end_resize()
    }

    /// The chosen picture's file, found (a shell opens a drawing from it).
    pub fn chosen_picture_file(&self) -> Option<PathBuf> {
        let (a, b) = self.selected_picture()?;
        let name = images::embed_in(&self.editor.text()[a..b])?;
        let p = self.note_pictures()?;
        let found = p.images.borrow_mut().place(p.vault, &p.dir, name);
        match found {
            Lookup::Found(path, _, _) => Some(path),
            _ => None,
        }
    }

    /// The chosen picture swapped for `png`, saved as a pasted one: its line
    /// keeps its width and caption, and it stays chosen. One undo puts the
    /// old one back.
    pub fn replace_chosen_picture(&mut self, png: &[u8]) -> bool {
        let Some((a, b)) = self.selected_picture() else { return false };
        let Some(path) = self.save_picture(png, &format!("Pasted image {}.png", stamp(&*self.services.calendar))) else { return false };
        let old = self.editor.text()[a..b].to_string();
        let Some(name) = images::embed_in(&old) else { return false };
        let new = old.replacen(name, &name_of(&path), 1);
        self.editor.set_selection(Selection { anchor: a, head: b });
        self.edit(|e| e.insert(&new));
        self.pictures.borrow_mut().forget_places();
        self.select_picture(a)
    }

    /// The chosen drawing drawn on again: `png` written over its file (one
    /// Kolada made, `Drawing <stamp>.png`), and shown anew.
    pub fn redraw_chosen_picture(&mut self, png: &[u8]) -> bool {
        if self.settings.read_only || self.scrubbing() {
            return false;
        }
        let Some(path) = self.chosen_picture_file().filter(|p| is_drawing(&name_of(p))) else { return false };
        if std::fs::write(&path, png).is_err() {
            return false;
        }
        self.pictures.borrow_mut().forget(&path);
        true
    }

    /// The chosen picture and its line gone (a shell's Delete).
    pub fn delete_chosen_picture(&mut self) -> bool {
        self.delete_picture().unwrap_or(false)
    }

    // Resizing: drag a picture's right edge.

    /// Chooses the picture on the line at `at`, whole.
    pub(super) fn select_picture(&mut self, at: usize) -> bool {
        let line = self.editor.line_at(at.min(self.editor.text().len()));
        self.editor.set_selection(Selection { anchor: line.start, head: line.end });
        self.reveal = false;
        true
    }

    /// A double click on the picture at `at`: it grows out of the page to
    /// fill the window (see `kolada_ui::lightbox`).
    pub(super) fn open_lightbox(&mut self, at: usize) -> bool {
        let Some(line) = self.lines.iter().find(|l| l.para.start == at) else { return false };
        let Some(path) = line.para.picture.as_ref().and_then(|p| p.path.clone()) else { return false };
        let Some(from) = self.hits.iter().find(|(_, t)| *t == kolada_ui::scene::Target::Picture(at)).map(|(r, _)| *r) else { return false };
        let shown = ((from.width() * self.scale) as u32, (from.height() * self.scale) as u32);
        let text = &self.editor.text()[line.para.start..line.para.end];
        let dir = self.path.as_deref().and_then(Path::parent).filter(|d| d.starts_with(&self.vault)).unwrap_or(&self.vault).to_path_buf();
        let size = match images::embed_in(text).map(|name| self.pictures.borrow_mut().place(&self.vault, &dir, name)) {
            Some(images::Lookup::Found(_, w, h)) => (w, h),
            _ => shown,
        };
        self.lightbox = Some(kolada_ui::lightbox::Lightbox::open(path, from, size));
        true
    }

    /// A click or Esc puts the picture back: true if it was out.
    pub(super) fn close_lightbox(&mut self) -> bool {
        self.lightbox.as_mut().is_some_and(|l| l.close())
    }

    /// The line at `at`, if it shows a picture that is there: its start and end.
    fn picture_line(&self, at: usize) -> Option<(usize, usize)> {
        let text = self.editor.text();
        let line = self.editor.line_at(at.min(text.len()));
        let name = images::embed_in(&text[line.start..line.end])?;
        let p = self.note_pictures()?;
        let found = p.images.borrow_mut().place(p.vault, &p.dir, name);
        (!matches!(found, Lookup::Missing)).then_some((line.start, line.end))
    }

    /// The picture chosen whole, if that is what is selected.
    pub(super) fn selected_picture(&self) -> Option<(usize, usize)> {
        let s = self.editor.selection();
        self.picture_line(s.start()).filter(|&(a, b)| s.start() == a && s.end() == b && a < b)
    }

    /// A caret left inside a picture's line (by a click beside it, or the
    /// arrows) chooses the picture: its text is never edited by hand.
    pub(super) fn snap_to_picture(&mut self) {
        let s = self.editor.selection();
        if s.is_caret() {
            if let Some((a, _)) = self.picture_line(s.head) {
                self.select_picture(a);
            }
        }
    }

    /// An arrow with a picture chosen: to the line before it or after.
    /// False if there is no picture chosen.
    pub(super) fn step_off_picture(&mut self, back: bool) -> bool {
        let Some((a, b)) = self.selected_picture() else { return false };
        let len = self.editor.text().len();
        let to = if back { a.checked_sub(1) } else { (b < len).then_some(b + 1) };
        if let Some(to) = to {
            self.editor.set_selection(Selection::caret(to));
            self.snap_to_picture();
            self.caret_moved(true);
            self.reveal = true;
        }
        true
    }

    /// Backspace or Delete with a picture chosen: it goes, with its line
    /// break, so no empty line is left. None with no picture chosen.
    pub(super) fn delete_picture(&mut self) -> Option<bool> {
        let (a, b) = self.selected_picture()?;
        let len = self.editor.text().len();
        let (from, to) = if b < len { (a, b + 1) } else { (a.saturating_sub(1), b) };
        self.editor.set_selection(Selection { anchor: from, head: to });
        Some(self.edit(|e| e.backspace(false)))
    }

    /// Return with a picture chosen starts a line after it. None with no
    /// picture chosen.
    pub(super) fn line_after_picture(&mut self) -> Option<bool> {
        let (_, b) = self.selected_picture()?;
        self.editor.set_selection(Selection::caret(b));
        Some(self.edit(|e| e.insert("\n")))
    }

    /// Typing (or pasting) with a picture chosen goes on a new line after
    /// it, rather than over it.
    pub(super) fn leave_picture_for_text(&mut self) {
        if let Some((_, b)) = self.selected_picture() {
            self.editor.set_selection(Selection::caret(b));
            self.edit(|e| e.insert("\n"));
        }
    }

    /// The right edge of the picture at `at` was pressed: a drag resizes it.
    pub(super) fn begin_resize(&mut self, at: usize, x: f32) -> bool {
        let Some(line) = self.lines.iter().find(|l| l.para.start == at) else { return false };
        let Some(pic) = &line.para.picture else { return false };
        let most = self.column_width - line.para.x;
        let text = &self.editor.text()[line.para.start..line.para.end];
        // Its own width: its pixels, a pixel a DIP, as far as the column allows.
        let natural = match (&pic.path, images::embed_in(text)) {
            (Some(_), Some(name)) => {
                let dir = self.path.as_deref().and_then(Path::parent).filter(|d| d.starts_with(&self.vault)).unwrap_or(&self.vault).to_path_buf();
                match self.pictures.borrow_mut().place(&self.vault, &dir, name) {
                    Lookup::Found(_, w, h) => fit_picture(w, h, most, None).0,
                    _ => pic.width,
                }
            }
            _ => pic.width,
        };
        self.resizing = Some(Resizing { line: at, from_x: x, start_width: pic.width, width: pic.width, most, natural });
        self.select_picture(at)
    }

    /// The pointer is at `x` DIPs with the edge held.
    pub(super) fn resize_to(&mut self, x: f32) -> bool {
        let Some(r) = &mut self.resizing else { return false };
        let was = r.width;
        r.width = (r.start_width + x - r.from_x).clamp(PICTURE_LEAST, r.most.max(PICTURE_LEAST));
        // Close to its own width, it snaps there.
        if (r.width - r.natural).abs() < 6.0 {
            r.width = r.natural;
        }
        r.width != was
    }

    /// The drag ended: the line says the width (or, back at the picture's own
    /// width, no width), as Obsidian writes it.
    pub(super) fn end_resize(&mut self) -> bool {
        let Some(r) = self.resizing.take() else { return false };
        if (r.width - r.start_width).abs() < 1.0 {
            return true;
        }
        let line = self.editor.line_at(r.line.min(self.editor.text().len()));
        let old = self.editor.text()[line.start..line.end].to_string();
        let width = ((r.width - r.natural).abs() >= 1.0).then(|| r.width.round() as u32);
        let new = images::with_width(&old, width);
        if new != old {
            self.editor.set_selection(Selection { anchor: line.start, head: line.end });
            self.edit(|e| e.insert(&new));
            self.select_picture(line.start);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_convert::{Pasted, Picture};

    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-app-pics-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_a_free_name() {
        let dir = scratch("free");
        std::fs::write(dir.join("shot.png"), "").unwrap();
        std::fs::write(dir.join("shot 1.png"), "").unwrap();
        assert_eq!(free_name(&dir, "shot.png"), dir.join("shot 2.png"));
        assert_eq!(free_name(&dir, "new.png"), dir.join("new.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pictures_are_known_by_how_they_start() {
        assert!(is_picture(b"\x89PNG\r\n\x1a\nrest"));
        assert!(is_picture(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(is_picture(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(!is_picture(b"<!DOCTYPE html><html>"));
        assert!(!is_picture(b"RIFF\0\0\0\0WAVEfmt "));
        assert!(!is_picture(b""));
    }

    #[test]
    fn the_stamp_is_fourteen_digits() {
        let s = stamp(&crate::services::NoServices);
        assert_eq!(s.len(), 14, "{s}");
        assert!(s.bytes().all(|b| b.is_ascii_digit()) && s.starts_with("20"), "{s}");
    }

    #[test]
    fn a_pastes_pictures_are_saved_and_web_ones_wait_to_be_fetched() {
        let dir = scratch("save");
        let dir_in = dir.join("attachments");
        let outside = dir.join("word.png");
        std::fs::write(&outside, b"local").unwrap();
        let pasted = Pasted {
            markdown: "A\n\n![[one.png]]\n\nB ![[two.jpg]]\n\n![[three.png]]".into(),
            pictures: vec![Picture { name: "one.png".into(), data: b"inside".to_vec() }],
            fetch: vec![("two.jpg".into(), "https://x/y.jpg".into()), ("three.png".into(), format!("file:{}", outside.display()))],
        };
        let (text, fetch) = save_pasted(pasted, Some(&dir_in), "20261005183012");
        assert_eq!(text, "A\n\n![[Pasted image 20261005183012.png]]\n\nB ![[Pasted image 20261005183012.jpg]]\n\n![[Pasted image 20261005183012 1.png]]");
        assert_eq!(std::fs::read(dir_in.join("Pasted image 20261005183012.png")).unwrap(), b"inside");
        assert_eq!(std::fs::read(dir_in.join("Pasted image 20261005183012 1.png")).unwrap(), b"local");
        // The web one holds its name, empty, until it is fetched.
        assert_eq!(fetch, vec![(dir_in.join("Pasted image 20261005183012.jpg"), "https://x/y.jpg".to_string())]);
        assert_eq!(std::fs::read(&fetch[0].0).unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A PNG's first bytes: its signature and a header saying `w` x `h`.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 6, 0, 0, 0]);
        b
    }

    #[test]
    fn a_chosen_picture_is_stepped_off_by_the_arrows_and_backspace_takes_it_and_its_line() {
        let dir = scratch("keys");
        std::fs::write(dir.join("pic.png"), png_header(800, 400)).unwrap();
        std::fs::write(dir.join("T.md"), "# T\nbefore\n![[pic.png]]\nafter\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        // Arrowing onto its line chooses it whole.
        let before = m.text().find("before").unwrap() + 3;
        m.editor.set_selection(Selection::caret(before));
        m.go(kolada_editor::Motion::Down, false);
        assert_eq!(m.selected_text(), "![[pic.png]]");
        // Down again: off it, to the line after.
        m.go(kolada_editor::Motion::Down, false);
        assert_eq!(m.selection().head, m.text().find("after").unwrap());
        m.go(kolada_editor::Motion::Up, false);
        assert_eq!(m.selected_text(), "![[pic.png]]");
        // Typing goes on a new line after it, not over it.
        m.insert("x");
        assert!(m.text().contains("![[pic.png]]\nx\nafter"), "{:?}", m.text());
        m.undo();
        m.undo();
        m.editor.set_selection(Selection { anchor: m.text().find("![[").unwrap(), head: m.text().find("]]").unwrap() + 2 });
        // Backspace: the picture and its line go.
        assert!(m.backspace(false));
        assert_eq!(m.text(), "# T\nbefore\nafter\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dragging_a_pictures_right_edge_resizes_it_and_the_line_says_the_width() {
        use kolada_ui::scene::Target;
        let dir = scratch("resize");
        std::fs::write(dir.join("pic.png"), png_header(800, 400)).unwrap();
        std::fs::write(dir.join("Note.md"), "# T\n\n![[pic.png]]\n\nafter\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        let shown = m.hits.iter().find_map(|(r, t)| matches!(t, Target::Picture(_)).then_some(*r)).expect("the picture is drawn and takes clicks");
        let start = shown.right - shown.left;
        // A click on the picture chooses it whole.
        assert!(m.pointer_down(shown.left + 20.0, shown.top + 20.0, 1, false));
        assert_eq!(m.selected_text(), "![[pic.png]]");
        // Its right edge, dragged 100 DIPs in.
        let (x, y) = (shown.right + 2.0, shown.top + 10.0);
        assert!(m.pointer_down(x, y, 1, false));
        assert!(m.resizing.is_some(), "the edge was caught");
        assert!(m.pointer_dragged(x - 100.0, y));
        m.frame(2000, 1440, 2.0);
        assert!(m.pointer_up());
        assert!(m.resizing.is_none());
        let width = images::embed_width(m.text().lines().find(|l| l.starts_with("![[")).unwrap()).expect("the line says a width");
        assert!((width as f32 - (start - 100.0)).abs() <= 1.0, "{width} for {start}");
        // One undo gives the picture its own size back.
        assert!(m.undo());
        assert!(m.text().contains("![[pic.png]]"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_double_click_opens_a_picture_full_size_and_a_click_or_esc_puts_it_back() {
        use kolada_ui::scene::Target;
        let dir = scratch("lightbox");
        std::fs::write(dir.join("pic.png"), png_header(1600, 900)).unwrap();
        std::fs::write(dir.join("Note.md"), "# T\n\n![[pic.png|300]]\n\nafter\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        let (w, h) = (1000.0, 720.0);
        m.frame(2000, 1440, 2.0);
        let shown = m.hits.iter().find_map(|(r, t)| matches!(t, Target::Picture(_)).then_some(*r)).unwrap();
        let (x, y) = (shown.left + 20.0, shown.top + 20.0);
        m.pointer_down(x, y, 1, false);
        m.pointer_up();
        assert!(m.pointer_down(x, y, 2, false), "the double click");
        m.pointer_up();
        let l = m.lightbox.clone().expect("out of the page");
        assert_eq!((l.from, l.size), (shown, (1600, 900)), "from where it was, its own size known");
        assert!(m.wants_frames(), "it grows");
        // Grown, over everything: its ground and a click anywhere are its.
        for _ in 0..120 {
            m.lightbox.as_mut().unwrap().step(1.0 / 60.0);
        }
        m.frame(2000, 1440, 2.0);
        assert!(!m.wants_frames(), "settled: no frames");
        assert_eq!(kolada_ui::scene::hit(&m.hits, 5.0, 5.0), Some(Target::Lightbox));
        let full = m.lightbox.as_ref().unwrap().full(w, h, 2.0);
        assert!(full.width() > shown.width() * 2.0, "full size: {full:?}");
        // Typing writes nothing behind it: it puts the picture back.
        let before = m.text().to_string();
        assert!(m.insert("x"));
        assert_eq!(m.text(), before);
        assert!(!m.lightbox.as_ref().unwrap().is_open());
        while m.lightbox.as_mut().is_some_and(|l| l.step(1.0 / 60.0)) {}
        m.tick();
        assert!(m.lightbox.is_none() && !m.wants_frames(), "back in the page, and gone");
        // Esc, and a click, put it back too.
        m.frame(2000, 1440, 2.0);
        m.pointer_down(x, y, 2, false);
        m.pointer_up();
        assert!(m.escape() && !m.lightbox.as_ref().unwrap().is_open());
        m.lightbox = None;
        m.frame(2000, 1440, 2.0);
        m.pointer_down(x, y, 2, false);
        m.pointer_up();
        assert!(m.pointer_down(10.0, 10.0, 1, false) && !m.lightbox.as_ref().unwrap().is_open());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_touch_screens_chosen_picture_is_sized_replaced_drawn_on_and_deleted() {
        let dir = scratch("touch");
        std::fs::write(dir.join("pic.png"), png_header(800, 400)).unwrap();
        std::fs::write(dir.join("Note.md"), "# T\n\n![[pic.png]]\n\nafter\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.set_bare(true);
        m.frame(1179, 2556, 3.0);
        assert!(m.chosen_picture().is_none(), "nothing chosen");
        let shown = m.hits.iter().find_map(|(r, t)| matches!(t, kolada_ui::scene::Target::Picture(_)).then_some(*r)).unwrap();
        // Tapped: chosen whole, where it is drawn, and not a drawing.
        m.tap(shown.left + 20.0, shown.top + 20.0, 1);
        let (place, drawing) = m.chosen_picture().expect("chosen");
        assert_eq!(place, shown);
        assert!(!drawing);
        let column = shown.right - shown.left;
        // Its handle dragged 80 DIPs in: the line says the width.
        assert!(m.drag_picture_edge(0, shown.right));
        m.drag_picture_edge(1, shown.right - 80.0);
        m.drag_picture_edge(2, shown.right - 80.0);
        let line = |m: &Model| m.text().lines().find(|l| l.starts_with("![[")).unwrap().to_string();
        assert!(images::embed_width(&line(&m)).is_some_and(|w| (w as f32 - (column - 80.0)).abs() <= 1.0), "{}", line(&m));
        // Half the column; then its own size again.
        assert!(m.size_chosen_picture(Some(0.5)));
        let half = images::embed_width(&line(&m)).unwrap() as f32;
        assert!(half > 100.0 && half < column, "{half}");
        assert!(m.size_chosen_picture(None));
        assert_eq!(line(&m), "![[pic.png]]");
        // Replaced: a new file, the width kept, still chosen; undo puts it back.
        m.size_chosen_picture(Some(0.5));
        assert!(m.replace_chosen_picture(&png_header(100, 100)));
        let replaced = line(&m);
        assert!(replaced.starts_with("![[Pasted image ") && replaced.ends_with(&format!("|{}]]", half as u32)), "{replaced}");
        assert_eq!(m.selected_text(), replaced);
        assert!(m.undo());
        assert!(line(&m).starts_with("![[pic.png|"));
        // Not a drawing: nothing drawn over it.
        m.frame(1179, 2556, 3.0);
        let shown = m.hits.iter().find_map(|(r, t)| matches!(t, kolada_ui::scene::Target::Picture(_)).then_some(*r)).unwrap();
        m.tap(shown.left + 5.0, shown.top + 5.0, 1);
        assert!(!m.redraw_chosen_picture(&png_header(10, 10)));
        assert_eq!(std::fs::read(dir.join("pic.png")).unwrap(), png_header(800, 400));
        // Deleted: it and its line.
        assert!(m.delete_chosen_picture());
        assert!(!m.text().contains("![["), "{:?}", m.text());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_drawing_made_here_is_named_so_and_drawn_on_again_in_place() {
        assert!(is_drawing("Drawing 20261027101500.png") && is_drawing("art/Drawing 20261027101500 1.png"));
        assert!(!is_drawing("Pasted image 20261027101500.png") && !is_drawing("Drawings.png") && !is_drawing("Drawing 1.jpg"));
        let dir = scratch("drawing");
        std::fs::write(dir.join("Note.md"), "# T\n\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.set_bare(true);
        m.frame(1179, 2556, 3.0);
        assert!(m.insert_drawing(&png_header(300, 200)));
        let line = m.text().lines().find(|l| l.starts_with("![[")).unwrap().to_string();
        assert!(line.starts_with("![[Drawing ") && line.ends_with(".png]]"), "{line}");
        m.frame(1179, 2556, 3.0);
        let at = m.text().find("![[").unwrap();
        m.select_picture(at);
        m.frame(1179, 2556, 3.0);
        let (_, drawing) = m.chosen_picture().expect("chosen");
        assert!(drawing);
        let file = m.chosen_picture_file().expect("found");
        assert!(m.redraw_chosen_picture(&png_header(400, 200)));
        assert_eq!(std::fs::read(&file).unwrap(), png_header(400, 200), "written in place");
        assert_eq!(m.text().lines().find(|l| l.starts_with("![[")).unwrap(), line, "the note unchanged");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_dragged_back_to_its_own_width_says_no_width() {
        use kolada_ui::scene::Target;
        let dir = scratch("resize-back");
        std::fs::write(dir.join("pic.png"), png_header(300, 150)).unwrap();
        std::fs::write(dir.join("Note.md"), "# T\n\n![[pic.png|200]]\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        let shown = m.hits.iter().find_map(|(r, t)| matches!(t, Target::Picture(_)).then_some(*r)).unwrap();
        assert!((shown.right - shown.left - 200.0).abs() < 1.0);
        let (x, y) = (shown.right + 2.0, shown.top + 10.0);
        m.pointer_down(x, y, 1, false);
        // 100 wider is its own 300; a few DIPs short still snaps to it.
        m.pointer_dragged(x + 97.0, y);
        m.pointer_up();
        assert!(m.text().contains("![[pic.png]]"), "{:?}", m.text());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_folder_the_embeds_are_left_out() {
        let pasted = Pasted {
            markdown: "A\n\n![[one.png]]\n\nB".into(),
            pictures: vec![Picture { name: "one.png".into(), data: Vec::new() }],
            fetch: Vec::new(),
        };
        assert_eq!(save_pasted(pasted, None, "1"), ("A\n\nB".to_string(), Vec::new()));
    }
}
