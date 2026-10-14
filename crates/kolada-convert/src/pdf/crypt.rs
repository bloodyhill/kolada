//! Reading encrypted PDFs that open without a password: the standard
//! security handler's revisions 2 to 4 (RC4 of 40 to 128 bits, AES-128),
//! as PDFs locked against copying or printing are (ISO 32000 §7.6).
//! MD5, RC4 and AES decryption are here; no file is ever written.

use super::object::{Dict, Obj};
use crate::Error;

/// The padding a blank password becomes (Algorithm 2).
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9,
    0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// How a file's strings and streams are encrypted.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Method {
    Rc4,
    Aes,
    None,
}

pub struct Crypt {
    key: Vec<u8>,
    streams: Method,
    strings: Method,
}

impl Crypt {
    /// The decryption for a file with this /Encrypt dictionary and first
    /// /ID; an error if it needs a password or a method not read here.
    pub fn new(enc: &Dict, id: &[u8]) -> Result<Crypt, Error> {
        let int = |k: &str| enc.get(k).and_then(Obj::as_i64);
        if enc.get("Filter").and_then(Obj::as_name) != Some("Standard") {
            return Err(Error("the PDF is protected in a way Kolada does not read".to_string()));
        }
        let (v, r) = (int("V").unwrap_or(0), int("R").unwrap_or(0));
        if !(2..=4).contains(&r) {
            return Err(Error("the PDF is encrypted in a way Kolada does not read yet (AES-256)".to_string()));
        }
        let bytes = |k: &str| match enc.get(k) {
            Some(Obj::Str(s)) => s.clone(),
            _ => Vec::new(),
        };
        let (o, u) = (bytes("O"), bytes("U"));
        let p = int("P").unwrap_or(0) as i32;
        let length = if r == 2 { 40 } else { int("Length").unwrap_or(128) };
        // Revision 4: the crypt filters say which method each kind uses.
        let method = |name: &str| -> Method {
            if v < 4 {
                return Method::Rc4;
            }
            let filter = enc.get(name).and_then(Obj::as_name).unwrap_or("Identity");
            if filter == "Identity" {
                return Method::None;
            }
            let cfm = enc.get("CF").and_then(Obj::as_dict).and_then(|cf| cf.get(filter)).and_then(Obj::as_dict).and_then(|f| f.get("CFM")).and_then(Obj::as_name);
            match cfm {
                Some("AESV2") => Method::Aes,
                Some("None") => Method::None,
                _ => Method::Rc4,
            }
        };
        let n = if v >= 4 { 16 } else { (length.clamp(40, 128) / 8) as usize };
        let encrypt_metadata = !matches!(enc.get("EncryptMetadata"), Some(Obj::Bool(false)));
        // Algorithm 2: the file's key, from a blank user password.
        let mut input = PAD.to_vec();
        input.extend_from_slice(&o);
        input.extend_from_slice(&p.to_le_bytes());
        input.extend_from_slice(id);
        if r >= 4 && !encrypt_metadata {
            input.extend_from_slice(&[0xFF; 4]);
        }
        let mut hash = md5(&input);
        if r >= 3 {
            for _ in 0..50 {
                hash = md5(&hash[..n]);
            }
        }
        let key = hash[..n].to_vec();
        // Algorithms 4 and 5: the blank password is the user password.
        let opens = if r == 2 {
            rc4(&key, &PAD) == u
        } else {
            let mut check = [&PAD[..], id].concat();
            check = md5(&check).to_vec();
            check = rc4(&key, &check);
            for i in 1..=19u8 {
                let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
                check = rc4(&k, &check);
            }
            u.len() >= 16 && check[..16] == u[..16]
        };
        if !opens {
            return Err(Error("the PDF needs a password to open".to_string()));
        }
        Ok(Crypt { key, streams: method("StmF"), strings: method("StrF") })
    }

    /// An object's own key (Algorithm 1).
    fn object_key(&self, num: u32, generation: u16, aes: bool) -> Vec<u8> {
        let mut input = self.key.clone();
        input.extend_from_slice(&num.to_le_bytes()[..3]);
        input.extend_from_slice(&generation.to_le_bytes());
        if aes {
            input.extend_from_slice(b"sAlT");
        }
        md5(&input)[..(self.key.len() + 5).min(16)].to_vec()
    }

    fn decrypt(&self, method: Method, num: u32, generation: u16, data: &[u8]) -> Vec<u8> {
        match method {
            Method::None => data.to_vec(),
            Method::Rc4 => rc4(&self.object_key(num, generation, false), data),
            Method::Aes => aes_cbc(&self.object_key(num, generation, true), data),
        }
    }

    /// An object read from the file, its strings and stream decrypted.
    pub fn object(&self, num: u32, generation: u16, o: Obj) -> Obj {
        match o {
            Obj::Str(s) => Obj::Str(self.decrypt(self.strings, num, generation, &s)),
            Obj::Array(a) => Obj::Array(a.into_iter().map(|x| self.object(num, generation, x)).collect()),
            Obj::Dict(d) => Obj::Dict(self.dict(num, generation, d)),
            Obj::Stream(d, data) => {
                // Cross-reference streams are never encrypted; metadata may not be.
                let plain = d.get("Type").and_then(Obj::as_name) == Some("XRef");
                let data = if plain { data } else { std::rc::Rc::new(self.decrypt(self.streams, num, generation, &data)) };
                Obj::Stream(self.dict(num, generation, d), data)
            }
            other => other,
        }
    }

    fn dict(&self, num: u32, generation: u16, d: Dict) -> Dict {
        Dict(d.0.into_iter().map(|(k, v)| (k, self.object(num, generation, v))).collect())
    }
}

/// MD5 (RFC 1321).
pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16,
        23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32).collect();
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    let (mut a0, mut b0, mut c0, mut d0) = (0x6745_2301u32, 0xEFCD_AB89u32, 0x98BA_DCFEu32, 0x1032_5476u32);
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// RC4: encrypting and decrypting are the same.
pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j: u8 = 0;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|&b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

const SBOX: [u8; 256] = {
    // The AES S-box, computed: the multiplicative inverse in GF(2^8),
    // then the affine transform.
    let mut sbox = [0u8; 256];
    let mut p: u8 = 1;
    let mut q: u8 = 1;
    loop {
        // p times 3
        p = p ^ (p << 1) ^ if p & 0x80 != 0 { 0x1B } else { 0 };
        // q divided by 3
        q ^= q << 1;
        q ^= q << 2;
        q ^= q << 4;
        if q & 0x80 != 0 {
            q ^= 0x09;
        }
        let x = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4);
        sbox[p as usize] = x ^ 0x63;
        if p == 1 {
            break;
        }
    }
    sbox[0] = 0x63;
    sbox
};

const INV_SBOX: [u8; 256] = {
    let mut inv = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        inv[SBOX[i] as usize] = i as u8;
        i += 1;
    }
    inv
};

fn xtime(b: u8) -> u8 {
    (b << 1) ^ if b & 0x80 != 0 { 0x1B } else { 0 }
}

fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

/// AES round keys for a 128 or 256 bit key.
fn expand(key: &[u8]) -> Vec<[u8; 16]> {
    let nk = key.len() / 4;
    let rounds = nk + 6;
    let mut w: Vec<[u8; 4]> = key.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
    let mut rcon: u8 = 1;
    for i in nk..4 * (rounds + 1) {
        let mut t = w[i - 1];
        if i % nk == 0 {
            t = [SBOX[t[1] as usize] ^ rcon, SBOX[t[2] as usize], SBOX[t[3] as usize], SBOX[t[0] as usize]];
            rcon = xtime(rcon);
        } else if nk > 6 && i % nk == 4 {
            t = t.map(|b| SBOX[b as usize]);
        }
        let prev = w[i - nk];
        w.push([prev[0] ^ t[0], prev[1] ^ t[1], prev[2] ^ t[2], prev[3] ^ t[3]]);
    }
    w.chunks(4).map(|c| std::array::from_fn(|k| c[k / 4][k % 4])).collect()
}

fn decrypt_block(round_keys: &[[u8; 16]], block: &mut [u8; 16]) {
    let rounds = round_keys.len() - 1;
    let add = |b: &mut [u8; 16], k: &[u8; 16]| b.iter_mut().zip(k).for_each(|(x, y)| *x ^= y);
    add(block, &round_keys[rounds]);
    for round in (0..rounds).rev() {
        // Inverse shift rows, then inverse substitution.
        let s = *block;
        for c in 0..4 {
            for r in 0..4 {
                block[c * 4 + r] = INV_SBOX[s[((c + 4 - r) % 4) * 4 + r] as usize];
            }
        }
        add(block, &round_keys[round]);
        if round > 0 {
            for c in 0..4 {
                let col = [block[c * 4], block[c * 4 + 1], block[c * 4 + 2], block[c * 4 + 3]];
                for r in 0..4 {
                    block[c * 4 + r] =
                        gmul(col[r], 14) ^ gmul(col[(r + 1) % 4], 11) ^ gmul(col[(r + 2) % 4], 13) ^ gmul(col[(r + 3) % 4], 9);
                }
            }
        }
    }
}

/// AES-CBC decryption: the IV first, PKCS#5 padding at the end.
pub fn aes_cbc(key: &[u8], data: &[u8]) -> Vec<u8> {
    if data.len() < 32 || !matches!(key.len(), 16 | 32) {
        return Vec::new();
    }
    let keys = expand(key);
    let mut prev: [u8; 16] = data[..16].try_into().expect("16 bytes");
    let mut out = Vec::with_capacity(data.len());
    for &cipher in data[16..].as_chunks::<16>().0 {
        let mut block = cipher;
        decrypt_block(&keys, &mut block);
        out.extend(block.iter().zip(prev).map(|(b, p)| b ^ p));
        prev = cipher;
    }
    if let Some(&pad) = out.last()
        && (1..=16).contains(&pad)
        && out.len() >= pad as usize
    {
        out.truncate(out.len() - pad as usize);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn md5_rc4_and_aes_as_their_standards_give_them() {
        assert_eq!(md5(b"").to_vec(), hex("d41d8cd98f00b204e9800998ecf8427e"));
        assert_eq!(md5(b"The quick brown fox jumps over the lazy dog").to_vec(), hex("9e107d9d372bb6826bd81d3542a419d6"));
        assert_eq!(rc4(b"Key", b"Plaintext"), hex("bbf316e8d940af0ad3"));
        assert_eq!(SBOX[0x53], 0xED);
        // FIPS-197 appendix C.1 and C.3: one block each, behind a zero IV.
        let mut block: [u8; 16] = hex("69c4e0d86a7b0430d8cdb78070b4c55a").try_into().unwrap();
        decrypt_block(&expand(&hex("000102030405060708090a0b0c0d0e0f")), &mut block);
        assert_eq!(block.to_vec(), hex("00112233445566778899aabbccddeeff"));
        let mut block: [u8; 16] = hex("8ea2b7ca516745bfeafc49904b496089").try_into().unwrap();
        decrypt_block(&expand(&hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")), &mut block);
        assert_eq!(block.to_vec(), hex("00112233445566778899aabbccddeeff"));
    }
}
