//! Math drawn: TeX read by `kolada_markdown::math` set as TeX sets it
//! (scripts, fractions, roots, big operators, delimiters, arrays), in the
//! note's own face and ink. Letters and symbols are glyphs of the note's
//! font; fraction bars and rules are fills; roots and tall brackets are
//! shapes filled from outlines (see `ShapeMark`). Nothing here runs for a
//! note without math, and a formula is set once and kept while it shows.
//!
//! Sizes follow TeX's parameters for its math fonts, in ems of the size a
//! piece is set at; a letter's height and depth come from what kind of
//! letter it is (x-height, ascender, descender), the same on every OS.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use kolada_markdown::math::{self as tex, Accent, ArrayKind, Class, ColAlign, Delim, Font, MathStyle, Node};
use kolada_text::{Format, Layout, Role};

use crate::scene::{self, Eng, Fill, Ink, Rect, Scene, ShapeMark, Text};

/// A formula set: its width, how far it reaches above and below its
/// baseline, and what it draws, in DIPs from its left end on its baseline
/// (y down).
pub struct Formula {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    items: Vec<Item>,
}

#[derive(Clone)]
enum Item {
    /// Glyphs: their layout, where its baseline starts, the baseline's
    /// depth in the layout, and whether they are an error.
    Glyphs { x: f32, y: f32, layout: Rc<Layout>, top: f32, error: bool },
    /// A rule: left, top, right, bottom.
    Bar([f32; 4]),
    /// A shape: the rectangle it fills, its polygons from that
    /// rectangle's top left.
    Shape { rect: [f32; 4], polygons: Polygons },
    /// A soft panel behind (`\boxed`).
    Panel([f32; 4]),
}

impl Item {
    fn moved(mut self, dx: f32, dy: f32) -> Item {
        match &mut self {
            Item::Glyphs { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            Item::Bar(r) | Item::Shape { rect: r, .. } | Item::Panel(r) => {
                r[0] += dx;
                r[2] += dx;
                r[1] += dy;
                r[3] += dy;
            }
        }
        self
    }
}

impl Formula {
    /// Draws it with its baseline's left end at `x`, `baseline`, in `ink`
    /// (errors in the error ink), at opacity `alpha`.
    pub fn draw(&self, x: f32, baseline: f32, ink: Ink, alpha: f32, scene: &mut Scene) {
        for item in &self.items {
            if let Item::Panel([l, t, r, b]) = *item {
                scene.fills.push(Fill::faded(Rect::new(x + l, baseline + t, x + r, baseline + b), 5.0, Ink::Field, alpha));
            }
        }
        for item in &self.items {
            match item {
                Item::Glyphs { x: gx, y, layout, top, error } => {
                    scene.texts.push(Text { x: x + gx, y: baseline + y - top, layout: layout.clone(), ink: if *error { Ink::Danger } else { ink }, alpha });
                }
                Item::Bar([l, t, r, b]) => scene.fills.push(Fill::faded(Rect::new(x + l, baseline + t, x + r, baseline + b), 0.0, ink, alpha)),
                Item::Shape { rect: [l, t, r, b], polygons } => {
                    let layer = scene.layer_now();
                    scene.shapes.push(ShapeMark { layer, rect: Rect::new(x + l, baseline + t, x + r, baseline + b), polygons: polygons.clone(), ink, alpha });
                }
                Item::Panel(_) => {}
            }
        }
    }

    /// Whether it has anything it could not read.
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|i| matches!(i, Item::Glyphs { error: true, .. }))
    }
}

/// TeX's math font parameters (σ and ξ), in ems.
const X_HEIGHT: f32 = 0.46;
const AXIS: f32 = 0.26;
const NUM1: f32 = 0.677;
const NUM2: f32 = 0.394;
const NUM3: f32 = 0.444;
const DENOM1: f32 = 0.686;
const DENOM2: f32 = 0.345;
const SUP1: f32 = 0.413;
const SUP2: f32 = 0.363;
const SUP3: f32 = 0.289;
const SUB1: f32 = 0.15;
const SUB2: f32 = 0.247;
const SUP_DROP: f32 = 0.386;
const SUB_DROP: f32 = 0.05;
const BIG_OP_SPACING1: f32 = 0.111;
const BIG_OP_SPACING2: f32 = 0.166;
const BIG_OP_SPACING3: f32 = 0.2;
const BIG_OP_SPACING4: f32 = 0.6;
const BIG_OP_SPACING5: f32 = 0.1;
/// Room either side of a fraction, and the space after scripts.
const NULL_DELIM: f32 = 0.12;
const SCRIPT_SPACE: f32 = 0.05;
/// An array's struts: the least a row reaches above and below its
/// baseline.
const STRUT_HEIGHT: f32 = 0.84;
const STRUT_DEPTH: f32 = 0.36;

/// A style, and whether it is cramped (superscripts kept low: under a
/// bar, in a root, in a subscript).
#[derive(Clone, Copy, Debug, PartialEq)]
struct St {
    style: MathStyle,
    cramped: bool,
}

impl St {
    fn scale(self) -> f32 {
        match self.style {
            MathStyle::Display | MathStyle::Text => 1.0,
            MathStyle::Script => 0.7,
            MathStyle::ScriptScript => 0.5,
        }
    }

    fn display(self) -> bool {
        self.style == MathStyle::Display
    }

    fn smaller(self) -> MathStyle {
        match self.style {
            MathStyle::Display | MathStyle::Text => MathStyle::Script,
            _ => MathStyle::ScriptScript,
        }
    }

    fn sup(self) -> St {
        St { style: self.smaller(), cramped: self.cramped }
    }

    fn sub(self) -> St {
        St { style: self.smaller(), cramped: true }
    }

    fn num(self) -> St {
        let style = match self.style {
            MathStyle::Display => MathStyle::Text,
            MathStyle::Text => MathStyle::Script,
            _ => MathStyle::ScriptScript,
        };
        St { style, cramped: self.cramped }
    }

    fn den(self) -> St {
        St { cramped: true, ..self.num() }
    }

    fn cramp(self) -> St {
        St { cramped: true, ..self }
    }
}

/// A box being set: width, height above the baseline, depth below, what
/// it draws (from its left end on the baseline), and its italic
/// correction (how far its last letter leans past its width).
#[derive(Clone, Default)]
struct MBox {
    w: f32,
    h: f32,
    d: f32,
    items: Vec<Item>,
    italic: f32,
}

impl MBox {
    fn empty(w: f32) -> MBox {
        MBox { w, ..MBox::default() }
    }

    /// Puts `other` in this box at `dx`, `dy` (down), growing it to hold
    /// it.
    fn put(&mut self, other: MBox, dx: f32, dy: f32) {
        self.h = self.h.max(other.h - dy);
        self.d = self.d.max(other.d + dy);
        self.w = self.w.max(dx + other.w);
        self.items.extend(other.items.into_iter().map(|i| i.moved(dx, dy)));
    }
}

/// Sets math in the note's text face.
struct Setter<'a> {
    eng: &'a Eng,
    /// The size of text style, DIPs.
    size: f32,
    /// The height and depth a `\middle` delimiter is to reach, while the
    /// list between `\left` and `\right` is being set.
    fence: Option<(f32, f32)>,
}

/// Kept formulas: by what they are, with the frame each was last used.
#[derive(Default)]
struct Kept {
    frame: u64,
    formulas: HashMap<KeptKey, (String, Rc<Formula>, u64)>,
}

/// A kept formula's source hash, size, style and font generation.
type KeptKey = (u64, u32, bool, u32);

thread_local! {
    static KEPT: RefCell<Kept> = RefCell::new(Kept::default());
}

/// A new frame: formulas no frame has drawn for a while are let go.
pub fn next_frame() {
    KEPT.with(|k| {
        let mut k = k.borrow_mut();
        k.frame += 1;
        let frame = k.frame;
        if !k.formulas.is_empty() {
            k.formulas.retain(|_, f| f.2 + 30 >= frame);
        }
    });
}

/// `src` set as math at `size` (text style's, DIPs): as a block's, in
/// display style, or in a line's text style. Kept, so a formula on
/// screen is set once.
pub fn formula(eng: &Eng, src: &str, size: f32, display: bool) -> Rc<Formula> {
    let generation = eng.borrow().generation();
    let key = (scene::text_hash(src), size.to_bits(), display, generation);
    let found = KEPT.with(|k| {
        let mut k = k.borrow_mut();
        let frame = k.frame;
        k.formulas.get_mut(&key).filter(|f| f.0 == src).map(|f| {
            f.2 = frame;
            f.1.clone()
        })
    });
    if let Some(f) = found {
        return f;
    }
    let nodes = tex::parse(src);
    let mut setter = Setter { eng, size, fence: None };
    let st = St { style: if display { MathStyle::Display } else { MathStyle::Text }, cramped: false };
    let b = setter.list(&nodes, st);
    let f = Rc::new(Formula { width: b.w.max(0.0), ascent: b.h.max(0.0), descent: b.d.max(0.0), items: b.items });
    KEPT.with(|k| {
        let mut k = k.borrow_mut();
        let frame = k.frame;
        k.formulas.insert(key, (src.to_string(), f.clone(), frame));
    });
    f
}

/// The room TeX puts between atoms of two classes, in eighteenths of an
/// em; the spaces in parentheses in TeX's table only in display and text
/// style.
fn gap(left: Class, right: Class, st: St) -> f32 {
    use Class::*;
    let (mu, always) = match (left, right) {
        (Ord, Op) | (Op, Ord) | (Op, Op) | (Close, Op) | (Inner, Op) => (3.0, true),
        (Ord, Inner) | (Op, Inner) | (Close, Inner) | (Inner, Ord) | (Inner, Open) | (Inner, Punct) | (Inner, Inner) => (3.0, false),
        (Punct, Ord) | (Punct, Op) | (Punct, Rel) | (Punct, Open) | (Punct, Close) | (Punct, Punct) | (Punct, Inner) => (3.0, false),
        (Ord | Close | Inner, Bin) | (Bin, Ord | Op | Open | Inner) => (4.0, false),
        (Ord | Op | Close | Inner, Rel) | (Rel, Ord | Op | Open | Inner) => (5.0, false),
        _ => (0.0, true),
    };
    if always || matches!(st.style, MathStyle::Display | MathStyle::Text) { mu / 18.0 } else { 0.0 }
}

/// A letter's height and depth in ems, by its kind: what TeX's font
/// metrics say of a math font's letters, close to every text face's.
fn extent(c: char) -> (f32, f32) {
    match c {
        'a' | 'c' | 'e' | 'm' | 'n' | 'o' | 'r' | 's' | 'u' | 'v' | 'w' | 'x' | 'z' => (X_HEIGHT, 0.0),
        'b' | 'd' | 'f' | 'h' | 'k' | 'l' => (0.72, 0.0),
        'i' => (0.68, 0.0),
        't' => (0.62, 0.0),
        'j' => (0.68, 0.21),
        'g' | 'p' | 'q' | 'y' => (X_HEIGHT, 0.21),
        'A'..='Z' | '0'..='9' => (0.70, 0.0),
        'β' | 'ζ' | 'ξ' => (0.72, 0.21),
        'δ' | 'θ' | 'λ' | 'ϑ' | '∂' | 'ℓ' => (0.72, 0.0),
        'γ' | 'η' | 'μ' | 'ρ' | 'φ' | 'χ' | 'ψ' | 'ς' | 'ϕ' | 'ϱ' => (X_HEIGHT, 0.21),
        'α' | 'ε' | 'ϵ' | 'ι' | 'κ' | 'ν' | 'ο' | 'π' | 'σ' | 'τ' | 'υ' | 'ω' | 'ϖ' | '∞' => (X_HEIGHT, 0.0),
        '\u{0391}'..='\u{03A9}' => (0.70, 0.0),
        '(' | ')' | '[' | ']' | '{' | '}' | '|' | '‖' | '⟨' | '⟩' | '⌊' | '⌋' | '⌈' | '⌉' | '/' | '\\' => (0.75, 0.25),
        '+' | '−' | '=' | '±' | '∓' | '×' | '÷' | '<' | '>' | '≤' | '≥' | '≠' | '≈' | '≡' | '∼' | '→' | '←' | '↔' | '⇒' | '⇐' | '⇔' => (0.58, 0.08),
        ',' | ';' => (0.12, 0.18),
        '.' | '⋅' | '∙' | '⋯' | '…' => (0.12, 0.0),
        '\u{2032}' | '\u{2033}' | '\u{2034}' => (0.75, 0.0),
        _ => (0.70, 0.02),
    }
}

/// How far a font's italic leans past a letter's width, in ems.
const ITALIC_LEAN: f32 = 0.06;

/// Glyphs set as the symbol of a big operator, as a share of the size:
/// in display style, and in text style.
fn op_scale(text: &str, display: bool) -> f32 {
    let integral = text.chars().next().is_some_and(|c| matches!(c, '∫' | '∬' | '∭' | '∮'));
    match (integral, display) {
        (true, true) => 2.0,
        (true, false) => 1.35,
        (false, true) => 1.6,
        (false, false) => 1.15,
    }
}

/// A point.
type P = (f32, f32);

/// A shape's outline: polygons, shared with the scenes that draw it.
type Polygons = Rc<Vec<Vec<P>>>;

impl Setter<'_> {
    fn em(&self, st: St) -> f32 {
        self.size * st.scale()
    }

    fn rule(&self, st: St) -> f32 {
        (0.05 * self.em(st)).max(0.8)
    }

    /// A run of glyphs in a face, at a size: as wide as its glyphs, as tall
    /// and deep as its letters.
    fn glyphs(&self, text: &str, font: Font, size: f32, error: bool) -> MBox {
        let (weight, italic, role) = match font {
            Font::Italic => (400, true, Role::Text),
            Font::Upright => (400, false, Role::Text),
            Font::Bold => (700, false, Role::Text),
            Font::BoldItalic => (700, true, Role::Text),
            Font::Mono => (400, false, Role::Code),
        };
        // The baseline sits a size below the layout's top.
        let format = Format { weight, italic, line: Some(size * 1.25), baseline: Some(0.8), wrap: false, rtl: Some(false), ..Format::new(role, size) };
        let layout = scene::kept_layout(self.eng, text, &format, 100_000.0, 100_000.0, MATH_LAYOUT, || scene::layout_styled(self.eng, text, &format, 100_000.0, 100_000.0, &[]));
        let w = layout.measure().width;
        let (mut h, mut d) = (0.0f32, 0.0f32);
        for c in text.chars().filter(|c| !c.is_whitespace()) {
            let (ch, cd) = extent(c);
            h = h.max(ch);
            d = d.max(cd);
        }
        let lean = if italic && text.chars().last().is_some_and(char::is_alphabetic) { ITALIC_LEAN * size } else { 0.0 };
        MBox { w, h: h * size, d: d * size, items: vec![Item::Glyphs { x: 0.0, y: 0.0, layout, top: size, error }], italic: lean }
    }

    /// How far a box's glyphs reach above and below its baseline, as the
    /// face draws them (measured on their pictures at twice the size, so
    /// to a quarter of a pixel's room either way).
    fn ink(&self, b: &MBox) -> Option<(f32, f32)> {
        const SCALE: f32 = 2.0;
        let mut eng = self.eng.borrow_mut();
        let (mut above, mut below) = (f32::MIN, f32::MIN);
        for item in &b.items {
            let Item::Glyphs { y, layout, top, .. } = item else { continue };
            for g in layout.glyphs((0.0, 0.0), SCALE) {
                let Some(m) = eng.mask(&g) else { continue };
                // The glyph's baseline, from the box's.
                let base = g.y as f32 / SCALE - top + y;
                above = above.max(m.top as f32 / SCALE - base);
                below = below.max((m.height as f32 - m.top as f32) / SCALE + base);
            }
        }
        (above > f32::MIN).then_some((above, below))
    }

    /// A list of nodes set side by side, spaced by their classes.
    fn list(&mut self, nodes: &[Node], st: St) -> MBox {
        let mut st = st;
        let mut atoms: Vec<(Option<Class>, MBox)> = Vec::with_capacity(nodes.len());
        for n in nodes {
            match n {
                Node::Style(s) => st = St { style: *s, ..st },
                Node::Space(em) => atoms.push((None, MBox::empty(em * self.em(st)))),
                n => {
                    let (class, b) = self.node(n, st);
                    atoms.push((Some(class), b));
                }
            }
        }
        // A binary operator with nothing to join is an ordinary symbol.
        let classes: Vec<usize> = (0..atoms.len()).filter(|&i| atoms[i].0.is_some()).collect();
        for (k, &i) in classes.iter().enumerate() {
            if atoms[i].0 != Some(Class::Bin) {
                continue;
            }
            let before = k.checked_sub(1).and_then(|p| atoms[classes[p]].0);
            let after = classes.get(k + 1).and_then(|&n| atoms[n].0);
            let alone_before = before.is_none_or(|c| matches!(c, Class::Bin | Class::Op | Class::Rel | Class::Open | Class::Punct));
            let alone_after = after.is_none_or(|c| matches!(c, Class::Rel | Class::Close | Class::Punct));
            if alone_before || alone_after {
                atoms[i].0 = Some(Class::Ord);
            }
        }
        let mut out = MBox::default();
        let mut prev: Option<Class> = None;
        let mut italic = 0.0;
        for (class, b) in atoms {
            if let (Some(p), Some(c)) = (prev, class) {
                out.w += gap(p, c, st) * self.em(st);
            }
            // An italic letter before an upright one leans into it: room.
            if italic > 0.0 && b.italic == 0.0 && class.is_some() {
                out.w += italic * 0.5;
            }
            italic = b.italic;
            let x = out.w;
            let w = b.w;
            out.put(b, x, 0.0);
            out.w = x + w;
            if class.is_some() {
                prev = class;
            }
        }
        out.italic = italic;
        out
    }

    /// One node: its class and its box.
    fn node(&mut self, n: &Node, st: St) -> (Class, MBox) {
        let em = self.em(st);
        match n {
            Node::Atom { text, class, font } => (*class, self.glyphs(text, *font, em, false)),
            Node::Text { text, font } => (Class::Ord, self.glyphs(text, *font, em, false)),
            Node::Error(text) => (Class::Ord, self.glyphs(text, Font::Upright, em, true)),
            Node::BigOp { text, symbol, .. } => (Class::Op, self.op(text, *symbol, st)),
            Node::Group(list) => (Class::Ord, self.list(list, st)),
            Node::Scripts { base, sub, sup } => self.scripts(base, sub.as_deref(), sup.as_deref(), st),
            Node::Frac { num, den, bar, delims, style } => (Class::Ord, self.frac(num, den, *bar, *delims, style.map(|s| St { style: s, ..st }).unwrap_or(st))),
            Node::Sqrt { index, body } => (Class::Ord, self.sqrt(index.as_deref(), body, st)),
            Node::Fenced { left, right, body } => (Class::Inner, self.fenced(*left, *right, body, st)),
            Node::Middle(d) => {
                let (h, dp) = self.fence.unwrap_or((0.7 * em, 0.2 * em));
                (Class::Rel, self.delim(*d, h, dp, st))
            }
            Node::Big { delim, size, class } => {
                let total = [1.2, 1.8, 2.4, 3.0][(*size as usize).clamp(1, 4) - 1] * em;
                let axis = AXIS * em;
                (*class, self.delim_sized(*delim, total, axis, st))
            }
            Node::Accent { accent, body } => (Class::Ord, self.accent(*accent, body, st)),
            Node::Over { over, body, under } => {
                let class = match body.as_ref() {
                    Node::Atom { class, .. } => *class,
                    Node::Group(g) if g.len() == 1 => match &g[0] {
                        Node::Atom { class, .. } => *class,
                        _ => Class::Ord,
                    },
                    _ => Class::Ord,
                };
                let b = self.list(std::slice::from_ref(body), st);
                let o = self.list(std::slice::from_ref(over), if *under { st.sub() } else { st.sup() });
                let (sup, sub) = if *under { (None, Some(o)) } else { (Some(o), None) };
                (class, self.limits(b, sup, sub, st, 0.0))
            }
            Node::Space(e) => (Class::Ord, MBox::empty(e * em)),
            Node::Array { rows, cols, left, right, kind, lines } => (Class::Ord, self.array(rows, cols, *left, *right, *kind, lines, st)),
            Node::Style(_) => (Class::Ord, MBox::default()),
            Node::Boxed(body) => {
                let b = self.list(std::slice::from_ref(body), st);
                let pad = 0.3 * em;
                let mut out = MBox { w: b.w + 2.0 * pad, h: b.h + pad, d: b.d + pad, ..MBox::default() };
                out.items.push(Item::Panel([0.0, -(b.h + pad), b.w + 2.0 * pad, b.d + pad]));
                let h = b.h + pad;
                let d = b.d + pad;
                out.put(b, pad, 0.0);
                out.h = h;
                out.d = d;
                (Class::Ord, out)
            }
            Node::Phantom { body, width, height } => {
                let b = self.list(std::slice::from_ref(body), st);
                (Class::Ord, MBox { w: if *width { b.w } else { 0.0 }, h: if *height { b.h } else { 0.0 }, d: if *height { b.d } else { 0.0 }, ..MBox::default() })
            }
        }
    }

    /// A big operator on its own: its symbol larger, centred on the axis;
    /// or its name.
    fn op(&mut self, text: &str, symbol: bool, st: St) -> MBox {
        let em = self.em(st);
        if !symbol {
            return self.glyphs(text, Font::Upright, em, false);
        }
        let k = op_scale(text, st.display());
        let size = em * k;
        let mut b = self.glyphs(text, Font::Upright, size, false);
        // The glyph's own extent, as its face draws it (faces set a sum and
        // an integral each their own way).
        let integral = text.starts_with(['∫', '∬', '∭', '∮']);
        let (h, d) = self.ink(&b).unwrap_or(if integral { (0.86 * size, 0.24 * size) } else { (0.74 * size, 0.04 * size) });
        let shift = (h - d) / 2.0 - AXIS * em;
        b.items = b.items.into_iter().map(|i| i.moved(0.0, shift)).collect();
        b.h = h - shift;
        b.d = d + shift;
        if integral {
            // An integral leans: its scripts sit past its slant.
            b.italic = 0.12 * size;
        }
        b
    }

    fn scripts(&mut self, base: &Node, sub: Option<&Node>, sup: Option<&Node>, st: St) -> (Class, MBox) {
        let em = self.em(st);
        // Primes are glyphs that already sit high: beside the base at its
        // size, not raised; what follows them is the superscript.
        if let Some(Node::Atom { text, .. }) = sup.and_then(first_of)
            && text.starts_with(['\u{2032}', '\u{2033}', '\u{2034}'])
        {
            let rest: Vec<Node> = match sup {
                Some(Node::Group(g)) => g[1..].to_vec(),
                _ => Vec::new(),
            };
            let primed = Node::Group(vec![base.clone(), Node::Atom { text: text.clone(), class: Class::Ord, font: Font::Upright }]);
            if rest.is_empty() && sub.is_none() {
                return self.node(&primed, st);
            }
            let rest = (!rest.is_empty()).then_some(Node::Group(rest));
            return self.scripts(&primed, sub, rest.as_ref(), st);
        }
        let (class, b) = self.node(base, st);
        let limits = match base {
            // Above and below in display style, but an integral's beside
            // it; as `\limits` or `\nolimits` says, in any style.
            Node::BigOp { limits, symbol, text } => {
                let integral = *symbol && text.starts_with(['∫', '∬', '∭', '∮']);
                limits.unwrap_or(st.display() && !integral)
            }
            Node::Accent { accent: Accent::OverBrace | Accent::UnderBrace, .. } => true,
            _ => false,
        };
        let sup_box = sup.map(|n| self.list(std::slice::from_ref(n), st.sup()));
        let sub_box = sub.map(|n| self.list(std::slice::from_ref(n), st.sub()));
        if limits {
            let lean = b.italic;
            return (class, self.limits(b, sup_box, sub_box, st, lean));
        }
        let single = matches!(base, Node::Atom { .. } | Node::Text { .. }) || matches!(base, Node::BigOp { symbol: false, .. });
        let theta = self.rule(st);
        let sup_em = self.em(st.sup());
        let sub_em = self.em(st.sub());
        let mut u = if single { 0.0 } else { b.h - SUP_DROP * sup_em };
        let mut v = if single { 0.0 } else { b.d + SUB_DROP * sub_em };
        if let Some(sp) = &sup_box {
            let p = if st.cramped {
                SUP3
            } else if st.display() {
                SUP1
            } else {
                SUP2
            };
            u = u.max(p * em).max(sp.d + 0.25 * X_HEIGHT * em);
        }
        match (&sup_box, &sub_box) {
            (None, Some(sb)) => v = v.max(SUB1 * em).max(sb.h - 0.8 * X_HEIGHT * em),
            (Some(sp), Some(sb)) => {
                v = v.max(SUB2 * em);
                let gap = (u - sp.d) - (sb.h - v);
                if gap < 4.0 * theta {
                    v += 4.0 * theta - gap;
                    let psi = 0.8 * X_HEIGHT * em - (u - sp.d);
                    if psi > 0.0 {
                        u += psi;
                        v -= psi;
                    }
                }
            }
            _ => {}
        }
        let lean = b.italic;
        let bw = b.w;
        let mut out = MBox::default();
        out.put(b, 0.0, 0.0);
        let mut right = bw;
        if let Some(sp) = sup_box {
            let w = sp.w;
            out.put(sp, bw + lean, -u);
            right = right.max(bw + lean + w);
        }
        if let Some(sb) = sub_box {
            // An integral's subscript tucks under its slant.
            let x = if matches!(base, Node::BigOp { symbol: true, .. }) { bw - lean * 0.6 } else { bw };
            let w = sb.w;
            out.put(sb, x, v);
            right = right.max(x + w);
        }
        out.w = right + SCRIPT_SPACE * em;
        (class, out)
    }

    /// An operator with its scripts above and below it, centred.
    fn limits(&mut self, op: MBox, sup: Option<MBox>, sub: Option<MBox>, st: St, lean: f32) -> MBox {
        let em = self.em(st);
        let w = [Some(op.w), sup.as_ref().map(|b| b.w), sub.as_ref().map(|b| b.w)].into_iter().flatten().fold(0.0f32, f32::max);
        let mut out = MBox::default();
        let (oh, od, ow) = (op.h, op.d, op.w);
        out.put(op, (w - ow) / 2.0, 0.0);
        if let Some(sp) = sup {
            let kern = (BIG_OP_SPACING1 * em).max(BIG_OP_SPACING3 * em - sp.d);
            let y = -(oh + kern + sp.d);
            let top = sp.h - y + BIG_OP_SPACING5 * em;
            let sw = sp.w;
            out.put(sp, (w - sw) / 2.0 + lean / 2.0, y);
            out.h = out.h.max(top);
        }
        if let Some(sb) = sub {
            let kern = (BIG_OP_SPACING2 * em).max(BIG_OP_SPACING4 * em - sb.h);
            let y = od + kern + sb.h;
            let bottom = y + sb.d + BIG_OP_SPACING5 * em;
            let sw = sb.w;
            out.put(sb, (w - sw) / 2.0 - lean / 2.0, y);
            out.d = out.d.max(bottom);
        }
        out.w = w;
        out
    }

    fn frac(&mut self, num: &Node, den: &Node, bar: bool, delims: Option<(char, char)>, st: St) -> MBox {
        let em = self.em(st);
        let theta = if bar { self.rule(st) } else { 0.0 };
        let n = self.list(std::slice::from_ref(num), st.num());
        let d = self.list(std::slice::from_ref(den), st.den());
        let (mut u, mut v) = if st.display() { (NUM1 * em, DENOM1 * em) } else { (if bar { NUM2 } else { NUM3 } * em, DENOM2 * em) };
        let axis = AXIS * em;
        if bar {
            let phi = if st.display() { 3.0 * theta } else { theta };
            let above = (u - n.d) - (axis + theta / 2.0);
            if above < phi {
                u += phi - above;
            }
            let below = (axis - theta / 2.0) - (d.h - v);
            if below < phi {
                v += phi - below;
            }
        } else {
            let phi = if st.display() { 7.0 * self.rule(st) } else { 3.0 * self.rule(st) };
            let psi = (u - n.d) - (d.h - v);
            if psi < phi {
                u += (phi - psi) / 2.0;
                v += (phi - psi) / 2.0;
            }
        }
        let pad = NULL_DELIM * em;
        let w = n.w.max(d.w);
        let mut out = MBox::default();
        let (nw, dw) = (n.w, d.w);
        out.put(n, pad + (w - nw) / 2.0, -u);
        out.put(d, pad + (w - dw) / 2.0, v);
        if bar {
            out.items.push(Item::Bar([pad * 0.5, -(axis + theta / 2.0), pad * 1.5 + w, -(axis - theta / 2.0)]));
        }
        out.w = w + 2.0 * pad;
        match delims {
            Some((l, r)) => {
                let (h, dp) = (out.h, out.d);
                let left = self.delim(Delim::Char(l), h, dp, st);
                let right = self.delim(Delim::Char(r), h, dp, st);
                let mut row = MBox::default();
                let lw = left.w;
                row.put(left, 0.0, 0.0);
                let fw = out.w;
                row.put(out, lw, 0.0);
                let x = lw + fw;
                let rw = right.w;
                row.put(right, x, 0.0);
                row.w = x + rw;
                row
            }
            None => out,
        }
    }

    fn sqrt(&mut self, index: Option<&Node>, body: &Node, st: St) -> MBox {
        let em = self.em(st);
        let b = self.list(std::slice::from_ref(body), st.cramp());
        let theta = self.rule(st);
        let phi = if st.display() { X_HEIGHT * em } else { theta };
        let psi = theta + phi / 4.0;
        let top = b.h.max(X_HEIGHT * em) + psi + theta;
        let bottom = b.d.max(0.0) + 0.08 * em;
        let tall = top + bottom;
        // The sign: a little wider as it grows.
        let sign = (0.56 + 0.05 * (tall / em - 1.2).max(0.0)).min(0.9) * em;
        let end = sign + b.w + 0.1 * em;
        let polygons = radical(sign, end, top, bottom, theta, em);
        let mut out = MBox::default();
        out.items.push(Item::Shape { rect: [0.0, -top, end, bottom], polygons: Rc::new(polygons) });
        out.h = top + theta;
        out.d = bottom;
        out.put(b, sign + 0.04 * em, 0.0);
        out.w = end;
        let Some(index) = index else { return out };
        // An index rides on the sign's tick, as TeX's `\root` puts it: 5mu
        // in, raised six tenths of the sign, the sign 10mu back under it.
        let i = self.list(std::slice::from_ref(index), St { style: MathStyle::ScriptScript, cramped: false });
        let raise = 0.6 * (out.h - out.d);
        let lead = 5.0 / 18.0 * em;
        let mut sign_x = lead + i.w - 10.0 / 18.0 * em;
        let shift = (-sign_x).max(0.0);
        sign_x += shift;
        let mut row = MBox::default();
        row.put(i, lead + shift, -raise);
        let ow = out.w;
        row.put(out, sign_x, 0.0);
        row.w = sign_x + ow;
        row
    }

    /// `\left … \right`: the list, then delimiters as tall as it.
    fn fenced(&mut self, left: Delim, right: Delim, body: &[Node], st: St) -> MBox {
        let saved = self.fence;
        let mut b = self.list(body, st);
        if body.iter().any(|n| matches!(n, Node::Middle(_))) {
            self.fence = Some((b.h, b.d));
            b = self.list(body, st);
        }
        self.fence = saved;
        let (h, d) = (b.h, b.d);
        let l = self.delim(left, h, d, st);
        let r = self.delim(right, h, d, st);
        let mut out = MBox::default();
        let lw = l.w;
        out.put(l, 0.0, 0.0);
        let bw = b.w;
        out.put(b, lw, 0.0);
        let rw = r.w;
        out.put(r, lw + bw, 0.0);
        out.w = lw + bw + rw;
        out
    }

    /// A delimiter tall enough for something `h` above and `d` below the
    /// baseline, centred on the axis, as TeX sizes them.
    fn delim(&mut self, delim: Delim, h: f32, d: f32, st: St) -> MBox {
        let em = self.em(st);
        let axis = AXIS * em;
        let delta = (h - axis).max(d + axis);
        let total = (delta * 2.0 * 0.901).max(2.0 * delta - 0.5 * em);
        self.delim_sized(delim, total, axis, st)
    }

    /// A delimiter `total` tall, centred `axis` above the baseline: the
    /// font's own glyph while that is tall enough, else filled from an
    /// outline.
    fn delim_sized(&mut self, delim: Delim, total: f32, axis: f32, st: St) -> MBox {
        let em = self.em(st);
        let Delim::Char(c) = delim else { return MBox::empty(NULL_DELIM * em) };
        if total <= 1.15 * em || !drawn(c) {
            let mut b = self.glyphs(&c.to_string(), Font::Upright, em, false);
            if !drawn(c) && total > 1.15 * em {
                // Arrows and the like: as large as asked.
                let k = total / em;
                b = self.glyphs(&c.to_string(), Font::Upright, em * k, false);
                let shift = (b.h - b.d) / 2.0 - axis;
                b.items = b.items.into_iter().map(|i| i.moved(0.0, shift)).collect();
                b.h -= shift;
                b.d += shift;
            }
            return b;
        }
        let top = axis + total / 2.0;
        let bottom = total / 2.0 - axis;
        let (w, polygons) = delimiter(c, total, self.rule(st), em);
        MBox { w, h: top, d: bottom, items: vec![Item::Shape { rect: [0.0, -top, w, bottom], polygons: Rc::new(polygons) }], italic: 0.0 }
    }

    fn accent(&mut self, accent: Accent, body: &Node, st: St) -> MBox {
        let em = self.em(st);
        let b = self.list(std::slice::from_ref(body), st.cramp());
        let theta = self.rule(st);
        let lean = b.italic * 0.6;
        let (bw, bh, bd) = (b.w, b.h, b.d);
        let mut out = MBox::default();
        out.put(b, 0.0, 0.0);
        out.w = bw;
        match accent {
            Accent::Overline => {
                let y = -(bh + 3.0 * theta);
                out.items.push(Item::Bar([0.0, y - theta, bw, y]));
                out.h = out.h.max(bh + 4.0 * theta);
            }
            Accent::Underline => {
                let y = bd + 3.0 * theta;
                out.items.push(Item::Bar([0.0, y, bw, y + theta]));
                out.d = out.d.max(bd + 4.0 * theta);
            }
            Accent::Bar => {
                let w = (bw * 0.8).max(0.3 * em);
                let x = (bw - w) / 2.0 + lean;
                let y = -(bh.max(X_HEIGHT * em) + 0.12 * em);
                out.items.push(Item::Bar([x, y - theta, x + w, y]));
                out.h = out.h.max(-y + theta);
            }
            Accent::OverArrow | Accent::OverLeftArrow | Accent::Vec if accent != Accent::Vec || bw > 0.9 * em => {
                // Its shaft on `mid`, in a box `half` either side of it.
                let (w, half) = (bw.max(0.4 * em), 0.15 * em);
                let mid = -(bh.max(X_HEIGHT * em) + 0.18 * em);
                let polygons = arrow(w, half, theta, em, accent == Accent::OverLeftArrow);
                out.items.push(Item::Shape { rect: [0.0, mid - half, w, mid + half], polygons: Rc::new(polygons) });
                out.h = out.h.max(-mid + half);
            }
            Accent::WideHat | Accent::WideTilde if bw > 1.1 * em => {
                let y = -(bh + 0.12 * em);
                let rise = (0.06 * bw).clamp(0.18 * em, 0.4 * em);
                let polygons = if accent == Accent::WideHat { hat(bw, rise, theta) } else { tilde(bw, rise, theta) };
                out.items.push(Item::Shape { rect: [0.0, y - rise - theta, bw, y + theta], polygons: Rc::new(polygons) });
                out.h = out.h.max(-y + rise + theta);
            }
            Accent::OverBrace | Accent::UnderBrace => {
                let over = accent == Accent::OverBrace;
                let thick = 0.3 * em;
                let polygons = brace_across(bw, thick, theta, em, over);
                let y = if over { -(bh + 0.12 * em + thick) } else { bd + 0.12 * em };
                out.items.push(Item::Shape { rect: [0.0, y, bw, y + thick], polygons: Rc::new(polygons) });
                if over { out.h = out.h.max(-y) } else { out.d = out.d.max(y + thick) }
            }
            _ => {
                let mark = match accent {
                    Accent::Hat | Accent::WideHat => "\u{02C6}",
                    Accent::Tilde | Accent::WideTilde => "\u{02DC}",
                    Accent::Vec => "\u{2192}",
                    Accent::Dot => "\u{02D9}",
                    Accent::Ddot => "\u{00A8}",
                    Accent::Acute => "\u{00B4}",
                    Accent::Grave => "\u{0060}",
                    Accent::Breve => "\u{02D8}",
                    _ => "\u{02C7}",
                };
                let size = if accent == Accent::Vec { em * 0.8 } else { em };
                let a = self.glyphs(mark, Font::Upright, size, false);
                // The mark's ink just over the body, wherever its face puts
                // it: its bottom a little above the body's top.
                let (above, below) = self.ink(&a).unwrap_or((0.78 * size, -0.55 * size));
                let gap = 0.08 * em;
                let lift = bh.max(X_HEIGHT * em) + gap + below;
                let aw = a.w;
                out.put(MBox { h: 0.0, d: 0.0, ..a }, (bw - aw) / 2.0 + lean, -lift);
                out.h = out.h.max(lift + above);
            }
        }
        out.w = bw;
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn array(&mut self, rows: &[Vec<Node>], cols: &[ColAlign], left: Delim, right: Delim, kind: ArrayKind, lines: &[usize], st: St) -> MBox {
        let em = self.em(st);
        let cell_st = match kind {
            ArrayKind::Aligned => St { style: MathStyle::Display, cramped: st.cramped },
            ArrayKind::Small => St { style: MathStyle::Script, cramped: st.cramped },
            _ => St { style: if st.style == MathStyle::Display { MathStyle::Text } else { st.style }, cramped: st.cramped },
        };
        let cem = self.em(cell_st);
        let mut grid: Vec<Vec<MBox>> = Vec::with_capacity(rows.len());
        for row in rows {
            let mut cells = Vec::with_capacity(row.len());
            for (c, cell) in row.iter().enumerate() {
                // An aligned line's right half starts at its relation: set
                // as if something stood before it.
                let lead = kind == ArrayKind::Aligned && cols.is_empty() && c % 2 == 1;
                let b = if lead {
                    let mut with = vec![Node::Atom { text: String::new(), class: Class::Ord, font: Font::Upright }];
                    match cell {
                        Node::Group(g) => with.extend(g.iter().cloned()),
                        n => with.push(n.clone()),
                    }
                    self.list(&with, cell_st)
                } else {
                    self.list(std::slice::from_ref(cell), cell_st)
                };
                cells.push(b);
            }
            grid.push(cells);
        }
        let ncols = grid.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0.0f32; ncols];
        for row in &grid {
            for (c, b) in row.iter().enumerate() {
                widths[c] = widths[c].max(b.w);
            }
        }
        let align = |c: usize| -> ColAlign {
            match kind {
                ArrayKind::Cases => ColAlign::Left,
                ArrayKind::Aligned if cols.is_empty() => {
                    if c.is_multiple_of(2) {
                        ColAlign::Right
                    } else {
                        ColAlign::Left
                    }
                }
                _ => cols.get(c).or(cols.last()).copied().unwrap_or(ColAlign::Center),
            }
        };
        // Room before each column.
        let space = |c: usize| -> f32 {
            if c == 0 {
                return 0.0;
            }
            match kind {
                ArrayKind::Aligned if cols.is_empty() => {
                    if c % 2 == 1 {
                        0.0
                    } else {
                        2.0 * cem
                    }
                }
                ArrayKind::Small => 0.35 * em,
                _ => 1.0 * cem,
            }
        };
        let mut xs = Vec::with_capacity(ncols);
        let mut x = 0.0;
        for (c, w) in widths.iter().enumerate() {
            x += space(c);
            xs.push(x);
            x += w;
        }
        let total_w = x;
        let jot = if kind == ArrayKind::Aligned { 0.25 * cem } else { 0.0 };
        let strut = if kind == ArrayKind::Small { 0.7 } else { 1.0 };
        let mut heights: Vec<(f32, f32)> = grid
            .iter()
            .map(|row| {
                let h = row.iter().map(|b| b.h).fold(STRUT_HEIGHT * cem * strut, f32::max);
                let d = row.iter().map(|b| b.d).fold(STRUT_DEPTH * cem * strut, f32::max);
                (h, d)
            })
            .collect();
        if heights.is_empty() {
            heights.push((STRUT_HEIGHT * cem, STRUT_DEPTH * cem));
        }
        let total_h: f32 = heights.iter().map(|(h, d)| h + d).sum::<f32>() + jot * (heights.len() - 1) as f32;
        // The array's middle on the axis.
        let axis = AXIS * em;
        let top = -(axis + total_h / 2.0);
        let mut table = MBox::default();
        let mut y = top;
        let mut rule_ys = Vec::new();
        for (r, row) in grid.into_iter().enumerate() {
            let (h, d) = heights[r];
            if lines.contains(&r) {
                rule_ys.push(y - jot / 2.0);
            }
            let base = y + h;
            for (c, b) in row.into_iter().enumerate() {
                let slack = widths[c] - b.w;
                let dx = match align(c) {
                    ColAlign::Left => 0.0,
                    ColAlign::Center => slack / 2.0,
                    ColAlign::Right => slack,
                };
                table.put(b, xs[c] + dx, base);
            }
            y = base + d + jot;
        }
        if lines.contains(&heights.len()) {
            rule_ys.push(y - jot);
        }
        let theta = self.rule(st);
        for ry in rule_ys {
            table.items.push(Item::Bar([0.0, ry - theta / 2.0, total_w, ry + theta / 2.0]));
        }
        table.w = total_w;
        table.h = -top;
        table.d = total_h + top;
        if left == Delim::None && right == Delim::None {
            return table;
        }
        let (h, d) = (table.h, table.d);
        let pad = if kind == ArrayKind::Cases { 0.2 * em } else { 0.1 * em };
        let l = self.delim(left, h, d, st);
        let r = self.delim(right, h, d, st);
        let mut out = MBox::default();
        let lw = l.w;
        out.put(l, 0.0, 0.0);
        out.put(table, lw + pad, 0.0);
        let rx = lw + pad + total_w + if right == Delim::None { 0.0 } else { pad };
        let rw = r.w;
        out.put(r, rx, 0.0);
        out.w = rx + rw;
        out
    }
}

/// The first node of a script: itself, or a group's first.
fn first_of(n: &Node) -> Option<&Node> {
    match n {
        Node::Group(g) => g.first(),
        n => Some(n),
    }
}

/// Delimiters drawn from outlines when tall; others are glyphs at any size.
fn drawn(c: char) -> bool {
    matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | '|' | '‖' | '⟨' | '⟩' | '⌊' | '⌋' | '⌈' | '⌉' | '/' | '\\')
}

/// Kept-layout key for math's glyph runs (see `scene::kept_layout`).
const MATH_LAYOUT: u64 = 0x3A7B_0000_0000_0001;

/// A band `width` thick from `a` to `b`, reaching `reach` past each end
/// (so bands that meet leave no gap), wound one way.
fn band(a: P, b: P, width: f32, reach: f32) -> Vec<P> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-4);
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy * width / 2.0, ux * width / 2.0);
    let (a, b) = ((a.0 - ux * reach, a.1 - uy * reach), (b.0 + ux * reach, b.1 + uy * reach));
    wound(vec![(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)])
}

/// A polygon turned to wind clockwise on screen, so shapes that overlap
/// fill as one.
fn wound(mut p: Vec<P>) -> Vec<P> {
    let area: f32 = (0..p.len()).map(|i| {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        a.0 * b.1 - b.0 * a.1
    }).sum();
    if area < 0.0 {
        p.reverse();
    }
    p
}

fn rect(l: f32, t: f32, r: f32, b: f32) -> Vec<P> {
    wound(vec![(l, t), (r, t), (r, b), (l, b)])
}

/// A root's sign and its bar, `end` wide: the sign `sign` wide, from
/// `top` above the baseline to `bottom` below, its bar `theta` thick; in
/// the rectangle's coordinates (its top left at 0, 0).
fn radical(sign: f32, end: f32, top: f32, bottom: f32, theta: f32, em: f32) -> Vec<Vec<P>> {
    let h = top + bottom;
    let y_bottom = h;
    // The tick starts a little way up, and climbs a little.
    let y_tick = y_bottom - (0.5 * em).min(0.5 * h);
    let thin = theta * 1.05;
    let thick = (theta * 2.4).max(0.09 * em);
    let a = (0.02 * em, y_tick + 0.04 * em);
    let b = (0.16 * em, y_tick - 0.06 * em);
    let c = (sign * 0.55, y_bottom - thin * 0.5);
    let d = (sign, theta * 0.5);
    let mut out = vec![band(a, b, thin, 0.0), band(b, c, thick, thin * 0.3), band(c, d, thin, thin * 0.3)];
    // Its tip and the bar meet square.
    out.push(rect(sign - thin * 0.4, 0.0, end, theta));
    out
}

/// A tall delimiter `total` high from its outline: its width and its
/// polygons, from its box's top left.
fn delimiter(c: char, total: f32, theta: f32, em: f32) -> (f32, Vec<Vec<P>>) {
    let h = total;
    let grow = (h / em - 1.0).max(0.0);
    let stem = (theta * 1.5).max(0.075 * em) * (1.0 + 0.08 * grow).min(1.6);
    let mirror = |w: f32, ps: Vec<Vec<P>>| -> Vec<Vec<P>> { ps.into_iter().map(|p| wound(p.into_iter().map(|(x, y)| (w - x, y)).collect())).collect() };
    match c {
        '(' | ')' => {
            let w = ((0.4 + 0.05 * grow) * em).min(0.8 * em);
            let (xr, xl) = (w - 0.05 * em, 0.08 * em);
            let end = theta * 0.8;
            // Controls that bring the curve's middle to its far side.
            let outer = (4.0 * xl - xr) / 3.0;
            let inner = (4.0 * (xl + stem) - (xr - end)) / 3.0;
            let k = h * 0.1;
            let path = format!(
                "M{xr},0 C{outer},{k} {outer},{} {xr},{h} L{},{h} C{inner},{} {inner},{k} {},0 Z",
                h - k,
                xr - end,
                h - k,
                xr - end
            );
            let poly = crate::vector::polygons(&path, 24).into_iter().map(wound).collect();
            (w, if c == '(' { poly } else { mirror(w, poly) })
        }
        '[' | ']' => {
            let w = 0.34 * em;
            let x = 0.1 * em;
            let serif = (stem * 0.8).max(theta);
            let poly = vec![rect(x, 0.0, x + stem, h), rect(x, 0.0, w - 0.04 * em, serif), rect(x, h - serif, w - 0.04 * em, h)];
            (w, if c == '[' { poly } else { mirror(w, poly) })
        }
        '⌊' | '⌋' | '⌈' | '⌉' => {
            let w = 0.38 * em;
            let x = 0.1 * em;
            let serif = (stem * 0.8).max(theta);
            let foot = if matches!(c, '⌊' | '⌋') { rect(x, h - serif, w - 0.04 * em, h) } else { rect(x, 0.0, w - 0.04 * em, serif) };
            let poly = vec![rect(x, 0.0, x + stem, h), foot];
            (w, if matches!(c, '⌊' | '⌈') { poly } else { mirror(w, poly) })
        }
        '{' | '}' => {
            let w = ((0.5 + 0.03 * grow) * em).min(0.8 * em);
            let poly = brace(w, h, stem, theta, em);
            (w, if c == '{' { poly } else { mirror(w, poly) })
        }
        '|' => {
            let w = 0.28 * em;
            (w, vec![rect((w - stem) / 2.0, 0.0, (w + stem) / 2.0, h)])
        }
        '‖' => {
            let w = 0.44 * em;
            let g = 0.1 * em;
            (w, vec![rect(w / 2.0 - g - stem, 0.0, w / 2.0 - g, h), rect(w / 2.0 + g, 0.0, w / 2.0 + g + stem, h)])
        }
        '⟨' | '⟩' => {
            let w = ((0.36 + 0.06 * grow) * em).min(0.8 * em);
            let (xl, xr) = (0.08 * em, w - 0.06 * em);
            let mid = h / 2.0;
            let poly = vec![band((xr, 0.0), (xl, mid), stem * 0.8, 0.0), band((xl, mid), (xr, h), stem * 0.8, 0.0)];
            (w, if c == '⟨' { poly } else { mirror(w, poly) })
        }
        _ => {
            // A slash or a backslash, leaning across.
            let w = (0.2 * h).clamp(0.4 * em, 1.2 * em);
            let poly = vec![band((0.05 * em, h), (w - 0.05 * em, 0.0), stem * 0.9, 0.0)];
            (w, if c == '/' { poly } else { mirror(w, poly) })
        }
    }
}

/// A left brace `w` wide and `h` tall: tips on the right, its point on the
/// left halfway down.
fn brace(w: f32, h: f32, stem: f32, theta: f32, em: f32) -> Vec<Vec<P>> {
    let (xr, xl) = (w - 0.05 * em, 0.03 * em);
    let xc = w * 0.55;
    let mid = h / 2.0;
    let q = (0.3 * em).min(h * 0.22);
    let (tip, point) = (theta * 0.8, theta * 0.5);
    // Its upper half along its middle line: the tip curls into the stem,
    // the stem runs down, then bends to the point; thin at both ends.
    let mut upper: Vec<(P, f32)> = Vec::new();
    cubic_along(&mut upper, [(xr, tip / 2.0), (xc, tip / 2.0), (xc, tip / 2.0), (xc, q)], (tip, stem));
    upper.push(((xc, mid - q), stem));
    cubic_along(&mut upper, [(xc, mid - q), (xc, mid - q * 0.3), (xc - (xc - xl) * 0.4, mid), (xl, mid)], (stem, point));
    let lower: Vec<(P, f32)> = upper.iter().rev().map(|&((x, y), t)| ((x, h - y), t)).collect();
    vec![ribbon(&upper), ribbon(&lower)]
}

/// Points along a cubic from `c[0]` to `c[3]`, each with a thickness
/// eased from the first of `from_to` to the second.
fn cubic_along(out: &mut Vec<(P, f32)>, c: [P; 4], (from, to): (f32, f32)) {
    let n = 12;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let (a, b, cc, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        let x = a * c[0].0 + b * c[1].0 + cc * c[2].0 + d * c[3].0;
        let y = a * c[0].1 + b * c[1].1 + cc * c[2].1 + d * c[3].1;
        let ease = t * t * (3.0 - 2.0 * t);
        out.push(((x, y), from + (to - from) * ease));
    }
}

/// A band along `points`, as thick at each as it says: one polygon.
fn ribbon(points: &[(P, f32)]) -> Vec<P> {
    let mut left = Vec::with_capacity(points.len());
    let mut right = Vec::with_capacity(points.len());
    for (i, &((x, y), t)) in points.iter().enumerate() {
        let (a, b) = (points[i.saturating_sub(1)].0, points[(i + 1).min(points.len() - 1)].0);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(1e-4);
        let (nx, ny) = (-dy / len * t / 2.0, dx / len * t / 2.0);
        left.push((x + nx, y + ny));
        right.push((x - nx, y - ny));
    }
    right.reverse();
    left.extend(right);
    wound(left)
}

/// A brace lying across `w` and `thick` high, its point up (over) or down
/// (under), from its box's top left.
fn brace_across(w: f32, thick: f32, theta: f32, em: f32, over: bool) -> Vec<Vec<P>> {
    let stem = (theta * 1.4).max(0.07 * em);
    // A left brace turned: its height along the width.
    brace(thick, w, stem, theta, em)
        .into_iter()
        .map(|p| wound(p.into_iter().map(|(x, y)| if over { (y, x) } else { (y, thick - x) }).collect()))
        .collect()
}

/// An arrow across `w` with its shaft on `y`: its head right, or left.
fn arrow(w: f32, y: f32, theta: f32, em: f32, left: bool) -> Vec<Vec<P>> {
    let head = 0.22 * em;
    let shaft = rect(0.0, y - theta / 2.0, w - head * 0.4, y + theta / 2.0);
    let tip = wound(vec![(w, y), (w - head, y - head * 0.55), (w - head * 0.75, y), (w - head, y + head * 0.55)]);
    let polys = vec![shaft, tip];
    if left { polys.into_iter().map(|p| wound(p.into_iter().map(|(x, py)| (w - x, py)).collect())).collect() } else { polys }
}

/// A wide hat over `w`, rising `rise`, from its box's top left.
fn hat(w: f32, rise: f32, theta: f32) -> Vec<Vec<P>> {
    let t = theta * 1.2;
    let bottom = rise + t;
    vec![band((0.0, bottom - t / 2.0), (w / 2.0, t / 2.0), t, 0.0), band((w / 2.0, t / 2.0), (w, bottom - t / 2.0), t, 0.0)]
}

/// A wide tilde over `w`, `rise` high, from its box's top left.
fn tilde(w: f32, rise: f32, theta: f32) -> Vec<Vec<P>> {
    let t = theta * 1.3;
    let n = 24;
    let mid = (rise + t) / 2.0;
    let points: Vec<P> = (0..=n)
        .map(|i| {
            let x = w * i as f32 / n as f32;
            (x, mid - (std::f32::consts::TAU * i as f32 / n as f32).sin() * rise / 2.0)
        })
        .collect();
    points.windows(2).map(|s| band(s[0], s[1], t, t * 0.2)).collect()
}

#[cfg(test)]
mod tests;
