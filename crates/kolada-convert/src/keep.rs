//! Google Keep, as Google Takeout gives it: a `.zip` (or the folder it
//! unpacks into) with `Takeout/Keep/` in it, a `.json` per note beside its
//! attachments (and an `.html` copy of each, not read). Read into notes:
//! the title and text, checklists as tasks ticked as they were, labels as
//! tags, links the note kept, attachments embedded, dated as Keep dated
//! them. Archived notes go in a folder of their own; trashed ones and
//! colours are left out.

use crate::import::{ARCHIVED, Collection, Entry, file_name_for, unique_in};
use crate::json::{self, Value};
use crate::notion::is_picture;
use crate::tree::{Tree, dir_of, join, name_of, stem};
use crate::{Error, Picture};

pub fn read(tree: &Tree) -> Result<Collection, Error> {
    let mut c = Collection::default();
    let mut found = false;
    let names: Vec<&str> = tree.names().filter(|n| n.to_ascii_lowercase().ends_with(".json")).collect();
    for path in names {
        let Some(bytes) = tree.read(path) else {
            c.unreadable += 1;
            continue;
        };
        let Some(note) = json::parse(&String::from_utf8_lossy(&bytes)).filter(is_note) else { continue };
        found = true;
        if note.get("isTrashed").is_some_and(Value::truthy) {
            continue;
        }
        match entry(&note, dir_of(path), tree, &mut c.attachments_left) {
            Some(e) => c.notes.push(e),
            None => c.skipped += 1,
        }
    }
    if !found {
        return Err(Error::unreadable());
    }
    Ok(c)
}

/// Whether a JSON file is one of Keep's notes (Takeout holds other JSON).
pub(crate) fn is_note(v: &Value) -> bool {
    ["textContent", "listContent"].iter().any(|k| v.get(k).is_some()) && ["userEditedTimestampUsec", "createdTimestampUsec", "isTrashed"].iter().any(|k| v.get(k).is_some())
}

/// One note, or None when it holds nothing; attachments it names that are
/// not in the export are counted in `left`.
fn entry(note: &Value, dir: &str, tree: &Tree, left: &mut usize) -> Option<Entry> {
    let title = note.get("title").and_then(Value::str).unwrap_or("").trim().to_string();
    let text = note.get("textContent").and_then(Value::str).unwrap_or("").replace("\r\n", "\n");
    let mut body: Vec<String> = Vec::new();
    if !text.trim().is_empty() {
        body.push(escape(text.trim_end()));
    }
    // A checklist: its items as tasks, ticked as they were.
    let items: Vec<String> = note
        .get("listContent")
        .map(Value::array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|item| {
            let t = item.get("text").and_then(Value::str)?.replace(['\r', '\n'], " ");
            let t = t.trim();
            (!t.is_empty()).then(|| format!("- [{}] {t}", if item.get("isChecked").is_some_and(Value::truthy) { 'x' } else { ' ' }))
        })
        .collect();
    if !items.is_empty() {
        body.push(items.join("\n"));
    }
    // Links Keep found in the note, as it showed them under it.
    let links: Vec<String> = note
        .get("annotations")
        .map(Value::array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|a| {
            let url = a.get("url").and_then(Value::str)?.trim();
            if url.is_empty() || text.contains(url) {
                return None;
            }
            let shown = a.get("title").and_then(Value::str).unwrap_or("").trim().replace(['[', ']'], "");
            Some(if shown.is_empty() { url.to_string() } else { format!("[{shown}]({url})") })
        })
        .collect();
    if !links.is_empty() {
        body.push(links.join("\n"));
    }
    let mut files: Vec<Picture> = Vec::new();
    let mut embeds = Vec::new();
    for a in note.get("attachments").map(Value::array).unwrap_or(&[]) {
        let Some(file) = a.get("filePath").and_then(Value::str) else { continue };
        let Some((found, data)) = attachment(tree, dir, file) else {
            *left += 1;
            continue;
        };
        let name = unique_in(&files, &file_name_for(name_of(&found), "bin"));
        embeds.push(if is_picture(&name) { format!("![[{name}]]") } else { format!("[[{name}]]") });
        files.push(Picture { name, data });
    }
    if !embeds.is_empty() {
        body.push(embeds.join("\n"));
    }
    let tags: Vec<String> = note.get("labels").map(Value::array).unwrap_or(&[]).iter().filter_map(|l| l.get("name").and_then(Value::str)).filter_map(tag).collect();
    if !tags.is_empty() {
        body.push(tags.join(" "));
    }
    if title.is_empty() && body.is_empty() {
        return None;
    }
    // Untitled, it is named by its first words, as Kolada names notes.
    let name = if title.is_empty() { first_words(&text, &items).unwrap_or_else(|| "Untitled".to_string()) } else { title.clone() };
    let mut markdown = if title.is_empty() { String::new() } else { format!("# {}\n\n", title.replace('\n', " ")) };
    markdown.push_str(&body.join("\n\n"));
    let usec = |k: &str| note.get(k).and_then(Value::int).and_then(|u| crate::import::sane(u / 1000)).unwrap_or(0);
    let archived = note.get("isArchived").is_some_and(Value::truthy);
    Some(Entry {
        folders: if archived { vec![ARCHIVED.to_string()] } else { Vec::new() },
        title: name,
        markdown: format!("{}\n", markdown.trim_end()),
        created: usec("createdTimestampUsec"),
        modified: usec("userEditedTimestampUsec"),
        files,
    })
}

/// An attachment beside the note: as named, or (Takeout sometimes names
/// the file `.jpeg` where the note says `.jpg`, or the other way) the file
/// of the same name with another extension.
fn attachment(tree: &Tree, dir: &str, file: &str) -> Option<(String, Vec<u8>)> {
    let path = join(dir, file);
    if let Some(data) = tree.read(&path) {
        return Some((path, data));
    }
    let want = stem(&path).to_lowercase();
    let other = tree.names().find(|n| dir_of(n).eq_ignore_ascii_case(dir_of(&path)) && stem(n).to_lowercase() == want && !n.to_ascii_lowercase().ends_with(".json"))?.to_string();
    tree.read(&other).map(|data| (other, data))
}

/// Plain text as Markdown that reads as it did: what Markdown (or
/// Obsidian) would read as a mark, a link, a task or a heading is escaped.
fn escape(text: &str) -> String {
    crate::md::start_safe(&crate::md::escape(text))
}

/// A label as a tag: `#Work`, `#Trip-ideas`, `#عمل`; None when nothing of
/// it can be a tag.
pub(crate) fn tag(label: &str) -> Option<String> {
    let mut out = String::new();
    for c in label.trim().chars() {
        match c {
            c if c.is_alphanumeric() || matches!(c, '_' | '-' | '/') => out.push(c),
            c if c.is_whitespace() && !out.ends_with('-') => out.push('-'),
            _ => {}
        }
    }
    let out = out.trim_matches(['-', '/']).to_string();
    (!out.is_empty() && !out.chars().all(|c| c.is_ascii_digit())).then(|| format!("#{out}"))
}

/// A note's first words, for a name when it has no title.
fn first_words(text: &str, items: &[String]) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string).or_else(|| items.first().map(|i| i[6..].to_string()))?;
    let words: String = line.chars().take(60).collect();
    Some(words.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::stored;

    fn note(fields: &str) -> String {
        format!("{{\"color\": \"YELLOW\", \"isTrashed\": false, \"isPinned\": false, \"isArchived\": false, {fields}}}")
    }

    /// A Takeout export of Keep's notes, with the other products' files a
    /// Takeout also holds.
    fn takeout() -> Vec<u8> {
        let text = note(
            r#""title": "Trip: Lisbon/Porto?", "textContent": "Pack light\n# not a heading\nSee https://example.com/x", "createdTimestampUsec": 1600000000000000, "userEditedTimestampUsec": 1700000000123456, "labels": [{"name": "Travel plans"}, {"name": "2026"}], "annotations": [{"source": "WEBLINK", "title": "Example", "url": "https://example.com/x"}, {"title": "Guide [PDF]", "url": "https://guide.example/a"}], "attachments": [{"filePath": "1a2b.jpg", "mimetype": "image/jpeg"}, {"filePath": "voice 1.3gp", "mimetype": "audio/3gpp"}]"#,
        );
        let list = note(r#""title": "", "listContent": [{"text": "Milk", "isChecked": true}, {"text": "Bread\nfresh", "isChecked": false}, {"text": "  ", "isChecked": false}], "createdTimestampUsec": 1600000000000000, "userEditedTimestampUsec": 1600000000000000, "isPinned": true"#);
        let arabic = note(r#""title": "ملاحظة", "textContent": "مرحبا بالعالم\nسطر ثان", "createdTimestampUsec": 1650000000000000, "userEditedTimestampUsec": 1650000000000000, "isArchived": true, "labels": [{"name": "عمل مهم"}]"#);
        let trashed = note(r#""title": "Gone", "textContent": "deleted", "isTrashed": true, "userEditedTimestampUsec": 1"#);
        let empty = note(r#""title": "", "textContent": "", "userEditedTimestampUsec": 1"#);
        stored(&[
            ("Takeout/archive_browser.html", b"<html></html>"),
            ("Takeout/Keep/Trip_ Lisbon_Porto_.json", text.as_bytes()),
            ("Takeout/Keep/Trip_ Lisbon_Porto_.html", b"<html></html>"),
            // Takeout's quirk: the note says .jpg, the file is .jpeg.
            ("Takeout/Keep/1a2b.jpeg", b"JPEG"),
            ("Takeout/Keep/voice 1.3gp", b"AUDIO"),
            ("Takeout/Keep/2020-09-13T12_26_40.000Z.json", list.as_bytes()),
            ("Takeout/Keep/ملاحظة.json", arabic.as_bytes()),
            ("Takeout/Keep/Gone.json", trashed.as_bytes()),
            ("Takeout/Keep/Empty.json", empty.as_bytes()),
            ("Takeout/Keep/Labels.txt", b"Travel plans\n"),
            ("Takeout/Drive/other.json", br#"{"kind": "drive#file"}"#),
        ])
    }

    fn by_title<'a>(c: &'a Collection, t: &str) -> &'a Entry {
        c.notes.iter().find(|n| n.title == t).unwrap_or_else(|| panic!("no note {t}: {:?}", c.notes.iter().map(|n| &n.title).collect::<Vec<_>>()))
    }

    #[test]
    fn a_note_keeps_its_title_text_tags_links_and_attachments() {
        let c = read(&Tree::zip(&takeout()).unwrap()).unwrap();
        let trip = by_title(&c, "Trip: Lisbon/Porto?");
        assert_eq!(
            trip.markdown,
            "# Trip: Lisbon/Porto?\n\nPack light\n\\# not a heading\nSee https://example.com/x\n\n[Guide PDF](https://guide.example/a)\n\n![[1a2b.jpeg]]\n[[voice 1.3gp]]\n\n#Travel-plans\n"
        );
        assert_eq!(trip.files.iter().map(|f| (f.name.as_str(), f.data.as_slice())).collect::<Vec<_>>(), [("1a2b.jpeg", &b"JPEG"[..]), ("voice 1.3gp", b"AUDIO")]);
        assert_eq!((trip.created, trip.modified), (1_600_000_000_000, 1_700_000_000_123));
        assert!(trip.folders.is_empty());
    }

    #[test]
    fn a_checklist_becomes_tasks_ticked_as_they_were() {
        let c = read(&Tree::zip(&takeout()).unwrap()).unwrap();
        // Untitled: named by its first item, no heading added.
        let list = by_title(&c, "Milk");
        assert_eq!(list.markdown, "- [x] Milk\n- [ ] Bread fresh\n");
    }

    #[test]
    fn archived_notes_come_in_apart_and_trashed_and_empty_ones_do_not() {
        let c = read(&Tree::zip(&takeout()).unwrap()).unwrap();
        let arabic = by_title(&c, "ملاحظة");
        assert_eq!(arabic.folders, [ARCHIVED]);
        assert_eq!(arabic.markdown, "# ملاحظة\n\nمرحبا بالعالم\nسطر ثان\n\n#عمل-مهم\n");
        assert!(c.notes.iter().all(|n| n.title != "Gone"));
        assert_eq!((c.notes.len(), c.skipped), (3, 1));
    }

    #[test]
    fn a_bare_keep_folder_reads_as_its_zip_does() {
        let dir = std::env::temp_dir().join(format!("kolada-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a #1 (copy).json"), note(r#""title": "Folder note", "textContent": "x", "userEditedTimestampUsec": 1700000000000000"#)).unwrap();
        std::fs::write(dir.join("pic.png"), b"PNG").unwrap();
        let c = read(&Tree::dir(&dir).unwrap()).unwrap();
        assert_eq!(c.notes.len(), 1);
        assert_eq!(c.notes[0].markdown, "# Folder note\n\nx\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_zip_without_keep_notes_is_refused() {
        assert!(read(&Tree::zip(&stored(&[("Takeout/Drive/a.json", br#"{"title": "x"}"#)])).unwrap()).is_err());
    }

    #[test]
    fn labels_become_tags_where_they_can() {
        assert_eq!(tag("Work").as_deref(), Some("#Work"));
        assert_eq!(tag(" trip  ideas! ").as_deref(), Some("#trip-ideas"));
        assert_eq!(tag("Home/Kitchen").as_deref(), Some("#Home/Kitchen"));
        assert_eq!(tag("2026"), None, "a number is no tag");
        assert_eq!(tag("!!!"), None);
    }

    #[test]
    fn attachments_missing_from_the_export_are_counted_and_dates_out_of_range_left_off() {
        let n = note(r#""title": "T", "textContent": "x", "createdTimestampUsec": 1000000000000000000000, "userEditedTimestampUsec": -5, "attachments": [{"filePath": "gone.png"}, {"filePath": "here.png"}]"#);
        let c = read(&Tree::zip(&stored(&[("Keep/t.json", n.as_bytes()), ("Keep/here.png", b"PNG")])).unwrap()).unwrap();
        assert_eq!(c.attachments_left, 1);
        assert_eq!(c.notes[0].files.len(), 1);
        assert_eq!((c.notes[0].created, c.notes[0].modified), (0, 0));
    }

    #[test]
    fn keeps_text_reads_as_it_did_in_keep() {
        let n = note(r#""title": "T", "textContent": "2 * 3 and snake_case and *stars*\n[[Not a link]]\n- [ ] not a task\n<b>bold?</b>\n# no heading\n1. no list""#);
        let c = read(&Tree::zip(&stored(&[("Keep/t.json", n.as_bytes())])).unwrap()).unwrap();
        let body = c.notes[0].markdown.trim_start_matches("# T\n\n").trim_end().to_string();
        assert_eq!(body, "2 \\* 3 and snake_case and \\*stars\\*\n\\[\\[Not a link\\]\\]\n\\- \\[ \\] not a task\n\\<b>bold?\\</b>\n\\# no heading\n1\\. no list");
    }
}
