use super::*;

/// A small vault of its own: tests run side by side, so they cannot share one.
fn vault(test: &str, notes: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kolada-app-past-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Projects")).unwrap();
    for (name, text) in notes {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

/// A model on `v` whose notes keep their past in a folder of the test's own,
/// never the app's data.
fn drawn(v: &Path) -> (Model, PathBuf) {
    let data = v.with_extension("data");
    let _ = std::fs::remove_dir_all(&data);
    let mut m = Model::new(v.to_path_buf(), true);
    m.data = Some(data.clone());
    m.frame(2000, 1440, 2.0);
    (m, data)
}

fn history_of(data: &Path, v: &Path, rel: &str) -> History {
    History::open(&history::file(&history::dir(data, v), rel))
}

fn names(m: &Model) -> Vec<String> {
    m.session.sidebar.iter().map(|r| r.name.clone()).collect()
}

/// Edits the note: `more` at its end, then written and left (a version each).
fn write_more(m: &mut Model, more: &str) {
    m.go(Motion::DocEnd, false);
    m.insert(more);
    assert!(m.save());
    m.save_flush();
}

/// Settles the scrubber's motion.
fn settle_all(m: &mut Model) {
    for _ in 0..300 {
        m.scrub_step(0.02);
    }
}

#[test]
fn a_notes_past_is_kept_as_it_is_written_what_the_file_said_first_then_a_version_every_half_minute() {
    let v = vault("keep", &[("Plan.md", "# Plan\n\nOne.\n")]);
    let (mut m, data) = drawn(&v);
    m.go(Motion::DocEnd, false);
    m.insert("Two.\n");
    assert!(m.save());
    // What the file said before this run's first save, and nothing more yet:
    // typing goes on.
    let mut h = history_of(&data, &v, "Plan.md");
    assert_eq!((h.len(), h.text(0)), (1, "# Plan\n\nOne.\n".to_string()));
    // Leaving the note keeps it as it is.
    m.save_flush();
    let mut h = history_of(&data, &v, "Plan.md");
    assert_eq!((h.len(), h.text(1)), (2, "# Plan\n\nOne.\nTwo.\n".to_string()));
    // Written again soon: not a version, until half a minute has gone.
    m.insert("Three.\n");
    assert!(m.save());
    assert_eq!(history_of(&data, &v, "Plan.md").len(), 2);
    let kept_at = m.past.kept.1;
    m.keep_version(false, kept_at + KEEP_EVERY_MS - 1);
    assert_eq!(history_of(&data, &v, "Plan.md").len(), 2);
    m.keep_version(false, kept_at + KEEP_EVERY_MS + 1);
    assert_eq!(history_of(&data, &v, "Plan.md").len(), 3);
    let _ = std::fs::remove_dir_all(&v);
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn a_renamed_note_takes_its_past_along() {
    let v = vault("rename", &[("Plan.md", "# Plan\n\nBody.\n"), ("Other.md", "# Other\n")]);
    let (mut m, data) = drawn(&v);
    m.open("Plan.md");
    m.go(Motion::DocStart, false);
    m.go(Motion::LineEnd, false);
    m.insert(" B");
    assert!(m.save());
    m.open("Other.md");
    assert!(v.join("Plan B.md").exists());
    assert_eq!(history_of(&data, &v, "Plan.md").len(), 0, "nothing left under the old name");
    assert!(history_of(&data, &v, "Plan B.md").len() >= 2, "the base and the edit follow the file");
    let _ = std::fs::remove_dir_all(&v);
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn the_scrubber_opens_on_the_version_before_now_steps_restores_in_one_undo_step_and_closes() {
    let v = vault("scrub", &[("Plan.md", "# Plan\n\nOne.\n")]);
    let (mut m, data) = drawn(&v);
    write_more(&mut m, "Two.\n");
    m.insert("Three.\n");
    // Opening writes the note: three versions, the newest is now.
    assert!(m.toggle_scrub());
    assert_eq!(history_of(&data, &v, "Plan.md").len(), 3);
    assert_eq!(m.shown(), Some(1));
    settle_all(&mut m);
    m.frame(2000, 1440, 2.0);
    assert_eq!(m.scrub_page().unwrap().0, "# Plan\n\nOne.\nTwo.\n");
    assert!(m.hits.iter().any(|(_, t)| *t == Target::ScrubTrack) && m.hits.iter().any(|(_, t)| *t == Target::ScrubRestore), "the dock is there to use");
    // Arrows step, Home and End go to the ends; typing is held back.
    assert_eq!(m.scrub_command("moveLeft:"), Some(true));
    assert_eq!(m.shown(), Some(0));
    assert_eq!(m.scrub_command("moveLeft:"), Some(false), "nothing before the first");
    assert_eq!(m.scrub_command("moveToEndOfDocument:"), Some(true));
    assert_eq!(m.shown(), Some(2));
    assert_eq!(m.scrub_page().unwrap().0, m.text(), "the newest is the note as it is");
    assert!(!m.insert("x"), "the past is not typed into");
    assert_eq!(m.text(), "# Plan\n\nOne.\nTwo.\nThree.\n");
    assert_eq!(m.scrub_command("scrollToBeginningOfDocument:"), Some(true));
    assert_eq!(m.scrub_page().unwrap().0, "# Plan\n\nOne.\n");
    // Return brings the version back: one undo step takes it away.
    assert_eq!(m.scrub_command("insertNewline:"), Some(true));
    assert_eq!(m.text(), "# Plan\n\nOne.\n");
    settle_all(&mut m);
    assert!(m.past.scrub.is_none() && !m.scrubbing(), "closed once the dock has faded");
    assert!(m.undo());
    assert_eq!(m.text(), "# Plan\n\nOne.\nTwo.\nThree.\n");
    // Esc closes without changing the note; keys are ours only while it is open.
    assert!(m.toggle_scrub());
    assert_eq!(m.scrub_command("cancelOperation:"), Some(true));
    settle_all(&mut m);
    assert!(m.past.scrub.is_none());
    assert_eq!(m.scrub_command("moveLeft:"), None);
    assert_eq!(m.text(), "# Plan\n\nOne.\nTwo.\nThree.\n");
    let _ = std::fs::remove_dir_all(&v);
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn a_note_with_no_earlier_version_has_no_past_to_open() {
    let v = vault("nopast", &[("Plan.md", "# Plan\n")]);
    let (mut m, data) = drawn(&v);
    // One version is now: nothing before it. It says so, as Windows does
    // (seen on the Mac: Cmd+Shift+H on a new note seemed to do nothing).
    assert!(m.toggle_scrub(), "the screen changes: it says why");
    assert!(!m.scrubbing());
    assert_eq!(m.whispered(), Some("No earlier versions yet: they are kept as you write"));
    assert!(m.new_note() && !m.toggle_scrub(), "a page with no file has none");
    let _ = std::fs::remove_dir_all(&v);
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn the_dates_open_the_past_and_a_drag_along_the_track_goes_to_the_nearest_version() {
    let v = vault("drag", &[("Plan.md", "# Plan\n\nOne.\n")]);
    let (mut m, data) = drawn(&v);
    write_more(&mut m, "Two.\n");
    write_more(&mut m, "Three.\n");
    let date = m.hits.iter().find(|(_, t)| *t == Target::PageDate).map(|(r, _)| *r).expect("the note's dates are a target");
    assert!(m.pointer_down((date.left + date.right) / 2.0, (date.top + date.bottom) / 2.0, 1, false));
    assert!(m.scrubbing());
    settle_all(&mut m);
    m.frame(2000, 1440, 2.0);
    let track = m.hits.iter().find(|(_, t)| *t == Target::ScrubTrack).map(|(r, _)| *r).expect("the track");
    let y = (track.top + track.bottom) / 2.0;
    assert!(m.pointer_down(track.left + 12.0, y, 1, false));
    assert_eq!(m.shown(), Some(0), "the far left is the oldest");
    assert!(m.pointer_dragged(track.right - 12.0, y));
    assert_eq!(m.shown(), Some(2), "dragged to now");
    assert!(m.scrub_dragging() && m.pointer_up() && !m.scrub_dragging());
    assert!(!m.pointer_dragged(track.left + 12.0, y), "no longer held");
    // Clicking the page does not put a caret in the past.
    settle_all(&mut m);
    m.frame(2000, 1440, 2.0);
    let caret = m.selection().head;
    let line = m.lines.first().unwrap();
    assert!(!m.pointer_down(line.x + 20.0, line.y + 4.0, 1, false));
    assert_eq!(m.selection().head, caret);
    // The close button, and the dates again.
    let close = m.hits.iter().find(|(_, t)| *t == Target::ScrubClose).map(|(r, _)| *r).expect("the close button");
    assert!(m.pointer_down((close.left + close.right) / 2.0, (close.top + close.bottom) / 2.0, 1, false));
    settle_all(&mut m);
    assert!(m.past.scrub.is_none());
    let _ = std::fs::remove_dir_all(&v);
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn a_changed_line_comes_back_on_its_own_in_one_undo_step() {
    let v = vault("line", &[("Plan.md", "# Plan\n\nKeep this.\nDrop this.\n")]);
    let (mut m, data) = drawn(&v);
    // "Drop this." goes away, and a line is added.
    m.select_all();
    m.insert("# Plan\n\nKeep this.\nAdded.\n");
    assert!(m.save());
    m.save_flush();
    assert!(m.toggle_scrub());
    assert_eq!(m.shown(), Some(0), "opens on the version before now: the original");
    settle_all(&mut m);
    m.frame(2000, 1440, 2.0);
    let (text, lines) = {
        let (t, l) = m.scrub_page().unwrap();
        (t.to_string(), l.to_vec())
    };
    let changed = history::changed(&text, m.text());
    let i = changed.iter().position(|&c| c).expect("a line differs");
    assert!(text[lines[i].start..lines[i].end].contains("Drop"));
    // The pointer over it offers to bring it back.
    let at = m.lines.iter().find(|l| l.para.start == lines[i].start).unwrap();
    assert!(m.pointer_moved(at.x + 40.0, at.y + 6.0), "hovering the line changes the frame");
    m.frame(2000, 1440, 2.0);
    let pill = m.hits.iter().find(|(_, t)| *t == Target::BringBack(i)).map(|(r, _)| *r).expect("the offer to bring it back");
    assert!(m.pointer_down((pill.left + pill.right) / 2.0, (pill.top + pill.bottom) / 2.0, 1, false));
    assert!(m.text().contains("Drop this."), "it is back");
    assert!(m.dirty());
    assert!(!history::changed(&text, m.text())[i], "and its mark is gone");
    m.scrub_command("cancelOperation:");
    settle_all(&mut m);
    assert!(m.undo());
    assert!(!m.text().contains("Drop this.") && m.text().contains("Added."), "undone in one step");
    let _ = std::fs::remove_dir_all(&v);
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn the_scrubber_draws_the_dock_and_the_version_over_the_page() {
    let v = vault("draw", &[("Plan.md", "# Plan\n\nOne.\n")]);
    let (mut m, data) = drawn(&v);
    let before = m.surface().pixels().to_vec();
    write_more(&mut m, "Two.\n");
    m.frame(2000, 1440, 2.0);
    let plain = m.surface().pixels().to_vec();
    assert!(m.toggle_scrub());
    settle_all(&mut m);
    m.frame(2000, 1440, 2.0);
    let past = m.surface().pixels();
    let differing = past.iter().zip(&plain).filter(|(a, b)| a != b).count();
    assert!(differing > 10_000, "the dock and the older page change the frame: {differing} pixels");
    assert_eq!(before.len(), plain.len());
    let _ = std::fs::remove_dir_all(&v);
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn a_note_just_written_goes_to_the_top_of_its_folder_in_the_side_menu() {
    let v = vault("newest", &[("Old.md", "# Old\n"), ("Newer.md", "# Newer\n")]);
    let t = |s: u64| std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000 + s);
    for (f, at) in [("Old.md", 0), ("Newer.md", 3600)] {
        std::fs::File::options().write(true).open(v.join(f)).unwrap().set_modified(t(at)).unwrap();
    }
    let (mut m, data) = drawn(&v);
    assert_eq!(names(&m), ["Projects", "Newer", "Old"], "newest first, folders before notes");
    assert_eq!(m.session.note, "Newer.md", "and it is the one that opens");
    m.open("Old.md");
    m.go(Motion::DocEnd, false);
    m.insert("More.\n");
    assert!(m.save());
    assert_eq!(names(&m), ["Projects", "Old", "Newer"], "written, so first, without the folder being read again");
    let _ = std::fs::remove_dir_all(&v);
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn the_date_line_gives_the_dates_and_the_words_and_follows_the_typing() {
    let v = vault("dates", &[("Plan.md", "# Plan\n\nShip the beta now.\n")]);
    let (mut m, data) = drawn(&v);
    let line = m.date_line().expect("a note on file has dates");
    assert!(line.contains("Created") || line.contains("Edited"), "{line}");
    assert!(line.ends_with(" \u{00B7} 5 words"), "{line}");
    assert!(m.hits.iter().any(|(_, t)| *t == Target::PageDate), "drawn, and a target");
    m.go(Motion::DocEnd, false);
    m.insert("Then more words here.\n");
    assert!(m.date_line().unwrap().ends_with(" \u{00B7} 9 words"));
    m.open("Plan.md");
    assert!(m.new_note());
    assert_eq!(m.date_line(), None, "a page with no file has no dates");
    let _ = std::fs::remove_dir_all(&v);
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn fades_move_toward_where_they_head_and_stop_there() {
    let mut x = 0.0;
    assert!(toward(&mut x, 1.0, 8.0, 0.05));
    assert!((x - 0.4).abs() < 1e-6);
    while toward(&mut x, 1.0, 8.0, 0.05) {}
    assert_eq!(x, 1.0);
    assert!(!toward(&mut x, 1.0, 8.0, 0.05));
    assert_eq!(time_of(ms(time_of(1_234_567))), time_of(1_234_567));
}
