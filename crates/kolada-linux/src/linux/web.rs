//! The web, for the model's `Web` service: a picture fetched for a paste and
//! a note shared as a link, through `curl` (on every desktop distribution),
//! so the app carries no TLS stack of its own. Only ever called from the
//! model's own threads, after the first frame.

use std::io::Write;
use std::process::{Command, Stdio};

use kolada_app::services::Web;

pub struct LinuxWeb;

/// Only the web's own addresses: never a file, never another protocol.
fn allowed(address: &str) -> bool {
    let lower = address.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && !address.chars().any(|c| c.is_control())
}

/// The arguments `curl` gets for a request (the address last, after `--`).
pub fn curl_args(method: &str, address: &str, headers: &[(&str, &str)], most: usize, body: bool) -> Vec<String> {
    let mut a: Vec<String> = ["--silent", "--location", "--max-redirs", "5", "--proto", "=https,http", "--proto-redir", "=https,http", "--max-time", "30", "--connect-timeout", "10"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // The body on stdout, then a line with the status.
    a.extend(["--max-filesize".into(), most.to_string(), "--request".into(), method.to_string(), "--write-out".into(), "\n%{http_code}".into()]);
    for (k, v) in headers {
        a.extend(["--header".into(), format!("{k}: {v}")]);
    }
    if body {
        a.extend(["--data-binary".into(), "@-".into()]);
    }
    a.extend(["--".into(), address.to_string()]);
    a
}

/// One request: the status and the body (at most `most` bytes).
fn request(method: &str, address: &str, headers: &[(&str, &str)], body: Option<&[u8]>, most: usize) -> Option<(u16, Vec<u8>)> {
    if !allowed(address) || headers.iter().any(|(k, v)| k.contains(['\r', '\n', ':']) || v.contains(['\r', '\n'])) {
        return None;
    }
    let args = curl_args(method, address, headers, most, body.is_some());
    let mut child = Command::new("curl").args(&args).stdin(if body.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    if let (Some(b), Some(mut stdin)) = (body, child.stdin.take()) {
        let _ = stdin.write_all(b);
    }
    let done = child.wait_with_output().ok()?;
    // Cut off past `most`, timed out, refused: no answer.
    if !done.status.success() {
        return None;
    }
    let (status, bytes) = answer(done.stdout)?;
    (status != 0 && bytes.len() <= most).then_some((status, bytes))
}

/// What curl wrote: the body, then a line with the status.
fn answer(mut out: Vec<u8>) -> Option<(u16, Vec<u8>)> {
    let at = out.iter().rposition(|&b| b == b'\n')?;
    let status = std::str::from_utf8(&out[at + 1..]).ok()?.trim().parse().ok()?;
    out.truncate(at);
    Some((status, out))
}

/// The arguments `curl` gets to stream a big file (an update, the writing
/// model) from byte `from` on: the answer's headers first on stdout, then
/// the body, with no time limit but a stall's (a minute under a byte a second).
pub fn stream_args(address: &str, from: u64) -> Vec<String> {
    let mut a: Vec<String> = ["--silent", "--location", "--max-redirs", "5", "--proto", "=https,http", "--proto-redir", "=https,http", "--connect-timeout", "10", "--speed-limit", "1", "--speed-time", "60", "--dump-header", "-"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if from > 0 {
        a.extend(["--range".into(), format!("{from}-")]);
    }
    a.extend(["--".into(), address.to_string()]);
    a
}

/// Reads the headers `curl --dump-header -` writes before the body: every
/// answer it was given on the way (a redirect, a proxy's, a 100 Continue)
/// and then the last. Fed what came so far, it says the last's status and
/// where its body starts, once they are in.
pub fn headers_end(seen: &[u8]) -> Option<(u16, usize)> {
    let mut at = 0;
    loop {
        let rest = &seen[at..];
        let end = rest.windows(4).position(|w| w == b"\r\n\r\n").map(|e| e + 4).or_else(|| rest.windows(2).position(|w| w == b"\n\n").map(|e| e + 2))?;
        let block = String::from_utf8_lossy(&rest[..end]).to_ascii_lowercase();
        let first = block.lines().next().unwrap_or("");
        let status: u16 = first.split_whitespace().nth(1)?.parse().ok()?;
        at += end;
        // Another answer follows: one that is not the last.
        let more = (100..200).contains(&status) || first.contains("connection established") || ((300..400).contains(&status) && block.lines().any(|l| l.starts_with("location:")));
        if !more {
            return Some((status, at));
        }
    }
}

/// Streams `address` from byte `from` on into `body` (which may stop it):
/// the status, or None when no answer came or it broke off.
fn stream(address: &str, from: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16> {
    use std::io::Read;
    if !allowed(address) {
        return None;
    }
    let mut child = Command::new("curl").args(stream_args(address, from)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut head: Vec<u8> = Vec::new();
    let mut status = None;
    loop {
        let n = match out.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let piece: &[u8] = match status {
            Some(_) => &buf[..n],
            None => {
                head.extend_from_slice(&buf[..n]);
                if head.len() > 64 * 1024 {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                let Some((s, at)) = headers_end(&head) else { continue };
                status = Some(s);
                head.drain(..at);
                &head
            }
        };
        if !piece.is_empty() && !body(piece) {
            let _ = child.kill();
            let _ = child.wait();
            return status;
        }
        head.clear();
    }
    let ended = child.wait().ok()?;
    if !ended.success() {
        return None;
    }
    status
}

impl Web for LinuxWeb {
    fn get(&self, address: &str, most: usize) -> Option<Vec<u8>> {
        let (status, body) = request("GET", address, &[], None, most)?;
        (200..300).contains(&status).then_some(body)
    }

    fn send(&self, method: &str, address: &str, headers: &[(&str, &str)], body: &[u8], most: usize) -> Option<(u16, Vec<u8>)> {
        request(method, address, headers, (!body.is_empty() || method != "GET").then_some(body), most)
    }

    fn get_from(&self, address: &str, from: u64, body: &mut dyn FnMut(&[u8]) -> bool) -> Option<u16> {
        stream(address, from, body)
    }
}

/// A big file whole (an update): at most `most` bytes, on a 2xx only.
pub fn get_big(address: &str, most: usize) -> Option<Vec<u8>> {
    let mut got = Vec::new();
    let mut over = false;
    let status = stream(address, 0, &mut |piece| {
        over = got.len() + piece.len() > most;
        if !over {
            got.extend_from_slice(piece);
        }
        !over
    })?;
    ((200..300).contains(&status) && !over).then_some(got)
}

/// A web server on this machine for tests: answers GETs for the paths it
/// was given (a `Range: bytes=N-` with a 206), 404 for any other, and a
/// path given as `-> /other` with a redirect there.
#[cfg(test)]
pub mod loopback {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    pub type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;

    /// Serves `files` (shared: a test may change them while it runs) on a
    /// port of 127.0.0.1; its address, `http://127.0.0.1:<port>`.
    pub fn serve(files: Files) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let files = files.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    let _ = reader.read_line(&mut line);
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let mut from = 0usize;
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                        if let Some(r) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            from = r.trim().trim_end_matches('-').parse().unwrap_or(0);
                        }
                    }
                    let body = files.lock().unwrap().get(&path).cloned();
                    let mut out = stream;
                    let _ = match body {
                        Some(b) if b.starts_with(b"-> ") => write!(out, "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", String::from_utf8_lossy(&b[3..])),
                        Some(b) if from > 0 && from < b.len() => {
                            let _ = write!(out, "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{}\r\nConnection: close\r\n\r\n", b.len() - from, b.len() - 1, b.len());
                            out.write_all(&b[from..])
                        }
                        Some(b) => {
                            let _ = write!(out, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len());
                            out.write_all(&b)
                        }
                        None => write!(out, "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found"),
                    };
                });
            }
        });
        format!("http://127.0.0.1:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_big_file_streams_after_its_headers_through_redirects_and_from_a_byte_on() {
        assert_eq!(headers_end(b"HTTP/1.1 302 Found\r\nLocation: /b\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi"), Some((200, 74)));
        assert_eq!(headers_end(b"HTTP/1.1 100 Continue\r\n\r\nHTTP/2 206\r\n\r\n"), Some((206, 39)));
        assert_eq!(headers_end(b"HTTP/1.1 200 Connection established\r\n\r\nHTTP/1.1 404 Not Found\r\n\r\nx"), Some((404, 65)));
        assert_eq!(headers_end(b"HTTP/1.1 200 OK\r\nContent-Le"), None, "not all in yet");
        let a = stream_args("https://kolada.app/m.gguf", 100);
        assert!(a.windows(2).any(|w| w == ["--range", "100-"]));
        assert_eq!(&a[a.len() - 2..], ["--", "https://kolada.app/m.gguf"]);

        let body: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
        let files = loopback::Files::default();
        files.lock().unwrap().insert("/big".into(), body.clone());
        files.lock().unwrap().insert("/moved".into(), b"-> /big".to_vec());
        let site = loopback::serve(files);
        let mut got = Vec::new();
        assert_eq!(LinuxWeb.get_from(&format!("{site}/moved"), 0, &mut |p| { got.extend_from_slice(p); true }), Some(200));
        assert!(got == body, "every byte, in order, after a redirect: {} of {}", got.len(), body.len());
        got.clear();
        assert_eq!(LinuxWeb.get_from(&format!("{site}/big"), 500_000, &mut |p| { got.extend_from_slice(p); true }), Some(206));
        assert!(got == body[500_000..], "the rest from where it stopped");
        let mut pieces = 0;
        assert_eq!(LinuxWeb.get_from(&format!("{site}/big"), 0, &mut |_| { pieces += 1; false }), Some(200), "stopped at the first piece");
        assert_eq!(pieces, 1);
        assert_eq!(LinuxWeb.get_from(&format!("{site}/none"), 0, &mut |_| true), Some(404), "an error page says so");
        assert_eq!(get_big(&format!("{site}/big"), body.len()).map(|b| b.len()), Some(body.len()));
        assert_eq!(get_big(&format!("{site}/big"), body.len() - 1), None, "more than it may weigh");
        assert_eq!(get_big(&format!("{site}/none"), 100), None);
        assert_eq!(LinuxWeb.get_from("http://127.0.0.1:1/x", 0, &mut |_| true), None, "no answer");
    }

    #[test]
    fn only_web_addresses_go_out_and_the_address_comes_after_the_options() {
        assert!(allowed("https://notsy.co/x") && allowed("HTTP://example.com"));
        for bad in ["file:///etc/passwd", "ftp://x", "-K/etc/passwd", "https://x\n-o/tmp/y"] {
            assert!(!allowed(bad), "{bad}");
        }
        assert_eq!(LinuxWeb.get("file:///etc/passwd", 100), None, "refused before curl runs");
        let a = curl_args("POST", "https://notsy.co/s", &[("Content-Type", "text/plain")], 1000, true);
        let tail = &a[a.len() - 2..];
        assert_eq!(tail, ["--", "https://notsy.co/s"], "{a:?}");
        assert!(a.windows(2).any(|w| w == ["--header", "Content-Type: text/plain"]));
        assert!(a.windows(2).any(|w| w == ["--data-binary", "@-"]), "the body goes through stdin");
        assert!(a.windows(2).any(|w| w == ["--proto", "=https,http"]));
        assert_eq!(answer(b"{\"a\":\n1}\n201".to_vec()), Some((201, b"{\"a\":\n1}".to_vec())), "the body keeps its own lines");
        assert_eq!(answer(b"\n000".to_vec()), Some((0, Vec::new())), "no answer at all");
    }
}
