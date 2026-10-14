//! What the Mac's font list must keep promising, written to match the tests
//! of the same names in kolada-win's fonts.rs.

use super::*;
use kolada_text::{Engine, Format, Layout, Role, Style};
use std::time::Instant;

fn engine() -> Engine {
    Engine::new(font_set(0))
}

/// How many of `text`'s glyphs have no real picture: nothing to draw, or the
/// box a font draws for a character it lacks (a character no font has is
/// drawn to find out what that looks like).
fn lacking(e: &mut Engine, text: &str) -> usize {
    let format = Format::new(Role::Text, 16.0);
    let tofu = {
        let l = Layout::new(e, "\u{378}", &format, 300.0, 50.0, &[]);
        l.glyphs((0.0, 0.0), 1.0).first().and_then(|g| e.mask(g))
    };
    let l = Layout::new(e, text, &format, 300.0, 50.0, &[]);
    l.glyphs((0.0, 0.0), 1.0)
        .iter()
        .filter(|g| match e.mask(g) {
            None => true,
            Some(m) => m.width <= 2 || tofu.as_ref().is_some_and(|t| t.width == m.width && t.height == m.height && t.data == m.data),
        })
        .count()
}

#[test]
fn a_font_list_without_the_fallbacks_is_noticed_as_missing_scripts() {
    // The check below must be able to fail: with only the core fonts the
    // Hebrew has no picture of its own.
    let mut e = Engine::new(FontSet { fallback: Vec::new(), ..font_set(0) });
    assert!(lacking(&mut e, "\u{5E9}\u{5DC}\u{5D5}\u{5DD}") > 0);
}

fn width(e: &mut Engine, text: &str, f: &Format) -> f32 {
    Layout::new(e, text, f, 2000.0, 100.0, &[]).measure().width
}

#[test]
fn the_settings_and_the_font_list_agree_on_each_options_family() {
    use kolada_app::services::Fonts;
    for option in 0..3 {
        let (family, files) = MacFonts.note_family(option);
        let set = font_set(option);
        assert_eq!(family, set.text, "option {option}");
        // What the settings load after launch is not left out of the list.
        assert!(files.iter().all(|f| set.files.contains(f) || set.fallback.contains(f)), "option {option}");
    }
}

#[test]
fn every_file_the_list_names_is_a_font_this_mac_has() {
    // A renamed or moved file would be skipped silently by the loader and
    // the script it served would fall to tofu: say it here.
    for option in 0..3 {
        let set = font_set(option);
        for f in set.files.iter().chain(&set.fallback) {
            assert!(f.is_file(), "{f:?} is named by the font list (option {option}) but not on this machine");
        }
    }
}

#[test]
fn loads_in_a_few_milliseconds_and_finds_every_family() {
    // The best of three: the first read of fonts can be cold, and what this
    // guards against (a scan of the system's fonts) costs a hundred
    // milliseconds or more every time.
    let (mut ms, mut faces) = (f64::MAX, 0);
    for _ in 0..3 {
        let t = Instant::now();
        let e = Engine::new(font_set(0));
        ms = ms.min(t.elapsed().as_secs_f64() * 1000.0);
        faces = e.faces();
        assert!(faces >= 6, "the first screen's fonts: {:?}", e.families());
    }
    eprintln!("{faces} faces in {ms:.2} ms");
    assert!(ms < 10.0, "loading fonts must not eat the launch: {ms} ms");
}

#[test]
fn the_weights_the_app_uses_are_three_different_faces() {
    let mut e = engine();
    let w = |e: &mut Engine, weight: u16| width(e, "Heading text for a menu", &Format { weight, ..Format::new(Role::Ui, 14.0) });
    let (regular, semi, bold) = (w(&mut e, 400), w(&mut e, 600), w(&mut e, 700));
    eprintln!("regular {regular:.1} semibold {semi:.1} bold {bold:.1}");
    // Helvetica Neue has Regular, Medium and Bold: a 600 weight is drawn in
    // Bold (Windows has Segoe UI Semibold). The two heavier ones need not differ.
    assert!(regular < semi && semi <= bold, "{regular} {semi} {bold}");
}

#[test]
fn icons_draw_in_the_icon_font() {
    let mut e = engine();
    for icon in ["\u{E713}", "\u{E700}", "\u{E74D}"] {
        let l = Layout::new(&mut e, icon, &Format::new(Role::Icon, 15.0), 40.0, 40.0, &[]);
        let g = l.glyphs((0.0, 0.0), 1.0);
        let m = e.mask(&g[0]).unwrap_or_else(|| panic!("no picture for {icon:?}"));
        assert!(m.width >= 8 && m.data.iter().any(|&a| a > 200), "{icon:?}: {}x{}", m.width, m.height);
    }
}

#[test]
fn every_script_the_note_may_hold_has_a_font() {
    let mut e = engine();
    for (name, text) in [
        ("latin", "Notes"),
        ("arabic", "\u{645}\u{631}\u{62D}\u{628}\u{627}"),
        ("hebrew", "\u{5E9}\u{5DC}\u{5D5}\u{5DD}"),
        ("japanese", "\u{65E5}\u{672C}\u{8A9E}\u{306E}\u{30C6}\u{30B9}\u{30C8}"),
        ("korean", "\u{D55C}\u{AD6D}\u{C5B4}"),
        ("chinese", "\u{4E2D}\u{6587}"),
        ("symbols", "\u{2713}\u{2192}\u{2022}"),
    ] {
        assert_eq!(lacking(&mut e, text), 0, "{name}: every glyph has a picture of its own");
        assert!(width(&mut e, text, &Format::new(Role::Text, 16.0)) > 8.0, "{name}");
    }
}

#[test]
fn every_script_has_a_font_in_every_typeface_option() {
    // The fallback is the same under Sans, Serif and Mono: a note in Hebrew
    // is not tofu because the user chose the monospace.
    for option in 0..3 {
        let mut e = Engine::new(font_set(option));
        for (name, text) in [("arabic", "\u{645}\u{631}\u{62D}\u{628}\u{627}"), ("hebrew", "\u{5E9}\u{5DC}\u{5D5}\u{5DD}"), ("cjk", "\u{65E5}\u{672C}\u{4E2D}\u{6587}\u{D55C}")] {
            assert_eq!(lacking(&mut e, text), 0, "{name} under option {option}");
        }
    }
}

#[test]
fn colour_emoji_come_in_colour() {
    let mut e = engine();
    let l = Layout::new(&mut e, "\u{1F389}", &Format::new(Role::Text, 16.0), 100.0, 40.0, &[]);
    let g = l.glyphs((0.0, 0.0), 1.0);
    let m = e.mask(&g[0]).expect("the emoji has a picture");
    assert!(m.color, "from the emoji font, in colour");
    let coloured = m.data.chunks(4).filter(|p| p[3] > 200 && (p[0] as i32 - p[2] as i32).abs() > 60).count();
    assert!(coloured > 10, "{coloured} pixels with real colour");
}

#[test]
fn every_family_the_app_names_is_among_the_loaded() {
    for option in 0..3 {
        let set = font_set(option);
        let e = Engine::new(set.clone());
        let names = e.families();
        for want in [&set.ui, &set.text, &set.code, &set.icon] {
            assert!(names.contains(want), "{want:?} not in {names:?}");
        }
    }
}

#[test]
fn fonts_for_other_scripts_load_when_text_first_needs_them() {
    let mut e = engine();
    let before = e.faces();
    Layout::new(&mut e, "English only, caf\u{E9} \u{2026}", &Format::new(Role::Text, 16.0), 300.0, 50.0, &[]);
    assert_eq!(e.faces(), before, "nothing more is loaded for Latin text");
    Layout::new(&mut e, "\u{65E5}\u{672C}", &Format::new(Role::Text, 16.0), 300.0, 50.0, &[]);
    let loaded = e.faces();
    assert!(loaded > before, "the rest loads when a script needs it");
    Layout::new(&mut e, "\u{5E9}\u{5DC}\u{5D5}\u{5DD}", &Format::new(Role::Text, 16.0), 300.0, 50.0, &[]);
    assert_eq!(e.faces(), loaded, "and only once");
}

#[test]
fn the_font_option_changes_the_note_and_nothing_else() {
    let (sans, serif, mono) = (font_set(0), font_set(1), font_set(2));
    assert_eq!((sans.text.as_str(), sans.ui.as_str()), ("Helvetica Neue", "Helvetica Neue"));
    assert_eq!(serif.text, "Georgia");
    assert_eq!(mono.text, mono.code);
    assert_eq!(serif.ui, "Helvetica Neue");
    let mut e = Engine::new(serif);
    let body = width(&mut e, "The quick brown fox", &Format::new(Role::Text, 16.0));
    let ui = width(&mut e, "The quick brown fox", &Format::new(Role::Ui, 16.0));
    assert!((body - ui).abs() > 1.0, "serif text differs from the UI font: {body} vs {ui}");
    let mut e = Engine::new(mono);
    let body = width(&mut e, "The quick brown fox", &Format::new(Role::Text, 16.0));
    assert!((body - width(&mut e, "The quick brown fox", &Format::new(Role::Ui, 16.0))).abs() > 1.0, "monospace text differs from the UI font");
}

#[test]
fn choosing_serif_at_run_time_finds_its_face_though_launch_left_it_out() {
    // Launched in Sans, then the user picks Serif: Georgia was not loaded at
    // launch, and must be there for the next layout.
    let mut e = engine();
    assert!(!e.families().iter().any(|f| f == "Georgia"), "launch does not load the serif");
    let sans = width(&mut e, "The quick brown fox", &Format::new(Role::Text, 16.0));
    e.set_family(Role::Text, &font_set(1).text);
    assert!(e.families().iter().any(|f| f == "Georgia"), "{:?}", e.families());
    let serif = width(&mut e, "The quick brown fox", &Format::new(Role::Text, 16.0));
    assert!((serif - sans).abs() > 1.0, "{serif} vs {sans}");
}

#[test]
fn code_is_set_in_the_code_face_inside_a_sans_paragraph() {
    let mut e = engine();
    let mut code = Style::new(4..9);
    code.role = Some(Role::Code);
    let plain = Layout::new(&mut e, "run iiiii now", &Format::new(Role::Text, 16.0), 400.0, 50.0, &[]).measure().width;
    let mixed = Layout::new(&mut e, "run iiiii now", &Format::new(Role::Text, 16.0), 400.0, 50.0, &[code]).measure().width;
    assert!(mixed > plain + 10.0, "monospace i is wide: {mixed} vs {plain}");
}
