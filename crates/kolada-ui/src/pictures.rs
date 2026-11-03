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

/// Sizes of one picture kept at once: those drawn within `IN_USE`, the
/// most recent first (a picture on its card and in the note, or in a list
/// item and on its own, is drawn at two sizes in one frame).
const SIZES: usize = 4;
const IN_USE: Duration = Duration::from_secs(1);

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

/// Decoded pixels, when they arrived (for their fade in), and when they
/// were last drawn.
struct Decoded {
    pixels: Arc<Pixels>,
    at: Instant,
    used: Instant,
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
        if let Some(d) = self.pixels.get_mut(&key) {
            d.used = Instant::now();
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
                        // Other sizes of it go, but the few drawn just now:
                        // dropping one still on screen would ask for it again,
                        // and that again drop this one, without end.
                        let mut kept: Vec<(u32, Instant)> = self.pixels.iter().filter(|((q, _), d)| *q == path && d.used.elapsed() < IN_USE).map(|((_, w), d)| (*w, d.used)).collect();
                        kept.sort_by_key(|(_, used)| std::cmp::Reverse(*used));
                        kept.truncate(SIZES - 1);
                        self.pixels.retain(|(q, w), _| *q != path || kept.iter().any(|(k, _)| k == w));
                        self.pixels.insert((path, width), Decoded { pixels: Arc::new(p), at, used: Instant::now() });
                    }
                }
            }
        }
        any
    }

    /// `path` was written again (a drawing drawn on): decoded afresh.
    pub fn forget(&mut self, path: &Path) {
        self.pixels.retain(|(p, _), _| p != path);
        self.decoding.retain(|(p, _)| p != path);
        self.forget_places();
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
    kolada_core::obsidian::string(app_json, "attachmentFolderPath")
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

/// The pictures a note embeds (a line of its own: `![[photo.png]]`,
/// `![](cats/tabby.webp)`) that are in the vault, once each: the embed's
/// target as the note writes it, and its file. What Beam sends with a note.
pub fn note_pictures(vault: &Path, from: &Path, text: &str) -> Vec<(String, PathBuf)> {
    let mut known = None;
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for line in text.lines() {
        let Some(target) = crate::images::embed_in(line) else { continue };
        if out.iter().any(|(t, _)| t == target) {
            continue;
        }
        if let Some(file) = find(vault, from, target, &mut known) {
            out.push((target.to_string(), file));
        }
    }
    out
}

/// A picture a note embeds, read for Beam: its target as the note writes
/// it, its type, its bytes and its size in pixels.
pub struct NoteFile {
    pub name: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The note's pictures (`note_pictures`) read from the vault, each up to
/// `most` bytes; a bigger one, or one that can't be read, is left out.
pub fn read_note_pictures(vault: &Path, from: &Path, text: &str, most: usize) -> Vec<NoteFile> {
    note_pictures(vault, from, text)
        .into_iter()
        .filter_map(|(name, file)| {
            let bytes = std::fs::read(&file).ok().filter(|b| !b.is_empty() && b.len() <= most)?;
            let (width, height) = header_size(&file).unwrap_or((0, 0));
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
            let mime = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                "bmp" => "image/bmp",
                "tif" | "tiff" => "image/tiff",
                "ico" => "image/x-icon",
                _ => "image/png",
            };
            Some(NoteFile { name, mime, bytes, width, height })
        })
        .collect()
}

/// Where a picture beamed with a note goes when the note is saved in `dir`:
/// at its target as the note writes it, so the note reads the same. None
/// for a target that would leave `dir` (`..`, a drive, a root).
pub fn note_picture_path(dir: &Path, target: &str) -> Option<PathBuf> {
    let t = target.replace("%20", " ").replace('\\', "/");
    if t.starts_with('/') || t.contains(':') {
        return None;
    }
    let mut p = dir.to_path_buf();
    for part in t.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            s => p.push(s),
        }
    }
    (p != dir).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes a blank picture at any width, counting how often.
    struct Counting(Arc<std::sync::atomic::AtomicUsize>);

    impl Decoder for Counting {
        fn size(&self, _: &Path) -> Option<(u32, u32)> {
            Some((800, 600))
        }
        fn decode(&self, _: &Path, width: u32) -> Option<Pixels> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(Pixels { width, height: width * 3 / 4, data: vec![0; (width * width * 3 / 4) as usize] })
        }
    }

    /// Frames until no decode is waiting: how many it took.
    fn settle(pics: &mut Pictures, path: &Path, widths: &[u32]) -> usize {
        for frame in 1..=50 {
            let drawn: Vec<bool> = widths.iter().map(|w| pics.pixels.contains_key(&(path.to_path_buf(), *w))).collect();
            for w in widths {
                pics.pixels(path, *w);
            }
            if drawn.iter().all(|d| *d) && pics.decoding.is_empty() {
                return frame;
            }
            std::thread::sleep(Duration::from_millis(20));
            pics.poll();
        }
        panic!("still decoding after 50 frames");
    }

    /// A picture drawn at two sizes in one frame (on its card and in the
    /// note, or in a list item and on its own) is decoded once at each and
    /// then left alone, not decoded over and over as each size pushes the
    /// other out; sizes no longer drawn go, and only a few are ever kept.
    #[test]
    fn a_picture_at_two_sizes_at_once_is_decoded_once_each() {
        let decodes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut pics = Pictures::new();
        let counter = decodes.clone();
        pics.start(move || Some(Box::new(Counting(counter)) as Box<dyn Decoder>), || {});
        let path = Path::new("Coast.png");
        settle(&mut pics, path, &[640, 300]);
        let count = decodes.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(count, 2, "once at each size");
        for _ in 0..10 {
            pics.pixels(path, 640);
            pics.pixels(path, 300);
            std::thread::sleep(Duration::from_millis(10));
            pics.poll();
        }
        assert_eq!(decodes.load(std::sync::atomic::Ordering::SeqCst), 2, "and then left alone");
        // A window dragged wider: many sizes, one after another; few kept.
        for w in (700..1000).step_by(20) {
            settle(&mut pics, path, &[w]);
        }
        assert!(pics.pixels.len() <= SIZES, "{} sizes kept", pics.pixels.len());
    }

    /// A note's embedded pictures, found where Obsidian would, once each;
    /// and where each goes beside the note when it is saved, never outside.
    #[test]
    fn a_notes_pictures_are_found_and_put_back_inside_the_folder() {
        let dir = std::env::temp_dir().join(format!("kolada-note-pictures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("attachments")).unwrap();
        std::fs::create_dir_all(dir.join("cats")).unwrap();
        std::fs::write(dir.join("attachments").join("photo.png"), b"png").unwrap();
        std::fs::write(dir.join("cats").join("tabby.webp"), b"webp").unwrap();
        let text = "# Plan
![[photo.png]]
some text
![a cat](cats/tabby.webp)
![[photo.png|300]]
![[missing.png]]
![x](https://example.com/a.png)
";
        let found = note_pictures(&dir, &dir, text);
        assert_eq!(found.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["photo.png", "cats/tabby.webp"]);
        assert_eq!(found[0].1, dir.join("attachments").join("photo.png"), "found by name, where Obsidian keeps it");
        let to = std::path::Path::new("vault");
        assert_eq!(note_picture_path(to, "photo.png"), Some(to.join("photo.png")));
        assert_eq!(note_picture_path(to, "cats/tabby.webp"), Some(to.join("cats").join("tabby.webp")));
        assert_eq!(note_picture_path(to, "../secret.png"), None);
        assert_eq!(note_picture_path(to, "/etc/a.png"), None);
        assert_eq!(note_picture_path(to, "C:/a.png"), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Every way a note embeds a picture finds its file: a name with
    /// spaces, a path from the note, one written with %20, the attachments
    /// folder Obsidian is set to, a pasted picture's name, and a file of
    /// that name elsewhere in the vault; where embeds usually are, without
    /// a walk of the vault (`nearby`).
    #[test]
    fn every_embed_form_finds_its_picture() {
        let vault = std::env::temp_dir().join(format!("kolada-embed-forms-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        let png = |rel: &str| {
            let p = vault.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            // A 2 x 1 PNG's first bytes: enough for its size.
            let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            b.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
            std::fs::write(&p, b).unwrap();
            p
        };
        std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
        std::fs::write(vault.join(".obsidian").join("app.json"), r#"{"attachmentFolderPath":"attachments"}"#).unwrap();
        let from = vault.join("Trips");
        let cases = [
            ("![[Welcome to Kolada.png]]", png("Welcome to Kolada.png"), true),
            ("![](<shots/coast walk.png>)", png("Trips/shots/coast walk.png"), true),
            ("![a view](shots/view.png)", png("Trips/shots/view.png"), true),
            ("![](coast%20walk%202.png)", png("Trips/coast walk 2.png"), true),
            ("![[garden plan.png]]", png("attachments/garden plan.png"), true),
            ("![[Pasted image 20261102.png]]", png("attachments/Pasted image 20261102.png"), true),
            ("![[far away.png]]", png("Archive/Old/far away.png"), false),
        ];
        let mut known = None;
        let mut pictures = Pictures::new();
        for (line, file, near) in &cases {
            let target = crate::images::embed_in(line).unwrap_or_else(|| panic!("{line} is an embed"));
            assert_eq!(find(&vault, &from, target, &mut known).as_ref(), Some(file), "{line}");
            assert_eq!(pictures.nearby(&vault, &from, target).map(|(p, w, h)| (p, w, h)), near.then(|| (file.clone(), 2, 1)), "{line}, where embeds usually are");
        }
        // Inside a list item, a numbered one, a task, a quote and a nested
        // list: the same picture.
        for line in ["- ![[garden plan.png]]", "3. ![[garden plan.png]]", "- [ ] ![[garden plan.png]]", "- [x] ![](<garden plan.png>)", "> ![[garden plan.png]]", "    - ![[garden plan.png|300]]", "> - ![[garden plan.png]]"] {
            let target = crate::images::embed_in(line).unwrap_or_else(|| panic!("{line} is an embed"));
            assert_eq!(find(&vault, &from, target, &mut known), Some(vault.join("attachments").join("garden plan.png")), "{line}");
        }
        assert_eq!(crate::images::embed_width("    - ![[garden plan.png|300]]"), Some(300));
        assert_eq!(crate::images::with_width("> - ![[garden plan.png|300]]", Some(120)), "> - ![[garden plan.png|120]]", "its marks kept as it is resized");
        assert_eq!(crate::images::embed_in("- see ![[garden plan.png]]"), None, "words beside it: not a picture line");
        assert_eq!(find(&vault, &from, "not here.png", &mut known), None);
        let _ = std::fs::remove_dir_all(vault);
    }

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
