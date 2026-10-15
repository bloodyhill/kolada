//! Clean editing: the note reads with no Markdown showing (the view hides
//! every mark, see `kolada_markdown::hidden`), so editing keeps to what
//! shows. The caret never rests inside hidden marks, and steps over a run
//! of them as one; where a run sits between letters it takes the left end,
//! so after a bold word it is still in the bold and typing goes on bold,
//! and before one it is outside. Deleting takes only text that shows and
//! keeps each span's marks balanced. Bold and the rest at a bare caret wait
//! for what is typed next. Fences and rules are drawn as a panel's edge and
//! a line: the caret steps over their lines.
//!
//! The text stays plain Markdown: none of this writes anything a note did
//! not have. With the options' Show Markdown on, `clean` is off and editing
//! is as it was, marks showing at the caret.

use kolada_markdown::{self as md, Block, Line};

use crate::{Edit, Editor, Inline, Kind, Selection, next_grapheme, next_word, prev_grapheme, prev_word};

/// Formatting chosen with nothing selected, waiting for the text typed
/// next at `at` (in the text of `version`): the marks of a span the caret
/// is inside, closed before that text and opened again after it (the
/// formatting turned off there), and formatting turned on, in the order
/// chosen.
#[derive(Clone, Debug)]
pub(crate) struct Pending {
    at: usize,
    version: u64,
    off: Option<(String, String)>,
    on: Vec<Inline>,
}

impl Pending {
    /// What goes before the typed text, and after it.
    pub(crate) fn around(&self) -> (String, String) {
        let mut before = self.off.as_ref().map_or_else(String::new, |(close, _)| close.clone());
        let mut after = String::new();
        for s in &self.on {
            before.push_str(s.mark());
        }
        for s in self.on.iter().rev() {
            after.push_str(s.mark());
        }
        if let Some((_, open)) = &self.off {
            after.push_str(open);
        }
        (before, after)
    }
}

/// A line drawn as something else, whose text the caret never enters: a
/// code fence (the edge of its panel) or a rule (a line across).
fn is_bar(line: &Line) -> bool {
    matches!(line.block, Block::Fence | Block::Rule)
}

impl Editor {
    /// Clean editing on (the default) or off (the options' Show Markdown).
    pub fn set_clean(&mut self, on: bool) {
        self.clean = on;
        self.pending = None;
    }

    pub fn is_clean(&self) -> bool {
        self.clean
    }

    /// Formatting waiting at the caret for the text typed next, turned on
    /// (see `toggle_inline`).
    pub fn pending_styles(&self) -> Vec<Inline> {
        match &self.pending {
            Some(p) if self.pending_at(p.at) => p.on.clone(),
            _ => Vec::new(),
        }
    }

    /// Formatting waiting at the caret is let go once the caret leaves.
    pub(crate) fn drop_stale_pending(&mut self) {
        if self.pending.as_ref().is_some_and(|p| !self.pending_at(p.at)) {
            self.pending = None;
        }
    }

    fn pending_at(&self, at: usize) -> bool {
        self.pending.as_ref().is_some_and(|p| p.at == at && p.version == self.version) && self.selection.is_caret() && self.selection.head == at
    }

    /// The formatting waiting at `at`, if any, taken to be typed.
    pub(crate) fn take_pending(&mut self, at: usize) -> Option<Pending> {
        if self.pending_at(at) { self.pending.take() } else { None }
    }

    /// Whether a line's block mark is drawn as something else (a bullet, a
    /// number, a checkbox, a quote's bar, a heading's size) and so steps as
    /// one. With Show Markdown, a heading's `#`s show at the caret and are
    /// plain text to it.
    pub(crate) fn hides_mark(&self, line: &Line) -> bool {
        line.mark > 0 && (self.clean || !matches!(line.block, Block::Heading(_)))
    }

    /// The runs of hidden inline marks on `line` (merged where they touch),
    /// in clean editing; none otherwise.
    fn runs(&self, line: &Line) -> Vec<(usize, usize)> {
        if !self.clean || is_bar(line) || self.is_picture(line) {
            return Vec::new();
        }
        let mut runs = md::hidden(&self.text, line);
        if line.mark > 0 && !runs.is_empty() {
            // The block mark comes first, on its own.
            runs.remove(0);
        }
        runs
    }

    /// A line that is only a picture (`![[photo.png]]`, `![](photo.png)`):
    /// the shell chooses it whole, and places the caret around it itself.
    fn is_picture(&self, line: &Line) -> bool {
        let content = &self.text[line.content()..line.end];
        content.starts_with('!') && md::wrapped(&self.text, line).first().is_some_and(|w| w.start == line.content() && w.end == line.end)
    }

    /// Where the caret can rest nearest `at`, looking `forward` (or back)
    /// first: out of a block mark (forward to the line's text, back to the
    /// end of the line before), into a table's cell, past fence and rule
    /// lines (only the note's last line may hold it, at its end), and in
    /// clean editing at the left end of a run of hidden marks it touches.
    /// With `keep_side`, only out of a run it is strictly inside: a caret
    /// left after a span's closing marks stays there (the edit around it
    /// did not move it).
    pub(crate) fn rest(&self, at: usize, forward: bool, keep_side: bool) -> usize {
        let mut at = at.min(self.text.len());
        let mut forward = forward;
        for _ in 0..self.outline.len() + 8 {
            let line = self.line_at(at);
            if self.clean && is_bar(&line) {
                if forward {
                    if line.end >= self.text.len() {
                        return line.end;
                    }
                    at = line.end + 1;
                } else if line.start > 0 {
                    at = line.start - 1;
                } else {
                    forward = true;
                }
                continue;
            }
            if self.hides_mark(&line) && at < line.content() {
                if forward || line.start == 0 {
                    return line.content();
                }
                at = line.start - 1;
                continue;
            }
            let cell = self.to_cell(at, forward);
            if cell != at {
                at = cell;
                continue;
            }
            for (a, b) in self.runs(&line) {
                if (a < at && at < b) || (!keep_side && a <= at && at <= b) {
                    return a;
                }
            }
            return at;
        }
        at
    }

    /// Past the run of hidden marks starting at `at`, if one does: where
    /// the next letter shown starts.
    pub(crate) fn past_runs(&self, at: usize) -> usize {
        let line = self.line_at(at);
        self.runs(&line).into_iter().find(|&(a, _)| a == at).map_or(at, |(_, b)| b)
    }

    /// Before the run of hidden marks ending at `at`, if one does: where
    /// the letter shown before it ends.
    fn before_runs(&self, at: usize, line: &Line) -> usize {
        self.runs(line).into_iter().find(|&(a, b)| a < b && b == at).map_or(at, |(a, _)| a)
    }

    /// The ranges deleting `a..b` takes, as it reads: the text shown in it,
    /// and spans whose text all goes with their marks; the marks of spans
    /// only partly in it stay, so what is left stays balanced. Block marks
    /// and whole lines go as they are. In order, not touching.
    pub(crate) fn visible_cut(&self, a: usize, b: usize) -> Vec<(usize, usize)> {
        if !self.clean || a >= b {
            return vec![(a, b)];
        }
        let mut cut = vec![(a, b)];
        let mut keep = Vec::new();
        let (first, last) = (self.line_index(a), self.line_index(b));
        for line in &self.outline[first..=last] {
            for w in md::wrapped(&self.text, line) {
                if w.text_start < w.text_end && a <= w.text_start && w.text_end <= b {
                    cut.push((w.start, w.end));
                } else {
                    keep.extend(w.marks().into_iter().filter(|&(x, y)| x < y && x < b && y > a));
                }
            }
        }
        cut.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(cut.len());
        for (x, y) in cut {
            match merged.last_mut() {
                Some(last) if x <= last.1 => last.1 = last.1.max(y),
                _ => merged.push((x, y)),
            }
        }
        for (x, y) in keep {
            merged = merged
                .into_iter()
                .flat_map(|(c, d)| {
                    if y <= c || d <= x {
                        vec![(c, d)]
                    } else {
                        [(c, x.max(c)), (y.min(d), d)].into_iter().filter(|&(p, q)| p < q).collect()
                    }
                })
                .collect();
        }
        merged
    }

    /// Deletes `a..b` as it reads (see `visible_cut`), with `typed` in its
    /// place: one step of `kind` when it is a plain range, else one undo
    /// step. The caret goes after what is typed, or with nothing typed to
    /// `caret` if that is before what goes.
    pub(crate) fn remove_visible(&mut self, a: usize, b: usize, kind: Kind, typed: &str, caret: usize) {
        let cut = self.visible_cut(a, b);
        let Some(&(first, _)) = cut.first() else {
            if !typed.is_empty() {
                self.replace(kind, a, 0, typed, a + typed.len());
            }
            return;
        };
        let caret = if typed.is_empty() { caret.min(first) } else { first + typed.len() };
        if cut == [(a, b)] {
            return self.replace(kind, a, b - a, typed, caret);
        }
        let edits = cut.iter().enumerate().map(|(i, &(x, y))| Edit { at: x, len: y - x, text: if i == 0 { typed.to_string() } else { String::new() } }).collect();
        self.apply_edits(edits, Some(Selection::caret(caret)));
    }

    /// Backspace at a caret in clean editing: the letter shown before it
    /// (or the word), stepping over hidden marks; at a line's start, a rule
    /// above goes, and code keeps its fences.
    pub(crate) fn backspace_clean(&mut self, head: usize, word: bool) {
        let line = self.line_at(head);
        let end = self.before_runs(head, &line);
        if self.hides_mark(&line) && end == line.content() {
            return self.replace(Kind::Other, line.start, line.mark, "", line.start);
        }
        if end == line.start && line.start > 0 {
            let above = self.line_at(line.start - 1);
            match above.block {
                // A rule above goes, as a picture does.
                Block::Rule => return self.replace(Kind::Other, above.start, line.start - above.start, "", above.start),
                Block::Fence if line.block == Block::Code => return self.unfence(line),
                // Below a code block: an empty line goes; from text, the
                // caret goes up into the code.
                Block::Fence if line.start == line.end => {
                    let to = self.rest(line.start - 1, false, false);
                    self.apply_edits(vec![Edit::remove(line.start - 1, 1)], Some(Selection::caret(to)));
                    return;
                }
                Block::Fence => {
                    let at = self.rest(line.start - 1, false, false);
                    self.set_selection(Selection::caret(at));
                    return;
                }
                _ => {}
            }
        }
        let floor = if self.hides_mark(&line) { line.content() } else { 0 };
        let from = if word { prev_word(&self.text, end, floor) } else { prev_grapheme(&self.text, end) };
        self.remove_visible(from, end, Kind::Deleting, "", from);
    }

    /// Backspace at the start of a code block's first line: an empty block
    /// goes, fences and all; one with code in it stays as it is.
    fn unfence(&mut self, first: Line) {
        let i = self.line_index(first.start);
        let open = self.outline[i - 1];
        let mut j = i;
        while self.outline.get(j).is_some_and(|l| l.block == Block::Code) {
            j += 1;
        }
        let Some(close) = self.outline.get(j).copied().filter(|l| l.block == Block::Fence) else { return };
        if self.outline[i..j].iter().any(|l| l.start < l.end) {
            return;
        }
        let (from, to) = if close.end < self.text.len() {
            (open.start, close.end + 1)
        } else if open.start > 0 {
            (open.start - 1, close.end)
        } else {
            (open.start, close.end)
        };
        self.apply_edits(vec![Edit::remove(from, to - from)], Some(Selection::caret(from)));
    }

    /// Delete at a caret in clean editing: the letter shown after it (or
    /// the word), stepping over hidden marks; at a line's end, a rule below
    /// goes, and a fence below stays.
    pub(crate) fn delete_clean(&mut self, head: usize, word: bool) {
        let line = self.line_at(head);
        let start = self.past_runs(head);
        if start >= line.end {
            if line.end >= self.text.len() {
                return;
            }
            let next = self.line_at(line.end + 1);
            return match next.block {
                Block::Rule => self.replace(Kind::Other, line.end, next.end - line.end, "", head),
                Block::Fence => {}
                _ => self.replace(Kind::Deleting, line.end, next.content() - line.end, "", head.min(line.end)),
            };
        }
        let to = if word { next_word(&self.text, start) } else { next_grapheme(&self.text, start) };
        self.remove_visible(start, to, Kind::Deleting, "", head);
    }

    /// Enter on the empty last line of a code block leaves the block: the
    /// line goes, and a new one opens after the closing fence. False
    /// elsewhere.
    pub(crate) fn leave_code(&mut self) -> bool {
        let at = self.selection.head;
        if !self.clean || !self.selection.is_caret() {
            return false;
        }
        let line = self.line_at(at);
        if line.block != Block::Code || line.start != line.end || line.start == 0 {
            return false;
        }
        let i = self.line_index(at);
        let Some(close) = self.outline.get(i + 1).copied().filter(|l| l.block == Block::Fence) else { return false };
        self.apply_edits(vec![Edit::remove(line.start - 1, 1), Edit::insert(close.end, "\n")], Some(Selection::caret(close.end)));
        true
    }

    /// Bold and the rest with nothing selected, in clean editing: no empty
    /// pair of marks is written. At the end of a span of the style, the
    /// caret goes past its closing marks and what is typed is plain; at its
    /// start, inside its opening marks. Inside one, the text typed next is
    /// left out of it; elsewhere, the text typed next gets the style.
    pub(crate) fn pend(&mut self, style: Inline) {
        let at = self.selection.head;
        let line = self.line_at(at);
        if matches!(line.block, Block::Fence | Block::Code | Block::Frontmatter) {
            return;
        }
        if self.pending_at(at)
            && let Some(p) = &self.pending
        {
            let mut p = p.clone();
            match p.on.iter().position(|&s| s == style) {
                Some(i) => {
                    p.on.remove(i);
                }
                None => p.on.push(style),
            }
            if p.on.is_empty() && p.off.is_none() {
                self.pending = None;
            } else if self.reads_at(&p) {
                self.pending = Some(p);
            }
            return;
        }
        let spans: Vec<md::Span> = md::line_spans(&self.text, &line).into_iter().filter(|s| s.style == style.style()).collect();
        if let Some(s) = spans.iter().rev().find(|s| s.text_end == at) {
            return self.jump(s.end);
        }
        if let Some(s) = spans.iter().find(|s| s.start == at) {
            return self.jump(s.text_start);
        }
        if let Some(s) = spans.iter().find(|s| s.text_start == at) {
            return self.jump(s.start);
        }
        let version = self.version;
        let p = match spans.iter().rev().find(|s| s.text_start < at && at < s.text_end) {
            Some(s) => Pending { at, version, off: Some((self.text[s.text_end..s.end].to_string(), self.text[s.start..s.text_start].to_string())), on: Vec::new() },
            None => Pending { at, version, off: None, on: vec![style] },
        };
        if self.reads_at(&p) {
            self.pending = Some(p);
        }
    }

    /// Whether the marks `p` would write at its place read as marks: none
    /// runs into a mark of the same character beside it (`**bold*x***`
    /// reads as no formatting at all). Where they would not, the style is
    /// not taken: better nothing than marks showing.
    fn reads_at(&self, p: &Pending) -> bool {
        let mut before: Vec<&str> = p.off.iter().map(|(close, _)| close.as_str()).collect();
        before.extend(p.on.iter().map(|s| s.mark()));
        let mut after: Vec<&str> = p.on.iter().rev().map(|s| s.mark()).collect();
        after.extend(p.off.iter().map(|(_, open)| open.as_str()));
        let b = self.text.as_bytes();
        let left = p.at.checked_sub(1).map(|i| b[i]);
        let right = b.get(p.at).copied();
        let touch = |pieces: &[&str], outside: Option<u8>, first: bool| {
            let edge = if first { pieces.first().and_then(|m| m.bytes().next()) } else { pieces.last().and_then(|m| m.bytes().last()) };
            // Bold and italic together are `***`, which reads as both.
            let joins = |w: &[&str]| w[0].bytes().last() == w[1].bytes().next() && !matches!((w[0], w[1]), ("**", "*") | ("*", "**"));
            (edge.is_some() && edge == outside) || pieces.windows(2).any(joins)
        };
        !touch(&before, left, true) && !touch(&after, right, false)
    }

    /// The caret to `to`, as it is: not drawn to the left of the marks it
    /// stands after, so what is typed there is outside them.
    fn jump(&mut self, to: usize) {
        self.selection = Selection::caret(to);
        self.goal_x = None;
        self.pending = None;
        self.history.seal();
    }
}
