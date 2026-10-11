//! Notes another app keeps in the user's Documents, found when Notsy is
//! installed, so the first run can open them where they are (a folder of
//! Markdown: an Obsidian vault, a Logseq graph, any folder of `.md` notes)
//! or import them (an export of Notes, `.nbk`). Only names and counts are
//! read, never a note's text, and only a little way down.

use std::fs;
use std::path::{Path, PathBuf};

/// What was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Notsy's own folder from an earlier install.
    Notsy,
    Obsidian,
    Logseq,
    /// A folder of Markdown notes no app marked as its own.
    Markdown,
    /// An export of Notes (github.com/nuttyartist/notes): imported.
    NotesExport,
}

impl Kind {
    /// Opened as it is (a folder of Markdown), not imported.
    pub fn opens_in_place(self) -> bool {
        !matches!(self, Kind::NotesExport)
    }

    /// What the installer calls it.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Notsy => "Your Notsy notes",
            Kind::Obsidian => "Obsidian vault",
            Kind::Logseq => "Logseq graph",
            Kind::Markdown => "Markdown notes",
            Kind::NotesExport => "Notes export",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub kind: Kind,
    pub path: PathBuf,
    /// Notes in it (Markdown files), counted up to `MOST_COUNTED`; 0 for an
    /// export.
    pub notes: usize,
}

/// Found at most, besides Notsy's own folder.
pub const MOST: usize = 3;
/// Notes counted at most in one folder.
pub const MOST_COUNTED: usize = 9_999;
/// A folder needs this many notes to count as notes, when no app marked it.
const FEWEST: usize = 5;
/// How deep notes are counted, and how deep a vault may sit.
const DEPTH: usize = 4;

/// Notes in `documents` and the folders right inside it, best first: Notsy's
/// own folder, then vaults and graphs, then other Markdown folders, the
/// bigger first, then exports.
pub fn in_documents(documents: &Path) -> Vec<Found> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(documents) else { return found };
    let mut dirs = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            dirs.push((name, path));
        } else if kind.is_file() && path.extension().is_some_and(|x| x.eq_ignore_ascii_case("nbk")) {
            found.push(Found { kind: Kind::NotesExport, path, notes: 0 });
        }
    }
    dirs.sort();
    for (name, dir) in dirs {
        if name.eq_ignore_ascii_case("Notsy") {
            found.push(Found { kind: Kind::Notsy, notes: count_notes(&dir, DEPTH), path: dir });
            continue;
        }
        if let Some(kind) = marked(&dir) {
            found.push(Found { kind, notes: count_notes(&dir, DEPTH), path: dir });
            continue;
        }
        // A folder of vaults (Documents\Obsidian\Work), one level down.
        let mut inner = false;
        if let Ok(children) = fs::read_dir(&dir) {
            for c in children.flatten().filter(|c| c.file_type().is_ok_and(|t| t.is_dir())) {
                if let Some(kind) = marked(&c.path()) {
                    found.push(Found { kind, notes: count_notes(&c.path(), DEPTH), path: c.path() });
                    inner = true;
                }
            }
        }
        if !inner {
            let notes = count_notes(&dir, 2);
            if notes >= FEWEST {
                found.push(Found { kind: Kind::Markdown, notes, path: dir });
            }
        }
    }
    found.sort_by(|a, b| a.kind.cmp(&b.kind).then(b.notes.cmp(&a.notes)));
    let own = found.iter().filter(|f| f.kind == Kind::Notsy).count();
    found.truncate(own + MOST);
    found
}

/// The app a folder belongs to, by the folder it keeps inside.
fn marked(dir: &Path) -> Option<Kind> {
    if dir.join(".obsidian").is_dir() {
        Some(Kind::Obsidian)
    } else if dir.join("logseq").is_dir() && dir.join("pages").is_dir() {
        Some(Kind::Logseq)
    } else {
        None
    }
}

/// Markdown files in `dir`, `depth` folders down at most (hidden folders
/// skipped), up to `MOST_COUNTED`.
pub fn count_notes(dir: &Path, depth: usize) -> usize {
    let mut n = 0;
    let mut todo = vec![(dir.to_path_buf(), 0)];
    while let Some((d, level)) = todo.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let Ok(t) = e.file_type() else { continue };
            let name = e.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            if t.is_dir() && level + 1 < depth {
                todo.push((e.path(), level + 1));
            } else if t.is_file() && e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("md")) {
                n += 1;
                if n >= MOST_COUNTED {
                    return n;
                }
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes(dir: &Path, n: usize) {
        fs::create_dir_all(dir).unwrap();
        for i in 0..n {
            fs::write(dir.join(format!("{i}.md")), "x").unwrap();
        }
    }

    #[test]
    fn finds_other_apps_notes_in_documents_best_first() {
        let docs = std::env::temp_dir().join(format!("notsy-found-{}", std::process::id()));
        let _ = fs::remove_dir_all(&docs);
        notes(&docs.join("Work"), 3);
        fs::create_dir_all(docs.join("Work").join(".obsidian")).unwrap();
        notes(&docs.join("Obsidian").join("Personal").join("Daily"), 12);
        fs::create_dir_all(docs.join("Obsidian").join("Personal").join(".obsidian")).unwrap();
        notes(&docs.join("Graph").join("pages"), 4);
        fs::create_dir_all(docs.join("Graph").join("logseq")).unwrap();
        notes(&docs.join("Writing"), 6);
        notes(&docs.join("Two notes"), 2);
        notes(&docs.join(".hidden"), 9);
        fs::create_dir_all(docs.join("Photos")).unwrap();
        fs::write(docs.join("old.nbk"), b"SQLite").unwrap();
        let found = in_documents(&docs);
        let got: Vec<(Kind, String, usize)> = found.iter().map(|f| (f.kind, f.path.strip_prefix(&docs).unwrap().display().to_string(), f.notes)).collect();
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            got,
            [(Kind::Obsidian, format!("Obsidian{sep}Personal"), 12), (Kind::Obsidian, "Work".into(), 3), (Kind::Logseq, "Graph".into(), 4)],
            "vaults first, the bigger first; at most {MOST}"
        );
        // Notsy's own folder from before comes first, beside the others.
        notes(&docs.join("Notsy"), 1);
        let again = in_documents(&docs);
        assert_eq!((again[0].kind, again.len()), (Kind::Notsy, 1 + MOST));
        fs::remove_dir_all(&docs).unwrap();
    }

    #[test]
    fn markdown_folders_and_exports_count_when_there_is_room() {
        let docs = std::env::temp_dir().join(format!("notsy-found-md-{}", std::process::id()));
        let _ = fs::remove_dir_all(&docs);
        notes(&docs.join("Writing"), 6);
        notes(&docs.join("Few"), 4);
        fs::write(docs.join("notes.nbk"), b"SQLite").unwrap();
        let found = in_documents(&docs);
        assert_eq!(found.iter().map(|f| f.kind).collect::<Vec<_>>(), [Kind::Markdown, Kind::NotesExport]);
        assert!(found[0].kind.opens_in_place() && !found[1].kind.opens_in_place());
        assert!(in_documents(&docs.join("missing")).is_empty());
        fs::remove_dir_all(&docs).unwrap();
    }
}
