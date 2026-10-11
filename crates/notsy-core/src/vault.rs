//! Vault helpers. A vault is a plain folder of Markdown files, compatible
//! with Obsidian.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::session::{RowKind, SidebarRow};

/// Folders that are never part of the note tree.
fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// The vault a note belongs to: the nearest ancestor holding `.obsidian`,
/// or the note's own folder.
pub fn root_for(note: &Path) -> PathBuf {
    let parent = note.parent().unwrap_or(Path::new("."));
    parent
        .ancestors()
        .find(|dir| dir.join(".obsidian").is_dir())
        .unwrap_or(parent)
        .to_path_buf()
}

/// `note` relative to `vault`, with forward slashes.
pub fn relative(vault: &Path, note: &Path) -> Option<String> {
    let rel = note.strip_prefix(vault).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// One folder's entries: its folders first, alphabetical and collapsed;
/// then its notes, the one written last first (alphabetical where two were
/// written at the same moment), so what was just edited is at the top.
fn list(dir: &Path, depth: u8) -> io::Result<Vec<SidebarRow>> {
    let mut rows = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_hidden(&name) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            rows.push((SidebarRow { depth, kind: RowKind::Folder { expanded: false }, name }, None));
        } else if let Some(stem) = name.strip_suffix(".md") {
            // On Windows the time comes with the listing: no further look.
            let written = entry.metadata().and_then(|m| m.modified()).ok();
            rows.push((SidebarRow { depth, kind: RowKind::Note, name: stem.to_string() }, written));
        }
    }
    let by_name = |a: &SidebarRow, b: &SidebarRow| a.name.to_lowercase().cmp(&b.name.to_lowercase());
    rows.sort_by(|(a, at), (b, bt)| match (a.kind, b.kind) {
        (RowKind::Folder { .. }, RowKind::Note) => Ordering::Less,
        (RowKind::Note, RowKind::Folder { .. }) => Ordering::Greater,
        (RowKind::Note, RowKind::Note) => bt.cmp(at).then_with(|| by_name(a, b)),
        _ => by_name(a, b),
    });
    Ok(rows.into_iter().map(|(row, _)| row).collect())
}

/// The top level of the file tree.
pub fn top_level_rows(vault: &Path) -> io::Result<Vec<SidebarRow>> {
    list(vault, 0)
}

/// The entries of `folder` (relative to the vault), one level deeper.
pub fn children(vault: &Path, folder: &str, depth: u8) -> io::Result<Vec<SidebarRow>> {
    list(&vault.join(folder), depth)
}

/// The whole visible tree: the top level, plus the contents of every folder
/// in `expanded` (vault-relative paths), recursively.
pub fn tree_rows(vault: &Path, expanded: &HashSet<String>) -> io::Result<Vec<SidebarRow>> {
    fn walk(vault: &Path, prefix: &str, depth: u8, expanded: &HashSet<String>, out: &mut Vec<SidebarRow>) -> io::Result<()> {
        for mut row in list(&vault.join(prefix), depth)? {
            let path = if prefix.is_empty() { row.name.clone() } else { format!("{prefix}/{}", row.name) };
            let open = matches!(row.kind, RowKind::Folder { .. }) && expanded.contains(&path);
            if open {
                row.kind = RowKind::Folder { expanded: true };
            }
            out.push(row);
            if open {
                walk(vault, &path, depth + 1, expanded, out)?;
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(vault, "", 0, expanded, &mut out)?;
    Ok(out)
}

/// Each row's vault-relative path (notes without `.md`), derived from the
/// depth-ordered tree.
pub fn row_paths(rows: &[SidebarRow]) -> Vec<String> {
    let mut stack: Vec<&str> = Vec::new();
    rows.iter()
        .map(|row| {
            stack.truncate(row.depth as usize);
            let path = stack.iter().copied().chain([row.name.as_str()]).collect::<Vec<_>>().join("/");
            stack.push(&row.name);
            path
        })
        .collect()
}

/// A note was just written: its row (`note`: vault-relative, without
/// `.md`) goes first among its folder's notes, where a fresh listing puts
/// it (see `list`). False if it was there already or is not shown.
pub fn written(rows: &mut Vec<SidebarRow>, note: &str) -> bool {
    let paths = row_paths(rows);
    let Some(i) = paths.iter().position(|p| p == note) else { return false };
    let depth = rows[i].depth;
    // Up through its folder (past what open folders in it hold) to the
    // first of its notes.
    let mut top = i;
    for j in (0..i).rev() {
        if rows[j].depth < depth {
            break;
        }
        if rows[j].depth == depth && rows[j].kind == RowKind::Note {
            top = j;
        }
    }
    if top == i {
        return false;
    }
    let row = rows.remove(i);
    rows.insert(top, row);
    true
}

/// Paths of the folders that are open in `rows`.
pub fn expanded_folders(rows: &[SidebarRow]) -> HashSet<String> {
    row_paths(rows)
        .into_iter()
        .zip(rows)
        .filter(|(_, r)| r.kind == RowKind::Folder { expanded: true })
        .map(|(p, _)| p)
        .collect()
}

/// Closes the folder at `i`, removing everything nested under it.
pub fn collapse(rows: &mut Vec<SidebarRow>, i: usize) {
    let depth = rows[i].depth;
    let end = rows[i + 1..].iter().position(|r| r.depth <= depth).map_or(rows.len(), |n| i + 1 + n);
    rows.drain(i + 1..end);
    rows[i].kind = RowKind::Folder { expanded: false };
}

/// Opens the folder at `i`, inserting its `children` (one level deeper).
pub fn expand(rows: &mut Vec<SidebarRow>, i: usize, children: Vec<SidebarRow>) {
    rows[i].kind = RowKind::Folder { expanded: true };
    rows.splice(i + 1..i + 1, children);
}

/// A note to open when nothing else is known: the one at the top level
/// written last.
pub fn first_note(vault: &Path) -> Option<String> {
    top_level_rows(vault)
        .ok()?
        .into_iter()
        .find(|r| r.kind == RowKind::Note)
        .map(|r| format!("{}.md", r.name))
}

/// The note a `[[link]]` points to, as Obsidian resolves it: a path from the
/// vault's root (`Folder/Note`), or else a note of that name in any folder,
/// ignoring case. Hidden folders are skipped.
pub fn find_note(vault: &Path, link: &str) -> Option<PathBuf> {
    let link = link.trim().trim_end_matches(".md");
    if link.is_empty() {
        return None;
    }
    let direct = vault.join(format!("{link}.md"));
    if direct.is_file() {
        return Some(direct);
    }
    let wanted = format!("{}.md", link.rsplit('/').next().unwrap_or(link)).to_lowercase();
    let mut dirs = vec![vault.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_hidden(&name) {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => dirs.push(entry.path()),
                Ok(_) if name.to_lowercase() == wanted => return Some(entry.path()),
                _ => {}
            }
        }
    }
    None
}

/// The first free "Untitled.md", "Untitled 1.md", ... at the vault's top
/// level, as Obsidian names new notes.
pub fn new_note_name(vault: &Path) -> String {
    (0..)
        .map(|n| if n == 0 { "Untitled.md".to_string() } else { format!("Untitled {n}.md") })
        .find(|name| !vault.join(name).exists())
        .expect("an unused name exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("notsy-vault-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_folders_first_and_skips_hidden() {
        let dir = temp_vault("list");
        for d in ["zeta", "Alpha", ".git", ".smart-env"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        // Notes written an hour apart, oldest first; two at the same moment.
        let base = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_790_000_000);
        for (k, f) in ["b note.md", "A note.md", "image.png", "c same.md", "C same 2.md"].into_iter().enumerate() {
            fs::write(dir.join(f), "").unwrap();
            let at = base + std::time::Duration::from_secs(3600 * k.min(3) as u64);
            fs::File::options().write(true).open(dir.join(f)).unwrap().set_modified(at).unwrap();
        }
        let rows = top_level_rows(&dir).unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zeta", "c same", "C same 2", "A note", "b note"]);
        // A note written again goes to the top.
        fs::File::options().write(true).open(dir.join("b note.md")).unwrap().set_modified(base + std::time::Duration::from_secs(9 * 3600)).unwrap();
        assert_eq!(top_level_rows(&dir).unwrap()[2].name, "b note");
        assert_eq!(first_note(&dir).as_deref(), Some("b note.md"));
        assert_eq!(root_for(&dir.join("Alpha").join("x.md")), dir.join("Alpha"));
        fs::create_dir_all(dir.join(".obsidian")).unwrap();
        assert_eq!(root_for(&dir.join("Alpha").join("x.md")), dir);
        assert_eq!(relative(&dir, &dir.join("Alpha").join("x.md")).as_deref(), Some("Alpha/x.md"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn expands_folders_recursively_and_tracks_paths() {
        let dir = temp_vault("tree");
        fs::create_dir_all(dir.join("Work/Clients")).unwrap();
        fs::create_dir_all(dir.join("Home")).unwrap();
        fs::write(dir.join("Work/plan.md"), "").unwrap();
        fs::write(dir.join("Work/Clients/acme.md"), "").unwrap();
        fs::write(dir.join("Home/list.md"), "").unwrap();
        fs::write(dir.join("inbox.md"), "").unwrap();

        let expanded: HashSet<String> = ["Work".to_string(), "Work/Clients".to_string()].into();
        let rows = tree_rows(&dir, &expanded).unwrap();
        let paths = row_paths(&rows);
        assert_eq!(paths, ["Home", "Work", "Work/Clients", "Work/Clients/acme", "Work/plan", "inbox"]);
        let depths: Vec<u8> = rows.iter().map(|r| r.depth).collect();
        assert_eq!(depths, [0, 0, 1, 2, 1, 0]);
        assert_eq!(expanded_folders(&rows), expanded);
        assert_eq!(children(&dir, "Home", 1).unwrap()[0].name, "list");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_note_just_written_goes_first_among_its_folders_notes() {
        let row = |depth, kind, name: &str| SidebarRow { depth, kind, name: name.into() };
        let note = RowKind::Note;
        let mut rows = vec![
            row(0, RowKind::Folder { expanded: true }, "Work"),
            row(1, note, "plan"),
            row(1, note, "budget"),
            row(0, note, "inbox"),
            row(0, note, "ideas"),
        ];
        assert!(written(&mut rows, "ideas"));
        assert_eq!(row_paths(&rows), ["Work", "Work/plan", "Work/budget", "ideas", "inbox"]);
        assert!(written(&mut rows, "Work/budget"));
        assert_eq!(row_paths(&rows), ["Work", "Work/budget", "Work/plan", "ideas", "inbox"]);
        // First already, or not shown: nothing moves.
        assert!(!written(&mut rows, "ideas"));
        assert!(!written(&mut rows, "Home/list"));
    }

    #[test]
    fn collapses_and_expands_folders_in_place() {
        let row = |depth, kind, name: &str| SidebarRow { depth, kind, name: name.into() };
        let open = RowKind::Folder { expanded: true };
        let mut rows = vec![
            row(0, open, "Work"),
            row(1, open, "Clients"),
            row(2, RowKind::Note, "acme"),
            row(1, RowKind::Note, "plan"),
            row(0, RowKind::Note, "inbox"),
        ];
        collapse(&mut rows, 1);
        assert_eq!(row_paths(&rows), ["Work", "Work/Clients", "Work/plan", "inbox"]);
        collapse(&mut rows, 0);
        assert_eq!(row_paths(&rows), ["Work", "inbox"]);
        expand(&mut rows, 0, vec![row(1, RowKind::Note, "plan")]);
        assert_eq!(row_paths(&rows), ["Work", "Work/plan", "inbox"]);
        assert_eq!(rows[0].kind, open);
        // Collapsing the last folder removes rows up to the end.
        let mut tail = vec![row(0, open, "A"), row(1, RowKind::Note, "x")];
        collapse(&mut tail, 0);
        assert_eq!(tail.len(), 1);
    }

    #[test]
    fn resolves_links_by_path_or_name() {
        let dir = temp_vault("links");
        fs::create_dir_all(dir.join("Work/Deep")).unwrap();
        fs::create_dir_all(dir.join(".trash")).unwrap();
        fs::write(dir.join("Work/Deep/Plan.md"), "").unwrap();
        fs::write(dir.join(".trash/Gone.md"), "").unwrap();
        fs::write(dir.join("Inbox.md"), "").unwrap();
        assert_eq!(find_note(&dir, "Inbox"), Some(dir.join("Inbox.md")));
        assert_eq!(find_note(&dir, "plan"), Some(dir.join("Work").join("Deep").join("Plan.md")));
        assert_eq!(find_note(&dir, "Work/Deep/Plan"), Some(dir.join("Work/Deep/Plan.md")));
        assert_eq!(find_note(&dir, "Gone"), None, "hidden folders are not part of the vault");
        assert_eq!(find_note(&dir, "Missing"), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_new_notes_like_obsidian() {
        let dir = temp_vault("new");
        assert_eq!(new_note_name(&dir), "Untitled.md");
        fs::write(dir.join("Untitled.md"), "").unwrap();
        fs::write(dir.join("Untitled 1.md"), "").unwrap();
        assert_eq!(new_note_name(&dir), "Untitled 2.md");
        fs::remove_dir_all(&dir).unwrap();
    }
}
