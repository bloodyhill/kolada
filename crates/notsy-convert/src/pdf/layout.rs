//! A PDF's glyphs into a note: words, lines, paragraphs (joined across the
//! lines they were wrapped into), headings by size, lists, tables, in
//! reading order (columns one after the other), without the running
//! headers, footers and page numbers every page repeats.
//!
//! Lines and words as pdfminer finds them (glyphs on one baseline, a space
//! where the gap is wider than a tenth of the text's size), then more than
//! pdfminer does: blocks ordered by recursive XY-cuts, sizes and boldness
//! read as structure, right-to-left text put back in reading order.

use std::collections::HashMap;

use super::content::Glyph;
use crate::md::{Block, Span};

/// A page's glyphs, small painted shapes (bullets drawn rather than
/// typed), links and box (points; y up from `bottom`).
pub struct Page {
    pub glyphs: Vec<Glyph>,
    pub dots: Vec<[f32; 4]>,
    pub links: Vec<([f32; 4], String)>,
    pub bottom: f32,
    pub top: f32,
}

/// A run of text that looks one way.
#[derive(Clone, Debug, PartialEq)]
struct Run {
    text: String,
    bold: bool,
    italic: bool,
    mono: bool,
    link: Option<String>,
}

/// A piece of a line: glyphs close enough to be read together.
#[derive(Clone, Debug)]
struct Seg {
    x0: f32,
    x1: f32,
    /// Baseline.
    y: f32,
    size: f32,
    runs: Vec<Run>,
}

impl Seg {
    fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    fn bold(&self) -> bool {
        self.runs.iter().filter(|r| !r.text.trim().is_empty()).all(|r| r.bold)
    }

    fn mono(&self) -> bool {
        self.runs.iter().filter(|r| !r.text.trim().is_empty()).all(|r| r.mono)
    }
}

/// What a block of a page is.
#[derive(Debug)]
enum Kind {
    /// Lines read as text.
    Text(Vec<Seg>),
    /// Rows of cells.
    Table(Vec<Vec<Vec<Run>>>),
}

/// A line of a page: its baseline and size, its glyphs left to right, and
/// its pieces.
struct Row {
    y: f32,
    size: f32,
    glyphs: Vec<Glyph>,
    segs: Vec<Seg>,
}

#[derive(Debug)]
struct Region {
    x0: f32,
    x1: f32,
    top: f32,
    bottom: f32,
    kind: Kind,
}

const BULLETS: &[char] = &['•', '◦', '▪', '▫', '‣', '⁃', '●', '○', '■', '□', '–', '-', '*', '·', '➢', '►', '✓', '✔', '\u{F0B7}', '\u{F0A7}', '◆', '◇', '❖'];

/// Every page's blocks, as the note's.
pub fn blocks(pages: Vec<Page>) -> Vec<Block> {
    let mut rows: Vec<Vec<Row>> = pages.iter().map(rows_of).collect();
    furniture(&pages, &mut rows);
    let body = body_size(&rows);
    let mut regions: Vec<Vec<Region>> = Vec::new();
    for rows in rows {
        let mut found = tables(&rows);
        let used: Vec<usize> = found.iter().flat_map(|(r, _)| r.clone()).collect();
        let text_rows: Vec<&Row> = rows.iter().enumerate().filter(|(i, _)| !used.contains(i)).map(|(_, r)| r).collect();
        let mut page_regions = paragraphs(&text_rows);
        page_regions.extend(found.drain(..).map(|(_, r)| r));
        regions.push(order(page_regions));
    }
    write(regions, body)
}

/// A page's lines, top to bottom.
fn rows_of(page: &Page) -> Vec<Row> {
    let mut glyphs: Vec<&Glyph> = page.glyphs.iter().filter(|g| !g.text.is_empty() && g.size > 0.5).collect();
    glyphs.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));
    // Glyphs on one baseline (within a third of their size) are a line.
    let mut lines: Vec<Vec<&Glyph>> = Vec::new();
    for g in glyphs {
        match lines.last_mut() {
            Some(line) if (line[0].y - g.y).abs() <= 0.35 * line[0].size.max(g.size) => line.push(g),
            _ => lines.push(vec![g]),
        }
    }
    let mut rows: Vec<Row> = Vec::new();
    for mut line in lines {
        line.sort_by(|a, b| a.x.total_cmp(&b.x));
        // A glyph drawn twice over itself (a fake bold) once.
        line.dedup_by(|b, a| a.text == b.text && (a.x - b.x).abs() < 0.12 * a.size && !a.text.trim().is_empty());
        let size = typical(line.iter().filter(|g| !g.text.trim().is_empty()).map(|g| g.size)).unwrap_or(line[0].size);
        // Pieces apart by more than a space or two are separate (columns,
        // table cells), but a list's bullet stays with its text.
        let mut pieces: Vec<Vec<&Glyph>> = Vec::new();
        for g in line {
            match pieces.last_mut() {
                Some(p) => {
                    let last = p[p.len() - 1];
                    let gap = g.x - (last.x + last.w);
                    let marker = is_marker(&p.iter().map(|g| g.text.as_str()).collect::<String>());
                    if gap > 1.25 * size && !marker {
                        pieces.push(vec![g]);
                    } else {
                        p.push(g);
                    }
                }
                None => pieces.push(vec![g]),
            }
        }
        let y = typical(pieces.iter().flatten().map(|g| g.y)).unwrap_or(0.0);
        let glyphs: Vec<Glyph> = pieces.iter().flatten().map(|g| (*g).clone()).collect();
        let segs: Vec<Seg> = pieces.into_iter().map(|p| seg(p, size, y, &page.links)).filter(|s| !s.text().trim().is_empty()).collect();
        if !segs.is_empty() {
            rows.push(Row { y, size, glyphs, segs });
        }
    }
    // A small shape just left of a line's text is its bullet.
    for d in &page.dots {
        let mid = (d[1] + d[3]) / 2.0;
        let found = rows.iter_mut().filter_map(|r| {
            let s = r.segs.iter_mut().find(|s| s.x0 >= d[2])?;
            let near = s.x0 - d[2] <= 2.5 * r.size && mid >= r.y - 0.1 * r.size && mid <= r.y + 0.7 * r.size;
            near.then_some(s)
        });
        if let Some(s) = found.min_by(|a, b| a.x0.total_cmp(&b.x0))
            && marker(&s.text()).is_none()
        {
            s.runs.insert(0, Run { text: "• ".to_string(), bold: false, italic: false, mono: false, link: None });
            s.x0 = d[0];
        }
    }
    rows
}

/// A piece's glyphs as text: spaces where they are wide apart, put in
/// reading order where they run right to left.
fn seg(glyphs: Vec<&Glyph>, size: f32, y: f32, links: &[([f32; 4], String)]) -> Seg {
    #[derive(Clone)]
    struct Unit<'g> {
        text: &'g str,
        glyph: &'g Glyph,
    }
    let mut units: Vec<Unit> = Vec::with_capacity(glyphs.len() * 2);
    for (i, g) in glyphs.iter().enumerate() {
        if i > 0 {
            let prev = glyphs[i - 1];
            let gap = g.x - (prev.x + prev.w);
            if gap > 0.15 * size && !prev.text.ends_with(char::is_whitespace) && !g.text.starts_with(char::is_whitespace) {
                units.push(Unit { text: " ", glyph: g });
            }
        }
        units.push(Unit { text: &g.text, glyph: g });
    }
    // Right to left: the line is in reading order backwards.
    let class = |t: &str| t.chars().find_map(|c| if rtl(c) { Some('R') } else if c.is_alphanumeric() { Some('L') } else { None }).unwrap_or('N');
    let classes: Vec<char> = units.iter().map(|u| class(u.text)).collect();
    let (r, l) = (classes.iter().filter(|&&c| c == 'R').count(), classes.iter().filter(|&&c| c == 'L').count());
    let mut order: Vec<usize> = (0..units.len()).collect();
    if r > 0 {
        let base_rtl = r >= l;
        if base_rtl {
            order.reverse();
        }
        // Runs of the other direction (from a strong letter to a strong
        // letter) read the other way within the line.
        let other = if base_rtl { 'L' } else { 'R' };
        let mut i = 0;
        while i < order.len() {
            if classes[order[i]] == other {
                let mut j = i;
                let mut end = i;
                while j < order.len() && classes[order[j]] != if base_rtl { 'R' } else { 'L' } {
                    if classes[order[j]] == other {
                        end = j;
                    }
                    j += 1;
                }
                order[i..=end].reverse();
                i = end + 1;
            } else {
                i += 1;
            }
        }
    }
    let link_of = |g: &Glyph| {
        let (cx, cy) = (g.x + g.w / 2.0, g.y + g.size / 3.0);
        links.iter().find(|(r, _)| cx >= r[0] && cx <= r[2] && cy >= r[1] && cy <= r[3]).map(|(_, u)| u.clone())
    };
    let mut runs: Vec<Run> = Vec::new();
    for &i in &order {
        let u = &units[i];
        let g = u.glyph;
        let link = link_of(g);
        match runs.last_mut() {
            Some(r) if (r.bold, r.italic, r.mono, &r.link) == (g.bold, g.italic, g.mono, &link) || u.text.trim().is_empty() => r.text.push_str(u.text),
            _ => runs.push(Run { text: u.text.to_string(), bold: g.bold, italic: g.italic, mono: g.mono, link }),
        }
    }
    let x0 = glyphs.iter().map(|g| g.x).fold(f32::MAX, f32::min);
    let x1 = glyphs.iter().map(|g| g.x + g.w).fold(f32::MIN, f32::max);
    Seg { x0, x1, y, size, runs }
}

fn rtl(c: char) -> bool {
    matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}

/// The most common value, to half a point.
fn typical(values: impl Iterator<Item = f32>) -> Option<f32> {
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for v in values {
        *counts.entry((v * 2.0).round() as i32).or_default() += 1;
    }
    counts.into_iter().max_by_key(|&(k, n)| (n, k)).map(|(k, _)| k as f32 / 2.0)
}

/// The body text's size: the size most of the text is in.
fn body_size(rows: &[Vec<Row>]) -> f32 {
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for seg in rows.iter().flatten().flat_map(|r| &r.segs) {
        *counts.entry((seg.size * 2.0).round() as i32).or_default() += seg.text().chars().count();
    }
    counts.into_iter().max_by_key(|&(k, n)| (n, -k)).map_or(10.0, |(k, _)| k as f32 / 2.0)
}

/// Whether a line's text starts with (or is) a list's bullet or number.
fn is_marker(t: &str) -> bool {
    marker(t.trim()).is_some_and(|(_, rest)| rest.trim().is_empty())
}

/// A list marker at the start of `t`: whether numbered, and the rest.
fn marker(t: &str) -> Option<(bool, &str)> {
    let t = t.trim_start();
    let first = t.chars().next()?;
    if BULLETS.contains(&first) {
        let rest = &t[first.len_utf8()..];
        // "-5" or "--" is not a bullet.
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return Some((false, rest.trim_start()));
        }
        return None;
    }
    let end = t.find(['.', ')'])?;
    let (n, rest) = (&t[..end], &t[end + 1..]);
    let numbered = (1..=3).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit())
        || n.len() == 1 && n.bytes().all(|b| b.is_ascii_alphabetic())
        || (1..=5).contains(&n.len()) && n.bytes().all(|b| b"ivxlcIVXLC".contains(&b));
    (numbered && (rest.is_empty() || rest.starts_with(char::is_whitespace))).then(|| (true, rest.trim_start()))
}

/// Takes away what every page repeats at its top and bottom (a running
/// title, a footer) and page numbers.
fn furniture(pages: &[Page], rows: &mut [Vec<Row>]) {
    let band = |p: &Page, y: f32| {
        let h = (p.top - p.bottom).max(1.0);
        y > p.top - 0.09 * h || y < p.bottom + 0.09 * h
    };
    let norm = |t: &str| t.chars().map(|c| if c.is_ascii_digit() { '#' } else { c.to_ascii_lowercase() }).filter(|c| !c.is_whitespace()).collect::<String>();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (p, rs) in pages.iter().zip(rows.iter()) {
        let mut on_page: Vec<String> = rs.iter().filter(|r| band(p, r.y)).map(|r| norm(&r.segs.iter().map(Seg::text).collect::<String>())).collect();
        on_page.sort();
        on_page.dedup();
        for t in on_page {
            *seen.entry(t).or_default() += 1;
        }
    }
    let repeated = |t: &str| pages.len() >= 2 && seen.get(t).is_some_and(|&n| n >= 2 && n * 2 >= pages.len());
    for (p, rs) in pages.iter().zip(rows.iter_mut()) {
        rs.retain(|r| {
            if !band(p, r.y) {
                return true;
            }
            let text: String = r.segs.iter().map(Seg::text).collect::<Vec<_>>().join(" ");
            !(repeated(&norm(&text)) || page_number(&text))
        });
    }
}

/// "12", "- 12 -", "Page 3", "Page 3 of 10", "3 / 10".
fn page_number(t: &str) -> bool {
    let t = t.trim().trim_matches(|c: char| c == '-' || c == '–' || c.is_whitespace()).to_ascii_lowercase();
    let t = t.strip_prefix("page").map_or(t.as_str(), str::trim);
    let mut parts = t.split(|c: char| c == '/' || c.is_whitespace()).filter(|p| !p.is_empty() && *p != "of");
    let first = parts.next();
    first.is_some_and(|p| p.bytes().all(|b| b.is_ascii_digit())) && parts.all(|p| p.bytes().all(|b| b.is_ascii_digit()))
}

/// Where a line's words start, after its first (and the gap before each).
fn word_starts(r: &Row) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    let mut prev: Option<&Glyph> = None;
    for g in &r.glyphs {
        if g.text.trim().is_empty() {
            continue;
        }
        if let Some(p) = prev {
            let gap = g.x - (p.x + p.w);
            if gap > 0.15 * r.size {
                out.push((g.x, gap));
            }
        }
        prev = Some(g);
    }
    out
}

/// Rows that are a table: consecutive lines whose words start at the same
/// places (its columns' edges), with real gaps before those places. Gaps
/// alone do not find a table: a tight one has cells a space apart.
fn tables(rows: &[Row]) -> Vec<(Vec<usize>, Region)> {
    let starts: Vec<Vec<(f32, f32)>> = rows.iter().map(word_starts).collect();
    let shares = |a: &[(f32, f32)], b: &[(f32, f32)]| a.iter().any(|x| b.iter().any(|y| (x.0 - y.0).abs() <= 1.5));
    let near = |a: &Row, b: &Row, ems: f32| a.y - b.y <= ems * b.size && (b.size - a.size).abs() < 1.0;
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let mut j = i + 1;
        while j < rows.len() && near(&rows[j - 1], &rows[j], 3.5) && shares(&starts[j - 1], &starts[j]) {
            j += 1;
        }
        if j - i >= 2
            && let Some(region) = table(&rows[i..j], &aligned(&rows[i..j], &starts[i..j]), false)
        {
            out.push(((i..j).collect(), region));
            i = j;
            continue;
        }
        // Numbers set right under headings set left start nowhere near
        // them, but leave the same gaps between the columns.
        if headings(&rows[i]) {
            let mut cover = Cover::default();
            cover.add(&rows[i]);
            let mut j = i + 1;
            while j < rows.len() && j - i < 60 && near(&rows[j - 1], &rows[j], 3.5) && {
                cover.add(&rows[j]);
                cover.gutters() >= 2
            } {
                j += 1;
            }
            // Without the rows at the end that fill a cell or two, if all
            // of them are not a table: totals under an invoice's items.
            let mut items = j;
            while items > i + 2 && pieces(&rows[items - 1]).len() <= 2 {
                items -= 1;
            }
            let tries = [j, items];
            if let Some((j, region)) = tries.iter().take(if items < j { 2 } else { 1 }).find_map(|&j| Some((j, table(&rows[i..j], &gutters(&rows[i + 1..j]), true)?))) {
                out.push(((i..j).collect(), region));
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Column edges: places at least half the rows (and two) start a word,
/// with a gap of an em before it in one of them and half an em in two.
fn aligned(rows: &[Row], starts: &[Vec<(f32, f32)>]) -> Vec<f32> {
    let size = rows[0].size;
    let mut all: Vec<(f32, f32, usize)> = starts.iter().enumerate().flat_map(|(r, s)| s.iter().map(move |&(x, g)| (x, g, r))).collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut edges: Vec<f32> = Vec::new();
    let mut k = 0;
    while k < all.len() {
        let mut m = k;
        while m + 1 < all.len() && all[m + 1].0 - all[k].0 <= 1.5 {
            m += 1;
        }
        let cluster = &all[k..=m];
        let mut in_rows: Vec<usize> = cluster.iter().map(|c| c.2).collect();
        in_rows.dedup();
        let wide = cluster.iter().filter(|c| c.1 >= 0.5 * size).count();
        let widest = cluster.iter().map(|c| c.1).fold(0.0, f32::max);
        if in_rows.len() >= 2 && in_rows.len() * 2 >= rows.len() && wide >= 2 && widest >= size {
            edges.push(cluster[0].0);
        }
        k = m + 1;
    }
    edges
}

/// A row's pieces: its words an em or less apart, left to right.
fn pieces(row: &Row) -> Vec<Vec<&Glyph>> {
    let mut glyphs: Vec<&Glyph> = row.glyphs.iter().filter(|g| !g.text.trim().is_empty()).collect();
    glyphs.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut out: Vec<(Vec<&Glyph>, f32)> = Vec::new();
    for g in glyphs {
        match out.last_mut() {
            Some((piece, reach)) if g.x - *reach < row.size => {
                piece.push(g);
                *reach = reach.max(g.x + g.w);
            }
            _ => out.push((vec![g], g.x + g.w)),
        }
    }
    out.into_iter().map(|(piece, _)| piece).collect()
}

/// A row that could head a table found by its gaps: three pieces or more,
/// mostly words, none a number.
fn headings(row: &Row) -> bool {
    let pieces = pieces(row);
    let texts: Vec<String> = pieces.iter().map(|p| p.iter().map(|g| g.text.as_str()).collect()).collect();
    let worded = texts.iter().filter(|t| t.chars().any(char::is_alphabetic)).count();
    pieces.len() >= 3 && worded * 2 >= pieces.len() && !texts.iter().any(|t| numeric_text(t))
}

/// What rows' glyphs cover across the page, as rows are added: the gaps of
/// an em or more left between are a table's gutters.
#[derive(Default)]
struct Cover {
    spans: Vec<(f32, f32)>,
    size: f32,
}

impl Cover {
    fn add(&mut self, row: &Row) {
        self.size = self.size.max(row.size);
        let mut all: Vec<(f32, f32)> = row.glyphs.iter().filter(|g| !g.text.trim().is_empty()).map(|g| (g.x, g.x + g.w)).collect();
        all.append(&mut self.spans);
        all.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (a, b) in all {
            match self.spans.last_mut() {
                // Gaps narrower than a gutter are never one: merged away.
                Some(last) if a - last.1 < self.size => last.1 = last.1.max(b),
                _ => self.spans.push((a, b)),
            }
        }
    }

    fn gutters(&self) -> usize {
        self.spans.len().saturating_sub(1)
    }
}

/// Where columns start after gaps of an em or more that stay empty down
/// every row.
fn gutters(rows: &[Row]) -> Vec<f32> {
    let size = rows.iter().map(|r| r.size).fold(0.0, f32::max);
    let mut spans: Vec<(f32, f32)> = rows.iter().flat_map(|r| r.glyphs.iter().filter(|g| !g.text.trim().is_empty()).map(|g| (g.x, g.x + g.w))).collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let Some(&(_, mut reach)) = spans.first() else { return out };
    for &(a, b) in &spans[1..] {
        if a - reach >= size {
            out.push(a);
        }
        reach = reach.max(b);
    }
    out
}

/// A heading row over columns found by the gaps below it: headings set
/// left over numbers set right stand in the gap before their column, so
/// each heading (its words an em or less apart) goes over the column it
/// overlaps most, or else the next one to its right.
fn over_columns<'g>(header: &'g Row, body: &[Vec<Vec<&Glyph>>]) -> Vec<Vec<&'g Glyph>> {
    let count = body[0].len();
    let extents: Vec<Option<(f32, f32)>> = (0..count)
        .map(|c| {
            let gs = body.iter().flat_map(|row| &row[c]).filter(|g| !g.text.trim().is_empty());
            gs.fold(None, |e: Option<(f32, f32)>, g| Some(e.map_or((g.x, g.x + g.w), |(a, b)| (a.min(g.x), b.max(g.x + g.w)))))
        })
        .collect();
    let headings = pieces(header);
    let mut out: Vec<Vec<&Glyph>> = vec![Vec::new(); count];
    for h in headings {
        let (x0, x1) = (h[0].x, h.iter().map(|g| g.x + g.w).fold(f32::MIN, f32::max));
        let overlap = |c: usize| extents[c].map_or(0.0, |(a, b)| (x1.min(b) - x0.max(a)).max(0.0));
        let c = (0..count)
            .filter(|&c| overlap(c) > 0.0)
            .max_by(|&a, &b| overlap(a).total_cmp(&overlap(b)))
            .or_else(|| (0..count).find(|&c| extents[c].is_some_and(|(a, _)| a >= x1)))
            .unwrap_or(count - 1);
        out[c].extend(h);
    }
    // Spaces back where they were, for the words to be found.
    for column in &mut out {
        column.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
    out
}

/// Text that is a number: an amount, a quantity, a percentage.
fn numeric_text(text: &str) -> bool {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let digits = chars.iter().filter(|c| c.is_ascii_digit()).count();
    let numberish = chars.iter().filter(|c| c.is_ascii_digit() || ".,%-+()/$\u{20AC}\u{00A3}".contains(**c)).count();
    digits > 0 && numberish * 10 >= chars.len() * 7
}

/// The rows as a table with columns starting at `edges`, if they read as
/// one. By gaps (`by_gaps`), it has a column of numbers below its heading.
fn table(rows: &[Row], edges: &[f32], by_gaps: bool) -> Option<Region> {
    if edges.is_empty() {
        return None;
    }
    let mut grid: Vec<Vec<Vec<&Glyph>>> = rows
        .iter()
        .map(|r| {
            let mut columns: Vec<Vec<&Glyph>> = vec![Vec::new(); edges.len() + 1];
            for g in &r.glyphs {
                columns[edges.iter().filter(|&&e| g.x >= e - 1.5).count()].push(g);
            }
            columns
        })
        .collect();
    if by_gaps {
        grid[0] = over_columns(&rows[0], &grid[1..]);
    }
    // First by the glyphs' own text: cheaper than laying out every cell,
    // and never longer than the cells' (no spaces), so what it turns down
    // the cells would too.
    let raw: Vec<Vec<String>> = grid.iter().map(|row| row.iter().map(|gs| gs.iter().flat_map(|g| g.text.chars()).filter(|c| !c.is_whitespace()).collect()).collect()).collect();
    if !reads_as_table(&raw, by_gaps) {
        return None;
    }
    let cells: Vec<Vec<Vec<Run>>> = grid
        .into_iter()
        .zip(rows)
        .map(|(columns, r)| {
            columns
                .into_iter()
                .map(|gs| {
                    let gs: Vec<&Glyph> = gs.into_iter().skip_while(|g| g.text.trim().is_empty()).collect();
                    if gs.is_empty() { Vec::new() } else { seg(gs, r.size, r.y, &[]).runs }
                })
                .collect()
        })
        .collect();
    let texts: Vec<Vec<String>> = cells.iter().map(|row| row.iter().map(|c| c.iter().map(|r| r.text.trim()).collect()).collect()).collect();
    if !reads_as_table(&texts, by_gaps) {
        return None;
    }
    let all_glyphs = rows.iter().flat_map(|r| &r.glyphs);
    Some(Region {
        x0: all_glyphs.clone().map(|g| g.x).fold(f32::MAX, f32::min),
        x1: all_glyphs.map(|g| g.x + g.w).fold(f32::MIN, f32::max),
        top: rows[0].y + rows[0].size,
        bottom: rows[rows.len() - 1].y - 0.3 * rows[rows.len() - 1].size,
        kind: Kind::Table(cells),
    })
}

/// Cells' text (rows of columns) that read as a table.
fn reads_as_table(texts: &[Vec<String>], by_gaps: bool) -> bool {
    let columns = texts[0].len();
    // Every row is in two cells or more; a table of two columns has short
    // cells (two columns of prose line up too).
    if texts.iter().any(|row| row.iter().filter(|t| !t.is_empty()).count() < 2) {
        return false;
    }
    let mut lengths: Vec<usize> = texts.iter().flatten().map(|t| t.chars().count()).filter(|&n| n > 0).collect();
    lengths.sort_unstable();
    let median = lengths[lengths.len() / 2];
    if columns < 3 && (texts.len() < 3 || median > 25) {
        return false;
    }
    // Found by its gaps alone, it is a table of short cells, mostly
    // filled, under a row of headings, with a column of numbers: not a
    // chart's scale or a reference's parts.
    if by_gaps {
        let headings = texts[0].iter().filter(|t| !t.is_empty());
        let worded = headings.clone().filter(|t| t.chars().any(char::is_alphabetic)).count();
        let numbered = headings.clone().any(|t| numeric_text(t));
        let empty = texts.iter().flatten().filter(|t| t.is_empty()).count();
        let numbers = (0..columns).any(|c| texts[1..].iter().filter(|row| numeric_text(&row[c])).count() * 2 >= texts.len() - 1)
            // One row under headings: an item's quantity and amount, not a
            // chart's labels.
            && (texts.len() > 2 || texts[1].iter().filter(|t| numeric_text(t)).count() >= 2);
        if median > 30 || lengths[lengths.len() - 1] > 50 || numbered || worded * 2 < headings.count() || empty * 5 > texts.len() * columns * 2 || !numbers {
            return false;
        }
    }
    true
}

/// Lines into blocks: a line joins the block above it when it is the next
/// line down (by the block's spacing), overlaps it across, and is the same
/// size; a bold line over plain text starts its own.
fn paragraphs(rows: &[&Row]) -> Vec<Region> {
    let mut blocks: Vec<(Vec<Seg>, Option<f32>)> = Vec::new();
    for row in rows {
        for s in &row.segs {
            let mut joined = false;
            for (lines, spacing) in blocks.iter_mut().rev() {
                let last = lines.last().expect("a block has a line");
                let gap = last.y - s.y;
                let overlaps = s.x0 < last.x1.max(lines[0].x1) + 0.5 * s.size && s.x1 > lines.iter().map(|l| l.x0).fold(f32::MAX, f32::min) - 0.5 * s.size;
                let same_size = (s.size / last.size).max(last.size / s.size) <= 1.2;
                let near = match spacing {
                    Some(sp) => gap > 0.0 && gap <= *sp * 1.35 + 0.5,
                    None => gap > 0.0 && gap <= 1.75 * s.size.max(last.size),
                };
                let heading_over_text = last.bold() && !s.bold() && lines.len() == 1;
                let marker_starts = marker(&s.text()).is_some() && marker(&last.text()).is_none() && lines.len() > 1;
                if near && overlaps && same_size && !heading_over_text && !marker_starts {
                    if spacing.is_none() {
                        *spacing = Some(gap);
                    }
                    lines.push(s.clone());
                    joined = true;
                    break;
                }
                // Only the blocks just above are candidates.
                if last.y - s.y > 3.0 * s.size {
                    break;
                }
            }
            if !joined {
                blocks.push((vec![s.clone()], None));
            }
        }
    }
    blocks
        .into_iter()
        .map(|(lines, _)| Region {
            x0: lines.iter().map(|l| l.x0).fold(f32::MAX, f32::min),
            x1: lines.iter().map(|l| l.x1).fold(f32::MIN, f32::max),
            top: lines[0].y + lines[0].size,
            bottom: lines[lines.len() - 1].y - 0.3 * lines[lines.len() - 1].size,
            kind: Kind::Text(lines),
        })
        .collect()
}

/// Reading order by recursive XY-cuts: a clear vertical gutter splits the
/// regions into columns (left one first) when it runs their whole height;
/// otherwise a clear horizontal gap splits them top to bottom.
fn order(mut regions: Vec<Region>) -> Vec<Region> {
    if regions.len() <= 1 {
        return regions;
    }
    // Columns first, if a gutter runs from top to bottom.
    if let Some(split) = gap(&regions, |r| (r.x0, r.x1), 6.0) {
        let (left, right): (Vec<Region>, Vec<Region>) = regions.into_iter().partition(|r| r.x1 <= split);
        if !left.is_empty() && !right.is_empty() {
            let mut out = order(left);
            out.extend(order(right));
            return out;
        }
        regions = left.into_iter().chain(right).collect();
    }
    // Then bands, top to bottom (y runs up, so the highest first).
    if let Some(split) = gap(&regions, |r| (-r.top, -r.bottom), 0.5) {
        let (above, below): (Vec<Region>, Vec<Region>) = regions.into_iter().partition(|r| -r.bottom <= split);
        if !above.is_empty() && !below.is_empty() {
            let mut out = order(above);
            out.extend(order(below));
            return out;
        }
        regions = above.into_iter().chain(below).collect();
    }
    regions.sort_by(|a, b| b.top.total_cmp(&a.top).then(a.x0.total_cmp(&b.x0)));
    regions
}

/// Where the regions' extents (from `span`) leave a clear gap at least
/// `least` wide: the middle of the widest one.
fn gap(regions: &[Region], span: impl Fn(&Region) -> (f32, f32), least: f32) -> Option<f32> {
    let mut spans: Vec<(f32, f32)> = regions.iter().map(span).collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut best: Option<(f32, f32)> = None;
    let mut reach = spans[0].1;
    for &(a, b) in &spans[1..] {
        if a - reach >= least && best.is_none_or(|(w, _)| a - reach > w) {
            best = Some((a - reach, (a + reach) / 2.0));
        }
        reach = reach.max(b);
    }
    best.map(|(_, mid)| mid)
}

/// The note's blocks, from each page's regions in reading order.
fn write(pages: Vec<Vec<Region>>, body: f32) -> Vec<Block> {
    // Heading sizes: larger than the body, largest first.
    let mut sizes: Vec<i32> = Vec::new();
    for r in pages.iter().flatten() {
        if let Kind::Text(lines) = &r.kind
            && heading_like(lines, body)
        {
            sizes.push((lines[0].size * 2.0).round() as i32);
        }
    }
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    sizes.dedup();
    let level = |size: f32| sizes.iter().position(|&s| s == (size * 2.0).round() as i32).map(|i| (i + 1).min(6) as u8);
    let bold_level = (sizes.len() + 1).min(6) as u8;
    // List items' left edges: how deep each is.
    let mut out: Vec<Block> = Vec::new();
    for regions in pages {
        let mut first_on_page = true;
        // Where the page's list markers stand, left to right: an item's
        // depth is which of them it is at.
        let mut indents: Vec<f32> = regions
            .iter()
            .flat_map(|r| match &r.kind {
                Kind::Text(lines) => lines.iter().filter(|l| marker(&l.text()).is_some()).map(|l| l.x0).collect(),
                _ => Vec::new(),
            })
            .collect();
        indents.sort_by(f32::total_cmp);
        indents.dedup_by(|b, a| *b - *a <= 3.0);
        // The page's left margin: where most of its lines start.
        let margin = typical(regions.iter().flat_map(|r| match &r.kind {
            Kind::Text(lines) => lines.iter().map(|l| l.x0).collect(),
            _ => Vec::new(),
        }));
        for r in regions {
            match r.kind {
                Kind::Table(rows) => {
                    let mut rows: Vec<Vec<Vec<Span>>> = rows.into_iter().map(|row| row.into_iter().map(|cell| spans(&cell)).collect()).collect();
                    // The header row is bold already, as a table shows it.
                    for cell in rows.iter_mut().take(1).flatten() {
                        cell.iter_mut().for_each(|s| s.bold = false);
                    }
                    out.push(Block::Table(rows));
                }
                Kind::Text(lines) => {
                    let text_size = lines[0].size;
                    if heading_like(&lines, body) {
                        let text = join(&lines).into_iter().map(|r| r.text).collect::<String>();
                        out.push(Block::Heading(level(text_size).unwrap_or(1), vec![Span::plain(text.trim())]));
                    } else if lines.len() == 1 && lines[0].bold() && short_label(&lines[0].text()) && marker(&lines[0].text()).is_none() {
                        out.push(Block::Heading(bold_level, vec![Span::plain(lines[0].text().trim())]));
                    } else if lines.len() >= 2 && lines.iter().all(Seg::mono) {
                        out.push(Block::Code(lines.iter().map(Seg::text).collect::<Vec<_>>().join("\n")));
                    } else if margin.is_some_and(|m| lines.iter().all(|l| (m + 2.5 * l.size..=m + 8.0 * l.size).contains(&l.x0) && marker(&l.text()).is_none()))
                        && join(&lines).iter().map(|r| r.text.as_str()).collect::<String>().split_whitespace().count() >= 2
                    {
                        // Set in from the margin (a little: further in is a
                        // form's field), a sentence, not a list: a quote.
                        out.push(Block::Quote(spans(&join(&lines))));
                    } else {
                        text_blocks(&lines, &indents, first_on_page, &mut out);
                    }
                }
            }
            first_on_page = false;
        }
    }
    out
}

/// A heading: larger than the body text, a few short lines.
fn heading_like(lines: &[Seg], body: f32) -> bool {
    let size = lines[0].size;
    let text: usize = lines.iter().map(|l| l.text().chars().count()).sum();
    size >= body * 1.15 && lines.len() <= 3 && text <= 200 && lines.iter().all(|l| (l.size - size).abs() < 0.6)
}

/// A bold line that reads as a heading: short, not a sentence.
fn short_label(t: &str) -> bool {
    let t = t.trim();
    !t.is_empty() && t.split_whitespace().count() <= 10 && !t.ends_with(['.', ',', ';', ':']) && t.chars().any(char::is_alphabetic)
}

/// A block's lines as paragraphs and list items.
fn text_blocks(lines: &[Seg], indents: &[f32], first_on_page: bool, out: &mut Vec<Block>) {
    let width = lines.iter().map(|l| l.x1).fold(f32::MIN, f32::max) - lines.iter().map(|l| l.x0).fold(f32::MAX, f32::min);
    let mut current: Vec<Seg> = Vec::new();
    let mut item: Option<(bool, usize)> = None;
    let flush = |current: &mut Vec<Seg>, item: &mut Option<(bool, usize)>, out: &mut Vec<Block>, first: bool| {
        if current.is_empty() {
            return;
        }
        let runs = join(current);
        let mut spans = spans(&runs);
        match item.take() {
            Some((ordered, depth)) => {
                // The marker goes; the item's text stays.
                if let Some(first) = spans.first_mut() {
                    let t = first.text.trim_start().to_string();
                    first.text = marker(&t).map_or(t.clone(), |(_, rest)| rest.to_string());
                }
                out.push(Block::Item { ordered, depth, task: None, spans });
            }
            None => {
                // A paragraph cut by a page: joined to the one before it.
                let text: String = spans.iter().map(|s| s.text.as_str()).collect();
                let continues = first && text.trim_start().starts_with(char::is_lowercase);
                if let (true, Some(Block::Paragraph(prev))) = (continues, out.last_mut()) {
                    let ends = prev.iter().map(|s| s.text.as_str()).collect::<String>();
                    if !ends.trim_end().ends_with(['.', '!', '?', ':', '"', '”']) {
                        prev.push(Span::plain(" "));
                        prev.append(&mut spans);
                        current.clear();
                        return;
                    }
                }
                out.push(Block::Paragraph(spans));
            }
        }
        current.clear();
    };
    // Only the page's first paragraph can carry on the page before's.
    let mut written = false;
    for (k, l) in lines.iter().enumerate() {
        let text = l.text();
        if let Some((ordered, _)) = marker(&text) {
            let first = first_on_page && !written;
            written |= !current.is_empty();
            flush(&mut current, &mut item, out, first);
            let depth = indents.iter().rposition(|&x| x <= l.x0 + 3.0).unwrap_or(0).min(5);
            item = Some((ordered, depth));
            current.push(l.clone());
            continue;
        }
        // A short line ending a sentence ends its paragraph.
        if let Some(prev) = k.checked_sub(1).map(|p| &lines[p])
            && item.is_none()
            && prev.x1 < prev.x0 + width * 0.75
            && prev.text().trim_end().ends_with(['.', '!', '?', ':', '"', '”', ')'])
        {
            let first = first_on_page && !written;
            written |= !current.is_empty();
            flush(&mut current, &mut item, out, first);
        }
        current.push(l.clone());
    }
    flush(&mut current, &mut item, out, first_on_page && !written);
}

/// Lines as one run of text: a space between them, or nothing where a
/// word was hyphenated over the break.
fn join(lines: &[Seg]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (k, l) in lines.iter().enumerate() {
        if k > 0 {
            let prev_text = lines[k - 1].text();
            let next = l.text();
            let hyphenated = prev_text.trim_end().ends_with('-')
                && prev_text.trim_end().chars().rev().nth(1).is_some_and(char::is_alphabetic)
                && next.trim_start().starts_with(char::is_lowercase);
            if hyphenated {
                if let Some(last) = runs.last_mut() {
                    let t = last.text.trim_end().trim_end_matches('-').to_string();
                    last.text = t;
                }
            } else if let Some(last) = runs.last_mut()
                && !last.text.ends_with(' ')
            {
                last.text.push(' ');
            }
        }
        for r in &l.runs {
            match runs.last_mut() {
                Some(last) if (last.bold, last.italic, last.mono, &last.link) == (r.bold, r.italic, r.mono, &r.link) => last.text.push_str(&r.text),
                _ => runs.push(r.clone()),
            }
        }
    }
    runs
}

/// Runs as the note's spans: spaces collapsed, bold and italic kept,
/// monospace as code, links as links.
fn spans(runs: &[Run]) -> Vec<Span> {
    runs.iter()
        .map(|r| {
            let mut text = String::with_capacity(r.text.len());
            let mut space = false;
            for c in r.text.chars() {
                if c.is_whitespace() {
                    space = true;
                } else {
                    if space {
                        text.push(' ');
                        space = false;
                    }
                    text.push(c);
                }
            }
            if space {
                text.push(' ');
            }
            Span { text, bold: r.bold, italic: r.italic, code: r.mono && !r.text.trim().is_empty(), link: r.link.clone(), ..Span::default() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_and_page_numbers() {
        assert_eq!(marker("• item"), Some((false, "item")));
        assert_eq!(marker("12. item"), Some((true, "item")));
        assert_eq!(marker("b) item"), Some((true, "item")));
        assert_eq!(marker("iv. item"), Some((true, "item")));
        assert_eq!(marker("-5 degrees"), None);
        assert_eq!(marker("2024. A year"), None);
        assert_eq!(marker("e.g. this"), None);
        for p in ["12", "- 12 -", "Page 3", "Page 3 of 10", "3 / 10"] {
            assert!(page_number(p), "{p}");
        }
        assert!(!page_number("Chapter 3"));
    }
}
