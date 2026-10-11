//! A note's gist: one line saying what it is about, shown faintly beside
//! its date and never written into the note.

use crate::mask;
use crate::text::{self, Script};
use notsy_markdown::{self as md, Block};

/// Shorter notes say what they are about at a glance.
pub const MIN_WORDS: usize = 150;
/// The model reads a note's opening, not all of it.
const MAX_SOURCE: usize = 6000;
const MAX_CHARS: usize = 120;

const ENDS: [char; 7] = ['.', '!', '?', '。', '！', '？', '؟'];
const QUOTES: [(&str, &str); 9] = [("\"", "\""), ("'", "'"), ("“", "”"), ("‘", "’"), ("„", "“"), ("«", "»"), ("「", "」"), ("『", "』"), ("`", "`")];
/// What a model puts before a gist: only the label goes (`Summary: x` → `x`,
/// `This note is about x` → `x`).
const LABELS: [&str; 15] = [
    "one-line summary:",
    "one line summary:",
    "summary:",
    "gist:",
    "tl;dr:",
    "tldr:",
    "in short:",
    "this note:",
    "this note is about ",
    "the note is about ",
    "this note is ",
    "the note is ",
    "this note ",
    "the note ",
    "note:",
];

/// What of the note the model reads: the plain text (notsy_markdown::plain)
/// of its lines, skipping frontmatter, fences, code, rules and empty lines,
/// joined by \n; capped at 6000 bytes on a char boundary.
pub fn source(note: &str) -> String {
    let mut out = String::new();
    for l in md::lines(note) {
        if matches!(l.block, Block::Frontmatter | Block::Fence | Block::Code | Block::Rule) {
            continue;
        }
        let plain = md::plain(&note[l.start..l.end]);
        let plain = plain.trim();
        if plain.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(plain);
        if out.len() >= MAX_SOURCE {
            break;
        }
    }
    let mut end = out.len().min(MAX_SOURCE);
    while !out.is_char_boundary(end) {
        end -= 1;
    }
    out.truncate(end);
    out
}

/// The model's answer as the gist to show, or None: trimmed; strips wrapping
/// quotes and leading labels (`Summary:`, `Gist:`, `This note`… only the
/// label, e.g. `Summary: x` → `x`); one line; 3–20 words (CJK: 4–40 chars);
/// ≤ 120 chars; the note's dominant script; not equal (case-insensitively)
/// to the title; no markers; ends without trailing `.` if it has only one
/// sentence.
pub fn accept(answer: &str, title: &str, note: &str) -> Option<String> {
    let answer = answer.trim();
    let mut s = unquote(answer.strip_suffix("<|im_end|>").unwrap_or(answer).trim_end());
    if let Some(rest) = after_label(s) {
        s = unquote(rest);
    }
    if !s.trim_end_matches(ENDS).contains(ENDS) {
        s = s.trim_end_matches(['.', '。']).trim_end();
    }
    let script = text::dominant(s);
    let (least, most) = if script == Some(Script::Cjk) { (4, 40) } else { (3, 20) };
    let fits = (least..=most).contains(&text::words(s)) && s.chars().count() <= MAX_CHARS && !s.contains(['\n', '\r']);
    if !fits || has_marker(s) || same(s, title) || stutters(s) {
        return None;
    }
    if let Some(note) = text::dominant(&source(note))
        && script != Some(note)
    {
        return None;
    }
    Some(s.to_string())
}

/// Whether a gist made at `then` words is out of date at `now` words: a
/// change of ≥ 15% (or ≥ 60 words).
pub fn stale(then: usize, now: usize) -> bool {
    let changed = then.abs_diff(now);
    changed > 0 && (changed >= 60 || changed * 100 >= then * 15)
}

fn unquote(s: &str) -> &str {
    QUOTES.iter().find_map(|&(open, close)| s.strip_prefix(open)?.strip_suffix(close)).map_or(s, str::trim)
}

fn after_label(s: &str) -> Option<&str> {
    LABELS.iter().find_map(|l| {
        let rest = s.get(l.len()..)?;
        s[..l.len()].eq_ignore_ascii_case(l).then(|| rest.trim_start())
    })
}

/// A word said twice in a row (`SEO SEO`): a small model stuck, not a
/// gist. CJK characters may repeat (`看看`).
fn stutters(s: &str) -> bool {
    let words: Vec<String> = text::tokens(s).iter().filter(|t| t.kind == text::Kind::Word).map(|t| t.text.to_lowercase()).collect();
    let cjk = |w: &str| w.chars().next().and_then(text::script) == Some(Script::Cjk);
    words.windows(2).any(|w| w[0] == w[1] && !cjk(&w[0]))
}

fn same(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(ENDS).trim_end().to_lowercase();
    norm(a) == norm(b)
}

/// A fix's placeholder: a gist is read from plain text, so either end of
/// one means the model echoed its other task.
fn has_marker(s: &str) -> bool {
    let (one, two) = (mask::marker(1), mask::marker(2));
    let open = one.chars().zip(two.chars()).take_while(|(x, y)| x == y).count();
    let close = one.chars().rev().zip(two.chars().rev()).take_while(|(x, y)| x == y).count();
    let open: String = one.chars().take(open).collect();
    let close: String = one.chars().skip(one.chars().count() - close).collect();
    [open, close].iter().any(|m| !m.is_empty() && s.contains(m.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mask::marker;

    fn note() -> String {
        format!("# Trip to Lisbon\n\n{}", "We fly to Lisbon in May and stay with Ana near the river. ".repeat(20))
    }

    #[test]
    fn the_source_is_the_notes_prose() {
        let note = "---\ntags: travel\n---\n# Trip to Lisbon\n\nWe fly on **May 3** and stay with [[Ana|Ana's family]].\n```\nlet x = 1;\n```\n---\n- [ ] book the hotel #todo\n";
        assert_eq!(source(note), "Trip to Lisbon\nWe fly on May 3 and stay with Ana's family.\nbook the hotel #todo");
        let long = "مرحبا بالعالم ".repeat(1000);
        let s = source(&long);
        assert!(s.len() <= MAX_SOURCE && s.len() > MAX_SOURCE - 4, "{}", s.len());
        assert!(long.starts_with(&s));
        assert_eq!(source(""), "");
    }

    #[test]
    fn gists_are_cleaned_and_checked() {
        let (title, note) = ("Trip to Lisbon", note());
        for (answer, want) in [
            ("Planning a May trip to Lisbon with Ana.", "Planning a May trip to Lisbon with Ana"),
            ("Summary: Planning a May trip to Lisbon with Ana.", "Planning a May trip to Lisbon with Ana"),
            ("  \"Planning a May trip to Lisbon.\"  ", "Planning a May trip to Lisbon"),
            ("Gist: “Flights and a stay with Ana”<|im_end|>", "Flights and a stay with Ana"),
            ("This note is about planning a May trip to Lisbon", "planning a May trip to Lisbon"),
            ("This note covers flights, dates and where to stay", "covers flights, dates and where to stay"),
            ("Flights booked. Hotel still open.", "Flights booked. Hotel still open."),
            ("Where to stay in Lisbon?", "Where to stay in Lisbon?"),
        ] {
            assert_eq!(accept(answer, title, &note).as_deref(), Some(want), "{answer:?}");
        }
        for answer in [
            "",
            "Lisbon trip",
            "Trip to Lisbon",
            "trip to lisbon.",
            "Flights booked, hotel hotel near the river",
            "Summary: Trip to Lisbon",
            "Planning a May trip to Lisbon with Ana, booking flights, finding a hotel near the river and seeing the old city",
            "Internationalization considerations, accessibility requirements, documentation responsibilities, implementation particularities and administrative misunderstandings",
            "Planning a trip\nto Lisbon",
            "التخطيط لرحلة إلى لشبونة في مايو",
        ] {
            assert_eq!(accept(answer, title, &note), None, "{answer:?}");
        }
        assert_eq!(accept(&format!("Planning {} trip to Lisbon", marker(1)), title, &note), None);
    }

    #[test]
    fn gists_follow_the_notes_script() {
        let note = "去里斯本旅行的计划，五月出发，住在朋友家。".repeat(10);
        assert_eq!(accept("五月去里斯本旅行的计划。", "里斯本", &note).as_deref(), Some("五月去里斯本旅行的计划"));
        assert_eq!(accept("旅行", "里斯本", &note), None);
        assert_eq!(accept("A plan for a May trip to Lisbon", "里斯本", &note), None);
        let note = "رحلة إلى لشبونة في شهر مايو مع العائلة. ".repeat(20);
        assert_eq!(accept("خطة رحلة إلى لشبونة في مايو.", "لشبونة", &note).as_deref(), Some("خطة رحلة إلى لشبونة في مايو"));
    }

    #[test]
    fn a_gist_goes_stale_with_the_note() {
        assert!(!stale(200, 200));
        assert!(!stale(200, 229));
        assert!(stale(200, 230));
        assert!(stale(200, 170));
        assert!(!stale(1000, 1059));
        assert!(stale(1000, 1060));
        assert!(stale(1000, 940));
        assert!(stale(0, 150));
        assert!(!stale(0, 0));
    }
}
