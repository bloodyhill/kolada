//! What a device keeps: its keys (handed to the shell's secret store as one
//! blob) and its state (in the app's data folder, sealed with a local key so
//! passwords waiting to be copied are never on disk in the clear).

use crate::crypto::{open, random, seal, DeviceKeys, GroupKeys};

/// A small length-prefixed binary writer and reader.
#[derive(Default)]
pub struct W(pub Vec<u8>);

impl W {
    pub fn u8(&mut self, v: u8) -> &mut Self { self.0.push(v); self }
    pub fn u32(&mut self, v: u32) -> &mut Self { self.0.extend_from_slice(&v.to_be_bytes()); self }
    pub fn u64(&mut self, v: u64) -> &mut Self { self.0.extend_from_slice(&v.to_be_bytes()); self }
    pub fn raw(&mut self, b: &[u8]) -> &mut Self { self.0.extend_from_slice(b); self }
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self { self.u32(b.len() as u32).raw(b) }
    pub fn str(&mut self, s: &str) -> &mut Self { self.bytes(s.as_bytes()) }
}

pub struct R<'a>(pub &'a [u8]);

impl<'a> R<'a> {
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (a, b) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = b;
        Some(a)
    }
    pub fn u8(&mut self) -> Option<u8> { Some(self.take(1)?[0]) }
    pub fn u32(&mut self) -> Option<u32> { Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?)) }
    pub fn u64(&mut self) -> Option<u64> { Some(u64::from_be_bytes(self.take(8)?.try_into().ok()?)) }
    pub fn arr<const N: usize>(&mut self) -> Option<[u8; N]> { self.take(N)?.try_into().ok() }
    pub fn bytes(&mut self) -> Option<&'a [u8]> { let n = self.u32()? as usize; self.take(n) }
    pub fn str(&mut self) -> Option<String> { String::from_utf8(self.bytes()?.to_vec()).ok() }
}

/// Membership of a group, as this device knows it.
#[derive(Clone)]
pub struct Member {
    pub group: [u8; 16],
    pub secret: [u8; 32],
    pub keys: GroupKeys,
    /// The epoch before, kept to open items sealed just before a re-key.
    pub prev: Option<([u8; 32], GroupKeys)>,
    /// Made the group: registers with `create`, not a voucher.
    pub creator: bool,
    /// The server knows this device.
    pub registered: bool,
    pub voucher: Vec<u8>,
}

impl Member {
    pub fn new(group: [u8; 16], secret: [u8; 32], epoch: u32, creator: bool, voucher: Vec<u8>) -> Member {
        Member { group, secret, keys: GroupKeys::new(group, &secret, epoch), prev: None, creator, registered: false, voucher }
    }

    /// The keys for an envelope's epoch: this one or the one before.
    pub fn keys_for(&self, epoch: u32) -> Option<&GroupKeys> {
        if epoch == self.keys.epoch {
            return Some(&self.keys);
        }
        self.prev.as_ref().map(|(_, k)| k).filter(|k| k.epoch == epoch)
    }
}

/// The secret blob: the device's keys, the local key, and the group.
pub struct Keys {
    pub me: DeviceKeys,
    pub local: [u8; 32],
    pub member: Option<Member>,
}

const MAGIC: &[u8; 4] = b"KSK1";

impl Keys {
    pub fn new() -> Keys {
        Keys { me: DeviceKeys::new(), local: random(), member: None }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.raw(MAGIC).raw(&self.me.sign).raw(&self.me.boxed).raw(&self.local);
        match &self.member {
            None => { w.u8(0); }
            Some(m) => {
                w.u8(1).raw(&m.group).raw(&m.secret).u32(m.keys.epoch).u8(m.creator as u8).u8(m.registered as u8).bytes(&m.voucher);
                match &m.prev {
                    Some((s, k)) => { w.u8(1).raw(s).u32(k.epoch); }
                    None => { w.u8(0); }
                }
            }
        }
        w.0
    }

    pub fn decode(b: &[u8]) -> Option<Keys> {
        let mut r = R(b);
        if r.take(4)? != MAGIC {
            return None;
        }
        let me = DeviceKeys { sign: r.arr()?, boxed: r.arr()? };
        let local = r.arr()?;
        let member = if r.u8()? == 1 {
            let (group, secret, epoch) = (r.arr()?, r.arr()?, r.u32()?);
            let (creator, registered) = (r.u8()? == 1, r.u8()? == 1);
            let voucher = r.bytes()?.to_vec();
            let mut m = Member::new(group, secret, epoch, creator, voucher);
            m.registered = registered;
            if r.u8()? == 1 {
                let (s, e): ([u8; 32], u32) = (r.arr()?, r.u32()?);
                m.prev = Some((s, GroupKeys::new(group, &s, e)));
            }
            Some(m)
        } else {
            None
        };
        Some(Keys { me, local, member })
    }
}

impl Default for Keys {
    fn default() -> Self {
        Keys::new()
    }
}

/// Seals state with the local key, for the data folder.
pub fn seal_state(local: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let mut out = b"KSS1".to_vec();
    out.extend(seal(local, b"kolada/v1/state", plain));
    out
}

pub fn open_state(local: &[u8; 32], b: &[u8]) -> Option<Vec<u8>> {
    if b.get(..4)? != b"KSS1" {
        return None;
    }
    open(local, b"kolada/v1/state", &b[4..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        let mut k = Keys::new();
        let back = Keys::decode(&k.encode()).unwrap();
        assert_eq!(back.me.sign, k.me.sign);
        assert!(back.member.is_none());
        let mut m = Member::new([1; 16], [2; 32], 3, true, vec![9; 82]);
        m.registered = true;
        m.prev = Some(([4; 32], GroupKeys::new([1; 16], &[4; 32], 2)));
        k.member = Some(m);
        let back = Keys::decode(&k.encode()).unwrap();
        let m = back.member.unwrap();
        assert_eq!((m.group, m.secret, m.keys.epoch, m.creator, m.registered), ([1; 16], [2; 32], 3, true, true));
        assert_eq!(m.voucher, vec![9; 82]);
        assert_eq!(m.keys_for(2).unwrap().send, GroupKeys::new([1; 16], &[4; 32], 2).send);
        assert!(m.keys_for(1).is_none());
    }

    #[test]
    fn state_opens_only_with_its_key() {
        let s = seal_state(&[1; 32], b"cards");
        assert_eq!(open_state(&[1; 32], &s).unwrap(), b"cards");
        assert!(open_state(&[2; 32], &s).is_none());
    }
}
