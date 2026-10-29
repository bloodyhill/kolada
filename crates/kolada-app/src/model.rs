//! What the Mac shell shows, and what a click, a key or a scroll does to it.
//! No AppKit in here: it takes a vault, and gives back the pixels of a frame
//! by the shared pipeline (scene, recorded glyphs, blended into a surface),
//! so it also runs headless (`kolada-mac --dump`). Typing is the shared
//! editor's: the model only says where the caret is asked about, and saves.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use kolada_core::links;
use kolada_core::search::{self, Hit, Indexed};
use kolada_core::session::{self, RowKind, Session};
use kolada_core::trash::{self, Trashed};
use kolada_core::{note, vault};
use kolada_editor::{Editor, Motion, Selection};
use kolada_ui::edit::{OneLine, apply_tool};
use kolada_ui::note::{NoteText, title_line};
use kolada_ui::pictures::Pictures;
use kolada_ui::raster::Surface;
use kolada_ui::scene::{self, Eng, Fonts, Input, Line, Rect, SearchView, Target, Tool, TrashEntry};
use kolada_ui::{chrome, display, theme};

use crate::services::{Change, Services};

pub mod arrive;
pub mod access;
mod commands;
mod agenda;
mod assist;
mod bars;
mod card;
mod daily;
mod drawn;
mod files;
mod find;
mod formatting;
mod kept;
mod link;
mod linkfield;
mod modelcard;
mod page;
mod imports;
mod motion;
mod opening;
mod palette;
mod panels;
mod popup;
mod related;
pub mod quick;
pub mod share;
mod social;
mod suggest;
pub mod transfer;
mod visits;
mod pictures;
#[cfg(test)]
mod font;
mod scroll;
mod scrub;
#[cfg(test)]
mod sizes;
#[cfg(test)]
mod speed;
mod forever;
mod outside;
use find::Find;
pub use motion::WindowButton;

/// UTF-16 units before byte `at` of `s`.
pub fn utf16_at(s: &str, at: usize) -> usize {
    s[..at.min(s.len())].encode_utf16().count()
}

/// The byte offset of UTF-16 position `units` in `s` (clamped to its end, and
/// to the end of a character when it falls inside one).
pub fn byte_at(s: &str, units: usize) -> usize {
    let mut seen = 0;
    for (i, c) in s.char_indices() {
        if seen >= units {
            return i;
        }
        seen += c.len_utf16();
        if seen > units {
            return i + c.len_utf8();
        }
    }
    s.len()
}

/// The side menu's search: what is typed, what it found, which result is chosen.
struct Search {
    query: Editor,
    hits: Vec<Hit>,
    selected: usize,
    /// The first result shown (the list scrolls to keep the chosen one in view).
    first: usize,
    /// With `>`, commands instead (`palette.rs`): all there are while the
    /// palette is open, those shown, and what they do.
    commands: Option<Vec<palette::Command>>,
    shown: Vec<scene::MenuItem>,
    runs: Vec<palette::Run>,
    /// The hits are for the query as it is, from the whole index.
    fresh: bool,
    /// Whether the keys are the search's. A result opened with a click (or
    /// a click in the note after) leaves the search open, its query and
    /// results kept, and the keys to the note; the first edit there or Esc
    /// ends it (as on Windows).
    focused: bool,
}

/// How many results the search keeps.
const SEARCH_LIMIT: usize = 50;

/// How long ago `at` was, in a word or two.
fn when(at: u64, now: u64) -> String {
    let s = now.saturating_sub(at);
    match s {
        0..60 => "Just now".to_string(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        86_400..172_800 => "Yesterday".to_string(),
        _ => format!("{} days ago", s / 86_400),
    }
}

/// A file's modification time and length: when they change, another app
/// wrote it.
pub(super) type Stamp = (std::time::SystemTime, u64);

pub(super) fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// What goes in the file for `text`: without the name line `added` over it
/// while that line is untouched. Once edited, the line is part of the note
/// and `added` is cleared.
pub(super) fn file_text<'a>(text: &'a str, added: &mut Option<String>) -> std::borrow::Cow<'a, str> {
    match added.as_deref().map(|line| kolada_editor::title::file_text(text, line)) {
        Some(Some(file)) => file.into(),
        Some(None) => {
            *added = None;
            text.into()
        }
        None => text.into(),
    }
}

/// A note's text as it opens: its name as a first line over a text that
/// does not start with it (see `kolada_editor::title`), and that line.
pub(super) fn opened(path: &Path, file: String) -> (String, Option<String>) {
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    kolada_editor::title::open(&name, file)
}

/// Where writing starts in a note just opened: the line under its title.
pub(super) fn body_start(text: &str, lines: &[kolada_markdown::Line]) -> usize {
    match title_line(text, lines) {
        Some(t) if t + 1 < lines.len() => lines[t + 1].content().min(lines[t + 1].end),
        Some(t) => lines[t].end,
        None => 0,
    }
}

/// Where the session is kept between launches.
fn session_path() -> Option<PathBuf> {
    kolada_core::app_data_dir().map(|d| d.join("session.bin"))
}


/// The checker's misspellings less the app's own words (its name, the
/// welcome note's keys: `kolada_ui::spell::OWN_WORDS`), taken as spelled for
/// every run without touching the user's dictionary.
fn spelled_as_ours(units: &[u16], marks: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    marks
        .into_iter()
        .filter(|&(start, end)| {
            let word = units.get(start..end).map(String::from_utf16_lossy).unwrap_or_default();
            !kolada_ui::spell::OWN_WORDS.contains(&word.as_str())
        })
        .collect()
}

pub struct Model {
    eng: Eng,
    fonts: Fonts,
    session: Session,
    vault: PathBuf,
    editor: Editor,
    /// The open note's file (none for the welcome page), and how it is written.
    path: Option<PathBuf>,
    /// Which file the open note is, whatever it is called (to follow it
    /// when another app renames or moves it).
    file_id: Option<outside::FileId>,
    format: note::Format,
    /// The text has changed since it was saved.
    dirty: bool,
    /// The editor's version as the note was read: past it, the note was
    /// edited here (versions are unique to every change, never 0).
    opened_version: u64,
    /// The title the file is named for.
    named: String,
    /// The session is kept between launches (the app, not tests or dumps).
    persist: bool,
    /// Where the app keeps what it knows about notes outside the vault: their
    /// past. None in dumps, which write nothing.
    data: Option<PathBuf>,
    /// The open note's past (the time scrubber) and dates.
    past: scrub::Past,
    surface: Surface,
    pictures: RefCell<Pictures>,
    /// Device pixels per DIP, as of the last frame.
    scale: f32,
    /// Whether the screen is dark: the system's setting, unless the theme says.
    dark: bool,
    system_dark: bool,
    /// The last `dark` the window was told about.
    told_dark: Option<bool>,
    hover: Option<Target>,
    search: Option<Search>,
    /// Misspelled words by line, as far as lines have been checked.
    spelling: kolada_ui::spell::Marks,
    find: Option<Find>,
    /// Every note's text, read on a thread of its own when search first opens.
    index: Arc<Mutex<Option<Vec<Indexed>>>>,
    pub(super) index_started: bool,
    /// PDFs and pictures found by their text, after the notes (`files.rs`).
    files: files::Files,
    /// What other apps change in the vault.
    outside: Option<std::sync::mpsc::Receiver<Change>>,
    /// The trash, in place of the file tree, while it is open.
    trash: Option<(Vec<Trashed>, Vec<TrashEntry>)>,
    /// Notes just deleted, and the editor version they were deleted at: Cmd+Z
    /// brings the last back while nothing has been typed since.
    undo_delete: Vec<(Trashed, u64)>,
    /// What the OS gives the model: the calendar, the spell checker, fonts...
    services: Services,
    /// Moves a file to the OS's trash (the app's; none in tests and dumps).
    bin: Option<forever::Bin>,
    /// How far the note is scrolled: from the start of the paragraph at
    /// byte `scroll_line`, DIPs (so a frame never measures what is above it).
    scroll_line: usize,
    scroll: f32,
    /// Whether the last frame showed the end of the note.
    at_end: bool,
    /// The next frame scrolls the caret into view.
    reveal: bool,
    /// What the last frame drew: what is under each point, the paragraphs
    /// and where they went, the caret's box, and the note column's width.
    hits: Vec<(Rect, Target)>,
    /// The last frame's scene, kept while a screen reader asks what the
    /// controls are called (`access_controls`; off unless the shell asks).
    access_kept: bool,
    kept_scene: Option<scene::Scene>,
    lines: Vec<Line>,
    caret: Option<Rect>,
    column_width: f32,
    /// The viewport, DIPs.
    viewport: (f32, f32),
    /// The settings panel and the page of highlights (`settings.rs`), and
    /// what the right-click offers for a misspelled word (`spelling.rs`).
    settings: settings::State,
    fixes: spelling::Fixes,
    /// What threads bring back (fetched pictures, converted documents), and
    /// a picture's right edge being dragged (`arrive.rs`, `pictures.rs`).
    arrivals: arrive::Arrivals,
    resizing: Option<pictures::Resizing>,
    /// What moves between frames, the scroll bars, the pointer (`motion.rs`).
    ui: motion::Ui,
    /// The part of the text being composed that the input method is
    /// converting (bytes into it), for the page to underline.
    ime_target: Option<(usize, usize)>,
    /// A note open over the page (`popup.rs`): while open, it has the keys.
    popup: Option<popup::Popup>,
    /// A menu the model opened (`menu.rs`).
    menus: menu::Menus,
    /// The folder a fresh page saves into (vault-relative), when not the vault's top.
    fresh_dir: Option<String>,
    /// A note in a window of its own: no side menu, and a button that takes
    /// the note back to the main window.
    compact: bool,
    /// The note alone with none of Kolada's own chrome: the shell draws its
    /// own around it (the iPhone's native one).
    bare: bool,
    /// The OS the shell is (which options the settings offer): the Mac
    /// unless the shell says otherwise.
    platform: scene::Platform,
    /// The app's first run, on the default vault not made yet: the welcome
    /// note is written once arrivals start (`arrive.rs`).
    first_run: bool,
    /// The OS draws its window buttons over the top left (the Mac's traffic
    /// lights): our controls keep clear of them. Not in full screen.
    os_buttons: bool,
    /// The name line shown over a note whose text does not start with its
    /// name, kept out of its file until it is edited; the file's stamp as
    /// last read or written (another app's change since makes a conflict copy).
    added: Option<String>,
    disk: Option<Stamp>,
    /// The notes visited, back and forward (`visits.rs`).
    visits: visits::Visits,
    /// Notes suggested for the `[[link]]` being typed (`suggest.rs`).
    suggest: suggest::Suggesting,
    /// The side menu and the toolbar: docked, tucked away, revealed (`panels.rs`).
    panels: panels::Panels,
    /// The social preview (`social.rs`).
    social: social::Social,
    /// What is due, and the Today card (`agenda.rs`).
    agenda: agenda::Agenda,
    /// The share card under Send, and its answers from notsy.co (`link.rs`).
    sharing: link::Sharing,
    /// Related notes in the margin, and notes carried from the tree (`related.rs`).
    margin: related::Margin,
    /// Called from the index's thread once every note is read.
    index_wake: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Files dragged over the window, before they are let go (`card.rs`).
    arrival: Option<card::Arrival>,
    /// The writing model at work on the open note (`assist.rs`).
    writing: assist::Writing,
    /// The notes written last, for the Dock's menu (`opening.rs`).
    recent: Arc<Mutex<Vec<PathBuf>>>,
    /// The toolbar's Aa card, a button's name, the selection bar
    /// (`formatting.rs`).
    formatting: formatting::Formatting,
    /// The link field (Cmd+K) and the address card of a link rested on
    /// (`linkfield.rs`).
    linking: linkfield::Linking,
    /// The first run's writing model card (`modelcard.rs`).
    model_card: Option<modelcard::Shown>,
    /// Quick Send's popup (`quick.rs`): off until `start_quick`.
    quick: quick::Quick,
}

impl Model {
    /// Opens `vault` on no services (UTC, test fonts, no spell checker...): its
    /// newest top-level note, and the top of its file tree. For tests.
    pub fn new(vault: PathBuf, dark: bool) -> Model {
        Model::open_vault(Services::none(), vault, dark, None)
    }

    /// Opens `vault` on the shell's `services`: its newest top-level note,
    /// and the top of its file tree. What a dump runs on.
    pub fn with_services(services: Services, vault: PathBuf, dark: bool) -> Model {
        Model::open_vault(services, vault, dark, None)
    }

    /// The app's launch: the session as it was left (settings, the file
    /// tree, the note and where in it), if it was for this vault.
    pub fn launch(services: Services, vault: PathBuf, dark: bool) -> Model {
        // A first run: no session kept yet, and the default vault, not made yet.
        let first_run = session_path().is_some_and(|p| !p.exists()) && Some(&vault) == kolada_core::default_vault_dir().as_ref() && !vault.exists();
        let saved = session_path().and_then(|p| session::load(&p)).filter(|s| s.vault == vault.to_string_lossy());
        let mut model = Model::open_vault(services, vault, dark, saved);
        // As the settings say: the full window, or the minimal one.
        model.session.minimal = model.session.window_mode == 1;
        model.persist = true;
        model.data = kolada_core::app_data_dir();
        model.use_the_os_trash();
        model.ui.animate = true;
        model.first_run = first_run;
        model
    }

    /// A note's own window: the note at `rel` alone, saved as it is typed;
    /// the main window's session is left as it is.
    pub fn note_window(services: Services, vault: PathBuf, rel: &str, dark: bool) -> Model {
        let mut model = Model::open_vault(services, vault, dark, None);
        model.open(rel);
        model.compact = true;
        model.data = kolada_core::app_data_dir();
        model.use_the_os_trash();
        model.ui.animate = true;
        model
    }

    /// The vault as the settings show it: where it is, or on iOS, which
    /// keeps vaults in places with no path worth reading (the app's
    /// container, the Files app's providers), the folder's name.
    #[cfg(test)]
    pub(super) fn vault_label(&self) -> String {
        kolada_ui::chrome::vault_shown(self.platform, &self.session.vault).to_string()
    }

    /// The vault this shows.
    pub fn vault(&self) -> &Path {
        &self.vault
    }

    /// Removing a note for good from Kolada's trash moves it to the OS's.
    fn use_the_os_trash(&mut self) {
        let bin = self.services.bin.clone();
        self.bin = Some(Box::new(move |path| bin.recycle(path)));
    }

    fn open_vault(services: Services, vault: PathBuf, dark: bool, saved: Option<Session>) -> Model {
        let mut session = Session { vault: vault.to_string_lossy().into_owned(), ..Session::default() };
        let resumed = saved.as_ref().map(|s| (s.note.clone(), (s.scroll_line as usize, s.scroll_px), s.caret, s.launch == 1 || (s.launch == 0 && s.resumes(session::now()))));
        match saved {
            Some(s) => session = s,
            None => {
                session.sidebar = vault::top_level_rows(&vault).unwrap_or_default();
                // Quiet at rest: no menu bar icon, no global hotkey and no
                // related notes until asked for, the toolbar tucked away
                // until the pointer nears it.
                session.tray = 1;
                session.hotkey = 1;
                session.related = 1;
                session.toolbar_collapsed = true;
            }
        }
        let session = session;
        let mut engine = kolada_text::Engine::new(services.fonts.font_set(session.font));
        if let Some(draw) = services.fonts.color_glyphs() {
            engine.set_color_glyphs(draw);
        }
        engine.set_covered(services.fonts.covered());
        let mut model = Model {
            eng: Eng::new(engine),
            fonts: Fonts::new(scene::text_scale(session.text_size), session.font),
            session,
            vault,
            editor: Editor::new(""),
            path: None,
            format: note::Format::default(),
            dirty: false,
            opened_version: 0,
            named: String::new(),
            persist: false,
            data: None,
            past: Default::default(),
            surface: Surface::new((1, 1, 96)),
            pictures: RefCell::new(Pictures::new()),
            scale: 2.0,
            dark,
            system_dark: dark,
            told_dark: None,
            hover: None,
            search: None,
            spelling: Default::default(),
            find: None,
            index: Arc::new(Mutex::new(None)),
            index_started: false,
            files: Default::default(),
            outside: None,
            file_id: None,
            trash: None,
            undo_delete: Vec::new(),
            bin: None,
            services,
            scroll_line: 0,
            scroll: 0.0,
            at_end: false,
            reveal: false,
            hits: Vec::new(),
            access_kept: false,
            kept_scene: None,
            lines: Vec::new(),
            caret: None,
            column_width: 600.0,
            viewport: (1000.0, 700.0),
            settings: Default::default(),
            fixes: Default::default(),
            arrivals: Default::default(),
            resizing: None,
            ui: Default::default(),
            ime_target: None,
            popup: None,
            menus: Default::default(),
            fresh_dir: None,
            compact: false,
            bare: false,
            platform: scene::Platform::Mac,
            first_run: false,
            os_buttons: true,
            added: None,
            disk: None,
            visits: Default::default(),
            suggest: Default::default(),
            panels: Default::default(),
            social: Default::default(),
            agenda: Default::default(),
            sharing: Default::default(),
            margin: Default::default(),
            index_wake: None,
            arrival: None,
            writing: Default::default(),
            recent: Default::default(),
            formatting: Default::default(),
            linking: Default::default(),
            model_card: None,
            quick: Default::default(),
        };
        model.apply_theme();
        if model.session.font != 0 {
            model.apply_font();
        }
        if let Some((rel, scroll, caret, resume)) = resumed {
            let rel = if resume && model.vault.join(&rel).is_file() { rel } else { String::new() };
            if rel.is_empty() {
                // A fresh page, as the launch option says (or the note is gone).
                model.session.start_fresh();
                model.show(String::new(), String::new(), None);
            } else {
                model.open(&rel);
                let head = (caret as usize).min(model.editor.text().len());
                let head = (0..=head).rev().find(|&i| model.editor.text().is_char_boundary(i)).unwrap_or(0);
                model.editor.set_selection(Selection::caret(head));
                (model.scroll_line, model.scroll) = scroll;
            }
            return model;
        }
        match crate::launch::newest(&model.vault) {
            Some(path) => {
                if let Ok(rel) = path.strip_prefix(&model.vault) {
                    model.open(&rel.to_string_lossy());
                }
            }
            // No note yet: a fresh page to write the first on, as on Windows.
            None => {
                model.session.start_fresh();
                model.show(String::new(), String::new(), None);
            }
        }
        model
    }

    /// Shows `text` as the note `rel` (vault-relative, with its `.md`).
    fn show(&mut self, text: String, rel: String, path: Option<PathBuf>) {
        self.close_popup();
        self.fresh_dir = None;
        self.added = None;
        self.editor = Editor::new(text);
        self.editor.set_clean(!self.session.show_marks);
        self.opened_version = self.editor.version();
        self.editor.set_snippets(pictures::snippets(&self.services.calendar));
        // A page of its own: Cmd+Z is no longer a delete's.
        self.undo_delete.clear();
        self.named = self.editor.title();
        self.file_id = path.as_deref().and_then(outside::file_id);
        self.path = path;
        self.session.note = rel;
        self.scroll_line = 0;
        self.scroll = 0.0;
        self.dirty = false;
        self.reveal = false;
        self.noted();
        self.settings.read_only = false;
        self.fade_in_page();
        self.ui.scroll_pending = 0.0;
        self.note_opened();
        self.caret_moved(false);
    }

    /// Opens the note at `rel` (vault-relative path, with `.md`), saving the one open.
    pub fn open(&mut self, rel: &str) {
        self.remember_visit();
        self.save_flush();
        let path = self.vault.join(rel);
        if let Ok((text, format)) = note::load(&path) {
            let (text, added) = opened(&path, text);
            self.format = format;
            self.disk = stamp(&path);
            self.remember_left();
            self.show(text, rel.to_string(), Some(path));
            self.remember_opened();
            self.added = added;
            // Writing starts under the title: typing right away adds to the
            // note rather than renaming it.
            let at = body_start(self.editor.text(), self.editor.outline());
            self.editor.set_selection(Selection::caret(at));
        }
    }

    /// Writes the note if it changed. Returns whether it was written.
    pub fn save(&mut self) -> bool {
        self.save_popup(false);
        self.save_with(false)
    }

    /// Writes the note, and renames its file after its title: as the note is left or the app quits.
    pub fn save_flush(&mut self) -> bool {
        let wrote = self.save_with(true);
        self.persist_session();
        wrote
    }

    /// Keeps the session for the next launch.
    pub(super) fn persist_session(&mut self) {
        if !self.persist {
            return;
        }
        let Some(path) = session_path() else { return };
        self.session.scroll_line = self.scroll_line as u64;
        self.session.scroll_px = self.scroll;
        self.session.caret = self.editor.selection().head as u64;
        self.session.last_active = session::now();
        self.session.expanded = vault::expanded_folders(&self.session.sidebar).into_iter().collect();
        let _ = session::save(&path, &self.session);
    }

    fn save_with(&mut self, flush: bool) -> bool {
        // Leaving an edited note tidies it: the spaces and empty lines nobody meant.
        // (An autosave may have written the edits already: the note counts as edited by its version.)
        let edited = self.dirty || self.editor.version() != self.opened_version;
        if flush && edited && self.editor.tidy() {
            self.dirty = true;
        }
        // A task's @fri becomes @Fri 9 Oct 2026 once its line is left.
        if self.dirty || (flush && edited) {
            self.settle_dates(flush);
        }
        let wrote = self.write_note();
        self.written(wrote, flush);
        if wrote {
            self.refresh_due();
        }
        // The name follows the title once the caret is off the title line, or the note is left.
        let text = self.editor.text();
        let title_end = text.find('\n').unwrap_or(text.len());
        if self.path.is_some() && (flush || self.editor.selection().head > title_end) {
            self.rename_to_title();
        }
        wrote
    }

    fn write_note(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        // A fresh page becomes a file, named for its title, once it has words.
        if self.path.is_none() && self.session.note.is_empty() {
            if self.editor.text().trim().is_empty() {
                return false;
            }
            // The vault's folder is made with its first note.
            if std::fs::create_dir_all(&self.vault).is_err() {
                return false;
            }
            let dir = self.fresh_dir.as_ref().map(|d| self.vault.join(d)).filter(|d| d.is_dir()).unwrap_or_else(|| self.vault.clone());
            let path = note::unique_path(&dir, &note::file_stem_for(&self.editor.title()), None);
            let Some(rel) = vault::relative(&self.vault, &path) else { return false };
            if note::save(&path, self.editor.text(), self.format).is_err() {
                return false;
            }
            self.disk = stamp(&path);
            self.written_recently(&path);
            self.session.note = rel;
            self.remember_opened();
            self.path = Some(path);
            self.file_id = outside::file_id(self.path.as_deref().unwrap());
            self.dirty = false;
            self.refresh_tree();
            return true;
        }
        let Some(mut path) = self.path.clone() else { return false };
        let mut moved = false;
        // Another app changed the file since it was read or written: these
        // edits go beside theirs, never over them.
        if let (Some(was), Some(now)) = (self.disk, stamp(&path))
            && was != now
            && let Some(dir) = path.parent()
        {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            path = note::unique_path(dir, &format!("{stem} (conflict)"), None);
            moved = true;
            if let Some(rel) = vault::relative(&self.vault, &path) {
                self.session.note = rel;
                self.remember_opened();
                self.path = Some(path.clone());
                self.named = self.editor.title();
            }
        }
        self.keep_base();
        let text = file_text(self.editor.text(), &mut self.added).into_owned();
        match note::save(&path, &text, self.format) {
            Ok(()) => {
                self.dirty = false;
                self.disk = stamp(&path);
                self.written_recently(&path);
                self.file_id = outside::file_id(&path);
                if moved {
                    self.refresh_tree();
                }
                true
            }
            Err(_) => false,
        }
    }

    /// The file takes its title's name; links to it, here and in other notes, follow.
    fn rename_to_title(&mut self) {
        let title = self.editor.title();
        let Some(path) = self.path.clone() else { return };
        if title == self.named || title.is_empty() || self.dirty {
            return;
        }
        self.named = title.clone();
        let Some(dir) = path.parent() else { return };
        let wanted = note::unique_path(dir, &note::file_stem_for(&title), Some(&path));
        if wanted == path || std::fs::rename(&path, &wanted).is_err() {
            return;
        }
        let (Some(from), Some(to)) = (vault::relative(&self.vault, &path), vault::relative(&self.vault, &wanted)) else { return };
        // Its past goes with it.
        self.history_renamed(&from, &to);
        let stem = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let (old, new) = (stem(&path), stem(&wanted));
        self.session.renamed(&from, &to);
        self.session.note = to.clone();
        self.path = Some(wanted);
        // Links in this note change as edits, so undo takes them back.
        let own = links::renamed(self.editor.text(), &old, &new);
        if !own.is_empty() {
            self.editor.replace_ranges(&own);
            self.dirty = true;
        }
        self.refresh_tree();
        self.links_follow(old, new, to);
    }

    /// Links to the note `old` (by name) in other notes are rewritten to
    /// `new`, on a thread of their own; the note now at `to` is left alone.
    pub(super) fn links_follow(&self, old: String, new: String, to: String) {
        let vault = self.vault.clone();
        std::thread::spawn(move || {
            let needle = old.to_lowercase();
            for n in search::index(&vault) {
                if n.path == to || !n.text().contains("[[") || !n.text().to_lowercase().contains(&needle) {
                    continue;
                }
                let file = vault.join(&n.path);
                let Ok((text, format)) = note::load(&file) else { continue };
                let changes = links::renamed(&text, &old, &new);
                if !changes.is_empty() {
                    let _ = note::save(&file, &links::apply(&text, &changes), format);
                }
            }
        });
    }

    /// The file tree again, from disk, with its folders as they were open.
    fn refresh_tree(&mut self) {
        let open = vault::expanded_folders(&self.session.sidebar);
        if let Ok(rows) = vault::tree_rows(&self.vault, &open) {
            self.session.sidebar = rows;
        }
        self.clamp_tree();
    }

    /// The vault changed in a way no watcher reported (a shell with no
    /// `Watcher` of its own: iOS, when iCloud has downloaded notes): the
    /// file tree is read again. Whether the screen changes.
    pub fn refresh(&mut self) -> bool {
        let before = self.session.sidebar.len();
        self.refresh_tree();
        self.session.sidebar.len() != before
    }

    /// A new, blank page: it becomes a file when it has words.
    /// `text` as a fresh page, not yet a file: written only once edited
    /// (then named as any new note is), so a beam read here leaves nothing
    /// in the vault.
    pub(super) fn open_unwritten(&mut self, text: String) {
        self.save_flush();
        self.search = None;
        self.trash = None;
        self.format = note::Format::default();
        self.session.start_fresh();
        self.show(text, String::new(), None);
    }

    pub fn new_note(&mut self) -> bool {
        self.save_flush();
        self.search = None;
        self.trash = None;
        self.format = note::Format::default();
        self.session.start_fresh();
        self.show(String::new(), String::new(), None);
        true
    }

    // Search.

    pub fn open_search(&mut self) -> bool {
        self.trash = None;
        match &mut self.search {
            // Back in a search kept open: the caret at the query's end, to refine it.
            Some(s) if !s.focused => {
                s.focused = true;
                let end = s.query.text().len();
                s.query.set_selection(Selection::caret(end));
            }
            Some(s) => s.query.select_all(),
            None => self.search = Some(Search { query: Editor::field(""), hits: Vec::new(), selected: 0, first: 0, commands: None, shown: Vec::new(), runs: Vec::new(), fresh: false, focused: true }),
        }
        self.start_index();
        true
    }

    /// Reads every note's text on a thread of its own, once.
    pub fn start_index(&mut self) {
        if !self.index_started {
            self.index_started = true;
            let (slot, vault) = (self.index.clone(), self.vault.clone());
            let (corpus, wake) = (self.margin.corpus.clone(), self.index_wake.clone());
            let due = self.agenda.slot.clone();
            let files = self.files.clone();
            let services = self.services.clone();
            std::thread::spawn(move || {
                let all = search::index(&vault);
                // The tasks with dates, for the Today card.
                *due.lock().unwrap() = Some(agenda::due_in(&all, &*services.calendar));
                // What every note is about, for the margin.
                let built = kolada_core::related::Corpus::build(&all);
                *slot.lock().unwrap() = Some(all);
                *corpus.lock().unwrap() = Some(built);
                if let Some(wake) = &wake {
                    wake();
                }
                // Then the vault's PDFs and pictures, by their text.
                if files::add_to(&slot, &vault, &files, &services) && let Some(wake) = &wake {
                    wake();
                }
            });
        }
    }

    // What other apps do to the vault.

    /// Starts following the vault's files (after the first frame); `wake` is
    /// called from another thread when something changed, for `poll_outside`.
    /// `wake` is called from the index's thread once every note is read
    /// (the margin is found then).
    pub fn on_index(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.index_wake = Some(Arc::new(wake));
    }

    pub fn start_watching(&mut self, wake: impl Fn() + Send + 'static) {
        self.outside = Some(self.services.watcher.watch(&self.vault, Box::new(wake)));
        // Every note's name and text, for search and link suggestions; the
        // notes written last, for the Dock.
        self.start_index();
        self.find_recent();
    }

    /// Takes the news of changed files; whether the screen changes.
    pub fn poll_outside(&mut self) -> bool {
        let mut changes = Vec::new();
        if let Some(rx) = &self.outside {
            while let Ok(change) = rx.try_recv() {
                changes.push(change);
            }
        }
        self.sync_saw(&changes);
        !changes.is_empty() && self.apply_outside(&changes)
    }

    /// The vault changed under us: the file tree, the search's index and the
    /// open note (unless it has edits of its own) are read again.
    fn apply_outside(&mut self, changes: &[crate::services::Change]) -> bool {
        // The open note may have been renamed, moved or deleted.
        self.follow_open_note(changes);
        self.refresh_tree();
        *self.index.lock().unwrap() = None;
        self.index_started = false;
        if self.search.is_some() {
            self.start_index();
            if let Some(s) = &mut self.search {
                s.fresh = false;
            }
        }
        self.pictures.borrow_mut().forget_places();
        if let (Some(path), false, false) = (&self.path, self.dirty, self.has_marked()) {
            if let Ok((text, format)) = note::load(path) {
                self.disk = stamp(path);
                // Our own write, reported back: nothing to read. (Read as a
                // note just opened, a title shortened on its line, still under
                // its longer name, would get that name put over it.)
                if text == file_text(self.editor.text(), &mut self.added.clone()) {
                    return true;
                }
                let (text, added) = opened(path, text);
                if text != self.editor.text() {
                    let mut head = self.editor.selection().head.min(text.len());
                    while !text.is_char_boundary(head) {
                        head -= 1;
                    }
                    self.editor = Editor::new(text);
                    self.editor.set_clean(!self.session.show_marks);
                    self.opened_version = self.editor.version();
                    self.editor.set_snippets(pictures::snippets(&self.services.calendar));
                    self.editor.set_selection(Selection::caret(head));
                    self.format = format;
                    self.named = self.editor.title();
                    self.added = added;
                }
            }
        }
        true
    }

    pub fn close_search(&mut self) -> bool {
        self.search.take().is_some()
    }

    /// Whether the side menu's search takes the keys (open, and not kept
    /// open beside a note it opened).
    pub(super) fn search_has_keys(&self) -> bool {
        self.search.as_ref().is_some_and(|s| s.focused)
    }

    /// The results for the query, once the index is read.
    fn run_search(&mut self) {
        if self.run_palette() {
            return;
        }
        let Some(s) = &mut self.search else { return };
        if s.query.text().trim().is_empty() {
            s.hits.clear();
            s.selected = 0;
            s.fresh = true;
            return;
        }
        if let Some(all) = self.index.lock().unwrap().as_ref() {
            s.hits = search::search(all, s.query.text(), SEARCH_LIMIT);
            s.selected = s.selected.min(s.hits.len().saturating_sub(1));
            s.fresh = true;
        }
    }

    /// Opens the `i`th result; a note's keeps the search open on it, the
    /// keys the search's when it was opened with them (Down and Enter open
    /// the next), else the note's.
    fn open_hit(&mut self, i: usize, by_keys: bool) -> bool {
        if self.search.as_ref().is_some_and(|s| s.commands.is_some()) {
            return self.run_command(i);
        }
        let Some(path) = self.search.as_ref().and_then(|s| s.hits.get(i)).map(|h| h.path.clone()) else { return false };
        if !search::is_note(&path) {
            return self.open_file_hit(&path);
        }
        let kept = self.search.take();
        self.open(&path);
        self.search = kept;
        if let Some(s) = &mut self.search {
            s.selected = i;
            s.focused = by_keys;
        }
        true
    }

    /// Runs an editing command on the search's query, then searches again.
    fn query(&mut self, f: impl FnOnce(&mut Editor)) -> bool {
        if self.linking.open.is_some() {
            return self.link_query(f);
        }
        if let Some(s) = &mut self.search {
            f(&mut s.query);
            s.fresh = false;
            s.selected = 0;
            return true;
        }
        self.find_field(f)
    }

    /// A field (the search's or the find bar's) has the keys, not the page.
    fn in_field(&self) -> bool {
        self.linking.open.is_some() || self.search_has_keys() || self.find_focused()
    }

    /// Escape: closes the search, or the trash.
    pub fn escape(&mut self) -> bool {
        if self.code_key(crate::model::quick::QuickKey::Escape) {
            return true;
        }
        self.tooltip_away();
        if !self.in_field() && let Some(done) = self.suggest_key(suggest::SuggestKey::Away) {
            return done;
        }
        self.hide_link_card();
        // The link field, the Aa card and the selection bar go first.
        if self.close_link_field() || self.close_reading() || self.dismiss_selbar() {
            return true;
        }
        if self.close_share() {
            return true;
        }
        if self.social.platform.is_some() {
            return self.show_social(None);
        }
        if self.popup.is_some() {
            return self.close_popup();
        }
        self.close_options() | self.close_search() | self.trash.take().is_some() | self.close_find()
    }

    // Trash.

    /// Where the app notes what each deleted note was: with its data, or,
    /// for a model that keeps none (tests, dumps), hidden beside the vault,
    /// never in the app's real data.
    pub(super) fn trash_log(&self) -> PathBuf {
        match &self.data {
            Some(data) => data.join("trash.tsv"),
            None => self.vault.join(".kolada-trash.tsv"),
        }
    }

    pub fn open_trash(&mut self) -> bool {
        self.search = None;
        let items = trash::list(&self.vault, &self.trash_log());
        let now = session::now();
        let entries = items.iter().map(|t| TrashEntry { name: t.name(), when: when(t.at, now) }).collect();
        self.trash = Some((items, entries));
        true
    }

    /// Every note in the vault, the one written last first, for a shell's
    /// own list of them (the iPhone's All notes); whether each is a
    /// favorite. Names and times only: nothing is read.
    pub fn every_note(&self) -> Vec<(vault::Listed, bool)> {
        vault::every_note(&self.vault).into_iter().map(|n| {
            let favorite = self.session.favorites.iter().any(|f| *f == n.path);
            (n, favorite)
        }).collect()
    }

    /// The open note goes to the vault's trash and leaves a fresh page.
    pub fn trash_open_note(&mut self) -> bool {
        self.save();
        let rel = self.session.note.clone();
        if rel.is_empty() || self.path.is_none() {
            return false;
        }
        let Ok(item) = trash::trash(&self.vault, &rel, &self.trash_log(), session::now()) else { return false };
        // Gone from Recent at once (brought back, it opens and is first again).
        self.session.recent.retain(|n| *n != rel);
        self.dirty = false;
        self.session.start_fresh();
        self.show(String::new(), String::new(), None);
        self.undo_delete.push((item, self.editor.version()));
        self.refresh_tree();
        if self.trash.is_some() {
            self.open_trash();
        }
        true
    }

    /// Brings back the `i`th note in the trash, and opens it.
    fn restore(&mut self, i: usize) -> bool {
        let Some(item) = self.trash.as_ref().and_then(|(items, _)| items.get(i)).cloned() else { return false };
        if !self.bring_back(&item) {
            return false;
        }
        self.trash = None;
        true
    }

    // The theme.

    /// System, then light, then dark.
    fn cycle_theme(&mut self) -> bool {
        self.session.theme = match self.session.theme {
            0 => 1,
            1 => 2,
            _ => 0,
        };
        self.apply_theme();
        true
    }

    fn apply_theme(&mut self) {
        self.dark = match self.session.theme {
            1 => false,
            2 => true,
            _ => self.system_dark,
        };
    }

    /// The screen's darkness if the window has not been told it yet.
    pub fn dark_to_tell(&mut self) -> Option<bool> {
        (self.told_dark != Some(self.dark)).then(|| {
            self.told_dark = Some(self.dark);
            self.dark
        })
    }

    #[cfg(test)]
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    #[cfg(test)]
    pub fn text(&self) -> &str {
        self.editor.text()
    }

    #[cfg(test)]
    pub fn selection(&self) -> Selection {
        self.editor.selection()
    }

    /// The caret's box in the last frame, DIPs.
    pub fn caret_rect(&self) -> Option<Rect> {
        if self.popup.is_some() {
            return self.popup_caret();
        }
        self.caret
    }

    /// The system's light or dark setting, which the theme follows unless it says otherwise.
    pub fn set_dark(&mut self, system: bool) {
        self.system_dark = system;
        self.apply_theme();
    }

    /// The note as the editor asks about it: where lines wrap, where the
    /// caret goes. The same layout the screen draws from.
    fn note_text(&self) -> NoteText<'_> {
        let (text, lines) = (self.editor.text(), self.editor.outline());
        NoteText {
            eng: &self.eng,
            fonts: &self.fonts,
            text,
            lines,
            width: self.column_width,
            title: title_line(text, lines),
            caret: Some(self.editor.selection().head),
            clean: !self.session.show_marks,
            pictures: self.note_pictures(),
            tables: Default::default(),
        }
    }

    /// The pictures the note's lines embed, for the editor's layout.
    fn note_pictures(&self) -> Option<kolada_ui::note::Pictures<'_>> {
        let dir = self.path.as_deref().and_then(Path::parent).filter(|d| d.starts_with(&self.vault)).unwrap_or(&self.vault).to_path_buf();
        let resize = self.resizing.as_ref().map(|r| (r.line, r.width));
        Some(kolada_ui::note::Pictures { images: &self.pictures, vault: &self.vault, dir, scale: self.scale.max(1.0), resize })
    }

    /// Starts the pictures' worker (after the first frame); `wake` is called
    /// from its thread whenever something is ready for `poll_pictures`.
    pub fn start_pictures(&mut self, wake: impl Fn() + Send + 'static) {
        let images = self.services.images.clone();
        self.pictures.borrow_mut().start(move || images.decoder(), wake);
    }

    /// Takes the pictures that arrived; whether the screen changes.
    pub fn poll_pictures(&mut self) -> bool {
        self.pictures.borrow_mut().poll()
    }

    /// A picture is still fading in.
    pub fn pictures_fading(&self) -> bool {
        self.pictures.borrow().fading()
    }

    /// Checks the spelling of the next few lines not yet checked, nearest the
    /// top first. Returns whether marks changed, and whether lines remain.
    pub fn spell_step(&mut self) -> (bool, bool) {
        let spell = self.services.spell.clone();
        self.spell_step_with(move |units| spelled_as_ours(units, spell.check(units)))
    }

    /// `spell_step` with `check` as the checker: a line's prose (UTF-16) to
    /// the UTF-16 ranges of its misspelled words.
    fn spell_step_with(&mut self, check: impl Fn(&[u16]) -> Vec<(usize, usize)>) -> (bool, bool) {
        if self.session.spell != 0 {
            return (false, false);
        }
        const BUDGET: usize = 12;
        let text = self.editor.text();
        let lines = self.editor.outline();
        let caret = self.editor.selection().head;
        let mut checked = 0;
        let mut changed = false;
        let mut found: Vec<(u64, Vec<(u32, u32)>)> = Vec::new();
        let mut more = false;
        for l in lines {
            if l.end > text.len() {
                break;
            }
            let key = kolada_ui::spell::line_key(&text[l.start..l.end]);
            if self.spelling.contains_key(&key) {
                continue;
            }
            if checked == BUDGET {
                more = true;
                break;
            }
            checked += 1;
            let marks = match kolada_ui::spell::prose(text, l) {
                Some((units, offsets)) => {
                    let mut marks: Vec<(u32, u32)> =
                        check(&units).into_iter().filter(|&(a, b)| !self.is_ignored(&units[a..b])).map(|(a, b)| (offsets[a], offsets[b.min(offsets.len() - 1)])).collect();
                    // The word being typed is not marked.
                    if (l.start..=l.end).contains(&caret) {
                        let at = (caret - l.start) as u32;
                        marks.retain(|&(a, b)| at < a || at > b);
                    }
                    marks
                }
                None => Vec::new(),
            };
            changed |= !marks.is_empty();
            found.push((key, marks));
        }
        if self.spelling.len() > 4000 {
            let keys: std::collections::HashSet<u64> = lines.iter().filter(|l| l.end <= text.len()).map(|l| kolada_ui::spell::line_key(&text[l.start..l.end])).collect();
            self.spelling.retain(|k, _| keys.contains(k));
        }
        self.spelling.extend(found);
        (changed, more)
    }

    /// Builds the scene for `w` x `h` pixels at `scale`.
    fn build(&mut self, w: u32, h: u32, scale: f32) -> scene::Scene {
        self.follow_selection();
        let (width, height) = (w as f32 / scale, h as f32 / scale);
        let device_rows = self.device_rows();
        let pair_left = self.pair_left();
        let sync_facts = self.sync_facts();
        let s = &self.session;
        // The real folder: the scene finds pictures by it (the settings name it as each OS does).
        let vault_shown = self.session.vault.clone();
        let sel = self.editor.selection();
        let active = s.note.strip_suffix(".md").unwrap_or(&s.note);
        let search = self.search.as_ref().map(|q| SearchView {
            query: q.query.text(),
            caret: q.query.selection().head,
            hits: &q.hits,
            selected: q.selected,
            first: q.first,
            commands: q.commands.is_some().then_some(q.shown.as_slice()),
            focused: q.focused,
        });
        // Scrubbing through the note's past: the version shown, no caret.
        let (text, lines, selection) = match self.scrub_page() {
            Some((text, lines)) => (text, lines, (usize::MAX, usize::MAX)),
            None => (self.editor.text(), self.editor.outline(), (sel.anchor, sel.head)),
        };
        let date = self.date_line();
        let reading = self.scrub_page().is_some() || self.settings.read_only;
        let tools = if reading { Vec::new() } else { kolada_ui::edit::active_tools(text, lines, selection.1) };
        // The open note's row in the tree reads as its title does, before the file follows.
        let title = self.editor.title();
        let live_name = (!s.note.is_empty() && title != self.named && !title.is_empty()).then(|| note::file_stem_for(&title));
        let scrollbar = self.scroll_view();
        // The map shows with the bar: found only then, not at every key.
        let mapped = scrollbar.is_some_and(|b| b.alpha > 0.0) || matches!(self.hover, Some(Target::Heading(_)));
        let kept = if mapped { self.headings_for_map().clone() } else { Vec::new() };
        let here = kept.iter().rposition(|h| h.2 <= self.scroll_line);
        let landmarks: Vec<scene::Landmark> = kept.iter().enumerate().map(|(i, h)| scene::Landmark { at: h.3, text: &h.0, level: h.1, here: Some(i) == here }).collect();
        let marked = self.composition().filter(|_| !reading);
        let (chips, dragged, drop_at, peek) = self.margin_view();
        let (selection_bar, reading, tooltip) = self.formatting_view();
        let (menu, menu_row, menu_anchor) = self.menu_view();
        let input = Input {
            vault_name: self.vault.file_name().and_then(|n| n.to_str()).unwrap_or(""),
            active_note: active,
            text,
            sidebar: &s.sidebar,
            recent: &s.recent,
            favorites: &s.favorites,
            folded: s.folded,
            scroll_line: self.scroll_line,
            scroll_px: self.scroll,
            selection,
            lines,
            options: self.option_values(),
            vault_path: &vault_shown,
            page_fade: self.page_fade(),
            active_tools: &tools,
            live_name: live_name.as_deref(),
            tree_top: self.ui.tree_top,
            scrollbar,
            dragging: if self.holding_edge() { Some(Target::SidebarEdge) } else { self.ui.scroll_drag.map(|d| d.bar) },
            landmarks: &landmarks,
            marked,
            whisper: self.whisper_view(),
            opened_in: self.ui.opened_in,
            keystroke: self.keystroke_median(),
            popup: self.popup_view(),
            social_drawer: s.social == 2,
            social: self.social_view(),
            compact: self.compact || self.session.minimal || self.bare,
            bare: self.bare,
            chips: &chips,
            dragged,
            drop_at,
            peek,
            carried: self.carried(),
            arriving: self.arrival_view(),
            wash: self.wash_view(),
            page_gist: if date.is_some() { self.gist_view() } else { None },
            suggestions: self.suggestions_view(),
            images: Some(&self.pictures),
            scale,
            sidebar_dock: self.panels.dock[0],
            sidebar_show: self.panels.show[0],
            toolbar_dock: self.panels.dock[1],
            toolbar_show: self.panels.show[1],
            sidebar_wave: &self.panels.wave[0],
            toolbar_wave: &self.panels.wave[1],
            sidebar_width: s.sidebar_width as f32,
            hover: self.hover,
            search,
            spelling: (self.session.spell == 0 && self.scrub_page().is_none()).then_some(&self.spelling),
            page_date: date.as_deref(),
            scrub: self.scrub_view(),
            find: self.find.as_ref().map(Find::view),
            matches: self.find.as_ref().map_or(&[][..], |f| &f.matches),
            ticks: self.find.as_ref().map_or(&[][..], |f| &f.ticks),
            row_date: self.ui.row_date.as_ref().map(|(i, d)| (*i, d.as_str())),
            trash: self.trash.as_ref().map(|(_, entries)| entries.as_slice()),
            no_margin: s.related == 1 || s.minimal || self.compact,
            native_caption: self.os_buttons,
            maximized: self.ui.maximized,
            platform: self.platform,
            today: Some(agenda::today(&*self.services.calendar)),
            due_now: self.agenda.now,
            agenda: self.agenda_view(),
            options_open: self.settings.open,
            options_scroll: self.settings.scroll,
            highlights_open: self.settings.read_only,
            picture_resize: self.resizing.as_ref().map(|r| (r.line, r.width)),
            show_marks: s.show_marks,
            share: self.share_view(),
            link_field: self.link_field_view(),
            link_card: self.link_card_view(),
            model_card: self.model_view(),
            updater: self.settings.updater && !self.compact,
            update: if self.compact { None } else { self.settings.update.as_deref() },
            update_check: self.settings.update_check.view(),
            quick: self.beam_view(),
            devices: self.devices_view(&device_rows, &pair_left, &sync_facts),
            selection_bar,
            reading,
            tooltip,
            menu,
            menu_row,
            menu_anchor,
            ..Input::default()
        };
        scene::build(&self.eng, &self.fonts, &input, width, height).expect("a scene builds")
    }

    /// Draws a frame of `w` x `h` pixels at `scale` pixels per DIP.
    pub fn frame(&mut self, w: u32, h: u32, scale: f32) {
        self.viewport = (w as f32 / scale, h as f32 / scale);
        self.scale = scale;
        self.panels_for_width(self.viewport.0);
        if self.search.as_ref().is_some_and(|s| !s.fresh) {
            self.run_search();
        }
        // A line the caret left may take a list, or be asked about again.
        self.assist_caret();
        // More passes while the caret is scrolled into view.
        self.fix_scroll_line();
        let mut built = self.build(w, h, scale);
        for _ in 0..3 {
            self.hits = std::mem::take(&mut built.hits);
            self.lines = std::mem::take(&mut built.lines);
            self.caret = built.caret;
            self.column_width = built.column.width();
            self.ui.column = built.column;
            self.ui.tree_rows = built.tree_rows;
            self.ui.tree_track = built.tree_track;
            self.ui.result_rows = built.result_rows;
            self.panels.pills = built.pills;
            if !std::mem::take(&mut self.reveal) {
                break;
            }
            let (top, bottom) = (built.column.top + 8.0, self.viewport.1 - 56.0);
            match self.caret {
                Some(c) => {
                    let (above, below) = (c.top - top, c.bottom - bottom);
                    let dy = if above < 0.0 { above } else if below > 0.0 { below } else { 0.0 };
                    if dy == 0.0 {
                        break;
                    }
                    self.shift_scroll(dy);
                }
                // Not drawn at all: the view goes to its paragraph, and is looked at again.
                None => {
                    if !self.scroll_to_caret(built.column.top, bottom) {
                        break;
                    }
                    self.reveal = true;
                }
            }
            built = self.build(w, h, scale);
        }
        // The popup keeps its caret in its card; the frame is drawn again if it scrolled.
        if self.popup_drawn(built.popup.take()) {
            built = self.build(w, h, scale);
            self.hits = std::mem::take(&mut built.hits);
            self.lines = std::mem::take(&mut built.lines);
            self.caret = built.caret;
            let drawn = built.popup.take();
            if let Some(p) = &mut self.popup {
                p.drawn = drawn;
            }
        }
        let height = self.viewport.1;
        self.at_end = self.lines.last().is_some_and(|l| l.para.end >= self.editor.text().len() && l.y + l.height < height - 24.0);
        self.settings.scroll_max = built.options_scroll_max;
        self.social.scroll_max = built.social_scroll_max;
        // The past shown and the page of highlights have no caret.
        if self.settings.read_only {
            built.caret = None;
        }
        self.glide_to(built.caret);
        // While a note is open over the page, its caret is the one typing.
        if self.popup.is_some() {
            built.caret = None;
        }
        built.place_caret(self.ui.caret_offset);
        let list = display::record(&self.eng, &built, theme::for_mode(self.dark), scale).expect("a frame is recorded");
        self.surface.draw((w, h, (scale * 96.0).round() as u32), &list);
        if self.access_kept {
            self.kept_scene = Some(built);
        }
    }

    /// The last frame's pixels, 0x00RRGGBB in top-down rows.
    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    // Editing. Each says whether the screen changes.

    /// Runs an editing command, then shows it.
    fn edit(&mut self, f: impl FnOnce(&mut Editor)) -> bool {
        if self.popup.is_some() {
            return self.popup_edit(f);
        }
        // The past shown, and the page of highlights, are looked at, not typed into.
        if self.scrubbing() || self.settings.read_only {
            return false;
        }
        let before = self.editor.version();
        f(&mut self.editor);
        let changed = self.editor.version() != before;
        self.dirty |= changed;
        // Written in: the Beam button offers this note, and fades while typing.
        self.note_used(changed);
        if changed {
            self.sync_typing();
        }
        // Written in: a search kept open beside it has done its work.
        if changed && self.search.as_ref().is_some_and(|s| !s.focused) {
            self.search = None;
        }
        self.reveal = true;
        // The text changes at once, so the caret moves with it: it glides
        // only through text that stays put (arrows, clicks).
        self.caret_moved(!changed);
        if changed {
            self.ime_target = None;
            self.assist_edited();
        }
        true
    }

    /// Text typed or pasted, replacing the selection.
    pub fn insert(&mut self, s: &str) -> bool {
        // The code typed in Settings > Devices.
        if self.code_typed(s) {
            return true;
        }
        self.hide_link_card();
        // Typing puts a drawn menu away, and types.
        self.close_menu();
        if let Some(done) = self.agenda_text(s) {
            return done;
        }
        if self.in_field() {
            return self.query(|q| q.insert(&s.replace('\n', " ")));
        }
        if self.popup.is_none() {
            self.leave_picture_for_text();
        }
        self.edit(|e| e.insert(s))
    }

    /// Return: a new line, continuing a list; `soft` (Shift) just breaks the line.
    pub fn newline(&mut self, soft: bool) -> bool {
        if self.code_key(crate::model::quick::QuickKey::Enter) {
            return true;
        }
        if self.linking.open.is_some() {
            return self.link_commit();
        }
        if !soft && !self.in_field() && let Some(done) = self.suggest_key(suggest::SuggestKey::Pick) {
            return done;
        }
        if let Some(s) = self.search.as_ref().filter(|s| s.focused) {
            let i = s.selected;
            return self.open_hit(i, true);
        }
        if self.find_focused() {
            return self.find_return(soft);
        }
        if self.popup.is_none() && let Some(done) = self.line_after_picture() {
            return done;
        }
        self.edit(|e| if soft { e.insert("\n") } else { e.newline() })
    }

    /// Tab nests a list item, elsewhere it is a tab; Shift+Tab outdents.
    pub fn tab(&mut self, outdent: bool) -> bool {
        if self.linking.open.is_some() {
            return self.link_tab();
        }
        if !outdent && !self.in_field() && let Some(done) = self.suggest_key(suggest::SuggestKey::Pick) {
            return done;
        }
        if self.find_focused() {
            return self.find_tab();
        }
        if self.search_has_keys() {
            return false;
        }
        self.edit(|e| {
            if !e.indent(outdent) && !outdent {
                e.insert("\t");
            }
        })
    }

    pub fn backspace(&mut self, word: bool) -> bool {
        if self.code_key(crate::model::quick::QuickKey::Backspace) {
            return true;
        }
        if self.in_field() {
            return self.query(|q| q.backspace(word));
        }
        if self.popup.is_none() && let Some(done) = self.delete_picture() {
            return done;
        }
        self.edit(|e| e.backspace(word))
    }

    /// Deletes to the start of the line (`back`) or its end: what the
    /// caret would move over, as one step.
    pub fn delete_to(&mut self, back: bool) -> bool {
        if self.in_field() {
            return self.query(|q| {
                let plan = q.plan(if back { Motion::LineStart } else { Motion::LineEnd }, true, &OneLine(q.text()));
                q.apply(plan);
                q.backspace(false);
            });
        }
        if self.popup.is_some() || self.scrubbing() || self.settings.read_only {
            return false;
        }
        let plan = self.editor.plan(if back { Motion::LineStart } else { Motion::LineEnd }, true, &self.note_text());
        self.editor.apply(plan);
        if self.editor.selection().is_caret() {
            // At the line's end already: the line break goes, as on the Mac.
            return if back { self.backspace(false) } else { self.delete(false) };
        }
        self.edit(|e| e.backspace(false))
    }

    pub fn delete(&mut self, word: bool) -> bool {
        if self.in_field() {
            return self.query(|q| q.delete(word));
        }
        if self.popup.is_none() && let Some(done) = self.delete_picture() {
            return done;
        }
        self.edit(|e| e.delete(word))
    }

    pub fn undo(&mut self) -> bool {
        if self.in_field() {
            return false;
        }
        if self.popup.is_none() && self.undo_last_delete() {
            return true;
        }
        let done = self.edit(|e| {
            e.undo();
        });
        if self.popup.is_none() {
            self.assist_undone();
        }
        done
    }

    pub fn redo(&mut self) -> bool {
        if self.in_field() {
            return false;
        }
        self.edit(|e| {
            e.redo();
        })
    }

    pub fn tool(&mut self, tool: Tool) -> bool {
        // On the page, Link asks what to link to (`linkfield.rs`); in a
        // note open over it, it wraps the words in [[ ]].
        if tool == Tool::Link && self.popup.is_none() {
            return self.open_link_field();
        }
        if self.linking.open.is_some() {
            return false;
        }
        self.edit(|e| apply_tool(e, tool))
    }

    /// The line made a heading of a level, or body text (Cmd+1 to Cmd+3, Cmd+0).
    pub fn block(&mut self, kind: kolada_editor::BlockKind) -> bool {
        self.edit(|e| e.toggle_block(kind))
    }

    /// Cmd+Return: the task at the caret checked off, or the line made one.
    pub fn toggle_task_here(&mut self) -> bool {
        self.edit(|e| {
            let at = e.selection().head;
            if !e.toggle_task(at) {
                e.toggle_block(kolada_editor::BlockKind::Task);
            }
        })
    }

    /// The open note, vault-relative, if it has a file.
    pub fn note_rel(&self) -> Option<String> {
        (self.path.is_some() && !self.session.note.is_empty()).then(|| self.session.note.clone())
    }

    // Input methods. AppKit counts text in UTF-16 units from the start of the
    // note; the editor counts bytes.

    /// The selection, as a UTF-16 location and length.
    pub fn selection_utf16(&self) -> (usize, usize) {
        let e = self.active();
        let (text, sel) = (e.text(), e.selection());
        let start = utf16_at(text, sel.start());
        (start, utf16_at(text, sel.end()) - start)
    }

    /// The text being composed, as a UTF-16 location and length.
    pub fn marked_utf16(&self) -> Option<(usize, usize)> {
        let (a, b) = self.active().marked()?;
        let text = self.active().text();
        Some((utf16_at(text, a), utf16_at(text, b) - utf16_at(text, a)))
    }

    /// What the page underlines: the text being composed, and the part of
    /// it being converted.
    pub(super) fn composition(&self) -> Option<scene::Composed> {
        let (a, b) = self.editor.marked()?;
        Some(((a, b), self.ime_target.map(|(x, y)| (a + x, (a + y).min(b)))))
    }

    pub fn has_marked(&self) -> bool {
        self.active().marked().is_some()
    }

    /// The editor with the keys: the note open over the page, or the page's.
    fn active(&self) -> &Editor {
        self.popup_editor().unwrap_or(&self.editor)
    }

    fn active_mut(&mut self) -> &mut Editor {
        if self.popup.is_some() {
            return self.popup_editor_mut().unwrap();
        }
        &mut self.editor
    }

    /// Selects the UTF-16 range an input method says it is replacing.
    pub fn select_utf16(&mut self, location: usize, length: usize) {
        let text = self.active().text();
        let (a, b) = (byte_at(text, location), byte_at(text, location + length));
        self.active_mut().set_selection(Selection { anchor: a, head: b });
    }

    /// Text being composed, shown where the caret is, the caret `cursor`
    /// UTF-16 units into it.
    #[cfg(test)]
    pub fn set_marked(&mut self, text: &str, cursor: usize) -> bool {
        self.set_marked_converting(text, cursor, 0)
    }

    /// `set_marked`, with the `converting` UTF-16 units after `cursor` the
    /// part the input method is converting (underlined in the accent).
    pub fn set_marked_converting(&mut self, text: &str, cursor: usize, converting: usize) -> bool {
        let (end, cursor) = (byte_at(text, cursor + converting), byte_at(text, cursor));
        if self.in_field() {
            return self.query(|q| q.set_marked(text, cursor));
        }
        let changed = self.edit(|e| e.set_marked(text, cursor));
        self.ime_target = (end > cursor).then_some((cursor, end));
        changed
    }

    /// The composition is done: `text` is what the input method settled on.
    pub fn commit_marked(&mut self, text: &str) -> bool {
        self.close_menu();
        self.ime_target = None;
        if self.in_field() {
            return self.query(|q| q.commit_marked(text));
        }
        self.edit(|e| e.commit_marked(text))
    }

    /// The composition was taken back (an input method that clears what it
    /// showed before it commits, as winit's do, or Escape in one): what it
    /// had replaced comes back, and nothing is recorded.
    pub fn cancel_marked(&mut self) -> bool {
        self.ime_target = None;
        if self.in_field() {
            return self.query(|q| q.cancel_marked());
        }
        if self.active().marked().is_none() {
            return false;
        }
        self.edit(|e| e.cancel_marked())
    }

    /// The composition ends as it stands.
    pub fn unmark(&mut self) -> bool {
        match self.active().marked() {
            Some((a, b)) => {
                let text = self.active().text()[a..b].to_string();
                self.commit_marked(&text)
            }
            None => false,
        }
    }

    /// The text offset, in UTF-16 units, nearest the point (x, y) DIPs.
    pub fn index_at(&self, x: f32, y: f32) -> usize {
        utf16_at(self.editor.text(), self.offset_at(x, y))
    }

    /// The text between two UTF-16 positions (for an input method that asks).
    pub fn substring_utf16(&self, location: usize, length: usize) -> String {
        let text = self.active().text();
        text[byte_at(text, location)..byte_at(text, location + length)].to_string()
    }

    pub fn select_all(&mut self) -> bool {
        if self.linking.open.is_some() {
            return self.link_query(|q| q.select_all());
        }
        if let Some(s) = self.search.as_mut().filter(|s| s.focused) {
            s.query.select_all();
            return true;
        }
        if self.find_focused() {
            return self.find_field(|e| e.select_all());
        }
        if self.popup.is_some() {
            return self.popup_edit(|e| e.select_all());
        }
        self.editor.select_all();
        self.reveal = true;
        true
    }

    /// The selected text, for copy and cut.
    pub fn selected_text(&self) -> String {
        self.active().selected_text().to_string()
    }

    /// Moves the caret (extending the selection with `extend`) by `motion`.
    pub fn go(&mut self, motion: Motion, extend: bool) -> bool {
        self.hide_link_card();
        if self.linking.open.is_some() {
            return self.link_go(motion, extend);
        }
        if !extend && !self.in_field() {
            let key = match motion {
                Motion::Up => Some(suggest::SuggestKey::Up),
                Motion::Down => Some(suggest::SuggestKey::Down),
                _ => None,
            };
            if let Some(done) = key.and_then(|k| self.suggest_key(k)) {
                return done;
            }
        }
        if let Some(s) = self.search.as_mut().filter(|s| s.focused) {
            let n = if s.commands.is_some() { s.shown.len() } else { s.hits.len() };
            let fit = self.ui.result_rows.max(1);
            match motion {
                Motion::Up => {
                    s.selected = s.selected.saturating_sub(1);
                    s.first = s.first.min(s.selected);
                }
                Motion::Down => {
                    s.selected = (s.selected + 1).min(n.saturating_sub(1));
                    // Scrolled along once it passes the last row that fits.
                    if s.selected >= s.first + fit {
                        s.first = s.selected + 1 - fit;
                    }
                }
                _ => {
                    let plan = s.query.plan(motion, extend, &OneLine(s.query.text()));
                    s.query.apply(plan);
                }
            }
            return true;
        }
        if self.find_focused() {
            return match motion {
                Motion::Up => self.find_step(true),
                Motion::Down => self.find_step(false),
                _ => self.find_field(|e| {
                    let plan = e.plan(motion, extend, &OneLine(e.text()));
                    e.apply(plan);
                }),
            };
        }
        if self.popup.is_some() {
            return match motion {
                Motion::Up => self.popup_go(None, -1, extend),
                Motion::Down => self.popup_go(None, 1, extend),
                m => self.popup_go(Some(m), 0, extend),
            };
        }
        if !extend {
            let back = matches!(motion, Motion::Left | Motion::Up | Motion::WordLeft);
            let ahead = matches!(motion, Motion::Right | Motion::Down | Motion::WordRight);
            if (back || ahead) && self.step_off_picture(back) {
                return true;
            }
        }
        let plan = self.editor.plan(motion, extend, &self.note_text());
        self.editor.apply(plan);
        if !extend {
            self.snap_to_picture();
        }
        self.reveal = true;
        self.caret_moved(true);
        true
    }

    /// Moves the caret up or down by `lines` lines of the window (Page Up/Down).
    pub fn page(&mut self, down: bool, extend: bool) -> bool {
        if self.popup.is_some() {
            let n = self.popup_page();
            return self.popup_go(None, if down { n } else { -n }, extend);
        }
        let lines = ((self.viewport.1 - 120.0) / 24.0).max(1.0) as isize;
        let plan = self.editor.plan_lines(if down { lines } else { -lines }, extend, &self.note_text());
        self.editor.apply(plan);
        self.reveal = true;
        self.caret_moved(true);
        true
    }

    /// The text offset under (x, y) DIPs: the paragraph there, or the last.
    fn offset_at(&self, x: f32, y: f32) -> usize {
        let nt = self.note_text();
        match self.lines.iter().find(|l| y < l.y + l.height).or(self.lines.last()) {
            Some(l) => {
                let y = (y - l.y).clamp(0.0, (l.height - 0.5).max(0.0));
                nt.offset_at(&l.para, x - l.x, y).unwrap_or(l.para.start)
            }
            None => self.editor.text().len(),
        }
    }

    // The pointer.

    /// The pointer moved to (x, y) DIPs; whether the screen changes.
    pub fn pointer_moved(&mut self, x: f32, y: f32) -> bool {
        let before = self.ui.pointer.replace((x, y));
        let over = scene::hit(&self.hits, x, y);
        let past = self.scrub_hover(x, y);
        // Near the right edge, the page's scroll bar comes out.
        let edge = |p: Option<(f32, f32)>| p.is_some_and(|(x, _)| x > self.viewport.0 - 48.0);
        let bar = edge(before) != edge(Some((x, y))) && self.scroll_view().is_some();
        let moved = std::mem::replace(&mut self.hover, over) != over;
        if moved {
            self.tooltip_hover(over);
        }
        let card = self.link_hover_moved(over == Some(Target::Note), (x, y));
        let changed = moved || past || bar || card;
        // A note's row says when it was last written.
        let row = match over {
            Some(Target::Row(i) | Target::RowMenu(i)) => Some(i),
            _ => None,
        };
        let chip = match over {
            Some(Target::Chip(i)) => Some(i),
            _ => None,
        };
        if chip != self.margin_hovered() {
            self.hover_chip(chip);
        }
        if row != self.ui.row_date.as_ref().map(|r| r.0) {
            self.ui.row_date = row.and_then(|i| {
                self.session.sidebar.get(i).filter(|r| r.kind == RowKind::Note)?;
                let rel = vault::row_paths(&self.session.sidebar).get(i)?.clone();
                let modified = std::fs::metadata(self.vault.join(format!("{rel}.md"))).ok()?.modified().ok()?;
                Some((i, crate::when::ago(&*self.services.calendar, modified)))
            });
        }
        self.panels_pointer(before, (x, y));
        changed
    }

    pub fn pointer_left(&mut self) -> bool {
        self.hover_chip(None);
        let last = self.ui.pointer.take();
        let changed = self.hover.take().is_some() | self.tooltip_away() | self.hide_link_card();
        self.panels_left(last);
        changed
    }

    /// Whether a press at (x, y) DIPs moves the window: the empty strip
    /// along the top where nothing of ours is, as a title bar would (the
    /// window has none of its own: the note runs under the OS's buttons).
    /// The shell drags the window there instead of calling `pointer_down`.
    pub fn drags_window(&self, x: f32, y: f32) -> bool {
        scene::hit(&self.hits, x, y) == Some(Target::Caption)
    }

    /// A click at (x, y) DIPs, the `clicks`th in a row (1 single, 2 double,
    /// 3 triple), with Shift held or not; whether the screen changes.
    pub fn pointer_down(&mut self, x: f32, y: f32, clicks: u32, shift: bool) -> bool {
        self.ui.pressed = scene::hit(&self.hits, x, y);
        // A drawn menu has the press first (`drawn.rs`).
        if let Some(done) = self.menu_down(self.ui.pressed) {
            return done;
        }
        let pressed = self.formatting_press(scene::hit(&self.hits, x, y));
        self.pointer_press(x, y, clicks, shift) | pressed
    }

    fn pointer_press(&mut self, x: f32, y: f32, clicks: u32, shift: bool) -> bool {
        // The writing model card asks over the dimmed page.
        if let Some(done) = self.model_card_click(scene::hit(&self.hits, x, y)) {
            return done;
        }
        // Beam's button and its menu (a click elsewhere closes the menu).
        if let Some(done) = self.beam_click(scene::hit(&self.hits, x, y)) {
            return done;
        }
        // Settings > Devices: pairing, as the popup does it (nothing there
        // asks the shell for anything).
        use kolada_ui::quick::Hit;
        if let Some(Target::Quick(h @ (Hit::ShowCode | Hit::EnterCode | Hit::CodeField | Hit::Pair | Hit::Cancel | Hit::Allow | Hit::Deny | Hit::Remove(_) | Hit::Rename | Hit::NameField | Hit::NameSave | Hit::NameCancel))) = scene::hit(&self.hits, x, y) {
            let _ = self.quick_press(h);
            return true;
        }
        let card = self.hide_link_card();
        // The link field: its own clicks; elsewhere it goes, and the press
        // does what it does.
        if let Some(done) = self.link_field_click(scene::hit(&self.hits, x, y)) {
            return done;
        }
        // The share card: its own clicks, and beside it, it closes.
        if let Some(done) = self.share_click(scene::hit(&self.hits, x, y)) {
            return done;
        }
        // The Today card, and the teal dot that opens it.
        if let Some(done) = self.agenda_click(scene::hit(&self.hits, x, y)) {
            return done;
        }
        if let Some(done) = self.popup_down(x, y, clicks, shift) {
            return done;
        }
        if let Some(done) = self.scrub_down(x, y) {
            return done;
        }
        // The settings panel, the highlights button, a link on the highlights page.
        if let Some(handled) = self.settings_click(x, y) {
            return handled;
        }
        if let Some(done) = self.social_click(scene::hit(&self.hits, x, y), x, y) {
            return done;
        }
        if let Some(done) = self.margin_down(scene::hit(&self.hits, x, y), x, y) {
            return done;
        }
        match scene::hit(&self.hits, x, y) {
            Some(bar @ (Target::Scrollbar | Target::TreeScrollbar)) => self.scroll_press(bar, y),
            Some(Target::ToggleSidebar | Target::RevealSidebar) => self.toggle_panel(scene::Panel::Sidebar),
            Some(Target::ToggleToolbar | Target::RevealToolbar) => self.toggle_panel(scene::Panel::Toolbar),
            Some(Target::SidebarEdge) => self.grab_sidebar_edge(x, clicks),
            Some(Target::Minimal) => self.set_minimal(true),
            Some(Target::ToMain) if self.session.minimal => self.set_minimal(false),
            Some(Target::Heading(i)) => match self.heading_line(i) {
                Some(line) => self.jump_to(line),
                None => false,
            },
            Some(Target::Suggestion(i)) => self.accept_suggestion(i),
            Some(Target::Row(i)) => self.click_row(i),
            t @ Some(Target::Recent(_) | Target::Favorite(_) | Target::FoldFavorites | Target::FoldRecent) => self.kept_click(t).unwrap_or(false),
            Some(Target::RowMenu(i)) => {
                let items = self.row_menu(i);
                self.request_menu(Target::RowMenu(i), items, false)
            }
            Some(Target::ToMain) if !self.session.note.is_empty() => {
                self.save_flush();
                self.settings.requests.push(settings::Request::ToMain(self.session.note.clone()));
                true
            }
            Some(Target::SendTo) => {
                let items = self.send_menu();
                self.request_menu(Target::SendTo, items, true)
            }
            Some(Target::NewNote) => self.new_note(),
            Some(Target::Update) => self.install_update(),
            Some(
                t @ (Target::FindField
                | Target::ReplaceField
                | Target::FindPrev
                | Target::FindNext
                | Target::FindReplace
                | Target::FindClose
                | Target::ReplaceOne
                | Target::ReplaceAll),
            ) => self.find_click(t),
            Some(Target::Search) => self.open_search(),
            // The field's ✕: the search ends, the notes come back.
            Some(Target::SearchClear) => self.close_search(),
            Some(Target::Result(i)) => self.open_hit(i, false),
            Some(Target::Quick(kolada_ui::quick::Hit::Show)) => {
                self.settings.requests.push(settings::Request::QuickOpen);
                false
            }
            Some(Target::Trash) if self.trash.is_some() => self.trash.take().is_some(),
            Some(Target::Trash) => self.open_trash(),
            Some(Target::TrashBack) => self.trash.take().is_some(),
            Some(Target::TrashEmpty) => self.empty_trash(),
            Some(Target::TrashRow(i)) | Some(Target::TrashRestore(i)) => self.restore(i),
            Some(Target::ThemeCycle) => self.cycle_theme(),
            Some(Target::TextSize(up)) => {
                // A size smaller or larger; nothing past the ends.
                let Some(next) = scene::step_text_size(self.session.text_size, up) else { return false };
                self.session.text_size = next;
                self.fonts = Fonts::new(scene::text_scale(next), self.session.font);
                true
            }
            Some(Target::LineWidth) => {
                // Narrow (1), default (0), wide (2), and around.
                self.session.line_width = match self.session.line_width {
                    1 => 0,
                    0 => 2,
                    _ => 1,
                };
                true
            }
            Some(Target::Checkbox(at)) => {
                let before = self.editor.version();
                self.editor.toggle_task(at);
                self.dirty |= self.editor.version() != before;
                true
            }
            Some(Target::Tool(tool)) => self.tool(tool),
            Some(Target::SelBar(tool)) => self.selbar_tool(tool),
            Some(Target::Reading) => self.toggle_reading(),
            Some(Target::Width(width)) => {
                self.session.line_width = width;
                true
            }
            Some(Target::ReadingPanel | Target::SelBarPanel) => true,
            // A picture is chosen whole; its right edge drags to resize it.
            Some(Target::Picture(at)) => self.select_picture(at),
            Some(Target::PictureEdge(at)) => self.begin_resize(at, x),
            Some(Target::Note) => {
                self.note_used(false);
                if let Some(f) = &mut self.find {
                    f.focus = find::Focus::Page;
                }
                // The note takes the keys; a search stays open until it is written in.
                if let Some(s) = &mut self.search {
                    s.focused = false;
                }
                let at = self.offset_at(x, y);
                match clicks {
                    2 => self.editor.select_word_at(at),
                    n if n >= 3 => self.editor.select_line_at(at),
                    _ if shift => self.editor.set_selection(Selection { anchor: self.editor.selection().anchor, head: at }),
                    _ => self.editor.set_selection(Selection::caret(at)),
                }
                self.reveal = false;
                self.caret_moved(true);
                true
            }
            _ => card,
        }
    }

    /// The button is held and the pointer is at (x, y): the selection follows.
    pub fn pointer_dragged(&mut self, x: f32, y: f32) -> bool {
        if self.past.scrub.is_some() {
            return self.scrub_dragging() && self.scrub_drag(x);
        }
        if self.resizing.is_some() {
            return self.resize_to(x);
        }
        if let Some(done) = self.margin_dragged(x, y) {
            return done;
        }
        if let Some(done) = self.popup_dragged(x, y) {
            return done;
        }
        if let Some(done) = self.drag_sidebar_edge(x) {
            return done;
        }
        if let Some(done) = self.social_dragged(x, y) {
            return done;
        }
        if self.ui.scroll_drag.is_some() {
            return self.scroll_drag_to(y);
        }
        // Only a press on the page selects as it drags: not one on a button,
        // a row or the chrome (a button that changed the window's size
        // under the pointer once dragged a selection out of the note).
        if self.ui.pressed != Some(Target::Note) {
            return false;
        }
        let at = self.offset_at(x, y);
        let sel = self.editor.selection();
        if sel.head == at {
            return false;
        }
        self.editor.set_selection(Selection { anchor: sel.anchor, head: at });
        // The caret stays under the pointer.
        self.caret_moved(false);
        true
    }

    pub(super) fn click_row(&mut self, i: usize) -> bool {
        let Some(row) = self.session.sidebar.get(i).cloned() else { return false };
        let path = vault::row_paths(&self.session.sidebar).swap_remove(i);
        match row.kind {
            RowKind::Note => self.open(&format!("{path}.md")),
            RowKind::Folder { expanded: true } => vault::collapse(&mut self.session.sidebar, i),
            RowKind::Folder { expanded: false } => {
                let children = vault::children(&self.vault, &path, row.depth + 1).unwrap_or_default();
                vault::expand(&mut self.session.sidebar, i, children);
            }
        }
        true
    }
}

pub mod menu;
pub mod paste;
pub mod settings;
pub mod spelling;

#[cfg(test)]
mod tests;
