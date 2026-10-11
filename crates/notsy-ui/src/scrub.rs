//! The time scrubber: a note's past along a track at the foot of the page.
//!
//! Each version is a bar as tall as how much it changed; a new sitting (an
//! hour or more apart) opens a wider gap, a new day a wider one still. The
//! playhead is a bar in the accent with a soft glow, gliding between
//! versions, and the bars swell as it nears them. The bars between it and
//! now are tinted: what restoring the version shown would undo. Above the
//! playhead, when that version was written. The page shows that version,
//! its lines that differ from now marked in the margin and the rest faded
//! (see `scene::draw_page`); the changed line under the pointer offers to
//! come back.

use notsy_text::{Align, Format};
use windows_core::Result;

use crate::scene::{self, Eng, Fill, Fonts, Ink, Input, Rect, Scene, Target, Text};

/// The note's past as the scrubber shows it.
pub struct ScrubView<'a> {
    /// Each version, oldest first: when it was written (ms since the Unix
    /// epoch) and how much it changed from the one before (bytes). The last
    /// is the note as it is now.
    pub times: &'a [u64],
    pub changes: &'a [usize],
    /// The version shown, and where the playhead is as its spring has it,
    /// in versions (between two while it glides).
    pub shown: usize,
    pub head: f32,
    /// When the version shown was written ("Tuesday 3:42 PM"), how long
    /// ago ("2 hours ago"); when the oldest was ("12 Sep").
    pub when: &'a str,
    pub ago: &'a str,
    pub since: &'a str,
    /// For each line of the version shown, whether it has text that now
    /// does not.
    pub changed: &'a [bool],
    /// How far into the past the page has gone, 0..1: lines unchanged since
    /// fade as it rises.
    pub past: f32,
    /// Opening and closing, 0..1.
    pub alpha: f32,
    /// The changed line under the pointer, which offers to come back.
    pub hover_line: Option<usize>,
}

pub const DOCK_HEIGHT: f32 = 76.0;
const DOCK_RADIUS: f32 = 18.0;
/// The dock's least and greatest width, DIPs.
const DOCK_LEAST: f32 = 300.0;
const DOCK_MOST: f32 = 680.0;
const BAR: f32 = 3.5;
/// Versions are at most this far apart on the track, however few.
const MAX_STEP: f32 = 8.0;
/// The bars' half height at most, and the playhead's.
const HALF: f32 = 15.0;
const CLOSE: f32 = 28.0;
const RESTORE: f32 = 92.0;
const HOUR: u64 = 3_600_000;
const DAY: u64 = 24 * HOUR;

/// What the dock holds beside its track: its padding, Restore, closing.
const BESIDE_TRACK: f32 = 24.0 + 18.0 + RESTORE + 10.0 + CLOSE + 16.0;

/// Where the dock sits: under the note column, at the window's foot, as
/// wide as a track of `track` DIPs needs (a short past, a short dock),
/// never wider than the column.
pub fn dock(column: Rect, width: f32, height: f32, track: f32) -> Rect {
    let least = DOCK_LEAST.min(width - 16.0);
    let w = (track + BESIDE_TRACK).clamp(least, column.width().clamp(least, DOCK_MOST));
    let mid = (column.left + column.right) / 2.0;
    let left = (mid - w / 2.0).clamp(8.0, (width - 8.0 - w).max(8.0));
    Rect::new(left, height - 20.0 - DOCK_HEIGHT, left + w, height - 20.0)
}

/// The track in the dock: the bars, and the dates under them.
pub fn track(dock: Rect) -> Rect {
    Rect::new(dock.left + 24.0, dock.top + 10.0, dock.right - (BESIDE_TRACK - 24.0), dock.top + 64.0)
}

/// How wide a track the versions written at `times` would take, each a
/// step apart at most (see `places`).
pub fn wanted(times: &[u64]) -> f32 {
    let mut u = 0.0;
    for w in times.windows(2) {
        let gap = w[1].saturating_sub(w[0]);
        u += if gap >= DAY { 5.0 } else if gap >= HOUR { 3.0 } else { 1.0 };
    }
    u * MAX_STEP
}

/// Where each version sits along a track `width` wide, from its left: a
/// step apart, three where a new sitting began (an hour or more between
/// two), five for a new day; at most `MAX_STEP` apart, reaching the right
/// end, where now is.
pub fn places(times: &[u64], width: f32) -> Vec<f32> {
    let mut units = Vec::with_capacity(times.len());
    let mut u = 0.0;
    for (i, &t) in times.iter().enumerate() {
        if i > 0 {
            let gap = t.saturating_sub(times[i - 1]);
            u += if gap >= DAY { 5.0 } else if gap >= HOUR { 3.0 } else { 1.0 };
        }
        units.push(u);
    }
    let step = if u > 0.0 { (width / u).min(MAX_STEP) } else { 0.0 };
    let start = width - u * step;
    units.into_iter().map(|u| start + u * step).collect()
}

/// The version nearest `x` along the track (from its left).
pub fn nearest(places: &[f32], x: f32) -> usize {
    let i = places.partition_point(|&p| p < x);
    match (i.checked_sub(1), places.get(i)) {
        (Some(a), Some(&b)) if x - places[a] > b - x => i,
        (Some(a), _) => a,
        (None, _) => 0,
    }
}

/// Where the playhead is along the track, between versions while it glides.
pub fn place_at(places: &[f32], head: f32) -> f32 {
    let Some(&last) = places.last() else { return 0.0 };
    let i = (head.max(0.0).floor() as usize).min(places.len() - 1);
    match places.get(i + 1) {
        Some(&next) => places[i] + (next - places[i]) * (head - i as f32).clamp(0.0, 1.0),
        None => last,
    }
}

/// A bar's half height for a change of `change` bytes when the biggest is
/// `most`, `d` DIPs from the playhead: by the change's order of size (a
/// typo small, a rewrite tall), swelling as the playhead nears.
pub fn half_height(change: usize, most: usize, d: f32) -> f32 {
    let size = ((change as f32 + 1.0).ln() / (most.max(1) as f32 + 1.0).ln()).clamp(0.0, 1.0);
    let swell = 1.0 + 0.6 * (-(d / 30.0) * (d / 30.0)).exp();
    ((1.8 + 11.0 * size.powf(1.6)) * swell).min(HALF - 1.0)
}

fn push_text(eng: &Eng, scene: &mut Scene, text: &str, format: &Format, r: Rect, ink: Ink) {
    let l = scene::kept_layout(eng, text, format, r.width(), r.height(), 0, || scene::layout_styled(eng, text, format, r.width(), r.height(), &[]));
    scene.texts.push(Text { x: r.left, y: r.top, layout: l, ink, alpha: 1.0 });
}

fn width_of(eng: &Eng, text: &str, format: &Format) -> f32 {
    scene::kept_layout(eng, text, format, 1000.0, 40.0, 0, || scene::layout_styled(eng, text, format, 1000.0, 40.0, &[])).measure().width
}

/// Draws the scrubber over the page: the dock with its track, the playhead's
/// bubble, and the line under the pointer's offer to come back.
pub fn draw(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    let Some(s) = &input.scrub else { return Ok(()) };
    let a = s.alpha.clamp(0.0, 1.0);
    let n = s.times.len().min(s.changes.len());
    if a <= 0.01 || n == 0 {
        return Ok(());
    }
    scene.overlay(None);
    let (fills_from, texts_from) = (scene.fills.len(), scene.texts.len());
    let hover = input.hover;

    // A changed line under the pointer: a pill at its right, to bring it
    // back.
    if let Some(i) = s.hover_line.filter(|&i| s.changed.get(i) == Some(&true))
        && let Some(start) = input.lines.get(i).map(|l| l.start)
        && let Some(line) = scene.lines.iter().find(|l| l.para.start == start)
    {
        let label = "Bring back";
        let w = 34.0 + width_of(eng, label, &fonts.label) + 14.0;
        let first = line.height.min(34.0);
        let left = if scene.column.right + 16.0 + w <= width - 16.0 { scene.column.right + 16.0 } else { scene.column.right - w };
        let r = Rect::new(left, line.y + (first - 28.0) / 2.0, left + w, line.y + (first + 28.0) / 2.0);
        scene.fills.push(Fill::shadow(r.offset(0.0, 3.0), 14.0, 12.0, 1.0));
        scene.fills.push(Fill::solid(r, 14.0, Ink::Raised));
        if hover == Some(Target::BringBack(i)) {
            scene.fills.push(Fill::faded(r, 14.0, Ink::Accent, 0.12));
        }
        push_text(eng, scene, "\u{E7A7}", &fonts.icon_small, Rect::new(r.left + 8.0, r.top, r.left + 30.0, r.bottom), Ink::Accent);
        push_text(eng, scene, label, &fonts.label, Rect::new(r.left + 32.0, r.top, r.right - 10.0, r.bottom), Ink::Accent);
        scene.hits.push((r, Target::BringBack(i)));
    }

    // The dock rises into place as it opens.
    let dock = dock(scene.column, width, height, wanted(&s.times[..n])).offset(0.0, (1.0 - a) * 18.0);
    scene.fills.push(Fill::shadow(dock.offset(0.0, 10.0), DOCK_RADIUS, 36.0, 2.2));
    scene.fills.push(Fill::solid(dock, DOCK_RADIUS, Ink::Border));
    scene.fills.push(Fill::solid(dock.inset(1.0), DOCK_RADIUS - 1.0, Ink::Panel));
    scene.hits.push((dock, Target::ScrubPanel));

    let track = track(dock);
    let mid = dock.top + 30.0;
    let places = places(&s.times[..n], track.width());
    let head_x = track.left + place_at(&places, s.head);
    let most = s.changes[..n].iter().copied().max().unwrap_or(1);
    // Bars closer than a bar and a gap share one, the biggest change of
    // them.
    let mut i = 0;
    while i < n {
        let mut j = i;
        let mut change = s.changes[i];
        while j + 1 < n && places[j + 1] - places[i] < BAR + 1.5 {
            j += 1;
            change = change.max(s.changes[j]);
        }
        let x = track.left + (places[i] + places[j]) / 2.0;
        let half = half_height(change, most, (x - head_x).abs());
        let bar = Rect::new(x - BAR / 2.0, mid - half, x + BAR / 2.0, mid + half);
        // Newer than the version shown: what restoring it would undo. Older
        // ones recede the further back they are.
        let (ink, alpha) = if x > head_x + 1.0 { (Ink::Accent, 0.36) } else { (Ink::Faint, 0.5 + 0.5 * j as f32 / (n - 1).max(1) as f32) };
        scene.fills.push(Fill::faded(bar, BAR / 2.0, ink, alpha));
        i = j + 1;
    }
    // The playhead, glowing.
    let head = Rect::new(head_x - 2.0, mid - HALF - 1.0, head_x + 2.0, mid + HALF + 1.0);
    scene.fills.push(Fill { blur: 14.0, ..Fill::faded(head.inset(-5.0), 9.0, Ink::Accent, 0.45) });
    scene.fills.push(Fill::solid(head, 2.0, Ink::Accent));
    scene.hits.push((Rect::new(track.left - 10.0, dock.top, track.right + 10.0, dock.bottom), Target::ScrubTrack));

    // Under the bars: when the oldest was written, and now.
    let at_now = s.shown + 1 >= n;
    let small = fonts.small;
    let labels = Rect::new(track.left, dock.top + 50.0, track.right, dock.top + 68.0);
    // The oldest date under the first bar, where the bars begin.
    let first = (track.left + places[0] - 2.0).min(labels.right - 120.0).max(labels.left);
    push_text(eng, scene, s.since, &small, Rect::new(first, labels.top, first + 120.0, labels.bottom), Ink::Faint);
    push_text(eng, scene, "Now", &Format { align: Align::End, ..small }, Rect::new(labels.right - 60.0, labels.top, labels.right + 2.0, labels.bottom), if at_now { Ink::Muted } else { Ink::Faint });

    // Restore, while a past version is shown; and closing.
    let centre = dock.top + DOCK_HEIGHT / 2.0;
    let close = Rect::new(dock.right - 16.0 - CLOSE, centre - CLOSE / 2.0, dock.right - 16.0, centre + CLOSE / 2.0);
    if hover == Some(Target::ScrubClose) {
        scene.fills.push(Fill::solid(close, 8.0, Ink::Hover));
    }
    push_text(eng, scene, "\u{E8BB}", &fonts.icon_small, close, if hover == Some(Target::ScrubClose) { Ink::Text } else { Ink::Muted });
    scene.hits.push((close, Target::ScrubClose));
    if !at_now {
        let restore = Rect::new(close.left - 10.0 - RESTORE, centre - 16.0, close.left - 10.0, centre + 16.0);
        let hot = hover == Some(Target::ScrubRestore);
        scene.fills.push(Fill::solid(restore, 16.0, if hot { Ink::Accent } else { Ink::Field }));
        push_text(eng, scene, "Restore", &Format { align: Align::Center, ..fonts.label }, restore, if hot { Ink::OnAccent } else { Ink::Accent });
        scene.hits.push((restore, Target::ScrubRestore));
    }

    // When the version shown was written, in a bubble over the playhead.
    let (when_w, ago_w) = (width_of(eng, s.when, &fonts.label), if s.ago.is_empty() { 0.0 } else { width_of(eng, s.ago, &small) });
    let w = 28.0 + when_w + if ago_w > 0.0 { 8.0 + ago_w } else { 0.0 };
    let bx = (head_x - w / 2.0).clamp(dock.left, dock.right - w);
    let bubble = Rect::new(bx, dock.top - 9.0 - 32.0, bx + w, dock.top - 9.0);
    scene.fills.push(Fill::shadow(bubble.offset(0.0, 4.0), 16.0, 16.0, 1.2));
    scene.fills.push(Fill::solid(bubble, 16.0, Ink::Raised));
    push_text(eng, scene, s.when, &fonts.label, Rect::new(bubble.left + 14.0, bubble.top, bubble.left + 14.0 + when_w + 1.0, bubble.bottom), Ink::Text);
    if ago_w > 0.0 {
        push_text(eng, scene, s.ago, &small, Rect::new(bubble.left + 22.0 + when_w, bubble.top, bubble.right - 10.0, bubble.bottom), Ink::Faint);
    }

    if a < 1.0 {
        scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= a);
        scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= a);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_sit_a_step_apart_with_gaps_between_sittings_reaching_now() {
        let m = 60_000;
        // Three in one sitting, then one a few hours later, then a day on.
        let times = [0, m, 2 * m, 3 * HOUR, 3 * HOUR + DAY];
        let p = places(&times, 400.0);
        let steps: Vec<f32> = p.windows(2).map(|w| w[1] - w[0]).collect();
        assert_eq!(steps, [MAX_STEP, MAX_STEP, 3.0 * MAX_STEP, 5.0 * MAX_STEP]);
        // The newest at the right end, where now is.
        assert_eq!(*p.last().unwrap(), 400.0);
        // Many: squeezed to fit, the first at the left end.
        let times: Vec<u64> = (0..1000).map(|i| i * m).collect();
        let p = places(&times, 300.0);
        assert!(p[0].abs() < 0.01 && (p[999] - 300.0).abs() < 0.01);
        assert_eq!(places(&[5], 300.0), [300.0]);
    }

    #[test]
    fn the_track_finds_the_nearest_version_and_the_playhead_glides_between() {
        let p = [0.0, 10.0, 40.0];
        assert_eq!(nearest(&p, -5.0), 0);
        assert_eq!(nearest(&p, 4.0), 0);
        assert_eq!(nearest(&p, 6.0), 1);
        assert_eq!(nearest(&p, 26.0), 2);
        assert_eq!(nearest(&p, 99.0), 2);
        assert_eq!(place_at(&p, 1.5), 25.0);
        assert_eq!(place_at(&p, 2.0), 40.0);
        assert_eq!(place_at(&p, 7.0), 40.0);
    }

    #[test]
    fn bars_are_as_tall_as_their_change_and_swell_near_the_playhead() {
        let (small, big) = (half_height(3, 5000, 200.0), half_height(5000, 5000, 200.0));
        assert!(small < big && small >= 2.0);
        assert!(half_height(3, 5000, 0.0) > small * 1.4);
        assert!(half_height(5000, 5000, 0.0) <= HALF - 1.0);
        // A rewrite stands several times taller than a typo.
        assert!(big > 3.0 * small);
    }

    #[test]
    fn the_dock_fits_under_the_column_at_every_width() {
        for (w, h) in [(360.0, 300.0), (640.0, 800.0), (1280.0, 800.0), (2560.0, 1400.0)] {
            let column = Rect::new(w * 0.2, 60.0, w * 0.2 + (w * 0.7).min(720.0), h);
            for want in [0.0, 120.0, 5000.0] {
                let d = dock(column, w, h, want);
                assert!(d.left >= 8.0 && d.right <= w - 8.0 + 0.01, "{d:?} in {w}");
                assert!(d.bottom < h && d.width() <= DOCK_MOST);
                // Room for bars beside Restore and closing.
                assert!(track(d).width() >= 100.0, "{:?}", track(d));
            }
        }
        // A short past, a short dock; a long one, as wide as the column.
        let column = Rect::new(200.0, 60.0, 900.0, 800.0);
        let (short, long) = (dock(column, 1280.0, 800.0, 120.0), dock(column, 1280.0, 800.0, 5000.0));
        assert!(short.width() < long.width());
        assert_eq!(track(short).width(), 120.0f32.max(track(short).width()));
        assert_eq!(long.width(), DOCK_MOST);
        let m = 60_000;
        assert_eq!(wanted(&[0, m, 2 * m, 2 * m + 2 * HOUR]), 5.0 * MAX_STEP);
    }
}
