//! A note's name is its first line: the title at the top of the page is
//! that line, edited like any other, and the file's name follows it.
//!
//! Notes written elsewhere may be named apart from their text: a file
//! "Pricing ideas" that starts with a paragraph. Such a note opens with its
//! name as a first line over the text. The file gets that line only once it
//! is edited; until then it keeps its text exactly as it was.

use kolada_markdown::{self as md, Block};

/// Where the title line starts: after the frontmatter, if there is any.
pub fn start(text: &str) -> usize {
    if !text.starts_with("---\n") {
        return 0;
    }
    let mut lines = md::lines(text);
    match lines.find(|l| l.block != Block::Frontmatter) {
        Some(l) => l.start,
        // Frontmatter that never closes is text.
        None => 0,
    }
}

/// Letters and digits, lowercased: file names drop or replace what file
/// systems forbid, so names compare on these alone.
fn key(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Whether a file named `name` is named after the line reading `first`: the
/// name is the line, or its start (long lines make shortened names), maybe
/// with a number that told two notes apart.
pub fn named_after(name: &str, first: &str) -> bool {
    let line = key(first);
    let matches = |name: &str| {
        let n = key(name);
        !n.is_empty() && line.starts_with(&n)
    };
    if matches(name) {
        return true;
    }
    // "Meeting 2" is named after "Meeting".
    match name.trim_end().rsplit_once(' ') {
        Some((stem, n)) if n.chars().all(|c| c.is_ascii_digit()) => matches(stem),
        _ => false,
    }
}

/// The text to edit for a file named `name` holding `file`, and the line
/// added over it, if any: the name, when the text does not start with it.
pub fn open(name: &str, file: String) -> (String, Option<String>) {
    let at = start(&file);
    let first = file[at..].lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = name.trim();
    if line.is_empty() || named_after(line, &md::plain(first)) {
        return (file, None);
    }
    let mut text = String::with_capacity(file.len() + line.len() + 1);
    text.push_str(&file[..at]);
    text.push_str(line);
    text.push('\n');
    text.push_str(&file[at..]);
    (text, Some(line.to_string()))
}

/// What to write to the file for `text`, which `open` gave with `added`
/// over it: the text without that line while the line is untouched. None
/// once it was edited: the whole text is the note from then on.
pub fn file_text(text: &str, added: &str) -> Option<String> {
    let at = start(text);
    let rest = &text[at..];
    let end = rest.find('\n').unwrap_or(rest.len());
    if rest[..end] != *added {
        return None;
    }
    let after = (at + end + 1).min(text.len());
    Some(format!("{}{}", &text[..at], &text[after..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_their_first_line_loosely() {
        assert!(named_after("Product launch", "Product launch"));
        assert!(named_after("product launch", "Product Launch!"));
        // Shortened, with what file names forbid replaced.
        assert!(named_after("1412 _ https___www.youtube.com_shorts_tJ", "1412 | https://www.youtube.com/shorts/tJ_Dxo8zVKI"));
        assert!(named_after("Meeting 2", "Meeting"));
        assert!(named_after("ملاحظات الاجتماع", "ملاحظات الاجتماع الأسبوعي"));
        assert!(!named_after("Untitled 32", "npx get add latent-spaces/brag"));
        assert!(!named_after("Pricing ideas", "Three tiers: free, pro and team."));
        assert!(!named_after("", "anything"));
    }

    #[test]
    fn a_note_named_after_its_first_line_opens_as_it_is() {
        for file in ["Product launch\nShip it.", "# Product launch\nShip it.", "\n\nProduct launch"] {
            assert_eq!(open("Product launch", file.to_string()), (file.to_string(), None));
        }
    }

    #[test]
    fn a_note_named_apart_opens_with_its_name_over_the_text() {
        let (text, added) = open("Pricing ideas", "Three tiers.\nAnnual plans.".to_string());
        assert_eq!(text, "Pricing ideas\nThree tiers.\nAnnual plans.");
        assert_eq!(added.as_deref(), Some("Pricing ideas"));
        // Written back without it, whatever else changed.
        let edited = text.replace("Annual", "Yearly");
        assert_eq!(file_text(&edited, "Pricing ideas").as_deref(), Some("Three tiers.\nYearly plans."));
        // Once the name line is edited, it is part of the note.
        assert_eq!(file_text("Pricing notes\nThree tiers.", "Pricing ideas"), None);
        // Joining the text's first line into it counts as an edit: nothing
        // is dropped.
        assert_eq!(file_text("Pricing ideasThree tiers.", "Pricing ideas"), None);
        // An empty note.
        let (text, _) = open("Empty", String::new());
        assert_eq!(text, "Empty\n");
        assert_eq!(file_text(&text, "Empty").as_deref(), Some(""));
    }

    #[test]
    fn the_name_goes_under_frontmatter() {
        let file = "---\ntags: a\n---\nBody".to_string();
        assert_eq!(start(&file), 16);
        let (text, added) = open("Named", file.clone());
        assert_eq!(text, "---\ntags: a\n---\nNamed\nBody");
        assert_eq!(file_text(&text, added.as_deref().unwrap()).as_deref(), Some(file.as_str()));
        // Frontmatter that never closes is not frontmatter.
        assert_eq!(start("---\nno end"), 0);
    }
}
