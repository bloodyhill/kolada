//! The model's work list: fixes first, then shapes, then gists; a newer job
//! for the same place replaces an older one; a note opened drops the last
//! note's jobs.

use crate::wire::{Kind, Request};

/// What becomes of the job running when another comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Abort {
    No,
    /// Stop it, and do it again after the more urgent job.
    Requeue,
    /// Stop it for good.
    Drop,
}

fn rank(kind: Kind) -> u8 {
    match kind {
        Kind::Fix => 0,
        Kind::Shape => 1,
        Kind::Gist | Kind::Cancel => 2,
    }
}

#[derive(Default)]
pub struct Queue {
    jobs: Vec<Request>,
    /// The note generation last kept by a cancel.
    live: Option<u64>,
}

impl Queue {
    /// Takes `r` in, given the job running now (if any).
    pub fn push(&mut self, r: Request, running: Option<&Request>) -> Abort {
        if r.kind == Kind::Cancel {
            self.jobs.retain(|j| j.note == r.note);
            self.live = Some(r.note);
            return if running.is_some_and(|j| j.note != r.note) { Abort::Drop } else { Abort::No };
        }
        if self.live.is_some_and(|g| g != r.note) {
            return Abort::No;
        }
        self.jobs.retain(|j| !(j.kind == r.kind && j.slot == r.slot));
        let abort = match running {
            Some(j) if j.kind == r.kind && j.slot == r.slot => Abort::Drop,
            Some(j) if j.kind == Kind::Gist && r.kind != Kind::Gist => Abort::Requeue,
            _ => Abort::No,
        };
        self.jobs.push(r);
        abort
    }

    /// Puts back a job stopped for a more urgent one, unless a newer one
    /// for its place came in or its note was closed.
    pub fn requeue(&mut self, r: Request) {
        let stale = self.live.is_some_and(|g| g != r.note) || self.jobs.iter().any(|j| j.kind == r.kind && j.slot == r.slot);
        if !stale {
            self.jobs.insert(0, r);
        }
    }

    /// The most urgent job, the oldest of its kind.
    pub fn pop(&mut self) -> Option<Request> {
        let i = (0..self.jobs.len()).min_by_key(|&i| (rank(self.jobs[i].kind), i))?;
        Some(self.jobs.remove(i))
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(kind: Kind, note: u64, slot: u64, text: &str) -> Request {
        Request { kind, id: 0, note, slot, context: String::new(), text: text.into() }
    }

    #[test]
    fn fixes_come_first_and_newer_jobs_replace_older() {
        let mut q = Queue::default();
        q.push(job(Kind::Gist, 1, 0, "note"), None);
        q.push(job(Kind::Shape, 1, 5, "a, b, c"), None);
        q.push(job(Kind::Fix, 1, 5, "old"), None);
        q.push(job(Kind::Fix, 1, 9, "other"), None);
        q.push(job(Kind::Fix, 1, 5, "new"), None);
        let order: Vec<_> = std::iter::from_fn(|| q.pop()).map(|j| j.text).collect();
        assert_eq!(order, ["other", "new", "a, b, c", "note"]);
    }

    #[test]
    fn a_fix_stops_a_gist_which_comes_back() {
        let mut q = Queue::default();
        let gist = job(Kind::Gist, 1, 0, "note");
        assert_eq!(q.push(job(Kind::Fix, 1, 3, "x"), Some(&gist)), Abort::Requeue);
        q.requeue(gist);
        assert_eq!(q.pop().unwrap().kind, Kind::Fix);
        assert_eq!(q.pop().unwrap().kind, Kind::Gist);
        let fix = job(Kind::Fix, 1, 3, "x");
        assert_eq!(q.push(job(Kind::Fix, 1, 3, "xy"), Some(&fix)), Abort::Drop);
        assert_eq!(q.push(job(Kind::Fix, 1, 4, "z"), Some(&fix)), Abort::No);
    }

    #[test]
    fn opening_another_note_drops_the_last_ones_jobs() {
        let mut q = Queue::default();
        q.push(job(Kind::Fix, 1, 3, "x"), None);
        q.push(job(Kind::Gist, 1, 0, "note"), None);
        let running = job(Kind::Fix, 1, 4, "y");
        assert_eq!(q.push(job(Kind::Cancel, 2, 0, ""), Some(&running)), Abort::Drop);
        assert!(q.is_empty());
        q.requeue(job(Kind::Gist, 1, 0, "note"));
        assert!(q.is_empty());
        q.push(job(Kind::Fix, 1, 9, "late"), None);
        assert!(q.is_empty());
        q.push(job(Kind::Fix, 2, 9, "here"), None);
        assert_eq!(q.pop().unwrap().text, "here");
    }
}
