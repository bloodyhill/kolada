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
/// Files copied (in Explorer, Ctrl+C): their paths, or none.
pub fn get_files(hwnd: HWND) -> Vec<std::path::PathBuf> {
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return Vec::new();
        }
        let files = (|| {
            let drop = HDROP(GetClipboardData(CF_HDROP.0 as u32).ok()?.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            Some((0..count).filter_map(|i| {
                let mut buf = vec![0u16; DragQueryFileW(drop, i, None) as usize + 1];
                let n = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
                (n > 0).then(|| std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n])))
            }).collect::<Vec<_>>())
        })();
        let _ = CloseClipboard();
        files.unwrap_or_default()
    }
}

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
    let mut formats = vec![(Format::Text, wide(text))];
    if let Some(html) = html {
        formats.push((Format::Html, cf_html(html)));
    }
    put_all(hwnd, &formats)
}

/// The clipboard formats Kolada writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Text,
    Html,
    /// The selection as Markdown, exactly: Kolada's own (see `kolada_format`).
    Kolada,
    /// Windows' own markers, which password managers set: keep this out of
    /// clipboard history, the cloud clipboard and clipboard watchers.
    Private(&'static windows::core::PCWSTR),
}

const EXCLUDE: windows::core::PCWSTR = w!("ExcludeClipboardContentFromMonitorProcessing");
const IN_HISTORY: windows::core::PCWSTR = w!("CanIncludeInClipboardHistory");
const TO_CLOUD: windows::core::PCWSTR = w!("CanUploadToCloudClipboard");

/// Whether what is on the clipboard came marked private: a password
/// manager's copy (Quick Send then sends it as a password).
pub fn concealed() -> bool {
    unsafe {
        let exclude = RegisterClipboardFormatW(EXCLUDE);
        if exclude != 0 && IsClipboardFormatAvailable(exclude).is_ok() {
            return true;
        }
        let history = RegisterClipboardFormatW(IN_HISTORY);
        history != 0 && IsClipboardFormatAvailable(history).is_ok() && read_dword(history) == Some(0)
    }
}

unsafe fn read_dword(format: u32) -> Option<u32> {
    unsafe {
        OpenClipboard(None).ok()?;
        let v = (|| {
            let global = HGLOBAL(GetClipboardData(format).ok()?.0);
            let ptr = GlobalLock(global) as *const u32;
            if ptr.is_null() || GlobalSize(global) < 4 {
                return None;
            }
            let v = *ptr;
            let _ = GlobalUnlock(global);
            Some(v)
        })();
        let _ = CloseClipboard();
        v
    }
}

/// A password on the clipboard, marked so clipboard history, the cloud
/// clipboard and clipboard watchers leave it alone.
pub fn set_secret(hwnd: HWND, text: &str) -> bool {
    put_all(hwnd, &[
        (Format::Text, wide(text)),
        (Format::Private(&EXCLUDE), vec![0u8; 4]),
        (Format::Private(&IN_HISTORY), 0u32.to_le_bytes().to_vec()),
        (Format::Private(&TO_CLOUD), 0u32.to_le_bytes().to_vec()),
    ])
}

/// Whether the clipboard still holds `text` (to clear a password from it).
pub fn holds(hwnd: HWND, text: &str) -> bool {
    get(hwnd).is_some_and(|t| t == text)
}

/// Empties the clipboard.
pub fn clear(hwnd: HWND) {
    unsafe {
        if OpenClipboard(Some(hwnd)).is_ok() {
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
        }
    }
}

/// Kolada's own clipboard format: what was copied as the Markdown it is,
/// so a copy in Kolada pastes back exactly, whatever the plain text says.
fn kolada_format() -> u32 {
    unsafe { RegisterClipboardFormatW(w!("Kolada Markdown")) }
}

/// Empties the clipboard and puts `formats` on it.
fn put_all(hwnd: HWND, formats: &[(Format, Vec<u8>)]) -> bool {
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let done = (|| {
            EmptyClipboard().ok()?;
            for (format, bytes) in formats {
                let id = match format {
                    Format::Text => CF_UNICODETEXT,
                    Format::Html => html_format(),
                    Format::Kolada => kolada_format(),
                    Format::Private(name) => RegisterClipboardFormatW(**name),
                };
                if id != 0 {
                    put(id, bytes)?;
                }
            }
            Some(())
        })();
        let _ = CloseClipboard();
        done.is_some()
    }
}

/// A picture on the clipboard: as a device-independent bitmap, which every
/// app takes, and as a PNG beside it when there is one (apps that keep
/// transparency take that). `bgra` is top-down rows, `width` x `height`.
pub fn set_picture(hwnd: HWND, width: u32, height: u32, bgra: &[u8], png: Option<&[u8]>) -> bool {
    if bgra.len() < (width * height * 4) as usize {
        return false;
    }
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let done = (|| {
            EmptyClipboard().ok()?;
            put(CF_DIB, &to_dib(width, height, bgra))?;
            if let Some(png) = png {
                let id = RegisterClipboardFormatW(w!("PNG"));
                if id != 0 {
                    put(id, png)?;
                }
            }
            Some(())
        })();
        let _ = CloseClipboard();
        done.is_some()
    }
}

/// Files on the clipboard, as Explorer puts them there (CF_HDROP): pasted
/// into Explorer, a chat app or a mail, they all come.
pub fn set_files(hwnd: HWND, paths: &[std::path::PathBuf]) -> bool {
    use windows::Win32::System::Ole::CF_HDROP;
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let done = (|| {
            EmptyClipboard().ok()?;
            put(CF_HDROP.0 as u32, &drop_files(paths))
        })();
        let _ = CloseClipboard();
        done.is_some()
    }
}

/// A DROPFILES block: its 20-byte head (where the names start, a point,
/// not client area, wide names), then each path in UTF-16, ended by an
/// empty one.
pub fn drop_files(paths: &[std::path::PathBuf]) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [20u32, 0, 0, 0, 1] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for p in paths {
        for u in p.to_string_lossy().encode_utf16().chain([0]) {
            b.extend_from_slice(&u.to_le_bytes());
        }
    }
    b.extend_from_slice(&[0, 0]);
    b
}

/// Top-down BGRA as a 32-bit device-independent bitmap (bottom-up rows).
pub fn to_dib(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut dib = Vec::with_capacity(40 + w * h * 4);
    for v in [40u32, width, height, 1 | (32 << 16), 0, (w * h * 4) as u32, 2835, 2835, 0, 0] {
        dib.extend_from_slice(&v.to_le_bytes());
    }
    for y in (0..h).rev() {
        dib.extend_from_slice(&bgra[y * w * 4..(y + 1) * w * 4]);
    }
    dib
}

/// What Copy puts on the clipboard, as each format's bytes: the plain
/// text (UTF-16, CRLF), the HTML (see `cf_html`), and the Markdown in
/// Kolada's own format (UTF-8, ended by a zero).
fn copy_formats(copied: &kolada_editor::slice::Copied) -> Vec<(Format, Vec<u8>)> {
    let mut kolada = copied.markdown.as_bytes().to_vec();
    kolada.push(0);
    vec![(Format::Text, wide(&copied.plain)), (Format::Html, cf_html(&copied.html)), (Format::Kolada, kolada)]
}

/// The selection copied (see `Editor::copied`): plain, rich and Kolada's own.
pub fn set_copied(hwnd: HWND, copied: &kolada_editor::slice::Copied) -> bool {
    put_all(hwnd, &copy_formats(copied))
}

/// The Markdown in Kolada's own format, as `copy_formats` wrote it.
fn read_kolada(bytes: &[u8]) -> Option<String> {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..len]).ok().map(str::to_string)
}

/// What a copy in Kolada left on the clipboard, as its Markdown, if the
/// clipboard still holds one.
pub fn get_kolada(hwnd: HWND) -> Option<String> {
    let format = kolada_format();
    if format == 0 || unsafe { IsClipboardFormatAvailable(format) }.is_err() {
        return None;
    }
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let bytes = data(format);
        let _ = CloseClipboard();
        read_kolada(&bytes?)
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

    /// Pictures copied as files are a DROPFILES block, as Explorer's own:
    /// its head says the names start at 20 and are wide; each path in
    /// UTF-16 and a zero, then one more zero. Put on the clipboard, they
    /// read back as the same paths.
    #[test]
    fn pictures_go_on_the_clipboard_as_files() {
        let paths = vec![std::path::PathBuf::from(r"C:\Kolada\copied\IMG_1.jpg"), std::path::PathBuf::from(r"C:\Kolada\copied\صورة.png")];
        let b = super::drop_files(&paths);
        assert_eq!(&b[..4], &20u32.to_le_bytes());
        assert_eq!(&b[16..20], &1u32.to_le_bytes(), "wide names");
        let units: Vec<u16> = b[20..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let names: Vec<String> = units.split(|u| *u == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect();
        assert_eq!(names, vec![paths[0].to_string_lossy().into_owned(), paths[1].to_string_lossy().into_owned()]);
        assert!(units.ends_with(&[0, 0]), "ended by an empty name");
    }

    #[test]
    fn a_picture_goes_on_the_clipboard_as_a_bitmap_that_reads_back() {
        let bgra: Vec<u8> = (0..3 * 2).flat_map(|i| [i as u8, 10, 20, 255]).collect();
        let dib = super::to_dib(3, 2, &bgra);
        assert_eq!(super::from_dib(&dib), Some((3, 2, bgra)), "the same pixels, the same way up");
    }

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

    /// The plain text as the clipboard holds it, read back as `get` does.
    fn unwide(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let len = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..len]).replace("\r\n", "\n")
    }

    fn copy_of(text: &str, from: &str) -> kolada_editor::slice::Copied {
        let mut e = kolada_editor::Editor::new(text);
        e.set_selection(kolada_editor::Selection { anchor: text.find(from).unwrap(), head: text.len() });
        e.copied(false).unwrap()
    }

    #[test]
    fn copy_writes_plain_html_and_kolada_markdown() {
        use super::{Format, copy_formats};
        let text = "Intro\n- one **two** [site](https://x.io)\n- [x] done";
        let formats = copy_formats(&copy_of(text, "ne **"));
        let kinds: Vec<Format> = formats.iter().map(|(f, _)| *f).collect();
        assert_eq!(kinds, [Format::Text, Format::Html, Format::Kolada]);
        let plain = &formats[0].1;
        assert!(plain.ends_with(&[0, 0]), "the plain text ends in a zero");
        assert!(plain.windows(4).any(|w| w == [b'\r', 0, b'\n', 0]), "Windows' line breaks");
        assert_eq!(unwide(plain), "\u{2022} ne two site (https://x.io)\n\u{2611} done", "plain text, no marks");
        let (page, _) = super::cf_html_page(&formats[1].1[..formats[1].1.len() - 1]).unwrap();
        assert!(page.contains("<strong>two</strong>") && page.contains("<a href=\"https://x.io\">site</a>") && page.contains("class=\"task done\""), "{page}");
        let kolada = &formats[2].1;
        assert_eq!(kolada.last(), Some(&0));
        assert_eq!(super::read_kolada(kolada).as_deref(), Some("- ne **two** [site](https://x.io)\n- [x] done"), "the Markdown, exactly");
    }

    #[test]
    fn a_kolada_copy_pastes_back_exactly() {
        let text = "# Plan\nShip **bold** ==now==\n- [ ] task [[Note|alias]]\n\n| a | b |\n| - | - |\n| 1 | 2 |\n";
        let formats = super::copy_formats(&copy_of(text, "# Plan"));
        let markdown = super::read_kolada(&formats[2].1).unwrap();
        let mut into = kolada_editor::Editor::new("");
        into.paste_markdown(&markdown);
        assert_eq!(into.text(), text);
        // Part of it, pasted into another note: its marks come whole.
        let formats = super::copy_formats(&copy_of(text, "old** =="));
        let mut into = kolada_editor::Editor::new("Before ");
        into.set_selection(kolada_editor::Selection::caret(7));
        into.paste_markdown(&super::read_kolada(&formats[2].1).unwrap());
        assert!(into.text().starts_with("Before **old** ==now==\n- [ ] task [[Note|alias]]"), "{:?}", into.text().len());
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
