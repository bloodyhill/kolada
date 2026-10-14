//! The note's text on screen: one text layout per paragraph, styled
//! from its Markdown. Drawing (`scene`) and caret movement (the editor's
//! `Layout`) both go through here, so they always agree on where text wraps.
//!
//! Marks stay in the text (so offsets never need translating) but are
//! shrunk to nothing where they should not show: block marks always (the
//! scene draws bullets, numbers, checkboxes and quote bars in their place),
//! inline marks unless the caret is inside their span, where they show
//! faintly for editing.

use std::rc::Rc;

use kolada_editor::Layout as EditorLayout;
use kolada_markdown::{self as md, Block, Line, Span, Style};
use kolada_text::{Align, Format, Layout, Role, Style as Look};
use windows_core::Result;

use crate::scene::{self, Eng, Fonts, Ink};

/// Indentation per list level; the first level's bullet sits in it.
pub const LIST_INDENT: f32 = 24.0;
pub const QUOTE_INDENT: f32 = 18.0;
/// Code sits inset on its panel.
pub const CODE_INDENT: f32 = 16.0;
/// Hidden marks keep their place in the text but take no room.
const HIDDEN_SIZE: f32 = 0.01;
/// Inline code is set a little smaller than the text around it.
const CODE_SCALE: f32 = 0.88;
/// Room inside a table cell, either side and above and below; the least a
/// column is narrowed to; a table's header weight; and the rows measured
/// for its columns (a longer table's rows below fit what those set).
pub const CELL_PAD: f32 = 12.0;
pub const CELL_VPAD: f32 = 7.0;
const CELL_LEAST: f32 = 48.0;
const HEADER_WEIGHT: u16 = 600;
const MEASURED_ROWS: usize = 200;
/// Kept-layout keys for a table line's hidden text and a cell's natural
/// width (see `scene::kept_layout`).
const HIDDEN_LINE: u64 = 0x7AB1_E0FF_0000_0001;
const NATURAL: u64 = 0x7AB1_E0FF_0000_0002;
/// The same, for a cell's longest word.
const LONGEST_WORD: u64 = 0x7AB1_E0FF_0000_0003;

/// The pictures a note shows (see `images`): what is known of them, the
/// vault and the note's folder their names are found from, and the scale
/// they are decoded at (pixels per DIP).
pub struct Pictures<'a> {
    pub images: &'a std::cell::RefCell<dyn crate::images::PictureSource>,
    pub vault: &'a std::path::Path,
    pub dir: std::path::PathBuf,
    pub scale: f32,
    /// A picture being resized: its line's start, and the width it has
    /// so far (DIPs).
    pub resize: Option<(usize, f32)>,
}

/// A picture in place of a line: its file (None while it is being found)
/// and its size on the page, DIPs.
pub struct Picture {
    pub path: Option<std::path::PathBuf>,
    pub width: f32,
    pub height: f32,
}

/// A picture at most this tall on the page, DIPs; and the space a picture
/// still being found takes.
const PICTURE_MAX: f32 = 540.0;
/// The narrowest a picture is resized to, DIPs.
pub const PICTURE_LEAST: f32 = 48.0;
const PICTURE_WAIT: f32 = 120.0;

/// A picture of `w` x `h` pixels on a column `width` wide: `wanted` wide
/// if it says, else its natural size (a pixel a DIP, as the web does) and
/// no taller than `PICTURE_MAX`; never wider than the column.
pub fn fit_picture(w: u32, h: u32, width: f32, wanted: Option<f32>) -> (f32, f32) {
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    if let Some(want) = wanted {
        let out = want.clamp(PICTURE_LEAST, width.max(PICTURE_LEAST));
        return (out, out * h / w);
    }
    let mut out = (w.min(width), 0.0);
    out.1 = out.0 * h / w;
    if out.1 > PICTURE_MAX {
        out = (PICTURE_MAX * w / h, PICTURE_MAX);
    }
    out
}

/// A paragraph (one line of the note) and its layout.
pub struct Para {
    pub start: usize,
    pub end: usize,
    pub line: Line,
    pub spans: Vec<Span>,
    pub rtl: bool,
    pub layout: Rc<Layout>,
    /// The layout's offset from the column's left edge (indentation), and
    /// the space above it.
    pub x: f32,
    pub pad: f32,
    /// The picture the line shows instead of its text.
    pub picture: Option<Picture>,
    /// A table's row, its cells laid out in their columns; or, with no
    /// cells, its alignment row, which takes no room.
    pub row: Option<Row>,
}

/// A table row: its cells in their columns, whether it is the header (on a
/// soft panel) or the last row (no hairline under it), its height, and the
/// table's width.
pub struct Row {
    pub cells: Vec<Cell>,
    pub header: bool,
    pub last: bool,
    pub height: f32,
    pub width: f32,
}

impl Row {
    /// The cell holding `offset`, or the nearest after it (a pipe or the
    /// space beside one belongs to the next cell), or the last.
    fn cell_near(&self, offset: usize) -> Option<&Cell> {
        self.cells.iter().find(|c| c.start <= offset && offset <= c.end).or_else(|| self.cells.iter().find(|c| c.start > offset)).or(self.cells.last())
    }
}

/// A table cell: its text's bytes in the note, its column's left edge and
/// width (from the paragraph's left), and its text laid out in the column.
pub struct Cell {
    pub start: usize,
    pub end: usize,
    pub x: f32,
    pub width: f32,
    pub layout: Rc<Layout>,
}

/// A table's columns (see `kolada_editor::table::columns`) and how each one's
/// cells align.
pub struct Table {
    pub columns: Vec<(f32, f32)>,
    pub aligns: Vec<md::Align>,
    /// Its header reads right to left: its first column is its rightmost.
    pub rtl: bool,
}

/// Bytes `a..z` of line `l` as they read with no caret among them: the
/// Markdown's marks and links' addresses left out.
fn shown(text: &str, l: &Line, spans: &[Span], a: usize, z: usize) -> String {
    let (a, z) = (l.start + a, l.start + z);
    let hidden: Vec<(usize, usize)> = spans.iter().flat_map(|s| s.marks()).filter(|&(x, y)| x < y && x < z && y > a).collect();
    let mut out = String::with_capacity(z - a);
    let mut at = a;
    for (i, ch) in text[a..z].char_indices() {
        let i = a + i;
        if i < at {
            continue;
        }
        match hidden.iter().find(|&&(x, y)| x <= i && i < y) {
            Some(&(_, y)) => at = y,
            None => out.push(ch),
        }
    }
    out
}

impl Para {
    /// The height it takes: its text's, or its row's.
    pub fn height(&self) -> f32 {
        match &self.row {
            Some(row) => row.height,
            None => self.layout.measure().height,
        }
    }

    /// The paragraph's text color.
    pub fn ink(&self) -> Ink {
        match self.line.block {
            Block::Quote => Ink::Muted,
            Block::Task { done: true } | Block::Frontmatter | Block::Fence | Block::TableDelimiter => Ink::Faint,
            _ => Ink::Text,
        }
    }
}

/// One wrapped line of a paragraph: its bytes within the paragraph, the
/// whitespace that hangs off its end, and where it is.
struct VisualLine {
    start: usize,
    end: usize,
    trailing_space: usize,
    top: f32,
    height: f32,
}

fn visual_lines(layout: &Layout, text: &str) -> Vec<VisualLine> {
    layout
        .lines()
        .into_iter()
        .map(|l| {
            let line = &text[l.start..l.end];
            VisualLine { start: l.start, end: l.end, trailing_space: line.len() - line.trim_end().len(), top: l.top, height: l.height }
        })
        .collect()
}

/// The visual line holding byte `index`: at a wrap, the caret belongs to the
/// line below.
fn line_index(lines: &[VisualLine], index: usize) -> usize {
    lines.iter().rposition(|l| l.start <= index).unwrap_or(0)
}

/// The line that is the note's title, and its name: the first with words,
/// after any frontmatter (or the first line, while there are none).
pub fn title_line(text: &str, lines: &[Line]) -> Option<usize> {
    let body = lines.iter().position(|l| l.block != Block::Frontmatter)?;
    let named = lines[body..].iter().position(|l| !text[l.start..l.end].trim().is_empty());
    Some(body + named.unwrap_or(0))
}

pub struct NoteText<'a> {
    pub eng: &'a Eng,
    pub fonts: &'a Fonts,
    pub text: &'a str,
    /// The text's lines and their blocks (see `kolada_markdown::lines`).
    pub lines: &'a [Line],
    /// Column width, DIPs.
    pub width: f32,
    /// The line drawn as the title (see `title_line`).
    pub title: Option<usize>,
    /// The caret: marks around it show.
    pub caret: Option<usize>,
    /// The pictures it shows; without, embeds read as text.
    pub pictures: Option<Pictures<'a>>,
    /// Tables' columns measured this frame, by their header line's start.
    pub tables: std::cell::RefCell<Vec<(usize, Rc<Table>)>>,
}

impl NoteText<'_> {
    fn format(&self, line: &Line, title: bool, caret_on_line: bool) -> Format {
        if title {
            return self.fonts.title;
        }
        match line.block {
            Block::Heading(n) => self.fonts.headings[(n.clamp(1, 6) - 1) as usize],
            Block::Code | Block::Frontmatter => self.fonts.mono,
            // A fence folds into a sliver of padding on the code's panel,
            // and opens when the caret is on it.
            Block::Fence if caret_on_line => self.fonts.mono,
            Block::Fence => self.fonts.collapsed,
            _ => self.fonts.body,
        }
    }

    fn caret_in(&self, from: usize, to: usize) -> bool {
        self.caret.is_some_and(|c| from <= c && c <= to)
    }

    /// Indentation and the space above, by block.
    fn geometry(line: &Line, title: bool, first: bool) -> (f32, f32) {
        let indent = match line.block {
            b if b.is_item() => (line.depth as f32 + 1.0) * LIST_INDENT,
            Block::Quote => line.depth as f32 * QUOTE_INDENT,
            Block::Fence | Block::Code => CODE_INDENT,
            _ => 0.0,
        };
        let pad = match line.block {
            _ if first || title => 0.0,
            Block::Heading(1) => 18.0,
            Block::Heading(2) => 14.0,
            Block::Heading(_) => 10.0,
            _ => 0.0,
        };
        (indent, pad)
    }

    /// Whether the `i`th line reads right to left: by its own first letter,
    /// or, with none yet (empty, or only numbers and marks), like the line
    /// before it. A new line after Arabic stays right to left until a Latin
    /// letter is typed.
    pub fn line_rtl(&self, i: usize) -> bool {
        (0..=i.min(self.lines.len().saturating_sub(1)))
            .rev()
            .take(64)
            .find_map(|j| {
                let l = self.lines[j];
                scene::direction(&self.text[l.content().min(l.end)..l.end])
            })
            .unwrap_or(false)
    }

    /// The paragraph of the `i`th line.
    pub fn para(&self, i: usize) -> Result<Para> {
        let line = self.lines[i];
        let title = self.title == Some(i);
        let text = &self.text[line.start..line.end];
        let on = self.caret_in(line.start, line.end);
        let rtl = self.line_rtl(i);
        let (indent, pad) = Self::geometry(&line, title, i == 0);
        let format = self.format(&line, title, on);
        let width = (self.width - indent).max(1.0);
        let spans = md::line_spans(self.text, &line);
        // A table's line: its cells in their columns; its own text hidden.
        if let Some(row) = self.table_row(i, &line, &spans, on) {
            let hidden = Look { range: 0..text.len(), size: Some(HIDDEN_SIZE), ..Look::default() };
            let layout = scene::kept_layout(self.eng, text, &format, width, 100_000.0, HIDDEN_LINE, || scene::layout_styled(self.eng, text, &format, width, 100_000.0, &[hidden]));
            return Ok(Para { start: line.start, end: line.end, line, spans, rtl, layout, x: 0.0, pad: 0.0, picture: None, row: Some(row) });
        }
        // A line that is only a picture shows the picture, at the size it
        // asks for (or is being dragged to); selected, it is chosen whole.
        if let Some(p) = &self.pictures
            && let Some(name) = crate::images::embed_in(text)
        {
            let found = p.images.borrow_mut().place(p.vault, &p.dir, name);
            let wanted = match p.resize {
                Some((at, w)) if at == line.start => Some(w),
                _ => crate::images::embed_width(text).map(|w| w as f32),
            };
            let picture = match found {
                crate::images::Lookup::Found(path, w, h) => {
                    let (width, height) = fit_picture(w, h, self.width - indent, wanted);
                    Some(Picture { path: Some(path), width, height })
                }
                crate::images::Lookup::Pending => Some(Picture { path: None, width: (self.width - indent).min(420.0), height: PICTURE_WAIT }),
                crate::images::Lookup::Missing => None,
            };
            if let Some(picture) = picture {
                // Its text is hidden, and its line is as tall as the picture.
                let hidden = Look { range: 0..text.len(), size: Some(HIDDEN_SIZE), ..Look::default() };
                let format = Format { line: Some(picture.height), rtl: Some(rtl), ..format };
                let layout = Rc::new(scene::layout_styled(self.eng, text, &format, width, 100_000.0, &[hidden]));
                return Ok(Para { start: line.start, end: line.end, line, spans, rtl, layout, x: indent, pad: 6.0, picture: Some(picture), row: None });
            }
        }
        // Kept between frames: what else styles it is its block, whether it
        // is the title, the caret on it, which of its marks are being
        // edited, and its direction.
        let block = match line.block {
            Block::Body => 0,
            Block::Heading(n) => n as u64,
            Block::Bullet => 10,
            Block::Ordered => 11,
            Block::Task { done } => 12 + done as u64,
            Block::Quote => 14,
            Block::Fence => 15,
            Block::Code => 16,
            Block::Rule => 17,
            Block::Frontmatter => 18,
            Block::TableDelimiter => 19,
            Block::TableRow => 20,
        };
        let mut style = block | (line.mark as u64) << 8 | (line.depth as u64) << 24 | (title as u64) << 32 | (on as u64) << 33 | (rtl as u64) << 34;
        for s in &spans {
            style = style.rotate_left(5) ^ (self.caret_in(s.start, s.end) as u64 + 1);
        }
        let layout = scene::kept_layout(self.eng, text, &format, width, 100_000.0, style, || {
            let looks = self.looks(&format, &line, &spans, on);
            scene::layout_styled(self.eng, text, &Format { rtl: Some(rtl), ..format }, width, 100_000.0, &looks)
        });
        Ok(Para { start: line.start, end: line.end, line, spans, rtl, layout, x: if rtl { 0.0 } else { indent }, pad, picture: None, row: None })
    }

    /// The format of a table's cells: its header's heavier.
    fn cell_format(&self, header: bool) -> Format {
        let f = self.fonts.body;
        if header { Format { weight: HEADER_WEIGHT, ..f } } else { f }
    }

    /// The columns of the table whose header is line `h`, measured once a
    /// frame: each as wide as its widest cell, or sharing the column (see
    /// `kolada_editor::table::columns`).
    fn table(&self, h: usize) -> Rc<Table> {
        let key = self.lines[h].start;
        if let Some((_, t)) = self.tables.borrow().iter().find(|(k, _)| *k == key) {
            return t.clone();
        }
        let d = self.lines[h + 1];
        let aligns = md::alignments(&self.text[d.start..d.end]).unwrap_or_default();
        let mut natural = vec![0f32; aligns.len()];
        // Each column's longest word: the least it can be without breaking one.
        let mut longest = vec![0f32; aligns.len()];
        let rows = std::iter::once(h).chain((h + 2..self.lines.len()).take_while(|&j| self.lines[j].block == Block::TableRow)).take(MEASURED_ROWS);
        let mut rtl = false;
        for j in rows {
            let l = self.lines[j];
            let t = &self.text[l.start..l.end];
            let f = Format { wrap: false, ..self.cell_format(j == h) };
            let spans = md::line_spans(self.text, &l);
            if j == h {
                rtl = scene::is_rtl(&shown(self.text, &l, &spans, 0, t.len()));
            }
            for (c, (a, z)) in md::cells(t).into_iter().take(natural.len()).enumerate() {
                // As it reads: a link's address, and the marks, take no room.
                let shown = shown(self.text, &l, &spans, a, z);
                let cell = shown.as_str();
                let layout = scene::kept_layout(self.eng, cell, &f, 100_000.0, 100_000.0, NATURAL, || scene::layout_styled(self.eng, cell, &f, 100_000.0, 100_000.0, &[]));
                natural[c] = natural[c].max(layout.measure().width);
                let word = cell.split_whitespace().max_by_key(|w| w.chars().count()).unwrap_or("");
                if !word.is_empty() && word.len() < cell.len() {
                    let w = scene::kept_layout(self.eng, word, &f, 100_000.0, 100_000.0, LONGEST_WORD, || scene::layout_styled(self.eng, word, &f, 100_000.0, 100_000.0, &[]));
                    longest[c] = longest[c].max(w.measure().width);
                } else {
                    longest[c] = longest[c].max(layout.measure().width);
                }
            }
        }
        let natural: Vec<f32> = natural.into_iter().map(|w| w + 2.0 * CELL_PAD).collect();
        // A little over the word, so rounding never tips it onto two lines.
        let longest: Vec<f32> = longest.into_iter().map(|w| w + 2.0 * CELL_PAD + 1.0).collect();
        let table = Rc::new(Table { columns: kolada_editor::table::columns(&natural, &longest, self.width, CELL_LEAST), aligns, rtl });
        self.tables.borrow_mut().push((key, table.clone()));
        table
    }

    /// Line `i` as a table row, if it is one: its cells laid out in their
    /// columns, styled as its Markdown says. A table's alignment row is a
    /// row with no cells.
    fn table_row(&self, i: usize, line: &Line, spans: &[Span], on: bool) -> Option<Row> {
        if line.block == Block::TableDelimiter {
            return Some(Row { cells: Vec::new(), header: false, last: false, height: 0.0, width: 0.0 });
        }
        let header = md::is_table_header(self.lines, i);
        if !header && line.block != Block::TableRow {
            return None;
        }
        let h = if header { i } else { (0..i).rev().find(|&j| self.lines[j].block == Block::TableDelimiter)?.checked_sub(1)? };
        let table = self.table(h);
        let last = self.lines.get(i + 1).is_none_or(|n| n.block != Block::TableRow);
        let text = &self.text[line.start..line.end];
        let ranges = kolada_editor::table::cells(text);
        let format = self.cell_format(header);
        let looks = self.looks(&format, line, spans, on);
        let mut cells = Vec::with_capacity(table.columns.len());
        let mut height: f32 = 0.0;
        for (c, &(x, w)) in table.columns.iter().enumerate() {
            // A cell's text; with the caret after spaces typed at its end
            // (still in the cell: see `table::Cell::room`), up to the caret.
            let (a, z) = ranges.get(c).map_or((text.len(), text.len()), |cell| match self.caret.map(|k| k.wrapping_sub(line.start)) {
                Some(k) if k > cell.text.1 && k <= cell.room.1 => (cell.text.0, k),
                _ => cell.text,
            });
            let cell = &text[a..z];
            let align = match table.aligns.get(c) {
                Some(md::Align::Center) => Align::Center,
                Some(md::Align::Right) => Align::End,
                _ => Align::Start,
            };
            let f = Format { align, ..format };
            let inner = (w - 2.0 * CELL_PAD).max(1.0);
            let clipped: Vec<Look> = looks
                .iter()
                .filter_map(|l| {
                    let (s, e) = (l.range.start.max(a), l.range.end.min(z));
                    (s < e).then(|| Look { range: s - a..e - a, ..l.clone() })
                })
                .collect();
            let mut style = 0x7AB1_E000_0000_0000 | header as u64 | (c as u64) << 1 | (align as u64) << 12;
            for s in spans.iter().filter(|s| s.start >= line.start + a && s.end <= line.start + z) {
                style = style.rotate_left(5) ^ (self.caret_in(s.start, s.end) as u64 + 1);
            }
            let layout = scene::kept_layout(self.eng, cell, &f, inner, 100_000.0, style, || scene::layout_styled(self.eng, cell, &f, inner, 100_000.0, &clipped));
            height = height.max(layout.measure().height);
            // A table that reads right to left runs its columns from the right.
            let x = if table.rtl { table.columns.last().map_or(0.0, |&(x, w)| x + w) - x - w } else { x };
            cells.push(Cell { start: line.start + a, end: line.start + z, x, width: w, layout });
        }
        let width = table.columns.last().map_or(0.0, |&(x, w)| x + w);
        Some(Row { cells, header, last, height: height + 2.0 * CELL_VPAD, width })
    }

    /// The Markdown's look over a paragraph's text: what to hide, color,
    /// embolden. Ranges are bytes within the paragraph; later looks win.
    fn looks(&self, format: &Format, line: &Line, spans: &[Span], on: bool) -> Vec<Look> {
        let mut out = Vec::new();
        let range = |a: usize, b: usize| (a - line.start)..(b - line.start);
        let hide = |out: &mut Vec<Look>, a: usize, b: usize| out.push(Look { range: range(a, b), size: Some(HIDDEN_SIZE), ..Look::default() });
        let ink = |out: &mut Vec<Look>, a: usize, b: usize, ink: Ink| out.push(Look { range: range(a, b), ink: ink.id(), ..Look::default() });
        if line.mark > 0 && on && matches!(line.block, Block::Heading(_)) {
            // A heading's #s, for editing, while the caret is on its line.
            ink(&mut out, line.start, line.content(), Ink::Faint);
        } else if line.mark > 0 {
            // Tabs that indent a list hide with the rest of the mark: a
            // hidden tab takes no room, and the scene draws the indentation.
            hide(&mut out, line.start, line.content());
        }
        match line.block {
            // A rule is drawn as a line; its dashes show only for editing.
            Block::Rule | Block::Fence if !on => {
                hide(&mut out, line.start, line.end);
                return out;
            }
            Block::Rule => {
                ink(&mut out, line.start, line.end, Ink::Faint);
                return out;
            }
            _ => {}
        }
        for s in spans {
            let editing = self.caret_in(s.start, s.end);
            for (a, b) in s.marks() {
                if a < b {
                    if editing { ink(&mut out, a, b, Ink::Faint) } else { hide(&mut out, a, b) }
                }
            }
            let (a, b) = (s.text_start, s.text_end);
            if a >= b {
                continue;
            }
            let r = range(a, b);
            match s.style {
                Style::Strong => out.push(Look { range: r, weight: Some(700), ..Look::default() }),
                Style::Emphasis => out.push(Look { range: r, italic: Some(true), ..Look::default() }),
                Style::Strike => out.push(Look { range: r, strike: true, ink: Ink::Muted.id(), ..Look::default() }),
                Style::Code => out.push(Look { range: r, role: Some(Role::Code), size: Some(format.size * CODE_SCALE), ..Look::default() }),
                Style::Highlight => {}
                Style::WikiLink | Style::Link | Style::Tag | Style::Url => ink(&mut out, a, b, Ink::Link),
            }
        }
        out
    }

    /// The index of the line holding `offset`.
    pub fn line_index_at(&self, offset: usize) -> usize {
        self.lines.partition_point(|l| l.start <= offset).max(1) - 1
    }

    /// The paragraph holding `offset`.
    pub fn para_at(&self, offset: usize) -> Result<Para> {
        self.para(self.line_index_at(offset.min(self.text.len())))
    }

    /// Height a paragraph takes when scrolling: its own and the space above.
    pub fn block_height(&self, p: &Para) -> Result<f32> {
        Ok(p.pad + p.height())
    }

    /// Whether the caret is on `line`.
    pub fn caret_on(&self, line: &Line) -> bool {
        self.caret_in(line.start, line.end)
    }

    /// Height of the paragraph's first visual line, where block marks sit.
    pub fn first_line_height(&self, p: &Para) -> f32 {
        match &p.row {
            Some(row) => row.height,
            None => p.layout.lines().first().map_or(0.0, |l| l.height),
        }
    }

    /// Caret box at `offset` within `p`: x from the column's left edge, y
    /// (top) from the paragraph's, and height: the whole visual line, which
    /// stays right even beside hidden marks.
    pub fn caret(&self, p: &Para, offset: usize) -> Result<(f32, f32, f32)> {
        if let Some(row) = &p.row {
            let Some(cell) = row.cell_near(offset) else { return Ok((p.x, 0.0, row.height.max(1.0))) };
            let c = cell.layout.caret(offset.clamp(cell.start, cell.end) - cell.start);
            return Ok((p.x + cell.x + CELL_PAD + c.x, CELL_VPAD + c.y, c.height));
        }
        let c = p.layout.caret(offset.clamp(p.start, p.end) - p.start);
        Ok((p.x + c.x, c.y, c.height))
    }

    /// The text offset nearest a point (x from the column's left edge, y
    /// from the paragraph's top).
    pub fn offset_at(&self, p: &Para, x: f32, y: f32) -> Result<usize> {
        if let Some(row) = &p.row {
            // In a table, the cell whose column the point is over (or the
            // nearest, beside the table), whichever way its columns run.
            let x = x - p.x;
            let off = |c: &&Cell| if x < c.x { c.x - x } else if x >= c.x + c.width { x - c.x - c.width } else { 0.0 };
            let Some(cell) = row.cells.iter().min_by(|a, b| off(a).total_cmp(&off(b))) else { return Ok(p.start) };
            return Ok(cell.start + cell.layout.offset_at(x - cell.x - CELL_PAD, y - CELL_VPAD).0);
        }
        Ok(p.start + p.layout.offset_at(x - p.x, y).0)
    }

    /// Boxes covering `from..to` within `p` (left, top, right, bottom): x
    /// from the column's left edge, y from the paragraph's top.
    pub fn range(&self, p: &Para, from: usize, to: usize) -> Result<Vec<[f32; 4]>> {
        if let Some(row) = &p.row {
            // In a table, within each cell: the pipes between take none.
            let mut out = Vec::new();
            for cell in &row.cells {
                let (a, b) = (from.max(cell.start), to.min(cell.end));
                if a < b {
                    for [x, y, w, h] in cell.layout.range(a - cell.start, b - cell.start) {
                        let left = p.x + cell.x + CELL_PAD + x;
                        out.push([left, CELL_VPAD + y, left + w, CELL_VPAD + y + h]);
                    }
                }
            }
            return Ok(out);
        }
        let boxes = p.layout.range(from - p.start, to - p.start);
        Ok(boxes.into_iter().map(|[x, y, w, h]| [p.x + x, y, p.x + x + w, y + h]).collect())
    }

    /// The offset closest to `x` on the middle of visual line `line` of `p`.
    fn offset_on(&self, p: &Para, line: &VisualLine, x: f32) -> Option<usize> {
        self.offset_at(p, x, line.top + line.height / 2.0).ok()
    }

    fn lines_at(&self, offset: usize) -> Option<(Para, Vec<VisualLine>, usize)> {
        let p = self.para_at(offset).ok()?;
        let lines = self.visual_lines_of(&p, offset);
        let i = line_index(&lines, offset.clamp(p.start, p.end) - p.start);
        Some((p, lines, i))
    }

    /// A paragraph's visual lines, bytes from its start: in a table row,
    /// those of the cell holding `offset`; a table's alignment row is one
    /// line with no height.
    fn visual_lines_of(&self, p: &Para, offset: usize) -> Vec<VisualLine> {
        match &p.row {
            Some(row) => match row.cell_near(offset) {
                Some(c) => visual_lines(&c.layout, &self.text[c.start..c.end])
                    .into_iter()
                    .map(|l| VisualLine { start: l.start + c.start - p.start, end: l.end + c.start - p.start, top: l.top + CELL_VPAD, ..l })
                    .collect(),
                None => vec![VisualLine { start: 0, end: p.end - p.start, trailing_space: 0, top: 0.0, height: 0.0 }],
            },
            None => visual_lines(&p.layout, &self.text[p.start..p.end]),
        }
    }

    /// A table's alignment row is never landed on, moving up or down.
    fn skips(p: &Para) -> bool {
        p.row.as_ref().is_some_and(|r| r.cells.is_empty())
    }
}

impl EditorLayout for NoteText<'_> {
    fn visual_line(&self, offset: usize) -> (usize, usize) {
        let Some((p, lines, i)) = self.lines_at(offset) else { return (offset, offset) };
        let s = &self.text[p.start..p.end];
        let l = &lines[i];
        let start = p.start + l.start;
        if i + 1 == lines.len() {
            return (start, p.end);
        }
        // End of a wrapped line: before the space it wrapped at. A word
        // broken mid-line has none; stop before its last character there,
        // since its end is also where the next line starts.
        let end = p.start + l.end - l.trailing_space;
        let end = if l.trailing_space == 0 { s[..end - p.start].char_indices().last().map_or(start, |(i, _)| p.start + i) } else { end };
        (start, end.max(start))
    }

    fn x_of(&self, offset: usize) -> f32 {
        self.para_at(offset).and_then(|p| self.caret(&p, offset)).map_or(0.0, |(x, _, _)| x)
    }

    fn above(&self, offset: usize, x: f32) -> Option<usize> {
        let (p, lines, i) = self.lines_at(offset)?;
        if i > 0 {
            return self.offset_on(&p, &lines[i - 1], x);
        }
        if p.start == 0 {
            return None;
        }
        let mut q = self.para_at(p.start - 1).ok()?;
        while Self::skips(&q) && q.start > 0 {
            q = self.para_at(q.start - 1).ok()?;
        }
        let q_lines = self.visual_lines_of(&q, q.end);
        self.offset_on(&q, q_lines.last()?, x)
    }

    fn below(&self, offset: usize, x: f32) -> Option<usize> {
        let (p, lines, i) = self.lines_at(offset)?;
        if i + 1 < lines.len() {
            return self.offset_on(&p, &lines[i + 1], x);
        }
        if p.end >= self.text.len() {
            return None;
        }
        let mut q = self.para_at(p.end + 1).ok()?;
        while Self::skips(&q) && q.end < self.text.len() {
            q = self.para_at(q.end + 1).ok()?;
        }
        let q_lines = self.visual_lines_of(&q, q.start);
        self.offset_on(&q, q_lines.first()?, x)
    }

    fn rtl(&self, offset: usize) -> bool {
        self.line_rtl(self.line_index_at(offset.min(self.text.len())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_editor::{Editor, Motion, Selection};

    #[test]
    fn fits_pictures_to_the_column() {
        assert_eq!(fit_picture(400, 200, 700.0, None), (400.0, 200.0), "natural size when it fits");
        assert_eq!(fit_picture(1400, 700, 700.0, None), (700.0, 350.0), "as wide as the column");
        assert_eq!(fit_picture(400, 200, 700.0, Some(300.0)), (300.0, 150.0), "as wide as it asks");
        assert_eq!(fit_picture(400, 200, 700.0, Some(900.0)), (700.0, 350.0), "but no wider than the column");
        assert_eq!(fit_picture(400, 200, 700.0, Some(10.0)).0, PICTURE_LEAST);
        let (w, h) = fit_picture(600, 2400, 700.0, None);
        assert!((h - 540.0).abs() < 0.01 && (w - 135.0).abs() < 0.01, "a tall one is shortened: {w} x {h}");
    }

    fn with_text(text: &str, width: f32, caret: Option<usize>, f: impl FnOnce(&NoteText)) {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let lines: Vec<Line> = md::lines(text).collect();
        f(&NoteText { eng: &eng, fonts: &fonts, text, lines: &lines, width, title: None, caret, pictures: None, tables: Default::default() });
    }

    #[test]
    fn moves_through_wrapped_lines_and_paragraphs() {
        let text = "one two three four five six seven eight nine ten\nshort";
        with_text(text, 120.0, None, |nt| {
            let mut e = Editor::new(text);
            e.set_selection(Selection::caret(0));
            // Down walks the wrapped lines of the first paragraph, then on
            // to the second.
            let mut heads = vec![];
            for _ in 0..8 {
                e.move_caret(Motion::Down, false, nt);
                heads.push(e.selection().head);
            }
            let first_break = text.find('\n').unwrap();
            assert!(heads[0] > 0 && heads[0] < first_break, "moved within the wrapped paragraph: {heads:?}");
            assert!(heads.windows(2).all(|w| w[0] <= w[1]));
            assert_eq!(*heads.last().unwrap(), text.len(), "reached the end: {heads:?}");
            // End stops at the wrap, before the space it wrapped at.
            e.set_selection(Selection::caret(0));
            e.move_caret(Motion::LineEnd, false, nt);
            let end = e.selection().head;
            assert!(end < first_break && text.as_bytes()[end] == b' ', "end of the first visual line: {end}");
            e.move_caret(Motion::LineStart, false, nt);
            assert_eq!(e.selection().head, 0);
        });
    }

    #[test]
    fn hit_tests_back_to_the_same_offset() {
        let text = "hello world";
        with_text(text, 400.0, None, |nt| {
            let p = nt.para_at(0).unwrap();
            for offset in [0, 3, 6, 11] {
                let (x, y, h) = nt.caret(&p, offset).unwrap();
                assert!(h > 10.0);
                assert_eq!(nt.offset_at(&p, x + 0.5, y + h / 2.0).unwrap(), offset);
            }
            let boxes = nt.range(&p, 0, 5).unwrap();
            assert_eq!(boxes.len(), 1);
            assert!(boxes[0][2] > boxes[0][0]);
        });
    }

    #[test]
    fn hides_marks_away_from_the_caret_and_shows_them_at_it() {
        let text = "a **bold** word";
        let x_of_word = |caret| {
            let mut x = 0.0;
            with_text(text, 400.0, caret, |nt| {
                let p = nt.para_at(0).unwrap();
                x = nt.caret(&p, text.find("word").unwrap()).unwrap().0;
            });
            x
        };
        let hidden = x_of_word(Some(text.len()));
        let shown = x_of_word(Some(5));
        assert!(shown > hidden + 8.0, "the marks take room only while editing: {hidden} vs {shown}");
    }

    #[test]
    fn lines_without_letters_keep_the_direction_before_them() {
        let text = "مرحبا بكم\n\n- \n123\nhello\n\n";
        with_text(text, 400.0, None, |nt| {
            let dirs: Vec<bool> = (0..nt.lines.len()).map(|i| nt.line_rtl(i)).collect();
            // Arabic, then an empty line, an empty item and numbers stay
            // right to left, until Latin letters turn it.
            assert_eq!(dirs, [true, true, true, true, false, false, false]);
            assert!(nt.para(1).unwrap().rtl, "an empty line after Arabic lays out right to left");
        });
    }

    #[test]
    fn a_tables_cells_line_up_in_columns_and_its_alignment_row_takes_no_room() {
        let text = "| Item | Cost |
| --- | --: |
| Press kit | 120 |
| Video | 4500 |";
        with_text(text, 600.0, None, |nt| {
            let (header, delimiter, row) = (nt.para(0).unwrap(), nt.para(1).unwrap(), nt.para(2).unwrap());
            assert!(header.row.as_ref().unwrap().header);
            assert_eq!(delimiter.height(), 0.0);
            let r = row.row.as_ref().unwrap();
            assert_eq!(r.cells.len(), 2);
            assert_eq!(&text[r.cells[0].start..r.cells[0].end], "Press kit");
            // The columns line up from row to row.
            let xs = |p: &Para| p.row.as_ref().unwrap().cells.iter().map(|c| c.x).collect::<Vec<_>>();
            assert_eq!(xs(&header), xs(&row));
            // A click in a cell lands in its text where the caret is drawn.
            let at = text.find("Press").unwrap() + 2;
            let (x, y, h) = nt.caret(&row, at).unwrap();
            assert_eq!(nt.offset_at(&row, x + 0.5, y + h / 2.0).unwrap(), at);
            // Down from the header goes past the alignment row to the rows.
            let mut e = Editor::new(text);
            e.set_selection(Selection::caret(2));
            e.move_caret(Motion::Down, false, nt);
            assert!(e.selection().head >= text.find("Press").unwrap(), "landed at {}", e.selection().head);
        });
    }

    /// A table wider than the page wraps its long cells, never a word: an
    /// email stays on one line, the titles beside it wrap instead.
    #[test]
    fn a_tables_words_are_not_broken_when_its_columns_share_the_page() {
        let text = "| # | Email | Name | Title |
| --- | --- | --- | --- |
| 2 | a.longername@example.com | Alex Longername | Communications and Marketing Officer, Regional |";
        with_text(text, 560.0, None, |nt| {
            let row = nt.para(2).unwrap();
            let cells = &row.row.as_ref().unwrap().cells;
            let lines = |c: usize| cells[c].layout.measure().height / cells[0].layout.measure().height;
            assert!(lines(1) < 1.5, "the email on one line: {} lines", lines(1));
            assert!(lines(3) > 1.5, "the title wraps instead: {} lines", lines(3));
        });
    }

    #[test]
    fn a_links_hidden_address_takes_no_room_in_its_column() {
        let text = "| Item | Source |\n| --- | --- |\n| Kit | see the [docs](https://example.com/a/very/long/path/that/goes/on/and/on/for/ever/and/ever/more/and/more/again) |";
        with_text(text, 500.0, None, |nt| {
            let row = nt.para(2).unwrap();
            let r = row.row.as_ref().unwrap();
            // The table fits the page: the address is not drawn, so it is
            // not measured.
            assert!(r.width <= 500.0, "the table is {} wide", r.width);
            assert!(r.cells[1].width < 300.0, "{}", r.cells[1].width);
        });
    }

    #[test]
    #[ignore = "writes a right-to-left table with long hidden link addresses for manual inspection"]
    fn dump_table_rtl() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let link = "([\u{0645}\u{0635}\u{062F}\u{0631}](https://example.org/reports/2026/funding-models-for-nonprofit-health-programs-and-long-term-sustainability.pdf))";
        let text = format!(
            "## \u{0627}\u{0644}\u{062A}\u{0645}\u{0648}\u{064A}\u{0644}\n\n| \u{0627}\u{0644}\u{0628}\u{0646}\u{062F} | \u{0627}\u{0644}\u{0648}\u{0636}\u{0639} | \u{0627}\u{0644}\u{062C}\u{0647}\u{0629} | \u{0645}\u{0644}\u{0627}\u{062D}\u{0638}\u{0627}\u{062A} |\n| --- | --- | --- | --- |\n| \u{062A}\u{0643}\u{0644}\u{0641}\u{0629} \u{0627}\u{0644}\u{062C}\u{0647}\u{0627}\u{0632} | \u{0645}\u{0646}\u{062D} \u{0633}\u{0646}\u{0648}\u{064A}\u{0629} \u{0645}\u{0646} \u{0645}\u{0624}\u{0633}\u{0633}\u{0627}\u{062A} \u{062E}\u{064A}\u{0631}\u{064A}\u{0629}\u{061B} Gene Therapy \u{0628}\u{062F}\u{0623} \u{0641}\u{064A} 2026 {link} | \u{062D}\u{0643}\u{0648}\u{0645}\u{064A} | \u{064A}\u{062D}\u{062A}\u{0627}\u{062C} \u{0645}\u{0631}\u{0627}\u{062C}\u{0639}\u{0629} \u{0633}\u{0646}\u{0648}\u{064A}\u{0629} |\n| \u{062A}\u{0631}\u{062A}\u{064A}\u{0628} \u{0627}\u{0644}\u{062F}\u{0641}\u{0639} | \u{062F}\u{0641}\u{0639}\u{0627}\u{062A} \u{0639}\u{0644}\u{0649} \u{0645}\u{0631}\u{0627}\u{062D}\u{0644} {link} | \u{062E}\u{0627}\u{0635} | \u{2014} |\n\nText after.\n"
        );
        let lines: Vec<Line> = md::lines(&text).collect();
        let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
        for (name, w) in [("wide", 1000.0), ("narrow", 560.0)] {
            let input = scene::Input { text: &text, lines: &lines, selection: (0, 0), page_fade: 1.0, toolbar_dock: 1.0, toolbar_show: 1.0, ..Default::default() };
            let s = scene::build(&eng, &fonts, &input, w, 420.0).unwrap();
            let list = crate::display::record(&eng, &s, &crate::theme::LIGHT, 1.5).unwrap();
            crate::raster::Surface::draw_new(((w * 1.5) as u32, 630, 144), &list).dump(&dir.join(format!("table-rtl-{name}.bmp"))).unwrap();
        }
    }

    #[test]
    fn a_table_that_reads_right_to_left_runs_its_columns_from_the_right() {
        let text = "| \u{0627}\u{0644}\u{0628}\u{0646}\u{062F} | \u{0627}\u{0644}\u{0645}\u{0635}\u{062F}\u{0631} |\n| --- | --- |\n| \u{062A}\u{0643}\u{0644}\u{0641}\u{0629} | \u{062A}\u{0645}\u{0648}\u{064A}\u{0644} |";
        with_text(text, 500.0, None, |nt| {
            let row = nt.para(2).unwrap();
            let r = row.row.as_ref().unwrap();
            assert!(r.cells[0].x > r.cells[1].x, "the first column on the right: {} vs {}", r.cells[0].x, r.cells[1].x);
            // A click over each column lands in that column's cell.
            for (k, c) in r.cells.iter().enumerate() {
                let at = nt.offset_at(&row, row.x + c.x + c.width / 2.0, 12.0).unwrap();
                assert!((c.start..=c.end).contains(&at), "column {k}: {at} not in {}..{}", c.start, c.end);
            }
        });
        // Left to right, as before.
        with_text("| a | b |\n| --- | --- |\n| 1 | 2 |", 500.0, None, |nt| {
            let row = nt.para(2).unwrap();
            let r = row.row.as_ref().unwrap();
            assert!(r.cells[0].x < r.cells[1].x);
        });
    }

    #[test]
    fn the_caret_shows_after_spaces_typed_in_a_cell_and_in_an_empty_cell() {
        // "Video " with the caret after its space: drawn past the text's end.
        let text = "| Item | Cost |\n| --- | --- |\n| Video  |  |";
        let end = text.find("Video").unwrap() + "Video".len();
        let x_at = |caret: usize| {
            let mut x = (0.0, 0.0);
            with_text(text, 600.0, Some(caret), |nt| {
                let row = nt.para(2).unwrap();
                x = (nt.caret(&row, caret).unwrap().0, row.row.as_ref().unwrap().cells[1].x);
            });
            x
        };
        let ((before, _), (after, next)) = (x_at(end), x_at(end + 1));
        assert!(after > before + 1.0 && after < next, "{before} {after} {next}");
        // An empty cell: the caret in its own column, not the one before.
        let empty = text.rfind("|  |").unwrap() + 2;
        with_text(text, 600.0, Some(empty), |nt| {
            let row = nt.para(2).unwrap();
            let r = row.row.as_ref().unwrap();
            let (x, _, _) = nt.caret(&row, empty).unwrap();
            assert!(x >= r.cells[1].x, "{x} is not in the second column at {}", r.cells[1].x);
        });
    }

    #[test]
    fn indents_list_items_and_hides_their_marks() {
        let text = "- milk\n\t- oat";
        with_text(text, 400.0, None, |nt| {
            let top = nt.para(0).unwrap();
            let nested = nt.para(1).unwrap();
            assert_eq!(top.x, LIST_INDENT);
            assert_eq!(nested.x, 2.0 * LIST_INDENT);
            // The text starts right at the indent: the "- " takes no room.
            let (x, _, _) = nt.caret(&top, 2).unwrap();
            assert!((x - LIST_INDENT).abs() < 0.5, "{x}");
        });
    }
}
