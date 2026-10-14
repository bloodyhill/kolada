//! PDF into Markdown: each page's content run for its glyphs (`content`),
//! and the glyphs laid out as a note (`layout`). What MarkItDown gets from
//! pdfminer is the text; this also reads the structure the text is in.

mod content;
mod crypt;
mod file;
mod font;
mod layout;
mod object;
mod tables;

use crate::md;
use crate::{Error, Note};
use file::File;
use object::{Dict, Obj};

/// Pages read at most: a note from a book's worth is plenty.
const MOST_PAGES: usize = 2000;

pub fn convert(bytes: &[u8]) -> Result<Note, Error> {
    let file = File::open(bytes)?;
    let catalog = file.entry(&file.trailer, "Root").ok_or_else(|| Error("the PDF has no catalog".to_string()))?;
    let pages_root = catalog.as_dict().and_then(|c| file.entry(c, "Pages")).ok_or_else(|| Error("the PDF has no pages".to_string()))?;
    let mut leaves = Vec::new();
    let mut seen = Vec::new();
    collect(&file, &pages_root, &Inherited::default(), &mut leaves, &mut seen, 0);
    let mut reader = content::Reader::new(&file);
    let mut pages = Vec::new();
    for (page, inherited) in leaves.into_iter().take(MOST_PAGES) {
        reader.glyphs.clear();
        reader.dots.clear();
        let data = contents(&file, &page);
        let resources = inherited.resources.clone().unwrap_or_default();
        let b = inherited.box_.unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let (ctm, bottom, top) = upright(b, inherited.rotate.unwrap_or(0));
        reader.run(&data, &resources, ctm);
        let links = content::links(&file, &page).into_iter().map(|(r, uri)| (turn(&ctm, r), uri)).collect();
        pages.push(layout::Page { glyphs: std::mem::take(&mut reader.glyphs), dots: std::mem::take(&mut reader.dots), links, bottom, top });
    }
    if pages.iter().all(|p| p.glyphs.iter().all(|g| g.text.trim().is_empty())) {
        return Err(Error("the PDF has no text in it (it may be a scan)".to_string()));
    }
    let blocks = layout::blocks(pages);
    let info = file.entry(&file.trailer, "Info");
    let title = info
        .as_deref()
        .and_then(Obj::as_dict)
        .and_then(|i| file.entry(i, "Title"))
        .and_then(|t| match t.as_ref() {
            Obj::Str(s) => Some(font::text_string(s).trim().to_string()),
            _ => None,
        })
        // "Microsoft Word - plan.docx" names a file, not the document.
        .filter(|t| !t.is_empty() && !t.to_ascii_lowercase().ends_with(".docx") && !t.to_ascii_lowercase().ends_with(".doc") && !t.starts_with("Microsoft "))
        .or_else(|| {
            blocks.iter().find_map(|b| match b {
                md::Block::Heading(_, s) => Some(md::inline(s)),
                _ => None,
            })
        });
    Ok(Note { title, markdown: arabic(md::write(&blocks)), pictures: Vec::new(), beside: Vec::new() })
}

/// Some writers map Arabic's yeh and kaf to the Persian letters that share
/// their shapes (ی, ک). In Arabic (letters only Arabic has, none only
/// Persian has) they are put back.
fn arabic(text: String) -> String {
    let arabic_only = text.contains(['\u{0629}', '\u{0623}', '\u{0625}', '\u{0624}', '\u{0626}', '\u{0649}', '\u{064A}']);
    let persian_only = text.contains(['\u{067E}', '\u{0686}', '\u{0698}', '\u{06AF}']);
    if arabic_only && !persian_only && text.contains(['\u{06CC}', '\u{06A9}']) {
        text.replace('\u{06CC}', "\u{064A}").replace('\u{06A9}', "\u{0643}")
    } else {
        text
    }
}

/// What a page takes from the page tree above it: its resources, its
/// box (crop box, else media box) and how it is turned to be read.
#[derive(Clone, Default)]
struct Inherited {
    resources: Option<Dict>,
    box_: Option<[f32; 4]>,
    rotate: Option<i64>,
}

/// The matrix that sets a page with this box and /Rotate upright (as it
/// is shown), and its bottom and top once it is.
fn upright(b: [f32; 4], rotate: i64) -> ([f32; 6], f32, f32) {
    let (x0, y0, x1, y1) = (b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3]));
    match rotate.rem_euclid(360) {
        90 => ([0.0, -1.0, 1.0, 0.0, -y0, x1], 0.0, x1 - x0),
        180 => ([-1.0, 0.0, 0.0, -1.0, x1, y1], 0.0, y1 - y0),
        270 => ([0.0, 1.0, -1.0, 0.0, y1, -x0], 0.0, x1 - x0),
        _ => ([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], y0, y1),
    }
}

/// A rectangle (left, bottom, right, top) through a matrix.
fn turn(m: &[f32; 6], r: [f32; 4]) -> [f32; 4] {
    let at = |x: f32, y: f32| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
    let (a, b) = (at(r[0], r[1]), at(r[2], r[3]));
    [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
}

fn collect(file: &File, node: &Obj, from: &Inherited, out: &mut Vec<(Dict, Inherited)>, seen: &mut Vec<usize>, depth: usize) {
    let Some(d) = node.as_dict() else { return };
    // Against trees that loop.
    let id = d as *const Dict as usize;
    if depth > 32 || seen.contains(&id) || out.len() >= MOST_PAGES {
        return;
    }
    seen.push(id);
    let mut here = from.clone();
    if let Some(r) = file.entry(d, "Resources").and_then(|r| r.as_dict().cloned()) {
        here.resources = Some(r);
    }
    if let Some(r) = file.entry(d, "Rotate").and_then(|r| r.as_i64()) {
        here.rotate = Some(r);
    }
    for key in ["MediaBox", "CropBox"] {
        if let Some(b) = file.entry(d, key).and_then(|b| {
            let v: Vec<f32> = b.as_array()?.iter().filter_map(|x| file.resolve(x)?.as_f64()).map(|x| x as f32).collect();
            (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
        }) {
            here.box_ = Some(b);
        }
    }
    match file.entry(d, "Kids") {
        Some(kids) if d.get("Type").and_then(Obj::as_name) != Some("Page") => {
            for kid in kids.as_array().unwrap_or(&[]) {
                if let Some(k) = file.resolve(kid) {
                    collect(file, &k, &here, out, seen, depth + 1);
                }
            }
        }
        _ => out.push((d.clone(), here)),
    }
}

/// A page's content: its stream, or its streams one after another.
fn contents(file: &File, page: &Dict) -> Vec<u8> {
    let mut out = Vec::new();
    match file.entry(page, "Contents").as_deref() {
        Some(s @ Obj::Stream(..)) => out = file.decode(s).unwrap_or_default(),
        Some(Obj::Array(parts)) => {
            for p in parts {
                if let Some(s) = file.resolve(p)
                    && let Some(data) = file.decode(&s)
                {
                    out.extend_from_slice(&data);
                    out.push(b'\n');
                }
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests;
