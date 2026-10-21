//! Pictures in notes, decoded by ImageIO: the Mac's half of the shared
//! `kolada_ui::pictures` (where files are, their sizes, the fade in). It runs
//! on the pictures' worker thread. For a note shared as a link, ImageIO also
//! decodes pictures from memory and encodes them again as JPEG or PNG
//! (`MacRecode`, the Mac's `kolada_core::share::Recode`).

use std::ffi::c_void;
use std::path::Path;

use kolada_core::share::{Raster, Recode};
use kolada_ui::pictures::Decoder;
use kolada_ui::pixels::Pixels;

type Ref = *const c_void;

#[repr(C)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(allocator: Ref, buffer: *const u8, length: isize, is_directory: u8) -> Ref;
    fn CFRelease(object: Ref);
    fn CFDataCreate(allocator: Ref, bytes: *const u8, length: isize) -> Ref;
    fn CFDataCreateMutable(allocator: Ref, capacity: isize) -> Ref;
    fn CFDataGetLength(data: Ref) -> isize;
    fn CFDataGetBytePtr(data: Ref) -> *const u8;
    fn CFStringCreateWithBytes(allocator: Ref, bytes: *const u8, length: isize, encoding: u32, external: u8) -> Ref;
    fn CFNumberCreate(allocator: Ref, kind: isize, value: *const c_void) -> Ref;
    fn CFDictionaryCreate(allocator: Ref, keys: *const Ref, values: *const Ref, count: isize, key_callbacks: *const c_void, value_callbacks: *const c_void) -> Ref;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithURL(url: Ref, options: Ref) -> Ref;
    fn CGImageSourceCreateWithData(data: Ref, options: Ref) -> Ref;
    fn CGImageSourceCreateImageAtIndex(source: Ref, index: usize, options: Ref) -> Ref;
    fn CGImageDestinationCreateWithData(data: Ref, kind: Ref, count: usize, options: Ref) -> Ref;
    fn CGImageDestinationAddImage(destination: Ref, image: Ref, properties: Ref);
    fn CGImageDestinationFinalize(destination: Ref) -> bool;
    static kCGImageDestinationLossyCompressionQuality: Ref;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageGetWidth(image: Ref) -> usize;
    fn CGImageGetHeight(image: Ref) -> usize;
    fn CGColorSpaceCreateDeviceRGB() -> Ref;
    fn CGColorSpaceCreateWithName(name: Ref) -> Ref;
    fn CGBitmapContextCreate(data: *mut c_void, width: usize, height: usize, bits: usize, row: usize, space: Ref, info: u32) -> Ref;
    fn CGBitmapContextCreateImage(context: Ref) -> Ref;
    fn CGContextSetInterpolationQuality(context: Ref, quality: i32);
    fn CGContextDrawImage(context: Ref, rect: CGRect, image: Ref);
    static kCGColorSpaceSRGB: Ref;
}

/// A Core Foundation object, released when it goes.
struct Owned(Ref);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

/// The picture's first frame, as a Core Graphics image.
fn open(path: &Path) -> Option<Owned> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let url = Owned(unsafe { CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, 0) });
    if url.0.is_null() {
        return None;
    }
    let source = Owned(unsafe { CGImageSourceCreateWithURL(url.0, std::ptr::null()) });
    if source.0.is_null() {
        return None;
    }
    let image = Owned(unsafe { CGImageSourceCreateImageAtIndex(source.0, 0, std::ptr::null()) });
    (!image.0.is_null()).then_some(image)
}

pub struct ImageIo;

/// ImageIO, for the model's `Images` service.
pub struct MacImages;

impl kolada_app::services::Images for MacImages {
    fn decoder(&self) -> Option<Box<dyn Decoder>> {
        Some(Box::new(ImageIo))
    }

    fn recode(&self) -> Option<Box<dyn Recode>> {
        Some(Box::new(MacRecode))
    }
}

/// ImageIO from and to memory, for sharing.
pub struct MacRecode;

/// A Core Foundation string.
fn cf_string(s: &str) -> Owned {
    // kCFStringEncodingUTF8
    Owned(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, 0x0800_0100, 0) })
}

/// Premultiplied alpha first, little-endian words: B, G, R, A in memory.
const BGRA_PREMULTIPLIED: u32 = 2 | (2 << 12);

impl Recode for MacRecode {
    fn decode(&self, bytes: &[u8], edge: u32) -> Option<Raster> {
        let data = Owned(unsafe { CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize) });
        if data.0.is_null() {
            return None;
        }
        let source = Owned(unsafe { CGImageSourceCreateWithData(data.0, std::ptr::null()) });
        if source.0.is_null() {
            return None;
        }
        // The first frame as stored (the shared code turns it upright by its EXIF).
        let image = Owned(unsafe { CGImageSourceCreateImageAtIndex(source.0, 0, std::ptr::null()) });
        if image.0.is_null() {
            return None;
        }
        let (w, h) = unsafe { (CGImageGetWidth(image.0), CGImageGetHeight(image.0)) };
        if w == 0 || h == 0 {
            return None;
        }
        // Smaller, never larger.
        let s = (edge as f64 / w.max(h) as f64).min(1.0);
        let (width, height) = (((w as f64 * s).round() as usize).max(1), ((h as f64 * s).round() as usize).max(1));
        let mut bgra = vec![0u8; width * height * 4];
        let space = Owned(unsafe { CGColorSpaceCreateWithName(kCGColorSpaceSRGB) });
        let context = Owned(unsafe { CGBitmapContextCreate(bgra.as_mut_ptr().cast(), width, height, 8, width * 4, space.0, BGRA_PREMULTIPLIED) });
        if context.0.is_null() {
            return None;
        }
        unsafe {
            CGContextSetInterpolationQuality(context.0, 3);
            CGContextDrawImage(context.0, CGRect { x: 0.0, y: 0.0, w: width as f64, h: height as f64 }, image.0);
        }
        drop(context);
        // Straight alpha, as the shared code takes it.
        for p in bgra.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a != 0 && a != 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        Some(Raster { width: width as u32, height: height as u32, bgra })
    }

    fn encode(&self, raster: &Raster, quality: u8, png: bool) -> Option<Vec<u8>> {
        let (w, h) = (raster.width as usize, raster.height as usize);
        if w == 0 || h == 0 || raster.bgra.len() != w * h * 4 {
            return None;
        }
        // Premultiplied again, for a bitmap context to make the image from
        // (a JPEG has no alpha: those pixels go on black, as on Windows).
        let mut pixels = raster.bgra.clone();
        for p in pixels.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if !png {
                p[3] = 255;
                for c in &mut p[..3] {
                    *c = ((*c as u32 * a + 127) / 255) as u8;
                }
            } else if a != 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u32 * a + 127) / 255) as u8;
                }
            }
        }
        let space = Owned(unsafe { CGColorSpaceCreateWithName(kCGColorSpaceSRGB) });
        let context = Owned(unsafe { CGBitmapContextCreate(pixels.as_mut_ptr().cast(), w, h, 8, w * 4, space.0, BGRA_PREMULTIPLIED) });
        if context.0.is_null() {
            return None;
        }
        let image = Owned(unsafe { CGBitmapContextCreateImage(context.0) });
        drop(context);
        if image.0.is_null() {
            return None;
        }
        let out = Owned(unsafe { CFDataCreateMutable(std::ptr::null(), 0) });
        let kind = cf_string(if png { "public.png" } else { "public.jpeg" });
        let destination = Owned(unsafe { CGImageDestinationCreateWithData(out.0, kind.0, 1, std::ptr::null()) });
        if destination.0.is_null() {
            return None;
        }
        let q = quality.clamp(1, 100) as f64 / 100.0;
        // kCFNumberFloat64Type
        let number = Owned(unsafe { CFNumberCreate(std::ptr::null(), 6, (&q as *const f64).cast()) });
        let keys = [unsafe { kCGImageDestinationLossyCompressionQuality }];
        let values = [number.0];
        let properties = Owned(unsafe {
            CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), values.as_ptr(), if png { 0 } else { 1 }, (&raw const kCFTypeDictionaryKeyCallBacks).cast(), (&raw const kCFTypeDictionaryValueCallBacks).cast())
        });
        let done = unsafe {
            CGImageDestinationAddImage(destination.0, image.0, properties.0);
            CGImageDestinationFinalize(destination.0)
        };
        if !done {
            return None;
        }
        let len = unsafe { CFDataGetLength(out.0) } as usize;
        let bytes = unsafe { std::slice::from_raw_parts(CFDataGetBytePtr(out.0), len) }.to_vec();
        (!bytes.is_empty()).then_some(bytes)
    }
}

impl Decoder for ImageIo {
    fn size(&self, path: &Path) -> Option<(u32, u32)> {
        let image = open(path)?;
        let (w, h) = unsafe { (CGImageGetWidth(image.0), CGImageGetHeight(image.0)) };
        (w > 0 && h > 0).then_some((w as u32, h as u32))
    }

    fn decode(&self, path: &Path, width: u32) -> Option<Pixels> {
        let image = open(path)?;
        let (w, h) = unsafe { (CGImageGetWidth(image.0), CGImageGetHeight(image.0)) };
        if w == 0 || h == 0 {
            return None;
        }
        let width = width.clamp(1, 8192);
        let height = ((h as f64 * width as f64 / w as f64).round() as u32).clamp(1, 16384);
        let mut data = vec![0u32; (width * height) as usize];
        let space = Owned(unsafe { CGColorSpaceCreateDeviceRGB() });
        // Premultiplied alpha first, little-endian words: 0xAARRGGBB, rows top-down.
        let info = 2 | (2 << 12);
        let context = Owned(unsafe { CGBitmapContextCreate(data.as_mut_ptr().cast(), width as usize, height as usize, 8, width as usize * 4, space.0, info) });
        if context.0.is_null() {
            return None;
        }
        unsafe {
            CGContextSetInterpolationQuality(context.0, 3);
            CGContextDrawImage(context.0, CGRect { x: 0.0, y: 0.0, w: width as f64, h: height as f64 }, image.0);
        }
        drop(context);
        Some(Pixels { width, height, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 x 2 PNG: red, green / blue, white.
    fn png() -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]| {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            let mut crc_in = kind.to_vec();
            crc_in.extend_from_slice(body);
            out.extend_from_slice(&crc_in);
            out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        // Two rows, each: filter byte 0, then RGB RGB. Stored (uncompressed) zlib.
        let raw = [0, 255, 0, 0, 0, 255, 0, 0, 0, 0, 255, 255, 255, 255];
        let mut z = vec![0x78, 0x01, 0x01, raw.len() as u8, 0, !(raw.len() as u8), 0xFF];
        z.extend_from_slice(&raw);
        z.extend_from_slice(&adler32(&raw).to_be_bytes());
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    fn adler32(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &x in data {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }

    #[test]
    fn imageio_reads_a_pictures_size_and_scales_its_pixels() {
        let dir = std::env::temp_dir().join(format!("kolada-mac-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("four.png");
        std::fs::write(&path, png()).unwrap();
        assert_eq!(kolada_ui::pictures::header_size(&path), Some((2, 2)));
        assert_eq!(ImageIo.size(&path), Some((2, 2)));
        let p = ImageIo.decode(&path, 4).expect("it decodes");
        assert_eq!((p.width, p.height), (4, 4));
        // Top-left is red, bottom-right white (opaque, premultiplied 0xAARRGGBB).
        assert_eq!(p.data[0] & 0x00FF_FFFF, 0x00FF_0000, "red at the top left");
        assert_eq!(p.data[15], 0xFFFF_FFFF, "white at the bottom right");
        assert!(ImageIo.size(&dir.join("missing.png")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_is_decoded_from_memory_made_smaller_and_encoded_as_jpeg_or_png() {
        // 2 x 2 PNG: red, green / blue, white.
        let r = MacRecode.decode(&png(), 4096).expect("decodes");
        assert_eq!((r.width, r.height), (2, 2), "never made larger");
        assert_eq!(&r.bgra[..4], &[0, 0, 255, 255], "red first, straight BGRA");
        assert_eq!(&r.bgra[12..], &[255, 255, 255, 255]);
        let small = MacRecode.decode(&png(), 1).unwrap();
        assert_eq!((small.width, small.height), (1, 1));
        assert!(MacRecode.decode(b"not a picture", 100).is_none());
        // A larger one, half transparent, round trips through PNG exactly
        // enough, and through JPEG as a smaller opaque picture.
        let (w, h) = (64u32, 48u32);
        let bgra: Vec<u8> = (0..w * h).flat_map(|i| [(i % 256) as u8, (i / 256 % 256) as u8, 200, if i % 2 == 0 { 255 } else { 128 }]).collect();
        let raster = Raster { width: w, height: h, bgra };
        let png_bytes = MacRecode.encode(&raster, 80, true).expect("a PNG");
        assert!(png_bytes.starts_with(b"\x89PNG"));
        let back = MacRecode.decode(&png_bytes, 4096).unwrap();
        assert_eq!((back.width, back.height), (w, h));
        assert_eq!(back.bgra[3], 255);
        assert!((back.bgra[7] as i32 - 128).abs() <= 1, "alpha kept: {}", back.bgra[7]);
        let jpeg = MacRecode.encode(&raster, 70, false).expect("a JPEG");
        assert!(jpeg.starts_with(&[0xFF, 0xD8]));
        let opaque = MacRecode.decode(&jpeg, 4096).unwrap();
        assert!(opaque.bgra.chunks_exact(4).all(|p| p[3] == 255));
        assert!(MacRecode.encode(&Raster { width: 2, height: 2, bgra: vec![0; 3] }, 70, false).is_none(), "a wrong length is refused");
    }
}
