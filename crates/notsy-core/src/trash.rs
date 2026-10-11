//! Deleted notes wait in the vault's `.trash` folder, where Obsidian puts
//! them too, until they are restored or deleted for good. A small list in
//! the app's data (not the vault) remembers where each came from and when it
//! was deleted.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::note;

pub const FOLDER: &str = ".trash";

/// A note in the trash.
#[derive(Clone, Debug, PartialEq)]
pub struct Trashed {
    /// Its file now, in the trash folder.
    pub file: PathBuf,
    /// Where it was, relative to the vault, with `/` between folders.
    pub from: String,
    /// When it was deleted, seconds since the Unix epoch.
    pub at: u64,
}

impl Trashed {
    /// The note's name: its file name without `.md`.
    pub fn name(&self) -> String {
        self.file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// The list's lines: vault, file name in the trash, where it was, when.
fn records(list: &Path) -> Vec<(String, String, String, u64)> {
    let text = fs::read_to_string(list).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (vault, file, from, at) = (f.next()?, f.next()?, f.next()?, f.next()?);
            Some((vault.to_string(), file.to_string(), from.to_string(), at.parse().ok()?))
        })
        .collect()
}

fn write_records(list: &Path, records: &[(String, String, String, u64)]) -> io::Result<()> {
    let text: String = records.iter().map(|(v, f, from, at)| format!("{v}\t{f}\t{from}\t{at}\n")).collect();
    if let Some(dir) = list.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(list, text)
}

fn key(vault: &Path) -> String {
    vault.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase()
}

/// Moves the vault's note at `rel` into the trash, noting it in `list`.
pub fn trash(vault: &Path, rel: &str, list: &Path, now: u64) -> io::Result<Trashed> {
    let path = vault.join(rel);
    let dir = vault.join(FOLDER);
    fs::create_dir_all(&dir)?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let file = note::unique_path(&dir, &stem, None);
    fs::rename(&path, &file)?;
    let name = file.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut all = records(list);
    all.retain(|r| !(r.0 == key(vault) && r.1 == name));
    all.push((key(vault), name, rel.to_string(), now));
    write_records(list, &all)?;
    Ok(Trashed { file, from: rel.to_string(), at: now })
}

/// The vault's trash, most recently deleted first. Notes put there by
/// another app were at the vault's top, as far as anyone knows.
pub fn list(vault: &Path, list: &Path) -> Vec<Trashed> {
    let all = records(list);
    let Ok(entries) = fs::read_dir(vault.join(FOLDER)) else { return Vec::new() };
    let mut out: Vec<Trashed> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.to_lowercase().ends_with(".md") {
                return None;
            }
            let record = all.iter().find(|r| r.0 == key(vault) && r.1 == name);
            let (from, at) = match record {
                Some(r) => (r.2.clone(), r.3),
                None => {
                    let modified = e.metadata().and_then(|m| m.modified()).ok();
                    let at = modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                    (name.clone(), at)
                }
            };
            Some(Trashed { file: e.path(), from, at })
        })
        .collect();
    out.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.from.cmp(&b.from)));
    out
}

/// Puts a note back where it was (beside a note that took its name since,
/// under a new number). Returns where it went.
pub fn restore(vault: &Path, item: &Trashed, list: &Path) -> io::Result<PathBuf> {
    let wanted = vault.join(&item.from);
    let dir = wanted.parent().unwrap_or(vault).to_path_buf();
    fs::create_dir_all(&dir)?;
    let stem = wanted.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let path = note::unique_path(&dir, &stem, None);
    fs::rename(&item.file, &path)?;
    forget(vault, item, list);
    Ok(path)
}

/// Drops a note's line from the list (it left the trash).
pub fn forget(vault: &Path, item: &Trashed, list: &Path) {
    let name = item.file.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut all = records(list);
    let before = all.len();
    all.retain(|r| !(r.0 == key(vault) && r.1 == name));
    if all.len() != before {
        let _ = write_records(list, &all);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("notsy-trash-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Work")).unwrap();
        dir
    }

    #[test]
    fn deleted_notes_come_back_where_they_were() {
        let vault = temp("roundtrip");
        let log = vault.join("list.tsv");
        fs::write(vault.join("Work/Plan.md"), "the plan").unwrap();
        let item = trash(&vault, "Work/Plan.md", &log, 100).unwrap();
        assert!(!vault.join("Work/Plan.md").exists());
        assert_eq!(item.file, vault.join(".trash/Plan.md"));
        assert_eq!(list_of(&vault, &log), vec![("Work/Plan.md".to_string(), 100)]);
        // Its folder went away meanwhile: it comes back anyway.
        fs::remove_dir_all(vault.join("Work")).unwrap();
        let back = restore(&vault, &item, &log).unwrap();
        assert_eq!(back, vault.join("Work/Plan.md"));
        assert_eq!(fs::read_to_string(&back).unwrap(), "the plan");
        assert!(list_of(&vault, &log).is_empty());
        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn same_names_and_foreign_files_get_along() {
        let vault = temp("names");
        let log = vault.join("list.tsv");
        for (rel, at) in [("Plan.md", 1), ("Work/Plan.md", 2)] {
            fs::write(vault.join(rel), rel).unwrap();
            trash(&vault, rel, &log, at).unwrap();
        }
        // Obsidian put one there too.
        fs::write(vault.join(".trash/Old.md"), "").unwrap();
        let items = list(&vault, &log);
        let names: Vec<String> = items.iter().map(Trashed::name).collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"Plan".to_string()) && names.contains(&"Plan 2".to_string()));
        assert!(items.iter().any(|t| t.from == "Old.md"));
        // A note took the name meanwhile: the restored one gets a number.
        fs::write(vault.join("Plan.md"), "new").unwrap();
        let first = items.iter().find(|t| t.from == "Plan.md").unwrap();
        assert_eq!(restore(&vault, first, &log).unwrap(), vault.join("Plan 2.md"));
        let _ = fs::remove_dir_all(&vault);
    }

    fn list_of(vault: &Path, l: &Path) -> Vec<(String, u64)> {
        list(vault, l).into_iter().map(|t| (t.from, t.at)).collect()
    }
}
