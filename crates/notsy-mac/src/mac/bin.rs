//! The Mac's Trash: a note removed for good from Notsy's own trash goes
//! there (never beyond), where the Finder can still put it back.

use std::path::Path;

use objc2_foundation::{NSFileManager, NSString, NSURL};

/// The Mac's Trash, for the model's `Bin` service.
pub struct MacBin;

impl notsy_app::services::Bin for MacBin {
    fn recycle(&self, path: &Path) -> bool {
        recycle(path)
    }
}

/// Moves the file at `path` to the Mac's Trash: whether it went.
pub fn recycle(path: &Path) -> bool {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error(&url, None).is_ok()
}
