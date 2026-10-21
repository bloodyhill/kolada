use super::*;
use kolada_core::session::{self, Session};
use kolada_ui::scene::{Rect, Target};
use std::path::Path;

/// A small vault of its own: tests run side by side.
fn vault(test: &str, notes: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kolada-app-settings-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in notes {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

fn drawn(v: &Path, size: (u32, u32, f32)) -> Model {
    let mut m = Model::new(v.to_path_buf(), true);
    m.frame(size.0, size.1, size.2);
    m
}

const BIG: (u32, u32, f32) = (2000, 1600, 2.0);

fn centre(m: &Model, target: Target) -> (f32, f32) {
    let r: Rect = m.hits.iter().find(|(_, t)| *t == target).map(|(r, _)| *r).unwrap_or_else(|| panic!("{target:?} is not on screen"));
    ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
}

fn click_in(m: &mut Model, target: Target, size: (u32, u32, f32)) -> bool {
    let (x, y) = centre(m, target);
    let changed = m.pointer_down(x, y, 1, false);
    m.frame(size.0, size.1, size.2);
    changed
}

fn click(m: &mut Model, target: Target) -> bool {
    click_in(m, target, BIG)
}

/// The settings the panel on screen offers a choice of.
fn offered(m: &Model) -> Vec<Setting> {
    let mut out = Vec::new();
    for (_, t) in &m.hits {
        if let Target::Choice(s, _) = t {
            if !out.contains(s) {
                out.push(*s);
            }
        }
    }
    out
}

#[test]
fn the_settings_button_opens_a_panel_with_only_the_options_the_mac_has_and_escape_closes_it() {
    let v = vault("panel", &[("Plan.md", "# Plan\n\nShip it.\n")]);
    let mut m = drawn(&v, BIG);
    assert!(offered(&m).is_empty());
    assert!(click(&mut m, Target::Options));
    assert!(m.settings.open);
    let offered = offered(&m);
    for s in [Setting::Theme, Setting::Font, Setting::TextSize, Setting::LineWidth, Setting::Spell, Setting::Assist, Setting::Related, Setting::Launch, Setting::Window, Setting::Tray, Setting::Hotkey] {
        assert!(offered.contains(&s), "{s:?} is offered");
    }
    assert!(m.hits.iter().any(|(_, t)| *t == Target::VaultShow) && m.hits.iter().any(|(_, t)| *t == Target::VaultOpen));
    assert!(m.escape() && !m.settings.open, "Escape closes it");
    m.frame(BIG.0, BIG.1, BIG.2);
    // The veil closes it too, and the card does not.
    click(&mut m, Target::Options);
    assert!(!click(&mut m, Target::OptionsPanel) && m.settings.open, "a click on the card leaves it open");
    // (Beside the card: the veil's middle is under it.)
    assert!(m.pointer_down(4.0, 400.0, 1, false) && !m.settings.open);
    m.frame(BIG.0, BIG.1, BIG.2);
    assert!(click(&mut m, Target::Options) && m.settings.open, "the button opens it again");
    assert!(click(&mut m, Target::OptionsClose) && !m.settings.open);
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn choosing_an_option_changes_what_it_says_and_is_kept_in_the_session() {
    let v = vault("choose", &[("Plan.md", "# Plan\n\nShip it.\n")]);
    let mut m = drawn(&v, BIG);
    click(&mut m, Target::Options);
    // Theme: dark and light, whatever the system is.
    click(&mut m, Target::Choice(Setting::Theme, 1));
    assert_eq!((m.session.theme, m.dark), (1, false));
    click(&mut m, Target::Choice(Setting::Theme, 2));
    assert_eq!((m.session.theme, m.dark), (2, true));
    // Text size: a step larger than the default.
    click(&mut m, Target::Choice(Setting::TextSize, 18));
    click(&mut m, Target::Choice(Setting::LineWidth, 1));
    click(&mut m, Target::Choice(Setting::Launch, 2));
    click(&mut m, Target::Choice(Setting::Spell, 1));
    click(&mut m, Target::Choice(Setting::Font, 2));
    assert_eq!([m.session.text_size, m.session.line_width, m.session.launch, m.session.spell, m.session.font], [18, 1, 2, 1, 2]);
    // The chosen value is the one drawn raised: the panel follows the session.
    let before = m.surface().pixels().to_vec();
    click(&mut m, Target::Choice(Setting::Spell, 0));
    assert_ne!(m.surface().pixels(), &before[..], "the frame shows the new choice");
    // What launch keeps, read back by the next one.
    let bytes = session::encode(&Session { last_active: session::now(), ..m.session.clone() });
    let back = Model::open_vault(crate::Services::none(), v.clone(), true, session::decode(&bytes));
    assert_eq!([back.session.theme, back.session.text_size, back.session.line_width, back.session.launch, back.session.spell, back.session.font], [2, 18, 1, 2, 0, 2]);
    assert!(back.dark);
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_menu_bar_icon_and_the_hotkey_start_off_and_ask_the_shell_when_chosen() {
    let v = vault("requests", &[("Plan.md", "# Plan\n")]);
    let mut m = drawn(&v, BIG);
    assert_eq!((m.tray_mode(), m.hotkey_on()), (1, false), "quiet at rest on a first launch");
    assert!(m.take_requests().is_empty());
    click(&mut m, Target::Options);
    click(&mut m, Target::Choice(Setting::Tray, 2));
    click(&mut m, Target::Choice(Setting::Hotkey, 0));
    assert_eq!(m.take_requests(), vec![Request::Tray(2), Request::Hotkey(true)]);
    assert!(m.take_requests().is_empty(), "each request is served once");
    assert_eq!((m.tray_mode(), m.hotkey_on()), (2, true));
    click(&mut m, Target::VaultShow);
    assert_eq!(m.take_requests(), vec![Request::ShowVault(v.clone())]);
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn the_wheel_scrolls_the_panel_when_the_window_is_short_and_not_the_note() {
    let long = format!("# Plan\n\n{}", "A line of the note.\n\n".repeat(80));
    let v = vault("scroll", &[("Plan.md", &long)]);
    let small = (900, 400, 1.0);
    let mut m = drawn(&v, small);
    click_in(&mut m, Target::Options, small);
    assert!(m.settings.scroll_max > 40.0, "the rows do not all fit: {}", m.settings.scroll_max);
    assert!(!m.scroll_by(-20.0), "already at the top");
    assert!(m.scroll_by(30.0));
    assert_eq!((m.scroll, m.settings.scroll), (0.0, 30.0), "the panel moved, the note did not");
    m.scroll_by(100_000.0);
    assert_eq!(m.settings.scroll, m.settings.scroll_max, "it stops at the end");
    m.frame(small.0, small.1, small.2);
    assert!(m.hits.iter().any(|(_, t)| *t == Target::VaultShow), "the last row is reachable");
    m.escape();
    assert!(m.scroll_by(100.0), "the note scrolls again");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn highlights_gathers_every_marked_passage_on_a_page_that_is_read_and_its_links_open_notes() {
    let v = vault(
        "highlights",
        &[("Plan.md", "# Plan\n\nShip ==the beta on friday== and ==rest==.\n"), ("Bread.md", "# Bread\n\n==a long cold proof builds flavor==\n"), ("Empty.md", "# Empty\n\nNothing marked.\n")],
    );
    let mut m = drawn(&v, BIG);
    // No row in the side menu: the palette's command opens the page.
    assert!(!m.hits.iter().any(|(_, t)| *t == Target::Highlights));
    assert!(m.open_highlights());
    m.frame(BIG.0, BIG.1, BIG.2);
    assert!(m.settings.read_only);
    let page = m.text().to_string();
    assert!(page.starts_with("# Highlights\n3 passages from 2 notes\n### [[Bread]]\n> a long cold proof builds flavor\n### [[Plan]]\n> the beta on friday\n> rest\n"), "{page}");
    assert_eq!(m.session.note, "", "a page of its own, not a file");
    // Read, not typed into.
    assert!(!m.insert("x") && !m.newline(false) && !m.backspace(false));
    assert_eq!(m.text(), page);
    assert!(!m.dirty());
    // A click on a note's name opens that note, and the page is an ordinary one again.
    let at = page.find("[[Plan]]").unwrap() + 3;
    let line = m.lines.iter().find(|l| l.para.start <= at && at <= l.para.end).expect("the line is drawn");
    let (x, y) = (line.x + 24.0, line.y + line.height / 2.0);
    assert!(m.pointer_down(x, y, 1, false));
    assert_eq!(m.session.note, "Plan.md");
    assert!(!m.settings.read_only && m.insert("!"), "an opened note can be typed into");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn highlights_say_how_to_make_one_when_there_are_none_and_a_new_note_leaves_the_page() {
    let v = vault("nohighlights", &[("Plan.md", "# Plan\n\nNothing marked.\n")]);
    let mut m = drawn(&v, BIG);
    m.open_highlights();
    assert!(m.text().contains("Mark a passage with =="));
    m.new_note();
    assert!(!m.settings.read_only);
    assert!(m.insert("typed"), "a new note takes text");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn finds_the_wiki_link_around_a_point() {
    let text = "one [[Plan]] and [[Bread notes|bread]] then [[Old#Part]]\nsecond line [[x";
    let at = |s: &str, extra: usize| text.find(s).unwrap() + extra;
    assert_eq!(wiki_link_at(text, at("[[Plan", 4)), Some("Plan"));
    assert_eq!(wiki_link_at(text, at("[[Plan", 0)), Some("Plan"));
    assert_eq!(wiki_link_at(text, at("Bread", 2)), Some("Bread notes"));
    assert_eq!(wiki_link_at(text, at("Old", 1)), Some("Old"));
    assert_eq!(wiki_link_at(text, at(" and", 1)), None, "between links");
    assert_eq!(wiki_link_at(text, at("[[x", 3)), None, "an unclosed link is not one");
}

#[test]
fn a_copy_that_updates_itself_offers_the_switch_and_its_pill_asks_the_shell_to_put_the_update_in() {
    let v = vault("updates", &[("Plan.md", "# Plan\n\nShip it.\n")]);
    let mut m = drawn(&v, BIG);
    click(&mut m, Target::Options);
    assert!(!offered(&m).contains(&Setting::Updates), "a copy the shell did not call an updater has no switch");
    assert!(!m.checks_for_updates());
    m.set_updater(true);
    m.frame(BIG.0, BIG.1, BIG.2);
    assert!(offered(&m).contains(&Setting::Updates));
    assert!(m.checks_for_updates(), "on by default");
    click(&mut m, Target::Choice(Setting::Updates, 1));
    assert_eq!((m.session.updates, m.checks_for_updates()), (1, false));
    assert_eq!(m.settings.requests.last(), Some(&Request::Updates(false)));
    click(&mut m, Target::Choice(Setting::Updates, 0));
    assert_eq!(m.settings.requests.last(), Some(&Request::Updates(true)));
    m.escape();
    m.settings.requests.clear();
    // Nothing waits: no pill.
    m.frame(BIG.0, BIG.1, BIG.2);
    assert!(!m.hits.iter().any(|(_, t)| *t == Target::Update));
    assert!(m.update_ready("0.4.0") && !m.update_ready("0.4.0"), "said once");
    m.frame(BIG.0, BIG.1, BIG.2);
    assert!(click(&mut m, Target::Update));
    assert_eq!(m.settings.requests, vec![Request::InstallUpdate]);
    let _ = std::fs::remove_dir_all(v);
}
