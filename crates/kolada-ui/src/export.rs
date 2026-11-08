//! A note exported as Markdown (Export Note), shared by every shell: its
//! pictures go with it, into a folder beside the exported file named for
//! it ("Plan pictures"), and each embed becomes a Markdown image there, so
//! another app shows them. Written as it was, the note's `![[photo.png]]`
//! named a picture in the vault, which another app could not find.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::images::is_image;
use crate::pictures::find;

/// What exporting a note to `out` writes: its text, with each picture it
/// embeds pointed at the folder beside `out`, and the pictures to copy
/// there (from, to). Pictures are looked up as the note shows them, from
/// `from` (the note's folder) in `vault`; one that is not there, a web
/// picture, and anything in a code block stay as they were.
pub fn markdown(text: &str, vault: &Path, from: &Path, out: &Path) -> (String, Vec<(PathBuf, PathBuf)>) {
    let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Note".to_string());
    let folder = format!("{stem} pictures");
    let dir = out.parent().unwrap_or(Path::new("")).join(&folder);
    let mut known = None;
    // Each picture once, by where it is; two of one name get their own.
    let mut copied: HashMap<PathBuf, String> = HashMap::new();
    let mut copies: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut place = |file: PathBuf| -> String {
        if let Some(name) = copied.get(&file) {
            return name.clone();
        }
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let taken = |n: &str| copies.iter().any(|(_, to)| to.file_name().is_some_and(|t| t.to_string_lossy() == n));
        let name = if !taken(&name) {
            name
        } else {
            let (base, ext) = name.rsplit_once('.').unwrap_or((&name, ""));
            (2..).map(|i| format!("{base} {i}.{ext}")).find(|n| !taken(n)).unwrap_or(name.clone())
        };
        copies.push((file.clone(), dir.join(&name)));
        copied.insert(file, name.clone());
        name
    };
    let mut lines = Vec::new();
    let mut fenced = false;
    for line in text.split('\n') {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
            lines.push(line.to_string());
            continue;
        }
        if fenced {
            lines.push(line.to_string());
            continue;
        }
        lines.push(rewrite(line, &mut |target| {
            if vault.as_os_str().is_empty() || target.contains("://") || target.starts_with("data:") || !is_image(target) {
                return None;
            }
            let file = find(vault, from, target, &mut known)?;
            let name = place(file);
            Some((stem_of(&name), format!("{}/{}", escape(&folder), escape(&name))))
        }));
    }
    (lines.join("\n"), copies)
}

/// Writes the note's Markdown to `out` and its pictures into the folder
/// beside it (made only when there are any); whether all of it was written.
pub fn write_markdown(text: &str, vault: &Path, from: &Path, out: &Path, line_end: &str) -> bool {
    let (text, copies) = markdown(text, vault, from, out);
    let text = if line_end == "\n" { text } else { text.replace('\n', line_end) };
    let mut ok = std::fs::write(out, text).is_ok();
    for (src, to) in copies {
        let same = std::fs::metadata(&to).ok().zip(std::fs::metadata(&src).ok()).is_some_and(|(a, b)| a.len() == b.len());
        ok &= to.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && (same || std::fs::copy(&src, &to).is_ok());
    }
    ok
}

/// `line` with each picture embed (`![[name|size]]`, `![alt](place)`) whose
/// target `found` knows made `![alt](place)`.
fn rewrite(line: &str, found: &mut dyn FnMut(&str) -> Option<(String, String)>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("![") {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        // ![[name]] and ![[name|300]].
        if let Some(inner) = tail.strip_prefix("![[").and_then(|t| t.find("]]").map(|e| &t[..e])) {
            let target = inner.split('|').next().unwrap_or("").trim();
            let whole = 3 + inner.len() + 2;
            match found(target) {
                Some((alt, place)) => out.push_str(&format!("![{alt}]({place})")),
                None => out.push_str(&tail[..whole]),
            }
            rest = &tail[whole..];
            continue;
        }
        // ![alt](place), the place maybe in <angle brackets>.
        if let Some((alt, dest, whole)) = markdown_image(tail) {
            let target = dest.trim_start_matches('<').trim_end_matches('>');
            match found(target) {
                Some((_, place)) => out.push_str(&format!("![{alt}]({place})")),
                None => out.push_str(&tail[..whole]),
            }
            rest = &tail[whole..];
            continue;
        }
        out.push_str("![");
        rest = &tail[2..];
    }
    out.push_str(rest);
    out
}

/// `![alt](dest)` at the start of `s`: its alt, its destination, its length.
fn markdown_image(s: &str) -> Option<(&str, &str, usize)> {
    let close = s.find("](")?;
    let alt = &s[2..close];
    if alt.contains(']') {
        return None;
    }
    let after = &s[close + 2..];
    // In <angle brackets> the place may hold a ")".
    let end = match after.strip_prefix('<') {
        Some(_) => {
            let gt = after.find('>')?;
            gt + after[gt..].find(')')?
        }
        None => after.find(')')?,
    };
    let dest = after[..end].trim();
    Some((alt, dest, close + 2 + end + 1))
}

fn stem_of(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_string()
}

/// A file or folder name as a Markdown link's destination: what would end
/// or break it escaped, the rest (letters of any script) as it is.
fn escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '\u{202f}' => out.push_str("%E2%80%AF"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kolada-ui-export-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Exported as Markdown, a note's pictures go with it into "<note>
    /// pictures" beside the file, and its embeds point there, so another
    /// app finds them; a picture in a subfolder, one named twice, the
    /// Markdown form, a width, spaces and a narrow space in a name all
    /// come; a missing one, a web picture and code stay as they were.
    #[test]
    fn a_notes_pictures_go_with_it_when_it_is_exported_as_markdown() {
        let vault = dir("vault");
        std::fs::create_dir_all(vault.join("attachments")).unwrap();
        std::fs::create_dir_all(vault.join("Trips")).unwrap();
        std::fs::write(vault.join("attachments").join("Beach photo.png"), b"PNG1").unwrap();
        std::fs::write(vault.join("Trips").join("Map.png"), b"PNG2").unwrap();
        std::fs::write(vault.join("Screenshot 10.00\u{202f}AM.png"), b"PNG3").unwrap();
        let note = "# Trip\n\n![[Beach photo.png|300]]\n- A list item\n  ![[Map.png]]\nSee ![the map](Trips/Map.png) and ![[Screenshot 10.00\u{202f}AM.png]].\n![[Gone.png]]\n![web](https://example.com/a.png)\n```\n![[Beach photo.png]]\n```\n";
        let out_dir = dir("out");
        let out = out_dir.join("Trip.md");
        assert!(write_markdown(note, &vault, &vault, &out, "\n"));
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(
            text,
            "# Trip\n\n![Beach photo](Trip%20pictures/Beach%20photo.png)\n- A list item\n  ![Map](Trip%20pictures/Map.png)\nSee ![the map](Trip%20pictures/Map.png) and ![Screenshot 10.00\u{202f}AM](Trip%20pictures/Screenshot%2010.00%E2%80%AFAM.png).\n![[Gone.png]]\n![web](https://example.com/a.png)\n```\n![[Beach photo.png]]\n```\n"
        );
        let folder = out_dir.join("Trip pictures");
        let mut names: Vec<String> = std::fs::read_dir(&folder).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["Beach photo.png", "Map.png", "Screenshot 10.00\u{202f}AM.png"], "each once");
        assert_eq!(std::fs::read(folder.join("Map.png")).unwrap(), b"PNG2");
        // Two pictures of one name from two folders: both come.
        std::fs::write(vault.join("Map.png"), b"PNG4").unwrap();
        let (text, copies) = markdown("![[Map.png]]\n![](Trips/Map.png)\n", &vault, &vault, &out);
        assert_eq!(text, "![Map](Trip%20pictures/Map.png)\n![](Trip%20pictures/Map%202.png)\n");
        assert_eq!(copies.len(), 2);
        // No pictures: no folder, and the text as it was.
        let plain = out_dir.join("Plain.md");
        assert!(write_markdown("# Plain\n\nWords.\n", &vault, &vault, &plain, "\r\n"));
        assert_eq!(std::fs::read_to_string(&plain).unwrap(), "# Plain\r\n\r\nWords.\r\n");
        assert!(!out_dir.join("Plain pictures").exists());
        let _ = (std::fs::remove_dir_all(vault), std::fs::remove_dir_all(out_dir));
    }
}
