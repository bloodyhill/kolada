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

impl Web for LinuxWeb {
    fn get(&self, address: &str, most: usize) -> Option<Vec<u8>> {
        let (status, body) = request("GET", address, &[], None, most)?;
        (200..300).contains(&status).then_some(body)
    }

    fn send(&self, method: &str, address: &str, headers: &[(&str, &str)], body: &[u8], most: usize) -> Option<(u16, Vec<u8>)> {
        request(method, address, headers, (!body.is_empty() || method != "GET").then_some(body), most)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
