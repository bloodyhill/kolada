//! A collapsed pill is an elastic string. Where the pointer comes close it
//! draws the string into a smooth swell, the way a magnet draws something
//! soft; as the pointer moves along, the swell follows a moment behind; let
//! go, it eases back. Damped close to critically, so it never bounces: at
//! most one faint overshoot, the way real soft things settle.
//!
//! It is the damped wave equation on a string of points with fixed ends,
//! each point also drawn toward the shape the pointer pulls it into.

use crate::geom::Rect;

/// Points along the string.
pub const SAMPLES: usize = 48;
/// Furthest the string reaches toward the pointer, DIPs.
pub const REACH: f32 = 11.0;
/// Distance at which the string starts to feel the pointer, DIPs.
pub const PROXIMITY: f32 = 180.0;
/// Width of the pull under the pointer (a Gaussian's sigma), DIPs.
const SPREAD: f32 = 22.0;
/// How fast a disturbance runs along the string (DIPs/s), how hard each
/// point springs toward its pulled place (1/s²), and damping (1/s): 0.75 of
/// critical for that spring (2 * sqrt(STIFFNESS)).
const SPEED: f32 = 200.0;
const STIFFNESS: f32 = 320.0;
const DAMPING: f32 = 27.0;
/// Longest integration step, seconds.
const STEP: f32 = 0.002;

#[derive(Clone)]
pub struct Wave {
    /// Each point's displacement toward the pointer, DIPs.
    pub d: [f32; SAMPLES],
    v: [f32; SAMPLES],
}

impl Default for Wave {
    fn default() -> Wave {
        Wave { d: [0.0; SAMPLES], v: [0.0; SAMPLES] }
    }
}

/// Where the pointer pulls each point of `pill` (which runs along y when
/// `vertical`): a swell under the pointer's projection onto it, the fuller
/// the closer the pointer (smoothstep, nothing beyond PROXIMITY). The ends
/// stay put.
pub fn target(pill: Rect, vertical: bool, pointer: Option<(f32, f32)>) -> [f32; SAMPLES] {
    let mut t = [0.0; SAMPLES];
    let Some((x, y)) = pointer else { return t };
    let dx = (pill.left - x).max(x - pill.right).max(0.0);
    let dy = (pill.top - y).max(y - pill.bottom).max(0.0);
    let n = (1.0 - dx.hypot(dy) / PROXIMITY).clamp(0.0, 1.0);
    let strength = n * n * (3.0 - 2.0 * n);
    if strength <= 0.0 {
        return t;
    }
    let (lo, hi, along) = if vertical { (pill.top, pill.bottom, y) } else { (pill.left, pill.right, x) };
    for (i, ti) in t.iter_mut().enumerate() {
        let s = lo + (hi - lo) * i as f32 / (SAMPLES - 1) as f32;
        let swell = (-0.5 * ((s - along) / SPREAD).powi(2)).exp();
        let held = (i.min(SAMPLES - 1 - i) as f32 / 4.0).min(1.0);
        *ti = REACH * strength * swell * held;
    }
    t
}

impl Wave {
    /// Moves the string `dt` seconds on, toward `target`; it is `length`
    /// DIPs long. False once it rests on the target.
    #[allow(clippy::needless_range_loop)] // each point reads its neighbors
    pub fn step(&mut self, dt: f32, length: f32, target: &[f32; SAMPLES]) -> bool {
        let h = (length / (SAMPLES - 1) as f32).max(1.0);
        let steps = (dt / STEP).ceil().max(1.0) as usize;
        let dt = dt / steps as f32;
        // Keep the explicit scheme stable on short strings.
        let c = SPEED.min(0.9 * h / dt);
        let k = c * c / (h * h);
        for _ in 0..steps {
            for i in 0..SAMPLES {
                let before = if i == 0 { 0.0 } else { self.d[i - 1] };
                let after = if i + 1 == SAMPLES { 0.0 } else { self.d[i + 1] };
                let a = k * (before - 2.0 * self.d[i] + after) + STIFFNESS * (target[i] - self.d[i]) - DAMPING * self.v[i];
                self.v[i] += a * dt;
            }
            for i in 0..SAMPLES {
                self.d[i] += self.v[i] * dt;
            }
        }
        let moving = self.d.iter().zip(target).any(|(d, t)| (d - t).abs() > 0.05) || self.v.iter().any(|v| v.abs() > 1.0);
        if !moving {
            self.d = *target;
            self.v = [0.0; SAMPLES];
        }
        moving
    }

    /// How far the string reaches at its most, 0..1 of REACH.
    pub fn pull(d: &[f32]) -> f32 {
        (d.iter().fold(0.0f32, |m, &x| m.max(x.abs())) / REACH).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PILL: Rect = Rect { left: 5.0, top: 200.0, right: 11.0, bottom: 520.0 };
    const FRAME: f32 = 1.0 / 60.0;

    fn run(w: &mut Wave, target: &[f32; SAMPLES], seconds: f32, dt: f32) -> Vec<f32> {
        let mut middle = Vec::new();
        for _ in 0..(seconds / dt) as usize {
            w.step(dt, PILL.height(), target);
            middle.push(w.d[SAMPLES / 2]);
        }
        middle
    }

    #[test]
    fn swells_under_the_pointer_and_nowhere_far_away() {
        let near = target(PILL, true, Some((30.0, 360.0)));
        let peak = near.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert!((peak as i32 - (SAMPLES / 2) as i32).abs() <= 1, "peak at {peak}");
        assert!(near[peak] > 0.5 * REACH && near[peak] <= REACH);
        assert_eq!((near[0], near[SAMPLES - 1]), (0.0, 0.0), "the ends are held");
        assert!(target(PILL, true, Some((600.0, 360.0))).iter().all(|&t| t == 0.0));
        assert!(target(PILL, true, None).iter().all(|&t| t == 0.0));
    }

    #[test]
    fn a_string_at_rest_stays_still() {
        let mut w = Wave::default();
        assert!(!w.step(FRAME, PILL.height(), &[0.0; SAMPLES]));
        assert!(w.d.iter().all(|&d| d == 0.0));
    }

    #[test]
    fn follows_and_lets_go_without_bouncing() {
        let mut w = Wave::default();
        let pulled = target(PILL, true, Some((20.0, 360.0)));
        let held = run(&mut w, &pulled, 1.0, FRAME);
        let (got, want) = (*held.last().unwrap(), pulled[SAMPLES / 2]);
        assert!(got > 0.75 * want && got <= want, "follows the pull: {got} of {want}");
        // Drawn in smoothly: at most a faint overshoot of where it settles.
        let peak = held.iter().fold(0.0f32, |m, &d| m.max(d));
        assert!(peak <= got * 1.06, "no bounce on the way in: peaked at {peak}, settled at {got}");
        assert!(held[12] > 0.8 * got, "and quickly: {} after 0.2 s", held[12]);
        // Let go: it eases back, dipping past rest by a hair at most, once.
        let free = run(&mut w, &[0.0; SAMPLES], 1.0, FRAME);
        let dip = free.iter().fold(0.0f32, |m, &d| m.min(d));
        assert!(-dip < 0.05 * got, "no bounce on the way out: dipped {dip}");
        let crossings = free.windows(2).filter(|p| p[0] > 0.0 && p[1] < 0.0 || p[0] < 0.0 && p[1] > 0.0).count();
        assert!(crossings <= 1, "no ringing: {crossings} crossings");
        assert!(!w.step(FRAME, PILL.height(), &[0.0; SAMPLES]), "settled within a second");
    }

    #[test]
    fn stays_stable_at_any_frame_rate() {
        for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 144.0, 0.1] {
            let mut w = Wave::default();
            let pulled = target(PILL, true, Some((20.0, 260.0)));
            for d in run(&mut w, &pulled, 1.0, dt) {
                assert!(d.is_finite() && d.abs() < 2.0 * REACH, "blew up at dt {dt}: {d}");
            }
            // A short string (the toolbar's) too.
            let mut short = Wave::default();
            for _ in 0..60 {
                short.step(dt, 120.0, &pulled);
            }
            assert!(short.d.iter().all(|d| d.is_finite() && d.abs() < 2.0 * REACH));
        }
    }
}
