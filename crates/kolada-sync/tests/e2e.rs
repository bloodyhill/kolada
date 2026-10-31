//! End to end against a local server: two engines pair with a code and send
//! to each other through the sync server under `wrangler dev`.
//!
//! Skipped unless KOLADA_SYNC_URL is set, e.g.:
//!   KOLADA_SYNC_URL=http://127.0.0.1:8787 cargo test -p kolada-sync --test e2e -- --nocapture

use std::collections::HashMap;
use std::net::TcpStream;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kolada_sync::{Channel, Engine, Event, Outgoing};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

type Sock = WebSocket<MaybeTlsStream<TcpStream>>;

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// A device: its engine, its sockets, and every event it has had.
struct Dev {
    e: Engine,
    socks: HashMap<&'static str, Sock>,
    events: Vec<Event>,
}

fn key(c: Channel) -> &'static str {
    match c {
        Channel::Group => "group",
        Channel::Pair => "pair",
    }
}

impl Dev {
    fn new(base: &str, name: &str) -> Dev {
        Dev { e: Engine::open(base, "desktop", name, None, None), socks: HashMap::new(), events: Vec::new() }
    }

    /// One turn of what a shell's network thread does.
    fn step(&mut self) {
        let t = now();
        for c in self.e.connects(t) {
            let mut req = c.url.as_str().into_client_request().unwrap();
            for (k, v) in &c.headers {
                req.headers_mut().insert(tungstenite::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
            }
            match tungstenite::connect(req) {
                Ok((ws, _)) => {
                    if let MaybeTlsStream::Plain(s) = ws.get_ref() {
                        s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
                    }
                    self.socks.insert(key(c.channel), ws);
                    self.e.opened(c.channel, t);
                }
                Err(tungstenite::Error::Http(res)) => {
                    let body = res.body().as_ref().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                    self.e.failed(c.channel, res.status().as_u16(), &body, t);
                }
                Err(_) => self.e.closed(c.channel, t),
            }
        }
        for (c, bytes) in self.e.outgoing() {
            if let Some(ws) = self.socks.get_mut(key(c)) {
                let _ = ws.send(Message::Binary(bytes.into()));
            }
        }
        for ch in [Channel::Group, Channel::Pair] {
            loop {
                let Some(ws) = self.socks.get_mut(key(ch)) else { break };
                match ws.read() {
                    Ok(Message::Binary(b)) => self.e.frame(ch, &b, now()),
                    Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
                        self.socks.remove(key(ch));
                        self.e.closed(ch, now());
                        break;
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
                    Err(_) => {
                        self.socks.remove(key(ch));
                        self.e.closed(ch, now());
                        break;
                    }
                }
            }
        }
        for t in self.e.transfers() {
            match t {
                kolada_sync::Transfer::Put { blob, url, body } => {
                    let status = http("PUT", &url, &body).map_or(0, |(s, _)| s);
                    self.e.put_done(blob, status, now());
                }
                kolada_sync::Transfer::Get { blob, url } => {
                    let got = http("GET", &url, &[]).filter(|(s, _)| *s == 200).map(|(_, b)| b);
                    self.e.got(blob, got);
                }
            }
        }
        self.e.tick(now());
        self.events.extend(self.e.events());
    }
}

/// Plain HTTP/1.1 to the local server (`http://host:port/...`): the status
/// and the body.
fn http(method: &str, url: &str, body: &[u8]) -> Option<(u16, Vec<u8>)> {
    http_with(method, url, &[], body)
}

fn http_with(method: &str, url: &str, headers: &[(String, String)], body: &[u8]) -> Option<(u16, Vec<u8>)> {
    use std::io::{Read, Write};
    let rest = url.strip_prefix("http://")?;
    let (host, path) = rest.split_at(rest.find('/')?);
    let mut s = TcpStream::connect(host).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let extra: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()).ok()?;
    s.write_all(body).ok()?;
    let mut all = Vec::new();
    s.read_to_end(&mut all).ok()?;
    let end = all.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&all[..end]).to_string();
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    let mut rest = all[end + 4..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        let mut out = Vec::new();
        let mut at = 0;
        loop {
            let line_end = rest[at..].windows(2).position(|w| w == b"\r\n")? + at;
            let n = usize::from_str_radix(String::from_utf8_lossy(&rest[at..line_end]).trim(), 16).ok()?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&rest[line_end + 2..line_end + 2 + n]);
            at = line_end + 2 + n + 2;
        }
        rest = out;
    }
    Some((status, rest))
}

fn until(devs: &mut [&mut Dev], what: &str, done: impl FnMut(&[&mut Dev]) -> bool) -> Duration {
    until_for(devs, what, 15, done)
}

fn until_for(devs: &mut [&mut Dev], what: &str, secs: u64, mut done: impl FnMut(&[&mut Dev]) -> bool) -> Duration {
    let start = Instant::now();
    while !done(devs) {
        assert!(start.elapsed() < Duration::from_secs(secs), "timed out waiting for {what}");
        for d in devs.iter_mut() {
            d.step();
        }
    }
    start.elapsed()
}

#[test]
fn two_devices_pair_and_quick_send_through_the_real_server() {
    let Ok(base) = std::env::var("KOLADA_SYNC_URL") else {
        eprintln!("KOLADA_SYNC_URL not set: skipped");
        return;
    };
    let mut a = Dev::new(&base, "PC");
    let mut b = Dev::new(&base, "Mac");

    a.e.show_code();
    until(&mut [&mut a], "a code", |d| d[0].events.iter().any(|e| matches!(e, Event::Code { .. })));
    let code = a.events.iter().find_map(|e| match e { Event::Code { code, .. } => Some(code.clone()), _ => None }).unwrap();
    // Only counts and shapes are printed: the code's shape, not the code.
    eprintln!("code shown: {}", code.chars().map(|c| if c == '-' { '-' } else { 'x' }).collect::<String>());

    b.e.enter_code(&code).unwrap();
    until(&mut [&mut a, &mut b], "the ask", |d| d[0].events.iter().any(|e| matches!(e, Event::Asks { .. })));
    a.e.allow(true, now());
    until(&mut [&mut a, &mut b], "pairing", |d| d[1].e.paired() && d[1].events.contains(&Event::Paired));
    // Names are sealed for the group, so only members can read them.
    until(&mut [&mut a, &mut b], "each other's names", |d| {
        d[0].e.devices().iter().any(|x| x.name == "Mac") && d[1].e.devices().iter().any(|x| x.name == "PC")
    });

    let id = a.e.send(Outgoing::Text("Gate B, row 14".into()), now()).unwrap();
    let took = until(&mut [&mut a, &mut b], "the send", |d| d[1].events.contains(&Event::Arrived(id)));
    eprintln!("arrived in {} ms (local)", took.as_millis());
    assert_eq!(b.e.copy(id).as_deref(), Some("Gate B, row 14"));
    until(&mut [&mut a, &mut b], "sent", |d| d[0].events.contains(&Event::Sent(id)));

    // A password: copied once on b, and gone from the server.
    let pw = a.e.send(Outgoing::Secret("hunter2!Xq9".into()), now()).unwrap();
    until(&mut [&mut a, &mut b], "the password", |d| d[1].events.contains(&Event::Arrived(pw)));
    assert!(b.e.copy(pw).is_some());
    assert!(b.e.copy(pw).is_none());
    for _ in 0..5 {
        b.step();
        a.step();
    }

    // b sends back the other way.
    let back = b.e.send(Outgoing::Text("https://kolada.app".into()), now()).unwrap();
    until(&mut [&mut a, &mut b], "the reply", |d| d[0].events.contains(&Event::Arrived(back)));

    // A picture: up to R2 by a ticket, its item with the thumbnail, then
    // down when b asks for it, whole.
    let picture: Vec<u8> = (0..150_000u32).map(|i| (i * 31 % 251) as u8).collect();
    let pic = a.e.send(Outgoing::Picture { bytes: picture.clone(), mime: "image/png".into(), name: "Screenshot.png".into(), width: 800, height: 600, thumb: vec![1, 2, 3] }, now()).unwrap();
    let took = until(&mut [&mut a, &mut b], "the picture's item", |d| d[1].events.contains(&Event::Arrived(pic)));
    eprintln!("a picture's item arrived in {} ms (local)", took.as_millis());
    until(&mut [&mut a, &mut b], "the picture", |d| d[1].events.iter().any(|e| matches!(e, Event::PictureReady(id) | Event::FetchFailed(id) if *id == pic)));
    assert_eq!(b.e.picture(pic), Some(picture), "the picture came down whole");
}

/// Pairs two devices through the server, as the first test does.
fn paired(base: &str) -> (Dev, Dev) {
    let mut a = Dev::new(base, "PC");
    let mut b = Dev::new(base, "iPhone");
    // A desk with note sync on, and a phone that follows it.
    a.e.set_sync(kolada_sync::SyncCap::On);
    b.e.set_sync(kolada_sync::SyncCap::Follow);
    a.e.show_code();
    until(&mut [&mut a], "a code", |d| d[0].events.iter().any(|e| matches!(e, Event::Code { .. })));
    let code = a.events.iter().find_map(|e| match e { Event::Code { code, .. } => Some(code.clone()), _ => None }).unwrap();
    b.e.enter_code(&code).unwrap();
    until(&mut [&mut a, &mut b], "the ask", |d| d[0].events.iter().any(|e| matches!(e, Event::Asks { .. })));
    a.e.allow(true, now());
    until(&mut [&mut a, &mut b], "pairing", |d| d[1].e.paired() && d[1].events.contains(&Event::Paired));
    until(&mut [&mut a, &mut b], "each other's names", |d| d[0].e.devices().iter().any(|x| x.name == "iPhone") && d[1].e.devices().iter().any(|x| x.name == "PC"));
    until(&mut [&mut a, &mut b], "note sync on, both ways", |d| !d[0].e.sync_peers().is_empty() && !d[1].e.sync_peers().is_empty());
    // Each vouched for by the other through the group's roster tag, as the server lists it.
    until(&mut [&mut a, &mut b], "each other's keys vouched for", |d| d[0].e.devices().iter().all(|x| x.trusted) && d[1].e.devices().iter().all(|x| x.trusted));
    (a, b)
}

/// Note sync through the real server: a note written on one device's vault
/// reaches the other's, an edit there comes back, and each batch is
/// acknowledged once applied. Only counts and lengths are printed.
#[test]
fn two_vaults_sync_notes_through_the_real_server() {
    let Ok(base) = std::env::var("KOLADA_SYNC_URL") else {
        eprintln!("KOLADA_SYNC_URL not set: skipped");
        return;
    };
    let (mut a, mut b) = paired(&base);
    let dir = std::env::temp_dir().join(format!("kolada-e2e-notes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (ra, rb) = (dir.join("a/vault"), dir.join("b/vault"));
    std::fs::create_dir_all(&ra).unwrap();
    std::fs::create_dir_all(&rb).unwrap();
    let mut va = kolada_sync::vault::Vault::open(&ra, &dir.join("a/data"), true);
    let mut vb = kolada_sync::vault::Vault::open(&rb, &dir.join("b/data"), true);
    std::fs::write(ra.join("Plan.md"), "# Plan\n- one\n").unwrap();
    std::fs::create_dir_all(ra.join("Work")).unwrap();
    std::fs::write(ra.join("Work/Ideas.md"), "ideas").unwrap();
    // Sends what a vault has waiting; applies on the other what arrives.
    fn send(from: &mut Dev, v: &mut kolada_sync::vault::Vault) -> usize {
        let batches = v.outgoing(true);
        for x in &batches {
            from.e.send_sync(x, now()).unwrap();
        }
        batches.len()
    }
    fn take(to: &mut Dev, v: &mut kolada_sync::vault::Vault) -> usize {
        let mut n = 0;
        for e in std::mem::take(&mut to.events) {
            if let Event::Synced { id, from, name, batch } = e {
                n += v.apply(&batch, from, &name, to.e.device_id(), now(), None).len();
                to.e.applied(id, now());
            }
        }
        n
    }
    assert_eq!(va.scan(), 2);
    assert_eq!(send(&mut a, &mut va), 1, "two notes, one batch");
    let start = Instant::now();
    let mut written = 0;
    while written < 2 {
        assert!(start.elapsed() < Duration::from_secs(15), "timed out waiting for the notes");
        a.step();
        b.step();
        written += take(&mut b, &mut vb);
    }
    eprintln!("2 notes arrived in {} ms (local)", start.elapsed().as_millis());
    assert_eq!(std::fs::read_to_string(rb.join("Work/Ideas.md")).unwrap().len(), 5);
    // An edit on b comes back.
    std::fs::write(rb.join("Plan.md"), "# Plan\n- one\n- two\n").unwrap();
    assert!(vb.changed("Plan.md"));
    assert_eq!(send(&mut b, &mut vb), 1);
    let start = Instant::now();
    while take(&mut a, &mut va) == 0 {
        assert!(start.elapsed() < Duration::from_secs(15), "timed out waiting for the edit");
        a.step();
        b.step();
    }
    assert_eq!(std::fs::read_to_string(ra.join("Plan.md")).unwrap(), "# Plan\n- one\n- two\n");
    // Nothing echoes: neither side has anything left to send.
    va.scan();
    vb.scan();
    assert!(!va.has_pending() && !vb.has_pending());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Sync's manifest exchange through the real server: two vaults that hold
/// a note alike swap manifests by relay (the server keeps nothing), and
/// then only what differs waits to go. A device not connected is "away".
#[test]
fn two_vaults_meet_through_the_real_server() {
    let Ok(base) = std::env::var("KOLADA_SYNC_URL") else {
        eprintln!("KOLADA_SYNC_URL not set: skipped");
        return;
    };
    let (mut a, mut b) = paired(&base);
    let dir = std::env::temp_dir().join(format!("kolada-e2e-meet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (ra, rb) = (dir.join("a/vault"), dir.join("b/vault"));
    std::fs::create_dir_all(&ra).unwrap();
    std::fs::create_dir_all(&rb).unwrap();
    let mut va = kolada_sync::vault::Vault::open(&ra, &dir.join("a/data"), true);
    let mut vb = kolada_sync::vault::Vault::open(&rb, &dir.join("b/data"), true);
    std::fs::write(ra.join("Same.md"), "alike").unwrap();
    std::fs::write(rb.join("Same.md"), "alike").unwrap();
    std::fs::write(ra.join("A.md"), "a").unwrap();
    std::fs::write(rb.join("B.md"), "b").unwrap();
    va.scan();
    vb.scan();
    // Waits for a whole manifest at `to`, as the sync loop gathers it.
    fn manifest(devs: &mut [&mut Dev], at: usize) -> std::collections::HashMap<String, kolada_sync::journal::Short> {
        let mut got = Vec::new();
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < Duration::from_secs(15), "timed out waiting for a manifest");
            for d in devs.iter_mut() {
                d.step();
            }
            got.extend(devs[at].e.take_exchange().parts.into_iter().filter_map(|(_, p)| kolada_sync::journal::part(&p)));
            if got.first().is_some_and(|p| got.len() == p.count as usize) {
                return got.into_iter().flat_map(|p| p.entries).collect();
            }
        }
    }
    let (ida, idb) = (a.e.device_id(), b.e.device_id());
    for part in va.manifest(true, 1) {
        assert!(a.e.send_manifest(idb, &part));
    }
    let theirs = manifest(&mut [&mut a, &mut b], 1);
    assert_eq!(theirs.len(), 2);
    assert!(vb.meet(&theirs, false, true).is_empty(), "nothing agreed before: no answer");
    for part in vb.manifest(false, 1) {
        assert!(b.e.send_manifest(ida, &part));
    }
    let theirs = manifest(&mut [&mut a, &mut b], 0);
    va.meet(&theirs, false, true);
    assert!(va.settled() && vb.settled());
    let paths = |v: &mut kolada_sync::vault::Vault| -> Vec<String> {
        v.outgoing(true).iter().flat_map(|x| kolada_sync::journal::unbatch(x).unwrap()).map(|r| match r {
            kolada_sync::journal::Record::Put { path, .. } => path,
            _ => String::new(),
        }).collect()
    };
    assert_eq!(paths(&mut va), ["A.md"], "only what the other lacks");
    assert_eq!(paths(&mut vb), ["B.md"]);
    // A device that isn't connected: the server says so.
    assert!(a.e.send_manifest([9; 10], &va.manifest(true, 2)[0]));
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < Duration::from_secs(15), "timed out waiting for away");
        a.step();
        if a.e.take_exchange().away.contains(&[9; 10]) {
            break;
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The share extension through the real server: an engine with no socket
/// and the phone's keys sends a picture by requests alone (its ticket, the
/// upload, its item), and the PC gets it whole.
#[test]
fn the_share_extension_sends_a_picture_through_the_real_server() {
    let Ok(base) = std::env::var("KOLADA_SYNC_URL") else {
        eprintln!("KOLADA_SYNC_URL not set: skipped");
        return;
    };
    let (mut a, b) = paired(&base);
    let mut ext = kolada_sync::Engine::open(&base, "ios", "iPhone", None, Some(&b.e.keys_blob()));
    let picture: Vec<u8> = (0..120_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let what = Outgoing::Picture { bytes: picture.clone(), mime: "image/png".into(), name: "Shared.png".into(), width: 640, height: 480, thumb: vec![1, 2, 3] };
    let (card, ask) = ext.post_picture(what, now()).unwrap();
    let (status, answer) = http_with("POST", &ask.url, &ask.headers, &ask.body).unwrap();
    assert_eq!(status, 200, "the ticket");
    ext.post_answer(&answer);
    let Some(kolada_sync::Transfer::Put { blob, url, body }) = ext.transfers().pop() else { panic!("an upload") };
    let (status, _) = http("PUT", &url, &body).unwrap();
    let item = ext.put_posted(blob, status, now()).expect("uploaded");
    let (status, _) = http_with("POST", &item.url, &item.headers, &item.body).unwrap();
    assert_eq!(status, 200, "the item");
    until(&mut [&mut a], "the picture's item", |d| d[0].events.contains(&Event::Arrived(card)));
    until(&mut [&mut a], "the picture", |d| d[0].events.iter().any(|e| matches!(e, Event::PictureReady(id) | Event::FetchFailed(id) if *id == card)));
    assert_eq!(a.e.picture(card), Some(picture), "whole");
}

/// This PC's half of a pairing with a device elsewhere (the Mac's panel,
/// driven by its debug script, `KOLADA_DRIVE`): joins the code it shows,
/// waits to be allowed, beams a line, and waits for one back. Skipped
/// unless KOLADA_PAIR_CODE is set as well.
#[test]
fn joins_a_code_shown_elsewhere_and_beams_both_ways() {
    let (Ok(base), Ok(code)) = (std::env::var("KOLADA_SYNC_URL"), std::env::var("KOLADA_PAIR_CODE")) else {
        eprintln!("KOLADA_SYNC_URL and KOLADA_PAIR_CODE not set: skipped");
        return;
    };
    let mut a = Dev::new(&base, "Desk PC");
    a.e.enter_code(&code).unwrap();
    until_for(&mut [&mut a], "being allowed", 120, |d| d[0].e.paired());
    until(&mut [&mut a], "the other's name", |d| d[0].e.devices().iter().any(|x| x.id != d[0].e.device_id() && !x.name.is_empty()));
    let id = a.e.send(Outgoing::Text("Hello from the PC".into()), now()).unwrap();
    until(&mut [&mut a], "sent", |d| d[0].events.contains(&Event::Sent(id)));
    eprintln!("beamed; waiting for one back");
    until_for(&mut [&mut a], "a clip back", 120, |d| d[0].e.cards().iter().any(|c| !c.mine));
    let back = a.e.cards().into_iter().find(|c| !c.mine).unwrap().text.clone();
    assert_eq!(back, "Hello from the Mac");
    eprintln!("a clip came back");
}
