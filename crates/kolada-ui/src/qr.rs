//! Pairing's QR code: the link a phone's camera opens
//! (`https://kolada.app/pair#CODE`), drawn as fills. The code rides in the
//! link's fragment, which a browser never sends: no server and no log sees
//! it. It is made only while a code shows, never at launch.
//!
//! Always dark modules on a light ground, in both themes: a camera reads
//! that way round. Each module is `MODULE` DIPs, whole pixels at the usual
//! scales, so no row meets the next on half a pixel.

use qrcodegen::{QrCode, QrCodeEcc};

use crate::geom::{Fill, Ink, Rect};

/// A module's side, in DIPs: whole pixels at 100, 125, 150, 175, 200, 250
/// and 300%.
pub const MODULE: f32 = 4.0;
/// The quiet margin around the modules, in modules.
pub const QUIET: usize = 3;

/// The link a code's QR holds: the code only in the fragment.
pub fn pair_link(code: &str) -> String {
    format!("https://kolada.app/pair#{code}")
}

/// `text`'s modules, row by row (true: dark), and how many a side.
pub fn modules(text: &str) -> Option<(usize, Vec<bool>)> {
    let q = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let n = q.size() as usize;
    Some((n, (0..n * n).map(|i| q.get_module((i % n) as i32, (i / n) as i32)).collect()))
}

/// How wide `text`'s code is drawn, its margin included (0: it can't be).
pub fn side(text: &str) -> f32 {
    modules(text).map_or(0.0, |(n, _)| (n + 2 * QUIET) as f32 * MODULE)
}

/// Draws `text`'s code with its top left near (`x`, `y`), on whole
/// modules: its light ground, then each row's runs of dark modules. Where
/// it went.
pub fn draw(fills: &mut Vec<Fill>, x: f32, y: f32, text: &str) -> Rect {
    let Some((n, m)) = modules(text) else { return Rect::default() };
    let (x, y) = ((x / MODULE).round() * MODULE, (y / MODULE).round() * MODULE);
    let s = (n + 2 * QUIET) as f32 * MODULE;
    let r = Rect::new(x, y, x + s, y + s);
    fills.push(Fill::solid(r, MODULE, Ink::QrGround));
    let o = QUIET as f32 * MODULE;
    let at = |k: usize| k as f32 * MODULE;
    for row in 0..n {
        let mut c = 0;
        while c < n {
            if !m[row * n + c] {
                c += 1;
                continue;
            }
            let start = c;
            while c < n && m[row * n + c] {
                c += 1;
            }
            fills.push(Fill::solid(Rect::new(x + o + at(start), y + o + at(row), x + o + at(c), y + o + at(row + 1)), 0.0, Ink::QrModule));
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What is drawn reads back as the link: the fills, rendered to a
    /// grid at 1.5x, decoded by a QR reader, give the link exactly.
    #[test]
    fn the_code_drawn_reads_back_as_the_link() {
        let link = pair_link("F4FS-2RSA");
        let mut fills = Vec::new();
        let r = draw(&mut fills, 0.0, 0.0, &link);
        let scale = 1.5;
        let (w, h) = ((r.width() * scale) as usize, (r.height() * scale) as usize);
        let mut px = vec![255u8; w * h];
        for f in fills.iter().filter(|f| f.ink == Ink::QrModule) {
            let (x0, y0, x1, y1) = ((f.rect.left * scale) as usize, (f.rect.top * scale) as usize, (f.rect.right * scale) as usize, (f.rect.bottom * scale) as usize);
            for y in y0..y1 {
                for x in x0..x1 {
                    px[y * w + x] = 0;
                }
            }
        }
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| px[y * w + x]);
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1, "one code found");
        let (_, text) = grids[0].decode().expect("it decodes");
        assert_eq!(text, link);
    }

    /// The code goes only in the link's fragment, which a browser never
    /// sends to the server.
    #[test]
    fn the_code_is_only_in_the_fragment() {
        let link = pair_link("F4FS-2RSA");
        let (before, after) = link.split_once('#').unwrap();
        assert_eq!(before, "https://kolada.app/pair");
        assert_eq!(after, "F4FS-2RSA");
        assert!(!before.contains("F4FS"));
    }

    /// Every edge falls on a whole module, so on whole pixels at the usual
    /// scales; a small code (version 3) for the link, 140 DIPs with its
    /// margin.
    #[test]
    fn modules_fall_on_whole_pixels() {
        let link = pair_link("F4FS-2RSA");
        assert_eq!(modules(&link).map(|m| m.0), Some(29));
        assert_eq!(side(&link), 140.0);
        let mut fills = Vec::new();
        draw(&mut fills, 33.3, 47.9, &link);
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            for f in &fills {
                for v in [f.rect.left, f.rect.top, f.rect.right, f.rect.bottom] {
                    assert!((v * scale - (v * scale).round()).abs() < 1e-3, "{scale}: {v}");
                }
            }
        }
    }
}
