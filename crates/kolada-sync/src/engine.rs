//! The engine: everything a device does for Quick Send and pairing, without
//! I/O (design, section 9).
//!
//! It never opens a socket, starts a thread or reads the clock. The shell
//! asks it what to connect to (`connects`), feeds it what arrives (`opened`,
//! `frame`, `failed`, `closed`) with the time, and sends what it gives out
//! (`outgoing`). Results come back as `events`. Windows drives it from
//! kolada-win; Mac, iOS and Linux from kolada-app.

use std::path::PathBuf;

use crate::code::{self, Code, CodeError};
use crate::crypto::{b64url, hex, hkdf, hmac, open, open_from, open_item, open_stream, envelope_epoch, random, same, seal, seal_item, seal_stream, seal_to, sha256, unb64url, unhex, verify, GroupKeys};
use crate::frame::{kv, Frame};
use crate::pair::{self, Intro, Opener, Session, Welcome};
use crate::store::{open_state, seal_state, Keys, Member, R, W};

pub type ItemId = [u8; 16];
pub type DeviceId = [u8; 10];

/// The history keeps this many clips, until the user removes them; past
/// it, the oldest go.
pub const HISTORY: usize = 200;
/// A password leaves the history after a day, copied or not.
pub const SECRET_KEEP_MS: u64 = 24 * 3_600_000;
/// The pictures kept here, sealed, at most in all; past it, the oldest
/// picture clips go first.
pub const PICTURES_KEPT_MOST: u64 = 500 * 1024 * 1024;
pub const SEEN_KEEP_MS: u64 = 8 * 86_400_000;
pub const BACKOFF_MAX_MS: u64 = 300_000;
/// A connection that stayed open this long starts its backoff afresh.
pub const STEADY_MS: u64 = 60_000;
/// The largest picture that goes (the server takes 25 MB, sealed).
pub const PICTURE_MOST: usize = 20 * 1024 * 1024;
/// The wrong tries a shown code takes before this device gives it up.
pub const PAIR_TRIES: u32 = 5;
/// Sync batches go out paced, so a big first sync never trips the server's
/// limits (30 sends a minute a device) and never overwhelms a device: one at
/// a time, the next only once the server has the last, and this many a
/// minute. The rest wait in the outbox, on disk.
pub const SYNC_IN_FLIGHT: usize = 1;
/// The most sync batches held waiting, in bytes: past it the oldest go, and
/// the next manifest exchange sends what they carried again (`resync`).
pub const SYNC_HELD_MOST: usize = 64 * 1024 * 1024;
/// A sync batch the server hasn't answered in this long goes again: a lost
/// answer never stalls sync until the next connection.
pub const SYNC_ANSWER_MS: u64 = 60_000;
pub const SYNC_PER_MINUTE: usize = 20;

/// The largest thumbnail an item carries: the item fits its largest bucket.
pub const THUMB_MOST: usize = 96 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Group,
    Pair,
}

/// A socket to open: wss URL and headers.
#[derive(Clone, Debug)]
pub struct Connect {
    pub channel: Channel,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

/// One HTTPS request, for a device without a socket (the iOS extensions).
#[derive(Clone, Debug)]
pub struct Post {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Link,
    Secret,
    /// A picture: its thumbnail comes with the item; the picture itself is
    /// fetched when wanted (`fetch`).
    Picture,
}

/// A picture a card carries: what it is, where its sealed bytes are, and a
/// thumbnail to show at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub blob: ItemId,
    key: [u8; 32],
    pub mime: String,
    pub name: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    /// A small JPEG (at most `THUMB_MOST` bytes), made by the sender.
    pub thumb: Vec<u8>,
    /// The note it went with (`Outgoing::Note`): that card's id. Its card is
    /// shown as part of the note's, and saved with it.
    pub note: Option<ItemId>,
}

/// A picture a note embeds, beamed with it (`Outgoing::Note`): its name is
/// the embed's target as the note writes it (`photo.png`, `cats/tabby.webp`),
/// so the note reads the same where it is saved.
#[derive(Clone, Debug, Default)]
pub struct NotePicture {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub thumb: Vec<u8>,
}

/// An HTTPS transfer of a picture's sealed bytes, which the shell does and
/// reports back (`put_done`, `got`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transfer {
    Put { blob: ItemId, url: String, body: Vec<u8> },
    Get { blob: ItemId, url: String },
}

/// A picture being sent: sealed, waiting for its ticket, then its upload.
struct Upload {
    card: ItemId,
    blob: ItemId,
    sealed: Vec<u8>,
    to: Option<Vec<DeviceId>>,
    asked: bool,
}

/// One Quick Send, received or sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub id: ItemId,
    pub from: DeviceId,
    pub at: u64,
    pub kind: Kind,
    pub text: String,
    pub title: String,
    pub read: bool,
    pub copied: bool,
    pub mine: bool,
    /// One of this device's that expired on the server before any device
    /// it was for had it (7 days): its card says it didn't arrive, and it
    /// can be sent again (`resend`).
    pub lost: bool,
    pub picture: Option<Picture>,
}

/// What to send.
pub enum Outgoing {
    Text(String),
    /// A password: masked, copied once, gone everywhere at the first copy.
    Secret(String),
    /// A picture's file bytes (PNG, JPEG...), its type, its file name, its
    /// size in pixels and a thumbnail the shell made (a JPEG).
    Picture { bytes: Vec<u8>, mime: String, name: String, width: u32, height: u32, thumb: Vec<u8> },
    /// A note: its Markdown as it is, under its name (`title`), and each
    /// picture it embeds, as picture items that go with it. Too long, and
    /// nothing goes; a picture too big for Beam is left out.
    Note { title: String, text: String, pictures: Vec<NotePicture> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub kind: String,
    pub name: String,
    pub box_pub: [u8; 32],
    /// Whether it syncs notes, as it says (`Engine::set_sync`).
    pub sync: SyncCap,
    /// Its box key is vouched for by the group (`roster_tag`), so a removal
    /// re-keys the group to it; one that isn't (a device the server made
    /// up, or one whose key isn't the one pinned) is left out.
    pub trusted: bool,
}

/// Whether a device syncs notes, as it tells the others. A group syncs
/// notes once one device says `On`; then every device that says `On` or
/// `Follow` syncs with every other that does. `Follow` is for a device
/// that can sync but waits for one that is switched on (a phone, until the
/// desktop release turns note sync on), so two of them alone sync nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncCap {
    #[default]
    Off,
    Follow,
    On,
}

impl SyncCap {
    fn word(self) -> &'static str {
        match self {
            SyncCap::Off => "",
            SyncCap::Follow => "follow",
            SyncCap::On => "sync",
        }
    }

    fn of(word: &str) -> SyncCap {
        match word {
            "sync" => SyncCap::On,
            "follow" => SyncCap::Follow,
            _ => SyncCap::Off,
        }
    }
}

/// Where pairing stands (see `Engine::pair_state`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairState {
    Idle,
    /// Asking the server for a slot.
    Opening,
    /// Showing this code until `expires` (ms).
    Showing { code: String, expires: u64 },
    /// A device knew the code: allow it?
    Asks { name: String },
    /// Waiting on the other device or the server.
    Waiting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No such code (or its device went away).
    NoSuchCode,
    Expired,
    /// Five wrong tries.
    Burned,
    /// The two devices aren't in the same country.
    Country,
    /// Someone else is trying this code right now.
    Busy,
    /// Too many tries from here.
    Slow,
    /// Pairing is resting until tomorrow (the free-tier budget).
    Quota,
    /// The code didn't match.
    WrongCode,
    /// The other device said No.
    Denied,
    /// The group is full (10 devices).
    Full,
    Network,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Store this blob in the OS secret store; give it back to `open`.
    Keys(Vec<u8>),
    /// Show this code (`F4FS-2RSA`) until `expires` (ms).
    Code { code: String, expires: u64 },
    /// A device knows the code: ask the user to allow it (decision D1).
    Asks { name: String, kind: String },
    Paired,
    CodeRefused(Refusal),
    DevicesChanged,
    Arrived(ItemId),
    Gone(ItemId),
    Sent(ItemId),
    /// Why, as the server said it: "slow" (too many this minute), "limit"
    /// (the day's), "full" (too much waiting), "size", "pictures"
    /// (resting until tomorrow), "upload".
    SendFailed(ItemId, String),
    /// A beam of this device's expired before any device it was for had
    /// it: its card is `lost`.
    Lost(ItemId),
    /// The server is resting until then (ms): nothing is lost, sends wait.
    Resting { until: u64 },
    /// Another device removed this one: it is no longer paired.
    Removed,
    /// Items for this device expired before it came back (sync catches up).
    Resync,
    /// A picture is kept here now (`picture`): one that arrived, fetched
    /// with its item.
    PictureReady(ItemId),
    /// A picture that arrived could not be opened (changed on the way): its
    /// card goes.
    FetchFailed(ItemId),
    /// A batch of note changes from another device (`name`, as it calls
    /// itself), for the vault (`vault::Vault::apply`). Its item is
    /// acknowledged once the shell says it was applied (`applied`): a crash
    /// in between brings it again.
    Synced { id: ItemId, from: DeviceId, name: String, batch: Vec<u8> },
}

/// Sync's manifest exchange as it reached this device, for the sync loop
/// to take (`take_exchange`): never stored by the server, only relayed.
#[derive(Debug, Default)]
pub struct Exchange {
    /// Manifest parts (`journal::part`), opened, by sender.
    pub parts: Vec<(DeviceId, Vec<u8>)>,
    /// Devices a part could not reach: not connected.
    pub away: Vec<DeviceId>,
    /// The server lost the group's items: meet a vault again.
    pub resync: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkState {
    Idle,
    Connecting,
    Open,
}

#[derive(Clone, Copy, Debug)]
struct Link {
    state: LinkState,
    attempts: u32,
    retry_at: u64,
    opened_at: u64,
}

impl Link {
    const fn new() -> Link {
        Link { state: LinkState::Idle, attempts: 0, retry_at: 0, opened_at: 0 }
    }
}

enum Pairing {
    None,
    /// Showing a code: waiting for the slot, then for a joiner; `bad`, the
    /// wrong tries so far (this device gives the code up at `PAIR_TRIES`,
    /// whatever the server counts).
    Showing { secret: String, slot: Option<String>, opener: Option<Opener>, expires: u64, bad: u32 },
    /// A joiner knew the code: waiting for the user.
    Asking { session: Session, intro: Intro },
    /// Allowed: waiting for this device's own group registration first.
    Welcoming { session: Session, intro: Intro },
    /// Entering a code: waiting for the opener's message, then the welcome.
    Entering { code: Code, session: Option<Session> },
}

pub struct Engine {
    base: String,
    kind: String,
    name: String,
    data: Option<PathBuf>,
    keys: Keys,
    devices: Vec<Device>,
    cursor: u64,
    every: u64,
    cards: Vec<Card>,
    seen: Vec<(ItemId, u64)>,
    outbox: Vec<(ItemId, Vec<u8>)>,
    links: [Link; 2],
    rest_until: u64,
    last_time: u64,
    ack: Option<u64>,
    head: u64,
    backlog: u64,
    out: Vec<(Channel, Vec<u8>)>,
    events: Vec<Event>,
    pairing: Pairing,
    name_sent: bool,
    /// Each device's box key as the group last vouched for it (or, for one
    /// from before tags, as first seen): kept on disk, so a list that later
    /// shows another is not trusted.
    pins: Vec<(DeviceId, [u8; 32])>,
    /// This device's own roster tag as last sent, this connection.
    tag_said: Option<String>,
    /// Sync batches sent this connection and not yet answered, when each
    /// went (the pace: `SYNC_IN_FLIGHT`, `SYNC_PER_MINUTE`), and until when
    /// the server asked them to wait.
    sync_flying: Vec<ItemId>,
    /// When the batches in flight went (for `SYNC_ANSWER_MS`).
    sync_flew: u64,
    sync_times: Vec<u64>,
    sync_wait: u64,
    /// Sync batches waiting to go, oldest first (each with its place, its
    /// id and its frame), and their bytes: kept out of the state file, each
    /// in a file of its own (`held_path`), written once and deleted once
    /// the server has it; at most `held_most` bytes.
    held: std::collections::VecDeque<(u64, ItemId, Vec<u8>)>,
    held_bytes: usize,
    held_next: u64,
    held_most: usize,
    /// The latest time this engine was told (`tick`, `frame`, `connects`).
    now: u64,
    /// This device's own box key's signature went this connection.
    box_sent: bool,
    /// Whether this device syncs notes, as it tells the group, and whether
    /// the group's list already says so.
    sync: SyncCap,
    sync_said: bool,
    /// This device's push token and whether it is the sandbox's, sent on
    /// every connection (one registered while no socket was open waits for
    /// the next).
    push: Option<(String, bool)>,
    uploads: Vec<Upload>,
    /// Pictures being fetched: the card, the blob, whether a ticket was
    /// asked, and the item's place in the log (its ack waits for it).
    fetches: Vec<(ItemId, ItemId, bool, u64)>,
    transfers: Vec<Transfer>,
    /// Pictures kept in memory when there is no data folder (tests).
    kept: Vec<(ItemId, Vec<u8>)>,
    /// The last place in the log acknowledged on this connection.
    acked: u64,
    /// Sync batches handed to the shell and not yet applied, with their
    /// place in the log (their ack waits).
    syncing: Vec<(ItemId, u64)>,
    exchange: Exchange,
}

fn idx(c: Channel) -> usize {
    match c {
        Channel::Group => 0,
        Channel::Pair => 1,
    }
}

impl Engine {
    /// Opens the engine from the secret blob the shell stored (None the first
    /// time: new keys are made and handed back as `Event::Keys`). `data` is
    /// this feature's folder in app data, or None to keep nothing on disk.
    /// `base` is the server, `https://sync.kolada.app`.
    pub fn open(base: &str, kind: &str, name: &str, data: Option<PathBuf>, keys: Option<&[u8]>) -> Engine {
        let loaded = keys.and_then(Keys::decode);
        let fresh = loaded.is_none();
        let mut e = Engine {
            base: base.trim_end_matches('/').to_string(),
            kind: kind.into(),
            name: name.into(),
            data,
            keys: loaded.unwrap_or_default(),
            devices: Vec::new(),
            cursor: 0,
            every: 10,
            cards: Vec::new(),
            seen: Vec::new(),
            outbox: Vec::new(),
            links: [Link::new(); 2],
            rest_until: 0,
            last_time: 0,
            ack: None,
            head: 0,
            backlog: 0,
            out: Vec::new(),
            events: Vec::new(),
            pairing: Pairing::None,
            name_sent: false,
            pins: Vec::new(),
            tag_said: None,
            sync_flying: Vec::new(),
            sync_flew: 0,
            sync_times: Vec::new(),
            sync_wait: 0,
            held: Default::default(),
            held_bytes: 0,
            held_next: 0,
            held_most: SYNC_HELD_MOST,
            now: 0,
            box_sent: false,
            sync: SyncCap::Off,
            sync_said: false,
            push: None,
            uploads: Vec::new(),
            fetches: Vec::new(),
            transfers: Vec::new(),
            kept: Vec::new(),
            acked: 0,
            syncing: Vec::new(),
            exchange: Exchange::default(),
        };
        if fresh {
            e.keys_changed();
        }
        e.load();
        e.load_held();
        e
    }

    pub fn paired(&self) -> bool {
        self.keys.member.is_some()
    }

    /// A pairing is under way (its socket is wanted).
    pub fn pairing(&self) -> bool {
        !matches!(self.pairing, Pairing::None)
    }

    /// Where pairing stands, for the drawer.
    pub fn pair_state(&self) -> PairState {
        match &self.pairing {
            Pairing::None => PairState::Idle,
            Pairing::Showing { slot: None, .. } => PairState::Opening,
            Pairing::Showing { slot: Some(slot), secret, expires, .. } => {
                PairState::Showing { code: Code { slot: slot.clone(), secret: secret.clone() }.show(), expires: *expires }
            }
            Pairing::Asking { intro, .. } => PairState::Asks { name: intro.name.clone() },
            Pairing::Welcoming { .. } | Pairing::Entering { .. } => PairState::Waiting,
        }
    }

    pub fn device_id(&self) -> DeviceId {
        self.keys.me.id()
    }

    /// This device's name, as the others see it.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn box_pub(&self) -> [u8; 32] {
        self.keys.me.box_pub()
    }

    /// The secret blob as it stands (also given by every `Event::Keys`).
    pub fn keys_blob(&self) -> Vec<u8> {
        self.keys.encode()
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// The sync interval the server allows now, in seconds.
    pub fn every(&self) -> u64 {
        self.every
    }

    pub fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    fn keys_changed(&mut self) {
        self.events.push(Event::Keys(self.keys.encode()));
    }

    // Connections.

    fn ws_base(&self) -> String {
        if let Some(rest) = self.base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = self.base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            self.base.clone()
        }
    }

    fn signed(&mut self, method: &str, path: &str, body: &[u8], now: u64) -> Vec<(String, String)> {
        let time = now.max(self.last_time + 1);
        self.last_time = time;
        let dev = hex(&self.keys.me.id());
        let text = format!("kolada/v1|{method}|{path}|{dev}|{time}|{}", hex(&sha256(body)));
        vec![
            ("x-kolada-device".into(), dev),
            ("x-kolada-time".into(), time.to_string()),
            ("x-kolada-sig".into(), b64url(&self.keys.me.sign(text.as_bytes()))),
        ]
    }

    /// The sockets to open now.
    pub fn connects(&mut self, now: u64) -> Vec<Connect> {
        self.now = self.now.max(now);
        let mut out = Vec::new();
        let ws = self.ws_base();
        let group_link = self.links[0];
        if let Some(m) = self.keys.member.clone()
            && group_link.state == LinkState::Idle
            && now >= group_link.retry_at
            && now >= self.rest_until
        {
            let path = format!("/v1/g/{}/ws", hex(&m.group));
            let mut headers = self.signed("GET", &path, &[], now);
            if !m.registered {
                headers.push(("x-kolada-sign-pub".into(), b64url(&self.keys.me.sign_pub())));
                headers.push(("x-kolada-box-pub".into(), b64url(&self.keys.me.box_pub())));
                headers.push(("x-kolada-kind".into(), self.kind.clone()));
                headers.push(("x-kolada-box-sig".into(), b64url(&self.box_sig())));
                headers.push(("x-kolada-caps".into(), self.sync.word().into()));
                headers.push(("x-kolada-tag".into(), self.my_tag().unwrap_or_default()));
                if m.creator {
                    headers.push(("x-kolada-create".into(), "1".into()));
                } else {
                    headers.push(("x-kolada-voucher".into(), b64url(&m.voucher)));
                }
            }
            self.links[0].state = LinkState::Connecting;
            out.push(Connect { channel: Channel::Group, url: format!("{ws}{path}"), headers });
        }
        if self.links[1].state == LinkState::Idle {
            let url = match &self.pairing {
                Pairing::Showing { slot: None, .. } => Some(format!("{ws}/v1/pair/open")),
                Pairing::Entering { code, session: None } => Some(format!("{ws}/v1/pair/join?slot={}", code.slot)),
                _ => None,
            };
            if let Some(url) = url {
                self.links[1].state = LinkState::Connecting;
                out.push(Connect { channel: Channel::Pair, url, headers: Vec::new() });
            }
        }
        out
    }

    pub fn opened(&mut self, ch: Channel, now: u64) {
        if ch == Channel::Group {
            self.acked = 0;
            self.box_sent = false;
            self.tag_said = None;
        }
        let l = &mut self.links[idx(ch)];
        l.state = LinkState::Open;
        l.opened_at = now;
    }

    /// The socket closed, or never opened (a network error).
    pub fn closed(&mut self, ch: Channel, now: u64) {
        let l = &mut self.links[idx(ch)];
        let steady = l.state == LinkState::Open && now.saturating_sub(l.opened_at) >= STEADY_MS;
        l.state = LinkState::Idle;
        match ch {
            Channel::Group => {
                if steady {
                    l.attempts = 0;
                }
                l.attempts += 1;
                l.retry_at = now + backoff(l.attempts);
            }
            Channel::Pair => {
                if !matches!(self.pairing, Pairing::None | Pairing::Welcoming { .. }) {
                    self.pairing = Pairing::None;
                    self.events.push(Event::CodeRefused(Refusal::Network));
                }
            }
        }
    }

    /// The server answered the upgrade with an error instead of a socket.
    pub fn failed(&mut self, ch: Channel, status: u16, body: &str, now: u64) {
        let fields = kv(body);
        let error = fields.iter().find(|(k, _)| k == "error").map(|(_, v)| v.as_str()).unwrap_or("");
        let retry = fields.iter().find(|(k, _)| k == "retry").and_then(|(_, v)| v.parse::<u64>().ok());
        match ch {
            Channel::Group => {
                self.links[0].state = LinkState::Idle;
                if error == "rest" {
                    self.rest_until = now + retry.unwrap_or(3600) * 1000;
                    self.events.push(Event::Resting { until: self.rest_until });
                    return;
                }
                let registered = self.keys.member.as_ref().is_some_and(|m| m.registered);
                if status == 401 && registered && error == "unknown" {
                    // A group this device made, that nobody else joined, was
                    // forgotten after a day: it is made again on the next
                    // connection. Otherwise this device was removed, or
                    // its group forgotten.
                    let me = self.keys.me.id();
                    let alone = self.devices.iter().all(|d| d.id == me);
                    if let Some(m) = self.keys.member.as_mut().filter(|m| m.creator && alone) {
                        m.registered = false;
                        self.keys_changed();
                        self.links[0].attempts += 1;
                        self.links[0].retry_at = now + backoff(self.links[0].attempts);
                        return;
                    }
                    self.leave();
                    return;
                }
                self.links[0].attempts += 1;
                self.links[0].retry_at = now + backoff(self.links[0].attempts);
            }
            Channel::Pair => {
                self.links[1].state = LinkState::Idle;
                let why = match (status, error) {
                    (403, _) => Refusal::Country,
                    (410, "burned") => Refusal::Burned,
                    (410, _) => Refusal::Expired,
                    (409, _) => Refusal::Busy,
                    (429, _) => Refusal::Slow,
                    (503, _) => Refusal::Quota,
                    (404, _) | (400, _) => Refusal::NoSuchCode,
                    _ => Refusal::Network,
                };
                self.pairing = Pairing::None;
                self.events.push(Event::CodeRefused(why));
            }
        }
    }

    /// Frames to send now, per socket.
    pub fn outgoing(&mut self) -> Vec<(Channel, Vec<u8>)> {
        // An item with a picture is acknowledged only once the picture is
        // down: the server drops both when every device has acknowledged.
        let held = self.fetches.iter().map(|f| f.3).chain(self.syncing.iter().map(|s| s.1)).min();
        if self.links[0].state == LinkState::Open
            && let Some(seq) = self.ack
        {
            let seq = held.map_or(seq, |h| seq.min(h.saturating_sub(1)));
            if seq > self.acked {
                self.acked = seq;
                self.out.push((Channel::Group, Frame::new("ack").with("seq", seq).encode()));
            }
            if held.is_none() {
                self.ack = None;
            }
        }
        let open = |c: Channel, links: &[Link; 2]| links[idx(c)].state == LinkState::Open;
        let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.out).into_iter().partition(|(c, _)| open(*c, &self.links));
        self.out = later;
        now
    }

    /// When to call `tick` next (ms), if ever.
    pub fn next_wake(&self) -> Option<u64> {
        let mut t: Option<u64> = None;
        let mut at = |x: u64| t = Some(t.map_or(x, |y| y.min(x)));
        if self.keys.member.is_some() && self.links[0].state == LinkState::Idle {
            at(self.links[0].retry_at.max(self.rest_until));
        }
        if let Some(c) = self.cards.iter().filter(|c| c.kind == Kind::Secret).map(|c| c.at + SECRET_KEEP_MS).min() {
            at(c);
        }
        if let Pairing::Showing { expires, slot: Some(_), .. } = &self.pairing {
            at(*expires);
        }
        // A batch unanswered too long goes again.
        if self.links[0].state == LinkState::Open && !self.sync_flying.is_empty() {
            at(self.sync_flew + SYNC_ANSWER_MS);
        }
        // Sync batches held by the pace: when the next may go.
        if self.links[0].state == LinkState::Open && self.sync_waiting() > 0 && self.sync_flying.len() < SYNC_IN_FLIGHT {
            let minute = if self.sync_times.len() >= SYNC_PER_MINUTE { self.sync_times.iter().min().map_or(0, |t| t + 60_000) } else { 0 };
            at(minute.max(self.sync_wait));
        }
        t
    }

    pub fn tick(&mut self, now: u64) {
        self.now = self.now.max(now);
        self.pace(now);
        let before = self.cards.len();
        self.cards.retain(|c| c.kind != Kind::Secret || now < c.at + SECRET_KEEP_MS);
        self.seen.retain(|(_, at)| now < at + SEEN_KEEP_MS);
        if let Pairing::Showing { expires, slot: Some(_), .. } = &self.pairing
            && now >= *expires
        {
            self.pairing = Pairing::None;
            self.events.push(Event::CodeRefused(Refusal::Expired));
        }
        if self.cards.len() != before {
            self.drop_pictures();
            self.save();
        }
    }

    // What arrives.

    pub fn frame(&mut self, ch: Channel, bytes: &[u8], now: u64) {
        self.now = self.now.max(now);
        let Some(f) = Frame::parse(bytes) else { return };
        match ch {
            Channel::Group => self.group_frame(f, now),
            Channel::Pair => self.pair_frame(f, now),
        }
    }

    fn group_frame(&mut self, f: Frame, now: u64) {
        match f.t() {
            "hello" => {
                self.links[0].attempts = 0;
                if let Some(m) = self.keys.member.as_mut()
                    && !m.registered
                {
                    m.registered = true;
                    self.keys_changed();
                }
                self.every = f.num("every").unwrap_or(10);
                self.head = f.num("head").unwrap_or(0);
                self.backlog = f.num("backlog").unwrap_or(0);
                self.cursor = self.cursor.max(f.num("cursor").unwrap_or(0));
                if self.backlog == 0 && self.head > self.cursor {
                    self.ack = Some(self.head);
                    self.cursor = self.head;
                }
                self.read_devices(&f);
                if !self.name_sent {
                    self.send_name();
                }
                self.send_push();
                self.sync_flying.clear();
                for (_, frame) in &self.outbox {
                    self.out.push((Channel::Group, frame.clone()));
                }
                self.pace(now);
                self.ask_tickets(true);
                if let Pairing::Welcoming { .. } = self.pairing {
                    self.welcome(now);
                }
            }
            "item" => self.item(&f, now),
            "sent" => {
                if let Some(id) = f.get("id").and_then(unhex::<16>) {
                    self.sync_flying.retain(|i| *i != id);
                    if self.unhold(&id) {
                        // A sync batch: its own file goes; the state file isn't touched.
                        self.pace(now);
                        return;
                    }
                    self.outbox.retain(|(i, _)| *i != id);
                    self.events.push(Event::Sent(id));
                    self.save();
                }
            }
            "error" => {
                if let Some(id) = f.get("id").and_then(unhex::<16>) {
                    let why = f.get("error").unwrap_or("").to_string();
                    // A picture's ticket refused: its upload and card go.
                    if let Some(at) = self.uploads.iter().position(|u| u.blob == id) {
                        let up = self.uploads.remove(at);
                        self.cards.retain(|c| c.id != up.card);
                        self.events.push(Event::SendFailed(up.card, why));
                        self.drop_pictures();
                        self.save();
                        return;
                    }
                    // A sync batch the server holds back (a burst, the
                    // day's share, too much waiting) waits and goes again;
                    // nothing of a vault is dropped.
                    if self.sync_flying.contains(&id) {
                        self.sync_flying.retain(|i| *i != id);
                        if matches!(why.as_str(), "slow" | "limit" | "full" | "rest") {
                            let retry = f.num("retry").unwrap_or(60).clamp(1, 86_400);
                            self.sync_wait = self.sync_wait.max(now + retry * 1000);
                            return;
                        }
                    }
                    if self.unhold(&id) {
                        // Dropped for good (its size, its frame, its devices):
                        // the next manifest exchange sends what it carried.
                        self.exchange.resync = true;
                        self.events.push(Event::SendFailed(id, why));
                        return;
                    }
                    self.outbox.retain(|(i, _)| *i != id);
                    self.events.push(Event::SendFailed(id, why));
                }
            }
            "lost" => {
                for id in f.list("ids").into_iter().filter_map(unhex::<16>) {
                    if let Some(c) = self.cards.iter_mut().find(|c| c.id == id && c.mine) {
                        c.lost = true;
                        self.events.push(Event::Lost(id));
                    }
                }
                self.save();
            }
            "gone" => {
                for id in f.list("ids").into_iter().filter_map(unhex::<16>) {
                    self.cards.retain(|c| c.id != id);
                    self.fetches.retain(|f| f.0 != id);
                    self.events.push(Event::Gone(id));
                }
                self.drop_pictures();
                self.save();
            }
            "ticket" => self.ticket(&f),
            "every" => self.every = f.num("s").unwrap_or(self.every),
            "devices" => self.read_devices(&f),
            "resync" => {
                self.exchange.resync = true;
                self.events.push(Event::Resync);
            }
            "from" => self.manifest_in(&f),
            "away" => {
                if let Some(dev) = f.get("dev").and_then(unhex::<10>) {
                    self.exchange.away.push(dev);
                }
            }
            "removed" => self.leave(),
            "rest" => {
                self.rest_until = now + f.num("retry").unwrap_or(3600) * 1000;
                self.events.push(Event::Resting { until: self.rest_until });
            }
            _ => {}
        }
    }

    fn item(&mut self, f: &Frame, now: u64) {
        let (Some(seq), Some(id), Some(from)) = (f.num("seq"), f.get("id").and_then(unhex::<16>), f.get("from").and_then(unhex::<10>)) else { return };
        let kind = f.get("kind").unwrap_or("").bytes().next().unwrap_or(0);
        // A sync batch counts as seen only once it is applied: one brought
        // again after a restart is applied then.
        if kind == b's' {
            if !self.seen.iter().any(|(s, _)| *s == id) && !self.syncing.iter().any(|(s, _)| *s == id) {
                self.sync_in(id, from, &f.body, seq);
            }
        } else if !self.seen.iter().any(|(s, _)| *s == id) {
            self.seen.push((id, now));
            match kind {
                b'q' => self.quick_send(id, from, f.num("at").unwrap_or(now), &f.body, seq),
                b'k' => self.rekey(&f.body),
                _ => {}
            }
        }
        self.cursor = self.cursor.max(seq);
        self.ack = Some(self.ack.unwrap_or(0).max(seq));
        if self.backlog > 0 {
            self.backlog -= 1;
            if self.backlog == 0 && self.head > self.cursor {
                self.cursor = self.head;
                self.ack = Some(self.head);
            }
        }
        self.save();
    }

    fn quick_send(&mut self, id: ItemId, from: DeviceId, at: u64, env: &[u8], seq: u64) {
        let Some(m) = &self.keys.member else { return };
        let Some(keys) = envelope_epoch(env).and_then(|e| m.keys_for(e)) else { return };
        let Some(plain) = open_item(keys, b'q', &id, &from, env) else { return };
        let Some((kind, text, title)) = decode_payload(&plain) else { return };
        let picture = if kind == Kind::Picture {
            match decode_picture(&plain) {
                Some(p) => Some(p),
                None => return,
            }
        } else {
            None
        };
        let blob = picture.as_ref().map(|p| p.blob);
        self.cards.push(Card { id, from, at, kind, text, title, read: false, copied: false, mine: false, lost: false, picture });
        self.trim();
        self.events.push(Event::Arrived(id));
        if let Some(blob) = blob {
            self.fetches.push((id, blob, false, seq));
            self.ask_tickets(false);
        }
    }

    fn sync_in(&mut self, id: ItemId, from: DeviceId, env: &[u8], seq: u64) {
        let Some(m) = &self.keys.member else { return };
        let Some(keys) = envelope_epoch(env).and_then(|e| m.keys_for(e)) else { return };
        let Some(batch) = open_item(keys, b's', &id, &from, env) else { return };
        let name = self.devices.iter().find(|d| d.id == from).map(|d| d.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "another device".into());
        self.syncing.push((id, seq));
        self.events.push(Event::Synced { id, from, name, batch });
    }

    /// The shell applied the sync batch `id`: its item is acknowledged.
    pub fn applied(&mut self, id: ItemId, now: u64) {
        let before = self.syncing.len();
        self.syncing.retain(|(s, _)| *s != id);
        if self.syncing.len() != before {
            self.seen.push((id, now));
            self.save();
        }
    }

    /// Sends a batch of note changes (`vault::Vault::outgoing`) to every
    /// other device, sealed under the sync key: now if connected, else as
    /// soon as the socket opens. None if not paired or too big for an item.
    pub fn send_sync(&mut self, batch: &[u8], now: u64) -> Option<ItemId> {
        // Only to the devices that sync notes: none, nothing goes.
        let peers = self.sync_peers();
        if peers.is_empty() {
            return None;
        }
        self.send_sync_to(batch, Some(&peers), now)
    }

    /// As `send_sync`, to these devices only (None: every other device):
    /// the answer to another device's manifest.
    pub fn send_sync_to(&mut self, batch: &[u8], to: Option<&[DeviceId]>, now: u64) -> Option<ItemId> {
        let m = self.keys.member.as_ref()?;
        let id: ItemId = random();
        let env = seal_item(&m.keys, b's', &id, &self.keys.me.id(), batch)?;
        let mut f = Frame::new("send").with("id", hex(&id)).with("kind", "s");
        if let Some(to) = to.filter(|t| !t.is_empty()) {
            f = f.with("to", to.iter().map(|d| hex(d)).collect::<Vec<_>>().join(","));
        }
        let bytes = f.body(env).encode();
        self.hold(id, bytes);
        self.pace(now);
        Some(id)
    }

    /// A sync batch held to go: its file written once; past `held_most`
    /// bytes, the oldest not yet sent go, and a manifest exchange repairs
    /// what they carried.
    fn hold(&mut self, id: ItemId, frame: Vec<u8>) {
        let at = self.held_next;
        self.held_next += 1;
        if let Some(path) = self.held_path(at, &id) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // Not kept on disk (a full disk): it goes from memory, and if the
            // app quits first, a manifest exchange repairs it.
            if std::fs::write(&path, seal_state(&self.keys.local, &frame)).is_err() {
                self.exchange.resync = true;
            }
        }
        self.held_bytes += frame.len();
        self.held.push_back((at, id, frame));
        while self.held_bytes > self.held_most && self.held.len() > 1 {
            let Some(old) = self.held.iter().position(|(_, i, _)| !self.sync_flying.contains(i)) else { break };
            let (at, id, frame) = self.held.remove(old).unwrap_or_default();
            self.held_bytes -= frame.len();
            if let Some(path) = self.held_path(at, &id) {
                let _ = std::fs::remove_file(path);
            }
            self.exchange.resync = true;
        }
    }

    /// The held batch `id` goes (the server has it, or refused it for
    /// good): whether it was one.
    fn unhold(&mut self, id: &ItemId) -> bool {
        let Some(i) = self.held.iter().position(|(_, h, _)| h == id) else { return false };
        let (at, id, frame) = self.held.remove(i).unwrap_or_default();
        self.held_bytes -= frame.len();
        if let Some(path) = self.held_path(at, &id) {
            let _ = std::fs::remove_file(path);
        }
        true
    }

    /// Where a held sync batch is kept: its own file, sealed, named by its
    /// place so they come back in order.
    fn held_path(&self, at: u64, id: &ItemId) -> Option<PathBuf> {
        self.data.as_ref().map(|d| d.join("sync-out").join(format!("{at:016x}-{}.bin", hex(id))))
    }

    /// The held sync batches back from their files, oldest first; a batch
    /// an older version kept in the state file joins them.
    fn load_held(&mut self) {
        let legacy: Vec<(ItemId, Vec<u8>)> = self.outbox.iter().filter(|(_, f)| is_sync(f)).cloned().collect();
        self.outbox.retain(|(_, f)| !is_sync(f));
        if let Some(dir) = self.data.as_ref().map(|d| d.join("sync-out")) {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            files.sort();
            for path in files {
                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let parsed = name.split_once('-').and_then(|(at, id)| Some((u64::from_str_radix(at, 16).ok()?, unhex::<16>(id)?)));
                let frame = std::fs::read(&path).ok().and_then(|b| open_state(&self.keys.local, &b));
                match (parsed, frame) {
                    (Some((at, id)), Some(frame)) => {
                        self.held_next = self.held_next.max(at + 1);
                        self.held_bytes += frame.len();
                        self.held.push_back((at, id, frame));
                    }
                    _ => {
                        // Unreadable: it goes, and a manifest exchange repairs it.
                        let _ = std::fs::remove_file(&path);
                        self.exchange.resync = true;
                    }
                }
            }
        }
        for (id, frame) in legacy {
            self.hold(id, frame);
        }
    }

    /// A smaller cap on held sync batches (tests; a shell with little room).
    pub fn set_sync_held_most(&mut self, bytes: usize) {
        self.held_most = bytes;
    }

    /// Sends what sync batches the pace allows now: the oldest first, at
    /// most `SYNC_IN_FLIGHT` unanswered and `SYNC_PER_MINUTE` a minute, and
    /// none while the server asked to wait.
    fn pace(&mut self, now: u64) {
        // The latest time seen: a frame stamped earlier never reopens the minute.
        let now = now.max(self.now);
        self.now = now;
        if self.links[0].state != LinkState::Open || now < self.sync_wait {
            return;
        }
        self.sync_times.retain(|t| now < t + 60_000);
        // No answer for a minute: what flew goes again.
        if !self.sync_flying.is_empty() && now >= self.sync_flew + SYNC_ANSWER_MS {
            self.sync_flying.clear();
        }
        // Only the batches that go are copied, never the whole queue.
        let mut i = 0;
        while i < self.held.len() && self.sync_flying.len() < SYNC_IN_FLIGHT && self.sync_times.len() < SYNC_PER_MINUTE {
            let (_, id, frame) = &self.held[i];
            if !self.sync_flying.contains(id) {
                let (id, frame) = (*id, frame.clone());
                self.sync_flying.push(id);
                self.sync_flew = now;
                self.sync_times.push(now);
                self.out.push((Channel::Group, frame));
            }
            i += 1;
        }
    }

    /// Sync batches waiting to go (the pace holds them).
    pub fn sync_waiting(&self) -> usize {
        self.held.iter().filter(|(_, i, _)| !self.sync_flying.contains(i)).count()
    }

    /// Bytes of sync batches held waiting (at most `SYNC_HELD_MOST`).
    pub fn sync_held_bytes(&self) -> usize {
        self.held_bytes
    }

    /// Relays a part of this vault's manifest (`journal::parts`) to `to`,
    /// sealed for the group; the server keeps nothing. False if it can't go
    /// now (not paired, or not connected): a manifest is never queued.
    pub fn send_manifest(&mut self, to: DeviceId, part: &[u8]) -> bool {
        let Some(m) = self.keys.member.as_ref() else { return false };
        if self.links[0].state != LinkState::Open {
            return false;
        }
        let id: ItemId = random();
        let Some(env) = seal_item(&m.keys, b'm', &id, &self.keys.me.id(), part) else { return false };
        let mut body = id.to_vec();
        body.extend_from_slice(&env);
        self.out.push((Channel::Group, Frame::new("to").with("dev", hex(&to)).body(body).encode()));
        true
    }

    fn manifest_in(&mut self, f: &Frame) {
        let (Some(m), Some(from)) = (&self.keys.member, f.get("dev").and_then(unhex::<10>)) else { return };
        if f.body.len() < 16 {
            return;
        }
        let (id, env) = f.body.split_at(16);
        let id: ItemId = id.try_into().unwrap_or_default();
        let Some(keys) = envelope_epoch(env).and_then(|e| m.keys_for(e)) else { return };
        if let Some(part) = open_item(keys, b'm', &id, &from, env) {
            self.exchange.parts.push((from, part));
        }
    }

    /// What of the manifest exchange came in since last taken.
    pub fn take_exchange(&mut self) -> Exchange {
        std::mem::take(&mut self.exchange)
    }

    /// A new group secret from the device that removed another.
    fn rekey(&mut self, body: &[u8]) {
        let Some(m) = self.keys.member.as_mut() else { return };
        let Some(p) = open_from(&self.keys.me, body) else { return };
        let mut r = R(&p);
        let (Some(secret), Some(epoch), Some(proof)) = (r.arr::<32>(), r.u32(), r.arr::<32>()) else { return };
        // Only a holder of the current secret can make this proof: the
        // server, which knows this device's public box key, cannot.
        if epoch != m.keys.epoch + 1 || !same(&proof, &rekey_proof(&m.secret, &m.group, epoch, &secret)) {
            return;
        }
        m.prev = Some((m.secret, m.keys.clone()));
        m.secret = secret;
        m.keys = GroupKeys::new(m.group, &secret, epoch);
        self.keys_changed();
        self.say_tag();
        self.events.push(Event::DevicesChanged);
    }

    /// The group's devices as the server lists them:
    /// `id:kind:sealed name:box key[:signing key:signature:caps:roster tag]`.
    ///
    /// Which box keys a removal re-keys the group to (`Device::trusted`) is
    /// decided here, never by the server alone:
    /// - a box key whose roster tag (`roster_tag`) checks under the group's
    ///   current secret is vouched for by a member, and pinned;
    /// - any other is trusted only if it is the key pinned already (a member
    ///   that hasn't tagged itself again since a re-key, or one away across
    ///   several); a device removed knows the secrets before, so a new key it
    ///   vouches for is not taken, and the server can't make one up.
    ///
    /// What a device says of sync (`SyncCap`) is in its tag too: it counts
    /// only from a tag that checks, and only for a trusted device.
    ///
    /// A box key badly signed by the key its id is the hash of leaves its
    /// device out altogether.
    fn read_devices(&mut self, f: &Frame) {
        let names = self.keys.member.as_ref().map(|m| m.keys.names);
        let me = self.keys.me.id();
        let mut devices = Vec::new();
        let mut mine_signed = false;
        let mut mine_tag = String::new();
        let pins_before = self.pins.clone();
        for d in f.list("devices") {
            let mut p = d.split(':');
            let Some(id) = p.next().and_then(unhex::<10>) else { continue };
            let Some(kind) = p.next().map(str::to_string) else { continue };
            let name = p.next().and_then(unb64url).and_then(|s| names.and_then(|k| open(&k, b"kolada/v1/name", &s))).and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
            let Some(listed) = p.next().and_then(unb64url).and_then(|b| <[u8; 32]>::try_from(b).ok()) else { continue };
            let signed = match (p.next().filter(|k| !k.is_empty()), p.next().filter(|s| !s.is_empty())) {
                (Some(k), Some(sig)) => {
                    let k = unb64url(k).and_then(|b| <[u8; 32]>::try_from(b).ok());
                    let sig = unb64url(sig).and_then(|b| <[u8; 64]>::try_from(b).ok());
                    match (k, sig) {
                        (Some(k), Some(sig)) if sha256(&k)[..10] == id && verify(&k, box_text(&id, &listed).as_bytes(), &sig) => Some(true),
                        _ => Some(false),
                    }
                }
                _ => None,
            };
            let sync = SyncCap::of(p.next().unwrap_or(""));
            let tag = p.next().unwrap_or("");
            if id == me {
                mine_signed = signed == Some(true);
                mine_tag = tag.to_string();
                self.sync_said = sync == self.sync;
            }
            if signed == Some(false) {
                continue;
            }
            let pinned = self.pins.iter().find(|(i, _)| *i == id).map(|(_, b)| *b);
            // The tag covers the box key and what the device says of sync,
            // so neither the server nor another member can change either.
            let (now_tag, before_tag) = self.tag_checks(tag, &id, &listed, sync);
            let (box_pub, trusted, sync) = if id == me {
                (listed, true, self.sync)
            } else if now_tag {
                (listed, true, sync)
            } else if pinned == Some(listed) {
                // The key the group vouched for before (a device offline
                // across re-keys, or one not yet tagged again): still
                // trusted; what it says of sync, only from a tag that checks.
                (listed, true, if before_tag { sync } else { SyncCap::Off })
            } else {
                (pinned.unwrap_or(listed), false, SyncCap::Off)
            };
            if trusted {
                self.pins.retain(|(i, _)| *i != id);
                self.pins.push((id, box_pub));
            }
            devices.push(Device { id, kind, name, box_pub, sync, trusted });
        }
        self.devices = devices;
        // A device no longer listed loses its pin: the server listing it
        // again with its old key doesn't make it trusted.
        let listed: Vec<DeviceId> = self.devices.iter().map(|d| d.id).collect();
        self.pins.retain(|(i, _)| listed.contains(i));
        if self.pins != pins_before {
            self.save();
        }
        self.say_sync();
        // This device vouches for itself under the current secret, when
        // the list doesn't already say so (each epoch, once a connection).
        let want = self.my_tag();
        if want.is_some() && want.as_deref() != Some(mine_tag.as_str()) {
            self.say_tag();
        }
        // A device from before box keys were signed signs its own, once.
        if !mine_signed && !self.box_sent && self.keys.member.as_ref().is_some_and(|m| m.registered) && self.links[0].state == LinkState::Open {
            self.box_sent = true;
            let sig = b64url(&self.box_sig());
            self.out.push((Channel::Group, Frame::new("device").with("op", "box").with("sig", sig.as_str()).encode()));
        }
        self.events.push(Event::DevicesChanged);
    }

    /// The roster tag this device gives itself under the group's current
    /// secret: `epoch.tag` (`roster_tag`), or None outside a group. The
    /// server lists it; tests' stand-in server takes it from here.
    pub fn my_tag(&self) -> Option<String> {
        let m = self.keys.member.as_ref()?;
        let me = self.keys.me.id();
        Some(format!("{}.{}", m.keys.epoch, b64url(&roster_tag(&m.secret, &m.group, m.keys.epoch, &me, &self.keys.me.box_pub(), self.sync))))
    }

    /// Whether a listed tag checks under the current secret, and under the
    /// one before, for this box key and what the device says of sync.
    fn tag_checks(&self, tag: &str, id: &DeviceId, box_pub: &[u8; 32], sync: SyncCap) -> (bool, bool) {
        let Some(m) = self.keys.member.as_ref() else { return (false, false) };
        let Some((epoch, mac)) = tag.split_once('.') else { return (false, false) };
        let (Ok(epoch), Some(mac)) = (epoch.parse::<u32>(), unb64url(mac)) else { return (false, false) };
        let check = |secret: &[u8; 32]| same(&mac, &roster_tag(secret, &m.group, epoch, id, box_pub, sync));
        if epoch == m.keys.epoch {
            return (check(&m.secret), false);
        }
        match &m.prev {
            Some((secret, keys)) if keys.epoch == epoch => (false, check(secret)),
            _ => (false, false),
        }
    }

    fn say_tag(&mut self) {
        let registered = self.keys.member.as_ref().is_some_and(|m| m.registered);
        let tag = self.my_tag();
        if !registered || self.links[0].state != LinkState::Open || tag.is_none() || self.tag_said == tag {
            return;
        }
        self.tag_said = tag.clone();
        self.out.push((Channel::Group, Frame::new("device").with("op", "tag").with("tag", tag.unwrap_or_default().as_str()).encode()));
    }

    /// Whether this device syncs notes, told to the group now if connected,
    /// else on the next connection. A desktop says `On` where note sync is
    /// switched on; a phone `Follow`.
    pub fn set_sync(&mut self, cap: SyncCap) {
        if cap != self.sync {
            self.sync = cap;
            self.sync_said = false;
            // Its tag covers it: a new one goes too.
            self.say_tag();
        }
        self.say_sync();
    }

    /// What this device says of note sync, as the server lists it (tests'
    /// stand-in server).
    pub fn sync_word(&self) -> &'static str {
        self.sync.word()
    }

    fn say_sync(&mut self) {
        let registered = self.keys.member.as_ref().is_some_and(|m| m.registered);
        if self.sync_said || !registered || self.links[0].state != LinkState::Open {
            return;
        }
        self.sync_said = true;
        self.out.push((Channel::Group, Frame::new("device").with("op", "caps").with("caps", self.sync.word()).encode()));
    }

    /// The devices this one syncs notes with now: every other that says
    /// `On` or `Follow`, once one in the group (this one too) says `On`.
    /// Empty while this device says `Off`, or nobody says `On`.
    pub fn sync_peers(&self) -> Vec<DeviceId> {
        let me = self.keys.me.id();
        // Only devices the group vouched for, and only what their tags say.
        let others = self.devices.iter().filter(|d| d.id != me && d.trusted);
        if self.sync == SyncCap::Off || !(self.sync == SyncCap::On || others.clone().any(|d| d.sync == SyncCap::On)) {
            return Vec::new();
        }
        others.filter(|d| d.sync != SyncCap::Off).map(|d| d.id).collect()
    }

    /// This device's box key, signed with its own key (`box_text`).
    fn box_sig(&self) -> [u8; 64] {
        self.keys.me.sign(box_text(&self.keys.me.id(), &self.keys.me.box_pub()).as_bytes())
    }

    fn send_name(&mut self) {
        let Some(m) = &self.keys.member else { return };
        let sealed = seal(&m.keys.names, b"kolada/v1/name", self.name.as_bytes());
        self.out.push((Channel::Group, Frame::new("device").with("op", "name").with("name", b64url(&sealed)).encode()));
        self.name_sent = true;
    }

    pub fn rename_self(&mut self, name: &str) {
        self.name = name.into();
        self.name_sent = false;
        if self.links[0].state == LinkState::Open {
            self.send_name();
        }
    }

    /// Registers an iPhone's APNs token with the group.
    /// Kept, and sent now if connected, else on the next connection; sent
    /// again on every connection after (the server keeps it once).
    pub fn push_token(&mut self, token_hex: &str, sandbox: bool) {
        self.push = Some((token_hex.to_string(), sandbox));
        if self.links[0].state == LinkState::Open {
            self.send_push();
        }
    }

    fn send_push(&mut self) {
        let Some((token, sandbox)) = &self.push else { return };
        let f = Frame::new("device").with("op", "push").with("token", token.as_str()).with("env", if *sandbox { "sandbox" } else { "production" });
        self.out.push((Channel::Group, f.encode()));
    }

    /// Removes a device from the group, and re-keys it so that device can't
    /// read anything sent from now on.
    pub fn remove(&mut self, device: DeviceId) {
        // Its pinned key goes: listed again later, it is a stranger.
        self.pins.retain(|(i, _)| *i != device);
        let Some(m) = self.keys.member.as_mut() else { return };
        let me = self.keys.me.id();
        let secret: [u8; 32] = random();
        let epoch = m.keys.epoch + 1;
        let proof = rekey_proof(&m.secret, &m.group, epoch, &secret);
        let mut plain = W::default();
        plain.raw(&secret).u32(epoch).raw(&proof);
        // Only to devices the group vouched for: one the server made up
        // never gets the new secret.
        for d in self.devices.iter().filter(|d| d.id != device && d.id != me && d.trusted) {
            let id: ItemId = random();
            let f = Frame::new("send").with("id", hex(&id)).with("kind", "k").with("to", hex(&d.id)).body(seal_to(&d.box_pub, &plain.0));
            self.out.push((Channel::Group, f.encode()));
        }
        self.out.push((Channel::Group, Frame::new("device").with("op", "remove").with("dev", hex(&device)).encode()));
        m.prev = Some((m.secret, m.keys.clone()));
        m.secret = secret;
        m.keys = GroupKeys::new(m.group, &secret, epoch);
        self.devices.retain(|d| d.id != device);
        self.keys_changed();
        self.say_tag();
    }

    /// Leaves the group: removed by another device, or the user unpaired.
    fn leave(&mut self) {
        self.keys.member = None;
        self.devices.clear();
        self.outbox.clear();
        // Nothing of the group stays: no pinned keys, no batches held.
        self.pins.clear();
        for (at, id, _) in std::mem::take(&mut self.held) {
            if let Some(path) = self.held_path(at, &id) {
                let _ = std::fs::remove_file(path);
            }
        }
        self.held_bytes = 0;
        self.sync_flying.clear();
        self.cursor = 0;
        self.keys_changed();
        self.events.push(Event::Removed);
        self.save();
    }

    // Quick Send.

    fn seal_send(&mut self, what: Outgoing, to: Option<&[DeviceId]>, now: u64) -> Option<(ItemId, Frame)> {
        let (kind, text) = match what {
            Outgoing::Text(t) => (if is_link(&t) { Kind::Link } else { Kind::Text }, t),
            Outgoing::Secret(t) => (Kind::Secret, t),
            Outgoing::Picture { .. } | Outgoing::Note { .. } => return None,
        };
        self.seal_text(kind, text, String::new(), to, now)
    }

    /// A text item's frame and card: a note is a text with its name as the
    /// title (a link's title is the page's, and only links have one else).
    fn seal_text(&mut self, kind: Kind, text: String, title: String, to: Option<&[DeviceId]>, now: u64) -> Option<(ItemId, Frame)> {
        let m = self.keys.member.as_ref()?;
        let id: ItemId = random();
        let me = self.keys.me.id();
        let env = seal_item(&m.keys, b'q', &id, &me, &encode_payload(kind, now, &text, &title))?;
        self.cards.push(Card { id, from: me, at: now, kind, text, title, read: true, copied: false, mine: true, lost: false, picture: None });
        self.trim();
        let mut f = Frame::new("send").with("id", hex(&id)).with("kind", "q");
        if let Some(to) = to.filter(|t| !t.is_empty()) {
            f = f.with("to", to.iter().map(|d| hex(d)).collect::<Vec<_>>().join(","));
        }
        Some((id, f.body(env)))
    }

    /// Sends now if connected, else as soon as the socket opens. None if not
    /// paired, or too big for the largest bucket.
    pub fn send(&mut self, what: Outgoing, now: u64) -> Option<ItemId> {
        self.send_to(what, None, now)
    }

    /// As `send`, to these devices only (None: every other device).
    pub fn send_to(&mut self, what: Outgoing, to: Option<&[DeviceId]>, now: u64) -> Option<ItemId> {
        let (id, f, pictures) = match what {
            Outgoing::Picture { bytes, mime, name, width, height, thumb } => return self.send_picture(bytes, mime, name, width, height, thumb, to, None, now),
            Outgoing::Note { title, text, pictures } => {
                let title = if title.trim().is_empty() { "Note".to_string() } else { title };
                let (id, f) = self.seal_text(Kind::Text, text, title, to, now)?;
                (id, f, pictures)
            }
            other => {
                let (id, f) = self.seal_send(other, to, now)?;
                (id, f, Vec::new())
            }
        };
        let bytes = f.encode();
        self.outbox.push((id, bytes.clone()));
        if self.links[0].state == LinkState::Open {
            self.out.push((Channel::Group, bytes));
        }
        // A note's pictures follow it, each tied to it.
        for p in pictures {
            self.send_picture(p.bytes, p.mime, p.name, p.width, p.height, p.thumb, to, Some(id), now);
        }
        self.save();
        Some(id)
    }

    /// The pictures that went with the note `id` (its picture cards), in the
    /// order they came.
    pub fn note_pictures(&self, id: ItemId) -> Vec<&Card> {
        self.cards.iter().filter(|c| c.picture.as_ref().is_some_and(|p| p.note == Some(id))).collect()
    }

    /// One request instead of a socket: the share extension, widget and Control.
    pub fn send_post(&mut self, what: Outgoing, now: u64) -> Option<(ItemId, Post)> {
        let (id, f) = self.seal_send(what, None, now)?;
        let group = hex(&self.keys.member.as_ref()?.group);
        let path = format!("/v1/g/{group}/send");
        let body = f.encode();
        let headers = self.signed("POST", &path, &body, now);
        Some((id, Post { url: format!("{}{path}", self.base), headers, body }))
    }

    /// A picture: sealed with a key of its own, its upload asked for (a
    /// ticket from the group, then a PUT the shell does: `transfers`), and
    /// its item sent once it is up (`put_done`). None if not paired, too
    /// big, or with no bytes.
    #[allow(clippy::too_many_arguments)]
    fn send_picture(&mut self, bytes: Vec<u8>, mime: String, name: String, width: u32, height: u32, thumb: Vec<u8>, to: Option<&[DeviceId]>, note: Option<ItemId>, now: u64) -> Option<ItemId> {
        if self.keys.member.is_none() || bytes.is_empty() || bytes.len() > PICTURE_MOST || thumb.len() > THUMB_MOST {
            return None;
        }
        let (card, blob, key): (ItemId, ItemId, [u8; 32]) = (random(), random(), random());
        let sealed = seal_stream(&key, &blob_aad(&blob), &bytes);
        let picture = Picture { blob, key, mime, name: name.clone(), size: bytes.len() as u64, width, height, thumb, note };
        let me = self.keys.me.id();
        self.keep_picture(card, &bytes);
        self.cards.push(Card { id: card, from: me, at: now, kind: Kind::Picture, text: name, title: String::new(), read: true, copied: false, mine: true, lost: false, picture: Some(picture) });
        self.trim();
        self.uploads.push(Upload { card, blob, sealed, to: to.filter(|t| !t.is_empty()).map(<[DeviceId]>::to_vec), asked: false });
        self.ask_tickets(false);
        self.save();
        Some(card)
    }

    /// Asks the group for a ticket for each transfer waiting on one (all of
    /// them again on a new socket: `again`).
    fn ask_tickets(&mut self, again: bool) {
        if self.links[0].state != LinkState::Open {
            return;
        }
        for u in &mut self.uploads {
            if again || !u.asked {
                u.asked = true;
                self.out.push((Channel::Group, Frame::new("ticket").with("id", hex(&u.blob)).with("op", "put").with("n", u.sealed.len()).encode()));
            }
        }
        for (_, blob, asked, _) in &mut self.fetches {
            if again || !*asked {
                *asked = true;
                self.out.push((Channel::Group, Frame::new("ticket").with("id", hex(blob)).with("op", "get").encode()));
            }
        }
    }

    /// A ticket from the group: the transfer it was for, for the shell.
    fn ticket(&mut self, f: &Frame) {
        let (Some(blob), Some(op), Some(exp), Some(sig)) = (f.get("id").and_then(unhex::<16>), f.get("op"), f.num("exp"), f.get("sig")) else { return };
        let Some(m) = &self.keys.member else { return };
        let mut url = format!("{}/v1/g/{}/blob/{}?exp={exp}&sig={sig}", self.base, hex(&m.group), hex(&blob));
        // The size the ticket binds (a put's), which the server checks the upload against.
        if let Some(n) = f.num("n") {
            url.push_str(&format!("&n={n}"));
        }
        match op {
            "put" => {
                if let Some(u) = self.uploads.iter().find(|u| u.blob == blob) {
                    self.transfers.push(Transfer::Put { blob, url, body: u.sealed.clone() });
                }
            }
            "get" => {
                if self.fetches.iter().any(|f| f.1 == blob) {
                    self.transfers.push(Transfer::Get { blob, url });
                }
            }
            _ => {}
        }
    }

    /// The transfers to do now: a PUT or GET each, reported back with
    /// `put_done` or `got`.
    pub fn transfers(&mut self) -> Vec<Transfer> {
        std::mem::take(&mut self.transfers)
    }

    /// A picture's upload ended with this HTTP status (0: no answer): at
    /// 200 its item goes; else the card goes, and `SendFailed` says why
    /// ("pictures": resting until tomorrow).
    pub fn put_done(&mut self, blob: ItemId, status: u16, now: u64) {
        let Some((card, f)) = self.uploaded(blob, status, now) else { return };
        let bytes = f.encode();
        self.outbox.push((card, bytes.clone()));
        if self.links[0].state == LinkState::Open {
            self.out.push((Channel::Group, bytes));
        }
        self.save();
    }

    /// A picture's upload ended: at 200, its item's frame; else its card
    /// goes, and `SendFailed` says why.
    fn uploaded(&mut self, blob: ItemId, status: u16, now: u64) -> Option<(ItemId, Frame)> {
        let at = self.uploads.iter().position(|u| u.blob == blob)?;
        let up = self.uploads.remove(at);
        let card = self.cards.iter().find(|c| c.id == up.card).cloned();
        let (Some(card), Some(m)) = (card, self.keys.member.as_ref()) else { return None };
        let pic = card.picture.clone()?;
        if status != 200 {
            self.cards.retain(|c| c.id != up.card);
            self.events.push(Event::SendFailed(up.card, if status == 503 { "pictures".into() } else { "upload".into() }));
            self.save();
            return None;
        }
        let me = self.keys.me.id();
        let payload = encode_picture(now, &pic);
        let env = seal_item(&m.keys, b'q', &up.card, &me, &payload)?;
        let mut f = Frame::new("send").with("id", hex(&up.card)).with("kind", "q").with("blob", hex(&blob));
        if let Some(to) = &up.to {
            f = f.with("to", to.iter().map(|d| hex(d)).collect::<Vec<_>>().join(","));
        }
        Some((up.card, f.body(env)))
    }

    // The share extension: a process with no socket, and only requests.

    /// A picture from a process with no socket (the share extension):
    /// sealed and kept as `send` does, with the request for its ticket to
    /// POST. Then `post_answer` with that answer's body, the PUT that
    /// `transfers` gives, and `put_posted` for the item's request. None if
    /// not paired, not a picture, too big, or with no bytes.
    pub fn post_picture(&mut self, what: Outgoing, now: u64) -> Option<(ItemId, Post)> {
        let Outgoing::Picture { bytes, mime, name, width, height, thumb } = what else { return None };
        let card = self.send_picture(bytes, mime, name, width, height, thumb, None, None, now)?;
        let up = self.uploads.iter_mut().find(|u| u.card == card)?;
        up.asked = true;
        let (blob, size) = (up.blob, up.sealed.len());
        let group = hex(&self.keys.member.as_ref()?.group);
        let path = format!("/v1/g/{group}/ticket/{}", hex(&blob));
        // Its size goes with it: the ticket binds it.
        let body = format!("put {size}").into_bytes();
        let headers = self.signed("POST", &path, &body, now);
        Some((card, Post { url: format!("{}{path}", self.base), headers, body }))
    }

    /// The body of the answer to `post_picture`'s request: its ticket. The
    /// upload is then in `transfers`.
    pub fn post_answer(&mut self, body: &[u8]) {
        if let Some(f) = Frame::parse(body).filter(|f| f.t() == "ticket") {
            self.ticket(&f);
        }
    }

    /// A picture's upload from a process with no socket ended with this
    /// HTTP status: at 200, the request for its item to POST (as
    /// `send_post` gives one); else its card goes, as `put_done` says.
    pub fn put_posted(&mut self, blob: ItemId, status: u16, now: u64) -> Option<Post> {
        let (_, f) = self.uploaded(blob, status, now)?;
        let group = hex(&self.keys.member.as_ref()?.group);
        let path = format!("/v1/g/{group}/send");
        let body = f.encode();
        let headers = self.signed("POST", &path, &body, now);
        self.save();
        Some(Post { url: format!("{}{path}", self.base), headers, body })
    }

    /// What this process sent (`id`), for the app to list as its own: the
    /// share extension writes it to the App Group, and the app takes it in
    /// (`take_handoff`). Sealed with this device's own key, which both
    /// hold; a picture's bytes go with it. None if there is no such card.
    pub fn handoff(&self, id: ItemId) -> Option<Vec<u8>> {
        let c = self.cards.iter().find(|c| c.id == id && c.mine)?;
        let mut w = W::default();
        w.raw(HANDOFF).raw(&c.id).u64(c.at).u8(kind_byte(c.kind)).str(&c.text).str(&c.title);
        match &c.picture {
            Some(p) => {
                w.u8(1).bytes(&encode_picture(c.at, p)).bytes(&self.picture(id).unwrap_or_default());
            }
            None => {
                w.u8(0);
            }
        }
        Some(seal_state(&self.keys.local, &w.0))
    }

    /// A send the share extension handed off (`handoff`): listed here as
    /// this device's own, once. Its id, if it was taken in.
    pub fn take_handoff(&mut self, sealed: &[u8]) -> Option<ItemId> {
        let plain = open_state(&self.keys.local, sealed)?;
        let mut r = R(&plain);
        (r.arr::<8>()? == *HANDOFF).then_some(())?;
        let (id, at) = (r.arr::<16>()?, r.u64()?);
        let kind = kind_of(r.u8()?)?;
        let (text, title) = (r.str()?, r.str()?);
        let (picture, bytes) = match r.u8()? {
            1 => (Some(decode_picture(r.bytes()?)?), r.bytes()?.to_vec()),
            _ => (None, Vec::new()),
        };
        if self.cards.iter().any(|c| c.id == id) {
            return None;
        }
        if !bytes.is_empty() {
            self.keep_picture(id, &bytes);
        }
        let me = self.keys.me.id();
        let at_most = self.cards.iter().position(|c| c.at > at).unwrap_or(self.cards.len());
        self.cards.insert(at_most, Card { id, from: me, at, kind, text, title, read: true, copied: false, mine: true, lost: false, picture });
        self.trim();
        self.save();
        Some(id)
    }

    /// A picture's sealed bytes came down (or None: they could not, and
    /// it is asked for again on the next connection). Opened, it is kept
    /// here (`picture`) and its item acknowledged.
    pub fn got(&mut self, blob: ItemId, sealed: Option<Vec<u8>>) {
        let Some(at) = self.fetches.iter().position(|f| f.1 == blob) else { return };
        let Some(sealed) = sealed else {
            self.fetches[at].2 = false;
            return;
        };
        let (card, _, _, _) = self.fetches.remove(at);
        let key = self.cards.iter().find(|c| c.id == card).and_then(|c| c.picture.as_ref()).map(|p| p.key);
        match key.and_then(|key| open_stream(&key, &blob_aad(&blob), &sealed)) {
            Some(bytes) => {
                self.keep_picture(card, &bytes);
                self.events.push(Event::PictureReady(card));
            }
            None => {
                self.cards.retain(|c| c.id != card);
                self.events.push(Event::FetchFailed(card));
                self.save();
            }
        }
    }

    /// A card's picture, as kept here (its file's bytes), once it is.
    pub fn picture(&self, id: ItemId) -> Option<Vec<u8>> {
        if let Some((_, b)) = self.kept.iter().find(|(c, _)| *c == id) {
            return Some(b.clone());
        }
        let path = self.picture_path(&id)?;
        open_state(&self.keys.local, &std::fs::read(path).ok()?)
    }

    /// Whether a card's picture is kept here yet.
    pub fn has_picture(&self, id: ItemId) -> bool {
        self.kept.iter().any(|(c, _)| *c == id) || self.picture_path(&id).is_some_and(|p| p.exists())
    }

    fn picture_path(&self, id: &ItemId) -> Option<PathBuf> {
        self.data.as_ref().map(|d| d.join("pictures").join(format!("{}.bin", hex(id))))
    }

    /// Keeps a picture's bytes, sealed with this device's own key.
    fn keep_picture(&mut self, id: ItemId, bytes: &[u8]) {
        match self.picture_path(&id) {
            Some(path) => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&path, seal_state(&self.keys.local, bytes));
            }
            None => self.kept.push((id, bytes.to_vec())),
        }
    }

    /// Lets go of the pictures of cards that are gone.
    fn drop_pictures(&mut self) {
        let live: Vec<ItemId> = self.cards.iter().filter(|c| c.picture.is_some()).map(|c| c.id).collect();
        self.kept.retain(|(c, _)| live.contains(c));
        let Some(dir) = self.data.as_ref().map(|d| d.join("pictures")) else { return };
        let Ok(files) = std::fs::read_dir(&dir) else { return };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".bin").and_then(unhex::<16>);
            if id.is_none_or(|id| !live.contains(&id)) {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }

    /// Newest first.
    pub fn cards(&self) -> Vec<&Card> {
        self.cards.iter().rev().collect()
    }

    pub fn unread(&self) -> usize {
        self.cards.iter().filter(|c| !c.read).count()
    }

    pub fn seen(&mut self, id: ItemId) {
        if let Some(c) = self.cards.iter_mut().find(|c| c.id == id && !c.read) {
            c.read = true;
            self.save();
        }
    }

    /// The text to put on the clipboard. A password gives it once, then
    /// never again, and tells the other devices and the server to drop it.
    pub fn copy(&mut self, id: ItemId) -> Option<String> {
        let c = self.cards.iter_mut().find(|c| c.id == id)?;
        c.read = true;
        if c.kind == Kind::Secret {
            if c.copied || c.mine {
                return None;
            }
            c.copied = true;
            let text = std::mem::take(&mut c.text);
            self.out.push((Channel::Group, Frame::new("consumed").with("id", hex(&id)).encode()));
            self.save();
            return Some(text);
        }
        c.copied = true;
        let text = c.text.clone();
        self.save();
        Some(text)
    }

    /// A clip saved as a note (it is a note now), or removed, leaves here.
    /// Only this device's history: the others keep theirs.
    /// One of this device's beams again, as a new one to every other
    /// device, its card in place of the old: a failed or lost one's resend.
    /// A picture goes again from the copy kept here. The new id, or None
    /// (not this device's, or its picture no longer kept).
    pub fn resend(&mut self, id: ItemId, now: u64) -> Option<ItemId> {
        let c = self.cards.iter().find(|c| c.id == id && c.mine)?.clone();
        let mut gone = vec![id];
        let what = match (&c.picture, c.kind) {
            (Some(p), _) => Outgoing::Picture { bytes: self.picture(id)?, mime: p.mime.clone(), name: c.text.clone(), width: p.width, height: p.height, thumb: p.thumb.clone() },
            (None, Kind::Secret) => Outgoing::Secret(c.text.clone()),
            // A note, with the pictures that went with it.
            (None, Kind::Text) if !c.title.is_empty() => {
                let pictures = self.note_pictures(id).into_iter().filter_map(|pc| {
                    let p = pc.picture.as_ref()?;
                    Some((pc.id, NotePicture { name: pc.text.clone(), mime: p.mime.clone(), bytes: self.picture(pc.id)?, width: p.width, height: p.height, thumb: p.thumb.clone() }))
                }).collect::<Vec<_>>();
                gone.extend(pictures.iter().map(|(i, _)| *i));
                Outgoing::Note { title: c.title.clone(), text: c.text.clone(), pictures: pictures.into_iter().map(|(_, p)| p).collect() }
            }
            (None, _) => Outgoing::Text(c.text.clone()),
        };
        for g in gone {
            self.outbox.retain(|(i, _)| *i != g);
            self.uploads.retain(|u| u.card != g);
            self.forget(g);
        }
        self.send_to(what, None, now)
    }

    pub fn forget(&mut self, id: ItemId) -> Option<Card> {
        let at = self.cards.iter().position(|c| c.id == id)?;
        let card = self.cards.remove(at);
        self.drop_pictures();
        self.save();
        Some(card)
    }

    /// The history, newest first, filtered by text (passwords never match).
    pub fn history(&self, query: &str) -> Vec<&Card> {
        let q = query.to_lowercase();
        self.cards.iter().rev().filter(|c| c.kind != Kind::Secret && (q.is_empty() || c.text.to_lowercase().contains(&q) || c.title.to_lowercase().contains(&q))).collect()
    }

    fn trim(&mut self) {
        let before = self.cards.len();
        if self.cards.len() > HISTORY {
            let extra = self.cards.len() - HISTORY;
            self.cards.drain(..extra);
        }
        keep_pictures_under(&mut self.cards, PICTURES_KEPT_MOST);
        if self.cards.len() != before {
            self.drop_pictures();
        }
    }

    // Pairing.

    /// Shows a code on this device: `Event::Code` follows once the server
    /// gave a slot, and `Event::Asks` when another device enters it.
    pub fn show_code(&mut self) {
        self.pairing = Pairing::Showing { secret: code::new_secret(), slot: None, opener: None, expires: 0, bad: 0 };
        self.links[1] = Link::new();
    }

    /// Joins with a code typed on this device.
    pub fn enter_code(&mut self, typed: &str) -> Result<(), CodeError> {
        let code = code::parse(typed)?;
        self.pairing = Pairing::Entering { code, session: None };
        self.links[1] = Link::new();
        Ok(())
    }

    pub fn cancel_pairing(&mut self) {
        self.pairing = Pairing::None;
        self.out.retain(|(c, _)| *c != Channel::Pair);
    }

    /// The user's answer to `Event::Asks`.
    pub fn allow(&mut self, yes: bool, now: u64) {
        let Pairing::Asking { session, intro } = std::mem::replace(&mut self.pairing, Pairing::None) else { return };
        if !yes {
            self.out.push((Channel::Pair, Frame::new("deny").encode()));
            self.out.push((Channel::Pair, Frame::new("done").encode()));
            return;
        }
        if self.keys.member.is_none() {
            // The first pairing makes the group; it registers before the
            // other device can.
            self.keys.member = Some(Member::new(random(), random(), 1, true, Vec::new()));
            self.keys_changed();
        }
        self.pairing = Pairing::Welcoming { session, intro };
        if self.keys.member.as_ref().is_some_and(|m| m.registered) && self.links[0].state == LinkState::Open {
            self.welcome(now);
        }
    }

    fn welcome(&mut self, now: u64) {
        let Pairing::Welcoming { session, intro } = std::mem::replace(&mut self.pairing, Pairing::None) else { return };
        let Some(m) = &self.keys.member else { return };
        let w = Welcome { group: m.group, secret: m.secret, epoch: m.keys.epoch, voucher: pair::voucher(&self.keys.me, &m.group, &intro, now) };
        self.out.push((Channel::Pair, Frame::new("welcome").body(pair::welcome(&session, &w)).encode()));
        self.out.push((Channel::Pair, Frame::new("done").encode()));
        self.events.push(Event::Paired);
    }

    fn pair_frame(&mut self, f: Frame, now: u64) {
        match (&mut self.pairing, f.t()) {
            (Pairing::Showing { secret, slot, opener, expires, .. }, "slot") => {
                let Some(s) = f.get("slot").filter(|s| s.len() == code::SLOT_LEN) else { return };
                let code = Code { slot: s.to_string(), secret: secret.clone() };
                let (o, msg_a) = Opener::new(code.clone());
                *slot = Some(s.to_string());
                *opener = Some(o);
                *expires = f.num("expires").unwrap_or(now + 300_000);
                let ev = Event::Code { code: code.show(), expires: *expires };
                self.out.push((Channel::Pair, Frame::new("pake").body(msg_a).encode()));
                self.events.push(ev);
            }
            (Pairing::Showing { opener: Some(o), bad, .. }, "confirm") => match o.check(&f.body) {
                Ok((session, intro)) => {
                    self.events.push(Event::Asks { name: intro.name.clone(), kind: intro.kind.clone() });
                    self.pairing = Pairing::Asking { session, intro };
                }
                Err(()) => {
                    // A wrong code: tell the server (it counts the try), and
                    // start again for the next one; after the fifth, this
                    // device gives the code up itself.
                    *bad += 1;
                    self.out.push((Channel::Pair, Frame::new("bad").encode()));
                    if *bad >= PAIR_TRIES {
                        self.pairing = Pairing::None;
                        self.events.push(Event::CodeRefused(Refusal::Burned));
                        return;
                    }
                    let (next, msg_a) = o.again();
                    *o = next;
                    self.out.push((Channel::Pair, Frame::new("pake").body(msg_a).encode()));
                }
            },
            (Pairing::Showing { .. } | Pairing::Asking { .. }, "gone") => {
                let why = match f.get("why") {
                    Some("burned") => Refusal::Burned,
                    _ => Refusal::Expired,
                };
                self.pairing = Pairing::None;
                self.events.push(Event::CodeRefused(why));
            }
            (Pairing::Entering { code, session }, "pake") => {
                let intro = Intro::of(&self.keys.me, &self.kind, &self.name);
                match pair::join(code, &f.body, &intro) {
                    Some((body, s)) => {
                        *session = Some(s);
                        self.out.push((Channel::Pair, Frame::new("confirm").body(body).encode()));
                    }
                    None => {
                        self.pairing = Pairing::None;
                        self.events.push(Event::CodeRefused(Refusal::WrongCode));
                    }
                }
            }
            (Pairing::Entering { session: Some(s), .. }, "welcome") => match pair::read_welcome(s, &f.body) {
                Some(w) => {
                    self.keys.member = Some(Member::new(w.group, w.secret, w.epoch, false, w.voucher));
                    self.pairing = Pairing::None;
                    self.links[0] = Link::new();
                    self.keys_changed();
                    self.events.push(Event::Paired);
                }
                None => {
                    self.pairing = Pairing::None;
                    self.events.push(Event::CodeRefused(Refusal::WrongCode));
                }
            },
            (Pairing::Entering { .. }, "refused") => {
                self.pairing = Pairing::None;
                self.events.push(Event::CodeRefused(Refusal::WrongCode));
            }
            (Pairing::Entering { .. }, "deny") => {
                self.pairing = Pairing::None;
                self.events.push(Event::CodeRefused(Refusal::Denied));
            }
            (Pairing::Entering { .. }, "gone") => {
                self.pairing = Pairing::None;
                self.events.push(Event::CodeRefused(Refusal::Expired));
            }
            _ => {}
        }
    }

    // State on disk.

    fn state_path(&self) -> Option<PathBuf> {
        self.data.as_ref().map(|d| d.join("quick-send.state"))
    }

    fn save(&self) {
        let Some(path) = self.state_path() else { return };
        let mut w = W::default();
        w.u64(self.cursor).u32(self.cards.len() as u32);
        for c in &self.cards {
            // Mine: 1, or 2 when it didn't arrive.
            w.raw(&c.id).raw(&c.from).u64(c.at).u8(kind_byte(c.kind)).u8(c.read as u8).u8(c.copied as u8).u8(c.mine as u8 + (c.mine && c.lost) as u8).str(&c.text).str(&c.title);
        }
        w.u32(self.seen.len() as u32);
        for (id, at) in &self.seen {
            w.raw(id).u64(*at);
        }
        w.u32(self.outbox.len() as u32);
        for (id, f) in &self.outbox {
            w.raw(id).bytes(f);
        }
        // A picture still waiting to go up is not kept: its bytes are not,
        // so after a restart it could never go (its card goes with it).
        let pictures: Vec<&Card> = self.cards.iter().filter(|c| c.picture.is_some() && !self.uploads.iter().any(|u| u.card == c.id)).collect();
        w.u32(pictures.len() as u32);
        for c in pictures {
            if let Some(p) = &c.picture {
                w.raw(&c.id).bytes(&encode_picture(c.at, p));
            }
        }
        // The pinned box keys, last: an older app stops reading before them.
        w.u32(self.pins.len() as u32);
        for (id, b) in &self.pins {
            w.raw(id).raw(b);
        }
        let sealed = seal_state(&self.keys.local, &w.0);
        let tmp = path.with_extension("tmp");
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&tmp, sealed).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    fn load(&mut self) {
        let Some(path) = self.state_path() else { return };
        let Ok(b) = std::fs::read(&path) else { return };
        let Some(plain) = open_state(&self.keys.local, &b) else { return };
        let mut r = R(&plain);
        let read = |r: &mut R, e: &mut Engine| -> Option<()> {
            e.cursor = r.u64()?;
            for _ in 0..r.u32()? {
                let (id, from, at) = (r.arr()?, r.arr()?, r.u64()?);
                let kind = kind_of(r.u8()?)?;
                let (read, copied, own) = (r.u8()? == 1, r.u8()? == 1, r.u8()?);
                let (text, title) = (r.str()?, r.str()?);
                e.cards.push(Card { id, from, at, kind, text, title, read, copied, mine: own >= 1, lost: own == 2, picture: None });
            }
            for _ in 0..r.u32()? {
                e.seen.push((r.arr()?, r.u64()?));
            }
            for _ in 0..r.u32()? {
                let id = r.arr()?;
                e.outbox.push((id, r.bytes()?.to_vec()));
            }
            for _ in 0..r.u32()? {
                let id: ItemId = r.arr()?;
                let p = decode_picture(r.bytes()?);
                if let Some(c) = e.cards.iter_mut().find(|c| c.id == id) {
                    c.picture = p;
                }
            }
            // A picture card whose picture did not come back has nothing to show.
            e.cards.retain(|c| c.kind != Kind::Picture || c.picture.is_some());
            // Pins, if this file has them (one from before has none).
            if let Some(n) = r.u32() {
                for _ in 0..n {
                    e.pins.push((r.arr()?, r.arr()?));
                }
            }
            Some(())
        };
        let _ = read(&mut r, self);
    }
}

/// Whether an outbox frame is a sync batch (sent paced: `Engine::pace`).
fn is_sync(frame: &[u8]) -> bool {
    Frame::parse(frame).is_some_and(|f| f.get("kind") == Some("s"))
}

/// What a device signs to vouch for its own box key (the server's `boxText`).
fn box_text(id: &DeviceId, box_pub: &[u8; 32]) -> String {
    format!("kolada/v1/box|{}|{}", hex(id), b64url(box_pub))
}

/// A device's roster tag: an HMAC under a key only the group's members
/// hold (from its secret at `epoch`) over its id and box key. A member
/// vouches for itself with it; the server, which never has the secret,
/// can't make one for a device it made up.
fn roster_tag(secret: &[u8; 32], group: &[u8; 16], epoch: u32, id: &DeviceId, box_pub: &[u8; 32], sync: SyncCap) -> [u8; 32] {
    hmac(&hkdf(group, secret, "kolada/v1/roster"), &[b"kolada/v1/roster|", &epoch.to_be_bytes(), id, box_pub, b"|", sync.word().as_bytes()])
}

fn rekey_proof(old: &[u8; 32], group: &[u8; 16], epoch: u32, new: &[u8; 32]) -> [u8; 32] {
    hmac(&hkdf(group, old, "kolada/v1/rekey-proof"), &[&epoch.to_be_bytes(), new])
}

/// 1 s, 2 s, 4 s ... up to 5 minutes, each between half and all of that.
pub fn backoff(attempts: u32) -> u64 {
    let full = (1000u64 << attempts.saturating_sub(1).min(9)).min(BACKOFF_MAX_MS);
    let r = u16::from_be_bytes(random()) as u64;
    full / 2 + full / 2 * r / 65535
}

fn is_link(t: &str) -> bool {
    let t = t.trim();
    (t.starts_with("https://") || t.starts_with("http://")) && !t.contains(char::is_whitespace)
}

fn kind_byte(k: Kind) -> u8 {
    match k {
        Kind::Text => 1,
        Kind::Link => 2,
        Kind::Secret => 3,
        Kind::Picture => 4,
    }
}

fn kind_of(b: u8) -> Option<Kind> {
    Some(match b {
        1 => Kind::Text,
        2 => Kind::Link,
        3 => Kind::Secret,
        4 => Kind::Picture,
        _ => return None,
    })
}

/// A Quick Send's plaintext: `kind | sent at | text | title`.
pub fn encode_payload(kind: Kind, at: u64, text: &str, title: &str) -> Vec<u8> {
    let mut w = W::default();
    w.u8(kind_byte(kind)).u64(at).str(text).str(title);
    w.0
}

/// A picture's item: the payload (its name as the text, its type as the
/// title), then `blob | key | size | width | height | thumbnail`.
fn encode_picture(at: u64, p: &Picture) -> Vec<u8> {
    let mut w = W::default();
    w.u8(kind_byte(Kind::Picture)).u64(at).str(&p.name).str(&p.mime);
    w.raw(&p.blob).raw(&p.key).u64(p.size).u32(p.width).u32(p.height).bytes(&p.thumb);
    // The note it went with, last: an app from before stops reading before it.
    if let Some(note) = &p.note {
        w.raw(note);
    }
    w.0
}

fn decode_picture(b: &[u8]) -> Option<Picture> {
    let mut r = R(b);
    (kind_of(r.u8()?)? == Kind::Picture).then_some(())?;
    let _at = r.u64()?;
    let (name, mime) = (r.str()?, r.str()?);
    let (blob, key) = (r.arr()?, r.arr()?);
    let (size, width, height) = (r.u64()?, r.u32()?, r.u32()?);
    let thumb = r.bytes()?.to_vec();
    let note = r.arr::<16>();
    Some(Picture { blob, key, mime, name, size, width, height, thumb, note })
}

/// A handoff's first bytes, inside its seal.
const HANDOFF: &[u8; 8] = b"KHANDOF1";

/// What a picture's sealed bytes are bound to: its blob.
fn blob_aad(blob: &ItemId) -> Vec<u8> {
    [&b"kolada/v1/blob"[..], blob].concat()
}

pub fn decode_payload(b: &[u8]) -> Option<(Kind, String, String)> {
    let mut r = R(b);
    let kind = kind_of(r.u8()?)?;
    let _at = r.u64()?;
    Some((kind, r.str()?, r.str()?))
}

/// Takes the oldest picture clips out of `cards` (oldest first) until the
/// pictures left come to `most` bytes or less.
fn keep_pictures_under(cards: &mut Vec<Card>, most: u64) {
    let mut total: u64 = cards.iter().filter_map(|c| c.picture.as_ref()).map(|p| p.size).sum();
    while total > most {
        let Some(at) = cards.iter().position(|c| c.picture.is_some()) else { break };
        total -= cards.remove(at).picture.map_or(0, |p| p.size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pictures are kept as long as their clips, but no more than so much
    /// in all: past it, the oldest pictures go, and nothing else does.
    #[test]
    fn the_oldest_pictures_go_once_the_kept_ones_pass_their_budget() {
        let card = |n: u8, size: Option<u64>| Card {
            id: [n; 16],
            from: [0; 10],
            at: n as u64,
            kind: if size.is_some() { Kind::Picture } else { Kind::Text },
            text: String::new(),
            title: String::new(),
            read: false,
            copied: false,
            mine: false,
            lost: false,
            picture: size.map(|size| Picture { blob: [n; 16], key: [0; 32], mime: "image/png".into(), name: String::new(), size, width: 1, height: 1, thumb: Vec::new(), note: None }),
        };
        let mut cards = vec![card(1, Some(40)), card(2, None), card(3, Some(40)), card(4, Some(30))];
        keep_pictures_under(&mut cards, 110);
        assert_eq!(cards.len(), 4, "under the budget, all stay");
        keep_pictures_under(&mut cards, 60);
        assert_eq!(cards.iter().map(|c| c.id[0]).collect::<Vec<_>>(), [2, 4], "the oldest pictures went, the text stayed");
    }

    #[test]
    fn backoff_doubles_to_five_minutes_with_jitter() {
        for (n, full) in [(1u32, 1000u64), (2, 2000), (3, 4000), (9, 256_000), (10, 300_000), (40, 300_000)] {
            for _ in 0..50 {
                let b = backoff(n);
                assert!(b >= full / 2 && b <= full, "attempt {n}: {b}");
            }
        }
    }

    #[test]
    fn a_link_is_one_url_alone() {
        assert!(is_link("https://kolada.app/x"));
        assert!(is_link("  http://a.b "));
        assert!(!is_link("see https://kolada.app"));
        assert!(!is_link("https://a.b and more"));
    }

    #[test]
    fn payloads_round_trip() {
        let p = encode_payload(Kind::Link, 5, "https://a.b", "A page");
        assert_eq!(decode_payload(&p), Some((Kind::Link, "https://a.b".into(), "A page".into())));
    }
}
