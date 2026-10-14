//! The fonts Kolada draws with on Windows, named file by file. kolada-text
//! never scans the system's fonts (that takes 100 ms to seconds); this list
//! is every face the app can need, all of which ship with Windows 10/11.
//! Missing ones are skipped, so a bare machine still draws, with what it has.

use std::path::PathBuf;

use kolada_text::FontSet;

fn dir() -> PathBuf {
    std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join("Fonts")
}

/// The font set for the Font option (0 sans serif, 1 serif, 2 monospace).
/// Only what the first screen needs is listed to load at once, and it is
/// not looked for first: a missing file is skipped by the loader. Two
/// probes (which icon font, which monospace) are all the disk sees.
pub fn font_set(option: u8) -> FontSet {
    let d = dir();
    // Segoe Fluent Icons comes with Windows 11; Windows 10 has the same
    // glyphs in Segoe MDL2 Assets.
    let fluent = d.join("SegoeIcons.ttf").is_file();
    let cascadia = d.join("CascadiaMono.ttf").is_file();
    let icon = if fluent { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
    let code = if cascadia { "Cascadia Mono" } else { "Consolas" };
    let text = match option {
        1 if d.join("SitkaVF.ttf").is_file() => "Sitka",
        1 => "Georgia",
        2 => code,
        _ => "Segoe UI",
    };
    // The app's own text and sans notes, in the weights it uses; the icons;
    // code.
    let mut now = vec!["segoeui.ttf", "segoeuib.ttf", "segoeuii.ttf", "segoeuiz.ttf", "seguisb.ttf"];
    now.push(if fluent { "SegoeIcons.ttf" } else { "segmdl2.ttf" });
    now.extend(if cascadia { &["CascadiaMono.ttf"][..] } else { &["consola.ttf", "consolab.ttf", "consolai.ttf"][..] });
    // The serif option sets the note in these.
    if option == 1 {
        now.extend(["georgia.ttf", "georgiab.ttf", "georgiai.ttf", "SitkaVF.ttf", "SitkaVF-Italic.ttf"]);
    }
    // What Segoe UI lacks: symbols, colour emoji, the CJK scripts; and the
    // faces other options use.
    let mut later = vec!["seguisym.ttf", "seguiemj.ttf", "YuGothR.ttc", "meiryo.ttc", "malgun.ttf", "msyh.ttc", "simsun.ttc"];
    if option != 1 {
        later.extend(["georgia.ttf", "georgiab.ttf", "georgiai.ttf", "SitkaVF.ttf", "SitkaVF-Italic.ttf"]);
    }
    let paths = |names: Vec<&str>| names.into_iter().map(|f| d.join(f)).collect();
    FontSet { files: paths(now), fallback: paths(later), data: Vec::new(), ui: "Segoe UI".into(), text: text.into(), code: code.into(), icon: icon.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_text::{Engine, Format, Layout, Role, Style};
    use std::time::Instant;

    fn engine() -> Engine {
        Engine::new(font_set(0))
    }

    fn width(e: &mut Engine, text: &str, f: &Format) -> f32 {
        Layout::new(e, text, f, 2000.0, 100.0, &[]).measure().width
    }

    #[test]
    fn loads_in_a_few_milliseconds_and_finds_every_family() {
        // The best of three: the first read of a just-built program's fonts
        // can be cold, and what this guards against (a scan of the system's
        // fonts) costs a hundred milliseconds or more every time.
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
        assert!(regular < semi && semi < bold, "{regular} {semi} {bold}");
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
            let l = Layout::new(&mut e, text, &Format::new(Role::Text, 16.0), 300.0, 50.0, &[]);
            let glyphs = l.glyphs((0.0, 0.0), 1.0);
            let drawn = glyphs.iter().filter(|g| e.mask(g).is_some_and(|m| m.width > 2)).count();
            assert_eq!(drawn, glyphs.len(), "{name}: every glyph has a picture");
            assert!(l.measure().width > 8.0, "{name}");
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
        // Premultiplied or not decides how it is blended: say which.
        let soft: Vec<&[u8]> = m.data.chunks(4).filter(|p| p[3] > 0 && p[3] < 255).collect();
        let premultiplied = soft.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
        eprintln!("emoji edge pixels: {} soft, premultiplied-looking: {premultiplied}", soft.len());
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
    fn the_font_option_changes_the_note_and_nothing_else() {
        let (sans, serif, mono) = (font_set(0), font_set(1), font_set(2));
        assert_eq!((sans.text.as_str(), sans.ui.as_str()), ("Segoe UI", "Segoe UI"));
        assert!(matches!(serif.text.as_str(), "Sitka" | "Georgia"));
        assert_eq!(mono.text, mono.code);
        assert_eq!(serif.ui, "Segoe UI");
        let mut e = Engine::new(serif);
        let body = width(&mut e, "The quick brown fox", &Format::new(Role::Text, 16.0));
        let ui = width(&mut e, "The quick brown fox", &Format::new(Role::Ui, 16.0));
        assert!((body - ui).abs() > 1.0, "serif text differs from the UI font: {body} vs {ui}");
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
}
