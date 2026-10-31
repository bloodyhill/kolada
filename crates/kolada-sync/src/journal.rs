//! Sync's journal (docs/specs/2026-10-26-sync-quick-send.md, section 8):
//! for each path in the vault, the hash both sides last agreed on (its
//! base), the file's stamp when that was taken and a Lamport counter; the
//! changes made here since, waiting to go; and renames, kept 7 days so a
//! late edit to the old name finds the new one.
//!
//! It does no file I/O on the vault: the shell's watcher and the startup
//! scan say what they saw (`seen`, `gone`, `moved`), and a read of a
//! file's bytes is passed in when a stamp changed. It keeps itself in one
//! file, appended to as it changes and rewritten whole when that grows to
//! twice what it holds.
//!
//! Changes go as records (`Record`), packed into batches that each seal
//! into one item (`batches`).
//!
//! Until it has met another device's vault (`meet`, the manifest exchange)
//! a journal is not settled, and nothing goes: a first sync, or one after
//! the server lost the group's items, sends only what differs.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::store::{R, W};

pub type Hash = [u8; 32];

/// A file's version: BLAKE3 of its bytes.
pub fn hash(bytes: &[u8]) -> Hash {
    *blake3::hash(bytes).as_bytes()
}

/// Larger files do not sync (Settings > Devices counts them).
pub const FILE_MOST: u64 = 256 * 1024;
/// A rename is remembered this long.
pub const MOVE_KEEP_MS: u64 = 7 * 86_400_000;
/// A batch holds at most this much (raw), and must seal into one item.
pub const BATCH_MOST: usize = 1024 * 1024;
/// Notes a batch carries at most: the first of a run 50, then 100 (the
/// user's rule, so a first sync or a big import never overwhelms a device).
pub const FIRST_NOTES: usize = 50;
pub const BATCH_NOTES: usize = 100;
/// A manifest part holds at most this much (raw): its hashes don't
/// compress, so this always seals into the largest item.
pub const PART_MOST: usize = 240 * 1024;

/// A version in a manifest: its hash's first 16 bytes.
pub type Short = [u8; 16];

pub fn short(h: &Hash) -> Short {
    let mut s = [0; 16];
    s.copy_from_slice(&h[..16]);
    s
}

const MAGIC: &[u8; 8] = b"KSYNCJ01";

/// When a file was last written and how long it is: unchanged, it is not
/// read again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stamp {
    pub mtime: u64,
    pub len: u64,
}

/// What both sides last agreed a path holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Known {
    pub base: Hash,
    pub stamp: Stamp,
    pub lamport: u64,
}

/// A change made here, not yet sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    /// Written: its hash then, when (ms) and its Lamport counter.
    Put { hash: Hash, edited: u64, lamport: u64 },
    Delete,
}

/// A rename, kept a while: an edit sent to `from` whose base is `hash` is
/// meant for `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    pub from: String,
    pub to: String,
    pub hash: Hash,
    pub at: u64,
}

/// One change, as it goes to the other devices. A put's `edited` is its
/// file's modified time on the device it came from (ms), which the device
/// it goes to gives the file it writes; `created`, when known and the
/// device it goes to reads it (`FEATURE_TIMES`), the file's created time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Put { path: String, text: String, hash: Hash, base: Option<Hash>, edited: u64, lamport: u64, created: Option<u64> },
    Delete { path: String, base: Hash },
    Move { from: String, to: String, hash: Hash },
}

pub struct Journal {
    known: BTreeMap<String, Known>,
    pending: BTreeMap<String, Pending>,
    moves: Vec<Moved>,
    /// Renames made here, not yet sent.
    moves_out: Vec<(String, String, Hash)>,
    /// Files too big to sync, by path.
    too_big: BTreeMap<String, u64>,
    clock: u64,
    file: Option<PathBuf>,
    /// What is waiting to be appended to the file, and how big it is.
    log: Vec<u8>,
    written: u64,
    /// Something only a rewrite drops (old renames): the next save writes
    /// the whole journal.
    compact: bool,
    /// It has met another device's vault since it began, or was last asked
    /// to resync: what it holds alike is agreed.
    settled: bool,
}

// The file's records: one byte of kind, then its fields.
const KNOWN: u8 = 1;
const FORGET: u8 = 2;
const PUT: u8 = 3;
const DELETE: u8 = 4;
const SENT: u8 = 5;
const MOVED: u8 = 6;
const MOVE_OUT: u8 = 7;
const MOVE_SENT: u8 = 8;
const SETTLED: u8 = 9;

impl Journal {
    /// The journal kept in `file` (None: in memory only, for tests and the
    /// extensions). A file cut off halfway reads up to its last whole record.
    pub fn open(file: Option<PathBuf>) -> Journal {
        let mut j = Journal { known: BTreeMap::new(), pending: BTreeMap::new(), moves: Vec::new(), moves_out: Vec::new(), too_big: BTreeMap::new(), clock: 0, file, log: Vec::new(), written: 0, compact: false, settled: false };
        let bytes = j.file.as_ref().and_then(|f| std::fs::read(f).ok()).unwrap_or_default();
        if bytes.starts_with(MAGIC) {
            let mut r = R(&bytes[MAGIC.len()..]);
            while let Some(n) = r.u32() {
                let Some(body) = r.take(n as usize) else { break };
                if j.replay(body).is_none() {
                    break;
                }
            }
            j.written = bytes.len() as u64;
        }
        j
    }

    fn replay(&mut self, body: &[u8]) -> Option<()> {
        let mut r = R(body);
        match r.u8()? {
            KNOWN => {
                let path = r.str()?;
                let k = Known { base: r.arr()?, stamp: Stamp { mtime: r.u64()?, len: r.u64()? }, lamport: r.u64()? };
                self.clock = self.clock.max(k.lamport);
                self.known.insert(path, k);
            }
            FORGET => {
                self.known.remove(&r.str()?);
            }
            PUT => {
                let path = r.str()?;
                let (hash, edited, lamport) = (r.arr()?, r.u64()?, r.u64()?);
                self.clock = self.clock.max(lamport);
                self.pending.insert(path, Pending::Put { hash, edited, lamport });
            }
            DELETE => {
                self.pending.insert(r.str()?, Pending::Delete);
            }
            SENT => {
                self.pending.remove(&r.str()?);
            }
            MOVED => {
                self.moves.push(Moved { from: r.str()?, to: r.str()?, hash: r.arr()?, at: r.u64()? });
            }
            MOVE_OUT => {
                self.moves_out.push((r.str()?, r.str()?, r.arr()?));
            }
            MOVE_SENT => {
                let (from, to) = (r.str()?, r.str()?);
                self.moves_out.retain(|(f, t, _)| *f != from || *t != to);
            }
            SETTLED => self.settled = r.u8()? != 0,
            _ => return None,
        }
        Some(())
    }

    fn note(&mut self, w: &W) {
        self.log.extend_from_slice(&(w.0.len() as u32).to_be_bytes());
        self.log.extend_from_slice(&w.0);
    }

    fn known_record(path: &str, k: &Known) -> W {
        let mut w = W::default();
        w.u8(KNOWN).str(path).raw(&k.base).u64(k.stamp.mtime).u64(k.stamp.len).u64(k.lamport);
        w
    }

    /// Writes what changed since the last save: appended, or the whole
    /// journal again once the file holds twice what it needs.
    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(file) = self.file.clone() else {
            self.log.clear();
            return Ok(());
        };
        if self.log.is_empty() && !self.compact {
            return Ok(());
        }
        let whole = self.whole();
        if std::mem::take(&mut self.compact) || self.written == 0 || self.written + self.log.len() as u64 > 2 * whole.len() as u64 + 64 * 1024 {
            write_whole(&file, &whole)?;
            self.written = whole.len() as u64;
        } else {
            let mut f = std::fs::OpenOptions::new().append(true).open(&file)?;
            f.write_all(&self.log)?;
            self.written += self.log.len() as u64;
        }
        self.log.clear();
        Ok(())
    }

    /// The journal as one file, from scratch.
    fn whole(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        let mut push = |w: W| {
            out.extend_from_slice(&(w.0.len() as u32).to_be_bytes());
            out.extend_from_slice(&w.0);
        };
        for (path, k) in &self.known {
            push(Journal::known_record(path, k));
        }
        for (path, p) in &self.pending {
            let mut w = W::default();
            match p {
                Pending::Put { hash, edited, lamport } => w.u8(PUT).str(path).raw(hash).u64(*edited).u64(*lamport),
                Pending::Delete => w.u8(DELETE).str(path),
            };
            push(w);
        }
        for m in &self.moves {
            let mut w = W::default();
            w.u8(MOVED).str(&m.from).str(&m.to).raw(&m.hash).u64(m.at);
            push(w);
        }
        for (from, to, hash) in &self.moves_out {
            let mut w = W::default();
            w.u8(MOVE_OUT).str(from).str(to).raw(hash);
            push(w);
        }
        if self.settled {
            let mut w = W::default();
            w.u8(SETTLED).u8(1);
            push(w);
        }
        out
    }

    /// Whether it has met another device's vault (see the module's note).
    pub fn settled(&self) -> bool {
        self.settled
    }

    fn set_settled(&mut self, settled: bool) {
        if self.settled != settled {
            self.settled = settled;
            let mut w = W::default();
            w.u8(SETTLED).u8(settled as u8);
            self.note(&w);
        }
    }

    /// A change made here waits to go for `path`.
    pub fn pending(&self, path: &str) -> bool {
        self.pending.contains_key(path)
    }

    /// A file's stamp moved without its text changing (its times were set
    /// to another device's): what was agreed stays, with the new stamp, so
    /// it isn't read again.
    pub fn restamp(&mut self, path: &str, stamp: Stamp) {
        if let Some(k) = self.known.get(path).copied().filter(|k| k.stamp != stamp) {
            self.agreed(path, k.base, stamp, k.lamport);
        }
    }

    /// The server lost the group's items (`resync`): meet another device's
    /// vault again before sending.
    pub fn unsettle(&mut self) {
        self.set_settled(false);
    }

    /// What this vault holds, for another device to compare: each path and
    /// its version, the change waiting to go or else what was agreed. A
    /// path deleted here, or too big, isn't in it.
    pub fn manifest(&self) -> Vec<(String, Short)> {
        let mut out: BTreeMap<&str, Short> = self.known.iter().map(|(p, k)| (p.as_str(), short(&k.base))).collect();
        for (p, v) in &self.pending {
            match v {
                Pending::Put { hash, .. } => out.insert(p.as_str(), short(hash)),
                Pending::Delete => out.remove(p.as_str()),
            };
        }
        out.into_iter().map(|(p, h)| (p.to_string(), h)).collect()
    }

    /// Another device's manifest (`theirs`), met: a note new here that it
    /// holds alike is agreed and never sent, `stamp` giving its file's
    /// stamp; what differs or it lacks still goes to every device. It is
    /// settled from now. What was agreed here before that it lacks or holds
    /// otherwise: an answer for that device alone (the others hold it).
    pub fn meet(&mut self, theirs: &std::collections::HashMap<String, Short>, stamp: impl Fn(&str) -> Option<Stamp>) -> Vec<String> {
        let alike: Vec<(String, Hash, u64)> = self
            .pending
            .iter()
            .filter_map(|(p, v)| match v {
                Pending::Put { hash, lamport, .. } if !self.known.contains_key(p) && theirs.get(p) == Some(&short(hash)) => Some((p.clone(), *hash, *lamport)),
                _ => None,
            })
            .collect();
        for (p, h, lamport) in alike {
            if let Some(st) = stamp(&p) {
                self.agreed(&p, h, st, lamport);
            }
        }
        self.set_settled(true);
        self.known.iter().filter(|(p, k)| !self.pending.contains_key(*p) && theirs.get(*p) != Some(&short(&k.base))).map(|(p, _)| p.clone()).collect()
    }

    pub fn known(&self, path: &str) -> Option<&Known> {
        self.known.get(path)
    }

    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.known.keys()
    }

    /// Changes waiting to go.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty() || !self.moves_out.is_empty()
    }

    /// Files too big to sync: how many.
    pub fn too_big(&self) -> usize {
        self.too_big.len()
    }

    /// The watcher or the scan saw the file at `path` with `stamp`; `read`
    /// gives its bytes when they must be looked at. True if it changed here
    /// since both sides agreed (it waits to go).
    pub fn seen(&mut self, path: &str, stamp: Stamp, read: impl FnOnce() -> Option<Vec<u8>>) -> bool {
        if stamp.len > FILE_MOST {
            self.too_big.insert(path.to_string(), stamp.len);
            return false;
        }
        self.too_big.remove(path);
        let known = self.known.get(path).copied();
        if known.is_some_and(|k| k.stamp == stamp) && !matches!(self.pending.get(path), Some(Pending::Delete)) {
            return false;
        }
        let Some(bytes) = read() else { return false };
        let h = hash(&bytes);
        if let Some(Pending::Put { hash, .. }) = self.pending.get(path)
            && *hash == h
        {
            return false;
        }
        if let Some(k) = known
            && k.base == h
        {
            // Touched, not changed (or changed back): only its stamp moves.
            let k = Known { stamp, ..k };
            self.note(&Journal::known_record(path, &k));
            self.known.insert(path.to_string(), k);
            if self.pending.remove(path).is_some() {
                let mut w = W::default();
                w.u8(SENT).str(path);
                self.note(&w);
            }
            return false;
        }
        self.clock += 1;
        let p = Pending::Put { hash: h, edited: stamp.mtime, lamport: self.clock };
        let mut w = W::default();
        w.u8(PUT).str(path).raw(&h).u64(stamp.mtime).u64(self.clock);
        self.note(&w);
        self.pending.insert(path.to_string(), p);
        if let Some(k) = known {
            let k = Known { stamp, ..k };
            self.note(&Journal::known_record(path, &k));
            self.known.insert(path.to_string(), k);
        }
        true
    }

    /// The file at `path` is gone. True if a delete waits to go.
    pub fn gone(&mut self, path: &str) -> bool {
        self.too_big.remove(path);
        if !self.known.contains_key(path) {
            // Never agreed on: nothing to tell the others.
            if self.pending.remove(path).is_some() {
                let mut w = W::default();
                w.u8(SENT).str(path);
                self.note(&w);
            }
            return false;
        }
        self.pending.insert(path.to_string(), Pending::Delete);
        let mut w = W::default();
        w.u8(DELETE).str(path);
        self.note(&w);
        true
    }

    /// `from` was renamed or moved to `to`, here, at `now`.
    pub fn moved(&mut self, from: &str, to: &str, now: u64) {
        let Some(k) = self.known.remove(from) else {
            // A file never agreed on: it simply goes under its new name.
            if let Some(p) = self.pending.remove(from) {
                self.pending.insert(to.to_string(), p);
                self.rewrite_pending(from, to);
            }
            return;
        };
        let mut w = W::default();
        w.u8(FORGET).str(from);
        self.note(&w);
        self.note(&Journal::known_record(to, &k));
        self.known.insert(to.to_string(), k);
        if let Some(p) = self.pending.remove(from) {
            self.pending.insert(to.to_string(), p);
            self.rewrite_pending(from, to);
        }
        self.remember_move(from, to, k.base, now);
        self.moves_out.push((from.to_string(), to.to_string(), k.base));
        let mut w = W::default();
        w.u8(MOVE_OUT).str(from).str(to).raw(&k.base);
        self.note(&w);
    }

    fn rewrite_pending(&mut self, from: &str, to: &str) {
        let mut w = W::default();
        w.u8(SENT).str(from);
        self.note(&w);
        let mut w = W::default();
        match self.pending[to] {
            Pending::Put { hash, edited, lamport } => w.u8(PUT).str(to).raw(&hash).u64(edited).u64(lamport),
            Pending::Delete => w.u8(DELETE).str(to),
        };
        self.note(&w);
    }

    fn remember_move(&mut self, from: &str, to: &str, hash: Hash, now: u64) {
        let m = Moved { from: from.to_string(), to: to.to_string(), hash, at: now };
        let mut w = W::default();
        w.u8(MOVED).str(&m.from).str(&m.to).raw(&m.hash).u64(m.at);
        self.note(&w);
        self.moves.push(m);
    }

    /// Where an edit sent to `path` on `base` is meant to go: a rename
    /// since (within 7 days) whose hash then was that base sends it on.
    pub fn redirect(&self, path: &str, base: Option<Hash>, now: u64) -> String {
        let mut path = path.to_string();
        for _ in 0..self.moves.len() {
            match self.moves.iter().rev().find(|m| m.from == path && Some(m.hash) == base && now < m.at + MOVE_KEEP_MS) {
                Some(m) => path = m.to.clone(),
                None => break,
            }
        }
        path
    }

    /// Both sides now agree `path` holds `hash` (sync wrote it, or a change
    /// from here went out). Sync's own writes are never sent back.
    pub fn agreed(&mut self, path: &str, hash: Hash, stamp: Stamp, lamport: u64) {
        self.clock = self.clock.max(lamport);
        let k = Known { base: hash, stamp, lamport };
        self.note(&Journal::known_record(path, &k));
        self.known.insert(path.to_string(), k);
        if self.pending.remove(path).is_some() {
            let mut w = W::default();
            w.u8(SENT).str(path);
            self.note(&w);
        }
    }

    /// `path` is gone on every side (a delete went out, or one came in).
    pub fn forgot(&mut self, path: &str) {
        if self.known.remove(path).is_some() {
            let mut w = W::default();
            w.u8(FORGET).str(path);
            self.note(&w);
        }
        if self.pending.remove(path).is_some() {
            let mut w = W::default();
            w.u8(SENT).str(path);
            self.note(&w);
        }
    }

    /// A rename that came in from another device: remembered as one made
    /// here is, without sending it back.
    pub fn moved_there(&mut self, from: &str, to: &str, hash: Hash, now: u64) {
        if let Some(k) = self.known.remove(from) {
            let mut w = W::default();
            w.u8(FORGET).str(from);
            self.note(&w);
            self.note(&Journal::known_record(to, &k));
            self.known.insert(to.to_string(), k);
        }
        self.remember_move(from, to, hash, now);
    }

    /// The Lamport counter, moved past one seen from another device.
    pub fn clock(&self) -> u64 {
        self.clock
    }

    /// The Lamport counter of what `path` holds here: its change waiting
    /// to go, else what was agreed.
    pub fn lamport_of(&self, path: &str) -> u64 {
        match self.pending.get(path) {
            Some(Pending::Put { lamport, .. }) => *lamport,
            _ => self.known.get(path).map_or(0, |k| k.lamport),
        }
    }

    /// The text here stays against what came in (`theirs`, the hash the
    /// others now hold; None when they deleted it): it goes back out to
    /// them, on that base.
    pub fn keep_mine(&mut self, path: &str, theirs: Option<Hash>, mine: Hash, stamp: Stamp) {
        match theirs {
            Some(base) => {
                let k = Known { base, stamp, lamport: self.lamport_of(path) };
                self.note(&Journal::known_record(path, &k));
                self.known.insert(path.to_string(), k);
            }
            None => {
                if self.known.remove(path).is_some() {
                    let mut w = W::default();
                    w.u8(FORGET).str(path);
                    self.note(&w);
                }
            }
        }
        self.clock += 1;
        let mut w = W::default();
        w.u8(PUT).str(path).raw(&mine).u64(stamp.mtime).u64(self.clock);
        self.note(&w);
        self.pending.insert(path.to_string(), Pending::Put { hash: mine, edited: stamp.mtime, lamport: self.clock });
    }

    /// A file found too big to go, though under `FILE_MOST` (it did not
    /// compress into an item): counted, and not tried again until it changes.
    pub fn mark_too_big(&mut self, path: &str, len: u64) {
        self.too_big.insert(path.to_string(), len);
    }

    /// The changes waiting to go, as records: puts read through `text`
    /// (None: the file is gone or unreadable now, and is left for the next
    /// look). Renames first, so a put after one lands under the new name.
    /// Each goes from the waiting list as it is taken; `agreed` then
    /// records what the others now hold.
    pub fn take(&mut self, text: impl Fn(&str) -> Option<String>) -> Vec<Record> {
        let mut out = Vec::new();
        for (from, to, hash) in std::mem::take(&mut self.moves_out) {
            let mut w = W::default();
            w.u8(MOVE_SENT).str(&from).str(&to);
            self.note(&w);
            out.push(Record::Move { from, to, hash });
        }
        let pending: Vec<(String, Pending)> = self.pending.iter().map(|(p, v)| (p.clone(), *v)).collect();
        for (path, p) in pending {
            let base = self.known.get(&path).map(|k| k.base);
            match p {
                Pending::Put { edited, lamport, .. } => {
                    let Some(text) = text(&path) else { continue };
                    // It may have changed again since it was seen: what goes
                    // is what is there now.
                    out.push(Record::Put { path: path.clone(), hash: hash(text.as_bytes()), text, base, edited, lamport, created: None });
                }
                Pending::Delete => {
                    if let Some(base) = base {
                        out.push(Record::Delete { path: path.clone(), base });
                    }
                }
            }
            self.pending.remove(&path);
            let mut w = W::default();
            w.u8(SENT).str(&path);
            self.note(&w);
        }
        out
    }

    /// Renames older than 7 days go.
    pub fn prune(&mut self, now: u64) {
        let before = self.moves.len();
        self.moves.retain(|m| now < m.at + MOVE_KEEP_MS);
        // Only a rewrite drops them from the file.
        self.compact |= self.moves.len() != before;
    }
}

fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

const R_PUT: u8 = 1;
const R_DELETE: u8 = 2;
const R_MOVE: u8 = 3;
/// A put with its created time: only to devices that read it (an app from
/// before reads no batch with one).
const R_PUT_TIMES: u8 = 4;

/// What a device reads, said in each manifest part it sends (a part from
/// before says nothing): a put's created time.
pub const FEATURE_TIMES: u8 = 1;

impl Record {
    fn encode(&self, w: &mut W) {
        match self {
            Record::Put { path, text, hash, base, edited, lamport, created } => {
                w.u8(if created.is_some() { R_PUT_TIMES } else { R_PUT }).str(path).str(text).raw(hash);
                match base {
                    Some(b) => w.u8(1).raw(b),
                    None => w.u8(0),
                };
                w.u64(*edited).u64(*lamport);
                if let Some(c) = created {
                    w.u64(*c);
                }
            }
            Record::Delete { path, base } => {
                w.u8(R_DELETE).str(path).raw(base);
            }
            Record::Move { from, to, hash } => {
                w.u8(R_MOVE).str(from).str(to).raw(hash);
            }
        }
    }

    fn decode(r: &mut R) -> Option<Record> {
        Some(match r.u8()? {
            kind @ (R_PUT | R_PUT_TIMES) => {
                let (path, text, hash) = (r.str()?, r.str()?, r.arr()?);
                let base = match r.u8()? {
                    0 => None,
                    _ => Some(r.arr()?),
                };
                let (edited, lamport) = (r.u64()?, r.u64()?);
                let created = if kind == R_PUT_TIMES { Some(r.u64()?) } else { None };
                Record::Put { path, text, hash, base, edited, lamport, created }
            }
            R_DELETE => Record::Delete { path: r.str()?, base: r.arr()? },
            R_MOVE => Record::Move { from: r.str()?, to: r.str()?, hash: r.arr()? },
            _ => return None,
        })
    }
}

/// Packs records into batches: each at most `BATCH_MOST` raw and small
/// enough, compressed, to seal into one item. A record too big for any
/// item on its own is left out (returned second).
///
/// At most `FIRST_NOTES` records in the first batch, then `BATCH_NOTES`.
/// Each record is compressed once to know about what it takes; a batch is
/// filled while those add up to what fits and its raw size allows, then
/// checked compressed whole (a few times at most when the estimate was
/// high): never a compression a record per batch (a big vault's first
/// sync has thousands).
pub fn batches(records: Vec<Record>) -> (Vec<Vec<u8>>, Vec<Record>) {
    let fits = |b: &[u8]| crate::crypto::pad(b).is_some();
    let batch = |items: &[(Vec<u8>, usize)]| {
        let mut b = W::default();
        b.u32(items.len() as u32);
        for (one, _) in items {
            b.raw(one);
        }
        b.0
    };
    // What one batch can hold compressed: the largest bucket, less the
    // envelope and the length in front.
    let room = crate::crypto::BUCKETS[crate::crypto::BUCKETS.len() - 1] - crate::crypto::OVERHEAD - 4;
    let (mut items, mut left) = (Vec::new(), Vec::new());
    for r in records {
        let mut one = W::default();
        r.encode(&mut one);
        let packed = miniz_oxide::deflate::compress_to_vec(&one.0, 6).len();
        if one.0.len() + 4 > BATCH_MOST || packed + 16 > room || !fits(&batch(&[(one.0.clone(), packed)])) {
            left.push(r);
        } else {
            items.push((one.0, packed));
        }
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < items.len() {
        let (mut raw, mut packed, mut end) = (4, 0, start);
        let most = if out.is_empty() { FIRST_NOTES } else { BATCH_NOTES };
        while end < items.len() && end - start < most && raw + items[end].0.len() <= BATCH_MOST && (end == start || packed + items[end].1 <= room * 97 / 100) {
            raw += items[end].0.len();
            packed += items[end].1;
            end += 1;
        }
        // The estimate holds nearly always; when it doesn't, a tenth fewer.
        while end > start + 1 && !fits(&batch(&items[start..end])) {
            end -= ((end - start) / 10).max(1);
        }
        out.push(batch(&items[start..end]));
        start = end;
    }
    (out, left)
}

/// One part of a manifest, as it goes: which exchange (`nonce`), its place
/// among `count` parts, and whether its sender asks for the other's back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub nonce: u64,
    pub index: u32,
    pub count: u32,
    pub ask: bool,
    pub entries: Vec<(String, Short)>,
    /// What its device reads (`FEATURE_TIMES`); 0 from an app from before.
    pub features: u8,
    /// Each entry's modified and created time on its device (ms; 0 where
    /// unknown), in the entries' order; empty from an app from before.
    pub times: Vec<(u64, u64)>,
}

/// A manifest in parts that each seal into one item (at least one, though
/// it holds nothing).
///
/// `times`, if as long as `entries`, go with them (each entry's modified and
/// created time), after what an app from before reads.
pub fn parts(entries: &[(String, Short)], times: &[(u64, u64)], ask: bool, nonce: u64) -> Vec<Vec<u8>> {
    let mut chunks: Vec<&[(String, Short)]> = Vec::new();
    let (mut start, mut size) = (0, 0);
    for (i, (p, _)) in entries.iter().enumerate() {
        let one = 4 + p.len() + 16 + 16;
        if size + one > PART_MOST && i > start {
            chunks.push(&entries[start..i]);
            (start, size) = (i, 0);
        }
        size += one;
    }
    chunks.push(&entries[start..]);
    let count = chunks.len() as u32;
    let with_times = times.len() == entries.len();
    let mut at = 0;
    chunks
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut w = W::default();
            w.u64(nonce).u32(i as u32).u32(count).u8(ask as u8).u32(c.len() as u32);
            for (p, h) in *c {
                w.str(p).raw(h);
            }
            // What this device reads, then each entry's times, after the
            // entries: an app from before stops reading at them.
            w.u8(FEATURE_TIMES);
            let mine = if with_times { &times[at..at + c.len()] } else { &[][..] };
            w.u32(mine.len() as u32);
            for (m, cr) in mine {
                w.u64(*m).u64(*cr);
            }
            at += c.len();
            w.0
        })
        .collect()
}

/// A manifest part; None if it does not read.
pub fn part(bytes: &[u8]) -> Option<Part> {
    let mut r = R(bytes);
    let (nonce, index, count, ask, n) = (r.u64()?, r.u32()?, r.u32()?, r.u8()? != 0, r.u32()?);
    if index >= count {
        return None;
    }
    let mut entries = Vec::with_capacity(n.min(100_000) as usize);
    for _ in 0..n {
        entries.push((r.str()?, r.arr()?));
    }
    let features = r.u8().unwrap_or(0);
    let mut times = Vec::new();
    if features & FEATURE_TIMES != 0 && r.u32() == Some(entries.len() as u32) {
        for _ in 0..entries.len() {
            let (Some(m), Some(c)) = (r.u64(), r.u64()) else {
                times.clear();
                break;
            };
            times.push((m, c));
        }
    }
    Some(Part { nonce, index, count, ask, entries, features, times })
}

/// A batch's records, in order; None if it does not read.
pub fn unbatch(bytes: &[u8]) -> Option<Vec<Record>> {
    let mut r = R(bytes);
    let n = r.u32()?;
    let mut out = Vec::with_capacity(n.min(10_000) as usize);
    for _ in 0..n {
        out.push(Record::decode(&mut r)?);
    }
    r.0.is_empty().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A put with its created time reads back with it; one without is the
    /// record apps from before read; a manifest part says this device reads
    /// created times, and one from before (without the byte) says nothing.
    #[test]
    fn created_times_go_only_where_they_are_read() {
        let put = |created| Record::Put { path: "N.md".into(), text: "n".into(), hash: hash(b"n"), base: None, edited: 7, lamport: 1, created };
        for r in [put(None), put(Some(5))] {
            let mut w = W::default();
            r.encode(&mut w);
            assert_eq!(w.0[0], if matches!(r, Record::Put { created: Some(_), .. }) { R_PUT_TIMES } else { R_PUT });
            let mut b = W::default();
            b.u32(1).raw(&w.0);
            assert_eq!(unbatch(&b.0), Some(vec![r]));
        }
        let p = parts(&[("N.md".into(), [1; 16])], &[(20, 10)], true, 9).remove(0);
        assert_eq!(part(&p).map(|p| (p.features, p.times)), Some((FEATURE_TIMES, vec![(20, 10)])));
        let before = &p[..p.len() - 1 - 4 - 16];
        assert_eq!(part(before).map(|p| (p.features, p.entries.len(), p.times.len())), Some((0, 1, 0)), "a part from before");
    }

    /// A manifest goes in parts that each seal into one item, and reads back
    /// whole; an empty one is still one part.
    #[test]
    fn a_manifest_goes_in_parts_that_each_fit_an_item() {
        let entries: Vec<(String, Short)> = (0..20_000u32).map(|i| (format!("Projects/Folder {}/A note about item {i}.md", i % 37), short(&hash(&i.to_le_bytes())))).collect();
        let ps = parts(&entries, &[], true, 7);
        assert!(ps.len() > 1, "{} parts", ps.len());
        let mut back = Vec::new();
        for (i, b) in ps.iter().enumerate() {
            assert!(crate::crypto::pad(b).is_some(), "part {i} fits an item");
            let p = part(b).unwrap();
            assert_eq!((p.nonce, p.index, p.count, p.ask), (7, i as u32, ps.len() as u32, true));
            back.extend(p.entries);
        }
        assert_eq!(back, entries);
        let none = parts(&[], &[], false, 1);
        assert_eq!(none.len(), 1);
        assert_eq!(part(&none[0]).map(|p| p.entries.len()), Some(0));
        assert!(part(&none[0][..5]).is_none());
    }

    fn st(mtime: u64, len: u64) -> Stamp {
        Stamp { mtime, len }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-journal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("journal")
    }

    /// A file is read only when its stamp moved, and is a change only when
    /// its bytes differ from what both sides agreed; touched or changed
    /// back, it is not.
    #[test]
    fn a_change_is_what_differs_from_the_base_and_a_stamp_spares_a_read() {
        let mut j = Journal::open(None);
        assert!(j.seen("Plan.md", st(1, 4), || Some(b"plan".to_vec())), "new here: it goes");
        j.agreed("Plan.md", hash(b"plan"), st(1, 4), 1);
        assert!(!j.has_pending());
        assert!(!j.seen("Plan.md", st(1, 4), || panic!("the same stamp is not read")));
        assert!(!j.seen("Plan.md", st(2, 4), || Some(b"plan".to_vec())), "touched, not changed");
        assert!(j.seen("Plan.md", st(3, 5), || Some(b"plans".to_vec())));
        assert!(!j.seen("Plan.md", st(4, 4), || Some(b"plan".to_vec())), "changed back");
        assert!(!j.has_pending());
        // Too big: never read, counted.
        assert!(!j.seen("Big.md", st(1, FILE_MOST + 1), || panic!("not read")));
        assert_eq!(j.too_big(), 1);
        assert!(!j.seen("Big.md", st(2, 10), || None));
        assert_eq!(j.too_big(), 0, "smaller now");
    }

    /// What goes: renames first, then each change as it is now, with its
    /// base, edit time and Lamport counter; a delete only of what was
    /// agreed; a file never agreed on and gone sends nothing.
    #[test]
    fn changes_go_as_records_with_their_base_and_renames_first() {
        let mut j = Journal::open(None);
        j.agreed("A.md", hash(b"a"), st(1, 1), 3);
        j.agreed("B.md", hash(b"b"), st(1, 1), 3);
        j.seen("A.md", st(5, 2), || Some(b"aa".to_vec()));
        j.moved("B.md", "Work/B.md", 10);
        j.gone("A.md");
        j.seen("C.md", st(6, 1), || Some(b"c".to_vec()));
        j.seen("D.md", st(6, 1), || Some(b"d".to_vec()));
        j.gone("D.md");
        let recs = j.take(|p| (p == "C.md").then(|| "c!".to_string()));
        assert_eq!(recs, [
            Record::Move { from: "B.md".into(), to: "Work/B.md".into(), hash: hash(b"b") },
            Record::Delete { path: "A.md".into(), base: hash(b"a") },
            Record::Put { path: "C.md".into(), text: "c!".into(), hash: hash(b"c!"), base: None, edited: 6, lamport: 5, created: None },
        ]);
        assert!(!j.has_pending());
        // An edit sent to the old name, on the base it had, goes on.
        assert_eq!(j.redirect("B.md", Some(hash(b"b")), 11), "Work/B.md");
        assert_eq!(j.redirect("B.md", Some(hash(b"other")), 11), "B.md", "a different base is a new file");
        assert_eq!(j.redirect("B.md", Some(hash(b"b")), 10 + MOVE_KEEP_MS), "B.md", "after 7 days, forgotten");
    }

    /// Kept in one file, appended to; read back after a restart as it was,
    /// a record cut off halfway dropped, rewritten whole once it grows.
    #[test]
    fn the_journal_reads_back_after_a_restart_and_a_torn_write() {
        let file = temp("file");
        let mut j = Journal::open(Some(file.clone()));
        j.agreed("A.md", hash(b"a"), st(1, 1), 7);
        j.seen("B.md", st(2, 1), || Some(b"b".to_vec()));
        j.moved("A.md", "Old/A.md", 50);
        j.save().unwrap();
        let again = Journal::open(Some(file.clone()));
        assert_eq!(again.known("Old/A.md").map(|k| k.base), Some(hash(b"a")));
        assert!(again.known("A.md").is_none());
        assert!(again.has_pending());
        assert_eq!(again.clock(), 8);
        assert_eq!(again.redirect("A.md", Some(hash(b"a")), 60), "Old/A.md");
        // Cut off mid-record: the rest still reads.
        let mut bytes = std::fs::read(&file).unwrap();
        bytes.extend_from_slice(&[0, 0, 0, 40, KNOWN, 1, 2]);
        std::fs::write(&file, &bytes).unwrap();
        let torn = Journal::open(Some(file.clone()));
        assert_eq!(torn.known("Old/A.md").map(|k| k.lamport), Some(7));
        // Many small changes: appended, then rewritten whole.
        let mut j = Journal::open(Some(file.clone()));
        for k in 0..3000u64 {
            j.agreed("A.md", hash(&k.to_be_bytes()), st(k, 8), k);
            j.save().unwrap();
        }
        let size = std::fs::metadata(&file).unwrap().len();
        assert!(size < 200 * 1024, "compacted: {size} bytes");
        assert_eq!(Journal::open(Some(file.clone())).known("A.md").map(|k| k.lamport), Some(2999));
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    /// Batches stay under 1 MB raw and seal into one item each; what can't
    /// fit even alone is left out, and everything reads back in order.
    #[test]
    fn small_notes_go_fifty_first_then_a_hundred_a_batch() {
        let records: Vec<Record> = (0..330).map(|i| Record::Put { path: format!("Note {i}.md"), text: format!("note {i}"), hash: hash(format!("note {i}").as_bytes()), base: None, edited: 1, lamport: 1, created: None }).collect();
        let (b, left) = batches(records);
        let sizes: Vec<usize> = b.iter().map(|x| unbatch(x).unwrap().len()).collect();
        assert_eq!(sizes, [FIRST_NOTES, BATCH_NOTES, BATCH_NOTES, 330 - FIRST_NOTES - 2 * BATCH_NOTES]);
        assert!(left.is_empty());
    }

    #[test]
    fn records_pack_into_batches_that_each_seal_into_one_item() {
        let put = |i: usize, text: String| Record::Put { path: format!("N{i}.md"), hash: hash(text.as_bytes()), text, base: None, edited: i as u64, lamport: i as u64, created: None };
        // Text compresses: many fit one item.
        let small: Vec<Record> = (0..50).map(|i| put(i, format!("# Note {i}\n\n{}", "words and more words ".repeat(200)))).collect();
        let (b, left) = batches(small.clone());
        assert!(left.is_empty());
        assert_eq!(b.iter().map(|x| unbatch(x).unwrap()).flatten().collect::<Vec<_>>(), small);
        assert!(b.iter().all(|x| x.len() <= BATCH_MOST + 4 && crate::crypto::pad(x).is_some()));
        // Text that does not compress: split across items.
        let mut seed = 1u64;
        let mut noise = || {
            (0..120_000)
                .map(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    char::from(b'a' + (seed >> 59) as u8 % 26)
                })
                .collect::<String>()
        };
        let big: Vec<Record> = (0..5).map(|i| put(i, noise())).collect();
        let (b, left) = batches(big.clone());
        assert!(left.is_empty() && b.len() > 1, "{} batches", b.len());
        assert_eq!(b.iter().map(|x| unbatch(x).unwrap()).flatten().collect::<Vec<_>>(), big);
        // Too big for an item on its own.
        let huge = put(9, (0..5).map(|_| noise()).collect());
        let (b, left) = batches(vec![huge.clone()]);
        assert!(b.is_empty() && left == [huge]);
        assert!(unbatch(&[0, 0, 0, 1, 99]).is_none());
    }
}
