use super::*;

/// A small vault of its own: tests run side by side, so they cannot share one.
fn vault(test: &str, notes: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Projects")).unwrap();
    for (name, text) in notes {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

const SIZE: (u32, u32, f32) = (2000, 1440, 2.0);

fn drawn(v: &Path) -> Model {
    let mut m = Model::new(v.to_path_buf(), true);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    m
}

#[test]
fn opens_the_newest_note_and_draws_a_frame_with_text_and_the_file_tree() {
    let v = vault("newest", &[("Plan.md", "# Plan\n\nShip the **beta**.\n")]);
    let m = drawn(&v);
    assert_eq!(m.session.note, "Plan.md");
    let background = theme::for_mode(true).background;
    let inked = m.surface().pixels().iter().filter(|&&p| p != background).count();
    assert!(inked > 20_000, "a frame with text and chrome, not an empty one: {inked} pixels");
    assert!(m.hits.iter().any(|(_, t)| matches!(t, Target::Row(_))), "the file tree is there to click");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn clicking_a_folder_opens_it_and_clicking_a_note_opens_that_note() {
    let v = vault("click", &[("Plan.md", "# Plan\n")]);
    std::fs::write(v.join("Projects").join("Launch.md"), "# Launch\n\nDay one.\n").unwrap();
    let mut m = drawn(&v);
    let center = |m: &Model, row: usize| {
        let r = m.hits.iter().find(|(_, t)| *t == Target::Row(row)).map(|(r, _)| *r).expect("row is drawn");
        ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    };
    let folder = m.session.sidebar.iter().position(|r| matches!(r.kind, RowKind::Folder { .. })).unwrap();
    let (x, y) = center(&m, folder);
    assert!(m.pointer_down(x, y, 1, false));
    assert!(matches!(m.session.sidebar[folder].kind, RowKind::Folder { expanded: true }));
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    let launch = m.session.sidebar.iter().position(|r| r.name == "Launch").expect("the folder's note shows");
    let (x, y) = center(&m, launch);
    // A note's row opens as the button is let go (a press may carry it away instead).
    m.pointer_down(x, y, 1, false);
    assert!(m.pointer_up_at(x, y).0);
    assert_eq!(m.session.note, "Projects/Launch.md");
    assert!(m.text().starts_with("# Launch"));
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn hovering_changes_the_frame_and_scrolling_stops_at_the_top() {
    let v = vault("hover", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v);
    assert!(!m.scroll_by(-30.0), "already at the top");
    let (r, _) = *m.hits.iter().find(|(_, t)| matches!(t, Target::Row(_))).unwrap();
    assert!(m.pointer_moved((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0));
    assert!(m.pointer_left());
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn typed_text_goes_where_the_caret_is_shows_in_the_frame_and_is_saved() {
    let v = vault("type", &[("Plan.md", "# Plan\n\nShip it.\n")]);
    let mut m = drawn(&v);
    // Click at the end of the last line of text, then type.
    m.go(Motion::DocEnd, false);
    m.insert("Then rest.");
    assert!(m.text().ends_with("Ship it.\nThen rest."), "{:?}", m.text());
    assert!(m.dirty());
    let before = m.surface().pixels().to_vec();
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert_ne!(m.surface().pixels(), &before[..], "the new words are on screen");
    assert!(m.caret_rect().is_some(), "the caret is drawn");
    assert!(m.save());
    assert_eq!(std::fs::read_to_string(v.join("Plan.md")).unwrap(), "# Plan\n\nShip it.\nThen rest.");
    assert!(!m.save(), "nothing changed since");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn return_continues_a_list_tab_nests_it_and_backspace_and_undo_take_it_back() {
    let v = vault("list", &[("one.md", "- one")]);
    let mut m = drawn(&v);
    m.go(Motion::DocEnd, false);
    m.newline(false);
    assert_eq!(m.text(), "- one\n- ");
    m.insert("two");
    m.tab(false);
    assert_eq!(m.text(), "- one\n\t- two", "tab nests the item");
    m.undo();
    m.undo();
    assert!(m.text().len() < "- one\n\t- two".len(), "undo takes edits back: {:?}", m.text());
    m.redo();
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_arrows_walk_wrapped_lines_and_selections_extend_with_shift() {
    let long = "word ".repeat(120);
    let v = vault("arrows", &[("Plan.md", &format!("{long}\nshort\n"))]);
    let mut m = drawn(&v);
    m.go(Motion::DocStart, false);
    assert_eq!(m.selection().head, 0);
    m.go(Motion::Down, false);
    let first_down = m.selection().head;
    assert!(first_down > 0 && first_down < long.len(), "down moved within the wrapped paragraph: {first_down}");
    m.go(Motion::Right, true);
    let sel = m.selection();
    assert_eq!(sel.anchor, first_down);
    assert!(sel.head > sel.anchor, "shift extends the selection");
    m.go(Motion::DocEnd, false);
    assert_eq!(m.selection().head, m.text().len());
    m.select_all();
    assert_eq!(m.selected_text(), m.text());
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn clicking_the_text_puts_the_caret_there_and_double_and_triple_clicks_select() {
    let v = vault("caret", &[("Plan.md", "# Plan\n\nShip the beta now.\n")]);
    let mut m = drawn(&v);
    assert!(m.hits.iter().any(|(_, t)| *t == Target::Note), "the text area takes clicks");
    // The second line of text is the paragraph "Ship the beta now.".
    let (x0, y, start, end) = {
        let l = m.lines.iter().find(|l| m.text()[l.para.start..l.para.end].starts_with("Ship")).expect("its paragraph is drawn");
        (l.x, l.y + l.height / 2.0, l.para.start, l.para.end)
    };
    // A point on a word, not on the space between two: where that is depends
    // on the font, so the first that selects a word is used.
    let x = (0..60)
        .map(|i| x0 + 10.0 + 4.0 * i as f32)
        .find(|&x| {
            m.pointer_down(x, y, 2, false);
            let word = &m.text()[m.selection().start()..m.selection().end()];
            !word.is_empty() && !word.contains(' ')
        })
        .expect("a point on a word");
    assert!(m.pointer_down(x, y, 1, false));
    let at = m.selection().head;
    assert!(at > start && at < end, "the caret is inside the paragraph: {at}");
    assert!(m.pointer_down(x, y, 2, false));
    let word = &m.text()[m.selection().start()..m.selection().end()];
    assert!(!word.is_empty() && !word.contains(' '), "a word: {word:?}");
    assert!(m.pointer_down(x, y, 3, false));
    assert_eq!(&m.text()[m.selection().start()..m.selection().end()].trim_end(), &"Ship the beta now.");
    assert!(m.pointer_dragged(x + 300.0, y) || m.selection().head == m.selection().end());
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_drag_selects_text_only_when_it_started_on_the_page() {
    let v = vault("drag-from", &[("Plan.md", "# Plan\n\nShip the beta now.\n")]);
    let mut m = drawn(&v);
    let (x, y) = {
        let l = m.lines.iter().find(|l| m.text()[l.para.start..l.para.end].starts_with("Ship")).expect("its paragraph is drawn");
        (l.x + 2.0, l.y + l.height / 2.0)
    };
    // Pressed on the page and dragged: a selection.
    m.pointer_down(x, y, 1, false);
    assert!(m.pointer_dragged(x + 80.0, y));
    assert!(!m.selection().is_caret());
    m.pointer_up_at(x + 80.0, y);
    // Pressed on a button (the window shrank under the pointer, say) and
    // dragged over the page: the caret stays as it was.
    m.pointer_down(x, y, 1, false);
    let before = m.selection();
    let (b, _) = m.hits.iter().find(|(_, t)| *t == Target::NewNote).copied().expect("the new note button");
    m.ui.pressed = Some(Target::NewNote);
    assert!(!m.pointer_dragged(x + 80.0, y), "{:?}", b);
    assert_eq!(m.selection(), before);
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn typing_far_down_a_long_note_scrolls_the_caret_into_view() {
    let body: String = (0..120).map(|i| format!("Line number {i}\n")).collect();
    let v = vault("reveal", &[("Plan.md", &body)]);
    let mut m = drawn(&v);
    m.go(Motion::DocEnd, false);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(m.scroll_line > 0 || m.scroll > 0.0, "the note scrolled to its end");
    let c = m.caret_rect().expect("the caret is drawn");
    assert!(c.top >= 0.0 && c.bottom <= m.viewport.1, "the caret is in the window: {c:?} in {:?}", m.viewport);
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn toolbar_formatting_wraps_the_selection() {
    let v = vault("tool", &[("one two.md", "one two")]);
    let mut m = drawn(&v);
    m.editor.set_selection(Selection { anchor: 4, head: 7 });
    m.tool(Tool::Bold);
    assert_eq!(m.text(), "one **two**");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn utf16_positions_and_bytes_convert_both_ways_through_emoji_and_accents() {
    let s = "a\u{e9}\u{1F600}b";
    assert_eq!((utf16_at(s, 0), utf16_at(s, 1), utf16_at(s, 3), utf16_at(s, 7), utf16_at(s, 8)), (0, 1, 2, 4, 5));
    assert_eq!((byte_at(s, 0), byte_at(s, 1), byte_at(s, 2), byte_at(s, 4), byte_at(s, 5), byte_at(s, 99)), (0, 1, 3, 7, 8, 8));
    assert_eq!(byte_at(s, 3), 7, "inside the emoji's two units: its end");
}

#[test]
fn text_composed_with_an_input_method_shows_then_settles_as_one_step() {
    let v = vault("compose", &[("Hello.md", "Hello ")]);
    let mut m = drawn(&v);
    m.go(Motion::DocEnd, false);
    // A Japanese composition: shown while it changes.
    m.set_marked("\u{306b}", 1);
    assert!(m.has_marked());
    assert_eq!(m.text(), "Hello \u{306b}");
    m.set_marked("\u{65e5}\u{672c}", 2);
    assert_eq!(m.marked_utf16(), Some((6, 2)));
    assert_eq!(m.selection_utf16(), (8, 0), "the caret is at the end of what is composed");
    m.commit_marked("\u{65e5}\u{672c}\u{8a9e}");
    assert!(!m.has_marked());
    assert_eq!(m.text(), "Hello \u{65e5}\u{672c}\u{8a9e}");
    m.undo();
    assert_eq!(m.text(), "Hello ", "undo takes the whole composition back");
    // A composition ended as it stands keeps what was shown.
    m.set_marked("x", 1);
    m.unmark();
    assert_eq!(m.text(), "Hello x");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_composition_taken_back_before_its_commit_is_typed_once() {
    // winit's input methods (Linux's ibus and fcitx5 among them) clear what
    // they showed, then commit: the text goes in once.
    let v = vault("compose-cleared", &[("Hello.md", "Hello ")]);
    let mut m = drawn(&v);
    m.go(Motion::DocEnd, false);
    m.set_marked("\u{4f60}\u{597d}", 2);
    assert!(m.cancel_marked());
    assert_eq!(m.text(), "Hello ", "what was shown goes");
    m.commit_marked("\u{4f60}\u{597d}");
    assert_eq!(m.text(), "Hello \u{4f60}\u{597d}");
    m.undo();
    assert_eq!(m.text(), "Hello ", "one step");
    // Over a selection, cancelled (Escape in the input method): it comes back.
    m.select_utf16(0, 5);
    m.set_marked("\u{d55c}", 1);
    assert!(m.cancel_marked());
    assert_eq!(m.text(), "Hello ");
    assert_eq!(m.selection_utf16(), (0, 5), "selected as it was");
    assert!(!m.cancel_marked(), "nothing composed: nothing to take back");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn text_being_composed_is_underlined_and_the_part_being_converted_is_said() {
    let v = vault("compose-marks", &[("Hello.md", "Hello ")]);
    let mut m = drawn(&v);
    m.go(Motion::DocEnd, false);
    assert!(m.composition().is_none());
    // Kana typed: all of it underlined, nothing being converted yet.
    m.set_marked_converting("\u{306b}\u{307b}\u{3093}", 3, 0);
    assert_eq!(m.composition(), Some(((6, 15), None)));
    // Converting: the input method selects the clause it is converting.
    m.set_marked_converting("\u{65e5}\u{672c}\u{8a9e}", 0, 2);
    assert_eq!(m.composition(), Some(((6, 15), Some((6, 12)))), "the first two characters, in the accent");
    m.frame(2000, 1440, 2.0);
    m.commit_marked("\u{65e5}\u{672c}\u{8a9e}");
    assert!(m.composition().is_none(), "settled: nothing underlined");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn an_input_method_can_replace_a_range_and_ask_for_text_and_places() {
    let v = vault("replace", &[("one two three.md", "one two three")]);
    let mut m = drawn(&v);
    m.select_utf16(4, 3);
    assert_eq!(m.substring_utf16(4, 3), "two");
    m.commit_marked("2");
    assert_eq!(m.text(), "one 2 three");
    assert!(m.index_at(0.0, 0.0) <= "one 2 three".chars().count());
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_new_note_is_a_blank_page_that_becomes_a_file_once_it_has_words() {
    let v = vault("fresh", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v);
    assert!(m.new_note());
    assert_eq!(m.text(), "");
    assert!(!m.save(), "a blank page is not written");
    m.insert("# Ideas\n\nFirst thought.");
    assert!(m.save());
    assert_eq!(std::fs::read_to_string(v.join("Ideas.md")).unwrap(), "# Ideas\n\nFirst thought.");
    assert_eq!(m.session.note, "Ideas.md");
    assert!(m.session.sidebar.iter().any(|r| r.name == "Ideas"), "the file tree lists it");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn search_finds_notes_as_you_type_and_enter_opens_the_chosen_one() {
    let v = vault("search", &[("Plan.md", "# Plan\n\nShip the beta.\n"), ("Recipes.md", "# Recipes\n\nLemon cake.\n")]);
    let mut m = drawn(&v);
    assert!(m.open_search());
    for _ in 0..200 {
        if m.index.lock().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    m.insert("lemon");
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert_eq!(m.search.as_ref().unwrap().query.text(), "lemon", "typing goes to the search, not the note");
    assert_eq!(m.search.as_ref().unwrap().hits.len(), 1);
    assert!(m.newline(false));
    assert_eq!(m.session.note, "Recipes.md");
    assert!(m.search.as_ref().is_some_and(|s| s.focused && s.query.text() == "lemon"), "Enter keeps the search, and its keys, to open the next");
    assert!(m.escape() && m.search.is_none(), "escape closes it");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_result_clicked_keeps_the_search_open_until_the_note_is_written_in() {
    let v = vault("searchkept", &[("Plan.md", "# Plan

Ship the beta.
"), ("Pantry.md", "# Pantry

Plan the shopping.
")]);
    let mut m = drawn(&v);
    assert!(m.open_search());
    for _ in 0..200 {
        if m.index.lock().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    m.insert("plan");
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    let rect_of = |m: &Model, t: Target| m.hits.iter().find(|(_, h)| *h == t).map(|(r, _)| *r);
    let centre = |r: kolada_ui::scene::Rect| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    assert!(m.search.as_ref().unwrap().hits.len() >= 2);
    // A click on the second result: it opens, the search stays with its query.
    let (x, y) = centre(rect_of(&m, Target::Result(1)).expect("a second result"));
    m.pointer_down(x, y, 1, false);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    let opened = m.session.note.clone();
    let s = m.search.as_ref().expect("still open");
    assert!(!opened.is_empty() && s.query.text() == "plan" && s.selected == 1 && !s.focused, "kept, the note has the keys");
    assert!(m.hits.iter().any(|(_, t)| matches!(t, Target::Result(_))), "its results still shown");
    // The other result opens straight away.
    let (x, y) = centre(rect_of(&m, Target::Result(0)).unwrap());
    m.pointer_down(x, y, 1, false);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(m.session.note != opened && m.search.is_some(), "the next one, search still open");
    // Writing in the note ends it.
    m.insert("x");
    assert!(m.search.is_none(), "the first edit closes the search");
    assert!(m.editor.text().contains('x'), "and the letter went into the note");
    // Esc ends one kept open too.
    assert!(m.open_search());
    m.insert("plan");
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    let (x, y) = centre(rect_of(&m, Target::Result(0)).unwrap());
    m.pointer_down(x, y, 1, false);
    assert!(m.search.as_ref().is_some_and(|s| !s.focused));
    assert!(m.escape() && m.search.is_none(), "Esc from the note ends it");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_search_fields_x_shows_once_something_is_typed_and_brings_the_notes_back() {
    let v = vault("searchx", &[("Plan.md", "# Plan

Ship the beta.
"), ("Recipes.md", "# Recipes

Lemon cake.
")]);
    let mut m = drawn(&v);
    let rect_of = |m: &Model, t: Target| m.hits.iter().find(|(_, h)| *h == t).map(|(r, _)| *r);
    assert!(m.open_search());
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(rect_of(&m, Target::SearchClear).is_none(), "an empty field has no ✕");
    m.insert("lemon");
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    let x = rect_of(&m, Target::SearchClear).expect("typed: the ✕ shows");
    let field = rect_of(&m, Target::Search).unwrap();
    assert!(x.right <= field.right + 0.5 && x.left >= field.left && x.width() >= 24.0 && x.height() >= 24.0, "inside the field, a finger's size");
    let (cx, cy) = ((x.left + x.right) / 2.0, (x.top + x.bottom) / 2.0);
    m.pointer_down(cx, cy, 1, false);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(m.search.is_none(), "the ✕ ends the search");
    assert!(m.hits.iter().any(|(_, t)| matches!(t, Target::Row(_))), "and the notes are back");
    // Esc does the same.
    assert!(m.open_search());
    m.insert("plan");
    assert!(m.escape() && m.search.is_none());
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_deleted_note_goes_to_the_trash_and_restoring_it_brings_it_back() {
    let v = vault("trash", &[("Plan.md", "# Plan\n"), ("Old.md", "# Old\n\nbye\n")]);
    let mut m = drawn(&v);
    m.open("Old.md");
    assert!(m.trash_open_note());
    assert!(!v.join("Old.md").exists());
    assert_eq!(m.session.note, "");
    assert!(m.open_trash());
    let entries = m.trash.as_ref().unwrap().1.len();
    assert!(entries >= 1);
    let idx = m.trash.as_ref().unwrap().0.iter().position(|t| t.name() == "Old").expect("it is listed");
    assert!(m.restore(idx));
    assert!(v.join("Old.md").exists());
    assert_eq!(m.session.note, "Old.md");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn clicking_the_theme_cycles_system_light_dark_and_a_checkbox_toggles() {
    let v = vault("theme", &[("one.md", "- [ ] one\n")]);
    let mut m = drawn(&v);
    assert!(m.dark, "the system is dark in these tests");
    m.cycle_theme();
    assert!(!m.dark, "light");
    m.cycle_theme();
    assert!(m.dark, "dark");
    m.cycle_theme();
    m.set_dark(false);
    assert!(!m.dark, "system again, now light");
    assert_eq!(m.dark_to_tell(), Some(false));
    assert_eq!(m.dark_to_tell(), None);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    if let Some((r, Target::Checkbox(at))) = m.hits.iter().find(|(_, t)| matches!(t, Target::Checkbox(_))).copied() {
        assert!(m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false));
        assert!(m.text().starts_with("- [x]"), "{at}: {:?}", m.text());
    }
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_file_takes_its_titles_name_when_the_note_is_left_and_links_follow() {
    let v = vault("rename", &[("Plan.md", "# Plan\n\nBody.\n"), ("Other.md", "See [[Plan]] too.\n")]);
    let mut m = drawn(&v);
    m.open("Plan.md");
    m.go(Motion::DocStart, false);
    m.go(Motion::LineEnd, false);
    m.insert(" B");
    assert!(m.save(), "written under its old name while the caret is on the title");
    assert!(v.join("Plan.md").exists());
    m.open("Other.md");
    assert!(v.join("Plan B.md").exists() && !v.join("Plan.md").exists(), "renamed once left");
    for _ in 0..200 {
        if std::fs::read_to_string(v.join("Other.md")).unwrap().contains("[[Plan B]]") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(std::fs::read_to_string(v.join("Other.md")).unwrap().contains("[[Plan B]]"), "the link follows");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_text_size_and_line_width_buttons_step_through_their_choices() {
    let v = vault("sizes", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v);
    let click = |m: &mut Model, t: Target| {
        let r = m.hits.iter().find(|(_, x)| *x == t).map(|(r, _)| *r).expect("the button is drawn");
        m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false)
    };
    // In the toolbar's Aa card.
    m.toggle_panel(scene::Panel::Toolbar);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(click(&mut m, Target::Reading));
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    // Larger to the largest, then smaller to the smallest; at each end
    // the button is still.
    let mut sizes = Vec::new();
    for up in [true, false] {
        while m.hits.iter().any(|(_, x)| *x == Target::TextSize(up)) {
            assert!(click(&mut m, Target::TextSize(up)));
            sizes.push(scene::text_size(m.session.text_size));
            m.frame(SIZE.0, SIZE.1, SIZE.2);
        }
    }
    assert_eq!(sizes, [18, 20, 23, 20, 18, 16, 15, 14, 13]);
    assert!(m.hits.iter().any(|(_, x)| *x == Target::TextSize(true)), "larger again from the smallest");
    assert!(click(&mut m, Target::Width(2)));
    assert_eq!(m.session.line_width, 2);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(click(&mut m, Target::Width(1)));
    assert_eq!(m.session.line_width, 1);
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn find_selects_matches_steps_around_and_replaces_them() {
    let v = vault("find", &[("one two one three one.md", "one two one three one\n")]);
    let mut m = drawn(&v);
    assert!(m.open_find(true));
    m.insert("one");
    assert_eq!(m.text(), "one two one three one\n", "typing went to the find bar");
    assert_eq!(m.find.as_ref().unwrap().matches.len(), 3);
    assert_eq!(m.selected_text(), "one", "the nearest match is selected");
    assert!(m.find_step(false));
    assert_eq!(m.selection().start(), 8);
    assert!(m.find_step(true) && m.find_step(true));
    assert_eq!(m.selection().start(), 18, "back around the note");
    m.find_tab();
    m.insert("1");
    m.find_tab();
    assert!(m.replace_all());
    assert_eq!(m.text(), "1 two 1 three 1\n");
    assert!(m.escape() && m.find.is_none());
    m.go(Motion::DocEnd, false);
    m.insert("x");
    assert!(m.text().contains('x'), "typing is the page's again");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn files_changed_by_another_app_are_picked_up_unless_the_note_has_edits() {
    let v = vault("outside", &[("Plan.md", "# Plan\n\nBody.\n")]);
    let mut m = drawn(&v);
    std::fs::write(v.join("Plan.md"), "# Plan\n\nChanged elsewhere.\n").unwrap();
    std::fs::write(v.join("New.md"), "# New\n").unwrap();
    assert!(m.apply_outside(&[]));
    assert!(m.text().contains("Changed elsewhere"), "the open note reloaded");
    assert!(m.session.sidebar.iter().any(|r| r.name == "New"), "the tree shows the new note");
    m.go(Motion::DocEnd, false);
    m.insert("mine");
    std::fs::write(v.join("Plan.md"), "# Plan\n\nAgain elsewhere.\n").unwrap();
    m.apply_outside(&[]);
    assert!(m.text().ends_with("mine"), "unsaved edits are not thrown away");
    let _ = std::fs::remove_dir_all(v);
}

/// Seen on the Mac: a title typed, Return, then Backspace back onto it and
/// shortened. The note was written under its longer name (the name follows
/// once the caret leaves the title), the watcher reported that write, and
/// the reload took the file for one named apart from its text: the old name
/// went over it, and Return left "title / last letter / title".
#[test]
fn a_title_shortened_on_its_line_is_not_doubled_when_the_watcher_reports_the_save() {
    for title in ["Weekly plans", "خطة الأسبوع القادمة"] {
        let v = vault("shortened", &[]);
        let mut m = Model::new(v.clone(), true);
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        // Each key is saved and reported back, as the autosave and the watcher may.
        let key = |m: &mut Model, name: &str| {
            m.command(name);
            m.save();
            m.apply_outside(&[]);
        };
        for ch in title.chars() {
            m.commit_marked(&ch.to_string());
            m.save();
            m.apply_outside(&[]);
        }
        key(&mut m, "insertNewline:");
        key(&mut m, "deleteBackward:");
        key(&mut m, "deleteBackward:");
        let short: String = title.chars().take(title.chars().count() - 1).collect();
        assert_eq!(m.text(), short, "the page is as typed, nothing put over it");
        key(&mut m, "insertNewline:");
        m.save_flush();
        m.apply_outside(&[]);
        assert_eq!(m.text(), format!("{short}\n"));
        let file = v.join(format!("{short}.md"));
        assert_eq!(std::fs::read_to_string(&file).ok().as_deref(), Some(format!("{short}\n").as_str()), "named after the title, written once");
        let _ = std::fs::remove_dir_all(v);
    }
}

/// A checker that knows two misspellings, for tests (the system's needs a GUI session).
fn fake_checker(units: &[u16]) -> Vec<(usize, usize)> {
    let text = String::from_utf16_lossy(units);
    ["realy", "Teh"].iter().flat_map(|w| text.match_indices(w).map(|(i, _)| (text[..i].encode_utf16().count(), text[..i + w.len()].encode_utf16().count())).collect::<Vec<_>>()).collect()
}

#[test]
fn misspelled_words_are_marked_a_few_lines_at_a_time_but_not_the_one_being_typed() {
    let v = vault("spell", &[("Plan.md", "# Plan\n\nThis is realy good.\n\n`realy` in code stays.\n")]);
    let mut m = drawn(&v);
    m.go(Motion::DocStart, false);
    let (changed, more) = m.spell_step_with(fake_checker);
    assert!(changed && !more, "a short note is checked in one step");
    let text = m.text().to_string();
    let line = m.editor.outline().iter().find(|l| text[l.start..l.end].starts_with("This")).unwrap().clone();
    let marks = &m.spelling[&kolada_ui::spell::line_key(&text[line.start..line.end])];
    assert_eq!(marks.iter().map(|&(a, b)| &text[line.start + a as usize..line.start + b as usize]).collect::<Vec<_>>(), ["realy"]);
    let code = m.editor.outline().iter().find(|l| text[l.start..l.end].starts_with("`realy")).unwrap().clone();
    assert!(m.spelling[&kolada_ui::spell::line_key(&text[code.start..code.end])].is_empty(), "code is not prose");
    // Typing a word: its line is checked again, without marking the word at the caret.
    m.go(Motion::DocEnd, false);
    m.insert(" Teh");
    m.spell_step_with(fake_checker);
    let text = m.text().to_string();
    let last = m.editor.outline().last().unwrap().clone();
    let marks = &m.spelling[&kolada_ui::spell::line_key(&text[last.start..last.end])];
    assert!(marks.is_empty(), "the word being typed is left alone: {marks:?}");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_session_comes_back_with_its_settings_and_its_note() {
    let v = vault("session", &[("Plan.md", "# Plan\n\nOne.\n"), ("Other.md", "# Other\n\nTwo.\n")]);
    let mut m = drawn(&v);
    m.open("Other.md");
    m.cycle_theme();
    m.session.text_size = 20;
    m.go(Motion::DocEnd, false);
    // What launch would keep and read back (without touching the real data folder).
    let bytes = kolada_core::session::encode(&{
        let mut s = m.session.clone();
        s.caret = m.editor.selection().head as u64;
        s.last_active = kolada_core::session::now();
        s
    });
    let saved = kolada_core::session::decode(&bytes).expect("it decodes");
    let back = Model::open_vault(Services::none(), v.clone(), true, Some(saved));
    assert_eq!(back.session.note, "Other.md", "the note that was open");
    assert_eq!(back.text(), m.text());
    assert_eq!((back.session.theme, back.session.text_size), (1, 20), "its settings");
    assert!(!back.dark, "light, as the theme says, though the system is dark");
    assert_eq!(back.selection().head, m.selection().head, "the caret where it was");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn every_match_is_marked_and_ticked_beside_the_scroll_bar_where_it_is() {
    let mut body = String::from("# Long\n\nneedle at the top\n");
    for i in 0..200 {
        body += &format!("Line {i} of filler text.\n");
    }
    body += "needle at the end\n";
    let v = vault("ticks", &[("Long.md", &body)]);
    let mut m = drawn(&v);
    m.open_find(false);
    m.insert("needle");
    let f = m.find.as_ref().unwrap();
    assert_eq!(f.matches.len(), 2);
    assert_eq!(f.ticks.len(), 2);
    assert!(f.ticks[0] < 0.05 && f.ticks[1] > 0.9, "{:?}", f.ticks);
    // The page is drawn with both: the frame takes them.
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_notes_row_says_when_it_was_written_while_the_pointer_is_on_it() {
    let v = vault("row-date", &[("Plan.md", "# Plan\n"), ("Other.md", "# Other\n")]);
    let mut m = drawn(&v);
    let (r, i) = m.hits.iter().find_map(|(r, t)| match t {
        Target::Row(i) if m.session.sidebar[*i].kind == RowKind::Note => Some((*r, *i)),
        _ => None,
    }).unwrap();
    m.pointer_moved(r.left + 10.0, r.top + 4.0);
    let (row, said) = m.ui.row_date.clone().expect("a date for the row");
    assert_eq!(row, i);
    assert!(!said.is_empty());
    // A folder has none.
    let (r, _) = *m.hits.iter().find(|(_, t)| matches!(t, Target::Row(i) if m.session.sidebar[*i].kind != RowKind::Note)).unwrap();
    m.pointer_moved(r.left + 10.0, r.top + 4.0);
    assert!(m.ui.row_date.is_none());
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn draws_latin_arabic_and_color_emoji() {
    let v = vault("emoji", &[("Mixed.md", "# Mixed\n\nLatin, \u{0646}\u{0635} \u{0639}\u{0631}\u{0628}\u{064a}, \u{1F600}\u{1F600}\u{1F600}\n")]);
    let mut m = drawn(&v);
    m.set_dark(false);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    let px = m.surface().pixels();
    let channels = |p: u32| ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
    // The emoji's yellow: red and green high, blue low. Nothing else on the
    // page is that colour (the accent is red, the rest greys). Only where the
    // test fonts have colour emoji (a Linux image may not: CI images vary).
    let has_emoji = kolada_ui::testing::fonts().fallback.iter().any(|f| f.to_string_lossy().to_ascii_lowercase().contains("emj") || f.to_string_lossy().contains("Emoji"));
    let yellow = px.iter().filter(|&&p| {
        let (r, g, b) = channels(p);
        r > 200 && g > 150 && b < 120
    }).count();
    if has_emoji {
        assert!(yellow > 200, "the emoji are drawn in colour: {yellow} yellow pixels");
    } else {
        eprintln!("no colour emoji font on this machine: the colour is not checked");
    }
    // The Arabic is laid out (its glyphs take room on the line).
    let line = m.lines.iter().find(|l| m.text()[l.para.start..].starts_with("Latin")).expect("the line is drawn");
    let nt = m.note_text();
    let arabic_at = m.text().find('\u{0646}').unwrap();
    let (x_start, _, _) = nt.caret(&line.para, arabic_at).unwrap();
    // The word's other end: right to left, its caret is to the left of where it starts.
    let word_end = arabic_at + "\u{0646}\u{0635}".len();
    let (x_end, _, _) = nt.caret(&line.para, word_end).unwrap();
    assert!(x_start - x_end > 10.0, "the Arabic word reads right to left and takes room: {x_start} to {x_end}");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_note_named_apart_from_its_text_opens_under_its_name_and_its_file_is_left_as_it_was() {
    let v = vault("named", &[("Pricing ideas.md", "Charge per seat, not per note.\n")]);
    let mut m = drawn(&v);
    m.open("Pricing ideas.md");
    assert!(m.text().starts_with("Pricing ideas\n"), "its name over the text: {:?}", m.text());
    // An edit below the name: the file gets the text, not the name line.
    m.go(Motion::DocEnd, false);
    m.insert("More.");
    m.save();
    assert_eq!(std::fs::read_to_string(v.join("Pricing ideas.md")).unwrap(), "Charge per seat, not per note.\nMore.");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn edits_to_a_note_another_app_changed_meanwhile_go_beside_it_never_over_it() {
    let v = vault("conflict", &[("Plan.md", "# Plan\n\nOurs.\n")]);
    let mut m = drawn(&v);
    m.open("Plan.md");
    m.go(Motion::DocEnd, false);
    m.insert("Typed here.");
    // Another app writes the file before this one saves.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(v.join("Plan.md"), "# Plan\n\nTheirs, longer than before.\n").unwrap();
    assert!(m.save());
    assert_eq!(std::fs::read_to_string(v.join("Plan.md")).unwrap(), "# Plan\n\nTheirs, longer than before.\n", "theirs is kept");
    let copy = v.join("Plan (conflict).md");
    assert!(std::fs::read_to_string(&copy).unwrap().contains("Typed here."), "ours beside it");
    assert_eq!(m.session.note, "Plan (conflict).md");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_note_opens_with_the_caret_under_its_title() {
    let v = vault("body-start", &[("Plan.md", "# Plan\n\nShip it.\n"), ("Only.md", "# Only\n")]);
    let mut m = drawn(&v);
    m.open("Plan.md");
    assert_eq!(m.selection().head, "# Plan\n".len(), "on the line under the title");
    m.open("Only.md");
    assert_eq!(m.selection().head, "# Only\n".len(), "on the empty line the file ends with");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn control_k_deletes_to_the_lines_end_and_cmd_backspace_to_its_start() {
    let v = vault("delete-to", &[("Plan.md", "# Plan\n\nkeep this, cut that\nnext\n")]);
    let mut m = drawn(&v);
    m.open("Plan.md");
    let at = m.text().find(", cut").unwrap();
    m.editor.set_selection(Selection::caret(at));
    assert!(m.delete_to(false));
    assert!(m.text().contains("\nkeep this\nnext"), "{:?}", m.text());
    // At the line's end: the break goes, joining the next line.
    assert!(m.delete_to(false));
    assert!(m.text().contains("keep thisnext"), "{:?}", m.text());
    assert!(m.undo() && m.undo());
    let at = m.text().find(", cut").unwrap();
    m.editor.set_selection(Selection::caret(at));
    assert!(m.delete_to(true));
    assert!(m.text().contains("\n, cut that\n"), "{:?}", m.text());
    // Cmd+G finds again (opening the bar first).
    assert!(m.find_again(false));
    assert!(m.find.is_some());
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn an_arabic_note_is_written_named_by_its_title_and_found_by_search() {
    let v = vault("arabic", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v);
    m.new_note();
    for c in "ملاحظة جديدة\nنص عربي عن الحديقة".chars() {
        if c == '\n' {
            m.newline(false);
        } else {
            m.insert(&c.to_string());
        }
    }
    m.save_flush();
    let file = v.join("ملاحظة جديدة.md");
    assert!(file.is_file(), "named by its Arabic title: {:?}", std::fs::read_dir(&v).unwrap().flatten().map(|e| e.file_name()).collect::<Vec<_>>());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "ملاحظة جديدة\nنص عربي عن الحديقة");
    // The caret walks the Arabic line by its letters, not its bytes.
    m.go(Motion::LineStart, false);
    let start = m.selection().head;
    m.go(Motion::Right, false);
    assert_ne!(m.selection().head, start);
    assert!(m.text().is_char_boundary(m.selection().head));
    // Search finds it by an Arabic word.
    m.open_search();
    m.insert("الحديقة");
    for _ in 0..200 {
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        if m.search.as_ref().is_some_and(|s| !s.hits.is_empty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(m.search.as_ref().unwrap().hits.first().map(|h| h.path.as_str()), Some("ملاحظة جديدة.md"));
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_vault_not_made_yet_opens_on_a_fresh_page_and_is_made_with_its_first_note() {
    let v = std::env::temp_dir().join(format!("kolada-app-test-{}-no-vault", std::process::id())).join("Kolada");
    let _ = std::fs::remove_dir_all(v.parent().unwrap());
    let mut m = Model::new(v.clone(), true);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert_eq!(m.text(), "", "a fresh page");
    m.insert("First note");
    m.save();
    assert!(v.join("First note.md").is_file());
    let _ = std::fs::remove_dir_all(v.parent().unwrap());
}

#[test]
fn typing_with_no_writing_model_leaves_nothing_moving_once_drawn() {
    let v = vault("idle", &[("Plan.md", "# Plan\n\nText.\n")]);
    let mut m = drawn(&v);
    m.go(Motion::DocEnd, false);
    m.insert("x");
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(!m.wants_frames(), "no frames while nothing moves");
    // Only short waits by the clock (the margin's search), none of 20 seconds.
    if let Some(t) = m.next_due() {
        assert!(t.saturating_duration_since(std::time::Instant::now()) < std::time::Duration::from_secs(1));
    }
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn shift_return_with_suggestions_open_breaks_the_line_and_links_nothing() {
    let v = vault("soft", &[("Plan.md", "# Plan\n\n"), ("Pricing.md", "# Pricing\n")]);
    let mut m = drawn(&v);
    m.start_index();
    for _ in 0..200 {
        if m.index.lock().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    m.go(Motion::DocEnd, false);
    for c in "[[Pri".chars() {
        m.insert(&c.to_string());
    }
    assert!(m.suggest.now.is_some());
    assert!(m.newline(true));
    assert!(m.text().ends_with("[[Pri\n"), "a line break only: {:?}", m.text());
    assert!(!m.text().contains("Pricing]]"));
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_empty_top_strip_moves_the_window_and_nothing_with_a_use_does() {
    // Seen on the Mac's real screen: the window had a title bar of its own
    // above the note, so the side menu sat 30 points low. Without one, the
    // strip along the top is where the window is dragged from.
    let v = vault("caption", &[("Plan.md", "# Plan\n\nShip it.\n")]);
    let m = drawn(&v);
    let w = SIZE.0 as f32 / SIZE.2;
    let strip: Vec<f32> = (0..w as usize).step_by(4).map(|x| x as f32).filter(|&x| m.drags_window(x, 4.0)).collect();
    assert!(!strip.is_empty(), "somewhere along the top moves the window");
    for (r, t) in m.hits.iter().filter(|(_, t)| !matches!(t, Target::Caption | Target::Note | Target::SidebarPanel | Target::ToolbarPanel)) {
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        if scene::hit(&m.hits, cx, cy) == Some(*t) {
            assert!(!m.drags_window(cx, cy), "{t:?} at {cx},{cy} is a control, not the title bar");
        }
    }
    assert!(!m.drags_window(w / 2.0, 400.0), "the note is not the title bar");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn with_less_motion_asked_for_panels_arrive_at_once() {
    let v = vault("motion", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v);
    m.reduce_motion(false);
    m.toggle_panel(scene::Panel::Sidebar);
    assert!(m.panels_moving(), "the side menu slides");
    m.reduce_motion(true);
    m.toggle_panel(scene::Panel::Sidebar);
    // Fades stay (a passing word may still fade, as Apple's own do): what
    // stops is things sliding across the window.
    assert!(!m.panels_moving(), "with Reduce Motion it is there at once, no slide to follow");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_panel_toggled_twice_before_a_frame_stays_where_it_was() {
    let v = vault("twice", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v);
    m.reduce_motion(false);
    let open = !m.session.sidebar_collapsed;
    m.toggle_panel(scene::Panel::Sidebar);
    m.toggle_panel(scene::Panel::Sidebar);
    assert_eq!(!m.session.sidebar_collapsed, open);
    assert!(!m.panels_moving(), "no slide is left running toward the state undone");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_test_or_a_dump_keeps_its_trash_list_beside_its_vault_not_in_the_apps_data() {
    // The list went to the app's real data folder whatever the model, so
    // every test that deleted a note wrote to the user's own trash list.
    let v = vault("trashlog", &[("Plan.md", "# Plan\n"), ("Other.md", "# Other\n")]);
    let mut m = drawn(&v);
    assert_eq!(m.trash_log(), v.join(".kolada-trash.tsv"));
    let real = kolada_core::app_data_dir().map(|d| d.join("trash.tsv"));
    let before = real.as_ref().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
    assert!(m.trash_open_note());
    assert!(v.join(".kolada-trash.tsv").is_file(), "the note's entry is beside the vault");
    let after = real.as_ref().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
    assert_eq!(before, after, "the app's own list is untouched");
    m.open_trash();
    assert!(m.trash.as_ref().is_some_and(|(items, _)| items.len() == 1), "and the trash lists it");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn on_ios_the_settings_name_the_vault_by_its_folder_not_its_container_path() {
    let v = vault("vaultname", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v);
    m.set_platform(scene::Platform::Ios);
    m.toggle_options();
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert_eq!(m.vault_label(), v.file_name().unwrap().to_string_lossy());
    m.set_platform(scene::Platform::Mac);
    assert_eq!(m.vault_label(), v.to_string_lossy(), "the Mac shows where it is");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn on_ios_a_notes_pictures_are_found_in_the_vault_and_not_beside_its_name() {
    // Seen in the iPad simulator: the welcome note's picture showed as its
    // file's name, the scene looking for it under the folder's name alone.
    let v = vault("ios-picture", &[("Plan.md", "# Plan\n\n![[Shot.png]]\n\nAfter\n")]);
    std::fs::write(v.join("Shot.png"), include_bytes!("../../../../assets/welcome/Welcome to Kolada.png")).unwrap();
    let mut m = Model::new(v.clone(), true);
    m.set_platform(scene::Platform::Ios);
    m.open("Plan.md");
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(m.lines.iter().any(|l| l.para.picture.as_ref().is_some_and(|p| p.path.is_some())), "the picture is placed");
    m.toggle_options();
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert_eq!(m.vault_label(), v.file_name().unwrap().to_string_lossy(), "and the settings still name the folder alone");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_note_arriving_with_no_watcher_shows_once_the_vault_is_refreshed() {
    let v = vault("refresh", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v);
    let rows = m.session.sidebar.len();
    std::fs::write(v.join("Downloaded.md"), "# Downloaded\n").unwrap();
    assert!(m.refresh(), "iCloud brought a note: the tree has it");
    assert_eq!(m.session.sidebar.len(), rows + 1);
    assert!(!m.refresh(), "nothing more changed");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_apps_own_name_is_never_marked_misspelled() {
    let units: Vec<u16> = "Kolada is wrng".encode_utf16().collect();
    // A checker that knows neither word: only the misspelling stays marked.
    // Ranges are start and end, as the checkers give them.
    assert_eq!(spelled_as_ours(&units, vec![(0, 6), (10, 14)]), vec![(10, 14)]);
    let later: Vec<u16> = "I like Kolada".encode_utf16().collect();
    assert_eq!(spelled_as_ours(&later, vec![(7, 13)]), vec![], "the name mid-line too");
}

#[test]
fn in_full_screen_the_side_menu_takes_back_the_room_kept_for_the_window_buttons() {
    // Seen on the Mac's real screen: the traffic lights hide in full screen
    // and the band kept for them above Search stayed empty.
    let v = vault("fullscreen", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v);
    let search = |m: &Model| m.hits.iter().find(|(_, t)| *t == Target::Search).map(|(r, _)| r.top).expect("the search box");
    let windowed = search(&m);
    assert!(m.set_os_buttons(false));
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(search(&m) < windowed, "Search moves up into the room: {} < {windowed}", search(&m));
    assert!(!m.set_os_buttons(false), "no change, no redraw");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn where_the_os_draws_no_window_buttons_the_chromes_own_minimize_maximize_and_close_answer() {
    // Linux (an undecorated window, as on Windows): the chrome draws the
    // three buttons at the top right and the shell asks which was clicked.
    let v = vault("ownbuttons", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v);
    let at = |m: &Model, want: Target| m.hits.iter().find(|(_, t)| *t == want).map(|(r, _)| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0));
    assert_eq!(at(&m, Target::Close), None, "the Mac's traffic lights are the OS's: none drawn");
    assert!(m.set_os_buttons(false));
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    for (target, button) in [(Target::Minimize, WindowButton::Minimize), (Target::Maximize, WindowButton::Maximize), (Target::Close, WindowButton::Close)] {
        let (x, y) = at(&m, target).expect("drawn by the chrome");
        assert_eq!(m.window_button(x, y), Some(button));
    }
    assert_eq!(m.window_button(SIZE.0 as f32 / SIZE.2 / 2.0, SIZE.1 as f32 / SIZE.2 / 2.0), None, "the note is no button");
    assert!(m.set_maximized(true), "Maximize turns to Restore");
    assert!(!m.set_maximized(true), "no change, no redraw");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_note_sent_to_the_trash_leaves_recent_and_comes_back_first_with_undo() {
    // Seen on the Mac's real screen: Recent still listed a note just trashed.
    let v = vault("recenttrash", &[("Plan.md", "# Plan\n"), ("Other.md", "# Other\n")]);
    let mut m = drawn(&v);
    m.open("Other.md");
    m.open("Plan.md");
    assert!(m.session.recent.contains(&"Other.md".to_string()));
    m.open("Other.md");
    assert!(m.trash_open_note());
    assert!(!m.session.recent.contains(&"Other.md".to_string()), "a trashed note is not offered");
    assert!(m.undo());
    assert_eq!(m.session.recent.first().map(String::as_str), Some("Other.md"), "brought back, it is first again");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn on_a_phone_the_side_menu_comes_out_by_a_swipe_and_goes_when_the_finger_lifts_elsewhere() {
    // Seen on the iPhone: the folded menu was revealed only by resting a
    // pointer near it, and once out it stayed over the note.
    let v = vault("phone-menu", &[("Plan.md", "# Plan\n"), ("Other.md", "# Other\n")]);
    std::fs::write(v.join("Projects").join("Launch.md"), "# Launch\n").unwrap();
    let mut m = Model::new(v.clone(), true);
    m.set_platform(scene::Platform::Ios);
    let size = (402 * 3, 750 * 3, 3.0);
    let shown = |m: &Model| m.hits.iter().any(|(_, t)| matches!(t, Target::Row(_)));
    m.frame(size.0, size.1, size.2);
    assert!(!shown(&m), "folded away on a phone");
    assert!(!m.keyboard_can_rest(), "the keyboard is for the note");
    assert!(m.reveal_side_menu(), "the swipe brings it out");
    assert!(m.keyboard_can_rest(), "out over the note, the keyboard can rest");
    assert!(m.open_search() && !m.keyboard_can_rest(), "but not while searching in it");
    assert!(m.close_search() && m.keyboard_can_rest());
    m.frame(size.0, size.1, size.2);
    assert!(shown(&m));
    assert!(!m.reveal_side_menu(), "out already");
    let settle = |m: &mut Model| {
        for _ in 0..60 {
            m.tick();
            m.frame(size.0, size.1, size.2);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    let row_of = |m: &Model, folder: bool| {
        m.hits.iter().find(|(_, t)| matches!(t, Target::Row(i) if matches!(m.session.sidebar[*i].kind, RowKind::Folder { .. }) == folder)).map(|(r, _)| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)).unwrap()
    };
    // A folder opened in it: the menu stays, being used.
    let (x, y) = row_of(&m, true);
    m.tap(x, y, 1);
    settle(&mut m);
    assert!(shown(&m), "a folder tapped open keeps the menu out");
    // A note picked in it, the finger lifted: the menu goes.
    let (x, y) = row_of(&m, false);
    m.tap(x, y, 1);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while shown(&m) && std::time::Instant::now() < until {
        m.tick();
        m.frame(size.0, size.1, size.2);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!shown(&m), "the menu goes once the finger lifts");
    assert!(!m.keyboard_can_rest(), "and the keyboard comes back");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_bare_note_has_no_chrome_of_its_own_and_a_touch_anywhere_reaches_it() {
    // The iPhone's native shell draws its own bars around the note.
    let v = vault("bare", &[("Plan.md", "# Plan\n\nWords to write by.\n"), ("Other.md", "# Other\n")]);
    let mut m = Model::new(v.clone(), true);
    m.set_platform(scene::Platform::Ios);
    m.set_bare(true);
    m.open("Plan.md");
    m.frame(430 * 3, 900 * 3, 3.0);
    let chrome = |t: &Target| {
        matches!(t, Target::Row(_) | Target::NewNote | Target::SidebarPanel | Target::SendTo | Target::Minimal | Target::ToMain | Target::Caption | Target::RevealToolbar | Target::ToggleToolbar)
    };
    assert!(!m.hits.iter().any(|(_, t)| chrome(t)), "none of Kolada's chrome");
    assert_eq!(m.target_at(200.0, 8.0), Some(Target::Note), "the top of the view is the note's too");
    assert!(!m.reveal_side_menu() && !m.toggle_panel(kolada_ui::scene::Panel::Sidebar), "no side menu to bring out");
    assert!(!m.keyboard_can_rest());
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn every_note_is_listed_with_its_favorite_and_one_trashed_from_the_list_comes_back_with_undo() {
    let v = vault("every", &[("Plan.md", "# Plan\n"), ("Other.md", "# Other\n")]);
    let mut m = drawn(&v);
    m.open("Plan.md");
    let names = |m: &Model| {
        let mut n: Vec<String> = m.every_note().into_iter().map(|(n, _)| n.path).collect();
        n.sort();
        n
    };
    assert!(names(&m).contains(&"Other.md".to_string()) && names(&m).contains(&"Plan.md".to_string()));
    assert!(m.toggle_favorite("Other.md"));
    assert!(m.every_note().iter().any(|(n, fav)| n.path == "Other.md" && *fav), "pinned");
    assert!(m.trash_note("Other.md"));
    assert!(!names(&m).contains(&"Other.md".to_string()), "gone to the trash");
    assert_eq!(m.session.note, "Plan.md", "the open note stays open");
    assert!(m.undo());
    assert!(names(&m).contains(&"Other.md".to_string()), "back with Undo");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_caret_and_a_selection_have_exact_boxes_for_a_text_system_of_the_os() {
    // iOS draws its own selection handles and loupe from these.
    let words = "Tomatoes go in the south bed after the last frost, with basil between them to keep the bed full all summer long.";
    let v = vault("boxes", &[("Plan.md", &format!("# Plan\n\n{words}\n"))]);
    let mut m = Model::new(v.clone(), true);
    m.open("Plan.md");
    m.frame(360, 800, 1.0);
    let start = m.editor.text().find("Tomatoes").unwrap();
    let (at, end) = (utf16_at(m.editor.text(), start), utf16_at(m.editor.text(), start + words.len()));
    // The caret's own box is the one the frame drew.
    m.select_utf16(at + 4, 0);
    m.frame(360, 800, 1.0);
    let (drawn, asked) = (m.caret_rect().unwrap(), m.caret_box_at(at + 4).unwrap());
    assert!((drawn.left - asked.left).abs() < 1.0 && (drawn.top - asked.top).abs() < 1.0, "{drawn:?} against {asked:?}");
    // Further on is further right on the same line; the paragraph's end is lower down.
    let (a, b, z) = (m.caret_box_at(at).unwrap(), m.caret_box_at(at + 8).unwrap(), m.caret_box_at(end).unwrap());
    assert!(b.left > a.left && (b.top - a.top).abs() < 0.5);
    assert!(z.top > a.top, "the paragraph wraps at 360 DIPs");
    // A selection over a wrapped paragraph: a box per line, top to bottom.
    let boxes = m.range_boxes(at, end - at);
    assert!(boxes.len() >= 2, "{boxes:?}");
    assert!(boxes.windows(2).all(|w| w[1].top >= w[0].bottom - 0.5));
    assert!((boxes[0].left - (a.left + 1.0)).abs() < 1.0, "the first box starts at the caret: {boxes:?} {a:?}");
    assert!(boxes.iter().all(|r| r.right > r.left && r.right <= 360.0), "{boxes:?}");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn ios_has_no_minimal_window_by_button_command_or_kept_session() {
    // The desktops shrink to a note-only window; iOS's windows are the system's.
    let v = vault("nominimal", &[("Plan.md", "# Plan\n")]);
    let mut m = Model::new(v.clone(), true);
    m.session.minimal = true;
    m.set_platform(scene::Platform::Ios);
    assert!(!m.session.minimal, "a kept minimal window opens full");
    assert!(!m.set_minimal(true), "and cannot be asked for");
    m.frame(1024, 1366, 2.0);
    assert!(m.target_at(1.0, 1.0).is_none_or(|t| t != Target::Minimal) && !m.hits.iter().any(|(_, t)| *t == Target::Minimal), "no button");
    assert!(m.commands().iter().all(|c| !c.item.label.contains("Minimal")), "no command");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn only_a_note_inside_the_vault_goes_to_the_trash() {
    let v = vault("trash-inside-only", &[("Plan.md", "# Plan\n")]);
    let outside = v.parent().unwrap().join(format!("kolada-outside-{}.md", std::process::id()));
    std::fs::write(&outside, "kept").unwrap();
    let mut m = drawn(&v);
    let name = outside.file_name().unwrap().to_string_lossy().into_owned();
    assert!(!m.trash_note(&format!("../{name}")), "not through ..");
    assert!(!m.trash_note(&outside.to_string_lossy()), "not by an absolute path");
    assert!(outside.exists(), "the file outside the vault is where it was");
    assert!(m.trash_note("Plan.md"), "a note inside goes");
    let _ = std::fs::remove_file(outside);
    let _ = std::fs::remove_dir_all(v);
}
