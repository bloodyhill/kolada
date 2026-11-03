//! Moves on whole lines: moving the selected lines up or down past their
//! neighbour, duplicating them, and sorting them. A list item goes with the
//! items nested under it, and steps over a sibling's nested items whole, so
//! a list keeps its shape. Each is one undo step.

use std::cmp::Ordering;

use kolada_markdown::Block;

use super::*;

impl Editor {
    /// The lines the selection covers, by index: a selection ending at a
    /// line's very start leaves that line out.
    fn span(&self) -> (usize, usize) {
        let (a, b) = (self.selection.start(), self.selection.end());
        let first = self.line_index(a);
        let mut last = self.line_index(b);
        if last > first && self.outline[last].start == b {
            last -= 1;
        }
        (first, last)
    }

    /// The last line of the item at `i` with the items nested under it.
    fn subtree_end(&self, i: usize) -> usize {
        let line = self.outline[i];
        if !line.block.is_item() {
            return i;
        }
        let mut j = i;
        while j + 1 < self.outline.len() && self.outline[j + 1].block.is_item() && self.outline[j + 1].depth > line.depth {
            j += 1;
        }
        j
    }

    fn lines_text(&self, first: usize, last: usize) -> &str {
        &self.text[self.outline[first].start..self.outline[last].end]
    }

    /// Alt+Up and Alt+Down: the selected lines trade places with the line
    /// above or below (an item with its nested items, past a sibling with
    /// its own), the selection going with them. False at the top or bottom.
    pub fn move_lines(&mut self, up: bool) -> bool {
        self.single_caret();
        let (first, last) = self.span();
        let last = self.subtree_end(last);
        let item = self.outline[first].block.is_item();
        let depth = self.outline[first].depth;
        // The neighbour's lines: the sibling item with its nested items, or
        // the one line.
        let (top, bottom) = if up {
            if first == 0 {
                return false;
            }
            let mut k = first - 1;
            while k > 0 && item && self.outline[k].block.is_item() && self.outline[k].depth > depth {
                k -= 1;
            }
            let sibling = item && self.outline[k].block.is_item() && self.outline[k].depth == depth;
            (if sibling { k } else { first - 1 }, first - 1)
        } else {
            if last + 1 >= self.outline.len() {
                return false;
            }
            let next = last + 1;
            let sibling = item && self.outline[next].block.is_item() && self.outline[next].depth == depth;
            (next, if sibling { self.subtree_end(next) } else { next })
        };
        let block = self.lines_text(first, last).to_string();
        let other = self.lines_text(top, bottom).to_string();
        let start = self.outline[first.min(top)].start;
        let end = self.outline[last.max(bottom)].end;
        let (text, shift) = if up {
            (format!("{block}\n{other}"), -((other.len() + 1) as isize))
        } else {
            (format!("{other}\n{block}"), (other.len() + 1) as isize)
        };
        let s = self.selection;
        let moved = |at: usize| (at as isize + shift) as usize;
        let select = Selection { anchor: moved(s.anchor), head: moved(s.head) };
        self.apply_edits(vec![Edit { at: start, len: end - start, text }], Some(select));
        true
    }

    /// Shift+Alt+Down and Shift+Alt+Up: the selected lines again, below
    /// them (the selection going to the copy) or above (staying put).
    pub fn duplicate_lines(&mut self, up: bool) {
        self.single_caret();
        let (first, last) = self.span();
        let block = self.lines_text(first, last).to_string();
        let s = self.selection;
        if up {
            self.apply_edits(vec![Edit::insert(self.outline[first].start, format!("{block}\n"))], Some(s));
        } else {
            let shift = block.len() + 1;
            let select = Selection { anchor: s.anchor + shift, head: s.head + shift };
            self.apply_edits(vec![Edit::insert(self.outline[last].end, format!("\n{block}"))], Some(select));
        }
    }

    /// Sorts the selected lines by their text as it reads (marks aside,
    /// letter case aside, numbers by value), or with no lines selected the
    /// run of lines around the caret up to a blank one. An item goes with
    /// its nested items; a numbered list is numbered afresh. The lines stay
    /// selected. False if they were in order already.
    pub fn sort_lines(&mut self) -> bool {
        self.single_caret();
        let (mut first, mut last) = self.span();
        let blank = |l: &Line| self.text[l.start..l.end].trim().is_empty();
        if first == last {
            while first > 0 && !blank(&self.outline[first - 1]) {
                first -= 1;
            }
            while last + 1 < self.outline.len() && !blank(&self.outline[last + 1]) {
                last += 1;
            }
        }
        if first == last {
            return false;
        }
        // Units: a line, with the items nested under it.
        let depth = self.outline[first..=last].iter().filter(|l| l.block.is_item()).map(|l| l.depth).min().unwrap_or(0);
        let mut units: Vec<(usize, usize)> = Vec::new();
        for i in first..=last {
            let l = self.outline[i];
            match units.last_mut() {
                Some(u) if l.block.is_item() && l.depth > depth && self.outline[u.0].block.is_item() => u.1 = i,
                _ => units.push((i, i)),
            }
        }
        let key = |u: &(usize, usize)| {
            let l = self.outline[u.0];
            md::plain(&self.text[l.content()..l.end]).trim().to_lowercase()
        };
        let mut sorted = units.clone();
        sorted.sort_by(|a, b| natural(&key(a), &key(b)));
        if sorted == units {
            return false;
        }
        // A numbered list counts on from where it started.
        let numbered = units.iter().all(|u| self.outline[u.0].block == Block::Ordered);
        let mut number = if numbered { self.number(units[0].0).unwrap_or(1) } else { 0 };
        let pieces: Vec<String> = sorted
            .iter()
            .map(|&(a, b)| {
                let text = self.lines_text(a, b);
                if !numbered {
                    return text.to_string();
                }
                let renumbered = self.renumber(a, number);
                number += 1;
                renumbered + &text[self.outline[a].end - self.outline[a].start..]
            })
            .collect();
        let start = self.outline[first].start;
        let end = self.outline[last].end;
        let text = pieces.join("\n");
        let select = Selection { anchor: start, head: start + text.len() };
        self.apply_edits(vec![Edit { at: start, len: end - start, text }], Some(select));
        true
    }

    /// The number of the numbered item at line `i`.
    fn number(&self, i: usize) -> Option<usize> {
        let l = self.outline[i];
        let mark = self.text[l.start..l.content()].trim_start();
        mark[..mark.find(|c: char| !c.is_ascii_digit())?].parse().ok()
    }

    /// Line `i` with its number made `n`.
    fn renumber(&self, i: usize, n: usize) -> String {
        let l = self.outline[i];
        let line = &self.text[l.start..l.end];
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        let digits = line[indent..].find(|c: char| !c.is_ascii_digit()).unwrap_or(0);
        format!("{}{n}{}", &line[..indent], &line[indent + digits..])
    }
}

/// Text order with runs of digits compared by value: "item 2" before
/// "item 10".
fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.next_if(char::is_ascii_digit) {
                        s.push(c);
                    }
                    s
                };
                let (x, y) = (run(&mut a), run(&mut b));
                let (tx, ty) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}
