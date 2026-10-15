//! Links without seeing addresses, the same on every OS. Ctrl+K (and the
//! toolbar's Link) opens a small field under the selection: a note's name
//! (with the `[[` suggestions) or a web address, then Enter, writes the
//! link over the selected words. On a link, it opens with what the link
//! leads to, and can take the link away. A web address pasted over
//! selected words links them, and resting the pointer on a link shows
//! where it goes (`address_at`): the only place an address shows.

use kolada_editor::{Editor, Selection};
use kolada_markdown::{self as md, Span, Style};

use crate::links::{Link, link_at};

/// The link field while it is open: what is typed in it, the words the
/// link goes over, and the link it edits, if it opened on one.
pub struct LinkField {
    /// A note's name or a web address.
    pub query: Editor,
    /// The note's text the link takes the place of: the selection (on one
    /// line), or the whole link being edited.
    pub range: (usize, usize),
    /// The words the link shows.
    pub words: String,
    /// It opened on a link, so it offers Remove.
    pub existing: bool,
    /// The suggested note Enter takes, if any is picked (by Up and Down).
    pub picked: Option<usize>,
}

/// The link the selection is in (or is), if any: a Markdown link, a note's
/// `[[link]]` (not a picture's `![[embed]]`), or an address written out.
pub fn link_under(text: &str, lines: &[md::Line], s: Selection) -> Option<Span> {
    let (a, b) = (s.start(), s.end());
    let i = lines.partition_point(|l| l.start <= a).max(1) - 1;
    let line = lines.get(i)?;
    md::line_spans(text, line).into_iter().rfind(|sp| {
        let kind = matches!(sp.style, Style::Link | Style::Url) || (sp.style == Style::WikiLink && !text[sp.start..].starts_with('!'));
        let around = if a == b { sp.text_start <= a && a <= sp.text_end } else { sp.start <= a && b <= sp.end };
        kind && around
    })
}

/// What a link span leads to, as the field shows it: an address, or a
/// note's name (with its heading, if it names one).
fn target_of(text: &str, sp: &Span) -> String {
    match sp.style {
        Style::WikiLink => {
            let inner = &text[sp.start + 2..sp.end - 2];
            inner.split('|').next().unwrap_or("").trim().to_string()
        }
        Style::Url => text[sp.text_start..sp.text_end].to_string(),
        _ => {
            let inner = &text[(sp.text_end + 2).min(sp.end - 1)..sp.end - 1];
            inner.split_whitespace().next().unwrap_or("").trim_start_matches('<').trim_end_matches('>').to_string()
        }
    }
}

/// Endings that name files kept with notes, never taken for a site's domain.
const FILE_ENDINGS: &[&str] = &["md", "txt", "pdf", "png", "jpg", "jpeg", "gif", "webp", "docx", "doc", "html", "htm", "csv", "canvas"];

/// `s` as a web address, if it is one: with a scheme (`https://`,
/// `mailto:`), `www.`, or a bare domain (`kolada.app/notes`). The address
/// to link to, with `https://` where it had no scheme.
pub fn web_address(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return (s.len() > lower.find("://")? + 3).then(|| s.to_string());
    }
    if lower.starts_with("mailto:") && s.contains('@') {
        return Some(s.to_string());
    }
    if lower.starts_with("www.") && s.len() > 4 {
        return Some(format!("https://{s}"));
    }
    // A bare domain: labels of letters, digits and dashes, the last of
    // letters only, then perhaps a path.
    let host = s.split(['/', '?', '#']).next().unwrap_or("");
    let labels: Vec<&str> = host.split('.').collect();
    let tld = labels.last().copied().unwrap_or("");
    let domain = labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && (2..=24).contains(&tld.len())
        && tld.chars().all(|c| c.is_ascii_alphabetic())
        // A file's name (a note, a picture, a document) is not a site.
        && !FILE_ENDINGS.contains(&tld.to_ascii_lowercase().as_str());
    domain.then(|| format!("https://{s}"))
}

/// An address pasted over a selection: a strict one only (a scheme or
/// `www.`), so text that only looks like a domain pastes as text.
fn pasted_address(s: &str) -> Option<String> {
    let lower = s.trim().to_ascii_lowercase();
    let strict = ["https://", "http://", "mailto:", "www."].iter().any(|p| lower.starts_with(p));
    if strict { web_address(s) } else { None }
}

/// An address as a Markdown link's target: what would end the link or
/// break it, escaped.
fn as_target(url: &str) -> String {
    url.replace(' ', "%20").replace('(', "%28").replace(')', "%29").replace('<', "%3C").replace('>', "%3E")
}

/// What linking `words` to `to` (a note's name or a web address) writes:
/// `[words](https://…)`, `[[Note|words]]`, or `[[Note]]` when the words are
/// the note's name (or there are none). With no words, an address shows as
/// itself.
pub fn written(to: &str, words: &str) -> Option<String> {
    let to = to.trim();
    if to.is_empty() {
        return None;
    }
    if let Some(url) = web_address(to) {
        if words.trim().is_empty() || words == url || words == to {
            let bare = url.to_ascii_lowercase().starts_with("http") && !url.contains(['(', ')', '<', '>']);
            return Some(if bare { url } else { format!("[{to}]({})", as_target(&url)) });
        }
        return Some(format!("[{words}]({})", as_target(&url)));
    }
    let note: String = to.chars().filter(|c| !matches!(c, '[' | ']' | '|' | '\n')).collect();
    let note = note.trim();
    if note.is_empty() {
        return None;
    }
    let shown = md::plain(words).replace(['[', ']', '|'], "");
    let shown = shown.trim();
    Some(if shown.is_empty() || shown == note { format!("[[{note}]]") } else { format!("[[{note}|{shown}]]") })
}

impl LinkField {
    /// The field for `e`'s selection: on a link, filled with what it leads
    /// to; else empty, to link the selected words (those on the
    /// selection's first line), or to put a link at the caret.
    pub fn open(e: &Editor) -> LinkField {
        let text = e.text();
        let s = e.selection();
        if let Some(sp) = link_under(text, e.outline(), s) {
            let words = match sp.style {
                Style::WikiLink if text[sp.start..sp.text_start].contains('|') => text[sp.text_start..sp.text_end].to_string(),
                Style::WikiLink | Style::Url => String::new(),
                _ => text[sp.text_start..sp.text_end].to_string(),
            };
            let mut query = Editor::field(target_of(text, &sp));
            query.select_all();
            return LinkField { query, range: (sp.start, sp.end), words, existing: true, picked: None };
        }
        let (a, b) = (s.start(), s.end());
        let b = text[a..b].find('\n').map_or(b, |i| a + i);
        LinkField { query: Editor::field(""), range: (a, b), words: text[a..b].to_string(), existing: false, picked: None }
    }

    /// What is typed, when it is a note's name being looked for (not an
    /// address): the suggestions are for it.
    pub fn note_typed(&self) -> Option<&str> {
        let q = self.query.text().trim();
        (web_address(q).is_none()).then_some(q)
    }

    /// Enter: the link to `to` (the note picked, or what is typed) written
    /// over the field's range, as one undo step, the caret after it.
    /// False when there is nothing to link to.
    pub fn apply(&self, e: &mut Editor, to: &str) -> bool {
        let Some(link) = written(to, &self.words) else { return false };
        let (a, b) = self.range;
        if b > e.text().len() || !e.text().is_char_boundary(a) || !e.text().is_char_boundary(b) {
            return false;
        }
        if e.text()[a..b] == link {
            e.set_caret_after(a + link.len());
            return true;
        }
        e.replace_ranges(&[(a, b, link.clone())]);
        e.set_caret_after(a + link.len());
        true
    }

    /// Remove: the link goes, its words stay (selected).
    pub fn remove(&self, e: &mut Editor) -> bool {
        if !self.existing {
            return false;
        }
        let (a, b) = self.range;
        let words = if self.words.is_empty() { target_of_range(e.text(), a, b) } else { self.words.clone() };
        e.replace_ranges(&[(a, b, words.clone())]);
        e.set_selection(Selection { anchor: a, head: a + words.len() });
        true
    }
}

/// The words a link with none of its own shows (`[[Note]]`, an address).
fn target_of_range(text: &str, a: usize, b: usize) -> String {
    let link = &text[a..b];
    match link.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
        Some(inner) => inner.split('|').next_back().unwrap_or("").to_string(),
        None => link.to_string(),
    }
}

/// The address `pasted` would link the selection to, if pasting it should
/// make the selected words a link: a web address (strictly: a scheme or
/// `www.`) over words on one line of text that takes Markdown, not already
/// a link nor the address itself.
pub fn address_over(e: &Editor, pasted: &str) -> Option<String> {
    let s = e.selection();
    let selected = e.selected_text();
    if s.is_caret() || selected.contains('\n') || selected.trim().is_empty() || !e.takes_markdown() {
        return None;
    }
    let url = pasted_address(pasted)?;
    if selected.trim() == pasted.trim() || link_under(e.text(), e.outline(), s).is_some() {
        return None;
    }
    Some(url)
}

/// Pastes a web address over selected words as a link to it (see
/// `address_over`), one undo step, the caret after it. False when it is
/// not such a paste.
pub fn paste_address(e: &mut Editor, pasted: &str) -> bool {
    let Some(url) = address_over(e, pasted) else { return false };
    let s = e.selection();
    let link = format!("[{}]({})", e.selected_text(), as_target(&url));
    e.replace_ranges(&[(s.start(), s.end(), link.clone())]);
    e.set_selection(Selection::caret(s.start() + link.len()));
    true
}

/// Where the link at `at` goes, as the faint card shows it while the
/// pointer rests on it: its web address, or the note's name. None off a
/// link (and on a tag).
pub fn address_at(text: &str, at: usize) -> Option<String> {
    match link_at(text, at)? {
        Link::Web(url) => Some(url.strip_prefix("mailto:").map_or(url.clone(), str::to_string)),
        Link::Note(name) if !name.is_empty() => Some(name),
        _ => None,
    }
}

/// How long the pointer rests on a link before its address shows.
pub const ADDRESS_DELAY_MS: u32 = 400;

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &str, from: &str, to: &str) -> Editor {
        let mut e = Editor::new(text);
        let a = text.find(from).unwrap();
        e.set_selection(Selection { anchor: a, head: a + to.len() });
        e
    }

    #[test]
    fn the_link_field_makes_a_web_link_or_a_note_link() {
        let text = "Read the plan today";
        // A web address over the selected words.
        let mut e = editor(text, "the plan", "the plan");
        let f = LinkField::open(&e);
        assert!(!f.existing && f.words == "the plan" && f.query.text().is_empty());
        assert!(f.apply(&mut e, "kolada.app/notes"));
        assert_eq!(e.text(), "Read [the plan](https://kolada.app/notes) today");
        assert!(e.selection().is_caret() && e.selection().head == "Read [the plan](https://kolada.app/notes)".len());
        assert!(e.undo() && e.text() == text, "one undo step");
        // A note, its name different from the words: an alias.
        let mut e = editor(text, "the plan", "the plan");
        let f = LinkField::open(&e);
        assert_eq!(f.note_typed(), Some(""));
        assert!(f.apply(&mut e, "Q3 Plan"));
        assert_eq!(e.text(), "Read [[Q3 Plan|the plan]] today");
        // The words are the note's name: no alias.
        let mut e = editor("See Q3 Plan", "Q3 Plan", "Q3 Plan");
        assert!(LinkField::open(&e).apply(&mut e, "Q3 Plan"));
        assert_eq!(e.text(), "See [[Q3 Plan]]");
        // Nothing selected: the link at the caret, an address as itself.
        let mut e = Editor::new("Go ");
        e.set_selection(Selection::caret(3));
        assert!(LinkField::open(&e).apply(&mut e, "https://x.io/a"));
        assert_eq!(e.text(), "Go https://x.io/a");
        let mut e = Editor::new("Go ");
        e.set_selection(Selection::caret(3));
        assert!(LinkField::open(&e).apply(&mut e, "Plan"));
        assert_eq!(e.text(), "Go [[Plan]]");
        // Nothing typed: nothing written.
        let mut e = editor(text, "plan", "plan");
        assert!(!LinkField::open(&e).apply(&mut e, "  "));
        assert_eq!(e.text(), text);
        // Words over two lines: the first line's part is linked.
        let mut e = editor("one two\nthree", "two", "two\nthr");
        assert!(LinkField::open(&e).apply(&mut e, "www.x.io"));
        assert_eq!(e.text(), "one [two](https://www.x.io)\nthree");
        // What is typed reads as an address or a note's name.
        assert_eq!(web_address("mailto:a@b.co").as_deref(), Some("mailto:a@b.co"));
        assert_eq!(web_address("Meeting notes 1.2"), None);
        assert_eq!(web_address("v1.2"), None);
        assert_eq!(web_address("Notes.md"), None, "a note's file name is not a site");
        assert_eq!(web_address("kolada.app").as_deref(), Some("https://kolada.app"), "a domain is a domain");
        assert_eq!(written("x.io/a b", "w").as_deref(), Some("[[x.io/a b|w]]"), "with spaces, a note's name");
        assert_eq!(written("https://x.io/a(1)", "w").as_deref(), Some("[w](https://x.io/a%281%29)"));
    }

    #[test]
    fn an_existing_link_opens_filled_and_can_be_removed() {
        let text = "See [the site](https://x.io/a) and [[Plan#Goals|our plan]], [[Inbox]], www.y.io, ![[pic.png]]";
        let at = |s: &str| text.find(s).unwrap() + 1;
        let open = |caret: usize| {
            let mut e = Editor::new(text);
            e.set_selection(Selection::caret(caret));
            (LinkField::open(&e), e)
        };
        let (f, mut e) = open(at("site"));
        assert!(f.existing && f.query.text() == "https://x.io/a" && f.words == "the site");
        assert_eq!(f.query.selection(), Selection { anchor: 0, head: "https://x.io/a".len() }, "selected, to type over");
        // Changing where it goes keeps its words.
        assert!(f.apply(&mut e, "kolada.app"));
        assert!(e.text().starts_with("See [the site](https://kolada.app) and"));
        let (f, _) = open(at("our plan"));
        assert!(f.existing && f.query.text() == "Plan#Goals" && f.words == "our plan");
        let (f, mut e) = open(at("Inbox"));
        assert!(f.existing && f.query.text() == "Inbox" && f.words.is_empty());
        assert!(f.remove(&mut e));
        assert!(e.text().contains(", Inbox, www"), "the link goes, its words stay");
        assert_eq!(e.selected_text(), "Inbox");
        let (f, mut e) = open(at("site"));
        assert!(f.remove(&mut e));
        assert!(e.text().starts_with("See the site and"));
        assert!(e.undo() && e.text() == text, "Remove is one undo step");
        let (f, _) = open(at("y.io"));
        assert!(f.existing && f.query.text() == "www.y.io");
        // A picture's embed is no link to edit.
        let (f, mut e) = open(at("pic.png"));
        assert!(!f.existing && !f.remove(&mut e));
        // A new link has nothing to remove.
        let (f, mut e) = open(1);
        assert!(!f.existing && !f.remove(&mut e));
    }

    #[test]
    fn pasting_an_address_over_a_selection_links_it() {
        let text = "Our site is here.";
        let mut e = editor(text, "site", "site");
        assert!(paste_address(&mut e, " https://kolada.app \n"));
        assert_eq!(e.text(), "Our [site](https://kolada.app) is here.");
        assert!(e.undo() && e.text() == text);
        let mut e = editor(text, "site", "site");
        assert!(paste_address(&mut e, "www.kolada.app"));
        assert_eq!(e.text(), "Our [site](https://www.kolada.app) is here.");
        // Not an address, nothing selected, or a domain without a scheme:
        // a paste as any other.
        for pasted in ["just words", "kolada.app", "https://a b"] {
            let mut e = editor(text, "site", "site");
            assert!(!paste_address(&mut e, pasted), "{pasted}");
            assert_eq!(e.text(), text);
        }
        let mut e = Editor::new(text);
        e.set_selection(Selection::caret(4));
        assert!(!paste_address(&mut e, "https://x.io"));
        // Over two lines, in code, or over a link already: not a link.
        let mut e = editor("a\nb", "a", "a\nb");
        assert!(!paste_address(&mut e, "https://x.io"));
        let code = "```\nsite\n```";
        let mut e = editor(code, "site", "site");
        assert!(!paste_address(&mut e, "https://x.io"));
        let linked = "[site](https://a.io)";
        let mut e = editor(linked, "site", "site");
        assert!(!paste_address(&mut e, "https://x.io"));
    }

    #[test]
    fn resting_on_a_link_says_where_it_goes() {
        let text = "[site](https://x.io) [[Plan|ours]] [m](mailto:a@b.co) #tag plain";
        let at = |s: &str| text.find(s).unwrap() + 1;
        assert_eq!(address_at(text, at("site")).as_deref(), Some("https://x.io"));
        assert_eq!(address_at(text, at("ours")).as_deref(), Some("Plan"));
        assert_eq!(address_at(text, at("m]")).as_deref(), Some("a@b.co"));
        assert_eq!(address_at(text, at("tag")), None);
        assert_eq!(address_at(text, at("plain")), None);
    }
}
