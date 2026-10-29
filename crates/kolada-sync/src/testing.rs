//! Engines paired with no network, for the tests of what drives an engine
//! (kolada-app's popup): a stand-in for the server's part of pairing and
//! delivery, as tests/engine.rs does it step by step with its checks.

use crate::crypto::{b64url, hex};
use crate::frame::Frame;
use crate::{Channel, Engine, Event};

/// The time the stand-in server says it is, ms.
pub const T: u64 = 1_800_000_000_000;

fn out(e: &mut Engine, ch: Channel) -> Vec<Frame> {
    e.outgoing().into_iter().filter(|(c, _)| *c == ch).filter_map(|(_, b)| Frame::parse(&b)).collect()
}

fn all(e: &mut Engine) -> Vec<(Channel, Frame)> {
    e.outgoing().into_iter().filter_map(|(c, b)| Some((c, Frame::parse(&b)?))).collect()
}

fn feed(e: &mut Engine, ch: Channel, f: Frame) {
    e.frame(ch, &f.encode(), T);
}

/// What the server lists for a device: id, box key, kind, its sealed
/// name (empty until it sent one), and whether it syncs notes.
type Listed = (String, [u8; 32], String, String, &'static str, String);

fn hello(e: &mut Engine, devices: &[Listed]) {
    let list: Vec<String> = devices.iter().map(|(id, b, kind, name, caps, tag)| format!("{id}:{kind}:{name}:{}:::{caps}:{tag}", b64url(b))).collect();
    e.opened(Channel::Group, T);
    feed(e, Channel::Group, Frame::new("hello").with("cursor", 0).with("head", 0).with("backlog", 0).with("every", 10).with("devices", list.join(",")));
}

/// Pairs `b` into `a`'s group (made if it has none), as the server would
/// relay it, and says hello to both. Their events are left for the caller.
pub fn pair(a: &mut Engine, a_kind: &str, b: &mut Engine, b_kind: &str) {
    a.show_code();
    a.connects(T);
    a.opened(Channel::Pair, T);
    feed(a, Channel::Pair, Frame::new("slot").with("slot", "F4F").with("expires", T + 300_000));
    let code = a.events().into_iter().find_map(|e| match e {
        Event::Code { code, .. } => Some(code),
        _ => None,
    });
    let pake = out(a, Channel::Pair).into_iter().find(|f| f.t() == "pake");
    let (Some(code), Some(pake)) = (code, pake) else { return };
    if b.enter_code(&code).is_err() {
        return;
    }
    b.connects(T);
    b.opened(Channel::Pair, T);
    feed(b, Channel::Pair, Frame::new("pake").body(pake.body));
    let Some(confirm) = out(b, Channel::Pair).into_iter().find(|f| f.t() == "confirm") else { return };
    feed(a, Channel::Pair, confirm);
    a.allow(true, T);
    let mut both = [(hex(&a.device_id()), a.box_pub(), a_kind.to_string(), String::new(), a.sync_word(), a.my_tag().unwrap_or_default()), (hex(&b.device_id()), b.box_pub(), b_kind.to_string(), String::new(), b.sync_word(), String::new())];
    // Each sends its name, sealed for the group, once it is in: the server
    // lists it, and says hello again.
    let name = |frames: &[(Channel, Frame)]| frames.iter().find(|(c, f)| *c == Channel::Group && f.t() == "device" && f.get("op") == Some("name")).and_then(|(_, f)| f.get("name").map(str::to_string)).unwrap_or_default();
    hello(a, &both);
    let from_a = all(a);
    both[0].3 = name(&from_a);
    let Some((_, welcome)) = from_a.into_iter().find(|(c, f)| *c == Channel::Pair && f.t() == "welcome") else { return };
    feed(b, Channel::Pair, welcome);
    b.connects(T);
    both[1].5 = b.my_tag().unwrap_or_default();
    hello(b, &both);
    both[1].3 = name(&all(b));
    hello(a, &both);
    hello(b, &both);
}

/// Relays `from`'s last send to `to` as the server's item number `seq`:
/// whether there was one.
pub fn deliver(from: &mut Engine, to: &mut Engine, seq: u64) -> bool {
    deliver_at(from, to, seq, T)
}

/// The same, stamped `at` (ms) by the server.
pub fn deliver_at(from: &mut Engine, to: &mut Engine, seq: u64, at: u64) -> bool {
    let Some(send) = out(from, Channel::Group).into_iter().find(|f| f.t() == "send") else { return false };
    let (Some(id), Some(kind)) = (send.get("id").map(str::to_string), send.get("kind").map(str::to_string)) else { return false };
    let item = Frame::new("item").with("seq", seq).with("id", id).with("kind", kind).with("from", hex(&from.device_id())).with("at", at).with("head", seq).body(send.body.clone());
    feed(to, Channel::Group, item);
    true
}

/// Everything `from` sent on the group socket, carried to `to` as the
/// server would: items (numbered on from `seq`, which moves on) and relayed
/// manifest parts. How many sends and relays went.
pub fn carry(from: &mut Engine, to: &mut Engine, seq: &mut u64) -> (usize, usize) {
    let (mut sends, mut relays) = (0, 0);
    for f in out(from, Channel::Group) {
        match f.t() {
            "send" => {
                *seq += 1;
                let (id, kind) = (f.get("id").unwrap_or(""), f.get("kind").unwrap_or(""));
                feed(to, Channel::Group, Frame::new("item").with("seq", *seq).with("id", id).with("kind", kind).with("from", hex(&from.device_id())).with("at", T).with("head", *seq).body(f.body.clone()));
                sends += 1;
            }
            "to" => {
                feed(to, Channel::Group, Frame::new("from").with("dev", hex(&from.device_id())).body(f.body.clone()));
                relays += 1;
            }
            _ => {}
        }
    }
    (sends, relays)
}

/// As `carry`, with the server's answer to each send ("sent") back to
/// `from`, as the real server gives it. How many sends went.
pub fn carry_answered(from: &mut Engine, to: &mut Engine, seq: &mut u64) -> usize {
    let mut sends = 0;
    for f in out(from, Channel::Group) {
        match f.t() {
            "send" => {
                *seq += 1;
                let (id, kind) = (f.get("id").unwrap_or("").to_string(), f.get("kind").unwrap_or("").to_string());
                feed(to, Channel::Group, Frame::new("item").with("seq", *seq).with("id", id.as_str()).with("kind", kind.as_str()).with("from", hex(&from.device_id())).with("at", T).with("head", *seq).body(f.body.clone()));
                feed(from, Channel::Group, Frame::new("sent").with("id", id.as_str()).with("seq", *seq));
                sends += 1;
            }
            "to" => feed(to, Channel::Group, Frame::new("from").with("dev", hex(&from.device_id())).body(f.body.clone())),
            _ => {}
        }
    }
    sends
}

/// Everything `from` sent carried to `to`, pictures the whole way (their
/// tickets, uploads, items and downloads), as the server and both shells
/// would. How many items went.
pub fn carry_all(from: &mut Engine, to: &mut Engine, seq: &mut u64) -> usize {
    let ticket = |f: &Frame| Frame::new("ticket").with("id", f.get("id").unwrap_or("")).with("op", f.get("op").unwrap_or("")).with("exp", T + 600_000).with("sig", "test");
    let mut up: Vec<(crate::ItemId, Vec<u8>)> = Vec::new();
    let mut items = 0;
    for _ in 0..8 {
        for f in out(from, Channel::Group) {
            match f.t() {
                "send" => {
                    *seq += 1;
                    let (id, kind) = (f.get("id").unwrap_or(""), f.get("kind").unwrap_or(""));
                    let mut item = Frame::new("item").with("seq", *seq).with("id", id).with("kind", kind).with("from", hex(&from.device_id())).with("at", T).with("head", *seq);
                    if let Some(b) = f.get("blob") {
                        item = item.with("blob", b);
                    }
                    feed(to, Channel::Group, item.body(f.body.clone()));
                    items += 1;
                }
                "ticket" => feed(from, Channel::Group, ticket(&f)),
                _ => {}
            }
        }
        for t in from.transfers() {
            if let crate::Transfer::Put { blob, body, .. } = t {
                up.push((blob, body));
                from.put_done(blob, 200, T);
            }
        }
        for f in out(to, Channel::Group).into_iter().filter(|f| f.t() == "ticket") {
            feed(to, Channel::Group, ticket(&f));
        }
        for t in to.transfers() {
            if let crate::Transfer::Get { blob, .. } = t {
                let body = up.iter().find(|(b, _)| *b == blob).map(|(_, body)| body.clone());
                to.got(blob, body);
            }
        }
    }
    items
}

/// What `from` sent, as the server would hand it on: each item's frame
/// (numbered on from `seq`), and each picture's sealed bytes by blob, its
/// tickets and uploads done. For an engine a shell drives (a model's), fed
/// with `Engine::frame` and `Engine::got`.
pub fn items_from(from: &mut Engine, seq: &mut u64) -> (Vec<Vec<u8>>, Vec<(crate::ItemId, Vec<u8>)>) {
    let ticket = |f: &Frame| Frame::new("ticket").with("id", f.get("id").unwrap_or("")).with("op", f.get("op").unwrap_or("")).with("exp", T + 600_000).with("sig", "test");
    let (mut items, mut up) = (Vec::new(), Vec::new());
    for _ in 0..8 {
        for f in out(from, Channel::Group) {
            match f.t() {
                "send" => {
                    *seq += 1;
                    let (id, kind) = (f.get("id").unwrap_or(""), f.get("kind").unwrap_or(""));
                    let mut item = Frame::new("item").with("seq", *seq).with("id", id).with("kind", kind).with("from", hex(&from.device_id())).with("at", T).with("head", *seq);
                    if let Some(b) = f.get("blob") {
                        item = item.with("blob", b);
                    }
                    items.push(item.body(f.body.clone()).encode());
                }
                "ticket" => feed(from, Channel::Group, ticket(&f)),
                _ => {}
            }
        }
        for t in from.transfers() {
            if let crate::Transfer::Put { blob, body, .. } = t {
                up.push((blob, body));
                from.put_done(blob, 200, T);
            }
        }
    }
    (items, up)
}

/// Sends a picture from `from` to `to` the whole way, as the server and
/// both shells would: the ticket, the upload, the item (number `seq`,
/// stamped `at`), then the download. The card's id, if it went.
pub fn deliver_picture(from: &mut Engine, to: &mut Engine, seq: u64, what: crate::Outgoing, at: u64) -> Option<crate::ItemId> {
    let ticket = |f: &Frame| Frame::new("ticket").with("id", f.get("id").unwrap_or("")).with("op", f.get("op").unwrap_or("")).with("exp", T + 600_000).with("sig", "test");
    let id = from.send(what, at)?;
    let asked = out(from, Channel::Group).into_iter().find(|f| f.t() == "ticket")?;
    feed(from, Channel::Group, ticket(&asked));
    let (blob, body) = match from.transfers().pop()? {
        crate::Transfer::Put { blob, body, .. } => (blob, body),
        crate::Transfer::Get { .. } => return None,
    };
    from.put_done(blob, 200, at);
    deliver_at(from, to, seq, at).then_some(())?;
    let asked = out(to, Channel::Group).into_iter().find(|f| f.t() == "ticket")?;
    feed(to, Channel::Group, ticket(&asked));
    to.transfers();
    to.got(blob, Some(body));
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kind, Outgoing};

    #[test]
    fn pairs_two_engines_and_delivers_between_them() {
        let mut a = Engine::open("https://sync.kolada.app", "desktop", "PC", None, None);
        let mut b = Engine::open("https://sync.kolada.app", "desktop", "Mac", None, None);
        pair(&mut a, "desktop", &mut b, "desktop");
        assert!(a.paired() && b.paired());
        assert_eq!(a.devices().len(), 2);
        assert!(a.devices().iter().any(|d| d.name == "Mac") && b.devices().iter().any(|d| d.name == "PC"), "names are relayed");
        a.events();
        let id = a.send(Outgoing::Text("Gate B, row 14".into()), T).unwrap();
        assert!(deliver(&mut a, &mut b, 1));
        assert!(b.events().contains(&Event::Arrived(id)));
        assert_eq!(b.cards()[0].kind, Kind::Text);
        let pic = deliver_picture(&mut a, &mut b, 2, Outgoing::Picture { bytes: vec![1, 2, 3], mime: "image/png".into(), name: "x.png".into(), width: 1, height: 1, thumb: vec![] }, T).expect("a picture went");
        assert_eq!(b.picture(pic), Some(vec![1, 2, 3]));
    }

    /// A code shown runs out after its five minutes: pairing is idle again,
    /// so its QR goes with it, and the reason is said.
    #[test]
    fn a_code_that_runs_out_is_gone() {
        let mut a = Engine::open("https://sync.kolada.app", "desktop", "PC", None, None);
        a.show_code();
        a.connects(T);
        a.opened(Channel::Pair, T);
        feed(&mut a, Channel::Pair, Frame::new("slot").with("slot", "F4F").with("expires", T + 300_000));
        assert!(matches!(a.pair_state(), crate::PairState::Showing { .. }));
        a.tick(T + 299_000);
        assert!(matches!(a.pair_state(), crate::PairState::Showing { .. }), "not yet");
        a.tick(T + 300_000);
        assert!(!matches!(a.pair_state(), crate::PairState::Showing { .. }), "gone");
        assert!(a.events().contains(&Event::CodeRefused(crate::Refusal::Expired)));
    }

    /// A push token given while no socket is open is not lost: it goes on
    /// the next connection, and again on every connection after.
    #[test]
    fn a_push_token_given_offline_goes_on_the_next_connection() {
        let mut a = Engine::open("https://sync.kolada.app", "ios", "iPhone", None, None);
        let mut b = Engine::open("https://sync.kolada.app", "desktop", "PC", None, None);
        pair(&mut b, "desktop", &mut a, "ios");
        let pushes = |e: &mut Engine| out(e, Channel::Group).into_iter().filter(|f| f.t() == "device" && f.get("op") == Some("push")).count();
        pushes(&mut a);
        a.closed(Channel::Group, T);
        a.push_token(&"ab".repeat(32), false);
        assert_eq!(pushes(&mut a), 0, "no socket: kept, not sent");
        a.opened(Channel::Group, T);
        feed(&mut a, Channel::Group, Frame::new("hello").with("cursor", 0).with("head", 0).with("backlog", 0).with("every", 10));
        assert_eq!(pushes(&mut a), 1, "sent on connecting");
        a.closed(Channel::Group, T);
        a.opened(Channel::Group, T);
        feed(&mut a, Channel::Group, Frame::new("hello").with("cursor", 0).with("head", 0).with("backlog", 0).with("every", 10));
        assert_eq!(pushes(&mut a), 1, "and on every connection");
        a.push_token(&"cd".repeat(32), false);
        let f = out(&mut a, Channel::Group).into_iter().find(|f| f.get("op") == Some("push")).expect("connected: at once");
        assert_eq!((f.get("token"), f.get("env")), (Some("cd".repeat(32).as_str()), Some("production")));
    }

    /// The share extension, a process with no socket and the app's keys:
    /// a picture goes by requests alone (its ticket, the upload, its item),
    /// and what it sent is handed to the app, which lists it as its own,
    /// once, picture and all.
    #[test]
    fn the_share_extension_beams_a_picture_by_requests_and_hands_it_to_the_app() {
        let mut app = Engine::open("https://sync.kolada.app", "ios", "iPhone", None, None);
        let mut pc = Engine::open("https://sync.kolada.app", "desktop", "PC", None, None);
        pair(&mut pc, "desktop", &mut app, "ios");
        let mut ext = Engine::open("https://sync.kolada.app", "ios", "iPhone", None, Some(&app.keys_blob()));
        assert!(ext.paired());
        let bytes: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let what = Outgoing::Picture { bytes: bytes.clone(), mime: "image/png".into(), name: "Screenshot.png".into(), width: 40, height: 30, thumb: vec![9; 10] };
        let (card, ask) = ext.post_picture(what, T).expect("a ticket asked for");
        assert!(ask.url.contains("/ticket/") && ask.body.starts_with(b"put ") && ask.headers.iter().any(|(k, _)| k == "x-kolada-sig"));
        let size: usize = std::str::from_utf8(&ask.body[4..]).unwrap().parse().unwrap();
        assert!(ext.outgoing().is_empty(), "nothing waits on a socket");
        // The server's answer: the frame a socket would get.
        let blob = ask.url.rsplit('/').next().unwrap().to_string();
        ext.post_answer(&Frame::new("ticket").with("id", blob.as_str()).with("op", "put").with("exp", T + 600_000).with("sig", "test").encode());
        let crate::Transfer::Put { blob: b, body, url } = ext.transfers().pop().expect("the upload") else { panic!("a put") };
        assert!(url.contains("sig=test"));
        assert_eq!(body.len(), size, "the size the ticket was asked for");
        let item = ext.put_posted(b, 200, T).expect("the item's request");
        assert!(item.url.ends_with("/send"));
        // As the server relays it to the PC, which fetches the picture.
        let send = Frame::parse(&item.body).unwrap();
        feed(&mut pc, Channel::Group, Frame::new("item").with("seq", 1).with("id", send.get("id").unwrap()).with("kind", "q").with("from", hex(&app.device_id())).with("at", T).with("head", 1).with("blob", blob.as_str()).body(send.body.clone()));
        assert!(pc.events().contains(&Event::Arrived(card)));
        pc.got(b, Some(body));
        assert_eq!(pc.picture(card), Some(bytes.clone()));
        // Handed to the app: listed as its own, once.
        let h = ext.handoff(card).expect("a handoff");
        assert_eq!(app.take_handoff(&h), Some(card));
        assert_eq!(app.take_handoff(&h), None, "once");
        let cards = app.cards();
        let c = cards.iter().find(|c| c.id == card).unwrap();
        assert!(c.mine && c.kind == crate::Kind::Picture);
        assert_eq!(app.picture(card), Some(bytes));
        // Text from the extension, too; another device's keys can't open it.
        let (text, _) = ext.send_post(Outgoing::Text("Gate B, row 14".into()), T + 1).unwrap();
        let h = ext.handoff(text).unwrap();
        assert_eq!(pc.take_handoff(&h), None, "sealed for this device");
        assert_eq!(app.take_handoff(&h), Some(text));
        assert_eq!(app.copy(text).as_deref(), Some("Gate B, row 14"));
    }

    /// A note goes as its Markdown under its name, and each picture it embeds
    /// goes with it, tied to it; the other device lists the pictures as the
    /// note's. Too long a note sends nothing; a resend sends it all again.
    #[test]
    fn a_note_goes_with_its_pictures() {
        use crate::{Kind, NotePicture, Outgoing};
        let mut a = Engine::open("https://sync.kolada.app", "desktop", "PC", None, None);
        let mut b = Engine::open("https://sync.kolada.app", "desktop", "Mac", None, None);
        pair(&mut a, "desktop", &mut b, "desktop");
        a.events();
        b.events();
        let pic = |name: &str, fill: u8| NotePicture { name: name.into(), mime: "image/png".into(), bytes: vec![fill; 3000], width: 4, height: 3, thumb: vec![] };
        let text = "# Trip\n![[photo.png]]\n![map](maps/route.png)\n".to_string();
        let id = a.send(Outgoing::Note { title: "Trip".into(), text: text.clone(), pictures: vec![pic("photo.png", 1), pic("maps/route.png", 2)] }, T).unwrap();
        let mut seq = 0;
        assert_eq!(carry_all(&mut a, &mut b, &mut seq), 3, "the note and its two pictures");
        let note = b.cards().into_iter().find(|c| c.id == id).cloned().expect("the note came");
        assert_eq!((note.kind, note.title.as_str(), note.text.as_str()), (Kind::Text, "Trip", text.as_str()));
        let with: Vec<(String, Option<Vec<u8>>)> = b.note_pictures(id).iter().map(|c| (c.text.clone(), b.picture(c.id))).collect();
        assert_eq!(with, vec![("photo.png".to_string(), Some(vec![1; 3000])), ("maps/route.png".to_string(), Some(vec![2; 3000]))]);
        // Too long: nothing goes, not even its pictures.
        let long: String = (0..12_000).flat_map(|_| crate::crypto::random::<32>()).map(|b| char::from(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"[(b & 63) as usize])).collect();
        assert!(a.send(Outgoing::Note { title: "Long".into(), text: long, pictures: vec![pic("x.png", 3)] }, T).is_none());
        assert!(out(&mut a, Channel::Group).is_empty());
        // Sent again (it didn't arrive), it goes whole: note and pictures.
        let again = a.resend(id, T + 1).expect("again");
        assert_eq!(a.note_pictures(again).len(), 2);
        assert!(a.cards().iter().all(|c| c.id != id && c.picture.as_ref().is_none_or(|p| p.note != Some(id))), "the old ones go");
    }
}
