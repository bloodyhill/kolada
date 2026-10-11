//! What each note is about, one line each, as the writing model summed it
//! up: kept in the app's data (`gists.bin`), never in the note or the vault,
//! and only for notes that were open. Read on a thread once the window
//! shows; each new gist is written on a thread too, merged into the file as
//! it is then. The same file and format as Windows (`notsy-win` `gists.rs`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::Sender;

const MAGIC: [u8; 4] = *b"NGST";
const VERSION: u32 = 1;
const HEADER: usize = 4 + 4 + 8;
/// Gists kept at most: the oldest written go first.
const KEEP: usize = 5000;

/// A note's gist and the words it had then.
#[derive(Clone, Debug, PartialEq)]
pub struct Gist {
    pub words: u32,
    pub text: String,
}

#[derive(Default)]
pub struct Gists {
    by_note: HashMap<u64, Gist>,
}

/// Where a note's gist is kept: its path, hashed (the same across runs).
pub fn key(path: &Path) -> u64 {
    fnv1a(path.to_string_lossy().to_lowercase().as_bytes())
}

fn file() -> Option<PathBuf> {
    Some(notsy_core::app_data_dir()?.join("gists.bin"))
}

impl Gists {
    pub fn get(&self, key: u64) -> Option<&Gist> {
        self.by_note.get(&key)
    }

    /// Keeps `gist` for the note at `key`, here and in the file.
    pub fn set(&mut self, key: u64, gist: Gist) {
        self.by_note.insert(key, gist.clone());
        std::thread::spawn(move || {
            if let Some(path) = file() {
                let _ = save_one(&path, key, gist);
            }
        });
    }

    /// Reads the file on a thread; the gists arrive in `to`, and `wake` is
    /// called.
    pub fn load(to: Sender<Gists>, wake: impl Fn() + Send + 'static) {
        std::thread::spawn(move || {
            let entries = file().and_then(|p| std::fs::read(p).ok()).and_then(|b| decode(&b)).unwrap_or_default();
            let gists = Gists { by_note: entries.into_iter().collect() };
            if to.send(gists).is_ok() {
                wake();
            }
        });
    }
}

/// One of this process's workers writes the file at a time. Another
/// process writing at once may lose its entry: it is a cache.
static WRITING: Mutex<()> = Mutex::new(());

/// Puts `gist` in the file at `path` as it is now, last (newest), and
/// replaces the file whole.
fn save_one(path: &Path, key: u64, gist: Gist) -> std::io::Result<()> {
    let _held = WRITING.lock();
    let mut entries = std::fs::read(path).ok().and_then(|b| decode(&b)).unwrap_or_default();
    entries.retain(|(k, _)| *k != key);
    entries.push((key, gist));
    if entries.len() > KEEP {
        entries.drain(..entries.len() - KEEP);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, encode(&entries))?;
    std::fs::rename(&tmp, path)
}

/// Oldest first.
fn encode(entries: &[(u64, Gist)]) -> Vec<u8> {
    let mut p = Vec::with_capacity(entries.len() * 96);
    p.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (key, g) in entries {
        p.extend_from_slice(&key.to_le_bytes());
        p.extend_from_slice(&g.words.to_le_bytes());
        p.extend_from_slice(&(g.text.len() as u32).to_le_bytes());
        p.extend_from_slice(g.text.as_bytes());
    }
    let mut out = Vec::with_capacity(HEADER + p.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&fnv1a(&p).to_le_bytes());
    out.extend_from_slice(&p);
    out
}

/// None for a damaged file or another version: it is a cache.
fn decode(bytes: &[u8]) -> Option<Vec<(u64, Gist)>> {
    if bytes.len() < HEADER || bytes[..4] != MAGIC || bytes[4..8] != VERSION.to_le_bytes() {
        return None;
    }
    let payload = &bytes[HEADER..];
    if fnv1a(payload).to_le_bytes() != bytes[8..16] {
        return None;
    }
    let mut at: usize = 0;
    let mut take = |n: usize| -> Option<&[u8]> {
        let head = payload.get(at..at.checked_add(n)?)?;
        at += n;
        Some(head)
    };
    let u32_of = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap_or_default());
    let n = u32_of(take(4)?) as usize;
    let mut out = Vec::with_capacity(n.min(KEEP));
    for _ in 0..n {
        let key = u64::from_le_bytes(take(8)?.try_into().ok()?);
        let words = u32_of(take(4)?);
        let len = u32_of(take(4)?) as usize;
        let text = String::from_utf8(take(len)?.to_vec()).ok()?;
        out.push((key, Gist { words, text }));
    }
    (take(1).is_none()).then_some(out)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gist(words: u32, text: &str) -> Gist {
        Gist { words, text: text.into() }
    }

    #[test]
    fn round_trips_and_refuses_damage() {
        let entries = vec![(1, gist(180, "Plans for the launch in May")), (2, gist(400, "خطة الإطلاق في مايو"))];
        let bytes = encode(&entries);
        assert_eq!(decode(&bytes), Some(entries));
        for i in 0..bytes.len() {
            let mut bad = bytes.clone();
            bad[i] ^= 0x20;
            assert_eq!(decode(&bad), None, "flipped byte {i} was accepted");
        }
        assert_eq!(decode(&bytes[..bytes.len() - 1]), None);
        assert_eq!(decode(&encode(&[])), Some(vec![]));
    }

    #[test]
    fn keeps_the_newest_and_one_per_note() {
        let dir = std::env::temp_dir().join(format!("notsy-gists-test-{}", std::process::id()));
        let path = dir.join("gists.bin");
        save_one(&path, 7, gist(200, "first")).unwrap();
        save_one(&path, 8, gist(300, "second")).unwrap();
        save_one(&path, 7, gist(260, "first, again")).unwrap();
        let back = decode(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(back, vec![(8, gist(300, "second")), (7, gist(260, "first, again"))]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(key(Path::new("/Notes/Plan.md")), key(Path::new("/notes/plan.md")));
    }
}
