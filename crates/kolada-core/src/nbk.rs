//! Notes exported by Notes (github.com/nuttyartist/notes) as `.nbk`: a
//! SQLite database of folders and notes, each note with when it was made
//! and last changed. Read into notes to write as Markdown files, dated as
//! they were, so they keep their order.

use std::collections::HashMap;

use crate::sqlite::{Database, Error, Value};

/// The app's own folders, and its kinds of node.
const ROOT: i64 = 0;
const TRASH: i64 = 1;
const DEFAULT: i64 = 2;
const NOTE: i64 = 0;
const FOLDER: i64 = 1;
/// The title the app gives a note nothing is written in.
const PLACEHOLDER: &str = "New Note";

/// One note to import.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// The folders it is in, outermost first; empty for the app's own
    /// "Notes".
    pub folders: Vec<String>,
    /// Its Markdown. A note with nothing written under its title is its
    /// title.
    pub text: String,
    /// Milliseconds since 1970 (UTC), when it was made and last changed.
    pub created: i64,
    pub modified: i64,
}

/// What an export holds: its notes, oldest first, and how many were left
/// out for being in the trash or empty.
#[derive(Debug, Default)]
pub struct Export {
    pub notes: Vec<Note>,
    pub trashed: usize,
    pub empty: usize,
}

pub fn read(bytes: &[u8]) -> Result<Export, Error> {
    let db = Database::open(bytes)?;
    let nodes = db.table("node_table")?;
    let col = |name: &str| nodes.column(name).ok_or_else(|| Error(format!("not a Notes export: no {name} column")));
    let (id, title, kind, parent) = (col("id")?, col("title")?, col("node_type")?, col("parent_id")?);
    let (content, created, modified, deleted) = (col("content")?, col("creation_date")?, col("modification_date")?, col("deletion_date")?);
    let int = |row: &[Value], c: usize| row.get(c).and_then(Value::as_i64).unwrap_or(0);
    let text = |row: &[Value], c: usize| row.get(c).and_then(Value::as_str).unwrap_or("").to_string();

    // Folders: their names and where they are.
    let folders: HashMap<i64, (String, i64)> =
        nodes.rows.iter().filter(|r| int(r, kind) == FOLDER).map(|r| (int(r, id), (text(r, title), int(r, parent)))).collect();
    let tags = tags(&db);

    let mut export = Export::default();
    for row in nodes.rows.iter().filter(|r| int(r, kind) == NOTE) {
        let Some(path) = folder_path(&folders, int(row, parent)) else {
            export.trashed += 1;
            continue;
        };
        if int(row, deleted) > 0 {
            export.trashed += 1;
            continue;
        }
        let (title, mut body) = (text(row, title), text(row, content));
        if body.trim().is_empty() {
            if title.trim().is_empty() || title.trim() == PLACEHOLDER {
                export.empty += 1;
                continue;
            }
            body = title;
        }
        if let Some(names) = tags.get(&int(row, id)) {
            body = format!("{}\n\n{}", body.trim_end(), names.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "));
        }
        let text = body.replace("\r\n", "\n");
        export.notes.push(Note { folders: path, text, created: int(row, created), modified: int(row, modified) });
    }
    export.notes.sort_by_key(|n| n.created);
    Ok(export)
}

/// The names of the folders from the top down to `folder`, without the
/// app's root and its "Notes"; None if it is in the trash.
fn folder_path(folders: &HashMap<i64, (String, i64)>, mut folder: i64) -> Option<Vec<String>> {
    let mut names = Vec::new();
    // A damaged file could loop.
    for _ in 0..64 {
        match folder {
            TRASH => return None,
            ROOT | -1 => break,
            DEFAULT => {}
            _ => match folders.get(&folder) {
                Some((name, _)) => names.push(name.clone()),
                None => break,
            },
        }
        folder = folders.get(&folder).map_or(ROOT, |f| f.1);
    }
    names.reverse();
    Some(names)
}

/// Each note's tags, as Markdown tags (spaces become dashes); none if the
/// export has no tags.
fn tags(db: &Database) -> HashMap<i64, Vec<String>> {
    let (Ok(names), Ok(links)) = (db.table("tag_table"), db.table("tag_relationship")) else { return HashMap::new() };
    let (Some(id), Some(name)) = (names.column("id"), names.column("name")) else { return HashMap::new() };
    let (Some(node), Some(tag)) = (links.column("node_id"), links.column("tag_id")) else { return HashMap::new() };
    let named: HashMap<i64, String> = names
        .rows
        .iter()
        .filter_map(|r| {
            let n = r.get(name)?.as_str()?.trim().trim_start_matches('#').split_whitespace().collect::<Vec<_>>().join("-");
            Some((r.get(id)?.as_i64()?, n)).filter(|(_, n)| !n.is_empty())
        })
        .collect();
    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    for r in &links.rows {
        if let (Some(n), Some(t)) = (r.get(node).and_then(Value::as_i64), r.get(tag).and_then(Value::as_i64))
            && let Some(name) = named.get(&t)
        {
            out.entry(n).or_default().push(name.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// See testdata/make.py.
    const EXPORT: &[u8] = include_bytes!("../testdata/notes.nbk");
    const T0: i64 = 1_665_419_890_626;
    const DAY: i64 = 86_400_000;

    #[test]
    fn reads_notes_with_their_folders_and_dates() {
        let export = read(EXPORT).unwrap();
        let got: Vec<(Vec<&str>, &str, i64, i64)> =
            export.notes.iter().map(|n| (n.folders.iter().map(String::as_str).collect(), n.text.as_str(), n.created, n.modified)).collect();
        assert_eq!(
            got,
            [
                (vec![], "Groceries\n- milk\n- bread", T0, T0 + DAY),
                (vec!["Work"], "# What: now?\n**Soon**", T0 + 2 * DAY, T0 + 30 * DAY),
                (vec!["Work", "Plans: 2026"], "Launch\nthe plan", T0 + 3 * DAY, T0 + 3 * DAY),
                (vec![], "Only a title, nothing written under it", T0 + 5 * DAY, T0 + 6 * DAY),
                (vec![], "Tagged\nwith two\n\n#reading-list #ideas", T0 + 6 * DAY, T0 + 6 * DAY),
                (vec![], "Groceries\nagain, the week after", T0 + 7 * DAY, T0 + 8 * DAY),
            ]
        );
        assert_eq!((export.trashed, export.empty), (1, 1), "the trash and the placeholder are left out");
    }

    #[test]
    fn refuses_other_databases() {
        let other = include_bytes!("../testdata/sqlite.db");
        assert!(read(other).is_err());
        assert!(read(b"# just markdown").is_err());
    }
}
