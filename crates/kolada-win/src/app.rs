//! The running app: window state, input, editing, and the window procedure.
//!
//! Frames are scheduled, not drawn per event: input marks the app dirty and
//! the message loop draws at most once per display refresh (see
//! `needs_frame`/`frame`), so a fast mouse or key repeat never queues
//! redundant frames.
//!
//! Notes save themselves. Every change schedules an autosave, and leaving a
//! note (for another, a fresh page, another app, or closing) saves at once.
//! A fresh page has no file until it has words; then it becomes a note named
//! after its first line. Every note's first line is its title, and its file
//! name follows that line (links to it in other notes follow too).

mod agenda;
mod assist;
mod bars;
mod find;
mod share_card;
mod daily;
mod link;
mod menu;
mod model_card;
mod outside;
mod palette;
mod arrive;
mod paste;
mod scrub;
mod compose;
mod pictures;
mod scroll;
mod share;
mod suggest;
mod trash;
mod tray;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use kolada_core::links;
use kolada_core::note::{self, Format};
use kolada_core::related::{Corpus, Related};
use kolada_core::search::{self, Hit, Indexed};
use kolada_core::session::{self, Placement, RowKind, Session, SidebarRow};
use kolada_core::vault;
use kolada_editor::{BlockKind, Editor, Inline, Motion, Selection, title};
use kolada_markdown::{self as md, Block, Style};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DWMWA_USE_IMMERSIVE_DARK_MODE;
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT, ScreenToClient};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_HOME, VK_INSERT, VK_LEFT, VK_MENU,
    VK_ESCAPE, VK_NEXT, VK_OEM_5, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GetMessageTime, GetSystemMetrics, GetWindowPlacement, HTCAPTION, HTCLIENT,
    HTCLOSE, HTMAXBUTTON, HTMINBUTTON, HTTOP, HTTOPLEFT, HTTOPRIGHT, IDC_ARROW, IDC_IBEAM, IDC_SIZEALL, IsIconic, IsZoomed,
    KillTimer, LoadCursorW, NCCALCSIZE_PARAMS, PostMessageW, PostQuitMessage, SM_CXDOUBLECLK, SM_CYDOUBLECLK,
    SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOWMAXIMIZED, SWP_NOACTIVATE, SWP_NOZORDER, SetCursor,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, WINDOWPLACEMENT, WM_ACTIVATE, WM_CAPTURECHANGED,
    WM_CHAR, WM_CLOSE, WM_COPYDATA, WM_DESTROY, WM_DPICHANGED, WM_DROPFILES, WM_GETMINMAXINFO, WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
    WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_PAINT, WM_QUERYENDSESSION, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SIZE,
    WM_SYSKEYDOWN, WM_TIMER, WM_XBUTTONUP,
};
use windows::core::Result;

use crate::chrome;
use crate::clipboard;
use crate::cpu::{self, Surface};
use kolada_ui::edit::apply_tool;
use crate::display;
use crate::note::NoteText;
use crate::scene::{self, CHIP_GAP, CHIP_HEIGHT, Chip, Fonts, Line, Panel, Rect, Scene, ScrollView, SearchView, Setting, Target, Tool};
use crate::wave::{self, SAMPLES, Wave};
use crate::theme;
use crate::window::{self, WM_APP_ASSIST, WM_APP_IMAGE, WM_APP_IMPORTED, WM_APP_INDEX, WM_APP_VAULT, WM_APP_SIDEBAR, WM_APP_SPELL, WM_APP_THEME, WM_APP_TRAY, WM_APP_WHISPER};

/// Set with `--no-activate`: windows this app opens (a note's own window)
/// show without taking the keyboard either.
pub static NO_ACTIVATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// windows-rs puts this one in UI::Controls, a large feature for one constant.
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Docking or undocking a panel.
const DOCK_ANIMATION: Duration = Duration::from_millis(180);
/// Revealing or hiding a panel in focus mode: quicker, it is transient.
const REVEAL_ANIMATION: Duration = Duration::from_millis(140);
/// The pointer must rest in a reveal area this long, so sweeping across
/// the window edge on the way elsewhere opens nothing.
const REVEAL_DELAY_MS: u32 = 50;
/// A revealed panel lingers this long after the pointer leaves it.
const HIDE_DELAY_MS: u32 = 300;

const AUTOSAVE_TIMER: usize = 30;
/// A pause this long in typing saves the note.
const AUTOSAVE_DELAY_MS: u32 = 300;
const RELATED_TIMER: usize = 40;
/// A pause this long in writing, or a move to another paragraph, updates
/// the related notes in the margin.
const RELATED_DELAY_MS: u32 = 350;
/// Related notes shown at most.
const CHIP_LIMIT: usize = 4;
/// A note that opens fades in over this long.
const PAGE_FADE: Duration = Duration::from_millis(170);
/// Resting on a chip this long peeks at its note; a peek fades in over the
/// second.
const PEEK_TIMER: usize = 41;
const PEEK_DELAY_MS: u32 = 280;
const PEEK_FADE: Duration = Duration::from_millis(130);
/// "Saved as ..." shows this long, fading in and out at its ends.
const WHISPER_TIMER: usize = 42;

/// Notes up to this long (bytes) count their words at every key; longer
/// ones when typing pauses this long.
const WORDS_AT_ONCE: usize = 64 * 1024;
const WORDS_PAUSE: Duration = Duration::from_millis(400);
const WORDS_TIMER: usize = 44;
const WHISPER: Duration = Duration::from_millis(2600);
const WHISPER_IN: Duration = Duration::from_millis(160);
const WHISPER_OUT: Duration = Duration::from_millis(550);
/// A note open over the page saves after a pause in typing, like the page;
/// it fades in over this long.
const POPUP_SAVE_TIMER: usize = 43;
const POPUP_FADE: Duration = Duration::from_millis(140);
/// One notch of the mouse wheel: three lines of body text, DIPs.
const WHEEL_STEP: f32 = 72.0;
/// A scroll eases in: what is left of it shrinks at this rate per second.
const SCROLL_RATE: f32 = 18.0;
/// Room kept between the caret and the bottom of the window, DIPs.
const CARET_MARGIN: f32 = 40.0;
/// Stiffness of the caret's glide (rad/s), critically damped: it arrives
/// quickly and never overshoots.
const CARET_OMEGA: f32 = 42.0;
/// Rough line height for paging, DIPs.
const PAGE_LINE: f32 = 24.0;

const PANELS: [Panel; 2] = [Panel::Sidebar, Panel::Toolbar];
/// Leaving the window this close to its left edge counts as reaching for
/// the side menu, DIPs.
const OVERSHOOT: f32 = 90.0;

fn index(panel: Panel) -> usize {
    match panel {
        Panel::Sidebar => 0,
        Panel::Toolbar => 1,
    }
}

/// Timer ids: reveal and hide, for each panel.
fn reveal_timer(panel: Panel) -> usize {
    10 + index(panel)
}

fn hide_timer(panel: Panel) -> usize {
    20 + index(panel)
}

/// Chrome state that is not saved: what the pointer is over and where, and
/// how far each panel is docked and shown (0..1, in between while animating).
#[derive(Clone, Copy, Debug)]
pub struct Ui {
    pub hover: Option<Target>,
    pub pointer: Option<(f32, f32)>,
    /// Per panel, by `index`.
    pub dock: [f32; 2],
    pub show: [f32; 2],
    /// Each collapsed pill's pull toward the pointer, as its spring has it.
    pub wave: [[f32; SAMPLES]; 2],
    pub maximized: bool,
    /// The first file tree row shown, whether the options are open, and
    /// how far they are scrolled.
    pub tree_top: usize,
    pub options_open: bool,
    pub options_scroll: f32,
}

impl Ui {
    /// Resting state for a session in a window `width` DIPs wide: panels
    /// docked or in focus mode (the side menu tucked away where it does not
    /// fit).
    pub fn resting(s: &Session, width: f32) -> Ui {
        let side = !s.sidebar_collapsed && !s.minimal && chrome::sidebar_fits(width);
        let d = [side as u8 as f32, (!s.toolbar_collapsed && !s.minimal) as u8 as f32];
        Ui {
            hover: None,
            pointer: None,
            dock: d,
            show: d,
            wave: [[0.0; SAMPLES]; 2],
            maximized: s.window.maximized,
            tree_top: 0,
            options_open: false,
            options_scroll: 0.0,
        }
    }

    fn values(&self) -> [f32; 4] {
        [self.dock[0], self.dock[1], self.show[0], self.show[1]]
    }
}

struct Animation {
    start: Instant,
    duration: Duration,
    from: [f32; 4],
    to: [f32; 4],
}

/// A file's modification time and length: when they change, another app
/// edited it.
type Stamp = (SystemTime, u64);

fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

/// The search field's text is one line: moving in it needs no real layout.
struct OneLine<'a>(&'a str);

impl kolada_editor::Layout for OneLine<'_> {
    fn visual_line(&self, _: usize) -> (usize, usize) {
        (0, self.0.len())
    }

    fn x_of(&self, offset: usize) -> f32 {
        offset as f32
    }

    fn above(&self, _: usize, _: f32) -> Option<usize> {
        None
    }

    fn below(&self, _: usize, _: f32) -> Option<usize> {
        None
    }

    fn rtl(&self, _: usize) -> bool {
        scene::is_rtl(self.0)
    }
}

/// A related note in the margin, and its springs: it drifts in from the
/// right, follows the caret's line up and down, and fades out when it no
/// longer relates.
struct ChipState {
    rel: Related,
    /// Top edge (NaN until first placed) and its velocity; how far it still
    /// has to drift in, and that velocity; opacity.
    y: f32,
    vy: f32,
    dx: f32,
    vdx: f32,
    alpha: f32,
    /// No longer related: fading out where it is.
    leaving: bool,
}

/// What only the running app shows; the first frame has none of it.
pub struct Live<'a> {
    pub search: Option<SearchView<'a>>,
    pub chips: Vec<Chip<'a>>,
    pub dragged: Option<(usize, f32, f32)>,
    pub drop_at: Option<usize>,
    pub page_fade: f32,
    pub highlights_open: bool,
    pub peek: Option<scene::Peek<'a>>,
    pub whisper: Option<scene::Whisper<'a>>,
    pub popup: Option<scene::PopupView<'a>>,
    pub live_name: Option<&'a str>,
    pub menu: Option<scene::MenuView<'a>>,
    pub menu_row: Option<usize>,
    pub menu_anchor: Option<Target>,
    pub trash: Option<&'a [scene::TrashEntry]>,
    pub scrollbar: Option<ScrollView>,
    pub dragging: Option<Target>,
    pub opened_in: Option<f32>,
    /// What a keystroke costs, from its message to its frame (median ms).
    pub keystroke: Option<f32>,
    /// A note in a window of its own (see `scene::Input::compact`).
    pub compact: bool,
    pub find: Option<scene::FindView<'a>>,
    pub matches: &'a [(usize, usize)],
    pub ticks: &'a [f32],
    pub suggestions: Option<scene::Suggestions<'a>>,
    pub link_field: Option<scene::LinkFieldView<'a>>,
    pub link_card: Option<(&'a str, f32, f32)>,
    pub landmarks: &'a [scene::Landmark<'a>],
    pub carried: Option<(&'a str, f32, f32)>,
    pub arriving: Option<scene::Arriving<'a>>,
    pub marked: Option<scene::Composed>,
    pub spelling: Option<&'a crate::spell::Marks>,
    pub page_date: Option<&'a str>,
    pub row_date: Option<(usize, &'a str)>,
    pub page_gist: Option<(&'a str, f32)>,
    pub wash: Option<(&'a [(usize, usize)], f32)>,
    pub images: Option<&'a RefCell<dyn crate::images::PictureSource>>,
    pub picture_resize: Option<(usize, f32)>,
    pub social: Option<crate::social::SocialView>,
    pub scrub: Option<kolada_ui::scrub::ScrubView<'a>>,
    pub due_now: usize,
    pub agenda: Option<kolada_ui::agenda::AgendaView<'a>>,
    /// This copy updates itself (the options' switch), and the version
    /// ready for the pill (see `updates`).
    pub updater: bool,
    pub update: Option<&'a str>,
    /// The share card under Send (see `link`).
    pub share: Option<kolada_ui::share::ShareView<'a>>,
    pub model_card: Option<kolada_ui::modelcard::ModelCardView<'a>>,
    /// The selection bar and the Aa card, as far as they have come in, and
    /// the button whose name shows.
    pub selection_bar: Option<f32>,
    pub reading: Option<f32>,
    pub tooltip: Option<Target>,
}

impl Live<'_> {
    pub fn none() -> Live<'static> {
        Live {
            search: None,
            chips: Vec::new(),
            dragged: None,
            drop_at: None,
            page_fade: 1.0,
            highlights_open: false,
            peek: None,
            whisper: None,
            popup: None,
            live_name: None,
            menu: None,
            menu_row: None,
            menu_anchor: None,
            trash: None,
            scrollbar: None,
            dragging: None,
            opened_in: None,
            keystroke: None,
            compact: false,
            find: None,
            matches: &[],
            ticks: &[],
            suggestions: None,
            link_field: None,
            link_card: None,
            landmarks: &[],
            carried: None,
            arriving: None,
            marked: None,
            spelling: None,
            page_date: None,
            row_date: None,
            page_gist: None,
            wash: None,
            images: None,
            picture_resize: None,
            social: None,
            scrub: None,
            due_now: 0,
            agenda: None,
            updater: false,
            update: None,
            share: None,
            model_card: None,
            selection_bar: None,
            reading: None,
            tooltip: None,
        }
    }
}

/// A note open over the page (see `scene::popup`): read, scroll and edit it
/// without leaving the note being written.
struct Popup {
    path: PathBuf,
    name: String,
    editor: Editor,
    format: Format,
    saved_version: u64,
    /// Its file as last read or saved (another app's edits change it).
    disk: Option<Stamp>,
    /// The name line added over a text that does not start with it (see
    /// `title`), and the title its file is named after.
    added: Option<String>,
    named: String,
    /// How far its text is scrolled; the height it opened beside; when.
    scroll: f32,
    anchor: f32,
    opened: Instant,
    /// Where it was dragged to, and where the pointer holds it while it is
    /// dragged (DIPs from its offset).
    offset: (f32, f32),
    held: Option<(f32, f32)>,
    /// Where the last frame put it.
    drawn: Option<scene::PopupDrawn>,
    /// Bring its caret into view in the next frame; a mouse selection is
    /// being dragged.
    reveal: bool,
    selecting: bool,
}

/// A peek at a chip's note: which chip, since when, and what it shows.
struct PeekState {
    chip: usize,
    since: Instant,
    title: String,
    body: String,
}

/// A note's opening, as a peek shows it: its first lines as they read,
/// without frontmatter or Markdown marks.
fn opening(text: &str) -> String {
    let lines = kolada_markdown::lines(text).filter(|l| !matches!(l.block, Block::Frontmatter | Block::Fence));
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        let plain = md::plain(&text[l.start..l.end]);
        let plain = plain.trim();
        if plain.is_empty() {
            continue;
        }
        let cut: String = plain.chars().take(160).collect();
        out.push(if plain.chars().count() > 160 { format!("{cut}\u{2026}") } else { cut });
        if out.len() == 7 {
            break;
        }
    }
    out.join("\n")
}

/// Finding a note from the side menu.
struct SearchState {
    query: Editor,
    hits: Vec<Hit>,
    /// The result Enter opens, and the first shown (the list scrolls to
    /// keep it in view).
    selected: usize,
    first: usize,
    /// With `>`, commands instead (see `palette`): all there are while the
    /// palette is open, those shown, and what they do.
    commands: Option<Vec<palette::Command>>,
    shown: Vec<scene::MenuItem>,
    runs: Vec<palette::Run>,
}

/// The smallest the window gets, DIPs.
const MIN_WINDOW: (f32, f32) = (360.0, 320.0);

/// Results shown at most.
const SEARCH_LIMIT: usize = 40;

/// Reads every note of the vault on a low-priority worker, for search, and
/// posts them to the window.
pub fn start_index() {
    with_app(|a| a.start_index());
}

/// Watches the vault for other apps' changes (after the first frame).
pub fn start_watch() {
    with_app(|a| a.watch_vault());
}

/// A first run (after the first frame): the welcome note and its picture go
/// into a new Documents\Kolada on a worker, then open. A folder that exists
/// is never written to (see `kolada_core::welcome`).
pub fn start_welcome() {
    with_app(|a| {
        if !a.session.vault.is_empty() {
            return;
        }
        let Some(vault) = kolada_core::default_vault_dir() else { return };
        let hwnd = crate::Handoff(a.hwnd);
        std::thread::spawn(move || {
            let hwnd = hwnd;
            let Ok(Some(note)) = kolada_core::welcome::seed(&vault, "Ctrl") else { return };
            let note = Box::into_raw(Box::new(note));
            let posted = unsafe { PostMessageW(Some(hwnd.0), window::WM_APP_WELCOME, WPARAM(0), LPARAM(note as isize)) };
            if posted.is_err() {
                drop(unsafe { Box::from_raw(note) });
            }
        });
    });
}

/// After the first frame: the replaced program goes if this start followed
/// an update (`updated`: the pill was clicked, so the note says so), and
/// the daily check starts where this copy updates itself (see `updates`).
pub fn start_updates(updated: bool) {
    crate::updates::tidy();
    crate::setup::work::tidy_old_install();
    with_app(|a| {
        if updated {
            a.whisper(format!("Updated to Kolada {}", env!("CARGO_PKG_VERSION")));
        }
        a.updater = !a.compact && crate::updates::offered();
        if a.updater {
            crate::updates::watch(a.hwnd, a.session.updates == 0);
        }
    });
}

/// Takes Win+Alt+N for a quick note (after the first frame).
pub fn start_hotkey() {
    with_app(|a| a.update_hotkey(false));
}

/// Puts the icon in the tray (after the first frame).
pub fn start_tray() {
    with_app(|a| a.update_tray());
}

/// Opens search at once (launched from the taskbar menu's Search).
pub fn search_now() {
    with_app(|a| a.open_search());
}

pub use share::is_export;

/// Files dragged over the window (see `drag`), at client pixels: true if
/// any can be taken.
pub fn drag_enter(files: Vec<PathBuf>, x: i32, y: i32) -> bool {
    with_app(|a| a.drag_enter(files, x, y)).unwrap_or(false)
}

pub fn drag_over(x: i32, y: i32) -> bool {
    with_app(|a| a.drag_over(x, y)).unwrap_or(false)
}

pub fn drag_leave() {
    with_app(|a| a.drag_leave());
}

pub fn drag_drop(files: Vec<PathBuf>, x: i32, y: i32) -> bool {
    with_app(|a| a.drag_drop(files, x, y)).unwrap_or(false)
}

/// Imports another app's export, or a document, given on the command line.
pub fn import_now(export: &Path) {
    with_app(|a| a.import_arg(export));
}

/// Starts checking spelling, on a worker (after the first frame).
pub fn start_spell() {
    with_app(|a| {
        a.spell = Some(crate::spell::Spell::start(a.hwnd));
        a.request_frame();
    });
}

/// Starts the writing model a moment later (after the first frame; see
/// `assist`).
pub fn start_assist() {
    with_app(|a| {
        a.assist_soon();
        a.offer_model();
    });
}

/// Writes the last frame to `path` when the window closes (`--dump-frame`).
pub fn dump_on_close(path: Option<PathBuf>) {
    with_app(|a| a.dump_on_close = path);
}

pub struct App {
    pub hwnd: HWND,
    pub surface: Surface,
    pub eng: scene::Eng,
    pub fonts: Fonts,
    pub session: Session,
    pub editor: Editor,
    pub session_path: Option<PathBuf>,
    /// Hit regions of the last frame, in DIPs.
    pub hits: Vec<(Rect, Target)>,
    pub ui: Ui,
    /// The note's file format, the editor version last written to it, and
    /// the file's stamp then.
    format: Format,
    saved_version: u64,
    disk: Option<Stamp>,
    /// The name line added over a note whose text does not start with its
    /// name (see `title`), and the title its file is named after: when the
    /// first line reads otherwise, the file follows.
    added: Option<String>,
    named: String,
    /// The last frame's note column and visible paragraphs, and the editor
    /// version they show.
    column: Rect,
    lines: Vec<Line>,
    drawn_version: u64,
    /// Estimated heights and the end of the note, for its scroll bar (see
    /// `scroll`); a scroll bar being dragged.
    heights: RefCell<scroll::Heights>,
    scroll_drag: Option<scroll::Drag>,
    /// A menu open over everything, and the trash while it is showing.
    menu: Option<menu::Menu>,
    trash: Option<trash::TrashList>,
    /// Notes deleted since the text last changed: Ctrl+Z brings them back,
    /// the last first.
    undo_delete: Vec<(kolada_core::trash::Trashed, u64)>,
    /// How long this launch took to show its first frame, ms.
    opened_in: Option<f32>,
    /// This copy updates itself (not the Store's build, nor a note's own
    /// window), and the version downloaded and checked, for the pill.
    updater: bool,
    update: Option<String>,
    /// When the key being handled arrived, and what the last keys cost, from
    /// their message to their frame drawn (ms).
    key_arrived: Option<Instant>,
    key_ms: Vec<f32>,
    /// When the last character was typed.
    last_key: Option<Instant>,
    /// When the page (or the popup) last scrolled: its scroll bar shows for
    /// a moment after. The file tree's scroll bar track in the last frame.
    scrolled_at: Option<Instant>,
    tree_track: Option<Rect>,
    /// This is a note's own window (`--window`), beside the main one.
    compact: bool,
    /// The find bar; notes suggested for the `[[link]]` being typed, and
    /// where a link was put away with Esc.
    find: Option<find::Find>,
    suggest: Option<suggest::Suggest>,
    suggest_dismissed: Option<usize>,
    /// The link field (Ctrl+K), and the link the pointer rests on.
    link: Option<link::LinkState>,
    link_hover: Option<link::LinkHover>,
    /// A note row pressed in the file tree (and where, in pixels), and the
    /// pointer while it is carried (DIPs).
    row_press: Option<(usize, i32, i32)>,
    row_drag: Option<(f32, f32)>,
    /// The icon is in the tray; the app is quitting for real (not hiding
    /// in the tray).
    tray_shown: bool,
    quitting: bool,
    /// Win+Alt+N is this app's (see `tray::update_hotkey`).
    hotkey_held: bool,
    /// A picture being resized by its edge.
    resizing: Option<pictures::Resizing>,
    /// The social preview's platform while it is open, how far it is
    /// scrolled, and how far it can be (from the last frame).
    social: Option<kolada_core::social::Platform>,
    social_scroll: f32,
    social_scroll_max: f32,
    /// Where the preview was dragged to (DIPs from its place), and where
    /// the pointer holds it while it is dragged.
    social_offset: (f32, f32),
    social_grip: Option<(f32, f32)>,
    /// The side menu's edge being dragged: where the pointer took it and
    /// the menu's width then, DIPs.
    sidebar_grip: Option<(f32, f32)>,
    /// Notes left for another (to go back to, with where the view and the
    /// caret were), notes gone back from (to go forward to), and whether a
    /// step back or forward is under way.
    back: Vec<Visit>,
    ahead: Vec<Visit>,
    stepping: bool,
    /// The note's word count, for the text version it is for.
    words: std::cell::Cell<(u64, usize)>,
    /// Pictures in notes, found and decoded on a worker.
    images: RefCell<crate::images::Images>,
    /// When the note's file was made and last written; when the file tree
    /// row under the pointer was edited.
    note_times: (Option<SystemTime>, Option<SystemTime>),
    /// The dates said for the page, kept for a second (not said every frame).
    dates: kolada_ui::when::DateCache,
    row_date: Option<(usize, String)>,
    /// Spelling, checked on a worker after the first frame; the offset a
    /// text menu is opening for.
    spell: Option<crate::spell::Spell>,
    menu_word: Option<usize>,
    /// The writing model: its process, and what it was asked about the
    /// open note.
    writing: assist::Writing,
    /// The folder a fresh page saves into, when not the vault's top.
    fresh_dir: Option<PathBuf>,
    /// The caret glides: where it belongs in the last frame, how far from
    /// there it was drawn, and how fast that gap closes.
    caret_target: Option<Rect>,
    caret_offset: (f32, f32),
    caret_velocity: (f32, f32),
    caret_gliding: bool,
    /// The caret moved since the last frame; with `glide`, it glides there.
    caret_jumped: bool,
    glide: bool,
    /// Scroll the caret into view in the next frame.
    reveal_caret: bool,
    /// Scroll distance still to travel, DIPs (positive is down).
    scroll_pending: f32,
    /// A mouse selection is being dragged.
    selecting: bool,
    /// The last click: message time, position in pixels, and 1 to 3 for
    /// single, double and triple clicks.
    last_click: (u32, i32, i32, u32),
    /// First half of a character outside the Basic Multilingual Plane.
    high_surrogate: Option<u16>,
    /// The side menu's search while it is open, and every note to search.
    search: Option<SearchState>,
    index: Vec<Indexed>,
    /// Wheel movement over the file tree not yet a whole row, and how many
    /// rows fit in the last frame.
    tree_scroll: f32,
    tree_rows: usize,
    /// Search results or commands that fit, for scrolling with the keys.
    result_rows: usize,
    /// The system's light or dark setting, once read.
    system_dark: Option<bool>,
    dump_on_close: Option<PathBuf>,
    /// Magnetic memory: what every note is about, the related notes in the
    /// margin, and whether any of them is still moving.
    corpus: Option<Corpus>,
    chips: Vec<ChipState>,
    chips_moving: bool,
    /// The note version, paragraph and note the margin was found for.
    related_key: (u64, usize, String),
    /// A chip pressed (index, and where, in pixels); the pointer while it is
    /// dragged (DIPs); and where in the text it would land as a link.
    chip_press: Option<(usize, i32, i32)>,
    chip_drag: Option<(f32, f32)>,
    drop_at: Option<usize>,
    /// Files dragged in from outside, before they are let go (see
    /// `arrive`), and whether its card is moving.
    arrival: Option<arrive::Arrival>,
    arrival_moving: bool,
    /// The note's past, open over the page (see `scrub`), and whether it
    /// is moving.
    scrub: Option<scrub::Scrub>,
    scrub_moving: bool,
    /// What is due, and the Today sheet (see `agenda`).
    agenda: agenda::State,
    /// The share card under Send (see `link`).
    share_card: Option<share_card::Open>,
    /// The first run's writing model card (see `model_card`).
    model_card: Option<model_card::Shown>,
    /// The note whose past was last kept, and when (ms since the epoch):
    /// while typing goes on, a version at most every half minute.
    kept: (String, u64),
    /// The part of the text being composed that the input method is
    /// converting (bytes into it), for the page to underline (see `compose`).
    ime_target: Option<(usize, usize)>,
    /// When the note on screen was opened, while it fades in.
    page_opened: Option<Instant>,
    /// A generated page is showing (the highlights): it can be read and its
    /// links followed, but not edited or saved.
    read_only: bool,
    /// A peek at a chip's note, and a chip the pointer rests on, waiting to
    /// peek.
    peek: Option<PeekState>,
    peek_pending: Option<usize>,
    /// "Saved as ...", and since when.
    whisper: Option<(String, Instant)>,
    /// The pointer left the window through its left edge, overshooting the
    /// side menu in focus mode: the menu stays out until it comes back.
    exited_left: bool,
    /// A note open over the page.
    popup: Option<Popup>,
    animation: Option<Animation>,
    /// Collapsed panels currently revealed by hovering near them.
    revealed: [bool; 2],
    /// Too narrow to dock the side menu: it stays tucked away, floating
    /// over the note when revealed, whatever the session says.
    narrow: bool,
    /// How far the options can scroll, from the last frame.
    options_scroll_max: f32,
    /// A panel collapsed from inside stays hidden until the pointer leaves
    /// it, instead of being revealed again right under the pointer.
    held_back: [bool; 2],
    /// Pending reveal and hide timers, so repeated moves don't restart them.
    reveal_pending: [bool; 2],
    hide_pending: [bool; 2],
    /// A frame is due at the next display refresh.
    dirty: bool,
    /// Where the collapsed pills were in the last frame, and their magnets'
    /// spring velocities (along, pull). While a magnet moves, frames keep
    /// coming.
    pills: [Option<Rect>; 2],
    waves: [Wave; 2],
    magnets_moving: bool,
    last_frame: Instant,
    /// Target under the mouse when the button went down.
    pressed: Option<Target>,
    tracking: bool,
    /// The toolbar's Aa card, a button's name, the selection bar.
    bars: bars::Bars,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn install(app: App) {
    APP.with(|a| *a.borrow_mut() = Some(app));
}

/// The process is ending, everything saved: the app is let go undropped.
/// Dropped as Windows tears the process down, the spell checker's COM
/// objects make a call that never returns, and the process stays, holding
/// the instance.
pub fn leave() {
    APP.with(|a| {
        if let Ok(mut a) = a.try_borrow_mut() {
            std::mem::forget(a.take());
        }
    });
}

/// Runs `f` on the app if it exists and is not already borrowed (window
/// messages can re-enter while the app is handling one).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok()?.as_mut().map(f))
}

/// True when a frame is due: something changed, or something is moving.
pub fn needs_frame() -> bool {
    with_app(|a| {
        a.dirty
            || a.animation.is_some()
            || a.magnets_moving
            || a.caret_gliding
            || a.scroll_pending != 0.0
            || a.chips_moving
            || a.arrival_moving
            || a.agenda_moving()
            || a.scrub_moving
            || a.page_opened.is_some()
            || a.peek.as_ref().is_some_and(|p| p.since.elapsed() < PEEK_FADE)
            || a.popup.as_ref().is_some_and(|p| p.opened.elapsed() < POPUP_FADE)
            || a.menu.as_ref().is_some_and(|m| m.opened.elapsed() < menu::FADE)
            || a.share_card_fading()
            || a.model_card_moving()
            || a.bars.moving()
            || a.scrolled_at.is_some_and(|t| t.elapsed() < scroll::SHOWN + scroll::FADE)
            || a.images.borrow().fading()
            || a.assist_moving()
            || a.whisper.as_ref().is_some_and(|(_, t)| {
                let e = t.elapsed();
                e < WHISPER_IN || e > WHISPER - WHISPER_OUT
            })
    })
        .unwrap_or(false)
}

/// Advances animations and draws, once per display refresh.
pub fn frame() {
    with_app(|a| a.frame());
}

/// A note as it opens: its text with the title line (see `title`), its
/// file's format, and the name line added over the text, if any.
pub type Opened = (String, Format, Option<String>);

/// Reads the session's note, or nothing for a fresh page.
pub fn read_note(session: &Session) -> Opened {
    if session.vault.is_empty() || session.note.is_empty() {
        return Default::default();
    }
    read_file(&Path::new(&session.vault).join(&session.note)).unwrap_or_default()
}

fn read_file(path: &Path) -> Option<Opened> {
    let (file, format) = note::load(path).ok()?;
    let (text, added) = title::open(&stem(path), file);
    Some((text, format, added))
}

/// A note's name: its file's name without `.md`.
fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What goes in the file for `text`: without the name line `added` over it
/// while that line is untouched. Once edited, the line is part of the note
/// and `added` is cleared.
fn file_text<'a>(text: &'a str, added: &mut Option<String>) -> std::borrow::Cow<'a, str> {
    match added.as_deref().map(|line| title::file_text(text, line)) {
        Some(Some(file)) => file.into(),
        Some(None) => {
            *added = None;
            text.into()
        }
        None => text.into(),
    }
}

/// A note as it was left: its file, the paragraph at the top of the view
/// and how far that was scrolled, and the caret.
#[derive(Clone, Debug)]
struct Visit {
    path: PathBuf,
    scroll: (u64, f32),
    caret: usize,
}

/// Steps back kept at most.
const HISTORY: usize = 64;

use kolada_ui::when::{count_words, thousands, words_phrase};

/// What a link at some place in a note leads to.
#[derive(Debug, PartialEq)]
pub enum Link {
    /// A note, by name (`[[Note]]`, or `[x](Note.md)`).
    Note(String),
    /// A web page or an email (`[x](https://…)`, `https://…`, `www.…`).
    Web(String),
    /// A tag, searched for.
    Tag(String),
}

/// `%20`-style escapes in a link's address, decoded.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Copies `editor`'s selection: plain text without marks for other apps,
/// HTML for those that keep formatting, and the Markdown itself for
/// Kolada (see `Editor::copied`).
fn copy_from(hwnd: HWND, editor: &Editor) {
    // With "Show Markdown" on (the editor not clean), the plain text is the
    // Markdown as it is.
    if let Some(copied) = editor.copied(!editor.is_clean()) {
        clipboard::set_copied(hwnd, &copied);
    }
}

/// The link at byte `at` of `text`, if there is one.
pub fn link_at(text: &str, at: usize) -> Option<Link> {
    let at = at.min(text.len());
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = md::lines(&text[start..]).next()?;
    let line = md::Line { start: line.start + start, end: line.end + start, ..line };
    let spans = md::line_spans(text, &line);
    let span = spans
        .iter()
        .rfind(|s| matches!(s.style, Style::WikiLink | Style::Link | Style::Url | Style::Tag) && s.start <= at && at <= s.end)?;
    let whole = &text[span.start..span.end];
    Some(match span.style {
        Style::WikiLink => {
            let inner = whole.trim_start_matches('!').trim_start_matches("[[").trim_end_matches("]]");
            // [[Note#Heading|shown]]: the note is before any # or |.
            Link::Note(inner.split(['|', '#', '^']).next().unwrap_or("").trim().to_string())
        }
        Style::Url if whole.to_ascii_lowercase().starts_with("www.") => Link::Web(format!("https://{whole}")),
        Style::Url => Link::Web(whole.to_string()),
        Style::Tag => Link::Tag(whole.to_string()),
        _ => {
            let address = text[(span.text_end + 2).min(span.end - 1)..span.end - 1].trim();
            let lower = address.to_ascii_lowercase();
            if ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p)) {
                Link::Web(address.to_string())
            } else if lower.starts_with("www.") {
                Link::Web(format!("https://{address}"))
            } else {
                let file = unescape(address);
                let name = file.rsplit(['/', '\\']).next().unwrap_or(&file);
                Link::Note(name.strip_suffix(".md").unwrap_or(name).to_string())
            }
        }
    })
}

/// Where writing starts in a note just opened: the line under its title
/// (typing right away adds to the note rather than renaming it).
fn body_start(e: &Editor) -> usize {
    body_start_of(e.text(), e.outline())
}

/// The same for a text and its lines, before there is an editor.
pub fn body_start_of(text: &str, lines: &[md::Line]) -> usize {
    match crate::note::title_line(text, lines) {
        Some(t) if t + 1 < lines.len() => lines[t + 1].content().min(lines[t + 1].end),
        Some(t) => lines[t].end,
        None => 0,
    }
}

/// Whether the caret is on the title line.
fn on_title(e: &Editor) -> bool {
    let lines = e.outline();
    let head = e.selection().head;
    crate::note::title_line(e.text(), lines).is_some_and(|t| lines[t].start <= head && head <= lines[t].end)
}

fn vault_name(vault: &str) -> &str {
    vault.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or(vault)
}

/// Lays out the view for a client area of `width` x `height` pixels at
/// `dpi`. Shared by the first frame, which is laid out on a worker before
/// the window exists, and every later frame. `lines` are the text's
/// Markdown lines (see `kolada_markdown::lines`).
#[allow(clippy::too_many_arguments)]
pub fn build_scene(
    eng: &scene::Eng,
    fonts: &Fonts,
    s: &Session,
    text: &str,
    lines: &[md::Line],
    selection: (usize, usize),
    (width, height, dpi): (u32, u32, u32),
    ui: &Ui,
    live: Live,
) -> Result<Scene> {
    let scale = dpi as f32 / 96.0;
    let active = s.note.strip_suffix(".md").unwrap_or(&s.note);
    let tools = if live.highlights_open { Vec::new() } else { kolada_ui::edit::active_tools(text, lines, selection.1) };
    let input = scene::Input {
        vault_name: vault_name(&s.vault),
        active_note: active,
        live_name: live.live_name,
        text,
        sidebar: &s.sidebar,
        scroll_line: s.scroll_line as usize,
        scroll_px: s.scroll_px,
        selection,
        lines,
        active_tools: &tools,
        tree_top: ui.tree_top,
        search: live.search,
        chips: &live.chips,
        dragged: live.dragged,
        drop_at: live.drop_at,
        page_fade: live.page_fade,
        highlights_open: live.highlights_open,
        peek: live.peek,
        whisper: live.whisper,
        popup: live.popup,
        options_open: ui.options_open,
        options_scroll: ui.options_scroll,
        options: [s.theme, s.text_size, s.line_width, s.font, s.launch, s.related, s.spell, s.tray, s.hotkey, s.window_mode, assist::option(s), s.remind, s.updates, s.show_marks as u8],
        vault_path: &s.vault,
        opened_in: live.opened_in,
        keystroke: live.keystroke,
        menu: live.menu,
        menu_row: live.menu_row,
        menu_anchor: live.menu_anchor,
        trash: live.trash,
        scrollbar: live.scrollbar,
        dragging: live.dragging,
        sidebar_width: s.sidebar_width as f32,
        no_margin: s.related == 1 || s.minimal,
        compact: live.compact,
        find: live.find,
        matches: live.matches,
        ticks: live.ticks,
        suggestions: live.suggestions,
        link_field: live.link_field,
        link_card: live.link_card,
        landmarks: live.landmarks,
        carried: live.carried,
        arriving: live.arriving,
        marked: live.marked,
        spelling: live.spelling,
        page_date: live.page_date,
        row_date: live.row_date,
        page_gist: live.page_gist,
        wash: live.wash,
        images: live.images,
        picture_resize: live.picture_resize,
        social_drawer: s.social == 2,
        social: live.social,
        scrub: live.scrub,
        scale,
        sidebar_dock: ui.dock[0],
        sidebar_show: ui.show[0],
        toolbar_dock: ui.dock[1],
        toolbar_show: ui.show[1],
        hover: ui.hover,
        sidebar_wave: &ui.wave[0],
        toolbar_wave: &ui.wave[1],
        maximized: ui.maximized,
        native_caption: false,
        platform: scene::Platform::Windows,
        today: Some(daily::today()),
        due_now: live.due_now,
        recent: &s.recent,
        favorites: &s.favorites,
        folded: s.folded,
        agenda: live.agenda,
        updater: live.updater,
        update: live.update,
        show_marks: s.show_marks,
        share: live.share,
        model_card: live.model_card,
        selection_bar: live.selection_bar,
        reading: live.reading,
        tooltip: live.tooltip,
    };
    scene::build(eng, fonts, &input, width as f32 / scale, height as f32 / scale)
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hwnd: HWND,
        surface: Surface,
        eng: scene::Eng,
        fonts: Fonts,
        session: Session,
        (text, format, added): Opened,
        session_path: Option<PathBuf>,
        hits: Vec<(Rect, Target)>,
        column: Rect,
        compact: bool,
        mut images: crate::images::Images,
    ) -> App {
        images.start(hwnd);
        let (w, _, dpi) = window::view(hwnd);
        let width = w as f32 * 96.0 / dpi as f32;
        let ui = Ui::resting(&session, width);
        let mut editor = Editor::new(text);
        editor.set_clean(!session.show_marks);
        editor.set_snippets(daily::snippets());
        editor.set_selection(Selection::caret(session.caret as usize));
        let disk = if session.note.is_empty() { None } else { stamp(&Path::new(&session.vault).join(&session.note)) };
        let mut app = App {
            hwnd,
            surface,
            eng,
            fonts,
            session,
            saved_version: editor.version(),
            editor,
            session_path,
            hits,
            ui,
            format,
            disk,
            added,
            named: String::new(),
            column,
            lines: Vec::new(),
            heights: RefCell::new(scroll::Heights::default()),
            scroll_drag: None,
            menu: None,
            trash: None,
            undo_delete: Vec::new(),
            opened_in: crate::trace::at("visible").map(|ms| ms as f32),
            updater: false,
            update: None,
            key_arrived: None,
            key_ms: Vec::new(),
            last_key: None,
            fresh_dir: None,
            scrolled_at: None,
            tree_track: None,
            compact,
            find: None,
            suggest: None,
            suggest_dismissed: None,
            link: None,
            link_hover: None,
            row_press: None,
            row_drag: None,
            spell: None,
            menu_word: None,
            writing: assist::Writing::new(),
            note_times: (None, None),
            dates: Default::default(),
            row_date: None,
            tray_shown: false,
            hotkey_held: false,
            resizing: None,
            social: None,
            social_scroll: 0.0,
            social_scroll_max: 0.0,
            social_offset: (0.0, 0.0),
            social_grip: None,
            sidebar_grip: None,
            quitting: false,
            back: Vec::new(),
            ahead: Vec::new(),
            stepping: false,
            words: std::cell::Cell::new((u64::MAX, 0)),
            images: RefCell::new(images),
            // Not drawn by the app yet: the first frame's paragraphs stayed
            // on its worker thread.
            drawn_version: u64::MAX,
            caret_target: None,
            caret_offset: (0.0, 0.0),
            caret_velocity: (0.0, 0.0),
            caret_gliding: false,
            caret_jumped: false,
            glide: false,
            reveal_caret: false,
            scroll_pending: 0.0,
            selecting: false,
            last_click: (0, 0, 0, 0),
            high_surrogate: None,
            search: None,
            index: Vec::new(),
            tree_scroll: 0.0,
            tree_rows: 0,
            result_rows: 0,
            system_dark: None,
            dump_on_close: None,
            corpus: None,
            chips: Vec::new(),
            chips_moving: false,
            related_key: (u64::MAX, 0, String::new()),
            chip_press: None,
            chip_drag: None,
            drop_at: None,
            arrival: None,
            arrival_moving: false,
            scrub: None,
            scrub_moving: false,
            agenda: agenda::State::default(),
            share_card: None,
            model_card: None,
            kept: (String::new(), 0),
            ime_target: None,
            page_opened: None,
            read_only: false,
            peek: None,
            peek_pending: None,
            whisper: None,
            exited_left: false,
            popup: None,
            animation: None,
            revealed: [false; 2],
            narrow: !chrome::sidebar_fits(width),
            options_scroll_max: 0.0,
            held_back: [false; 2],
            reveal_pending: [false; 2],
            hide_pending: [false; 2],
            dirty: false,
            pills: [None; 2],
            waves: [Wave::default(), Wave::default()],
            magnets_moving: false,
            last_frame: Instant::now(),
            pressed: None,
            tracking: false,
            bars: bars::Bars::default(),
        };
        app.named = app.editor.title();
        app.read_times();
        app.remember_untouched();
        app
    }

    fn scene(&self, view: (u32, u32, u32)) -> Result<Scene> {
        let s = self.editor.selection();
        let e = &self.editor;
        let search = self.search.as_ref().map(|s| SearchView {
            query: s.query.text(),
            caret: s.query.selection().head,
            hits: &s.hits,
            selected: s.selected,
            first: s.first,
            commands: s.commands.is_some().then_some(s.shown.as_slice()),
        });
        let chips = self
            .chips
            .iter()
            .filter(|c| !c.y.is_nan())
            .map(|c| Chip {
                name: &c.rel.name,
                detail: &c.rel.detail,
                kind: c.rel.kind,
                // Scores of 0.5 and up are about as related as notes get.
                strength: (c.rel.score * 2.0).min(1.0),
                y: c.y,
                dx: c.dx,
                alpha: c.alpha,
            })
            .collect();
        let dragged = match (self.chip_press, self.chip_drag) {
            (Some((i, _, _)), Some((x, y))) => Some((i, x, y)),
            _ => None,
        };
        let page_fade = self.page_opened.map_or(1.0, |t| chrome::ease(t.elapsed().as_secs_f32() / PAGE_FADE.as_secs_f32()));
        let peek = self.peek.as_ref().filter(|_| dragged.is_none()).map(|p| scene::Peek {
            chip: p.chip,
            title: &p.title,
            body: &p.body,
            alpha: chrome::ease(p.since.elapsed().as_secs_f32() / PEEK_FADE.as_secs_f32()),
        });
        let whisper = self.whisper.as_ref().map(|(text, since)| {
            let t = since.elapsed();
            let alpha = if t < WHISPER_IN {
                t.as_secs_f32() / WHISPER_IN.as_secs_f32()
            } else if t > WHISPER - WHISPER_OUT {
                (WHISPER.saturating_sub(t)).as_secs_f32() / WHISPER_OUT.as_secs_f32()
            } else {
                1.0
            };
            scene::Whisper { text, alpha }
        });
        let popup_note = self.popup.as_ref().and_then(|p| {
            let rel = vault::relative(Path::new(&self.session.vault), &p.path)?;
            Some(rel.strip_suffix(".md").unwrap_or(&rel).to_string())
        });
        let popup = self.popup.as_ref().map(|p| {
            let s = p.editor.selection();
            scene::PopupView {
                marked: p.editor.marked().map(|(a, b)| ((a, b), self.ime_target.map(|(x, y)| (a + x, (a + y).min(b))))),
                name: &p.name,
                text: p.editor.text(),
                lines: p.editor.outline(),
                selection: (s.anchor, s.head),
                scroll: p.scroll,
                anchor: p.anchor,
                alpha: chrome::ease(p.opened.elapsed().as_secs_f32() / POPUP_FADE.as_secs_f32()),
                offset: p.offset,
                hover: self.ui.hover,
                scrolling: self.scroll_drag.is_some_and(|d| d.bar == Target::PopupScrollbar),
                bar_alpha: self.popup_bar_alpha(),
                spelling: self.spell.as_ref().filter(|_| self.session.spell == 0).map(|s| &s.marks),
                note: popup_note.as_deref(),
            }
        });
        // The open note's row in the tree reads as its title does, before
        // the file follows.
        let title = self.editor.title();
        let live_name = (!self.session.note.is_empty() && title != self.named && !title.is_empty()).then(|| note::file_stem_for(&title));
        // The headings, for the scroll bar's map; the one whose section is at
        // the top of the view is here.
        let top = self.session.scroll_line as usize;
        let scrollbar = self.scroll_view(view);
        // The map shows with the bar: found only then, not at every key.
        let mapped = scrollbar.is_some_and(|b| b.alpha > 0.0) || matches!(self.ui.hover, Some(Target::Heading(_)));
        let none = Vec::new();
        let kept;
        let headings: &Vec<(String, u8, usize, f32)> = if mapped {
            kept = self.headings_for_map();
            &kept
        } else {
            &none
        };
        let here = headings.iter().rposition(|h| h.2 <= top);
        let landmarks: Vec<scene::Landmark> =
            headings.iter().enumerate().map(|(i, h)| scene::Landmark { at: h.3, text: &h.0, level: h.1, here: Some(i) == here }).collect();
        let page_date = if self.read_only || self.session.note.is_empty() {
            None
        } else {
            let long = self.editor.text().len() > WORDS_AT_ONCE;
            let typing = self.last_key.is_some_and(|t| t.elapsed() < WORDS_PAUSE);
            let words = match self.words.get() {
                (v, n) if v == self.editor.version() => n,
                // A long note is counted again once typing pauses (the
                // timer draws it then); until, the count it had.
                (v, n) if long && typing && v != u64::MAX => n,
                _ => {
                    let n = count_words(self.editor.text());
                    self.words.set((self.editor.version(), n));
                    n
                }
            };
            let words = words_phrase(words);
            self.dates.get(self.note_times, || crate::when::page_dates(self.note_times.0, self.note_times.1)).map(|d| d + &words)
        };
        let menu = self.menu.as_ref().map(|m| scene::MenuView {
            x: m.x,
            y: m.y,
            align_right: m.anchor.is_some(),
            items: &m.items,
            alpha: chrome::ease(m.opened.elapsed().as_secs_f32() / menu::FADE.as_secs_f32()),
        });
        let live = Live {
            search,
            chips,
            dragged,
            drop_at: self.drop_at,
            page_fade,
            highlights_open: self.read_only,
            peek,
            whisper,
            popup,
            live_name: live_name.as_deref(),
            menu,
            menu_row: self.menu.as_ref().and_then(|m| m.row),
            menu_anchor: self.menu.as_ref().and_then(|m| m.anchor),
            trash: self.trash.as_ref().map(|t| t.entries.as_slice()),
            scrollbar,
            dragging: if self.sidebar_grip.is_some() { Some(Target::SidebarEdge) } else { self.scroll_drag.map(|d| d.bar) },
            opened_in: self.opened_in,
            keystroke: self.keystroke_median(),
            compact: self.compact || self.session.minimal,
            find: self.find.as_ref().map(find::Find::view),
            matches: self.find.as_ref().map_or(&[][..], |f| &f.matches),
            ticks: self.find.as_ref().map_or(&[][..], |f| &f.ticks),
            suggestions: self.suggest.as_ref().map(|s| scene::Suggestions {
                names: &s.names,
                selected: s.selected,
                create: s.create.as_deref(),
            }),
            link_field: self.link.as_ref().map(link::LinkState::view),
            link_card: self.link_hover.as_ref().filter(|h| h.shown).map(|h| (h.address.as_str(), h.at.0, h.at.1)),
            landmarks: &landmarks,
            spelling: self.spell.as_ref().filter(|_| !self.read_only && self.session.spell == 0).map(|s| &s.marks),
            page_gist: if page_date.is_some() && self.session.scroll_line == 0 { self.gist_view() } else { None },
            wash: self.wash_view(),
            page_date: page_date.as_deref(),
            row_date: self.row_date.as_ref().map(|(i, d)| (*i, d.as_str())),
            images: Some(&self.images),
            picture_resize: self.resizing.as_ref().map(|r| (r.line, r.width)),
            social: self.social.map(|platform| crate::social::SocialView { platform, scroll: self.social_scroll, offset: self.social_offset }),
            arriving: self.arrival_view(),
            scrub: self.scrub_view(),
            due_now: self.agenda.now,
            agenda: self.agenda_view(),
            updater: self.updater,
            update: self.update.as_deref(),
            share: self.share_view(),
            model_card: self.model_view(),
            selection_bar: self.bars.views().0.filter(|_| !self.read_only && self.popup.is_none() && self.scrub_page().is_none()),
            reading: self.bars.views().1,
            tooltip: self.bars.tooltip,
            marked: self.note_marked(),
            carried: match (self.row_press, self.row_drag) {
                (Some((i, _, _)), Some((x, y))) => self.session.sidebar.get(i).map(|r| (r.name.as_str(), x, y)),
                _ => None,
            },
        };
        // Scrubbing through the note's past: the version shown, no caret.
        let (text, lines, selection) = match self.scrub_page() {
            Some((text, lines)) => (text, lines, (usize::MAX, usize::MAX)),
            None => (e.text(), e.outline(), (s.anchor, s.head)),
        };
        build_scene(&self.eng, &self.fonts, &self.session, text, lines, selection, view, &self.ui, live)
    }

    pub fn draw(&mut self) -> Result<()> {
        self.refresh_find();
        // Lines typed, opened or scrolled to since the last frame go to the
        // checker: those around the view, however long the note.
        let versions = (self.editor.version(), self.popup.as_ref().map_or(0, |p| p.editor.version()), self.session.scroll_line);
        if let Some(spell) = &mut self.spell
            && spell.seen != versions
            && self.session.spell == 0
        {
            spell.seen = versions;
            if !self.read_only {
                let lines = self.editor.outline();
                let first = lines.partition_point(|l| (l.start as u64) < self.session.scroll_line).saturating_sub(20);
                spell.look_through(self.editor.text(), &lines[first..(first + 200).min(lines.len())]);
            }
            if let Some(p) = &self.popup {
                spell.look_through(p.editor.text(), p.editor.outline());
            }
        }
        let view = window::view(self.hwnd);
        self.ui.maximized = unsafe { IsZoomed(self.hwnd) }.as_bool();
        // The selection bar goes once its selection is not the one there is.
        let sel = self.editor.selection();
        self.bars.selbar.follow((sel.anchor, sel.head));
        let mut scene = self.scene(view)?;
        if std::mem::take(&mut self.reveal_caret) && self.scroll_to_caret(&scene, view) {
            scene = self.scene(view)?;
        }
        if self.reveal_popup(&scene) {
            scene = self.scene(view)?;
        }
        let popup_drawn = scene.popup.take();
        if let Some(p) = &mut self.popup {
            p.drawn = popup_drawn;
        }
        // The margin follows the caret from line to line.
        if scene.caret.map(|c| c.top) != self.caret_target.map(|c| c.top) && !self.chips.is_empty() {
            self.chips_moving = true;
        }
        // A page that cannot be edited has no caret.
        if self.read_only {
            scene.caret = None;
        }
        self.glide_to(scene.caret);
        // While a note is open over the page, its caret is the one typing;
        // the page's stays where it was (the margin still follows it).
        if self.popup.is_some() {
            scene.caret = None;
        }
        // Writing on, or moving to another paragraph, finds related notes
        // again after a pause.
        let head = self.editor.selection().head;
        let key = (self.editor.version(), self.editor.line_at(head).start, self.session.note.clone());
        if key != self.related_key {
            self.related_key = key;
            self.start_timer(RELATED_TIMER, RELATED_DELAY_MS);
        }
        scene.place_caret(self.caret_offset);
        self.hits = std::mem::take(&mut scene.hits);
        self.pills = scene.pills;
        self.tree_rows = scene.tree_rows;
        self.result_rows = scene.result_rows;
        self.options_scroll_max = scene.options_scroll_max;
        self.social_scroll_max = scene.social_scroll_max;
        self.tree_track = scene.tree_track;
        self.column = scene.column;
        self.lines = std::mem::take(&mut scene.lines);
        self.drawn_version = self.editor.version();
        let list = display::record(&self.eng, &scene, theme::for_mode(self.session.dark), view.2 as f32 / 96.0)?;
        self.surface.draw(view, &list);
        cpu::blit_changed(&self.surface, self.hwnd);
        self.place_candidates();
        self.dirty = false;
        // A key's frame is drawn: what it cost, from its message.
        if let Some(t) = self.key_arrived.take() {
            if self.key_ms.len() >= 64 {
                self.key_ms.remove(0);
            }
            self.key_ms.push(t.elapsed().as_secs_f32() * 1000.0);
        }
        Ok(())
    }

    fn request_frame(&mut self) {
        self.dirty = true;
    }

    /// The middle of what the last keystrokes cost, ms.
    fn keystroke_median(&self) -> Option<f32> {
        if self.key_ms.len() < 5 {
            return None;
        }
        let mut ms = self.key_ms.clone();
        ms.sort_by(f32::total_cmp);
        Some(ms[ms.len() / 2])
    }

    /// The line the caret is in (its spelling key) and the caret's byte in
    /// it: the word there is being typed.
    fn typing_at(&self) -> Option<(u64, u32)> {
        let (text, lines, at) = (self.editor.text(), self.editor.outline(), self.editor.selection().head);
        let line = lines.get(lines.partition_point(|l| l.start <= at).checked_sub(1)?)?;
        (line.end <= text.len()).then(|| (crate::spell::line_key(&text[line.start..line.end]), (at - line.start) as u32))
    }

    fn frame(&mut self) {
        // Frames arrive once per display refresh; after an idle stretch, step
        // as if one refresh passed rather than jumping.
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame).as_secs_f32();
        let dt = if elapsed > 0.1 { 1.0 / 60.0 } else { elapsed.min(1.0 / 30.0) };
        self.last_frame = now;
        // Answers whose nudge was lost while the app was busy.
        self.assist_events();
        if self.magnets_moving {
            self.magnets_moving = self.step_magnets(dt);
            self.dirty = true;
        }
        if self.caret_gliding {
            let (ox, oy) = &mut self.caret_offset;
            let (vx, vy) = &mut self.caret_velocity;
            let moving_x = settle(ox, vx, dt, CARET_OMEGA);
            let moving_y = settle(oy, vy, dt, CARET_OMEGA);
            self.caret_gliding = moving_x || moving_y;
            self.dirty = true;
        }
        if self.chips_moving {
            self.chips_moving = self.step_chips(dt);
            self.dirty = true;
        }
        if self.agenda_moving() {
            self.step_agenda(dt);
            self.dirty = true;
        }
        if self.arrival_moving {
            self.arrival_moving = self.step_arrival(dt);
            self.dirty = true;
        }
        if self.scrub_moving {
            self.scrub_moving = self.step_scrub(dt);
            self.dirty = true;
        }
        if let Some(t) = self.page_opened {
            if t.elapsed() >= PAGE_FADE {
                self.page_opened = None;
            }
            self.dirty = true;
        }
        if self.peek.as_ref().is_some_and(|p| p.since.elapsed() < PEEK_FADE + Duration::from_millis(20)) {
            self.dirty = true;
        }
        if self.popup.as_ref().is_some_and(|p| p.opened.elapsed() < POPUP_FADE + Duration::from_millis(20)) {
            self.dirty = true;
        }
        if self.menu.as_ref().is_some_and(|m| m.opened.elapsed() < menu::FADE + Duration::from_millis(20)) || self.share_card_fading() || self.model_card_moving() {
            self.dirty = true;
        }
        self.model_card_tick();
        if self.images.borrow().fading() {
            self.dirty = true;
        }
        if let Some(t) = self.scrolled_at {
            if t.elapsed() >= scroll::SHOWN + scroll::FADE + Duration::from_millis(20) {
                self.scrolled_at = None;
            }
            self.dirty = true;
        }
        if let Some((_, t)) = &self.whisper {
            let e = t.elapsed();
            if e >= WHISPER {
                self.whisper = None;
                self.dirty = true;
            } else if e < WHISPER_IN + Duration::from_millis(20) || e > WHISPER - WHISPER_OUT {
                self.dirty = true;
            }
        }
        if self.scroll_pending != 0.0 {
            let rest = self.scroll_pending;
            let step = if rest.abs() < 0.5 { rest } else { rest * (1.0 - (-SCROLL_RATE * dt).exp()) };
            // Stop at the top or the end instead of pushing against it.
            self.scroll_pending = if self.scroll_by(step) { rest - step } else { 0.0 };
            self.dirty = true;
        }
        if let Some(a) = &self.animation {
            let p = (a.start.elapsed().as_secs_f32() / a.duration.as_secs_f32()).min(1.0);
            let v: [f32; 4] = std::array::from_fn(|i| a.from[i] + (a.to[i] - a.from[i]) * p);
            self.ui.dock = [v[0], v[1]];
            self.ui.show = [v[2], v[3]];
            if p >= 1.0 {
                self.animation = None;
            }
            self.dirty = true;
        }
        self.step_assist();
        if self.dirty {
            self.assist_caret();
            let _ = self.draw();
        }
    }

    /// Input that needs this frame's layout (placing the caret with the
    /// mouse) first brings the layout up to date with the text.
    fn ensure_drawn(&mut self) {
        if self.drawn_version != self.editor.version() || self.dirty {
            let _ = self.draw();
        }
    }

    // The note's text and the caret.

    /// The page's text as it is laid out: the note, or the version of it
    /// being scrubbed to (which scrolls as it is drawn).
    fn note_text(&self) -> NoteText<'_> {
        let (text, lines, caret) = match self.scrub_page() {
            Some((text, lines)) => (text, lines, None),
            None => (self.editor.text(), self.editor.outline(), Some(self.editor.selection().head)),
        };
        NoteText {
            eng: &self.eng,
            fonts: &self.fonts,
            text,
            lines,
            width: self.column.width(),
            title: crate::note::title_line(text, lines),
            caret,
            clean: !self.session.show_marks,
            pictures: self.pictures_of(&self.note_path()),
            tables: Default::default(),
        }
    }

    /// The pictures of the note at `path`, for laying it out as it is drawn.
    fn pictures_of(&self, path: &Path) -> Option<crate::note::Pictures<'_>> {
        if self.session.vault.is_empty() {
            return None;
        }
        let vault = Path::new(&self.session.vault);
        let dir = path.parent().filter(|d| d.starts_with(vault)).map_or_else(|| vault.to_path_buf(), Path::to_path_buf);
        Some(crate::note::Pictures { images: &self.images, vault, dir, scale: window::dpi(self.hwnd) as f32 / 96.0, resize: None })
    }

    fn note_path(&self) -> PathBuf {
        Path::new(&self.session.vault).join(&self.session.note)
    }

    /// When the open note's file was made and last written.
    fn read_times(&mut self) {
        self.note_times = if self.session.note.is_empty() {
            (None, None)
        } else {
            match std::fs::metadata(self.note_path()) {
                Ok(m) => (m.created().ok(), m.modified().ok()),
                Err(_) => (None, None),
            }
        };
    }

    fn view_height(&self) -> f32 {
        let (_, height, dpi) = window::view(self.hwnd);
        height as f32 * 96.0 / dpi as f32
    }

    /// The caret moved; it glides there unless `glide` is false (dragging a
    /// selection, where it must stay under the pointer).
    fn caret_moved(&mut self, glide: bool) {
        self.caret_jumped = true;
        self.glide = glide;
        self.reveal_caret = true;
        self.update_suggestions();
        self.request_frame();
    }

    fn glide_to(&mut self, target: Option<Rect>) {
        if std::mem::take(&mut self.caret_jumped) {
            match (self.caret_target, target) {
                (Some(old), Some(new)) if self.glide => {
                    // Start from where the caret was drawn and close the gap.
                    self.caret_offset.0 += old.left - new.left;
                    self.caret_offset.1 += old.top - new.top;
                    self.caret_gliding = true;
                }
                _ => {
                    self.caret_offset = (0.0, 0.0);
                    self.caret_velocity = (0.0, 0.0);
                    self.caret_gliding = false;
                }
            }
        }
        self.caret_target = target;
    }

    /// Runs an editing command, then shows it and saves soon.
    fn edit(&mut self, f: impl FnOnce(&mut Editor)) {
        if self.read_only {
            return;
        }
        let before = self.editor.version();
        f(&mut self.editor);
        let changed = self.editor.version() != before;
        if changed {
            self.fix_scroll_line();
            self.clamp_scroll();
            self.start_timer(AUTOSAVE_TIMER, AUTOSAVE_DELAY_MS);
            self.assist_edited();
            self.session.last_active = session::now();
            // Ctrl+Z is the text's again.
            self.undo_delete.clear();
        }
        // The text changes at once, so the caret moves with it: gliding
        // there would draw it over the letters just typed. It glides only
        // through text that stays put (arrows, clicks).
        self.caret_moved(!changed);
    }

    fn go(&mut self, motion: Motion, extend: bool) {
        if !extend {
            let back = matches!(motion, Motion::Left | Motion::Up | Motion::WordLeft);
            let ahead = matches!(motion, Motion::Right | Motion::Down | Motion::WordRight);
            if (back || ahead) && self.step_off_picture(back) {
                return;
            }
        }
        let plan = self.editor.plan(motion, extend, &self.note_text());
        self.editor.apply(plan);
        if !extend {
            self.snap_to_picture();
        }
        self.caret_moved(true);
    }

    /// Page Up and Page Down: the caret moves a window's height of lines.
    fn page(&mut self, down: bool, extend: bool) {
        let lines = ((self.view_height() - self.column.top) / PAGE_LINE).max(1.0) as isize;
        let plan = self.editor.plan_lines(if down { lines } else { -lines }, extend, &self.note_text());
        self.editor.apply(plan);
        self.caret_moved(true);
    }

    /// A key went down; false if it is not ours.
    fn key(&mut self, vk: u16, ctrl: bool, shift: bool) -> bool {
        let k = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| key.0 == vk;
        if self.menu.is_some() {
            return self.menu_key(vk);
        }
        self.tooltip_away();
        if k(VK_ESCAPE) && self.close_share_card() {
            return true;
        }
        // Escape puts the Aa card and the selection bar away first.
        if k(VK_ESCAPE) && self.bars.reading.is_some() {
            self.close_reading();
            return true;
        }
        if k(VK_ESCAPE) && self.dismiss_selbar() {
            return true;
        }
        // Ctrl+T brings the Today sheet down, or sends it back up.
        if ctrl && !shift && vk == b'T' as u16 {
            self.toggle_agenda();
            return true;
        }
        if self.agenda_key(vk, ctrl) {
            return true;
        }
        if ctrl && shift && vk == b'H' as u16 {
            self.toggle_scrub();
            return true;
        }
        if self.scrub_key(vk) {
            return true;
        }
        self.hide_link_card();
        if self.link_key(vk, ctrl, shift) {
            return true;
        }
        if self.suggest.is_some() && !ctrl && self.suggest_key(vk) {
            return true;
        }
        if self.find_key(vk, ctrl, shift) {
            return true;
        }
        if ctrl && !shift && (vk == b'F' as u16 || vk == b'H' as u16) {
            self.open_find(vk == b'H' as u16);
            return true;
        }
        if vk == windows::Win32::UI::Input::KeyboardAndMouse::VK_F3.0 {
            if self.find.is_none() {
                self.open_find(false);
            } else {
                self.find_step(shift);
            }
            return true;
        }
        if self.find.is_some() && k(VK_ESCAPE) && self.popup.is_none() {
            self.close_find();
            return true;
        }
        if k(windows::Win32::UI::Input::KeyboardAndMouse::VK_APPS) || (k(windows::Win32::UI::Input::KeyboardAndMouse::VK_F10) && shift) {
            self.open_text_menu_at_caret();
            return true;
        }
        if self.social.is_some() && k(VK_ESCAPE) {
            self.show_social(None);
            return true;
        }
        if self.ui.options_open && k(VK_ESCAPE) {
            self.ui.options_open = false;
            self.request_frame();
            return true;
        }
        if ctrl && !shift && vk == b'Z' as u16 && self.undo_delete.last().is_some_and(|(_, v)| *v == self.editor.version()) && self.popup.is_none() {
            self.undo_last_delete();
            return true;
        }
        if self.trash.is_some() && k(VK_ESCAPE) {
            self.close_trash();
            return true;
        }
        if self.popup.is_some() {
            return self.popup_key(vk, ctrl, shift);
        }
        if self.search.is_some() {
            return self.search_key(vk, ctrl, shift);
        }
        if !ctrl && self.picture_key(vk) {
            return true;
        }
        if k(VK_LEFT) {
            self.go(if ctrl { Motion::WordLeft } else { Motion::Left }, shift);
        } else if k(VK_RIGHT) {
            self.go(if ctrl { Motion::WordRight } else { Motion::Right }, shift);
        } else if k(VK_UP) {
            self.go(Motion::Up, shift);
        } else if k(VK_DOWN) {
            self.go(Motion::Down, shift);
        } else if k(VK_HOME) {
            self.go(if ctrl { Motion::DocStart } else { Motion::LineStart }, shift);
        } else if k(VK_END) {
            self.go(if ctrl { Motion::DocEnd } else { Motion::LineEnd }, shift);
        } else if k(VK_PRIOR) || k(VK_NEXT) {
            self.page(k(VK_NEXT), shift);
        } else if k(VK_BACK) {
            self.edit(|e| e.backspace(ctrl));
        } else if k(VK_DELETE) && shift && !ctrl {
            self.cut();
        } else if k(VK_DELETE) {
            self.edit(|e| e.delete(ctrl));
        } else if k(VK_RETURN) && ctrl {
            // Ctrl+Enter checks a task off, or makes the line one.
            self.edit(|e| {
                let at = e.selection().head;
                if !e.toggle_task(at) {
                    e.toggle_block(BlockKind::Task);
                }
            });
        } else if k(VK_RETURN) {
            // Shift+Enter breaks the line without continuing a list.
            self.edit(|e| if shift { e.insert("\n") } else { e.newline() });
        } else if k(VK_TAB) && !ctrl {
            // In a list, Tab nests the item; elsewhere it is a tab.
            self.edit(|e| {
                if !e.indent(shift) && !shift {
                    e.insert("\t");
                }
            });
        } else if k(VK_INSERT) && ctrl {
            self.copy();
        } else if k(VK_INSERT) && shift {
            self.paste();
        } else if ctrl {
            match vk as u8 {
                b'A' => {
                    self.editor.select_all();
                    self.caret_moved(false);
                }
                b'B' => self.tool(Tool::Bold),
                b'I' => self.tool(Tool::Italic),
                b'X' if shift => self.tool(Tool::Strike),
                b'E' => self.tool(Tool::Code),
                b'K' => self.tool(Tool::Link),
                b'L' => self.tool(Tool::Checklist),
                b'8' if shift => self.tool(Tool::List),
                b'1' | b'2' | b'3' => self.edit(|e| e.toggle_block(BlockKind::Heading(vk as u8 - b'0'))),
                b'0' => self.edit(|e| e.toggle_block(BlockKind::Body)),
                b'Z' if shift => self.edit(|e| {
                    e.redo();
                }),
                b'Z' => {
                    self.edit(|e| {
                        e.undo();
                    });
                    self.assist_undone();
                }
                b'Y' => self.edit(|e| {
                    e.redo();
                }),
                b'C' => self.copy(),
                b'X' => self.cut(),
                b'V' if shift => self.paste_plain(),
                b'V' => self.paste(),
                b'N' => self.fresh_page(),
                b'O' => self.open_search(),
                b'P' => self.open_palette(),
                b'F' if shift => self.open_search(),
                _ => return false,
            }
        } else {
            return false;
        }
        true
    }

    // Search.

    fn start_index(&self) {
        if self.session.vault.is_empty() {
            return;
        }
        let vault = PathBuf::from(&self.session.vault);
        let hwnd = crate::Handoff(self.hwnd);
        std::thread::spawn(move || {
            let hwnd = hwnd;
            crate::background_priority();
            let notes = search::index(&vault);
            let corpus = Corpus::build(&notes);
            let due = agenda::due_in(&notes);
            let index = Box::into_raw(Box::new((notes, corpus, due)));
            let posted = unsafe { PostMessageW(Some(hwnd.0), WM_APP_INDEX, WPARAM(0), LPARAM(index as isize)) };
            if posted.is_err() {
                drop(unsafe { Box::from_raw(index) });
            }
            // Then the vault's PDFs, by their text: read once, kept after.
            let files = crate::convert::index_files(&vault);
            if !files.is_empty() {
                let files = Box::into_raw(Box::new((vault.to_string_lossy().into_owned(), files)));
                let posted = unsafe { PostMessageW(Some(hwnd.0), WM_APP_INDEX, WPARAM(1), LPARAM(files as isize)) };
                if posted.is_err() {
                    drop(unsafe { Box::from_raw(files) });
                }
            }
        });
    }

    // A note open over the page.

    /// Opens the note at `path` over the page, beside height `anchor`, with
    /// the caret at `passage` if given. The page's own note opens nowhere.
    fn open_popup(&mut self, path: &Path, anchor: f32, passage: Option<String>) {
        if !self.session.note.is_empty() && path == self.note_path() {
            return;
        }
        self.save_popup(true);
        if self.search.is_some() {
            self.close_search();
        }
        let Some((text, format, added)) = read_file(path) else { return };
        let mut editor = Editor::new(text);
        editor.set_clean(!self.session.show_marks);
        editor.set_snippets(daily::snippets());
        let at = passage.as_deref().and_then(|p| editor.text().find(p)).unwrap_or_else(|| body_start(&editor));
        editor.set_selection(Selection::caret(at));
        self.popup = Some(Popup {
            path: path.to_path_buf(),
            name: stem(path),
            saved_version: editor.version(),
            disk: stamp(path),
            named: editor.title(),
            added,
            editor,
            format,
            scroll: 0.0,
            anchor,
            opened: Instant::now(),
            offset: (0.0, 0.0),
            held: None,
            drawn: None,
            reveal: passage.is_some(),
            selecting: false,
        });
        self.peek = None;
        self.request_frame();
    }

    /// Writes the popup's note if it changed. Its file name follows its
    /// title once the caret leaves that line, or the popup closes (`flush`).
    fn save_popup(&mut self, flush: bool) {
        self.stop_timer(POPUP_SAVE_TIMER);
        if self.popup.as_ref().is_some_and(|p| p.editor.marked().is_some()) {
            if !flush {
                return self.start_timer(POPUP_SAVE_TIMER, AUTOSAVE_DELAY_MS);
            }
            self.settle_composition();
        }
        let Some(p) = &mut self.popup else { return };
        if flush && p.editor.version() != p.saved_version {
            p.editor.tidy();
        }
        if p.editor.version() != p.saved_version {
            // Another app changed it since: these edits go beside theirs.
            if let (Some(was), Some(now)) = (p.disk, stamp(&p.path))
                && was != now
                && let Some(dir) = p.path.parent()
            {
                p.path = note::unique_path(dir, &format!("{} (conflict)", stem(&p.path)), None);
                p.name = stem(&p.path);
                self.refresh_tree();
            }
            let Some(p) = &mut self.popup else { return };
            let text = file_text(p.editor.text(), &mut p.added).into_owned();
            if note::save(&p.path, &text, p.format).is_ok() {
                p.saved_version = p.editor.version();
                p.disk = stamp(&p.path);
                let path = p.path.clone();
                self.written_to_top(&path);
            }
        }
        let Some(p) = &mut self.popup else { return };
        let title = p.editor.title();
        if title != p.named && !title.is_empty() && (flush || !on_title(&p.editor)) {
            let from = p.path.clone();
            p.named = title.clone();
            if let Some(to) = self.rename_note(&from, &title)
                && let Some(p) = &mut self.popup
            {
                p.name = stem(&to);
                p.path = to;
            }
        }
    }

    fn close_popup(&mut self) {
        self.save_popup(true);
        if self.popup.take().is_some() {
            self.request_frame();
        }
    }

    /// The popup was let go at a client point in pixels after dragging it.
    /// Out of the window, it becomes a window of its own there.
    fn popup_dropped(&mut self, x: i32, y: i32) {
        let (w, h, _) = window::view(self.hwnd);
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            let mut at = POINT { x, y };
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ClientToScreen(self.hwnd, &mut at);
            }
            if let Some(path) = self.popup.as_ref().map(|p| p.path.clone()) {
                self.close_popup();
                self.open_window(&path, Some((at.x, at.y)));
            }
        }
        self.request_frame();
    }

    /// The popup's note becomes the page.
    fn expand_popup(&mut self) {
        let Some(path) = self.popup.as_ref().map(|p| p.path.clone()) else { return };
        self.close_popup();
        self.open(&path);
    }

    /// Keeps the popup's caret inside its card; true if it scrolled.
    fn reveal_popup(&mut self, scene: &Scene) -> bool {
        let Some(p) = &mut self.popup else { return false };
        if !std::mem::take(&mut p.reveal) {
            return false;
        }
        let (Some(d), Some(c)) = (&scene.popup, scene.popup.as_ref().and_then(|d| d.caret)) else { return false };
        let delta = if c.top < d.content.top + 8.0 {
            c.top - d.content.top - 8.0
        } else if c.bottom > d.content.bottom - 24.0 {
            c.bottom - (d.content.bottom - 24.0)
        } else {
            return false;
        };
        let max = (d.height - d.content.height()).max(0.0);
        let scroll = (p.scroll + delta).clamp(0.0, max);
        let moved = scroll != p.scroll;
        p.scroll = scroll;
        moved
    }

    /// The popup's text laid out as the last frame drew it, for moving its
    /// caret.
    fn popup_text<'a>(&'a self, p: &'a Popup) -> NoteText<'a> {
        let width = p.drawn.as_ref().map_or(452.0, |d| d.content.width());
        NoteText {
            eng: &self.eng,
            fonts: &self.fonts,
            text: p.editor.text(),
            lines: p.editor.outline(),
            width,
            title: crate::note::title_line(p.editor.text(), p.editor.outline()),
            caret: Some(p.editor.selection().head),
            clean: !self.session.show_marks,
            pictures: self.pictures_of(&p.path),
            tables: Default::default(),
        }
    }

    /// Runs an editing command in the popup; it saves soon after.
    fn popup_edit(&mut self, f: impl FnOnce(&mut Editor)) {
        let Some(p) = &mut self.popup else { return };
        let before = p.editor.version();
        f(&mut p.editor);
        p.reveal = true;
        if p.editor.version() != before {
            self.start_timer(POPUP_SAVE_TIMER, AUTOSAVE_DELAY_MS);
        }
        self.request_frame();
    }

    fn popup_go(&mut self, motion: Option<Motion>, lines: isize, extend: bool) {
        let Some(p) = &self.popup else { return };
        let layout = self.popup_text(p);
        let plan = match motion {
            Some(m) => p.editor.plan(m, extend, &layout),
            None => p.editor.plan_lines(lines, extend, &layout),
        };
        if let Some(p) = &mut self.popup {
            p.editor.apply(plan);
            p.reveal = true;
        }
        self.request_frame();
    }

    /// Keys while a note is open over the page: they edit it.
    fn popup_key(&mut self, vk: u16, ctrl: bool, shift: bool) -> bool {
        let k = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| key.0 == vk;
        let page = self.popup.as_ref().and_then(|p| p.drawn.as_ref()).map_or(10, |d| (d.content.height() / PAGE_LINE) as isize);
        if k(VK_ESCAPE) {
            self.close_popup();
        } else if k(VK_LEFT) {
            self.popup_go(Some(if ctrl { Motion::WordLeft } else { Motion::Left }), 0, shift);
        } else if k(VK_RIGHT) {
            self.popup_go(Some(if ctrl { Motion::WordRight } else { Motion::Right }), 0, shift);
        } else if k(VK_UP) {
            self.popup_go(None, -1, shift);
        } else if k(VK_DOWN) {
            self.popup_go(None, 1, shift);
        } else if k(VK_HOME) {
            self.popup_go(Some(if ctrl { Motion::DocStart } else { Motion::LineStart }), 0, shift);
        } else if k(VK_END) {
            self.popup_go(Some(if ctrl { Motion::DocEnd } else { Motion::LineEnd }), 0, shift);
        } else if k(VK_PRIOR) || k(VK_NEXT) {
            self.popup_go(None, if k(VK_NEXT) { page } else { -page }, shift);
        } else if k(VK_BACK) {
            self.popup_edit(|e| e.backspace(ctrl));
        } else if k(VK_DELETE) {
            self.popup_edit(|e| e.delete(ctrl));
        } else if k(VK_RETURN) && ctrl {
            self.popup_edit(|e| {
                let at = e.selection().head;
                if !e.toggle_task(at) {
                    e.toggle_block(BlockKind::Task);
                }
            });
        } else if k(VK_RETURN) {
            self.popup_edit(|e| if shift { e.insert("\n") } else { e.newline() });
        } else if k(VK_TAB) && !ctrl {
            self.popup_edit(|e| {
                if !e.indent(shift) && !shift {
                    e.insert("\t");
                }
            });
        } else if ctrl {
            match vk as u8 {
                b'A' => self.popup_edit(|e| e.select_all()),
                b'Z' if shift => self.popup_edit(|e| {
                    e.redo();
                }),
                b'Z' => self.popup_edit(|e| {
                    e.undo();
                }),
                b'Y' => self.popup_edit(|e| {
                    e.redo();
                }),
                b'X' if shift => self.tool(Tool::Strike),
                b'C' | b'X' => {
                    if let Some(p) = &self.popup
                        && !p.editor.selection().is_caret()
                    {
                        copy_from(self.hwnd, &p.editor);
                        if vk as u8 == b'X' {
                            self.popup_edit(|e| e.backspace(false));
                        }
                    }
                }
                b'V' => self.popup_paste(shift),
                b'B' => self.tool(Tool::Bold),
                b'I' => self.tool(Tool::Italic),
                b'E' => self.tool(Tool::Code),
                b'K' => self.tool(Tool::Link),
                b'L' => self.tool(Tool::Checklist),
                b'8' if shift => self.tool(Tool::List),
                b'1' | b'2' | b'3' => self.popup_edit(|e| e.toggle_block(BlockKind::Heading(vk as u8 - b'0'))),
                b'0' => self.popup_edit(|e| e.toggle_block(BlockKind::Body)),
                _ => return false,
            }
        } else {
            return false;
        }
        true
    }

    /// The text offset in the popup under a point in DIPs.
    fn popup_offset(&self, x: f32, y: f32) -> Option<usize> {
        let p = self.popup.as_ref()?;
        let d = p.drawn.as_ref()?;
        let line = d.lines.iter().find(|l| y < l.y + l.height).or(d.lines.last())?;
        let ly = (y - line.y).clamp(0.0, (line.height - 0.5).max(0.0));
        self.popup_text(p).offset_at(&line.para, x - line.x, ly).ok()
    }

    /// The button went down on the popup's text.
    fn popup_down(&mut self, px: i32, py: i32) {
        if self.dirty {
            let _ = self.draw();
        }
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let Some(at) = self.popup_offset(px as f32 / scale, py as f32 / scale) else { return };
        let clicks = self.count_click(px, py);
        let shift = key_down(VK_SHIFT.0);
        let hwnd = self.hwnd;
        let Some(p) = &mut self.popup else { return };
        match clicks {
            2 => p.editor.select_word_at(at),
            3 => p.editor.select_line_at(at),
            _ => {
                let anchor = if shift { p.editor.selection().anchor } else { at };
                p.editor.set_selection(Selection { anchor, head: at });
            }
        }
        p.selecting = clicks == 1;
        if p.selecting {
            unsafe { SetCapture(hwnd) };
        }
        self.request_frame();
    }

    fn popup_drag(&mut self, px: i32, py: i32) {
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let Some(at) = self.popup_offset(px as f32 / scale, py as f32 / scale) else { return };
        if let Some(p) = &mut self.popup {
            let s = p.editor.selection();
            if at != s.head {
                p.editor.set_selection(Selection { anchor: s.anchor, head: at });
                p.reveal = true;
                self.request_frame();
            }
        }
    }

    fn scroll_popup(&mut self, notches: f32) {
        if let Some(p) = &mut self.popup
            && let Some(d) = &p.drawn
        {
            let max = (d.height - d.content.height()).max(0.0);
            p.scroll = (p.scroll - notches * WHEEL_STEP).clamp(0.0, max);
            self.scrolled_at = Some(Instant::now());
            self.request_frame();
        }
    }

    // Magnetic memory.

    /// Finds the notes related to what is being written (most to the
    /// paragraph at the caret and its neighbors). Chips still related stay
    /// and move to their new places, new ones drift in, the rest fade out.
    fn update_related(&mut self) {
        self.stop_timer(RELATED_TIMER);
        let e = &self.editor;
        let head = e.selection().head;
        self.related_key = (e.version(), e.line_at(head).start, self.session.note.clone());
        let found = match &self.corpus {
            Some(corpus) if !corpus.is_empty() && !e.text().trim().is_empty() && !self.read_only && self.session.related == 0 => {
                let lines = e.outline();
                let i = lines.partition_point(|l| l.start <= head).max(1) - 1;
                let from = lines[i.saturating_sub(1)].start;
                let to = lines[(i + 1).min(lines.len() - 1)].end;
                corpus.related(e.text(), &e.text()[from..to], &self.session.note, CHIP_LIMIT)
            }
            _ => Vec::new(),
        };
        let mut next = Vec::with_capacity(found.len() + self.chips.len());
        for rel in found {
            let key = rel.key();
            match self.chips.iter().position(|c| c.rel.key() == key) {
                Some(i) => {
                    let mut chip = self.chips.remove(i);
                    chip.rel = rel;
                    chip.leaving = false;
                    next.push(chip);
                }
                None => next.push(ChipState { rel, y: f32::NAN, vy: 0.0, dx: 28.0, vdx: 0.0, alpha: 0.0, leaving: false }),
            }
        }
        for mut chip in self.chips.drain(..) {
            chip.leaving = true;
            next.push(chip);
        }
        self.chips = next;
        self.chips_moving = true;
        // A peek belongs to a chip that may have moved.
        self.peek = None;
        self.request_frame();
    }

    /// Steps the chips' springs; true while any still moves. They stack
    /// beside the caret's line, as far as the window allows.
    fn step_chips(&mut self, dt: f32) -> bool {
        let anchor = self.caret_target.map_or(self.column.top, |c| c.top);
        let bottom = self.view_height() - 16.0;
        let shown = self.chips.iter().filter(|c| !c.leaving).count() as f32;
        let stack = shown * (CHIP_HEIGHT + CHIP_GAP) - CHIP_GAP;
        let top = (anchor - 14.0).min(bottom - stack).max(self.column.top);
        let mut moving = false;
        let mut k = 0.0;
        for c in &mut self.chips {
            if !c.leaving {
                let target = top + k * (CHIP_HEIGHT + CHIP_GAP);
                k += 1.0;
                if c.y.is_nan() {
                    c.y = target;
                }
                moving |= drift(&mut c.y, &mut c.vy, target, dt);
            }
            moving |= drift(&mut c.dx, &mut c.vdx, if c.leaving { 16.0 } else { 0.0 }, dt);
            let goal = if c.leaving { 0.0 } else { 1.0 };
            let step = dt * 5.0;
            c.alpha = if c.alpha < goal { (c.alpha + step).min(goal) } else { (c.alpha - step).max(goal) };
            moving |= c.alpha != goal;
        }
        self.chips.retain(|c| !(c.leaving && c.alpha <= 0.0));
        moving
    }

    /// A chip clicked: its note opens over the page (a marked passage's, at
    /// the passage).
    fn open_chip(&mut self, i: usize) {
        let Some(chip) = self.chips.get(i) else { return };
        let path = Path::new(&self.session.vault).join(&chip.rel.path);
        let passage = (chip.rel.kind == kolada_core::related::Kind::Highlight).then(|| chip.rel.detail.clone());
        let anchor = chip.y;
        self.open_popup(&path, anchor, passage);
    }

    /// A chip dropped into the text: a link to its note, there.
    fn link_chip(&mut self, i: usize, at: usize) {
        let Some(name) = self.chips.get(i).map(|c| c.rel.name.clone()) else { return };
        self.insert_link(&name, at);
    }

    /// A `[[link]]` to the note `name` at `at`, spaced from the words
    /// around it.
    fn insert_link(&mut self, name: &str, at: usize) {
        if self.read_only {
            return;
        }
        let text = self.editor.text();
        let at = at.min(text.len());
        let mut link = String::new();
        if text[..at].chars().next_back().is_some_and(|c| !c.is_whitespace()) {
            link.push(' ');
        }
        link.push_str(&format!("[[{name}]]"));
        if text[at..].chars().next().is_some_and(|c| !c.is_whitespace()) {
            link.push(' ');
        }
        self.editor.set_selection(Selection::caret(at));
        self.edit(|e| e.insert(&link));
    }

    /// A note row carried to a client point in pixels.
    fn carry_row(&mut self, x: i32, y: i32) {
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let p = (x as f32 / scale, y as f32 / scale);
        self.row_drag = Some(p);
        let over_text = scene::hit(&self.hits, p.0, p.1) == Some(Target::Note);
        self.drop_at = over_text.then(|| self.offset_at(p.0, p.1));
        self.request_frame();
    }

    /// A carried row let go at a client point in pixels: over the text it
    /// links there; out of the window, its note opens in a window there.
    fn drop_row(&mut self, i: usize, x: i32, y: i32) {
        let Some(row) = self.session.sidebar.get(i).cloned() else { return };
        let rel = vault::row_paths(&self.session.sidebar)[i].clone();
        let path = PathBuf::from(&self.session.vault).join(format!("{rel}.md"));
        let (w, h, _) = window::view(self.hwnd);
        if let Some(at) = self.drop_at.take() {
            if path != self.note_path() {
                self.insert_link(&row.name, at);
            }
        } else if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            let mut at = POINT { x, y };
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ClientToScreen(self.hwnd, &mut at);
            }
            self.open_window(&path, Some((at.x, at.y)));
        }
        self.request_frame();
    }

    fn drag_chip(&mut self, x: i32, y: i32) {
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let p = (x as f32 / scale, y as f32 / scale);
        self.chip_drag = Some(p);
        let over_text = scene::hit(&self.hits, p.0, p.1) == Some(Target::Note);
        self.drop_at = over_text.then(|| self.offset_at(p.0, p.1));
        self.request_frame();
    }

    /// Search: the side menu's field takes the keys. In focus mode the menu
    /// slides out while searching.
    fn open_search(&mut self) {
        self.close_popup();
        self.ui.options_open = false;
        match &mut self.search {
            Some(s) => s.query.select_all(),
            None => {
                self.search = Some(SearchState {
                    query: Editor::field(""),
                    hits: Vec::new(),
                    selected: 0,
                    first: 0,
                    commands: None,
                    shown: Vec::new(),
                    runs: Vec::new(),
                });
                self.start_index();
            }
        }
        if self.collapsed(Panel::Sidebar) && !self.revealed[0] {
            self.revealed[0] = true;
            self.held_back[0] = false;
            self.retarget(REVEAL_ANIMATION);
        }
        self.request_frame();
    }

    fn close_search(&mut self) {
        self.search = None;
        self.request_frame();
        // A menu slid out for the search slides back unless the pointer is on it.
        self.update_reveal();
    }

    fn run_search(&mut self) {
        let palette = self.search.as_ref().is_some_and(|s| s.query.text().starts_with('>'));
        // The commands are gathered as the palette opens (installed agents
        // are looked up then), not on every key.
        let commands = (palette && self.search.as_ref().is_some_and(|s| s.commands.is_none())).then(|| self.commands());
        if let Some(s) = &mut self.search {
            if palette {
                if let Some(c) = commands {
                    s.commands = Some(c);
                }
                let all = s.commands.as_deref().unwrap_or_default();
                let order = palette::ranked(all, &s.query.text()[1..]);
                s.shown = order.iter().map(|&i| all[i].item.clone()).collect();
                s.runs = order.iter().map(|&i| all[i].run.clone()).collect();
                s.hits.clear();
            } else {
                s.commands = None;
                s.shown.clear();
                s.runs.clear();
                s.hits = search::search(&self.index, s.query.text(), SEARCH_LIMIT);
            }
            s.selected = 0;
            s.first = 0;
        }
        self.request_frame();
    }

    fn open_hit(&mut self, i: usize) {
        if self.search.as_ref().is_some_and(|s| s.commands.is_some()) {
            return self.run_command(i);
        }
        let Some(path) = self.search.as_ref().and_then(|s| s.hits.get(i)).map(|h| h.path.clone()) else { return };
        self.close_search();
        let note = search::is_note(&path);
        let path = Path::new(&self.session.vault).join(path);
        if note {
            self.open(&path);
        } else if !share::open_file(&path) {
            // A PDF opens in the app Windows has for it.
            self.whisper("Windows has no app to open it".to_string());
        }
    }

    /// Keys while searching; every key is the search's then.
    fn search_key(&mut self, vk: u16, ctrl: bool, shift: bool) -> bool {
        let k = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| key.0 == vk;
        let Some(s) = &mut self.search else { return false };
        let motion = |m: Motion, s: &mut SearchState| {
            let plan = s.query.plan(m, shift, &OneLine(s.query.text()));
            s.query.apply(plan);
        };
        let before = s.query.version();
        if k(VK_ESCAPE) {
            self.close_search();
            return true;
        } else if k(VK_RETURN) {
            let i = s.selected;
            self.open_hit(i);
            return true;
        } else if k(VK_UP) {
            s.selected = s.selected.saturating_sub(1);
            s.first = s.first.min(s.selected);
        } else if k(VK_DOWN) {
            let n = if s.commands.is_some() { s.shown.len() } else { s.hits.len() };
            s.selected = (s.selected + 1).min(n.saturating_sub(1));
            // Scrolled along once it passes the last row that fit.
            let fit = self.result_rows.max(1);
            if s.selected >= s.first + fit {
                s.first = s.selected + 1 - fit;
            }
        } else if k(VK_LEFT) {
            motion(if ctrl { Motion::WordLeft } else { Motion::Left }, s);
        } else if k(VK_RIGHT) {
            motion(if ctrl { Motion::WordRight } else { Motion::Right }, s);
        } else if k(VK_HOME) {
            motion(Motion::DocStart, s);
        } else if k(VK_END) {
            motion(Motion::DocEnd, s);
        } else if k(VK_BACK) {
            s.query.backspace(ctrl);
        } else if k(VK_DELETE) {
            s.query.delete(ctrl);
        } else if ctrl && vk == b'A' as u16 {
            s.query.select_all();
        } else if ctrl && vk == b'V' as u16 {
            if let Some(text) = clipboard::get(self.hwnd) {
                let line = text.lines().next().unwrap_or("").to_string();
                if let Some(s) = &mut self.search {
                    s.query.insert(&line);
                }
            }
        } else if ctrl && vk == b'N' as u16 {
            self.close_search();
            self.fresh_page();
            return true;
        }
        if self.search.as_ref().is_some_and(|s| s.query.version() != before) {
            self.run_search();
        }
        self.request_frame();
        true
    }

    // Options.

    fn set_dark(&mut self, dark: bool) {
        if self.session.dark != dark {
            self.session.dark = dark;
            window::set_attribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, dark);
        }
        self.request_frame();
    }

    fn choose(&mut self, setting: Setting, value: u8) {
        match setting {
            Setting::Theme => {
                self.session.theme = value;
                let system = self.system_dark.unwrap_or(self.session.dark);
                self.set_dark(match value {
                    1 => false,
                    2 => true,
                    _ => system,
                });
            }
            Setting::TextSize | Setting::Font => {
                if setting == Setting::Font {
                    self.session.font = value;
                } else {
                    self.session.text_size = value;
                }
                if setting == Setting::Font {
                    // The note's typeface is the engine's; layouts made in
                    // the old one are let go with its generation.
                    self.eng.borrow_mut().set_family(kolada_text::Role::Text, &crate::fonts::font_set(value).text);
                }
                self.fonts = Fonts::new(scene::text_scale(self.session.text_size), self.session.font);
            }
            Setting::LineWidth => self.session.line_width = value,
            Setting::Launch => self.session.launch = value,
            Setting::Related => {
                self.session.related = value;
                self.update_related();
            }
            Setting::Spell => self.session.spell = value,
            Setting::Tray => {
                self.session.tray = value;
                self.update_tray();
            }
            Setting::Hotkey => {
                self.session.hotkey = value;
                self.update_hotkey(true);
            }
            Setting::Window => self.session.window_mode = value,
            Setting::Assist => self.choose_assist(value),
            Setting::Remind => self.session.remind = value,
            Setting::Updates => {
                self.session.updates = value;
                if self.updater {
                    crate::updates::watch(self.hwnd, value == 0);
                }
            }
            Setting::ShowMarks => {
                self.session.show_marks = value == 1;
                self.editor.set_clean(value == 0);
                if let Some(p) = &mut self.popup {
                    p.editor.set_clean(value == 0);
                }
            }
        }
        self.caret_jumped = true;
        self.glide = false;
        self.request_frame();
    }

    /// A typed character, one UTF-16 unit at a time (WM_CHAR). Control
    /// characters arrive as keys instead.
    fn typed(&mut self, unit: u16) {
        // The past is looked at, not typed into; nor is the page under the
        // Today sheet (Space there ticked a task; other keys let it go first).
        if self.scrub_page().is_some() || self.agenda_down() {
            return;
        }
        let c = match unit {
            0xD800..=0xDBFF => {
                self.high_surrogate = Some(unit);
                return;
            }
            0xDC00..=0xDFFF => {
                let Some(high) = self.high_surrogate.take() else { return };
                char::decode_utf16([high, unit]).next().and_then(|c| c.ok())
            }
            u if u < 0x20 || u == 0x7F => None,
            u => char::from_u32(u as u32),
        };
        if let Some(c) = c {
            let mut buf = [0; 4];
            let s: &str = c.encode_utf8(&mut buf);
            if self.link_typed(s) {
                return;
            }
            if self.find_typed(s) {
                return;
            }
            if self.popup.is_some() {
                return self.popup_edit(|e| e.insert(s));
            }
            if let Some(search) = &mut self.search {
                search.query.insert(s);
                self.run_search();
                return;
            }
            self.leave_picture_for_text();
            self.edit(|e| e.insert(s));
        }
    }

    /// A toolbar button, or its shortcut.
    fn tool(&mut self, tool: Tool) {
        if self.popup.is_some() {
            return self.popup_edit(|e| apply_tool(e, tool));
        }
        // On the page, Link asks what to link to (see `link`).
        if tool == Tool::Link {
            return self.open_link_field();
        }
        self.edit(|e| apply_tool(e, tool));
    }

    /// Ctrl+click on a `[[link]]` (at `y` DIPs): opens the note it names
    /// over the page, creating it if there is none yet. False if there is no
    /// link at `at`.
    fn follow_link(&mut self, at: usize, y: f32) -> bool {
        let name = match link_at(self.editor.text(), at) {
            None => return false,
            Some(Link::Note(name)) => name,
            Some(Link::Web(url)) => {
                share::open_web(&url);
                return true;
            }
            Some(Link::Tag(tag)) => {
                self.open_search();
                if let Some(s) = &mut self.search {
                    s.query = Editor::field(tag);
                    s.query.select_all();
                    let end = s.query.text().len();
                    s.query.set_selection(Selection::caret(end));
                }
                self.run_search();
                return true;
            }
        };
        let Some(vault) = self.ensure_vault() else { return true };
        let path = vault::find_note(&vault, &name).or_else(|| {
            let path = note::unique_path(&vault, &note::file_stem_for(&name), None);
            std::fs::write(&path, "").ok().map(|_| path)
        });
        if let Some(path) = path {
            self.open_popup(&path, y, None);
            self.refresh_tree();
        }
        true
    }


    fn copy(&self) {
        copy_from(self.hwnd, &self.editor);
    }

    fn cut(&mut self) {
        if !self.editor.selection().is_caret() {
            self.copy();
            self.edit(|e| e.backspace(false));
        }
    }

    /// The text offset under a point in DIPs, from the last frame's layout.
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

    /// The side menu's edge pressed at `(x, y)` (pixels): dragged, it sets
    /// the menu's width; double-clicked, it goes back to its own.
    fn grab_sidebar_edge(&mut self, x: i32, y: i32) {
        if self.count_click(x, y) == 2 {
            self.session.sidebar_width = 0;
            self.save_session();
            return self.request_frame();
        }
        let (w, _, dpi) = window::view(self.hwnd);
        let scale = dpi as f32 / 96.0;
        let width = chrome::menu_width(self.session.sidebar_width as f32, w as f32 / scale);
        self.sidebar_grip = Some((x as f32 / scale, width));
        unsafe { SetCapture(self.hwnd) };
        self.request_frame();
    }

    /// The side menu's edge dragged to `x` (pixels). Near its own width it
    /// settles there.
    fn drag_sidebar_edge(&mut self, x: i32) {
        let Some((from, width)) = self.sidebar_grip else { return };
        let (w, _, dpi) = window::view(self.hwnd);
        let scale = dpi as f32 / 96.0;
        let wanted = (width + x as f32 / scale - from).clamp(chrome::SIDEBAR_LEAST, chrome::SIDEBAR_MOST);
        let fits = chrome::menu_width(wanted, w as f32 / scale);
        let set = if (fits - chrome::SIDEBAR_WIDTH).abs() < 6.0 { 0 } else { fits.round() as u16 };
        if set != self.session.sidebar_width {
            self.session.sidebar_width = set;
            self.request_frame();
        }
    }

    /// Counts a click as single, double or triple.
    fn count_click(&mut self, x: i32, y: i32) -> u32 {
        let time = unsafe { GetMessageTime() } as u32;
        let (last, lx, ly, n) = self.last_click;
        let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK)) };
        let again = time.wrapping_sub(last) <= unsafe { GetDoubleClickTime() }
            && (x - lx).abs() <= cx / 2 + 1
            && (y - ly).abs() <= cy / 2 + 1;
        let n = if again { n % 3 + 1 } else { 1 };
        self.last_click = (time, x, y, n);
        n
    }

    /// The button went down on the note text: place the caret, select a
    /// word or paragraph, or extend the selection with Shift.
    fn note_down(&mut self, px: i32, py: i32) {
        if self.search.is_some() {
            self.close_search();
        }
        self.ensure_drawn();
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let at = self.offset_at(px as f32 / scale, py as f32 / scale);
        // Ctrl+click follows a link; on a page that cannot be edited, so
        // does a plain click.
        if (key_down(VK_CONTROL.0) || self.read_only) && self.follow_link(at, py as f32 / scale) {
            return;
        }
        let clicks = self.count_click(px, py);
        let shift = key_down(VK_SHIFT.0);
        match clicks {
            2 => self.editor.select_word_at(at),
            3 => self.editor.select_line_at(at),
            _ => {
                let anchor = if shift { self.editor.selection().anchor } else { at };
                self.editor.set_selection(Selection { anchor, head: at });
                if !shift {
                    self.snap_to_picture();
                }
            }
        }
        // Dragging extends a click's selection; after a double or triple
        // click it would shrink the word or line again.
        self.selecting = clicks == 1;
        if self.selecting {
            unsafe { SetCapture(self.hwnd) };
        }
        self.caret_moved(clicks == 1 && !shift);
    }

    fn note_drag(&mut self, px: i32, py: i32) {
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let at = self.offset_at(px as f32 / scale, py as f32 / scale);
        let s = self.editor.selection();
        if at != s.head {
            self.editor.set_selection(Selection { anchor: s.anchor, head: at });
            self.caret_moved(false);
        }
    }

    /// A drag offset for the preview that keeps it in the window (as the
    /// card will be drawn: see `social::card_bounds`).
    fn keep_social_in(&self, offset: (f32, f32)) -> (f32, f32) {
        let (w, h, dpi) = window::view(self.hwnd);
        let scale = dpi as f32 / 96.0;
        let (width, height) = (w as f32 / scale, h as f32 / scale);
        let home = crate::social::card_bounds(width, height, (0.0, 0.0));
        let at = crate::social::card_bounds(width, height, offset);
        (at.left - home.left, at.top - home.top)
    }

    /// Opens the social preview for `platform`, or closes it.
    pub(super) fn show_social(&mut self, platform: Option<kolada_core::social::Platform>) {
        if platform != self.social {
            self.social_scroll = 0.0;
        }
        self.social = platform;
        if platform.is_some() {
            self.close_popup();
        }
        self.request_frame();
    }

    fn wheel(&mut self, notches: f32) {
        // Scrolling sends the selection bar away.
        self.dismiss_selbar();
        // Over the social preview, it scrolls the post.
        if self.social.is_some() && matches!(self.ui.hover, Some(Target::SocialCard | Target::SocialHeader | Target::SocialTab(_) | Target::SocialClose)) {
            self.social_scroll = (self.social_scroll - notches * WHEEL_STEP).clamp(0.0, self.social_scroll_max);
            return self.request_frame();
        }
        // The options scroll where the window is too short for them.
        if self.ui.options_open {
            self.ui.options_scroll = (self.ui.options_scroll - notches * WHEEL_STEP).clamp(0.0, self.options_scroll_max);
            return self.request_frame();
        }
        // Over the side menu, the wheel scrolls the file tree, a row at a time.
        if self.ui.hover.and_then(Target::panel) == Some(Panel::Sidebar) {
            if self.search.is_none() {
                self.tree_scroll -= notches * 3.0;
                let rows = self.tree_scroll.trunc();
                self.tree_scroll -= rows;
                let max = self.session.sidebar.len().saturating_sub(self.tree_rows) as f32;
                self.ui.tree_top = (self.ui.tree_top as f32 + rows).clamp(0.0, max) as usize;
                self.request_frame();
            }
            return;
        }
        if matches!(
            self.ui.hover,
            Some(
                Target::PopupCard
                    | Target::PopupHeader
                    | Target::PopupText
                    | Target::PopupOpen
                    | Target::PopupClose
                    | Target::PopupCheckbox(_)
                    | Target::PopupScrollbar
            )
        ) {
            return self.scroll_popup(notches);
        }
        // The wheel lets an open Today card go, and scrolls the page.
        self.wheel_agenda();
        self.scroll_pending -= notches * WHEEL_STEP;
        self.scrolled_at = Some(Instant::now());
        self.request_frame();
    }

    /// Keeps the top of the view on a paragraph start after the text
    /// changed under it.
    fn fix_scroll_line(&mut self) {
        let text = self.scrub_page().map_or(self.editor.text(), |(t, _)| t);
        let mut at = (self.session.scroll_line as usize).min(text.len());
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        self.session.scroll_line = text[..at].rfind('\n').map_or(0, |i| i + 1) as u64;
    }

    /// Scrolls by `delta` DIPs, paragraph by paragraph, stopping at the top
    /// and where the note's end reaches the bottom of the window. False if
    /// it could not move.
    fn scroll_by(&mut self, delta: f32) -> bool {
        self.fix_scroll_line();
        let before = (self.session.scroll_line, self.session.scroll_px);
        let (mut line, mut px) = (self.session.scroll_line as usize, self.session.scroll_px + delta);
        {
            let nt = self.note_text();
            loop {
                if px < 0.0 {
                    if line == 0 {
                        px = 0.0;
                        break;
                    }
                    let Ok(prev) = nt.para_at(line - 1) else { break };
                    line = prev.start;
                    px += nt.block_height(&prev).unwrap_or(0.0);
                } else {
                    let Ok(p) = nt.para_at(line) else { break };
                    let h = nt.block_height(&p).unwrap_or(0.0);
                    if p.end >= nt.text.len() {
                        px = px.min((h - PAGE_LINE).max(0.0));
                        break;
                    }
                    if px < h {
                        break;
                    }
                    px -= h;
                    line = p.end + 1;
                }
            }
        }
        if delta > 0.0 {
            let end = self.scroll_end();
            if (line, px) > end {
                (line, px) = end;
            }
        }
        self.session.scroll_line = line as u64;
        self.session.scroll_px = px;
        (self.session.scroll_line, self.session.scroll_px) != before
    }

    /// Brings the caret into view: eased when it is just outside, at once
    /// when it is far away. True if the view jumped and must be laid out
    /// again.
    fn scroll_to_caret(&mut self, scene: &Scene, (_, height, dpi): (u32, u32, u32)) -> bool {
        let top = scene.column.top;
        let bottom = height as f32 * 96.0 / dpi as f32 - CARET_MARGIN;
        let head = self.editor.selection().head;
        if let Some(c) = scene.caret {
            self.scroll_pending = if c.top < top {
                c.top - top
            } else if c.bottom > bottom {
                c.bottom - bottom
            } else {
                0.0
            };
            if self.scroll_pending != 0.0 {
                self.request_frame();
            }
            return false;
        }
        // Off screen: the caret's line goes to the top when it is above the
        // view, and to the bottom when below.
        let target = {
            let nt = self.note_text();
            let Ok(p) = nt.para_at(head) else { return false };
            let Ok((_, y, h)) = nt.caret(&p, head) else { return false };
            let above = (head as u64) < self.session.scroll_line;
            (p.start, if above { p.pad + y } else { p.pad + y + h - (bottom - top) })
        };
        self.session.scroll_line = target.0 as u64;
        self.session.scroll_px = 0.0;
        self.scroll_pending = 0.0;
        self.scroll_by(target.1);
        true
    }

    // Saving, opening, and fresh pages.

    /// Where notes go when no vault was ever opened: Documents\Kolada.
    fn ensure_vault(&mut self) -> Option<PathBuf> {
        if self.session.vault.is_empty() {
            let dir = kolada_core::default_vault_dir()?;
            std::fs::create_dir_all(&dir).ok()?;
            self.session.vault = dir.to_string_lossy().into_owned();
            self.watch_vault();
        }
        Some(PathBuf::from(&self.session.vault))
    }

    fn refresh_tree(&self) {
        let mut open = vault::expanded_folders(&self.session.sidebar);
        open.extend(self.session.expanded.iter().cloned());
        crate::refresh_sidebar(self.hwnd, self.session.vault.clone(), open);
    }

    /// Writes the note if it changed. A fresh page becomes a file once it
    /// has words. The file's name follows the note's title; the rename waits
    /// until the caret leaves the title line or the note is left (`flush`),
    /// not every pause in typing a title.
    fn save_note(&mut self, flush: bool) {
        self.stop_timer(AUTOSAVE_TIMER);
        // A word half composed is not saved: later, or settled first when
        // the note is left.
        if self.editor.marked().is_some() {
            if !flush {
                return self.start_timer(AUTOSAVE_TIMER, AUTOSAVE_DELAY_MS);
            }
            self.settle_composition();
        }
        if self.read_only {
            return;
        }
        // Leaving an edited note tidies it: the spaces and empty lines nobody meant.
        if flush && self.editor.version() != self.saved_version {
            self.editor.tidy();
        }
        // A task's @fri becomes @Fri 9 Oct 2026 once its line is left, so
        // it stays this Friday (the line being typed in waits).
        if self.editor.version() != self.saved_version {
            let typing = (!flush).then(|| self.editor.line_at(self.editor.selection().head).start);
            let edits = kolada_core::tasks::settle(self.editor.text(), daily::today(), typing);
            if !edits.is_empty() {
                self.editor.amend(&edits);
            }
        }
        let blank = self.editor.text().trim().is_empty();
        if self.session.note.is_empty() {
            if blank {
                return;
            }
            let Some(vault) = self.ensure_vault() else { return };
            let dir = self.fresh_dir.clone().filter(|d| d.is_dir() && d.starts_with(&vault)).unwrap_or(vault.clone());
            let path = note::unique_path(&dir, &note::file_stem_for(&self.editor.title()), None);
            let Some(rel) = vault::relative(&vault, &path) else { return };
            self.session.remember(&rel);
            self.session.note = rel;
            self.session.captured = true;
            self.named = self.editor.title();
            self.disk = None;
            self.write();
            self.refresh_tree();
            self.whisper(format!("Saved as {}", stem(&path)));
            return;
        }
        if self.session.captured && blank {
            // Everything was deleted: the capture is undone, and the page is
            // fresh again (undo still brings the text back).
            if flush {
                let _ = std::fs::remove_file(self.note_path());
                self.session.start_fresh();
                self.saved_version = self.editor.version();
                self.disk = None;
                self.refresh_tree();
            }
            return;
        }
        // Its past: what the file said before this run's first save, then
        // versions as it is written (see `scrub`).
        let wrote = self.editor.version() != self.saved_version;
        if wrote {
            self.keep_base();
            self.write();
        }
        if wrote || (flush && self.kept.0 == self.session.note) {
            self.keep_version(flush);
        }
        let title = self.editor.title();
        if title != self.named && !title.is_empty() && (flush || !on_title(&self.editor)) {
            self.named = title.clone();
            let from = self.note_path();
            if let Some(to) = self.rename_note(&from, &title)
                && let Some(rel) = vault::relative(Path::new(&self.session.vault), &to)
            {
                let was = std::mem::take(&mut self.session.note);
                self.session.renamed(&was, &rel);
                self.session.note = rel;
                self.disk = stamp(&to);
            }
        }
    }

    /// Renames the note at `path` after `title`, and points the vault's
    /// links to it at the new name: in other notes' files, and in the page
    /// and the popup as they are being edited. Returns where it went.
    fn rename_note(&mut self, path: &Path, title: &str) -> Option<PathBuf> {
        let dir = path.parent()?;
        let wanted = note::unique_path(dir, &note::file_stem_for(title), Some(path));
        if wanted == path || std::fs::rename(path, &wanted).is_err() {
            return None;
        }
        // Its past goes with it.
        let vault_dir = Path::new(&self.session.vault);
        if let (Some(data), Some(from), Some(to)) = (kolada_core::app_data_dir(), vault::relative(vault_dir, path), vault::relative(vault_dir, &wanted)) {
            let _ = kolada_core::history::rename(&kolada_core::history::dir(&data, vault_dir), &from, &to);
            if self.kept.0 == from {
                self.kept.0 = to;
            }
        }
        let (old, new) = (stem(path), stem(&wanted));
        let mut updated = 0;
        let needle = old.to_lowercase();
        let vault = PathBuf::from(&self.session.vault);
        let open = [self.note_path(), self.popup.as_ref().map_or_else(PathBuf::new, |p| p.path.clone())];
        for n in &self.index {
            let file = vault.join(&n.path);
            if !search::is_note(&n.path) || open.contains(&file) || file == wanted || !n.text().contains("[[") || !n.text().to_lowercase().contains(&needle) {
                continue;
            }
            let Ok((text, format)) = note::load(&file) else { continue };
            let changes = links::renamed(&text, &old, &new);
            if !changes.is_empty() && note::save(&file, &links::apply(&text, &changes), format).is_ok() {
                updated += 1;
            }
        }
        // The notes being edited change as edits, so undo takes them back.
        let changes = links::renamed(self.editor.text(), &old, &new);
        if !changes.is_empty() && !self.read_only {
            self.editor.replace_ranges(&changes);
            self.start_timer(AUTOSAVE_TIMER, AUTOSAVE_DELAY_MS);
            updated += 1;
        }
        if let Some(p) = &mut self.popup {
            let changes = links::renamed(p.editor.text(), &old, &new);
            if !changes.is_empty() {
                p.editor.replace_ranges(&changes);
                self.start_timer(POPUP_SAVE_TIMER, AUTOSAVE_DELAY_MS);
                updated += 1;
            }
        }
        self.refresh_tree();
        // Search and related notes know it by its new name.
        self.start_index();
        self.whisper(match updated {
            0 => format!("Renamed to {new}"),
            1 => format!("Renamed to {new} \u{00B7} 1 link updated"),
            n => format!("Renamed to {new} \u{00B7} {n} links updated"),
        });
        Some(wanted)
    }

    /// Says quietly, at the top of the page, what just happened on its own.
    /// The update pill, clicked: everything saved, the new program put in
    /// place, and Kolada closes to start it on the same note (see `updates`).
    fn install_update(&mut self) {
        self.save_popup(true);
        self.save_note(true);
        self.save_session();
        match crate::updates::put_in_place() {
            Ok(_) => {
                crate::updates::RELAUNCH.store(true, std::sync::atomic::Ordering::Relaxed);
                self.quit();
            }
            Err(_) => {
                self.update = None;
                self.whisper("The update could not go in; Kolada stays as it is".to_string());
            }
        }
    }

    fn whisper(&mut self, text: String) {
        self.whisper = Some((text, Instant::now()));
        // Wakes the frames again for the fade out.
        self.start_timer(WHISPER_TIMER, (WHISPER - WHISPER_OUT).as_millis() as u32);
        self.request_frame();
    }

    /// The pointer rests on chip `i` (or on none): after a moment, peek at
    /// its note.
    fn hover_chip(&mut self, chip: Option<usize>) {
        if self.peek.as_ref().map(|p| p.chip) == chip && chip.is_some() {
            return;
        }
        if self.peek.take().is_some() {
            self.request_frame();
        }
        self.peek_pending = chip;
        match chip {
            Some(_) => self.start_timer(PEEK_TIMER, PEEK_DELAY_MS),
            None => self.stop_timer(PEEK_TIMER),
        }
    }

    fn show_peek(&mut self) {
        self.stop_timer(PEEK_TIMER);
        let Some(i) = self.peek_pending.take() else { return };
        if self.ui.hover != Some(Target::Chip(i)) || self.chip_drag.is_some() {
            return;
        }
        let Some(chip) = self.chips.get(i) else { return };
        let Some(note) = self.index.iter().find(|n| n.path == chip.rel.path) else { return };
        let body = opening(note.text());
        self.peek = Some(PeekState { chip: i, since: Instant::now(), title: chip.rel.name.clone(), body });
        self.request_frame();
    }

    fn write(&mut self) {
        let mut path = self.note_path();
        // Another app changed the file since it was read or saved: keep
        // both versions rather than overwrite theirs.
        if let (Some(was), Some(now)) = (self.disk, stamp(&path))
            && was != now
            && let Some(dir) = path.parent()
        {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            path = note::unique_path(dir, &format!("{stem} (conflict)"), None);
            if let Some(rel) = vault::relative(Path::new(&self.session.vault), &path) {
                self.session.note = rel;
                self.session.captured = false;
                self.refresh_tree();
            }
        }
        let text = file_text(self.editor.text(), &mut self.added).into_owned();
        if note::save(&path, &text, self.format).is_ok() {
            self.saved_version = self.editor.version();
            self.disk = stamp(&path);
            self.read_times();
            self.written_to_top(&path);
        }
    }

    /// A note just written goes to the top of its folder in the side menu,
    /// as its newest.
    fn written_to_top(&mut self, path: &Path) {
        let Some(rel) = vault::relative(Path::new(&self.session.vault), path) else { return };
        if vault::written(&mut self.session.sidebar, rel.strip_suffix(".md").unwrap_or(&rel)) {
            self.request_frame();
        }
    }

    /// Picks up edits another app made while this one was in the
    /// background, if there is nothing unsaved here.
    fn check_disk(&mut self) {
        if self.session.note.is_empty() || self.editor.version() != self.saved_version {
            return;
        }
        let path = self.note_path();
        let now = stamp(&path);
        if now.is_none() || now == self.disk {
            return;
        }
        if let Some((text, format, added)) = read_file(&path) {
            if text != self.editor.text() {
                let caret = outside::map_caret(self.editor.text(), &text, self.editor.selection().head);
                self.load((text, format, added), caret, false);
                self.whisper("Updated with edits made in another app".to_string());
            }
            self.disk = now;
        }
    }

    /// Shows `text` as the note. With `fade`, it fades in: another note was
    /// opened.
    fn load(&mut self, (text, format, added): Opened, caret: usize, fade: bool) {
        if fade {
            self.page_opened = Some(Instant::now());
        }
        self.scrub = None;
        self.scrub_moving = false;
        self.editor = Editor::new(text);
        self.editor.set_clean(!self.session.show_marks);
        self.editor.set_snippets(daily::snippets());
        self.editor.set_selection(Selection::caret(caret.min(self.editor.text().len())));
        // Versions start over with the new text: its words are counted anew.
        self.words.set((u64::MAX, 0));
        self.format = format;
        self.added = added;
        self.named = self.editor.title();
        self.read_times();
        self.saved_version = self.editor.version();
        self.disk = if self.session.note.is_empty() { None } else { stamp(&self.note_path()) };
        self.scroll_pending = 0.0;
        self.fix_scroll_line();
        self.caret_moved(false);
        self.note_opened();
    }

    /// Zero-step capture: an empty page, ready to type. It becomes a note
    /// when it has words.
    fn fresh_page(&mut self) {
        self.close_popup();
        self.save_note(true);
        if !self.session.note.is_empty() {
            self.remember_visit();
        }
        self.read_only = false;
        self.fresh_dir = None;
        self.session.start_fresh();
        self.session.last_active = session::now();
        self.load(Opened::default(), 0, true);
        self.update_related();
    }

    /// The welcome note opens in its new vault, unless writing already
    /// began on the fresh page: then it waits in the side menu.
    fn welcome(&mut self, note: &Path) {
        let Some(vault) = note.parent() else { return };
        let fresh = self.session.vault.is_empty() && self.session.note.is_empty();
        if fresh && self.editor.text().is_empty() {
            self.open(note);
        } else if self.session.vault.is_empty() {
            self.session.vault = vault.to_string_lossy().into_owned();
            self.refresh_tree();
        }
        self.watch_vault();
    }

    fn open(&mut self, path: &Path) {
        // A note in the open vault stays in it, even without `.obsidian`.
        let root = if !self.session.vault.is_empty() && path.starts_with(&self.session.vault) {
            PathBuf::from(&self.session.vault)
        } else {
            vault::root_for(path)
        };
        let Some(rel) = vault::relative(&root, path) else { return };
        self.close_popup();
        self.save_note(true);
        if !self.session.note.is_empty() && self.note_path() != path {
            self.remember_visit();
        }
        self.read_only = false;
        let changed_vault = self.session.vault != root.to_string_lossy();
        if changed_vault {
            self.session.recent.clear();
        } else {
            // The note left is recent too, however it was opened.
            let left = self.session.note.clone();
            self.session.remember(&left);
        }
        self.session.vault = root.to_string_lossy().into_owned();
        self.session.remember(&rel);
        self.session.note = rel;
        self.session.scroll_line = 0;
        self.session.scroll_px = 0.0;
        self.session.captured = false;
        self.session.last_active = session::now();
        if changed_vault {
            self.session.sidebar.clear();
            self.refresh_tree();
        }
        let opened = read_note(&self.session);
        self.load(opened, 0, true);
        let at = body_start(&self.editor);
        self.editor.set_selection(Selection::caret(at));
        if changed_vault {
            self.corpus = None;
            self.start_index();
        }
        self.update_related();
    }


    /// The open note goes on the way back (a new step forgets the way
    /// forward).
    /// A favorite: its note opened, or its folder opened out in the tree and
    /// scrolled to; one no longer there is let go.
    fn open_favorite(&mut self, i: usize) {
        let Some(fav) = self.session.favorites.get(i).cloned() else { return };
        let vault = PathBuf::from(&self.session.vault);
        let Some(folder) = fav.strip_suffix('/') else {
            let path = vault.join(&fav);
            if path.is_file() {
                return self.open(&path);
            }
            self.session.favorites.retain(|f| *f != fav);
            return self.whisper("That note is not there any more".to_string());
        };
        if !vault.join(folder).is_dir() {
            self.session.favorites.retain(|f| *f != fav);
            return self.whisper("That folder is not there any more".to_string());
        }
        // Each folder on the way, opened out in turn.
        let mut at = None;
        let mut path = String::new();
        for part in folder.split('/') {
            path = if path.is_empty() { part.to_string() } else { format!("{path}/{part}") };
            let paths = vault::row_paths(&self.session.sidebar);
            let Some(i) = paths.iter().position(|p| *p == path) else { break };
            if matches!(self.session.sidebar[i].kind, RowKind::Folder { expanded: false }) {
                self.click_row(i);
            }
            at = Some(i);
        }
        if let Some(i) = at {
            let max = self.session.sidebar.len().saturating_sub(self.tree_rows);
            self.ui.tree_top = i.min(max);
        }
        self.request_frame();
    }

    /// Adds a note or folder to the favorites, or takes it off them.
    pub(super) fn toggle_favorite(&mut self, path: &str) {
        let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path).trim_end_matches(".md").to_string();
        let kept = self.session.toggle_favorite(path);
        self.whisper(if kept { format!("{name} is in Favorites") } else { format!("{name} is out of Favorites") });
        self.request_frame();
    }

    /// A note under Recent: opened, or let go if it is not there any more.
    fn open_recent(&mut self, i: usize) {
        let Some(rel) = self.session.recent.get(i).cloned() else { return };
        let path = Path::new(&self.session.vault).join(&rel);
        if path.is_file() {
            self.open(&path);
        } else {
            self.session.recent.retain(|n| *n != rel);
            self.whisper("That note is not there any more".to_string());
            self.request_frame();
        }
    }

    fn remember_visit(&mut self) {
        let visit = self.visit();
        if self.stepping {
            return;
        }
        self.back.push(visit);
        if self.back.len() > HISTORY {
            self.back.remove(0);
        }
        self.ahead.clear();
    }

    fn visit(&self) -> Visit {
        Visit {
            path: self.note_path(),
            scroll: (self.session.scroll_line, self.session.scroll_px),
            caret: self.editor.selection().head,
        }
    }

    /// Back to the note left before (`forward`: the one gone back from),
    /// where it was left: Alt+Left and Alt+Right, the mouse's side buttons.
    fn step(&mut self, forward: bool) {
        loop {
            let next = if forward { self.ahead.pop() } else { self.back.pop() };
            let Some(v) = next else { return };
            // A note deleted or renamed since is skipped.
            if !v.path.is_file() {
                continue;
            }
            let here = (!self.session.note.is_empty()).then(|| self.visit());
            self.stepping = true;
            self.open(&v.path);
            self.stepping = false;
            if let Some(here) = here {
                if forward { self.back.push(here) } else { self.ahead.push(here) }
            }
            let len = self.editor.text().len();
            self.editor.set_selection(Selection::caret(v.caret.min(len)));
            self.session.scroll_line = v.scroll.0.min(len as u64);
            self.session.scroll_px = v.scroll.1;
            self.fix_scroll_line();
            self.caret_moved(false);
            self.reveal_caret = false;
            return;
        }
    }

    /// Another folder as the vault: its first note opens, or a fresh page.
    fn pick_vault(&mut self) {
        let Some(dir) = crate::dialog::pick_folder(self.hwnd, "Open a folder of notes") else { return };
        self.ui.options_open = false;
        self.close_popup();
        self.close_trash();
        self.close_find();
        if self.search.is_some() {
            self.close_search();
        }
        self.save_note(true);
        self.read_only = false;
        self.session.vault = dir.to_string_lossy().into_owned();
        self.session.sidebar.clear();
        self.session.recent.clear();
        self.session.start_fresh();
        self.session.last_active = session::now();
        match vault::first_note(&dir) {
            Some(rel) => {
                self.session.note = rel;
                let opened = read_note(&self.session);
                self.load(opened, 0, true);
                let at = body_start(&self.editor);
                self.editor.set_selection(Selection::caret(at));
            }
            None => self.load(Opened::default(), 0, true),
        }
        self.refresh_tree();
        self.corpus = None;
        self.index.clear();
        self.start_index();
        self.watch_vault();
        self.update_related();
        self.whisper(format!("Opened {}", vault_name(&self.session.vault)));
    }

    /// Highlight to remember: every marked passage in the vault, gathered on
    /// one page, each under a link to its note.
    fn open_highlights(&mut self) {
        let Some(corpus) = &self.corpus else { return };
        let text = kolada_core::related::highlights_page(&corpus.highlights());
        self.save_note(true);
        self.close_search();
        self.session.start_fresh();
        self.read_only = true;
        self.load((text, Format::default(), None), 0, true);
        self.update_related();
    }

    // The chrome.

    /// The target under a client-area point in pixels.
    /// What is under the pointer now, by the last frame's hits: Some(None)
    /// when it is in the window over nothing, None when it is outside it.
    fn under_pointer(&self) -> Option<Option<Target>> {
        let mut p = POINT::default();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p).ok()?;
            let _ = ScreenToClient(self.hwnd, &mut p);
        }
        let (w, h, _) = window::view(self.hwnd);
        (p.x >= 0 && p.y >= 0 && (p.x as u32) < w && (p.y as u32) < h).then(|| self.hit_px(p.x, p.y))
    }

    fn hit_px(&self, x: i32, y: i32) -> Option<Target> {
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        scene::hit(&self.hits, x as f32 / scale, y as f32 / scale)
    }

    fn collapsed(&self, panel: Panel) -> bool {
        match panel {
            Panel::Sidebar => self.session.sidebar_collapsed || self.narrow || self.session.minimal,
            Panel::Toolbar => self.session.toolbar_collapsed || self.session.minimal,
        }
    }

    /// The window was resized: past the width the side menu docks at, it
    /// tucks away or comes back, at once (an edge being dragged leaves no
    /// frames for an animation).
    fn resized(&mut self) {
        let (w, _, dpi) = window::view(self.hwnd);
        let narrow = !chrome::sidebar_fits(w as f32 * 96.0 / dpi as f32);
        if narrow == self.narrow {
            return;
        }
        self.narrow = narrow;
        self.revealed[0] = false;
        self.settle_panels();
    }

    /// Every panel where its state puts it, at once.
    fn settle_panels(&mut self) {
        self.animation = None;
        for panel in PANELS {
            let i = index(panel);
            let collapsed = self.collapsed(panel);
            self.ui.dock[i] = if collapsed { 0.0 } else { 1.0 };
            self.ui.show[i] = if !collapsed || self.revealed[i] { 1.0 } else { 0.0 };
        }
    }

    /// Makes the window minimal (the note alone, small, where it was last
    /// minimal) or full again (where it was), at once. A note's own window
    /// is neither.
    pub(super) fn set_minimal(&mut self, on: bool) {
        if on == self.session.minimal || self.compact {
            return;
        }
        // Where it is now is kept, for coming back.
        self.save_session();
        self.session.minimal = on;
        let to = if on { window::minimal_placement(&self.session.minimal_window, &self.session.window) } else { window::placement(&self.session.window) };
        self.menu = None;
        self.revealed = [false; 2];
        window::place(self.hwnd, &to);
        window::keep_on_top(self.hwnd, on);
        // Windows sends the new size while this runs, when the app is not
        // free to take it (`with_app`): read it now, or a window that was
        // narrow while minimal stays narrow, and the side menu never pins.
        self.resized();
        self.settle_panels();
        self.save_session();
        self.request_frame();
    }

    /// Animates every panel toward where its docked/revealed state puts it.
    fn retarget(&mut self, duration: Duration) {
        let mut to = [0.0; 4];
        for panel in PANELS {
            let i = index(panel);
            let collapsed = self.collapsed(panel);
            to[i] = if collapsed { 0.0 } else { 1.0 };
            to[2 + i] = if !collapsed || self.revealed[i] { 1.0 } else { 0.0 };
        }
        let from = self.ui.values();
        if from != to {
            self.animation = Some(Animation { start: Instant::now(), duration, from, to });
        }
        self.request_frame();
    }

    /// Docks a collapsed panel, or collapses a docked one into focus mode.
    /// Where the side menu cannot dock, it shows or hides, floating.
    fn toggle(&mut self, panel: Panel) {
        let i = index(panel);
        if panel == Panel::Sidebar && self.narrow {
            self.revealed[0] = !self.revealed[0];
            self.held_back[0] = !self.revealed[0];
            return self.retarget(REVEAL_ANIMATION);
        }
        let collapsed = !self.collapsed(panel);
        match panel {
            Panel::Sidebar => self.session.sidebar_collapsed = collapsed,
            Panel::Toolbar => {
                self.session.toolbar_collapsed = collapsed;
                self.close_reading();
            }
        }
        self.revealed[i] = false;
        self.held_back[i] = collapsed;
        self.retarget(DOCK_ANIMATION);
    }

    fn start_timer(&mut self, id: usize, ms: u32) {
        unsafe { SetTimer(Some(self.hwnd), id, ms, None) };
    }

    fn stop_timer(&mut self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), id);
        }
    }

    /// Reveals collapsed panels the pointer rests near, and hides revealed
    /// panels it has left. Runs on every pointer move and leave.
    fn update_reveal(&mut self) {
        for panel in PANELS {
            let i = index(panel);
            let over = self.ui.hover.and_then(Target::panel) == Some(panel);
            if !over {
                self.held_back[i] = false;
            }
            let wants_reveal = self.collapsed(panel) && over && !self.revealed[i] && !self.held_back[i];
            if wants_reveal && !self.reveal_pending[i] {
                self.reveal_pending[i] = true;
                self.start_timer(reveal_timer(panel), REVEAL_DELAY_MS);
            } else if !wants_reveal && self.reveal_pending[i] {
                self.reveal_pending[i] = false;
                self.stop_timer(reveal_timer(panel));
            }
            // Searching or choosing options keeps the menu out.
            let busy = panel == Panel::Sidebar
                && (self.search.is_some() || self.ui.options_open || self.exited_left || self.menu.as_ref().is_some_and(|m| m.row.is_some()));
            let wants_hide = self.revealed[i] && !over && !busy;
            if wants_hide && !self.hide_pending[i] {
                self.hide_pending[i] = true;
                self.start_timer(hide_timer(panel), HIDE_DELAY_MS);
            } else if !wants_hide && self.hide_pending[i] {
                self.hide_pending[i] = false;
                self.stop_timer(hide_timer(panel));
            }
        }
    }

    /// Handles a timer; false for timers that are not ours.
    fn on_timer(&mut self, id: usize) -> bool {
        if self.assist_timer(id) {
            return true;
        }
        if id == agenda::REMIND_TIMER {
            self.remind();
            return true;
        }
        if id == POPUP_SAVE_TIMER {
            self.save_popup(false);
            return true;
        }
        if id == PEEK_TIMER {
            self.show_peek();
            return true;
        }
        if id == link::LINK_TIMER {
            self.link_hover_rested();
            return true;
        }
        if id == bars::TOOLTIP_TIMER {
            self.tooltip_rested();
            return true;
        }
        if id == WHISPER_TIMER {
            self.stop_timer(WHISPER_TIMER);
            self.request_frame();
            return true;
        }
        if id == WORDS_TIMER {
            // Typing paused: the date line counts the words again, and the
            // gist comes back.
            self.stop_timer(WORDS_TIMER);
            if self.words.get().0 != self.editor.version() {
                self.request_frame();
            }
            self.gist_pause();
            return true;
        }
        if id == RELATED_TIMER {
            self.update_related();
            return true;
        }
        if id == AUTOSAVE_TIMER {
            self.save_note(false);
            self.refresh_due();
            return true;
        }
        for panel in PANELS {
            let i = index(panel);
            if id == reveal_timer(panel) {
                self.stop_timer(id);
                self.reveal_pending[i] = false;
                let over = self.ui.hover.and_then(Target::panel) == Some(panel);
                if over && self.collapsed(panel) {
                    self.revealed[i] = true;
                    self.retarget(REVEAL_ANIMATION);
                }
                return true;
            }
            if id == hide_timer(panel) {
                self.stop_timer(id);
                self.hide_pending[i] = false;
                // Where the pointer is now, against the panel as drawn now: a
                // pointer that outran the panel sliding in is over it by now.
                if let Some(now) = self.under_pointer() {
                    self.ui.hover = now;
                }
                if self.ui.hover.and_then(Target::panel) != Some(panel) {
                    self.revealed[i] = false;
                    if panel == Panel::Toolbar {
                        self.close_reading();
                    }
                    self.retarget(REVEAL_ANIMATION);
                }
                return true;
            }
        }
        false
    }

    /// The pointer moved to a client point in pixels.
    fn pointer_moved(&mut self, x: i32, y: i32) {
        self.exited_left = false;
        let scale = window::dpi(self.hwnd) as f32 / 96.0;
        let p = (x as f32 / scale, y as f32 / scale);
        let target = scene::hit(&self.hits, p.0, p.1);
        let before = self.ui.pointer.replace(p);
        self.link_hover_moved(target == Some(Target::Note), p);
        if target != self.ui.hover {
            self.ui.hover = target;
            self.request_frame();
            self.tooltip_hover(target);
            let row = match target {
                Some(Target::Row(i) | Target::RowMenu(i)) => Some(i),
                _ => None,
            };
            if row != self.row_date.as_ref().map(|r| r.0) {
                self.row_date = row.and_then(|i| {
                    self.session.sidebar.get(i).filter(|r| r.kind == RowKind::Note)?;
                    let rel = vault::row_paths(&self.session.sidebar).get(i)?.clone();
                    let modified = std::fs::metadata(Path::new(&self.session.vault).join(format!("{rel}.md"))).ok()?.modified().ok()?;
                    Some((i, crate::when::ago(modified)))
                });
            }
            let chip = match target {
                Some(Target::Chip(i)) => Some(i),
                _ => None,
            };
            self.hover_chip(chip);
        }
        // Collapsed pills feel the pointer: wake their magnets when it is,
        // or just was, within reach of one.
        if self.near_collapsed_pill(p) || before.is_some_and(|b| self.near_collapsed_pill(b)) {
            self.magnets_moving = true;
        }
        self.update_reveal();
    }

    /// Steps each pill's string toward the wave the pointer pulls it into;
    /// true while either still moves.
    fn step_magnets(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for (i, vertical) in [(0, true), (1, false)] {
            let (target, length) = match self.pills[i] {
                Some(pill) => (wave::target(pill, vertical, self.ui.pointer), if vertical { pill.height() } else { pill.width() }),
                None => ([0.0; SAMPLES], 100.0),
            };
            moving |= self.waves[i].step(dt, length, &target);
            self.ui.wave[i] = self.waves[i].d;
        }
        moving
    }

    fn near_collapsed_pill(&self, (x, y): (f32, f32)) -> bool {
        let reach = wave::PROXIMITY + 16.0;
        (self.collapsed(Panel::Sidebar) && x < reach) || (self.session.toolbar_collapsed && y < reach)
    }

    fn pointer_left(&mut self) {
        self.hover_chip(None);
        self.hide_link_card();
        self.tooltip_away();
        // Reaching for the side menu and overshooting the window's edge
        // still means the menu.
        if self.collapsed(Panel::Sidebar) && self.ui.pointer.is_some_and(|(x, _)| x < OVERSHOOT) {
            self.exited_left = true;
            if !self.revealed[0] {
                self.revealed[0] = true;
                self.held_back[0] = false;
                self.retarget(REVEAL_ANIMATION);
            }
        }
        self.tracking = false;
        self.ui.pointer = None;
        self.magnets_moving = true;
        self.ui.hover = None;
        self.request_frame();
        self.update_reveal();
    }

    /// Responds to a click on `target`.
    fn activate(&mut self, target: Target) {
        let hwnd = self.hwnd;
        match target {
            Target::PageDate => self.toggle_scrub(),
            Target::DueDot => self.open_agenda(),
            Target::Recent(i) => self.open_recent(i),
            Target::Favorite(i) => self.open_favorite(i),
            Target::FoldFavorites => {
                self.session.folded ^= session::FOLD_FAVORITES;
                self.request_frame();
            }
            Target::FoldRecent => {
                self.session.folded ^= session::FOLD_RECENT;
                self.request_frame();
            }
            Target::AgendaShade => self.close_agenda(),
            Target::AgendaPanel => {}
            Target::AgendaOpen(i) => self.go_to_due(i),
            Target::AgendaTick(i) => self.tick_due(i),
            Target::ScrubClose => self.close_scrub(),
            Target::ScrubRestore => self.restore_version(),
            Target::BringBack(i) => self.bring_back_line(i),
            Target::ScrubPanel | Target::ScrubTrack => {}
            Target::Minimize => unsafe {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            },
            Target::Maximize => unsafe {
                let zoomed = IsZoomed(hwnd).as_bool();
                let _ = ShowWindow(hwnd, if zoomed { SW_RESTORE } else { SW_MAXIMIZE });
            },
            Target::Close => unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            // The pill, the collapse and pin buttons, and clicks in a reveal
            // area all dock or undock.
            Target::ToggleSidebar | Target::RevealSidebar => self.toggle(Panel::Sidebar),
            Target::ToggleToolbar | Target::RevealToolbar => self.toggle(Panel::Toolbar),
            Target::NewNote => self.fresh_page(),
            Target::Update => self.install_update(),
            // The installer's own window (see `setup`).
            Target::Setup(_) => {}
            Target::Share(hit) => self.share_click(hit),
            Target::Model(hit) => self.model_click(hit),
            Target::Highlights => self.open_highlights(),
            Target::Trash => {
                if self.trash.is_some() {
                    self.close_trash();
                } else {
                    self.open_trash();
                }
            }
            Target::TrashBack => self.close_trash(),
            Target::TrashRow(i) | Target::TrashRestore(i) => self.restore(i),
            Target::TrashEmpty => self.empty_trash(),
            Target::RowMenu(i) => self.open_row_menu(i, None),
            Target::SendTo => self.quick_menu(target),
            Target::Transfer(what) => self.transfer(what),
            Target::FindField
            | Target::ReplaceField
            | Target::FindPrev
            | Target::FindNext
            | Target::FindReplace
            | Target::FindClose
            | Target::ReplaceOne
            | Target::ReplaceAll => self.find_click(target),
            Target::FindPanel => {}
            Target::Suggestion(i) => self.accept_suggestion(i),
            Target::LinkRemove | Target::LinkSuggestion(_) => self.link_click(target),
            Target::LinkPanel | Target::LinkField => {}
            Target::Heading(i) => {
                let line = self.headings_for_map().get(i).map(|h| h.2);
                if let Some(line) = line {
                    self.jump_to(line);
                }
            }
            Target::ToMain if self.session.minimal => self.set_minimal(false),
            Target::ToMain => self.back_to_main(),
            Target::Minimal => self.set_minimal(true),
            Target::MenuItem(i) => self.menu_action(i),
            Target::MenuPanel | Target::Scrollbar | Target::PopupScrollbar | Target::TreeScrollbar => {}
            Target::ThemeCycle => {
                // System, then light, then dark.
                let next = match self.session.theme {
                    0 => 1,
                    1 => 2,
                    _ => 0,
                };
                self.choose(Setting::Theme, next);
            }
            Target::TextSize(up) => {
                // A size smaller or larger; nothing past the ends.
                if let Some(next) = scene::step_text_size(self.session.text_size, up) {
                    self.choose(Setting::TextSize, next);
                }
            }
            Target::LineWidth => {
                // Narrow (1), default (0), wide (2), and around.
                let next = match self.session.line_width {
                    1 => 0,
                    0 => 2,
                    _ => 1,
                };
                self.choose(Setting::LineWidth, next);
            }
            Target::Width(value) => self.choose(Setting::LineWidth, value),
            Target::Reading => self.toggle_reading(),
            Target::SelBar(tool) => self.selbar_tool(tool),
            Target::ReadingPanel | Target::SelBarPanel => {}
            Target::Row(i) => self.click_row(i),
            // The past shown is not edited: the toolbar waits.
            Target::Tool(_) if self.scrub_page().is_some() => {}
            Target::Tool(tool) => self.tool(tool),
            Target::Picture(_) | Target::PictureEdge(_) | Target::SocialCard | Target::SocialHeader | Target::SidebarEdge => {}
            Target::SocialDrawer => {
                self.session.social = if self.session.social == 2 { 1 } else { 2 };
                self.request_frame();
            }
            // The drawer's platform opens its preview (again: closes it);
            // the card's tabs switch it.
            Target::Social(p) if self.social == Some(p) => self.show_social(None),
            Target::Social(p) | Target::SocialTab(p) => self.show_social(Some(p)),
            Target::SocialClose => self.show_social(None),
            Target::Checkbox(at) => {
                self.edit(|e| {
                    e.toggle_task(at);
                });
                // Checking a task off leaves the view where it is.
                self.reveal_caret = false;
            }
            Target::Search => self.open_search(),
            Target::Result(i) => self.open_hit(i),
            Target::Options => {
                self.ui.options_open = !self.ui.options_open;
                self.ui.options_scroll = 0.0;
                if self.search.is_some() {
                    self.close_search();
                }
                self.request_frame();
            }
            Target::Choice(setting, value) => self.choose(setting, value),
            Target::OptionsClose => {
                self.ui.options_open = false;
                self.request_frame();
            }
            Target::VaultOpen => self.pick_vault(),
            Target::VaultShow => {
                if !self.session.vault.is_empty() {
                    let _ = std::process::Command::new("explorer.exe").arg(&self.session.vault).spawn();
                }
            }
            Target::OptionsVeil => {}
            Target::OptionsPanel | Target::Chip(_) => {}
            Target::PopupClose => self.close_popup(),
            Target::PopupOpen => self.expand_popup(),
            Target::PopupCheckbox(at) => self.popup_edit(|e| {
                e.toggle_task(at);
            }),
            Target::PopupCard | Target::PopupHeader | Target::PopupText => {}
            Target::Caption | Target::SidebarPanel | Target::ToolbarPanel | Target::Note => {}
        }
    }

    fn click_row(&mut self, i: usize) {
        let Some(row) = self.session.sidebar.get(i).cloned() else { return };
        let path = vault::row_paths(&self.session.sidebar)[i].clone();
        let vault = PathBuf::from(&self.session.vault);
        match row.kind {
            RowKind::Note => self.open(&vault.join(format!("{path}.md"))),
            RowKind::Folder { expanded: true } => {
                vault::collapse(&mut self.session.sidebar, i);
                self.request_frame();
            }
            RowKind::Folder { expanded: false } => {
                if let Ok(children) = vault::children(&vault, &path, row.depth + 1) {
                    vault::expand(&mut self.session.sidebar, i, children);
                    self.request_frame();
                }
            }
        }
    }

    fn track_leave(&mut self, nonclient: bool) {
        let mut t = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: if nonclient { TME_LEAVE | TME_NONCLIENT } else { TME_LEAVE },
            hwndTrack: self.hwnd,
            dwHoverTime: 0,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut t);
        }
        self.tracking = true;
    }

    fn save_session(&mut self) {
        let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        if unsafe { GetWindowPlacement(self.hwnd, &mut wp) }.is_ok() {
            let r = wp.rcNormalPosition;
            let placed = Placement {
                x: r.left,
                y: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
                dpi: window::dpi(self.hwnd),
                maximized: wp.showCmd == SW_SHOWMAXIMIZED.0 as u32,
            };
            // Minimal and full each keep their own place.
            if self.session.minimal {
                self.session.minimal_window = placed;
            } else {
                self.session.window = placed;
            }
        }
        self.session.caret = self.editor.selection().head as u64;
        self.session.last_active = session::now();
        if let Some(path) = &self.session_path {
            let _ = session::save(path, &self.session);
        }
    }
}

/// The message Explorer broadcasts when the taskbar comes back (after a
/// restart of Explorer): tray icons must be added again.
fn taskbar_created() -> u32 {
    static ID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *ID.get_or_init(|| unsafe { windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) })
}

fn point(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32)
}

/// Non-client hit codes for the window buttons, so Windows 11 shows its
/// Snap Layouts flyout over our own maximize button.
fn target_for_nc(code: u32) -> Option<Target> {
    match code {
        c if c == HTMINBUTTON => Some(Target::Minimize),
        c if c == HTMAXBUTTON => Some(Target::Maximize),
        c if c == HTCLOSE => Some(Target::Close),
        _ => None,
    }
}

pub unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCCALCSIZE => {
            // No title bar: let Windows compute the side and bottom resize
            // borders, then give the caption area back to the client. A
            // maximized window hangs over the screen edge by its frame, so
            // its top moves down by that much. The rectangle to adjust comes
            // first in NCCALCSIZE_PARAMS, or alone when wParam is FALSE (as
            // during window creation).
            let rect = if wp.0 != 0 {
                unsafe { &mut (*(lp.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0] }
            } else {
                unsafe { &mut *(lp.0 as *mut RECT) }
            };
            let top = rect.top;
            let result = unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
            let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
            rect.top = top + if zoomed { window::frame_thickness(hwnd) } else { 0 };
            return result;
        }
        WM_NCHITTEST => {
            let result = unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
            if result.0 as u32 != HTCLIENT {
                return result;
            }
            let (sx, sy) = point(lp);
            let mut p = POINT { x: sx, y: sy };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut p);
            }
            // The top resize edge now lies inside the client area.
            let frame = window::frame_thickness(hwnd);
            if p.y < frame && !unsafe { IsZoomed(hwnd) }.as_bool() {
                let width = window::view(hwnd).0 as i32;
                let code = if p.x < 2 * frame { HTTOPLEFT } else if p.x >= width - 2 * frame { HTTOPRIGHT } else { HTTOP };
                return LRESULT(code as isize);
            }
            let code = match with_app(|a| a.hit_px(p.x, p.y)).flatten() {
                Some(Target::Minimize) => HTMINBUTTON,
                Some(Target::Maximize) => HTMAXBUTTON,
                Some(Target::Close) => HTCLOSE,
                Some(Target::Caption) => HTCAPTION,
                _ => HTCLIENT,
            };
            return LRESULT(code as isize);
        }
        WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
            let cursor = with_app(|a| match a.ui.hover {
                // The past shown is read, not edited.
                Some(Target::Note) if a.scrub_page().is_some() => IDC_ARROW,
                Some(Target::Note | Target::PopupText) => IDC_IBEAM,
                Some(Target::PopupHeader) => IDC_SIZEALL,
                Some(Target::PictureEdge(_) | Target::SidebarEdge) => windows::Win32::UI::WindowsAndMessaging::IDC_SIZEWE,
                _ if a.sidebar_grip.is_some() => windows::Win32::UI::WindowsAndMessaging::IDC_SIZEWE,
                Some(Target::SocialHeader) => IDC_SIZEALL,
                _ => IDC_ARROW,
            })
            .unwrap_or(IDC_ARROW);
            unsafe {
                if let Ok(cursor) = LoadCursorW(None, cursor) {
                    SetCursor(Some(cursor));
                }
            }
            return LRESULT(1);
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lp);
            with_app(|a| {
                if !a.tracking {
                    a.track_leave(false);
                }
                a.pointer_moved(x, y);
                if a.scrub_dragging() {
                    a.scrub_point(x);
                } else if a.scrub.is_some() {
                    a.scrub_hover(x, y);
                }
                if a.scroll_drag.is_some() {
                    let scale = window::dpi(a.hwnd) as f32 / 96.0;
                    a.scroll_drag_to(y as f32 / scale);
                }
                if a.selecting {
                    a.note_drag(x, y);
                }
                if a.resizing.is_some() {
                    a.resize_to(x);
                }
                if let Some((gx, gy)) = a.social_grip {
                    let scale = window::dpi(a.hwnd) as f32 / 96.0;
                    a.social_offset = a.keep_social_in((x as f32 / scale - gx, y as f32 / scale - gy));
                    a.request_frame();
                }
                if a.sidebar_grip.is_some() {
                    a.drag_sidebar_edge(x);
                }
                if a.popup.as_ref().is_some_and(|p| p.selecting) {
                    a.popup_drag(x, y);
                }
                let scale = window::dpi(a.hwnd) as f32 / 96.0;
                if let Some(p) = &mut a.popup
                    && let Some((gx, gy)) = p.held
                {
                    p.offset = (x as f32 / scale - gx, y as f32 / scale - gy);
                    a.dirty = true;
                }
                if let Some((_, px, py)) = a.row_press
                    && (a.row_drag.is_some() || (x - px).abs() + (y - py).abs() > 6)
                {
                    a.carry_row(x, y);
                }
                // A pressed chip that moves is being dragged toward the text.
                if let Some((_, px, py)) = a.chip_press
                    && (a.chip_drag.is_some() || (x - px).abs() + (y - py).abs() > 6)
                {
                    a.drag_chip(x, y);
                }
            });
            return LRESULT(0);
        }
        WM_NCMOUSEMOVE => {
            with_app(|a| {
                a.track_leave(true);
                let target = target_for_nc(wp.0 as u32);
                if a.ui.hover != target {
                    a.ui.hover = target;
                    a.request_frame();
                }
                a.update_reveal();
            });
            // Default handling shows the Snap Layouts flyout.
        }
        WM_MOUSELEAVE | WM_NCMOUSELEAVE => {
            with_app(|a| a.pointer_left());
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            let notches = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            with_app(|a| a.wheel(notches));
            return LRESULT(0);
        }
        WM_TIMER => {
            if with_app(|a| a.on_timer(wp.0)).unwrap_or(false) {
                return LRESULT(0);
            }
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lp);
            with_app(|a| {
                a.settle_composition();
                a.pressed = a.hit_px(x, y);
                a.tooltip_away();
                // A click elsewhere sends the selection bar away, and puts
                // the Aa card away.
                if !matches!(a.pressed, Some(Target::SelBar(_) | Target::SelBarPanel)) {
                    a.dismiss_selbar();
                }
                if !matches!(a.pressed, Some(Target::Reading | Target::ReadingPanel | Target::TextSize(_) | Target::Width(_))) {
                    a.close_reading();
                }
                // A click outside the link field puts it away.
                a.hide_link_card();
                if a.link.is_some() && !matches!(a.pressed, Some(Target::LinkPanel | Target::LinkField | Target::LinkRemove | Target::LinkSuggestion(_))) {
                    a.close_link_field();
                }
                // A click outside an open menu only closes it.
                if a.menu.is_some() && !matches!(a.pressed, Some(Target::MenuItem(_) | Target::MenuPanel)) {
                    let anchor = a.menu.as_ref().and_then(|m| m.anchor);
                    a.close_menu();
                    // Pressing another title bar button opens its menu instead.
                    if anchor.is_none() || anchor == a.pressed || a.pressed != Some(Target::SendTo) {
                        a.pressed = None;
                    }
                    return;
                }
                if let Some(bar @ (Target::Scrollbar | Target::PopupScrollbar | Target::TreeScrollbar)) = a.pressed {
                    let scale = window::dpi(a.hwnd) as f32 / 96.0;
                    a.scroll_press(bar, y as f32 / scale);
                    if a.pressed == Some(Target::Scrollbar) {
                        a.close_popup();
                    }
                    return;
                }
                // A click anywhere else closes the options.
                let on_options = matches!(
                    a.pressed,
                    Some(
                        Target::Options
                            | Target::OptionsPanel
                            | Target::Choice(..)
                            | Target::Transfer(_)
                            | Target::OptionsClose
                            | Target::VaultOpen
                            | Target::VaultShow
                    )
                );
                if a.ui.options_open && !on_options {
                    a.ui.options_open = false;
                    a.request_frame();
                    // A click beside the options only closes them.
                    if a.pressed == Some(Target::OptionsVeil) {
                        a.pressed = None;
                        return;
                    }
                }
                // A click outside the note open over the page closes it (a
                // chip opens another in its place).
                let on_popup = matches!(
                    a.pressed,
                    Some(
                        Target::PopupCard
                            | Target::PopupHeader
                            | Target::PopupText
                            | Target::PopupOpen
                            | Target::PopupClose
                            | Target::PopupCheckbox(_)
                            | Target::PopupScrollbar
                    )
                );
                if a.popup.is_some() && !on_popup && !matches!(a.pressed, Some(Target::Chip(_))) {
                    a.close_popup();
                }
                if a.pressed == Some(Target::PopupText) {
                    a.popup_down(x, y);
                }
                if a.pressed == Some(Target::PopupHeader) {
                    let scale = window::dpi(a.hwnd) as f32 / 96.0;
                    let hwnd = a.hwnd;
                    if let Some(p) = &mut a.popup {
                        p.held = Some((x as f32 / scale - p.offset.0, y as f32 / scale - p.offset.1));
                        unsafe { SetCapture(hwnd) };
                    }
                }
                if a.pressed == Some(Target::ScrubTrack) {
                    a.scrub_point(x);
                    unsafe { SetCapture(a.hwnd) };
                    return;
                }
                // The past shown is looked at: no caret, no selection.
                if a.pressed == Some(Target::Note) && a.scrub_page().is_some() {
                    return;
                }
                if a.pressed == Some(Target::Note) {
                    if let Some(f) = &mut a.find {
                        f.focus = find::Focus::Page;
                    }
                    a.note_down(x, y);
                }
                if let Some(Target::Picture(at)) = a.pressed {
                    a.picture_click(at, x, y);
                }
                if let Some(Target::PictureEdge(at)) = a.pressed {
                    a.begin_resize(at, x);
                }
                if a.pressed == Some(Target::SidebarEdge) {
                    a.grab_sidebar_edge(x, y);
                }
                if matches!(a.pressed, Some(Target::SocialCard | Target::SocialHeader)) {
                    let scale = window::dpi(a.hwnd) as f32 / 96.0;
                    a.social_grip = Some((x as f32 / scale - a.social_offset.0, y as f32 / scale - a.social_offset.1));
                    unsafe { SetCapture(a.hwnd) };
                }
                if let Some(Target::Chip(i)) = a.pressed {
                    a.chip_press = Some((i, x, y));
                    unsafe { SetCapture(a.hwnd) };
                }
                // A note row may be carried away rather than clicked.
                if let Some(Target::Row(i)) = a.pressed
                    && a.session.sidebar.get(i).is_some_and(|r| r.kind == RowKind::Note)
                {
                    a.row_press = Some((i, x, y));
                    unsafe { SetCapture(a.hwnd) };
                }
            });
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lp);
            with_app(|a| {
                if a.scrub_dragging() {
                    a.scrub_release();
                    a.pressed = None;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    return;
                }
                if a.resizing.is_some() {
                    a.pressed = None;
                    return a.end_resize();
                }
                if a.social_grip.take().is_some() {
                    a.pressed = None;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    return;
                }
                if a.sidebar_grip.take().is_some() {
                    a.pressed = None;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    a.save_session();
                    return a.request_frame();
                }
                if let Some((i, _, _)) = a.row_press.take() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    if a.row_drag.take().is_some() {
                        a.pressed = None;
                        a.drop_row(i, x, y);
                        return;
                    }
                }
                if let Some((i, _, _)) = a.chip_press.take() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    a.pressed = None;
                    let dragged = a.chip_drag.take().is_some();
                    match a.drop_at.take() {
                        Some(at) if dragged => a.link_chip(i, at),
                        _ if !dragged => a.open_chip(i),
                        _ => {}
                    }
                    a.request_frame();
                    return;
                }
                let popup_selecting = a.popup.as_mut().is_some_and(|p| std::mem::take(&mut p.selecting));
                if a.popup.as_mut().is_some_and(|p| p.held.take().is_some()) {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    a.pressed = None;
                    a.popup_dropped(x, y);
                    return;
                }
                if a.scroll_drag.take().is_some() {
                    a.pressed = None;
                    a.request_frame();
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    return;
                }
                if std::mem::take(&mut a.selecting) || popup_selecting {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                // A selection made with the mouse (a drag, a double click)
                // brings the selection bar.
                if a.pressed == Some(Target::Note) {
                    a.mouse_selected();
                }
                if let Some(target) = a.hit_px(x, y)
                    && a.pressed.take() == Some(target)
                {
                    a.activate(target);
                }
            });
            return LRESULT(0);
        }
        WM_CAPTURECHANGED => {
            with_app(|a| {
                a.selecting = false;
                if a.scroll_drag.take().is_some() {
                    a.request_frame();
                }
                if let Some(p) = &mut a.popup {
                    p.selecting = false;
                    p.held = None;
                }
                if a.chip_drag.take().is_some() || a.row_drag.take().is_some() {
                    a.drop_at = None;
                    a.request_frame();
                }
                a.row_press = None;
            });
            return LRESULT(0);
        }
        WM_RBUTTONDOWN => {
            return LRESULT(0);
        }
        WM_RBUTTONUP => {
            let (x, y) = point(lp);
            with_app(|a| a.context_menu(x, y));
            return LRESULT(0);
        }
        WM_NCLBUTTONDOWN => {
            if let Some(target) = target_for_nc(wp.0 as u32) {
                // Handled here: the default would track and draw the old
                // non-client buttons over ours.
                with_app(|a| a.pressed = Some(target));
                return LRESULT(0);
            }
        }
        WM_NCLBUTTONUP => {
            if let Some(target) = target_for_nc(wp.0 as u32) {
                with_app(|a| {
                    if a.pressed.take() == Some(target) {
                        a.activate(target);
                    }
                });
                return LRESULT(0);
            }
        }
        WM_SYSKEYDOWN if wp.0 as u16 == VK_LEFT.0 || wp.0 as u16 == VK_RIGHT.0 => {
            let forward = wp.0 as u16 == VK_RIGHT.0;
            with_app(|a| a.step(forward));
            return LRESULT(0);
        }
        WM_XBUTTONUP => {
            // XBUTTON1 goes back, XBUTTON2 forward.
            let forward = (wp.0 >> 16) & 0xFFFF == 2;
            with_app(|a| a.step(forward));
            return LRESULT(1);
        }
        WM_KEYDOWN => {
            // AltGr arrives as Ctrl+Alt and types characters: not a shortcut.
            let ctrl = key_down(VK_CONTROL.0) && !key_down(VK_MENU.0);
            let shift = key_down(VK_SHIFT.0);
            let vk = wp.0 as u16;
            let handled = if ctrl && vk == VK_OEM_5.0 {
                with_app(|a| {
                    // The note alone (minimal, or its own window) has no
                    // panels to toggle.
                    if !a.session.minimal && !a.compact {
                        a.toggle(if shift { Panel::Toolbar } else { Panel::Sidebar });
                    }
                });
                true
            } else {
                with_app(|a| a.key(vk, ctrl, shift)).unwrap_or(false)
            };
            if handled {
                return LRESULT(0);
            }
        }
        // Input methods (see `ime` and `app::compose`): their own composition
        // window is off where the text shows in place.
        windows::Win32::UI::WindowsAndMessaging::WM_IME_SETCONTEXT => {
            return unsafe { DefWindowProcW(hwnd, msg, wp, crate::ime::without_composition_window(lp)) };
        }
        windows::Win32::UI::WindowsAndMessaging::WM_IME_STARTCOMPOSITION => {
            if with_app(|a| a.compose_field().is_some()).unwrap_or(false) {
                return LRESULT(0);
            }
        }
        windows::Win32::UI::WindowsAndMessaging::WM_IME_COMPOSITION => {
            let (result, composing) = crate::ime::read(hwnd, lp.0 as u32);
            let shown = with_app(|a| {
                a.key_arrived.get_or_insert_with(Instant::now);
                a.last_key = Some(Instant::now());
                a.start_timer(WORDS_TIMER, WORDS_PAUSE.as_millis() as u32 + 20);
                a.gist_typing();
                a.ime_composed(result, composing)
            });
            // Elsewhere (the find bar) the input method's window shows it and
            // its result comes as characters. Busy (settling a composition
            // already), there is nothing to take.
            if shown != Some(false) {
                return LRESULT(0);
            }
        }
        windows::Win32::UI::WindowsAndMessaging::WM_IME_ENDCOMPOSITION => {
            with_app(|a| a.ime_ended());
        }
        WM_CHAR => {
            with_app(|a| {
                a.key_arrived.get_or_insert_with(Instant::now);
                a.last_key = Some(Instant::now());
                a.start_timer(WORDS_TIMER, WORDS_PAUSE.as_millis() as u32 + 20);
                a.gist_typing();
                a.typed(wp.0 as u16)
            });
            return LRESULT(0);
        }
        WM_ACTIVATE => {
            // Leaving for another app saves; coming back picks up edits made
            // there.
            let active = wp.0 & 0xFFFF != 0;
            with_app(|a| {
                if active {
                    a.check_disk();
                } else {
                    a.save_note(true);
                    a.save_popup(true);
                    a.session.last_active = session::now();
                }
            });
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut ps) };
            // The surface still holds the last frame: just copy it.
            with_app(|a| cpu::blit(&a.surface, hwnd, Some(dc)));
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            return LRESULT(0);
        }
        WM_SIZE => {
            // Drawn at once: while the user drags an edge, Windows runs its
            // own loop and the frame scheduler does not get to run.
            if !unsafe { IsIconic(hwnd) }.as_bool() {
                with_app(|a| {
                    a.resized();
                    let _ = a.draw();
                });
            }
            return LRESULT(0);
        }
        WM_GETMINMAXINFO => {
            // Smaller than this, nothing has room.
            let info = unsafe { &mut *(lp.0 as *mut windows::Win32::UI::WindowsAndMessaging::MINMAXINFO) };
            let scale = window::dpi(hwnd) as f32 / 96.0;
            info.ptMinTrackSize = POINT { x: (MIN_WINDOW.0 * scale) as i32, y: (MIN_WINDOW.1 * scale) as i32 };
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            // Move to the size Windows suggests for the new DPI; the
            // resulting WM_SIZE redraws at that DPI.
            let r = unsafe { &*(lp.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            return LRESULT(0);
        }
        WM_COPYDATA => {
            let cds = unsafe { &*(lp.0 as *const COPYDATASTRUCT) };
            let chars = cds.cbData as usize / 2;
            let text = (chars > 0 && !cds.lpData.is_null()).then(|| {
                let wide = unsafe { std::slice::from_raw_parts(cds.lpData as *const u16, chars) };
                String::from_utf16_lossy(wide)
            });
            let command = cds.dwData == 2;
            with_app(|a| {
                // An update or an installer wants it gone, tray or not.
                if command && text.as_deref() == Some("quit") {
                    return a.quit();
                }
                a.bring_up();
                match (command, text.as_deref()) {
                    // From the taskbar's menu.
                    (true, Some("new")) => a.fresh_page(),
                    (true, Some("search")) => a.open_search(),
                    (true, _) => {}
                    (false, Some(path)) if is_export(Path::new(path)) || crate::convert::is_document(Path::new(path)) => a.import_arg(Path::new(path)),
                    (false, Some(path)) => a.open(Path::new(path)),
                    // Launched again: smart resume, as at startup.
                    (false, None) if !a.session.resumes(session::now()) => a.fresh_page(),
                    (false, None) => {}
                }
            });
            return LRESULT(1);
        }
        WM_HOTKEY => {
            with_app(|a| a.quick_note());
            return LRESULT(0);
        }
        WM_APP_TRAY => {
            let event = (lp.0 & 0xFFFF) as u32;
            let (x, y) = ((wp.0 & 0xFFFF) as i16 as i32, ((wp.0 >> 16) & 0xFFFF) as i16 as i32);
            with_app(|a| a.tray_event(event, x, y));
            return LRESULT(0);
        }
        WM_APP_SIDEBAR => {
            let rows = unsafe { Box::from_raw(lp.0 as *mut Vec<SidebarRow>) };
            with_app(|a| {
                // Listed whole: its rows say which folders are open.
                a.session.expanded.clear();
                if a.session.sidebar != *rows {
                    a.session.sidebar = *rows;
                    a.request_frame();
                }
            });
            return LRESULT(0);
        }
        WM_APP_THEME => {
            let dark = wp.0 != 0;
            with_app(|a| {
                a.system_dark = Some(dark);
                if a.session.theme == 0 {
                    a.set_dark(dark);
                }
            });
            return LRESULT(0);
        }
        WM_APP_SPELL => {
            let found = unsafe { Box::from_raw(lp.0 as *mut Vec<(u64, Vec<(u32, u32)>)>) };
            with_app(|a| {
                let typing = a.typing_at();
                if let Some(spell) = &mut a.spell
                    && spell.arrived(*found, typing)
                {
                    a.request_frame();
                }
            });
            return LRESULT(0);
        }
        WM_DROPFILES => {
            let drop = windows::Win32::UI::Shell::HDROP(wp.0 as *mut _);
            let (files, at) = unsafe {
                use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, DragQueryPoint};
                let count = DragQueryFileW(drop, u32::MAX, None);
                let files: Vec<PathBuf> = (0..count)
                    .filter_map(|i| {
                        let mut buf = vec![0u16; DragQueryFileW(drop, i, None) as usize + 1];
                        let n = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
                        (n > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
                    })
                    .collect();
                let mut at = POINT::default();
                let _ = DragQueryPoint(drop, &mut at);
                DragFinish(drop);
                (files, at)
            };
            with_app(|a| a.drop_files(files, at.x, at.y, None));
            return LRESULT(0);
        }
        WM_APP_VAULT => {
            let changes = unsafe { Box::from_raw(lp.0 as *mut crate::watch::Changes) };
            with_app(|a| a.vault_changed(*changes));
            return LRESULT(0);
        }
        WM_APP_IMAGE => {
            let found = unsafe { Box::from_raw(lp.0 as *mut crate::images::Found) };
            with_app(|a| {
                if a.images.borrow_mut().arrived(*found) {
                    a.request_frame();
                }
            });
            return LRESULT(0);
        }
        window::WM_APP_WELCOME => {
            let note = unsafe { Box::from_raw(lp.0 as *mut PathBuf) };
            with_app(|a| a.welcome(&note));
            return LRESULT(0);
        }
        window::WM_APP_HELPER => {
            // Writing help can start now (it had given up for want of it).
            with_app(|a| a.model_changed());
            return LRESULT(0);
        }
        window::WM_APP_UPDATE => {
            let version = unsafe { Box::from_raw(lp.0 as *mut String) };
            with_app(|a| {
                a.update = Some(*version);
                a.request_frame();
            });
            return LRESULT(0);
        }
        WM_APP_WHISPER => {
            let text = unsafe { Box::from_raw(lp.0 as *mut String) };
            with_app(|a| a.whisper(*text));
            return LRESULT(0);
        }
        WM_APP_ASSIST => {
            with_app(|a| a.assist_events());
            return LRESULT(0);
        }
        WM_APP_IMPORTED => {
            let done = unsafe { Box::from_raw(lp.0 as *mut share::ExportsIn) };
            with_app(|a| a.exports_in(*done));
            return LRESULT(0);
        }
        window::WM_APP_DROPPED => {
            let d = unsafe { Box::from_raw(lp.0 as *mut arrive::Dropped) };
            with_app(|a| a.drop_files(d.files, d.x, d.y, d.at));
            return LRESULT(0);
        }
        window::WM_APP_SHARE => {
            let answer = unsafe { Box::from_raw(lp.0 as *mut crate::share::Answer) };
            with_app(|a| a.share_arrived(*answer));
            return LRESULT(0);
        }
        window::WM_APP_FETCHED => {
            let fetched = unsafe { Box::from_raw(lp.0 as *mut paste::Fetched) };
            with_app(|a| a.picture_fetched(*fetched));
            return LRESULT(0);
        }
        window::WM_APP_CONVERTED => {
            let done = unsafe { Box::from_raw(lp.0 as *mut crate::convert::Converted) };
            with_app(|a| a.documents_in(*done));
            return LRESULT(0);
        }
        WM_APP_INDEX if wp.0 == 1 => {
            // The vault's PDFs, after its notes: they join the index.
            let files = unsafe { Box::from_raw(lp.0 as *mut (String, Vec<Indexed>)) };
            let (vault, files) = *files;
            with_app(|a| {
                if a.session.vault == vault && !a.index.is_empty() {
                    a.index.retain(|n| search::is_note(&n.path));
                    a.index.extend(files);
                    if a.search.is_some() {
                        a.run_search();
                    }
                }
            });
            return LRESULT(0);
        }
        WM_APP_INDEX => {
            let index = unsafe { Box::from_raw(lp.0 as *mut (Vec<Indexed>, Corpus, agenda::Due)) };
            let (notes, corpus, due) = *index;
            with_app(|a| {
                // Recent shows only in a vault of more than a few dozen notes.
                if a.session.counted(notes.len()) {
                    a.request_frame();
                }
                a.index = notes;
                a.due_from_index(due);
                a.corpus = Some(corpus);
                a.update_related();
                if a.search.is_some() {
                    a.run_search();
                }
            });
            return LRESULT(0);
        }
        WM_QUERYENDSESSION => {
            // Windows is shutting down or signing out.
            with_app(|a| {
                a.save_popup(true);
                a.save_note(true);
                a.save_session();
            });
            return LRESULT(1);
        }
        WM_CLOSE => {
            let hide = with_app(|a| {
                a.save_popup(true);
                a.save_note(true);
                a.save_session();
                if let Some(path) = &a.dump_on_close {
                    let _ = a.surface.dump(path);
                }
                let hide = a.close_to_tray();
                if hide {
                    a.assist_hidden();
                }
                hide
            })
            .unwrap_or(false);
            unsafe {
                // With "Stay running", the window waits in the tray.
                if hide {
                    let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
                } else {
                    let _ = DestroyWindow(hwnd);
                }
            }
            return LRESULT(0);
        }
        WM_DESTROY => {
            with_app(|a| {
                a.remove_tray();
                // The model's process goes with the window, its pipes closed.
                a.stop_helper();
                if let Some(ms) = a.keystroke_median() {
                    crate::trace::info("keystroke_ms", &format!("{ms:.3}"));
                }
            });
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        m if m == taskbar_created() => {
            with_app(|a| a.tray_again());
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Advances a soft, slightly floaty spring toward `target`: how margin chips
/// drift. False once it has settled.
fn drift(x: &mut f32, v: &mut f32, target: f32, dt: f32) -> bool {
    const OMEGA: f32 = 11.0;
    const ZETA: f32 = 0.82;
    let a = OMEGA * OMEGA * (target - *x) - 2.0 * ZETA * OMEGA * *v;
    *v += a * dt;
    *x += *v * dt;
    if (target - *x).abs() < 0.1 && v.abs() < 2.0 {
        *x = target;
        *v = 0.0;
        return false;
    }
    true
}

/// Moves `x` (with velocity `v`) toward 0 as a critically damped spring of
/// stiffness `omega`, by `dt` seconds. Solved exactly rather than stepped,
/// so a stiff spring stays smooth at any frame rate. False once settled.
fn settle(x: &mut f32, v: &mut f32, dt: f32, omega: f32) -> bool {
    let decay = (-omega * dt).exp();
    let k = *v + omega * *x;
    *x = (*x + k * dt) * decay;
    *v = (*v - omega * k * dt) * decay;
    // A quarter DIP is under a pixel at any scale: stop there rather than
    // drawing frames nobody can see.
    if x.abs() < 0.25 && v.abs() < 5.0 {
        *x = 0.0;
        *v = 0.0;
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{Link, link_at, settle};

    /// Windows keeps one timer per id: a second feature on the same id
    /// silently replaces the first (the reminders' minute once took the
    /// writing helper's start, which then waited a minute to begin).
    #[test]
    fn every_timer_has_an_id_of_its_own() {
        use super::*;
        let ids = [
            reveal_timer(Panel::Sidebar),
            reveal_timer(Panel::Toolbar),
            hide_timer(Panel::Sidebar),
            hide_timer(Panel::Toolbar),
            AUTOSAVE_TIMER,
            RELATED_TIMER,
            PEEK_TIMER,
            WHISPER_TIMER,
            POPUP_SAVE_TIMER,
            WORDS_TIMER,
            assist::ASSIST_TIMER,
            assist::GIST_TIMER,
            assist::ASSIST_START,
            agenda::REMIND_TIMER,
            link::LINK_TIMER,
            bars::TOOLTIP_TIMER,
        ];
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            assert!(seen.insert(id), "timer id {id} is used twice");
        }
    }

    #[test]
    fn counts_words_and_writes_thousands() {
        assert_eq!(super::count_words("# Plan\n- [ ] ship **it** now\n---\nملاحظة هنا"), 6);
        assert_eq!(super::count_words(""), 0);
        assert_eq!(super::count_words("Moodboard\n![[Pasted image 1.png]]\nMore words."), 3);
        assert_eq!(super::thousands(7), "7");
        assert_eq!(super::thousands(1240), "1,240");
        assert_eq!(super::thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn links_lead_to_notes_pages_and_tags() {
        let text = "See [[Plan#Goals|the plan]], [site](https://x.io), www.y.io and [doc](My%20note.md) #idea";
        let at = |s: &str| text.find(s).unwrap() + 1;
        assert_eq!(link_at(text, at("the plan")), Some(Link::Note("Plan".into())));
        assert_eq!(link_at(text, at("site")), Some(Link::Web("https://x.io".into())));
        assert_eq!(link_at(text, at("www.y")), Some(Link::Web("https://www.y.io".into())));
        assert_eq!(link_at(text, at("doc")), Some(Link::Note("My note".into())));
        assert_eq!(link_at(text, at("#idea")), Some(Link::Tag("#idea".into())));
        assert_eq!(link_at(text, 1), None);
    }

    #[test]
    fn caret_glide_closes_the_gap_without_overshoot_or_jitter() {
        for dt in [1.0 / 60.0, 1.0 / 144.0, 1.0 / 30.0] {
            let (mut x, mut v) = (-120.0f32, 0.0f32);
            let mut frames = 0;
            let mut last = x;
            while settle(&mut x, &mut v, dt, 42.0) {
                assert!(x <= 0.0, "overshot at dt {dt}: {x}");
                assert!(x >= last, "moved backward at dt {dt}");
                last = x;
                frames += 1;
            }
            assert_eq!(x, 0.0);
            let seconds = frames as f32 * dt;
            assert!(seconds < 0.35, "took {seconds}s at dt {dt}");
        }
        // Most of the way there within 100 ms, so it reads as instant.
        let (mut x, mut v) = (-120.0f32, 0.0f32);
        for _ in 0..6 {
            settle(&mut x, &mut v, 1.0 / 60.0, 42.0);
        }
        assert!(x.abs() < 0.1 * 120.0, "{x}");
    }
}
