//! CPU rendering into a plain pixel buffer, which each shell puts on screen.
//!
//! Why the CPU: no GPU driver is loaded at startup. Creating a D3D11 device
//! alone costs 70-90 ms on the benchmark laptop, more than the whole budget.
//!
//! How:
//! - every distinct glyph (font face, glyph, pixel size, quarter-pixel
//!   offset) is rasterized once by the text engine into a coverage mask (or
//!   a colour picture, for emoji), and kept for later frames;
//! - glyph instances and fills are blended into the buffer here, in
//!   horizontal bands on several threads;
//! - worker threads never call GDI or USER. A thread that does becomes a
//!   kernel "GUI thread", and those contend with window creation on the main
//!   thread: CreateWindowEx measured 2-3 ms alone, 7 ms next to GDI workers;
//! - text comes in runs (one per laid-out text, see `display::Run`), and a
//!   run is placed in pixels once: while its id is drawn, its glyphs' masks
//!   and places are kept, relative to the pixel its origin falls in, so a
//!   frame that draws it again, or scrolls it, only moves it;
//! - a frame repaints only what changed. Each frame keeps what it drew (a
//!   hash of every shape, picture and run, and the pixels it touched, by
//!   layer); the next one compares its own against it and clears, paints and
//!   copies only where they differ. A run redrawn where it was (a paragraph
//!   typed in) differs only where its glyphs do: a keystroke repaints the
//!   rest of its line and the caret, not a 4K window.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::thread;

use notsy_text::{GlyphKey, Mask};

use crate::display::{DisplayList, Glyph, Run, Shape, mix};

/// Threads per frame, for blending bands.
pub const THREADS: usize = 4;
/// How far a pulled pill's far edge swells, of how far the string reaches.
const SWELL: f32 = 0.75;
/// Coverage contrast boost, so thin strokes keep their weight when blended
/// in gamma space.
const CONTRAST: f32 = 0.35;

/// A fast hasher for keys made of a few integers (glyph keys, run ids). It
/// does not resist chosen collisions; nothing here needs it to.
#[derive(Default, Clone, Copy)]
struct Fx(u64);

impl std::hash::Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_u8(&mut self, v: u8) {
        self.write_u64(v as u64);
    }
    fn write_u16(&mut self, v: u16) {
        self.write_u64(v as u64);
    }
    fn write_u32(&mut self, v: u32) {
        self.write_u64(v as u64);
    }
    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517C_C1B7_2722_0A95);
    }
}

type FxMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<Fx>>;

/// One glyph of a placed run: its mask (in `GlyphCache::masks`), where its
/// origin lands (pixels, from the run's), its color and opacity.
#[derive(Clone, Copy)]
struct PxGlyph {
    mask: u32,
    x: i32,
    y: i32,
    color: u32,
    alpha: u8,
}

/// A run placed in pixels, relative to the whole pixel its origin falls in:
/// its glyphs with any coverage, the pixels they can touch, and the frame it
/// was last drawn in.
struct PxRun {
    glyphs: Vec<PxGlyph>,
    bounds: Option<PxRect>,
    used: u64,
}

/// The whole pixel `origin` (DIPs) falls in, and where in it (0..1).
fn pixel_of(origin: (f32, f32), scale: f32) -> ((i32, i32), (f32, f32)) {
    let (px, py) = (origin.0 * scale, origin.1 * scale);
    let (ix, iy) = (px.floor(), py.floor());
    ((ix as i32, iy as i32), (px - ix, py - iy))
}

/// Glyph pictures, kept across frames: each found by its key.
#[derive(Default)]
struct GlyphCache {
    masks: Vec<Arc<Mask>>,
    index: FxMap<GlyphKey, u32>,
}

impl GlyphCache {
    /// Where `glyph`'s picture is kept, and whether it was not yet.
    fn intern(&mut self, glyph: &Glyph) -> (u32, bool) {
        if let Some(&i) = self.index.get(&glyph.key) {
            return (i, false);
        }
        let i = self.masks.len() as u32;
        self.masks.push(glyph.mask.clone());
        self.index.insert(glyph.key, i);
        (i, true)
    }
}

/// Maps coverage to blend weight, boosting mid-tones like DirectWrite's
/// "enhanced contrast" so strokes keep their weight.
fn contrast_table(k: f32) -> [u8; 256] {
    let mut t = [0u8; 256];
    for (a, v) in t.iter_mut().enumerate() {
        let x = a as f32 / 255.0;
        *v = (255.0 * x * (1.0 + k) / (1.0 + k * x)).round() as u8;
    }
    t
}

/// `src` over `dst` with weight `a` (0..=255), for 0x00RRGGBB pixels.
#[inline]
fn blend(dst: u32, src: u32, a: u32) -> u32 {
    let inv = 255 - a;
    let rb = (src & 0x00FF_00FF) * a + (dst & 0x00FF_00FF) * inv;
    let g = (src & 0x0000_FF00) * a + (dst & 0x0000_FF00) * inv;
    // Divide each channel by 255, rounded.
    let rb = ((rb + 0x0080_0080 + ((rb >> 8) & 0x00FF_00FF)) >> 8) & 0x00FF_00FF;
    let g = ((g + 0x0000_8000 + ((g >> 8) & 0x0000_FF00)) >> 8) & 0x0000_FF00;
    rb | g
}

/// A shape in pixels. Edges snap to whole pixels, like text does.
#[derive(Clone, Copy, Debug)]
struct PxShape {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
    radius: f32,
    blur: f32,
    color: u32,
    alpha: f32,
    ring: bool,
    /// Circle melted in with a smooth union: (x, y, radius, neck), pixels.
    blob: Option<(f32, f32, f32, f32)>,
    /// A string's wave: running along y if vertical, its middle line
    /// displaced by these (pixels).
    wave: Option<(bool, [f32; crate::wave::SAMPLES])>,
}

impl PxShape {
    fn new(s: &Shape, scale: f32) -> PxShape {
        let px = |v: f32| (v * scale).round();
        let (l, t, r, b) = (px(s.rect.left), px(s.rect.top), px(s.rect.right), px(s.rect.bottom));
        let radius = (s.radius * scale).min((r - l) * 0.5).min((b - t) * 0.5).max(0.0);
        let blob = s.blob.map(|b| (b.x * scale, b.y * scale, b.radius * scale, b.smooth * scale));
        let wave = s.wave.map(|w| (w.vertical, w.d.map(|d| d * scale)));
        let alpha = s.alpha.clamp(0.0, 1.0);
        PxShape { l, t, r, b, radius, blur: s.blur * scale, color: s.color, alpha, ring: s.ring, blob, wave }
    }

    /// A wave pill's coverage. The edge along the window's stays straight;
    /// the other swells out where the string is pulled, as something soft
    /// drawn out by a magnet. Round ends.
    fn wave_coverage(&self, vertical: bool, d: &[f32; crate::wave::SAMPLES], x: f32, y: f32) -> f32 {
        let (along, across, lo, hi, rest, width) = if vertical {
            (y, x, self.t, self.b, (self.l + self.r) * 0.5, self.r - self.l)
        } else {
            (x, y, self.l, self.r, (self.t + self.b) * 0.5, self.b - self.t)
        };
        if along < lo || along > hi {
            return 0.0;
        }
        let n = crate::wave::SAMPLES - 1;
        let s = (along - lo) / (hi - lo).max(1.0) * n as f32;
        let i = (s.floor() as usize).min(n - 1);
        let offset = d[i] + (d[i + 1] - d[i]) * (s - i as f32);
        // The window's side (top, or left) holds; the far side follows the
        // pull, never thinner than half the pill.
        let near = rest - width * 0.5;
        let far = (rest + width * 0.5 + SWELL * offset).max(near + width * 0.5);
        let (middle, mut half) = ((near + far) * 0.5, (far - near) * 0.5);
        let end = (along - lo).min(hi - along);
        if end < half {
            half = (half * half - (half - end) * (half - end)).max(0.0).sqrt();
        }
        let dist = (across - middle).abs() - half;
        (0.5 - dist).clamp(0.0, 1.0)
    }

    /// The pixel area the shape can touch, before clipping.
    fn bounds(&self) -> (f32, f32, f32, f32) {
        let reach = self.blur * 0.5;
        let (mut l, mut t, mut r, mut b) = (self.l - reach, self.t - reach, self.r + reach, self.b + reach);
        if let Some((x, y, radius, neck)) = self.blob {
            let e = radius + neck + reach;
            (l, t, r, b) = (l.min(x - e), t.min(y - e), r.max(x + e), b.max(y + e));
        }
        if let Some((vertical, d)) = &self.wave {
            let (lo, hi) = d.iter().fold((0.0f32, 0.0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
            let swell = (self.r - self.l).min(self.b - self.t);
            if *vertical {
                (l, r) = (l + lo - swell, r + hi + swell);
            } else {
                (t, b) = (t + lo - swell, b + hi + swell);
            }
        }
        (l, t, r, b)
    }

    /// Coverage (0..1) of the pixel centered at (x, y), from its signed
    /// distance to the shape: a 1-pixel ramp for crisp shapes, a smooth
    /// fade over `blur` pixels for shadows.
    fn coverage(&self, x: f32, y: f32) -> f32 {
        if let Some((vertical, d)) = &self.wave {
            return self.wave_coverage(*vertical, d, x, y);
        }
        let (hw, hh) = ((self.r - self.l) * 0.5, (self.b - self.t) * 0.5);
        let qx = (x - (self.l + hw)).abs() - (hw - self.radius);
        let qy = (y - (self.t + hh)).abs() - (hh - self.radius);
        let mut d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - self.radius;
        if let Some((bx, by, radius, neck)) = self.blob {
            // Polynomial smooth minimum: the circle and the rectangle merge
            // through a neck `neck` pixels wide instead of meeting at a crease.
            let c = (x - bx).hypot(y - by) - radius;
            let h = (0.5 + 0.5 * (c - d) / neck).clamp(0.0, 1.0);
            d = c + (d - c) * h - neck * h * (1.0 - h);
        }
        if self.blur <= 0.0 {
            (0.5 - d).clamp(0.0, 1.0)
        } else {
            let u = ((d + self.blur * 0.5) / self.blur).clamp(0.0, 1.0);
            1.0 - u * u * (3.0 - 2.0 * u)
        }
    }
}

/// One horizontal band of the buffer: `rows` starts at row `top`.
struct Band<'a> {
    rows: &'a mut [u32],
    top: i32,
    width: i32,
    /// Pixels outside this (left, top, right, bottom; whole surface) are
    /// left alone.
    clip: Option<(i32, i32, i32, i32)>,
}

impl Band<'_> {
    fn height(&self) -> i32 {
        (self.rows.len() / self.width as usize) as i32
    }

    /// Blends `color` over `pixels` with weight `w` (0..=255).
    fn blend_span(pixels: &mut [u32], color: u32, w: u32) {
        if w >= 255 {
            pixels.fill(color);
        } else if w > 0 {
            for p in pixels {
                *p = blend(*p, color, w);
            }
        }
    }

    /// Draws a rounded rectangle. Each row has an interior span whose
    /// coverage only depends on the row, so it is filled at once; only the
    /// pixels near the row's two ends get the exact per-pixel test.
    fn shape(&mut self, s: &PxShape) {
        if s.r <= s.l || s.b <= s.t || s.alpha <= 0.0 {
            return;
        }
        let reach = s.blur * 0.5;
        let (bl, bt, br, bb) = s.bounds();
        let (cl, ct, cr, cb) = self.clip.unwrap_or((0, i32::MIN, i32::MAX, i32::MAX));
        let x0 = (bl.floor() as i32).max(0).max(cl);
        let x1 = (br.ceil() as i32).min(self.width).min(cr);
        let y0 = (bt.floor() as i32).max(self.top).max(ct);
        let y1 = (bb.ceil() as i32).min(self.top + self.height()).min(cb);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let alpha = s.alpha * 255.0;
        let weight = |c: f32| (c * alpha).round() as u32;
        if s.blob.is_some() || s.wave.is_some() {
            // Melted and bent shapes are small (pills): test every pixel.
            for y in y0..y1 {
                let row = ((y - self.top) * self.width) as usize;
                for x in x0..x1 {
                    let w = weight(s.coverage(x as f32 + 0.5, y as f32 + 0.5));
                    let p = &mut self.rows[row + x as usize];
                    *p = if w >= 255 { s.color } else if w > 0 { blend(*p, s.color, w) } else { *p };
                }
            }
            return;
        }
        let mid_x = (s.l + s.r) * 0.5;
        for y in y0..y1 {
            let cy = y as f32 + 0.5;
            // Rows level with a corner need the exact test further in.
            let straight = cy >= s.t + s.radius + reach && cy <= s.b - s.radius - reach;
            let edge = if straight { reach } else { s.radius + reach } + 1.0;
            let span_l = ((s.l + edge).ceil() as i32).clamp(x0, x1);
            let span_r = ((s.r - edge).floor() as i32).clamp(span_l, x1);
            let row = ((y - self.top) * self.width) as usize;
            let pixels = &mut self.rows[row..row + self.width as usize];

            let mid = s.coverage(mid_x, cy);
            if !(s.ring && mid >= 1.0) {
                Band::blend_span(&mut pixels[span_l as usize..span_r as usize], s.color, weight(mid));
            }
            for x in (x0..span_l).chain(span_r..x1) {
                let w = weight(s.coverage(x as f32 + 0.5, cy));
                let p = &mut pixels[x as usize];
                *p = if w >= 255 { s.color } else if w > 0 { blend(*p, s.color, w) } else { *p };
            }
        }
    }

    /// Copies a picture over `(l, t, r, b)` (pixels), sampling its pixels
    /// (premultiplied) to fit, at opacity `alpha`, its corners rounded by
    /// `round` pixels.
    fn picture(&mut self, (l, t, r, b): (i32, i32, i32, i32), p: &crate::pixels::Pixels, alpha: f32, round: i32) {
        let (cl, ct, cr, cb) = self.clip.unwrap_or((0, i32::MIN, i32::MAX, i32::MAX));
        let x0 = l.max(0).max(cl);
        let x1 = r.min(self.width).min(cr);
        let y0 = t.max(self.top).max(ct);
        let y1 = b.min(self.top + self.height()).min(cb);
        if x0 >= x1 || y0 >= y1 || p.width == 0 || p.height == 0 {
            return;
        }
        let (dw, dh) = ((r - l).max(1) as i64, (b - t).max(1) as i64);
        let alpha = alpha.clamp(0.0, 1.0);
        let round = round.clamp(0, ((r - l).min(b - t)) / 2);
        let rf = round as f32;
        for y in y0..y1 {
            let sy = (((y - t) as i64 * p.height as i64) / dh).clamp(0, p.height as i64 - 1) as usize;
            let src = &p.data[sy * p.width as usize..][..p.width as usize];
            let row = ((y - self.top) * self.width) as usize;
            // The corner's center, in rows where a corner is.
            let cy = if y < t + round {
                Some((t + round) as f32)
            } else if y >= b - round {
                Some((b - round) as f32)
            } else {
                None
            };
            for x in x0..x1 {
                let cover = match cy {
                    Some(cy) if x < l + round || x >= r - round => {
                        let cx = if x < l + round { (l + round) as f32 } else { (r - round) as f32 };
                        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                        (rf - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0)
                    }
                    _ => 1.0,
                };
                let fade = (alpha * cover * 255.0).round() as u32;
                let sx = (((x - l) as i64 * p.width as i64) / dw).clamp(0, p.width as i64 - 1) as usize;
                let s = src[sx];
                let a = (s >> 24) * fade / 255;
                if a == 0 {
                    continue;
                }
                let d = &mut self.rows[row + x as usize];
                if a >= 255 {
                    *d = s & 0x00FF_FFFF;
                    continue;
                }
                // Premultiplied over: out = src * fade + dst * (1 - a).
                let ch = |v: u32, shift: u32| (((v >> shift) & 0xFF) * fade / 255 + ((*d >> shift) & 0xFF) * (255 - a) / 255).min(255);
                *d = ch(s, 16) << 16 | ch(s, 8) << 8 | ch(s, 0);
            }
        }
    }

    /// Blends a glyph's picture with its origin at `(x, y)` (pixels): its
    /// coverage in the glyph's color, or, for colour glyphs, the picture
    /// itself (premultiplied RGBA).
    fn glyph(&mut self, m: &Mask, (x, y): (i32, i32), p: &PxGlyph, table: &[u8; 256]) {
        let (gx, gy) = (x + m.left, y - m.top - self.top);
        let (cl, ct, cr, cb) = self.clip.unwrap_or((0, i32::MIN, i32::MAX, i32::MAX));
        let (mw, mh) = (m.width as i32, m.height as i32);
        let x0 = gx.max(0).max(cl);
        let x1 = (gx + mw).min(self.width).min(cr);
        let y0 = gy.max(0).max(ct.saturating_sub(self.top));
        let y1 = (gy + mh).min(self.height()).min(cb.saturating_sub(self.top));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let fade = p.alpha as u32;
        if m.color {
            for y in y0..y1 {
                let src = &m.data[(y - gy) as usize * m.width as usize * 4..][..m.width as usize * 4];
                let row = (y * self.width) as usize;
                for x in x0..x1 {
                    let s = &src[(x - gx) as usize * 4..][..4];
                    let a = s[3] as u32 * fade / 255;
                    if a == 0 {
                        continue;
                    }
                    let d = &mut self.rows[row + x as usize];
                    // Premultiplied over: out = src * fade + dst * (1 - a).
                    let ch = |v: u8, shift: u32| ((v as u32 * fade / 255) + ((*d >> shift) & 0xFF) * (255 - a) / 255).min(255);
                    *d = ch(s[0], 16) << 16 | ch(s[1], 8) << 8 | ch(s[2], 0);
                }
            }
            return;
        }
        for y in y0..y1 {
            let src = &m.data[(y - gy) as usize * m.width as usize..][..m.width as usize];
            let row = (y * self.width) as usize;
            let dst = &mut self.rows[row + x0 as usize..row + x1 as usize];
            let src = &src[(x0 - gx) as usize..(x1 - gx) as usize];
            for (d, &a) in dst.iter_mut().zip(src) {
                if a != 0 {
                    let w = table[a as usize] as u32;
                    *d = blend(*d, p.color, if fade == 255 { w } else { w * fade / 255 });
                }
            }
        }
    }
}

/// A layer to paint: its shapes (from, to), runs (from, to), and clip in
/// pixels (left, top, right, bottom).
type PxLayer = (usize, usize, usize, usize, Option<(i32, i32, i32, i32)>);

/// Pixels (left, top, right, bottom).
pub type PxRect = (i32, i32, i32, i32);

fn intersect(a: PxRect, b: PxRect) -> Option<PxRect> {
    let r = (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3));
    (r.0 < r.2 && r.1 < r.3).then_some(r)
}

fn union(a: PxRect, b: PxRect) -> PxRect {
    (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
}

fn area(r: PxRect) -> i64 {
    (r.2 - r.0).max(0) as i64 * (r.3 - r.1).max(0) as i64
}

/// What one layer drew: its clip, and each shape, picture and run (a hash
/// of all that makes its pixels, and the pixels it can touch), in order;
/// for each run, its id and the pixel its origin falls in.
#[derive(Default)]
struct Drawn {
    clip: Option<PxRect>,
    items: [Vec<(u64, PxRect)>; 3],
    runs: Vec<(u64, (i32, i32))>,
}

/// A run drawn: its id and the pixel its origin falls in.
type RunAt = (u64, (i32, i32));

/// Which of `x`'s items (old) are gone and which of `y`'s (new) are new:
/// past the common start and end, each new item is looked for among the
/// old ones; one found out of its order (it may overlap others) counts as
/// both. Also the items kept, as (old, new) indices.
#[allow(clippy::type_complexity)]
fn differ(x: &[(u64, PxRect)], y: &[(u64, PxRect)]) -> (Vec<usize>, Vec<usize>, Vec<(usize, usize)>) {
    let start = x.iter().zip(y).take_while(|(p, q)| p == q).count();
    let end = x[start..].iter().rev().zip(y[start..].iter().rev()).take_while(|(p, q)| p == q).count();
    let (xs, ys) = (&x[start..x.len() - end], &y[start..y.len() - end]);
    let mut kept: Vec<(usize, usize)> = (0..start).map(|i| (i, i)).collect();
    kept.extend((0..end).map(|k| (x.len() - end + k, y.len() - end + k)));
    if xs.is_empty() || ys.is_empty() {
        return ((start..start + xs.len()).collect(), (start..start + ys.len()).collect(), kept);
    }
    let mut old: FxMap<(u64, PxRect), Vec<usize>> = FxMap::default();
    for (i, item) in xs.iter().enumerate().rev() {
        old.entry(*item).or_default().push(i);
    }
    let mut found = vec![false; xs.len()];
    let mut new = Vec::new();
    let mut last = None;
    for (j, item) in ys.iter().enumerate() {
        match old.get_mut(item).and_then(|at| at.pop()) {
            Some(i) if last.is_none_or(|l| i > l) => {
                found[i] = true;
                last = Some(i);
                kept.push((start + i, start + j));
            }
            _ => new.push(start + j),
        }
    }
    let gone = (0..xs.len()).filter(|&i| !found[i]).map(|i| start + i).collect();
    (gone, new, kept)
}

/// What a frame drew: its size and scale, its background, its layers.
#[derive(Default)]
struct Painted {
    view: (u32, u32, u32),
    background: u32,
    layers: Vec<Drawn>,
}

/// Where two frames differ, in pixels: None if everywhere (a new size, a new
/// background, or most of it). `refine` tells where a run redrawn in its
/// place differs from what it was there (None: everywhere it reaches).
fn damage(old: &Painted, new: &Painted, refine: impl Fn(RunAt, RunAt) -> Option<Vec<PxRect>>) -> Option<Vec<PxRect>> {
    if old.view != new.view || old.background != new.background || old.layers.is_empty() {
        return None;
    }
    let whole = (0, 0, new.view.0 as i32, new.view.1 as i32);
    let mut rects: Vec<PxRect> = Vec::new();
    let mut add = |r: PxRect, clip: Option<PxRect>| {
        if let Some(r) = intersect(r, clip.unwrap_or(whole)).and_then(|r| intersect(r, whole)) {
            rects.push(r);
        }
    };
    let empty = Drawn::default();
    for k in 0..old.layers.len().max(new.layers.len()) {
        let (a, b) = (old.layers.get(k).unwrap_or(&empty), new.layers.get(k).unwrap_or(&empty));
        if a.clip != b.clip {
            // A layer clipped anew: all of what it showed and shows.
            for (items, clip) in [(&a.items, a.clip), (&b.items, b.clip)] {
                for &(_, r) in items.iter().flatten() {
                    add(r, clip);
                }
            }
            continue;
        }
        for kind in 0..3 {
            let (x, y) = (&a.items[kind], &b.items[kind]);
            let (mut gone, new, kept) = differ(x, y);
            for j in new {
                // A run where one was (a paragraph typed in): only where
                // their glyphs differ, if nothing kept that overlaps them
                // changed its order with them.
                let pair = (kind == 2).then(|| gone.iter().position(|&i| a.runs[i].1 == b.runs[j].1)).flatten();
                let refined = pair.and_then(|p| {
                    let i = gone[p];
                    let reach = union(x[i].1, y[j].1);
                    let crossed = kept.iter().any(|&(ki, kj)| (ki < i) != (kj < j) && intersect(x[ki].1, reach).is_some());
                    if crossed { None } else { refine(a.runs[i], b.runs[j]).map(|rects| (p, rects)) }
                });
                match refined {
                    Some((p, rects)) => {
                        gone.swap_remove(p);
                        for r in rects {
                            add(r, a.clip);
                        }
                    }
                    None => add(y[j].1, a.clip),
                }
            }
            for i in gone {
                add(x[i].1, a.clip);
            }
        }
    }
    // A few rectangles: nearby ones merge.
    rects.sort_by_key(|r| (r.1, r.0));
    let mut merged: Vec<PxRect> = Vec::new();
    for r in rects {
        match merged.iter_mut().find(|m| area(union(**m, r)) <= area(**m) + area(r) + 64 * 64) {
            Some(m) => *m = union(*m, r),
            None => merged.push(r),
        }
    }
    if merged.len() > 8 {
        merged = vec![merged.iter().copied().fold(merged[0], union)];
    }
    // Most of the window: repaint it all, it is no slower.
    let total: i64 = merged.iter().map(|&r| area(r)).sum();
    (total * 2 < area(whole)).then_some(merged)
}

/// The window's pixels, 0x00RRGGBB in top-down rows, and the glyph cache.
pub struct Surface {
    pub width: u32,
    pub height: u32,
    scale: f32,
    pub pixels: Vec<u32>,
    cache: GlyphCache,
    /// Runs placed in pixels, by id and scale.
    runs: FxMap<(u64, u32), PxRun>,
    /// Frames painted, for letting go of runs no longer drawn.
    frame: u64,
    table: [u8; 256],
    /// Glyphs rasterized by the last draw (0 once the cache is warm).
    pub last_new_glyphs: usize,
    /// What the last frame drew, and where the last draw changed pixels
    /// (all of them: None).
    painted: Painted,
    pub last_damage: Option<Vec<PxRect>>,
}

/// Splits `height` rows into up to `n` bands: (top, rows) pairs.
fn split(height: u32, n: usize) -> Vec<(u32, u32)> {
    let n = (n as u32).clamp(1, height.max(1));
    let step = height.div_ceil(n);
    (0..n).map(|i| (i * step, step.min(height.saturating_sub(i * step)))).filter(|b| b.1 > 0).collect()
}

impl Surface {
    pub fn new((width, height, dpi): (u32, u32, u32)) -> Surface {
        Surface {
            width,
            height,
            scale: dpi as f32 / 96.0,
            // Zeroed pages come from the OS lazily; the first clear touches them.
            pixels: vec![0; (width * height) as usize],
            cache: GlyphCache::default(),
            runs: FxMap::default(),
            frame: 0,
            table: contrast_table(CONTRAST),
            last_new_glyphs: 0,
            painted: Painted::default(),
            last_damage: None,
        }
    }

    fn bands(&mut self, n: usize) -> Vec<Band<'_>> {
        let width = self.width;
        let mut rest: &mut [u32] = &mut self.pixels;
        let mut bands = Vec::new();
        for (top, rows) in split(self.height, n) {
            let (band, tail) = std::mem::take(&mut rest).split_at_mut((rows * width) as usize);
            rest = tail;
            bands.push(Band { rows: band, top: top as i32, width: width as i32, clip: None });
        }
        bands
    }

    pub fn clear(&mut self, color: u32, threads: usize) {
        thread::scope(|s| {
            for band in self.bands(threads) {
                s.spawn(move || band.rows.fill(color));
            }
        });
    }

    /// Paints `list`: everything, or (with `partial`) only where it differs
    /// from the last frame. Returns where pixels changed (None: everywhere).
    pub fn paint(&mut self, list: &DisplayList, clear: bool, threads: usize, partial: bool) -> Option<Vec<PxRect>> {
        let scale = self.scale;
        self.place_runs(list);
        // Each run as placed, where its origin's pixel is, and the pixels it
        // can touch there.
        let runs: Vec<(&PxRun, (i32, i32), Option<PxRect>)> = list
            .runs
            .iter()
            .map(|run| {
                let placed = &self.runs[&(run.id, scale.to_bits())];
                let (at, _) = pixel_of(run.origin, scale);
                (placed, at, placed.bounds.map(|(l, t, r, b)| (l + at.0, t + at.1, r + at.0, b + at.1)))
            })
            .collect();
        let runs = &runs;

        let shapes: Vec<PxShape> = list.shapes.iter().map(|s| PxShape::new(s, scale)).collect();
        let masks = &self.cache.masks;
        let table = self.table;
        let background = list.background;
        // Each layer's shapes and glyphs, and its clip in pixels.
        let layers: Vec<PxLayer> = list
            .layers
            .iter()
            .enumerate()
            .map(|(k, l)| {
                let next = list.layers.get(k + 1);
                let (s1, r1) = next.map_or((shapes.len(), runs.len()), |n| (n.shapes, n.runs));
                let clip = l.clip.map(|c| {
                    let px = |v: f32| (v * scale).round() as i32;
                    (px(c.left), px(c.top), px(c.right), px(c.bottom))
                });
                (l.shapes.min(s1), s1.min(shapes.len()), l.runs.min(r1), r1.min(runs.len()), clip)
            })
            .collect();
        let layers = &layers;
        // A picture's layer, where it goes (pixels), its pixels, opacity and
        // corners (pixels).
        type At<'a> = (usize, (i32, i32, i32, i32), &'a crate::pixels::Pixels, f32, i32);
        let pictures: Vec<At> = list
            .pictures
            .iter()
            .map(|(layer, r, p, a, round)| {
                let px = |v: f32| (v * scale).round() as i32;
                (*layer, (px(r.left), px(r.top), px(r.right), px(r.bottom)), &**p, *a, px(*round))
            })
            .collect();
        let pictures = &pictures;

        // What this frame draws, to compare with the last.
        let painted = Painted {
            view: (self.width, self.height, (scale * 96.0).round() as u32),
            background,
            layers: layers
                .iter()
                .enumerate()
                .map(|(k, &(s0, s1, r0, r1, clip))| {
                    let shape_items = shapes[s0..s1]
                        .iter()
                        .map(|s| {
                            let mut h = mix(mix(mix(0, s.l.to_bits() as u64), s.t.to_bits() as u64), s.r.to_bits() as u64);
                            h = mix(mix(mix(h, s.b.to_bits() as u64), s.radius.to_bits() as u64), s.blur.to_bits() as u64);
                            h = mix(mix(mix(h, s.color as u64), s.alpha.to_bits() as u64), s.ring as u64);
                            if let Some((x, y, r, n)) = s.blob {
                                h = mix(mix(mix(mix(h, x.to_bits() as u64), y.to_bits() as u64), r.to_bits() as u64), n.to_bits() as u64);
                            }
                            if let Some((v, d)) = &s.wave {
                                h = d.iter().fold(mix(h, *v as u64), |h, d| mix(h, d.to_bits() as u64));
                            }
                            let (l, t, r, b) = s.bounds();
                            (h, (l.floor() as i32, t.floor() as i32, r.ceil() as i32, b.ceil() as i32))
                        })
                        .collect();
                    let picture_items = pictures
                        .iter()
                        .filter(|p| p.0 == k)
                        .map(|&(_, r, p, a, round)| {
                            let h = mix(mix(mix(mix(mix(mix(0, p as *const _ as u64), r.0 as u64), r.1 as u64), r.2 as u64), r.3 as u64), a.to_bits() as u64);
                            (mix(h, round as u64), r)
                        })
                        .collect();
                    // A run's id stands for all it draws, where within a
                    // pixel included; the pixel is the rest.
                    let mut run_items = Vec::with_capacity(r1 - r0);
                    let mut run_keys = Vec::with_capacity(r1 - r0);
                    for (run, &(_, at, bounds)) in list.runs[r0..r1].iter().zip(&runs[r0..r1]) {
                        if let Some(b) = bounds {
                            run_items.push((mix(mix(mix(0, run.id), at.0 as u64), at.1 as u64), b));
                            run_keys.push((run.id, at));
                        }
                    }
                    Drawn { clip, items: [shape_items, picture_items, run_items], runs: run_keys }
                })
                .collect(),
        };
        // Two runs in one place: where their glyphs differ.
        let placed = &self.runs;
        let refine = |old: RunAt, new: RunAt| {
            let (p, q) = (placed.get(&(old.0, scale.to_bits()))?, placed.get(&(new.0, scale.to_bits()))?);
            let same = |g: &PxGlyph, h: &PxGlyph| (g.mask, g.x, g.y, g.color, g.alpha) == (h.mask, h.x, h.y, h.color, h.alpha);
            let start = p.glyphs.iter().zip(&q.glyphs).take_while(|(g, h)| same(g, h)).count();
            let end = p.glyphs[start..].iter().rev().zip(q.glyphs[start..].iter().rev()).take_while(|(g, h)| same(g, h)).count();
            let (_, at) = new;
            let reach = |glyphs: &[PxGlyph]| {
                glyphs.iter().map(|g| {
                    let m = &masks[g.mask as usize];
                    let (l, t) = (g.x + m.left + at.0, g.y - m.top + at.1);
                    (l, t, l + m.width as i32, t + m.height as i32)
                })
                .reduce(union)
            };
            Some([reach(&p.glyphs[start..p.glyphs.len() - end]), reach(&q.glyphs[start..q.glyphs.len() - end])].into_iter().flatten().collect())
        };
        let region = if partial && !clear { None } else if partial { damage(&self.painted, &painted, refine) } else { None };
        self.painted = painted;
        // Nothing changed: nothing to paint.
        if region.as_ref().is_some_and(|r| r.is_empty()) {
            return region;
        }

        // Borrow the pixels separately from the cache for the band threads.
        let width = self.width;
        let paint_band = |band: &mut Band, limit: Option<PxRect>| {
            if clear {
                match limit {
                    Some((l, t, r, b)) => {
                        for y in t.max(band.top)..b.min(band.top + band.height()) {
                            let row = ((y - band.top) * band.width) as usize;
                            band.rows[row + l as usize..row + r as usize].fill(background);
                        }
                    }
                    None => band.rows.fill(background),
                }
            }
            // Bottom layer (the note), then the chrome, then overlays.
            for (k, &(s0, s1, r0, r1, clip)) in layers.iter().enumerate() {
                band.clip = match (clip, limit) {
                    (Some(c), Some(l)) => match intersect(c, l) {
                        Some(c) => Some(c),
                        None => continue,
                    },
                    (c, l) => c.or(l),
                };
                for shape in &shapes[s0..s1] {
                    band.shape(shape);
                }
                for &(_, r, p, a, round) in pictures.iter().filter(|p| p.0 == k) {
                    band.picture(r, p, a, round);
                }
                // Runs that reach into what is painted, glyph by glyph.
                let (_, bt, _, bb) = band.clip.unwrap_or((0, band.top, 0, band.top + band.height()));
                let (bt, bb) = (bt.max(band.top), bb.min(band.top + band.height()));
                for &(run, (x, y), bounds) in &runs[r0..r1] {
                    let Some((l, t, r, b)) = bounds else { continue };
                    if b <= bt || t >= bb || band.clip.is_some_and(|c| r <= c.0 || l >= c.2) {
                        continue;
                    }
                    for g in &run.glyphs {
                        band.glyph(&masks[g.mask as usize], (g.x + x, g.y + y), g, &table);
                    }
                }
            }
        };
        match &region {
            // Only where it changed: small, so on this thread.
            Some(rects) => {
                for &(l, t, r, b) in rects {
                    let rows = &mut self.pixels[(t as u32 * width) as usize..(b as u32 * width) as usize];
                    let mut band = Band { rows, top: t, width: width as i32, clip: None };
                    paint_band(&mut band, Some((l, t, r, b)));
                }
            }
            None => {
                let mut rest: &mut [u32] = &mut self.pixels;
                thread::scope(|s| {
                    for (top, rows) in split(self.height, threads) {
                        let (chunk, tail) = std::mem::take(&mut rest).split_at_mut((rows * width) as usize);
                        rest = tail;
                        let paint_band = &paint_band;
                        s.spawn(move || {
                            let mut band = Band { rows: chunk, top: top as i32, width: width as i32, clip: None };
                            paint_band(&mut band, None);
                        });
                    }
                });
            }
        }
        region
    }

    /// Places every run of `list` not placed at this scale before, keeping
    /// the pictures its glyphs need, and lets go of runs no longer drawn.
    fn place_runs(&mut self, list: &DisplayList) {
        let scale = self.scale;
        self.frame += 1;
        let frame = self.frame;
        let mut fresh: Vec<&Run> = Vec::new();
        for run in &list.runs {
            match self.runs.get_mut(&(run.id, scale.to_bits())) {
                Some(placed) => placed.used = frame,
                None => fresh.push(run),
            }
        }
        self.last_new_glyphs = 0;
        for run in fresh {
            let mut glyphs = Vec::with_capacity(run.glyphs.len());
            let mut bounds: Option<PxRect> = None;
            for g in run.glyphs.iter() {
                let (mask, new) = self.cache.intern(g);
                self.last_new_glyphs += new as usize;
                let m = &self.cache.masks[mask as usize];
                if m.width == 0 || m.height == 0 {
                    continue;
                }
                let (l, t) = (g.x + m.left, g.y - m.top);
                let r = (l, t, l + m.width as i32, t + m.height as i32);
                bounds = Some(bounds.map_or(r, |b| union(b, r)));
                glyphs.push(PxGlyph { mask, x: g.x, y: g.y, color: g.color, alpha: g.alpha });
            }
            self.runs.insert((run.id, scale.to_bits()), PxRun { glyphs, bounds, used: frame });
        }
        // Runs not drawn for a few frames are gone from the scene.
        if self.runs.len() > 2 * list.runs.len() + 64 {
            self.runs.retain(|_, r| r.used + 3 >= frame);
        }
    }

    /// Draws `list` for a client area of `view`, reusing the buffer and the
    /// glyph cache when they fit, and painting only where it differs from
    /// the last frame (see `last_damage`).
    pub fn draw(&mut self, (width, height, dpi): (u32, u32, u32), list: &DisplayList) {
        if (width, height) != (self.width, self.height) {
            (self.width, self.height) = (width, height);
            self.pixels = vec![0; (width * height) as usize];
            self.painted = Painted::default();
        }
        self.scale = dpi as f32 / 96.0;
        self.last_damage = self.paint(list, true, THREADS, true);
    }

    /// Creates a surface and draws `list` into it.
    pub fn draw_new(view: (u32, u32, u32), list: &DisplayList) -> Surface {
        let mut surface = Surface::new(view);
        surface.paint(list, true, THREADS, false);
        surface
    }

    /// The pixels, 0x00RRGGBB in top-down rows.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Writes the buffer as a 32-bit BMP (`--dump-frame`), to check what a
    /// frame drew without a screen capture.
    pub fn dump(&self, path: &Path) -> std::io::Result<()> {
        let (w, h) = (self.width as i32, self.height as i32);
        let size = self.pixels.len() * 4;
        let mut b = Vec::with_capacity(54 + size);
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&((54 + size) as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&w.to_le_bytes());
        b.extend_from_slice(&(-h).to_le_bytes()); // top-down
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&32u16.to_le_bytes());
        b.extend_from_slice(&[0u8; 24]);
        for p in &self.pixels {
            b.extend_from_slice(&p.to_le_bytes());
        }
        std::fs::write(path, b)
    }
}

#[cfg(test)]
mod tests;
