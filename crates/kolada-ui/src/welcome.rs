//! The welcome screen a first run opens on, in place of a note: the bird,
//! "Welcome to Kolada" and one quiet line; then three things, each a few
//! controls. BRING YOUR NOTES: a grid of quiet tiles, one for each place
//! notes come from (a small mark, the app's name, a faint hint of what to
//! pick), and under it one line: where to drop files, or how an import is
//! coming along, or what it came to. WRITING HELP: the writing model's
//! switch, Light | Full under it while it is on. YOUR DEVICES: Sync and
//! Beam's switch, pairing under it (the same rows as Settings). Then a teal
//! "Start writing", which puts it away for a fresh page.
//!
//! What it holds is `Welcome` (plain data, every shell's); the shell picks
//! the files a tile asks for (`From::pick`), brings them in on a worker
//! (`kolada_convert::bring`) and tells it how that goes. The switches are
//! Settings' own (`Target::Switch`, `Target::Choice`), and pairing is
//! Settings' Devices (`quick::devices_panel`): each shell handles them as
//! it does there. `draw` puts it on the screen; Tab walks its controls
//! (`next_focus`).

use kolada_text::{Align, Format};
use windows_core::Result;

use crate::chrome::{self, Painter};
use crate::geom::{Fill, Ink, Rect};
use crate::scene::{self, Eng, Fonts, Input, Platform, Scene, Setting, Target, VectorMark};
use crate::vector::Logo;

/// Where notes come from: each a tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum From {
    Obsidian,
    /// Apple Notes on this Mac (the Mac's only).
    AppleNotes,
    Keep,
    Bear,
    Evernote,
    Notion,
    /// The Notes app's export (`.nbk`).
    NotesApp,
    /// Any folder of Markdown or text.
    Folder,
    /// Word, PDF and web pages.
    Documents,
}

/// A tile's mark: a glyph of the icon font, or one drawn as a vector at
/// the same size and ink (the font has no archive box nor note page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Glyph(&'static str),
    Drawn(Logo),
}

/// Every tile, in the grid's order.
pub const ALL: [From; 9] = [From::Obsidian, From::AppleNotes, From::Keep, From::Bear, From::Evernote, From::Notion, From::NotesApp, From::Folder, From::Documents];

/// What a tile asks the shell to pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A folder (the picker's title).
    Folder(&'static str),
    /// Files of these kinds (the picker's title, what they are called, and
    /// their extensions), one or `many`.
    Files { title: &'static str, kind: &'static str, exts: &'static [&'static str], many: bool },
    /// Nothing to pick: read where it is (Apple Notes, on this Mac).
    Nothing,
}

impl From {
    /// Its name, as its tile says it.
    pub fn name(self) -> &'static str {
        match self {
            From::Obsidian => "Obsidian",
            From::AppleNotes => "Apple Notes",
            From::Keep => "Google Keep",
            From::Bear => "Bear",
            From::Evernote => "Evernote",
            From::Notion => "Notion",
            From::NotesApp => "Notes app",
            From::Folder => "Any folder",
            From::Documents => "Documents",
        }
    }

    /// What to pick, faintly under its name.
    pub fn hint(self) -> &'static str {
        match self {
            From::Obsidian => "A vault folder",
            From::AppleNotes => "On this Mac",
            From::Keep => "A Takeout .zip",
            From::Bear => "A backup or export",
            From::Evernote => ".enex",
            From::Notion => "An export .zip",
            From::NotesApp => ".nbk",
            From::Folder => "Markdown or text",
            From::Documents => "Word, PDF, web pages",
        }
    }

    /// Its small mark, by the kind of thing it reads, one of Kolada's own
    /// filled marks (never another app's logo): a vault, a folder, an
    /// archive box for a zip, a document, a note, a list.
    fn mark(self) -> Mark {
        match self {
            From::Obsidian => Mark::Glyph("\u{E8B7}"),
            From::Folder => Mark::Glyph("\u{E838}"),
            From::Keep | From::Notion | From::Bear => Mark::Drawn(Logo::Archive),
            From::Evernote | From::Documents => Mark::Glyph("\u{E7C3}"),
            From::NotesApp => Mark::Drawn(Logo::NotePage),
            From::AppleNotes => Mark::Glyph("\u{E8FD}"),
        }
    }

    /// Whether its tile shows on `platform`: Apple Notes only on the Mac.
    pub fn offered(self, platform: Platform) -> bool {
        self != From::AppleNotes || platform == Platform::Mac
    }

    /// Whether Kolada reads it yet: a tile whose reader is on its way says
    /// so (`Line::Coming`). Every one reads now.
    pub fn ready(self) -> bool {
        true
    }

    pub fn pick(self) -> Pick {
        match self {
            From::Obsidian => Pick::Folder("Choose your Obsidian vault"),
            From::Folder => Pick::Folder("Choose a folder of notes"),
            From::AppleNotes => Pick::Nothing,
            From::Keep => Pick::Files { title: "Choose your Google Takeout", kind: "Takeout", exts: &["zip"], many: false },
            From::Bear => Pick::Files { title: "Choose your Bear backup", kind: "Bear backup or export", exts: &["bear2bk", "zip"], many: false },
            From::Evernote => Pick::Files { title: "Choose your Evernote export", kind: "Evernote export", exts: &["enex"], many: true },
            From::Notion => Pick::Files { title: "Choose your Notion export", kind: "Notion export", exts: &["zip"], many: false },
            From::NotesApp => Pick::Files { title: "Choose your Notes export", kind: "Notes export", exts: &["nbk"], many: false },
            From::Documents => Pick::Files { title: "Choose documents", kind: "Word, PDF and web pages", exts: &["docx", "pdf", "html", "htm"], many: true },
        }
    }

    /// The vault folder what it brings goes into (a folder brought in goes
    /// into one named after it instead).
    pub fn folder(self) -> String {
        match self {
            From::Documents => "From documents".to_string(),
            From::NotesApp => "From Notes".to_string(),
            f => format!("From {}", f.name()),
        }
    }

    /// How a sentence names it ("530 notes from Evernote ...").
    pub fn said(self) -> &'static str {
        match self {
            From::Documents => "your documents",
            From::Folder => "the folder",
            f => f.name(),
        }
    }
}

/// What `kolada_convert::bring` says of a file that is not an export:
/// said as it is.
const NOT_AN_EXPORT: &str = "This isn't an export Kolada can read";

/// The line under the tiles.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Line {
    /// Where to drop files.
    #[default]
    Rest,
    /// A tile whose reader is on its way.
    Coming(From),
    /// Bringing in: notes done of all of them (0 while counting); `said`
    /// names where they come from.
    Bringing { from: From, said: String, done: usize, total: usize },
    /// Brought in: how many notes, into which folder of the vault, and how
    /// many files could not be read.
    Brought { from: From, said: String, count: usize, folder: String, failed: usize },
    /// Nothing could be brought in, and why.
    Failed { from: From, said: String, why: String },
    /// Stopped part way: nothing kept.
    Stopped { from: From, said: String },
}

/// The welcome screen's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Welcome {
    pub line: Line,
    /// Tiles brought in from: their marks are teal, ticked.
    pub brought: Vec<From>,
    /// How far it is scrolled, DIPs (clamped by `draw` to the scene's
    /// `welcome_scroll_max`).
    pub scroll: f32,
    /// The control Tab came to (drawn as the pointer's hover is).
    pub focus: Option<Target>,
}

/// What a click on the screen's own parts is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    /// The screen itself, between its controls.
    Page,
    Tile(From),
    /// "Stop", while bringing in.
    Stop,
    Start,
}

/// What the shell does after a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    None,
    /// Only the screen changed.
    Redraw,
    /// Ask for what this tile reads (`From::pick`), then bring it in.
    Pick(From),
    /// Stop the import under way.
    Stop,
    /// Put the welcome away, a fresh page in its place.
    Start,
}

impl Welcome {
    pub fn new() -> Welcome {
        Welcome::default()
    }

    /// Bringing something in now.
    pub fn busy(&self) -> bool {
        matches!(self.line, Line::Bringing { .. })
    }

    /// Bringing in what can be stopped part way: not Apple Notes, asked of
    /// Notes itself.
    pub fn stoppable(&self) -> bool {
        matches!(self.line, Line::Bringing { from, .. } if from != From::AppleNotes)
    }

    pub fn click(&mut self, hit: Hit) -> Command {
        match hit {
            Hit::Page => Command::None,
            Hit::Tile(_) if self.busy() => Command::None,
            Hit::Tile(from) if !from.ready() => {
                self.line = Line::Coming(from);
                Command::Redraw
            }
            Hit::Tile(from) => Command::Pick(from),
            Hit::Stop if self.stoppable() => Command::Stop,
            Hit::Stop => Command::None,
            Hit::Start => Command::Start,
        }
    }

    /// An import began, from what `said` names.
    pub fn began(&mut self, from: From, said: &str) {
        self.line = Line::Bringing { from, said: said.to_string(), done: 0, total: 0 };
    }

    /// How far it has come; whether that changed what shows.
    pub fn progress(&mut self, (now, of): (usize, usize)) -> bool {
        match &mut self.line {
            Line::Bringing { done, total, .. } if (*done, *total) != (now, of) => {
                (*done, *total) = (now, of);
                true
            }
            _ => false,
        }
    }

    /// The export turned out to be `from`'s (a Takeout on Notion's tile,
    /// Bear's bundles dropped), or plain Markdown zipped, a folder named
    /// `said`: the line names it so.
    pub fn found(&mut self, from: From, said: &str) {
        if let Line::Bringing { from: f, said: s, .. } = &mut self.line {
            (*f, *s) = (from, said.to_string());
        }
    }

    /// It is over: `count` notes into `folder` (`failed` files not read),
    /// stopped, or not brought in at all.
    pub fn finished(&mut self, outcome: Outcome) {
        let (from, said) = match &self.line {
            Line::Bringing { from, said, .. } => (*from, said.clone()),
            _ => return,
        };
        self.line = match outcome {
            Outcome::Brought { count, folder, failed } => {
                if !self.brought.contains(&from) {
                    self.brought.push(from);
                }
                Line::Brought { from, said, count, folder, failed }
            }
            Outcome::Stopped => Line::Stopped { from, said },
            Outcome::Failed(why) => Line::Failed { from, said, why },
        };
    }

    /// The line's words.
    pub fn words(&self) -> String {
        match &self.line {
            Line::Rest => String::new(),
            Line::Coming(from) => format!("{} is coming in a moment.", from.name()),
            Line::Bringing { said, done, total, from } => {
                let what = if *from == From::Documents { "files" } else { "notes" };
                if *total == 0 { format!("Bringing in {said}\u{2026}") } else { format!("Bringing in {said}\u{2026} {} of {} {what}", thousands(*done), thousands(*total)) }
            }
            Line::Brought { said, count, folder, failed, .. } => {
                let notes = if *count == 1 { "1 note".to_string() } else { format!("{} notes", thousands(*count)) };
                let are = if *count == 1 { "is" } else { "are" };
                let left = if *failed == 0 { String::new() } else if *failed == 1 { "; 1 file could not be read".to_string() } else { format!("; {failed} files could not be read") };
                format!("{notes} from {said} {are} in the folder \u{2018}{folder}\u{2019}{left}")
            }
            Line::Failed { why, .. } if why == NOT_AN_EXPORT => format!("{why}."),
            Line::Failed { said, why, .. } => format!("Could not bring in {said}: {why}"),
            Line::Stopped { said, .. } => format!("Stopped. Nothing from {said} was kept."),
        }
    }
}

/// What files dropped on the welcome screen are brought in as: a folder
/// (an Obsidian vault when it has `.obsidian`), another app's export, or
/// documents and notes; and the files of that kind. None when it reads
/// none of them.
pub fn dropped(files: &[std::path::PathBuf]) -> Option<(From, Vec<std::path::PathBuf>)> {
    let ext = |f: &std::path::PathBuf| f.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if let Some(dir) = files.iter().find(|f| f.is_dir()) {
        let from = if dir.join(".obsidian").is_dir() { From::Obsidian } else { From::Folder };
        return Some((from, vec![dir.clone()]));
    }
    // A zip is taken for Notion's until it is read (Keep's Takeout, Bear's).
    for (from, exts) in [(From::Evernote, &["enex"][..]), (From::Bear, &["bear2bk"]), (From::Notion, &["zip"]), (From::NotesApp, &["nbk"])] {
        let of: Vec<std::path::PathBuf> = files.iter().filter(|f| exts.contains(&ext(f).as_str())).cloned().collect();
        if !of.is_empty() {
            // One export at a time, but every .enex of a drop.
            let of = if from == From::Evernote { of } else { of.into_iter().take(1).collect() };
            return Some((from, of));
        }
    }
    let read = ["docx", "pdf", "html", "htm", "xhtml", "md", "markdown", "txt"];
    let documents: Vec<std::path::PathBuf> = files.iter().filter(|f| read.contains(&ext(f).as_str())).cloned().collect();
    (!documents.is_empty()).then_some((From::Documents, documents))
}

/// How an import ended (see `Welcome::finished`).
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Brought { count: usize, folder: String, failed: usize },
    Stopped,
    Failed(String),
}

/// `n` with thousands set apart: 1,234.
fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What the scene draws the screen from.
pub struct View<'a> {
    pub welcome: &'a Welcome,
    /// The writing model chosen, by its place (None: none yet, so neither
    /// Light nor Full is lit: picking one downloads it, as in Settings).
    pub model: Option<usize>,
}

const COLUMN: f32 = 760.0;
const TILE_H: f32 = 64.0;
const TILE_MIN: f32 = 150.0;
const GAP: f32 = 10.0;
const MARK: f32 = 30.0;
const BIRD: f32 = 44.0;
const ROW_H: f32 = 40.0;
const SECTION_GAP: f32 = 34.0;
const LABEL_H: f32 = 24.0;
const LINE_H: f32 = 28.0;
const START_H: f32 = 40.0;
const CHOICE_W: f32 = 240.0;
/// The window's top row, where its buttons are: the screen scrolls under
/// it, fading out over `FADE` (as the note does under it).
const TOP: f32 = 40.0;
const FADE: f32 = 12.0;

/// How many tiles a row holds in a column `w` wide, of `n`.
fn columns(w: f32, n: usize) -> usize {
    let fit = (((w + GAP) / (TILE_MIN + GAP)).floor() as usize).max(1);
    // Eight sit as two rows of four (or four of two); nine as three of
    // three: no row left short.
    match (n % 4 == 0, fit) {
        (true, 4..) => 4,
        (true, 2..) => 2,
        (false, 3..) => 3,
        (_, f) => f,
    }
}

/// A switch's rows: how far open (1 shown, 0 folded away), as they fold on
/// a spring like Settings' (`Input::folding`).
fn open(input: &Input, setting: Setting) -> f32 {
    match input.folding {
        Some((s, t)) if s == setting => t.max(0.0),
        _ => scene::switch_on(setting, input.options[setting.index()]) as u8 as f32,
    }
}

/// Draws the screen over the whole window, under its buttons.
pub fn draw(eng: &Eng, fonts: &Fonts, input: &Input, view: &View, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    let w = view.welcome;
    scene.overlay(None);
    let whole = Rect::new(0.0, 0.0, width, height);
    let hover = w.focus.or(input.hover);
    let mut p = Painter { eng, fonts, hover, scene };
    p.scene.fills.push(Fill::solid(whole, 0.0, Ink::Page));
    p.scene.hits.push((whole, Target::Welcome(Hit::Page)));

    let side = if width < 480.0 { 16.0 } else { 28.0 };
    let cw = (width - 2.0 * side).min(COLUMN).max(1.0);
    let left = (width - cw) / 2.0;
    let right = left + cw;
    let tiles: Vec<From> = ALL.into_iter().filter(|f| f.offered(input.platform)).collect();
    let cols = columns(cw, tiles.len());
    let rows = tiles.len().div_ceil(cols);
    let tw = (cw - GAP * (cols - 1) as f32) / cols as f32;
    let assist = open(input, Setting::Assist);
    let beam = open(input, Setting::Beam);
    let devices_h = input.devices.as_ref().map_or(0.0, crate::quick::devices_height);

    // How tall it all is, to centre it when the window has room, and to
    // scroll it when not.
    let head = BIRD + 18.0 + 40.0 + 26.0;
    let notes = LABEL_H + rows as f32 * TILE_H + (rows - 1) as f32 * GAP + 12.0 + LINE_H;
    let help = LABEL_H + ROW_H + assist * (ROW_H + 4.0);
    let devices = LABEL_H + ROW_H + beam * (devices_h + 8.0);
    let footer = START_H;
    let body = head + SECTION_GAP + notes + SECTION_GAP + help + SECTION_GAP + devices + SECTION_GAP + footer;
    let room = height - TOP - 24.0;
    let max = (body - room).max(0.0);
    p.scene.welcome_scroll_max = max;
    let scroll = w.scroll.clamp(0.0, max);
    let mut y = if max > 0.0 { TOP + 8.0 } else { TOP + ((room - body) / 2.0).max(8.0) } - scroll;
    // What scrolls up passes under the window's top row, dissolving into
    // the page there as the note does; it takes no clicks there.
    let view_r = Rect::new(0.0, TOP, width, height);
    p.scene.overlay(Some(Rect::new(0.0, 0.0, width, height)));
    let hits_from = p.scene.hits.len();

    // The logo (the user's drawing: the teal bird, its white wings and
    // eye); the title in the interface's own face; one line.
    let mid = width / 2.0;
    let bird = Rect::new(mid - BIRD / 2.0, y, mid + BIRD / 2.0, y + BIRD);
    for (logo, ink, alpha) in [(Logo::Mark, Ink::Accent, 1.0), (Logo::MarkWings, Ink::OnAccent, 0.95), (Logo::MarkEye, Ink::OnAccent, 1.0)] {
        p.scene.vectors.push(VectorMark { layer: p.scene.layer_now(), rect: bird, logo, ink, alpha });
    }
    y += BIRD + 18.0;
    let title_size = if cw < 360.0 { 24.0 } else { 30.0 };
    let title = Format { size: title_size, weight: 600, align: Align::Center, center_y: true, wrap: false, ellipsis: true, ..fonts.label };
    p.text("Welcome to Kolada", &title, Rect::new(left, y, right, y + 40.0), Ink::Text)?;
    y += 40.0;
    let quiet = Format { align: Align::Center, center_y: true, ..fonts.label };
    p.text("Three things, then it\u{2019}s yours.", &quiet, Rect::new(left, y, right, y + 26.0), Ink::Muted)?;
    y += 26.0 + SECTION_GAP;

    // Bring your notes.
    section(&mut p, "BRING YOUR NOTES", left, right, y)?;
    y += LABEL_H;
    let name = Format { center_y: true, wrap: false, ellipsis: true, ..fonts.label };
    let hint = Format { center_y: true, wrap: false, ellipsis: true, ..fonts.small };
    let icon = Format { align: Align::Center, center_y: true, ..fonts.icon };
    for (i, &from) in tiles.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let x = left + c as f32 * (tw + GAP);
        let t = Rect::new(x, y + r as f32 * (TILE_H + GAP), x + tw, y + r as f32 * (TILE_H + GAP) + TILE_H);
        let target = Target::Welcome(Hit::Tile(from));
        let lit = p.hover == Some(target) && !w.busy();
        p.scene.fills.push(Fill::solid(t, 12.0, if lit { Ink::Hover } else { Ink::Field }));
        p.scene.hits.push((t, target));
        let m = Rect::new(t.left + 14.0, t.top + (TILE_H - MARK) / 2.0, t.left + 14.0 + MARK, t.top + (TILE_H + MARK) / 2.0);
        let done = w.brought.contains(&from);
        p.scene.fills.push(Fill::solid(m, 8.0, if done { Ink::Accent } else { Ink::Button }));
        if done {
            p.centred("\u{E73E}", &icon, m, Ink::OnAccent)?;
        } else {
            match from.mark() {
                Mark::Glyph(g) => p.centred(g, &icon, m, Ink::Muted)?,
                Mark::Drawn(logo) => {
                    let k = 15.0;
                    let r = Rect::new((m.left + m.right - k) / 2.0, (m.top + m.bottom - k) / 2.0, (m.left + m.right + k) / 2.0, (m.top + m.bottom + k) / 2.0);
                    p.scene.vectors.push(VectorMark { layer: p.scene.layer_now(), rect: r, logo, ink: Ink::Muted, alpha: 1.0 });
                }
            }
        }
        let tx = m.right + 12.0;
        let words = Rect::new(tx, t.top + 13.0, t.right - 10.0, t.top + 33.0);
        p.text(from.name(), &name, words, if w.busy() && !lit { Ink::Muted } else { Ink::Text })?;
        p.text(from.hint(), &hint, Rect::new(tx, words.bottom, t.right - 10.0, words.bottom + 18.0), Ink::Faint)?;
    }
    y += rows as f32 * TILE_H + (rows - 1) as f32 * GAP + 12.0;
    status_line(&mut p, w, left, right, y)?;
    y += LINE_H + SECTION_GAP;

    // Writing help: the writing model's switch, its choice under it.
    section(&mut p, "WRITING HELP", left, right, y)?;
    y += LABEL_H;
    // The switches and what they hold end where the tiles do.
    let near = right;
    switch_row(&mut p, input, "Writing model", Setting::Assist, left, near, y)?;
    y += ROW_H;
    if assist > 0.0 {
        let fold = Rect::new(0.0, y, width, y + assist * (ROW_H + 4.0));
        p.scene.overlay(Some(clip(fold, Rect::new(0.0, 0.0, width, height))));
        let slide = (1.0 - assist.min(1.0)) * (ROW_H + 4.0);
        let bar = Rect::new(left, y + 2.0 - slide, left + CHOICE_W.min(cw), y + 2.0 - slide + 36.0);
        let value = input.options[Setting::Assist.index()];
        // As Settings shows it: the model chosen, Light until one is.
        let chosen = (value != crate::modelcard::OFF).then_some(Target::Choice(Setting::Assist, value.min(1)));
        let hits_at = p.scene.hits.len();
        chrome::segments(&mut p, bar, &[("Light", Target::Choice(Setting::Assist, 0)), ("Full", Target::Choice(Setting::Assist, 1))], chosen)?;
        // The one chosen, plain on graphite as on paper: the accent's tint
        // over its raised segment, as the model card lights its choice.
        if let Some(c) = chosen
            && let Some(r) = p.scene.hits[hits_at..].iter().find(|(_, t)| *t == c).map(|(r, _)| *r)
        {
            p.scene.fills.push(Fill::faded(r, 6.0, Ink::Accent, 0.22));
        }
        clip_hits(p.scene, hits_at, fold);
        p.scene.overlay(Some(Rect::new(0.0, 0.0, width, height)));
        y += assist * (ROW_H + 4.0);
    }
    y += SECTION_GAP;

    // Your devices: Sync and Beam's switch, pairing under it.
    section(&mut p, "YOUR DEVICES", left, right, y)?;
    y += LABEL_H;
    switch_row(&mut p, input, "Sync and Beam", Setting::Beam, left, near, y)?;
    y += ROW_H;
    if beam > 0.0 {
        if let Some(v) = &input.devices {
            let shown = beam * (devices_h + 8.0);
            let fold = Rect::new(0.0, y, width, y + shown);
            p.scene.overlay(Some(clip(fold, Rect::new(0.0, 0.0, width, height))));
            let slide = (1.0 - beam.min(1.0)) * (devices_h + 8.0);
            let hits_at = p.scene.hits.len();
            crate::quick::devices_panel(&mut p, v, left, near, y + 4.0 - slide, true)?;
            clip_hits(p.scene, hits_at, fold);
            p.scene.overlay(Some(Rect::new(0.0, 0.0, width, height)));
            y += shown;
        }
    }
    y += SECTION_GAP;

    // Start writing, and the one faint line beside it (under it, narrow).
    let word = Format { size: 13.0, weight: 600, align: Align::Center, center_y: true, ..fonts.label };
    let sw = scene::single_line(eng, "Start writing", &word, 400.0, START_H).measure().width + 44.0;
    let start = Rect::new(left, y, left + sw, y + START_H);
    let target = Target::Welcome(Hit::Start);
    let hot = p.hover == Some(target);
    p.scene.fills.push(Fill::shadow(start.offset(0.0, 3.0), START_H / 2.0, 12.0, if hot { 1.1 } else { 0.7 }));
    p.scene.fills.push(Fill::solid(start, START_H / 2.0, Ink::Accent));
    if hot {
        p.scene.fills.push(Fill::faded(start, START_H / 2.0, Ink::OnAccent, 0.14));
    }
    p.scene.hits.push((start, target));
    p.text("Start writing", &word, start, Ink::OnAccent)?;

    let scrolled = (scroll / 24.0).clamp(0.0, 1.0);
    if scrolled > 0.0 {
        p.scene.fills.push(Fill { blur: FADE, alpha: scrolled, ..Fill::solid(Rect::new(-FADE, -2.0 * FADE, width + FADE, TOP - FADE / 2.0), 0.0, Ink::Page) });
    }
    // Tab walks every control, in view or not; what has scrolled under the
    // window's top row takes no clicks.
    let controls: Vec<(Rect, Target)> = p.scene.hits[hits_from..].iter().filter(|(_, t)| focusable(*t)).copied().collect();
    p.scene.welcome_controls = controls;
    clip_hits(p.scene, hits_from, view_r);
    p.scene.overlay(None);
    // The window's own buttons stay over it.
    chrome::window_buttons(&mut p, input, width)?;
    Ok(())
}

/// The part of `r` inside `within`.
fn clip(r: Rect, within: Rect) -> Rect {
    Rect::new(r.left.max(within.left), r.top.max(within.top), r.right.min(within.right), r.bottom.min(within.bottom).max(r.top.max(within.top)))
}

/// The hits pushed since `from` cut to `within`; a sliver too thin to
/// click is dropped.
fn clip_hits(scene: &mut Scene, from: usize, within: Rect) {
    let hits: Vec<(Rect, Target)> = scene.hits.drain(from..).collect();
    scene.hits.extend(hits.into_iter().map(|(r, t)| (clip(r, within), t)).filter(|(r, _)| r.height() >= 8.0 && r.width() > 0.0));
}

/// A section's small heading.
fn section(p: &mut Painter, words: &str, left: f32, right: f32, y: f32) -> Result<()> {
    let f = Format { center_y: true, wrap: false, ..p.fonts.section };
    p.text(words, &f, Rect::new(left, y, right, y + LABEL_H - 6.0), Ink::Faint)
}

/// A switch's row: its name, and the switch at the row's end.
fn switch_row(p: &mut Painter, input: &Input, words: &str, setting: Setting, left: f32, right: f32, y: f32) -> Result<()> {
    let row = Rect::new(left, y, right, y + ROW_H);
    let target = Target::Switch(setting);
    let name = Format { center_y: true, wrap: false, ellipsis: true, ..p.fonts.label };
    p.text(words, &name, Rect::new(left, y, right - chrome::SWITCH_W - 16.0, y + ROW_H), Ink::Text)?;
    // Its track ends where the tiles do (`chrome::switch` sets it 2 in).
    let bar = Rect::new(right - chrome::SWITCH_W - 2.0, y, right, y + ROW_H);
    chrome::switch(p, input, bar, setting);
    // The whole row turns it: its name too.
    p.scene.hits.push((row, target));
    Ok(())
}

/// The line under the tiles: where to drop files, or how an import goes
/// (with a thin teal bar and Stop), or what it came to.
fn status_line(p: &mut Painter, w: &Welcome, left: f32, right: f32, y: f32) -> Result<()> {
    let r = Rect::new(left, y, right, y + LINE_H);
    let words = w.words();
    let f = Format { center_y: true, wrap: false, ellipsis: true, ..p.fonts.small };
    let ink = match w.line {
        Line::Rest => Ink::Faint,
        Line::Failed { .. } => Ink::Danger,
        _ => Ink::Muted,
    };
    if let Line::Bringing { done, total, .. } = w.line {
        let mut end = right;
        if w.stoppable() {
            let stop_f = Format { align: Align::Center, center_y: true, ..p.fonts.small };
            let sw = scene::single_line(p.eng, "Stop", &stop_f, 200.0, LINE_H).measure().width + 20.0;
            let stop = Rect::new(right - sw, y + 2.0, right, y + LINE_H - 2.0);
            p.button(stop, Target::Welcome(Hit::Stop), 8.0);
            p.text("Stop", &stop_f, stop, if p.hover == Some(Target::Welcome(Hit::Stop)) { Ink::Text } else { Ink::Muted })?;
            end = stop.left - 12.0;
        }
        p.text(&words, &f, Rect::new(left, y, end, y + LINE_H - 4.0), ink)?;
        // How far, along the line's foot.
        let track = Rect::new(left, y + LINE_H - 3.0, end, y + LINE_H - 1.0);
        p.scene.fills.push(Fill::solid(track, 1.0, Ink::Field));
        let k = if total == 0 { 0.06 } else { (done as f32 / total as f32).clamp(0.0, 1.0) };
        p.scene.fills.push(Fill::solid(Rect::new(track.left, track.top, track.left + track.width() * k, track.bottom), 1.0, Ink::Accent));
        return Ok(());
    }
    p.text(&words, &f, r, ink)
}

/// Whether Tab stops at `t` on the welcome screen.
pub fn focusable(t: Target) -> bool {
    use crate::quick::Hit as Q;
    matches!(
        t,
        Target::Welcome(Hit::Tile(_) | Hit::Stop | Hit::Start)
            | Target::Switch(Setting::Assist | Setting::Beam)
            | Target::Choice(Setting::Assist, _)
            | Target::Quick(Q::ShowCode | Q::EnterCode | Q::CodeField | Q::Pair | Q::Cancel | Q::Allow | Q::Deny | Q::Remove(_) | Q::Rename | Q::NameField | Q::NameSave | Q::NameCancel)
    )
}

/// The control Tab (or Shift+Tab, `back`) goes to from `now`, among the
/// screen's (`Scene::welcome_controls`, in the order drawn): round to the
/// first after the last.
pub fn next_focus(hits: &[(Rect, Target)], now: Option<Target>, back: bool) -> Option<Target> {
    let mut order: Vec<Target> = Vec::new();
    for &(_, t) in hits {
        if focusable(t) && !order.contains(&t) {
            order.push(t);
        }
    }
    if order.is_empty() {
        return None;
    }
    let at = now.and_then(|n| order.iter().position(|&t| t == n));
    let n = order.len();
    Some(match (at, back) {
        (None, false) => order[0],
        (None, true) => order[n - 1],
        (Some(i), false) => order[(i + 1) % n],
        (Some(i), true) => order[(i + n - 1) % n],
    })
}

/// Scrolls so the control `t` is in full view.
pub fn reveal(w: &mut Welcome, scene: &Scene, t: Target, height: f32) {
    let Some(r) = scene.welcome_controls.iter().find(|(_, x)| *x == t).map(|(r, _)| *r) else { return };
    let (top, bottom) = (TOP + 12.0, height - 16.0);
    if r.top < top {
        w.scroll -= top - r.top;
    } else if r.bottom > bottom {
        w.scroll += r.bottom - bottom;
    }
    w.scroll = w.scroll.clamp(0.0, scene.welcome_scroll_max);
}

/// A name for each control, for screen readers.
pub fn name(t: Target) -> Option<String> {
    Some(match t {
        Target::Welcome(Hit::Tile(f)) => format!("Bring in from {}: {}", f.name(), f.hint()),
        Target::Welcome(Hit::Stop) => "Stop bringing notes in".to_string(),
        Target::Welcome(Hit::Start) => "Start writing".to_string(),
        Target::Welcome(Hit::Page) => "Welcome to Kolada".to_string(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
