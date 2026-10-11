//! Windows' own HTTP client (winhttp, loaded the first time it is needed),
//! proxies as the system has them, redirects followed, in limited time and
//! size. Notsy goes online for two things only: a picture pasted with what
//! was copied from a page (see `convert::paste`), and the daily check for
//! updates, which the options can turn off (see `updates`; never in the
//! Store's build).

use windows::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPEN_REQUEST_FLAGS,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::{PCWSTR, w};

/// The most a picture may weigh.
pub const MOST: usize = 20 * 1024 * 1024;

/// A winhttp handle, closed when dropped.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// What `url` (http or https) answers, when it answers 200 with at most
/// `most` bytes: 5 s to find and reach the server, 15 s for each wait
/// after.
pub fn get(url: &str, most: usize) -> Option<Vec<u8>> {
    let wide: Vec<u16> = url.encode_utf16().collect();
    let mut parts = URL_COMPONENTS {
        dwStructSize: size_of::<URL_COMPONENTS>() as u32,
        dwSchemeLength: u32::MAX,
        dwHostNameLength: u32::MAX,
        dwUrlPathLength: u32::MAX,
        dwExtraInfoLength: u32::MAX,
        ..Default::default()
    };
    unsafe { WinHttpCrackUrl(&wide, 0, &mut parts).ok()? };
    if parts.lpszHostName.is_null() || parts.dwHostNameLength == 0 {
        return None;
    }
    let piece = |p: *mut u16, n: u32| unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec();
    let mut host = piece(parts.lpszHostName.0, parts.dwHostNameLength);
    host.push(0);
    // The path and the query after it, as one.
    let mut path = if parts.lpszUrlPath.is_null() { Vec::new() } else { piece(parts.lpszUrlPath.0, parts.dwUrlPathLength + parts.dwExtraInfoLength) };
    if path.is_empty() {
        path.push(b'/' as u16);
    }
    path.push(0);
    let secure = parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;
    unsafe {
        let session = Handle(WinHttpOpen(w!("Notsy"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return None;
        }
        let _ = WinHttpSetTimeouts(session.0, 5_000, 5_000, 15_000, 15_000);
        let connect = Handle(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), parts.nPort, 0));
        if connect.0.is_null() {
            return None;
        }
        let flags = if secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let request = Handle(WinHttpOpenRequest(connect.0, w!("GET"), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags));
        if request.0.is_null() {
            return None;
        }
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).ok()?;
        let (mut status, mut size) = (0u32, size_of::<u32>() as u32);
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut size,
            std::ptr::null_mut(),
        )
        .ok()?;
        if status != 200 {
            return None;
        }
        let mut body = Vec::new();
        loop {
            let mut ready = 0u32;
            WinHttpQueryDataAvailable(request.0, &mut ready).ok()?;
            if ready == 0 {
                return Some(body);
            }
            if body.len() + ready as usize > most {
                return None;
            }
            let at = body.len();
            body.resize(at + ready as usize, 0);
            let mut read = 0u32;
            WinHttpReadData(request.0, body[at..].as_mut_ptr() as *mut _, ready, &mut read).ok()?;
            body.truncate(at + read as usize);
        }
    }
}

/// Whether `bytes` are a picture Notsy shows (PNG, JPEG, GIF, WebP, BMP),
/// by how they start: a page's error answered in its place is not.
pub fn is_picture(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
        || bytes.starts_with(b"BM")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_known_by_how_they_start() {
        assert!(is_picture(b"\x89PNG\r\n\x1a\nrest"));
        assert!(is_picture(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(is_picture(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(!is_picture(b"<!DOCTYPE html><html>"));
        assert!(!is_picture(b"RIFF\0\0\0\0WAVEfmt "));
        assert!(!is_picture(b""));
    }

    #[test]
    #[ignore = "goes online: fetches a picture from the web"]
    fn fetches_a_picture_from_the_web() {
        let bytes = get("https://www.rust-lang.org/static/images/rust-logo-blk.svg", MOST).expect("an answer");
        assert!(bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml"));
        assert!(get("https://www.rust-lang.org/no-such-picture.png", MOST).is_none(), "a 404 is no picture");
    }
}
