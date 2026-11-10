//! The extension's device: Beam's engine, paired by code as any device is,
//! that sends caught pages by requests (it keeps no socket open), and the
//! page reading, behind one call that takes a frame and gives frames.

use kolada_sync::frame::Frame;
use kolada_sync::{Channel, Engine, Event, NotePicture, Outgoing, Transfer};

use crate::clip::{self, Mode};

/// What a request gives back: frames, each with its length first.
fn pack(frames: &[Frame]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in frames {
        let b = f.encode();
        out.extend_from_slice(&(b.len() as u32).to_be_bytes());
        out.extend_from_slice(&b);
    }
    out
}

fn channel(word: Option<&str>) -> Channel {
    if word == Some("pair") { Channel::Pair } else { Channel::Group }
}

fn word(ch: Channel) -> &'static str {
    match ch {
        Channel::Pair => "pair",
        Channel::Group => "group",
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex16(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The extension's device, until `open`: unpaired, on the production server.
pub struct Catch {
    engine: Engine,
}

impl Default for Catch {
    fn default() -> Self {
        Self::new()
    }
}

impl Catch {
    pub fn new() -> Catch {
        Catch { engine: Engine::open(kolada_sync::SERVER, "other", "Browser", None, None) }
    }

    pub fn engine(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// One request, `t` naming it; the time is its `now` (ms). Every
    /// answer ends with what the engine has to say: frames for the sockets
    /// (`out`, by channel), sockets to open (`connect`), what happened
    /// (`event`), and where it stands (`state`). Requests:
    ///
    /// - `open` (`name`, `base` for a test server; body: the keys kept, or
    ///   empty): the device, as it was left.
    /// - `pair` (`code`): joins with a code Kolada shows. `cancel`.
    /// - `opened`, `closed`, `failed` (`ch`, `status`; body: the server's
    ///   words), `frame` (`ch`; body: the bytes), `tick`: the sockets.
    /// - `extract` (`mode`: page, selection, link; `url`, `title`; body:
    ///   the page's or the selection's HTML, then its plain words after a
    ///   NUL): the page read as a note: `clipped` (`title`; body: its
    ///   Markdown), a `fetch` (`name`, `url`) for each picture from
    ///   elsewhere, an `inside` (`name`; body: its bytes) for each in it.
    /// - `riding` (`names`: `name:size` comma-separated, sizes of the
    ///   pictures whose bytes are here): `riding` (`names`) those that go.
    /// - `compose` (`title`, `url`, `line`, `caught`, `rides`: names
    ///   comma-separated, `links`: `name>url` comma-separated; body: the
    ///   Markdown): `composed` (`renames`: `old>new` comma-separated; body:
    ///   the note whole).
    /// - `send` (`title`, `url`; body: the note whole): the clip by one
    ///   request: `post` (`id`, `url`, headers as `h:name`; body).
    /// - `picture` (`note`, `name`, `mime`, `width`, `height`, `thumb`:
    ///   the thumbnail's length; body: thumbnail then picture): `post` for
    ///   its ticket (`card`).
    /// - `answer` (body: the ticket's answer): `put` (`blob`, `url`; body:
    ///   the sealed bytes) for each upload ready.
    /// - `put_done` (`blob`, `status`): at 200, `post` for the picture's item.
    pub fn call(&mut self, req: &[u8]) -> Vec<u8> {
        let Some(f) = Frame::parse(req) else { return pack(&[Frame::new("error").with("why", "frame")]) };
        let now = f.num("now").unwrap_or(0);
        let mut out = Vec::new();
        match f.t() {
            "open" => {
                let base = f.get("base").filter(|b| b.starts_with("http")).unwrap_or(kolada_sync::SERVER);
                let keys = (!f.body.is_empty()).then_some(f.body.as_slice());
                self.engine = Engine::open(base, "other", f.get("name").unwrap_or("Browser"), None, keys);
            }
            "pair" => {
                if self.engine.enter_code(f.get("code").unwrap_or("")).is_err() {
                    out.push(Frame::new("event").with("what", "refused").with("why", "WrongCode"));
                }
            }
            "cancel" => self.engine.cancel_pairing(),
            "opened" => self.engine.opened(channel(f.get("ch")), now),
            "closed" => self.engine.closed(channel(f.get("ch")), now),
            "failed" => self.engine.failed(channel(f.get("ch")), f.num("status").unwrap_or(0) as u16, &String::from_utf8_lossy(&f.body), now),
            "frame" => self.engine.frame(channel(f.get("ch")), &f.body, now),
            "tick" => self.engine.tick(now),
            "extract" => {
                let (html, text) = match f.body.iter().position(|b| *b == 0) {
                    Some(at) => (&f.body[..at], &f.body[at + 1..]),
                    None => (f.body.as_slice(), &[][..]),
                };
                let c = clip::extract(Mode::of(f.get("mode").unwrap_or("page")), &String::from_utf8_lossy(html), &String::from_utf8_lossy(text), f.get("url").unwrap_or(""), f.get("title").unwrap_or(""));
                out.push(Frame::new("clipped").with("title", &c.title).body(c.markdown.into_bytes()));
                for (name, url) in c.fetch {
                    out.push(Frame::new("fetch").with("name", name).with("url", url));
                }
                for (name, bytes) in c.inside {
                    out.push(Frame::new("inside").with("name", name).body(bytes));
                }
            }
            "riding" => {
                let have: Vec<(String, usize)> = list(f.get("names")).filter_map(|p| p.rsplit_once(':').map(|(n, s)| (n.to_string(), s.parse().unwrap_or(0)))).collect();
                out.push(Frame::new("riding").with("names", clip::riding(&have).join(",")));
            }
            "compose" => {
                let rides: Vec<String> = list(f.get("rides")).map(str::to_string).collect();
                let links: Vec<(String, String)> = list(f.get("links")).filter_map(|p| p.split_once('>').map(|(n, u)| (n.to_string(), u.to_string()))).collect();
                let (text, renames) = clip::compose(f.get("title").unwrap_or(""), f.get("url").unwrap_or(""), f.get("line").unwrap_or(""), f.get("caught").unwrap_or(""), &String::from_utf8_lossy(&f.body), &rides, &links);
                out.push(Frame::new("composed").with("renames", renames.iter().map(|(o, n)| format!("{o}>{n}")).collect::<Vec<_>>().join(",")).body(text.into_bytes()));
            }
            "send" => {
                let what = Outgoing::Clip { title: f.get("title").unwrap_or("").into(), source: f.get("url").unwrap_or("").into(), text: String::from_utf8_lossy(&f.body).into_owned(), pictures: Vec::new() };
                match self.engine.send_post(what, now) {
                    Some((id, post)) => out.push(post_frame(post).with("id", hex(&id))),
                    None => out.push(Frame::new("error").with("why", if self.engine.paired() { "size" } else { "unpaired" })),
                }
            }
            "picture" => {
                let n = (f.num("thumb").unwrap_or(0) as usize).min(f.body.len());
                let p = NotePicture { name: f.get("name").unwrap_or("picture.png").into(), mime: f.get("mime").unwrap_or("image/png").into(), width: f.num("width").unwrap_or(0) as u32, height: f.num("height").unwrap_or(0) as u32, thumb: f.body[..n].to_vec(), bytes: f.body[n..].to_vec() };
                match f.get("note").and_then(unhex16).and_then(|note| self.engine.post_picture_to(note, p, now)) {
                    Some((card, post)) => out.push(post_frame(post).with("card", hex(&card))),
                    None => out.push(Frame::new("error").with("why", "picture")),
                }
            }
            "answer" => {
                self.engine.post_answer(&f.body);
                for t in self.engine.transfers() {
                    if let Transfer::Put { blob, url, body } = t {
                        out.push(Frame::new("put").with("blob", hex(&blob)).with("url", url).body(body));
                    }
                }
            }
            "put_done" => {
                if let Some(post) = f.get("blob").and_then(unhex16).and_then(|b| self.engine.put_posted(b, f.num("status").unwrap_or(0) as u16, now)) {
                    out.push(post_frame(post));
                }
            }
            _ => out.push(Frame::new("error").with("why", "unknown")),
        }
        self.after(now, &mut out);
        pack(&out)
    }

    /// What the engine has to say after a request.
    fn after(&mut self, now: u64, out: &mut Vec<Frame>) {
        for c in self.engine.connects(now) {
            let mut f = Frame::new("connect").with("ch", word(c.channel)).with("url", c.url);
            for (k, v) in c.headers {
                f = f.with(&format!("h:{k}"), v);
            }
            out.push(f);
        }
        for (ch, bytes) in self.engine.outgoing() {
            out.push(Frame::new("out").with("ch", word(ch)).body(bytes));
        }
        for e in self.engine.events() {
            match e {
                Event::Keys(k) => out.push(Frame::new("keys").body(k)),
                Event::Paired => out.push(Frame::new("event").with("what", "paired")),
                Event::CodeRefused(r) => out.push(Frame::new("event").with("what", "refused").with("why", format!("{r:?}"))),
                Event::Removed => out.push(Frame::new("event").with("what", "removed")),
                Event::Sent(id) => out.push(Frame::new("event").with("what", "sent").with("id", hex(&id))),
                Event::SendFailed(id, why) => out.push(Frame::new("event").with("what", "failed").with("id", hex(&id)).with("why", why)),
                Event::Lost(id) => out.push(Frame::new("event").with("what", "lost").with("id", hex(&id))),
                Event::Resting { until } => out.push(Frame::new("event").with("what", "resting").with("until", until)),
                _ => {}
            }
        }
        let others = self.engine.devices().iter().filter(|d| d.id != self.engine.device_id()).count();
        out.push(Frame::new("state").with("paired", self.engine.paired() as u8).with("devices", others).with("wake", self.engine.next_wake().unwrap_or(0)));
    }
}

fn post_frame(p: kolada_sync::Post) -> Frame {
    let mut f = Frame::new("post").with("url", p.url);
    for (k, v) in p.headers {
        f = f.with(&format!("h:{k}"), v);
    }
    f.body(p.body)
}

fn list(v: Option<&str>) -> impl Iterator<Item = &str> {
    v.unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unpack(b: &[u8]) -> Vec<Frame> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 4 <= b.len() {
            let n = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
            out.push(Frame::parse(&b[at + 4..at + 4 + n]).unwrap());
            at += 4 + n;
        }
        out
    }

    fn call(c: &mut Catch, f: Frame) -> Vec<Frame> {
        unpack(&c.call(&f.with("now", 1_000).encode()))
    }

    /// The extension pairs as any device does: the code opens the pairing
    /// socket, the engine's frames go out on it, and once paired it says
    /// so and hands over its keys to keep; opened again from them, it is
    /// still paired.
    #[test]
    fn the_extension_pairs_by_code_and_keeps_its_keys() {
        let mut pc = Engine::open(kolada_sync::SERVER, "desktop", "PC", None, None);
        let mut c = Catch::new();
        // Drive both engines to paired, as the testing helper does, then
        // check what the interface says.
        kolada_sync::testing::pair(&mut pc, "desktop", c.engine(), "other");
        let got = call(&mut c, Frame::new("tick"));
        // The script keeps each `keys` as it comes: the last one counts.
        let keys = got.iter().rev().find(|f| f.t() == "keys").map(|f| f.body.clone()).unwrap_or_else(|| c.engine().keys_blob());
        let state = got.iter().find(|f| f.t() == "state").unwrap();
        assert_eq!((state.get("paired"), state.get("devices")), (Some("1"), Some("1")));
        let mut again = Catch::new();
        let state = call(&mut again, Frame::new("open").with("name", "Chrome").body(keys));
        assert_eq!(state.iter().find(|f| f.t() == "state").and_then(|f| f.get("paired")), Some("1"));
        // A code that isn't one is refused at once.
        let mut fresh = Catch::new();
        let got = call(&mut fresh, Frame::new("pair").with("code", "nope"));
        assert!(got.iter().any(|f| f.t() == "event" && f.get("what") == Some("refused")));
        // A real one asks for the pairing socket.
        let got = call(&mut fresh, Frame::new("pair").with("code", "F4FS-2RSA"));
        assert!(got.iter().any(|f| f.t() == "connect" && f.get("ch") == Some("pair") && f.get("url").is_some_and(|u| u.contains("/v1/pair/join"))), "{:?}", got.iter().map(|f| f.t().to_string()).collect::<Vec<_>>());
    }

    /// A page caught through the interface: read, its pictures chosen,
    /// written whole, sent by one request, a picture by its ticket, upload
    /// and item, all tied together.
    #[test]
    fn a_page_goes_through_the_interface() {
        let mut pc = Engine::open(kolada_sync::SERVER, "desktop", "PC", None, None);
        let mut c = Catch::new();
        kolada_sync::testing::pair(&mut pc, "desktop", c.engine(), "other");
        c.engine().outgoing();
        let html = "<html><body><article><h1>Walking</h1><p>An hour a day.</p><img src=\"/w.jpg\"></article></body></html>";
        let got = call(&mut c, Frame::new("extract").with("mode", "page").with("url", "https://longreads.example/walking").with("title", "Walking").body(html.as_bytes().to_vec()));
        let clipped = got.iter().find(|f| f.t() == "clipped").unwrap();
        let fetch = got.iter().find(|f| f.t() == "fetch").unwrap();
        assert_eq!(fetch.get("url"), Some("https://longreads.example/w.jpg"));
        let name = fetch.get("name").unwrap().to_string();
        let got = call(&mut c, Frame::new("riding").with("names", format!("{name}:2000")));
        assert_eq!(got[0].get("names"), Some(name.as_str()));
        let got = call(&mut c, Frame::new("compose").with("title", clipped.get("title").unwrap()).with("url", "https://longreads.example/walking").with("line", "For Thursday").with("caught", "2026-11-08T14:03").with("rides", &name).body(clipped.body.clone()));
        let composed = got.iter().find(|f| f.t() == "composed").unwrap();
        let text = String::from_utf8(composed.body.clone()).unwrap();
        assert!(text.starts_with("---\nsource: https://longreads.example/walking\n") && text.contains("For Thursday") && text.contains("![[Walking 1.jpg]]"), "{text:?}");
        let new = composed.get("renames").unwrap().split_once('>').unwrap().1.to_string();
        let got = call(&mut c, Frame::new("send").with("title", "Walking").with("url", "https://longreads.example/walking").body(text.into_bytes()));
        let post = got.iter().find(|f| f.t() == "post").unwrap();
        assert!(post.get("url").unwrap().ends_with("/send") && post.get("h:x-kolada-sig").is_some());
        let id = post.get("id").unwrap().to_string();
        let mut body = vec![1u8; 10];
        body.extend(vec![9u8; 2000]);
        let got = call(&mut c, Frame::new("picture").with("note", &id).with("name", &new).with("mime", "image/jpeg").with("width", 40).with("height", 30).with("thumb", 10).body(body));
        let ask = got.iter().find(|f| f.t() == "post").unwrap();
        let blob = ask.get("url").unwrap().rsplit('/').next().unwrap().to_string();
        let answer = Frame::new("ticket").with("id", blob.as_str()).with("op", "put").with("exp", 900_000).with("sig", "test").encode();
        let got = call(&mut c, Frame::new("answer").body(answer));
        let put = got.iter().find(|f| f.t() == "put").unwrap();
        assert!(put.get("url").unwrap().contains(&format!("/blob/{blob}")));
        let got = call(&mut c, Frame::new("put_done").with("blob", &blob).with("status", 200));
        assert!(got.iter().any(|f| f.t() == "post" && f.get("url").is_some_and(|u| u.ends_with("/send"))), "the picture's item");
        let cards = c.engine().cards().into_iter().cloned().collect::<Vec<_>>();
        let note = unhex16(&id).unwrap();
        assert!(cards.iter().any(|k| k.id == note && k.source.as_deref() == Some("https://longreads.example/walking")));
        assert_eq!(c.engine().note_pictures(note).len(), 1);
    }
}
