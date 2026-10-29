//! The crypto (docs/specs/2026-10-26-sync-quick-send.md, section 2).
//!
//! - Items are sealed with XChaCha20-Poly1305 under a key derived from the
//!   group secret, with random 24-byte nonces. The envelope is
//!   `version (1) | epoch (4) | nonce (24) | ciphertext + tag`, authenticated
//!   with the group, epoch, kind, item id and sender, so the server can't
//!   swap or forge one.
//! - The plaintext is deflated and padded so every envelope is exactly one
//!   of six sizes: the server can't tell a password from a short note.
//! - Devices sign with Ed25519 and receive a new group secret, when a device
//!   is removed, through X25519.

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signer, Verifier};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub const VERSION: u8 = 1;
/// Envelope sizes, in bytes: every sealed item is exactly one of these.
pub const BUCKETS: [usize; 6] = [256, 1024, 4096, 16384, 65536, 262144];
/// version + epoch + nonce + tag.
pub const OVERHEAD: usize = 1 + 4 + 24 + 16;

pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the OS has no randomness");
    b
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn hkdf(salt: &[u8], ikm: &[u8], info: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), ikm).expand(info.as_bytes(), &mut out).expect("32 bytes is a valid length");
    out
}

pub fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("any key length");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// Equal in constant time.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// XChaCha20-Poly1305: `nonce | ciphertext + tag`.
pub fn seal(key: &[u8; 32], aad: &[u8], msg: &[u8]) -> Vec<u8> {
    let nonce: [u8; 24] = random();
    let mut out = nonce.to_vec();
    out.extend(XChaCha20Poly1305::new(key.into()).encrypt(XNonce::from_slice(&nonce), Payload { msg, aad }).expect("sealing cannot fail"));
    out
}

pub fn open(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 24 + 16 {
        return None;
    }
    XChaCha20Poly1305::new(key.into()).decrypt(XNonce::from_slice(&sealed[..24]), Payload { msg: &sealed[24..], aad }).ok()
}

/// A picture's bytes in segments of this many, each sealed on its own.
pub const SEGMENT: usize = 64 * 1024;

/// A large item (a picture) sealed as a STREAM (XChaCha20-Poly1305,
/// big-endian 32-bit counter, the last segment marked): `nonce (19) |
/// segment + tag | ...`. A segment can't be dropped, moved or cut off.
pub fn seal_stream(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    use chacha20poly1305::aead::stream::EncryptorBE32;
    let nonce: [u8; 19] = random();
    let mut out = Vec::with_capacity(19 + plain.len() + (plain.len() / SEGMENT + 1) * 16);
    out.extend_from_slice(&nonce);
    let mut e = EncryptorBE32::<XChaCha20Poly1305>::new(key.into(), (&nonce).into());
    let mut chunks = plain.chunks(SEGMENT).peekable();
    if chunks.peek().is_none() {
        out.extend(e.encrypt_last(Payload { msg: &[], aad }).expect("sealing cannot fail"));
        return out;
    }
    while let Some(c) = chunks.next() {
        if chunks.peek().is_none() {
            out.extend(e.encrypt_last(Payload { msg: c, aad }).expect("sealing cannot fail"));
            break;
        }
        out.extend(e.encrypt_next(Payload { msg: c, aad }).expect("sealing cannot fail"));
    }
    out
}

/// The bytes `seal_stream` sealed, if every segment is there and whole.
pub fn open_stream(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    use chacha20poly1305::aead::stream::DecryptorBE32;
    if sealed.len() < 19 + 16 {
        return None;
    }
    let (nonce, mut rest) = sealed.split_at(19);
    let nonce: [u8; 19] = nonce.try_into().ok()?;
    let mut d = DecryptorBE32::<XChaCha20Poly1305>::new(key.into(), (&nonce).into());
    let mut out = Vec::with_capacity(rest.len());
    loop {
        if rest.len() <= SEGMENT + 16 {
            out.extend(d.decrypt_last(Payload { msg: rest, aad }).ok()?);
            return Some(out);
        }
        let (seg, more) = rest.split_at(SEGMENT + 16);
        out.extend(d.decrypt_next(Payload { msg: seg, aad }).ok()?);
        rest = more;
    }
}

/// The group's keys, from its secret (one set per epoch).
#[derive(Clone)]
pub struct GroupKeys {
    pub group: [u8; 16],
    pub epoch: u32,
    pub send: [u8; 32],
    pub sync: [u8; 32],
    pub names: [u8; 32],
}

impl GroupKeys {
    pub fn new(group: [u8; 16], secret: &[u8; 32], epoch: u32) -> GroupKeys {
        GroupKeys {
            group,
            epoch,
            send: hkdf(&group, secret, "kolada/v1/send"),
            sync: hkdf(&group, secret, "kolada/v1/sync"),
            names: hkdf(&group, secret, "kolada/v1/names"),
        }
    }

    fn key(&self, kind: u8) -> &[u8; 32] {
        if kind == b's' { &self.sync } else { &self.send }
    }
}

fn item_aad(group: &[u8; 16], epoch: u32, kind: u8, id: &[u8; 16], sender: &[u8; 10]) -> Vec<u8> {
    let mut a = b"kolada/v1".to_vec();
    a.extend_from_slice(group);
    a.extend_from_slice(&epoch.to_be_bytes());
    a.push(kind);
    a.extend_from_slice(id);
    a.extend_from_slice(sender);
    a
}

/// The smallest bucket that holds `n` plaintext bytes, as the plaintext's
/// padded length; None if nothing does.
pub fn bucket_for(n: usize) -> Option<usize> {
    BUCKETS.iter().map(|b| b - OVERHEAD).find(|room| *room >= n)
}

/// Deflates and pads: `u32 length | deflated | zeros`, to a bucket.
pub fn pad(plain: &[u8]) -> Option<Vec<u8>> {
    let packed = miniz_oxide::deflate::compress_to_vec(plain, 6);
    let room = bucket_for(4 + packed.len())?;
    let mut out = Vec::with_capacity(room);
    out.extend_from_slice(&(packed.len() as u32).to_be_bytes());
    out.extend_from_slice(&packed);
    out.resize(room, 0);
    Some(out)
}

pub fn unpad(padded: &[u8]) -> Option<Vec<u8>> {
    let n = u32::from_be_bytes(padded.get(..4)?.try_into().ok()?) as usize;
    let packed = padded.get(4..4 + n)?;
    miniz_oxide::inflate::decompress_to_vec_with_limit(packed, 4 * BUCKETS[5]).ok()
}

/// Seals an item: the envelope the server stores and relays.
pub fn seal_item(keys: &GroupKeys, kind: u8, id: &[u8; 16], sender: &[u8; 10], plain: &[u8]) -> Option<Vec<u8>> {
    let padded = pad(plain)?;
    let aad = item_aad(&keys.group, keys.epoch, kind, id, sender);
    let mut out = vec![VERSION];
    out.extend_from_slice(&keys.epoch.to_be_bytes());
    out.extend(seal(keys.key(kind), &aad, &padded));
    Some(out)
}

/// The envelope's epoch, so the right keys can be picked.
pub fn envelope_epoch(env: &[u8]) -> Option<u32> {
    if env.first() != Some(&VERSION) {
        return None;
    }
    Some(u32::from_be_bytes(env.get(1..5)?.try_into().ok()?))
}

pub fn open_item(keys: &GroupKeys, kind: u8, id: &[u8; 16], sender: &[u8; 10], env: &[u8]) -> Option<Vec<u8>> {
    if envelope_epoch(env)? != keys.epoch {
        return None;
    }
    let aad = item_aad(&keys.group, keys.epoch, kind, id, sender);
    unpad(&open(keys.key(kind), &aad, &env[5..])?)
}

/// A device's own keys: Ed25519 to sign, X25519 to receive a new group secret.
#[derive(Clone)]
pub struct DeviceKeys {
    pub sign: [u8; 32],
    pub boxed: [u8; 32],
}

impl DeviceKeys {
    pub fn new() -> DeviceKeys {
        DeviceKeys { sign: random(), boxed: random() }
    }

    pub fn sign_pub(&self) -> [u8; 32] {
        ed25519_dalek::SigningKey::from_bytes(&self.sign).verifying_key().to_bytes()
    }

    pub fn box_pub(&self) -> [u8; 32] {
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(self.boxed)).to_bytes()
    }

    /// The first 10 bytes of SHA-256 of the signing key, as the server checks.
    pub fn id(&self) -> [u8; 10] {
        sha256(&self.sign_pub())[..10].try_into().unwrap()
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        ed25519_dalek::SigningKey::from_bytes(&self.sign).sign(msg).to_bytes()
    }
}

impl Default for DeviceKeys {
    fn default() -> Self {
        DeviceKeys::new()
    }
}

pub fn verify(sign_pub: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    match ed25519_dalek::VerifyingKey::from_bytes(sign_pub) {
        Ok(k) => k.verify(msg, &ed25519_dalek::Signature::from_bytes(sig)).is_ok(),
        Err(_) => false,
    }
}

/// Seals bytes to one device's X25519 key: `ephemeral key (32) | nonce | sealed`.
pub fn seal_to(box_pub: &[u8; 32], msg: &[u8]) -> Vec<u8> {
    let eph = x25519_dalek::StaticSecret::from(random::<32>());
    let eph_pub = x25519_dalek::PublicKey::from(&eph).to_bytes();
    let shared = eph.diffie_hellman(&x25519_dalek::PublicKey::from(*box_pub));
    let key = hkdf(&[eph_pub, *box_pub].concat(), shared.as_bytes(), "kolada/v1/rekey");
    let mut out = eph_pub.to_vec();
    out.extend(seal(&key, b"kolada/v1/rekey", msg));
    out
}

pub fn open_from(keys: &DeviceKeys, sealed: &[u8]) -> Option<Vec<u8>> {
    let eph_pub: [u8; 32] = sealed.get(..32)?.try_into().ok()?;
    let me = x25519_dalek::StaticSecret::from(keys.boxed);
    let shared = me.diffie_hellman(&x25519_dalek::PublicKey::from(eph_pub));
    let key = hkdf(&[eph_pub, keys.box_pub()].concat(), shared.as_bytes(), "kolada/v1/rekey");
    open(&key, b"kolada/v1/rekey", &sealed[32..])
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url without padding, as the server expects.
pub fn b64url(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 4 / 3 + 3);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            s.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    s
}

pub fn unb64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = B64.iter().position(|&x| x == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_stream_opens_whole_across_segment_edges_and_not_cut_or_moved() {
        let key: [u8; 32] = random();
        for n in [0usize, 1, SEGMENT - 1, SEGMENT, SEGMENT + 1, 3 * SEGMENT + 17] {
            let plain: Vec<u8> = (0..n).map(|i| (i * 7 % 251) as u8).collect();
            let sealed = seal_stream(&key, b"blob", &plain);
            assert_eq!(open_stream(&key, b"blob", &sealed).as_deref(), Some(&plain[..]), "{n} bytes");
            assert!(open_stream(&key, b"other", &sealed).is_none(), "bound to its blob");
            if n > SEGMENT {
                // The last segment cut off: the one before is not the last.
                assert!(open_stream(&key, b"blob", &sealed[..19 + SEGMENT + 16]).is_none(), "{n}: cut short");
                let mut bad = sealed.clone();
                bad[19 + 5] ^= 1;
                assert!(open_stream(&key, b"blob", &bad).is_none(), "{n}: changed");
            }
        }
    }

    use super::*;

    fn keys() -> GroupKeys {
        GroupKeys::new([1; 16], &[2; 32], 1)
    }

    #[test]
    fn an_item_opens_with_the_same_keys_and_fields() {
        let k = keys();
        let env = seal_item(&k, b'q', &[3; 16], &[4; 10], b"hello").unwrap();
        assert_eq!(open_item(&k, b'q', &[3; 16], &[4; 10], &env).unwrap(), b"hello");
    }

    #[test]
    fn changing_any_bound_field_fails() {
        let k = keys();
        let env = seal_item(&k, b'q', &[3; 16], &[4; 10], b"hello").unwrap();
        assert!(open_item(&k, b's', &[3; 16], &[4; 10], &env).is_none(), "kind");
        assert!(open_item(&k, b'q', &[9; 16], &[4; 10], &env).is_none(), "id");
        assert!(open_item(&k, b'q', &[3; 16], &[9; 10], &env).is_none(), "sender");
        assert!(open_item(&GroupKeys::new([9; 16], &[2; 32], 1), b'q', &[3; 16], &[4; 10], &env).is_none(), "group");
        assert!(open_item(&GroupKeys::new([1; 16], &[2; 32], 2), b'q', &[3; 16], &[4; 10], &env).is_none(), "epoch");
        assert!(open_item(&GroupKeys::new([1; 16], &[7; 32], 1), b'q', &[3; 16], &[4; 10], &env).is_none(), "secret");
        let mut bad = env.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(open_item(&k, b'q', &[3; 16], &[4; 10], &bad).is_none(), "tampered");
    }

    #[test]
    fn every_envelope_is_one_of_the_bucket_sizes() {
        let k = keys();
        for n in [0, 1, 8, 200, 1000, 5000, 20000] {
            let text: Vec<u8> = (0..n).map(|i| (i * 7919 % 251) as u8).collect();
            let env = seal_item(&k, b'q', &[3; 16], &[4; 10], &text).unwrap();
            assert!(BUCKETS.contains(&env.len()), "{n} -> {}", env.len());
        }
        // A password and a short note look the same.
        let a = seal_item(&k, b'q', &[3; 16], &[4; 10], b"hunter2!Xq9").unwrap();
        let b = seal_item(&k, b'q', &[3; 16], &[4; 10], b"Buy milk on the way home").unwrap();
        assert_eq!(a.len(), 256);
        assert_eq!(a.len(), b.len());
    }

    #[test]
    fn refuses_what_cannot_fit_the_largest_bucket() {
        // Noise deflate can't shrink (xorshift).
        let mut x = 0x9e3779b97f4a7c15u64;
        let noise: Vec<u8> = (0..300_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        assert!(seal_item(&keys(), b'q', &[3; 16], &[4; 10], &noise).is_none());
    }

    #[test]
    fn signatures_and_ids_match_what_the_server_checks() {
        let d = DeviceKeys::new();
        let sig = d.sign(b"kolada/v1|GET|/x|dev|1|abc");
        assert!(verify(&d.sign_pub(), b"kolada/v1|GET|/x|dev|1|abc", &sig));
        assert!(!verify(&d.sign_pub(), b"kolada/v1|GET|/y|dev|1|abc", &sig));
        assert_eq!(d.id(), sha256(&d.sign_pub())[..10]);
    }

    #[test]
    fn a_new_secret_sealed_to_one_device_opens_only_there() {
        let a = DeviceKeys::new();
        let b = DeviceKeys::new();
        let sealed = seal_to(&a.box_pub(), b"new secret");
        assert_eq!(open_from(&a, &sealed).unwrap(), b"new secret");
        assert!(open_from(&b, &sealed).is_none());
    }

    #[test]
    fn hex_and_base64url_round_trip() {
        for n in 0..40 {
            let b: Vec<u8> = (0..n).map(|i| (i * 37) as u8).collect();
            assert_eq!(unb64url(&b64url(&b)).unwrap(), b);
        }
        assert_eq!(b64url(&[0xfb, 0xff]), "-_8");
        assert_eq!(unhex::<3>(&hex(&[1, 2, 255])), Some([1, 2, 255]));
        assert_eq!(unhex::<2>("zz00"), None);
    }
}
