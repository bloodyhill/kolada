//! Beam on Linux: what the shared model needs from the desktop for it. A
//! socket to the server (tungstenite, with rustls for TLS: nothing linked
//! from the system), the device's keys in a file only this user can read
//! (Secret Service is a later step), Alt+Z where the desktop lets an app have
//! keys of its own (X11: a grab on the root window; Wayland lets no app
//! have them, so its keys stay off), and what was copied, read for the model.
//! The popup itself is a window of `app.rs`'s, drawn by the model.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kolada_app::model::quick::Clip;
use kolada_app::services::{KeyStore, Link, Socket};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

/// How long a read waits before the socket is let go for a send.
const READ_SLICE: Duration = Duration::from_millis(50);

pub struct LinuxSocket;

impl Socket for LinuxSocket {
    fn connect(&self, url: &str, headers: &[(String, String)]) -> Result<Box<dyn Link>, (u16, String)> {
        // rustls's crypto: ring, once for the process.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut req = url.into_client_request().map_err(|_| (0, String::new()))?;
        for (k, v) in headers {
            let (Ok(name), Ok(value)) = (tungstenite::http::HeaderName::from_bytes(k.as_bytes()), v.parse()) else { continue };
            req.headers_mut().insert(name, value);
        }
        match tungstenite::connect(req) {
            Ok((ws, _)) => {
                let tcp: &TcpStream = match ws.get_ref() {
                    MaybeTlsStream::Plain(s) => s,
                    MaybeTlsStream::Rustls(s) => s.get_ref(),
                    _ => return Err((0, String::new())),
                };
                tcp.set_read_timeout(Some(READ_SLICE)).map_err(|_| (0, String::new()))?;
                Ok(Box::new(LinuxLink { ws: Mutex::new(ws), closed: AtomicBool::new(false) }))
            }
            Err(tungstenite::Error::Http(res)) => {
                let body = res.body().as_ref().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                Err((res.status().as_u16(), body))
            }
            Err(_) => Err((0, String::new())),
        }
    }
}

/// An open socket: one thread receives while another sends, each holding
/// it a moment at a time.
struct LinuxLink {
    ws: Mutex<WebSocket<MaybeTlsStream<TcpStream>>>,
    closed: AtomicBool,
}

impl Link for LinuxLink {
    fn send(&self, bytes: &[u8]) -> bool {
        !self.closed.load(Ordering::Relaxed) && self.ws.lock().is_ok_and(|mut ws| ws.send(Message::Binary(bytes.to_vec().into())).is_ok())
    }

    fn receive(&self) -> Option<Vec<u8>> {
        loop {
            if self.closed.load(Ordering::Relaxed) {
                return None;
            }
            let got = self.ws.lock().ok()?.read();
            match got {
                Ok(Message::Binary(b)) => return Some(b.to_vec()),
                // The server's answer to a ping: heard.
                Ok(Message::Text(t)) if t.as_str() == "pong" => return Some(Vec::new()),
                Ok(Message::Close(_)) => return None,
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    // Its pongs and other answers go out meanwhile.
                    let _ = self.ws.lock().ok()?.flush();
                }
                Err(_) => return None,
            }
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        if let Ok(mut ws) = self.ws.lock() {
            let _ = ws.close(None);
            let _ = ws.flush();
        }
    }

    fn ping(&self) -> bool {
        !self.closed.load(Ordering::Relaxed) && self.ws.lock().is_ok_and(|mut ws| ws.send(Message::Text("ping".into())).is_ok())
    }
}

/// The device's keys, in a file in Kolada's own data that only this user
/// can read (0600, in a 0700 folder), replaced whole.
pub struct LinuxKeys {
    file: Option<PathBuf>,
}

impl LinuxKeys {
    pub fn new(data: Option<PathBuf>) -> LinuxKeys {
        LinuxKeys { file: data.map(|d| d.join("sync").join("keys")) }
    }
}

impl KeyStore for LinuxKeys {
    fn load(&self) -> Option<Vec<u8>> {
        std::fs::read(self.file.as_ref()?).ok().filter(|b| !b.is_empty())
    }

    fn save(&self, keys: &[u8]) -> bool {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let Some(file) = &self.file else { return false };
        let Some(dir) = file.parent() else { return false };
        let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
        let tmp = file.with_extension("tmp");
        let wrote = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| f.write_all(keys).and_then(|_| f.sync_all()));
        wrote.is_ok() && std::fs::rename(&tmp, file).is_ok()
    }
}

/// What was copied, for the model: its text, and whether a password
/// manager marked it secret; or a picture (as PNG).
pub fn clip(board: &mut super::clipboard::Clipboard) -> Option<Clip> {
    match board.paste()? {
        super::clipboard::Pasted::Kolada(text) | super::clipboard::Pasted::Text { plain: text, .. } => Some(Clip { text, concealed: super::clipboard::concealed(), picture: None }),
        super::clipboard::Pasted::Picture(png) => Some(Clip { text: String::new(), concealed: false, picture: Some(("image/png".into(), png)) }),
    }
}

/// Beam's keys, Alt+Z, held while it lives: on X11, a grab on the root
/// window, on a display connection of its own, on a thread of its own.
pub struct Keys {
    stop: Arc<AtomicBool>,
}

impl Drop for Keys {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Takes Alt+Z for Beam, calling `pressed` (on its thread) each press:
/// None where the desktop lets no app have keys of its own (Wayland), or
/// another app holds them.
pub fn take_keys(pressed: impl Fn() + Send + 'static) -> Option<Keys> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("KOLADA_BEAM_X11_KEYS").is_none() {
        return None;
    }
    std::env::var_os("DISPLAY")?;
    let stop = Arc::new(AtomicBool::new(false));
    let (told, held) = std::sync::mpsc::channel();
    let s = stop.clone();
    std::thread::Builder::new().name("beam-keys".into()).spawn(move || x11::hold(&s, &told, pressed)).ok()?;
    held.recv_timeout(Duration::from_secs(2)).ok().filter(|h| *h).map(|_| Keys { stop })
}

mod x11 {
    use std::os::raw::{c_int, c_uint};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;

    use x11_dl::xlib;

    /// Set when a grab is refused (another app holds the keys).
    static REFUSED: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" fn refused(_: *mut xlib::Display, _: *mut xlib::XErrorEvent) -> c_int {
        REFUSED.store(true, Ordering::Relaxed);
        0
    }

    /// Grabs Alt+Z on the root window (with Caps Lock and Num Lock on or
    /// off), says whether it held, then calls `pressed` for each press
    /// until `stop`.
    pub fn hold(stop: &AtomicBool, told: &Sender<bool>, pressed: impl Fn()) {
        let Ok(x) = xlib::Xlib::open() else {
            let _ = told.send(false);
            return;
        };
        unsafe {
            let dpy = (x.XOpenDisplay)(std::ptr::null());
            if dpy.is_null() {
                let _ = told.send(false);
                return;
            }
            let root = (x.XDefaultRootWindow)(dpy);
            let code = (x.XKeysymToKeycode)(dpy, x11_dl::keysym::XK_z as xlib::KeySym) as c_int;
            let mods: [c_uint; 4] = [xlib::Mod1Mask, xlib::Mod1Mask | xlib::LockMask, xlib::Mod1Mask | xlib::Mod2Mask, xlib::Mod1Mask | xlib::LockMask | xlib::Mod2Mask];
            REFUSED.store(false, Ordering::Relaxed);
            let before = (x.XSetErrorHandler)(Some(refused));
            for m in mods {
                (x.XGrabKey)(dpy, code, m, root, xlib::True, xlib::GrabModeAsync, xlib::GrabModeAsync);
            }
            (x.XSync)(dpy, xlib::False);
            (x.XSetErrorHandler)(before);
            if REFUSED.load(Ordering::Relaxed) || code == 0 {
                (x.XCloseDisplay)(dpy);
                let _ = told.send(false);
                return;
            }
            let _ = told.send(true);
            let fd = (x.XConnectionNumber)(dpy);
            while !stop.load(Ordering::Relaxed) {
                let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
                libc::poll(&mut p, 1, 250);
                while (x.XPending)(dpy) > 0 {
                    let mut ev: xlib::XEvent = std::mem::zeroed();
                    (x.XNextEvent)(dpy, &mut ev);
                    if ev.get_type() == xlib::KeyPress {
                        pressed();
                    }
                }
            }
            for m in mods {
                (x.XUngrabKey)(dpy, code, m, root);
            }
            (x.XCloseDisplay)(dpy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys are kept for this user alone (0600, in a 0700 folder),
    /// replaced whole, and read back; none kept reads as none.
    #[test]
    fn the_keys_are_kept_for_this_user_alone_and_read_back() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("kolada-linux-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let k = LinuxKeys::new(Some(dir.clone()));
        assert_eq!(k.load(), None);
        assert!(k.save(b"first"));
        assert!(k.save(b"the keys"));
        assert_eq!(k.load().as_deref(), Some(&b"the keys"[..]));
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.join("sync").join("keys")), 0o600);
        assert_eq!(mode(&dir.join("sync")), 0o700);
        assert!(!LinuxKeys::new(None).save(b"x"), "no data folder, nothing kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server that refuses says why (its status and body); none at all is
    /// a status of 0, so the engine waits and tries again.
    #[test]
    fn a_refusal_says_why_and_no_server_is_status_0() {
        assert_eq!(LinuxSocket.connect("ws://127.0.0.1:9/v1/pair/open", &[]).err().map(|e| e.0), Some(0));
    }

    /// Through the real server (KOLADA_SYNC_URL, wss), over this socket:
    /// two devices pair and a beam crosses, and a ping is answered.
    #[test]
    #[ignore = "needs a server: KOLADA_SYNC_URL"]
    fn two_devices_pair_and_beam_through_a_server_over_this_socket() {
        use kolada_sync::{Channel, Engine, Event, Outgoing};
        let base = std::env::var("KOLADA_SYNC_URL").expect("KOLADA_SYNC_URL");
        let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let mut engines = [Engine::open(&base, "desktop", "Linux test PC", None, None), Engine::open(&base, "desktop", "Linux test laptop", None, None)];
        let links: std::rc::Rc<std::cell::RefCell<Vec<[Option<Arc<dyn Link>>; 2]>>> = std::rc::Rc::new(std::cell::RefCell::new(vec![[None, None], [None, None]]));
        let shared = links.clone();
        let (tx, rx) = std::sync::mpsc::channel::<(usize, usize, Option<Vec<u8>>)>();
        let mut events: Vec<Vec<Event>> = vec![Vec::new(), Vec::new()];
        let mut pongs = 0;
        let mut pump = |engines: &mut [Engine; 2], events: &mut Vec<Vec<Event>>, pongs: &mut usize| {
            for i in 0..2 {
                for c in engines[i].connects(now()) {
                    let k = if c.channel == Channel::Group { 0 } else { 1 };
                    match LinuxSocket.connect(&c.url, &c.headers) {
                        Ok(link) => {
                            let link: Arc<dyn Link> = Arc::from(link);
                            shared.borrow_mut()[i][k] = Some(link.clone());
                            engines[i].opened(c.channel, now());
                            let tx = tx.clone();
                            std::thread::spawn(move || {
                                while let Some(b) = link.receive() {
                                    let _ = tx.send((i, k, Some(b)));
                                }
                                let _ = tx.send((i, k, None));
                            });
                        }
                        Err((0, _)) => engines[i].closed(c.channel, now()),
                        Err((s, b)) => engines[i].failed(c.channel, s, &b, now()),
                    }
                }
                for (c, bytes) in engines[i].outgoing() {
                    if let Some(l) = &shared.borrow()[i][if c == Channel::Group { 0 } else { 1 }] {
                        l.send(&bytes);
                    }
                }
                engines[i].tick(now());
                events[i].extend(engines[i].events());
            }
            while let Ok((i, k, m)) = rx.recv_timeout(Duration::from_millis(50)) {
                let ch = if k == 0 { Channel::Group } else { Channel::Pair };
                match m {
                    Some(b) if b.is_empty() => *pongs += 1,
                    Some(b) => engines[i].frame(ch, &b, now()),
                    None => {
                        shared.borrow_mut()[i][k] = None;
                        engines[i].closed(ch, now());
                    }
                }
            }
        };
        let until = |what: &str, done: &mut dyn FnMut(&[Engine; 2], &Vec<Vec<Event>>) -> bool, engines: &mut [Engine; 2], events: &mut Vec<Vec<Event>>, pongs: &mut usize, pump: &mut dyn FnMut(&mut [Engine; 2], &mut Vec<Vec<Event>>, &mut usize)| {
            let start = std::time::Instant::now();
            while !done(engines, events) {
                assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
                pump(engines, events, pongs);
            }
        };
        engines[0].show_code();
        until("a code", &mut |_, ev| ev[0].iter().any(|e| matches!(e, Event::Code { .. })), &mut engines, &mut events, &mut pongs, &mut pump);
        let code = events[0].iter().find_map(|e| if let Event::Code { code, .. } = e { Some(code.clone()) } else { None }).unwrap();
        engines[1].enter_code(&code).unwrap();
        until("the ask", &mut |_, ev| ev[0].iter().any(|e| matches!(e, Event::Asks { .. })), &mut engines, &mut events, &mut pongs, &mut pump);
        engines[0].allow(true, now());
        until("pairing", &mut |en, _| en[1].paired() && en[0].devices().len() == 2 && en[1].devices().len() == 2, &mut engines, &mut events, &mut pongs, &mut pump);
        let id = engines[1].send(Outgoing::Text("From the Linux socket".into()), now()).unwrap();
        until("the beam", &mut |_, ev| ev[0].contains(&Event::Arrived(id)), &mut engines, &mut events, &mut pongs, &mut pump);
        assert_eq!(engines[0].copy(id).as_deref(), Some("From the Linux socket"));
        // A ping is answered "pong".
        let group = links.borrow()[0][0].clone().expect("the group socket");
        assert!(group.ping());
        until("the pong", &mut |_, _| true, &mut engines, &mut events, &mut pongs, &mut pump);
        let start = std::time::Instant::now();
        while pongs == 0 && start.elapsed() < Duration::from_secs(10) {
            pump(&mut engines, &mut events, &mut pongs);
        }
        assert!(pongs > 0, "the server answered the ping");
        // The throwaway laptop leaves the group.
        let laptop = engines[1].device_id();
        engines[0].remove(laptop);
        until("removed", &mut |_, ev| ev[1].contains(&Event::Removed), &mut engines, &mut events, &mut pongs, &mut pump);
    }
}
