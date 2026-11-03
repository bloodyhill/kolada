//! Bear's notes: its backup (`.bear2bk`, a zip of `.textbundle` folders,
//! each a `text.markdown` with its `info.json` and `assets/`), a TextBundle
//! export, or a Markdown export (`.md` files, each note's files in a folder
//! beside it), zipped or as the folder it was written into. Read into
//! notes as Bear wrote them: `#tags` and `#nested/tags` stay tags (a
//! `#tag with spaces#` joins its words), `::marked::` text is marked, and
//! attachments are embedded. Dated from `info.json` where it says;
//! archived notes go in a folder of their own, trashed ones are left out.

use std::collections::HashMap;

use crate::import::{ARCHIVED, Collection, Entry, millis};
use crate::json::{self, Value};
use crate::notion::relink;
use crate::tree::{Tree, dir_of, stem};
use crate::{Error, Picture};

pub fn read(tree: &Tree) -> Result<Collection, Error> {
    // Each bundle by its folder: the text in it, and its info.
    let mut bundles: Vec<String> = tree
        .names()
        .filter_map(|n| {
            let lower = n.to_ascii_lowercase();
            let at = lower.find(".textbundle/")?;
            Some(n[..at + ".textbundle".len()].to_string())
        })
        .collect();
    // A TextPack holds one bundle's files at its top: its text by that exact
    // name, with the bundle's info beside it (a note of a Markdown export
    // titled "Text" is a page like the others).
    let top = |name: &str| tree.names().any(|n| n == name);
    let root = top("info.json") && (top("text.markdown") || top("text.md"));
    if root {
        bundles.push(String::new());
    }
    bundles.sort();
    bundles.dedup();
    // Bundles are Bear's own: its notes, not just Markdown shaped like them.
    let mut c = Collection { marked: !bundles.is_empty(), ..Collection::default() };
    for b in &bundles {
        let Some(path) = ["text.markdown", "text.md", "text.txt"].iter().map(|t| format!("{b}/{t}")).find(|p| tree.has(p)) else { continue };
        let Some(data) = tree.read(&path) else {
            c.unreadable += 1;
            continue;
        };
        let info = tree.read(&format!("{b}/info.json")).and_then(|i| json::parse(&String::from_utf8_lossy(&i)));
        let bear = info.as_ref().map(|i| i.get("net.shinyfrog.bear").unwrap_or(i));
        if bear.and_then(|b| b.get("trashed")).is_some_and(Value::truthy) {
            continue;
        }
        let archived = bear.and_then(|b| b.get("archived")).is_some_and(Value::truthy);
        let date = |k: &str| bear.and_then(|b| b.get(k)).map_or(0, date_of);
        let (created, modified) = (date("creationDate"), date("modificationDate"));
        let modified = if modified == 0 { tree.modified(&path) } else { modified };
        match entry(tree, &data, b, stem(b), archived, true, &mut c.attachments_left) {
            Some(e) => c.notes.push(Entry { created, modified, ..e }),
            None => c.skipped += 1,
        }
    }
    // A Markdown export: notes as files, outside any bundle.
    let pages: Vec<&str> = tree
        .names()
        .filter(|n| {
            let lower = n.to_ascii_lowercase();
            (lower.ends_with(".md") || lower.ends_with(".markdown")) && !lower.contains(".textbundle/") && !(root && !n.contains('/'))
        })
        .collect();
    for path in &pages {
        let Some(data) = tree.read(path) else {
            c.unreadable += 1;
            continue;
        };
        let modified = tree.modified(path);
        match entry(tree, &data, dir_of(path), stem(path), false, c.marked, &mut c.attachments_left) {
            Some(e) => c.notes.push(Entry { modified, ..e }),
            None => c.skipped += 1,
        }
    }
    if bundles.is_empty() && pages.is_empty() {
        return Err(Error::unreadable());
    }
    Ok(c)
}

/// One note from its text; `dir` is where its links are from, `name` what
/// it is called when its text gives no title.
/// `bear`: the export is Bear's own (it holds Bear's bundles), so Bear's
/// marks are read as Bear means them; plain Markdown keeps its text as it
/// is (`std::vector` stays code, not a mark).
fn entry(tree: &Tree, data: &[u8], dir: &str, name: &str, archived: bool, bear: bool, left: &mut usize) -> Option<Entry> {
    let text = String::from_utf8_lossy(data).trim_start_matches('\u{feff}').replace("\r\n", "\n");
    if text.trim().is_empty() {
        return None;
    }
    let text = if bear { bear_marks(&text) } else { text };
    let mut files: Vec<Picture> = Vec::new();
    // An attachment there but not readable is counted, its link kept.
    let read_file = |p: &str| tree.has(p).then(|| tree.read(p).ok_or(()));
    let markdown = relink(&text, dir, &HashMap::new(), &read_file, &mut files, left);
    let title = title_of(&markdown).unwrap_or_else(|| name.trim().to_string());
    Some(Entry {
        folders: if archived { vec![ARCHIVED.to_string()] } else { Vec::new() },
        title: if title.is_empty() { "Untitled".to_string() } else { title },
        markdown: format!("{}\n", markdown.trim_end()),
        created: 0,
        modified: 0,
        files,
    })
}

/// A note's title: its first heading, or its first line with words.
fn title_of(markdown: &str) -> Option<String> {
    let line = markdown.lines().map(str::trim).find(|l| !l.is_empty())?;
    let t = line.trim_start_matches('#').trim();
    (!t.is_empty()).then(|| t.chars().take(100).collect())
}

/// Bear's own marks as Markdown: `#tag with spaces#` as `#tag-with-spaces`,
/// `::marked::` as `==marked==`, and the sizes Bear 2 notes after a
/// picture (`<!-- {"width":…} -->`) left out. Code is left as it is.
fn bear_marks(text: &str) -> String {
    let mut out = Vec::new();
    let mut fence = false;
    for line in text.split('\n') {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            out.push(line.to_string());
            continue;
        }
        if fence {
            out.push(line.to_string());
            continue;
        }
        let line = without_sizes(line);
        out.push(marked(&closed_tags(&line)));
    }
    out.join("\n")
}

/// `<!-- {…} -->` taken out.
fn without_sizes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("<!-- {") {
        let Some(end) = rest[i..].find("} -->") else { break };
        out.push_str(&rest[..i]);
        rest = &rest[i + end + "} -->".len()..];
    }
    out.push_str(rest);
    out
}

/// `::text::` as `==text==`, outside inline code.
fn marked(line: &str) -> String {
    if !line.contains("::") {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len());
    for (k, part) in line.split('`').enumerate() {
        if k > 0 {
            out.push('`');
        }
        if k % 2 == 1 {
            out.push_str(part);
            continue;
        }
        let mut rest = part;
        while let Some(i) = rest.find("::") {
            let inner = &rest[i + 2..];
            match inner.find("::").filter(|&j| j > 0 && !inner[..j].starts_with(' ') && !inner[..j].ends_with(' ')) {
                Some(j) => {
                    out.push_str(&rest[..i]);
                    out.push_str("==");
                    out.push_str(&inner[..j]);
                    out.push_str("==");
                    rest = &inner[j + 2..];
                }
                None => {
                    out.push_str(&rest[..i + 2]);
                    rest = inner;
                }
            }
        }
        out.push_str(rest);
    }
    out
}

/// Bear's `#tag with spaces#` (a tag that closes with `#`) as one tag,
/// its spaces as dashes. Tags that do not close are left as they are.
fn closed_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find('#') {
        let before = if i > 0 { rest[..i].chars().last() } else { out.chars().last() };
        let inner = &rest[i + 1..];
        let opens = before.is_none_or(char::is_whitespace) && inner.chars().next().is_some_and(|c| !c.is_whitespace() && c != '#');
        let close = inner.find('#').filter(|&j| {
            let t = &inner[..j];
            let after = inner[j + 1..].chars().next();
            t.contains(' ') && !t.ends_with(' ') && after.is_none_or(|c| c.is_whitespace() || c.is_ascii_punctuation())
        });
        match (opens, close) {
            (true, Some(j)) => {
                out.push_str(&rest[..i]);
                out.push('#');
                out.push_str(&inner[..j].split_whitespace().collect::<Vec<_>>().join("-"));
                rest = &inner[j + 1..];
            }
            _ => {
                out.push_str(&rest[..i + 1]);
                rest = inner;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A date as Bear's `info.json` writes it, in milliseconds since 1970:
/// ISO 8601 (`2023-06-14T09:31:44Z`, with a fraction or an offset), or a
/// number of seconds (or milliseconds) since 1970.
fn date_of(v: &Value) -> i64 {
    if let Value::Number(_) = v {
        let n = v.int().unwrap_or(0);
        let ms = if n > 100_000_000_000 { n } else { n.saturating_mul(1000) };
        return crate::import::sane(ms).unwrap_or(0);
    }
    v.str().and_then(iso).unwrap_or(0)
}

fn iso(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b).and_then(|t| t.parse::<u32>().ok());
    let (y, mo, d) = (num(0, 4)? as i32, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut ms = millis(y, mo, d, h, mi, sec)?;
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        ms += frac[..digits.min(3)].parse::<i64>().map_or(0, |f| f * 10i64.pow(3 - digits.min(3) as u32));
        rest = &frac[digits..];
    }
    // An offset: the time was that far ahead of UTC.
    let rest = rest.trim();
    if let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        let digits: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
        if digits.len() >= 4 {
            let off = (digits[..2].parse::<i64>().ok()? * 60 + digits[2..4].parse::<i64>().ok()?) * 60_000;
            ms -= if sign == '+' { off } else { -off };
        }
    }
    Some(ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::stored;

    fn info(fields: &str) -> String {
        format!("{{\"net.shinyfrog.bear\": {{{fields}}}, \"type\": \"net.daringfireball.markdown\", \"version\": 2}}")
    }

    fn backup() -> Vec<u8> {
        let plan = "# Plan: Q3 / 2026?\n\nWork on #projects/kolada and #home.\nA #tag with spaces# here, and ::marked:: text.\n\n![](assets/photo%201.png)<!-- {\"width\":320} -->\n[report.pdf](assets/report.pdf)\n\n```\n#not a tag# ::kept::\n```\n";
        let arabic = "# ملاحظات\n\nنص من اليمين إلى اليسار #عربي\n";
        stored(&[
            ("Plan.textbundle/text.markdown", plan.as_bytes()),
            ("Plan.textbundle/info.json", info(r#""creationDate": "2023-06-14T09:31:44Z", "modificationDate": "2024-01-02T03:04:05.250+02:00", "trashed": 0, "archived": 0, "pinned": 1"#).as_bytes()),
            ("Plan.textbundle/assets/photo 1.png", b"PNG"),
            ("Plan.textbundle/assets/report.pdf", b"PDF"),
            ("ملاحظات.textbundle/text.markdown", arabic.as_bytes()),
            ("ملاحظات.textbundle/info.json", info(r#""creationDate": "2022-01-01T00:00:00Z", "archived": 1"#).as_bytes()),
            ("Old.textbundle/text.markdown", b"# Old\n\ngone\n"),
            ("Old.textbundle/info.json", info(r#""trashed": 1"#).as_bytes()),
            ("Blank.textbundle/text.markdown", b"  \n"),
        ])
    }

    #[test]
    fn a_backup_keeps_tags_marks_dates_and_attachments() {
        let c = read(&Tree::zip(&backup()).unwrap()).unwrap();
        let plan = &c.notes.iter().find(|n| n.title == "Plan: Q3 / 2026?").unwrap();
        assert_eq!(
            plan.markdown,
            "# Plan: Q3 / 2026?\n\nWork on #projects/kolada and #home.\nA #tag-with-spaces here, and ==marked== text.\n\n![[photo 1.png]]\n[[report.pdf]]\n\n```\n#not a tag# ::kept::\n```\n"
        );
        assert_eq!(plan.files.iter().map(|f| (f.name.as_str(), f.data.as_slice())).collect::<Vec<_>>(), [("photo 1.png", &b"PNG"[..]), ("report.pdf", b"PDF")]);
        assert_eq!(plan.created, millis(2023, 6, 14, 9, 31, 44).unwrap());
        assert_eq!(plan.modified, millis(2024, 1, 2, 1, 4, 5).unwrap() + 250);
        assert!(plan.folders.is_empty());
    }

    #[test]
    fn archived_notes_come_in_apart_and_trashed_and_empty_ones_do_not() {
        let c = read(&Tree::zip(&backup()).unwrap()).unwrap();
        let arabic = c.notes.iter().find(|n| n.title == "ملاحظات").unwrap();
        assert_eq!(arabic.folders, [ARCHIVED]);
        assert_eq!(arabic.markdown, "# ملاحظات\n\nنص من اليمين إلى اليسار #عربي\n");
        assert!(c.notes.iter().all(|n| n.title != "Old"));
        assert_eq!((c.notes.len(), c.skipped), (2, 1));
    }

    #[test]
    fn a_markdown_export_reads_with_each_notes_files_beside_it() {
        let note = "Groceries\n\n- [ ] eggs #home\n\n![](Groceries%20%231/list%20(1).JPG)\n";
        let zip = stored(&[("Groceries #1.md", note.as_bytes()), ("Groceries #1/list (1).JPG", b"JPG")]);
        let c = read(&Tree::zip(&zip).unwrap()).unwrap();
        assert_eq!(c.notes.len(), 1);
        assert_eq!(c.notes[0].title, "Groceries");
        assert_eq!(c.notes[0].markdown, "Groceries\n\n- [ ] eggs #home\n\n![[list (1).jpg]]\n");
        assert_eq!(c.notes[0].files[0].data, b"JPG");
        // The same, as the folder it was exported into, dated by its files.
        let dir = std::env::temp_dir().join(format!("kolada-bear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Groceries #1")).unwrap();
        std::fs::write(dir.join("Groceries #1.md"), note).unwrap();
        std::fs::write(dir.join("Groceries #1").join("list (1).JPG"), b"JPG").unwrap();
        let c = read(&Tree::dir(&dir).unwrap()).unwrap();
        assert_eq!(c.notes[0].markdown, "Groceries\n\n- [ ] eggs #home\n\n![[list (1).jpg]]\n");
        assert!(c.notes[0].modified > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_zip_with_no_notes_is_refused() {
        assert!(read(&Tree::zip(&stored(&[("photo.png", b"PNG")])).unwrap()).is_err());
    }

    #[test]
    fn closed_tags_join_and_open_ones_stay() {
        assert_eq!(closed_tags("#a b# and #c"), "#a-b and #c");
        assert_eq!(closed_tags("#one# x"), "#one# x", "no space: not a closed tag");
        assert_eq!(closed_tags("issue #4 and #5"), "issue #4 and #5");
        assert_eq!(closed_tags("x#y z#"), "x#y z#", "not at a word's start");
        assert_eq!(marked("a ::b:: `::c::` :: d"), "a ==b== `::c::` :: d");
    }

    #[test]
    fn dates_read_as_bear_writes_them() {
        assert_eq!(iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(iso("1970-01-01 00:00:00 -0100"), Some(3_600_000));
        assert_eq!(iso("nonsense"), None);
        assert_eq!(date_of(&Value::Number("1700000000".into())), 1_700_000_000_000);
    }

    #[test]
    fn a_link_to_another_note_stays_a_link_and_is_not_copied_in() {
        let zip = stored(&[("A.md", b"# A\n\nSee [B](B.md) and ![](A/pic.png)\n"), ("B.md", b"# B\n"), ("A/pic.png", b"PNG")]);
        let c = read(&Tree::zip(&zip).unwrap()).unwrap();
        let a = c.notes.iter().find(|n| n.title == "A").unwrap();
        assert_eq!(a.markdown, "# A\n\nSee [B](B.md) and ![[pic.png]]\n");
        assert_eq!(a.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["pic.png"], "B.md is a note, not an attachment");
        assert_eq!(c.notes.len(), 2);
    }

    #[test]
    fn a_textpack_is_one_bundle_at_its_top() {
        let pack = stored(&[("text.markdown", b"# Packed\n\n![](assets/a.png)\n"), ("info.json", br#"{"net.shinyfrog.bear": {"creationDate": "2026-01-02T03:04:05Z"}}"#), ("assets/a.png", b"A")]);
        let c = read(&Tree::zip(&pack).unwrap()).unwrap();
        assert_eq!(c.notes.len(), 1, "{:?}", c.notes.iter().map(|n| &n.title).collect::<Vec<_>>());
        assert_eq!(c.notes[0].markdown, "# Packed\n\n![[a.png]]\n");
        assert_eq!(c.notes[0].created, millis(2026, 1, 2, 3, 4, 5).unwrap());
    }

    /// Plain Markdown (an Obsidian vault zipped, notes with code in them)
    /// keeps its text as written: Bear's marks are read only in Bear's own
    /// export.
    #[test]
    fn plain_markdown_keeps_its_text() {
        let text = "# Code\n\nUse std::vector and foo::bar, and a #tag with spaces# too.\n";
        let zip = stored(&[("Code.md", text.as_bytes()), ("Other.md", b"# Other\n\nWords.\n")]);
        let c = read(&Tree::zip(&zip).unwrap()).unwrap();
        let code = c.notes.iter().find(|n| n.title == "Code").unwrap();
        assert!(code.markdown.contains("std::vector and foo::bar") && code.markdown.contains("#tag with spaces#"), "{:?}", code.markdown.len());
    }

    /// A Markdown export with a note titled "Text" is every note it holds,
    /// not one TextPack.
    #[test]
    fn a_note_called_text_is_a_page_of_the_export() {
        let export = stored(&[("Text.md", b"# Text\n\nA note called Text.\n"), ("Groceries.md", b"# Groceries\n\n- [ ] Eggs\n"), ("Plans.md", b"# Plans\n\nShip it.\n")]);
        let c = read(&Tree::zip(&export).unwrap()).unwrap();
        let mut titles: Vec<&str> = c.notes.iter().map(|n| n.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, ["Groceries", "Plans", "Text"]);
        let lower = stored(&[("text.md", b"# text\n\nlower.\n"), ("Plans.md", b"# Plans\n\nShip it.\n")]);
        assert_eq!(read(&Tree::zip(&lower).unwrap()).unwrap().notes.len(), 2, "no info beside it: a page");
    }

    #[test]
    fn dates_out_of_range_are_left_off() {
        for n in ["1000000000000000000000", "-5", "9223372036854775807"] {
            assert_eq!(date_of(&Value::Number(n.into())), 0, "{n}");
        }
    }
}
