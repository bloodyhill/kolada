//! Markdown as Notsy shows it, one paragraph (line) at a time: each line's
//! block (heading, list item, quote, code...) with the length of its block
//! mark, and the inline spans in it (bold, links...) with the ranges of
//! their marks.
//!
//! The editor treats block marks as one unit (the caret never rests inside
//! `- ` or `## `); the view replaces them with bullets, checkboxes and
//! heading styles, and hides inline marks away from the caret. The file on
//! disk stays plain Markdown, compatible with Obsidian.
//!
//! This is deliberately a line-level reading of Markdown, not a full
//! CommonMark parser: it covers what notes use and never guesses across
//! paragraphs, so every line can be styled on its own.

#[cfg(test)]
mod tests;

pub mod html;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Body,
    /// Level 1 to 6.
    Heading(u8),
    Bullet,
    Ordered,
    Task { done: bool },
    Quote,
    /// A code fence line (```), and the lines inside one.
    Fence,
    Code,
    /// A thematic break: `---`, `***` or `___`.
    Rule,
    Frontmatter,
    /// A table's alignment row (`| --- | :-: | --: |`). The plain line just
    /// above it, with as many cells, is the table's header (see
    /// `is_table_header`).
    TableDelimiter,
    /// A row of a table, below its alignment row.
    TableRow,
}

impl Block {
    /// List items: bullets, numbers and tasks.
    pub fn is_item(self) -> bool {
        matches!(self, Block::Bullet | Block::Ordered | Block::Task { .. })
    }

    /// Lines whose text is Markdown (not code or metadata).
    pub fn has_spans(self) -> bool {
        !matches!(self, Block::Fence | Block::Code | Block::Rule | Block::Frontmatter | Block::TableDelimiter)
    }
}

/// How a table column's cells sit: as its alignment row says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// The cells of a table line, as byte ranges of their text (trimmed) within
/// the line: split at each `|` not escaped as `\|`, the outer pipes left off.
pub fn cells(line: &str) -> Vec<(usize, usize)> {
    raw_cells(line)
        .into_iter()
        .map(|(a, z)| {
            let cell = &line[a..z];
            let lead = cell.len() - cell.trim_start().len();
            let text = cell.trim();
            (a + lead, a + lead + text.len())
        })
        .collect()
}

/// The cells of a table line as `cells` finds them, untrimmed: everything
/// between their pipes.
pub fn raw_cells(line: &str) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let mut bars = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'|' => {
                bars.push(i);
                i += 1;
            }
            _ => i += 1,
        }
    }
    let mut bounds = vec![0];
    for &bar in &bars {
        bounds.push(bar);
        bounds.push(bar + 1);
    }
    bounds.push(line.len());
    let mut out: Vec<(usize, usize)> = bounds.chunks(2).map(|w| (w[0], w[1])).collect();
    // A leading or trailing pipe opens or closes the row: no cell outside it.
    let blank = |(a, z): (usize, usize)| line[a..z].trim().is_empty();
    if !bars.is_empty() && out.len() > 1 && blank(out[0]) && line[..bars[0]].trim().is_empty() {
        out.remove(0);
    }
    if let (Some(&last_bar), Some(&last)) = (bars.last(), out.last())
        && out.len() > 1
        && blank(last)
        && line[last_bar + 1..].trim().is_empty()
    {
        out.pop();
    }
    out
}

/// A line with a pipe outside escapes: it could be a table's header or row.
fn piped(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'|' => return true,
            _ => i += 1,
        }
    }
    false
}

/// A table's alignment row: cells of dashes, each with an optional colon at
/// either end, at least one pipe. Its alignments, one per column.
pub fn alignments(line: &str) -> Option<Vec<Align>> {
    if !piped(line) {
        return None;
    }
    cells(line)
        .into_iter()
        .map(|(a, z)| {
            let c = &line[a..z];
            let (left, right) = (c.starts_with(':'), c.ends_with(':') && c.len() > 1);
            let dashes = c.trim_start_matches(':').trim_end_matches(':');
            (!dashes.is_empty() && dashes.bytes().all(|b| b == b'-')).then_some(match (left, right) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            })
        })
        .collect()
}

/// Whether line `i` is a table's header: plain, just above an alignment row.
pub fn is_table_header(lines: &[Line], i: usize) -> bool {
    lines.get(i).is_some_and(|l| l.block == Block::Body) && lines.get(i + 1).is_some_and(|l| l.block == Block::TableDelimiter)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    /// Before the line break.
    pub end: usize,
    pub block: Block,
    /// Bytes of the block mark at the start, indentation included: `- `,
    /// `## `, `\t1. `, `> `.
    pub mark: usize,
    /// Nesting: indentation levels for list items, `>` count for quotes.
    pub depth: u8,
}

impl Line {
    /// Where the line's text starts, after its block mark.
    pub fn content(&self) -> usize {
        self.start + self.mark
    }
}

/// The block of one line outside code and frontmatter, with the length of
/// its mark and its depth.
pub fn block(line: &str) -> (Block, usize, u8) {
    let rest = line.trim_start_matches([' ', '\t']);
    let indent = line.len() - rest.len();
    let lead = &line[..indent];
    let depth = (lead.matches('\t').count() + lead.matches(' ').count() / 2).min(8) as u8;
    if indent == 0 {
        let hashes = rest.bytes().take_while(|&b| b == b'#').count();
        if (1..=6).contains(&hashes) && rest[hashes..].starts_with(' ') {
            let spaces = rest[hashes..].len() - rest[hashes..].trim_start_matches(' ').len();
            return (Block::Heading(hashes as u8), hashes + spaces, 0);
        }
    }
    if indent <= 3 && is_rule(rest) {
        return (Block::Rule, 0, 0);
    }
    if rest.starts_with('>') {
        let (mut at, mut depth) = (0, 0u8);
        while rest[at..].starts_with('>') {
            at += 1;
            depth = depth.saturating_add(1);
            if rest[at..].starts_with(' ') {
                at += 1;
            }
        }
        return (Block::Quote, indent + at, depth.min(8));
    }
    if matches!(rest.bytes().next(), Some(b'-' | b'*' | b'+')) && rest[1..].starts_with(' ') {
        let after = &rest[2..];
        for (task, done) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
            if let Some(tail) = after.strip_prefix(task)
                && (tail.is_empty() || tail.starts_with(' '))
            {
                let mark = indent + 2 + task.len() + tail.starts_with(' ') as usize;
                return (Block::Task { done }, mark, depth);
            }
        }
        return (Block::Bullet, indent + 2, depth);
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) && (rest[digits..].starts_with(". ") || rest[digits..].starts_with(") ")) {
        return (Block::Ordered, indent + digits + 2, depth);
    }
    (Block::Body, 0, 0)
}

fn is_rule(s: &str) -> bool {
    let s = s.trim_end();
    let Some(c) = s.chars().next().filter(|c| matches!(c, '-' | '*' | '_')) else { return false };
    s.chars().all(|x| x == c || x == ' ') && s.chars().filter(|&x| x == c).count() >= 3
}

/// A code fence: up to three spaces, then three or more backticks or
/// tildes. Returns the fence character and its count.
fn fence(line: &str) -> Option<(u8, usize)> {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return None;
    }
    let c = *rest.as_bytes().first()?;
    if c != b'`' && c != b'~' {
        return None;
    }
    let n = rest.bytes().take_while(|&b| b == c).count();
    // An info string after backticks cannot hold another backtick.
    (n >= 3 && !(c == b'`' && rest[n..].contains('`'))).then_some((c, n))
}

/// Where reading lines stands between two: in frontmatter, inside a code
/// fence (its character and length), at the text's first line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LexState {
    frontmatter: bool,
    fence: Option<(u8, usize)>,
    first: bool,
    /// Cells of the line before, if it could head a table; and the columns
    /// of the table being read, if inside one.
    pipes: u16,
    table: u16,
}

impl LexState {
    /// Before the first line of `text`.
    pub fn start(text: &str) -> LexState {
        LexState { frontmatter: has_frontmatter(text), fence: None, first: true, pipes: 0, table: 0 }
    }
}

/// Whether `text` opens with frontmatter: `---` on its first line, closed
/// by `---` (or `...`), and between them only what YAML properties are
/// made of: `key: value` lines, list items, lines indented under them,
/// blank lines. A `---` that opens headings and paragraphs is a rule, and
/// what follows it the note (as Obsidian reads it, where such YAML is no
/// properties).
pub fn has_frontmatter(text: &str) -> bool {
    if text == "---" {
        return true;
    }
    let Some(rest) = text.strip_prefix("---\n") else { return false };
    for line in rest.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line == "---" || line == "..." {
            return true;
        }
        let indented = line.starts_with([' ', '\t']);
        let item = line.trim_start().starts_with("- ") || line.trim() == "-";
        if line.trim().is_empty() || indented || item || yaml_key(line) {
            continue;
        }
        return false;
    }
    false
}

/// A YAML `key: value` (or `key:`) line: a short key, no sentence, before
/// the colon.
fn yaml_key(line: &str) -> bool {
    let Some((key, value)) = line.split_once(':') else { return false };
    let key = key.trim_end();
    !key.is_empty()
        && key.chars().count() <= 40
        && !key.starts_with(['#', '-', '>', '|', '*', '['])
        && !key.contains(['.', '\u{060C}', '\u{061B}', '?', '!', '(', ')'])
        && (value.is_empty() || value.starts_with([' ', '\t']))
}

/// The line of `text` starting at `start`, read in `state`: the line, the
/// state after it, and where the next line starts (None at the end).
pub fn lex_line(text: &str, start: usize, state: LexState) -> (Line, LexState, Option<usize>) {
    let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    let next = (end < text.len()).then_some(end + 1);
    let line = &text[start..end];
    let mut after = LexState { first: false, pipes: 0, table: 0, ..state };
    let (block, mark, depth) = if state.frontmatter {
        if !state.first && (line == "---" || line == "...") {
            after.frontmatter = false;
        }
        (Block::Frontmatter, 0, 0)
    } else if let Some((c, n)) = state.fence {
        let closes = fence(line).is_some_and(|(fc, fn_)| fc == c && fn_ >= n)
            && line.trim_start_matches(' ').trim_start_matches(c as char).trim().is_empty();
        if closes {
            after.fence = None;
            (Block::Fence, 0, 0)
        } else {
            (Block::Code, 0, 0)
        }
    } else if let Some(f) = fence(line) {
        after.fence = Some(f);
        (Block::Fence, 0, 0)
    } else if state.table > 0 && piped(line) {
        // A row of the table being read.
        after.table = state.table;
        (Block::TableRow, 0, 0)
    } else if state.pipes > 0 && alignments(line).is_some_and(|a| a.len() == state.pipes as usize) {
        after.table = state.pipes;
        (Block::TableDelimiter, 0, 0)
    } else {
        let (block, mark, depth) = block(line);
        if block == Block::Body && piped(line) {
            after.pipes = cells(line).len().min(u16::MAX as usize) as u16;
        }
        (block, mark, depth)
    };
    (Line { start, end, block, mark, depth }, after, next)
}

/// Every line of `text` with its block. Text that is empty or ends in a line
/// break has an empty last line.
pub fn lines(text: &str) -> impl Iterator<Item = Line> + '_ {
    lines_with_states(text).map(|(line, _)| line)
}

/// Every line of `text`, with the state reading is in after it (for reading
/// again from any line: see `relex`).
pub fn lines_with_states(text: &str) -> impl Iterator<Item = (Line, LexState)> + '_ {
    let mut next = Some(0);
    let mut state = LexState::start(text);
    std::iter::from_fn(move || {
        let (line, after, following) = lex_line(text, next?, state);
        next = following;
        state = after;
        Some((line, after))
    })
}

/// After `removed` bytes at `at` became `inserted` bytes, brings `lines`
/// and `states` (as `lines_with_states` gave them for the text before) up
/// to date with `text`, the text after. Only lines from the one the edit
/// touched are read again, until they read as before; the rest move by the
/// edit's length.
pub fn relex(text: &str, lines: &mut Vec<Line>, states: &mut Vec<LexState>, at: usize, removed: usize, inserted: usize) {
    // Whether the text opens with frontmatter can turn on an edit anywhere
    // in it: then every line is read again.
    let had = lines.first().is_some_and(|l| l.block == Block::Frontmatter);
    if lines.is_empty() || had != has_frontmatter(text) {
        let (l, s): (Vec<_>, Vec<_>) = lines_with_states(text).unzip();
        (*lines, *states) = (l, s);
        return;
    }
    let delta = inserted as isize - removed as isize;
    // The first line the edit touches, and how reading stood before it.
    let first = lines.partition_point(|l| l.start <= at).max(1) - 1;
    let mut state = if first == 0 { LexState::start(text) } else { states[first - 1] };
    let edit_end = at + inserted;
    let mut read: Vec<(Line, LexState)> = Vec::new();
    let mut pos = Some(lines[first].start);
    let mut resume = None;
    while let Some(start) = pos {
        // Past the edit, a line the old text had at the same place, read
        // from the same state, reads the same: the rest follows as it was.
        if start > edit_end || (start == edit_end && inserted > 0 && start > at) {
            let old_start = (start as isize - delta) as usize;
            let j = lines.partition_point(|l| l.start < old_start);
            let before_j = if j == 0 { None } else { Some(states[j - 1]) };
            if j > first && j < lines.len() && lines[j].start == old_start && before_j == Some(state) {
                resume = Some(j);
                break;
            }
        }
        let (line, after, next) = lex_line(text, start, state);
        read.push((line, after));
        state = after;
        pos = next;
    }
    let tail = match resume {
        Some(j) => j,
        None => lines.len(),
    };
    let shift = |l: Line| Line { start: (l.start as isize + delta) as usize, end: (l.end as isize + delta) as usize, ..l };
    let (new_lines, new_states): (Vec<Line>, Vec<LexState>) = read.into_iter().unzip();
    let rest_lines: Vec<Line> = lines[tail..].iter().map(|&l| shift(l)).collect();
    let rest_states: Vec<LexState> = states[tail..].to_vec();
    lines.truncate(first);
    lines.extend(new_lines);
    lines.extend(rest_lines);
    states.truncate(first);
    states.extend(new_states);
    states.extend(rest_states);
}

/// What Enter continues a line with: the mark for the next line, given the
/// line's text and block. None for blocks that do not continue.
pub fn continuation(line: &str, block: Block, mark: usize) -> Option<String> {
    let m = &line[..mark];
    let rest = m.trim_start_matches([' ', '\t']);
    let lead = &m[..m.len() - rest.len()];
    match block {
        Block::Bullet => Some(m.to_string()),
        Block::Task { .. } => Some(format!("{lead}{}[ ] ", &rest[..2])),
        Block::Ordered => {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            let n: u64 = rest[..digits].parse().ok()?;
            Some(format!("{lead}{}{}", n + 1, &rest[digits..]))
        }
        Block::Quote => Some(m.to_string()),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Strong,
    Emphasis,
    Strike,
    Code,
    Highlight,
    /// `[[note]]`, `[[note|shown text]]`, `![[embed]]`.
    WikiLink,
    /// `[text](url)`.
    Link,
    /// `#tag`: shown as it is, with no marks to hide.
    Tag,
    /// A web address written out (`https://…`, `www.…`): a link as it is.
    Url,
}

/// An inline span, in byte offsets into the text it was found in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub style: Style,
    /// The span with its marks.
    pub start: usize,
    pub end: usize,
    /// The text shown between the marks.
    pub text_start: usize,
    pub text_end: usize,
}

impl Span {
    /// The mark before the text and the one after (for a link, `](url)`).
    pub fn marks(&self) -> [(usize, usize); 2] {
        [(self.start, self.text_start), (self.text_end, self.end)]
    }

    fn at(self, offset: usize) -> Span {
        Span {
            start: self.start + offset,
            end: self.end + offset,
            text_start: self.text_start + offset,
            text_end: self.text_end + offset,
            ..self
        }
    }
}

fn is_space(c: Option<char>) -> bool {
    c.is_none_or(char::is_whitespace)
}

/// The length of a web address starting at `s`, if one does: to the next
/// space, without the punctuation that ends a sentence around it, or the
/// emphasis mark `around` it.
fn url_len(s: &str, around: Option<char>) -> Option<usize> {
    let lower = s.get(..8).unwrap_or(s).to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("www.")) {
        return None;
    }
    let mut end = s.find(char::is_whitespace).unwrap_or(s.len());
    while end > 0 {
        let last = s[..end].chars().next_back()?;
        // A closing parenthesis ends it unless the address opened one.
        let unbalanced = last == ')' && s[..end].matches('(').count() < s[..end].matches(')').count();
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'' | '>' | ']') || unbalanced || Some(last) == around {
            end -= last.len_utf8();
        } else {
            break;
        }
    }
    let min = if lower.starts_with("www.") { 5 } else { 9 };
    (end >= min).then_some(end)
}

/// Spans whose insides are not Markdown: code, links, tags, addresses.
fn atoms(line: &str) -> Vec<Span> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let escaped = i > 0 && b[i - 1] == b'\\';
        let before = line[..i].chars().next_back();
        if matches!(b[i], b'h' | b'H' | b'w' | b'W')
            && before.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '<' | '"' | '*' | '_' | '~' | '='))
            && let Some(n) = url_len(&line[i..], before.filter(|c| matches!(c, '*' | '_' | '~' | '=')))
        {
            out.push(Span { style: Style::Url, start: i, end: i + n, text_start: i, text_end: i + n });
            i += n;
            continue;
        }
        match b[i] {
            b'`' if !escaped => {
                let n = b[i..].iter().take_while(|&&c| c == b'`').count();
                let mut j = i + n;
                let mut found = None;
                while j < b.len() {
                    if b[j] == b'`' {
                        let m = b[j..].iter().take_while(|&&c| c == b'`').count();
                        if m == n {
                            found = Some(j);
                            break;
                        }
                        j += m;
                    } else {
                        j += 1;
                    }
                }
                match found {
                    Some(j) if j > i + n => {
                        out.push(Span { style: Style::Code, start: i, end: j + n, text_start: i + n, text_end: j });
                        i = j + n;
                    }
                    _ => i += n,
                }
            }
            b'[' | b'!' if !escaped && (line[i..].starts_with("[[") || line[i..].starts_with("![[")) => {
                let open = if b[i] == b'!' { i + 3 } else { i + 2 };
                match line[open..].find("]]") {
                    Some(k) if k > 0 && !line[open..open + k].contains(['[', ']']) => {
                        let close = open + k;
                        // [[target|shown]]: only the alias shows.
                        let text_start = line[open..close].find('|').map_or(open, |p| open + p + 1);
                        out.push(Span { style: Style::WikiLink, start: i, end: close + 2, text_start, text_end: close });
                        i = close + 2;
                    }
                    _ => i += 2,
                }
            }
            b'[' if !escaped => {
                let link = line[i + 1..].find(']').and_then(|k| {
                    let rb = i + 1 + k;
                    let text = &line[i + 1..rb];
                    if text.is_empty() || text.contains('[') || !line[rb + 1..].starts_with('(') {
                        return None;
                    }
                    let rp = line[rb + 2..].find(')')? + rb + 2;
                    Some(Span { style: Style::Link, start: i, end: rp + 1, text_start: i + 1, text_end: rb })
                });
                match link {
                    Some(span) => {
                        i = span.end;
                        out.push(span);
                    }
                    None => i += 1,
                }
            }
            b'#' if !escaped && is_space(line[..i].chars().next_back()) => {
                let body = line[i + 1..]
                    .char_indices()
                    .find(|&(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '/')))
                    .map_or(line.len() - i - 1, |(k, _)| k);
                let tag = &line[i + 1..i + 1 + body];
                if !tag.is_empty() && !tag.chars().all(|c| c.is_ascii_digit()) {
                    out.push(Span { style: Style::Tag, start: i, end: i + 1 + body, text_start: i, text_end: i + 1 + body });
                    i += 1 + body;
                } else {
                    i += 1;
                }
            }
            _ => i += line[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    out
}

/// The inline spans of a line of Markdown text (after its block mark), in
/// order of their start. Spans nest but never cross.
pub fn spans(line: &str) -> Vec<Span> {
    let atoms = atoms(line);
    // Bytes that delimiters cannot use: inside atoms, or already a mark.
    let mut taken = vec![false; line.len()];
    for a in &atoms {
        taken[a.start..a.end].iter_mut().for_each(|t| *t = true);
    }
    let mut found: Vec<Span> = Vec::new();
    let delimiters: [(&str, Style); 7] = [
        ("***", Style::Strong),
        ("**", Style::Strong),
        ("__", Style::Strong),
        ("~~", Style::Strike),
        ("==", Style::Highlight),
        ("*", Style::Emphasis),
        ("_", Style::Emphasis),
    ];
    for (mark, style) in delimiters {
        let n = mark.len();
        let word_bound = mark.starts_with('_');
        let mut i = 0;
        while let Some(k) = line[i..].find(mark) {
            let open = i + k;
            i = open + 1;
            let free = |at: usize| !taken[at..at + n].iter().any(|&t| t);
            let before = line[..open].chars().next_back();
            let after = line[open + n..].chars().next();
            // A run longer than the mark belongs to a longer mark.
            let c = mark.as_bytes()[0] as char;
            if !free(open) || is_space(after) || before == Some('\\') || before == Some(c) || after == Some(c) {
                continue;
            }
            if word_bound && before.is_some_and(char::is_alphanumeric) {
                continue;
            }
            let mut j = open + n;
            let close = loop {
                let Some(k) = line[j..].find(mark) else { break None };
                let at = j + k;
                j = at + 1;
                let before = line[..at].chars().next_back();
                let after = line[at + n..].chars().next();
                if !free(at) || is_space(before) || before == Some('\\') || after == Some(c) || before == Some(c) {
                    continue;
                }
                if word_bound && after.is_some_and(char::is_alphanumeric) {
                    continue;
                }
                break Some(at);
            };
            let Some(close) = close else { continue };
            let span = Span { style, start: open, end: close + n, text_start: open + n, text_end: close };
            // Nest inside or around what was found, never across it.
            let crosses = found.iter().chain(&atoms).any(|s| {
                let disjoint = span.end <= s.start || s.end <= span.start;
                let inside = s.text_start <= span.start && span.end <= s.text_end;
                let around = span.text_start <= s.start && s.end <= span.text_end;
                !(disjoint || inside || around)
            });
            if crosses {
                continue;
            }
            for (a, b) in span.marks() {
                taken[a..b].iter_mut().for_each(|t| *t = true);
            }
            if n == 3 {
                // ***both***: strong around an emphasis with no marks of its own.
                found.push(Span { style: Style::Emphasis, start: open + 3, end: close, text_start: open + 3, text_end: close });
            }
            found.push(span);
            i = close + n;
        }
    }
    found.extend(atoms);
    found.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));
    found
}

/// A line as it reads, without its block mark or inline marks: for titles
/// and search.
pub fn plain(line: &str) -> String {
    let (block, mark, _) = block(line);
    let content = &line[mark..];
    let mut hidden = vec![false; content.len()];
    if block.has_spans() {
        for s in spans(content) {
            for (a, b) in s.marks() {
                hidden[a..b].iter_mut().for_each(|h| *h = true);
            }
        }
    }
    content.char_indices().filter(|&(i, _)| !hidden[i]).map(|(_, c)| c).collect()
}

/// The spans of a line, in offsets into the whole text.
pub fn line_spans(text: &str, line: &Line) -> Vec<Span> {
    if !line.block.has_spans() {
        return Vec::new();
    }
    let from = line.content();
    spans(&text[from..line.end]).into_iter().map(|s| s.at(from)).collect()
}
