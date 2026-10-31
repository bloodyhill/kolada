//! Engines paired and talking through a stand-in for the server, which does
//! what the sync server does with frames, minus storage. The real
//! server is exercised end to end in tests/e2e.rs.

use kolada_sync::crypto::hex;
use kolada_sync::frame::Frame;
use kolada_sync::{Channel, Engine, Event, Kind, Outgoing, Refusal};

const T: u64 = 1_800_000_000_000;

fn engine(name: &str, kind: &str) -> Engine {
    Engine::open("https://sync.kolada.app", kind, name, None, None)
}

fn out(e: &mut Engine, ch: Channel) -> Vec<Frame> {
    e.outgoing().into_iter().filter(|(c, _)| *c == ch).map(|(_, b)| Frame::parse(&b).unwrap()).collect()
}

fn feed(e: &mut Engine, ch: Channel, f: Frame) {
    e.frame(ch, &f.encode(), T);
}

fn hello(e: &mut Engine, devices: &[(DeviceIdHex, [u8; 32], String)]) {
    // The tag carries what the device says of sync: "word|tag".
    let list: Vec<String> = devices.iter().map(|(id, b, tag)| {
        let (caps, tag) = tag.split_once('|').unwrap_or(("", tag));
        format!("{id}:desktop::{}:::{caps}:{tag}", kolada_sync::crypto::b64url(b))
    }).collect();
    e.opened(Channel::Group, T);
    feed(e, Channel::Group, Frame::new("hello").with("cursor", 0).with("head", 0).with("backlog", 0).with("every", 10).with("devices", list.join(",")));
}

type DeviceIdHex = String;

/// What the server lists for a device: its id and box key.
fn who(e: &Engine) -> (DeviceIdHex, [u8; 32], String) {
    (hex(&e.device_id()), e.box_pub(), format!("{}|{}", e.sync_word(), e.my_tag().unwrap_or_default()))
}

/// Pairs b into a's group (a makes it if it has none); returns a's code.
fn pair(a: &mut Engine, b: &mut Engine) -> String {
    a.show_code();
    let c = a.connects(T);
    assert_eq!(c.iter().find(|c| c.channel == Channel::Pair).unwrap().url, "wss://sync.kolada.app/v1/pair/open");
    a.opened(Channel::Pair, T);
    feed(a, Channel::Pair, Frame::new("slot").with("slot", "F4F").with("expires", T + 300_000));
    let code = a.events().into_iter().find_map(|e| match e { Event::Code { code, .. } => Some(code), _ => None }).unwrap();
    let msg_a = out(a, Channel::Pair).into_iter().find(|f| f.t() == "pake").unwrap().body;

    b.enter_code(&code.to_lowercase()).unwrap();
    let c = b.connects(T);
    assert!(c.iter().any(|c| c.url.ends_with("/v1/pair/join?slot=F4F")));
    b.opened(Channel::Pair, T);
    feed(b, Channel::Pair, Frame::new("pake").body(msg_a));
    let confirm = out(b, Channel::Pair).into_iter().find(|f| f.t() == "confirm").unwrap();
    feed(a, Channel::Pair, confirm);
    assert!(a.events().iter().any(|e| matches!(e, Event::Asks { name, .. } if name == &b.name())));
    a.allow(true, T);
    assert!(a.paired(), "allowing makes the group if there was none");
    let ab = [who(a), who(b)];
    hello(a, &ab);
    let frames = out(a, Channel::Pair);
    let welcome = frames.iter().find(|f| f.t() == "welcome").unwrap().clone();
    assert!(frames.iter().any(|f| f.t() == "done"));
    feed(b, Channel::Pair, welcome);
    assert!(b.events().contains(&Event::Paired));
    assert!(b.paired());
    let c = b.connects(T);
    let g = c.iter().find(|c| c.channel == Channel::Group).unwrap();
    assert!(g.headers.iter().any(|(k, _)| k == "x-kolada-voucher"), "a new device joins with a voucher");
    // Each in the group's list with the roster tag it registered with.
    let ab = [who(a), who(b)];
    hello(a, &ab);
    hello(b, &ab);
    code
}

/// Relays one `send` from `from` to `to` as the server's `item`.
fn deliver(from: &mut Engine, to: &mut Engine, seq: u64) -> Frame {
    let send = out(from, Channel::Group).into_iter().find(|f| f.t() == "send").expect("a send frame");
    let item = Frame::new("item").with("seq", seq).with("id", send.get("id").unwrap()).with("kind", send.get("kind").unwrap())
        .with("from", hex(&from.device_id())).with("at", T).with("head", seq).body(send.body.clone());
    feed(to, Channel::Group, item);
    send
}

#[test]
fn two_devices_pair_with_the_code_and_send_to_each_other() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("iPhone", "ios");
    let code = pair(&mut a, &mut b);
    assert_eq!(code.len(), 9);
    a.events();
    b.events();

    let id = a.send(Outgoing::Text("Gate B, row 14".into()), T).unwrap();
    deliver(&mut a, &mut b, 1);
    assert!(b.events().contains(&Event::Arrived(id)));
    let card = b.cards()[0].clone();
    assert_eq!((card.kind, card.text.as_str(), card.read, card.mine), (Kind::Text, "Gate B, row 14", false, false));
    assert_eq!(b.unread(), 1);
    assert_eq!(b.copy(id).as_deref(), Some("Gate B, row 14"));
    assert_eq!(b.unread(), 0);
    // b acknowledges what it stored.
    assert!(out(&mut b, Channel::Group).iter().any(|f| f.t() == "ack" && f.get("seq") == Some("1")));
}

#[test]
fn a_clip_can_go_to_one_device_leaves_once_saved_and_stays_until_removed() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("iPhone", "ios");
    pair(&mut a, &mut b);
    let only = [b.device_id()];
    a.send_to(Outgoing::Text("for the phone".into()), Some(&only), T).unwrap();
    let send = out(&mut a, Channel::Group).into_iter().find(|f| f.t() == "send").unwrap();
    assert_eq!(send.get("to"), Some(hex(&b.device_id()).as_str()));
    // Saved as a note: it leaves Quick Send.
    let id = a.send(Outgoing::Text("keep me".into()), T).unwrap();
    assert!(a.forget(id).is_some());
    assert!(a.cards().iter().all(|c| c.id != id));
    assert!(a.forget(id).is_none(), "once");
    // Kept until removed: a year on, still there.
    a.tick(T + 365 * 86_400_000);
    assert_eq!(a.cards().len(), 1);
    assert_eq!(a.next_wake(), None, "nothing to wake for");
    // A password, though, goes after a day.
    a.send(Outgoing::Secret("hunter2!Xq9".into()), T + 365 * 86_400_000).unwrap();
    a.tick(T + 366 * 86_400_000);
    assert_eq!(a.cards().len(), 1);
}

/// A batch of note changes goes sealed under the sync key, comes out the
/// same on the other device, and is acknowledged only once applied: if the
/// app stops first, the server brings it again and it is applied then.
#[test]
fn a_sync_batch_arrives_whole_and_is_acknowledged_once_applied() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("iPhone", "ios");
    pair(&mut a, &mut b);
    let batch = b"\x00\x00\x00\x00 a batch of changes".to_vec();
    // To every other device (send_sync goes only to devices that sync notes).
    a.send_sync_to(&batch, None, T).unwrap();
    let send = out(&mut a, Channel::Group).into_iter().find(|f| f.t() == "send").unwrap();
    assert_eq!(send.get("kind"), Some("s"));
    assert!(!send.body.windows(7).any(|w| w == b"changes"), "sealed");
    let item = Frame::new("item").with("seq", 1).with("id", send.get("id").unwrap()).with("kind", "s").with("from", hex(&a.device_id())).with("at", T).with("head", 1).body(send.body.clone());
    feed(&mut b, Channel::Group, item.clone());
    let got = b.events().into_iter().find_map(|e| match e { Event::Synced { id, name, batch, .. } => Some((id, name, batch)), _ => None }).expect("handed to the vault");
    assert_eq!(got.2, batch);
    assert!(!got.1.is_empty(), "named for the history it goes into (this stand-in relays no names)");
    assert!(!out(&mut b, Channel::Group).iter().any(|f| f.t() == "ack" && f.get("seq") == Some("1")), "not before it is applied");
    // Brought again before it was applied (a reconnect): handed over once.
    feed(&mut b, Channel::Group, item.clone());
    assert!(!b.events().iter().any(|e| matches!(e, Event::Synced { .. })));
    b.applied(got.0, T);
    assert!(out(&mut b, Channel::Group).iter().any(|f| f.t() == "ack" && f.get("seq") == Some("1")));
    // Applied, it is not applied twice.
    feed(&mut b, Channel::Group, item);
    assert!(!b.events().iter().any(|e| matches!(e, Event::Synced { .. })));
}

#[test]
fn a_link_is_a_link_and_a_duplicate_is_dropped() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    a.send(Outgoing::Text("https://kolada.app".into()), T).unwrap();
    let send = deliver(&mut a, &mut b, 1);
    assert_eq!(b.cards()[0].kind, Kind::Link);
    let again = Frame::new("item").with("seq", 2).with("id", send.get("id").unwrap()).with("kind", "q").with("from", hex(&a.device_id())).with("at", T).body(send.body);
    feed(&mut b, Channel::Group, again);
    assert_eq!(b.cards().len(), 1);
}

#[test]
fn a_password_copies_once_and_goes_everywhere() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("iPhone", "ios");
    pair(&mut a, &mut b);
    let id = a.send(Outgoing::Secret("hunter2!Xq9".into()), T).unwrap();
    deliver(&mut a, &mut b, 1);
    assert_eq!(b.cards()[0].kind, Kind::Secret);
    assert!(b.history("").is_empty(), "passwords stay out of search");
    assert_eq!(b.copy(id).as_deref(), Some("hunter2!Xq9"));
    assert_eq!(b.copy(id), None);
    assert!(b.cards()[0].text.is_empty(), "the text is gone once copied");
    assert!(out(&mut b, Channel::Group).iter().any(|f| f.t() == "consumed" && f.get("id") == Some(hex(&id).as_str())));
    // The sender can't copy its own password back out of history.
    assert_eq!(a.copy(id), None);
    // Passwords are wiped after 24 hours either way.
    a.tick(T + 24 * 3_600_000 + 1);
    assert!(a.cards().is_empty());
}

#[test]
fn the_server_dropping_an_item_removes_its_card() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    let id = a.send(Outgoing::Secret("hunter2!Xq9".into()), T).unwrap();
    deliver(&mut a, &mut b, 1);
    feed(&mut b, Channel::Group, Frame::new("gone").with("ids", hex(&id)));
    assert!(b.cards().is_empty());
}

#[test]
fn a_wrong_code_is_refused_and_the_opener_tries_again() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("iPhone", "ios");
    a.show_code();
    a.connects(T);
    a.opened(Channel::Pair, T);
    feed(&mut a, Channel::Pair, Frame::new("slot").with("slot", "F4F").with("expires", T + 300_000));
    let msg_a = out(&mut a, Channel::Pair).into_iter().find(|f| f.t() == "pake").unwrap().body;
    b.enter_code("F4F0-0000").unwrap();
    b.connects(T);
    b.opened(Channel::Pair, T);
    feed(&mut b, Channel::Pair, Frame::new("pake").body(msg_a));
    let confirm = out(&mut b, Channel::Pair).into_iter().find(|f| f.t() == "confirm").unwrap();
    feed(&mut a, Channel::Pair, confirm);
    let sent: Vec<String> = out(&mut a, Channel::Pair).iter().map(|f| f.t().to_string()).collect();
    assert_eq!(sent, vec!["bad", "pake"], "tells the server, then offers a fresh start");
    assert!(!a.events().iter().any(|e| matches!(e, Event::Asks { .. })));
    feed(&mut b, Channel::Pair, Frame::new("refused"));
    assert!(b.events().contains(&Event::CodeRefused(Refusal::WrongCode)));
}

#[test]
fn saying_no_leaves_the_other_device_unpaired() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("iPhone", "ios");
    a.show_code();
    a.connects(T);
    a.opened(Channel::Pair, T);
    feed(&mut a, Channel::Pair, Frame::new("slot").with("slot", "F4F").with("expires", T + 300_000));
    let code = a.events().into_iter().find_map(|e| match e { Event::Code { code, .. } => Some(code), _ => None }).unwrap();
    let msg_a = out(&mut a, Channel::Pair).into_iter().find(|f| f.t() == "pake").unwrap().body;
    b.enter_code(&code).unwrap();
    b.connects(T);
    b.opened(Channel::Pair, T);
    feed(&mut b, Channel::Pair, Frame::new("pake").body(msg_a));
    feed(&mut a, Channel::Pair, out(&mut b, Channel::Pair).remove(0));
    a.allow(false, T);
    assert!(!a.paired(), "saying no makes no group");
    let deny = out(&mut a, Channel::Pair).into_iter().find(|f| f.t() == "deny").unwrap();
    feed(&mut b, Channel::Pair, deny);
    assert!(b.events().contains(&Event::CodeRefused(Refusal::Denied)));
    assert!(!b.paired());
}

#[test]
fn the_server_refusing_a_code_says_why() {
    let mut b = engine("iPhone", "ios");
    for (status, body, why) in [
        (403, "error=country\n", Refusal::Country),
        (410, "error=expired\n", Refusal::Expired),
        (410, "error=burned\n", Refusal::Burned),
        (404, "error=code\n", Refusal::NoSuchCode),
        (503, "error=quota\nretry=100\n", Refusal::Quota),
        (429, "error=slow\n", Refusal::Slow),
    ] {
        b.enter_code("F4FS-2RSA").unwrap();
        b.connects(T);
        b.failed(Channel::Pair, status, body, T);
        assert!(b.events().contains(&Event::CodeRefused(why)), "{status} {body}");
    }
}

#[test]
fn a_removed_device_cannot_read_what_is_sent_after() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    let mut c = engine("Old phone", "ios");
    pair(&mut a, &mut b);
    pair(&mut a, &mut c);
    // Everyone knows everyone (with their box keys).
    let all = [who(&a), who(&b), who(&c)];
    hello(&mut a, &all);
    a.remove(c.device_id());
    let frames = out(&mut a, Channel::Group);
    let rekeys: Vec<&Frame> = frames.iter().filter(|f| f.t() == "send" && f.get("kind") == Some("k")).collect();
    assert_eq!(rekeys.len(), 1, "one new secret, sealed to b only");
    assert_eq!(rekeys[0].get("to"), Some(hex(&b.device_id()).as_str()));
    assert!(frames.iter().any(|f| f.t() == "device" && f.get("op") == Some("remove")));
    let k = rekeys[0];
    feed(&mut b, Channel::Group, Frame::new("item").with("seq", 5).with("id", k.get("id").unwrap()).with("kind", "k").with("from", hex(&a.device_id())).with("at", T).body(k.body.clone()));
    // c gets the same box by mistake: it can't open it.
    feed(&mut c, Channel::Group, Frame::new("item").with("seq", 5).with("id", k.get("id").unwrap()).with("kind", "k").with("from", hex(&a.device_id())).with("at", T).body(k.body.clone()));

    let id = a.send(Outgoing::Text("after".into()), T).unwrap();
    let send = deliver(&mut a, &mut b, 6);
    assert!(b.events().contains(&Event::Arrived(id)), "b has the new keys");
    feed(&mut c, Channel::Group, Frame::new("item").with("seq", 6).with("id", send.get("id").unwrap()).with("kind", "q").with("from", hex(&a.device_id())).with("at", T).body(send.body));
    assert!(!c.events().contains(&Event::Arrived(id)), "c doesn't");
}

#[test]
fn a_rekey_the_server_forged_is_ignored() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    b.events();
    // The server knows b's box key, so it can seal to it, but it has no group secret.
    let mut plain = Vec::new();
    plain.extend_from_slice(&[9u8; 32]);
    plain.extend_from_slice(&2u32.to_be_bytes());
    plain.extend_from_slice(&[0u8; 32]);
    let forged = kolada_sync::crypto::seal_to(&b.box_pub(), &plain);
    feed(&mut b, Channel::Group, Frame::new("item").with("seq", 3).with("id", "ab".repeat(16)).with("kind", "k").with("from", hex(&a.device_id())).with("at", T).body(forged));
    assert!(!b.events().iter().any(|e| matches!(e, Event::Keys(_))));
    let id = a.send(Outgoing::Text("still ours".into()), T).unwrap();
    deliver(&mut a, &mut b, 4);
    assert!(b.events().contains(&Event::Arrived(id)));
}

#[test]
fn a_send_waits_for_the_socket_and_is_resent_until_the_server_has_it() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    a.closed(Channel::Group, T);
    let id = a.send(Outgoing::Text("offline".into()), T).unwrap();
    assert!(out(&mut a, Channel::Group).is_empty());
    let ab = [who(&a), who(&b)];
    hello(&mut a, &ab);
    let first = out(&mut a, Channel::Group).into_iter().find(|f| f.t() == "send").unwrap();
    assert_eq!(first.get("id"), Some(hex(&id).as_str()));
    // Dropped before `sent`: the same item goes again on the next connect.
    a.closed(Channel::Group, T);
    let ab = [who(&a), who(&b)];
    hello(&mut a, &ab);
    assert!(out(&mut a, Channel::Group).iter().any(|f| f.t() == "send" && f.get("id") == Some(hex(&id).as_str())));
    feed(&mut a, Channel::Group, Frame::new("sent").with("id", hex(&id)).with("seq", 1));
    a.closed(Channel::Group, T);
    let ab = [who(&a), who(&b)];
    hello(&mut a, &ab);
    assert!(!out(&mut a, Channel::Group).iter().any(|f| f.t() == "send"));
}

#[test]
fn reconnects_back_off_and_rest_is_obeyed() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    a.closed(Channel::Group, T);
    assert!(a.connects(T).is_empty(), "not straight back");
    let wake = a.next_wake().unwrap();
    assert!(wake > T && wake <= T + 1000);
    assert_eq!(a.connects(wake).len(), 1);
    a.failed(Channel::Group, 503, "error=rest\nretry=3600\n", wake);
    assert!(a.events().contains(&Event::Resting { until: wake + 3_600_000 }));
    assert!(a.connects(wake + 3_599_000).is_empty());
    assert_eq!(a.connects(wake + 3_600_000).len(), 1);
}

#[test]
fn keys_and_history_survive_a_restart() {
    let dir = std::env::temp_dir().join(format!("kolada-sync-test-{}", hex(&kolada_sync::crypto::random::<6>())));
    let mut a = engine("PC", "desktop");
    let mut b = Engine::open("https://sync.kolada.app", "desktop", "Mac", Some(dir.clone()), None);
    pair(&mut a, &mut b);
    let blob = b.events().into_iter().rev().find_map(|e| match e { Event::Keys(k) => Some(k), _ => None });
    let blob = blob.unwrap_or_else(|| b.keys_blob());
    let id = a.send(Outgoing::Text("kept".into()), T).unwrap();
    deliver(&mut a, &mut b, 1);
    drop(b);
    let again = Engine::open("https://sync.kolada.app", "desktop", "Mac", Some(dir.clone()), Some(&blob));
    assert!(again.paired());
    assert_eq!(again.cards()[0].id, id);
    assert_eq!(again.cards()[0].text, "kept");
    // On disk it's sealed: no text in the clear.
    let raw = std::fs::read(dir.join("quick-send.state")).unwrap();
    assert!(!raw.windows(4).any(|w| w == b"kept"));
    let _ = std::fs::remove_dir_all(dir);
}

/// What the group answers a ticket request with.
fn ticket_for(f: &Frame) -> Frame {
    Frame::new("ticket").with("id", f.get("id").unwrap()).with("op", f.get("op").unwrap()).with("exp", T + 600_000).with("sig", "c2ln")
}

#[test]
fn a_picture_goes_up_by_ticket_then_its_item_and_comes_down_when_asked() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    a.events();
    b.events();
    let bytes: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
    let thumb = vec![0xFF, 0xD8, 1, 2, 3];
    let id = a.send(Outgoing::Picture { bytes: bytes.clone(), mime: "image/png".into(), name: "Screenshot.png".into(), width: 1920, height: 1080, thumb: thumb.clone() }, T).unwrap();
    // The card shows at once; nothing is sent until the picture is up.
    assert_eq!(a.cards()[0].kind, Kind::Picture);
    let asked: Vec<Frame> = out(&mut a, Channel::Group);
    let ticket = asked.iter().find(|f| f.t() == "ticket").expect("a ticket asked for");
    assert_eq!(ticket.get("op"), Some("put"));
    assert!(!asked.iter().any(|f| f.t() == "send"), "no item before the picture is up");
    feed(&mut a, Channel::Group, ticket_for(ticket));
    let put = a.transfers();
    let (blob, sealed) = match &put[..] {
        [kolada_sync::Transfer::Put { blob, url, body }] => {
            assert!(url.starts_with("https://sync.kolada.app/v1/g/") && url.contains(&format!("/blob/{}?exp=", hex(blob))) && url.ends_with("&sig=c2ln"), "{url}");
            assert!(body.len() > bytes.len(), "sealed, with its tags");
            assert!(!body.windows(64).any(|w| w == &bytes[1000..1064]), "the server never sees the picture");
            (*blob, body.clone())
        }
        other => panic!("one PUT: {other:?}"),
    };
    a.put_done(blob, 200, T);
    let item = deliver(&mut a, &mut b, 1);
    assert_eq!(item.get("blob"), Some(hex(&blob).as_str()), "the item names its blob, so the server can drop both together");
    assert!(b.events().contains(&Event::Arrived(id)));
    let card = b.cards()[0].clone();
    let pic = card.picture.clone().expect("a picture card");
    assert_eq!((card.kind, card.text.as_str(), pic.mime.as_str(), pic.size, pic.width, pic.height, pic.thumb.clone()), (Kind::Picture, "Screenshot.png", "image/png", 200_000, 1920, 1080, thumb));
    // It comes down at once, and its item is acknowledged only after: the
    // server drops both once every device has.
    let asked = out(&mut b, Channel::Group);
    let ticket = asked.iter().find(|f| f.t() == "ticket").expect("a ticket for the GET");
    assert_eq!(ticket.get("op"), Some("get"));
    assert!(!asked.iter().any(|f| f.t() == "ack"), "no ack while the picture is still up there");
    assert!(!b.has_picture(id));
    feed(&mut b, Channel::Group, ticket_for(ticket));
    assert!(matches!(&b.transfers()[..], [kolada_sync::Transfer::Get { .. }]));
    // No answer: asked again on the next connection, still unacknowledged.
    b.got(blob, None);
    assert!(!out(&mut b, Channel::Group).iter().any(|f| f.t() == "ack"));
    b.got(blob, Some(sealed.clone()));
    assert!(b.events().contains(&Event::PictureReady(id)));
    assert_eq!(b.picture(id), Some(bytes.clone()), "kept here, whole");
    assert!(out(&mut b, Channel::Group).iter().any(|f| f.t() == "ack" && f.get("seq") == Some("1")), "acknowledged once down");
    assert_eq!(a.picture(id), Some(bytes), "the sender keeps its own");
    // One changed on the way is refused, and its card goes.
    let mut c = engine("Laptop", "desktop");
    pair(&mut a, &mut c);
    c.events();
    let id2 = a.send(Outgoing::Picture { bytes: vec![5; 3000], mime: "image/png".into(), name: "x.png".into(), width: 1, height: 1, thumb: vec![] }, T).unwrap();
    let t = out(&mut a, Channel::Group).into_iter().find(|f| f.t() == "ticket").unwrap();
    feed(&mut a, Channel::Group, ticket_for(&t));
    let (blob2, mut bad) = match a.transfers().pop() {
        Some(kolada_sync::Transfer::Put { blob, body, .. }) => (blob, body),
        other => panic!("{other:?}"),
    };
    a.put_done(blob2, 200, T);
    deliver(&mut a, &mut c, 1);
    bad[40] ^= 1;
    c.got(blob2, Some(bad));
    assert!(c.events().contains(&Event::FetchFailed(id2)));
    assert!(c.cards().iter().all(|k| k.id != id2));
}

#[test]
fn a_picture_the_server_refuses_leaves_and_says_why() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    a.events();
    let id = a.send(Outgoing::Picture { bytes: vec![1; 1000], mime: "image/jpeg".into(), name: "Photo.jpg".into(), width: 10, height: 10, thumb: vec![] }, T).unwrap();
    let t = out(&mut a, Channel::Group).into_iter().find(|f| f.t() == "ticket").unwrap();
    feed(&mut a, Channel::Group, ticket_for(&t));
    let blob = match &a.transfers()[..] {
        [kolada_sync::Transfer::Put { blob, .. }] => *blob,
        other => panic!("{other:?}"),
    };
    a.put_done(blob, 503, T);
    assert!(a.events().contains(&Event::SendFailed(id, "pictures".into())));
    assert!(a.cards().is_empty(), "the card goes, to be sent again");
    assert!(!out(&mut a, Channel::Group).iter().any(|f| f.t() == "send"));
    // Too big, or not paired: nothing.
    assert!(a.send(Outgoing::Picture { bytes: vec![0; kolada_sync::engine::PICTURE_MOST + 1], mime: "image/png".into(), name: "x.png".into(), width: 1, height: 1, thumb: vec![] }, T).is_none());
    let mut c = engine("Laptop", "desktop");
    assert!(c.send(Outgoing::Picture { bytes: vec![1], mime: "image/png".into(), name: "x.png".into(), width: 1, height: 1, thumb: vec![] }, T).is_none());
}

#[test]
fn picture_cards_and_their_thumbnails_survive_a_restart() {
    let dir = std::env::temp_dir().join(format!("kolada-sync-pictures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut a = Engine::open("https://sync.kolada.app", "desktop", "PC", Some(dir.clone()), None);
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    let keys = a.keys_blob();
    let id = a.send(Outgoing::Picture { bytes: vec![7; 5000], mime: "image/png".into(), name: "Plan.png".into(), width: 40, height: 30, thumb: vec![9; 300] }, T).unwrap();
    let t = out(&mut a, Channel::Group).into_iter().find(|f| f.t() == "ticket").expect("a ticket asked for");
    feed(&mut a, Channel::Group, ticket_for(&t));
    let blob = match &a.transfers()[..] {
        [kolada_sync::Transfer::Put { blob, .. }] => *blob,
        other => panic!("{other:?}"),
    };
    a.put_done(blob, 200, T);
    // One more, never up when the app quits: it can't go after, so it goes.
    let waiting = a.send(Outgoing::Picture { bytes: vec![8; 50], mime: "image/png".into(), name: "Later.png".into(), width: 4, height: 3, thumb: vec![] }, T).unwrap();
    drop(a);
    let again = Engine::open("https://sync.kolada.app", "desktop", "PC", Some(dir.clone()), Some(&keys));
    let card = again.cards().into_iter().find(|c| c.id == id).cloned().expect("the card came back");
    let pic = card.picture.expect("with its picture");
    assert_eq!((pic.name.as_str(), pic.width, pic.thumb.len()), ("Plan.png", 40, 300));
    assert!(again.cards().iter().all(|c| c.id != waiting), "a picture that never went up does not come back");
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture's ticket names its size, which the server binds into the
/// ticket and checks the upload against; the URL carries the size back.
#[test]
fn a_pictures_ticket_names_its_size_and_the_upload_carries_it() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    a.send(Outgoing::Picture { bytes: vec![7; 5000], mime: "image/png".into(), name: "Plan.png".into(), width: 40, height: 30, thumb: vec![] }, T).unwrap();
    let t = out(&mut a, Channel::Group).into_iter().find(|f| f.t() == "ticket").expect("a ticket asked for");
    let n: usize = t.get("n").and_then(|n| n.parse().ok()).expect("its size");
    assert!(n > 5000, "the sealed size, tags included");
    feed(&mut a, Channel::Group, ticket_for(&t).with("n", n));
    match &a.transfers()[..] {
        [kolada_sync::Transfer::Put { url, body, .. }] => {
            assert!(url.ends_with(&format!("&n={n}")), "{url}");
            assert_eq!(body.len(), n);
        }
        other => panic!("one PUT: {other:?}"),
    }
}

/// The group refuses a picture's ticket (the day's pictures, a burst): the
/// upload and its card go, and the shell is told why, under the card's id.
#[test]
fn a_refused_ticket_fails_its_pictures_card() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    a.events();
    let card = a.send(Outgoing::Picture { bytes: vec![7; 500], mime: "image/png".into(), name: "Plan.png".into(), width: 4, height: 3, thumb: vec![] }, T).unwrap();
    let t = out(&mut a, Channel::Group).into_iter().find(|f| f.t() == "ticket").unwrap();
    feed(&mut a, Channel::Group, Frame::new("error").with("id", t.get("id").unwrap()).with("error", "limit").with("retry", 3600));
    assert!(a.events().contains(&Event::SendFailed(card, "limit".into())));
    assert!(a.cards().iter().all(|c| c.id != card));
    assert!(a.transfers().is_empty());
}

/// A beam that expired on the server before any device had it: its card
/// says so, across a restart, and resending sends it again as a new beam
/// in its place. Text and pictures alike.
#[test]
fn a_beam_that_didnt_arrive_says_so_and_goes_again() {
    let dir = std::env::temp_dir().join(format!("kolada-sync-lost-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut a = Engine::open("https://sync.kolada.app", "desktop", "PC", Some(dir.clone()), None);
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    let keys = a.keys_blob();
    let text = a.send(Outgoing::Text("Gate B, row 14".into()), T).unwrap();
    let pic = a.send(Outgoing::Picture { bytes: vec![7; 5000], mime: "image/png".into(), name: "Plan.png".into(), width: 40, height: 30, thumb: vec![9; 30] }, T).unwrap();
    let t = out(&mut a, Channel::Group).into_iter().find(|f| f.t() == "ticket").unwrap();
    feed(&mut a, Channel::Group, ticket_for(&t));
    let blob = match &a.transfers()[..] {
        [kolada_sync::Transfer::Put { blob, .. }] => *blob,
        other => panic!("{other:?}"),
    };
    a.put_done(blob, 200, T);
    a.events();
    // Another device's id is ignored: only this device's own cards.
    feed(&mut a, Channel::Group, Frame::new("lost").with("ids", format!("{},{},{}", hex(&text), hex(&pic), hex(&[3u8; 16]))));
    let ev = a.events();
    assert!(ev.contains(&Event::Lost(text)) && ev.contains(&Event::Lost(pic)));
    assert_eq!(ev.iter().filter(|e| matches!(e, Event::Lost(_))).count(), 2);
    drop(a);
    let mut a = Engine::open("https://sync.kolada.app", "desktop", "PC", Some(dir.clone()), Some(&keys));
    let cards = a.cards();
    assert!(cards.iter().find(|c| c.id == text).is_some_and(|c| c.lost && c.mine), "still lost after a restart");
    assert!(cards.iter().find(|c| c.id == pic).is_some_and(|c| c.lost));
    a.opened(Channel::Group, T);
    feed(&mut a, Channel::Group, Frame::new("hello").with("cursor", 0).with("head", 0).with("backlog", 0).with("every", 10));
    out(&mut a, Channel::Group);
    let again = a.resend(text, T + 1).expect("sent again");
    assert_ne!(again, text);
    let cards = a.cards();
    assert!(cards.iter().all(|c| c.id != text), "the old card goes");
    assert!(cards.iter().find(|c| c.id == again).is_some_and(|c| !c.lost && c.text == "Gate B, row 14"));
    assert!(out(&mut a, Channel::Group).iter().any(|f| f.t() == "send" && f.get("id") == Some(hex(&again).as_str())));
    let pic_again = a.resend(pic, T + 2).expect("the picture from the copy kept here");
    assert_eq!(a.picture(pic_again), Some(vec![7; 5000]));
    assert!(out(&mut a, Channel::Group).iter().any(|f| f.t() == "ticket" && f.get("op") == Some("put")), "it goes up again");
    assert_eq!(a.resend([3; 16], T + 3), None, "not this device's");
    let _ = std::fs::remove_dir_all(dir);
}

/// The server lists each device's box key, which a removal re-keys the
/// group to: a key signed by the device's own key is taken; a badly signed
/// one leaves its device out; an unsigned one can't replace the key first
/// seen. A device from before signs its own, once a connection.
#[test]
fn a_box_key_the_server_swaps_is_not_trusted() {
    use kolada_sync::crypto::{b64url, DeviceKeys};
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    out(&mut a, Channel::Group);
    // A new connection, its own entry unsigned (a device from before).
    let both = [who(&a), who(&b)];
    hello(&mut a, &both);
    let sent = out(&mut a, Channel::Group);
    let boxes = sent.iter().filter(|f| f.t() == "device" && f.get("op") == Some("box")).count();
    assert_eq!(boxes, 1, "its own box key signed, once");
    let real = b.box_pub();
    let entry = |id: &str, bx: &[u8; 32]| format!("{id}:desktop::{}", b64url(bx));
    let (aid, bid) = (hex(&a.device_id()), hex(&b.device_id()));
    // An unsigned swap: the first key stays.
    let me = entry(&aid, &a.box_pub());
    let swapped = [me.clone(), entry(&bid, &[9; 32])].join(",");
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", swapped));
    assert_eq!(a.devices().iter().find(|d| hex(&d.id) == bid).unwrap().box_pub, real);
    // A device whose key is signed by its own key is taken; badly signed, left out.
    let c = DeviceKeys::new();
    let text = format!("kolada/v1/box|{}|{}", hex(&c.id()), b64url(&c.box_pub()));
    let good = format!("{}:{}:{}", entry(&hex(&c.id()), &c.box_pub()), b64url(&c.sign_pub()), b64url(&c.sign(text.as_bytes())));
    let bad = format!("{}:{}:{}", entry(&hex(&c.id()), &[7; 32]), b64url(&c.sign_pub()), b64url(&c.sign(text.as_bytes())));
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me.clone(), good].join(",")));
    assert_eq!(a.devices().iter().find(|d| d.id == c.id()).map(|d| d.box_pub), Some(c.box_pub()));
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me, bad].join(",")));
    assert!(a.devices().iter().all(|d| d.id != c.id()), "a badly signed key leaves its device out");
    assert!(!out(&mut a, Channel::Group).iter().any(|f| f.get("op") == Some("box")), "not again this connection");
}

/// The device showing a code gives it up after five wrong tries itself,
/// whatever the server counts.
#[test]
fn a_shown_code_goes_after_five_wrong_tries() {
    let mut a = engine("PC", "desktop");
    a.show_code();
    a.connects(T);
    a.opened(Channel::Pair, T);
    feed(&mut a, Channel::Pair, Frame::new("slot").with("slot", "F4F").with("expires", T + 300_000));
    a.events();
    for i in 0..kolada_sync::PAIR_TRIES {
        assert!(!a.events().contains(&Event::CodeRefused(Refusal::Burned)), "try {i}");
        feed(&mut a, Channel::Pair, Frame::new("confirm").body(vec![1; 80]));
    }
    assert!(a.events().contains(&Event::CodeRefused(Refusal::Burned)));
    let bads = out(&mut a, Channel::Pair).iter().filter(|f| f.t() == "bad").count();
    assert_eq!(bads as u32, kolada_sync::PAIR_TRIES);
}

/// A removal re-keys the group only to devices the group vouched for:
/// a device the server made up, even with a box key properly signed by
/// its own key, gets nothing; a key vouched for under the secret before a
/// re-key (which a removed device knows) is taken only if it was pinned
/// already; a device from before tags keeps the key first seen, until the
/// cutoff. Pins outlive a restart.
#[test]
fn the_group_vouches_for_the_keys_it_rekeys_to() {
    use kolada_sync::crypto::{b64url, DeviceKeys};
    let dir = std::env::temp_dir().join(format!("kolada-sync-roster-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut a = Engine::open("https://sync.kolada.app", "desktop", "PC", Some(dir.clone()), None);
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    let keys = a.keys_blob();
    let entry = |id: &str, bx: &[u8; 32], rest: &str| format!("{id}:desktop::{}:{rest}", b64url(bx));
    let (aid, bid) = (hex(&a.device_id()), hex(&b.device_id()));
    let me = entry(&aid, &a.box_pub(), &format!(":::{}", a.my_tag().unwrap()));
    let real_b = entry(&bid, &b.box_pub(), &format!(":::{}", b.my_tag().unwrap()));
    // A ghost the server made up: its own keys, its box key signed, no tag.
    let ghost = DeviceKeys::new();
    let text = format!("kolada/v1/box|{}|{}", hex(&ghost.id()), b64url(&ghost.box_pub()));
    let ghost_entry = entry(&hex(&ghost.id()), &ghost.box_pub(), &format!("{}:{}::", b64url(&ghost.sign_pub()), b64url(&ghost.sign(text.as_bytes()))));
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me.clone(), real_b.clone(), ghost_entry].join(",")));
    let listed = a.devices().to_vec();
    assert!(listed.iter().find(|d| d.id == ghost.id()).is_some_and(|d| !d.trusted), "listed, never trusted");
    assert!(listed.iter().find(|d| hex(&d.id) == bid).is_some_and(|d| d.trusted));
    // A swapped key for b with a made-up tag: b's pinned key stays.
    let forged = entry(&bid, &[9; 32], &format!(":::0.{}", b64url(&[1u8; 32])));
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me.clone(), forged].join(",")));
    assert_eq!(a.devices().iter().find(|d| hex(&d.id) == bid).unwrap().box_pub, b.box_pub());
    assert!(!a.devices().iter().find(|d| hex(&d.id) == bid).unwrap().trusted);
    // The removal seals the new secret to b only: never to the ghost.
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me.clone(), real_b.clone(), {
        let t = format!("kolada/v1/box|{}|{}", hex(&ghost.id()), b64url(&ghost.box_pub()));
        entry(&hex(&ghost.id()), &ghost.box_pub(), &format!("{}:{}::", b64url(&ghost.sign_pub()), b64url(&ghost.sign(t.as_bytes()))))
    }].join(",")));
    out(&mut a, Channel::Group);
    let c = DeviceKeys::new();
    a.remove(c.id());
    let rekeys: Vec<Frame> = out(&mut a, Channel::Group).into_iter().filter(|f| f.get("kind") == Some("k")).collect();
    assert_eq!(rekeys.iter().map(|f| f.get("to").unwrap().to_string()).collect::<Vec<_>>(), vec![bid.clone()]);
    // After the re-key, b's tag under the secret before still counts for its
    // pinned key, not for a new one (a removed device knows that secret).
    let stale = b.my_tag().unwrap();
    let me_now = entry(&aid, &a.box_pub(), &format!(":::{}", a.my_tag().unwrap()));
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me_now, real_b.clone()].join(",")));
    assert!(a.devices().iter().find(|d| hex(&d.id) == bid).is_some_and(|d| d.trusted), "b, not yet re-tagged, stays");
    let moved = entry(&bid, &[8; 32], &format!(":::{stale}"));
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me, moved].join(",")));
    assert!(a.devices().iter().find(|d| hex(&d.id) == bid).is_some_and(|d| !d.trusted && d.box_pub == b.box_pub()));
    // Pins come back after a restart.
    drop(a);
    let mut a = Engine::open("https://sync.kolada.app", "desktop", "PC", Some(dir.clone()), Some(&keys));
    let swapped = entry(&bid, &[7; 32], "");
    a.frame(Channel::Group, &Frame::new("devices").with("devices", swapped).encode(), T);
    assert_eq!(a.devices()[0].box_pub, b.box_pub(), "the pinned key, not the server's");
    let _ = std::fs::remove_dir_all(dir);
}

/// A device without a tag is trusted only with the key already pinned for
/// it: an untagged device the server adds is never trusted. A device away
/// across two re-keys keeps its pinned key, whatever its tag's age. What a
/// device says of sync counts only from a tag that checks: a server that
/// marks one `sync` without it changes nothing.
#[test]
fn trust_and_sync_come_only_from_the_group() {
    use kolada_sync::crypto::b64url;
    use kolada_sync::SyncCap;
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    a.set_sync(SyncCap::Follow);
    b.set_sync(SyncCap::Follow);
    pair(&mut a, &mut b);
    let aid = hex(&a.device_id());
    let me = format!("{aid}:desktop::{}:::follow:{}", b64url(&a.box_pub()), a.my_tag().unwrap());
    // An untagged ghost beside b: listed, never trusted.
    let bid = hex(&b.device_id());
    let real_b = format!("{bid}:desktop::{}:::follow:{}", b64url(&b.box_pub()), b.my_tag().unwrap());
    let ghost = DeviceKeysLike::new();
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me.clone(), real_b.clone(), format!("{}:desktop::{}", ghost.0, b64url(&ghost.1))].join(",")));
    assert!(a.devices().iter().find(|d| hex(&d.id) == ghost.0).is_some_and(|d| !d.trusted));
    // b listed as "sync" by the server, its tag still saying "follow": b's
    // word doesn't change, and two followers sync nothing.
    let forged = format!("{bid}:desktop::{}:::sync:{}", b64url(&b.box_pub()), b.my_tag().unwrap());
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me.clone(), forged].join(",")));
    let listed_b = a.devices().iter().find(|d| hex(&d.id) == bid).cloned().unwrap();
    assert!(listed_b.trusted, "its key is the one pinned");
    assert_eq!(listed_b.sync, SyncCap::Off, "a word its tag doesn't cover counts for nothing");
    assert!(a.sync_peers().is_empty());
    // With its own tag for "sync", it counts.
    b.set_sync(SyncCap::On);
    let real = format!("{bid}:desktop::{}:::sync:{}", b64url(&b.box_pub()), b.my_tag().unwrap());
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me, real].join(",")));
    assert_eq!(a.sync_peers(), vec![b.device_id()]);
    // Two re-keys while b was away: its tag is two secrets old, its key
    // the one pinned; it is still trusted, and gets the next re-key.
    let c1 = DeviceKeysLike::new();
    let c2 = DeviceKeysLike::new();
    a.remove(unhex10(&c1.0));
    a.remove(unhex10(&c2.0));
    out(&mut a, Channel::Group);
    let old = format!("{bid}:desktop::{}:::sync:{}", b64url(&b.box_pub()), b.my_tag().unwrap());
    let me_now = format!("{aid}:desktop::{}:::follow:{}", b64url(&a.box_pub()), a.my_tag().unwrap());
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me_now, old].join(",")));
    assert!(a.devices().iter().find(|d| hex(&d.id) == bid).is_some_and(|d| d.trusted), "away across two re-keys, still trusted");
    a.remove(unhex10(&ghost.0));
    let to: Vec<String> = out(&mut a, Channel::Group).into_iter().filter(|f| f.get("kind") == Some("k")).filter_map(|f| f.get("to").map(str::to_string)).collect();
    assert_eq!(to, vec![bid], "re-keyed to b, never to the ghost");
}

fn unhex10(s: &str) -> [u8; 10] {
    let mut out = [0u8; 10];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}

struct DeviceKeysLike(String, [u8; 32]);
impl DeviceKeysLike {
    fn new() -> Self {
        let k = kolada_sync::crypto::DeviceKeys::new();
        DeviceKeysLike(hex(&k.id()), k.box_pub())
    }
}

/// A group this device made, that nobody joined, is forgotten by the
/// server after a day: the device makes it again rather than unpairing.
/// A device whose group had others in it, told it is unknown, leaves.
#[test]
fn a_lone_creators_forgotten_group_is_made_again() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    let mut lone = engine("Desk", "desktop");
    let mut gone = engine("Phone", "ios");
    pair(&mut lone, &mut gone);
    // Only the creator listed: the other never registered.
    let only = [who(&lone)];
    hello(&mut lone, &only);
    out(&mut lone, Channel::Group);
    lone.closed(Channel::Group, T);
    lone.failed(Channel::Group, 401, "error=unknown\n", T);
    assert!(lone.paired(), "still paired");
    let c = lone.connects(T + 400_000);
    let g = c.iter().find(|c| c.channel == Channel::Group).expect("connects again");
    assert!(g.headers.iter().any(|(k, v)| k == "x-kolada-create" && v == "1"), "makes the group again");
    // With another device in its list, the same answer means it was removed.
    b.closed(Channel::Group, T);
    b.failed(Channel::Group, 401, "error=unknown\n", T);
    assert!(!b.paired());
}

/// A removed device's pinned key goes with it, and a device the list stops
/// naming loses its pin: listed again with its old key, by a server that
/// kept it, it is a stranger and gets no re-key.
#[test]
fn a_removed_device_listed_again_is_not_trusted() {
    use kolada_sync::crypto::b64url;
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    let mut c = engine("Old phone", "ios");
    pair(&mut a, &mut b);
    pair(&mut a, &mut c);
    let all = [who(&a), who(&b), who(&c)];
    hello(&mut a, &all);
    assert!(a.devices().iter().all(|d| d.trusted));
    let (bid, cid) = (hex(&b.device_id()), hex(&c.device_id()));
    let old_c = format!("{cid}:desktop::{}:::{}", b64url(&c.box_pub()), c.my_tag().unwrap().split_once('|').map_or(String::new(), |(_, t)| t.to_string()));
    a.remove(c.device_id());
    out(&mut a, Channel::Group);
    // The server lists c again, with its old key and old tag.
    let me = format!("{}:desktop::{}::::{}", hex(&a.device_id()), b64url(&a.box_pub()), a.my_tag().unwrap());
    let b_now = format!("{bid}:desktop::{}::::{}", b64url(&b.box_pub()), b.my_tag().unwrap());
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me.clone(), b_now.clone(), old_c.clone()].join(",")));
    assert!(a.devices().iter().find(|d| hex(&d.id) == cid).is_some_and(|d| !d.trusted), "removed: no longer pinned");
    let d = DeviceKeysLike::new();
    a.remove(unhex10(&d.0));
    let to: Vec<String> = out(&mut a, Channel::Group).into_iter().filter(|f| f.get("kind") == Some("k")).filter_map(|f| f.get("to").map(str::to_string)).collect();
    assert!(!to.contains(&cid), "no re-key to the removed device");
    // A device the list stops naming loses its pin too.
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", me.clone()));
    let b_old = format!("{bid}:desktop::{}::::", b64url(&b.box_pub()));
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me, b_old].join(",")));
    assert!(a.devices().iter().find(|d| hex(&d.id) == bid).is_some_and(|d| !d.trusted), "unlisted, then back untagged: a stranger");
}

/// Sync batches held waiting stay out of the state file, each in a file
/// of its own, so a big held sync never makes saving slow; past the cap the
/// oldest go and the next manifest exchange repairs them.
#[test]
fn held_sync_batches_stay_out_of_the_state_file_and_are_capped() {
    let dir = std::env::temp_dir().join(format!("kolada-sync-held-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut a = Engine::open("https://sync.kolada.app", "desktop", "PC", Some(dir.clone()), None);
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    let keys = a.keys_blob();
    a.closed(Channel::Group, T);
    let batch = vec![7u8; 50_000];
    for _ in 0..40 {
        a.send_sync_to(&batch, None, T).unwrap();
    }
    assert_eq!(a.sync_waiting(), 40);
    let state = std::fs::metadata(dir.join("quick-send.state")).map_or(0, |m| m.len());
    assert!(state < 64 * 1024, "the state file holds no sync batch: {state} bytes");
    assert_eq!(std::fs::read_dir(dir.join("sync-out")).unwrap().count(), 40, "a file each");
    // They come back after a restart, in order.
    drop(a);
    let mut a = Engine::open("https://sync.kolada.app", "desktop", "PC", Some(dir.clone()), Some(&keys));
    assert_eq!(a.sync_waiting(), 40);
    // Past the cap, the oldest go and a resync is asked for.
    assert!(!a.take_exchange().resync);
    let cap = a.sync_held_bytes() / 2;
    a.set_sync_held_most(cap);
    a.send_sync_to(&batch, None, T).unwrap();
    assert!(a.sync_held_bytes() <= cap && a.sync_waiting() < 41, "{} held over a cap of {cap}", a.sync_held_bytes());
    assert!(a.take_exchange().resync, "the manifest exchange repairs what went");
    assert_eq!(std::fs::read_dir(dir.join("sync-out")).unwrap().count(), a.sync_waiting());
    let _ = std::fs::remove_dir_all(dir);
}

/// A sync batch the server never answered goes again after a minute; one it
/// refuses for good (its size) goes, and a manifest exchange is asked for so
/// nothing it carried is lost.
#[test]
fn an_unanswered_batch_goes_again_and_a_refused_one_asks_for_a_resync() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    pair(&mut a, &mut b);
    out(&mut a, Channel::Group);
    let id = a.send_sync_to(b"batch", None, T).unwrap();
    let sends = |e: &mut Engine| out(e, Channel::Group).into_iter().filter(|f| f.t() == "send").count();
    assert_eq!(sends(&mut a), 1);
    a.tick(T + 30_000);
    assert_eq!(sends(&mut a), 0, "still waiting for its answer");
    assert!(a.next_wake().is_some_and(|t| t <= T + kolada_sync::SYNC_ANSWER_MS));
    a.tick(T + kolada_sync::SYNC_ANSWER_MS + 1);
    assert_eq!(sends(&mut a), 1, "no answer for a minute: it goes again");
    assert!(!a.take_exchange().resync);
    feed(&mut a, Channel::Group, Frame::new("error").with("id", hex(&id)).with("error", "size"));
    assert_eq!(a.sync_waiting(), 0, "refused for good: it goes");
    assert!(a.take_exchange().resync, "and a manifest exchange repairs it");
}

/// The server's list of a group as the sync server keeps it: each
/// device's kind, sealed name, box key and roster tag, as its frames set them.
struct Roster(Vec<(DeviceIdHex, String, String, [u8; 32], String)>);

impl Roster {
    fn of(es: &[&mut Engine]) -> Roster {
        Roster(es.iter().map(|e| (hex(&e.device_id()), String::new(), String::new(), e.box_pub(), e.my_tag().unwrap_or_default())).collect())
    }

    fn list(&self, es: &[&mut Engine]) -> Frame {
        let line = |(id, kind, name, bx, tag): &(DeviceIdHex, String, String, [u8; 32], String)| {
            let e = es.iter().find(|e| hex(&e.device_id()) == *id).unwrap();
            let kind = if kind.is_empty() { if e.name() == "iPhone" { "ios" } else { "desktop" } } else { kind.as_str() };
            format!("{id}:{kind}:{name}:{}:::{}:{tag}", kolada_sync::crypto::b64url(bx), e.sync_word())
        };
        Frame::new("devices").with("devices", self.0.iter().map(line).collect::<Vec<_>>().join(","))
    }

    /// Carries every engine's frames until none come: names, tags and
    /// removals change the list (sent to each device in it), re-keys go to
    /// the device they're sealed to.
    fn relay(&mut self, es: &mut [&mut Engine], seq: &mut u64) {
        for _ in 0..10 {
            let mut changed = false;
            for i in 0..es.len() {
                let from = hex(&es[i].device_id());
                for f in out(es[i], Channel::Group) {
                    match (f.t(), f.get("op")) {
                        ("device", Some("name")) => {
                            self.0.iter_mut().filter(|d| d.0 == from).for_each(|d| d.2 = f.get("name").unwrap().to_string());
                            changed = true;
                        }
                        ("device", Some("tag")) => {
                            self.0.iter_mut().filter(|d| d.0 == from).for_each(|d| d.4 = f.get("tag").unwrap().to_string());
                            changed = true;
                        }
                        ("device", Some("remove")) => {
                            self.0.retain(|d| Some(d.0.as_str()) != f.get("dev"));
                            changed = true;
                        }
                        ("send", _) if f.get("kind") == Some("k") => {
                            *seq += 1;
                            let to = es.iter_mut().find(|e| Some(hex(&e.device_id()).as_str()) == f.get("to")).unwrap();
                            let item = Frame::new("item").with("seq", *seq).with("id", f.get("id").unwrap()).with("kind", "k").with("from", from.as_str()).with("at", T).with("head", *seq).body(f.body.clone());
                            feed(to, Channel::Group, item);
                        }
                        _ => {}
                    }
                }
            }
            if !changed {
                return;
            }
            let list = self.list(es);
            for e in es.iter_mut().filter(|e| self.0.iter().any(|d| d.0 == hex(&e.device_id()))) {
                feed(e, Channel::Group, list.clone());
            }
        }
        panic!("frames never stopped");
    }
}

fn name_of(e: &Engine, other: &Engine) -> String {
    e.devices().iter().find(|d| d.id == other.device_id()).map(|d| d.name.clone()).unwrap_or_default()
}

/// Removing a device re-keys the group, and with it the key names are
/// sealed under: every device left seals its name again under the new key,
/// a name still sealed under the key before reads with that one, and a name
/// that can't be read shows its device's kind, never a blank row.
#[test]
fn removing_one_device_keeps_the_others_names() {
    let mut a = engine("PC", "desktop");
    let mut b = engine("Mac", "desktop");
    let mut c = engine("iPhone", "ios");
    pair(&mut a, &mut b);
    pair(&mut a, &mut c);
    // Each tells the server its name, as on its first connection.
    for e in [&mut a, &mut b, &mut c] {
        let n = e.name().to_string();
        e.rename_self(&n);
    }
    let mut seq = 0;
    let mut roster = Roster::of(&[&mut a, &mut b, &mut c]);
    roster.relay(&mut [&mut a, &mut b, &mut c], &mut seq);
    assert_eq!((name_of(&a, &b), name_of(&a, &c), name_of(&b, &a), name_of(&b, &c), name_of(&c, &a), name_of(&c, &b)),
        ("Mac".into(), "iPhone".into(), "PC".into(), "iPhone".into(), "PC".into(), "Mac".into()));
    let old_a = roster.0.iter().find(|d| d.0 == hex(&a.device_id())).unwrap().clone();

    a.remove(c.device_id());
    roster.relay(&mut [&mut a, &mut b, &mut c], &mut seq);
    assert_eq!(roster.0.len(), 2);
    assert_eq!((name_of(&a, &b), name_of(&b, &a)), ("Mac".into(), "PC".into()), "both still named on both");
    assert!(roster.0.iter().all(|d| !d.2.is_empty() && d.2 != old_a.2), "each sealed its name again");

    // A name sealed under the key before the re-key (a device that hasn't
    // sealed it again yet) still reads, on a device that knew no name for it.
    let bid = hex(&b.device_id());
    let me_b = roster.list(&[&mut a, &mut b]).get("devices").unwrap().split(',').find(|l| l.starts_with(&bid)).unwrap().to_string();
    feed(&mut b, Channel::Group, Frame::new("devices").with("devices", me_b.as_str()));
    let stale = |name: &str| format!("{}:desktop:{name}:{}:::{}:{}", old_a.0, kolada_sync::crypto::b64url(&old_a.3), a.sync_word(), old_a.4);
    feed(&mut b, Channel::Group, Frame::new("devices").with("devices", [me_b.clone(), stale(&old_a.2)].join(",")));
    assert_eq!(name_of(&b, &a), "PC", "read with the key before");
    // One no key opens shows its kind.
    feed(&mut b, Channel::Group, Frame::new("devices").with("devices", me_b.as_str()));
    feed(&mut b, Channel::Group, Frame::new("devices").with("devices", [me_b.clone(), stale("AAAA")].join(",")));
    assert_eq!(name_of(&b, &a), "Computer", "never blank");
}

/// A device says what it is once a connection, where the list doesn't say
/// it; the others draw and name it by that, and a name that can't be read
/// reads as its model's ("iPad"), never blank.
#[test]
fn a_device_says_what_it_is_and_is_named_by_it() {
    use kolada_sync::crypto::b64url;
    let mut a = engine("PC", "desktop");
    let mut b = engine("Tablet", "ios");
    pair(&mut a, &mut b);
    out(&mut a, Channel::Group);
    a.set_model("windows-laptop");
    let said: Vec<Frame> = out(&mut a, Channel::Group).into_iter().filter(|f| f.get("op") == Some("model")).collect();
    assert_eq!(said.len(), 1, "told at once, while connected");
    assert_eq!(said[0].get("model"), Some("windows-laptop"));
    a.set_model("a toaster");
    assert_eq!(a.model(), "windows-laptop", "only the models it knows");
    // b, an iPad whose sealed name doesn't open here.
    let me = format!("{}:desktop::{}:::{}:{}:windows-laptop", hex(&a.device_id()), b64url(&a.box_pub()), a.sync_word(), a.my_tag().unwrap());
    let ipad = format!("{}:ios:AAAA:{}:::{}:{}:ipad", hex(&b.device_id()), b64url(&b.box_pub()), b.sync_word(), b.my_tag().unwrap());
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", [me.clone(), ipad].join(",")));
    let d = a.devices().iter().find(|d| d.id == b.device_id()).unwrap().clone();
    assert_eq!((d.name.as_str(), d.model.as_str()), ("iPad", "ipad"));
    assert!(!out(&mut a, Channel::Group).iter().any(|f| f.get("op") == Some("model")), "the list says it: nothing to tell");
    // A server from before lists no model: told once this connection, not on every list.
    let old = format!("{}:desktop::{}:::{}:{}", hex(&a.device_id()), b64url(&a.box_pub()), a.sync_word(), a.my_tag().unwrap());
    a.closed(Channel::Group, T);
    a.opened(Channel::Group, T);
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", old.as_str()));
    feed(&mut a, Channel::Group, Frame::new("devices").with("devices", old.as_str()));
    assert_eq!(out(&mut a, Channel::Group).iter().filter(|f| f.get("op") == Some("model")).count(), 1);
}

/// The name the user gives this device is sealed and sent at once, and
/// kept: it outlives a restart and wins over the shell's default; an
/// empty one goes back to the default.
#[test]
fn a_name_the_user_gives_is_sent_and_kept() {
    let dir = std::env::temp_dir().join(format!("kolada-sync-named-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut a = Engine::open("https://sync.kolada.app", "desktop", "Windows laptop", Some(dir.clone()), None);
    let mut b = engine("iPhone", "ios");
    pair(&mut a, &mut b);
    out(&mut a, Channel::Group);
    a.rename_self("  Kitchen laptop ");
    assert_eq!(a.name(), "Kitchen laptop");
    assert!(out(&mut a, Channel::Group).iter().any(|f| f.get("op") == Some("name")), "sealed and sent now");
    let keys = a.keys_blob();
    drop(a);
    let mut a = Engine::open("https://sync.kolada.app", "desktop", "Windows laptop", Some(dir.clone()), Some(&keys));
    assert_eq!(a.name(), "Kitchen laptop", "kept over the default");
    a.rename_self("");
    assert_eq!(a.name(), "Windows laptop", "empty: the default again");
    drop(a);
    let a = Engine::open("https://sync.kolada.app", "desktop", "Windows PC", Some(dir.clone()), Some(&keys));
    assert_eq!(a.name(), "Windows PC");
    let _ = std::fs::remove_dir_all(dir);
}
