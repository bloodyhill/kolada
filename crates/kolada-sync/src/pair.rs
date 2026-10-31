//! Pairing's crypto (design, section 1): SPAKE2 on the code's secret, then
//! confirmations and a sealed channel, all relayed by a server that learns
//! nothing it could use.
//!
//! ```text
//! opener                                   joiner
//!   msg_a  -------- (server keeps it) -------->
//!   <------ confirm: msg_b | confirm_b | sealed intro (keys, kind, name)
//!   checks confirm_b; the user allows the new device
//!   welcome: confirm_a | sealed (group, secret, epoch, voucher) ----->
//! ```
//!
//! The joiner's keys travel inside the sealed channel, so the server can't
//! substitute its own; a man in the middle gets one guess at the secret per
//! try.

use spake2::{Ed25519Group, Identity, Password, Spake2};

use crate::code::Code;
use crate::crypto::{b64url, hex, hkdf, hmac, open, same, seal, DeviceKeys};

const ID_A: &[u8] = b"kolada/v1/pair/opener";
const ID_B: &[u8] = b"kolada/v1/pair/joiner";
const SALT: &[u8] = b"kolada/v1/pair";

/// What both sides derive from SPAKE2's key.
#[derive(Clone)]
pub struct Session {
    confirm_a: [u8; 32],
    confirm_b: [u8; 32],
    channel: [u8; 32],
}

impl Session {
    fn new(key: &[u8], msg_a: &[u8], msg_b: &[u8]) -> Session {
        let ka = hkdf(SALT, key, "confirm-a");
        let kb = hkdf(SALT, key, "confirm-b");
        Session {
            confirm_a: hmac(&ka, &[b"A", msg_a, msg_b]),
            confirm_b: hmac(&kb, &[b"B", msg_a, msg_b]),
            channel: hkdf(SALT, key, "channel"),
        }
    }
}

/// A device introducing itself to the group it joins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intro {
    pub sign_pub: [u8; 32],
    pub box_pub: [u8; 32],
    pub kind: String,
    pub name: String,
}

impl Intro {
    pub fn of(keys: &DeviceKeys, kind: &str, name: &str) -> Intro {
        Intro { sign_pub: keys.sign_pub(), box_pub: keys.box_pub(), kind: kind.into(), name: name.into() }
    }

    pub fn id(&self) -> [u8; 10] {
        crate::crypto::sha256(&self.sign_pub)[..10].try_into().unwrap()
    }

    fn encode(&self) -> Vec<u8> {
        let mut b = self.sign_pub.to_vec();
        b.extend_from_slice(&self.box_pub);
        b.push(self.kind.len().min(255) as u8);
        b.extend_from_slice(&self.kind.as_bytes()[..self.kind.len().min(255)]);
        let name = &self.name.as_bytes()[..self.name.len().min(1000)];
        b.extend_from_slice(&(name.len() as u16).to_be_bytes());
        b.extend_from_slice(name);
        b
    }

    fn decode(b: &[u8]) -> Option<Intro> {
        let sign_pub = b.get(..32)?.try_into().ok()?;
        let box_pub = b.get(32..64)?.try_into().ok()?;
        let k = *b.get(64)? as usize;
        let kind = String::from_utf8(b.get(65..65 + k)?.to_vec()).ok()?;
        let at = 65 + k;
        let n = u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize;
        let name = String::from_utf8_lossy(b.get(at + 2..at + 2 + n)?).into_owned();
        Some(Intro { sign_pub, box_pub, kind, name })
    }
}

/// What the joiner receives: the group, and a voucher to show the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    pub group: [u8; 16],
    pub secret: [u8; 32],
    pub epoch: u32,
    pub voucher: Vec<u8>,
}

/// The opener's side of one try.
pub struct Opener {
    code: Code,
    spake: Option<Spake2<Ed25519Group>>,
    msg_a: Vec<u8>,
}

impl Opener {
    /// Starts a try; returns SPAKE2's message for the server to keep.
    pub fn new(code: Code) -> (Opener, Vec<u8>) {
        let (spake, msg_a) = Spake2::<Ed25519Group>::start_a(&Password::new(code.password()), &Identity::new(ID_A), &Identity::new(ID_B));
        (Opener { code, spake: Some(spake), msg_a: msg_a.clone() }, msg_a)
    }

    /// A fresh start after a failed try (SPAKE2 is single use).
    pub fn again(&self) -> (Opener, Vec<u8>) {
        Opener::new(self.code.clone())
    }

    /// Checks the joiner's `confirm`. Ok means the joiner knew the code.
    pub fn check(&mut self, body: &[u8]) -> Result<(Session, Intro), ()> {
        let n = *body.first().ok_or(())? as usize;
        let msg_b = body.get(1..1 + n).ok_or(())?;
        let confirm_b = body.get(1 + n..33 + n).ok_or(())?;
        let sealed = body.get(33 + n..).ok_or(())?;
        let key = self.spake.take().ok_or(())?.finish(msg_b).map_err(|_| ())?;
        let s = Session::new(&key, &self.msg_a, msg_b);
        if !same(&s.confirm_b, confirm_b) {
            return Err(());
        }
        let intro = Intro::decode(&open(&s.channel, b"kolada/v1/pair/intro", sealed).ok_or(())?).ok_or(())?;
        Ok((s, intro))
    }
}

/// The joiner's whole side: answers the opener's message.
pub fn join(code: &Code, msg_a: &[u8], intro: &Intro) -> Option<(Vec<u8>, Session)> {
    let (spake, msg_b) = Spake2::<Ed25519Group>::start_b(&Password::new(code.password()), &Identity::new(ID_A), &Identity::new(ID_B));
    let key = spake.finish(msg_a).ok()?;
    let s = Session::new(&key, msg_a, &msg_b);
    let mut body = vec![msg_b.len() as u8];
    body.extend_from_slice(&msg_b);
    body.extend_from_slice(&s.confirm_b);
    body.extend(seal(&s.channel, b"kolada/v1/pair/intro", &intro.encode()));
    Some((body, s))
}

/// The opener's welcome, once the user allowed the new device.
pub fn welcome(s: &Session, w: &Welcome) -> Vec<u8> {
    let mut plain = w.group.to_vec();
    plain.extend_from_slice(&w.secret);
    plain.extend_from_slice(&w.epoch.to_be_bytes());
    plain.extend_from_slice(&w.voucher);
    let mut body = s.confirm_a.to_vec();
    body.extend(seal(&s.channel, b"kolada/v1/pair/welcome", &plain));
    body
}

/// The joiner reads the welcome; None unless it came from whoever knew the code.
pub fn read_welcome(s: &Session, body: &[u8]) -> Option<Welcome> {
    if !same(body.get(..32)?, &s.confirm_a) {
        return None;
    }
    let p = open(&s.channel, b"kolada/v1/pair/welcome", &body[32..])?;
    Some(Welcome {
        group: p.get(..16)?.try_into().ok()?,
        secret: p.get(16..48)?.try_into().ok()?,
        epoch: u32::from_be_bytes(p.get(48..52)?.try_into().ok()?),
        voucher: p.get(52..)?.to_vec(),
    })
}

/// A member's voucher for a new device, as the server checks it
/// (the sync server): `by | time | signature`.
pub fn voucher(member: &DeviceKeys, group: &[u8; 16], newcomer: &Intro, time: u64) -> Vec<u8> {
    let text = format!(
        "kolada/v1/voucher|{}|{}|{}|{}|{}",
        hex(group),
        hex(&newcomer.id()),
        b64url(&newcomer.sign_pub),
        b64url(&newcomer.box_pub),
        time
    );
    let mut v = member.id().to_vec();
    v.extend_from_slice(&time.to_be_bytes());
    v.extend_from_slice(&member.sign(text.as_bytes()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::parse;

    fn intro() -> Intro {
        Intro::of(&DeviceKeys::new(), "ios", "iPhone")
    }

    #[test]
    fn two_sides_with_the_same_code_agree() {
        let code = parse("F4FS-2RSA").unwrap();
        let (mut a, msg_a) = Opener::new(code.clone());
        let me = intro();
        let (body, sb) = join(&code, &msg_a, &me).unwrap();
        let (sa, got) = a.check(&body).unwrap();
        assert_eq!(got, me);
        let w = Welcome { group: [1; 16], secret: [2; 32], epoch: 1, voucher: vec![3; 82] };
        assert_eq!(read_welcome(&sb, &welcome(&sa, &w)), Some(w));
    }

    #[test]
    fn a_wrong_secret_fails_the_confirm() {
        let (mut a, msg_a) = Opener::new(parse("F4FS-2RSA").unwrap());
        let (body, _) = join(&parse("F4FS-2RSB").unwrap(), &msg_a, &intro()).unwrap();
        assert!(a.check(&body).is_err());
    }

    #[test]
    fn the_same_secret_under_another_slot_fails() {
        let (mut a, msg_a) = Opener::new(parse("F4FS-2RSA").unwrap());
        let (body, _) = join(&parse("F4GS-2RSA").unwrap(), &msg_a, &intro()).unwrap();
        assert!(a.check(&body).is_err());
    }

    #[test]
    fn a_relay_that_swaps_anything_fails() {
        let code = parse("F4FS-2RSA").unwrap();
        // The server substitutes its own SPAKE2 message (it doesn't know the secret).
        let (mut a, _) = Opener::new(code.clone());
        let (_, fake_a) = Opener::new(parse("ZZZZ-ZZZZ").unwrap());
        let (body, _) = join(&code, &fake_a, &intro()).unwrap();
        assert!(a.check(&body).is_err());
        // The server swaps the joiner's keys inside the sealed intro.
        let (mut a, msg_a) = Opener::new(code.clone());
        let (mut body, _) = join(&code, &msg_a, &intro()).unwrap();
        let last = body.len() - 1;
        body[last] ^= 1;
        assert!(a.check(&body).is_err());
        // A welcome from anyone but the opener is refused.
        let (mut a, msg_a) = Opener::new(code.clone());
        let (body, sb) = join(&code, &msg_a, &intro()).unwrap();
        let (sa, _) = a.check(&body).unwrap();
        let w = Welcome { group: [1; 16], secret: [2; 32], epoch: 1, voucher: vec![] };
        let mut wb = welcome(&sa, &w);
        wb[0] ^= 1;
        assert!(read_welcome(&sb, &wb).is_none());
    }

    #[test]
    fn a_try_is_single_use() {
        let code = parse("F4FS-2RSA").unwrap();
        let (mut a, msg_a) = Opener::new(code.clone());
        let (body, _) = join(&code, &msg_a, &intro()).unwrap();
        assert!(a.check(&body).is_ok());
        assert!(a.check(&body).is_err());
    }

    #[test]
    fn a_voucher_is_82_bytes_and_verifies() {
        let member = DeviceKeys::new();
        let me = intro();
        let v = voucher(&member, &[7; 16], &me, 1234);
        assert_eq!(v.len(), 82);
        assert_eq!(&v[..10], &member.id());
        let text = format!("kolada/v1/voucher|{}|{}|{}|{}|1234", hex(&[7; 16]), hex(&me.id()), b64url(&me.sign_pub), b64url(&me.box_pub));
        assert!(crate::crypto::verify(&member.sign_pub(), text.as_bytes(), v[18..].try_into().unwrap()));
    }
}
