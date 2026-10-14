//! The vault's PDFs and pictures in search, found by what they say, as on
//! Windows (`kolada-win` `convert.rs`, `index_files`): their text is read
//! once, after the notes are indexed and off the main thread at background
//! priority, and kept in the app's data (never the vault) until the file
//! changes. PDFs are read by the shared converter; pictures by Vision
//! (`vision.rs`), a few hundred a time. Nothing here runs before the first
//! frame: search's index starts after it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kolada_core::search::{self, File, Indexed, TextCache};

use super::*;
use crate::services::{Ocr, Read};

/// Pictures read in one go: a large photo library is worked through over
/// a few launches.
pub const PICTURES_AT_ONCE: usize = 200;

/// The cache is written this often while files are read, so quitting
/// half way keeps what was done.
const SAVE_EVERY: usize = 25;

/// How a file's text is read. `None` means it could not be read now (so it
/// is tried again next time); `Some("")` that it holds no text.
pub type Reader = dyn Fn(&File) -> Option<String> + Send + Sync;

/// Where the cache is kept and how files are read: the app's data and the
/// real readers, unless a test says otherwise.
#[derive(Clone, Default)]
pub struct Files {
    pub data: Option<PathBuf>,
    pub reader: Option<Arc<Reader>>,
}

/// A PDF's text, or a picture's words.
pub fn read(f: &File, ocr: &dyn Ocr) -> Option<String> {
    if search::is_picture(&f.rel) {
        return match ocr.read(&f.path) {
            Read::Words(w) => Some(w),
            Read::Missing => None,
        };
    }
    // A file that stops the reader (a panic) is kept as having no text, so
    // the next time goes on past it.
    let bytes = std::fs::read(&f.path).ok()?;
    let text = std::panic::catch_unwind(|| kolada_convert::convert(kolada_convert::Kind::Pdf, &bytes).map(|n| n.markdown).unwrap_or_default());
    Some(text.unwrap_or_default())
}

/// The vault's PDFs and pictures for search: by their names, and the text
/// read from them (a picture with no words stays out). `cache_in` is the
/// app's data folder.
pub fn index_files(vault: &Path, cache_in: Option<&Path>, read: &dyn Fn(&File) -> Option<String>) -> Vec<Indexed> {
    let files = search::files(vault);
    if files.is_empty() {
        return Vec::new();
    }
    let at = cache_in.map(|d| TextCache::path(d, vault));
    let mut cache = at.as_deref().map(TextCache::load).unwrap_or_default();
    let mut pictures = 0;
    let todo: Vec<&File> = files
        .iter()
        .filter(|f| cache.get(f).is_none())
        .filter(|f| !search::is_picture(&f.rel) || { pictures += 1; pictures <= PICTURES_AT_ONCE })
        .collect();
    let mut kept = false;
    for (k, f) in todo.iter().enumerate() {
        if let Some(text) = read(f) {
            cache.put(f, text);
            kept = true;
        }
        if (k + 1) % SAVE_EVERY == 0 && let Some(at) = &at {
            let _ = cache.save(at);
        }
    }
    cache.keep_only(&files);
    if let Some(at) = &at && (!todo.is_empty() || kept || at.exists()) {
        let _ = cache.save(at);
    }
    files
        .iter()
        .filter_map(|f| {
            let text = cache.get(f).unwrap_or_default();
            (!search::is_picture(&f.rel) || !text.trim().is_empty()).then(|| Indexed::new(f.rel.clone(), text.to_string()))
        })
        .collect()
}

/// Reads the files on a thread of its own at background priority, then adds
/// them to `slot` (the notes' index), if it is still there. Returns whether
/// the index changed.
pub fn add_to(slot: &Mutex<Option<Vec<Indexed>>>, vault: &Path, files: &Files, services: &Services) -> bool {
    services.priority.lower_thread();
    let data = files.data.clone().or_else(kolada_core::app_data_dir);
    let found = match &files.reader {
        Some(r) => index_files(vault, data.as_deref(), &**r),
        None => index_files(vault, data.as_deref(), &|f: &File| read(f, &*services.ocr)),
    };
    let mut slot = slot.lock().unwrap();
    let Some(all) = slot.as_mut() else { return false };
    // Files of an earlier read go; the notes stay.
    all.retain(|n| search::is_note(&n.path));
    let any = !found.is_empty();
    all.extend(found);
    any
}

/// A `file://` address for `path`, for the system to open.
pub fn file_url(path: &Path) -> String {
    // A Windows path (drive, colon, backslashes) is `file:///C:/Users/me/a.png`.
    let text = path.to_string_lossy();
    let drive = text.as_bytes().get(1) == Some(&b':') && text.as_bytes()[0].is_ascii_alphabetic();
    let text = if drive { text.replace('\\', "/") } else { text.into_owned() };
    let mut out = String::from(if drive { "file:///" } else { "file://" });
    for (i, b) in text.bytes().enumerate() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b':' if drive && i == 1 => out.push(':'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Model {
    /// The index took in the vault's files: search looks again.
    pub fn index_arrived(&mut self) -> bool {
        // Recent shows only in a vault of more than a few dozen notes:
        // counted here, kept in the session so the next first frame knows.
        let notes = self.index.lock().ok().and_then(|i| i.as_ref().map(Vec::len));
        let counted = notes.is_some_and(|n| self.session.counted(n));
        if counted {
            self.persist_session();
        }
        let looked = match &mut self.search {
            Some(s) => {
                s.fresh = false;
                true
            }
            None => false,
        };
        counted || looked
    }

    /// A result that is not a note (a PDF, a picture) opens in the app the
    /// Mac has for it: the view is asked to, as for a link.
    pub(super) fn open_file_hit(&mut self, rel: &str) -> bool {
        let url = file_url(&self.vault.join(rel));
        self.search = None;
        self.menus.effects.push(share::Effect::Url(url));
        true
    }

    #[cfg(test)]
    pub(super) fn read_files_with(&mut self, data: PathBuf, reader: impl Fn(&File) -> Option<String> + Send + Sync + 'static) {
        self.files = Files { data: Some(data), reader: Some(Arc::new(reader)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn dir(test: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kolada-app-files-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn big() -> Vec<u8> {
        vec![7; 20 * 1024]
    }

    /// What a fake reader says of each file, by name: a PDF's text, a
    /// picture's words.
    fn said(f: &File) -> Option<String> {
        Some(match f.rel.as_str() {
            "Scans/lease.pdf" => "The tenant pays rent monthly".to_string(),
            "receipt.png" => "Total 42.00 Hardware store".to_string(),
            _ => String::new(),
        })
    }

    fn vault(test: &str) -> PathBuf {
        let v = dir(test);
        std::fs::create_dir_all(v.join("Scans")).unwrap();
        std::fs::write(v.join("Scans/lease.pdf"), b"%PDF-fake").unwrap();
        std::fs::write(v.join("receipt.png"), big()).unwrap();
        std::fs::write(v.join("blank.png"), big()).unwrap();
        std::fs::write(v.join("icon.png"), b"tiny").unwrap();
        std::fs::write(v.join("Plan.md"), "# Plan\n").unwrap();
        v
    }

    #[test]
    fn pdfs_and_pictures_are_found_by_what_they_say_and_each_is_read_once() {
        let (v, data) = (vault("once"), dir("once-data"));
        let reads = AtomicUsize::new(0);
        let reader = |f: &File| {
            reads.fetch_add(1, Ordering::SeqCst);
            said(f)
        };
        let found = index_files(&v, Some(&data), &reader);
        let names: Vec<&str> = found.iter().map(|n| n.path.as_str()).collect();
        // The PDF by name and text, the picture with words; the blank picture
        // is out, and the small one was never read.
        assert_eq!(names, ["Scans/lease.pdf", "receipt.png"]);
        assert_eq!(reads.load(Ordering::SeqCst), 3);
        let hits = search::search(&found, "rent monthly", 10);
        assert_eq!(hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(), ["Scans/lease.pdf"]);
        let hits = search::search(&found, "hardware", 10);
        assert_eq!(hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(), ["receipt.png"]);
        // Again: kept in the app's data, nothing is read.
        let again = index_files(&v, Some(&data), &reader);
        assert_eq!(again.len(), 2);
        assert_eq!(reads.load(Ordering::SeqCst), 3);
        // A changed file is read again, the others not.
        std::fs::write(v.join("Scans/lease.pdf"), b"%PDF-fake, longer now").unwrap();
        index_files(&v, Some(&data), &reader);
        assert_eq!(reads.load(Ordering::SeqCst), 4);
        let _ = std::fs::remove_dir_all(&v);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_picture_that_could_not_be_read_is_tried_again_and_a_blank_one_is_not() {
        let (v, data) = (vault("again"), dir("again-data"));
        let reads = AtomicUsize::new(0);
        let off = |_: &File| {
            reads.fetch_add(1, Ordering::SeqCst);
            None
        };
        assert!(index_files(&v, Some(&data), &off).iter().all(|n| n.path.ends_with(".pdf")));
        let first = reads.load(Ordering::SeqCst);
        index_files(&v, Some(&data), &off);
        assert_eq!(reads.load(Ordering::SeqCst), first * 2, "nothing learned, so asked again");
        let _ = std::fs::remove_dir_all(&v);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn only_so_many_pictures_are_read_at_a_time() {
        let (v, data) = (dir("many"), dir("many-data"));
        for i in 0..PICTURES_AT_ONCE + 5 {
            std::fs::write(v.join(format!("p{i}.png")), big()).unwrap();
        }
        let reads = AtomicUsize::new(0);
        let reader = |_: &File| {
            reads.fetch_add(1, Ordering::SeqCst);
            Some("words".to_string())
        };
        assert_eq!(index_files(&v, Some(&data), &reader).len(), PICTURES_AT_ONCE);
        assert_eq!(index_files(&v, Some(&data), &reader).len(), PICTURES_AT_ONCE + 5, "the rest the next time");
        assert_eq!(reads.load(Ordering::SeqCst), PICTURES_AT_ONCE + 5);
        let _ = std::fs::remove_dir_all(&v);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_pdf_that_will_not_read_is_kept_as_having_no_text() {
        let v = dir("bad");
        std::fs::write(v.join("broken.pdf"), b"not a pdf at all").unwrap();
        let f = &search::files(&v)[0];
        assert_eq!(read(f, &crate::services::NoServices), Some(String::new()));
        let _ = std::fs::remove_dir_all(&v);
    }

    #[test]
    fn file_addresses_are_escaped() {
        assert_eq!(file_url(Path::new("/Users/a b/Notes/\u{e9}.pdf")), "file:///Users/a%20b/Notes/%C3%A9.pdf");
    }

    #[test]
    fn search_finds_a_pdf_and_a_picture_once_the_files_are_read_and_return_opens_them_outside() {
        let (v, data) = (vault("model"), dir("model-data"));
        let mut m = Model::new(v.clone(), true);
        m.frame(2000, 1440, 2.0);
        m.read_files_with(data.clone(), said);
        assert!(m.open_search());
        // The notes' index and then the files' arrive on a thread.
        let ready = |m: &Model| m.index.lock().unwrap().as_ref().is_some_and(|all| all.iter().any(|n| n.path == "receipt.png"));
        for _ in 0..200 {
            if ready(&m) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(ready(&m), "the files join the index");
        assert!(m.index_arrived());
        for c in "hardware".chars() {
            m.insert(&c.to_string());
        }
        m.frame(2000, 1440, 2.0);
        assert_eq!(m.search.as_ref().unwrap().hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(), ["receipt.png"]);
        assert!(m.newline(false));
        assert!(m.search.is_none());
        let sent = m.take_effects();
        let [share::Effect::Url(url)] = sent.as_slice() else { panic!("{sent:?}") };
        assert!(url.starts_with("file://") && url.ends_with("/receipt.png"), "{url}");
        let _ = std::fs::remove_dir_all(&v);
        let _ = std::fs::remove_dir_all(&data);
    }
}
