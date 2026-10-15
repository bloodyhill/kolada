//! Platform-independent text editing: the note's text, the selection,
//! editing commands and undo.
//!
//! Offsets are UTF-8 byte offsets that always sit on grapheme cluster
//! boundaries, so the caret never lands inside an emoji, a letter with its
//! combining marks, or an Arabic ligature. Line breaks are `\n`; the
//! platform converts on load and save. Anything that depends on how text
//! wraps on screen (up/down, visual line ends, right-to-left paragraphs)
//! comes from the platform through [`Layout`].
//!
//! The text is Markdown, and block marks (`- `, `## `, `1. `, `- [ ] `,
//! `> `) behave as one unit: the view draws them as bullets, checkboxes and
//! heading styles, so the caret never rests inside one, Backspace at the
//! start of an item removes its mark, and Enter continues lists. In clean
//! editing (the default) inline marks are hidden too, and editing keeps to
//! the text that shows (see `clean`).

use kolada_markdown::{self as md, Block, Line};
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

mod clean;
pub mod find;
pub mod slice;
pub mod snippets;
pub mod tidy;
pub mod table;
mod format;
pub mod title;
pub use format::{BlockKind, Inline};
pub use snippets::Snippets;
pub use table::{TableEdit, TablePlace};

/// The selection: `anchor` stays put while `head` (the caret) moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Selection {
        Selection { anchor: at, head: at }
    }

    pub fn is_caret(&self) -> bool {
        self.anchor == self.head
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }
}

/// How the text wraps on screen, answered by the platform's text layout.
pub trait Layout {
    /// Byte range of the visual (wrapped) line holding `offset`.
    fn visual_line(&self, offset: usize) -> (usize, usize);
    /// Horizontal caret position at `offset`, in any unit.
    fn x_of(&self, offset: usize) -> f32;
    /// The offset on the visual line above or below the one holding
    /// `offset`, closest to `x`; None at the top or bottom of the note.
    fn above(&self, offset: usize, x: f32) -> Option<usize>;
    fn below(&self, offset: usize, x: f32) -> Option<usize>;
    /// Whether the paragraph holding `offset` reads right to left.
    fn rtl(&self, offset: usize) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// One grapheme toward the left or right of the screen.
    Left,
    Right,
    /// One word toward the left or right of the screen.
    WordLeft,
    WordRight,
    Up,
    Down,
    /// Start and end of the visual line.
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
}

/// A caret move worked out by [`Editor::plan`], not yet applied.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    selection: Selection,
    goal_x: Option<f32>,
}

/// One text replacement, with the selections around it, for undo.
#[derive(Clone, Debug)]
struct Change {
    at: usize,
    removed: String,
    inserted: String,
    before: Selection,
    after: Selection,
}

/// A replacement for [`Editor::apply_edits`], in offsets before any of them.
#[derive(Clone, Debug)]
struct Edit {
    at: usize,
    len: usize,
    text: String,
}

impl Edit {
    fn insert(at: usize, text: impl Into<String>) -> Edit {
        Edit { at, len: 0, text: text.into() }
    }

    fn remove(at: usize, len: usize) -> Edit {
        Edit { at, len, text: String::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Typing,
    Deleting,
    Other,
    /// A writing fix applied from outside ([`Editor::amend`]): undone on
    /// its own, never grown by what is typed next.
    Amend,
}

/// Undo steps. Typing groups by word and deleting by run, the way people
/// think of their edits; moving the caret ends a group.
#[derive(Default)]
struct History {
    undo: Vec<(Kind, Vec<Change>)>,
    redo: Vec<(Kind, Vec<Change>)>,
    /// Whether the last step may still grow.
    open: bool,
}

impl History {
    fn record(&mut self, kind: Kind, change: Change) {
        self.redo.clear();
        if self.open
            && let Some((last_kind, changes)) = self.undo.last_mut()
        {
            let prev = changes.last().expect("steps are never empty");
            let joins = *last_kind == kind
                && match kind {
                    // Contiguous typing, split where a new word starts.
                    Kind::Typing => {
                        change.at == prev.at + prev.inserted.len()
                            && !(prev.inserted.ends_with(char::is_whitespace)
                                && !change.inserted.starts_with(char::is_whitespace))
                    }
                    // A run of backspaces or of deletes.
                    Kind::Deleting => change.at + change.removed.len() == prev.at || change.at == prev.at,
                    Kind::Other | Kind::Amend => false,
                };
            if joins {
                changes.push(change);
                return;
            }
        }
        self.undo.push((kind, vec![change]));
        self.open = matches!(kind, Kind::Typing | Kind::Deleting);
    }

    /// Several changes made as one command: one undo step, closed.
    fn record_step(&mut self, kind: Kind, changes: Vec<Change>) {
        self.redo.clear();
        self.undo.push((kind, changes));
        self.open = false;
    }

    fn seal(&mut self) {
        self.open = false;
    }
}

pub struct Editor {
    text: String,
    /// The text's lines and their Markdown blocks, kept current with it.
    outline: Vec<Line>,
    /// How reading the lines stood after each (for reading only what an
    /// edit touches again: see `md::relex`).
    states: Vec<md::LexState>,
    selection: Selection,
    history: History,
    /// Horizontal position kept while moving up and down through lines of
    /// different lengths.
    goal_x: Option<f32>,
    /// A new one on every change to the text (see `next_version`).
    version: u64,
    /// Text being composed with an input method, if any (see `set_marked`).
    marked: Option<Marked>,
    /// What `;;name` triggers expand to (see `snippets`).
    snippets: Snippets,
    /// Clean editing: no Markdown shows, and the caret and deletes keep to
    /// the text that does (see `clean`). Off with Show Markdown.
    clean: bool,
    /// Formatting chosen at a bare caret, for the text typed next.
    pending: Option<clean::Pending>,
}

/// Text being composed with an input method (Japanese, Chinese, Korean, the
/// emoji panel): in the text where it will go, but not yet the note's. It
/// has no undo step and no list or pair handling until it is committed; a
/// cancel puts back what it replaced.
#[derive(Clone, Debug)]
struct Marked {
    start: usize,
    len: usize,
    /// The selected text it replaced, and the selection before it began.
    replaced: String,
    before: Selection,
    /// Formatting waiting at the caret when it began: the composed text
    /// gets it.
    pending: Option<clean::Pending>,
}

/// Versions come from one count for the whole process, so no two texts
/// share one: whatever is kept by version (heights, the note's end, what
/// spelling has seen) can never take one note's for another's, as it did
/// when every note opened at 0.
fn next_version() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn prev_grapheme(text: &str, at: usize) -> usize {
    let mut c = GraphemeCursor::new(at, text.len(), true);
    c.prev_boundary(text, 0).ok().flatten().unwrap_or(0)
}

fn next_grapheme(text: &str, at: usize) -> usize {
    let mut c = GraphemeCursor::new(at, text.len(), true);
    c.next_boundary(text, 0).ok().flatten().unwrap_or(text.len())
}

/// The nearest grapheme boundary at or before `at`.
fn snap(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let mut c = GraphemeCursor::new(at, text.len(), true);
    if c.is_boundary(text, 0).unwrap_or(true) { at } else { prev_grapheme(text, at) }
}

fn is_word(s: &str) -> bool {
    s.chars().any(char::is_alphanumeric)
}

/// Byte range of the logical line (paragraph) holding `at`, without its
/// newline.
fn line_range(text: &str, at: usize) -> (usize, usize) {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    (start, end)
}

/// Start of the word before `at`, skipping spaces and punctuation, but not
/// back past `floor`. A line break is a stop of its own.
fn prev_word(text: &str, at: usize, floor: usize) -> usize {
    let (start, _) = line_range(text, at);
    if at == start {
        return prev_grapheme(text, at);
    }
    let from = start.max(floor).min(at);
    text[from..at]
        .split_word_bound_indices()
        .rev()
        .find(|(_, piece)| is_word(piece))
        .map_or(from, |(i, _)| from + i)
}

/// End of the word after `at`, skipping spaces and punctuation.
fn next_word(text: &str, at: usize) -> usize {
    let (_, end) = line_range(text, at);
    if at == end {
        return next_grapheme(text, at);
    }
    text[at..end]
        .split_word_bound_indices()
        .find(|(_, piece)| is_word(piece))
        .map_or(end, |(i, piece)| at + i + piece.len())
}

impl Editor {
    pub fn new(text: impl Into<String>) -> Editor {
        let text = text.into();
        let (outline, states) = md::lines_with_states(&text).unzip();
        Editor {
            text,
            outline,
            states,
            selection: Selection::default(),
            history: History::default(),
            goal_x: None,
            version: next_version(),
            marked: None,
            snippets: Snippets::default(),
            clean: true,
            pending: None,
        }
    }

    /// A one-line field (search, find and replace, the palette): what is
    /// typed there is shown as it is, so it edits as text, not Markdown.
    pub fn field(text: impl Into<String>) -> Editor {
        let mut e = Editor::new(text);
        e.clean = false;
        e
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The lines of the text and their Markdown blocks.
    pub fn outline(&self) -> &[Line] {
        &self.outline
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// The line holding `at`.
    pub fn line_at(&self, at: usize) -> Line {
        let i = self.outline.partition_point(|l| l.start <= at).max(1) - 1;
        self.outline[i]
    }

    /// Moves `at` going backward to where the caret can rest: out of a
    /// block mark (to the end of the line before, or the line's text at the
    /// top of the note), back into a table's cell when it is between them,
    /// and in clean editing past fences, rules and hidden marks (see
    /// `rest`).
    fn before_mark(&self, at: usize) -> usize {
        self.rest(at, false, false)
    }

    /// Places the selection, snapping both ends to grapheme boundaries and
    /// the caret where it can rest (see `rest`): out of block marks and
    /// into table cells, and in clean editing never inside hidden marks.
    pub fn set_selection(&mut self, s: Selection) {
        self.place(s, false);
        self.drop_stale_pending();
    }

    /// A caret at `at`, kept after a span's closing marks when it is there:
    /// what follows a link just made is typed outside it.
    pub fn set_caret_after(&mut self, at: usize) {
        self.place(Selection::caret(at), true);
        self.drop_stale_pending();
    }

    /// `set_selection`, keeping the caret after a span's closing marks if
    /// it is there (`keep_side`, for edits that did not move it). A
    /// selection keeps its ends where they are put, as long as they are
    /// not inside marks: one that takes a span whole takes its marks too.
    fn place(&mut self, s: Selection, keep_side: bool) {
        self.settle_marked();
        let head = self.rest(snap(&self.text, s.head), true, keep_side || s.anchor != s.head);
        let anchor = if s.anchor == s.head { head } else { snap(&self.text, s.anchor) };
        self.selection = Selection { anchor, head };
        self.goal_x = None;
        self.history.seal();
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.start()..self.selection.end()]
    }

    /// The text changed everywhere (several edits at once).
    fn changed(&mut self) {
        (self.outline, self.states) = md::lines_with_states(&self.text).unzip();
        self.goal_x = None;
        self.version = next_version();
    }

    /// `removed` bytes at `at` became `inserted` bytes: only the lines
    /// around them are read again.
    fn changed_at(&mut self, at: usize, removed: usize, inserted: usize) {
        md::relex(&self.text, &mut self.outline, &mut self.states, at, removed, inserted);
        self.goal_x = None;
        self.version = next_version();
    }

    /// Replaces `at..at + len` with `inserted`, leaving the caret at `caret`.
    fn replace(&mut self, kind: Kind, at: usize, len: usize, inserted: &str, caret: usize) {
        if self.marked.is_some() {
            // Offsets were taken from the text with the composition in it,
            // which stays: settled first, they still hold.
            self.settle_marked();
        }
        if len == 0 && inserted.is_empty() {
            return;
        }
        let removed = self.text[at..at + len].to_string();
        let before = self.selection;
        self.text.replace_range(at..at + len, inserted);
        self.selection = Selection::caret(caret);
        self.changed_at(at, len, inserted.len());
        let after = self.selection;
        self.history.record(kind, Change { at, removed, inserted: inserted.to_string(), before, after });
    }

    /// Applies non-overlapping edits (offsets into the text before any of
    /// them) as one undo step. The selection follows the text around them,
    /// or becomes `select` (offsets after them).
    fn apply_edits(&mut self, mut edits: Vec<Edit>, select: Option<Selection>) {
        self.settle_marked();
        edits.retain(|e| e.len > 0 || !e.text.is_empty());
        if edits.is_empty() {
            return;
        }
        // Back to front, so earlier offsets stay valid.
        edits.sort_by_key(|e| std::cmp::Reverse(e.at));
        let before = self.selection;
        let map = |mut pos: usize| {
            for e in &edits {
                if pos >= e.at + e.len {
                    pos = pos - e.len + e.text.len();
                } else if pos > e.at {
                    pos = e.at + e.text.len();
                }
            }
            pos
        };
        let after = select.unwrap_or(Selection { anchor: map(before.anchor), head: map(before.head) });
        let mut changes = Vec::with_capacity(edits.len());
        for e in &edits {
            let removed = self.text[e.at..e.at + e.len].to_string();
            self.text.replace_range(e.at..e.at + e.len, &e.text);
            changes.push(Change { at: e.at, removed, inserted: e.text.clone(), before, after });
        }
        self.changed();
        self.history.record_step(Kind::Other, changes);
        self.selection = after;
        self.place(after, select.is_none());
        if let Some((_, changes)) = self.history.undo.last_mut() {
            changes.iter_mut().for_each(|c| c.after = self.selection);
        }
    }

    /// Types `s` over the selection. In a table's cell it stays on one line
    /// with its pipes escaped; over a selection across cells, only their
    /// text goes.
    pub fn insert(&mut self, s: &str) {
        if self.expand_snippet(s) {
            return;
        }
        let (start, end) = (self.selection.start(), self.selection.end());
        if start < end && self.crosses_cells(start, end) {
            return self.replace_across_cells(start, end, s);
        }
        let s = &*self.typed_at(start, s);
        // Bold (or the rest) chosen with nothing selected: the text typed
        // gets it, the caret before its closing marks.
        if let Some(p) = self.take_pending(start).filter(|_| !s.is_empty()) {
            let (before, after) = p.around();
            let caret = start + before.len() + s.len();
            return self.replace(Kind::Other, start, 0, &format!("{before}{s}{after}"), caret);
        }
        if start < end && self.clean {
            return self.remove_visible(start, end, Kind::Other, s, start);
        }
        let kind = if end > start || s.graphemes(true).count() > 1 { Kind::Other } else { Kind::Typing };
        self.replace(kind, start, end - start, s, start + s.len());
    }

    /// Whether Markdown pasted at the caret keeps its structure: not in a
    /// code block or front matter, nor in a table's cell (where pasted text
    /// goes on one line).
    pub fn takes_markdown(&self) -> bool {
        let at = self.selection.start();
        !matches!(self.line_at(at).block, Block::Fence | Block::Code | Block::Frontmatter) && self.cell_at(at).is_none()
    }

    /// Pastes `markdown` (copied text with its structure, see
    /// `takes_markdown`) over the selection, as one step. On an empty list
    /// item, task, quote or heading, pasted blocks that bring their own
    /// marks take its place rather than going inside it.
    pub fn paste_markdown(&mut self, markdown: &str) {
        let s = self.selection;
        let line = self.line_at(s.start());
        let first = markdown.lines().next().unwrap_or("");
        let marked = md::block(first).1 > 0 || first.trim_start().starts_with('|');
        if s.is_caret() && line.mark > 0 && line.content() == line.end && marked {
            self.set_selection(Selection { anchor: line.start, head: line.end });
        }
        self.insert(markdown);
    }

    /// Enter. In a list or quote, the next line continues it; on an empty
    /// item (or heading) the mark goes instead; at the start of an item's
    /// text, a new item opens above, keeping this one whole. In a table, a
    /// new row (see `table_newline`).
    pub fn newline(&mut self) {
        if self.table_newline() || self.leave_code() {
            return;
        }
        let (start, end) = (self.selection.start(), self.selection.end());
        let line = self.line_at(start);
        let content = line.content();
        if line.mark > 0 && start == end && start == content {
            if content == line.end {
                self.replace(Kind::Other, line.start, line.mark, "", line.start);
            } else {
                let mut above = if line.block.is_item() || line.block == Block::Quote {
                    self.text[line.start..content].replace("[x]", "[ ]").replace("[X]", "[ ]")
                } else {
                    String::new()
                };
                above.push('\n');
                let caret = start + above.len();
                self.replace(Kind::Other, line.start, 0, &above, caret);
            }
            return;
        }
        let next = md::continuation(&self.text[line.start..line.end], line.block, line.mark);
        match next {
            Some(next) if start >= content => {
                let inserted = format!("\n{next}");
                self.replace(Kind::Other, start, end - start, &inserted, start + inserted.len());
            }
            _ => self.insert("\n"),
        }
    }

    /// Backspace: deletes the selection, or the grapheme (or word) before
    /// the caret. At the start of an item's text it removes the item's mark
    /// (bullet, number, checkbox, heading), leaving the text. In a table it
    /// keeps to the cell (see `backspace_in_cell`).
    pub fn backspace(&mut self, word: bool) {
        let s = self.selection;
        if !s.is_caret() {
            return self.delete_selection();
        }
        if self.backspace_in_cell(s.head, word) {
            return;
        }
        let line = self.line_at(s.head);
        if self.hides_mark(&line) && s.head == line.content() {
            return self.replace(Kind::Other, line.start, line.mark, "", line.start);
        }
        if self.clean {
            return self.backspace_clean(s.head, word);
        }
        let floor = if self.hides_mark(&line) { line.content() } else { 0 };
        let from = if word { prev_word(&self.text, s.head, floor) } else { prev_grapheme(&self.text, s.head) };
        self.replace(Kind::Deleting, from, s.head - from, "", from);
    }

    /// Delete: deletes the selection, or the grapheme (or word) after the
    /// caret. At the end of a line it joins the next line's text, without
    /// that line's mark. In a table it keeps to the cell.
    pub fn delete(&mut self, word: bool) {
        let s = self.selection;
        if !s.is_caret() {
            return self.delete_selection();
        }
        if self.delete_in_cell(s.head, word) {
            return;
        }
        if self.clean {
            return self.delete_clean(s.head, word);
        }
        let line = self.line_at(s.head);
        let to = if s.head == line.end && line.end < self.text.len() {
            self.line_at(line.end + 1).content()
        } else if word {
            next_word(&self.text, s.head)
        } else {
            next_grapheme(&self.text, s.head)
        };
        self.replace(Kind::Deleting, s.head, to - s.head, "", s.head);
    }

    fn delete_selection(&mut self) {
        let (a, b) = (self.selection.start(), self.selection.end());
        if self.crosses_cells(a, b) {
            return self.replace_across_cells(a, b, "");
        }
        self.remove_visible(a, b, Kind::Other, "", a);
    }

    /// Where `motion` takes the caret; with `extend`, the selection grows
    /// instead. The layout may borrow this editor's text: apply the result
    /// with [`Editor::apply`] once it is dropped.
    pub fn plan(&self, motion: Motion, extend: bool, layout: &dyn Layout) -> Plan {
        let s = self.selection;
        let head = s.head;
        // Arrow keys move on screen: in a right-to-left paragraph, left
        // means forward through the text.
        let forward = matches!(motion, Motion::Right | Motion::WordRight) != layout.rtl(head);
        let target = match motion {
            // A plain arrow collapses the selection toward its side.
            Motion::Left | Motion::Right if !s.is_caret() && !extend => {
                if forward { s.end() } else { s.start() }
            }
            // A run of hidden marks steps as one.
            Motion::Left | Motion::Right => {
                if forward { self.rest(self.step_forward(self.past_runs(head)), true, false) } else { self.before_mark(prev_grapheme(&self.text, head)) }
            }
            Motion::WordLeft | Motion::WordRight => {
                if forward { next_word(&self.text, head) } else { self.before_mark(prev_word(&self.text, head, 0)) }
            }
            Motion::Up => return self.plan_lines(-1, extend, layout),
            Motion::Down => return self.plan_lines(1, extend, layout),
            Motion::LineStart => self.within_cell(head, layout.visual_line(head).0),
            Motion::LineEnd => self.within_cell(head, layout.visual_line(head).1),
            Motion::DocStart => 0,
            Motion::DocEnd => self.text.len(),
        };
        Plan { selection: Selection { anchor: if extend { s.anchor } else { target }, head: target }, goal_x: None }
    }

    /// Where moving `lines` visual lines down (up, if negative) takes the
    /// caret, keeping its horizontal position. Past the first or last line
    /// it goes to the start or end of the note.
    pub fn plan_lines(&self, lines: isize, extend: bool, layout: &dyn Layout) -> Plan {
        let s = self.selection;
        let x = self.goal_x.unwrap_or_else(|| layout.x_of(s.head));
        let mut head = s.head;
        for _ in 0..lines.unsigned_abs() {
            let next = if lines < 0 { layout.above(head, x) } else { layout.below(head, x) };
            match next {
                Some(at) => head = at,
                None => {
                    head = if lines < 0 { 0 } else { self.text.len() };
                    break;
                }
            }
        }
        if self.clean {
            // Over a fence or a rule, on in the direction it went.
            head = self.rest(head, lines > 0, false);
        }
        Plan { selection: Selection { anchor: if extend { s.anchor } else { head }, head }, goal_x: Some(x) }
    }

    pub fn apply(&mut self, plan: Plan) {
        self.set_selection(plan.selection);
        self.goal_x = plan.goal_x;
    }

    /// Moves the caret; with `extend`, the selection grows instead.
    pub fn move_caret(&mut self, motion: Motion, extend: bool, layout: &dyn Layout) {
        let plan = self.plan(motion, extend, layout);
        self.apply(plan);
    }

    /// All of the note: to its very end, even past a table's last pipe.
    pub fn select_all(&mut self) {
        self.set_selection(Selection::caret(0));
        self.selection = Selection { anchor: 0, head: self.text.len() };
    }

    /// Double-click: the word (or run of spaces, or punctuation mark) at
    /// `at`.
    pub fn select_word_at(&mut self, at: usize) {
        let at = snap(&self.text, at);
        let line = self.line_at(at);
        let from = line.content().min(at);
        let mut range = (at, at);
        for (i, piece) in self.text[from..line.end].split_word_bound_indices() {
            range = (from + i, from + i + piece.len());
            if at < range.1 {
                break;
            }
        }
        self.set_selection(Selection { anchor: range.0, head: range.1 });
    }

    /// Triple-click: the whole paragraph at `at`, its mark included; in a
    /// table, the cell's text.
    pub fn select_line_at(&mut self, at: usize) {
        let at = snap(&self.text, at);
        let (start, end) = self.cell_text_at(at).unwrap_or_else(|| line_range(&self.text, at));
        self.set_selection(Selection { anchor: start, head: end });
    }

    fn apply_change(&mut self, change: &Change, reverse: bool) {
        let (put, take) = if reverse { (&change.removed, &change.inserted) } else { (&change.inserted, &change.removed) };
        self.text.replace_range(change.at..change.at + take.len(), put);
        self.selection = if reverse { change.before } else { change.after };
        self.changed_at(change.at, take.len(), put.len());
    }

    pub fn undo(&mut self) -> bool {
        self.settle_marked();
        self.history.seal();
        let Some(step) = self.history.undo.pop() else { return false };
        for change in step.1.iter().rev() {
            self.apply_change(change, true);
        }
        self.history.redo.push(step);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.settle_marked();
        self.history.seal();
        let Some(step) = self.history.redo.pop() else { return false };
        for change in &step.1 {
            self.apply_change(change, false);
        }
        self.history.undo.push(step);
        true
    }

    /// The text being composed with an input method (see `set_marked`), as
    /// a byte range.
    pub fn marked(&self) -> Option<(usize, usize)> {
        self.marked.as_ref().map(|m| (m.start, m.start + m.len))
    }

    /// Shows `text` being composed where the caret is (over the selection,
    /// which comes back on cancel), the caret `cursor` bytes into it. Called
    /// again as the composition changes. Nothing is recorded, and no list or
    /// pair handling applies, until `commit_marked`.
    pub fn set_marked(&mut self, text: &str, cursor: usize) {
        let (start, len) = match &self.marked {
            Some(m) => (m.start, m.len),
            None => {
                let (start, end) = (self.selection.start(), self.selection.end());
                self.history.seal();
                let pending = if start == end { self.take_pending(start) } else { None };
                self.marked = Some(Marked { start, len: end - start, replaced: self.text[start..end].to_string(), before: self.selection, pending });
                (start, end - start)
            }
        };
        self.text.replace_range(start..start + len, text);
        if let Some(m) = &mut self.marked {
            m.len = text.len();
        }
        self.selection = Selection::caret(start + snap(text, cursor.min(text.len())));
        self.goal_x = None;
        self.changed_at(start, len, text.len());
    }

    /// The composition is done: `text`, what the input method settled on,
    /// takes its place as one typed step, which undo takes back to the text
    /// before composing. With no composition, `text` is simply typed (the
    /// emoji panel sends its picks this way).
    pub fn commit_marked(&mut self, text: &str) {
        let Some(m) = self.marked.take() else {
            if !text.is_empty() {
                self.insert(text);
            }
            return;
        };
        // Bold chosen before composing: the composed text is bold.
        let (open, close) = m.pending.as_ref().filter(|_| !text.is_empty()).map(clean::Pending::around).unwrap_or_default();
        let wrapped = format!("{open}{text}{close}");
        let text = wrapped.as_str();
        self.text.replace_range(m.start..m.start + m.len, text);
        let after = Selection::caret(m.start + text.len() - close.len());
        self.selection = after;
        self.goal_x = None;
        self.changed_at(m.start, m.len, text.len());
        if !m.replaced.is_empty() || !text.is_empty() {
            let kind = if m.replaced.is_empty() && text.graphemes(true).count() == 1 { Kind::Typing } else { Kind::Other };
            self.history.record(kind, Change { at: m.start, removed: m.replaced, inserted: text.to_string(), before: m.before, after });
        }
    }

    /// The composition was abandoned: what it replaced comes back, selected
    /// as it was.
    pub fn cancel_marked(&mut self) {
        let Some(m) = self.marked.take() else { return };
        self.text.replace_range(m.start..m.start + m.len, &m.replaced);
        self.selection = m.before;
        self.goal_x = None;
        self.changed_at(m.start, m.len, m.replaced.len());
    }

    /// Anything else done to the text mid-composition commits it first, as
    /// it stands: the note is never left half composed.
    fn settle_marked(&mut self) {
        if let Some(m) = &self.marked {
            let text = self.text[m.start..m.start + m.len].to_string();
            self.commit_marked(&text);
        }
    }

    /// Replaces byte ranges `(start, end, text)` (in offsets before any of
    /// them) as one undo step, the selection following the text around them.
    pub fn replace_ranges(&mut self, replacements: &[(usize, usize, String)]) {
        let edits = replacements.iter().map(|(s, e, t)| Edit { at: *s, len: e - s, text: t.clone() }).collect();
        self.apply_edits(edits, None);
    }

    /// Rewrites byte ranges `(start, end, text)` of the current text
    /// (non-overlapping, any order) as one undo step of its own that never
    /// merges with typing. The selection keeps its place in the text around
    /// the ranges; one inside a replaced range moves to that range's end.
    /// Only the touched lines are read again, range by range from the back,
    /// not the whole note. Returns the replaced ranges in offsets after the
    /// edit, in text order; edits that change nothing are left out.
    ///
    /// A range out of bounds, off a char boundary or overlapping another is
    /// the caller's bug: nothing changes and the result is empty.
    pub fn amend(&mut self, edits: &[(usize, usize, String)]) -> Vec<(usize, usize)> {
        let text = &self.text;
        let valid = edits.iter().all(|&(s, e, _)| s <= e && e <= text.len() && text.is_char_boundary(s) && text.is_char_boundary(e));
        debug_assert!(valid, "amend ranges must be in bounds and on char boundaries");
        if !valid {
            return Vec::new();
        }
        let mut edits: Vec<&(usize, usize, String)> = edits.iter().filter(|(s, e, t)| s < e || !t.is_empty()).collect();
        // Stable: insertions at one place keep the order they came in.
        edits.sort_by_key(|&&(s, e, _)| (s, e));
        if edits.windows(2).any(|w| w[1].0 < w[0].1) {
            return Vec::new();
        }
        edits.retain(|(s, e, t)| text[*s..*e] != **t);
        if edits.is_empty() {
            return Vec::new();
        }
        let mut placed = Vec::with_capacity(edits.len());
        let (mut old_end, mut new_end) = (0, 0);
        for &&(s, e, ref t) in &edits {
            let at = new_end + (s - old_end);
            (old_end, new_end) = (e, at + t.len());
            placed.push((at, new_end));
        }
        let map = |pos: usize| {
            let mut out = pos;
            for (&&(s, e, _), &(_, end)) in edits.iter().zip(&placed) {
                if pos >= e {
                    out = end + (pos - e);
                } else if pos > s {
                    return end;
                } else {
                    break;
                }
            }
            out
        };
        let before = self.selection;
        let mapped = Selection { anchor: map(before.anchor), head: map(before.head) };
        let mut changes = Vec::with_capacity(edits.len());
        // Back to front, so each range's offsets hold when it is replaced,
        // and when undo and redo replay the changes.
        for &&(s, e, ref t) in edits.iter().rev() {
            let removed = self.text[s..e].to_string();
            self.text.replace_range(s..e, t);
            self.changed_at(s, e - s, t.len());
            changes.push(Change { at: s, removed, inserted: t.clone(), before, after: mapped });
        }
        // Out of any block mark the new text puts under the caret, kept on
        // its side of any span's marks.
        self.place(mapped, true);
        let after = self.selection;
        changes.iter_mut().for_each(|c| c.after = after);
        self.history.record_step(Kind::Amend, changes);
        placed
    }

    /// After [`Editor::undo`]: if the step just undone was an amend, the
    /// range of text it put back, in offsets now (from its first range's
    /// start to the end of its last). None otherwise, and after any new
    /// edit or redo, since those change the redo stack's top.
    pub fn undone_amend(&self) -> Option<(usize, usize)> {
        let (kind, changes) = self.history.redo.last()?;
        if *kind != Kind::Amend {
            return None;
        }
        let start = changes.iter().map(|c| c.at).min()?;
        let end = changes.iter().map(|c| c.at + c.removed.len()).max()?;
        Some((start, end))
    }

    /// The note's title: see [`title_of`].
    pub fn title(&self) -> String {
        title_of(&self.text)
    }
}

/// A note's title: its first non-empty line after any frontmatter, as it
/// reads, without Markdown marks, at most 80 graphemes.
pub fn title_of(text: &str) -> String {
    let line = text[title::start(text)..].lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let plain = md::plain(line);
    plain.trim().graphemes(true).take(80).collect::<String>().trim_end().to_string()
}

#[cfg(test)]
mod tests;
