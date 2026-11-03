//! Notes brought in from elsewhere in one go, the way the welcome screen
//! does it (`kolada_ui::welcome`): an Obsidian vault, a folder of notes,
//! another app's export, a Notes app export or documents, each into a new
//! folder of the vault, on a worker of the shell's. It says how far it has
//! come (`Progress`), and it stops when asked: what it had written is
//! taken away again, so a stopped import leaves the vault as it was.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use kolada_core::note;

use crate::import::{self, Collection, Entry, Source};

/// What a file picked for an export says when it is not one.
pub const NOT_AN_EXPORT: &str = "This isn't an export Kolada can read";

/// What is brought in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An Obsidian vault: its notes and everything beside them (pictures,
    /// PDFs), folders kept; its own settings (`.obsidian`) left behind.
    Vault,
    /// A folder of Markdown and text, folders kept.
    Folder,
    /// Another app's export file (Evernote's `.enex`, a Notion `.zip`).
    Export(Source),
    /// The Notes app's export (`.nbk`).
    Nbk,
    /// Word, PDF and web pages, a note each (converted by the shell's way:
    /// on Windows in a process of its own).
    Documents,
}

/// How far an import has come, shared with the shell's frame: notes done
/// of all of them (0 while that is not known yet), and a stop asked for.
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicUsize,
    total: AtomicUsize,
    stop: AtomicBool,
}

impl Progress {
    /// Notes done, and of how many (0: still counting).
    pub fn get(&self) -> (usize, usize) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }

    /// Asks the import to stop; it takes back what it wrote.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn total(&self, n: usize) {
        self.total.store(n, Ordering::Relaxed);
    }

    fn step(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }
}

/// What an import came to.
#[derive(Debug, Default, PartialEq)]
pub struct Brought {
    /// The notes written, in the order they came.
    pub notes: Vec<PathBuf>,
    /// The folder of the vault they are in.
    pub folder: PathBuf,
    /// Files that could not be read: their names, and why.
    pub failed: Vec<(String, String)>,
    /// It was stopped, and nothing it wrote is left.
    pub stopped: bool,
    /// The export was found to be another app's than the one it was picked
    /// for (a Takeout on Notion's tile, dropped Bear Markdown): whose. The
    /// folder is named for it.
    pub found: Option<Source>,
    /// It was plain Markdown, zipped, no app's: its folder is named for its
    /// file, and it is said as a folder is.
    pub plain: bool,
}

/// Brings `picked` (the folder, or the files, chosen) in as `kind`, into a
/// new folder of `vault` named `name` (or `name 2`, ...); pictures of
/// exports go to `attachments`. Documents are made notes by `document`
/// (a file, the folder it goes into: its notes). An error when nothing
/// could be brought in, in words a person would use.
pub fn bring(kind: Kind, picked: &[PathBuf], vault: &Path, name: &str, attachments: &Path, progress: &Progress, document: &dyn Fn(&Path, &Path) -> Result<Vec<PathBuf>, String>) -> Result<Brought, String> {
    let mut top = free_folder(vault, name);
    let mut found = None;
    let mut plain: Option<String> = None;
    let mut made = Made::default();
    let result = match kind {
        Kind::Vault | Kind::Folder => {
            let from = picked.first().ok_or("nothing was chosen")?;
            if from.starts_with(vault) || vault.starts_with(from) {
                return Err("that folder is this one".to_string());
            }
            copy_tree(from, &top, kind == Kind::Vault, progress, &mut made)
        }
        Kind::Export(source) => {
            // Every file picked (Evernote's notebooks are one file each),
            // all into the one folder.
            let mut c = Collection::default();
            for file in picked {
                if !file.exists() {
                    return Err("it is not there any more".to_string());
                }
                let (whose, one) = read_export(source, file)?;
                // Markdown zipped with nothing of Bear's in it, on another
                // tile, is no app's: named for its file, as a folder is
                // (`import::folder_name`).
                let name = import::folder_name(whose, &one, file);
                if whose == Source::Bear && source != Source::Bear && name != export_folder(Source::Bear) {
                    plain = Some(name);
                } else if whose != source {
                    found = Some(whose);
                }
                c.notes.extend(one.notes);
                c.skipped += one.skipped;
            }
            // Named for whose it is, before anything is written.
            if let Some(name) = &plain {
                top = free_folder(vault, name);
                found = None;
            } else if let Some(s) = found {
                top = free_folder(vault, &export_folder(s));
            }
            write_collection(&c, &top, attachments, progress, &mut made)
        }
        Kind::Nbk => {
            let file = picked.first().ok_or("nothing was chosen")?;
            let bytes = std::fs::read(file).map_err(|e| said(&e))?;
            let export = kolada_core::nbk::read(&bytes).map_err(|_| NOT_AN_EXPORT.to_string())?;
            let c = Collection {
                notes: export.notes.into_iter().map(|n| Entry { folders: n.folders, title: title_of(&n.text), markdown: n.text, created: n.created, modified: n.modified, files: Vec::new() }).collect(),
                skipped: export.trashed + export.empty,
                ..Default::default()
            };
            write_collection(&c, &top, attachments, progress, &mut made)
        }
        Kind::Documents => {
            progress.total(picked.len());
            let mut failed = Vec::new();
            for f in picked {
                if progress.stopped() {
                    break;
                }
                if std::fs::create_dir_all(&top).is_ok() {
                    made.dir(&top);
                }
                match document(f, &top) {
                    Ok(notes) => made.notes.extend(notes),
                    Err(why) => failed.push((name_of(f), why)),
                }
                progress.step();
            }
            Ok(failed)
        }
    };
    if progress.stopped() {
        made.take_back();
        return Ok(Brought { notes: Vec::new(), folder: top, failed: Vec::new(), stopped: true, found, plain: plain.is_some() });
    }
    let failed = match result {
        Ok(failed) => failed,
        Err(why) => {
            made.take_back();
            return Err(why);
        }
    };
    if made.notes.is_empty() {
        made.take_back();
        return Err(match failed.as_slice() {
            [(_, why)] => why.clone(),
            [] => "it has no notes".to_string(),
            _ => format!("none of the {} files could be read", failed.len()),
        });
    }
    Ok(Brought { notes: made.notes, folder: top, failed, stopped: false, found, plain: plain.is_some() })
}

/// An export picked for `source`'s tile (or dropped), read once: whose it
/// is by what is in it, and its notes. A zip on Notion's tile may be Keep's
/// or Bear's; on Keep's or Bear's, one that is only a zip is not theirs.
fn read_export(source: Source, file: &Path) -> Result<(Source, Collection), String> {
    match import::read_found(file) {
        // A zip that reads as nothing else is taken for Notion's: not what
        // Keep's or Bear's tile was given.
        Ok((Source::Notion, _)) if matches!(source, Source::GoogleKeep | Source::Bear) => Err(NOT_AN_EXPORT.to_string()),
        Ok((whose, c)) if !c.notes.is_empty() || whose == source => Ok((whose, c)),
        // Keep's and Bear's folders and files, read as theirs.
        _ => import::read_path(source, file).map(|c| (source, c)).map_err(|_| NOT_AN_EXPORT.to_string()),
    }
}

/// The folder an export found to be `source`'s goes into, as its tile
/// would name it (`kolada_ui::welcome::From::folder`).
fn export_folder(source: Source) -> String {
    match source {
        Source::Evernote => "From Evernote".to_string(),
        Source::Notion => "From Notion".to_string(),
        s => s.folder().unwrap_or("Imported notes").to_string(),
    }
}

/// What an import wrote, to take back when it stops.
#[derive(Default)]
struct Made {
    notes: Vec<PathBuf>,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl Made {
    fn dir(&mut self, d: &Path) {
        if !self.dirs.iter().any(|x| x == d) {
            self.dirs.push(d.to_path_buf());
        }
    }

    /// Removes what was written, then the folders it made (deepest first)
    /// that are empty: one something else was put in stays.
    fn take_back(&mut self) {
        for f in self.notes.iter().chain(&self.files) {
            let _ = std::fs::remove_file(f);
        }
        let mut dirs: Vec<&PathBuf> = self.dirs.iter().collect();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
        for d in dirs {
            let _ = std::fs::remove_dir(d);
        }
        self.notes.clear();
        self.files.clear();
    }
}

/// `vault/name`, or `name 2`, ... when that is taken.
fn free_folder(vault: &Path, name: &str) -> PathBuf {
    let name = note::file_stem_for(name);
    (1..).map(|n| if n == 1 { vault.join(&name) } else { vault.join(format!("{name} {n}")) }).find(|p| !p.exists()).expect("a free name")
}

fn is_note(path: &Path) -> bool {
    path.extension().is_some_and(|e| ["md", "markdown", "txt"].contains(&e.to_string_lossy().to_ascii_lowercase().as_str()))
}

/// The files of `from` to copy, with where each goes under `to`: notes,
/// and with `everything` the rest beside them; nothing hidden (`.obsidian`,
/// `.trash`, `.git`). In name order, folder by folder.
fn files_of(from: &Path, to: &Path, everything: bool) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    let mut todo = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((src, dst)) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&src) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let path = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => todo.push((path, dst.join(&name))),
                Ok(t) if t.is_file() && (everything || is_note(&path)) => out.push((path, dst.join(&name))),
                _ => {}
            }
        }
    }
    out
}

/// Copies a folder's notes (and with `everything`, what is beside them)
/// into `top`, folders kept, dated as they were. The files that failed.
fn copy_tree(from: &Path, top: &Path, everything: bool, progress: &Progress, made: &mut Made) -> Result<Vec<(String, String)>, String> {
    if !from.is_dir() {
        return Err("that is not a folder".to_string());
    }
    let files = files_of(from, top, everything);
    progress.total(files.iter().filter(|(f, _)| is_note(f)).count());
    let mut failed = Vec::new();
    for (src, dst) in files {
        if progress.stopped() {
            break;
        }
        let Some(dir) = dst.parent() else { continue };
        if std::fs::create_dir_all(dir).is_err() {
            failed.push((name_of(&src), "its folder could not be made".to_string()));
            continue;
        }
        // Each folder made, from the top down, to take back if stopped.
        for d in dir.ancestors().take_while(|d| d.starts_with(top)) {
            made.dir(d);
        }
        let note = is_note(&src);
        match std::fs::copy(&src, &dst) {
            Ok(_) => {
                keep_dates(&src, &dst);
                if note { made.notes.push(dst) } else { made.files.push(dst) }
            }
            Err(e) => failed.push((name_of(&src), said(&e))),
        }
        if note {
            progress.step();
        }
    }
    Ok(failed)
}

/// `to` dated as `from` was made and last changed.
fn keep_dates(from: &Path, to: &Path) {
    let Ok(meta) = std::fs::metadata(from) else { return };
    let mut times = std::fs::FileTimes::new();
    if let Ok(m) = meta.modified() {
        times = times.set_modified(m).set_accessed(m);
    }
    #[cfg(windows)]
    if let Ok(c) = meta.created() {
        use std::os::windows::fs::FileTimesExt;
        times = times.set_created(c);
    }
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(to) {
        let _ = f.set_times(times);
    }
}

/// An export's notes written into the new folder `top`, a note at a time
/// (`import::Writer`), each counted as it goes; a stop asked for ends it
/// there.
fn write_collection(c: &Collection, top: &Path, attachments: &Path, progress: &Progress, made: &mut Made) -> Result<Vec<(String, String)>, String> {
    if c.notes.is_empty() {
        return Err("it has no notes".to_string());
    }
    progress.total(c.notes.len());
    let (Some(dir), Some(name)) = (top.parent(), top.file_name()) else { return Err("it has no folder to go in".to_string()) };
    // Made here: taken back with the rest when it stops.
    if !attachments.exists() {
        made.dir(attachments);
    }
    let mut w = import::Writer::new(&name.to_string_lossy(), dir, attachments);
    for n in &c.notes {
        if progress.stopped() {
            break;
        }
        if let Some(to) = w.write(n) {
            for d in to.ancestors().skip(1).take_while(|d| d.starts_with(top)) {
                made.dir(d);
            }
            made.notes.push(to);
        }
        progress.step();
    }
    made.files.append(&mut w.files);
    let failed = if w.failed == 0 { Vec::new() } else { vec![(String::new(), import::unwritten(w.failed))] };
    Ok(failed)
}

/// A note's title from its text: its first line that says something, as
/// plain words.
fn title_of(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let plain = line.trim_start_matches(['#', '>', '-', '*', ' ']).replace(['*', '_', '`', '[', ']'], "");
    plain.trim().chars().take(80).collect::<String>().trim_end().to_string()
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A file error in words a person would use.
fn said(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "it is not there any more".to_string(),
        std::io::ErrorKind::PermissionDenied => "Kolada is not allowed to open it".to_string(),
        _ => "it could not be opened".to_string(),
    }
}

#[cfg(test)]
mod tests;
