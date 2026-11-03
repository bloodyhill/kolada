//! What a spell checker reports, as data the screen draws from: the shell
//! asks its own checker (Windows' ISpellChecker, the Mac's NSSpellChecker)
//! and fills this in.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use kolada_markdown::{self as md, Block, Line, Style};

/// Words no dictionary has but every shell's checker should take as spelled:
/// the app's own name, first of all (the welcome note says it often).
pub const OWN_WORDS: &[&str] = &["Kolada", "Ctrl"];

/// Misspelled words by line: the line's text (hashed) to byte ranges in it.
pub type Marks = HashMap<u64, Vec<(u32, u32)>>;

/// What a line of text is called in `Marks`.
pub fn line_key(line: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut h);
    h.finish()
}

/// The text of `line` to check, as UTF-16 with what is not prose blanked,
/// and the byte offset (in the line) of each unit. None for lines that are
/// not prose at all.
pub fn prose(text: &str, line: &Line) -> Option<(Vec<u16>, Vec<u32>)> {
    if matches!(line.block, Block::Fence | Block::Code | Block::Frontmatter | Block::Rule | Block::Math { .. }) {
        return None;
    }
    let s = &text[line.start..line.end];
    if !s.chars().any(char::is_alphabetic) {
        return None;
    }
    let mut blank = vec![false; s.len()];
    blank[..line.mark.min(s.len())].iter_mut().for_each(|b| *b = true);
    for sp in md::line_spans(text, line) {
        let (a, b) = (sp.start - line.start, sp.end - line.start);
        match sp.style {
            Style::Code | Style::WikiLink | Style::Url | Style::Tag | Style::Math => blank[a..b].iter_mut().for_each(|x| *x = true),
            _ => {
                for (m, n) in sp.marks() {
                    blank[m - line.start..n - line.start].iter_mut().for_each(|x| *x = true);
                }
            }
        }
    }
    // Keys to press, "Ctrl+P", "Shift+Alt+Down": not words to spell.
    for (a, b) in chords(s) {
        blank[a..b].iter_mut().for_each(|x| *x = true);
    }
    let mut units = Vec::with_capacity(s.len());
    let mut offsets = Vec::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        let mut buf = [0u16; 2];
        for &u in c.encode_utf16(&mut buf).iter() {
            units.push(if blank[i] { b' ' as u16 } else { u });
            offsets.push(i as u32);
        }
    }
    offsets.push(s.len() as u32);
    Some((units, offsets))
}

/// The keys a modifier names, as notes write them.
const MODIFIERS: &[&str] = &["ctrl", "control", "alt", "shift", "cmd", "command", "option", "opt", "win", "super", "fn", "meta"];

/// The byte ranges in `s` that name keys to press: words joined by `+`,
/// one of them a modifier ("Ctrl+P", "Shift+Alt+F"), or after a Mac
/// modifier's sign ("⌘P").
fn chords(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    for word in s.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ',' | ';' | '"')) {
        let (a, b) = (at, at + word.len());
        at = b + s[b..].chars().next().map_or(0, char::len_utf8);
        // (Bold or marked keys alike: "**⌘P**".)
        let bare = word.trim_matches(|c: char| matches!(c, '*' | '_' | '=' | '~'));
        let mac = bare.starts_with(['\u{2318}', '\u{2325}', '\u{2303}', '\u{21E7}']);
        let joined = word.contains('+') && word.split('+').any(|k| MODIFIERS.contains(&k.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase().as_str()));
        if mac || joined {
            out.push((a, b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prose_of(text: &str) -> String {
        let lines: Vec<Line> = md::lines(text).collect();
        prose(text, &lines[0]).map(|(u, _)| String::from_utf16_lossy(&u)).unwrap_or_default()
    }

    /// `text` with each of `parts` blanked.
    fn blanked(text: &str, parts: &[&str]) -> String {
        parts.iter().fold(text.to_string(), |t, p| t.replacen(p, &" ".repeat(p.chars().count()), 1))
    }

    #[test]
    fn checks_only_prose() {
        assert_eq!(prose_of("- **Teh** plan"), blanked("- **Teh** plan", &["- ", "**", "**"]));
        let text = "see `cdoe` and [[Nmae]] or https://x.io #tgas";
        assert_eq!(prose_of(text), blanked(text, &["`cdoe`", "[[Nmae]]", "https://x.io", "#tgas"]));
        let link = "[the siet](https://x.io)";
        assert_eq!(prose_of(link), blanked(link, &["[", "](https://x.io)"]));
        assert_eq!(prose_of("```rust"), "");
        assert_eq!(prose_of("12 34"), "");
        let (_, offsets) = prose("مرحبا a", &md::lines("مرحبا a").next().unwrap()).unwrap();
        assert_eq!(offsets, vec![0, 2, 4, 6, 8, 10, 11, 12]);
    }

    /// Keys to press are not spelled: "Ctrl+P", "Shift+Alt+F", "⌘P"; a
    /// plus between words is.
    #[test]
    fn keys_to_press_are_not_spelled() {
        let text = "Press **Ctrl+P** or Shift+Alt+F, or \u{2318}P, for every command";
        assert_eq!(prose_of(text), blanked(text, &["**", "Ctrl+P", "**", "Shift+Alt+F", "\u{2318}P"]));
        let welcome = "Press **Ctrl+P** (**\u{2318}P** on a Mac) for every command";
        assert_eq!(prose_of(welcome), blanked(welcome, &["**Ctrl+P**", "**\u{2318}P**"]));
        let plus = "salt+pepper";
        assert_eq!(prose_of(plus), plus);
    }

    #[test]
    fn a_line_has_one_key_and_other_lines_have_others() {
        assert_eq!(line_key("same words"), line_key("same words"));
        assert_ne!(line_key("same words"), line_key("other words"));
    }
}
