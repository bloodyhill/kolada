//! Pictures coming into a note: pasted (a screenshot on the clipboard) or
//! dropped from Explorer. They are saved where the vault keeps attachments
//! (Obsidian's setting, if it has one) and embedded on a line of their own,
//! the way Obsidian names and embeds them.

use crate::clipboard::Picture;
use crate::images;

use super::*;

/// `dir/name`, or `dir/name 1.ext`, `dir/name 2.ext`... if taken.
fn free_name(dir: &Path, name: &str) -> PathBuf {
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

/// A name for a pasted picture, as Obsidian gives them: when it was pasted.
fn pasted_name() -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("Pasted image {:04}{:02}{:02}{:02}{:02}{:02}.png", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

impl App {
    /// The folder a picture for the open note goes in (made if need be).
    /// Where pictures of notes in `dir` go, as the vault's settings say.
    pub(super) fn attachments_for(&mut self, dir: &Path) -> Option<PathBuf> {
        let vault = self.ensure_vault()?;
        let setting = std::fs::read_to_string(vault.join(".obsidian").join("app.json")).ok().and_then(|j| images::attachment_setting(&j));
        Some(images::attachment_dir(&vault, dir, setting.as_deref().unwrap_or("")))
    }

    pub(super) fn attachment_dir(&mut self) -> Option<PathBuf> {
        let vault = self.ensure_vault()?;
        let setting = std::fs::read_to_string(vault.join(".obsidian").join("app.json")).ok().and_then(|j| images::attachment_setting(&j));
        let note_dir = if self.session.note.is_empty() {
            vault.clone()
        } else {
            self.note_path().parent().map_or_else(|| vault.clone(), Path::to_path_buf)
        };
        let dir = images::attachment_dir(&vault, &note_dir, setting.as_deref().unwrap_or(""));
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    /// Embeds `names` at `at`, each on a line of its own, and puts the caret
    /// on the line after them.
    fn embed_pictures(&mut self, names: &[String], at: usize) {
        if names.is_empty() || self.read_only {
            return;
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
        self.edit(|e| e.insert(&block));
        self.images.borrow_mut().forget_places();
    }

    /// Ctrl+V with a picture (and no text) on the clipboard: it is saved as
    /// a PNG with the vault's attachments and embedded at the caret. False
    /// if there is none.
    pub(super) fn paste_picture(&mut self) -> bool {
        if self.read_only {
            return false;
        }
        let Some(picture) = clipboard::get_picture(self.hwnd) else { return false };
        let Some(dir) = self.attachment_dir() else { return true };
        let path = free_name(&dir, &pasted_name());
        let saved = match picture {
            Picture::Png(bytes) => std::fs::write(&path, bytes).is_ok(),
            Picture::Pixels { width, height, bgra } => images::write_png(&path, width, height, &bgra, false).is_ok(),
        };
        if !saved {
            self.whisper("Could not save the picture".to_string());
            return true;
        }
        let name = stem_with_ext(&path);
        let at = self.editor.selection().head;
        self.embed_pictures(&[name], at);
        true
    }

    /// Files dropped from Explorer at a client point in pixels: pictures
    /// are copied in with the attachments and embedded there; notes are
    /// imported, and the first opens.
    /// Pictures land at `at` if given (where a drag showed them going), else
    /// at the point.
    pub(super) fn drop_files(&mut self, files: Vec<PathBuf>, x: i32, y: i32, at: Option<usize>) {
        let (pictures, notes): (Vec<PathBuf>, Vec<PathBuf>) =
            files.into_iter().partition(|f| images::is_image(&f.to_string_lossy()));
        if !pictures.is_empty()
            && !self.read_only
            && let Some(dir) = self.attachment_dir()
        {
            let vault = PathBuf::from(&self.session.vault);
            let mut names = Vec::new();
            for f in &pictures {
                // Already in the vault: embedded where it is.
                let path = if f.starts_with(&vault) {
                    f.clone()
                } else {
                    let to = free_name(&dir, &stem_with_ext(f));
                    if std::fs::copy(f, &to).is_err() {
                        continue;
                    }
                    to
                };
                names.push(stem_with_ext(&path));
            }
            self.ensure_drawn();
            let scale = window::dpi(self.hwnd) as f32 / 96.0;
            let at = at.unwrap_or_else(|| self.offset_at(x as f32 / scale, y as f32 / scale));
            self.embed_pictures(&names, at);
        }
        // Another app's export: its notes, in a folder of their own.
        let (exports, notes): (Vec<PathBuf>, Vec<PathBuf>) = notes.into_iter().partition(|f| super::share::is_export(f));
        if !exports.is_empty()
            && let Some(vault) = self.ensure_vault()
        {
            return self.import_exports(&exports, &vault);
        }
        // Documents (DOCX, PDF, HTML): each made a note.
        let (documents, notes): (Vec<PathBuf>, Vec<PathBuf>) = notes.into_iter().partition(|f| crate::convert::is_document(f));
        if !documents.is_empty()
            && let Some(vault) = self.ensure_vault()
        {
            self.import_documents(&documents, &vault);
        }
        let notes: Vec<PathBuf> = notes
            .into_iter()
            .filter(|f| f.extension().is_some_and(|e| ["md", "markdown", "txt"].contains(&e.to_string_lossy().to_lowercase().as_str())))
            .collect();
        if notes.is_empty() {
            return;
        }
        let Some(vault) = self.ensure_vault() else { return };
        // A note of this vault just opens.
        if let [only] = &notes[..]
            && only.starts_with(&vault)
        {
            return self.open(only);
        }
        let (mut count, mut first) = (0, None);
        for f in notes.iter().filter(|f| !f.starts_with(&vault)) {
            let to = note::unique_path(&vault, &stem(f), None);
            if std::fs::copy(f, &to).is_ok() {
                count += 1;
                first.get_or_insert(to);
            }
        }
        self.imported(count, first);
    }
}

/// A picture being resized by its right edge: its line, where the drag
/// began and the width then, its width now, the widest it can be, and its
/// own width (which a drag back to it returns to).
pub(super) struct Resizing {
    pub line: usize,
    from_x: f32,
    start_width: f32,
    pub width: f32,
    most: f32,
    natural: f32,
}

impl App {
    /// The line holding `at`, if it shows a picture (a picture embed whose
    /// file is there, or being looked for): its start and end.
    fn picture_line(&self, at: usize) -> Option<(usize, usize)> {
        let line = self.editor.line_at(at.min(self.editor.text().len()));
        let name = images::embed_in(&self.editor.text()[line.start..line.end])?;
        let p = self.pictures_of(&self.note_path())?;
        let found = p.images.borrow_mut().place(p.vault, &p.dir, name);
        (!matches!(found, images::Lookup::Missing)).then_some((line.start, line.end))
    }

    /// The picture chosen whole, if that is what is selected.
    pub(super) fn selected_picture(&self) -> Option<(usize, usize)> {
        let s = self.editor.selection();
        self.picture_line(s.start()).filter(|&(a, b)| s.start() == a && s.end() == b && a < b)
    }

    /// Chooses the picture on the line at `at`, whole.
    fn select_picture(&mut self, at: usize) {
        let line = self.editor.line_at(at);
        self.editor.set_selection(Selection { anchor: line.start, head: line.end });
        self.caret_moved(false);
    }

    /// A caret left inside a picture's line (by a click beside it, or the
    /// arrows) chooses the picture: its text is never edited by hand.
    pub(super) fn snap_to_picture(&mut self) {
        let s = self.editor.selection();
        if s.is_caret()
            && let Some((a, _)) = self.picture_line(s.head)
        {
            self.select_picture(a);
        }
    }

    /// An arrow key with a picture chosen: to the line before it or after.
    /// False if there is no picture, or nowhere to go.
    pub(super) fn step_off_picture(&mut self, back: bool) -> bool {
        let Some((a, b)) = self.selected_picture() else { return false };
        let len = self.editor.text().len();
        let to = if back { a.checked_sub(1) } else { (b < len).then_some(b + 1) };
        if let Some(to) = to {
            self.editor.set_selection(Selection::caret(to));
            self.snap_to_picture();
            self.caret_moved(true);
        }
        true
    }

    /// Keys for a chosen picture: Backspace and Delete remove it, Enter
    /// starts a line after it. False for other keys, or no picture.
    pub(super) fn picture_key(&mut self, vk: u16) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_BACK, VK_DELETE, VK_RETURN};
        let Some((a, b)) = self.selected_picture() else { return false };
        let len = self.editor.text().len();
        if vk == VK_BACK.0 || vk == VK_DELETE.0 {
            // With its line break, so no empty line is left.
            let (from, to) = if b < len { (a, b + 1) } else { (a.saturating_sub(1), b) };
            self.editor.set_selection(Selection { anchor: from, head: to });
            self.edit(|e| e.backspace(false));
            return true;
        }
        if vk == VK_RETURN.0 {
            self.editor.set_selection(Selection::caret(b));
            self.edit(|e| e.insert("\n"));
            return true;
        }
        false
    }

    /// Typing (or pasting) with a picture chosen goes on a new line after
    /// it, rather than over it.
    pub(super) fn leave_picture_for_text(&mut self) {
        if let Some((_, b)) = self.selected_picture() {
            self.editor.set_selection(Selection::caret(b));
            self.edit(|e| e.insert("\n"));
        }
    }

    /// A click on a picture chooses it; a double click opens its file.
    pub(super) fn picture_click(&mut self, at: usize, px: i32, py: i32) {
        if self.search.is_some() {
            self.close_search();
        }
        if self.count_click(px, py) >= 2 {
            let path = self.lines.iter().find(|l| l.para.start == at).and_then(|l| l.para.picture.as_ref()).and_then(|p| p.path.clone());
            if let Some(path) = path {
                let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
            }
            return;
        }
        self.select_picture(at);
    }

    /// The right edge of the picture at `at` was pressed: a drag resizes it.
    pub(super) fn begin_resize(&mut self, at: usize, px: i32) {
        let Some(line) = self.lines.iter().find(|l| l.para.start == at) else { return };
        let Some(pic) = &line.para.picture else { return };
        let most = self.column.width() - line.para.x;
        let natural = match (&pic.path, self.pictures_of(&self.note_path())) {
            (Some(_), Some(p)) => {
                let text = &self.editor.text()[line.para.start..line.para.end];
                match images::embed_in(text).map(|name| p.images.borrow_mut().place(p.vault, &p.dir, name)) {
                    Some(images::Lookup::Found(_, w, h)) => crate::note::fit_picture(w, h, most, None).0,
                    _ => pic.width,
                }
            }
            _ => pic.width,
        };
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        self.resizing = Some(Resizing { line: at, from_x: px as f32 / scale, start_width: pic.width, width: pic.width, most, natural });
        self.select_picture(at);
        unsafe { SetCapture(self.hwnd) };
    }

    pub(super) fn resize_to(&mut self, px: i32) {
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        if let Some(r) = &mut self.resizing {
            r.width = (r.start_width + px as f32 / scale - r.from_x).clamp(crate::note::PICTURE_LEAST, r.most);
            // Close to its own width, it snaps there.
            if (r.width - r.natural).abs() < 6.0 {
                r.width = r.natural;
            }
            self.request_frame();
        }
    }

    /// The drag ended: the line says the width (or, back at the picture's
    /// own width, no width), as Obsidian writes it.
    pub(super) fn end_resize(&mut self) {
        let Some(r) = self.resizing.take() else { return };
        unsafe {
            let _ = ReleaseCapture();
        }
        self.request_frame();
        if (r.width - r.start_width).abs() < 1.0 {
            return;
        }
        let line = self.editor.line_at(r.line);
        let old = self.editor.text()[line.start..line.end].to_string();
        let width = ((r.width - r.natural).abs() >= 1.0).then(|| r.width.round() as u32);
        let new = images::with_width(&old, width);
        if new != old {
            self.editor.set_selection(Selection { anchor: line.start, head: line.end });
            self.edit(|e| e.insert(&new));
            self.select_picture(line.start);
        }
    }
}

/// A file's name with its extension.
fn stem_with_ext(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_free_name() {
        let dir = std::env::temp_dir().join(format!("kolada-free-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shot.png"), "").unwrap();
        std::fs::write(dir.join("shot 1.png"), "").unwrap();
        assert_eq!(free_name(&dir, "shot.png"), dir.join("shot 2.png"));
        assert_eq!(free_name(&dir, "new.png"), dir.join("new.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
