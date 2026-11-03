//! Lays out what is on screen: the visible part of the note, one
//! DirectWrite layout per paragraph (see `note`), and the window chrome
//! floating over it (see `chrome`). Only visible paragraphs are laid out, so
//! the cost does not grow with the note's length.
//!
//! Besides what to draw, a scene records what is under each point (`hits`),
//! which drives hover, clicks and the window's drag and resize areas, and
//! where each visible paragraph is, for placing the caret with the mouse.

use kolada_core::related::Kind as Relation;
use kolada_core::session::SidebarRow;
use kolada_markdown::{Block, Style};
use kolada_text::{Align, Engine, Format, Layout, Role};
use std::cell::RefCell;
use std::rc::Rc;
use windows_core::Result;

use crate::chrome;
use crate::note::{self, LIST_INDENT, MathView, NoteText, Para, QUOTE_INDENT};

pub use crate::geom::{Fill, Ink, Rect, WaveShape};

const BODY_SIZE: f32 = 16.0;
const CARET_WIDTH: f32 = 2.0;
/// Width of the mark showing a selected line break.
const BREAK_WIDTH: f32 = 6.0;
/// The note's gist shows beside its dates only with this much room left,
/// DIPs: less would be an ellipsis and a word.
const GIST_ROOM: f32 = 120.0;
/// Room the teal dot of things due takes after the dates.
const DOT_ROOM: f32 = 20.0;
/// Obsidian's heading scale, h1 to h6.
pub const HEADING_SCALE: [f32; 6] = [1.802, 1.602, 1.424, 1.266, 1.125, 1.0];

pub struct Text {
    pub x: f32,
    pub y: f32,
    pub layout: Rc<Layout>,
    pub ink: Ink,
    /// Opacity, 0..1.
    pub alpha: f32,
}

/// Settings > Devices: the paired devices and pairing, one place for Beam
/// and note sync alike (`quick::devices_panel`).
#[derive(Clone, Debug, PartialEq)]
pub struct DevicesView<'a> {
    pub devices: &'a [crate::quick::DeviceRow<'a>],
    pub pairing: crate::quick::Pairing<'a>,
    /// A line to say (a refusal), and quiet facts ("3 notes are too big
    /// to sync").
    pub said: Option<&'a str>,
    pub facts: &'a [String],
    /// What this device is called in a sentence ("PC", "Mac").
    pub this: &'a str,
    /// This device's name while it is being typed (Rename).
    pub naming: Option<&'a str>,
}

/// What Beam's row and button show (`chrome.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BeamView<'a> {
    /// Beams arrived and not yet seen.
    pub unseen: u32,
    /// The row's second line: the newest beam's first line while some are
    /// new, else where the last one came from and when ("From iPhone, 2 h
    /// ago"); empty before any.
    pub line: &'a str,
    /// Typing just now: the button fades further.
    pub typing: bool,
    /// The button's menu is open: the note it would beam.
    pub menu: Option<&'a str>,
    /// The keys that open the popup ("Alt+Z", "⌥Z"), as held now; empty
    /// when none are.
    pub keys: &'a str,
    /// A beam sent with no popup just reached the server: its check beside
    /// the button, its scale and alpha (`quick::Marks::button`).
    pub check: Option<(f32, f32)>,
}

/// Where Settings > Updates' "Check now" has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateCheck<'a> {
    /// Not asked yet.
    #[default]
    Idle,
    /// Asking the feed.
    Checking,
    /// The feed has nothing newer.
    UpToDate,
    /// A newer version, on its way (then the pill, as for the daily check).
    Downloading(&'a str),
    /// The feed did not answer, or what came failed its check.
    Failed,
}

impl UpdateCheck<'_> {
    /// What its button says, given the version ready (the pill's), if any;
    /// and whether it can be pressed.
    pub fn label(self, ready: Option<&str>) -> (String, bool) {
        match (ready, self) {
            (Some(v), _) => (format!("{v} is ready"), false),
            (None, UpdateCheck::Idle) => ("Check now".into(), true),
            (None, UpdateCheck::Checking) => ("Checking\u{2026}".into(), false),
            (None, UpdateCheck::UpToDate) => ("Up to date".into(), true),
            (None, UpdateCheck::Downloading(v)) => (format!("Downloading {v}\u{2026}"), false),
            (None, UpdateCheck::Failed) => ("Couldn't check".into(), true),
        }
    }
}

/// `UpdateCheck` as a shell keeps it, between frames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CheckState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Downloading(String),
    Failed,
}

impl CheckState {
    pub fn view(&self) -> UpdateCheck<'_> {
        match self {
            CheckState::Idle => UpdateCheck::Idle,
            CheckState::Checking => UpdateCheck::Checking,
            CheckState::UpToDate => UpdateCheck::UpToDate,
            CheckState::Downloading(v) => UpdateCheck::Downloading(v),
            CheckState::Failed => UpdateCheck::Failed,
        }
    }

    /// A check is under way: pressing again does nothing.
    pub fn busy(&self) -> bool {
        matches!(self, CheckState::Checking | CheckState::Downloading(_))
    }
}

/// Something that reacts to the mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// Empty space along the top edge: drags the window.
    Caption,
    Minimize,
    Maximize,
    Close,
    /// Docks or undocks the panel (the pill, the collapse and pin buttons).
    ToggleSidebar,
    ToggleToolbar,
    /// Invisible areas near a collapsed panel: hovering there reveals it.
    RevealSidebar,
    RevealToolbar,
    /// A panel's background, and the margin that keeps a revealed panel
    /// open while the pointer is just outside it.
    SidebarPanel,
    ToolbarPanel,
    Search,
    /// The ✕ at the search field's end, once something is typed: the
    /// search ends, and the side menu's notes come back.
    SearchClear,
    NewNote,
    /// Every marked passage, gathered on one page.
    Highlights,
    Options,
    /// The side menu's bottom bar: the trash, and a step through the
    /// appearances.
    Trash,
    ThemeCycle,
    /// The notes as the side menu's list or as a grid of boxes under the
    /// note (see `grid`): the side menu's icon, and the grid's own.
    ViewToggle,
    /// The grid (see `grid`): its sheet, a box (by its place among the
    /// notes), and while browsing, the note above it (back to the note).
    GridSheet,
    GridBox(usize),
    GridNote,
    /// The text size (true: larger), in the toolbar's Aa card; and a step
    /// through the line widths (the palette's).
    TextSize(bool),
    LineWidth,
    /// The toolbar's Aa button, the small card it opens (text size and line
    /// width), and a line width in it (0 standard, 1 narrow, 2 wide).
    Reading,
    ReadingPanel,
    Width(u8),
    /// The selection bar (see `selbar`): a button on it, and the capsule.
    SelBar(Tool),
    SelBarPanel,
    /// A row of the file tree, by index into the session's rows, and the
    /// button on it that opens its menu.
    Row(usize),
    RowMenu(usize),
    /// The trash in place of the file tree: a deleted note (by index), the
    /// button that restores it, the way back to the tree, and emptying it.
    TrashRow(usize),
    TrashRestore(usize),
    TrashBack,
    TrashEmpty,
    /// A menu's background, and one of its items.
    MenuPanel,
    MenuItem(usize),
    /// The title bar's quick actions: hand the note to an AI app; in a
    /// note's own window, back to the main one.
    SendTo,
    ToMain,
    /// Notes in and out, from the options.
    Transfer(Transfer),
    /// The find bar: its background, its fields, its buttons.
    FindPanel,
    FindField,
    ReplaceField,
    FindPrev,
    FindNext,
    FindReplace,
    FindClose,
    ReplaceOne,
    ReplaceAll,
    /// A note suggested for the `[[link]]` being typed, by index.
    Suggestion(usize),
    /// The link field (Ctrl+K): its card, its field, taking the link away,
    /// and a note suggested in it, by index.
    LinkPanel,
    LinkField,
    LinkRemove,
    /// The link field on a link: open it, copy where it goes, edit that;
    /// and while editing, keep what is typed or go back.
    LinkOpen,
    LinkCopy,
    LinkEdit,
    LinkSave,
    LinkCancel,
    LinkSuggestion(usize),
    /// A heading on the scroll bar's map of the note, by index.
    Heading(usize),
    /// The scroll bars of the page, of the note open over it, and of the
    /// file tree.
    Scrollbar,
    PopupScrollbar,
    TreeScrollbar,
    Tool(Tool),
    /// The note text.
    Note,
    /// A task's checkbox, by the offset of its line.
    Checkbox(usize),
    /// A heading's fold mark, or the count of a folded section's lines, by
    /// the offset of the heading's line.
    Fold(usize),
    /// The command palette (see `palette`): around it (a click there
    /// closes it), the panel, its field, and a command, by its row.
    PaletteVeil,
    PalettePanel,
    PaletteField,
    PaletteRow(usize),
    /// A picture in the note, and the edge that resizes it, by the offset
    /// of its line.
    Picture(usize),
    PictureEdge(usize),
    /// A picture opened full size (see `lightbox`): a click puts it back.
    Lightbox,
    /// The side menu's right edge: dragged, it widens or narrows the menu.
    SidebarEdge,
    /// Beside Send: the window made minimal, the note alone and small.
    Minimal,
    /// The side menu's social drawer: its fold, and a platform in it; the
    /// preview's card, its tabs, its close button.
    SocialDrawer,
    Social(kolada_core::social::Platform),
    SocialCard,
    SocialHeader,
    SocialTab(kolada_core::social::Platform),
    SocialClose,
    /// The note open over the page: its card, its text, its buttons, and a
    /// checkbox in it.
    PopupCard,
    PopupHeader,
    PopupText,
    PopupOpen,
    PopupClose,
    PopupCheckbox(usize),
    /// A search result, by index.
    Result(usize),
    /// The options: the page dimmed behind them, their card, one of their
    /// choices, closing them, and the vault's folder (another one, or it
    /// in Explorer).
    OptionsVeil,
    OptionsPanel,
    Choice(Setting, u8),
    OptionsClose,
    VaultOpen,
    VaultShow,
    /// Settings > Updates: check now, whatever the daily rule says.
    CheckUpdates,
    /// A switch of Settings > Features: turns its setting on or off.
    Switch(Setting),
    /// Settings > Font's "Other": opens the finder of the machine's fonts
    /// (or closes it); its field; a family it found, by index.
    FontOther,
    FontField,
    FontName(usize),
    /// A related note in the margin, by index.
    Chip(usize),
    /// The time scrubber (see `scrub`): the note's dates that open it, its
    /// dock, the track of versions, restoring the version shown, closing;
    /// and bringing back a line of it, by index.
    PageDate,
    /// A note under Recent in the side menu, by its place in `Input::recent`.
    Recent(usize),
    /// A favorite (note or folder), by its place in `Input::favorites`.
    Favorite(usize),
    /// The side menu's section headings: they fold their section shut or open.
    FoldFavorites,
    FoldRecent,
    /// The teal dot beside the dates when something is due: opens the Today
    /// sheet (see `agenda`). The sheet itself, the page behind it, a task's
    /// row (opens its note) and its box (ticks it), by index.
    DueDot,
    AgendaPanel,
    AgendaShade,
    AgendaOpen(usize),
    AgendaTick(usize),
    ScrubPanel,
    ScrubTrack,
    ScrubRestore,
    ScrubClose,
    BringBack(usize),
    /// The pill above the side menu's icons when an update is ready:
    /// restarts into it.
    Update,
    /// The installer's screen (see `setup`).
    Setup(crate::setup::Hit),
    /// The share card under Send (see `share`).
    Share(crate::share::Hit),
    /// The writing model's card, offered on first run (see `modelcard`).
    Model(crate::modelcard::Hit),
    /// Quick Send's popup, and Settings' button to its devices (see `quick`).
    Quick(crate::quick::Hit),
    /// The welcome screen a first run opens on (see `welcome`).
    Welcome(crate::welcome::Hit),
}

/// A related note (or marked passage) drifting in the margin.
pub struct Chip<'a> {
    pub name: &'a str,
    pub detail: &'a str,
    pub kind: Relation,
    /// How strongly it relates, 0..1: weaker ones stay fainter.
    pub strength: f32,
    /// Its top edge (DIPs from the window's top), how far it still has to
    /// drift in from the right, and its opacity.
    pub y: f32,
    pub dx: f32,
    pub alpha: f32,
}

/// A peek at a chip's note, floating beside it: its name and first lines.
pub struct Peek<'a> {
    pub chip: usize,
    pub title: &'a str,
    pub body: &'a str,
    pub alpha: f32,
}

/// A passing word at the top of the page ("Saved as ..."), fading.
pub struct Whisper<'a> {
    pub text: &'a str,
    pub alpha: f32,
}

/// Size of a margin chip and the space between chips, DIPs.
pub const CHIP_HEIGHT: f32 = 50.0;
pub const CHIP_GAP: f32 = 8.0;
const CHIP_WIDTH: f32 = 232.0;

/// Where margin chips go: right of the note column, if there is room.
pub fn margin(column: Rect, width: f32) -> Option<(f32, f32)> {
    let left = column.right + 44.0;
    let w = (width - left - 24.0).min(CHIP_WIDTH);
    (w >= 160.0).then_some((left, w))
}

/// Notes in (files, a folder, the clipboard) and the open note out (a
/// Markdown file, a web page, a PDF), from the options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transfer {
    Files,
    Folder,
    Clipboard,
    /// An export of Notes (`.nbk`, see `kolada_core::nbk`).
    NotesApp,
    Markdown,
    Page,
    Pdf,
}

/// The OS the interface is drawn for, where what it offers differs (the
/// options: a tray icon is Windows', a menu bar icon the Mac's). Each
/// platform's shell sets its own; Windows is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    #[default]
    Windows,
    Mac,
    /// iPad and iPhone.
    Ios,
    /// Linux desktops (X11 and Wayland): Ctrl keys, a window of its own, no
    /// tray icon, quick note keys, reminders or updater yet.
    Linux,
}

/// What the options set; values as in the session snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Setting {
    Theme,
    TextSize,
    LineWidth,
    Font,
    Launch,
    Related,
    Spell,
    Tray,
    Hotkey,
    /// Opening as the full window or the minimal one.
    Window,
    /// The writing model: the value is its place in the catalogue
    /// (kolada-assist's, the shell's to map: 0 Light, 1 Full), with 2
    /// (`modelcard::OFF`) meaning Off.
    Assist,
    /// Reminders of tasks due: a nudge, or none.
    Remind,
    /// Checking for updates once a day, or not (only where the app updates
    /// itself: see `Input::updater`).
    Updates,
    /// Show Markdown: 0 clean editing (no marks show), 1 marks show at the
    /// caret.
    ShowMarks,
    /// Beam's keys: 0 Alt+Z (the Mac's Ctrl+Opt+Shift+Z), 1 Win+Shift+Z,
    /// 2 Off.
    QuickKeys,
    /// What Quick Send's keys do: 0 show what was copied first, 1 beam it
    /// at once.
    QuickBeam,
    /// Beam altogether: 0 on, 1 off (no row, button, keys or devices'
    /// listening).
    Beam,
    /// Motion: 0 on, 1 off (every transition snaps to where it ends).
    Motion,
}

/// How many settings there are: the length of `Input::options`.
pub const SETTINGS: usize = 18;

/// The Font option's value for a font of the machine's own, named by
/// `Input::font_name` (0 sans serif, 1 serif and 2 monospace are the
/// shell's own three).
pub const INSTALLED_FONT: u8 = 3;

/// Whether a switch of the options (`chrome`'s Features) is on, by its
/// setting's value: the writing model is on with a model chosen, the rest
/// at 0.
pub fn switch_on(setting: Setting, value: u8) -> bool {
    match setting {
        Setting::Assist => value != crate::modelcard::OFF,
        _ => value == 0,
    }
}

/// What a press on a switch sets its setting to: off, or on again (the
/// writing model on `model`, the one chosen before).
pub fn switched(setting: Setting, value: u8, model: Option<usize>) -> u8 {
    match setting {
        Setting::Assist => crate::modelcard::setting_value(switch_on(setting, value), model),
        _ => u8::from(value == 0),
    }
}

/// Settings > Font's finder of the machine's fonts, while it is open: what
/// is typed, where the caret is in it, the families that match (each drawn
/// in its own face, loaded for it by the shell), the one Up and Down picked,
/// and how many match in all.
#[derive(Clone, Copy, Debug, Default)]
pub struct FontFinder<'a> {
    pub query: &'a str,
    pub caret: usize,
    pub names: &'a [&'static str],
    pub picked: Option<usize>,
    pub total: usize,
}

impl Setting {
    /// Where its value is in `Input::options`.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// An item of a menu: its icon, what it says, its shortcut, whether it
/// destroys something, and whether a divider goes above it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuItem {
    pub icon: &'static str,
    pub label: String,
    pub keys: &'static str,
    pub danger: bool,
    pub divider: bool,
}

/// A menu open at a point (DIPs), fading in. Aligned right, the point is
/// its top right corner.
pub struct MenuView<'a> {
    pub x: f32,
    pub y: f32,
    pub align_right: bool,
    pub items: &'a [MenuItem],
    pub alpha: f32,
}

/// The find bar, as the view shows it: what is sought (and the caret in
/// it), what replaces it when replacing, which field types (0 none: the
/// page does, 1 find, 2 replace), and how many matches, which one is on.
pub struct FindView<'a> {
    pub query: &'a str,
    pub query_caret: usize,
    pub replace: Option<(&'a str, usize)>,
    pub focus: u8,
    pub count: usize,
    pub current: Option<usize>,
}

/// Notes suggested for the `[[link]]` being typed, under the caret: their
/// names and folders, the one Enter takes, and a name to create a note by
/// when none is it.
pub struct Suggestions<'a> {
    pub names: &'a [(String, String)],
    pub selected: usize,
    pub create: Option<&'a str>,
}

/// The link field under the selection (see `linkfield`): what is typed and
/// the caret in it, the notes suggested for it (names and folders) and the
/// one picked, whether it opened on a link (so offers its actions), and
/// whether what it leads to is being edited.
pub struct LinkFieldView<'a> {
    pub query: &'a str,
    pub caret: usize,
    pub names: &'a [(String, String)],
    pub picked: Option<usize>,
    pub existing: bool,
    pub editing: bool,
}

/// A heading of a long note, placed along its scroll bar: where scrolling
/// puts it at the top (0..1), what it says, its level, and whether the
/// view is in its section.
pub struct Landmark<'a> {
    pub at: f32,
    pub text: &'a str,
    pub level: u8,
    pub here: bool,
}

/// A deleted note as the trash lists it: its name and when it went.
pub struct TrashEntry {
    pub name: String,
    pub when: String,
}

/// The page's scroll bar: where the view is (0 at the top, 1 at the end),
/// how much of the note it shows (0..1), and how visible it is: it shows
/// while scrolling and when the pointer comes near, and fades at rest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollView {
    pub at: f32,
    pub size: f32,
    pub alpha: f32,
}

/// The side menu's search, as the view shows it.
pub struct SearchView<'a> {
    pub query: &'a str,
    pub caret: usize,
    pub hits: &'a [kolada_core::search::Hit],
    pub selected: usize,
    /// The first result shown.
    pub first: usize,
    /// With `>` typed: the commands that match, in place of notes.
    pub commands: Option<&'a [MenuItem]>,
    /// Whether the field takes the keys (its caret shows); a search kept
    /// open beside the note it opened does not.
    pub focused: bool,
}

/// The two panels that collapse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Sidebar,
    Toolbar,
}

impl Target {
    /// The panel this target belongs to: while the pointer is over any of
    /// them, a revealed panel stays open.
    pub fn panel(self) -> Option<Panel> {
        match self {
            Target::ToggleSidebar
            | Target::RevealSidebar
            | Target::SidebarPanel
            | Target::Search
            | Target::SearchClear
            | Target::NewNote
            | Target::Highlights
            | Target::Options
            | Target::Trash
            | Target::ThemeCycle
            | Target::ViewToggle
            | Target::Row(_)
            | Target::RowMenu(_)
            | Target::TrashRow(_)
            | Target::TrashRestore(_)
            | Target::TrashBack
            | Target::TrashEmpty
            | Target::TreeScrollbar
            | Target::Result(_)
            | Target::SocialDrawer
            | Target::Social(_)
            | Target::Transfer(_) => Some(Panel::Sidebar),
            Target::ToggleToolbar
            | Target::RevealToolbar
            | Target::ToolbarPanel
            | Target::Tool(_)
            | Target::TextSize(_)
            | Target::LineWidth
            | Target::Reading
            | Target::ReadingPanel
            | Target::Width(_) => Some(Panel::Toolbar),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Heading,
    Bold,
    Italic,
    Strike,
    List,
    Checklist,
    Quote,
    Code,
    Link,
    /// `==marked==` text (the selection bar's).
    Highlight,
}

/// How round pictures' corners are (DIPs), like the page's panels.
pub const PICTURE_ROUND: f32 = 10.0;

/// A picture on the screen: the layer it is in (see `Scene::layer_now`),
/// where (DIPs), its pixels, its opacity.
pub struct Picture {
    pub layer: usize,
    pub rect: Rect,
    pub pixels: std::sync::Arc<crate::pixels::Pixels>,
    pub alpha: f32,
    /// How round its corners are, DIPs.
    pub round: f32,
}

/// A mark drawn from its outline (see `vector`), in an ink: its layer,
/// the square it fills, its opacity.
pub struct VectorMark {
    pub layer: usize,
    pub rect: Rect,
    pub logo: crate::vector::Logo,
    pub ink: Ink,
    pub alpha: f32,
}

/// A shape filled from its outline in an ink (math's roots and tall
/// brackets): its layer, the rectangle it is drawn in, its polygons (DIPs
/// from that rectangle's top left), its opacity.
pub struct ShapeMark {
    pub layer: usize,
    pub rect: Rect,
    pub polygons: Rc<Vec<Vec<(f32, f32)>>>,
    pub ink: Ink,
    pub alpha: f32,
}

/// A visible paragraph of the note, where it was drawn.
pub struct Line {
    pub para: Para,
    pub x: f32,
    pub y: f32,
    pub height: f32,
}

#[derive(Default)]
pub struct Scene {
    pub fills: Vec<Fill>,
    pub texts: Vec<Text>,
    /// Its icons' ink is in their boxes' middles already (`centre_icons`,
    /// at the scale it was built for): drawing moves them no further.
    pub icons_centred: bool,
    /// Where the chrome starts in `fills` and `texts`: everything before
    /// belongs to the note, which the chrome floats over.
    pub chrome_from: (usize, usize),
    /// Layers over the chrome: where each starts in `fills` and `texts`,
    /// and the rectangle it is clipped to.
    pub overlays: Vec<(usize, usize, Option<Rect>)>,
    /// Later entries are on top.
    pub hits: Vec<(Rect, Target)>,
    /// Where the collapsed pills are (side menu, toolbar), for the magnets.
    pub pills: [Option<Rect>; 2],
    /// The note column, and its visible paragraphs top to bottom.
    pub column: Rect,
    /// The teal dot of things due, where it was drawn: the Today card opens there.
    pub due_dot: Option<Rect>,
    pub lines: Vec<Line>,
    /// Where the caret goes, if it is on screen; `place_caret` draws it.
    pub caret: Option<Rect>,
    /// The other carets on screen, drawn with it.
    pub carets: Vec<Rect>,
    /// Things to click inside the note (checkboxes), above the text.
    pub note_hits: Vec<(Rect, Target)>,
    /// File tree rows that fit in the side menu, and its scroll bar's
    /// track, for scrolling it.
    pub tree_rows: usize,
    pub tree_track: Option<Rect>,
    /// Search results that fit in the side menu.
    pub result_rows: usize,
    /// How far the options' rows can scroll (0: they all fit).
    pub options_scroll_max: f32,
    /// A resized picture's size, said beside its corner (drawn over the
    /// note, with the chrome).
    pub picture_label: Option<(Rect, String)>,
    /// How far the social preview can scroll.
    pub social_scroll_max: f32,
    /// How far Quick Send's clips can scroll, DIPs (`quick::popup`).
    pub quick_scroll_max: f32,
    /// Where Settings > Devices starts in the options page (its scroll).
    pub options_devices_at: f32,
    /// How far the welcome screen scrolls, DIPs (0: it fits).
    pub welcome_scroll_max: f32,
    /// Where each of the welcome screen's controls is, in the order drawn,
    /// scrolled out of view or not: what Tab walks (`welcome::next_focus`).
    pub welcome_controls: Vec<(Rect, Target)>,
    /// Beam's send field's text as drawn (its index in `texts`), for the
    /// point a click lands on (`quick::draft_offset_at`).
    pub draft_text: Option<usize>,
    /// Where the note open over the page went.
    pub popup: Option<PopupDrawn>,
    /// Where the card saying a link's address went, if it shows.
    pub link_card: Option<Rect>,
    /// The selection on the page as drawn: its first line and its last
    /// (each that line's selected part), for the selection bar.
    pub selection_span: Option<(Rect, Rect)>,
    /// Where the selection bar and a button's name went, if they show.
    pub selbar: Option<Rect>,
    pub tooltip: Option<Rect>,
    /// Pictures, and whether drawing has moved on from the note to the
    /// chrome (for which layer a picture is in).
    pub pictures: Vec<Picture>,
    pub vectors: Vec<VectorMark>,
    pub shapes: Vec<ShapeMark>,
    pub in_chrome: bool,
}

impl Scene {
    /// Draws the caret `offset` DIPs from where it belongs (while it glides
    /// there), in the note's layer.
    pub fn place_caret(&mut self, (dx, dy): (f32, f32)) {
        // The other carets stay put while the main one glides.
        let others = self.carets.iter().map(|r| (*r, (0.0, 0.0)));
        let all: Vec<(Rect, (f32, f32))> = self.caret.map(|r| (r, (dx, dy))).into_iter().chain(others).collect();
        for (r, (dx, dy)) in all {
            let fill = Fill::solid(r.offset(dx, dy), CARET_WIDTH / 2.0, Ink::Accent);
            self.fills.insert(self.chrome_from.0, fill);
            self.chrome_from.0 += 1;
            self.overlays.iter_mut().for_each(|o| o.0 += 1);
        }
    }

    /// Starts a layer over everything so far, clipped to `clip`.
    pub fn overlay(&mut self, clip: Option<Rect>) {
        self.overlays.push((self.fills.len(), self.texts.len(), clip));
    }

    /// The layer being drawn: the note (0), the chrome (1), or an overlay.
    pub fn layer_now(&self) -> usize {
        if self.in_chrome { 1 + self.overlays.len() } else { 0 }
    }
}

/// The topmost target under a point in DIPs; later hits are on top.
pub fn hit(hits: &[(Rect, Target)], x: f32, y: f32) -> Option<Target> {
    hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, t)| *t)
}

/// The text engine, shared by everything that lays text out: layouts take
/// it for the moment they are made.
pub type Eng = RefCell<Engine>;

/// How each kind of text in the app is set.
#[derive(Clone, Copy)]
pub struct Fonts {
    pub body: Format,
    pub title: Format,
    pub headings: [Format; 6],
    pub mono: Format,
    /// Single-line UI label, vertically centered in its box.
    pub label: Format,
    /// Small section label above the file tree.
    pub section: Format,
    /// Icon glyphs, centered in their box.
    pub icon: Format,
    /// Smaller icons for the window buttons.
    pub icon_small: Format,
    /// The "H" of the heading tool.
    pub glyph: Format,
    /// A folded code fence: a sliver of space, no text.
    pub collapsed: Format,
    /// Small single-line text: search snippets, option names.
    pub small: Format,
    /// A peek's name and text.
    pub peek_title: Format,
    pub peek: Format,
    /// Centered labels in segmented controls.
    pub choice: Format,
}

/// Lines are `line_height` times the size, with the baseline a fifth of the
/// line up from its bottom, as the app has always set them.
fn format(role: Role, weight: u16, size: f32, line_height: f32, wrap: bool) -> Format {
    // One-line text is cut at the layout's width, not spilled.
    Format { weight, line: Some(size * line_height), baseline: Some(0.8), wrap, ..Format::new(role, size) }
}

/// A single-line format whose text is centered vertically (and, with
/// `center`, horizontally) in the box given to its layouts.
fn boxed(role: Role, weight: u16, size: f32, center: bool) -> Format {
    Format { center_y: true, align: if center { Align::Center } else { Align::Start }, ..format(role, weight, size, 1.25, false) }
}

const NORMAL: u16 = 400;
const SEMI_BOLD: u16 = 600;
const BOLD: u16 = 700;

/// The note's body text size, DIPs, for the Text size option (see
/// `Session::text_size`): 0 is the default; anything else not among the
/// sizes reads as the nearest.
pub fn text_size(option: u8) -> u8 {
    use kolada_core::session::{DEFAULT_TEXT_SIZE, TEXT_SIZES};
    if option == 0 {
        return DEFAULT_TEXT_SIZE;
    }
    TEXT_SIZES.into_iter().min_by_key(|&s| (s as i16 - option as i16).abs()).unwrap_or(DEFAULT_TEXT_SIZE)
}

/// The note's text size for the Text size option, as a scale of the
/// default.
pub fn text_scale(option: u8) -> f32 {
    text_size(option) as f32 / BODY_SIZE
}

/// The Text size option a step smaller or larger (the default kept as 0),
/// or None past the smallest or the largest.
pub fn step_text_size(option: u8, up: bool) -> Option<u8> {
    use kolada_core::session::{DEFAULT_TEXT_SIZE, TEXT_SIZES};
    let i = TEXT_SIZES.iter().position(|&s| s == text_size(option))?;
    let next = if up { TEXT_SIZES.get(i + 1) } else { i.checked_sub(1).and_then(|i| TEXT_SIZES.get(i)) };
    next.map(|&s| if s == DEFAULT_TEXT_SIZE { 0 } else { s })
}

impl Fonts {
    /// The formats, with note text at `scale` times its default size. The
    /// typeface the Font option picks is the engine's (see `fonts`).
    pub fn new(scale: f32, font: u8) -> Fonts {
        // Monospace sets wider: a touch smaller reads the same.
        let body = BODY_SIZE * scale * if font == 2 { 0.9 } else { 1.0 };
        let heading = |i: usize| format(Role::Text, BOLD, body * HEADING_SCALE[i], 1.3, true);
        Fonts {
            body: format(Role::Text, NORMAL, body, 1.5, true),
            title: format(Role::Text, BOLD, body * HEADING_SCALE[0], 1.3, true),
            headings: [heading(0), heading(1), heading(2), heading(3), heading(4), heading(5)],
            mono: format(Role::Code, NORMAL, body * 0.875, 1.5, true),
            small: boxed(Role::Ui, NORMAL, 11.5, false),
            peek_title: format(Role::Ui, SEMI_BOLD, 14.0, 1.3, true),
            peek: format(Role::Ui, NORMAL, 13.0, 1.45, true),
            choice: boxed(Role::Ui, NORMAL, 12.5, true),
            label: boxed(Role::Ui, NORMAL, 13.0, false),
            section: boxed(Role::Ui, SEMI_BOLD, 11.0, false),
            icon: boxed(Role::Icon, NORMAL, 15.0, true),
            icon_small: boxed(Role::Icon, NORMAL, 10.0, true),
            glyph: boxed(Role::Ui, SEMI_BOLD, 15.0, true),
            collapsed: format(Role::Ui, NORMAL, 1.0, 8.0, true),
        }
    }
}

#[derive(Default)]
pub struct Input<'a> {
    pub vault_name: &'a str,
    /// Vault-relative path of the open note, without `.md`; and its name as
    /// its title now reads, when the file has yet to follow.
    pub active_note: &'a str,
    pub live_name: Option<&'a str>,
    pub text: &'a str,
    pub sidebar: &'a [SidebarRow],
    /// Byte offset of the paragraph at the top of the viewport.
    pub scroll_line: usize,
    pub scroll_px: f32,
    /// Selection anchor and head (the caret), byte offsets.
    pub selection: (usize, usize),
    /// The other carets and their selections, when there are several.
    pub carets: &'a [(usize, usize)],
    /// The sections folded under their headings (see `note::NoteText`).
    pub sections_folded: &'a [(usize, usize)],
    /// The text's lines and their Markdown blocks.
    pub lines: &'a [kolada_markdown::Line],
    /// Toolbar buttons for formatting the caret is in.
    pub active_tools: &'a [Tool],
    /// The first file tree row shown.
    pub tree_top: usize,
    /// The side menu's search, while it is open.
    pub search: Option<SearchView<'a>>,
    /// The options popover is open, and the options' values, by
    /// `Setting::index` (see `Session`); how fast this launch drew its
    /// first frame, ms.
    pub options_open: bool,
    /// How far the options' rows are scrolled, DIPs.
    pub options_scroll: f32,
    pub options: [u8; SETTINGS],
    /// The faces the Font option's sans, serif and monospace stand for
    /// here, each choice's name drawn in its own ("": the UI's); the
    /// machine's font chosen (`INSTALLED_FONT`), and its finder while open.
    pub font_faces: [&'static str; 3],
    pub font_name: Option<&'static str>,
    pub font_finder: Option<FontFinder<'a>>,
    /// A switch whose rows are folding out or away, and how far open they
    /// are (`chrome::folding`); None, each as its switch is.
    pub folding: Option<(Setting, f32)>,
    /// The vault's folder, for the options.
    pub vault_path: &'a str,
    pub opened_in: Option<f32>,
    /// What a keystroke costs here, from its message to its frame (ms).
    pub keystroke: Option<f32>,
    /// A menu open over everything; the file tree row it belongs to, or
    /// the title bar button it opened from.
    pub menu: Option<MenuView<'a>>,
    pub menu_row: Option<usize>,
    pub menu_anchor: Option<Target>,
    /// The trash, in place of the file tree.
    pub trash: Option<&'a [TrashEntry]>,
    /// The page's scroll bar, if the note is longer than the window; the
    /// scroll bar being dragged.
    pub scrollbar: Option<ScrollView>,
    pub dragging: Option<Target>,
    /// The side menu's width as set by dragging its edge, DIPs; 0 for the
    /// default (see `chrome::menu_width`).
    pub sidebar_width: f32,
    /// The find bar; what it found, and where along the note each match is
    /// (0..1, for marks beside the scroll bar).
    pub find: Option<FindView<'a>>,
    pub matches: &'a [(usize, usize)],
    pub ticks: &'a [f32],
    /// Notes for the `[[link]]` being typed.
    pub suggestions: Option<Suggestions<'a>>,
    /// The command palette, while it is open.
    pub palette: Option<crate::palette::PaletteView<'a>>,
    /// The link field (Ctrl+K), while it is open; and where the link the
    /// pointer rests on goes, with the pointer's place, for the faint card
    /// beside it.
    pub link_field: Option<LinkFieldView<'a>>,
    pub link_card: Option<(&'a str, f32, f32)>,
    /// The note's headings, shown along the scroll bar while it is out.
    pub landmarks: &'a [Landmark<'a>],
    /// A note dragged from the file tree: its name, under the pointer.
    pub carried: Option<(&'a str, f32, f32)>,
    /// Files dragged in from outside, before they are let go.
    pub arriving: Option<Arriving<'a>>,
    /// Text being composed with an input method, and the part of it being
    /// converted: underlined.
    pub marked: Option<Composed>,
    /// The pictures notes show, and the screen's pixels per DIP (pictures
    /// are decoded at the size they are drawn).
    pub images: Option<&'a std::cell::RefCell<dyn crate::images::PictureSource>>,
    pub scale: f32,
    /// A picture being resized: its line's start and its width so far.
    pub picture_resize: Option<(usize, f32)>,
    /// The side menu's social drawer is open; the preview, when shown.
    pub social_drawer: bool,
    pub social: Option<crate::social::SocialView>,
    /// Misspelled words, by line (see `spell`).
    pub spelling: Option<&'a crate::spell::Marks>,
    /// The note's dates, small above its title; when the file tree row
    /// under the pointer was edited.
    pub page_date: Option<&'a str>,
    pub row_date: Option<(usize, &'a str)>,
    /// What the note is about, after its dates, and how visible (it fades
    /// while typing).
    pub page_gist: Option<(&'a str, f32)>,
    /// Today, for the dates in tasks: tinted, and faintly red once due.
    pub today: Option<kolada_core::when::Day>,
    /// How many tasks are late or due today: a teal dot beside the dates.
    pub due_now: usize,
    /// The notes opened last, newest first (vault-relative, with `.md`):
    /// Recent, above the vault's tree.
    pub recent: &'a [String],
    /// Notes and folders kept at hand, first in the side menu (a folder's
    /// ends with `/`).
    pub favorites: &'a [String],
    /// Side menu sections shut (`session::FOLD_FAVORITES`, `FOLD_RECENT`).
    pub folded: u8,
    /// The Today sheet, while it is down (or on its way).
    pub agenda: Option<crate::agenda::AgendaView<'a>>,
    /// Text the writing model just changed, washed in the accent, and how
    /// strongly (it fades out).
    pub wash: Option<(&'a [(usize, usize)], f32)>,
    /// Related notes in the margin; one being dragged, at a point; and where
    /// it would link if dropped.
    pub chips: &'a [Chip<'a>],
    pub dragged: Option<(usize, f32, f32)>,
    pub drop_at: Option<usize>,
    /// A note that just opened fades in and rises into place (0..1).
    pub page_fade: f32,
    /// The page of highlights is showing.
    pub highlights_open: bool,
    pub peek: Option<Peek<'a>>,
    pub whisper: Option<Whisper<'a>>,
    pub popup: Option<PopupView<'a>>,
    /// Related notes are hidden (an option).
    pub no_margin: bool,
    /// A note in a window of its own: no side menu, and a button that
    /// takes the note back to the main window.
    pub compact: bool,
    /// The note alone, with none of Kolada's own chrome (no side menu,
    /// toolbar or buttons): the shell draws its own around it, as the
    /// iPhone's native one does. A touch anywhere reaches the note.
    pub bare: bool,
    /// How much room each panel takes in the layout, 0 (collapsed) to 1
    /// (docked); in between while animating.
    pub sidebar_dock: f32,
    pub toolbar_dock: f32,
    /// How visible each panel is, 0 to 1. Above `*_dock` while a collapsed
    /// panel is revealed on hover: it then floats over the note.
    pub sidebar_show: f32,
    pub toolbar_show: f32,
    pub hover: Option<Target>,
    /// The collapsed pills' strings, as the pointer pulls them (empty: at
    /// rest).
    pub sidebar_wave: &'a [f32],
    pub toolbar_wave: &'a [f32],
    pub maximized: bool,
    /// The OS draws the window's buttons (the Mac's traffic lights): the
    /// chrome draws none of its own.
    pub native_caption: bool,
    /// The OS this is drawn for: the options it has differ.
    pub platform: Platform,
    /// The note's past, being scrubbed through: `text` and `lines` are then
    /// the version shown.
    pub scrub: Option<crate::scrub::ScrubView<'a>>,
    /// The app updates itself here (not from a store), so the options offer
    /// the switch; and the version downloaded, checked and ready, for the
    /// pill above the side menu's icons.
    pub updater: bool,
    pub update: Option<&'a str>,
    /// Settings > Updates' "Check now", as far as it has gone.
    pub update_check: UpdateCheck<'a>,
    /// Beam runs here: its row at the side menu's foot and the window's
    /// button (None: neither).
    pub quick: Option<BeamView<'a>>,
    /// Settings > Devices (None: pairing does not run here).
    pub devices: Option<DevicesView<'a>>,
    /// The options' Show Markdown: marks show faintly at the caret, as
    /// they are written. Off, editing is clean (see `NoteText::clean`).
    pub show_marks: bool,
    /// The share card, open under Send (see `share`).
    pub share: Option<crate::share::ShareView<'a>>,
    /// The writing model's card at the note area's bottom right, or the
    /// strip of its download (see `modelcard`).
    pub model_card: Option<crate::modelcard::ModelCardView<'a>>,
    /// The welcome screen, over the whole window (see `welcome`).
    pub welcome: Option<crate::welcome::View<'a>>,
    /// The selection bar over text selected with the pointer, as far as it
    /// has come in (0..1; see `selbar`); none while it is away.
    pub selection_bar: Option<f32>,
    /// The toolbar's Aa card, open, as far as it has faded in (0..1).
    pub reading: Option<f32>,
    /// The button whose name shows under it (the pointer rested on it).
    pub tooltip: Option<Target>,
    /// A picture opened full size, over everything (see `lightbox`).
    pub lightbox: Option<&'a crate::lightbox::Lightbox>,
    /// The notes as a grid of boxes under the note, when that is the view
    /// chosen (see `grid`).
    pub grid: Option<crate::grid::GridView<'a>>,
}

/// A text's direction: most of its letters decide, a tie goes to the
/// first (`kolada_markdown::direction`). Some(true) for Hebrew or Arabic,
/// Some(false) for other letters, None if it has none.
pub fn direction(text: &str) -> Option<bool> {
    kolada_markdown::direction(text)
}

pub fn is_rtl(text: &str) -> bool {
    direction(text) == Some(true)
}

/// A layout of `text` in a box `width` wide (and `height` tall, for formats
/// that center vertically).
/// A layout kept between frames: its text (to tell hash twins apart), its
/// format (held, so no other format takes its address while it is kept),
/// and the frame it was last used in.
struct Kept {
    text: String,
    layout: Rc<Layout>,
    used: u64,
}

/// Kept layouts by what made them: their text's hash, format, box and
/// styling; and the frame being built.
#[derive(Default)]
struct Keep {
    frame: u64,
    layouts: std::collections::HashMap<(u64, u64, u32, u32, u64, u32), Kept>,
}

thread_local! {
    static KEEP: std::cell::RefCell<Keep> = std::cell::RefCell::new(Keep::default());
}

/// A new frame is being built: layouts no frame has used for a few are let
/// go.
pub fn next_frame() {
    crate::math::next_frame();
    KEEP.with(|k| {
        let mut k = k.borrow_mut();
        k.frame += 1;
        let frame = k.frame;
        k.layouts.retain(|_, kept| kept.used + 3 >= frame);
    });
}

/// Forgets every kept layout (for tests that compare against none kept).
#[doc(hidden)]
pub fn forget_kept() {
    KEEP.with(|k| k.borrow_mut().layouts.clear());
}

/// A 64-bit hash of `s`.
pub fn text_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// The layout `make` makes for `text` in `format`, `width` x `height`,
/// styled as `style` says (what else it depends on, hashed), kept from an
/// earlier frame when one made the same. A layout from here is never
/// changed after it is made, so frames can share it (and its recorded
/// glyphs: see `display`).
pub fn kept_layout(
    eng: &Eng,
    text: &str,
    format: &Format,
    width: f32,
    height: f32,
    style: u64,
    make: impl FnOnce() -> Layout,
) -> Rc<Layout> {
    // A change of typeface (the Font option) makes every kept layout stale.
    let generation = eng.borrow().generation();
    let key = (text_hash(text), format.id(), width.to_bits(), height.to_bits(), style, generation);
    let found = KEEP.with(|k| {
        let mut k = k.borrow_mut();
        let frame = k.frame;
        k.layouts.get_mut(&key).filter(|kept| kept.text == text).map(|kept| {
            kept.used = frame;
            kept.layout.clone()
        })
    });
    if let Some(layout) = found {
        return layout;
    }
    let layout = Rc::new(make());
    KEEP.with(|k| {
        let mut k = k.borrow_mut();
        let used = k.frame;
        k.layouts.insert(key, Kept { text: text.to_string(), layout: layout.clone(), used });
    });
    layout
}

/// A layout of `text`, read in the direction its first letter says.
pub fn layout(eng: &Eng, text: &str, format: &Format, width: f32, height: f32) -> Rc<Layout> {
    layout_directed(eng, text, format, width, height, is_rtl(text))
}

/// A layout of `text` read right to left or left to right as told.
pub fn layout_directed(eng: &Eng, text: &str, format: &Format, width: f32, height: f32, rtl: bool) -> Rc<Layout> {
    Rc::new(layout_styled(eng, text, &Format { rtl: Some(rtl), ..*format }, width, height, &[]))
}

/// A layout with styles over ranges of it (the format says its direction).
pub fn layout_styled(eng: &Eng, text: &str, format: &Format, width: f32, height: f32, styles: &[kolada_text::Style]) -> Layout {
    Layout::new(&mut eng.borrow_mut(), text, format, width, height, styles)
}

pub fn build(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32) -> Result<Scene> {
    next_frame();
    let mut scene = Scene { column: chrome::column(input, width, height), ..Scene::default() };
    note_text(eng, fonts, input, height, &mut scene)?;
    // A note that just opened fades in.
    if input.page_fade < 1.0 {
        let fade = input.page_fade.clamp(0.0, 1.0);
        scene.fills.iter_mut().for_each(|f| f.alpha *= fade);
        scene.texts.iter_mut().for_each(|t| t.alpha *= fade);
        scene.pictures.iter_mut().for_each(|p| p.alpha *= fade);
    }
    if let Some((left, w)) = margin(scene.column, width) {
        for (i, chip) in input.chips.iter().enumerate() {
            let hover = input.hover == Some(Target::Chip(i)) && input.dragged.is_none();
            let lifted = input.dragged.is_some_and(|(d, _, _)| d == i);
            let r = Rect::new(left + chip.dx, chip.y, left + chip.dx + w, chip.y + CHIP_HEIGHT);
            draw_chip(eng, fonts, chip, r, if lifted { 0.35 } else { 1.0 }, hover, &mut scene)?;
            if chip.alpha > 0.5 {
                scene.note_hits.push((r, Target::Chip(i)));
            }
        }
    }
    scene.chrome_from = (scene.fills.len(), scene.texts.len());
    scene.in_chrome = true;
    chrome::layout(eng, fonts, input, width, height, &mut scene)?;
    // The page's scroll bar, along the window's right edge.
    if let Some(bar) = input.scrollbar {
        let track = chrome::scroll_track(scene.column, width, height);
        let hot = input.hover == Some(Target::Scrollbar) || input.dragging == Some(Target::Scrollbar);
        if bar.alpha > 0.01 || hot {
            chrome::scroll_thumb(&mut scene, track, bar.at, bar.size, hot, if hot { 1.0 } else { bar.alpha });
        }
        scene.hits.push((track, Target::Scrollbar));
    }
    // The note's headings along the scroll bar: a map of it, out while the
    // bar is, to jump by.
    if let Some(bar) = input.scrollbar
        && input.landmarks.len() >= 2
    {
        let over = matches!(input.hover, Some(Target::Heading(_)));
        let shown = if over { 1.0 } else { bar.alpha };
        if shown > 0.02 {
            chrome::landmarks(eng, fonts, input, scene.column, width, height, bar, shown, &mut scene)?;
        }
    }
    // Where the matches are along the note, beside the scroll bar.
    if input.find.is_some() && input.scrollbar.is_some() && !input.ticks.is_empty() {
        let track = chrome::scroll_track(scene.column, width, height);
        for &t in input.ticks {
            let y = track.top + (track.height() - 2.0) * t.clamp(0.0, 1.0);
            scene.fills.push(Fill::faded(Rect::new(track.right - 9.0, y, track.right - 2.0, y + 2.0), 1.0, Ink::Accent, 0.9));
        }
    }
    if let Some(view) = &input.grid {
        crate::grid::draw(eng, fonts, input, view, width, height, &mut scene)?;
    }
    chrome::find_bar(eng, fonts, input, &mut scene)?;
    crate::scrub::draw(eng, fonts, input, width, height, &mut scene)?;
    // A word about what just happened, at the top of the page.
    if let Some(w) = &input.whisper
        && w.alpha > 0.01
    {
        let column = scene.column;
        let text = format!("\u{E73E}  {}", w.text);
        // The check mark is an icon (three bytes in UTF-8), set at the end.
        let mut mark = kolada_text::Style::new(0..3);
        mark.role = Some(Role::Icon);
        let at_end = Format { align: Align::End, rtl: Some(is_rtl(&text)), ..fonts.small };
        // Beside the find bar when it is open, clear of the window's
        // buttons, never past the note's left edge.
        let (right, top) = chrome::whisper_place(input, column, width);
        let w_room = (right - column.left).clamp(80.0, 360.0);
        let l = Rc::new(layout_styled(eng, &text, &at_end, w_room, 20.0, &[mark]));
        scene.texts.push(Text { x: right - w_room, y: top, layout: l, ink: Ink::Faint, alpha: w.alpha });
    }
    // A peek floats beside its chip, over the note.
    if let (Some(peek), Some((left, _)), None) = (&input.peek, margin(scene.column, width), &input.popup)
        && let Some(chip) = input.chips.get(peek.chip)
    {
        let a = peek.alpha.clamp(0.0, 1.0);
        let w = 340.0;
        let title = layout(eng, peek.title, &fonts.peek_title, w - 32.0, 100_000.0);
        let body = layout(eng, peek.body, &fonts.peek, w - 32.0, 100_000.0);
        let (th, bh) = (title.measure().height, body.measure().height);
        let card = Rect::new(left - 14.0 - w, chip.y - 6.0, left - 14.0, chip.y - 6.0 + 16.0 + th + 8.0 + bh + 16.0);
        let card = card.offset(0.0, (height - 16.0 - card.bottom).min(0.0));
        scene.fills.push(Fill { alpha: 1.4 * a, ..Fill::shadow(card.offset(0.0, 6.0), 12.0, 22.0, 1.0) });
        scene.fills.push(Fill { alpha: a * 0.9, ..Fill::shadow(card.offset(0.0, 1.0), 12.0, 3.0, 1.0) });
        scene.fills.push(Fill { alpha: a, ..Fill::shadow(card.offset(0.0, 6.0), 12.0, 18.0, 1.0) });
        scene.fills.push(Fill::faded(card, 12.0, Ink::Panel, a));
        scene.texts.push(Text { x: card.left + 16.0, y: card.top + 16.0, layout: title, ink: Ink::Text, alpha: a });
        scene.texts.push(Text { x: card.left + 16.0, y: card.top + 16.0 + th + 8.0, layout: body, ink: Ink::Muted, alpha: a });
    }
    // Beam's button, bottom right, over the note and under every card.
    chrome::beam_button(eng, fonts, input, width, height, &mut scene)?;
    // The welcome screen, over the whole window but for its buttons; the
    // writing model's download strip over it, at its foot.
    if let Some(view) = &input.welcome {
        crate::welcome::draw(eng, fonts, input, view, width, height, &mut scene)?;
    }
    // The writing model's card, over the window's middle (its strip at the
    // note's foot).
    if let Some(view) = &input.model_card {
        let area = Rect::new(chrome::content_left(input, width), 0.0, width, height);
        crate::modelcard::draw(eng, fonts, view, input.hover, Rect::new(0.0, 0.0, width, height), area, &mut scene)?;
    }
    // A note open over the page, above everything else.
    if let Some(view) = &input.popup {
        scene.popup = Some(popup(eng, fonts, input, view, width, height, &mut scene)?);
    }
    // A chip being dragged floats over everything, under the pointer.
    if let (Some((i, x, y)), Some((_, w))) = (input.dragged, margin(scene.column, width))
        && let Some(chip) = input.chips.get(i)
    {
        let r = Rect::new(x - 24.0, y - CHIP_HEIGHT / 2.0, x - 24.0 + w, y + CHIP_HEIGHT / 2.0);
        scene.fills.push(Fill::shadow(r.offset(0.0, 6.0), 10.0, 18.0, 1.6));
        draw_chip(eng, fonts, &Chip { alpha: 1.0, ..*chip }, r, 1.0, true, &mut scene)?;
    }
    if let Some(a) = &input.arriving {
        arriving(eng, fonts, a, width, height, &mut scene)?;
    }
    // A note carried from the file tree: into the text it links, out of the
    // window it opens in one of its own.
    if let Some((name, x, y)) = input.carried {
        scene.overlay(None);
        let l = single_line(eng, name, &fonts.label, 220.0, 32.0);
        // Narrowed to the name, so a name read right to left sits against it.
        let w = l.measure().width.min(220.0);
        let l = Rc::new(l.with_width(&mut eng.borrow_mut(), w + 0.5));
        let r = Rect::new(x - 12.0, y - 16.0, x + 26.0 + w + 14.0, y + 16.0);
        scene.fills.push(Fill::shadow(r.offset(0.0, 6.0), 10.0, 18.0, 1.6));
        scene.fills.push(Fill::solid(r, 10.0, Ink::Raised));
        let icon = layout(eng, "\u{E7C3}", &fonts.icon_small, 24.0, 32.0);
        scene.texts.push(Text { x: r.left + 6.0, y: r.top, layout: icon, ink: Ink::Faint, alpha: 1.0 });
        scene.texts.push(Text { x: r.left + 30.0, y: r.top, layout: l, ink: Ink::Text, alpha: 1.0 });
    }
    crate::social::card(eng, fonts, input, width, height, &mut scene)?;
    chrome::suggestions(eng, fonts, input, &mut scene)?;
    chrome::link_card(eng, fonts, input, width, height, &mut scene)?;
    crate::selbar::draw(eng, fonts, input, width, height, &mut scene)?;
    chrome::link_field(eng, fonts, input, width, height, &mut scene)?;
    chrome::options_page(eng, fonts, input, width, height, &mut scene)?;
    if let Some(v) = &input.palette {
        crate::palette::draw(eng, fonts, v, width, height, &mut scene)?;
    }
    if let Some(a) = &input.agenda {
        crate::agenda::draw(eng, fonts, a, input.hover, width, height, &mut scene)?;
    }
    if let Some(view) = &input.share {
        crate::share::draw(eng, fonts, view, input.hover, width, height, &mut scene)?;
    }
    chrome::reading(eng, fonts, input, width, &mut scene)?;
    chrome::menu(eng, fonts, input, width, height, &mut scene)?;
    chrome::tooltip(eng, fonts, input, width, height, &mut scene)?;
    if let Some(lightbox) = input.lightbox {
        crate::lightbox::draw(input, lightbox, width, height, &mut scene);
    }
    // Checkboxes sit above the text's own area, below the chrome.
    let above_note = scene.hits.iter().position(|(_, t)| *t == Target::Note).map_or(0, |i| i + 1);
    let note_hits = std::mem::take(&mut scene.note_hits);
    scene.hits.splice(above_note..above_note, note_hits);
    centre_icons(eng, &mut scene, input.scale);
    Ok(scene)
}

/// Every icon laid out centred in its box has its ink in the box's middle,
/// within half a pixel at `scale`, not where its font's line puts it
/// (`chrome::ink_middle`): once, for every button, tile and row that draws
/// one, so an icon never sits high in its highlight or off the line of the
/// icons beside it. A scene built without its scale (0) is centred as it
/// is drawn instead (`display::record` moves each icon by `icon_shift`).
pub fn centre_icons(eng: &Eng, scene: &mut Scene, scale: f32) {
    if scale <= 0.0 || scene.icons_centred {
        return;
    }
    scene.icons_centred = true;
    for t in &mut scene.texts {
        let (dx, dy) = icon_shift(eng, t, scale);
        t.x += dx;
        t.y += dy;
    }
}

/// How far to move `t`, if it is an icon laid out centred in its box, for
/// its ink to be in the box's middle at `scale`: whole pixels, so its
/// pixels stay its own. Its ink's own offset in its box: applied once.
pub fn icon_shift(eng: &Eng, t: &Text, scale: f32) -> (f32, f32) {
    let l = &t.layout;
    if l.role() != Role::Icon || scale <= 0.0 {
        return (0.0, 0.0);
    }
    let f = l.format();
    if f.align != Align::Center || !f.center_y {
        return (0.0, 0.0);
    }
    let (px, py) = (t.x * scale, t.y * scale);
    let (fx, fy) = (px - px.floor(), py - py.floor());
    let Some((ix, iy)) = chrome::ink_middle(eng, l, (fx, fy), scale) else { return (0.0, 0.0) };
    // The box's middle, and the ink's, in pixels.
    let (mx, my) = (px + l.width() * scale / 2.0, py + l.box_height() * scale / 2.0);
    let (ix, iy) = (px.floor() + ix, py.floor() + iy);
    ((mx - ix).round() / scale, (my - iy).round() / scale)
}

/// Text being composed with an input method, as byte ranges: all of it, and
/// the part being converted.
pub type Composed = ((usize, usize), Option<(usize, usize)>);

/// Files dragged in from outside, before they are let go: a card under the
/// pointer (what they are, and what they become), the note stepping back
/// when they land elsewhere, and where pictures will go in it.
pub struct Arriving<'a> {
    pub title: &'a str,
    pub change: &'a str,
    /// The card's corner, DIPs; its opacity.
    pub x: f32,
    pub y: f32,
    pub alpha: f32,
    /// Something can be taken: the card in the accent; else muted.
    pub readable: bool,
    /// More than one: a second card behind.
    pub stack: bool,
    /// How far the note steps back (0..1).
    pub dim: f32,
    /// Pictures: the end of the line they land after, the gap's height, and
    /// how many.
    pub landing: Option<(usize, f32, usize)>,
}

fn arriving(eng: &Eng, fonts: &Fonts, a: &Arriving, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    scene.overlay(None);
    if a.dim > 0.01 {
        let c = scene.column;
        scene.fills.push(Fill::faded(Rect::new((c.left - 40.0).max(0.0), 0.0, (c.right + 40.0).min(width), height), 0.0, Ink::Page, 0.5 * a.dim));
    }
    let title = single_line(eng, a.title, &fonts.label, 280.0, 20.0);
    let change = single_line(eng, a.change, &fonts.small, 280.0, 18.0);
    let (t, c) = (title.measure(), change.measure());
    let w = t.width.max(c.width).min(280.0) + 28.0;
    let r = Rect::new(a.x, a.y, a.x + w.max(140.0), a.y + 14.0 + t.height + 2.0 + c.height + 12.0);
    let (fill, ink) = if a.readable { (Ink::Accent, Ink::OnAccent) } else { (Ink::Raised, Ink::Muted) };
    if a.stack {
        scene.fills.push(Fill::faded(r.offset(5.0, 5.0), 10.0, fill, 0.45 * a.alpha));
    }
    scene.fills.push(Fill::shadow(r.offset(0.0, 6.0), 10.0, 18.0, 1.6 * a.alpha));
    scene.fills.push(Fill::faded(r, 10.0, fill, a.alpha));
    scene.texts.push(Text { x: r.left + 14.0, y: r.top + 12.0, layout: title, ink, alpha: a.alpha });
    scene.texts.push(Text { x: r.left + 14.0, y: r.top + 14.0 + t.height, layout: change, ink, alpha: 0.85 * a.alpha });
    Ok(())
}

/// The visible paragraphs, the selection behind them, what their hidden
/// marks stand for, and the caret's place.
fn note_text(eng: &Eng, fonts: &Fonts, input: &Input, bottom: f32, scene: &mut Scene) -> Result<()> {
    let column = scene.column;
    let text = input.text;
    let caret = input.selection.1;
    // A caret from an old snapshot may no longer fit the text; then it is
    // simply not drawn.
    let caret = (caret <= text.len() && text.is_char_boundary(caret)).then_some(caret);
    let nt = NoteText {
        eng,
        fonts,
        text,
        lines: input.lines,
        width: column.width(),
        title: note::title_line(text, input.lines),
        caret,
        clean: !input.show_marks,
        pictures: pictures_for(input, input.active_note).map(|p| note::Pictures { resize: input.picture_resize, ..p }),
        tables: Default::default(),
        folded: input.sections_folded,
    };
    let x = column.left;
    let mut y = column.top - input.scroll_px;

    // A note that just opened rises into place as it fades in.
    y += (1.0 - input.page_fade.clamp(0.0, 1.0)) * 10.0;

    // When the note was written, small above its title; then what it is
    // about, on the same line, where there is room for it.
    if let Some(date) = input.page_date
        && input.scroll_line == 0
    {
        // A passing word (see `Whisper`) takes their row for its moment.
        let shown = 1.0 - input.whisper.as_ref().map_or(0.0, |w| w.alpha.clamp(0.0, 1.0));
        let l = kept_layout(eng, date, &fonts.small, column.width(), 18.0, 0, || {
            layout_styled(eng, date, &Format { rtl: Some(is_rtl(date)), ..fonts.small }, column.width(), 18.0, &[])
        });
        // Something due: a teal dot just after them; clicked, the Today
        // sheet comes down. What the note is about moves along for it.
        let dot = if input.due_now > 0 { DOT_ROOM } else { 0.0 };
        if dot > 0.0 {
            let left = x + l.measure().width + 7.0;
            let d = Rect::new(left, y - 18.0, left + 7.0, y - 11.0);
            scene.fills.push(Fill::faded(d, 3.5, Ink::Accent, shown));
            scene.note_hits.push((d.inset(-5.0), Target::DueDot));
            scene.due_dot = Some(d);
        }
        if let Some((gist, alpha)) = input.page_gist.filter(|g| g.1 > 0.01) {
            let dates = l.measure().width + dot;
            let room = column.width() - dates;
            if room >= GIST_ROOM {
                let line = format!(" \u{00B7} {gist}");
                let g = kept_layout(eng, &line, &fonts.small, room, 18.0, 1, || {
                    // Beside the dates whatever its script, cut with an ellipsis.
                    layout_styled(eng, &line, &Format { rtl: Some(false), ellipsis: true, ..fonts.small }, room, 18.0, &[])
                });
                scene.texts.push(Text { x: x + dates, y: y - 24.0, layout: g, ink: Ink::Faint, alpha: alpha * shown });
            }
        }
        // Clicked, they open the note's past (see `scrub`).
        let hot = input.hover == Some(Target::PageDate) || input.scrub.is_some();
        scene.note_hits.push((Rect::new(x - 4.0, y - 26.0, x + l.measure().width + 4.0, y - 4.0), Target::PageDate));
        scene.texts.push(Text { x, y: y - 24.0, layout: l, ink: if hot { Ink::Muted } else { Ink::Faint }, alpha: shown });
    }

    // An empty fresh page: a faint hint where the title will be, gone with
    // the first keystroke. With the grid, the caret alone waits.
    if text.is_empty() && input.grid.is_none() {
        let hint = layout(eng, "Start writing", &fonts.title, column.width(), 100_000.0);
        scene.texts.push(Text { x, y, layout: hint, ink: Ink::Faint, alpha: 1.0 });
    }

    let then = input.scrub.as_ref().map(|s| (s.changed, s.past));
    let page = Page { nt: &nt, column, first: input.lines.partition_point(|l| l.start < input.scroll_line), y, bottom, hair: 1.0 / input.scale.max(1.0), then, ring: !input.bare, carets: input.carets, hover: input.hover };
    let landing = input.arriving.as_ref().and_then(|a| a.landing);
    let drawn = draw_page(eng, fonts, &page, input.selection, input.drop_at, landing, input.marked, input.matches, input.spelling, input.wash, input.today, scene)?;
    scene.caret = drawn.caret;
    scene.carets = drawn.carets;
    scene.lines = drawn.lines;
    scene.selection_span = selection_span(&drawn.selected);
    for (r, at) in drawn.checkboxes {
        scene.note_hits.push((r, Target::Checkbox(at)));
    }
    // Pictures take clicks, and their right edges a drag that resizes.
    for (r, at) in drawn.pictures {
        scene.note_hits.push((r, Target::Picture(at)));
        scene.note_hits.push((Rect::new(r.right - 8.0, r.top, r.right + 10.0, r.bottom + 10.0), Target::PictureEdge(at)));
        if let Some((_, w)) = input.picture_resize.filter(|(line, _)| *line == at) {
            scene.picture_label = Some((r, format!("{:.0} \u{00D7} {:.0}", w, r.height())));
        }
    }
    Ok(())
}

/// The selection's first line and its last, from its boxes: each the
/// union of the boxes on that line.
fn selection_span(boxes: &[Rect]) -> Option<(Rect, Rect)> {
    let union = |on: &dyn Fn(&Rect) -> bool| {
        boxes.iter().filter(|r| on(r)).fold(None, |u: Option<Rect>, r| Some(u.map_or(*r, |u| Rect::new(u.left.min(r.left), u.top.min(r.top), u.right.max(r.right), u.bottom.max(r.bottom)))))
    };
    let top = boxes.iter().map(|r| r.top).fold(f32::INFINITY, f32::min);
    let bottom = boxes.iter().map(|r| r.bottom).fold(f32::NEG_INFINITY, f32::max);
    Some((union(&|r| r.top < top + 1.0)?, union(&|r| r.bottom > bottom - 1.0)?))
}

/// The pictures of the note at `note` (vault-relative, without `.md`).
fn pictures_for<'a>(input: &Input<'a>, note: &str) -> Option<note::Pictures<'a>> {
    let images = input.images?;
    let vault = std::path::Path::new(input.vault_path);
    let dir = vault.join(note).parent().map(std::path::Path::to_path_buf).unwrap_or_else(|| vault.to_path_buf());
    Some(note::Pictures { images, vault, dir, scale: input.scale.max(1.0), resize: None })
}

/// Where a run of paragraphs goes: its text, the column, the first line
/// drawn and where its top goes, and the bottom drawing stops at.
struct Page<'a> {
    nt: &'a NoteText<'a>,
    column: Rect,
    first: usize,
    y: f32,
    bottom: f32,
    /// One device pixel, DIPs: a table's hairlines.
    hair: f32,
    /// A past version of the note (see `scrub`): which of its lines differ
    /// from now, and how far into the past the page has gone.
    then: Option<(&'a [bool], f32)>,
    /// A chosen picture is ringed, with its handle: not where the shell
    /// draws its own (a bare note's knobs).
    ring: bool,
    /// The other carets and their selections (see `Input::carets`).
    carets: &'a [(usize, usize)],
    /// What the pointer is over: a heading's fold mark shows for it.
    hover: Option<Target>,
}

/// What `draw_page` drew: the paragraphs and where, the caret's place, the
/// checkboxes to click (with their lines' offsets), and where the text
/// ended (below everything, if it measured every line).
struct Drawn {
    lines: Vec<Line>,
    caret: Option<Rect>,
    carets: Vec<Rect>,
    checkboxes: Vec<(Rect, usize)>,
    /// The pictures, with their lines' offsets.
    pictures: Vec<(Rect, usize)>,
    /// The selection's boxes, a line's part at a time, top to bottom.
    selected: Vec<Rect>,
    end: f32,
}

/// Draws paragraphs from `page.first` down to `page.bottom`, with the
/// selection behind them, what their hidden marks stand for, and the
/// caret's place (and where a dragged chip would drop).
#[allow(clippy::too_many_arguments)]
fn draw_page(
    eng: &Eng,
    fonts: &Fonts,
    page: &Page,
    selection: (usize, usize),
    drop_at: Option<usize>,
    landing: Option<(usize, f32, usize)>,
    marked: Option<Composed>,
    matches: &[(usize, usize)],
    spelling: Option<&crate::spell::Marks>,
    wash: Option<(&[(usize, usize)], f32)>,
    today: Option<kolada_core::when::Day>,
    scene: &mut Scene,
) -> Result<Drawn> {
    let (nt, column) = (page.nt, page.column);
    let text = nt.text;
    let x = column.left;
    let mut y = page.y;
    let caret = nt.caret;
    let (from, to) = (selection.0.min(selection.1), selection.0.max(selection.1));
    let selected = from < to && to <= text.len() && text.is_char_boundary(from) && text.is_char_boundary(to);
    let mut drawn = Drawn { lines: Vec::new(), caret: None, carets: Vec::new(), checkboxes: Vec::new(), pictures: Vec::new(), selected: Vec::new(), end: y };
    // A code block's panel: where it starts in `fills` (under everything
    // drawn for its lines) and its top.
    let mut panel: Option<(usize, f32)> = None;
    let mut next = page.first;
    while next < nt.lines.len() {
        let i = next;
        next += 1;
        if y > page.bottom {
            break;
        }
        // Folded away under a heading: on past the section.
        if let Some(after) = nt.past_fold(i) {
            next = after;
            continue;
        }
        let para = nt.para(i)?;
        let h = para.height();
        let top = y + para.pad;
        // Fills of this line go here, under what it draws next.
        let fills_at = scene.fills.len();
        let texts_at = scene.texts.len();
        let shapes_at = scene.shapes.len();
        let (start, end) = (para.start, para.end);
        let code = matches!(para.line.block, Block::Fence | Block::Code);
        if code && panel.is_none() {
            panel = Some((scene.fills.len(), top));
        }
        // Misspelled words, dotted underneath; not the word being typed.
        if let Some(marks) = spelling.and_then(|m| m.get(&crate::spell::line_key(&text[start..end]))) {
            for &(a, b) in marks {
                let (a, b) = (start + a as usize, start + b as usize);
                let composed = marked.is_some_and(|((ms, me), _)| a < me && b > ms);
                if composed || caret.is_some_and(|c| a <= c && c <= b) || b > end || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
                    continue;
                }
                for [l, _, r, bot] in nt.range(&para, a, b)? {
                    let y = top + bot - 2.5;
                    let mut dx = l;
                    while dx + 1.6 <= r {
                        scene.fills.push(Fill::faded(Rect::new(x + dx, y, x + dx + 1.8, y + 1.8), 0.9, Ink::Danger, 0.85));
                        dx += 3.2;
                    }
                }
            }
        }
        // What the find bar found, tinted.
        let first = matches.partition_point(|&(_, e)| e <= start);
        for &(a, b) in matches[first..].iter().take_while(|&&(a, _)| a < end.max(start + 1)) {
            if a < b && text.is_char_boundary(a) && text.is_char_boundary(b.min(end)) && a.max(start) < b.min(end) {
                for [l, t, r, bot] in nt.range(&para, a.max(start), b.min(end))? {
                    scene.fills.push(Fill::faded(Rect::new(x + l - 1.0, top + t + 1.0, x + r + 1.0, top + bot - 1.0), 3.0, Ink::Accent, 0.22));
                }
            }
        }
        // What the writing model just changed, fading.
        if let Some((ranges, alpha)) = wash {
            for &(a, b) in ranges {
                let (a, b) = (a.max(start), b.min(end));
                if a < b && text.is_char_boundary(a) && text.is_char_boundary(b) {
                    for [l, t, r, bot] in nt.range(&para, a, b)? {
                        scene.fills.push(Fill::faded(Rect::new(x + l - 1.0, top + t + 1.0, x + r + 1.0, top + bot - 1.0), 3.0, Ink::Accent, alpha));
                    }
                }
            }
        }
        // Behind inline code and marked text.
        for s in &para.spans {
            let (ink, pad) = match s.style {
                Style::Code => (Ink::Field, 3.0),
                Style::Highlight => (Ink::Mark, 1.0),
                _ => continue,
            };
            if s.text_start < s.text_end {
                for [l, t, r, b] in nt.range(&para, s.text_start, s.text_end)? {
                    // Marked text gets a highlighter's stroke across its
                    // lower half; code a soft box.
                    let upper = if s.style == Style::Highlight { t + (b - t) * 0.42 } else { t + 3.0 };
                    let r = Rect::new(x + l - pad, top + upper, x + r + pad, top + b - 3.0);
                    scene.fills.push(Fill::solid(r, 4.0, ink));
                }
            }
        }
        // An open task's dates: a soft pill, faintly red once due (a
        // repeating one stays quiet: its turn depends on its ticks).
        if let (Some(today), Block::Task { done: false }) = (today, para.line.block) {
            for (i, due) in kolada_core::when::find_all(&text[start..end], today) {
                let ink = if due.repeat.is_none() && due.day <= today { Ink::Mark } else { Ink::Field };
                for [l, t, r, b] in nt.range(&para, start + i, start + i + 1 + due.len)? {
                    scene.fills.push(Fill::solid(Rect::new(x + l - 3.0, top + t + 2.0, x + r + 3.0, top + b - 2.0), 5.0, ink));
                }
            }
        }
        if selected && from <= end && to > start && para.picture.is_none() {
            for [l, t, r, b] in nt.range(&para, from.max(start), to.min(end))? {
                let r = Rect::new(x + l, top + t, x + r, top + b);
                scene.fills.push(Fill::solid(r, 3.0, Ink::Highlight));
                drawn.selected.push(r);
            }
            // The line break is selected too: a short mark past the end.
            if to > end && end < text.len() {
                let (cx, cy, ch) = nt.caret(&para, end)?;
                let mark = Rect::new(x + cx, top + cy, x + cx + BREAK_WIDTH, top + cy + ch);
                scene.fills.push(Fill::solid(mark, 3.0, Ink::Highlight));
            }
        }
        // Text being composed with an input method: underlined, the part
        // being converted more strongly, in the accent.
        if let Some(((ms, me), target)) = marked.filter(|((ms, me), _)| ms < me && *ms <= end && *me >= start) {
            let (a, b) = (ms.max(start), me.min(end));
            if a < b && text.is_char_boundary(a) && text.is_char_boundary(b) {
                for [l, _, r, bot] in nt.range(&para, a, b)? {
                    scene.fills.push(Fill::solid(Rect::new(x + l, top + bot - 2.0, x + r, top + bot - 0.8), 0.5, Ink::Muted));
                }
            }
            if let Some((ta, tb)) = target.map(|(ta, tb)| (ta.max(start), tb.min(end))).filter(|(ta, tb)| ta < tb && text.is_char_boundary(*ta) && text.is_char_boundary(*tb)) {
                for [l, _, r, bot] in nt.range(&para, ta, tb)? {
                    scene.fills.push(Fill::solid(Rect::new(x + l, top + bot - 2.6, x + r, top + bot - 0.4), 1.0, Ink::Accent));
                }
            }
        }
        if let Some(r) = decorate(eng, fonts, nt, &para, column, top, h, scene)? {
            drawn.checkboxes.push((r, para.line.start));
        }
        if let Some(c) = caret.filter(|c| (start..=end).contains(c) && para.picture.is_none()) {
            let (cx, cy, ch) = nt.caret(&para, c)?;
            let left = x + cx - CARET_WIDTH / 2.0;
            drawn.caret = Some(Rect::new(left, top + cy, left + CARET_WIDTH, top + cy + ch));
        }
        // The other carets, and what each has selected, as the main one.
        if para.picture.is_none() {
            for &(anchor, head) in page.carets {
                let (a, b) = (anchor.min(head).max(start), anchor.max(head).min(end));
                if a < b && text.is_char_boundary(a) && text.is_char_boundary(b) {
                    for [l, t, r, bt] in nt.range(&para, a, b)? {
                        scene.fills.insert(fills_at, Fill::solid(Rect::new(x + l, top + t, x + r, top + bt), 3.0, Ink::Highlight));
                    }
                }
                if (start..=end).contains(&head) && text.is_char_boundary(head) {
                    let (cx, cy, ch) = nt.caret(&para, head)?;
                    let left = x + cx - CARET_WIDTH / 2.0;
                    drawn.carets.push(Rect::new(left, top + cy, left + CARET_WIDTH, top + cy + ch));
                }
            }
        }
        fold_marks(fonts, nt, &para, i, page, top, scene)?;
        // Where a dragged chip would land as a link.
        if let Some(d) = drop_at.filter(|d| (start..=end).contains(d) && text.is_char_boundary(*d)) {
            let (cx, cy, ch) = nt.caret(&para, d)?;
            let r = Rect::new(x + cx - 1.0, top + cy + 2.0, x + cx + 1.0, top + cy + ch - 2.0);
            scene.fills.push(Fill::solid(r, 1.0, Ink::Link));
        }
        // A line that is only a picture: the picture, or a quiet box while
        // it is found and decoded.
        if let Some(pic) = &para.picture {
            let r = Rect::new(x + para.x, top, x + para.x + pic.width, top + pic.height);
            drawn.pictures.push((r, start));
            // Chosen: a ring around it; alone, with the handle that resizes
            // it on its right edge.
            if selected && from <= start && to >= end && page.ring {
                scene.fills.push(Fill::solid(r.inset(-3.0), PICTURE_ROUND + 3.0, Ink::Accent));
                scene.fills.push(Fill::solid(r.inset(-1.0), PICTURE_ROUND + 1.0, Ink::Page));
                if from == start && to == end {
                    let mid = (r.top + r.bottom) / 2.0;
                    let handle = Rect::new(r.right + 4.0, mid - 16.0, r.right + 9.0, mid + 16.0);
                    scene.fills.push(Fill::solid(handle, 2.5, Ink::Accent));
                    let corner = Rect::new(r.right + 1.0, r.bottom + 1.0, r.right + 9.0, r.bottom + 9.0);
                    scene.fills.push(Fill::solid(corner, 4.0, Ink::Accent));
                }
            }
            let ready = match (&pic.path, &nt.pictures) {
                (Some(path), Some(p)) => p.images.borrow_mut().pixels(path, (pic.width * p.scale).round().max(1.0) as u32),
                _ => None,
            };
            let alpha = ready.as_ref().map_or(0.0, |(_, a)| *a);
            if alpha < 1.0 {
                scene.fills.push(Fill::solid(r, PICTURE_ROUND, Ink::Field));
            }
            // Not in the vault: the picture glyph and its name, muted.
            if let Some(name) = &pic.missing {
                // Kept between frames: drawn again on every keystroke.
                let mark = Format { align: Align::Center, center_y: true, wrap: false, size: 20.0, rtl: Some(false), ..fonts.icon };
                let glyph = kept_layout(eng, "\u{E91B}", &mark, 36.0, r.height(), 0, || layout_styled(eng, "\u{E91B}", &mark, 36.0, r.height(), &[]));
                scene.texts.push(Text { x: r.left + 10.0, y: r.top, layout: glyph, ink: Ink::Faint, alpha: 1.0 });
                let label = Format { center_y: true, wrap: false, ellipsis: true, rtl: Some(is_rtl(name)), ..fonts.label };
                let room = (r.width() - 64.0).max(1.0);
                let named = kept_layout(eng, name, &label, room, r.height(), 0, || layout_styled(eng, name, &label, room, r.height(), &[]));
                scene.texts.push(Text { x: r.left + 52.0, y: r.top, layout: named, ink: Ink::Muted, alpha: 1.0 });
            }
            if let Some((pixels, alpha)) = ready {
                // Eased, like the page's other fades.
                let alpha = 1.0 - (1.0 - alpha) * (1.0 - alpha);
                scene.pictures.push(Picture { layer: scene.layer_now(), rect: r, pixels, alpha, round: PICTURE_ROUND });
            }
        }
        let closing = para.line.block == Block::Fence && panel.is_some_and(|(_, t)| t < top);
        let next_code = nt.lines.get(i + 1).is_some_and(|l| matches!(l.block, Block::Fence | Block::Code));
        if let Some(row) = &para.row {
            // A table row: the header on a soft panel, a hairline under each
            // row but the last, its cells in their columns.
            let left = x + para.x;
            if row.header && !row.cells.is_empty() {
                scene.fills.insert(fills_at, Fill::solid(Rect::new(left, top, left + row.width, top + h), 6.0, Ink::Field));
            }
            if !row.header && !row.last && !row.cells.is_empty() {
                scene.fills.push(Fill::faded(Rect::new(left, top + h - page.hair, left + row.width, top + h), 0.0, Ink::Text, 0.13));
            }
            for cell in &row.cells {
                scene.texts.push(Text { x: left + cell.x + note::CELL_PAD, y: top + note::CELL_VPAD, layout: cell.layout.clone(), ink: Ink::Text, alpha: 1.0 });
            }
        } else {
            scene.texts.push(Text { x: x + para.x, y: top, layout: para.layout.clone(), ink: para.ink(), alpha: 1.0 });
        }
        draw_math(&para, column, top, selected && from <= end && to > start, scene);
        // A past version: a line that differs from now marked in the margin,
        // the others faded as the page goes back.
        if let Some((changed, past)) = page.then {
            if changed.get(i).copied().unwrap_or(false) {
                // Nearer the text where the column hugs the window's edge.
                let gap = (column.left - 12.0).clamp(10.0, 18.0);
                let r = Rect::new(column.left - gap - 2.5, top + 4.0, column.left - gap, top + h - 4.0);
                scene.fills.push(Fill::faded(r, 1.25, Ink::Accent, 0.85 * past));
            } else {
                let fade = 1.0 - 0.6 * past.clamp(0.0, 1.0);
                scene.texts[texts_at..].iter_mut().for_each(|t| t.alpha *= fade);
                scene.fills[fills_at..].iter_mut().for_each(|f| f.alpha *= fade);
                scene.shapes[shapes_at..].iter_mut().for_each(|s| s.alpha *= fade);
            }
        }
        drawn.lines.push(Line { para, x, y: top, height: h });
        y = top + h;
        // Pictures dragged over the note: the gap they open after this
        // line, with a slot for each (up to four).
        if let Some((_, gap, count)) = landing.filter(|l| l.0 == end) {
            let r = Rect::new(column.left, y + 6.0, column.right, y + gap - 6.0);
            if r.height() > 2.0 {
                scene.fills.push(Fill::faded(r, 10.0, Ink::Accent, 0.10));
                let slot = (r.height() - 16.0).max(0.0);
                for i in 0..count.min(4) {
                    let left = r.left + 10.0 + i as f32 * (slot * 1.5 + 8.0);
                    if left + slot * 1.5 > r.right - 10.0 {
                        break;
                    }
                    scene.fills.push(Fill::faded(Rect::new(left, r.top + 8.0, left + slot * 1.5, r.top + 8.0 + slot), 6.0, Ink::Accent, 0.30));
                }
            }
            y += gap;
        }
        if code && (closing || !next_code)
            && let Some((at, t)) = panel.take()
        {
            scene.fills.insert(at, Fill::solid(Rect::new(column.left, t, column.right, y), 10.0, Ink::Field));
        }
    }
    if let Some((at, t)) = panel {
        scene.fills.insert(at, Fill::solid(Rect::new(column.left, t, column.right, y), 10.0, Ink::Field));
    }
    drawn.end = y;
    Ok(drawn)
}

/// The math a paragraph shows (see `note::MathView`): a block's formula
/// centred in the column (behind it the selection, when it is chosen), one
/// under its source while that is edited, and the formulas in its line,
/// each where its first letter holds its room.
fn draw_math(para: &Para, column: Rect, top: f32, chosen: bool, scene: &mut Scene) {
    let centred = |f: &crate::math::Formula| column.left + ((column.width() - f.width) / 2.0).max(0.0);
    match &para.math {
        Some(MathView::Block { formula, baseline }) => {
            let left = centred(formula);
            if chosen {
                let r = Rect::new(left - 4.0, top + baseline - formula.ascent - 4.0, left + formula.width + 4.0, top + baseline + formula.descent + 4.0);
                scene.fills.push(Fill::solid(r, 3.0, Ink::Highlight));
            }
            formula.draw(left, top + baseline, Ink::Text, 1.0, scene);
        }
        Some(MathView::Preview { formula, top: at }) => formula.draw(centred(formula), top + at + formula.ascent, Ink::Text, 1.0, scene),
        _ => {}
    }
    for (at, formula) in &para.inline {
        let place = match &para.row {
            Some(row) => row.cells.iter().find(|c| c.start <= *at && *at < c.end).and_then(|c| {
                let (gx, base) = c.layout.glyph_at(at - c.start)?;
                Some((column.left + para.x + c.x + note::CELL_PAD + gx, top + note::CELL_VPAD + base))
            }),
            None => para.layout.glyph_at(at - para.start).map(|(gx, base)| (column.left + para.x + gx, top + base)),
        };
        if let Some((x, baseline)) = place {
            formula.draw(x, baseline, para.ink(), 1.0, scene);
        }
    }
}

/// A heading's fold mark, in the margin before it: always on a folded one,
/// and on one with a section while the pointer is at its margin or the
/// caret is on it. A folded heading has the count of its lines at its end,
/// in a quiet capsule; either takes a click that opens it again.
#[allow(clippy::too_many_arguments)]
fn fold_marks(fonts: &Fonts, nt: &NoteText, para: &note::Para, i: usize, page: &Page, top: f32, scene: &mut Scene) -> Result<()> {
    if !matches!(para.line.block, Block::Heading(_)) || !nt.foldable(i) {
        return Ok(());
    }
    let (start, end) = (para.start, para.end);
    let folded = nt.folded.iter().find(|&&(from, _)| from == end).copied();
    let (_, cy, ch) = nt.caret(para, start)?;
    let mid = top + cy + ch / 2.0;
    let column = page.column;
    // Nearer the text where the column hugs the window's edge.
    let gap = (column.left - 6.0).clamp(4.0, 12.0);
    let size = 14.0;
    // Never past the window's edge, where the column hugs it.
    let left = (column.left - gap - size).max(2.0);
    let mark = Rect::new(left, mid - size / 2.0, left + size, mid + size / 2.0);
    let hit = Rect::new(mark.left - 8.0, top + cy, column.left, top + cy + ch);
    scene.note_hits.push((hit, Target::Fold(start)));
    let hot = page.hover == Some(Target::Fold(start));
    // Revealed on intent: the pointer at it, or the section folded; never
    // at rest beside a heading (a note opens with its caret on the title).
    let on = folded.is_some() || hot;
    if on {
        let logo = if folded.is_some() { crate::vector::Logo::FoldShut } else { crate::vector::Logo::FoldOpen };
        let ink = if hot { Ink::Muted } else { Ink::Faint };
        scene.vectors.push(VectorMark { layer: scene.layer_now(), rect: mark, logo, ink, alpha: 1.0 });
    }
    if let Some((from, to)) = folded {
        let lines = nt.lines.partition_point(|l| l.start <= to) - nt.lines.partition_point(|l| l.start <= from);
        let label = if lines == 1 { "1 line".to_string() } else { format!("{lines} lines") };
        let l = kept_layout(nt.eng, &label, &fonts.small, 200.0, 18.0, 0, || layout_styled(nt.eng, &label, &Format { rtl: Some(false), ..fonts.small }, 200.0, 18.0, &[]));
        let (ex, ey, eh) = nt.caret(para, end)?;
        let (w, h) = (l.measure().width + 16.0, 20.0);
        let left = column.left + para.x + ex + 10.0;
        let r = Rect::new(left, top + ey + (eh - h) / 2.0, left + w, top + ey + (eh + h) / 2.0);
        scene.fills.push(Fill::solid(r, h / 2.0, if hot { Ink::Hover } else { Ink::Field }));
        scene.texts.push(Text { x: r.left + 8.0, y: r.top + (h - l.measure().height) / 2.0, layout: l, ink: Ink::Faint, alpha: 1.0 });
        scene.note_hits.push((r, Target::Fold(start)));
    }
    Ok(())
}

/// The note that opened over the page, as the view shows it.
pub struct PopupView<'a> {
    pub name: &'a str,
    /// Text being composed in it, and the part being converted.
    pub marked: Option<Composed>,
    pub text: &'a str,
    pub lines: &'a [kolada_markdown::Line],
    pub selection: (usize, usize),
    /// How far its text is scrolled, DIPs; the height it opened beside; its
    /// opacity while it fades in.
    pub scroll: f32,
    pub anchor: f32,
    pub alpha: f32,
    /// Where it was dragged to, from where it opened (DIPs).
    pub offset: (f32, f32),
    /// What the pointer is over, whether its scroll bar is dragged, and how
    /// visible that is at rest (see `ScrollView`).
    pub hover: Option<Target>,
    pub scrolling: bool,
    pub bar_alpha: f32,
    pub spelling: Option<&'a crate::spell::Marks>,
    /// Its note, vault-relative without `.md` (where its pictures are
    /// found from).
    pub note: Option<&'a str>,
}

/// Where the popup's scroll bar goes: inside its card's right edge.
pub fn popup_track(card: Rect, content: Rect) -> Rect {
    Rect::new(card.right - 14.0, content.top + 4.0, card.right - 1.0, content.bottom - 4.0)
}

fn input_hover_is(view: &PopupView, target: Target) -> bool {
    view.hover == Some(target)
}

/// One line of `text` in a box, cut with an ellipsis where it does not fit,
/// right-aligned when it reads right to left.
pub fn single_line(eng: &Eng, text: &str, format: &Format, width: f32, height: f32) -> Rc<Layout> {
    layout(eng, text, &Format { wrap: false, ellipsis: true, center_y: true, ..*format }, width, height)
}

/// Where the popup went: the text's area, the paragraphs drawn, the
/// caret's place, and the height of all its text.
pub struct PopupDrawn {
    pub card: Rect,
    pub content: Rect,
    pub lines: Vec<Line>,
    pub caret: Option<Rect>,
    pub height: f32,
}

/// Popup size, and its header's height, DIPs.
const POPUP_WIDTH: f32 = 500.0;
const POPUP_HEIGHT: f32 = 620.0;
const POPUP_HEADER: f32 = 50.0;

/// A note open over the page, to read, scroll and edit without leaving it:
/// a card beside the margin (or over the column in a narrow window) with
/// the note's name, a button to open it as the page, one to close it, and
/// its text, clipped to the card.
fn popup(eng: &Eng, fonts: &Fonts, input: &Input, view: &PopupView, width: f32, height: f32, scene: &mut Scene) -> Result<PopupDrawn> {
    let column = scene.column;
    let w = POPUP_WIDTH.min(width - 48.0);
    let right = match margin(column, width) {
        Some((left, _)) => (left - 14.0).max(w + 24.0),
        None => ((column.left + column.right + w) / 2.0).min(width - 24.0),
    };
    // As tall as its text, up to a limit: a short note gets a small card.
    let text_width = w - 48.0;
    let (_, head) = view.selection;
    let caret = (head <= view.text.len() && view.text.is_char_boundary(head)).then_some(head);
    let title = note::title_line(view.text, view.lines);
    let nt = NoteText { eng, fonts, text: view.text, lines: view.lines, width: text_width, title, caret, clean: !input.show_marks, pictures: view.note.and_then(|n| pictures_for(input, n)), tables: Default::default(), folded: &[] };
    let mut text_height = 0.0;
    // Where the title line ends, from the top of the text.
    let mut title_bottom = 0.0;
    for i in 0..view.lines.len() {
        let p = nt.para(i)?;
        text_height += p.pad + p.height();
        if Some(i) == title {
            title_bottom = text_height;
        }
        if text_height > POPUP_HEIGHT && title.is_none_or(|t| i >= t) {
            break;
        }
    }
    let h = (POPUP_HEADER + 1.0 + 14.0 + text_height + 28.0).clamp(160.0, POPUP_HEIGHT.min(height - 48.0).max(160.0));
    let top = (view.anchor - 80.0).clamp(24.0, (height - 24.0 - h).max(24.0));
    // Dragged by its header, it goes anywhere its header stays in reach.
    let (dx, dy) = view.offset;
    let left = (right - w + dx).clamp(48.0 - w, width - 48.0);
    let top = (top + dy).clamp(0.0, height - POPUP_HEADER);
    let card = Rect::new(left, top, left + w, top + h);
    let a = view.alpha.clamp(0.0, 1.0);
    scene.overlay(None);
    scene.fills.push(Fill { alpha: 1.6 * a, ..Fill::shadow(card.offset(0.0, 10.0), 16.0, 36.0, 1.0) });
    scene.fills.push(Fill { alpha: a * 0.9, ..Fill::shadow(card.offset(0.0, 1.0), 16.0, 3.0, 1.0) });
    scene.fills.push(Fill { alpha: a, ..Fill::shadow(card.offset(0.0, 8.0), 16.0, 24.0, 1.0) });
    scene.fills.push(Fill::faded(card, 16.0, Ink::Panel, a));
    scene.hits.push((card, Target::PopupCard));
    scene.hits.push((Rect::new(card.left, card.top, card.right, card.top + POPUP_HEADER), Target::PopupHeader));

    // Header: the buttons. The note's title is its first line, in the text;
    // it comes up here, small, only once that line has scrolled away.
    let close = Rect::new(card.right - 12.0 - 32.0, card.top + 10.0, card.right - 12.0, card.top + 42.0);
    let open = close.offset(-36.0, 0.0);
    let away = ((view.scroll - title_bottom) / 16.0).clamp(0.0, 1.0);
    if away > 0.0 {
        let name = single_line(eng, view.name, &fonts.peek_title, open.left - 8.0 - (card.left + 22.0), POPUP_HEADER);
        scene.texts.push(Text { x: card.left + 22.0, y: card.top + 2.0, layout: name, ink: Ink::Text, alpha: a * away });
        let divider = Rect::new(card.left + 1.0, card.top + POPUP_HEADER, card.right - 1.0, card.top + POPUP_HEADER + 1.0);
        scene.fills.push(Fill::faded(divider, 0.0, Ink::Border, a * away));
    }
    for (r, target, glyph) in [(open, Target::PopupOpen, "\u{E8A7}"), (close, Target::PopupClose, "\u{E711}")] {
        if input_hover_is(view, target) {
            scene.fills.push(Fill::faded(r, 8.0, Ink::Hover, a));
        }
        let icon = layout(eng, glyph, &fonts.icon, r.width(), r.height());
        scene.texts.push(Text { x: r.left, y: r.top, layout: icon, ink: Ink::Muted, alpha: a });
        scene.hits.push((r, target));
    }

    // The text, scrolled, clipped to the card below the header.
    let content = Rect::new(card.left + 24.0, card.top + POPUP_HEADER + 1.0, card.right - 24.0, card.bottom - 10.0);
    scene.overlay(Some(content));
    let page = Page { nt: &nt, column: content, first: 0, y: content.top + 2.0 - view.scroll, bottom: f32::INFINITY, hair: 1.0 / input.scale.max(1.0), then: None, ring: !input.bare, carets: &[], hover: None };
    let fills_from = scene.fills.len();
    let texts_from = scene.texts.len();
    let drawn = draw_page(eng, fonts, &page, view.selection, None, None, view.marked, &[], view.spelling, None, None, scene)?;
    if a < 1.0 {
        scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= a);
        scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= a);
    }
    if let Some(c) = drawn.caret
        && c.bottom > content.top
        && c.top < content.bottom
    {
        scene.fills.push(Fill::faded(c, CARET_WIDTH / 2.0, Ink::Accent, a));
    }
    scene.hits.push((content, Target::PopupText));
    for (r, at) in &drawn.checkboxes {
        if r.top >= content.top && r.bottom <= content.bottom {
            scene.hits.push((*r, Target::PopupCheckbox(*at)));
        }
    }
    let height = drawn.end + view.scroll - content.top + 14.0;
    // Its scroll bar, inside the card's right edge, over the text's layer.
    if height > content.height() + 1.0 {
        scene.overlay(None);
        let track = popup_track(card, content);
        let max = height - content.height();
        let hot = view.hover == Some(Target::PopupScrollbar) || view.scrolling;
        let shown = if hot { 1.0 } else { view.bar_alpha };
        if shown > 0.01 {
            chrome::scroll_thumb(scene, track, view.scroll / max, content.height() / height, hot, a * shown);
        }
        scene.hits.push((track, Target::PopupScrollbar));
    }
    Ok(PopupDrawn { card, content, lines: drawn.lines, caret: drawn.caret, height })
}

/// A margin chip: the note's name and why it relates (the words in common),
/// or a marked passage and the note it is from.
#[allow(clippy::too_many_arguments)]
fn draw_chip(eng: &Eng, fonts: &Fonts, chip: &Chip, r: Rect, dim: f32, hover: bool, scene: &mut Scene) -> Result<()> {
    let strength = if hover { 1.0 } else { 0.6 + 0.4 * chip.strength.clamp(0.0, 1.0) };
    let a = chip.alpha.clamp(0.0, 1.0) * dim * strength;
    if a <= 0.01 {
        return Ok(());
    }
    let bg = if hover { Ink::Hover } else { Ink::Field };
    scene.fills.push(Fill::faded(r, 10.0, bg, a * 0.8));
    let (glyph, first, second, first_ink) = match chip.kind {
        Relation::Highlight => {
            // A passage is marked the way it is in its note.
            let bar = Rect::new(r.left + 12.0, r.top + 10.0, r.left + 15.0, r.bottom - 10.0);
            scene.fills.push(Fill::faded(bar, 1.5, Ink::Accent, a));
            (None, chip.detail.to_string(), format!("from {}", chip.name), Ink::Text)
        }
        Relation::Backlink => (Some("\u{E71B}"), chip.name.to_string(), chip.detail.to_string(), Ink::Muted),
        Relation::Similar => (Some("\u{E7C3}"), chip.name.to_string(), chip.detail.to_string(), Ink::Muted),
    };
    // Right to left, the chip mirrors: its icon goes to the right.
    let rtl = is_rtl(&first);
    let (text_left, text_right, icon_left) = if rtl { (r.left + 10.0, r.right - 30.0, r.right - 30.0) } else { (r.left + 30.0, r.right - 10.0, r.left + 6.0) };
    if let Some(glyph) = glyph {
        let icon = layout(eng, glyph, &fonts.icon_small, 24.0, 26.0);
        scene.texts.push(Text { x: icon_left, y: r.top + 6.0, layout: icon, ink: Ink::Faint, alpha: a });
    }
    let name = layout(eng, &first, &fonts.label, text_right - text_left, 22.0);
    let ink = if hover { Ink::Text } else { first_ink };
    scene.texts.push(Text { x: text_left, y: r.top + 5.0, layout: name, ink, alpha: a });
    let detail = layout_directed(eng, &second, &fonts.small, text_right - text_left, 18.0, rtl || is_rtl(&second));
    scene.texts.push(Text { x: text_left, y: r.top + 26.0, layout: detail, ink: Ink::Faint, alpha: a });
    Ok(())
}

/// What a paragraph's hidden block mark stands for: a bullet, number or
/// checkbox in the indentation before its text (after it, right to left),
/// bars beside a quote, a line for a rule.
#[allow(clippy::too_many_arguments)]
fn decorate(eng: &Eng, fonts: &Fonts, nt: &NoteText, para: &Para, column: Rect, top: f32, h: f32, scene: &mut Scene) -> Result<Option<Rect>> {
    let line = &para.line;
    // Beside a picture, the mark sits by its top, where a line of words'
    // would (as a number does), not halfway down it.
    let first = if para.picture.is_some() {
        let body = Format { rtl: Some(false), ..fonts.body };
        kept_layout(eng, "x", &body, 100.0, 100.0, 0, || layout_styled(eng, "x", &body, 100.0, 100.0, &[])).measure().height
    } else {
        nt.first_line_height(para)
    };
    let mid = top + first / 2.0;
    // The mark's slot: one indentation level wide, just before the text.
    let level = line.depth as f32 * LIST_INDENT;
    let (slot_left, slot_right) = if para.rtl {
        (column.right - level - LIST_INDENT, column.right - level)
    } else {
        (column.left + level, column.left + level + LIST_INDENT)
    };
    // Marks lean toward their text.
    let center = (slot_left + slot_right) / 2.0 + if para.rtl { 2.0 } else { -2.0 };
    match line.block {
        Block::Bullet => {
            let (radius, ink) = if line.depth == 0 { (2.75, Ink::Muted) } else { (2.25, Ink::Faint) };
            let dot = Rect::new(center - radius, mid - radius, center + radius, mid + radius);
            scene.fills.push(Fill::solid(dot, radius, ink));
        }
        Block::Ordered => {
            let label = nt.text[line.start..line.content()].trim();
            let width = LIST_INDENT + 16.0;
            // Numbers sit against their text: at the end of their slot.
            let l = if para.rtl {
                layout(eng, label, &fonts.body, width, 100.0)
            } else {
                Rc::new(layout_styled(eng, label, &Format { align: Align::End, rtl: Some(is_rtl(label)), ..fonts.body }, width, 100.0, &[]))
            };
            let left = if para.rtl { slot_left + 6.0 } else { slot_right - 6.0 - width };
            scene.texts.push(Text { x: left, y: top, layout: l, ink: Ink::Muted, alpha: 1.0 });
        }
        Block::Task { done } => {
            let size = 15.0;
            let b = Rect::new(center - size / 2.0, mid - size / 2.0, center + size / 2.0, mid + size / 2.0);
            if done {
                scene.fills.push(Fill::solid(b, 4.5, Ink::Accent));
                let check = layout(eng, "\u{E73E}", &fonts.icon_small, b.width(), b.height());
                scene.texts.push(Text { x: b.left, y: b.top + 0.5, layout: check, ink: Ink::Page, alpha: 1.0 });
            } else {
                scene.fills.push(Fill::solid(b, 4.5, Ink::Faint));
                scene.fills.push(Fill::solid(b.inset(1.5), 3.0, Ink::Page));
            }
            return Ok(Some(b.inset(-5.0)));
        }
        Block::Quote => {
            for d in 0..line.depth {
                let offset = d as f32 * QUOTE_INDENT;
                let left = if para.rtl { column.right - offset - 5.0 } else { column.left + offset + 2.0 };
                scene.fills.push(Fill::solid(Rect::new(left, top, left + 3.0, top + h), 1.5, Ink::Border));
            }
        }
        Block::Rule if !nt.caret_on(line) => {
            let rule = Rect::new(column.left, top + h / 2.0 - 0.5, column.right, top + h / 2.0 + 0.5);
            scene.fills.push(Fill::solid(rule, 0.0, Ink::Border));
        }
        _ => {}
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_paragraph_direction() {
        assert!(is_rtl("ترند الأخبار 781K"));
        assert!(is_rtl("  123 — שלום"));
        assert!(!is_rtl("Hello مرحبا"), "a tie: the first letter");
        assert!(!is_rtl("42 !"));
        assert!(is_rtl("Northwind كتبوا فكرة جديدة"), "mostly Arabic, a Latin name first");
        assert!(!is_rtl("نصيحة: always back up your notes before you sync"), "mostly English, an Arabic word first");
    }

    /// A mostly-Arabic line that starts with a Latin name reads from the
    /// right: the name at the right edge, the rest after it leftwards; the
    /// Latin words, "(Atlas 5.5)" and "3D" each whole in their place; and
    /// wrapped, the lines follow the text's order, each read from the right.
    #[test]
    fn a_latin_name_ahead_of_arabic_still_reads_from_the_right() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let text = "Northwind كتبوا فكرة جديدة في دفتر اسمه (Atlas 5.5) بعنوان \"Lantern\" تجمع لك خريطة 3D لكل مدينة تزورها وتحفظ لك فيها ملاحظاتك وصورك بلمسة";
        assert_eq!(direction(text), Some(true));
        let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("{s}"));
        let fonts = Fonts::new(1.0, 0);
        let format = Format { rtl: Some(is_rtl(text)), ..fonts.title.clone() };
        // The left edge of an LTR run's first letter (where its caret sits).
        let left_of = |l: &kolada_text::Layout, s: &str| l.caret(at(s)).x;
        // One line: Northwind rightmost, Lantern left of the Arabic before it.
        let one = layout(&eng, text, &format, 4000.0, 100.0);
        assert_eq!(one.lines().len(), 1);
        let x = |s: &str| one.caret(at(s)).x;
        assert!(x("Northwind") > x("كتبوا") && x("كتبوا") > x("(Atlas") && x("(Atlas") > x("بعنوان") && x("بعنوان") > x("Lantern"), "read from the right in the text's order");
        assert!(x("Lantern") > x("تجمع") && x("تجمع") > x("3D") && x("3D") > x("لكل"), "and on to its end");
        assert!(left_of(&one, "3D") < one.caret(at("3D") + 1).x, "3D whole, the 3 before the D");
        assert!(one.caret(at("Atlas")).x < one.caret(at("5.5")).x, "(Atlas 5.5) whole, left to right");
        // Wrapped at a few widths: the lines take the text in order (the
        // first starts with Northwind at its right edge), each from the right.
        for width in [700.0, 520.0, 360.0, 240.0] {
            let l = layout(&eng, text, &format, width, 100.0);
            let lines = l.lines();
            assert!(lines.len() >= 2, "{width}: wraps");
            assert_eq!(lines[0].start, 0, "{width}: the first line starts the text");
            let first = l.caret(at("Northwind"));
            let name = layout(&eng, "Northwind", &Format { rtl: None, ..format.clone() }, 4000.0, 100.0).measure().width;
            assert!(first.y == lines[0].top && first.x + name >= width - 2.0, "{width}: Northwind at the first line's right edge ({} + {name})", first.x);
            for w in lines.windows(2) {
                assert_eq!(w[0].end, w[1].start, "{width}: in the text's order");
            }
            let y = |s: &str| l.caret(at(s)).y;
            assert!(y("Northwind") <= y("Lantern") && y("Lantern") <= y("3D") && y("3D") <= y("بلمسة"), "{width}: lines in order");
            assert_eq!(y("(Atlas"), y("Atlas"), "{width}: no break after the opening bracket");
            let three = l.caret(at("3D"));
            let d = l.caret(at("3D") + 1);
            assert!(three.y == d.y && three.x < d.x, "{width}: 3D whole on one line");
        }
    }

    #[test]
    fn an_open_tasks_dates_are_tinted_and_faintly_red_once_due() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let text = "- [ ] Pay @today\n- [ ] Later @Sat 25 Dec 2027\n- [ ] Plants @every monday\n- [x] Done @today\nSay @today\n";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        let today = kolada_core::when::Day { year: 2026, month: 10, day: 5 };
        let tints = |today| {
            let input = Input { text, lines: &lines, page_fade: 1.0, today, ..Default::default() };
            let scene = build(&eng, &fonts, &input, 1000.0, 700.0).unwrap();
            let count = |ink| scene.fills.iter().filter(|f| f.ink == ink && f.radius == 5.0).count();
            (count(Ink::Mark), count(Ink::Field))
        };
        assert_eq!(tints(Some(today)), (1, 2), "due today in red; the far date and the repeat quiet; none for done tasks or plain text");
        assert_eq!(tints(None), (0, 0));
    }

    #[test]
    fn a_folded_section_takes_no_room_and_its_heading_shows_its_mark_and_count() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let text = "# Trip\nIntro\n## Packing\nsunscreen\nadapter\nshoes\n## Days\nAlfama\n";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        let packing = text.find("## Packing").unwrap();
        let from = packing + "## Packing".len();
        let folded = [(from, text.find("shoes").unwrap() + "shoes".len())];
        let at = |scene: &Scene, s: &str| scene.lines.iter().find(|l| l.para.start == text.find(s).unwrap()).map(|l| l.y);
        let intro = text.find("Intro").unwrap();
        let open = build(&eng, &fonts, &Input { text, lines: &lines, page_fade: 1.0, selection: (intro, intro), ..Default::default() }, 1000.0, 700.0).unwrap();
        let shut = build(&eng, &fonts, &Input { text, lines: &lines, page_fade: 1.0, selection: (intro, intro), sections_folded: &folded, ..Default::default() }, 1000.0, 700.0).unwrap();
        assert!(at(&shut, "sunscreen").is_none() && at(&shut, "shoes").is_none(), "its lines are not drawn");
        assert!(at(&shut, "## Days").unwrap() < at(&open, "## Days").unwrap(), "what follows moves up");
        let marks = |scene: &Scene, logo| scene.vectors.iter().filter(|v| v.logo == logo).count();
        assert_eq!((marks(&shut, crate::vector::Logo::FoldShut), marks(&open, crate::vector::Logo::FoldOpen)), (1, 0), "at rest only the folded heading shows its mark");
        let count = shut.texts.iter().filter(|t| t.ink == Ink::Faint && t.layout.text() == "3 lines").count();
        assert_eq!(count, 1, "and the count of its lines");
        assert!(shut.hits.iter().any(|(_, t)| *t == Target::Fold(packing)), "a click opens it");
        // Revealed on intent: the pointer at its margin; never at rest
        // with the caret on it (a note opens with its caret on the title).
        let hover = build(&eng, &fonts, &Input { text, lines: &lines, page_fade: 1.0, selection: (intro, intro), hover: Some(Target::Fold(packing)), ..Default::default() }, 1000.0, 700.0).unwrap();
        assert_eq!(marks(&hover, crate::vector::Logo::FoldOpen), 1);
        let caret = build(&eng, &fonts, &Input { text, lines: &lines, page_fade: 1.0, selection: (packing + 4, packing + 4), ..Default::default() }, 1000.0, 700.0).unwrap();
        assert_eq!(marks(&caret, crate::vector::Logo::FoldOpen), 0, "the caret on a heading shows no mark");
        let title = build(&eng, &fonts, &Input { text, lines: &lines, page_fade: 1.0, selection: (2, 2), ..Default::default() }, 1000.0, 700.0).unwrap();
        assert_eq!(marks(&title, crate::vector::Logo::FoldOpen), 0, "nor beside the title a note opens on");
        let bare = "# Alone\n";
        let bare_lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(bare).collect();
        let alone = build(&eng, &fonts, &Input { text: bare, lines: &bare_lines, page_fade: 1.0, hover: Some(Target::Fold(0)), ..Default::default() }, 1000.0, 700.0).unwrap();
        assert!(!alone.hits.iter().any(|(_, t)| *t == Target::Fold(0)), "a heading with nothing under it has no mark");
    }

    /// A picture in a list item, a task, a numbered item, a quote or a
    /// nested list is drawn there, indented with the item and no wider than
    /// it allows; its mark beside it, by its top; its embed never shows as
    /// words.
    #[test]
    fn a_picture_in_a_list_or_a_quote_is_drawn_with_its_item() {
        struct Source(std::sync::Arc<crate::pixels::Pixels>);
        impl crate::images::PictureSource for Source {
            fn place(&mut self, _: &std::path::Path, _: &std::path::Path, _: &str) -> crate::images::Lookup {
                crate::images::Lookup::Found("coast.png".into(), 2000, 1000)
            }
            fn pixels(&mut self, _: &std::path::Path, _: u32) -> Option<(std::sync::Arc<crate::pixels::Pixels>, f32)> {
                Some((self.0.clone(), 1.0))
            }
        }
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let source = std::cell::RefCell::new(Source(std::sync::Arc::new(crate::pixels::Pixels { width: 2, height: 1, data: vec![0xFF00_0000; 2] })));
        for (line, indent) in [("- ![[coast.png]]", LIST_INDENT), ("- [ ] ![[coast.png]]", LIST_INDENT), ("1. ![[coast.png]]", LIST_INDENT), ("> ![[coast.png]]", QUOTE_INDENT), ("- Outer\n    - ![[coast.png]]", 0.0), ("- Bags\n  ![[coast.png]]", LIST_INDENT)] {
            let text = format!("# Trip\n{line}\nAfter.");
            let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(&text).collect();
            // Nested: where its item's words are, by its depth.
            let indent = if indent == 0.0 { (lines[lines.len() - 2].depth as f32 + 1.0) * LIST_INDENT } else { indent };
            let s = build(&eng, &fonts, &Input { text: &text, lines: &lines, page_fade: 1.0, images: Some(&source), vault_path: "vault", ..Default::default() }, 1000.0, 900.0).unwrap();
            assert_eq!(s.pictures.len(), 1, "{line}: the picture");
            let r = s.pictures[0].rect;
            assert!((r.left - (s.column.left + indent)).abs() < 1.0, "{line}: indented with its item ({} from {})", r.left, s.column.left);
            assert!(r.right <= s.column.right + 0.5, "{line}: no wider than the item allows");
            assert!(!s.texts.iter().any(|t| t.layout.text().contains("coast.png") && t.layout.measure().width > 1.0), "{line}: its embed not as words");
        }
    }

    /// A picture the vault does not have (a note synced without it) is a
    /// quiet tile with the file's name in muted ink, never a line of
    /// link-coloured text; one it has is a picture.
    #[test]
    fn a_missing_picture_is_a_quiet_tile_with_its_name() {
        struct Source;
        impl crate::images::PictureSource for Source {
            fn place(&mut self, _: &std::path::Path, _: &std::path::Path, name: &str) -> crate::images::Lookup {
                if name.starts_with("here") { crate::images::Lookup::Found("here.png".into(), 400, 200) } else { crate::images::Lookup::Missing }
            }
            fn pixels(&mut self, _: &std::path::Path, _: u32) -> Option<(std::sync::Arc<crate::pixels::Pixels>, f32)> {
                None
            }
        }
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let source = std::cell::RefCell::new(Source);
        for text in ["# Trip\n![[Gone with the sync.png]]\nAfter.", "# Trip\n![](shots/Gone%20with%20the%20sync.png)\nAfter."] {
            let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
            let s = build(&eng, &fonts, &Input { text, lines: &lines, page_fade: 1.0, images: Some(&source), vault_path: "vault", ..Default::default() }, 1000.0, 700.0).unwrap();
            // (The line's own text is there too, hidden: no width.)
            let named: Vec<&Text> = s.texts.iter().filter(|t| t.layout.text().contains("Gone with the sync") && t.layout.measure().width > 1.0).collect();
            assert_eq!(named.len(), 1, "{text}: the name once");
            assert_eq!((named[0].layout.text(), named[0].ink), ("Gone with the sync.png", Ink::Muted), "{text}: its file's name, muted");
            assert!(!s.texts.iter().any(|t| t.ink == Ink::Link), "{text}: nothing link-coloured");
            let tile = s.fills.iter().find(|f| f.ink == Ink::Field && f.rect.height() == note::MISSING_HEIGHT).expect("the tile");
            assert!(tile.rect.contains(named[0].x + 1.0, named[0].y + 1.0) && tile.rect.width() <= note::MISSING_WIDTH, "{text}: on its tile");
            assert!(s.texts.iter().any(|t| t.layout.text() == "\u{E91B}" && t.ink == Ink::Faint && tile.rect.contains(t.x + 1.0, t.y + 1.0)), "{text}: the picture glyph on it");
        }
        let here = "# Trip\n![[here.png]]\nAfter.";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(here).collect();
        let s = build(&eng, &fonts, &Input { text: here, lines: &lines, page_fade: 1.0, images: Some(&source), vault_path: "vault", ..Default::default() }, 1000.0, 700.0).unwrap();
        assert!(!s.texts.iter().any(|t| t.layout.text().contains("here.png") && t.layout.measure().width > 1.0), "a picture it has shows no name");
        assert!(s.fills.iter().any(|f| f.ink == Ink::Field && (f.rect.height() - 200.0).abs() < 0.5), "its box while it decodes");
    }

    #[test]
    fn other_carets_are_drawn_with_the_main_one_and_their_selections_behind() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let text = "tram up\nby tram\n";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        let carets = [(0, 4), (11, 11)];
        let input = Input { text, lines: &lines, page_fade: 1.0, selection: (15, 15), carets: &carets, ..Default::default() };
        let mut scene = build(&eng, &fonts, &input, 1000.0, 700.0).unwrap();
        assert_eq!(scene.carets.len(), 2);
        assert!(scene.fills.iter().any(|f| f.ink == Ink::Highlight), "the first one's word is selected");
        let accents = |s: &Scene| s.fills.iter().filter(|f| f.ink == Ink::Accent && f.rect.width() <= CARET_WIDTH + 0.01).count();
        let before = accents(&scene);
        scene.place_caret((0.0, 0.0));
        assert_eq!(accents(&scene), before + 3, "all three blink as one");
    }

    #[test]
    fn seven_text_sizes_step_evenly_from_small_to_large() {
        use kolada_core::session::TEXT_SIZES;
        // Smaller to larger from the default, one step at a time, and no
        // further at either end.
        let mut down = vec![text_size(0)];
        let mut at = 0;
        while let Some(next) = step_text_size(at, false) {
            down.push(text_size(next));
            at = next;
        }
        assert_eq!(down, [16, 15, 14, 13]);
        let mut up = vec![];
        let mut at = 13;
        while let Some(next) = step_text_size(at, true) {
            up.push(text_size(next));
            at = next;
        }
        assert_eq!(up, [14, 15, 16, 18, 20, 23]);
        assert_eq!(step_text_size(15, true), Some(0), "the default is kept as 0");
        assert_eq!((step_text_size(13, false), step_text_size(23, true)), (None, None));
        // Each step a little more than the one before, never a jump.
        for w in TEXT_SIZES.windows(3) {
            let (a, b) = (w[1] as f32 / w[0] as f32, w[2] as f32 / w[1] as f32);
            assert!(a > 1.0 && b > 1.0 && b - a < 0.08, "{w:?}");
        }
        assert!(text_scale(23) > 1.125, "the largest is larger than the old Large");
        assert_eq!(text_scale(0), 1.0);
        // Sizes no step has read as the nearest.
        assert_eq!((text_size(1), text_size(17), text_size(200)), (13, 16, 23));
        // Headings grow with the text.
        let (small, large) = (Fonts::new(text_scale(13), 0), Fonts::new(text_scale(23), 0));
        assert!(large.body.size / small.body.size > 1.7 && (large.headings[0].size / large.body.size - small.headings[0].size / small.body.size).abs() < 0.01);
    }

    #[test]
    fn topmost_hit_wins() {
        let scene = Scene {
            hits: vec![
                (Rect::new(0.0, 0.0, 100.0, 40.0), Target::Caption),
                (Rect::new(80.0, 0.0, 100.0, 30.0), Target::Close),
            ],
            ..Scene::default()
        };
        assert_eq!(hit(&scene.hits, 90.0, 10.0), Some(Target::Close));
        assert_eq!(hit(&scene.hits, 10.0, 10.0), Some(Target::Caption));
        assert_eq!(hit(&scene.hits, 10.0, 50.0), None);
    }
}
