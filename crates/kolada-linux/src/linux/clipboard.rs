//! The desktop's clipboard (arboard: X11's selections, or Wayland's through
//! XWayland), opened on first use, never before the first frame. A copy
//! writes plain text and HTML; Kolada's own exact Markdown is kept in the
//! app beside them, and a paste of that same text gets it back exactly (the
//! clipboard has no type of Kolada's own here yet).

use kolada_editor::slice::Copied;

#[derive(Default)]
pub struct Clipboard {
    board: Option<arboard::Clipboard>,
    /// The last copy made in Kolada: its plain text and its Markdown.
    ours: Option<(String, String)>,
}

/// What a paste brings: Kolada's own copy, or the clipboard's text and HTML.
#[derive(Debug, PartialEq)]
pub enum Pasted {
    Kolada(String),
    Text { plain: String, html: Option<String> },
    /// A picture and no text, as PNG bytes.
    Picture(Vec<u8>),
}

impl Clipboard {
    fn board(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.board.is_none() {
            self.board = arboard::Clipboard::new().ok();
        }
        self.board.as_mut()
    }

    /// A copy made in Kolada: plain text, HTML when there is some, and the
    /// Markdown remembered for a paste here.
    pub fn copy(&mut self, c: Copied) {
        let wrote = match self.board() {
            Some(b) if !c.html.is_empty() => b.set_html(c.html.as_str(), Some(c.plain.as_str())).is_ok(),
            Some(b) => b.set_text(c.plain.as_str()).is_ok(),
            None => false,
        };
        self.ours = wrote.then_some((c.plain, c.markdown));
    }

    /// Text on the clipboard (a link, a note's words) with no Markdown of ours.
    pub fn set_text(&mut self, text: &str, html: Option<&str>) {
        self.ours = None;
        if let Some(b) = self.board() {
            let _ = match html {
                Some(h) => b.set_html(h, Some(text)),
                None => b.set_text(text),
            };
        }
    }

    /// A password from Beam: kept out of the clipboard's history where the
    /// desktop's manager honours it (X11's `x-kde-passwordManagerHint`).
    pub fn set_secret(&mut self, text: &str) {
        use arboard::SetExtLinux;
        self.ours = None;
        if let Some(b) = self.board() {
            let _ = b.set().exclude_from_history().text(text);
        }
    }

    /// A picture from Beam (a PNG's or JPEG's bytes), as the clipboard's
    /// image.
    /// Files, as a file manager copies them (a uri-list).
    pub fn set_files(&mut self, paths: &[std::path::PathBuf]) -> bool {
        self.ours = None;
        self.board().is_some_and(|b| b.set().file_list(paths).is_ok())
    }

    pub fn set_picture(&mut self, bytes: &[u8]) -> bool {
        self.ours = None;
        let Ok(picture) = image::load_from_memory(bytes) else { return false };
        let rgba = picture.to_rgba8();
        let (w, h) = (rgba.width() as usize, rgba.height() as usize);
        self.board().is_some_and(|b| b.set_image(arboard::ImageData { width: w, height: h, bytes: rgba.into_raw().into() }).is_ok())
    }

    /// What a paste brings now.
    pub fn paste(&mut self) -> Option<Pasted> {
        let ours = self.ours.clone();
        let b = self.board()?;
        let Some(plain) = b.get_text().ok().filter(|t| !t.is_empty()) else {
            // No text: a picture, if there is one (as PNG).
            let image = b.get_image().ok()?;
            return super::arrive::png(image.width, image.height, &image.bytes).map(Pasted::Picture);
        };
        Some(match ours {
            Some((p, markdown)) if p == plain => Pasted::Kolada(markdown),
            _ => Pasted::Text { html: b.get().html().ok().filter(|h| !h.is_empty()), plain },
        })
    }

    /// Whether the clipboard holds formatted text (for a menu's "Paste as
    /// Plain Text").
    pub fn has_html(&mut self) -> bool {
        self.board().is_some_and(|b| b.get().html().is_ok_and(|h| !h.is_empty()))
    }

    /// The clipboard's text alone.
    pub fn text(&mut self) -> Option<String> {
        self.board()?.get_text().ok()
    }

    /// Files on the clipboard, as a file manager reads them.
    #[cfg(test)]
    pub fn files(&mut self) -> Vec<std::path::PathBuf> {
        self.board().and_then(|b| b.get().file_list().ok()).unwrap_or_default()
    }
}

/// Whether what is on the clipboard came marked secret: a password
/// manager's copy (`x-kde-passwordManagerHint` = `secret`, as KeePassXC,
/// Klipper and Beam's own `set_secret` mark it; Wayland's apps' come
/// through XWayland). Asked of the clipboard's owner only when Beam reads
/// the clipboard, on a display connection of its own, waiting at most a
/// quarter second.
pub fn concealed() -> bool {
    use std::time::{Duration, Instant};
    use x11_dl::xlib;
    let Ok(x) = xlib::Xlib::open() else { return false };
    unsafe {
        let dpy = (x.XOpenDisplay)(std::ptr::null());
        if dpy.is_null() {
            return false;
        }
        let atom = |name: &std::ffi::CStr| (x.XInternAtom)(dpy, name.as_ptr(), xlib::False);
        let (clipboard, hint, into) = (atom(c"CLIPBOARD"), atom(c"x-kde-passwordManagerHint"), atom(c"KOLADA_HINT"));
        let mut secret = false;
        if (x.XGetSelectionOwner)(dpy, clipboard) != 0 {
            let root = (x.XDefaultRootWindow)(dpy);
            let win = (x.XCreateSimpleWindow)(dpy, root, 0, 0, 1, 1, 0, 0, 0);
            (x.XConvertSelection)(dpy, clipboard, hint, into, win, xlib::CurrentTime);
            (x.XFlush)(dpy);
            let until = Instant::now() + Duration::from_millis(250);
            let mut ev: xlib::XEvent = std::mem::zeroed();
            while Instant::now() < until {
                if (x.XCheckTypedWindowEvent)(dpy, win, xlib::SelectionNotify, &mut ev) != 0 {
                    // Offered: its value; not offered, no property.
                    if ev.selection.property != 0 {
                        let (mut kind, mut format, mut n, mut after) = (0, 0, 0, 0);
                        let mut data: *mut u8 = std::ptr::null_mut();
                        let read = (x.XGetWindowProperty)(dpy, win, into, 0, 16, xlib::True, xlib::AnyPropertyType as u64, &mut kind, &mut format, &mut n, &mut after, &mut data);
                        if read == 0 && !data.is_null() {
                            secret = format == 8 && std::slice::from_raw_parts(data, n as usize) == b"secret";
                            (x.XFree)(data.cast());
                        }
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            (x.XDestroyWindow)(dpy, win);
        }
        (x.XCloseDisplay)(dpy);
        secret
    }
}

#[cfg(test)]
mod tests {
    use super::{Clipboard, concealed};

    /// The desktop has one clipboard: its tests take turns.
    static BOARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A password Beam puts on the clipboard reads back as marked secret
    /// (as a password manager's would); plain text does not. Needs a
    /// display (X11, or XWayland).
    #[test]
    fn a_secret_copy_reads_back_marked_and_plain_text_does_not() {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("no display: skipped");
            return;
        }
        let _turn = BOARD.lock().unwrap_or_else(|e| e.into_inner());
        let mut board = Clipboard::default();
        board.set_secret("9fG!kQ2#vB7z");
        assert!(concealed(), "marked secret");
        board.set_text("Pick up dry cleaning", None);
        assert!(!concealed(), "plain text is not");
    }

    /// Pictures copied as files read back as the same files (where there
    /// is a desktop's clipboard to hold them: under Xvfb, or a session).
    #[test]
    fn pictures_go_on_the_clipboard_as_files() {
        let _turn = BOARD.lock().unwrap_or_else(|e| e.into_inner());
        let mut c = Clipboard::default();
        if c.board().is_none() {
            eprintln!("no clipboard here (no display): nothing to check");
            return;
        }
        let dir = std::env::temp_dir().join(format!("kolada-copied-{}", std::process::id()));
        let paths = kolada_core::files::copies(&dir, &[("one.png".into(), vec![1]), ("two.png".into(), vec![2])]).unwrap();
        assert!(c.set_files(&paths));
        assert_eq!(c.files(), paths);
        let _ = std::fs::remove_dir_all(dir);
    }
}
