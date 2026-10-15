//! Formatting commands, for the toolbar and shortcuts. They write plain
//! Markdown marks, which the view then shows as formatting: bold wraps the
//! selection in `**`, a bullet list puts `- ` before each line.

use kolada_markdown::{self as md, Block, Line, Span, Style};

use crate::{Edit, Editor, Selection};

/// Inline formatting, written as marks around text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inline {
    Bold,
    Italic,
    Strike,
    Code,
    Highlight,
}

impl Inline {
    /// The span it makes, as Markdown reads it.
    pub(crate) fn style(self) -> Style {
        match self {
            Inline::Bold => Style::Strong,
            Inline::Italic => Style::Emphasis,
            Inline::Strike => Style::Strike,
            Inline::Code => Style::Code,
            Inline::Highlight => Style::Highlight,
        }
    }

    pub(crate) fn mark(self) -> &'static str {
        match self {
            Inline::Bold => "**",
            Inline::Italic => "*",
            Inline::Strike => "~~",
            Inline::Code => "`",
            Inline::Highlight => "==",
        }
    }
}

/// What a line is, as the toolbar sets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Body,
    Heading(u8),
    Bullet,
    Numbered,
    Task,
    Quote,
}

impl BlockKind {
    fn is(self, block: Block) -> bool {
        match (self, block) {
            (BlockKind::Body, Block::Body) => true,
            (BlockKind::Heading(a), Block::Heading(b)) => a == b,
            (BlockKind::Bullet, Block::Bullet) => true,
            (BlockKind::Numbered, Block::Ordered) => true,
            (BlockKind::Task, Block::Task { .. }) => true,
            (BlockKind::Quote, Block::Quote) => true,
            _ => false,
        }
    }
}

impl Editor {
    /// The lines the selection touches (a selection ending at the very start
    /// of a line leaves that line out).
    fn selected_lines(&self) -> Vec<Line> {
        let (a, b) = (self.selection.start(), self.selection.end());
        let lines: Vec<Line> = self
            .outline
            .iter()
            .filter(|l| l.start <= b && l.end >= a && !(a < b && b == l.start && l.start > a))
            .copied()
            .collect();
        if lines.is_empty() { vec![self.line_at(a)] } else { lines }
    }

    /// Bold, italic and the rest: wraps the selection in the style's marks,
    /// or removes them if it already has them. With nothing selected, in
    /// clean editing the style waits for what is typed next (see `pend`);
    /// with Show Markdown, an empty pair to type into.
    pub fn toggle_inline(&mut self, style: Inline) {
        if self.selection.is_caret() {
            if self.clean {
                return self.pend(style);
            }
            let mark = style.mark();
            return self.wrap(mark, mark);
        }
        self.format_selection(style);
    }

    /// `style` over the selection, as Markdown can hold it: never one span
    /// inside another of its kind (bold inside bold shows its marks), never
    /// in code, never across another span's marks. Spans of the style the
    /// selection overlaps merge into one; a selection wholly in the style
    /// comes out of it (splitting a span it is in the middle of). One undo
    /// step; the same words stay selected.
    fn format_selection(&mut self, style: Inline) {
        let (a, b) = (self.selection.start(), self.selection.end());
        let target = style.style();
        let mark = style.mark();
        // Each line's part of the selection, without the spaces at its
        // ends, reaching over any other span it cuts into.
        let mut parts: Vec<(usize, usize, Vec<Span>)> = Vec::new();
        for line in self.selected_lines() {
            if !line.block.has_spans() {
                continue;
            }
            let mut x = a.max(line.content());
            let mut y = b.min(line.end);
            if x >= y {
                continue;
            }
            let part = &self.text[x..y];
            x += part.len() - part.trim_start().len();
            y -= part.len() - part.trim_end().len();
            if x >= y {
                continue;
            }
            let spans = md::line_spans(&self.text, &line);
            // Code holds nothing but itself.
            if target != Style::Code && spans.iter().any(|s| s.style == Style::Code && s.start < y && s.end > x) {
                continue;
            }
            loop {
                // A span that crosses an end of the selection (one it is
                // wholly inside of is fine: the new span nests in it).
                let inside = |s: &Span| s.text_start <= x && y <= s.text_end;
                let cut = spans.iter().find(|s| s.style != target && s.start < y && s.end > x && (s.start < x || s.end > y) && !inside(s));
                match cut {
                    Some(s) => {
                        x = x.min(s.start);
                        y = y.max(s.end);
                    }
                    None => break,
                }
            }
            parts.push((x, y, spans));
        }
        if parts.is_empty() {
            return;
        }
        // The span of the style that every letter of `x..y` shown is in.
        let text = &self.text;
        let within = |x: usize, y: usize, spans: &[Span]| {
            let mark = |p: usize| spans.iter().any(|s| (s.start..s.text_start).contains(&p) || (s.text_end..s.end).contains(&p));
            let shown: Vec<usize> = (x..y).filter(|&p| text.is_char_boundary(p) && !mark(p) && !text[p..].starts_with(char::is_whitespace)).collect();
            if shown.is_empty() {
                return None;
            }
            spans.iter().find(|s| s.style == target && shown.iter().all(|&p| s.text_start <= p && p < s.text_end)).copied()
        };
        let unwrap = parts.iter().all(|(x, y, spans)| within(*x, *y, spans).is_some());
        let mut edits = Vec::new();
        for (x, y, spans) in &parts {
            let (x, y) = (*x, *y);
            if unwrap {
                let s = within(x, y, spans).unwrap();
                // Marks hug their words: what stays in the style before
                // the selection closes before the spaces, what stays after
                // opens after them.
                if x <= s.text_start {
                    edits.push(Edit::remove(s.start, s.text_start - s.start));
                } else {
                    let at = s.text_start.max(x - (self.text[..x].len() - self.text[..x].trim_end().len()));
                    edits.push(Edit::insert(at, self.text[s.text_end..s.end].to_string()));
                }
                if y >= s.text_end {
                    edits.push(Edit::remove(s.text_end, s.end - s.text_end));
                } else {
                    let rest = &self.text[y..s.text_end];
                    let at = (y + (rest.len() - rest.trim_start().len())).min(s.text_end);
                    edits.push(Edit::insert(at, self.text[s.start..s.text_start].to_string()));
                }
                continue;
            }
            if within(x, y, spans).is_some() {
                // Already in the style: left as it is.
                continue;
            }
            let over: Vec<Span> = spans.iter().filter(|s| s.style == target && s.start <= y && s.end >= x).copied().collect();
            // The new span reaches over every span of the style it touches;
            // their marks go, and fresh ones go in at its two ends.
            let from = over.iter().map(|s| s.start).fold(x, usize::min);
            let to = over.iter().map(|s| s.end).fold(y, usize::max);
            for s in &over {
                edits.push(Edit::remove(s.start, s.text_start - s.start));
                edits.push(Edit::remove(s.text_end, s.end - s.text_end));
            }
            // Code shows everything as it is: the other spans in it lose
            // their marks.
            if target == Style::Code {
                for s in spans.iter().filter(|s| s.style != Style::Code && from <= s.start && s.end <= to) {
                    edits.push(Edit::remove(s.start, s.text_start - s.start));
                    edits.push(Edit::remove(s.text_end, s.end - s.text_end));
                }
            }
            // What the new marks would touch, once the old ones are gone.
            let inner_start = over.iter().find(|s| s.start == from).map_or(from, |s| s.text_start);
            let inner_end = over.iter().find(|s| s.end == to).map_or(to, |s| s.text_end);
            let words = &self.text[inner_start..inner_end];
            // Next to another span's `*`, a run of stars reads some other
            // way: bold and italic take their underscore form. (A span that
            // is exactly the selection nests as `***x***`, which reads.)
            let whole = spans.iter().any(|s| s.style != target && s.start == inner_start && s.end == inner_end);
            let touching = self.text[..from].ends_with('*') || self.text[to..].starts_with('*');
            let starred = touching || (!whole && (words.starts_with('*') || words.ends_with('*')));
            let mark = match (style, starred) {
                (Inline::Bold, true) => "__",
                (Inline::Italic, true) => "_",
                _ => mark,
            };
            edits.push(Edit::insert(from, mark));
            edits.push(Edit::insert(to, mark));
        }
        // The words chosen stay chosen: from after an opening mark set at
        // the selection's start to before a closing one set at its end.
        let (from, to) = (parts[0].0, parts[parts.len() - 1].1);
        let moved = |pos: usize, opening: bool| {
            let mut out = pos as isize;
            for e in &edits {
                let before = if e.len == 0 { e.at < pos || (opening && e.at == pos) } else { e.at + e.len <= pos };
                if before {
                    out += e.text.len() as isize - e.len as isize;
                }
            }
            out.max(0) as usize
        };
        let select = Selection { anchor: moved(from, true), head: moved(to, false) };
        let select = if self.selection.anchor > self.selection.head { Selection { anchor: select.head, head: select.anchor } } else { select };
        self.apply_edits(edits, Some(select));
    }

    /// A link to another note: `[[selection]]`.
    pub fn link(&mut self) {
        self.wrap("[[", "]]");
    }

    fn wrap(&mut self, open: &str, close: &str) {
        let (a, b) = (self.selection.start(), self.selection.end());
        let (no, nc) = (open.len(), close.len());
        if a == b {
            self.apply_edits(vec![Edit::insert(a, format!("{open}{close}"))], Some(Selection::caret(a + no)));
            return;
        }
        let before = &self.text[..a];
        let after = &self.text[b..];
        // `*` next to `**` is part of bold, not italic around the selection.
        let part_of_longer = open == "*" && before.ends_with("**") && !before.ends_with("***");
        if before.ends_with(open) && after.starts_with(close) && !part_of_longer {
            let edits = vec![Edit::remove(b, nc), Edit::remove(a - no, no)];
            self.apply_edits(edits, Some(Selection { anchor: a - no, head: b - no }));
            return;
        }
        let selected = &self.text[a..b];
        if selected.len() >= no + nc && selected.starts_with(open) && selected.ends_with(close) {
            let edits = vec![Edit::remove(b - nc, nc), Edit::remove(a, no)];
            self.apply_edits(edits, Some(Selection { anchor: a, head: b - no - nc }));
            return;
        }
        // Marks do not cross lines: wrap each line's part, without the spaces
        // at its ends (`** bold **` is not bold).
        let mut edits = Vec::new();
        let mut inner = None;
        for line in self.selected_lines() {
            let mut x = a.max(line.content());
            let mut y = b.min(line.end);
            if x >= y {
                continue;
            }
            let part = &self.text[x..y];
            x += part.len() - part.trim_start().len();
            y -= part.len() - part.trim_end().len();
            if x >= y {
                continue;
            }
            edits.push(Edit::insert(y, close));
            edits.push(Edit::insert(x, open));
            inner = Some(match inner {
                None => (x, y),
                Some((first, _)) => (first, y),
            });
        }
        let select = match (inner, edits.len()) {
            (Some((x, y)), 2) => Some(Selection { anchor: x + no, head: y + no }),
            _ => None,
        };
        self.apply_edits(edits, select);
    }

    /// Makes the selected lines `kind`, or plain text if they all are
    /// already.
    pub fn toggle_block(&mut self, kind: BlockKind) {
        let lines = self.selected_lines();
        let all = lines.iter().filter(|l| editable(l)).all(|l| kind.is(l.block));
        self.set_lines(&lines, if all { BlockKind::Body } else { kind });
    }

    /// The heading button: plain text, then heading 1, 2, 3, and back.
    pub fn cycle_heading(&mut self) {
        let line = self.line_at(self.selection.head);
        let next = match line.block {
            Block::Heading(n) if n >= 3 => BlockKind::Body,
            Block::Heading(n) => BlockKind::Heading(n + 1),
            _ => BlockKind::Heading(1),
        };
        let lines = self.selected_lines();
        self.set_lines(&lines, next);
    }

    fn set_lines(&mut self, lines: &[Line], kind: BlockKind) {
        let mut number = 0;
        let edits = lines
            .iter()
            .filter(|l| editable(l))
            .map(|l| {
                // Lists keep their indentation when they change kind.
                let mark = &self.text[l.start..l.content()];
                let lead = if l.block.is_item() { &mark[..mark.len() - mark.trim_start_matches([' ', '\t']).len()] } else { "" };
                let text = match kind {
                    BlockKind::Body => String::new(),
                    BlockKind::Heading(n) => format!("{} ", "#".repeat(n as usize)),
                    BlockKind::Bullet => format!("{lead}- "),
                    BlockKind::Task => format!("{lead}- [ ] "),
                    BlockKind::Numbered => {
                        number += 1;
                        format!("{lead}{number}. ")
                    }
                    BlockKind::Quote => "> ".to_string(),
                };
                Edit { at: l.start, len: l.mark, text }
            })
            .collect();
        self.apply_edits(edits, None);
    }

    /// Code: a fenced block around several lines, backticks around a
    /// selection within one.
    pub fn code(&mut self) {
        let lines = self.selected_lines();
        if lines.len() > 1 {
            let (first, last) = (lines[0], lines[lines.len() - 1]);
            self.apply_edits(vec![Edit::insert(last.end, "\n```"), Edit::insert(first.start, "```\n")], None);
        } else {
            self.toggle_inline(Inline::Code);
        }
    }

    /// Tab and Shift+Tab in a list: nests the selected items one level
    /// deeper, or brings them out; in a table, the next or previous cell
    /// (see `next_cell`). False (nothing done) outside lists and tables.
    pub fn indent(&mut self, outdent: bool) -> bool {
        if self.next_cell(outdent) {
            return true;
        }
        let lines: Vec<Line> = self.selected_lines().into_iter().filter(|l| l.block.is_item()).collect();
        if lines.is_empty() {
            return false;
        }
        let edits = lines
            .iter()
            .map(|l| {
                let line = &self.text[l.start..l.end];
                if !outdent {
                    Edit::insert(l.start, "\t")
                } else if line.starts_with('\t') {
                    Edit::remove(l.start, 1)
                } else {
                    // Two spaces are one level for space-indented lists.
                    Edit::remove(l.start, (line.len() - line.trim_start_matches(' ').len()).min(2))
                }
            })
            .collect();
        self.apply_edits(edits, None);
        true
    }

    /// Checks or unchecks the task on the line holding `at`. False if it is
    /// not a task.
    pub fn toggle_task(&mut self, at: usize) -> bool {
        let line = self.line_at(at);
        let Block::Task { done } = line.block else { return false };
        let mark = &self.text[line.start..line.content()];
        let Some(open) = mark.find('[') else { return false };
        let at = line.start + open + 1;
        self.apply_edits(vec![Edit { at, len: 1, text: if done { " " } else { "x" }.into() }], None);
        true
    }
}

/// Lines that formatting may change: not code or metadata.
fn editable(line: &Line) -> bool {
    !matches!(line.block, Block::Fence | Block::Code | Block::Frontmatter)
}
