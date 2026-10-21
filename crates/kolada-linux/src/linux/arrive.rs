//! What comes into the window from outside: a picture on the clipboard (a
//! screenshot), and files dragged onto the window. The model decides what
//! they become (`kolada-app` `model/arrive.rs`, `model/card.rs`,
//! `model/pictures.rs`); this is how Linux hands them over.
//!
//! Drags come through winit, which on X11 names the files as they come over
//! (`HoveredFile`, one event a file) and as they are let go (`DroppedFile`,
//! likewise) but not where the pointer is while it drags: X gives the
//! pointer to the app being dragged from. So the pointer is asked of X
//! (`XQueryPointer`) while a drag is over the window, a frame at a time, and
//! the card that says what the files become follows it.

use std::path::PathBuf;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use winit::window::Window;

/// Files dragged over the window, gathered as winit names them.
#[derive(Default)]
pub struct Drag {
    /// Over the window, not yet let go.
    pub hovering: Vec<PathBuf>,
    /// Let go: taken in once the events of the drop are all in.
    pub dropped: Vec<PathBuf>,
}

impl Drag {
    /// One more file of the drag over the window; whether it is the first.
    pub fn hover(&mut self, path: PathBuf) -> bool {
        let first = self.hovering.is_empty();
        if !self.hovering.contains(&path) {
            self.hovering.push(path);
        }
        first
    }

    pub fn drop(&mut self, path: PathBuf) {
        if !self.dropped.contains(&path) {
            self.dropped.push(path);
        }
    }

    /// The drag left, or was let go.
    pub fn end(&mut self) -> Vec<PathBuf> {
        self.hovering.clear();
        std::mem::take(&mut self.dropped)
    }

    pub fn active(&self) -> bool {
        !self.hovering.is_empty()
    }
}

/// The pointer over `window`, physical pixels, asked of X; None on Wayland
/// (or where X will not say).
pub fn pointer(window: &Window) -> Option<(f64, f64)> {
    let RawDisplayHandle::Xlib(d) = window.display_handle().ok()?.as_raw() else { return None };
    let RawWindowHandle::Xlib(w) = window.window_handle().ok()?.as_raw() else { return None };
    let display = d.display?.as_ptr() as *mut x11_dl::xlib::Display;
    let xlib = xlib()?;
    let (mut root, mut child) = (0, 0);
    let (mut rx, mut ry, mut x, mut y) = (0, 0, 0, 0);
    let mut mask = 0;
    let found = unsafe { (xlib.XQueryPointer)(display, w.window, &mut root, &mut child, &mut rx, &mut ry, &mut x, &mut y, &mut mask) };
    (found != 0).then_some((x as f64, y as f64))
}

fn xlib() -> Option<&'static x11_dl::xlib::Xlib> {
    static XLIB: std::sync::OnceLock<Option<x11_dl::xlib::Xlib>> = std::sync::OnceLock::new();
    XLIB.get_or_init(|| x11_dl::xlib::Xlib::open().ok()).as_ref()
}

/// The clipboard's picture as PNG bytes (`width` by `height`, RGBA rows).
pub fn png(width: usize, height: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return None;
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::write_buffer_with_format(&mut out, &rgba[..width * height * 4], width as u32, height as u32, image::ExtendedColorType::Rgba8, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drags_files_are_gathered_as_they_come_and_taken_once() {
        let mut d = Drag::default();
        assert!(!d.active());
        assert!(d.hover("/a/One.md".into()), "the first file starts the drag");
        assert!(!d.hover("/a/Two.png".into()));
        assert!(!d.hover("/a/One.md".into()));
        assert_eq!(d.hovering.len(), 2);
        d.drop("/a/One.md".into());
        d.drop("/a/Two.png".into());
        assert_eq!(d.end(), [PathBuf::from("/a/One.md"), PathBuf::from("/a/Two.png")]);
        assert!(!d.active() && d.end().is_empty());
    }

    #[test]
    fn a_clipboard_picture_becomes_a_png() {
        let rgba = [255, 0, 0, 255, 0, 255, 0, 128];
        let bytes = png(2, 1, &rgba).expect("a PNG");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let back = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(back.as_raw(), &rgba);
        assert!(png(0, 1, &rgba).is_none() && png(4, 4, &rgba).is_none());
    }
}
