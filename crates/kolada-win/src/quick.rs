//! Quick Send's network side on Windows (docs/specs/2026-10-26-sync-quick-send.md):
//! the shared engine (`kolada_sync::Engine`, which does no I/O of its own),
//! a thread that owns its sockets, and the device's keys, kept with DPAPI
//! so only this Windows account can read them. Started after the first
//! frame; until this device pairs, it opens no connection at all.
//!
//! The UI thread calls the engine through `with` (locked, then the thread
//! is woken to send what it queued); what the engine says comes back as
//! `WM_APP_SYNC`, and `take_events` hands it over.

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kolada_sync::{Channel, Engine, Event};
use windows::Win32::Foundation::{HLOCAL, LPARAM, LocalFree, WPARAM};
use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::ws::Ws;

/// Posted when the engine has news (see `take_events`).
pub const WM_APP_SYNC: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 26;

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

enum Msg {
    Kick,
    Frame(Channel, u64, Vec<u8>),
    Closed(Channel, u64),
    /// A picture's upload ended with this status (0: no answer).
    Put(kolada_sync::ItemId, u16),
    /// A picture's sealed bytes came down (None: they could not).
    Got(kolada_sync::ItemId, Option<Vec<u8>>),
}

/// A socket quiet this long is pinged; one silent this long after is lost.
const PING_MS: u64 = 30_000;
const DEAD_MS: u64 = 75_000;

#[derive(Debug, PartialEq)]
enum Beat {
    Quiet,
    Ping,
    Dead,
}

/// What a socket last heard from at `heard`, last pinged at `pinged`, needs
/// at `now`.
fn beat(heard: u64, pinged: u64, now: u64) -> Beat {
    let silent = now.saturating_sub(heard);
    if silent >= DEAD_MS {
        Beat::Dead
    } else if silent >= PING_MS && now.saturating_sub(pinged) >= PING_MS {
        Beat::Ping
    } else {
        Beat::Quiet
    }
}

/// The most a picture's sealed bytes may be, coming down.
const BLOB_MOST: usize = 26 * 1024 * 1024;

pub struct Net {
    engine: Arc<Mutex<Engine>>,
    kick: Sender<Msg>,
    events: Arc<Mutex<Vec<Event>>>,
}

impl Net {
    /// Opens the engine from the keys kept in `data` and starts its thread.
    /// `base` is the server; `name` this device's, as the others see it.
    pub fn start(hwnd: crate::Handoff<windows::Win32::Foundation::HWND>, data: PathBuf, base: &str, name: &str) -> Net {
        let keys_path = data.join("sync.keys");
        let keys = std::fs::read(&keys_path).ok().and_then(|b| unprotect(&b));
        let engine = Arc::new(Mutex::new(Engine::open(base, "desktop", name, Some(data.join("sync")), keys.as_deref())));
        let events = Arc::new(Mutex::new(Vec::new()));
        let (kick, rx) = mpsc::channel();
        let (e, ev, tx) = (engine.clone(), events.clone(), kick.clone());
        std::thread::Builder::new()
            .name("quick-send".into())
            .spawn(move || run(hwnd, e, ev, tx, rx, keys_path))
            .ok();
        Net { engine, kick, events }
    }

    /// The engine, shared with note sync's loop.
    pub fn engine(&self) -> Arc<Mutex<Engine>> {
        self.engine.clone()
    }

    /// Wakes the thread to send what the engine queued, from any thread.
    pub fn kicker(&self) -> Arc<dyn Fn() + Send + Sync> {
        let kick = Mutex::new(self.kick.clone());
        Arc::new(move || {
            if let Ok(k) = kick.lock() {
                let _ = k.send(Msg::Kick);
            }
        })
    }

    /// Calls the engine, then wakes the thread to send what it queued.
    pub fn with<R>(&self, f: impl FnOnce(&mut Engine, u64) -> R) -> R {
        let r = {
            let mut e = self.engine.lock().unwrap_or_else(|p| p.into_inner());
            f(&mut e, now())
        };
        let _ = self.kick.send(Msg::Kick);
        r
    }

    /// Reads the engine without waking anything.
    pub fn read<R>(&self, f: impl FnOnce(&Engine) -> R) -> R {
        f(&self.engine.lock().unwrap_or_else(|p| p.into_inner()))
    }

    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

/// The thread: opens what the engine asks for, feeds it what arrives, sends
/// what it queues, and sleeps until the next frame or the engine's next wake.
fn run(hwnd: crate::Handoff<windows::Win32::Foundation::HWND>, engine: Arc<Mutex<Engine>>, events: Arc<Mutex<Vec<Event>>>, tx: Sender<Msg>, rx: mpsc::Receiver<Msg>, keys_path: PathBuf) {
    let hwnd = hwnd;
    let mut socks: [Option<(Arc<Ws>, u64)>; 2] = [None, None];
    // When each socket was last heard from, and last pinged: one silent
    // past `DEAD_MS` is closed, and the engine opens another.
    let mut heard = [0u64; 2];
    let mut pinged = [0u64; 2];
    let mut generation = 0u64;
    let slot = |c: Channel| if c == Channel::Group { 0 } else { 1 };
    loop {
        let connects = engine.lock().unwrap_or_else(|p| p.into_inner()).connects(now());
        for c in connects {
            match Ws::connect(&c.url, &c.headers) {
                Ok(ws) => {
                    generation += 1;
                    let ws = Arc::new(ws);
                    socks[slot(c.channel)] = Some((ws.clone(), generation));
                    heard[slot(c.channel)] = now();
                    engine.lock().unwrap_or_else(|p| p.into_inner()).opened(c.channel, now());
                    let (tx, ch, g) = (tx.clone(), c.channel, generation);
                    std::thread::spawn(move || {
                        while let Some(bytes) = ws.receive() {
                            if tx.send(Msg::Frame(ch, g, bytes)).is_err() {
                                return;
                            }
                        }
                        let _ = tx.send(Msg::Closed(ch, g));
                    });
                }
                Err(r) => {
                    let mut e = engine.lock().unwrap_or_else(|p| p.into_inner());
                    if r.status == 0 { e.closed(c.channel, now()) } else { e.failed(c.channel, r.status, &r.body, now()) }
                }
            }
        }

        let (out, wake, pairing, paired, news, transfers) = {
            let mut e = engine.lock().unwrap_or_else(|p| p.into_inner());
            let out = e.outgoing();
            (out, e.next_wake(), e.pairing(), e.paired(), e.events(), e.transfers())
        };
        // A picture's bytes, up or down, each on a thread of its own: the
        // connection goes on meanwhile.
        for t in transfers {
            let tx = tx.clone();
            std::thread::spawn(move || match t {
                kolada_sync::Transfer::Put { blob, url, body } => {
                    let status = crate::http::send("PUT", &url, &[("content-type", "application/octet-stream")], &body, 4096).map_or(0, |(s, _)| s);
                    let _ = tx.send(Msg::Put(blob, status));
                }
                kolada_sync::Transfer::Get { blob, url } => {
                    let got = crate::http::send("GET", &url, &[], &[], BLOB_MOST).filter(|(s, _)| *s == 200).map(|(_, b)| b);
                    let _ = tx.send(Msg::Got(blob, got));
                }
            });
        }
        for (ch, bytes) in out {
            if let Some((ws, _)) = &socks[slot(ch)]
                && !ws.send(&bytes)
            {
                ws.close();
            }
        }
        // A pairing over (or cancelled), or this device unpaired: its socket goes.
        if !pairing && let Some((ws, _)) = &socks[1] {
            ws.close();
        }
        if !paired && let Some((ws, _)) = &socks[0] {
            ws.close();
        }
        if !news.is_empty() {
            let mut told = Vec::new();
            for ev in news {
                match ev {
                    Event::Keys(blob) => {
                        if let Some(sealed) = protect(&blob) {
                            write_atomic(&keys_path, &sealed);
                        }
                    }
                    other => told.push(other),
                }
            }
            if !told.is_empty() {
                events.lock().unwrap_or_else(|p| p.into_inner()).extend(told);
                unsafe {
                    let _ = PostMessageW(Some(hwnd.0), WM_APP_SYNC, WPARAM(0), LPARAM(0));
                }
            }
        }

        // The heartbeat: a socket quiet for a while is pinged; one that stays
        // silent was lost without a word (a server restarted, a laptop slept,
        // the network changed) and is closed.
        let t = now();
        for (k, s) in socks.iter().enumerate() {
            let Some((ws, _)) = s else { continue };
            if beat(heard[k], pinged[k], t) == Beat::Ping && ws.ping() {
                pinged[k] = t;
            } else if beat(heard[k], pinged[k], t) == Beat::Dead {
                ws.close();
            }
        }
        let beat_due = socks.iter().any(Option::is_some).then(|| Duration::from_millis(PING_MS / 2));
        let wait = wake.map_or(Duration::from_secs(60), |t| Duration::from_millis(t.saturating_sub(now()).clamp(20, 60_000)));
        let wait = beat_due.map_or(wait, |b| wait.min(b));
        match rx.recv_timeout(wait) {
            Ok(Msg::Frame(ch, g, bytes)) => {
                if socks[slot(ch)].as_ref().is_some_and(|(_, sg)| *sg == g) {
                    heard[slot(ch)] = now();
                    if bytes != b"pong" {
                        engine.lock().unwrap_or_else(|p| p.into_inner()).frame(ch, &bytes, now());
                    }
                }
            }
            Ok(Msg::Closed(ch, g)) => {
                if socks[slot(ch)].as_ref().is_some_and(|(_, sg)| *sg == g) {
                    socks[slot(ch)] = None;
                    engine.lock().unwrap_or_else(|p| p.into_inner()).closed(ch, now());
                }
            }
            Ok(Msg::Put(blob, status)) => engine.lock().unwrap_or_else(|p| p.into_inner()).put_done(blob, status, now()),
            Ok(Msg::Got(blob, sealed)) => engine.lock().unwrap_or_else(|p| p.into_inner()).got(blob, sealed),
            Ok(Msg::Kick) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        engine.lock().unwrap_or_else(|p| p.into_inner()).tick(now());
    }
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Seals `data` for this Windows account (DPAPI).
pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(&input, windows::core::w!("Kolada Beam"), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        Some(v)
    }
}

pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A socket that goes quiet is pinged every 30 s; one silent for 75 s
    /// was lost without a word and is closed, to be opened again. Anything
    /// heard, the pong included, keeps it.
    #[test]
    fn a_silent_socket_is_pinged_then_given_up() {
        assert_eq!(beat(0, 0, 10_000), Beat::Quiet);
        assert_eq!(beat(0, 0, 30_000), Beat::Ping);
        assert_eq!(beat(0, 30_000, 40_000), Beat::Quiet, "pinged just now");
        assert_eq!(beat(0, 30_000, 60_000), Beat::Ping, "again");
        assert_eq!(beat(0, 60_000, 75_000), Beat::Dead);
        assert_eq!(beat(70_000, 60_000, 75_000), Beat::Quiet, "a pong came");
    }

    /// Two devices' data folders paired through a local server over winhttp
    /// (`ws`), as two test instances then find them, with one send between
    /// them. KOLADA_SYNC_URL is the server (wrangler dev); KOLADA_PAIR_A and
    /// KOLADA_PAIR_B the folders (a test instance's `Kolada.test`).
    #[test]
    #[ignore = "needs a local server (services/kolada-sync, wrangler dev)"]
    fn pairs_two_data_folders_through_a_local_server() {
        let base = std::env::var("KOLADA_SYNC_URL").expect("KOLADA_SYNC_URL");
        let dirs = [std::env::var("KOLADA_PAIR_A").expect("KOLADA_PAIR_A"), std::env::var("KOLADA_PAIR_B").expect("KOLADA_PAIR_B")];
        let names = ["Desk PC", "Laptop"];
        let mut engines: Vec<Engine> = (0..2).map(|i| Engine::open(&base, "desktop", names[i], Some(PathBuf::from(&dirs[i]).join("sync")), None)).collect();
        let mut socks: Vec<[Option<Arc<Ws>>; 2]> = vec![[None, None], [None, None]];
        let mut events: Vec<Vec<Event>> = vec![Vec::new(), Vec::new()];
        let (tx, rx) = mpsc::channel::<(usize, usize, Option<Vec<u8>>)>();
        let ch = |k: usize| if k == 0 { Channel::Group } else { Channel::Pair };
        let mut until = |what: &str, engines: &mut Vec<Engine>, events: &mut Vec<Vec<Event>>, done: &dyn Fn(&[Engine], &[Vec<Event>]) -> bool| {
            let start = std::time::Instant::now();
            while !done(engines, events) {
                assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
                for i in 0..2 {
                    for c in engines[i].connects(now()) {
                        let k = if c.channel == Channel::Group { 0 } else { 1 };
                        match Ws::connect(&c.url, &c.headers) {
                            Ok(ws) => {
                                let ws = Arc::new(ws);
                                socks[i][k] = Some(ws.clone());
                                engines[i].opened(c.channel, now());
                                let tx = tx.clone();
                                std::thread::spawn(move || {
                                    while let Some(b) = ws.receive() {
                                        let _ = tx.send((i, k, Some(b)));
                                    }
                                    let _ = tx.send((i, k, None));
                                });
                            }
                            Err(r) if r.status == 0 => engines[i].closed(c.channel, now()),
                            Err(r) => engines[i].failed(c.channel, r.status, &r.body, now()),
                        }
                    }
                    for (c, bytes) in engines[i].outgoing() {
                        if let Some(ws) = &socks[i][if c == Channel::Group { 0 } else { 1 }] {
                            ws.send(&bytes);
                        }
                    }
                    events[i].extend(engines[i].events());
                }
                while let Ok((i, k, m)) = rx.recv_timeout(Duration::from_millis(100)) {
                    match m {
                        Some(b) => engines[i].frame(ch(k), &b, now()),
                        None => {
                            socks[i][k] = None;
                            engines[i].closed(ch(k), now());
                        }
                    }
                }
            }
        };
        engines[0].show_code();
        until("a code", &mut engines, &mut events, &|_, ev| ev[0].iter().any(|e| matches!(e, Event::Code { .. })));
        let code = events[0].iter().find_map(|e| if let Event::Code { code, .. } = e { Some(code.clone()) } else { None }).unwrap();
        engines[1].enter_code(&code).unwrap();
        until("the ask", &mut engines, &mut events, &|_, ev| ev[0].iter().any(|e| matches!(e, Event::Asks { .. })));
        engines[0].allow(true, now());
        until("pairing", &mut engines, &mut events, &|en, _| en[1].paired() && en[0].devices().iter().any(|d| d.name == "Laptop") && en[1].devices().iter().any(|d| d.name == "Desk PC"));
        let id = engines[1].send(kolada_sync::Outgoing::Text("Hello from the laptop".into()), now()).unwrap();
        until("the send", &mut engines, &mut events, &|_, ev| ev[0].contains(&Event::Arrived(id)));
        for i in 0..2 {
            std::fs::create_dir_all(&dirs[i]).unwrap();
            std::fs::write(PathBuf::from(&dirs[i]).join("sync.keys"), protect(&engines[i].keys_blob()).unwrap()).unwrap();
        }
    }

    /// Throwaway devices on a server, through this PC's own connections
    /// (winhttp: TLS against the real one): sockets, uploads and downloads
    /// as `run` does them.
    struct Live {
        engines: Vec<Mutex<Engine>>,
        socks: Vec<[Option<Arc<Ws>>; 2]>,
        events: Vec<Vec<Event>>,
        tx: mpsc::Sender<(usize, usize, Option<Vec<u8>>)>,
        rx: mpsc::Receiver<(usize, usize, Option<Vec<u8>>)>,
    }

    impl Live {
        fn new(engines: Vec<Engine>) -> Live {
            let n = engines.len();
            let (tx, rx) = mpsc::channel();
            Live { engines: engines.into_iter().map(Mutex::new).collect(), socks: vec![[None, None]; n], events: vec![Vec::new(); n], tx, rx }
        }

        fn e(&self, i: usize) -> std::sync::MutexGuard<'_, Engine> {
            self.engines[i].lock().unwrap()
        }

        /// One turn for every device: connect, send, transfer, read.
        fn pump(&mut self) {
            let ch = |k: usize| if k == 0 { Channel::Group } else { Channel::Pair };
            for i in 0..self.engines.len() {
                let connects = self.e(i).connects(now());
                for c in connects {
                    let k = if c.channel == Channel::Group { 0 } else { 1 };
                    match Ws::connect(&c.url, &c.headers) {
                        Ok(ws) => {
                            let ws = Arc::new(ws);
                            self.socks[i][k] = Some(ws.clone());
                            self.e(i).opened(c.channel, now());
                            let tx = self.tx.clone();
                            std::thread::spawn(move || {
                                while let Some(b) = ws.receive() {
                                    let _ = tx.send((i, k, Some(b)));
                                }
                                let _ = tx.send((i, k, None));
                            });
                        }
                        Err(r) if r.status == 0 => self.e(i).closed(c.channel, now()),
                        Err(r) => self.e(i).failed(c.channel, r.status, &r.body, now()),
                    }
                }
                let out = self.e(i).outgoing();
                for (c, bytes) in out {
                    if let Some(ws) = &self.socks[i][if c == Channel::Group { 0 } else { 1 }] {
                        ws.send(&bytes);
                    }
                }
                let transfers = self.e(i).transfers();
                for t in transfers {
                    match t {
                        kolada_sync::Transfer::Put { blob, url, body } => {
                            let status = crate::http::send("PUT", &url, &[("content-type", "application/octet-stream")], &body, 4096).map_or(0, |(s, _)| s);
                            self.e(i).put_done(blob, status, now());
                        }
                        kolada_sync::Transfer::Get { blob, url } => {
                            let got = crate::http::send("GET", &url, &[], &[], BLOB_MOST).filter(|(s, _)| *s == 200).map(|(_, b)| b);
                            self.e(i).got(blob, got);
                        }
                    }
                }
                self.e(i).tick(now());
                let ev = self.e(i).events();
                self.events[i].extend(ev);
            }
            while let Ok((i, k, m)) = self.rx.recv_timeout(Duration::from_millis(50)) {
                match m {
                    Some(b) => self.e(i).frame(ch(k), &b, now()),
                    None => {
                        self.socks[i][k] = None;
                        self.e(i).closed(ch(k), now());
                    }
                }
            }
        }

        fn until(&mut self, what: &str, mut done: impl FnMut(&mut Live) -> bool) {
            let start = std::time::Instant::now();
            while !done(self) {
                assert!(start.elapsed() < Duration::from_secs(45), "timed out waiting for {what}");
                self.pump();
            }
        }

        fn pair(&mut self, names: [&str; 2]) {
            self.e(0).show_code();
            self.until("a code", |l| l.events[0].iter().any(|e| matches!(e, Event::Code { .. })));
            let code = self.events[0].iter().find_map(|e| if let Event::Code { code, .. } = e { Some(code.clone()) } else { None }).unwrap();
            self.e(1).enter_code(&code).unwrap();
            self.until("the ask", |l| l.events[0].iter().any(|e| matches!(e, Event::Asks { .. })));
            self.e(0).allow(true, now());
            let [a, b] = names;
            self.until("pairing", |l| l.e(1).paired() && l.e(0).devices().iter().any(|d| d.name == b) && l.e(1).devices().iter().any(|d| d.name == a));
        }
    }

    /// A POST as a share extension makes it (`kolada_sync::Post`): status
    /// and body.
    fn post(p: &kolada_sync::Post) -> (u16, Vec<u8>) {
        let headers: Vec<(&str, &str)> = p.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        crate::http::send("POST", &p.url, &headers, &p.body, 4096).unwrap_or((0, Vec::new()))
    }

    /// Everything that crosses the server, over TLS through this PC's own
    /// connections, between throwaway devices (KOLADA_SYNC_URL, e.g. the
    /// real https://sync.kolada.app): a picture up and down whole; a share
    /// extension's picture by plain requests; two vaults meeting and their
    /// notes syncing, an edit after. Only counts and sizes are printed; the
    /// throwaway pair is unpaired at the end.
    #[test]
    #[ignore = "needs a server: KOLADA_SYNC_URL"]
    fn pictures_and_notes_cross_a_server_over_tls() {
        let base = std::env::var("KOLADA_SYNC_URL").expect("KOLADA_SYNC_URL");
        let names = ["TLS test PC", "TLS test phone"];
        // A desk with note sync on, and a phone that follows it.
        let (mut desk, mut phone) = (Engine::open(&base, "desktop", names[0], None, None), Engine::open(&base, "ios", names[1], None, None));
        desk.set_sync(kolada_sync::SyncCap::On);
        phone.set_sync(kolada_sync::SyncCap::Follow);
        let mut live = Live::new(vec![desk, phone]);
        live.pair(names);
        eprintln!("paired over {}", if base.starts_with("https://") { "TLS" } else { "plain HTTP" });

        // A picture, up to R2 and down whole.
        let picture: Vec<u8> = (0..180_000u32).map(|i| (i * 13 % 251) as u8).collect();
        let what = kolada_sync::Outgoing::Picture { bytes: picture.clone(), mime: "image/png".into(), name: "TLS test.png".into(), width: 800, height: 600, thumb: vec![1, 2, 3] };
        let pic = live.e(0).send(what, now()).unwrap();
        live.until("the picture", |l| l.events[1].iter().any(|e| matches!(e, Event::PictureReady(id) | Event::FetchFailed(id) if *id == pic)));
        assert_eq!(live.e(1).picture(pic).map(|b| b.len()), Some(picture.len()));
        assert_eq!(live.e(1).picture(pic), Some(picture.clone()), "whole");
        eprintln!("a picture of {} bytes crossed whole", picture.len());

        // The share extension: the phone's keys, no socket, three requests.
        let mut ext = Engine::open(&base, "ios", names[1], None, Some(&live.e(1).keys_blob()));
        let shared: Vec<u8> = (0..90_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let what = kolada_sync::Outgoing::Picture { bytes: shared.clone(), mime: "image/png".into(), name: "Shared.png".into(), width: 400, height: 300, thumb: vec![4, 5] };
        let (card, ask) = ext.post_picture(what, now()).unwrap();
        let (status, answer) = post(&ask);
        assert_eq!(status, 200, "the ticket");
        ext.post_answer(&answer);
        let Some(kolada_sync::Transfer::Put { blob, url, body }) = ext.transfers().pop() else { panic!("an upload") };
        let status = crate::http::send("PUT", &url, &[("content-type", "application/octet-stream")], &body, 4096).map_or(0, |(s, _)| s);
        let item = ext.put_posted(blob, status, now()).expect("uploaded");
        assert_eq!(post(&item).0, 200, "the item");
        live.until("the shared picture", |l| l.events[0].iter().any(|e| matches!(e, Event::PictureReady(id) | Event::FetchFailed(id) if *id == card)));
        assert_eq!(live.e(0).picture(card), Some(shared), "whole");
        eprintln!("the share extension's picture crossed whole");

        // Note sync: two vaults meet, then what each lacks crosses; an edit after.
        let dir = std::env::temp_dir().join(format!("kolada-tls-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let roots = [dir.join("a/vault"), dir.join("b/vault")];
        for r in &roots {
            std::fs::create_dir_all(r).unwrap();
        }
        std::fs::write(roots[0].join("TLS test plan.md"), "# Plan\n- one\n").unwrap();
        std::fs::write(roots[0].join("TLS test same.md"), "alike").unwrap();
        std::fs::write(roots[1].join("TLS test same.md"), "alike").unwrap();
        std::fs::write(roots[1].join("TLS test phone.md"), "from the phone").unwrap();
        let mut works = [0, 1].map(|i| kolada_sync::syncer::Work::new(kolada_sync::vault::Vault::open(&roots[i], &dir.join(["a", "b"][i]).join("data"), true)));
        let sync = |live: &mut Live, works: &mut [kolada_sync::syncer::Work; 2]| {
            for i in 0..2 {
                for e in std::mem::take(&mut live.events[i]) {
                    if let Event::Synced { id, from, name, batch } = e {
                        works[i].on(kolada_sync::syncer::Msg::Batch { id, from, name, batch }, &live.engines[i], now());
                    }
                }
                works[i].tick(&live.engines[i], now());
            }
            live.pump();
        };
        let start = std::time::Instant::now();
        while !(roots[1].join("TLS test plan.md").exists() && roots[0].join("TLS test phone.md").exists()) {
            assert!(start.elapsed() < Duration::from_secs(60), "timed out waiting for the vaults to meet and sync");
            sync(&mut live, &mut works);
        }
        assert!(works[0].vault().settled() && works[1].vault().settled());
        eprintln!("two vaults met and synced in {} ms", start.elapsed().as_millis());
        std::fs::write(roots[1].join("TLS test plan.md"), "# Plan\n- one\n- two\n").unwrap();
        works[1].on(kolada_sync::syncer::Msg::Changed("TLS test plan.md".into()), &live.engines[1], now());
        let start = std::time::Instant::now();
        while std::fs::read_to_string(roots[0].join("TLS test plan.md")).unwrap_or_default().lines().count() < 3 {
            assert!(start.elapsed() < Duration::from_secs(60), "timed out waiting for the edit");
            sync(&mut live, &mut works);
        }
        eprintln!("an edit crossed in {} ms", start.elapsed().as_millis());

        // Unpaired: the phone is removed from the throwaway group.
        let phone = live.e(1).device_id();
        live.e(0).remove(phone);
        live.until("the phone removed", |l| l.events[1].contains(&Event::Removed));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_are_sealed_for_this_account_and_open_again() {
        let sealed = protect(b"the keys").unwrap();
        assert!(!sealed.windows(8).any(|w| w == b"the keys"));
        assert_eq!(unprotect(&sealed).unwrap(), b"the keys");
        assert!(unprotect(b"not sealed").is_none());
    }
}
