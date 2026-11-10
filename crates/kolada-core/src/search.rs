//! Finding notes as you type: by name first, then by what they say.
//!
//! The index is every note's name and text, lowercased once when it is
//! built (on a worker, after the first frame), so each keystroke is a scan
//! of memory: a couple of milliseconds for thousands of notes.

use std::fs;
use std::path::Path;

use crate::note;

pub struct Indexed {
    /// Vault-relative path, with forward slashes and `.md`.
    pub path: String,
    /// The file name without `.md`.
    pub name: String,
    /// The name and text as searched: see [`fold`].
    name_key: String,
    text: String,
    text_key: String,
}

impl Indexed {
    pub fn new(path: String, text: String) -> Indexed {
        let file = path.rsplit('/').next().unwrap_or(&path);
        let name = file.strip_suffix(".md").unwrap_or(file).to_string();
        Indexed { name_key: fold(&name), text_key: fold(&text), name, path, text }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Text as it is searched: lowercased, accents and marks taken off
/// (`é` is `e`, `ß` is `ss`), and Arabic and Hebrew written without their
/// vowel marks and with one form of each letter (`أ إ آ` are `ا`, `ة` is
/// `ه`, `ى` is `ي`), so a search finds a word however it was typed.
pub fn fold(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        fold_char(c, &mut out);
    }
    out
}

fn fold_char(c: char, out: &mut String) {
    if c.is_ascii() {
        out.push(c.to_ascii_lowercase());
        return;
    }
    for c in c.to_lowercase() {
        match c {
            // Combining accents; Arabic vowel marks, its dagger alif and
            // tatweel; Hebrew points (not its punctuation).
            '\u{0300}'..='\u{036F}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{0640}' => {}
            '\u{0591}'..='\u{05C7}' if !matches!(c, '\u{05BE}' | '\u{05C0}' | '\u{05C3}' | '\u{05C6}') => {}
            'أ' | 'إ' | 'آ' | 'ٱ' => out.push('ا'),
            'ى' | 'ئ' => out.push('ي'),
            'ة' => out.push('ه'),
            'ؤ' => out.push('و'),
            'ß' => out.push_str("ss"),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            'þ' => out.push_str("th"),
            c @ ('\u{00C0}'..='\u{017F}' | '\u{0219}' | '\u{021B}') => {
                out.push(LATIN.iter().find(|(_, from)| from.contains(c)).map_or(c, |(to, _)| *to))
            }
            c => out.push(c),
        }
    }
}

/// Latin letters with accents, by the letter under them.
const LATIN: &[(char, &str)] = &[
    ('a', "àáâãäåāăą"),
    ('c', "çćĉċč"),
    ('d', "ďđð"),
    ('e', "èéêëēĕėęě"),
    ('g', "ĝğġģ"),
    ('h', "ĥħ"),
    ('i', "ìíîïĩīĭįı"),
    ('j', "ĵ"),
    ('k', "ķ"),
    ('l', "ĺļľŀł"),
    ('n', "ñńņňŉ"),
    ('o', "òóôõöøōŏő"),
    ('r', "ŕŗř"),
    ('s', "śŝşšș"),
    ('t', "ţťŧț"),
    ('u', "ùúûüũūŭůűų"),
    ('w', "ŵ"),
    ('y', "ýÿŷ"),
    ('z', "źżž"),
];

/// Where `query` is in `text`, as a search finds it (see [`fold`]): each
/// match's byte range in `text` itself, in order, none overlapping. An empty
/// query matches nothing.
pub fn find(text: &str, query: &str) -> Vec<(usize, usize)> {
    let q = fold(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    let key = fold(text);
    let mut out = Vec::new();
    // ASCII folds byte for byte: its offsets are the text's own.
    if text.is_ascii() {
        out.extend(key.match_indices(q.as_str()).map(|(at, m)| (at, at + m.len())));
        return out;
    }
    // Matches come in order, so one walk along the text maps them all.
    let mut walk = Walk::new(text);
    let mut from = 0;
    while let Some(at) = key[from..].find(&q).map(|i| from + i) {
        let (a, mut b) = (walk.to(at), walk.to(at + q.len()));
        // The marks on its last letter (vowel marks, a tatweel, accents) are
        // the word's: a range never ends inside a letter's cluster.
        let mut buf = String::new();
        for c in text[b..].chars() {
            buf.clear();
            fold_char(c, &mut buf);
            if !buf.is_empty() {
                break;
            }
            b += c.len_utf8();
        }
        if a < b {
            out.push((a, b));
        }
        from = at + q.len();
    }
    out
}

/// Where in `text` its folded form's byte `key_at` comes from.
fn source_offset(text: &str, key_at: usize) -> usize {
    Walk::new(text).to(key_at)
}

/// A walk along a text and its folded form together, forward only.
struct Walk<'a> {
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    /// The folded bytes of the characters walked past.
    folded: usize,
    end: usize,
    buf: String,
}

impl<'a> Walk<'a> {
    fn new(text: &'a str) -> Walk<'a> {
        Walk { chars: text.char_indices().peekable(), folded: 0, end: text.len(), buf: String::new() }
    }

    /// Where in the text its folded form's byte `key_at` comes from: the
    /// first character whose fold starts there or after. `key_at` never
    /// goes back between calls.
    fn to(&mut self, key_at: usize) -> usize {
        while let Some(&(i, c)) = self.chars.peek() {
            if self.folded >= key_at {
                return i;
            }
            self.buf.clear();
            fold_char(c, &mut self.buf);
            self.folded += self.buf.len();
            self.chars.next();
        }
        self.end
    }
}

/// Every note in the vault, read. Hidden folders are skipped.
pub fn index(vault: &Path) -> Vec<Indexed> {
    let mut out = Vec::new();
    let mut dirs = vec![vault.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => dirs.push(path),
                Ok(_) if name.ends_with(".md") => {
                    let Some(rel) = crate::vault::relative(vault, &path) else { continue };
                    let text = note::load(&path).map(|(t, _)| t).unwrap_or_default();
                    out.push(Indexed::new(rel, text));
                }
                _ => {}
            }
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub path: String,
    pub name: String,
    /// For a match in the text: the words around it.
    pub snippet: Option<String>,
}

/// About this many characters of text around a match in its snippet.
const SNIPPET: usize = 64;

/// What joins a table row's cells in a snippet.
pub const CELLS: &str = " \u{00B7} ";

/// The line holding byte `at` of `text` as it reads (see [`readable`]),
/// cut at words to about `SNIPPET` characters around it, with "…" at each
/// end that was cut. The ends are the line's in its logical order: a line
/// read right to left is cut the same way, and drawn with its ellipsis
/// where its direction puts it.
fn snippet(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    let table = piped(text[start..end].trim_end_matches('\r')) && [text[..start.saturating_sub(1)].rsplit('\n').next(), text.get(end + 1..).and_then(|t| t.split('\n').next())].into_iter().flatten().any(piped);
    // A long line is read only near the match (its marks take time that
    // grows faster than the line): a snippet is a few words of it.
    let start = floor_boundary(text, at.saturating_sub(WINDOW).max(start));
    let end = ceil_boundary(text, (at + WINDOW).min(end));
    let line = text[start..end].trim_end_matches('\r');
    let (shown, from) = readable(line, table);
    // Where the match is in what shows.
    let m = from.iter().position(|&s| s >= at - start).unwrap_or(shown.len());
    let m = (0..=m).rev().find(|&i| shown.is_char_boundary(i)).unwrap_or(0);
    around(&shown, m, SNIPPET)
}

/// How many bytes of a line either side of a match a snippet reads.
const WINDOW: usize = 1024;

fn floor_boundary(text: &str, mut i: usize) -> usize {
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(text: &str, mut i: usize) -> usize {
    while !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Whether a line holds a pipe that is not escaped: with a line like it
/// above or below, a table's row.
fn piped(line: &str) -> bool {
    line.trim_start().starts_with('|') || kolada_markdown::raw_cells(line).len() > 1
}

/// A line of a note as it reads where it cannot be drawn (a result's
/// snippet): its block's mark and inline marks dropped, math read as
/// text, a table row's cells joined by [`CELLS`]. With it, for each of its
/// bytes, the byte of the line it came from.
pub fn readable(line: &str, table: bool) -> (String, Vec<usize>) {
    let mut out = String::new();
    let mut from = Vec::new();
    if table {
        for (a, b) in kolada_markdown::cells(line) {
            let (cell, cell_from) = inline(&line[a..b], a);
            let cell_t = cell.trim();
            if cell_t.is_empty() || cell_t.chars().all(|c| matches!(c, '-' | ':')) {
                continue;
            }
            if !out.is_empty() {
                out.push_str(CELLS);
                from.extend(std::iter::repeat_n(a, CELLS.len()));
            }
            let lead = cell.len() - cell.trim_start().len();
            out.push_str(cell_t);
            from.extend_from_slice(&cell_from[lead..lead + cell_t.len()]);
        }
    } else {
        let (block, mark, _) = kolada_markdown::block(line);
        if block.has_spans() {
            (out, from) = inline(&line[mark..], mark);
        } else {
            out.push_str(line);
            from.extend(0..line.len());
        }
    }
    // Trimmed, as it reads.
    let lead = out.len() - out.trim_start().len();
    let keep = out.trim().len();
    (out[lead..lead + keep].to_string(), from[lead..lead + keep].to_vec())
}

/// A line's text after its block mark (`base` bytes in), with its inline
/// marks dropped and its math read as text; each byte's source.
fn inline(content: &str, base: usize) -> (String, Vec<usize>) {
    use kolada_markdown::Style;
    let spans = kolada_markdown::spans(content);
    let mut hidden = vec![false; content.len()];
    let mut math = Vec::new();
    for s in &spans {
        if s.style == Style::Math {
            math.push(*s);
        } else {
            for (a, b) in s.marks() {
                hidden[a..b].iter_mut().for_each(|h| *h = true);
            }
        }
    }
    let (mut out, mut from) = (String::new(), Vec::new());
    let mut i = 0;
    while i < content.len() {
        if let Some(m) = math.iter().find(|m| m.start == i) {
            let read = kolada_markdown::math::linear(&content[m.text_start..m.text_end]);
            from.extend(std::iter::repeat_n(base + i, read.len()));
            out.push_str(&read);
            i = m.end;
            continue;
        }
        let c = content[i..].chars().next().unwrap_or(' ');
        if !hidden[i] {
            out.push(c);
            from.extend(std::iter::repeat_n(base + i, c.len_utf8()));
        }
        i += c.len_utf8();
    }
    (out, from)
}

/// Marks that belong to the letter before them (accents, vowel marks,
/// joiners, variation selectors, skin tones): a cut never comes before
/// one.
pub fn is_mark(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}' | '\u{0483}'..='\u{0489}' | '\u{0591}'..='\u{05C7}' | '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}'
        | '\u{06D6}'..='\u{06ED}' | '\u{0900}'..='\u{0903}' | '\u{093A}'..='\u{094F}' | '\u{0E31}' | '\u{0E34}'..='\u{0E3A}' | '\u{0E47}'..='\u{0E4E}'
        | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' | '\u{200C}' | '\u{200D}' | '\u{20D0}'..='\u{20FF}' | '\u{FE00}'..='\u{FE0F}'
        | '\u{FE20}'..='\u{FE2F}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0100}'..='\u{E01EF}')
}

/// What a cut drops from the end it leaves: spaces, and the punctuation
/// that would sit against the ellipsis.
const LOOSE: &[char] = &[' ', '\t', ',', ';', ':', '\u{00B7}', '-', '\u{2013}', '\u{2014}', '\u{060C}', '\u{061B}'];

/// Whether a word starts at byte `i` of `s`: after a space, at a letter
/// (not a mark of the one before, not a cell's dot).
fn starts_word(s: &str, i: usize) -> bool {
    let Some(c) = s[i..].chars().next() else { return false };
    !c.is_whitespace() && !is_mark(c) && c != '\u{00B7}' && s[..i].chars().next_back().is_none_or(char::is_whitespace)
}

/// Where a word of `s` starts, at or after byte `from` and up to `at`; if
/// none does, where the word holding `at` starts.
pub fn word_start(s: &str, from: usize, at: usize) -> usize {
    let at = at.min(s.len());
    if let Some(i) = (from..=at).find(|&i| s.is_char_boundary(i) && starts_word(s, i)) {
        return i;
    }
    s[..at].char_indices().rev().find(|&(i, _)| starts_word(s, i)).map_or(0, |(i, _)| i)
}

/// The last place in `s` at or before byte `to` where a word ends (a space
/// follows it, or the text does), past byte `after`; None if no word ends
/// there.
pub fn word_end_before(s: &str, after: usize, to: usize) -> Option<usize> {
    let to = to.min(s.len());
    let ends = |i: usize| s.is_char_boundary(i) && s[i..].chars().next().is_none_or(char::is_whitespace) && s[..i].chars().next_back().is_some_and(|c| !c.is_whitespace());
    (after + 1..=to).rev().find(|&i| ends(i))
}

/// The start of the letter holding byte `i` of `s` (a char boundary): a
/// cut there leaves no mark without its letter, and splits no joined
/// emoji.
pub fn letter_start(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    while i > 0 && (s[i..].chars().next().is_some_and(is_mark) || s[..i].ends_with('\u{200D}')) {
        i -= s[..i].chars().next_back().map_or(1, char::len_utf8);
    }
    i
}

/// `s` cut at words to about `room` characters, keeping byte `m` (a match)
/// a little in from the start when it is far along: "…" at each end cut.
fn around(s: &str, m: usize, room: usize) -> String {
    if s.chars().count() <= room {
        return s.to_string();
    }
    let ahead = s[..m].chars().count();
    let a = if ahead > room / 2 {
        let from = s[..m].char_indices().rev().nth(room / 3 - 1).map_or(0, |(i, _)| i);
        // A word's start; in words too long for the room (or a script
        // written without spaces), between two letters.
        let a = word_start(s, from, m);
        if s[a..m].chars().count() > room / 2 { letter_start(s, from) } else { a }
    } else {
        0
    };
    let e = s[a..].char_indices().nth(room).map_or(s.len(), |(i, _)| a + i);
    let b = if e >= s.len() {
        s.len()
    } else {
        // The last word to end in the room, past the match; else the
        // match's own word whole, or cut between letters when it is long.
        word_end_before(s, m, e).unwrap_or_else(|| match s[e..].find(char::is_whitespace) {
            Some(i) if s[e..e + i].chars().count() <= room / 4 => e + i,
            None if s[e..].chars().count() <= room / 4 => s.len(),
            _ => Some(letter_start(s, e)).filter(|&i| i > m).unwrap_or(s.len()),
        })
    };
    let mut out = String::new();
    if a > 0 {
        out.push('\u{2026}');
    }
    let kept = &s[a..b];
    out.push_str(if b < s.len() { kept.trim_end_matches(LOOSE) } else { kept });
    if b < s.len() {
        out.push('\u{2026}');
    }
    out
}

/// Notes matching every word of `query`: those whose name has them all,
/// best first (name starts with the query, then contains it, then has the
/// words anywhere), then those whose text has them, the most fitting first
/// (see `score`), with a snippet. When nothing has them, names with the
/// query's letters in order, so a typo still finds its note.
pub fn search(notes: &[Indexed], query: &str, limit: usize) -> Vec<Hit> {
    let q = fold(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut by_name: Vec<(u8, usize, &Indexed)> = notes
        .iter()
        .filter(|n| words.iter().all(|w| n.name_key.contains(w)))
        .map(|n| {
            let rank = if n.name_key.starts_with(&q) { 0 } else if n.name_key.contains(&q) { 1 } else { 2 };
            (rank, n.name.len(), n)
        })
        .collect();
    by_name.sort_by(|a, b| (a.0, a.1, &a.2.name_key).cmp(&(b.0, b.1, &b.2.name_key)));
    let mut hits: Vec<Hit> =
        by_name.iter().take(limit).map(|(_, _, n)| Hit { path: n.path.clone(), name: n.name.clone(), snippet: None }).collect();
    if hits.len() < limit {
        let mut by_text: Vec<(u32, &Indexed)> = notes
            .iter()
            .filter(|n| !words.iter().all(|w| n.name_key.contains(w)) && words.iter().all(|w| n.text_key.contains(w)))
            .map(|n| (score(&n.text_key, &q, &words), n))
            .collect();
        // Best first; equals keep the vault's order.
        by_text.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, n) in by_text.into_iter().take(limit - hits.len()) {
            let at = best_place(&n.text_key, &q, words[0]);
            hits.push(Hit { path: n.path.clone(), name: n.name.clone(), snippet: Some(snippet(&n.text, source_offset(&n.text, at))) });
        }
    }
    if hits.is_empty() && q.chars().count() >= 3 && words.len() == 1 {
        let mut near: Vec<&Indexed> = notes.iter().filter(|n| subsequence(&n.name_key, &q)).collect();
        near.sort_by_key(|n| n.name.chars().count());
        hits = near.into_iter().take(limit).map(|n| Hit { path: n.path.clone(), name: n.name.clone(), snippet: None }).collect();
    }
    hits
}

/// How well a note's (folded) text fits the query: the whole query as
/// written, a heading holding the words, words standing whole rather than
/// inside others, and how often they come, each counting less as it adds up.
fn score(text: &str, q: &str, words: &[&str]) -> u32 {
    let mut score = 0;
    if words.len() > 1 && text.contains(q) {
        score += 40;
    }
    // Headings, found by their marks rather than reading every line.
    let heads = text.starts_with('#').then_some(0).into_iter().chain(text.match_indices("\n#").map(|(i, _)| i + 1));
    if heads.map(|i| &text[i..text[i..].find('\n').map_or(text.len(), |e| i + e)]).any(|l| words.iter().all(|w| l.contains(w))) {
        score += 25;
    }
    for w in words {
        let (mut all, mut whole) = (0, 0);
        for (i, _) in text.match_indices(w).take(12) {
            all += 1;
            if is_whole(text, i, w.len()) {
                whole += 1;
            }
        }
        score += tally(all) * 2 + tally(whole) * 3;
    }
    score
}

/// 1, 2, 3, ... counting less and less: 10 matches are not ten times one.
fn tally(n: u32) -> u32 {
    match n {
        0 => 0,
        1 => 4,
        2 => 6,
        3..=5 => 8,
        _ => 10,
    }
}

/// Whether `text[at..at + len]` is a word of its own, not part of one.
fn is_whole(text: &str, at: usize, len: usize) -> bool {
    let before = text[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
    let after = text[at + len..].chars().next().is_none_or(|c| !c.is_alphanumeric());
    before && after
}

/// Where a snippet shows the match best: the whole query, else the first
/// word standing whole, else where it first comes.
fn best_place(text: &str, q: &str, first: &str) -> usize {
    text.find(q)
        .or_else(|| text.match_indices(first).find(|(i, _)| is_whole(text, *i, first.len())).map(|(i, _)| i))
        .or_else(|| text.find(first))
        .unwrap_or(0)
}

/// Whether the letters of `query` appear in `name` in order.
fn subsequence(name: &str, query: &str) -> bool {
    let mut chars = name.chars();
    query.chars().all(|q| chars.any(|c| c == q))
}

/// The first letter of each word of `name`.
fn initials(name: &str) -> String {
    name.split(|c: char| !c.is_alphanumeric()).filter_map(|w| w.chars().next()).collect()
}

/// How well `name` matches `q` (both lowercased), best first: 0 the same,
/// 1 starts with it, 2 has a word that does, 3 its words' initials do, 4
/// contains it, 5 has its letters in order; None, not at all.
pub fn rank(name: &str, q: &str) -> Option<u8> {
    Some(if name == q {
        0
    } else if name.starts_with(q) {
        1
    } else if name.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(q)) {
        2
    } else if initials(name).starts_with(q) {
        3
    } else if name.contains(q) {
        4
    } else if subsequence(name, q) {
        5
    } else {
        return None;
    })
}

/// Note names for a `[[link]]` being typed: exact first, then names that
/// start with the query, have a word that does, whose words' initials do,
/// that contain it, or have its letters in order; shorter names first
/// within each.
pub fn complete<'a>(notes: &'a [Indexed], query: &str, limit: usize) -> Vec<&'a Indexed> {
    let q = fold(query.trim());
    let mut ranked: Vec<(u8, usize, &Indexed)> = notes
        .iter()
        .filter_map(|n| Some((if q.is_empty() { 5 } else { rank(&n.name_key, &q)? }, n.name.chars().count(), n)))
        .collect();
    ranked.sort_by(|a, b| (a.0, a.1, &a.2.name_key).cmp(&(b.0, b.1, &b.2.name_key)));
    ranked.into_iter().take(limit).map(|(_, _, n)| n).collect()
}

// Files other than notes: their text, read once.

/// The largest file whose text is read for search.
pub const LARGEST_FILE: u64 = 100 * 1024 * 1024;

/// A file in the vault, not a note, whose text search reads: a PDF, or a
/// picture (by the words in it, where the OS can read them).
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    /// Vault-relative, `/` between folders.
    pub rel: String,
    pub path: std::path::PathBuf,
    pub size: u64,
    /// When it last changed, seconds since 1970.
    pub changed: u64,
}

/// The vault's PDFs and pictures (hidden folders skipped, none over
/// `LARGEST_FILE`, no picture under `SMALLEST_PICTURE`: icons and
/// bullets hold no words worth finding).
pub fn files(vault: &Path) -> Vec<File> {
    let mut out = Vec::new();
    let mut dirs = vec![vault.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => dirs.push(path),
                Ok(_) if name.to_ascii_lowercase().ends_with(".pdf") || is_picture(&name) => {
                    let Ok(meta) = entry.metadata() else { continue };
                    let Some(rel) = crate::vault::relative(vault, &path) else { continue };
                    let changed = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                    if meta.len() <= LARGEST_FILE && (meta.len() >= SMALLEST_PICTURE || !is_picture(&name)) {
                        out.push(File { rel, path, size: meta.len(), changed });
                    }
                }
                _ => {}
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

/// The text read from files, kept in the app's data (never the vault) so
/// each is read once: by where it is, its size and when it changed.
#[derive(Debug, Default, PartialEq)]
pub struct TextCache {
    entries: std::collections::HashMap<String, (u64, u64, String)>,
}

impl TextCache {
    /// Where a vault's cache is kept, beside its notes' past.
    pub fn path(app_data: &Path, vault: &Path) -> std::path::PathBuf {
        let key = crate::history::dir(app_data, vault).file_name().map(|k| k.to_string_lossy().into_owned()).unwrap_or_default();
        app_data.join("search").join(format!("{key}.txt"))
    }

    pub fn load(path: &Path) -> TextCache {
        let bytes = fs::read(path).unwrap_or_default();
        let mut entries = std::collections::HashMap::new();
        let mut at = 0;
        // Each: `rel<TAB>size<TAB>changed<TAB>length<LF>` then the text.
        while let Some(end) = bytes[at..].iter().position(|&b| b == b'\n').map(|e| at + e) {
            let head = String::from_utf8_lossy(&bytes[at..end]);
            let f: Vec<&str> = head.split('\t').collect();
            let [rel, size, changed, len] = f[..] else { break };
            let (Ok(size), Ok(changed), Ok(len)) = (size.parse(), changed.parse(), len.parse::<usize>()) else { break };
            let Some(text) = bytes.get(end + 1..end + 1 + len) else { break };
            entries.insert(rel.to_string(), (size, changed, String::from_utf8_lossy(text).into_owned()));
            at = end + 1 + len;
        }
        TextCache { entries }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut out = Vec::new();
        let mut keys: Vec<&String> = self.entries.keys().collect();
        keys.sort();
        for rel in keys {
            let (size, changed, text) = &self.entries[rel];
            out.extend_from_slice(format!("{}\t{size}\t{changed}\t{}\n", rel.replace(['\t', '\n'], " "), text.len()).as_bytes());
            out.extend_from_slice(text.as_bytes());
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, out)
    }

    /// The text kept for `f`, if it was read as it is now.
    pub fn get(&self, f: &File) -> Option<&str> {
        self.entries.get(&f.rel).filter(|(size, changed, _)| *size == f.size && *changed == f.changed).map(|(_, _, t)| t.as_str())
    }

    pub fn put(&mut self, f: &File, text: String) {
        self.entries.insert(f.rel.clone(), (f.size, f.changed, text));
    }

    /// Forgets files no longer in `files`.
    pub fn keep_only(&mut self, files: &[File]) {
        self.entries.retain(|rel, _| files.iter().any(|f| f.rel == *rel));
    }
}

/// Pictures under this many bytes are not read for words.
pub const SMALLEST_PICTURE: u64 = 16 * 1024;

/// Whether a file is a picture whose words search can read.
pub fn is_picture(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".bmp", ".gif", ".tif", ".tiff", ".webp"].iter().any(|e| lower.ends_with(e))
}

/// Whether a hit is a note (else a file such as a PDF, opened by its app).
pub fn is_note(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".md")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A search's matches, in the text as written: however a word was
    /// typed (case, accents, Arabic letter forms and vowel marks, tatweel),
    /// in mixed Arabic and Latin, every one, and none for an empty search.
    #[test]
    fn matches_are_found_in_the_text_as_written() {
        let at = |text: &str, q: &str| find(text, q).into_iter().map(|(a, b)| text[a..b].to_string()).collect::<Vec<_>>();
        assert_eq!(at("Door code for the Studio: 4931, studio B", "studio"), vec!["Studio", "studio"]);
        assert_eq!(at("Café crème", "creme"), vec!["crème"]);
        assert_eq!(at("جدول الإجتماع القادم: Sprint 14", "الاجتماع"), vec!["الإجتماع"]);
        assert_eq!(at("موعد الاجْتِمَاع مع فريق التصميم", "الاجتماع"), vec!["الاجْتِمَاع"], "vowel marks inside the word");
        assert_eq!(at("مدرسة جميلة", "مدرسه"), vec!["مدرسة"]);
        assert_eq!(at("Notes: الاجتماع then Meeting", "meeting"), vec!["Meeting"]);
        // The marks on a match's last letter, and a tatweel after it, are the
        // word's: a tint never stops inside a letter's cluster.
        assert_eq!(at("كِتَابٌ جديد", "كتاب"), vec!["كِتَابٌ"]);
        assert_eq!(at("كتابـ جديد", "كتاب"), vec!["كتابـ"]);
        assert_eq!(at("cafe\u{301} noir", "cafe"), vec!["cafe\u{301}"]);
        assert!(find("anything", "  ").is_empty());
        assert!(find("abc", "abcd").is_empty());
    }

    /// Finding is one walk along the text, however many matches: a
    /// megabyte's note, Latin or Arabic, has every match of one letter
    /// found in a few seconds at worst (it was minutes, walked from the
    /// start for each match; a tint is drawn as a note opens).
    #[test]
    fn a_big_notes_matches_are_found_in_one_walk() {
        for (unit, q) in [("the cat sat on the mat. ", "e"), ("مدرسة جميلة كِتَابٌ é ", "ة")] {
            let text = unit.repeat(1_000_000 / unit.len());
            let t = std::time::Instant::now();
            let found = find(&text, q);
            let took = t.elapsed();
            assert_eq!(found.len(), text.matches(q).count(), "{q}");
            assert!(found.iter().all(|&(a, b)| text.is_char_boundary(a) && text.is_char_boundary(b) && fold(&text[a..b]).contains(&fold(q))));
            assert!(took < std::time::Duration::from_secs(3), "{q}: {took:?}");
        }
    }

    #[test]
    fn the_vaults_pdfs_are_listed_and_their_text_kept_until_they_change() {
        let dir = std::env::temp_dir().join(format!("kolada-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Papers")).unwrap();
        fs::create_dir_all(dir.join(".trash")).unwrap();
        fs::write(dir.join("Papers/Lease.PDF"), b"%PDF-1.4").unwrap();
        fs::write(dir.join("Papers/Receipt.JPG"), vec![0u8; SMALLEST_PICTURE as usize]).unwrap();
        fs::write(dir.join("icon.png"), b"tiny").unwrap();
        fs::write(dir.join("Note.md"), b"# Note").unwrap();
        fs::write(dir.join(".trash/Old.pdf"), b"%PDF").unwrap();
        let found = files(&dir);
        assert_eq!(found.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>(), ["Papers/Lease.PDF", "Papers/Receipt.JPG"], "not the tiny icon");
        let f = &found[0];
        let mut cache = TextCache::default();
        assert_eq!(cache.get(f), None);
        cache.put(f, "rent is due\non the 1st\twith tabs".into());
        cache.put(&File { rel: "gone.pdf".into(), ..f.clone() }, "x".into());
        cache.keep_only(&found);
        let at = dir.join("cache/search.txt");
        cache.save(&at).unwrap();
        let back = TextCache::load(&at);
        assert_eq!(back, cache);
        assert_eq!(back.get(f), Some("rent is due\non the 1st\twith tabs"));
        assert_eq!(back.get(&File { size: f.size + 1, ..f.clone() }), None, "changed since: read again");
        assert_eq!(TextCache::load(&dir.join("missing")), TextCache::default());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pdf_is_found_by_its_text_under_its_file_name() {
        let notes = vec![Indexed::new("Papers/Lease 2026.pdf".into(), "The rent is due on the first".into()), Indexed::new("Home.md".into(), "rent".into())];
        let hits = search(&notes, "rent due", 5);
        assert_eq!(hits[0].name, "Lease 2026.pdf");
        assert_eq!(hits[0].snippet.as_deref(), Some("The rent is due on the first"));
        assert!(!is_note(&hits[0].path) && is_note("a/B.MD"));
    }

    #[test]
    fn completes_link_names_best_first() {
        let notes: Vec<Indexed> = ["Plan", "Weekly plan", "Plans for 2027", "Pricing ideas", "Explanations", "ملاحظات"]
            .into_iter()
            .map(|n| Indexed::new(format!("{n}.md"), String::new()))
            .collect();
        let names = |q: &str| complete(&notes, q, 10).iter().map(|n| n.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names("plan"), ["Plan", "Plans for 2027", "Weekly plan", "Explanations"]);
        assert_eq!(names("pi"), ["Pricing ideas", "Explanations"]);
        // Letters in order, shortest first.
        assert_eq!(names("pln"), ["Plan", "Weekly plan", "Explanations", "Plans for 2027"]);
        assert_eq!(names("ملا"), ["ملاحظات"]);
        assert_eq!(complete(&notes, "", 3).len(), 3);
    }

    fn notes() -> Vec<Indexed> {
        [
            ("Plans/Weekly plan.md", "# Weekly\n- ship the editor"),
            ("Plan.md", "short"),
            ("Groceries.md", "- milk\n- **oat** milk for the plan"),
            ("Ideas.md", "nothing here"),
        ]
        .into_iter()
        .map(|(p, t)| Indexed::new(p.into(), t.into()))
        .collect()
    }

    #[test]
    fn ranks_names_before_text() {
        let hits = search(&notes(), "plan", 10);
        let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["Plan", "Weekly plan", "Groceries"]);
        assert_eq!(hits[0].snippet, None);
        assert_eq!(hits[2].snippet.as_deref(), Some("oat milk for the plan"));
        assert_eq!(hits[1].path, "Plans/Weekly plan.md");
    }

    #[test]
    fn needs_every_word_and_ignores_case() {
        let hits = search(&notes(), "MILK oat", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "Groceries");
        assert!(search(&notes(), "   ", 10).is_empty());
        assert!(search(&notes(), "zebra", 10).is_empty());
        assert_eq!(search(&notes(), "e", 2).len(), 2, "stops at the limit");
    }

    fn found(notes: &[(&str, &str)], q: &str) -> Vec<String> {
        let notes: Vec<Indexed> = notes.iter().map(|(p, t)| Indexed::new(p.to_string(), t.to_string())).collect();
        search(&notes, q, 10).into_iter().map(|h| h.name).collect()
    }

    #[test]
    fn accents_and_marks_do_not_have_to_be_typed() {
        let notes = [("Café.md", "crème brûlée"), ("Straße.md", ""), ("قراءة.md", "مَدْرَسَة جميلة"), ("Ideas.md", "Über ÆON")];
        assert_eq!(found(&notes, "cafe"), ["Café"]);
        assert_eq!(found(&notes, "CREME brulee"), ["Café"]);
        assert_eq!(found(&notes, "strasse"), ["Straße"]);
        assert_eq!(found(&notes, "قراءه"), ["قراءة"], "ة and ه are one letter to search");
        assert_eq!(found(&notes, "مدرسه"), ["قراءة"], "the vowel marks are not needed");
        assert_eq!(found(&notes, "uber aeon"), ["Ideas"]);
        assert_eq!(fold("أحمد إلى آخر"), "احمد الي اخر");
        assert_eq!(fold("שָׁלוֹם"), "שלום");
    }

    #[test]
    fn the_snippet_shows_the_match_in_the_text_as_written() {
        let notes = vec![Indexed::new("a.md".into(), "Notes\nÉtude über naïve résumé here".into())];
        assert_eq!(search(&notes, "resume", 5)[0].snippet.as_deref(), Some("Étude über naïve résumé here"));
        let long = format!("{}needle", "é".repeat(100));
        let notes = vec![Indexed::new("b.md".into(), long)];
        assert!(search(&notes, "needle", 5)[0].snippet.as_deref().unwrap().ends_with("needle"));
    }

    #[test]
    fn the_best_fitting_text_comes_first() {
        let notes = [
            ("Passing.md", "a meeting about partners somewhere, budget later"),
            ("Heading.md", "# Budget meeting\nnotes"),
            ("Phrase.md", "we had the budget meeting today"),
            ("Inside.md", "budgeting for meetings"),
            ("Often.md", "budget budget budget and a meeting"),
        ];
        assert_eq!(found(&notes, "budget meeting"), ["Heading", "Phrase", "Often", "Passing", "Inside"]);
        let budget = found(&notes, "budget");
        assert_eq!(budget[..2], ["Heading", "Often"], "a heading saying it, then saying it often");
        assert_eq!(budget.last().map(String::as_str), Some("Inside"), "a word inside another comes last");
    }

    #[test]
    fn a_typo_still_finds_a_name_when_nothing_else_does() {
        let notes = [("Groceries.md", ""), ("Grocery list archive.md", ""), ("Gardening.md", "")];
        assert_eq!(found(&notes, "grocries"), ["Groceries"]);
        assert_eq!(found(&notes, "grcy"), ["Grocery list archive"]);
        assert!(found(&notes, "gr zz").is_empty(), "only for a single word");
    }

    /// Each key typed into search scans the whole index: it has to stay
    /// quick for a large vault (release build).
    #[test]
    #[ignore = "timing: cargo test --release -p kolada-core searching_10000_notes -- --ignored --nocapture"]
    fn searching_10000_notes_takes_a_few_milliseconds() {
        let words = ["plan", "garden", "meeting", "budget", "café", "travel", "notes", "idea", "ملاحظة", "week"];
        let start = std::time::Instant::now();
        let notes: Vec<Indexed> = (0..10_000)
            .map(|i| {
                let text: String = (0..300).map(|k| words[(i * 7 + k * 13) % words.len()]).collect::<Vec<_>>().join(" ");
                Indexed::new(format!("folder/note {i}.md"), format!("# Note {i}\n\n{text}"))
            })
            .collect();
        println!("built in {:.0} ms", start.elapsed().as_secs_f64() * 1000.0);
        for q in ["plan", "budget meeting", "cafe", "zzz", "not"] {
            let start = std::time::Instant::now();
            let runs = 20;
            for _ in 0..runs {
                std::hint::black_box(search(&notes, q, 50));
            }
            let ms = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;
            println!("{q:>16}: {ms:.2} ms");
            assert!(ms < 30.0, "{q}: {ms:.1} ms");
        }
    }

    #[test]
    fn cuts_long_lines_around_the_match() {
        let text = format!("{} needle {}", "word ".repeat(40), "tail ".repeat(40));
        let s = snippet(&text, text.find("needle").unwrap());
        assert!(s.starts_with('…') && s.ends_with('…') && s.contains("needle"), "{s}");
        assert!(s.chars().count() <= SNIPPET + 2);
    }

    /// The words a cut snippet keeps: between its ellipses, a run of whole
    /// words of the line.
    fn kept(s: &str) -> &str {
        s.trim_start_matches('\u{2026}').trim_end_matches('\u{2026}')
    }

    /// A snippet reads only near its match: a long line with marks left
    /// open (it took seconds per result) gives its few words at once, cut
    /// at both ends.
    #[test]
    fn a_long_lines_snippet_is_read_near_its_match() {
        for unit in ["*a ", "_a ", "~~a ", "$a$ "] {
            let mut line = unit.repeat(200_000 / unit.len());
            let at = line.len() / 2;
            line.insert_str(at, " lemons ");
            let t = std::time::Instant::now();
            let s = snippet(&line, at + 1);
            assert!(t.elapsed() < std::time::Duration::from_millis(500), "{unit}: {:?}", t.elapsed());
            assert!(s.contains("lemons") && s.starts_with('\u{2026}') && s.ends_with('\u{2026}'), "{unit}: {s}");
        }
    }

    #[test]
    fn a_cut_snippet_ends_at_a_word_with_an_ellipsis_at_each_end_cut() {
        let line = "Oat milk, eggs, a loaf from the corner bakery and two lemons, then a slow afternoon of baking the tart for Sunday lunch with everyone";
        let words: Vec<&str> = line.split_whitespace().collect();
        let whole = |k: &str| {
            let w: Vec<&str> = k.split_whitespace().collect();
            words.windows(w.len()).any(|x| x.iter().zip(&w).enumerate().all(|(i, (a, b))| a == b || (i == w.len() - 1 && a.trim_end_matches([',', ';']) == *b)))
        };
        // Near the start: whole there, cut at the end only.
        let s = snippet(line, line.find("eggs").unwrap());
        assert!(!s.starts_with('\u{2026}') && s.ends_with('\u{2026}'), "{s:?}");
        assert!(whole(kept(&s)) && s.chars().count() <= SNIPPET + 1, "whole words: {s:?}");
        assert!(!kept(&s).ends_with([',', ' ']), "no comma against the ellipsis: {s:?}");
        // Far along: cut at both ends, the match kept.
        let s = snippet(line, line.find("afternoon").unwrap());
        assert!(s.starts_with('\u{2026}') && s.contains("afternoon"), "{s:?}");
        assert!(whole(kept(&s)), "whole words at both ends: {s:?}");
        // Near the end: cut at the start only.
        let s = snippet(line, line.find("everyone").unwrap());
        assert!(s.starts_with('\u{2026}') && !s.ends_with('\u{2026}') && s.ends_with("everyone") && whole(kept(&s)), "{s:?}");
        // A short line: as it is, no ellipsis.
        assert_eq!(snippet("Sugar, lemons, a slow afternoon.", 7), "Sugar, lemons, a slow afternoon.");
        // No spaces to cut at (one long word, or a script written without
        // them): cut between letters, never before a letter's accent.
        let long = format!("{}needle{}", "e\u{301}".repeat(80), "e\u{301}".repeat(80));
        let s = snippet(&long, long.find("needle").unwrap());
        assert!(s.starts_with('\u{2026}') && s.ends_with('\u{2026}') && s.contains("needle"), "{s:?}");
        assert!(s.chars().count() <= SNIPPET + 2 && kept(&s).starts_with('e') && kept(&s).ends_with('\u{301}'), "{s:?}");
    }

    #[test]
    fn a_right_to_left_snippet_is_cut_at_its_words_never_inside_a_letter() {
        // Made-up words, each with its vowel marks: a cut inside one would
        // leave a mark without its letter.
        let word = |i: usize| ["\u{643}\u{650}\u{62A}\u{64E}\u{627}\u{628}\u{64C}", "\u{645}\u{64E}\u{648}\u{652}\u{639}\u{650}\u{62F}", "\u{62C}\u{64E}\u{62F}\u{650}\u{64A}\u{62F}"][i % 3];
        let mut line: Vec<&str> = (0..40).map(word).collect();
        line[24] = "\u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639}";
        let line = line.join(" ");
        let notes = vec![Indexed::new("a.md".into(), format!("# Notes\n{line}"))];
        let s = search(&notes, "\u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639}", 5)[0].snippet.clone().unwrap();
        assert!(s.starts_with('\u{2026}') && s.ends_with('\u{2026}'), "cut at both ends, in logical order");
        let k = kept(&s);
        assert!(k.contains("\u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639}"));
        // Whole words, each with all its marks: no cut inside a letter.
        assert!(k.split(' ').all(|w| (0..3).any(|i| word(i) == w) || w == "\u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639}"), "whole words only: {} words", k.split(' ').count());
        assert!(!k.starts_with(is_mark), "it starts at a letter");
    }

    #[test]
    fn a_table_row_reads_as_its_cells_and_marks_are_dropped() {
        let text = "# Shop\n\n| Fruit | Count |\n| --- | --: |\n| **Lemons** | 4 |\n| Limes | 2 |\n\nBuy *ripe* lemons at [[Market|the market]], `today`, from [the stall](https://example.com) $x^2$\n";
        let notes = vec![Indexed::new("Shop list.md".into(), text.into())];
        let hit = |q: &str| search(&notes, q, 5)[0].snippet.clone().unwrap();
        assert_eq!(hit("lemons"), "Lemons \u{00B7} 4", "a table's row: its cells, joined");
        assert_eq!(hit("count"), "Fruit \u{00B7} Count", "its header too");
        assert_eq!(hit("ripe"), "Buy ripe lemons at the market, today, from the stall x\u{B2}");
        // A line with a pipe and no table around it stays as written.
        let notes = vec![Indexed::new("a.md".into(), "# A\n\nThis or | that, said once\n".into())];
        assert_eq!(search(&notes, "that", 5)[0].snippet.as_deref(), Some("This or | that, said once"));
    }
}
