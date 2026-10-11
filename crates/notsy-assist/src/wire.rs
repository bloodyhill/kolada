//! What passes between Notsy and the model's process: length-prefixed
//! frames over its standard input and output.
//!
//! A frame is a little-endian `u32` length, then the body: numbers are
//! little-endian, strings a `u32` length and UTF-8.

use std::io::{self, Read, Write};

/// Frames larger than this are refused (a paragraph or a note's opening is
/// far smaller).
pub const MAX_FRAME: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Spelling, grammar and punctuation of one paragraph.
    Fix,
    /// Whether one paragraph is a list written in a run.
    Shape,
    /// One line saying what the note is about.
    Gist,
    /// Drop every job that is not for note generation `note`.
    Cancel,
}

impl Kind {
    fn byte(self) -> u8 {
        match self {
            Kind::Fix => 1,
            Kind::Shape => 2,
            Kind::Gist => 3,
            Kind::Cancel => 4,
        }
    }

    fn from_byte(b: u8) -> Option<Kind> {
        Some(match b {
            1 => Kind::Fix,
            2 => Kind::Shape,
            3 => Kind::Gist,
            4 => Kind::Cancel,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub kind: Kind,
    pub id: u64,
    /// The open note's generation: a new one each time a note is opened.
    pub note: u64,
    /// Where in the note: a newer job for the same kind and slot replaces
    /// an older one.
    pub slot: u64,
    /// Read, never rewritten: the title and the paragraph before.
    pub context: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub id: u64,
    pub note: u64,
    pub status: Status,
    pub text: String,
    /// How long the model took, for the eval and the log.
    pub ms: u32,
}

struct Out(Vec<u8>);

impl Out {
    fn new() -> Out {
        Out(vec![0; 4])
    }

    fn u8(&mut self, v: u8) -> &mut Out {
        self.0.push(v);
        self
    }

    fn u32(&mut self, v: u32) -> &mut Out {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn u64(&mut self, v: u64) -> &mut Out {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn str(&mut self, s: &str) -> &mut Out {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
        self
    }

    fn frame(&mut self) -> Vec<u8> {
        let len = (self.0.len() - 4) as u32;
        self.0[..4].copy_from_slice(&len.to_le_bytes());
        std::mem::take(&mut self.0)
    }
}

struct In<'a>(&'a [u8]);

impl In<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn str(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

impl Request {
    /// The request as a whole frame, its length first.
    pub fn frame(&self) -> Vec<u8> {
        Out::new().u8(self.kind.byte()).u64(self.id).u64(self.note).u64(self.slot).str(&self.context).str(&self.text).frame()
    }

    /// A frame's body (see `read_frame`).
    pub fn parse(body: &[u8]) -> Option<Request> {
        let mut i = In(body);
        let r = Request { kind: Kind::from_byte(i.u8()?)?, id: i.u64()?, note: i.u64()?, slot: i.u64()?, context: i.str()?, text: i.str()? };
        i.0.is_empty().then_some(r)
    }
}

impl Reply {
    pub fn frame(&self) -> Vec<u8> {
        let status = match self.status {
            Status::Done => 0,
            Status::Failed => 1,
            Status::Cancelled => 2,
        };
        Out::new().u64(self.id).u64(self.note).u8(status).str(&self.text).u32(self.ms).frame()
    }

    pub fn parse(body: &[u8]) -> Option<Reply> {
        let mut i = In(body);
        let (id, note) = (i.u64()?, i.u64()?);
        let status = match i.u8()? {
            0 => Status::Done,
            1 => Status::Failed,
            2 => Status::Cancelled,
            _ => return None,
        };
        let r = Reply { id, note, status, text: i.str()?, ms: i.u32()? };
        i.0.is_empty().then_some(r)
    }
}

/// The next frame's body; None at a clean end of the stream.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

pub fn write_frame(w: &mut impl Write, frame: &[u8]) -> io::Result<()> {
    w.write_all(frame)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_replies_round_trip() {
        let req = Request { kind: Kind::Fix, id: 7, note: 3, slot: 120, context: "Title — before".into(), text: "i has went هنا".into() };
        let reply = Reply { id: 7, note: 3, status: Status::Done, text: "I went هنا".into(), ms: 412 };
        let mut stream = Vec::new();
        write_frame(&mut stream, &req.frame()).unwrap();
        write_frame(&mut stream, &reply.frame()).unwrap();
        let mut r = &stream[..];
        assert_eq!(Request::parse(&read_frame(&mut r).unwrap().unwrap()), Some(req));
        assert_eq!(Reply::parse(&read_frame(&mut r).unwrap().unwrap()), Some(reply));
        assert_eq!(read_frame(&mut r).unwrap(), None);
    }

    #[test]
    fn bad_frames_are_refused() {
        let mut body = Request { kind: Kind::Gist, id: 1, note: 1, slot: 0, context: String::new(), text: "x".into() }.frame()[4..].to_vec();
        body.push(0);
        assert_eq!(Request::parse(&body), None);
        assert_eq!(Request::parse(&[9]), None);
        let huge = (MAX_FRAME as u32 + 1).to_le_bytes();
        assert!(read_frame(&mut &huge[..]).is_err());
    }
}
