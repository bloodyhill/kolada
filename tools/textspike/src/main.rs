//! Phase 0 spike: does a portable shaper (cosmic-text: harfrust shaping,
//! unicode-bidi, fontdb fallback, swash rasterizing) hold Notsy's text
//! budget? Run in release on every OS:
//!
//!   cargo run --release -p textspike [-- out.png]
//!
//! It times what the product depends on: finding fonts at launch, laying out
//! a 6,000-line note, re-laying out one edited paragraph, and drawing a 4K
//! frame of glyphs; then checks Arabic joining, Hebrew, and emoji, and writes
//! a PNG to look at.

use std::time::Instant;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, LineEnding, Metrics, Shaping, SwashCache, SwashContent, Wrap, fontdb};

const SIZE: f32 = 16.0;
const LINE: f32 = 24.0;
const WIDTH: f32 = 720.0;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// A 6,000-line note: mostly English, with Arabic, Hebrew, CJK and emoji
/// mixed in the way real notes are.
fn note() -> String {
    let en = "The quick brown fox jumps over the lazy dog while the notes app keeps up with every keystroke and never makes the writer wait.";
    let ar = "السلام عليكم ورحمة الله وبركاته، هذه ملاحظة مكتوبة باللغة العربية لاختبار الربط والاتجاه من اليمين إلى اليسار.";
    let he = "שלום עולם, זו הערה בעברית כדי לבדוק כיוון כתיבה מימין לשמאל ואותיות ללא ניקוד.";
    let mixed = "Meeting at 10:30 مع الفريق about the الميزانية budget for Q3 🚀 and שלום to everyone ✅";
    let cjk = "今日は新しいノートアプリを試しています。速度がとても大切です。한국어도 지원합니다.";
    let emoji = "Done 🎉 shipped 👩‍💻 family 👨‍👩‍👧‍👦 flags 🇸🇦🇯🇵 heart ❤️";
    let mut s = String::new();
    for i in 0..6000 {
        match i % 12 {
            0 => s.push_str("# Heading number "),
            1 => s.push_str("- "),
            _ => {}
        }
        s.push_str(match i % 6 {
            0 | 1 => en,
            2 => ar,
            3 => mixed,
            4 => he,
            _ => {
                if i % 12 < 6 {
                    cjk
                } else {
                    emoji
                }
            }
        });
        s.push('\n');
    }
    s
}

fn new_buffer(fs: &mut FontSystem, text: &str, height: Option<f32>) -> Buffer {
    let mut b = Buffer::new(fs, Metrics::new(SIZE, LINE));
    b.set_wrap(Wrap::WordOrGlyph);
    b.set_size(Some(WIDTH), height);
    b.set_text(text, &Attrs::new().family(Family::SansSerif), Shaping::Advanced, None);
    b
}

/// Blend every visible glyph into an RGBA frame, the way the CPU renderer
/// does: one cached mask per glyph, alpha-blended.
fn draw(buf: &mut Buffer, fs: &mut FontSystem, cache: &mut SwashCache, frame: &mut [u8], fw: usize, fh: usize, ox: i32) -> usize {
    let mut n = 0;
    for run in buf.layout_runs() {
        for g in run.glyphs {
            let p = g.physical((ox as f32, run.line_y), 1.0);
            let Some(img) = cache.get_image(fs, p.cache_key).as_ref() else { continue };
            let (iw, ih) = (img.placement.width as i32, img.placement.height as i32);
            let (gx, gy) = (p.x + img.placement.left, p.y - img.placement.top + 0);
            n += 1;
            for yy in 0..ih {
                let y = gy + yy;
                if y < 0 || y >= fh as i32 {
                    continue;
                }
                for xx in 0..iw {
                    let x = gx + xx;
                    if x < 0 || x >= fw as i32 {
                        continue;
                    }
                    let o = (y as usize * fw + x as usize) * 4;
                    let (a, rgb): (u32, [u32; 3]) = match img.content {
                        SwashContent::Mask => (img.data[(yy * iw + xx) as usize] as u32, [235, 235, 240]),
                        SwashContent::Color => {
                            let i = ((yy * iw + xx) * 4) as usize;
                            (img.data[i + 3] as u32, [img.data[i] as u32, img.data[i + 1] as u32, img.data[i + 2] as u32])
                        }
                        SwashContent::SubpixelMask => (img.data[((yy * iw + xx) * 4 + 3) as usize] as u32, [235, 235, 240]),
                    };
                    for c in 0..3 {
                        let d = frame[o + c] as u32;
                        frame[o + c] = ((rgb[c] * a + d * (255 - a)) / 255) as u8;
                    }
                }
            }
        }
    }
    n
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "textspike.png".into());
    let text = note();
    println!("note: {} lines, {} KB", text.lines().count(), text.len() / 1024);

    // 1. Finding fonts. The product starts in ~26 ms in all, so this is
    // the number that decides whether system font discovery can be on the
    // launch path.
    // SPIKE_FONTS=a.ttf;b.ttf loads just those files (what a shipped app
    // would do) instead of scanning every font on the system.
    let t = Instant::now();
    let mut db = fontdb::Database::new();
    match std::env::var("SPIKE_FONTS") {
        Ok(list) => {
            for f in list.split(';').filter(|f| !f.is_empty()) {
                db.load_font_file(f).unwrap_or_else(|e| panic!("{f}: {e}"));
            }
            if let Ok(f) = std::env::var("SPIKE_SANS") {
                db.set_sans_serif_family(f);
            }
        }
        Err(_) => db.load_system_fonts(),
    }
    let scan = ms(t);
    let faces = db.faces().count();
    let t = Instant::now();
    let mut fs = FontSystem::new_with_locale_and_db("en-US".into(), db);
    println!("fonts: system scan {scan:.1} ms for {faces} faces; FontSystem {:.1} ms", ms(t));

    // 2. Visible-only layout: what launch pays (first screen).
    let t = Instant::now();
    let mut visible = new_buffer(&mut fs, &text, Some(1200.0));
    visible.shape_until_scroll(&mut fs, false);
    println!("first screen (1200 px of 6,000 lines): {:.1} ms", ms(t));

    // 3. The whole note laid out: the worst case, e.g. resize.
    let t = Instant::now();
    let mut full = new_buffer(&mut fs, &text, None);
    full.shape_until_scroll(&mut fs, false);
    let total = full.layout_runs().count();
    println!("whole note: {:.1} ms, {total} visual lines", ms(t));

    // 4. Width change: a window resize re-wraps everything shaped.
    let t = Instant::now();
    full.set_size(Some(WIDTH + 200.0), None);
    full.shape_until_scroll(&mut fs, false);
    println!("re-wrap whole note at a new width: {:.1} ms", ms(t));

    // 5. One edited paragraph, in the middle of the note: the keystroke.
    let mid = 3000;
    let mut best = f64::MAX;
    for k in 0..200 {
        let line = format!("{} typed{k}", full.lines[mid].text());
        let t = Instant::now();
        let attrs = full.lines[mid].attrs_list().clone();
        full.lines[mid].set_text(line, LineEnding::Lf, attrs);
        full.shape_until_scroll(&mut fs, false);
        let _ = full.layout_runs().count();
        best = best.min(ms(t));
    }
    println!("edit one line (best of 200): {best:.3} ms");

    // 6. Hit testing and caret: what the editor's Layout trait asks.
    let t = Instant::now();
    let mut hits = 0;
    for i in 0..1000 {
        if visible.hit((i % 700) as f32, ((i * 7) % 1100) as f32).is_some() {
            hits += 1;
        }
    }
    println!("hit test x1000 ({hits} found): {:.3} ms", ms(t));

    // 7. Draw a 4K frame's text: cold (rasterize each glyph) then warm.
    let (fw, fh) = (3840usize, 2160usize);
    let mut frame = vec![0u8; fw * fh * 4];
    for px in frame.chunks_mut(4) {
        px.copy_from_slice(&[17, 17, 19, 255]);
    }
    let mut cache = SwashCache::new();
    let mut wide = new_buffer(&mut fs, &text, Some(fh as f32));
    wide.shape_until_scroll(&mut fs, false);
    let t = Instant::now();
    let n = draw(&mut wide, &mut fs, &mut cache, &mut frame, fw, fh, 40);
    println!("4K frame, cold glyph cache: {:.1} ms ({n} glyphs)", ms(t));
    let mut best = f64::MAX;
    for _ in 0..20 {
        let t = Instant::now();
        draw(&mut wide, &mut fs, &mut cache, &mut frame, fw, fh, 40);
        best = best.min(ms(t));
    }
    println!("4K frame, warm cache (best of 20): {best:.2} ms  <- the renderer's own budget is 0.07 ms for damage only");

    // 8. Correctness: Arabic must join (fewer glyphs than letters is not
    // required, but lam-alef ligature and cluster counts are), Hebrew and
    // mixed text must come out in visual order, emoji must be coloured.
    let probe = |fs: &mut FontSystem, s: &str| {
        let mut b = new_buffer(fs, s, None);
        b.shape_until_scroll(fs, false);
        let run = b.layout_runs().next().map(|r| (r.rtl, r.glyphs.iter().map(|g| (g.start, g.end, g.level.number(), g.x as i32, g.glyph_id)).collect::<Vec<_>>()));
        run
    };
    let (rtl, g) = probe(&mut fs, "لا").unwrap();
    println!("lam-alef 'لا': rtl={rtl}, {} glyph(s) for 2 letters (1 = ligature) {:?}", g.len(), g);
    let (rtl, g) = probe(&mut fs, "مرحبا").unwrap();
    println!("'مرحبا': rtl={rtl}, {} glyphs for 5 letters, levels {:?}", g.len(), g.iter().map(|x| x.2).collect::<Vec<_>>());
    let (rtl, g) = probe(&mut fs, "abc שלום def").unwrap();
    println!("'abc שלום def': rtl={rtl}, x order of clusters {:?}", g.iter().map(|x| (x.0, x.3)).collect::<Vec<_>>());
    let mut cb = new_buffer(&mut fs, "🎉", None);
    cb.shape_until_scroll(&mut fs, false);
    let key = cb.layout_runs().next().and_then(|r| r.glyphs.first().map(|g| g.physical((0.0, 0.0), 1.0).cache_key));
    match key.and_then(|k| cache.get_image(&mut fs, k).as_ref().map(|i| (matches!(i.content, SwashContent::Color), i.placement.width, i.placement.height))) {
        Some((color, w, h)) => println!("emoji 🎉: colour={color} {w}x{h}"),
        None => println!("emoji 🎉: NO GLYPH IMAGE"),
    }

    // 9. Look at it: a small sample of the mixed note on graphite.
    let (pw, ph) = (900usize, 700usize);
    let mut pic = vec![0u8; pw * ph * 4];
    for px in pic.chunks_mut(4) {
        px.copy_from_slice(&[17, 17, 19, 255]);
    }
    let sample: String = text.lines().skip(2).take(24).collect::<Vec<_>>().join("\n");
    let mut sb = new_buffer(&mut fs, &sample, Some(ph as f32));
    sb.shape_until_scroll(&mut fs, false);
    draw(&mut sb, &mut fs, &mut cache, &mut pic, pw, ph, 20);
    let file = std::fs::File::create(&out).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), pw as u32, ph as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&pic).unwrap();
    println!("wrote {out}");
}
