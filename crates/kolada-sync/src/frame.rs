//! The wire format shared with the server (services/kolada-sync/src/wire.js).
//!
//! A frame is `u16 header length (big-endian) | header | body`. The header is
//! `key=value` lines, the house style for talking to Kolada's servers (no
//! JSON library): values hold no newline, lists are comma-separated, ids are
//! hex and keys base64url. The body is sealed bytes, or empty.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub fields: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub const MAX_HEADER: usize = 4096;

impl Frame {
    pub fn new(t: &str) -> Frame {
        Frame { fields: vec![("t".into(), t.into())], body: Vec::new() }
    }

    pub fn with(mut self, k: &str, v: impl ToString) -> Frame {
        self.fields.push((k.into(), v.to_string().replace(['\r', '\n'], " ")));
        self
    }

    pub fn body(mut self, b: Vec<u8>) -> Frame {
        self.body = b;
        self
    }

    pub fn t(&self) -> &str {
        self.get("t").unwrap_or("")
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.fields.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
    }

    pub fn num(&self, k: &str) -> Option<u64> {
        self.get(k)?.parse().ok()
    }

    pub fn list(&self, k: &str) -> Vec<&str> {
        self.get(k).map(|v| v.split(',').filter(|s| !s.is_empty()).collect()).unwrap_or_default()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut h = String::new();
        for (k, v) in &self.fields {
            h.push_str(k);
            h.push('=');
            h.push_str(v);
            h.push('\n');
        }
        let mut out = Vec::with_capacity(2 + h.len() + self.body.len());
        out.extend_from_slice(&(h.len() as u16).to_be_bytes());
        out.extend_from_slice(h.as_bytes());
        out.extend_from_slice(&self.body);
        out
    }

    pub fn parse(b: &[u8]) -> Option<Frame> {
        let n = u16::from_be_bytes(b.get(..2)?.try_into().ok()?) as usize;
        if n > MAX_HEADER {
            return None;
        }
        let head = std::str::from_utf8(b.get(2..2 + n)?).ok()?;
        let mut fields = Vec::new();
        for line in head.split('\n').filter(|l| !l.is_empty()) {
            let (k, v) = line.split_once('=')?;
            if k.is_empty() {
                return None;
            }
            fields.push((k.to_string(), v.to_string()));
        }
        let f = Frame { fields, body: b[2 + n..].to_vec() };
        (!f.t().is_empty()).then_some(f)
    }
}

/// A `key=value` lines body, as the server answers plain requests.
pub fn kv(body: &str) -> Vec<(String, String)> {
    body.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_keeps_newlines_out_of_values() {
        let f = Frame::new("send").with("id", "ab").with("note", "two\nlines").body(vec![1, 2, 3]);
        let g = Frame::parse(&f.encode()).unwrap();
        assert_eq!(g.t(), "send");
        assert_eq!(g.get("note"), Some("two lines"));
        assert_eq!(g.body, vec![1, 2, 3]);
    }

    #[test]
    fn reads_what_the_server_writes() {
        // services/kolada-sync/src/wire.js: frame({ t: 'gone', ids: ['a', 'b'] })
        let bytes = [&[0u8, 15][..], b"t=gone\nids=a,b\n"].concat();
        let f = Frame::parse(&bytes).unwrap();
        assert_eq!(f.list("ids"), vec!["a", "b"]);
        assert!(f.body.is_empty());
    }

    #[test]
    fn refuses_malformed_frames() {
        assert!(Frame::parse(&[0]).is_none());
        assert!(Frame::parse(&[0, 9, b'x']).is_none());
        assert!(Frame::parse(&[&[0u8, 4][..], b"x=1\n"].concat()).is_none(), "no type");
        assert!(Frame::parse(&[&[0u8, 4][..], b"=1x\n"].concat()).is_none());
    }
}
