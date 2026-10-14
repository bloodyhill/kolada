//! What the screen's buttons and shortcuts do to an editor, the same on every
//! OS: the toolbar's formatting.

use kolada_editor::{BlockKind, Editor, Inline, Layout};

use crate::scene::Tool;

/// A toolbar button's formatting, on any editor.
pub fn apply_tool(e: &mut Editor, tool: Tool) {
    match tool {
        Tool::Heading => e.cycle_heading(),
        Tool::Bold => e.toggle_inline(Inline::Bold),
        Tool::Italic => e.toggle_inline(Inline::Italic),
        Tool::Strike => e.toggle_inline(Inline::Strike),
        Tool::List => e.toggle_block(BlockKind::Bullet),
        Tool::Checklist => e.toggle_block(BlockKind::Task),
        Tool::Quote => e.toggle_block(BlockKind::Quote),
        Tool::Code => e.code(),
        Tool::Link => e.link(),
    }
}

/// The toolbar buttons for the formatting at `caret` (lit while the caret
/// is in it). The Windows shell's `app.rs` has the same.
pub fn active_tools(text: &str, lines: &[kolada_markdown::Line], caret: usize) -> Vec<Tool> {
    use kolada_markdown::{Block, Style};
    if lines.is_empty() || caret > text.len() || !text.is_char_boundary(caret) {
        return Vec::new();
    }
    let line = lines[lines.partition_point(|l| l.start <= caret).max(1) - 1];
    let mut tools = Vec::new();
    match line.block {
        Block::Heading(_) => tools.push(Tool::Heading),
        Block::Bullet | Block::Ordered => tools.push(Tool::List),
        Block::Task { .. } => tools.push(Tool::Checklist),
        Block::Quote => tools.push(Tool::Quote),
        Block::Fence | Block::Code => tools.push(Tool::Code),
        _ => {}
    }
    for s in kolada_markdown::line_spans(text, &line) {
        if s.text_start <= caret && caret <= s.text_end {
            tools.push(match s.style {
                Style::Strong => Tool::Bold,
                Style::Emphasis => Tool::Italic,
                Style::Strike => Tool::Strike,
                Style::Code => Tool::Code,
                Style::WikiLink | Style::Link | Style::Url => Tool::Link,
                Style::Highlight | Style::Tag => continue,
            });
        }
    }
    tools
}

/// A field's text is one line: moving in it needs no real layout.
pub struct OneLine<'a>(pub &'a str);

impl Layout for OneLine<'_> {
    fn visual_line(&self, _: usize) -> (usize, usize) {
        (0, self.0.len())
    }

    fn x_of(&self, offset: usize) -> f32 {
        offset as f32
    }

    fn above(&self, _: usize, _: f32) -> Option<usize> {
        None
    }

    fn below(&self, _: usize, _: f32) -> Option<usize> {
        None
    }

    fn rtl(&self, _: usize) -> bool {
        crate::scene::is_rtl(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_editor::Selection;

    #[test]
    fn bold_wraps_the_selection_and_a_list_marks_the_line() {
        let mut e = Editor::new("one two");
        e.set_selection(Selection { anchor: 4, head: 7 });
        apply_tool(&mut e, Tool::Bold);
        assert_eq!(e.text(), "one **two**");
        e.set_selection(Selection::caret(0));
        apply_tool(&mut e, Tool::List);
        assert_eq!(e.text(), "- one **two**");
    }
}
