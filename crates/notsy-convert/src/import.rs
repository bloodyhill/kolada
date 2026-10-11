//! Other note apps' exports into a vault: Evernote's `.enex` and Notion's
//! "Markdown & CSV" `.zip`. Each is read into a [`Collection`] of notes with
//! their folders, dates and attachments, then written as Markdown files into
//! a folder of their own (see [`write`]), dated as they were.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use notsy_core::note;

use crate::{Error, Picture, enex, notion};

/// The apps whose exports are read here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Evernote,
    Notion,
}

impl Source {
    /// The app a file comes from, by its extension. A `.zip` is taken for a
    /// Notion export until it is read and turns out not to be one.
    pub fn of(path: &Path) -> Option<Source> {
        let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
        match ext.as_str() {
            "enex" => Some(Source::Evernote),
            "zip" => Some(Source::Notion),
            _ => None,
        }
    }
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
}

/// Reads an export of `source` from its bytes.
pub fn read(source: Source, bytes: &[u8]) -> Result<Collection, Error> {
    match source {
        Source::Evernote => enex::read(bytes),
        Source::Notion => notion::read(bytes),
    }
}

/// Writes `c` into a new folder of `dir` named `name` (or `name 2`, ...),
/// in the notes' folders, their attachments into `attachments`. Each note
/// is dated as the export says. The notes written, in the export's order.
pub fn write(c: &Collection, name: &str, dir: &Path, attachments: &Path) -> Result<Vec<PathBuf>, Error> {
    let name = note::file_stem_for(name);
    let top = (1..).map(|n| if n == 1 { dir.join(&name) } else { dir.join(format!("{name} {n}")) }).find(|p| !p.exists()).expect("a free name");
    let mut written = Vec::new();
    // The folder is new: names taken in it are known without asking the disk.
    let mut taken: HashSet<PathBuf> = HashSet::new();
    let mut files_taken: HashSet<PathBuf> = HashSet::new();
    for n in &c.notes {
        let folder = n.folders.iter().fold(top.clone(), |p, f| p.join(note::file_stem_for(f)));
        if std::fs::create_dir_all(&folder).is_err() {
            continue;
        }
        let stem = note::file_stem_for(&n.title);
        let to = (1..)
            .map(|k| folder.join(if k == 1 { format!("{stem}.md") } else { format!("{stem} {k}.md") }))
            .find(|p| !taken.contains(&lower(p)))
            .expect("a free name");
        taken.insert(lower(&to));
        // Attachments: under a free name each, the note's links following.
        let mut renamed = Vec::new();
        for f in &n.files {
            if std::fs::create_dir_all(attachments).is_err() {
                break;
            }
            let at = free_name(attachments, &f.name, &mut files_taken);
            if std::fs::write(&at, &f.data).is_ok() {
                let now = at.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if now != f.name {
                    renamed.push((f.name.clone(), now));
                }
            }
        }
        let text = if renamed.is_empty() { n.markdown.clone() } else { relink(&n.markdown, &renamed) };
        let Ok(mut file) = std::fs::File::create_new(&to) else { continue };
        if std::io::Write::write_all(&mut file, text.as_bytes()).is_err() {
            continue;
        }
        let _ = file.set_times(times(n.created, n.modified));
        written.push(to);
    }
    if written.is_empty() && c.notes.is_empty() {
        return Err(Error("it has no notes".to_string()));
    }
    Ok(written)
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
    let at = |ms: i64| (ms > 0).then(|| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms as u64));
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

/// Milliseconds since 1970 for a UTC time.
pub(crate) fn millis(year: i32, month: u32, day: u32, h: u32, m: u32, s: u32) -> Option<i64> {
    let d = notsy_core::when::Day::new(year, month, day)?;
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

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("notsy-import-{name}-{}", std::process::id()));
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
            skipped: 0,
        };
        let written = write(&c, "My Notebook", &dir, &pics).unwrap();
        let rel: Vec<String> = written.iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(rel, ["My Notebook/Trips/Rome.md", "My Notebook/Trips/Rome 2.md", "My Notebook/Plans 2026.md"]);
        let rome = std::fs::read_to_string(&written[0]).unwrap();
        assert_eq!(rome, "# Rome\n\n![[photo 1.png]] and [[photo 1.png|the photo]]\n", "the vault's own photo.png is kept");
        assert_eq!(std::fs::read(pics.join("photo 1.png")).unwrap(), b"new");
        assert_eq!(std::fs::read(pics.join("photo 2.png")).unwrap(), b"other");
        let modified = std::fs::metadata(&written[0]).unwrap().modified().unwrap();
        assert_eq!(modified.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(), 1_700_000_000_000);
        // A second import of the same goes beside the first.
        let again = write(&c, "My Notebook", &dir, &pics).unwrap();
        assert!(again[0].starts_with(dir.join("My Notebook 2")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relinking_renames_each_link_once() {
        let renamed = [("a.png".to_string(), "b.png".to_string()), ("b.png".to_string(), "c.png".to_string())];
        assert_eq!(relink("![[a.png]] [[b.png|B]] [[x]] [[", &renamed), "![[b.png]] [[c.png|B]] [[x]] [[");
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
