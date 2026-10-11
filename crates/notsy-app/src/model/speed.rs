//! The speed gate's timing tests for the model, on stand-in fonts (the Mac's
//! own are timed in `notsy-mac`'s `speed.rs`; tools/speed-gate-mac.sh runs
//! them with `--ignored`, in a release build): the same budgets, notes and
//! window as Windows' `time_a_frame`, `time_a_keystroke_in_a_long_note` and
//! `time_a_keystroke_under_the_title` (crates/notsy-win/src/cpu/tests.rs).
//! A frame here is the whole path the window takes: the scene, the recorded
//! glyphs, the blend into the surface.

use super::*;

/// What a frame and a keystroke may cost, ms: the same as Windows'.
const FRAME_BUDGET_MS: f64 = 0.1;
const KEYSTROKE_BUDGET_MS: f64 = 0.3;

/// 4K: 1268 x 754 DIPs at 3 pixels each, as Windows measures.
const VIEW: (u32, u32, f32) = (3804, 2262, 3.0);

const BLOCK: &str = concat!(
    "## Section heading\n",
    "A paragraph with **bold**, *italic*, `code` and a [[Link]] that is long enough to wrap onto a second line in the column.\n",
    "- first item with ==marked== words\n",
    "- [ ] a task to do\n",
    "- [x] a task done\n",
    "> a quote, muted\n",
    "1. numbered\n",
    "\n",
);

/// `measure` (a median, ms) within `budget`. The Mac build host is shared and
/// often busy with other builds, so a slow run gets two more tries before it
/// fails: what is held is what the code costs, not what the neighbours do.
fn within(budget: f64, what: &str, measure: impl Fn() -> f64) {
    let mut tries = Vec::new();
    for _ in 0..3 {
        let ms = measure();
        if ms <= budget {
            return;
        }
        tries.push(format!("{ms:.3}"));
    }
    panic!("{what}: {} ms, over its {budget} ms budget", tries.join(", then "));
}

/// A vault of its own holding one note of `blocks` blocks (8 lines each).
fn long_vault(test: &str, blocks: usize) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("notsy-app-speed-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Long note.md"), format!("Long note\n{}", BLOCK.repeat(blocks))).unwrap();
    dir
}

fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[times.len() / 2]
}

fn ms(since: std::time::Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}

/// One frame at 4K with the caret moving through a 40-block note: the
/// median of 30, ms.
fn frame_ms() -> f64 {
    let dir = long_vault("frame", 40);
    let mut m = Model::new(dir.clone(), true);
    m.frame(VIEW.0, VIEW.1, VIEW.2);
    let mut times = Vec::new();
    for i in 0..30 {
        let caret = 200 + i * 7;
        m.editor.set_selection(Selection::caret(caret));
        let start = std::time::Instant::now();
        m.frame(VIEW.0, VIEW.1, VIEW.2);
        times.push(ms(start));
    }
    let _ = std::fs::remove_dir_all(dir);
    let m = median(times);
    println!("frame median: {m:.3} ms");
    m
}

/// One keystroke at 4K in a 6,000-line note: the editor's insert and the
/// frame it asks for, the median of 30 once the glyph cache is warm, ms.
/// `at_top` is under the title as the note opens; else mid-way down.
fn keystroke_ms(at_top: bool) -> f64 {
    let dir = long_vault("key", 750);
    let mut m = Model::new(dir.clone(), true);
    let line = m.editor.outline()[if at_top { 1 } else { 3001 }];
    m.editor.set_selection(Selection::caret(line.start));
    m.reveal = true;
    m.frame(VIEW.0, VIEW.1, VIEW.2);
    let (mut edit, mut total) = (Vec::new(), Vec::new());
    for _ in 0..40 {
        let start = std::time::Instant::now();
        m.insert("a");
        edit.push(ms(start));
        m.frame(VIEW.0, VIEW.1, VIEW.2);
        total.push(ms(start));
    }
    let _ = std::fs::remove_dir_all(dir);
    let (e, t) = (median(edit.split_off(10)), median(total.split_off(10)));
    println!("keystroke median ({}): edit {e:.3} ms, total {t:.3} ms", if at_top { "under the title" } else { "mid-way" });
    t
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

#[test]
#[ignore = "speed gate: the next few lines' spelling checked in a 6,000-line note, as a pause in typing allows"]
fn time_a_spell_step_in_a_long_note() {
    let dir = long_vault("spell", 750);
    let mut m = Model::new(dir.clone(), true);
    m.frame(VIEW.0, VIEW.1, VIEW.2);
    // Not the system's checker: what this times is the walk over the lines.
    let none = |_: &[u16]| Vec::new();
    while m.spell_step_with(none).1 {}
    let mut times = Vec::new();
    for _ in 0..30 {
        m.insert("a");
        let start = std::time::Instant::now();
        m.spell_step_with(none);
        times.push(ms(start));
    }
    let _ = std::fs::remove_dir_all(dir);
    let med = median(times);
    println!("spell step median (all other lines checked): {med:.3} ms");
    // Runs once in a quiet moment, not per key; it still must not be a stall.
    assert!(med <= 2.0, "a spell step over a long note: {med:.3} ms");
}

#[test]
#[ignore = "diagnostic: where a 4K frame's time goes (scene, record, paint)"]
fn frame_phases() {
    let dir = long_vault("phases", 40);
    let mut m = Model::new(dir.clone(), true);
    m.frame(VIEW.0, VIEW.1, VIEW.2);
    let (mut scene, mut record, mut paint) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..60 {
        m.editor.set_selection(Selection::caret(200 + i * 7));
        let t = std::time::Instant::now();
        let mut built = m.build(VIEW.0, VIEW.1, VIEW.2);
        scene.push(ms(t));
        let t = std::time::Instant::now();
        built.place_caret((0.0, 0.0));
        let list = display::record(&m.eng, &built, theme::for_mode(m.dark), VIEW.2).unwrap();
        record.push(ms(t));
        let t = std::time::Instant::now();
        m.surface.draw((VIEW.0, VIEW.1, 288), &list);
        paint.push(ms(t));
    }
    let t = std::time::Instant::now();
    for _ in 0..1000 {
        std::hint::black_box(m.date_line());
    }
    println!("the date line: {:.4} ms a frame", ms(t) / 1000.0);
    println!("frame phases (medians): scene {:.3} ms, record {:.3} ms, paint {:.3} ms", median(scene), median(record), median(paint));
    let _ = std::fs::remove_dir_all(dir);
}
