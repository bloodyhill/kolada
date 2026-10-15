//! Share link: Send's first item opens the share card under the button
//! (`kolada_ui::share`): the links kept here show at once while the
//! server's list comes; Create sends the note (see `crate::share`) and the
//! link it comes back with is put on the clipboard. Escape, or a click
//! beside it, closes the card.

use kolada_ui::share::{Card, Command, Hit, ShareView};

use super::*;
use crate::share::{self as sending, Answer, Outgoing};

/// The card, and when it opened (it fades in as a menu does).
pub struct Open {
    card: Card,
    opened: Instant,
}

impl App {
    /// Share link…: the card, under Send.
    pub(super) fn open_share_card(&mut self) {
        let (expiry, links) = sending::kept();
        let note = self.session.note.clone();
        self.share_card = Some(Open { card: Card::new(expiry, &note, &links, kolada_core::session::now()), opened: Instant::now() });
        self.ui.options_open = false;
        sending::list(self.hwnd);
        self.request_frame();
    }

    pub(super) fn close_share_card(&mut self) -> bool {
        let open = self.share_card.take().is_some();
        if open {
            self.request_frame();
        }
        open
    }

    /// While it fades in, frames keep coming.
    pub(super) fn share_card_fading(&self) -> bool {
        self.share_card.as_ref().is_some_and(|s| s.opened.elapsed() < menu::FADE + Duration::from_millis(20))
    }

    pub(super) fn share_view(&self) -> Option<ShareView<'_>> {
        let s = self.share_card.as_ref()?;
        Some(ShareView { card: &s.card, alpha: chrome::ease(s.opened.elapsed().as_secs_f32() / menu::FADE.as_secs_f32()), now: kolada_core::session::now() })
    }

    fn keep_links(&self) {
        if let Some(s) = &self.share_card {
            sending::keep(s.card.expiry, &s.card.links);
        }
    }

    /// A click on the card.
    pub(super) fn share_click(&mut self, hit: Hit) {
        let Some(s) = &mut self.share_card else { return };
        let before = s.card.expiry;
        let command = s.card.click(hit);
        let chose = s.card.expiry != before;
        match command {
            Command::Nothing => {}
            Command::Close => {
                self.close_share_card();
            }
            Command::Create(expiry) => {
                let (text, title) = (self.editor.text().to_string(), self.editor.title());
                let vault = PathBuf::from(&self.session.vault);
                let path = self.note_path();
                let dir = path.parent().filter(|d| !self.session.note.is_empty() && d.starts_with(&vault)).map_or_else(|| vault.clone(), Path::to_path_buf);
                sending::create(self.hwnd, Outgoing { text, vault, dir, note: self.session.note.clone(), title }, expiry);
            }
            Command::Copy(url) => {
                clipboard::set(self.hwnd, &url);
            }
            Command::Delete(id) => sending::delete(self.hwnd, id),
        }
        if chose {
            self.keep_links();
        }
        self.request_frame();
    }

    /// What a request came back with.
    pub(super) fn share_arrived(&mut self, answer: Answer) {
        let now = kolada_core::session::now();
        let Some(s) = &mut self.share_card else {
            // Closed meanwhile: a link made or deleted is still kept.
            let (expiry, mut links) = sending::kept();
            match answer {
                Answer::Created(Ok(link)) => {
                    clipboard::set(self.hwnd, &link.url());
                    self.whisper(format!("Copied {}", kolada_core::share::short(&link.url())));
                    links.push(link);
                }
                Answer::Deleted(id, Ok(())) => links.retain(|l| l.id != id),
                Answer::Listed(Ok(listed)) => links = kolada_core::share::merge(&links, listed),
                _ => return,
            }
            sending::keep(expiry, &kolada_core::share::live(&links, now));
            return;
        };
        match answer {
            Answer::Created(Ok(link)) => {
                clipboard::set(self.hwnd, &link.url());
                s.card.created(link, now);
            }
            Answer::Created(Err(p)) => {
                s.card.failed(&p);
                // Limits are counted on the server: its list says which links count.
                if matches!(p, kolada_core::share::Problem::Limit { .. }) {
                    sending::list(self.hwnd);
                }
            }
            Answer::Listed(Ok(listed)) => {
                let merged = kolada_core::share::merge(&s.card.links, listed);
                s.card.listed(&merged, now);
            }
            // Offline: what is kept here stays shown.
            Answer::Listed(Err(_)) => {}
            Answer::Deleted(id, result) => s.card.deleted(&id, result, now),
        }
        self.keep_links();
        self.request_frame();
    }
}
