//! A note's past, for the time scrubber: versions of it kept in the app's
//! data, never in the vault, so the note can be shown as it was at any of
//! them and what it had brought back.
//!
//! Each note has one file of versions, appended to as it is written: a
//! version is the whole text now and then (a keyframe) and otherwise only
//! what changed since the one before (the middle that differs, between the
//! start and end they share). A version written halfway when the app stops
//! is cut off the next time, so the file always reads. Older versions thin
//! out: every one from the last day, one an hour for a week, one a day
//! after that.
//!
//! A version another device wrote (sync) is kept whole, with that device's
//! short name, at its own time among the rest (`insert`). A record of a
//! kind this build does not know, from a newer one, is passed over and
//! kept: the file is never cut short at it, and nothing is built on it.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"NTSYHIS1";
/// A keyframe at least this often, so no version is more than this many
/// changes away from whole text.
const KEYFRAME_EVERY: usize = 64;
const FULL: u8 = 0;
const DELTA: u8 = 1;
/// A whole version from another device: after the head, its short name's
/// length (one byte) and the name, then the text.
const REMOTE: u8 = 2;
/// A record's body starts: kind, time (ms), bytes changed from the version
/// before.
const HEAD: usize = 13;
const HOUR: u64 = 3_600_000;
const DAY: u64 = 24 * HOUR;

/// Where a vault's history lives, under the app's data folder.
pub fn dir(app_data: &Path, vault: &Path) -> PathBuf {
    let key = vault.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    app_data.join("history").join(format!("{:016x}", fnv(key.as_bytes())))
}

/// The file holding the versions of the vault's note at `rel` (relative to
/// the vault, `/` or `\` between folders).
pub fn file(dir: &Path, rel: &str) -> PathBuf {
    let key = rel.replace('\\', "/").to_lowercase();
    dir.join(format!("{:016x}.nh", fnv(key.as_bytes())))
}

/// A note was renamed or moved: its past goes with it.
pub fn rename(dir: &Path, from: &str, to: &str) -> io::Result<()> {
    let (a, b) = (file(dir, from), file(dir, to));
    if a == b || !a.exists() {
        return Ok(());
    }
    fs::rename(a, b)
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

fn check(bytes: &[u8]) -> u32 {
    let h = fnv(bytes);
    (h ^ (h >> 32)) as u32
}

/// One version as read from the file: when it was written, where its record
/// is, how long its text is and how much it changed from the one before.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
    at: u64,
    full: bool,
    /// The record's body: kind, time, then text or change.
    body: (usize, usize),
    /// Where a whole version's text starts in its body.
    text: usize,
    len: usize,
    change: usize,
    /// The device a version came from (its name's bytes in the file).
    from: Option<(usize, usize)>,
}

/// A note's versions, oldest first.
pub struct History {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
    /// Where the file's last whole record ends: what follows (a version
    /// written halfway) is cut off before the next one is added.
    good: usize,
    /// The version last put together, to step from.
    cached: Option<(usize, String)>,
    /// Records of kinds this build does not know, passed over; and whether
    /// the last record was one (the next one written is then whole).
    unknown: usize,
    tail_unknown: bool,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

impl History {
    /// The versions in `path`; none if there is no file or it is not one.
    pub fn open(path: &Path) -> History {
        History::read(fs::read(path).unwrap_or_default())
    }

    fn read(bytes: Vec<u8>) -> History {
        let mut entries: Vec<Entry> = Vec::new();
        let mut at = MAGIC.len();
        if !bytes.starts_with(MAGIC) {
            return History { bytes: Vec::new(), entries, good: 0, cached: None, unknown: 0, tail_unknown: false };
        }
        // Whether the record before is one this build read: a change from
        // one it passed over cannot be put together.
        let (mut base_ok, mut unknown, mut tail_unknown) = (true, 0, false);
        // Each record: body length, body, a check of the body.
        while let Some(n) = u32_at(&bytes, at).map(|n| n as usize) {
            let (start, end) = (at + 4, at + 4 + n);
            let Some(sum) = u32_at(&bytes, end) else { break };
            let body = &bytes[start..end.min(bytes.len())];
            if body.len() != n || n < HEAD || check(body) != sum {
                break;
            }
            let time = u64_at(body, 1).unwrap_or(0);
            let change = u32_at(body, 9).unwrap_or(0) as usize;
            let before = entries.last().map_or(0, |e| e.len);
            let entry = match body[0] {
                FULL => Some(Entry { at: time, full: true, body: (start, end), text: HEAD, len: n - HEAD, change, from: None }),
                REMOTE if n > HEAD => {
                    let name = body[HEAD] as usize;
                    if HEAD + 1 + name > n {
                        break;
                    }
                    let text = HEAD + 1 + name;
                    Some(Entry { at: time, full: true, body: (start, end), text, len: n - text, change, from: Some((start + HEAD + 1, start + text)) })
                }
                DELTA if !base_ok => None,
                DELTA if n >= HEAD + 8 && !entries.is_empty() => {
                    let (front, back) = (u32_at(body, HEAD).unwrap_or(0) as usize, u32_at(body, HEAD + 4).unwrap_or(0) as usize);
                    if front + back > before {
                        break;
                    }
                    Some(Entry { at: time, full: false, body: (start, end), text: HEAD + 8, len: front + n - HEAD - 8 + back, change, from: None })
                }
                // A kind from a newer build: passed over, kept.
                k if k > REMOTE => None,
                _ => break,
            };
            base_ok = entry.is_some();
            tail_unknown = entry.is_none();
            match entry {
                Some(e) => entries.push(e),
                None => unknown += 1,
            }
            at = end + 4;
        }
        History { bytes, entries, good: at, cached: None, unknown, tail_unknown }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// When version `i` was written, milliseconds since the Unix epoch.
    pub fn at(&self, i: usize) -> u64 {
        self.entries[i].at
    }

    /// How much version `i` changed from the one before, in bytes removed
    /// and added (the first: its length).
    pub fn change(&self, i: usize) -> usize {
        self.entries[i].change
    }

    /// The device version `i` came from (its short name, "iPhone"), or None
    /// when it was written here.
    pub fn from(&self, i: usize) -> Option<&str> {
        let (a, b) = self.entries[i].from?;
        std::str::from_utf8(&self.bytes[a..b]).ok()
    }

    /// The note's text at version `i`.
    pub fn text(&mut self, i: usize) -> String {
        if let Some((k, text)) = &self.cached
            && *k == i
        {
            return text.clone();
        }
        // From the cached version when it is on the way, else from the
        // keyframe at or before `i`.
        let key = (0..=i).rev().find(|&k| self.entries[k].full).unwrap_or(0);
        let (mut k, mut text) = match self.cached.take() {
            Some((k, text)) if k <= i && k >= key => (k, text),
            _ => (key, self.full_text(key)),
        };
        while k < i {
            k += 1;
            text = self.apply(k, &text);
        }
        self.cached = Some((i, text.clone()));
        text
    }

    fn full_text(&self, k: usize) -> String {
        let e = self.entries[k];
        String::from_utf8_lossy(&self.bytes[e.body.0 + e.text..e.body.1]).into_owned()
    }

    /// Version `k`'s text, from the text of the one before.
    fn apply(&self, k: usize, before: &str) -> String {
        let e = self.entries[k];
        if e.full {
            return self.full_text(k);
        }
        let (a, b) = e.body;
        let body = &self.bytes[a..b];
        let (front, back) = (u32_at(body, HEAD).unwrap_or(0) as usize, u32_at(body, HEAD + 4).unwrap_or(0) as usize);
        let mut out = Vec::with_capacity(e.len);
        out.extend_from_slice(&before.as_bytes()[..front]);
        out.extend_from_slice(&body[HEAD + 8..]);
        out.extend_from_slice(&before.as_bytes()[before.len() - back..]);
        String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }

    /// The record for `text` at `at`, after `before` (the text of the
    /// version before it, with `since` versions since the last keyframe);
    /// whole, with its device's name, when it came `from` another.
    fn record(text: &str, at: u64, before: Option<&str>, since: usize, from: Option<&str>) -> Vec<u8> {
        let t = text.as_bytes();
        // The start and end it shares with the version before; what lies
        // between them changed.
        let (front, back, change) = match before {
            Some(prev) => {
                let p = prev.as_bytes();
                let mut front = p.iter().zip(t).take_while(|(a, b)| a == b).count();
                while !text.is_char_boundary(front) || !prev.is_char_boundary(front) {
                    front -= 1;
                }
                let most = p.len().min(t.len()) - front;
                let mut back = p.iter().rev().zip(t.iter().rev()).take(most).take_while(|(a, b)| a == b).count();
                while !text.is_char_boundary(t.len() - back) || !prev.is_char_boundary(p.len() - back) {
                    back -= 1;
                }
                (front, back, (p.len() - front - back) + (t.len() - front - back))
            }
            None => (0, 0, t.len()),
        };
        let delta = from.is_none() && before.is_some() && since + 1 < KEYFRAME_EVERY;
        let mut body = Vec::with_capacity(HEAD + 8 + if delta { t.len() - front - back } else { t.len() });
        body.push(if delta { DELTA } else if from.is_some() { REMOTE } else { FULL });
        body.extend_from_slice(&at.to_le_bytes());
        body.extend_from_slice(&(change.min(u32::MAX as usize) as u32).to_le_bytes());
        if let Some(name) = from {
            // At most 255 bytes, cut where a character ends.
            let mut n = name.len().min(255);
            while !name.is_char_boundary(n) {
                n -= 1;
            }
            body.push(n as u8);
            body.extend_from_slice(&name.as_bytes()[..n]);
            body.extend_from_slice(t);
        } else if delta {
            body.extend_from_slice(&(front as u32).to_le_bytes());
            body.extend_from_slice(&(back as u32).to_le_bytes());
            body.extend_from_slice(&t[front..t.len() - back]);
        } else {
            body.extend_from_slice(t);
        }
        let mut out = Vec::with_capacity(body.len() + 8);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&check(&body).to_le_bytes());
        out
    }

    /// Versions since the last keyframe.
    fn since_keyframe(&self) -> usize {
        self.entries.iter().rev().take_while(|e| !e.full).count()
    }
}

/// Keeps `text` as the note's newest version, written at `at` (ms since the
/// Unix epoch; never before the version before it), unless it is what the
/// newest already says. True if it was kept. Now and then the file is
/// thinned (see `thin`).
pub fn record(path: &Path, text: &str, at: u64) -> io::Result<bool> {
    let mut h = History::open(path);
    let n = h.len();
    let latest = (n > 0).then(|| h.text(n - 1));
    if latest.as_deref() == Some(text) {
        return Ok(false);
    }
    let at = at.max(h.entries.last().map_or(0, |e| e.at));
    // After a record passed over, whole: nothing is built on what this
    // build cannot read.
    let since = if h.tail_unknown { KEYFRAME_EVERY } else { h.since_keyframe() };
    let record = History::record(text, at, latest.as_deref(), since, None);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if h.good != h.bytes.len() || h.bytes.is_empty() {
        // New, or a version cut off halfway at the end: write it whole again.
        let mut bytes = if h.bytes.is_empty() { MAGIC.to_vec() } else { h.bytes[..h.good].to_vec() };
        bytes.extend_from_slice(&record);
        write_whole(path, &bytes)?;
    } else {
        let mut f = fs::OpenOptions::new().append(true).open(path)?;
        f.write_all(&record)?;
    }
    if (n + 1).is_multiple_of(128) {
        thin(path, at)?;
    }
    Ok(true)
}

/// Writes a new file beside `path` and puts it in place, so a crash leaves
/// the old one or the new one, never half of either.
fn write_whole(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("nh.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// Which versions to keep at `now`, given when each was written (oldest
/// first): every one from the last day, the newest of each hour for a week,
/// the newest of each day before that. The newest is always kept.
pub fn kept(times: &[u64], now: u64) -> Vec<bool> {
    let bucket = |t: u64| {
        let age = now.saturating_sub(t);
        if age <= DAY {
            None
        } else if age <= 7 * DAY {
            Some((1, t / HOUR))
        } else {
            Some((2, t / DAY))
        }
    };
    (0..times.len()).map(|i| i + 1 == times.len() || bucket(times[i]).is_none() || bucket(times[i]) != bucket(times[i + 1])).collect()
}

/// Rewrites the file at `path` with only the versions `kept` keeps at `now`.
/// A file holding records this build does not know is left as it is.
pub fn thin(path: &Path, now: u64) -> io::Result<()> {
    let mut h = History::open(path);
    let times: Vec<u64> = h.entries.iter().map(|e| e.at).collect();
    let keep = kept(&times, now);
    if keep.iter().all(|&k| k) || h.unknown > 0 {
        return Ok(());
    }
    let versions: Vec<Version> = keep.iter().enumerate().filter(|(_, k)| **k).map(|(i, _)| (h.text(i), h.at(i), h.from(i).map(str::to_string))).collect();
    rewrite(path, &versions)
}

/// A version to write: its text, when, and the device it came from.
type Version = (String, u64, Option<String>);

/// Writes `versions` (oldest first) as the whole file, in place at once.
fn rewrite(path: &Path, versions: &[Version]) -> io::Result<()> {
    let mut bytes = MAGIC.to_vec();
    let (mut before, mut since): (Option<&str>, usize) = (None, 0);
    for (text, at, from) in versions {
        let record = History::record(text, *at, before, since, from.as_deref());
        since = if record[4] == DELTA { since + 1 } else { 0 };
        bytes.extend_from_slice(&record);
        before = Some(text);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    write_whole(path, &bytes)
}

/// Keeps a version another device wrote (`from`, its short name), at its
/// own time `at` among the note's versions, after any at the same time:
/// an older edit lands in the past, not on top. The file is rewritten
/// whole. False if the version just before it already says the same, or
/// the file holds records this build does not know (it is left alone).
pub fn insert(path: &Path, text: &str, at: u64, from: &str) -> io::Result<bool> {
    let mut h = History::open(path);
    if h.unknown > 0 {
        return Ok(false);
    }
    let mut versions: Vec<Version> = (0..h.len()).map(|i| (h.text(i), h.at(i), h.from(i).map(str::to_string))).collect();
    let at_ = versions.partition_point(|v| v.1 <= at);
    if at_ > 0 && versions[at_ - 1].0 == text {
        return Ok(false);
    }
    versions.insert(at_, (text.to_string(), at, Some(from.to_string())));
    rewrite(path, &versions)?;
    Ok(true)
}

/// Lines of `then` matched with lines of `now`, in order: for each line of
/// `then`, the line of `now` it is, or None where it is not in `now` any
/// more (changed or gone). Blank lines are matched to none and count as
/// unchanged; they say nothing of what changed.
pub fn matched(then: &str, now: &str) -> Vec<Option<usize>> {
    let lines: Vec<&str> = now.split('\n').collect();
    let mut at: HashMap<&str, Vec<usize>> = HashMap::new();
    for (j, l) in lines.iter().enumerate().rev() {
        if !l.trim().is_empty() {
            at.entry(l).or_default().push(j);
        }
    }
    // Greedy and in order: each line takes the first unused match after
    // the last one taken, so moved lines read as changed, never crossed.
    let mut next = 0;
    then.split('\n')
        .map(|l| {
            let found = at.get_mut(l)?;
            while found.last().is_some_and(|&j| j < next) {
                found.pop();
            }
            let j = found.pop()?;
            next = j + 1;
            Some(j)
        })
        .collect()
}

/// Which lines of `then` the scrubber marks: those with text that `now`
/// does not have.
pub fn changed(then: &str, now: &str) -> Vec<bool> {
    matched(then, now).iter().zip(then.split('\n')).map(|(m, l)| m.is_none() && !l.trim().is_empty()).collect()
}

/// Bringing back line `i` of `then` into `now`: as a replacement of bytes
/// `start..end` of `now` with the returned text. Where the lines around it
/// are both still there and as many lines lie between them now as then, it
/// takes the place of the one in its position (a paragraph changed since
/// comes back as it was); otherwise it goes back in after the line before
/// it.
pub fn bring_back(then: &str, now: &str, i: usize) -> (usize, usize, String) {
    let old: Vec<&str> = then.split('\n').collect();
    let m = matched(then, now);
    let lines: Vec<&str> = now.split('\n').collect();
    let starts: Vec<usize> = std::iter::once(0).chain(lines.iter().scan(0, |at, l| {
        *at += l.len() + 1;
        Some(*at)
    })).collect();
    // The lines around it that are still there, then and now.
    let before = (0..i).rev().find_map(|k| m[k].map(|j| (k, j)));
    let after = (i + 1..old.len()).find_map(|k| m[k].map(|j| (k, j)));
    let (ok, nk) = before.map_or((None, 0), |(k, j)| (Some(k), j + 1));
    let (oe, ne) = after.unwrap_or((old.len(), lines.len()));
    let gap_then = oe - ok.map_or(0, |k| k + 1);
    let k = i - ok.map_or(0, |k| k + 1);
    let line = old[i].to_string();
    if ne - nk == gap_then && ne > nk {
        let j = nk + k;
        return (starts[j], starts[j] + lines[j].len(), line);
    }
    // In again, after the line before it (at the top if none).
    let j = (nk + k).min(ne);
    if j < lines.len() {
        (starts[j], starts[j], line + "\n")
    } else {
        (now.len(), now.len(), format!("\n{line}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-history-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("note.nh")
    }

    #[test]
    fn versions_are_kept_as_changes_and_read_back_whole() {
        let path = temp("roundtrip");
        let texts = ["Plan\n- milk", "Plan\n- milk\n- oat", "Plan\n- oat milk\n- eggs", "Plan — été\n- oat milk\n- eggs", "", "Fresh start"];
        for (k, t) in texts.iter().enumerate() {
            assert!(record(&path, t, 1_000 + k as u64).unwrap());
        }
        // The same text again is not a version.
        assert!(!record(&path, "Fresh start", 9_000).unwrap());
        let mut h = History::open(&path);
        assert_eq!(h.len(), texts.len());
        for (k, t) in texts.iter().enumerate() {
            assert_eq!(h.text(k), *t, "version {k}");
            assert_eq!(h.at(k), 1_000 + k as u64);
        }
        // Read backward too, past the cache.
        for k in (0..texts.len()).rev() {
            assert_eq!(h.text(k), texts[k]);
        }
        // How much each changed: the second added "\n- oat".
        assert_eq!(h.change(1), "\n- oat".len());
        // Changes, not copies: 100 versions of a long note take little more
        // than two copies of it (a keyframe every 64).
        let size = fs::metadata(&path).unwrap().len() as usize;
        let text: String = "a line of a long note\n".repeat(400);
        for k in 0..100 {
            record(&path, &format!("{text}{k}"), 10_000 + k).unwrap();
        }
        let mut long = History::open(&path);
        assert_eq!(long.len(), texts.len() + 100);
        assert_eq!(long.text(long.len() - 1), format!("{text}99"));
        assert_eq!(long.text(70), format!("{text}{}", 70 - texts.len()));
        // "...98" to "...99": one byte out, one in.
        assert_eq!(long.change(long.len() - 1), 2);
        let grown = fs::metadata(&path).unwrap().len() as usize - size;
        assert!(grown < text.len() * 3, "{grown} bytes for 100 versions of {} bytes", text.len());
    }

    #[test]
    fn a_version_cut_off_halfway_is_dropped_and_the_file_goes_on() {
        let path = temp("torn");
        record(&path, "one", 1).unwrap();
        record(&path, "one two", 2).unwrap();
        // The app stopped while writing the third.
        let mut bytes = fs::read(&path).unwrap();
        let whole = bytes.len();
        bytes.extend_from_slice(&History::record("one two three", 3, Some("one two"), 1, None)[..10]);
        fs::write(&path, &bytes).unwrap();
        assert_eq!(History::open(&path).len(), 2);
        record(&path, "one two four", 4).unwrap();
        let mut h = History::open(&path);
        assert_eq!(h.len(), 3);
        assert_eq!(h.text(2), "one two four");
        assert!(fs::metadata(&path).unwrap().len() as usize > whole);
        // Not a history file at all: no versions, and recording starts one.
        fs::write(&path, b"garbage").unwrap();
        assert!(History::open(&path).is_empty());
        record(&path, "new", 5).unwrap();
        assert_eq!(History::open(&path).text(0), "new");
    }

    /// Another device's version goes in at its own time, whole and named,
    /// between the versions written here; changes before and after it read
    /// back, and thinning keeps the names.
    #[test]
    fn another_devices_version_goes_in_at_its_own_time_with_its_name() {
        let path = temp("remote");
        record(&path, "Plan", 1_000).unwrap();
        record(&path, "Plan\n- milk", 3_000).unwrap();
        // Written on the iPhone between the two, arriving now.
        assert!(insert(&path, "Plan\n- eggs", 2_000, "iPhone").unwrap());
        assert!(!insert(&path, "Plan\n- eggs", 2_000, "iPhone").unwrap(), "once");
        record(&path, "Plan\n- milk\n- tea", 4_000).unwrap();
        let mut h = History::open(&path);
        let all: Vec<(String, u64, Option<&str>)> = (0..h.len()).map(|i| (h.text(i), h.at(i), None)).collect();
        assert_eq!(all.iter().map(|v| (v.0.as_str(), v.1)).collect::<Vec<_>>(), [("Plan", 1_000), ("Plan\n- eggs", 2_000), ("Plan\n- milk", 3_000), ("Plan\n- milk\n- tea", 4_000)]);
        assert_eq!((0..h.len()).map(|i| h.from(i)).collect::<Vec<_>>(), [None, Some("iPhone"), None, None]);
        // A name in any script, cut to 255 bytes where a character ends.
        let long = "\u{0647}".repeat(200);
        insert(&path, "Later", 5_000, &long).unwrap();
        let h = History::open(&path);
        let name = h.from(h.len() - 1).unwrap();
        assert!(name.len() <= 255 && long.starts_with(name));
        // Thinned, the names stay.
        thin(&path, 400 * DAY).unwrap();
        let h = History::open(&path);
        assert_eq!(h.from(h.len() - 1).map(str::len), Some(name.len()));
    }

    /// A newer build's record of a kind this one does not know is passed
    /// over, never cut off: what this build adds goes after it, whole, and
    /// it is still there for the newer build to read.
    #[test]
    fn a_record_of_a_kind_not_known_is_passed_over_and_kept() {
        let path = temp("unknown");
        record(&path, "one", 1).unwrap();
        record(&path, "one two", 2).unwrap();
        // A kind 9 record, then a change built on it.
        let mut body = vec![9u8];
        body.extend_from_slice(&3u64.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(b"from the future");
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&body);
        bytes.extend_from_slice(&check(&body).to_le_bytes());
        bytes.extend_from_slice(&History::record("from the future, changed", 4, Some("from the future"), 0, None));
        fs::write(&path, &bytes).unwrap();
        let mut h = History::open(&path);
        assert_eq!(h.len(), 2, "both passed over");
        assert_eq!(h.text(1), "one two");
        // Written here: after them, whole, nothing cut.
        record(&path, "one two three", 5).unwrap();
        let after = fs::read(&path).unwrap();
        assert!(after.starts_with(&bytes), "the newer records are kept");
        let mut h = History::open(&path);
        assert_eq!(h.len(), 3);
        assert_eq!(h.text(2), "one two three");
        assert_eq!(after[bytes.len() + 4], FULL, "whole, not built on what it can't read");
        // Thinning and inserting leave such a file alone.
        thin(&path, 400 * DAY).unwrap();
        assert!(!insert(&path, "x", 6, "iPhone").unwrap());
        assert!(fs::read(&path).unwrap().starts_with(&bytes));
    }

    #[test]
    fn older_versions_thin_out_by_hour_then_by_day() {
        let now = 30 * DAY;
        let times = [
            now - 20 * DAY,              // a day long ago: kept (its day's last)
            now - 20 * DAY + HOUR,       // same day, later: kept, the first goes
            now - 3 * DAY,               // within the week, its hour's only one
            now - 3 * DAY + 60_000,      // same hour: the newest of it stays
            now - 2 * HOUR,              // today: all kept
            now - 2 * HOUR + 1_000,
            now,
        ];
        assert_eq!(kept(&times, now), [false, true, false, true, true, true, true]);
        let path = temp("thin");
        for (k, &t) in times.iter().enumerate() {
            record(&path, &format!("version {k}"), t).unwrap();
        }
        thin(&path, now).unwrap();
        let mut h = History::open(&path);
        let left: Vec<String> = (0..h.len()).map(|i| h.text(i)).collect();
        assert_eq!(left, ["version 1", "version 3", "version 4", "version 5", "version 6"]);
    }

    #[test]
    fn a_notes_past_follows_it_when_renamed() {
        let path = temp("rename");
        let dir = path.parent().unwrap().to_path_buf();
        record(&file(&dir, "Work/Plan.md"), "plan", 1).unwrap();
        rename(&dir, "Work/Plan.md", "Work/Launch plan.md").unwrap();
        assert!(!file(&dir, "Work/Plan.md").exists());
        assert_eq!(History::open(&file(&dir, "Work\\Launch plan.md")).text(0), "plan");
        // Vaults apart, notes apart.
        assert_ne!(super::dir(Path::new("a"), Path::new(r"C:\notes")), super::dir(Path::new("a"), Path::new(r"C:\other")));
        assert_eq!(super::dir(Path::new("a"), Path::new(r"C:\Notes\")), super::dir(Path::new("a"), Path::new(r"c:\notes")));
    }

    #[test]
    fn lines_that_differ_then_and_now_are_marked() {
        let then = "Plan\n\n- milk\n- eggs\nShip Friday";
        let now = "Plan\n\n- oat milk\n- eggs\nShip Monday\nNew line";
        assert_eq!(changed(then, now), [false, false, true, false, true]);
        // Moved lines read as changed rather than crossing.
        assert_eq!(changed("a\nb\nc", "c\na\nb"), [false, false, true]);
        assert_eq!(changed("same", "same"), [false]);
    }

    #[test]
    fn a_paragraph_brought_back_takes_its_place_or_goes_back_in() {
        let then = "Plan\n- milk\n- eggs\nShip Friday";
        // Changed since: it takes the changed one's place.
        let now = "Plan\n- oat milk\n- eggs\nShip Friday";
        let (a, b, text) = bring_back(then, now, 1);
        assert_eq!(format!("{}{}{}", &now[..a], text, &now[b..]), then);
        // Deleted since: it goes back in where it was.
        let now = "Plan\n- milk\nShip Friday";
        let (a, b, text) = bring_back(then, now, 2);
        assert_eq!(format!("{}{}{}", &now[..a], text, &now[b..]), then);
        // The last line, gone: back at the end.
        let now = "Plan\n- milk\n- eggs";
        let (a, b, text) = bring_back(then, now, 3);
        assert_eq!(format!("{}{}{}", &now[..a], text, &now[b..]), then);
        // The first, changed: its place at the top.
        let now = "Planning\n- milk\n- eggs\nShip Friday";
        let (a, b, text) = bring_back(then, now, 0);
        assert_eq!(format!("{}{}{}", &now[..a], text, &now[b..]), then);
    }
}
