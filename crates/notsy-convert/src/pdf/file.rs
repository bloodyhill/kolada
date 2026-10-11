//! A PDF file: where each object is (its cross-reference tables or
//! streams, or, when those are damaged, a scan of the whole file), objects
//! packed into object streams, and stream filters.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::crypt::Crypt;
use super::object::{Dict, Lexer, Obj};
use crate::{Error, inflate};

/// Where an object is: at an offset, or the `index`th in an object stream.
#[derive(Clone, Copy, Debug)]
enum Place {
    At(usize),
    Packed(u32, usize),
}

pub struct File<'a> {
    bytes: &'a [u8],
    places: HashMap<u32, Place>,
    pub trailer: Dict,
    cache: RefCell<HashMap<u32, Rc<Obj>>>,
    /// Object streams already unpacked: their objects, in order.
    packed: RefCell<HashMap<u32, Rc<Vec<Obj>>>>,
    /// Objects being resolved, against references that loop.
    busy: RefCell<Vec<u32>>,
    /// An encrypted file's decryption, and its /Encrypt object (never
    /// encrypted itself).
    crypt: Option<(Crypt, Option<u32>)>,
}

/// Decoded streams larger than this are refused (a crafted file could
/// expand to gigabytes).
const MOST: usize = 256 * 1024 * 1024;

impl<'a> File<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<File<'a>, Error> {
        let head = &bytes[..bytes.len().min(1024)];
        if !head.windows(5).any(|w| w == b"%PDF-") {
            return Err(Error("not a PDF".to_string()));
        }
        let mut file = File {
            bytes,
            places: HashMap::new(),
            trailer: Dict::default(),
            cache: RefCell::new(HashMap::new()),
            packed: RefCell::new(HashMap::new()),
            busy: RefCell::new(Vec::new()),
            crypt: None,
        };
        let read = file.read_xrefs();
        // A missing or damaged index: the file scanned for its objects.
        let scanned = read.is_err() || file.trailer.get("Root").is_none() || file.places.is_empty();
        if scanned {
            file.scan();
        }
        // Locked against copying or printing but opened without a
        // password: read as its viewer would.
        if let Some(e) = file.trailer.get("Encrypt").filter(|e| !matches!(e, Obj::Null)).cloned() {
            let num = match e {
                Obj::Ref(n, _) => Some(n),
                _ => None,
            };
            let dict = file.resolve(&e).and_then(|d| d.as_dict().cloned()).ok_or_else(|| Error("the PDF is encrypted".to_string()))?;
            let id = match file.trailer.get("ID").and_then(Obj::as_array).and_then(|a| a.first()) {
                Some(Obj::Str(s)) => s.clone(),
                _ => Vec::new(),
            };
            file.crypt = Some((Crypt::new(&dict, &id)?, num));
            // What was read before was read undecrypted.
            file.cache.borrow_mut().clear();
            file.packed.borrow_mut().clear();
            if scanned {
                file.scan();
            }
        }
        Ok(file)
    }

    fn read_xrefs(&mut self) -> Result<(), Error> {
        let tail = &self.bytes[self.bytes.len().saturating_sub(2048)..];
        let at = tail.windows(9).rposition(|w| w == b"startxref").ok_or_else(|| Error("no startxref".to_string()))?;
        let mut lex = Lexer::new(tail, at + 9);
        let mut next = lex.object().and_then(|o| o.as_i64()).map(|v| v as usize);
        let mut seen = Vec::new();
        while let Some(offset) = next.take() {
            if offset >= self.bytes.len() || seen.contains(&offset) || seen.len() > 64 {
                break;
            }
            seen.push(offset);
            let trailer = self.read_xref(offset)?;
            if self.trailer.0.is_empty() {
                self.trailer = trailer.clone();
            }
            // A hybrid file's stream of the newer objects.
            if let Some(stm) = trailer.get("XRefStm").and_then(Obj::as_i64) {
                let _ = self.read_xref(stm as usize);
            }
            next = trailer.get("Prev").and_then(Obj::as_i64).map(|v| v as usize);
        }
        Ok(())
    }

    /// Reads one cross-reference section (a table or a stream) at `offset`;
    /// entries already known (from newer sections) are kept. Its trailer.
    fn read_xref(&mut self, offset: usize) -> Result<Dict, Error> {
        let bad = || Error("a damaged cross-reference".to_string());
        let mut lex = Lexer::new(self.bytes, offset);
        if lex.peek_word() == b"xref" {
            lex.word();
            loop {
                let w = lex.peek_word();
                if w == b"trailer" {
                    lex.word();
                    return lex.object().and_then(|o| o.as_dict().cloned()).ok_or_else(bad);
                }
                let start = super::object::number(lex.word()).and_then(|o| o.as_i64()).ok_or_else(bad)?;
                let count = super::object::number(lex.word()).and_then(|o| o.as_i64()).ok_or_else(bad)?;
                for i in 0..count.clamp(0, 10_000_000) {
                    let off = lex.word();
                    let _generation = lex.word();
                    let kind = lex.word();
                    let num = (start + i) as u32;
                    if kind == b"n"
                        && let Some(off) = std::str::from_utf8(off).ok().and_then(|o| o.parse::<usize>().ok())
                    {
                        self.places.entry(num).or_insert(Place::At(off));
                    }
                    if kind.is_empty() {
                        return Err(bad());
                    }
                }
            }
        }
        // A cross-reference stream: `n g obj << /Type /XRef ... >> stream`.
        let (_, _, obj) = self.object_at(offset).ok_or_else(bad)?;
        let Obj::Stream(dict, _) = &obj else { return Err(bad()) };
        let data = self.decode(&obj).ok_or_else(bad)?;
        let w: Vec<usize> = dict.get("W").and_then(Obj::as_array).ok_or_else(bad)?.iter().map(|v| v.as_i64().unwrap_or(0).clamp(0, 8) as usize).collect();
        if w.len() < 3 {
            return Err(bad());
        }
        let size = dict.get("Size").and_then(Obj::as_i64).unwrap_or(0);
        let index: Vec<i64> = dict.get("Index").and_then(Obj::as_array).map(|a| a.iter().filter_map(Obj::as_i64).collect()).unwrap_or_else(|| vec![0, size]);
        let row = w[0] + w[1] + w[2];
        let field = |b: &[u8]| b.iter().fold(0u64, |v, &x| v << 8 | x as u64);
        let mut at = 0;
        for pair in index.chunks(2) {
            let (start, count) = (pair[0], pair.get(1).copied().unwrap_or(0));
            for i in 0..count.clamp(0, 10_000_000) {
                let Some(r) = data.get(at..at + row) else { break };
                at += row;
                let kind = if w[0] == 0 { 1 } else { field(&r[..w[0]]) };
                let a = field(&r[w[0]..w[0] + w[1]]);
                let b = field(&r[w[0] + w[1]..]);
                let num = (start + i) as u32;
                match kind {
                    1 => {
                        self.places.entry(num).or_insert(Place::At(a as usize));
                    }
                    2 => {
                        self.places.entry(num).or_insert(Place::Packed(a as u32, b as usize));
                    }
                    _ => {}
                }
            }
        }
        Ok(dict.clone())
    }

    /// Finds every `n g obj` in the file (the last of a number wins, as a
    /// file updated in place appends them) and a trailer with a /Root.
    fn scan(&mut self) {
        let b = self.bytes;
        self.places.clear();
        let mut i = 0;
        while let Some(k) = b[i..].windows(3).position(|w| w == b"obj").map(|k| i + k) {
            i = k + 3;
            if b.get(k + 3).is_some_and(|c| !super::object::is_white(*c) && !super::object::is_delim(*c)) {
                continue;
            }
            // Back over "gen" and "num".
            let mut j = k;
            let mut words = Vec::new();
            for _ in 0..2 {
                while j > 0 && super::object::is_white(b[j - 1]) {
                    j -= 1;
                }
                let end = j;
                while j > 0 && b[j - 1].is_ascii_digit() {
                    j -= 1;
                }
                if j == end {
                    break;
                }
                words.push(&b[j..end]);
            }
            if let [_, num] = words[..]
                && let Some(num) = std::str::from_utf8(num).ok().and_then(|n| n.parse::<u32>().ok())
            {
                self.places.insert(num, Place::At(j));
            }
        }
        // Objects inside object streams.
        let streams: Vec<u32> = self.places.keys().copied().collect();
        for num in streams {
            if let Some(Obj::Stream(d, _)) = self.get(num).as_deref()
                && d.get("Type").and_then(Obj::as_name) == Some("ObjStm")
                && let Some(objects) = self.unpack_numbers(num)
            {
                for (index, n) in objects.into_iter().enumerate() {
                    self.places.entry(n).or_insert(Place::Packed(num, index));
                }
            }
        }
        // The trailer: the last one with a /Root, else the catalog itself.
        let mut at = 0;
        while let Some(k) = b[at..].windows(7).position(|w| w == b"trailer").map(|k| at + k) {
            at = k + 7;
            if let Some(Obj::Dict(d)) = Lexer::new(b, at).object()
                && d.get("Root").is_some()
            {
                self.trailer = d;
            }
        }
        if self.trailer.get("Root").is_none() {
            let nums: Vec<u32> = self.places.keys().copied().collect();
            for n in nums {
                if self.get(n).as_deref().and_then(Obj::as_dict).and_then(|d| d.get("Type")).and_then(Obj::as_name) == Some("Catalog") {
                    self.trailer.0.push(("Root".to_string(), Obj::Ref(n, 0)));
                    break;
                }
            }
        }
        self.cache.borrow_mut().clear();
    }

    /// The object `n g obj ... endobj` at `offset`: its number, generation
    /// and value.
    fn object_at(&self, offset: usize) -> Option<(u32, u16, Obj)> {
        let mut lex = Lexer::new(self.bytes, offset);
        let num = super::object::number(lex.word())?.as_i64()? as u32;
        let generation = super::object::number(lex.word()).and_then(|g| g.as_i64()).unwrap_or(0) as u16;
        if lex.word() != b"obj" {
            return None;
        }
        let value = lex.object()?;
        // A stream follows its dictionary.
        if let Obj::Dict(d) = &value {
            let save = lex.at;
            if lex.word() == b"stream" {
                let mut start = lex.at;
                if self.bytes.get(start) == Some(&b'\r') {
                    start += 1;
                }
                if self.bytes.get(start) == Some(&b'\n') {
                    start += 1;
                }
                let declared = d.get("Length").and_then(|l| match l {
                    Obj::Ref(n, _) => self.get(*n).as_deref().and_then(Obj::as_i64),
                    o => o.as_i64(),
                });
                let fits = |len: usize| {
                    let end = start + len;
                    end <= self.bytes.len() && {
                        let mut l = Lexer::new(self.bytes, end);
                        l.peek_word().starts_with(b"endstream")
                    }
                };
                let len = match declared.map(|l| l.max(0) as usize) {
                    Some(len) if fits(len) => len,
                    // A wrong length: up to `endstream`.
                    _ => {
                        let end = self.bytes[start..].windows(9).position(|w| w == b"endstream").map_or(self.bytes.len(), |k| start + k);
                        let mut e = end;
                        while e > start && matches!(self.bytes[e - 1], b'\r' | b'\n') {
                            e -= 1;
                        }
                        e - start
                    }
                };
                let data = self.bytes[start..start + len].to_vec();
                return Some((num, generation, Obj::Stream(d.clone(), Rc::new(data))));
            }
            let _ = save;
        }
        Some((num, generation, value))
    }

    /// Object `num`, read once.
    pub fn get(&self, num: u32) -> Option<Rc<Obj>> {
        if let Some(o) = self.cache.borrow().get(&num) {
            return Some(o.clone());
        }
        if self.busy.borrow().contains(&num) || self.busy.borrow().len() > 32 {
            return None;
        }
        self.busy.borrow_mut().push(num);
        let found = match *self.places.get(&num)? {
            Place::At(offset) => self.object_at(offset).filter(|(n, ..)| *n == num).map(|(n, generation, o)| match &self.crypt {
                Some((crypt, encrypt)) if *encrypt != Some(n) => crypt.object(n, generation, o),
                _ => o,
            }),
            Place::Packed(stream, index) => self.unpack(stream).and_then(|objs| objs.get(index).cloned()),
        };
        self.busy.borrow_mut().retain(|&n| n != num);
        let found = Rc::new(found?);
        self.cache.borrow_mut().insert(num, found.clone());
        Some(found)
    }

    /// `o`, or what it refers to.
    pub fn resolve(&self, o: &Obj) -> Option<Rc<Obj>> {
        match o {
            Obj::Ref(n, _) => self.get(*n),
            other => Some(Rc::new(other.clone())),
        }
    }

    /// A dictionary entry, resolved.
    pub fn entry(&self, d: &Dict, key: &str) -> Option<Rc<Obj>> {
        self.resolve(d.get(key)?)
    }

    /// The objects in object stream `num`, in order.
    fn unpack(&self, num: u32) -> Option<Rc<Vec<Obj>>> {
        if let Some(p) = self.packed.borrow().get(&num) {
            return Some(p.clone());
        }
        let stream = self.get(num)?;
        let d = stream.as_dict()?;
        let n = d.get("N").and_then(Obj::as_i64)?.clamp(0, 1_000_000) as usize;
        let first = d.get("First").and_then(Obj::as_i64)?.max(0) as usize;
        let data = self.decode(&stream)?;
        let mut header = Lexer::new(&data, 0);
        let mut offsets = Vec::with_capacity(n);
        for _ in 0..n {
            let _num = header.object();
            offsets.push(header.object().and_then(|o| o.as_i64()).unwrap_or(0).max(0) as usize);
        }
        let objects: Vec<Obj> = offsets.iter().map(|&off| Lexer::new(&data, first + off).object().unwrap_or(Obj::Null)).collect();
        let objects = Rc::new(objects);
        self.packed.borrow_mut().insert(num, objects.clone());
        Some(objects)
    }

    /// The object numbers an object stream holds, in order.
    fn unpack_numbers(&self, num: u32) -> Option<Vec<u32>> {
        let stream = self.get(num)?;
        let n = stream.as_dict()?.get("N").and_then(Obj::as_i64)?.clamp(0, 1_000_000) as usize;
        let data = self.decode(&stream)?;
        let mut header = Lexer::new(&data, 0);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(header.object().and_then(|o| o.as_i64()).unwrap_or(0) as u32);
            header.object();
        }
        Some(out)
    }

    /// A stream's bytes with its filters undone. Images' own filters (DCT,
    /// JPX, CCITT, JBIG2) are not undone: there is no text in them.
    pub fn decode(&self, stream: &Obj) -> Option<Vec<u8>> {
        let Obj::Stream(d, raw) = stream else { return None };
        let filters: Vec<String> = match d.get("Filter").or(d.get("F")).and_then(|f| self.resolve(f)).as_deref() {
            Some(Obj::Name(n)) => vec![n.clone()],
            Some(Obj::Array(a)) => a.iter().filter_map(|f| f.as_name().map(str::to_string)).collect(),
            _ => Vec::new(),
        };
        let params: Vec<Option<Dict>> = match d.get("DecodeParms").or(d.get("DP")).and_then(|p| self.resolve(p)).as_deref() {
            Some(Obj::Dict(p)) => vec![Some(p.clone())],
            Some(Obj::Array(a)) => a.iter().map(|p| self.resolve(p).and_then(|p| p.as_dict().cloned())).collect(),
            _ => Vec::new(),
        };
        let mut data = raw.as_ref().clone();
        for (i, f) in filters.iter().enumerate() {
            data = match f.as_str() {
                "FlateDecode" | "Fl" => {
                    let mut out = Vec::new();
                    // A damaged stream still gives what came before the damage.
                    let _ = inflate::zlib_into(&data, &mut out);
                    if out.is_empty() && !data.is_empty() {
                        return None;
                    }
                    out
                }
                "LZWDecode" | "LZW" => {
                    let early = params.get(i).and_then(|p| p.as_ref()).and_then(|p| p.get("EarlyChange")).and_then(Obj::as_i64).unwrap_or(1);
                    lzw(&data, early != 0)
                }
                "ASCIIHexDecode" | "AHx" => ascii_hex(&data),
                "ASCII85Decode" | "A85" => ascii85(&data)?,
                "RunLengthDecode" | "RL" => run_length(&data),
                _ => return None,
            };
            if data.len() > MOST {
                return None;
            }
            if let Some(Some(p)) = params.get(i)
                && matches!(f.as_str(), "FlateDecode" | "Fl" | "LZWDecode" | "LZW")
            {
                data = predict(data, p)?;
            }
        }
        Some(data)
    }
}

/// Undoes a PNG (10 to 15) or TIFF (2) predictor.
fn predict(data: Vec<u8>, p: &Dict) -> Option<Vec<u8>> {
    let predictor = p.get("Predictor").and_then(Obj::as_i64).unwrap_or(1);
    if predictor < 2 {
        return Some(data);
    }
    let colors = p.get("Colors").and_then(Obj::as_i64).unwrap_or(1).clamp(1, 32) as usize;
    let bits = p.get("BitsPerComponent").and_then(Obj::as_i64).unwrap_or(8).clamp(1, 16) as usize;
    let columns = p.get("Columns").and_then(Obj::as_i64).unwrap_or(1).clamp(1, 1 << 20) as usize;
    let bpp = (colors * bits).div_ceil(8).max(1);
    let row = (colors * bits * columns).div_ceil(8);
    if predictor == 2 {
        // TIFF, 8 bits per component only (as images use it; not for text).
        if bits != 8 {
            return Some(data);
        }
        let mut out = data;
        for r in out.chunks_mut(row) {
            for i in bpp..r.len() {
                r[i] = r[i].wrapping_add(r[i - bpp]);
            }
        }
        return Some(out);
    }
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        if chunk.len() < 2 {
            break;
        }
        let kind = chunk[0];
        let mut cur = chunk[1..].to_vec();
        cur.resize(row, 0);
        for i in 0..row {
            let left = if i >= bpp { cur[i - bpp] } else { 0 };
            let up = prev[i];
            let corner = if i >= bpp { prev[i - bpp] } else { 0 };
            cur[i] = cur[i].wrapping_add(match kind {
                1 => left,
                2 => up,
                3 => ((left as u16 + up as u16) / 2) as u8,
                4 => {
                    let p = left as i16 + up as i16 - corner as i16;
                    let (pa, pb, pc) = ((p - left as i16).abs(), (p - up as i16).abs(), (p - corner as i16).abs());
                    if pa <= pb && pa <= pc {
                        left
                    } else if pb <= pc {
                        up
                    } else {
                        corner
                    }
                }
                _ => 0,
            });
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    Some(out)
}

fn ascii_hex(data: &[u8]) -> Vec<u8> {
    // The hex string reader, without its opening `<`.
    let mut out = Vec::new();
    let mut half: Option<u8> = None;
    for &c in data {
        let v = match c {
            b'>' => break,
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => continue,
        };
        match half.take() {
            Some(h) => out.push(h << 4 | v),
            None => half = Some(v),
        }
    }
    if let Some(h) = half {
        out.push(h << 4);
    }
    out
}

fn ascii85(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 4 / 5);
    let mut group = [0u32; 5];
    let mut n = 0;
    let body = data.strip_prefix(b"<~").unwrap_or(data);
    for &c in body {
        match c {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
            b'!'..=b'u' => {
                group[n] = (c - b'!') as u32;
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u64, |v, &g| v * 85 + g as u64);
                    out.extend_from_slice(&(v as u32).to_be_bytes());
                    n = 0;
                }
            }
            c if super::object::is_white(c) => {}
            _ => return None,
        }
    }
    if n > 1 {
        for g in group.iter_mut().skip(n) {
            *g = 84;
        }
        let v = group.iter().fold(0u64, |v, &g| v * 85 + g as u64);
        out.extend_from_slice(&(v as u32).to_be_bytes()[..n - 1]);
    }
    Some(out)
}

fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let n = data[i] as usize;
        i += 1;
        match n {
            0..=127 => {
                let end = (i + n + 1).min(data.len());
                out.extend_from_slice(&data[i..end]);
                i = end;
            }
            128 => break,
            _ => {
                if let Some(&b) = data.get(i) {
                    out.extend(std::iter::repeat_n(b, 257 - n));
                }
                i += 1;
            }
        }
    }
    out
}

/// LZW as PDF uses it: codes of 9 to 12 bits, 256 clears, 257 ends.
fn lzw(data: &[u8], early: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut table: Vec<Vec<u8>> = (0..=255u16).map(|b| vec![b as u8]).collect();
    table.push(Vec::new());
    table.push(Vec::new());
    let mut width = 9;
    let (mut buf, mut have, mut at) = (0u32, 0, 0);
    let mut prev: Option<Vec<u8>> = None;
    loop {
        while have < width {
            let Some(&b) = data.get(at) else { return out };
            at += 1;
            buf = buf << 8 | b as u32;
            have += 8;
        }
        let code = ((buf >> (have - width)) & ((1 << width) - 1)) as usize;
        have -= width;
        match code {
            256 => {
                table.truncate(258);
                width = 9;
                prev = None;
                continue;
            }
            257 => return out,
            _ => {}
        }
        let entry = match (table.get(code), &prev) {
            (Some(e), _) if code < table.len() && (code < 256 || !e.is_empty()) => e.clone(),
            (_, Some(p)) => {
                let mut e = p.clone();
                e.push(p[0]);
                e
            }
            _ => return out,
        };
        out.extend_from_slice(&entry);
        if let Some(p) = prev {
            let mut e = p;
            e.push(entry[0]);
            if table.len() < 4096 {
                table.push(e);
            }
        }
        let next = table.len() + early as usize;
        width = if next >= 2048 { 12 } else if next >= 1024 { 11 } else if next >= 512 { 10 } else { 9 };
        prev = Some(entry);
        if out.len() > MOST {
            return out;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_undo_what_they_did() {
        assert_eq!(ascii_hex(b"48 65 6c6C6F>"), b"Hello");
        assert_eq!(ascii85(b"<~87cURD]i,\"Ebo7~>").unwrap(), b"Hello World");
        assert_eq!(run_length(&[2, b'a', b'b', b'c', 254, b'x', 128]), b"abcxxx");
        // The PDF specification's own example (7.4.4.2).
        let lzw_data = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(&lzw(&lzw_data, true)[..], b"-----A---B");
    }
}
