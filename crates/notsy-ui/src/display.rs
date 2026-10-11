//! Display lists: a scene flattened into rectangles and individually
//! positioned glyphs, with colors resolved. This is the data; recording one
//! from a scene is `display::record` in the shell crate for now, and each
//! shell's rasterizer (`raster`) blends it into pixels.

use std::sync::Arc;

use notsy_text::{GlyphKey, Mask};

use crate::geom::{Blob, Rect, WaveShape};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use notsy_text::Layout;
use windows_core::Result;

use crate::scene::{Eng, Fill, Ink, Scene};
use crate::theme::Theme;

/// One glyph of a run, placed in pixels from the whole pixel the run's
/// origin falls in.
#[derive(Clone)]
pub struct Glyph {
    pub x: i32,
    pub y: i32,
    /// What it is: one picture serves every glyph with its key.
    pub key: GlyphKey,
    pub mask: Arc<Mask>,
    /// 0xRRGGBB.
    pub color: u32,
    /// Opacity, 255 for solid text.
    pub alpha: u8,
}

/// A rounded rectangle with its color resolved; see `scene::Fill`.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub rect: Rect,
    pub radius: f32,
    pub blur: f32,
    /// 0xRRGGBB.
    pub color: u32,
    pub alpha: f32,
    pub ring: bool,
    pub blob: Option<Blob>,
    pub wave: Option<WaveShape>,
}

/// One text's glyphs, drawn at `origin` (DIPs). Its `id` is a hash of all
/// that decides its pixels relative to the pixel its origin falls in: its
/// glyphs, where within that pixel, the scale. Two runs with one id draw
/// alike, whichever layouts they came from.
#[derive(Clone)]
pub struct Run {
    pub id: u64,
    pub origin: (f32, f32),
    pub glyphs: Arc<[Glyph]>,
}

pub struct DisplayList {
    pub background: u32,
    pub shapes: Vec<Shape>,
    pub runs: Vec<Run>,
    /// Layers, bottom to top: where each starts in `shapes` and `runs`, and
    /// the rectangle (DIPs) it is clipped to. A layer's shapes are all drawn
    /// before its pictures, and those before its text.
    pub layers: Vec<Layer>,
    /// Pictures: their layer, where (DIPs), their pixels and opacity.
    pub pictures: Vec<(usize, Rect, std::sync::Arc<crate::pixels::Pixels>, f32, f32)>,
}

#[derive(Clone, Copy, Debug)]
pub struct Layer {
    pub shapes: usize,
    pub runs: usize,
    pub clip: Option<Rect>,
}

/// A cheap running hash, for telling things apart between frames.
#[inline]
pub fn mix(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29)
}

/// What a layout draws with: its color and opacity.
struct Paint {
    color: u32,
    alpha: u8,
}

/// What one text recorded, relative to where it was drawn: its run's id and
/// glyphs, and its lines; its layout, held so no other takes its address;
/// the frame last replayed in.
struct Replay {
    _layout: Rc<Layout>,
    id: u64,
    glyphs: Arc<[Glyph]>,
    rects: Vec<(Rect, u32)>,
    used: u64,
}

/// A run's id (see `Run`): `within` is where in its pixel it is drawn, as
/// bits.
fn run_id(glyphs: &[Glyph], within: (u32, u32), scale: f32) -> u64 {
    let mut h = mix(mix(mix(0x2545_F491_4F6C_DD1D, within.0 as u64), within.1 as u64), scale.to_bits() as u64);
    for g in glyphs {
        let mut key = std::collections::hash_map::DefaultHasher::new();
        g.key.hash(&mut key);
        h = mix(h, key.finish());
        h = mix(mix(h, g.x as u32 as u64 | (g.y as u32 as u64) << 32), g.alpha as u64 | (g.color as u64) << 8);
    }
    h
}

/// A layout drawn in a color and opacity, at a place within a pixel
/// (horizontal and vertical), at a scale, with the theme's inks.
type ReplayKey = (usize, u32, u8, u32, u32, u32, u64);

#[derive(Default)]
struct Replays {
    frame: u64,
    kept: std::collections::HashMap<ReplayKey, Replay>,
}

thread_local! {
    static REPLAYS: RefCell<Replays> = RefCell::new(Replays::default());
}

/// Forgets every kept recording (for tests that compare against none).
#[doc(hidden)]
pub fn forget_kept() {
    REPLAYS.with(|r| r.borrow_mut().kept.clear());
}

/// What the theme turns inks into, as one number: recordings made under
/// another theme are not reused.
fn theme_hash(theme: &Theme) -> u64 {
    Ink::all().iter().fold(0x51_7CC1_B727_220Au64, |h, &ink| mix(h, theme.color(ink) as u64 | (ink.id() as u64) << 32))
}

/// Flattens `scene` for a device with `scale` pixels per DIP.
pub fn record(eng: &Eng, scene: &Scene, theme: &Theme, scale: f32) -> Result<DisplayList> {
    let themed = theme_hash(theme);
    let frame = REPLAYS.with(|r| {
        let mut r = r.borrow_mut();
        r.frame += 1;
        let frame = r.frame;
        r.kept.retain(|_, k| k.used + 3 >= frame);
        frame
    });
    // Layers by where they start in fills and texts: the note, the chrome,
    // then any overlays.
    let mut bounds = vec![(0, 0, None), (scene.chrome_from.0, scene.chrome_from.1, None)];
    bounds.extend(scene.overlays.iter().copied());
    for k in 1..bounds.len() {
        bounds[k].0 = bounds[k].0.clamp(bounds[k - 1].0, scene.fills.len());
        bounds[k].1 = bounds[k].1.clamp(bounds[k - 1].1, scene.texts.len());
    }
    let mut runs: Vec<Run> = Vec::with_capacity(scene.texts.len());
    let mut rects: Vec<(Rect, u32)> = Vec::new();
    // Where each layer's texts start: (runs, rects).
    let mut starts = vec![(0, 0); bounds.len()];
    let mut layer = 0;
    for (i, text) in scene.texts.iter().enumerate() {
        while layer + 1 < bounds.len() && i >= bounds[layer + 1].1 {
            layer += 1;
            starts[layer] = (runs.len(), rects.len());
        }
        let paint = Paint { color: theme.color(text.ink), alpha: (text.alpha.clamp(0.0, 1.0) * 255.0).round() as u8 };
        if paint.alpha == 0 {
            continue;
        }
        let (px, py) = (text.x * scale, text.y * scale);
        let within = (px - px.floor(), py - py.floor());
        let key: ReplayKey = (Rc::as_ptr(&text.layout) as *const () as usize, paint.color, paint.alpha, within.0.to_bits(), within.1.to_bits(), scale.to_bits(), themed);
        let origin = (text.x, text.y);
        // Drawn before at this place within a pixel: its run again, moved.
        let replayed = REPLAYS.with(|r| {
            let mut r = r.borrow_mut();
            let kept = r.kept.get_mut(&key)?;
            kept.used = frame;
            rects.extend(kept.rects.iter().map(|&(r, c)| (r.offset(text.x, text.y), c)));
            Some(Run { id: kept.id, origin, glyphs: kept.glyphs.clone() })
        });
        if let Some(run) = replayed {
            if !run.glyphs.is_empty() {
                runs.push(run);
            }
            continue;
        }
        // Kept for the frames after, relative to where it was drawn; this
        // frame draws it as those will, so they match it exactly.
        let glyphs: Arc<[Glyph]> = {
            let mut engine = eng.borrow_mut();
            text.layout
                .glyphs(within, scale)
                .into_iter()
                .filter_map(|g| {
                    let mask = engine.mask(&g)?;
                    let color = Ink::from_id(g.ink).map_or(paint.color, |ink| theme.color(ink));
                    Some(Glyph { x: g.x, y: g.y, key: g.key(), mask, color, alpha: paint.alpha })
                })
                .collect()
        };
        let lines: Vec<(Rect, u32)> = text
            .layout
            .strikes()
            .into_iter()
            .map(|s| {
                let [x, y, w, h] = s.rect;
                let color = Ink::from_id(s.ink).map_or(paint.color, |ink| theme.color(ink));
                (Rect::new(x, y, x + w, y + h.max(1.0 / scale)), color)
            })
            .collect();
        rects.extend(lines.iter().map(|&(r, c)| (r.offset(text.x, text.y), c)));
        let id = run_id(&glyphs, (within.0.to_bits(), within.1.to_bits()), scale);
        if !glyphs.is_empty() {
            runs.push(Run { id, origin, glyphs: glyphs.clone() });
        }
        REPLAYS.with(|r| {
            r.borrow_mut().kept.insert(key, Replay { _layout: text.layout.clone(), id, glyphs, rects: lines, used: frame });
        });
    }
    // Layers with no texts start where the texts ended.
    for start in starts.iter_mut().skip(layer + 1) {
        *start = (runs.len(), rects.len());
    }
    let fill = |f: &Fill| Shape {
        rect: f.rect,
        radius: f.radius,
        blur: f.blur,
        color: theme.color(f.ink),
        alpha: if f.ink == Ink::Shadow { f.alpha * theme.shadow } else { f.alpha },
        ring: f.ring,
        blob: f.blob,
        wave: f.wave,
    };
    let line = |&(rect, color): &(Rect, u32)| Shape { rect, radius: 0.0, blur: 0.0, color, alpha: 1.0, ring: false, blob: None, wave: None };
    let mut shapes = Vec::with_capacity(scene.fills.len() + rects.len());
    let mut layers = Vec::with_capacity(bounds.len());
    for k in 0..bounds.len() {
        let fills_end = bounds.get(k + 1).map_or(scene.fills.len(), |b| b.0);
        let rects_end = starts.get(k + 1).map_or(rects.len(), |s| s.1);
        layers.push(Layer { shapes: shapes.len(), runs: starts[k].0, clip: bounds[k].2 });
        shapes.extend(scene.fills[bounds[k].0..fills_end].iter().map(fill));
        shapes.extend(rects[starts[k].1..rects_end].iter().map(line));
    }
    let mut pictures: Vec<_> = scene.pictures.iter().map(|p| (p.layer, p.rect, p.pixels.clone(), p.alpha, p.round)).collect();
    // Marks from outlines: filled at the size they are drawn, in their ink.
    for v in &scene.vectors {
        let size = ((v.rect.right * scale).round() - (v.rect.left * scale).round()).max(1.0) as usize;
        pictures.push((v.layer, v.rect, crate::vector::mark(v.logo, size, theme.color(v.ink)), v.alpha, 0.0));
    }
    Ok(DisplayList { background: theme.background, shapes, runs, layers, pictures })
}
