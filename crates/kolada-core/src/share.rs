//! A note shared as a short link on notsy.co, the part every shell shares:
//! the install's key, the note and its pictures made ready to send (EXIF and
//! GPS stripped without recompressing, large pictures made smaller, within
//! the size a shared note may be), the request's body, and what the server
//! answers. No JSON and no HTTP library: bodies are `key=value` lines and
//! tab-separated rows (see `services/notsy`). The shell does the sending, on
//! a thread of its own, and decodes and encodes pictures with its OS's
//! imaging (`Recode`). Nothing here runs before the first frame: only when
//! the user shares.

use std::path::{Path, PathBuf};

/// Where links are made.
pub const BASE: &str = "https://notsy.co";
/// How large a shared note may be, its Markdown and its pictures together,
/// unless the server says otherwise (its 413 carries its own).
pub const LIMIT: usize = 1_000_000;
/// How many links an install may have live at once.
pub const MOST_LIVE: usize = 3;
/// A JPEG or WebP this small (bytes) and no wider than `STEPS[0]` is sent as
/// it is, its metadata stripped, never recompressed.
pub const SMALL: usize = 300_000;
/// How pictures are made smaller: their longer side, and the JPEG quality,
/// tried in turn while the note is over its size.
pub const STEPS: [(u32, u8); 3] = [(1600, 82), (1200, 72), (900, 62)];

/// How long a link lives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Expiry {
    Day,
    #[default]
    Week,
    Month,
}

impl Expiry {
    pub const ALL: [Expiry; 3] = [Expiry::Day, Expiry::Week, Expiry::Month];

    pub fn days(self) -> u32 {
        match self {
            Expiry::Day => 1,
            Expiry::Week => 7,
            Expiry::Month => 30,
        }
    }

    /// What its pill says.
    pub fn label(self) -> &'static str {
        match self {
            Expiry::Day => "1 day",
            Expiry::Week => "7 days",
            Expiry::Month => "30 days",
        }
    }

    pub fn from_days(days: u32) -> Option<Expiry> {
        Expiry::ALL.into_iter().find(|e| e.days() == days)
    }
}

// The install's key.

/// The file in the app's data folder that holds the install's key (a test
/// instance's data folder is its own, so its key is too).
pub fn key_path(data: &Path) -> PathBuf {
    data.join("share.key")
}

/// 32 random bytes as the server takes them: base64url, without padding.
pub fn key_text(bytes: &[u8; 32]) -> String {
    base64url(bytes)
}

/// A key as kept in its file, if it is one (43 base64url characters).
pub fn read_key(text: &str) -> Option<String> {
    let t = text.trim();
    (t.len() == 43 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')).then(|| t.to_string())
}

/// The install's key, made the first time it is needed from `fresh` (the
/// OS's random bytes) and kept in `data` from then on.
pub fn load_or_make_key(data: &Path, fresh: impl FnOnce() -> Option<[u8; 32]>) -> Option<String> {
    let path = key_path(data);
    if let Some(key) = std::fs::read_to_string(&path).ok().as_deref().and_then(read_key) {
        return Some(key);
    }
    let key = key_text(&fresh()?);
    std::fs::create_dir_all(data).ok()?;
    std::fs::write(&path, &key).ok()?;
    Some(key)
}

fn base64url(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 2);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

// Pictures.

/// A picture's file type, by how it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Webp,
    Gif,
    Other,
}

impl Kind {
    pub fn of(b: &[u8]) -> Kind {
        if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Kind::Jpeg
        } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
            Kind::Png
        } else if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
            Kind::Webp
        } else if b.starts_with(b"GIF8") {
            Kind::Gif
        } else {
            Kind::Other
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Kind::Jpeg => "jpg",
            Kind::Png => "png",
            Kind::Webp => "webp",
            Kind::Gif => "gif",
            Kind::Other => "",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Kind::Jpeg => "image/jpeg",
            Kind::Png => "image/png",
            Kind::Webp => "image/webp",
            Kind::Gif => "image/gif",
            Kind::Other => "application/octet-stream",
        }
    }
}

/// A picture's size in pixels, from its header.
pub fn dimensions(b: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32);
    let be32 = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let le16 = |i: usize| Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32);
    let le24 = |i: usize| Some(le16(i)? | (*b.get(i + 2)? as u32) << 16);
    let le32 = |i: usize| Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?));
    match Kind::of(b) {
        Kind::Png if b.get(12..16) == Some(b"IHDR") => Some((be32(16)?, be32(20)?)),
        Kind::Gif => Some((le16(6)?, le16(8)?)),
        Kind::Webp => match b.get(12..16)? {
            b"VP8 " => Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF)),
            b"VP8L" => {
                let bits = le32(21)?;
                Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
            }
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        },
        Kind::Jpeg => {
            let mut i = 2;
            while i + 9 <= b.len() {
                if b[i] != 0xFF {
                    return None;
                }
                let marker = b[i + 1];
                match marker {
                    0xFF => i += 1,
                    0x01 | 0xD0..=0xD8 => i += 2,
                    0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => return Some((be16(i + 7)?, be16(i + 5)?)),
                    _ => i += 2 + be16(i + 2)? as usize,
                }
            }
            None
        }
        _ => None,
    }
}

/// How a camera says a photo is turned (EXIF orientation, 1 to 8; 1 when it
/// says nothing): a photo sent without it would show on its side.
pub fn orientation(b: &[u8]) -> u16 {
    let tiff = match Kind::of(b) {
        Kind::Jpeg => jpeg_segments(b).into_iter().find_map(|(marker, data)| (marker == 0xE1).then(|| data.strip_prefix(b"Exif\0\0")).flatten()),
        Kind::Webp => riff_chunks(b).into_iter().find_map(|(id, data)| (id == *b"EXIF").then_some(data)).map(|d| d.strip_prefix(b"Exif\0\0").unwrap_or(d)),
        Kind::Png => png_chunks(b).into_iter().find_map(|(id, data)| (id == *b"eXIf").then_some(data)),
        _ => None,
    };
    tiff.and_then(tiff_orientation).filter(|o| (1..=8).contains(o)).unwrap_or(1)
}

fn tiff_orientation(t: &[u8]) -> Option<u16> {
    let big = match t.get(0..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |i: usize| -> Option<u16> {
        let b: [u8; 2] = t.get(i..i + 2)?.try_into().ok()?;
        Some(if big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
    };
    let u32_at = |i: usize| -> Option<u32> {
        let b: [u8; 4] = t.get(i..i + 4)?.try_into().ok()?;
        Some(if big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    (0..count.min(512)).map(|k| ifd + 2 + 12 * k).find(|&e| u16_at(e) == Some(0x0112)).and_then(|e| u16_at(e + 8))
}

/// A JPEG's segments before its first scan: (marker, data without length).
fn jpeg_segments(b: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    let mut i = 2;
    while i + 4 <= b.len() && b[i] == 0xFF {
        let marker = b[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let Some(data) = b.get(i + 4..i + 2 + len) else { break };
        out.push((marker, data));
        i += 2 + len;
    }
    out
}

/// Whether a JPEG segment is kept when its metadata is stripped: JFIF, the
/// colour profile (photos look washed out without it) and Adobe's (how its
/// colours are coded) stay; EXIF, GPS, XMP, IPTC and the rest go.
fn kept_app(marker: u8, data: &[u8]) -> bool {
    match marker {
        0xE0 => true,
        0xE2 => data.starts_with(b"ICC_PROFILE\0"),
        0xEE => data.starts_with(b"Adobe"),
        0xE1..=0xEF => false,
        _ => true,
    }
}

/// A JPEG without its metadata segments (EXIF, GPS, XMP, IPTC: APP1 to
/// APP15 save the colour profile and Adobe's), every other byte as it was:
/// the picture is not recompressed. What follows its end (more pictures a
/// phone tucks in after it, each with its own EXIF) goes too.
pub fn strip_jpeg(b: &[u8]) -> Option<Vec<u8>> {
    if Kind::of(b) != Kind::Jpeg {
        return None;
    }
    let mut out = Vec::with_capacity(b.len());
    out.extend_from_slice(&b[..2]);
    let mut i = 2;
    loop {
        // A marker, after any fill bytes.
        while b.get(i) == Some(&0xFF) && b.get(i + 1) == Some(&0xFF) {
            i += 1;
        }
        if b.get(i) != Some(&0xFF) {
            return None;
        }
        let marker = *b.get(i + 1)?;
        match marker {
            0xD9 => {
                out.extend_from_slice(&b[i..i + 2]);
                return Some(out);
            }
            0x01 | 0xD0..=0xD7 => {
                out.extend_from_slice(&b[i..i + 2]);
                i += 2;
                continue;
            }
            _ => {}
        }
        let len = u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]) as usize;
        let end = i + 2 + len;
        let data = b.get(i + 4..end)?;
        if kept_app(marker, data) {
            out.extend_from_slice(&b[i..end]);
        }
        i = end;
        if marker == 0xDA {
            // The scan's coded data, to the next marker (0xFF then 0 is a
            // byte of it; restarts are part of it).
            let start = i;
            while i + 1 < b.len() && !(b[i] == 0xFF && b[i + 1] != 0 && !(0xD0..=0xD7).contains(&b[i + 1])) {
                i += 1;
            }
            if i + 1 >= b.len() {
                // No end marker: what there is.
                out.extend_from_slice(&b[start..]);
                return Some(out);
            }
            out.extend_from_slice(&b[start..i]);
        }
    }
}

/// A RIFF file's chunks: (FourCC, data).
fn riff_chunks(b: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut i = 12;
    while i + 8 <= b.len() {
        let id: [u8; 4] = b[i..i + 4].try_into().unwrap();
        let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        let Some(data) = b.get(i + 8..i + 8 + len) else { break };
        out.push((id, data));
        i += 8 + len + (len & 1);
    }
    out
}

/// A WebP without its EXIF and XMP chunks (its header's flags and size set
/// to match), every other chunk as it was.
pub fn strip_webp(b: &[u8]) -> Option<Vec<u8>> {
    if Kind::of(b) != Kind::Webp {
        return None;
    }
    let mut out = b[..12].to_vec();
    for (id, data) in riff_chunks(b) {
        if id == *b"EXIF" || id == *b"XMP " {
            continue;
        }
        out.extend_from_slice(&id);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        if &id == b"VP8X" && !data.is_empty() {
            // Flags: ICC 0x20, alpha 0x10, EXIF 0x08, XMP 0x04, animation 0x02.
            let at = out.len() - data.len();
            out[at] &= !(0x08 | 0x04);
        }
        if data.len() & 1 == 1 {
            out.push(0);
        }
    }
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    Some(out)
}

/// A PNG's chunks: (type, data).
fn png_chunks(b: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut i = 8;
    while i + 12 <= b.len() {
        let len = u32::from_be_bytes(b[i..i + 4].try_into().unwrap()) as usize;
        let id: [u8; 4] = b[i + 4..i + 8].try_into().unwrap();
        let Some(data) = b.get(i + 8..i + 8 + len) else { break };
        out.push((id, data));
        i += 12 + len;
    }
    out
}

/// A PNG without its text, time and EXIF chunks (where a camera or an
/// editor writes who, where and when), every other chunk as it was.
pub fn strip_png(b: &[u8]) -> Option<Vec<u8>> {
    if Kind::of(b) != Kind::Png {
        return None;
    }
    let mut out = b[..8].to_vec();
    let mut i = 8;
    while i + 12 <= b.len() {
        let len = u32::from_be_bytes(b[i..i + 4].try_into().ok()?) as usize;
        let end = (i + 12 + len).min(b.len());
        let id = &b[i + 4..i + 8];
        if !matches!(id, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME") {
            out.extend_from_slice(&b[i..end]);
        }
        if id == b"IEND" {
            break;
        }
        i = end;
    }
    Some(out)
}

/// Pixels as the shell's imaging hands them over: straight (not
/// premultiplied) BGRA, 4 bytes each, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Raster {
    /// Whether any pixel is less than opaque.
    pub fn has_alpha(&self) -> bool {
        self.bgra.chunks_exact(4).any(|p| p[3] < 255)
    }

    /// Turned upright, as its EXIF orientation says.
    pub fn oriented(self, o: u16) -> Raster {
        if o <= 1 || o > 8 {
            return self;
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
        let mut bgra = vec![0u8; self.bgra.len()];
        for oy in 0..oh {
            for ox in 0..ow {
                let (sx, sy) = match o {
                    2 => (w - 1 - ox, oy),
                    3 => (w - 1 - ox, h - 1 - oy),
                    4 => (ox, h - 1 - oy),
                    5 => (oy, ox),
                    6 => (oy, h - 1 - ox),
                    7 => (w - 1 - oy, h - 1 - ox),
                    _ => (w - 1 - oy, ox),
                };
                let (s, d) = (4 * (sy * w + sx), 4 * (oy * ow + ox));
                bgra[d..d + 4].copy_from_slice(&self.bgra[s..s + 4]);
            }
        }
        Raster { width: ow as u32, height: oh as u32, bgra }
    }
}

/// What a shell's imaging does for sharing (Windows' WIC, the Mac's
/// ImageIO). Called from the thread that shares, never the UI's.
pub trait Recode {
    /// The picture in `bytes`, decoded and made smaller (never larger) so
    /// its longer side is at most `edge` pixels; None if it cannot be read.
    fn decode(&self, bytes: &[u8], edge: u32) -> Option<Raster>;
    /// The pixels as a JPEG at `quality` (1 to 100), or as a PNG.
    fn encode(&self, raster: &Raster, quality: u8, png: bool) -> Option<Vec<u8>>;
}

/// A picture made ready to send: its bytes and file type.
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub bytes: Vec<u8>,
    pub kind: Kind,
}

/// A small picture sent as it is, its metadata stripped: a JPEG, WebP, PNG
/// or GIF within `SMALL` bytes and `STEPS[0]`'s size, and upright as stored.
fn as_it_is(b: &[u8]) -> Option<Prepared> {
    let kind = Kind::of(b);
    let (w, h) = dimensions(b)?;
    if b.len() > SMALL || w.max(h) > STEPS[0].0 || orientation(b) != 1 {
        return None;
    }
    let bytes = match kind {
        Kind::Jpeg => strip_jpeg(b)?,
        Kind::Webp => strip_webp(b)?,
        Kind::Png => strip_png(b)?,
        Kind::Gif => b.to_vec(),
        Kind::Other => return None,
    };
    Some(Prepared { bytes, kind })
}

/// A picture decoded, made smaller, turned upright and encoded again: JPEG,
/// or PNG where it is see-through.
fn recoded(b: &[u8], (edge, quality): (u32, u8), recode: &dyn Recode) -> Option<Prepared> {
    let raster = recode.decode(b, edge)?.oriented(orientation(b));
    let png = raster.has_alpha();
    let bytes = recode.encode(&raster, quality, png)?;
    Some(Prepared { kind: if png { Kind::Png } else { Kind::Jpeg }, bytes })
}

/// A picture made ready at a step (0: the first try, 1 and 2 smaller): sent
/// as it is when small, else made smaller; at a later step, whichever of
/// the two is lighter.
pub fn prepare(b: &[u8], step: usize, recode: &dyn Recode) -> Option<Prepared> {
    let step = step.min(STEPS.len() - 1);
    let kept = as_it_is(b);
    if step == 0 && kept.is_some() {
        return kept;
    }
    let made = recoded(b, STEPS[step], recode);
    match (kept, made) {
        (Some(k), Some(m)) if k.bytes.len() <= m.bytes.len() => Some(k),
        (k, m) => m.or(k),
    }
}

// The note and its pictures.

/// A picture the note embeds: where in the text, the file it names, and
/// what it says about the picture (Obsidian's `|300` size dropped).
#[derive(Clone, Debug, PartialEq)]
struct Embed {
    start: usize,
    end: usize,
    target: String,
    alt: String,
}

/// File types read as pictures when sharing (more than are shown: the OS
/// may read a phone's HEIC).
const PICTURES: [&str; 13] = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico", "heic", "heif", "avif", "jfif"];

fn is_picture_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| PICTURES.contains(&ext.to_ascii_lowercase().as_str()))
}

/// What is left of an embed's words once a size (`300`, `300x200`) is taken off.
fn without_size(words: &str) -> String {
    let words = match words.rsplit_once('|') {
        Some((rest, size)) if size.trim().split('x').all(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit())) => rest,
        _ if words.trim().split('x').all(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit())) => "",
        _ => words,
    };
    words.replace(['[', ']'], "").trim().to_string()
}

/// The note's local pictures (`![[photo.png]]`, `![[photo.png|300]]`,
/// `![what](photos/a.jpg)`), outside code. Pictures on the web stay links.
fn embeds(text: &str) -> Vec<Embed> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let start = at;
        at += line.len();
        let t = line.trim_start();
        if let Some(f) = fence {
            if t.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some(&t[..3]);
            continue;
        }
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'`' {
                // Inline code, to its closing run of the same length.
                let run = b[i..].iter().take_while(|&&c| c == b'`').count();
                let close = "`".repeat(run);
                i = match line[i + run..].find(&close) {
                    Some(k) => i + run + k + run,
                    None => i + run,
                };
                continue;
            }
            if b[i] == b'!' && b.get(i + 1) == Some(&b'[') {
                if let Some(e) = embed_at(line, i) {
                    i = e.end;
                    out.push(Embed { start: start + e.start, end: start + e.end, ..e });
                    continue;
                }
            }
            i += 1;
        }
    }
    out
}

/// The picture embed starting at `i` (at its `!`) in `line`, if one does.
fn embed_at(line: &str, i: usize) -> Option<Embed> {
    let rest = &line[i..];
    if let Some(inner) = rest.strip_prefix("![[") {
        let end = inner.find("]]")?;
        let inner = &inner[..end];
        if inner.contains('\n') {
            return None;
        }
        let (target, words) = inner.split_once('|').unwrap_or((inner, ""));
        let target = target.trim();
        return is_picture_name(target).then(|| Embed { start: i, end: i + 3 + end + 2, target: target.to_string(), alt: without_size(words) });
    }
    let after = rest.strip_prefix("![")?;
    let close = after.find("](")?;
    let alt = &after[..close];
    if alt.contains(['\n', '[']) {
        return None;
    }
    let url = &after[close + 2..];
    let end = url.find(')')?;
    let inside = url[..end].trim();
    // A title after the address ("a.png "Title"), and <brackets>.
    let target = inside.split_whitespace().next().unwrap_or("");
    let target = target.strip_prefix('<').and_then(|t| t.strip_suffix('>')).unwrap_or(target);
    let local = !target.contains("://") && !target.starts_with("data:") && !target.starts_with("//");
    (local && is_picture_name(target)).then(|| Embed { start: i, end: i + 2 + close + 2 + end + 1, target: target.to_string(), alt: without_size(alt) })
}

/// A file going up with the note: its name in the note (`0.jpg`), the name
/// the note gave it, and its bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Upload {
    pub name: String,
    pub from: String,
    pub kind: Kind,
    pub bytes: Vec<u8>,
}

/// A note made ready to share: its Markdown with each picture embed naming
/// the file sent with it, and those files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bundle {
    pub note: String,
    pub files: Vec<Upload>,
}

impl Bundle {
    /// What the server counts: the Markdown's bytes and the pictures'.
    pub fn size(&self) -> usize {
        self.note.len() + self.files.iter().map(|f| f.bytes.len()).sum::<usize>()
    }

    /// The name the note gave an uploaded file (`0.jpg`), for what to say.
    pub fn original(&self, uploaded: &str) -> Option<&str> {
        self.files.iter().find(|f| f.name == uploaded).map(|f| file_name(&f.from))
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The note `text` made ready to share within `limit` bytes. `read` gives a
/// picture's bytes by the name its embed uses (None: there is none, or it
/// cannot be read). Pictures are sent as they are when small, else made
/// smaller, then smaller again while the note is over; if it still is, it
/// is not shared and the answer says why: nothing is ever left out.
pub fn bundle(text: &str, read: &mut dyn FnMut(&str) -> Option<Vec<u8>>, recode: &dyn Recode, limit: usize) -> Result<Bundle, Problem> {
    if text.trim().is_empty() {
        return Err(Problem::Empty);
    }
    let found = embeds(text);
    let mut names: Vec<String> = Vec::new();
    for e in &found {
        if !names.contains(&e.target) {
            names.push(e.target.clone());
        }
    }
    let mut originals = Vec::with_capacity(names.len());
    for name in &names {
        match read(name) {
            Some(b) => originals.push(b),
            None => return Err(Problem::Unreadable(file_name(name).to_string())),
        }
    }
    let pictures = !names.is_empty();
    let mut size = text.len() + originals.iter().map(Vec::len).sum::<usize>();
    if text.len() > limit {
        return Err(Problem::TooBig { bytes: text.len(), limit, pictures });
    }
    for step in 0..STEPS.len() {
        let mut files = Vec::with_capacity(names.len());
        for (k, (name, bytes)) in names.iter().zip(&originals).enumerate() {
            let Some(p) = prepare(bytes, step, recode) else { return Err(Problem::Unreadable(file_name(name).to_string())) };
            files.push(Upload { name: format!("{k}.{}", p.kind.ext()), from: name.clone(), kind: p.kind, bytes: p.bytes });
        }
        let note = rewrite(text, &found, &files);
        let b = Bundle { note, files };
        size = b.size();
        if size <= limit {
            return Ok(b);
        }
    }
    Err(Problem::TooBig { bytes: size, limit, pictures })
}

/// `text` with each picture embed naming the file sent for it.
fn rewrite(text: &str, found: &[Embed], files: &[Upload]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for e in found {
        let Some(f) = files.iter().find(|f| f.from == e.target) else { continue };
        out.push_str(&text[at..e.start]);
        out.push_str(&format!("![{}]({})", e.alt, f.name));
        at = e.end;
    }
    out.push_str(&text[at..]);
    out
}

// The request.

/// A boundary for the request's parts that none of them holds.
pub fn boundary(bundle: &Bundle, seed: u64) -> String {
    let mut n = seed;
    loop {
        let b = format!("kolada-{n:016x}");
        let inside = |hay: &[u8]| hay.windows(b.len()).any(|w| w == b.as_bytes());
        if !inside(bundle.note.as_bytes()) && !bundle.files.iter().any(|f| inside(&f.bytes)) {
            return b;
        }
        n = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
    }
}

/// The request's `Content-Type`.
pub fn content_type(boundary: &str) -> String {
    format!("multipart/form-data; boundary={boundary}")
}

/// The body of `POST /api/links`: how long it lives, the note, its pictures.
pub fn multipart(boundary: &str, expiry: Expiry, bundle: &Bundle) -> Vec<u8> {
    let mut out = Vec::with_capacity(bundle.size() + 512);
    let mut part = |headers: String, data: &[u8]| {
        out.extend_from_slice(format!("--{boundary}\r\n{headers}\r\n\r\n").as_bytes());
        out.extend_from_slice(data);
        out.extend_from_slice(b"\r\n");
    };
    part("Content-Disposition: form-data; name=\"expires\"".into(), expiry.days().to_string().as_bytes());
    part("Content-Disposition: form-data; name=\"note\"\r\nContent-Type: text/markdown; charset=utf-8".into(), bundle.note.as_bytes());
    for f in &bundle.files {
        part(format!("Content-Disposition: form-data; name=\"{0}\"; filename=\"{0}\"\r\nContent-Type: {1}", f.name, f.kind.mime()), &f.bytes);
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}

// What the server answers.

/// Why a note was not shared (or a link not listed or deleted), and what
/// to tell the user.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// Nothing written yet.
    Empty,
    /// No answer: offline, or the server is out of reach.
    Offline,
    /// The server does not take this install's key.
    Key,
    /// As many links are live as an install may have.
    Limit { live: usize },
    /// The note, with its pictures, is over the size a shared note may be.
    TooBig { bytes: usize, limit: usize, pictures: bool },
    /// A picture that could not be read (or that the server took for none).
    Unreadable(String),
    /// Too many links made at once: try again in `retry` seconds.
    Rate { retry: u64 },
    /// notsy.co is near its free-tier budget and makes no new links until
    /// the next UTC day (or month): `retry` seconds.
    Quota { retry: u64 },
    /// The server did not take the request (the app's fault, not the user's).
    Bad,
    /// The server failed.
    Server,
}

/// Megabytes, at most one decimal: `1 MB`, `1.4 MB`; rounded up, so a note
/// just over says more than the limit.
fn megabytes(bytes: usize, up: bool) -> String {
    let tenths = if up { bytes.div_ceil(100_000) } else { bytes / 100_000 };
    if tenths % 10 == 0 { format!("{} MB", tenths / 10) } else { format!("{}.{} MB", tenths / 10, tenths % 10) }
}

impl Problem {
    pub fn message(&self) -> String {
        match self {
            Problem::Empty => "Write something first, then share it.".into(),
            Problem::Offline => "Couldn't reach notsy.co. Check the connection and try again.".into(),
            Problem::Key => "notsy.co didn't take this app's key. Try again.".into(),
            Problem::Limit { live } => format!("{live} links are live. Delete one to share another."),
            Problem::TooBig { bytes, limit, pictures: true } => {
                format!("This note is {} with its pictures. Shared notes can be {}. Remove a picture or two.", megabytes(*bytes, true), megabytes(*limit, false))
            }
            Problem::TooBig { bytes, limit, pictures: false } => format!("This note is {}. Shared notes can be {}.", megabytes(*bytes, true), megabytes(*limit, false)),
            Problem::Unreadable(name) => format!("Couldn't read '{name}' to share it."),
            Problem::Rate { retry } => format!("Too many links at once. Try again in {}.", wait_words(*retry)),
            Problem::Quota { retry } if *retry > 86_400 => "Sharing is resting until next month.".into(),
            Problem::Quota { .. } => "Sharing is resting until tomorrow.".into(),
            Problem::Bad => "notsy.co couldn't take this note.".into(),
            Problem::Server => "notsy.co had a problem. Try again in a moment.".into(),
        }
    }
}

/// How long to wait, in words: `a minute`, `20 minutes`, `an hour`, `3 hours`.
fn wait_words(secs: u64) -> String {
    match secs {
        0..=90 => "a minute".into(),
        91..=3_599 => format!("{} minutes", secs.div_ceil(60)),
        3_600..=5_399 => "an hour".into(),
        _ => format!("{} hours", secs.div_ceil(3_600)),
    }
}

/// A value of a `key=value` body.
pub fn field<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    body.lines().find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim()))
}

fn number(body: &str, key: &str) -> Option<u64> {
    field(body, key)?.parse().ok()
}

/// What an answer that is not a success means. `sent`: the bytes the note
/// came to (for a 413); `bundle`, to name a picture by the note's name.
fn problem(status: u16, body: &str, sent: usize, bundle: Option<&Bundle>) -> Problem {
    match (status, field(body, "error")) {
        (401, _) | (_, Some("key")) => Problem::Key,
        (409, _) | (_, Some("limit")) => Problem::Limit { live: number(body, "live").map_or(MOST_LIVE, |n| n as usize) },
        (413, _) | (_, Some("size")) => Problem::TooBig { bytes: sent, limit: number(body, "limit").map_or(LIMIT, |n| n as usize), pictures: bundle.is_some_and(|b| !b.files.is_empty()) },
        (415, _) | (_, Some("picture")) => {
            let name = field(body, "name").unwrap_or("");
            Problem::Unreadable(bundle.and_then(|b| b.original(name)).unwrap_or(name).to_string())
        }
        (429, _) | (_, Some("rate")) => Problem::Rate { retry: number(body, "retry").unwrap_or(60) },
        (_, Some("quota")) => Problem::Quota { retry: number(body, "retry").unwrap_or(86_400) },
        (400..=499, _) => Problem::Bad,
        _ => Problem::Server,
    }
}

/// A link made.
#[derive(Clone, Debug, PartialEq)]
pub struct Created {
    pub id: String,
    pub url: String,
    /// When it goes, Unix seconds.
    pub expires: u64,
}

/// What `POST /api/links` answered (`None`: no answer).
pub fn created(answer: Option<(u16, Vec<u8>)>, bundle: &Bundle) -> Result<Created, Problem> {
    let (status, body) = answer.ok_or(Problem::Offline)?;
    let body = String::from_utf8_lossy(&body);
    if status != 201 && status != 200 {
        return Err(problem(status, &body, bundle.size(), Some(bundle)));
    }
    let id = field(&body, "id").filter(|id| is_id(id)).ok_or(Problem::Server)?.to_string();
    let url = field(&body, "url").filter(|u| u.starts_with("https://")).map_or_else(|| link_url(&id), str::to_string);
    let expires = number(&body, "expires").ok_or(Problem::Server)?;
    Ok(Created { id, url, expires })
}

/// A link's id as the server makes them: a few URL-safe characters (its
/// alphabet is the server's to choose; nothing here depends on it).
fn is_id(id: &str) -> bool {
    (1..=16).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A live link of this install's, as listed (by the server, or kept here).
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub id: String,
    /// Made and going, Unix seconds.
    pub created: u64,
    pub expires: u64,
    /// The note's title as shared.
    pub title: String,
    /// The note it was made from (vault-relative, with `.md`), when made
    /// here; empty when only the server knows it.
    pub note: String,
}

impl Link {
    pub fn url(&self) -> String {
        link_url(&self.id)
    }
}

/// A link's address.
pub fn link_url(id: &str) -> String {
    format!("{BASE}/{id}")
}

/// An address as shown: without its scheme (`notsy.co/Y5tq`).
pub fn short(url: &str) -> &str {
    url.split_once("://").map_or(url, |(_, rest)| rest)
}

/// What `GET /api/links` answered: the install's live links, newest first.
pub fn listed(answer: Option<(u16, Vec<u8>)>) -> Result<Vec<Link>, Problem> {
    let (status, body) = answer.ok_or(Problem::Offline)?;
    let body = String::from_utf8_lossy(&body);
    if status != 200 {
        return Err(problem(status, &body, 0, None));
    }
    Ok(body
        .lines()
        .filter_map(|l| {
            let mut f = l.splitn(4, '\t');
            let id = f.next()?.trim();
            let created = f.next()?.trim().parse().ok()?;
            let expires = f.next()?.trim().parse().ok()?;
            let title = f.next().unwrap_or("").trim().to_string();
            is_id(id).then(|| Link { id: id.to_string(), created, expires, title, note: String::new() })
        })
        .collect())
}

/// What `DELETE /api/links/{id}` answered: gone (a link already gone is
/// gone too), or why not.
pub fn deleted(answer: Option<(u16, Vec<u8>)>) -> Result<(), Problem> {
    let (status, body) = answer.ok_or(Problem::Offline)?;
    match status {
        200 | 204 | 404 | 410 => Ok(()),
        _ => Err(problem(status, &String::from_utf8_lossy(&body), 0, None)),
    }
}

// The links kept here, so the card shows them at once.

/// The file in the app's data folder that keeps this install's links.
pub fn links_path(data: &Path) -> PathBuf {
    data.join("share.links")
}

/// Links as kept: a line each, `id created expires note title`, by tabs.
pub fn links_text(links: &[Link]) -> String {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    links.iter().map(|l| format!("{}\t{}\t{}\t{}\t{}\n", l.id, l.created, l.expires, clean(&l.note), clean(&l.title))).collect()
}

pub fn read_links(text: &str) -> Vec<Link> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.splitn(5, '\t');
            let id = f.next()?;
            let created = f.next()?.parse().ok()?;
            let expires = f.next()?.parse().ok()?;
            let note = f.next()?.to_string();
            let title = f.next().unwrap_or("").to_string();
            is_id(id).then(|| Link { id: id.to_string(), created, expires, title, note })
        })
        .collect()
}

/// The server's list, with the notes the links kept here were made from.
pub fn merge(kept: &[Link], from_server: Vec<Link>) -> Vec<Link> {
    from_server
        .into_iter()
        .map(|mut l| {
            if let Some(k) = kept.iter().find(|k| k.id == l.id) {
                l.note = k.note.clone();
                if l.title.is_empty() {
                    l.title = k.title.clone();
                }
            }
            l
        })
        .collect()
}

/// What is kept here between shares: the expiry chosen last (a first line
/// `expiry<TAB>7`), then the links.
pub fn kept_text(expiry: Expiry, links: &[Link]) -> String {
    format!("expiry\t{}\n{}", expiry.days(), links_text(links))
}

pub fn read_kept(text: &str) -> (Expiry, Vec<Link>) {
    let expiry = text.lines().next().and_then(|l| l.strip_prefix("expiry\t")).and_then(|d| d.trim().parse().ok()).and_then(Expiry::from_days).unwrap_or_default();
    (expiry, read_links(text))
}

/// Those still live at `now` (Unix seconds).
pub fn live(links: &[Link], now: u64) -> Vec<Link> {
    links.iter().filter(|l| l.expires > now).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG of a size in pixels, about `len` bytes long: its header, a
    /// frame, a scan of filler (with stuffed 0xFF bytes, as real ones have),
    /// and its end.
    fn jpeg(w: u32, h: u32, len: usize) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        b.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        b.extend_from_slice(&[0xFF, 0xDB, 0, 5, 0, 1, 2]);
        b.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8]);
        b.extend_from_slice(&(h as u16).to_be_bytes());
        b.extend_from_slice(&(w as u16).to_be_bytes());
        b.extend_from_slice(&[1, 1, 0x11, 0]);
        b.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 63, 0]);
        // Its first few thousand bytes varied, then quiet (and quick).
        let (mut k, end) = (0u32, len.max(b.len() + 8));
        while b.len() + 2 < end.min(4096) {
            k = k.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let byte = (k >> 16) as u8;
            b.push(byte);
            if byte == 0xFF {
                b.push(0);
            }
        }
        b.resize(b.len().max(end - 2), 0x55);
        b.extend_from_slice(&[0xFF, 0xD9]);
        b
    }

    /// `jpeg` with an EXIF segment saying where it was taken (GPS) and how
    /// it is turned, and a colour profile.
    fn photo(w: u32, h: u32, len: usize, turned: u16) -> Vec<u8> {
        let plain = jpeg(w, h, len);
        let mut tiff = b"MM\0\x2a\0\0\0\x08\0\x02".to_vec();
        tiff.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1]);
        tiff.extend_from_slice(&turned.to_be_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&[0x88, 0x25, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0x26]);
        tiff.extend_from_slice(b"\0\0\0\0GPS 51.5007N 0.1246W");
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend_from_slice(&tiff);
        let mut b = plain[..20].to_vec();
        b.extend_from_slice(&[0xFF, 0xE1]);
        b.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        b.extend_from_slice(&exif);
        let icc = b"ICC_PROFILE\0\x01\x01profile";
        b.extend_from_slice(&[0xFF, 0xE2]);
        b.extend_from_slice(&((icc.len() + 2) as u16).to_be_bytes());
        b.extend_from_slice(icc);
        let xmp = b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>Lat 51.5</x:xmpmeta>";
        b.extend_from_slice(&[0xFF, 0xE1]);
        b.extend_from_slice(&((xmp.len() + 2) as u16).to_be_bytes());
        b.extend_from_slice(xmp);
        b.extend_from_slice(&plain[20..]);
        b
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    /// An imaging stand-in: reads sizes from headers, "encodes" a JPEG
    /// whose weight follows its pixels and quality, and counts its work.
    struct Fake {
        /// Bytes per pixel at quality 100.
        density: f32,
        alpha: bool,
        decoded: std::cell::Cell<usize>,
    }

    impl Fake {
        fn new(density: f32) -> Fake {
            Fake { density, alpha: false, decoded: Default::default() }
        }
    }

    impl Recode for Fake {
        fn decode(&self, bytes: &[u8], edge: u32) -> Option<Raster> {
            let (w, h) = dimensions(bytes)?;
            self.decoded.set(self.decoded.get() + 1);
            let s = (edge as f32 / w.max(h) as f32).min(1.0);
            let (w, h) = (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1));
            let a = if self.alpha { 128 } else { 255 };
            // Large ones carry one pixel, standing for all of them: only
            // their size is weighed, and the tests stay quick.
            let n = if w * h <= 10_000 { w * h } else { 1 };
            Some(Raster { width: w, height: h, bgra: [0, 0, 0, a].repeat(n as usize) })
        }

        fn encode(&self, r: &Raster, quality: u8, png: bool) -> Option<Vec<u8>> {
            let len = (r.width * r.height) as f32 * self.density * quality as f32 / 100.0;
            let mut out = jpeg(r.width, r.height, len as usize);
            if png {
                out = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
                out.extend_from_slice(&r.width.to_be_bytes());
                out.extend_from_slice(&r.height.to_be_bytes());
                out.resize(len as usize + 24, 0);
            }
            Some(out)
        }
    }

    #[test]
    fn keys_are_made_once_and_kept() {
        assert_eq!(key_text(&[0; 32]), "A".repeat(43));
        let k = key_text(&[0xFB; 32]);
        assert_eq!(k.len(), 43);
        assert!(k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "url-safe, no padding");
        assert_eq!(base64url(b"Man"), "TWFu");
        assert_eq!(base64url(b"Ma"), "TWE");
        assert_eq!(base64url(&[0xFF, 0xEF]), "_-8");
        assert_eq!(read_key(&format!("{k}\n")), Some(k.clone()));
        assert_eq!(read_key("short"), None);
        let dir = std::env::temp_dir().join(format!("kolada-share-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = load_or_make_key(&dir, || Some([7; 32])).unwrap();
        let again = load_or_make_key(&dir, || panic!("made again")).unwrap();
        assert_eq!(first, again);
        assert!(load_or_make_key(&dir.join("none"), || None).is_none(), "no random bytes, no key");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exif_and_gps_are_stripped_without_recompressing() {
        let b = photo(1200, 800, 40_000, 1);
        assert!(contains(&b, b"GPS") && contains(&b, b"xmpmeta"));
        let s = strip_jpeg(&b).unwrap();
        assert!(!contains(&s, b"Exif") && !contains(&s, b"GPS") && !contains(&s, b"xmpmeta"), "EXIF, GPS and XMP gone");
        assert!(contains(&s, b"ICC_PROFILE") && contains(&s, b"JFIF"), "the colour profile and JFIF stay");
        // The coded picture is the same bytes: from the tables to its end.
        let from = |x: &[u8]| x.windows(2).position(|w| w == [0xFF, 0xDB]).unwrap();
        assert_eq!(&s[from(&s)..], &b[from(&b)..], "not recompressed");
        assert_eq!(dimensions(&s), Some((1200, 800)));
        // What a phone tucks in after the picture's end goes.
        let mut tail = b.clone();
        tail.extend_from_slice(b"\xFF\xD8\xFF\xE1\0\x10Exif\0\0GPS hidden");
        assert_eq!(strip_jpeg(&tail).unwrap(), s);

        // A WebP: its EXIF and XMP chunks go, its flags and size follow.
        let mut w = b"RIFF\0\0\0\0WEBP".to_vec();
        let chunk = |w: &mut Vec<u8>, id: &[u8; 4], data: &[u8]| {
            w.extend_from_slice(id);
            w.extend_from_slice(&(data.len() as u32).to_le_bytes());
            w.extend_from_slice(data);
            if data.len() % 2 == 1 {
                w.push(0);
            }
        };
        chunk(&mut w, b"VP8X", &[0x10 | 0x08 | 0x04, 0, 0, 0, 99, 0, 0, 49, 0, 0]);
        chunk(&mut w, b"VP8L", b"\x2fpixels, as coded");
        chunk(&mut w, b"EXIF", b"MM\0\x2a\0\0\0\x08GPS 51.5N");
        chunk(&mut w, b"XMP ", b"<x:xmpmeta/>");
        let size = (w.len() - 8) as u32;
        w[4..8].copy_from_slice(&size.to_le_bytes());
        let s = strip_webp(&w).unwrap();
        assert!(!contains(&s, b"EXIF") && !contains(&s, b"GPS") && !contains(&s, b"xmpmeta"));
        assert!(contains(&s, b"\x2fpixels, as coded"), "the picture's own chunk as it was");
        assert_eq!(s[20], 0x10, "only the alpha flag left");
        assert_eq!(u32::from_le_bytes(s[4..8].try_into().unwrap()) as usize, s.len() - 8);
        assert_eq!(dimensions(&s), Some((100, 50)));

        // A PNG: its text and EXIF chunks go.
        let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
        let png_chunk = |p: &mut Vec<u8>, id: &[u8; 4], data: &[u8]| {
            p.extend_from_slice(&(data.len() as u32).to_be_bytes());
            p.extend_from_slice(id);
            p.extend_from_slice(data);
            p.extend_from_slice(&[1, 2, 3, 4]);
        };
        png_chunk(&mut p, b"IHDR", &[0, 0, 0, 9, 0, 0, 0, 9, 8, 2, 0, 0, 0]);
        png_chunk(&mut p, b"tEXt", b"Comment\0taken at home");
        png_chunk(&mut p, b"eXIf", b"MM\0\x2a\0\0\0\x08GPS");
        png_chunk(&mut p, b"IDAT", b"pixels");
        png_chunk(&mut p, b"IEND", b"");
        let s = strip_png(&p).unwrap();
        assert!(!contains(&s, b"home") && !contains(&s, b"GPS") && contains(&s, b"IDATpixels") && s.ends_with(b"IEND\x01\x02\x03\x04"));
    }

    #[test]
    fn small_jpegs_are_sent_as_they_are() {
        let fake = Fake::new(0.5);
        let b = photo(1600, 1000, 290_000, 1);
        let p = prepare(&b, 0, &fake).unwrap();
        assert_eq!(fake.decoded.get(), 0, "not decoded");
        assert_eq!(p.kind, Kind::Jpeg);
        assert_eq!(p.bytes, strip_jpeg(&b).unwrap());
        // Larger than that, wider than that, or turned on its side: made again.
        for (other, why) in [(photo(1600, 1000, 320_000, 1), "heavy"), (photo(2000, 1000, 100_000, 1), "wide"), (photo(80, 60, 5_000, 6), "turned")] {
            let p = prepare(&other, 0, &fake).unwrap();
            let (w, h) = dimensions(&p.bytes).unwrap();
            assert!(w.max(h) <= 1600, "{why}");
            assert!(!contains(&p.bytes, b"GPS"), "{why}");
            if why == "turned" {
                assert_eq!((w, h), (60, 80), "upright");
            }
        }
        // A see-through picture stays a PNG.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&3000u32.to_be_bytes());
        png.extend_from_slice(&2000u32.to_be_bytes());
        let p = prepare(&png, 0, &Fake { alpha: true, ..Fake::new(0.1) }).unwrap();
        assert_eq!(p.kind, Kind::Png);
        assert_eq!(dimensions(&p.bytes), Some((1600, 1067)));
    }

    #[test]
    fn photos_are_turned_upright() {
        // 2 x 3: pixels numbered by their place.
        let r = Raster { width: 2, height: 3, bgra: (0..6).flat_map(|i| [i, 0, 0, 255]).collect() };
        let ids = |r: &Raster| r.bgra.chunks(4).map(|p| p[0]).collect::<Vec<_>>();
        let turned = r.clone().oriented(6);
        assert_eq!((turned.width, turned.height), (3, 2));
        // 0 1      4 2 0
        // 2 3  ->  5 3 1
        // 4 5
        assert_eq!(ids(&turned), [4, 2, 0, 5, 3, 1]);
        assert_eq!(ids(&r.clone().oriented(3)), [5, 4, 3, 2, 1, 0]);
        assert_eq!(ids(&r.clone().oriented(8)), [1, 3, 5, 0, 2, 4]);
        assert_eq!(r.clone().oriented(1), r);
        assert_eq!(orientation(&photo(10, 10, 100, 6)), 6);
        assert_eq!(orientation(&jpeg(10, 10, 100)), 1);
    }

    #[test]
    fn embeds_are_rewritten_to_uploaded_names() {
        let text = "# Trip\n\n![[beach.png|300]]\n\nAt ![the pier|240](photos/pier.jpg) and ![[beach.png]] again.\n\n\
                    ![web](https://example.com/a.png) ![[Other note]] `![[code.png]]`\n\n```\n![[fenced.png]]\n```\n![[My Photo.heic|a caption]]\n";
        let mut asked = Vec::new();
        let mut read = |name: &str| {
            asked.push(name.to_string());
            Some(jpeg(100, 100, 2_000))
        };
        let b = bundle(text, &mut read, &Fake::new(0.5), LIMIT).unwrap();
        assert_eq!(asked, ["beach.png", "photos/pier.jpg", "My Photo.heic"], "each picture read once; the web's, code and notes left");
        assert_eq!(
            b.note,
            "# Trip\n\n![](0.jpg)\n\nAt ![the pier](1.jpg) and ![](0.jpg) again.\n\n\
             ![web](https://example.com/a.png) ![[Other note]] `![[code.png]]`\n\n```\n![[fenced.png]]\n```\n![a caption](2.jpg)\n"
        );
        assert_eq!(b.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["0.jpg", "1.jpg", "2.jpg"]);
        assert_eq!(b.original("1.jpg"), Some("pier.jpg"));
        // A picture that cannot be read is named, not left out.
        let e = bundle("![[photo.heic]]", &mut |_| None, &Fake::new(0.5), LIMIT).unwrap_err();
        assert_eq!(e.message(), "Couldn't read 'photo.heic' to share it.");
        // Nor one that is not a picture.
        let e = bundle("![[photo.png]]", &mut |_| Some(b"<html>".to_vec()), &Fake::new(0.5), LIMIT).unwrap_err();
        assert_eq!(e, Problem::Unreadable("photo.png".into()));
        assert_eq!(bundle(" \n", &mut |_| None, &Fake::new(0.5), LIMIT), Err(Problem::Empty));
        // No pictures: the note as it is.
        assert_eq!(bundle("Just words", &mut |_| None, &Fake::new(0.5), LIMIT).unwrap(), Bundle { note: "Just words".into(), files: vec![] });
    }

    #[test]
    fn oversized_notes_step_down_then_explain() {
        // Four large photos: at 1600 px and q82 they come to more than 1 MB.
        let text = "![[a.jpg]]\n![[b.jpg]]\n![[c.jpg]]\n![[d.jpg]]\n";
        let fake = Fake::new(0.2);
        let big = photo(4000, 3000, 3_000_000, 1);
        let b = bundle(text, &mut |_| Some(big.clone()), &fake, LIMIT).unwrap();
        let (w, h) = dimensions(&b.files[0].bytes).unwrap();
        assert!(w.max(h) < 1600, "stepped down to {w}x{h}");
        assert!(b.size() <= LIMIT);
        // Too much even at the smallest: said plainly, with both sizes.
        let many: String = (0..12).map(|i| format!("![[p{i}.jpg]]\n")).collect();
        let e = bundle(&many, &mut |_| Some(big.clone()), &Fake::new(0.4), LIMIT).unwrap_err();
        let Problem::TooBig { bytes, limit, pictures } = e.clone() else { panic!("{e:?}") };
        assert!(bytes > limit && pictures && limit == LIMIT);
        assert!(e.message().starts_with("This note is ") && e.message().ends_with(" with its pictures. Shared notes can be 1 MB. Remove a picture or two."), "{}", e.message());
        assert_eq!(Problem::TooBig { bytes: 1_400_000, limit: 1_000_000, pictures: true }.message(), "This note is 1.4 MB with its pictures. Shared notes can be 1 MB. Remove a picture or two.");
        assert_eq!(Problem::TooBig { bytes: 1_000_001, limit: 1_000_000, pictures: false }.message(), "This note is 1.1 MB. Shared notes can be 1 MB.");
        // Text alone over: no picture is decoded to find out.
        let fake = Fake::new(0.2);
        let long = "x".repeat(LIMIT + 1) + "![[a.jpg]]";
        assert!(matches!(bundle(&long, &mut |_| Some(big.clone()), &fake, LIMIT), Err(Problem::TooBig { .. })));
        assert_eq!(fake.decoded.get(), 0);
    }

    #[test]
    fn the_bundle_never_exceeds_the_limit() {
        let photos = [(800, 90_000), (1600, 290_000), (3000, 900_000), (6000, 4_000_000)].map(|(w, len)| (w, photo(w, w * 3 / 4, len, 1)));
        for count in 0..8 {
            for (w, p) in &photos {
                for density in [0.05, 0.2, 0.6] {
                    for limit in [200_000, LIMIT] {
                        let text: String = std::iter::once("# Trip\n".to_string()).chain((0..count).map(|i| format!("Line {i}\n![[p{i}.jpg]]\n"))).collect();
                        match bundle(&text, &mut |_| Some(p.clone()), &Fake::new(density), limit) {
                            Ok(b) => assert!(b.size() <= limit, "{count} x {w}px, {density}: {}", b.size()),
                            Err(Problem::TooBig { bytes, .. }) => assert!(bytes > limit),
                            Err(e) => panic!("{e:?}"),
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn multipart_body_is_well_formed() {
        let b = Bundle {
            note: "# Hi\n\n![](0.jpg)\n".into(),
            files: vec![Upload { name: "0.jpg".into(), from: "a.jpg".into(), kind: Kind::Jpeg, bytes: vec![0xFF, 0xD8, 0xFF, 0xD9] }],
        };
        let boundary = boundary(&b, 42);
        assert_eq!(content_type(&boundary), format!("multipart/form-data; boundary={boundary}"));
        let body = multipart(&boundary, Expiry::Week, &b);
        let mut want = Vec::new();
        want.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"expires\"\r\n\r\n7\r\n").as_bytes());
        want.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"note\"\r\nContent-Type: text/markdown; charset=utf-8\r\n\r\n# Hi\n\n![](0.jpg)\n\r\n").as_bytes());
        want.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"0.jpg\"; filename=\"0.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n").as_bytes());
        want.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xD9]);
        want.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        assert_eq!(body, want);
        // A boundary is never in what it separates.
        let tricky = Bundle { note: format!("--{} {}", super::boundary(&b, 42), super::boundary(&b, 42)), files: vec![] };
        assert!(!tricky.note.contains(&super::boundary(&tricky, 42)));
        assert_eq!(Expiry::from_days(30), Some(Expiry::Month));
        assert_eq!(Expiry::default(), Expiry::Week);
    }

    #[test]
    fn responses_parse_without_json() {
        let b = Bundle { note: "x".repeat(10), files: vec![Upload { name: "0.jpg".into(), from: "pics/photo.png".into(), kind: Kind::Jpeg, bytes: vec![0; 1_399_990] }] };
        let ok = created(Some((201, b"id=Y5tq\nurl=https://notsy.co/Y5tq\nexpires=1792000000\n".to_vec())), &b).unwrap();
        assert_eq!(ok, Created { id: "Y5tq".into(), url: "https://notsy.co/Y5tq".into(), expires: 1_792_000_000 });
        assert_eq!(short(&ok.url), "notsy.co/Y5tq");
        let err = |status: u16, body: &str| created(Some((status, body.as_bytes().to_vec())), &b).unwrap_err();
        assert_eq!(err(409, "error=limit\nlive=3\n").message(), "3 links are live. Delete one to share another.");
        assert_eq!(err(413, "error=size\nlimit=1000000\n").message(), "This note is 1.4 MB with its pictures. Shared notes can be 1 MB. Remove a picture or two.");
        assert_eq!(err(415, "error=picture\nname=0.jpg\n").message(), "Couldn't read 'photo.png' to share it.");
        assert_eq!(err(429, "error=rate\nretry=60\n"), Problem::Rate { retry: 60 });
        assert_eq!(err(429, "error=rate\nretry=60\n").message(), "Too many links at once. Try again in a minute.");
        assert_eq!(err(429, "error=rate\nretry=3600\n").message(), "Too many links at once. Try again in an hour.");
        assert_eq!(err(429, "error=rate\nretry=1200\n").message(), "Too many links at once. Try again in 20 minutes.");
        assert_eq!(err(401, "error=key\n"), Problem::Key);
        assert_eq!(err(503, "error=quota\nretry=3600\n"), Problem::Quota { retry: 3600 });
        assert_eq!(err(503, "error=quota\nretry=3600\n").message(), "Sharing is resting until tomorrow.");
        assert_eq!(err(503, "error=quota\nretry=86400\n").message(), "Sharing is resting until tomorrow.");
        assert_eq!(err(503, "error=quota\nretry=1496934\n").message(), "Sharing is resting until next month.");
        assert_eq!(err(503, "error=quota\n").message(), "Sharing is resting until tomorrow.");
        assert_eq!(listed(Some((503, b"error=quota\nretry=600\n".to_vec()))).unwrap_err(), Problem::Quota { retry: 600 });
        assert_eq!(err(400, "error=bad\nmessage=no note\n"), Problem::Bad);
        assert_eq!(err(503, "<html>"), Problem::Server);
        assert_eq!(err(201, "id=\n"), Problem::Server, "a success without an id is not one");
        assert_eq!(created(None, &b).unwrap_err().message(), "Couldn't reach notsy.co. Check the connection and try again.");

        let list = listed(Some((200, b"Y5tq\t1791000000\t1791600000\tTrip to the coast\nAb3D\t1790000000\t1790086400\t\nnot a row\n".to_vec()))).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].id.as_str(), list[0].expires, list[0].title.as_str()), ("Y5tq", 1_791_600_000, "Trip to the coast"));
        assert_eq!(list[1].title, "");
        assert_eq!(listed(Some((200, Vec::new()))).unwrap(), vec![]);
        assert_eq!(listed(Some((401, b"error=key".to_vec()))).unwrap_err(), Problem::Key);

        assert_eq!(deleted(Some((204, Vec::new()))), Ok(()));
        assert_eq!(deleted(Some((404, b"error=missing\n".to_vec()))), Ok(()), "gone already is gone");
        assert_eq!(deleted(None), Err(Problem::Offline));

        // Kept here: the notes they came from survive the server's list.
        let kept = vec![Link { id: "Y5tq".into(), created: 1, expires: 1_791_600_000, title: "Trip".into(), note: "Trips/Coast.md".into() }];
        let text = links_text(&kept);
        assert_eq!(read_links(&text), kept);
        assert_eq!(read_kept(&kept_text(Expiry::Month, &kept)), (Expiry::Month, kept.clone()));
        assert_eq!(read_kept(""), (Expiry::Week, vec![]));
        let merged = merge(&kept, list);
        assert_eq!(merged[0].note, "Trips/Coast.md");
        assert_eq!(merged[0].title, "Trip to the coast", "the server's title");
        assert_eq!(live(&merged, 1_790_100_000).len(), 1);
    }
}
