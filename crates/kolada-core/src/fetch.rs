//! Downloading one big file of known size and SHA-256 (the writing model):
//! into a `.part` file that resumes where it stopped, hashed as it arrives,
//! checked whole, then renamed into place. The OS's HTTP is behind
//! `Source`, so every platform shares the rest; the shell runs `download`
//! off the UI thread.

use crate::update::{hex, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// What to fetch: one file of known size and SHA-256 (hex, any case).
pub struct Want<'a> {
    pub url: &'a str,
    pub file: &'a str,
    pub bytes: u64,
    pub sha256: &'a str,
}

#[derive(Debug, PartialEq)]
pub enum Failure {
    Network,
    BadHash,
    Disk,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Done(PathBuf),
    Cancelled,
    Failed(Failure),
}

/// The OS's HTTP: GET `url` asking for bytes from `from` on (a `Range: bytes=from-` header when from > 0);
/// calls `body` with each piece as it arrives (body returns false to stop early). Returns the status
/// (200 or 206) or None when the network failed. Never buffers the whole file.
pub trait Source {
    fn get_from(&self, url: &str, from: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16>;
}

/// Bytes written to the disk at a time.
const BUFFER: usize = 1 << 20;

/// Downloads `want` into `dir`: into `<file>.part`, resuming from what is there (206; a 200 starts over),
/// reporting `progress(done, total)`, stopping when `cancel` is set (the part file is kept for next time),
/// then checks the size and SHA-256 of the whole file and renames it to `<file>`. A wrong hash deletes the
/// part file: Failed(BadHash). Already in place with the right size: Done at once, nothing fetched.
pub fn download(dir: &Path, want: &Want, source: &dyn Source, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Outcome {
    let named = !want.file.is_empty() && !want.file.contains(['/', '\\', ':']) && want.file != "." && want.file != "..";
    if !named {
        return Outcome::Failed(Failure::Disk);
    }
    if want.sha256.len() != 64 || !want.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Outcome::Failed(Failure::BadHash);
    }
    let done = dir.join(want.file);
    if present(dir, want.file, want.bytes) {
        progress(want.bytes, want.bytes);
        return Outcome::Done(done);
    }
    if fs::create_dir_all(dir).is_err() {
        return Outcome::Failed(Failure::Disk);
    }
    let part = part_of(dir, want.file);
    match fetch(&part, want, source, progress, cancel) {
        Ok(Step::Whole(file)) => {
            drop(file);
            // Replaces a damaged file of the same name, if there is one.
            match fs::rename(&part, &done) {
                Ok(()) => Outcome::Done(done),
                Err(_) => Outcome::Failed(Failure::Disk),
            }
        }
        Ok(Step::Cancelled) => Outcome::Cancelled,
        Ok(Step::Network) => Outcome::Failed(Failure::Network),
        Ok(Step::BadHash) => {
            let _ = fs::remove_file(&part);
            Outcome::Failed(Failure::BadHash)
        }
        Err(_) => Outcome::Failed(Failure::Disk),
    }
}

enum Step {
    /// The part file is the whole file, checked and synced.
    Whole(File),
    Cancelled,
    Network,
    BadHash,
}

fn fetch(part: &Path, want: &Want, source: &dyn Source, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> io::Result<Step> {
    let total = want.bytes;
    let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(part)?;
    let have = file.metadata()?.len();
    if have > total {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the part is longer than the file"));
    }
    // What is there already is hashed now, so the check at the end reads
    // nothing a second time.
    let mut whole = Sha256::new();
    let mut buf = vec![0u8; BUFFER];
    let mut left = have;
    while left > 0 {
        let n = file.read(&mut buf[..BUFFER.min(left as usize)])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        whole.update(&buf[..n]);
        left -= n as u64;
    }
    drop(buf);
    progress(have, total);
    if have == total {
        return finish(file, whole, want);
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(Step::Cancelled);
    }

    file.seek(SeekFrom::Start(have))?;
    let mut out = BufWriter::with_capacity(BUFFER, file);
    // A server that ignores the range answers 200 with the file from its
    // start, which only the status says, at the end: so the new bytes are
    // also hashed on their own, in case they turn out to be the file.
    let mut fresh = if have > 0 { Some(Sha256::new()) } else { None };
    let mut got = 0u64;
    let mut too_long = false;
    let mut failed: Option<io::Error> = None;
    let status = source.get_from(want.url, have, &mut |piece: &[u8]| {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        if got + piece.len() as u64 > total {
            too_long = true;
            return false;
        }
        if let Err(e) = out.write_all(piece) {
            failed = Some(e);
            return false;
        }
        whole.update(piece);
        if let Some(h) = fresh.as_mut() {
            h.update(piece);
        }
        got += piece.len() as u64;
        progress((have + got).min(total), total);
        true
    });
    if let Some(e) = failed {
        return Err(e);
    }
    let mut file = out.into_inner().map_err(|e| e.into_error())?;

    let (len, hash) = match status {
        Some(206) => (have + got, whole),
        Some(200) if have > 0 => {
            // The file again from its start: it takes the part's place.
            shift_down(&mut file, have, got)?;
            (got, fresh.take().unwrap_or_default())
        }
        Some(200) => (got, whole),
        // Lost on the way: what came is kept for the next try (the hash
        // at the end catches a server that sent something else).
        None => (have + got, whole),
        Some(_) => {
            // An error page, not the file.
            file.set_len(have)?;
            file.sync_all()?;
            return Ok(Step::Network);
        }
    };
    file.set_len(len)?;
    file.sync_all()?;
    if too_long || len > total {
        return Ok(Step::BadHash);
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(Step::Cancelled);
    }
    if status.is_none() || len < total {
        return Ok(Step::Network);
    }
    finish(file, hash, want)
}

/// The part is the whole file: right if its hash is.
fn finish(file: File, hash: Sha256, want: &Want) -> io::Result<Step> {
    if !hex(&hash.finish()).eq_ignore_ascii_case(want.sha256) {
        return Ok(Step::BadHash);
    }
    file.sync_all()?;
    Ok(Step::Whole(file))
}

/// Moves the `len` bytes at `from` to the start of `file`.
fn shift_down(file: &mut File, from: u64, len: u64) -> io::Result<()> {
    let mut buf = vec![0u8; BUFFER];
    let mut at = 0u64;
    while at < len {
        let n = BUFFER.min((len - at) as usize);
        file.seek(SeekFrom::Start(from + at))?;
        file.read_exact(&mut buf[..n])?;
        file.seek(SeekFrom::Start(at))?;
        file.write_all(&buf[..n])?;
        at += n as u64;
    }
    Ok(())
}

fn part_of(dir: &Path, file: &str) -> PathBuf {
    dir.join(format!("{file}.part"))
}

/// The model file is in place (exists with exactly `bytes`).
pub fn present(dir: &Path, file: &str, bytes: u64) -> bool {
    fs::metadata(dir.join(file)).is_ok_and(|m| m.is_file() && m.len() == bytes)
}

/// Removes every file in `dir` named in `known` (and their .part files) except `keep`: switching sizes frees the space.
pub fn remove_others(dir: &Path, keep: Option<&str>, known: &[&str]) {
    for &name in known {
        if Some(name) == keep || name.is_empty() || name.contains(['/', '\\']) {
            continue;
        }
        let _ = fs::remove_file(dir.join(name));
        let _ = fs::remove_file(part_of(dir, name));
    }
}

/// The address for a model: the catalogue's (only https://huggingface.co/...), or under a test instance
/// (kolada_core::testing()) a base from KOLADA_MODEL_BASE + file, so tests can use a local server.
pub fn address(catalogue: Option<String>, file: &str) -> Option<String> {
    let testing = crate::testing();
    let base = if testing { std::env::var("KOLADA_MODEL_BASE").ok() } else { None };
    address_in(catalogue, file, testing, base)
}

fn address_in(catalogue: Option<String>, file: &str, testing: bool, base: Option<String>) -> Option<String> {
    if testing {
        if let Some(base) = base.filter(|b| local(b)) {
            return Some(format!("{}/{file}", base.trim_end_matches('/')));
        }
    }
    catalogue.filter(|url| url.starts_with("https://huggingface.co/") && !url.contains(char::is_whitespace))
}

/// A server on this machine, for a test instance only.
fn local(base: &str) -> bool {
    ["http://127.0.0.1:", "http://localhost:", "http://[::1]:", "http://127.0.0.1/", "http://localhost/", "http://[::1]/"]
        .iter()
        .any(|p| base.starts_with(p))
        || matches!(base, "http://127.0.0.1" | "http://localhost" | "http://[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::sha256;
    use std::cell::{Cell, RefCell};

    /// A server over an in-memory file, sending it in odd-sized pieces.
    struct Fake<'a> {
        body: Vec<u8>,
        /// Answers a range with 206; when false, always the whole file, 200.
        ranges: bool,
        /// Bytes sent before the connection drops (None).
        drop_after: Option<usize>,
        /// Sets this after so many bytes, as a person pressing Cancel.
        cancel_after: Option<(usize, &'a AtomicBool)>,
        status: Option<u16>,
        asked: RefCell<Vec<u64>>,
        sent: Cell<usize>,
    }

    impl<'a> Fake<'a> {
        fn new(body: &[u8]) -> Fake<'a> {
            Fake { body: body.to_vec(), ranges: true, drop_after: None, cancel_after: None, status: None, asked: RefCell::default(), sent: Cell::new(0) }
        }
    }

    impl Source for Fake<'_> {
        fn get_from(&self, _url: &str, from: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16> {
            self.asked.borrow_mut().push(from);
            if let Some(s) = self.status {
                body(b"<html>Not here</html>");
                return Some(s);
            }
            let start = if self.ranges { from as usize } else { 0 };
            let mut this_call = 0;
            for piece in self.body[start..].chunks(4093) {
                let mut piece = piece;
                if let Some(n) = self.drop_after {
                    if this_call + piece.len() > n {
                        piece = &piece[..n - this_call];
                        body(piece);
                        self.sent.set(self.sent.get() + piece.len());
                        return None;
                    }
                }
                if let Some((n, flag)) = self.cancel_after {
                    if this_call >= n {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
                if !body(piece) {
                    return Some(if self.ranges && from > 0 { 206 } else { 200 });
                }
                this_call += piece.len();
                self.sent.set(self.sent.get() + piece.len());
            }
            Some(if self.ranges && from > 0 { 206 } else { 200 })
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-fetch-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn model() -> Vec<u8> {
        (0..100_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect()
    }

    const FILE: &str = "writer.gguf";

    fn get(dir: &Path, body: &[u8], hash: &str, source: &dyn Source, cancel: &AtomicBool) -> (Outcome, Vec<(u64, u64)>) {
        let want = Want { url: "https://huggingface.co/x/resolve/abc/writer.gguf", file: FILE, bytes: body.len() as u64, sha256: hash };
        let mut seen = Vec::new();
        let out = download(dir, &want, source, &mut |d, t| seen.push((d, t)), cancel);
        (out, seen)
    }

    fn hash_of(b: &[u8]) -> String {
        hex(&sha256(b)).to_uppercase()
    }

    #[test]
    fn a_partial_download_resumes() {
        let dir = scratch("resume");
        let body = model();
        let no = AtomicBool::new(false);
        let first = Fake { drop_after: Some(40_000), ..Fake::new(&body) };
        let (out, _) = get(&dir, &body, &hash_of(&body), &first, &no);
        assert_eq!(out, Outcome::Failed(Failure::Network));
        assert_eq!(fs::read(dir.join("writer.gguf.part")).unwrap(), &body[..40_000], "what came is kept");
        assert!(!dir.join(FILE).exists());

        let second = Fake::new(&body);
        let (out, seen) = get(&dir, &body, &hash_of(&body), &second, &no);
        assert_eq!(out, Outcome::Done(dir.join(FILE)));
        assert_eq!(*second.asked.borrow(), [40_000], "asks from the part's length");
        assert_eq!(second.sent.get(), body.len() - 40_000, "only the rest is fetched");
        assert_eq!(seen.first(), Some(&(40_000, body.len() as u64)));
        assert_eq!(seen.last(), Some(&(body.len() as u64, body.len() as u64)));
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "progress only goes forward");
        assert_eq!(fs::read(dir.join(FILE)).unwrap(), body);
        assert!(!dir.join("writer.gguf.part").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_wrong_hash_is_refused_and_the_part_deleted() {
        let dir = scratch("hash");
        let body = model();
        let no = AtomicBool::new(false);
        let mut other = body.clone();
        other[77_777] ^= 1;
        let (out, _) = get(&dir, &body, &hash_of(&other), &Fake::new(&body), &no);
        assert_eq!(out, Outcome::Failed(Failure::BadHash));
        assert!(!dir.join("writer.gguf.part").exists() && !dir.join(FILE).exists());
        // Lowercase is as good as uppercase.
        let (out, _) = get(&dir, &body, &hash_of(&body).to_lowercase(), &Fake::new(&body), &no);
        assert_eq!(out, Outcome::Done(dir.join(FILE)));
        // Not a hash at all: nothing is fetched.
        fs::remove_file(dir.join(FILE)).unwrap();
        let fake = Fake::new(&body);
        assert_eq!(get(&dir, &body, "abc", &fake, &no).0, Outcome::Failed(Failure::BadHash));
        assert!(fake.asked.borrow().is_empty());
        // A server that sends more than the file.
        let mut longer = body.clone();
        longer.extend_from_slice(b"more");
        let (out, _) = get(&dir, &body, &hash_of(&body), &Fake::new(&longer), &no);
        assert_eq!(out, Outcome::Failed(Failure::BadHash));
        assert!(!dir.join("writer.gguf.part").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_200_to_a_range_request_starts_over() {
        let dir = scratch("200");
        let body = model();
        let no = AtomicBool::new(false);
        // A part that does not even match: the whole file replaces it.
        fs::write(dir.join("writer.gguf.part"), vec![0xAB; 30_000]).unwrap();
        let fake = Fake { ranges: false, ..Fake::new(&body) };
        let (out, seen) = get(&dir, &body, &hash_of(&body), &fake, &no);
        assert_eq!(out, Outcome::Done(dir.join(FILE)));
        assert_eq!(*fake.asked.borrow(), [30_000]);
        assert_eq!(fs::read(dir.join(FILE)).unwrap(), body);
        assert!(seen.iter().all(|&(d, t)| d <= t));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_error_page_is_not_kept() {
        let dir = scratch("404");
        let body = model();
        let no = AtomicBool::new(false);
        fs::write(dir.join("writer.gguf.part"), &body[..10_000]).unwrap();
        let fake = Fake { status: Some(404), ..Fake::new(&body) };
        assert_eq!(get(&dir, &body, &hash_of(&body), &fake, &no).0, Outcome::Failed(Failure::Network));
        assert_eq!(fs::read(dir.join("writer.gguf.part")).unwrap(), &body[..10_000]);
        // A part longer than the file: refused before anything is asked.
        fs::write(dir.join("writer.gguf.part"), vec![0; body.len() + 1]).unwrap();
        let fake = Fake::new(&body);
        assert_eq!(get(&dir, &body, &hash_of(&body), &fake, &no).0, Outcome::Failed(Failure::Disk));
        assert!(fake.asked.borrow().is_empty());
        // A part that is already whole is checked and put in place, unasked.
        fs::write(dir.join("writer.gguf.part"), &body).unwrap();
        let fake = Fake::new(&body);
        assert_eq!(get(&dir, &body, &hash_of(&body), &fake, &no).0, Outcome::Done(dir.join(FILE)));
        assert!(fake.asked.borrow().is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancel_keeps_the_part() {
        let dir = scratch("cancel");
        let body = model();
        let cancel = AtomicBool::new(false);
        let fake = Fake { cancel_after: Some((30_000, &cancel)), ..Fake::new(&body) };
        let (out, _) = get(&dir, &body, &hash_of(&body), &fake, &cancel);
        assert_eq!(out, Outcome::Cancelled);
        let part = fs::read(dir.join("writer.gguf.part")).unwrap();
        assert!(part.len() >= 30_000 && part.len() < body.len(), "{}", part.len());
        assert_eq!(part, &body[..part.len()]);
        assert!(!dir.join(FILE).exists());
        // Cancelled before it starts: nothing is asked.
        let fake = Fake::new(&body);
        assert_eq!(get(&dir, &body, &hash_of(&body), &fake, &cancel).0, Outcome::Cancelled);
        assert!(fake.asked.borrow().is_empty());
        // Next time it carries on.
        cancel.store(false, Ordering::Relaxed);
        let fake = Fake::new(&body);
        assert_eq!(get(&dir, &body, &hash_of(&body), &fake, &cancel).0, Outcome::Done(dir.join(FILE)));
        assert_eq!(*fake.asked.borrow(), [part.len() as u64]);
        assert_eq!(fs::read(dir.join(FILE)).unwrap(), body);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_in_place_is_not_fetched_again() {
        let dir = scratch("present");
        let body = model();
        let no = AtomicBool::new(false);
        assert!(!present(&dir, FILE, body.len() as u64));
        fs::write(dir.join(FILE), &body).unwrap();
        assert!(present(&dir, FILE, body.len() as u64));
        let fake = Fake::new(&body);
        let (out, seen) = get(&dir, &body, &hash_of(&body), &fake, &no);
        assert_eq!(out, Outcome::Done(dir.join(FILE)));
        assert!(fake.asked.borrow().is_empty(), "nothing fetched");
        assert_eq!(seen, [(body.len() as u64, body.len() as u64)]);
        // The wrong size is not the file: it is fetched and replaced.
        fs::write(dir.join(FILE), &body[..5]).unwrap();
        assert!(!present(&dir, FILE, body.len() as u64));
        let fake = Fake::new(&body);
        assert_eq!(get(&dir, &body, &hash_of(&body), &fake, &no).0, Outcome::Done(dir.join(FILE)));
        assert_eq!(*fake.asked.borrow(), [0]);
        assert_eq!(fs::read(dir.join(FILE)).unwrap(), body);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remove_others_frees_the_old_size() {
        let dir = scratch("others");
        let known = ["light.gguf", "full.gguf"];
        for f in ["light.gguf", "light.gguf.part", "full.gguf", "full.gguf.part", "mine.gguf"] {
            fs::write(dir.join(f), b"x").unwrap();
        }
        remove_others(&dir, Some("full.gguf"), &known);
        let left = |f: &str| dir.join(f).exists();
        assert!(!left("light.gguf") && !left("light.gguf.part"), "the other size is gone");
        assert!(left("full.gguf") && left("full.gguf.part"), "the chosen one stays, part and all");
        assert!(left("mine.gguf"), "a file it does not know is the person's own");
        remove_others(&dir, None, &known);
        assert!(!left("full.gguf") && !left("full.gguf.part") && left("mine.gguf"));
        remove_others(&dir.join("missing"), None, &known);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_huggingface_addresses_unless_testing() {
        let hub = "https://huggingface.co/kolada/writer/resolve/0123abc/writer.gguf".to_string();
        assert_eq!(address_in(Some(hub.clone()), FILE, false, None), Some(hub.clone()));
        for bad in ["http://huggingface.co/x/writer.gguf", "https://huggingface.co.evil.example/x", "https://evil.example/huggingface.co/x", "https://huggingface.com/x", ""] {
            assert_eq!(address_in(Some(bad.into()), FILE, false, None), None, "{bad}");
            assert_eq!(address_in(Some(bad.into()), FILE, true, None), None, "{bad}");
        }
        assert_eq!(address_in(None, FILE, false, None), None, "not published yet");
        // A base for tests is used only by a test instance, and only on this machine.
        let base = Some("http://127.0.0.1:8765/models/".to_string());
        assert_eq!(address_in(None, FILE, true, base.clone()), Some("http://127.0.0.1:8765/models/writer.gguf".into()));
        assert_eq!(address_in(Some(hub.clone()), FILE, true, base.clone()), Some("http://127.0.0.1:8765/models/writer.gguf".into()));
        assert_eq!(address_in(Some(hub.clone()), FILE, false, base), Some(hub.clone()));
        assert_eq!(address_in(None, FILE, true, Some("https://evil.example/".into())), None);
        assert_eq!(address_in(None, FILE, true, Some("http://localhost.evil.example/".into())), None);
        assert_eq!(address_in(Some(hub.clone()), FILE, true, Some(String::new())), Some(hub));
    }
}
