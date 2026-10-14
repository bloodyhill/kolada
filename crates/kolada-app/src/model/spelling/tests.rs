use super::*;
use kolada_editor::Motion;
use std::path::PathBuf;

/// A checker that knows two misspellings (the system's needs a GUI session).
fn fake_checker(units: &[u16]) -> Vec<(usize, usize)> {
    let text = String::from_utf16_lossy(units);
    ["realy", "Teh"].iter().flat_map(|w| text.match_indices(w).map(|(i, _)| (text[..i].encode_utf16().count(), text[..i + w.len()].encode_utf16().count())).collect::<Vec<_>>()).collect()
}

fn guesses(word: &str) -> Vec<String> {
    match word {
        "realy" => vec!["really".into(), "rely".into(), "realty".into()],
        _ => Vec::new(),
    }
}

/// Every line checked, as the app does a few at a time.
fn check_all(m: &mut Model) {
    while m.spell_step_with(fake_checker).1 {}
}

/// A model with "realy" (twice) and "Teh" marked, and where to click the first "realy".
fn marked(test: &str) -> (Model, PathBuf, (f32, f32)) {
    let dir = std::env::temp_dir().join(format!("kolada-app-spelling-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Plan.md"), "# Plan\n\nThis is realy good, and realy fine.\n\nTeh end.\n").unwrap();
    let mut m = Model::new(dir.clone(), true);
    m.frame(2000, 1440, 2.0);
    // The caret in the first "realy" gives the place to click.
    let at = m.editor.text().find("realy").unwrap() + 2;
    m.editor.set_selection(Selection::caret(at));
    m.frame(2000, 1440, 2.0);
    let c = m.caret_rect().expect("the caret is drawn");
    // The caret goes away from the words, so none is the one being typed.
    m.go(Motion::DocEnd, false);
    m.editor.set_selection(Selection::caret(m.editor.text().len()));
    check_all(&mut m);
    m.frame(2000, 1440, 2.0);
    (m, dir, ((c.left + c.right) / 2.0, (c.top + c.bottom) / 2.0))
}

#[test]
fn a_misspelled_word_offers_the_dictionarys_guesses_then_ignore_and_learn() {
    let (mut m, dir, (x, y)) = marked("items");
    assert!(m.spell_menu_items_with(5.0, 5.0, guesses).is_none(), "not on the note");
    let items = m.spell_menu_items_with(x, y, guesses).expect("the word is misspelled");
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["really", "rely", "realty", "Ignore Spelling", "Learn Spelling"]);
    assert_eq!(items.iter().map(|i| i.divider).collect::<Vec<_>>(), [false, false, false, true, false]);
    assert_eq!(m.editor.selected_text(), "realy", "the word is selected, as in any Mac text field");
    // No guesses: just the two.
    let items = m.spell_menu_items_with(x, y, |_| Vec::new()).unwrap();
    assert_eq!(items.iter().map(|i| (i.label.as_str(), i.divider)).collect::<Vec<_>>(), [("Ignore Spelling", false), ("Learn Spelling", false)]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_guess_replaces_the_word_in_one_undo_step() {
    let (mut m, dir, (x, y)) = marked("replace");
    let before = m.text().to_string();
    m.spell_menu_items_with(x, y, guesses).unwrap();
    assert!(m.spell_pick_with(0, |_| panic!("nothing is learned")));
    assert_eq!(m.text(), before.replacen("realy", "really", 1), "the clicked word, not the other");
    assert!(m.dirty());
    m.undo();
    assert_eq!(m.text(), before, "one Cmd+Z takes it back");
    assert!(!m.spell_pick_with(0, |_| {}), "a menu is picked from once");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ignore_leaves_the_word_unmarked_everywhere_and_learn_teaches_the_dictionary() {
    let (mut m, dir, (x, y)) = marked("ignore");
    let count = |m: &Model| m.spelling.values().map(Vec::len).sum::<usize>();
    assert_eq!(count(&m), 3, "two realy and a Teh");
    m.spell_menu_items_with(x, y, guesses).unwrap();
    assert!(m.spell_pick_with(3, |_| panic!("ignoring teaches nothing")));
    assert_eq!(count(&m), 0, "every line is checked again");
    check_all(&mut m);
    assert_eq!(count(&m), 1, "only Teh is marked now: both realy are ignored");
    // Learn: the dictionary is asked once, and the marks are looked at again.
    let (mut other, dir2, (x, y)) = marked("learn");
    other.spell_menu_items_with(x, y, guesses).unwrap();
    let mut learned = Vec::new();
    assert!(other.spell_pick_with(4, |w| learned.push(w.to_string())));
    assert_eq!(learned, ["realy"]);
    assert!(other.spelling.is_empty());
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

#[test]
fn nothing_is_offered_where_spelling_is_off_or_the_word_changed_under_the_menu() {
    let (mut m, dir, (x, y)) = marked("off");
    m.session.spell = 1;
    assert!(m.spell_menu_items_with(x, y, guesses).is_none(), "spelling is not marked");
    m.session.spell = 0;
    m.spell_menu_items_with(x, y, guesses).unwrap();
    // The word is edited while the menu is open: the pick finds something else there.
    m.editor.set_selection(Selection { anchor: 0, head: 0 });
    m.insert("xx");
    assert!(!m.spell_pick_with(0, |_| {}), "the word moved: nothing is replaced");
    let _ = std::fs::remove_dir_all(dir);
}
