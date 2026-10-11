//! What comes into the note from outside, beyond the keyboard: files dropped
//! on the window (pictures are saved with the attachments and embedded;
//! Markdown, Word, PDF and web page files become notes), and what a paste
//! brought from the web, fetched after it. Documents are converted and
//! pictures fetched on threads of their own; each answers through a channel
//! the shell polls when `wake` says so (the same way the pictures' worker and
//! the vault's watcher do). Reference: notsy-win app/pictures.rs drop_files,
//! app/share.rs import_documents, convert.rs one, app/paste.rs picture_fetched.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use notsy_core::{note, vault};
use notsy_ui::images;

use super::Model;
use super::pictures::{free_name, is_picture, name_of};

/// The most a pasted picture from the web may be, bytes.
const MOST: usize = 25 * 1024 * 1024;

/// What a thread brought back.
pub enum Arrival {
    /// A pasted picture from the web: where it was to go, where it was, the
    /// note it was pasted into, and whether it came.
    Fetched { path: PathBuf, url: String, note: Option<PathBuf>, ok: bool },
    /// A first run's welcome note, written into the vault it made.
    Welcome(PathBuf),
    /// Documents read into notes.
    Documents(Converted),
}

/// What converting documents found: the notes written, and the files that
/// could not be read (their names, and why).
#[derive(Debug, Default)]
pub struct Converted {
    pub notes: Vec<PathBuf>,
    pub failed: Vec<(String, String)>,
}

/// The channel back from the threads, and how to wake the main one.
pub struct Arrivals {
    tx: Sender<Arrival>,
    rx: Receiver<Arrival>,
    wake: Option<Arc<Mutex<Box<dyn Fn() + Send>>>>,
}

impl Default for Arrivals {
    fn default() -> Arrivals {
        let (tx, rx) = channel();
        Arrivals { tx, rx, wake: None }
    }
}

impl Arrivals {
    /// A way for a thread to send, and then wake the main thread.
    fn sender(&self) -> impl Fn(Arrival) + Send + 'static + use<> {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        move |a| {
            let _ = tx.send(a);
            if let Some(w) = &wake {
                if let Ok(w) = w.lock() {
                    w();
                }
            }
        }
    }
}

/// Whether `path` is a note Notsy takes as it is.
pub fn is_note_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| ["md", "markdown", "txt"].contains(&e.to_string_lossy().to_lowercase().as_str()))
}

/// Whether `path` is a document that converts into a note (DOCX, PDF,
/// HTML), or another app's export read into many (Evernote's .enex, a
/// Notion export .zip).
pub fn is_document(path: &Path) -> bool {
    notsy_convert::Kind::of(path).is_some() || notsy_convert::import::Source::of(path).is_some() || is_nbk(path)
}

/// Whether `path` is an export of the Notes app (`.nbk`), read into many
/// dated notes.
pub fn is_nbk(path: &Path) -> bool {
    path.extension().is_some_and(|x| x.eq_ignore_ascii_case("nbk"))
}

/// `times` with the file's creation date set, where the OS lets a program
/// set it (macOS and Windows); elsewhere unchanged.
fn with_created(times: std::fs::FileTimes, created: std::time::SystemTime) -> std::fs::FileTimes {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::FileTimesExt;
        return times.set_created(created);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTimesExt;
        return times.set_created(created);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = created;
        times
    }
}

/// Writes the notes of the Notes export at `path` as Markdown into a new
/// folder of `dir` named after it, in their folders, each file dated when
/// its note was made and last changed, so they sort as they did. As
/// `notsy-win` `app/share.rs` `write_export`; the notes, newest first.
pub fn write_nbk(path: &Path, dir: &Path) -> Result<Vec<PathBuf>, String> {
    use std::io::Write as _;
    let bytes = std::fs::read(path).map_err(|e| said(&e))?;
    let export = notsy_core::nbk::read(&bytes).map_err(|e| e.to_string())?;
    let name = note::file_stem_for(&path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let top = (1..).map(|n| if n == 1 { dir.join(&name) } else { dir.join(format!("{name} {n}")) }).find(|p| !p.exists()).expect("a free name");
    let mut written: Vec<(i64, PathBuf)> = Vec::new();
    let mut taken = std::collections::HashSet::new();
    for n in &export.notes {
        let folder = n.folders.iter().fold(top.clone(), |p, f| p.join(note::file_stem_for(f)));
        if std::fs::create_dir_all(&folder).is_err() {
            continue;
        }
        let stem = note::file_stem_for(&notsy_editor::title_of(&n.text));
        let to = (1..)
            .map(|k| folder.join(if k == 1 { format!("{stem}.md") } else { format!("{stem} {k}.md") }))
            .find(|p| !taken.contains(&p.to_string_lossy().to_lowercase()))
            .expect("a free name");
        taken.insert(to.to_string_lossy().to_lowercase());
        let Ok(mut file) = std::fs::File::create_new(&to) else { continue };
        if file.write_all(n.text.as_bytes()).is_err() {
            continue;
        }
        let at = |ms: i64| (ms > 0).then(|| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms as u64));
        let mut times = std::fs::FileTimes::new();
        if let Some(m) = at(n.modified) {
            times = times.set_modified(m).set_accessed(m);
        }
        if let Some(c) = at(n.created) {
            times = with_created(times, c);
        }
        let _ = file.set_times(times);
        written.push((n.modified, to));
    }
    if written.is_empty() {
        return Err("that export has no notes".to_string());
    }
    written.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(written.into_iter().map(|(_, p)| p).collect())
}

/// What the shell can say a drag becomes: how many of `files` are pictures
/// (into this note) and how many become notes.
pub fn sorted(files: &[PathBuf]) -> (usize, usize) {
    let pictures = files.iter().filter(|f| images::is_image(&f.to_string_lossy())).count();
    (pictures, files.iter().filter(|f| is_document(f) || is_note_file(f)).count())
}

/// The path a `file://` URL names (percent escapes decoded), as the
/// pasteboard gives dragged files.
pub fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.trim().strip_prefix("file://")?;
    // `file://localhost/Users/...` and `file:///Users/...`.
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%' && i + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        if let Some(b) = escaped {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
    }
    #[cfg(not(unix))]
    {
        Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
    }
}

impl Model {
    /// Starts the arrivals' wake (after the first frame): `wake` is called
    /// from another thread when something is ready for `poll_arrivals`.
    pub fn start_arrivals(&mut self, wake: impl Fn() + Send + 'static) {
        self.arrivals.wake = Some(Arc::new(Mutex::new(Box::new(wake))));
        self.seed_welcome();
    }

    /// A first run (no session yet, the default vault, not made yet): the
    /// welcome note and its picture are written on a thread, after the
    /// first frame, and open if writing has not begun on the fresh page.
    fn seed_welcome(&mut self) {
        if !self.first_run {
            return;
        }
        self.first_run = false;
        let (send, vault) = (self.arrivals.sender(), self.vault.clone());
        let modifier = if matches!(self.platform, notsy_ui::scene::Platform::Mac | notsy_ui::scene::Platform::Ios) { "\u{2318}" } else { "Ctrl" };
        std::thread::spawn(move || {
            if let Ok(Some(note)) = notsy_core::welcome::seed(&vault, modifier) {
                send(Arrival::Welcome(note));
            }
        });
    }

    /// The welcome note landed: it opens on a page nothing was written on,
    /// else it is only listed.
    fn welcome(&mut self, note: PathBuf) -> bool {
        let fresh = self.session.note.is_empty() && self.editor.text().is_empty();
        match vault::relative(&self.vault, &note) {
            Some(rel) if fresh => self.open(&rel),
            _ => self.refresh_tree(),
        }
        true
    }

    /// Takes what the threads brought back; whether the screen changes.
    pub fn poll_arrivals(&mut self) -> bool {
        let mut any = false;
        while let Ok(a) = self.arrivals.rx.try_recv() {
            any |= match a {
                Arrival::Fetched { path, url, note, ok } => self.picture_fetched(path, &url, note, ok),
                Arrival::Documents(done) => self.documents_in(done),
                Arrival::Welcome(note) => self.welcome(note),
            };
        }
        any
    }

    // Pictures pasted from the web.

    /// Fetches `fetch` (the file each goes to, and its address), one after
    /// another, on a thread of its own.
    pub(super) fn fetch_pictures(&mut self, fetch: Vec<(PathBuf, String)>) {
        let web = self.services.web.clone();
        self.fetch_pictures_with(fetch, move |url| web.get(url, MOST));
    }

    pub(super) fn fetch_pictures_with(&mut self, fetch: Vec<(PathBuf, String)>, get: impl Fn(&str) -> Option<Vec<u8>> + Send + 'static) {
        if fetch.is_empty() {
            return;
        }
        let (send, note) = (self.arrivals.sender(), self.path.clone());
        std::thread::spawn(move || {
            for (path, url) in fetch {
                let ok = get(&url).filter(|b| is_picture(b)).is_some_and(|b| std::fs::write(&path, b).is_ok());
                send(Arrival::Fetched { path, url, note: note.clone(), ok });
            }
        });
    }

    /// A pasted picture came from the web, and is drawn; or it could not be
    /// had, and its embed becomes a link to where it is, in the open note or,
    /// if that has been left, in its file.
    fn picture_fetched(&mut self, path: PathBuf, url: &str, note: Option<PathBuf>, ok: bool) -> bool {
        self.pictures.borrow_mut().forget_places();
        if ok {
            return true;
        }
        let _ = std::fs::remove_file(&path);
        let (embed, link) = (format!("![[{}]]", name_of(&path)), format!("![]({})", url.replace(' ', "%20")));
        if let Some(at) = self.editor.text().find(&embed) {
            self.edit(|e| e.replace_ranges(&[(at, at + embed.len(), link)]));
        } else if let Some(file) = note.filter(|n| Some(n) != self.path.as_ref()) {
            if let Ok((text, format)) = note::load(&file) {
                if text.contains(&embed) {
                    let _ = note::save(&file, &text.replacen(&embed, &link, 1), format);
                }
            }
        }
        true
    }

    // Files dropped on the window.

    /// Files dropped at (x, y) DIPs (None: at the caret): pictures are saved
    /// with the attachments and embedded there; notes are copied into the
    /// vault (one already in it just opens); documents are made notes on a
    /// thread of their own. Whether the screen changes.
    /// `drop_files`, pictures landing after the line ending at `end` (where
    /// the gap opened as they were dragged over).
    pub fn drop_files_after(&mut self, files: Vec<PathBuf>, end: usize) -> bool {
        let end = end.min(self.editor.text().len());
        self.editor.set_selection(notsy_editor::Selection::caret(end));
        self.drop_files(files, None)
    }

    pub fn drop_files(&mut self, files: Vec<PathBuf>, at: Option<(f32, f32)>) -> bool {
        let mut changed = false;
        let (pictures, rest): (Vec<PathBuf>, Vec<PathBuf>) = files.into_iter().partition(|f| images::is_image(&f.to_string_lossy()));
        if !pictures.is_empty() && !self.in_field() && !self.scrubbing() && !self.settings.read_only {
            if let Some(dir) = self.attachment_dir() {
                let mut names = Vec::new();
                for f in &pictures {
                    // Already in the vault: embedded where it is.
                    let path = if f.starts_with(&self.vault) {
                        f.clone()
                    } else {
                        let to = free_name(&dir, &name_of(f));
                        if std::fs::copy(f, &to).is_err() {
                            continue;
                        }
                        to
                    };
                    names.push(name_of(&path));
                }
                let head = match at {
                    Some((x, y)) => self.offset_at(x, y),
                    None => self.editor.selection().head,
                };
                changed |= self.embed_pictures(&names, head);
            }
        }
        // Documents (DOCX, PDF, HTML): each made a note.
        let (documents, rest): (Vec<PathBuf>, Vec<PathBuf>) = rest.into_iter().partition(|f| is_document(f));
        if !documents.is_empty() {
            self.import_documents(documents);
        }
        let notes: Vec<PathBuf> = rest.into_iter().filter(|f| is_note_file(f)).collect();
        changed | self.import_notes(&notes)
    }

    /// Notes copied into the vault; the first opens. A single note of the
    /// vault just opens.
    pub(super) fn import_notes(&mut self, notes: &[PathBuf]) -> bool {
        if notes.is_empty() {
            return false;
        }
        if let [only] = notes {
            if only.starts_with(&self.vault) {
                return match vault::relative(&self.vault, only) {
                    Some(rel) => {
                        self.open(&rel);
                        true
                    }
                    None => false,
                };
            }
        }
        let mut first = None;
        for f in notes.iter().filter(|f| !f.starts_with(&self.vault)) {
            let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let to = note::unique_path(&self.vault, &stem, None);
            if std::fs::copy(f, &to).is_ok() {
                first.get_or_insert(to);
            }
        }
        self.imported(first)
    }

    /// New notes are in the vault: the tree shows them and the first opens.
    fn imported(&mut self, first: Option<PathBuf>) -> bool {
        let Some(first) = first else { return false };
        if let Some(rel) = vault::relative(&self.vault, &first) {
            self.open(&rel);
        }
        self.refresh_tree();
        self.pictures.borrow_mut().forget_places();
        true
    }

    /// Documents made notes at the vault's top, their pictures with the
    /// attachments, on a thread of their own.
    pub(super) fn import_documents(&mut self, files: Vec<PathBuf>) {
        let (vault, pictures) = (self.vault.clone(), self.attachment_path());
        let send = self.arrivals.sender();
        std::thread::spawn(move || send(Arrival::Documents(convert_documents(&files, &vault, &pictures))));
    }

    /// Documents converted: the tree shows their notes and the first opens.
    fn documents_in(&mut self, done: Converted) -> bool {
        self.imported(done.notes.first().cloned())
    }

    /// Waits for what the threads bring, for tests.
    #[cfg(test)]
    pub fn wait_for_arrivals(&mut self) {
        for _ in 0..200 {
            if self.poll_arrivals() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("nothing arrived");
    }
}

/// Converts `files` into notes in `dir`, their pictures into `pictures`.
pub fn convert_documents(files: &[PathBuf], dir: &Path, pictures: &Path) -> Converted {
    let mut done = Converted::default();
    for f in files {
        let made = match notsy_convert::import::Source::of(f) {
            _ if is_nbk(f) => write_nbk(f, dir),
            Some(source) => import(f, source, dir, pictures),
            None => convert_document(f, dir, pictures).map(|p| vec![p]),
        };
        match made {
            Ok(paths) => done.notes.extend(paths),
            Err(why) => done.failed.push((name_of(f), why)),
        }
    }
    done
}

/// Another app's export read into notes, in a folder of `dir` named after
/// it (a Notion `Export-<id>` as "Notion"), their attachments into
/// `pictures`; the notes' paths. As `notsy-win` `convert.rs` `import`.
fn import(file: &Path, source: notsy_convert::import::Source, dir: &Path, pictures: &Path) -> Result<Vec<PathBuf>, String> {
    let bytes = std::fs::read(file).map_err(|e| said(&e))?;
    let export = notsy_convert::import::read(source, &bytes).map_err(|e| e.to_string())?;
    if export.notes.is_empty() {
        return Err("it has no notes".to_string());
    }
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = if source == notsy_convert::import::Source::Notion && (stem.starts_with("Export-") || stem.is_empty()) { "Notion".to_string() } else { stem };
    notsy_convert::import::write(&export, &name, dir, pictures).map_err(|e| e.to_string())
}

/// A file error in words a person would use.
fn said(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "it is not there any more".to_string(),
        std::io::ErrorKind::PermissionDenied => "macOS did not let Notsy open it".to_string(),
        _ => e.to_string(),
    }
}

/// Converts one document into a note in `dir`; its path.
fn convert_document(file: &Path, dir: &Path, pictures: &Path) -> Result<PathBuf, String> {
    let kind = notsy_convert::Kind::of(file).ok_or("not a document Notsy reads")?;
    let bytes = std::fs::read(file).map_err(|e| said(&e))?;
    let converted = notsy_convert::convert(kind, &bytes).map_err(|e| e.to_string())?;
    let mut text = converted.markdown;
    // Pictures inside the document, saved beside the vault's others; the
    // note names them as saved.
    let mut saved = Vec::new();
    for p in &converted.pictures {
        let _ = std::fs::create_dir_all(pictures);
        let to = free_name(pictures, &p.name);
        if std::fs::write(&to, &p.data).is_ok() {
            saved.push((p.name.clone(), name_of(&to)));
        }
    }
    // Pictures beside an HTML page, copied in; ones not there, left out.
    let from_dir = file.parent().unwrap_or(Path::new("."));
    let mut missing = Vec::new();
    for (name, rel) in &converted.beside {
        let src = from_dir.join(rel);
        if src.is_file() {
            let _ = std::fs::create_dir_all(pictures);
            let to = free_name(pictures, name);
            if std::fs::copy(&src, &to).is_ok() {
                saved.push((name.clone(), name_of(&to)));
                continue;
            }
        }
        missing.push(name.clone());
    }
    for (was, now) in &saved {
        if was != now {
            text = text.replace(&format!("![[{was}]]"), &format!("![[{now}]]"));
        }
    }
    if !missing.is_empty() {
        let gone: Vec<String> = missing.iter().map(|n| format!("![[{n}]]")).collect();
        let kept: Vec<&str> = text.lines().filter(|l| !gone.iter().any(|g| l.trim() == g)).collect();
        // Blank lines left where they were, one.
        let mut joined = String::new();
        for line in kept {
            if line.is_empty() && (joined.is_empty() || joined.ends_with("\n\n")) {
                continue;
            }
            joined.push_str(line);
            joined.push('\n');
        }
        text = format!("{}\n", joined.trim_end());
    }
    // The note's first line is its title: the document's own heading, or
    // else the title it gives itself, or its file's name.
    if !text.trim_start().starts_with('#') {
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let title = converted.title.filter(|t| !t.trim().is_empty()).unwrap_or(stem);
        text = format!("# {}\n\n{}", title.trim(), text.trim_start());
    }
    let stem = note::file_stem_for(&notsy_editor::title_of(&text));
    std::fs::create_dir_all(dir).map_err(|e| said(&e))?;
    let to = note::unique_path(dir, &stem, None);
    std::fs::write(&to, &text).map_err(|e| said(&e))?;
    // Dated as the document was, so notes sort as their sources did.
    if let Ok(modified) = std::fs::metadata(file).and_then(|m| m.modified()) {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&to) {
            let _ = f.set_times(std::fs::FileTimes::new().set_modified(modified).set_accessed(modified));
        }
    }
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use notsy_editor::Selection;

    fn dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("notsy-app-arrive-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn model(vault: &Path, note: &str) -> Model {
        std::fs::write(vault.join("Note.md"), note).unwrap();
        let mut m = Model::new(vault.to_path_buf(), true);
        m.frame(2000, 1440, 2.0);
        m
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nxxxx";

    #[test]
    fn dragged_files_are_read_from_their_urls() {
        assert_eq!(file_url_path("file:///Users/me/My%20Notes/a%C3%A9.md"), Some(PathBuf::from("/Users/me/My Notes/a\u{e9}.md")));
        assert_eq!(file_url_path("file://localhost/tmp/x.png"), Some(PathBuf::from("/tmp/x.png")));
        assert_eq!(file_url_path("https://example.com/x.png"), None);
        assert_eq!(file_url_path("file:///100%"), Some(PathBuf::from("/100%")));
    }

    #[test]
    fn a_drag_is_sorted_into_pictures_and_notes() {
        let files: Vec<PathBuf> = ["a.png", "b.JPG", "c.docx", "d.pdf", "e.md", "f.exe"].iter().map(PathBuf::from).collect();
        assert_eq!(sorted(&files), (2, 3));
        // Other apps' exports become notes too (a .zip is read as Notion's).
        let exports: Vec<PathBuf> = ["w.enex", "x.zip", "y.nbk"].iter().map(PathBuf::from).collect();
        assert_eq!(sorted(&exports), (0, 3));
    }

    #[test]
    fn exports_of_notes_evernote_and_notes_app_become_folders_of_notes() {
        let from = dir("exports-from");
        let v = dir("exports");
        std::fs::write(from.join("From phone.nbk"), include_bytes!("../../../notsy-core/testdata/notes.nbk")).unwrap();
        let enex = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<en-export><note><title>Shopping</title><content><![CDATA[<en-note><div>Milk</div><div>\u{062e}\u{0628}\u{0632}</div></en-note>]]></content><created>20201231T235959Z</created></note></en-export>";
        std::fs::write(from.join("Work.enex"), enex).unwrap();
        assert!(is_document(&from.join("Work.enex")) && is_document(&from.join("From phone.nbk")));
        let done = convert_documents(&[from.join("From phone.nbk"), from.join("Work.enex")], &v, &v);
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert_eq!(done.notes.len(), 7, "six from the Notes export, one from Evernote: {:?}", done.notes);
        assert!(v.join("From phone").join("Work").join("What now.md").is_file());
        let meta = std::fs::metadata(v.join("From phone").join("Groceries.md")).unwrap();
        let ms = |t: std::time::SystemTime| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        assert_eq!(ms(meta.modified().unwrap()), 1_665_419_890_626 + 86_400_000, "dated as it was");
        assert_eq!(ms(meta.created().unwrap()), 1_665_419_890_626);
        let shopping = std::fs::read_to_string(v.join("Work").join("Shopping.md")).unwrap();
        assert!(shopping.contains("Milk") && shopping.contains("\u{062e}\u{0628}\u{0632}"), "{shopping:?}");
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }

    #[test]
    fn a_dropped_picture_is_saved_with_the_attachments_and_embedded_after_the_line() {
        let v = dir("dropped");
        let from = dir("dropped-from");
        std::fs::write(from.join("cat.png"), PNG).unwrap();
        let mut m = model(&v, "# Pets\n\nA cat.\n");
        let at = m.text().find("A cat").unwrap();
        m.editor.set_selection(Selection::caret(at));
        assert!(m.drop_files(vec![from.join("cat.png")], None));
        assert_eq!(std::fs::read(v.join("cat.png")).unwrap(), PNG, "the vault's top is the default attachments folder");
        assert!(m.text().contains("A cat.\n![[cat.png]]\n"), "{:?}", m.text());
        // A second with the same name does not overwrite the first.
        assert!(m.drop_files(vec![from.join("cat.png")], None));
        assert!(v.join("cat 1.png").is_file() && m.text().contains("![[cat 1.png]]"));
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }

    #[test]
    fn pictures_follow_the_vaults_attachment_setting() {
        let v = dir("setting");
        let from = dir("setting-from");
        std::fs::create_dir_all(v.join(".obsidian")).unwrap();
        std::fs::write(v.join(".obsidian").join("app.json"), r#"{"attachmentFolderPath":"Files/Pictures"}"#).unwrap();
        std::fs::write(from.join("a.png"), PNG).unwrap();
        let mut m = model(&v, "# T\n");
        m.drop_files(vec![from.join("a.png")], None);
        assert!(v.join("Files").join("Pictures").join("a.png").is_file());
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }

    #[test]
    fn a_dropped_picture_is_one_undo_step() {
        let v = dir("undo");
        let from = dir("undo-from");
        std::fs::write(from.join("a.png"), PNG).unwrap();
        let mut m = model(&v, "# T\n");
        let before = m.text().to_string();
        m.drop_files(vec![from.join("a.png")], None);
        assert!(m.text().contains("![[a.png]]"));
        assert!(m.undo());
        assert_eq!(m.text(), before);
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }

    #[test]
    fn dropped_notes_are_copied_into_the_vault_and_the_first_opens() {
        let v = dir("notes");
        let from = dir("notes-from");
        std::fs::write(from.join("Ideas.md"), "# Ideas\n\nOne.\n").unwrap();
        std::fs::write(from.join("Plain.txt"), "Plain words.\n").unwrap();
        std::fs::write(from.join("skip.zip"), "x").unwrap();
        let mut m = model(&v, "# Old\n");
        assert!(m.drop_files(vec![from.join("Ideas.md"), from.join("Plain.txt"), from.join("skip.zip")], None));
        assert!(v.join("Ideas.md").is_file() && v.join("Plain.md").is_file(), "a text file is kept as a note");
        assert!(!v.join("skip.zip").exists());
        assert_eq!(m.session.note, "Ideas.md");
        // Dropping a note that is already in the vault opens it.
        assert!(m.drop_files(vec![v.join("Note.md")], None));
        assert_eq!(m.session.note, "Note.md");
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }

    #[test]
    fn a_dropped_web_page_becomes_a_note_on_a_thread_of_its_own() {
        let v = dir("html");
        let from = dir("html-from");
        std::fs::write(from.join("Page.html"), "<html><body><article><h1>Trip plan</h1><p>Pack light.</p></article></body></html>").unwrap();
        let mut m = model(&v, "# Old\n");
        m.drop_files(vec![from.join("Page.html")], None);
        m.wait_for_arrivals();
        assert_eq!(m.session.note, "Trip plan.md");
        assert!(m.text().starts_with("# Trip plan") && m.text().contains("Pack light."));
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }

    #[test]
    fn a_document_that_cannot_be_read_is_said_not_made() {
        let v = dir("bad");
        let from = dir("bad-from");
        std::fs::write(from.join("broken.docx"), "not a zip").unwrap();
        let done = convert_documents(&[from.join("broken.docx"), from.join("gone.pdf")], &v, &v);
        assert!(done.notes.is_empty());
        assert_eq!(done.failed.len(), 2);
        assert_eq!(done.failed[0].0, "broken.docx");
        assert_eq!(done.failed[1].1, "it is not there any more");
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }

    #[test]
    fn a_web_picture_is_fetched_after_the_paste_and_a_missing_one_becomes_a_link() {
        let v = dir("fetch");
        let mut m = model(&v, "# T\n");
        m.insert("![[Pasted image 1.png]]\n![[Pasted image 2.png]]\n");
        let (one, two) = (v.join("Pasted image 1.png"), v.join("Pasted image 2.png"));
        std::fs::write(&one, "").unwrap();
        std::fs::write(&two, "").unwrap();
        m.fetch_pictures_with(
            vec![(one.clone(), "https://x/one.png".into()), (two.clone(), "https://x/two.png".into())],
            |url| (url.ends_with("one.png")).then(|| PNG.to_vec()),
        );
        // Each answers on its own.
        while !m.text().contains("![](https://x/two.png)") {
            m.wait_for_arrivals();
        }
        assert_eq!(std::fs::read(&one).unwrap(), PNG);
        assert!(!two.exists(), "the file held for it is let go");
        assert!(m.text().contains("![[Pasted image 1.png]]") && !m.text().contains("![[Pasted image 2.png]]"));
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn an_error_page_in_a_pictures_place_is_not_kept() {
        let v = dir("errpage");
        let mut m = model(&v, "# T\n");
        m.insert("![[Pasted image 1.png]]\n");
        let one = v.join("Pasted image 1.png");
        std::fs::write(&one, "").unwrap();
        m.fetch_pictures_with(vec![(one.clone(), "https://x/one.png".into())], |_| Some(b"<html>404</html>".to_vec()));
        m.wait_for_arrivals();
        assert!(!one.exists());
        assert!(m.text().contains("![](https://x/one.png)"));
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn a_fetch_that_fails_after_the_note_was_left_changes_its_file() {
        let v = dir("left");
        let mut m = model(&v, "# T\n");
        m.insert("![[Pasted image 1.png]]\n");
        m.save_flush();
        let file = m.path.clone().expect("the note is a file");
        let one = v.join("Pasted image 1.png");
        std::fs::write(&one, "").unwrap();
        m.fetch_pictures_with(vec![(one.clone(), "https://x/one.png".into())], |_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            None
        });
        m.new_note();
        m.wait_for_arrivals();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("![](https://x/one.png)") && !text.contains("![[Pasted"), "{}", text.len());
        let _ = std::fs::remove_dir_all(v);
    }
}

#[cfg(test)]
mod welcome_tests {
    use super::*;

    #[test]
    fn a_first_run_opens_on_the_welcome_note_written_into_the_new_vault() {
        let root = std::env::temp_dir().join(format!("notsy-app-test-{}-welcome", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let vault = root.join("Notsy");
        let mut m = Model::new(vault.clone(), true);
        m.frame(1000, 700, 1.0);
        m.first_run = true;
        m.start_arrivals(|| {});
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !m.poll_arrivals() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(m.session.note, "Welcome to Notsy.md");
        assert!(m.text().contains("\u{2318}"), "the Mac's and iPad's shortcut key");
        assert!(vault.join("Welcome to Notsy.png").is_file());
        // A second start writes nothing more.
        m.start_arrivals(|| {});
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_welcome_note_waits_in_the_list_when_writing_has_begun() {
        let root = std::env::temp_dir().join(format!("notsy-app-test-{}-welcome-typed", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let vault = root.join("Notsy");
        let mut m = Model::new(vault.clone(), true);
        m.frame(1000, 700, 1.0);
        m.insert("Already writing");
        m.first_run = true;
        m.start_arrivals(|| {});
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !m.poll_arrivals() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(m.text().starts_with("Already writing"), "what was typed stays open");
        assert!(m.session.sidebar.iter().any(|r| r.name.contains("Welcome")), "the welcome is listed");
        let _ = std::fs::remove_dir_all(root);
    }
}
