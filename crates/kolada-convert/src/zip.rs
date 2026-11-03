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
    /// Bytes the archive may still inflate to, all its entries together
    /// (see `budget`), and the entries of archives inside it.
    left: std::rc::Rc<std::cell::Cell<usize>>,
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

/// What an archive of `size` bytes may inflate to in all: photos hardly
/// compress, text a few times over, so a real export stays well inside;
/// one built to fill memory does not.
fn budget(size: usize) -> usize {
    // 4 GB, or all a 32-bit address space can hold.
    let most = (4u64 << 30).min(usize::MAX as u64) as usize;
    size.saturating_mul(32).clamp(256 << 20, most)
}

impl<'a> Zip<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Zip<'a>, Error> {
        Zip::read_directory(bytes, std::rc::Rc::new(std::cell::Cell::new(budget(bytes.len()))))
    }

    /// An archive read out of `outer` (a Notion export in parts): it
    /// inflates within what is left of `outer`'s budget, not a new one.
    /// What reading the part out of `outer` cost is given back, so its
    /// contents are not paid for twice.
    pub fn open_within(bytes: &'a [u8], outer: &Zip) -> Result<Zip<'a>, Error> {
        let zip = Zip::read_directory(bytes, outer.left.clone())?;
        outer.left.set(outer.left.get().saturating_add(bytes.len()));
        Ok(zip)
    }

    fn read_directory(bytes: &'a [u8], left: std::rc::Rc<std::cell::Cell<usize>>) -> Result<Zip<'a>, Error> {
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
        Ok(Zip { bytes, entries, left })
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
        // An entry gives no more than it says it holds, and the archive no
        // more than its budget.
        let left = self.left.get();
        if e.size > left {
            return Err(bad("more inside than an export holds"));
        }
        let out = match e.method {
            0 if data.len() <= e.size => data.to_vec(),
            0 => return Err(bad("an entry larger than it says")),
            8 => {
                let mut out = Vec::with_capacity(e.size.min(1 << 20));
                inflate::inflate_at_most(data, &mut out, e.size).map_err(|_| bad("an entry larger than it says, or damaged"))?;
                out
            }
            m => return Err(Error(format!("a zip entry compressed in a way not read here ({m})"))),
        };
        self.left.set(left - out.len());
        Ok(out)
    }
}

/// A zip of `files`, stored (not compressed), names marked UTF-8: for tests
/// that build their exports.
#[cfg(test)]
pub(crate) fn stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    let entries: Vec<(&str, u16, &[u8], u32)> = files.iter().map(|&(n, d)| (n, 0, d, d.len() as u32)).collect();
    build(&entries)
}

/// A zip of entries as given: name, method (0 stored, 8 deflated), the
/// bytes as stored, and the size each says it holds.
#[cfg(test)]
pub(crate) fn build(files: &[(&str, u16, &[u8], u32)]) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
            }
        }
        !c
    }
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for &(name, method, data, size) in files {
        let at = out.len() as u32;
        let head = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend_from_slice(&sig.to_le_bytes());
            if central {
                h.extend_from_slice(&20u16.to_le_bytes());
            }
            h.extend_from_slice(&20u16.to_le_bytes());
            h.extend_from_slice(&0x0800u16.to_le_bytes());
            h.extend_from_slice(&method.to_le_bytes());
            h.extend_from_slice(&[0; 4]);
            h.extend_from_slice(&crc(data).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&size.to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            if central {
                // comment length, disk, internal and external attributes
                h.extend_from_slice(&[0; 10]);
                h.extend_from_slice(&at.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(head(0x0403_4b50, false));
        out.extend_from_slice(data);
        central.extend(head(0x0201_4b50, true));
    }
    let (at, len) = (out.len() as u32, central.len() as u32);
    out.extend(central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&at.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DEFLATE data that inflates to `n` bytes of 'a' (a little more, to a
    /// whole 258), in a few bytes per 258: one literal, then copies of the
    /// byte before, in fixed Huffman codes.
    pub(crate) fn bomb(n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut acc, mut have) = (0u32, 0u32);
        let mut put = |value: u32, bits: u32, out: &mut Vec<u8>| {
            acc |= value << have;
            have += bits;
            while have >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                have -= 8;
            }
        };
        // Huffman codes go out from their top bit.
        let code = |c: u32, bits: u32, put: &mut dyn FnMut(u32, u32, &mut Vec<u8>), out: &mut Vec<u8>| {
            for k in (0..bits).rev() {
                put((c >> k) & 1, 1, out);
            }
        };
        put(1, 1, &mut out); // the last block
        put(1, 2, &mut out); // fixed codes
        code(0x30 + b'a' as u32, 8, &mut put, &mut out);
        for _ in 0..n / 258 {
            code(0xC5, 8, &mut put, &mut out); // length 258
            code(0, 5, &mut put, &mut out); // distance 1
        }
        code(0, 7, &mut put, &mut out); // the end
        put(0, 7, &mut out);
        out
    }

    #[test]
    fn an_entry_that_says_less_than_it_holds_is_refused() {
        let data = bomb(4 * 1024 * 1024);
        assert!(data.len() < 64 * 1024, "{} bytes", data.len());
        // Honest about its size: read whole.
        let honest = build(&[("a.txt", 8, &data, (4 * 1024 * 1024 / 258 * 258 + 1) as u32)]);
        let z = Zip::open(&honest).unwrap();
        assert_eq!(z.read("a.txt").unwrap().unwrap().len(), 4 * 1024 * 1024 / 258 * 258 + 1);
        // Saying it holds a kilobyte: refused at the kilobyte, not inflated.
        let lying = build(&[("a.txt", 8, &data, 1024)]);
        assert!(Zip::open(&lying).unwrap().read("a.txt").unwrap().is_err());
        let stored_lie = build(&[("b.txt", 0, b"more than five", 5)]);
        assert!(Zip::open(&stored_lie).unwrap().read("b.txt").unwrap().is_err());
    }

    #[test]
    fn a_part_inside_an_archive_is_not_paid_for_twice() {
        let data = bomb(1024 * 1024);
        let size = (1024 * 1024 / 258 * 258 + 1) as u32;
        let part = build(&[("page.txt", 8, data.as_slice(), size)]);
        let outer = build(&[("Export-1-Part-1.zip", 0, part.as_slice(), part.len() as u32)]);
        let z = Zip::open(&outer).unwrap();
        // Just enough for the part's file and what is inside it, once each
        // but for the part's own bytes, which it gives back.
        z.left.set(part.len() + size as usize - 1);
        let bytes = z.read("Export-1-Part-1.zip").unwrap().unwrap();
        let inner = Zip::open_within(&bytes, &z).unwrap();
        assert!(inner.read("page.txt").unwrap().is_ok(), "the part's contents fit what the export may inflate to");
        assert_eq!(z.left.get(), part.len() - 1);
    }

    #[test]
    fn an_archive_inflates_to_no_more_than_its_budget() {
        let data = bomb(1024 * 1024);
        let size = (1024 * 1024 / 258 * 258 + 1) as u32;
        let names: Vec<String> = (0..8).map(|i| format!("{i}.txt")).collect();
        let entries: Vec<(&str, u16, &[u8], u32)> = names.iter().map(|n| (n.as_str(), 8, data.as_slice(), size)).collect();
        let bytes = build(&entries);
        let z = Zip::open(&bytes).unwrap();
        // A budget of three entries' worth: the fourth is refused.
        z.left.set(3 * size as usize + 10);
        let read: Vec<bool> = names.iter().map(|n| z.read(n).unwrap().is_ok()).collect();
        assert_eq!(read, [true, true, true, false, false, false, false, false]);
        // Archives inside it share what is left of it.
        let inner = build(&[("one.txt", 8, data.as_slice(), size)]);
        let part = Zip::open_within(&inner, &z).unwrap();
        assert!(part.read("one.txt").unwrap().is_err(), "the outer budget is spent");
        let fresh = Zip::open(&bytes).unwrap();
        let part = Zip::open_within(&inner, &fresh).unwrap();
        assert!(part.read("one.txt").unwrap().is_ok());
        assert!(fresh.left.get() < budget(bytes.len()), "what the part took, the archive no longer has");
        // The real budget: generous for an export, bounded for a bomb.
        assert_eq!(budget(1024), 256 * 1024 * 1024);
        assert_eq!(budget(usize::MAX) as u64, (4u64 << 30).min(usize::MAX as u64));
    }
}
