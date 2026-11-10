use super::*;
use kolada_ui::geom::Ink;

/// A vault of its own, of made-up notes.
fn vault(test: &str, notes: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kolada-app-found-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in notes {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

const SIZE: (u32, u32, f32) = (2000, 1440, 2.0);

fn frame(m: &mut Model) {
    m.frame(SIZE.0, SIZE.1, SIZE.2);
}

/// The search open, the index read, `query` typed.
fn search(m: &mut Model, query: &str) {
    assert!(m.open_search());
    for _ in 0..300 {
        if m.index.lock().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    m.insert(query);
    frame(m);
}

fn rect_of(m: &Model, t: Target) -> Option<Rect> {
    m.hits.iter().find(|(_, h)| *h == t).map(|(r, _)| *r)
}

fn click(m: &mut Model, t: Target) {
    let r = rect_of(m, t).unwrap_or_else(|| panic!("{t:?} is drawn"));
    m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false);
    m.pointer_up();
    frame(m);
}

/// A long note with "lemon" far down it, twice, and once more in other words.
fn long_note() -> String {
    let mut text = String::from("# Kitchen\n\n");
    for i in 0..140 {
        let line = match i {
            90 => "The Lemon tart wants two lemons and a slow oven.".to_string(),
            104 => "Zest a LEMON over the top.".to_string(),
            _ => format!("Paragraph {i} about bread, flour and a warm oven."),
        };
        text.push_str(&line);
        text.push_str("\n\n");
    }
    text
}

/// The tints drawn in the last frame, in the note's column.
fn tints(m: &Model) -> Vec<Rect> {
    let s = m.kept_scene.as_ref().expect("the scene is kept");
    s.fills.iter().filter(|f| f.ink == Ink::Mark && f.rect.left >= m.ui.column.left - 2.0 && f.rect.right <= m.ui.column.right + 2.0).map(|f| f.rect).collect()
}

#[test]
fn opening_from_search_puts_the_first_match_a_little_above_the_middle_every_match_tinted() {
    let v = vault("open", &[("Kitchen.md", &long_note()), ("Garden.md", "# Garden\n\nRaised beds.\n")]);
    let mut m = Model::new(v.clone(), true);
    m.access_kept = true;
    frame(&mut m);
    search(&mut m, "lemon");
    click(&mut m, Target::Result(0));
    assert_eq!(m.session.note, "Kitchen.md");
    let text = m.editor.text().to_string();
    let found: Vec<&str> = m.found_view().iter().map(|&(a, b)| &text[a..b]).collect();
    assert_eq!(found, ["Lemon", "lemon", "LEMON"], "every match, as written");
    let first = text.find("Lemon").unwrap();
    assert_eq!(m.editor.selection().head, first, "the caret at the first match");
    let c = m.caret.expect("the first match is drawn");
    let h = m.viewport.1;
    assert!(c.top >= h * 0.3 && c.top <= h * 0.5, "a little above the middle: its line at {} of {h}", c.top);
    // Its tint is on its line, across its letters (the caret before them).
    let tinted = tints(&m);
    assert!(tinted.len() >= 2, "the matches in sight are tinted: {tinted:?}");
    assert!(tinted.iter().any(|t| (t.top - (c.top + 1.0)).abs() < 2.0 && (t.left - (c.left - 1.0)).abs() < 2.0 && t.width() > 20.0), "{tinted:?} on {c:?}");
    // Drawn again with nothing done: the view stays where it went.
    let view = (m.scroll_line, m.scroll);
    frame(&mut m);
    assert_eq!((m.scroll_line, m.scroll), view);
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_tint_stays_through_clicks_and_caret_moves_and_goes_with_an_edit_another_note_or_esc() {
    let v = vault("lasts", &[("Kitchen.md", &long_note()), ("Garden.md", "# Garden\n\nRaised beds by the lemon tree.\n")]);
    let mut m = Model::new(v.clone(), true);
    frame(&mut m);
    search(&mut m, "lemon");
    let kitchen = |m: &Model| m.search.as_ref().unwrap().hits.iter().position(|h| h.path == "Kitchen.md").unwrap();
    let i = kitchen(&m);
    click(&mut m, Target::Result(i));
    assert_eq!((m.session.note.as_str(), m.found_view().len()), ("Kitchen.md", 3));
    assert_eq!(m.found_query(), Some("lemon"));
    // A click in the note, and the caret moving: kept.
    click(&mut m, Target::Note);
    assert_eq!(m.found_view().len(), 3, "a click in the note keeps it");
    m.go(Motion::Down, false);
    m.go(Motion::WordRight, true);
    frame(&mut m);
    assert_eq!(m.found_view().len(), 3, "so does the caret moving, selecting too");
    // The first edit: gone.
    m.insert("x");
    frame(&mut m);
    assert!(m.found_view().is_empty() && m.found.is_none(), "written in: no tint");
    // Again, then Esc.
    search(&mut m, "lemon");
    let i = kitchen(&m);
    click(&mut m, Target::Result(i));
    assert_eq!(m.found_view().len(), 3);
    assert!(m.escape());
    assert!(m.found.is_none(), "Esc ends it");
    assert!(m.escape() && m.search.is_none(), "and the next the search");
    // Again, then another note.
    search(&mut m, "lemon");
    let i = kitchen(&m);
    click(&mut m, Target::Result(i));
    assert_eq!(m.found_view().len(), 3);
    m.open("Garden.md");
    frame(&mut m);
    assert!(m.found.is_none(), "another note opened: the tint was the last one's");
    // The search's other result opened: that one's matches.
    m.close_search();
    search(&mut m, "lemon");
    let other = m.search.as_ref().unwrap().hits.iter().position(|h| h.path == "Garden.md").unwrap();
    click(&mut m, Target::Result(other));
    let text = m.editor.text().to_string();
    assert_eq!(m.found_view().iter().map(|&(a, b)| &text[a..b]).collect::<Vec<_>>(), ["lemon"]);
    // The search's ✕: search left, the tint with it.
    m.open_search();
    frame(&mut m);
    click(&mut m, Target::SearchClear);
    assert!(m.search.is_none() && m.found.is_none());
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn esc_takes_the_tint_first_then_the_search_or_the_grid() {
    let v = vault("esc", &[("Kitchen.md", &long_note()), ("Garden.md", "# Garden\n\nRaised beds by the lemon tree.\n")]);
    let mut m = Model::new(v.clone(), true);
    frame(&mut m);
    // The side menu's search, kept open beside the note it opened.
    search(&mut m, "lemon");
    let i = m.search.as_ref().unwrap().hits.iter().position(|h| h.path == "Kitchen.md").unwrap();
    click(&mut m, Target::Result(i));
    assert!(m.search.is_some() && m.found_view().len() == 3);
    assert!(m.command("cancelOperation:"));
    assert!(m.found.is_none() && m.search.is_some(), "one Esc: the tint, the search kept");
    assert!(m.command("cancelOperation:"));
    assert!(m.search.is_none(), "the next: the search");
    // The find bar opened over a tinted note closes before the tint, as on
    // Windows (Settings and Trash the same).
    search(&mut m, "lemon");
    let i = m.search.as_ref().unwrap().hits.iter().position(|h| h.path == "Kitchen.md").unwrap();
    click(&mut m, Target::Result(i));
    assert!(!m.found_view().is_empty());
    assert!(m.open_find(false) && m.find.is_some());
    assert!(m.command("cancelOperation:"));
    assert!(m.find.is_none() && !m.found_view().is_empty(), "one Esc: the find bar, the tint kept");
    assert!(m.command("cancelOperation:"));
    assert!(m.found_view().is_empty(), "the next: the tint");
    // The grid's boxes up over a note opened from its search.
    assert!(m.toggle_view());
    frame(&mut m);
    search(&mut m, "zest");
    for _ in 0..50 {
        frame(&mut m);
        if m.grid.state.as_ref().is_some_and(|g| g.notes.searching() && g.notes.count() > 0) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(m.newline(false), "Enter opens the chosen box");
    frame(&mut m);
    assert_eq!(m.found_view().len(), 1);
    if !m.grid_browsing() {
        assert!(m.grid_menu_key());
    }
    frame(&mut m);
    assert!(m.grid_browsing());
    assert!(m.command("cancelOperation:"));
    assert!(m.found.is_none() && m.grid_browsing(), "one Esc: the tint, the boxes kept");
    assert!(m.command("cancelOperation:"));
    assert!(!m.grid_browsing(), "the next: back to the note");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn arabic_and_mixed_notes_open_on_their_matches_however_the_words_were_typed() {
    // Made-up lines: a meeting with its vowel marks, one drawn out with a
    // tatweel, then the same word in a line of English.
    let note = "# \u{645}\u{644}\u{627}\u{62D}\u{638}\u{627}\u{62A}\n\n\u{645}\u{648}\u{639}\u{62F} \u{627}\u{644}\u{627}\u{62C}\u{652}\u{62A}\u{650}\u{645}\u{64E}\u{627}\u{639}\u{64F} \u{63A}\u{62F}\u{627}\n\n\u{627}\u{62C}\u{62A}\u{645}\u{640}\u{640}\u{627}\u{639} \u{627}\u{644}\u{641}\u{631}\u{64A}\u{642}\n\nThe \u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639} with the Kolada team is at ten.\n";
    let v = vault("arabic", &[("Notes.md", note), ("Other.md", "# Other\n\nNothing here.\n")]);
    let mut m = Model::new(v.clone(), true);
    m.access_kept = true;
    frame(&mut m);
    search(&mut m, "\u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639}");
    click(&mut m, Target::Result(0));
    assert_eq!(m.session.note, "Notes.md");
    let text = m.editor.text().to_string();
    let found: Vec<&str> = m.found_view().iter().map(|&(a, b)| &text[a..b]).collect();
    assert_eq!(found.len(), 3, "with its marks, drawn out, and among English words");
    assert!(found[0].contains('\u{64F}') && found[1].contains('\u{640}'), "the marks and the tatweel inside the tint");
    let tinted = tints(&m);
    assert_eq!(tinted.len(), 3, "all three in sight, tinted: {tinted:?}");
    for t in &tinted {
        assert!(t.width() > 10.0 && t.height() > 10.0, "{t:?}");
    }
    // The English word in the same line is found too (Esc: the tint, then
    // the search).
    m.escape();
    m.escape();
    search(&mut m, "kolada");
    click(&mut m, Target::Result(0));
    let text = m.editor.text().to_string();
    assert_eq!(m.found_view().iter().map(|&(a, b)| &text[a..b]).collect::<Vec<_>>(), ["Kolada"]);
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_match_only_in_the_title_tints_the_title_at_the_top() {
    let v = vault("title", &[("Plum jam.md", "# Plum jam\n\nSugar, lemons, a slow afternoon.\n"), ("Other.md", "# Other\n")]);
    let mut m = Model::new(v.clone(), true);
    m.access_kept = true;
    frame(&mut m);
    search(&mut m, "plum");
    click(&mut m, Target::Result(0));
    assert_eq!(m.session.note, "Plum jam.md");
    let text = m.editor.text().to_string();
    assert_eq!(m.found_view().iter().map(|&(a, b)| &text[a..b]).collect::<Vec<_>>(), ["Plum"]);
    assert_eq!((m.scroll_line, m.scroll), (0, 0.0), "at the top");
    assert!(m.editor.selection().head > text.find('\n').unwrap(), "the caret under the title, where writing starts");
    let title = m.lines.first().expect("drawn");
    assert!(tints(&m).iter().any(|t| t.top >= title.y - 1.0 && t.bottom <= title.y + title.height + 1.0), "the title tinted");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_shells_own_search_opens_a_note_on_its_match_too() {
    // A shell's own search opens through the model.
    let v = vault("native", &[("Kitchen.md", &long_note()), ("Garden.md", "# Garden\n")]);
    let mut m = Model::new(v.clone(), true);
    frame(&mut m);
    m.open_from_search("Kitchen.md", "zest");
    frame(&mut m);
    assert_eq!((m.session.note.as_str(), m.found_query()), ("Kitchen.md", Some("zest")));
    let c = m.caret.expect("drawn");
    assert!(c.top >= m.viewport.1 * 0.3 && c.top <= m.viewport.1 * 0.5, "a little above the middle: {}", c.top);
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_card_chosen_from_the_grids_search_opens_on_its_match() {
    let v = vault("grid", &[("Kitchen.md", &long_note()), ("Garden.md", "# Garden\n\nRaised beds.\n")]);
    let mut m = Model::new(v.clone(), true);
    frame(&mut m);
    assert!(m.toggle_view());
    frame(&mut m);
    search(&mut m, "zest");
    // The boxes follow the search once its answer is in.
    for _ in 0..50 {
        frame(&mut m);
        if m.grid.state.as_ref().is_some_and(|g| g.notes.searching() && g.notes.count() > 0) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(m.newline(false), "Enter opens the chosen box");
    frame(&mut m);
    assert_eq!(m.session.note, "Kitchen.md");
    let text = m.editor.text().to_string();
    assert_eq!(m.found_view().iter().map(|&(a, b)| &text[a..b]).collect::<Vec<_>>(), ["Zest"]);
    let c = m.caret.expect("drawn");
    assert!(c.top >= m.viewport.1 * 0.3 && c.top <= m.viewport.1 * 0.5, "a little above the middle: {}", c.top);
    let _ = std::fs::remove_dir_all(v);
}

/// A caught page found by its title: it opens at its top, the dates and
/// the frontmatter's one line drawn above the title, not scrolled past.
#[test]
fn a_caught_note_found_by_its_title_opens_with_its_line_and_dates_above_it() {
    let note = "---\nsource: https://kitchen.example/lemon-tart\ncaught: 2026-11-08T15:20\n---\n# Lemon tart with a crisp base\n\nFor Sunday lunch\n\nZest two lemons into the sugar.\n";
    let v = vault("caught", &[("Lemon tart with a crisp base.md", note), ("Other.md", "# Other\n")]);
    let mut m = Model::new(v.clone(), true);
    m.access_kept = true;
    frame(&mut m);
    search(&mut m, "lemon tart with");
    click(&mut m, Target::Result(0));
    frame(&mut m);
    assert_eq!(m.session.note, "Lemon tart with a crisp base.md");
    assert_eq!((m.scroll_line, m.scroll), (0, 0.0), "at its top, as a note opens");
    assert!(rect_of(&m, Target::Source).is_some(), "the frontmatter's line, its host to click");
    assert!(rect_of(&m, Target::PageDate).is_some(), "the dates above it");
    // A window just tall enough that the title at OPEN_AT would put the
    // view's top a little into the note's head: at its top all the same.
    let top = m.ui.column.top;
    let short = ((top + 10.0) / kolada_ui::found::OPEN_AT * SIZE.2).round() as u32;
    let mut n = Model::new(v.clone(), true);
    n.frame(SIZE.0, short, SIZE.2);
    // (Too short for the side menu's results: opened as a shell's own search opens it.)
    n.open_from_search("Lemon tart with a crisp base.md", "crisp base");
    n.frame(SIZE.0, short, SIZE.2);
    assert_eq!(n.session.note, "Lemon tart with a crisp base.md");
    assert_eq!((n.scroll_line, n.scroll), (0, 0.0), "at its top in a short window too");
    assert!(rect_of(&n, Target::Source).is_some() && rect_of(&n, Target::PageDate).is_some(), "the line and the dates in view");
    let _ = std::fs::remove_dir_all(v);
}

/// A match only in the frontmatter (its host, say): the page opens folded,
/// the caret past it, the view at the top.
#[test]
fn a_match_only_in_the_frontmatter_opens_the_page_folded_at_its_top() {
    let note = "---\nsource: https://kitchen.example/lemon-tart\ncaught: 2026-11-08T15:20\n---\n# Lemon tart with a crisp base\n\nFor Sunday lunch\n";
    let v = vault("in-front", &[("Lemon tart with a crisp base.md", note), ("Other.md", "# Other\n")]);
    let mut m = Model::new(v.clone(), true);
    frame(&mut m);
    search(&mut m, "kitchen");
    click(&mut m, Target::Result(0));
    frame(&mut m);
    assert_eq!(m.session.note, "Lemon tart with a crisp base.md");
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(note).collect();
    assert!(!kolada_ui::frontmatter::caret_in(&lines, m.editor.selection().head), "the caret past the frontmatter");
    assert_eq!((m.scroll_line, m.scroll), (0, 0.0), "at the top");
    assert!(rect_of(&m, Target::Source).is_some() && rect_of(&m, Target::PageDate).is_some(), "the one line and the dates above the title");
    let _ = std::fs::remove_dir_all(v);
}
