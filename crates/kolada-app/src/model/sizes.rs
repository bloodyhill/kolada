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
    let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-sizes", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Work")).unwrap();
    let body: String = (0..60).map(|i| format!("Line {i} of a note, long enough to wrap in a narrow window, with **bold** and a [[Link]].\n")).collect();
    std::fs::write(dir.join("Plan.md"), format!("# Plan\n\n- [ ] a task\n\n{body}")).unwrap();
    std::fs::write(dir.join("Work").join("Inbox.md"), "# Inbox\n").unwrap();
    std::fs::write(dir.join("Arabic.md"), "# \u{645}\u{631}\u{62D}\u{628}\u{627}\n\n\u{645}\u{631}\u{62D}\u{628}\u{627} \u{628}\u{643}\u{645} \u{641}\u{64A} \u{627}\u{644}\u{645}\u{644}\u{627}\u{62D}\u{638}\u{627}\u{62A}\n").unwrap();
    let mut checked = 0;
    for note in ["Plan.md", "Arabic.md"] {
        for state in ["note", "search", "trash", "find", "settings", "minimal", "link", "model", "update"] {
            for scale in [1.0f32, 2.0] {
                let mut m = Model::new(dir.clone(), true);
                m.open(note);
                match state {
                    "search" => assert!(m.open_search()),
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
                    "update" => {
                        m.set_updater(true);
                        assert!(m.update_ready("9.9.9"));
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
                                )
                        }) {
                            assert!(overlap(*r, lights) <= 0.0, "{t:?} {r:?} is under the traffic lights, {at}");
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
                        checked += 1;
                    }
                }
            }
        }
    }
    eprintln!("{checked} sizes and states framed");
    let _ = std::fs::remove_dir_all(dir);
}
