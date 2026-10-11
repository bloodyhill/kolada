//! Pasting. What was copied with its formatting (from a web page, Word,
//! Google Docs, Outlook, Teams, Excel) comes in as Markdown, its headings,
//! lists, links, tables and pictures kept (see `convert::paste`); plain
//! text, code from an editor, and anything pasted into code or a table's
//! cell, as it is. Pictures inside what was copied, and those Word and
//! Outlook keep on this machine, are saved with the note's attachments at
//! once; pictures on the web are fetched after (see `http`), drawn as a
//! quiet box until they come, and left as a link to where they are when
//! they cannot be had.

use super::*;

/// A picture of a paste, fetched from the web: the file it went to, where
/// it was, and whether it came.
pub(crate) struct Fetched {
    path: PathBuf,
    url: String,
    ok: bool,
}

impl App {
    /// Ctrl+V on the page. A picture on the clipboard (and no text), as a
    /// picture.
    pub(super) fn paste(&mut self) {
        let Some(text) = clipboard::get(self.hwnd) else {
            self.paste_picture();
            return;
        };
        self.leave_picture_for_text();
        let pasted = if self.editor.takes_markdown() && !self.read_only {
            let dir = self.attachment_dir();
            self.copied(dir.as_deref())
        } else {
            None
        };
        match pasted {
            Some(p) => {
                self.edit(|e| e.paste_markdown(&p.markdown));
                self.pictures_pasted(p.fetch);
            }
            None => self.edit(|e| e.insert(&text)),
        }
    }

    /// Ctrl+Shift+V: the copied text alone, its formatting left behind.
    pub(super) fn paste_plain(&mut self) {
        if let Some(text) = clipboard::get(self.hwnd) {
            self.leave_picture_for_text();
            self.edit(|e| e.insert(&text));
        }
    }

    /// Ctrl+V (`plain`: Ctrl+Shift+V) in the note open over the page.
    pub(super) fn popup_paste(&mut self, plain: bool) {
        let Some(text) = clipboard::get(self.hwnd) else { return };
        let Some(p) = &self.popup else { return };
        let rich = !plain && p.editor.takes_markdown();
        let pasted = if rich {
            let dir = p.path.parent().map(Path::to_path_buf);
            let dir = dir.and_then(|d| self.attachments_for(&d));
            self.copied(dir.as_deref())
        } else {
            None
        };
        match pasted {
            Some(p) => {
                self.popup_edit(|e| e.paste_markdown(&p.markdown));
                self.pictures_pasted(p.fetch);
            }
            None => self.popup_edit(|e| e.insert(&text)),
        }
    }

    /// What was copied, as Markdown, when its formatting says more than its
    /// plain text; its pictures saved in `pictures`.
    fn copied(&self, pictures: Option<&Path>) -> Option<crate::convert::Paste> {
        crate::convert::paste(&clipboard::get_html(self.hwnd)?, pictures)
    }

    /// Pictures just pasted: those saved are found, those on the web are
    /// fetched, one after another, beside the window's thread.
    fn pictures_pasted(&mut self, fetch: Vec<(PathBuf, String)>) {
        self.images.borrow_mut().coming(fetch.iter().map(|(path, _)| path.clone()));
        self.request_frame();
        if fetch.is_empty() {
            return;
        }
        let hwnd = crate::Handoff(self.hwnd);
        std::thread::spawn(move || {
            let hwnd = hwnd;
            for (path, url) in fetch {
                let ok = crate::http::get(&url, crate::http::MOST).filter(|b| crate::http::is_picture(b)).is_some_and(|b| std::fs::write(&path, b).is_ok());
                let fetched = Box::into_raw(Box::new(Fetched { path, url, ok }));
                let posted = unsafe { PostMessageW(Some(hwnd.0), window::WM_APP_FETCHED, WPARAM(0), LPARAM(fetched as isize)) };
                if posted.is_err() {
                    drop(unsafe { Box::from_raw(fetched) });
                }
            }
        });
    }

    /// A pasted picture came from the web, and is drawn; or it could not
    /// be had, and its embed becomes a link to where it is, wherever the
    /// note it went into is open.
    pub(super) fn picture_fetched(&mut self, f: Fetched) {
        if !f.ok {
            let _ = std::fs::remove_file(&f.path);
            let name = f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (embed, link) = (format!("![[{name}]]"), format!("![]({})", f.url.replace(' ', "%20")));
            if let Some(at) = self.editor.text().find(&embed) {
                let swap = (at, at + embed.len(), link.clone());
                self.edit(|e| e.replace_ranges(&[swap]));
            }
            if let Some(at) = self.popup.as_ref().and_then(|p| p.editor.text().find(&embed)) {
                self.popup_edit(|e| e.replace_ranges(&[(at, at + embed.len(), link)]));
            }
        }
        self.images.borrow_mut().came(&f.path);
        self.request_frame();
    }
}
