//! The welcome screen (`kolada_ui::welcome`), as Windows has it (`kolada-win`
//! `app/welcome.rs`): a first run opens on it in place of a note, and the
//! palette's Welcome opens it again. A tile asks the shell to pick what it
//! reads (`Request::Pick(Pick::Bring)`), the shell hands the pick to
//! `bring_in`, and a thread brings it in (`kolada_convert::bring`), saying
//! how far it has come as frames ask (`tick`) and what it came to through
//! the arrivals' channel. The switches and pairing on it are Settings'
//! own, handled as they are there.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kolada_convert::bring::{self, Kind, Progress};
use kolada_ui::welcome::{self, Command, From, Hit, Outcome, Welcome};

use super::arrive::Arrival;
use super::settings::Request;
use super::transfer::Pick;
use super::*;

/// How often a frame comes, while bringing in, to say how far it has got.
pub(super) const EVERY: Duration = Duration::from_millis(80);

/// An import the welcome began: how far it has come, and when the screen
/// last looked.
pub(super) struct Bringing {
    pub progress: Arc<Progress>,
    pub looked: Instant,
    /// What it reads from, and how the line names it: a welcome opened
    /// again while it runs picks its line up.
    pub from: From,
    pub said: String,
}

/// What `kolada_convert::bring` reads for a tile.
fn kind(from: From) -> Option<Kind> {
    use kolada_convert::import::Source;
    Some(match from {
        From::Obsidian => Kind::Vault,
        From::Folder => Kind::Folder,
        From::Evernote => Kind::Export(Source::Evernote),
        From::Notion => Kind::Export(Source::Notion),
        From::NotesApp => Kind::Nbk,
        From::Documents => Kind::Documents,
        From::Keep => Kind::Export(Source::GoogleKeep),
        From::Bear => Kind::Export(Source::Bear),
        // Asked of Notes itself, on the Mac (`Model::import_apple_notes`).
        From::AppleNotes => return None,
    })
}

/// What was brought in was not what its tile reads: another app's export
/// (said as that app's tile) or plain Markdown zipped (as a folder, by its
/// name). None when it was.
fn whose(b: &bring::Brought) -> Option<(From, String)> {
    use kolada_convert::import::Source;
    if b.plain {
        return Some((From::Folder, b.folder.file_name()?.to_string_lossy().into_owned()));
    }
    let from = match b.found? {
        Source::Evernote => From::Evernote,
        Source::Notion => From::Notion,
        Source::GoogleKeep => From::Keep,
        Source::Bear => From::Bear,
        Source::AppleNotes => From::AppleNotes,
    };
    Some((from, from.said().to_string()))
}

/// The keys the welcome takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WelcomeKey {
    /// Tab, or Shift+Tab (`true`).
    Tab(bool),
    /// Enter or Space: the control Tab came to.
    Press,
    Escape,
}

impl Model {
    /// The welcome screen, in place of the note (a first run's, or the
    /// palette's Welcome).
    pub fn show_welcome(&mut self) -> bool {
        if self.compact || self.welcome.is_some() || self.phone() {
            return false;
        }
        self.settings.open = false;
        // The writing model's first-run card gives way: the welcome offers
        // it, with the rest.
        if self.model_card.as_ref().is_some_and(|s| matches!(s.card.phase, kolada_ui::modelcard::Phase::Choose { .. })) {
            self.model_card = None;
        }
        let mut w = Welcome::new();
        if let Some(b) = &self.bringing {
            w.began(b.from, &b.said);
            w.progress(b.progress.get());
        } else if *self.apple_notes() == super::applenotes::AppleNotes::Running {
            w.began(From::AppleNotes, From::AppleNotes.said());
        }
        self.welcome = Some(w);
        // The writing model is offered on it, so its first-run card is not.
        self.session.model_asked = true;
        true
    }

    /// A phone or tablet's shell (the note bare in native chrome, or iOS):
    /// no welcome screen there, whose tiles it cannot pick for.
    pub(super) fn phone(&self) -> bool {
        self.bare || self.platform == kolada_ui::scene::Platform::Ios
    }

    pub fn welcome_open(&self) -> bool {
        self.welcome.is_some()
    }

    /// The welcome is over the page and has its keys (Settings and the
    /// palette not over it).
    pub(super) fn welcome_covers(&self) -> bool {
        self.welcome.is_some() && !self.settings.open && !self.search_has_keys()
    }

    /// Settings or the palette opened over it: it goes, as Start writing
    /// would, but leaves the page as it is.
    pub(super) fn welcome_away(&mut self) {
        self.welcome = None;
    }

    /// The welcome's view for the scene, with the writing model chosen.
    pub(super) fn welcome_view(&self) -> Option<welcome::View<'_>> {
        self.welcome.as_ref().map(|w| welcome::View { welcome: w, model: super::assist::chosen_index(&self.session) })
    }

    /// A press while the welcome shows: its own controls, and Settings'
    /// switches and choices on it. None when it is not up (or Settings is
    /// open over it), or for the window's own buttons.
    pub(super) fn welcome_click(&mut self, target: Option<Target>) -> Option<bool> {
        if self.welcome.is_none() || self.settings.open {
            return None;
        }
        Some(match target {
            Some(Target::Welcome(hit)) => self.welcome_hit(hit),
            Some(Target::Switch(setting)) => self.switch(setting),
            Some(Target::Choice(setting, value)) => self.choose(setting, value),
            // The window's own buttons, pairing (`quick_press`, as in
            // Settings) and the writing model's download strip (its Cancel
            // and Retry) are handled where they always are.
            Some(Target::Caption | Target::Minimize | Target::Maximize | Target::Close | Target::Quick(_) | Target::Model(_)) | None => return None,
            _ => false,
        })
    }

    fn welcome_hit(&mut self, hit: Hit) -> bool {
        let Some(w) = &mut self.welcome else { return false };
        match w.click(hit) {
            Command::None => false,
            Command::Redraw => true,
            // Apple Notes is asked of Notes itself: nothing to pick.
            Command::Pick(From::AppleNotes) => {
                if self.import_apple_notes()
                    && let Some(w) = &mut self.welcome
                {
                    w.began(From::AppleNotes, From::AppleNotes.said());
                }
                true
            }
            Command::Pick(from) => {
                self.settings.requests.push(Request::Pick(Pick::Bring(from)));
                false
            }
            Command::Stop => {
                if let Some(b) = &self.bringing {
                    b.progress.stop();
                }
                true
            }
            Command::Start => self.start_writing(),
        }
    }

    /// "Start writing" (or Esc): the welcome goes, a fresh page in its
    /// place. An import under way goes on, and says when it is done. The
    /// writing model was offered here, so its first-run card is not.
    pub fn start_writing(&mut self) -> bool {
        if self.welcome.take().is_none() {
            return false;
        }
        self.session.model_asked = true;
        if !self.session.note.is_empty() || !self.editor.text().is_empty() {
            self.new_note();
        }
        true
    }

    /// What the shell picked for a tile (`Request::Pick(Pick::Bring)`):
    /// brought in on a thread, into a new folder of the vault.
    pub fn bring_in(&mut self, from: From, picked: Vec<PathBuf>) -> bool {
        let Some(kind) = kind(from).filter(|_| !picked.is_empty()) else { return false };
        if self.bringing_in() {
            return false;
        }
        let _ = std::fs::create_dir_all(&self.vault);
        let (vault, pictures) = (self.vault.clone(), self.attachment_path());
        // A folder goes in under its own name, and is named by it.
        let (folder, said) = match from {
            From::Folder => {
                let name = picked[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".to_string());
                (name.clone(), name)
            }
            f => (f.folder(), f.said().to_string()),
        };
        let progress = Arc::new(Progress::default());
        if let Some(w) = &mut self.welcome {
            w.began(from, &said);
        }
        self.bringing = Some(Bringing { progress: progress.clone(), looked: Instant::now(), from, said: said.clone() });
        let send = self.arrivals_sender();
        let (exe, priority, in_app) = self.converter();
        std::thread::spawn(move || {
            let document = |file: &Path, dir: &Path| -> Result<Vec<PathBuf>, String> {
                if super::arrive::is_note_file(file) {
                    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let to = kolada_core::note::unique_path(dir, &stem, None);
                    return std::fs::copy(file, &to).map(|_| vec![to]).map_err(|_| "it could not be copied".to_string());
                }
                let one = std::slice::from_ref(&file.to_path_buf()).to_vec();
                let done = match in_app {
                    Some(most) => super::arrive::convert_guarded(&one, dir, &pictures, most),
                    None => exe.as_ref().and_then(|exe| super::arrive::convert_in_process(exe, &one, dir, &pictures, &*priority)).unwrap_or_else(|| super::arrive::convert_documents(&one, dir, &pictures)),
                };
                match done.failed.into_iter().next() {
                    Some((_, why)) if done.notes.is_empty() => Err(why),
                    _ => Ok(done.notes),
                }
            };
            let result = bring::bring(kind, &picked, &vault, &folder, &pictures, &progress, &document);
            send(Arrival::Brought { from, result });
        });
        true
    }

    /// An import under way, the welcome's or Apple Notes': one at a time.
    fn bringing_in(&self) -> bool {
        self.bringing.is_some() || *self.apple_notes() == super::applenotes::AppleNotes::Running
    }

    /// Files dropped while the welcome shows: brought in by what they are
    /// (a folder, an export, documents). Whether the screen changes.
    pub(super) fn welcome_drop(&mut self, files: &[PathBuf]) -> bool {
        match welcome::dropped(files) {
            Some(_) if self.bringing_in() => false,
            Some((from, files)) => self.bring_in(from, files),
            None => {
                self.whisper("Kolada cannot read those");
                true
            }
        }
    }

    /// While bringing in: how far it has come, for the line. Whether that
    /// changed.
    pub(super) fn bringing_tick(&mut self) -> bool {
        let Some(b) = &mut self.bringing else { return false };
        if b.looked.elapsed() < EVERY {
            return false;
        }
        b.looked = Instant::now();
        let now = b.progress.get();
        self.welcome.as_mut().is_some_and(|w| w.progress(now))
    }

    /// When the next look at an import's progress is due.
    pub(super) fn bringing_due(&self) -> Option<Instant> {
        self.bringing.as_ref().map(|b| b.looked + EVERY)
    }

    /// An import is done: the line says what it came to (or, the welcome
    /// put away, a whisper does); the list shows the new folder.
    pub(super) fn brought(&mut self, from: From, result: Result<bring::Brought, String>) -> bool {
        self.bringing = None;
        // Another app's export than the tile's: the line names whose.
        let whose = result.as_ref().ok().and_then(whose);
        if let (Some(w), Some((f, said))) = (&mut self.welcome, &whose) {
            w.found(*f, said);
        }
        let (from, said) = whose.unwrap_or_else(|| (from, from.said().to_string()));
        let outcome = match result {
            Ok(b) if b.stopped => Outcome::Stopped,
            Ok(b) => {
                let folder = b.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                Outcome::Brought { count: b.notes.len(), folder, failed: b.failed.len() }
            }
            Err(why) => Outcome::Failed(why),
        };
        if matches!(outcome, Outcome::Brought { .. }) {
            self.refresh_tree();
            *self.index.lock().unwrap() = None;
            self.index_started = false;
        }
        match &mut self.welcome {
            Some(w) => w.finished(outcome),
            None => {
                let mut w = Welcome::new();
                w.began(from, &said);
                w.finished(outcome);
                self.whisper(&w.words());
            }
        }
        true
    }

    /// A key while the welcome shows (Settings and the palette not over
    /// it): Tab walks its controls, Enter and Space press the one Tab came
    /// to, Esc starts writing. None when it is not up.
    pub fn welcome_key(&mut self, key: WelcomeKey) -> Option<bool> {
        if self.welcome.is_none() || self.settings.open || self.search_has_keys() {
            return None;
        }
        Some(match key {
            WelcomeKey::Escape => self.start_writing(),
            WelcomeKey::Tab(back) => {
                let (controls, max, height) = (self.welcome_controls.clone(), self.welcome_scroll_max, self.viewport.1);
                let Some(w) = &mut self.welcome else { return Some(false) };
                w.focus = welcome::next_focus(&controls, w.focus, back);
                if let Some(t) = w.focus {
                    let placed = kolada_ui::scene::Scene { welcome_controls: controls, welcome_scroll_max: max, ..Default::default() };
                    welcome::reveal(w, &placed, t, height);
                }
                true
            }
            // Only a control still on the screen (a switch turned off hides
            // the rows under it).
            WelcomeKey::Press => match self.welcome.as_ref().and_then(|w| w.focus).filter(|t| self.welcome_controls.iter().any(|c| c.1 == *t)) {
                Some(Target::Welcome(hit)) => self.welcome_hit(hit),
                Some(Target::Switch(setting)) => self.switch(setting),
                Some(Target::Choice(setting, value)) => self.choose(setting, value),
                Some(Target::Quick(h)) => {
                    let _ = self.quick_press(h);
                    true
                }
                _ => false,
            },
        })
    }

    /// Apple Notes' import ended: the welcome's line says what it came to.
    pub(super) fn apple_notes_on_welcome(&mut self, state: &super::applenotes::AppleNotes) {
        use super::applenotes::AppleNotes;
        let Some(w) = &mut self.welcome else { return };
        if !matches!(w.line, kolada_ui::welcome::Line::Bringing { from: From::AppleNotes, .. }) {
            return;
        }
        w.finished(match state {
            AppleNotes::Done(b) => Outcome::Brought { count: b.notes.len(), folder: kolada_convert::applenotes::FOLDER.to_string(), failed: 0 },
            AppleNotes::Refused => Outcome::Failed("allow Kolada in System Settings, Privacy & Security, Automation".to_string()),
            AppleNotes::Failed(why) => Outcome::Failed(why.clone()),
            AppleNotes::Idle | AppleNotes::Running => return,
        });
    }

    /// The wheel over the welcome scrolls it.
    pub(super) fn welcome_wheel(&mut self, dy: f32) -> bool {
        let max = self.welcome_scroll_max;
        let Some(w) = &mut self.welcome else { return false };
        w.scroll = (w.scroll - dy).clamp(0.0, max);
        true
    }
}

#[cfg(test)]
mod tests;
