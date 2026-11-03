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

/// Whether a paste takes the files copied rather than the text: any files
/// with no text beside them, or pictures among them whatever the text
/// (some apps put the files' names there too).
fn pastes_files(files: &[PathBuf], text: Option<&str>) -> bool {
    !files.is_empty() && (text.is_none_or(|t| t.trim().is_empty()) || files.iter().any(|f| crate::images::is_image(&f.to_string_lossy())))
}

/// A picture of a paste, fetched from the web: the file it went to, where
/// it was, and whether it came.
pub(crate) struct Fetched {
    path: PathBuf,
    url: String,
    ok: bool,
}

impl App {
    /// Ctrl+V on the page. A picture on the clipboard (and no text), as a
    /// picture; files copied (a picture copied in Explorer or Photos), as
    /// if they were dropped at the caret.
    pub(super) fn paste(&mut self) {
        let text = clipboard::get(self.hwnd);
        let files = clipboard::get_files(self.hwnd);
        if pastes_files(&files, text.as_deref()) {
            let at = self.editor.selection().head;
            self.drop_files(files, 0, 0, Some(at));
            return;
        }
        let Some(text) = text else {
            self.paste_picture();
            return;
        };
        self.leave_picture_for_text();
        // A web address over selected words makes them a link.
        if !self.read_only && kolada_ui::linkfield::address_over(&self.editor, &text).is_some() {
            self.edit(|e| {
                kolada_ui::linkfield::paste_address(e, &text);
            });
            return;
        }
        // Copied in Kolada: its Markdown, exactly.
        if let Some(markdown) = clipboard::get_kolada(self.hwnd) {
            let structured = self.editor.takes_markdown();
            self.edit(|e| if structured { e.paste_markdown(&markdown) } else { e.insert(&markdown) });
            return;
        }
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
        if !plain && let Some(markdown) = clipboard::get_kolada(self.hwnd) {
            let structured = p.editor.takes_markdown();
            self.popup_edit(|e| if structured { e.paste_markdown(&markdown) } else { e.insert(&markdown) });
            return;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture copied as a file (Explorer, Photos) is pasted as the
    /// picture; text copied stays text; files with their names as text
    /// beside them are the files when pictures are among them.
    #[test]
    fn a_picture_copied_as_a_file_pastes_as_the_picture() {
        let png = [PathBuf::from(r"C:\Users\me\Pictures\Beach.png")];
        let doc = [PathBuf::from(r"C:\Users\me\Documents\Plan.docx")];
        assert!(pastes_files(&png, None), "Explorer's Copy: files, no text");
        assert!(pastes_files(&png, Some("Beach.png")), "a picture wins over its name");
        assert!(pastes_files(&doc, None), "a document alone, as a drop would take it");
        assert!(!pastes_files(&doc, Some("Plan.docx")), "a document's name copied as text stays text");
        assert!(!pastes_files(&[], Some("Gate B, row 14")), "plain text");
        assert!(!pastes_files(&[], None));
    }
}
