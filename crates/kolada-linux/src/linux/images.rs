//! Pictures in notes, decoded by the `image` crate (PNG, JPEG, GIF, WebP):
//! Linux's half of the shared `kolada_ui::pictures` (where files are, their
//! sizes, the fade in). It runs on the pictures' worker thread.

use std::path::Path;

use kolada_ui::pictures::Decoder;
use kolada_ui::pixels::Pixels;

pub struct Decode;

/// For the model's `Images` service.
pub struct LinuxImages;

impl kolada_app::services::Images for LinuxImages {
    fn decoder(&self) -> Option<Box<dyn Decoder>> {
        Some(Box::new(Decode))
    }

    fn recode(&self) -> Option<Box<dyn kolada_core::share::Recode>> {
        Some(Box::new(Recoder))
    }
}

/// Sharing a note as a link: its pictures decoded from their bytes, made
/// smaller, and encoded again as a JPEG or a PNG, all in memory (the `image`
/// crate's half of `kolada_core::share::Recode`, as WIC is Windows' and
/// ImageIO the Mac's). On the thread that shares.
pub struct Recoder;

impl kolada_core::share::Recode for Recoder {
    fn decode(&self, bytes: &[u8], edge: u32) -> Option<kolada_core::share::Raster> {
        let picture = image::load_from_memory(bytes).ok()?;
        let (w, h) = (picture.width(), picture.height());
        if w == 0 || h == 0 {
            return None;
        }
        // Smaller, never larger: its longer side at most `edge`.
        let s = (edge as f64 / w.max(h) as f64).min(1.0);
        let (width, height) = (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1));
        let rgba = picture.to_rgba8();
        let rgba = if (width, height) == (w, h) { rgba } else { image::imageops::resize(&rgba, width, height, image::imageops::FilterType::Lanczos3) };
        let bgra = rgba.as_raw().chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        Some(kolada_core::share::Raster { width, height, bgra })
    }

    fn encode(&self, raster: &kolada_core::share::Raster, quality: u8, png: bool) -> Option<Vec<u8>> {
        let (w, h) = (raster.width, raster.height);
        if w == 0 || h == 0 || raster.bgra.len() < (w * h * 4) as usize {
            return None;
        }
        let mut out = std::io::Cursor::new(Vec::new());
        if png {
            let rgba: Vec<u8> = raster.bgra.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
            image::write_buffer_with_format(&mut out, &rgba, w, h, image::ExtendedColorType::Rgba8, image::ImageFormat::Png).ok()?;
        } else {
            let rgb: Vec<u8> = raster.bgra.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100)).encode(&rgb, w, h, image::ExtendedColorType::Rgb8).ok()?;
        }
        Some(out.into_inner()).filter(|b| !b.is_empty())
    }
}

impl Decoder for Decode {
    fn size(&self, path: &Path) -> Option<(u32, u32)> {
        let (w, h) = image::image_dimensions(path).ok()?;
        (w > 0 && h > 0).then_some((w, h))
    }

    fn decode(&self, path: &Path, width: u32) -> Option<Pixels> {
        let picture = image::open(path).ok()?;
        let (w, h) = (picture.width(), picture.height());
        if w == 0 || h == 0 {
            return None;
        }
        let width = width.clamp(1, 8192);
        let height = ((h as f64 * width as f64 / w as f64).round() as u32).clamp(1, 16384);
        let rgba = picture.to_rgba8();
        let scaled = if (width, height) == (w, h) { rgba } else { image::imageops::resize(&rgba, width, height, image::imageops::FilterType::CatmullRom) };
        Some(Pixels { width, height, data: premultiplied(scaled.as_raw()) })
    }
}

/// RGBA bytes as premultiplied 0xAARRGGBB words, as the surface blends them.
pub fn premultiplied(rgba: &[u8]) -> Vec<u32> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            let a = p[3] as u32;
            let m = |c: u8| (c as u32 * a + 127) / 255;
            (a << 24) | (m(p[0]) << 16) | (m(p[1]) << 8) | m(p[2])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_sized_and_scaled_into_premultiplied_pixels() {
        let dir = std::env::temp_dir().join(format!("kolada-linux-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("four.png");
        // Red, green / blue, white.
        let px: [u8; 12] = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        image::save_buffer(&path, &px, 2, 2, image::ExtendedColorType::Rgb8).unwrap();
        assert_eq!(kolada_ui::pictures::header_size(&path), Some((2, 2)));
        assert_eq!(Decode.size(&path), Some((2, 2)));
        let p = Decode.decode(&path, 2).expect("it decodes");
        assert_eq!((p.width, p.height), (2, 2));
        assert_eq!(p.data, vec![0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF, 0xFFFF_FFFF]);
        let big = Decode.decode(&path, 4).expect("it scales");
        assert_eq!((big.width, big.height), (4, 4));
        assert_eq!(big.data[0] & 0x00FF_FFFF, 0x00FF_0000, "red at the top left");
        assert!(Decode.size(&dir.join("missing.png")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shared_picture_is_made_smaller_and_encoded_again() {
        use kolada_core::share::Recode;
        // A 400 x 200 picture, red on the left half, blue on the right.
        let mut rgb = Vec::new();
        for _ in 0..200 {
            for x in 0..400 {
                rgb.extend_from_slice(if x < 200 { &[255, 0, 0] } else { &[0, 0, 255] });
            }
        }
        let mut png = std::io::Cursor::new(Vec::new());
        image::write_buffer_with_format(&mut png, &rgb, 400, 200, image::ExtendedColorType::Rgb8, image::ImageFormat::Png).unwrap();
        let r = Recoder.decode(png.get_ref(), 100).expect("it decodes");
        assert_eq!((r.width, r.height), (100, 50), "its longer side at the edge given");
        assert_eq!(&r.bgra[..4], &[0, 0, 255, 255], "BGRA: red is 0, 0, 255");
        let same = Recoder.decode(png.get_ref(), 4000).unwrap();
        assert_eq!((same.width, same.height), (400, 200), "never made larger");
        let jpeg = Recoder.encode(&r, 80, false).expect("a JPEG");
        assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF]);
        let back = image::load_from_memory(&jpeg).unwrap().to_rgb8();
        assert_eq!(back.dimensions(), (100, 50));
        assert!(back.get_pixel(10, 25)[0] > 200 && back.get_pixel(90, 25)[2] > 200, "red stays left and blue right");
        let png_again = Recoder.encode(&r, 80, true).expect("a PNG");
        assert_eq!(&png_again[..8], b"\x89PNG\r\n\x1a\n");
        assert!(Recoder.decode(b"not a picture", 100).is_none());
    }

    #[test]
    fn see_through_pixels_are_premultiplied() {
        assert_eq!(premultiplied(&[255, 128, 0, 128]), vec![0x8080_4000]);
        assert_eq!(premultiplied(&[10, 20, 30, 0]), vec![0]);
    }
}
