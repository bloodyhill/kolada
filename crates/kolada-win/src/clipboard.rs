//! Text on the clipboard: plain, and HTML beside it for rich text. Windows
//! holds plain text as UTF-16 with CRLF line breaks; notes use LF.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::core::w;

/// windows-rs puts this one in System::Ole, a large feature for one constant.
const CF_UNICODETEXT: u32 = 13;

pub fn get(hwnd: HWND) -> Option<String> {
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let text = (|| {
            let global = HGLOBAL(GetClipboardData(CF_UNICODETEXT).ok()?.0);
            let ptr = GlobalLock(global) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let units = std::slice::from_raw_parts(ptr, GlobalSize(global) / 2);
            let len = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            let text = String::from_utf16_lossy(&units[..len]);
            let _ = GlobalUnlock(global);
            Some(text)
        })();
        let _ = CloseClipboard();
        Some(text?.replace("\r\n", "\n").replace('\r', "\n"))
    }
}

fn html_format() -> u32 {
    unsafe { RegisterClipboardFormatW(w!("HTML Format")) }
}

/// Whether what was copied came with its formatting (as HTML).
pub fn has_html() -> bool {
    let format = html_format();
    format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok()
}

/// What was copied with its formatting: the clipboard's HTML as it holds
/// it (see `cf_html_page`).
pub fn get_html(hwnd: HWND) -> Option<Vec<u8>> {
    let format = html_format();
    if format == 0 {
        return None;
    }
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let bytes = (|| {
            let global = HGLOBAL(GetClipboardData(format).ok()?.0);
            let ptr = GlobalLock(global) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let all = std::slice::from_raw_parts(ptr, GlobalSize(global));
            let len = all.iter().position(|&b| b == 0).unwrap_or(all.len());
            let bytes = all[..len].to_vec();
            let _ = GlobalUnlock(global);
            Some(bytes)
        })();
        let _ = CloseClipboard();
        bytes
    }
}

/// The page in the clipboard's HTML (see `cf_html`): from its start to its
/// end as the header says (or the fragment alone, when the page's ends
/// are not given), and the address it was copied from.
pub fn cf_html_page(bytes: &[u8]) -> Option<(String, Option<String>)> {
    let head_end = bytes.windows(1).position(|w| w[0] == b'<').unwrap_or(bytes.len());
    let head = String::from_utf8_lossy(&bytes[..head_end]);
    let field = |name: &str| head.lines().find_map(|l| l.strip_prefix(name)?.strip_prefix(':').map(|v| v.trim().to_string()));
    let offset = |name: &str| field(name).and_then(|v| v.parse::<i64>().ok()).filter(|&n| n >= 0).map(|n| n as usize);
    let span = |a: Option<usize>, b: Option<usize>| match (a, b) {
        (Some(a), Some(b)) if a < b && b <= bytes.len() => Some((a, b)),
        _ => None,
    };
    let (a, b) = span(offset("StartHTML"), offset("EndHTML"))
        .or_else(|| span(offset("StartFragment"), offset("EndFragment")))
        .unwrap_or((head_end, bytes.len()));
    let source = field("SourceURL").filter(|u| !u.is_empty());
    Some((String::from_utf8_lossy(&bytes[a..b]).into_owned(), source))
}

/// A picture on the clipboard: a PNG file as it is, or pixels (32-bit
/// BGRA, top-down rows).
pub enum Picture {
    Png(Vec<u8>),
    Pixels { width: u32, height: u32, bgra: Vec<u8> },
}

/// windows-rs puts these in System::Ole too.
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;

/// The bytes of the open clipboard's `format`, if it has it.
unsafe fn data(format: u32) -> Option<Vec<u8>> {
    unsafe {
        let global = HGLOBAL(GetClipboardData(format).ok()?.0);
        let ptr = GlobalLock(global) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, GlobalSize(global)).to_vec();
        let _ = GlobalUnlock(global);
        Some(bytes)
    }
}

/// A device-independent bitmap (as the clipboard holds screenshots) as
/// top-down BGRA: 32 or 24 bits a pixel, uncompressed or with bit masks.
pub fn from_dib(dib: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let u32_at = |at: usize| Some(u32::from_le_bytes(dib.get(at..at + 4)?.try_into().ok()?));
    let header = u32_at(0)? as usize;
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bits = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?) as usize;
    let compression = u32_at(16)?;
    if width <= 0 || height == 0 || !(bits == 32 || bits == 24) || !(compression == 0 || compression == 3) {
        return None;
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let stride = (w * bits).div_ceil(32) * 4;
    // A color table may follow the header (rare at these depths), and bit
    // masks follow a basic header. Later headers hold the masks inside, but
    // the ones Windows makes from a bitmap repeat them after it too.
    let mut offset = header + u32_at(32)? as usize * 4;
    if compression == 3
        && (header == 40 || (dib.get(header..header + 12) == dib.get(40..52) && dib.len() >= offset + 12 + stride * h))
    {
        offset += 12;
    }
    let pixels = dib.get(offset..offset + stride * h)?;
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let src = &pixels[if height > 0 { h - 1 - y } else { y } * stride..][..stride];
        for x in 0..w {
            let d = &mut out[(y * w + x) * 4..][..4];
            let s = &src[x * bits / 8..];
            d.copy_from_slice(&[s[0], s[1], s[2], if bits == 32 { s[3] } else { 255 }]);
        }
    }
    // Most 32-bit screenshots leave alpha at zero: they are opaque.
    if bits == 32 && out.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
        out.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
    }
    Some((w as u32, h as u32, out))
}

/// The picture on the clipboard, if there is one and no text.
pub fn get_picture(hwnd: HWND) -> Option<Picture> {
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let picture = (|| {
            if GetClipboardData(CF_UNICODETEXT).is_ok() {
                return None;
            }
            let png = RegisterClipboardFormatW(w!("PNG"));
            if png != 0 && let Some(bytes) = data(png) {
                return Some(Picture::Png(bytes));
            }
            let dib = data(CF_DIBV5).or_else(|| data(CF_DIB))?;
            let (width, height, bgra) = from_dib(&dib)?;
            Some(Picture::Pixels { width, height, bgra })
        })();
        let _ = CloseClipboard();
        picture
    }
}

/// Puts `bytes` on the open clipboard as `format`.
unsafe fn put(format: u32, bytes: &[u8]) -> Option<()> {
    unsafe {
        let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).ok()?;
        let ptr = GlobalLock(global) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(Some(global));
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(global);
        // The clipboard owns the memory once this succeeds.
        if SetClipboardData(format, Some(HANDLE(global.0))).is_err() {
            let _ = GlobalFree(Some(global));
            return None;
        }
        Some(())
    }
}

fn wide(text: &str) -> Vec<u8> {
    text.replace('\n', "\r\n").encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

pub fn set(hwnd: HWND, text: &str) -> bool {
    set_rich(hwnd, text, None)
}

/// Plain `text`, and with `html` (a fragment) the same as rich text, which
/// word processors and mail keep formatted when it is pasted.
pub fn set_rich(hwnd: HWND, text: &str, html: Option<&str>) -> bool {
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let done = (|| {
            EmptyClipboard().ok()?;
            put(CF_UNICODETEXT, &wide(text))?;
            if let Some(html) = html {
                let format = html_format();
                if format != 0 {
                    put(format, &cf_html(html))?;
                }
            }
            Some(())
        })();
        let _ = CloseClipboard();
        done.is_some()
    }
}

/// The clipboard's HTML format: a header of byte offsets, then the page
/// with the fragment marked, in UTF-8.
fn cf_html(fragment: &str) -> Vec<u8> {
    const HEADER: &str = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let before = "<html><head><meta charset=\"utf-8\"></head><body><!--StartFragment-->";
    let after = "<!--EndFragment--></body></html>";
    let start_html = HEADER.len();
    let start_fragment = start_html + before.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + after.len();
    let header = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n"
    );
    let mut out = format!("{header}{before}{fragment}{after}").into_bytes();
    out.push(0);
    out
}

#[cfg(test)]
mod tests {
    use super::cf_html;

    #[test]
    fn the_clipboards_html_is_read_as_its_page_and_source() {
        let bytes = cf_html("<p>Hi <b>there</b></p>");
        let (page, source) = super::cf_html_page(&bytes[..bytes.len() - 1]).unwrap();
        assert!(page.starts_with("<html>") && page.contains("<p>Hi <b>there</b></p>") && page.ends_with("</html>"), "{page}");
        assert_eq!(source, None);
        // A browser's: the page's ends not given (-1), its address said.
        let head = "Version:1.0\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:0000000140\r\nEndFragment:0000000152\r\nSourceURL:https://example.com/post\r\n";
        let mut bytes = head.as_bytes().to_vec();
        bytes.resize(140, b' ');
        bytes.extend_from_slice(b"<p>copied</p>");
        let (page, source) = super::cf_html_page(&bytes).unwrap();
        assert_eq!(page, "<p>copied</p");
        assert_eq!(source.as_deref(), Some("https://example.com/post"));
    }

    #[test]
    fn skips_masks_repeated_after_a_v5_header() {
        // 1 x 1, 32 bits, a V5 header with bit masks, repeated after it.
        let mut dib = vec![0u8; 124];
        dib[0] = 124;
        dib[4] = 1;
        dib[8] = 1;
        dib[14] = 32;
        dib[16] = 3;
        let masks = [0, 0, 0xFF, 0, 0, 0xFF, 0, 0, 0xFF, 0, 0, 0];
        dib[40..52].copy_from_slice(&masks);
        dib.extend_from_slice(&masks);
        dib.extend_from_slice(&[1, 2, 3, 255]);
        assert_eq!(super::from_dib(&dib).unwrap().2, vec![1, 2, 3, 255]);
        // Without the repeat, the pixels come right after the header.
        let mut dib = dib[..124].to_vec();
        dib.extend_from_slice(&[4, 5, 6, 255]);
        assert_eq!(super::from_dib(&dib).unwrap().2, vec![4, 5, 6, 255]);
    }

    #[test]
    fn reads_bitmaps_bottom_up_and_top_down() {
        // 2 x 2, 24 bits: rows of 6 bytes padded to 8, bottom row first.
        let mut dib = vec![0u8; 40];
        dib[0] = 40;
        dib[4] = 2;
        dib[8] = 2;
        dib[14] = 24;
        dib.extend_from_slice(&[1, 2, 3, 4, 5, 6, 0, 0]); // bottom row
        dib.extend_from_slice(&[7, 8, 9, 10, 11, 12, 0, 0]); // top row
        let (w, h, px) = super::from_dib(&dib).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(&px[..8], &[7, 8, 9, 255, 10, 11, 12, 255], "the top row first");
        // Top-down (negative height), 32 bits with no alpha: opaque.
        let mut dib = vec![0u8; 40];
        dib[0] = 40;
        dib[4] = 1;
        dib[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        dib[14] = 32;
        dib.extend_from_slice(&[9, 8, 7, 0]);
        assert_eq!(super::from_dib(&dib).unwrap().2, vec![9, 8, 7, 255]);
    }

    #[test]
    fn html_offsets_point_at_the_fragment() {
        let bytes = cf_html("<p>ملاحظة</p>");
        let s = std::str::from_utf8(&bytes[..bytes.len() - 1]).unwrap();
        let field = |name: &str| -> usize {
            let at = s.find(name).unwrap() + name.len() + 1;
            s[at..at + 10].parse().unwrap()
        };
        assert_eq!(&s[field("StartFragment")..field("EndFragment")], "<p>ملاحظة</p>");
        assert!(s[field("StartHTML")..].starts_with("<html>"));
        assert_eq!(field("EndHTML"), s.len());
    }
}
