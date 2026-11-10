//! The speed gate's timing tests for the model, on stand-in fonts (the Mac's
//! own are timed in `kolada-mac`'s `speed.rs`; tools/speed-gate-mac.sh runs
//! them with `--ignored`, in a release build): the same budgets, notes and
//! window as Windows' `time_a_frame`, `time_a_keystroke_in_a_long_note` and
//! `time_a_keystroke_under_the_title` (crates/kolada-win/src/cpu/tests.rs).
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
    long_vault_under(test, "", blocks)
}

/// The same, the note opening with `head` (frontmatter, say).
fn long_vault_under(test: &str, head: &str, blocks: usize) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kolada-app-speed-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Long note.md"), format!("{head}Long note\n{}", BLOCK.repeat(blocks))).unwrap();
    dir
}

/// A caught page's frontmatter, as the catcher writes it.
const CAUGHT: &str = "---\nsource: https://kitchen.example/lemon-tart\ncaught: 2026-11-08T15:20\n---\n";

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
    keystroke_under_ms(at_top, "")
}

/// The same, the note opening with `head`.
fn keystroke_under_ms(at_top: bool, head: &str) -> f64 {
    keystroke_shown_ms(at_top, head, false).1
}

/// The same, with Show Markdown on (`raw`): the frontmatter's lines as
/// written, as every note showed them before they folded. The edit's
/// median and the whole keystroke's (the edit and its frame), ms.
fn keystroke_shown_ms(at_top: bool, head: &str, raw: bool) -> (f64, f64) {
    let dir = long_vault_under(if head.is_empty() { "key" } else if raw { "key-raw" } else { "key-front" }, head, 750);
    let mut m = Model::new(dir.clone(), true);
    if raw {
        m.session.show_marks = true;
        m.editor.set_clean(false);
    }
    let first = head.lines().count();
    let line = m.editor.outline()[first + if at_top { 1 } else { 3001 }];
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
    let front = match (head.lines().count(), raw) {
        (0, _) => String::new(),
        (n, false) => format!(", {n} lines of frontmatter folded above"),
        (n, true) => format!(", {n} lines of frontmatter as written above"),
    };
    println!("keystroke median ({}{front}): edit {e:.3} ms, total {t:.3} ms", if at_top { "under the title" } else { "mid-way" });
    (e, t)
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

/// The same under a caught page's title: its frontmatter folded to one line
/// above, kept from frame to frame, not made again at each key.
#[test]
#[ignore = "speed gate: a keystroke under a caught 6,000-line note's title, its frontmatter folded (release build)"]
fn time_a_keystroke_under_a_caught_title() {
    within(KEYSTROKE_BUDGET_MS, "a keystroke under a caught title", || keystroke_under_ms(true, CAUGHT));
}

/// Frontmatter `lines` long (Obsidian properties, one a line), as a note
/// opens with it.
fn long_front(lines: usize) -> String {
    let props: String = (0..lines).map(|i| format!("property{i}: value number {i}\n")).collect();
    format!("---\n{props}---\n")
}

/// A keystroke under the title of a note whose frontmatter is 1,000 lines
/// long, folded: its lines are passed at once, not each one measured, at
/// every frame.
#[test]
#[ignore = "speed gate: a keystroke under a title with 1,000 lines of frontmatter folded above (release build)"]
fn time_a_keystroke_under_a_long_frontmatter() {
    let head = long_front(1_000);
    within(KEYSTROKE_BUDGET_MS, "a keystroke under 1,000 lines of frontmatter", || keystroke_under_ms(true, &head));
}

/// 3,000 lines of it, folded, cost about what they did before they
/// folded: the edit about what it costs with them as written (Show
/// Markdown on; the editor reads so long a frontmatter either way), and the
/// frame about what a note with none costs (folded, its lines are passed at
/// once). Each within a twentieth of a millisecond.
#[test]
#[ignore = "speed gate: a keystroke under 3,000 lines of frontmatter folded costs about what it did as written (release build)"]
fn time_a_keystroke_under_a_very_long_frontmatter() {
    let head = long_front(3_000);
    let mut tries = Vec::new();
    for _ in 0..3 {
        let ((edit, total), (written, _), (_, none)) = (keystroke_shown_ms(true, &head, false), keystroke_shown_ms(true, &head, true), keystroke_shown_ms(true, "", false));
        let (frame, plain) = (total - edit, none);
        if edit <= written + 0.05 && frame <= plain + 0.05 {
            return;
        }
        tries.push(format!("edit {edit:.3} (as written {written:.3}), frame {frame:.3} (a note with none, edit and frame {plain:.3})"));
    }
    panic!("a keystroke under 3,000 lines of frontmatter: {}", tries.join("; then "));
}

/// The system's lists as a busy user has them: 500 text replacements, and
/// a checker that has no correction (it runs on its own thread, so what a
/// keystroke pays is the lookup and handing the word over).
struct Lists;

impl crate::services::Spell for Lists {
    fn check(&self, _: &[u16]) -> Vec<(usize, usize)> {
        Vec::new()
    }
    fn learn(&self, _: &str) {}
    fn suggest(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
    fn replacements(&self) -> Vec<(String, String)> {
        (0..500).map(|i| (format!("short{i}"), format!("A longer phrase number {i}"))).collect()
    }
    fn correct(&self, _: &str) -> Option<String> {
        None
    }
}

/// A space that ends a word, at 4K mid-way down a 6,000-line note, with
/// the system's text replacements and spelling corrections on: the word's
/// lookup, the hand-over to the checker's thread, the frame. The median of
/// 30, ms.
fn word_end_ms() -> f64 {
    let dir = long_vault("word-end", 750);
    let mut m = Model::with_services(crate::services::Services { spell: std::sync::Arc::new(Lists), ..crate::services::Services::none() }, dir.clone(), true);
    m.read_replacements();
    m.start_arrivals(|| {});
    let line = m.editor.outline()[3001];
    m.editor.set_selection(Selection::caret(line.start));
    m.reveal = true;
    m.frame(VIEW.0, VIEW.1, VIEW.2);
    let mut times = Vec::new();
    for _ in 0..40 {
        m.insert("w");
        m.insert("o");
        m.insert("r");
        m.frame(VIEW.0, VIEW.1, VIEW.2);
        let start = std::time::Instant::now();
        m.insert(" ");
        m.frame(VIEW.0, VIEW.1, VIEW.2);
        times.push(ms(start));
    }
    let _ = std::fs::remove_dir_all(dir);
    let t = median(times.split_off(10));
    println!("keystroke median (a space ending a word, replacements and corrections on): {t:.3} ms");
    t
}

#[test]
#[ignore = "speed gate: a space ending a word, with the system's replacements and corrections on, at 4K (release build)"]
fn time_a_keystroke_that_ends_a_word() {
    within(KEYSTROKE_BUDGET_MS, "a keystroke that ends a word", word_end_ms);
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
