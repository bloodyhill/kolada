//! A picture opened full size, by a double click on it in a note: it grows
//! out of its place on the page to fill the window, on a spring, over the
//! page's own ground (graphite or paper); a click or Esc puts it back.
//! Nothing leaves the app. Settled, nothing moves.

use std::path::PathBuf;

use crate::geom::{Fill, Ink, Rect};
use crate::scene::{Input, Scene, Target};

/// The room kept around the picture at full size, DIPs.
const MARGIN: f32 = 32.0;
/// The most a picture grows past its own pixels: a small one stays sharp.
const MOST_GROWTH: f32 = 2.0;
/// The spring, critically damped: there in about a quarter of a second,
/// never past.
const OMEGA: f32 = 18.0;
/// Its corners on the page; full size, square.
const PAGE_ROUND: f32 = 8.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Lightbox {
    pub path: PathBuf,
    /// Where it sits on the page (DIPs), and its own size in pixels.
    pub from: Rect,
    pub size: (u32, u32),
    open: bool,
    /// How far out of the page it has come: 0 in its place, 1 full size.
    pub t: f32,
    velocity: f32,
}

impl Lightbox {
    /// The picture at `path`, shown at `from` on the page, `size` pixels.
    pub fn open(path: PathBuf, from: Rect, size: (u32, u32)) -> Lightbox {
        Lightbox { path, from, size, open: true, t: 0.0, velocity: 0.0 }
    }

    /// Back into the page: whether it was out.
    pub fn close(&mut self) -> bool {
        std::mem::replace(&mut self.open, false)
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// `dt` seconds on: true while it still moves.
    pub fn step(&mut self, dt: f32) -> bool {
        let target = if self.open { 1.0 } else { 0.0 };
        let (x, v, w) = (self.t - target, self.velocity, OMEGA);
        let e = (-w * dt).exp();
        let x1 = (x + (v + w * x) * dt) * e;
        let v1 = (v - w * (v + w * x) * dt) * e;
        if x1.abs() < 0.0005 && v1.abs() < 0.005 {
            self.t = target;
            self.velocity = 0.0;
            return false;
        }
        self.t = target + x1;
        self.velocity = v1;
        true
    }

    pub fn moving(&self) -> bool {
        self.t != if self.open { 1.0 } else { 0.0 } || self.velocity != 0.0
    }

    /// Put back and settled: the shell lets it go.
    pub fn gone(&self) -> bool {
        !self.open && !self.moving()
    }

    /// Where it is full size in a window `width` by `height` DIPs at
    /// `scale` pixels a DIP: as large as the window holds with room around
    /// it, its shape kept, no more than twice its own size; centred.
    pub fn full(&self, width: f32, height: f32, scale: f32) -> Rect {
        let (pw, ph) = (self.size.0.max(1) as f32, self.size.1.max(1) as f32);
        let own = MOST_GROWTH / scale.max(0.1);
        let k = ((width - 2.0 * MARGIN).max(1.0) / pw).min((height - 2.0 * MARGIN).max(1.0) / ph).min(own);
        let (w, h) = (pw * k, ph * k);
        let (left, top) = (((width - w) / 2.0).round(), ((height - h) / 2.0).round());
        Rect::new(left, top, left + w.round(), top + h.round())
    }

    /// Where it is drawn now: on its way between the page and full size.
    pub fn rect(&self, width: f32, height: f32, scale: f32) -> Rect {
        let (a, b, t) = (self.from, self.full(width, height, scale), self.t.clamp(0.0, 1.0));
        let at = |p: f32, q: f32| p + (q - p) * t;
        Rect::new(at(a.left, b.left), at(a.top, b.top), at(a.right, b.right), at(a.bottom, b.bottom))
    }
}

/// Draws it over everything: the page's ground fading in behind it, the
/// picture over that. A click anywhere puts it back.
pub fn draw(input: &Input, lightbox: &Lightbox, width: f32, height: f32, scene: &mut Scene) {
    let t = lightbox.t.clamp(0.0, 1.0);
    let window = Rect::new(0.0, 0.0, width, height);
    // The ground on a layer of its own, the picture on the next: over it
    // whatever order a layer draws its own in.
    scene.overlay(None);
    scene.fills.push(Fill::faded(Rect::new(-1.0, -1.0, width + 1.0, height + 1.0), 0.0, Ink::Page, t));
    scene.hits.push((window, Target::Lightbox));
    scene.overlay(None);
    let r = lightbox.rect(width, height, input.scale.max(0.1));
    // Asked for at its full size once, so it decodes once; until then the
    // page's smaller one stands in (see `PictureSource::pixels`).
    let full = lightbox.full(width, height, input.scale.max(0.1));
    let px = (full.width() * input.scale.max(1.0)).round().max(1.0) as u32;
    if let Some((pixels, _)) = input.images.and_then(|images| images.borrow_mut().pixels(&lightbox.path, px)) {
        let layer = scene.layer_now();
        scene.pictures.push(crate::scene::Picture { layer, rect: r, pixels, alpha: 1.0, round: PAGE_ROUND * (1.0 - t) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened() -> Lightbox {
        Lightbox::open(PathBuf::from("sea.png"), Rect::new(100.0, 300.0, 400.0, 500.0), (3000, 2000))
    }

    #[test]
    fn it_grows_from_its_place_to_fill_the_window_and_settles() {
        let mut l = opened();
        let (w, h) = (1600.0, 1000.0);
        assert_eq!(l.rect(w, h, 1.0), l.from, "it starts where it was on the page");
        let full = l.full(w, h, 1.0);
        assert!((full.height() - (h - 2.0 * MARGIN)).abs() <= 1.0, "as tall as the window holds: {full:?}");
        assert!((full.width() / full.height() - 1.5).abs() < 0.01, "its shape kept");
        assert!(((full.left + full.right) / 2.0 - w / 2.0).abs() <= 1.0, "centred");
        let (mut last, mut time) = (0.0, 0.0);
        while l.step(1.0 / 120.0) {
            assert!(l.t >= last - 1e-6 && l.t <= 1.0 + 1e-6, "out without passing its size");
            last = l.t;
            time += 1.0 / 120.0;
        }
        assert!(time < 0.6, "settled in {time} s");
        assert_eq!(l.rect(w, h, 1.0), full);
        assert!(!l.moving() && !l.step(1.0 / 60.0), "settled: no more frames");
    }

    #[test]
    fn a_click_or_esc_puts_it_back_where_it_was() {
        let mut l = opened();
        while l.step(1.0 / 60.0) {}
        assert!(l.close() && !l.is_open() && !l.close(), "closed once");
        assert!(!l.gone(), "on its way back");
        while l.step(1.0 / 60.0) {}
        assert!(l.gone());
        assert_eq!(l.rect(1600.0, 1000.0, 1.0), l.from, "back in its place");
    }

    #[test]
    fn a_small_picture_grows_no_more_than_twice_its_own_size() {
        let l = Lightbox::open(PathBuf::from("icon.png"), Rect::new(0.0, 0.0, 40.0, 40.0), (40, 40));
        assert_eq!(l.full(1600.0, 1000.0, 1.0).width(), 80.0);
        assert_eq!(l.full(1600.0, 1000.0, 2.0).width(), 40.0, "its own pixels count at the screen's scale");
        // A tall picture in a narrow window: the window's width holds it.
        let tall = Lightbox::open(PathBuf::from("t.png"), Rect::default(), (1000, 4000));
        let r = tall.full(300.0, 2000.0, 1.0);
        assert!(r.left >= MARGIN - 1.0 && r.right <= 300.0 - MARGIN + 1.0 && r.top >= 0.0);
    }
}
