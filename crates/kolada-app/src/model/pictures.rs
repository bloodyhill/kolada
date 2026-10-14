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
        if self.in_field() || self.scrubbing() || self.settings.read_only {
            return false;
        }
        let Some(dir) = self.attachment_dir() else { return false };
        let path = free_name(&dir, &format!("Pasted image {stamp}.png"));
        if std::fs::write(&path, png).is_err() {
            return false;
        }
        let at = self.editor.selection().head;
        self.embed_pictures(&[name_of(&path)], at)
    }

    // Resizing: drag a picture's right edge.

    /// Chooses the picture on the line at `at`, whole.
    pub(super) fn select_picture(&mut self, at: usize) -> bool {
        let line = self.editor.line_at(at.min(self.editor.text().len()));
        self.editor.set_selection(Selection { anchor: line.start, head: line.end });
        self.reveal = false;
        true
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
