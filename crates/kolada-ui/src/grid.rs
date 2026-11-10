//! The notes as a grid of boxes under the note: the side menu's view
//! choice (`Session::view`). The window is split, the note on top and the
//! boxes below, and where the split is says what is being done:
//!
//! - at rest: a fresh page in the top 30%, the caret waiting; the boxes
//!   under it. Typing starts a note.
//! - writing: the note takes the window; the boxes wait below it.
//! - peek: the pointer at the window's foot while writing: the boxes rise
//!   to 30% of it, and drop back when the pointer leaves them.
//! - browse: the peeked boxes clicked: the note goes back to the top 30%,
//!   the boxes take the rest, and search and the menu's icons show above
//!   the note. A box opens its note; the note above them goes back to it.
//!
//! The split moves on a spring and, settled, holds still (no frames).
//! Every box is the same size: its note's title on one line, then a line of
//! its words, or its first picture, table or list (see `preview`). What
//! goes in a box is decided in `preview`; this places and draws them.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use kolada_core::vault::Listed;

use kolada_text::Format;

use crate::chrome::{self, Painter};
use crate::geom::{Fill, Ink, Rect};
use crate::preview::{Item, Mark, Preview, Rich};
use crate::scene::{self, Eng, Fonts, Input, Scene, Target, Text};
use windows_core::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Rest,
    Writing,
    Peek,
    Browse,
}

impl Phase {
    /// How much of the window's height the note keeps.
    pub fn note_share(self) -> f32 {
        match self {
            Phase::Writing => 1.0,
            Phase::Peek => 0.7,
            Phase::Rest | Phase::Browse => 0.3,
        }
    }
}

/// How near the window's foot the pointer brings the boxes up, DIPs; and
/// how far above them it may stray before they drop back.
pub const PEEK_EDGE: f32 = 24.0;

/// The split's spring, critically damped: settled in about a quarter of a
/// second, never past where it goes.
const OMEGA: f32 = 18.0;

/// The grid's state: what is being done, and where the split is on its way
/// there.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub phase: Phase,
    /// The note's share of the window's height now.
    pub split: f32,
    velocity: f32,
    /// How far the boxes are scrolled, DIPs.
    pub scroll: f32,
}

impl Grid {
    /// A fresh page opens at rest; a note, writing.
    pub fn new(fresh: bool) -> Grid {
        Grid::at(if fresh { Phase::Rest } else { Phase::Writing })
    }

    /// Settled in `phase`.
    pub fn at(phase: Phase) -> Grid {
        Grid { phase, split: phase.note_share(), velocity: 0.0, scroll: 0.0 }
    }

    /// Goes to `phase`: whether that changed anything.
    fn go(&mut self, phase: Phase) -> bool {
        let changed = self.phase != phase;
        self.phase = phase;
        changed
    }

    /// Something typed into the note: it takes the window.
    pub fn typed(&mut self) -> bool {
        self.go(Phase::Writing)
    }

    /// The pointer at `y` in a window `height` DIPs tall, or gone from it
    /// (None): at the foot while writing, the boxes peek; away from them,
    /// they drop back.
    pub fn pointer(&mut self, y: Option<f32>, height: f32) -> bool {
        match (self.phase, y) {
            (Phase::Writing, Some(y)) if y >= height - PEEK_EDGE => self.go(Phase::Peek),
            (Phase::Peek, Some(y)) if y < height * Phase::Peek.note_share() - PEEK_EDGE => self.go(Phase::Writing),
            (Phase::Peek, None) => self.go(Phase::Writing),
            _ => false,
        }
    }

    /// The boxes clicked while they peek: they take most of the window.
    pub fn clicked(&mut self) -> bool {
        self.phase == Phase::Peek && self.go(Phase::Browse)
    }

    /// The boxes browsed with the keys (a search in the bar, or the side
    /// menu's key while the grid is the view): they take most of the window.
    pub fn browse(&mut self) -> bool {
        self.go(Phase::Browse)
    }

    /// The pill at the window's foot clicked: the boxes come up. Riding
    /// the panel's edge while they are up (`chrome::grid_pill_rides`), it
    /// takes them back down.
    pub fn pill_clicked(&mut self) -> bool {
        match self.phase {
            Phase::Writing | Phase::Peek => self.go(Phase::Browse),
            Phase::Rest | Phase::Browse => self.go(Phase::Writing),
        }
    }

    /// A note opened (a box's), or the note above the boxes clicked.
    pub fn back_to_note(&mut self) -> bool {
        self.go(Phase::Writing)
    }

    /// A fresh page to write on.
    pub fn fresh(&mut self) -> bool {
        self.scroll = 0.0;
        self.go(Phase::Rest)
    }

    /// The split moved `dt` seconds along: true while it still moves.
    pub fn step(&mut self, dt: f32) -> bool {
        let target = self.phase.note_share();
        let (x, v, w) = (self.split - target, self.velocity, OMEGA);
        let e = (-w * dt).exp();
        let x1 = (x + (v + w * x) * dt) * e;
        let v1 = (v - w * (v + w * x) * dt) * e;
        if x1.abs() < 0.0005 && v1.abs() < 0.005 {
            self.split = target;
            self.velocity = 0.0;
            return false;
        }
        self.split = target + x1;
        self.velocity = v1;
        true
    }

    /// Whether the split has yet to settle.
    pub fn moving(&self) -> bool {
        self.split != self.phase.note_share() || self.velocity != 0.0
    }

    /// Where the boxes begin, in a window `height` DIPs tall.
    pub fn top(&self, height: f32) -> f32 {
        (self.split * height).round()
    }

    /// Box `i` of `count` brought into sight on the shelf: whether it moved.
    pub fn reveal(&mut self, i: usize, count: usize, width: f32, height: f32) -> bool {
        let s = sheet(self, width, height);
        let (shelf, r) = (shelf(s), place(s, i, self.scroll));
        let dy = if r.top < shelf.top + GAP / 2.0 {
            r.top - (shelf.top + GAP / 2.0)
        } else if r.bottom > shelf.bottom - GAP {
            r.bottom - (shelf.bottom - GAP)
        } else {
            0.0
        };
        dy != 0.0 && self.scroll_by(dy, count, width, height)
    }

    /// The boxes scrolled by `dy` DIPs (down is positive), `count` of them
    /// in a window `width` by `height`: no further than they go.
    pub fn scroll_by(&mut self, dy: f32, count: usize, width: f32, height: f32) -> bool {
        let sheet = sheet(self, width, height);
        let most = (content_height(count, width) - sheet.height()).max(0.0);
        let to = (self.scroll + dy).clamp(0.0, most);
        let changed = to != self.scroll;
        self.scroll = to;
        changed
    }
}

/// The room between boxes and around the grid, DIPs; a box's corners and
/// padding; its title's height, and a row's under it.
const GAP: f32 = 12.0;
const INSET: f32 = 24.0;
const BOX_RADIUS: f32 = 10.0;
const BOX_PAD: f32 = 14.0;
const TITLE_H: f32 = 22.0;
const ROW_H: f32 = 21.0;
/// A box keeps to this width: a column comes or goes past it.
const BOX_LEAST: f32 = 220.0;
const BOX_MOST: f32 = 300.0;
const LEAST_COLUMNS: usize = 2;
const MOST_COLUMNS: usize = 5;
/// The panel under the bar and the boxes: its room from the window's sides,
/// and its top corners (it runs on past the window's foot).
pub const PANEL_INSET: f32 = 8.0;
pub const PANEL_RADIUS: f32 = 16.0;
/// The bar along the grid's top edge (search and the menu's icons): how
/// far down it starts, its height; the boxes start under it.
const BAR_TOP: f32 = 14.0;
const BAR_H: f32 = 34.0;
const HEAD: f32 = BAR_TOP + BAR_H + 14.0;

/// How the boxes sit across a window: their left edge, how many a row,
/// and each one's width and height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Across {
    pub left: f32,
    pub cols: usize,
    pub width: f32,
    pub height: f32,
}

impl Across {
    /// The right edge of the last column.
    pub fn right(&self) -> f32 {
        self.left + self.cols as f32 * (self.width + GAP) - GAP
    }
}

/// The boxes across a window `width` DIPs wide: two to five columns, as
/// many as fit with each box `BOX_LEAST` to `BOX_MOST` wide (narrower only
/// where two are all there is room for), cards a little wider than tall;
/// centred, wider windows keeping their room at the sides.
pub fn across(width: f32) -> Across {
    let room = (width - 2.0 * INSET).max(1.0);
    let cols = (((room + GAP) / (BOX_LEAST + GAP)).floor().max(0.0) as usize).clamp(LEAST_COLUMNS, MOST_COLUMNS);
    let w = ((room - GAP * (cols - 1) as f32) / cols as f32).min(BOX_MOST).floor().max(1.0);
    let total = cols as f32 * (w + GAP) - GAP;
    Across { left: ((width - total) / 2.0).round(), cols, width: w, height: (w * 0.68).clamp(140.0, 200.0).round() }
}

/// How many boxes a row holds in a window `width` DIPs wide.
pub fn columns(width: f32) -> usize {
    across(width).cols
}

/// The panel the bar and the boxes are on: from the split to the window's
/// foot.
pub fn sheet(grid: &Grid, width: f32, height: f32) -> Rect {
    Rect::new(0.0, grid.top(height), width, height)
}

/// The panel the grid is drawn on: the sheet, in from the window's sides.
pub fn panel(sheet: Rect) -> Rect {
    Rect::new(sheet.left + PANEL_INSET, sheet.top, sheet.right - PANEL_INSET, sheet.bottom)
}

/// Where the boxes show on `sheet`: on its panel, under its bar.
pub fn shelf(sheet: Rect) -> Rect {
    let p = panel(sheet);
    Rect::new(p.left, (sheet.top + HEAD - GAP / 2.0).min(p.bottom), p.right, p.bottom)
}

/// How tall `count` boxes stand in a window `width` DIPs wide, the bar
/// above them and the room under them included.
pub fn content_height(count: usize, width: f32) -> f32 {
    let a = across(width);
    let rows = count.div_ceil(a.cols);
    HEAD + if rows == 0 { 0.0 } else { rows as f32 * (a.height + GAP) - GAP + INSET }
}

/// The `i`th box on `sheet`, the boxes scrolled `scroll`.
pub fn place(sheet: Rect, i: usize, scroll: f32) -> Rect {
    let a = across(sheet.width());
    let (row, col) = (i / a.cols, i % a.cols);
    let left = a.left + col as f32 * (a.width + GAP);
    let top = (sheet.top + HEAD + row as f32 * (a.height + GAP) - scroll).round();
    Rect::new(left, top, left + a.width, top + a.height)
}

/// The boxes that show (some of them, at least) on the grid as it is, of
/// `count`: only these are read and drawn.
pub fn shown(grid: &Grid, count: usize, width: f32, height: f32) -> Range<usize> {
    let room = sheet(grid, width, height).height() - HEAD;
    if room < 1.0 || count == 0 {
        return 0..0;
    }
    let a = across(width);
    let row = a.height + GAP;
    let first = (grid.scroll / row).floor().max(0.0) as usize;
    let last = ((grid.scroll + room) / row).floor().max(0.0) as usize;
    (first * a.cols).min(count)..((last + 1) * a.cols).min(count)
}

/// An arrow key over the boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Left,
    Right,
    Up,
    Down,
}

/// The box an arrow chooses from `chosen`, of `count` in `cols` columns:
/// the first when none is; along the row and over the rows, never past the
/// first or the last; in a grid read right to left, Left is the next box.
pub fn step(chosen: Option<usize>, step: Step, cols: usize, count: usize, rtl: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let Some(i) = chosen else { return Some(0) };
    let cols = cols.max(1);
    let (back, on) = if rtl { (Step::Right, Step::Left) } else { (Step::Left, Step::Right) };
    Some(match step {
        s if s == back => i.saturating_sub(1),
        s if s == on => (i + 1).min(count - 1),
        Step::Up => if i >= cols { i - cols } else { i },
        // Down from a row with none under it: the last box.
        _ => if i + cols < count { i + cols } else if i / cols < (count - 1) / cols { count - 1 } else { i },
    })
}

/// A box as the shell knows it: its note's name until it is read, then
/// what it shows, and its picture's file when it shows one; when it is a
/// search's match, the words around the match.
pub struct BoxView<'a> {
    pub name: &'a str,
    pub preview: Option<&'a Preview>,
    pub picture: Option<&'a Path>,
    pub snippet: Option<&'a str>,
}

/// A search's match as the grid shows it: its note (vault-relative, with
/// `.md`), its name, and the words around the match in its text.
pub type Match = (String, String, Option<String>);

/// The notes among a search's hits, as boxes show them (a PDF the search
/// found is the side menu's to open, not a box's).
pub fn matches(hits: &[kolada_core::search::Hit]) -> Vec<Match> {
    // The words around a match as search reads them (`search::readable`: math as text, no marks).
    hits.iter().filter(|h| kolada_core::search::is_note(&h.path)).map(|h| (h.path.clone(), h.name.clone(), h.snippet.clone())).collect()
}

/// A box read: its note, and what it shows with its picture's file (None:
/// the note could not be read).
pub type Read = (String, Option<(Preview, Option<PathBuf>)>);

/// The grid's notes, newest first, and the boxes read so far: what a shell
/// keeps while the grid is the view. The reading is the shell's, on a
/// worker (`read`); this says what to read, and keeps what came.
#[derive(Default)]
pub struct Notes {
    pub list: Vec<Listed>,
    read: HashMap<String, (Preview, Option<PathBuf>)>,
    asked: HashSet<String>,
    /// While a search is typed: its matches, best first, in place of the
    /// notes.
    found: Option<Vec<Match>>,
}

impl Notes {
    /// The vault's notes, listed anew: their boxes are read again.
    pub fn listed(&mut self, list: Vec<Listed>) {
        self.asked.clear();
        self.list = list;
    }

    /// A search's matches shown in place of the notes, or the notes again
    /// (None).
    pub fn found(&mut self, matches: Option<Vec<Match>>) {
        self.found = matches;
    }

    /// Whether the boxes are a search's matches.
    pub fn searching(&self) -> bool {
        self.found.is_some()
    }

    /// How many boxes there are: the matches while searching, else the notes.
    pub fn count(&self) -> usize {
        self.found.as_ref().map_or(self.list.len(), Vec::len)
    }

    /// Box `i`: its note, its name, the words around a match.
    fn at(&self, i: usize) -> Option<(&str, &str, Option<&str>)> {
        match &self.found {
            Some(m) => m.get(i).map(|(p, n, s)| (p.as_str(), n.as_str(), s.as_deref())),
            None => self.list.get(i).map(|n| (n.path.as_str(), n.name.as_str(), None)),
        }
    }

    /// The boxes in sight not asked for yet, now asked for.
    pub fn wanted(&mut self, grid: &Grid, width: f32, height: f32) -> Vec<String> {
        let shown = shown(grid, self.count(), width, height);
        let paths: Vec<String> = shown.filter_map(|i| self.at(i).map(|(p, _, _)| p.to_string())).collect();
        paths.into_iter().filter(|p| self.asked.insert(p.clone())).collect()
    }

    /// Boxes a worker read.
    pub fn read(&mut self, boxes: Vec<Read>) {
        for (rel, got) in boxes {
            match got {
                Some(b) => self.read.insert(rel, b),
                None => self.read.remove(&rel),
            };
        }
    }

    /// The note at `rel` was saved: the newest now, its box to read again.
    pub fn saved(&mut self, rel: &str) {
        if rel.is_empty() {
            return;
        }
        let name = rel.rsplit('/').next().unwrap_or(rel).trim_end_matches(".md").to_string();
        self.list.retain(|n| n.path != rel);
        self.list.insert(0, Listed { path: rel.to_string(), name, written: Some(std::time::SystemTime::now()) });
        self.read.remove(rel);
        self.asked.remove(rel);
    }

    /// The `i`th box's note.
    pub fn note(&self, i: usize) -> Option<&str> {
        self.at(i).map(|(p, _, _)| p)
    }

    /// The boxes in sight: the first's place, and the boxes.
    pub fn boxes(&self, grid: &Grid, width: f32, height: f32) -> (usize, Vec<BoxView<'_>>) {
        let shown = shown(grid, self.count(), width, height);
        let first = shown.start;
        let boxes = shown
            .filter_map(|i| self.at(i))
            .map(|(path, name, snippet)| {
                let read = self.read.get(path);
                BoxView { name, preview: read.map(|r| &r.0), picture: read.and_then(|r| r.1.as_deref()), snippet }
            })
            .collect();
        (first, boxes)
    }
}

/// What a worker does: the boxes of the notes `wanted` in `vault`.
pub fn read(vault: &Path, wanted: Vec<String>) -> Vec<Read> {
    let mut known = None;
    wanted
        .into_iter()
        .map(|rel| {
            let got = crate::preview::of_note(vault, &rel, &mut known);
            (rel, got)
        })
        .collect()
}

/// The grid as the scene draws it.
pub struct GridView<'a> {
    pub grid: &'a Grid,
    /// How many boxes there are; those `shown` from `first` on.
    pub count: usize,
    pub first: usize,
    pub boxes: &'a [BoxView<'a>],
    /// The search typed in the bar's field, while one is.
    pub search: Option<GridSearch<'a>>,
    /// The box the arrows chose (Enter opens it).
    pub chosen: Option<usize>,
}

/// The bar's search: what is typed, and the caret in it while the field
/// has the keys.
#[derive(Clone, Copy, Debug)]
pub struct GridSearch<'a> {
    pub query: &'a str,
    pub caret: Option<usize>,
}

/// A drawn icon's ink, DIPs: as tall as the icon font's glyphs beside it.
pub(crate) const MARK: f32 = 13.0;

/// The view's button: one icon, the view a press switches to (the grid's
/// four rounded tiles from the list, the list's rows with their dots from
/// the grid), drawn as fills in the ink of the icons beside it.
/// `alpha`: its opacity at rest (the side menu's icons are quieter until
/// reached for).
pub(crate) fn view_toggle(p: &mut Painter, r: Rect, grid_on: bool, ink: Ink, alpha: f32) {
    p.button(r, Target::ViewToggle, 8.0);
    let hot = p.hover == Some(Target::ViewToggle);
    let (ink, alpha) = if hot { (Ink::Text, 1.0) } else { (ink, alpha) };
    let (x, y) = (((r.left + r.right - MARK) / 2.0).round(), ((r.top + r.bottom - MARK) / 2.0).round());
    if !grid_on {
        let gap = 2.6;
        let tile = (MARK - gap) / 2.0;
        for (i, j) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            let (l, t) = (x + i * (tile + gap), y + j * (tile + gap));
            p.scene.fills.push(Fill::faded(Rect::new(l, t, l + tile, t + tile), 1.6, ink, alpha));
        }
    } else {
        let (dot, bar) = (3.0, 2.2);
        for j in 0..3 {
            let mid = y + dot / 2.0 + j as f32 * (MARK - dot) / 2.0;
            p.scene.fills.push(Fill::faded(Rect::new(x, mid - dot / 2.0, x + dot, mid + dot / 2.0), dot / 2.0, ink, alpha));
            p.scene.fills.push(Fill::faded(Rect::new(x + 4.6, mid - bar / 2.0, x + MARK, mid + bar / 2.0), bar / 2.0, ink, alpha));
        }
    }
}
const SEARCH_MOST: f32 = 320.0;
/// How far up the note above the panel fades into the page, DIPs.
const STRIP_FADE: f32 = 28.0;
/// The bar's room from the panel's sides where the window is narrow.
const BAR_PAD: f32 = 12.0;
const SEARCH_LEAST: f32 = 140.0;

/// Draws the grid over the note: on a panel of its own over the page, its
/// bar along its top edge, the boxes under it. After the chrome: its
/// targets are over the note's.
pub fn draw(eng: &Eng, fonts: &Fonts, input: &Input, view: &GridView, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    let grid = view.grid;
    let sheet = sheet(grid, width, height);
    let mut p = Painter { eng, fonts, hover: input.hover, scene };
    if grid.phase == Phase::Browse {
        // The note above the boxes, under the window's top row: back to it.
        let top = chrome::MARGIN + chrome::TOOLBAR_HEIGHT;
        if sheet.top > top {
            p.scene.hits.push((Rect::new(0.0, top, width, sheet.top), Target::GridNote));
        }
    }
    // The pill the boxes come up from: under them as they do (the sheet
    // covers it), or riding their panel's edge, over it.
    let rides = chrome::grid_pill_rides();
    if !rides || sheet.height() < 0.5 {
        chrome::grid_pill(&mut p, input, grid, width, height);
    }
    if sheet.height() < 0.5 {
        return Ok(());
    }
    // The page under it (the note does not show through), then the panel:
    // a step off the page, its top corners round (its foot runs on past the
    // window's), no line or shadow.
    p.scene.fills.push(Fill::solid(Rect::new(sheet.left - 1.0, sheet.top, sheet.right + 1.0, sheet.bottom + 1.0), 0.0, Ink::Page));
    // The note above it ends softly: its last line in sight dissolves into
    // the page, never cut through the middle by the panel's edge.
    p.scene.fills.push(Fill { blur: STRIP_FADE, ..Fill::solid(Rect::new(-STRIP_FADE, sheet.top - STRIP_FADE / 2.0, width + STRIP_FADE, sheet.bottom + STRIP_FADE), 0.0, Ink::Page) });
    let panel = panel(sheet);
    p.scene.fills.push(Fill::solid(Rect::new(panel.left, panel.top, panel.right, panel.bottom + PANEL_RADIUS), PANEL_RADIUS, Ink::Side));
    p.scene.hits.push((sheet, Target::GridSheet));
    if rides {
        chrome::grid_pill(&mut p, input, grid, width, height);
    }
    // The trash, open from the bar: in place of the boxes, as the side
    // menu has it in place of its tree.
    let shelf = shelf(sheet);
    if let Some(entries) = input.trash {
        // As wide as the bar: its Empty right under the bar's last icon.
        let a = across(width);
        let (left, right) = bar_edges(input, sheet, a);
        chrome::trash(&mut p, entries, left, right, shelf.top + GAP, shelf.bottom - GAP)?;
        return bar(&mut p, input, view, sheet, a);
    }
    // The boxes, clipped under the bar as they scroll; while a search is
    // typed, what it found tinted in them.
    p.scene.overlay(Some(shelf));
    let query = view.search.map(|s| s.query).filter(|q| !q.trim().is_empty());
    for (k, b) in view.boxes.iter().enumerate() {
        let i = view.first + k;
        let r = place(sheet, i, grid.scroll);
        if r.bottom <= shelf.top || r.top >= shelf.bottom {
            continue;
        }
        let hot = p.hover == Some(Target::GridBox(i));
        draw_box(&mut p, input, b, r, shelf, hot, view.chosen == Some(i), query)?;
        p.scene.hits.push((Rect::new(r.left, r.top.max(shelf.top), r.right, r.bottom.min(shelf.bottom)), Target::GridBox(i)));
    }
    // A search with nothing found: one quiet line where the boxes would be.
    if view.count == 0 && view.search.is_some_and(|s| !s.query.trim().is_empty()) {
        let a = across(width);
        let r = Rect::new(a.left, shelf.top + GAP, a.right(), shelf.top + GAP + 28.0);
        if r.bottom <= shelf.bottom {
            let quiet = Format { align: kolada_text::Align::Center, ..p.fonts.label.clone() };
            line(&mut p, "No notes match", &quiet, r, Ink::Faint, 1.0);
        }
    }
    // Whatever comes next is drawn whole again.
    p.scene.overlay(None);
    bar(&mut p, input, view, sheet, across(width))
}

/// Search, a new note and the menu's icons along the grid's top edge,
/// travelling with it as it peeks and comes up: the field at the boxes'
/// left edge, the icons at their right, as the side menu's foot has them
/// (`chrome::menu_icons`), then Beam's button where Beam runs.
fn bar(p: &mut Painter, input: &Input, view: &GridView, sheet: Rect, a: Across) -> Result<()> {
    let top = sheet.top + BAR_TOP;
    if top >= sheet.bottom {
        return Ok(());
    }
    let mut row: Vec<(Target, &str)> = vec![(Target::NewNote, chrome::icon::ADD)];
    row.extend(chrome::menu_icons(input));
    if input.quick.is_some() {
        row.push((Target::Quick(crate::quick::Hit::Fab), ""));
    }
    let width_of = |n: usize| n as f32 * chrome::ICON_PITCH - chrome::BTN_GAP;
    let (mut left, right) = bar_edges(input, sheet, a);
    // Narrow, every icon stays: the search is an icon too, the bar as wide
    // as the panel; the field, once it has the keys or a query, takes the
    // bar's whole width over them.
    if bar_is_narrow(input, a) {
        let typing = view.search.is_some_and(|s| s.caret.is_some() || !s.query.is_empty());
        if typing {
            return search_field(p, view.search, Rect::new(left, top, right, top + BAR_H));
        }
        let r = Rect::new(left, top + 1.0, left + chrome::ICON_TILE, top + BAR_H - 1.0);
        p.button(r, Target::Search, 8.0);
        p.icon(chrome::icon::SEARCH, r, if p.hover == Some(Target::Search) { Ink::Text } else { Ink::Muted })?;
        left += chrome::ICON_PITCH;
    } else {
        let x = right - width_of(row.len());
        search_field(p, view.search, Rect::new(left, top, left + (x - left - GAP).min(SEARCH_MOST), top + BAR_H))?;
    }
    let mut x = (right - width_of(row.len())).max(left);
    for (target, glyph) in row {
        if x + chrome::ICON_TILE > right + 0.5 {
            break;
        }
        let r = Rect::new(x, top + 1.0, x + chrome::ICON_TILE, top + BAR_H - 1.0);
        x += chrome::ICON_PITCH;
        match target {
            // The list, a press away.
            Target::ViewToggle => view_toggle(p, r, true, Ink::Muted, 1.0),
            Target::Quick(_) => beam_cell(p, input, r),
            _ => {
                if (target == Target::Options && input.options_open) || (target == Target::Trash && input.trash.is_some()) {
                    p.scene.fills.push(Fill::solid(r, 8.0, Ink::Selection));
                }
                p.button(r, target, 8.0);
                let ink = if p.hover == Some(target) { Ink::Text } else { Ink::Muted };
                // The filled square sits smaller in its em than the glyphs
                // beside it: a step up, to be as tall as they are.
                let size = if target == Target::NewNote { p.fonts.icon.size * 1.12 } else { p.fonts.icon.size };
                let format = Format { size, ..p.fonts.icon.clone() };
                p.text(glyph, &format, r, ink)?;
            }
        }
    }
    Ok(())
}

/// How many icons the bar has: a new note, the menu's four, and Beam's
/// where Beam runs.
fn bar_icons(input: &Input) -> usize {
    5 + input.quick.is_some() as usize
}

/// Whether the bar is too narrow for its field beside its icons.
fn bar_is_narrow(input: &Input, a: Across) -> bool {
    let icons = bar_icons(input) as f32 * chrome::ICON_PITCH - chrome::BTN_GAP;
    a.right() - a.left - icons - GAP < SEARCH_LEAST
}

/// The bar's left and right edges: the boxes', or narrow, the panel's.
fn bar_edges(input: &Input, sheet: Rect, a: Across) -> (f32, f32) {
    if bar_is_narrow(input, a) {
        let panel = panel(sheet);
        (panel.left + BAR_PAD, panel.right - BAR_PAD)
    } else {
        (a.left, a.right())
    }
}

/// The bar's search field: what is typed and its caret while it has the
/// keys, a way to clear it once there is something, else "Search".
fn search_field(p: &mut Painter, search: Option<GridSearch>, field: Rect) -> Result<()> {
    let focused = search.is_some_and(|s| s.caret.is_some());
    let ink = if focused { Ink::Field } else if p.hover == Some(Target::Search) { Ink::Hover } else { Ink::Field };
    p.scene.fills.push(Fill::solid(field, BOX_RADIUS, ink));
    p.scene.hits.push((field, Target::Search));
    p.icon(chrome::icon::SEARCH, Rect::new(field.left + 4.0, field.top, field.left + 32.0, field.bottom), Ink::Faint)?;
    let query = search.map_or("", |s| s.query);
    let typed = !query.is_empty();
    let room = Rect::new(field.left + 34.0, field.top, field.right - if typed { 34.0 } else { 10.0 }, field.bottom);
    let format = Format { wrap: false, ellipsis: true, center_y: true, ..p.fonts.label };
    // Typed past its end, left to right: the query slides left under the
    // field's edge so the caret stays in sight (cut with "…" it would hide
    // what is being typed).
    let eng = p.eng;
    let whole = Format { ellipsis: false, rtl: Some(false), ..format };
    let long = typed && !scene::is_rtl(query) && {
        let l = scene::kept_layout(eng, query, &whole, 4000.0, room.height(), 0, || scene::layout_styled(eng, query, &whole, 4000.0, room.height(), &[]));
        l.measure().width > room.width()
    };
    if long {
        let l = scene::kept_layout(eng, query, &whole, 4000.0, room.height(), 0, || scene::layout_styled(eng, query, &whole, 4000.0, room.height(), &[]));
        let at = search.and_then(|s| s.caret).unwrap_or(query.len()).min(query.len());
        let shift = (l.caret(at).x - room.width() + 2.0).max(0.0);
        p.scene.overlay(Some(room));
        p.scene.texts.push(scene::Text { x: room.left - shift, y: room.top, layout: l.clone(), ink: Ink::Text, alpha: 1.0 });
        if search.is_some_and(|s| s.caret.is_some()) {
            let x = (room.left - shift + l.caret(at).x).round();
            p.scene.fills.push(Fill::solid(Rect::new(x, field.top + 8.0, x + 2.0, field.bottom - 8.0), 1.0, Ink::Accent));
        }
        p.scene.overlay(None);
    } else if typed {
        line(p, query, &format, room, Ink::Text, 1.0);
    } else {
        line(p, "Search", &format, room, Ink::Faint, 1.0);
    }
    if typed {
        let clear = Rect::new(field.right - 32.0, field.top + 3.0, field.right - 4.0, field.bottom - 3.0);
        p.button(clear, Target::SearchClear, 8.0);
        let hot = p.hover == Some(Target::SearchClear);
        p.text(chrome::icon::CLOSE, &p.fonts.icon_small.clone(), clear, if hot { Ink::Text } else { Ink::Faint })?;
    }
    // The caret, where the keys go.
    if let Some(at) = search.and_then(|s| s.caret).filter(|_| !long) {
        let l = scene::kept_layout(eng, query, &format, room.width(), room.height(), 0, || {
            scene::layout_styled(eng, query, &Format { rtl: Some(scene::is_rtl(query)), ..format }, room.width(), room.height(), &[])
        });
        let x = if typed { room.left + l.caret(at.min(query.len())).x } else { room.left };
        p.scene.fills.push(Fill::solid(Rect::new(x.round(), field.top + 8.0, x.round() + 2.0, field.bottom - 8.0), 1.0, Ink::Accent));
    }
    Ok(())
}

/// Beam's button on the bar, in place of the round one over the window's
/// corner (which would cover the boxes): its mark, teal with what is new.
fn beam_cell(p: &mut Painter, input: &Input, r: Rect) {
    let target = Target::Quick(crate::quick::Hit::Fab);
    p.button(r, target, 8.0);
    let new = input.quick.is_some_and(|b| b.unseen > 0);
    let ink = if new { Ink::Accent } else if p.hover == Some(target) { Ink::Text } else { Ink::Muted };
    // Its ink is 17/24 of its square: as tall as the icons beside it.
    let g = (MARK * 24.0 / 17.0).round();
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    crate::quick::beam_mark(p, Rect::new((cx - g / 2.0).round(), (cy - g / 2.0).round(), (cx + g / 2.0).round(), (cy + g / 2.0).round()), ink, 1.0);
    if new {
        let d = Rect::new(r.right - 11.0, r.top + 4.0, r.right - 4.0, r.top + 11.0);
        p.scene.fills.push(Fill::solid(d, 3.5, Ink::Accent));
    }
}

/// The letters a card's one line of a match keeps before it, at most.
const CARD_LEAD: usize = 12;
/// That line's height, under the title.
const SNIPPET_H: f32 = 19.0;

/// One box: its title, then its line, picture, table or list.
/// The box the arrows chose is a grey step up from the rest, lifted (the
/// selection's grey is the panel's own in paper). A search's match shows
/// the words around what it found, tinted: in place of a note's lines; in
/// the box's own table or list when it shows the match there; else on one
/// line over its picture, table or list.
#[allow(clippy::too_many_arguments)]
fn draw_box(p: &mut Painter, input: &Input, b: &BoxView, r: Rect, sheet: Rect, hot: bool, chosen: bool, query: Option<&str>) -> Result<()> {
    if hot || chosen {
        p.scene.fills.push(Fill::shadow(r.offset(0.0, 4.0), BOX_RADIUS, 14.0, 0.9));
    }
    p.scene.fills.push(Fill::solid(r, BOX_RADIUS, if chosen { Ink::Hover } else { Ink::Raised }));
    let inner = Rect::new(r.left + BOX_PAD, r.top + BOX_PAD - 2.0, r.right - BOX_PAD, r.bottom - BOX_PAD);
    let title = b.preview.map_or(b.name, |pv| pv.title.as_str());
    let title_format = Format { size: 15.0, weight: 600, ..p.fonts.label };
    line_found(p, title, &title_format, Rect::new(inner.left, inner.top, inner.right, inner.top + TITLE_H), Ink::Text, query);
    let mut below = Rect::new(inner.left, inner.top + TITLE_H + 6.0, inner.right, inner.bottom);
    let body = Format { size: 13.5, line: Some(19.0), ..p.fonts.label };
    // A search's match: the words around it; over a
    // picture, or a table or list that does not show it, on one line,
    // which stay under it.
    if let Some(words_at) = b.snippet {
        if b.preview.is_none_or(|pv| pv.rich.is_none()) {
            words(p, &[words_at.to_string()], &body, below, Ink::Muted, query);
            return Ok(());
        }
        let rich = b.preview.and_then(|pv| pv.rich.as_ref());
        if !query.zip(rich).is_some_and(|(q, rich)| shows(p, rich, below, q)) {
            let led = crate::found::lead(words_at, query.unwrap_or(""), CARD_LEAD);
            snippet_found(p, &led, &body, Rect::new(below.left, below.top, below.right, below.top + SNIPPET_H), Ink::Muted, query);
            below.top += SNIPPET_H + 4.0;
        }
    }
    let Some(pv) = b.preview else { return Ok(()) };
    match &pv.rich {
        None => words(p, &pv.lines, &body, below, Ink::Muted, query),
        // Edge to edge under the title, its foot in the box's corners.
        Some(Rich::Picture(name)) => picture(p, input, b.picture, name, Rect::new(r.left, below.top + 4.0, r.right, r.bottom), sheet),
        Some(Rich::Table(rows)) => table(p, rows, below, query),
        Some(Rich::List(items)) => list(p, items, below, query),
    }
    Ok(())
}

/// Whether a box's table or list, set in `r`, shows what a search for
/// `query` found: in a cell or an item that fits, and in what of it shows
/// (not cut away by its ellipsis).
fn shows(p: &Painter, rich: &Rich, r: Rect, query: &str) -> bool {
    let small = Format { size: 13.0, ..p.fonts.label };
    let has = |text: &str, format: &Format, at: Rect| !crate::found::ranges(one_line(p, text, format, at).cut_text(), query).is_empty();
    match rich {
        Rich::Picture(_) => false,
        Rich::Table(rows) => {
            let head = Format { weight: 600, ..small };
            table_cells(rows, r).into_iter().any(|(cell, header, at)| has(cell, if header { &head } else { &small }, at))
        }
        Rich::List(items) => list_items(items, r).into_iter().any(|it| has(&it.item.text, &small, it.words)),
    }
}

/// A table's first rows as a box sets them in `r`: each cell's text,
/// whether it is the header's, and its place. Each column an even share.
fn table_cells(rows: &[Vec<String>], r: Rect) -> Vec<(&str, bool, Rect)> {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let w = r.width() / cols as f32;
    let mut out = Vec::new();
    for (k, row) in rows.iter().enumerate().take_while(|(k, _)| r.top + (*k as f32 + 1.0) * ROW_H <= r.bottom + 0.5) {
        let top = r.top + k as f32 * ROW_H;
        for (c, cell) in row.iter().enumerate() {
            let x = r.left + c as f32 * w;
            out.push((cell.as_str(), k == 0, Rect::new(x + 2.0, top, x + w - 6.0, top + ROW_H)));
        }
    }
    out
}

/// A table's first rows: the header in the text's ink on a faint band,
/// each column an even share; what a search found tinted in its cells.
fn table(p: &mut Painter, rows: &[Vec<String>], r: Rect, query: Option<&str>) {
    let small = Format { size: 13.0, ..p.fonts.label };
    let head = Format { weight: 600, ..small };
    if !rows.is_empty() && r.top + ROW_H <= r.bottom + 0.5 {
        p.scene.fills.push(Fill::faded(Rect::new(r.left - 6.0, r.top, r.right + 6.0, r.top + ROW_H), 6.0, Ink::Well, 1.0));
    }
    for (cell, header, at) in table_cells(rows, r) {
        let (format, ink) = if header { (&head, Ink::Text) } else { (&small, Ink::Muted) };
        line_found(p, cell, format, at, ink, query);
    }
}

/// A list's item as a box sets it: its place, its direction, and where
/// its mark and its words go.
struct Placed<'a> {
    item: &'a Item,
    top: f32,
    rtl: bool,
    mark: Rect,
    words: Rect,
}

/// A list's first items, as many as fit in `r`. A right to left item has
/// its mark on the right, as in the note.
fn list_items(items: &[Item], r: Rect) -> Vec<Placed<'_>> {
    items
        .iter()
        .enumerate()
        .take_while(|(k, _)| r.top + (*k as f32 + 1.0) * ROW_H <= r.bottom + 0.5)
        .map(|(k, item)| {
            let top = r.top + k as f32 * ROW_H;
            let rtl = scene::is_rtl(&item.text);
            let indent = item.depth.min(3) as f32 * 14.0;
            let mark = if rtl { Rect::new(r.right - indent - 18.0, top, r.right - indent, top + ROW_H) } else { Rect::new(r.left + indent, top, r.left + indent + 18.0, top + ROW_H) };
            let words = if rtl { Rect::new(r.left, top, mark.left - 2.0, top + ROW_H) } else { Rect::new(mark.right + 2.0, top, r.right, top + ROW_H) };
            Placed { item, top, rtl, mark, words }
        })
        .collect()
}

/// A list's first items, each with its mark; what a search found tinted
/// in their words.
fn list(p: &mut Painter, items: &[Item], r: Rect, query: Option<&str>) {
    let small = Format { size: 13.0, ..p.fonts.label };
    for Placed { item, top, rtl, mark, words } in list_items(items, r) {
        let ink = match &item.mark {
            Mark::Bullet => {
                let x = if rtl { mark.right - 9.0 } else { mark.left + 4.0 };
                let d = Rect::new(x, top + ROW_H / 2.0 - 2.5, x + 5.0, top + ROW_H / 2.0 + 2.5);
                p.scene.fills.push(Fill::solid(d, 2.5, Ink::Accent));
                Ink::Muted
            }
            Mark::Number(n) => {
                let at = Format { align: if rtl { kolada_text::Align::End } else { kolada_text::Align::Start }, ..small };
                line(p, n, &at, mark, Ink::Accent, 1.0);
                Ink::Muted
            }
            Mark::Task { done } => {
                let x = if rtl { mark.right - 13.0 } else { mark.left + 1.0 };
                let b = Rect::new(x, top + ROW_H / 2.0 - 6.0, x + 12.0, top + ROW_H / 2.0 + 6.0);
                p.scene.fills.push(if *done { Fill::solid(b, 3.0, Ink::Accent) } else { Fill::faded(b, 3.0, Ink::Faint, 0.45) });
                if *done { Ink::Faint } else { Ink::Muted }
            }
        };
        line_found(p, &item.text, &small, words, ink, query);
    }
}

/// A box's picture, cut to fill `r` (the box's foot): its top edge
/// straight, its lower corners the box's. The picture glyph on a quiet
/// field until it is decoded (off the UI thread, by the shell's
/// `PictureSource`); with its file's name under it when the vault does not
/// have it (`file` None once the box is read).
fn picture(p: &mut Painter, input: &Input, file: Option<&Path>, name: &str, r: Rect, sheet: Rect) {
    // Drawn a corner taller and cut at its top: only its foot rounds.
    let drawn = Rect::new(r.left, r.top - BOX_RADIUS, r.right, r.bottom);
    let cut = Rect::new(r.left, r.top.max(sheet.top), r.right, r.bottom.min(sheet.bottom));
    p.scene.overlay(Some(cut));
    let ready = file.zip(input.images).and_then(|(f, images)| images.borrow_mut().pixels(f, (r.width() * input.scale.max(1.0)).round() as u32));
    match ready {
        Some((px, fade)) => {
            let layer = p.scene.layer_now();
            let pixels = crate::quick::crop(&px, drawn.width() / drawn.height().max(1.0));
            p.scene.pictures.push(scene::Picture { layer, rect: drawn, pixels, alpha: fade, round: BOX_RADIUS });
        }
        None => {
            p.scene.fills.push(Fill::solid(drawn, BOX_RADIUS, Ink::Field));
            let format = Format { center_y: true, align: kolada_text::Align::Center, ..p.fonts.icon_small };
            if file.is_some() || r.height() < 52.0 {
                line(p, "\u{E91B}", &format, r, Ink::Faint, 1.0);
            } else {
                let mid = (r.top + r.bottom) / 2.0;
                let large = Format { size: 22.0, ..format };
                line(p, "\u{E91B}", &large, Rect::new(r.left, mid - 30.0, r.right, mid), Ink::Faint, 1.0);
                let file = name.replace("%20", " ");
                let file = file.rsplit(['/', '\\']).next().unwrap_or(&file).to_string();
                let label = Format { size: 12.5, center_y: true, wrap: false, ellipsis: true, align: kolada_text::Align::Center, ..p.fonts.label };
                line(p, &file, &label, Rect::new(r.left + BOX_PAD, mid + 2.0, r.right - BOX_PAD, mid + 22.0), Ink::Muted, 1.0);
            }
        }
    }
    // The boxes after it, on the sheet again.
    p.scene.overlay(Some(sheet));
}

/// A plain note's lines of words, each wrapped, as many as fit in `r`;
/// the last line that fits ends in an ellipsis when more follow. Each line
/// of the note is laid out on its own (a layout's lines count from their
/// own paragraph). With a search's `query`, what it found tinted.
fn words(p: &mut Painter, lines: &[String], format: &Format, r: Rect, ink: Ink, query: Option<&str>) {
    if r.width() < 1.0 {
        return;
    }
    let eng = p.eng;
    let wrapped = Format { wrap: true, ellipsis: false, center_y: false, ..*format };
    let make = |t: &str| {
        scene::kept_layout(eng, t, &wrapped, r.width(), 10_000.0, 0, || scene::layout_styled(eng, t, &Format { rtl: Some(scene::is_rtl(t)), ..wrapped }, r.width(), 10_000.0, &[]))
    };
    let mut y = r.top;
    let across = (r.left, r.right);
    for (k, text) in lines.iter().filter(|t| !t.is_empty()).enumerate() {
        let all = make(text);
        let rows = all.lines();
        let found = query.map_or_else(Vec::new, |q| crate::found::ranges(text, q));
        let more = k + 1 < lines.len();
        let fit = rows.iter().take_while(|l| y + l.top + l.height <= r.bottom + 0.5).count();
        // Whole, with room for a line of the next under it (or none next).
        let row_h = rows.first().map_or(0.0, |l| l.height);
        if fit == rows.len() && (!more || y + all.measure().height + row_h <= r.bottom + 0.5) {
            crate::found::tint_ranges(p.scene, &all, r.left, y, &found, across);
            y += all.measure().height;
            p.scene.texts.push(Text { x: r.left, y: y - all.measure().height, layout: all, ink, alpha: 1.0 });
            continue;
        }
        // The last line that fits, cut with an ellipsis; those before it whole.
        let Some(last) = fit.checked_sub(1).map(|k| &rows[k]) else { return };
        let head = text[..last.start].trim_end();
        if !head.is_empty() {
            let l = make(head);
            let tinted: Vec<(usize, usize)> = found.iter().filter(|&&(a, _)| a < head.len()).map(|&(a, b)| (a, b.min(head.len()))).collect();
            crate::found::tint_ranges(p.scene, &l, r.left, y, &tinted, across);
            p.scene.texts.push(Text { x: r.left, y, layout: l, ink, alpha: 1.0 });
        }
        // More follows it: it says so, cut or not.
        let tail = text[last.start..].trim_end();
        let tail = if fit == rows.len() { format!("{tail}\u{2026}") } else { tail.to_string() };
        let at = Rect::new(r.left, y + last.top, r.right, y + last.top + last.height);
        // A search's words: cut at a word's end.
        if query.is_some() {
            snippet_found(p, &tail, format, at, ink, query);
        } else {
            line(p, &tail, format, at, ink, 1.0);
        }
        return;
    }
}

/// `line`, with what a search for `query` found in it tinted behind it.
fn line_found(p: &mut Painter, text: &str, format: &Format, r: Rect, ink: Ink, query: Option<&str>) {
    let Some(q) = query.filter(|_| !text.is_empty() && r.width() >= 1.0) else { return line(p, text, format, r, ink, 1.0) };
    let layout = one_line(p, text, format, r);
    crate::found::tint(p.scene, &layout, r.left, r.top, q, (r.left, r.right));
    p.scene.texts.push(Text { x: r.left, y: r.top, layout, ink, alpha: 1.0 });
}

/// A search's words around its match on one line, as `line_found` draws
/// them, cut short at the end of a word (`found::snippet_line`).
fn snippet_found(p: &mut Painter, text: &str, format: &Format, r: Rect, ink: Ink, query: Option<&str>) {
    if text.is_empty() || r.width() < 1.0 {
        return;
    }
    let format = Format { center_y: true, ..*format };
    let layout = crate::found::snippet_line(p.eng, text, &format, r.width(), r.height());
    if let Some(q) = query {
        crate::found::tint(p.scene, &layout, r.left, r.top, q, (r.left, r.right));
    }
    p.scene.texts.push(Text { x: r.left, y: r.top, layout, ink, alpha: 1.0 });
}

/// `text` laid out on one line in `r`, cut short with an ellipsis; kept
/// between frames, as the note's lines are.
fn one_line(p: &Painter, text: &str, format: &Format, r: Rect) -> std::rc::Rc<kolada_text::Layout> {
    let format = Format { wrap: false, ellipsis: true, center_y: true, ..*format };
    let eng = p.eng;
    scene::kept_layout(eng, text, &format, r.width(), r.height(), 0, || {
        scene::layout_styled(eng, text, &Format { rtl: Some(scene::is_rtl(text)), ..format }, r.width(), r.height(), &[])
    })
}

/// `text` on one line in `r`, cut short with an ellipsis.
fn line(p: &mut Painter, text: &str, format: &Format, r: Rect, ink: Ink, alpha: f32) {
    if text.is_empty() || r.width() < 1.0 {
        return;
    }
    let layout = one_line(p, text, format, r);
    p.scene.texts.push(Text { x: r.left, y: r.top, layout, ink, alpha });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(g: &mut Grid) -> f32 {
        let mut t = 0.0;
        while g.step(1.0 / 120.0) {
            t += 1.0 / 120.0;
            assert!(t < 2.0, "it settles");
        }
        t
    }

    #[test]
    fn the_four_states_follow_what_is_done() {
        let h = 800.0;
        let mut g = Grid::new(true);
        assert_eq!((g.phase, g.split), (Phase::Rest, 0.3), "a fresh page: the note's 30%, the boxes under it");
        assert!(g.typed() && g.phase == Phase::Writing, "typing: the note takes the window");
        assert!(!g.pointer(Some(400.0), h), "the middle of the window: nothing");
        assert!(g.pointer(Some(h - 4.0), h) && g.phase == Phase::Peek, "the foot: the boxes peek");
        assert!(!g.pointer(Some(h * 0.7 + 10.0), h), "over the peeked boxes: they stay");
        assert!(g.pointer(Some(h * 0.5), h) && g.phase == Phase::Writing, "away: they drop back");
        g.pointer(Some(h - 1.0), h);
        assert!(g.pointer(None, h) && g.phase == Phase::Writing, "out of the window: they drop back");
        g.pointer(Some(h - 1.0), h);
        assert!(g.clicked() && g.phase == Phase::Browse, "clicked: the boxes take the window's most");
        assert!(!g.pointer(Some(10.0), h), "browsing stays where the pointer goes");
        assert!(g.back_to_note() && g.phase == Phase::Writing, "a box opened, or the note clicked");
        assert!(!g.clicked(), "boxes not peeking take no click to browse");
        // The pill at the foot: over it the boxes peek (it is in the foot's
        // band); clicked, writing or peeking, they come up.
        assert!(g.pill_clicked() && g.phase == Phase::Browse, "the pill clicked while writing: the boxes come up");
        g.back_to_note();
        g.pointer(Some(h - 6.0), h);
        assert!(g.phase == Phase::Peek && g.pill_clicked() && g.phase == Phase::Browse, "and while they peek");
        assert!(g.pill_clicked() && g.phase == Phase::Writing, "riding their edge, it takes them down");
        assert!(g.fresh() && g.phase == Phase::Rest, "a fresh page: at rest");
        assert_eq!([Phase::Rest, Phase::Writing, Phase::Peek, Phase::Browse].map(Phase::note_share), [0.3, 1.0, 0.7, 0.3]);
    }

    #[test]
    fn the_split_springs_into_place_without_overshoot_then_holds_still() {
        let mut g = Grid::new(true);
        assert!(!g.moving(), "at rest from the start");
        g.typed();
        let mut last = g.split;
        let mut t = 0.0;
        while g.step(1.0 / 120.0) {
            assert!(g.split >= last - 1e-6 && g.split <= 1.0 + 1e-6, "it rises without passing its place");
            last = g.split;
            t += 1.0 / 120.0;
        }
        assert!(t < 0.6, "settled in {t} s");
        assert_eq!(g.split, 1.0);
        assert!(!g.moving() && !g.step(1.0 / 60.0), "settled: no more frames");
        // Turned around mid-way, it goes the other way at once.
        g.pointer(Some(799.0), 800.0);
        g.step(0.05);
        g.pointer(None, 800.0);
        assert!(settle(&mut g) < 0.6 && g.split == 1.0);
    }

    #[test]
    fn only_the_boxes_in_sight_are_read_once_and_a_saved_note_comes_first() {
        let vault = std::env::temp_dir().join(format!("kolada-grid-notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        std::fs::create_dir_all(&vault).unwrap();
        for i in 0..60 {
            std::fs::write(vault.join(format!("n{i}.md")), format!("n{i}\nline {i}\n")).unwrap();
        }
        let (w, h) = (1200.0, 800.0);
        let g = Grid::new(true);
        let mut notes = Notes::default();
        notes.listed(kolada_core::vault::every_note(&vault));
        assert_eq!(notes.list.len(), 60);
        let first = notes.wanted(&g, w, h);
        assert!(!first.is_empty() && first.len() < 60, "a screenful: {}", first.len());
        assert!(notes.wanted(&g, w, h).is_empty(), "asked once");
        let (_, boxes) = notes.boxes(&g, w, h);
        assert!(boxes.iter().all(|b| b.preview.is_none()), "names until read");
        notes.read(read(&vault, first));
        let (start, boxes) = notes.boxes(&g, w, h);
        assert_eq!(start, 0);
        assert!(boxes.iter().all(|b| b.preview.is_some_and(|p| p.line.starts_with("line "))));
        let last = notes.list[59].path.clone();
        notes.saved(&last);
        assert_eq!(notes.note(0), Some(last.as_str()), "the note saved is the newest");
        assert_eq!(notes.list.len(), 60, "not twice");
        assert_eq!(notes.wanted(&g, w, h), vec![last], "and its box is read again");
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn two_to_five_columns_of_cards_follow_the_windows_width() {
        assert_eq!(columns(2560.0), 5, "a very wide window: five, no more");
        assert_eq!(columns(1600.0), 5);
        assert_eq!(columns(1366.0), 5);
        assert_eq!(columns(1100.0), 4);
        assert_eq!(columns(960.0), 3, "half a 1920 screen");
        assert_eq!(columns(640.0), 2, "half a 1280 one, or a scaled 1920");
        assert_eq!(columns(300.0), 2, "never fewer than two");
        for w in (600..=3840).step_by(8).map(|w| w as f32) {
            let a = across(w);
            assert!(a.width >= BOX_LEAST && a.width <= BOX_MOST, "cards keep their width at {w}: {a:?}");
            assert!(a.width > a.height && a.height >= 140.0, "a card, not a strip, at {w}: {a:?}");
            assert!(a.left >= INSET - 0.5 && (w - a.right() - a.left).abs() <= 1.0, "centred at {w}: {a:?}");
        }
    }

    #[test]
    fn boxes_tile_the_sheet_and_only_those_showing_are_read() {
        let (w, h) = (1200.0, 800.0);
        let mut g = Grid::new(true);
        let cols = columns(w);
        let s = sheet(&g, w, h);
        assert_eq!(s.top, 240.0);
        let (a, b) = (place(s, 0, 0.0), place(s, 1, 0.0));
        assert!(a.right < b.left && a.top == b.top && (a.width(), a.height()) == (b.width(), b.height()), "side by side, one size");
        assert!(a.top >= shelf(s).top, "under the bar");
        let last = place(s, cols - 1, 0.0);
        assert!((w - last.right - (a.left - s.left)).abs() <= 1.0, "even margins");
        let below = place(s, cols, 0.0);
        assert_eq!((below.left, below.top), (a.left, a.bottom + GAP), "the next row under the first");
        let shown_now = shown(&g, 1000, w, h);
        assert_eq!(shown_now.start, 0);
        assert!(shown_now.len() <= 6 * cols && shown_now.len() >= cols, "a screenful, not a thousand: {shown_now:?}");
        // Scrolled, later ones; never past the last.
        assert!(g.scroll_by(1e9, 1000, w, h));
        let end = shown(&g, 1000, w, h);
        assert_eq!(end.end, 1000);
        assert!(place(s, 999, g.scroll).bottom <= h, "the last box in sight at the end");
        assert!(!g.scroll_by(50.0, 1000, w, h), "and no further");
        assert!(g.scroll_by(-1e9, 1000, w, h) && g.scroll == 0.0);
        // Writing, the boxes are out of sight: none are read.
        g.typed();
        settle(&mut g);
        assert_eq!(shown(&g, 1000, w, h), 0..0);
    }

    /// The arrows walk the boxes as they read: along a row, over the rows,
    /// never past the ends; Down from a row with none under it, the last.
    #[test]
    fn arrows_walk_the_boxes_and_stop_at_the_ends() {
        // Three across, five boxes: 0 1 2 / 3 4.
        assert_eq!(step(None, Step::Down, 3, 5, false), Some(0), "the first when none is");
        assert_eq!(step(Some(0), Step::Right, 3, 5, false), Some(1));
        assert_eq!(step(Some(0), Step::Left, 3, 5, false), Some(0));
        assert_eq!(step(Some(1), Step::Down, 3, 5, false), Some(4));
        assert_eq!(step(Some(2), Step::Down, 3, 5, false), Some(4), "none under it: the last");
        assert_eq!(step(Some(4), Step::Down, 3, 5, false), Some(4));
        assert_eq!(step(Some(4), Step::Up, 3, 5, false), Some(1));
        assert_eq!(step(Some(1), Step::Up, 3, 5, false), Some(1));
        assert_eq!(step(Some(1), Step::Left, 3, 5, true), Some(2), "right to left, Left is the next");
        assert_eq!(step(Some(0), Step::Down, 3, 0, false), None);
    }

    /// A search's notes are the boxes, from the first, with the words
    /// around each match; a PDF it found is not a box; cleared, the notes
    /// come back.
    #[test]
    fn a_search_shows_its_notes_as_the_boxes_and_clearing_it_the_notes_again() {
        let listed = |p: &str| Listed { path: p.to_string(), name: p.trim_end_matches(".md").to_string(), written: None };
        let mut notes = Notes::default();
        notes.listed(vec![listed("Plan.md"), listed("Groceries.md"), listed("Work/Ideas.md")]);
        assert_eq!(notes.count(), 3);
        let hit = |p: &str, snippet: Option<&str>| kolada_core::search::Hit { path: p.to_string(), name: p.to_string(), snippet: snippet.map(str::to_string) };
        let found = matches(&[hit("Work/Ideas.md", Some("a grid of notes")), hit("Scan.pdf", None), hit("Plan.md", None)]);
        notes.found(Some(found));
        assert!(notes.searching());
        assert_eq!((notes.count(), notes.note(0), notes.note(1)), (2, Some("Work/Ideas.md"), Some("Plan.md")));
        let g = Grid::at(Phase::Browse);
        let (first, boxes) = notes.boxes(&g, 1200.0, 800.0);
        assert_eq!((first, boxes.len()), (0, 2));
        assert_eq!((boxes[0].snippet, boxes[1].snippet), (Some("a grid of notes"), None));
        assert_eq!(notes.wanted(&g, 1200.0, 800.0), vec!["Work/Ideas.md".to_string(), "Plan.md".to_string()], "the matches in sight are read");
        notes.found(None);
        assert!(!notes.searching());
        assert_eq!((notes.count(), notes.note(0)), (3, Some("Plan.md")));
    }

    /// A search's cards: a table or list that shows the match is tinted
    /// where it is, with no line of it over them (the card would say it
    /// twice); one that does not show it keeps that line; the line, cut, ends
    /// at a word with an ellipsis.
    #[test]
    fn a_card_tints_the_match_in_its_own_table_or_list_and_says_it_once() {
        use crate::preview::preview;
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let note = "Plan\nShip it.";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(note).collect();
        // Made-up notes: the match in a table's row, in a list's item, in
        // neither (a table of other things, the match further down).
        let table = preview("Shop", "Shop\n| Fruit | Count |\n| --- | --- |\n| Lemons | 4 |\n| Limes | 2 |\n");
        let list = preview("Tart", "Tart\n- Flour\n- [ ] Two lemons, zested\n- Sugar\n");
        let other = preview("Costs", "Costs\n| Item | Cost |\n| --- | --- |\n| Tea | 3 |\n");
        let far = "Further down, the lemons for the tart, and a long way on past them to the end of the line";
        let boxes = [
            BoxView { name: "Shop", preview: Some(&table), picture: None, snippet: Some("Lemons \u{00B7} 4") },
            BoxView { name: "Tart", preview: Some(&list), picture: None, snippet: Some("Two lemons, zested") },
            BoxView { name: "Costs", preview: Some(&other), picture: None, snippet: Some(far) },
        ];
        let (width, height) = (1200.0, 800.0);
        let g = Grid::at(Phase::Browse);
        let shown = shown(&g, boxes.len(), width, height);
        let view = GridView { grid: &g, count: boxes.len(), first: shown.start, boxes: &boxes[shown.clone()], search: Some(GridSearch { query: "lemons", caret: Some(6) }), chosen: None };
        let input = Input { text: note, lines: &lines, page_fade: 1.0, grid: Some(view), ..Default::default() };
        let s = scene::build(&eng, &fonts, &input, width, height).unwrap();
        let cards: Vec<Rect> = s.hits.iter().filter(|(_, t)| matches!(t, Target::GridBox(_))).map(|(r, _)| *r).collect();
        assert_eq!(cards.len(), 3);
        let inside = |r: Rect, x: f32, y: f32| x >= r.left - 0.5 && x < r.right && y >= r.top && y < r.bottom;
        let texts = |r: Rect| s.texts.iter().filter(|t| inside(r, t.x, t.y)).map(|t| t.layout.text().to_string()).collect::<Vec<_>>();
        let tints = |r: Rect| s.fills.iter().filter(|f| f.ink == Ink::Mark && f.radius == 3.0 && inside(r, f.rect.left + 2.0, f.rect.top)).map(|f| f.rect).collect::<Vec<_>>();
        for (k, said) in [(0, "Lemons"), (1, "Two lemons, zested")] {
            let t = texts(cards[k]);
            assert_eq!(t.iter().filter(|x| x.contains("emons")).count(), 1, "card {k}: the match said once, in its own {}", ["table", "list"][k]);
            assert!(t.iter().any(|x| x == said), "card {k}: its {} shows it", ["cell", "item"][k]);
            let tinted = tints(cards[k]);
            assert_eq!(tinted.len(), 1, "card {k}: tinted where it is: {tinted:?}");
            let at = s.texts.iter().find(|x| x.layout.text() == said).unwrap();
            assert!(tinted[0].top >= at.y - 1.0 && tinted[0].bottom <= at.y + at.layout.box_height() + 1.0, "card {k}: on its row");
        }
        // The match not in the table: one line of it over the table, cut
        // at a word with an ellipsis, the table as it is.
        let t = texts(cards[2]);
        let line = t.iter().find(|x| x.contains("lemons")).expect("the line over the table");
        assert!(line.ends_with('\u{2026}'), "cut, with an ellipsis");
        let kept = line.trim_start_matches('\u{2026}').trim_end_matches(['\u{2026}', '\u{200E}']);
        let at = far.find(kept).expect("words of the line, in order");
        assert!(far[at + kept.len()..].starts_with([' ', ',']), "it ends at a word's end, {} of {} chars", kept.len(), far.len());
        assert!(t.iter().any(|x| x == "Tea") && tints(cards[2]).len() == 1, "the table under it, the line's match tinted");
    }

    /// The grid searching "lemons" over two made-up notes, one with the
    /// match in its table and one in its list, in graphite and paper at 2x.
    #[test]
    #[ignore = "writes the grid's search cards to KOLADA_DUMP_DIR, for manual inspection"]
    fn dump_search_cards() {
        use crate::preview::preview;
        let dir = std::path::PathBuf::from(std::env::var("KOLADA_DUMP_DIR").unwrap());
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let note = "Plan\nShip it.";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(note).collect();
        let budget = preview("Budget", "Budget\n| Item | Count |\n| --- | --- |\n| Lemons | 4 |\n| Eggs | 12 |\n");
        let market = preview("Market day", "Market day\n- A bag for the lemons and the herbs\n- Bread from the corner\n- Honey\n");
        let boxes = [
            BoxView { name: "Budget", preview: Some(&budget), picture: None, snippet: Some("Lemons \u{00B7} 4") },
            BoxView { name: "Market day", preview: Some(&market), picture: None, snippet: Some("A bag for the lemons and the herbs") },
        ];
        let (width, height) = (1000.0, 640.0);
        let g = Grid::at(Phase::Browse);
        let shown = shown(&g, boxes.len(), width, height);
        let view = GridView { grid: &g, count: boxes.len(), first: shown.start, boxes: &boxes[shown.clone()], search: Some(GridSearch { query: "lemons", caret: Some(6) }), chosen: None };
        let input = Input { text: note, lines: &lines, page_fade: 1.0, grid: Some(view), ..Default::default() };
        let s = scene::build(&eng, &fonts, &input, width, height).unwrap();
        for (name, theme) in [("graphite", &crate::theme::DARK), ("paper", &crate::theme::LIGHT)] {
            let list = crate::display::record(&eng, &s, theme, 2.0).unwrap();
            crate::raster::Surface::draw_new((2000, 1280, 192), &list).dump(&dir.join(format!("grid-cards-lemons-{name}.bmp"))).unwrap();
        }
    }
}
