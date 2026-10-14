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
        options: [0; 13],
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
        ("options-dark", &DARK, 1400.0, (1.0, 1.0), (1.0, 1.0), Some(scene::Target::Choice(scene::Setting::TextSize, 2)), rest, (2, 2)),
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
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
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
        options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
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
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
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
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
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
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
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
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
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
            options: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
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
            options: [0; 13],
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
            options: [0; 13],
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
        let mut s = scene::build(&dw, &fonts, &input, 1268.0, 754.0).unwrap();
        s.place_caret((0.0, 0.0));
        let list = display::record(&dw, &s, theme, 1.5).unwrap();
        let surface = Surface::draw_new((1902, 1131, 144), &list);
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
        let shape = Some(WaveShape { vertical: true, d: *d });
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
            options: [2, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0],
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
                options: [0; 13],
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
                ..Default::default()
            };
            let mut s = scene::build(&dw, &fonts, &input, width, height).unwrap();
            s.place_caret((0.0, 0.0));
            let list = display::record(&dw, &s, &LIGHT, 1.0).unwrap();
            let surface = Surface::draw_new((width as u32, height as u32, 96), &list);
            surface.dump(&dir.join(format!("size-{size}-{state}.bmp"))).unwrap();
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
                options: [0; 13],
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
                options: [0; 13],
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
            options: [0; 13],
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
            options: [0; 13],
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
    let (width, height) = (900.0, 600.0);
    let draw = |toolbar: f32| {
        let input = scene::Input {
            vault_name: "notes",
            active_note: "Plan",
            text,
            sidebar: &rows,
            lines: &lines,
            options: [0; 13],
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
        let fill = Fill { wave: Some(WaveShape { vertical: false, d }), ..Fill::solid(pill, 3.0, Ink::Text) };
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
            options: [0; 13],
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
                options: [0; 13],
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
