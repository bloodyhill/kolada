//! Windows' own HTTP client (winhttp, loaded the first time it is needed),
//! proxies as the system has them, redirects followed, in limited time and
//! size. Kolada goes online for four things only: a picture pasted with
//! what was copied from a page (see `convert::paste`), the daily check for
//! updates, which the options can turn off (see `updates`; never in the
//! Store's build), a note the user shares as a link on notsy.co, with the
//! list and deletion of those links (see `share`; only when asked), and the
//! writing model the user chooses, downloaded once (`get_from`).

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
    send("GET", url, &[], &[], most).and_then(|(status, body)| (status == 200).then_some(body))
}

/// A request (`method` on `url`, with `headers` and `body`) and whatever
/// the server answers: its status and at most `most` bytes of its body.
/// None when no answer comes (offline, refused, timed out, too long). Same
/// times, proxies and redirects as `get`.
pub fn send(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8], most: usize) -> Option<(u16, Vec<u8>)> {
    let answer = open(method, url, headers, body)?;
    let mut body = Vec::new();
    loop {
        let ready = answer.ready()?;
        if ready == 0 {
            return Some((answer.status, body));
        }
        if body.len() + ready as usize > most {
            return None;
        }
        let at = body.len();
        body.resize(at + ready as usize, 0);
        let read = answer.read(&mut body[at..])?;
        body.truncate(at + read);
    }
}

/// What a download reads at a time, into the one buffer.
const PIECE: usize = 64 * 1024;

/// The file `url` answers, from byte `from` on (asked with `Range:
/// bytes=from-` when `from` is past the start), handed to `body` in pieces
/// of at most 64 KiB as they arrive, never the whole file at once. `body`
/// returning false stops it. The status when the server answers 200 (the
/// whole file: a server that ignores the range answers so too) or 206 (the
/// rest), whether or not all of it came; None for any other status, or when
/// no answer comes. Same times, proxies and redirects as `get`.
pub fn get_from(url: &str, from: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16> {
    let range = format!("bytes={from}-");
    let headers: &[(&str, &str)] = if from > 0 { &[("Range", &range)] } else { &[] };
    let answer = open("GET", url, headers, &[])?;
    if !matches!(answer.status, 200 | 206) {
        return None;
    }
    let mut piece = vec![0u8; PIECE];
    loop {
        let ready = match answer.ready() {
            Some(0) | None => break,
            Some(n) => (n as usize).min(PIECE),
        };
        let Some(read) = answer.read(&mut piece[..ready]) else { break };
        if read == 0 || !body(&piece[..read]) {
            break;
        }
    }
    Some(answer.status)
}

/// The downloader kolada-core's `fetch` uses on Windows: `get_from`.
pub struct Http;

impl kolada_core::fetch::Source for Http {
    fn get_from(&self, url: &str, from: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16> {
        get_from(url, from, body)
    }
}

/// A request sent and its answer's headers in, its body still to read. Its
/// handles close in the order they must: the request first.
struct Answer {
    request: Handle,
    _connect: Handle,
    _session: Handle,
    status: u16,
}

impl Answer {
    /// How many bytes can be read now; 0 at the end of the body.
    fn ready(&self) -> Option<u32> {
        let mut ready = 0u32;
        unsafe { WinHttpQueryDataAvailable(self.request.0, &mut ready) }.ok()?;
        Some(ready)
    }

    /// Reads into `into`, and says how much came.
    fn read(&self, into: &mut [u8]) -> Option<usize> {
        let mut read = 0u32;
        unsafe { WinHttpReadData(self.request.0, into.as_mut_ptr() as *mut _, u32::try_from(into.len()).ok()?, &mut read) }.ok()?;
        Some(read as usize)
    }
}

/// Sends `method` on `url` with `headers` and `body`, and waits for the
/// answer's headers: 5 s to find and reach the server, 15 s for each wait
/// after; proxies as the system has them, redirects followed.
fn open(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Option<Answer> {
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
    let verb: Vec<u16> = method.encode_utf16().chain([0]).collect();
    let extra: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
    let extra: Vec<u16> = extra.encode_utf16().collect();
    let secure = parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;
    let length = u32::try_from(body.len()).ok()?;
    unsafe {
        let session = Handle(WinHttpOpen(w!("Kolada"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return None;
        }
        let _ = WinHttpSetTimeouts(session.0, 5_000, 5_000, 15_000, 15_000);
        let connect = Handle(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), parts.nPort, 0));
        if connect.0.is_null() {
            return None;
        }
        let flags = if secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let request = Handle(WinHttpOpenRequest(connect.0, PCWSTR(verb.as_ptr()), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags));
        if request.0.is_null() {
            return None;
        }
        let optional = (!body.is_empty()).then_some(body.as_ptr() as *const core::ffi::c_void);
        WinHttpSendRequest(request.0, (!extra.is_empty()).then_some(&extra[..]), optional, length, length, 0).ok()?;
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
        Some(Answer { request, _connect: connect, _session: session, status: u16::try_from(status).ok()? })
    }
}

/// Whether `bytes` are a picture Kolada shows (PNG, JPEG, GIF, WebP, BMP),
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
    use kolada_core::fetch::Source;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

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

    /// Answers one request on `stream` as a file server would: `/file` is
    /// `file` (its rest from a `Range: bytes=N-`), `/moved` sends there,
    /// anything else is not found.
    fn answer(mut stream: TcpStream, file: &[u8]) {
        let mut asked = Vec::new();
        let mut byte = [0u8; 1];
        while !asked.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(1) => asked.push(byte[0]),
                _ => return,
            }
        }
        let asked = String::from_utf8_lossy(&asked).into_owned();
        let path = asked.split_whitespace().nth(1).unwrap_or("");
        let from = asked
            .lines()
            .find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").and_then(|r| r.trim_end_matches('-').parse::<usize>().ok()));
        let head = match (path, from) {
            ("/moved", _) => "HTTP/1.1 302 Found\r\nLocation: /file\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
            ("/file", Some(from)) => {
                format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", file.len() - 1, file.len(), file.len() - from)
            }
            ("/file", None) => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", file.len()),
            _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
        };
        let _ = stream.write_all(head.as_bytes());
        if path == "/file" {
            // A reader that stopped early hangs up: what it left goes nowhere.
            let _ = stream.write_all(&file[from.unwrap_or(0)..]);
        }
    }

    /// A file server on this machine only, for as long as the test runs.
    fn serve(file: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let file = std::sync::Arc::new(file);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let file = file.clone();
                std::thread::spawn(move || answer(stream, &file));
            }
        });
        address
    }

    /// What `get_from` hands over, and the pieces it came in.
    fn fetch(url: &str, from: u64, stop_after: usize) -> (Option<u16>, Vec<u8>, usize) {
        let (mut got, mut pieces) = (Vec::new(), 0);
        let status = Http.get_from(url, from, &mut |piece| {
            assert!(!piece.is_empty() && piece.len() <= PIECE, "a piece of {} bytes", piece.len());
            got.extend_from_slice(piece);
            pieces += 1;
            pieces < stop_after
        });
        (status, got, pieces)
    }

    #[test]
    fn streams_a_file_and_resumes_from_a_byte() {
        // Several pieces' worth, no two alike in a row.
        let file: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let server = serve(file.clone());
        let url = format!("{server}/file");

        let (status, got, pieces) = fetch(&url, 0, usize::MAX);
        assert_eq!(status, Some(200));
        assert!(got == file, "the whole file, {} bytes of {}", got.len(), file.len());
        assert!(pieces >= file.len() / PIECE, "in pieces, not at once");

        let from = 123_457;
        let (status, got, _) = fetch(&url, from as u64, usize::MAX);
        assert_eq!(status, Some(206));
        assert!(got == file[from..], "only the rest, {} bytes", got.len());

        // The range holds across a redirect, as a download's link has one.
        let (status, got, _) = fetch(&format!("{server}/moved"), from as u64, usize::MAX);
        assert_eq!(status, Some(206));
        assert!(got == file[from..]);

        // Told to stop, it stops: one piece and no more.
        let (status, got, pieces) = fetch(&url, 0, 1);
        assert_eq!((status, pieces), (Some(200), 1));
        assert!(got.len() <= PIECE && got[..] == file[..got.len()]);

        assert_eq!(fetch(&format!("{server}/nothing"), 0, usize::MAX), (None, Vec::new(), 0), "a 404 hands over nothing");
    }
}
