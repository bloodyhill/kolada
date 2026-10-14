//! DEFLATE (RFC 1951) and zlib (RFC 1950) decompression: how zip archives
//! (DOCX) and PDF streams are compressed.
//!
//! After Mark Adler's `puff`: canonical Huffman codes decoded a bit at a
//! time, which is small and exact, and quick enough for documents (tens of
//! MB/s) on the worker that converts them.

use crate::Error;

/// Longest code, in bits.
const MAX_BITS: usize = 15;

/// Lengths and distances: base values and extra bits, by symbol.
const LENGTH_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// The order code-length codes are listed in.
const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn bad(what: &str) -> Error {
    Error(format!("damaged compressed data: {what}"))
}

/// Reads bits from the low end of each byte, as DEFLATE packs them.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    buf: u64,
    have: u32,
}

impl Bits<'_> {
    fn need(&mut self, n: u32) -> Result<(), Error> {
        while self.have < n {
            let byte = *self.data.get(self.at).ok_or_else(|| bad("it ends early"))?;
            self.at += 1;
            self.buf |= (byte as u64) << self.have;
            self.have += 8;
        }
        Ok(())
    }

    fn bits(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.have -= n;
        Ok(v)
    }

    /// Drops the bits left in the byte being read.
    fn align(&mut self) {
        let drop = self.have % 8;
        self.buf >>= drop;
        self.have -= drop;
    }
}

/// A canonical Huffman code: how many codes of each length, and the
/// symbols in code order.
struct Code {
    count: [u16; MAX_BITS + 1],
    symbol: Vec<u16>,
}

impl Code {
    /// The code for these lengths, and how many codes it lacks (0:
    /// complete; less than 0 is refused as over-subscribed).
    fn new(lengths: &[u8]) -> Result<(Code, i32), Error> {
        let mut count = [0u16; MAX_BITS + 1];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut left: i32 = 1;
        for &c in &count[1..] {
            left <<= 1;
            left -= c as i32;
            if left < 0 {
                return Err(bad("a code with too many symbols"));
            }
        }
        let mut offs = [0u16; MAX_BITS + 1];
        for len in 1..MAX_BITS {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Ok((Code { count, symbol }, left))
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, Error> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=MAX_BITS {
            code |= bits.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return Ok(self.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(bad("a code that means nothing"))
    }
}

/// Decompresses raw DEFLATE data.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(data.len() * 4);
    inflate_into(data, &mut out)?;
    Ok(out)
}

/// Decompresses raw DEFLATE data into `out`; on an error, `out` holds what
/// came before it.
pub fn inflate_into(data: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
    let mut bits = Bits { data, at: 0, buf: 0, have: 0 };
    loop {
        let last = bits.bits(1)? == 1;
        match bits.bits(2)? {
            0 => stored(&mut bits, out)?,
            1 => {
                let (lit, dist) = fixed();
                codes(&mut bits, out, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = dynamic(&mut bits)?;
                codes(&mut bits, out, &lit, &dist)?;
            }
            _ => return Err(bad("a block of an unknown kind")),
        }
        if last {
            return Ok(());
        }
    }
}

/// Decompresses zlib data: a two-byte header, DEFLATE data, a checksum
/// (not checked: a damaged stream fails on its own, and a PDF's are often
/// cut short).
pub fn zlib(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(data.len() * 4);
    zlib_into(data, &mut out)?;
    Ok(out)
}

/// As `zlib`, into `out`; on an error, `out` holds what came before it.
pub fn zlib_into(data: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
    match data {
        [cmf, flg, rest @ ..] if cmf & 0x0F == 8 && ((*cmf as u16) << 8 | *flg as u16).is_multiple_of(31) => {
            // A preset dictionary is never used by these formats.
            if flg & 0x20 != 0 {
                return Err(bad("a preset dictionary"));
            }
            inflate_into(rest, out)
        }
        // Some writers leave the header out.
        _ => inflate_into(data, out),
    }
}

fn stored(bits: &mut Bits, out: &mut Vec<u8>) -> Result<(), Error> {
    bits.align();
    let len = bits.bits(16)? as usize;
    let nlen = bits.bits(16)? as usize;
    if len != !nlen & 0xFFFF {
        return Err(bad("a stored block's length"));
    }
    // Whole bytes left in the buffer come first.
    let mut copied = 0;
    while copied < len && bits.have >= 8 {
        out.push(bits.bits(8)? as u8);
        copied += 1;
    }
    let rest = len - copied;
    let block = bits.data.get(bits.at..bits.at + rest).ok_or_else(|| bad("a stored block cut short"))?;
    out.extend_from_slice(block);
    bits.at += rest;
    Ok(())
}

fn fixed() -> (Code, Code) {
    let mut lengths = [0u8; 288];
    lengths[..144].fill(8);
    lengths[144..256].fill(9);
    lengths[256..280].fill(7);
    lengths[280..].fill(8);
    let lit = Code::new(&lengths).expect("the fixed code").0;
    let dist = Code::new(&[5u8; 30]).expect("the fixed code").0;
    (lit, dist)
}

fn dynamic(bits: &mut Bits) -> Result<(Code, Code), Error> {
    let nlen = bits.bits(5)? as usize + 257;
    let ndist = bits.bits(5)? as usize + 1;
    let ncode = bits.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(bad("too many codes"));
    }
    let mut lengths = [0u8; 19];
    for &i in &ORDER[..ncode] {
        lengths[i] = bits.bits(3)? as u8;
    }
    let (lencode, left) = Code::new(&lengths)?;
    if left != 0 {
        return Err(bad("an incomplete code-length code"));
    }
    let mut lengths = vec![0u8; nlen + ndist];
    let mut i = 0;
    while i < nlen + ndist {
        let sym = lencode.decode(bits)?;
        let (value, repeat) = match sym {
            0..=15 => (sym as u8, 1),
            16 => {
                let previous = *lengths.get(i.wrapping_sub(1)).ok_or_else(|| bad("a repeat with nothing before it"))?;
                (previous, 3 + bits.bits(2)? as usize)
            }
            17 => (0, 3 + bits.bits(3)? as usize),
            _ => (0, 11 + bits.bits(7)? as usize),
        };
        if i + repeat > nlen + ndist {
            return Err(bad("too many lengths"));
        }
        lengths[i..i + repeat].fill(value);
        i += repeat;
    }
    if lengths[256] == 0 {
        return Err(bad("no end-of-block code"));
    }
    // An incomplete code is allowed only when it has a single symbol.
    let (lit, left) = Code::new(&lengths[..nlen])?;
    if left > 0 && nlen as i32 - lit.count[0] as i32 != 1 {
        return Err(bad("an incomplete literal code"));
    }
    let (dist, left) = Code::new(&lengths[nlen..])?;
    if left > 0 && ndist as i32 - dist.count[0] as i32 != 1 {
        return Err(bad("an incomplete distance code"));
    }
    Ok((lit, dist))
}

fn codes(bits: &mut Bits, out: &mut Vec<u8>, lit: &Code, dist: &Code) -> Result<(), Error> {
    loop {
        let sym = lit.decode(bits)? as usize;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Ok(()),
            _ => {
                let s = sym - 257;
                if s >= 29 {
                    return Err(bad("a length out of range"));
                }
                let len = LENGTH_BASE[s] as usize + bits.bits(LENGTH_EXTRA[s] as u32)? as usize;
                let d = dist.decode(bits)? as usize;
                if d >= 30 {
                    return Err(bad("a distance out of range"));
                }
                let back = DIST_BASE[d] as usize + bits.bits(DIST_EXTRA[d] as u32)? as usize;
                if back > out.len() {
                    return Err(bad("a distance before the start"));
                }
                let from = out.len() - back;
                if back >= len {
                    out.extend_from_within(from..from + len);
                } else {
                    // Overlapping: the copy repeats what it is writing.
                    for k in 0..len {
                        out.push(out[from + k]);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From Python: zlib.compress(b"hello hello hello hello, world") at
    /// levels 9 (a fixed-code block) and 0 (a stored block), and a longer
    /// text at 9 (a dynamic-code block).
    const HELLO_9: &[u8] = &[
        0x78, 0xda, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x75, 0x14, 0xca, 0xf3, 0x8b, 0x72, 0x52, 0x00, 0xad, 0x6f, 0x0b, 0x25,
    ];
    const HELLO_0: &[u8] = &[
        0x78, 0x01, 0x01, 0x1e, 0x00, 0xe1, 0xff, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x20, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x20, 0x68, 0x65, 0x6c, 0x6c, 0x6f,
        0x20, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x2c, 0x20, 0x77, 0x6f, 0x72, 0x6c, 0x64, 0xad, 0x6f, 0x0b, 0x25,
    ];
    const DYNAMIC: &[u8] = &[
        0x78, 0xda, 0xed, 0x8e, 0x39, 0x0e, 0xc2, 0x30, 0x10, 0x45, 0xaf, 0xf2, 0x11, 0x8b, 0x80, 0x22, 0x62, 0x5f, 0x44, 0xc3, 0x09, 0xa8, 0x52,
        0x23, 0x25, 0x30, 0xb1, 0x0d, 0x89, 0x27, 0xf1, 0x92, 0xe0, 0x14, 0x9c, 0x1d, 0x8b, 0x1b, 0xd0, 0x53, 0xff, 0xe5, 0xbd, 0x54, 0x12, 0x1a,
        0xaf, 0x6e, 0x4f, 0xe4, 0x86, 0x3b, 0x8d, 0x82, 0x5f, 0x78, 0xf8, 0xaa, 0xb6, 0xe0, 0x96, 0x0c, 0x5c, 0x8c, 0xcb, 0xac, 0x0f, 0xb8, 0xb3,
        0x48, 0x70, 0x61, 0x67, 0x03, 0xb8, 0x26, 0x6d, 0x91, 0x53, 0xc1, 0x86, 0x10, 0xd8, 0xa3, 0x50, 0x5a, 0x59, 0xf9, 0xed, 0x3a, 0xc9, 0x5e,
        0x48, 0x77, 0xc2, 0x62, 0xb9, 0x5a, 0x6f, 0xb6, 0xbb, 0xfd, 0xe1, 0x88, 0xf7, 0xe0, 0x3c, 0x1c, 0x8d, 0xaf, 0x93, 0xf9, 0x74, 0x86, 0x3a,
        0x8b, 0xa4, 0x2a, 0x20, 0x8f, 0x98, 0x4e, 0x39, 0x19, 0xb7, 0x2d, 0xc5, 0xf3, 0x9e, 0x34, 0x4a, 0xd5, 0x78, 0x36, 0x91, 0x2e, 0x6c, 0x82,
        0xf4, 0xef, 0xf5, 0x93, 0xd7, 0x07, 0x3b, 0xc3, 0x9a, 0x96,
    ];

    #[test]
    fn inflates_fixed_and_stored_blocks() {
        assert_eq!(zlib(HELLO_9).unwrap(), b"hello hello hello hello, world");
        assert_eq!(zlib(HELLO_0).unwrap(), b"hello hello hello hello, world");
        let text = b"The quick brown fox jumps over the lazy dog. Notsy opens before you finish the thought; 0123456789 ~!@#$%^&*() pack my box with five dozen liquor jugs. ".repeat(3);
        assert_eq!(zlib(DYNAMIC).unwrap(), text);
    }

    #[test]
    fn refuses_damage_without_panicking() {
        assert!(zlib(&HELLO_9[..8]).is_err());
        let mut flipped = HELLO_9.to_vec();
        for i in 2..flipped.len() {
            flipped[i] ^= 0xA5;
            let _ = zlib(&flipped);
            flipped[i] ^= 0xA5;
        }
        for junk in [&b""[..], b"\x78\xda", b"\xff\xff\xff\xff", b"\x05\x00\x00"] {
            let _ = zlib(junk);
        }
    }
}
