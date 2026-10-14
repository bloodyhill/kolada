//! Renders the app mark from `assets/logo.svg` (the user's bird in red, its
//! eye white): the icon at every size Windows asks for (taskbar, Alt+Tab,
//! title bar, Explorer views) in one .ico, a 256 px PNG of it, the icon's
//! SVG, and a 1024 px PNG of the mark for anything that is not an icon.
//! Also the icons of the taskbar menu's items (a note, a new note, search)
//! from the user's drawings in `assets/jump`. Everything is filled shapes,
//! never strokes.
//!
//! The mark runs nearly edge to edge; the icon sets it in from the edges,
//! as Windows' own icons are, so it stands the size of its neighbours.
//!
//! Run from the workspace root after changing the logo:
//! `cargo run --release -p icongen`. Its output is committed, so building the
//! app does not need this tool. `icongen --preview out.png a.svg b.svg`
//! renders SVGs at icon sizes on a white and a dark taskbar, to judge them.

use std::fs;
use std::process::ExitCode;

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

const SOURCE: &str = "assets/logo.svg";
const ICON: &str = "crates/kolada-win/kolada.ico";
const PREVIEW: &str = "assets/icon-256.png";
const MARK_PNG: &str = "assets/logo.png";
const ICON_SVG: &str = "assets/icon.svg";
/// The MSIX package's images (`tools/msix.ps1` packs them).
const MSIX_DIR: &str = "assets/msix";
/// The Mac's app icon, which `tools/mac-bundle.sh` puts in Kolada.app.
const MAC_ICON: &str = "crates/kolada-mac/Kolada.icns";
/// The Mac sets an app's art in from its square further (macOS's icon grid
/// keeps it within 824 of 1024 points), so it stands the size of the Dock's
/// other icons.
const MAC_FILL: f32 = 0.80;
/// The Mac's icon images: their type and size (Retina pairs share sizes).
const MAC_SIZES: [(&[u8; 4], u32); 10] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic09", 512),
    (b"ic10", 1024),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic13", 256),
    (b"ic14", 512),
];

/// The iPad's and iPhone's app icon: one 1024 px square each for the light
/// and dark Home Screen, opaque (iOS rounds the corners), in the app's
/// asset catalog, which `tools/ios-build.sh` compiles.
const IOS_ICONS: &str = "apps/ios/Kolada/Assets.xcassets/AppIcon.appiconset";
/// iOS's icon grid keeps a mark well inside the square.
const IOS_FILL: f32 = 0.66;
/// The bird on paper, and on graphite (kolada-ui theme.rs).
const IOS_GROUNDS: [(&str, [u8; 3]); 2] = [("icon.png", [0xF7, 0xF7, 0xF8]), ("icon-dark.png", [0x1A, 0x1A, 0x1A])];

/// How much of the icon's square the mark spans, its wider way.
const FILL: f32 = 0.87;
const SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];

/// The taskbar menu's icons show from 16 to 48 pixels.
const JUMP_SIZES: [u32; 7] = [16, 20, 24, 32, 40, 48, 64];

/// The taskbar menu's icons: (drawing, icon), each drawing set in its
/// square as the app's mark is.
const JUMP_ICONS: [(&str, &str); 3] = [
    // A note: the bird twice, as a file of them.
    ("assets/jump/note.svg", "crates/kolada-win/jump-note.ico"),
    // A new note: a page with the bird's eye and a plus.
    ("assets/jump/new.svg", "crates/kolada-win/jump-new.ico"),
    // Search: the bird behind a magnifying glass.
    ("assets/jump/search.svg", "crates/kolada-win/jump-search.ico"),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.split_first() {
        Some((flag, rest)) if flag == "--preview" && rest.len() >= 2 => preview(&rest[0], &rest[1..]),
        Some(_) => Err("usage: icongen [--preview out.png icon.svg...]".to_string()),
        None => run(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("icongen: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Each SVG at the sizes Windows shows small icons, magnified four times
/// pixel for pixel, then at actual size up to 128, on a white and a dark
/// taskbar: one row per SVG and background, for judging an icon before
/// making it (`icongen --preview out.png a.svg b.svg`).
fn preview(out: &str, files: &[String]) -> Result<(), String> {
    use resvg::tiny_skia::{FilterQuality, Paint, PixmapPaint, Rect};
    const SMALL: [u32; 5] = [16, 20, 24, 32, 48];
    const ACTUAL: [u32; 7] = [16, 20, 24, 32, 48, 64, 128];
    const ZOOM: u32 = 4;
    const GAP: u32 = 16;
    const BACKGROUNDS: [[u8; 3]; 2] = [[0xFF, 0xFF, 0xFF], [0x20, 0x20, 0x20]];
    let row = 48 * ZOOM + 2 * GAP;
    let width = GAP + SMALL.iter().map(|s| s * ZOOM + GAP).sum::<u32>() + GAP + ACTUAL.iter().map(|s| s + GAP).sum::<u32>();
    let mut sheet = Pixmap::new(width, row * (files.len() * BACKGROUNDS.len()) as u32).ok_or("bad sheet size")?;
    for (i, file) in files.iter().enumerate() {
        let svg = fs::read_to_string(file).map_err(|e| format!("reading {file}: {e}"))?;
        let tree = Tree::from_str(&svg, &Options::default()).map_err(|e| format!("parsing {file}: {e}"))?;
        let canvas = tree.size().width();
        let at = |size: u32| -> Result<Pixmap, String> {
            let mut pixmap = Pixmap::new(size, size).ok_or("bad icon size")?;
            resvg::render(&tree, Transform::from_scale(size as f32 / canvas, size as f32 / canvas), &mut pixmap.as_mut());
            Ok(pixmap)
        };
        for (b, [r, g, bl]) in BACKGROUNDS.iter().enumerate() {
            let top = row * (i * BACKGROUNDS.len() + b) as u32;
            let mut paint = Paint::default();
            paint.set_color_rgba8(*r, *g, *bl, 255);
            let band = Rect::from_xywh(0.0, top as f32, width as f32, row as f32).ok_or("bad band")?;
            sheet.fill_rect(band, &paint, Transform::identity(), None);
            let nearest = PixmapPaint { quality: FilterQuality::Nearest, ..PixmapPaint::default() };
            let mut x = GAP;
            for size in SMALL {
                let icon = at(size)?;
                let place = Transform::from_scale(ZOOM as f32, ZOOM as f32).post_translate(x as f32, (top + GAP) as f32);
                sheet.draw_pixmap(0, 0, icon.as_ref(), &nearest, place, None);
                x += size * ZOOM + GAP;
            }
            x += GAP;
            for size in ACTUAL {
                let icon = at(size)?;
                sheet.draw_pixmap(x as i32, (top + GAP) as i32, icon.as_ref(), &nearest, Transform::identity(), None);
                x += size + GAP;
            }
        }
    }
    sheet.save_png(out).map_err(|e| format!("writing {out}: {e}"))?;
    println!("wrote {out}");
    Ok(())
}

/// The icon's SVG: the logo's drawing set in from the square's edges, the
/// mark spanning `FILL` of it, centred.
fn icon_svg(logo: &str) -> Result<String, String> {
    icon_svg_filling(logo, FILL)
}

/// `icon_svg` with the mark spanning `fill` of the square.
fn icon_svg_filling(logo: &str, fill: f32) -> Result<String, String> {
    let tree = Tree::from_str(logo, &Options::default()).map_err(|e| format!("parsing the logo: {e}"))?;
    let open = logo.find("<svg").and_then(|s| logo[s..].find('>').map(|e| s + e + 1)).ok_or("no <svg> in the logo")?;
    let close = logo.rfind("</svg>").ok_or("unclosed <svg> in the logo")?;
    // The bounds are measured on the canvas, which starts at the viewBox's
    // corner; the drawing's own coordinates are the viewBox's.
    let view: Vec<f32> = logo[..open]
        .split("viewBox=\"")
        .nth(1)
        .and_then(|v| v.split('"').next())
        .map(|v| v.split_whitespace().filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_default();
    let [vx, vy, vw, vh] = view[..] else { return Err("the logo has no viewBox".to_string()) };
    if (tree.size().width() - vw).abs() > 0.01 || (tree.size().height() - vh).abs() > 0.01 {
        return Err("the logo's width and height must be its viewBox's".to_string());
    }
    let b = tree.root().abs_bounding_box();
    let k = 1024.0 * fill / b.width().max(b.height());
    let (cx, cy) = (b.x() + b.width() / 2.0 + vx, b.y() + b.height() / 2.0 + vy);
    Ok(format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><g transform="translate(512 512) scale({k:.5}) translate({:.3} {:.3})">{}</g></svg>"#,
        -cx,
        -cy,
        logo[open..close].trim()
    ))
}

/// An icon image as a 32-bit DIB with straight alpha and a 1-bit mask, the
/// format every Windows component reads (some, like .NET's icon reader,
/// reject PNG entries at small sizes).
fn dib(pixmap: &Pixmap) -> Vec<u8> {
    let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
    let mask_row = w.div_ceil(32) * 4;
    let mut out = Vec::with_capacity(40 + w * h * 4 + mask_row * h);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(2 * h as i32).to_le_bytes()); // color rows + mask rows
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&((w * h * 4 + mask_row * h) as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    let px = pixmap.data(); // premultiplied RGBA, top-down
    for y in (0..h).rev() {
        for x in 0..w {
            let p = &px[(y * w + x) * 4..][..4];
            let a = p[3] as u32;
            let straight = |c: u8| (c as u32 * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8);
            out.extend_from_slice(&[straight(p[2]), straight(p[1]), straight(p[0]), p[3]]);
        }
    }
    // Mask: set where fully transparent, for components that ignore alpha.
    for y in (0..h).rev() {
        let mut row = vec![0u8; mask_row];
        for x in 0..w {
            if px[(y * w + x) * 4 + 3] == 0 {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        out.extend_from_slice(&row);
    }
    out
}

/// An .ico from (size, image data) pairs.
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, png) in images {
        let dim = if *size >= 256 { 0 } else { *size as u8 }; // 0 means 256
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += png.len();
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}

/// Renders `svg` (on a `canvas` wide square) at each of `sizes` into an
/// .ico: bitmaps for small sizes, which every Windows component reads, and
/// PNG for large ones, which keeps the file small.
fn render(svg: &str, canvas: f32, sizes: &[u32]) -> Result<Vec<(u32, Vec<u8>)>, String> {
    let tree = Tree::from_str(svg, &Options::default()).map_err(|e| format!("parsing an icon: {e}"))?;
    let mut images = Vec::new();
    for &size in sizes {
        let mut pixmap = Pixmap::new(size, size).ok_or("bad icon size")?;
        let scale = size as f32 / canvas;
        resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        let data = if size >= 64 { pixmap.encode_png().map_err(|e| format!("encoding {size}px: {e}"))? } else { dib(&pixmap) };
        images.push((size, data));
    }
    Ok(images)
}

/// An .icns from (type, PNG) pairs: a header, then each image with its
/// type and length.
fn icns(images: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let len = 8 + images.iter().map(|(_, png)| 8 + png.len()).sum::<usize>();
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(len as u32).to_be_bytes());
    for (kind, png) in images {
        out.extend_from_slice(*kind);
        out.extend_from_slice(&((8 + png.len()) as u32).to_be_bytes());
        out.extend_from_slice(png);
    }
    out
}

/// The Mac's app icon from the logo, every image a PNG.
fn mac_icon(logo: &str) -> Result<(), String> {
    let svg = icon_svg_filling(logo, MAC_FILL)?;
    let tree = Tree::from_str(&svg, &Options::default()).map_err(|e| format!("parsing the Mac icon: {e}"))?;
    let mut images = Vec::new();
    for (kind, size) in MAC_SIZES {
        let mut pixmap = Pixmap::new(size, size).ok_or("bad icon size")?;
        let scale = size as f32 / 1024.0;
        resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        images.push((kind, pixmap.encode_png().map_err(|e| format!("encoding {size}px: {e}"))?));
    }
    fs::write(MAC_ICON, icns(&images)).map_err(|e| format!("writing {MAC_ICON}: {e}"))?;
    println!("wrote {MAC_ICON}");
    Ok(())
}

/// The iOS app icons from the logo.
fn ios_icon(logo: &str) -> Result<(), String> {
    let svg = icon_svg_filling(logo, IOS_FILL)?;
    let tree = Tree::from_str(&svg, &Options::default()).map_err(|e| format!("parsing the iOS icon: {e}"))?;
    for (file, [r, g, b]) in IOS_GROUNDS {
        let mut pixmap = Pixmap::new(1024, 1024).ok_or("bad icon size")?;
        pixmap.fill(resvg::tiny_skia::Color::from_rgba8(r, g, b, 255));
        resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
        let path = format!("{IOS_ICONS}/{file}");
        fs::write(&path, pixmap.encode_png().map_err(|e| format!("encoding {file}: {e}"))?).map_err(|e| format!("writing {path}: {e}"))?;
        println!("wrote {path}");
    }
    Ok(())
}

/// The MSIX package's images, each the icon set in a canvas: the name the
/// manifest uses with its scale or target-size qualifier, the canvas, and
/// how big the icon's square stands in it (the icon itself already leaves
/// a margin, see `FILL`). The 44 px logo is the app's icon in Start, the
/// taskbar and Alt+Tab; its target sizes come "unplated" too, so the
/// taskbar draws them as they are, with no tile behind.
fn msix_images() -> Vec<(String, u32, u32, u32)> {
    let mut out = Vec::new();
    for (scale, k) in [(100, 1.0), (125, 1.25), (150, 1.5), (200, 2.0), (400, 4.0)] {
        let s = (44.0 * k) as u32;
        out.push((format!("Square44x44Logo.scale-{scale}.png"), s, s, s));
    }
    for t in [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256] {
        out.push((format!("Square44x44Logo.targetsize-{t}.png"), t, t, t));
        out.push((format!("Square44x44Logo.targetsize-{t}_altform-unplated.png"), t, t, t));
        out.push((format!("Square44x44Logo.targetsize-{t}_altform-lightunplated.png"), t, t, t));
    }
    for (scale, k) in [(100, 1), (200, 2), (400, 4)] {
        out.push((format!("Square150x150Logo.scale-{scale}.png"), 150 * k, 150 * k, 90 * k));
        out.push((format!("Wide310x150Logo.scale-{scale}.png"), 310 * k, 150 * k, 90 * k));
        out.push((format!("StoreLogo.scale-{scale}.png"), 50 * k, 50 * k, 50 * k));
    }
    out
}

/// Writes `msix_images` into `MSIX_DIR`.
fn msix_assets(icon: &str) -> Result<(), String> {
    let tree = Tree::from_str(icon, &Options::default()).map_err(|e| format!("parsing the icon: {e}"))?;
    fs::create_dir_all(MSIX_DIR).map_err(|e| format!("making {MSIX_DIR}: {e}"))?;
    let images = msix_images();
    for (name, w, h, side) in &images {
        let mut pixmap = Pixmap::new(*w, *h).ok_or("bad image size")?;
        let k = *side as f32 / 1024.0;
        let at = Transform::from_scale(k, k).post_translate((w - side) as f32 / 2.0, (h - side) as f32 / 2.0);
        resvg::render(&tree, at, &mut pixmap.as_mut());
        let file = format!("{MSIX_DIR}/{name}");
        pixmap.save_png(&file).map_err(|e| format!("writing {file}: {e}"))?;
    }
    println!("wrote {} images in {MSIX_DIR}", images.len());
    Ok(())
}

/// The taskbar menu's icons.
fn jump_icons() -> Result<(), String> {
    for (source, file) in JUMP_ICONS {
        let drawing = fs::read_to_string(source).map_err(|e| format!("reading {source}: {e}"))?;
        let images = render(&icon_svg(&drawing)?, 1024.0, &JUMP_SIZES)?;
        fs::write(file, ico(&images)).map_err(|e| format!("writing {file}: {e}"))?;
        println!("wrote {file}");
    }
    Ok(())
}

fn run() -> Result<(), String> {
    jump_icons()?;
    let logo = fs::read_to_string(SOURCE).map_err(|e| format!("reading {SOURCE}: {e}"))?;
    let icon = icon_svg(&logo)?;
    mac_icon(&logo)?;
    ios_icon(&logo)?;
    msix_assets(&icon)?;
    fs::write(ICON_SVG, &icon).map_err(|e| format!("writing {ICON_SVG}: {e}"))?;
    // Small sizes as bitmaps, which every Windows component reads; large
    // ones as PNG, which keeps the icon (and the exe) small.
    let images = render(&icon, 1024.0, &SIZES)?;
    if let Some((_, png)) = images.iter().find(|(size, _)| *size == 256) {
        fs::write(PREVIEW, png).map_err(|e| format!("writing {PREVIEW}: {e}"))?;
    }
    fs::write(ICON, ico(&images)).map_err(|e| format!("writing {ICON}: {e}"))?;
    // The mark alone, 1024 pixels its wider way, for what is not an icon.
    let tree = Tree::from_str(&logo, &Options::default()).map_err(|e| format!("parsing the logo: {e}"))?;
    let size = tree.size();
    let k = 1024.0 / size.width().max(size.height());
    let mut mark = Pixmap::new((size.width() * k).round() as u32, (size.height() * k).round() as u32).ok_or("bad mark size")?;
    resvg::render(&tree, Transform::from_scale(k, k), &mut mark.as_mut());
    mark.save_png(MARK_PNG).map_err(|e| format!("writing {MARK_PNG}: {e}"))?;
    println!("wrote {ICON} ({} sizes), {PREVIEW}, {ICON_SVG} and {MARK_PNG}", images.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_the_logo_in_from_the_squares_edges() {
        let logo = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 960"><path d="M100 200H300V400H100Z" fill="#DC4848"/></svg>"##;
        let icon = icon_svg(logo).unwrap();
        // 200 units wide, to span 0.87 of 1024; its middle, (200, 300), to the
        // square's.
        assert!(icon.contains(r#"translate(512 512) scale(4.45440) translate(-200.000 -300.000)"#), "{icon}");
        assert!(icon.contains(r##"<path d="M100 200H300V400H100Z" fill="#DC4848"/></g></svg>"##), "{icon}");
        // A viewBox that does not start at 0 0: the middle is the drawing's.
        let offset = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="100 100 200 200"><path d="M150 150H250V250H150Z"/></svg>"##;
        let icon = icon_svg(offset).unwrap();
        assert!(icon.contains("scale(8.90880) translate(-200.000 -200.000)"), "{icon}");
    }

    #[test]
    fn dib_rows_are_bottom_up_with_straight_alpha_and_a_mask() {
        let mut pixmap = Pixmap::new(2, 2).unwrap();
        // Top-left: half-transparent premultiplied red; the rest transparent.
        pixmap.data_mut()[..4].copy_from_slice(&[128, 0, 0, 128]);
        let d = dib(&pixmap);
        assert_eq!(i32::from_le_bytes(d[8..12].try_into().unwrap()), 4); // 2 rows x 2
        let colors = &d[40..40 + 16];
        // Bottom-up: the top-left pixel is the first of the second row.
        assert_eq!(&colors[8..12], &[0, 0, 255, 128]);
        let mask = &d[56..];
        assert_eq!(mask.len(), 8); // two 4-byte rows
        assert_eq!(mask[4] & 0xC0, 0x40, "only the opaque pixel is unmasked");
    }

    #[test]
    fn writes_an_icns_its_length_and_each_image_typed() {
        let data = icns(&[(b"ic07", vec![1, 2, 3]), (b"ic10", vec![4])]);
        assert_eq!(&data[..4], b"icns");
        assert_eq!(u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize, data.len());
        assert_eq!(&data[8..12], b"ic07");
        assert_eq!(u32::from_be_bytes(data[12..16].try_into().unwrap()), 11);
        assert_eq!(&data[16..19], &[1, 2, 3]);
        assert_eq!(&data[19..23], b"ic10");
    }

    #[test]
    fn the_msix_images_are_every_one_the_manifest_names() {
        let images = msix_images();
        let names: Vec<&str> = images.iter().map(|(n, ..)| n.as_str()).collect();
        // Store and Start need these at 100% at least, and the taskbar the
        // unplated small sizes.
        for must in ["Square44x44Logo.scale-100.png", "Square150x150Logo.scale-100.png", "Wide310x150Logo.scale-100.png", "StoreLogo.scale-100.png", "Square44x44Logo.targetsize-24_altform-unplated.png"] {
            assert!(names.contains(&must), "{must}");
        }
        // The icon's square always fits its canvas.
        assert!(images.iter().all(|(_, w, h, side)| side <= w && side <= h));
        assert!(images.iter().any(|(n, w, h, _)| n == "Wide310x150Logo.scale-200.png" && (*w, *h) == (620, 300)));
    }

    #[test]
    fn writes_a_valid_icon_directory() {
        let data = ico(&[(16, vec![1, 2, 3]), (256, vec![4, 5])]);
        assert_eq!(&data[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(data[6], 16); // first entry width
        assert_eq!(data[22], 0); // 256 is stored as 0
        let first_offset = u32::from_le_bytes(data[18..22].try_into().unwrap()) as usize;
        assert_eq!(&data[first_offset..first_offset + 3], &[1, 2, 3]);
        assert_eq!(data.len(), 6 + 32 + 5);
    }
}
