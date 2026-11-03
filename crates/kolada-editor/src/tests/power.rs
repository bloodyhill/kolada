//! The editor's power moves: lines moved, duplicated and sorted, several
//! carets, and sections folded under their headings.

use super::*;

fn select(e: &mut Editor, anchor: usize, head: usize) {
    e.set_selection(Selection { anchor, head });
}

// Moving lines.

#[test]
fn a_line_moves_up_and_down_past_its_neighbour_with_the_caret() {
    let mut e = marked_editor("one\ntwo\nthree", 5);
    assert!(e.move_lines(true));
    assert_eq!(e.text(), "two\none\nthree");
    assert_eq!(e.selection(), Selection::caret(1), "the caret keeps its place in the line");
    assert!(!e.move_lines(true), "nothing above the first line");
    assert!(e.move_lines(false));
    assert!(e.move_lines(false));
    assert_eq!(e.text(), "one\nthree\ntwo", "the last line has no line break after it, and still moves");
    assert!(!e.move_lines(false));
    assert!(e.undo());
    assert_eq!(e.text(), "one\ntwo\nthree", "each move is one undo step");
}

#[test]
fn selected_lines_move_together_and_stay_selected() {
    let mut e = marked_editor("a\nb\nc\nd", 0);
    // From inside b to the very start of d: d is not taken.
    select(&mut e, 2, 6);
    assert!(e.move_lines(false));
    assert_eq!(e.text(), "a\nd\nb\nc");
    assert_eq!(e.selected_text(), "b\nc", "to the note's end now, where d's line began");
}

#[test]
fn an_item_moves_with_its_nested_items_past_a_sibling_with_its_own() {
    let text = "- one\n\t- one a\n- two\n\t- two a\n\t- two b\n- three";
    let mut e = editor(text, text.find("two").unwrap());
    assert!(e.move_lines(true));
    assert_eq!(e.text(), "- two\n\t- two a\n\t- two b\n- one\n\t- one a\n- three");
    assert!(e.move_lines(false));
    assert!(e.move_lines(false));
    assert_eq!(e.text(), "- one\n\t- one a\n- three\n- two\n\t- two a\n\t- two b");
    // A nested item moves among its siblings alone.
    let at = e.text().find("two b").unwrap();
    e.set_selection(Selection::caret(at));
    assert!(e.move_lines(true));
    assert_eq!(e.text(), "- one\n\t- one a\n- three\n- two\n\t- two b\n\t- two a");
}

// Duplicating lines.

#[test]
fn a_line_duplicates_below_with_the_caret_or_above_without() {
    let mut e = marked_editor("one\ntwo", 1);
    e.duplicate_lines(false);
    assert_eq!(e.text(), "one\none\ntwo");
    assert_eq!(e.selection(), Selection::caret(5), "the caret goes to the copy below");
    e.duplicate_lines(true);
    assert_eq!(e.text(), "one\none\none\ntwo");
    assert_eq!(e.selection(), Selection::caret(5), "a copy above leaves the caret on the lower line");
    let mut e = marked_editor("a\nb\nc", 0);
    select(&mut e, 0, 3);
    e.duplicate_lines(false);
    assert_eq!(e.text(), "a\nb\na\nb\nc", "selected lines duplicate together");
    assert_eq!(e.selected_text(), "a\nb");
    assert!(e.undo());
    assert_eq!(e.text(), "a\nb\nc");
}

// Sorting lines.

#[test]
fn selected_lines_sort_as_they_read() {
    let mut e = editor("- [ ] pears\n- [x] Apples\n- [ ] item 10\n- [ ] item 2", 0);
    e.select_all();
    assert!(e.sort_lines());
    assert_eq!(e.text(), "- [x] Apples\n- [ ] item 2\n- [ ] item 10\n- [ ] pears", "marks and case aside, numbers by value");
    assert_eq!(e.selection(), Selection { anchor: 0, head: e.text().len() }, "the lines stay selected");
    assert!(!e.sort_lines(), "in order already: nothing to do");
    assert!(e.undo());
    assert_eq!(e.text(), "- [ ] pears\n- [x] Apples\n- [ ] item 10\n- [ ] item 2");
}

#[test]
fn at_a_caret_the_run_of_lines_around_it_sorts() {
    let mut e = marked_editor("Title\n\ncherry\napple\nbanana\n\nafter", 9);
    assert!(e.sort_lines());
    assert_eq!(e.text(), "Title\n\napple\nbanana\ncherry\n\nafter", "up to the blank lines");
    let mut e = marked_editor("alone\n\nz", 1);
    assert!(!e.sort_lines(), "one line has nothing to sort");
}

#[test]
fn sorting_keeps_nested_items_with_theirs_and_numbers_afresh() {
    let mut e = editor("3. pear\n\t- ripe\n4. apple\n5. fig", 0);
    e.select_all();
    assert!(e.sort_lines());
    assert_eq!(e.text(), "3. apple\n4. fig\n5. pear\n\t- ripe");
}

// Several carets.

#[test]
fn typing_at_several_carets_types_at_each_and_undoes_as_one() {
    let mut e = marked_editor("ab\nab\nab", 1);
    e.add_caret(4);
    e.add_caret(7);
    assert_eq!(e.selections().len(), 3);
    type_str(&mut e, "XY");
    assert_eq!(e.text(), "aXYb\naXYb\naXYb");
    assert_eq!(e.selections(), [Selection::caret(3), Selection::caret(8), Selection::caret(13)]);
    e.backspace(false);
    assert_eq!(e.text(), "aXb\naXb\naXb");
    e.newline();
    assert_eq!(e.text(), "aX\nb\naX\nb\naX\nb");
    assert!(e.undo());
    assert_eq!(e.text(), "aXb\naXb\naXb", "Enter at every caret is one step");
    assert!(e.undo());
    assert!(e.undo());
    assert_eq!(e.text(), "ab\nab\nab", "the letters typed at them are one step");
    assert!(!e.has_carets(), "undo goes back to one caret");
}

#[test]
fn arrows_move_every_caret_and_carets_that_meet_become_one() {
    let mut e = marked_editor("abc\nabc", 1);
    e.add_caret(5);
    go(&mut e, Motion::Right, false);
    assert_eq!(e.selections(), [Selection::caret(2), Selection::caret(6)]);
    go(&mut e, Motion::LineStart, true);
    assert_eq!(e.selections(), [Selection { anchor: 2, head: 0 }, Selection { anchor: 6, head: 4 }]);
    e.insert("-");
    assert_eq!(e.text(), "-c\n-c");
    let mut e = marked_editor("abc", 1);
    e.add_caret(2);
    go(&mut e, Motion::Left, false);
    go(&mut e, Motion::Left, false);
    assert!(!e.has_carets(), "both at the start: one caret");
}

#[test]
fn alt_click_on_a_caret_takes_it_away_and_a_click_or_esc_leaves_one() {
    let mut e = marked_editor("one two three", 0);
    e.add_caret(4);
    e.add_caret(8);
    e.add_caret(4);
    assert_eq!(e.selections(), [Selection::caret(0), Selection::caret(8)]);
    assert!(e.single_caret());
    assert_eq!(e.selections(), [Selection::caret(8)], "the last added stays");
    assert!(!e.single_caret());
    e.add_caret(0);
    e.set_selection(Selection::caret(3));
    assert!(!e.has_carets(), "a plain click places one caret");
}

#[test]
fn ctrl_d_selects_the_word_then_each_next_match() {
    let mut e = marked_editor("tram to the tram, by tram", 1);
    assert!(e.add_next_match());
    assert_eq!(e.selected_text(), "tram");
    assert!(e.add_next_match());
    assert!(e.add_next_match());
    assert_eq!(e.selections().len(), 3);
    assert!(!e.add_next_match(), "every match is taken");
    type_str(&mut e, "bus");
    assert_eq!(e.text(), "bus to the bus, by bus");
    // From the middle, it goes around past the end.
    let mut e = marked_editor("cat dog cat dog", 8);
    select(&mut e, 8, 11);
    assert!(e.add_next_match());
    assert_eq!(e.selection(), Selection { anchor: 0, head: 3 });
}

#[test]
fn any_other_edit_leaves_one_caret() {
    let mut e = editor("one\ntwo", 1);
    e.add_caret(5);
    e.toggle_block(BlockKind::Heading(1));
    assert!(!e.has_carets());
}

// Folding.

const OUTLINE: &str = "# Trip\nIntro\n## Packing\nsunscreen\nadapter\n\n## Days\nAlfama\n# Back\nHome";

#[test]
fn a_heading_folds_its_section_up_to_the_next_of_its_level() {
    let e = editor(OUTLINE, 0);
    let packing = OUTLINE.find("## Packing").unwrap();
    let (from, to) = e.section(packing).unwrap();
    assert_eq!(&OUTLINE[from..to], "\nsunscreen\nadapter", "the blank line before the next heading stays out");
    let (from, to) = e.section(0).unwrap();
    assert_eq!(&OUTLINE[from..to], "\nIntro\n## Packing\nsunscreen\nadapter\n\n## Days\nAlfama", "a level 1 heading holds its level 2s");
    assert_eq!(e.section(OUTLINE.find("Intro").unwrap()), None, "not a heading");
    assert_eq!(e.section(OUTLINE.len()), None, "a heading with nothing under it");
}

#[test]
fn folding_from_inside_a_section_puts_the_caret_at_its_heading() {
    let at = OUTLINE.find("adapter").unwrap();
    let mut e = editor(OUTLINE, at);
    assert!(e.fold(at));
    let packing = OUTLINE.find("## Packing").unwrap();
    assert!(e.is_folded(packing));
    assert_eq!(e.selection(), Selection::caret(packing + "## Packing".len()));
    assert!(!e.fold(e.selection().head), "folded already");
    assert_eq!(e.text(), OUTLINE, "folds never change the text");
}

#[test]
fn the_caret_steps_over_a_folded_section() {
    let packing = OUTLINE.find("## Packing").unwrap();
    let end = packing + "## Packing".len();
    let mut e = editor(OUTLINE, end);
    e.fold(packing);
    go(&mut e, Motion::Right, false);
    assert_eq!(e.selection().head, OUTLINE.find("\n\n## Days").unwrap() + 1, "on past it");
    go(&mut e, Motion::Left, false);
    assert_eq!(e.selection().head, end, "back to the heading's end");
    go(&mut e, Motion::Down, false);
    assert!(e.selection().head > OUTLINE.find("adapter").unwrap(), "down skips it too");
}

#[test]
fn typing_at_a_folded_heading_keeps_it_folded_and_breaking_into_it_opens_it() {
    let packing = OUTLINE.find("## Packing").unwrap();
    let end = packing + "## Packing".len();
    let mut e = editor(OUTLINE, end);
    e.fold(packing);
    type_str(&mut e, "!");
    assert!(e.is_folded(packing), "renaming it keeps it folded");
    e.insert("\n");
    assert!(!e.is_folded(packing), "a new line goes into the section, which opens");
    let mut e = editor(OUTLINE, end);
    e.fold(packing);
    e.delete(false);
    assert!(!e.is_folded(packing), "joining its first line to the heading opens it");
}

#[test]
fn folds_follow_the_text_above_them_and_go_with_their_heading() {
    let packing = OUTLINE.find("## Packing").unwrap();
    let mut e = editor(OUTLINE, 2);
    e.fold(packing);
    type_str(&mut e, "Big ");
    assert_eq!(e.fold_heads(), [packing + 4], "the fold moves with its heading");
    e.set_selection(Selection::caret(e.text().find("Intro").unwrap()));
    e.newline();
    assert_eq!(e.fold_heads(), [packing + 5]);
    // The heading's mark removed: no longer a heading, no fold.
    let head = e.fold_heads()[0];
    e.set_selection(Selection::caret(head + 3));
    e.toggle_block(BlockKind::Heading(2));
    assert!(e.fold_heads().is_empty());
    // Kept with the note, and put back.
    let mut e = editor(OUTLINE, 0);
    e.set_folds(&[packing, 3, 0]);
    assert_eq!(e.fold_heads(), [0, packing], "only headings fold");
    assert!(e.unfold(0));
    assert!(e.toggle_fold(packing + 3));
    assert!(e.fold_heads().is_empty());
}
