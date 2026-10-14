//! Tables (see `kolada_markdown::cells`): how their columns share the width,
//! and editing them as tables rather than as lines of pipes. The caret stays
//! in cells, typing keeps a row's cells apart, Tab and Enter move through
//! them and add rows, and reshaping writes the table out with its columns
//! lined up. Pure text work, the same on every platform.

use kolada_markdown::{self as md, Block};
use unicode_segmentation::UnicodeSegmentation;

use crate::{Edit, Editor, Selection, next_grapheme, next_word, prev_grapheme, prev_word};

/// Columns' left edges and widths (DIPs) for cells whose widest natural
/// widths are `natural` and whose longest words are `longest` (padding
/// included in both), in `available`. When all fit, each column is as wide
/// as its widest cell. When not, columns narrower than an even share keep
/// their width and the rest share what is left evenly (their text wraps),
/// none narrower than its longest word, so a word (an email, a link) is
/// never broken mid-way, nor than `least` (the table may then pass
/// `available`).
pub fn columns(natural: &[f32], longest: &[f32], available: f32, least: f32) -> Vec<(f32, f32)> {
    let n = natural.len();
    let total: f32 = natural.iter().sum();
    let widths: Vec<f32> = if total <= available {
        natural.to_vec()
    } else {
        // The least each column can be: its longest word, never more than
        // its widest cell.
        let floor: Vec<f32> = (0..n).map(|c| longest.get(c).copied().unwrap_or(0.0).max(least).min(natural[c])).collect();
        // Water filling: settle the columns that fit an even share of what
        // is left, and those whose words cannot go narrower than it, until
        // none more do.
        let mut kept: Vec<Option<f32>> = vec![None; n];
        let (mut left, mut open) = (available, n);
        loop {
            let share = left / open.max(1) as f32;
            let mut settled = false;
            for c in 0..n {
                if kept[c].is_some() {
                    continue;
                }
                let width = if natural[c] <= share {
                    natural[c]
                } else if floor[c] >= share {
                    floor[c]
                } else {
                    continue;
                };
                kept[c] = Some(width);
                left -= width;
                open -= 1;
                settled = true;
            }
            if !settled || open == 0 {
                break;
            }
        }
        let share = if open > 0 { left / open as f32 } else { 0.0 };
        (0..n).map(|c| kept[c].unwrap_or(share.max(floor[c]))).collect()
    };
    let mut x = 0.0;
    widths
        .into_iter()
        .map(|w| {
            let column = (x, w);
            x += w;
            column
        })
        .collect()
}

/// A cell of a table's header or row, in offsets into its line (or, from
/// the editor, into the text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Its text, trimmed: what Tab selects, where arrow keys enter and
    /// leave it. An empty cell's sits one space in from its left pipe.
    pub text: (usize, usize),
    /// Where the caret may be: from its text's start to one space short of
    /// the next pipe, so spaces typed after its text stay in it. What a
    /// shell lays out for the cell, so the caret shows after them.
    pub room: (usize, usize),
}

impl Cell {
    fn shift(self, by: usize) -> Cell {
        Cell { text: (self.text.0 + by, self.text.1 + by), room: (self.room.0 + by, self.room.1 + by) }
    }

    fn holds(&self, at: usize) -> bool {
        self.room.0 <= at && at <= self.room.1
    }

    fn is_empty(&self) -> bool {
        self.text.0 == self.text.1
    }
}

/// The cells of a table's header or row.
pub fn cells(line: &str) -> Vec<Cell> {
    md::raw_cells(line)
        .into_iter()
        .map(|(r0, r1)| {
            let raw = &line[r0..r1];
            let text = raw.trim();
            if text.is_empty() {
                let at = r0 + raw.chars().next().map_or(0, char::len_utf8);
                let end = r1 - raw.chars().next_back().map_or(0, char::len_utf8);
                Cell { text: (at, at), room: (at, end.max(at)) }
            } else {
                let a = r0 + raw.len() - raw.trim_start().len();
                let z = a + text.len();
                let end = line[z..r1].chars().next_back().map_or(z, |c| r1 - c.len_utf8());
                Cell { text: (a, z), room: (a, end) }
            }
        })
        .collect()
}

/// `s` as it may go into a cell: on one line, its pipes escaped (after an
/// odd run of backslashes, one more, so the run stays text).
fn cell_text(s: &str, before: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    let mut slashes = before.len() - before.trim_end_matches('\\').len();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '|' if slashes.is_multiple_of(2) => out.push_str("\\|"),
            '|' => out.push_str("\\\\|"),
            '\r' if chars.peek() == Some(&'\n') => continue,
            '\r' | '\n' => out.push(' '),
            _ => out.push(c),
        }
        slashes = if c == '\\' { slashes + 1 } else { 0 };
    }
    out
}

/// How many columns `s` takes in a monospaced font: two for wide East
/// Asian characters and emoji, one for any other grapheme.
fn width(s: &str) -> usize {
    s.graphemes(true).map(|g| if g.chars().next().is_some_and(wide) { 2 } else { 1 }).sum()
}

fn wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD)
}

/// A table's text as cells, for reshaping it: rows of cell texts, its
/// header first, and each column's alignment as written (a colon at its
/// left, at its right).
#[derive(Clone, Debug, PartialEq)]
struct Shape {
    rows: Vec<Vec<String>>,
    aligns: Vec<(bool, bool)>,
}

impl Shape {
    /// Writes the table out with each column padded to its widest cell, and
    /// where each cell's text starts in what is written.
    fn write(&self) -> (String, Vec<Vec<usize>>) {
        let n = self.aligns.len();
        let count = self.rows.iter().map(Vec::len).max().unwrap_or(0).max(n);
        let mut widths = vec![3; count];
        for row in &self.rows {
            for (c, cell) in row.iter().enumerate() {
                widths[c] = widths[c].max(width(cell));
            }
        }
        let mut out = String::new();
        let mut starts = Vec::with_capacity(self.rows.len());
        for (r, row) in self.rows.iter().enumerate() {
            if r > 0 {
                out.push('\n');
            }
            let mut at = Vec::new();
            out.push('|');
            let count = if r == 0 { n } else { row.len().max(n) };
            for (c, &w) in widths.iter().enumerate().take(count) {
                let cell = row.get(c).map_or("", String::as_str);
                let pad = w - width(cell);
                let (left, right) = match self.aligns.get(c) {
                    Some((true, true)) => (pad / 2, pad - pad / 2),
                    Some((false, true)) => (pad, 0),
                    _ => (0, pad),
                };
                out.push(' ');
                // An empty cell's text sits one space in (see `cells`).
                at.push(out.len() + if cell.is_empty() { 0 } else { left });
                out.extend(std::iter::repeat_n(' ', left));
                out.push_str(cell);
                out.extend(std::iter::repeat_n(' ', right));
                out.push_str(" |");
            }
            starts.push(at);
            if r == 0 {
                out.push_str("\n|");
                for (c, &(l, r)) in self.aligns.iter().enumerate() {
                    out.push(' ');
                    out.push_str(if l { ":" } else { "" });
                    out.extend(std::iter::repeat_n('-', widths[c] - l as usize - r as usize));
                    out.push_str(if r { ": |" } else { " |" });
                }
            }
        }
        (out, starts)
    }
}

/// Changes to a table's shape, for its menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableEdit {
    RowAbove,
    RowBelow,
    DeleteRow,
    ColumnLeft,
    ColumnRight,
    DeleteColumn,
    Align(md::Align),
}

/// Where the caret is in a table, for its menu: on the header row or not,
/// how many columns the table has, how the caret's column is aligned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TablePlace {
    pub header: bool,
    pub columns: usize,
    pub align: md::Align,
}

/// A table in the outline, by line: its header (the alignment row is the
/// line after) and its last line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Grid {
    header: usize,
    last: usize,
}

impl Grid {
    /// The row of the table a line is, the header being 0.
    fn row(&self, line: usize) -> usize {
        if line == self.header { 0 } else { line - self.header - 1 }
    }

    /// The line of the row before and after (past the alignment row).
    fn above(&self, line: usize) -> Option<usize> {
        match line {
            l if l == self.header + 2 => Some(self.header),
            l if l > self.header + 2 => Some(l - 1),
            _ => None,
        }
    }

    fn below(&self, line: usize) -> Option<usize> {
        let next = if line == self.header { line + 2 } else { line + 1 };
        (next <= self.last).then_some(next)
    }
}

impl Editor {
    fn line_index(&self, at: usize) -> usize {
        self.outline.partition_point(|l| l.start <= at).max(1) - 1
    }

    /// Whether line `i` has cells: a table's header or row.
    fn has_cells(&self, i: usize) -> bool {
        self.outline[i].block == Block::TableRow || md::is_table_header(&self.outline, i)
    }

    fn is_table_line(&self, i: usize) -> bool {
        self.outline[i].block == Block::TableDelimiter || self.has_cells(i)
    }

    /// The table line `i` is part of.
    fn grid(&self, i: usize) -> Option<Grid> {
        let mut header = i;
        while self.outline[header].block == Block::TableRow {
            header = header.checked_sub(1)?;
        }
        if self.outline[header].block == Block::TableDelimiter {
            header = header.checked_sub(1)?;
        }
        if !md::is_table_header(&self.outline, header) {
            return None;
        }
        let mut last = header + 1;
        while self.outline.get(last + 1).is_some_and(|l| l.block == Block::TableRow) {
            last += 1;
        }
        Some(Grid { header, last })
    }

    /// The cells of line `i`, in offsets into the text.
    fn cells_of(&self, i: usize) -> Vec<Cell> {
        let l = self.outline[i];
        cells(&self.text[l.start..l.end]).into_iter().map(|c| c.shift(l.start)).collect()
    }

    /// The table cell holding the caret at `at`: its line, its place in the
    /// row, the cell.
    pub(crate) fn cell_at(&self, at: usize) -> Option<(usize, usize, Cell)> {
        let i = self.line_index(at);
        if !self.has_cells(i) {
            return None;
        }
        let cells = self.cells_of(i);
        cells.iter().position(|c| c.holds(at)).map(|c| (i, c, cells[c]))
    }

    /// Whether the caret is in a table, where its menu applies.
    pub fn in_table(&self) -> bool {
        self.cell_at(self.selection.head).is_some()
    }

    /// Where the caret is in a table, if it is in one.
    pub fn table_place(&self) -> Option<TablePlace> {
        let (i, c, _) = self.cell_at(self.selection.head)?;
        let grid = self.grid(i)?;
        let d = self.outline[grid.header + 1];
        let aligns = md::alignments(&self.text[d.start..d.end]).unwrap_or_default();
        Some(TablePlace { header: i == grid.header, columns: aligns.len(), align: aligns.get(c).copied().unwrap_or(md::Align::Left) })
    }

    /// Moves `at` into a cell when it falls between them (on a pipe, the
    /// padding beside one, or the alignment row): forward to the next
    /// cell's text, or back to the end of the one before. Outside tables
    /// it stays.
    pub(crate) fn to_cell(&self, mut at: usize, mut forward: bool) -> usize {
        loop {
            let i = self.line_index(at);
            let line = self.outline[i];
            if line.block == Block::TableDelimiter {
                if forward && line.end < self.text.len() {
                    at = line.end + 1;
                } else {
                    at = line.start.saturating_sub(1);
                    forward = false;
                }
                continue;
            }
            if !self.has_cells(i) {
                return at;
            }
            let cells = self.cells_of(i);
            if cells.iter().any(|c| c.holds(at)) {
                return at;
            }
            if forward {
                if let Some(c) = cells.iter().find(|c| c.room.0 > at) {
                    return c.room.0;
                }
                if line.end < self.text.len() {
                    at = line.end + 1;
                    continue;
                }
                // Nothing further: the end of the last cell.
            }
            if let Some(c) = cells.iter().rev().find(|c| c.room.1 < at) {
                return c.text.1;
            }
            if line.start == 0 {
                return cells.first().map_or(at, |c| c.room.0);
            }
            at = line.start - 1;
        }
    }

    /// One grapheme forward from `head`: from the end of a cell's text,
    /// past the spaces after it to the next cell (they are the table's
    /// padding to the eye).
    pub(crate) fn step_forward(&self, head: usize) -> usize {
        match self.cell_at(head) {
            Some((_, _, c)) if head >= c.text.1 => next_grapheme(&self.text, c.room.1),
            _ => next_grapheme(&self.text, head),
        }
    }

    /// Home and End keep to the cell, and End stops at the end of its
    /// text, not of the spaces after it.
    pub(crate) fn within_cell(&self, head: usize, to: usize) -> usize {
        match self.cell_at(head) {
            Some((_, _, c)) => to.clamp(c.room.0, c.text.1),
            None => to,
        }
    }

    /// The text of the cell at `at`, when triple-clicked there.
    pub(crate) fn cell_text_at(&self, at: usize) -> Option<(usize, usize)> {
        self.cell_at(at).map(|(_, _, c)| c.text)
    }

    /// Tab in a table: selects the next cell's text (with `back`, the one
    /// before), row after row; Tab in the last cell adds a row. False
    /// outside tables.
    pub(crate) fn next_cell(&mut self, back: bool) -> bool {
        let Some((i, c, _)) = self.cell_at(self.selection.head) else { return false };
        let Some(grid) = self.grid(i) else { return false };
        let count = self.cells_of(i).len();
        let target = if back {
            if c > 0 { Some((i, c - 1)) } else { grid.above(i).map(|j| (j, self.cells_of(j).len() - 1)) }
        } else if c + 1 < count {
            Some((i, c + 1))
        } else if let Some(j) = grid.below(i) {
            Some((j, 0))
        } else {
            let mut shape = self.shape(grid);
            let width = shape.aligns.len();
            shape.rows.push(vec![String::new(); width]);
            let row = shape.rows.len() - 1;
            self.reshape(grid, &shape, (row, 0, 0));
            return true;
        };
        if let Some((j, c)) = target {
            let cell = self.cells_of(j)[c];
            self.set_selection(Selection { anchor: cell.text.0, head: cell.text.1 });
        }
        true
    }

    /// Enter in a table: a new row below the caret's, the caret in its
    /// first cell; on an empty last row, the table ends there instead (the
    /// row becomes the line after it). False outside tables.
    pub(crate) fn table_newline(&mut self) -> bool {
        let head = self.selection.head;
        let i = self.line_index(head);
        if !self.has_cells(i) {
            return false;
        }
        let Some(grid) = self.grid(i) else { return false };
        let line = self.outline[i];
        if i == grid.last && self.cells_of(i).iter().all(Cell::is_empty) {
            self.apply_edits(vec![Edit::remove(line.start, line.end - line.start)], Some(Selection::caret(line.start)));
            return true;
        }
        let mut shape = self.shape(grid);
        let row = grid.row(i) + 1;
        shape.rows.insert(row, vec![String::new(); shape.aligns.len()]);
        self.reshape(grid, &shape, (row, 0, 0));
        true
    }

    /// Backspace with the caret in a cell: nothing at its start, except
    /// that a row of empty cells goes. False outside cells.
    pub(crate) fn backspace_in_cell(&mut self, head: usize, word: bool) -> bool {
        let Some((i, _, cell)) = self.cell_at(head) else { return false };
        if head == cell.room.0 {
            let empty = self.cells_of(i).iter().all(Cell::is_empty);
            if let Some(grid) = self.grid(i)
                && empty
                && i > grid.header
            {
                let above = grid.above(i).expect("a row has a row above");
                let caret = self.cells_of(above).last().expect("rows have cells").text.1;
                let line = self.outline[i];
                self.apply_edits(vec![Edit::remove(line.start - 1, line.end - line.start + 1)], Some(Selection::caret(caret)));
            }
            return true;
        }
        let mut from = if word { prev_word(&self.text, head, cell.room.0) } else { prev_grapheme(&self.text, head) };
        from = from.max(cell.room.0);
        let mut to = head;
        // An escaped pipe goes whole, so no backslash is left to escape the
        // pipe after it.
        if self.text[from..to].ends_with('|') && self.text[..to - 1].ends_with('\\') {
            from = from.min(to - 2);
        }
        if self.text[from..to].ends_with('\\') && self.text[to..].starts_with('|') {
            to += 1;
        }
        self.replace(crate::Kind::Deleting, from, to - from, "", from);
        true
    }

    /// Delete with the caret in a cell: nothing at its end. False outside
    /// cells.
    pub(crate) fn delete_in_cell(&mut self, head: usize, word: bool) -> bool {
        let Some((_, _, cell)) = self.cell_at(head) else { return false };
        if head >= cell.room.1 {
            return true;
        }
        let mut to = if word { next_word(&self.text, head) } else { next_grapheme(&self.text, head) };
        to = to.min(cell.room.1);
        let mut from = head;
        if self.text[from..to].starts_with('|') && self.text[..from].ends_with('\\') {
            from -= 1;
        }
        if self.text[from..to].ends_with('\\') && self.text[to..].starts_with('|') {
            to += 1;
        }
        self.replace(crate::Kind::Deleting, from, to - from, "", from);
        true
    }

    /// `s` as it may be typed at `at`: on one line with its pipes escaped
    /// in a cell, as it is elsewhere.
    pub(crate) fn typed_at<'s>(&self, at: usize, s: &'s str) -> std::borrow::Cow<'s, str> {
        if self.cell_at(at).is_some() && s.contains(['|', '\n', '\r']) {
            std::borrow::Cow::Owned(cell_text(s, &self.text[..at]))
        } else {
            std::borrow::Cow::Borrowed(s)
        }
    }

    /// Whether replacing `a..b` would take pipes or line breaks out of a
    /// table: a selection across cells, or from a table out of it.
    pub(crate) fn crosses_cells(&self, a: usize, b: usize) -> bool {
        let (ia, ib) = (self.line_index(a), self.line_index(b));
        if !self.is_table_line(ia) && !self.is_table_line(ib) {
            return false;
        }
        !(ia == ib && self.cells_of(ia).iter().any(|c| c.holds(a) && c.holds(b)))
    }

    /// Replaces `a..b` with `s` keeping tables whole: in the rows it starts
    /// or ends in (in all of a table's rows, when it is within one table),
    /// only cells' text goes, their pipes and line breaks stay. Lines wholly
    /// inside it go, rows included.
    pub(crate) fn replace_across_cells(&mut self, a: usize, b: usize, s: &str) {
        let (ia, ib) = (self.line_index(a), self.line_index(b));
        let within = match (self.grid(ia), self.grid(ib)) {
            (Some(g), Some(h)) if g == h => !(a <= self.outline[g.header].start && b >= self.outline[g.last].end),
            _ => false,
        };
        let kept = |i: usize| {
            let l = self.outline[i];
            self.is_table_line(i) && (within || (i == ia && a > l.start) || (i == ib && b < l.end))
        };
        let mut out: Vec<String> = Vec::new();
        // The start of the first line, when not kept, waiting for the end
        // of the last to join it.
        let mut head: Option<String> = None;
        for i in ia..=ib {
            let l = self.outline[i];
            if kept(i) {
                out.extend(head.take());
                let mut line = String::new();
                let mut from = l.start;
                if self.has_cells(i) {
                    for c in self.cells_of(i) {
                        let (x, y) = (a.max(c.room.0), b.min(c.room.1));
                        if x < y {
                            line.push_str(&self.text[from..x]);
                            from = y;
                        }
                    }
                }
                line.push_str(&self.text[from..l.end]);
                out.push(line);
                continue;
            }
            if i == ia && a > l.start {
                head = Some(self.text[l.start..a].to_string());
            }
            if i == ib {
                let tail = &self.text[b.clamp(l.start, l.end)..l.end];
                let mut line = head.take().unwrap_or_default();
                line.push_str(tail);
                out.push(line);
            }
        }
        out.extend(head);
        // What is typed goes where the selection started: in a kept row,
        // its cell (text before it is unchanged).
        let start = self.outline[ia].start;
        let (caret, typed) = if kept(ia) && self.has_cells(ia) {
            let cells = self.cells_of(ia);
            match cells.iter().find(|c| c.holds(a)).map(|_| a).or_else(|| cells.iter().find(|c| c.room.0 > a).map(|c| c.room.0)) {
                Some(at) => (at, cell_text(s, &self.text[..at])),
                None => (a, String::new()),
            }
        } else {
            (a, s.to_string())
        };
        let mut text = out.join("\n");
        text.insert_str(caret - start, &typed);
        let end = self.outline[ib].end;
        self.apply_edits(vec![Edit { at: start, len: end - start, text }], Some(Selection::caret(caret + typed.len())));
    }

    /// Reshapes the table the caret is in. False if it is in none, or if
    /// the change would leave no table (the header row, the only column).
    pub fn edit_table(&mut self, edit: TableEdit) -> bool {
        let head = self.selection.head;
        let Some((i, c, cell)) = self.cell_at(head) else { return false };
        let Some(grid) = self.grid(i) else { return false };
        let mut shape = self.shape(grid);
        let row = grid.row(i);
        let width = shape.aligns.len();
        let caret = match edit {
            TableEdit::RowAbove | TableEdit::DeleteRow if row == 0 => return false,
            TableEdit::RowAbove | TableEdit::RowBelow => {
                let at = if edit == TableEdit::RowAbove { row } else { row + 1 };
                shape.rows.insert(at, vec![String::new(); width]);
                (at, c, 0)
            }
            TableEdit::DeleteRow => {
                shape.rows.remove(row);
                (row.min(shape.rows.len() - 1), c, 0)
            }
            TableEdit::ColumnLeft | TableEdit::ColumnRight => {
                let at = if edit == TableEdit::ColumnLeft { c } else { c + 1 };
                for r in &mut shape.rows {
                    if r.len() < at {
                        r.resize(at, String::new());
                    }
                    r.insert(at, String::new());
                }
                shape.aligns.insert(at.min(width), (false, false));
                (row, at, 0)
            }
            TableEdit::DeleteColumn if width <= 1 || c >= width => return false,
            TableEdit::DeleteColumn => {
                for r in &mut shape.rows {
                    if c < r.len() {
                        r.remove(c);
                    }
                }
                shape.aligns.remove(c);
                (row, c.min(width - 2), 0)
            }
            TableEdit::Align(_) if c >= width => return false,
            TableEdit::Align(align) => {
                shape.aligns[c] = match align {
                    md::Align::Left => (false, false),
                    md::Align::Center => (true, true),
                    md::Align::Right => (false, true),
                };
                (row, c, head.min(cell.text.1).saturating_sub(cell.text.0))
            }
        };
        self.reshape(grid, &shape, caret);
        true
    }

    fn shape(&self, grid: Grid) -> Shape {
        let texts = |i: usize| self.cells_of(i).iter().map(|c| self.text[c.text.0..c.text.1].to_string()).collect::<Vec<_>>();
        let delimiter = self.outline[grid.header + 1];
        let aligns = cells(&self.text[delimiter.start..delimiter.end])
            .into_iter()
            .map(|c| {
                let d = &self.text[delimiter.start + c.text.0..delimiter.start + c.text.1];
                (d.starts_with(':'), d.len() > 1 && d.ends_with(':'))
            })
            .collect();
        let rows = std::iter::once(grid.header).chain(grid.header + 2..=grid.last).map(texts).collect();
        Shape { rows, aligns }
    }

    /// Writes `shape` over the table, the caret at `(row, column, offset
    /// into its text)`: one undo step.
    fn reshape(&mut self, grid: Grid, shape: &Shape, (row, column, offset): (usize, usize, usize)) {
        let (text, starts) = shape.write();
        let start = self.outline[grid.header].start;
        let end = self.outline[grid.last].end;
        let row = row.min(shape.rows.len() - 1);
        let column = column.min(starts[row].len() - 1);
        let len = shape.rows[row].get(column).map_or(0, String::len);
        let caret = start + starts[row][column] + offset.min(len);
        self.apply_edits(vec![Edit { at: start, len: end - start, text }], Some(Selection::caret(caret)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_as_wide_as_their_cells_or_share_what_is_left() {
        // All fit: each as wide as its widest cell, side by side.
        assert_eq!(columns(&[40.0, 100.0, 60.0], &[], 700.0, 48.0), [(0.0, 40.0), (40.0, 100.0), (140.0, 60.0)]);
        // Too wide: the narrow ones keep their width, the two wide ones
        // share the rest evenly.
        let c = columns(&[40.0, 500.0, 400.0], &[], 400.0, 48.0);
        assert_eq!(c[0], (0.0, 40.0));
        assert_eq!(c[1].1, 180.0);
        assert_eq!(c[2], (220.0, 180.0));
        // Never under the least width, even past the room.
        let c = columns(&[300.0, 300.0, 300.0], &[], 90.0, 48.0);
        assert!(c.iter().all(|&(_, w)| w == 48.0));
        assert!(columns(&[], &[], 500.0, 48.0).is_empty());
    }

    #[test]
    fn a_column_is_never_narrower_than_its_longest_word() {
        // #, email, name, title, phone: the email has no break in it.
        let natural = [40.0, 260.0, 200.0, 320.0, 150.0];
        let longest = [40.0, 250.0, 110.0, 130.0, 60.0];
        let c = columns(&natural, &longest, 600.0, 48.0);
        let widths: Vec<f32> = c.iter().map(|&(_, w)| w).collect();
        assert!(widths[1] >= 250.0, "the email keeps its width: {widths:?}");
        assert!(widths[2] >= 110.0 && widths[3] >= 130.0 && widths[4] >= 60.0, "{widths:?}");
        let total: f32 = widths.iter().sum();
        assert!(total <= 600.0 + 0.5, "the others wrap to make room: {total}");
        // Where even the words do not fit, they keep their width and the
        // table goes past the page rather than break one.
        let c = columns(&natural, &longest, 300.0, 48.0);
        assert!(c[1].1 >= 250.0);
    }

    #[test]
    fn a_cell_has_its_text_and_room_for_the_caret_up_to_its_padding() {
        let line = "| ab  |  | c |";
        let c = cells(line);
        assert_eq!(c.len(), 3);
        // Text trimmed; room through spaces typed after it, one short of the pipe.
        assert_eq!((c[0].text, c[0].room), ((2, 4), (2, 5)));
        // An empty cell one space in.
        assert_eq!((c[1].text, c[1].room), ((8, 8), (8, 8)));
        assert_eq!((c[2].text, c[2].room), ((11, 12), (11, 12)));
        // No outer pipes, no padding.
        let c = cells("a|b");
        assert_eq!((c[0].room, c[1].room), ((0, 1), (2, 3)));
    }

    #[test]
    fn text_typed_into_a_cell_keeps_to_it() {
        assert_eq!(cell_text("a|b", ""), r"a\|b");
        assert_eq!(cell_text("one\r\ntwo\nthree", ""), "one two three");
        // After a lone backslash, one more, so the pipe is escaped and the
        // backslash stays text.
        assert_eq!(cell_text("|", r"x\"), r"\\|");
        assert_eq!(cell_text("|", r"x\\"), r"\|");
    }

    #[test]
    fn a_table_is_written_out_with_its_columns_lined_up() {
        let shape = Shape {
            rows: vec![
                vec!["Item".into(), "Qty".into(), "Note".into()],
                vec!["Press kit".into(), "12".into()],
                vec!["日本".into(), "3".into(), "ok".into()],
            ],
            aligns: vec![(false, false), (false, true), (true, true)],
        };
        let (text, starts) = shape.write();
        assert_eq!(
            text,
            "| Item      | Qty | Note |\n\
             | --------- | --: | :--: |\n\
             | Press kit |  12 |      |\n\
             | 日本      |   3 |  ok  |"
        );
        // Where each cell's text starts; an empty one sits one space in.
        let line = |r: usize| text.lines().nth(if r == 0 { 0 } else { r + 1 }).unwrap();
        let first = text.len() - text.lines().last().unwrap().len();
        assert_eq!(&text[starts[2][0]..starts[2][0] + "日本".len()], "日本");
        assert_eq!(starts[2][2] - first, line(2).find("ok").unwrap());
        let second = text.find("| Press").unwrap();
        assert_eq!(starts[1][2] - second, "| Press kit |  12 | ".len());
    }
}
