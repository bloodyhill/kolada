//! Builds Kolada's icon font from open (hollow) SVG icons.
//!
//! Windows draws the app's icons from a font that ships with Windows (Segoe
//! Fluent Icons), by private-use codepoints the code names (`\u{E713}` is
//! settings). Other platforms have no such font, so this makes one with the
//! same codepoints, from icons on allsvgicons.com (Lucide, ISC; Qlementine
//! Icons, MIT; see THIRD-PARTY.md), so the shared screen code draws them the
//! same way everywhere with no change.
//!
//!   cargo run -p iconfont          writes assets/fonts/KoladaIcons.ttf
//!
//! `assets/icons/map.tsv` says which icon each codepoint is. Lucide's icons
//! are strokes; a font holds only fills, so each stroke is turned into the
//! outline of its shape (round caps and joins, as Lucide draws them). The
//! font is plain TrueType: straight-edged contours (the SVG curves are
//! flattened finely), one em square per icon, y up.

use std::path::{Path, PathBuf};

use kolada_ui::vector::subpaths;

type Point = (f32, f32);

/// The em, in font units; an icon's 24-unit square fills it.
const EM: i32 = 1000;
const ASCENT: i32 = 800;
const DESCENT: i32 = -200;
/// Segments per quarter turn when flattening curves.
const DETAIL: usize = 12;
/// Points around a round cap or join.
const ROUND: usize = 16;

thread_local! {
    /// Coarser curves for `--outline`: a mark a few pixels across needs a
    /// fraction of the points a font's em does, and each costs at launch.
    static COARSE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn detail() -> usize {
    if COARSE.get() { 3 } else { DETAIL }
}

fn round() -> usize {
    if COARSE.get() { 8 } else { ROUND }
}

struct Glyph {
    code: u16,
    contours: Vec<Vec<(i16, i16)>>,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// The codepoints and icons in `map`: `E713<TAB>settings`, or
/// `E710<TAB>qlementine-icons:new-24`; `#` for comments.
fn parse_map(map: &str) -> Vec<(u16, String)> {
    map.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (code, name) = l.split_once('\t').unwrap_or_else(|| panic!("a tab between code and name: {l:?}"));
            (u16::from_str_radix(code.trim(), 16).unwrap_or_else(|_| panic!("a hex codepoint: {code:?}")), name.trim().to_string())
        })
        .collect()
}

/// An icon's file: `pack:name` is assets/icons/pack/name.svg, a bare name is Lucide's.
fn icon_file(icons: &Path, icon: &str) -> PathBuf {
    let (pack, name) = icon.split_once(':').unwrap_or(("lucide", icon));
    icons.join(pack).join(format!("{name}.svg"))
}

/// A tag's attributes, `key="value"`.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = tag;
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq].rsplit(|c: char| c.is_whitespace()).next().unwrap_or("").to_string();
        let after = &rest[eq + 2..];
        let end = after.find('"').expect("an attribute value ends");
        out.push((key, after[..end].to_string()));
        rest = &after[end + 1..];
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn number(attrs: &[(String, String)], key: &str) -> f32 {
    attr(attrs, key).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

/// A shape element as path data, if it is one.
fn path_of(tag: &str, a: &[(String, String)]) -> Option<String> {
    Some(match tag {
        "path" => attr(a, "d")?.to_string(),
        "circle" | "ellipse" => {
            let (cx, cy) = (number(a, "cx"), number(a, "cy"));
            let (rx, ry) = if tag == "circle" { (number(a, "r"), number(a, "r")) } else { (number(a, "rx"), number(a, "ry")) };
            format!("M{},{}a{rx},{ry} 0 1 0 {},0a{rx},{ry} 0 1 0 {},0Z", cx - rx, cy, 2.0 * rx, -2.0 * rx)
        }
        "line" => format!("M{},{}L{},{}", number(a, "x1"), number(a, "y1"), number(a, "x2"), number(a, "y2")),
        "polyline" | "polygon" => {
            let nums: Vec<f32> = attr(a, "points")?.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).map(|t| t.parse().unwrap()).collect();
            let mut d = String::new();
            for (i, xy) in nums.chunks(2).enumerate() {
                d += &format!("{}{},{}", if i == 0 { "M" } else { "L" }, xy[0], xy[1]);
            }
            if tag == "polygon" {
                d.push('Z');
            }
            d
        }
        "rect" => {
            let (x, y, w, h) = (number(a, "x"), number(a, "y"), number(a, "width"), number(a, "height"));
            let rx = number(a, "rx").min(w / 2.0);
            let ry = if attr(a, "ry").is_some() { number(a, "ry") } else { rx }.min(h / 2.0);
            format!(
                "M{},{y}H{}a{rx},{ry} 0 0 1 {rx},{ry}V{}a{rx},{ry} 0 0 1 {},{ry}H{}a{rx},{ry} 0 0 1 {},{}V{}a{rx},{ry} 0 0 1 {rx},{}Z",
                x + rx,
                x + w - rx,
                y + h - ry,
                -rx,
                x + rx,
                -rx,
                -ry,
                y + ry,
                -ry
            )
        }
        _ => return None,
    })
}

fn circle(c: Point, r: f32) -> Vec<Point> {
    (0..round())
        .map(|i| {
            let t = i as f32 / round() as f32 * std::f32::consts::TAU;
            (c.0 + r * libm::cosf(t), c.1 + r * libm::sinf(t))
        })
        .collect()
}

/// The shapes that, filled together (non-zero), cover `width` around the line
/// through `pts`, with round caps and joins: a quad per segment, a disc at
/// each end and at each sharp turn, a wedge at each gentle one.
fn stroke(pts: &[Point], closed: bool, width: f32) -> Vec<Vec<Point>> {
    let half = width / 2.0;
    let mut p: Vec<Point> = Vec::with_capacity(pts.len());
    for &q in pts {
        if p.last().is_none_or(|l| libm::hypotf(l.0 - q.0, l.1 - q.1) > 1e-4) {
            p.push(q);
        }
    }
    if closed && p.len() > 1 && libm::hypotf(p[0].0 - p[p.len() - 1].0, p[0].1 - p[p.len() - 1].1) <= 1e-4 {
        p.pop();
    }
    if p.len() == 1 {
        return vec![circle(p[0], half)];
    }
    let n = p.len();
    let segs = if closed { n } else { n - 1 };
    let dir = |i: usize| {
        let (a, b) = (p[i], p[(i + 1) % n]);
        let len = libm::hypotf(b.0 - a.0, b.1 - a.1);
        ((b.0 - a.0) / len, (b.1 - a.1) / len)
    };
    let mut out = Vec::new();
    for i in 0..segs {
        let (a, b, d) = (p[i], p[(i + 1) % n], dir(i));
        let nx = (-d.1 * half, d.0 * half);
        out.push(vec![(a.0 + nx.0, a.1 + nx.1), (b.0 + nx.0, b.1 + nx.1), (b.0 - nx.0, b.1 - nx.1), (a.0 - nx.0, a.1 - nx.1)]);
    }
    let joins: Vec<usize> = if closed { (0..n).collect() } else { (1..n - 1).collect() };
    for v in joins {
        let (d1, d2) = (dir((v + segs - 1) % segs), dir(v % segs));
        let turn = libm::atan2f(d1.0 * d2.1 - d1.1 * d2.0, d1.0 * d2.0 + d1.1 * d2.1).abs();
        if turn > 0.6 {
            out.push(circle(p[v], half));
        } else if turn > 0.01 {
            let c = p[v];
            let (n1, n2) = ((-d1.1 * half, d1.0 * half), (-d2.1 * half, d2.0 * half));
            out.push(vec![c, (c.0 + n1.0, c.1 + n1.1), (c.0 + n2.0, c.1 + n2.1)]);
            out.push(vec![c, (c.0 - n1.0, c.1 - n1.1), (c.0 - n2.0, c.1 - n2.1)]);
        }
    }
    if !closed {
        out.push(circle(p[0], half));
        out.push(circle(p[n - 1], half));
    }
    out
}

fn area(p: &[Point]) -> f32 {
    (0..p.len()).map(|i| p[i].0 * p[(i + 1) % p.len()].1 - p[(i + 1) % p.len()].0 * p[i].1).sum::<f32>() / 2.0
}

fn inside(point: Point, poly: &[Point]) -> bool {
    let mut yes = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a.1 > point.1) != (b.1 > point.1) && point.0 < (b.0 - a.0) * (point.1 - a.1) / (b.1 - a.1) + a.0 {
            yes = !yes;
        }
    }
    yes
}

/// An icon's shapes in font units, y up, wound so that filling them together
/// (non-zero, as a font is) gives the icon: strokes and plain fills wind one
/// way; an even-odd fill's holes wind the other.
fn shapes(svg: &str) -> Vec<Vec<Point>> {
    let scale = EM as f32 / 24.0;
    let flip = |p: Point| (p.0 * scale, ASCENT as f32 - p.1 * scale);
    let mut tags = svg.split('<').skip(1).map(|t| t.split('>').next().unwrap_or("").trim_end_matches('/').trim());
    let root = tags.next().expect("an svg tag").to_string();
    assert!(root.starts_with("svg") && root.contains("viewBox=\"0 0 24 24\""), "icons are 24 units square");
    let root = attributes(&root);
    let stroked = attr(&root, "stroke").is_some_and(|s| s != "none");
    let width = attr(&root, "stroke-width").and_then(|w| w.parse().ok()).unwrap_or(2.0);
    let mut out: Vec<Vec<Point>> = Vec::new();
    let mut in_defs = false;
    for tag in tags {
        let name = tag.split_whitespace().next().unwrap_or("");
        match name {
            "defs" => in_defs = true,
            "/defs" => in_defs = false,
            _ if in_defs => {}
            _ => {
                let a = attributes(tag);
                let Some(d) = path_of(name, &a) else { continue };
                let pieces = subpaths(&d, detail());
                if stroked {
                    for (pts, closed) in pieces {
                        out.extend(stroke(&pts, closed, width).into_iter().map(|poly| poly.into_iter().map(flip).collect::<Vec<_>>()));
                    }
                    continue;
                }
                let polys: Vec<Vec<Point>> = pieces.into_iter().map(|(pts, _)| pts).filter(|p| p.len() > 2).map(|p| p.into_iter().map(flip).collect()).collect();
                if attr(&a, "fill-rule") == Some("evenodd") {
                    // A hole winds the other way: count the shapes around each.
                    let depths: Vec<usize> = (0..polys.len()).map(|i| (0..polys.len()).filter(|&j| j != i && inside(polys[i][0], &polys[j])).count()).collect();
                    out.extend(polys.into_iter().zip(depths).map(|(mut p, depth)| {
                        if (area(&p) < 0.0) == (depth % 2 == 1) {
                            p.reverse();
                        }
                        p
                    }));
                } else {
                    out.extend(polys);
                }
            }
        }
    }
    if stroked {
        for p in &mut out {
            if area(p) > 0.0 {
                p.reverse();
            }
        }
    }
    out
}

/// An icon as font contours: whole units.
fn contours(svg: &str) -> Vec<Vec<(i16, i16)>> {
    shapes(svg)
        .into_iter()
        .filter_map(|poly| {
            let mut pts: Vec<(i16, i16)> = Vec::with_capacity(poly.len());
            for (x, y) in poly {
                let p = (x.round() as i16, y.round() as i16);
                if pts.last() != Some(&p) {
                    pts.push(p);
                }
            }
            while pts.len() > 1 && pts.first() == pts.last() {
                pts.pop();
            }
            (pts.len() >= 3).then_some(pts)
        })
        .collect()
}

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

fn bounds(g: &Glyph) -> (i16, i16, i16, i16) {
    let pts = g.contours.iter().flatten();
    let (mut x0, mut y0, mut x1, mut y1) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
    for &(x, y) in pts {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    if x0 > x1 { (0, 0, 0, 0) } else { (x0, y0, x1, y1) }
}

/// A simple glyph's record: every point on the curve, coordinates as 16-bit deltas.
fn glyph_record(g: &Glyph) -> Vec<u8> {
    if g.contours.is_empty() {
        return Vec::new();
    }
    let mut v = Vec::new();
    be16(&mut v, g.contours.len() as u16);
    let (x0, y0, x1, y1) = bounds(g);
    for b in [x0, y0, x1, y1] {
        be16(&mut v, b as u16);
    }
    let mut end = 0u16;
    for c in &g.contours {
        end += c.len() as u16;
        be16(&mut v, end - 1);
    }
    be16(&mut v, 0); // no instructions
    let points: Vec<(i16, i16)> = g.contours.iter().flatten().copied().collect();
    v.extend(std::iter::repeat_n(0x01u8, points.len()));
    let mut prev = 0i16;
    for &(x, _) in &points {
        be16(&mut v, x.wrapping_sub(prev) as u16);
        prev = x;
    }
    prev = 0;
    for &(_, y) in &points {
        be16(&mut v, y.wrapping_sub(prev) as u16);
        prev = y;
    }
    pad4(&mut v);
    v
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
}

/// The font file for `glyphs` (sorted by codepoint), family "Kolada Icons".
fn build(glyphs: &[Glyph]) -> Vec<u8> {
    let n = glyphs.len() as u16 + 1; // glyph 0 is .notdef, empty
    let mut glyf = Vec::new();
    // Where each glyph starts, and where the last ends: .notdef is empty, so
    // the first two are both 0.
    let mut loca: Vec<u8> = Vec::new();
    be32(&mut loca, 0);
    be32(&mut loca, 0);
    let (mut max_points, mut max_contours) = (0u16, 0u16);
    let mut hmtx = Vec::new();
    be16(&mut hmtx, EM as u16);
    be16(&mut hmtx, 0);
    let (mut gx0, mut gy0, mut gx1, mut gy1) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
    let mut min_lsb = i16::MAX;
    for g in glyphs {
        glyf.extend(glyph_record(g));
        be32(&mut loca, glyf.len() as u32);
        let (x0, y0, x1, y1) = bounds(g);
        (gx0, gy0, gx1, gy1) = (gx0.min(x0), gy0.min(y0), gx1.max(x1), gy1.max(y1));
        min_lsb = min_lsb.min(x0);
        max_points = max_points.max(g.contours.iter().map(|c| c.len() as u16).sum());
        max_contours = max_contours.max(g.contours.len() as u16);
        be16(&mut hmtx, EM as u16);
        be16(&mut hmtx, x0 as u16);
    }
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000); // font revision 1.0
    be32(&mut head, 0); // checkSumAdjustment, set once the file is whole
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0x0003); // baseline at y 0, left side bearing at x 0
    be16(&mut head, EM as u16);
    head.extend([0u8; 16]); // created, modified
    for b in [gx0, gy0, gx1, gy1] {
        be16(&mut head, b as u16);
    }
    be16(&mut head, 0); // macStyle
    be16(&mut head, 8); // smallest readable size
    be16(&mut head, 2);
    be16(&mut head, 1); // long loca offsets
    be16(&mut head, 0);

    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    be16(&mut hhea, ASCENT as u16);
    be16(&mut hhea, DESCENT as u16);
    be16(&mut hhea, 0);
    be16(&mut hhea, EM as u16);
    be16(&mut hhea, min_lsb as u16);
    be16(&mut hhea, 0);
    be16(&mut hhea, gx1 as u16);
    be16(&mut hhea, 1);
    be16(&mut hhea, 0);
    be16(&mut hhea, 0);
    hhea.extend([0u8; 8]);
    be16(&mut hhea, 0);
    be16(&mut hhea, n);

    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    be16(&mut maxp, n);
    be16(&mut maxp, max_points);
    be16(&mut maxp, max_contours);
    for v in [0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut maxp, v);
    }

    // cmap: one Windows Unicode (BMP) table, a segment per codepoint.
    let segs = glyphs.len() + 1;
    let mut sub = Vec::new();
    be16(&mut sub, 4);
    be16(&mut sub, (16 + segs * 8) as u16);
    be16(&mut sub, 0);
    be16(&mut sub, (segs * 2) as u16);
    let pow = 1usize << (usize::BITS - 1 - segs.leading_zeros());
    be16(&mut sub, (pow * 2) as u16);
    be16(&mut sub, pow.trailing_zeros() as u16);
    be16(&mut sub, (segs * 2 - pow * 2) as u16);
    for g in glyphs {
        be16(&mut sub, g.code);
    }
    be16(&mut sub, 0xFFFF);
    be16(&mut sub, 0);
    for g in glyphs {
        be16(&mut sub, g.code);
    }
    be16(&mut sub, 0xFFFF);
    for (i, g) in glyphs.iter().enumerate() {
        be16(&mut sub, (i as u16 + 1).wrapping_sub(g.code));
    }
    be16(&mut sub, 1);
    for _ in 0..segs {
        be16(&mut sub, 0);
    }
    let mut cmap = Vec::new();
    be16(&mut cmap, 0);
    be16(&mut cmap, 1);
    be16(&mut cmap, 3);
    be16(&mut cmap, 1);
    be32(&mut cmap, 12);
    cmap.extend(sub);

    let names = [(1u16, "Kolada Icons"), (2, "Regular"), (3, "Kolada Icons 1.0"), (4, "Kolada Icons"), (5, "Version 1.0"), (6, "KoladaIcons-Regular")];
    let mut name = Vec::new();
    be16(&mut name, 0);
    be16(&mut name, names.len() as u16);
    be16(&mut name, (6 + names.len() * 12) as u16);
    let mut strings = Vec::new();
    for (id, s) in names {
        let u = utf16(s);
        be16(&mut name, 3);
        be16(&mut name, 1);
        be16(&mut name, 0x409);
        be16(&mut name, id);
        be16(&mut name, u.len() as u16);
        be16(&mut name, strings.len() as u16);
        strings.extend(u);
    }
    name.extend(strings);

    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    be32(&mut post, 0);
    be16(&mut post, (-100i16) as u16);
    be16(&mut post, 50);
    post.extend([0u8; 20]);

    let mut os2 = Vec::new();
    be16(&mut os2, 4);
    be16(&mut os2, EM as u16);
    be16(&mut os2, 400);
    be16(&mut os2, 5);
    be16(&mut os2, 0);
    for v in [650, 600, 0, 75, 650, 600, 0, 350, 50, 300, 0] {
        be16(&mut os2, v);
    }
    os2.extend([0u8; 10]);
    for r in [0u32, 1 << 28, 0, 0] {
        be32(&mut os2, r); // the private use area
    }
    os2.extend(*b"NTSY");
    be16(&mut os2, 0x0040);
    be16(&mut os2, glyphs.first().map_or(0, |g| g.code));
    be16(&mut os2, glyphs.last().map_or(0, |g| g.code));
    for v in [ASCENT as u16, DESCENT as u16, 0, ASCENT as u16, (-DESCENT) as u16] {
        be16(&mut os2, v);
    }
    be32(&mut os2, 1);
    be32(&mut os2, 0);
    for v in [0, 0, 0, 32, 0] {
        be16(&mut os2, v);
    }
    assert_eq!(os2.len(), 96);

    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ];
    tables.sort_by_key(|t| t.0);
    let count = tables.len() as u16;
    let pow = 1u16 << (15 - count.leading_zeros());
    let mut file = Vec::new();
    be32(&mut file, 0x0001_0000);
    be16(&mut file, count);
    be16(&mut file, pow * 16);
    be16(&mut file, pow.trailing_zeros() as u16);
    be16(&mut file, count * 16 - pow * 16);
    let mut offset = 12 + tables.len() * 16;
    let mut head_at = 0;
    for (tag, data) in &tables {
        file.extend_from_slice(tag);
        be32(&mut file, checksum(data));
        be32(&mut file, offset as u32);
        be32(&mut file, data.len() as u32);
        if tag == b"head" {
            head_at = offset;
        }
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        file.extend_from_slice(data);
        pad4(&mut file);
    }
    let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&file));
    file[head_at + 8..head_at + 12].copy_from_slice(&adjust.to_be_bytes());
    file
}

/// The font for `map` (see `parse_map`), icons read from `icons` (see `icon_file`).
fn font_from(map: &str, icons: &Path) -> Vec<u8> {
    let mut glyphs: Vec<Glyph> = parse_map(map)
        .into_iter()
        .map(|(code, name)| {
            let file = icon_file(icons, &name);
            let svg = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            Glyph { code, contours: contours(&svg) }
        })
        .collect();
    glyphs.sort_by_key(|g| g.code);
    glyphs.dedup_by_key(|g| g.code);
    build(&glyphs)
}

/// An icon's filled outline as SVG path data in its own 24-unit square, y
/// down: for a shell that draws an icon as a vector mark (see
/// `kolada_ui::vector::Logo`) rather than from the font.
fn outline(svg: &str) -> String {
    COARSE.set(true);
    let scale = EM as f32 / 24.0;
    let mut d = String::new();
    for poly in shapes(svg) {
        for (i, (x, y)) in poly.iter().enumerate() {
            let (x, y) = (x / scale, (ASCENT as f32 - y) / scale);
            d.push_str(&format!("{}{:.2} {:.2}", if i == 0 { "M" } else { "L" }, x, y));
        }
        d.push('Z');
    }
    d
}

fn main() {
    let root = root();
    // `cargo run -p iconfont -- --outline star` prints that icon's outline.
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, icon] = &args[..]
        && flag == "--outline"
    {
        let svg = std::fs::read_to_string(icon_file(&root.join("assets/icons"), icon)).expect("the icon's SVG");
        println!("{}", outline(&svg));
        return;
    }
    let map = std::fs::read_to_string(root.join("assets/icons/map.tsv")).expect("assets/icons/map.tsv");
    let font = font_from(&map, &root.join("assets/icons"));
    let out = root.join("assets/fonts/KoladaIcons.ttf");
    std::fs::create_dir_all(out.parent().expect("a folder")).expect("assets/fonts");
    std::fs::write(&out, &font).expect("the font is written");
    println!("wrote {} ({} bytes)", out.display(), font.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_map_and_skips_comments() {
        assert_eq!(parse_map("# c\nE713\tgear\n\nE721\tmagnifying-glass\n"), vec![(0xE713, "gear".into()), (0xE721, "magnifying-glass".into())]);
        assert_eq!(icon_file(Path::new("i"), "settings"), Path::new("i").join("lucide").join("settings.svg"));
        assert_eq!(icon_file(Path::new("i"), "qlementine-icons:new-24"), Path::new("i").join("qlementine-icons").join("new-24.svg"));
    }

    #[test]
    fn a_square_becomes_a_contour_in_the_em_with_y_up() {
        let c = contours(r#"<svg viewBox="0 0 24 24"><path d="M6,6H18V18H6Z"/></svg>"#);
        assert_eq!(c.len(), 1);
        let xs: Vec<i16> = c[0].iter().map(|p| p.0).collect();
        let ys: Vec<i16> = c[0].iter().map(|p| p.1).collect();
        assert_eq!((xs.iter().min().copied(), xs.iter().max().copied()), (Some(250), Some(750)));
        assert_eq!((ys.iter().min().copied(), ys.iter().max().copied()), (Some(50), Some(550)), "y is flipped: the top of the icon is the highest");
    }

    #[test]
    fn a_stroked_line_becomes_a_filled_bar_with_round_ends() {
        let svg = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 12H20" /></svg>"#;
        let c = contours(svg);
        let pts: Vec<(i16, i16)> = c.iter().flatten().copied().collect();
        let (x0, x1) = (pts.iter().map(|p| p.0).min().unwrap(), pts.iter().map(|p| p.0).max().unwrap());
        let (y0, y1) = (pts.iter().map(|p| p.1).min().unwrap(), pts.iter().map(|p| p.1).max().unwrap());
        assert_eq!((x0, x1), (125, 875), "the caps reach a stroke-half past each end");
        assert_eq!((y0, y1), (258, 342), "two units thick, centred on the line");
        assert!(c.iter().all(|k| area(&k.iter().map(|&(x, y)| (x as f32, y as f32)).collect::<Vec<_>>()) < 0.0), "all wind the same way");
    }

    #[test]
    fn an_even_odd_outline_keeps_its_hole() {
        let svg = r#"<svg viewBox="0 0 24 24"><path fill-rule="evenodd" d="M2,2H22V22H2ZM6,6H18V18H6Z"/></svg>"#;
        let c = shapes(svg);
        assert_eq!(c.len(), 2);
        assert!(area(&c[0]) * area(&c[1]) < 0.0, "the hole winds the other way");
    }

    #[test]
    fn every_icon_in_the_map_has_a_shape() {
        let root = root();
        let map = std::fs::read_to_string(root.join("assets/icons/map.tsv")).unwrap();
        for (code, name) in parse_map(&map) {
            let svg = std::fs::read_to_string(icon_file(&root.join("assets/icons"), &name)).unwrap();
            let c = contours(&svg);
            let points: usize = c.iter().map(Vec::len).sum();
            assert!(!c.is_empty() && points >= 8, "{code:04X} {name}: {} contours, {points} points ({})", c.len(), &svg[..svg.len().min(300)]);
        }
    }

    #[test]
    fn the_committed_font_is_what_the_icons_and_the_map_make() {
        let root = root();
        let map = std::fs::read_to_string(root.join("assets/icons/map.tsv")).unwrap();
        let made = font_from(&map, &root.join("assets/icons"));
        let committed = std::fs::read(root.join("assets/fonts/KoladaIcons.ttf")).expect("run `cargo run -p iconfont` and commit the font");
        assert!(made == committed, "assets/fonts/KoladaIcons.ttf is stale: run `cargo run -p iconfont`");
    }

    #[test]
    fn the_file_has_the_tables_a_loader_needs_and_a_whole_checksum() {
        let root = root();
        let map = std::fs::read_to_string(root.join("assets/icons/map.tsv")).unwrap();
        let font = font_from(&map, &root.join("assets/icons"));
        let tags: Vec<&[u8]> = (0..u16::from_be_bytes([font[4], font[5]]) as usize).map(|i| &font[12 + i * 16..16 + i * 16]).collect();
        for t in [&b"cmap"[..], b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp", b"name", b"post", b"OS/2"] {
            assert!(tags.contains(&t), "{}", String::from_utf8_lossy(t));
        }
        assert_eq!(checksum(&font), 0xB1B0_AFBA, "the whole file sums to the magic number");
        // One offset per glyph, and one more for the end of the last.
        let table = |tag: &[u8]| {
            let i = (0..u16::from_be_bytes([font[4], font[5]]) as usize).find(|&i| &font[12 + i * 16..16 + i * 16] == tag).unwrap();
            let (at, len) = (u32::from_be_bytes(font[20 + i * 16..24 + i * 16].try_into().unwrap()) as usize, u32::from_be_bytes(font[24 + i * 16..28 + i * 16].try_into().unwrap()) as usize);
            &font[at..at + len]
        };
        let glyphs = u16::from_be_bytes([table(b"maxp")[4], table(b"maxp")[5]]) as usize;
        assert_eq!(table(b"loca").len(), (glyphs + 1) * 4, "loca has glyphs + 1 offsets");
        assert_eq!(glyphs, parse_map(&map).len() + 1, "every icon, and .notdef");
    }
}
