//! Quick Send's live connection on the Mac (`kolada_app::services::Socket`):
//! Foundation's WebSocket task, so the system's proxies and TLS apply. It is
//! driven from Quick Send's own thread: each call waits for the task's
//! completion handler. The task does not ping by itself, so a ping goes
//! every 30 s while the socket is open (a hibernating server answers pings
//! without waking). A refused handshake shows its status and headers, never
//! its body: the server repeats its fields in `x-kolada`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURLSession, NSURLSessionTaskState, NSURLSessionWebSocketCloseCode, NSURLSessionWebSocketMessage,
    NSURLSessionWebSocketTask, NSURL,
};

use kolada_app::services::{Link, Socket};

/// The most one message may be: the largest sealed item and its frame.
const MOST: isize = 300 * 1024;
/// How long the handshake may take.
const OPEN: Duration = Duration::from_secs(15);
/// How long a send may take.
const SEND: Duration = Duration::from_secs(20);
const PING: Duration = Duration::from_secs(30);

pub struct MacSocket;

/// The task, which Foundation lets any thread use.
struct Task(Retained<NSURLSessionWebSocketTask>);

unsafe impl Send for Task {}
unsafe impl Sync for Task {}

struct MacLink {
    task: Arc<Task>,
    open: Arc<AtomicBool>,
}

/// The refusal's fields from `x-kolada` ("error=rest; retry=3600"), one per line.
fn fields(header: &str) -> String {
    header.split(';').map(str::trim).filter(|f| !f.is_empty()).collect::<Vec<_>>().join("\n")
}

/// Waits for `f`'s handler to say whether it went (no error).
fn wait(start: impl FnOnce(&RcBlock<dyn Fn(*mut NSError)>), most: Duration) -> bool {
    let (tx, rx) = mpsc::channel();
    let done = RcBlock::new(move |error: *mut NSError| {
        let _ = tx.send(error.is_null());
    });
    start(&done);
    rx.recv_timeout(most).unwrap_or(false)
}

impl Socket for MacSocket {
    fn connect(&self, url: &str, headers: &[(String, String)]) -> Result<Box<dyn Link>, (u16, String)> {
        let Some(address) = NSURL::URLWithString(&NSString::from_str(url)) else { return Err((0, String::new())) };
        let request = NSMutableURLRequest::requestWithURL(&address);
        for (k, v) in headers {
            request.setValue_forHTTPHeaderField(Some(&NSString::from_str(v)), &NSString::from_str(k));
        }
        let task = NSURLSession::sharedSession().webSocketTaskWithRequest(&request);
        task.setMaximumMessageSize(MOST);
        task.resume();
        // The server's answer to the handshake: 101 is open, anything else
        // a refusal. The task shows it once it has it.
        let task = Arc::new(Task(task));
        let start = Instant::now();
        let answer = loop {
            if let Some(r) = task.0.response().and_then(|r| r.downcast::<NSHTTPURLResponse>().ok()) {
                break Some(r);
            }
            if task.0.state() == NSURLSessionTaskState::Completed || start.elapsed() > OPEN {
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let status = answer.as_ref().map_or(0, |r| r.statusCode().clamp(0, u16::MAX as isize) as u16);
        if status != 101 {
            let body = answer.and_then(|r| r.valueForHTTPHeaderField(&NSString::from_str("x-kolada"))).map(|h| fields(&h.to_string())).unwrap_or_default();
            task.0.cancel();
            return Err((status, body));
        }
        let open = Arc::new(AtomicBool::new(true));
        let (t, o) = (task.clone(), open.clone());
        std::thread::Builder::new()
            .name("quick-send-ping".into())
            .spawn(move || {
                while o.load(Ordering::Relaxed) {
                    std::thread::sleep(PING);
                    if o.load(Ordering::Relaxed) && !ping(&t) {
                        o.store(false, Ordering::Relaxed);
                        t.0.cancel();
                    }
                }
            })
            .ok();
        Ok(Box::new(MacLink { task, open }))
    }
}

/// A ping on an open socket, answered with a pong: whether it was, in time.
/// (One sent before the handshake is through is never answered.)
fn ping(task: &Task) -> bool {
    wait(|done| unsafe { task.0.sendPingWithPongReceiveHandler(done) }, OPEN)
}

impl Link for MacLink {
    fn send(&self, bytes: &[u8]) -> bool {
        if !self.open.load(Ordering::Relaxed) {
            return false;
        }
        let message = NSURLSessionWebSocketMessage::initWithData(NSURLSessionWebSocketMessage::alloc(), &NSData::with_bytes(bytes));
        wait(|done| unsafe { self.task.0.sendMessage_completionHandler(&message, done) }, SEND)
    }

    fn receive(&self) -> Option<Vec<u8>> {
        if !self.open.load(Ordering::Relaxed) {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        let got = RcBlock::new(move |message: *mut NSURLSessionWebSocketMessage, error: *mut NSError| {
            let bytes = if error.is_null() && !message.is_null() {
                let message = unsafe { &*message };
                message.data().map(|d| d.to_vec()).or_else(|| message.string().map(|s| s.to_string().into_bytes()))
            } else {
                None
            };
            let _ = tx.send(bytes);
        });
        unsafe { self.task.0.receiveMessageWithCompletionHandler(&got) };
        // The handler always runs: with a message, or with the error that
        // closing the task gives a receive that is waiting.
        let bytes = rx.recv().ok().flatten();
        if bytes.is_none() {
            self.open.store(false, Ordering::Relaxed);
        }
        bytes
    }

    fn close(&self) {
        if self.open.swap(false, Ordering::Relaxed) {
            self.task.0.cancelWithCloseCode_reason(NSURLSessionWebSocketCloseCode::NormalClosure, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusals_header_reads_as_the_body_would() {
        assert_eq!(fields("error=rest; retry=3600"), "error=rest\nretry=3600");
        assert_eq!(fields(""), "");
    }

    /// Against a local `wrangler dev` (the sync server): a pairing
    /// slot opens, and a group the server does not know is refused with
    /// its reason from the header.
    #[test]
    #[ignore = "needs a local server: KOLADA_SYNC_URL=http://127.0.0.1:8787"]
    fn opens_a_pairing_socket_and_reads_a_refusal() {
        let base = std::env::var("KOLADA_SYNC_URL").expect("KOLADA_SYNC_URL").replacen("http", "ws", 1);
        let link = MacSocket.connect(&format!("{base}/v1/pair/open"), &[]).unwrap_or_else(|e| panic!("refused: {e:?}"));
        // SAFETY: what `connect` returned is a MacLink.
        let link: &MacLink = unsafe { &*(&*link as *const dyn Link as *const MacLink) };
        assert!(link.receive().is_some_and(|b| !b.is_empty()), "the slot frame comes");
        assert!(ping(&link.task), "a ping is answered");
        link.close();
        assert!(link.receive().is_none(), "closed");
        match MacSocket.connect(&format!("{base}/v1/g/{}/ws", "0".repeat(32)), &[]) {
            Err((status, body)) => assert!(status >= 400 && body.starts_with("error="), "refused with a status and its reason: {status} {body}"),
            Ok(_) => panic!("an unknown group opens no socket"),
        }
    }
}
