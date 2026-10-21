//! The C interface end to end, as the Swift shell drives it. Each test
//! opens a generated vault through `kolada_open` (the app's own launch,
//! which keeps its session) in a home of the tests' own, one at a time.

use super::*;
use crate::services::{KoladaServices, Sink};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
static HOME: OnceLock<PathBuf> = OnceLock::new();

/// A spell checker for the tests: words starting "xx" are misspelled.
extern "C" fn check(units: *const u16, n: usize, out: *mut usize, most: usize) -> usize {
    let u = unsafe { std::slice::from_raw_parts(units, n) };
    let out = unsafe { std::slice::from_raw_parts_mut(out, most * 2) };
    let mut found = 0;
    let mut i = 0;
    while i + 1 < n && found < most {
        let start = i == 0 || !char::from_u32(u[i - 1] as u32).is_some_and(char::is_alphanumeric);
        if start && u[i] == 'x' as u16 && u[i + 1] == 'x' as u16 {
            let end = (i..n).find(|&j| !char::from_u32(u[j] as u32).is_some_and(char::is_alphanumeric)).unwrap_or(n);
            out[found * 2] = i;
            out[found * 2 + 1] = end;
            found += 1;
            i = end;
        }
        i += 1;
    }
    found
}

extern "C" fn guesses(_: *const c_char, s: Sink, to: *mut c_void) {
    let g = "plan";
    s(to, g.as_ptr().cast(), g.len());
}

/// The tests' home and services, made once; then a vault of `notes`
/// opened through the C interface, and drawn at an iPad's size.
pub fn open(name: &str, notes: &[(&str, &str)]) -> (MutexGuard<'static, ()>, PathBuf, *mut KoladaApp) {
    let lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    HOME.get_or_init(|| {
        // In a home of the tests' own, so neither the user's data nor a
        // test instance's is touched; every OS's data root points into it.
        let home = std::env::temp_dir().join(format!("kolada-ios-test-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        unsafe {
            std::env::set_var("KOLADA_TEST_INSTANCE", "1");
            std::env::set_var("HOME", &home);
            std::env::set_var("LOCALAPPDATA", &home);
            std::env::set_var("XDG_DATA_HOME", &home);
        }
        // The app's own fonts (no system font is found by name here) and a checker.
        let fonts = CString::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/ios").to_string_lossy().as_bytes()).unwrap();
        let table = KoladaServices { spell_check: Some(check), spell_guesses: Some(guesses), fonts_dir: fonts.as_ptr(), ..Default::default() };
        services::kolada_services(&table);
        home
    });
    let dir = std::env::temp_dir().join(format!("kolada-ios-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (n, t) in notes {
        std::fs::write(dir.join(n), t).unwrap();
    }
    let path = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
    let h = kolada_open(path.as_ptr(), false);
    kolada_frame(h, 1640, 2360, 2.0);
    (lock, dir, h)
}

fn string(s: *mut c_char) -> String {
    assert!(!s.is_null());
    let out = unsafe { CStr::from_ptr(s) }.to_str().unwrap().to_string();
    kolada_free_string(s);
    out
}

fn all(h: *mut KoladaApp) -> String {
    string(kolada_substring(h, 0, kolada_length(h)))
}

fn events(h: *mut KoladaApp) -> Vec<String> {
    std::iter::from_fn(|| {
        let e = kolada_take_event(h);
        (!e.is_null()).then(|| string(e))
    })
    .collect()
}

/// The middle of the first thing drawn that `is` picks.
fn spot(h: *mut KoladaApp, is: impl Fn(&kolada_ui::scene::Target) -> bool) -> (f32, f32) {
    let r = app(h).model.drawn_targets().iter().find(|(r, t)| r.right > r.left && is(t)).map(|(r, _)| *r).expect("drawn");
    ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
}

fn close(h: *mut KoladaApp, dir: PathBuf) {
    kolada_close(h);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_shell_opens_draws_types_and_reads_the_note_through_the_c_interface() {
    let (_one, dir, h) = open("basics", &[("Plan.md", "# Plan\n")]);
    let pixels = unsafe { std::slice::from_raw_parts(kolada_pixels(h), 1640 * 2360) };
    assert!(pixels.iter().filter(|&&p| p != pixels[0]).count() > 10_000, "a frame with something on it");
    kolada_command(h, c"moveToEndOfDocument:".as_ptr());
    assert!(kolada_insert(h, c"Ship it".as_ptr()));
    kolada_command(h, c"moveWordLeftAndModifySelection:".as_ptr());
    let (mut l, mut n) = (0, 0);
    kolada_selection(h, &mut l, &mut n);
    assert_eq!((l, n), (12, 2), "the word just typed is selected");
    assert_eq!(string(kolada_substring(h, l, n)), "it");
    // A Japanese input method composes, then settles.
    assert!(kolada_set_marked(h, c"\u{306B}\u{307B}".as_ptr(), 2, 0));
    assert!(kolada_marked(h, &mut l, &mut n));
    assert_eq!((l, n), (12, 2));
    assert!(kolada_insert(h, c"\u{65E5}\u{672C}".as_ptr()));
    assert!(!kolada_marked(h, &mut l, &mut n));
    assert_eq!(kolada_length(h), 14);
    kolada_frame(h, 1640, 2360, 2.0);
    let mut caret = [0.0f32; 4];
    assert!(kolada_caret(h, caret.as_mut_ptr()), "the caret is drawn");
    let mut at = [0.0f32; 4];
    assert!(kolada_rect(h, 14, 0, at.as_mut_ptr()));
    assert_eq!(at, caret, "an empty range at the caret is the caret's box");
    assert!(kolada_rect(h, 0, 6, at.as_mut_ptr()) && at[2] > at[0] + 100.0, "the title's line spans the column: {at:?}");
    assert!(kolada_do(h, c"undo".as_ptr()));
    // Paste HTML as Markdown.
    kolada_command(h, c"moveToEndOfDocument:".as_ptr());
    kolada_command(h, c"insertNewline:".as_ptr());
    assert!(clip::kolada_paste(h, c"Big".as_ptr(), c"<h2>Big</h2>".as_ptr(), std::ptr::null()));
    assert!(all(h).contains("## Big"), "the HTML came in as a heading");
    assert!(events(h).is_empty(), "nothing asked of the OS");
    // Export the note.
    assert!(string(kolada_export(h, c"markdown".as_ptr())).contains("## Big"));
    assert!(string(kolada_export(h, c"html".as_ptr())).contains("<h2"), "a web page of the note");
    // VoiceOver reads it by lines.
    assert_eq!(kolada_line_of(h, 0), 0);
    assert!(kolada_line_range(h, 0, &mut l, &mut n) && (l, n) == (0, 7), "the title's line, its newline included: {l} {n}");
    assert!(!kolada_line_range(h, 99, &mut l, &mut n));
    kolada_visible(h, &mut l, &mut n);
    assert_eq!(l, 0);
    assert_eq!(string(kolada_label(h)), "Plan");
    // A file brought in (it opens, as the first imported note does).
    let outside = std::env::temp_dir().join(format!("kolada-ios-test-{}-in.md", std::process::id()));
    std::fs::write(&outside, "# Brought in\n").unwrap();
    let list = CString::new(outside.to_string_lossy().as_bytes()).unwrap();
    let notes = |d: &Path| std::fs::read_dir(d).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".md")).count();
    let before = notes(&dir);
    assert!(kolada_import_files(h, list.as_ptr()));
    assert_eq!(notes(&dir), before + 1, "the file came into the vault");
    let _ = std::fs::remove_file(outside);
    kolada_close(h);
    let data = kolada_core::app_data_dir().unwrap();
    assert!(data.starts_with(HOME.get().unwrap()) && data.join("session.bin").is_file(), "the session went to the tests' own home: {data:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_long_press_on_the_text_gives_its_menu_and_on_a_row_the_rows() {
    let (_one, dir, h) = open("menus", &[("Plan.md", "# Plan\n\nsome words here\n"), ("Other.md", "# Other\n")]);
    let plan = CString::new(dir.join("Plan.md").to_string_lossy().as_bytes()).unwrap();
    arrive::kolada_open_file(h, plan.as_ptr());
    kolada_frame(h, 1640, 2360, 2.0);
    // The note's text: select there, then its menu (the pasteboard, formatting).
    let (x, y) = spot(h, |t| *t == kolada_ui::scene::Target::Note);
    assert_eq!(menus::kolada_press_kind(h, x, y), 1);
    // The whole line selected, pressed at its start.
    kolada_select(h, 11, 0);
    kolada_frame(h, 1640, 2360, 2.0);
    let mut r = [0.0f32; 4];
    assert!(kolada_caret(h, r.as_mut_ptr()));
    kolada_select(h, 8, 15);
    let (mut l, mut n) = (0, 0);
    kolada_selection(h, &mut l, &mut n);
    assert_eq!((l, n), (8, 15));
    let at = kolada_index_at(h, r[0] + 4.0, (r[1] + r[3]) / 2.0);
    assert!((8..=23).contains(&at), "pressed at {at}, caret box {r:?}");
    let lines = string(menus::kolada_menu(h, r[0] + 4.0, (r[1] + r[3]) / 2.0, false));
    let labels: Vec<&str> = lines.lines().map(|l| l.split('\t').next().unwrap()).collect();
    for want in ["Cut", "Copy", "Paste"] {
        assert!(labels.contains(&want), "{want} in {labels:?}");
    }
    let copy = labels.iter().position(|l| *l == "Copy").unwrap();
    assert!(!menus::kolada_menu_pick(h, copy));
    assert_eq!(events(h), ["edit\tcopy"], "the shell's pasteboard does it");
    // A row of the side menu: its own menu, without what iOS has no use for.
    let row = |t: &kolada_ui::scene::Target| matches!(t, kolada_ui::scene::Target::Row(_));
    let (x, y) = spot(h, row);
    assert_eq!(menus::kolada_press_kind(h, x, y), 2);
    let lines = string(menus::kolada_menu(h, x, y, false));
    assert!(lines.contains("Duplicate") && lines.contains("Rename"), "{lines}");
    assert!(!lines.contains("New Window") && !lines.contains("Finder"), "one window, no Finder: {lines}");
    let i = lines.lines().position(|l| l.starts_with("Duplicate")).unwrap();
    let md = |d: &Path| std::fs::read_dir(d).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".md")).count();
    let before = md(&dir);
    assert!(menus::kolada_menu_pick(h, i));
    assert_eq!(md(&dir), before + 1, "duplicated");
    close(h, dir);
}

#[test]
fn with_the_selection_bar_up_the_menu_leaves_its_formatting_to_the_bar() {
    let (_one, dir, h) = open("selbar", &[("Plan.md", "# Plan\n\nsome words here\n")]);
    kolada_select(h, 9, 0);
    kolada_frame(h, 1640, 2360, 2.0);
    let mut r = [0.0f32; 4];
    assert!(kolada_caret(h, r.as_mut_ptr()));
    let (x, y) = (r[0], (r[1] + r[3]) / 2.0);
    // A long press: the word chosen, as a double click chooses it.
    kolada_pointer_down(h, x, y, 2, false);
    kolada_pointer_up(h, x, y);
    kolada_frame(h, 1640, 2360, 2.0);
    assert!(app(h).model.selection_bar_shown(), "the bar comes up over the word");
    let lines = string(menus::kolada_menu(h, x, y, false));
    assert!(lines.contains("Copy") && !lines.contains("Bold"), "{lines}");
    close(h, dir);
}

#[test]
fn a_rows_three_dots_open_its_menu_where_the_model_asks() {
    let (_one, dir, h) = open("dots", &[("Plan.md", "# Plan\n"), ("Other.md", "# Other\n")]);
    // Pointed at, a row shows its dots; tapped, they ask for its menu.
    let (x, y) = spot(h, |t| matches!(t, kolada_ui::scene::Target::Row(_)));
    kolada_pointer_moved(h, x, y);
    kolada_frame(h, 1640, 2360, 2.0);
    let (x, y) = spot(h, |t| matches!(t, kolada_ui::scene::Target::RowMenu(_)));
    kolada_pointer_down(h, x, y, 1, false);
    kolada_pointer_up(h, x, y);
    let asked = events(h);
    assert!(asked.iter().any(|e| e.starts_with("menu\t")), "{asked:?}");
    let lines = string(menus::kolada_menu_requested(h));
    assert!(lines.contains("Rename"), "{lines}");
    assert!(menus::kolada_menu_requested(h).is_null(), "given once");
    close(h, dir);
}

#[test]
fn a_misspelled_word_is_marked_and_its_menu_offers_the_dictionarys_fix() {
    let (_one, dir, h) = open("spelling", &[("Plan.md", "# Plan\n\nthe xxplan is good\n")]);
    let mut more = true;
    let mut changed = false;
    for _ in 0..20 {
        changed |= kolada_spell_step(h, &mut more);
        if !more {
            break;
        }
    }
    assert!(changed, "the misspelling is marked");
    kolada_frame(h, 1640, 2360, 2.0);
    // Pressed in the word ("the " is 12 to 16, "xxplan" from there).
    kolada_select(h, 14, 0);
    kolada_frame(h, 1640, 2360, 2.0);
    let mut r = [0.0f32; 4];
    assert!(kolada_caret(h, r.as_mut_ptr()));
    kolada_select(h, 0, 0);
    let lines = string(menus::kolada_menu(h, r[0] + 4.0, (r[1] + r[3]) / 2.0, false));
    assert!(lines.starts_with("plan\t"), "the fix first: {lines}");
    assert!(menus::kolada_menu_pick(h, 0));
    assert!(all(h).contains("the plan is good"), "fixed");
    close(h, dir);
}

#[test]
fn a_copy_is_plain_html_and_markdown_and_pastes_back_exactly() {
    let (_one, dir, h) = open("copy", &[("Plan.md", "# Plan\n\nsome **bold** words\n")]);
    kolada_select(h, 15, 4);
    let (mut p, mut html, mut md) = (std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
    assert!(clip::kolada_copy(h, &mut p, &mut html, &mut md));
    let (p, html, md) = (string(p), string(html), string(md));
    assert_eq!(p, "bold");
    assert_eq!(md, "**bold**", "the marks come with it");
    assert!(html.contains("<strong>") || html.contains("<b>"), "{html}");
    kolada_command(h, c"moveToEndOfDocument:".as_ptr());
    let m = CString::new(md).unwrap();
    assert!(clip::kolada_paste(h, c"bold".as_ptr(), std::ptr::null(), m.as_ptr()));
    assert!(all(h).ends_with("words\n**bold**") || all(h).ends_with("**bold**"), "pasted back exactly");
    // A web address over selected words links them.
    kolada_select(h, 8, 4);
    assert!(clip::kolada_paste(h, c"https://kolada.app".as_ptr(), std::ptr::null(), std::ptr::null()));
    assert!(all(h).contains("[some](https://kolada.app)"), "linked");
    // Cut takes it out, one step.
    kolada_select(h, 0, 0);
    close(h, dir);
}

#[test]
fn a_screenshot_pasted_and_pictures_dropped_are_saved_and_embedded() {
    let png = include_bytes!("../../../assets/welcome/Welcome to Kolada.png");
    let (_one, dir, h) = open("pictures", &[("Plan.md", "# Plan\n\nfirst\n")]);
    kolada_command(h, c"moveToEndOfDocument:".as_ptr());
    assert!(clip::kolada_paste_picture(h, png.as_ptr(), png.len()));
    assert!(all(h).contains("![[Pasted image "), "embedded");
    // Dragged in from the Files app: the card knows a picture by its name.
    assert!(arrive::kolada_drag_enter(h, c"Shot.png".as_ptr(), 400.0, 300.0));
    arrive::kolada_drag_over(h, 420.0, 320.0);
    let end = arrive::kolada_drop(h);
    let file = std::env::temp_dir().join(format!("kolada-ios-test-{}-Shot.png", std::process::id()));
    std::fs::write(&file, png).unwrap();
    let list = CString::new(file.to_string_lossy().as_bytes()).unwrap();
    assert!(arrive::kolada_drop_files(h, list.as_ptr(), 420.0, 320.0, end));
    assert!(all(h).contains(&format!("![[{}]]", file.file_name().unwrap().to_string_lossy())), "the dropped picture is embedded");
    let _ = std::fs::remove_file(file);
    close(h, dir);
}

#[test]
fn a_file_opened_with_kolada_is_brought_in_and_one_of_the_vaults_opens() {
    let (_one, dir, h) = open("openwith", &[("Plan.md", "# Plan\n"), ("Other.md", "# Other\n")]);
    let own = CString::new(dir.join("Other.md").to_string_lossy().as_bytes()).unwrap();
    assert!(arrive::kolada_open_file(h, own.as_ptr()));
    assert_eq!(string(kolada_label(h)), "Other");
    let inbox = std::env::temp_dir().join(format!("kolada-ios-test-{}-Inbox", std::process::id()));
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join("Shared.md"), "# Shared\n").unwrap();
    let from = CString::new(inbox.join("Shared.md").to_string_lossy().as_bytes()).unwrap();
    assert!(arrive::kolada_open_file(h, from.as_ptr()));
    assert!(dir.join("Shared.md").is_file(), "a copy in the vault");
    let _ = std::fs::remove_dir_all(inbox);
    close(h, dir);
}

#[test]
fn typing_redraws_only_part_of_the_screen() {
    let (_one, dir, h) = open("damage", &[("Plan.md", "# Plan\n\nsome words\n")]);
    let mut rects = [0i32; 64];
    kolada_frame(h, 1640, 2360, 2.0);
    kolada_command(h, c"moveToEndOfDocument:".as_ptr());
    kolada_frame(h, 1640, 2360, 2.0);
    kolada_insert(h, c"x".as_ptr());
    kolada_frame(h, 1640, 2360, 2.0);
    let n = kolada_damage(h, rects.as_mut_ptr(), 16);
    assert!(n > 0, "some of it changed: {n}");
    let area: i64 = rects.chunks(4).take(n as usize).map(|r| ((r[2] - r[0]) as i64) * ((r[3] - r[1]) as i64)).sum();
    assert!(area < 1640 * 2360 / 4, "a keystroke repaints a small part of the screen, not all of it: {area}");
    kolada_frame(h, 1600, 2360, 2.0);
    assert_eq!(kolada_damage(h, rects.as_mut_ptr(), 16), -1, "a new size paints everything");
    close(h, dir);
}

#[test]
fn the_keyboards_shortcuts_are_listed_for_the_menu_bar_and_format_the_note() {
    let (_one, dir, h) = open("shortcuts", &[("Plan.md", "# Plan\n\nsome words\n")]);
    let lines = string(keys::kolada_commands());
    assert!(lines.lines().all(|l| l.split('\t').count() == 5), "{lines}");
    assert!(lines.contains("Format\tBold\tb\tcmd\tbold"));
    kolada_select(h, 8, 4);
    assert!(kolada_do(h, c"bold".as_ptr()));
    assert!(all(h).contains("**some**"));
    // The send menu's Share link opens the share card.
    assert!(kolada_do(h, c"share".as_ptr()));
    assert!(app(h).model.sharing());
    assert!(!kolada_do(h, c"no-such-thing".as_ptr()));
    close(h, dir);
}
