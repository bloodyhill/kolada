use super::*;
use crate::display;
use crate::scene::{self, Fonts, Ink, Scene, Text};
use kolada_text::{Format, Role};
use crate::theme::{DARK, LIGHT, Theme};
use kolada_core::session::{RowKind, SidebarRow};

/// The text engine on Windows' own fonts, as the app starts it.
fn factory() -> scene::Eng {
    scene::Eng::new(kolada_text::Engine::new(crate::fonts::font_set(0)))
}

/// A scene of plain text lines in Segoe UI 16, one per entry, at (x, y) DIPs.
fn text_scene(eng: &scene::Eng, lines: &[(&str, f32, f32)]) -> Scene {
    let format = Format::new(Role::Ui, 16.0);
    let texts = lines.iter().map(|&(text, x, y)| Text { x, y, layout: scene::layout(eng, text, &format, 250.0, 100.0), ink: Ink::Text, alpha: 1.0 }).collect();
    Scene { texts, ..Scene::default() }
}

fn render(eng: &scene::Eng, scene: &Scene, theme: &Theme, view: (u32, u32, u32), threads: usize) -> Surface {
    let list = display::record(eng, scene, theme, view.2 as f32 / 96.0).unwrap();
    let mut surface = Surface::new(view);
    surface.paint(&list, true, threads, false);
    surface
}

#[test]
fn draws_latin_arabic_and_color_emoji() {
    let dw = factory();
    for text in ["Hello, world", "ترند الأخبار", "🔥"] {
        let scene = text_scene(&dw, &[(text, 10.0, 10.0)]);
        let surface = render(&dw, &scene, &DARK, (600, 120, 192), THREADS);
        let inked = surface.pixels.iter().filter(|&&p| p != DARK.background).count();
        assert!(inked > 50, "{text:?}: only {inked} pixels drawn");
    }
    // Emoji keep their colors: some pixels must be clearly saturated.
    let scene = text_scene(&dw, &[("🔥", 10.0, 10.0)]);
    let surface = render(&dw, &scene, &DARK, (600, 120, 192), THREADS);
    let saturated = surface.pixels.iter().filter(|&&p| {
        let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
        r.max(g).max(b) - r.min(g).min(b) > 80
    });
    assert!(saturated.count() > 20, "emoji drawn without color");
}

fn full_scene(dw: &scene::Eng, fonts: &Fonts) -> Scene {
    let rows = vec![
        SidebarRow { depth: 0, kind: RowKind::Folder { expanded: false }, name: "Notes".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Untitled 34".into() },
    ];
    let long = "A line of body text that is long enough to wrap onto more than one line. ".repeat(3);
    let text = format!(
        "# Heading\n{long}\n| []( https://www.tiktok.com/@trendnews24 )[ترند الاخبار] | 781K |\nترند الأخبار 781K 🔥\n{long}\n"
    );
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(&text).collect();
    let input = scene::Input {
        vault_name: "my notes",
        active_note: "Untitled 34",
        text: &text,
        sidebar: &rows,
        scroll_line: 0,
        scroll_px: 0.0,
        // At the end: the heading's #s fold away, as they do away from the
        // caret (the reference draws every glyph in one color).
        selection: (text.len(), text.len()),
        lines: &lines,
        active_tools: &[],
        tree_top: 0,
        search: None,
        options_open: false,
        options: [0; scene::SETTINGS],
        chips: &[],
        dragged: None,
        drop_at: None,
        page_fade: 1.0,
            highlights_open: false,
            peek: None,
            whisper: None,
            popup: None,
        sidebar_dock: 1.0,
        sidebar_show: 1.0,
        toolbar_dock: 1.0,
        toolbar_show: 1.0,
        hover: None,
        sidebar_wave: &[],
        toolbar_wave: &[],
        maximized: false,
        ..Default::default()
    };
    scene::build(dw, fonts, &input, 1280.0, 752.0).unwrap()
}

#[test]
fn bands_produce_exactly_the_single_thread_frame() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let scene = full_scene(&dw, &fonts);
    let one = render(&dw, &scene, &DARK, (3840, 2256, 288), 1);
    let many = render(&dw, &scene, &DARK, (3840, 2256, 288), 7);
    let diff: Vec<usize> = (0..one.pixels.len()).filter(|&i| one.pixels[i] != many.pixels[i]).collect();
    if let Some(&i) = diff.first() {
        let rows: std::collections::BTreeSet<usize> = diff.iter().map(|i| i / 3840).collect();
        panic!("{} pixels differ; first at ({}, {}); rows {:?}", diff.len(), i % 3840, i / 3840, rows.iter().take(20).collect::<Vec<_>>());
    }
}

#[test]
fn reuses_cached_glyphs_on_the_next_frame() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let scene = full_scene(&dw, &fonts);
    let view = (1920, 1080, 144);
    let list = display::record(&dw, &scene, &LIGHT, 1.5).unwrap();
    let mut surface = Surface::draw_new(view, &list);
    assert!(surface.last_new_glyphs > 20);
    surface.draw(view, &list);
    assert_eq!(surface.last_new_glyphs, 0);
}

#[test]
#[ignore = "writes chrome screenshots for manual inspection"]
fn dump_chrome() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![
        SidebarRow { depth: 0, kind: RowKind::Folder { expanded: true }, name: "Notes".into() },
        SidebarRow { depth: 1, kind: RowKind::Note, name: "Meeting notes".into() },
        SidebarRow { depth: 1, kind: RowKind::Note, name: "Untitled 34".into() },
        SidebarRow { depth: 0, kind: RowKind::Folder { expanded: false }, name: "Projects".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Reading list".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "ترند الأخبار".into() },
    ];
    let text = concat!(
        "# Weekly plan\n",
        "Ship the **frameless window** and the *side menu*, then ~~polish~~ test it.\n",
        "- Rounded panels\n",
        "- Collapsible toolbar\n",
        "\t- Magnetic pills\n",
        "- [x] Glyph cache renderer\n",
        "- [ ] Hidden syntax\n",
        "1. Draft\n",
        "2. Review\n",
        "## Notes\n",
        "> Speed is a feature, not a benchmark.\n",
        "Build with `cargo build --release`, then see [[Benchmarks]] and ==remember this== #perf\n",
        "---\n",
        "```rust\n",
        "fn main() {}\n",
        "```\n",
        "ترند الأخبار 781K 🔥\n",
    );
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    // A string at rest, and one pulled by a pointer near its middle.
    let rest: &[f32] = &[];
    let near: [f32; crate::wave::SAMPLES] = std::array::from_fn(|i| {
        crate::wave::REACH * (-0.5 * ((i as f32 - 26.0) / 3.5).powi(2)).exp() + 2.0 * ((i as f32) * 0.9).sin() * (-(i as f32 - 26.0).abs() / 9.0).exp()
    });
    // (name, theme, width, sidebar (dock, show), toolbar (dock, show), hover,
    // side pill magnet, selection)
    let cases = [
        ("open-dark", &DARK, 1400.0, (1.0, 1.0), (1.0, 1.0), Some(scene::Target::NewNote), rest, (2, 2)),
        ("edge-hover", &DARK, 1400.0, (1.0, 1.0), (1.0, 1.0), Some(scene::Target::SidebarEdge), rest, (2, 2)),
        ("focus-rest", &DARK, 1400.0, (0.0, 0.0), (0.0, 0.0), None, rest, (2, 2)),
        ("focus-near", &DARK, 1400.0, (0.0, 0.0), (0.0, 0.0), None, &near[..], (2, 2)),
        ("focus-revealed", &DARK, 1400.0, (0.0, 1.0), (0.0, 0.0), Some(scene::Target::Row(4)), rest, (2, 2)),
        ("open-light", &LIGHT, 1400.0, (1.0, 1.0), (1.0, 1.0), Some(scene::Target::Tool(scene::Tool::Bold)), rest, (2, 2)),
        ("select-dark", &DARK, 1400.0, (1.0, 1.0), (1.0, 1.0), None, rest, (19, 80)),
        ("select-light", &LIGHT, 1400.0, (1.0, 1.0), (1.0, 1.0), None, rest, (19, 80)),
        // A narrow window: the revealed menu floats over the note text.
        ("narrow-revealed", &DARK, 760.0, (0.0, 1.0), (0.0, 0.0), None, rest, (19, 80)),
        ("options-dark", &DARK, 1400.0, (1.0, 1.0), (1.0, 1.0), Some(scene::Target::Choice(scene::Setting::TextSize, 18)), rest, (2, 2)),
    ];
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    for (name, theme, width, side, top, hover, magnet, selection) in cases {
        let input = scene::Input {
            vault_name: "my notes",
            active_note: "Notes/Untitled 34",
            text,
            sidebar: &rows,
            scroll_line: 0,
            scroll_px: 0.0,
            selection,
            lines: &lines,
            active_tools: &[],
            tree_top: 0,
            search: None,
            options_open: name == "options-dark",
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            chips: &[],
            dragged: None,
            drop_at: None,
            page_fade: 1.0,
            highlights_open: false,
            peek: None,
            whisper: None,
            popup: None,
            sidebar_dock: side.0,
            sidebar_show: side.1,
            toolbar_dock: top.0,
            toolbar_show: top.1,
            hover,
            sidebar_wave: magnet,
            toolbar_wave: &[],
            maximized: false,
            ..Default::default()
        };
        let mut s = scene::build(&dw, &fonts, &input, width, 820.0).unwrap();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, theme, 1.5).unwrap();
        let surface = Surface::draw_new(((width * 1.5) as u32, 1230, 144), &list);
        surface.dump(&dir.join(format!("chrome-{name}.bmp"))).unwrap();
    }
}

/// A note with "かんじ" being composed after "Plan " on its second line, the
/// last two kana being converted, as the scene draws it.
fn composing_scene(dw: &scene::Eng, fonts: &Fonts) -> (Scene, (usize, usize), (usize, usize)) {
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Plan".into() }];
    let text = "# Plan\nPlan \u{304B}\u{3093}\u{3058} for the week\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let start = text.find('\u{304B}').unwrap();
    let (marked, target) = ((start, start + 9), (start + 3, start + 9));
    let input = scene::Input {
        vault_name: "notes",
        active_note: "Plan",
        text,
        sidebar: &rows,
        selection: (start + 9, start + 9),
        lines: &lines,
        options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        page_fade: 1.0,
        sidebar_dock: 1.0,
        sidebar_show: 1.0,
        toolbar_dock: 1.0,
        toolbar_show: 1.0,
        marked: Some((marked, Some(target))),
        ..Default::default()
    };
    (scene::build(dw, fonts, &input, 1200.0, 560.0).unwrap(), marked, target)
}

#[test]
fn text_being_composed_is_underlined_and_its_converted_part_in_the_accent() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let (scene, _, _) = composing_scene(&dw, &fonts);
    // Thin bars under text: the whole composition muted, the converted part
    // in the accent, shorter (two of three kana) and thicker.
    let c = scene.column;
    let in_note = |r: &scene::Rect| r.left >= c.left && r.right <= c.right && r.top > c.top;
    let bars = |ink: Ink| scene.fills.iter().filter(|f| f.ink == ink && f.rect.height() < 3.0 && f.rect.width() > 4.0 && in_note(&f.rect)).map(|f| f.rect).collect::<Vec<_>>();
    let (all, converted) = (bars(Ink::Muted), bars(Ink::Accent));
    assert_eq!((all.len(), converted.len()), (1, 1), "{all:?} {converted:?}");
    let (all, converted) = (all[0], converted[0]);
    assert!(converted.width() < all.width() && converted.height() > all.height());
    assert!(converted.left > all.left && (converted.right - all.right).abs() < 1.0, "the last two kana: {all:?} {converted:?}");
}

#[test]
#[ignore = "writes text being composed with an input method for manual inspection"]
fn dump_composing() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let (mut scene, _, _) = composing_scene(&dw, &fonts);
    scene.place_caret((0.0, 0.0));
    for (name, theme) in [("dark", &DARK), ("light", &LIGHT)] {
        let list = display::record(&dw, &scene, theme, 1.5).unwrap();
        Surface::draw_new((1800, 840, 144), &list).dump(&dir.join(format!("composing-{name}.bmp"))).unwrap();
    }
}

#[test]
#[ignore = "writes files dragged over the note for manual inspection"]
fn dump_arriving() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Weekly plan".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Reading list".into() },
    ];
    let text = "# Weekly plan\nShip the frameless window and the side menu, then test it on every screen size.\nSpeed is a feature, not a benchmark. Every frame counts.\nRead the invoice before Thursday and reply to the vendor.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let after = lines[1].end;
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let card = |title, change, readable, stack, dim, landing| scene::Arriving { title, change, x: 640.0, y: 200.0, alpha: 1.0, readable, stack, dim, landing };
    let cases = [
        ("document-dark", &DARK, card("Invoice 1042.pdf", "PDF \u{2192} New note", true, false, 1.0, None)),
        ("document-light", &LIGHT, card("Invoice 1042.pdf", "PDF \u{2192} New note", true, false, 1.0, None)),
        ("documents-dark", &DARK, card("3 documents", "PDF \u{00B7} DOCX \u{2192} 3 new notes", true, true, 1.0, None)),
        ("pictures-dark", &DARK, card("2 pictures", "PNG \u{00B7} JPG \u{2192} This note", true, true, 0.0, Some((after, 76.0, 2)))),
        ("pictures-light", &LIGHT, card("2 pictures", "PNG \u{00B7} JPG \u{2192} This note", true, true, 0.0, Some((after, 76.0, 2)))),
        ("unreadable-dark", &DARK, card("setup.exe", "Kolada can't read this", false, false, 0.0, None)),
    ];
    for (name, theme, arriving) in cases {
        let input = scene::Input {
            vault_name: "my notes",
            active_note: "Weekly plan",
            text,
            sidebar: &rows,
            selection: (2, 2),
            lines: &lines,
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            page_fade: 1.0,
            sidebar_dock: 1.0,
            sidebar_show: 1.0,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            arriving: Some(arriving),
            ..Default::default()
        };
        let mut s = scene::build(&dw, &fonts, &input, 1200.0, 560.0).unwrap();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, theme, 1.5).unwrap();
        Surface::draw_new((1800, 840, 144), &list).dump(&dir.join(format!("arriving-{name}.bmp"))).unwrap();
    }
}

#[test]
#[ignore = "writes the share card in each phase for manual inspection"]
fn dump_share() {
    use kolada_core::share::{Expiry, Link, Problem};
    use kolada_ui::share::{Card, Phase, ShareView};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let text = "# Weekly plan\nShip the frameless window and the side menu, then test it on every screen size.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let now = 1_791_000_000;
    let link = |id: &str, title: &str, note: &str, days: u64| Link { id: id.into(), created: now - 100, expires: now + days * 86_400 + 60, title: title.into(), note: note.into() };
    let two = [link("Y5tq", "Weekly plan", "Weekly plan.md", 6), link("Ab3D", "Trip to the coast", "Trip.md", 1)];
    let three = [two[0].clone(), two[1].clone(), link("Qq7w", "Reading list for the autumn", "Reading.md", 29)];
    let choose = Card::new(Expiry::Week, "Weekly plan.md", &two, now);
    let mut ready = choose.clone();
    ready.phase = Phase::Ready { url: "https://notsy.co/Y5tq".into() };
    ready.copied = Some(kolada_ui::share::Copied::New);
    let mut full = Card::new(Expiry::Week, "Weekly plan.md", &three, now);
    full.confirming = Some("Ab3D".into());
    let mut failed = Card::new(Expiry::Month, "Weekly plan.md", &[], now);
    failed.phase = Phase::Failed(Problem::TooBig { bytes: 1_400_000, limit: 1_000_000, pictures: true }.message());
    let cases = [("choose", &choose), ("ready", &ready), ("full", &full), ("failed", &failed)];
    for (name, card) in cases {
        for (theme_name, theme) in [("dark", &DARK), ("light", &LIGHT)] {
            let input = scene::Input {
                vault_name: "my notes",
                active_note: "Weekly plan",
                text,
                selection: (2, 2),
                lines: &lines,
                page_fade: 1.0,
                toolbar_dock: 1.0,
                toolbar_show: 1.0,
                share: Some(ShareView { card, alpha: 1.0, now }),
                ..Default::default()
            };
            let s = scene::build(&dw, &fonts, &input, 1000.0, 560.0).unwrap();
            let list = display::record(&dw, &s, theme, 1.5).unwrap();
            Surface::draw_new((1500, 840, 144), &list).dump(&dir.join(format!("share-{name}-{theme_name}.bmp"))).unwrap();
        }
    }
}

#[test]
#[ignore = "writes a table and its menu for manual inspection"]
fn dump_table() {
    use crate::scene::{MenuItem, MenuView};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch budget".into() }];
    let text = "# Launch budget\nWhat the launch costs, by item.\n| Item | Owner | Cost |\n| --- | :-: | --: |\n| Press kit | Sam | 120 |\n| Launch video, cut for social | Omar | 4,500 |\n| Booth | Lea | 900 |\nTotal goes to finance on Friday.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let caret = text.find("Omar").unwrap() + 2;
    let item = |icon: &'static str, label: &str, keys: &'static str| MenuItem { icon, label: label.into(), keys, ..MenuItem::default() };
    let divided = |i: MenuItem| MenuItem { divider: true, ..i };
    let danger = |i: MenuItem| MenuItem { danger: true, ..i };
    let table = vec![
        item("\u{E710}", "Add row below", "Enter"),
        item("\u{E710}", "Add row above", ""),
        item("\u{E710}", "Add column right", ""),
        item("\u{E710}", "Add column left", ""),
        divided(item("\u{E8E4}", "Align left", "")),
        item("\u{E8E2}", "Align right", ""),
        divided(danger(item("\u{E74D}", "Delete row", ""))),
        danger(item("\u{E74D}", "Delete column", "")),
    ];
    // As the app builds it (see `text_menu`): the table, the clipboard,
    // Select all.
    let mut menu = table;
    menu.extend([divided(item("\u{E77F}", "Paste", "Ctrl+V")), divided(item("\u{E8B3}", "Select all", "Ctrl+A"))]);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let cases: [(&str, &Theme, Option<&[MenuItem]>); 4] = [("dark", &DARK, None), ("light", &LIGHT, None), ("menu-dark", &DARK, Some(&menu)), ("menu-light", &LIGHT, Some(&menu))];
    for (name, theme, menu) in cases {
        let input = scene::Input {
            vault_name: "my notes",
            active_note: "Launch budget",
            text,
            sidebar: &rows,
            selection: (caret, caret),
            lines: &lines,
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            page_fade: 1.0,
            sidebar_dock: 1.0,
            sidebar_show: 1.0,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            menu: menu.map(|items| MenuView { x: 640.0, y: 120.0, align_right: false, items, alpha: 1.0 }),
            hover: menu.map(|_| scene::Target::MenuItem(2)),
            ..Default::default()
        };
        let mut s = scene::build(&dw, &fonts, &input, 1200.0, 760.0).unwrap();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, theme, 1.5).unwrap();
        Surface::draw_new((1800, 1140, 144), &list).dump(&dir.join(format!("table-{name}.bmp"))).unwrap();
    }
}

#[test]
#[ignore = "writes the top of the window, the toolbar revealed and a passing word, for manual inspection"]
fn dump_top() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch plan".into() }];
    let text = "Launch plan\nShip the public beta on October 12.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    // Name, toolbar (dock, show), the note alone, window width.
    for (name, bar, compact, w) in [("revealed", (0.0, 1.0), false, 1000.0), ("focus", (0.0, 0.0), false, 1000.0), ("docked", (1.0, 1.0), false, 1000.0), ("alone", (0.0, 0.0), true, 560.0)] {
        let input = scene::Input {
            vault_name: "my notes",
            active_note: "Launch plan",
            text,
            sidebar: &rows,
            selection: (4, 4),
            lines: &lines,
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            page_fade: 1.0,
            page_date: Some("Edited today, 9:42 AM \u{00B7} Created 28 Sep"),
            whisper: Some(scene::Whisper { text: "Deleted Daily summary \u{00B7} Ctrl+Z brings it back", alpha: 1.0 }),
            toolbar_dock: bar.0,
            toolbar_show: bar.1,
            compact,
            no_margin: compact,
            ..Default::default()
        };
        let mut s = scene::build(&dw, &fonts, &input, w, 300.0).unwrap();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, &DARK, 1.5).unwrap();
        Surface::draw_new(((w * 1.5) as u32, 450, 144), &list).dump(&dir.join(format!("top-{name}.bmp"))).unwrap();
    }
}

/// A note's past for the scrubber's pictures: a week of versions in
/// sittings, the note as it was three versions back, and as it is now.
fn scrub_case() -> (Vec<u64>, Vec<usize>, &'static str, &'static str) {
    let (m, h, d) = (60_000u64, 3_600_000u64, 86_400_000u64);
    let start = 1_790_000_000_000u64;
    let mut times = Vec::new();
    let mut changes = Vec::new();
    // Four sittings: a first draft, two returns, today's edits.
    let sittings: [(u64, &[usize]); 4] = [
        (0, &[640, 120, 88, 30, 12, 260, 45, 18, 9, 140]),
        (2 * d + 3 * h, &[22, 8, 310, 64, 12]),
        (4 * d + 9 * h, &[5, 14, 90, 33, 7, 480, 26, 11, 60]),
        (6 * d + 2 * h, &[18, 140, 9, 36, 72, 12, 210]),
    ];
    for (at, sizes) in sittings {
        for (k, &c) in sizes.iter().enumerate() {
            times.push(start + at + k as u64 * 4 * m);
            changes.push(c);
        }
    }
    let then = "# Launch plan\nShip the public beta on **October 12**, with the press kit ready a week before.\n\n## Owners\n- Sam: pricing page and the FAQ\n- Lea: the launch video\n- Omar: press list\n\nBudget stays under 6,000 for the first month.\nRisks: the video slips; pricing needs legal.\n";
    let now = "# Launch plan\nShip the public beta on **October 19**, with the press kit ready a week before.\n\n## Owners\n- Sam: pricing page and the FAQ\n- Lea: the launch video and the social cut\n\nBudget stays under 6,000 for the first month.\nRisks: pricing needs legal.\n";
    (times, changes, then, now)
}

#[test]
#[ignore = "writes the time scrubber over a note for manual inspection"]
fn dump_scrub() {
    use kolada_ui::scrub::ScrubView;
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch plan".into() }];
    let (times, changes, then, now) = scrub_case();
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(then).collect();
    let changed = kolada_core::history::changed(then, now);
    let shown = times.len() - 4;
    let changed_line = changed.iter().position(|&c| c).unwrap();
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    // Name, theme, window size, what the pointer is over, the line it is on.
    type Case<'a> = (&'a str, &'a Theme, f32, f32, Option<scene::Target>, Option<usize>);
    let cases: [Case; 6] = [
        ("dark", &DARK, 1200.0, 760.0, None, None),
        ("light", &LIGHT, 1200.0, 760.0, None, None),
        ("bring-back-dark", &DARK, 1200.0, 760.0, Some(scene::Target::BringBack(changed_line)), Some(changed_line)),
        ("restore-light", &LIGHT, 1200.0, 760.0, Some(scene::Target::ScrubRestore), None),
        ("half-dark", &DARK, 640.0, 800.0, None, None),
        ("tiny-light", &LIGHT, 360.0, 420.0, None, None),
    ];
    for (name, theme, w, h, hover, hover_line) in cases {
        let input = scene::Input {
            vault_name: "my notes",
            active_note: "Launch plan",
            text: then,
            sidebar: &rows,
            selection: (usize::MAX, usize::MAX),
            lines: &lines,
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            page_fade: 1.0,
            page_date: Some("Edited today, 9:42 AM \u{00B7} Created 28 Sep \u{00B7} 61 words"),
            sidebar_dock: if w < 700.0 { 0.0 } else { 1.0 },
            sidebar_show: if w < 700.0 { 0.0 } else { 1.0 },
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            hover,
            scrub: Some(ScrubView {
                times: &times,
                changes: &changes,
                shown,
                head: shown as f32,
                when: "Tue 29 Sep, 11:20 AM",
                ago: "5 days ago",
                since: "28 Sep",
                changed: &changed,
                past: 1.0,
                alpha: 1.0,
                hover_line,
            }),
            ..Default::default()
        };
        let mut s = scene::build(&dw, &fonts, &input, w, h).unwrap();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, theme, 1.5).unwrap();
        Surface::draw_new(((w * 1.5) as u32, (h * 1.5) as u32, 144), &list).dump(&dir.join(format!("scrub-{name}.bmp"))).unwrap();
    }
}

#[test]
#[ignore = "writes screenshots of text scrolled under the window's top row"]
fn dump_under_top() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let text = "RTL version of the app, complete\nThe side menu, the toolbar and the caret follow the writing's direction.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    for (name, scroll_px) in [("top", 0.0), ("half", 30.0), ("under", 70.0)] {
        let input = scene::Input {
            text,
            lines: &lines,
            scroll_px,
            page_fade: 1.0,
            page_date: Some("Created 1 min ago \u{00B7} 65 words"),
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            compact: true,
            no_margin: true,
            ..Default::default()
        };
        let mut s = scene::build(&dw, &fonts, &input, 1000.0, 300.0).unwrap();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, &DARK, 1.5).unwrap();
        let surface = Surface::draw_new((1500, 450, 144), &list);
        surface.dump(&dir.join(format!("under-top-{name}.bmp"))).unwrap();
    }
}

/// Each button's icon (or letter) in the toolbar and the selection bar has
/// its ink in the middle of the button, so the round highlight (the
/// button's own square) and the glyph share one centre: within a pixel at
/// 2x, in both themes.
#[test]
fn the_bars_glyphs_sit_in_the_middle_of_their_buttons() {
    use scene::Target;
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let text = "# Plan\nShip the public beta soon.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let ship = text.find("Ship").unwrap();
    for theme in [&DARK, &LIGHT] {
        let input = scene::Input { text, lines: &lines, selection: (ship, ship + 4), page_fade: 1.0, toolbar_dock: 0.0, toolbar_show: 1.0, selection_bar: Some(1.0), ..Default::default() };
        let s = scene::build(&dw, &fonts, &input, 1000.0, 400.0).unwrap();
        let list = display::record(&dw, &s, theme, 2.0).unwrap();
        let surface = Surface::draw_new((2000, 800, 192), &list);
        let px = |x: i32, y: i32| surface.pixels[(y * 2000 + x) as usize];
        let buttons: Vec<(scene::Rect, Target)> = s.hits.iter().copied().filter(|(_, t)| matches!(t, Target::Tool(_) | Target::Reading | Target::ToggleToolbar | Target::SelBar(_))).collect();
        assert_eq!(buttons.len(), 7 + 2 + 6);
        for (r, t) in buttons {
            let (x0, y0, x1, y1) = ((r.left * 2.0).round() as i32, (r.top * 2.0).round() as i32, (r.right * 2.0).round() as i32, (r.bottom * 2.0).round() as i32);
            // The capsule's own shade, at the button's corner (outside its round).
            let bg = px(x0 + 1, y0 + 1);
            let differs = |p: u32| {
                let d = |s: u32| ((p >> s) & 0xFF) as i32 - ((bg >> s) & 0xFF) as i32;
                d(16).abs() + d(8).abs() + d(0).abs() > 60
            };
            let (mut a, mut b, mut c, mut e) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            for y in y0..y1 {
                for x in x0..x1 {
                    if differs(px(x, y)) {
                        (a, b, c, e) = (a.min(x), b.min(y), c.max(x + 1), e.max(y + 1));
                    }
                }
            }
            assert!(a < c, "{t:?} draws nothing");
            let (cx, cy) = ((a + c) as f32 / 2.0, (b + e) as f32 / 2.0);
            let (mx, my) = ((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0);
            assert!((cx - mx).abs() <= 1.0 && (cy - my).abs() <= 1.0, "{t:?}: ink centred at ({cx}, {cy}), the button at ({mx}, {my})");
        }
    }
}

/// The toolbar and the selection bar, for looking at: at rest, brought,
/// pointed at, with a tool lit, with Aa's card, with a button's name, the
/// selection bar over words and under them, and pinned; graphite and paper,
/// wide and narrow. Each picture is the window's top (or the selection's
/// part) at 2x, in KOLADA_DUMP_DIR.
#[test]
#[ignore = "writes the toolbar and the selection bar for manual inspection"]
fn dump_toolbar() {
    use scene::{Target, Tool};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch plan".into() }];
    let text = "# Launch plan\nShip the **public beta** on October 12, with the press kit ready a week before the day itself.\n- Owners: Sam, Lea and Omar\n- [ ] Pricing page\n> Speed is a feature.\nBudget stays under 6,000 for the first month.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let bold = text.find("public").unwrap();
    let words = (text.find("press kit").unwrap(), text.find("press kit").unwrap() + "press kit".len());
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    // (name, toolbar (dock, show), selection, active tools, hover, tooltip,
    // Aa card, selection bar, scrolled DIPs)
    type Case = (&'static str, (f32, f32), (usize, usize), &'static [Tool], Option<Target>, Option<Target>, bool, bool, f32);
    let cases: [Case; 10] = [
        ("rest", (0.0, 0.0), (bold, bold), &[], None, None, false, false, 0.0),
        ("revealed", (0.0, 1.0), (bold, bold), &[], None, None, false, false, 0.0),
        ("coming", (0.0, 0.45), (bold, bold), &[], None, None, false, false, 0.0),
        ("hover", (0.0, 1.0), (bold, bold), &[], Some(Target::Tool(Tool::Italic)), None, false, false, 0.0),
        ("active", (0.0, 1.0), (bold + 3, bold + 3), &[Tool::Bold], Some(Target::Tool(Tool::Bold)), Some(Target::Tool(Tool::Bold)), false, false, 0.0),
        ("aa", (0.0, 1.0), (bold, bold), &[], Some(Target::Width(2)), None, true, false, 0.0),
        ("pinned", (1.0, 1.0), (bold, bold), &[], Some(Target::ToggleToolbar), Some(Target::ToggleToolbar), false, false, 0.0),
        ("selbar", (0.0, 0.0), words, &[], Some(Target::SelBar(Tool::Highlight)), Some(Target::SelBar(Tool::Highlight)), false, true, 0.0),
        ("selbar-lit", (0.0, 0.0), (bold, bold + 6), &[Tool::Bold], None, None, false, true, 0.0),
        ("selbar-below", (0.0, 0.0), words, &[], Some(Target::SelBar(Tool::Bold)), Some(Target::SelBar(Tool::Bold)), false, true, 60.0),
    ];
    for (theme_name, theme) in [("dark", &DARK), ("light", &LIGHT)] {
        for (width, size_name) in [(1240.0, "wide"), (560.0, "narrow")] {
            for (name, bar, selection, tools, hover, tooltip, aa, selbar, scrolled) in cases {
                let input = scene::Input {
                    vault_name: "my notes",
                    active_note: "Launch plan",
                    text,
                    sidebar: &rows,
                    selection,
                    lines: &lines,
                    active_tools: tools,
                    scroll_px: scrolled,
                    options: [if theme_name == "dark" { 2 } else { 1 }, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                    page_fade: 1.0,
                    page_date: Some("Edited today, 9:42 AM \u{00B7} Created 28 Sep"),
                    toolbar_dock: bar.0,
                    toolbar_show: bar.1,
                    hover,
                    tooltip,
                    reading: aa.then_some(1.0),
                    selection_bar: selbar.then_some(1.0),
                    ..Default::default()
                };
                let mut s = scene::build(&dw, &fonts, &input, width, 360.0).unwrap();
                s.place_caret((0.0, 0.0));
                let list = display::record(&dw, &s, theme, 2.0).unwrap();
                let surface = Surface::draw_new(((width * 2.0) as u32, 720, 192), &list);
                surface.dump(&dir.join(format!("toolbar-{size_name}-{theme_name}-{name}.bmp"))).unwrap();
            }
        }
    }
}

/// The link field on a link (its actions, Copy link pointed at, then
/// editing where it goes) and on new words, in graphite and paper, in a
/// wide and a narrow window, in KOLADA_DUMP_DIR.
#[test]
#[ignore = "writes the link field for manual inspection"]
fn dump_link() {
    use scene::Target;
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch plan".into() }];
    let text = "# Launch plan\nRead the [press guide](https://example.com/press/guide) before the day, and the brief.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let on = text.find("press guide").unwrap() + 2;
    let brief = (text.find("brief").unwrap(), text.find("brief").unwrap() + 5);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    // (name, existing, editing, query, caret, selection, hover)
    type Case = (&'static str, bool, bool, &'static str, usize, (usize, usize), Option<Target>);
    let cases: [Case; 4] = [
        ("link", true, false, "https://example.com/press/guide", 0, (on, on), None),
        ("link-hover", true, false, "https://example.com/press/guide", 0, (on, on), Some(Target::LinkCopy)),
        ("link-editing", true, true, "https://example.com/press/guide", 31, (on, on), None),
        ("new", false, true, "", 0, brief, None),
    ];
    for (theme_name, theme) in [("dark", &DARK), ("light", &LIGHT)] {
        for (width, size_name) in [(1000.0, "wide"), (380.0, "narrow")] {
            for case in cases {
                let names: Vec<(String, String)> = Vec::new();
                let field = scene::LinkFieldView { query: case.3, caret: case.4, names: &names, picked: None, existing: case.1, editing: case.2, };
                let input = scene::Input {
                    vault_name: "my notes",
                    active_note: "Launch plan",
                    text,
                    sidebar: &rows,
                    selection: case.5,
                    lines: &lines,
                    options: [if theme_name == "dark" { 2 } else { 1 }, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                    page_fade: 1.0,
                    hover: case.6,
                    link_field: Some(field),
                    ..Default::default()
                };
                let mut s = scene::build(&dw, &fonts, &input, width, 300.0).unwrap();
                s.place_caret((0.0, 0.0));
                let list = display::record(&dw, &s, theme, 2.0).unwrap();
                let surface = Surface::draw_new(((width * 2.0) as u32, 600, 192), &list);
                surface.dump(&dir.join(format!("link-{size_name}-{theme_name}-{}.bmp", case.0))).unwrap();
            }
        }
    }
}

/// A mostly-Arabic title that starts with a Latin name, at three widths
/// (one line, two, three), in graphite, in KOLADA_DUMP_DIR.
#[test]
#[ignore = "writes mixed right-to-left text for manual inspection"]
fn dump_bidi() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Lantern".into() }];
    let text = "Northwind كتبوا فكرة جديدة في دفتر اسمه (Atlas 5.5) بعنوان \"Lantern\" تجمع لك خريطة 3D لكل مدينة تزورها وتحفظ لك فيها ملاحظاتك وصورك بلمسة\nNorthwind كتبوا فكرة جديدة في دفتر اسمه (Atlas 5.5) بعنوان \"Lantern\" تجمع لك خريطة 3D لكل مدينة تزورها وتحفظ لك فيها ملاحظاتك وصورك بلمسة\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    for (width, name) in [(1600.0, "wide"), (1000.0, "mid"), (600.0, "narrow")] {
        let input = scene::Input {
            vault_name: "my notes",
            active_note: "Lantern",
            text,
            sidebar: &rows,
            selection: (0, 0),
            lines: &lines,
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            page_fade: 1.0,
            page_date: Some("Edited just now \u{00B7} Created today \u{00B7} 88 words"),
            ..Default::default()
        };
        let mut s = scene::build(&dw, &fonts, &input, width, 420.0).unwrap();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, &DARK, 2.0).unwrap();
        let surface = Surface::draw_new(((width * 2.0) as u32, 840, 192), &list);
        surface.dump(&dir.join(format!("bidi-{name}.bmp"))).unwrap();
    }
}

/// How the page reads, for looking at: the Aa card at the smallest and the
/// largest text size, the options' text size row (pointed at), and the
/// side menu docked with Wide chosen and related notes on, in a window too
/// narrow for both and one wide enough; graphite and paper, in
/// KOLADA_DUMP_DIR.
#[test]
#[ignore = "writes the Aa card, the text size row and Wide beside the side menu for manual inspection"]
fn dump_reading() {
    use kolada_core::related::Kind;
    use scene::{Setting, Target};
    let dw = factory();
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch plan".into() }, SidebarRow { depth: 0, kind: RowKind::Note, name: "Motion studies".into() }];
    let text = "# Launch plan\nShip the **public beta** on October 12, with the press kit ready a week before the day itself, and the launch video cut to under a minute.\n## Owners\n- Sam, Lea and Omar\n- [ ] Pricing page\n> Speed is a feature.\nBudget stays under 6,000 for the first month, with a review at the end of it.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let chips = [
        scene::Chip { name: "Motion studies", detail: "springs \u{00B7} physical \u{00B7} tweens", kind: Kind::Similar, strength: 0.9, y: 220.0, dx: 0.0, alpha: 1.0 },
        scene::Chip { name: "Weekly review", detail: "links to this note", kind: Kind::Backlink, strength: 0.4, y: 278.0, dx: 0.0, alpha: 1.0 },
    ];
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    for (theme_name, theme) in [("dark", &DARK), ("light", &LIGHT)] {
        let theme_option = if theme_name == "dark" { 2 } else { 1 };
        // The Aa card under the toolbar, the note at the size it says.
        for (size, hover) in [(13u8, Some(Target::TextSize(true))), (0, None), (23, Some(Target::TextSize(false)))] {
            let fonts = Fonts::new(scene::text_scale(size), 0);
            let mut options = [0; scene::SETTINGS];
            options[Setting::Theme.index()] = theme_option;
            options[Setting::TextSize.index()] = size;
            let input = scene::Input {
                vault_name: "my notes",
                active_note: "Launch plan",
                text,
                sidebar: &rows,
                lines: &lines,
                options,
                page_fade: 1.0,
                page_date: Some("Edited today, 9:42 AM \u{00B7} Created 28 Sep"),
                toolbar_dock: 0.0,
                toolbar_show: 1.0,
                hover,
                reading: Some(1.0),
                no_margin: true,
                ..Default::default()
            };
            let mut s = scene::build(&dw, &fonts, &input, 1240.0, 420.0).unwrap();
            s.place_caret((0.0, 0.0));
            let list = display::record(&dw, &s, theme, 2.0).unwrap();
            let surface = Surface::draw_new((2480, 840, 192), &list);
            surface.dump(&dir.join(format!("reading-aa-{}-{theme_name}.bmp", scene::text_size(size)))).unwrap();
        }
        // The options, in two columns and in one, the text size's larger
        // pointed at.
        let fonts = Fonts::new(1.0, 0);
        for (width, height, size) in [(1000.0, 820.0, 0u8), (520.0, 820.0, 23), (420.0, 700.0, 13)] {
            let mut options = [0; scene::SETTINGS];
            options[Setting::Theme.index()] = theme_option;
            options[Setting::TextSize.index()] = size;
            let hover = scene::step_text_size(size, true).map(|v| Target::Choice(Setting::TextSize, v));
            let input = scene::Input {
                vault_name: "my notes",
                vault_path: "C:\\Users\\me\\Documents\\Kolada",
                active_note: "Launch plan",
                text,
                sidebar: &rows,
                lines: &lines,
                options,
                options_open: true,
                page_fade: 1.0,
                sidebar_dock: 1.0,
                sidebar_show: 1.0,
                toolbar_dock: 1.0,
                toolbar_show: 1.0,
                hover,
                ..Default::default()
            };
            let mut s = scene::build(&dw, &fonts, &input, width, height).unwrap();
            s.place_caret((0.0, 0.0));
            let list = display::record(&dw, &s, theme, 1.5).unwrap();
            let surface = Surface::draw_new(((width * 1.5) as u32, (height * 1.5) as u32, 144), &list);
            surface.dump(&dir.join(format!("reading-options-{width}-{theme_name}.bmp"))).unwrap();
        }
        // The side menu docked, Wide chosen, related notes on: too narrow
        // for both, the note is Wide and the margin gives way; wide enough,
        // both.
        for width in [1400.0, 2000.0] {
            let mut options = [0; scene::SETTINGS];
            options[Setting::Theme.index()] = theme_option;
            options[Setting::LineWidth.index()] = 2;
            let input = scene::Input {
                vault_name: "my notes",
                active_note: "Launch plan",
                text,
                sidebar: &rows,
                lines: &lines,
                options,
                chips: &chips,
                page_fade: 1.0,
                page_date: Some("Edited today, 9:42 AM \u{00B7} Created 28 Sep"),
                sidebar_dock: 1.0,
                sidebar_show: 1.0,
                toolbar_dock: 0.0,
                toolbar_show: 0.0,
                ..Default::default()
            };
            let mut s = scene::build(&dw, &fonts, &input, width, 600.0).unwrap();
            s.place_caret((0.0, 0.0));
            let list = display::record(&dw, &s, theme, 1.0).unwrap();
            let surface = Surface::draw_new((width as u32, 600, 96), &list);
            surface.dump(&dir.join(format!("reading-wide-{width}-{theme_name}.bmp"))).unwrap();
        }
    }
}

/// A blink frame, the caret's fill left out and then put back, repaints
/// the caret's sliver at 4K and nothing else.
#[test]
fn a_blink_frame_repaints_only_the_caret() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let view = (3840, 2256, 288);
    let text = format!("# Plan\n{}", "A line of body text that is long enough to wrap onto more than one line.\n".repeat(20));
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(&text).collect();
    let input = scene::Input {
        vault_name: "my notes",
        active_note: "Plan",
        text: &text,
        lines: &lines,
        // The caret in the second line's first word.
        selection: (9, 9),
        page_fade: 1.0,
        sidebar_dock: 1.0,
        sidebar_show: 1.0,
        toolbar_dock: 1.0,
        toolbar_show: 1.0,
        ..Default::default()
    };
    let hidden = scene::build(&dw, &fonts, &input, 1280.0, 752.0).unwrap();
    let mut shown = scene::build(&dw, &fonts, &input, 1280.0, 752.0).unwrap();
    shown.place_caret((0.0, 0.0));
    let c = shown.caret.expect("a caret");
    let record = |s: &Scene| display::record(&dw, s, &DARK, 3.0).unwrap();
    let mut surface = Surface::draw_new(view, &record(&shown));
    for scene in [&hidden, &shown] {
        surface.draw(view, &record(scene));
        let damage = surface.last_damage.clone().expect("a partial frame");
        assert_eq!(damage.len(), 1, "one rectangle: {damage:?}");
        let (l, t, r, b) = damage[0];
        let px = |v: f32| (v * 3.0).round() as i32;
        assert!(l <= px(c.left) && t <= px(c.top) && r >= px(c.right) && b >= px(c.bottom), "it covers the caret: {:?} {c:?}", damage[0]);
        assert!(r - l <= px(c.width()) + 6 && b - t <= px(c.height()) + 6, "and no more: {:?} {c:?}", damage[0]);
        println!("a blink frame at 4K repaints {}x{} px", r - l, b - t);
    }
}

/// What a frame and a keystroke may cost, ms: what notesy.ink claims for
/// theirs, so failing one means losing the claim (ours are about 0.07 and
/// 0.08-0.13). Held by tools/speed-gate.sh.
const FRAME_BUDGET_MS: f64 = 0.1;
const KEYSTROKE_BUDGET_MS: f64 = 0.3;

/// `measure` (a median, ms) within `budget`; a busy machine gets a second
/// try before it fails.
fn within(budget: f64, what: &str, measure: impl Fn() -> f64) {
    let first = measure();
    if first <= budget {
        return;
    }
    let second = measure();
    assert!(second <= budget, "{what}: {first:.3} ms, then {second:.3} ms, over its {budget} ms budget");
}

#[test]
#[ignore = "speed gate: one frame at 4K within its budget (release build)"]
fn time_a_frame() {
    within(FRAME_BUDGET_MS, "a frame at 4K", frame_ms);
}

#[test]
#[ignore = "speed gate: a keystroke mid-way down a 6,000-line note at 4K (release build)"]
fn time_a_keystroke_in_a_long_note() {
    within(KEYSTROKE_BUDGET_MS, "a keystroke in a long note", || keystroke_ms(false));
}

#[test]
#[ignore = "speed gate: a keystroke under a 6,000-line note's title, as it opens (release build)"]
fn time_a_keystroke_under_the_title() {
    within(KEYSTROKE_BUDGET_MS, "a keystroke under the title", || keystroke_ms(true));
}

/// One keystroke's frame at 4K: the median of 30, ms.
fn frame_ms() -> f64 {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let block = concat!(
        "## Section heading\n",
        "A paragraph with **bold**, *italic*, `code` and a [[Link]] that is long enough to wrap onto a second line in the column.\n",
        "- first item with ==marked== words\n",
        "- [ ] a task to do\n",
        "- [x] a task done\n",
        "> a quote, muted\n",
        "1. numbered\n",
        "\n",
    );
    let text = block.repeat(40);
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(&text).collect();
    let rows: Vec<SidebarRow> = (0..40).map(|i| SidebarRow { depth: 0, kind: RowKind::Note, name: format!("Note {i}") }).collect();
    let view = (3804, 2262, 288);
    let mut surface = Surface::draw_new((16, 16, 96), &display::record(&dw, &Scene::default(), &DARK, 1.0).unwrap());
    let mut times = Vec::new();
    for i in 0..30 {
        let caret = 200 + i * 7;
        let start = std::time::Instant::now();
        let input = scene::Input {
            vault_name: "notes",
            active_note: "Timing",
            text: &text,
            sidebar: &rows,
            scroll_line: 0,
            scroll_px: 0.0,
            selection: (caret, caret),
            lines: &lines,
            active_tools: &[],
            tree_top: 0,
            search: None,
            options_open: false,
            options: [0; scene::SETTINGS],
            chips: &[],
            dragged: None,
            drop_at: None,
            page_fade: 1.0,
            highlights_open: false,
            peek: None,
            whisper: None,
            popup: None,
            sidebar_dock: 1.0,
            sidebar_show: 1.0,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            hover: None,
            sidebar_wave: &[],
            toolbar_wave: &[],
            maximized: false,
            ..Default::default()
        };
        let mut s = scene::build(&dw, &fonts, &input, 1268.0, 754.0).unwrap();
        let built = start.elapsed();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, &DARK, 3.0).unwrap();
        let recorded = start.elapsed();
        surface.draw(view, &list);
        let drawn = start.elapsed();
        times.push((built, recorded, drawn));
    }
    times.sort_by_key(|t| t.2);
    let (b, r, d) = times[times.len() / 2];
    println!("frame median: scene {:.2} ms, +record {:.2} ms, +paint {:.2} ms (total {:.2} ms)",
        b.as_secs_f64() * 1e3, (r - b).as_secs_f64() * 1e3, (d - r).as_secs_f64() * 1e3, d.as_secs_f64() * 1e3);
    d.as_secs_f64() * 1e3
}

#[test]
#[ignore = "writes the margin (related notes and a peek) for manual inspection"]
fn dump_margin() {
    use kolada_core::related::Kind;
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let text = "# Product launch\nShip the public beta on **October 12**.\n- [x] Pricing page\n- [ ] Press kit\nThe launch video should feel physical and quiet: springs, not tweens.";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Product launch".into() }];
    let chips = [
        scene::Chip { name: "Motion studies", detail: "springs · physical · tweens", kind: Kind::Similar, strength: 0.9, y: 220.0, dx: 0.0, alpha: 1.0 },
        scene::Chip { name: "Weekly review", detail: "links to this note", kind: Kind::Backlink, strength: 0.4, y: 278.0, dx: 0.0, alpha: 1.0 },
        scene::Chip { name: "Design principles", detail: "Quiet at rest, alive on intent.", kind: Kind::Highlight, strength: 0.7, y: 336.0, dx: 6.0, alpha: 0.7 },
    ];
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let popup_text = "# Motion studies
Springs with a little **overshoot** read as physical.
- [x] Pills as strings
- [ ] Chips drift in
> Tweens feel scripted.

Keep motion quiet and short. ==Physical, not flashy.==
".repeat(3);
    let popup_lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(&popup_text).collect();
    for (name, theme, with_popup) in [("margin-dark", &DARK, false), ("margin-light", &LIGHT, false), ("popup-dark", &DARK, true), ("popup-light", &LIGHT, true)] {
        let input = scene::Input {
            vault_name: "demo",
            active_note: "Product launch",
            text,
            sidebar: &rows,
            scroll_line: 0,
            scroll_px: 0.0,
            selection: (text.len(), text.len()),
            lines: &lines,
            active_tools: &[],
            tree_top: 0,
            search: None,
            options_open: false,
            options: [0; scene::SETTINGS],
            chips: &chips,
            dragged: None,
            drop_at: None,
            page_fade: 1.0,
            highlights_open: false,
            peek: Some(scene::Peek {
                chip: 0,
                title: "Motion studies",
                body: "Springs with a little overshoot read as physical.\nTweens feel scripted.\nKeep motion quiet and short.",
                alpha: 1.0,
            }),
            whisper: Some(scene::Whisper { text: "Saved as Product launch", alpha: 1.0 }),
            popup: with_popup.then(|| scene::PopupView {
                marked: None,
                name: "Motion studies",
                text: &popup_text,
                lines: &popup_lines,
                selection: (40, 40),
                scroll: 30.0,
                anchor: 220.0,
                alpha: 1.0,
                hover: None,
                scrolling: false,
                bar_alpha: 1.0,
                offset: (0.0, 0.0),
                spelling: None,
                note: None,
            }),
            sidebar_dock: 1.0,
            sidebar_show: 1.0,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            hover: Some(scene::Target::Chip(0)),
            sidebar_wave: &[],
            toolbar_wave: &[],
            maximized: false,
            ..Default::default()
        };
        // Wide enough for the note at its width and the margin beside it.
        let mut s = scene::build(&dw, &fonts, &input, 1400.0, 754.0).unwrap();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, theme, 1.5).unwrap();
        let surface = Surface::draw_new((2100, 1131, 144), &list);
        surface.dump(&dir.join(format!("{name}.bmp"))).unwrap();
    }
}

#[test]
#[ignore = "writes a filmstrip of the side pill's string for manual inspection"]
fn dump_wave_strip() {
    use crate::scene::{Fill, Rect, WaveShape};
    use crate::wave::{self, SAMPLES, Wave};
    let dw = factory();
    let pill = Rect::new(5.0, 40.0, 11.0, 360.0);
    let mut w = Wave::default();
    // (seconds to run, pointer): approach, slide down along it, let go.
    let script: Vec<(f32, Option<(f32, f32)>)> = (0..40)
        .map(|k| {
            let t = k as f32;
            let pointer = if k < 14 { Some((60.0 - t * 3.0, 150.0)) } else if k < 26 { Some((20.0, 150.0 + (t - 14.0) * 12.0)) } else { None };
            (1.0 / 30.0, pointer)
        })
        .collect();
    let mut frames = Vec::new();
    for (k, (dt, pointer)) in script.iter().enumerate() {
        let target = wave::target(pill, true, *pointer);
        for _ in 0..2 {
            w.step(dt / 2.0, pill.height(), &target);
        }
        if k % 3 == 0 {
            frames.push((w.d, *pointer));
        }
    }
    let mut fills = Vec::new();
    for (i, (d, pointer)) in frames.iter().enumerate() {
        let dx = i as f32 * 44.0;
        let r = pill.offset(dx + 8.0, 0.0);
        let shape = Some(WaveShape { vertical: true, flipped: false, d: *d });
        fills.push(Fill { wave: shape, ..Fill::faded(r, 3.0, crate::scene::Ink::Text, 0.9) });
        if let Some((x, y)) = pointer {
            let p = Rect::new(dx + 8.0 + x - 2.0, y - 2.0, dx + 8.0 + x + 2.0, y + 2.0);
            fills.push(Fill::solid(p, 2.0, crate::scene::Ink::Accent));
        }
        let _ = SAMPLES;
    }
    let scene = Scene { fills, ..Scene::default() };
    let surface = render(&dw, &scene, &DARK, ((frames.len() as u32 * 44 + 20) * 2, 400 * 2, 192), THREADS);
    surface.dump(&std::path::Path::new(&std::env::var("KOLADA_DUMP_DIR").unwrap()).join("wave-strip.bmp")).unwrap();
}

#[test]
#[ignore = "writes menus, the trash, the options and scroll bars for manual inspection"]
fn dump_menus() {
    use crate::scene::{MenuItem, MenuView, ScrollView, Setting, Target, TrashEntry};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![
        SidebarRow { depth: 0, kind: RowKind::Folder { expanded: true }, name: "Notes".into() },
        SidebarRow { depth: 1, kind: RowKind::Note, name: "Meeting notes".into() },
        SidebarRow { depth: 1, kind: RowKind::Note, name: "Product launch".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Reading list".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "ترند الأخبار".into() },
    ];
    let text = "Product launch\nShip the public beta on **October 12**.\n- [x] Pricing page\n- [ ] Press kit\n".repeat(12);
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(&text).collect();
    let item = |icon: &'static str, label: &str, keys: &'static str| MenuItem { icon, label: label.into(), keys, ..MenuItem::default() };
    let items = vec![
        item("\u{E8C6}", "Cut", "Ctrl+X"),
        item("\u{E8C8}", "Copy", "Ctrl+C"),
        item("\u{E77F}", "Paste", "Ctrl+V"),
        MenuItem { divider: true, ..item("\u{E8DD}", "Bold", "Ctrl+B") },
        item("\u{E8DB}", "Italic", "Ctrl+I"),
        item("\u{E7E6}", "Highlight", ""),
        item("\u{E71B}", "Link", "Ctrl+K"),
        MenuItem { divider: true, ..item("\u{E8B3}", "Select all", "Ctrl+A") },
        MenuItem { divider: true, danger: true, ..item("\u{E74D}", "Delete note", "") },
    ];
    let trash = [
        TrashEntry { name: "Old ideas".into(), when: "Just now".into() },
        TrashEntry { name: "ملاحظات قديمة".into(), when: "2 h ago".into() },
        TrashEntry { name: "Draft".into(), when: "Yesterday".into() },
    ];
    let long_name = "ملاحظات اجتماع فريق التصميم حول إطلاق المنتج الجديد والجدول الزمني";
    let popup_text = format!("{long_name}\n{}", "سطر من النص العربي الذي يستمر ويستمر حتى يلتف.\n".repeat(30));
    let popup_lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(&popup_text).collect();
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let send = [
        MenuItem { icon: "\u{E8BD}", label: "New chat in Claude".into(), keys: "App", ..MenuItem::default() },
        MenuItem { icon: "\u{E8BD}", label: "New chat in ChatGPT".into(), keys: "Browser", ..MenuItem::default() },
        MenuItem { icon: "\u{E72D}", label: "Export note".into(), divider: true, ..MenuItem::default() },
        MenuItem { icon: "\u{E8C8}", label: "Copy".into(), ..MenuItem::default() },
    ];
    let matches = kolada_editor::find::find_all(&text, "launch");
    let ticks: Vec<f32> = (0..matches.len()).map(|i| i as f32 / matches.len() as f32).collect();
    let names = [("Launch checklist".to_string(), "Work".to_string()), ("Launch video".to_string(), String::new()), ("Product launch".to_string(), "Work".to_string())];
    let marks = [
        scene::Landmark { at: 0.0, text: "Goals", level: 2, here: false },
        scene::Landmark { at: 0.18, text: "Timeline and owners", level: 2, here: true },
        scene::Landmark { at: 0.2, text: "Week one", level: 3, here: false },
        scene::Landmark { at: 0.45, text: "Pricing", level: 2, here: false },
        scene::Landmark { at: 0.62, text: "ملاحظات الإطلاق", level: 2, here: false },
        scene::Landmark { at: 0.9, text: "Risks we accept", level: 2, here: false },
    ];
    let cases: [(&str, &Theme, Option<Target>); 10] = [
        ("map-dark", &DARK, Some(Target::Heading(3))),
        ("find-dark", &DARK, Some(Target::FindNext)),
        ("suggest-light", &LIGHT, Some(Target::Suggestion(1))),
        ("send-dark", &DARK, Some(Target::MenuItem(0))),
        ("compact-light", &LIGHT, Some(Target::ToMain)),
        ("menu-dark", &DARK, Some(Target::MenuItem(4))),
        ("row-dots-light", &LIGHT, Some(Target::RowMenu(3))),
        ("trash-light", &LIGHT, Some(Target::TrashRow(1))),
        ("options-full-dark", &DARK, Some(Target::Choice(Setting::Font, 1))),
        ("popup-rtl-dark", &DARK, Some(Target::PopupScrollbar)),
    ];
    for (name, theme, hover) in cases {
        let input = scene::Input {
            vault_name: "demo",
            active_note: "Notes/Product launch",
            text: &text,
            sidebar: &rows,
            selection: (20, 20),
            lines: &lines,
            options_open: name == "options-full-dark",
            options: [2, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            opened_in: Some(27.4),
            menu: match name {
                "menu-dark" => Some(MenuView { x: 520.0, y: 180.0, align_right: false, items: &items, alpha: 1.0 }),
                "send-dark" => Some(MenuView { x: 1268.0 - 138.0 - 20.0, y: 38.0, align_right: true, items: &send, alpha: 1.0 }),
                _ => None,
            },
            menu_anchor: (name == "send-dark").then_some(Target::SendTo),
            find: (name == "find-dark").then_some(scene::FindView {
                query: "launch",
                query_caret: 6,
                replace: Some(("release", 7)),
                focus: 1,
                count: matches.len(),
                current: Some(2),
            }),
            matches: if name == "find-dark" { &matches } else { &[] },
            ticks: if name == "find-dark" { &ticks } else { &[] },
            landmarks: if name == "map-dark" { &marks } else { &[] },
            page_date: Some("Edited today, 3:42 PM \u{00B7} Created 12 Sep"),
            row_date: Some((3, "yesterday, 9:10 PM")),
            vault_path: r"C:\Users\me\Documents\my notes",
            suggestions: (name == "suggest-light").then_some(scene::Suggestions { names: &names, selected: 0, create: Some("Launch") }),
            compact: name == "compact-light",
            no_margin: name == "compact-light",
            menu_row: None,
            trash: (name == "trash-light").then_some(&trash[..]),
            scrollbar: Some(ScrollView { at: 0.3, size: 0.25, alpha: 1.0 }),
            page_fade: 1.0,
            popup: (name == "popup-rtl-dark").then(|| scene::PopupView {
                marked: None,
                name: long_name,
                text: &popup_text,
                lines: &popup_lines,
                selection: (0, 0),
                scroll: 160.0,
                anchor: 200.0,
                alpha: 1.0,
                hover,
                scrolling: false,
                bar_alpha: 1.0,
                offset: (0.0, 0.0),
                spelling: None,
                note: None,
            }),
            hover: if name.starts_with("menu") || name.starts_with("send") { hover } else { hover.or(Some(Target::Row(2))) },
            sidebar_dock: if name == "compact-light" { 0.0 } else { 1.0 },
            sidebar_show: if name == "compact-light" { 0.0 } else { 1.0 },
            toolbar_dock: if name == "compact-light" { 0.0 } else { 1.0 },
            toolbar_show: if name == "compact-light" { 0.0 } else { 1.0 },
            ..Default::default()
        };
        let (w, h) = if name == "compact-light" { (580.0, 720.0) } else { (1268.0, 754.0) };
        let mut s = scene::build(&dw, &fonts, &input, w, h).unwrap();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, theme, 1.5).unwrap();
        let surface = Surface::draw_new(((w * 1.5) as u32, (h * 1.5) as u32, 144), &list);
        surface.dump(&dir.join(format!("{name}.bmp"))).unwrap();
    }
}

/// The window sizes the layout must hold at, DIPs: what Windows snaps to on
/// common screens (a 3840 x 2400 screen at 300%, 1920 x 1080 at 125%), and
/// small and large extremes.
pub(crate) const SIZES: [(&str, f32, f32); 12] = [
    ("full-300", 1280.0, 800.0),
    ("half-300", 640.0, 800.0),
    ("third-300", 427.0, 800.0),
    ("two-thirds-300", 853.0, 800.0),
    ("quarter-300", 640.0, 400.0),
    ("full-125", 1536.0, 864.0),
    ("half-125", 768.0, 864.0),
    ("third-125", 512.0, 864.0),
    ("quarter-125", 768.0, 432.0),
    ("small", 480.0, 360.0),
    ("tiny", 360.0, 300.0),
    ("wide", 2560.0, 1400.0),
];

#[test]
#[ignore = "writes the window at every size and state for manual inspection"]
fn dump_sizes() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![
        SidebarRow { depth: 0, kind: RowKind::Folder { expanded: true }, name: "Notes".into() },
        SidebarRow { depth: 1, kind: RowKind::Note, name: "Meeting notes with a long name".into() },
        SidebarRow { depth: 1, kind: RowKind::Note, name: "Launch plan".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Reading list".into() },
    ];
    let text = "# Launch plan\nShip the **public beta** on October 12, then see [[Pricing ideas]] #launch.\n- [ ] Press kit\n- [x] Pricing page\n> Speed is a feature.\nA long paragraph that has to wrap when the window is narrow, so the column's width shows in how the words flow from line to line.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let devices = [kolada_ui::quick::DeviceRow { name: "Windows laptop", look: kolada_ui::quick::Look::Laptop, this: true }, kolada_ui::quick::DeviceRow { name: "iPhone", look: kolada_ui::quick::Look::Phone, this: false }];
    // (name, sidebar (dock, show), toolbar (dock, show), options open, the
    // side menu's width as dragged, its edge pointed at)
    let states = [
        ("docked", (1.0, 1.0), (1.0, 1.0), false, 0.0, false),
        ("tucked", (1.0, 1.0), (0.0, 0.0), false, 0.0, false),
        ("focus", (0.0, 0.0), (0.0, 0.0), false, 0.0, false),
        ("revealed", (0.0, 1.0), (0.0, 0.0), false, 0.0, false),
        ("options", (1.0, 1.0), (1.0, 1.0), true, 0.0, false),
        ("wide-menu", (1.0, 1.0), (1.0, 1.0), false, crate::chrome::SIDEBAR_MOST, true),
        ("narrow-menu", (1.0, 1.0), (1.0, 1.0), false, crate::chrome::SIDEBAR_LEAST, false),
    ];
    for (size, width, height) in SIZES {
        for (state, side, top, options_open, set, edge) in states {
            // As the app has it: where the side menu cannot dock, it is
            // tucked away.
            let side = if side.0 > 0.0 && !crate::chrome::sidebar_fits(width) { (0.0, 0.0) } else { side };
            let input = scene::Input {
                vault_name: "my notes",
                active_note: "Notes/Launch plan",
                text,
                sidebar: &rows,
                selection: (20, 20),
                lines: &lines,
                options_open,
                options: [0; scene::SETTINGS],
                page_fade: 1.0,
                sidebar_dock: side.0,
                sidebar_show: side.1,
                toolbar_dock: top.0,
                toolbar_show: top.1,
                page_date: Some("Edited today, 3:42 PM \u{00B7} 184 words"),
                page_gist: Some(("The public beta on October 12: the press kit, pricing and what is still open", 1.0)),
                sidebar_width: set,
                hover: edge.then_some(scene::Target::SidebarEdge),
                // An update ready: its pill above the side menu's icons.
                updater: true,
                update: Some("0.2.0"),
                devices: options_open.then_some(scene::DevicesView { devices: &devices, pairing: kolada_ui::quick::Pairing::Idle, said: None, facts: &[], this: "PC", naming: None }),
                ..Default::default()
            };
            // Settings opened on its Devices, as the popup's devices chip opens it.
            let input = scene::Input { options_scroll: if options_open { kolada_ui::chrome::devices_scroll(&input, width) } else { 0.0 }, ..input };
            let mut s = scene::build(&dw, &fonts, &input, width, height).unwrap();
            s.place_caret((0.0, 0.0));
            let list = display::record(&dw, &s, &LIGHT, 1.0).unwrap();
            let surface = Surface::draw_new((width as u32, height as u32, 96), &list);
            surface.dump(&dir.join(format!("size-{size}-{state}.bmp"))).unwrap();
        }
    }
}

/// The command palette and the editor's power moves as the app draws them,
/// graphite and paper, wide and in a side-by-side snap
/// (`palette-<state>-<size>-<theme>.bmp`): opened with what ran last,
/// typed into, and scrolled through many matches; and a note with a
/// section folded and three carets.
#[test]
#[ignore = "writes the palette and power moves for manual inspection"]
fn dump_palette() {
    use kolada_ui::palette::{PaletteView, rows_for};
    use kolada_ui::scene::MenuItem;
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![
        SidebarRow { depth: 0, kind: RowKind::Folder { expanded: true }, name: "Travel".into() },
        SidebarRow { depth: 1, kind: RowKind::Note, name: "Trip to Lisbon".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Reading list".into() },
    ];
    let text = "# Trip to Lisbon\nFlights land at 09:40, so the first morning is free.\n## Packing\n- sunscreen\n- adapter plug\n- camera charger\n- walking shoes\n## Days\nAlfama on foot, then the tram up to the castle.\nBelem by tram in the morning, before the crowds.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let item = |label: &str, keys: &'static str| MenuItem { label: label.into(), keys, ..MenuItem::default() };
    let recent = [item("New note", "Ctrl+N"), item("Move line up", "Alt+\u{2191}"), item("Note history", "Ctrl+Shift+H")];
    let typed = [item("Move line up", "Alt+\u{2191}"), item("Move line down", "Alt+\u{2193}"), item("Move note to trash", "")];
    let many: Vec<MenuItem> = kolada_ui::commands::ALL.iter().filter(|c| c.name.contains('e')).map(|c| item(c.name, c.keys(scene::Platform::Windows))).collect();
    let packing = text.find("## Packing").unwrap();
    let folded = [(packing + "## Packing".len(), text.find("- walking shoes").unwrap() + "- walking shoes".len())];
    let tram = text.find("tram").unwrap();
    let tram2 = text.rfind("tram").unwrap();
    let carets = [(tram, tram + 4), (tram2, tram2 + 4)];
    let views: [(&str, Option<PaletteView>); 4] = [
        ("recent", Some(PaletteView { query: "", caret: 0, rows: &recent, first: 0, chosen: 0, drop: 1.0, height: rows_for(3), fill: 0.0 })),
        ("typed", Some(PaletteView { query: "move", caret: 4, rows: &typed, first: 0, chosen: 0, drop: 1.0, height: rows_for(3), fill: 0.0 })),
        ("many", Some(PaletteView { query: "e", caret: 1, rows: &many, first: 3, chosen: 6, drop: 1.0, height: rows_for(many.len()), fill: 3.0 })),
        ("moves", None),
    ];
    for (size, width, height) in [("wide", 1280.0, 800.0), ("snap", 380.0, 800.0)] {
        for (state, palette) in &views {
            let side = if crate::chrome::sidebar_fits(width) { (1.0, 1.0) } else { (0.0, 0.0) };
            let moves = palette.is_none();
            let input = scene::Input {
                vault_name: "my notes",
                active_note: "Travel/Trip to Lisbon",
                text,
                sidebar: &rows,
                selection: if moves { (tram2 + 15, tram2 + 15) } else { (20, 20) },
                carets: if moves { &carets } else { &[] },
                sections_folded: if moves { &folded } else { &[] },
                lines: &lines,
                options: [0; scene::SETTINGS],
                page_fade: 1.0,
                sidebar_dock: side.0,
                sidebar_show: side.1,
                toolbar_dock: 1.0,
                toolbar_show: 1.0,
                palette: palette.as_ref().map(|v| PaletteView { rows: v.rows, ..*v }),
                ..Default::default()
            };
            let mut s = scene::build(&dw, &fonts, &input, width, height).unwrap();
            s.place_caret((0.0, 0.0));
            for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
                let list = display::record(&dw, &s, theme, 1.0).unwrap();
                Surface::draw_new((width as u32, height as u32, 96), &list).dump(&dir.join(format!("palette-{state}-{size}-{theme_name}.bmp"))).unwrap();
            }
        }
    }
}

/// Beam's side-menu row and window button, at every size, graphite and
/// paper: two beams new (`quick-button-<size>-<theme>.bmp`), and none new
/// with the button's menu open (`quick-menu-<size>-<theme>.bmp`); docked
/// where the menu docks and revealed where it is tucked away.
#[test]
#[ignore = "writes the window at every size for manual inspection"]
fn dump_quick_send_button() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![
        SidebarRow { depth: 0, kind: RowKind::Folder { expanded: true }, name: "Notes".into() },
        SidebarRow { depth: 1, kind: RowKind::Note, name: "Launch plan".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Reading list".into() },
    ];
    let text = "# Launch plan\nShip the **public beta** on October 12.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let new = scene::BeamView { unseen: 2, line: "Gate code for Friday", typing: false, menu: None, check: None, keys: "Alt+Z" };
    let menu = scene::BeamView { unseen: 0, line: "From iPhone, 2 h ago", typing: false, menu: Some("Launch plan"), check: None, keys: "Alt+Z" };
    for ((size, width, height), (name, beam)) in SIZES.into_iter().flat_map(|s| [(s, ("button", new)), (s, ("menu", menu))]) {
        let side = if crate::chrome::sidebar_fits(width) { (1.0, 1.0) } else { (0.0, 1.0) };
        let input = scene::Input {
            vault_name: "my notes",
            active_note: "Notes/Launch plan",
            text,
            sidebar: &rows,
            lines: &lines,
            options: [0; scene::SETTINGS],
            page_fade: 1.0,
            sidebar_dock: side.0,
            sidebar_show: side.1,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            quick: Some(beam),
            ..Default::default()
        };
        let s = scene::build(&dw, &fonts, &input, width, height).unwrap();
        for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
            let list = display::record(&dw, &s, theme, 1.0).unwrap();
            Surface::draw_new((width as u32, height as u32, 96), &list).dump(&dir.join(format!("quick-{name}-{size}-{theme_name}.bmp"))).unwrap();
        }
    }
}

#[test]
fn the_gist_fits_beside_the_dates_or_stays_out() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let text = "# Launch plan\nShip it.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let gist = "A gist long enough to be cut at every width the window takes, and then a few words more";
    let (mut shown, mut hidden) = (0, 0);
    for (size, width, height) in SIZES {
        for side in [0.0, 1.0] {
            let side = if crate::chrome::sidebar_fits(width) { side } else { 0.0 };
            let input = scene::Input {
                text,
                lines: &lines,
                options: [0; scene::SETTINGS],
                page_fade: 1.0,
                sidebar_dock: side,
                sidebar_show: side,
                toolbar_dock: 1.0,
                toolbar_show: 1.0,
                page_date: Some("Created 12 March, edited today, 3:42 PM \u{00B7} 1,240 words"),
                page_gist: Some((gist, 1.0)),
                ..Default::default()
            };
            let s = scene::build(&dw, &fonts, &input, width, height).unwrap();
            let line: Vec<&Text> = s.texts.iter().filter(|t| t.ink == Ink::Faint && (t.y - (s.column.top - 24.0)).abs() < 0.5).collect();
            assert!((1..=2).contains(&line.len()), "{size}: {} texts on the date line", line.len());
            for t in &line {
                let m = t.layout.measure();
                assert!(t.x + m.width <= s.column.right + 0.5, "{size}: the date line runs past the column");
                assert!(m.height < 24.0, "{size}: the date line wraps");
            }
            if line.len() == 2 {
                assert!(line[0].layout.width() >= 120.0, "{size}: a gist squeezed in");
                shown += 1;
            } else {
                hidden += 1;
            }
        }
    }
    assert!(shown > 0 && hidden > 0, "shown at {shown} sizes, hidden at {hidden}");
}

#[test]
#[ignore = "writes the social preview, per platform, for manual inspection"]
fn dump_social() {
    use kolada_core::social::PLATFORMS;
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch post".into() }];
    let text = "# Launch post\nToday we're opening the **Kolada beta** to everyone. It's a notes app that opens before you finish the thought: under 30 ms from click to cursor, on a 4K screen.\n\nWhat's inside:\n- Live Markdown that stays out of your way\n- Pictures, links and a vault that is just a folder\n- Win+Alt+N for a note from anywhere\n\nTry it and tell us what you'd change: https://notsy.app/beta\n\n#notes #productivity #windows #launch";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    for (theme, tname) in [(&DARK, "dark"), (&LIGHT, "light")] {
        for platform in PLATFORMS {
            let input = scene::Input {
                vault_name: "my notes",
                active_note: "Launch post",
                text,
                sidebar: &rows,
                selection: (40, 40),
                lines: &lines,
                options: [0; scene::SETTINGS],
                page_fade: 1.0,
                sidebar_dock: 1.0,
                sidebar_show: 1.0,
                toolbar_dock: 1.0,
                toolbar_show: 1.0,
                social_drawer: true,
                social: Some(crate::social::SocialView { platform, scroll: 0.0, offset: (0.0, 0.0) }),
                ..Default::default()
            };
            let mut s = scene::build(&dw, &fonts, &input, 1280.0, 800.0).unwrap();
            s.place_caret((0.0, 0.0));
            let list = display::record(&dw, &s, theme, 1.5).unwrap();
            let surface = Surface::draw_new((1920, 1200, 144), &list);
            surface.dump(&dir.join(format!("social-{}-{tname}.bmp", platform.name().to_lowercase()))).unwrap();
        }
    }
}

/// One keystroke in a 6,000-line note at 4K, as notesy.ink measures: the
/// median of 30 after the glyph cache is warm, ms.
fn keystroke_ms(at_top: bool) -> f64 {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let block = concat!(
        "## Section heading\n",
        "A paragraph with **bold**, *italic*, `code` and a [[Link]] that is long enough to wrap onto a second line in the column.\n",
        "- first item with ==marked== words\n",
        "- [ ] a task to do\n",
        "- [x] a task done\n",
        "> a quote, muted\n",
        "1. numbered\n",
        "\n",
    );
    // In the middle of the note, at the end of a paragraph; or (`at_top`)
    // under its title, scrolled to the top, as a note opens.
    let mut editor = kolada_editor::Editor::new(format!("Long note
{}", block.repeat(750)));
    let line = editor.outline()[if at_top { 1 } else { 3001 }];
    editor.set_selection(kolada_editor::Selection::caret(line.start));
    let rows: Vec<SidebarRow> = (0..40).map(|i| SidebarRow { depth: 0, kind: RowKind::Note, name: format!("Note {i}") }).collect();
    let view = (3804, 2262, 288);
    let mut surface = Surface::draw_new((16, 16, 96), &display::record(&dw, &Scene::default(), &DARK, 1.0).unwrap());
    // As the app has them (KOLADA_AS_APP): the spelling marks, the pictures'
    // store, the date line.
    let as_app = std::env::var_os("KOLADA_AS_APP").is_some();
    let marks = crate::spell::Marks::new();
    let images = std::cell::RefCell::new(crate::images::Images::new());
    let mut times = Vec::new();
    for _ in 0..40 {
        // KOLADA_GAP (ms): idle between keys, as typing is, so the CPU sleeps.
        if let Some(gap) = std::env::var("KOLADA_GAP").ok().and_then(|g| g.parse().ok()) {
            std::thread::sleep(std::time::Duration::from_millis(gap));
        }
        let start = std::time::Instant::now();
        editor.insert("a");
        let edited = start.elapsed();
        let caret = editor.selection().head;
        // The view as the app keeps it: scrolled so the caret's paragraph
        // is a few lines down.
        let top = if at_top { 0 } else { editor.outline()[2995].start };
        let input = scene::Input {
            vault_name: "notes",
            active_note: "Long note",
            text: editor.text(),
            sidebar: &rows,
            scroll_line: top,
            selection: (caret, caret),
            lines: editor.outline(),
            spelling: as_app.then_some(&marks),
            images: as_app.then_some(&images),
            vault_path: if as_app { r"C:\notes" } else { "" },
            page_date: as_app.then_some("Edited just now \u{00B7} 45,000 words"),
            scale: 3.0,
            options: [0; scene::SETTINGS],
            page_fade: 1.0,
            sidebar_dock: 1.0,
            sidebar_show: 1.0,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            ..Default::default()
        };
        let mut s = scene::build(&dw, &fonts, &input, 1268.0, 754.0).unwrap();
        let built = start.elapsed();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, &DARK, 3.0).unwrap();
        let recorded = start.elapsed();
        surface.draw(view, &list);
        let drawn = start.elapsed();
        times.push((edited, built, recorded, drawn));
    }
    // The first frames warm the glyph cache; the rest are what typing costs.
    let mut times = times.split_off(10);
    times.sort_by_key(|t| t.3);
    let (e, b, r, d) = times[times.len() / 2];
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
    println!(
        "keystroke median at 4K, 6,000 lines: edit {:.2} ms, +scene {:.2} ms, +record {:.2} ms, +paint {:.2} ms (total {:.2} ms)",
        ms(e),
        ms(b - e),
        ms(r - b),
        ms(d - r),
        ms(d)
    );
    ms(d)
}

/// A note typed into: the frame before, and the one after a character (the
/// caret moves, the word count changes, the line rewraps).
fn typed_frames(dw: &scene::Eng, fonts: &Fonts) -> (display::DisplayList, display::DisplayList, display::DisplayList) {
    let rows: Vec<SidebarRow> = (0..12).map(|i| SidebarRow { depth: 0, kind: RowKind::Note, name: format!("Note {i}") }).collect();
    let before = "# Plan\nShip the **beta** on the twelfth, with a [[Press kit]] and a picture.\n- [ ] Tell the team\n> Speed is a feature.\nMore words below the fold, so the page has something under the line being typed.\n".repeat(6);
    let at = before.find("twelfth").unwrap() + 7;
    let after = format!("{}s{}", &before[..at], &before[at..]);
    let list = |text: &str, caret: usize| {
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        let input = scene::Input {
            vault_name: "notes",
            active_note: "Plan",
            text,
            sidebar: &rows,
            selection: (caret, caret),
            lines: &lines,
            options: [0; scene::SETTINGS],
            page_fade: 1.0,
            sidebar_dock: 1.0,
            sidebar_show: 1.0,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            page_date: Some(if text.len() > before.len() { "Edited just now \u{00B7} 91 words" } else { "Edited just now \u{00B7} 90 words" }),
            ..Default::default()
        };
        let mut s = scene::build(dw, fonts, &input, 1268.0, 754.0).unwrap();
        s.place_caret((0.0, 0.0));
        display::record(dw, &s, &DARK, 3.0).unwrap()
    };
    (list(&before, at), list(&after, at + 1), list(&after, at + 1))
}

#[test]
fn repaints_only_what_changed_and_exactly() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let (before, after, again) = typed_frames(&dw, &fonts);
    let view = (3804, 2262, 288);
    let mut surface = Surface::draw_new((16, 16, 96), &display::record(&dw, &Scene::default(), &DARK, 1.0).unwrap());
    surface.draw(view, &before);
    assert!(surface.last_damage.is_none(), "a new size paints everything");
    surface.draw(view, &after);
    let rects = surface.last_damage.clone().expect("a keystroke paints part of the window");
    let changed: i64 = rects.iter().map(|&(l, t, r, b)| (r - l) as i64 * (b - t) as i64).sum();
    let whole = view.0 as i64 * view.1 as i64;
    assert!(changed * 10 < whole, "a keystroke repaints {changed} of {whole} pixels: {rects:?}");
    // What it painted, over the frame before, is the frame after, exactly.
    let full = Surface::draw_new(view, &after);
    let differ = surface.pixels().iter().zip(full.pixels()).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 0, "partial repaint differs from a full one in {differ} pixels");
    // The same frame again paints nothing.
    surface.draw(view, &again);
    assert_eq!(surface.last_damage.as_deref(), Some(&[][..]), "an unchanged frame paints nothing");
}

/// The side menu revealed from focus mode floats over a docked toolbar
/// whole: inside it, nothing of the toolbar shows. Revealed, it is its own
/// width, whatever it was dragged to docked.
#[test]
fn a_revealed_side_menu_covers_the_toolbar() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Plan".into() }];
    let text = "# Plan\nShip it.";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    // Narrow enough that the toolbar, centred over the note, reaches under
    // the revealed menu.
    let (width, height) = (760.0, 600.0);
    let draw = |toolbar: f32| {
        let input = scene::Input {
            vault_name: "notes",
            active_note: "Plan",
            text,
            sidebar: &rows,
            lines: &lines,
            options: [0; scene::SETTINGS],
            page_fade: 1.0,
            sidebar_dock: 0.0,
            sidebar_show: 1.0,
            toolbar_dock: toolbar,
            toolbar_show: toolbar,
            sidebar_width: crate::chrome::SIDEBAR_MOST,
            ..Default::default()
        };
        let s = scene::build(&dw, &fonts, &input, width, height).unwrap();
        let panel = s.hits.iter().rev().find(|(_, t)| *t == scene::Target::SidebarPanel).map(|(r, _)| *r).unwrap();
        let bold = s.hits.iter().find(|(_, t)| *t == scene::Target::Tool(scene::Tool::Bold)).map(|(r, _)| *r);
        let list = display::record(&dw, &s, &LIGHT, 1.0).unwrap();
        (Surface::draw_new((width as u32, height as u32, 96), &list), panel, bold)
    };
    let ((over, panel, bold), (bare, _, _)) = (draw(1.0), draw(0.0));
    assert!((panel.width() - crate::chrome::SIDEBAR_WIDTH).abs() < 0.5, "revealed, its own width: {}", panel.width());
    let bold = bold.expect("the toolbar is docked");
    assert!(bold.left < panel.right, "the toolbar reaches under the menu here ({bold:?}, {panel:?})");
    // Inside it, clear of its rounded corners.
    let (l, t, r, b) = (panel.left as usize + 14, panel.top as usize + 14, panel.right as usize - 14, panel.bottom as usize - 14);
    let differ = (t..b).flat_map(|y| (l..r).map(move |x| (x, y))).filter(|&(x, y)| over.pixels()[y * width as usize + x] != bare.pixels()[y * width as usize + x]).count();
    let first: Vec<(usize, usize, u32, u32)> = (t..b).flat_map(|y| (l..r).map(move |x| (x, y))).filter(|&(x, y)| over.pixels()[y * width as usize + x] != bare.pixels()[y * width as usize + x]).take(5).map(|(x, y)| (x, y, over.pixels()[y * width as usize + x], bare.pixels()[y * width as usize + x])).collect();
    assert_eq!(differ, 0, "the toolbar shows through the menu in {differ} pixels: {first:x?}");
}

/// A pulled pill keeps its edge along the window straight and swells out
/// the other way, under the pointer.
#[test]
fn a_pulled_pill_swells_from_a_straight_edge() {
    use crate::scene::{Fill, Ink, Rect, WaveShape};
    let dw = factory();
    let pill = Rect::new(300.0, 5.0, 620.0, 11.0);
    let pulled = crate::wave::target(pill, false, Some((520.0, 40.0)));
    assert!(pulled[crate::wave::SAMPLES * 5 / 8] > 5.0, "pulled toward the pointer");
    let draw = |d: [f32; crate::wave::SAMPLES]| {
        let fill = Fill { wave: Some(WaveShape { vertical: false, flipped: false, d }), ..Fill::solid(pill, 3.0, Ink::Text) };
        let scene = Scene { fills: vec![fill], ..Scene::default() };
        let surface = Surface::draw_new((700, 60, 96), &display::record(&dw, &scene, &DARK, 1.0).unwrap());
        surface.pixels().to_vec()
    };
    let (still, drawn) = (draw([0.0; crate::wave::SAMPLES]), draw(pulled));
    let ink = |px: &[u32], x: usize, y: usize| px[y * 700 + x] != DARK.background;
    // Above the pill: nothing, pulled or not; its top row: the same.
    for x in 290..630 {
        for y in 0..5 {
            assert!(!ink(&drawn, x, y), "pulled above its edge at ({x}, {y})");
        }
        assert_eq!(drawn[5 * 700 + x], still[5 * 700 + x], "its straight edge moved at x {x}");
    }
    // Below: deeper under the pointer than away from it.
    let depth = |px: &[u32], x: usize| (0..60).rev().find(|&y| ink(px, x, y)).unwrap_or(0);
    assert!(depth(&drawn, 520) >= depth(&still, 520) + 6, "swells under the pointer: {} vs {}", depth(&drawn, 520), depth(&still, 520));
    assert!(depth(&drawn, 320) <= depth(&still, 320) + 1, "and not far from it");
}

/// The grid's pill at the window's foot, flipped: its edge along the foot
/// stays straight and it swells up, toward the note, under the pointer.
#[test]
fn a_pulled_pill_at_the_foot_swells_up_from_a_straight_edge() {
    use crate::scene::{Fill, Ink, Rect, WaveShape};
    let dw = factory();
    let pill = Rect::new(300.0, 49.0, 620.0, 55.0);
    let pulled = crate::wave::target(pill, false, Some((520.0, 20.0)));
    let draw = |d: [f32; crate::wave::SAMPLES]| {
        let fill = Fill { wave: Some(WaveShape { vertical: false, flipped: true, d }), ..Fill::solid(pill, 3.0, Ink::Text) };
        let scene = Scene { fills: vec![fill], ..Scene::default() };
        let surface = Surface::draw_new((700, 60, 96), &display::record(&dw, &scene, &DARK, 1.0).unwrap());
        surface.pixels().to_vec()
    };
    let (still, drawn) = (draw([0.0; crate::wave::SAMPLES]), draw(pulled));
    let ink = |px: &[u32], x: usize, y: usize| px[y * 700 + x] != DARK.background;
    // Below the pill: nothing, pulled or not; its bottom row: the same.
    for x in 290..630 {
        for y in 55..60 {
            assert!(!ink(&drawn, x, y), "pulled below its edge at ({x}, {y})");
        }
        assert_eq!(drawn[54 * 700 + x], still[54 * 700 + x], "its straight edge moved at x {x}");
    }
    // Above: higher under the pointer than away from it.
    let reach = |px: &[u32], x: usize| (0..60).find(|&y| ink(px, x, y)).unwrap_or(60);
    assert!(reach(&drawn, 520) + 6 <= reach(&still, 520), "swells up under the pointer: {} vs {}", reach(&drawn, 520), reach(&still, 520));
    assert!(reach(&drawn, 320) + 1 >= reach(&still, 320), "and not far from it");
}

/// Typing at the end of a paragraph that wraps repaints the line typed in,
/// not the paragraph, and exactly.
#[test]
fn typing_repaints_the_line_not_the_paragraph() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let before = format!("# Plan\n{}\nAnd a line after it.\n", "A paragraph long enough to wrap across the column when it is laid out at this width. ".repeat(4).trim_end());
    let at = before.find("\nAnd").unwrap();
    let after = format!("{}s{}", &before[..at], &before[at..]);
    let list = |text: &str, caret: usize| {
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        let input = scene::Input {
            vault_name: "notes",
            active_note: "Plan",
            text,
            selection: (caret, caret),
            lines: &lines,
            options: [0; scene::SETTINGS],
            page_fade: 1.0,
            sidebar_dock: 1.0,
            sidebar_show: 1.0,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            ..Default::default()
        };
        let mut s = scene::build(&dw, &fonts, &input, 1268.0, 754.0).unwrap();
        s.place_caret((0.0, 0.0));
        display::record(&dw, &s, &DARK, 3.0).unwrap()
    };
    let view = (3804, 2262, 288);
    let mut surface = Surface::draw_new(view, &list(&before, at));
    let typed = list(&after, at + 1);
    surface.draw(view, &typed);
    let rects = surface.last_damage.clone().expect("a keystroke paints part of the window");
    // One line of body text at 3x is about 80 pixels.
    let tallest = rects.iter().map(|r| r.3 - r.1).max().unwrap_or(0);
    assert!(tallest < 120, "typing repaints more than its line: {rects:?}");
    let full = Surface::draw_new(view, &typed);
    let differ = surface.pixels().iter().zip(full.pixels()).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 0, "partial repaint differs from a full one in {differ} pixels");
}

/// Frames drawn with everything kept between them (layouts, recordings,
/// pixels) match frames drawn from nothing, through typing, the caret
/// going in and out of marks, a theme switch and scrolling by fractions of
/// a pixel.
#[test]
fn kept_frames_match_fresh_ones() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let rows: Vec<SidebarRow> = (0..8).map(|i| SidebarRow { depth: 0, kind: RowKind::Note, name: format!("Note {i}") }).collect();
    let mut text = String::from("# Plan\nShip the **beta** on the twelfth, see [[Press kit]] and `code`.\n- [ ] Tell the team\n## Notes\nA paragraph long enough to wrap across the column when it is laid out at this width, twice over.\n");
    let view = (1902, 1131, 144);
    let mut surface = Surface::draw_new((16, 16, 96), &display::record(&dw, &Scene::default(), &DARK, 1.0).unwrap());
    // (what happens, caret, theme, scroll)
    let bold = text.find("beta").unwrap();
    let steps: Vec<(&str, usize, bool, f32)> = vec![
        ("start", 3, true, 0.0),
        ("into the bold", bold + 1, true, 0.0),
        ("type", bold + 1, true, 0.0),
        ("out of it", bold + 20, true, 0.0),
        ("onto the heading", 2, true, 0.0),
        ("light", 2, false, 0.0),
        ("scroll a little", 2, false, 0.37),
        ("scroll more", 40, false, 1.61),
        ("dark again", 40, true, 1.61),
    ];
    for (what, caret, dark, scroll) in steps {
        if what == "type" {
            text.insert(bold + 1, 'x');
        }
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(&text).collect();
        let make = || {
            let input = scene::Input {
                vault_name: "notes",
                active_note: "Plan",
                text: &text,
                sidebar: &rows,
                selection: (caret, caret),
                lines: &lines,
                scroll_px: scroll,
                options: [0; scene::SETTINGS],
                page_fade: 1.0,
                sidebar_dock: 1.0,
                sidebar_show: 1.0,
                toolbar_dock: 1.0,
                toolbar_show: 1.0,
                ..Default::default()
            };
            let mut s = scene::build(&dw, &fonts, &input, 1268.0, 754.0).unwrap();
            s.place_caret((0.0, 0.0));
            display::record(&dw, &s, if dark { &DARK } else { &LIGHT }, 1.5).unwrap()
        };
        surface.draw(view, &make());
        scene::forget_kept();
        display::forget_kept();
        let fresh = Surface::draw_new(view, &make());
        let differ = surface.pixels().iter().zip(fresh.pixels()).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "{what}: the kept frame differs from a fresh one in {differ} pixels");
    }
}

#[test]
#[ignore = "writes the installer's screen at each stage for manual inspection"]
fn dump_setup() {
    use kolada_ui::setup::{self, Folder, Stage, View};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let folders = [
        Folder { name: "A new folder", detail: r"Documents\Kolada" },
        Folder { name: "Obsidian vault", detail: "Documents/Work \u{00B7} 214 notes \u{00B7} opens as it is" },
        Folder { name: "Notes export", detail: "Documents/old.nbk \u{00B7} imported into a new Documents/Kolada" },
        Folder { name: "Another folder\u{2026}", detail: "Choose any folder" },
    ];
    let (w, h) = setup::SIZE;
    for (name, stage, options, updating) in [
        ("welcome", Stage::Welcome, 0.0, None),
        ("options", Stage::Welcome, 1.0, None),
        ("update", Stage::Welcome, 1.0, Some("0.0.9")),
        ("working", Stage::Working(0.45), 0.0, None),
        ("done", Stage::Done, 0.0, None),
        ("remove", Stage::Remove, 0.0, None),
        ("removed", Stage::Removed, 0.0, None),
    ] {
        let view = View { stage, updating, version: "0.1.0", options, folders: if updating.is_some() { &[] } else { &folders }, chosen: 0, choices: [true, true, true], message: Some(r"Your notes are in Documents\Kolada"), offer_default: true, hover: None };
        for (theme, t) in [("paper", &LIGHT), ("graphite", &kolada_ui::theme::DARK)] {
            let s = setup::build(&dw, &fonts, &view, w, h).unwrap();
            let list = display::record(&dw, &s, t, 1.0).unwrap();
            Surface::draw_new((w as u32, h as u32, 96), &list).dump(&dir.join(format!("setup-{name}-{theme}.bmp"))).unwrap();
        }
    }
}

#[test]
#[ignore = "writes Quick Send's popup in every state, graphite and paper, for manual inspection"]
fn dump_quick_send() {
    use kolada_ui::quick::{self as q, ClipView, DeviceRow, Kind, Pairing, PicView, PopupView, Screen, Staged};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let c = |kind, text, title, from, goes| ClipView { sent: q::Sent::None, from, goes, kind, text, title, copied: false, spent: false, mine: false, picture: None, more: &[], names: &[], caught: "", glow: 0.0, open: 0.0 };
    let clips = [
        c(Kind::Text, "Gate B, row 14. Boarding at 19:05.", "", "From iPhone \u{00B7} 2 min ago", ""),
        c(Kind::Secret, "x", "", "From iPhone \u{00B7} 1 hour ago", "Goes once copied"),
        c(Kind::Link, "https://blog.rust-lang.org/", "Rust release notes", "You beamed \u{00B7} 3 hours ago", ""),
        c(Kind::Text, "Door code for the studio: 4931", "", "From Laptop \u{00B7} Yesterday", ""),
    ];
    let to = ["All devices", "iPhone", "Laptop"];
    let devices = [DeviceRow { name: "Desk PC", look: kolada_ui::quick::Look::Laptop, this: true }, DeviceRow { name: "iPhone", look: kolada_ui::quick::Look::Phone, this: false }];
    // A laptop at 200% leaves the popup about 330 DIPs: drawn at scale 2.
    let long = format!("{}\n\n{}", "\u{0646}\u{0635} \u{062A}\u{062C}\u{0631}\u{064A}\u{0628}\u{064A} ".repeat(30), "\u{0633}\u{0637}\u{0631} ".repeat(40));
    let long_clips = [ClipView { text: &long, ..clips[0].clone() }, clips[2].clone()];
    // A made-up picture: a sky, a hill and a sun, as a thumbnail's pixels.
    let (pw, ph) = (360u32, 240u32);
    let data = (0..pw * ph)
        .map(|i| {
            let (x, y) = ((i % pw) as f32, (i / pw) as f32);
            let hill = y > 150.0 + 30.0 * (x / 60.0).sin();
            let sun = (x - 270.0).powi(2) + (y - 60.0).powi(2) < 900.0;
            let (r, g, b) = if sun { (250, 220, 150) } else if hill { (40, 110 + (y as u32 % 40), 70) } else { (120 + (y as u32 / 3), 170, 220) };
            0xFF00_0000 | (r << 16) | (g << 8) | b
        })
        .collect();
    let pixels = std::sync::Arc::new(kolada_ui::pixels::Pixels { width: pw, height: ph, data });
    let pic = |ready| PicView { pixels: Some(pixels.clone()), width: 1920, height: 1280, size: "1.2 MB", ready };
    let picture_clips = [
        ClipView { kind: Kind::Picture, text: "Screenshot 2026-10-27.png", picture: Some(pic(true)), ..c(Kind::Picture, "", "", "From iPhone \u{00B7} 2 min ago", "") },
        ClipView { kind: Kind::Picture, text: "Whiteboard.jpg", picture: Some(PicView { pixels: None, ..pic(false) }), ..c(Kind::Picture, "", "", "From Laptop \u{00B7} just now", "") },
        clips[0].clone(),
    ];
    let states: [(&str, Screen, f32); 17] = [
        ("typing", Screen::Main { find: q::Find::default(), staged: None, to: &to, chosen: 0, clips: &clips[..2], scroll: 0.0, saved: None, draft: "Gate B, row 14\nBoarding at 19:05", selection: None, more: None }, q::H),
        ("picture-ready", Screen::Main { find: q::Find::default(), staged: Some(Staged { what: "a picture", kind: Kind::Picture, text: "Picture.png", picture: Some(pic(true)), more: &[], unsure: false }), to: &to, chosen: 0, clips: &clips[..1], scroll: 0.0, saved: None, draft: "", selection: None, more: None }, q::H),
        ("pictures", Screen::Main { find: q::Find::default(), staged: None, to: &to, chosen: 0, clips: &picture_clips, scroll: 0.0, saved: None, draft: "", selection: None, more: None }, q::H),
        ("long", Screen::Main { find: q::Find::default(), staged: Some(Staged { what: "text", kind: Kind::Text, text: &long, picture: None, more: &[], unsure: false }), to: &to, chosen: 0, clips: &long_clips, scroll: 0.0, saved: None, draft: "", selection: None, more: None }, q::H),
        ("first-use", Screen::FirstUse, q::H),
        ("empty", Screen::Main { find: q::Find::default(), staged: None, to: &to, chosen: 0, clips: &[], scroll: 0.0, saved: None, draft: "", selection: None, more: None }, q::H),
        ("ready", Screen::Main { find: q::Find::default(), staged: Some(Staged { what: "a link", kind: Kind::Link, text: "https://blog.rust-lang.org/platform/pricing/", picture: None, more: &[], unsure: false }), to: &to, chosen: 0, clips: &clips[..1], scroll: 0.0, saved: None, draft: "", selection: None, more: None }, q::H),
        ("maybe-password", Screen::Main { find: q::Find::default(), staged: Some(Staged { what: "a password", kind: Kind::Secret, text: "x7kp2qm9vld8wz", picture: None, more: &[], unsure: true }), to: &to, chosen: 0, clips: &clips[..1], scroll: 0.0, saved: None, draft: "", selection: None, more: None }, q::H),
        ("clips", Screen::Main { find: q::Find::default(), staged: None, to: &to, chosen: 0, clips: &clips, scroll: 0.0, saved: None, draft: "", selection: None, more: None }, q::H),
        ("saved", Screen::Main { find: q::Find::default(), staged: None, to: &to, chosen: 0, clips: &clips[1..], scroll: 0.0, saved: Some("Gate B, row 14"), draft: "", selection: None, more: None }, q::H),
        ("devices-code", Screen::Devices { devices: &devices, pairing: Pairing::Showing { code: "F4FS-2RSA", left: "4:12 left" }, facts: &[], naming: None }, q::H),
        ("devices-asks", Screen::Devices { devices: &devices, pairing: Pairing::Asks { name: "Laptop" }, facts: &[], naming: None }, q::H),
        ("beamed", Screen::Beamed { to: "iPhone and Laptop", what: "Rust release notes" }, q::BEAMED_H),
        ("short-first-use", Screen::FirstUse, 330.0),
        ("short-ready", Screen::Main { find: q::Find::default(), staged: Some(Staged { what: "a link", kind: Kind::Link, text: "https://blog.rust-lang.org/platform/pricing/", picture: None, more: &[], unsure: false }), to: &to, chosen: 0, clips: &clips, scroll: 0.0, saved: None, draft: "", selection: None, more: None }, 330.0),
        ("short-devices-code", Screen::Devices { devices: &devices, pairing: Pairing::Showing { code: "F4FS-2RSA", left: "4:12 left" }, facts: &[], naming: None }, 330.0),
        ("short-devices-asks", Screen::Devices { devices: &devices, pairing: Pairing::Asks { name: "Laptop" }, facts: &[], naming: None }, 330.0),
    ];
    for (name, screen, h) in states {
        let view = PopupView { screen, devices: 2, said: None, host: q::Host::WINDOWS };
        let scene = q::popup(&dw, &fonts, &view, None, q::W, h).unwrap();
        let scale = if name.starts_with("short") { 2.0 } else { 1.5 };
        for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
            let list = display::record(&dw, &scene, theme, scale).unwrap();
            let px = ((q::W * scale) as u32, (h * scale) as u32, (96.0 * scale) as u32);
            Surface::draw_new(px, &list).dump(&dir.join(format!("quick-{name}-{theme_name}.bmp"))).unwrap();
        }
    }
}

/// Beam for review, as a designer sees it: the popup empty, while typing,
/// with a mixed timeline (grouped texts, a long one opened, one cut past
/// its limit with Read more, a picture, a password; this device's own on
/// the right), with the recipient chip, and with a card's menu open; the
/// window's button at rest, with a count, under the pointer and with its
/// menu; the side menu's row; Settings > Devices. In English and with
/// Arabic content, graphite and paper, at 100% and 150%:
/// `review-<view>-<lang>-<theme>-<scale>.bmp`.
#[test]
#[ignore = "writes Beam's screens for a design review"]
fn dump_beam_review() {
    use kolada_ui::quick::{self as q, ClipView, DeviceRow, Kind, Pairing, PicView, PopupView, Screen, Staged};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let (pw, ph) = (360u32, 240u32);
    let data = (0..pw * ph)
        .map(|i| {
            let (x, y) = ((i % pw) as f32, (i / pw) as f32);
            let hill = y > 150.0 + 30.0 * (x / 60.0).sin();
            let sun = (x - 270.0).powi(2) + (y - 60.0).powi(2) < 900.0;
            let (r, g, b) = if sun { (250, 220, 150) } else if hill { (40, 110 + (y as u32 % 40), 70) } else { (120 + (y as u32 / 3), 170, 220) };
            0xFF00_0000 | (r << 16) | (g << 8) | b
        })
        .collect();
    let pixels = std::sync::Arc::new(kolada_ui::pixels::Pixels { width: pw, height: ph, data });
    let pic = PicView { pixels: Some(pixels), width: 1920, height: 1280, size: "1.2 MB", ready: true };
    let devices = [DeviceRow { name: "Desk PC", look: kolada_ui::quick::Look::Laptop, this: true }, DeviceRow { name: "iPhone", look: kolada_ui::quick::Look::Phone, this: false }, DeviceRow { name: "Laptop", look: kolada_ui::quick::Look::Laptop, this: false }];
    const TO: [&str; 3] = ["All devices", "iPhone", "Laptop"];
    let to: &'static [&'static str] = &TO;
    struct Lang {
        name: &'static str,
        text: &'static str,
        more: &'static str,
        sent: &'static str,
        list: String,
        long: String,
        title: &'static str,
        note: &'static str,
        line: &'static str,
        typed: &'static str,
    }
    let en_list = (1..=7).map(|n| format!("{n}. Pack the charger, the passport and the blue folder")).collect::<Vec<_>>().join("\n");
    let en_long = (1..=40).map(|n| format!("Line {n} of the agenda for Thursday, with the budget and the release.")).collect::<Vec<_>>().join("\n");
    let ar_line = "\u{0627}\u{0644}\u{0628}\u{0646}\u{062F} \u{0627}\u{0644}\u{062A}\u{0627}\u{0644}\u{064A} \u{0645}\u{0646} \u{062C}\u{062F}\u{0648}\u{0644} \u{0627}\u{0644}\u{062E}\u{0645}\u{064A}\u{0633}";
    let ar_list = (1..=7).map(|n| format!("{n}. {ar_line}")).collect::<Vec<_>>().join("\n");
    // "Line n of Thursday's schedule", as the English: a long text, not a list.
    let ar_long = (1..=40).map(|n| format!("\u{0627}\u{0644}\u{0633}\u{0637}\u{0631} {n} \u{0645}\u{0646} \u{062C}\u{062F}\u{0648}\u{0644} \u{0627}\u{0644}\u{062E}\u{0645}\u{064A}\u{0633}")).collect::<Vec<_>>().join("\n");
    let langs = [
        Lang { name: "en", text: "Gate B, row 14. Boarding at 19:05.", more: "Taxi at 17:30 from the hotel's front door.", sent: "Pick up the keys from reception before six.", list: en_list, long: en_long, title: "Trip plans", note: "# Trip plans\nFlights, the hotel and what to pack.\n", line: "Gate B, row 14. Boarding at 19:05.", typed: "Gate B, row 14\nBoarding at 19:05" },
        Lang {
            name: "ar",
            text: "\u{0627}\u{0644}\u{0628}\u{0648}\u{0627}\u{0628}\u{0629} \u{0628}\u{060C} \u{0627}\u{0644}\u{0635}\u{0641} \u{0661}\u{0664}. \u{0627}\u{0644}\u{0635}\u{0639}\u{0648}\u{062F} \u{0641}\u{064A} \u{0667}:\u{0660}\u{0665} \u{0645}\u{0633}\u{0627}\u{0621}\u{064B}.",
            more: "\u{0627}\u{0644}\u{062A}\u{0627}\u{0643}\u{0633}\u{064A} \u{0641}\u{064A} \u{0665}:\u{0663}\u{0660} \u{0645}\u{0646} \u{0628}\u{0627}\u{0628} \u{0627}\u{0644}\u{0641}\u{0646}\u{062F}\u{0642}.",
            sent: "\u{062E}\u{0630} \u{0627}\u{0644}\u{0645}\u{0641}\u{0627}\u{062A}\u{064A}\u{062D} \u{0645}\u{0646} \u{0627}\u{0644}\u{0627}\u{0633}\u{062A}\u{0642}\u{0628}\u{0627}\u{0644} \u{0642}\u{0628}\u{0644} \u{0627}\u{0644}\u{0633}\u{0627}\u{062F}\u{0633}\u{0629}.",
            list: ar_list,
            long: ar_long,
            title: "\u{062E}\u{0637}\u{0629} \u{0627}\u{0644}\u{0631}\u{062D}\u{0644}\u{0629}",
            note: "# \u{062E}\u{0637}\u{0629} \u{0627}\u{0644}\u{0631}\u{062D}\u{0644}\u{0629}\n\u{0627}\u{0644}\u{0631}\u{062D}\u{0644}\u{0627}\u{062A} \u{0648}\u{0627}\u{0644}\u{0641}\u{0646}\u{062F}\u{0642} \u{0648}\u{0645}\u{0627} \u{0646}\u{062D}\u{0632}\u{0645}\u{0647}.\n",
            line: "\u{0627}\u{0644}\u{0628}\u{0648}\u{0627}\u{0628}\u{0629} \u{0628}\u{060C} \u{0627}\u{0644}\u{0635}\u{0641} \u{0661}\u{0664}.",
            typed: "\u{0627}\u{0644}\u{0628}\u{0648}\u{0627}\u{0628}\u{0629} \u{0628}\u{060C} \u{0627}\u{0644}\u{0635}\u{0641} \u{0661}\u{0664}",
        },
    ];
    let scales = [1.0f32, 1.5];
    let themes: [(&str, &Theme); 2] = [("graphite", &DARK), ("paper", &LIGHT)];
    let write = |name: &str, scene: &Scene, (w, h): (f32, f32)| {
        for &scale in &scales {
            for (theme_name, theme) in themes {
                let list = display::record(&dw, scene, theme, scale).unwrap();
                let px = ((w * scale).round() as u32, (h * scale).round() as u32, (96.0 * scale) as u32);
                Surface::draw_new(px, &list).dump(&dir.join(format!("review-{name}-{theme_name}-{}.bmp", (scale * 100.0) as u32))).unwrap();
            }
        }
    };
    for lang in &langs {
        let c = |kind, text, from, mine| ClipView { sent: q::Sent::None, from, goes: "", kind, text, title: "", copied: false, spent: false, mine, picture: None, more: &[], names: &[], caught: "", glow: 0.0, open: 0.0 };
        // Newest first.
        let timeline = [
            c(Kind::Text, lang.more, "", false),
            c(Kind::Text, lang.text, "iPhone \u{00B7} 2 min ago", false),
            ClipView { open: 1.0, ..c(Kind::Text, &lang.list, "You \u{00B7} 9 min ago", true) },
            c(Kind::Text, lang.sent, "", true),
            ClipView { open: 1.0, ..c(Kind::Text, &lang.long, "Laptop \u{00B7} 40 min ago", false) },
            ClipView { picture: Some(pic.clone()), ..c(Kind::Picture, "Screenshot 2026-10-27.png", "Laptop \u{00B7} 1 hour ago", false) },
            ClipView { goes: "Goes once copied", ..c(Kind::Secret, "x", "iPhone \u{00B7} 2 hours ago", false) },
            ClipView { title: "Rust release notes", ..c(Kind::Link, "https://blog.rust-lang.org/", "You \u{00B7} 3 hours ago", true) },
        ];
        let main = |clips, draft, more, to: &'static [&'static str], staged| Screen::Main { find: q::Find::default(), staged, to, chosen: 0, clips, scroll: 0.0, saved: None, draft, selection: None, more };
        let screens: [(&str, Screen, Option<scene::Target>); 7] = [
            ("popup-empty", main(&[], "", None, &to[..2], None), None),
            ("popup-typing", main(&timeline[..2], lang.typed, None, &to[..2], None), None),
            ("popup-timeline", main(&timeline[..5], "", None, &to[..2], None), Some(scene::Target::Quick(q::Hit::Card(1)))),
            ("popup-older", main(&timeline, "", None, &to[..2], None), None),
            ("popup-chip", main(&timeline[..3], lang.typed, None, to, None), None),
            ("popup-menu", main(&timeline[..3], "", Some(1), &to[..2], None), None),
            ("popup-ready", main(&timeline[..2], "", None, &to[..2], Some(Staged { what: "a picture", kind: Kind::Picture, text: "Picture.png", picture: Some(pic.clone()), more: &[], unsure: false })), None),
        ];
        for (name, screen, hover) in screens {
            let scroll_older = name == "popup-older";
            let screen = match screen {
                Screen::Main { find, staged, to, chosen, clips, saved, draft, selection, more, .. } if scroll_older => Screen::Main { find, staged, to, chosen, clips, scroll: 10_000.0, saved, draft, selection, more },
                s => s,
            };
            let view = PopupView { screen, devices: 2, said: None, host: q::Host { keys: "Alt+Z", ..q::Host::WINDOWS } };
            let scene = q::popup(&dw, &fonts, &view, hover, q::W, q::H).unwrap();
            write(&format!("{name}-{}", lang.name), &scene, (q::W, q::H));
        }
        // A beam just sent from here, at the foot: pending, its check, failed.
        for (name, sent) in [("popup-sent-pending", q::Sent::Pending), ("popup-sent-check", q::Sent::Check { scale: 1.0, alpha: 1.0 }), ("popup-sent-failed", q::Sent::Failed)] {
            let mut tl = vec![ClipView { sent, ..c(Kind::Text, lang.sent, "You \u{00B7} now", true) }];
            tl.extend(timeline[..2].iter().cloned());
            let screen = Screen::Main { find: q::Find::default(), staged: None, to: &to[..2], chosen: 0, clips: &tl, scroll: 0.0, saved: None, draft: "", selection: None, more: None };
            let view = PopupView { screen, devices: 2, said: None, host: q::Host { keys: "Alt+Z", ..q::Host::WINDOWS } };
            write(&format!("{name}-{}", lang.name), &q::popup(&dw, &fonts, &view, None, q::W, q::H).unwrap(), (q::W, q::H));
        }
        if lang.name == "en" {
            let view = PopupView { screen: Screen::FirstUse, devices: 0, said: None, host: q::Host::WINDOWS };
            write("popup-first-use-en", &q::popup(&dw, &fonts, &view, None, q::W, q::H).unwrap(), (q::W, q::H));
            // Pairing in the popup: the same Devices as Settings'.
            let alone = [q::DeviceRow { name: "Desk PC", look: kolada_ui::quick::Look::Laptop, this: true }];
            let facts = ["1 note is too big to sync".to_string()];
            for (name, list, pairing, facts) in [
                ("popup-devices-unpaired", &alone[..], Pairing::Idle, &[][..]),
                ("popup-devices-code", &alone[..], Pairing::Showing { code: "F4FS-2RSA", left: "4:12 left" }, &[][..]),
                ("popup-devices-list", &devices[..], Pairing::Idle, &facts[..]),
            ] {
                let view = PopupView { screen: Screen::Devices { devices: list, pairing, facts, naming: None }, devices: list.len() - 1, said: None, host: q::Host::WINDOWS };
                write(&format!("{name}-en"), &q::popup(&dw, &fonts, &view, None, q::W, q::H).unwrap(), (q::W, q::H));
            }
        }
        // The window: the side menu with Beam's row, the note, the button.
        let rows = vec![
            SidebarRow { depth: 0, kind: RowKind::Folder { expanded: true }, name: "Notes".into() },
            SidebarRow { depth: 1, kind: RowKind::Note, name: lang.title.into() },
            SidebarRow { depth: 0, kind: RowKind::Note, name: "Reading list".into() },
        ];
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(lang.note).collect();
        let (ww, wh) = (1100.0f32, 720.0f32);
        let window = |beam: scene::BeamView, hover: Option<scene::Target>, devices: Option<scene::DevicesView>, options: bool| {
            let mut input = scene::Input {
                vault_name: "my notes",
                active_note: lang.title,
                text: lang.note,
                sidebar: &rows,
                lines: &lines,
                options: [0; scene::SETTINGS],
                page_fade: 1.0,
                sidebar_dock: 1.0,
                sidebar_show: 1.0,
                toolbar_dock: 1.0,
                toolbar_show: 1.0,
                quick: Some(beam),
                devices,
                options_open: options,
                hover,
                ..Default::default()
            };
            if options {
                input.options_scroll = kolada_ui::chrome::devices_scroll(&input, ww);
            }
            scene::build(&dw, &fonts, &input, ww, wh).unwrap()
        };
        let rest = scene::BeamView { unseen: 0, line: "iPhone \u{00B7} 2 min ago", typing: false, menu: None, check: None, keys: "Alt+Z" };
        write(&format!("window-rest-{}", lang.name), &window(rest, None, None, false), (ww, wh));
        write(&format!("window-count-{}", lang.name), &window(scene::BeamView { unseen: 2, line: lang.line, ..rest }, None, None, false), (ww, wh));
        write(&format!("window-hover-{}", lang.name), &window(rest, Some(scene::Target::Quick(q::Hit::Fab)), None, false), (ww, wh));
        write(&format!("window-menu-{}", lang.name), &window(scene::BeamView { menu: Some(lang.title), ..rest }, None, None, false), (ww, wh));
        write(&format!("window-check-{}", lang.name), &window(scene::BeamView { check: Some((1.0, 1.0)), ..rest }, None, None, false), (ww, wh));
        if lang.name == "en" {
            for (name, pairing) in [("idle", Pairing::Idle), ("code", Pairing::Showing { code: "F4FS-2RSA", left: "4:12 left" }), ("asks", Pairing::Asks { name: "Laptop" }), ("entering", Pairing::Entering { typed: "F4FS-2R" })] {
                let v = scene::DevicesView { devices: &devices, pairing, said: None, facts: &[], this: "PC", naming: None };
                write(&format!("settings-devices-{name}"), &window(rest, None, Some(v), true), (ww, wh));
            }
        }
    }
}


/// Settings > Devices as the user's report showed it, as 0.4.4 draws it:
/// this laptop by its friendly name, a phone whose name didn't read by its
/// kind, pairing's buttons; and this one being renamed. Graphite and paper,
/// at 100% and 150%: `devices-<state>-<theme>-<scale>.bmp`, the window whole.
#[test]
#[ignore = "writes Settings > Devices for a design review"]
fn dump_settings_devices() {
    use kolada_ui::quick::{DeviceRow, Look, Pairing};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let devices = [DeviceRow { name: "Windows laptop", look: Look::Laptop, this: true }, DeviceRow { name: "iPhone", look: Look::Phone, this: false }];
    let (ww, wh) = (1100.0f32, 720.0f32);
    for (state, naming) in [("list", None), ("rename", Some("Kitchen laptop"))] {
        let mut input = scene::Input {
            vault_name: "my notes",
            active_note: "Trip plans",
            text: "# Trip plans
",
            options: [0; scene::SETTINGS],
            page_fade: 1.0,
            sidebar_dock: 1.0,
            sidebar_show: 1.0,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            devices: Some(scene::DevicesView { devices: &devices, pairing: Pairing::Idle, said: None, facts: &[], this: "PC", naming }),
            options_open: true,
            ..Default::default()
        };
        input.options_scroll = kolada_ui::chrome::devices_scroll(&input, ww);
        let s = scene::build(&dw, &fonts, &input, ww, wh).unwrap();
        for scale in [1.0f32, 1.5] {
            for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
                let list = display::record(&dw, &s, theme, scale).unwrap();
                let px = ((ww * scale).round() as u32, (wh * scale).round() as u32, (96.0 * scale) as u32);
                Surface::draw_new(px, &list).dump(&dir.join(format!("devices-{state}-{theme_name}-{}.bmp", (scale * 100.0) as u32))).unwrap();
            }
        }
    }
}

/// Settings' choices for a design review: the font (Sans, Serif, Mono and
/// the machine's own), text size, line width and the features' switches.
/// At rest with everything on; the finder of the machine's fonts open; and
/// a font of the machine's chosen with the writing model, Beam and motion
/// off. Graphite and paper at 150%, and the narrowest column at 100%:
/// `choices-<state>-<theme>-<scale>.bmp`.
#[test]
#[ignore = "writes Settings' choices for a design review"]
fn dump_settings_choices() {
    use scene::{Setting, Target};
    let dw = factory();
    let d = std::path::PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts");
    dw.borrow_mut().load_files(&["calibri.ttf", "cambria.ttc", "Candara.ttf", "constan.ttf", "corbel.ttf", "CascadiaCode.ttf", "pala.ttf"].map(|f| d.join(f)));
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let set = crate::fonts::font_set(0);
    let faces = ["Segoe UI", if d.join("SitkaVF.ttf").is_file() { "Sitka" } else { "Georgia" }, if d.join("CascadiaMono.ttf").is_file() { "Cascadia Mono" } else { "Consolas" }];
    assert_eq!(set.ui, "Segoe UI");
    let found = ["Calibri", "Cambria", "Candara", "Cascadia Code", "Constantia", "Corbel"];
    let devices = [kolada_ui::quick::DeviceRow { name: "Windows laptop", look: kolada_ui::quick::Look::Laptop, this: true }, kolada_ui::quick::DeviceRow { name: "iPhone", look: kolada_ui::quick::Look::Phone, this: false }];
    let mut off = [0; scene::SETTINGS];
    off[Setting::Font.index()] = scene::INSTALLED_FONT;
    off[Setting::Assist.index()] = kolada_ui::modelcard::OFF;
    off[Setting::Beam.index()] = 1;
    off[Setting::Motion.index()] = 1;
    let states: [(&str, [u8; scene::SETTINGS], Option<scene::FontFinder>); 3] = [
        ("rest", [0; scene::SETTINGS], None),
        ("finder", [0; scene::SETTINGS], Some(scene::FontFinder { query: "C", caret: 1, names: &found, picked: Some(1), total: found.len() })),
        ("off", off, None),
    ];
    // Sync and Beam turning off, and the writing model on: frames of the
    // fold's spring (`fold-<what>-<ms>-graphite.bmp`).
    let mut folds: Vec<(String, [u8; scene::SETTINGS], Option<(Setting, f32)>)> = Vec::new();
    for ms in [0u32, 40, 80, 120, 180, 260, 360, 700] {
        let mut o = [0; scene::SETTINGS];
        o[Setting::Beam.index()] = 1;
        let beam = kolada_ui::chrome::folding(ms as f32 / 1000.0, false);
        folds.push((format!("fold-beam-{ms:03}"), o, beam.map(|t| (Setting::Beam, t))));
        let model = kolada_ui::chrome::folding(ms as f32 / 1000.0, true);
        folds.push((format!("fold-model-{ms:03}"), [0; scene::SETTINGS], model.map(|t| (Setting::Assist, t))));
    }
    for (name, options, folding) in &folds {
        let input = scene::Input {
            vault_name: "my notes",
            active_note: "Trip plans",
            text: "# Trip plans\n",
            options: *options,
            font_faces: faces,
            folding: *folding,
            vault_path: r"C:\Users\me\Documents\my notes",
            page_fade: 1.0,
            sidebar_dock: 1.0,
            sidebar_show: 1.0,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            devices: Some(scene::DevicesView { devices: &devices, pairing: kolada_ui::quick::Pairing::Idle, said: None, facts: &[], this: "PC", naming: None }),
            options_open: true,
            ..Default::default()
        };
        let s = scene::build(&dw, &fonts, &input, 1180.0, 860.0).unwrap();
        let list = display::record(&dw, &s, &DARK, 1.0).unwrap();
        Surface::draw_new((1180, 860, 96), &list).dump(&dir.join(format!("{name}-graphite.bmp"))).unwrap();
    }
    for (state, options, finder) in states {
        for (w, h, scales) in [(1180.0f32, 860.0f32, &[1.5f32][..]), (480.0, 860.0, &[1.0][..])] {
            let input = scene::Input {
                vault_name: "my notes",
                active_note: "Trip plans",
                text: "# Trip plans\n",
                options,
                font_faces: faces,
                font_name: (state == "off").then_some("Palatino Linotype"),
                font_finder: finder,
                page_fade: 1.0,
                sidebar_dock: 1.0,
                sidebar_show: 1.0,
                toolbar_dock: 1.0,
                toolbar_show: 1.0,
                vault_path: r"C:\Users\me\Documents\my notes",
                // Beam off: nothing of it runs, so no devices either.
                devices: (state != "off").then_some(scene::DevicesView { devices: &devices, pairing: kolada_ui::quick::Pairing::Idle, said: None, facts: &[], this: "PC", naming: None }),
                options_open: true,
                hover: (state == "rest").then_some(Target::Switch(Setting::Motion)),
                ..Default::default()
            };
            let s = scene::build(&dw, &fonts, &input, w, h).unwrap();
            for &scale in scales {
                for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
                    let list = display::record(&dw, &s, theme, scale).unwrap();
                    let px = ((w * scale).round() as u32, (h * scale).round() as u32, (96.0 * scale) as u32);
                    let name = if w < 600.0 { format!("choices-{state}-narrow-{theme_name}.bmp") } else { format!("choices-{state}-{theme_name}-{}.bmp", (scale * 100.0) as u32) };
                    Surface::draw_new(px, &list).dump(&dir.join(name)).unwrap();
                }
            }
        }
    }
}

/// The welcome screen a first run opens on, for a design review: at rest,
/// bringing notes in, brought in, and with both switches off; wide and at
/// a side-by-side snap, in graphite and paper:
/// `welcome-<state>-<size>-<theme>.bmp`.
#[test]
#[ignore = "writes the welcome screen for a design review"]
fn dump_welcome() {
    use kolada_ui::welcome::{From, Outcome, View, Welcome};
    use scene::Setting;
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let devices = [kolada_ui::quick::DeviceRow { name: "Windows laptop", look: kolada_ui::quick::Look::Laptop, this: true }];
    let mut off = [0; scene::SETTINGS];
    off[Setting::Assist.index()] = kolada_ui::modelcard::OFF;
    off[Setting::Beam.index()] = 1;
    let rest = Welcome::new();
    let mut busy = Welcome::new();
    busy.began(From::Evernote, "Evernote");
    busy.progress((214, 530));
    let mut done = busy.clone();
    done.finished(Outcome::Brought { count: 530, folder: "From Evernote".into(), failed: 0 });
    let states = [("rest", &rest, [0; scene::SETTINGS]), ("bringing", &busy, [0; scene::SETTINGS]), ("done", &done, [0; scene::SETTINGS]), ("off", &rest, off)];
    for (state, welcome, options) in states {
        for (size, w, h) in [("wide", 1280.0f32, 900.0f32), ("snap", 640.0, 1000.0)] {
            let input = scene::Input {
                vault_name: "Kolada",
                options,
                page_fade: 1.0,
                devices: Some(scene::DevicesView { devices: &devices, pairing: kolada_ui::quick::Pairing::Idle, said: None, facts: &[], this: "PC", naming: None }),
                welcome: Some(View { welcome, model: Some(0) }),
                ..Default::default()
            };
            let s = scene::build(&dw, &fonts, &input, w, h).unwrap();
            for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
                let list = display::record(&dw, &s, theme, 1.0).unwrap();
                Surface::draw_new((w as u32, h as u32, 96), &list).dump(&dir.join(format!("welcome-{state}-{size}-{theme_name}.bmp"))).unwrap();
            }
        }
    }
}

/// Kolada's surfaces side by side, for a review of their tones and lift:
/// Beam's popup empty, with a picture ready, with a few beams and with one
/// picture beamed; Settings > Devices; the note's menu; the window with
/// its side menu. Graphite and paper, at 100% and 150%:
/// `surfaces-<view>-<theme>-<scale>.bmp`.
#[test]
#[ignore = "writes Kolada's surfaces for a design review"]
fn dump_surfaces() {
    use crate::scene::MenuItem;
    use kolada_ui::quick::{self as q, ClipView, DeviceRow, Kind, Look, Pairing, PicView, PopupView, Screen, Staged};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let write = |name: &str, scene: &Scene, (w, h): (f32, f32)| {
        for scale in [1.0f32, 1.5] {
            for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
                let list = display::record(&dw, scene, theme, scale).unwrap();
                let px = ((w * scale).round() as u32, (h * scale).round() as u32, (96.0 * scale) as u32);
                Surface::draw_new(px, &list).dump(&dir.join(format!("surfaces-{name}-{theme_name}-{}.bmp", (scale * 100.0) as u32))).unwrap();
            }
        }
    };
    let (pw, ph) = (360u32, 240u32);
    let data = (0..pw * ph)
        .map(|i| {
            let (x, y) = ((i % pw) as f32, (i / pw) as f32);
            let hill = y > 150.0 + 30.0 * (x / 60.0).sin();
            let sun = (x - 270.0).powi(2) + (y - 60.0).powi(2) < 900.0;
            let (r, g, b) = if sun { (250, 220, 150) } else if hill { (40, 110 + (y as u32 % 40), 70) } else { (120 + (y as u32 / 3), 170, 220) };
            0xFF00_0000 | (r << 16) | (g << 8) | b
        })
        .collect();
    let pixels = std::sync::Arc::new(kolada_ui::pixels::Pixels { width: pw, height: ph, data });
    let pic = PicView { pixels: Some(pixels), width: 1920, height: 1280, size: "1.2 MB", ready: true };
    let c = |kind, text, from| ClipView { sent: q::Sent::None, from, goes: "", kind, text, title: "", copied: false, spent: false, mine: false, picture: None, more: &[], names: &[], caught: "", glow: 0.0, open: 0.0 };
    let text = [
        c(Kind::Text, "Taxi at 17:30 from the hotel's front door.", ""),
        c(Kind::Text, "Gate B, row 14. Boarding at 19:05.", "iPhone \u{00B7} 2 min ago"),
        ClipView { mine: true, ..c(Kind::Link, "https://blog.rust-lang.org/", "You \u{00B7} 1 hour ago") },
    ];
    let picture = [ClipView { picture: Some(pic.clone()), ..c(Kind::Picture, "IMG_2041.jpg", "iPhone \u{00B7} just now") }];
    // Several pictures, each its own scene's colours.
    let shade = |k: u32| {
        let data = (0..pw * ph)
            .map(|i| {
                let (x, y) = ((i % pw) as f32, (i / pw) as f32);
                let band = ((x + y * (k as f32 + 1.0)) / 60.0) as u32 % 2;
                let (r, g, b) = [(214, 120, 90), (90, 150, 200), (120, 170, 110), (200, 170, 90), (150, 120, 190), (90, 170, 170)][k as usize % 6];
                let d = if band == 0 { 0 } else { 30 };
                0xFF00_0000 | ((r - d) << 16) | ((g - d) << 8) | (b - d)
            })
            .collect();
        PicView { pixels: Some(std::sync::Arc::new(kolada_ui::pixels::Pixels { width: pw, height: ph, data })), width: 1600, height: 1200, size: "2.1 MB", ready: true }
    };
    let shots: Vec<PicView> = (0..6).map(shade).collect();
    let grids = [
        ClipView { picture: Some(shots[0].clone()), more: &shots[1..6], ..c(Kind::Picture, "IMG_3001.jpg", "iPhone \u{00B7} just now") },
        ClipView { picture: Some(shots[2].clone()), more: &shots[3..5], mine: true, ..c(Kind::Picture, "IMG_2990.jpg", "You \u{00B7} 5 min ago") },
        ClipView { picture: Some(shots[4].clone()), more: &shots[5..6], ..c(Kind::Picture, "IMG_2950.jpg", "iPhone \u{00B7} 1 hour ago") },
    ];
    const TO: [&str; 2] = ["All devices", "iPhone"];
    let main = |clips, staged| Screen::Main { find: q::Find::default(), staged, to: &TO, chosen: 0, clips, scroll: 0.0, saved: None, draft: "", selection: None, more: None };
    for (name, screen) in [
        ("popup-empty", main(&[], None)),
        ("popup-ready", main(&[], Some(Staged { what: "a picture", kind: Kind::Picture, text: "Screenshot.png", picture: Some(pic.clone()), more: &[], unsure: false }))),
        ("popup-text", main(&text, None)),
        ("popup-picture", main(&picture, None)),
        ("popup-staged-several", main(&picture, Some(Staged { what: "a picture", kind: Kind::Picture, text: "IMG_3001.jpg", picture: Some(shots[0].clone()), more: &shots[1..4], unsure: false }))),
        ("popup-grids", main(&grids, None)),
    ] {
        let view = PopupView { screen, devices: 1, said: None, host: q::Host { keys: "Alt+Z", ..q::Host::WINDOWS } };
        write(name, &q::popup(&dw, &fonts, &view, None, q::W, q::H).unwrap(), (q::W, q::H));
    }
    let rows = vec![
        SidebarRow { depth: 0, kind: RowKind::Folder { expanded: true }, name: "Notes".into() },
        SidebarRow { depth: 1, kind: RowKind::Note, name: "Trip plans".into() },
        SidebarRow { depth: 1, kind: RowKind::Note, name: "Meeting notes".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Reading list".into() },
    ];
    let note = "# Trip plans\nFlights, the hotel and what to pack.\n- [x] Book the hotel\n- [ ] Pack the charger\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(note).collect();
    let devices = [DeviceRow { name: "Windows laptop", look: Look::Laptop, this: true }, DeviceRow { name: "iPhone", look: Look::Phone, this: false }];
    let item = |icon: &'static str, label: &str, keys: &'static str| MenuItem { icon, label: label.into(), keys, ..MenuItem::default() };
    let items = vec![
        item("\u{E8C6}", "Cut", "Ctrl+X"),
        item("\u{E8C8}", "Copy", "Ctrl+C"),
        item("\u{E77F}", "Paste", "Ctrl+V"),
        MenuItem { divider: true, ..item("\u{E8DD}", "Bold", "Ctrl+B") },
        item("\u{E71B}", "Link", "Ctrl+K"),
    ];
    let (ww, wh) = (1100.0f32, 720.0f32);
    for name in ["window", "menu", "settings"] {
        let mut input = scene::Input {
            vault_name: "my notes",
            active_note: "Notes/Trip plans",
            text: note,
            sidebar: &rows,
            lines: &lines,
            options: [0; scene::SETTINGS],
            page_fade: 1.0,
            sidebar_dock: 1.0,
            sidebar_show: 1.0,
            toolbar_dock: 1.0,
            toolbar_show: 1.0,
            page_date: Some("Edited today, 3:42 PM \u{00B7} 24 words"),
            updater: true,
            update: Some("0.4.4"),
            quick: Some(scene::BeamView { unseen: 1, line: "iPhone \u{00B7} just now", typing: false, menu: None, check: None, keys: "Alt+Z" }),
            menu: (name == "menu").then_some(scene::MenuView { x: 520.0, y: 200.0, align_right: false, items: &items, alpha: 1.0 }),
            hover: (name == "menu").then_some(scene::Target::MenuItem(1)),
            devices: (name == "settings").then_some(scene::DevicesView { devices: &devices, pairing: Pairing::Idle, said: None, facts: &[], this: "PC", naming: None }),
            options_open: name == "settings",
            ..Default::default()
        };
        if name == "settings" {
            input.options_scroll = kolada_ui::chrome::devices_scroll(&input, ww);
        }
        write(name, &scene::build(&dw, &fonts, &input, ww, wh).unwrap(), (ww, wh));
    }
}

/// Beam's text cards by length: four lines, six, seven and twenty (wrapped
/// at the card's width), and an Arabic one, in the popup. Graphite and
/// paper, at 100% and 150%: `lines-<theme>-<scale>.bmp`.
#[test]
#[ignore = "writes Beam's text cards for a design review"]
fn dump_beam_lines() {
    use kolada_ui::quick::{self as q, ClipView, Kind, PopupView, Screen};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let words = |n: usize| (1..=n).map(|k| format!("Line {k}: the charger")).collect::<Vec<_>>().join("\n");
    let (four, six, seven) = (words(4), words(6), words(7));
    // One paragraph that wraps past six lines at the card's width.
    let twenty = "Pack the charger, the passport and the blue folder before the taxi comes at half past five. ".repeat(4);
    let arabic = (1..=7).map(|k| format!("\u{0627}\u{0644}\u{0633}\u{0637}\u{0631} {k} \u{0645}\u{0646} \u{062C}\u{062F}\u{0648}\u{0644} \u{0627}\u{0644}\u{062E}\u{0645}\u{064A}\u{0633}")).collect::<Vec<_>>().join("\n");
    let c = |text, from, mine| ClipView { sent: q::Sent::None, from, goes: "", kind: Kind::Text, text, title: "", copied: false, spent: false, mine, picture: None, more: &[], names: &[], caught: "", glow: 0.0, open: 0.0 };
    let clips = [
        c(&four, "iPhone \u{00B7} just now", false),
        c(&six, "", true),
        c(&seven, "Laptop \u{00B7} 5 min ago", false),
        c(&twenty, "", false),
        c(&arabic, "iPhone \u{00B7} 1 hour ago", false),
    ];
    const TO: [&str; 2] = ["All devices", "iPhone"];
    for (name, list) in [("lines", &clips[..4]), ("lines-rtl", &clips[4..])] {
        let view = PopupView { screen: Screen::Main { find: q::Find::default(), staged: None, to: &TO, chosen: 0, clips: list, scroll: 0.0, saved: None, draft: "", selection: None, more: None }, devices: 1, said: None, host: q::Host { keys: "Alt+Z", ..q::Host::WINDOWS } };
        let scene = q::popup(&dw, &fonts, &view, None, q::W, q::H).unwrap();
        for scale in [1.0f32, 1.5] {
            for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
                let list = display::record(&dw, &scene, theme, scale).unwrap();
                let px = ((q::W * scale).round() as u32, (q::H * scale).round() as u32, (96.0 * scale) as u32);
                Surface::draw_new(px, &list).dump(&dir.join(format!("{name}-{theme_name}-{}.bmp", (scale * 100.0) as u32))).unwrap();
            }
        }
    }
}

/// A picture opened full size from a note, on its way out and there,
/// graphite and paper: `lightbox-<step>-<theme>.bmp`. The picture is a
/// made-up gradient.
#[test]
#[ignore = "writes the full-size picture for manual inspection"]
fn dump_lightbox() {
    use std::sync::Arc;
    struct Made(Arc<kolada_ui::pixels::Pixels>);
    impl kolada_ui::images::PictureSource for Made {
        fn place(&mut self, _: &std::path::Path, _: &std::path::Path, _: &str) -> kolada_ui::images::Lookup {
            kolada_ui::images::Lookup::Found(std::path::PathBuf::from("coast.png"), 1600, 1000)
        }
        fn pixels(&mut self, _: &std::path::Path, _: u32) -> Option<(Arc<kolada_ui::pixels::Pixels>, f32)> {
            Some((self.0.clone(), 1.0))
        }
    }
    let (w, h) = (1600u32, 1000u32);
    let data = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
            let sky = (1.0 - y * 1.6).max(0.0);
            let sea = ((y - 0.62) * 3.0).clamp(0.0, 1.0);
            let r = (40.0 + 170.0 * sky * (1.0 - x * 0.3) + 10.0 * sea) as u32;
            let g = (80.0 + 130.0 * sky + 60.0 * sea) as u32;
            let b = (110.0 + 120.0 * sky + 70.0 * sea + 20.0 * x) as u32;
            0xFF00_0000 | (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
        })
        .collect();
    let images = std::cell::RefCell::new(Made(Arc::new(kolada_ui::pixels::Pixels { width: w, height: h, data })));
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let text = "# Coast weekend\nLeave Friday at nine, before the traffic.\n![[coast.png|360]]\nThe cottage has a woodstove, so bring matches.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let (width, height) = (1400.0, 900.0);
    let input = scene::Input {
        vault_name: "my notes",
        active_note: "Coast weekend",
        text,
        lines: &lines,
        options: [0; scene::SETTINGS],
        page_fade: 1.0,
        toolbar_dock: 0.0,
        images: Some(&images),
        scale: 1.0,
        ..Default::default()
    };
    let page = scene::build(&dw, &fonts, &input, width, height).unwrap();
    let from = page.hits.iter().find(|(_, t)| matches!(t, scene::Target::Picture(_))).map(|(r, _)| *r).expect("the picture on the page");
    let mut lightbox = kolada_ui::lightbox::Lightbox::open(std::path::PathBuf::from("coast.png"), from, (w, h));
    for (step, t) in [("page", 0.0), ("growing", 0.5), ("full", 1.0)] {
        while lightbox.t < t && lightbox.step(1.0 / 240.0) {}
        let shown = scene::Input { lightbox: Some(&lightbox), ..scene::Input { images: Some(&images), ..scene::Input { vault_name: "my notes", active_note: "Coast weekend", text, lines: &lines, options: [0; scene::SETTINGS], page_fade: 1.0, toolbar_dock: 0.0, scale: 1.0, ..Default::default() } } };
        for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
            let s = scene::build(&dw, &fonts, &shown, width, height).unwrap();
            let list = display::record(&dw, &s, theme, 1.0).unwrap();
            let surface = Surface::draw_new((width as u32, height as u32, 96), &list);
            surface.dump(&dir.join(format!("lightbox-{step}-{theme_name}.bmp"))).unwrap();
        }
    }
}

/// The notes as a grid under the note, in each of its four states (at rest,
/// writing, the boxes peeking, browsing them), graphite and paper, wide and
/// snapped side by side: `grid-<size>-<state>-<theme>.bmp`. The boxes are a
/// vault made here, read as the app reads them (`preview::of_note`); its
/// picture is a made-up gradient.
#[test]
#[ignore = "writes the grid's states for manual inspection"]
fn dump_grid() {
    use kolada_ui::grid::{self, BoxView, Grid, GridView, Phase};
    use std::sync::Arc;
    struct Made(Arc<kolada_ui::pixels::Pixels>);
    impl kolada_ui::images::PictureSource for Made {
        fn place(&mut self, _: &std::path::Path, _: &std::path::Path, _: &str) -> kolada_ui::images::Lookup {
            kolada_ui::images::Lookup::Missing
        }
        fn pixels(&mut self, _: &std::path::Path, _: u32) -> Option<(Arc<kolada_ui::pixels::Pixels>, f32)> {
            Some((self.0.clone(), 1.0))
        }
    }
    let (w, h) = (480u32, 320u32);
    let data = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
            let sky = (1.0 - y).max(0.0);
            let r = (40.0 + 150.0 * sky * (1.0 - x * 0.3)) as u32;
            let g = (90.0 + 120.0 * sky) as u32;
            let b = (110.0 + 120.0 * sky + 20.0 * x) as u32;
            0xFF00_0000 | (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
        })
        .collect();
    let images = std::cell::RefCell::new(Made(Arc::new(kolada_ui::pixels::Pixels { width: w, height: h, data })));

    let vault = std::env::temp_dir().join(format!("kolada-grid-dump-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&vault);
    std::fs::create_dir_all(vault.join("Trips")).unwrap();
    std::fs::write(vault.join("Trips").join("coast.png"), b"made up").unwrap();
    let notes: [(&str, &str); 10] = [
        ("Trips/Coast weekend", "Coast weekend\n![[coast.png]]\nLeave Friday at nine.\n"),
        ("Groceries", "Groceries\nOat milk, eggs, a loaf from the corner bakery and lemons.\nRice, the big bag this time.\nCoffee beans if they have the dark roast.\nSomething for Sunday lunch.\n"),
        ("Launch checklist", "Launch checklist\n- [x] Press kit\n- [ ] Pricing page\n- [ ] Store listing\n"),
        ("Budget", "Budget\n| Item | Cost | Paid |\n| --- | --: | --- |\n| Rent | 1200 | yes |\n| Power | 80 | no |\n| Phone | 25 | yes |\n"),
        ("Reading list", "Reading list\n1. The Overstory\n2. Piranesi\n3. A Psalm for the Wild-Built\n"),
        ("Ideas for the garden", "Ideas for the garden\nRaised beds along the south fence, and a bench under the plum.\nMove the rosemary to where it gets the afternoon sun.\nAsk next door about the shared hedge.\n"),
        ("Call with the builder", "Call with the builder\nThey can start in March if the permit comes through.\nThe quote holds until the end of January.\nThey need the drawings by Friday, and a key for the side gate.\nCall back after lunch.\n"),
        ("Packing", "Packing\n- Chargers\n- Rain jacket\n  - the light one\n"),
        ("Recipes to try", "Recipes to try\nLentil soup with lemon; flatbread on the cast iron pan.\n"),
        ("A note with a name long enough to run past its box", "A note with a name long enough to run past its box\nShort.\n"),
    ];
    let base = std::time::SystemTime::now();
    for (k, (rel, text)) in notes.iter().enumerate() {
        let path = vault.join(format!("{rel}.md"));
        std::fs::write(&path, text).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(base - std::time::Duration::from_secs(60 * k as u64)).unwrap();
    }
    let listed = kolada_core::vault::every_note(&vault);
    let mut known = None;
    let read: Vec<_> = listed.iter().map(|n| kolada_ui::preview::of_note(&vault, &n.path, &mut known)).collect();
    let boxes: Vec<BoxView> = listed
        .iter()
        .zip(&read)
        .map(|(n, r)| BoxView { name: &n.name, preview: r.as_ref().map(|r| &r.0), picture: r.as_ref().and_then(|r| r.1.as_deref()), snippet: None })
        .collect();

    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let note = "Coast weekend\nLeave Friday at nine, before the traffic. The cottage has a woodstove, so bring matches and the good blanket.\n\n- [ ] Book the ferry\n- [x] Ask Sam about the dog\n";
    for (size, width, height) in [("wide", 1600.0, 1000.0), ("snap", 800.0, 1000.0)] {
        for phase in [Phase::Rest, Phase::Writing, Phase::Peek, Phase::Browse] {
            let text = if phase == Phase::Rest { "" } else { note };
            let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
            let g = Grid::at(phase);
            let shown = grid::shown(&g, boxes.len(), width, height);
            let view = GridView { grid: &g, count: boxes.len(), first: shown.start, boxes: &boxes[shown.clone()], search: None, chosen: None };
            let input = scene::Input {
                vault_name: "my notes",
                active_note: if text.is_empty() { "" } else { "Trips/Coast weekend" },
                text,
                selection: (text.len().min(13), text.len().min(13)),
                lines: &lines,
                options: [0; scene::SETTINGS],
                page_fade: 1.0,
                toolbar_dock: 0.0,
                page_date: (!text.is_empty()).then_some("Edited today, 9:12 AM \u{00B7} 31 words"),
                images: Some(&images),
                scale: 1.0,
                grid: Some(view),
                // Peeking: the pointer at the window's foot.
                hover: (phase == Phase::Peek).then_some(scene::Target::GridSheet),
                ..Default::default()
            };
            for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
                let mut s = scene::build(&dw, &fonts, &input, width, height).unwrap();
                s.place_caret((0.0, 0.0));
                let list = display::record(&dw, &s, theme, 1.0).unwrap();
                let surface = Surface::draw_new((width as u32, height as u32, 96), &list);
                surface.dump(&dir.join(format!("grid-{size}-{phase:?}-{theme_name}.bmp").to_lowercase())).unwrap();
            }
        }
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// The grid's pill at the window's foot, where the boxes come up from:
/// writing, peeking and browsing, graphite and paper, at a wide window and
/// a side-by-side snap; and browsing with the pill riding the panel's top
/// edge (the prototype), wide: `grid-pill-<size>-<state>-<theme>.bmp`.
/// Made-up notes only.
#[test]
#[ignore = "writes the grid's pill at the foot for manual inspection"]
fn dump_grid_bottom_pill() {
    use kolada_ui::grid::{self, BoxView, Grid, GridView, Phase};
    use kolada_ui::preview::preview;
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let kinds = [
        preview("Groceries", "Groceries\nOat milk, eggs, a loaf from the corner bakery and lemons.\nRice, the big bag this time.\n"),
        preview("Launch checklist", "Launch checklist\n- [x] Press kit\n- [ ] Pricing page\n- [ ] Store listing\n"),
        preview("Reading list", "Reading list\n1. A novel about trees\n2. A book of essays\n3. Something short\n"),
        preview("Ideas for the garden", "Ideas for the garden\nRaised beds along the south fence, and a bench under the plum.\n"),
        preview("Call with the builder", "Call with the builder\nThey can start in March if the permit comes through.\n"),
    ];
    let names = ["Groceries", "Launch checklist", "Reading list", "Ideas for the garden", "Call with the builder"];
    let boxes: Vec<BoxView> = (0..20).map(|i| BoxView { name: names[i % 5], preview: Some(&kinds[i % 5]), picture: None, snippet: None }).collect();
    // A made-up note long enough to run past the window's foot.
    let mut note = String::from("Coast weekend\n");
    for i in 0..40 {
        note.push_str(&format!("Paragraph {} of a made-up note about the weekend at the coast: leave early, bring the good blanket, and book the ferry before Thursday.\n\n", i + 1));
    }
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(&note).collect();
    let mut shots = vec![];
    for (size, width, height) in [("wide", 1600.0, 1000.0), ("snap", 760.0, 1000.0)] {
        // Scrolled, a line runs across the foot: it dissolves before the pill.
        for (state, phase, rides, scroll) in [("writing", Phase::Writing, false, 0.0), ("writing-scrolled", Phase::Writing, false, 40.0), ("peek", Phase::Peek, false, 0.0), ("browse", Phase::Browse, false, 0.0), ("browse-riding", Phase::Browse, true, 0.0)] {
            if rides && size != "wide" {
                continue;
            }
            kolada_ui::chrome::ride_grid_pill(rides);
            let g = Grid::at(phase);
            let shown = grid::shown(&g, boxes.len(), width, height);
            let view = GridView { grid: &g, count: boxes.len(), first: shown.start, boxes: &boxes[shown.clone()], search: None, chosen: None };
            let input = scene::Input {
                vault_name: "my notes",
                active_note: "Coast weekend",
                text: &note,
                selection: (13, 13),
                lines: &lines,
                options: [0; scene::SETTINGS],
                page_fade: 1.0,
                toolbar_dock: 0.0,
                scroll_px: scroll,
                page_date: Some("Edited today, 9:12 AM \u{00B7} 912 words"),
                scale: 1.0,
                grid: Some(view),
                ..Default::default()
            };
            for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
                let mut s = scene::build(&dw, &fonts, &input, width, height).unwrap();
                s.place_caret((0.0, 0.0));
                let list = display::record(&dw, &s, theme, 1.0).unwrap();
                let surface = Surface::draw_new((width as u32, height as u32, 96), &list);
                let name = format!("grid-pill-{size}-{state}-{theme_name}.bmp");
                surface.dump(&dir.join(&name)).unwrap();
                shots.push((name, s.pills[1], s.pills[2], s.column));
            }
            kolada_ui::chrome::ride_grid_pill(false);
        }
    }
    for (name, top, foot, column) in shots {
        eprintln!("{name}: top pill {top:?}, foot pill {foot:?}, note column {column:?}");
    }
}

/// The top of the window, where the toolbar's pill is, with the side menu
/// docked and tucked away, at a wide size and a snap: `pill-<size>-<menu>.bmp`.
#[test]
#[ignore = "writes the window's top for manual inspection"]
fn dump_pill() {
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch plan".into() }];
    let text = "# Launch plan\nShip the public beta on the twelfth, then the press kit.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    for (size, width, height) in [("wide", 1600.0, 600.0), ("snap", 800.0, 600.0)] {
        for (menu, side) in [("menu", 1.0), ("no-menu", 0.0)] {
            let input = scene::Input {
                vault_name: "my notes",
                active_note: "Launch plan",
                text,
                sidebar: &rows,
                lines: &lines,
                options: [0; scene::SETTINGS],
                page_fade: 1.0,
                sidebar_dock: side,
                sidebar_show: side,
                toolbar_dock: 0.0,
                ..Default::default()
            };
            let s = scene::build(&dw, &fonts, &input, width, height).unwrap();
            let list = display::record(&dw, &s, &LIGHT, 1.0).unwrap();
            let surface = Surface::draw_new((width as u32, height as u32, 96), &list);
            surface.dump(&dir.join(format!("pill-{size}-{menu}.bmp"))).unwrap();
        }
    }
}

/// The list and grid icons up close (three times over), graphite and
/// paper: in the side menu (the list in use) and on the grid's edge (the
/// grid in use): `view-icons-<theme>.bmp`, the two strips one above the
/// other.
#[test]
#[ignore = "writes the view icons for manual inspection"]
fn dump_view_icons() {
    use kolada_ui::grid::{Grid, GridView, Phase};
    let dw = factory();
    let fonts = Fonts::new(3.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch plan".into() }];
    let (width, height) = (1100.0, 640.0);
    let g = Grid::at(Phase::Browse);
    for (name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
        let list = scene::Input { vault_name: "my notes", sidebar: &rows, options: [0; scene::SETTINGS], page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, scale: 3.0, ..Default::default() };
        let grid = scene::Input { grid: Some(GridView { grid: &g, count: 0, first: 0, boxes: &[], search: None, chosen: None }), ..scene::Input { options: [0; scene::SETTINGS], page_fade: 1.0, scale: 3.0, ..Default::default() } };
        let mut strips = Vec::new();
        for input in [&list, &grid] {
            let s = scene::build(&dw, &fonts, input, width, height).unwrap();
            let r = s.hits.iter().rev().find(|(_, t)| *t == scene::Target::ViewToggle).map(|(r, _)| *r).expect("the view icons");
            let rec = display::record(&dw, &s, theme, 3.0).unwrap();
            let surface = Surface::draw_new(((width * 3.0) as u32, (height * 3.0) as u32, 288), &rec);
            strips.push((surface, r));
        }
        // Around each pair: 90 DIPs either side of the toggle, 40 tall.
        let (cw, ch) = (180 * 3, 40 * 3);
        let mut out = Surface::new((cw, ch * 2, 288));
        for (k, (s, r)) in strips.iter().enumerate() {
            let (cx, cy) = (((r.left + r.right) / 2.0 * 3.0) as i64 - cw as i64 / 2, ((r.top + r.bottom) / 2.0 * 3.0) as i64 - ch as i64 / 2);
            for y in 0..ch as i64 {
                for x in 0..cw as i64 {
                    let (sx, sy) = ((cx + x).clamp(0, s.width as i64 - 1), (cy + y).clamp(0, s.height as i64 - 1));
                    out.pixels[((y + k as i64 * ch as i64) * cw as i64 + x) as usize] = s.pixels[(sy * s.width as i64 + sx) as usize];
                }
            }
        }
        out.dump(&dir.join(format!("view-icons-{name}.bmp"))).unwrap();
    }
}

/// Rows of icons up close (three times over) with one hovered in each, and
/// a dashed line through the hovered tile's middle: the side menu's foot,
/// the grid's top edge, the toolbar and the window's buttons, graphite and
/// paper: `icon-rows-<theme>.bmp`, one strip a row.
#[test]
#[ignore = "writes the icon rows for manual inspection"]
fn dump_icon_rows() {
    use kolada_ui::grid::{Grid, GridView, Phase};
    use scene::Target;
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch plan".into() }];
    let text = "# Plan\nShip it.";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let (width, height, k) = (1100.0, 640.0, 3.0);
    let g = Grid::at(Phase::Browse);
    for (name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
        // (the row's strip, DIPs; what is hovered; the grid or not)
        let shots: [(fn(f32) -> (f32, f32, f32), Target, bool); 4] = [
            (|_| (0.0, 280.0, 0.0), Target::Trash, false),
            (|w| (w - 330.0, w - 10.0, 0.0), Target::ThemeCycle, true),
            (|w| (w / 2.0 - 170.0, w / 2.0 + 170.0, 0.0), Target::Tool(scene::Tool::Bold), false),
            (|w| (w - 230.0, w, 0.0), Target::Close, false),
        ];
        let strip_h = 44.0;
        let cw = (340.0 * k) as u32;
        let mut out = Surface::new((cw, (strip_h * k) as u32 * shots.len() as u32, 288));
        for (n, (span, hover, grid)) in shots.iter().enumerate() {
            let base = scene::Input { vault_name: "my notes", active_note: "Plan", text, lines: &lines, sidebar: &rows, options: [0; scene::SETTINGS], page_fade: 1.0, scale: k, hover: Some(*hover), ..Default::default() };
            let input = if *grid {
                scene::Input { grid: Some(GridView { grid: &g, count: 0, first: 0, boxes: &[], search: None, chosen: None }), ..base }
            } else {
                scene::Input { sidebar_dock: 1.0, sidebar_show: 1.0, toolbar_dock: 1.0, toolbar_show: 1.0, ..base }
            };
            let s = scene::build(&dw, &fonts, &input, width, height).unwrap();
            let hit = s.hits.iter().rev().find(|(_, t)| t == hover).map(|(r, _)| *r).expect("the hovered button");
            let tile = s.fills.iter().filter(|f| matches!(f.ink, Ink::Hover) && f.rect.contains((hit.left + hit.right) / 2.0, (hit.top + hit.bottom) / 2.0)).map(|f| f.rect).last().unwrap_or(hit);
            let mid = (tile.top + tile.bottom) / 2.0;
            let rec = display::record(&dw, &s, theme, k).unwrap();
            let surface = Surface::draw_new(((width * k) as u32, (height * k) as u32, 288), &rec);
            let (x0, _, _) = span(width);
            let top = ((mid - strip_h / 2.0) * k) as i64;
            let sh = (strip_h * k) as i64;
            for y in 0..sh {
                for x in 0..cw as i64 {
                    let (sx, sy) = ((x0 * k) as i64 + x, top + y);
                    let p = if sx >= 0 && sy >= 0 && sx < surface.width as i64 && sy < surface.height as i64 { surface.pixels[(sy * surface.width as i64 + sx) as usize] } else { 0xFF80_8080 };
                    // The tile's middle, dashed across the row.
                    let line = (sy as f32 - mid * k).abs() < 0.5 && (x / 6) % 2 == 0;
                    out.pixels[((y + n as i64 * sh) * cw as i64 + x) as usize] = if line { 0xFFE0_3030 } else { p };
                }
            }
        }
        out.dump(&dir.join(format!("icon-rows-{name}.bmp"))).unwrap();
    }
}

/// The view's one button up close (three times over), graphite and paper:
/// in the side menu (the list in use: the grid a press away) and on the
/// grid's edge (the grid in use: the list a press away), at rest and
/// pointed at: `view-toggle-<theme>.bmp`, four strips.
#[test]
#[ignore = "writes the view's button for manual inspection"]
fn dump_view_toggle() {
    use kolada_ui::grid::{Grid, GridView, Phase};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Launch plan".into() }];
    let (width, height, k) = (1100.0, 640.0, 3.0);
    let g = Grid::at(Phase::Browse);
    for (name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
        let (cw, ch) = (260 * 3, 44 * 3);
        let mut out = Surface::new((cw, ch * 4, 288));
        let mut n = 0;
        for grid in [false, true] {
            for hover in [None, Some(scene::Target::ViewToggle)] {
                let base = scene::Input { vault_name: "my notes", sidebar: &rows, options: [0; scene::SETTINGS], page_fade: 1.0, scale: k, hover, ..Default::default() };
                let input = if grid { scene::Input { grid: Some(GridView { grid: &g, count: 0, first: 0, boxes: &[], search: None, chosen: None }), ..base } } else { scene::Input { sidebar_dock: 1.0, sidebar_show: 1.0, ..base } };
                let s = scene::build(&dw, &fonts, &input, width, height).unwrap();
                let r = s.hits.iter().rev().find(|(_, t)| *t == scene::Target::ViewToggle).map(|(r, _)| *r).expect("the view's button");
                let rec = display::record(&dw, &s, theme, k).unwrap();
                let surface = Surface::draw_new(((width * k) as u32, (height * k) as u32, 288), &rec);
                // The button and what is beside it: from its left, its row.
                let (x0, y0) = (((r.left - 20.0) * k) as i64, (((r.top + r.bottom) / 2.0 - 22.0) * k) as i64);
                for y in 0..ch as i64 {
                    for x in 0..cw as i64 {
                        let (sx, sy) = ((x0 + x).clamp(0, surface.width as i64 - 1), (y0 + y).clamp(0, surface.height as i64 - 1));
                        out.pixels[((y + n * ch as i64) * cw as i64 + x) as usize] = surface.pixels[(sy * surface.width as i64 + sx) as usize];
                    }
                }
                n += 1;
            }
        }
        out.dump(&dir.join(format!("view-toggle-{name}.bmp"))).unwrap();
    }
}

/// The grid's bar up close, three times over, graphite and paper:
/// `grid-bar-<theme>.bmp`.
#[test]
#[ignore = "writes the grid's bar for manual inspection"]
fn dump_grid_bar() {
    use kolada_ui::grid::{Grid, GridView, Phase};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let (width, height, k) = (1100.0, 640.0, 3.0);
    let g = Grid::at(Phase::Browse);
    let quick = scene::BeamView { unseen: 0, line: "", typing: false, menu: None, check: None, keys: "Alt+Z" };
    for (name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
        let input = scene::Input { options: [0; scene::SETTINGS], page_fade: 1.0, scale: k, quick: Some(quick), grid: Some(GridView { grid: &g, count: 0, first: 0, boxes: &[], search: None, chosen: None }), ..Default::default() };
        let s = scene::build(&dw, &fonts, &input, width, height).unwrap();
        let bar = s.hits.iter().find(|(_, t)| *t == scene::Target::NewNote).map(|(r, _)| *r).unwrap();
        let rec = display::record(&dw, &s, theme, k).unwrap();
        let surface = Surface::draw_new(((width * k) as u32, (height * k) as u32, 288), &rec);
        let (x0, y0, w, h) = (((bar.left - 12.0) * k) as u32, ((bar.top - 8.0) * k) as u32, (300.0 * k) as u32, ((bar.height() + 16.0) * k) as u32);
        let mut out = Surface::new((w, h, 288));
        for y in 0..h {
            for x in 0..w {
                out.pixels[(y * w + x) as usize] = surface.pixels[((y0 + y) * surface.width + x0 + x) as usize];
            }
        }
        out.dump(&dir.join(format!("grid-bar-{name}.bmp"))).unwrap();
    }
}

/// A picture the vault does not have, in a note and on its card, graphite
/// and paper: `missing-picture-<theme>.bmp`.
#[test]
#[ignore = "writes the missing picture's tile for manual inspection"]
fn dump_missing_picture() {
    use kolada_ui::grid::{BoxView, Grid, GridView, Phase};
    use std::sync::Arc;
    struct Gone;
    impl kolada_ui::images::PictureSource for Gone {
        fn place(&mut self, _: &std::path::Path, _: &std::path::Path, _: &str) -> kolada_ui::images::Lookup {
            kolada_ui::images::Lookup::Missing
        }
        fn pixels(&mut self, _: &std::path::Path, _: u32) -> Option<(Arc<kolada_ui::pixels::Pixels>, f32)> {
            None
        }
    }
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let (width, height, k) = (900.0, 640.0, 3.0);
    let text = "# Coast weekend\n\n![[Harbor at dusk.png]]\n\nLeave Friday at nine.";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let images = std::cell::RefCell::new(Gone);
    let preview = kolada_ui::preview::preview("Coast weekend", text);
    let boxes = [BoxView { name: "Coast weekend", preview: Some(&preview), picture: None, snippet: None }];
    let g = Grid::at(Phase::Browse);
    for (name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
        for (shot, grid) in [("note", None), ("card", Some(GridView { grid: &g, count: 1, first: 0, boxes: &boxes, search: None, chosen: None }))] {
            let input = scene::Input { text, lines: &lines, options: [0; scene::SETTINGS], page_fade: 1.0, scale: k, images: Some(&images), vault_path: "vault", grid, ..Default::default() };
            let s = scene::build(&dw, &fonts, &input, width, height).unwrap();
            let rec = display::record(&dw, &s, theme, k).unwrap();
            let surface = Surface::draw_new(((width * k) as u32, (height * k) as u32, 288), &rec);
            surface.dump(&dir.join(format!("missing-picture-{shot}-{name}.bmp"))).unwrap();
        }
    }
}

/// Pictures in a list item, a task, a numbered item, a quote and a nested
/// list, graphite and paper: `nested-pictures-<theme>.bmp`.
#[test]
#[ignore = "writes pictures inside lists and quotes for manual inspection"]
fn dump_nested_pictures() {
    use std::sync::Arc;
    struct Made(Arc<kolada_ui::pixels::Pixels>);
    impl kolada_ui::images::PictureSource for Made {
        fn place(&mut self, _: &std::path::Path, _: &std::path::Path, _: &str) -> kolada_ui::images::Lookup {
            kolada_ui::images::Lookup::Found("made.png".into(), 480, 200)
        }
        fn pixels(&mut self, _: &std::path::Path, _: u32) -> Option<(Arc<kolada_ui::pixels::Pixels>, f32)> {
            Some((self.0.clone(), 1.0))
        }
    }
    let (w, h) = (480u32, 200u32);
    let data = (0..w * h).map(|i| 0xFF00_0000 | (((i % w) * 255 / w) << 16) | (90 << 8) | 140).collect();
    let images = std::cell::RefCell::new(Made(Arc::new(kolada_ui::pixels::Pixels { width: w, height: h, data })));
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let (width, height, k) = (900.0, 1500.0, 3.0);
    let text = "# Trip\n- Bags\n- ![[coast.png|160]]\n- [ ] ![[coast.png|160]]\n1. ![[coast.png|160]]\n> ![[coast.png|160]]\n- Outer\n    - ![[coast.png|160]]\nAfter.";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    for (name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
        let input = scene::Input { text, lines: &lines, options: [0; scene::SETTINGS], page_fade: 1.0, scale: k, images: Some(&images), vault_path: "vault", ..Default::default() };
        let s = scene::build(&dw, &fonts, &input, width, height).unwrap();
        let rec = display::record(&dw, &s, theme, k).unwrap();
        let surface = Surface::draw_new(((width * k) as u32, (height * k) as u32, 288), &rec);
        surface.dump(&dir.join(format!("nested-pictures-{name}.bmp"))).unwrap();
    }
}

/// A note beamed with its picture, as its card shows it: a heading, a
/// line, the picture it embeds, tasks, and an embed whose picture didn't
/// come; opened whole. Graphite and paper, at 100% and 150%:
/// `note-card-<theme>-<scale>.bmp`.
#[test]
#[ignore = "writes a beamed note's card for a design review"]
fn dump_beam_note() {
    use kolada_ui::quick::{self as q, ClipView, Kind, PicView, PopupView, Screen};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let (pw, ph) = (360u32, 240u32);
    let data = (0..pw * ph)
        .map(|i| {
            let (x, y) = ((i % pw) as f32, (i / pw) as f32);
            let hill = y > 150.0 + 30.0 * (x / 60.0).sin();
            let sun = (x - 270.0).powi(2) + (y - 60.0).powi(2) < 900.0;
            let (r, g, b) = if sun { (250, 220, 150) } else if hill { (40, 110 + (y as u32 % 40), 70) } else { (120 + (y as u32 / 3), 170, 220) };
            0xFF00_0000 | (r << 16) | (g << 8) | b
        })
        .collect();
    let pic = PicView { pixels: Some(std::sync::Arc::new(kolada_ui::pixels::Pixels { width: pw, height: ph, data })), width: 1600, height: 1067, size: "1.4 MB", ready: true };
    let text = "# Welcome to Kolada\nNotes from the trip, with the view from the hotel.\n![[Pasted image 20261103175852.png]]\n- [x] Book the hotel\n- [ ] Pack the charger\n![](maps/route.png)\n";
    let pics = [pic];
    let clips = [ClipView { sent: q::Sent::None, from: "iPhone \u{00B7} just now", goes: "With 1 picture", kind: Kind::Text, text, title: "Welcome", copied: false, spent: false, mine: false, picture: None, more: &pics, names: &["Pasted image 20261103175852.png"], caught: "", glow: 0.0, open: 1.0 }];
    const TO: [&str; 2] = ["All devices", "iPhone"];
    let view = PopupView { screen: Screen::Main { find: q::Find::default(), staged: None, to: &TO, chosen: 0, clips: &clips, scroll: 0.0, saved: None, draft: "", selection: None, more: None }, devices: 1, said: None, host: q::Host { keys: "Alt+Z", ..q::Host::WINDOWS } };
    let scene = q::popup(&dw, &fonts, &view, None, q::W, q::H).unwrap();
    for scale in [1.0f32, 1.5] {
        for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
            let list = display::record(&dw, &scene, theme, scale).unwrap();
            let px = ((q::W * scale).round() as u32, (q::H * scale).round() as u32, (96.0 * scale) as u32);
            Surface::draw_new(px, &list).dump(&dir.join(format!("note-card-{theme_name}-{}.bmp", (scale * 100.0) as u32))).unwrap();
        }
    }
}

/// Search in Beam and a caught page's card, as the app draws them, for the
/// design review: at rest with the pointer over the top bar (the search
/// glyph), searching (marks and count), Arabic, nothing found, and a result
/// opened onto (washed in teal). Writes `search-<state>-<theme>.bmp` at
/// 150% to KOLADA_DUMP_DIR.
#[test]
#[ignore = "writes Search in Beam for a design review"]
fn dump_beam_search() {
    use kolada_ui::quick::{self as q, ClipView, Find, Hit, Kind, PicView, PopupView, Screen};
    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let (pw, ph) = (360u32, 240u32);
    let data = (0..pw * ph)
        .map(|i| {
            let (x, y) = ((i % pw) as f32, (i / pw) as f32);
            let hill = y > 150.0 + 30.0 * (x / 60.0).sin();
            let (r, g, b) = if hill { (40, 110 + (y as u32 % 40), 70) } else { (120 + (y as u32 / 3), 170, 220) };
            0xFF00_0000 | (r << 16) | (g << 8) | b
        })
        .collect();
    let pic = PicView { pixels: Some(std::sync::Arc::new(kolada_ui::pixels::Pixels { width: pw, height: ph, data })), width: 1600, height: 1067, size: "1.4 MB", ready: true };
    let c = |kind, text, title, from, mine| ClipView { sent: q::Sent::None, from, goes: "", kind, text, title, copied: false, spent: false, mine, picture: None, more: &[], names: &[], caught: "", glow: 0.0, open: 0.0 };
    let all = [
        ClipView { mine: true, ..c(Kind::Text, "Door code for the studio: 4931", "", "You \u{00B7} just now", true) },
        c(Kind::Text, "موعد الإجتماع مع فريق التصميم يوم الخميس", "", "iPad \u{00B7} 20 min ago", false),
        ClipView { caught: "longreads.example", ..c(Kind::Text, "# Inside a design studio", "Inside a design studio that never meets", "Chrome \u{00B7} yesterday", false) },
        c(Kind::Link, "https://blog.rust-lang.org/", "Rust release notes", "You \u{00B7} yesterday", true),
        ClipView { picture: Some(pic.clone()), ..c(Kind::Picture, "Studio whiteboard.jpg", "", "iPhone \u{00B7} 2 days ago", false) },
        c(Kind::Text, "Gate B, row 14. Boarding at 19:05.", "", "iPhone \u{00B7} 3 days ago", false),
    ];
    let studio = [all[0].clone(), all[2].clone(), all[4].clone()];
    let arabic = [all[1].clone()];
    let mut washed = all.clone();
    washed[2].glow = 0.8;
    const TO: [&str; 2] = ["All devices", "iPhone"];
    let states: [(&str, &[ClipView], Find, Option<Hit>); 5] = [
        ("rest", &all, Find::default(), Some(Hit::Header)),
        ("studio", &studio, Find { query: Some("studio"), reveal: None }, None),
        ("arabic", &arabic, Find { query: Some("الاجتماع"), reveal: None }, None),
        ("none", &[], Find { query: Some("invoice"), reveal: None }, None),
        ("result", &washed, Find { query: None, reveal: Some(2) }, None),
    ];
    for (name, clips, find, hover) in states {
        let view = PopupView { screen: Screen::Main { find, staged: None, to: &TO, chosen: 0, clips, scroll: 0.0, saved: None, draft: "", selection: None, more: None }, devices: 2, said: None, host: q::Host { keys: "Alt+Z", ..q::Host::WINDOWS } };
        let scene = q::popup(&dw, &fonts, &view, hover.map(crate::scene::Target::Quick), q::W, q::H).unwrap();
        for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
            let list = display::record(&dw, &scene, theme, 1.5).unwrap();
            let px = ((q::W * 1.5).round() as u32, (q::H * 1.5).round() as u32, 144);
            Surface::draw_new(px, &list).dump(&dir.join(format!("search-{name}-{theme_name}.bmp"))).unwrap();
        }
    }
}

/// A note opened from search, and the search's previews: the results' rows
/// with what was found tinted, the grid's cards (a picture's with one line
/// of the match over it), the note opened on its first match a little above
/// the middle with every match tinted, and the same in Arabic; graphite and
/// paper, 1600 x 1000 at 2x: `found-<what>-<theme>.bmp`. Made-up notes.
#[test]
#[ignore = "writes a search's matches, in its previews and the note it opened, for a design review"]
fn dump_search_highlight() {
    use kolada_core::search::{self, Indexed};
    use kolada_ui::grid::{self, BoxView, Grid, GridSearch, GridView, Phase};
    use std::sync::Arc;
    struct Made(Arc<kolada_ui::pixels::Pixels>);
    impl kolada_ui::images::PictureSource for Made {
        fn place(&mut self, _: &std::path::Path, _: &std::path::Path, _: &str) -> kolada_ui::images::Lookup {
            kolada_ui::images::Lookup::Missing
        }
        fn pixels(&mut self, _: &std::path::Path, _: u32) -> Option<(Arc<kolada_ui::pixels::Pixels>, f32)> {
            Some((self.0.clone(), 1.0))
        }
    }
    let (pw, ph) = (480u32, 320u32);
    let data = (0..pw * ph)
        .map(|i| {
            let (x, y) = ((i % pw) as f32 / pw as f32, (i / pw) as f32 / ph as f32);
            let sky = (1.0 - y).max(0.0);
            let (r, g, b) = ((40.0 + 150.0 * sky * (1.0 - x * 0.3)) as u32, (90.0 + 120.0 * sky) as u32, (110.0 + 120.0 * sky + 20.0 * x) as u32);
            0xFF00_0000 | (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
        })
        .collect();
    let images = std::cell::RefCell::new(Made(Arc::new(kolada_ui::pixels::Pixels { width: pw, height: ph, data })));

    // The notes, made up, in a vault of their own.
    let mut kitchen = String::from("Kitchen notes\n\n");
    for i in 0..60 {
        kitchen.push_str(&match i {
            31 => "The lemon tart wants two Lemons, a slow oven and an hour to set.".to_string(),
            33 => "Zest a LEMON over the top before it cools; a lemon a tart is plenty.".to_string(),
            36 => "Next time: lemon curd in jars, for the neighbours.".to_string(),
            _ => format!("Bread, day {i}: flour, water, salt and a warm place to rise overnight."),
        });
        kitchen.push_str("\n\n");
    }
    let arabic = "\u{648}\u{635}\u{641}\u{627}\u{62A}\n\n\u{62E}\u{628}\u{632} \u{627}\u{644}\u{635}\u{628}\u{627}\u{62D} \u{648}\u{627}\u{644}\u{634}\u{627}\u{64A}.\n\n\u{639}\u{635}\u{64A}\u{631} \u{627}\u{644}\u{644}\u{651}\u{64E}\u{64A}\u{652}\u{645}\u{64F}\u{648}\u{646}\u{650} \u{627}\u{644}\u{637}\u{627}\u{632}\u{62C} \u{645}\u{639} \u{627}\u{644}\u{646}\u{639}\u{646}\u{627}\u{639}.\n\n\u{643}\u{639}\u{643}\u{629} \u{627}\u{644}\u{644}\u{64A}\u{645}\u{640}\u{640}\u{648}\u{646} \u{644}\u{644}\u{636}\u{64A}\u{648}\u{641}\u{60C} \u{648} lemon tart \u{644}\u{644}\u{623}\u{637}\u{641}\u{627}\u{644}.\n";
    let notes: [(&str, String); 7] = [
        ("Kitchen notes", kitchen.clone()),
        ("Lemon curd", "Lemon curd\n\nFour eggs, butter, sugar and the juice of three.\n".to_string()),
        ("Groceries", "Groceries\n\nOat milk, eggs, a loaf from the corner bakery and the lemons for Sunday lunch, if they have them.\n".to_string()),
        ("Market day", "Market day\n\n- Bring a bag for the lemons and the herbs\n- Ask about the olive oil\n- Flowers\n".to_string()),
        ("Trips/Coast weekend", "Coast weekend\n![[coast.png]]\nPack a lemon cake for the ferry.\n".to_string()),
        ("Budget", "Budget\n| Item | Cost |\n| --- | --: |\n| Lemons | 4 |\n| Flour | 3 |\n".to_string()),
        ("\u{648}\u{635}\u{641}\u{627}\u{62A}", arabic.to_string()),
    ];
    let vault = std::env::temp_dir().join(format!("kolada-found-dump-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&vault);
    std::fs::create_dir_all(vault.join("Trips")).unwrap();
    std::fs::write(vault.join("Trips").join("coast.png"), b"made up").unwrap();
    for (rel, text) in &notes {
        std::fs::write(vault.join(format!("{rel}.md")), text).unwrap();
    }
    let index: Vec<Indexed> = notes.iter().map(|(rel, text)| Indexed::new(format!("{rel}.md"), text.clone())).collect();

    let dw = factory();
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let (width, height, scale) = (1600.0f32, 1000.0f32, 2.0f32);
    let rows = vec![
        SidebarRow { depth: 0, kind: RowKind::Folder { expanded: false }, name: "Trips".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Kitchen notes".into() },
        SidebarRow { depth: 0, kind: RowKind::Note, name: "Groceries".into() },
    ];
    let write = |s: &mut Scene, name: &str| {
        s.place_caret((0.0, 0.0));
        for (theme_name, theme) in [("graphite", &DARK), ("paper", &LIGHT)] {
            let list = display::record(&dw, s, theme, scale).unwrap();
            let surface = Surface::draw_new(((width * scale) as u32, (height * scale) as u32, (96.0 * scale) as u32), &list);
            surface.dump(&dir.join(format!("found-{name}-{theme_name}.bmp"))).unwrap();
        }
    };
    // The view that puts `at` where a note opened from search puts it: as
    // the shells do (`found::view_at`, then their scroll, stopping at the top).
    let opened_at = |text: &str, lines: &[kolada_markdown::Line], at: usize, column: scene::Rect| -> (usize, f32) {
        let nt = crate::note::NoteText { eng: &dw, fonts: &fonts, text, lines, width: column.width(), title: crate::note::title_line(text, lines), caret: Some(at), clean: true, pictures: None, tables: Default::default(), folded: &[], front: Default::default() };
        let (mut line, mut px) = kolada_ui::found::view_at(&nt, at, column.top, height).unwrap();
        loop {
            if px < 0.0 {
                if line == 0 {
                    return (0, 0.0);
                }
                let prev = nt.para_at(line - 1).unwrap();
                line = prev.start;
                px += nt.block_height(&prev).unwrap();
            } else {
                let p = nt.para_at(line).unwrap();
                let h = nt.block_height(&p).unwrap();
                if px < h || p.end >= text.len() {
                    return (line, px);
                }
                px -= h;
                line = p.end + 1;
            }
        }
    };

    for (what, query, note) in [("rows", "lemon", 0usize), ("arabic", "\u{644}\u{64A}\u{645}\u{648}\u{646}", 6)] {
        let hits = search::search(&index, query, 40);
        let (rel, text) = (notes[note].0, notes[note].1.as_str());
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        let found = kolada_ui::found::Found::opened(text, &lines, query, 1).expect("the note has it");
        let caret = found.caret().unwrap_or(0);
        let base = |search: scene::SearchView<'_>, found: &[(usize, usize)], (scroll_line, scroll_px): (usize, f32)| {
            let input = scene::Input {
                vault_name: "my notes",
                active_note: rel,
                text,
                sidebar: &rows,
                lines: &lines,
                selection: (caret, caret),
                scroll_line,
                scroll_px,
                options: [0; scene::SETTINGS],
                page_fade: 1.0,
                sidebar_dock: 1.0,
                sidebar_show: 1.0,
                toolbar_dock: 0.0,
                search: Some(search),
                found,
                scale,
                ..Default::default()
            };
            scene::build(&dw, &fonts, &input, width, height).unwrap()
        };
        let typing = scene::SearchView { query, caret: query.len(), hits: &hits, selected: 0, first: 0, commands: None, focused: true };
        // (a), (d): the results as they are typed, the note not opened yet.
        let mut s = base(typing, &[], (0, 0.0));
        write(&mut s, &format!("{what}-results"));
        // (c), (d): the note opened from the first result, on its first match.
        let kept = scene::SearchView { query, caret: query.len(), hits: &hits, selected: 0, first: 0, commands: None, focused: false };
        let view = opened_at(text, &lines, caret, s.column);
        let mut s = base(kept, found.ranges(1), view);
        let tints = s.fills.iter().filter(|f| f.ink == Ink::Mark && f.radius == 3.0 && f.rect.left >= s.column.left - 2.0).count();
        assert!(tints >= 2, "{what}: the matches tinted in the note ({tints})");
        if let Some(c) = s.caret.filter(|_| view.0 > 0) {
            assert!(c.top >= height * 0.3 && c.top <= height * 0.5, "{what}: the first match a little above the middle ({})", c.top);
        }
        write(&mut s, &format!("{what}-opened"));
    }

    // (b): the grid's search, the matches' cards: words with the match
    // tinted, and a picture's, a list's and a table's with one line over them.
    let hits = search::search(&index, "lemon", 40);
    let matches = grid::matches(&hits);
    let mut known = None;
    let read: Vec<_> = matches.iter().map(|(path, _, _)| kolada_ui::preview::of_note(&vault, path, &mut known)).collect();
    let boxes: Vec<BoxView> = matches
        .iter()
        .zip(&read)
        .map(|((_, name, snippet), r)| BoxView { name, preview: r.as_ref().map(|r| &r.0), picture: r.as_ref().and_then(|r| r.1.as_deref()), snippet: snippet.as_deref() })
        .collect();
    assert!(boxes.iter().any(|b| b.picture.is_some() && b.snippet.is_some()), "a picture's card among the matches");
    let g = Grid::at(Phase::Browse);
    let shown = grid::shown(&g, boxes.len(), width, height);
    let view = GridView { grid: &g, count: boxes.len(), first: shown.start, boxes: &boxes[shown.clone()], search: Some(GridSearch { query: "lemon", caret: Some(5) }), chosen: Some(0) };
    let note = "Kitchen notes\nBread, day 1: flour, water, salt and a warm place to rise overnight.\n";
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(note).collect();
    let input = scene::Input {
        vault_name: "my notes",
        active_note: "Kitchen notes",
        text: note,
        lines: &lines,
        selection: (13, 13),
        options: [0; scene::SETTINGS],
        page_fade: 1.0,
        toolbar_dock: 0.0,
        images: Some(&images),
        scale,
        grid: Some(view),
        search: Some(scene::SearchView { query: "lemon", caret: 5, hits: &hits, selected: 0, first: 0, commands: None, focused: true }),
        ..Default::default()
    };
    let mut s = scene::build(&dw, &fonts, &input, width, height).unwrap();
    write(&mut s, "grid");
    let _ = std::fs::remove_dir_all(&vault);
}
