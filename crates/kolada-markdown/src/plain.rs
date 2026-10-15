//! A note as plain text that reads well with no Markdown: what Copy puts
//! on the clipboard for apps that take only text, and a shared note's
//! `.txt`. Bullets become `•`, tasks `☐`/`☑`, tables tab-separated cells,
//! and links keep their address after their text when the two differ.

use crate::{Block, Span, Style, cells, is_table_header, lines, spans};

/// `md` as readable text: no marks, frontmatter left out.
pub fn to_plain(md: &str) -> String {
    let all: Vec<crate::Line> = lines(md).collect();
    let mut out: Vec<String> = Vec::with_capacity(all.len());
    for (i, l) in all.iter().enumerate() {
        let line = &md[l.start..l.end];
        let indent = |depth: u8| "  ".repeat(depth as usize);
        match l.block {
            Block::Frontmatter | Block::Fence | Block::TableDelimiter => continue,
            Block::Code => out.push(line.to_string()),
            Block::Rule => out.push(String::new()),
            Block::TableRow => out.push(row(line)),
            Block::Body if is_table_header(&all, i) => out.push(row(line)),
            Block::Body if is_picture(line) => continue,
            Block::Body | Block::Heading(_) | Block::Quote => out.push(inline(&line[l.mark..])),
            Block::Bullet => out.push(format!("{}\u{2022} {}", indent(l.depth), inline(&line[l.mark..]))),
            Block::Task { done } => {
                out.push(format!("{}{} {}", indent(l.depth), if done { '\u{2611}' } else { '\u{2610}' }, inline(&line[l.mark..])))
            }
            Block::Ordered => {
                let number = line[..l.mark].trim();
                out.push(format!("{}{number} {}", indent(l.depth), inline(&line[l.mark..])))
            }
        }
    }
    out.join("\n")
}

/// A note's title as the editor names it: its first line with words after
/// any frontmatter, without its marks, at most 80 characters.
pub fn title(md: &str) -> String {
    let start = lines(md).find(|l| l.block != Block::Frontmatter).map_or(0, |l| l.start);
    // Frontmatter that never closes is text.
    let start = if md.starts_with("---\n") && lines(md).all(|l| l.block == Block::Frontmatter) { 0 } else { start };
    let line = md[start..].lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let plain = crate::plain(line);
    let mut out: String = plain.trim().chars().take(80).collect();
    // Never cut a letter from the marks that sit on it.
    out.extend(plain.trim().chars().skip(80).take_while(|c| is_combining(*c)));
    out.trim_end().to_string()
}

fn is_combining(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED | 0x200C | 0x200D | 0xFE0F | 0x1F3FB..=0x1F3FF)
}

/// A table's line as its cells, tab-separated.
fn row(line: &str) -> String {
    cells(line).into_iter().map(|(a, z)| inline(&line[a..z])).collect::<Vec<_>>().join("\t")
}

/// A line that is only a picture: `![[photo.png]]`, `![alt](photo.png)`.
fn is_picture(line: &str) -> bool {
    let s = line.trim();
    let target = if let Some(inner) = s.strip_prefix("![[").and_then(|r| r.strip_suffix("]]")) {
        inner.split('|').next().unwrap_or("")
    } else if let Some((_, url)) = s.strip_prefix("![").and_then(|r| r.split_once("](")) {
        url.strip_suffix(')').and_then(|u| u.split_whitespace().next()).unwrap_or("")
    } else {
        return false;
    };
    picture_name(target)
}

/// Whether a file (or address) names a picture, by its extension.
fn picture_name(target: &str) -> bool {
    let file = target.split(['?', '#']).next().unwrap_or("");
    let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    ["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "tif", "tiff", "ico", "avif"].contains(&ext.as_str())
}

/// A line's text (after its block mark) without its inline marks.
fn inline(text: &str) -> String {
    let mut out = String::new();
    walk(text, &spans(text), 0, text.len(), &mut out);
    out
}

/// Whether a link's address goes on the web or to mail, so is worth
/// saying beside its text (a note's file name is not).
fn is_address(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("://") || lower.starts_with("mailto:") || lower.starts_with("www.")
}

/// `s[from..to]` with its spans (nested, in order) as plain text.
fn walk(s: &str, spans: &[Span], from: usize, to: usize, out: &mut String) {
    let mut at = from;
    let mut i = 0;
    while i < spans.len() {
        let sp = spans[i];
        if sp.start < at || sp.end > to {
            i += 1;
            continue;
        }
        let mut before = &s[at..sp.start];
        // An inline picture (`![alt](photo.png)`) goes, its `!` with it.
        let picture = sp.style == Style::Link && before.ends_with('!') && picture_name(link_url(s, &sp));
        if picture {
            before = &before[..before.len() - 1];
        }
        unescape(before, out);
        let inside: Vec<Span> = spans[i + 1..].iter().filter(|c| c.start >= sp.text_start && c.end <= sp.text_end).copied().collect();
        let text = &s[sp.text_start..sp.text_end];
        match sp.style {
            _ if picture => {}
            Style::Code | Style::Url | Style::Tag => out.push_str(text),
            Style::Link => {
                let mut shown = String::new();
                walk(s, &inside, sp.text_start, sp.text_end, &mut shown);
                let url = link_url(s, &sp);
                out.push_str(&shown);
                let bare = url.strip_prefix("mailto:").unwrap_or(url);
                if is_address(url) && shown.trim() != url && shown.trim() != bare {
                    out.push_str(&format!(" ({url})"));
                }
            }
            Style::WikiLink => {
                // `[[Note#Heading]]` reads as Obsidian shows it.
                let aliased = s[sp.start..sp.text_start].contains('|');
                if aliased { out.push_str(text) } else { out.push_str(&text.replace('#', " > ")) }
            }
            Style::Strong | Style::Emphasis | Style::Strike | Style::Highlight => walk(s, &inside, sp.text_start, sp.text_end, out),
        }
        at = sp.end;
        i += 1;
        while i < spans.len() && spans[i].start < sp.end {
            i += 1;
        }
    }
    unescape(&s[at..to], out);
}

/// The address of a `[text](url "title")` link.
fn link_url<'a>(s: &'a str, sp: &Span) -> &'a str {
    let inner = &s[(sp.text_end + 2).min(sp.end - 1)..sp.end - 1];
    inner.split_whitespace().next().unwrap_or("").trim_start_matches('<').trim_end_matches('>')
}

/// Text with Markdown's escapes (`\*`) read as the marks themselves.
fn unescape(s: &str, out: &mut String) {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(|n| n.is_ascii_punctuation()) {
            continue;
        }
        out.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_reads_without_marks() {
        let md = "---\ntags: [a]\n---\n# Plan\nShip **it** on *Monday*, ==really== ~~not~~ `x*y`.\n\n## Steps\n- one\n\t- nested **bold**\n- [ ] open\n- [x] done\n1. first\n12) twelfth\n> quoted *text*\n\n```\n- not a bullet\n```\n---\nSee [site](https://x.io) and [https://y.io](https://y.io), [mail](mailto:a@b.co), [[Other note]], [[Note|alias]] and [[Doc#Part]].\n![[photo.png]]\nText ![pic](a.png) after\nAn escaped \\*star\\* and #tag\n\n| Name | Cost |\n| --- | --: |\n| **Tea** | 4 |\n| Pipe \\| bar | 5 |";
        let plain = to_plain(md);
        let want = "Plan\nShip it on Monday, really not x*y.\n\nSteps\n\u{2022} one\n  \u{2022} nested bold\n\u{2610} open\n\u{2611} done\n1. first\n12) twelfth\nquoted text\n\n- not a bullet\n\nSee site (https://x.io) and https://y.io, mail (mailto:a@b.co), Other note, alias and Doc > Part.\nText  after\nAn escaped *star* and #tag\n\nName\tCost\nTea\t4\nPipe | bar\t5";
        assert_eq!(plain, want);
    }

    #[test]
    fn plain_text_keeps_lines_and_right_to_left_text() {
        assert_eq!(to_plain("a\n\nb\n"), "a\n\nb\n");
        assert_eq!(to_plain("ملاحظة **مهمة**"), "ملاحظة مهمة");
        assert_eq!(to_plain(""), "");
        assert_eq!(to_plain("<b>raw</b>"), "<b>raw</b>", "plain text is not HTML: nothing to escape");
    }

    #[test]
    fn a_link_whose_text_is_its_address_says_it_once() {
        assert_eq!(to_plain("[www.x.io](www.x.io)"), "www.x.io");
        assert_eq!(to_plain("[a@b.co](mailto:a@b.co)"), "a@b.co");
        assert_eq!(to_plain("[notes](Notes.md)"), "notes", "a note's file is no address");
        assert_eq!(to_plain("[t](https://x.io \"title\")"), "t (https://x.io)");
    }

    #[test]
    fn the_title_is_the_first_line_as_it_reads() {
        assert_eq!(title("---\na: 1\n---\n\n# **Big** plan\nbody"), "Big plan");
        assert_eq!(title("\n\n  - [ ] task one\n"), "task one");
        assert_eq!(title(""), "");
        let long = "x".repeat(200);
        assert_eq!(title(&long).chars().count(), 80);
    }
}
