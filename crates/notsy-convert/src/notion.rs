//! Notion's "Markdown & CSV" export: a `.zip` (sometimes of further `.zip`
//! parts) of Markdown pages, each named with its 32-character id
//! (`Trip 1f2e...9a.md`), a page's subpages in a folder of the same name, its
//! pictures and files beside them, and each database as a `.csv`. Read into
//! notes named without the ids, links between pages as `[[wiki links]]` and
//! pictures embedded.

use std::collections::HashMap;

use crate::import::{Collection, Entry, file_name_for, unique_in};
use crate::{Error, Picture, zip::Zip};

pub fn read(bytes: &[u8]) -> Result<Collection, Error> {
    let outer = Zip::open(bytes)?;
    // Large exports come as a zip of zips.
    let parts: Vec<Vec<u8>> =
        outer.names().filter(|n| n.to_ascii_lowercase().ends_with(".zip")).filter_map(|n| outer.read(n).and_then(Result::ok)).collect();
    let mut zips = vec![outer];
    for p in &parts {
        zips.push(Zip::open(p)?);
    }
    // Every file, by its path: which zip holds it, and where in it.
    let mut files: HashMap<String, (usize, usize)> = HashMap::new();
    for (k, z) in zips.iter().enumerate() {
        for (i, name) in z.names().enumerate().filter(|(_, n)| !n.ends_with('/')) {
            files.insert(name.to_string(), (k, i));
        }
    }
    let pages: Vec<&String> = files.keys().filter(|p| is_page(p)).collect();
    if !pages.iter().any(|p| has_id(stem(p))) {
        return Err(Error("not a Notion export".to_string()));
    }
    let read_file = |path: &str| files.get(path).and_then(|&(k, i)| zips[k].read_at(i)).and_then(Result::ok);
    // The title each page is known by in links.
    let titles: HashMap<&str, String> = pages.iter().map(|p| (p.as_str(), title_of(p))).collect();
    let mut sorted = pages.clone();
    sorted.sort();
    let mut c = Collection::default();
    for path in sorted {
        let Some(bytes) = read_file(path) else { continue };
        let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
        let title = titles[path.as_str()].clone();
        let folders = folders_of(path);
        let mut files_of_note = Vec::new();
        let markdown = if path.to_ascii_lowercase().ends_with(".csv") {
            table(&title, &text)
        } else {
            relink(&text, dir_of(path), &titles, &read_file, &mut files_of_note)
        };
        if markdown.trim().is_empty() {
            c.skipped += 1;
            continue;
        }
        let markdown = if markdown.trim_start().starts_with("# ") { markdown } else { format!("# {title}\n\n{}", markdown.trim_start()) };
        c.notes.push(Entry { folders, title, markdown: format!("{}\n", markdown.trim_end()), created: 0, modified: 0, files: files_of_note });
    }
    Ok(c)
}

/// A page, or a database's table (not its `_all` copy, which newer exports
/// add beside it).
fn is_page(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || (lower.ends_with(".csv") && !lower.ends_with("_all.csv"))
}

fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Whether a name ends with Notion's id: a space and 32 hex digits.
fn has_id(name: &str) -> bool {
    name.len() >= 33 && {
        let (head, id) = name.split_at(name.len() - 32);
        head.ends_with(' ') && id.bytes().all(|b| b.is_ascii_hexdigit())
    }
}

fn without_id(name: &str) -> &str {
    if has_id(name) { name[..name.len() - 33].trim_end() } else { name }
}

fn title_of(path: &str) -> String {
    let t = without_id(stem(path)).trim();
    if t.is_empty() { "Untitled".to_string() } else { t.to_string() }
}

/// The folders a page is in, ids taken off: a page's subpages sit in a
/// folder named as it is, beside it. The export's own top folder is not one.
fn folders_of(path: &str) -> Vec<String> {
    let dir = dir_of(path);
    dir.split('/')
        .filter(|d| !d.is_empty())
        .enumerate()
        .filter(|(i, d)| !(*i == 0 && (d.starts_with("Export-") || *d == "Private & Shared")))
        .map(|(_, d)| without_id(d).trim().to_string())
        .filter(|d| !d.is_empty())
        .collect()
}

/// `%20` and the like back to what they stand for.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `dir/target`, with `..` and `.` worked out.
fn join(dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in target.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// The page's Markdown with links to other pages as `[[Title]]` and its
/// pictures and files embedded by name, read in from the export.
fn relink(
    text: &str,
    dir: &str,
    titles: &HashMap<&str, String>,
    read_file: &dyn Fn(&str) -> Option<Vec<u8>>,
    files: &mut Vec<Picture>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("](") {
        let before = &rest[..i];
        let Some(open) = before.rfind('[').filter(|&o| !before[o..].contains('\n')) else {
            out.push_str(&rest[..i + 2]);
            rest = &rest[i + 2..];
            continue;
        };
        let after = &rest[i + 2..];
        let Some(close) = after.find(')').filter(|&c| !after[..c].contains('\n')) else {
            out.push_str(&rest[..i + 2]);
            rest = &rest[i + 2..];
            continue;
        };
        let picture = open > 0 && before.as_bytes()[open - 1] == b'!';
        let shown = &before[open + 1..];
        let target = after[..close].trim().trim_matches(['<', '>']);
        let start = if picture { open - 1 } else { open };
        let replaced = if target.contains("://") || target.starts_with('#') || target.starts_with("mailto:") {
            None
        } else {
            let path = join(dir, &percent_decode(target.split('#').next().unwrap_or(target)));
            if let Some(title) = titles.get(path.as_str()) {
                Some(if shown.trim() == title || shown.trim().is_empty() { format!("[[{title}]]") } else { format!("[[{title}|{}]]", shown.trim()) })
            } else if let Some(data) = read_file(&path) {
                let given = path.rsplit('/').next().unwrap_or(&path);
                let name = unique_in(files, &file_name_for(given, "bin"));
                files.push(Picture { name: name.clone(), data });
                Some(if picture || is_picture(&name) { format!("![[{name}]]") } else { format!("[[{name}]]") })
            } else {
                None
            }
        };
        match replaced {
            Some(link) => {
                out.push_str(&rest[..start]);
                out.push_str(&link);
            }
            None => out.push_str(&rest[..i + 2 + close + 1]),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

fn is_picture(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg"].iter().any(|e| lower.ends_with(e))
}

/// A database's CSV as a Markdown table under its title.
fn table(title: &str, csv: &str) -> String {
    let rows = parse_csv(csv.trim_start_matches('\u{feff}'));
    let Some(width) = rows.iter().map(Vec::len).max().filter(|&w| w > 0) else { return String::new() };
    let cell = |s: &str| s.trim().replace('|', "\\|").replace('\n', "<br>");
    let line = |row: &[String]| format!("| {} |", (0..width).map(|k| cell(row.get(k).map_or("", String::as_str))).collect::<Vec<_>>().join(" | "));
    let mut out = format!("# {title}\n\n{}\n| {} |\n", line(&rows[0]), vec!["---"; width].join(" | "));
    for row in &rows[1..] {
        out.push_str(&line(row));
        out.push('\n');
    }
    out
}

fn parse_csv(s: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field) = (Vec::new(), Vec::new(), String::new());
    let mut chars = s.chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if field.is_empty() => quoted = true,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            ('\r', false) => {}
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef";
    const ID2: &str = "fedcba9876543210fedcba9876543210";

    /// A zip with stored (uncompressed) files.
    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        fn crc(data: &[u8]) -> u32 {
            let mut c = !0u32;
            for &b in data {
                c ^= b as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
                }
            }
            !c
        }
        let (mut out, mut central) = (Vec::new(), Vec::new());
        for (name, data) in files {
            let at = out.len() as u32;
            let head = |sig: u32, central: bool| {
                let mut h = Vec::new();
                h.extend_from_slice(&sig.to_le_bytes());
                if central {
                    h.extend_from_slice(&20u16.to_le_bytes());
                }
                h.extend_from_slice(&20u16.to_le_bytes());
                h.extend_from_slice(&0x0800u16.to_le_bytes());
                h.extend_from_slice(&[0; 6]);
                h.extend_from_slice(&crc(data).to_le_bytes());
                h.extend_from_slice(&(data.len() as u32).to_le_bytes());
                h.extend_from_slice(&(data.len() as u32).to_le_bytes());
                h.extend_from_slice(&(name.len() as u16).to_le_bytes());
                h.extend_from_slice(&0u16.to_le_bytes());
                if central {
                    // comment length, disk, internal and external attributes
                    h.extend_from_slice(&[0; 10]);
                    h.extend_from_slice(&at.to_le_bytes());
                }
                h.extend_from_slice(name.as_bytes());
                h
            };
            out.extend(head(0x0403_4b50, false));
            out.extend_from_slice(data);
            central.extend(head(0x0201_4b50, true));
        }
        let (at, len) = (out.len() as u32, central.len() as u32);
        out.extend(central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&[0; 2]);
        out
    }

    fn export() -> Vec<u8> {
        let trip = format!("# Trip\n\nSee [Packing](Trip%20{ID}/Packing%20{ID2}.md) and [the list](Trip%20{ID}/Packing%20{ID2}.md).\n\n![](Trip%20{ID}/beach.png)\n\n[Ticket.pdf](Trip%20{ID}/Ticket.pdf)\n\n[Web](https://example.com/a(b)\n");
        let packing = format!("# Packing\n\nBack to [Trip](../Trip%20{ID}.md)\n");
        zip(&[
            (&format!("Trip {ID}.md"), trip.as_bytes()),
            (&format!("Trip {ID}/Packing {ID2}.md"), packing.as_bytes()),
            (&format!("Trip {ID}/beach.png"), b"PNG"),
            (&format!("Trip {ID}/Ticket.pdf"), b"PDF"),
            (&format!("Trip {ID}/Budget {ID2}.csv"), "\u{feff}Item,Cost,Notes\nFlight,300,\"window, please\"\nHotel,\"1,200\",\"a | b\"\n".as_bytes()),
            (&format!("Trip {ID}/Budget {ID2}_all.csv"), b"Item\nFlight\n"),
        ])
    }

    #[test]
    fn pages_lose_their_ids_and_keep_their_place() {
        let c = read(&export()).unwrap();
        let got: Vec<(Vec<String>, &str)> = c.notes.iter().map(|n| (n.folders.clone(), n.title.as_str())).collect();
        assert_eq!(got, [(vec![], "Trip"), (vec!["Trip".to_string()], "Budget"), (vec!["Trip".to_string()], "Packing")]);
    }

    #[test]
    fn links_become_wiki_links_and_files_come_along() {
        let c = read(&export()).unwrap();
        let trip = &c.notes[0];
        assert_eq!(
            trip.markdown,
            "# Trip\n\nSee [[Packing]] and [[Packing|the list]].\n\n![[beach.png]]\n\n[[Ticket.pdf]]\n\n[Web](https://example.com/a(b)\n"
        );
        assert_eq!(trip.files.iter().map(|f| (f.name.as_str(), f.data.as_slice())).collect::<Vec<_>>(), [("beach.png", &b"PNG"[..]), ("Ticket.pdf", b"PDF")]);
        assert_eq!(c.notes[2].markdown, "# Packing\n\nBack to [[Trip]]\n");
    }

    #[test]
    fn a_database_becomes_a_table() {
        let c = read(&export()).unwrap();
        assert_eq!(c.notes[1].markdown, "# Budget\n\n| Item | Cost | Notes |\n| --- | --- | --- |\n| Flight | 300 | window, please |\n| Hotel | 1,200 | a \\| b |\n");
    }

    #[test]
    fn an_export_in_parts_is_read_whole() {
        let page = |t: &str, id: &str| (format!("Export-1234/{t} {id}.md"), format!("# {t}\n\nText\n"));
        let (a, b) = (page("One", ID), page("Two", ID2));
        let part1 = zip(&[(&a.0, a.1.as_bytes())]);
        let part2 = zip(&[(&b.0, b.1.as_bytes())]);
        let c = read(&zip(&[("Export-1234-Part-1.zip", &part1), ("Export-1234-Part-2.zip", &part2)])).unwrap();
        let titles: Vec<(&str, usize)> = c.notes.iter().map(|n| (n.title.as_str(), n.folders.len())).collect();
        assert_eq!(titles, [("One", 0), ("Two", 0)], "the export's own folder is not kept");
    }

    #[test]
    fn a_zip_that_is_not_from_notion_is_refused() {
        assert!(read(&zip(&[("notes/plain.md", b"# Hi\n")])).is_err());
        assert!(read(b"not a zip").is_err());
    }

    #[test]
    fn ids_are_only_taken_off_when_they_are_ids() {
        assert_eq!(without_id(&format!("Plan {ID}")), "Plan");
        assert_eq!(without_id("Plan 2026"), "Plan 2026");
        assert_eq!(without_id(&format!("Plan{ID}")), format!("Plan{ID}"));
        assert_eq!(title_of(&format!("a/ {ID}.md")), "Untitled");
    }
}
