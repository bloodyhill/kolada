//! The welcome screen on Windows (`kolada_ui::welcome`): shown on a first
//! run in place of a note, and from the palette's Welcome. A tile picks
//! what it reads with Windows' own pickers, then brings it in on a worker
//! (`kolada_convert::bring`) that posts how far it has come
//! (`WM_APP_BRINGING`) and what it came to (`WM_APP_BROUGHT`); documents
//! convert in a process of their own, as dropped ones do (`convert`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kolada_convert::bring::{self, Kind, Progress};
use kolada_ui::welcome::{self, Command, From, Hit, Outcome, Pick, Welcome};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use super::App;
use crate::{dialog, window};

/// An import under way: how far it has come.
pub struct Bringing {
    pub progress: Arc<Progress>,
    /// What it reads from, and how the line names it: a welcome opened
    /// again while it runs picks its line up.
    pub from: From,
    pub said: String,
}

/// What an import came to, posted back from its worker.
pub struct Brought {
    pub from: From,
    pub result: Result<bring::Brought, String>,
}

/// How often the worker says how far it has come.
const EVERY: Duration = Duration::from_millis(80);

/// What `kolada_convert::bring` reads for a tile, from what was picked.
fn kind(from: From, picked: &[PathBuf]) -> Option<Kind> {
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
    .filter(|_| !picked.is_empty())
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

fn is_note(f: &Path) -> bool {
    f.extension().is_some_and(|e| ["md", "markdown", "txt"].contains(&e.to_string_lossy().to_ascii_lowercase().as_str()))
}

impl App {
    /// A first run's welcome, after the first frame, in place of a note.
    pub(super) fn show_welcome(&mut self) {
        if self.compact || self.welcome.is_some() {
            return;
        }
        self.ui.options_open = false;
        // The writing model's first-run card gives way: the welcome offers
        // it, with the rest.
        if self.model_card.as_ref().is_some_and(|c| matches!(c.card.phase, kolada_ui::modelcard::Phase::Choose { .. })) {
            self.model_card = None;
        }
        let mut w = Welcome::new();
        if let Some(b) = &self.bringing {
            w.began(b.from, &b.said);
            w.progress(b.progress.get());
        }
        self.welcome = Some(w);
        // The writing model is offered on it, so its first-run card is not.
        self.session.model_asked = true;
        self.request_frame();
    }

    /// Settings or the palette opened over it: it goes, the page left as
    /// it is.
    pub(super) fn welcome_away(&mut self) {
        if self.welcome.take().is_some() {
            self.request_frame();
        }
    }

    pub(super) fn welcome_click(&mut self, hit: Hit) {
        let Some(w) = &mut self.welcome else { return };
        match w.click(hit) {
            Command::None => {}
            Command::Redraw => self.request_frame(),
            Command::Pick(from) => self.welcome_pick(from),
            Command::Stop => {
                if let Some(b) = &self.bringing {
                    b.progress.stop();
                }
            }
            Command::Start => self.start_writing(),
        }
    }

    /// "Start writing" (or Esc): the welcome goes, a fresh page in its
    /// place. An import under way goes on, and says when it is done. The
    /// writing model was offered here, so its first-run card is not.
    pub(super) fn start_writing(&mut self) {
        if self.welcome.take().is_none() {
            return;
        }
        self.session.model_asked = true;
        if !self.session.note.is_empty() || !self.editor.text().is_empty() {
            self.fresh_page();
        }
        self.request_frame();
    }

    fn welcome_pick(&mut self, from: From) {
        let picked = match from.pick() {
            Pick::Folder(title) => dialog::pick_folder(self.hwnd, title).into_iter().collect(),
            Pick::Files { title, kind, exts, many } => {
                let pattern = exts.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(";");
                let files = dialog::open_files(self.hwnd, title, &[(kind, &pattern)]);
                if many { files } else { files.into_iter().take(1).collect() }
            }
            Pick::Nothing => Vec::new(),
        };
        self.bring_in(from, picked);
    }

    /// Files dropped while the welcome shows: brought in by what they are.
    pub(super) fn welcome_drop(&mut self, files: Vec<PathBuf>) {
        match welcome::dropped(&files) {
            Some((from, files)) if self.welcome.as_ref().is_some_and(|w| !w.busy()) => self.bring_in(from, files),
            Some(_) => {}
            None => self.whisper("Kolada cannot read those".to_string()),
        }
    }

    /// Brings `picked` in as `from` on a worker, into a new folder of the
    /// vault (made now if this is a first run).
    pub(super) fn bring_in(&mut self, from: From, picked: Vec<PathBuf>) {
        let Some(kind) = kind(from, &picked) else { return };
        // One at a time.
        if self.bringing.is_some() {
            return;
        }
        let Some(vault) = self.ensure_vault() else { return };
        let Some(pictures) = self.attachments_for(&vault) else { return };
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
        self.bringing = Some(Bringing { progress: progress.clone(), from, said: said.clone() });
        self.request_frame();
        let work = crate::Handoff((self.hwnd, picked, vault, pictures, folder));
        std::thread::spawn(move || {
            let work = work;
            let (hwnd, picked, vault, pictures, folder) = work.0;
            // How far it has come, every so often, while it works.
            let ticking = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let ticker = {
                let (ticking, progress, hwnd) = (ticking.clone(), progress.clone(), crate::Handoff(hwnd));
                std::thread::spawn(move || {
                    let hwnd = hwnd;
                    let mut last = (usize::MAX, 0);
                    while ticking.load(std::sync::atomic::Ordering::Relaxed) {
                        let now = progress.get();
                        if now != last {
                            last = now;
                            let _ = unsafe { PostMessageW(Some(hwnd.0), window::WM_APP_BRINGING, WPARAM(0), LPARAM(0)) };
                        }
                        std::thread::sleep(EVERY);
                    }
                })
            };
            let document = |file: &Path, dir: &Path| -> Result<Vec<PathBuf>, String> {
                if is_note(file) {
                    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let to = kolada_core::note::unique_path(dir, &stem, None);
                    return std::fs::copy(file, &to).map(|_| vec![to]).map_err(|_| "it could not be copied".to_string());
                }
                let done = crate::convert::run(std::slice::from_ref(&file.to_path_buf()), dir, &pictures);
                match done.failed.into_iter().next() {
                    Some((_, why)) if done.notes.is_empty() => Err(why),
                    _ => Ok(done.notes),
                }
            };
            let result = bring::bring(kind, &picked, &vault, &folder, &pictures, &progress, &document);
            ticking.store(false, std::sync::atomic::Ordering::Relaxed);
            let _ = ticker.join();
            let done = Box::into_raw(Box::new(Brought { from, result }));
            let posted = unsafe { PostMessageW(Some(hwnd), window::WM_APP_BROUGHT, WPARAM(0), LPARAM(done as isize)) };
            if posted.is_err() {
                drop(unsafe { Box::from_raw(done) });
            }
        });
    }

    /// The worker says how far it has come.
    pub(super) fn bringing_moved(&mut self) {
        let (Some(w), Some(b)) = (&mut self.welcome, &self.bringing) else { return };
        if w.progress(b.progress.get()) {
            self.request_frame();
        }
    }

    /// It is done: the line says what it came to (or, the welcome put
    /// away, a whisper does); the side menu shows the new folder.
    pub(super) fn brought(&mut self, done: Brought) {
        self.bringing = None;
        // Another app's export than the tile's: the line names whose.
        let whose = done.result.as_ref().ok().and_then(whose);
        if let (Some(w), Some((f, said))) = (&mut self.welcome, &whose) {
            w.found(*f, said);
        }
        let (from, said) = whose.unwrap_or_else(|| (done.from, done.from.said().to_string()));
        let outcome = match done.result {
            Ok(b) if b.stopped => Outcome::Stopped,
            Ok(b) => {
                let folder = b.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                Outcome::Brought { count: b.notes.len(), folder, failed: b.failed.len() }
            }
            Err(why) => Outcome::Failed(why),
        };
        if matches!(outcome, Outcome::Brought { .. }) {
            self.refresh_tree();
            self.start_index();
        }
        match &mut self.welcome {
            Some(w) => w.finished(outcome),
            None => {
                // Put away while it worked: what it came to, in a word.
                let mut w = Welcome::new();
                w.began(from, &said);
                w.finished(outcome);
                self.whisper(w.words());
            }
        }
        self.request_frame();
    }

    /// Keys while the welcome shows: Tab and Shift+Tab walk its controls,
    /// Enter and Space press the one Tab came to, Esc starts writing. It
    /// takes every key, so none reaches the page under it.
    pub(super) fn welcome_key(&mut self, vk: u16, shift: bool) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN, VK_TAB, VK_UP};
        let (places, height) = (self.welcome_places(), self.view_height());
        let Some(w) = &mut self.welcome else { return false };
        let k = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| key.0 == vk;
        if k(VK_ESCAPE) {
            self.start_writing();
        } else if k(VK_TAB) || k(VK_DOWN) || k(VK_UP) {
            let back = (k(VK_TAB) && shift) || k(VK_UP);
            w.focus = welcome::next_focus(&self.welcome_controls, w.focus, back);
            if let Some(t) = w.focus {
                welcome::reveal(w, &places, t, height);
            }
            self.request_frame();
        } else if k(VK_RETURN) {
            self.welcome_press();
        }
        // Space presses on its character (`welcome_char`), so none is left
        // over to be typed into the page.
        true
    }

    /// Enter or Space: the control Tab came to, if it is still there (a
    /// switch turned off hides the rows under it).
    fn welcome_press(&mut self) {
        let focus = self.welcome.as_ref().and_then(|w| w.focus).filter(|t| self.welcome_controls.iter().any(|c| c.1 == *t));
        if let Some(t) = focus {
            self.activate(t);
        }
    }

    /// A character while the welcome has the keys: Space presses, the rest
    /// are not typed anywhere.
    pub(super) fn welcome_char(&mut self, unit: u16) {
        if unit == 0x20 {
            self.welcome_press();
        }
    }

    /// The welcome's controls as last drawn, for `welcome::reveal`.
    fn welcome_places(&self) -> kolada_ui::scene::Scene {
        kolada_ui::scene::Scene { welcome_controls: self.welcome_controls.clone(), welcome_scroll_max: self.welcome_scroll_max, ..Default::default() }
    }

    /// The wheel over the welcome scrolls it.
    pub(super) fn welcome_wheel(&mut self, notches: f32) -> bool {
        let max = self.welcome_scroll_max;
        let Some(w) = &mut self.welcome else { return false };
        w.scroll = (w.scroll - notches * super::WHEEL_STEP).clamp(0.0, max);
        self.request_frame();
        true
    }
}
