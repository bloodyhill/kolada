//! PDF's objects and the lexer that reads them: numbers, names, strings,
//! arrays, dictionaries, references and streams (ISO 32000 §7.3).

use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(String),
    Str(Vec<u8>),
    Array(Vec<Obj>),
    Dict(Dict),
    Ref(u32, u16),
    /// A stream: its dictionary and its bytes as stored (still encoded).
    Stream(Dict, Rc<Vec<u8>>),
    /// An operator in a content stream (`Tj`, `BT`...).
    Op(String),
}

/// A dictionary: few entries, looked up in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dict(pub Vec<(String, Obj)>);

impl Dict {
    pub fn get(&self, key: &str) -> Option<&Obj> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

impl Obj {
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Obj::Int(i) => Some(i as f64),
            Obj::Real(r) => Some(r),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Obj::Int(i) => Some(i),
            Obj::Real(r) => Some(r as i64),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&str> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) | Obj::Stream(d, _) => Some(d),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Obj]> {
        match self {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }
}

pub fn is_white(b: u8) -> bool {
    matches!(b, b'\0' | b'\t' | b'\n' | b'\x0C' | b'\r' | b' ')
}

pub fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

/// Reads objects from bytes, from a position.
pub struct Lexer<'a> {
    pub b: &'a [u8],
    pub at: usize,
}

/// Deepest nesting of arrays and dictionaries read: damaged files can nest
/// without end.
const DEEPEST: usize = 64;

impl<'a> Lexer<'a> {
    pub fn new(b: &'a [u8], at: usize) -> Lexer<'a> {
        Lexer { b, at }
    }

    pub fn skip_white(&mut self) {
        while self.at < self.b.len() {
            let c = self.b[self.at];
            if is_white(c) {
                self.at += 1;
            } else if c == b'%' {
                while self.at < self.b.len() && !matches!(self.b[self.at], b'\r' | b'\n') {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
    }

    /// The next bare word (a keyword or number), without reading it.
    pub fn peek_word(&mut self) -> &'a [u8] {
        self.skip_white();
        let start = self.at;
        let mut end = start;
        while end < self.b.len() && !is_white(self.b[end]) && !is_delim(self.b[end]) {
            end += 1;
        }
        &self.b[start..end]
    }

    pub fn word(&mut self) -> &'a [u8] {
        let w = self.peek_word();
        self.at += w.len();
        w
    }

    /// Reads one object; None at the end or at something unreadable.
    /// Integers followed by `G R` are references.
    pub fn object(&mut self) -> Option<Obj> {
        self.object_at(0)
    }

    fn object_at(&mut self, depth: usize) -> Option<Obj> {
        if depth > DEEPEST {
            return None;
        }
        self.skip_white();
        let c = *self.b.get(self.at)?;
        match c {
            b'/' => {
                self.at += 1;
                Some(Obj::Name(self.name()))
            }
            b'(' => {
                self.at += 1;
                Some(Obj::Str(self.literal()))
            }
            b'<' if self.b.get(self.at + 1) == Some(&b'<') => {
                self.at += 2;
                let mut d = Vec::new();
                loop {
                    self.skip_white();
                    match self.b.get(self.at) {
                        None => break,
                        Some(b'>') => {
                            self.at += 2;
                            break;
                        }
                        Some(b'/') => {
                            self.at += 1;
                            let key = self.name();
                            let value = self.object_at(depth + 1).unwrap_or(Obj::Null);
                            d.push((key, value));
                        }
                        // Junk inside a dictionary: skipped.
                        Some(_) => {
                            if self.object_at(depth + 1).is_none() {
                                self.at += 1;
                            }
                        }
                    }
                }
                Some(Obj::Dict(Dict(d)))
            }
            b'<' => {
                self.at += 1;
                Some(Obj::Str(self.hex()))
            }
            b'[' => {
                self.at += 1;
                let mut a = Vec::new();
                loop {
                    self.skip_white();
                    match self.b.get(self.at) {
                        None => break,
                        Some(b']') => {
                            self.at += 1;
                            break;
                        }
                        Some(_) => match self.object_at(depth + 1) {
                            Some(o) => a.push(o),
                            None => self.at += 1,
                        },
                    }
                }
                Some(Obj::Array(a))
            }
            b']' | b'>' | b')' | b'{' | b'}' => {
                self.at += 1;
                Some(Obj::Op(String::from_utf8_lossy(&[c]).into_owned()))
            }
            _ => {
                let w = self.word();
                if w.is_empty() {
                    self.at += 1;
                    return Some(Obj::Op(String::new()));
                }
                if let Some(n) = number(w) {
                    // `num gen R`
                    if let Obj::Int(num) = n {
                        let save = self.at;
                        let gen_word = self.word();
                        if let Some(Obj::Int(generation)) = number(gen_word)
                            && self.word() == b"R"
                            && num >= 0
                        {
                            return Some(Obj::Ref(num as u32, generation.clamp(0, 65535) as u16));
                        }
                        self.at = save;
                    }
                    return Some(n);
                }
                Some(match w {
                    b"true" => Obj::Bool(true),
                    b"false" => Obj::Bool(false),
                    b"null" => Obj::Null,
                    _ => Obj::Op(String::from_utf8_lossy(w).into_owned()),
                })
            }
        }
    }

    fn name(&mut self) -> String {
        let mut out = Vec::new();
        while let Some(&c) = self.b.get(self.at) {
            if is_white(c) || is_delim(c) {
                break;
            }
            if c == b'#'
                && let Some(v) = self.b.get(self.at + 1..self.at + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                self.at += 3;
                continue;
            }
            out.push(c);
            self.at += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn literal(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1;
        while let Some(&c) = self.b.get(self.at) {
            self.at += 1;
            match c {
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                b'\\' => {
                    let Some(&e) = self.b.get(self.at) else { break };
                    self.at += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'\r' => {
                            if self.b.get(self.at) == Some(&b'\n') {
                                self.at += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.b.get(self.at) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.at += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        other => out.push(other),
                    }
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn hex(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut half: Option<u8> = None;
        while let Some(&c) = self.b.get(self.at) {
            self.at += 1;
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
}

/// A number, if the word is one.
pub fn number(w: &[u8]) -> Option<Obj> {
    let s = std::str::from_utf8(w).ok()?;
    let first = *w.first()?;
    if !(first.is_ascii_digit() || matches!(first, b'+' | b'-' | b'.')) {
        return None;
    }
    if !s.contains('.') {
        // Some writers emit numbers like "--5"; leniently, their sign.
        let t = s.trim_start_matches(['+', '-']);
        let neg = s.starts_with('-');
        let v: i64 = t.parse().ok()?;
        return Some(Obj::Int(if neg { -v } else { v }));
    }
    let t = s.trim_start_matches('+');
    let v: f64 = t.parse().ok().or_else(|| t.replacen("--", "-", 1).parse().ok())?;
    Some(Obj::Real(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_object() {
        let src = br"<< /Type /Page /Kids [3 0 R 4 0 R] /N -12 /R .5 /S (a\(b\) \101\n) /H <48656C6c6F> /Na /A#20B /T true /Z null >>";
        let o = Lexer::new(src, 0).object().unwrap();
        let d = o.as_dict().unwrap();
        assert_eq!(d.get("Type"), Some(&Obj::Name("Page".into())));
        assert_eq!(d.get("Kids"), Some(&Obj::Array(vec![Obj::Ref(3, 0), Obj::Ref(4, 0)])));
        assert_eq!(d.get("N"), Some(&Obj::Int(-12)));
        assert_eq!(d.get("R"), Some(&Obj::Real(0.5)));
        assert_eq!(d.get("S"), Some(&Obj::Str(b"a(b) A\n".to_vec())));
        assert_eq!(d.get("H"), Some(&Obj::Str(b"Hello".to_vec())));
        assert_eq!(d.get("Na"), Some(&Obj::Name("A B".into())));
        assert_eq!(d.get("T"), Some(&Obj::Bool(true)));
        assert_eq!(d.get("Z"), Some(&Obj::Null));
    }

    #[test]
    fn junk_ends_without_a_panic() {
        for junk in [&b"<< /A [1 2 << /B"[..], b"((((", b"<<<<<<", b"[[[[[[", b"/#zz 1 2", b")", b"%only a comment"] {
            let mut l = Lexer::new(junk, 0);
            while l.at < junk.len() && l.object().is_some() {}
        }
        let deep = "[".repeat(10_000);
        let _ = Lexer::new(deep.as_bytes(), 0).object();
    }
}
