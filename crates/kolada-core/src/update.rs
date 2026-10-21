//! Updating the app outside a store: what the feed says, whether it is
//! newer, when to look again, checking the download, and putting the new
//! program in place of the running one. The shell fetches (Windows:
//! `kolada-win/src/updates.rs`); everything here is plain files and bytes,
//! so every platform's updater can share it.
//!
//! The feed is a tiny JSON on the website:
//!
//! ```text
//! { "version": "0.2.0", "url": "https://kolada.app/download/Kolada-0.2.0.exe",
//!   "sha256": "<64 hex>", "size": 4960000 }
//! ```
//!
//! Integrity is HTTPS plus that SHA-256, both from the same site. Signing
//! the feed with an ed25519 key built into the app would be the next step.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The feed the app checks, at most once a day. Installs from before the
/// rename ask `before::FEED`, which the website keeps serving with a
/// download on that site (a feed's download must be on the site the feed
/// was asked from).
pub const FEED: &str = "https://kolada.app/download/feed.json";

/// The Mac's feed: the same JSON, its `url` a zip of the signed and
/// notarized `Kolada.app` (`ditto -c -k --keepParent`), on the same site.
pub const FEED_MAC: &str = "https://kolada.app/download/mac.json";

/// Seconds between checks.
pub const DAY: u64 = 24 * 60 * 60;

/// The most an update may weigh (the app is about 5 MB).
pub const MOST: usize = 64 * 1024 * 1024;

/// A version as `major.minor.patch` (more parts are kept and compared).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(Vec<u32>);

impl Version {
    /// `0.1.0`, `v0.1.0` or `0.1`; None for anything else.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = Vec::new();
        for part in text.split('.') {
            if part.is_empty() || part.len() > 9 || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            parts.push(part.parse().ok()?);
        }
        if parts.len() > 4 {
            return None;
        }
        // 0.1 and 0.1.0 are the same version.
        while parts.len() > 1 && parts.last() == Some(&0) {
            parts.pop();
        }
        Some(Version(parts))
    }

    /// The version of this build.
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("the workspace version")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = self.0.clone();
        parts.resize(parts.len().max(3), 0);
        let parts: Vec<String> = parts.iter().map(u32::to_string).collect();
        f.write_str(&parts.join("."))
    }
}

/// What the feed offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feed {
    pub version: Version,
    pub url: String,
    /// Lowercase hex.
    pub sha256: String,
    pub size: usize,
    /// The writing helper that goes with this version (`kolada-ai.exe`),
    /// when the feed lists it (`helper_url`, `helper_sha256`,
    /// `helper_size`). A copy that updates brings it along; one that has
    /// none fetches it. Feeds before 0.3.1 have none, and copies before it
    /// pass over these fields.
    pub helper: Option<Part>,
}

/// A file the feed offers: where, its hash (lowercase hex) and size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub url: String,
    pub sha256: String,
    pub size: usize,
}

impl Part {
    /// Whether `bytes` are what the feed promised.
    pub fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() == self.size && hex(&sha256(bytes)) == self.sha256
    }
}

/// A download the feed may name, if it is one the app takes: on the
/// feed's own site, over HTTPS (or a server on this machine under test),
/// with a hash and a size.
fn part(url: String, sha256: String, size: usize, from: &str, local: bool) -> Option<Part> {
    let sha256 = sha256.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if size == 0 || size > MOST {
        return None;
    }
    let origin = origin(from)?;
    if !url.starts_with(&origin) || url.len() == origin.len() {
        return None;
    }
    let secure = origin.starts_with("https://");
    let on_this_machine = ["http://localhost", "http://127.0.0.1"].iter().any(|h| origin.starts_with(h));
    if !secure && !(local && on_this_machine) {
        return None;
    }
    Some(Part { url, sha256, size })
}

/// What the updater does with a feed it was given.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Nothing: this copy is up to date and has what it needs.
    Nothing,
    /// A newer version: the program, and its helper when the feed has one.
    Update,
    /// This version, but its helper is missing (it came by an update that
    /// knew only the program): the helper alone.
    Helper,
}

impl Feed {
    /// What to fetch for a copy at `current`, with or without its helper.
    pub fn plan(&self, current: &Version, helper_here: bool) -> Plan {
        if self.version > *current {
            Plan::Update
        } else if self.version == *current && !helper_here && self.helper.is_some() {
            Plan::Helper
        } else {
            Plan::Nothing
        }
    }
}

impl Feed {
    /// The feed's JSON, fetched from `from`. The download must be on the
    /// same site as the feed, over HTTPS (plain HTTP only when `local` is
    /// allowed, for tests against a server on this machine), and say its
    /// hash and size.
    pub fn parse(json: &str, from: &str, local: bool) -> Option<Feed> {
        let version = Version::parse(&string_field(json, "version")?)?;
        let size: usize = number_field(json, "size")?.try_into().ok()?;
        let program = part(string_field(json, "url")?, string_field(json, "sha256")?, size, from, local)?;
        // A helper the app would not take is left out; the update itself
        // still goes.
        let helper = (|| {
            let size: usize = number_field(json, "helper_size")?.try_into().ok()?;
            part(string_field(json, "helper_url")?, string_field(json, "helper_sha256")?, size, from, local)
        })();
        Some(Feed { version, url: program.url, sha256: program.sha256, size: program.size, helper })
    }

    /// Whether `bytes` are what the feed promised.
    pub fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() == self.size && hex(&sha256(bytes)) == self.sha256
    }
}

/// `https://host[:port]/` of a URL.
fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "https" | "http") {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some(format!("{scheme}://{host}/"))
}

/// A string value of the feed's (flat, small) JSON. Escapes are refused:
/// none of its values need them.
fn string_field(json: &str, key: &str) -> Option<String> {
    let rest = value_of(json, key)?.strip_prefix('"')?;
    let end = rest.find('"')?;
    let value = &rest[..end];
    (!value.contains('\\')).then(|| value.to_string())
}

fn number_field(json: &str, key: &str) -> Option<u64> {
    let rest = value_of(json, key)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 12 {
        return None;
    }
    digits.parse().ok()
}

/// What follows `"key":` at the top of the object.
fn value_of<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let quoted = format!("\"{key}\"");
    let mut from = 0;
    while let Some(at) = json[from..].find(&quoted) {
        let after = json[from + at + quoted.len()..].trim_start();
        if let Some(value) = after.strip_prefix(':') {
            return Some(value.trim_start());
        }
        from += at + quoted.len();
    }
    None
}

/// Whether a check is due: never checked, a day since the last, or the
/// clock went back past it.
pub fn due(last: Option<u64>, now: u64) -> bool {
    match last {
        None => true,
        Some(last) => now < last || now - last >= DAY,
    }
}

/// When the last check was, as kept in `dir` (see `staging`).
pub fn last_check(dir: &Path) -> Option<u64> {
    fs::read_to_string(dir.join("checked")).ok()?.trim().parse().ok()
}

pub fn checked(dir: &Path, now: u64) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join("checked"), now.to_string())
}

/// Where a downloaded update waits: `update` in the app's data folder.
pub fn staging(app_data: &Path) -> PathBuf {
    app_data.join("update")
}

/// The program waiting in `dir`, with the name it takes when in place.
fn staged_exe(dir: &Path) -> PathBuf {
    dir.join("next.exe")
}

/// The writing helper waiting in `dir` beside the program.
fn staged_helper(dir: &Path) -> PathBuf {
    dir.join("next-ai.exe")
}

/// Keeps a verified download in `dir` until it goes in: the program (and
/// its helper, when the feed had one) first, then the note of its version
/// that says it is ready.
pub fn stage(dir: &Path, version: &Version, bytes: &[u8], helper: Option<&[u8]>) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let _ = fs::remove_file(dir.join("ready"));
    let _ = fs::remove_file(staged_helper(dir));
    let part = dir.join("next.part");
    if let Some(helper) = helper {
        fs::write(&part, helper)?;
        fs::rename(&part, staged_helper(dir))?;
    }
    fs::write(&part, bytes)?;
    fs::rename(&part, staged_exe(dir))?;
    fs::write(dir.join("ready"), version.to_string())
}

/// The helper staged with the update in `dir`, if it came with one.
pub fn staged_helper_file(dir: &Path) -> Option<PathBuf> {
    let helper = staged_helper(dir);
    helper.is_file().then_some(helper)
}

/// Writes a verified file to `to` whole or not at all (beside it first,
/// then renamed into place): the helper fetched on its own.
pub fn put(to: &Path, bytes: &[u8]) -> io::Result<()> {
    let part = to.with_extension("part");
    fs::write(&part, bytes)?;
    fs::rename(&part, to).inspect_err(|_| {
        let _ = fs::remove_file(&part);
    })
}

/// The update ready in `dir`, if there is one and it is newer than
/// `current`. One waiting that is not newer (the app was updated some other
/// way) is cleared.
pub fn staged(dir: &Path, current: &Version) -> Option<(Version, PathBuf)> {
    let version = Version::parse(&fs::read_to_string(dir.join("ready")).ok()?)?;
    let exe = staged_exe(dir);
    if version <= *current || !exe.is_file() {
        clear(dir);
        return None;
    }
    Some((version, exe))
}

/// Forgets the update waiting in `dir`.
pub fn clear(dir: &Path) {
    for name in ["ready", "next.exe", "next-ai.exe", "next.part"] {
        let _ = fs::remove_file(dir.join(name));
    }
}

/// Where the program that was replaced goes: beside it, deleted by the next
/// start once it has stopped running.
pub fn old_exe(exe: &Path) -> PathBuf {
    let stem = exe.file_stem().map_or_else(|| "kolada".into(), |s| s.to_string_lossy().into_owned());
    exe.with_file_name(format!("{stem}.old.exe"))
}

/// Puts `new` in place of the program at `exe`, which may be running (on
/// Windows a running program can be renamed, not written over): `exe` goes
/// to `old_exe`, `new` to `exe`. When the second step fails the first is
/// undone. `new` may be on another drive: then it is copied.
pub fn swap(exe: &Path, new: &Path) -> io::Result<()> {
    let old = old_exe(exe);
    if old.exists() {
        // Still running (a note's own window): this update waits.
        fs::remove_file(&old)?;
    }
    fs::rename(exe, &old)?;
    let moved = fs::rename(new, exe).or_else(|_| {
        let part = exe.with_extension("part");
        fs::copy(new, &part)?;
        fs::rename(&part, exe)?;
        let _ = fs::remove_file(new);
        Ok(())
    });
    if let Err(e) = moved {
        let _ = fs::rename(&old, exe);
        return Err(e);
    }
    Ok(())
}

/// The Mac's app bundle waiting in `dir` (unpacked and its signature
/// checked by the shell before it is staged).
fn staged_bundle_dir(dir: &Path) -> PathBuf {
    dir.join("Kolada.app")
}

/// Keeps a verified app bundle in `dir` until it goes in: `app` (a folder
/// unpacked beside `dir`, on the same volume) is moved there, then the
/// note of its version that says it is ready.
pub fn stage_bundle(dir: &Path, version: &Version, app: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let _ = fs::remove_file(dir.join("ready"));
    let to = staged_bundle_dir(dir);
    if to.exists() {
        fs::remove_dir_all(&to)?;
    }
    fs::rename(app, &to)?;
    fs::write(dir.join("ready"), version.to_string())
}

/// The app bundle ready in `dir`, if there is one and it is newer than
/// `current`; one waiting that is not newer is cleared.
pub fn staged_bundle(dir: &Path, current: &Version) -> Option<(Version, PathBuf)> {
    let version = Version::parse(&fs::read_to_string(dir.join("ready")).ok()?)?;
    let app = staged_bundle_dir(dir);
    if version <= *current || !app.join("Contents").is_dir() {
        clear_bundle(dir);
        return None;
    }
    Some((version, app))
}

/// Forgets the app bundle waiting in `dir`.
pub fn clear_bundle(dir: &Path) {
    let _ = fs::remove_file(dir.join("ready"));
    let _ = fs::remove_dir_all(staged_bundle_dir(dir));
}

/// Where the app bundle that was replaced goes until the new one has
/// started: beside it, hidden (`.Kolada.old.app`), so the Finder and
/// Launchpad never show two.
pub fn old_bundle(app: &Path) -> PathBuf {
    let name = app.file_stem().map_or_else(|| "Kolada".into(), |s| s.to_string_lossy().into_owned());
    app.with_file_name(format!(".{name}.old.app"))
}

/// Puts the bundle `new` in place of the running one at `app` (macOS lets
/// a running app's folder be renamed: it keeps running from its files):
/// `app` goes to `old_bundle`, `new` to `app`. When the second step fails
/// the first is undone. Both must be on the same volume (a rename).
pub fn swap_bundle(app: &Path, new: &Path) -> io::Result<()> {
    if !new.join("Contents").is_dir() {
        return Err(io::ErrorKind::NotFound.into());
    }
    let old = old_bundle(app);
    if old.exists() {
        fs::remove_dir_all(&old)?;
    }
    fs::rename(app, &old)?;
    if let Err(e) = fs::rename(new, app) {
        let _ = fs::rename(&old, app);
        return Err(e);
    }
    Ok(())
}

/// Where a dev build runs from (under a cargo `target` folder): it never
/// updates itself.
pub fn is_dev_build(exe: &Path) -> bool {
    exe.ancestors().any(|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("target")))
}

/// Where Windows' Settings > Apps finds the website's install (per user,
/// under HKCU), shared by the installer and the app. A test instance's
/// install keeps apart, as "Kolada (test)".
pub fn uninstall_key() -> &'static str {
    if crate::testing() {
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Kolada.Test"
    } else {
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Kolada"
    }
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 (FIPS 180-4) of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finish()
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
    0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
    0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
    0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
    0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 fed in pieces, for a file too big to hold at once (a model
/// download is hashed as it arrives).
#[derive(Clone)]
pub struct Sha256 {
    h: [u32; 8],
    /// A block not yet full.
    buf: [u8; 64],
    used: usize,
    /// Bytes seen so far.
    len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Sha256 {
        Sha256 { h: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19], buf: [0; 64], used: 0, len: 0 }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.used > 0 {
            let take = (64 - self.used).min(data.len());
            self.buf[self.used..self.used + take].copy_from_slice(&data[..take]);
            self.used += take;
            data = &data[take..];
            if self.used < 64 {
                return;
            }
            let buf = self.buf;
            self.block(&buf);
            self.used = 0;
        }
        let full = data.len() / 64 * 64;
        for chunk in data[..full].chunks_exact(64) {
            self.block(chunk);
        }
        let rest = &data[full..];
        self.buf[..rest.len()].copy_from_slice(rest);
        self.used = rest.len();
    }

    pub fn finish(mut self) -> [u8; 32] {
        // The rest, a 1 bit, zeros, and the length in bits.
        let bits = self.len.wrapping_mul(8);
        let mut tail = [0u8; 128];
        tail[..self.used].copy_from_slice(&self.buf[..self.used]);
        tail[self.used] = 0x80;
        let n = if self.used < 56 { 64 } else { 128 };
        tail[n - 8..n].copy_from_slice(&bits.to_be_bytes());
        for chunk in tail[..n].chunks_exact(64) {
            self.block(chunk);
        }
        let mut out = [0u8; 32];
        for (i, v) in self.h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }

    fn block(&mut self, chunk: &[u8]) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = self.h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, v) in self.h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(v("0.2.0") > v("0.1.0"));
        assert!(v("0.10.0") > v("0.9.9"), "ten is more than nine");
        assert!(v("1.0.0") > v("0.99.99"));
        assert_eq!(v("0.1"), v("0.1.0"));
        assert_eq!(v("v0.1.0"), v("0.1.0"));
        assert!(v("0.1.0.1") > v("0.1.0"));
        assert_eq!(v("0.1").to_string(), "0.1.0");
        for bad in ["", "0..1", "0.1.x", "-1.0", "0.1.0-beta", "1.2.3.4.5", "1234567890.0"] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn feed_json(url: &str) -> String {
        format!("{{\n  \"version\": \"0.2.0\",\n  \"url\": \"{url}\",\n  \"sha256\": \"{}\",\n  \"size\": 3\n}}\n", HASH.to_uppercase())
    }

    #[test]
    fn the_feed_says_what_to_fetch_from_its_own_site() {
        let from = "https://kolada.app/download/latest.json";
        let feed = Feed::parse(&feed_json("https://kolada.app/download/Kolada-0.2.0.exe"), from, false).expect("a feed");
        assert_eq!(feed, Feed { version: v("0.2.0"), url: "https://kolada.app/download/Kolada-0.2.0.exe".into(), sha256: HASH.into(), size: 3, helper: None });
        assert!(feed.matches(b"abc"));
        assert!(!feed.matches(b"abd"), "another file of the same size");
        assert!(!feed.matches(b"abcd"));
        // Somewhere else, or not over HTTPS: refused.
        for url in ["https://kolada.app.evil.example/x.exe", "https://evil.example/Kolada.exe", "http://kolada.app/download/x.exe", "https://kolada.app/"] {
            assert_eq!(Feed::parse(&feed_json(url), from, false), None, "{url}");
        }
        assert_eq!(Feed::parse(&feed_json("https://user@kolada.app/x.exe"), "https://user@kolada.app/latest.json", false), None);
        // A server on this machine, for tests only.
        let local = "http://127.0.0.1:8765/latest.json";
        assert!(Feed::parse(&feed_json("http://127.0.0.1:8765/Kolada.exe"), local, true).is_some());
        assert_eq!(Feed::parse(&feed_json("http://127.0.0.1:8765/Kolada.exe"), local, false), None);
        // Damaged or missing fields.
        assert_eq!(Feed::parse("{}", from, false), None);
        assert_eq!(Feed::parse(&feed_json("https://kolada.app/x.exe").replace(HASH.to_uppercase().as_str(), "abc"), from, false), None);
        assert_eq!(Feed::parse(&feed_json("https://kolada.app/x.exe").replace("\"size\": 3", "\"size\": 0"), from, false), None);
        assert_eq!(Feed::parse(&feed_json("https://kolada.app/x.exe").replace("0.2.0", "soon"), from, false), None);
        assert_eq!(Feed::parse("<!DOCTYPE html><html>", from, false), None, "a page in its place");
    }

    #[test]
    fn the_feed_brings_the_writing_helper_along() {
        let from = "https://kolada.app/download/feed.json";
        let with = |url: &str| {
            feed_json("https://kolada.app/download/Kolada-0.3.1.exe").replace("0.2.0", "0.3.1").trim_end().trim_end_matches('}').to_string()
                + &format!(",\n  \"helper_url\": \"{url}\",\n  \"helper_sha256\": \"{}\",\n  \"helper_size\": 3\n}}\n", HASH.to_uppercase())
        };
        let feed = Feed::parse(&with("https://kolada.app/download/kolada-ai-0.3.1.exe"), from, false).expect("a feed");
        let helper = feed.helper.clone().expect("the helper");
        assert_eq!(helper, Part { url: "https://kolada.app/download/kolada-ai-0.3.1.exe".into(), sha256: HASH.into(), size: 3 });
        assert!(helper.matches(b"abc") && !helper.matches(b"abd"));
        // A helper from somewhere else is left out; the update still goes.
        let elsewhere = Feed::parse(&with("https://evil.example/kolada-ai.exe"), from, false).expect("the update still");
        assert_eq!(elsewhere.helper, None);
        // What to do: a newer version updates (with its helper); this
        // version without its helper fetches only that; else nothing.
        assert_eq!(feed.plan(&v("0.3.0"), true), Plan::Update);
        assert_eq!(feed.plan(&v("0.3.1"), false), Plan::Helper);
        assert_eq!(feed.plan(&v("0.3.1"), true), Plan::Nothing);
        assert_eq!(feed.plan(&v("0.4.0"), false), Plan::Nothing, "a newer copy takes no older helper");
        // A feed with no helper (before 0.3.1) never asks for one.
        let old = Feed::parse(&feed_json("https://kolada.app/download/Kolada-0.2.0.exe"), from, false).unwrap();
        assert_eq!(old.plan(&v("0.2.0"), false), Plan::Nothing);
    }

    #[test]
    fn an_update_stages_its_helper_and_clears_both() {
        let dir = std::env::temp_dir().join(format!("kolada-stage-helper-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        stage(&dir, &v("9.0.0"), b"program", Some(b"helper")).unwrap();
        let (version, exe) = staged(&dir, &v("1.0.0")).unwrap();
        assert_eq!((version, fs::read(&exe).unwrap()), (v("9.0.0"), b"program".to_vec()));
        assert_eq!(fs::read(staged_helper_file(&dir).unwrap()).unwrap(), b"helper");
        // Staged again without one: no helper from before is left.
        stage(&dir, &v("9.0.1"), b"program", None).unwrap();
        assert_eq!(staged_helper_file(&dir), None);
        stage(&dir, &v("9.0.2"), b"program", Some(b"helper")).unwrap();
        clear(&dir);
        assert!(staged_helper_file(&dir).is_none() && !dir.join("next.exe").exists());
        // The helper alone goes into place whole.
        let to = dir.join("kolada-ai.exe");
        put(&to, b"helper").unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"helper");
        assert!(!to.with_extension("part").exists());
        // Each program steps aside to its own old name.
        assert_eq!(old_exe(&dir.join("kolada.exe")), dir.join("kolada.old.exe"));
        assert_eq!(old_exe(&to), dir.join("kolada-ai.old.exe"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn kolada_and_the_old_site_each_offer_their_own_downloads() {
        let feed = Feed::parse(&feed_json("https://kolada.app/download/Kolada-0.2.0.exe"), FEED, false).expect("the app's feed");
        assert_eq!(feed.url, "https://kolada.app/download/Kolada-0.2.0.exe");
        // What installs from before the rename ask: the same download, from the old site.
        let bridge = crate::before::FEED;
        let old_site = format!("{}/Kolada-0.2.0.exe", bridge.rsplit_once('/').unwrap().0);
        assert!(Feed::parse(&feed_json(&old_site), bridge, false).is_some());
        assert_eq!(Feed::parse(&feed_json("https://kolada.app/download/Kolada-0.2.0.exe"), bridge, false), None, "only from the site asked");
        assert_eq!(Feed::parse(&feed_json(&old_site), FEED, false), None);
    }

    #[test]
    fn checks_at_most_once_a_day() {
        let t = 1_790_000_000;
        assert!(due(None, t), "never checked");
        assert!(!due(Some(t), t));
        assert!(!due(Some(t), t + DAY - 1));
        assert!(due(Some(t), t + DAY));
        assert!(due(Some(t), t - 60), "the clock went back");
        let dir = std::env::temp_dir().join(format!("kolada-update-checked-{}", std::process::id()));
        assert_eq!(last_check(&dir), None);
        checked(&dir, t).unwrap();
        assert_eq!(last_check(&dir), Some(t));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), HASH);
        assert_eq!(hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        assert_eq!(hex(&sha256(&vec![b'a'; 1_000_000])), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn sha256_in_pieces_equals_sha256_at_once() {
        // Odd-sized pieces cross every block boundary, and the lengths around
        // the padding's edge (55, 56, 63, 64 bytes) take one tail block or two.
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 31 + i / 7) as u8).collect();
        for len in [0, 1, 55, 56, 57, 63, 64, 65, 119, 120, 128, 1000, 5000] {
            for piece in [1, 3, 7, 13, 63, 64, 65, 129] {
                let mut h = Sha256::new();
                for p in data[..len].chunks(piece) {
                    h.update(p);
                }
                assert_eq!(h.finish(), sha256(&data[..len]), "{len} bytes in pieces of {piece}");
            }
        }
        let mut h = Sha256::new();
        let a = vec![b'a'; 1_000_000];
        for p in a.chunks(997) {
            h.update(p);
            h.update(&[]);
        }
        assert_eq!(hex(&h.finish()), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-update-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_staged_update_waits_until_it_is_newer_than_the_app() {
        let dir = scratch("staged").join("update");
        assert_eq!(staged(&dir, &v("0.1.0")), None);
        stage(&dir, &v("0.2.0"), b"new program", None).unwrap();
        let (version, exe) = staged(&dir, &v("0.1.0")).expect("ready");
        assert_eq!((version, fs::read(&exe).unwrap()), (v("0.2.0"), b"new program".to_vec()));
        // The app got 0.2.0 some other way: what waits is cleared.
        assert_eq!(staged(&dir, &v("0.2.0")), None);
        assert!(!exe.exists() && !dir.join("ready").exists());
        // A download cut short is never ready.
        stage(&dir, &v("0.3.0"), b"x", None).unwrap();
        fs::remove_file(dir.join("next.exe")).unwrap();
        assert_eq!(staged(&dir, &v("0.1.0")), None);
        fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_new_program_takes_the_old_ones_place() {
        let dir = scratch("swap");
        let exe = dir.join("kolada.exe");
        let new = dir.join("update").join("next.exe");
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        fs::write(&exe, b"old").unwrap();
        fs::write(&new, b"new").unwrap();
        swap(&exe, &new).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        assert_eq!(fs::read(old_exe(&exe)).unwrap(), b"old", "kept until the next start deletes it");
        assert!(!new.exists());
        // Again, over the last update's leftover.
        fs::write(&new, b"newer").unwrap();
        swap(&exe, &new).unwrap();
        assert_eq!((fs::read(&exe).unwrap(), fs::read(old_exe(&exe)).unwrap()), (b"newer".to_vec(), b"new".to_vec()));
        // Nothing to put in: the program stays where it was.
        assert!(swap(&exe, &dir.join("missing.exe")).is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"newer");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A bundle as the Mac's has it: Contents, Info.plist and the binary.
    fn bundle(at: &Path, binary: &[u8]) {
        fs::create_dir_all(at.join("Contents").join("MacOS")).unwrap();
        fs::write(at.join("Contents").join("Info.plist"), b"<plist/>").unwrap();
        fs::write(at.join("Contents").join("MacOS").join("kolada-mac"), binary).unwrap();
    }

    fn binary(app: &Path) -> Vec<u8> {
        fs::read(app.join("Contents").join("MacOS").join("kolada-mac")).unwrap()
    }

    #[test]
    fn a_mac_bundle_waits_staged_until_it_is_newer_than_the_app() {
        let root = scratch("bundle-staged");
        let dir = root.join("update");
        assert_eq!(staged_bundle(&dir, &v("0.3.1")), None);
        let unpacked = root.join("unpacked").join("Kolada.app");
        bundle(&unpacked, b"0.4");
        stage_bundle(&dir, &v("0.4.0"), &unpacked).unwrap();
        assert!(!unpacked.exists(), "moved, not copied");
        let (version, app) = staged_bundle(&dir, &v("0.3.1")).expect("ready");
        assert_eq!((version, binary(&app)), (v("0.4.0"), b"0.4".to_vec()));
        // Staged again over it: the newer one.
        bundle(&unpacked, b"0.5");
        stage_bundle(&dir, &v("0.5.0"), &unpacked).unwrap();
        assert_eq!(staged_bundle(&dir, &v("0.3.1")).map(|(v, a)| (v, binary(&a))), Some((v("0.5.0"), b"0.5".to_vec())));
        // The app got 0.5.0 some other way: what waits is cleared.
        assert_eq!(staged_bundle(&dir, &v("0.5.0")), None);
        assert!(!dir.join("Kolada.app").exists() && !dir.join("ready").exists());
        // A bundle with no Contents is never ready.
        fs::create_dir_all(dir.join("Kolada.app")).unwrap();
        fs::write(dir.join("ready"), "0.9.0").unwrap();
        assert_eq!(staged_bundle(&dir, &v("0.3.1")), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_new_bundle_takes_the_running_ones_place() {
        let root = scratch("bundle-swap");
        let app = root.join("Applications").join("Kolada.app");
        let new = root.join("update").join("Kolada.app");
        bundle(&app, b"old");
        bundle(&new, b"new");
        swap_bundle(&app, &new).unwrap();
        assert_eq!(binary(&app), b"new");
        assert_eq!(old_bundle(&app), root.join("Applications").join(".Kolada.old.app"));
        assert_eq!(binary(&old_bundle(&app)), b"old", "kept until the new one has started");
        assert!(!new.exists());
        // Again, over the last update's leftover.
        bundle(&new, b"newer");
        swap_bundle(&app, &new).unwrap();
        assert_eq!((binary(&app), binary(&old_bundle(&app))), (b"newer".to_vec(), b"new".to_vec()));
        // Nothing (or no bundle) to put in: the app stays where it was.
        assert!(swap_bundle(&app, &root.join("missing.app")).is_err());
        fs::create_dir_all(root.join("empty.app")).unwrap();
        assert!(swap_bundle(&app, &root.join("empty.app")).is_err());
        assert_eq!(binary(&app), b"newer");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dev_builds_never_update_themselves() {
        let path = |parts: &[&str]| parts.iter().collect::<PathBuf>();
        assert!(is_dev_build(&path(&["src", "kolada", "target", "release", "kolada.exe"])));
        assert!(is_dev_build(&path(&["src", "kolada", "TARGET", "test", "release", "kolada.exe"])));
        assert!(!is_dev_build(&path(&["Users", "a", "AppData", "Local", "Programs", "Kolada", "kolada.exe"])));
        assert!(!is_dev_build(&path(&["Users", "a", "Downloads", "kolada.exe"])));
    }
}
