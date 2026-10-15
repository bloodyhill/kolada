//! The selection as a piece of Markdown that stands on its own, for Copy:
//! a bold word cut in half is still bold, a link cut in half still links,
//! and lines taken from the middle of a list or a code block keep being
//! one. Copy puts it on the clipboard three ways (see [`Copied`]).

use kolada_markdown::{self as md, Block, Span, Style, html, plain};

use crate::Editor;

/// What Copy puts on the clipboard: the selection as Markdown (for Kolada,
/// which pastes it back exactly), as plain text (for apps that take only
/// text), and as HTML (for those that keep formatting).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copied {
    pub markdown: String,
    pub plain: String,
    pub html: String,
}

/// `text[x..y]`, its spans (nested, in order) closed where the range cuts
/// through them: a span's marks come with any of its text.
fn piece(text: &str, spans: &[Span], x: usize, y: usize, out: &mut String) {
    let mut at = x;
    let mut i = 0;
    while i < spans.len() {
        let sp = spans[i];
        // The spans inside this one follow it (spans nest, never cross).
        let mut j = i + 1;
        while j < spans.len() && spans[j].start < sp.end {
            j += 1;
        }
        let inside = &spans[i + 1..j];
        i = j;
        if sp.end <= at || sp.start >= y {
            continue;
        }
        if sp.start > at {
            out.push_str(&text[at..sp.start]);
        }
        if sp.start >= x && sp.end <= y {
            out.push_str(&text[sp.start..sp.end]);
        } else {
            let (tx, ty) = (x.max(sp.text_start), y.min(sp.text_end));
            // Only some of its marks taken, none of its text: nothing.
            if tx < ty {
                out.push_str(&text[sp.start..sp.text_start]);
                if sp.style == Style::Code {
                    out.push_str(&text[tx..ty]);
                } else {
                    piece(text, inside, tx, ty, out);
                }
                out.push_str(&text[sp.text_end..sp.end]);
            }
        }
        at = at.max(sp.end.min(y));
    }
    if at < y {
        out.push_str(&text[at..y]);
    }
}

impl Editor {
    /// The selection as valid Markdown (empty with nothing selected): spans
    /// it cuts through get their marks back; over several lines, the first
    /// keeps its block mark (a list item's, a heading's) even when the
    /// selection starts after it, and code keeps its fences.
    pub fn selected_markdown(&self) -> String {
        let (a, b) = (self.selection.start(), self.selection.end());
        if a == b {
            return String::new();
        }
        let lines: Vec<md::Line> = self.outline.iter().filter(|l| l.end >= a && l.start <= b && !(b == l.start && l.start > a)).copied().collect();
        let Some(&first) = lines.first() else { return String::new() };
        let last = *lines.last().unwrap_or(&first);
        let several = lines.len() > 1;
        let mut out = String::new();
        if several && first.block == Block::Code
            && let Some(fence) = self.fence_of(first.start, false)
        {
            out.push_str(&self.text[fence.start..fence.end]);
            out.push('\n');
        }
        for (k, l) in lines.iter().enumerate() {
            let (x, y) = (a.max(l.start), b.min(l.end));
            // The first line, taken from inside its text: its mark comes too.
            if k == 0 && several && x > l.start && l.mark > 0 {
                out.push_str(&self.text[l.start..l.content()]);
            }
            let x = if k == 0 && several && x > l.start && l.mark > 0 { x.max(l.content()) } else { x };
            piece(&self.text, &md::line_spans(&self.text, l), x, y, &mut out);
            if k + 1 < lines.len() {
                out.push('\n');
            }
        }
        if several && last.block == Block::Code {
            let close = self.fence_of(last.end, true).map_or("```".to_string(), |f| self.text[f.start..f.end].to_string());
            out.push('\n');
            out.push_str(&close);
        }
        // Up to the start of the next line: the last line's break too.
        if b > last.end {
            out.push('\n');
        }
        out
    }

    /// The fence line that opens (or, `after`, closes) the code block
    /// holding `at`.
    fn fence_of(&self, at: usize, after: bool) -> Option<md::Line> {
        let i = self.outline.partition_point(|l| l.start <= at).max(1) - 1;
        if after {
            self.outline[i..].iter().find(|l| l.block == Block::Fence).copied()
        } else {
            self.outline[..i].iter().rev().find(|l| l.block == Block::Fence).copied()
        }
    }

    /// What Copy puts on the clipboard for the selection, None with nothing
    /// selected. The plain text reads without marks, unless `show_marks`
    /// (the "Show Markdown" setting) asks for the Markdown as it is.
    /// Selected code is copied exactly, its plain text too.
    pub fn copied(&self, show_marks: bool) -> Option<Copied> {
        let (a, b) = (self.selection.start(), self.selection.end());
        if a == b {
            return None;
        }
        let markdown = self.selected_markdown();
        let in_code = |at: usize| matches!(self.line_at(at).block, Block::Code | Block::Fence);
        if in_code(a) && in_code(b) && !self.text[a..b].contains("\n```") {
            let raw = self.text[a..b].to_string();
            let fenced = format!("```\n{raw}\n```");
            return Some(Copied { html: html::to_html(&fenced, false), plain: raw.clone(), markdown: raw });
        }
        let plain = if show_marks { markdown.clone() } else { plain::to_plain(&markdown) };
        Some(Copied { html: html::to_html(&markdown, false), plain, markdown })
    }
}

#[cfg(test)]
mod tests {
    use crate::{Editor, Selection};

    fn slice(text: &str, from: &str, to: &str) -> String {
        let mut e = Editor::new(text);
        let a = text.find(from).unwrap();
        let b = text.find(to).unwrap() + to.len();
        e.set_selection(Selection { anchor: a, head: b });
        e.selected_markdown()
    }

    #[test]
    fn selected_markdown_closes_cut_spans() {
        let text = "Ship **bold words** and [a link](https://x.io) now ==mark *in* it== `co de`";
        assert_eq!(slice(text, "old", "wo"), "**old wo**");
        assert_eq!(slice(text, "words", "a li"), "**words** and [a li](https://x.io)");
        assert_eq!(slice(text, "Ship", "Ship **bold words**"), "Ship **bold words**", "whole spans as they are");
        assert_eq!(slice(text, "rk *i", "rk *i"), "==rk *i*==", "nested spans close in order");
        assert_eq!(slice(text, "de", "de"), "`de`");
        // Taking a mark but none of its text takes nothing of it.
        assert_eq!(slice(text, "s** and", "s** and"), "**s** and");
        assert_eq!(slice(text, "** and", "** and"), " and");
        // Over lines: the first keeps its block mark, cut spans close.
        let list = "- one **two\tthree**\n- four\n\tnext";
        assert_eq!(slice(list, "ne", "four"), "- ne **two\tthree**\n- four");
        assert_eq!(slice(list, "hree", "four"), "- **hree**\n- four");
        assert_eq!(slice(list, "ne", "ne"), "ne", "a word in an item is just the word");
        let heading = "## A **big** title\nbody";
        assert_eq!(slice(heading, "big", "bo"), "## **big** title\nbo");
        // Code from inside its block keeps its fences.
        let code = "Intro\n```rust\nlet a = 1;\nlet b = 2;\n```\nafter";
        assert_eq!(slice(code, "a = 1", "let b"), "```rust\na = 1;\nlet b\n```");
        assert_eq!(slice(code, "tro", "a = 1"), "tro\n```rust\nlet a = 1\n```", "a block cut through is closed");
        // Right to left: same bytes, same rule.
        assert_eq!(slice("نص **مهم جدا** هنا", "هم", "هم"), "**هم**");
    }

    #[test]
    fn copy_writes_markdown_plain_text_and_html_of_the_selection() {
        let text = "- one **two**\n- [x] three [site](https://x.io)";
        let mut e = Editor::new(text);
        assert!(e.copied(false).is_none(), "nothing selected, nothing copied");
        let a = text.find("ne").unwrap();
        e.set_selection(Selection { anchor: a, head: text.len() });
        let c = e.copied(false).unwrap();
        assert_eq!(c.markdown, "- ne **two**\n- [x] three [site](https://x.io)");
        assert_eq!(c.plain, "\u{2022} ne two\n\u{2611} three site (https://x.io)");
        assert!(c.html.contains("<strong>two</strong>") && c.html.contains("<a href=\"https://x.io\">site</a>"), "{}", c.html);
        // With Show Markdown on, the plain text is the Markdown.
        assert_eq!(e.copied(true).unwrap().plain, c.markdown);
        // Code is copied exactly: a dash in code is no bullet.
        let code = "```\n- not a list\n**raw**\n```";
        let mut e = Editor::new(code);
        let a = code.find("- not").unwrap();
        e.set_selection(Selection { anchor: a, head: code.find("raw**").unwrap() + 5 });
        let c = e.copied(false).unwrap();
        assert_eq!((c.plain.as_str(), c.markdown.as_str()), ("- not a list\n**raw**", "- not a list\n**raw**"));
        assert!(c.html.starts_with("<pre><code>- not a list\n"), "{}", c.html);
    }

    #[test]
    fn a_copy_pastes_back_as_the_same_markdown() {
        let text = "# Plan\n- one **two** ==three==\n- [ ] four [[Note|alias]]\n\n| a | b |\n| - | - |\n| 1 | 2 |\n";
        let mut from = Editor::new(text);
        from.set_selection(Selection { anchor: 0, head: text.len() });
        let copied = from.copied(false).unwrap();
        assert_eq!(copied.markdown, text, "all of a note is the note");
        let mut to = Editor::new("");
        to.paste_markdown(&copied.markdown);
        assert_eq!(to.text(), text);
        // A part of it pastes back as the same part, its marks whole.
        let a = text.find("two").unwrap();
        from.set_selection(Selection { anchor: a, head: text.find("four").unwrap() });
        let part = from.copied(false).unwrap().markdown;
        let mut to = Editor::new("");
        to.paste_markdown(&part);
        assert_eq!(to.text(), "- **two** ==three==\n- [ ] ");
    }
}
