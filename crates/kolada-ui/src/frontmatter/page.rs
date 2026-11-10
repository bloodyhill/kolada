//! The frontmatter's line on the page: folded at rest, opened by the caret,
//! its host a link, cut to the column, set where the title is.

use super::*;
use crate::geom::Ink;
use crate::note::{FrontView, NoteText, title_line};
use crate::scene::{self, Eng, Fonts, Input, Target};
use kolada_editor::{Editor, Motion as Move, Selection};

pub const CAUGHT_NOTE: &str = "---\nsource: https://kitchen.example/lemon-tart\ncaught: 2026-11-08T15:20\n---\n# Lemon tart with a crisp base\n\nZest two lemons into the sugar.\n";
pub const TAGS_NOTE: &str = "---\ntags:\n  - recipe\n  - lemon\naliases: [Lemon tart]\n---\n# Lemon tart\n\nBake it blind first.\n";
pub const ARABIC_NOTE: &str = "---\nsource: https://kitchen.example/lemon-tart\ncaught: 2026-11-08T15:20\n---\n# \u{062A}\u{0627}\u{0631}\u{062A} \u{0627}\u{0644}\u{0644}\u{064A}\u{0645}\u{0648}\u{0646}\n\n\u{0627}\u{0628}\u{0634}\u{0631}\u{064A} \u{0644}\u{064A}\u{0645}\u{0648}\u{0646}\u{062A}\u{064A}\u{0646}.\n";
pub const TODAY: Day = Day { year: 2026, month: 11, day: 9 };

fn with(text: &str, width: f32, caret: usize, clean: bool, open: Option<f32>, f: impl FnOnce(&NoteText)) {
    let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
    let fonts = Fonts::new(1.0, 0);
    let lines: Vec<Line> = md::lines(text).collect();
    let title = title_line(text, &lines);
    f(&NoteText { eng: &eng, fonts: &fonts, text, lines: &lines, width, title, caret: Some(caret), clean, pictures: None, tables: Default::default(), folded: &[], front: Front::new(open, 2026) });
}

pub fn title_at(text: &str) -> usize {
    text.find("\n# ").unwrap() + 1
}

#[test]
fn at_rest_the_block_is_one_quiet_line() {
    let title = title_at(CAUGHT_NOTE);
    with(CAUGHT_NOTE, 700.0, title + 4, true, None, |nt| {
        let first = nt.para(0).unwrap();
        let Some(FrontView::Line(line)) = &first.front else { panic!("one line at rest") };
        assert_eq!(line.layout.text(), "kitchen.example  \u{00B7}  caught 8 Nov");
        assert_eq!(line.ink, Ink::Muted, "the muted ink: not faint, not the text's");
        assert_eq!(line.layout.lines().len(), 1);
        assert!((first.height() - line.height).abs() < 0.01 && line.height > 10.0);
        for i in 1..=3 {
            let p = nt.para(i).unwrap();
            assert!(matches!(p.front, Some(FrontView::Folded)) && p.height() == 0.0, "line {i} folds away");
        }
        assert!(nt.para(4).unwrap().front.is_none(), "the title as it was");
    });
    with(TAGS_NOTE, 700.0, title_at(TAGS_NOTE) + 3, true, None, |nt| {
        let Some(FrontView::Line(line)) = &nt.para(0).unwrap().front else { panic!() };
        assert_eq!(line.layout.text(), "#recipe #lemon \u{00B7} Lemon tart");
        assert!(line.link.is_none() && line.url.is_none());
    });
    // Only keys: faint.
    let keys = "---\npublish: true\n---\n# Draft\n";
    with(keys, 700.0, keys.len(), true, None, |nt| {
        let Some(FrontView::Line(line)) = &nt.para(0).unwrap().front else { panic!() };
        assert_eq!((line.layout.text(), line.ink), ("publish", Ink::Faint));
    });
}

#[test]
fn the_caret_opens_it_and_leaving_folds_it() {
    let text = CAUGHT_NOTE;
    let title = title_at(text);
    let lines: Vec<Line> = md::lines(text).collect();
    let written = |nt: &NoteText| (0..4).map(|i| nt.para(i).unwrap()).all(|p| p.front.is_none() && p.height() > 10.0);
    // Up from the title lands on the line, and it opens.
    let mut e = Editor::new(text);
    e.set_selection(Selection::caret(title + 4));
    with(text, 700.0, title + 4, true, None, |nt| e.move_caret(Move::Up, false, nt));
    let at = e.selection().head;
    assert!(caret_in(&lines, at), "up lands in the frontmatter: {at}");
    with(text, 700.0, at, true, None, |nt| assert!(written(nt), "its lines as written"));
    // Down through them and off: folded again.
    for _ in 0..4 {
        let head = e.selection().head;
        with(text, 700.0, head, true, None, |nt| e.move_caret(Move::Down, false, nt));
    }
    let off = e.selection().head;
    assert!(off >= title, "{off}");
    with(text, 700.0, off, true, None, |nt| assert!(matches!(nt.para(0).unwrap().front, Some(FrontView::Line(_)))));
    // Half open: each line as tall as it is open; the block between its one
    // line and its lines as written, so the text below moves without a jump.
    let height = |open: Option<f32>, caret: usize| {
        let mut h = 0.0;
        with(text, 700.0, caret, true, open, |nt| h = (0..4).map(|i| nt.para(i).unwrap().height()).sum());
        h
    };
    let (shut, half, full) = (height(None, title), height(Some(0.5), title), height(None, 0));
    assert!(shut < half && half < full, "{shut} {half} {full}");
    assert!((half - (shut + full) / 2.0).abs() < 0.5, "halfway: {half}");
    assert!((height(Some(0.0), 0) - shut).abs() < 0.01 && (height(Some(1.0), title) - full).abs() < 0.01);
    with(text, 700.0, title, true, Some(0.5), |nt| assert!(matches!(nt.para(0).unwrap().front, Some(FrontView::Moving { .. }))));
    // Show Markdown on: always as written.
    with(text, 700.0, title + 4, false, None, |nt| assert!(written(nt)));
}

/// At rest, what draws, scrolls and moves goes past the folded lines at
/// once, not one by one: a long frontmatter costs a frame no more than a
/// short one (`time_a_keystroke_under_a_long_frontmatter`).
#[test]
fn the_folded_lines_are_passed_at_once() {
    let text = CAUGHT_NOTE;
    let title = title_at(text);
    with(text, 700.0, title + 4, true, None, |nt| {
        assert_eq!(nt.front_last(), Some(3));
        assert_eq!((nt.past_front(0), nt.past_front(1), nt.past_front(3), nt.past_front(4)), (None, Some(4), Some(4), None));
        let first = nt.para(0).unwrap();
        assert_eq!(nt.after(&first), title, "after the one line, the title");
        let before = nt.before(title).unwrap();
        assert!(before.start == 0 && matches!(before.front, Some(FrontView::Line(_))), "before the title, the one line");
    });
    // Open, line by line as written.
    with(text, 700.0, 0, true, None, |nt| {
        assert_eq!(nt.past_front(1), None);
        let first = nt.para(0).unwrap();
        assert_eq!(nt.after(&first), first.end + 1);
        assert_eq!(nt.before(title).unwrap().start, text[..title - 1].rfind('\n').unwrap() + 1);
    });
}

#[test]
fn a_long_line_is_cut_at_a_word() {
    let yaml = "tags: [recipe, lemon, baking, dessert, summer, citrus, pastry]\naliases: [Lemon tart with a crisp base and a soft middle]";
    let text = format!("---\n{yaml}\n---\n# Tart\n");
    let full = shown(yaml, 2026, false).unwrap().text;
    for width in [240.0, 320.0, 420.0] {
        with(&text, width, text.len(), true, None, |nt| {
            let Some(FrontView::Line(line)) = &nt.para(0).unwrap().front else { panic!() };
            let cut = line.layout.text();
            let kept = cut.strip_suffix('\u{2026}').unwrap_or_else(|| panic!("cut with an ellipsis at {width}: {cut}"));
            assert!(full.starts_with(kept) && full[kept.len()..].starts_with(' '), "at a word: {cut:?}");
            assert!(!kept.ends_with([' ', '\u{00B7}']));
            assert_eq!(line.layout.lines().len(), 1, "one line at {width}");
            assert!(line.layout.measure().width <= width + 0.5, "{} in {width}", line.layout.measure().width);
        });
    }
}

pub fn input<'a>(text: &'a str, lines: &'a [Line], caret: usize) -> Input<'a> {
    Input { text, lines, selection: (caret, caret), page_fade: 1.0, today: Some(TODAY), ..Default::default() }
}

#[test]
fn the_host_opens_the_page_and_the_rest_of_the_line_opens_the_lines() {
    let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
    let fonts = Fonts::new(1.0, 0);
    let text = CAUGHT_NOTE;
    let lines: Vec<Line> = md::lines(text).collect();
    let s = scene::build(&eng, &fonts, &input(text, &lines, text.len()), 1200.0, 800.0).unwrap();
    let link = s.hits.iter().find(|(_, t)| *t == Target::Source).map(|(r, _)| *r).expect("the host takes a click");
    let line = s.texts.iter().find(|t| t.layout.text().starts_with("kitchen")).unwrap();
    let host = line.layout.range(0, "kitchen.example".len());
    assert!((link.left - (line.x + host[0][0])).abs() < 3.0 && link.width() < 200.0, "{link:?}");
    assert_eq!(scene::hit(&s.hits, (link.left + link.right) / 2.0, (link.top + link.bottom) / 2.0), Some(Target::Source));
    // Past it, the line is the note's: a click there puts the caret in the
    // frontmatter.
    let y = (link.top + link.bottom) / 2.0;
    let past = link.right + 40.0;
    assert_eq!(scene::hit(&s.hits, past, y), Some(Target::Note));
    let drawn = s.lines.iter().find(|l| y < l.y + l.height).unwrap();
    let nt = NoteText { eng: &eng, fonts: &fonts, text, lines: &lines, width: s.column.width(), title: title_line(text, &lines), caret: None, clean: true, pictures: None, tables: Default::default(), folded: &[], front: Front::default() };
    let at = nt.offset_at(&drawn.para, past - drawn.x, y - drawn.y).unwrap();
    assert!(caret_in(&lines, at), "the click lands in the frontmatter: {at}");
    assert_eq!(source_url(text).as_deref(), Some("https://kitchen.example/lemon-tart"));
    // Opened, there is no link to click.
    let open = scene::build(&eng, &fonts, &input(text, &lines, 0), 1200.0, 800.0).unwrap();
    assert!(!open.hits.iter().any(|(_, t)| *t == Target::Source));
}

/// Whether pixel `p` is ink on `page`.
fn inked(p: u32, page: u32) -> bool {
    let d = |s: u32| ((p >> s) & 0xFF).abs_diff((page >> s) & 0xFF);
    d(0) + d(8) + d(16) > 60
}

/// The leftmost and rightmost ink in rows `top..bottom` of the column
/// `left..right` (DIPs, a little either side), drawn at `scale`, and the
/// first and last row with any.
#[allow(clippy::too_many_arguments)]
fn ink_box(px: &[u32], w: usize, scale: f32, (left, right): (f32, f32), top: f32, bottom: f32, page: u32) -> Option<(f32, f32, f32, f32)> {
    let (mut l, mut r, mut t, mut b) = (usize::MAX, 0, usize::MAX, 0);
    let (x0, x1) = (((left - 8.0) * scale).max(0.0) as usize, (((right + 8.0) * scale) as usize).min(w));
    for y in (top * scale) as usize..(bottom * scale) as usize {
        for x in x0..x1 {
            if inked(px[y * w + x], page) {
                l = l.min(x);
                r = r.max(x + 1);
                t = t.min(y);
                b = b.max(y + 1);
            }
        }
    }
    (l < r).then(|| (l as f32 / scale, r as f32 / scale, t as f32 / scale, b as f32 / scale))
}

/// The line's ink starts where the title's does (ends there, right to
/// left), and sits in the middle of its slot.
#[test]
fn the_lines_ink_lines_up_with_the_title() {
    let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
    let fonts = Fonts::new(1.0, 0);
    for (text, rtl) in [(CAUGHT_NOTE, false), (ARABIC_NOTE, true)] {
        let lines: Vec<Line> = md::lines(text).collect();
        // The caret after the title's last letter: on it, not at its edge.
        let caret = text[title_at(text)..].find('\n').unwrap() + title_at(text);
        let scale = 2.0;
        let (w, h) = (1000.0, 600.0);
        // With the note's dates above it.
        let dated = Input { page_date: Some("Created 8 Nov \u{00B7} 9 words"), ..input(text, &lines, caret) };
        let s = scene::build(&eng, &fonts, &dated, w, h).unwrap();
        let list = crate::display::record(&eng, &s, &crate::theme::LIGHT, scale).unwrap();
        let surface = crate::raster::Surface::draw_new(((w * scale) as u32, (h * scale) as u32, 192), &list);
        let (px, pw) = (surface.pixels(), (w * scale) as usize);
        let page = px[(10.0 * scale) as usize * pw + 2];
        let line = &s.lines[0];
        let title = s.lines.iter().find(|l| l.para.line.start == title_at(text)).unwrap();
        let Some(FrontView::Line(front)) = &line.para.front else { panic!("folded") };
        assert_eq!(front.rtl, rtl);
        let (fl, fr, ft, fb) = ink_box(px, pw, scale, (s.column.left, s.column.right), line.y, line.y + line.height, page).expect("the line's ink");
        let (tl, tr, _, _) = ink_box(px, pw, scale, (s.column.left, s.column.right), title.y, title.y + title.height, page).expect("the title's ink");
        if rtl {
            assert!((fr - tr).abs() <= 2.0, "right to left, the line ends where the title does: {fr} vs {tr}");
            assert!(fl > s.column.left + 100.0, "at the right: {fl}..{fr} in {:?}", s.column);
        } else {
            assert!((fl - tl).abs() <= 2.0, "the line starts where the title does: {fl} vs {tl}");
        }
        assert!(fr <= s.column.right + 0.5 && fl >= s.column.left - 0.5, "in the column: {fl}..{fr}");
        // The dates take the note's direction too: their ink ends at the
        // title's right edge right to left, starts at its left edge left to
        // right; the teal dot after them, on the side they read toward.
        let (ml, mr, _, _) = ink_box(px, pw, scale, (s.column.left, s.column.right), line.y - 24.0, line.y - 6.0, page).expect("the dates' ink");
        // Something due: the dot (drawn apart, so the dates' ink is theirs alone).
        let due = scene::build(&eng, &fonts, &Input { due_now: 1, ..dated }, w, h).unwrap();
        let dot = due.due_dot.expect("the dot");
        if rtl {
            assert!((mr - tr).abs() <= 2.0, "right to left, the dates end where the title does: {mr} vs {tr}");
            assert!(dot.right <= ml && ml - dot.right < 12.0, "the dot to their left: {dot:?}, ink from {ml}");
        } else {
            assert!((ml - tl).abs() <= 2.0, "the dates start where the title does: {ml} vs {tl}");
            assert!(dot.left >= mr && dot.left - mr < 12.0, "the dot to their right: {dot:?}, ink to {mr}");
        }
        let hit = s.hits.iter().find(|(_, t)| *t == Target::PageDate).map(|(r, _)| *r).expect("the dates take a click");
        assert!(hit.left <= ml + 1.0 && hit.right >= mr - 1.0, "their click covers them: {hit:?} for {ml}..{mr}");
        // Its letters in the middle of its slot: as far from the top of
        // their ink down to the middle as from there to their baseline
        // (within two DIPs); nothing clipped above or below.
        let (_, base) = front.layout.glyph_at(0).expect("its first letter");
        let mid = line.y + line.height / 2.0;
        let (rise, fall) = (mid - ft, line.y + base - mid);
        assert!((rise - fall).abs() <= 2.0, "centred in its slot: {rise} above the middle, {fall} below");
        assert!(ft > line.y + 1.0 && fb < line.y + line.height - 1.0, "inside its slot: {ft}..{fb} in {}..{}", line.y, line.y + line.height);
    }
}

/// A view whose top falls in the folded block (or on the title just under
/// it, above its start) is the one line's slot: the line, and the dates
/// over it, are drawn, the title right under them, no gap.
#[test]
fn a_top_inside_the_folded_block_still_shows_the_line_and_the_dates() {
    let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
    let fonts = Fonts::new(1.0, 0);
    let text = CAUGHT_NOTE;
    let lines: Vec<Line> = md::lines(text).collect();
    let title = title_at(text);
    for line in [lines[1].start, lines[2].start, lines[3].start, title] {
        for px in [-80.0, -20.0, 0.0, 6.0] {
            let at = format!("top at byte {line}, {px} below");
            let dated = Input { scroll_line: line, scroll_px: px, page_date: Some("Created 8 Nov \u{00B7} 9 words"), ..input(text, &lines, title + 4) };
            let s = scene::build(&eng, &fonts, &dated, 1000.0, 600.0).unwrap();
            let one = s.texts.iter().find(|t| t.layout.text().starts_with("kitchen")).unwrap_or_else(|| panic!("the one line, {at}"));
            let named = s.texts.iter().find(|t| t.layout.text().starts_with("# Lemon")).unwrap_or_else(|| panic!("the title, {at}"));
            let slot = s.lines[0].height;
            assert!(s.lines[0].para.start == 0 && matches!(s.lines[0].para.front, Some(FrontView::Line(_))), "drawn from the one line, {at}");
            assert!((named.y - (one.y + slot)).abs() < 0.5, "the title right under it, {at}");
            assert!(s.texts.iter().any(|t| t.layout.text().starts_with("Created")), "the dates over it, {at}");
            // The folded lines take no room: `px` below them is below the
            // slot's foot; never above the note's top (no gap over it).
            let into = (px + slot).max(0.0);
            assert!((one.y - (s.column.top - into)).abs() < 0.5, "the line {into} into the view's top: at {} for {}, {at}", one.y, s.column.top);
            if px <= -slot {
                assert!((one.y - s.column.top).abs() < 0.5, "at the top of the note, no gap above, {at}");
            }
        }
    }
}

/// A caught note opened from search on a match in its title, the view put
/// on the match as the shells put it (`found::view_at`, settled as they
/// settle it): the one line's ink above the title.
#[test]
fn a_caught_note_found_by_its_title_draws_its_line_above_the_title() {
    let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
    let fonts = Fonts::new(1.0, 0);
    let text = CAUGHT_NOTE;
    let lines: Vec<Line> = md::lines(text).collect();
    let (w, scale) = (1000.0, 2.0);
    let probe = scene::build(&eng, &fonts, &input(text, &lines, 0), w, 900.0).unwrap();
    let top = probe.column.top;
    // A tall window, and one just tall enough that the title at OPEN_AT
    // would leave the view's top a little into the note's head.
    for h in [900.0, ((top + 10.0) / crate::found::OPEN_AT).round()] {
        for found in [title_at(text) + 2, title_at(text) + 22] {
            let nt = NoteText { eng: &eng, fonts: &fonts, text, lines: &lines, width: probe.column.width(), title: title_line(text, &lines), caret: Some(found), clean: true, pictures: None, tables: Default::default(), folded: &[], front: Front::default() };
            let (line, px) = crate::found::view_at(&nt, found, top, h).expect("a view");
            assert_eq!((line, px), (0, 0.0), "a match in the title opens the note at its top, {h} tall");
            let s = scene::build(&eng, &fonts, &Input { scroll_line: line, scroll_px: px, page_date: Some("Created 8 Nov \u{00B7} 9 words"), ..input(text, &lines, found) }, w, h).unwrap();
            let named = s.lines.iter().find(|l| l.para.line.start == title_at(text)).expect("the title drawn");
            let one = &s.lines[0];
            assert!(matches!(one.para.front, Some(FrontView::Line(_))) && one.y + one.height <= named.y + 0.5, "the one line above the title");
            // Both whole, at full strength: no fade over the column's top.
            for (what, starts) in [("the one line", "kitchen"), ("the dates", "Created")] {
                let t = s.texts.iter().find(|t| t.layout.text().starts_with(starts)).unwrap_or_else(|| panic!("{what} drawn"));
                assert_eq!(t.alpha, 1.0, "{what} at full strength");
                assert!(t.y >= 0.0 && t.y + t.layout.measure().height <= named.y + 0.5, "{what} in the window, above the title");
            }
            assert!(!s.fills.iter().any(|f| f.ink == crate::geom::Ink::Page && f.blur > 0.0 && f.rect.bottom > one.y - 30.0 && f.rect.top < named.y), "no fade over them, {h} tall");
            let list = crate::display::record(&eng, &s, &crate::theme::DARK, scale).unwrap();
            let surface = crate::raster::Surface::draw_new(((w * scale) as u32, (h * scale) as u32, 192), &list);
            let (pxs, pw) = (surface.pixels(), (w * scale) as usize);
            let page = pxs[((h - 4.0) * scale) as usize * pw + 4];
            assert!(ink_box(pxs, pw, scale, (s.column.left, s.column.right), named.y - one.height, named.y, page).is_some(), "its ink above the title, {h} tall");
            assert!(ink_box(pxs, pw, scale, (s.column.left, s.column.right), named.y - one.height - 24.0, named.y - one.height - 6.0, page).is_some(), "the dates' ink above it, {h} tall");
        }
    }
}

/// Renders for review: a caught note and a tags note, wide and snapped,
/// graphite and paper, folded and open; and an Arabic note folded. Made-up
/// notes only.
#[test]
#[ignore = "writes the frontmatter's line for manual inspection (KOLADA_DUMP_DIR)"]
fn dump_front() {
    let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
    let fonts = Fonts::new(1.0, 0);
    let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
    let mut shots = Vec::new();
    for (name, text) in [("caught", CAUGHT_NOTE), ("tags", TAGS_NOTE)] {
        for (size, w, h) in [("wide", 1400.0, 900.0), ("snap", 640.0, 1000.0)] {
            for (theme, t) in [("graphite", &crate::theme::DARK), ("paper", &crate::theme::LIGHT)] {
                for (state, caret) in [("folded", text.len()), ("revealed", 0)] {
                    shots.push((format!("{name}-{size}-{theme}-{state}"), text, w, h, t, caret));
                }
            }
        }
    }
    shots.push(("arabic-wide-paper-folded".into(), ARABIC_NOTE, 1400.0, 900.0, &crate::theme::LIGHT, ARABIC_NOTE.len()));
    shots.push(("arabic-wide-graphite-folded".into(), ARABIC_NOTE, 1400.0, 900.0, &crate::theme::DARK, ARABIC_NOTE.len()));
    for (name, text, w, h, theme, caret) in shots {
        let lines: Vec<Line> = md::lines(text).collect();
        let side = if w < 1000.0 { 0.0 } else { 1.0 };
        let input = Input { sidebar_dock: side, sidebar_show: side, toolbar_dock: 1.0, toolbar_show: 1.0, vault_name: "notes", active_note: "Lemon tart", page_date: Some("Created 8 Nov \u{00B7} 9 words"), ..input(text, &lines, caret) };
        let s = scene::build(&eng, &fonts, &input, w, h).unwrap();
        let list = crate::display::record(&eng, &s, theme, 1.5).unwrap();
        crate::raster::Surface::draw_new(((w * 1.5) as u32, (h * 1.5) as u32, 144), &list).dump(&dir.join(format!("{name}.bmp"))).unwrap();
    }
}
