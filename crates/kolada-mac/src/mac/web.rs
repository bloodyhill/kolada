//! The web, through Foundation's own loading (it follows redirects and
//! speaks the system's proxies and certificates): a pasted page's pictures,
//! a note shared as a link (any method, any status: `send`), and the
//! writing model, streamed in pieces with `Range` so it never sits whole in
//! memory and picks up where it stopped (`get_from`). Each call blocks the
//! thread it is made on, never the window's.

#![allow(deprecated)]

use objc2::rc::Retained;
use objc2_foundation::{NSData, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL, NSURLConnection, NSURLResponse};

/// Foundation's loading, for the model's `Web` service.
pub struct MacWeb;

impl kolada_app::services::Web for MacWeb {
    fn get(&self, address: &str, most: usize) -> Option<Vec<u8>> {
        get(address, most)
    }

    fn send(&self, method: &str, address: &str, headers: &[(&str, &str)], body: &[u8], most: usize) -> Option<(u16, Vec<u8>)> {
        let (status, data, _) = request(method, address, headers, body, 60.0)?;
        (data.len() <= most).then_some((status, data))
    }

    fn get_from(&self, address: &str, from: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16> {
        get_from(address, from, PIECE, body)
    }

    fn random(&self) -> Option<[u8; 32]> {
        let mut b = [0u8; 32];
        (unsafe { libc::getentropy(b.as_mut_ptr().cast(), b.len()) } == 0).then_some(b)
    }
}

/// Bytes asked for at a time while streaming a big file.
const PIECE: u64 = 4 << 20;

fn web_address(address: &str) -> Option<Retained<NSURL>> {
    let lower = address.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return None;
    }
    NSURL::URLWithString(&NSString::from_str(address))
}

/// The bytes at `address` (http or https only), if they come and are no more
/// than `most`.
pub fn get(address: &str, most: usize) -> Option<Vec<u8>> {
    let url = web_address(address)?;
    let data = NSData::dataWithContentsOfURL(&url)?;
    (data.len() <= most).then(|| data.to_vec())
}

/// `method` on `address` with `headers` and `body`: the status, the body,
/// and the response's `Content-Range` (for `get_from`). None when no answer
/// came (offline, a bad address, a timeout).
fn request(method: &str, address: &str, headers: &[(&str, &str)], body: &[u8], timeout: f64) -> Option<(u16, Vec<u8>, Option<String>)> {
    let url = web_address(address)?;
    let req = NSMutableURLRequest::requestWithURL(&url);
    req.setHTTPMethod(&NSString::from_str(method));
    req.setTimeoutInterval(timeout);
    // Never an answer from the cache: a list of links, a part of a file.
    req.setCachePolicy(objc2_foundation::NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
    for (name, value) in headers {
        req.setValue_forHTTPHeaderField(Some(&NSString::from_str(value)), &NSString::from_str(name));
    }
    if !body.is_empty() {
        req.setHTTPBody(Some(&NSData::with_bytes(body)));
    }
    let mut response: Option<Retained<NSURLResponse>> = None;
    let data = NSURLConnection::sendSynchronousRequest_returningResponse_error(&req, Some(&mut response)).ok()?;
    let http = response?.downcast::<NSHTTPURLResponse>().ok()?;
    let status = u16::try_from(http.statusCode()).ok()?;
    let range = http.valueForHTTPHeaderField(&NSString::from_str("Content-Range")).map(|s| s.to_string());
    Some((status, data.to_vec(), range))
}

/// The whole length a `Content-Range: bytes a-b/total` names.
fn total_of(range: Option<&str>) -> Option<u64> {
    range?.rsplit_once('/')?.1.trim().parse().ok()
}

/// GET `address` from byte `from` on, `piece` bytes a request (`Range:
/// bytes=a-b`), handing each to `body` until the file ends or `body`
/// says stop. 206 while the server answers ranges, 200 when it sent the
/// whole file instead (once, from its start), None when the network failed
/// on the way (what came before is kept by the caller).
fn get_from(address: &str, from: u64, piece: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16> {
    let mut at = from;
    loop {
        let range = format!("bytes={at}-{}", at + piece - 1);
        let (status, data, content_range) = request("GET", address, &[("Range", &range)], &[], 120.0)?;
        match status {
            // The whole file, the range ignored: given once, as it came.
            200 => {
                body(&data);
                return Some(200);
            }
            206 => {}
            // Asked from its very end: nothing more to come.
            416 if at > 0 => return Some(206),
            other => return Some(other),
        }
        if data.is_empty() || !body(&data) {
            return Some(206);
        }
        at += data.len() as u64;
        let ended = match total_of(content_range.as_deref()) {
            Some(total) => at >= total,
            None => (data.len() as u64) < piece,
        };
        if ended {
            return Some(206);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    /// A server on this machine that answers one request with `body`.
    fn serve(body: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/pic.png", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(body);
            }
        });
        address
    }

    #[test]
    fn a_picture_is_fetched_over_http_within_its_limit_and_only_over_http() {
        assert_eq!(get(&serve(b"\x89PNG\r\n\x1a\nabc"), 1000).as_deref(), Some(&b"\x89PNG\r\n\x1a\nabc"[..]));
        assert_eq!(get(&serve(b"0123456789"), 5), None, "too big");
        assert_eq!(get("file:///etc/hosts", 1000), None);
        assert_eq!(get("ftp://example.com/a.png", 1000), None);
    }

    /// What a request asked for: its first line, its headers and its body.
    fn read_request(stream: &mut TcpStream) -> (String, Vec<(String, String)>, Vec<u8>) {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).unwrap_or(0) == 0 {
                break;
            }
            buf.push(byte[0]);
        }
        let head = String::from_utf8_lossy(&buf).to_string();
        let mut lines = head.split("\r\n");
        let first = lines.next().unwrap_or("").to_string();
        let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
        let length: usize = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
        let mut body = vec![0u8; length];
        let _ = stream.read_exact(&mut body);
        (first, headers, body)
    }

    #[test]
    fn a_request_says_its_method_headers_and_body_and_any_status_comes_back() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/api/links", listener.local_addr().unwrap());
        let seen = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let asked = read_request(&mut stream);
            let answer = b"{\"error\":\"quota\"}";
            let _ = write!(stream, "HTTP/1.1 429 Too Many Requests\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", answer.len());
            let _ = stream.write_all(answer);
            asked
        });
        use kolada_app::services::Web;
        let answer = MacWeb.send("POST", &address, &[("X-Kolada-Key", "abc"), ("Content-Type", "text/plain")], b"hello", 1000);
        assert_eq!(answer, Some((429, b"{\"error\":\"quota\"}".to_vec())), "an error's status and body, not None");
        let (first, headers, body) = seen.join().unwrap();
        assert!(first.starts_with("POST /api/links"), "{first}");
        assert!(headers.contains(&("x-kolada-key".to_string(), "abc".to_string())));
        assert_eq!(body, b"hello");
        assert_eq!(MacWeb.send("GET", "file:///etc/hosts", &[], &[], 1000), None);
        assert!(MacWeb.random().is_some_and(|a| MacWeb.random().is_some_and(|b| a != b)));
    }

    /// A server that answers ranges of `file` (or, `ranges` false, the
    /// whole file every time), `requests` times.
    fn ranged(file: Vec<u8>, ranges: bool, requests: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/model.gguf", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for _ in 0..requests {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let (_, headers, _) = read_request(&mut stream);
                let range = headers.iter().find(|(k, _)| k == "range").map(|(_, v)| v.clone());
                let asked = range.as_deref().and_then(|r| r.strip_prefix("bytes=")).and_then(|r| r.split_once('-')).and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)));
                match asked.filter(|_| ranges) {
                    Some((a, _)) if a >= file.len() => {
                        let _ = write!(stream, "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                    Some((a, b)) => {
                        let b = b.min(file.len() - 1);
                        let _ = write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", file.len(), b + 1 - a);
                        let _ = stream.write_all(&file[a..=b]);
                    }
                    None => {
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", file.len());
                        let _ = stream.write_all(&file);
                    }
                }
            }
        });
        address
    }

    #[test]
    fn a_big_file_streams_in_pieces_and_resumes_from_a_byte() {
        let file: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let mut got = Vec::new();
        let status = get_from(&ranged(file.clone(), true, 10), 0, 1000, &mut |p| {
            got.extend_from_slice(p);
            true
        });
        assert_eq!((status, got.len()), (Some(206), file.len()));
        assert!(got == file);
        // From a byte on, to the end.
        let mut rest = Vec::new();
        let status = get_from(&ranged(file.clone(), true, 10), 7_500, 1000, &mut |p| {
            rest.extend_from_slice(p);
            true
        });
        assert_eq!(status, Some(206));
        assert!(rest == file[7_500..]);
        // Stopped by the caller after the first piece.
        let mut pieces = 0;
        assert_eq!(get_from(&ranged(file.clone(), true, 10), 0, 1000, &mut |_| { pieces += 1; false }), Some(206));
        assert_eq!(pieces, 1);
        // A server that ignores ranges: the whole file once, as a 200.
        let mut whole = Vec::new();
        assert_eq!(get_from(&ranged(file.clone(), false, 1), 5_000, 1000, &mut |p| { whole.extend_from_slice(p); true }), Some(200));
        assert!(whole == file);
        // Nobody there: no status.
        assert_eq!(get_from("http://127.0.0.1:9/none", 0, 1000, &mut |_| true), None);
    }
}
