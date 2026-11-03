//! Other note apps' exports into a vault: Evernote's `.enex`, Notion's
//! "Markdown & CSV" `.zip`, Google Keep's Takeout, Bear's backup or
//! Markdown export, and Apple Notes' notes as Notes gives them (see
//! `applenotes`). Each is read into a [`Collection`] of notes with their
//! folders, dates and attachments, then written as Markdown files into a
//! folder of their own (see [`write`]), dated as they were.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use kolada_core::note;

use crate::tree::Tree;
use crate::{Error, Picture, applenotes, bear, enex, keep, notion};

/// The apps whose exports are read here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Evernote,
    Notion,
    /// Google Takeout's Keep: its `.zip`, or the folder it unpacks into.
    GoogleKeep,
    /// Bear's `.bear2bk` backup, or its TextBundle or Markdown export
    /// (zipped, or the folder it was written into).
    Bear,
    /// The Mac's Notes, asked for its notes (no file: see `applenotes`).
    AppleNotes,
}

/// The folder archived notes come in under.
pub const ARCHIVED: &str = "Archived";

impl Source {
    /// The app a file comes from, by its extension. A `.zip` is taken for a
    /// Notion export until it is read (see [`Source::detect`]).
    pub fn of(path: &Path) -> Option<Source> {
        let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
        match ext.as_str() {
            "enex" => Some(Source::Evernote),
            "zip" => Some(Source::Notion),
            // A TextBundle zipped is a TextPack.
            "bear2bk" | "textpack" => Some(Source::Bear),
            _ => None,
        }
    }

    /// The app a file or folder comes from, by what is in it: a zip of
    /// Keep's notes, or of Bear's bundles, is theirs and not Notion's; a
    /// folder of either is too.
    pub fn detect(path: &Path) -> Option<Source> {
        if path.is_dir() {
            return sniff(&Tree::dir(path).ok()?, true);
        }
        match Source::of(path)? {
            Source::Notion => {
                let bytes = std::fs::read(path).ok()?;
                Some(sniff(&Tree::zip(&bytes).ok()?, false).unwrap_or(Source::Notion))
            }
            s => Some(s),
        }
    }

    /// The folder its notes come into, for apps whose exports carry no name
    /// of their own; None for the others (named after their file).
    pub fn folder(self) -> Option<&'static str> {
        match self {
            Source::GoogleKeep => Some("From Google Keep"),
            Source::Bear => Some("From Bear"),
            Source::AppleNotes => Some(applenotes::FOLDER),
            Source::Evernote | Source::Notion => None,
        }
    }
}

/// Whose export a tree is, by its files: Keep's notes (under a `Keep`
/// folder; in a folder of its own, any of Keep's JSON notes), or Bear's
/// bundles.
fn sniff(tree: &Tree, folder: bool) -> Option<Source> {
    let mut json = Vec::new();
    // Keep's notes win over bundles found elsewhere in a Takeout (a
    // `.textbundle` kept in Drive is no sign of Bear).
    let (mut bundles, mut pages, mut notion) = (false, false, false);
    for n in tree.names() {
        let lower = n.to_ascii_lowercase();
        bundles |= lower.contains(".textbundle/");
        if lower.ends_with(".json") {
            if lower.starts_with("keep/") || lower.contains("/keep/") {
                return Some(Source::GoogleKeep);
            }
            json.push(n.to_string());
        }
        if notion::is_page(n) {
            pages |= lower.ends_with(".md") || lower.ends_with(".markdown");
            notion |= notion::has_id(crate::tree::stem(n));
        }
    }
    if bundles {
        return Some(Source::Bear);
    }
    // Markdown with none of Notion's ids on its pages: Bear's export.
    if pages && !notion && !folder {
        return Some(Source::Bear);
    }
    // A bare Keep folder: its notes at its top. A few are enough to tell.
    let keep = folder && json.iter().take(8).any(|n| tree.read(n).and_then(|b| crate::json::parse(&String::from_utf8_lossy(&b))).is_some_and(|v| keep::is_note(&v)));
    keep.then_some(Source::GoogleKeep)
}

/// One note of an export.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    /// The folders it is in, outermost first.
    pub folders: Vec<String>,
    pub title: String,
    /// Its Markdown, its title first. Attachments are embedded or linked as
    /// `![[name]]` / `[[name]]` by the names in `files`.
    pub markdown: String,
    /// Milliseconds since 1970 (UTC); 0 when the export does not say.
    pub created: i64,
    pub modified: i64,
    /// Attachments, by names unique within the note.
    pub files: Vec<Picture>,
}

/// What an export holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Collection {
    pub notes: Vec<Entry>,
    /// Notes left out: empty ones.
    pub skipped: usize,
    /// Notes left where they were: locked ones, and ones the app would not
    /// give; and attachments that did not come with their notes.
    pub locked: usize,
    pub unreadable: usize,
    pub attachments_left: usize,
    /// The app stopped before it gave every note (Apple Notes): the notes
    /// here are those it gave, and `not_reached` the ones it never got to.
    pub stopped: bool,
    pub not_reached: usize,
    /// The export carries its app's own marks (Bear's bundles), not just
    /// the shape of its export: its folder is named for the app.
    pub marked: bool,
}

/// The folder the export at `file` comes into: "From Google Keep"; "From
/// Bear" for Bear's own backup or bundles; "Notion" for an `Export-<id>`;
/// otherwise the file's (or folder's) own name, as a zip of Markdown that
/// is only shaped like Bear's export (an Obsidian vault, a repository).
pub fn folder_name(source: Source, c: &Collection, file: &Path) -> String {
    let named = if file.is_dir() { file.file_name() } else { file.file_stem() };
    let stem = named.map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = file.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match source {
        Source::GoogleKeep => "From Google Keep".to_string(),
        Source::Bear if c.marked || matches!(ext.as_str(), "bear2bk" | "textpack") => "From Bear".to_string(),
        Source::Notion if stem.starts_with("Export-") || stem.is_empty() => "Notion".to_string(),
        _ if stem.is_empty() => "Imported notes".to_string(),
        _ => stem,
    }
}

/// Reads an export of `source` from its file's bytes.
pub fn read(source: Source, bytes: &[u8]) -> Result<Collection, Error> {
    match source {
        Source::Evernote => enex::read(bytes),
        Source::Notion => notion::read(bytes),
        Source::GoogleKeep => keep::read(&Tree::zip(bytes)?),
        Source::Bear => bear::read(&Tree::zip(bytes)?),
        Source::AppleNotes => applenotes::read(bytes),
    }
}

/// Reads an export of `source` at `path`: its file, or (Keep, Bear) the
/// folder it was unpacked or exported into.
pub fn read_path(source: Source, path: &Path) -> Result<Collection, Error> {
    if path.is_dir() {
        let tree = Tree::dir(path)?;
        return match source {
            Source::GoogleKeep => keep::read(&tree),
            Source::Bear => bear::read(&tree),
            Source::Evernote | Source::Notion | Source::AppleNotes => Err(Error("choose the export's file, not a folder".to_string())),
        };
    }
    let bytes = std::fs::read(path).map_err(|_| Error("the file could not be read".to_string()))?;
    read(source, &bytes)
}

/// Reads whatever export `path` is, telling whose by what is in it (see
/// [`Source::detect`]) with one read of the file: whose, and its notes.
pub fn read_found(path: &Path) -> Result<(Source, Collection), Error> {
    let unknown = Error::unreadable;
    if path.is_dir() {
        let tree = Tree::dir(path)?;
        let source = sniff(&tree, true).ok_or_else(unknown)?;
        let c = if source == Source::Bear { bear::read(&tree)? } else { keep::read(&tree)? };
        return Ok((source, c));
    }
    let source = Source::of(path).ok_or_else(unknown)?;
    let bytes = std::fs::read(path).map_err(|_| Error("the file could not be read".to_string()))?;
    let source = match source {
        Source::Notion => Tree::zip(&bytes).ok().and_then(|t| sniff(&t, false)).unwrap_or(Source::Notion),
        s => s,
    };
    match read(source, &bytes) {
        // What Notion refuses may still be Bear's Markdown, zipped.
        Err(e) if source == Source::Notion => match read(Source::Bear, &bytes) {
            Ok(c) if !c.notes.is_empty() => Ok((Source::Bear, c)),
            _ => Err(e),
        },
        got => Ok((source, got?)),
    }
}

/// What `write` did: the notes it wrote, in the export's order, and how
/// many it could not (a folder or file the disk refused).
#[derive(Debug, Default, PartialEq)]
pub struct Written {
    pub notes: Vec<PathBuf>,
    pub failed: usize,
}

/// Writes `c` into a new folder of `dir` named `name` (or `name 2`, ...),
/// in the notes' folders, their attachments into `attachments`. Each note
/// is dated as the export says. The notes written, in the export's order,
/// and how many the disk refused.
pub fn write(c: &Collection, name: &str, dir: &Path, attachments: &Path) -> Result<Written, Error> {
    let mut w = Writer::new(name, dir, attachments);
    let written: Vec<PathBuf> = c.notes.iter().filter_map(|n| w.write(n)).collect();
    if written.is_empty() && c.notes.is_empty() {
        return Err(Error("it has no notes".to_string()));
    }
    Ok(Written { notes: written, failed: w.failed })
}

/// Notes `write` could not write, in words.
pub fn unwritten(failed: usize) -> String {
    if failed == 1 { "1 of its notes could not be written".to_string() } else { format!("{failed} of its notes could not be written") }
}

/// Writes notes one at a time into a new folder of `dir` named `name` (or
/// `name 2`, ...), made with the first note: for an import read note by
/// note, so only one note (and its pictures) is held at once.
pub struct Writer {
    name: String,
    dir: PathBuf,
    attachments: PathBuf,
    top: Option<PathBuf>,
    /// Notes the disk refused (a folder or a file it would not make).
    pub failed: usize,
    /// The attachments it wrote, to take back if the import stops.
    pub files: Vec<PathBuf>,
    // The folder is new: names taken in it are known without asking the disk.
    taken: HashSet<PathBuf>,
    files_taken: HashSet<PathBuf>,
}

impl Writer {
    pub fn new(name: &str, dir: &Path, attachments: &Path) -> Writer {
        Writer { name: note::file_stem_for(name), dir: dir.to_path_buf(), attachments: attachments.to_path_buf(), top: None, taken: HashSet::new(), files_taken: HashSet::new(), failed: 0, files: Vec::new() }
    }

    /// Writes `n` in its folders, its attachments beside the vault's
    /// others, dated as it was; where it went, or None if it could not be.
    pub fn write(&mut self, n: &Entry) -> Option<PathBuf> {
        let (dir, name) = (&self.dir, &self.name);
        let top = self.top.get_or_insert_with(|| (1..).map(|k| if k == 1 { dir.join(name) } else { dir.join(format!("{name} {k}")) }).find(|p| !p.exists()).expect("a free name"));
        let folder = n.folders.iter().fold(top.clone(), |p, f| p.join(note::file_stem_for(f)));
        if std::fs::create_dir_all(&folder).is_err() {
            self.failed += 1;
            return None;
        }
        let stem = note::file_stem_for(&n.title);
        let to = (1..)
            .map(|k| folder.join(if k == 1 { format!("{stem}.md") } else { format!("{stem} {k}.md") }))
            .find(|p| !self.taken.contains(&lower(p)))
            .expect("a free name");
        self.taken.insert(lower(&to));
        // Attachments: under a free name each, the note's links following.
        let mut renamed = Vec::new();
        for f in &n.files {
            if std::fs::create_dir_all(&self.attachments).is_err() {
                break;
            }
            let at = free_name(&self.attachments, &f.name, &mut self.files_taken);
            if std::fs::write(&at, &f.data).is_ok() {
                let now = at.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if now != f.name {
                    renamed.push((f.name.clone(), now));
                }
                self.files.push(at.clone());
            }
        }
        let text = if renamed.is_empty() { n.markdown.clone() } else { relink(&n.markdown, &renamed) };
        let Ok(mut file) = std::fs::File::create_new(&to) else {
            self.failed += 1;
            return None;
        };
        if std::io::Write::write_all(&mut file, text.as_bytes()).is_err() {
            self.failed += 1;
            return None;
        }
        let _ = file.set_times(times(n.created, n.modified));
        Some(to)
    }
}

fn lower(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().to_lowercase())
}

/// `dir/name`, or `name 1.ext`, `name 2.ext`... when taken on disk or by
/// this import.
fn free_name(dir: &Path, name: &str, taken: &mut HashSet<PathBuf>) -> PathBuf {
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let at = (0..)
        .map(|n| match (n, ext) {
            (0, _) => dir.join(name),
            (n, "") => dir.join(format!("{stem} {n}")),
            (n, ext) => dir.join(format!("{stem} {n}.{ext}")),
        })
        .find(|p| !taken.contains(&lower(p)) && !p.exists())
        .expect("a free name");
    taken.insert(lower(&at));
    at
}

/// `text` with each `[[was]]` (or `![[was]]`, `[[was|shown]]`) pointing at
/// its new name, in one pass so no renamed link is renamed again.
fn relink(text: &str, renamed: &[(String, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("[[") {
        out.push_str(&rest[..i + 2]);
        rest = &rest[i + 2..];
        let Some(end) = rest.find("]]") else { break };
        let inner = &rest[..end];
        let (target, shown) = inner.split_once('|').map_or((inner, None), |(t, s)| (t, Some(s)));
        match renamed.iter().find(|(was, _)| was == target) {
            Some((_, now)) => {
                out.push_str(now);
                if let Some(s) = shown {
                    out.push('|');
                    out.push_str(s);
                }
            }
            None => out.push_str(inner),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// A file's times for one made at `created` and last changed at `modified`
/// (milliseconds since 1970; 0 leaves one as it is).
fn times(created: i64, modified: i64) -> std::fs::FileTimes {
    let at = |ms: i64| sane(ms).and_then(|ms| std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_millis(ms as u64)));
    let mut times = std::fs::FileTimes::new();
    if let Some(m) = at(modified).or(at(created)) {
        times = times.set_modified(m).set_accessed(m);
    }
    #[cfg(windows)]
    if let Some(c) = at(created) {
        use std::os::windows::fs::FileTimesExt;
        times = times.set_created(c);
    }
    times
}

/// The latest time a note is dated: the year 9999 (a file system takes no
/// later, and Windows' clock panics past about the year 30,000).
const LATEST: i64 = 253_402_300_799_000;

/// `ms` when it is a time a file can be dated: after 1970, before `LATEST`.
pub(crate) fn sane(ms: i64) -> Option<i64> {
    (ms > 0 && ms <= LATEST).then_some(ms)
}

/// Milliseconds since 1970 for a UTC time.
pub(crate) fn millis(year: i32, month: u32, day: u32, h: u32, m: u32, s: u32) -> Option<i64> {
    let d = kolada_core::when::Day::new(year, month, day)?;
    (h < 24 && m < 60 && s < 61).then(|| ((d.number() * 24 + i64::from(h)) * 60 + i64::from(m)) * 60_000 + i64::from(s) * 1000)
}

/// A name for an attachment that no other in the same note has: `name`,
/// else `name 2.ext`, ...
pub(crate) fn unique_in(files: &[Picture], name: &str) -> String {
    let (stem, ext) = name.rsplit_once('.').map_or((name, String::new()), |(s, e)| (s, format!(".{e}")));
    (1..)
        .map(|n| if n == 1 { name.to_string() } else { format!("{stem} {n}{ext}") })
        .find(|c| !files.iter().any(|f| f.name.eq_ignore_ascii_case(c)))
        .expect("a free name")
}

/// A file name safe to write and to link: what `note::file_stem_for`
/// keeps, with its extension.
pub(crate) fn file_name_for(name: &str, fallback_ext: &str) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()) => (s, e.to_ascii_lowercase()),
        _ => (name, fallback_ext.to_string()),
    };
    let stem = note::file_stem_for(stem);
    let stem = if stem == "Untitled" { "attachment".to_string() } else { stem };
    if ext.is_empty() { stem } else { format!("{stem}.{ext}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file that is no export, or not the one its tile asked for, is
    /// refused in one wording that names no app.
    #[test]
    fn a_file_kolada_cannot_read_is_said_so_naming_no_app() {
        let dir = std::env::temp_dir().join(format!("kolada-unreadable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip = dir.join("random.zip");
        // A zip holding one text file: no export of any app.
        const RANDOM: &[u8] = &[80, 75, 3, 4, 20, 0, 0, 0, 0, 0, 7, 107, 99, 93, 220, 177, 145, 248, 18, 0, 0, 0, 18, 0, 0, 0, 10, 0, 0, 0, 114, 101, 97, 100, 109, 101, 46, 116, 120, 116, 110, 111, 116, 32, 97, 32, 110, 111, 116, 101, 115, 32, 101, 120, 112, 111, 114, 116, 80, 75, 1, 2, 20, 0, 20, 0, 0, 0, 0, 0, 7, 107, 99, 93, 220, 177, 145, 248, 18, 0, 0, 0, 18, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 1, 0, 0, 0, 0, 114, 101, 97, 100, 109, 101, 46, 116, 120, 116, 80, 75, 5, 6, 0, 0, 0, 0, 1, 0, 1, 0, 56, 0, 0, 0, 58, 0, 0, 0, 0, 0];
        std::fs::write(&zip, RANDOM).unwrap();
        let said = read_found(&zip).err().expect("refused").to_string();
        assert_eq!(said, "it isn't an export Kolada can read");
        let bytes = std::fs::read(&zip).unwrap();
        for source in [Source::Notion, Source::GoogleKeep, Source::Bear, Source::Evernote] {
            let said = read(source, &bytes).err().map(|e| e.to_string()).unwrap_or_default();
            assert!(!["Notion", "Keep", "Bear", "Evernote"].iter().any(|app| said.contains(app)), "{source:?}: {said}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(folders: &[&str], title: &str, markdown: &str, files: &[(&str, &[u8])]) -> Entry {
        Entry {
            folders: folders.iter().map(|s| s.to_string()).collect(),
            title: title.into(),
            markdown: markdown.into(),
            created: 1_600_000_000_000,
            modified: 1_700_000_000_000,
            files: files.iter().map(|(n, d)| Picture { name: n.to_string(), data: d.to_vec() }).collect(),
        }
    }

    #[test]
    fn notes_land_in_their_folders_dated_with_attachments_beside_the_vaults_others() {
        let dir = temp("write");
        let pics = dir.join("attachments");
        std::fs::create_dir_all(&pics).unwrap();
        std::fs::write(pics.join("photo.png"), b"already here").unwrap();
        let c = Collection {
            notes: vec![
                entry(&["Trips"], "Rome", "# Rome\n\n![[photo.png]] and [[photo.png|the photo]]\n", &[("photo.png", b"new")]),
                entry(&["Trips"], "Rome", "# Rome\n\nagain\n", &[]),
                entry(&[], "Plans: 2026?", "# Plans\n", &[("photo.png", b"other")]),
            ],
            ..Collection::default()
        };
        let written = write(&c, "My Notebook", &dir, &pics).unwrap().notes;
        let rel: Vec<String> = written.iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(rel, ["My Notebook/Trips/Rome.md", "My Notebook/Trips/Rome 2.md", "My Notebook/Plans 2026.md"]);
        let rome = std::fs::read_to_string(&written[0]).unwrap();
        assert_eq!(rome, "# Rome\n\n![[photo 1.png]] and [[photo 1.png|the photo]]\n", "the vault's own photo.png is kept");
        assert_eq!(std::fs::read(pics.join("photo 1.png")).unwrap(), b"new");
        assert_eq!(std::fs::read(pics.join("photo 2.png")).unwrap(), b"other");
        let modified = std::fs::metadata(&written[0]).unwrap().modified().unwrap();
        assert_eq!(modified.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(), 1_700_000_000_000);
        // A second import of the same goes beside the first.
        let again = write(&c, "My Notebook", &dir, &pics).unwrap().notes;
        assert!(again[0].starts_with(dir.join("My Notebook 2")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Keep note's JSON, as Takeout writes it.
    fn keep_note(title: &str, text: &str, attachment: Option<&str>) -> String {
        let files = attachment.map_or(String::new(), |a| format!(", \"attachments\": [{{\"filePath\": \"{a}\", \"mimetype\": \"image/png\"}}]"));
        format!("{{\"title\": \"{title}\", \"textContent\": \"{text}\", \"isTrashed\": false, \"isArchived\": false, \"createdTimestampUsec\": 1600000000000000, \"userEditedTimestampUsec\": 1700000000000000{files}}}")
    }

    #[test]
    fn exports_are_told_apart_by_what_is_in_them() {
        use crate::zip::stored;
        let dir = temp("detect");
        let keep = dir.join("takeout-20261101T000000Z-001.zip");
        std::fs::write(&keep, stored(&[("Takeout/Keep/a.json", keep_note("A", "x", None).as_bytes())])).unwrap();
        let notion = dir.join("Export-1234.zip");
        std::fs::write(&notion, stored(&[("Plan 0123456789abcdef0123456789abcdef.md", b"# Plan\n")])).unwrap();
        let bear_zip = dir.join("notes.zip");
        std::fs::write(&bear_zip, stored(&[("A.textbundle/text.markdown", b"# A\n")])).unwrap();
        let backup = dir.join("Bear Notes 2026-11-01.bear2bk");
        std::fs::write(&backup, stored(&[("A.textbundle/text.markdown", b"# A\n")])).unwrap();
        assert_eq!(Source::detect(&keep), Some(Source::GoogleKeep));
        assert_eq!(Source::detect(&notion), Some(Source::Notion));
        assert_eq!(Source::detect(&bear_zip), Some(Source::Bear));
        assert_eq!(Source::detect(&backup), Some(Source::Bear));
        assert_eq!(Source::of(&backup), Some(Source::Bear), "a backup is known by its name alone");
        assert_eq!(Source::detect(&dir.join("notes.txt")), None);
        // A bare Keep folder, and a folder that is nobody's export.
        let bare = dir.join("Keep");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(bare.join("a.json"), keep_note("A", "x", None)).unwrap();
        assert_eq!(Source::detect(&bare), Some(Source::GoogleKeep));
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("settings.json"), "{\"theme\": \"dark\"}").unwrap();
        assert_eq!(Source::detect(&other), None);
        // One read: whose, and its notes.
        let (source, c) = read_found(&keep).unwrap();
        assert_eq!((source, c.notes.len()), (Source::GoogleKeep, 1));
        assert_eq!(read_found(&bare).unwrap().0, Source::GoogleKeep);
        assert!(read_path(Source::Notion, &bare).is_err(), "a Notion export is its file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keep_and_bear_land_in_their_own_folders_with_their_attachments_and_dates() {
        use crate::zip::stored;
        let dir = temp("keep-bear");
        let pics = dir.join("attachments");
        std::fs::create_dir_all(&pics).unwrap();
        // The vault already has a picture of the same name.
        std::fs::write(pics.join("صورة 1.png"), b"the vault's").unwrap();
        let takeout = stored(&[
            ("Takeout/Keep/x.json", keep_note("رحلة: مكة/المدينة؟", "نص عربي", Some("صورة 1.png")).as_bytes()),
            ("Takeout/Keep/صورة 1.png", b"KEEP"),
        ]);
        let c = read(Source::GoogleKeep, &takeout).unwrap();
        let written = write(&c, Source::GoogleKeep.folder().unwrap(), &dir, &pics).unwrap().notes;
        let rel: Vec<String> = written.iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(rel.len(), 1);
        assert!(rel[0].starts_with("From Google Keep/") && rel[0].ends_with(".md") && rel[0].contains("رحلة"), "{rel:?}");
        let text = std::fs::read_to_string(&written[0]).unwrap();
        assert_eq!(text, "# رحلة: مكة/المدينة؟\n\nنص عربي\n\n![[صورة 1 1.png]]\n", "the link follows the renamed picture");
        assert_eq!(std::fs::read(pics.join("صورة 1 1.png")).unwrap(), b"KEEP");
        assert_eq!(std::fs::read(pics.join("صورة 1.png")).unwrap(), b"the vault's");
        let modified = std::fs::metadata(&written[0]).unwrap().modified().unwrap();
        assert_eq!(modified.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(), 1_700_000_000_000);
        // Bear's beside it, in a folder of its own.
        let backup = stored(&[("Plan.textbundle/text.markdown", b"# Plan\n\n![](assets/a.png)\n"), ("Plan.textbundle/assets/a.png", b"A")]);
        let c = read(Source::Bear, &backup).unwrap();
        let written = write(&c, Source::Bear.folder().unwrap(), &dir, &pics).unwrap().notes;
        assert_eq!(written, [dir.join("From Bear").join("Plan.md")]);
        assert_eq!(std::fs::read_to_string(&written[0]).unwrap(), "# Plan\n\n![[a.png]]\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_zipped_bear_markdown_export_is_bears_and_a_takeout_is_keeps_whatever_else_it_holds() {
        use crate::zip::stored;
        let dir = temp("detect-more");
        // Markdown with no Notion ids: Bear's export, zipped.
        let bear = dir.join("Bear export.zip");
        std::fs::write(&bear, stored(&[("Groceries.md", b"Groceries\n\n- [ ] eggs #home\n"), ("Plans.md", b"# Plans\n")])).unwrap();
        assert_eq!(Source::detect(&bear), Some(Source::Bear));
        let (source, c) = read_found(&bear).unwrap();
        assert_eq!((source, c.notes.len()), (Source::Bear, 2));
        // A Takeout holding a bundle in Drive beside Keep's notes is Keep's.
        let takeout = dir.join("takeout.zip");
        std::fs::write(&takeout, stored(&[("Takeout/Drive/Old.textbundle/text.markdown", b"# Old\n"), ("Takeout/Keep/a.json", keep_note("A", "x", None).as_bytes())])).unwrap();
        assert_eq!(Source::detect(&takeout), Some(Source::GoogleKeep));
        assert_eq!(read_found(&takeout).unwrap().0, Source::GoogleKeep);
        // A TextPack is Bear's by its name.
        let pack = dir.join("Note.textpack");
        std::fs::write(&pack, stored(&[("text.markdown", b"# Note\n")])).unwrap();
        assert_eq!(Source::of(&pack), Some(Source::Bear));
        assert_eq!(read_found(&pack).unwrap().1.notes.len(), 1);
        // A zip that is no one's export is still refused.
        let other = dir.join("photos.zip");
        std::fs::write(&other, stored(&[("a.png", b"PNG")])).unwrap();
        assert!(read_found(&other).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_the_disk_refuses_are_counted() {
        let dir = temp("refused");
        let note = |folders: &[&str], title: &str| Entry { folders: folders.iter().map(|s| s.to_string()).collect(), title: title.into(), markdown: "x\n".into(), ..Entry::default() };
        // The second note's folder would be the first note's file.
        let c = Collection { notes: vec![note(&[], "Plan.md"), note(&["Plan.md.md"], "Inside"), note(&[], "Other")], ..Collection::default() };
        let w = write(&c, "Refused", &dir, &dir).unwrap();
        assert_eq!((w.notes.len(), w.failed), (2, 1));
        assert_eq!(unwritten(1), "1 of its notes could not be written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_import_comes_into_a_folder_named_for_its_app_only_when_it_is_that_apps() {
        use crate::zip::stored;
        let dir = temp("names");
        let read_as = |name: &str, bytes: Vec<u8>| {
            let file = dir.join(name);
            std::fs::write(&file, bytes).unwrap();
            let (source, c) = read_found(&file).unwrap();
            folder_name(source, &c, &file)
        };
        // Plain Markdown, zipped: read as Bear's export, named as itself.
        assert_eq!(read_as("My Vault.zip", stored(&[("Ideas.md", b"# Ideas\n\n#tag\n"), ("Plans.md", b"# Plans\n")])), "My Vault");
        // Bear's own bundles, or its backup by name.
        assert_eq!(read_as("notes.zip", stored(&[("A.textbundle/text.markdown", b"# A\n")])), "From Bear");
        assert_eq!(read_as("Bear Notes.bear2bk", stored(&[("A.md", b"# A\n")])), "From Bear");
        assert_eq!(read_as("takeout-001.zip", stored(&[("Takeout/Keep/a.json", keep_note("A", "x", None).as_bytes())])), "From Google Keep");
        let page = ("Plan 0123456789abcdef0123456789abcdef.md", &b"# Plan\n"[..]);
        assert_eq!(read_as("Export-1234.zip", stored(&[page])), "Notion");
        assert_eq!(read_as("Team wiki.zip", stored(&[page])), "Team wiki");
        // A folder is named by its own name.
        let folder = dir.join("Old notes");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.md"), "# A\n").unwrap();
        let c = read_path(Source::Bear, &folder).unwrap();
        assert_eq!(folder_name(Source::Bear, &c, &folder), "Old notes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_that_cannot_be_read_are_counted() {
        use crate::zip::build;
        // An entry that lies about its size reads as nothing: counted.
        let lie: &[u8] = b"far more than eight bytes";
        let keep = build(&[("Keep/a.json", 0, keep_note("A", "x", None).as_bytes(), keep_note("A", "x", None).len() as u32), ("Keep/b.json", 0, lie, 8)]);
        let c = read(Source::GoogleKeep, &keep).unwrap();
        assert_eq!((c.notes.len(), c.unreadable), (1, 1));
        let bear = build(&[("A.md", 0, b"# A\n", 4), ("B.md", 0, lie, 8)]);
        let c = read(Source::Bear, &bear).unwrap();
        assert_eq!((c.notes.len(), c.unreadable), (1, 1));
        let bundle = build(&[("A.textbundle/text.markdown", 0, lie, 8), ("B.textbundle/text.markdown", 0, b"# B\n", 4)]);
        let c = read(Source::Bear, &bundle).unwrap();
        assert_eq!((c.notes.len(), c.unreadable), (1, 1));
        let id = "0123456789abcdef0123456789abcdef";
        let (a, b) = (format!("A {id}.md"), format!("B {id}.md"));
        let notion = build(&[(&a, 0, b"# A\n", 4), (&b, 0, lie, 8)]);
        let c = read(Source::Notion, &notion).unwrap();
        assert_eq!((c.notes.len(), c.unreadable), (1, 1));
    }

    #[test]
    fn an_attachment_that_does_not_read_is_counted_and_its_link_kept() {
        use crate::zip::build;
        let lie: &[u8] = b"far more than eight bytes";
        let id = "0123456789abcdef0123456789abcdef";
        let page = format!("Trip {id}.md");
        let text = b"# Trip\n\n![](photo.png) and ![](here.png) and ![](nowhere.png)\n";
        let notion = build(&[(&page, 0, text, text.len() as u32), ("photo.png", 0, lie, 8), ("here.png", 0, b"PNG", 3)]);
        let c = read(Source::Notion, &notion).unwrap();
        assert_eq!(c.attachments_left, 1, "the one that is there but does not read; a link to nothing is no attachment");
        assert_eq!(c.notes[0].markdown, "# Trip\n\n![](photo.png) and ![[here.png]] and ![](nowhere.png)\n");
        let note = b"# A\n\n![](assets/x.png)\n";
        let bear = build(&[("A.textbundle/text.markdown", 0, note, note.len() as u32), ("A.textbundle/assets/x.png", 0, lie, 8)]);
        let c = read(Source::Bear, &bear).unwrap();
        assert_eq!((c.notes.len(), c.attachments_left), (1, 1));
    }

    #[test]
    fn relinking_renames_each_link_once() {
        let renamed = [("a.png".to_string(), "b.png".to_string()), ("b.png".to_string(), "c.png".to_string())];
        assert_eq!(relink("![[a.png]] [[b.png|B]] [[x]] [[", &renamed), "![[b.png]] [[c.png|B]] [[x]] [[");
    }

    #[test]
    fn a_note_dated_out_of_range_is_written_and_left_undated() {
        let dir = temp("times");
        let at = |created, modified| Entry { title: format!("{created} {modified}"), markdown: "x\n".into(), created, modified, ..Entry::default() };
        let c = Collection { notes: vec![at(i64::MAX, i64::MAX), at(i64::MIN, -5), at(1_000_000_000_000_000_000, 0), at(0, LATEST + 1)], ..Collection::default() };
        let written = write(&c, "Dates", &dir, &dir).unwrap().notes;
        assert_eq!(written.len(), 4, "every note is written");
        assert_eq!(sane(LATEST), Some(LATEST));
        assert_eq!((sane(0), sane(-1), sane(LATEST + 1)), (None, None, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn times_are_counted_from_1970() {
        assert_eq!(millis(1970, 1, 1, 0, 0, 0), Some(0));
        assert_eq!(millis(2020, 12, 31, 23, 59, 59), Some(1_609_459_199_000));
        assert_eq!(millis(2020, 2, 30, 0, 0, 0), None);
    }

    #[test]
    fn attachment_names_are_safe_and_unique_in_a_note() {
        assert_eq!(file_name_for("my: scan?.PDF", "bin"), "my scan.pdf");
        assert_eq!(file_name_for("", "png"), "attachment.png");
        assert_eq!(file_name_for("no extension", "jpg"), "no extension.jpg");
        let files = vec![Picture { name: "a.png".into(), data: vec![] }, Picture { name: "a 2.png".into(), data: vec![] }];
        assert_eq!(unique_in(&files, "A.png"), "A 3.png");
        assert_eq!(unique_in(&files, "b.png"), "b.png");
    }
}
