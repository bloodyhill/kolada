//! Running a page's content stream for its text: every glyph shown, where
//! (its baseline origin on the page), how big, how wide, in which font
//! (ISO 32000 §8.2, §9.3, §9.4).

use std::collections::HashMap;
use std::rc::Rc;

use super::file::File;
use super::font::Font;
use super::object::{Dict, Lexer, Obj};

/// One glyph on a page, in the page's units (points, y up).
#[derive(Clone, Debug)]
pub struct Glyph {
    pub x: f32,
    pub y: f32,
    /// Its advance along the line.
    pub w: f32,
    pub size: f32,
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
}

type Matrix = [f32; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` then `n`.
fn mul(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

#[derive(Clone)]
struct State {
    ctm: Matrix,
    font: Option<Rc<Font>>,
    size: f32,
    char_space: f32,
    word_space: f32,
    scale: f32,
    leading: f32,
    rise: f32,
}

/// Fonts already read, by the object they are.
pub type Fonts = HashMap<u32, Rc<Font>>;

pub struct Reader<'f, 'a> {
    pub file: &'f File<'a>,
    pub fonts: Fonts,
    pub glyphs: Vec<Glyph>,
    /// Small round or square shapes painted (list bullets some writers
    /// draw rather than type): left, bottom, right, top.
    pub dots: Vec<[f32; 4]>,
    /// Form XObjects being run, against forms that draw themselves.
    forms: Vec<u32>,
}

impl<'f, 'a> Reader<'f, 'a> {
    pub fn new(file: &'f File<'a>) -> Reader<'f, 'a> {
        Reader { file, fonts: HashMap::new(), glyphs: Vec::new(), dots: Vec::new(), forms: Vec::new() }
    }

    /// Runs `content` with `resources`, from the matrix `ctm`.
    pub fn run(&mut self, content: &[u8], resources: &Dict, ctm: Matrix) {
        let mut gs = State { ctm, font: None, size: 0.0, char_space: 0.0, word_space: 0.0, scale: 1.0, leading: 0.0, rise: 0.0 };
        let mut saved: Vec<State> = Vec::new();
        let (mut tm, mut tlm) = (IDENTITY, IDENTITY);
        let mut ops: Vec<Obj> = Vec::new();
        // The path being built: its extent on the page.
        let mut path: Option<[f32; 4]> = None;
        let mut lex = Lexer::new(content, 0);
        let num = |ops: &[Obj], i: usize| ops.get(i).and_then(Obj::as_f64).unwrap_or(0.0) as f32;
        while lex.at < content.len() {
            let Some(o) = lex.object() else { break };
            let Obj::Op(op) = o else {
                ops.push(o);
                if ops.len() > 64 {
                    ops.clear();
                }
                continue;
            };
            match op.as_str() {
                "q" => {
                    if saved.len() < 64 {
                        saved.push(gs.clone());
                    }
                }
                "Q" => {
                    if let Some(s) = saved.pop() {
                        gs = s;
                    }
                }
                "cm" if ops.len() >= 6 => {
                    let m = [num(&ops, 0), num(&ops, 1), num(&ops, 2), num(&ops, 3), num(&ops, 4), num(&ops, 5)];
                    gs.ctm = mul(&m, &gs.ctm);
                }
                "BT" => {
                    tm = IDENTITY;
                    tlm = IDENTITY;
                }
                "m" | "l" | "c" | "v" | "y" | "re" => {
                    let mut points: Vec<(f32, f32)> = ops.chunks(2).filter(|p| p.len() == 2).map(|p| (num(p, 0), num(p, 1))).collect();
                    if op == "re" && ops.len() >= 4 {
                        let (x, y, w, h) = (num(&ops, 0), num(&ops, 1), num(&ops, 2), num(&ops, 3));
                        points = vec![(x, y), (x + w, y + h)];
                    }
                    for (x, y) in points {
                        let (px, py) = (x * gs.ctm[0] + y * gs.ctm[2] + gs.ctm[4], x * gs.ctm[1] + y * gs.ctm[3] + gs.ctm[5]);
                        let b = path.get_or_insert([px, py, px, py]);
                        *b = [b[0].min(px), b[1].min(py), b[2].max(px), b[3].max(py)];
                    }
                }
                "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "S" | "s" => {
                    if let Some([x0, y0, x1, y1]) = path.take() {
                        let (w, h) = (x1 - x0, y1 - y0);
                        if (1.0..=7.0).contains(&w) && (1.0..=7.0).contains(&h) && (0.6..=1.6).contains(&(w / h)) && self.dots.len() < 100_000 {
                            self.dots.push([x0, y0, x1, y1]);
                        }
                    }
                }
                "n" => path = None,
                "Tf" if ops.len() >= 2 => {
                    gs.font = ops[0].as_name().map(|n| self.font(resources, n));
                    gs.size = num(&ops, 1);
                }
                "Tc" => gs.char_space = num(&ops, 0),
                "Tw" => gs.word_space = num(&ops, 0),
                "Tz" => gs.scale = num(&ops, 0) / 100.0,
                "TL" => gs.leading = num(&ops, 0),
                "Ts" => gs.rise = num(&ops, 0),
                "Td" | "TD" if ops.len() >= 2 => {
                    let (tx, ty) = (num(&ops, 0), num(&ops, 1));
                    if op == "TD" {
                        gs.leading = -ty;
                    }
                    tlm = mul(&[1.0, 0.0, 0.0, 1.0, tx, ty], &tlm);
                    tm = tlm;
                }
                "Tm" if ops.len() >= 6 => {
                    tlm = [num(&ops, 0), num(&ops, 1), num(&ops, 2), num(&ops, 3), num(&ops, 4), num(&ops, 5)];
                    tm = tlm;
                }
                "T*" => {
                    tlm = mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -gs.leading], &tlm);
                    tm = tlm;
                }
                "Tj" => {
                    if let Some(Obj::Str(s)) = ops.first() {
                        self.show(s, &gs, &mut tm);
                    }
                }
                "'" | "\"" => {
                    if op == "\"" && ops.len() >= 3 {
                        gs.word_space = num(&ops, 0);
                        gs.char_space = num(&ops, 1);
                    }
                    tlm = mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -gs.leading], &tlm);
                    tm = tlm;
                    if let Some(Obj::Str(s)) = ops.last() {
                        self.show(s, &gs, &mut tm);
                    }
                }
                "TJ" => {
                    if let Some(Obj::Array(items)) = ops.first() {
                        for item in items {
                            match item {
                                Obj::Str(s) => self.show(s, &gs, &mut tm),
                                n => {
                                    let adjust = n.as_f64().unwrap_or(0.0) as f32;
                                    let tx = -adjust / 1000.0 * gs.size * gs.scale;
                                    tm = mul(&[1.0, 0.0, 0.0, 1.0, tx, 0.0], &tm);
                                }
                            }
                        }
                    }
                }
                "Do" => {
                    if let Some(name) = ops.first().and_then(Obj::as_name) {
                        self.form(resources, name, gs.ctm);
                    }
                }
                // An inline image: its data is skipped, to its `EI`.
                "BI" => {
                    let data = &content[lex.at..];
                    let id = data.windows(3).position(|w| w[0].is_ascii_whitespace() && &w[1..] == b"ID").map(|k| lex.at + k + 3);
                    if let Some(start) = id {
                        let mut k = start + 1;
                        while k + 2 < content.len() {
                            if content[k].is_ascii_whitespace() && &content[k + 1..k + 3] == b"EI" && content.get(k + 3).is_none_or(|c| c.is_ascii_whitespace()) {
                                break;
                            }
                            k += 1;
                        }
                        lex.at = (k + 3).min(content.len());
                    } else {
                        break;
                    }
                }
                _ => {}
            }
            ops.clear();
        }
    }

    fn font(&mut self, resources: &Dict, name: &str) -> Rc<Font> {
        let fonts = self.file.entry(resources, "Font");
        let entry = fonts.as_deref().and_then(Obj::as_dict).and_then(|f| f.get(name).cloned());
        let key = match &entry {
            Some(Obj::Ref(n, _)) => Some(*n),
            _ => None,
        };
        if let Some(f) = key.and_then(|k| self.fonts.get(&k)) {
            return f.clone();
        }
        let font = match entry.as_ref().and_then(|e| self.file.resolve(e)) {
            Some(d) => match d.as_dict() {
                Some(d) => Font::load(self.file, d),
                None => Font::unknown(),
            },
            None => Font::unknown(),
        };
        let font = Rc::new(font);
        if let Some(k) = key {
            self.fonts.insert(k, font.clone());
        }
        font
    }

    /// Shows a string: a glyph per code, each moving the text matrix on.
    fn show(&mut self, s: &[u8], gs: &State, tm: &mut Matrix) {
        let Some(font) = gs.font.clone() else { return };
        for code in font.codes(s) {
            let w0 = font.width(code) / 1000.0;
            let space = if !font.two_bytes() && code == 32 { gs.word_space } else { 0.0 };
            let tx = (w0 * gs.size + gs.char_space + space) * gs.scale;
            let trm = mul(&[gs.size * gs.scale, 0.0, 0.0, gs.size, 0.0, gs.rise], &mul(tm, &gs.ctm));
            let m = mul(tm, &gs.ctm);
            // Only text running left to right along the page is read: no
            // rotated or mirrored lines.
            let horizontal = m[0] > 0.0 && m[1].abs() <= m[0].abs() * 0.2;
            if horizontal && let Some(text) = font.text(code) {
                let size = trm[3].abs().max(trm[2].hypot(trm[3])).max(0.1);
                self.glyphs.push(Glyph {
                    x: trm[4],
                    y: trm[5],
                    w: tx * m[0],
                    size,
                    text,
                    bold: font.bold,
                    italic: font.italic,
                    mono: font.mono,
                });
            }
            *tm = mul(&[1.0, 0.0, 0.0, 1.0, tx, 0.0], tm);
        }
    }

    /// A form XObject drawn with `Do`: its content, run in its own
    /// resources and matrix.
    fn form(&mut self, resources: &Dict, name: &str, ctm: Matrix) {
        let xobjects = self.file.entry(resources, "XObject");
        let Some(Obj::Ref(n, _)) = xobjects.as_deref().and_then(Obj::as_dict).and_then(|x| x.get(name).cloned()) else { return };
        if self.forms.contains(&n) || self.forms.len() > 8 {
            return;
        }
        let Some(stream) = self.file.get(n) else { return };
        let Obj::Stream(d, _) = stream.as_ref() else { return };
        if d.get("Subtype").and_then(Obj::as_name) != Some("Form") {
            return;
        }
        let Some(data) = self.file.decode(&stream) else { return };
        let own = self.file.entry(d, "Resources").and_then(|r| r.as_dict().cloned());
        let matrix = match self.file.entry(d, "Matrix").as_deref() {
            Some(Obj::Array(m)) if m.len() == 6 => {
                let v: Vec<f32> = m.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
                [v[0], v[1], v[2], v[3], v[4], v[5]]
            }
            _ => IDENTITY,
        };
        self.forms.push(n);
        self.run(&data, own.as_ref().unwrap_or(resources), mul(&matrix, &ctm));
        self.forms.pop();
    }
}

/// The page's links: where each is (left, bottom, right, top) and where it
/// goes.
pub fn links(file: &File, page: &Dict) -> Vec<([f32; 4], String)> {
    let mut out = Vec::new();
    let Some(annots) = file.entry(page, "Annots") else { return out };
    for a in annots.as_array().unwrap_or(&[]) {
        let Some(a) = file.resolve(a) else { continue };
        let Some(d) = a.as_dict() else { continue };
        if d.get("Subtype").and_then(Obj::as_name) != Some("Link") {
            continue;
        }
        let Some(action) = file.entry(d, "A") else { continue };
        let Some(action) = action.as_dict() else { continue };
        if action.get("S").and_then(Obj::as_name) != Some("URI") {
            continue;
        }
        let Some(Obj::Str(uri)) = file.entry(action, "URI").as_deref().cloned() else { continue };
        let Some(rect) = file.entry(d, "Rect") else { continue };
        let r: Vec<f32> = rect.as_array().unwrap_or(&[]).iter().filter_map(|v| v.as_f64()).map(|v| v as f32).collect();
        let uri = String::from_utf8_lossy(&uri).trim().to_string();
        // Links to the web and mail work in a note; to files on the
        // machine the PDF was made on, not.
        let lower = uri.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")) {
            continue;
        }
        if r.len() == 4 {
            out.push(([r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])], uri));
        }
    }
    out
}
