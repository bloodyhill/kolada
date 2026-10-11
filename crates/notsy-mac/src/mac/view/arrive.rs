//! What comes into the window from outside: a screenshot on the pasteboard,
//! and files dragged onto the view (NSDraggingDestination). Thin AppKit calls
//! around the model (`model/arrive.rs`, `model/pictures.rs`), where what is
//! done with them is decided and tested.

use std::path::PathBuf;

use objc2::DefinedClass;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSDragOperation, NSDraggingInfo, NSPasteboard};
use objc2_foundation::{NSArray, NSDictionary, NSString};

use super::NotsyView;
use crate::mac::model::arrive::{file_url_path, sorted};

/// The pasteboard's picture as PNG bytes, if it holds one and no text: a
/// screenshot (`public.png`) or a copy from Preview or a browser
/// (`public.tiff`, made a PNG).
pub(super) fn picture_on_pasteboard(pasteboard: &NSPasteboard) -> Option<Vec<u8>> {
    if let Some(png) = pasteboard.dataForType(&NSString::from_str("public.png")) {
        return Some(png.to_vec());
    }
    let tiff = pasteboard.dataForType(&NSString::from_str("public.tiff"))?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(png.to_vec())
}

/// The files a drag carries, as paths (each item's `public.file-url`).
fn dragged_files(pasteboard: &NSPasteboard) -> Vec<PathBuf> {
    let Some(items) = pasteboard.pasteboardItems() else { return Vec::new() };
    let kind = NSString::from_str("public.file-url");
    items.iter().filter_map(|item| item.stringForType(&kind)).filter_map(|url| path_of_file_url(&url.to_string())).collect()
}

/// A file URL's path. The Finder drags file reference URLs
/// (`file:///.file/id=...`), which only the OS can turn into the file's
/// path (seen on the real screen: a note dragged from the Finder was "a
/// file Notsy can't read", named id=...).
fn path_of_file_url(url: &str) -> Option<PathBuf> {
    if url.starts_with("file:///.file/") {
        let reference = objc2_foundation::NSURL::URLWithString(&NSString::from_str(url))?;
        return reference.filePathURL()?.path().map(|p| PathBuf::from(p.to_string()));
    }
    file_url_path(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_dragged_from_the_finder_is_known_by_its_path() {
        let file = std::env::temp_dir().join(format!("notsy-mac-drag-{}.md", std::process::id()));
        std::fs::write(&file, "# Dragged\n").unwrap();
        let url = objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
        let reference = url.fileReferenceURL().expect("a reference URL").absoluteString().unwrap().to_string();
        assert!(reference.starts_with("file:///.file/id="), "{reference}");
        let path = path_of_file_url(&reference).expect("resolved");
        assert_eq!(path.canonicalize().unwrap(), file.canonicalize().unwrap());
        assert_eq!(path_of_file_url("file:///tmp/a%20b.md"), Some(PathBuf::from("/tmp/a b.md")), "a plain file URL as before");
        let _ = std::fs::remove_file(file);
    }
}

impl NotsyView {
    /// Takes files dragged onto the window: the view asks for them.
    pub(super) fn accept_drags(&self) {
        let kinds = NSArray::from_retained_slice(&[NSString::from_str("public.file-url")]);
        self.registerForDraggedTypes(&kinds);
    }

    /// A drag comes over the window: the card that says what its files
    /// become follows the pointer; a copy if anything here is read.
    pub(super) fn drag_enter(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        let files = dragged_files(&info.draggingPasteboard());
        let p = self.convertPoint_fromView(info.draggingLocation(), None);
        let takes = self.ivars().state.borrow_mut().model.drag_enter(&files, p.x as f32, p.y as f32);
        self.redraw_if(true);
        if takes { NSDragOperation::Copy } else { NSDragOperation::None }
    }

    /// Over the window with a drag: the card follows, the gap moves.
    pub(super) fn drag_over(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        let p = self.convertPoint_fromView(info.draggingLocation(), None);
        let takes = {
            let mut st = self.ivars().state.borrow_mut();
            let files = dragged_files(&info.draggingPasteboard());
            let (pictures, notes) = sorted(&files);
            st.model.drag_over(p.x as f32, p.y as f32) || pictures + notes > 0
        };
        self.redraw_if(true);
        if takes { NSDragOperation::Copy } else { NSDragOperation::None }
    }

    /// The drag left the window: the card fades.
    pub(super) fn drag_exit(&self) {
        let changed = self.ivars().state.borrow_mut().model.drag_leave();
        self.redraw_if(changed);
    }

    /// The drag was let go: its files come in where it was let go.
    pub(super) fn take_drop(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
        let files = dragged_files(&info.draggingPasteboard());
        if files.is_empty() {
            return false;
        }
        let p = self.convertPoint_fromView(info.draggingLocation(), None);
        let changed = {
            let mut st = self.ivars().state.borrow_mut();
            // Pictures land where the gap opened; else under the pointer.
            match st.model.drag_drop() {
                Some(end) => st.model.drop_files_after(files, end),
                None => st.model.drop_files(files, Some((p.x as f32, p.y as f32))),
            }
        };
        self.redraw_if(true);
        self.edited(changed);
        true
    }
}
