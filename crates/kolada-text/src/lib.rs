//! Text for every OS: shaping, line breaking, bidirectional order, caret and
//! hit positions, and glyph pictures, by one engine (cosmic-text: harfrust,
//! unicode-bidi, swash), so a note wraps, moves and looks the same on
//! every platform.
//!
//! Offsets are UTF-8 bytes into the paragraph's text, as the editor's are.
//! Sizes and positions are in DIPs unless a function says pixels.
//!
//! The shell lists the font files to use ([`FontSet`]). Nothing here scans
//! the system's fonts: that costs 100 ms to seconds and launch cannot pay it.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

pub mod installed;

use cosmic_text::{
    Align as CAlign, Attrs, Buffer, CacheKey, Ellipsize, Family, FontSystem, Metrics, Shaping, Style as CStyle, SwashCache,
    SwashContent, Weight, Wrap, fontdb,
};

/// Which family a run of text is set in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// The app's own text: menus, labels, dates.
    Ui,
    /// The note's text (the user picks sans, serif or mono).
    Text,
    /// Code, in notes.
    Code,
    /// Icons.
    Icon,
}

/// The fonts an app ships or finds, by file, and the family name each role
/// uses from them.
#[derive(Clone, Debug, Default)]
pub struct FontSet {
    /// Loaded at once: what the first screen is drawn in.
    pub files: Vec<PathBuf>,
    /// Loaded when text first needs a script, symbol or emoji the first
    /// files lack (see `needs_more_fonts`): launch does not pay for them.
    pub fallback: Vec<PathBuf>,
    /// Fonts that ship inside the program (an icon font), loaded at once.
    pub data: Vec<&'static [u8]>,
    pub ui: String,
    pub text: String,
    pub code: String,
    pub icon: String,
}

/// The weight drawn for `weight` asked of `role`: the app's own semibold
/// as `semibold` says (`Engine::set_semibold`), anything else as asked.
fn ui_weight(role: Role, weight: u16, semibold: Option<u16>) -> u16 {
    match semibold {
        Some(w) if role == Role::Ui && weight == 600 => w,
        _ => weight,
    }
}

/// Fonts and the glyph picture cache. One per app.
/// Draws a colour glyph the engine cannot read itself (iOS keeps its emoji
/// in Apple's own compressed pictures, which only the OS decodes): the
/// face's PostScript name, the glyph's id and its size in pixels, to its
/// picture (premultiplied RGBA). Given by the shell; nothing else uses it.
pub type ColorGlyphs = Arc<dyn Fn(&str, u16, f32) -> Option<Mask> + Send + Sync>;

pub struct Engine {
    /// The OS's drawing of colour glyphs it keeps in a form of its own.
    color: Option<ColorGlyphs>,
    /// The weight the app's own text draws semibold in (`set_semibold`).
    semibold: Option<u16>,
    /// What the first fonts hold beyond `needs_more_fonts`'s guess (`set_covered`).
    covered: String,
    fs: FontSystem,
    masks: HashMap<CacheKey, Option<Arc<Mask>>>,
    swash: SwashCache,
    set: FontSet,
    generation: u32,
    fallback_loaded: bool,
    /// The families `load_faces` has loaded.
    faces: Vec<String>,
}

impl Engine {
    /// Loads the named files only (a missing one is skipped).
    pub fn new(set: FontSet) -> Engine {
        let mut db = fontdb::Database::new();
        for f in &set.files {
            let _ = db.load_font_file(f);
        }
        for d in &set.data {
            db.load_font_data(d.to_vec());
        }
        db.set_sans_serif_family(set.ui.clone());
        db.set_monospace_family(set.code.clone());
        Engine { color: None, covered: String::new(), semibold: None, fs: FontSystem::new_with_locale_and_db("en-US".into(), db), masks: HashMap::new(), swash: SwashCache::new(), set, generation: 0, fallback_loaded: false, faces: Vec::new() }
    }

    /// Loads more font files (a missing one is skipped): the faces of a
    /// family the user picks after launch.
    pub fn load_files(&mut self, files: &[PathBuf]) {
        let db = self.fs.db_mut();
        for f in files {
            let _ = db.load_font_file(f);
        }
    }

    /// The number of faces loaded.
    pub fn faces(&self) -> usize {
        self.fs.db().faces().count()
    }

    /// Loads the fallback fonts the first time `text` needs them.
    fn ensure_fonts(&mut self, text: &str) {
        if self.fallback_loaded || !needs_more_fonts(text) {
            return;
        }
        // What the first fonts are known to hold (the keys' ⌘ in a symbol
        // face loaded at launch) asks for nothing more.
        if !self.covered.is_empty() && !text.chars().any(|c| needs_more_fonts(c.encode_utf8(&mut [0; 4])) && !self.covered.contains(c)) {
            return;
        }
        self.load_fallback();
    }

    /// Characters the first fonts are known to hold although they are past
    /// what `needs_more_fonts` takes them to have: text with no others
    /// loads no fallback (iOS loads a symbol face at launch for the ⌘ the
    /// first screen shows, and its fallbacks are many and large).
    pub fn set_covered(&mut self, chars: &str) {
        self.covered = chars.to_string();
    }

    /// The weight the app's own text (`Role::Ui`) draws semibold (600) in,
    /// where its face has none and the nearest would be bold (the Mac's
    /// Helvetica Neue: Medium, 500). None: as asked. Set before drawing.
    pub fn set_semibold(&mut self, weight: Option<u16>) {
        self.semibold = weight;
    }

    /// The family names of the faces loaded, sorted.
    pub fn families(&self) -> Vec<String> {
        let mut names: Vec<String> = self.fs.db().faces().flat_map(|f| f.families.iter().map(|(n, _)| n.clone())).collect();
        names.sort();
        names.dedup();
        names
    }

    /// The colour emoji family among the faces loaded (Apple Color Emoji,
    /// Segoe UI Emoji, Noto Color Emoji), if there is one.
    fn emoji_family(&self) -> Option<String> {
        let names = self.families();
        names.iter().find(|n| n.contains("Color Emoji")).or_else(|| names.iter().find(|n| n.contains("Emoji"))).cloned()
    }

    /// Changes the family a role is set in (the user picked another font
    /// for notes). Layouts made before keep what they were made with; the
    /// shell builds new ones when [`Engine::generation`] has moved.
    pub fn set_family(&mut self, role: Role, name: &str) {
        let slot = match role {
            Role::Ui => &mut self.set.ui,
            Role::Text => &mut self.set.text,
            Role::Code => &mut self.set.code,
            Role::Icon => &mut self.set.icon,
        };
        if slot != name {
            *slot = name.to_string();
            self.generation += 1;
            // A face launch left out (the serif or monospace the user moves
            // to) is among the fallback files: load them now, not whenever
            // some other script happens to ask.
            if !self.fallback_loaded && !self.families().iter().any(|f| f == name) {
                self.load_fallback();
            }
        }
    }

    /// Faces that ship inside the program for a moment of their own (the
    /// first run's welcome): loaded the first time it is drawn, never at
    /// launch.
    pub fn load_faces(&mut self, family: &str, data: &[&'static [u8]]) {
        if self.faces.iter().any(|f| f == family) {
            return;
        }
        self.faces.push(family.to_string());
        let db = self.fs.db_mut();
        for d in data {
            db.load_font_data(d.to_vec());
        }
    }

    fn load_fallback(&mut self) {
        self.fallback_loaded = true;
        let db = self.fs.db_mut();
        for f in &self.set.fallback {
            let _ = db.load_font_file(f);
        }
    }

    /// The shell's drawing of colour glyphs the engine cannot read (iOS's emoji).
    pub fn set_color_glyphs(&mut self, draw: ColorGlyphs) {
        self.color = Some(draw);
        self.masks.clear();
    }

    /// Counts the changes made by `set_family`.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// The picture of one glyph, rendered once and kept. None for a glyph
    /// with nothing to draw, or one too small to see (a hidden mark).
    pub fn mask(&mut self, g: &Glyph) -> Option<Arc<Mask>> {
        if g.size < MIN_VISIBLE {
            return None;
        }
        if let Some(m) = self.masks.get(&g.key.0) {
            return m.clone();
        }
        let mut m = self.swash.get_image_uncached(&mut self.fs, g.key.0).map(|img| {
            Arc::new(Mask {
                left: img.placement.left,
                top: img.placement.top,
                width: img.placement.width,
                height: img.placement.height,
                color: matches!(img.content, SwashContent::Color),
                data: img.data,
            })
        });
        // An emoji with no colour picture here (iOS's come out as grey
        // stand-in shapes): the OS draws it, where it can.
        if m.as_ref().is_none_or(|m| !m.color) && let Some(draw) = &self.color && let Some(face) = self.fs.db().face(g.key.0.font_id) && face.families.iter().any(|(n, _)| n.contains("Emoji")) {
            let drawn = draw(&face.post_script_name, g.key.0.glyph_id, f32::from_bits(g.key.0.font_size_bits)).filter(|m| m.color && m.data.len() == (m.width * m.height * 4) as usize);
            if let Some(d) = drawn {
                m = Some(Arc::new(d));
            }
        }
        self.masks.insert(g.key.0, m.clone());
        m
    }
}

/// Whether `text` has anything beyond what the first fonts cover: letters
/// of other scripts, symbols, emoji. Latin (with its extensions), general
/// punctuation and the private-use area (icons) do not.
pub fn needs_more_fonts(text: &str) -> bool {
    text.chars().any(|c| c as u32 >= 0x250 && !matches!(c as u32, 0x2000..=0x206F | 0xE000..=0xF8FF))
}

/// Glyphs set smaller than this (em, pixels) are hidden marks: no picture.
const MIN_VISIBLE: f32 = 0.5;

/// A glyph's picture: one byte of coverage per pixel, or four (RGBA)
/// when `color`.
#[derive(Debug)]
pub struct Mask {
    /// Pixels from the glyph's origin to the picture's left edge, and from
    /// its baseline up to the top.
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub color: bool,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    /// The side the paragraph starts on: left, or right for right to left.
    #[default]
    Start,
    Center,
    End,
}

/// How a paragraph is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Format {
    pub role: Role,
    /// A face to set this in instead of the role's (a display face for a
    /// heading), used when the machine has it; else the role's.
    pub face: Option<&'static str>,
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
    /// Every line is this tall, whatever the sizes on it. None: 1.25 times
    /// the size.
    pub line: Option<f32>,
    /// Where the baseline sits in a line, as a share of its height (0.8: a
    /// fifth of the line is below the letters). None: centred on the font.
    pub baseline: Option<f32>,
    pub align: Align,
    /// Wrap at the width. Off, the paragraph is one line, and what passes
    /// the width is cut (whole glyphs) or, with `ellipsis`, ends in "…".
    pub wrap: bool,
    pub ellipsis: bool,
    /// Force the direction: Some(true) reads right to left (a blank line
    /// after Arabic stays Arabic), Some(false) left to right (an Arabic
    /// gist keeps its place on the line). None: the first strong letter.
    pub rtl: Option<bool>,
    /// Centre the lines in the height given.
    pub center_y: bool,
}

impl Format {
    pub fn new(role: Role, size: f32) -> Format {
        Format { role, face: None, size, weight: 400, italic: false, line: None, baseline: None, align: Align::Start, wrap: true, ellipsis: false, rtl: None, center_y: false }
    }

    fn line_height(&self) -> f32 {
        self.line.unwrap_or(self.size * 1.25)
    }

    /// A number that is the same for formats that are the same, to key
    /// caches by.
    pub fn id(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.role, self.size.to_bits(), self.weight, self.italic, self.line.map(f32::to_bits), self.baseline.map(f32::to_bits)).hash(&mut h);
        (self.align, self.wrap, self.ellipsis, self.rtl, self.center_y).hash(&mut h);
        h.finish()
    }
}

/// A change to a range of the paragraph. Later styles win over earlier
/// ones where they overlap.
#[derive(Clone, Debug, Default)]
pub struct Style {
    pub range: Range<usize>,
    pub size: Option<f32>,
    pub weight: Option<u16>,
    pub italic: Option<bool>,
    pub role: Option<Role>,
    pub strike: bool,
    /// Which ink the shell draws these glyphs in (0: the paragraph's own).
    pub ink: u32,
    /// Room after each character, DIPs: a hidden character this wide
    /// keeps a place in the line for something drawn over it (math).
    pub spacing: Option<f32>,
}

impl Style {
    pub fn new(range: Range<usize>) -> Style {
        Style { range, ..Style::default() }
    }
}

/// A line as the paragraph wrapped: its bytes, and its place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualLine {
    pub start: usize,
    pub end: usize,
    pub top: f32,
    pub height: f32,
}

/// Where a caret is drawn: its x, and its line's top and height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub x: f32,
    pub y: f32,
    pub height: f32,
}

/// What a laid-out paragraph measures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    /// The widest line, trailing spaces included.
    pub width: f32,
    pub height: f32,
    pub lines: usize,
}

/// What a glyph is: the same for the same glyph at the same size and
/// quarter-pixel place, whatever layout it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey(CacheKey);

/// One glyph placed in whole pixels.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    /// Where its origin is on the baseline, pixels.
    pub x: i32,
    pub y: i32,
    pub ink: u32,
    /// Its size in pixels; a hidden mark is a hair.
    pub size: f32,
    key: GlyphKey,
}

impl Glyph {
    pub fn key(&self) -> GlyphKey {
        self.key
    }
}

/// A strikethrough: x, y, width, height in DIPs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strike {
    pub rect: [f32; 4],
    pub ink: u32,
}

const RLM: &str = "\u{200F}";
const LRM: &str = "\u{200E}";
/// How many bytes a direction mark takes.
const MARK: usize = 3;
const STRIKE_BIT: usize = 1 << 40;

/// Where, in a paragraph read one way, words written the other way carry
/// brackets or quotes ("(Atlas", "5.5)", `"Lantern"` in Arabic; "(مرحبا)"
/// in English): just outside that punctuation, which a mark of the word's
/// own direction then keeps with it. Without it the punctuation takes the
/// paragraph's direction, and the change of direction between it and the
/// word is a place the line may wrap, leaving a bracket alone at a line's
/// end. Byte offsets into `text`, in order; none unless it mixes scripts.
fn word_marks(text: &str, rtl: bool) -> Vec<usize> {
    let rtl_char = |c: char| {
        let u = c as u32;
        (0x0590..=0x08FF).contains(&u) || (0xFB1D..=0xFDFF).contains(&u) || (0xFE70..=0xFEFF).contains(&u)
    };
    let ltr_char = |c: char| c.is_alphabetic() && !rtl_char(c);
    // Plain ASCII (most lines) has no right-to-left letter: nothing to look for.
    if text.is_ascii() || !(text.chars().any(rtl_char) && text.chars().any(ltr_char)) {
        return Vec::new();
    }
    let mut marks = Vec::new();
    let mut start = None;
    let mut word = |a: usize, b: usize, marks: &mut Vec<usize>| {
        let w = &text[a..b];
        let other = if rtl { w.chars().any(ltr_char) && !w.chars().any(rtl_char) } else { w.chars().any(rtl_char) && !w.chars().any(ltr_char) };
        if !other {
            return;
        }
        if w.chars().next().is_some_and(|c| !c.is_alphanumeric()) {
            marks.push(a);
        }
        if w.chars().next_back().is_some_and(|c| !c.is_alphanumeric()) {
            marks.push(b);
        }
    };
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(a)) => {
                word(a, i, &mut marks);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(a) = start {
        word(a, text.len(), &mut marks);
    }
    marks
}

/// One paragraph, shaped and laid out.
pub struct Layout {
    buf: Buffer,
    /// Bytes of the invisible direction mark ahead of the text.
    prefix: usize,
    /// Where marks of the other direction stand in the text (see
    /// `word_marks`), each `MARK` bytes in what is shaped.
    marks: Vec<usize>,
    text: String,
    /// Cut to fit with an ellipsis: the whole text and its styles, as asked
    /// for (`text`, `with_width`).
    source: Option<(String, Vec<Style>)>,
    format: Format,
    styles: Vec<Style>,
    line: f32,
    width: f32,
    height: f32,
    y0: f32,
}

impl Layout {
    /// Lays `text` out in `width` (and `height`, for centring) with `styles`
    /// over it. One line with an ellipsis that does not fit is cut where its
    /// text is, not where its glyphs are: the longest start of it that fits
    /// with "\u{2026}" after it, laid out in the paragraph's direction, so
    /// the ellipsis is at the paragraph's visual end even where words of the
    /// other direction come last (shaping's own ellipsis follows the glyph
    /// order, and in mixed text it stood mid-line).
    pub fn new(e: &mut Engine, text: &str, format: &Format, width: f32, height: f32, styles: &[Style]) -> Layout {
        if !(format.ellipsis && !format.wrap) || text.is_empty() {
            return Layout::shaped(e, text, format, width, height, styles);
        }
        let plain = Format { ellipsis: false, ..*format };
        let whole = Layout::shaped(e, text, &plain, width, height, styles);
        if whole.natural_width() <= width.max(1.0) + 0.5 {
            return whole;
        }
        // The text's own direction, whatever its last words are.
        let rtl = format.rtl.unwrap_or_else(|| whole.is_rtl());
        let plain = Format { rtl: Some(rtl), ..plain };
        // A mark of the paragraph's own direction ahead of the ellipsis: after
        // a word of the other (and the mark that keeps that word whole), it
        // would go with that word, mid-line.
        let mark = if rtl { RLM } else { LRM };
        let cut = |n: usize| format!("{}{mark}\u{2026}", text[..n].trim_end());
        let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).skip(1).collect();
        let fits = |e: &mut Engine, n: usize| Layout::shaped(e, &cut(n), &plain, width, height, &[]).natural_width() <= width.max(1.0) + 0.5;
        let (mut lo, mut hi) = (0usize, ends.len());
        // The most characters that fit: `lo` of them do, `hi` do not.
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if fits(e, ends[mid - 1]) { lo = mid } else { hi = mid }
        }
        let n = if lo == 0 { 0 } else { ends[lo - 1] };
        let kept: Vec<Style> = styles.iter().filter(|st| st.range.start < n).map(|st| Style { range: st.range.start..st.range.end.min(n), ..st.clone() }).collect();
        let mut l = Layout::shaped(e, &cut(n), &plain, width, height, &kept);
        // What it says, and what a wider one would show, is the whole.
        l.source = Some((text.to_string(), styles.to_vec()));
        l.format = *format;
        l
    }

    /// The widest line's own width, cut by nothing.
    fn natural_width(&self) -> f32 {
        self.buf.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
    }

    fn shaped(e: &mut Engine, text: &str, format: &Format, width: f32, height: f32, styles: &[Style]) -> Layout {
        let width = width.max(1.0);
        let height = height.max(1.0);
        e.ensure_fonts(text);
        let line = format.line_height();
        let (ui, txt, code, icon) = (e.set.ui.clone(), e.set.text.clone(), e.set.code.clone(), e.set.icon.clone());
        let semibold = e.semibold;
        let name = |r: Role| match r {
            Role::Ui => ui.as_str(),
            Role::Text => txt.as_str(),
            Role::Code => code.as_str(),
            Role::Icon => icon.as_str(),
        };
        let attrs = |size: f32, weight: u16, italic: bool, role: Role, meta: usize| {
            let weight = ui_weight(role, weight, semibold);
            let mut a = Attrs::new().family(Family::Name(name(role))).weight(Weight(weight)).metrics(Metrics::new(size.max(0.01), line)).metadata(meta);
            if italic {
                a = a.style(CStyle::Italic);
            }
            a
        };
        let mut base = attrs(format.size, format.weight, format.italic, format.role, 0);
        // The face asked for, where the machine has it (a face launch left
        // out is among the fallback files).
        let face = format.face.filter(|face| {
            if !e.fallback_loaded && !e.families().iter().any(|f| f == face) {
                e.load_fallback();
            }
            e.families().iter().any(|f| f == face)
        });
        if let Some(face) = face {
            base = base.family(Family::Name(face));
        }
        // Emoji (characters that are emoji by default, and ones asked for
        // as emoji by a U+FE0F after them) are set in the colour emoji face:
        // chosen by the character alone, a heart picked from the emoji panel
        // (❤️) drew as the black-and-white text heart a symbol font has
        // first, and a 😀 on Linux as DejaVu Sans's plain one.
        let emoji = emoji_ranges(text);
        let emoji_family = if emoji.is_empty() { None } else { e.emoji_family() };

        let mark = match format.rtl {
            Some(true) => RLM,
            Some(false) => LRM,
            None => "",
        };
        let prefix = mark.len();
        let marks = format.rtl.map_or_else(Vec::new, |rtl| word_marks(text, rtl));
        let other = if format.rtl == Some(true) { LRM } else { RLM };
        let mut cuts: Vec<usize> = vec![0, text.len()];
        cuts.extend(&marks);
        if emoji_family.is_some() {
            cuts.extend(emoji.iter().flat_map(|r| [r.start, r.end]));
        }
        for s in styles {
            for b in [s.range.start, s.range.end] {
                if b <= text.len() && text.is_char_boundary(b) {
                    cuts.push(b);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        if prefix > 0 {
            spans.push((mark, base.clone()));
        }
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (mut size, mut weight, mut italic, mut role, mut ink, mut strike) = (format.size, format.weight, format.italic, format.role, 0u32, false);
            let mut spacing = None;
            for s in styles.iter().filter(|s| s.range.start <= a && s.range.end >= b) {
                size = s.size.unwrap_or(size);
                spacing = s.spacing.or(spacing);
                weight = s.weight.unwrap_or(weight);
                italic = s.italic.unwrap_or(italic);
                role = s.role.unwrap_or(role);
                if s.ink != 0 {
                    ink = s.ink;
                }
                strike |= s.strike;
            }
            let meta = ink as usize | if strike { STRIKE_BIT } else { 0 };
            let mut a_ = attrs(size, weight, italic, role, meta);
            // Spacing is in ems of the run's size.
            if let Some(room) = spacing {
                a_ = a_.letter_spacing(room / size.max(0.01));
            }
            if let (Some(face), true) = (face, role == format.role) {
                a_ = a_.family(Family::Name(face));
            }
            if let Some(family) = &emoji_family {
                if emoji.iter().any(|r| r.start <= a && b <= r.end) {
                    a_ = a_.family(Family::Name(family));
                }
            }
            if marks.binary_search(&a).is_ok() {
                spans.push((other, a_.clone()));
            }
            spans.push((&text[a..b], a_));
        }
        if marks.last() == Some(&text.len()) {
            spans.push((other, base.clone()));
        }

        let mut buf = Buffer::new(&mut e.fs, Metrics::new(format.size.max(0.01), line));
        buf.set_wrap(if format.wrap { Wrap::WordOrGlyph } else { Wrap::None });
        buf.set_ellipsize(Ellipsize::None);
        buf.set_size(Some(width), None);
        let align = match format.align {
            Align::Start => None,
            Align::Center => Some(CAlign::Center),
            Align::End => Some(CAlign::End),
        };
        if text.is_empty() && prefix == 0 {
            buf.set_text("", &base, Shaping::Advanced, align);
        } else {
            buf.set_rich_text(spans, &base, Shaping::Advanced, align);
        }
        buf.shape_until_scroll(&mut e.fs, false);

        let mut l = Layout { buf, prefix, marks, text: text.to_string(), source: None, format: *format, styles: styles.to_vec(), line, width, height, y0: 0.0 };
        if format.center_y {
            l.y0 = ((height - l.measure().height) / 2.0).max(0.0);
        }
        l
    }

    /// The same paragraph in another width.
    pub fn with_width(&self, e: &mut Engine, width: f32) -> Layout {
        match &self.source {
            Some((text, styles)) => Layout::new(e, text, &self.format, width, self.height, styles),
            None => Layout::new(e, &self.text, &self.format, width, self.height, &self.styles),
        }
    }

    /// The width it was laid out in.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// The height it was laid out in (its box's, not its lines').
    pub fn box_height(&self) -> f32 {
        self.height
    }

    /// The format it was laid out in.
    pub fn format(&self) -> &Format {
        &self.format
    }

    /// The text laid out, whole even where it is cut to fit (a screen
    /// reader names a control by it).
    pub fn text(&self) -> &str {
        self.source.as_ref().map_or(&self.text, |(t, _)| t)
    }

    /// What shows: the text, or where it is cut to fit, its start and "…".
    pub fn cut_text(&self) -> &str {
        &self.text
    }

    /// What it is set as: an icon, the app's own words, the note's.
    pub fn role(&self) -> Role {
        self.format.role
    }

    fn clips(&self) -> bool {
        !self.format.wrap && !self.format.ellipsis
    }

    /// A byte of the text, where it is in what was shaped: past the marks
    /// before it (and one standing at it).
    fn pos_in(&self, p: usize) -> usize {
        p + self.prefix + MARK * self.marks.partition_point(|&m| m <= p)
    }

    /// A byte of what was shaped, back in the text: a mark is where it stands.
    fn pos_out(&self, p: usize) -> usize {
        let raw = p.saturating_sub(self.prefix);
        let mut passed = 0;
        for (k, &m) in self.marks.iter().enumerate() {
            // The mark's place in what was shaped.
            let at = m + MARK * k;
            if raw < at {
                break;
            }
            if raw < at + MARK {
                return m;
            }
            passed = k + 1;
        }
        (raw - MARK * passed).min(self.text.len())
    }

    /// Whether a glyph is drawn at all: one-line text without an ellipsis
    /// loses the whole glyphs that pass its width.
    fn shown(&self, g: &cosmic_text::LayoutGlyph) -> bool {
        !self.clips() || (g.x >= -0.01 && g.x + g.w <= self.width + 0.01)
    }

    /// Width of the widest line, trailing spaces included; height; and the
    /// number of lines.
    pub fn measure(&self) -> Measure {
        let (mut w, mut n) = (0.0f32, 0usize);
        for run in self.buf.layout_runs() {
            w = w.max(run.line_w);
            n += 1;
        }
        if self.clips() {
            w = w.min(self.width);
        }
        Measure { width: w, height: n as f32 * self.line, lines: n.max(1) }
    }

    /// The wrapped lines, top to bottom.
    pub fn lines(&self) -> Vec<VisualLine> {
        let mut out: Vec<VisualLine> = Vec::new();
        let mut prev_end = 0;
        for run in self.buf.layout_runs() {
            let (mut s, mut e) = (usize::MAX, 0usize);
            for g in run.glyphs {
                s = s.min(g.start);
                e = e.max(g.end);
            }
            let (start, end) = if s == usize::MAX { (self.pos_out(prev_end), self.pos_out(prev_end)) } else { (self.pos_out(s), self.pos_out(e)) };
            prev_end = if s == usize::MAX { prev_end } else { e };
            out.push(VisualLine { start, end, top: run.line_top + self.y0, height: self.line });
        }
        if out.is_empty() {
            out.push(VisualLine { start: 0, end: 0, top: self.y0, height: self.line });
        }
        // A space where a line wraps hangs off its end and has no glyph; the
        // line still owns it, so lines meet with no byte between them.
        for i in 1..out.len() {
            out[i - 1].end = out[i].start;
        }
        if let Some(last) = out.last_mut() {
            last.end = self.text.len();
        }
        out
    }

    /// Whether the paragraph reads right to left.
    pub fn is_rtl(&self) -> bool {
        match self.format.rtl {
            Some(forced) => forced,
            None => self.buf.layout_runs().next().is_some_and(|r| r.rtl),
        }
    }

    fn empty_x(&self) -> f32 {
        match (self.format.align, self.is_rtl()) {
            (Align::Center, _) => self.width / 2.0,
            (Align::Start, true) | (Align::End, false) => self.width,
            _ => 0.0,
        }
    }

    /// Where a caret at byte `at` is drawn (before the cluster starting there).
    pub fn caret(&self, at: usize) -> Caret {
        let p = self.pos_in(at.min(self.text.len()));
        // The line where the cluster at `p` starts; failing that (a hanging
        // space, or the end of the text) the last line that starts before it.
        let mut hanging: Option<Caret> = None;
        for run in self.buf.layout_runs() {
            let y = run.line_top + self.y0;
            let mut end_x: Option<(usize, f32)> = None;
            let mut first = usize::MAX;
            for g in run.glyphs {
                if g.start <= p && p < g.end {
                    let x = if g.level.is_rtl() { g.x + g.w } else { g.x };
                    return Caret { x, y, height: self.line };
                }
                first = first.min(g.start);
                // The trailing edge of the logically last cluster.
                let edge = if g.level.is_rtl() { g.x } else { g.x + g.w };
                if end_x.is_none_or(|(e, _)| g.end >= e) {
                    end_x = Some((g.end, edge));
                }
            }
            if first != usize::MAX && first > p {
                break;
            }
            hanging = Some(Caret { x: end_x.map_or(self.empty_x(), |(_, x)| x), y, height: self.line });
        }
        hanging.unwrap_or(Caret { x: self.empty_x(), y: self.y0, height: self.line })
    }

    /// The caret position nearest the point, and whether the point is
    /// inside the text's lines.
    pub fn offset_at(&self, x: f32, y: f32) -> (usize, bool) {
        let y = y - self.y0;
        let runs: Vec<_> = self.buf.layout_runs().collect();
        let Some(first) = runs.first() else { return (0, false) };
        let inside_y = y >= 0.0 && y < runs.len() as f32 * self.line;
        let run = runs.iter().find(|r| y < r.line_top + self.line).unwrap_or(runs.last().unwrap_or(first));
        let mut gs: Vec<&cosmic_text::LayoutGlyph> = run.glyphs.iter().collect();
        if gs.is_empty() {
            return (self.pos_out(0), false);
        }
        gs.sort_by(|a, b| a.x.total_cmp(&b.x));
        let (left, right) = (gs[0], gs[gs.len() - 1]);
        // A cluster's right edge is its end when it reads left to right, its
        // start when it reads right to left; its left edge the other.
        let edge = |g: &cosmic_text::LayoutGlyph, right_edge: bool| -> usize { if right_edge != g.level.is_rtl() { g.end } else { g.start } };
        // Left of everything: the visually first glyph's left edge (past the
        // end of a right-to-left line); right of everything: the last's
        // right edge.
        if x <= left.x {
            return (self.pos_out(edge(left, false)), inside_y && x >= 0.0);
        }
        if x >= right.x + right.w {
            return (self.pos_out(edge(right, true)), inside_y && x <= self.width);
        }
        for g in gs {
            if x >= g.x && x < g.x + g.w {
                let right_half = x >= g.x + g.w / 2.0;
                // Right half of an LTR cluster is its end; of an RTL cluster, its start.
                let p = if right_half != g.level.is_rtl() { g.end } else { g.start };
                return (self.pos_out(p), inside_y);
            }
        }
        (self.pos_out(right.end), inside_y)
    }

    /// The rectangles (x, y, width, height) covering bytes `from..to`, a
    /// piece per line (and per direction change).
    pub fn range(&self, from: usize, to: usize) -> Vec<[f32; 4]> {
        let (a, b) = (self.pos_in(from.min(self.text.len())), self.pos_in(to.min(self.text.len())));
        let mut out = Vec::new();
        if a >= b {
            return out;
        }
        for run in self.buf.layout_runs() {
            let y = run.line_top + self.y0;
            let mut spans: Vec<(f32, f32)> = run.glyphs.iter().filter(|g| g.start < b && g.end > a).map(|g| (g.x, g.x + g.w)).collect();
            spans.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut merged: Vec<(f32, f32)> = Vec::new();
            for s in spans {
                match merged.last_mut() {
                    Some(m) if s.0 <= m.1 + 0.5 => m.1 = m.1.max(s.1),
                    _ => merged.push(s),
                }
            }
            out.extend(merged.into_iter().map(|(l, r)| [l, y, r - l, self.line]));
        }
        out
    }

    /// Where a line's baseline is, from the layout's top.
    fn baseline_of(&self, run: &cosmic_text::LayoutRun) -> f32 {
        match self.format.baseline {
            Some(b) => run.line_top + b * self.line,
            None => run.line_y,
        }
    }

    /// Every glyph that is drawn, placed in whole pixels with the layout's
    /// top left at `origin` (pixels) when scaled by `scale`.
    pub fn glyphs(&self, origin: (f32, f32), scale: f32) -> Vec<Glyph> {
        let mut out = Vec::new();
        for run in self.buf.layout_runs() {
            let base = self.baseline_of(&run) + self.y0;
            for g in run.glyphs.iter().filter(|g| self.shown(g)) {
                let p = g.physical((origin.0, origin.1 + base * scale), scale);
                out.push(Glyph { x: p.x, y: p.y, ink: (g.metadata & (STRIKE_BIT - 1)) as u32, size: g.font_size * scale, key: GlyphKey(p.cache_key) });
            }
        }
        out
    }

    /// The glyph whose cluster starts at byte `at`, if one is drawn there:
    /// its left edge and its line's baseline, from the layout's top left.
    pub fn glyph_at(&self, at: usize) -> Option<(f32, f32)> {
        let p = self.pos_in(at);
        for run in self.buf.layout_runs() {
            if let Some(g) = run.glyphs.iter().find(|g| g.start == p) {
                return Some((g.x, self.baseline_of(&run) + self.y0));
            }
        }
        None
    }

    /// Strikethroughs, one per stretch of struck glyphs on a line.
    pub fn strikes(&self) -> Vec<Strike> {
        let mut out = Vec::new();
        for run in self.buf.layout_runs() {
            let base = self.baseline_of(&run) + self.y0;
            let mut open: Option<(f32, f32, u32)> = None;
            let mut y = 0.0;
            let mut thick = 1.0;
            for g in run.glyphs.iter().filter(|g| self.shown(g)) {
                if g.metadata & STRIKE_BIT != 0 {
                    y = base - g.font_size * 0.32;
                    thick = (g.font_size / 16.0).max(1.0);
                    open = Some(match open {
                        Some((l, r, ink)) => (l.min(g.x), r.max(g.x + g.w), ink),
                        None => (g.x, g.x + g.w, (g.metadata & (STRIKE_BIT - 1)) as u32),
                    });
                } else if let Some((l, r, ink)) = open.take() {
                    out.push(Strike { rect: [l, y, r - l, thick], ink });
                }
            }
            if let Some((l, r, ink)) = open {
                out.push(Strike { rect: [l, y, r - l, thick], ink });
            }
        }
        out
    }
}

/// The stretches of `text` drawn as emoji, byte ranges: a character that is
/// an emoji by default (unless a U+FE0E after it asks for text) or has a
/// U+FE0F after it, with what joins it (a keycap's U+20E3, a skin tone, a
/// flag's tags, a U+200D and the character after that). Emoji side by side
/// are one stretch.
fn emoji_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        if !(next == Some('\u{FE0F}') || (next != Some('\u{FE0E}') && emoji_by_default(c))) {
            continue;
        }
        let mut end = i + c.len_utf8();
        while let Some(&(j, n)) = chars.peek() {
            if !matches!(n, '\u{FE0F}' | '\u{20E3}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}' | '\u{200D}') {
                break;
            }
            chars.next();
            end = j + n.len_utf8();
            if n == '\u{200D}' {
                if let Some((k, after)) = chars.next() {
                    end = k + after.len_utf8();
                }
            }
        }
        match out.last_mut() {
            Some(last) if last.end == i => last.end = end,
            _ => out.push(i..end),
        }
    }
    out
}

/// Whether `c` is drawn as an emoji when nothing asks otherwise (Unicode
/// 16.0's Emoji_Presentation): 😀 and ⭐ are, ✓ and ❤ are not.
fn emoji_by_default(c: char) -> bool {
    let c = c as u32;
    c >= 0x231A && EMOJI_BY_DEFAULT.binary_search_by(|&(a, b)| if b < c { std::cmp::Ordering::Less } else if a > c { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal }).is_ok()
}

/// Emoji_Presentation=Yes from Unicode's emoji-data.txt, inclusive ranges.
const EMOJI_BY_DEFAULT: &[(u32, u32)] = &[
    (0x231A, 0x231B), (0x23E9, 0x23EC), (0x23F0, 0x23F0), (0x23F3, 0x23F3), (0x25FD, 0x25FE), (0x2614, 0x2615), (0x2648, 0x2653),
    (0x267F, 0x267F), (0x2693, 0x2693), (0x26A1, 0x26A1), (0x26AA, 0x26AB), (0x26BD, 0x26BE), (0x26C4, 0x26C5), (0x26CE, 0x26CE),
    (0x26D4, 0x26D4), (0x26EA, 0x26EA), (0x26F2, 0x26F3), (0x26F5, 0x26F5), (0x26FA, 0x26FA), (0x26FD, 0x26FD), (0x2705, 0x2705),
    (0x270A, 0x270B), (0x2728, 0x2728), (0x274C, 0x274C), (0x274E, 0x274E), (0x2753, 0x2755), (0x2757, 0x2757), (0x2795, 0x2797),
    (0x27B0, 0x27B0), (0x27BF, 0x27BF), (0x2B1B, 0x2B1C), (0x2B50, 0x2B50), (0x2B55, 0x2B55),
    (0x1F004, 0x1F004), (0x1F0CF, 0x1F0CF), (0x1F18E, 0x1F18E), (0x1F191, 0x1F19A), (0x1F1E6, 0x1F1FF), (0x1F201, 0x1F201),
    (0x1F21A, 0x1F21A), (0x1F22F, 0x1F22F), (0x1F232, 0x1F236), (0x1F238, 0x1F23A), (0x1F250, 0x1F251), (0x1F300, 0x1F320),
    (0x1F32D, 0x1F335), (0x1F337, 0x1F37C), (0x1F37E, 0x1F393), (0x1F3A0, 0x1F3CA), (0x1F3CF, 0x1F3D3), (0x1F3E0, 0x1F3F0),
    (0x1F3F4, 0x1F3F4), (0x1F3F8, 0x1F43E), (0x1F440, 0x1F440), (0x1F442, 0x1F4FC), (0x1F4FF, 0x1F53D), (0x1F54B, 0x1F54E),
    (0x1F550, 0x1F567), (0x1F57A, 0x1F57A), (0x1F595, 0x1F596), (0x1F5A4, 0x1F5A4), (0x1F5FB, 0x1F64F), (0x1F680, 0x1F6C5),
    (0x1F6CC, 0x1F6CC), (0x1F6D0, 0x1F6D2), (0x1F6D5, 0x1F6D7), (0x1F6DC, 0x1F6DF), (0x1F6EB, 0x1F6EC), (0x1F6F4, 0x1F6FC),
    (0x1F7E0, 0x1F7EB), (0x1F7F0, 0x1F7F0), (0x1F90C, 0x1F93A), (0x1F93C, 0x1F945), (0x1F947, 0x1F9FF), (0x1FA70, 0x1FA7C),
    (0x1FA80, 0x1FA89), (0x1FA8F, 0x1FAC6), (0x1FACE, 0x1FADC), (0x1FADF, 0x1FAE9), (0x1FAF0, 0x1FAF8),
];

#[cfg(test)]
mod tests;
