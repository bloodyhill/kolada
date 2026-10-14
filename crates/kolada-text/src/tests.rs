use super::*;

/// An engine on fonts every machine of the OS has, or None where there are
/// none (the test then says so and passes: CI images vary).
fn engine() -> Option<Engine> {
    let set = |dir: &str, files: &[&str], body: &str, mono: &str| FontSet {
        files: files.iter().map(|f| PathBuf::from(dir).join(f)).filter(|p| p.is_file()).collect(),
        fallback: Vec::new(), data: Vec::new(),
        ui: body.into(),
        text: body.into(),
        code: mono.into(),
        icon: body.into(),
    };
    let candidates = [
        set(r"C:\Windows\Fonts", &["arial.ttf", "arialbd.ttf", "ariali.ttf", "cour.ttf"], "Arial", "Courier New"),
        set("/System/Library/Fonts/Supplemental", &["Arial.ttf", "Arial Bold.ttf", "Arial Italic.ttf", "Courier New.ttf"], "Arial", "Courier New"),
        set("/usr/share/fonts/truetype/dejavu", &["DejaVuSans.ttf", "DejaVuSans-Bold.ttf", "DejaVuSans-Oblique.ttf", "DejaVuSansMono.ttf"], "DejaVu Sans", "DejaVu Sans Mono"),
    ];
    let set = candidates.into_iter().find(|s| !s.files.is_empty());
    if set.is_none() {
        eprintln!("no known fonts on this machine: test skipped");
    }
    set.map(Engine::new)
}

fn layout(e: &mut Engine, text: &str, width: f32) -> Layout {
    Layout::new(e, text, &Format::new(Role::Text, 16.0), width, 400.0, &[])
}

#[test]
fn wraps_into_lines_that_cover_the_text_without_gaps() {
    let Some(mut e) = engine() else { return };
    let text = "The quick brown fox jumps over the lazy dog while the notes app keeps up with every keystroke.";
    let l = layout(&mut e, text, 200.0);
    let lines = l.lines();
    assert!(lines.len() >= 4, "{lines:?}");
    assert_eq!(lines[0].start, 0);
    assert_eq!(lines.last().unwrap().end, text.len());
    for w in lines.windows(2) {
        assert_eq!(w[0].end, w[1].start, "no byte is lost or doubled between lines");
        assert!(w[1].top > w[0].top);
    }
    let m = l.measure();
    assert_eq!(m.lines, lines.len());
    assert!(m.width <= 200.5, "{m:?}");
    assert_eq!(m.height, lines.len() as f32 * 20.0, "every line is as tall as the format says");
}

#[test]
fn a_point_on_a_caret_hits_that_caret() {
    let Some(mut e) = engine() else { return };
    let text = "hello wrapped world of notes that goes on and on";
    let l = layout(&mut e, text, 140.0);
    for at in text.char_indices().map(|(i, _)| i) {
        let c = l.caret(at);
        let (back, inside) = l.offset_at(c.x + 0.2, c.y + c.height / 2.0);
        assert!(inside);
        assert_eq!(back, at, "caret {at} at {c:?}");
    }
}

#[test]
fn clicking_past_the_ends_lands_at_the_ends() {
    let Some(mut e) = engine() else { return };
    let l = layout(&mut e, "short", 300.0);
    assert_eq!(l.offset_at(-20.0, 5.0).0, 0);
    assert_eq!(l.offset_at(290.0, 5.0).0, 5);
    assert_eq!(l.offset_at(50.0, 900.0).0, 5, "below the last line is its end");
    assert!(!l.offset_at(50.0, 900.0).1);
}

#[test]
fn the_end_of_the_text_has_a_caret_after_the_last_letter() {
    let Some(mut e) = engine() else { return };
    let l = layout(&mut e, "abc", 300.0);
    let (a, b) = (l.caret(2), l.caret(3));
    assert!(b.x > a.x && b.y == a.y);
}

#[test]
fn an_empty_paragraph_is_one_line_with_its_caret_at_the_start() {
    let Some(mut e) = engine() else { return };
    let l = layout(&mut e, "", 300.0);
    assert_eq!(l.lines().len(), 1);
    assert_eq!(l.measure().lines, 1);
    assert_eq!(l.caret(0), Caret { x: 0.0, y: 0.0, height: 20.0 });
}

#[test]
fn a_blank_line_forced_right_to_left_keeps_its_caret_on_the_right() {
    let Some(mut e) = engine() else { return };
    let mut f = Format::new(Role::Text, 16.0);
    f.rtl = Some(true);
    let l = Layout::new(&mut e, "", &f, 300.0, 100.0, &[]);
    assert!(l.is_rtl());
    assert_eq!(l.caret(0).x, 300.0);
    // Digits alone have no direction of their own: the forced direction decides.
    let l = Layout::new(&mut e, "123", &f, 300.0, 100.0, &[]);
    assert!(l.is_rtl());
    assert!(l.caret(0).x > 150.0, "{:?}", l.caret(0));
    assert_eq!(l.lines()[0].start, 0, "the invisible direction mark is not in the offsets");
    assert_eq!(l.lines()[0].end, 3);
}

#[test]
fn hidden_marks_take_no_room_and_shown_ones_do() {
    let Some(mut e) = engine() else { return };
    let text = "**bold**";
    let plain = layout(&mut e, "bold", 400.0).measure().width;
    let mut hide = vec![Style::new(0..2), Style::new(6..8)];
    hide.iter_mut().for_each(|s| s.size = Some(0.01));
    let hidden = Layout::new(&mut e, text, &Format::new(Role::Text, 16.0), 400.0, 100.0, &hide).measure().width;
    let shown = layout(&mut e, text, 400.0).measure().width;
    assert!((hidden - plain).abs() < 0.5, "hidden {hidden} vs plain {plain}");
    assert!(shown > plain + 10.0);
    // A caret before the word sits where the word is, marks or not.
    let l = Layout::new(&mut e, text, &Format::new(Role::Text, 16.0), 400.0, 100.0, &hide);
    assert!(l.caret(2).x < 0.5);
}

#[test]
fn bold_is_wider_than_regular_and_styles_cover_only_their_range() {
    let Some(mut e) = engine() else { return };
    let f = Format::new(Role::Text, 16.0);
    let regular = Layout::new(&mut e, "Hello world", &f, 400.0, 100.0, &[]);
    let mut b = Style::new(0..5);
    b.weight = Some(700);
    let bold = Layout::new(&mut e, "Hello world", &f, 400.0, 100.0, &[b]);
    assert!(bold.caret(5).x > regular.caret(5).x + 0.5, "{} vs {}", bold.caret(5).x, regular.caret(5).x);
    let tail = |l: &Layout| l.caret(11).x - l.caret(5).x;
    assert!((tail(&bold) - tail(&regular)).abs() < 0.5, "the rest is not bold");
}

#[test]
fn centred_text_sits_in_the_middle() {
    let Some(mut e) = engine() else { return };
    let mut f = Format::new(Role::Text, 16.0);
    f.align = Align::Center;
    let l = Layout::new(&mut e, "mid", &f, 200.0, 100.0, &[]);
    let w = l.measure().width;
    assert!((l.caret(0).x - (200.0 - w) / 2.0).abs() < 1.0, "{:?} width {w}", l.caret(0));
}

#[test]
fn vertical_centring_puts_the_line_in_the_middle_of_the_height() {
    let Some(mut e) = engine() else { return };
    let mut f = Format::new(Role::Text, 16.0);
    f.center_y = true;
    let l = Layout::new(&mut e, "mid", &f, 200.0, 60.0, &[]);
    assert_eq!(l.lines()[0].top, 20.0);
    assert_eq!(l.caret(0).y, 20.0);
}

#[test]
fn a_long_label_is_cut_to_its_width_without_wrapping() {
    let Some(mut e) = engine() else { return };
    let mut f = Format::new(Role::Text, 13.0);
    f.wrap = false;
    f.ellipsis = true;
    let l = Layout::new(&mut e, "A very long note title that cannot possibly fit in a narrow side menu", &f, 100.0, 20.0, &[]);
    assert_eq!(l.measure().lines, 1);
    assert!(l.measure().width <= 100.5, "{:?}", l.measure());
}

#[test]
fn a_range_is_a_rectangle_per_line_it_covers() {
    let Some(mut e) = engine() else { return };
    let text = "one two three four five six seven eight nine ten";
    let l = layout(&mut e, text, 120.0);
    let lines = l.lines();
    assert!(lines.len() >= 3);
    let rects = l.range(0, text.len());
    assert_eq!(rects.len(), lines.len());
    for (r, line) in rects.iter().zip(&lines) {
        assert_eq!(r[1], line.top);
        assert!(r[2] > 0.0);
    }
    let word = l.range(4, 7);
    assert_eq!(word.len(), 1);
    assert!((word[0][0] - l.caret(4).x).abs() < 0.5);
    assert!(l.range(3, 3).is_empty());
}

#[test]
fn strikethrough_covers_the_struck_glyphs_only() {
    let Some(mut e) = engine() else { return };
    let mut s = Style::new(4..9);
    s.strike = true;
    let l = Layout::new(&mut e, "not struck out", &Format::new(Role::Text, 16.0), 400.0, 100.0, &[s]);
    let strikes = l.strikes();
    assert_eq!(strikes.len(), 1);
    let r = strikes[0].rect;
    assert!((r[0] - l.caret(4).x).abs() < 0.5 && (r[0] + r[2] - l.caret(9).x).abs() < 0.5, "{r:?}");
    assert!(r[1] > 0.0 && r[1] < 20.0);
}

#[test]
fn glyphs_carry_their_ink_and_draw_a_picture() {
    let Some(mut e) = engine() else { return };
    let mut s = Style::new(1..2);
    s.ink = 7;
    let l = Layout::new(&mut e, "abc", &Format::new(Role::Text, 16.0), 400.0, 100.0, &[s]);
    let glyphs = l.glyphs((10.0, 10.0), 1.0);
    assert_eq!(glyphs.iter().map(|g| g.ink).collect::<Vec<_>>(), vec![0, 7, 0]);
    let m = e.mask(&glyphs[0]).expect("a picture for 'a'");
    assert!(m.width > 3 && m.height > 3 && m.data.iter().any(|&a| a > 128));
    assert!(glyphs[1].x > glyphs[0].x && glyphs[0].y > 10);
}

#[test]
fn glyphs_scale_with_the_screen() {
    let Some(mut e) = engine() else { return };
    let l = layout(&mut e, "abc", 400.0);
    let one = l.glyphs((0.0, 0.0), 1.0);
    let two = l.glyphs((0.0, 0.0), 2.0);
    assert!(two[2].x > one[2].x * 3 / 2, "{} vs {}", two[2].x, one[2].x);
}

/// A Hebrew word inside English reads right to left within the line.
#[test]
fn hebrew_in_english_runs_right_to_left_with_its_caret_following() {
    let Some(mut e) = engine() else { return };
    let text = "abc \u{5E9}\u{5DC}\u{5D5}\u{5DD} def";
    let l = layout(&mut e, text, 400.0);
    assert!(!l.is_rtl());
    // The Hebrew letters start at byte 4, two bytes each.
    let (shin, mem) = (l.caret(4), l.caret(10));
    assert!(shin.x > mem.x, "first Hebrew letter is rightmost: {shin:?} vs {mem:?}");
    // Clicking on the left of the Hebrew word lands near its logical end.
    let (at, _) = l.offset_at(mem.x + 1.0, 10.0);
    assert!(at == 10 || at == 8, "{at}");
}

fn one_line(size: f32) -> Format {
    Format { wrap: false, ..Format::new(Role::Text, size) }
}

#[test]
fn hidden_marks_have_no_picture_and_visible_ones_do() {
    let Some(mut e) = engine() else { return };
    let mut hide = Style::new(0..2);
    hide.size = Some(0.01);
    let l = Layout::new(&mut e, "**bold", &Format::new(Role::Text, 16.0), 400.0, 100.0, &[hide]);
    let g = l.glyphs((0.0, 0.0), 1.0);
    assert!(g[0].size < 0.5 && g[1].size < 0.5 && g[2].size > 10.0);
    assert!(e.mask(&g[0]).is_none() && e.mask(&g[1]).is_none(), "a hidden mark draws nothing");
    assert!(e.mask(&g[2]).is_some());
}

#[test]
fn a_hidden_tab_takes_no_room() {
    let Some(mut e) = engine() else { return };
    let mut hide = Style::new(0..1);
    hide.size = Some(0.01);
    let l = Layout::new(&mut e, "\titem", &Format::new(Role::Text, 16.0), 400.0, 100.0, &[hide]);
    assert!(l.caret(1).x < 0.5, "{:?}", l.caret(1));
    let shown = layout(&mut e, "\titem", 400.0);
    assert!(shown.caret(1).x > 8.0, "a tab shown does take room");
}

#[test]
fn one_line_text_loses_the_whole_glyphs_that_pass_its_width() {
    let Some(mut e) = engine() else { return };
    let text = "A label too long for the box it sits in";
    let l = Layout::new(&mut e, text, &one_line(13.0), 60.0, 20.0, &[]);
    assert_eq!(l.measure().lines, 1);
    assert!(l.measure().width <= 60.0 + 0.01, "{:?}", l.measure());
    let n = l.glyphs((0.0, 0.0), 1.0).len();
    assert!(n > 3 && n < text.len(), "{n} of {}", text.len());
    let wide = Layout::new(&mut e, text, &one_line(13.0), 600.0, 20.0, &[]);
    assert_eq!(wide.glyphs((0.0, 0.0), 1.0).len(), text.chars().filter(|c| *c != ' ').count() + text.matches(' ').count());
}

#[test]
fn a_layout_can_be_made_again_in_another_width() {
    let Some(mut e) = engine() else { return };
    let l = layout(&mut e, "one two three four five six seven eight nine ten", 400.0);
    let narrow = l.with_width(&mut e, 90.0);
    assert_eq!(l.width(), 400.0);
    assert_eq!(narrow.width(), 90.0);
    assert!(narrow.measure().lines > l.measure().lines);
    assert_eq!(narrow.lines().last().unwrap().end, 48);
}

#[test]
fn a_forced_direction_beats_the_letters() {
    let Some(mut e) = engine() else { return };
    let arabic = "\u{645}\u{631}\u{62D}\u{628}\u{627}";
    let auto = layout(&mut e, arabic, 300.0);
    if !auto.is_rtl() {
        eprintln!("no Arabic in the fonts here: test skipped");
        return;
    }
    let ltr = Layout::new(&mut e, arabic, &Format { rtl: Some(false), ..Format::new(Role::Text, 16.0) }, 300.0, 50.0, &[]);
    assert!(!ltr.is_rtl());
    assert!(ltr.caret(0).x < 150.0 && auto.caret(0).x > 150.0, "{:?} vs {:?}", ltr.caret(0), auto.caret(0));
    assert_eq!(ltr.lines()[0].end, arabic.len(), "the direction mark is not in the offsets");
}

#[test]
fn a_click_beside_a_line_lands_at_its_near_end_whichever_way_it_reads() {
    let Some(mut e) = engine() else { return };
    // Arabic, set right to left at the right: the space left of it is past
    // its end, the space right of it before its start.
    let arabic = "\u{645}\u{631}\u{62D}\u{628}\u{627} \u{628}\u{643}";
    let l = layout(&mut e, arabic, 400.0);
    if !l.is_rtl() {
        eprintln!("no Arabic in the fonts here: test skipped");
        return;
    }
    let mid = l.caret(0).y + l.caret(0).height / 2.0;
    assert_eq!(l.offset_at(5.0, mid).0, arabic.len(), "left of the line: its end");
    assert_eq!(l.offset_at(399.0, mid).0, 0, "right of the line: its start");
    // English, set left to right: the same, mirrored.
    let english = "hello there";
    let l = layout(&mut e, english, 400.0);
    assert_eq!(l.offset_at(399.0, mid).0, english.len());
    assert_eq!(l.offset_at(-5.0, mid).0, 0);
}

#[test]
fn the_baseline_sits_where_the_format_puts_it() {
    let Some(mut e) = engine() else { return };
    let f = Format { line: Some(24.0), baseline: Some(0.8), ..Format::new(Role::Text, 16.0) };
    let l = Layout::new(&mut e, "Hxg\nlow", &f, 300.0, 100.0, &[]);
    let g = l.glyphs((0.0, 0.0), 1.0);
    assert!((g[0].y as f32 - 19.2).abs() <= 1.0, "first line baseline {}", g[0].y);
    let second = g.iter().find(|g| g.y > 30).expect("a second line");
    assert!((second.y as f32 - 43.2).abs() <= 1.0, "second line baseline {}", second.y);
    let plain = Layout::new(&mut e, "Hxg", &Format { line: Some(24.0), ..Format::new(Role::Text, 16.0) }, 300.0, 100.0, &[]);
    assert!(plain.glyphs((0.0, 0.0), 1.0)[0].y != g[0].y, "without a baseline it is centred on the font");
}

#[test]
fn the_same_glyph_has_the_same_key_in_any_layout() {
    let Some(mut e) = engine() else { return };
    let a = layout(&mut e, "abc", 400.0).glyphs((10.0, 5.0), 1.0);
    let b = layout(&mut e, "xabcx", 400.0).glyphs((10.0, 5.0), 1.0);
    let c = layout(&mut e, "abc", 400.0).glyphs((10.0, 5.0), 2.0);
    assert_eq!(a[0].key(), layout(&mut e, "abd", 400.0).glyphs((10.0, 5.0), 1.0)[0].key());
    assert_ne!(a[0].key(), b[0].key(), "a different glyph");
    assert_ne!(a[0].key(), c[0].key(), "another size");
    let mut set = std::collections::HashSet::new();
    set.insert(a[0].key());
    assert!(set.contains(&a[0].key()));
}

#[test]
fn a_glyph_picture_is_made_once_and_shared() {
    let Some(mut e) = engine() else { return };
    let g = layout(&mut e, "aa", 400.0).glyphs((0.0, 0.0), 1.0);
    let (m1, m2) = (e.mask(&g[0]).unwrap(), e.mask(&g[0]).unwrap());
    assert!(std::sync::Arc::ptr_eq(&m1, &m2));
}

#[test]
fn trailing_spaces_count_in_the_width() {
    let Some(mut e) = engine() else { return };
    let f = one_line(16.0);
    let bare = Layout::new(&mut e, "ab", &f, 400.0, 20.0, &[]).measure().width;
    let spaced = Layout::new(&mut e, "ab   ", &f, 400.0, 20.0, &[]).measure().width;
    assert!(spaced > bare + 10.0, "{spaced} vs {bare}");
}

#[test]
fn one_line_text_can_be_centred_or_set_at_the_end() {
    let Some(mut e) = engine() else { return };
    let w = Layout::new(&mut e, "mid", &one_line(16.0), 400.0, 40.0, &[]).measure().width;
    let centred = Layout::new(&mut e, "mid", &Format { align: Align::Center, center_y: true, ..one_line(16.0) }, 200.0, 40.0, &[]);
    assert!((centred.caret(0).x - (200.0 - w) / 2.0).abs() < 1.0, "{:?}", centred.caret(0));
    assert_eq!(centred.caret(0).y, 10.0, "centred in the height too");
    let end = Layout::new(&mut e, "mid", &Format { align: Align::End, ..one_line(16.0) }, 200.0, 40.0, &[]);
    assert!((end.caret(3).x - 200.0).abs() < 1.0, "{:?}", end.caret(3));
}

#[test]
fn a_cut_label_stays_centred_in_its_height() {
    let Some(mut e) = engine() else { return };
    let f = Format { ellipsis: true, center_y: true, ..one_line(13.0) };
    let l = Layout::new(&mut e, "Something far too long to fit in this narrow box at all", &f, 80.0, 40.0, &[]);
    assert!(l.measure().width <= 80.5);
    assert_eq!(l.lines()[0].top, (40.0 - 16.25) / 2.0);
}

#[test]
fn a_family_change_is_counted_and_reaches_new_layouts() {
    let Some(mut e) = engine() else { return };
    // Whatever the machine's body face (Arial, DejaVu Sans), another name is
    // a change and the same name again is not.
    let before = e.generation();
    e.set_family(Role::Text, "Courier New");
    assert_eq!(e.generation(), before + 1);
    e.set_family(Role::Text, "Courier New");
    assert_eq!(e.generation(), before + 1, "the same name is no change");
}

#[test]
fn a_family_moved_to_that_launch_left_out_loads_from_the_fallback() {
    let Some(base) = engine() else { return };
    // The first file only now; the rest (a face the Font option moves to) later.
    let spare = base.set.files.iter().skip(1).cloned().collect::<Vec<_>>();
    if spare.is_empty() {
        return;
    }
    let mut e = Engine::new(FontSet { files: vec![base.set.files[0].clone()], fallback: spare, ..base.set.clone() });
    let before = e.faces();
    e.set_family(Role::Text, "No Such Family");
    assert!(e.faces() > before, "the fallback files load when the family asked for is not loaded");
    let loaded = e.faces();
    e.set_family(Role::Text, "Another Missing One");
    assert_eq!(e.faces(), loaded, "and only once");
}

#[test]
fn the_engine_can_move_between_threads() {
    fn send<T: Send>() {}
    send::<Engine>();
    send::<Layout>();
}

#[test]
fn fonts_for_other_scripts_load_when_text_first_needs_them() {
    assert!(!needs_more_fonts("Plain text, caf\u{E9}, na\u{EF}ve \u{2026} \u{2014} \u{E713}"));
    assert!(needs_more_fonts("\u{645}\u{631}\u{62D}\u{628}\u{627}"));
    assert!(needs_more_fonts("\u{1F389}"));
    assert!(needs_more_fonts("\u{2713}"));
    let Some(base) = engine() else { return };
    let spare = base.set.files.iter().skip(1).cloned().collect::<Vec<_>>();
    if spare.is_empty() {
        return;
    }
    let first = base.set.files[0].clone();
    let mut e = Engine::new(FontSet { files: vec![first], fallback: spare, ..base.set.clone() });
    let before = e.faces();
    layout(&mut e, "English only", 300.0);
    assert_eq!(e.faces(), before, "nothing more is loaded for Latin text");
    layout(&mut e, "\u{65E5}\u{672C}", 300.0);
    assert!(e.faces() > before, "the rest loads when a script needs it");
    let loaded = e.faces();
    layout(&mut e, "\u{65E5}\u{672C}", 300.0);
    assert_eq!(e.faces(), loaded, "and only once");
}

/// The icon font the tools build (tools/iconfont) and the map it is built from.
const ICON_FONT: &[u8] = include_bytes!("../../../assets/fonts/KoladaIcons.ttf");
const ICON_MAP: &str = include_str!("../../../assets/icons/map.tsv");

fn icons_only() -> Engine {
    Engine::new(FontSet { data: vec![ICON_FONT], ui: "Kolada Icons".into(), text: "Kolada Icons".into(), code: "Kolada Icons".into(), icon: "Kolada Icons".into(), ..FontSet::default() })
}

#[test]
fn the_icon_font_loads_and_draws_every_icon_it_maps() {
    let mut e = icons_only();
    assert!(e.families().iter().any(|f| f == "Kolada Icons"), "{:?}", e.families());
    let mut drawn = 0;
    for line in ICON_MAP.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let code = u32::from_str_radix(line.split('\t').next().unwrap(), 16).unwrap();
        let ch = char::from_u32(code).unwrap().to_string();
        let l = Layout::new(&mut e, &ch, &Format::new(Role::Icon, 15.0), 40.0, 40.0, &[]);
        let glyphs = l.glyphs((0.0, 0.0), 1.0);
        assert_eq!(glyphs.len(), 1, "{line}: one glyph");
        let m = e.mask(&glyphs[0]).unwrap_or_else(|| panic!("{line}: no picture"));
        assert!(m.width >= 5 && m.height >= 3, "{line}: {}x{}", m.width, m.height);
        assert!(m.data.iter().any(|&a| a > 120), "{line}: nothing drawn");
        drawn += 1;
    }
    assert!(drawn >= 60, "{drawn} icons");
}

#[test]
fn an_icon_is_one_em_wide_and_sits_inside_its_line() {
    let mut e = icons_only();
    let f = Format { line: Some(18.75), baseline: Some(0.8), ..Format::new(Role::Icon, 15.0) };
    let l = Layout::new(&mut e, "\u{E713}", &f, 40.0, 40.0, &[]);
    assert!((l.measure().width - 15.0).abs() < 0.1, "{:?}", l.measure());
    let g = l.glyphs((0.0, 0.0), 1.0);
    let m = e.mask(&g[0]).unwrap();
    // Above the baseline by at most the ascent, below it by at most the descent.
    assert!(m.top <= 12 && (m.height as i32 - m.top) <= 3, "top {} height {}", m.top, m.height);
    assert!(g[0].y > 0 && g[0].y < 19, "baseline {}", g[0].y);
}

#[test]
fn a_character_asked_for_as_emoji_is_drawn_in_the_colour_emoji_face() {
    // Seen on the Mac: a heart picked from the emoji panel (U+2764 U+FE0F)
    // drew as Apple Symbols' black-and-white heart.
    assert_eq!(emoji_ranges("a\u{2764}\u{FE0F}b"), vec![1..7]);
    assert_eq!(emoji_ranges("1\u{FE0F}\u{20E3}"), vec![0..7], "a keycap");
    assert_eq!(emoji_ranges("\u{FE0F}plain"), vec![], "a selector with nothing before it");
    let emoji = std::path::Path::new("/System/Library/Fonts/Apple Color Emoji.ttc");
    let symbols = std::path::Path::new("/System/Library/Fonts/Apple Symbols.ttf");
    if !emoji.is_file() || !symbols.is_file() {
        eprintln!("no Apple emoji and symbol fonts here: the face check is skipped");
        return;
    }
    let Some(base) = engine() else { return };
    let mut e = Engine::new(FontSet { fallback: vec![symbols.to_path_buf(), emoji.to_path_buf()], ..base.set.clone() });
    let face_of = |e: &mut Engine, text: &str| -> Vec<String> {
        let l = layout(e, text, 400.0);
        let mut out = Vec::new();
        for run in l.buf.layout_runs() {
            for g in run.glyphs.iter() {
                if text[g.start..g.end].starts_with('\u{2764}') {
                    let face = e.fs.db().face(g.font_id).map(|f| f.families[0].0.clone()).unwrap_or_default();
                    out.push(face);
                }
            }
        }
        out
    };
    let colour = |faces: Vec<String>| !faces.is_empty() && faces.iter().all(|f| f.contains("Apple Color Emoji"));
    assert!(colour(face_of(&mut e, "I \u{2764}\u{FE0F} it")), "the colour heart");
}

#[test]
fn an_emoji_is_drawn_in_colour_even_where_the_text_face_has_a_plain_one() {
    // Seen on Linux: DejaVu Sans has its own black-and-white smileys, so a
    // 😀 typed into a note drew in it and never reached Noto Color Emoji.
    // A character that is an emoji by default goes to the colour face; one
    // asked for as text (U+FE0E after it) stays in the text face.
    assert_eq!(emoji_ranges("a\u{1F600}b"), vec![1..5]);
    assert_eq!(emoji_ranges("\u{1F600}\u{1F600}"), vec![0..8], "side by side, one run");
    assert_eq!(emoji_ranges("\u{2B50}\u{FE0E}"), vec![], "asked for as text");
    assert_eq!(emoji_ranges("\u{1F44D}\u{1F3FD}!"), vec![0..8], "with its skin tone");
    assert_eq!(emoji_ranges("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"), vec![0..18], "a family, joined");
    assert_eq!(emoji_ranges("\u{1F1F8}\u{1F1EA}"), vec![0..8], "a flag");
    assert_eq!(emoji_ranges("plain \u{2713} \u{2192} \u{00E9}"), vec![], "symbols that are text by default");
    // A text face with a plain 😀, and a colour emoji face behind it.
    let pairs = [
        (r"C:\Windows\Fonts\seguisym.ttf", "Segoe UI Symbol", r"C:\Windows\Fonts\seguiemj.ttf"),
        ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "DejaVu Sans", "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf"),
    ];
    let Some((text, family, emoji)) = pairs.into_iter().find(|(t, _, e)| std::path::Path::new(t).is_file() && std::path::Path::new(e).is_file()) else {
        eprintln!("no text face with plain emoji and colour emoji face here: the face check is skipped");
        return;
    };
    let mut e = Engine::new(FontSet {
        files: vec![text.into()], fallback: vec![emoji.into()], data: Vec::new(),
        ui: family.into(), text: family.into(), code: family.into(), icon: family.into(),
    });
    let l = layout(&mut e, "Hi \u{1F600}", 400.0);
    let g = l.glyphs((0.0, 0.0), 2.0);
    let m = e.mask(g.last().unwrap()).expect("a picture for the emoji");
    assert!(m.color, "the 😀 is drawn from the colour face, not {family}'s plain one");
}
