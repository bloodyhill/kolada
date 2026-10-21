//! Putting a frame on the screen: only what changed. The model's surface
//! says where its last frame differs from the one before (`last_damage`);
//! the window's buffer may be the one shown last (age 1), one shown before
//! that (age 2, where the compositor swaps between two), or new (age 0). The
//! parts that buffer lacks are copied in and only they are presented.

use std::collections::VecDeque;

use kolada_ui::raster::PxRect;

/// A part of the window, pixels: left, top, width, height.
pub type Part = (u32, u32, u32, u32);

/// The damage of the last few frames (None: the whole window).
#[derive(Default)]
pub struct History {
    frames: VecDeque<Option<Vec<PxRect>>>,
    /// What is on the screen is not what was drawn: the next frame goes up whole.
    stale: bool,
}

const KEPT: usize = 3;

impl History {
    /// A frame was drawn with this damage (see `Surface::last_damage`).
    pub fn drew(&mut self, damage: Option<Vec<PxRect>>) {
        let damage = if std::mem::take(&mut self.stale) { None } else { damage };
        self.frames.push_front(damage);
        self.frames.truncate(KEPT);
    }

    /// The window changed size, or the display lost what it showed (a
    /// window that covered it went away): what was drawn before counts for
    /// nothing, and the next frame goes up whole, whatever it changed.
    pub fn forget(&mut self) {
        self.frames.clear();
        self.stale = true;
    }

    /// What a buffer `age` frames old lacks, clipped to `w` x `h`: the
    /// damage of the frames drawn since it was shown; everything when it is
    /// new or older than what is kept. Empty when it has it all.
    pub fn lacking(&self, age: u8, w: u32, h: u32) -> Vec<Part> {
        let whole = vec![(0, 0, w, h)];
        let age = age as usize;
        if age == 0 || age > self.frames.len() {
            return whole;
        }
        let mut parts = Vec::new();
        for frame in self.frames.iter().take(age) {
            match frame {
                None => return whole,
                Some(rects) => parts.extend(rects.iter().filter_map(|&r| clip(r, w, h))),
            }
        }
        parts
    }
}

fn clip((l, t, r, b): PxRect, w: u32, h: u32) -> Option<Part> {
    let (l, t) = (l.max(0) as u32, t.max(0) as u32);
    let (r, b) = ((r.max(0) as u32).min(w), (b.max(0) as u32).min(h));
    (r > l && b > t).then_some((l, t, r - l, b - t))
}

/// Copies `parts` of `from` into `to`, both `w` pixels wide.
pub fn copy(from: &[u32], to: &mut [u32], w: u32, parts: &[Part]) {
    for &(x, y, pw, ph) in parts {
        for row in y..y + ph {
            let at = (row * w + x) as usize;
            let n = pw as usize;
            to[at..at + n].copy_from_slice(&from[at..at + n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_the_shown_buffer_lacks_is_copied() {
        let mut h = History::default();
        assert_eq!(h.lacking(1, 100, 50), vec![(0, 0, 100, 50)], "nothing drawn yet: all of it");
        h.drew(None);
        assert_eq!(h.lacking(1, 100, 50), vec![(0, 0, 100, 50)], "the first frame is whole");
        h.drew(Some(vec![(10, 10, 20, 20)]));
        assert_eq!(h.lacking(1, 100, 50), vec![(10, 10, 10, 10)], "the last frame shown: its change only");
        assert_eq!(h.lacking(2, 100, 50), vec![(0, 0, 100, 50)], "two frames back was a whole one");
        h.drew(Some(vec![(90, 40, 120, 60)]));
        assert_eq!(h.lacking(1, 100, 50), vec![(90, 40, 10, 10)], "clipped to the window");
        assert_eq!(h.lacking(2, 100, 50), vec![(90, 40, 10, 10), (10, 10, 10, 10)], "a swapped buffer lacks both changes");
        assert_eq!(h.lacking(0, 100, 50), vec![(0, 0, 100, 50)], "a new buffer lacks everything");
        h.drew(Some(Vec::new()));
        assert!(h.lacking(1, 100, 50).is_empty(), "nothing changed: nothing to copy or present");
        h.forget();
        assert_eq!(h.lacking(1, 100, 50), vec![(0, 0, 100, 50)]);
        // The frame drawn next goes up whole, though it changed a little
        // (seen under Xvfb: a dialog's place stayed black after it closed).
        h.drew(Some(vec![(10, 10, 20, 20)]));
        assert_eq!(h.lacking(1, 100, 50), vec![(0, 0, 100, 50)]);
        h.drew(Some(vec![(10, 10, 20, 20)]));
        assert_eq!(h.lacking(1, 100, 50), vec![(10, 10, 10, 10)], "and after it, only what changes");
    }

    #[test]
    fn parts_are_copied_and_nothing_else() {
        let (w, hh) = (4u32, 3u32);
        let from: Vec<u32> = (0..w * hh).collect();
        let mut to = vec![0u32; (w * hh) as usize];
        copy(&from, &mut to, w, &[(1, 1, 2, 2)]);
        assert_eq!(to, vec![0, 0, 0, 0, 0, 5, 6, 0, 0, 9, 10, 0]);
    }
}
