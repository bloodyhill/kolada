use super::*;
use kolada_convert::fixtures;
use kolada_ui::commands::Id;
use kolada_ui::scene::Setting;
use kolada_ui::welcome::Line;

const SIZE: (u32, u32, f32) = (1400, 1100, 1.0);

/// A first run's model: a vault not made yet under a folder of its own,
/// and the welcome up.
fn first_run(test: &str) -> (Model, PathBuf) {
    let root = std::env::temp_dir().join(format!("kolada-app-welcome-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut m = Model::new(root.join("Kolada"), true);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    m.first_run = true;
    m.start_arrivals(|| {});
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    (m, root)
}

fn click(m: &mut Model, target: Target) -> bool {
    let r = m.hits.iter().rev().find(|(_, t)| *t == target).map(|(r, _)| *r).unwrap_or_else(|| panic!("{target:?} is not on screen"));
    let changed = m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    changed
}

fn line(m: &Model) -> &Line {
    &m.welcome.as_ref().expect("the welcome shows").line
}

/// Waits for an import to come back.
fn brought(m: &mut Model) {
    let until = Instant::now() + Duration::from_secs(20);
    while m.bringing.is_some() && Instant::now() < until {
        m.poll_arrivals();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(m.bringing.is_none(), "it came back");
    m.frame(SIZE.0, SIZE.1, SIZE.2);
}

/// What the shell was asked to pick, taken.
fn picks(m: &mut Model) -> Vec<Pick> {
    m.take_requests().into_iter().filter_map(|r| if let Request::Pick(p) = r { Some(p) } else { None }).collect()
}

#[test]
fn a_first_run_opens_on_the_welcome_screen_and_writes_nothing() {
    let (m, root) = first_run("first");
    assert!(m.welcome_open());
    assert!(!root.join("Kolada").exists(), "no folder made, no note seeded");
    assert!(m.hits.iter().any(|(_, t)| *t == Target::Welcome(Hit::Start)));
    assert!(m.model_card.is_none(), "the writing model is offered on it, not on its card");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn each_tile_asks_the_shell_for_its_pick() {
    let (mut m, root) = first_run("tiles");
    for from in [From::Obsidian, From::Keep, From::Bear, From::Evernote, From::Notion, From::NotesApp, From::Folder, From::Documents] {
        click(&mut m, Target::Welcome(Hit::Tile(from)));
        assert_eq!(picks(&mut m), [Pick::Bring(from)], "{from:?}");
    }
    let apple = m.hits.iter().any(|(_, t)| *t == Target::Welcome(Hit::Tile(From::AppleNotes)));
    assert_eq!(apple, m.platform == scene::Platform::Mac, "Apple Notes is the Mac's");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn each_place_brings_its_notes_into_a_folder_of_their_own() {
    let (mut m, root) = first_run("paths");
    let from = root.join("from");
    std::fs::create_dir_all(from.join("vault/.obsidian")).unwrap();
    std::fs::write(from.join("vault/Ideas.md"), "# Ideas\n").unwrap();
    std::fs::write(from.join("vault/dot.png"), "PNG").unwrap();
    std::fs::create_dir_all(from.join("Research")).unwrap();
    std::fs::write(from.join("Research/a.md"), "a").unwrap();
    std::fs::write(from.join("Research/b.txt"), "b").unwrap();
    std::fs::write(from.join("My.enex"), fixtures::enex(4)).unwrap();
    std::fs::write(from.join("Export-1.zip"), fixtures::notion(3)).unwrap();
    std::fs::write(from.join("Notes.nbk"), include_bytes!("../../../../kolada-core/testdata/notes.nbk")).unwrap();
    std::fs::write(from.join("Page.html"), "<html><head><title>A page</title></head><body><h1>A page</h1><p>Made up.</p></body></html>").unwrap();
    std::fs::write(from.join("Loose.md"), "# Loose\n").unwrap();
    let vault = root.join("Kolada");
    let cases: [(From, Vec<PathBuf>, &str, usize); 6] = [
        (From::Obsidian, vec![from.join("vault")], "From Obsidian", 1),
        (From::Folder, vec![from.join("Research")], "Research", 2),
        (From::Evernote, vec![from.join("My.enex")], "From Evernote", 4),
        (From::Notion, vec![from.join("Export-1.zip")], "From Notion", 3),
        (From::NotesApp, vec![from.join("Notes.nbk")], "From Notes", 0),
        (From::Documents, vec![from.join("Page.html"), from.join("Loose.md")], "From documents", 2),
    ];
    for (tile, picked, folder, count) in cases {
        assert!(m.bring_in(tile, picked));
        assert!(matches!(line(&m), Line::Bringing { .. }));
        brought(&mut m);
        match line(&m) {
            Line::Brought { from, count: n, folder: f, failed, .. } => {
                assert_eq!((*from, f.as_str(), *failed), (tile, folder, 0), "{tile:?}");
                assert!(count == 0 || *n == count, "{tile:?}: {n} notes");
                assert!(*n > 0);
            }
            other => panic!("{tile:?}: {other:?}"),
        }
        assert!(vault.join(folder).is_dir(), "{tile:?}");
        assert!(m.welcome.as_ref().unwrap().brought.contains(&tile), "its tile is ticked");
    }
    assert!(vault.join("From Obsidian/dot.png").is_file() && !vault.join("From Obsidian/.obsidian").exists());
    assert!(m.session.sidebar.iter().any(|r| r.name == "From Evernote"), "the list shows what came in");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn stopped_part_way_nothing_is_kept_and_it_says_so() {
    let (mut m, root) = first_run("stop");
    let file = root.join("Big.enex");
    std::fs::write(&file, fixtures::enex(3000)).unwrap();
    assert!(m.bring_in(From::Evernote, vec![file]));
    assert!(!m.bring_in(From::Notion, vec![root.join("x.zip")]), "one at a time");
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(click(&mut m, Target::Welcome(Hit::Stop)));
    brought(&mut m);
    assert_eq!(*line(&m), Line::Stopped { from: From::Evernote, said: "Evernote".into() });
    assert!(!root.join("Kolada/From Evernote").exists(), "nothing kept");
    assert!(m.welcome.as_ref().unwrap().brought.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn while_it_brings_in_frames_come_to_say_how_far() {
    let (mut m, root) = first_run("progress");
    let file = root.join("Big.enex");
    std::fs::write(&file, fixtures::enex(2000)).unwrap();
    m.bring_in(From::Evernote, vec![file]);
    assert!(m.next_due().is_some(), "a frame is due while it works");
    let mut saw = false;
    let until = Instant::now() + Duration::from_secs(20);
    while m.bringing.is_some() && Instant::now() < until {
        std::thread::sleep(EVERY);
        m.tick();
        saw |= matches!(line(&m), Line::Bringing { total, .. } if *total == 2000);
        m.poll_arrivals();
    }
    assert!(saw, "it said how many");
    assert!(matches!(line(&m), Line::Brought { count: 2000, .. }));
    assert!(m.bringing_due().is_none(), "and then no more frames");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_file_it_cannot_read_is_said_and_nothing_is_made() {
    let (mut m, root) = first_run("bad");
    let file = root.join("Not really.enex");
    std::fs::write(&file, "hello").unwrap();
    m.bring_in(From::Evernote, vec![file]);
    brought(&mut m);
    assert!(matches!(line(&m), Line::Failed { from: From::Evernote, .. }), "{:?}", line(&m));
    assert_eq!(m.welcome.as_ref().unwrap().words(), "This isn't an export Kolada can read.");
    assert!(!root.join("Kolada/From Evernote").exists());
    // A Notion export on Keep's tile is not Keep's.
    let notion = root.join("Export-1.zip");
    std::fs::write(&notion, fixtures::notion(2)).unwrap();
    m.bring_in(From::Keep, vec![notion]);
    brought(&mut m);
    assert_eq!(m.welcome.as_ref().unwrap().words(), "This isn't an export Kolada can read.");
    let _ = std::fs::remove_dir_all(root);
}

/// Bear's bundles, zipped and dropped, are taken for Notion's by the name
/// but come in as Bear's; plain Markdown zipped is no app's, and comes in
/// as a folder named for its file.
#[test]
fn an_export_found_to_be_another_apps_is_said_as_it() {
    let (mut m, root) = first_run("found");
    let file = root.join("Bear notes.zip");
    std::fs::write(&file, fixtures::zip(&[("A made-up note.textbundle/text.markdown", b"# A made-up note\n\nSome words #tag\n")])).unwrap();
    assert!(m.drop_files(vec![file], None));
    brought(&mut m);
    match line(&m) {
        Line::Brought { from, folder, .. } => assert_eq!((*from, folder.as_str()), (From::Bear, "From Bear")),
        other => panic!("{other:?}"),
    }
    assert!(m.welcome.as_ref().unwrap().words().contains("from Bear"));
    let plain = root.join("My notes.zip");
    std::fs::write(&plain, fixtures::zip(&[("A made-up note.md", b"# A made-up note\n\nSome words\n")])).unwrap();
    assert!(m.drop_files(vec![plain], None));
    brought(&mut m);
    match line(&m) {
        Line::Brought { from, folder, .. } => assert_eq!((*from, folder.as_str()), (From::Folder, "My notes"), "not called Bear's"),
        other => panic!("{other:?}"),
    }
    assert!(m.welcome.as_ref().unwrap().words().contains("from My notes"));
    let _ = std::fs::remove_dir_all(root);
}

/// Opened again while Apple Notes comes in, it says so and waits.
#[test]
fn opened_again_while_apple_notes_comes_in_it_says_so() {
    let (mut m, root) = first_run("apple-again");
    m.escape();
    m.arrivals.apple_notes = super::super::applenotes::AppleNotes::Running;
    assert!(m.show_welcome());
    assert!(matches!(line(&m), Line::Bringing { from: From::AppleNotes, .. }));
    let _ = std::fs::remove_dir_all(root);
}

/// A phone's shell (the note bare in its own chrome, or iOS) has no screen
/// of tiles: a first run there writes the welcome note, as before.
#[test]
fn a_phone_gets_the_welcome_note_and_never_the_screen() {
    for (test, ios) in [("bare", false), ("ios", true)] {
        let root = std::env::temp_dir().join(format!("kolada-app-welcome-{}-phone-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let vault = root.join("Kolada");
        let mut m = Model::new(vault.clone(), true);
        if ios {
            m.set_platform(scene::Platform::Ios);
        } else {
            m.set_bare(true);
        }
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        m.first_run = true;
        m.start_arrivals(|| {});
        assert!(!m.welcome_open(), "{test}");
        let until = Instant::now() + Duration::from_secs(5);
        while !m.poll_arrivals() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(m.session.note, "Welcome to Kolada.md", "{test}");
        assert!(!m.show_welcome() && !m.welcome_open(), "nor from the palette");
        let _ = std::fs::remove_dir_all(root);
    }
}

/// The writing model's download strip keeps its Cancel over the welcome.
#[test]
fn the_download_strip_answers_over_it() {
    let (mut m, root) = first_run("strip");
    assert_eq!(m.welcome_click(Some(Target::Model(kolada_ui::modelcard::Hit::Cancel))), None, "handled where it always is");
    assert_eq!(m.welcome_click(Some(Target::Welcome(Hit::Page))), Some(false));
    let _ = std::fs::remove_dir_all(root);
}

/// A picture or a web page pasted under it: nothing pasted, and nothing
/// saved or fetched for it.
#[test]
fn a_paste_under_it_writes_nothing() {
    let (mut m, root) = first_run("paste");
    assert!(!m.paste_picture(b"\x89PNG\r\n\x1a\n"));
    assert!(!root.join("Kolada").exists(), "no folder made, no picture saved");
    assert!(!m.paste("words", || Some(("<p>A page <img src=\"https://example.com/a.png\"></p>".to_string(), None))));
    assert!(m.text().is_empty() && !root.join("Kolada").exists());
    let _ = std::fs::remove_dir_all(root);
}

/// A note opened from the Finder or Files shows, the welcome put away;
/// given before the first frame, no first run's welcome comes over it.
#[test]
fn a_note_opened_from_outside_shows_over_it() {
    let (mut m, root) = first_run("outside");
    let note = root.join("Elsewhere/A made-up note.md");
    std::fs::create_dir_all(note.parent().unwrap()).unwrap();
    std::fs::write(&note, "# A made-up note\n").unwrap();
    m.open_path(&note);
    assert!(!m.welcome_open());
    assert_eq!(m.session.note, "A made-up note.md");
    // Before arrivals start.
    let mut m = Model::new(root.join("Kolada"), true);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    m.first_run = true;
    m.open_path(&note);
    m.start_arrivals(|| {});
    assert!(!m.welcome_open(), "what was opened stays in sight");
    // A document opened over it: it goes, for the note the document becomes.
    let (mut m, root2) = first_run("outside-doc");
    let page = root2.join("Page.html");
    std::fs::write(&page, "<html><body><h1>A page</h1></body></html>").unwrap();
    m.open_path(&page);
    assert!(!m.welcome_open());
    // A path it refuses changes nothing.
    let (mut m, root3) = first_run("outside-refused");
    m.open_path(&root3.join("missing.md"));
    assert!(m.welcome_open());
    let _ = (std::fs::remove_dir_all(root2), std::fs::remove_dir_all(root3));
    let _ = std::fs::remove_dir_all(root);
}

/// While Apple Notes is brought in, a drop waits: one import at a time.
#[test]
fn a_drop_while_apple_notes_comes_in_waits() {
    let (mut m, root) = first_run("apple-busy");
    m.arrivals.apple_notes = super::super::applenotes::AppleNotes::Running;
    let file = root.join("Trip.enex");
    std::fs::write(&file, fixtures::enex(2)).unwrap();
    assert!(!m.drop_files(vec![file.clone()], None));
    assert!(!m.bring_in(From::Evernote, vec![file]));
    assert!(m.bringing.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn files_dropped_on_it_are_brought_in_by_what_they_are() {
    let (mut m, root) = first_run("drop");
    let file = root.join("Trip.enex");
    std::fs::write(&file, fixtures::enex(2)).unwrap();
    assert!(m.drop_files(vec![file], Some((400.0, 400.0))));
    brought(&mut m);
    assert!(matches!(line(&m), Line::Brought { from: From::Evernote, count: 2, .. }));
    // A folder with `.obsidian` in it is a vault.
    std::fs::create_dir_all(root.join("Vault/.obsidian")).unwrap();
    std::fs::write(root.join("Vault/a.md"), "a").unwrap();
    m.drop_files(vec![root.join("Vault")], None);
    brought(&mut m);
    assert!(matches!(line(&m), Line::Brought { from: From::Obsidian, .. }));
    assert!(m.text().is_empty(), "nothing went into the page under it");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn start_writing_or_esc_puts_it_away_for_a_fresh_page_and_typing_waits() {
    let (mut m, root) = first_run("start");
    assert!(!m.insert("x"), "nothing is typed under it");
    assert!(m.text().is_empty());
    assert!(m.escape());
    assert!(!m.welcome_open() && m.session.model_asked, "the writing model was asked");
    assert!(m.insert("x") && m.text() == "x", "the page takes the keys");
    // From the palette, again; Start writing puts it away.
    assert!(m.run_id(Id::Welcome));
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(m.welcome_open());
    click(&mut m, Target::Welcome(Hit::Start));
    assert!(!m.welcome_open());
    assert!(m.text().is_empty(), "a fresh page");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn its_switches_are_settings_own() {
    let (mut m, root) = first_run("switches");
    for s in [Setting::Assist, Setting::Beam] {
        assert!(scene::switch_on(s, m.option_values()[s.index()]), "{s:?} starts on");
        click(&mut m, Target::Switch(s));
        assert!(!scene::switch_on(s, m.option_values()[s.index()]), "{s:?} off");
    }
    assert_eq!((m.session.assist, m.session.beam), (1, 1), "written where Settings writes them");
    assert!(!m.hits.iter().any(|(_, t)| matches!(t, Target::Choice(Setting::Assist, _))), "Light | Full fold away");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn tab_walks_its_controls_and_enter_presses_the_one_it_came_to() {
    let (mut m, root) = first_run("keys");
    assert!(m.tab(false));
    assert_eq!(m.welcome.as_ref().unwrap().focus, Some(Target::Welcome(Hit::Tile(From::Obsidian))));
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    m.newline(false);
    assert_eq!(picks(&mut m), [Pick::Bring(From::Obsidian)], "Enter pressed it");
    assert!(m.text().is_empty(), "and no line went into the page");
    assert!(m.tab(true));
    assert_eq!(m.welcome.as_ref().unwrap().focus, Some(Target::Welcome(Hit::Start)), "Shift+Tab goes round to the last");
    m.insert(" ");
    assert!(!m.welcome_open(), "Space pressed Start writing");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_screen_reader_hears_each_control_by_name() {
    let (mut m, root) = first_run("names");
    m.keep_for_access(true);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    let controls = m.access_controls();
    let label = |t: Target| controls.iter().find(|c| c.target == t).map(|c| c.label.clone()).unwrap_or_else(|| panic!("{t:?} is not exposed"));
    assert_eq!(label(Target::Welcome(Hit::Tile(From::Evernote))), "Bring in from Evernote: .enex");
    assert_eq!(label(Target::Welcome(Hit::Start)), "Start writing");
    assert!(label(Target::Switch(Setting::Assist)).contains("Writing"));
    assert!(label(Target::Switch(Setting::Beam)).contains("Beam"));
    assert!(!controls.iter().any(|c| c.target == Target::Welcome(Hit::Page)), "the screen itself is no control");
    // Nothing of the page under it is read out.
    assert!(!controls.iter().any(|c| matches!(c.target, Target::Note | Target::NewNote | Target::Search)));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_letter_typed_presses_nothing_and_reaches_no_page() {
    let (mut m, root) = first_run("letters");
    m.tab(true);
    assert_eq!(m.welcome.as_ref().unwrap().focus, Some(Target::Welcome(Hit::Start)));
    assert!(!m.insert("a") && m.welcome_open(), "a letter is not a press");
    assert!(!m.backspace(false) && m.text().is_empty(), "nor does anything else reach the page under it");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn opened_again_while_it_brings_in_it_picks_the_line_up() {
    let (mut m, root) = first_run("again");
    let file = root.join("Big.enex");
    std::fs::write(&file, fixtures::enex(3000)).unwrap();
    m.bring_in(From::Evernote, vec![file]);
    m.escape();
    assert!(!m.welcome_open() && m.bringing.is_some(), "it goes on");
    m.show_welcome();
    assert!(matches!(line(&m), Line::Bringing { from: From::Evernote, .. }), "{:?}", line(&m));
    assert!(!m.bring_in(From::Notion, vec![root.join("x.zip")]), "one at a time");
    brought(&mut m);
    assert!(matches!(line(&m), Line::Brought { from: From::Evernote, count: 3000, .. }));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn settings_opened_over_it_puts_it_away() {
    let (mut m, root) = first_run("settings");
    assert!(m.session.model_asked, "the writing model is offered on it, not on its card");
    m.toggle_options();
    assert!(!m.welcome_open() && m.settings.open);
    let _ = std::fs::remove_dir_all(root);
}
