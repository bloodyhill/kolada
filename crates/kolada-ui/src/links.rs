//! What a link at some place in a note leads to, the same on every OS (the
//! Windows shell's `app.rs` has the same `link_at`).

use kolada_markdown::{self as md, Style};

/// What a link at some place in a note leads to.
#[derive(Debug, PartialEq)]
pub enum Link {
    /// A note, by name (`[[Note]]`, or `[x](Note.md)`).
    Note(String),
    /// A web page or an email (`[x](https://…)`, `https://…`, `www.…`).
    Web(String),
    /// A tag, searched for.
    Tag(String),
}

/// `%20`-style escapes in a link's address, decoded.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The link at byte `at` of `text`, if there is one.
pub fn link_at(text: &str, at: usize) -> Option<Link> {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = md::lines(&text[start..]).next()?;
    let line = md::Line { start: line.start + start, end: line.end + start, ..line };
    let spans = md::line_spans(text, &line);
    let span = spans.iter().rfind(|s| matches!(s.style, Style::WikiLink | Style::Link | Style::Url | Style::Tag) && s.start <= at && at <= s.end)?;
    let whole = &text[span.start..span.end];
    Some(match span.style {
        Style::WikiLink => {
            let inner = whole.trim_start_matches('!').trim_start_matches("[[").trim_end_matches("]]");
            // [[Note#Heading|shown]]: the note is before any # or |.
            Link::Note(inner.split(['|', '#', '^']).next().unwrap_or("").trim().to_string())
        }
        Style::Url if whole.to_ascii_lowercase().starts_with("www.") => Link::Web(format!("https://{whole}")),
        Style::Url => Link::Web(whole.to_string()),
        Style::Tag => Link::Tag(whole.to_string()),
        _ => {
            let address = text[(span.text_end + 2).min(span.end - 1)..span.end - 1].trim();
            let lower = address.to_ascii_lowercase();
            if ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p)) {
                Link::Web(address.to_string())
            } else if lower.starts_with("www.") {
                Link::Web(format!("https://{address}"))
            } else {
                let file = unescape(address);
                let name = file.rsplit(['/', '\\']).next().unwrap_or(&file);
                Link::Note(name.strip_suffix(".md").unwrap_or(name).to_string())
            }
        }
    })
}

/// Whether `url` is one a note may open in the browser (or mail): nothing
/// else a link could name.
pub fn is_web(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p))
}

/// The `[[link]]` being typed at `caret`: where its name starts and what
/// it is so far. None outside one, or past its name (in an alias or a
/// heading). The Windows shell's `app/suggest.rs` has the same.
pub fn link_at_caret(text: &str, caret: usize) -> Option<(usize, &str)> {
    let mut caret = caret.min(text.len());
    while !text.is_char_boundary(caret) {
        caret -= 1;
    }
    let line_start = text[..caret].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line_start..caret];
    let open = before.rfind("[[")?;
    let typed = &before[open + 2..];
    if typed.contains("]]") || typed.contains(['|', '#', '^', '[', ']']) || typed.chars().count() > 80 {
        return None;
    }
    Some((line_start + open + 2, typed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_lead_to_notes_pages_and_tags() {
        let text = "See [[Plan#Goals|the plan]], [site](https://x.io), www.y.io and [doc](My%20note.md) #idea";
        let at = |s: &str| text.find(s).unwrap() + 1;
        assert_eq!(link_at(text, at("the plan")), Some(Link::Note("Plan".into())));
        assert_eq!(link_at(text, at("site")), Some(Link::Web("https://x.io".into())));
        assert_eq!(link_at(text, at("www.y")), Some(Link::Web("https://www.y.io".into())));
        assert_eq!(link_at(text, at("doc")), Some(Link::Note("My note".into())));
        assert_eq!(link_at(text, at("#idea")), Some(Link::Tag("#idea".into())));
        assert_eq!(link_at(text, 1), None);
    }

    #[test]
    fn knows_the_link_being_typed() {
        let text = "See [[Pric";
        assert_eq!(link_at_caret(text, text.len()), Some((6, "Pric")));
        assert_eq!(link_at_caret("See [[", 6), Some((6, "")));
        assert_eq!(link_at_caret("![[Img", 6), Some((3, "Img")), "embeds too");
        assert_eq!(link_at_caret("[[Note|ali", 10), None, "past the name");
        assert_eq!(link_at_caret("[[Done]] and", 12), None, "closed");
        assert_eq!(link_at_caret("[[a\nb", 5), None, "on another line");
        let arabic = "انظر [[ملا";
        assert_eq!(link_at_caret(arabic, arabic.len()).map(|(_, t)| t), Some("ملا"));
    }

    #[test]
    fn links_in_arabic_text_lead_to_their_notes() {
        let text = "راجع [[ملاحظة عربية]] و #فكرة";
        let at = |s: &str| text.find(s).unwrap() + 2;
        assert_eq!(link_at(text, at("ملاحظة")), Some(Link::Note("ملاحظة عربية".into())));
        // Inside a character, it is the character's.
        assert_eq!(link_at(text, text.find("ملاحظة").unwrap() + 1), Some(Link::Note("ملاحظة عربية".into())));
        assert!(matches!(link_at(text, at("فكرة")), Some(Link::Tag(_))));
        assert!(is_web("HTTPS://x") && is_web("mailto:a@b") && !is_web("file:///etc") && !is_web("javascript:x"));
    }
}
