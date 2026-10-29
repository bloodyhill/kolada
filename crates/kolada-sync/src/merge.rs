//! What to do with a change that came in (docs/specs/2026-10-26-sync-quick-send.md,
//! section 8, "Applying an incoming put"): decided here from hashes, times
//! and counters alone, so every platform decides alike. Nothing is lost:
//! whichever text does not win the file goes into the note's history.

use crate::engine::DeviceId;
use crate::journal::Hash;

/// An edit time this far in the future counts as now: a device with its
/// clock ahead can't win forever.
pub const SKEW_MOST_MS: u64 = 5 * 60_000;
/// A note typed in this recently keeps its text, whatever comes in.
pub const TYPING_MS: u64 = 10_000;

/// The file here, as the incoming change finds it.
#[derive(Clone, Copy, Debug)]
pub struct Here {
    /// Its hash; None when there is no file.
    pub hash: Option<Hash>,
    /// When it was last written (ms) and its Lamport counter.
    pub edited: u64,
    pub lamport: u64,
    /// Open, and typed in within `TYPING_MS`.
    pub typing: bool,
}

/// A put, as it came.
#[derive(Clone, Copy, Debug)]
pub struct Put {
    pub hash: Hash,
    pub base: Option<Hash>,
    pub edited: u64,
    pub lamport: u64,
    pub from: DeviceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It holds this already.
    Nothing,
    /// The incoming text takes the file; what was here goes into history
    /// first (a fast-forward, or the newer edit winning).
    Write,
    /// The text here stays; the incoming one goes into history at its own
    /// time, marked with its device.
    KeepHere,
}

/// What an incoming put does to the file here, at `now`; `me` is this
/// device (the last tie-break).
pub fn put(here: &Here, put: &Put, me: DeviceId, now: u64) -> Outcome {
    if here.hash == Some(put.hash) {
        return Outcome::Nothing;
    }
    // A note never changes under the cursor of someone typing in it.
    if here.typing && here.hash.is_some() {
        return Outcome::KeepHere;
    }
    match here.hash {
        // New here, or deleted here since: an edit beats a delete.
        None => Outcome::Write,
        // Unchanged here since the base: a fast-forward.
        Some(h) if Some(h) == put.base => Outcome::Write,
        // Both changed since the base (or two new files at one path).
        Some(_) => {
            let clamp = |t: u64| if t > now + SKEW_MOST_MS { now } else { t };
            let theirs = (clamp(put.edited), put.lamport, put.from);
            let mine = (clamp(here.edited), here.lamport, me);
            if theirs > mine { Outcome::Write } else { Outcome::KeepHere }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gone {
    /// Unchanged here: into this device's trash, where it can be restored.
    Trash,
    /// Changed here since: the edit wins, and goes back out.
    Keep,
    /// Not here anyway.
    Nothing,
}

/// What an incoming delete of a file whose agreed hash was `base` does to
/// the file here (`here`: its hash, None when there is none).
pub fn delete(here: Option<Hash>, base: Hash) -> Gone {
    match here {
        None => Gone::Nothing,
        Some(h) if h == base => Gone::Trash,
        Some(_) => Gone::Keep,
    }
}

/// Where an incoming file goes when `path` differs only in case from one
/// here (`taken` says whether a path is taken, ignoring case, as Windows
/// and the Mac do): "Name (2).md", "Name (3).md"... Otherwise `path`.
pub fn free_path(path: &str, same_but_case: Option<&str>, taken: impl Fn(&str) -> bool) -> String {
    if same_but_case.is_none_or(|o| o == path) {
        return path.to_string();
    }
    let (stem, ext) = match path.rfind('.').filter(|&i| i > path.rfind('/').map_or(0, |s| s + 1)) {
        Some(i) => (&path[..i], &path[i..]),
        None => (path, ""),
    };
    (2..).map(|n| format!("{stem} ({n}){ext}")).find(|p| !taken(p)).unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::hash;

    const ME: DeviceId = [1; 10];
    const THEM: DeviceId = [2; 10];
    const NOW: u64 = 1_800_000_000_000;

    fn here(text: Option<&str>, edited: u64, lamport: u64) -> Here {
        Here { hash: text.map(|t| hash(t.as_bytes())), edited, lamport, typing: false }
    }

    fn put_of(text: &str, base: Option<&str>, edited: u64, lamport: u64) -> Put {
        Put { hash: hash(text.as_bytes()), base: base.map(|b| hash(b.as_bytes())), edited, lamport, from: THEM }
    }

    /// The rule's table (spec section 8), row by row.
    #[test]
    fn an_incoming_edit_fast_forwards_or_the_newer_edit_wins() {
        // Unchanged here since the base: a fast-forward, however old.
        assert_eq!(put(&here(Some("v1"), NOW, 9), &put_of("v2", Some("v1"), NOW - 3_600_000, 1), ME, NOW), Outcome::Write);
        // The same already.
        assert_eq!(put(&here(Some("v2"), NOW, 1), &put_of("v2", Some("v1"), NOW, 1), ME, NOW), Outcome::Nothing);
        // Both changed: the later edit wins, the other goes into history.
        assert_eq!(put(&here(Some("mine"), NOW - 60_000, 5), &put_of("theirs", Some("v1"), NOW - 1_000, 2), ME, NOW), Outcome::Write);
        assert_eq!(put(&here(Some("mine"), NOW - 1_000, 2), &put_of("theirs", Some("v1"), NOW - 60_000, 5), ME, NOW), Outcome::KeepHere);
        // Same time: the higher Lamport counter, then the higher device id.
        assert_eq!(put(&here(Some("mine"), NOW, 3), &put_of("theirs", Some("v1"), NOW, 4), ME, NOW), Outcome::Write);
        assert_eq!(put(&here(Some("mine"), NOW, 4), &put_of("theirs", Some("v1"), NOW, 4), ME, NOW), Outcome::Write, "their id is higher");
        assert_eq!(put(&here(Some("mine"), NOW, 4), &put_of("theirs", Some("v1"), NOW, 4), [9; 10], NOW), Outcome::KeepHere, "mine is");
        // Typed in just now: this device's text stays, whatever the times.
        let typing = Here { typing: true, ..here(Some("v1 and typing"), NOW - 9_000, 1) };
        assert_eq!(put(&typing, &put_of("v2", Some("v1"), NOW, 99), ME, NOW), Outcome::KeepHere);
        // Two new files at one path: a conflict with no base.
        assert_eq!(put(&here(Some("mine"), NOW - 5_000, 1), &put_of("theirs", None, NOW, 1), ME, NOW), Outcome::Write);
        // New here, or deleted here since: the edit wins, the file comes back.
        assert_eq!(put(&here(None, 0, 0), &put_of("new", None, NOW, 1), ME, NOW), Outcome::Write);
        assert_eq!(put(&here(None, 0, 0), &put_of("edited", Some("v1"), NOW, 1), ME, NOW), Outcome::Write);
    }

    /// A clock far ahead can't win forever: an edit time more than five
    /// minutes in the future counts as now.
    #[test]
    fn an_edit_from_the_future_counts_as_now() {
        let ahead = put_of("theirs", Some("v1"), NOW + 86_400_000, 1);
        // Mine, a second ago, with the higher counter: theirs counts as now,
        // ties on time, and loses on the counter.
        assert_eq!(put(&here(Some("mine"), NOW, 2), &ahead, ME, NOW), Outcome::KeepHere);
        // A little ahead (within five minutes) is taken as it is.
        assert_eq!(put(&here(Some("mine"), NOW, 2), &put_of("theirs", Some("v1"), NOW + 60_000, 1), ME, NOW), Outcome::Write);
    }

    #[test]
    fn a_delete_takes_only_what_was_not_changed_here() {
        let base = hash(b"v1");
        assert_eq!(delete(Some(base), base), Gone::Trash);
        assert_eq!(delete(Some(hash(b"v1, edited")), base), Gone::Keep, "the edit wins");
        assert_eq!(delete(None, base), Gone::Nothing);
    }

    /// On Windows and the Mac, "plan.md" and "Plan.md" are one file: the
    /// incoming one is kept beside it as "Plan (2).md".
    #[test]
    fn a_path_that_differs_only_in_case_is_kept_beside_as_a_second() {
        assert_eq!(free_path("Work/Plan.md", None, |_| false), "Work/Plan.md");
        assert_eq!(free_path("Work/Plan.md", Some("Work/plan.md"), |_| false), "Work/Plan (2).md");
        assert_eq!(free_path("Work/Plan.md", Some("Work/plan.md"), |p| p == "Work/Plan (2).md"), "Work/Plan (3).md");
        assert_eq!(free_path("v1.2/README", Some("v1.2/readme"), |_| false), "v1.2/README (2)", "a dot in a folder is not an extension");
        assert_eq!(free_path("Plan.md", Some("Plan.md"), |_| true), "Plan.md", "the same path is no twin");
    }
}
