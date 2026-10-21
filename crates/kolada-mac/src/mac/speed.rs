//! The speed gate's timing tests that need the Mac's own fonts (the system's
//! Helvetica Neue, Menlo, the icon font and the fallbacks): the same budgets,
//! note and 4K window as `kolada-app`'s `model/speed.rs` (which times the model
//! on stand-in fonts and the private paths), through the model's public calls.
//! tools/speed-gate-mac.sh runs them with `--ignored`, in a release build.

use kolada_app::Model;
use kolada_editor::Motion;

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

/// `measure` (a median, ms) within `budget`, with two more tries on a busy host.
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

fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[times.len() / 2]
}

fn ms(since: std::time::Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}

/// A model on the Mac's services over a vault of one note of `blocks` blocks.
fn model(test: &str, blocks: usize) -> (Model, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("kolada-mac-speed-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Long note.md"), format!("Long note\n{}", BLOCK.repeat(blocks))).unwrap();
    let mut m = Model::with_services(super::services::services(), dir.clone(), true);
    m.frame(VIEW.0, VIEW.1, VIEW.2);
    (m, dir)
}

fn frame_ms() -> f64 {
    let (mut m, dir) = model("frame", 40);
    let mut times = Vec::new();
    for _ in 0..30 {
        for _ in 0..7 {
            m.go(Motion::Right, false);
        }
        let start = std::time::Instant::now();
        m.frame(VIEW.0, VIEW.1, VIEW.2);
        times.push(ms(start));
    }
    let _ = std::fs::remove_dir_all(dir);
    let med = median(times);
    println!("frame median (Mac fonts): {med:.3} ms");
    med
}

fn keystroke_ms() -> f64 {
    let (mut m, dir) = model("key", 750);
    m.go(Motion::DocStart, false);
    m.go(Motion::Down, false);
    m.frame(VIEW.0, VIEW.1, VIEW.2);
    let mut total = Vec::new();
    for _ in 0..40 {
        let start = std::time::Instant::now();
        m.insert("a");
        m.frame(VIEW.0, VIEW.1, VIEW.2);
        total.push(ms(start));
    }
    let _ = std::fs::remove_dir_all(dir);
    let med = median(total.split_off(10));
    println!("keystroke median (Mac fonts, under the title): total {med:.3} ms");
    med
}

/// A keystroke as far as the screen: the edit, its frame, and its pixels
/// put in a 4K context as AppKit's `drawRect:` gets it, clipped to the
/// rectangles the frame changed (`view/blit.rs`). Prints what putting the
/// whole frame there costs too, as the Mac did before.
fn keystroke_on_screen_ms() -> f64 {
    use super::view::blit;
    let (mut m, dir) = model("screen", 750);
    let size = blit::Size { w: VIEW.0, h: VIEW.1, scale: VIEW.2 };
    let mut screen = vec![0u32; (size.w * size.h) as usize];
    let ctx = blit::flipped_context(&mut screen, size);
    m.go(Motion::DocStart, false);
    m.go(Motion::Down, false);
    m.frame(VIEW.0, VIEW.1, VIEW.2);
    blit::present_in(&ctx, m.surface().pixels(), size, None);
    let (mut parts, mut whole) = (Vec::new(), Vec::new());
    for _ in 0..40 {
        let start = std::time::Instant::now();
        m.insert("a");
        m.frame(VIEW.0, VIEW.1, VIEW.2);
        let rects = blit::points(m.surface().last_damage.as_deref(), size);
        blit::present_in(&ctx, m.surface().pixels(), size, rects.as_deref());
        parts.push(ms(start));
    }
    // Putting the whole frame there each time, as before: in a run of its
    // own, so the first run's caches are not spoilt by it.
    for _ in 0..20 {
        let start = std::time::Instant::now();
        m.insert("a");
        m.frame(VIEW.0, VIEW.1, VIEW.2);
        blit::present_in(&ctx, m.surface().pixels(), size, None);
        whole.push(ms(start));
    }
    drop(ctx);
    let _ = std::fs::remove_dir_all(dir);
    let med = median(parts.split_off(10));
    println!("keystroke on the screen median (Mac fonts, 4K): {med:.3} ms; with the whole frame put there instead: {:.3} ms", median(whole.split_off(5)));
    med
}

#[test]
#[ignore = "speed gate: a keystroke as far as the screen at 4K, only what changed drawn, on the Mac's fonts (release build)"]
fn time_a_keystroke_on_the_screen_on_the_macs_fonts() {
    within(KEYSTROKE_BUDGET_MS, "a keystroke on the screen", keystroke_on_screen_ms);
}

#[test]
#[ignore = "speed gate: one frame at 4K within its budget, on the Mac's fonts (release build)"]
fn time_a_frame_on_the_macs_fonts() {
    within(FRAME_BUDGET_MS, "a frame at 4K", frame_ms);
}

#[test]
#[ignore = "speed gate: a keystroke in a 6,000-line note at 4K, on the Mac's fonts (release build)"]
fn time_a_keystroke_on_the_macs_fonts() {
    within(KEYSTROKE_BUDGET_MS, "a keystroke in a long note", keystroke_ms);
}
