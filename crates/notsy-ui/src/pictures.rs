//! Pictures in notes, shared by every shell: where an embed's file is (named
//! the way Obsidian does: by file name anywhere in the vault, or by a path
//! from the vault's top or the note's folder), its size from the file's first
//! bytes so the first frame keeps its place, and its pixels at the size
//! drawn, decoded on a worker the shell supplies the decoder for (Windows'
//! WIC, the Mac's ImageIO). A picture beside the note, at the vault's top or
//! with its attachments is sized at once; others are searched for on the
//! worker, which starts after the first frame.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::images::{Lookup, PictureSource, is_image};
use crate::pixels::Pixels;

/// How long a picture takes to fade in once decoded.
const FADE: Duration = Duration::from_millis(180);

/// What a shell's image library does for the worker.
pub trait Decoder {
    /// A picture's size in pixels.
    fn size(&self, path: &Path) -> Option<(u32, u32)>;
    /// The picture scaled to `width` pixels across (keeping its shape).
    fn decode(&self, path: &Path, width: u32) -> Option<Pixels>;
}

/// Work for the decoding thread.
enum Job {
    /// Find the file an embed names, and its size.
    Find { vault: PathBuf, from: PathBuf, name: String },
    /// Decode a file at a width in pixels.
    Decode { path: PathBuf, width: u32 },
}

/// What comes back from the worker.
enum Found {
    /// An embed's file (None: there is none) and its size in pixels.
    Place { from: PathBuf, name: String, file: Option<(PathBuf, u32, u32)> },
    Pixels { path: PathBuf, width: u32, pixels: Option<Pixels> },
}

/// Where an embed's picture is and its size in pixels (`None`: nowhere).
type Place = Option<(PathBuf, u32, u32)>;

/// Decoded pixels, and when they arrived (for their fade in).
struct Decoded {
    pixels: Arc<Pixels>,
    at: Instant,
}

/// Pictures known so far, and those asked for.
pub struct Pictures {
    /// The worker's queue, once it runs; what it will do until then.
    jobs: Option<Sender<Job>>,
    waiting: Vec<Job>,
    results: Option<Receiver<Found>>,
    /// An embed's file and size, by the note's folder and the name.
    places: HashMap<(PathBuf, String), Place>,
    pixels: HashMap<(PathBuf, u32), Decoded>,
    asked: HashSet<(PathBuf, String)>,
    decoding: HashSet<(PathBuf, u32)>,
    /// Each vault's attachments folder, from Obsidian's settings.
    attachments: HashMap<PathBuf, Option<String>>,
}

impl Default for Pictures {
    fn default() -> Self {
        Self::new()
    }
}

impl PictureSource for Pictures {
    fn place(&mut self, vault: &Path, from: &Path, name: &str) -> Lookup {
        Pictures::place(self, vault, from, name)
    }

    fn pixels(&mut self, path: &Path, width: u32) -> Option<(Arc<Pixels>, f32)> {
        Pictures::pixels(self, path, width)
    }
}

impl Pictures {
    /// Before the first frame: lookups work, work for the worker waits.
    pub fn new() -> Pictures {
        Pictures {
            jobs: None,
            waiting: Vec::new(),
            results: None,
            places: HashMap::new(),
            pixels: HashMap::new(),
            asked: HashSet::new(),
            decoding: HashSet::new(),
            attachments: HashMap::new(),
        }
    }

    /// Starts the worker (after the first frame). `make` builds its decoder on
    /// the worker's own thread (an image library may need that); `wake` is
    /// called whenever something is ready for `poll`.
    pub fn start(&mut self, make: impl FnOnce() -> Option<Box<dyn Decoder>> + Send + 'static, wake: impl Fn() + Send + 'static) {
        let (jobs, rx) = channel();
        let (out, results) = channel();
        std::thread::spawn(move || {
            let Some(decoder) = make() else { return };
            let mut known = None;
            while let Ok(job) = rx.recv() {
                let found = match job {
                    Job::Find { vault, from, name } => {
                        let file = find(&vault, &from, &name, &mut known).and_then(|p| decoder.size(&p).map(|(w, h)| (p, w, h)));
                        Found::Place { from, name, file }
                    }
                    Job::Decode { path, width } => {
                        let pixels = decoder.decode(&path, width);
                        Found::Pixels { path, width, pixels }
                    }
                };
                if out.send(found).is_err() {
                    break;
                }
                wake();
            }
        });
        for job in self.waiting.drain(..) {
            let _ = jobs.send(job);
        }
        self.jobs = Some(jobs);
        self.results = Some(results);
    }

    fn send(&mut self, job: Job) {
        match &self.jobs {
            Some(jobs) => {
                let _ = jobs.send(job);
            }
            None => self.waiting.push(job),
        }
    }

    /// The file and size of the picture `name` in a note in `from` (a
    /// folder of `vault`), asking for it the first time.
    pub fn place(&mut self, vault: &Path, from: &Path, name: &str) -> Lookup {
        let key = (from.to_path_buf(), name.to_string());
        match self.places.get(&key) {
            Some(Some((p, w, h))) => return Lookup::Found(p.clone(), *w, *h),
            Some(None) => return Lookup::Missing,
            None => {}
        }
        if !self.asked.insert(key.clone()) {
            return Lookup::Pending;
        }
        if let Some((p, w, h)) = self.nearby(vault, from, name) {
            self.places.insert(key, Some((p.clone(), w, h)));
            return Lookup::Found(p, w, h);
        }
        self.send(Job::Find { vault: vault.to_path_buf(), from: from.to_path_buf(), name: name.to_string() });
        Lookup::Pending
    }

    /// A picture where embeds usually are (beside the note, at the vault's
    /// top, with its attachments), sized from its first bytes.
    fn nearby(&mut self, vault: &Path, from: &Path, name: &str) -> Option<(PathBuf, u32, u32)> {
        let name = name.replace("%20", " ");
        let setting = self
            .attachments
            .entry(vault.to_path_buf())
            .or_insert_with(|| std::fs::read_to_string(vault.join(".obsidian").join("app.json")).ok().and_then(|j| attachment_setting(&j)))
            .clone();
        let attachments = setting.map(|s| attachment_dir(vault, from, &s));
        [Some(from.to_path_buf()), Some(vault.to_path_buf()), attachments].into_iter().flatten().find_map(|dir| {
            let p = dir.join(&name);
            let (w, h) = header_size(&p)?;
            Some((p, w, h))
        })
    }

    /// `path` decoded `width` pixels wide, asking for it the first time;
    /// and how far it has faded in. While another size is on its way, the
    /// one there is stands in.
    pub fn pixels(&mut self, path: &Path, width: u32) -> Option<(Arc<Pixels>, f32)> {
        let key = (path.to_path_buf(), width);
        if let Some(d) = self.pixels.get(&key) {
            return Some((d.pixels.clone(), (d.at.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0)));
        }
        if self.decoding.insert(key.clone()) {
            self.send(Job::Decode { path: key.0, width });
        }
        self.pixels.iter().find(|((p, _), _)| p == path).map(|(_, d)| (d.pixels.clone(), 1.0))
    }

    /// Whether a picture is still fading in (a little past its end, so its
    /// last frame is drawn whole).
    pub fn fading(&self) -> bool {
        self.pixels.values().any(|d| d.at.elapsed() < FADE + Duration::from_millis(20))
    }

    /// Takes what the worker has found; true if the view should be drawn again.
    pub fn poll(&mut self) -> bool {
        let mut found = Vec::new();
        if let Some(rx) = &self.results {
            while let Ok(f) = rx.try_recv() {
                found.push(f);
            }
        }
        let any = !found.is_empty();
        for f in found {
            match f {
                Found::Place { from, name, file } => {
                    self.places.insert((from, name), file);
                }
                Found::Pixels { path, width, pixels } => {
                    self.decoding.remove(&(path.clone(), width));
                    if let Some(p) = pixels {
                        // A new size of a picture shown already does not fade.
                        let shown = self.pixels.keys().any(|(p, _)| *p == path);
                        let at = if shown { Instant::now() - FADE } else { Instant::now() };
                        // Other sizes of it are no longer needed.
                        self.pixels.retain(|(p, _), _| *p != path);
                        self.pixels.insert((path, width), Decoded { pixels: Arc::new(p), at });
                    }
                }
            }
        }
        any
    }

    /// Files changed (a paste, a drop): embeds are looked up again.
    pub fn forget_places(&mut self) {
        self.places.clear();
        self.asked.clear();
        self.attachments.clear();
    }
}

/// Where the vault keeps attachments, from Obsidian's `app.json`: a folder
/// from the vault's top, the note's own folder (`./`), or (unset) the top.
pub fn attachment_setting(app_json: &str) -> Option<String> {
    notsy_core::obsidian::string(app_json, "attachmentFolderPath")
}

/// The attachments folder `setting` means for a note in `from`.
pub fn attachment_dir(vault: &Path, from: &Path, setting: &str) -> PathBuf {
    match setting.trim() {
        "" | "/" => vault.to_path_buf(),
        "./" | "." => from.to_path_buf(),
        s if s.starts_with("./") => from.join(&s[2..]),
        s => vault.join(s.trim_start_matches('/')),
    }
}

/// A picture's size in pixels from its file's first bytes, for the kinds
/// that say it there (PNG, GIF, BMP, JPEG, WebP).
pub fn header_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = Vec::with_capacity(4096);
    (&mut file).take(4096).read_to_end(&mut head).ok()?;
    // A JPEG's size can follow its metadata (a camera's thumbnail): most
    // say it in the first 64 KB.
    if head.starts_with(&[0xFF, 0xD8]) && head.len() == 4096 && sniff_size(&head).is_none() {
        file.take(60 * 1024).read_to_end(&mut head).ok()?;
    }
    sniff_size(&head).filter(|&(w, h)| w > 0 && h > 0)
}

fn sniff_size(b: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32);
    let be32 = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let le16 = |i: usize| Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32);
    let le24 = |i: usize| Some(le16(i)? | (*b.get(i + 2)? as u32) << 16);
    let le32 = |i: usize| Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?));
    if b.starts_with(b"\x89PNG\r\n\x1a\n") && b.get(12..16) == Some(b"IHDR") {
        return Some((be32(16)?, be32(20)?));
    }
    if b.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if b.starts_with(b"BM") {
        // An old core header holds 16-bit sizes; later ones 32-bit, the
        // height negative for top-down rows.
        return if le32(14)? == 12 {
            Some((le16(18)?, le16(20)?))
        } else {
            Some((le32(18)?, (le32(22)? as i32).unsigned_abs()))
        };
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        return match b.get(12..16)? {
            b"VP8 " => Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF)),
            b"VP8L" => {
                let bits = le32(21)?;
                Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
            }
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        };
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 <= b.len() {
            if b[i] != 0xFF {
                return None;
            }
            let marker = b[i + 1];
            match marker {
                // Padding, and markers without a length.
                0xFF => i += 1,
                0x01 | 0xD0..=0xD8 => i += 2,
                // A frame's start (not the Huffman, arithmetic or JPEG-LS
                // tables that share the range) holds height, then width.
                0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => return Some((be16(i + 7)?, be16(i + 5)?)),
                _ => i += 2 + be16(i + 2)? as usize,
            }
        }
    }
    None
}

/// The vault's picture files, by lowercased name.
fn index(vault: &Path) -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    let mut dirs = vec![vault.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => dirs.push(e.path()),
                Ok(_) if is_image(&name) => {
                    out.entry(name.to_lowercase()).or_insert_with(|| e.path());
                }
                _ => {}
            }
        }
    }
    out
}

/// The file an embed names: a path from the note's folder or the vault's
/// top, or a file of that name anywhere in the vault.
pub fn find(vault: &Path, from: &Path, name: &str, known: &mut Option<(PathBuf, HashMap<String, PathBuf>)>) -> Option<PathBuf> {
    let name = name.replace("%20", " ");
    for base in [from, vault] {
        let p = base.join(&name);
        if p.is_file() {
            return Some(p);
        }
    }
    if known.as_ref().is_none_or(|(v, _)| v != vault) {
        *known = Some((vault.to_path_buf(), index(vault)));
    }
    let file = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_lowercase();
    let found = known.as_ref()?.1.get(&file).cloned();
    // Pasted since the index was made.
    found.or_else(|| {
        *known = Some((vault.to_path_buf(), index(vault)));
        known.as_ref()?.1.get(&file).cloned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sizes_from_first_bytes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&900u32.to_be_bytes());
        png.extend_from_slice(&360u32.to_be_bytes());
        assert_eq!(sniff_size(&png), Some((900, 360)));
        assert_eq!(sniff_size(b"GIF89a\x40\x01\xf0\x00"), Some((320, 240)));
        // A JPEG: start, an APP0 segment of 16 bytes, then a baseline frame.
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        jpg.extend_from_slice(&[0; 14]);
        jpg.extend_from_slice(&[0xFF, 0xC0, 0, 17, 8, 0x01, 0xE0, 0x02, 0x80, 3]);
        assert_eq!(sniff_size(&jpg), Some((640, 480)));
        // WebP, lossless: 14 bits of width - 1, then of height - 1.
        let mut webp = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0\x2f".to_vec();
        webp.extend_from_slice(&((99u32) | (49u32 << 14)).to_le_bytes());
        assert_eq!(sniff_size(&webp), Some((100, 50)));
        // A bitmap, top-down.
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0; 12]);
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&64u32.to_le_bytes());
        bmp.extend_from_slice(&(-32i32).to_le_bytes());
        assert_eq!(sniff_size(&bmp), Some((64, 32)));
        assert_eq!(sniff_size(b"not a picture"), None);
    }

    #[test]
    fn finds_the_attachments_folder() {
        let (v, n) = (Path::new("V"), Path::new("V/Projects"));
        assert_eq!(attachment_dir(v, n, ""), Path::new("V"));
        assert_eq!(attachment_dir(v, n, "./"), Path::new("V/Projects"));
        assert_eq!(attachment_dir(v, n, "./img"), Path::new("V/Projects").join("img"));
        assert_eq!(attachment_dir(v, n, "attachments"), Path::new("V").join("attachments"));
        assert_eq!(attachment_setting(r#"{"attachmentFolderPath": "attachments", "x": 1}"#).as_deref(), Some("attachments"));
        assert_eq!(attachment_setting(r#"{"promptDelete":false,"attachmentFolderPath":"./"}"#).as_deref(), Some("./"));
        assert_eq!(attachment_setting("{}"), None);
    }
}
