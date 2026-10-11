//! Pictures in notes, decoded by ImageIO: the Mac's half of the shared
//! `notsy_ui::pictures` (where files are, their sizes, the fade in). It runs
//! on the pictures' worker thread.

use std::ffi::c_void;
use std::path::Path;

use notsy_ui::pictures::Decoder;
use notsy_ui::pixels::Pixels;

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
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithURL(url: Ref, options: Ref) -> Ref;
    fn CGImageSourceCreateImageAtIndex(source: Ref, index: usize, options: Ref) -> Ref;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageGetWidth(image: Ref) -> usize;
    fn CGImageGetHeight(image: Ref) -> usize;
    fn CGColorSpaceCreateDeviceRGB() -> Ref;
    fn CGBitmapContextCreate(data: *mut c_void, width: usize, height: usize, bits: usize, row: usize, space: Ref, info: u32) -> Ref;
    fn CGContextSetInterpolationQuality(context: Ref, quality: i32);
    fn CGContextDrawImage(context: Ref, rect: CGRect, image: Ref);
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

impl notsy_app::services::Images for MacImages {
    fn decoder(&self) -> Option<Box<dyn Decoder>> {
        Some(Box::new(ImageIo))
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
        let dir = std::env::temp_dir().join(format!("notsy-mac-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("four.png");
        std::fs::write(&path, png()).unwrap();
        assert_eq!(notsy_ui::pictures::header_size(&path), Some((2, 2)));
        assert_eq!(ImageIo.size(&path), Some((2, 2)));
        let p = ImageIo.decode(&path, 4).expect("it decodes");
        assert_eq!((p.width, p.height), (4, 4));
        // Top-left is red, bottom-right white (opaque, premultiplied 0xAARRGGBB).
        assert_eq!(p.data[0] & 0x00FF_FFFF, 0x00FF_0000, "red at the top left");
        assert_eq!(p.data[15], 0xFFFF_FFFF, "white at the bottom right");
        assert!(ImageIo.size(&dir.join("missing.png")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
