//! Settings' strong choices, as every platform has them: the note's font
//! (the app's three, or one of the machine's from Other's finder), and the
//! switches that turn the louder features off, each stopping what it runs.
//! Windows does the same in kolada-win (`app/fontpick.rs`, `choose`).

use super::*;
use kolada_core::session::{self, INSTALLED_FONT};
use kolada_ui::scene::{Setting, Target};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn vault(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kolada-app-choices-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Plan.md"), "# Plan\n\nThe quick brown fox jumps over the lazy dog.\n").unwrap();
    dir
}

const SIZE: (u32, u32, f32) = (2000, 1800, 2.0);

fn opened(dir: &Path) -> Model {
    let mut m = Model::new(dir.to_path_buf(), true);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    m.toggle_options();
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    m
}

fn click(m: &mut Model, target: Target) -> bool {
    let r = m.hits.iter().find(|(_, t)| *t == target).map(|(r, _)| *r).unwrap_or_else(|| panic!("{target:?} is not on screen"));
    let changed = m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false);
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    changed
}

fn shows(m: &Model, target: Target) -> bool {
    m.hits.iter().any(|(_, t)| *t == target)
}

/// What a launch reads back.
fn relaunched(m: &Model) -> Model {
    let bytes = session::encode(&Session { last_active: session::now(), ..m.session.clone() });
    Model::open_vault(crate::Services::none(), m.vault.clone(), true, session::decode(&bytes))
}

#[test]
fn each_switch_turns_its_feature_off_and_on_and_is_kept() {
    let dir = vault("switches");
    let mut m = opened(&dir);
    for s in [Setting::Assist, Setting::Beam, Setting::Spell, Setting::Motion] {
        assert!(shows(&m, Target::Switch(s)), "{s:?} has a switch");
        assert!(scene::switch_on(s, m.option_values()[s.index()]), "{s:?} starts on");
    }
    for s in [Setting::Assist, Setting::Beam, Setting::Spell, Setting::Motion] {
        assert!(click(&mut m, Target::Switch(s)));
        assert!(!scene::switch_on(s, m.option_values()[s.index()]), "{s:?} off");
    }
    assert_eq!((m.session.assist, m.session.beam, m.session.spell, m.session.motion), (1, 1, 1, 1));
    let back = relaunched(&m);
    assert_eq!((back.session.assist, back.session.beam, back.session.spell, back.session.motion), (1, 1, 1, 1), "kept for the next launch");
    for s in [Setting::Assist, Setting::Beam, Setting::Spell, Setting::Motion] {
        click(&mut m, Target::Switch(s));
    }
    assert_eq!((m.session.assist, m.session.beam, m.session.spell, m.session.motion), (0, 0, 0, 0), "and on again");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_row_that_follows_a_switch_shows_only_while_it_is_on() {
    let dir = vault("follows");
    let mut m = opened(&dir);
    assert!(shows(&m, Target::Choice(Setting::Assist, 1)), "the model's size, while it is on");
    assert!(shows(&m, Target::Choice(Setting::QuickBeam, 1)), "Beam's keys, while it is on");
    click(&mut m, Target::Switch(Setting::Assist));
    click(&mut m, Target::Switch(Setting::Beam));
    assert!(!shows(&m, Target::Choice(Setting::Assist, 1)) && !shows(&m, Target::Choice(Setting::QuickBeam, 1)));
    // On again, the writing model is the one chosen before.
    m.session.model = "full".into();
    click(&mut m, Target::Switch(Setting::Assist));
    assert_eq!((m.session.assist, m.session.model.as_str()), (0, "full"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn spell_check_off_checks_nothing() {
    let dir = vault("spell");
    let mut m = opened(&dir);
    let checked = Arc::new(std::sync::Mutex::new(0usize));
    let c = checked.clone();
    click(&mut m, Target::Switch(Setting::Spell));
    let (asked, _) = m.spell_step_with(move |_| {
        *c.lock().unwrap() += 1;
        Vec::new()
    });
    assert!(!asked && *checked.lock().unwrap() == 0, "off: no line goes to the checker");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_writing_model_off_stops_its_helper_and_its_download() {
    let dir = vault("model-off");
    let mut m = opened(&dir);
    m.writing.download = crate::download::Download::start_for_test();
    assert!(m.writing.download.is_some());
    click(&mut m, Target::Switch(Setting::Assist));
    assert!(m.writing.helper.is_none() && m.writing.download.is_none(), "nothing of it runs while it is off");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn beam_off_stops_its_engine_and_sync_and_on_starts_them_again() {
    let dir = vault("beam");
    let mut m = opened(&dir);
    m.start_quick("http://127.0.0.1:9", "desktop", "Test PC", Arc::new(|| {}));
    assert!(m.quick_on());
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    m.take_requests();
    click(&mut m, Target::Switch(Setting::Beam));
    assert!(!m.quick_on() && !m.quick_wants_keys(), "the engine is let go, and the keys with it");
    assert_eq!(m.take_requests(), vec![settings::Request::Beam(false)], "the shell gives its keys back and puts its popup away");
    assert!(m.beam_view().is_none(), "no row or button");
    // Started again while it is off (the next launch), nothing starts.
    m.start_quick("http://127.0.0.1:9", "desktop", "Test PC", Arc::new(|| {}));
    assert!(!m.quick_on());
    click(&mut m, Target::Switch(Setting::Beam));
    assert!(m.quick_on(), "on again, it starts as the shell first started it");
    assert_eq!(m.take_requests(), vec![settings::Request::Beam(true)]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_switchs_rows_fold_on_a_spring_and_snap_with_motion_off() {
    let dir = vault("fold");
    let mut m = opened(&dir);
    m.ui.animate = true;
    click(&mut m, Target::Switch(Setting::Beam));
    let (s, t) = m.folding().expect("its rows fold away");
    assert!(s == Setting::Beam && t > 0.5, "they start open and go");
    assert!(m.wants_frames(), "frames come while they fold");
    m.ui.fold = m.ui.fold.map(|(s, _, on)| (s, Instant::now() - Duration::from_secs(1), on));
    assert!(m.tick() && m.folding().is_none() && m.ui.fold.is_none(), "settled, nothing more is due");
    // Motion off: nothing folds, the rows are there or not at once.
    click(&mut m, Target::Switch(Setting::Motion));
    click(&mut m, Target::Switch(Setting::Beam));
    assert!(m.folding().is_none() && shows(&m, Target::Choice(Setting::QuickBeam, 1)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn motion_off_snaps_what_would_move() {
    let dir = vault("motion");
    let mut m = opened(&dir);
    m.ui.animate = true;
    click(&mut m, Target::Switch(Setting::Motion));
    assert!(!m.ui.animate);
    m.ui.scroll_pending = 300.0;
    m.tick();
    assert_eq!(m.ui.scroll_pending, 0.0, "scrolling lands where it is going in one frame");
    click(&mut m, Target::Switch(Setting::Motion));
    assert!(m.ui.animate, "on again");
    // The OS asking for less motion wins over the switch.
    m.reduce_motion(true);
    click(&mut m, Target::Switch(Setting::Motion));
    click(&mut m, Target::Switch(Setting::Motion));
    assert!(m.session.motion == 0 && !m.ui.animate);
    let _ = std::fs::remove_dir_all(dir);
}

/// Other opens the finder of the machine's fonts (read on a worker); typing
/// narrows it, Enter chooses; the note is set in it at once and, by its
/// files, at the next launch, with nothing looked for.
#[test]
fn a_font_of_the_machines_is_found_chosen_and_read_at_launch() {
    let dir = vault("font");
    let mut m = opened(&dir);
    let sans = m.surface().pixels().to_vec();
    assert!(click(&mut m, Target::FontOther));
    let t = Instant::now();
    while m.font.finder.as_ref().is_some_and(|f| f.view().total == 0) && t.elapsed() < Duration::from_secs(20) {
        m.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(shows(&m, Target::FontName(0)), "the machine's fonts are listed");
    assert!(m.insert("pra"));
    assert_eq!(m.font.finder.as_ref().unwrap().view().names, ["Prata"]);
    assert!(m.newline(false), "Enter chooses it");
    assert_eq!((m.session.font, m.session.font_family.as_str(), m.session.font_files.len()), (INSTALLED_FONT, "Prata", 1));
    assert!(m.font.finder.is_none(), "the finder is put away");
    m.toggle_options();
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    assert!(m.surface().pixels() != &sans[..], "the note is set in it");
    let back = relaunched(&m);
    assert!(back.eng.borrow().families().contains(&"Prata".to_string()), "a launch reads its file");
    assert_eq!(back.font.reading.is_none() && back.font.finder.is_none(), true, "and looks through nothing");
    // Gone since: sans, and the choice kept for when it comes back.
    let gone = Session { font_files: vec!["/nowhere/Prata.ttf".into()], ..m.session.clone() };
    let after = Model::open_vault(crate::Services::none(), dir.clone(), true, Some(gone));
    assert!(!after.eng.borrow().families().contains(&"Prata".to_string()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn escape_puts_the_finder_away_and_a_font_of_the_apps_closes_it() {
    let dir = vault("finder-away");
    let mut m = opened(&dir);
    click(&mut m, Target::FontOther);
    assert!(m.font.finder.is_some() && shows(&m, Target::FontField));
    assert!(m.escape() && m.font.finder.is_none() && m.settings.open, "Escape puts the finder away, Settings stays");
    m.frame(SIZE.0, SIZE.1, SIZE.2);
    click(&mut m, Target::FontOther);
    click(&mut m, Target::Choice(Setting::Font, 1));
    assert!(m.font.finder.is_none() && m.session.font == 1);
    let _ = std::fs::remove_dir_all(dir);
}
