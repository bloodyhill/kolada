use super::*;
use crate::quick::{DeviceRow, Look, Pairing};
use crate::scene::{DevicesView, Fonts, Text};

fn eng() -> Eng {
    Eng::new(kolada_text::Engine::new(crate::testing::fonts()))
}

fn rect_of(scene: &Scene, t: Target) -> Option<Rect> {
    scene.hits.iter().rev().find(|(_, x)| *x == t).map(|(r, _)| *r)
}

fn text_of<'a>(scene: &'a Scene, words: &str) -> Option<&'a Text> {
    scene.texts.iter().find(|t| t.layout.text() == words)
}

const DEVICES: [DeviceRow; 2] = [DeviceRow { name: "Desk PC", look: Look::Desktop, this: true }, DeviceRow { name: "\u{647}\u{627}\u{62A}\u{641} \u{633}\u{627}\u{631}\u{629}", look: Look::Phone, this: false }];

/// The screen as a shell builds it: switches as `options` say, devices
/// under Sync and Beam.
fn build(eng: &Eng, w: &Welcome, platform: Platform, options: [u8; scene::SETTINGS], folding: Option<(Setting, f32)>, width: f32, height: f32) -> Scene {
    let fonts = Fonts::new(1.0, 0);
    let input = Input {
        vault_name: "notes",
        platform,
        native_caption: platform == Platform::Mac,
        options,
        folding,
        devices: Some(DevicesView { devices: &DEVICES, pairing: Pairing::Idle, said: None, facts: &[], this: "PC", naming: None }),
        welcome: Some(View { welcome: w, model: Some(0) }),
        page_fade: 1.0,
        ..Default::default()
    };
    scene::build(eng, &fonts, &input, width, height).unwrap()
}

fn on() -> [u8; scene::SETTINGS] {
    [0; scene::SETTINGS]
}

fn off() -> [u8; scene::SETTINGS] {
    let mut o = [0; scene::SETTINGS];
    o[Setting::Assist.index()] = crate::modelcard::OFF;
    o[Setting::Beam.index()] = 1;
    o
}

#[test]
fn a_tile_for_each_place_notes_come_from_apple_notes_on_the_mac_only() {
    let eng = eng();
    let w = Welcome::new();
    for platform in [Platform::Windows, Platform::Mac, Platform::Linux, Platform::Ios] {
        let s = build(&eng, &w, platform, on(), None, 1200.0, 900.0);
        let tiles: Vec<From> = ALL.into_iter().filter(|&f| rect_of(&s, Target::Welcome(Hit::Tile(f))).is_some()).collect();
        let want: Vec<From> = ALL.into_iter().filter(|&f| f != From::AppleNotes || platform == Platform::Mac).collect();
        assert_eq!(tiles, want, "{platform:?}");
        for f in &tiles {
            assert!(text_of(&s, f.name()).is_some() && text_of(&s, f.hint()).is_some(), "{f:?} names itself and what to pick");
        }
    }
    // Eight sit two rows of four; nine, three of three.
    let s = build(&eng, &w, Platform::Windows, on(), None, 1200.0, 900.0);
    let top = |s: &Scene, f| rect_of(s, Target::Welcome(Hit::Tile(f))).unwrap().top;
    assert_eq!(top(&s, From::Obsidian), top(&s, From::Evernote));
    assert!(top(&s, From::Notion) > top(&s, From::Evernote));
    // Narrower, four rows of two: none left alone in a row.
    let snap = build(&eng, &w, Platform::Windows, on(), None, 640.0, 1000.0);
    assert_eq!(top(&snap, From::Obsidian), top(&snap, From::Keep));
    assert_eq!(top(&snap, From::Folder), top(&snap, From::Documents));
    assert!(top(&snap, From::Bear) > top(&snap, From::Keep));
}

#[test]
fn the_screen_says_what_it_is_and_how_to_start() {
    let eng = eng();
    let s = build(&eng, &Welcome::new(), Platform::Windows, on(), None, 1200.0, 1000.0);
    for words in ["Welcome to Kolada", "Three things, then it\u{2019}s yours.", "BRING YOUR NOTES", "WRITING HELP", "YOUR DEVICES", "Writing model", "Light", "Full", "Sync and Beam", "Pair a device", "Enter a code", "Start writing"] {
        assert!(text_of(&s, words).is_some(), "{words}");
    }
    assert_eq!(s.vectors.iter().take(3).map(|v| v.logo).collect::<Vec<_>>(), [Logo::Mark, Logo::MarkWings, Logo::MarkEye], "the logo as the user drew it");
    for t in [Target::Switch(Setting::Assist), Target::Choice(Setting::Assist, 0), Target::Choice(Setting::Assist, 1), Target::Switch(Setting::Beam), Target::Quick(crate::quick::Hit::ShowCode), Target::Quick(crate::quick::Hit::EnterCode), Target::Welcome(Hit::Start)] {
        assert!(rect_of(&s, t).is_some(), "{t:?}");
    }
    // The window's buttons stay over it, and its top row drags it.
    let close = rect_of(&s, Target::Close).unwrap();
    assert_eq!(scene::hit(&s.hits, (close.left + close.right) / 2.0, 10.0), Some(Target::Close));
    assert_eq!(scene::hit(&s.hits, 400.0, 10.0), Some(Target::Caption));
    assert_eq!(scene::hit(&s.hits, 20.0, 600.0), Some(Target::Welcome(Hit::Page)), "nothing under it takes a click");
}

#[test]
fn a_switch_off_folds_its_rows_away() {
    let eng = eng();
    let w = Welcome::new();
    let s = build(&eng, &w, Platform::Windows, off(), None, 1200.0, 1000.0);
    assert!(rect_of(&s, Target::Switch(Setting::Assist)).is_some() && rect_of(&s, Target::Switch(Setting::Beam)).is_some());
    for gone in [Target::Choice(Setting::Assist, 0), Target::Quick(crate::quick::Hit::ShowCode)] {
        assert!(rect_of(&s, gone).is_none(), "{gone:?}");
    }
    let (open, half) = (build(&eng, &w, Platform::Windows, on(), None, 1200.0, 1000.0), build(&eng, &w, Platform::Windows, off(), Some((Setting::Assist, 0.5)), 1200.0, 1000.0));
    let start = |s: &Scene| rect_of(s, Target::Welcome(Hit::Start)).unwrap();
    assert!(start(&s).top < start(&open).top, "the rows under it come up");
    // Half way: what is under the switch moves half the way.
    let light = rect_of(&half, Target::Choice(Setting::Assist, 0));
    let switch = rect_of(&half, Target::Switch(Setting::Assist)).unwrap();
    assert!(light.is_none_or(|r| r.top >= switch.bottom), "clipped under its switch: {light:?}");
}

#[test]
fn the_line_tells_how_bringing_in_goes() {
    let mut w = Welcome::new();
    assert_eq!(w.words(), "", "at rest the line says nothing: no line explains");
    assert_eq!(w.click(Hit::Tile(From::Keep)), Command::Pick(From::Keep));
    w.line = Line::Coming(From::Keep);
    assert_eq!(w.words(), "Google Keep is coming in a moment.");
    assert_eq!(w.click(Hit::Tile(From::Evernote)), Command::Pick(From::Evernote));
    w.began(From::Evernote, "Evernote");
    assert_eq!(w.words(), "Bringing in Evernote\u{2026}");
    assert_eq!(w.click(Hit::Tile(From::Notion)), Command::None, "one at a time");
    assert!(w.progress((214, 530)) && !w.progress((214, 530)));
    assert_eq!(w.words(), "Bringing in Evernote\u{2026} 214 of 530 notes");
    assert_eq!(w.click(Hit::Stop), Command::Stop);
    w.finished(Outcome::Brought { count: 530, folder: "From Evernote".into(), failed: 0 });
    assert_eq!(w.words(), "530 notes from Evernote are in the folder \u{2018}From Evernote\u{2019}");
    assert_eq!(w.brought, [From::Evernote], "its tile is ticked");
    assert_eq!(w.click(Hit::Stop), Command::None);
    w.began(From::Documents, "your documents");
    w.progress((1, 3));
    assert_eq!(w.words(), "Bringing in your documents\u{2026} 1 of 3 files");
    w.finished(Outcome::Brought { count: 1, folder: "From documents".into(), failed: 2 });
    assert_eq!(w.words(), "1 note from your documents is in the folder \u{2018}From documents\u{2019}; 2 files could not be read");
    w.began(From::Notion, "Notion");
    w.finished(Outcome::Stopped);
    assert_eq!(w.words(), "Stopped. Nothing from Notion was kept.");
    assert_eq!(w.brought, [From::Evernote, From::Documents], "a stop ticks nothing");
    w.began(From::NotesApp, "Notes app");
    w.finished(Outcome::Failed("it is not a Notes export".into()));
    assert_eq!(w.words(), "Could not bring in Notes app: it is not a Notes export");
    assert_eq!(thousands(1_234_567), "1,234,567");
    assert_eq!(w.click(Hit::Start), Command::Start);
}

#[test]
fn bringing_in_shows_its_bar_and_stop_and_a_done_tile_is_teal() {
    let eng = eng();
    let mut w = Welcome::new();
    w.began(From::Evernote, "Evernote");
    w.progress((214, 530));
    let s = build(&eng, &w, Platform::Windows, on(), None, 1200.0, 900.0);
    assert!(rect_of(&s, Target::Welcome(Hit::Stop)).is_some());
    assert!(text_of(&s, "Bringing in Evernote\u{2026} 214 of 530 notes").is_some());
    let bar = s.fills.iter().filter(|f| f.ink == Ink::Accent && f.rect.height() <= 2.5).count();
    assert_eq!(bar, 1, "a thin teal bar of how far");
    w.finished(Outcome::Brought { count: 530, folder: "From Evernote".into(), failed: 0 });
    let s = build(&eng, &w, Platform::Windows, on(), None, 1200.0, 900.0);
    assert!(rect_of(&s, Target::Welcome(Hit::Stop)).is_none());
    let tile = rect_of(&s, Target::Welcome(Hit::Tile(From::Evernote))).unwrap();
    assert!(s.fills.iter().any(|f| f.ink == Ink::Accent && tile.contains(f.rect.left + 1.0, f.rect.top + 1.0) && f.rect.width() < 40.0), "its mark is teal");
}

#[test]
fn what_could_not_be_brought_in_is_said_in_red() {
    let eng = eng();
    let mut w = Welcome::new();
    w.began(From::Keep, "Google Keep");
    w.finished(Outcome::Failed(NOT_AN_EXPORT.into()));
    let s = build(&eng, &w, Platform::Windows, on(), None, 1200.0, 900.0);
    let said = text_of(&s, "This isn't an export Kolada can read.").expect("said as it is");
    assert_eq!(said.ink, Ink::Danger);
    w.began(From::Evernote, "Evernote");
    let s = build(&eng, &w, Platform::Windows, on(), None, 1200.0, 900.0);
    assert_eq!(text_of(&s, "Bringing in Evernote\u{2026}").unwrap().ink, Ink::Muted, "red is only for what went wrong");
}

/// Notes itself is asked, and cannot be stopped part way: no Stop.
#[test]
fn apple_notes_offers_no_stop() {
    let eng = eng();
    let mut w = Welcome::new();
    w.began(From::AppleNotes, From::AppleNotes.said());
    assert!(w.busy() && !w.stoppable());
    assert_eq!(w.click(Hit::Stop), Command::None);
    assert_eq!(w.click(Hit::Tile(From::Evernote)), Command::None, "one at a time");
    let s = build(&eng, &w, Platform::Mac, on(), None, 1200.0, 900.0);
    assert!(rect_of(&s, Target::Welcome(Hit::Stop)).is_none());
    assert!(text_of(&s, "Bringing in Apple Notes\u{2026}").is_some());
}

/// A Takeout picked on Notion's tile is said as Keep's.
#[test]
fn an_export_found_to_be_another_apps_is_named_for_it() {
    let mut w = Welcome::new();
    w.began(From::Notion, "Notion");
    w.found(From::Keep, From::Keep.said());
    w.finished(Outcome::Brought { count: 3, folder: "From Google Keep".into(), failed: 0 });
    assert_eq!(w.words(), "3 notes from Google Keep are in the folder \u{2018}From Google Keep\u{2019}");
    assert_eq!(w.brought, [From::Keep], "Keep's tile is ticked");
}

/// Every size, side-by-side snaps and phones included, both switches each
/// way, each line, long words and right-to-left ones: nothing leaves the
/// window, the tiles never overlap, and scrolled to its end, Start is in
/// reach.
#[test]
fn holds_together_at_every_window_size() {
    let eng = eng();
    let mut lines = vec![Welcome::new()];
    let mut w = Welcome::new();
    w.began(From::Evernote, "Evernote");
    w.progress((12_345, 67_890));
    lines.push(w.clone());
    w.finished(Outcome::Brought { count: 67_890, folder: "\u{645}\u{646} \u{625}\u{64A}\u{641}\u{631}\u{646}\u{648}\u{62A} \u{627}\u{644}\u{642}\u{62F}\u{64A}\u{645}\u{629}".into(), failed: 3 });
    lines.push(w.clone());
    w.began(From::Folder, "Research notes from a very long folder name");
    w.finished(Outcome::Failed("it is not there any more".into()));
    lines.push(w);
    let mut checked = 0;
    for platform in [Platform::Windows, Platform::Mac] {
        for (wi, welcome) in lines.iter().enumerate() {
            for options in [on(), off()] {
                for width in (320..=2560).step_by(80).map(|w| w as f32) {
                    for height in [480.0, 640.0, 900.0, 1440.0] {
                        let mut welcome = welcome.clone();
                        let s = build(&eng, &welcome, platform, options, None, width, height);
                        let mut tiles: Vec<Rect> = ALL.iter().filter_map(|&f| rect_of(&s, Target::Welcome(Hit::Tile(f)))).collect();
                        for t in &s.texts {
                            let m = t.layout.measure();
                            assert!(t.x >= -0.5 && t.x + m.width <= width + 0.5, "{width}x{height} line {wi}: {:?} at {} wide {}", t.layout.text(), t.x, m.width);
                        }
                        for (r, t) in s.hits.iter().filter(|(_, t)| focusable(*t) || matches!(t, Target::Welcome(_))) {
                            assert!(r.left >= -0.5 && r.right <= width + 0.5 && r.top >= -0.5 && r.bottom <= height + 0.5, "{width}x{height}: {t:?} {r:?}");
                        }
                        tiles.sort_by(|a, b| a.top.total_cmp(&b.top).then(a.left.total_cmp(&b.left)));
                        for (i, a) in tiles.iter().enumerate() {
                            for b in &tiles[i + 1..] {
                                let apart = a.right <= b.left + 0.5 || b.right <= a.left + 0.5 || a.bottom <= b.top + 0.5 || b.bottom <= a.top + 0.5;
                                assert!(apart, "{width}x{height}: tiles overlap {a:?} {b:?}");
                            }
                        }
                        // At its end, Start is there to press.
                        welcome.scroll = s.welcome_scroll_max;
                        let end = build(&eng, &welcome, platform, options, None, width, height);
                        let start = rect_of(&end, Target::Welcome(Hit::Start)).expect("Start");
                        assert!(start.height() >= 39.0 && start.bottom <= height, "{width}x{height}: {start:?}");
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 1000);
}

#[test]
fn tab_walks_its_controls_in_order_and_scrolls_them_into_view() {
    let eng = eng();
    let mut w = Welcome::new();
    let s = build(&eng, &w, Platform::Windows, on(), None, 1000.0, 900.0);
    let c = &s.welcome_controls;
    let first = next_focus(c, None, false).unwrap();
    assert_eq!(first, Target::Welcome(Hit::Tile(From::Obsidian)));
    assert_eq!(next_focus(c, None, true), Some(Target::Welcome(Hit::Start)), "back from nothing: the last");
    let mut walked = vec![first];
    let mut at = first;
    loop {
        at = next_focus(c, Some(at), false).unwrap();
        if at == first {
            break;
        }
        walked.push(at);
    }
    let i = |t| walked.iter().position(|&x| x == t).unwrap();
    assert!(i(Target::Welcome(Hit::Tile(From::Documents))) < i(Target::Switch(Setting::Assist)));
    assert!(i(Target::Switch(Setting::Assist)) < i(Target::Choice(Setting::Assist, 1)));
    assert!(i(Target::Choice(Setting::Assist, 1)) < i(Target::Switch(Setting::Beam)));
    assert!(i(Target::Quick(crate::quick::Hit::EnterCode)) < i(Target::Welcome(Hit::Start)));
    assert_eq!(walked.iter().filter(|t| matches!(t, Target::Close | Target::Caption | Target::Welcome(Hit::Page))).count(), 0);
    // Focus is drawn as the pointer's hover: a tile lights.
    w.focus = Some(Target::Welcome(Hit::Tile(From::Notion)));
    let lit = build(&eng, &w, Platform::Windows, on(), None, 1000.0, 900.0);
    let tile = rect_of(&lit, Target::Welcome(Hit::Tile(From::Notion))).unwrap();
    assert!(lit.fills.iter().any(|f| f.ink == Ink::Hover && f.rect == tile));
    // A short window: Start is below, and Tab to it brings it into view.
    let short = build(&eng, &w, Platform::Windows, on(), None, 700.0, 520.0);
    assert!(short.welcome_scroll_max > 0.0);
    assert!(rect_of(&short, Target::Welcome(Hit::Start)).is_none(), "out of view, out of reach of the pointer");
    assert_eq!(next_focus(&short.welcome_controls, None, true), Some(Target::Welcome(Hit::Start)), "but not of Tab");
    reveal(&mut w, &short, Target::Welcome(Hit::Start), 520.0);
    assert!(w.scroll > 0.0);
    let after = build(&eng, &w, Platform::Windows, on(), None, 700.0, 520.0);
    let start = rect_of(&after, Target::Welcome(Hit::Start)).unwrap();
    assert!(start.bottom <= 520.0 && start.height() >= 39.0, "{start:?}");
}

#[test]
fn every_control_has_a_name_a_screen_reader_can_say() {
    for f in ALL {
        assert!(name(Target::Welcome(Hit::Tile(f))).unwrap().contains(f.name()));
    }
    assert_eq!(name(Target::Welcome(Hit::Start)).as_deref(), Some("Start writing"));
    assert!(name(Target::Welcome(Hit::Stop)).is_some());
    assert!(name(Target::Close).is_none(), "the rest are named where they always are");
}

#[test]
fn each_tile_asks_for_what_it_reads() {
    assert_eq!(From::Obsidian.pick(), Pick::Folder("Choose your Obsidian vault"));
    assert!(matches!(From::Evernote.pick(), Pick::Files { exts: ["enex"], many: true, .. }));
    assert!(matches!(From::NotesApp.pick(), Pick::Files { exts: ["nbk"], .. }));
    assert!(matches!(From::Documents.pick(), Pick::Files { many: true, .. }));
    assert_eq!(From::AppleNotes.pick(), Pick::Nothing);
    assert_eq!(From::Evernote.folder(), "From Evernote");
    assert_eq!(From::Documents.folder(), "From documents");
    assert!(ALL.iter().all(|f| f.ready()), "every tile reads now");
    assert!(matches!(From::Bear.pick(), Pick::Files { exts: ["bear2bk", "zip"], .. }));
}

/// Where each control is, for driving a test instance by hand.
#[test]
#[ignore = "prints where the welcome's controls are"]
fn print_places() {
    let eng = eng();
    let (w, h) = (1200.0, 740.0);
    for (name, options) in [("on", on()), ("beam off", { let mut o = on(); o[Setting::Beam.index()] = 1; o })] {
        let mut busy = Welcome::new();
        busy.began(From::Evernote, "Evernote");
        let s = build(&eng, &busy, Platform::Windows, options, None, w, h);
        println!("== {name}, max scroll {}", s.welcome_scroll_max);
        for (r, t) in &s.welcome_controls {
            println!("{t:?} {:.0} {:.0}", (r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        }
    }
}

/// Every section ends where the tiles do, and Start writing is the one
/// filled button: Pair a device is a plain one, as Enter a code is.
#[test]
fn its_sections_share_one_right_edge_and_one_primary_button() {
    let eng = eng();
    for (w, h) in [(1280.0, 1000.0), (640.0, 1000.0)] {
        let s = build(&eng, &Welcome::new(), Platform::Windows, on(), None, w, h);
        let tiles = ALL.iter().filter_map(|&f| rect_of(&s, Target::Welcome(Hit::Tile(f)))).map(|r| r.right).fold(0.0, f32::max);
        // Each switch's track (its teal fill, both on) ends there.
        let tracks: Vec<f32> = s.fills.iter().filter(|f| f.ink == Ink::Accent && (f.rect.width() - crate::chrome::SWITCH_W).abs() < 0.5).map(|f| f.rect.right).collect();
        assert_eq!(tracks.len(), 2, "{w}");
        for right in tracks {
            assert!((right - tiles).abs() < 1.0, "{w}: a switch ends at {right}, the tiles at {tiles}");
        }
        let pair = rect_of(&s, Target::Quick(crate::quick::Hit::ShowCode)).unwrap();
        let filled = s.fills.iter().filter(|f| f.rect == pair).map(|f| f.ink).collect::<Vec<_>>();
        assert!(!filled.contains(&Ink::Text), "{w}: Pair a device is not filled like a primary button: {filled:?}");
    }
}

/// Light | Full shows the one chosen, as Settings does (Light until a
/// model is), and the only small words are the tiles' hints.
#[test]
fn the_writing_models_choice_shows_and_no_line_explains_anything() {
    let eng = eng();
    let s = build(&eng, &Welcome::new(), Platform::Windows, on(), None, 1280.0, 1000.0);
    let light = rect_of(&s, Target::Choice(Setting::Assist, 0)).unwrap();
    assert!(s.fills.iter().any(|f| f.ink == Ink::Raised && f.rect == light), "Light is shown chosen");
    assert!(s.fills.iter().any(|f| f.ink == Ink::Accent && f.rect == light && f.alpha > 0.1), "and tinted, plain on graphite too");
    // No line explains anything: the only small words are the tiles' hints.
    for words in ["Or drop files anywhere on this window.", "All of this lives in Settings and the palette later."] {
        assert!(text_of(&s, words).is_none(), "{words}");
    }
    let small = Fonts::new(1.0, 0).small.size;
    let hints: Vec<&str> = ALL.iter().map(|f| f.hint()).collect();
    for t in &s.texts {
        let text = t.layout.text();
        // Icons (private-use glyphs) are no words.
        let icon = text.chars().all(|c| ('\u{E000}'..='\u{F8FF}').contains(&c));
        if t.layout.format().size <= small && !text.is_empty() && !icon {
            assert!(hints.contains(&text) || ["BRING YOUR NOTES", "WRITING HELP", "YOUR DEVICES", "This one", "Rename", "Pair a device", "Enter a code"].contains(&text), "a small line: {text:?}");
        }
    }
}

/// Scrolled, the tiles dissolve under the window's top row rather than
/// stop hard at it.
#[test]
fn scrolled_it_fades_under_the_top_row() {
    let eng = eng();
    let mut w = Welcome::new();
    let rest = build(&eng, &w, Platform::Windows, on(), None, 700.0, 520.0);
    assert!(!rest.fills.iter().any(|f| f.blur > 0.0 && f.ink == Ink::Page), "at the top, no fade");
    w.scroll = 80.0;
    let s = build(&eng, &w, Platform::Windows, on(), None, 700.0, 520.0);
    assert!(s.fills.iter().any(|f| f.blur > 0.0 && f.ink == Ink::Page && f.rect.bottom <= TOP), "a soft page-coloured edge under the top row");
}

/// Every tile's mark is one of Kolada's filled marks, by kind: the zips
/// share the archive box, the documents one glyph; never a letter.
#[test]
fn every_tile_has_a_filled_mark_by_its_kind() {
    assert_eq!(From::Keep.mark(), Mark::Drawn(Logo::Archive));
    assert_eq!(From::Notion.mark(), Mark::Drawn(Logo::Archive));
    assert_eq!(From::Bear.mark(), Mark::Drawn(Logo::Archive));
    assert_eq!(From::NotesApp.mark(), Mark::Drawn(Logo::NotePage), "a filled page, not an outlined one");
    assert_eq!(From::Evernote.mark(), From::Documents.mark());
    for f in ALL {
        if let Mark::Glyph(g) = f.mark() {
            assert!(g.chars().all(|c| ('\u{E000}'..='\u{F8FF}').contains(&c)), "{f:?}: an icon, not a letter");
        }
    }
    let eng = eng();
    let s = build(&eng, &Welcome::new(), Platform::Windows, on(), None, 1280.0, 1000.0);
    let boxes: Vec<&VectorMark> = s.vectors.iter().filter(|v| v.logo == Logo::Archive).collect();
    assert_eq!(boxes.len(), 3);
    assert!(boxes.iter().all(|b| b.ink == Ink::Muted && (b.rect.width() - 15.0).abs() < 0.1));
}
