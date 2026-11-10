//! The Mac window at every size it can be: the Mac model draws the shared
//! scene with its own inputs (the title bar is the system's, the note runs
//! to the top), so the shared test of the scene at every size
//! (`holds_together_at_every_window_size` in kolada-ui's chrome.rs) is not
//! enough: this one frames the model itself, in each state its screen can be
//! in, at every size and both pixel densities.

use super::*;

fn overlap(a: Rect, b: Rect) -> f32 {
    (a.right.min(b.right) - a.left.max(b.left)).max(0.0) * (a.bottom.min(b.bottom) - a.top.max(b.top)).max(0.0)
}

fn rect_of(m: &Model, target: Target) -> Option<Rect> {
    m.hits.iter().find(|(_, t)| *t == target).map(|(r, _)| *r)
}

#[test]
fn the_mac_window_holds_together_at_every_window_size() {
    let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-mac-window-sizes", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Work")).unwrap();
    let body: String = (0..60).map(|i| format!("Line {i} of a note, long enough to wrap in a narrow window, with **bold** and a [[Link]].\n")).collect();
    std::fs::write(dir.join("Plan.md"), format!("# Plan\n\n- [ ] a task\n\n{body}")).unwrap();
    std::fs::write(dir.join("Work").join("Inbox.md"), "# Inbox\n").unwrap();
    // A caught page: its frontmatter one line over the title, its host a link.
    std::fs::write(dir.join("Caught.md"), "---\nsource: https://www.a-kitchen-with-a-rather-long-name.example/recipes/lemon-tart\ncaught: 2025-11-08T15:20\n---\n# Lemon tart\n\nZest two lemons.\n").unwrap();
    std::fs::write(dir.join("Arabic.md"), "# \u{645}\u{631}\u{62D}\u{628}\u{627}\n\n\u{645}\u{631}\u{62D}\u{628}\u{627} \u{628}\u{643}\u{645} \u{641}\u{64A} \u{627}\u{644}\u{645}\u{644}\u{627}\u{62D}\u{638}\u{627}\u{62A}\n").unwrap();
    let mut checked = 0;
    for note in ["Plan.md", "Arabic.md", "Caught.md"] {
        for state in ["note", "search", "found", "trash", "find", "settings", "minimal", "link", "model", "update", "welcome", "grid writing", "grid browse"] {
            if note == "Caught.md" && !matches!(state, "note" | "minimal" | "find" | "update") {
                continue;
            }
            for scale in [1.0f32, 2.0] {
                let mut m = Model::new(dir.clone(), true);
                m.open(note);
                match state {
                    "search" => assert!(m.open_search()),
                    // Searching, the note opened on what was found: its
                    // matches tinted, the search beside it.
                    "found" => {
                        let query = if note == "Plan.md" { "line" } else { "\u{645}\u{631}\u{62D}\u{628}\u{627}" };
                        assert!(m.open_search());
                        m.insert(query);
                        m.opened_from_search(query);
                        assert!(!m.found_view().is_empty());
                        m.access_kept = true;
                    }
                    "trash" => assert!(m.open_trash()),
                    "find" => assert!(m.open_find(true)),
                    "settings" => assert!(m.toggle_options()),
                    "minimal" => assert!(m.set_minimal(true)),
                    // The link field over the note's last words.
                    "link" => {
                        m.go(Motion::DocEnd, false);
                        m.go(Motion::WordLeft, true);
                        assert!(m.open_link_field());
                        m.insert("Li");
                    }
                    "model" => m.demo_model_card(),
                    "welcome" => {
                        m.set_platform(scene::Platform::Mac);
                        assert!(m.show_welcome());
                    }
                    "update" => {
                        m.set_updater(true);
                        assert!(m.update_ready("9.9.9"));
                    }
                    // The notes as a grid: writing (the boxes down, the pill
                    // at the foot), and browsing them (up, over it).
                    "grid writing" | "grid browse" => {
                        assert!(m.toggle_view());
                        let phase = if state == "grid writing" { kolada_ui::grid::Phase::Writing } else { kolada_ui::grid::Phase::Browse };
                        m.grid.state.as_mut().unwrap().grid = kolada_ui::grid::Grid::at(phase);
                    }
                    _ => {}
                }
                for height in [320.0f32, 400.0, 720.0, 1400.0] {
                    for width in (360..=2600).step_by(40).map(|w| w as f32) {
                        let at = format!("{note} {state} at {width} x {height} (x{scale})");
                        let (w, h) = ((width * scale) as u32, (height * scale) as u32);
                        m.frame(w, h, scale);
                        assert_eq!(m.surface().pixels().len(), (w * h) as usize, "the frame is the window's size, {at}");
                        // Every control that is drawn is inside the window and clear of the others.
                        let controls: Vec<(Target, Rect)> = m
                            .hits
                            .iter()
                            .filter(|(r, t)| {
                                r.right > r.left
                                    && (state != "settings" || *t == Target::OptionsClose)
                                    // The model card asks over the dimmed page: its own controls.
                                    && (state != "model" || matches!(t, Target::Model(_)))
                                    // The card's own panel (and the dimmed page) hold the controls.
                                    && *t != Target::Model(kolada_ui::modelcard::Hit::Card)
                                    && matches!(
                                        t,
                                        Target::Search
                                            | Target::NewNote
                                            | Target::Trash
                                            | Target::ThemeCycle
                                            | Target::TextSize(_)
                                            | Target::LineWidth
                                            | Target::Tool(_)
                                            | Target::TrashBack
                                            | Target::OptionsClose
                                            | Target::Update
                                            | Target::LinkField
                                            | Target::LinkRemove
                                            | Target::LinkSuggestion(_)
                                            | Target::Model(_)
                                            | Target::Source
                                    )
                            })
                            .map(|(r, t)| (*t, *r))
                            .collect();
                        if state == "note" && width >= 1000.0 {
                            for want in [Target::Search, Target::NewNote, Target::Trash, Target::ThemeCycle] {
                                assert!(controls.iter().any(|(t, _)| *t == want), "{want:?} is missing, {at}");
                            }
                        }
                        if state == "link" {
                            assert!(controls.iter().any(|(t, _)| *t == Target::LinkField), "the link field is drawn, {at}");
                        }
                        // The frontmatter folded to one line, its host in the note's column.
                        if note == "Caught.md" && matches!(state, "note" | "minimal") {
                            let host = rect_of(&m, Target::Source).unwrap_or_else(|| panic!("the host on the frontmatter's line, {at}"));
                            assert!(host.left >= m.ui.column.left - 3.0 && host.right <= m.ui.column.right + 3.0, "the host {host:?} out of the column {:?}, {at}", m.ui.column);
                        }
                        if state == "model" {
                            for want in [Target::Model(kolada_ui::modelcard::Hit::Download), Target::Model(kolada_ui::modelcard::Hit::NotNow)] {
                                assert!(controls.iter().any(|(t, _)| *t == want), "{want:?} is missing, {at}");
                            }
                        }
                        for (t, r) in &controls {
                            assert!(r.left >= -0.5 && r.top >= -0.5 && r.right <= width + 0.5 && r.bottom <= height + 0.5, "{t:?} leaves the window ({r:?}), {at}");
                        }
                        for (i, (ta, a)) in controls.iter().enumerate() {
                            for (tb, b) in &controls[i + 1..] {
                                assert!(overlap(*a, *b) <= 1.0, "{ta:?} and {tb:?} overlap ({a:?}, {b:?}), {at}");
                            }
                        }
                        // The traffic lights float over the top left: no control,
                        // row, reveal zone or pill of ours is under them.
                        let lights = kolada_ui::chrome::TRAFFIC_LIGHTS;
                        for (r, t) in m.hits.iter().filter(|(r, t)| {
                            r.right > r.left
                                && matches!(
                                    t,
                                    Target::Search | Target::NewNote | Target::Highlights | Target::Trash | Target::ThemeCycle | Target::Options | Target::ToggleSidebar | Target::ToggleToolbar
                                        | Target::TextSize(_) | Target::LineWidth | Target::Tool(_) | Target::TrashBack | Target::Row(_) | Target::RevealSidebar | Target::RevealToolbar
                                        // The dates over the title, and the teal dot by them (seen in
                                        // the minimal window on the real screen: they touched the lights).
                                        | Target::PageDate | Target::DueDot
                                        // The update pill, and the link field's own controls.
                                        | Target::Update | Target::LinkField | Target::LinkRemove
                                        // The frontmatter's host, on its one line.
                                        | Target::Source
                                )
                        }) {
                            assert!(overlap(*r, lights) <= 0.0, "{t:?} {r:?} is under the traffic lights, {at}");
                        }
                        // The welcome's tiles and Start, as far as it shows: across
                        // the window's width, clear of each other and the lights.
                        if state == "welcome" {
                            use kolada_ui::welcome::Hit;
                            let shown: Vec<(Target, Rect)> = m.hits.iter().filter(|(r, t)| r.right > r.left && r.top < height && matches!(t, Target::Welcome(Hit::Tile(_) | Hit::Start))).map(|(r, t)| (*t, *r)).collect();
                            assert!(shown.iter().any(|(t, _)| matches!(t, Target::Welcome(Hit::Tile(_)))), "the welcome's tiles show, {at}");
                            for (i, (t, r)) in shown.iter().enumerate() {
                                assert!(r.left >= -0.5 && r.right <= width + 0.5, "{t:?} leaves the window ({r:?}), {at}");
                                assert!(overlap(*r, lights) <= 0.0, "{t:?} {r:?} is under the traffic lights, {at}");
                                for (u, s) in &shown[i + 1..] {
                                    assert!(overlap(*r, *s) <= 1.0, "{t:?} and {u:?} overlap, {at}");
                                }
                            }
                        }
                        // The options' choices are across the window's width (the panel scrolls down).
                        for (r, t) in m.hits.iter().filter(|(_, t)| matches!(t, Target::Choice(..))) {
                            assert!(r.left >= -0.5 && r.right <= width + 0.5, "{t:?} leaves the window ({r:?}), {at}");
                        }
                        // The note is where there is room to read it, inside the window.
                        if state != "trash" {
                            if let Some(n) = rect_of(&m, Target::Note) {
                                assert!(n.left >= -0.5 && n.right <= width + 0.5, "the note leaves the window ({n:?}), {at}");
                                assert!(n.width() >= 220.0_f32.min(width - 40.0), "the note is too narrow to read ({n:?}), {at}");
                            }
                            for l in &m.lines {
                                assert!(l.x >= -0.5 && l.x <= width, "a line starts outside the window ({}), {at}", l.x);
                            }
                        }
                        if let Some(c) = m.caret_rect() {
                            assert!(c.right <= width + 1.0 && c.left >= -1.0, "the caret is outside the window, {at}");
                        }
                        // The grid's pill: writing, on the foot's middle, in the
                        // window, under the note's text and clear of every
                        // control; browsing, under the boxes. The list has none.
                        let (pill, zone) = (m.panels.pills[2], rect_of(&m, Target::GridPill));
                        if state == "grid writing" {
                            if let Some(p) = pill {
                                let zone = zone.expect("the pill's area");
                                assert!(((p.left + p.right) / 2.0 - width / 2.0).abs() <= 0.5 && p.left >= 0.0 && p.right <= width && p.bottom <= height, "the grid's pill {p:?} off the foot's middle, {at}");
                                assert!(p.top >= m.ui.column.bottom, "the grid's pill {p:?} over the note's text ({:?}), {at}", m.ui.column);
                                assert!(zone.left >= -0.5 && zone.right <= width + 0.5 && zone.bottom <= height + 0.5, "its area {zone:?} leaves the window, {at}");
                                for (t, r) in &controls {
                                    assert!(overlap(*r, zone) <= 0.5, "{t:?} {r:?} under the grid's pill, {at}");
                                }
                                for l in &m.lines {
                                    assert!(l.y <= m.ui.column.bottom + 0.5, "a line drawn under the grid's pill ({}), {at}", l.y);
                                }
                            } else {
                                assert!(width < 900.0, "no grid pill in a wide window, {at}");
                            }
                        } else {
                            assert!(pill.is_none() && zone.is_none(), "the grid's pill {pill:?} in the list or over the boxes, {at}");
                        }
                        // What the search found: tinted on the note's words
                        // (inside its column) and in the results' rows, inside
                        // the window.
                        if state == "found" {
                            let s = m.kept_scene.as_ref().expect("kept");
                            let tints: Vec<Rect> = s.fills.iter().filter(|f| f.ink == kolada_ui::geom::Ink::Mark && f.radius == 3.0).map(|f| f.rect).collect();
                            let rows: Vec<Rect> = m.hits.iter().filter(|(_, t)| matches!(t, Target::Result(_))).map(|(r, _)| *r).collect();
                            assert!(!tints.is_empty(), "the matches in sight are tinted, {at}");
                            for t in &tints {
                                // (A line the window's foot cuts through is cut with its
                                // tint; the page lays out a little past it, unseen.)
                                if t.top >= height {
                                    continue;
                                }
                                assert!(t.left >= -0.5 && t.right <= width + 0.5, "a tint {t:?} leaves the window, {at}");
                                let in_note = t.left >= m.ui.column.left - 1.5 && t.right <= m.ui.column.right + 1.5;
                                let in_row = rows.iter().any(|r| t.left >= r.left - 0.5 && t.right <= r.right + 0.5 && t.top >= r.top - 0.5 && t.bottom <= r.bottom + 0.5);
                                assert!(in_note || in_row, "a tint {t:?} off the note's column {:?} and the results, {at}", m.ui.column);
                            }
                            // The first match in the note is never under the side
                            // menu (docked beside it; a floating one closes).
                            if let Some(first) = tints.iter().find(|t| t.left >= m.ui.column.left - 1.5 && t.right <= m.ui.column.right + 1.5 && t.top < height) {
                                for (r, _) in m.hits.iter().filter(|(_, t)| *t == Target::SidebarPanel) {
                                    assert!(overlap(*first, *r) <= 0.0, "the first match {first:?} under the side menu {r:?}, {at}");
                                }
                            }
                        }
                        checked += 1;
                    }
                }
            }
        }
    }
    eprintln!("{checked} sizes and states framed");
    let _ = std::fs::remove_dir_all(dir);
}

/// iPad and iPhone: every size iOS gives a window. An iPad's screens
/// (744 to 1032 points across, either way up), Split View and Slide Over
/// beside another app (320 points up), iPadOS's free windows, and every
/// iPhone either way up; the note's view runs from under the status bar to
/// the keyboard, so its height goes down to an iPhone on its side with the
/// keyboard up (under 200 points). Each at the scales those screens have.
#[test]
fn the_ipad_and_iphone_window_holds_together_at_every_size_split_view_and_slide_over_included() {
    // Widths: Slide Over and the narrowest Split View (320), iPhones
    // (375 to 440), Split View halves and thirds (438 to 694), iPad screens
    // (744 to 1032) and landscape ones (1133 to 1376); in between, the free
    // windows of iPadOS.
    let widths: Vec<f32> = (320..=1376).step_by(24).map(|w| w as f32).chain([375.0, 390.0, 393.0, 402.0, 430.0, 440.0, 504.0, 507.0, 551.0, 639.0, 678.0, 694.0, 744.0, 820.0, 834.0, 1024.0, 1032.0, 1133.0, 1180.0, 1194.0, 1366.0]).collect();
    // Phones are 3x (2x for the smallest), iPads 2x.
    holds_together_on_a_touch_screen(scene::Platform::Ios, &widths, |w, h| if w <= 440.0 && h <= 956.0 { 3.0 } else { 2.0 });
}

/// Android phones and tablets: every size a window gets. Phones upright
/// (360 to 480 DIPs across) and on their side, split screen's halves and
/// thirds (down to 320 across and under 200 high with the keyboard up),
/// foldables folded and open (600 to 884), tablets (600 to 1366) and
/// free-form windows between. Phones at 2.625 to 3.5 pixels a DIP,
/// tablets at 2.
#[test]
fn the_android_window_holds_together_at_every_size_split_screen_and_foldables_included() {
    let widths: Vec<f32> = (320..=1376).step_by(24).map(|w| w as f32).chain([360.0, 384.0, 393.0, 411.0, 412.0, 432.0, 448.0, 480.0, 600.0, 673.0, 717.0, 800.0, 841.0, 884.0, 960.0, 1280.0, 1366.0]).collect();
    holds_together_on_a_touch_screen(scene::Platform::Android, &widths, |w, h| if w <= 412.0 && h <= 915.0 { 3.5 } else if w <= 480.0 { 2.625 } else { 2.0 });
}

/// Every state of the app framed at every `widths` x heights, on a touch
/// screen's `platform`, at the `scale` its screens have there: every
/// control in the window, none on another, the note wide enough to read.
fn holds_together_on_a_touch_screen(platform: scene::Platform, widths: &[f32], scale: impl Fn(f32, f32) -> f32) {
    let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-{platform:?}-sizes", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Work")).unwrap();
    let body: String = (0..40).map(|i| format!("Line {i} of a note, long enough to wrap on a phone, with **bold**, a [[Link]] and - [ ] a task.\n")).collect();
    std::fs::write(dir.join("Plan.md"), format!("# Plan\n\n- [ ] a task @fri\n\n| A | B |\n|---|---|\n| one | two |\n\n{body}")).unwrap();
    std::fs::write(dir.join("Work").join("Inbox.md"), "# Inbox\n").unwrap();
    std::fs::write(dir.join("Arabic.md"), "# \u{645}\u{631}\u{62D}\u{628}\u{627}\n\n\u{645}\u{631}\u{62D}\u{628}\u{627} \u{628}\u{643}\u{645} \u{641}\u{64A} \u{627}\u{644}\u{645}\u{644}\u{627}\u{62D}\u{638}\u{627}\u{62A}\n").unwrap();
    let heights = [180.0f32, 260.0, 420.0, 640.0, 900.0, 1366.0];
    let mut checked = 0;
    for note in ["Plan.md", "Arabic.md"] {
        for state in ["note", "menu", "search", "trash", "find", "settings", "palette", "today"] {
            let mut m = Model::new(dir.clone(), true);
            m.set_platform(platform);
            m.reduce_motion(true);
            m.open(note);
            match state {
                "menu" => {
                    m.frame(390 * 3, 844 * 3, 3.0);
                    m.reveal_side_menu();
                }
                "search" => assert!(m.open_search()),
                "trash" => assert!(m.open_trash()),
                "find" => assert!(m.open_find(true)),
                "settings" => assert!(m.toggle_options()),
                "palette" => assert!(m.open_palette()),
                "today" => {
                    m.toggle_agenda();
                }
                _ => {}
            }
            for &height in &heights {
                for &width in widths {
                    let scale = scale(width, height);
                    let at = format!("{platform:?}: {note} {state} at {width} x {height} (x{scale})");
                    let (w, h) = ((width * scale) as u32, (height * scale) as u32);
                    m.frame(w, h, scale);
                    assert_eq!(m.surface().pixels().len(), (w * h) as usize, "the frame is the window's size, {at}");
                    let controls: Vec<(Target, Rect)> = m
                        .hits
                        .iter()
                        .filter(|(r, t)| {
                            r.right > r.left
                                && (state != "settings" || *t == Target::OptionsClose)
                                && matches!(t, Target::Search | Target::NewNote | Target::Trash | Target::ThemeCycle | Target::TextSize(_) | Target::LineWidth | Target::Tool(_) | Target::TrashBack | Target::OptionsClose | Target::Options | Target::Row(_) | Target::Result(_) | Target::TrashRow(_) | Target::ToggleSidebar | Target::ToggleToolbar)
                        })
                        .map(|(r, t)| (*t, *r))
                        .collect();
                    for (t, r) in &controls {
                        assert!(r.left >= -0.5 && r.top >= -0.5 && r.right <= width + 0.5 && r.bottom <= height + 0.5, "{t:?} leaves the window ({r:?}), {at}");
                    }
                    for (i, (ta, a)) in controls.iter().enumerate() {
                        for (tb, b) in &controls[i + 1..] {
                            assert!(overlap(*a, *b) <= 1.0, "{ta:?} and {tb:?} overlap ({a:?}, {b:?}), {at}");
                        }
                    }
                    // Whatever a finger can tap is on the screen: a row, a
                    // result, an option, the card's buttons.
                    for (r, t) in m.hits.iter().filter(|(r, t)| r.right > r.left && matches!(t, Target::Choice(..) | Target::Row(_) | Target::Result(_) | Target::TrashRow(_) | Target::OptionsClose)) {
                        assert!(r.left >= -0.5 && r.right <= width + 0.5, "{t:?} leaves the window across ({r:?}), {at}");
                    }
                    if !matches!(state, "trash" | "settings") {
                        if let Some(n) = rect_of(&m, Target::Note) {
                            assert!(n.left >= -0.5 && n.right <= width + 0.5, "the note leaves the window ({n:?}), {at}");
                            assert!(n.width() >= 220.0_f32.min(width - 40.0), "the note is too narrow to read ({n:?}), {at}");
                        }
                        for l in &m.lines {
                            assert!(l.x >= -0.5 && l.x <= width, "a line starts outside the window ({}), {at}", l.x);
                        }
                    }
                    if let Some(c) = m.caret_rect() {
                        assert!(c.right <= width + 1.0 && c.left >= -1.0, "the caret is outside the window, {at}");
                    }
                    checked += 1;
                }
            }
        }
    }
    eprintln!("{checked} {platform:?} sizes and states framed");
    let _ = std::fs::remove_dir_all(dir);
}
