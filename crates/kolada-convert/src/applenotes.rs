//! Apple Notes' notes, as Notes itself gives them through its scripting
//! dictionary: the shell runs [`SCRIPT`] with `osascript` (the user is
//! asked once whether Kolada may control Notes), it writes each note's
//! folders, name, dates and body as HTML into a folder, and [`each`] reads
//! them one note at a time (so a library of photos is never held at once),
//! all of them, or as many as came when Notes stopped part way. Notes in
//! Notes' bin, Recently Deleted, are left out, whatever language Notes
//! speaks. Pictures inside a note come with it
//! (Notes puts them in the HTML); its other attachments stay in Notes and
//! are counted. Locked notes are never read: Notes says which they are, and
//! they are counted too.
//!
//! Notes' own library is never opened: its format is undocumented, changes
//! with macOS, and reading it would need Full Disk Access.

use std::path::Path;

use crate::import::{Collection, Entry, file_name_for, unique_in};
use crate::{Error, Picture, html};

/// The script, for `osascript -l JavaScript <script> <folder>`.
pub const SCRIPT: &str = include_str!("applenotes.js");

/// The folder the notes go into.
pub const FOLDER: &str = "From Apple Notes";

/// Between a record's fields, a folder path's names, and records.
const FIELD: char = '\u{1F}';
const PATH: char = '\u{1D}';
const RECORD: char = '\u{1E}';

/// Notes' bin, by the name Notes gives it in each language it speaks
/// (compared without case). Its notes are not brought in.
const DELETED: &[&str] = &[
    "recently deleted",
    "zuletzt gelöscht",
    "suppressions récentes",
    "récemment supprimées",
    "récemment supprimés",
    "eliminado recientemente",
    "eliminados recientemente",
    "eliminadas recientemente",
    "eliminati di recente",
    "eliminate di recente",
    "apagados recentemente",
    "apagadas recentemente",
    "eliminados recentemente",
    "eliminadas recentemente",
    "recent verwijderd",
    "nyligen raderade",
    "for nylig slettet",
    "nylig slettet",
    "äskettäin poistetut",
    "ostatnio usunięte",
    "nedávno smazané",
    "nedávno odstránené",
    "nemrég törölt",
    "șterse recent",
    "nedavno izbrisano",
    "недавно удалённые",
    "недавно удаленные",
    "нещодавно видалені",
    "πρόσφατα διαγραμμένα",
    "son silinenler",
    "المحذوفة مؤخرًا",
    "المحذوفة مؤخراً",
    "נמחקו לאחרונה",
    "हाल ही में डिलीट किए गए",
    "ที่ลบล่าสุด",
    "đã xóa gần đây",
    "baru saja dihapus",
    "dipadam baru-baru ini",
    "最近削除した項目",
    "最近删除",
    "最近刪除",
    "최근 삭제된 항목",
];

/// Whether a folder of this name is Notes' bin.
pub fn is_deleted_folder(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    DELETED.iter().any(|d| *d == name)
}

/// What came from Notes, beside the notes themselves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Empty notes, and notes in Notes' bin, left out.
    pub skipped: usize,
    pub deleted: usize,
    /// Left in Notes: locked notes, notes Notes would not give (or that
    /// could not be saved for Kolada to read), attachments other than
    /// pictures.
    pub locked: usize,
    pub unreadable: usize,
    pub attachments_left: usize,
    /// Notes stopped part way: the notes it never got to.
    pub stopped: bool,
    pub not_reached: usize,
}

/// Counts in one of the script's small files: its fields after the version.
fn counts(dir: &Path, name: &str) -> Option<Vec<usize>> {
    let text = std::fs::read_to_string(dir.join(name)).ok()?;
    Some(text.split(FIELD).skip(1).map(|n| n.trim().parse().unwrap_or(0)).collect())
}

/// The notes the script wrote into `dir`, handed to `take` one at a time in
/// Notes' order, each read, converted and let go before the next. Gaps are
/// read past. An export that did not run to its end (no `done.txt`: Notes
/// stopped answering, or quit) still gives the notes that came, `stopped`
/// set and the notes it never reached counted (from `total.txt`); one that
/// did, but has fewer notes than it says it saved, counts the rest as
/// unreadable. Nothing come at all is an error.
pub fn each(dir: &Path, mut take: impl FnMut(Entry)) -> Result<Tally, Error> {
    let done = counts(dir, "done.txt");
    let total = counts(dir, "total.txt").unwrap_or_default();
    let many = total.get(2).copied().unwrap_or(0) > 1;
    let mut files: Vec<(usize, std::path::PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| Error(e.to_string()))?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix("note-")?.strip_suffix(".txt")?.parse().ok()?;
            Some((n, e.path()))
        })
        .collect();
    files.sort();
    if done.is_none() && files.is_empty() {
        return Err(Error("Notes stopped before it gave a note".to_string()));
    }
    let mut t = Tally::default();
    for (_, path) in &files {
        let Ok(record) = std::fs::read_to_string(path) else {
            t.unreadable += 1;
            continue;
        };
        match note(&record, many) {
            Read::Note(e, left) => {
                t.attachments_left += left;
                take(e);
            }
            Read::Deleted => t.deleted += 1,
            Read::Empty => t.skipped += 1,
        }
    }
    let came = files.len();
    match done {
        Some(d) => {
            let saved = d.first().copied().unwrap_or(came);
            t.locked = d.get(1).copied().unwrap_or(0);
            t.unreadable += d.get(2).copied().unwrap_or(0) + saved.saturating_sub(came);
        }
        None => {
            t.stopped = true;
            t.locked = total.get(1).copied().unwrap_or(0);
            t.not_reached = total.first().copied().unwrap_or(0).saturating_sub(came);
        }
    }
    Ok(t)
}

/// The notes the script wrote into `dir`, all at once (see [`each`]).
pub fn read_dir(dir: &Path) -> Result<Collection, Error> {
    let mut c = Collection::default();
    let t = each(dir, |e| c.notes.push(e))?;
    c.skipped = t.skipped + t.deleted;
    c.locked = t.locked;
    c.unreadable = t.unreadable;
    c.attachments_left = t.attachments_left;
    c.stopped = t.stopped;
    c.not_reached = t.not_reached;
    Ok(c)
}

/// Records as the script writes them, one after another.
pub fn read(bytes: &[u8]) -> Result<Collection, Error> {
    let text = String::from_utf8_lossy(bytes);
    let mut c = Collection::default();
    for record in text.split(RECORD).filter(|r| !r.trim().is_empty()) {
        match note(record, false) {
            Read::Note(e, left) => {
                c.attachments_left += left;
                c.notes.push(e);
            }
            Read::Deleted | Read::Empty => c.skipped += 1,
        }
    }
    Ok(c)
}

/// Whether `osascript`'s complaint means the user (or macOS) did not let
/// Kolada control Notes: then it is asked for again, not reported as a
/// fault.
pub fn refused(stderr: &str) -> bool {
    stderr.contains("-1743") || stderr.contains("Not authorized to send Apple events") || stderr.contains("-10004")
}

/// What a record holds.
enum Read {
    /// The note, and how many of its attachments stayed in Notes.
    Note(Entry, usize),
    /// A note in Notes' bin.
    Deleted,
    Empty,
}

/// One record. With `many` accounts, a note's folders start with its
/// account's name.
fn note(record: &str, many: bool) -> Read {
    let f: Vec<&str> = record.split(FIELD).collect();
    let [_version, account, path, name, created, modified, attachments, body, ..] = f.as_slice() else { return Read::Empty };
    let mut folders: Vec<String> = path.split(PATH).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
    if folders.first().is_some_and(|f| is_deleted_folder(f)) {
        return Read::Deleted;
    }
    if many && !account.trim().is_empty() {
        folders.insert(0, account.trim().to_string());
    }
    let Ok(converted) = html::convert(checklists(body).as_bytes()) else { return Read::Empty };
    let title = name.trim();
    let title = if title.is_empty() { "Untitled".to_string() } else { title.to_string() };
    // Pictures named for their note, the note's embeds following.
    let mut files: Vec<Picture> = Vec::new();
    let mut markdown = converted.markdown;
    for p in converted.pictures {
        let ext = p.name.rsplit_once('.').map_or("png", |(_, e)| e).to_string();
        let name = unique_in(&files, &file_name_for(&format!("{title}.{ext}"), &ext));
        markdown = markdown.replace(&format!("![[{}]]", p.name), &format!("![[{name}]]"));
        files.push(Picture { name, data: p.data });
    }
    let markdown = with_title(&markdown, &title);
    if markdown.trim() == format!("# {title}") && name.trim().is_empty() {
        return Read::Empty;
    }
    let left = attachments.trim().parse::<usize>().unwrap_or(0).saturating_sub(files.len());
    let created = created.trim().parse().unwrap_or(0);
    let modified = modified.trim().parse().unwrap_or(0);
    Read::Note(Entry { folders, title, markdown, created, modified: if modified > 0 { modified } else { created }, files }, left)
}

/// The note's Markdown under its title as a heading. Notes makes a note's
/// first line its name, so that line becomes the heading rather than
/// showing twice.
fn with_title(markdown: &str, title: &str) -> String {
    let body = markdown.trim();
    let plain = |l: &str| l.trim_start_matches('#').replace(['*', '_'], "").trim().to_string();
    let rest = match body.split_once('\n') {
        Some((first, rest)) if plain(first) == title => rest.trim(),
        None if plain(body) == title => "",
        _ => body,
    };
    if rest.is_empty() { format!("# {title}\n") } else { format!("# {title}\n\n{rest}\n") }
}

/// Notes' checklists as the converter reads tasks: a checkbox at the start
/// of each item of a list Notes marks as a checklist, ticked where the item
/// is marked done. Lists without the marks stay lists.
fn checklists(body: &str) -> String {
    let lower = body.to_ascii_lowercase();
    if !lower.contains("checklist") && !lower.contains("checked") {
        return body.to_string();
    }
    let mut out = String::with_capacity(body.len() + 64);
    // Whether each open list is a checklist.
    let mut lists: Vec<bool> = Vec::new();
    let mut at = 0;
    while let Some(i) = body[at..].find('<') {
        let start = at + i;
        let Some(len) = body[start..].find('>') else { break };
        let end = start + len + 1;
        let tag = &lower[start..end];
        out.push_str(&body[at..end]);
        let name: String = tag[1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '/').collect();
        match name.as_str() {
            "ul" | "ol" => lists.push(tag.contains("checklist")),
            "/ul" | "/ol" => {
                lists.pop();
            }
            "li" => {
                let done = tag.contains("checked") || tag.contains("done");
                if done || lists.last().copied().unwrap_or(false) {
                    out.push_str(if done { "<input type=checkbox checked>" } else { "<input type=checkbox>" });
                }
            }
            _ => {}
        }
        at = end;
    }
    out.push_str(&body[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record as the script writes it.
    fn record(folders: &[&str], name: &str, created: i64, modified: i64, attachments: usize, body: &str) -> String {
        on_account("iCloud", folders, name, created, modified, attachments, body)
    }

    fn on_account(account: &str, folders: &[&str], name: &str, created: i64, modified: i64, attachments: usize, body: &str) -> String {
        let path = folders.join(&PATH.to_string());
        ["2", account, &path, name, &created.to_string(), &modified.to_string(), &attachments.to_string(), body].join(&FIELD.to_string())
    }

    fn fields(f: &[&str]) -> String {
        f.join(&FIELD.to_string())
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-applenotes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const DOT: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

    #[test]
    fn a_note_keeps_its_folders_dates_formatting_and_pictures() {
        let body = format!(
            "<div><b><span style=\"font-size: 24px\">Trip to Lisbon</span></b></div><div>Flights land at <b>09:40</b>.</div><div><br></div><div><img src=\"data:image/png;base64,{DOT}\"></div><ul><li>sunscreen</li><li>adapter</li></ul>"
        );
        let c = read(record(&["Travel", "2026"], "Trip to Lisbon", 1_700_000_000_000, 1_700_000_500_000, 1, &body).as_bytes()).unwrap();
        let n = &c.notes[0];
        assert_eq!(n.folders, ["Travel", "2026"]);
        assert_eq!((n.created, n.modified), (1_700_000_000_000, 1_700_000_500_000));
        assert!(n.markdown.starts_with("# Trip to Lisbon\n\nFlights land at **09:40**."), "the first line is the heading, once: {:?}", n.markdown);
        assert!(n.markdown.contains("- sunscreen\n- adapter"));
        assert_eq!(n.files.len(), 1);
        assert_eq!(n.files[0].name, "Trip to Lisbon.png", "named for its note");
        assert!(n.markdown.contains("![[Trip to Lisbon.png]]"));
        assert_eq!(c.attachments_left, 0, "its one attachment is the picture");
    }

    #[test]
    fn checklists_become_tasks_ticked_as_they_were() {
        let body = "<div>Packing</div><ul class=\"checklist\"><li class=\"checked\">passport</li><li>charger</li></ul><ul><li>a plain list</li></ul>";
        let c = read(record(&[], "Packing", 0, 0, 0, body).as_bytes()).unwrap();
        assert_eq!(c.notes[0].markdown, "# Packing\n\n- [x] passport\n- [ ] charger\n- a plain list\n");
    }

    #[test]
    fn what_stays_in_notes_is_counted_and_empty_notes_left_out() {
        let records = [
            record(&["Work"], "Scan", 0, 5, 3, "<div>Scan</div><div>See the PDF.</div>"),
            record(&[], "", 0, 0, 0, "<div><br></div>"),
            record(&[], "Untitled body", 9, 0, 0, "<div>Only words</div>"),
        ]
        .join(&RECORD.to_string());
        let c = read(records.as_bytes()).unwrap();
        assert_eq!(c.notes.len(), 2);
        assert_eq!(c.skipped, 1, "the empty one");
        assert_eq!(c.attachments_left, 3, "a PDF and the rest stay in Notes");
        assert_eq!(c.notes[0].modified, 5);
        assert_eq!(c.notes[1].modified, 9, "no change date: the creation date");
        assert_eq!(c.notes[1].markdown, "# Untitled body\n\nOnly words\n");
    }

    #[test]
    fn the_scripts_folder_is_read_whole_with_locked_notes_counted() {
        let dir = std::env::temp_dir().join(format!("kolada-applenotes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_dir(&dir).is_err(), "nothing came at all");
        std::fs::write(dir.join("note-1.txt"), record(&["Home"], "Shopping", 1, 2, 0, "<div>Shopping</div><div>Milk</div>")).unwrap();
        std::fs::write(dir.join("note-2.txt"), record(&[], "Ideas", 3, 4, 0, "<div>Ideas</div>")).unwrap();
        std::fs::write(dir.join("done.txt"), fields(&["2", "2", "4", "1"])).unwrap();
        let c = read_dir(&dir).unwrap();
        assert_eq!(c.notes.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["Shopping", "Ideas"]);
        assert_eq!((c.locked, c.unreadable), (4, 1));
        assert!(!c.stopped);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_that_came_before_notes_stopped_are_kept_and_the_rest_counted() {
        let dir = std::env::temp_dir().join(format!("kolada-applenotes-part-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Five to read, and two locked (never "not reached").
        std::fs::write(dir.join("total.txt"), fields(&["2", "5", "2", "1"])).unwrap();
        std::fs::write(dir.join("note-1.txt"), record(&["Home"], "Shopping", 1, 2, 0, "<div>Shopping</div><div>Milk</div>")).unwrap();
        std::fs::write(dir.join("note-2.txt"), record(&[], "Ideas", 3, 4, 0, "<div>Ideas</div>")).unwrap();
        // No done.txt: Notes stopped answering after two.
        let c = read_dir(&dir).unwrap();
        assert_eq!(c.notes.len(), 2);
        assert!(c.stopped);
        assert_eq!(c.not_reached, 3, "of five, three never came");
        assert_eq!(c.locked, 2, "the locked ones are counted as locked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_note_that_could_not_be_saved_leaves_the_rest_and_is_counted() {
        let dir = temp("gap");
        std::fs::write(dir.join("total.txt"), fields(&["2", "4", "0", "1"])).unwrap();
        std::fs::write(dir.join("note-1.txt"), record(&[], "One", 1, 1, 0, "<div>One</div>")).unwrap();
        // note-2 never reached the disk; the script said it saved three.
        std::fs::write(dir.join("note-3.txt"), record(&[], "Three", 1, 1, 0, "<div>Three</div>")).unwrap();
        std::fs::write(dir.join("done.txt"), fields(&["2", "3", "0", "1"])).unwrap();
        let c = read_dir(&dir).unwrap();
        assert_eq!(c.notes.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["One", "Three"], "read past the gap");
        assert_eq!(c.unreadable, 2, "the one missing, and the one Notes said it could not save");
        assert!(!c.stopped);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_in_recently_deleted_stay_out_in_any_language() {
        let dir = temp("deleted");
        std::fs::write(dir.join("total.txt"), fields(&["2", "4", "0", "2"])).unwrap();
        std::fs::write(dir.join("note-1.txt"), on_account("iCloud", &["Zuletzt gelöscht"], "Alt", 1, 1, 0, "<div>Alt</div>")).unwrap();
        std::fs::write(dir.join("note-2.txt"), on_account("iCloud", &["Notizen"], "Neu", 1, 1, 0, "<div>Neu</div>")).unwrap();
        std::fs::write(dir.join("note-3.txt"), on_account("Auf meinem Mac", &["المحذوفة مؤخرًا"], "x", 1, 1, 0, "<div>x</div>")).unwrap();
        std::fs::write(dir.join("note-4.txt"), on_account("Auf meinem Mac", &["Recently Deleted"], "y", 1, 1, 0, "<div>y</div>")).unwrap();
        std::fs::write(dir.join("done.txt"), fields(&["2", "4", "0", "0"])).unwrap();
        let c = read_dir(&dir).unwrap();
        assert_eq!(c.notes.len(), 1);
        assert_eq!(c.notes[0].folders, ["iCloud", "Notizen"], "two accounts: each note under its account");
        assert_eq!(c.skipped, 3, "the bin's notes are left out");
        assert!(is_deleted_folder("  ZULETZT GELÖSCHT "));
        assert!(!is_deleted_folder("Deleted ideas"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_are_handed_over_one_at_a_time() {
        let dir = temp("each");
        for n in 1..=3 {
            std::fs::write(dir.join(format!("note-{n}.txt")), record(&[], &format!("N{n}"), 1, 1, 0, &format!("<div>N{n}</div>"))).unwrap();
        }
        std::fs::write(dir.join("done.txt"), fields(&["2", "3", "0", "0"])).unwrap();
        let mut seen = Vec::new();
        let t = each(&dir, |e| seen.push(e.title)).unwrap();
        assert_eq!(seen, ["N1", "N2", "N3"], "in Notes' order");
        assert_eq!(t, Tally::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refusal_is_told_apart_from_a_fault() {
        assert!(refused("execution error: Not authorized to send Apple events to Notes. (-1743)"));
        assert!(!refused("execution error: Notes got an error: Can't get folder 1. (-1728)"));
    }

    #[test]
    fn the_script_only_reads() {
        // It asks Notes for properties; it never makes, moves or deletes.
        for verb in ["make(", ".delete(", ".move(", "duplicate(", ".body = ", ".name = "] {
            assert!(!SCRIPT.contains(verb), "the script writes to Notes: {verb}");
        }
        assert!(SCRIPT.contains("passwordProtected") && SCRIPT.contains("done.txt") && SCRIPT.contains("total.txt"));
        // A note is counted as saved only once it is.
        assert!(SCRIPT.contains("if (save('note-' + (written + 1) + '.txt', record)) { written += 1; } else { failed += 1; }"));
    }
}
