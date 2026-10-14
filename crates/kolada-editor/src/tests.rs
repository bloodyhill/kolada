use super::*;

/// One visual line per paragraph; x counts graphemes from the line start.
struct Mono<'a> {
    text: &'a str,
    rtl: bool,
}

impl Mono<'_> {
    fn column_at(&self, (start, end): (usize, usize), x: f32) -> usize {
        let mut at = start;
        for (col, (i, g)) in self.text[start..end].grapheme_indices(true).enumerate() {
            if col as f32 >= x {
                return start + i;
            }
            at = start + i + g.len();
        }
        at
    }
}

impl Layout for Mono<'_> {
    fn visual_line(&self, offset: usize) -> (usize, usize) {
        line_range(self.text, offset)
    }

    fn x_of(&self, offset: usize) -> f32 {
        let (start, _) = line_range(self.text, offset);
        self.text[start..offset].graphemes(true).count() as f32
    }

    fn above(&self, offset: usize, x: f32) -> Option<usize> {
        let (start, _) = line_range(self.text, offset);
        (start > 0).then(|| self.column_at(line_range(self.text, start - 1), x))
    }

    fn below(&self, offset: usize, x: f32) -> Option<usize> {
        let (_, end) = line_range(self.text, offset);
        (end < self.text.len()).then(|| self.column_at(line_range(self.text, end + 1), x))
    }

    fn rtl(&self, _offset: usize) -> bool {
        self.rtl
    }
}

fn editor(text: &str, caret: usize) -> Editor {
    let mut e = Editor::new(text);
    e.set_selection(Selection::caret(caret));
    e
}

fn go(e: &mut Editor, motion: Motion, extend: bool) {
    let text = e.text().to_string();
    e.move_caret(motion, extend, &Mono { text: &text, rtl: false });
}

fn type_str(e: &mut Editor, s: &str) {
    for g in s.graphemes(true) {
        e.insert(g);
    }
}

#[test]
fn typing_moves_the_caret() {
    let mut e = Editor::new("");
    type_str(&mut e, "hi 👋");
    assert_eq!(e.text(), "hi 👋");
    assert_eq!(e.selection(), Selection::caret(e.text().len()));
}

#[test]
fn backspace_and_delete_remove_whole_graphemes() {
    // Skin-toned emoji, a family (ZWJ sequence), e + combining acute, and
    // Arabic beh with a fatha.
    for g in ["👍🏽", "👨‍👩‍👧", "e\u{301}", "بَ"] {
        let mut e = editor(&format!("a{g}b"), 1 + g.len());
        e.backspace(false);
        assert_eq!(e.text(), "ab", "backspace over {g:?}");
        e.undo();
        e.set_selection(Selection::caret(1));
        e.delete(false);
        assert_eq!(e.text(), "ab", "delete over {g:?}");
    }
}

#[test]
fn arrows_step_over_graphemes() {
    let text = "a👨‍👩‍👧b";
    let mut e = editor(text, 1);
    go(&mut e, Motion::Right, false);
    assert_eq!(e.selection().head, text.len() - 1);
    go(&mut e, Motion::Left, false);
    assert_eq!(e.selection().head, 1);
}

#[test]
fn arrows_follow_the_screen_in_right_to_left_paragraphs() {
    let text = "مرحبا";
    let mut e = editor(text, 0);
    e.move_caret(Motion::Left, false, &Mono { text, rtl: true });
    assert_eq!(e.selection().head, "م".len());
    e.move_caret(Motion::Right, false, &Mono { text, rtl: true });
    assert_eq!(e.selection().head, 0);
}

#[test]
fn selection_snaps_to_grapheme_boundaries() {
    let text = "👍🏽x";
    let mut e = editor(text, 4); // between the thumb and its skin tone
    assert_eq!(e.selection().head, 0);
    e.set_selection(Selection::caret(2)); // inside a UTF-8 sequence
    assert_eq!(e.selection().head, 0);
    e.set_selection(Selection::caret(999));
    assert_eq!(e.selection().head, text.len());
}

#[test]
fn word_motion_skips_spaces_and_punctuation() {
    let text = "hello, world";
    let mut e = editor(text, 0);
    go(&mut e, Motion::WordRight, false);
    assert_eq!(e.selection().head, 5);
    go(&mut e, Motion::WordRight, false);
    assert_eq!(e.selection().head, 12);
    go(&mut e, Motion::WordLeft, false);
    assert_eq!(e.selection().head, 7);
    go(&mut e, Motion::WordLeft, false);
    assert_eq!(e.selection().head, 0);
}

#[test]
fn word_motion_stops_at_line_breaks() {
    let text = "one\ntwo";
    let mut e = editor(text, 4);
    go(&mut e, Motion::WordLeft, false);
    assert_eq!(e.selection().head, 3);
    go(&mut e, Motion::WordRight, false);
    assert_eq!(e.selection().head, 4);
    go(&mut e, Motion::WordRight, false);
    assert_eq!(e.selection().head, 7);
}

#[test]
fn ctrl_backspace_and_ctrl_delete_remove_words() {
    let mut e = editor("hello big world", 15);
    e.backspace(true);
    assert_eq!(e.text(), "hello big ");
    e.set_selection(Selection::caret(0));
    e.delete(true);
    assert_eq!(e.text(), " big ");
}

#[test]
fn shift_extends_and_plain_arrows_collapse() {
    let mut e = editor("abcd", 1);
    go(&mut e, Motion::Right, true);
    go(&mut e, Motion::Right, true);
    assert_eq!(e.selection(), Selection { anchor: 1, head: 3 });
    assert_eq!(e.selected_text(), "bc");
    go(&mut e, Motion::Left, false);
    assert_eq!(e.selection(), Selection::caret(1));
}

#[test]
fn typing_replaces_the_selection() {
    let mut e = editor("hello world", 0);
    e.set_selection(Selection { anchor: 11, head: 6 });
    e.insert("you");
    assert_eq!(e.text(), "hello you");
    assert_eq!(e.selection(), Selection::caret(9));
}

#[test]
fn up_and_down_keep_the_column() {
    let text = "long line\nab\nlong line";
    let mut e = editor(text, 7);
    go(&mut e, Motion::Down, false);
    assert_eq!(e.selection().head, 12, "end of the short line");
    go(&mut e, Motion::Down, false);
    assert_eq!(e.selection().head, 13 + 7, "back to column 7");
    go(&mut e, Motion::Down, false);
    assert_eq!(e.selection().head, text.len(), "past the last line");
    go(&mut e, Motion::DocStart, false);
    go(&mut e, Motion::Up, false);
    assert_eq!(e.selection().head, 0);
}

#[test]
fn page_moves_keep_the_column_and_stop_at_the_ends() {
    let text = "abcd\nabcd\nab\nabcd\nabcd";
    let mut e = editor(text, 3);
    let plan = e.plan_lines(3, false, &Mono { text, rtl: false });
    e.apply(plan);
    assert_eq!(e.selection().head, 13 + 3, "past the short line, back in column 3");
    let plan = e.plan_lines(10, true, &Mono { text, rtl: false });
    e.apply(plan);
    assert_eq!(e.selection(), Selection { anchor: 16, head: text.len() });
    let plan = e.plan_lines(-10, false, &Mono { text, rtl: false });
    e.apply(plan);
    assert_eq!(e.selection().head, 0);
}

#[test]
fn home_and_end_use_the_visual_line() {
    let mut e = editor("one\ntwo\nthree", 5);
    go(&mut e, Motion::LineStart, false);
    assert_eq!(e.selection().head, 4);
    go(&mut e, Motion::LineEnd, true);
    assert_eq!(e.selected_text(), "two");
}

#[test]
fn undo_groups_typing_by_word() {
    let mut e = Editor::new("");
    type_str(&mut e, "hello world");
    assert!(e.undo());
    assert_eq!(e.text(), "hello ");
    assert!(e.undo());
    assert_eq!(e.text(), "");
    assert!(!e.undo());
    assert!(e.redo());
    assert_eq!(e.text(), "hello ");
    assert!(e.redo());
    assert_eq!(e.text(), "hello world");
    assert_eq!(e.selection(), Selection::caret(11));
}

#[test]
fn undo_groups_a_run_of_deletes() {
    let mut e = editor("abcdef", 6);
    for _ in 0..3 {
        e.backspace(false);
    }
    assert_eq!(e.text(), "abc");
    e.undo();
    assert_eq!(e.text(), "abcdef");
    assert_eq!(e.selection(), Selection::caret(6));
}

#[test]
fn moving_the_caret_ends_an_undo_group() {
    let mut e = Editor::new("");
    type_str(&mut e, "ab");
    go(&mut e, Motion::Left, false);
    type_str(&mut e, "c");
    assert_eq!(e.text(), "acb");
    e.undo();
    assert_eq!(e.text(), "ab");
}

#[test]
fn undo_restores_the_selection() {
    let mut e = editor("hello world", 0);
    let selected = Selection { anchor: 6, head: 11 };
    e.set_selection(selected);
    e.insert("x");
    e.undo();
    assert_eq!(e.text(), "hello world");
    assert_eq!(e.selection(), selected);
}

#[test]
fn a_new_edit_clears_redo() {
    let mut e = Editor::new("");
    type_str(&mut e, "a");
    e.undo();
    type_str(&mut e, "b");
    assert!(!e.redo());
    assert_eq!(e.text(), "b");
}

#[test]
fn enter_continues_and_ends_lists() {
    let mut e = editor("- milk", 6);
    e.newline();
    assert_eq!(e.text(), "- milk\n- ");
    e.newline();
    assert_eq!(e.text(), "- milk\n", "enter on an empty item ends the list");

    let mut e = editor("9. nine", 7);
    e.newline();
    assert_eq!(e.text(), "9. nine\n10. ");

    let mut e = editor("  - [x] done", 12);
    e.newline();
    assert_eq!(e.text(), "  - [x] done\n  - [ ] ");

    let mut e = editor("- ab", 3);
    e.newline();
    assert_eq!(e.text(), "- a\n- b", "splitting an item");
    assert_eq!(e.selection(), Selection::caret(6));
}

#[test]
fn enter_at_the_start_of_an_item_opens_one_above() {
    let mut e = editor("- milk", 0);
    assert_eq!(e.selection().head, 2, "the caret starts after the mark");
    e.newline();
    assert_eq!(e.text(), "- \n- milk");
    assert_eq!(e.selection(), Selection::caret(5), "still before milk");
    let mut e = editor("# Plan", 2);
    e.newline();
    assert_eq!(e.text(), "\n# Plan", "a heading gets a plain line above");
}

#[test]
fn enter_on_an_empty_block_removes_its_mark_and_continues_quotes() {
    let mut e = editor("# ", 2);
    e.newline();
    assert_eq!(e.text(), "");
    let mut e = editor("> hi", 4);
    e.newline();
    assert_eq!(e.text(), "> hi\n> ");
    e.newline();
    assert_eq!(e.text(), "> hi\n");
}

#[test]
fn the_caret_never_rests_inside_a_block_mark() {
    let mut e = editor("- milk", 1);
    assert_eq!(e.selection().head, 2);
    go(&mut e, Motion::Left, false);
    assert_eq!(e.selection().head, 2, "nothing before the first item's text");
    let mut e = editor("x\n- milk", 4);
    go(&mut e, Motion::Left, false);
    assert_eq!(e.selection().head, 1, "from the item's text straight to the line above");
    go(&mut e, Motion::Right, false);
    assert_eq!(e.selection().head, 4, "and back, past the mark");
    go(&mut e, Motion::WordLeft, false);
    assert_eq!(e.selection().head, 1);
    go(&mut e, Motion::DocEnd, false);
    go(&mut e, Motion::LineStart, false);
    assert_eq!(e.selection().head, 4, "Home goes to the text, not the mark");
}

#[test]
fn the_caret_goes_into_a_headings_marks() {
    let mut e = editor("x\n# Plan", 4);
    go(&mut e, Motion::Left, false);
    assert_eq!(e.selection().head, 3, "between the # and the space");
    e.set_selection(Selection::caret(2));
    assert_eq!(e.selection().head, 2, "before the #, where one more makes it smaller");
    e.insert("#");
    assert_eq!(e.text(), "x\n## Plan");
}

#[test]
fn backspace_at_an_items_text_removes_its_mark() {
    let mut e = editor("- milk", 2);
    e.backspace(false);
    assert_eq!(e.text(), "milk");
    assert_eq!(e.selection(), Selection::caret(0));
    // A heading's #s show while it is edited: they delete as they look.
    let mut e = editor("## Title", 3);
    e.backspace(false);
    assert_eq!(e.text(), "##Title");
    // A word backspace stops at the text, too.
    let mut e = editor("- , x", 3);
    e.backspace(true);
    assert_eq!(e.text(), "-  x");
}

#[test]
fn delete_at_a_line_end_joins_the_next_text_without_its_mark() {
    let mut e = editor("a\n- b", 1);
    e.delete(false);
    assert_eq!(e.text(), "ab");
}

#[test]
fn bold_wraps_the_selection_and_unwraps_it_again() {
    let mut e = editor("a word b", 0);
    e.set_selection(Selection { anchor: 2, head: 6 });
    e.toggle_inline(Inline::Bold);
    assert_eq!(e.text(), "a **word** b");
    assert_eq!(e.selected_text(), "word");
    e.toggle_inline(Inline::Bold);
    assert_eq!(e.text(), "a word b");
    assert_eq!(e.selected_text(), "word");
    // Spaces at the selection's ends stay outside the marks.
    e.set_selection(Selection { anchor: 1, head: 7 });
    e.toggle_inline(Inline::Strike);
    assert_eq!(e.text(), "a ~~word~~ b");
}

#[test]
fn inline_formatting_without_a_selection_opens_a_pair() {
    let mut e = editor("x ", 2);
    e.toggle_inline(Inline::Code);
    assert_eq!(e.text(), "x ``");
    assert_eq!(e.selection(), Selection::caret(3));
    let mut e = editor("", 0);
    e.link();
    assert_eq!(e.text(), "[[]]");
    assert_eq!(e.selection(), Selection::caret(2));
}

#[test]
fn italic_on_bold_text_adds_to_it() {
    let mut e = editor("**x**", 0);
    e.set_selection(Selection { anchor: 2, head: 3 });
    e.toggle_inline(Inline::Italic);
    assert_eq!(e.text(), "***x***");
}

#[test]
fn inline_marks_wrap_each_line_of_a_selection() {
    let mut e = editor("one\n- two", 0);
    e.select_all();
    e.toggle_inline(Inline::Italic);
    assert_eq!(e.text(), "*one*\n- *two*");
}

#[test]
fn blocks_toggle_across_the_selected_lines() {
    let mut e = editor("a\nb", 0);
    e.select_all();
    e.toggle_block(BlockKind::Bullet);
    assert_eq!(e.text(), "- a\n- b");
    e.toggle_block(BlockKind::Numbered);
    assert_eq!(e.text(), "1. a\n2. b");
    e.toggle_block(BlockKind::Task);
    assert_eq!(e.text(), "- [ ] a\n- [ ] b");
    e.toggle_block(BlockKind::Task);
    assert_eq!(e.text(), "a\nb");
    e.toggle_block(BlockKind::Quote);
    assert_eq!(e.text(), "> a\n> b");
    // One command, one undo step.
    e.undo();
    assert_eq!(e.text(), "a\nb");
}

#[test]
fn the_heading_button_cycles_levels() {
    let mut e = editor("Plan", 4);
    let mut seen = vec![];
    for _ in 0..4 {
        e.cycle_heading();
        seen.push(e.text().to_string());
    }
    assert_eq!(seen, ["# Plan", "## Plan", "### Plan", "Plan"]);
    assert_eq!(e.selection(), Selection::caret(4), "the caret stays at the end of the text");
}

#[test]
fn tasks_check_and_uncheck_in_place() {
    let mut e = editor("- [ ] call", 10);
    assert!(e.toggle_task(0));
    assert_eq!(e.text(), "- [x] call");
    assert!(e.toggle_task(7));
    assert_eq!(e.text(), "- [ ] call");
    assert_eq!(e.selection(), Selection::caret(10));
    let mut plain = editor("call", 0);
    assert!(!plain.toggle_task(0));
}

#[test]
fn tab_nests_list_items() {
    let mut e = editor("- a\n- b", 7);
    assert!(e.indent(false));
    assert_eq!(e.text(), "- a\n\t- b");
    assert!(e.indent(true));
    assert_eq!(e.text(), "- a\n- b");
    let mut e = editor("  - c", 5);
    assert!(e.indent(true));
    assert_eq!(e.text(), "- c");
    let mut plain = editor("text", 0);
    assert!(!plain.indent(false));
}

#[test]
fn code_fences_several_lines() {
    let mut e = editor("a\nb", 0);
    e.select_all();
    e.code();
    assert_eq!(e.text(), "```\na\nb\n```");
}

#[test]
fn double_and_triple_click_select_words_and_lines() {
    let mut e = editor("one two\nthree", 0);
    e.select_word_at(5);
    assert_eq!(e.selected_text(), "two");
    e.select_word_at(7);
    assert_eq!(e.selected_text(), "two", "past the end of a line takes its last word");
    e.select_word_at(3);
    assert_eq!(e.selected_text(), " ");
    e.select_line_at(10);
    assert_eq!(e.selected_text(), "three");
    e.select_all();
    assert_eq!(e.selected_text(), "one two\nthree");
}

#[test]
fn version_changes_only_with_the_text() {
    let mut e = editor("abc", 3);
    let v = e.version();
    go(&mut e, Motion::Left, false);
    assert_eq!(e.version(), v);
    e.backspace(false);
    assert!(e.version() > v);
    let v = e.version();
    e.undo();
    assert!(e.version() > v);
}

#[test]
fn no_two_texts_share_a_version() {
    // Two notes opened one after the other, the same or not: what is kept
    // by version for one is never taken for the other's.
    let (a, b) = (Editor::new("one"), Editor::new("one"));
    assert_ne!(a.version(), b.version());
    let mut c = Editor::new("abc");
    c.set_selection(Selection::caret(3));
    c.backspace(false);
    assert!(c.version() != a.version() && c.version() != b.version());
}

fn fix(start: usize, end: usize, text: &str) -> (usize, usize, String) {
    (start, end, text.to_string())
}

#[test]
fn an_amend_is_one_undo_step() {
    let text = "teh cat sat on teh mat";
    let mut e = editor(text, text.len());
    let placed = e.amend(&[fix(15, 18, "the"), fix(0, 3, "the"), fix(8, 11, "sits")]);
    assert_eq!(e.text(), "the cat sits on the mat");
    assert_eq!(placed, [(0, 3), (8, 12), (16, 19)], "in text order, in offsets after the edit");
    assert_eq!(e.selection(), Selection::caret(e.text().len()));
    assert!(e.undo());
    assert_eq!(e.text(), text);
    assert_eq!(e.selection(), Selection::caret(text.len()));
    assert!(!e.undo(), "one step for all three");
    assert!(e.redo());
    assert_eq!(e.text(), "the cat sits on the mat");
    assert_eq!(e.selection(), Selection::caret(e.text().len()));
}

#[test]
fn undoing_an_amend_puts_back_the_selection() {
    let mut e = editor("a teh b", 0);
    let selected = Selection { anchor: 4, head: 1 };
    e.set_selection(selected);
    e.amend(&[fix(2, 5, "the")]);
    assert_eq!(e.selection(), Selection { anchor: 5, head: 1 }, "the anchor was inside the range");
    e.undo();
    assert_eq!(e.text(), "a teh b");
    assert_eq!(e.selection(), selected);
    e.redo();
    assert_eq!(e.selection(), Selection { anchor: 5, head: 1 });
}

#[test]
fn an_amend_keeps_the_caret_in_its_place() {
    // "a teh b": teh (2..5) becomes thee (2..6).
    for (caret, to, why) in [(1, 1, "before"), (2, 2, "at the start"), (3, 6, "inside"), (5, 6, "at the end"), (6, 7, "after"), (7, 8, "at the end of the note")] {
        let mut e = editor("a teh b", caret);
        e.amend(&[fix(2, 5, "thee")]);
        assert_eq!(e.text(), "a thee b");
        assert_eq!(e.selection(), Selection::caret(to), "caret {why}");
    }
    let mut e = editor("a teh b", 7);
    e.amend(&[fix(2, 5, "x")]);
    assert_eq!(e.selection(), Selection::caret(5), "a shorter range pulls it back");

    let mut e = editor("a teh b", 0);
    e.set_selection(Selection { anchor: 7, head: 1 });
    e.amend(&[fix(2, 5, "thee")]);
    assert_eq!(e.selection(), Selection { anchor: 8, head: 1 });
    assert_eq!(e.selected_text(), " thee b", "a selection around a range grows with it");
}

#[test]
fn an_amend_reads_again_only_what_matches_a_full_read() {
    let text = "# Plan\n``\ncode? no\n``\n- [ ] by milk\nend";
    let second = text.rfind("``").unwrap();
    let by = text.find("by").unwrap();
    let mut e = editor(text, text.len());
    // Back to front: the lower fence opens first, running to the end of
    // the note, then the upper one opens and the lower one closes.
    e.amend(&[fix(9, 9, "`"), fix(second + 2, second + 2, "`"), fix(by, by + 2, "buy"), fix(2, 6, "Plans")]);
    assert_eq!(e.text(), "# Plans\n```\ncode? no\n```\n- [ ] buy milk\nend");
    let check = |e: &Editor| {
        assert_eq!(e.outline(), md::lines(e.text()).collect::<Vec<_>>());
        assert_eq!(e.states, md::lines_with_states(e.text()).map(|(_, s)| s).collect::<Vec<_>>());
    };
    check(&e);
    assert_eq!(e.outline()[2].block, Block::Code);
    assert_eq!(e.outline()[4].block, Block::Task { done: false });
    e.undo();
    check(&e);
    e.redo();
    check(&e);
}

#[test]
fn amending_an_items_text_keeps_its_block() {
    let text = "- [ ] by teh milk\n- call mom";
    let teh = text.find("teh").unwrap();
    let call = text.find("call").unwrap();
    let mut e = editor(text, 6);
    e.amend(&[fix(call, call + 4, "Call"), fix(teh, teh + 3, "the")]);
    assert_eq!(e.text(), "- [ ] by the milk\n- Call mom");
    assert_eq!((e.outline()[0].block, e.outline()[0].mark), (Block::Task { done: false }, 6));
    assert_eq!((e.outline()[1].block, e.outline()[1].mark), (Block::Bullet, 2));
    assert_eq!(e.selection(), Selection::caret(6), "still at the start of the item's text");
    e.amend(&[fix(6, 17, "buy the milk")]);
    assert_eq!(e.text(), "- [ ] buy the milk\n- Call mom");
    assert_eq!(e.outline()[0].block, Block::Task { done: false });
}

#[test]
fn the_caret_moves_out_of_a_mark_an_amend_writes() {
    let mut e = editor("b", 0);
    e.amend(&[fix(0, 1, "- b")]);
    assert_eq!(e.text(), "- b");
    assert_eq!(e.selection(), Selection::caret(2));
    e.undo();
    assert_eq!(e.selection(), Selection::caret(0));
}

#[test]
fn undone_amend_names_what_undo_put_back() {
    let text = "teh cat sat on teh mat";
    let mut e = editor(text, 0);
    e.amend(&[fix(0, 3, "the"), fix(8, 11, "sits"), fix(15, 18, "a")]);
    assert_eq!(e.undone_amend(), None, "nothing undone yet");
    e.undo();
    assert_eq!(e.undone_amend(), Some((0, 18)));
    e.redo();
    assert_eq!(e.undone_amend(), None, "after a redo");
    e.undo();
    type_str(&mut e, "!");
    assert_eq!(e.undone_amend(), None, "after a new edit");
    e.undo();
    assert_eq!(e.undone_amend(), None, "after undoing typing");
}

#[test]
fn typing_after_an_amend_is_undone_on_its_own() {
    let mut e = Editor::new("");
    type_str(&mut e, "teh");
    e.amend(&[fix(0, 3, "the")]);
    type_str(&mut e, "y");
    assert_eq!(e.text(), "they");
    e.undo();
    assert_eq!(e.text(), "the");
    e.undo();
    assert_eq!(e.text(), "teh");
    assert_eq!(e.selection(), Selection::caret(3));
    e.undo();
    assert_eq!(e.text(), "");
}

#[test]
fn an_amend_that_changes_nothing_records_nothing() {
    let mut e = editor("the cat", 3);
    let v = e.version();
    assert!(e.amend(&[]).is_empty());
    assert!(e.amend(&[fix(2, 2, ""), fix(0, 3, "the")]).is_empty(), "empty, and the text already there");
    assert_eq!(e.version(), v);
    assert!(!e.undo());
    assert_eq!(e.amend(&[fix(7, 7, ""), fix(4, 7, "dog")]), [(4, 7)], "only the real change is recorded");
    e.undo();
    assert!(!e.undo());
}

#[test]
fn overlapping_amends_change_nothing() {
    let mut e = editor("teh cat", 7);
    let v = e.version();
    assert!(e.amend(&[fix(0, 3, "the"), fix(2, 5, "x")]).is_empty());
    assert!(e.amend(&[fix(4, 7, "dog"), fix(0, 5, "the c")]).is_empty());
    assert!(e.amend(&[fix(0, 4, "the "), fix(3, 3, "x")]).is_empty(), "an insertion inside a range");
    assert_eq!(e.text(), "teh cat");
    assert_eq!(e.selection(), Selection::caret(7));
    assert_eq!(e.version(), v);
    assert!(!e.undo());
}

#[test]
#[cfg_attr(debug_assertions, should_panic(expected = "amend ranges"))]
fn amends_out_of_bounds_change_nothing() {
    let text = "مرحبا";
    let mut e = editor(text, 0);
    // Out of bounds, backward, and inside a letter's UTF-8 bytes.
    for bad in [fix(0, 99, "x"), fix(4, 2, "x"), fix(1, 2, "x")] {
        assert!(e.amend(&[fix(0, 2, "ا"), bad]).is_empty());
    }
    assert_eq!(e.text(), text);
    assert!(!e.undo());
}

#[test]
fn amends_in_arabic_and_emoji() {
    let text = "كتبت ملاحظه 👍 اليوم";
    let word = text.find("ملاحظه").unwrap();
    let thumb = text.find('👍').unwrap();
    let mut e = editor(text, word + "ملا".len());
    let placed = e.amend(&[fix(thumb, thumb + '👍'.len_utf8(), "👍🏽"), fix(word, word + "ملاحظه".len(), "الملاحظة")]);
    assert_eq!(e.text(), "كتبت الملاحظة 👍🏽 اليوم");
    let parts: Vec<&str> = placed.iter().map(|&(a, b)| &e.text()[a..b]).collect();
    assert_eq!(parts, ["الملاحظة", "👍🏽"]);
    assert_eq!(e.selection(), Selection::caret(placed[0].1), "from inside the word to its end");
    e.set_selection(Selection::caret(e.text().len()));
    e.undo();
    assert_eq!(e.text(), text);
    assert_eq!(e.selection(), Selection::caret(word + "ملا".len()));
    e.redo();
    assert_eq!(e.text(), "كتبت الملاحظة 👍🏽 اليوم");
}

#[test]
fn titles_come_from_the_first_line() {
    assert_eq!(title_of("\n\n# Groceries \nmilk"), "Groceries");
    assert_eq!(title_of("- [ ] call mom"), "call mom");
    assert_eq!(title_of("> quoted idea"), "quoted idea");
    assert_eq!(title_of("#tag first"), "#tag first");
    assert_eq!(title_of("   "), "");
    let long = "👍🏽".repeat(100);
    assert_eq!(title_of(&long), "👍🏽".repeat(80));
}

#[test]
fn composing_shows_text_but_records_only_what_is_committed() {
    let mut e = editor("ab", 1);
    e.set_marked("k", 1);
    e.set_marked("\u{304B}", 3);
    assert_eq!((e.text(), e.marked(), e.selection()), ("a\u{304B}b", Some((1, 4)), Selection::caret(4)));
    e.commit_marked("\u{6F22}\u{5B57}");
    assert_eq!((e.text(), e.marked(), e.selection()), ("a\u{6F22}\u{5B57}b", None, Selection::caret(7)));
    // One step, back to before composing, and forward again.
    assert!(e.undo());
    assert_eq!((e.text(), e.selection()), ("ab", Selection::caret(1)));
    assert!(e.redo());
    assert_eq!(e.text(), "a\u{6F22}\u{5B57}b");
}

#[test]
fn a_cancelled_composition_puts_back_what_it_replaced() {
    let mut e = editor("hello world", 6);
    e.set_selection(Selection { anchor: 6, head: 11 });
    e.set_marked("x", 1);
    assert_eq!(e.text(), "hello x");
    e.cancel_marked();
    assert_eq!((e.text(), e.selection(), e.marked()), ("hello world", Selection { anchor: 6, head: 11 }, None));
    assert!(!e.undo() || e.text() == "hello world", "nothing of it was recorded");
}

#[test]
fn anything_else_done_mid_composition_commits_it_first() {
    let mut e = editor("ab", 1);
    e.set_marked("\u{304B}", 3);
    e.insert("!");
    assert_eq!((e.text(), e.marked()), ("a\u{304B}!b", None));
    // Committed, it is typing like any other: one word, one undo step.
    assert!(e.undo());
    assert_eq!(e.text(), "ab");
    // A click elsewhere settles it too.
    e.set_marked("\u{306A}", 3);
    e.set_selection(Selection::caret(0));
    assert_eq!((e.text(), e.marked()), ("a\u{306A}b", None));
    // With no composition, committed text is typed (the emoji panel).
    let mut e = editor("ab", 2);
    e.commit_marked("\u{1F600}");
    assert_eq!(e.text(), "ab\u{1F600}");
}

#[test]
fn pasted_markdown_keeps_its_structure_where_the_caret_can_take_it() {
    // In text: as it is, one undo step.
    let mut e = editor("Notes\n", 6);
    e.paste_markdown("## Plan\n\n- milk\n- eggs");
    assert_eq!(e.text(), "Notes\n## Plan\n\n- milk\n- eggs");
    e.undo();
    assert_eq!(e.text(), "Notes\n");
    // On an empty item, a pasted list takes its place.
    let mut e = editor("- milk\n- ", 9);
    e.paste_markdown("- eggs\n- oat");
    assert_eq!(e.text(), "- milk\n- eggs\n- oat");
    // Words into the empty item go in it.
    let mut e = editor("- milk\n- ", 9);
    e.paste_markdown("**fresh** bread");
    assert_eq!(e.text(), "- milk\n- **fresh** bread");
    // Not in code, nor in a table's cell.
    assert!(editor("Notes", 2).takes_markdown());
    assert!(!editor("```\ncode\n```", 5).takes_markdown());
    assert!(!editor(TABLE, 2).takes_markdown());
}

const TABLE: &str = "| a | bc |\n| - | -- |\n| 1 | 2 |\nend";

#[test]
fn the_caret_keeps_to_a_tables_cells() {
    // At the start of the note: into the first cell.
    let mut e = editor(TABLE, 0);
    assert_eq!(e.selection().head, 2);
    // Right from the end of a cell's text: the next cell, past the pipe.
    go(&mut e, Motion::Right, false);
    go(&mut e, Motion::Right, false);
    assert_eq!(e.selection().head, 6);
    go(&mut e, Motion::Left, false);
    assert_eq!(e.selection().head, 3);
    // From the header's last cell over the alignment row to the first row,
    // and back.
    e.set_selection(Selection::caret(8));
    go(&mut e, Motion::Right, false);
    assert_eq!(e.selection().head, TABLE.find('1').unwrap());
    go(&mut e, Motion::Left, false);
    assert_eq!(e.selection().head, 8);
    // Out of the last cell to the line after the table, and back.
    e.set_selection(Selection::caret(TABLE.find('2').unwrap() + 1));
    go(&mut e, Motion::Right, false);
    assert_eq!(e.selection().head, TABLE.find("end").unwrap());
    go(&mut e, Motion::Left, false);
    assert_eq!(e.selection().head, TABLE.find('2').unwrap() + 1);
    // Home and End keep to the cell.
    e.set_selection(Selection::caret(7));
    go(&mut e, Motion::LineStart, false);
    assert_eq!(e.selection().head, 6);
    go(&mut e, Motion::LineEnd, false);
    assert_eq!(e.selection().head, 8);
    assert!(e.in_table());
    e.set_selection(Selection::caret(TABLE.len()));
    assert!(!e.in_table());
}

#[test]
fn tab_goes_from_cell_to_cell_and_adds_a_row_after_the_last() {
    let mut e = editor(TABLE, 2);
    assert!(e.indent(false));
    assert_eq!(e.selected_text(), "bc");
    assert!(e.indent(false));
    assert_eq!(e.selected_text(), "1");
    assert!(e.indent(true));
    assert_eq!(e.selected_text(), "bc");
    e.indent(false);
    e.indent(false);
    assert_eq!(e.selected_text(), "2");
    // Past the last cell: a new row, the table lined up, the caret in it.
    assert!(e.indent(false));
    assert_eq!(e.text(), "| a   | bc  |\n| --- | --- |\n| 1   | 2   |\n|     |     |\nend");
    let row = e.text().rfind("\n|  ").unwrap() + 1;
    assert_eq!(e.selection(), Selection::caret(row + 2));
    // One undo step.
    e.undo();
    assert_eq!(e.text(), TABLE);
    // Shift+Tab in the first cell of the header stays.
    let mut e = editor(TABLE, 2);
    assert!(e.indent(true));
    assert_eq!(e.text(), TABLE);
}

#[test]
fn enter_in_a_table_adds_a_row_and_on_an_empty_last_row_ends_it() {
    let mut e = editor(TABLE, TABLE.find('1').unwrap());
    e.newline();
    assert_eq!(e.text(), "| a   | bc  |\n| --- | --- |\n| 1   | 2   |\n|     |     |\nend");
    type_str(&mut e, "x");
    assert_eq!(e.text(), "| a   | bc  |\n| --- | --- |\n| 1   | 2   |\n| x    |     |\nend");
    // Enter in the header: the new row comes first.
    let mut e = editor(TABLE, 2);
    e.newline();
    assert_eq!(e.text(), "| a   | bc  |\n| --- | --- |\n|     |     |\n| 1   | 2   |\nend");
    // On an empty last row, the row becomes a plain line after the table.
    let mut e = editor(TABLE, TABLE.find('2').unwrap());
    e.newline();
    e.newline();
    assert_eq!(e.text(), "| a   | bc  |\n| --- | --- |\n| 1   | 2   |\n\nend");
    assert_eq!(e.outline()[3].block, Block::Body);
    assert_eq!(e.selection(), Selection::caret(e.text().len() - "\nend".len()));
}

#[test]
fn typing_and_deleting_in_a_cell_keep_its_pipes() {
    // A typed pipe is escaped; it and its backslash go together.
    let mut e = editor(TABLE, 3);
    e.insert("|");
    assert_eq!(&e.text()[..12], r"| a\| | bc |");
    assert!(e.in_table());
    e.backspace(false);
    assert_eq!(e.text(), TABLE);
    // Pasted lines become one.
    e.insert("x\ny");
    assert_eq!(&e.text()[..14], "| ax y | bc |\n");
    // Nothing goes at a cell's start or end.
    let mut e = editor(TABLE, 6);
    e.backspace(false);
    e.backspace(true);
    e.set_selection(Selection::caret(8));
    e.delete(false);
    e.delete(true);
    assert_eq!(e.text(), TABLE);
    // A word deleted stops at the cell's edge.
    e.backspace(true);
    assert_eq!(e.text(), "| a |  |\n| - | -- |\n| 1 | 2 |\nend");
    // Backspace at the start of a row of empty cells takes the row.
    let text = "| a | b |\n| - | - |\n| 1 | 2 |\n|  |  |\nend";
    let mut e = editor(text, text.find("|  |").unwrap());
    e.backspace(false);
    assert_eq!(e.text(), "| a | b |\n| - | - |\n| 1 | 2 |\nend");
    assert_eq!(e.selection(), Selection::caret(text.find('2').unwrap() + 1));
}

#[test]
fn a_selection_across_cells_takes_their_text_not_the_table() {
    // Within a table: the cells' parts go, pipes and rows stay.
    let mut e = editor(TABLE, 2);
    e.set_selection(Selection { anchor: 2, head: 7 });
    e.backspace(false);
    assert_eq!(e.text(), "|  | c |\n| - | -- |\n| 1 | 2 |\nend");
    assert_eq!(e.selection(), Selection::caret(2));
    // Over rows: typed text goes in the first cell.
    let mut e = editor(TABLE, 6);
    e.set_selection(Selection { anchor: 6, head: TABLE.find('2').unwrap() });
    e.insert("z");
    assert_eq!(e.text(), "| a | z |\n| - | -- |\n|  | 2 |\nend");
    // From a paragraph into a table: the paragraph's end goes, its line
    // break and the header's pipes stay.
    let text = format!("intro\n{TABLE}");
    let mut e = editor(&text, 2);
    e.set_selection(Selection { anchor: 2, head: text.find('c').unwrap() });
    e.delete(false);
    assert_eq!(e.text(), "in\n|  | c |\n| - | -- |\n| 1 | 2 |\nend");
    // All of it: all of it goes.
    let mut e = editor(TABLE, 2);
    e.select_all();
    e.backspace(false);
    assert_eq!(e.text(), "");
    // Triple-click in a cell: its text.
    let mut e = editor(TABLE, 2);
    e.select_line_at(7);
    assert_eq!(e.selected_text(), "bc");
}

#[test]
fn a_tables_menu_adds_and_removes_rows_and_columns_and_aligns_them() {
    let mut e = editor(TABLE, TABLE.find('1').unwrap());
    assert!(e.edit_table(TableEdit::ColumnRight));
    assert_eq!(e.text(), "| a   |     | bc  |\n| --- | --- | --- |\n| 1   |     | 2   |\nend");
    let empty = e.text().rfind("|     |").unwrap() + 2;
    assert_eq!(e.selection(), Selection::caret(empty));
    type_str(&mut e, "42");
    assert!(e.edit_table(TableEdit::Align(md::Align::Right)));
    assert_eq!(e.text(), "| a   |     | bc  |\n| --- | --: | --- |\n| 1   |  42 | 2   |\nend");
    // The caret stays where it was in the cell's text.
    assert_eq!(e.selection(), Selection::caret(e.text().find("42").unwrap() + 2));
    assert!(e.edit_table(TableEdit::DeleteColumn));
    assert_eq!(e.text(), "| a   | bc  |\n| --- | --- |\n| 1   | 2   |\nend");
    assert!(e.edit_table(TableEdit::RowAbove));
    assert_eq!(e.text(), "| a   | bc  |\n| --- | --- |\n|     |     |\n| 1   | 2   |\nend");
    assert!(e.edit_table(TableEdit::DeleteRow));
    assert!(e.edit_table(TableEdit::DeleteRow));
    assert_eq!(e.text(), "| a   | bc  |\n| --- | --- |\nend");
    // Not the header row, not the only column.
    assert!(!e.edit_table(TableEdit::DeleteRow));
    assert!(!e.edit_table(TableEdit::RowAbove));
    let mut one = editor("| a |\n| - |\nend", 2);
    assert!(!one.edit_table(TableEdit::DeleteColumn));
    // Each change is one undo step (typing "42" is one more).
    for _ in 0..8 {
        e.undo();
    }
    assert_eq!(e.text(), TABLE);
    let mut plain = editor("no table", 2);
    assert!(!plain.edit_table(TableEdit::RowBelow));
    assert_eq!(plain.table_place(), None);
    // What the menu offers comes from where the caret is.
    let e = editor("| a | b |\n| - | --: |\n| 1 | 2 |", 6);
    assert_eq!(e.table_place(), Some(TablePlace { header: true, columns: 2, align: md::Align::Right }));
}
