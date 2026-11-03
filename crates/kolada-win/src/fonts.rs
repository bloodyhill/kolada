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
    // Segoe UI Variable (Windows 11): the display face of a few headings.
    let mut later = vec!["seguisym.ttf", "seguiemj.ttf", "YuGothR.ttc", "meiryo.ttc", "malgun.ttf", "msyh.ttc", "simsun.ttc", "SegUIVar.ttf"];
    if option != 1 {
        later.extend(["georgia.ttf", "georgiab.ttf", "georgiai.ttf", "SitkaVF.ttf", "SitkaVF-Italic.ttf"]);
    }
    let paths = |names: Vec<&str>| names.into_iter().map(|f| d.join(f)).collect();
    // The app's own icons (filled, as on the Mac and Linux), in the program;
    // Windows' icon face stays for the window's own buttons.
    FontSet { files: paths(now), fallback: paths(later), data: vec![include_bytes!("../../../assets/fonts/KoladaIcons.ttf")], ui: "Segoe UI".into(), text: text.into(), code: code.into(), icon: "Kolada Icons".into() }
}

/// The font set a launch draws with: `font_set`'s, and where a font of the
/// machine's is chosen, its files, named in the session (read, never looked
/// for). Gone since, the note is set in sans.
pub fn note_set(s: &kolada_core::session::Session) -> FontSet {
    let mut set = font_set(s.font);
    if s.font == kolada_core::session::INSTALLED_FONT && !s.font_family.is_empty() {
        let files: Vec<PathBuf> = s.font_files.iter().map(PathBuf::from).collect();
        if files.first().is_some_and(|f| f.is_file()) {
            set.files.extend(files);
            set.text = s.font_family.clone();
        }
    }
    set
}

/// The faces Settings' Sans, Serif and Mono stand for, each named in its
/// own: as `font_set` has them, found once a run.
pub fn faces() -> [&'static str; 3] {
    static FACES: std::sync::OnceLock<[&'static str; 3]> = std::sync::OnceLock::new();
    *FACES.get_or_init(|| {
        let d = dir();
        [
            "Segoe UI",
            if d.join("SitkaVF.ttf").is_file() { "Sitka" } else { "Georgia" },
            if d.join("CascadiaMono.ttf").is_file() { "Cascadia Mono" } else { "Consolas" },
        ]
    })
}

/// Where the machine's fonts are: Windows' own, and those installed for
/// this user alone.
pub fn installed_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![dir()];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
    }
    dirs
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

    /// A font of the machine's is read at launch by the files the session
    /// names, never looked for; gone since, the note is set in sans.
    #[test]
    fn a_font_of_the_machines_is_read_by_its_files_at_launch() {
        use kolada_core::session::{INSTALLED_FONT, Session};
        let pala = dir().join("pala.ttf");
        if !pala.is_file() {
            return eprintln!("no Palatino Linotype here");
        }
        let s = Session { font: INSTALLED_FONT, font_family: "Palatino Linotype".into(), font_files: vec![pala.to_string_lossy().into_owned()], ..Session::default() };
        let set = note_set(&s);
        assert_eq!((set.text.as_str(), set.ui.as_str()), ("Palatino Linotype", "Segoe UI"));
        assert!(set.files.contains(&pala));
        let e = Engine::new(set);
        assert!(e.families().contains(&"Palatino Linotype".to_string()));
        let gone = Session { font_files: vec![r"C:\nowhere\gone.ttf".into()], ..s };
        let set = note_set(&gone);
        assert_eq!(set.text, "Segoe UI", "its file is gone: sans");
        assert_eq!(note_set(&Session { font: 1, ..gone }).text, font_set(1).text, "the app's own three are as before");
    }

    /// Settings › Font › Other's catalogue: Windows' own families, each
    /// once, read in well under a second on a worker.
    #[test]
    fn the_machines_fonts_read_as_families() {
        let t = Instant::now();
        let files = kolada_text::installed::files_in(&installed_dirs());
        let found = kolada_text::installed::families(&files);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("{} files, {} families in {ms:.0} ms", files.len(), found.len());
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        for want in ["Segoe UI", "Georgia", "Consolas"] {
            assert!(names.contains(&want), "{want}");
        }
        assert!(!names.iter().any(|n| n.contains("MDL2") || n.contains("Fluent Icons")), "icon fonts are left out");
        let georgia = found.iter().find(|f| f.name == "Georgia").unwrap();
        assert_eq!(georgia.files[0].file_name().unwrap().to_ascii_lowercase(), "georgia.ttf", "the regular first");
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
