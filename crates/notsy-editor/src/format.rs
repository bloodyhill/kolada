//! Formatting commands, for the toolbar and shortcuts. They write plain
//! Markdown marks, which the view then shows as formatting: bold wraps the
//! selection in `**`, a bullet list puts `- ` before each line.

use notsy_markdown::{Block, Line};

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
    fn mark(self) -> &'static str {
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
    /// or removes them if it already has them. With nothing selected, an
    /// empty pair to type into.
    pub fn toggle_inline(&mut self, style: Inline) {
        let mark = style.mark();
        self.wrap(mark, mark);
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
