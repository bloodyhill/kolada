//! Turns a run's frames into timings.

use windows::Win32::Foundation::RECT;

use crate::capture::{Frame, MIP};
use crate::clock;

/// A rectangle in thumbnail pixels, end-exclusive.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Region {
    pub fn full(width: u32, height: u32) -> Region {
        Region { x0: 0, y0: 0, x1: width as usize, y1: height as usize }
    }

    /// Maps a screen rectangle onto the captured monitor's thumbnail,
    /// rounding inward so partly covered thumbnail pixels are left out.
    /// Returns None if the rectangle is not on this monitor.
    pub fn from_window(window: RECT, desktop: RECT, width: u32, height: u32) -> Option<Region> {
        let scale = 1u32 << MIP;
        let start = |v: i32, max: u32| (v.max(0) as u32).div_ceil(scale).min(max) as usize;
        let end = |v: i32, max: u32| ((v.max(0) as u32) / scale).min(max) as usize;
        let r = Region {
            x0: start(window.left - desktop.left, width),
            y0: start(window.top - desktop.top, height),
            x1: end(window.right - desktop.left, width),
            y1: end(window.bottom - desktop.top, height),
        };
        (r.x1 > r.x0 && r.y1 > r.y0).then_some(r)
    }

    pub fn intersect(&self, other: &Region) -> Option<Region> {
        let r = Region {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };
        (r.x1 > r.x0 && r.y1 > r.y0).then_some(r)
    }

    pub fn width(&self) -> usize {
        self.x1 - self.x0
    }

    pub fn height(&self) -> usize {
        self.y1 - self.y0
    }
}

pub struct Thresholds {
    /// Luminance change (0–255) for a thumbnail pixel to count as changed.
    pub pixel_delta: u8,
    /// Changed pixels needed for a frame to count as different. Keeps a
    /// blinking caret from counting as a change.
    pub min_pixels: usize,
}

/// Frame indices of the key moments of one launch.
pub struct Moments {
    pub baseline: usize,
    /// First frame where the window region differs from before the launch.
    pub first_pixel: Option<usize>,
    /// First frame from which the region stays within 2% of its final look.
    /// Ignores small late cosmetic changes (e.g. auto-collapsing scrollbars)
    /// but not missing content like note text or a sidebar.
    pub content: Option<usize>,
    /// First frame from which the region no longer changes until the end.
    pub complete: Option<usize>,
}

/// Share of the launch's total pixel change still allowed for `content`.
const CONTENT_TOLERANCE: f64 = 0.02;

pub fn changed_in(a: &[u8], b: &[u8], stride: usize, r: &Region, delta: u8) -> usize {
    let mut n = 0;
    for y in r.y0..r.y1 {
        let row = y * stride;
        let (ra, rb) = (&a[row + r.x0..row + r.x1], &b[row + r.x0..row + r.x1]);
        n += ra.iter().zip(rb).filter(|(p, q)| p.abs_diff(**q) > delta).count();
    }
    n
}

pub fn moments(frames: &[Frame], t0: i64, stride: usize, r: &Region, t: &Thresholds) -> Moments {
    let baseline = frames.iter().rposition(|f| f.qpc <= t0).unwrap_or(0);
    let last = frames.len() - 1;
    let changed = |a: usize, b: usize| {
        changed_in(&frames[a].lum, &frames[b].lum, stride, r, t.pixel_delta)
    };
    // Earliest frame after which every frame is within `tolerance` of the last.
    let settled_from = |tolerance: usize| {
        let mut c = last;
        while c > baseline + 1 && changed(c - 1, last) < tolerance {
            c -= 1;
        }
        c
    };

    let first_pixel = (baseline + 1..=last).find(|&i| changed(i, baseline) >= t.min_pixels);
    let total = changed(last, baseline);
    // If the final frame looks like the pre-launch screen, nothing appeared.
    if first_pixel.is_none() || total < t.min_pixels {
        return Moments { baseline, first_pixel, content: None, complete: None };
    }
    let content_tolerance = ((total as f64 * CONTENT_TOLERANCE) as usize).max(t.min_pixels);
    Moments {
        baseline,
        first_pixel,
        content: Some(settled_from(content_tolerance)),
        complete: Some(settled_from(t.min_pixels)),
    }
}

/// Per-frame CSV: time since launch and changed pixels vs the baseline and
/// vs the final frame. Shows exactly what drove each timing.
pub fn trace_csv(frames: &[Frame], t0: i64, stride: usize, r: &Region, t: &Thresholds) -> String {
    let base = frames.iter().rposition(|f| f.qpc <= t0).unwrap_or(0);
    let last = &frames[frames.len() - 1].lum;
    let mut csv = String::from("ms,changed_vs_baseline,changed_vs_final\n");
    for f in &frames[base..] {
        csv.push_str(&format!(
            "{:.1},{},{}\n",
            clock::ms_between(t0, f.qpc),
            changed_in(&frames[base].lum, &f.lum, stride, r, t.pixel_delta),
            changed_in(last, &f.lum, stride, r, t.pixel_delta)
        ));
    }
    csv
}

pub fn ms_after(frames: &[Frame], t0: i64, index: Option<usize>) -> Option<f64> {
    index.map(|i| clock::ms_between(t0, frames[i].qpc))
}

pub struct Stats {
    pub n: usize,
    pub min: f64,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
    pub mean: f64,
}

pub fn stats(values: &[f64]) -> Option<Stats> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    // Nearest-rank percentile.
    let rank = |p: f64| v[((p / 100.0 * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1];
    Some(Stats {
        n: v.len(),
        min: v[0],
        p50: rank(50.0),
        p95: rank(95.0),
        max: v[v.len() - 1],
        mean: v.iter().sum::<f64>() / v.len() as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 20;
    const H: usize = 10;

    fn frame(qpc: i64, fill: impl Fn(usize, usize) -> u8) -> Frame {
        let mut lum = vec![0u8; W * H];
        for y in 0..H {
            for x in 0..W {
                lum[y * W + x] = fill(x, y);
            }
        }
        Frame { qpc, lum }
    }

    fn thresholds() -> Thresholds {
        Thresholds { pixel_delta: 24, min_pixels: 4 }
    }

    /// Desktop (0) → blank window (100) → content (200 in the top half)
    /// → a late 2-pixel-wide cosmetic change in the last column.
    fn launch() -> Vec<Frame> {
        vec![
            frame(0, |_, _| 0),
            frame(10, |_, _| 100),
            frame(20, |_, y| if y < 5 { 200 } else { 100 }),
            frame(30, |x, y| if x == W - 1 { 50 } else if y < 5 { 200 } else { 100 }),
        ]
    }

    #[test]
    fn finds_first_pixel_content_and_complete() {
        let frames = launch();
        let m = moments(&frames, 5, W, &Region::full(W as u32, H as u32), &thresholds());
        assert_eq!(m.baseline, 0);
        assert_eq!(m.first_pixel, Some(1));
        // The late change is 10 of 200 changed pixels (5%), above the 2%
        // tolerance, so complete_98 waits for it too.
        assert_eq!(m.complete, Some(3));
        assert_eq!(m.content, Some(3));
    }

    #[test]
    fn content_ignores_small_late_changes() {
        let mut frames = launch();
        // A 3-pixel late change: counts for complete (min_pixels 3) but is
        // below the 2% tolerance (4 of 200 pixels) for complete_98.
        frames[3] = frame(30, |x, y| {
            if x == W - 1 && y < 3 { 50 } else if y < 5 { 200 } else { 100 }
        });
        let t = Thresholds { pixel_delta: 24, min_pixels: 3 };
        let m = moments(&frames, 5, W, &Region::full(W as u32, H as u32), &t);
        assert_eq!(m.complete, Some(3));
        assert_eq!(m.content, Some(2));
    }

    #[test]
    fn caret_blink_below_min_pixels_is_ignored() {
        let mut frames = launch();
        frames.truncate(3);
        // Caret on/off: 2 pixels toggling after the content appeared.
        frames.push(frame(30, |x, y| if x == 3 && y < 2 { 0 } else if y < 5 { 200 } else { 100 }));
        frames.push(frame(40, |_, y| if y < 5 { 200 } else { 100 }));
        let m = moments(&frames, 5, W, &Region::full(W as u32, H as u32), &thresholds());
        assert_eq!(m.complete, Some(2));
    }

    #[test]
    fn nothing_appeared() {
        let frames = vec![frame(0, |_, _| 0), frame(10, |_, _| 0)];
        let m = moments(&frames, 5, W, &Region::full(W as u32, H as u32), &thresholds());
        assert_eq!(m.first_pixel, None);
        assert_eq!(m.complete, None);
        assert_eq!(m.content, None);
    }

    #[test]
    fn frames_before_launch_are_baseline() {
        let mut frames = launch();
        frames.insert(1, frame(3, |_, _| 7)); // pre-launch noise below delta
        let m = moments(&frames, 5, W, &Region::full(W as u32, H as u32), &thresholds());
        assert_eq!(m.baseline, 1);
        assert_eq!(m.first_pixel, Some(2));
    }

    #[test]
    fn region_limits_analysis() {
        let frames = launch();
        // Left half only: the late change in the last column is outside.
        let left = Region { x0: 0, y0: 0, x1: W / 2, y1: H };
        let m = moments(&frames, 5, W, &left, &thresholds());
        assert_eq!(m.complete, Some(2));
    }

    #[test]
    fn window_rect_maps_to_thumbnail() {
        let rect = |l, t, r, b| RECT { left: l, top: t, right: r, bottom: b };
        let desktop = rect(0, 0, 3840, 2400);
        let r = Region::from_window(rect(80, 16, 800, 400), desktop, 480, 300).unwrap();
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (10, 2, 100, 50));
        // Off this monitor.
        assert!(Region::from_window(rect(4000, 0, 5000, 900), desktop, 480, 300).is_none());
    }

    #[test]
    fn nearest_rank_percentiles() {
        let v: Vec<f64> = (1..=20).map(f64::from).collect();
        let s = stats(&v).unwrap();
        assert_eq!((s.min, s.p50, s.p95, s.max), (1.0, 10.0, 19.0, 20.0));
        assert!(stats(&[]).is_none());
    }
}
