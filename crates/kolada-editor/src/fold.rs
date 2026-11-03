//! Folding under headings: a heading's section (the lines after it, up to
//! the next heading of its level or higher) can be put away and brought
//! back. Folds are the view's, never the file's: the text keeps every line,
//! the caret steps over a folded section, and an edit that reaches into one
//! (a delete joining it to its heading, a new line typed into it) opens it.

use kolada_markdown::Block;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fold {
    /// Where the heading's line starts.
    head: usize,
    /// The text put away: from the heading's line break to the end of the
    /// section's last line.
    from: usize,
    to: usize,
}

impl Editor {
    /// The section under the heading on the line holding `at`: the text it
    /// would put away, from the heading's line break to the end of its last
    /// line. None off a heading, or under one with nothing after it.
    pub fn section(&self, at: usize) -> Option<(usize, usize)> {
        let i = self.outline.partition_point(|l| l.start <= at).max(1) - 1;
        let Block::Heading(level) = self.outline[i].block else { return None };
        let next = self.outline[i + 1..].iter().position(|l| matches!(l.block, Block::Heading(n) if n <= level));
        let last = next.map_or(self.outline.len() - 1, |n| i + n);
        // Blank lines before the next heading stay out, so it keeps its
        // space above.
        let last = (i + 1..=last).rev().find(|&j| !self.text[self.outline[j].start..self.outline[j].end].trim().is_empty())?;
        Some((self.outline[i].end, self.outline[last].end))
    }

    /// The heading whose section holds `at`, or the heading on its line: its
    /// line's start.
    fn heading_over(&self, at: usize) -> Option<usize> {
        let i = self.outline.partition_point(|l| l.start <= at).max(1) - 1;
        self.outline[..=i]
            .iter()
            .rev()
            .filter(|l| matches!(l.block, Block::Heading(_)))
            .find(|l| self.section(l.start).is_some_and(|(_, to)| l.start == self.outline[i].start || at <= to))
            .map(|l| l.start)
    }

    /// Ctrl+Shift+[: folds the section the caret is in (or under the
    /// heading it is on); the caret goes to the heading's end if it was
    /// inside. False if there is nothing to fold.
    pub fn fold(&mut self, at: usize) -> bool {
        let Some(head) = self.heading_over(at) else { return false };
        let Some((from, to)) = self.section(head) else { return false };
        if self.folds.iter().any(|f| f.head == head) {
            return false;
        }
        self.folds.push(Fold { head, from, to });
        self.folds.sort_by_key(|f| f.head);
        self.folds_changed();
        let s = self.selection;
        if (from < s.head && s.head <= to) || (from < s.anchor && s.anchor <= to) {
            self.set_selection(Selection::caret(from));
        }
        true
    }

    /// Ctrl+Shift+]: opens the folded section the caret's heading has (or
    /// the one the caret is in). False if none is folded there.
    pub fn unfold(&mut self, at: usize) -> bool {
        let line = self.line_at(at);
        let before = self.folds.len();
        self.folds.retain(|f| f.head != line.start && !(f.from < at && at <= f.to));
        self.folds_changed();
        self.folds.len() != before
    }

    /// A click on a heading's fold mark: folds it, or opens it.
    pub fn toggle_fold(&mut self, at: usize) -> bool {
        let head = self.line_at(at).start;
        if self.folds.iter().any(|f| f.head == head) { self.unfold(head) } else { self.fold(head) }
    }

    /// The text put away, as `(from, to)` byte ranges in text order.
    pub fn folded(&self) -> &[(usize, usize)] {
        &self.fold_ranges
    }

    /// The folds changed: their ranges as the view reads them.
    fn folds_changed(&mut self) {
        self.fold_ranges = self.folds.iter().map(|f| (f.from, f.to)).collect();
    }

    /// Whether the heading starting its line at `head` is folded.
    pub fn is_folded(&self, head: usize) -> bool {
        self.folds.iter().any(|f| f.head == head)
    }

    /// The folded headings, by their lines' starts: to keep with the note.
    pub fn fold_heads(&self) -> Vec<usize> {
        self.folds.iter().map(|f| f.head).collect()
    }

    /// Folds the headings at `heads` (as `fold_heads` gave them) that are
    /// still headings with a section.
    pub fn set_folds(&mut self, heads: &[usize]) {
        self.folds.clear();
        self.folds_changed();
        for &head in heads {
            if head <= self.text.len() && self.line_at(head).start == head {
                self.fold(head);
            }
        }
    }

    /// Where the caret goes from `at` inside a folded section: on past it,
    /// or back to its heading's end. None outside one.
    pub(crate) fn out_of_fold(&self, at: usize, forward: bool) -> Option<usize> {
        let f = self.folds.iter().find(|f| f.from < at && at <= f.to)?;
        Some(if forward && f.to < self.text.len() { f.to + 1 } else { f.from })
    }

    /// `removed` bytes at `at` became `inserted` (the text holds them
    /// already, the outline not yet): a fold the edit reaches into opens,
    /// and the headings after it move with the text.
    pub(crate) fn touch_folds(&mut self, at: usize, removed: usize, inserted: usize) {
        if self.folds.is_empty() {
            return;
        }
        let breaks = self.text[at..at + inserted].contains('\n');
        let delta = inserted as isize - removed as isize;
        self.folds.retain_mut(|f| {
            let into = if removed > 0 { at < f.to && at + removed > f.from } else { (f.from < at && at <= f.to) || (at == f.from && breaks) };
            if into || (at < f.head && f.head < at + removed) {
                return false;
            }
            if f.head > at || (f.head == at && removed == 0 && breaks) {
                f.head = (f.head as isize + delta) as usize;
            }
            true
        });
        self.folds_changed();
    }

    /// The outline is current: each fold's section is measured again, and a
    /// fold that is no longer under a heading with a section goes.
    pub(crate) fn check_folds(&mut self) {
        if self.folds.is_empty() {
            return;
        }
        let folds = std::mem::take(&mut self.folds);
        self.folds = folds
            .into_iter()
            .filter_map(|f| {
                (f.head <= self.text.len() && self.line_at(f.head).start == f.head).then_some(())?;
                let (from, to) = self.section(f.head)?;
                Some(Fold { head: f.head, from, to })
            })
            .collect();
        self.folds_changed();
    }
}
