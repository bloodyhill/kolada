//! JSON, read as exports write it (Google Keep's notes, Bear's `info.json`):
//! objects, arrays, strings with their escapes, numbers kept as written,
//! `true`, `false`, `null`. Nesting deeper than any export goes is refused.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// A number as written, read as a number when asked.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The value at `key`, when this is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            // A key written twice: the last, as JSON readers take it.
            Value::Object(fields) => fields.iter().rfind(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// A whole number: one written as such, or as a string of digits.
    pub fn int(&self) -> Option<i64> {
        match self {
            Value::Number(n) | Value::String(n) => n.trim().parse::<i64>().ok().or_else(|| n.trim().parse::<f64>().ok().filter(|f| f.is_finite()).map(|f| f as i64)),
            _ => None,
        }
    }

    /// True for `true`, a number other than 0, or "true"/"1".
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Number(n) => n.trim().parse::<f64>().is_ok_and(|f| f != 0.0),
            Value::String(s) => matches!(s.as_str(), "true" | "1"),
            _ => false,
        }
    }

    pub fn array(&self) -> &[Value] {
        match self {
            Value::Array(items) => items,
            _ => &[],
        }
    }
}

/// Reads one JSON value (a byte-order mark before it is skipped).
pub fn parse(text: &str) -> Option<Value> {
    let mut p = Parser { s: text.trim_start_matches('\u{feff}').as_bytes(), at: 0, depth: 0 };
    let v = p.value()?;
    p.space();
    (p.at == p.s.len()).then_some(v)
}

/// Deeper than this, a file is not an export's.
const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.s.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, word: &str) -> bool {
        let ok = self.s[self.at..].starts_with(word.as_bytes());
        if ok {
            self.at += word.len();
        }
        ok
    }

    fn value(&mut self) -> Option<Value> {
        self.space();
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let v = match *self.s.get(self.at)? {
            b'{' => {
                self.at += 1;
                let mut fields = Vec::new();
                self.space();
                if !self.eat("}") {
                    loop {
                        self.space();
                        let key = self.string()?;
                        self.space();
                        if !self.eat(":") {
                            return None;
                        }
                        fields.push((key, self.value()?));
                        self.space();
                        if self.eat("}") {
                            break;
                        }
                        if !self.eat(",") {
                            return None;
                        }
                    }
                }
                Value::Object(fields)
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if !self.eat("]") {
                    loop {
                        items.push(self.value()?);
                        self.space();
                        if self.eat("]") {
                            break;
                        }
                        if !self.eat(",") {
                            return None;
                        }
                    }
                }
                Value::Array(items)
            }
            b'"' => Value::String(self.string()?),
            b't' if self.eat("true") => Value::Bool(true),
            b'f' if self.eat("false") => Value::Bool(false),
            b'n' if self.eat("null") => Value::Null,
            b'-' | b'0'..=b'9' => {
                let start = self.at;
                while self.s.get(self.at).is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E')) {
                    self.at += 1;
                }
                Value::Number(String::from_utf8_lossy(&self.s[start..self.at]).into_owned())
            }
            _ => return None,
        };
        self.depth -= 1;
        Some(v)
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat("\"") {
            return None;
        }
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.s.get(self.at)?;
            self.at += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.s.get(self.at)?;
                    self.at += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let mut code = self.hex4()?;
                            // A surrogate pair: two escapes, one character.
                            if (0xD800..0xDC00).contains(&code) && self.s[self.at..].starts_with(b"\\u") {
                                self.at += 2;
                                let low = self.hex4()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
                            out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        Some(String::from_utf8_lossy(&out).into_owned())
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = std::str::from_utf8(self.s.get(self.at..self.at + 4)?).ok()?;
        self.at += 4;
        u32::from_str_radix(h, 16).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_exports_write() {
        let v = parse("\u{feff}{\"title\": \"Caf\\u00e9 \\ud83d\\ude00 \\\"x\\\"\", \"n\": 1600000000000000, \"ok\": true, \"list\": [{\"a\": null}, -1.5e2], \"عربي\": \"مرحبا\"}").unwrap();
        assert_eq!(v.get("title").and_then(Value::str), Some("Café 😀 \"x\""));
        assert_eq!(v.get("n").and_then(Value::int), Some(1_600_000_000_000_000));
        assert!(v.get("ok").unwrap().truthy());
        assert_eq!(v.get("list").unwrap().array().len(), 2);
        assert_eq!(v.get("list").unwrap().array()[1].int(), Some(-150));
        assert_eq!(v.get("عربي").and_then(Value::str), Some("مرحبا"));
    }

    #[test]
    fn refuses_what_is_not_json() {
        for bad in ["", "{", "{\"a\" 1}", "[1,]", "tru", "{} x", "\"open"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert_eq!(parse(&"[".repeat(10_000)), None, "too deep to be an export");
    }
}
