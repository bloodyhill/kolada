//! Fetching a picture from the web: a pasted page's images come after the
//! paste, on a thread of their own (see `model/arrive.rs`). Foundation's own
//! download, which follows redirects and speaks the system's proxies and
//! certificates; it blocks the thread it is called on, never the window's.

use objc2_foundation::{NSData, NSString, NSURL};

/// Foundation's download, for the model's `Web` service.
pub struct MacWeb;

impl notsy_app::services::Web for MacWeb {
    fn get(&self, address: &str, most: usize) -> Option<Vec<u8>> {
        get(address, most)
    }
}

/// The bytes at `address` (http or https only), if they come and are no more
/// than `most`.
pub fn get(address: &str, most: usize) -> Option<Vec<u8>> {
    let lower = address.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return None;
    }
    let url = NSURL::URLWithString(&NSString::from_str(address))?;
    let data = NSData::dataWithContentsOfURL(&url)?;
    (data.len() <= most).then(|| data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A server on this machine that answers one request with `body`.
    fn serve(body: &'static [u8]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
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
}
