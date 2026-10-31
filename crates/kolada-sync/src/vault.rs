//! Sync's side of the vault (docs/specs/2026-10-26-sync-quick-send.md,
//! section 8): what changed here goes out as batches, and batches that came
//! in are applied to the files, through `kolada-core` so every platform
//! writes alike (an atomic save, the trash, a note's history).
//!
//! It runs on the shell's sync thread, never the UI's, and does the vault's
//! file I/O itself; the engine only seals and carries the batches. In this
//! first version notes (`.md`) sync, up to 256 KB, as their exact bytes:
//! what is written there hashes as it did here. Every path that comes in is
//! checked to be a plain note path inside the vault before a file is
//! touched.

use std::path::{Path, PathBuf};

use kolada_core::{history, note, trash, vault};

use crate::engine::DeviceId;
use crate::journal::{self, Hash, Journal, Record, Short, Stamp};
use crate::merge::{self, Gone, Here, Outcome, Put};

/// What applying a batch did to the vault, for the shell to show: an open
/// note written is reloaded, as after any outside change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    Written(String),
    Moved { from: String, to: String },
    Trashed(String),
}

pub struct Vault {
    root: PathBuf,
    journal: Journal,
    history: PathBuf,
    trash_list: PathBuf,
    /// The vault's own folder in the app's data (its journal, its
    /// snapshot); None when nothing is kept (tests, the extensions).
    kept: Option<PathBuf>,
    /// Each folder's names, lowercased to the name, read once a batch: a
    /// big first sync looks for case twins thousands of times.
    names: std::collections::HashMap<PathBuf, std::collections::HashMap<String, String>>,
    /// Notes this vault sent lately, and when: they are on their way to
    /// every device that syncs, so a manifest asked before they went isn't
    /// answered with them again (a first sync would send the vault twice).
    recent: std::collections::HashMap<String, std::time::Instant>,
}

/// How long a note sent stays "on its way" for `meet`.
const RECENT: std::time::Duration = std::time::Duration::from_secs(600);

/// Whether `rel` is a note sync carries: a plain note path inside the
/// vault, in no hidden folder (`.obsidian`, `.trash`) and not hidden itself.
pub fn syncs(rel: &str) -> bool {
    vault::is_note_path(rel) && !rel.split('/').any(|part| part.starts_with('.'))
}

/// Every file under `from` copied to the same place under `to`, folders
/// made as needed; links (to files or folders) are left out.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let there = to.join(entry.file_name());
        if kind.is_dir() {
            std::fs::create_dir_all(&there)?;
            copy_tree(&entry.path(), &there)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &there)?;
        }
    }
    Ok(())
}

/// A file's stamp: when it was last written (ms) and its length.
fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
    Some(Stamp { mtime, len: m.len() })
}

/// A file's created time (ms), where the file system keeps one.
fn created(path: &Path) -> Option<u64> {
    let t = std::fs::metadata(path).ok()?.created().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// A note written from another device takes that device's times: its
/// modified time, and where this file is new here, its created time (or,
/// unsaid, the modified time; set where the file system keeps one).
pub(crate) fn set_times(file: &Path, modified: u64, created: Option<u64>) {
    let at = |ms: u64| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms);
    let Ok(f) = std::fs::File::options().write(true).open(file) else { return };
    #[allow(unused_mut)]
    let mut times = std::fs::FileTimes::new().set_modified(at(modified)).set_accessed(at(modified));
    if let Some(c) = created {
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileTimesExt;
            times = times.set_created(at(c));
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::FileTimesExt;
            times = times.set_created(at(c));
        }
        #[cfg(target_os = "ios")]
        {
            use std::os::ios::fs::FileTimesExt;
            times = times.set_created(at(c));
        }
        let _ = c;
    }
    let _ = f.set_times(times);
}

impl Vault {
    /// Sync for the vault at `root`; with `keep`, its journal is kept under
    /// `app_data` (else in memory). Its history and trash are the app's.
    pub fn open(root: &Path, app_data: &Path, keep: bool) -> Vault {
        let history = history::dir(app_data, root);
        let key = history.file_name().map(|k| k.to_os_string()).unwrap_or_default();
        let kept = keep.then(|| app_data.join("sync").join(key));
        let file = kept.as_ref().map(|k| k.join("journal"));
        Vault { root: root.to_path_buf(), journal: Journal::open(file), history, trash_list: app_data.join("trash.tsv"), kept, names: Default::default(), recent: Default::default() }
    }

    /// Whether the copy taken before this vault's first sync is there (or
    /// nothing is kept, so none is needed).
    pub fn snapshot_taken(&self) -> bool {
        self.kept.as_ref().is_none_or(|k| k.join("before-sync").join("taken").exists())
    }

    /// Before this vault's very first sync: a copy of the whole vault (every
    /// file at its own path, pictures and `.obsidian` too; links left out),
    /// in a dated folder under the app's own data (`<app data>/sync/<vault
    /// key>/before-sync/2026-10-28 112233/`, UTC), never in the vault. The
    /// same on every platform. Taken once a vault: a mark beside it says
    /// so, and later syncs take none. Where it went; an error leaves no
    /// mark, so it is tried again and nothing syncs meanwhile.
    pub fn snapshot(&mut self, now_ms: u64) -> std::io::Result<Option<PathBuf>> {
        let Some(kept) = &self.kept else { return Ok(None) };
        let before = kept.join("before-sync");
        if before.join("taken").exists() {
            return Ok(None);
        }
        let secs = now_ms / 1000;
        let day = kolada_core::when::Day::from_number((secs / 86_400) as i64);
        let t = secs % 86_400;
        let to = before.join(format!("{} {:02}{:02}{:02}", day.iso(), t / 3600, t / 60 % 60, t % 60));
        std::fs::create_dir_all(&to)?;
        copy_tree(&self.root, &to)?;
        std::fs::write(before.join("taken"), to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())?;
        Ok(Some(to))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Notes too big to sync, for Settings > Devices.
    pub fn too_big(&self) -> usize {
        self.journal.too_big()
    }

    pub fn has_pending(&self) -> bool {
        self.journal.has_pending()
    }

    /// Whether it has met another device's vault: until then nothing goes
    /// (Settings > Devices: waiting for another device).
    pub fn settled(&self) -> bool {
        self.journal.settled()
    }

    /// The server lost the group's items: meet a vault again first.
    pub fn unsettle(&mut self) {
        self.journal.unsettle();
    }

    /// This vault's manifest, in parts to relay (`Engine::send_manifest`);
    /// with `ask`, the other device answers with its own.
    pub fn manifest(&self, ask: bool, nonce: u64) -> Vec<Vec<u8>> {
        let entries = self.journal.manifest();
        // Each note's times, for a device that holds it alike (`heal_times`).
        let times: Vec<(u64, u64)> = entries.iter().map(|(p, _)| {
            let f = self.root.join(p);
            (stamp(&f).map_or(0, |s| s.mtime), created(&f).unwrap_or(0))
        }).collect();
        journal::parts(&entries, &times, ask, nonce)
    }

    /// The journal's manifest (each path and its version), for tests.
    #[cfg(test)]
    pub fn journal_manifest(&self) -> Vec<(String, Short)> {
        self.journal.manifest()
    }

    /// Notes held alike here and on another device (`theirs`: each path's
    /// version and its modified and created times there) take the earlier
    /// of each time either side has: the later is only when it was copied.
    /// Times alone: nothing written, nothing sent, the same again changes
    /// nothing. How many notes' times moved.
    pub fn heal_times(&mut self, theirs: &std::collections::HashMap<String, (Short, u64, u64)>) -> usize {
        let mut n = 0;
        for (path, (version, m, c)) in theirs {
            let Some(k) = self.journal.known(path).copied() else { continue };
            let file = self.root.join(path);
            // Alike: the same version, and unchanged here since it was agreed.
            let Some(st) = stamp(&file) else { continue };
            if journal::short(&k.base) != *version || st != k.stamp || self.journal.pending(path) {
                continue;
            }
            let earlier = |mine: u64, theirs: u64| if theirs > 0 && theirs < mine { theirs } else { mine };
            let modified = earlier(st.mtime, *m);
            // Only where a created time can be set (Linux reads, never sets one).
            let made = if cfg!(any(windows, target_os = "macos", target_os = "ios")) { created(&file) } else { None };
            let new_made = made.map(|mine| earlier(mine, *c).min(modified)).filter(|t| Some(*t) != made);
            if modified == st.mtime && new_made.is_none() {
                continue;
            }
            set_times(&file, modified, new_made);
            if let Some(now) = stamp(&file) {
                self.journal.restamp(path, now);
            }
            n += 1;
        }
        n
    }

    /// Whether this vault has met another since its times began to be
    /// healed (`heal_times`); else it meets one again, once.
    pub fn times_met(&mut self) -> bool {
        let Some(mark) = self.kept.as_ref().map(|k| k.join("times-met")) else { return true };
        if mark.exists() {
            return true;
        }
        if let Some(dir) = mark.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&mark, b"1");
        false
    }

    /// Another device's whole manifest, met (`journal::Journal::meet`): what
    /// it already holds alike is agreed here and never sent. The batches to
    /// send that device alone: notes agreed here that it lacks or holds
    /// otherwise, each as new to it (no base), so the rule settles a
    /// difference alike on both devices.
    /// `got_recent`: the asking device was among those this vault's recent
    /// sends went to, so the notes sent lately are on their way to it and
    /// aren't sent again (a first sync between two vaults would otherwise
    /// send everything twice); a device new to the group gets them all.
    /// `times`: the device reads a put's created time (`FEATURE_TIMES`).
    pub fn meet(&mut self, theirs: &std::collections::HashMap<String, Short>, got_recent: bool, times: bool) -> Vec<Vec<u8>> {
        let root = self.root.clone();
        let answer = self.journal.meet(theirs, |p| stamp(&root.join(p)));
        self.recent.retain(|_, at| at.elapsed() < RECENT);
        let records: Vec<Record> = answer
            .into_iter()
            .filter(|path| !got_recent || !self.recent.contains_key(path))
            .filter_map(|path| {
                let k = *self.journal.known(&path)?;
                let text = std::fs::read(root.join(&path)).ok().and_then(|b| String::from_utf8(b).ok())?;
                let created = if times { created(&root.join(&path)) } else { None };
                Some(Record::Put { hash: journal::hash(text.as_bytes()), text, base: None, edited: k.stamp.mtime, lamport: k.lamport, created, path })
            })
            .collect();
        journal::batches(records).0
    }

    /// Every note against the journal: edits made while Kolada was closed
    /// wait to go, and notes gone since are deletes. A file is read only
    /// when its stamp moved. How many changed.
    pub fn scan(&mut self) -> usize {
        let notes = vault::every_note(&self.root);
        let mut n = 0;
        for l in &notes {
            n += self.changed(&l.path) as usize;
        }
        let here: std::collections::HashSet<&str> = notes.iter().map(|l| l.path.as_str()).collect();
        let gone: Vec<String> = self.journal.paths().filter(|p| !here.contains(p.as_str())).cloned().collect();
        for p in gone {
            n += self.journal.gone(&p) as usize;
        }
        n
    }

    /// The watcher saw `rel` change (written, created or deleted). True if
    /// a change now waits to go.
    pub fn changed(&mut self, rel: &str) -> bool {
        if !syncs(rel) {
            return false;
        }
        let path = self.root.join(rel);
        match stamp(&path) {
            Some(st) => self.journal.seen(rel, st, || std::fs::read(&path).ok().filter(|b| std::str::from_utf8(b).is_ok())),
            None => self.journal.gone(rel),
        }
    }

    /// A note was renamed or moved here.
    pub fn moved(&mut self, from: &str, to: &str, now: u64) {
        if syncs(from) && syncs(to) {
            self.journal.moved(from, to, now);
        }
    }

    /// What waits to go, as batches to seal (each fits one item). What is
    /// taken counts as agreed: the others hold it once they apply it.
    /// `times`: every device they go to reads a put's created time.
    pub fn outgoing(&mut self, times: bool) -> Vec<Vec<u8>> {
        let root = self.root.clone();
        let mut records = self.journal.take(|p| std::fs::read(root.join(p)).ok().and_then(|b| String::from_utf8(b).ok()));
        if times {
            for r in &mut records {
                if let Record::Put { path, created: c, .. } = r {
                    *c = created(&root.join(path.as_str()));
                }
            }
        }
        let (batches, left) = journal::batches(records.clone());
        for r in &records {
            match r {
                Record::Put { path, hash, lamport, .. } => {
                    if left.contains(r) {
                        self.journal.mark_too_big(path, stamp(&root.join(path)).map_or(0, |s| s.len));
                    } else if let Some(st) = stamp(&root.join(path)) {
                        self.journal.agreed(path, *hash, st, *lamport);
                        self.recent.insert(path.clone(), std::time::Instant::now());
                    }
                }
                Record::Delete { path, .. } => self.journal.forgot(path),
                Record::Move { .. } => {}
            }
        }
        batches
    }

    /// Applies a batch that came from `from` (its short name `from_name`)
    /// at `now`; `typing` is the note open and typed in just now, if any.
    pub fn apply(&mut self, batch: &[u8], from: DeviceId, from_name: &str, me: DeviceId, now: u64, typing: Option<&str>) -> Vec<Applied> {
        let Some(records) = journal::unbatch(batch) else { return Vec::new() };
        let mut out = Vec::new();
        self.names.clear();
        for r in records {
            match r {
                Record::Put { path, text, hash, base, edited, lamport, created } => {
                    if !syncs(&path) || journal::hash(text.as_bytes()) != hash {
                        continue;
                    }
                    let put = Put { hash, base, edited, lamport, from };
                    if let Some(done) = self.apply_put(&path, &text, &put, created, from_name, me, now, typing) {
                        out.push(done);
                    }
                }
                Record::Delete { path, base } => {
                    if !syncs(&path) {
                        continue;
                    }
                    let path = self.journal.redirect(&path, Some(base), now);
                    let file = self.root.join(&path);
                    let bytes = std::fs::read(&file).ok();
                    match merge::delete(bytes.as_deref().map(journal::hash), base) {
                        Gone::Trash => {
                            if let Some(text) = bytes.as_deref().and_then(|b| std::str::from_utf8(b).ok()) {
                                let _ = history::record(&history::file(&self.history, &path), text, now);
                            }
                            if trash::trash(&self.root, &path, &self.trash_list, now).is_ok() {
                                self.journal.forgot(&path);
                                out.push(Applied::Trashed(path));
                            }
                        }
                        // Changed here since: the edit wins and goes back out.
                        Gone::Keep => {
                            if let (Some(b), Some(st)) = (&bytes, stamp(&file)) {
                                self.journal.keep_mine(&path, None, journal::hash(b), st);
                            }
                        }
                        Gone::Nothing => self.journal.forgot(&path),
                    }
                }
                Record::Move { from: a, to: b, hash } => {
                    if !syncs(&a) || !syncs(&b) {
                        continue;
                    }
                    let (fa, fb) = (self.root.join(&a), self.root.join(&b));
                    if fa.exists() && !fb.exists() {
                        if let Some(dir) = fb.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        if std::fs::rename(&fa, &fb).is_err() {
                            continue;
                        }
                        let _ = history::rename(&self.history, &a, &b);
                        out.push(Applied::Moved { from: a.clone(), to: b.clone() });
                    }
                    // Remembered either way: a late edit to the old name follows.
                    self.journal.moved_there(&a, &b, hash, now);
                }
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_put(&mut self, path: &str, text: &str, put: &Put, created: Option<u64>, from_name: &str, me: DeviceId, now: u64, typing: Option<&str>) -> Option<Applied> {
        let mut path = self.journal.redirect(path, put.base, now);
        // "Plan.md" here and "plan.md" coming: both kept, the new one beside.
        if self.journal.known(&path).is_none()
            && let Some(twin) = self.case_twin(&path)
        {
            let root = self.root.clone();
            path = merge::free_path(&path, Some(&twin), |p| root.join(p).exists());
        }
        let file = self.root.join(&path);
        let bytes = std::fs::read(&file).ok();
        let st = stamp(&file).unwrap_or_default();
        let here = Here { hash: bytes.as_deref().map(journal::hash), edited: st.mtime, lamport: self.journal.lamport_of(&path), typing: typing == Some(path.as_str()) };
        let past = history::file(&self.history, &path);
        match merge::put(&here, put, me, now) {
            Outcome::Nothing => {
                self.journal.agreed(&path, put.hash, st, put.lamport);
                None
            }
            Outcome::Write => {
                // What was here first goes into the note's past.
                if let Some(old) = bytes.as_deref().and_then(|b| std::str::from_utf8(b).ok()) {
                    let _ = history::record(&past, old, st.mtime);
                }
                if let Some(dir) = file.parent() {
                    std::fs::create_dir_all(dir).ok()?;
                }
                note::save(&file, text, note::Format::default()).ok()?;
                // Its times are the other device's, never now's: a new note
                // keeps when it was made there too (else when it was last
                // written). Set before the journal takes its stamp, so it
                // never counts as a change made here.
                let modified = put.edited.min(now);
                set_times(&file, modified, bytes.is_none().then(|| created.unwrap_or(modified).min(modified)));
                if let (Some(dir), Some(name)) = (file.parent(), file.file_name()) {
                    let name = name.to_string_lossy().into_owned();
                    if let Some(known) = self.names.get_mut(dir) {
                        known.insert(name.to_lowercase(), name);
                    }
                }
                let _ = history::insert(&past, text, put.edited.min(now), from_name);
                self.journal.agreed(&path, put.hash, stamp(&file)?, put.lamport);
                Some(Applied::Written(path))
            }
            Outcome::KeepHere => {
                // Theirs goes into the past, at its own time; mine goes back out.
                let _ = history::insert(&past, text, put.edited.min(now), from_name);
                if let Some(mine) = here.hash {
                    self.journal.keep_mine(&path, Some(put.hash), mine, st);
                }
                None
            }
        }
    }

    /// Another file in the same folder whose name differs from `rel`'s only
    /// in case, where the file system treats them as one.
    fn case_twin(&mut self, rel: &str) -> Option<String> {
        if !cfg!(any(windows, target_os = "macos", target_os = "ios")) {
            return None;
        }
        let file = self.root.join(rel);
        let name = file.file_name()?.to_string_lossy().into_owned();
        let dir = file.parent()?.to_path_buf();
        let known = self.names.entry(dir.clone()).or_insert_with(|| {
            std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).map(|n| (n.to_lowercase(), n)).collect()).unwrap_or_default()
        });
        let twin = known.get(&name.to_lowercase()).filter(|n| **n != name).cloned()?;
        Some(match rel.rfind('/') {
            Some(i) => format!("{}/{twin}", &rel[..i]),
            None => twin,
        })
    }

    pub fn save(&mut self) {
        let _ = self.journal.save();
    }

    /// The journal's hash for `rel`, for tests and Settings.
    pub fn base(&self, rel: &str) -> Option<Hash> {
        self.journal.known(rel).map(|k| k.base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const A: DeviceId = [1; 10];
    const B: DeviceId = [2; 10];

    struct Side {
        vault: Vault,
        root: PathBuf,
        data: PathBuf,
        id: DeviceId,
        name: &'static str,
    }

    fn side(test: &str, name: &'static str, id: DeviceId) -> Side {
        let dir = std::env::temp_dir().join(format!("kolada-vault-sync-{test}-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (root, data) = (dir.join("vault"), dir.join("data"));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&data).unwrap();
        Side { vault: Vault::open(&root, &data, true), root, data, id, name }
    }

    impl Side {
        fn write(&self, rel: &str, text: &str) {
            let f = self.root.join(rel);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, text).unwrap();
        }
        fn read(&self, rel: &str) -> Option<String> {
            fs::read_to_string(self.root.join(rel)).ok()
        }
        fn past(&self, rel: &str) -> Vec<(String, Option<String>)> {
            let mut h = history::History::open(&history::file(&history::dir(&self.data, &self.root), rel));
            (0..h.len()).map(|i| (h.text(i), h.from(i).map(str::to_string))).collect()
        }
    }

    /// What `a` has to send, applied on `b`.
    fn send(a: &mut Side, b: &mut Side, now: u64, typing: Option<&str>) -> Vec<Applied> {
        let batches = a.vault.outgoing(true);
        batches.iter().flat_map(|x| b.vault.apply(x, a.id, a.name, b.id, now, typing)).collect()
    }

    const NOW: u64 = 1_800_000_000_000;

    /// The first sync between two vaults with notes copies what one side
    /// lacks and deletes nothing; then an edit, a rename and a delete follow
    /// from one to the other, and nothing comes back.
    #[test]
    fn notes_go_both_ways_and_edits_renames_and_deletes_follow() {
        let (mut a, mut b) = (side("both", "Desk", A), side("both", "iPhone", B));
        a.write("Plan.md", "# Plan\n");
        a.write("Work/Ideas.md", "ideas");
        b.write("Shopping.md", "milk");
        a.write(".obsidian/app.json", "{}");
        assert_eq!(a.vault.scan(), 2, "notes only; hidden folders skipped");
        b.vault.scan();
        assert_eq!(send(&mut a, &mut b, NOW, None).len(), 2);
        assert_eq!(send(&mut b, &mut a, NOW, None).len(), 1);
        assert_eq!(b.read("Work/Ideas.md").as_deref(), Some("ideas"));
        assert_eq!(a.read("Shopping.md").as_deref(), Some("milk"));
        assert!(b.read(".obsidian/app.json").is_none());
        // Nothing echoes back.
        a.vault.scan();
        b.vault.scan();
        assert!(!a.vault.has_pending() && !b.vault.has_pending());
        // An edit on the phone.
        b.write("Plan.md", "# Plan\n- ship\n");
        b.vault.changed("Plan.md");
        assert_eq!(send(&mut b, &mut a, NOW + 10, None), [Applied::Written("Plan.md".into())]);
        assert_eq!(a.read("Plan.md").as_deref(), Some("# Plan\n- ship\n"));
        // The old text is in the note's past, the new one marked as the phone's.
        assert_eq!(a.past("Plan.md"), [("# Plan\n".to_string(), None), ("# Plan\n- ship\n".to_string(), Some("iPhone".to_string()))]);
        // A rename on the desk, then a delete.
        fs::rename(a.root.join("Work/Ideas.md"), a.root.join("Ideas.md")).unwrap();
        a.vault.moved("Work/Ideas.md", "Ideas.md", NOW + 20);
        fs::remove_file(a.root.join("Shopping.md")).unwrap();
        a.vault.changed("Shopping.md");
        let applied = send(&mut a, &mut b, NOW + 30, None);
        assert_eq!(applied, [Applied::Moved { from: "Work/Ideas.md".into(), to: "Ideas.md".into() }, Applied::Trashed("Shopping.md".into())]);
        assert_eq!(b.read("Ideas.md").as_deref(), Some("ideas"));
        assert!(b.read("Shopping.md").is_none());
        assert!(b.root.join(".trash").read_dir().unwrap().count() == 1, "into the trash, where it can be restored");
    }

    /// `a`'s whole manifest, met by `b`: what `b` answers `a` alone, and
    /// (when asked) `b`'s manifest, met by `a` in turn.
    fn meet(a: &mut Side, b: &mut Side, ask: bool) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let whole = |parts: Vec<Vec<u8>>| -> std::collections::HashMap<String, Short> { parts.iter().flat_map(|p| journal::part(p).unwrap().entries).collect() };
        let to_a = b.vault.meet(&whole(a.vault.manifest(ask, 1)), false, true);
        let to_b = if ask { a.vault.meet(&whole(b.vault.manifest(false, 1)), false, true) } else { Vec::new() };
        (to_a, to_b)
    }

    fn records(batches: &[Vec<u8>]) -> Vec<String> {
        let mut paths: Vec<String> = batches
            .iter()
            .flat_map(|b| journal::unbatch(b).unwrap())
            .map(|r| match r {
                Record::Put { path, .. } | Record::Delete { path, .. } => path,
                Record::Move { to, .. } => to,
            })
            .collect();
        paths.sort();
        paths
    }

    /// Two vaults that both hold notes meet before anything goes: a note
    /// they hold alike is agreed and never sent; only what differs or one
    /// lacks goes, and both end alike, deleting nothing. Before they meet,
    /// a vault is not settled.
    #[test]
    fn a_first_sync_sends_only_what_differs() {
        let (mut a, mut b) = (side("meet", "Desk", A), side("meet", "iPhone", B));
        for i in 0..50 {
            a.write(&format!("Same {i}.md"), "alike");
            b.write(&format!("Same {i}.md"), "alike");
        }
        a.write("Desk only.md", "a");
        b.write("Phone only.md", "b");
        b.write("Both.md", "the phone's");
        std::thread::sleep(std::time::Duration::from_millis(30));
        a.write("Both.md", "the desk's");
        a.vault.scan();
        b.vault.scan();
        assert!(!a.vault.settled() && !b.vault.settled());
        let (to_a, to_b) = meet(&mut a, &mut b, true);
        assert!(to_a.is_empty() && to_b.is_empty(), "nothing was agreed before: no answers");
        assert!(a.vault.settled() && b.vault.settled());
        let (out_a, out_b) = (a.vault.outgoing(true), b.vault.outgoing(true));
        assert_eq!(records(&out_a), ["Both.md", "Desk only.md"]);
        assert_eq!(records(&out_b), ["Both.md", "Phone only.md"]);
        for x in &out_a {
            b.vault.apply(x, A, "Desk", B, NOW, None);
        }
        for x in &out_b {
            a.vault.apply(x, B, "iPhone", A, NOW, None);
        }
        for s in [&a, &b] {
            assert_eq!(s.read("Desk only.md").as_deref(), Some("a"), "{}", s.name);
            assert_eq!(s.read("Phone only.md").as_deref(), Some("b"), "{}", s.name);
            assert_eq!(s.read("Both.md").as_deref(), Some("the desk's"), "{}: the newer, on both", s.name);
            assert_eq!(s.read("Same 7.md").as_deref(), Some("alike"));
        }
        assert!(b.past("Both.md").iter().any(|v| v.0 == "the phone's"), "nothing lost");
        assert_eq!(a.vault.base("Same 7.md"), b.vault.base("Same 7.md"));
        assert!(a.vault.base("Same 7.md").is_some(), "agreed");
    }

    /// A device whose items the server lost (resync), or one new to the
    /// group, meets a vault that has synced before: the notes agreed there
    /// that it lacks or holds otherwise come to it alone, and the settled
    /// state survives a restart.
    #[test]
    fn a_device_that_resyncs_is_answered_with_what_it_lacks() {
        let (mut a, mut b) = (side("resync", "Desk", A), side("resync", "Laptop", B));
        a.write("Plan.md", "plan");
        a.write("Old.md", "old");
        a.vault.scan();
        let (mut c, _) = (side("resync", "iPhone", [3; 10]), ());
        meet(&mut a, &mut c, true);
        for x in a.vault.outgoing(true) {
            c.vault.apply(&x, A, "Desk", c.id, NOW, None);
        }
        assert!(a.vault.base("Plan.md").is_some() && a.vault.settled());
        a.vault.save();
        let again = Vault::open(&a.root, &a.data, true);
        assert!(again.settled(), "kept across a restart");
        // The laptop holds one note, written otherwise.
        b.write("Plan.md", "plan, the laptop's");
        b.vault.scan();
        let (to_b, _) = meet(&mut b, &mut a, false);
        assert_eq!(records(&to_b), ["Old.md", "Plan.md"], "what it lacks or holds otherwise, to it alone");
        for x in &to_b {
            b.vault.apply(x, A, "Desk", B, NOW, None);
        }
        assert_eq!(b.read("Old.md").as_deref(), Some("old"));
        // A resync asks again.
        a.vault.unsettle();
        assert!(!a.vault.settled());
    }

    /// Before a vault's first sync, a dated copy of the whole vault, every
    /// file at its own path, in the app's own data and never in the vault;
    /// once a vault.
    #[test]
    fn a_vault_is_copied_once_outside_it_before_its_first_sync() {
        let mut a = side("snapshot", "Desk", A);
        a.write("Plan.md", "# Plan\n");
        a.write("Work/Ideas.md", "ideas");
        a.write(".obsidian/app.json", "{}");
        a.write("photo.png", "png");
        assert!(!a.vault.snapshot_taken());
        // 2026-10-28 11:22:33 UTC.
        let to = a.vault.snapshot(1_793_186_553_000).unwrap().expect("taken");
        assert!(to.starts_with(&a.data) && !to.starts_with(&a.root), "outside the vault, in the app's data");
        assert_eq!(to.file_name().unwrap().to_string_lossy(), "2026-10-28 112233");
        assert_eq!(fs::read_to_string(to.join("Plan.md")).unwrap(), "# Plan\n");
        assert_eq!(fs::read_to_string(to.join("Work/Ideas.md")).unwrap(), "ideas");
        assert_eq!(fs::read_to_string(to.join(".obsidian/app.json")).unwrap(), "{}", "the whole vault");
        assert_eq!(fs::read_to_string(to.join("photo.png")).unwrap(), "png");
        assert!(a.vault.snapshot_taken());
        assert_eq!(a.vault.snapshot(1_793_190_000_000).unwrap(), None, "once a vault");
        // Kept across a restart.
        assert!(Vault::open(&a.root, &a.data, true).snapshot_taken());
        // Nothing kept, nothing taken (the extensions).
        let mut none = Vault::open(&a.root, &a.data.join("elsewhere"), false);
        assert!(none.snapshot_taken() && none.snapshot(0).unwrap().is_none());
    }

    /// An edit made to a note's old name, after the other device renamed
    /// it, lands on the new name: one note, both changes, nothing left at
    /// the old path.
    #[test]
    fn an_edit_to_a_renamed_note_follows_it_to_its_new_name() {
        let (mut a, mut b) = (side("rename", "Desk", A), side("rename", "iPhone", B));
        a.write("Work/Ideas.md", "ideas");
        a.vault.scan();
        send(&mut a, &mut b, NOW, None);
        b.vault.scan();
        // The desk renames it; meanwhile the phone edits it at its old name.
        fs::rename(a.root.join("Work/Ideas.md"), a.root.join("Ideas.md")).unwrap();
        a.vault.moved("Work/Ideas.md", "Ideas.md", NOW + 10);
        b.write("Work/Ideas.md", "ideas, and one more");
        b.vault.changed("Work/Ideas.md");
        send(&mut b, &mut a, NOW + 20, None);
        assert_eq!(a.read("Ideas.md").as_deref(), Some("ideas, and one more"), "the edit follows the rename");
        assert!(a.read("Work/Ideas.md").is_none(), "nothing comes back at the old name");
        send(&mut a, &mut b, NOW + 30, None);
        assert_eq!(b.read("Ideas.md").as_deref(), Some("ideas, and one more"));
        assert!(b.read("Work/Ideas.md").is_none(), "and the phone takes the rename");
    }

    /// Two notes whose names differ only in case, where the file system
    /// can't hold both (Windows, the Mac): the incoming one is kept beside
    /// as "Name (2)", and neither is lost.
    #[test]
    fn a_note_whose_name_differs_only_in_case_is_kept_beside() {
        let (mut a, mut b) = (side("case", "Desk", A), side("case", "iPhone", B));
        a.write("Plan.md", "the desk's plan");
        b.write("plan.md", "the phone's plan");
        a.vault.scan();
        b.vault.scan();
        send(&mut b, &mut a, NOW, None);
        if a.root.join("PLAN.md").exists() {
            // A file system that folds case: both kept, side by side.
            assert_eq!(a.read("Plan.md").as_deref(), Some("the desk's plan"));
            assert_eq!(a.read("plan (2).md").as_deref().or(a.read("Plan (2).md").as_deref()), Some("the phone's plan"));
        } else {
            assert_eq!(a.read("plan.md").as_deref(), Some("the phone's plan"));
        }
    }

    /// The first sync between two vaults that both hold notes: what one
    /// lacks is copied; the same note with different text is a conflict,
    /// the newer edit winning on both and the other kept in its past;
    /// nothing is deleted, though each side lacks the other's notes.
    #[test]
    fn the_first_sync_copies_what_is_missing_and_deletes_nothing() {
        let (mut a, mut b) = (side("first", "Desk", A), side("first", "iPhone", B));
        a.write("Desk only.md", "a");
        b.write("Phone only.md", "b");
        b.write("Both.md", "the phone's");
        std::thread::sleep(std::time::Duration::from_millis(30));
        a.write("Both.md", "the desk's");
        a.vault.scan();
        b.vault.scan();
        send(&mut a, &mut b, NOW, None);
        send(&mut b, &mut a, NOW, None);
        for s in [&a, &b] {
            assert_eq!(s.read("Desk only.md").as_deref(), Some("a"), "{}: copied, nothing deleted", s.name);
            assert_eq!(s.read("Phone only.md").as_deref(), Some("b"), "{}: copied, nothing deleted", s.name);
        }
        // The desk's text is the newer: it wins on the phone; the phone's is
        // in the phone's past.
        assert_eq!(b.read("Both.md").as_deref(), Some("the desk's"));
        assert!(b.past("Both.md").iter().any(|v| v.0 == "the phone's"), "the phone's own text kept");
        send(&mut b, &mut a, NOW + 10, None);
        assert_eq!(a.read("Both.md").as_deref(), Some("the desk's"));
    }

    /// A note over 256 KB doesn't go, and is counted for Settings > Devices;
    /// cut below the line, it goes.
    #[test]
    fn a_note_over_256_kb_is_counted_not_sent() {
        let (mut a, mut b) = (side("big", "Desk", A), side("big", "iPhone", B));
        a.write("Big.md", &"x".repeat(journal::FILE_MOST as usize + 1));
        a.write("Small.md", "s");
        a.vault.scan();
        assert_eq!(a.vault.too_big(), 1);
        send(&mut a, &mut b, NOW, None);
        assert!(b.read("Big.md").is_none() && b.read("Small.md").is_some());
        a.write("Big.md", "cut down");
        a.vault.changed("Big.md");
        assert_eq!(a.vault.too_big(), 0);
        send(&mut a, &mut b, NOW + 10, None);
        assert_eq!(b.read("Big.md").as_deref(), Some("cut down"));
    }

    /// Both changed: the newer edit wins the file on both sides, and the
    /// other is in the note's past on both; the note being typed in keeps
    /// its text; an edit beats a delete.
    #[test]
    fn when_both_change_the_newer_wins_and_nothing_is_lost() {
        let (mut a, mut b) = (side("conflict", "Desk", A), side("conflict", "iPhone", B));
        a.write("Plan.md", "v1");
        a.vault.scan();
        send(&mut a, &mut b, NOW, None);
        b.vault.scan();
        // Both edit; the desk's edit is the later one.
        b.write("Plan.md", "v1 phone");
        std::thread::sleep(std::time::Duration::from_millis(30));
        a.write("Plan.md", "v1 desk");
        a.vault.changed("Plan.md");
        b.vault.changed("Plan.md");
        send(&mut b, &mut a, NOW + 10, None);
        assert_eq!(a.read("Plan.md").as_deref(), Some("v1 desk"), "the desk's is newer: it stays");
        assert!(a.past("Plan.md").iter().any(|v| v.0 == "v1 phone" && v.1.as_deref() == Some("iPhone")));
        send(&mut a, &mut b, NOW + 20, None);
        assert_eq!(b.read("Plan.md").as_deref(), Some("v1 desk"), "and wins there too");
        assert!(b.past("Plan.md").iter().any(|v| v.0 == "v1 phone"), "the phone's own text is in its past");
        // Typing on the phone: its text stays whatever comes.
        a.vault.scan();
        b.vault.scan();
        a.write("Plan.md", "desk again");
        a.vault.changed("Plan.md");
        send(&mut a, &mut b, NOW + 30, Some("Plan.md"));
        assert_eq!(b.read("Plan.md").as_deref(), Some("v1 desk"));
        assert!(b.past("Plan.md").iter().any(|v| v.0 == "desk again"));
        assert!(b.vault.has_pending(), "the phone's text goes back out");
        // An edit beats a delete.
        let (mut c, mut d) = (side("delete", "Desk", A), side("delete", "iPhone", B));
        c.write("Note.md", "x");
        c.vault.scan();
        send(&mut c, &mut d, NOW, None);
        d.vault.scan();
        fs::remove_file(c.root.join("Note.md")).unwrap();
        c.vault.changed("Note.md");
        d.write("Note.md", "x, edited");
        d.vault.changed("Note.md");
        send(&mut c, &mut d, NOW + 10, None);
        assert_eq!(d.read("Note.md").as_deref(), Some("x, edited"), "kept");
        send(&mut d, &mut c, NOW + 20, None);
        assert_eq!(c.read("Note.md").as_deref(), Some("x, edited"), "and comes back");
    }

    /// A batch from a paired device is still checked: no path outside the
    /// vault or into a hidden folder is ever written, and a put whose text
    /// does not match its hash is dropped.
    #[test]
    fn nothing_outside_the_vault_is_ever_written() {
        let mut b = side("paths", "iPhone", B);
        let put = |path: &str, text: &str| Record::Put { path: path.into(), text: text.into(), hash: journal::hash(text.as_bytes()), base: None, edited: NOW, lamport: 1, created: None };
        let records = vec![put("../escape.md", "x"), put("/abs.md", "x"), put(".obsidian/x.md", "x"), put("notes.txt", "x"), Record::Put { path: "Lie.md".into(), text: "x".into(), hash: [0; 32], base: None, edited: NOW, lamport: 1, created: None }, put("ok.md", "fine")];
        let (batches, _) = journal::batches(records);
        let applied: Vec<Applied> = batches.iter().flat_map(|x| b.vault.apply(x, A, "Desk", B, NOW, None)).collect();
        assert_eq!(applied, [Applied::Written("ok.md".into())]);
        assert!(!b.root.parent().unwrap().join("escape.md").exists());
        assert!(fs::read_dir(&b.root).unwrap().count() == 1);
    }
}
