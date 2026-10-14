//! Reading SQLite database files, just enough to import notes from apps
//! that keep them in one: every row of a table, read straight from the
//! file's pages. Read-only; an exported file has no journal or
//! write-ahead log to replay.
//!
//! The format (sqlite.org/fileformat.html): fixed-size pages; each table is
//! a B-tree of pages whose leaves hold rows as records, and a row too long
//! for its page continues on a chain of overflow pages. Page 1 holds the
//! schema table, which says where each table's tree starts and how it was
//! created (its columns).

use std::fmt;

/// One value of a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Value::Int(v) => Some(v),
            Value::Real(v) => Some(v as i64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

fn bad(what: &str) -> Error {
    Error(format!("not a readable SQLite file: {what}"))
}

/// A table's rows: its columns' names, and each row's values in their
/// order.
#[derive(Debug, Default)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl Table {
    /// The index of the column named `name` (any case).
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.eq_ignore_ascii_case(name))
    }
}

/// How text is stored: UTF-8, or UTF-16 little or big endian.
#[derive(Clone, Copy)]
enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

pub struct Database<'a> {
    bytes: &'a [u8],
    page_size: usize,
    /// Bytes of each page in use (some may be reserved at the end).
    usable: usize,
    encoding: Encoding,
}

impl<'a> Database<'a> {
    /// A database from a file's bytes.
    pub fn open(bytes: &'a [u8]) -> Result<Database<'a>> {
        if bytes.len() < 100 || &bytes[..16] != b"SQLite format 3\0" {
            return Err(Error("not a SQLite file".to_string()));
        }
        let page_size = match u16::from_be_bytes([bytes[16], bytes[17]]) {
            1 => 65536,
            n => n as usize,
        };
        if page_size < 512 || !page_size.is_power_of_two() {
            return Err(bad("page size"));
        }
        let usable = page_size - bytes[20] as usize;
        if usable < 480 {
            return Err(bad("reserved space"));
        }
        let encoding = match u32::from_be_bytes([bytes[56], bytes[57], bytes[58], bytes[59]]) {
            2 => Encoding::Utf16Le,
            3 => Encoding::Utf16Be,
            _ => Encoding::Utf8,
        };
        Ok(Database { bytes, page_size, usable, encoding })
    }

    /// Every row of the table named `name` (any case).
    pub fn table(&self, name: &str) -> Result<Table> {
        let schema = self.rows(1)?;
        let found = schema.iter().find(|(_, r)| {
            r.first().and_then(Value::as_str) == Some("table") && r.get(1).and_then(Value::as_str).is_some_and(|n| n.eq_ignore_ascii_case(name))
        });
        let Some((_, entry)) = found else { return Err(Error(format!("no table named {name}"))) };
        let root = entry.get(3).and_then(Value::as_i64).filter(|&p| p > 0).ok_or_else(|| bad("table root"))?;
        let sql = entry.get(4).and_then(Value::as_str).unwrap_or("");
        let (columns, alias) = columns(sql);
        let rows = self
            .rows(root as u32)?
            .into_iter()
            .map(|(rowid, mut values)| {
                values.resize(columns.len().max(values.len()), Value::Null);
                // A column that is the row's id is stored as nothing.
                if let Some(a) = alias.filter(|&a| values.get(a) == Some(&Value::Null)) {
                    values[a] = Value::Int(rowid);
                }
                values
            })
            .collect();
        Ok(Table { columns, rows })
    }

    fn page(&self, n: u32) -> Result<&'a [u8]> {
        let start = (n as usize).checked_sub(1).ok_or_else(|| bad("page 0"))? * self.page_size;
        self.bytes.get(start..start + self.page_size).ok_or_else(|| bad("page past the end"))
    }

    fn pages(&self) -> usize {
        self.bytes.len() / self.page_size
    }

    /// Every row of the table whose tree starts at page `root`, in the
    /// order of their ids: (id, values).
    fn rows(&self, root: u32) -> Result<Vec<(i64, Vec<Value>)>> {
        let mut out = Vec::new();
        let mut visited = 0;
        self.walk(root, &mut out, &mut visited)?;
        Ok(out)
    }

    fn walk(&self, n: u32, out: &mut Vec<(i64, Vec<Value>)>, visited: &mut usize) -> Result<()> {
        // A damaged file could loop: no tree has more pages than the file.
        *visited += 1;
        if *visited > self.pages() {
            return Err(bad("a page tree that loops"));
        }
        let page = self.page(n)?;
        let head = if n == 1 { 100 } else { 0 };
        let at = |i: usize| page.get(i).copied().ok_or_else(|| bad("page header"));
        let cells = u16::from_be_bytes([at(head + 3)?, at(head + 4)?]) as usize;
        let cell = |i: usize, header: usize| -> Result<usize> {
            let p = head + header + 2 * i;
            Ok(u16::from_be_bytes([at(p)?, at(p + 1)?]) as usize)
        };
        match at(head)? {
            // A leaf: rows.
            0x0D => {
                for i in 0..cells {
                    let p = cell(i, 8)?;
                    let (size, a) = varint(page.get(p..).unwrap_or(&[])).ok_or_else(|| bad("cell"))?;
                    let (rowid, b) = varint(page.get(p + a..).unwrap_or(&[])).ok_or_else(|| bad("cell"))?;
                    let payload = self.payload(page, p + a + b, size as usize)?;
                    out.push((rowid as i64, self.record(&payload)?));
                }
            }
            // Inside the tree: children, each holding the ids up to its key,
            // then the rightmost.
            0x05 => {
                for i in 0..cells {
                    let p = cell(i, 12)?;
                    let child = u32::from_be_bytes([at(p)?, at(p + 1)?, at(p + 2)?, at(p + 3)?]);
                    self.walk(child, out, visited)?;
                }
                let right = u32::from_be_bytes([at(head + 8)?, at(head + 9)?, at(head + 10)?, at(head + 11)?]);
                self.walk(right, out, visited)?;
            }
            _ => return Err(bad("not a table page")),
        }
        Ok(())
    }

    /// A row's `size` bytes, starting at `start` on `page` and continuing
    /// on overflow pages if they do not fit there.
    fn payload(&self, page: &[u8], start: usize, size: usize) -> Result<Vec<u8>> {
        let u = self.usable;
        let most = u - 35;
        let local = if size <= most {
            size
        } else {
            let least = (u - 12) * 32 / 255 - 23;
            let k = least + (size - least) % (u - 4);
            if k <= most { k } else { least }
        };
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(page.get(start..start + local).ok_or_else(|| bad("row past its page"))?);
        if local < size {
            let at = start + local;
            let next = page.get(at..at + 4).ok_or_else(|| bad("overflow"))?;
            let mut next = u32::from_be_bytes([next[0], next[1], next[2], next[3]]);
            let mut hops = 0;
            while out.len() < size {
                hops += 1;
                if next == 0 || hops > self.pages() {
                    return Err(bad("overflow chain"));
                }
                let more = self.page(next)?;
                next = u32::from_be_bytes([more[0], more[1], more[2], more[3]]);
                let take = (size - out.len()).min(u - 4);
                out.extend_from_slice(&more[4..4 + take]);
            }
        }
        Ok(out)
    }

    /// A record's values: a header of their types, then the values.
    fn record(&self, bytes: &[u8]) -> Result<Vec<Value>> {
        let (header, mut i) = varint(bytes).ok_or_else(|| bad("record"))?;
        let header = header as usize;
        let mut kinds = Vec::new();
        while i < header {
            let (k, n) = varint(bytes.get(i..header).unwrap_or(&[])).ok_or_else(|| bad("record header"))?;
            kinds.push(k);
            i += n;
        }
        let mut at = header;
        let mut values = Vec::with_capacity(kinds.len());
        for k in kinds {
            let len = match k {
                0 | 8 | 9 => 0,
                1..=4 => k as usize,
                5 => 6,
                6 | 7 => 8,
                10 | 11 => return Err(bad("record type")),
                _ => (k as usize - 12) / 2,
            };
            let v = bytes.get(at..at + len).ok_or_else(|| bad("record past its end"))?;
            at += len;
            values.push(match k {
                0 => Value::Null,
                1..=6 => Value::Int(v.iter().fold(if v[0] & 0x80 != 0 { -1i64 } else { 0 }, |n, &b| n << 8 | b as i64)),
                7 => Value::Real(f64::from_be_bytes(v.try_into().expect("8 bytes"))),
                8 => Value::Int(0),
                9 => Value::Int(1),
                _ if k % 2 == 0 => Value::Blob(v.to_vec()),
                _ => Value::Text(self.text(v)),
            });
        }
        Ok(values)
    }

    fn text(&self, v: &[u8]) -> String {
        match self.encoding {
            Encoding::Utf8 => utf8(v),
            Encoding::Utf16Le => String::from_utf16_lossy(&v.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect::<Vec<_>>()),
            Encoding::Utf16Be => String::from_utf16_lossy(&v.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).collect::<Vec<_>>()),
        }
    }
}

/// UTF-8 as it is, or as near as it reads. Apps that keep text as UTF-16
/// may write a character beyond the first 65,536 as its two surrogates,
/// each as if a character: read as the one it is. A character cut off at
/// the end (text shortened by bytes, or between two surrogates) is dropped;
/// anything else invalid is replaced.
fn utf8(v: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(v) {
        return s.to_string();
    }
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    loop {
        let e = match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                return out;
            }
            Err(e) => e,
        };
        let (good, bad) = rest.split_at(e.valid_up_to());
        out.push_str(std::str::from_utf8(good).expect("valid up to here"));
        let skip = match (surrogate(bad), surrogate(bad.get(3..).unwrap_or(&[]))) {
            (Some(hi @ 0xD800..=0xDBFF), Some(lo @ 0xDC00..=0xDFFF)) => {
                out.extend(char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)));
                6
            }
            (Some(0xD800..=0xDBFF), None) if bad.len() == 3 => return out,
            (Some(_), _) => {
                out.push('\u{FFFD}');
                3
            }
            (None, _) => match e.error_len() {
                None => return out,
                Some(n) => {
                    out.push('\u{FFFD}');
                    n
                }
            },
        };
        rest = &bad[skip..];
    }
}

/// The UTF-16 surrogate `b` starts with, if it starts with one written as
/// UTF-8.
fn surrogate(b: &[u8]) -> Option<u32> {
    match *b {
        [0xED, b1 @ 0xA0..=0xBF, b2 @ 0x80..=0xBF, ..] => Some(0xD000 | (b1 as u32 & 0x3F) << 6 | (b2 as u32 & 0x3F)),
        _ => None,
    }
}

/// A variable-length integer: up to nine bytes, seven bits each but the
/// ninth, which gives all eight. The value and how many bytes it took.
fn varint(b: &[u8]) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for i in 0..9 {
        let byte = *b.get(i)?;
        if i == 8 {
            return Some((v << 8 | byte as u64, 9));
        }
        v = v << 7 | (byte & 0x7F) as u64;
        if byte & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

/// A table's columns from the statement that created it, and which one (if
/// any) is the row's id: an INTEGER PRIMARY KEY.
fn columns(sql: &str) -> (Vec<String>, Option<usize>) {
    let (Some(open), Some(close)) = (sql.find('('), sql.rfind(')')) else { return (Vec::new(), None) };
    let (mut columns, mut integer) = (Vec::new(), Vec::new());
    let (mut alias, mut keyed) = (None, None);
    for part in split_top(&sql[open + 1..close]) {
        let part = part.trim();
        let (name, rest) = first_word(part);
        let rest = rest.to_ascii_uppercase();
        match name.to_ascii_uppercase().as_str() {
            "CONSTRAINT" | "UNIQUE" | "CHECK" | "FOREIGN" => {}
            // A table constraint: the key, if it is one column.
            "PRIMARY" => {
                let inner = part.find('(').zip(part.rfind(')')).map(|(a, b)| split_top(&part[a + 1..b])).unwrap_or_default();
                if let [one] = inner.as_slice() {
                    keyed = Some(first_word(one.trim()).0.to_string());
                }
            }
            _ => {
                let int = rest.trim_start().starts_with("INTEGER");
                if int && rest.contains("PRIMARY KEY") && !rest.contains("DESC") {
                    alias = Some(columns.len());
                }
                integer.push(int);
                columns.push(name.to_string());
            }
        }
    }
    if let Some(i) = keyed.and_then(|k| columns.iter().position(|c| c.eq_ignore_ascii_case(&k))).filter(|&i| integer[i]) {
        alias = Some(i);
    }
    (columns, alias)
}

/// `s` split at commas outside parentheses and quotes.
fn split_top(s: &str) -> Vec<&str> {
    let (mut parts, mut depth, mut quote, mut from) = (Vec::new(), 0i32, None::<char>, 0);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q || (q == '[' && c == ']') => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'' | '`' | '[') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                parts.push(&s[from..i]);
                from = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[from..]);
    parts
}

/// A definition's first word (a name, unquoted) and what follows it.
fn first_word(s: &str) -> (&str, &str) {
    let close = match s.chars().next() {
        Some('"') => Some('"'),
        Some('`') => Some('`'),
        Some('[') => Some(']'),
        Some('\'') => Some('\''),
        _ => None,
    };
    match close {
        Some(c) => match s[1..].find(c) {
            Some(end) => (&s[1..1 + end], &s[2 + end..]),
            None => (&s[1..], ""),
        },
        None => {
            let end = s.find(|c: char| c.is_whitespace() || c == '(').unwrap_or(s.len());
            (&s[..end], &s[end..])
        }
    }
}

#[cfg(test)]
mod tests;
