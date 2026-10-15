//! Share link, as on Windows (`kolada-win` `app/link.rs`): the send menu's
//! first item opens the share card under Send (`kolada_ui::share`). The
//! links kept here show at once and the server's list follows; Create sends
//! the note on a thread of its own (its pictures through the OS's imaging,
//! `services.images.recode`; the request through `services.web.send`) and
//! the link it comes back with goes on the clipboard (an `Effect`). The
//! shell gives a wake (`on_share`) and calls `share_ready` when it fires.

use std::collections::HashMap;
use std::time::Instant;

use kolada_core::share::{self as shared, BASE, Expiry, LIMIT, Link, Problem};
use kolada_ui::share::{Card, Command, Hit, ShareView};

use super::share::Effect;
use super::*;

/// What a request came to.
enum Answer {
    Created(Result<Link, Problem>),
    Listed(Result<Vec<Link>, Problem>),
    Deleted(String, Result<(), Problem>),
}

#[derive(Default)]
pub(super) struct Sharing {
    /// The card, and when it opened (it fades in as a menu does).
    open: Option<(Card, Instant)>,
    /// What the threads came back with, for `share_ready`.
    slot: Arc<Mutex<Vec<Answer>>>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// The most an answer may weigh, and a picture file read for sharing.
const ANSWER: usize = 256 * 1024;
const PICTURE: u64 = 64 * 1024 * 1024;
/// How long the card takes to fade in.
const FADE: f32 = 0.09;

impl Model {
    /// The shell's wake for answers from notsy.co: it then calls `share_ready`.
    pub fn on_share(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.sharing.wake = Some(Arc::new(wake));
    }

    /// Where the key and the links are kept: the app's data, or for a model
    /// that keeps none (tests, dumps), beside the vault, never in the real one.
    fn share_dir(&self) -> PathBuf {
        self.data.clone().unwrap_or_else(|| self.vault.join(".kolada-share"))
    }

    fn kept_links(&self) -> (Expiry, Vec<Link>) {
        std::fs::read_to_string(shared::links_path(&self.share_dir())).map_or_else(|_| (Expiry::default(), Vec::new()), |t| shared::read_kept(&t))
    }

    fn keep_links(&self, expiry: Expiry, links: &[Link]) {
        let dir = self.share_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(shared::links_path(&dir), shared::kept_text(expiry, links));
    }

    fn key_if_made(&self) -> Option<String> {
        std::fs::read_to_string(shared::key_path(&self.share_dir())).ok().as_deref().and_then(shared::read_key)
    }

    /// Runs `work` on a thread; its answer waits for `share_ready`.
    fn share_work(&self, work: impl FnOnce() -> Answer + Send + 'static) {
        let (slot, wake) = (self.sharing.slot.clone(), self.sharing.wake.clone());
        std::thread::spawn(move || {
            let answer = work();
            slot.lock().unwrap().push(answer);
            if let Some(wake) = wake {
                wake();
            }
        });
    }

    /// Share link…: the card, under Send, and the server's list behind it
    /// (asked only once a link was made here).
    pub fn open_share(&mut self) -> bool {
        let (expiry, links) = self.kept_links();
        self.sharing.open = Some((Card::new(expiry, &self.session.note, &links, kolada_core::session::now()), Instant::now()));
        self.close_options();
        if let Some(key) = self.key_if_made() {
            let web = self.services.web.clone();
            self.share_work(move || Answer::Listed(shared::listed(web.send("GET", &format!("{BASE}/api/links"), &[("X-Kolada-Key", &key)], &[], ANSWER))));
        }
        true
    }

    pub fn close_share(&mut self) -> bool {
        self.sharing.open.take().is_some()
    }

    /// The card is open.
    pub fn sharing(&self) -> bool {
        self.sharing.open.is_some()
    }

    pub(super) fn share_view(&self) -> Option<ShareView<'_>> {
        let (card, opened) = self.sharing.open.as_ref()?;
        let alpha = if self.ui.animate { kolada_ui::chrome::ease(opened.elapsed().as_secs_f32() / FADE) } else { 1.0 };
        Some(ShareView { card, alpha, now: kolada_core::session::now() })
    }

    /// While it fades in, frames keep coming.
    pub fn share_fading(&self) -> bool {
        self.ui.animate && self.sharing.open.as_ref().is_some_and(|(_, t)| t.elapsed().as_secs_f32() < FADE + 0.02)
    }

    /// A click while the card is open: its own (anywhere: beside it, it
    /// closes). None when it is not open.
    pub(super) fn share_click(&mut self, hit: Option<Target>) -> Option<bool> {
        self.sharing.open.as_ref()?;
        let Some(Target::Share(hit)) = hit else {
            self.sharing.open = None;
            return Some(true);
        };
        Some(self.share_hit(hit))
    }

    fn share_hit(&mut self, hit: Hit) -> bool {
        let Some((card, _)) = self.sharing.open.as_mut() else { return false };
        let before = card.expiry;
        let command = card.click(hit);
        let (expiry, links) = (card.expiry, card.links.clone());
        match command {
            Command::Nothing => {}
            Command::Close => self.sharing.open = None,
            Command::Copy(url) => self.menus.effects.push(Effect::Clipboard(url, String::new())),
            Command::Create(expiry) => self.share_create(expiry),
            Command::Delete(id) => {
                let (web, key) = (self.services.web.clone(), self.key_if_made());
                self.share_work(move || {
                    let done = match key {
                        None => Ok(()),
                        Some(key) => shared::deleted(web.send("DELETE", &format!("{BASE}/api/links/{id}"), &[("X-Kolada-Key", &key)], &[], ANSWER)),
                    };
                    Answer::Deleted(id, done)
                });
            }
        }
        if expiry != before {
            self.keep_links(expiry, &links);
        }
        true
    }

    /// The open note goes up: made ready (pictures found as the page finds
    /// them, through the OS's imaging), then sent.
    fn share_create(&mut self, expiry: Expiry) {
        let text = self.editor.text().to_string();
        let (title, note) = (self.editor.title(), self.session.note.clone());
        let vault = self.vault.clone();
        let dir = self.path.as_ref().and_then(|p| p.parent()).filter(|d| d.starts_with(&vault)).map_or_else(|| vault.clone(), Path::to_path_buf);
        let (web, images, data) = (self.services.web.clone(), self.services.images.clone(), self.share_dir());
        self.share_work(move || {
            let made = (|| {
                let recode = images.recode().ok_or(Problem::Server)?;
                let mut known = None;
                let mut seen: HashMap<String, Option<PathBuf>> = HashMap::new();
                let mut read = |name: &str| -> Option<Vec<u8>> {
                    let path = seen.entry(name.to_string()).or_insert_with(|| kolada_ui::pictures::find(&vault, &dir, name, &mut known)).clone()?;
                    (std::fs::metadata(&path).ok()?.len() <= PICTURE).then(|| std::fs::read(&path).ok()).flatten()
                };
                let bundle = shared::bundle(&text, &mut read, &*recode, LIMIT)?;
                let key = shared::load_or_make_key(&data, || web.random()).ok_or(Problem::Server)?;
                let seed = web.random().map_or(kolada_core::session::now(), |b| u64::from_le_bytes(b[..8].try_into().unwrap()));
                let boundary = shared::boundary(&bundle, seed);
                let body = shared::multipart(&boundary, expiry, &bundle);
                let answer = web.send("POST", &format!("{BASE}/api/links"), &[("X-Kolada-Key", &key), ("Content-Type", &shared::content_type(&boundary))], &body, ANSWER);
                let c = shared::created(answer, &bundle)?;
                Ok(Link { id: c.id, created: kolada_core::session::now(), expires: c.expires, title, note })
            })();
            Answer::Created(made)
        });
    }

    /// Answers from notsy.co, taken in; whether the screen changes.
    pub fn share_ready(&mut self) -> bool {
        let answers = std::mem::take(&mut *self.sharing.slot.lock().unwrap());
        if answers.is_empty() {
            return false;
        }
        let now = kolada_core::session::now();
        for answer in answers {
            let (expiry, mut links) = match &self.sharing.open {
                Some((card, _)) => (card.expiry, card.links.clone()),
                None => self.kept_links(),
            };
            match (answer, self.sharing.open.as_mut()) {
                (Answer::Created(Ok(link)), card) => {
                    self.menus.effects.push(Effect::Clipboard(link.url(), format!("Copied {}", shared::short(&link.url()))));
                    match card {
                        Some((card, _)) => card.created(link, now),
                        None => links.push(link),
                    }
                }
                (Answer::Created(Err(p)), Some((card, _))) => card.failed(&p),
                (Answer::Listed(Ok(listed)), card) => {
                    let merged = shared::merge(&links, listed);
                    match card {
                        Some((card, _)) => card.listed(&merged, now),
                        None => links = merged,
                    }
                }
                (Answer::Deleted(id, result), card) => match card {
                    Some((card, _)) => card.deleted(&id, result, now),
                    None if result.is_ok() => links.retain(|l| l.id != id),
                    None => {}
                },
                _ => continue,
            }
            let links = match &self.sharing.open {
                Some((card, _)) => card.links.clone(),
                None => shared::live(&links, now),
            };
            self.keep_links(expiry, &links);
        }
        true
    }

    /// Waits for the threads' answers (tests: no shell wakes the model).
    #[cfg(test)]
    pub(super) fn share_settle(&mut self) {
        for _ in 0..500 {
            if !self.sharing.slot.lock().unwrap().is_empty() {
                self.share_ready();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{Images, Services, Web};
    use kolada_core::share::{Raster, Recode};

    /// notsy.co as the tests have it: the requests it was sent, and what it
    /// answers (no network).
    #[derive(Default)]
    struct Notsy {
        sent: Mutex<Vec<(String, String, Vec<u8>)>>,
    }

    impl Web for Notsy {
        fn get(&self, _: &str, _: usize) -> Option<Vec<u8>> {
            None
        }

        fn send(&self, method: &str, address: &str, headers: &[(&str, &str)], body: &[u8], _: usize) -> Option<(u16, Vec<u8>)> {
            assert!(headers.iter().any(|(k, v)| *k == "X-Kolada-Key" && v.len() == 43), "every request carries the key");
            self.sent.lock().unwrap().push((method.to_string(), address.to_string(), body.to_vec()));
            Some(match method {
                "POST" => (201, b"id=Y5tq\nurl=https://notsy.co/Y5tq\nexpires=4102444800\n".to_vec()),
                "GET" => (200, b"Y5tq\t1791000000\t4102444800\tTrip\n".to_vec()),
                _ => (204, Vec::new()),
            })
        }

        fn random(&self) -> Option<[u8; 32]> {
            Some([9; 32])
        }
    }

    /// Imaging that only knows sizes.
    struct Sizes;

    impl Recode for Sizes {
        fn decode(&self, b: &[u8], _: u32) -> Option<Raster> {
            kolada_core::share::dimensions(b).map(|(width, height)| Raster { width, height, bgra: vec![0, 0, 0, 255] })
        }
        fn encode(&self, _: &Raster, _: u8, _: bool) -> Option<Vec<u8>> {
            None
        }
    }

    impl Images for Sizes {
        fn decoder(&self) -> Option<Box<dyn kolada_ui::pictures::Decoder>> {
            None
        }
        fn recode(&self) -> Option<Box<dyn Recode>> {
            Some(Box::new(Sizes))
        }
    }

    #[test]
    fn a_note_is_shared_and_its_link_copied() {
        let dir = std::env::temp_dir().join(format!("kolada-app-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Trip.md"), "# Trip\n\nThe coast.\n").unwrap();
        let notsy = Arc::new(Notsy::default());
        let services = Services { web: notsy.clone(), images: Arc::new(Sizes), ..Services::none() };
        let mut m = Model::with_services(services, dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        // Opened: nothing asked of the server before a link was made here.
        assert!(m.open_share());
        m.frame(2000, 1440, 2.0);
        assert!(m.hits.iter().any(|(_, t)| *t == Target::Share(Hit::Create)), "the card is drawn");
        assert!(notsy.sent.lock().unwrap().is_empty());
        // Create: the note goes up, the link comes back on the clipboard.
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::Share(Hit::Expiry(Expiry::Day))).unwrap();
        m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false);
        m.share_hit(Hit::Create);
        m.share_settle();
        let sent = notsy.sent.lock().unwrap().clone();
        assert_eq!((sent[0].0.as_str(), sent[0].1.as_str()), ("POST", "https://notsy.co/api/links"));
        let body = String::from_utf8_lossy(&sent[0].2);
        assert!(body.contains("name=\"expires\"\r\n\r\n1\r\n"), "the day chosen");
        let effects = m.take_effects();
        assert!(matches!(effects.as_slice(), [Effect::Clipboard(url, _)] if url == "https://notsy.co/Y5tq"));
        assert!(matches!(&m.sharing.open, Some((c, _)) if c.links.len() == 1 && c.expiry == Expiry::Day));
        // Kept: the key, and the link with its note, beside the vault in a test.
        assert!(dir.join(".kolada-share").join("share.key").is_file());
        assert!(m.close_share());
        // Opened again: the link at once, then the server's list.
        m.open_share();
        assert!(matches!(&m.sharing.open, Some((c, _)) if c.links[0].note == "Trip.md" && c.expiry == Expiry::Day));
        m.share_settle();
        assert_eq!(notsy.sent.lock().unwrap().last().unwrap().0, "GET");
        // Deleted: asked, then gone.
        m.share_hit(Hit::Delete(0));
        m.share_hit(Hit::Delete(0));
        m.share_settle();
        assert_eq!(notsy.sent.lock().unwrap().last().unwrap().1, "https://notsy.co/api/links/Y5tq");
        assert!(matches!(&m.sharing.open, Some((c, _)) if c.links.is_empty()));
        // A click beside the card closes it.
        m.frame(2000, 1440, 2.0);
        assert!(m.pointer_down(5.0, 1400.0, 1, false));
        assert!(!m.sharing());
        let _ = std::fs::remove_dir_all(dir);
    }
}
