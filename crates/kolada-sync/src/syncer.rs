//! Sync's loop (docs/specs/2026-10-26-sync-quick-send.md, section 8), on a
//! thread of its own so the vault's file work never waits on the UI or the
//! UI on it. It scans the vault once at the start, takes what the shell's
//! watcher saw, sends what changed every `Engine::every` seconds (and only
//! when something did), and applies batches that came in, acknowledging
//! each once applied. The engine is held only for a moment each time.
//!
//! The shell starts it (`Syncer::start`) once paired with a vault open,
//! feeds it (`changed`, `moved`, `typing`, and each `Event::Synced`), and
//! is told what was written (`Applied`), to reload an open note.
//!
//! Before anything goes, the vault meets another device's: it asks one
//! device at a time for its manifest, relaying its own (`Vault::manifest`),
//! and moves on to the next when one is away. The exchange comes in through
//! the engine (`Engine::take_exchange`), so the shell has nothing to pass.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use std::collections::HashMap;

use crate::engine::{DeviceId, Engine, ItemId};
use crate::journal::{self, Short};
use crate::merge::TYPING_MS;
use crate::vault::{Applied, Vault};

/// How long an asked device has to answer before the next is asked, and
/// how long to wait once every device was asked.
const ASK_WAIT_MS: u64 = 30_000;
const ASK_AGAIN_MS: u64 = 60_000;

pub enum Msg {
    Changed(String),
    /// Too much changed to list (the watcher overflowed): look at everything.
    Rescan,
    Moved(String, String),
    Typing(String),
    Batch { id: ItemId, from: DeviceId, name: String, batch: Vec<u8> },
    Stop,
}

/// Where note sync stands, for Settings > Devices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// Another paired device syncs notes with this one
    /// (`Engine::sync_peers`); until then sync is off, and says nothing.
    pub on: bool,
    /// Notes over 256 KB, which don't sync.
    pub too_big: usize,
    /// It has met another device's vault; until then nothing goes.
    pub settled: bool,
}

impl State {
    /// Settings > Devices' lines for it: one each, quiet; none while off.
    pub fn facts(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.on {
            return out;
        }
        if !self.settled {
            out.push("Waiting for another device to come online".to_string());
        }
        match self.too_big {
            0 => {}
            1 => out.push("1 note is too big to sync".to_string()),
            n => out.push(format!("{n} notes are too big to sync")),
        }
        out
    }
}

/// What the loop keeps between messages.
pub struct Work {
    vault: Vault,
    scanned: bool,
    last_out: u64,
    typing: Option<(String, u64)>,
    /// The manifest exchange: when to ask next, who was asked, and the
    /// parts come in so far, by sender and exchange.
    ask_at: u64,
    asked: Vec<DeviceId>,
    parts: HashMap<(DeviceId, u64), (bool, Vec<Option<Vec<(String, Short)>>>)>,
    /// Another device syncs notes with this one (`Engine::sync_peers`).
    on: bool,
    /// The devices every send of the last ten minutes went to, and when
    /// the last went: a manifest from one of them isn't answered with what
    /// is already on its way (`Vault::meet`).
    recent_to: Option<(Vec<DeviceId>, std::time::Instant)>,
}

impl Work {
    pub fn new(vault: Vault) -> Work {
        Work { vault, scanned: false, last_out: 0, typing: None, ask_at: 0, asked: Vec::new(), parts: HashMap::new(), on: false, recent_to: None }
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    pub fn state(&self) -> State {
        State { on: self.on, too_big: self.vault.too_big(), settled: self.vault.settled() }
    }

    /// One message at `now`. What it wrote to the vault.
    pub fn on(&mut self, msg: Msg, engine: &Mutex<Engine>, now: u64) -> Vec<Applied> {
        match msg {
            Msg::Changed(rel) => {
                self.vault.changed(&rel);
            }
            Msg::Rescan => self.scanned = false,
            Msg::Moved(from, to) => self.vault.moved(&from, &to, now),
            Msg::Typing(rel) => self.typing = Some((rel, now)),
            Msg::Batch { id, from, name, batch } => {
                let me = lock(engine).device_id();
                let typing = self.typing.as_ref().filter(|(_, at)| now < at + TYPING_MS).map(|(rel, _)| rel.as_str());
                let done = self.vault.apply(&batch, from, &name, me, now, typing);
                self.vault.save();
                lock(engine).applied(id, now);
                return done;
            }
            Msg::Stop => {}
        }
        Vec::new()
    }

    /// The scan the first time, then what waits, once `every` seconds have
    /// passed since the last send. True if anything went.
    pub fn tick(&mut self, engine: &Mutex<Engine>, now: u64) -> bool {
        // Only with devices that sync notes too: until one does, nothing
        // goes, nothing is taken, no copy is made and the vault isn't even
        // scanned (sync is on by default, paired or not).
        let (every, paired) = {
            let e = lock(engine);
            (e.every().max(1) * 1000, e.paired() && !e.sync_peers().is_empty())
        };
        self.on = paired;
        if !paired {
            // What came in is dropped: nothing is met before the copy.
            let _ = lock(engine).take_exchange();
            return false;
        }
        if !self.scanned {
            self.scanned = true;
            self.vault.scan();
        }
        // Before the first sync, a copy of the whole vault, outside it;
        // nothing goes or comes until it is taken.
        if paired && !self.vault.snapshot_taken() && self.vault.snapshot(now).is_err() {
            return false;
        }
        let met = self.exchange(engine, paired, now);
        // Unpaired, or before meeting another vault, changes wait: taken,
        // they would count as sent.
        if !paired || !self.vault.settled() || !self.vault.has_pending() || now < self.last_out + every {
            return met;
        }
        self.last_out = now;
        let batches = self.vault.outgoing();
        let mut e = lock(engine);
        for b in &batches {
            e.send_sync(b, now);
        }
        if !batches.is_empty() {
            let peers = e.sync_peers();
            self.recent_to = Some(match self.recent_to.take() {
                Some((was, at)) if at.elapsed() < std::time::Duration::from_secs(600) => (was.into_iter().filter(|d| peers.contains(d)).collect(), std::time::Instant::now()),
                _ => (peers, std::time::Instant::now()),
            });
        }
        drop(e);
        self.vault.save();
        !batches.is_empty()
    }
}

impl Work {
    /// The manifest exchange's part of a tick: what came in is met and
    /// answered, and an unsettled vault asks the next device. True if
    /// anything went.
    fn exchange(&mut self, engine: &Mutex<Engine>, paired: bool, now: u64) -> bool {
        let ex = lock(engine).take_exchange();
        let mut went = false;
        if ex.resync {
            self.vault.unsettle();
            (self.ask_at, self.asked) = (0, Vec::new());
        }
        if ex.away.iter().any(|d| self.asked.last() == Some(d)) {
            self.ask_at = now;
        }
        for (from, bytes) in ex.parts {
            // Off, a manifest is not answered: nothing is met before the copy.
            if !paired {
                continue;
            }
            let Some(p) = journal::part(&bytes) else { continue };
            let slot = self.parts.entry((from, p.nonce)).or_insert_with(|| (p.ask, vec![None; p.count as usize]));
            if slot.1.len() != p.count as usize {
                continue;
            }
            slot.1[p.index as usize] = Some(p.entries);
            if slot.1.iter().any(Option::is_none) {
                continue;
            }
            let Some((ask, whole)) = self.parts.remove(&(from, p.nonce)) else { continue };
            let theirs: HashMap<String, Short> = whole.into_iter().flatten().flatten().collect();
            let got = self.recent_to.as_ref().is_some_and(|(to, at)| at.elapsed() < std::time::Duration::from_secs(600) && to.contains(&from));
            let answer = self.vault.meet(&theirs, got);
            self.vault.save();
            let mut e = lock(engine);
            for b in &answer {
                e.send_sync_to(b, Some(&[from]), now);
            }
            if ask {
                for part in self.vault.manifest(false, p.nonce) {
                    e.send_manifest(from, &part);
                }
            }
            went = true;
        }
        if paired && !self.vault.settled() && now >= self.ask_at {
            let mut e = lock(engine);
            let me = e.device_id();
            let next = e.sync_peers().into_iter().find(|d| *d != me && !self.asked.contains(d));
            match next {
                Some(d) => {
                    let nonce = now ^ u64::from_le_bytes([me[0], me[1], me[2], me[3], me[4], me[5], me[6], me[7]]);
                    let parts = self.vault.manifest(true, nonce);
                    if parts.iter().all(|p| e.send_manifest(d, p)) {
                        self.asked.push(d);
                        self.ask_at = now + ASK_WAIT_MS;
                        went = true;
                    } else {
                        // Not connected: try again shortly.
                        self.ask_at = now + 5_000;
                    }
                }
                None => {
                    self.asked.clear();
                    self.ask_at = now + ASK_AGAIN_MS;
                }
            }
        }
        went
    }
}

fn lock(engine: &Mutex<Engine>) -> std::sync::MutexGuard<'_, Engine> {
    engine.lock().unwrap_or_else(|p| p.into_inner())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The loop's handle: messages go to its thread.
pub struct Syncer {
    tx: Sender<Msg>,
    state: Arc<Mutex<State>>,
}

impl Syncer {
    /// Starts the loop for `vault`. `kick` wakes the network thread to send
    /// what the engine queued; `done` is told what was written to the vault
    /// (on the loop's thread).
    pub fn start(vault: Vault, engine: Arc<Mutex<Engine>>, kick: Arc<dyn Fn() + Send + Sync>, done: Arc<dyn Fn(Vec<Applied>) + Send + Sync>) -> Syncer {
        Syncer::open(move || vault, engine, kick, done)
    }

    /// As `start`, opening the vault (`Vault::open`, which reads its
    /// journal) on the loop's thread: the UI's thread never waits on it.
    pub fn open(vault: impl FnOnce() -> Vault + Send + 'static, engine: Arc<Mutex<Engine>>, kick: Arc<dyn Fn() + Send + Sync>, done: Arc<dyn Fn(Vec<Applied>) + Send + Sync>) -> Syncer {
        let (tx, rx) = mpsc::channel();
        // Nothing said until the loop has looked.
        let state = Arc::new(Mutex::new(State { on: false, too_big: 0, settled: true }));
        let shared = state.clone();
        std::thread::Builder::new()
            .name("sync".into())
            .spawn(move || run(Work::new(vault()), &engine, &*kick, &*done, rx, &shared))
            .ok();
        Syncer { tx, state }
    }

    /// Where it stands now (as of its last turn, at most a second ago).
    pub fn state(&self) -> State {
        *self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn changed(&self, rel: &str) {
        let _ = self.tx.send(Msg::Changed(rel.to_string()));
    }

    pub fn rescan(&self) {
        let _ = self.tx.send(Msg::Rescan);
    }

    pub fn moved(&self, from: &str, to: &str) {
        let _ = self.tx.send(Msg::Moved(from.to_string(), to.to_string()));
    }

    /// The open note `rel` was typed in just now.
    pub fn typing(&self, rel: &str) {
        let _ = self.tx.send(Msg::Typing(rel.to_string()));
    }

    /// A batch that came in (`Event::Synced`).
    pub fn batch(&self, id: ItemId, from: DeviceId, name: String, batch: Vec<u8>) {
        let _ = self.tx.send(Msg::Batch { id, from, name, batch });
    }
}

impl Drop for Syncer {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

fn run(mut work: Work, engine: &Mutex<Engine>, kick: &(dyn Fn() + Send + Sync), done: &(dyn Fn(Vec<Applied>) + Send + Sync), rx: Receiver<Msg>, state: &Mutex<State>) {
    loop {
        *state.lock().unwrap_or_else(|p| p.into_inner()) = work.state();
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => {
                work.vault.save();
                return;
            }
            Ok(msg) => {
                let batch = matches!(msg, Msg::Batch { .. });
                let applied = work.on(msg, engine, now());
                if batch {
                    // Its ack goes out.
                    kick();
                }
                if !applied.is_empty() {
                    done(applied);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if work.tick(engine, now()) {
            kick();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{carry, pair, T};

    /// Settings > Devices' lines: one each, none when all is well, and none
    /// at all while sync is off (no other device syncs notes).
    #[test]
    fn settings_says_when_sync_waits_and_what_is_too_big() {
        assert!(State { on: true, too_big: 0, settled: true }.facts().is_empty());
        assert_eq!(State { on: true, too_big: 1, settled: true }.facts(), ["1 note is too big to sync"]);
        assert_eq!(State { on: true, too_big: 3, settled: false }.facts(), ["Waiting for another device to come online", "3 notes are too big to sync"]);
        assert!(State { on: false, too_big: 3, settled: false }.facts().is_empty(), "off: nothing said");
    }
    use crate::Event;
    use std::fs;
    use std::path::PathBuf;

    fn vault(test: &str, name: &str) -> (Vault, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-syncer-{test}-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let root = dir.join("vault");
        fs::create_dir_all(&root).unwrap();
        (Vault::open(&root, &dir.join("data"), true), root)
    }

    /// Two paired devices' loops: an edit on one is sent at the next
    /// interval, not before, reaches the other's vault through the engine,
    /// and is acknowledged once applied; a note typed in keeps its text.
    #[test]
    fn an_edit_goes_at_the_next_interval_and_is_applied_on_the_other_device() {
        let mut ea = Engine::open("https://sync.kolada.app", "desktop", "Desk", None, None);
        let mut eb = Engine::open("https://sync.kolada.app", "ios", "iPhone", None, None);
        // The desk switched on; the phone follows it.
        ea.set_sync(crate::SyncCap::On);
        eb.set_sync(crate::SyncCap::Follow);
        pair(&mut ea, "desktop", &mut eb, "ios");
        let (ea, eb) = (Mutex::new(ea), Mutex::new(eb));
        let ((va, ra), (vb, rb)) = (vault("loop", "a"), vault("loop", "b"));
        let (mut a, mut b) = (Work::new(va), Work::new(vb));
        fs::write(ra.join("Plan.md"), "# Plan\n").unwrap();
        fs::write(ra.join("Same.md"), "alike").unwrap();
        fs::write(rb.join("Same.md"), "alike").unwrap();
        let mut seq = 0;
        let mut go = |from: &Mutex<Engine>, to: &Mutex<Engine>, work: &mut Work, now: u64| -> (usize, usize, Vec<Applied>) {
            let (sends, relays) = carry(&mut lock(from), &mut lock(to), &mut seq);
            let evs = lock(to).events();
            let applied = evs
                .into_iter()
                .filter_map(|e| match e {
                    Event::Synced { id, from, name, batch } => Some(work.on(Msg::Batch { id, from, name, batch }, to, now)),
                    _ => None,
                })
                .flatten()
                .collect();
            (sends, relays, applied)
        };
        // The first tick copies the vault, scans and asks the phone for its
        // manifest: no note goes before the vaults meet.
        assert!(!a.vault().snapshot_taken());
        assert!(a.tick(&ea, T), "it asks");
        assert!(a.vault().snapshot_taken(), "copied first");
        assert!(!a.tick(&ea, T + 1), "and waits for the answer");
        assert_eq!(go(&ea, &eb, &mut b, T + 1), (0, 1, vec![]), "one manifest part, relayed");
        assert!(b.tick(&eb, T + 1), "the phone meets it and answers with its own");
        assert_eq!(go(&eb, &ea, &mut a, T + 1), (0, 1, vec![]));
        assert!(a.tick(&ea, T + 2), "met: what differs goes");
        assert_eq!(go(&ea, &eb, &mut b, T + 2), (1, 0, vec![Applied::Written("Plan.md".into())]), "one batch: the note the phone lacks, not the one alike");
        assert_eq!(fs::read_to_string(rb.join("Plan.md")).unwrap(), "# Plan\n");
        // An edit: it waits for the interval.
        fs::write(ra.join("Plan.md"), "# Plan\n- ship\n").unwrap();
        a.on(Msg::Changed("Plan.md".into()), &ea, T + 3);
        assert!(!a.tick(&ea, T + 4), "not before ten seconds since the last send");
        assert!(a.tick(&ea, T + 10_002));
        // Typed in on the phone just now: its text stays.
        b.on(Msg::Typing("Plan.md".into()), &eb, T + 10_002);
        assert!(go(&ea, &eb, &mut b, T + 10_003).2.is_empty());
        assert_eq!(fs::read_to_string(rb.join("Plan.md")).unwrap(), "# Plan\n");
        let _ = fs::remove_dir_all(ra.parent().unwrap());
        let _ = fs::remove_dir_all(rb.parent().unwrap());
    }

    /// Note sync runs only between devices that say they sync, once one in
    /// the group is switched on: two phones that only follow, or a phone
    /// beside a desktop with sync off, sync nothing, copy nothing and say
    /// nothing in Settings.
    #[test]
    fn notes_sync_only_once_a_device_is_switched_on() {
        use crate::SyncCap::{Follow, Off, On};
        for (a_cap, b_cap, on) in [(Follow, Follow, false), (Off, Follow, false), (On, Off, false), (On, Follow, true), (Follow, On, true), (On, On, true)] {
            let mut ea = Engine::open("https://sync.kolada.app", "ios", "A", None, None);
            let mut eb = Engine::open("https://sync.kolada.app", "ios", "B", None, None);
            ea.set_sync(a_cap);
            eb.set_sync(b_cap);
            pair(&mut ea, "ios", &mut eb, "ios");
            assert_eq!(!ea.sync_peers().is_empty(), on, "{a_cap:?} with {b_cap:?}");
            assert_eq!(!eb.sync_peers().is_empty(), on, "{b_cap:?} with {a_cap:?}");
            let ea = Mutex::new(ea);
            let (va, ra) = vault("caps", &format!("{a_cap:?}{b_cap:?}"));
            fs::write(ra.join("Plan.md"), "# Plan
").unwrap();
            let mut a = Work::new(va);
            assert_eq!(a.tick(&ea, T), on, "{a_cap:?} with {b_cap:?}: asks only when on");
            assert_eq!(a.vault().snapshot_taken(), on, "the copy only before a real sync");
            assert_eq!(a.state().on, on);
            if !on {
                assert!(a.state().facts().is_empty());
                assert!(lock(&ea).send_sync(b"batch", T).is_none(), "nothing goes");
            }
            let _ = fs::remove_dir_all(ra.parent().unwrap());
        }
    }

    /// A big vault's first sync (2,000 notes, some megabytes) goes in
    /// batches of at most 50 notes first, then 100, one at a time (the next
    /// once the server has the last) and never more than `SYNC_PER_MINUTE`
    /// a minute, so it trips no limit, and every note arrives, once.
    #[test]
    fn a_big_first_sync_goes_paced_and_whole() {
        use crate::testing::carry_answered;
        use crate::{SyncCap, SYNC_PER_MINUTE};
        let mut ea = Engine::open("https://sync.kolada.app", "desktop", "Desk", None, None);
        let mut eb = Engine::open("https://sync.kolada.app", "desktop", "Laptop", None, None);
        ea.set_sync(SyncCap::On);
        eb.set_sync(SyncCap::On);
        pair(&mut ea, "desktop", &mut eb, "desktop");
        let (ea, eb) = (Mutex::new(ea), Mutex::new(eb));
        let ((va, ra), (vb, rb)) = (vault("big", "a"), vault("big", "b"));
        for n in 0..2000 {
            // Text that compresses as notes do: words, not noise.
            let words: String = (0..600).map(|i| ["plan", "ship", "note", "idea", "list", "time", "draft", "call"][((n * 31 + i * 7) ^ (i / 3)) % 8]).collect::<Vec<_>>().join(" ");
            // And a few kilobytes that don't compress (pasted ids, keys...).
            let noise: String = (0..160).flat_map(|_| crate::crypto::random::<32>()).map(|b| char::from(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"[(b & 63) as usize])).collect();
            fs::write(ra.join(format!("Note {n}.md")), format!("# Note {n}
{words}
{noise}
")).unwrap();
        }
        let (mut a, mut b) = (Work::new(va), Work::new(vb));
        let mut seq = 0;
        let mut sent_at: Vec<u64> = Vec::new();
        let mut sizes: Vec<usize> = Vec::new();
        let mut now = T;
        for _ in 0..900 {
            now += 1000;
            a.tick(&ea, now);
            b.tick(&eb, now);
            lock(&ea).tick(now);
            lock(&eb).tick(now);
            let n = carry_answered(&mut lock(&ea), &mut lock(&eb), &mut seq);
            sent_at.extend(std::iter::repeat_n(now, n));
            carry_answered(&mut lock(&eb), &mut lock(&ea), &mut seq);
            let evs = lock(&eb).events();
            for e in evs {
                if let Event::Synced { id, from, name, batch } = e {
                    sizes.push(crate::journal::unbatch(&batch).map_or(0, |r| r.len()));
                    b.on(Msg::Batch { id, from, name, batch }, &eb, now);
                }
            }

            // Done once nothing waits and nothing more comes (looked at now and then).
            if (now - T) % 10_000 == 0 && lock(&ea).sync_waiting() == 0 && n == 0 && sent_at.len() > SYNC_PER_MINUTE {
                break;
            }
        }
        let arrived = fs::read_dir(&rb).unwrap().filter(|e| e.as_ref().is_ok_and(|e| e.file_name().to_string_lossy().ends_with(".md"))).count();
        assert_eq!(arrived, 2000, "every note, after {} batches", sent_at.len());
        assert!(sent_at.len() > SYNC_PER_MINUTE, "enough batches to need the pace: {}", sent_at.len());
        for (i, t) in sent_at.iter().enumerate() {
            let in_minute = sent_at[i..].iter().take_while(|u| **u < t + 60_000).count();
            assert!(in_minute <= SYNC_PER_MINUTE, "{in_minute} batches in a minute");
        }
        // The user's rule: the first batch at most 50 notes, then at most 100.
        assert!(sizes[0] <= crate::journal::FIRST_NOTES && sizes.iter().all(|n| *n <= crate::journal::BATCH_NOTES), "batch sizes {sizes:?}");
        assert_eq!(sizes.iter().sum::<usize>(), 2000, "each note once");
        eprintln!("2,000 notes: {} batches, {} s at the pace", sizes.len(), (now - T) / 1000);
        assert!(now - T < 10 * 60_000, "in under ten minutes: {} s", (now - T) / 1000);
        let _ = fs::remove_dir_all(ra.parent().unwrap());
        let _ = fs::remove_dir_all(rb.parent().unwrap());
    }
}
