//! A PDF font, for reading text: how a string's bytes split into codes,
//! what text each code is, and how wide it is (ISO 32000 §9.6 to §9.10).

use std::collections::HashMap;

use super::file::File;
use super::object::{Dict, Lexer, Obj};
use super::tables;

pub struct Font {
    /// Codes are two bytes (a composite font).
    two_bytes: bool,
    /// What each code is, from the font's ToUnicode map.
    to_unicode: HashMap<u32, String>,
    /// For a simple font: its encoding, code to character (NUL: none).
    encoding: Option<Box<[char; 256]>>,
    /// A composite font with a Unicode CMap: codes are UTF-16.
    ucs2: bool,
    /// Widths, thousandths of an em.
    widths: HashMap<u32, f32>,
    default_width: f32,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
}

impl Font {
    /// A font that knows nothing: codes are bytes, text is Latin-1.
    pub fn unknown() -> Font {
        Font {
            two_bytes: false,
            to_unicode: HashMap::new(),
            encoding: Some(Box::new(tables::WIN)),
            ucs2: false,
            widths: HashMap::new(),
            default_width: 500.0,
            bold: false,
            italic: false,
            mono: false,
        }
    }

    pub fn load(file: &File, d: &Dict) -> Font {
        let mut font = Font::unknown();
        let subtype = d.get("Subtype").and_then(Obj::as_name).unwrap_or("");
        let base = d.get("BaseFont").and_then(Obj::as_name).unwrap_or("");
        // "ABCDEF+Segoe-Bold": the subset tag goes.
        let name = base.split_once('+').map_or(base, |(_, n)| n);
        let mut descriptor = file.entry(d, "FontDescriptor");
        if subtype == "Type0" {
            font.two_bytes = true;
            font.encoding = None;
            let enc = d.get("Encoding").and_then(|e| file.resolve(e));
            match enc.as_deref() {
                Some(Obj::Name(n)) => {
                    font.ucs2 = n.contains("UCS2") || n.contains("UTF16");
                    if n == "OneByteIdentityH" || n == "OneByteIdentityV" {
                        font.two_bytes = false;
                    }
                }
                // An embedded CMap: its code space says how long codes are.
                Some(s @ Obj::Stream(..)) => {
                    if let Some(data) = file.decode(s)
                        && let Some(len) = code_length(&data)
                    {
                        font.two_bytes = len >= 2;
                    }
                }
                _ => {}
            }
            let descendant = file.entry(d, "DescendantFonts").and_then(|a| a.as_array().and_then(|a| a.first().cloned())).and_then(|f| file.resolve(&f));
            if let Some(desc) = descendant.as_deref().and_then(Obj::as_dict) {
                font.default_width = desc.get("DW").and_then(Obj::as_f64).unwrap_or(1000.0) as f32;
                if let Some(w) = file.entry(desc, "W") {
                    font.widths = cid_widths(file, w.as_array().unwrap_or(&[]));
                }
                if descriptor.is_none() {
                    descriptor = file.entry(desc, "FontDescriptor");
                }
            }
        } else {
            let mut table: [char; 256] = match subtype {
                "TrueType" => tables::WIN,
                _ => tables::STD,
            };
            let symbolic = descriptor.as_deref().and_then(Obj::as_dict).and_then(|f| f.get("Flags")).and_then(Obj::as_i64).is_some_and(|f| f & 4 != 0);
            if symbolic {
                table = tables::WIN;
            }
            let enc = d.get("Encoding").and_then(|e| file.resolve(e));
            let base_name = match enc.as_deref() {
                Some(Obj::Name(n)) => Some(n.clone()),
                Some(Obj::Dict(e)) => e.get("BaseEncoding").and_then(Obj::as_name).map(str::to_string),
                _ => None,
            };
            match base_name.as_deref() {
                Some("WinAnsiEncoding") => table = tables::WIN,
                Some("MacRomanEncoding") => table = tables::MAC,
                Some("StandardEncoding") => table = tables::STD,
                Some("PDFDocEncoding") => table = tables::PDF,
                _ => {}
            }
            if let Some(Obj::Dict(e)) = enc.as_deref()
                && let Some(diffs) = e.get("Differences").and_then(|x| file.resolve(x))
            {
                let mut code = 0usize;
                for item in diffs.as_array().unwrap_or(&[]) {
                    match item {
                        Obj::Int(c) => code = (*c).clamp(0, 255) as usize,
                        Obj::Name(n) => {
                            if code < 256 {
                                table[code] = glyph(n).unwrap_or('\0');
                            }
                            code += 1;
                        }
                        _ => {}
                    }
                }
            }
            font.encoding = Some(Box::new(table));
            let first = d.get("FirstChar").and_then(Obj::as_i64).unwrap_or(0).max(0) as u32;
            // A Type3 font's widths are in its own glyph units.
            let scale = match (subtype, file.entry(d, "FontMatrix").as_deref()) {
                ("Type3", Some(Obj::Array(m))) => m.first().and_then(Obj::as_f64).unwrap_or(0.001) as f32 * 1000.0,
                _ => 1.0,
            };
            if let Some(w) = file.entry(d, "Widths") {
                for (i, v) in w.as_array().unwrap_or(&[]).iter().enumerate() {
                    if let Some(v) = file.resolve(v).and_then(|v| v.as_f64()) {
                        font.widths.insert(first + i as u32, v as f32 * scale);
                    }
                }
            }
            let lower = name.to_ascii_lowercase();
            font.default_width = if lower.contains("courier") || lower.contains("mono") {
                600.0
            } else if lower.contains("times") {
                500.0
            } else {
                556.0
            };
        }
        if let Some(desc) = descriptor.as_deref().and_then(Obj::as_dict) {
            if let Some(m) = desc.get("MissingWidth").and_then(Obj::as_f64).filter(|m| *m > 0.0) {
                font.default_width = m as f32;
            }
            let flags = desc.get("Flags").and_then(Obj::as_i64).unwrap_or(0);
            font.italic |= flags & (1 << 6) != 0 || desc.get("ItalicAngle").and_then(Obj::as_f64).is_some_and(|a| a.abs() > 1.0);
            font.bold |= flags & (1 << 18) != 0 || desc.get("FontWeight").and_then(Obj::as_f64).is_some_and(|w| w >= 600.0);
            font.mono |= flags & 1 != 0;
        }
        let lower = name.to_ascii_lowercase();
        font.bold |= ["bold", "black", "heavy", "semibold", "demi"].iter().any(|w| lower.contains(w));
        font.italic |= lower.contains("italic") || lower.contains("oblique");
        font.mono |= ["mono", "courier", "consola", "menlo", "code"].iter().any(|w| lower.contains(w));
        if let Some(stream) = file.entry(d, "ToUnicode")
            && let Some(data) = file.decode(&stream)
        {
            font.to_unicode = to_unicode(&data);
        }
        font
    }

    /// A string's bytes as codes.
    pub fn codes(&self, bytes: &[u8]) -> Vec<u32> {
        if self.two_bytes {
            bytes.chunks(2).map(|c| if c.len() == 2 { (c[0] as u32) << 8 | c[1] as u32 } else { c[0] as u32 }).collect()
        } else {
            bytes.iter().map(|&b| b as u32).collect()
        }
    }

    /// What a code is, if the font says.
    pub fn text(&self, code: u32) -> Option<String> {
        if let Some(t) = self.to_unicode.get(&code) {
            return Some(clean(t));
        }
        if let Some(table) = &self.encoding {
            let c = *table.get(code as usize)?;
            return (c != '\0').then(|| clean(&c.to_string()));
        }
        if self.ucs2 {
            return char::from_u32(code).map(|c| clean(&c.to_string()));
        }
        None
    }

    /// A code's width, thousandths of an em.
    pub fn width(&self, code: u32) -> f32 {
        self.widths.get(&code).copied().unwrap_or(self.default_width)
    }

    pub fn two_bytes(&self) -> bool {
        self.two_bytes
    }
}

/// Text as a note wants it: ligatures as their letters, Arabic presentation
/// forms as the letters they show, no soft hyphens or no-break spaces.
pub fn clean(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
            '\u{00A0}' | '\u{2007}' | '\u{202F}' => out.push(' '),
            '\u{00AD}' | '\u{200B}' | '\u{FEFF}' => {}
            '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFE}' => match tables::ARABIC.binary_search_by_key(&(c as u32 as u16), |&(cp, _)| cp) {
                Ok(i) => out.push_str(tables::ARABIC[i].1),
                Err(_) => out.push(c),
            },
            _ => out.push(c),
        }
    }
    out
}

/// A glyph name's character: Adobe's names, `uniXXXX`, `uXXXX[XX]`, and
/// names with a suffix (`a.sc`) or joined (`f_i`, first letter only).
pub fn glyph(name: &str) -> Option<char> {
    let base = name.split('.').next().unwrap_or(name);
    let base = base.split('_').next().unwrap_or(base);
    if let Ok(i) = tables::GLYPHS.binary_search_by(|&(n, _)| n.cmp(base)) {
        return Some(tables::GLYPHS[i].1);
    }
    let hex = |h: &str| u32::from_str_radix(h, 16).ok().and_then(char::from_u32);
    if let Some(h) = base.strip_prefix("uni").filter(|h| h.len() >= 4) {
        return hex(&h[..4]);
    }
    if let Some(h) = base.strip_prefix('u').filter(|h| (4..=6).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_hexdigit())) {
        return hex(h);
    }
    None
}

/// A composite font's widths: `[c [w w ...]]` and `[c1 c2 w]`.
fn cid_widths(file: &File, w: &[Obj]) -> HashMap<u32, f32> {
    let mut out = HashMap::new();
    let mut i = 0;
    while i < w.len() {
        let Some(first) = file.resolve(&w[i]).and_then(|o| o.as_i64()) else { break };
        match w.get(i + 1).and_then(|o| file.resolve(o)).as_deref() {
            Some(Obj::Array(list)) => {
                for (k, v) in list.iter().enumerate().take(65536) {
                    if let Some(v) = v.as_f64() {
                        out.insert(first as u32 + k as u32, v as f32);
                    }
                }
                i += 2;
            }
            Some(o) => {
                let last = o.as_i64().unwrap_or(first);
                let v = w.get(i + 2).and_then(|o| file.resolve(o)).and_then(|o| o.as_f64()).unwrap_or(1000.0) as f32;
                for c in first..=last.min(first + 65535) {
                    out.insert(c as u32, v);
                }
                i += 3;
            }
            None => break,
        }
    }
    out
}

/// The longest code a CMap's code space ranges allow, in bytes.
fn code_length(cmap: &[u8]) -> Option<usize> {
    let mut lex = Lexer::new(cmap, 0);
    let mut longest = None;
    let mut inside = false;
    while let Some(o) = lex.object() {
        match o {
            Obj::Op(op) if op == "begincodespacerange" => inside = true,
            Obj::Op(op) if op == "endcodespacerange" => inside = false,
            Obj::Str(s) if inside => longest = Some(longest.unwrap_or(0).max(s.len())),
            _ => {}
        }
        if lex.at >= cmap.len() {
            break;
        }
    }
    longest
}

/// A ToUnicode CMap: `bfchar` pairs and `bfrange`s, code to text.
fn to_unicode(cmap: &[u8]) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let mut lex = Lexer::new(cmap, 0);
    let mut operands: Vec<Obj> = Vec::new();
    let code = |s: &[u8]| s.iter().take(4).fold(0u32, |v, &b| v << 8 | b as u32);
    while lex.at < cmap.len() {
        let Some(o) = lex.object() else { break };
        match o {
            Obj::Op(op) if op == "endbfchar" => {
                for pair in operands.chunks(2) {
                    if let [Obj::Str(src), Obj::Str(dst)] = pair {
                        out.insert(code(src), utf16(dst));
                    }
                }
                operands.clear();
            }
            Obj::Op(op) if op == "endbfrange" => {
                for triple in operands.chunks(3) {
                    let [Obj::Str(lo), Obj::Str(hi), dst] = triple else { continue };
                    let (lo, hi) = (code(lo), code(hi));
                    if hi < lo || hi - lo > 65535 {
                        continue;
                    }
                    match dst {
                        Obj::Str(d) => {
                            // The last unit counts up through the range.
                            let mut units: Vec<u16> = d.chunks(2).map(|c| if c.len() == 2 { u16::from_be_bytes([c[0], c[1]]) } else { c[0] as u16 }).collect();
                            for c in lo..=hi {
                                out.insert(c, String::from_utf16_lossy(&units));
                                if let Some(last) = units.last_mut() {
                                    *last = last.wrapping_add(1);
                                }
                            }
                        }
                        Obj::Array(list) => {
                            for (k, d) in list.iter().enumerate() {
                                if let Obj::Str(d) = d {
                                    out.insert(lo + k as u32, utf16(d));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                operands.clear();
            }
            Obj::Op(_) => operands.clear(),
            other => operands.push(other),
        }
    }
    out
}

fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks(2).map(|c| if c.len() == 2 { u16::from_be_bytes([c[0], c[1]]) } else { c[0] as u16 }).collect();
    String::from_utf16_lossy(&units)
}

/// A text string in a document's dictionaries (a title): UTF-16 with a
/// byte-order mark, UTF-8 with one, or PDFDocEncoding.
pub fn text_string(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest);
    }
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    b.iter().map(|&c| tables::PDF[c as usize]).filter(|&c| c != '\0').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cmaps_and_glyph_names() {
        let cmap = b"/CIDInit /ProcSet findresource begin 2 beginbfchar <0003> <0020> <0011> <00660069> endbfchar 1 beginbfrange <0024> <0026> <0041> endbfrange 1 beginbfrange <0030> <0031> [<00E9> <20AC>] endbfrange end";
        let m = to_unicode(cmap);
        assert_eq!(m[&3], " ");
        assert_eq!(m[&0x11], "fi");
        assert_eq!((m[&0x24].as_str(), m[&0x26].as_str()), ("A", "C"));
        assert_eq!((m[&0x30].as_str(), m[&0x31].as_str()), ("é", "€"));
        assert_eq!(glyph("Eacute"), Some('É'));
        assert_eq!(glyph("uni20AC"), Some('€'));
        assert_eq!(glyph("u1F600"), Some('😀'));
        assert_eq!(glyph("a.sc"), Some('a'));
        assert_eq!(glyph("g123"), None);
        assert_eq!(clean("\u{FB01}nd \u{FEB3}\u{FEE0}\u{FE8E}\u{FEE1}"), "find سلام");
        assert_eq!(text_string(b"\xFE\xFF\x00H\x00i"), "Hi");
    }
}
