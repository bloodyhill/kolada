//! Tidying a note: the untidiness nobody means to leave. Spaces left at the
//! ends of lines, runs of empty lines, empty lines before the first word and
//! after the last. Meant for the moment a note is left, not while it is typed
//! in. Code, front matter and tables are never touched, and a hard line break
//! (two spaces at the end of a line that carries on) stays.

use crate::{Block, Editor};

impl Editor {
    /// Tidies the note in one undo step. Whether anything changed.
    pub fn tidy(&mut self) -> bool {
        if self.marked.is_some() {
            return false;
        }
        let edits = self.tidy_edits();
        if edits.is_empty() {
            return false;
        }
        self.replace_ranges(&edits);
        true
    }

    /// The edits [`Editor::tidy`] would make: start, end, replacement.
    pub fn tidy_edits(&self) -> Vec<(usize, usize, String)> {
        let text = self.text();
        let blank = |i: usize| {
            let l = &self.outline[i];
            !matches!(l.block, Block::Fence | Block::Code | Block::Frontmatter | Block::TableRow | Block::TableDelimiter)
                && text[l.start..l.end].trim().is_empty()
        };
        let Some(last) = (0..self.outline.len()).rev().find(|&i| !blank(i)) else {
            // nothing but empty lines
            return if text.is_empty() { Vec::new() } else { vec![(0, text.len(), String::new())] };
        };
        let mut edits = Vec::new();
        let mut empty_run = 0;
        let mut seen = false;
        for i in 0..=last {
            let l = &self.outline[i];
            if matches!(l.block, Block::Fence | Block::Code | Block::Frontmatter | Block::TableRow | Block::TableDelimiter) {
                (empty_run, seen) = (0, true);
                continue;
            }
            let raw = &text[l.start..l.end];
            let kept = raw.trim_end_matches([' ', '\t']).len();
            if blank(i) {
                empty_run += 1;
                if empty_run > 1 || !seen {
                    // the whole line, and the break after it
                    edits.push((l.start, l.end + 1, String::new()));
                } else if kept < raw.len() {
                    edits.push((l.start, l.end, String::new()));
                }
                continue;
            }
            (empty_run, seen) = (0, true);
            if kept == raw.len() || (l.mark > 0 && l.content() == l.end) {
                continue; // nothing to strip, or an empty list item whose space is its mark
            }
            let hard_break = raw.ends_with("  ") && i < last && !blank(i + 1);
            if !hard_break {
                edits.push((l.start + kept, l.end, String::new()));
            }
        }
        // after the last word: no empty lines, and the final line break only if it had one
        let end = self.outline[last].end;
        let keep = if text.ends_with('\n') { "\n" } else { "" };
        if &text[end..] != keep {
            edits.push((end, text.len(), keep.to_string()));
        }
        edits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Selection;

    fn tidied(text: &str) -> String {
        let mut ed = Editor::new(text);
        ed.tidy();
        ed.text().to_string()
    }

    #[test]
    fn spaces_at_the_end_of_a_line_go() {
        assert_eq!(tidied("one  \ntwo \t\n\nthree"), "one  \ntwo\n\nthree", "a hard break stays");
        assert_eq!(tidied("one \ntwo\t\nthree"), "one\ntwo\nthree");
        assert_eq!(tidied("a   \n\nb"), "a\n\nb", "two spaces before an empty line break nothing");
    }

    #[test]
    fn a_run_of_empty_lines_becomes_one() {
        assert_eq!(tidied("a\n\n\n\nb\n"), "a\n\nb\n");
        assert_eq!(tidied("a\n  \n \t \nb"), "a\n\nb", "lines of only spaces are empty lines");
    }

    #[test]
    fn nothing_is_left_before_the_first_word_or_after_the_last() {
        assert_eq!(tidied("\n\n  \nHello\n"), "Hello\n");
        assert_eq!(tidied("Hello\n\n\n\n"), "Hello\n");
        assert_eq!(tidied("Hello\n  \n"), "Hello\n");
        assert_eq!(tidied("Hello \n\n"), "Hello\n");
        assert_eq!(tidied("Hello\n\n\n  "), "Hello", "a note saved without a final line break gets none");
    }

    #[test]
    fn code_front_matter_and_tables_are_left_as_written() {
        let note = "---\ntitle: x  \n\n\n---\n\n```\ncode   \n\n\n\nmore  \n```\n\n| a | b |\n| - | - |\n| 1 |  2 |  \n";
        assert_eq!(tidied(note), note);
        assert_eq!(tidied(&format!("{note}tail   \n\n\n\nend")), format!("{note}tail\n\nend"));
    }

    #[test]
    fn an_empty_list_item_keeps_its_mark() {
        assert_eq!(tidied("- a \n- \n- c"), "- a\n- \n- c");
        assert_eq!(tidied("- [ ] \ntext"), "- [ ] \ntext");
    }

    #[test]
    fn a_tidy_note_is_left_alone_and_nothing_is_recorded() {
        let mut ed = Editor::new("# Title\n\nSome words.  \nMore.\n\n- a\n- b\n");
        assert!(!ed.tidy());
        assert!(!ed.undo(), "no undo step for nothing");
        assert_eq!(ed.text(), "# Title\n\nSome words.  \nMore.\n\n- a\n- b\n");
        assert_eq!(tidied(""), "");
        assert_eq!(tidied("\n\n"), "");
    }

    #[test]
    fn it_is_one_undo_step_and_the_caret_stays_with_its_word() {
        let text = "a  \n\n\n\nb word \n\n";
        let mut ed = Editor::new(text);
        let at = text.find("word").unwrap() + 2;
        ed.set_selection(Selection::caret(at));
        assert!(ed.tidy());
        assert_eq!(ed.text(), "a\n\nb word\n");
        assert_eq!(&ed.text()[ed.selection().head..], "rd\n", "still inside 'word'");
        assert!(ed.undo());
        assert_eq!(ed.text(), text);
        assert!(!ed.undo());
    }

    #[test]
    fn tidying_twice_does_nothing_the_second_time() {
        let once = tidied("x \n\n\n y \n\n\n\n");
        assert_eq!(once, "x\n\n y\n");
        assert_eq!(tidied(&once), once);
    }

    #[test]
    fn arabic_and_emoji_survive() {
        assert_eq!(tidied("مرحبا  \n\n\n👨‍👩‍👧 \n"), "مرحبا\n\n👨‍👩‍👧\n");
        assert_eq!(tidied("مرحبا  \nأهلا \n"), "مرحبا  \nأهلا\n");
    }
}
