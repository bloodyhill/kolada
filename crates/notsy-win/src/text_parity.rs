//! notsy-text, the portable engine, against DirectWrite, which draws the
//! note today: the same paragraph in the same font wraps at the same words
//! and puts its carets where DirectWrite does, to within a pixel. This is
//! the test that lets the Windows app move to the shared engine.

use notsy_text::{Engine, FontSet, Format, Layout, Role};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_HIT_TEST_METRICS,
    DWRITE_LINE_METRICS, DWRITE_LINE_SPACING_METHOD_UNIFORM, DWriteCreateFactory, IDWriteFactory,
};
use windows::core::w;

const SIZE: f32 = 16.0;
const LINE: f32 = 24.0;

fn engine() -> Engine {
    let f = |n: &str| std::path::PathBuf::from(r"C:\Windows\Fonts").join(n);
    Engine::new(FontSet { files: vec![f("arial.ttf")], fallback: Vec::new(), data: Vec::new(), ui: "Arial".into(), text: "Arial".into(), code: "Arial".into(), icon: "Arial".into() })
}

/// DirectWrite's lines (start byte, length) and caret x at every byte of
/// ASCII `text`.
fn directwrite(text: &str, width: f32) -> (Vec<(usize, usize)>, Vec<f32>) {
    unsafe {
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
        let format = dw
            .CreateTextFormat(w!("Arial"), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, SIZE, w!("en-us"))
            .unwrap();
        format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, LINE, LINE * 0.8).unwrap();
        let wide: Vec<u16> = text.encode_utf16().collect();
        let layout = dw.CreateTextLayout(&wide, &format, width, 4000.0).unwrap();
        let mut m = vec![DWRITE_LINE_METRICS::default(); 64];
        let mut n = 0;
        layout.GetLineMetrics(Some(&mut m), &mut n).unwrap();
        let mut start = 0;
        let lines = m[..n as usize]
            .iter()
            .map(|l| {
                let r = (start, l.length as usize);
                start += l.length as usize;
                r
            })
            .collect();
        let carets = (0..=text.len())
            .map(|i| {
                let (mut x, mut y) = (0.0, 0.0);
                let mut hm = DWRITE_HIT_TEST_METRICS::default();
                layout.HitTestTextPosition(i as u32, false, &mut x, &mut y, &mut hm).unwrap();
                x
            })
            .collect();
        (lines, carets)
    }
}

/// DirectWrite's widest line width counting trailing spaces, and where the
/// first line's baseline is (uniform spacing puts it at 0.8 of the line).
fn directwrite_metrics(text: &str, width: f32) -> (f32, f32) {
    use windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_METRICS;
    unsafe {
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
        let format = dw
            .CreateTextFormat(w!("Arial"), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, SIZE, w!("en-us"))
            .unwrap();
        format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, LINE, LINE * 0.8).unwrap();
        let wide: Vec<u16> = text.encode_utf16().collect();
        let layout = dw.CreateTextLayout(&wide, &format, width, 4000.0).unwrap();
        let mut m = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut m).unwrap();
        (m.widthIncludingTrailingWhitespace, m.height)
    }
}

const PARAGRAPHS: [&str; 5] = [
    "The quick brown fox jumps over the lazy dog while the notes app keeps up with every keystroke and never makes the writer wait.",
    "Meeting notes: budget review, hiring plan, and the roadmap for the next quarter, including everything that slipped from this one.",
    "A short line.",
    "Averyveryverylongwordthatcannotpossiblyfitononelineofanormalwidthbutmustbebrokenanyway and then some more words after it.",
    "Numbers 1,234.56 and punctuation; (parentheses), \"quotes\", dashes - and slashes/backslashes all wrap somewhere sensible.",
];

#[test]
fn wraps_where_directwrite_wraps() {
    let mut e = engine();
    for text in PARAGRAPHS {
        for width in [140.0, 220.0, 360.0, 640.0] {
            let (dw_lines, _) = directwrite(text, width);
            let l = Layout::new(&mut e, text, &Format { line: Some(LINE), ..Format::new(Role::Text, SIZE) }, width, 4000.0, &[]);
            let ours: Vec<(usize, usize)> = l.lines().iter().map(|v| (v.start, v.end - v.start)).collect();
            assert_eq!(ours, dw_lines, "width {width}: {text}");
            assert_eq!(l.measure().height, dw_lines.len() as f32 * LINE);
        }
    }
}

#[test]
fn puts_carets_where_directwrite_puts_them() {
    let mut e = engine();
    let mut worst = 0.0f32;
    for text in PARAGRAPHS {
        for width in [220.0, 640.0] {
            let (dw_lines, dw_x) = directwrite(text, width);
            let l = Layout::new(&mut e, text, &Format { line: Some(LINE), ..Format::new(Role::Text, SIZE) }, width, 4000.0, &[]);
            for i in 0..=text.len() {
                // A caret at a wrap belongs to the line below in both; skip
                // the hanging space itself, where DirectWrite keeps the caret
                // after the space and we keep it at the word's end.
                if dw_lines.iter().any(|&(s, len)| i + 1 == s + len && s + len < text.len() && text.as_bytes()[i] == b' ') {
                    continue;
                }
                let d = (l.caret(i).x - dw_x[i]).abs();
                worst = worst.max(d);
                assert!(d < 1.0, "caret {i} at width {width}: ours {} theirs {} in {text}", l.caret(i).x, dw_x[i]);
            }
        }
    }
    eprintln!("worst caret difference: {worst:.3} px");
}

#[test]
fn measures_width_with_trailing_spaces_and_height_like_directwrite() {
    let mut e = engine();
    let one_line = Format { line: Some(LINE), wrap: false, ..Format::new(Role::Text, SIZE) };
    for text in ["Today", "Dates  ", "  indented and trailing   ", "Mixed 123 words, with punctuation. "] {
        let (w, h) = directwrite_metrics(text, 1000.0);
        let m = Layout::new(&mut e, text, &one_line, 1000.0, 100.0, &[]).measure();
        assert!((m.width - w).abs() < 1.0, "{text:?}: ours {} theirs {w}", m.width);
        assert_eq!(m.height, h, "{text:?}");
    }
}

#[test]
fn sets_the_baseline_where_directwrite_does() {
    let mut e = engine();
    let f = Format { line: Some(LINE), baseline: Some(0.8), ..Format::new(Role::Text, SIZE) };
    let l = Layout::new(&mut e, "Hxg", &f, 300.0, 100.0, &[]);
    // DirectWrite's baseline for a uniformly spaced line is 0.8 of its height.
    assert!((l.glyphs((0.0, 0.0), 1.0)[0].y as f32 - LINE * 0.8).abs() <= 1.0);
}
