//! What a search found, shown where it is: a note opened from search
//! opens on its first match, a little above the middle of the window, every
//! match tinted as marked text is (`Ink::Mark`, a fill); the results' rows
//! and the grid's cards tint the words they found the same way. The matcher
//! and its folding are search's own (`kolada_core::search::find`: case,
//! accents, Arabic letter forms, vowel marks and tatweel).
//!
//! The note's tint lasts until the note is first written in, another note
//! opens, or Esc (each shell's to clear, see `Found`); a click or the caret
//! moving keeps it. Its matches are found once, as the note opens: nothing
//! of it is on the way of a keystroke.

use std::rc::Rc;

use kolada_markdown::Line;
use kolada_text::{Format, Layout};

use crate::geom::{Fill, Ink, Rect};
use crate::note::NoteText;
use crate::scene::{self, Eng, Scene};

/// How far down the window the first match's line sits as the note opens:
/// a little above the middle.
pub const OPEN_AT: f32 = 0.4;

/// Where `query` is in `text`, as search finds it: the whole query as
/// written; if it is nowhere whole, each of its words (search finds a note
/// by all its words, anywhere in it). Byte ranges, in order, none
/// overlapping. A word of one letter ("a", "I") is found only where it
/// stands as a word of its own, not inside every word that has it.
pub fn ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    let words: Vec<&str> = query.split_whitespace().collect();
    let find = |w: &str| {
        let found = kolada_core::search::find(text, w);
        if one_letter(w) { found.into_iter().filter(|&r| stands_alone(text, r)).collect() } else { found }
    };
    let whole = if words.len() == 1 {
        find(words[0])
    } else {
        // A query that starts or ends with a one-letter word ("plan a")
        // holds that word apart there too: not "plan already".
        let (first, last) = (words.first().is_some_and(|w| one_letter(w)), words.last().is_some_and(|w| one_letter(w)));
        kolada_core::search::find(text, query).into_iter().filter(|&(a, b)| !(first && joined_before(text, a)) && !(last && joined_after(text, b))).collect()
    };
    if !whole.is_empty() || words.len() < 2 {
        return whole;
    }
    let mut all: Vec<(usize, usize)> = words.iter().flat_map(|w| find(w)).collect();
    all.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(all.len());
    for (a, b) in all {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Whether a word of a query is a single letter, as search reads it.
/// Scripts written without spaces between words (Chinese, Japanese, Thai,
/// Lao, Khmer, Myanmar) have no such single letters: one character there
/// is a word or part of one, found wherever it is.
fn one_letter(word: &str) -> bool {
    let folded = kolada_core::search::fold(word);
    let mut chars = folded.chars();
    match (chars.next(), chars.next()) {
        (None, _) => true,
        (Some(c), None) => !unspaced(c),
        _ => false,
    }
}

/// A character of a script written without spaces between words.
fn unspaced(c: char) -> bool {
    matches!(c as u32,
        0x0E00..=0x0EFF // Thai, Lao
        | 0x1000..=0x109F // Myanmar
        | 0x1780..=0x17FF // Khmer
        | 0x3040..=0x30FF | 0x31F0..=0x31FF // kana
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF) // CJK ideographs
}

/// A letter or digit (or a letter's mark), as joins a word.
fn joined(c: char) -> bool {
    c.is_alphanumeric() || kolada_core::search::is_mark(c)
}

fn joined_before(text: &str, a: usize) -> bool {
    text[..a].chars().next_back().is_some_and(joined)
}

fn joined_after(text: &str, b: usize) -> bool {
    text[b..].chars().next().is_some_and(joined)
}

/// Whether `text[a..b]` is a word of its own: no letter or digit (or a
/// letter's mark) against it on either side.
fn stands_alone(text: &str, (a, b): (usize, usize)) -> bool {
    !joined_before(text, a) && !joined_after(text, b)
}

/// A note opened from search, while its matches show.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    query: String,
    ranges: Vec<(usize, usize)>,
    /// The editor's version they were found in: once the text is another
    /// (an edit, the file read again), they are not shown.
    version: u64,
    /// Where the caret goes: the first match, unless that is in the title
    /// (typing there would rename the note): then under it, as notes open.
    caret: Option<usize>,
    /// The view, still to be put on the first match (the next frame does,
    /// once the note is laid out).
    center: Option<usize>,
}

impl Found {
    /// `query` in the note just opened (`text`, its `lines`, the editor at
    /// `version`); None when it is not in the text at all (a name matched
    /// that the note's title does not have).
    pub fn opened(text: &str, lines: &[Line], query: &str, version: u64) -> Option<Found> {
        let ranges = ranges(text, query);
        ranges.first()?;
        let title = crate::note::title_line(text, lines).and_then(|t| lines.get(t)).map(|l| (l.start, l.end));
        // The frontmatter opens folded: a match in it is not where the caret
        // goes; the view goes to the first one past it, or the title.
        let past = crate::frontmatter::block(lines).map_or(0, |last| (lines[last].end + 1).min(text.len()));
        let Some(&(first, _)) = ranges.iter().find(|r| r.0 >= past) else {
            return Some(Found { query: query.trim().to_string(), caret: None, center: Some(title.map_or(past, |t| t.0)), ranges, version });
        };
        let in_title = title.is_some_and(|(a, b)| first >= a && first <= b);
        Some(Found { query: query.trim().to_string(), caret: (!in_title).then_some(first), center: Some(first), ranges, version })
    }

    /// What was searched for.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The matches to tint in the text at `version`: none once it changed.
    pub fn ranges(&self, version: u64) -> &[(usize, usize)] {
        if version == self.version { &self.ranges } else { &[] }
    }

    /// Whether the text is still the one they were found in.
    pub fn holds(&self, version: u64) -> bool {
        version == self.version
    }

    /// Where the caret goes as the note opens (None: where notes put it).
    pub fn caret(&self) -> Option<usize> {
        self.caret
    }

    /// The match the view is still to be put on, once.
    pub fn take_center(&mut self) -> Option<usize> {
        self.center.take()
    }
}

/// The view that puts byte `at` of the note `OPEN_AT` down a window
/// `height` tall whose note starts at `column_top`: the paragraph at the
/// view's top, and how far below its start (the note's top when that
/// would leave the view's top in the note's head: its dates, the
/// frontmatter's one line, its title). None if `at` has no paragraph.
pub fn view_at(nt: &NoteText, at: usize, column_top: f32, height: f32) -> Option<(usize, f32)> {
    let p = nt.para_at(at).ok()?;
    let (_, y, _) = nt.caret(&p, at).ok()?;
    let want = (height * OPEN_AT - column_top).max(0.0);
    // A top on the frontmatter's folded lines is its one line's slot.
    let (line, px) = nt.settle(p.start, p.pad + y - want);
    // Never with the view's top in the note's head (its dates, the
    // frontmatter's one line, the title): the note opens at its top, as a
    // note does, whole, the match tinted where it is.
    if let Some(title) = nt.title.and_then(|t| nt.lines.get(t)) {
        let title_bottom = nt.para_at(title.start).ok().map_or(0.0, |t| nt.block_height(&t).unwrap_or(0.0));
        if line < title.start || (line == title.start && px < title_bottom) {
            return Some((0, 0.0));
        }
    }
    Some((line, px))
}

/// Tints where `query` is in what the one-line or wrapped layout `l` shows
/// (placed at `x`, `y`), behind its letters, as marked text is tinted; no
/// part of a tint past `left..right`. Only what shows: a word cut short by
/// an ellipsis is not tinted.
pub fn tint(scene: &mut Scene, l: &Layout, x: f32, y: f32, query: &str, (left, right): (f32, f32)) {
    tint_ranges(scene, l, x, y, &ranges(l.cut_text(), query), (left, right));
}

/// Tints `ranges` (bytes of what `l` shows) as `tint` does.
pub fn tint_ranges(scene: &mut Scene, l: &Layout, x: f32, y: f32, ranges: &[(usize, usize)], (left, right): (f32, f32)) {
    for &(a, b) in ranges {
        // (x, y, width, height), a piece per line.
        for [px, top, w, h] in l.range(a, b) {
            let r = Rect::new((x + px - 1.0).max(left), y + top + 1.0, (x + px + w + 1.0).min(right), y + top + h - 1.0);
            if r.right - r.left >= 1.0 {
                scene.fills.push(Fill::solid(r, 3.0, Ink::Mark));
            }
        }
    }
}

/// A snippet for one line: its first match near the start, the words more
/// than `before` letters ahead of it cut, with an ellipsis, so the match
/// shows however narrow the line. Cut at the start of a word (between two
/// letters only in a word too long to keep), never inside a letter.
pub fn lead(snippet: &str, query: &str, before: usize) -> String {
    use kolada_core::search::{letter_start, word_start};
    let Some(&(at, _)) = ranges(snippet, query).first() else { return snippet.to_string() };
    let ahead = snippet[..at].chars().count();
    if ahead <= before {
        return snippet.to_string();
    }
    // From the start of a word, `before` letters or fewer ahead of the match.
    let from = match before {
        0 => at,
        n => snippet[..at].char_indices().rev().nth(n - 1).map_or(0, |(i, _)| i),
    };
    let start = word_start(snippet, from, at);
    let start = if snippet[start..at].chars().count() > before.max(4) * 2 { letter_start(snippet, from) } else { start };
    if start == 0 {
        return snippet.to_string();
    }
    format!("\u{2026}{}", &snippet[start..])
}

/// One line of a snippet, `text`, laid out in `format` in `width` by
/// `height`: where it does not fit, cut at the end of the last word that
/// shows whole, with "…" after it (`cut_at_word`), where the layout's own
/// ellipsis sits: at the end its direction reads to.
pub fn snippet_line(eng: &Eng, text: &str, format: &Format, width: f32, height: f32) -> Rc<Layout> {
    let format = Format { wrap: false, ellipsis: true, ..*format };
    let make = |t: &str| {
        scene::kept_layout(eng, t, &format, width, height, 0, || scene::layout_styled(eng, t, &Format { rtl: Some(scene::is_rtl(t)), ..format }, width, height, &[]))
    };
    let l = make(text);
    match cut_at_word(text, l.cut_text()) {
        Some(cut) => make(&cut),
        None => l,
    }
}

/// Where a one-line layout cut `text` short (`shown`: the start of it that
/// fit, then "…"), the cut moved back to the end of the last word that
/// showed whole (in a word longer than what shows, to the start of a
/// letter), with "…" after it. None when nothing was cut, or the cut is
/// there already.
pub fn cut_at_word(text: &str, shown: &str) -> Option<String> {
    use kolada_core::search::{letter_start, word_end_before};
    if shown == text {
        return None;
    }
    let start = shown.trim_end_matches(['\u{2026}', '\u{200E}', '\u{200F}']);
    let n = start.len();
    if n == 0 || !text.starts_with(start) {
        return None;
    }
    let at = word_end_before(text, 0, n).unwrap_or_else(|| letter_start(text, n));
    let kept = text[..at].trim_end_matches([' ', '\t', ',', ';', ':', '\u{00B7}', '-', '\u{2013}', '\u{2014}', '\u{060C}', '\u{061B}', '\u{2026}']);
    if kept.is_empty() || kept.len() == n {
        return None;
    }
    // A mark of the text's own direction keeps the ellipsis at its end
    // after a word of the other (as the layout's own cut does).
    let mark = if scene::is_rtl(text) { '\u{200F}' } else { '\u{200E}' };
    Some(format!("{kept}{mark}\u{2026}"))
}

#[cfg(test)]
mod tests;
