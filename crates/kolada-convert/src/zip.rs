//! Reading files out of a zip archive, as a DOCX is one: its central
//! directory at the end lists every entry, and each entry is stored or
//! DEFLATE-compressed.

use crate::{Error, inflate};

/// One entry: its name, how it is stored, its sizes, where its local header
/// is.
struct Entry {
    name: String,
    method: u16,
    compressed: usize,
    size: usize,
    header: usize,
}

pub struct Zip<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn bad(what: &str) -> Error {
    Error(format!("not a readable zip archive: {what}"))
}

/// Entries larger than this are refused: a document's parts are far
/// smaller, and a crafted archive could claim gigabytes.
const MOST: usize = 512 * 1024 * 1024;

impl<'a> Zip<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Zip<'a>, Error> {
        if !bytes.starts_with(b"PK\x03\x04") {
            return Err(bad("no zip signature"));
        }
        // The end-of-directory record: its signature, searched from the end
        // (a comment of up to 64 KB may follow it).
        let from = bytes.len().saturating_sub(22 + 65535);
        let end = (from..bytes.len().saturating_sub(21)).rev().find(|&i| bytes[i..i + 4] == *b"PK\x05\x06").ok_or_else(|| bad("no directory"))?;
        let count = u16_at(bytes, end + 10).ok_or_else(|| bad("directory"))? as usize;
        let mut at = u32_at(bytes, end + 16).ok_or_else(|| bad("directory"))? as usize;
        let mut entries = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            if u32_at(bytes, at) != Some(0x0201_4b50) {
                return Err(bad("a directory entry"));
            }
            let field = |o: usize| u32_at(bytes, at + o).map(|v| v as usize).ok_or_else(|| bad("a directory entry"));
            let short = |o: usize| u16_at(bytes, at + o).map(|v| v as usize).ok_or_else(|| bad("a directory entry"));
            let method = short(10)? as u16;
            let (compressed, size) = (field(20)?, field(24)?);
            let (name_len, extra_len, comment_len) = (short(28)?, short(30)?, short(32)?);
            let header = field(42)?;
            let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(|| bad("a name"))?;
            entries.push(Entry { name: String::from_utf8_lossy(name).replace('\\', "/"), method, compressed, size, header });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Zip { bytes, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    /// The contents of the entry named `name` (case as written; a leading
    /// slash is ignored), if there is one.
    pub fn read(&self, name: &str) -> Option<Result<Vec<u8>, Error>> {
        let name = name.trim_start_matches('/');
        let entry = self.entries.iter().find(|e| e.name == name || e.name.eq_ignore_ascii_case(name))?;
        Some(self.contents(entry))
    }

    /// The contents of the `i`th entry, in the order `names` gives them.
    pub fn read_at(&self, i: usize) -> Option<Result<Vec<u8>, Error>> {
        self.entries.get(i).map(|e| self.contents(e))
    }

    fn contents(&self, e: &Entry) -> Result<Vec<u8>, Error> {
        if e.size > MOST || e.compressed > MOST {
            return Err(bad("an entry too large"));
        }
        let b = self.bytes;
        if u32_at(b, e.header) != Some(0x0403_4b50) {
            return Err(bad("an entry's header"));
        }
        let name_len = u16_at(b, e.header + 26).ok_or_else(|| bad("an entry's header"))? as usize;
        let extra_len = u16_at(b, e.header + 28).ok_or_else(|| bad("an entry's header"))? as usize;
        let start = e.header + 30 + name_len + extra_len;
        let data = b.get(start..start + e.compressed).ok_or_else(|| bad("an entry cut short"))?;
        match e.method {
            0 => Ok(data.to_vec()),
            8 => {
                let mut out = Vec::with_capacity(e.size.min(MOST));
                inflate::inflate_into(data, &mut out)?;
                Ok(out)
            }
            m => Err(Error(format!("a zip entry compressed in a way not read here ({m})"))),
        }
    }
}
