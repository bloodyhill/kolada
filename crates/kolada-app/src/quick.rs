//! Quick Send's network side for every shell on the app model
//! (docs/specs/2026-10-26-sync-quick-send.md): the shared engine
//! (`kolada_sync::Engine`, which does no I/O of its own), a thread that owns
//! its sockets (`services::Socket`), and the device's keys, kept by the OS
//! (`services::KeyStore`). Windows has the same in kolada-win/src/quick.rs
//! on winhttp and DPAPI. Started after the first frame; until this device
//! pairs, it opens no connection at all.
//!
//! The UI thread calls the engine through `with` (locked, then the thread
//! is woken to send what it queued); what the engine says comes back by the
//! shell's wake, and `take_events` hands it over.

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kolada_sync::{Channel, Engine, Event};
pub use kolada_sync::SERVER;
/// What a device is called and drawn as (shells name themselves by it).
pub use kolada_sync::names;

use crate::services::{KeyStore, Link, Socket, Web};

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

/// The most a picture's sealed bytes may be, coming down.
const BLOB_MOST: usize = 26 * 1024 * 1024;

/// A group socket quiet this long is pinged; heard from nothing this long,
/// it died without a word (a laptop's sleep, a network gone) and is closed,
/// to be opened again. Only for sockets that can ping (`Link::ping`).
const PING_MS: u64 = 30_000;
const DEAD_MS: u64 = 75_000;

#[derive(Debug, PartialEq, Eq)]
enum Beat {
    Quiet,
    Ping,
    Dead,
}

/// What a socket last heard (`heard`) and was last pinged (`pinged`) call
/// for at `now`.
fn beat(heard: u64, pinged: u64, now: u64) -> Beat {
    if now >= heard + DEAD_MS {
        Beat::Dead
    } else if now >= heard.max(pinged) + PING_MS {
        Beat::Ping
    } else {
        Beat::Quiet
    }
}

pub struct Net {
    engine: Arc<Mutex<Engine>>,
    kick: Sender<Msg>,
    events: Arc<Mutex<Vec<Event>>>,
}

impl Net {
    /// Opens the engine from the keys kept, and starts its thread. `data`
    /// holds its history; `base` is the server; `kind` this device's
    /// ("desktop"); `name` how the others see it. `wake` is called (on the
    /// thread) when the engine has news.
    #[allow(clippy::too_many_arguments)]
    pub fn start(socket: Arc<dyn Socket>, keys: Arc<dyn KeyStore>, web: Arc<dyn Web>, data: Option<PathBuf>, base: &str, kind: &str, name: &str, wake: Arc<dyn Fn() + Send + Sync>) -> Net {
        let kept = keys.load();
        Net::with_engine(Engine::open(base, kind, name, data, kept.as_deref()), socket, keys, web, wake)
    }

    /// The same around an engine already open (tests hand in a paired one).
    pub fn with_engine(engine: Engine, socket: Arc<dyn Socket>, keys: Arc<dyn KeyStore>, web: Arc<dyn Web>, wake: Arc<dyn Fn() + Send + Sync>) -> Net {
        let engine = Arc::new(Mutex::new(engine));
        let events = Arc::new(Mutex::new(Vec::new()));
        let (kick, rx) = mpsc::channel();
        let (e, ev, tx) = (engine.clone(), events.clone(), kick.clone());
        std::thread::Builder::new().name("quick-send".into()).spawn(move || run(socket, keys, web, wake, e, ev, tx, rx)).ok();
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

    /// Waits (up to 5 s) until the thread has handed news over.
    #[cfg(test)]
    pub(crate) fn wait_for_events(&self) {
        let start = std::time::Instant::now();
        while self.events.lock().unwrap_or_else(|p| p.into_inner()).is_empty() && start.elapsed() < Duration::from_secs(5) {
            let _ = self.kick.send(Msg::Kick);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// The thread: opens what the engine asks for, feeds it what arrives, sends
/// what it queues, and sleeps until the next frame or the engine's next wake.
#[allow(clippy::too_many_arguments)]
fn run(socket: Arc<dyn Socket>, keys: Arc<dyn KeyStore>, web: Arc<dyn Web>, wake: Arc<dyn Fn() + Send + Sync>, engine: Arc<Mutex<Engine>>, events: Arc<Mutex<Vec<Event>>>, tx: Sender<Msg>, rx: mpsc::Receiver<Msg>) {
    // The keys are kept on a thread of their own: the OS's store may wait
    // (a Keychain prompt), and the connection must not. The newest wins.
    let (save, saves) = mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new()
        .name("quick-send-keys".into())
        .spawn(move || {
            while let Ok(mut blob) = saves.recv() {
                while let Ok(newer) = saves.try_recv() {
                    blob = newer;
                }
                keys.save(&blob);
            }
        })
        .ok();
    let mut socks: [Option<(Arc<dyn Link>, u64)>; 2] = [None, None];
    let mut generation = 0u64;
    // The group socket's heartbeat: when it was last heard, last pinged,
    // and whether it can be.
    let (mut heard, mut pinged, mut pings) = (0u64, 0u64, false);
    let slot = |c: Channel| if c == Channel::Group { 0 } else { 1 };
    let lock = || engine.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        let connects = lock().connects(now());
        for c in connects {
            match socket.connect(&c.url, &c.headers) {
                Ok(link) => {
                    generation += 1;
                    let link: Arc<dyn Link> = Arc::from(link);
                    socks[slot(c.channel)] = Some((link.clone(), generation));
                    if c.channel == Channel::Group {
                        (heard, pinged, pings) = (now(), 0, true);
                    }
                    lock().opened(c.channel, now());
                    let (tx, ch, g) = (tx.clone(), c.channel, generation);
                    std::thread::spawn(move || {
                        while let Some(bytes) = link.receive() {
                            if tx.send(Msg::Frame(ch, g, bytes)).is_err() {
                                return;
                            }
                        }
                        let _ = tx.send(Msg::Closed(ch, g));
                    });
                }
                Err((0, _)) => lock().closed(c.channel, now()),
                Err((status, body)) => lock().failed(c.channel, status, &body, now()),
            }
        }

        let (out, next, pairing, paired, news, transfers) = {
            let mut e = lock();
            let out = e.outgoing();
            (out, e.next_wake(), e.pairing(), e.paired(), e.events(), e.transfers())
        };
        // A picture's bytes, up or down, each on a thread of its own: the
        // connection goes on meanwhile.
        for t in transfers {
            let (web, tx) = (web.clone(), tx.clone());
            std::thread::spawn(move || match t {
                kolada_sync::Transfer::Put { blob, url, body } => {
                    let status = web.send("PUT", &url, &[("content-type", "application/octet-stream")], &body, 4096).map_or(0, |(s, _)| s);
                    let _ = tx.send(Msg::Put(blob, status));
                }
                kolada_sync::Transfer::Get { blob, url } => {
                    let got = web.send("GET", &url, &[], &[], BLOB_MOST).filter(|(s, _)| *s == 200).map(|(_, b)| b);
                    let _ = tx.send(Msg::Got(blob, got));
                }
            });
        }
        for (ch, bytes) in out {
            if let Some((link, _)) = &socks[slot(ch)]
                && !link.send(&bytes)
            {
                link.close();
            }
        }
        // A pairing over (or cancelled), or this device unpaired: its socket goes.
        if !pairing && let Some((link, _)) = &socks[1] {
            link.close();
        }
        if !paired && let Some((link, _)) = &socks[0] {
            link.close();
        }
        if !news.is_empty() {
            let mut told = Vec::new();
            for ev in news {
                match ev {
                    Event::Keys(blob) => {
                        let _ = save.send(blob);
                    }
                    other => told.push(other),
                }
            }
            if !told.is_empty() {
                events.lock().unwrap_or_else(|p| p.into_inner()).extend(told);
                wake();
            }
        }

        // A quiet group socket is pinged; one that stopped answering is
        // closed, and opened again.
        if pings && let Some((link, _)) = &socks[0] {
            match beat(heard, pinged, now()) {
                Beat::Quiet => {}
                Beat::Ping => {
                    pinged = now();
                    pings = link.ping();
                }
                Beat::Dead => link.close(),
            }
        }
        let beat_at = (pings && socks[0].is_some()).then(|| heard.max(pinged) + PING_MS);
        let next = [next, beat_at].into_iter().flatten().min();
        let wait = next.map_or(Duration::from_secs(60), |t| Duration::from_millis(t.saturating_sub(now()).clamp(20, 60_000)));
        match rx.recv_timeout(wait) {
            Ok(Msg::Frame(ch, g, bytes)) => {
                if socks[slot(ch)].as_ref().is_some_and(|(_, sg)| *sg == g) {
                    if ch == Channel::Group {
                        heard = now();
                    }
                    // A pong: heard, and nothing for the engine.
                    if !bytes.is_empty() {
                        lock().frame(ch, &bytes, now());
                    }
                }
            }
            Ok(Msg::Closed(ch, g)) => {
                if socks[slot(ch)].as_ref().is_some_and(|(_, sg)| *sg == g) {
                    socks[slot(ch)] = None;
                    lock().closed(ch, now());
                }
            }
            Ok(Msg::Put(blob, status)) => lock().put_done(blob, status, now()),
            Ok(Msg::Got(blob, sealed)) => lock().got(blob, sealed),
            Ok(Msg::Kick) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        lock().tick(now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Receiver;

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

    /// A store whose save never comes back (a Keychain prompt no one answers).
    struct Stuck(Mutex<Receiver<()>>);

    impl KeyStore for Stuck {
        fn load(&self) -> Option<Vec<u8>> {
            None
        }

        fn save(&self, _: &[u8]) -> bool {
            let _ = self.0.lock().unwrap().recv();
            false
        }
    }

    /// Says which addresses were asked for, and refuses them all.
    struct Told(Mutex<Sender<String>>);

    impl Socket for Told {
        fn connect(&self, url: &str, _: &[(String, String)]) -> Result<Box<dyn Link>, (u16, String)> {
            let _ = self.0.lock().unwrap().send(url.to_string());
            Err((0, String::new()))
        }
    }

    #[test]
    fn a_key_store_that_waits_never_holds_up_the_connection() {
        let (_never, wait) = mpsc::channel();
        let (tell, told) = mpsc::channel();
        let net = Net::start(Arc::new(Told(Mutex::new(tell))), Arc::new(Stuck(Mutex::new(wait))), Arc::new(crate::services::NoServices), None, "https://sync.kolada.app", "desktop", "Mac", Arc::new(|| {}));
        // A new device makes its keys, which go to the store, and stick there.
        net.with(|e, _| e.show_code());
        let url = told.recv_timeout(Duration::from_secs(5)).expect("the pairing socket is asked for");
        assert!(url.ends_with("/v1/pair/open"));
    }
}
