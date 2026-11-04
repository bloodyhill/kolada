//! The window chrome: floating side menu, toolbar, their collapsed pills,
//! and the window buttons.
//!
//! Each panel has two amounts, both 0..1: how much room it takes in the
//! layout (`dock`) and how visible it is (`show`). Docked panels have both
//! at one. In focus mode (collapsed) both are 0 and only a pill remains;
//! moving the pointer near it reveals the panel as an overlay (`show` rises
//! while `dock` stays 0), so the note never jumps. Everything is a function of
//! these amounts, so the same code draws resting states and animation frames.

use kolada_core::session::RowKind;
use kolada_core::vault;
use kolada_text::{Align, Format};
use std::rc::Rc;
use windows_core::Result;

use crate::scene::{self, Eng, Fill, Fonts, Ink, Input, Platform, Rect, Scene, SearchView, Setting, Target, Text, Tool, Transfer, TrashEntry, VectorMark, WaveShape};
use crate::vector::Logo;

/// Gap between floating panels and the window edge, DIPs.
pub const MARGIN: f32 = 8.0;
pub const SIDEBAR_WIDTH: f32 = 264.0;
const PANEL_RADIUS: f32 = 12.0;
const CONTROL_RADIUS: f32 = 8.0;
const CONTROL: f32 = 32.0;
/// The New note mark on Windows, square (Basil's outline fills about 70% of it).
const NEW_NOTE_MARK: f32 = 20.0;
const PAD: f32 = 10.0;
const ROW: f32 = 30.0;
const INDENT: f32 = 14.0;
/// The toolbar's row: the room it keeps above the note, revealed or not.
pub const TOOLBAR_HEIGHT: f32 = 40.0;
/// The toolbar (and the selection bar) is a capsule sized to its buttons,
/// on one 4 DIP grid: its height, a button's square, the same inset on
/// every side ((34 - 28) / 2), the space between buttons, and the quiet
/// gap before Aa.
pub(crate) const BAR: f32 = 34.0;
pub(crate) const BTN: f32 = 28.0;
pub(crate) const INSET: f32 = (BAR - BTN) / 2.0;
pub(crate) const BTN_GAP: f32 = 4.0;
const QUIET_GAP: f32 = 12.0;
/// How far the toolbar comes down as it shows (and rises as it goes).
const BAR_DROP: f32 = 6.0;
pub const CAPTION_WIDTH: f32 = 46.0;
/// Widest the note column gets, by the Line width option.
fn column_max(option: u8) -> f32 {
    match option {
        1 => 600.0,
        2 => 860.0,
        _ => 700.0,
    }
}
const COLUMN_PAD: f32 = 48.0;
const SHADOW_BLUR: f32 = 18.0;

/// Collapsed pills: thickness at rest.
const PILL: f32 = 6.0;
/// Invisible reveal areas: the left edge strip, the wider band around the
/// side pill, and the area around the top pill.
const EDGE_ZONE: f32 = 32.0;
const SIDE_PILL_ZONE: f32 = 56.0;
const TOP_ZONE_HEIGHT: f32 = 28.0;
const TOP_ZONE_REACH: f32 = 110.0;
/// How far outside a revealed panel the pointer can stray before it hides.
const KEEP_OPEN: f32 = 28.0;
/// The top pill at its shortest, DIPs: shorter, it does not show.
const PILL_LEAST: f32 = 60.0;

pub(crate) mod icon {
    pub const SEARCH: &str = "\u{E721}";
    pub const ADD: &str = "\u{E710}";
    pub const SETTINGS: &str = "\u{E713}";
    /// A left-pointing arrow into a pane edge: collapse to the left.
    pub const COLLAPSE_LEFT: &str = "\u{E8A0}";
    pub const PIN: &str = "\u{E718}";
    pub const CHEVRON_RIGHT: &str = "\u{E76C}";
    pub const CHEVRON_DOWN: &str = "\u{E70D}";
    pub const CHEVRON_UP: &str = "\u{E70E}";
    pub const PAGE: &str = "\u{E7C3}";
    pub const DELETE: &str = "\u{E74D}";
    pub const SUN: &str = "\u{E706}";
    pub const MOON: &str = "\u{E708}";
    pub const SYSTEM: &str = "\u{E7A1}";
    pub const SMALLER: &str = "\u{E8E7}";
    pub const LARGER: &str = "\u{E8E8}";
    pub const MINIMIZE: &str = "\u{E921}";
    pub const MAXIMIZE: &str = "\u{E922}";
    pub const RESTORE: &str = "\u{E923}";
    pub const CLOSE: &str = "\u{E8BB}";
    pub const BOLD: &str = "\u{E8DD}";
    pub const ITALIC: &str = "\u{E8DB}";
    pub const STRIKE: &str = "\u{EDE0}";
    pub const LIST: &str = "\u{E8FD}";
    pub const CHECKLIST: &str = "\u{E9D5}";
    pub const QUOTE: &str = "\u{E9B2}";
    pub const CODE: &str = "\u{E943}";
    pub const LINK: &str = "\u{E71B}";
    pub const BACK: &str = "\u{E72B}";
    pub const SEND: &str = "\u{E724}";
    pub const TO_MAIN: &str = "\u{E740}";
    pub const MINIMAL: &str = "\u{E73F}";
    pub const RESTORE_NOTE: &str = "\u{E7A7}";
    pub const HIGHLIGHT: &str = "\u{E7E6}";
    pub const GRID: &str = "\u{E80A}";
}

/// A menu icon's tile and the step from one to the next: the toolbar's
/// gap between them, the same along the side menu's foot and the grid's
/// top edge.
pub(crate) const ICON_TILE: f32 = 36.0;
pub(crate) const ICON_PITCH: f32 = ICON_TILE + BTN_GAP;

/// The menu's icons, in the one order both rows have them: the view,
/// the appearance, the trash, settings.
pub(crate) fn menu_icons(input: &Input) -> [(Target, &'static str); 4] {
    let theme = match input.options[0] {
        1 => icon::SUN,
        2 => icon::MOON,
        _ => icon::SYSTEM,
    };
    [(Target::ViewToggle, icon::GRID), (Target::ThemeCycle, theme), (Target::Trash, icon::DELETE), (Target::Options, icon::SETTINGS)]
}

/// Toolbar contents, left to right; then a quiet gap, Aa (text size and
/// line width) and the pin. Heading is the shortcuts' and the selection
/// bar's.
const TOOLS: [Tool; 7] = [Tool::Bold, Tool::Italic, Tool::List, Tool::Checklist, Tool::Quote, Tool::Code, Tool::Link];

/// A tool's icon.
pub(crate) fn tool_glyph(tool: Tool) -> &'static str {
    match tool {
        // A letter, the same on every OS (the icon fonts' heading glyphs
        // differ: Segoe's E8D2 is a font's "AA").
        Tool::Heading => "H",
        Tool::Bold => icon::BOLD,
        Tool::Italic => icon::ITALIC,
        Tool::Strike => icon::STRIKE,
        Tool::List => icon::LIST,
        Tool::Checklist => icon::CHECKLIST,
        Tool::Quote => icon::QUOTE,
        Tool::Code => icon::CODE,
        Tool::Link => icon::LINK,
        Tool::Highlight => icon::HIGHLIGHT,
    }
}

/// A button's name and its keys on the platform's keyboard (none where it
/// has none), for the label that shows when the pointer rests on it; None
/// for what has no label. `docked`: the toolbar is pinned to stay shown.
pub fn button_name(target: Target, platform: Platform, docked: bool) -> Option<(&'static str, &'static str)> {
    let apple = apple_keys(platform);
    let k = |win: &'static str, mac: &'static str| if apple { mac } else { win };
    let tool = match target {
        Target::Tool(t) | Target::SelBar(t) => t,
        Target::Reading => return Some(("Text size and width", "")),
        Target::ToggleToolbar if docked => return Some(("Hide the toolbar", k("Ctrl+Shift+\\", "\u{21E7}\u{2318}\\"))),
        Target::ToggleToolbar => return Some(("Keep the toolbar shown", k("Ctrl+Shift+\\", "\u{21E7}\u{2318}\\"))),
        // The button says its own name and keys as it opens.
        Target::Quick(crate::quick::Hit::Fab) => return None,
        _ => return None,
    };
    Some(match tool {
        // The Mac's ⌘H hides the app: its headings are Option-Command.
        Tool::Heading if platform == Platform::Mac => ("Heading", "\u{2325}\u{2318}1"),
        Tool::Heading => ("Heading", k("Ctrl+1", "\u{2318}1")),
        Tool::Bold => ("Bold", k("Ctrl+B", "\u{2318}B")),
        Tool::Italic => ("Italic", k("Ctrl+I", "\u{2318}I")),
        Tool::Strike => ("Strikethrough", k("Ctrl+Shift+X", "\u{21E7}\u{2318}X")),
        Tool::List => ("Bulleted list", k("Ctrl+Shift+8", "\u{2318}8")),
        Tool::Checklist => ("Checklist", k("Ctrl+L", "\u{2318}L")),
        Tool::Quote => ("Quote", ""),
        Tool::Code => ("Code", k("Ctrl+E", "\u{2318}E")),
        Tool::Link => ("Link", k("Ctrl+K", "\u{2318}K")),
        Tool::Highlight => ("Highlight", k("", "\u{21E7}\u{2318}U")),
    })
}

/// Cubic ease-out: fast start, gentle stop.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// A pill fades out during the first half of its panel's reveal, so it
/// never shows through the panel sliding in over it.
fn pill_alpha(show: f32) -> f32 {
    (1.0 - 2.0 * show).clamp(0.0, 1.0)
}

pub(crate) struct Painter<'a> {
    pub(crate) eng: &'a Eng,
    pub(crate) fonts: &'a Fonts,
    pub(crate) hover: Option<Target>,
    pub(crate) scene: &'a mut Scene,
}

thread_local! {
    /// Where a laid out icon's ink is (see `ink_middle`), by its text, its
    /// format and box, the engine's fonts, the scale and where in a pixel
    /// it starts.
    static OPTICAL: std::cell::RefCell<std::collections::HashMap<(u64, u64, [u32; 6]), Option<(f32, f32)>>> = Default::default();
}

/// The middle of what `layout` draws when its box's corner is at
/// `phase` (pixels, within the first) at `scale` pixels a DIP: pixels from
/// that pixel's corner. An icon font sets a glyph where its metrics put it
/// (Segoe Fluent Icons on an em from the line's top, Kolada Icons
/// elsewhere again), and a small glyph's pixels are not its outline's:
/// centred by its line, an icon sat high in its button, off its
/// highlight. Measured once per icon, scale and place in a pixel.
pub(crate) fn ink_middle(eng: &Eng, layout: &kolada_text::Layout, phase: (f32, f32), scale: f32) -> Option<(f32, f32)> {
    let q = |v: f32| (v * 64.0).round() as u32;
    let key = (scene::text_hash(layout.text()), layout.format().id(), [q(layout.width()), q(layout.box_height()), q(scale), q(phase.0), q(phase.1), eng.borrow().generation()]);
    if let Some(d) = OPTICAL.with(|c| c.borrow().get(&key).copied()) {
        return d;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    {
        let mut e = eng.borrow_mut();
        for g in layout.glyphs(phase, scale) {
            // Direction marks and spaces have an empty picture: no ink.
            if let Some(m) = e.mask(&g).filter(|m| m.width > 0 && m.height > 0) {
                let (left, top) = ((g.x + m.left) as f32, (g.y - m.top) as f32);
                x0 = x0.min(left);
                y0 = y0.min(top);
                x1 = x1.max(left + m.width as f32);
                y1 = y1.max(top + m.height as f32);
            }
        }
    }
    let d = (x0 < x1 && y0 < y1).then(|| ((x0 + x1) / 2.0, (y0 + y1) / 2.0));
    OPTICAL.with(|c| c.borrow_mut().insert(key, d));
    d
}

impl Painter<'_> {
    pub(crate) fn text(&mut self, text: &str, format: &Format, r: Rect, ink: Ink) -> Result<()> {
        let eng = self.eng;
        let layout = scene::kept_layout(eng, text, format, r.width(), r.height(), 0, || {
            scene::layout_styled(eng, text, &Format { rtl: Some(scene::is_rtl(text)), ..*format }, r.width(), r.height(), &[])
        });
        self.scene.texts.push(Text { x: r.left, y: r.top, layout, ink, alpha: 1.0 });
        Ok(())
    }

    pub(crate) fn icon(&mut self, glyph: &str, r: Rect, ink: Ink) -> Result<()> {
        let format = self.fonts.icon.clone();
        self.text(glyph, &format, r, ink)
    }

    /// A glyph whose ink is in the middle of `r`: the capsules', whose
    /// highlights share their icons' centres. Every icon's ink is centred
    /// as the scene is finished (`scene::centre_icons`), this one with them.
    pub(crate) fn centred(&mut self, glyph: &str, format: &Format, r: Rect, ink: Ink) -> Result<()> {
        if format.role == kolada_text::Role::Icon {
            return self.text(glyph, format, r, ink);
        }
        // Letters set as a mark (Aa): their ink in the middle too, measured
        // finely (at 4x) where it is laid out.
        let eng = self.eng;
        let layout = scene::kept_layout(eng, glyph, format, r.width(), r.height(), 0, || {
            scene::layout_styled(eng, glyph, &Format { rtl: Some(scene::is_rtl(glyph)), ..*format }, r.width(), r.height(), &[])
        });
        let (dx, dy) = ink_middle(eng, &layout, (0.0, 0.0), 4.0).map_or((0.0, 0.0), |(x, y)| (r.width() / 2.0 - x / 4.0, r.height() / 2.0 - y / 4.0));
        self.scene.texts.push(Text { x: r.left + dx, y: r.top + dy, layout, ink, alpha: 1.0 });
        Ok(())
    }

    /// A capsule's button: lit while what it makes is where the caret is,
    /// a shade under the pointer; either exactly its own round square,
    /// around its icon's middle.
    pub(crate) fn tool_button(&mut self, r: Rect, target: Target, glyph: &str, lit: bool) -> Result<()> {
        if lit {
            self.scene.fills.push(Fill::solid(r, BTN / 2.0, Ink::Selection));
        }
        self.button(r, target, BTN / 2.0);
        let ink = if lit || self.hover == Some(target) { Ink::Text } else { Ink::Muted };
        // An icon, or a letter (Heading's H) in the interface's face.
        let format = if glyph.is_ascii() { self.fonts.glyph } else { self.fonts.icon };
        self.centred(glyph, &format, r, ink)
    }

    /// A capsule (the toolbar, the selection bar): a fill a shade off the
    /// page and only a whisper of shadow (it reads on paper; graphite
    /// barely shows it). No rim.
    pub(crate) fn capsule(&mut self, r: Rect) {
        let fills = &mut self.scene.fills;
        fills.push(whisper(r, BAR / 2.0));
        fills.push(Fill::solid(r, BAR / 2.0, Ink::Panel));
    }

    /// A floating panel: a soft, diffuse shadow and its fill, never a rim.
    /// `lift` deepens the shadow for panels floating over the note.
    pub(crate) fn panel(&mut self, r: Rect, lift: f32) {
        self.panel_of(r, lift, Ink::Panel);
    }

    /// A floating panel in `ink` (the side menu's own, `Ink::Side`).
    pub(crate) fn panel_of(&mut self, r: Rect, lift: f32, ink: Ink) {
        let fills = &mut self.scene.fills;
        fills.push(Fill::shadow(r.offset(0.0, 1.0), PANEL_RADIUS, 2.0, 0.5));
        fills.push(Fill::shadow(r.offset(0.0, 2.0 + 4.0 * lift), PANEL_RADIUS, SHADOW_BLUR * (1.0 + lift), 1.0 + lift));
        fills.push(Fill::solid(r, PANEL_RADIUS, ink));
    }

    /// A button's hover highlight, and its click target.
    pub(crate) fn button(&mut self, r: Rect, target: Target, radius: f32) {
        if self.hover == Some(target) {
            self.scene.fills.push(Fill::solid(r, radius, Ink::Hover));
        }
        self.scene.hits.push((r, target));
    }

    /// A collapsed pill: an elastic string, quiet at rest. The pointer
    /// pulls it into a wave (see `wave`), which it follows, ripples with and
    /// rings back from. `vertical` pills sit on the left edge and reach
    /// right; others sit on top and reach down.
    fn pill(&mut self, r: Rect, vertical: bool, wave: &[f32], alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let mut d = [0.0; crate::wave::SAMPLES];
        for (x, &v) in d.iter_mut().zip(wave) {
            *x = v;
        }
        let pull = crate::wave::Wave::pull(&d);
        let shape = Some(WaveShape { vertical, d });
        let half = PILL / 2.0;
        let fills = &mut self.scene.fills;
        // Barely-there at rest, a little brighter where pulled, with only a
        // hint of the accent: it should feel drawn, not lit.
        fills.push(Fill { wave: shape, ..Fill::faded(r, half, Ink::Faint, (0.35 + 0.35 * pull) * alpha) });
        if pull > 0.01 {
            fills.push(Fill { wave: shape, ..Fill::faded(r, half, Ink::Accent, 0.22 * pull * alpha) });
        }
    }
}

/// The barely-there shadow under what floats over the note from the
/// toolbar (its capsule, the selection bar, the Aa card, a button's name):
/// a little below it, soft and faint, rounded as it is.
pub(crate) fn whisper(r: Rect, radius: f32) -> Fill {
    Fill::shadow(r.offset(0.0, 1.0), radius, 6.0, 0.8)
}

/// The space that keeps the quick actions apart from the window buttons.
const ACTIONS_GAP: f32 = 16.0;

/// Where the OS draws its window buttons when it does (the Mac's traffic
/// lights, at the top left), DIPs: nothing of ours goes under them.
pub const TRAFFIC_LIGHTS: Rect = Rect { left: 12.0, top: 6.0, right: 80.0, bottom: 30.0 };
/// How far right of them, and below them, our own controls keep.
const LIGHTS_GAP: f32 = 8.0;

/// The left edge, on the top row, our controls keep to: right of the OS's
/// window buttons, if it draws them there.
pub(crate) fn top_row_left(input: &Input) -> f32 {
    if input.native_caption { TRAFFIC_LIGHTS.right + LIGHTS_GAP } else { f32::MIN }
}

/// Where the side menu's content starts: its padding in, and below the OS's
/// window buttons when they are drawn over its corner.
fn side_content_top(input: &Input, panel_top: f32) -> f32 {
    let top = panel_top + PAD;
    if input.native_caption { top.max(TRAFFIC_LIGHTS.bottom + LIGHTS_GAP) } else { top }
}

/// A menu's width.
pub const MENU_WIDTH: f32 = 240.0;

/// The social drawer's platforms: how far in from the menu's text, each
/// one's cell, and its mark, DIPs.
const SOCIAL_INDENT: f32 = 16.0;
const SOCIAL_CELL: f32 = 30.0;
const SOCIAL_MARK: f32 = 16.0;

fn actions(input: &Input) -> &'static [(Target, &'static str)] {
    // The note alone (minimal, or in a window of its own) grows back to the
    // full window; the full window makes itself minimal.
    if input.compact {
        &[(Target::SendTo, icon::SEND), (Target::ToMain, icon::TO_MAIN)]
    } else if input.platform == Platform::Ios {
        // iOS has no window of the note's own to shrink to: its windows are
        // the system's (Split View, Slide Over, Stage Manager).
        &[(Target::SendTo, icon::SEND)]
    } else {
        &[(Target::SendTo, icon::SEND), (Target::Minimal, icon::MINIMAL)]
    }
}

/// Where the quick actions start, from the left.
pub(crate) fn actions_left(input: &Input, width: f32) -> f32 {
    // Left of the window's buttons; when the OS draws them (the Mac's, on
    // the left), at the window's right edge.
    let buttons = if input.native_caption { 0.0 } else { 3.0 * CAPTION_WIDTH + ACTIONS_GAP };
    width - buttons - actions(input).len() as f32 * CAPTION_WIDTH
}

/// Room kept right of the note column for related notes: the gap, a chip,
/// and the window's edge.
const MARGIN_ROOM: f32 = 44.0 + 232.0 + 24.0;

/// Room the docked side menu takes on the left, DIPs.
pub(crate) fn content_left(input: &Input, width: f32) -> f32 {
    (side_width(input, width) + 2.0 * MARGIN) * side_dock(input)
}

/// How docked the side menu is (eased). Never with the grid, which stands
/// in for it: then it only comes out over the note, to search.
fn side_dock(input: &Input) -> f32 {
    if input.grid.is_some() { 0.0 } else { ease(input.sidebar_dock) }
}

/// The side menu's width as it is now: docked, as dragged; revealed from
/// focus mode, its own; on the way between, between.
fn side_width(input: &Input, width: f32) -> f32 {
    let (own, set) = (menu_width(0.0, width), menu_width(input.sidebar_width, width));
    own + (set - own) * side_dock(input)
}

/// How narrow and how wide the side menu can be dragged.
pub const SIDEBAR_LEAST: f32 = 200.0;
pub const SIDEBAR_MOST: f32 = 440.0;

/// The side menu's width in a window `width` DIPs wide: as set by dragging
/// its edge (0 for the default), within what reads well, leaving the note
/// its room beside it.
pub fn menu_width(set: f32, width: f32) -> f32 {
    let wanted = if set > 0.0 { set.clamp(SIDEBAR_LEAST, SIDEBAR_MOST) } else { SIDEBAR_WIDTH };
    wanted.min((width - 2.0 * MARGIN - MIN_NOTE_BESIDE_MENU).max(SIDEBAR_WIDTH)).min(width - 2.0 * MARGIN)
}

/// The narrowest the note gets beside the docked side menu: a narrower
/// window tucks the menu away (it floats over the note when revealed).
const MIN_NOTE_BESIDE_MENU: f32 = 340.0;

/// Whether a window `width` DIPs wide has room to dock the side menu.
/// The vault as the settings show it: where it is, or on iOS, which keeps
/// vaults in places with no path worth reading (the app's container, the
/// Files app's providers), the folder's name.
pub fn vault_shown(platform: Platform, path: &str) -> &str {
    match platform {
        Platform::Ios => path.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or(path),
        _ => path,
    }
}

pub fn sidebar_fits(width: f32) -> bool {
    width >= SIDEBAR_WIDTH + 2.0 * MARGIN + MIN_NOTE_BESIDE_MENU
}

/// The room beside the window buttons the toolbar's row takes there (less
/// drops it to a row of its own; its buttons give way before that).
const TOOLBAR_LEAST: f32 = 300.0;

/// Where the toolbar's row is: beside the window buttons when the room
/// between the side menu and them holds it, else a row of its own under
/// them.
fn toolbar_top(input: &Input, width: f32, left: f32, right: f32) -> f32 {
    let beside = (right + 4.0).min(actions_left(input, width) - MARGIN) - (left - 4.0).max(top_row_left(input));
    if beside >= TOOLBAR_LEAST.min(right - left + 8.0) { MARGIN } else { CAPTION_BOTTOM + 6.0 }
}

/// The note column: where the note text goes, centered in the room the
/// docked panels leave (its bottom is the window's).
pub fn column(input: &Input, width: f32, height: f32) -> Rect {
    let content_left = content_left(input, width);
    let content_width = width - content_left;
    // Narrow windows keep less room around the text.
    let pad = (content_width * 0.06).clamp(20.0, COLUMN_PAD);
    let column_width = (content_width - 2.0 * pad).clamp(80.0, column_max(input.options[2]));
    let mut column_left = content_left + (content_width - column_width) / 2.0;
    // With the grid, the note starts where the boxes do: at their left edge,
    // or for a note read right to left, ends at their right.
    if input.grid.is_some() {
        let a = crate::grid::across(width);
        let column_width = column_width.min(a.right() - a.left);
        let left = if starts_right(input.text) { a.right() - column_width } else { a.left };
        return grid_column(input, width, height, left, column_width);
    }
    // Keep room on the right for related notes when the window allows:
    // move the column left of center, only as far as that room needs, so
    // the note stays as near the middle as it can. Never narrower for them:
    // the line width chosen wins, and where it leaves no room the margin
    // gives way. It depends on the window alone, so the text never jumps
    // as notes come and go.
    let side = (content_width - column_width) / 2.0;
    if !input.no_margin && side < MARGIN_ROOM && content_width - COLUMN_PAD - MARGIN_ROOM >= column_width {
        column_left = content_left + (2.0 * side - MARGIN_ROOM).max(COLUMN_PAD);
    }
    // A toolbar on a row of its own takes that row too. Tucked away, its
    // room is kept all the same: revealed, it floats there over no text,
    // and the note never moves as it comes and goes. Only a window without
    // one (the note alone) starts its note higher.
    let bar_row = toolbar_top(input, width, column_left, column_left + column_width) - MARGIN;
    let room = if input.compact { ease(input.toolbar_dock) } else { 1.0 };
    let mut column_top = MARGIN + 40.0 + (TOOLBAR_HEIGHT + 8.0 + bar_row) * room;
    // Where the column reaches under the OS's window buttons (the minimal
    // window, a narrow one), the dates above the title, 26 DIPs over the
    // column, keep below them (seen on the Mac: they touched the lights).
    if input.native_caption && column_left < TRAFFIC_LIGHTS.right + LIGHTS_GAP {
        column_top = column_top.max(TRAFFIC_LIGHTS.bottom + LIGHTS_GAP + 26.0);
    }
    column_top = clear_of_buttons(input, width, column_left + column_width, column_top);
    Rect::new(column_left, column_top, column_left + column_width, height)
}

/// Whether a note reads right to left, by its start (its first couple of
/// thousand bytes: a frame never reads a long note through).
fn starts_right(text: &str) -> bool {
    let mut end = text.len().min(2048);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    scene::is_rtl(&text[..end])
}

/// The note column beside the grid: at `left`, `column_width` wide, from
/// where the column always starts.
fn grid_column(input: &Input, width: f32, height: f32, left: f32, column_width: f32) -> Rect {
    let bar_row = toolbar_top(input, width, left, left + column_width) - MARGIN;
    let room = if input.compact { ease(input.toolbar_dock) } else { 1.0 };
    let mut top = MARGIN + 40.0 + (TOOLBAR_HEIGHT + 8.0 + bar_row) * room;
    if input.native_caption && left < TRAFFIC_LIGHTS.right + LIGHTS_GAP {
        top = top.max(TRAFFIC_LIGHTS.bottom + LIGHTS_GAP + 26.0);
    }
    let top = clear_of_buttons(input, width, left + column_width, top);
    Rect::new(left, top, left + column_width, height)
}

/// Where the column reaches under the window's own buttons and the quick
/// actions beside them (the minimal window, a narrow one), the dates above
/// the title, 26 DIPs over the column, keep below them, as from the Mac's
/// lights. A column `right` its right edge, starting at `top`.
fn clear_of_buttons(input: &Input, width: f32, right: f32, top: f32) -> f32 {
    if input.bare || (input.native_caption && actions(input).is_empty()) || right <= actions_left(input, width) - LIGHTS_GAP {
        return top;
    }
    top.max(CAPTION_BOTTOM + 3.0 + LIGHTS_GAP + 26.0)
}

/// Where a passing word goes (see `scene::Whisper`): its right edge and its
/// top. Above the note at its right (left of the find bar, when that is
/// open), and left of the window's buttons when it is up among them.
pub fn whisper_place(input: &Input, column: Rect, width: f32) -> (f32, f32) {
    let top = column.top - 30.0;
    let right = if input.find.is_some() { column.right - FIND_WIDTH - 12.0 } else { column.right };
    if top < CAPTION_BOTTOM + 2.0 { (right.min(actions_left(input, width) - 12.0), top) } else { (right, top) }
}

/// Lays out the chrome into `scene` and returns the note column.
pub fn layout(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32, scene: &mut Scene) -> Result<Rect> {
    let mut p = Painter { eng, fonts, hover: input.hover, scene };
    let (s_dock, s_show) = (side_dock(input), ease(input.sidebar_show));
    let t_dock = ease(input.toolbar_dock);

    // The top strip drags the window wherever nothing else is on top.
    if !input.bare {
        p.scene.hits.push((Rect::new(0.0, 0.0, width, MARGIN + TOOLBAR_HEIGHT), Target::Caption));
    }
    let column = column(input, width, height);
    let note_top = if input.bare { 0.0 } else { MARGIN + TOOLBAR_HEIGHT };
    p.scene.hits.push((Rect::new(content_left(input, width), note_top, width, height), Target::Note));

    // Text scrolled up passes under the window's top row: it dissolves into
    // the page there rather than run under the buttons. Not while the page
    // is at its top, where the date line sits in that row.
    let scrolled = if input.scroll_line == 0 { (input.scroll_px / SCROLLED_IN).clamp(0.0, 1.0) } else { 1.0 };
    if scrolled > 0.0 {
        let edge = column.top - UNDER_TOP_FADE;
        p.scene.fills.push(Fill { blur: UNDER_TOP_FADE, alpha: scrolled, ..Fill::solid(Rect::new(-UNDER_TOP_FADE, -2.0 * UNDER_TOP_FADE, width + UNDER_TOP_FADE, edge), 0.0, Ink::Page) });
    }

    let caption_left = width - 3.0 * CAPTION_WIDTH;
    // The note alone (minimal, or a note's own window): no toolbar, not even
    // tucked away, and no side menu.
    if !input.compact {
        toolbar(&mut p, input, column, width, t_dock, input.toolbar_show.clamp(0.0, 1.0))?;
        // The grid is a mode of its own: its bar has the search and the
        // menu's icons, and the side menu never shows with it.
        if input.grid.is_none() {
            side_menu(&mut p, input, width, height, s_dock, s_show)?;
        }
    }

    // Quick actions, then the window buttons: one row, all alike.
    let open = input.menu.is_some().then_some(input.menu_anchor).flatten().or(input.share.as_ref().map(|_| Target::SendTo));
    if !input.bare {
        for (i, &(target, glyph)) in actions(input).iter().enumerate() {
            caption_button(&mut p, target, glyph, actions_left(input, width) + i as f32 * CAPTION_WIDTH, open == Some(target), None)?;
        }
    }
    window_controls(&mut p, input, caption_left)?;
    // A picture being resized says its size, inside its lower right corner.
    if let Some((r, size)) = p.scene.picture_label.take() {
        let small = p.fonts.small.clone();
        let l = scene::layout(p.eng, &size, &small, 200.0, 22.0);
        let m = l.measure();
        let w = m.width + 16.0;
        let pill = Rect::new(r.right - w - 8.0, r.bottom - 30.0, r.right - 8.0, r.bottom - 8.0);
        if pill.left > r.left && pill.top > r.top {
            p.scene.fills.push(Fill::faded(pill, 11.0, Ink::Text, 0.78));
            p.scene.texts.push(Text { x: pill.left + 8.0, y: pill.top + (22.0 - m.height) / 2.0, layout: l, ink: Ink::Page, alpha: 1.0 });
        }
    }
    Ok(column)
}

/// How soft the page's edge under the top row is, DIPs, and how far the
/// page scrolls before that edge is all there.
const UNDER_TOP_FADE: f32 = 12.0;
const SCROLLED_IN: f32 = 24.0;

/// Where the top right row's highlight sits: a little below the window's
/// top edge, above the toolbar's middle.
const CAPTION_TOP: f32 = 5.0;
pub(crate) const CAPTION_BOTTOM: f32 = 37.0;

/// A button in the top right row (a quick action or a window button): a
/// rounded highlight under the pointer (red for Close) or while its menu is
/// open. It answers from the window's top edge down, so throwing the
/// pointer into the corner still hits it.
/// The face Windows draws its window buttons in (Windows 11's; Windows
/// 10 falls back to the app's own icons).
pub const WINDOWS_ICONS: &str = "Segoe Fluent Icons";

/// A button in the top row: `face`, the icons' face to draw it in where it
/// is not the app's own (the window's buttons on Windows).
/// Minimize, Maximize and Close from `left` (where the window draws its
/// own buttons).
fn window_controls(p: &mut Painter, input: &Input, left: f32) -> Result<()> {
    let buttons = [
        (Target::Minimize, icon::MINIMIZE),
        (Target::Maximize, if input.maximized { icon::RESTORE } else { icon::MAXIMIZE }),
        (Target::Close, icon::CLOSE),
    ];
    if !input.native_caption && !input.bare {
        for (i, (target, glyph)) in buttons.into_iter().enumerate() {
            // Windows' own window buttons, as every app there has them.
            let face = (input.platform == Platform::Windows).then_some(WINDOWS_ICONS);
            caption_button(p, target, glyph, left + i as f32 * CAPTION_WIDTH, false, face)?;
        }
    }
    Ok(())
}

/// The window's top row over a screen covering it (the welcome): it drags
/// the window, its buttons on it.
pub(crate) fn window_buttons(p: &mut Painter, input: &Input, width: f32) -> Result<()> {
    if !input.bare {
        p.scene.hits.push((Rect::new(0.0, 0.0, width, MARGIN + TOOLBAR_HEIGHT), Target::Caption));
    }
    window_controls(p, input, width - 3.0 * CAPTION_WIDTH)
}

fn caption_button(p: &mut Painter, target: Target, glyph: &str, left: f32, open: bool, face: Option<&'static str>) -> Result<()> {
    let hit = Rect::new(left, 0.0, left + CAPTION_WIDTH, CAPTION_BOTTOM + 3.0);
    let r = Rect::new(left + 3.0, CAPTION_TOP, left + CAPTION_WIDTH - 3.0, CAPTION_BOTTOM);
    let hover = p.hover == Some(target);
    let (bg, ink) = match (hover, open) {
        (true, _) if target == Target::Close => (Some(Ink::Danger), Ink::OnDanger),
        (true, _) => (Some(Ink::Hover), Ink::Text),
        (false, true) => (Some(Ink::Selection), Ink::Text),
        (false, false) => (None, Ink::Muted),
    };
    if let Some(bg) = bg {
        p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS, bg));
    }
    p.scene.hits.push((hit, target));
    let small = Format { face: face.or(p.fonts.icon_small.face), ..p.fonts.icon_small };
    p.text(glyph, &small, r, ink)
}

/// The toolbar's width with `tools` buttons, with Aa or not, and the pin.
fn bar_width(tools: usize, aa: bool) -> f32 {
    let n = tools as f32;
    2.0 * INSET + n * BTN + (n - 1.0).max(0.0) * BTN_GAP + if aa { QUIET_GAP + BTN } else { 0.0 } + BTN_GAP + BTN
}

/// The toolbar's buttons, left to right, that fit in `room` DIPs: all and
/// Aa, or without Aa, then without the least used, dropped in `DROPPED`
/// order (Heading keeps its place in it, for when it is in the bar).
fn tools_that_fit(room: f32) -> (Vec<Tool>, bool) {
    const DROPPED: [Tool; 5] = [Tool::Quote, Tool::Code, Tool::Link, Tool::Heading, Tool::List];
    let build = |dropped: usize| TOOLS.iter().copied().filter(|t| !DROPPED[..dropped].contains(t)).collect::<Vec<_>>();
    let all = build(0);
    if bar_width(all.len(), true) <= room {
        return (all, true);
    }
    for dropped in 0..=DROPPED.len() {
        let tools = build(dropped);
        if bar_width(tools.len(), false) <= room || dropped == DROPPED.len() {
            return (tools, false);
        }
    }
    unreachable!()
}

/// A spring's way from 0 to 1 as `t` goes 0..1: quick, then settling with
/// the least overshoot (under 3%: a fraction of a pixel of the bar's drop).
pub fn spring(t: f32) -> f32 {
    if t >= 1.0 {
        return 1.0;
    }
    if t <= 0.0 {
        return 0.0;
    }
    const OMEGA: f32 = 12.0;
    const DAMPING: f32 = 0.75;
    let wd = OMEGA * (1.0 - DAMPING * DAMPING).sqrt();
    1.0 - (-DAMPING * OMEGA * t).exp() * ((wd * t).cos() + DAMPING * OMEGA / wd * (wd * t).sin())
}

/// Where the toolbar's capsule rests, and what it holds: over the note
/// column, sized to its buttons; on the window buttons' row, its middle
/// is theirs.
fn bar_rect(input: &Input, column: Rect, width: f32) -> (Rect, Vec<Tool>, bool) {
    let caption_left = actions_left(input, width);
    let row = toolbar_top(input, width, column.left, column.right);
    // Beside the window buttons it stays between the side menu and them, as
    // its pill does; on a row of its own it may reach the window's edges.
    let (left, right) = if row == MARGIN {
        ((content_left(input, width) + MARGIN).max(top_row_left(input)), caption_left - MARGIN)
    } else {
        ((content_left(input, width) + MARGIN).max(MARGIN), width - MARGIN)
    };
    let (tools, aa) = tools_that_fit(right - left);
    let w = bar_width(tools.len(), aa);
    // Where its pill is: the window's middle, as far as its room allows.
    let mid = width / 2.0;
    let x = (mid - w / 2.0).min(right - w).max(left);
    let top = if row == MARGIN { (CAPTION_TOP + CAPTION_BOTTOM - BAR) / 2.0 } else { row };
    (Rect::new(x, top, x + w, top + BAR), tools, aa)
}

/// The toolbar. Tucked away (as it starts), only its pill and an area over
/// the note's top that brings it remain; brought (or pinned), it is a
/// capsule over the note column: the formatting, a quiet gap, Aa, and the
/// pin. `show` is how far it has come in, 0..1 not eased: it comes down
/// `BAR_DROP` on a spring and fades in.
fn toolbar(p: &mut Painter, input: &Input, column: Rect, width: f32, dock: f32, show: f32) -> Result<()> {
    let collapsed = dock < 0.5;
    let caption_left = actions_left(input, width);

    // Focus mode: the pill, and the invisible area that reveals the bar:
    // the top of the window over the note, and wider around the pill.
    // Centred on the window (the note column moves off centre, for related
    // notes or the grid), but between the side menu and the quick actions,
    // shorter where that is narrow.
    let (lo, hi) = ((content_left(input, width) + MARGIN + 8.0).max(top_row_left(input)), caption_left - 16.0);
    // Always on the window's middle: shorter where its room is short, and
    // gone where even a short one would not be centred (a narrow window).
    let mid = width / 2.0;
    let pill_len = (column.width() * 0.26).clamp(120.0, 260.0).min(2.0 * (mid - lo).min(hi - mid));
    let pill_box = Rect::new(mid - pill_len / 2.0, 5.0, mid + pill_len / 2.0, 5.0 + PILL);
    let pill_shows = pill_len >= PILL_LEAST;
    if collapsed {
        let zone = Rect::new(
            (pill_box.left - TOP_ZONE_REACH).min(column.left).max(top_row_left(input)),
            0.0,
            (pill_box.right + TOP_ZONE_REACH).max(column.right),
            TOP_ZONE_HEIGHT,
        );
        p.scene.hits.push((zone, Target::RevealToolbar));
        if pill_shows {
            p.scene.pills[1] = Some(pill_box);
            p.pill(pill_box, false, input.toolbar_wave, pill_alpha(ease(show)));
        }
    }
    if show <= 0.0 {
        return Ok(());
    }

    let (settled, tools, aa) = bar_rect(input, column, width);
    if collapsed {
        // Where it is going, not where it is on the way: a pointer that
        // outruns it coming in is over it all the same.
        p.scene.hits.push((settled.inset(-KEEP_OPEN), Target::ToolbarPanel));
    }
    let bar = settled.offset(0.0, -BAR_DROP * (1.0 - spring(show)));
    let (fills_from, texts_from) = (p.scene.fills.len(), p.scene.texts.len());
    p.capsule(bar);
    p.scene.hits.push((bar, Target::ToolbarPanel));

    let y = bar.top + INSET;
    let mut x = bar.left + INSET;
    for tool in tools {
        let r = Rect::new(x, y, x + BTN, y + BTN);
        p.tool_button(r, Target::Tool(tool), tool_glyph(tool), input.active_tools.contains(&tool))?;
        x += BTN + BTN_GAP;
    }
    // How the page reads (text size, line width), in a small card.
    if aa {
        x += QUIET_GAP - BTN_GAP;
        let r = Rect::new(x, y, x + BTN, y + BTN);
        let open = input.reading.is_some();
        if open {
            p.scene.fills.push(Fill::solid(r, BTN / 2.0, Ink::Selection));
        }
        p.button(r, Target::Reading, BTN / 2.0);
        let ink = if open || p.hover == Some(Target::Reading) { Ink::Text } else { Ink::Muted };
        let format = Format { size: 13.0, ..p.fonts.glyph };
        p.centred("Aa", &format, r, ink)?;
        x += BTN + BTN_GAP;
    }
    // Brought from focus mode: the pin keeps it shown. Pinned: it tucks it
    // away again. Small and faint, at the end.
    let r = Rect::new(x, y, x + BTN, y + BTN);
    p.button(r, Target::ToggleToolbar, BTN / 2.0);
    let ink = if p.hover == Some(Target::ToggleToolbar) { Ink::Muted } else { Ink::Faint };
    let format = Format { size: 12.0, ..p.fonts.icon };
    p.centred(if collapsed { icon::PIN } else { icon::CHEVRON_UP }, &format, r, ink)?;

    let a = ease(show);
    if a < 1.0 {
        p.scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= a);
        p.scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= a);
    }
    Ok(())
}

/// The Aa card's inset and a cell of its rows, DIPs.
const CARD_PAD: f32 = 4.0;
const SEGMENT: f32 = 36.0;

/// The toolbar's Aa card, under Aa: the text size (smaller, the size,
/// larger) over the line width (narrow, standard, wide), as the options
/// have them, a click nearer, in cells one above the other. Small, over
/// everything but menus.
pub fn reading(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, scene: &mut Scene) -> Result<()> {
    let Some(alpha) = input.reading else { return Ok(()) };
    let Some(aa) = scene.hits.iter().rev().find(|(_, t)| *t == Target::Reading).map(|(r, _)| *r) else { return Ok(()) };
    let w = 2.0 * CARD_PAD + 3.0 * SEGMENT;
    let h = 3.0 * CARD_PAD + 2.0 * BTN;
    let left = ((aa.left + aa.right) / 2.0 - w / 2.0).min(width - 8.0 - w).max(8.0);
    let a = ease(alpha);
    let top = aa.bottom + INSET + 6.0 - 4.0 * (1.0 - a);
    let card = Rect::new(left, top, left + w, top + h);
    scene.overlay(None);
    let (fills_from, texts_from) = (scene.fills.len(), scene.texts.len());
    let mut p = Painter { eng, fonts, hover: input.hover, scene };
    p.scene.fills.push(whisper(card, PANEL_RADIUS));
    p.scene.fills.push(Fill::solid(card, PANEL_RADIUS, Ink::Panel));
    p.scene.hits.push((card, Target::ReadingPanel));
    let icons = fonts.icon;
    // Text size: the size between smaller and larger, each dimmed (and
    // still) at its end.
    let size = input.options[1];
    let row = |k: f32| {
        let x = card.left + CARD_PAD + k * SEGMENT;
        Rect::new(x, card.top + CARD_PAD, x + SEGMENT, card.top + CARD_PAD + BTN)
    };
    for (k, up, glyph) in [(0.0, false, icon::SMALLER), (2.0, true, icon::LARGER)] {
        let (r, target) = (row(k), Target::TextSize(up));
        let end = scene::step_text_size(size, up).is_none();
        if !end {
            p.button(r, target, CONTROL_RADIUS);
        }
        let ink = if end { Ink::Faint } else if p.hover == Some(target) { Ink::Text } else { Ink::Muted };
        p.centred(glyph, &icons, r, ink)?;
    }
    p.centred(&scene::text_size(size).to_string(), &fonts.choice, row(1.0), Ink::Text)?;
    // Line width: a ruler as long as the column is wide.
    let y = card.top + 2.0 * CARD_PAD + BTN;
    for (k, value) in [1u8, 0, 2].into_iter().enumerate() {
        let x = card.left + CARD_PAD + k as f32 * SEGMENT;
        let r = Rect::new(x, y, x + SEGMENT, y + BTN);
        let target = Target::Width(value);
        let chosen = input.options[2] == value;
        // The chosen one sits in a shade; the pointer only brightens the others' rulers.
        if chosen {
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS, Ink::Selection));
        }
        p.scene.hits.push((r, target));
        let ink = if chosen || p.hover == Some(target) { Ink::Text } else { Ink::Muted };
        let half = match value {
            1 => 5.0,
            2 => 10.5,
            _ => 7.5,
        };
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        let fills = &mut p.scene.fills;
        fills.push(Fill::solid(Rect::new(cx - half, cy - 0.75, cx + half, cy + 0.75), 0.75, ink));
        for end in [cx - half, cx + half] {
            fills.push(Fill::solid(Rect::new(end - 0.75, cy - 4.5, end + 0.75, cy + 4.5), 0.75, ink));
        }
    }
    if a < 1.0 {
        scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= a);
        scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= a);
    }
    Ok(())
}

/// A button's name and keys (see `button_name`) in a small quiet card under
/// it, as a link's address shows: icon-only buttons say what they are when
/// the pointer rests on them. A selection bar's button's goes above that
/// bar when the bar is over the selection, leaving the text in view.
pub fn tooltip(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    const H: f32 = 24.0;
    let Some(target) = input.tooltip else { return Ok(()) };
    let Some((name, keys)) = button_name(target, input.platform, input.toolbar_dock >= 0.5) else { return Ok(()) };
    let Some(r) = scene.hits.iter().rev().find(|(_, t)| *t == target).map(|(r, _)| *r) else { return Ok(()) };
    let small = fonts.small;
    let measured = |text: &str| {
        let l = scene::single_line(eng, text, &small, 300.0, H);
        let w = l.measure().width.min(300.0);
        (Rc::new(l.with_width(&mut eng.borrow_mut(), w + 0.5)), w)
    };
    let (name, nw) = measured(name);
    let keys = (!keys.is_empty()).then(|| measured(keys));
    let w = 10.0 + nw + keys.as_ref().map_or(0.0, |(_, kw)| 8.0 + kw) + 10.0;
    let above = matches!(target, Target::SelBar(_)) && scene.selbar.zip(scene.selection_span).is_some_and(|(bar, (first, _))| bar.bottom <= first.top + 0.5);
    let top = if above { r.top - INSET - 6.0 - H } else { r.bottom + INSET + 6.0 };
    let top = top.min(height - 8.0 - H).max(8.0);
    let left = ((r.left + r.right) / 2.0 - w / 2.0).min(width - 8.0 - w).max(8.0);
    let card = Rect::new(left, top, left + w, top + H);
    scene.overlay(None);
    scene.fills.push(whisper(card, H / 2.0));
    scene.fills.push(Fill::solid(card, H / 2.0, Ink::Panel));
    scene.texts.push(Text { x: card.left + 10.0, y: card.top, layout: name, ink: Ink::Muted, alpha: 1.0 });
    if let Some((keys, _)) = keys {
        scene.texts.push(Text { x: card.left + 10.0 + nw + 8.0, y: card.top, layout: keys, ink: Ink::Faint, alpha: 1.0 });
    }
    scene.tooltip = Some(card);
    Ok(())
}

/// New note's keys, as the platform's keyboard writes them: the Command key
/// on Apple's (the iPad's too, seen there as Ctrl+N), Ctrl elsewhere.
fn new_note_keys(platform: Platform) -> &'static str {
    if apple_keys(platform) { "\u{2318}N" } else { "Ctrl+N" }
}

/// Whether the platform's keyboard has the Command key (the Mac's, an
/// iPad's), which shortcuts then name.
fn apple_keys(platform: Platform) -> bool {
    matches!(platform, Platform::Mac | Platform::Ios)
}

fn side_menu(p: &mut Painter, input: &Input, width: f32, height: f32, dock: f32, show: f32) -> Result<()> {
    let collapsed = dock < 0.5;

    // Focus mode: the pill, and the invisible areas that reveal the menu.
    let pill_len = (height * 0.42).max(120.0);
    let pill_mid = height * 0.46;
    if collapsed {
        // Not over the OS's window buttons, which are theirs.
        let below = if input.native_caption { TRAFFIC_LIGHTS.bottom } else { 0.0 };
        p.scene.hits.push((Rect::new(0.0, below, EDGE_ZONE, height), Target::RevealSidebar));
        let band = Rect::new(0.0, pill_mid - pill_len / 2.0 - 24.0, SIDE_PILL_ZONE, pill_mid + pill_len / 2.0 + 24.0);
        p.scene.hits.push((band, Target::RevealSidebar));
        let pill_box = Rect::new(5.0, pill_mid - pill_len / 2.0, 5.0 + PILL, pill_mid + pill_len / 2.0);
        p.scene.pills[0] = Some(pill_box);
        p.pill(pill_box, true, input.sidebar_wave, pill_alpha(show));
    }
    if show <= 0.0 {
        return Ok(());
    }

    let menu_width = side_width(input, width);
    let left = MARGIN - (menu_width + MARGIN + SHADOW_BLUR) * (1.0 - show);
    // Floating in a window too narrow for it and the window buttons, it
    // starts below them.
    let top = if MARGIN + menu_width > actions_left(input, width) - 8.0 { CAPTION_BOTTOM + 6.0 } else { MARGIN };
    let panel = Rect::new(left, top, left + menu_width, height - MARGIN);
    let floating = (show - dock).clamp(0.0, 1.0);
    // Floating, it goes over the toolbar whole, text and all: a layer of
    // its own (a layer draws all its shapes before any of its text).
    if floating > 0.0 {
        p.scene.overlay(None);
    }
    if collapsed {
        // Where it is going, not where it is on the way: a pointer that
        // outruns it sliding in is over it all the same.
        let settled = Rect::new(MARGIN, top, MARGIN + menu_width, height - MARGIN);
        p.scene.hits.push((settled.inset(-KEEP_OPEN), Target::SidebarPanel));
    }
    // Its own surface; what sits on it (search, New note, the chosen row,
    // the update card) is raised.
    p.panel_of(panel, floating, Ink::Side);
    p.scene.hits.push((panel, Target::SidebarPanel));

    let (x0, x1) = (panel.left + PAD, panel.right - PAD);
    let mut y = side_content_top(input, panel.top);

    // Search field, and the button that collapses (or, revealed, pins) it.
    let toggle = Rect::new(x1 - CONTROL, y, x1, y + CONTROL);
    let search = Rect::new(x0, y, toggle.left - 6.0, y + CONTROL);
    let searching = input.search.as_ref();
    if searching.is_some() {
        // Typed in: raised out of its inset, no rim.
        p.scene.fills.push(Fill::shadow(search.offset(0.0, 1.0), CONTROL_RADIUS, 4.0, 0.8));
        p.scene.fills.push(Fill::solid(search, CONTROL_RADIUS, Ink::Raised));
    } else {
        p.scene.fills.push(Fill::solid(search, CONTROL_RADIUS, Ink::Raised));
        if p.hover == Some(Target::Search) {
            p.scene.fills.push(Fill::faded(search, CONTROL_RADIUS, Ink::Hover, 0.6));
        }
    }
    p.scene.hits.push((search, Target::Search));
    p.icon(icon::SEARCH, Rect::new(search.left + 4.0, y, search.left + 32.0, y + CONTROL), Ink::Faint)?;
    let label = p.fonts.label.clone();
    // Once something is typed, a ✕ at the field's end ends the search: a
    // square the field's height, big enough for a finger.
    let clear = searching.filter(|s| !s.query.is_empty()).map(|_| Rect::new(search.right - CONTROL, y, search.right, y + CONTROL));
    let field = Rect::new(search.left + 32.0, y, clear.map_or(search.right - 8.0, |c| c.left), y + CONTROL);
    if let Some(c) = clear {
        let hot = p.hover == Some(Target::SearchClear);
        p.button(c.inset(4.0), Target::SearchClear, CONTROL_RADIUS - 2.0);
        let small = p.fonts.icon_small.clone();
        p.text(icon::CLOSE, &small, c, if hot { Ink::Text } else { Ink::Muted })?;
    }
    match searching {
        Some(s) => {
            let caret_x = if s.query.is_empty() {
                p.text("Search notes", &label, field, Ink::Faint)?;
                0.0
            } else {
                let l = scene::layout(p.eng, s.query, &label, field.width(), field.height());
                let cx = l.caret(s.caret.min(s.query.len())).x;
                p.scene.texts.push(Text { x: field.left, y: field.top, layout: l, ink: Ink::Text, alpha: 1.0 });
                cx
            };
            if s.focused {
                let caret = Rect::new(field.left + caret_x - 0.75, y + 8.0, field.left + caret_x + 0.75, y + CONTROL - 8.0);
                p.scene.fills.push(Fill::solid(caret, 0.75, Ink::Accent));
            }
        }
        None => p.text("Search", &label, field, Ink::Faint)?,
    }
    p.button(toggle, Target::ToggleSidebar, CONTROL_RADIUS);
    let ink = if p.hover == Some(Target::ToggleSidebar) { Ink::Text } else { Ink::Faint };
    // Revealed where it cannot dock, the button puts it away.
    p.icon(if collapsed && sidebar_fits(width) { icon::PIN } else { icon::COLLAPSE_LEFT }, toggle, ink)?;
    y += CONTROL + 8.0;

    // A phone on its side with the keyboard up leaves the menu too short
    // for more than the search: while searching there, the results follow
    // the box at once, and New note, Social and the icons below step aside.
    let tight = input.platform == Platform::Ios && searching.is_some() && panel.height() < 420.0;
    if !tight {
        // New note: the menu's first act, a soft button the width of the
        // search above it, its key at its end.
        let new_note = Rect::new(x0, y, x1, y + CONTROL);
        let hot = p.hover == Some(Target::NewNote);
        p.scene.fills.push(Fill::solid(new_note, CONTROL_RADIUS, Ink::Raised));
        p.button(new_note, Target::NewNote, CONTROL_RADIUS);
        // Basil's add-outline, drawn from its outline (one mark on every
        // platform, whatever its icon font).
        let ink = if hot { Ink::Text } else { Ink::Muted };
        let (cx, cy) = (x0 + 18.0, y + CONTROL / 2.0);
        let mark = Rect::new(cx - NEW_NOTE_MARK / 2.0, cy - NEW_NOTE_MARK / 2.0, cx + NEW_NOTE_MARK / 2.0, cy + NEW_NOTE_MARK / 2.0);
        p.scene.vectors.push(VectorMark { layer: p.scene.layer_now(), rect: mark, logo: Logo::NewNote, ink, alpha: 1.0 });
        let keys = new_note_keys(input.platform);
        let small_text = p.fonts.small.clone();
        let k = scene::single_line(p.eng, keys, &small_text, 80.0, CONTROL);
        let kw = k.measure().width;
        let shown_keys = new_note.width() > 200.0;
        if shown_keys {
            p.scene.texts.push(Text { x: x1 - 12.0 - kw, y, layout: k, ink: Ink::Faint, alpha: 1.0 });
        }
        let end = if shown_keys { x1 - 20.0 - kw } else { x1 - 8.0 };
        p.text("New note", &label, Rect::new(x0 + 32.0, y, end, y + CONTROL), Ink::Text)?;
        y += CONTROL + 10.0;

        // Social: a drawer of the platforms a note previews for. Not on
        // iPad or iPhone, where sharing is the system's.
        if input.platform != Platform::Ios {
            let fold = Rect::new(x0, y, x1, y + 22.0);
            p.button(fold, Target::SocialDrawer, 6.0);
            let hot = p.hover == Some(Target::SocialDrawer);
            let small = p.fonts.icon_small.clone();
            p.text(if input.social_drawer { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT }, &small, Rect::new(x0 + 6.0, y, x0 + 22.0, y + 22.0), Ink::Faint)?;
            let section = p.fonts.section.clone();
            p.text("Social", &section, Rect::new(x0 + 24.0, y, x1 - 8.0, y + 22.0), if hot { Ink::Muted } else { Ink::Faint })?;
            y += 24.0;
            // Open, the platforms: small and quiet, close together under the
            // label like what the drawer holds, as its label's ink until pointed at.
            if input.social_drawer {
                for (i, platform) in kolada_core::social::PLATFORMS.into_iter().enumerate() {
                    let left = x0 + SOCIAL_INDENT + i as f32 * SOCIAL_CELL;
                    let cell = Rect::new(left, y, left + SOCIAL_CELL - 2.0, y + SOCIAL_CELL - 2.0);
                    let target = Target::Social(platform);
                    let shown = input.social.is_some_and(|v| v.platform == platform);
                    let pointed = p.hover == Some(target);
                    if shown {
                        p.scene.fills.push(Fill::solid(cell, 7.0, Ink::Raised));
                    } else if pointed {
                        p.scene.fills.push(Fill::solid(cell, 7.0, Ink::Hover));
                    }
                    p.scene.hits.push((cell, target));
                    let ink = if shown { Ink::Text } else if pointed { Ink::Muted } else { Ink::Faint };
                    let (mx, my) = ((cell.left + cell.right) / 2.0, (cell.top + cell.bottom) / 2.0);
                    let half = SOCIAL_MARK / 2.0;
                    crate::social::mark(p.scene, platform, Rect::new(mx - half, my - half, mx + half, my + half), ink);
                }
                y += SOCIAL_CELL + 4.0;
            }
        }
    }

    y += 12.0;

    // At the bottom, icons only: the notes as a grid, the trash,
    // appearance, settings.
    let options = Rect::new(x0, panel.bottom - PAD - CONTROL, x1, panel.bottom - PAD);
    let icons = menu_icons(input);
    let row_w = icons.len() as f32 * ICON_PITCH - BTN_GAP;
    let start = ((options.left + options.right - row_w) / 2.0).round();
    for (k, (target, glyph)) in icons.into_iter().filter(|_| !tight).enumerate() {
        let left = start + k as f32 * ICON_PITCH;
        let r = Rect::new(left, options.top, left + ICON_TILE, options.bottom);
        // The grid, a press away; as quiet as the icons beside it.
        if target == Target::ViewToggle {
            crate::grid::view_toggle(p, r, false, Ink::Muted, 0.72);
            continue;
        }
        if (target == Target::Options && input.options_open) || (target == Target::Trash && input.trash.is_some()) {
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS, Ink::Selection));
        }
        p.button(r, target, CONTROL_RADIUS);
        let hot = p.hover == Some(target);
        p.icon(glyph, r, if hot { Ink::Text } else { Ink::Muted })?;
        // Quieter than the rest of the menu until reached for.
        if !hot && let Some(t) = p.scene.texts.last_mut() {
            t.alpha = 0.72;
        }
    }

    // An update downloaded and checked: a quiet pill just above the icons.
    // Where the menu is too short for it, it waits (the update goes in at
    // the next start all the same).
    let mut bottom = if tight { panel.bottom - PAD } else { options.top - 8.0 };
    // Beam's row just above the icons: what arrived, quiet when nothing
    // is new. The update card stacks above it.
    let mut stack = options.top;
    if let Some(beam) = input.quick.filter(|_| !tight) {
        let h = if beam.unseen > 0 { BEAM_ROW_NEW } else { BEAM_ROW };
        let row = Rect::new(x0, options.top - 8.0 - h, x1, options.top - 8.0);
        if row.top >= y + ROW {
            beam_row(p, row, &beam)?;
            stack = row.top;
            bottom = row.top - 8.0;
        }
    }
    if let Some(version) = input.update.filter(|_| !tight) {
        let pill = update_pill_rect(x0, x1, stack);
        if pill.top >= y + ROW {
            update_pill(p, pill, version)?;
            bottom = pill.top - 8.0;
        }
    }

    match (searching, input.trash) {
        (Some(s), _) => results(p, s, x0, x1, y, bottom)?,
        (None, Some(entries)) => trash(p, entries, x0, x1, y, bottom)?,
        (None, None) => tree(p, input, panel, x0, x1, y, bottom)?,
    }

    // Docked, its right edge widens or narrows it: a fine line along the
    // whole of it, where its corners stop rounding, says so. Revealed from
    // focus mode it is as it is.
    if !collapsed {
        let edge = Rect::new(panel.right - 3.0, panel.top + PANEL_RADIUS, panel.right + 5.0, panel.bottom - PANEL_RADIUS);
        p.scene.hits.push((edge, Target::SidebarEdge));
        let held = input.dragging == Some(Target::SidebarEdge);
        if held || p.hover == Some(Target::SidebarEdge) {
            let line = Rect::new(panel.right - 0.5, panel.top + PANEL_RADIUS, panel.right + 0.5, panel.bottom - PANEL_RADIUS);
            p.scene.fills.push(Fill::faded(line, 0.5, Ink::Accent, if held { 0.8 } else { 0.5 }));
        }
    }
    Ok(())
}

/// How tall the update pill is, DIPs.
const UPDATE_PILL: f32 = 46.0;

/// Beam's row at the side menu's foot, the height of the update card's
/// lines: quiet, or on the tint with what is new.
const BEAM_ROW: f32 = 44.0;
const BEAM_ROW_NEW: f32 = 44.0;
/// Its mark, centred where New note's is (18 in), and its text where New
/// note's label starts.
const BEAM_ROW_MARK: f32 = 18.0;

/// Beam's row: the beam, "Beam" over a second line, quiet when nothing is
/// new; on the teal tint with how many are new and the newest one's first
/// line when some are. A click opens the popup on its beams.
fn beam_row(p: &mut Painter, r: Rect, beam: &scene::BeamView) -> Result<()> {
    let target = Target::Quick(crate::quick::Hit::Show);
    let hot = p.hover == Some(target);
    let new = beam.unseen > 0;
    if new || hot {
        p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS, if new { Ink::Mark } else { Ink::Hover }));
    }
    p.scene.hits.push((r, target));
    let (cx, cy) = (r.left + 18.0, (r.top + r.bottom) / 2.0);
    let m = BEAM_ROW_MARK / 2.0;
    crate::quick::beam_mark(p, Rect::new(cx - m, cy - m, cx + m, cy + m), if new { Ink::Accent } else { Ink::Muted }, 1.0);
    let tx = r.left + 32.0;
    let label = p.fonts.label.clone();
    let small = p.fonts.small.clone();
    let title = if new { format!("Beam \u{00B7} {} new", beam.unseen) } else { "Beam".to_string() };
    let tf = Format { wrap: false, ellipsis: true, weight: if new { 600 } else { label.weight }, ..label.clone() };
    if beam.line.is_empty() {
        p.text(&title, &tf, Rect::new(tx, r.top, r.right - 12.0, r.bottom), Ink::Text)?;
        return Ok(());
    }
    // Two lines, 20 and 16 tall, centred together.
    let top = r.top + (r.height() - 36.0) / 2.0;
    p.text(&title, &tf, Rect::new(tx, top, r.right - 12.0, top + 20.0), Ink::Text)?;
    p.text(beam.line, &Format { wrap: false, ellipsis: true, ..small }, Rect::new(tx, top + 20.0, r.right - 12.0, top + 36.0), if new { Ink::Muted } else { Ink::Faint })?;
    Ok(())
}

/// The window's Beam button, on the 4-point grid: 40 across at rest, 48
/// grown; 24 from the window's edges.
const BEAM_REST: f32 = 40.0;
const BEAM_GROWN: f32 = 48.0;
const BEAM_EDGE: f32 = 24.0;

/// The window's Beam button, bottom right, over the note: small and faint
/// at rest (fainter while typing), grown and solid with a count when beams
/// arrive; the pointer over it, it opens into a capsule that says what it
/// is and its keys. Its menu above it is a Kolada menu (`menu_row`).
pub fn beam_button(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    let Some(beam) = &input.quick else { return Ok(()) };
    if input.compact || input.options_open || input.scrub.is_some() || input.model_card.is_some() || width < 300.0 || height < 200.0 {
        return Ok(());
    }
    let mut p = Painter { eng, fonts, hover: input.hover, scene };
    let target = Target::Quick(crate::quick::Hit::Fab);
    // While the grid shows, its bar has the button (here it would cover the
    // boxes): its menu hangs under that one.
    if input.grid.as_ref().is_some_and(|g| crate::grid::sheet(g.grid, width, height).height() >= 0.5) {
        if beam.menu.is_some()
            && let Some(cell) = p.scene.hits.iter().rev().find(|(_, t)| *t == target).map(|(r, _)| *r)
        {
            beam_menu(&mut p, beam, cell.right.min(width - 8.0), cell.bottom + 8.0, false, width)?;
        }
        return Ok(());
    }
    let hot = p.hover == Some(target);
    let new = beam.unseen > 0;
    let (right, bottom) = (width - BEAM_EDGE, height - BEAM_EDGE);
    let big = new || hot || beam.menu.is_some();
    let d = if big { BEAM_GROWN } else { BEAM_REST };
    let label = Format { weight: 600, wrap: false, ..p.fonts.label.clone() };
    let small = Format { wrap: false, ..p.fonts.small.clone() };
    // Over it, it says what it is: the mark, "Beam", its keys, on one line.
    let open = hot && beam.menu.is_none();
    let (word_w, keys_w) = if open {
        let ww = scene::single_line(p.eng, "Beam", &label, 200.0, d).measure().width;
        let kw = if beam.keys.is_empty() { 0.0 } else { scene::single_line(p.eng, beam.keys, &small, 200.0, d).measure().width };
        (ww, kw)
    } else {
        (0.0, 0.0)
    };
    let extra = if open { word_w + if keys_w > 0.0 { 8.0 + keys_w } else { 0.0 } + 16.0 } else { 0.0 };
    let r = Rect::new(right - d - extra, bottom - d, right, bottom);
    let alpha = if big { 1.0 } else if beam.typing { 0.16 } else { 0.38 };
    if big {
        p.scene.fills.push(Fill { alpha: 0.6, ..Fill::shadow(r.offset(0.0, 4.0), d / 2.0, 16.0, 1.0) });
    }
    p.scene.fills.push(Fill { alpha, ..Fill::solid(r, d / 2.0, Ink::Accent) });
    p.scene.hits.push((r, target));
    // A beam sent from here with no popup: its check beside the button.
    if let Some((scale, alpha)) = beam.check {
        crate::quick::sent_mark(&mut p, r.left - 8.0 - 16.0, (r.top + r.bottom) / 2.0, crate::quick::Sent::Check { scale, alpha }, None);
    }
    let g = if big { 24.0 } else { 20.0 };
    let (cx, cy) = (r.left + d / 2.0, r.top + d / 2.0);
    crate::quick::beam_mark(&mut p, Rect::new(cx - g / 2.0, cy - g / 2.0, cx + g / 2.0, cy + g / 2.0), Ink::OnAccent, if big { 1.0 } else { alpha.max(0.6) });
    if open {
        let tx = r.left + d - 4.0;
        p.text("Beam", &Format { center_y: true, ..label.clone() }, Rect::new(tx, r.top, tx + word_w + 2.0, r.bottom), Ink::OnAccent)?;
        if keys_w > 0.0 {
            let kx = tx + word_w + 8.0;
            let from = p.scene.texts.len();
            on_baseline(&mut p, beam.keys, &small, &Format { center_y: true, ..label }, Rect::new(kx, r.top, kx + keys_w + 2.0, r.bottom), Ink::OnAccent);
            p.scene.texts[from..].iter_mut().for_each(|t| t.alpha = 0.75);
        }
    }
    // What arrived: a white count on its corner.
    if new {
        let text = if beam.unseen > 9 { "9+".to_string() } else { beam.unseen.to_string() };
        let f = Format { size: 11.0, weight: 700, wrap: false, ..p.fonts.small.clone() };
        let w = (scene::single_line(p.eng, &text, &f, 40.0, 20.0).measure().width + 12.0).max(20.0);
        let badge = Rect::new(r.right - w + 4.0, r.top - 4.0, r.right + 4.0, r.top + 16.0);
        p.scene.fills.push(Fill::solid(badge, 10.0, Ink::OnAccent));
        p.text(&text, &Format { align: Align::Center, center_y: true, ..f }, badge, Ink::Accent)?;
    }
    // Its menu above it: this note, or a new beam.
    if beam.menu.is_some() {
        beam_menu(&mut p, beam, right, r.top - 8.0, true, width)?;
    }
    Ok(())
}

/// Beam's menu, its right edge at `right`: this note, or a new beam. Its
/// foot at `edge` over the round button (`above`), or its top at `edge`
/// under the grid bar's.
fn beam_menu(p: &mut Painter, beam: &scene::BeamView, right: f32, edge: f32, above: bool, width: f32) -> Result<()> {
    p.scene.overlay(None);
    let items: [(crate::quick::Hit, crate::vector::Logo, &str, &str); 2] = [(crate::quick::Hit::BeamNote, crate::vector::Logo::Beam, "Beam this note", ""), (crate::quick::Hit::NewBeam, crate::vector::Logo::NewNote, "New beam", beam.keys)];
    let lw = items.iter().map(|i| scene::single_line(p.eng, i.2, &p.fonts.label, 400.0, MENU_ITEM).measure().width).fold(0.0, f32::max);
    let kw = if beam.keys.is_empty() { 0.0 } else { scene::single_line(p.eng, beam.keys, &p.fonts.small, 200.0, MENU_ITEM).measure().width };
    let w = (6.0 + MENU_TEXT + lw + if kw > 0.0 { kw + 20.0 } else { 0.0 } + 18.0).max(MENU_WIDTH).min(width - 16.0);
    let h = 12.0 + items.len() as f32 * MENU_ITEM;
    let right = right.max(8.0 + w);
    let card = if above { Rect::new(right - w, edge - h, right, edge) } else { Rect::new(right - w, edge, right, edge + h) };
    p.panel(card, 1.0);
    p.scene.hits.push((card, Target::Quick(crate::quick::Hit::Popup)));
    for (k, (hit, logo, word, keys)) in items.iter().enumerate() {
        let row = Rect::new(card.left + 6.0, card.top + 6.0 + MENU_ITEM * k as f32, card.right - 6.0, card.top + 6.0 + MENU_ITEM * (k as f32 + 1.0));
        menu_row(p, row, Target::Quick(*hit), RowIcon::Mark(*logo), word, keys, kw, false)?;
    }
    Ok(())
}

/// Where the update pill goes: the side menu's width, just above its icons
/// or Beam's row (whose top is `icons`).
fn update_pill_rect(x0: f32, x1: f32, icons: f32) -> Rect {
    Rect::new(x0, icons - 8.0 - UPDATE_PILL, x1, icons - 8.0)
}

/// The update pill: a soft field with a small teal dot, "Update ready" over
/// the version, and a Restart button at its end. A click anywhere on it
/// restarts into the update.
fn update_pill(p: &mut Painter, r: Rect, version: &str) -> Result<()> {
    p.scene.fills.push(Fill::solid(r, 14.0, Ink::Raised));
    p.button(r, Target::Update, 14.0);
    let hot = p.hover == Some(Target::Update);
    let mid = (r.top + r.bottom) / 2.0;
    let dot = Rect::new(r.left + 13.0, mid - 3.0, r.left + 19.0, mid + 3.0);
    p.scene.fills.push(Fill::solid(dot, 3.0, Ink::Accent));
    // The button: raised, like a chosen option.
    let choice = p.fonts.choice.clone();
    let word = scene::single_line(p.eng, "Restart", &choice, 200.0, 30.0);
    let w = word.measure().width + 24.0;
    let button = Rect::new(r.right - 8.0 - w, mid - 15.0, r.right - 8.0, mid + 15.0);
    p.scene.fills.push(Fill::solid(button, 15.0, Ink::Button));
    p.text("Restart", &choice, button, Ink::Text)?;
    // What is ready, over its version; "Update" alone where the menu is narrow.
    let (label, small) = (p.fonts.label.clone(), p.fonts.small.clone());
    let text = Rect::new(dot.right + 9.0, r.top + 5.0, button.left - 8.0, r.bottom - 5.0);
    let full = scene::single_line(p.eng, "Update ready", &label, 400.0, 20.0).measure().width <= text.width();
    let ink = if hot { Ink::Text } else { Ink::Muted };
    p.text(if full { "Update ready" } else { "Update" }, &label, Rect::new(text.left, text.top, text.right, text.top + 19.0), ink)?;
    p.text(version, &small, Rect::new(text.left, text.top + 19.0, text.right, text.bottom), Ink::Faint)
}

/// The vault's file tree, from the first row scrolled to, with a faint
/// indicator of where it is when it does not all fit.
#[allow(clippy::too_many_arguments)]
fn tree(p: &mut Painter, input: &Input, panel: Rect, x0: f32, x1: f32, top: f32, bottom: f32) -> Result<()> {
    // A name longer than its row ends in "…", on its own side.
    let label = Format { wrap: false, ellipsis: true, ..p.fonts.label.clone() };
    // Favorites and Recent, then the vault's folders and notes, unlabelled.
    let top = sections(p, input, x0, x1, top, bottom)?;
    let fit = ((bottom - top) / ROW).floor().max(0.0) as usize;
    p.scene.tree_rows = fit;
    let rows = input.sidebar;
    let first = input.tree_top.min(rows.len().saturating_sub(fit));
    let paths = vault::row_paths(rows);
    let mut y = top;
    for (i, row) in rows.iter().enumerate().skip(first).take(fit) {
        let r = Rect::new(x0, y, x1, y + ROW);
        let active = row.kind == RowKind::Note && paths[i] == input.active_note;
        let pointed = matches!(p.hover, Some(Target::Row(j) | Target::RowMenu(j)) if j == i);
        let menu = pointed || input.menu_row == Some(i);
        if active {
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS - 2.0, Ink::Raised));
        } else if pointed || input.menu_row == Some(i) {
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS - 2.0, Ink::Hover));
        }
        p.scene.hits.push((r, Target::Row(i)));
        let indent = x0 + 2.0 + row.depth as f32 * INDENT;
        let small = p.fonts.icon_small.clone();
        // Folders: a chevron, then a folder (open while it is); notes: their
        // icon under the folders' so names line up.
        let (chevron, logo) = match row.kind {
            RowKind::Folder { expanded: true } => (Some(icon::CHEVRON_DOWN), Logo::FolderOpen),
            RowKind::Folder { expanded: false } => (Some(icon::CHEVRON_RIGHT), Logo::Folder),
            RowKind::Note => (None, Logo::Note),
        };
        if let Some(c) = chevron {
            p.text(c, &small, Rect::new(indent, y, indent + 20.0, y + ROW), Ink::Faint)?;
        }
        let mark = Rect::new(indent + 22.0, y + 8.0, indent + 36.0, y + 22.0);
        let mark_ink = if row.kind == RowKind::Note { Ink::Faint } else { Ink::Muted };
        p.scene.vectors.push(VectorMark { layer: p.scene.layer_now(), rect: mark, logo, ink: mark_ink, alpha: 1.0 });
        let glyph_box = Rect::new(indent, y, mark.right + 4.0, y + ROW);
        let ink = if active || pointed { Ink::Text } else { Ink::Muted };
        let name = if active { input.live_name.unwrap_or(&row.name) } else { &row.name };
        let mut right = if menu { x1 - 30.0 } else { x1 - 8.0 };
        // When the note under the pointer was edited, faint at the row's end.
        if let Some((_, date)) = input.row_date.filter(|&(j, _)| j == i && pointed) {
            let small = p.fonts.small.clone();
            let l = scene::layout(p.eng, date, &small, 120.0, ROW);
            let w = l.measure().width;
            p.scene.texts.push(Text { x: right - w, y, layout: l, ink: Ink::Faint, alpha: 1.0 });
            right -= w + 8.0;
        }
        p.text(name, &label, Rect::new(glyph_box.right + 2.0, y, right, y + ROW), ink)?;
        // The row's actions, behind three dots at its end.
        if menu {
            let b = Rect::new(x1 - 28.0, y + 3.0, x1 - 4.0, y + ROW - 3.0);
            p.button(b, Target::RowMenu(i), 6.0);
            let ink = if p.hover == Some(Target::RowMenu(i)) || input.menu_row == Some(i) { Ink::Text } else { Ink::Muted };
            dots(p.scene, b, ink);
        }
        y += ROW;
    }
    // A scroll bar along the panel's edge: there while the pointer is in
    // the menu, thick under it, and draggable.
    if rows.len() > fit && fit > 0 {
        let track = Rect::new(panel.right - 13.0, top, panel.right - 1.0, top + fit as f32 * ROW);
        let at = first as f32 / (rows.len() - fit) as f32;
        let size = fit as f32 / rows.len() as f32;
        let hot = p.hover == Some(Target::TreeScrollbar) || input.dragging == Some(Target::TreeScrollbar);
        if hot || p.hover.and_then(Target::panel) == Some(scene::Panel::Sidebar) {
            scroll_thumb(p.scene, track, at, size, hot, 1.0);
        }
        p.scene.hits.push((track, Target::TreeScrollbar));
        p.scene.tree_track = Some(track);
    }
    Ok(())
}

/// Notes shown under Recent at most, besides the open one.
const RECENT_SHOWN: usize = 5;
/// Rows of the tree the sections above it always leave room for.
const TREE_KEPT: f32 = 4.0;
/// A section heading's height, and the space after a section.
const HEADING: f32 = 26.0;
const KEPT_GAP: f32 = 8.0;

/// What a row of a section above the tree is.
enum Kept<'a> {
    Note(&'a str),
    Folder(&'a str),
}

/// Favorites, then Recent, above the vault's tree: each a heading (a
/// chevron that folds it, its mark, its name) and its rows, as many as
/// leave the tree room (Recent gives way first). Where the tree starts.
fn sections(p: &mut Painter, input: &Input, x0: f32, x1: f32, top: f32, bottom: f32) -> Result<f32> {
    let active = format!("{}.md", input.active_note);
    let favorites: Vec<(usize, Kept)> = input
        .favorites
        .iter()
        .enumerate()
        .map(|(i, f)| (i, f.strip_suffix('/').map_or(Kept::Note(f), Kept::Folder)))
        .collect();
    // Recent only in a vault of more than a few dozen notes, and not once hidden.
    let recent: Vec<(usize, Kept)> = if kolada_core::session::shows_recent(input.folded) {
        input.recent.iter().enumerate().filter(|(_, n)| **n != active).take(RECENT_SHOWN).map(|(i, n)| (i, Kept::Note(n))).collect()
    } else {
        Vec::new()
    };
    let fav_open = input.folded & kolada_core::session::FOLD_FAVORITES == 0;
    let recent_open = input.folded & kolada_core::session::FOLD_RECENT == 0;
    let headings = [!favorites.is_empty(), !recent.is_empty()].iter().filter(|h| **h).count() as f32;
    let rows = ((bottom - top - headings * (HEADING + KEPT_GAP)) / ROW - TREE_KEPT).floor().max(0.0) as usize;
    let fav_rows = if fav_open { favorites.len().min(rows) } else { 0 };
    let recent_rows = if recent_open { recent.len().min(rows - fav_rows) } else { 0 };
    let mut y = top;
    // A heading only where it fits above the tree's room.
    let fits = |y: f32| y + HEADING + KEPT_GAP <= bottom - TREE_KEPT * ROW;
    if !favorites.is_empty() && fits(y) {
        y = section(p, input, "Favorites", Target::FoldFavorites, fav_open, &[Logo::Star], Ink::Muted, &favorites[..fav_rows], Target::Favorite, x0, x1, y)?;
    }
    if !recent.is_empty() && fits(y) {
        y = section(p, input, "Recent", Target::FoldRecent, recent_open, &[Logo::Recent], Ink::Muted, &recent[..recent_rows], Target::Recent, x0, x1, y)?;
    }
    Ok(y)
}

/// One section: its heading, then `rows` (each opening its note or folder).
#[allow(clippy::too_many_arguments)]
fn section(
    p: &mut Painter,
    input: &Input,
    name: &str,
    fold: Target,
    open: bool,
    mark: &[Logo],
    mark_ink: Ink,
    rows: &[(usize, Kept)],
    target: fn(usize) -> Target,
    x0: f32,
    x1: f32,
    top: f32,
) -> Result<f32> {
    let section = p.fonts.section.clone();
    let label = p.fonts.label.clone();
    let small = p.fonts.icon_small.clone();
    let head = Rect::new(x0, top, x1, top + HEADING);
    p.button(head, fold, CONTROL_RADIUS - 2.0);
    let hot = p.hover == Some(fold);
    p.text(if open { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT }, &small, Rect::new(x0 + 2.0, top, x0 + 22.0, top + HEADING), Ink::Faint)?;
    // The section's mark (a second part, if any, drawn over it in white).
    let m = Rect::new(x0 + 24.0, top + 6.0, x0 + 38.0, top + 20.0);
    let layer = p.scene.layer_now();
    for (k, logo) in mark.iter().enumerate() {
        let ink = if k == 0 { mark_ink } else { Ink::OnAccent };
        p.scene.vectors.push(VectorMark { layer, rect: m, logo: *logo, ink, alpha: 1.0 });
    }
    p.text(name, &section, Rect::new(m.right + 8.0, top + 2.0, x1 - 8.0, top + HEADING - 2.0), if hot { Ink::Muted } else { Ink::Faint })?;
    let mut y = top + HEADING;
    let active = format!("{}.md", input.active_note);
    for (i, kept) in rows {
        let r = Rect::new(x0, y, x1, y + ROW);
        let t = target(*i);
        let pointed = p.hover == Some(t);
        let (path, folder) = match kept {
            Kept::Note(n) => (*n, false),
            Kept::Folder(f) => (*f, true),
        };
        if !folder && path == active {
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS - 2.0, Ink::Raised));
        } else if pointed {
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS - 2.0, Ink::Hover));
        }
        p.scene.hits.push((r, t));
        let glyph = Rect::new(x0 + 26.0, y + 8.0, x0 + 40.0, y + 22.0);
        let logo = if folder { Logo::Folder } else { Logo::Note };
        p.scene.vectors.push(VectorMark { layer: p.scene.layer_now(), rect: glyph, logo, ink: Ink::Faint, alpha: 1.0 });
        let file = path.rsplit('/').next().unwrap_or(path);
        let name = file.strip_suffix(".md").unwrap_or(file);
        p.text(name, &label, Rect::new(x0 + 48.0, y, x1 - 8.0, y + ROW), if pointed { Ink::Text } else { Ink::Muted })?;
        y += ROW;
    }
    Ok(y + KEPT_GAP)
}

/// Search results in place of the tree: names, and for matches in the
/// text, the words around them.
fn results(p: &mut Painter, s: &SearchView, x0: f32, x1: f32, top: f32, bottom: f32) -> Result<()> {
    if let Some(commands) = s.commands {
        return command_rows(p, s, commands, x0, x1, top, bottom);
    }
    let section = p.fonts.section.clone();
    let label = p.fonts.label.clone();
    let small = p.fonts.small.clone();
    let status = match s.hits.len() {
        _ if s.query.trim().is_empty() => "Find a note, or type > for commands".to_string(),
        0 => "Nothing found".to_string(),
        1 => "1 note".to_string(),
        n => format!("{n} notes"),
    };
    p.text(&status, &section, Rect::new(x0 + 8.0, top, x1 - 8.0, top + 20.0), Ink::Faint)?;
    let mut y = top + 24.0;
    let first = s.first.min(s.selected);
    let mut shown = 0;
    for (i, hit) in s.hits.iter().enumerate().skip(first) {
        let h = if hit.snippet.is_some() { 46.0 } else { ROW };
        if y + h > bottom {
            break;
        }
        shown += 1;
        let r = Rect::new(x0, y, x1, y + h);
        if i == s.selected {
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS - 2.0, Ink::Selection));
            p.scene.hits.push((r, Target::Result(i)));
        } else {
            p.button(r, Target::Result(i), CONTROL_RADIUS - 2.0);
        }
        let icons = p.fonts.icon_small.clone();
        p.text(icon::PAGE, &icons, Rect::new(x0 + 2.0, y, x0 + 26.0, y + ROW), Ink::Faint)?;
        p.text(&hit.name, &label, Rect::new(x0 + 28.0, y, x1 - 8.0, y + ROW), Ink::Text)?;
        if let Some(snippet) = &hit.snippet {
            p.text(snippet, &small, Rect::new(x0 + 28.0, y + 24.0, x1 - 8.0, y + 42.0), Ink::Faint)?;
        }
        y += h;
    }
    p.scene.result_rows = shown;
    Ok(())
}

/// The command palette's rows: what each does, and the keys that do it.
#[allow(clippy::too_many_arguments)]
fn command_rows(p: &mut Painter, s: &SearchView, commands: &[scene::MenuItem], x0: f32, x1: f32, top: f32, bottom: f32) -> Result<()> {
    let section = p.fonts.section.clone();
    let label = p.fonts.label.clone();
    let small = p.fonts.small.clone();
    let status = if commands.is_empty() { "No command matches" } else { "Commands" };
    p.text(status, &section, Rect::new(x0 + 8.0, top, x1 - 8.0, top + 20.0), Ink::Faint)?;
    let top = top + 24.0;
    let fit = ((bottom - top) / ROW).floor().max(1.0) as usize;
    p.scene.result_rows = fit;
    // The selected row stays in view.
    let first = s.first.min(s.selected).max((s.selected + 1).saturating_sub(fit));
    let mut y = top;
    for (i, item) in commands.iter().enumerate().skip(first).take(fit) {
        let r = Rect::new(x0, y, x1, y + ROW);
        if i == s.selected {
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS - 2.0, Ink::Selection));
            p.scene.hits.push((r, Target::Result(i)));
        } else {
            p.button(r, Target::Result(i), CONTROL_RADIUS - 2.0);
        }
        let (icon_ink, ink) = if item.danger { (Ink::Danger, Ink::Danger) } else { (Ink::Muted, Ink::Text) };
        if !item.icon.is_empty() {
            p.icon(item.icon, Rect::new(x0 + 2.0, y, x0 + 28.0, y + ROW), icon_ink)?;
        }
        let mut right = x1 - 10.0;
        if !item.keys.is_empty() {
            let l = scene::layout(p.eng, item.keys, &small, 120.0, ROW);
            let w = l.measure().width;
            p.scene.texts.push(Text { x: right - w, y, layout: l, ink: Ink::Faint, alpha: 1.0 });
            right -= w + 10.0;
        }
        let l = scene::single_line(p.eng, &item.label, &label, right - x0 - 32.0, ROW);
        p.scene.texts.push(Text { x: x0 + 32.0, y, layout: l, ink, alpha: 1.0 });
        y += ROW;
    }
    Ok(())
}

/// Three dots stacked in `r`: a menu's button.
fn dots(scene: &mut Scene, r: Rect, ink: Ink) {
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    for k in [-1.0, 0.0, 1.0] {
        let y = cy + k * 4.5;
        scene.fills.push(Fill::solid(Rect::new(cx - 1.3, y - 1.3, cx + 1.3, y + 1.3), 1.3, ink));
    }
}

/// The trash in place of the tree: deleted notes, newest first, each with a
/// button that brings it back; the way back to the tree, and emptying it.
pub(crate) fn trash(p: &mut Painter, entries: &[TrashEntry], x0: f32, x1: f32, top: f32, bottom: f32) -> Result<()> {
    // No room above the menu's icons (a phone on its side with the keyboard
    // up): nothing is drawn over them.
    if top + 22.0 > bottom {
        return Ok(());
    }
    let section = p.fonts.section.clone();
    let label = p.fonts.label.clone();
    let small = p.fonts.small.clone();
    let back = Rect::new(x0, top - 4.0, x0 + 28.0, top + 22.0);
    p.button(back, Target::TrashBack, 6.0);
    let icons = p.fonts.icon_small.clone();
    let ink = if p.hover == Some(Target::TrashBack) { Ink::Text } else { Ink::Muted };
    p.text(icon::BACK, &icons, back, ink)?;
    let status = match entries.len() {
        0 => "Trash".to_string(),
        1 => "Trash \u{00B7} 1 note".to_string(),
        n => format!("Trash \u{00B7} {n} notes"),
    };
    p.text(&status, &section, Rect::new(back.right + 4.0, top, x1 - 60.0, top + 20.0), Ink::Faint)?;
    if !entries.is_empty() {
        let empty = Rect::new(x1 - 56.0, top - 4.0, x1, top + 22.0);
        p.button(empty, Target::TrashEmpty, 6.0);
        let choice = p.fonts.choice.clone();
        let ink = if p.hover == Some(Target::TrashEmpty) { Ink::Danger } else { Ink::Muted };
        p.text("Empty", &choice, empty, ink)?;
    }
    let mut y = top + 26.0;
    if entries.is_empty() {
        let peek = p.fonts.peek.clone();
        let l = scene::layout(p.eng, "Deleted notes wait here until you bring them back.", &peek, x1 - x0 - 16.0, 200.0);
        p.scene.texts.push(Text { x: x0 + 8.0, y: y + 6.0, layout: l, ink: Ink::Faint, alpha: 1.0 });
        return Ok(());
    }
    for (i, entry) in entries.iter().enumerate() {
        let h = 46.0;
        if y + h > bottom {
            break;
        }
        let r = Rect::new(x0, y, x1, y + h);
        let pointed = matches!(p.hover, Some(Target::TrashRow(j) | Target::TrashRestore(j)) if j == i);
        if pointed {
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS - 2.0, Ink::Hover));
        }
        p.scene.hits.push((r, Target::TrashRow(i)));
        p.text(icon::PAGE, &icons, Rect::new(x0 + 2.0, y, x0 + 26.0, y + ROW), Ink::Faint)?;
        let right = if pointed { x1 - 34.0 } else { x1 - 8.0 };
        p.text(&entry.name, &label, Rect::new(x0 + 28.0, y, right, y + ROW), if pointed { Ink::Text } else { Ink::Muted })?;
        // When it went lines up with its name (right-aligned under a right
        // to left one, still reading in its own direction).
        let at = if scene::is_rtl(&entry.name) { Align::End } else { Align::Start };
        let when = scene::layout(p.eng, &entry.when, &Format { align: at, ..small }, right - x0 - 28.0, 18.0);
        p.scene.texts.push(Text { x: x0 + 28.0, y: y + 24.0, layout: when, ink: Ink::Faint, alpha: 1.0 });
        if pointed {
            let b = Rect::new(x1 - 32.0, y + 7.0, x1 - 4.0, y + h - 7.0);
            p.button(b, Target::TrashRestore(i), 6.0);
            let ink = if p.hover == Some(Target::TrashRestore(i)) { Ink::Accent } else { Ink::Muted };
            p.icon(icon::RESTORE_NOTE, b, ink)?;
        }
        y += h;
    }
    Ok(())
}

/// An option's choices: label and value.
type Choices = &'static [(&'static str, u8)];

/// A titled group of options.
type Section = (&'static str, &'static [Row]);

/// A line of the options.
enum Row {
    /// A setting and its choices.
    Choice(&'static str, Setting, Choices),
    /// A setting of many steps (the text size): smaller, where it is, larger.
    Stepper(&'static str, Setting),
    /// Buttons that do something (import, export, the vault's folder).
    Buttons(&'static str, &'static [(&'static str, Target)]),
    /// Where the vault is.
    Vault,
    /// A row only one platform has; the others hide it. What has no
    /// counterpart there, or is not built there yet.
    On(Platform, &'static Row),
    /// A row only where the app updates itself (`Input::updater`): not in
    /// a store's build, which the store updates.
    Updater(&'static Row),
    /// A row every platform has but these.
    NotOn(&'static [Platform], &'static Row),
    /// The paired devices and pairing (`quick::devices_panel`), where
    /// pairing runs (`Input::devices`); as tall as it needs.
    Devices,
    /// Checking for updates now: one button that says how far it got
    /// (`Input::update_check`).
    CheckNow,
    /// A feature turned on or off (`scene::switch_on`).
    Switch(&'static str, Setting),
    /// A section's own switch, on its heading's line: the whole section is
    /// the feature's (Sync and Beam).
    Head(Setting),
    /// The note's font: the shell's sans, serif and monospace, each named
    /// in its own face, and Other, the machine's own (its finder opens
    /// under it, `Input::font_finder`).
    Font,
    /// A row shown while a switch is on: it folds out from under the
    /// switch as it turns on, and away as it turns off (`Input::folding`).
    IfOn(Setting, &'static Row),
}

/// A setting as a shell's own settings page shows it (the iPhone's): its
/// section, its name, and its choices (words and values); a stepper's are
/// its sizes, smallest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Offered {
    pub section: &'static str,
    pub label: &'static str,
    pub setting: Setting,
    pub choices: Vec<(String, u8)>,
    pub stepper: bool,
}

/// The settings `platform` offers, in the order its card shows them; its
/// buttons and the vault's folder are the shell's own to show.
pub fn offered_settings(platform: Platform, updater: bool) -> Vec<Offered> {
    fn here(r: &'static Row, platform: Platform, updater: bool) -> Option<&'static Row> {
        match r {
            Row::On(p, r) => (*p == platform).then(|| here(r, platform, updater)).flatten(),
            Row::NotOn(ps, r) => (!ps.contains(&platform)).then(|| here(r, platform, updater)).flatten(),
            Row::Updater(r) => updater.then(|| here(r, platform, updater)).flatten(),
            Row::IfOn(_, r) => here(r, platform, updater),
            r => Some(r),
        }
    }
    let switch = |section, label, setting: Setting| {
        let (on, off) = if scene::switch_on(setting, 0) { (0, 1) } else { (scene::switched(setting, 0, None), 0) };
        Offered { section, label, setting, choices: vec![("On".to_string(), on), ("Off".to_string(), off)], stepper: false }
    };
    use kolada_core::session::{DEFAULT_TEXT_SIZE, TEXT_SIZES};
    let mut out = Vec::new();
    for (section, rows) in LEFT.iter().chain(RIGHT) {
        for row in rows.iter().filter_map(|r| here(r, platform, updater)) {
            match row {
                Row::Choice(label, setting, choices) => out.push(Offered {
                    section,
                    label,
                    setting: *setting,
                    choices: choices.iter().map(|(w, v)| (w.to_string(), *v)).collect(),
                    stepper: false,
                }),
                Row::Stepper(label, setting) => out.push(Offered {
                    section,
                    label,
                    setting: *setting,
                    choices: TEXT_SIZES.iter().map(|&s| (s.to_string(), if s == DEFAULT_TEXT_SIZE { 0 } else { s })).collect(),
                    stepper: true,
                }),
                Row::Font => out.push(Offered {
                    section,
                    label: "Font",
                    setting: Setting::Font,
                    choices: [("Sans", 0), ("Serif", 1), ("Mono", 2)].iter().map(|(w, v)| (w.to_string(), *v)).collect(),
                    stepper: false,
                }),
                Row::Switch(label, setting) => out.push(switch(section, label, *setting)),
                Row::Head(setting) => out.push(switch(section, "Sync and Beam", *setting)),
                _ => {}
            }
        }
    }
    out
}

/// What a setting's row is called where `platform` shows it ("Theme"), for
/// a screen reader naming its choices.
pub fn setting_title(setting: Setting, platform: Platform) -> Option<&'static str> {
    fn find(row: &Row, setting: Setting, platform: Platform) -> Option<&'static str> {
        match row {
            Row::Choice(title, s, _) | Row::Stepper(title, s) | Row::Switch(title, s) if *s == setting => Some(*title),
            Row::Font if setting == Setting::Font => Some("Font"),
            Row::Head(Setting::Beam) if setting == Setting::Beam => Some("Sync and Beam"),
            Row::On(p, r) if *p == platform => find(r, setting, platform),
            Row::IfOn(_, r) => find(r, setting, platform),
            Row::NotOn(ps, r) if !ps.contains(&platform) => find(r, setting, platform),
            Row::Updater(r) => find(r, setting, platform),
            _ => None,
        }
    }
    LEFT.iter().chain(RIGHT).flat_map(|(_, rows)| rows.iter()).find_map(|r| find(r, setting, platform))
}

/// The options, in two columns of sections.
const LEFT: &[Section] = &[
    (
        "Appearance",
        &[
            Row::Choice("Theme", Setting::Theme, &[("System", 0), ("Light", 1), ("Dark", 2)]),
            Row::Font,
            // iOS sizes the note as it sizes every app's text: by the user's
            // text size (Dynamic Type), Body at its own size, no option here.
            Row::NotOn(&[Platform::Ios], &Row::Stepper("Text size", Setting::TextSize)),
            Row::Choice("Line width", Setting::LineWidth, &[("Narrow", 1), ("Default", 0), ("Wide", 2)]),
            // iOS follows the system's Reduce Motion, its one control for it.
            Row::NotOn(&[Platform::Ios], &Row::Switch("Motion", Setting::Motion)),
        ],
    ),
    (
        "Writing",
        &[
            // iOS starts no helper processes (the model waits to run in the
            // app itself): not offered rather than offered and idle. Its
            // size shows under it while it is on.
            Row::NotOn(&[Platform::Ios], &Row::Switch("Writing model", Setting::Assist)),
            Row::NotOn(&[Platform::Ios], &Row::IfOn(Setting::Assist, &Row::Choice("Model", Setting::Assist, &[("Light", 0), ("Full", 1)]))),
            Row::Switch("Spell check", Setting::Spell),
            Row::Choice("Related notes", Setting::Related, &[("Show", 0), ("Hide", 1)]),
            Row::Choice("Show Markdown", Setting::ShowMarks, &[("Off", 0), ("On", 1)]),
            // iOS has no notifications of Kolada's yet.
            Row::NotOn(&[Platform::Ios], &Row::Choice("Reminders", Setting::Remind, &[("On", 0), ("Off", 1)])),
        ],
    ),
    ("Updates", &[Row::Updater(&Row::Choice("Check for updates", Setting::Updates, &[("On", 0), ("Off", 1)])), Row::Updater(&Row::CheckNow)]),
];

const RIGHT: &[Section] = &[
    (
        "Opening and closing",
        &[
            Row::Choice("At launch", Setting::Launch, &[("Smart", 0), ("Last note", 1), ("New page", 2)]),
            // iOS sizes its windows itself.
            Row::NotOn(&[Platform::Ios], &Row::Choice("Window", Setting::Window, &[("Full", 0), ("Minimal", 1)])),
            Row::On(Platform::Windows, &Row::Choice("Tray icon", Setting::Tray, &[("Off", 1), ("On", 0), ("Stay running", 2)])),
            Row::On(Platform::Mac, &Row::Choice("Menu bar icon", Setting::Tray, &[("Off", 1), ("On", 0), ("Stay running", 2)])),
            Row::On(Platform::Windows, &Row::Choice("Quick note", Setting::Hotkey, &[("Win+Alt+N", 0), ("Off", 1)])),
            Row::On(Platform::Mac, &Row::Choice("Quick note", Setting::Hotkey, &[("\u{2303}\u{2325}N", 0), ("Off", 1)])),
        ],
    ),
    (
        "Sync and Beam",
        &[
            // One switch for the section: off, nothing of either runs and
            // the devices fold away with the keys.
            // iOS has Beam as a tab of its own, its engine started with
            // the app: no switch here that it would not follow.
            Row::NotOn(&[Platform::Ios], &Row::Head(Setting::Beam)),
            Row::IfOn(Setting::Beam, &Row::Devices),
            Row::IfOn(Setting::Beam, &Row::On(Platform::Windows, &Row::Choice("Keys", Setting::QuickKeys, &[("Alt+Z", 0), ("Win+Shift+Z", 1), ("Off", 2)]))),
            Row::IfOn(Setting::Beam, &Row::On(Platform::Mac, &Row::Choice("Keys", Setting::QuickKeys, &[("\u{2325}Z", 0), ("\u{2303}\u{2325}\u{21E7}Z", 1), ("Off", 2)]))),
            Row::IfOn(Setting::Beam, &Row::On(Platform::Linux, &Row::Choice("Keys", Setting::QuickKeys, &[("Alt+Z", 0), ("Off", 2)]))),
            Row::IfOn(Setting::Beam, &Row::NotOn(&[Platform::Ios], &Row::Choice("The keys", Setting::QuickBeam, &[("Show it first", 0), ("Beam at once", 1)]))),
        ],
    ),
    (
        "Vault",
        &[
            Row::Vault,
            Row::NotOn(&[Platform::Ios], &Row::Buttons("", &[("Open another\u{2026}", Target::VaultOpen), ("Show in folder", Target::VaultShow)])),
            // iOS has no folder window to show it in: the Files app's picker chooses another.
            Row::On(Platform::Ios, &Row::Buttons("", &[("Open another\u{2026}", Target::VaultOpen)])),
        ],
    ),
    (
        "Notes in and out",
        &[
            Row::Buttons(
                "Import",
                &[
                    ("Files\u{2026}", Target::Transfer(Transfer::Files)),
                    ("Folder\u{2026}", Target::Transfer(Transfer::Folder)),
                    ("Clipboard", Target::Transfer(Transfer::Clipboard)),
                ],
            ),
            Row::Buttons(
                "Export note",
                &[
                    ("Markdown", Target::Transfer(Transfer::Markdown)),
                    ("Web page", Target::Transfer(Transfer::Page)),
                    ("PDF", Target::Transfer(Transfer::Pdf)),
                ],
            ),
        ],
    ),
];

const ROW_H: f32 = 40.0;
const SECTION_H: f32 = 30.0;
const SECTION_GAP: f32 = 14.0;
// Room for "Show Markdown" in DejaVu Sans, Linux's widest fallback (106),
// and "Check for updates" in the Mac's Helvetica Neue (111).
const LABEL_W: f32 = 122.0;

/// A row shown, and the switch it folds under, if any.
type Shown = (&'static Row, Option<Setting>);

/// The rows of a section this platform has, and the sections left with any.
fn shown(sections: &'static [Section], input: &Input) -> Vec<(&'static str, Vec<Shown>)> {
    shown_as(sections, input, false)
}

/// The same, with `all_open`: every switch's rows as if it were on (what
/// the card is sized by, so it stays still as they fold).
fn shown_as(sections: &'static [Section], input: &Input, all_open: bool) -> Vec<(&'static str, Vec<Shown>)> {
    sections
        .iter()
        .map(|(name, rows)| (*name, rows.iter().filter_map(|r| offered(r, input, all_open)).collect::<Vec<Shown>>()))
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

/// The row this platform shows for `row`, if any (a row only some have
/// is resolved to the row itself), and the switch it folds under. A row
/// under a switch shows while it is open at all (`fold_open`).
fn offered(row: &'static Row, input: &Input, all_open: bool) -> Option<Shown> {
    match row {
        Row::On(only, r) => (*only == input.platform).then_some(*r).and_then(|r| offered(r, input, all_open)),
        Row::Updater(r) => input.updater.then_some(*r).and_then(|r| offered(r, input, all_open)),
        Row::NotOn(not, r) => (!not.contains(&input.platform)).then_some(*r).and_then(|r| offered(r, input, all_open)),
        Row::IfOn(s, r) => (all_open || fold_open(input, *s) > 0.0).then_some(*r).and_then(|r| offered(r, input, all_open)).map(|(r, _)| (r, (!all_open).then_some(*s))),
        Row::Devices => input.devices.is_some().then_some((row, None)),
        row => Some((row, None)),
    }
}

/// How far the rows under a switch are open: 1 on, 0 off, between while
/// they fold (`Input::folding`, on a spring that lands a touch past).
fn fold_open(input: &Input, setting: Setting) -> f32 {
    match input.folding {
        Some((s, t)) if s == setting => t.max(0.0),
        _ => f32::from(u8::from(scene::switch_on(setting, input.options[setting.index()]))),
    }
}

/// How long a switch's rows take to fold out or away, seconds.
pub const FOLD_SECS: f32 = 0.7;

/// How far a switch's rows are open `since` seconds after it was pressed
/// (`opening`: turned on), on a spring that passes its mark a little and
/// settles; None once it has (`Input::folding`).
pub fn folding(since: f32, opening: bool) -> Option<f32> {
    if since >= FOLD_SECS || since < 0.0 {
        return None;
    }
    let t = since / FOLD_SECS;
    let open = 1.0 - (-5.0 * t).exp() * (6.0 * t).cos();
    Some(if opening { open } else { 1.0 - open })
}

fn column_height(sections: &'static [Section], input: &Input) -> f32 {
    column_height_as(sections, input, false)
}

/// How tall a column is (`all_open`: as if every switch were on).
fn column_height_as(sections: &'static [Section], input: &Input, all_open: bool) -> f32 {
    shown_as(sections, input, all_open).iter().map(|(_, rows)| SECTION_H + SECTION_GAP + rows.iter().map(|r| shown_height(r, input)).sum::<f32>()).sum()
}

/// How tall a row is as shown: folding, as far as it is open.
fn shown_height((row, under): &Shown, input: &Input) -> f32 {
    row_height(row, input) * under.map_or(1.0, |s| fold_open(input, s).min(1.0))
}

/// How tall an option's row is: one line, the devices panel, or none (a
/// section's own switch, on its heading).
fn row_height(row: &Row, input: &Input) -> f32 {
    match (row, &input.devices) {
        (Row::Devices, Some(v)) => crate::quick::devices_height(v),
        (Row::Font, _) => ROW_H + input.font_finder.map_or(0.0, |f| finder_height(&f)),
        (Row::Head(_), _) => 0.0,
        _ => ROW_H,
    }
}

/// Whether a press on `t` is on Settings (it stays open; elsewhere, it
/// closes): its panel and options, and its Devices section's controls.
pub fn keeps_settings(t: Option<Target>) -> bool {
    use crate::quick::Hit as Q;
    matches!(
        t,
        Some(
            Target::Options
                | Target::OptionsPanel
                | Target::Choice(..)
                | Target::Transfer(_)
                | Target::OptionsClose
                | Target::VaultOpen
                | Target::VaultShow
                | Target::CheckUpdates
                | Target::Switch(_)
                | Target::FontOther
                | Target::FontField
                | Target::FontName(_)
                | Target::Quick(Q::ShowCode | Q::EnterCode | Q::CodeField | Q::Pair | Q::Cancel | Q::Allow | Q::Deny | Q::Remove(_) | Q::Rename | Q::NameField | Q::NameSave | Q::NameCancel)
        )
    )
}

/// How far down the options page Settings > Devices starts, for a shell
/// that opens Settings there (the popup's devices chip, its first use).
pub fn devices_scroll(input: &Input, width: f32) -> f32 {
    let mut y = if width >= 820.0 { 0.0 } else { column_height(LEFT, input) };
    for (name, rows) in shown(RIGHT, input) {
        if name == "Sync and Beam" {
            return y;
        }
        y += SECTION_H + SECTION_GAP + rows.iter().map(|r| shown_height(r, input)).sum::<f32>();
    }
    0.0
}

/// A row of buttons (or, with `setting`, a choice of one) in `bar`.
pub(crate) fn segments(p: &mut Painter, bar: Rect, items: &[(&str, Target)], chosen: Option<Target>) -> Result<()> {
    let choice = p.fonts.choice.clone();
    let choosing = chosen.is_some();
    if choosing {
        p.scene.fills.push(Fill::solid(bar, 8.0, Ink::Field));
    }
    let w = bar.width() / items.len() as f32;
    for (k, &(label, target)) in items.iter().enumerate() {
        let r = Rect::new(bar.left + k as f32 * w, bar.top, bar.left + (k + 1) as f32 * w, bar.bottom).inset(2.0);
        let on = chosen == Some(target);
        let hover = p.hover == Some(target);
        if on {
            p.scene.fills.push(Fill::shadow(r.offset(0.0, 1.0), 6.0, 4.0, 0.8));
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Raised));
        } else if !choosing {
            // A row of buttons: each the button's fill, its words in full ink.
            p.scene.fills.push(Fill::solid(r, 6.0, if hover { Ink::Pressed } else { Ink::Button }));
        } else if hover {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Hover));
        }
        p.scene.hits.push((r, target));
        p.text(label, &choice, r, if on || hover || !choosing { Ink::Text } else { Ink::Muted })?;
    }
    Ok(())
}

/// Settings > Updates' "Check now" in `bar`: a button while it can be
/// pressed, else where the check has got to, quietly, in its place.
fn check_now(p: &mut Painter, input: &Input, bar: Rect) -> Result<()> {
    let (label, live) = input.update_check.label(input.update);
    let r = bar.inset(2.0);
    if !live {
        p.scene.fills.push(Fill::solid(r, 6.0, Ink::Field));
        let choice = p.fonts.choice;
        return p.text(&label, &choice, r, Ink::Muted);
    }
    segments(p, bar, &[(label.as_str(), Target::CheckUpdates)], None)
}

/// The text size in `bar`, as a choice row reads: the size raised in the
/// middle, as a chosen segment is, smaller and larger either side of it;
/// each dimmed, and still, past the last size.
fn stepper(p: &mut Painter, input: &Input, bar: Rect, setting: Setting) -> Result<()> {
    let value = input.options[setting.index()];
    p.scene.fills.push(Fill::solid(bar, 8.0, Ink::Field));
    let w = bar.width() / 3.0;
    let cell = |k: f32| Rect::new(bar.left + k * w, bar.top, bar.left + (k + 1.0) * w, bar.bottom).inset(2.0);
    let icons = p.fonts.icon_small;
    let icons = Format { size: 13.0, ..icons };
    for (k, up, glyph) in [(0.0, false, icon::SMALLER), (2.0, true, icon::LARGER)] {
        let r = cell(k);
        let ink = match scene::step_text_size(value, up) {
            Some(next) => {
                let target = Target::Choice(setting, next);
                p.scene.hits.push((r, target));
                if input.hover == Some(target) {
                    p.scene.fills.push(Fill::solid(r, 6.0, Ink::Hover));
                    Ink::Text
                } else {
                    Ink::Muted
                }
            }
            None => Ink::Faint,
        };
        p.centred(glyph, &icons, r, ink)?;
    }
    let r = cell(1.0);
    p.scene.fills.push(Fill::shadow(r.offset(0.0, 1.0), 6.0, 4.0, 0.8));
    p.scene.fills.push(Fill::solid(r, 6.0, Ink::Raised));
    let choice = p.fonts.choice;
    p.text(&scene::text_size(value).to_string(), &choice, r, Ink::Text)
}

pub(crate) const SWITCH_W: f32 = 44.0;
const SWITCH_H: f32 = 24.0;

/// A feature's switch at the start of `bar`: a pill of the button's tone
/// with its muted knob at the start when off; teal, the knob white at the
/// end, when on.
pub(crate) fn switch(p: &mut Painter, input: &Input, bar: Rect, setting: Setting) {
    let on = scene::switch_on(setting, input.options[setting.index()]);
    let target = Target::Switch(setting);
    let cy = (bar.top + bar.bottom) / 2.0;
    let track = Rect::new(bar.left + 2.0, cy - SWITCH_H / 2.0, bar.left + 2.0 + SWITCH_W, cy + SWITCH_H / 2.0);
    let hover = p.hover == Some(target);
    let ground = match (on, hover) {
        (true, _) => Ink::Accent,
        (false, true) => Ink::Pressed,
        (false, false) => Ink::Button,
    };
    p.scene.fills.push(Fill::solid(track, SWITCH_H / 2.0, ground));
    let d = SWITCH_H - 6.0;
    let x = if on { track.right - 3.0 - d } else { track.left + 3.0 };
    let knob = Rect::new(x, cy - d / 2.0, x + d, cy + d / 2.0);
    p.scene.fills.push(Fill::shadow(knob.offset(0.0, 1.0), d / 2.0, 3.0, 0.6));
    p.scene.fills.push(Fill::solid(knob, d / 2.0, if on { Ink::OnAccent } else { Ink::Muted }));
    p.scene.hits.push((Rect::new(bar.left, bar.top, bar.left + SWITCH_W + 12.0, bar.bottom), target));
}

/// Settings > Font's choices in `bar`: Sans, Serif and Mono, each in the
/// face it stands for, and Other (the machine's font chosen, by its name in
/// its face, once one is).
fn font_choices(p: &mut Painter, input: &Input, bar: Rect) -> Result<()> {
    let value = input.options[Setting::Font.index()];
    p.scene.fills.push(Fill::solid(bar, 8.0, Ink::Field));
    let installed = value == scene::INSTALLED_FONT;
    let other = match input.font_name {
        Some(name) if installed => name,
        _ => "Other\u{2026}",
    };
    let names: [(&str, &'static str, Target); 4] = [
        ("Sans", input.font_faces[0], Target::Choice(Setting::Font, 0)),
        ("Serif", input.font_faces[1], Target::Choice(Setting::Font, 1)),
        ("Mono", input.font_faces[2], Target::Choice(Setting::Font, 2)),
        (other, if installed { input.font_name.unwrap_or("") } else { "" }, Target::FontOther),
    ];
    // Other is as wide as its name needs, within reason; the three share
    // the rest.
    let choice = p.fonts.choice.clone();
    let other_w = if installed { (scene::single_line(p.eng, other, &choice, 400.0, bar.height()).measure().width + 24.0).clamp(bar.width() / 4.0, bar.width() * 0.4) } else { bar.width() / 4.0 };
    let w = (bar.width() - other_w) / 3.0;
    for (k, (label, face, target)) in names.into_iter().enumerate() {
        let (l, r) = if k < 3 { (bar.left + k as f32 * w, bar.left + (k + 1) as f32 * w) } else { (bar.right - other_w, bar.right) };
        let r = Rect::new(l, bar.top, r, bar.bottom).inset(2.0);
        let on = if k < 3 { value == k as u8 } else { installed };
        let open = k == 3 && input.font_finder.is_some();
        let hover = input.hover == Some(target);
        if on || open {
            p.scene.fills.push(Fill::shadow(r.offset(0.0, 1.0), 6.0, 4.0, 0.8));
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Raised));
        } else if hover {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Hover));
        }
        p.scene.hits.push((r, target));
        let format = Format { face: (!face.is_empty()).then_some(face), ellipsis: true, ..choice.clone() };
        p.text(label, &format, Rect::new(r.left + 6.0, r.top, r.right - 6.0, r.bottom), if on || open || hover { Ink::Text } else { Ink::Muted })?;
    }
    Ok(())
}

const FINDER_ROW: f32 = 34.0;
/// The most families the finder lists at once: typing narrows them.
pub const FINDER_ROWS: usize = 6;

fn finder_height(f: &scene::FontFinder) -> f32 {
    6.0 + 36.0 + 6.0 + f.names.len().clamp(1, FINDER_ROWS) as f32 * FINDER_ROW + 8.0
}

/// The finder of the machine's fonts under the Font row, from `left` to
/// `right` at `y`: a field to type a name in, and the families that match,
/// each in its own face. Where it ends.
fn font_finder(p: &mut Painter, f: &scene::FontFinder, left: f32, right: f32, y: f32) -> Result<f32> {
    let card = Rect::new(left, y + 2.0, right, y + finder_height(f) - 4.0);
    p.scene.fills.push(Fill::solid(card, 10.0, Ink::Field));
    p.scene.hits.push((card, Target::FontField));
    let field = Rect::new(card.left + 6.0, card.top + 6.0, card.right - 6.0, card.top + 42.0);
    p.scene.fills.push(Fill::solid(field, 7.0, Ink::Raised));
    let label = p.fonts.label.clone();
    let text = Rect::new(field.left + 12.0, field.top, field.right - 12.0, field.bottom);
    if f.query.is_empty() {
        p.text("Find a font", &label, text, Ink::Faint)?;
    } else {
        p.text(f.query, &label, text, Ink::Text)?;
    }
    let caret = if f.query.is_empty() { 0.0 } else { scene::single_line(p.eng, &f.query[..f.caret.min(f.query.len())], &label, 1000.0, field.height()).measure().width };
    let cx = (text.left + caret).min(text.right);
    p.scene.fills.push(Fill::solid(Rect::new(cx, field.top + 9.0, cx + 1.5, field.bottom - 9.0), 0.0, Ink::Accent));
    let mut ry = field.bottom + 6.0;
    if f.names.is_empty() {
        let said = if f.total == 0 && f.query.is_empty() { "Reading the fonts\u{2026}" } else { "No font by that name" };
        p.text(said, &label, Rect::new(text.left, ry, text.right, ry + FINDER_ROW), Ink::Muted)?;
        return Ok(y + finder_height(f));
    }
    for (i, name) in f.names.iter().take(FINDER_ROWS).enumerate() {
        let r = Rect::new(card.left + 4.0, ry, card.right - 4.0, ry + FINDER_ROW);
        let target = Target::FontName(i);
        let picked = f.picked == Some(i) || p.hover == Some(target);
        if picked {
            p.scene.fills.push(Fill::solid(r, 7.0, Ink::Hover));
        }
        p.scene.hits.push((r, target));
        let format = Format { face: Some(name), ellipsis: true, size: label.size + 1.0, ..label.clone() };
        p.text(name, &format, Rect::new(r.left + 12.0, r.top, r.right - 12.0, r.bottom), if picked { Ink::Text } else { Ink::Muted })?;
        ry += FINDER_ROW;
    }
    Ok(y + finder_height(f))
}

/// A column of sections from `top`, in the card's scrolled `view`. Rows
/// under a switch slide out from under it as it opens, clipped to the room
/// they have so far, and back as it closes.
fn options_column(p: &mut Painter, input: &Input, sections: &'static [Section], left: f32, right: f32, top: f32, view: Rect) -> Result<()> {
    let section = p.fonts.section.clone();
    let mut y = top;
    for (name, rows) in shown(sections, input) {
        p.text(name, &section, Rect::new(left, y, right, y + SECTION_H), Ink::Faint)?;
        y += SECTION_H;
        let mut i = 0;
        while i < rows.len() {
            let Some(s) = rows[i].1 else {
                y = option_row(p, input, rows[i].0, left, right, y)?;
                i += 1;
                continue;
            };
            // The run of rows under switch `s`: drawn whole, slid up by
            // what is still folded, in a layer clipped to what shows.
            let run: Vec<&'static Row> = rows[i..].iter().take_while(|r| r.1 == Some(s)).map(|r| r.0).collect();
            i += run.len();
            let whole: f32 = run.iter().map(|r| row_height(r, input)).sum();
            let open = fold_open(input, s);
            let shows = whole * open.min(1.0);
            let clip = Rect::new(view.left, y.max(view.top), view.right, (y + shows).min(view.bottom));
            p.scene.overlay(Some(clip));
            let hits_from = p.scene.hits.len();
            let mut ry = y - whole * (1.0 - open);
            for row in run {
                ry = option_row(p, input, row, left, right, ry)?;
            }
            let hits: Vec<(Rect, Target)> = p.scene.hits.drain(hits_from..).collect();
            p.scene.hits.extend(hits.into_iter().map(|(r, t)| (Rect::new(r.left, r.top.max(clip.top), r.right, r.bottom.min(clip.bottom)), t)).filter(|(r, _)| r.height() >= 8.0));
            p.scene.overlay(Some(view));
            y += shows;
        }
        y += SECTION_GAP;
    }
    Ok(())
}

/// One option's row at `y`; where the next starts.
fn option_row(p: &mut Painter, input: &Input, row: &'static Row, left: f32, right: f32, mut y: f32) -> Result<f32> {
    let label = p.fonts.label.clone();
    let bar = Rect::new(left + LABEL_W, y + 5.0, right, y + ROW_H - 5.0);
    {
        {
            match row {
                Row::Choice(name, setting, choices) => {
                    p.text(name, &label, Rect::new(left, y, left + LABEL_W - 8.0, y + ROW_H), Ink::Muted)?;
                    let items: Vec<(&str, Target)> = choices.iter().map(|&(l, v)| (l, Target::Choice(*setting, v))).collect();
                    let value = input.options[setting.index()];
                    segments(p, bar, &items, Some(Target::Choice(*setting, value)))?;
                    y += ROW_H;
                }
                Row::Stepper(name, setting) => {
                    p.text(name, &label, Rect::new(left, y, left + LABEL_W - 8.0, y + ROW_H), Ink::Muted)?;
                    stepper(p, input, bar, *setting)?;
                    y += ROW_H;
                }
                Row::Buttons(name, items) => {
                    p.text(name, &label, Rect::new(left, y, left + LABEL_W - 8.0, y + ROW_H), Ink::Muted)?;
                    segments(p, bar, items, None)?;
                    y += ROW_H;
                }
                Row::Vault => {
                    p.text("Folder", &label, Rect::new(left, y, left + LABEL_W - 8.0, y + ROW_H), Ink::Muted)?;
                    let l = scene::single_line(p.eng, vault_shown(input.platform, input.vault_path), &label, right - left - LABEL_W, ROW_H);
                    p.scene.texts.push(Text { x: left + LABEL_W + 4.0, y, layout: l, ink: Ink::Text, alpha: 1.0 });
                    y += ROW_H;
                }
                Row::Devices => {
                    if let Some(v) = &input.devices {
                        y = crate::quick::devices_panel(p, v, left, right, y, false)?;
                    }
                }
                Row::CheckNow => {
                    check_now(p, input, bar)?;
                    y += ROW_H;
                }
                Row::Switch(name, setting) => {
                    p.text(name, &label, Rect::new(left, y, left + LABEL_W - 8.0, y + ROW_H), Ink::Muted)?;
                    switch(p, input, bar, *setting);
                    y += ROW_H;
                }
                // On the heading's line, above where the rows start.
                Row::Head(setting) => switch(p, input, Rect::new(left + LABEL_W, y - SECTION_H + 1.0, right, y - 1.0), *setting),
                Row::Font => {
                    p.text("Font", &label, Rect::new(left, y, left + LABEL_W - 8.0, y + ROW_H), Ink::Muted)?;
                    font_choices(p, input, bar)?;
                    y += ROW_H;
                    if let Some(f) = &input.font_finder {
                        y = font_finder(p, f, left + LABEL_W, right, y)?;
                    }
                }
                // Resolved by `shown`.
                Row::On(..) | Row::Updater(_) | Row::NotOn(..) | Row::IfOn(..) => {}
            }
        }
    }
    Ok(y)
}

/// Every option, in a card over the middle of the window (the page behind
/// dims): what things look like, how writing works, opening and closing,
/// the vault, notes in and out; how fast this launch was.
pub fn options_page(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    if !input.options_open {
        return Ok(());
    }
    scene.overlay(None);
    let mut p = Painter { eng, fonts, hover: input.hover, scene };
    p.scene.fills.push(Fill::faded(Rect::new(0.0, 0.0, width, height), 0.0, Ink::Page, 0.62));
    p.scene.hits.push((Rect::new(0.0, 0.0, width, height), Target::OptionsVeil));
    let two = width >= 820.0;
    let w = if two { (width - 64.0).min(900.0) } else { (width - 32.0).min(480.0) };
    let body = if two {
        column_height(LEFT, input).max(column_height(RIGHT, input))
    } else {
        column_height(LEFT, input) + column_height(RIGHT, input)
    };
    // As tall as it is with every switch on: the card's top stays there
    // while a switch's rows fold.
    let tallest = if two {
        column_height_as(LEFT, input, true).max(column_height_as(RIGHT, input, true))
    } else {
        column_height_as(LEFT, input, true) + column_height_as(RIGHT, input, true)
    };
    let header = 62.0;
    let footer = 44.0;
    // Its top is where the tallest card's would be; its foot follows the
    // rows, so a switch's rows folding away draw the card up after them.
    let most = (header + tallest + footer).min(height - 32.0);
    let top = ((height - most) / 2.0).max(16.0);
    let h = (header + body + footer).min(height - 32.0);
    let card = Rect::new((width - w) / 2.0, top, (width + w) / 2.0, top + h);
    p.panel(card, 1.0);
    p.scene.hits.push((card, Target::OptionsPanel));
    let title = p.fonts.peek_title.clone();
    let heading = scene::single_line(eng, "Settings", &title, 300.0, header);
    p.scene.texts.push(Text { x: card.left + 28.0, y: card.top, layout: heading, ink: Ink::Text, alpha: 1.0 });
    let close = Rect::new(card.right - 16.0 - 32.0, card.top + 15.0, card.right - 16.0, card.top + 47.0);
    p.button(close, Target::OptionsClose, 8.0);
    let small_icons = p.fonts.icon_small.clone();
    p.text(icon::CLOSE, &small_icons, close, if p.hover == Some(Target::OptionsClose) { Ink::Text } else { Ink::Muted })?;
    // The rows scroll between the title and the footer when the window
    // is too short for them all.
    let view = Rect::new(card.left, card.top + header, card.right, card.bottom - footer);
    let max = (body - view.height()).max(0.0);
    p.scene.options_scroll_max = max;
    p.scene.options_devices_at = devices_scroll(input, width);
    let scroll = input.options_scroll.clamp(0.0, max);
    p.scene.overlay(Some(view));
    let hits_from = p.scene.hits.len();
    let top = view.top - scroll;
    if two {
        let mid = (card.left + card.right) / 2.0;
        options_column(&mut p, input, LEFT, card.left + 28.0, mid - 20.0, top, view)?;
        options_column(&mut p, input, RIGHT, mid + 20.0, card.right - 28.0, top, view)?;
    } else {
        options_column(&mut p, input, LEFT, card.left + 20.0, card.right - 20.0, top, view)?;
        options_column(&mut p, input, RIGHT, card.left + 20.0, card.right - 20.0, top + column_height(LEFT, input), view)?;
    }
    // Rows scrolled out of view take no clicks, nor a sliver at its edge
    // (scrolling brings the rest of it into reach).
    let rows: Vec<(Rect, Target)> = p.scene.hits.drain(hits_from..).collect();
    p.scene.hits.extend(
        rows.into_iter()
            .map(|(r, t)| (Rect::new(r.left, r.top.max(view.top), r.right, r.bottom.min(view.bottom)), t))
            .filter(|(r, _)| r.height() >= 8.0),
    );
    if max > 0.0 {
        // A thin mark of where the view is, at the card's edge.
        let track = view.inset(6.0);
        let size = (view.height() / body * track.height()).max(24.0);
        let at = track.top + (track.height() - size) * scroll / max;
        p.scene.fills.push(Fill::faded(Rect::new(card.right - 7.0, at, card.right - 4.0, at + size), 1.5, Ink::Muted, 0.5));
    }
    p.scene.overlay(None);
    // How fast this launch was: the app's whole point.
    let line = Rect::new(card.left + 24.0, card.bottom - footer, card.right - 24.0, card.bottom - footer + 1.0);
    p.scene.fills.push(Fill::solid(line, 0.0, Ink::Border));
    let mut about = format!("Kolada {}", kolada_core::update::Version::current());
    if let Some(ms) = input.opened_in {
        about.push_str(&format!(" \u{00B7} opened in {ms:.0} ms"));
    }
    if let Some(ms) = input.keystroke {
        about.push_str(&format!(" \u{00B7} {ms:.2} ms a keystroke"));
    }
    let choice = p.fonts.choice.clone();
    p.text(&about, &choice, Rect::new(card.left, card.bottom - footer, card.right, card.bottom), Ink::Faint)?;
    Ok(())
}

/// A menu at a point: items with their icons and shortcuts, dividers
/// between groups. It opens down and to the right of the point, or up and
/// to the left where the window ends. Drawn over everything.
pub fn menu(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    const ITEM: f32 = 32.0;
    let Some(m) = &input.menu else { return Ok(()) };
    let label = fonts.label.clone();
    let small = fonts.small.clone();
    // As wide as its longest item: icon, label, a gap, shortcut.
    let measure = |text: &str, format: &Format| -> Result<f32> {
        Ok(scene::layout(eng, text, format, 1000.0, ITEM).measure().width)
    };
    let (mut labels, mut keys) = (0.0f32, 0.0f32);
    for item in m.items {
        labels = labels.max(measure(&item.label, &label)?);
        if !item.keys.is_empty() {
            keys = keys.max(measure(item.keys, &small)?);
        }
    }
    let keys_room = if keys > 0.0 { keys + 20.0 } else { 0.0 };
    let w = (6.0 + 34.0 + labels + keys_room + 18.0).clamp(MENU_WIDTH, 420.0);
    scene.overlay(None);
    let (fills_from, texts_from) = (scene.fills.len(), scene.texts.len());
    let mut p = Painter { eng, fonts, hover: input.hover, scene };
    let dividers = m.items.iter().filter(|i| i.divider).count() as f32;
    let h = 12.0 + m.items.len() as f32 * ITEM + dividers * 9.0;
    // Opened from a button at the right, it lines up with the button's
    // right edge; elsewhere it opens right of the point, or left of it at
    // the window's edge.
    let x = if m.align_right { m.x - w } else { m.x };
    let x = if x + w > width - 8.0 { (m.x - w).min(width - 8.0 - w) } else { x }.max(8.0);
    // Where it does not fit below the point it opens above it, and keeps
    // clear of the window's foot too (a right-click at the very bottom took
    // it past the edge).
    let y = if m.y + h > height - 8.0 { (m.y - h).min(height - 8.0 - h).max(8.0) } else { m.y };
    let card = Rect::new(x, y, x + w, y + h);
    p.panel(card, 1.0);
    p.scene.hits.push((card, Target::MenuPanel));
    let mut y = card.top + 6.0;
    for (i, item) in m.items.iter().enumerate() {
        if item.divider {
            let line = Rect::new(card.left + 12.0, y + 4.0, card.right - 12.0, y + 5.0);
            p.scene.fills.push(Fill::solid(line, 0.0, Ink::Border));
            y += 9.0;
        }
        let r = Rect::new(card.left + 6.0, y, card.right - 6.0, y + ITEM);
        let icon = if item.icon.is_empty() { RowIcon::None } else { RowIcon::Glyph(item.icon) };
        menu_row(&mut p, r, Target::MenuItem(i), icon, &item.label, item.keys, keys, item.danger)?;
        y += ITEM;
    }
    let a = m.alpha.clamp(0.0, 1.0);
    if a < 1.0 {
        scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= a);
        scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= a);
    }
    Ok(())
}

/// A menu row's icon: a glyph of the icon font, or a mark drawn from its
/// outline (filled on every platform).
#[derive(Clone, Copy)]
pub(crate) enum RowIcon<'a> {
    None,
    Glyph(&'a str),
    Mark(crate::vector::Logo),
}

/// A menu's row height, its icon column and where its text starts: one set
/// for every menu (the title bar's, a row's, the Beam button's).
pub(crate) const MENU_ITEM: f32 = 32.0;
const MENU_ICON: f32 = 28.0;
const MENU_TEXT: f32 = 34.0;
/// A mark's size in the icon column (a glyph is its font's).
const MENU_MARK: f32 = 18.0;

/// One menu row in `r`: its highlight under the pointer, its icon centred
/// in the icon column, its label, and its keys at the right end on the
/// label's baseline (`keys_w`: the menu's widest keys).
pub(crate) fn menu_row(p: &mut Painter, r: Rect, target: Target, icon: RowIcon, label: &str, keys: &str, keys_w: f32, danger: bool) -> Result<()> {
    p.button(r, target, 6.0);
    let (icon_ink, ink) = if danger { (Ink::Danger, Ink::Danger) } else { (Ink::Muted, Ink::Text) };
    let column = Rect::new(r.left + 2.0, r.top, r.left + 2.0 + MENU_ICON, r.bottom);
    match icon {
        RowIcon::None => {}
        RowIcon::Glyph(g) => p.icon(g, column, icon_ink)?,
        RowIcon::Mark(logo) => {
            let (cx, cy) = ((column.left + column.right) / 2.0, (column.top + column.bottom) / 2.0);
            let m = Rect::new(cx - MENU_MARK / 2.0, cy - MENU_MARK / 2.0, cx + MENU_MARK / 2.0, cy + MENU_MARK / 2.0);
            let layer = p.scene.layer_now();
            p.scene.vectors.push(VectorMark { layer, rect: m, logo, ink: icon_ink, alpha: 1.0 });
        }
    }
    let keys_room = if keys_w > 0.0 { keys_w + 20.0 } else { 0.0 };
    let label_format = p.fonts.label.clone();
    p.text(label, &label_format, Rect::new(r.left + MENU_TEXT, r.top, r.right - 12.0 - keys_room, r.bottom), ink)?;
    if !keys.is_empty() {
        let small = p.fonts.small.clone();
        on_baseline(p, keys, &Format { align: Align::End, ..small }, &label_format, Rect::new(r.right - 12.0 - keys_w - 4.0, r.top, r.right - 12.0, r.bottom), Ink::Faint);
    }
    Ok(())
}

/// Where a line of `format` centred in a box `h` tall puts its baseline,
/// from the box's top.
fn baseline_in(p: &Painter, format: &Format, h: f32) -> f32 {
    let l = scene::single_line(p.eng, "Hx", format, 200.0, h);
    l.glyphs((0.0, 0.0), 1.0).first().map_or(h * 0.7, |g| g.y as f32)
}

/// `text` in `r`, set on the baseline a line of `beside` centred in `r`
/// would have: a small key hint next to its label reads as one line.
pub(crate) fn on_baseline(p: &mut Painter, text: &str, format: &Format, beside: &Format, r: Rect, ink: Ink) {
    let want = baseline_in(p, beside, r.height());
    let own = baseline_in(p, &Format { center_y: false, ..format.clone() }, r.height());
    let l = scene::single_line(p.eng, text, &Format { center_y: false, ..format.clone() }, r.width().max(1.0), r.height());
    p.scene.texts.push(Text { x: r.left, y: r.top + want - own, layout: l, ink, alpha: 1.0 });
}

/// The find bar's width, and a row's height.
pub const FIND_WIDTH: f32 = 384.0;
const FIND_ROW: f32 = 40.0;

/// A one-line field: its text (or a faint placeholder), and the caret when
/// it has the keys.
fn field(p: &mut Painter, r: Rect, text: &str, caret: Option<usize>, placeholder: &str, target: Target) -> Result<()> {
    p.scene.fills.push(Fill::solid(r, 7.0, if caret.is_some() { Ink::Border } else { Ink::Field }));
    if caret.is_some() {
        p.scene.fills.push(Fill::solid(r.inset(1.0), 6.0, Ink::Field));
    }
    p.scene.hits.push((r, target));
    let label = p.fonts.label.clone();
    let inner = Rect::new(r.left + 10.0, r.top, r.right - 8.0, r.bottom);
    if text.is_empty() {
        p.text(placeholder, &label, inner, Ink::Faint)?;
    } else {
        p.text(text, &label, inner, Ink::Text)?;
    }
    if let Some(at) = caret {
        let l = scene::layout(p.eng, text, &label, inner.width(), inner.height());
        let cx = l.caret(at.min(text.len())).x;
        let c = Rect::new(inner.left + cx - 0.75, r.top + 7.0, inner.left + cx + 0.75, r.bottom - 7.0);
        p.scene.fills.push(Fill::solid(c, 0.75, Ink::Accent));
    }
    Ok(())
}

/// The find bar, over the top right of the note: what to find, how many
/// matches and which is on, the way to the one before and after, replacing
/// (a second row), and closing.
pub fn find_bar(eng: &Eng, fonts: &Fonts, input: &Input, scene: &mut Scene) -> Result<()> {
    let Some(f) = &input.find else { return Ok(()) };
    scene.overlay(None);
    let column = scene.column;
    let rows = if f.replace.is_some() { 2.0 } else { 1.0 };
    let top = (column.top - FIND_ROW - 8.0).max(MARGIN);
    let card = Rect::new(column.right - FIND_WIDTH, top, column.right, top + rows * FIND_ROW + 8.0);
    let mut p = Painter { eng, fonts, hover: input.hover, scene };
    p.panel(card, 1.0);
    p.scene.hits.push((card, Target::FindPanel));
    let y = card.top + 4.0;
    let buttons = [
        (Target::FindPrev, "\u{E70E}"),
        (Target::FindNext, "\u{E70D}"),
        (Target::FindReplace, "\u{E8AB}"),
        (Target::FindClose, icon::CLOSE),
    ];
    let buttons_left = card.right - 6.0 - buttons.len() as f32 * 30.0;
    let count = Rect::new(buttons_left - 64.0, y, buttons_left - 4.0, y + FIND_ROW);
    let query = Rect::new(card.left + 6.0, y + 4.0, count.left - 4.0, y + FIND_ROW - 4.0);
    field(&mut p, query, f.query, (f.focus == 1).then_some(f.query_caret), "Find in note", Target::FindField)?;
    let status = match (f.count, f.current) {
        _ if f.query.is_empty() => String::new(),
        (0, _) => "No match".to_string(),
        (n, Some(i)) => format!("{} of {n}", i + 1),
        (n, None) => format!("{n}"),
    };
    let choice = p.fonts.choice.clone();
    p.text(&status, &choice, count, if f.count == 0 { Ink::Faint } else { Ink::Muted })?;
    for (k, (target, glyph)) in buttons.into_iter().enumerate() {
        let r = Rect::new(buttons_left + k as f32 * 30.0, y + 5.0, buttons_left + k as f32 * 30.0 + 28.0, y + FIND_ROW - 5.0);
        let on = target == Target::FindReplace && f.replace.is_some();
        if on {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Selection));
        }
        p.button(r, target, 6.0);
        let small = p.fonts.icon_small.clone();
        p.text(glyph, &small, r, if p.hover == Some(target) || on { Ink::Text } else { Ink::Muted })?;
    }
    if let Some((text, caret)) = f.replace {
        let y = y + FIND_ROW;
        let one = Rect::new(buttons_left - 64.0, y + 5.0, buttons_left + 50.0, y + FIND_ROW - 5.0);
        let all = Rect::new(one.right + 4.0, one.top, card.right - 8.0, one.bottom);
        let replace = Rect::new(card.left + 6.0, y + 4.0, one.left - 8.0, y + FIND_ROW - 4.0);
        field(&mut p, replace, text, (f.focus == 2).then_some(caret), "Replace with", Target::ReplaceField)?;
        for (r, target, label) in [(one, Target::ReplaceOne, "Replace"), (all, Target::ReplaceAll, "All")] {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Field));
            p.button(r, target, 6.0);
            p.text(label, &choice, r, if p.hover == Some(target) { Ink::Text } else { Ink::Muted })?;
        }
    }
    Ok(())
}

/// Notes to link, in a card under the caret while a `[[link]]` is typed.
pub fn suggestions(eng: &Eng, fonts: &Fonts, input: &Input, scene: &mut Scene) -> Result<()> {
    const ROW_H: f32 = 30.0;
    const W: f32 = 300.0;
    let (Some(s), Some(caret)) = (&input.suggestions, scene.caret) else { return Ok(()) };
    let rows = s.names.len() + s.create.is_some() as usize;
    if rows == 0 {
        return Ok(());
    }
    scene.overlay(None);
    let h = 8.0 + rows as f32 * ROW_H;
    let mut top = caret.bottom + 6.0;
    // Above the line, when there is no room below.
    if top + h > scene.column.bottom - 8.0 {
        top = caret.top - 6.0 - h;
    }
    let left = (caret.left - 12.0).min(scene.column.right + 40.0 - W).max(8.0);
    let card = Rect::new(left, top, left + W, top + h);
    let mut p = Painter { eng, fonts, hover: input.hover, scene };
    p.panel(card, 1.0);
    let label = fonts.label.clone();
    let small = fonts.small.clone();
    let icons = fonts.icon_small.clone();
    for i in 0..rows {
        let y = card.top + 4.0 + i as f32 * ROW_H;
        let r = Rect::new(card.left + 4.0, y, card.right - 4.0, y + ROW_H);
        if i == s.selected {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Selection));
        } else if p.hover == Some(Target::Suggestion(i)) {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Hover));
        }
        p.scene.hits.push((r, Target::Suggestion(i)));
        match s.names.get(i) {
            Some((name, folder)) => {
                p.text(icon::PAGE, &icons, Rect::new(r.left + 2.0, y, r.left + 26.0, y + ROW_H), Ink::Faint)?;
                let right = if folder.is_empty() { r.right - 8.0 } else { r.right - 96.0 };
                p.text(name, &label, Rect::new(r.left + 28.0, y, right, y + ROW_H), Ink::Text)?;
                if !folder.is_empty() {
                    p.text(folder, &small, Rect::new(right + 6.0, y, r.right - 8.0, y + ROW_H), Ink::Faint)?;
                }
            }
            None => {
                let name = s.create.unwrap_or("");
                p.text(icon::ADD, &icons, Rect::new(r.left + 2.0, y, r.left + 26.0, y + ROW_H), Ink::Accent)?;
                p.text(&format!("New note \u{201C}{name}\u{201D}"), &label, Rect::new(r.left + 28.0, y, r.right - 8.0, y + ROW_H), Ink::Muted)?;
            }
        }
    }
    Ok(())
}

/// The link field's width at most, a suggestion row's height, and the
/// field row's.
pub const LINK_WIDTH: f32 = 340.0;
const LINK_ROW: f32 = 30.0;
const LINK_FIELD_ROW: f32 = 40.0;
/// The row of a link's actions (Open, Copy link, Edit, Remove link), or
/// of keeping an edit (Save, Cancel).
const LINK_ACTIONS_ROW: f32 = 36.0;
/// The link field's width on a link (its actions say themselves in full).
const LINK_ACTIONS_WIDTH: f32 = 400.0;

/// What a link's actions say: in full where they fit, else in a word.
const LINK_ACTIONS: [(Target, &str, &str); 4] = [
    (Target::LinkOpen, "Open", "Open"),
    (Target::LinkCopy, "Copy link", "Copy"),
    (Target::LinkEdit, "Edit", "Edit"),
    (Target::LinkRemove, "Remove link", "Remove"),
];
const LINK_EDIT_ACTIONS: [(Target, &str, &str); 2] = [(Target::LinkSave, "Save", "Save"), (Target::LinkCancel, "Cancel", "Cancel")];

/// The link field (Ctrl+K), in a card under the selection (above it when
/// there is no room below): a note's name or a web address to type, and
/// the notes that fit what is typed. On a link it opens on where the link
/// goes, quietly, and what can be done with it (Open the strongest); Edit
/// makes that the field, with Save and Cancel under it.
pub fn link_field(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    let Some(f) = &input.link_field else { return Ok(()) };
    scene.overlay(None);
    let rows = if f.editing { f.names.len() } else { 0 };
    let actions: &[(Target, &str, &str)] = match (f.existing, f.editing) {
        (true, false) => &LINK_ACTIONS,
        (true, true) => &LINK_EDIT_ACTIONS,
        _ => &[],
    };
    // A link's four actions need room to say themselves in full.
    let w = if f.existing { LINK_ACTIONS_WIDTH } else { LINK_WIDTH }.min(width - 16.0);
    let h = LINK_FIELD_ROW + 8.0 + if rows > 0 { rows as f32 * LINK_ROW + 4.0 } else { 0.0 } + if actions.is_empty() { 0.0 } else { LINK_ACTIONS_ROW };
    let column = scene.column;
    let anchor = scene.caret.unwrap_or(Rect::new(column.left, column.top, column.left, column.top + 24.0));
    let mut top = anchor.bottom + 8.0;
    if top + h > height - 8.0 {
        top = anchor.top - 8.0 - h;
    }
    let top = top.min(height - 8.0 - h).max(8.0);
    let left = (anchor.left - 16.0).min(width - 8.0 - w).max(8.0);
    let card = Rect::new(left, top, left + w, top + h);
    let mut p = Painter { eng, fonts, hover: input.hover, scene };
    p.panel(card, 1.0);
    p.scene.hits.push((card, Target::LinkPanel));
    let y = card.top + 4.0;
    let r = Rect::new(card.left + 6.0, y + 4.0, card.right - 6.0, y + LINK_FIELD_ROW - 4.0);
    if f.editing {
        field(&mut p, r, f.query, Some(f.caret), "Note or web address", Target::LinkField)?;
    } else {
        // Where it goes, as it is: quiet, and Edit on a press.
        p.scene.fills.push(Fill::solid(r, 7.0, Ink::Field));
        p.scene.hits.push((r, Target::LinkEdit));
        let label = p.fonts.label.clone();
        let line = scene::single_line(p.eng, f.query, &label, r.width() - 18.0, r.height());
        p.scene.texts.push(Text { x: r.left + 10.0, y: r.top, layout: line, ink: Ink::Muted, alpha: 1.0 });
    }
    if !actions.is_empty() {
        let top = card.bottom - 4.0 - LINK_ACTIONS_ROW;
        link_actions(&mut p, Rect::new(card.left + 6.0, top + 2.0, card.right - 6.0, top + LINK_ACTIONS_ROW - 4.0), actions)?;
    }
    let label = fonts.label.clone();
    let small = fonts.small.clone();
    let icons = fonts.icon_small.clone();
    for (i, (name, folder)) in f.names.iter().take(rows).enumerate() {
        let y = card.top + 4.0 + LINK_FIELD_ROW + 4.0 + i as f32 * LINK_ROW;
        let r = Rect::new(card.left + 4.0, y, card.right - 4.0, y + LINK_ROW);
        if f.picked == Some(i) {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Selection));
        } else if p.hover == Some(Target::LinkSuggestion(i)) {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Hover));
        }
        p.scene.hits.push((r, Target::LinkSuggestion(i)));
        p.text(icon::PAGE, &icons, Rect::new(r.left + 2.0, y, r.left + 26.0, y + LINK_ROW), Ink::Faint)?;
        let right = if folder.is_empty() { r.right - 8.0 } else { r.right - 96.0 };
        p.text(name, &label, Rect::new(r.left + 28.0, y, right, y + LINK_ROW), Ink::Text)?;
        if !folder.is_empty() {
            p.text(folder, &small, Rect::new(right + 6.0, y, r.right - 8.0, y + LINK_ROW), Ink::Faint)?;
        }
    }
    Ok(())
}

/// A link's actions in `row`, side by side: each the button's fill, its
/// words in the full ink; the first (Open, Save) the strongest, the text
/// ink itself with the page's words. In full where every one fits, else
/// each in a word.
fn link_actions(p: &mut Painter, row: Rect, actions: &[(Target, &str, &str)]) -> Result<()> {
    const GAP: f32 = 6.0;
    let n = actions.len() as f32;
    let w = (row.width() - GAP * (n - 1.0)) / n;
    let choice = p.fonts.choice.clone();
    let strong = Format { weight: 600, ..choice.clone() };
    let fits = actions.iter().all(|(_, long, _)| scene::single_line(p.eng, long, &strong, 1000.0, row.height()).measure().width <= w - 12.0);
    for (k, &(target, long, short)) in actions.iter().enumerate() {
        let r = Rect::new(row.left + k as f32 * (w + GAP), row.top, row.left + k as f32 * (w + GAP) + w, row.bottom);
        let hot = p.hover == Some(target);
        let first = k == 0;
        let (fill, ink) = if first { (Ink::Text, Ink::Page) } else { (if hot { Ink::Pressed } else { Ink::Button }, Ink::Text) };
        p.scene.fills.push(Fill { alpha: if first && hot { 0.85 } else { 1.0 }, ..Fill::solid(r, 6.0, fill) });
        p.scene.hits.push((r, target));
        p.text(if fits { long } else { short }, if first { &strong } else { &choice }, r, ink)?;
    }
    Ok(())
}

/// Where a link goes, in a small quiet card just under the pointer resting
/// on it (above it at the window's foot): the only place an address shows.
pub fn link_card(eng: &Eng, fonts: &Fonts, input: &Input, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    const H: f32 = 26.0;
    const MOST: f32 = 420.0;
    let Some((address, x, y)) = input.link_card else { return Ok(()) };
    if input.link_field.is_some() || address.is_empty() {
        return Ok(());
    }
    scene.overlay(None);
    let most = MOST.min(width - 16.0);
    let small = fonts.small.clone();
    let text = scene::single_line(eng, address, &small, most - 20.0, H);
    let w = (text.measure().width + 20.0).min(most);
    let text = Rc::new(text.with_width(&mut eng.borrow_mut(), w - 20.0 + 0.5));
    let mut top = y + 20.0;
    if top + H > height - 8.0 {
        top = y - 12.0 - H;
    }
    let top = top.min(height - 8.0 - H).max(8.0);
    let left = (x - 10.0).min(width - 8.0 - w).max(8.0);
    let r = Rect::new(left, top, left + w, top + H);
    scene.fills.push(Fill::shadow(r.offset(0.0, 2.0), 8.0, 10.0, 0.6));
    scene.fills.push(Fill::solid(r, 8.0, Ink::Panel));
    scene.texts.push(Text { x: r.left + 10.0, y: r.top, layout: text, ink: Ink::Muted, alpha: 1.0 });
    scene.link_card = Some(r);
    Ok(())
}

/// A long note's headings beside its scroll bar, each where the thumb is
/// when that heading is at the top; the section in view stands out. Where
/// two would overlap, the higher heading stays.
#[allow(clippy::too_many_arguments)]
pub fn landmarks(eng: &Eng, fonts: &Fonts, input: &Input, column: Rect, width: f32, height: f32, bar: scene::ScrollView, alpha: f32, scene: &mut Scene) -> Result<()> {
    const H: f32 = 22.0;
    const W: f32 = 230.0;
    let track = scroll_track(column, width, height);
    let (_, thumb) = thumb(track, bar.at, bar.size);
    // Which to show: by level first, then top to bottom, none overlapping.
    let mut order: Vec<usize> = (0..input.landmarks.len()).collect();
    order.sort_by_key(|&i| (input.landmarks[i].level, i));
    let mut placed: Vec<(usize, f32)> = Vec::new();
    for i in order {
        let y = track.top + (track.height() - thumb) * input.landmarks[i].at.clamp(0.0, 1.0);
        if placed.iter().all(|&(_, py)| (py - y).abs() >= H + 2.0) {
            placed.push((i, y));
        }
    }
    placed.sort_by(|a, b| a.1.total_cmp(&b.1));
    scene.overlay(None);
    let (fills_from, texts_from) = (scene.fills.len(), scene.texts.len());
    let small = fonts.small.clone();
    for (i, y) in placed {
        let mark = &input.landmarks[i];
        let hover = input.hover == Some(Target::Heading(i));
        let text = scene::single_line(eng, mark.text, &small, W - 20.0, H);
        let w = (text.measure().width + 20.0).min(W);
        // Right to left, text sits at its box's far end: fit the box to it.
        let text = Rc::new(text.with_width(&mut eng.borrow_mut(), w - 20.0 + 0.5));
        let r = Rect::new(track.left - 6.0 - w, y - H / 2.0 + 4.0, track.left - 6.0, y + H / 2.0 + 4.0);
        scene.fills.push(Fill::shadow(r.offset(0.0, 1.0), H / 2.0, 6.0, 0.6));
        scene.fills.push(Fill::solid(r, H / 2.0, if hover { Ink::Hover } else { Ink::Panel }));
        if mark.here {
            scene.fills.push(Fill::solid(Rect::new(r.right - 3.0, r.top + 6.0, r.right - 1.0, r.bottom - 6.0), 1.0, Ink::Accent));
        }
        let ink = if hover || mark.here { Ink::Text } else { Ink::Muted };
        scene.texts.push(Text { x: r.left + 10.0, y: r.top, layout: text, ink, alpha: 1.0 });
        scene.hits.push((r, Target::Heading(i)));
    }
    if alpha < 1.0 {
        scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= alpha);
        scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= alpha);
    }
    Ok(())
}

/// A scroll bar's thumb in `track`: `at` 0 at the top to 1 at the end,
/// `size` the share of the whole in view. Thin and faint at rest; it
/// thickens under the pointer and while dragged.
pub fn scroll_thumb(scene: &mut Scene, track: Rect, at: f32, size: f32, hot: bool, alpha: f32) {
    let w = if hot { 6.0 } else { 3.0 };
    let (top, h) = thumb(track, at, size);
    let x = track.right - 3.0 - w;
    let ink = if hot { Ink::Muted } else { Ink::Faint };
    scene.fills.push(Fill::faded(Rect::new(x, top, x + w, top + h), w / 2.0, ink, if hot { 0.8 } else { 0.55 } * alpha));
}

/// A scroll bar thumb's top and height in `track` (see `scroll_thumb`).
pub fn thumb(track: Rect, at: f32, size: f32) -> (f32, f32) {
    let h = (track.height() * size.clamp(0.0, 1.0)).clamp(32.0_f32.min(track.height()), track.height());
    (track.top + (track.height() - h) * at.clamp(0.0, 1.0), h)
}

/// Where a scroll bar's track goes for a view of `height` beside `column`.
pub fn scroll_track(column: Rect, width: f32, height: f32) -> Rect {
    Rect::new(width - 16.0, column.top + 2.0, width - 1.0, height - 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Panel, hit};
    use kolada_core::session::SidebarRow;

    /// (dock, show) for each panel.
    fn scene_for(width: f32, sidebar: (f32, f32), toolbar: (f32, f32), wave: &[f32]) -> (Scene, Rect) {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let rows = vec![
            SidebarRow { depth: 0, kind: RowKind::Folder { expanded: true }, name: "Work".into() },
            SidebarRow { depth: 1, kind: RowKind::Note, name: "Plan".into() },
            SidebarRow { depth: 0, kind: RowKind::Note, name: "Inbox".into() },
        ];
        let input = Input {
            vault_name: "notes",
            active_note: "Work/Plan",
            text: "",
            sidebar: &rows,
            scroll_line: 0,
            scroll_px: 0.0,
            selection: (0, 0),
            lines: &[],
            active_tools: &[],
            tree_top: 0,
            search: None,
            options_open: false,
            options: [0; scene::SETTINGS],
            chips: &[],
            dragged: None,
            drop_at: None,
            arriving: None,
            page_fade: 1.0,
            highlights_open: false,
            peek: None,
            whisper: None,
            popup: None,
            sidebar_dock: sidebar.0,
            sidebar_show: sidebar.1,
            toolbar_dock: toolbar.0,
            toolbar_show: toolbar.1,
            hover: None,
            sidebar_wave: wave,
            toolbar_wave: &[],
            maximized: false,
            ..Default::default()
        };
        let mut scene = Scene::default();
        let column = layout(&eng, &fonts, &input, width, 800.0, &mut scene).unwrap();
        (scene, column)
    }

    fn rect_of(scene: &Scene, target: Target) -> Option<Rect> {
        scene.hits.iter().find(|(_, t)| *t == target).map(|(r, _)| *r)
    }

    #[test]
    fn docked_layout_has_every_control_and_toolbar_clears_window_buttons() {
        for width in [700.0, 1000.0, 1400.0, 2400.0] {
            let (scene, column) = scene_for(width, (1.0, 1.0), (1.0, 1.0), &[]);
            for target in [Target::Search, Target::NewNote, Target::Options, Target::Close, Target::Row(1), Target::ToggleToolbar] {
                assert!(rect_of(&scene, target).is_some(), "{target:?} missing at width {width}");
            }
            let minimize = rect_of(&scene, Target::Minimize).unwrap();
            let toggle = rect_of(&scene, Target::ToggleToolbar).unwrap();
            // Beside the window buttons, or on its own row below them.
            assert!(toggle.right <= minimize.left || toggle.top >= minimize.bottom, "toolbar overlaps window buttons at width {width}");
            assert!(column.left >= MARGIN + SIDEBAR_WIDTH, "note column under the side menu at width {width}");
            assert!(rect_of(&scene, Target::RevealSidebar).is_none(), "docked panels have no reveal zones");
        }
    }

    #[test]
    fn focus_mode_leaves_reveal_zones_near_the_pills() {
        let (scene, column) = scene_for(1400.0, (0.0, 0.0), (0.0, 0.0), &[]);
        assert!(rect_of(&scene, Target::Search).is_none());
        assert!(rect_of(&scene, Target::Tool(Tool::Bold)).is_none());
        // Anywhere along the left edge, and further out beside the pill.
        assert_eq!(hit(&scene.hits, 5.0, 20.0), Some(Target::RevealSidebar));
        assert_eq!(hit(&scene.hits, 5.0, 780.0), Some(Target::RevealSidebar));
        assert_eq!(hit(&scene.hits, 45.0, 370.0), Some(Target::RevealSidebar));
        // Above the note column, well beyond the pill's own width.
        assert_eq!(hit(&scene.hits, 700.0, 10.0), Some(Target::RevealToolbar));
        assert_eq!(hit(&scene.hits, 530.0, 10.0), Some(Target::RevealToolbar));
        // Window buttons stay reachable and the rest of the top edge drags.
        assert_eq!(hit(&scene.hits, 1395.0, 10.0), Some(Target::Close));
        assert_eq!(hit(&scene.hits, 150.0, 10.0), Some(Target::Caption));
        // The column recenters in the whole window, and keeps the toolbar's
        // room: revealed, the bar covers no text, and the note stays put.
        let (_, docked) = scene_for(1400.0, (1.0, 1.0), (1.0, 1.0), &[]);
        assert_eq!(column.top, docked.top);
        assert!(((column.left + column.right) / 2.0 - 700.0).abs() < 1.0);
    }

    #[test]
    fn keeps_room_for_related_notes_when_the_window_allows() {
        for (width, side) in [(1340.0, (1.0, 1.0)), (1268.0, (0.0, 0.0)), (1400.0, (1.0, 1.0)), (2400.0, (1.0, 1.0))] {
            let (_, column) = scene_for(width, side, (1.0, 1.0), &[]);
            let (left, w) = scene::margin(column, width).expect("room for the margin");
            assert!(left + w <= width - 24.0 + 0.5, "chips fit inside the window at {width}");
            assert_eq!(column.width(), column_max(0), "the note keeps its width at {width}");
        }
        // Too narrow: the column keeps its width and the margin is dropped.
        let (_, narrow) = scene_for(900.0, (1.0, 1.0), (1.0, 1.0), &[]);
        assert!(scene::margin(narrow, 900.0).is_none());
    }

    #[test]
    fn the_aa_card_and_a_buttons_name_float_as_lightly_as_the_toolbar() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let input = Input { page_fade: 1.0, toolbar_dock: 1.0, toolbar_show: 1.0, reading: Some(1.0), tooltip: Some(Target::Tool(Tool::Bold)), ..Default::default() };
        let scene = scene::build(&eng, &fonts, &input, 1400.0, 800.0).unwrap();
        let shadows: Vec<&Fill> = scene.fills.iter().filter(|f| f.ink == Ink::Shadow).collect();
        let bar = rect_of(&scene, Target::ToolbarPanel).unwrap();
        let card = rect_of(&scene, Target::ReadingPanel).unwrap();
        let name = scene.tooltip.unwrap();
        let under = |r: Rect| shadows.iter().find(|f| f.rect == r.offset(0.0, 1.0)).map(|f| (f.blur, f.alpha)).unwrap_or_else(|| panic!("no shadow under {r:?}"));
        let capsule = under(bar);
        assert_eq!(under(card), capsule, "the Aa card");
        assert_eq!(under(name), capsule, "a button's name");
        assert!(shadows.iter().all(|f| (f.blur, f.alpha) == capsule), "nothing there floats higher");
    }

    /// The note column with the side menu docked (or not), related notes
    /// on, and the line width `option`.
    fn column_with(width: f32, docked: bool, option: u8) -> Rect {
        let mut options = [0; scene::SETTINGS];
        options[Setting::LineWidth.index()] = option;
        let dock = if docked { 1.0 } else { 0.0 };
        let input = Input { options, sidebar_dock: dock, sidebar_show: dock, toolbar_dock: 1.0, toolbar_show: 1.0, ..Default::default() };
        column(&input, width, 800.0)
    }

    #[test]
    fn the_line_width_chosen_wins_over_related_notes() {
        // Wide, beside the docked side menu, in a window too narrow for it
        // and the related notes both: the note is as wide as Wide, and the
        // related notes give way.
        let wide = column_with(1400.0, true, 2);
        assert_eq!(wide.width(), column_max(2));
        assert!(scene::margin(wide, 1400.0).is_none(), "no related notes over the wide note");
        // Where there is room for both, both.
        let roomy = column_with(2200.0, true, 2);
        assert_eq!(roomy.width(), column_max(2));
        assert!(scene::margin(roomy, 2200.0).is_some());
        // Every width, every window: the column is as wide as the width
        // chosen asks, or as the window allows, never narrowed for the
        // margin; and the margin, when there is one, is right of it.
        for option in [1, 0, 2] {
            for docked in [true, false] {
                let mut width = 640.0;
                while width <= 2600.0 {
                    let c = column_with(width, docked, option);
                    let input = Input { sidebar_dock: if docked { 1.0 } else { 0.0 }, ..Default::default() };
                    let content = width - content_left(&input, width);
                    let pad = (content * 0.06).clamp(20.0, COLUMN_PAD);
                    let want = column_max(option).min(content - 2.0 * pad).max(80.0);
                    assert!((c.width() - want).abs() < 0.01, "line width {option} at {width}, docked {docked}: {} for {want}", c.width());
                    if let Some((left, w)) = scene::margin(c, width) {
                        assert!(left >= c.right && left + w <= width, "the margin beside the note at {width}");
                    }
                    width += 20.0;
                }
            }
        }
    }

    #[test]
    fn revealed_panels_float_without_moving_the_note() {
        let (resting, resting_column) = scene_for(1400.0, (0.0, 0.0), (0.0, 0.0), &[]);
        let (revealed, column) = scene_for(1400.0, (0.0, 1.0), (0.0, 1.0), &[]);
        assert_eq!(column, resting_column, "revealing must not reflow the note");
        assert!(rect_of(&resting, Target::NewNote).is_none());
        let new_note = rect_of(&revealed, Target::NewNote).unwrap();
        assert!(new_note.left >= MARGIN);
        // A margin just outside the revealed panel still belongs to it.
        let panel_right = MARGIN + SIDEBAR_WIDTH;
        let owner = hit(&revealed.hits, panel_right + 10.0, 400.0).and_then(Target::panel);
        assert_eq!(owner, Some(Panel::Sidebar));
    }

    #[test]
    fn a_pulled_string_bends_toward_the_pointer_and_stays_subtle() {
        let (rest, _) = scene_for(1400.0, (0.0, 0.0), (1.0, 1.0), &[]);
        let side = rest.pills[0].expect("collapsed side menu has a pill");
        let base = |s: &Scene| s.fills.iter().find(|f| f.rect == side).copied().unwrap();
        let resting = base(&rest);
        assert!(resting.wave.unwrap().d.iter().all(|&d| d == 0.0) && resting.alpha < 0.5, "quiet and straight at rest");

        let pulled_wave = crate::wave::target(side, true, Some((side.right + 10.0, (side.top + side.bottom) / 2.0)));
        let (pulled, _) = scene_for(1400.0, (0.0, 0.0), (1.0, 1.0), &pulled_wave);
        let bent = base(&pulled);
        let d = bent.wave.unwrap().d;
        let mid = d[crate::wave::SAMPLES / 2];
        assert!(mid > 5.0 && d[0] == 0.0, "the middle reaches toward the pointer, the ends are held");
        assert!(bent.alpha > resting.alpha && bent.alpha < 0.8, "brighter, but no flash");
        let tint = pulled.fills.iter().find(|f| f.rect == side && f.ink == Ink::Accent).unwrap();
        assert!(tint.alpha < 0.3 && tint.blur == 0.0, "only a hint of accent, no glow");
    }

    /// What a person clicks, as opposed to zones, panels and the text.
    fn is_control(t: Target) -> bool {
        matches!(
            t,
            Target::Tool(_)
                | Target::TextSize(_)
                | Target::LineWidth
                | Target::Reading
                | Target::Width(_)
                | Target::SelBar(_)
                | Target::ToggleToolbar
                | Target::ToggleSidebar
                | Target::Search
                | Target::NewNote
                | Target::Highlights
                | Target::Trash
                | Target::ThemeCycle
                | Target::Options
                | Target::Row(_)
                | Target::Recent(_)
                | Target::Favorite(_)
                | Target::FoldFavorites
                | Target::FoldRecent
                | Target::Minimize
                | Target::Maximize
                | Target::Close
                | Target::SendTo
                | Target::Minimal
                | Target::ToMain
                | Target::Choice(..)
                | Target::OptionsClose
                | Target::Transfer(_)
                | Target::VaultOpen
                | Target::VaultShow
                | Target::SocialDrawer
                | Target::Social(_)
                | Target::SocialTab(_)
                | Target::SocialClose
                | Target::Update
                | Target::Model(crate::modelcard::Hit::Pick(_) | crate::modelcard::Hit::Download | crate::modelcard::Hit::NotNow | crate::modelcard::Hit::Cancel)
        )
    }

    fn overlap(a: Rect, b: Rect) -> f32 {
        (a.right.min(b.right) - a.left.max(b.left)).max(0.0) * (a.bottom.min(b.bottom) - a.top.max(b.top)).max(0.0)
    }

    /// With the OS's own window buttons (the Mac's traffic lights, on the
    /// left), the quick actions sit at the window's right edge, not
    /// 138 pixels short of it where Windows' buttons would have been.
    #[test]
    fn the_quick_actions_take_the_window_buttons_room_when_the_os_draws_them() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        for width in [360.0, 700.0, 1400.0] {
            let at = |native_caption| {
                let input = Input { page_fade: 1.0, native_caption, ..Default::default() };
                let scene = scene::build(&eng, &fonts, &input, width, 700.0).unwrap();
                (rect_of(&scene, Target::SendTo).unwrap(), rect_of(&scene, Target::Minimal).unwrap(), rect_of(&scene, Target::Close))
            };
            let (send, minimal, close) = at(true);
            assert!(close.is_none(), "the OS draws the close button");
            assert!(minimal.right >= width - 1.0 && minimal.right <= width + 1.0, "Minimal ends at the window's edge at {width}: {minimal:?}");
            assert!(send.right <= minimal.left + 1.0, "Send beside it at {width}");
            let (_, own, close) = at(false);
            assert!(own.right <= width - 3.0 * CAPTION_WIDTH && close.is_some(), "Windows' own buttons keep their room at {width}");
        }
    }

    /// Every size a window can be, in every state its panels can be in:
    /// each control inside the window and clear of the others, the note
    /// readable beside the docked panels, the toolbar's buttons there.
    /// New note comes first, under the search, as a filled button; no
    /// platform's menu has a Highlights row (the palette has it).
    #[test]
    fn new_note_sits_under_the_search_and_highlights_is_not_in_the_menu() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let input = Input { vault_name: "notes", page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, ..Default::default() };
        let scene = scene::build(&eng, &fonts, &input, 1400.0, 900.0).unwrap();
        let search = rect_of(&scene, Target::Search).unwrap();
        let new_note = rect_of(&scene, Target::NewNote).unwrap();
        let social = rect_of(&scene, Target::SocialDrawer).unwrap();
        assert!(new_note.top >= search.bottom && new_note.top - search.bottom < 12.0 && new_note.bottom <= social.top, "{search:?} {new_note:?} {social:?}");
        assert!(scene.fills.iter().any(|f| f.rect == new_note && f.ink == Ink::Raised), "a filled button, raised on the side menu");
        assert!(rect_of(&scene, Target::Highlights).is_none());
        let mac = scene::build(&eng, &fonts, &Input { platform: Platform::Mac, ..input }, 1400.0, 900.0).unwrap();
        assert!(rect_of(&mac, Target::Highlights).is_none(), "nor is the Mac's");
    }

    /// Revealed from focus mode, a panel slides in; a pointer that gets ahead
    /// of it is over it all the same, or it would hide under the pointer.
    #[test]
    fn a_panel_sliding_in_keeps_open_where_it_is_going() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let input = Input { vault_name: "notes", page_fade: 1.0, sidebar_dock: 0.0, sidebar_show: 0.3, toolbar_dock: 0.0, toolbar_show: 0.3, ..Default::default() };
        let scene = scene::build(&eng, &fonts, &input, 1400.0, 900.0).unwrap();
        // Where the side menu's middle will be, and the toolbar's.
        assert_eq!(scene::hit(&scene.hits, MARGIN + 150.0, 400.0).and_then(Target::panel), Some(scene::Panel::Sidebar));
        let bar = scene.hits.iter().filter(|(_, t)| *t == Target::ToolbarPanel).map(|(r, _)| *r).fold(None, |a: Option<Rect>, r| Some(a.map_or(r, |a| if r.height() > a.height() { r } else { a })));
        let bar = bar.expect("the toolbar's keep-open area");
        assert!(bar.bottom >= MARGIN + TOOLBAR_HEIGHT, "it reaches where the bar settles: {bar:?}");
    }

    #[test]
    fn favorites_then_recent_then_the_tree_with_no_vault_label() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let rows = vec![SidebarRow { depth: 0, kind: RowKind::Note, name: "Inbox".into() }];
        let recent: Vec<String> = ["Plan.md", "a.md", "b.md", "c.md", "d.md", "e.md"].map(String::from).to_vec();
        let favorites: Vec<String> = ["Work/", "Plan.md"].map(String::from).to_vec();
        use kolada_core::session::{FOLD_RECENT, HIDE_RECENT, MANY_NOTES};
        // A vault of more than 25 notes, unless a test says otherwise.
        let build = |recent: &[String], favorites: &[String], folded: u8, height: f32| {
            let input = Input { vault_name: "My vault", active_note: "Plan", sidebar: &rows, recent, favorites, folded: folded | MANY_NOTES, page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, ..Default::default() };
            scene::build(&eng, &fonts, &input, 1200.0, height).unwrap()
        };
        let top_of = |scene: &Scene, pick: &dyn Fn(&Target) -> bool| scene.hits.iter().find(|(_, t)| pick(t)).map(|(r, _)| r.top);
        let scene = build(&recent, &favorites, 0, 900.0);
        let fav = top_of(&scene, &|t| matches!(t, Target::Favorite(_))).unwrap();
        let rec = top_of(&scene, &|t| matches!(t, Target::Recent(_))).unwrap();
        let tree = top_of(&scene, &|t| matches!(t, Target::Row(_))).unwrap();
        assert!(fav < rec && rec < tree, "favorites, then recent, then the tree");
        let picked = |scene: &Scene, pick: &dyn Fn(&Target) -> Option<usize>| scene.hits.iter().filter_map(|(_, t)| pick(t)).collect::<Vec<_>>();
        assert_eq!(picked(&scene, &|t| if let Target::Recent(i) = t { Some(*i) } else { None }), [1, 2, 3, 4, 5], "five, not the open note");
        assert_eq!(picked(&scene, &|t| if let Target::Favorite(i) = t { Some(*i) } else { None }), [0, 1], "a favorite stays even when open");
        let marks: Vec<Logo> = scene.vectors.iter().map(|v| v.logo).collect();
        assert!(marks.contains(&Logo::Star) && marks.contains(&Logo::Recent) && marks.contains(&Logo::Folder) && marks.contains(&Logo::Note));
        // New note: Basil's mark, centred in its button's icon box, on
        // every platform.
        let new_note = rect_of(&scene, Target::NewNote).unwrap();
        let mark = scene.vectors.iter().find(|v| v.logo == Logo::NewNote).expect("New note's mark").rect;
        assert!((mark.width() - NEW_NOTE_MARK).abs() < 0.01 && (mark.height() - NEW_NOTE_MARK).abs() < 0.01);
        assert!(mark.left >= new_note.left && mark.right <= new_note.left + 32.0, "where the glyph was");
        assert!(((mark.top + mark.bottom) / 2.0 - (new_note.top + new_note.bottom) / 2.0).abs() < 0.5, "centred in the button");
        for platform in [Platform::Mac, Platform::Linux, Platform::Ios] {
            let input = Input { platform, vault_name: "My vault", active_note: "Plan", sidebar: &rows, recent: &recent, favorites: &favorites, folded: MANY_NOTES, page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, ..Default::default() };
            let other = scene::build(&eng, &fonts, &input, 1200.0, 900.0).unwrap();
            assert!(other.vectors.iter().any(|v| v.logo == Logo::NewNote), "{platform:?} draws it too");
        }
        let unnamed = { let input = Input { vault_name: "", active_note: "Plan", sidebar: &rows, recent: &recent, favorites: &favorites, folded: MANY_NOTES, page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, ..Default::default() }; scene::build(&eng, &fonts, &input, 1200.0, 900.0).unwrap() };
        assert_eq!(scene.texts.len(), unnamed.texts.len(), "no vault label above the tree");
        // Folded, a section keeps its heading and lets its rows go.
        let shut = build(&recent, &favorites, FOLD_RECENT, 900.0);
        assert!(picked(&shut, &|t| if let Target::Recent(i) = t { Some(*i) } else { None }).is_empty());
        assert!(shut.hits.iter().any(|(_, t)| *t == Target::FoldRecent));
        // A short window keeps room for the tree; nothing kept, no headings.
        let short = build(&recent, &favorites, 0, 520.0);
        assert!(picked(&short, &|t| if let Target::Recent(i) = t { Some(*i) } else { None }).len() < 5);
        let none = build(&[], &[], 0, 900.0);
        assert!(!none.hits.iter().any(|(_, t)| matches!(t, Target::FoldFavorites | Target::FoldRecent)));
        // Hidden from the right-click menu: no heading, no rows.
        let hidden = build(&recent, &favorites, HIDE_RECENT, 900.0);
        assert!(!hidden.hits.iter().any(|(_, t)| matches!(t, Target::FoldRecent | Target::Recent(_))));
        // 25 notes or fewer: the tree alone (newest edited first) after the favorites.
        let few = { let input = Input { vault_name: "My vault", active_note: "Plan", sidebar: &rows, recent: &recent, favorites: &favorites, page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, ..Default::default() }; scene::build(&eng, &fonts, &input, 1200.0, 900.0).unwrap() };
        assert!(!few.hits.iter().any(|(_, t)| matches!(t, Target::FoldRecent | Target::Recent(_))), "no Recent in a small vault");
        assert!(few.hits.iter().any(|(_, t)| matches!(t, Target::Favorite(_))), "favorites stay");
    }

    #[test]
    fn holds_together_at_every_window_size() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let rows = vec![
            SidebarRow { depth: 0, kind: RowKind::Folder { expanded: true }, name: "Work".into() },
            SidebarRow { depth: 1, kind: RowKind::Note, name: "Plan".into() },
            SidebarRow { depth: 0, kind: RowKind::Note, name: "Inbox".into() },
        ];
        // Recent above the tree: the open note is left out of it.
        let recent: Vec<String> = ["Work/Plan.md", "Inbox.md", "Moving flat.md", "A note with a very long name that goes on.md"].map(String::from).to_vec();
        let favorites: Vec<String> = ["Work/", "Inbox.md", "A favorite with a long name that goes on and on.md"].map(String::from).to_vec();
        let text = "# Plan
Ship it.";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        // (name, side menu (dock, show) where it fits, toolbar (dock, show),
        // options open, the side menu's width as dragged (0: its own))
        let states = [
            ("docked", (1.0, 1.0), (1.0, 1.0), false, 0.0),
            ("tucked", (1.0, 1.0), (0.0, 0.0), false, 0.0),
            ("focus", (0.0, 0.0), (0.0, 0.0), false, 0.0),
            // One panel revealed at a time: each hides as the pointer leaves.
            ("menu revealed", (0.0, 1.0), (0.0, 0.0), false, 0.0),
            ("bar revealed", (0.0, 0.0), (0.0, 1.0), false, 0.0),
            ("options", (1.0, 1.0), (1.0, 1.0), true, 0.0),
            // The Mac's own options (fewer rows), with the OS's window buttons.
            ("options mac", (1.0, 1.0), (1.0, 1.0), true, 0.0),
            ("options ios", (1.0, 1.0), (1.0, 1.0), true, 0.0),
            ("docked ios", (1.0, 1.0), (1.0, 1.0), false, 0.0),
            // The Mac's traffic lights float over the top left: nothing of
            // ours goes under them, in any state.
            ("docked mac", (1.0, 1.0), (1.0, 1.0), false, 0.0),
            ("docked narrow mac", (1.0, 1.0), (1.0, 1.0), false, SIDEBAR_LEAST),
            ("tucked mac", (1.0, 1.0), (0.0, 0.0), false, 0.0),
            ("focus mac", (0.0, 0.0), (0.0, 0.0), false, 0.0),
            ("menu revealed mac", (0.0, 1.0), (0.0, 0.0), false, 0.0),
            ("bar revealed mac", (0.0, 0.0), (0.0, 1.0), false, 0.0),
            ("docked narrow", (1.0, 1.0), (1.0, 1.0), false, SIDEBAR_LEAST),
            ("docked wide", (1.0, 1.0), (1.0, 1.0), false, SIDEBAR_MOST),
            ("revealed wide", (0.0, 1.0), (0.0, 0.0), false, SIDEBAR_MOST),
            // Minimal: the note alone, its panels tucked away.
            ("minimal", (0.0, 0.0), (0.0, 0.0), false, 0.0),
            // The iPhone's native shell: the note alone, none of our chrome.
            ("bare ios", (0.0, 0.0), (0.0, 0.0), false, 0.0),
            // Linux: the chrome's own window buttons, as on Windows, with
            // Linux's options (fewer rows) and Ctrl keys.
            ("docked linux", (1.0, 1.0), (1.0, 1.0), false, 0.0),
            ("tucked linux", (1.0, 1.0), (0.0, 0.0), false, 0.0),
            ("options linux", (1.0, 1.0), (1.0, 1.0), true, 0.0),
        ];
        let mut checked = 0;
        for height in [320.0, 400.0, 600.0, 900.0, 1400.0] {
            for width in (360..=2600).step_by(16).map(|w| w as f32) {
                for (state, side, top, options_open, set) in states {
                    let side = if side.0 > 0.0 && !sidebar_fits(width) { (0.0, 0.0) } else { side };
                    let make = |social: Option<crate::social::SocialView>| Input {
                        vault_name: "notes",
                        active_note: "Work/Plan",
                        text,
                        sidebar: &rows,
                        lines: &lines,
                        options_open,
                        page_fade: 1.0,
                        sidebar_dock: side.0,
                        sidebar_show: side.1,
                        toolbar_dock: top.0,
                        toolbar_show: top.1,
                        social_drawer: true,
                        social,
                        // The dates with the teal dot beside them: it too holds.
                        page_date: Some("Created 4 Oct \u{00B7} 375 words"),
                        due_now: 2,
                        recent: &recent,
                        favorites: &favorites,
                        folded: kolada_core::session::MANY_NOTES,
                        sidebar_width: set,
                        compact: matches!(state, "minimal" | "bare ios"),
                        bare: state == "bare ios",
                        no_margin: matches!(state, "minimal" | "bare ios"),
                        platform: if state.ends_with(" mac") {
                            Platform::Mac
                        } else if state.ends_with(" ios") {
                            Platform::Ios
                        } else if state.ends_with(" linux") {
                            Platform::Linux
                        } else {
                            Platform::Windows
                        },
                        native_caption: state.ends_with(" mac"),
                        // An update ready: its pill above the side menu's
                        // icons, and the switch in the options (Windows' own).
                        updater: !state.ends_with(" mac") && !state.ends_with(" ios") && !state.ends_with(" linux"),
                        update: Some("0.10.0"),
                        // Beam's row and button with a count, where it runs.
                        quick: (!state.ends_with(" ios") && !state.ends_with(" linux")).then_some(scene::BeamView { unseen: 12, line: "From iPhone, 2 h ago", typing: false, menu: None, check: None, keys: "Alt+Z" }),
                        // Show Markdown on in one state: the heading's #s
                        // show at the caret; clean editing in the rest.
                        show_marks: state == "docked wide",
                        ..Default::default()
                    };
                    let input = make(None);
                    let menu = menu_width(set, width);
                    if state == "bare ios" {
                        let scene = scene::build(&eng, &fonts, &input, width, height).unwrap();
                        for t in [Target::SidebarPanel, Target::RevealToolbar, Target::ToggleToolbar, Target::SendTo, Target::Minimal, Target::ToMain, Target::Caption, Target::Close] {
                            assert!(rect_of(&scene, t).is_none(), "the bare note draws no {t:?}, {state} at {width} x {height}");
                        }
                        assert!(scene.pills.iter().all(Option::is_none), "nor pills, {state} at {width} x {height}");
                        assert_eq!(scene::hit(&scene.hits, width / 2.0, 4.0), Some(Target::Note), "its top is the note's, {state} at {width} x {height}");
                    } else if state == "minimal" {
                        let scene = scene::build(&eng, &fonts, &input, width, height).unwrap();
                        assert!(rect_of(&scene, Target::SidebarPanel).is_none(), "the minimal window has no side menu, {state} at {width} x {height}");
                        assert!(scene.pills[1].is_none() && rect_of(&scene, Target::RevealToolbar).is_none() && rect_of(&scene, Target::ToggleToolbar).is_none(), "the minimal window has no toolbar, {state} at {width} x {height}");
                        let (grow, send) = (rect_of(&scene, Target::ToMain).expect("its way back"), rect_of(&scene, Target::SendTo).unwrap());
                        assert!(grow.left >= send.right - 0.5 && rect_of(&scene, Target::Minimal).is_none(), "it grows back beside Send, {state} at {width} x {height}");
                    } else if state.ends_with(" ios") && !options_open {
                        let scene = scene::build(&eng, &fonts, &input, width, height).unwrap();
                        assert!(rect_of(&scene, Target::Minimal).is_none() && rect_of(&scene, Target::SendTo).is_some(), "an iPad has Send and no minimal window, {state} at {width} x {height}");
                        assert!(rect_of(&scene, Target::SocialDrawer).is_none(), "nor the social drawer, {state} at {width} x {height}");
                    } else if !options_open {
                        let scene = scene::build(&eng, &fonts, &input, width, height).unwrap();
                        let (shrink, send) = (rect_of(&scene, Target::Minimal).expect("the minimal button"), rect_of(&scene, Target::SendTo).unwrap());
                        assert!(shrink.left >= send.right - 0.5 && shrink.right <= width, "Minimal beside Send, {state} at {width} x {height}");
                    }
                    let at = format!("{state} at {width} x {height}");
                    let scene = scene::build(&eng, &fonts, &input, width, height).unwrap();
                    // Settings > Devices, scrolled to: a long name, each
                    // mark, and this one being renamed; every control in
                    // the panel and apart.
                    if options_open {
                        use crate::quick::{DeviceRow, Hit as Q, Look, Pairing};
                        let devices = [
                            DeviceRow { name: "A computer with a very long name indeed", look: Look::Laptop, this: true },
                            DeviceRow { name: "iPhone", look: Look::Phone, this: false },
                            DeviceRow { name: "Studio desktop with a long name as well", look: Look::Desktop, this: false },
                        ];
                        for naming in [None, Some("Kitchen laptop")] {
                            let mut with = Input { devices: Some(scene::DevicesView { devices: &devices, pairing: Pairing::Idle, said: None, facts: &[], this: "PC", naming }), ..make(None) };
                            with.options_scroll = devices_scroll(&with, width);
                            let s = scene::build(&eng, &fonts, &with, width, height).unwrap();
                            let panel = rect_of(&s, Target::OptionsPanel).expect("the options");
                            let parts: Vec<(Rect, Target)> = s.hits.iter().copied().filter(|(_, t)| matches!(t, Target::Quick(Q::Rename | Q::NameField | Q::NameSave | Q::NameCancel | Q::Remove(_) | Q::ShowCode | Q::EnterCode))).collect();
                            assert!(parts.iter().any(|(_, t)| *t == Target::Quick(if naming.is_some() { Q::NameSave } else { Q::Rename })), "this one's {naming:?}, {at}");
                            for (i, (a, ta)) in parts.iter().enumerate() {
                                assert!(a.left >= panel.left - 0.5 && a.right <= panel.right + 0.5 && a.width() > 4.0, "{ta:?} {a:?} outside the options, {at}");
                                for (b, tb) in &parts[i + 1..] {
                                    assert!(overlap(*a, *b) <= 1.0, "{ta:?} overlaps {tb:?}, {at}");
                                }
                            }
                            for t in s.texts.iter().filter(|t| devices.iter().any(|d| d.name == t.layout.text()) || t.layout.text() == "This one") {
                                assert!(t.x + t.layout.measure().width <= panel.right + 1.0, "{:?} past the options, {at}", t.layout.text());
                            }
                        }
                    }
                    // The notes as a grid under the note, in each of its
                    // states: the boxes on the sheet, apart and whole, their
                    // words inside them; browsing, the bar's search and
                    // icons in the top row, clear of the window's buttons
                    // (and of the Mac's lights).
                    // (Every third width, the shortest window and a common
                    // one: each box's words are laid out anew at each width.)
                    if matches!(state, "docked" | "docked mac") && matches!(height as u32, 320 | 900) && (width as u32 - 360) % 48 == 0 {
                        use crate::grid::{self, BoxView, Grid, GridView, Phase};
                        use crate::preview::preview;
                        let kinds = [
                            preview("Groceries", "Groceries
Oat milk, eggs and a loaf of the good bread from the corner
"),
                            preview("Plan", "Plan
- [x] Book rooms
  - [ ] Pay the deposit before Friday
1. Pack
"),
                            preview("Costs", "Costs
| Item | Cost | Qty | Note |
| --- | --- | --- | --- |
| Tea | 3 | 1 | x |
| Milk | 2 | 2 | y |
"),
                            preview("A note whose name goes on far longer than any box is wide", "x
![[sea.png]]
"),
                        ];
                        // The boxes' own words (Beam's button floats over them).
                        let ours = |t: &str| {
                            t == "Unread"
                                || kinds.iter().any(|k| {
                                    k.title == t
                                        || t.lines().all(|l| k.lines.iter().any(|w| w == l))
                                        || match &k.rich {
                                            Some(crate::preview::Rich::Table(rows)) => rows.iter().flatten().any(|c| c == t),
                                            Some(crate::preview::Rich::List(items)) => items.iter().any(|i| i.text == t || matches!(&i.mark, crate::preview::Mark::Number(n) if n == t)),
                                            _ => false,
                                        }
                                })
                        };
                        let boxes: Vec<BoxView> = (0..40).map(|i| BoxView { name: "Unread", preview: (i % 5 != 4).then(|| &kinds[i % 4]), picture: None, snippet: None }).collect();
                        let typed = grid::GridSearch { query: "plan", caret: Some(4) };
                        let searches = [None, Some((typed, false)), Some((typed, true))];
                        for (phase, search) in [Phase::Rest, Phase::Writing, Phase::Peek, Phase::Browse].into_iter().flat_map(|p| searches.map(|s| (p, s))) {
                            // Browsed while a search is typed; nothing found: no boxes.
                            if search.is_some() && phase != Phase::Browse {
                                continue;
                            }
                            let none_found = search.is_some_and(|(_, empty)| empty);
                            let g = Grid::at(phase);
                            let boxes = if none_found { &boxes[..0] } else { &boxes[..] };
                            let shown = grid::shown(&g, boxes.len(), width, height);
                            let chosen = search.filter(|_| !none_found).map(|_| shown.start);
                            let view = GridView { grid: &g, count: boxes.len(), first: shown.start, boxes: &boxes[shown.clone()], search: search.map(|(s, _)| s), chosen };
                            // Even with the shell asking for it (docked, out, searching), no side menu.
                            let searching = search.map(|_| scene::SearchView { query: "plan", caret: 4, hits: &[], selected: 0, first: 0, commands: None, focused: true });
                            let quick = scene::BeamView { unseen: 3, line: "", typing: false, menu: None, check: None, keys: "Alt+Z" };
                            let s = scene::build(&eng, &fonts, &Input { grid: Some(view), sidebar_dock: 1.0, sidebar_show: 1.0, search: searching, quick: Some(quick), ..make(None) }, width, height).unwrap();
                            let at = format!("grid {phase:?} {}, {at}", match search { None => "", Some((_, false)) => "searching", Some(_) => "nothing found" });
                            assert!(rect_of(&s, Target::SidebarPanel).is_none(), "the grid is a mode of its own: no side menu, {at}");
                            assert!(!s.hits.iter().any(|(_, t)| matches!(t, Target::Row(_) | Target::Result(_) | Target::Recent(_) | Target::Favorite(_))), "nothing of the side menu's, {at}");
                            for t in [Target::Search, Target::NewNote, Target::ViewToggle, Target::Trash, Target::Options, Target::Quick(crate::quick::Hit::Fab)] {
                                assert!(s.hits.iter().filter(|(_, h)| *h == t).count() <= 1, "{t:?} twice, {at}");
                            }
                            let sheet = rect_of(&s, Target::GridSheet);
                            if phase == Phase::Writing {
                                assert!(sheet.is_none() && shown.is_empty(), "writing, the boxes are out of sight and unread, {at}");
                                continue;
                            }
                            let sheet = sheet.expect("the sheet");
                            assert_eq!(sheet.top, (phase.note_share() * height).round(), "{at}");
                            // Beam's round button would cover the boxes: it is the bar's.
                            if let Some(fab) = rect_of(&s, Target::Quick(crate::quick::Hit::Fab)) {
                                assert!(fab.bottom <= grid::shelf(sheet).top + 0.5, "Beam's button {fab:?} over the boxes, {at}");
                            }
                            // The panel: a step off the page, its edge with the sheet's.
                            let panel = grid::panel(sheet);
                            // The note above it ends softly, never sliced by the panel's edge.
                            assert!(s.fills.iter().any(|f| f.ink == Ink::Page && f.blur > 0.0 && f.rect.top < sheet.top && f.rect.bottom > sheet.top), "the note fades into the panel, {at}");
                            assert!(s.fills.iter().any(|f| f.ink == Ink::Side && (f.rect.top - sheet.top).abs() < 0.5 && (f.rect.left - panel.left).abs() < 0.5 && f.radius >= 12.0), "the filled panel at the sheet's edge, {at}");
                            let placed: Vec<Rect> = s.hits.iter().filter(|(_, t)| matches!(t, Target::GridBox(_))).map(|(r, _)| *r).collect();
                            if none_found {
                                assert!(placed.is_empty(), "{at}");
                                let quiet = s.texts.iter().filter(|t| t.layout.text() == "No notes match").count();
                                assert!(quiet == 1 || sheet.height() < 110.0, "one quiet line, {quiet}, {at}");
                                continue;
                            }
                            assert!(!placed.is_empty(), "boxes in sight, {at}");
                            for (i, a) in placed.iter().enumerate() {
                                assert!(a.left >= 0.0 && a.right <= width && a.top >= sheet.top && a.bottom <= height + 0.5, "a box {a:?} off the sheet, {at}");
                                assert!(a.width() >= 120.0, "a box too narrow to read ({}), {at}", a.width());
                                assert!(placed[i + 1..].iter().all(|b| overlap(*a, *b) <= 0.5), "boxes overlap, {at}");
                            }
                            // The note starts where the boxes do (this note reads left to right).
                            let a = grid::across(width);
                            assert!((s.column.left - a.left).abs() <= 1.0 && s.column.right <= a.right() + 1.0, "the note at the grid's left edge ({:?}, {a:?}), {at}", s.column);
                            assert!((2..=5).contains(&a.cols) && a.width > a.height, "cards, {a:?}, {at}");
                            for (k, _) in boxes[shown.clone()].iter().enumerate() {
                                let r = grid::place(sheet, shown.start + k, 0.0);
                                for t in s.texts[s.chrome_from.1..].iter().filter(|t| t.y >= r.top && t.y < r.bottom && t.x >= r.left && t.x < r.right && ours(t.layout.text())) {
                                    assert!(t.x + t.layout.measure().width <= r.right + 1.0, "{:?} runs out of its box, {at}", t.layout.text());
                                }
                            }
                            // The bar's own, the last of each (others may keep a target out of sight).
                            let bar = [Target::Search, Target::ViewToggle, Target::ThemeCycle, Target::Trash, Target::Options, Target::NewNote, Target::Quick(crate::quick::Hit::Fab)].map(|t| s.hits.iter().rev().find(|(r, h)| *h == t && r.left > -1e6).map(|(r, _)| *r));
                            // Search and the icons ride the panel's top edge, over
                            // none of the boxes; the note above stays clean.
                            if phase == Phase::Browse && sheet.top - 8.0 > MARGIN + TOOLBAR_HEIGHT {
                                assert!(scene::hit(&s.hits, width / 2.0, sheet.top - 8.0) == Some(Target::GridNote), "the note above the boxes goes back to it, {at}");
                            }
                            if sheet.height() < 60.0 {
                                continue;
                            }
                            // Every action, at every width; narrow, a search
                            // being typed takes the bar (Esc gives it back).
                            let narrow_typing = search.is_some() && bar[1].is_none();
                            assert!(bar[0].is_some(), "the search, always, {at}");
                            if !narrow_typing {
                                assert!(bar.iter().all(Option::is_some), "search, a new note, every icon and Beam, {at}: {bar:?}");
                            }
                            let shelf = grid::shelf(sheet);
                            let panel = grid::panel(sheet);
                            let shown_bar: Vec<Rect> = bar.iter().flatten().copied().collect();
                            for (i, b) in shown_bar.iter().enumerate() {
                                assert!(b.left >= panel.left - 0.5 && b.right <= panel.right + 0.5 && b.top >= sheet.top && b.bottom <= shelf.top, "the bar's {b:?} off the panel's top edge, {at}");
                                assert!(shown_bar[i + 1..].iter().all(|c| overlap(*b, *c) <= 0.5), "the bar's parts overlap, {at}");
                                assert!(placed.iter().all(|r| overlap(*b, *r) <= 0.5), "the bar over a box, {at}");
                            }
                        }
                    }
                    // Settings' font and features: a long name of the
                    // machine's chosen, Beam off; and the finder open with
                    // one found. Every control in the panel and apart.
                    if options_open {
                        let long = "A font with a very long family name indeed";
                        let names = [long, "Cambria", "Candara"];
                        let finder = scene::FontFinder { query: "Ca", caret: 2, names: &names, picked: Some(1), total: 3 };
                        // Beam's rows folding away too, half way.
                        for (finder, font, folding) in [(None, scene::INSTALLED_FONT, None), (Some(finder), 0, Some((Setting::Beam, 0.5)))] {
                            let mut options = [0; scene::SETTINGS];
                            options[Setting::Font.index()] = font;
                            options[Setting::Beam.index()] = 1;
                            let with = Input { options, font_name: Some(long), font_finder: finder, folding, ..make(None) };
                            let s = scene::build(&eng, &fonts, &with, width, height).unwrap();
                            let panel = rect_of(&s, Target::OptionsPanel).expect("the options");
                            let parts: Vec<(Rect, Target)> = s.hits.iter().copied().filter(|(_, t)| matches!(t, Target::Choice(..) | Target::Switch(_) | Target::FontOther | Target::FontName(_))).collect();
                            // (iOS has none: the system's Reduce Motion stands for it.)
                            assert!(parts.iter().any(|(_, t)| *t == Target::Switch(Setting::Motion)) || height < 900.0 || state.ends_with(" ios"), "the switches, {at}");
                            for (i, (a, ta)) in parts.iter().enumerate() {
                                assert!(a.left >= panel.left - 0.5 && a.right <= panel.right + 0.5 && a.width() > 4.0, "{ta:?} {a:?} outside the options, {at}");
                                for (b, tb) in &parts[i + 1..] {
                                    assert!(overlap(*a, *b) <= 1.0, "{ta:?} overlaps {tb:?}, {at}");
                                }
                            }
                            for t in s.texts.iter().filter(|t| names.contains(&t.layout.text())) {
                                assert!(t.x + t.layout.measure().width <= panel.right + 1.0, "{:?} past the options, {at}", t.layout.text());
                            }
                        }
                    }
                                        // The social preview floats over it all: inside the
                    // window, below its buttons, its tabs apart.
                    if state == "docked" {
                        let view = crate::social::SocialView { platform: kolada_core::social::Platform::LinkedIn, scroll: 0.0, offset: (0.0, 0.0) };
                        let over = scene::build(&eng, &fonts, &make(Some(view)), width, height).unwrap();
                        let card = rect_of(&over, Target::SocialCard).expect("the preview");
                        assert!(card.left >= 0.0 && card.right <= width && card.bottom <= height && card.top >= CAPTION_BOTTOM, "the preview leaves the window, {at}");
                        // Dragged far off, any way: it stays in the window.
                        for offset in [(-9000.0, 9000.0), (9000.0, -9000.0), (-9000.0, -9000.0), (9000.0, 9000.0)] {
                            let far = crate::social::SocialView { offset, ..view };
                            let moved = scene::build(&eng, &fonts, &make(Some(far)), width, height).unwrap();
                            let r = rect_of(&moved, Target::SocialCard).unwrap();
                            assert!(r.left >= 0.0 && r.right <= width && r.bottom <= height + 0.5 && r.top >= CAPTION_BOTTOM, "the dragged preview leaves the window ({r:?}), {at}");
                            assert!(rect_of(&moved, Target::SocialClose).is_some_and(|c| c.bottom <= height), "its close button out of reach, {at}");
                        }
                        let tabs: Vec<Rect> = over.hits.iter().filter(|(_, t)| matches!(t, Target::SocialTab(_) | Target::SocialClose)).map(|(r, _)| *r).collect();
                        assert_eq!(tabs.len(), 6, "{at}");
                        for (i, a) in tabs.iter().enumerate() {
                            assert!(a.right <= card.right && a.left >= card.left, "a tab outside the preview, {at}");
                            assert!(tabs[i + 1..].iter().all(|b| overlap(*a, *b) <= 1.0), "the preview's tabs overlap, {at}");
                        }
                    }
                    // The time scrubber: its dock in the window, its parts in
                    // it and apart, a line's offer to come back in reach.
                    if matches!(state, "docked" | "focus") {
                        let (h, m) = (3_600_000, 60_000);
                        let (times, changes, changed) = ([0, m, 4 * h, 4 * h + m], [40, 3, 120, 9], [true, false]);
                        let view = crate::scrub::ScrubView {
                            times: &times,
                            changes: &changes,
                            shown: 1,
                            head: 1.0,
                            when: "Tue 29 Sep, 11:20 AM",
                            ago: "5 days ago",
                            since: "28 Sep",
                            changed: &changed,
                            past: 1.0,
                            alpha: 1.0,
                            hover_line: Some(0),
                        };
                        let past = scene::build(&eng, &fonts, &Input { scrub: Some(view), ..make(None) }, width, height).unwrap();
                        let dock = rect_of(&past, Target::ScrubPanel).expect("the scrubber's dock");
                        assert!(dock.left >= 0.0 && dock.right <= width && dock.top >= 0.0 && dock.bottom <= height, "the scrubber leaves the window ({dock:?}), {at}");
                        let part = |t: Target| rect_of(&past, t).unwrap_or_else(|| panic!("{t:?} missing, {at}"));
                        let (track, restore, close) = (part(Target::ScrubTrack), part(Target::ScrubRestore), part(Target::ScrubClose));
                        for (r, t) in [(track, "track"), (restore, "Restore"), (close, "close")] {
                            assert!(r.left >= dock.left && r.right <= dock.right && r.width() > 20.0, "the scrubber's {t} ({r:?}) outside its dock, {at}");
                        }
                        assert!(overlap(track, restore) <= 1.0 && overlap(restore, close) <= 1.0, "the scrubber's parts overlap, {at}");
                        let pill = part(Target::BringBack(0));
                        assert!(pill.left >= 0.0 && pill.right <= width, "Bring back out of reach ({pill:?}), {at}");
                    }
                    // The link field under the caret, with Remove and
                    // suggestions, and the address card under a pointer
                    // anywhere: in the window, their parts in the card.
                    if matches!(state, "docked" | "focus" | "minimal" | "docked narrow mac") {
                        let names: Vec<(String, String)> = vec![
                            ("Q3 plan".into(), "Work".into()),
                            ("A note with a very long name that goes on and on and on".into(), String::new()),
                            ("Plans".into(), "Archive/Old".into()),
                        ];
                        // On a link (its actions, then editing it) and a new one.
                        for (existing, editing, caret_at, parts_shown) in [(true, false, 0usize, 5), (true, true, 0, 3 + names.len()), (false, true, text.len(), 1 + names.len())] {
                            let field = crate::scene::LinkFieldView { query: "https://kolada.app/a/very/long/address/that/goes/on", caret: 12, names: &names, picked: Some(1), existing, editing };
                            let with = scene::build(&eng, &fonts, &Input { link_field: Some(field), selection: (caret_at, caret_at), ..make(None) }, width, height).unwrap();
                            let card = rect_of(&with, Target::LinkPanel).unwrap_or_else(|| panic!("the link field, {at}"));
                            assert!(card.left >= 0.0 && card.right <= width && card.top >= 0.0 && card.bottom <= height, "the link field {card:?} leaves the window, {at}");
                            let parts: Vec<(Rect, Target)> = with
                                .hits
                                .iter()
                                .copied()
                                .filter(|(_, t)| matches!(t, Target::LinkField | Target::LinkRemove | Target::LinkSuggestion(_) | Target::LinkOpen | Target::LinkCopy | Target::LinkEdit | Target::LinkSave | Target::LinkCancel))
                                .collect();
                            assert_eq!(parts.len(), parts_shown, "{at} existing {existing} editing {editing}");
                            for (i, (r, t)) in parts.iter().enumerate() {
                                assert!(r.left >= card.left && r.right <= card.right && r.top >= card.top && r.bottom <= card.bottom, "{t:?} {r:?} outside the link field, {at}");
                                assert!(r.width() > 20.0 && r.height() > 20.0, "{t:?} squeezed, {at}");
                                assert!(parts[i + 1..].iter().all(|(o, _)| overlap(*r, *o) <= 1.0), "{t:?} overlaps, {at}");
                            }
                        }
                        for (x, y) in [(0.0, 0.0), (width, height), (width / 2.0, height / 2.0), (width - 2.0, 40.0)] {
                            let address = "https://kolada.app/a/very/long/address/that/goes/on/and/on/past/any/window/edge";
                            let with = scene::build(&eng, &fonts, &Input { link_card: Some((address, x, y)), ..make(None) }, width, height).unwrap();
                            let r = with.link_card.unwrap_or_else(|| panic!("the address card, {at}"));
                            assert!(r.left >= 0.0 && r.right <= width && r.top >= 0.0 && r.bottom <= height && r.width() > 40.0, "the address card {r:?} leaves the window, {at}");
                        }
                    }
                    // The command palette, with nothing typed and with more
                    // matches than it shows, and a long name with long keys:
                    // in the window, 16 DIPs from its edges at least, its
                    // field and rows inside it and apart.
                    if matches!(state, "docked" | "focus" | "minimal" | "docked mac" | "docked linux") {
                        let long = scene::MenuItem { label: "A command with a long name that goes on and on and on".into(), keys: "Ctrl+Shift+Alt+\u{2193}", ..scene::MenuItem::default() };
                        let rows: Vec<scene::MenuItem> = std::iter::once(long).chain((0..11).map(|i| scene::MenuItem { label: format!("Command {i}"), keys: "Ctrl+N", ..scene::MenuItem::default() })).collect();
                        for (shown, chosen, first) in [(0usize, 0usize, 0usize), (5, 4, 0), (12, 11, 4)] {
                            let view = crate::palette::PaletteView { query: if shown == 0 { "" } else { "comm" }, caret: 4, rows: &rows[..shown], first, chosen, drop: 1.0, height: crate::palette::rows_for(shown), fill: (chosen - first) as f32 };
                            let with = scene::build(&eng, &fonts, &Input { palette: Some(view), ..make(None) }, width, height).unwrap();
                            let card = rect_of(&with, Target::PalettePanel).unwrap_or_else(|| panic!("the palette, {at}"));
                            assert!(card.left >= 16.0 - 0.5 && card.right <= width - 16.0 + 0.5 && card.top >= 0.0 && card.bottom <= height, "the palette {card:?} leaves the window, {at}");
                            let parts: Vec<(Rect, Target)> = with.hits.iter().copied().filter(|(_, t)| matches!(t, Target::PaletteField | Target::PaletteRow(_))).collect();
                            let fit = ((card.bottom - card.top - 2.0 * crate::palette::PAD - crate::palette::FIELD) / crate::palette::ROW).max(0.0) as usize;
                            assert!(parts.len() >= 1 + shown.min(crate::palette::SHOWN).min(fit), "{} parts for {shown} rows, {at}", parts.len());
                            for (i, (r, t)) in parts.iter().enumerate() {
                                assert!(r.left >= card.left && r.right <= card.right && r.top >= card.top - 0.5 && r.bottom <= card.bottom + 0.5, "{t:?} {r:?} outside the palette, {at}");
                                assert!(parts[i + 1..].iter().all(|(o, _)| overlap(*r, *o) <= 1.0), "{t:?} overlaps, {at}");
                            }
                            for t in with.texts.iter().filter(|t| t.layout.text().starts_with("A command") || t.layout.text().starts_with("Ctrl+Shift+Alt")) {
                                assert!(t.x >= card.left && t.x + t.layout.measure().width <= card.right + 0.5, "{:?} past the palette, {at}", t.layout.text());
                            }
                        }
                    }
                    // A folded heading's mark in the window, whatever the
                    // margin.
                    if matches!(state, "docked" | "focus" | "minimal") {
                        let folded = [(text.find('\n').unwrap(), text.len())];
                        let with = scene::build(&eng, &fonts, &Input { sections_folded: &folded, ..make(None) }, width, height).unwrap();
                        let mark = with.vectors.iter().find(|v| v.logo == crate::vector::Logo::FoldShut).unwrap_or_else(|| panic!("the fold mark, {at}"));
                        assert!(mark.rect.left >= 0.0 && mark.rect.right <= width, "the fold mark {:?} leaves the window, {at}", mark.rect);
                    }
                    // The share card under Send, in each of its phases: in
                    // the window, under the button where there is room, its
                    // controls inside it and apart.
                    if matches!(state, "docked" | "minimal" | "docked mac") && (width as i32 - 360) % 48 == 0 {
                        use crate::share::{Card, Hit, Phase, ShareView};
                        use kolada_core::share::{Expiry, Link};
                        let now = 1_791_000_000;
                        let link = |id: &str, note: &str| Link { id: id.into(), created: now - 100, expires: now + 5 * 86_400, title: format!("A shared note with a long title {id}"), note: note.into() };
                        let two = [link("Aaaa", "Work/Plan.md"), link("Bbbb", "Inbox.md")];
                        let three = [link("Aaaa", "Work/Plan.md"), link("Bbbb", "Inbox.md"), link("Cccc", "Other.md")];
                        let mut cards = vec![Card::new(Expiry::Week, "Work/Plan.md", &two, now), Card::new(Expiry::Day, "Work/Plan.md", &three, now)];
                        cards[1].confirming = Some("Bbbb".into());
                        let mut failed = Card::new(Expiry::Month, "Work/Plan.md", &two, now);
                        failed.phase = Phase::Failed(kolada_core::share::Problem::TooBig { bytes: 1_400_000, limit: 1_000_000, pictures: true }.message());
                        let mut ready = Card::new(Expiry::Week, "Work/Plan.md", &two, now);
                        ready.phase = Phase::Ready { url: "https://notsy.co/Y5tq".into() };
                        cards.extend([failed, ready]);
                        let send = rect_of(&scene, Target::SendTo).unwrap();
                        for (k, card) in cards.iter().enumerate() {
                            let mut over = Scene { column: scene.column, ..Scene::default() };
                            over.hits.push((send, Target::SendTo));
                            crate::share::draw(&eng, &fonts, &ShareView { card, alpha: 1.0, now }, None, width, height, &mut over).unwrap();
                            let r = rect_of(&over, Target::Share(Hit::Card)).expect("the share card");
                            assert!(r.left >= 0.0 && r.right <= width && r.top >= 0.0 && r.bottom <= height, "the share card {k} leaves the window ({r:?}), {at}");
                            if height >= 600.0 {
                                // Lined up with Send's right edge, or against the window's left
                                // where there is not room for that.
                                assert!(r.top >= send.bottom && (r.right <= send.right + 0.5 || r.left <= 8.5), "the share card {k} not under Send, {at}");
                            }
                            let parts: Vec<(Rect, Target)> = over.hits.iter().copied().filter(|(_, t)| matches!(t, Target::Share(h) if !matches!(h, Hit::Shade | Hit::Card))).collect();
                            for (i, (a, ta)) in parts.iter().enumerate() {
                                assert!(a.left >= r.left && a.right <= r.right && a.top >= r.top && a.bottom <= r.bottom + 0.5 && a.width() > 4.0, "{ta:?} {a:?} outside the share card, {at}");
                                for (b, tb) in &parts[i + 1..] {
                                    assert!(overlap(*a, *b) <= 1.0, "{ta:?} overlaps {tb:?}, {at}");
                                }
                            }
                            let want = if matches!(card.phase, Phase::Ready { .. }) { Hit::CopyNew } else { Hit::Create };
                            assert!(rect_of(&over, Target::Share(want)).is_some(), "the share card {k} without {want:?}, {at}");
                            if height >= 400.0 {
                                assert!(rect_of(&over, Target::Share(Hit::Delete(card.links.len() - 1))).is_some(), "the share card {k} leaves a link out, {at}");
                            }
                            for t in &over.texts {
                                assert!(t.x >= r.left - 0.5 && t.x + t.layout.measure().width <= r.right + 1.0, "text past the share card {k}, {at}");
                            }
                        }
                    }
                    // The writing model's card in the note area's middle,
                    // then its strip at the foot: in the window,
                    // its controls in it and apart, clear of the chrome's.
                    if matches!(state, "docked" | "docked narrow" | "docked wide" | "focus" | "minimal" | "docked mac" | "options") && (width as i32 - 360) % 32 == 0 {
                        use crate::modelcard::{Choice, Hit, ModelCard, ModelCardView};
                        let mut card = ModelCard::new(vec![Choice { id: "light", name: "Light", bytes: 229_600_000 }, Choice { id: "full", name: "Full", bytes: 730_400_000 }]);
                        for (strip, downloading) in [(0.0, false), (1.0, true)] {
                            if downloading {
                                card.click(Hit::Pick(1));
                                card.click(Hit::Download);
                                card.progress(467_456_000, 730_400_000);
                            }
                            let view = || ModelCardView { card: &card, alpha: 1.0, strip, since: 10.0, glide: 1.0 };
                            let with = scene::build(&eng, &fonts, &Input { model_card: Some(view()), ..make(None) }, width, height).unwrap();
                            let phase = if downloading { "downloading" } else { "choosing" };
                            // The card's own (the veil behind it takes the same hit).
                            let r = with.hits.iter().rev().find(|(_, t)| *t == Target::Model(Hit::Card)).map(|(r, _)| *r).unwrap_or_else(|| panic!("the model card {phase}, {at}"));
                            assert!(r.left >= 0.0 && r.right <= width && r.top >= 0.0 && r.bottom <= height, "the model card {phase} leaves the window ({r:?}), {at}");
                            // The strip keeps to the note's part of the window (the card is over
                            // the whole of it, the page behind veiled).
                            if downloading && !options_open {
                                assert!(r.left >= content_left(&input, width), "the model card's strip is off the note's area ({r:?}), {at}");
                            }
                            let parts: Vec<(Rect, Target)> = with.hits.iter().copied().filter(|(_, t)| matches!(t, Target::Model(h) if *h != Hit::Card)).collect();
                            let want: &[Hit] = if downloading { &[Hit::Cancel] } else { &[Hit::Pick(0), Hit::Pick(1), Hit::Download, Hit::NotNow] };
                            assert_eq!(parts.iter().map(|(_, t)| *t).collect::<Vec<_>>(), want.iter().map(|h| Target::Model(*h)).collect::<Vec<_>>(), "the model card {phase}, {at}");
                            let chrome: Vec<(Rect, Target)> = with.hits.iter().copied().filter(|(_, t)| is_control(*t) && !matches!(t, Target::Model(_))).collect();
                            for (i, (a, ta)) in parts.iter().enumerate() {
                                assert!(a.left >= r.left && a.right <= r.right && a.top >= r.top && a.bottom <= r.bottom && a.width() >= 20.0 && a.height() >= 20.0, "{ta:?} {a:?} outside the model card, {at}");
                                for (b, tb) in &parts[i + 1..] {
                                    assert!(overlap(*a, *b) <= 1.0, "{ta:?} overlaps {tb:?}, {at}");
                                }
                                // Under the options' veil, the chrome is over it anyway; while
                                // choosing, the card's own veil is over the chrome.
                                if !options_open && downloading {
                                    for (b, tb) in &chrome {
                                        assert!(overlap(*a, *b) <= 1.0, "{ta:?} {a:?} under {tb:?} {b:?}, {at}");
                                    }
                                }
                            }
                            // Every word of it inside it.
                            let mut over = Scene::default();
                            let area = Rect::new(content_left(&input, width), 0.0, width, height);
                            crate::modelcard::draw(&eng, &fonts, &view(), None, Rect::new(0.0, 0.0, width, height), area, &mut over).unwrap();
                            assert_eq!(over.hits.iter().rev().find(|(_, t)| *t == Target::Model(Hit::Card)).map(|(r, _)| *r), Some(r));
                            for t in &over.texts {
                                let m = t.layout.measure();
                                assert!(t.x >= r.left && t.x + m.width <= r.right - 4.0 && t.y >= r.top && t.y + m.height <= r.bottom, "text past the model card {phase}, {at}");
                            }
                        }
                    }
                    // The toolbar's Aa card open under Aa with a button's name
                    // showing, and the selection bar over "Ship" with one of
                    // its names: each in the window, its parts in it and
                    // apart, clear of the toolbar's buttons.
                    if (width as i32 - 360) % 32 == 0 && !options_open && state != "minimal" {
                        let inside = |r: Rect| r.left >= 0.0 && r.right <= width && r.top >= 0.0 && r.bottom <= height;
                        if top.1 > 0.0 {
                            let with = scene::build(&eng, &fonts, &Input { reading: Some(1.0), tooltip: Some(Target::Tool(Tool::Bold)), ..make(None) }, width, height).unwrap();
                            let tip = with.tooltip.unwrap_or_else(|| panic!("Bold's name, {at}"));
                            assert!(inside(tip), "Bold's name {tip:?} leaves the window, {at}");
                            // The capsule (revealed, the keep-open area around it comes first).
                            let bar = with.hits.iter().rev().find(|(_, t)| *t == Target::ToolbarPanel).map(|(r, _)| *r).unwrap();
                            assert!(inside(bar) && (bar.height() - BAR).abs() < 0.01, "the toolbar {bar:?}, {at}");
                            if let Some(aa) = rect_of(&with, Target::Reading) {
                                let card = rect_of(&with, Target::ReadingPanel).unwrap_or_else(|| panic!("the Aa card, {at}"));
                                assert!(inside(card) && card.top >= bar.bottom, "the Aa card {card:?} out of place, {at}");
                                assert!(card.left <= aa.right && card.right >= aa.left, "the Aa card away from Aa, {at}");
                                let parts: Vec<Rect> = with.hits.iter().filter(|(_, t)| matches!(t, Target::TextSize(_) | Target::Width(_))).map(|(r, _)| *r).collect();
                                assert_eq!(parts.len(), 5, "{at}");
                                for (i, r) in parts.iter().enumerate() {
                                    assert!(r.left >= card.left && r.right <= card.right && r.top >= card.top && r.bottom <= card.bottom, "{r:?} outside the Aa card, {at}");
                                    assert!(parts[i + 1..].iter().all(|o| overlap(*r, *o) <= 0.0), "the Aa card's parts overlap, {at}");
                                }
                            }
                            // Its buttons on one row, a button apart, inside it.
                            let buttons: Vec<Rect> = with.hits.iter().filter(|(_, t)| matches!(t, Target::Tool(_) | Target::Reading | Target::ToggleToolbar)).map(|(r, _)| *r).collect();
                            for (i, b) in buttons.iter().enumerate() {
                                assert!((b.width() - BTN).abs() < 0.01 && (b.height() - BTN).abs() < 0.01 && (b.top - bar.top - INSET).abs() < 0.01, "{b:?} off the toolbar's row, {at}");
                                assert!(b.left >= bar.left + INSET - 0.01 && b.right <= bar.right - INSET + 0.01, "{b:?} out of the toolbar, {at}");
                                if let Some(n) = buttons.get(i + 1) {
                                    assert!(n.left - b.right >= BTN_GAP - 0.01, "toolbar buttons too close, {at}");
                                }
                            }
                        }
                        let ship = text.find("Ship").unwrap();
                        let with = scene::build(&eng, &fonts, &Input { selection: (ship, ship + 4), selection_bar: Some(1.0), tooltip: Some(Target::SelBar(Tool::Heading)), ..make(None) }, width, height).unwrap();
                        let sel = rect_of(&with, Target::SelBarPanel).unwrap_or_else(|| panic!("the selection bar, {at}"));
                        assert!(inside(sel) && sel.top >= with.column.top - 48.0 - 0.01, "the selection bar {sel:?} out of place, {at}");
                        let (first, _) = with.selection_span.unwrap();
                        assert!(sel.bottom <= first.top || sel.top >= first.bottom, "the selection bar over the words, {at}");
                        assert!(inside(with.tooltip.unwrap_or_else(|| panic!("Heading's name, {at}"))), "Heading's name leaves the window, {at}");
                        let bar_tools: Vec<Rect> = with.hits.iter().filter(|(_, t)| matches!(t, Target::Tool(_) | Target::Reading | Target::ToggleToolbar)).map(|(r, _)| *r).collect();
                        for b in with.hits.iter().filter(|(_, t)| matches!(t, Target::SelBar(_))).map(|(r, _)| *r) {
                            assert!(b.left >= sel.left && b.right <= sel.right, "{b:?} outside the selection bar, {at}");
                            assert!(bar_tools.iter().all(|t| overlap(*t, b) <= 0.0), "the selection bar under the toolbar, {at}");
                        }
                    }
                    // The update pill: in the side menu, above its icons,
                    // wherever the menu shows and is tall enough.
                    if let Some(pill) = rect_of(&scene, Target::Update) {
                        let panel = rect_of(&scene, Target::SidebarPanel).expect("the pill is in the side menu");
                        let icons = rect_of(&scene, Target::Trash).unwrap();
                        assert!(pill.left >= panel.left && pill.right <= panel.right && pill.bottom <= icons.top, "the update pill {pill:?} out of place, {at}");
                    } else if side.1 > 0.0 && height >= 600.0 && !options_open {
                        panic!("no update pill in a tall side menu, {at}");
                    }
                    // With the options open, what is above their veil.
                    let from = scene.hits.iter().position(|(_, t)| *t == Target::OptionsVeil).unwrap_or(0);
                    let controls: Vec<(Rect, Target)> = scene.hits[from..].iter().copied().filter(|(_, t)| is_control(*t)).collect();
                    for (r, t) in &controls {
                        assert!(r.left >= -0.5 && r.top >= -0.5 && r.right <= width + 0.5 && r.bottom <= height + 0.5, "{t:?} leaves the window ({r:?}), {at}");
                        assert!(r.width() > 4.0 && r.height() > 4.0, "{t:?} squeezed to nothing ({r:?}), {at}");
                    }
                    // The OS's window buttons, where it draws them, have the
                    // top left to themselves: no control, reveal zone or pill.
                    if input.native_caption {
                        let lights = TRAFFIC_LIGHTS;
                        let over = |r: Rect| overlap(r, lights) > 0.0;
                        for (r, t) in scene.hits.iter().filter(|(_, t)| is_control(*t) || matches!(t, Target::RevealSidebar | Target::RevealToolbar)) {
                            assert!(!over(*r), "{t:?} {r:?} is under the window buttons, {at}");
                        }
                        for pill in scene.pills.iter().flatten() {
                            assert!(!over(*pill), "a pill {pill:?} is under the window buttons, {at}");
                        }
                    }
                    for (i, (a, ta)) in controls.iter().enumerate() {
                        for (b, tb) in &controls[i + 1..] {
                            assert!(overlap(*a, *b) <= 1.0, "{ta:?} {a:?} overlaps {tb:?} {b:?}, {at}");
                        }
                    }
                    if options_open {
                        let card = rect_of(&scene, Target::OptionsPanel).expect("the options card");
                        assert!(card.left >= 0.0 && card.right <= width && card.top >= 0.0 && card.bottom <= height, "options card leaves the window, {at}");
                        for (r, t) in controls.iter().filter(|(_, t)| matches!(t, Target::Choice(..) | Target::Transfer(_))) {
                            assert!(r.top >= card.top && r.bottom <= card.bottom, "{t:?} outside the options card, {at}");
                        }
                        // The text size's smaller and larger, a fair size each
                        // (iOS has none: the system's text size is its own).
                        let steps: Vec<Rect> = scene.hits.iter().filter(|(_, t)| matches!(t, Target::Choice(Setting::TextSize, _))).map(|(r, _)| *r).collect();
                        assert!(steps.len() == 2 || scene.options_scroll_max > 0.0 || input.platform == Platform::Ios, "the text size's steps, {at}");
                        assert!(steps.iter().all(|r| r.width() >= 40.0 && r.left >= card.left && r.right <= card.right), "the text size's steps squeezed, {at}");
                        checked += 1;
                        continue;
                    }
                    let column = scene.column;
                    assert!(column.width() >= 240.0, "the note is {} wide, {at}", column.width());
                    // The teal dot of things due: beside the dates, in the note's column.
                    let dot = rect_of(&scene, Target::DueDot).expect("the dot of things due");
                    let dates = rect_of(&scene, Target::PageDate).expect("the dates");
                    assert!(dot.left >= dates.right - 6.0 && dot.right <= column.right + 8.0 && dot.right <= width, "the dot of things due, {at}");
                    if side.0 > 0.0 {
                        // Docked, its edge can be taken, inside the window.
                        let edge = rect_of(&scene, Target::SidebarEdge).expect("the side menu's edge");
                        assert!(edge.left >= 0.0 && edge.right <= width && (edge.left + edge.right) / 2.0 - (MARGIN + menu) < 2.0, "the side menu's edge, {at}");
                    }
                    if side.0 == 0.0 {
                        assert!(rect_of(&scene, Target::SidebarEdge).is_none(), "revealed or tucked, the side menu is as it is, {at}");
                    }
                    if side.0 > 0.0 {
                        assert!(column.left >= MARGIN + menu + 8.0, "the note under the side menu, {at}");
                    }
                    // Revealed over the page, the toolbar leaves the note's dates
                    // and title clear.
                    if state == "bar revealed" {
                        let pin = rect_of(&scene, Target::ToggleToolbar).expect("the revealed toolbar");
                        assert!(column.top - 24.0 >= pin.bottom + 8.0, "the revealed toolbar over the note's dates ({pin:?}, {column:?}), {at}");
                    }
                    // A passing word, clear of the window's buttons.
                    let (right, top_of) = whisper_place(&input, column, width);
                    assert!(top_of >= CAPTION_BOTTOM || right <= actions_left(&input, width) - 8.0, "a passing word under the window's buttons, {at}");
                    if top.0 > 0.0 {
                        for tool in [Tool::Bold, Tool::Italic, Tool::Checklist] {
                            assert!(rect_of(&scene, Target::Tool(tool)).is_some(), "{tool:?} missing, {at}");
                        }
                        let bar = rect_of(&scene, Target::ToggleToolbar).unwrap();
                        assert!(column.top >= bar.bottom + 8.0, "the note under the toolbar, {at}");
                        // Centred on the window wherever it has the room
                        // (the side menu, the window's buttons, its tools).
                        let capsule = scene.hits.iter().filter(|(_, t)| *t == Target::ToolbarPanel).map(|(r, _)| *r).last().unwrap();
                        let room = (content_left(&input, width) + MARGIN).max(top_row_left(&input))..actions_left(&input, width) - MARGIN;
                        if capsule.top < CAPTION_BOTTOM && room.start <= width / 2.0 - capsule.width() / 2.0 && width / 2.0 + capsule.width() / 2.0 <= room.end {
                            assert!(((capsule.left + capsule.right) / 2.0 - width / 2.0).abs() <= 1.0, "the toolbar centred on the window ({capsule:?}), {at}");
                        }
                    } else if let Some(pill) = scene.pills[1] {
                        let minimize = rect_of(&scene, Target::SendTo).unwrap();
                        assert!(pill.right <= minimize.left - 8.0, "the toolbar's pill under the window buttons, {at}");
                        if side.0 > 0.0 {
                            assert!(pill.left >= MARGIN + menu, "the toolbar's pill under the side menu, {at}");
                        }
                        // Centred on the window, not the note (which moves
                        // off centre for related notes), wherever it has the
                        // room between the side menu and the window's buttons.
                        let room = (content_left(&input, width) + MARGIN + 8.0).max(top_row_left(&input))..actions_left(&input, width) - 16.0;
                        if room.start <= width / 2.0 - pill.width() / 2.0 && width / 2.0 + pill.width() / 2.0 <= room.end {
                            assert!(((pill.left + pill.right) / 2.0 - width / 2.0).abs() <= 0.5, "the toolbar's pill centred on the window ({pill:?}), {at}");
                        }
                    }
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 141 * 5 * states.len(), "every size and state");
    }

    #[test]
    fn the_text_size_steps_in_the_options_with_the_size_between() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        // The steps offered (their values, left to right), and how many
        // texts sit between where smaller and larger are at the default.
        let steps = |size: u8| {
            let mut options = [0; scene::SETTINGS];
            options[Setting::TextSize.index()] = size;
            let input = Input { vault_name: "notes", options_open: true, options, page_fade: 1.0, ..Default::default() };
            let scene = scene::build(&eng, &fonts, &input, 1000.0, 1600.0).unwrap();
            let mut steps: Vec<(Rect, u8)> = scene.hits.iter().filter_map(|(r, t)| if let Target::Choice(Setting::TextSize, v) = t { Some((*r, *v)) } else { None }).collect();
            steps.sort_by(|a, b| a.0.left.total_cmp(&b.0.left));
            (steps, scene)
        };
        let (at_default, scene) = steps(0);
        assert_eq!(at_default.iter().map(|s| s.1).collect::<Vec<_>>(), [15, 18], "smaller and larger than the default");
        let (a, b) = (at_default[0].0, at_default[1].0);
        assert!((a.width() - b.width()).abs() < 0.01 && a.top == b.top, "alike, on one row");
        let between = |scene: &Scene| scene.texts.iter().filter(|t| t.x > a.right + 1.0 && t.x < b.left - 2.0 && t.y >= a.top - 6.0 && t.y <= a.bottom).count();
        assert_eq!(between(&scene), 1, "the size between them");
        let values = |size| steps(size).0.iter().map(|s| s.1).collect::<Vec<_>>();
        assert_eq!(values(15), [14, 0], "a step up from 15 is the default");
        assert_eq!(values(13), [14], "nothing smaller than the smallest");
        assert_eq!(values(23), [20], "nothing larger than the largest");
        let (_, largest) = steps(23);
        assert_eq!(between(&largest), 1, "the largest says its size too");
    }

    /// The settings (and buttons) the options card offers on a platform.
    #[test]
    fn a_shells_own_settings_page_gets_what_the_card_offers_there() {
        let ios = offered_settings(Platform::Ios, false);
        let has = |l: &[Offered], s: Setting| l.iter().any(|o| o.setting == s);
        for s in [Setting::Theme, Setting::Font, Setting::Spell, Setting::ShowMarks, Setting::Launch] {
            assert!(has(&ios, s), "iOS offers {s:?}");
        }
        // iOS's text size is the system's (Dynamic Type).
        for s in [Setting::Window, Setting::Tray, Setting::Hotkey, Setting::Assist, Setting::Remind, Setting::Updates, Setting::TextSize] {
            assert!(!has(&ios, s), "iOS has no {s:?}");
        }
        let theme = ios.iter().find(|o| o.setting == Setting::Theme).unwrap();
        assert_eq!((theme.section, theme.label, theme.choices.len(), theme.stepper), ("Appearance", "Theme", 3, false));
        let mac = offered_settings(Platform::Mac, false);
        let size = mac.iter().find(|o| o.setting == Setting::TextSize).unwrap();
        assert!(size.stepper && size.choices.len() == 7 && size.choices.iter().any(|(w, v)| w == "16" && *v == 0), "{size:?}");
        assert!(has(&offered_settings(Platform::Windows, true), Setting::Tray) && has(&offered_settings(Platform::Windows, true), Setting::Updates));
    }

    fn offered(platform: Platform, options: [u8; scene::SETTINGS], updater: bool) -> (Vec<Setting>, Vec<Target>) {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let input = Input { vault_name: "notes", options_open: true, options, platform, updater, page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, toolbar_dock: 1.0, toolbar_show: 1.0, ..Default::default() };
        let scene = scene::build(&eng, &fonts, &input, 1000.0, 1600.0).unwrap();
        assert_eq!(scene.options_scroll_max, 0.0, "everything fits a tall window");
        let mut settings: Vec<Setting> = Vec::new();
        for (_, t) in &scene.hits {
            if let Target::Choice(s, _) | Target::Switch(s) = t {
                if !settings.contains(s) {
                    settings.push(*s);
                }
            }
        }
        let buttons = scene.hits.iter().map(|(_, t)| *t).filter(|t| matches!(t, Target::Transfer(_) | Target::VaultOpen | Target::VaultShow | Target::Quick(_))).collect();
        (settings, buttons)
    }

    /// Settings > Updates' "Check now": a button while it can be pressed,
    /// how far the check got in its place while it runs, and the version
    /// ready once it is; never in a store's build. Each says it in full in
    /// the narrowest options column.
    #[test]
    fn check_now_says_how_far_the_check_got() {
        use scene::UpdateCheck as C;
        assert_eq!(C::Idle.label(None), ("Check now".to_string(), true));
        assert_eq!(C::Checking.label(None), ("Checking\u{2026}".to_string(), false));
        assert_eq!(C::UpToDate.label(None), ("Up to date".to_string(), true));
        assert_eq!(C::Downloading("0.4.4").label(None), ("Downloading 0.4.4\u{2026}".to_string(), false));
        assert_eq!(C::Failed.label(None), ("Couldn't check".to_string(), true));
        assert_eq!(C::Checking.label(Some("0.4.4")), ("0.4.4 is ready".to_string(), false), "once ready, whatever the check said");

        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let build = |check: C, ready: Option<&str>, updater: bool, width: f32| {
            let input = Input { vault_name: "notes", options_open: true, updater, update_check: check, update: ready, page_fade: 1.0, ..Default::default() };
            scene::build(&eng, &fonts, &input, width, 2400.0).unwrap()
        };
        let button = |s: &Scene| s.hits.iter().find(|(_, t)| *t == Target::CheckUpdates).map(|(r, _)| *r);
        assert_eq!(button(&build(C::Idle, None, false, 1000.0)), None, "a store's build: no button");
        for width in [360.0, 600.0, 1000.0] {
            let bar = button(&build(C::Idle, None, true, width)).expect("pressable at rest");
            for (check, ready, pressable) in [
                (C::Idle, None, true),
                (C::Checking, None, false),
                (C::UpToDate, None, true),
                (C::Downloading("10.10.10"), None, false),
                (C::Failed, None, true),
                (C::Idle, Some("10.10.10"), false),
            ] {
                assert_eq!(button(&build(check, ready, true, width)).is_some(), pressable, "{check:?} at {width}");
                let (label, _) = check.label(ready);
                let w = scene::single_line(&eng, &label, &fonts.choice, 2000.0, ROW_H).measure().width;
                assert!(w <= bar.width() - 8.0, "{label:?} is {w} wide in {} at {width}", bar.width());
            }
        }
    }

    /// A switch's rows fold out from under it on a spring that passes its
    /// mark a little and settles; away, the same backwards. While they
    /// fold, the card's top stays where it is and only what is under the
    /// switch moves; what has folded away takes no clicks.
    #[test]
    fn a_switchs_rows_fold_on_a_spring_and_the_card_stays_put() {
        let open: Vec<f32> = (0..=70).map(|i| folding(i as f32 / 100.0, true).unwrap_or(1.0)).collect();
        assert_eq!(open[0], 0.0);
        assert!(open.iter().any(|&t| t > 1.02) && open.iter().all(|&t| t < 1.15), "a little past, not a bounce");
        assert!((open[69] - 1.0).abs() < 0.02 && folding(FOLD_SECS, true).is_none(), "settled by the end");
        assert_eq!(folding(0.0, false), Some(1.0), "turning off starts open");
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let devices = [crate::quick::DeviceRow { name: "Desk PC", look: crate::quick::Look::Desktop, this: true }];
        let build = |folding: Option<(Setting, f32)>, beam: u8| {
            let mut options = [0; scene::SETTINGS];
            options[Setting::Beam.index()] = beam;
            let input = Input {
                vault_name: "notes",
                options_open: true,
                options,
                folding,
                devices: Some(scene::DevicesView { devices: &devices, pairing: crate::quick::Pairing::Idle, said: None, facts: &[], this: "PC", naming: None }),
                page_fade: 1.0,
                ..Default::default()
            };
            scene::build(&eng, &fonts, &input, 1200.0, 900.0).unwrap()
        };
        let (on, half, off) = (build(None, 0), build(Some((Setting::Beam, 0.5)), 1), build(None, 1));
        let card = |s: &Scene| rect_of(s, Target::OptionsPanel).unwrap();
        assert_eq!(card(&on).top, card(&half).top, "the card's top stays");
        assert_eq!(card(&on).top, card(&off).top);
        assert!(card(&off).bottom < card(&half).bottom && card(&half).bottom < card(&on).bottom, "its foot follows the rows");
        let switch = |s: &Scene| rect_of(s, Target::Switch(Setting::Beam)).unwrap();
        assert_eq!(switch(&on), switch(&half), "the switch itself does not move");
        assert!(rect_of(&off, Target::Choice(Setting::QuickBeam, 1)).is_none(), "off: its rows are gone");
        // Half folded: the keys' rows are under the switch, clipped to the
        // room they have; what has folded away takes no clicks.
        let shown = rect_of(&half, Target::Choice(Setting::QuickBeam, 1));
        assert!(shown.is_none_or(|r| r.top >= switch(&half).bottom), "{shown:?}");
        assert_eq!(rect_of(&half, Target::Choice(Setting::Theme, 1)), rect_of(&on, Target::Choice(Setting::Theme, 1)), "the other column stays");
    }

    #[test]
    fn a_setting_is_named_as_its_row_says_on_each_platform() {
        assert_eq!(setting_title(Setting::Theme, Platform::Ios), Some("Theme"));
        assert_eq!(setting_title(Setting::Tray, Platform::Mac), Some("Menu bar icon"));
        assert_eq!(setting_title(Setting::Tray, Platform::Windows), Some("Tray icon"));
        assert_eq!(setting_title(Setting::Tray, Platform::Ios), None, "not offered there");
        assert_eq!(setting_title(Setting::TextSize, Platform::Linux), Some("Text size"));
    }

    /// Where an icon's ink is, DIPs: the middle of what its glyphs draw at
    /// `scale` pixels a DIP.
    /// No text of the note comes within a few DIPs of the window's buttons
    /// or the quick actions beside them (nor the Mac's lights), at any
    /// width: the minimal window's dates and title kept below them too.
    #[test]
    fn the_note_keeps_clear_of_the_window_buttons_at_every_width() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let text = "# A long title for a note that runs on across the window\nThe first words of the note, enough of them to reach the far edge of a wide column.";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        let buttons = [Target::Minimize, Target::Maximize, Target::Close, Target::SendTo, Target::Minimal, Target::ToMain];
        const CLEAR: f32 = 4.0;
        let mut checked = 0;
        for platform in [Platform::Windows, Platform::Linux, Platform::Mac] {
            for state in ["minimal", "docked", "focus"] {
                for width in (320..=2000).step_by(24).map(|w| w as f32) {
                    let input = Input {
                        text,
                        lines: &lines,
                        page_fade: 1.0,
                        page_date: Some("Edited 2 hours ago \u{00B7} Created on Monday \u{00B7} 312 words"),
                        platform,
                        native_caption: platform == Platform::Mac,
                        compact: state == "minimal",
                        no_margin: state == "minimal",
                        sidebar_dock: if state == "docked" { 1.0 } else { 0.0 },
                        sidebar_show: if state == "docked" { 1.0 } else { 0.0 },
                        toolbar_dock: if state == "minimal" { 0.0 } else { 1.0 },
                        toolbar_show: if state == "minimal" { 0.0 } else { 1.0 },
                        ..Default::default()
                    };
                    let s = scene::build(&eng, &fonts, &input, width, 700.0).unwrap();
                    let mut keep: Vec<Rect> = s.hits.iter().filter(|(_, t)| buttons.contains(t)).map(|(r, _)| *r).collect();
                    if platform == Platform::Mac {
                        keep.push(TRAFFIC_LIGHTS);
                    }
                    let at = format!("{state} on {platform:?} at {width}");
                    for t in &s.texts[..s.chrome_from.1.min(s.texts.len())] {
                        let m = t.layout.measure();
                        let r = Rect::new(t.x, t.y, t.x + m.width, t.y + m.height);
                        for b in &keep {
                            let near = r.left < b.right + CLEAR && r.right > b.left - CLEAR && r.top < b.bottom + CLEAR && r.bottom > b.top - CLEAR;
                            assert!(!near, "{:?} at {r:?} under a window button {b:?}, {at}", t.layout.text().chars().take(24).collect::<String>());
                        }
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 500, "{checked}");
    }

    /// The grid's bar has every action at every width the window takes:
    /// narrow, the search is an icon too, and its field, once it has the
    /// keys, takes the bar's width (Esc gives the icons back). A query typed
    /// past the field's end slides so its caret stays in sight, with the
    /// field's ✕ there all the same.
    #[test]
    fn the_grid_bar_keeps_its_search_and_the_caret_in_sight() {
        use crate::grid::{Grid, GridSearch, GridView, Phase};
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let g = Grid::at(Phase::Browse);
        let quick = scene::BeamView { unseen: 0, line: "", typing: false, menu: None, check: None, keys: "Alt+Z" };
        let long = "a very long query typed well past the end of the field it is in";
        let every = [Target::Search, Target::NewNote, Target::ViewToggle, Target::ThemeCycle, Target::Trash, Target::Options, Target::Quick(crate::quick::Hit::Fab)];
        for width in (360..=1400).step_by(20).map(|w| w as f32) {
            // At rest: every action, apart.
            let input = Input { page_fade: 1.0, quick: Some(quick), grid: Some(GridView { grid: &g, count: 0, first: 0, boxes: &[], search: None, chosen: None }), ..Default::default() };
            let s = scene::build(&eng, &fonts, &input, width, 900.0).unwrap();
            let cells: Vec<Rect> = every.iter().map(|t| s.hits.iter().rev().find(|(_, h)| h == t).map(|(r, _)| *r).unwrap_or_else(|| panic!("{t:?} at {width}"))).collect();
            for (i, c) in cells.iter().enumerate() {
                assert!(c.left >= 0.0 && c.right <= width, "{:?} in the window at {width}", every[i]);
                assert!(cells[i + 1..].iter().all(|d| overlap(*c, *d) <= 0.5), "{:?} apart at {width}", every[i]);
            }
            for query in ["", long] {
                let search = Some(GridSearch { query, caret: Some(query.len()) });
                let input = Input { page_fade: 1.0, quick: Some(quick), grid: Some(GridView { grid: &g, count: 0, first: 0, boxes: &[], search, chosen: None }), ..Default::default() };
                let s = scene::build(&eng, &fonts, &input, width, 900.0).unwrap();
                let at = format!("{width}, {:?}", query.len());
                let field = s.hits.iter().rev().find(|(_, t)| *t == Target::Search).map(|(r, _)| *r).unwrap_or_else(|| panic!("the search, {at}"));
                let caret = s.fills.iter().filter(|f| f.ink == Ink::Accent && f.rect.width() <= 2.5 && f.rect.top >= field.top && f.rect.bottom <= field.bottom).last().map(|f| f.rect).unwrap_or_else(|| panic!("the caret, {at}"));
                assert!(caret.left >= field.left && caret.right <= field.right, "the caret in the field, {at}");
                if !query.is_empty() {
                    let clear = s.hits.iter().find(|(_, t)| *t == Target::SearchClear).map(|(r, _)| *r).unwrap_or_else(|| panic!("the ✕, {at}"));
                    assert!(caret.right <= clear.left + 0.5, "the caret clear of the ✕, {at}");
                }
            }
        }
    }

    /// The top pill is on the window's middle at every width, with the side
    /// menu out or tucked away, on each platform; where it cannot be (a
    /// narrow window, the quick actions near the middle), it does not show.
    #[test]
    fn the_top_pill_is_centred_or_not_there() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let mut shown = 0;
        for platform in [Platform::Windows, Platform::Mac, Platform::Linux] {
            for docked in [0.0, 1.0] {
                for width in (360..=2000).step_by(20).map(|w| w as f32) {
                    let input = Input { page_fade: 1.0, platform, native_caption: platform == Platform::Mac, sidebar_dock: docked, sidebar_show: docked, ..Default::default() };
                    let s = scene::build(&eng, &fonts, &input, width, 800.0).unwrap();
                    if let Some(pill) = s.pills[1] {
                        assert!(((pill.left + pill.right) / 2.0 - width / 2.0).abs() <= 0.5, "{platform:?} docked {docked} at {width}: {pill:?}");
                        assert!(pill.width() >= 59.5, "{platform:?} at {width}: a stub");
                        shown += 1;
                    }
                }
            }
        }
        assert!(shown > 100, "{shown}");
    }

    /// The grid's trash sits under its bar, its Empty at the bar's right
    /// edge (under the bar's last icon) at every width, narrow included.
    #[test]
    fn the_grid_trash_empties_from_the_bars_edge_at_every_width() {
        use crate::grid::{Grid, GridView, Phase};
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let g = Grid::at(Phase::Browse);
        let entries = [TrashEntry { name: "Old plan".into(), when: "2 days ago".into() }, TrashEntry { name: "خطة قديمة".into(), when: "a week ago".into() }];
        for quick in [None, Some(scene::BeamView { unseen: 0, line: "", typing: false, menu: None, check: None, keys: "Alt+Z" })] {
            for width in (360..=2400).step_by(40).map(|w| w as f32) {
                let input = Input { page_fade: 1.0, quick, trash: Some(&entries), grid: Some(GridView { grid: &g, count: 0, first: 0, boxes: &[], search: None, chosen: None }), ..Default::default() };
                let s = scene::build(&eng, &fonts, &input, width, 900.0).unwrap();
                let empty = rect_of(&s, Target::TrashEmpty).unwrap_or_else(|| panic!("Empty at {width}"));
                let last = [Target::Quick(crate::quick::Hit::Fab), Target::Options].iter().find_map(|t| rect_of(&s, *t)).unwrap();
                assert!((empty.right - last.right).abs() <= 1.0, "Empty {empty:?} under the bar's last icon {last:?} at {width}");
                assert!(s.hits.iter().any(|(_, t)| matches!(t, Target::TrashRow(1))), "the rows at {width}");
            }
        }
    }

    /// The ink of a scene's texts, in DIPs.
    fn ink_box(eng: &Eng, t: &Text, scale: f32) -> Option<Rect> {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut e = eng.borrow_mut();
        for g in t.layout.glyphs((t.x * scale, t.y * scale), scale) {
            if let Some(m) = e.mask(&g).filter(|m| m.width > 0 && m.height > 0) {
                let (left, top) = ((g.x + m.left) as f32, (g.y - m.top) as f32);
                x0 = x0.min(left);
                y0 = y0.min(top);
                x1 = x1.max(left + m.width as f32);
                y1 = y1.max(top + m.height as f32);
            }
        }
        (x0 < x1).then(|| Rect::new(x0 / scale, y0 / scale, x1 / scale, y1 / scale))
    }

    /// Every icon along the grid's bar and the side menu's foot is one
    /// size: the font's glyphs, the drawn view icon and Beam's mark are
    /// as tall as each other, within a DIP and a half.
    #[test]
    fn the_icon_rows_are_one_size() {
        use crate::grid::{Grid, GridView, Phase};
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let quick = scene::BeamView { unseen: 0, line: "", typing: false, menu: None, check: None, keys: "Alt+Z" };
        let g = Grid::at(Phase::Browse);
        let scale = 3.0;
        for grid in [true, false] {
            let view = grid.then(|| GridView { grid: &g, count: 0, first: 0, boxes: &[], search: None, chosen: None });
            let input = Input { page_fade: 1.0, quick: Some(quick), grid: view, sidebar_dock: 1.0, sidebar_show: 1.0, ..Default::default() };
            let s = scene::build(&eng, &fonts, &input, 1200.0, 800.0).unwrap();
            let targets: &[Target] = if grid { &[Target::NewNote, Target::ViewToggle, Target::ThemeCycle, Target::Trash, Target::Options, Target::Quick(crate::quick::Hit::Fab)] } else { &[Target::ViewToggle, Target::ThemeCycle, Target::Trash, Target::Options] };
            let mut tall = Vec::new();
            for &t in targets {
                let cell = s.hits.iter().rev().find(|(_, h)| *h == t).map(|(r, _)| *r).unwrap();
                let inside = |r: Rect| r.left >= cell.left - 0.5 && r.right <= cell.right + 0.5 && r.top >= cell.top - 0.5 && r.bottom <= cell.bottom + 0.5;
                let glyph = s.texts.iter().filter(|x| x.layout.role() == kolada_text::Role::Icon).filter_map(|x| ink_box(&eng, x, scale)).find(|r| inside(*r));
                let drawn = s.fills.iter().filter(|f| inside(f.rect) && f.rect.width() < 12.0 && f.ink != Ink::Hover && f.ink != Ink::Selection).map(|f| f.rect).reduce(|a, b| Rect::new(a.left.min(b.left), a.top.min(b.top), a.right.max(b.right), a.bottom.max(b.bottom)));
                // Beam's mark: its ink is 17/24 of its square.
                let mark = s.vectors.iter().find(|v| inside(v.rect)).map(|v| v.rect.height() * 17.0 / 24.0);
                let h = glyph.map(|r| r.height()).or(mark).or(drawn.map(|r| r.height())).unwrap_or_else(|| panic!("{t:?}: no ink"));
                tall.push((t, h));
            }
            let (lo, hi) = tall.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (_, h)| (lo.min(*h), hi.max(*h)));
            assert!(hi - lo <= 1.5 && lo >= 11.0, "{}: one size, {tall:?}", if grid { "grid" } else { "side menu" });
        }
    }

    fn ink_centre(eng: &Eng, t: &Text, scale: f32) -> Option<(f32, f32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut e = eng.borrow_mut();
        for g in t.layout.glyphs((t.x * scale, t.y * scale), scale) {
            if let Some(m) = e.mask(&g).filter(|m| m.width > 0 && m.height > 0) {
                let (left, top) = ((g.x + m.left) as f32, (g.y - m.top) as f32);
                x0 = x0.min(left);
                y0 = y0.min(top);
                x1 = x1.max(left + m.width as f32);
                y1 = y1.max(top + m.height as f32);
            }
        }
        (x0 < x1).then(|| ((x0 + x1) / 2.0 / scale, (y0 + y1) / 2.0 / scale))
    }


    /// The side menu's foot and the grid's bar have the menu's icons in one
    /// order (the view, the appearance, the trash, settings) and one
    /// rhythm, the toolbar's gap between them; the grid's starts with a new
    /// note and ends with Beam, whose menu opens under it there.
    #[test]
    fn the_side_menu_and_the_grid_have_the_icons_in_one_order_and_rhythm() {
        use crate::grid::{Grid, GridView, Phase};
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let text = "# Plan\nShip it.";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        let quick = scene::BeamView { unseen: 0, line: "", typing: false, menu: None, check: None, keys: "Alt+Z" };
        let base = || Input { active_note: "Plan", text, lines: &lines, page_fade: 1.0, quick: Some(quick), ..Default::default() };
        let order = [Target::ViewToggle, Target::ThemeCycle, Target::Trash, Target::Options];
        let row = |s: &Scene, targets: &[Target]| -> Vec<Rect> { targets.iter().map(|t| s.hits.iter().rev().find(|(_, h)| h == t).map(|(r, _)| *r).unwrap_or_else(|| panic!("{t:?}"))).collect() };
        let list = scene::build(&eng, &fonts, &Input { sidebar_dock: 1.0, sidebar_show: 1.0, ..base() }, 1200.0, 800.0).unwrap();
        let g = Grid::at(Phase::Browse);
        let grid = scene::build(&eng, &fonts, &Input { grid: Some(GridView { grid: &g, count: 0, first: 0, boxes: &[], search: None, chosen: None }), ..base() }, 1200.0, 800.0).unwrap();
        for (name, s, targets) in [("side menu", &list, order.to_vec()), ("grid", &grid, [&[Target::NewNote][..], &order, &[Target::Quick(crate::quick::Hit::Fab)]].concat())] {
            let r = row(s, &targets);
            for w in r.windows(2) {
                assert!((w[0].top - w[1].top).abs() < 0.5, "{name}: one row {r:?}");
                assert!((w[1].left - w[0].left - ICON_PITCH).abs() < 0.5, "{name}: the icons {ICON_PITCH} apart, {r:?}");
                assert!((w[0].width() - ICON_TILE).abs() < 0.5, "{name}: one tile, {r:?}");
            }
        }
        // The round button is not over the grid; its menu hangs under the bar's.
        let menu = scene::BeamView { menu: Some("Plan"), ..quick };
        let open = scene::build(&eng, &fonts, &Input { grid: Some(GridView { grid: &g, count: 0, first: 0, boxes: &[], search: None, chosen: None }), quick: Some(menu), ..base() }, 1200.0, 800.0).unwrap();
        let cell = rect_of(&open, Target::Quick(crate::quick::Hit::Fab)).unwrap();
        let card = rect_of(&open, Target::Quick(crate::quick::Hit::Popup)).expect("Beam's menu");
        assert!(card.top >= cell.bottom && card.right <= 1200.0 && card.left >= 0.0, "under its button: {card:?} {cell:?}");
        assert_eq!(open.hits.iter().filter(|(_, t)| *t == Target::Quick(crate::quick::Hit::Fab)).count(), 1, "one Beam button");
    }
    /// Every icon's ink in the middle of its button's tile (its hover,
    /// pressed and chosen fill), not where its font's line puts it, and the
    /// icons of a row on one line: the side menu's foot, the grid's edge,
    /// the toolbar, the window's buttons, Settings and Beam, at two widths,
    /// two scales and both themes' inks (one shape: the inks do not move it).
    #[test]
    fn every_icon_sits_in_the_middle_of_its_button() {
        use crate::grid::{Grid, GridView, Phase};
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let rows = vec![SidebarRow { depth: 0, kind: RowKind::Folder { expanded: false }, name: "Work".into() }, SidebarRow { depth: 0, kind: RowKind::Note, name: "Plan".into() }];
        let text = "# Plan\nShip it.";
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
        let g = Grid::at(Phase::Browse);
        let mut checked = 0;
        for width in [760.0, 1400.0] {
            for scale in [1.0, 2.0] {
                let quick = scene::BeamView { unseen: 2, line: "From iPhone", typing: false, menu: None, check: None, keys: "Alt+Z" };
                // Each state, with the pointer on `hover`.
                let make = |state: &str, hover: Option<Target>| {
                    let base = Input { vault_name: "notes", active_note: "Plan", text, lines: &lines, sidebar: &rows, page_fade: 1.0, scale, hover, ..Default::default() };
                    match state {
                        "docked" => Input { sidebar_dock: 1.0, sidebar_show: 1.0, toolbar_dock: 1.0, toolbar_show: 1.0, quick: Some(quick), ..base },
                        "grid" => Input { grid: Some(GridView { grid: &g, count: 0, first: 0, boxes: &[], search: None, chosen: None }), ..base },
                        "settings" => Input { sidebar_dock: 1.0, sidebar_show: 1.0, options_open: true, ..base },
                        _ => Input { sidebar_dock: 1.0, sidebar_show: 1.0, toolbar_dock: 1.0, toolbar_show: 1.0, platform: Platform::Mac, native_caption: true, ..base },
                    }
                };
                for state in ["docked", "grid", "settings", "mac"] {
                    let s = scene::build(&eng, &fonts, &make(state, None), width, 800.0).unwrap();
                    let at = format!("{state} at {width}, {scale}x");
                    let icons = |s: &Scene| -> Vec<(String, f32, f32)> {
                        s.texts
                            .iter()
                            .filter(|t| t.layout.role() == kolada_text::Role::Icon && t.layout.format().align == Align::Center && t.layout.format().center_y)
                            .filter_map(|t| ink_centre(&eng, t, scale).map(|(x, y)| (t.layout.text().to_string(), x, y)))
                            .collect()
                    };
                    for (glyph, x, y) in icons(&s) {
                        // Its button: the smallest control under its ink,
                        // a button's size, not a panel's.
                        let Some(&(_, target)) = s
                            .hits
                            .iter()
                            .filter(|(r, _)| r.contains(x, y) && r.width() <= 64.0 && r.height() <= 64.0)
                            .min_by(|a, b| (a.0.width() * a.0.height()).total_cmp(&(b.0.width() * b.0.height())))
                        else {
                            continue;
                        };
                        // Its tile: what the pointer on it fills behind it.
                        let hovered = scene::build(&eng, &fonts, &make(state, Some(target)), width, 800.0).unwrap();
                        let Some(&(_, x, y)) = icons(&hovered).iter().filter(|(g, _, _)| *g == glyph).min_by(|a, b| ((a.1 - x).abs() + (a.2 - y).abs()).total_cmp(&((b.1 - x).abs() + (b.2 - y).abs()))) else { continue };
                        let Some(tile) = hovered
                            .fills
                            .iter()
                            .filter(|f| matches!(f.ink, Ink::Hover | Ink::Pressed) && f.rect.contains(x, y) && f.rect.width() <= 64.0 && f.rect.height() <= 64.0)
                            .map(|f| f.rect)
                            .min_by(|a, b| (a.width() * a.height()).total_cmp(&(b.width() * b.height())))
                        else {
                            continue;
                        };
                        let (cx, cy) = ((tile.left + tile.right) / 2.0, (tile.top + tile.bottom) / 2.0);
                        let px = 0.5 / scale;
                        assert!((x - cx).abs() <= px + 0.01 && (y - cy).abs() <= px + 0.01, "{glyph:?} on {target:?}: ink at ({x:.2}, {y:.2}), its tile's middle ({cx:.2}, {cy:.2}), {at}");
                        checked += 1;
                    }
                    // The side menu's foot: the view's pair and the icons
                    // beside it, one line through their middles.
                    if state == "docked" {
                        let mids: Vec<f32> = [Target::ViewToggle, Target::Trash, Target::ThemeCycle, Target::Options].iter().filter_map(|t| rect_of(&s, *t)).map(|r| (r.top + r.bottom) / 2.0).collect();
                        assert_eq!(mids.len(), 4, "{at}");
                        assert!(mids.iter().all(|m| (m - mids[0]).abs() <= 0.5 / scale), "the foot's icons off one line: {mids:?}, {at}");
                    }
                }
            }
        }
        assert!(checked > 60, "icons were checked ({checked})");
    }

    /// Each section of the settings once, and each setting in one row on a
    /// platform (two merges once kept Devices and Beam twice).
    #[test]
    fn settings_show_each_section_and_setting_once() {
        let titles: Vec<&str> = LEFT.iter().chain(RIGHT).map(|(t, _)| *t).collect();
        for (i, t) in titles.iter().enumerate() {
            assert!(!titles[i + 1..].contains(t), "the section {t:?} twice");
        }
        for platform in [Platform::Windows, Platform::Mac, Platform::Linux, Platform::Ios] {
            let offered = offered_settings(platform, true);
            for (i, o) in offered.iter().enumerate() {
                assert!(offered[i + 1..].iter().all(|p| p.setting != o.setting || p.label != o.label), "{:?} twice on {platform:?}", o.setting);
            }
        }
    }

    #[test]
    fn each_platform_offers_only_the_options_it_has() {
        let (win, buttons) = offered(Platform::Windows, [0; scene::SETTINGS], true);
        for s in [Setting::Beam, Setting::Motion, Setting::Theme, Setting::Font, Setting::TextSize, Setting::LineWidth, Setting::Spell, Setting::Assist, Setting::Related, Setting::ShowMarks, Setting::Launch, Setting::Window, Setting::Tray, Setting::Hotkey, Setting::Remind] {
            assert!(win.contains(&s), "Windows offers {s:?}");
        }
        assert!(buttons.contains(&Target::VaultOpen) && buttons.contains(&Target::VaultShow) && buttons.contains(&Target::Transfer(Transfer::Pdf)));
        // The Mac offers its own menu bar icon and quick note keys in their place.
        let (mac, mac_buttons) = offered(Platform::Mac, [0; scene::SETTINGS], false);
        for s in [Setting::Theme, Setting::Font, Setting::TextSize, Setting::LineWidth, Setting::Spell, Setting::Assist, Setting::Related, Setting::ShowMarks, Setting::Launch, Setting::Window, Setting::Tray, Setting::Hotkey, Setting::Remind] {
            assert!(mac.contains(&s), "the Mac offers {s:?}");
        }
        assert_eq!(mac_buttons, buttons, "both open another vault, show it, import and export");
        assert!(win.contains(&Setting::Updates), "the website's build offers to check for updates");
        assert!(!mac.contains(&Setting::Updates), "the Mac has no updater yet");
        // A store's build is updated by the store: no switch.
        let (store, _) = offered(Platform::Windows, [0; scene::SETTINGS], false);
        assert!(!store.contains(&Setting::Updates) && store.contains(&Setting::Remind), "the Store build hides only the update switch");
        // iOS: no tray or menu bar icon, no system-wide hotkey, no window
        // size of its own, no folder window to show the vault in, no updater
        // (the App Store updates it).
        let (ios, ios_buttons) = offered(Platform::Ios, [0; scene::SETTINGS], false);
        for s in [Setting::Theme, Setting::Font, Setting::LineWidth, Setting::Spell, Setting::Related, Setting::ShowMarks, Setting::Launch] {
            assert!(ios.contains(&s), "iOS offers {s:?}");
        }
        // Nor Motion (the system's Reduce Motion) or a switch for Beam (a
        // tab of its own there).
        for s in [Setting::Motion, Setting::Beam] {
            assert!(!ios.contains(&s), "iOS has no {s:?} switch");
        }
        // Nor a text size of its own: iOS's is the system's (Dynamic Type).
        for s in [Setting::Tray, Setting::Hotkey, Setting::Window, Setting::Remind, Setting::Updates, Setting::Assist, Setting::TextSize, Setting::QuickKeys, Setting::QuickBeam] {
            assert!(!ios.contains(&s), "iOS has no {s:?}");
        }
        // Beam: its keys and what they do on Windows and the Mac. Its devices
        // are not behind a button of Beam's: Settings > Devices holds them.
        for s in [Setting::QuickKeys, Setting::QuickBeam] {
            assert!(win.contains(&s) && mac.contains(&s), "Windows and the Mac offer {s:?}");
        }
        assert!(!buttons.contains(&Target::Quick(crate::quick::Hit::Devices)) && !ios_buttons.contains(&Target::Quick(crate::quick::Hit::Devices)));
        assert!(ios_buttons.contains(&Target::VaultOpen) && !ios_buttons.contains(&Target::VaultShow));
        // Linux: a window of its own and a folder to show, writing help and
        // reminders (the desktop's notifications), Beam's keys (Alt+Z, where
        // the desktop lets an app have keys of its own); but no tray icon or
        // quick note keys yet; the updater only where the copy updates itself.
        let (linux, linux_buttons) = offered(Platform::Linux, [0; scene::SETTINGS], false);
        for s in [Setting::Theme, Setting::Font, Setting::TextSize, Setting::LineWidth, Setting::Spell, Setting::Assist, Setting::Related, Setting::ShowMarks, Setting::Remind, Setting::Launch, Setting::Window, Setting::QuickKeys, Setting::QuickBeam] {
            assert!(linux.contains(&s), "Linux offers {s:?}");
        }
        for s in [Setting::Tray, Setting::Hotkey, Setting::Updates] {
            assert!(!linux.contains(&s), "Linux has no {s:?} yet");
        }
        assert_eq!(linux_buttons, buttons, "it opens another vault, shows it, imports and exports");
    }

    #[test]
    fn a_search_kept_open_beside_its_note_shows_its_query_but_no_caret() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let carets = |focused: bool| {
            let search = SearchView { query: "plan", caret: 4, hits: &[], selected: 0, first: 0, commands: None, focused };
            let input = Input { page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, search: Some(search), ..Default::default() };
            let scene = scene::build(&eng, &fonts, &input, 1200.0, 800.0).unwrap();
            let field = rect_of(&scene, Target::Search).unwrap();
            let caret = scene.fills.iter().filter(|f| f.ink == Ink::Accent && f.rect.width() < 2.0 && field.contains(f.rect.left, f.rect.top)).count();
            let query = scene.texts.iter().any(|t| field.contains(t.x + 1.0, t.y + 1.0));
            (caret, query)
        };
        assert_eq!(carets(true), (1, true), "taking the keys: its caret");
        assert_eq!(carets(false), (0, true), "kept open beside the note: the query, no caret");
        // The ✕: only once something is typed, inside the field, at every width.
        for (query, width) in [("", 1200.0), ("plan", 1200.0), ("plan", 900.0), ("plan", 360.0), ("a very long query that runs past the field", 700.0)] {
            let search = SearchView { query, caret: query.len(), hits: &[], selected: 0, first: 0, commands: None, focused: true };
            let input = Input { page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, search: Some(search), ..Default::default() };
            let scene = scene::build(&eng, &fonts, &input, width, 800.0).unwrap();
            let x = rect_of(&scene, Target::SearchClear);
            if query.is_empty() {
                assert!(x.is_none(), "no ✕ in an empty field");
                continue;
            }
            let (x, field) = (x.expect("the ✕"), rect_of(&scene, Target::Search).unwrap());
            assert!(x.left >= field.left && x.right <= field.right && x.top >= field.top && x.bottom <= field.bottom, "inside the field at {width}");
            assert!(scene.texts.iter().filter(|t| field.contains(t.x + 1.0, t.y + 1.0)).all(|t| t.x + t.layout.measure().width <= x.left + 8.0 || t.x >= x.left), "the query stops short of it at {width}");
        }
    }

    #[test]
    fn new_note_shows_the_keys_of_the_platforms_keyboard() {
        assert_eq!(new_note_keys(Platform::Linux), "Ctrl+N");
        assert_eq!(new_note_keys(Platform::Windows), "Ctrl+N");
        assert_eq!(new_note_keys(Platform::Mac), "\u{2318}N");
        assert_eq!(new_note_keys(Platform::Ios), "\u{2318}N", "an iPad's keyboard has Command");
    }

    #[test]
    fn every_option_names_itself_in_its_column() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let mut named = 0;
        for (_, rows) in LEFT.iter().chain(RIGHT) {
            for row in rows.iter() {
                let row = match row {
                    // In DejaVu Sans the update switch's name is 119 wide:
                    // a column that holds it leaves a 360 wide window too
                    // little room for Check now's "Downloading" (Linux's
                    // follow-up: a shorter name there, or a narrower face).
                    Row::Updater(_) if cfg!(target_os = "linux") => continue,
                    Row::On(_, r) | Row::Updater(r) | Row::NotOn(_, r) => *r,
                    r => r,
                };
                if let Row::Choice(name, ..) | Row::Stepper(name, _) | Row::Buttons(name, _) = row {
                    let w = scene::single_line(&eng, name, &fonts.label, 1000.0, ROW_H).measure().width;
                    assert!(w <= LABEL_W - 8.0, "{name:?} is {w} wide, past its column");
                    named += 1;
                }
            }
        }
        assert!(named >= 14);
    }

    /// The side menu with an update ready, docked in a window `width` x
    /// `height`.
    fn with_update(width: f32, height: f32, hover: Option<Target>) -> Scene {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let rows: Vec<SidebarRow> = (0..40).map(|i| SidebarRow { depth: 0, kind: RowKind::Note, name: format!("Note {i}") }).collect();
        let input = Input { vault_name: "notes", sidebar: &rows, update: Some("0.10.0"), updater: true, hover, page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, toolbar_dock: 1.0, toolbar_show: 1.0, ..Default::default() };
        scene::build(&eng, &fonts, &input, width, height).unwrap()
    }

    /// Beam's row sits at the side menu's foot where it runs: quiet with
    /// where the last beam came from, on the tint with how many are new;
    /// the update card stacks above it, the icons stay three, below it.
    #[test]
    fn beams_row_sits_above_the_side_menus_icons_under_the_update_card() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let rows: Vec<SidebarRow> = (0..40).map(|i| SidebarRow { depth: 0, kind: RowKind::Note, name: format!("Note {i}") }).collect();
        let show = Target::Quick(crate::quick::Hit::Show);
        let quiet = scene::BeamView { unseen: 0, line: "From iPhone, 2 h ago", typing: false, menu: None, check: None, keys: "Alt+Z" };
        let new = scene::BeamView { unseen: 3, line: "Gate code for Friday", ..quiet };
        for (quick, width) in [(Some(quiet), 1200.0), (Some(new), 1200.0), (Some(new), 700.0), (None, 1200.0)] {
            let input = Input { vault_name: "notes", sidebar: &rows, update: Some("0.10.0"), updater: true, quick, page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, toolbar_dock: 1.0, toolbar_show: 1.0, ..Default::default() };
            let scene = scene::build(&eng, &fonts, &input, width, 800.0).unwrap();
            let icons: Vec<Rect> = [Target::ViewToggle, Target::Trash, Target::ThemeCycle, Target::Options].map(|t| rect_of(&scene, t).unwrap()).to_vec();
            let pill = rect_of(&scene, Target::Update).unwrap();
            let panel = rect_of(&scene, Target::SidebarPanel).unwrap();
            let Some(row) = rect_of(&scene, show) else {
                assert!(quick.is_none(), "drawn where Beam runs");
                assert!(pill.bottom <= icons[0].top - 4.0 && pill.bottom >= icons[0].top - 16.0, "the pill keeps its place");
                continue;
            };
            let beam = quick.expect("no row where it does not run");
            assert!(row.bottom <= icons[0].top - 4.0 && row.bottom >= icons[0].top - 16.0, "just above the icons ({row:?}, {:?})", icons[0]);
            assert!(pill.bottom <= row.top - 4.0, "the update card stacks above the row ({pill:?}, {row:?})");
            assert!(row.left >= panel.left && row.right <= panel.right);
            // The file tree stops above both.
            for (r, t) in &scene.hits {
                if matches!(t, Target::Row(_)) {
                    assert!(r.bottom <= pill.top, "{t:?} {r:?} runs under the pill");
                }
            }
            let inside: Vec<&str> = scene.texts.iter().filter(|t| t.x >= row.left && t.x < row.right && t.y >= row.top - 0.5 && t.y < row.bottom).map(|t| t.layout.text()).collect();
            let title = if beam.unseen > 0 { "Beam \u{00B7} 3 new" } else { "Beam" };
            assert!(inside.contains(&title) && inside.contains(&beam.line), "{inside:?}");
            // Teal only while beams are new.
            let tinted = scene.fills.iter().any(|f| f.ink == Ink::Mark && (f.rect.top - row.top).abs() < 0.5);
            assert_eq!(tinted, beam.unseen > 0);
        }
    }

    /// The window's Beam button, on the 4-point grid: 40 at rest, 24 from
    /// the window's edges, faint (fainter while typing); 48, solid, with a
    /// white count when beams arrive; over it, a capsule that says "Beam"
    /// and its keys on one baseline. Its menu is a Kolada menu: the same
    /// rows, both marks in one column and filled, the keys on their label's
    /// baseline, no preview line, no row tinted at rest. Hidden where there
    /// is no room for it.
    /// The Mac's Settings write keys as the Mac does (⌃⌥N), never the PC's words.
    #[test]
    fn the_macs_settings_write_keys_in_its_symbols() {
        let mut seen = 0;
        for (_, rows) in LEFT.iter().chain(RIGHT) {
            for row in rows.iter() {
                if let Row::On(Platform::Mac, Row::Choice(_, _, choices)) = row {
                    for (label, _) in choices.iter() {
                        assert!(!label.contains("Ctrl") && !label.contains("Opt") && !label.contains("Alt"), "{label:?}");
                        seen += 1;
                    }
                }
            }
        }
        assert!(seen >= 5);
    }

    /// On the Mac ⌘H hides the app, so its headings are Option-Command;
    /// the iPad keeps ⌘1, Windows and Linux Ctrl+1.
    #[test]
    fn the_heading_buttons_keys_follow_each_keyboard() {
        let heading = Target::Tool(Tool::Heading);
        assert_eq!(button_name(heading, Platform::Mac, false), Some(("Heading", "\u{2325}\u{2318}1")));
        assert_eq!(button_name(heading, Platform::Ios, false), Some(("Heading", "\u{2318}1")));
        assert_eq!(button_name(heading, Platform::Windows, false), Some(("Heading", "Ctrl+1")));
    }

    #[test]
    fn beams_button_rests_small_grows_on_arrival_and_offers_the_note() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let fab = Target::Quick(crate::quick::Hit::Fab);
        let rest = scene::BeamView { unseen: 0, line: "", typing: false, menu: None, check: None, keys: "Alt+Z" };
        let build = |beam: scene::BeamView, hover: Option<Target>, w: f32, h: f32| {
            let input = Input { vault_name: "notes", quick: Some(beam), hover, page_fade: 1.0, ..Default::default() };
            scene::build(&eng, &fonts, &input, w, h).unwrap()
        };
        let accent = |scene: &Scene, r: Rect| scene.fills.iter().find(|f| f.ink == Ink::Accent && (f.rect.left - r.left).abs() < 0.5 && (f.rect.top - r.top).abs() < 0.5).map(|f| f.alpha).unwrap();
        let on_grid = |r: Rect| [r.left, r.top, r.right, r.bottom].iter().all(|v| (v / 4.0).fract().abs() < 0.01);
        let s = build(rest, None, 1200.0, 800.0);
        let small = rect_of(&s, fab).expect("the button");
        assert_eq!((small.width(), small.right, small.bottom), (40.0, 1200.0 - 24.0, 800.0 - 24.0));
        assert!(on_grid(small));
        let at_rest = accent(&s, small);
        assert!(at_rest < 0.5, "faint at rest");
        assert!(s.vectors.iter().any(|v| v.logo == crate::vector::Logo::Beam), "the beam is a filled mark, not beads");
        let s = build(scene::BeamView { typing: true, ..rest }, None, 1200.0, 800.0);
        assert!(accent(&s, rect_of(&s, fab).unwrap()) < at_rest, "fainter while typing");
        // Arrived: grown, solid, with its count.
        let s = build(scene::BeamView { unseen: 2, ..rest }, None, 1200.0, 800.0);
        let big = rect_of(&s, fab).unwrap();
        assert!(big.height() == 48.0 && accent(&s, big) == 1.0 && on_grid(big));
        assert!(s.texts.iter().any(|t| t.layout.text() == "2" && t.y < big.top + 16.0));
        // Over it: "Beam" and its keys, on one baseline.
        let s = build(rest, Some(fab), 1200.0, 800.0);
        let base = |t: &Text| t.y + t.layout.glyphs((0.0, 0.0), 1.0).first().map_or(0.0, |g| g.y as f32);
        let word = s.texts.iter().find(|t| t.layout.text() == "Beam").expect("its name");
        let keys = s.texts.iter().find(|t| t.layout.text() == "Alt+Z").expect("its keys");
        assert!((base(word) - base(keys)).abs() < 0.6, "one baseline: {} and {}", base(word), base(keys));
        assert_eq!(button_name(fab, Platform::Windows, false), None, "it names itself; no tooltip over it");
        // The menu, above the button, in the window, at 1200 and 320 wide.
        for w in [1200.0, 320.0] {
            let s = build(scene::BeamView { menu: Some("Trip plans"), ..rest }, None, w, 800.0);
            let button = rect_of(&s, fab).unwrap();
            let rows: Vec<Rect> = [crate::quick::Hit::BeamNote, crate::quick::Hit::NewBeam].iter().map(|h| rect_of(&s, Target::Quick(*h)).expect("menu row")).collect();
            for r in &rows {
                assert!(r.bottom <= button.top && r.left >= 0.0 && r.right <= w, "{r:?} at {w}");
                assert_eq!(r.height(), MENU_ITEM, "a Kolada menu's row");
            }
            assert_eq!(rows[0].left, rows[1].left);
            assert_eq!(rows[0].bottom, rows[1].top, "no gap, same rows");
            // Both marks centred in one icon column, at the rows' middles.
            let marks: Vec<&VectorMark> = s.vectors.iter().filter(|v| rows.iter().any(|r| r.contains(v.rect.left + 1.0, v.rect.top + 1.0))).collect();
            assert_eq!(marks.iter().map(|v| v.logo).collect::<Vec<_>>(), [crate::vector::Logo::Beam, crate::vector::Logo::NewNote], "filled marks, no hairline plus");
            assert_eq!(marks[0].rect.left, marks[1].rect.left);
            for (m, r) in marks.iter().zip(&rows) {
                assert!(((m.rect.top + m.rect.bottom) / 2.0 - (r.top + r.bottom) / 2.0).abs() < 0.01);
            }
            // Labels in one text column; the keys on their label's baseline.
            let label = |name: &str| s.texts.iter().find(|t| t.layout.text() == name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(label("Beam this note").x, label("New beam").x);
            assert_eq!(label("Beam this note").x, rows[0].left + MENU_TEXT);
            assert!((base(label("New beam")) - base(label("Alt+Z"))).abs() < 0.6);
            let texts: Vec<&str> = s.texts.iter().map(|t| t.layout.text()).collect();
            assert!(!texts.iter().any(|t| t.contains("Trip plans") || t.contains("Send")), "no preview line; it beams, never sends: {texts:?}");
            // At rest, no row is tinted.
            assert!(!s.fills.iter().any(|f| f.ink == Ink::Hover && rows.iter().any(|r| r.contains(f.rect.left + 1.0, f.rect.top + 1.0))));
        }
        // No room, or the window is busy with something else: no button.
        assert!(rect_of(&build(rest, None, 280.0, 800.0), fab).is_none());
        let input = Input { vault_name: "notes", quick: Some(rest), options_open: true, page_fade: 1.0, ..Default::default() };
        assert!(rect_of(&scene::build(&eng, &fonts, &input, 1200.0, 800.0).unwrap(), fab).is_none());
    }

    /// Settings > Devices is the one place devices are paired, for Beam and
    /// note sync alike: each device with Remove (this one marked), then
    /// what pairing asks for now; a code typed there, Pair and Cancel. It
    /// shows only where pairing runs, and opening Settings for it lands on it.
    #[test]
    fn settings_holds_the_devices_and_pairing_in_one_place() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let devices = [crate::quick::DeviceRow { name: "Desk PC", look: crate::quick::Look::Laptop, this: true }, crate::quick::DeviceRow { name: "iPhone", look: crate::quick::Look::Phone, this: false }];
        let build = |pairing: crate::quick::Pairing, w: f32| {
            let v = scene::DevicesView { devices: &devices, pairing, said: None, facts: &["3 notes are too big to sync".to_string()], this: "PC", naming: None };
            let mut input = Input { vault_name: "notes", options_open: true, devices: Some(v), page_fade: 1.0, ..Default::default() };
            input.options_scroll = devices_scroll(&input, w);
            scene::build(&eng, &fonts, &input, w, 800.0).unwrap()
        };
        let q = |h: crate::quick::Hit| Target::Quick(h);
        for w in [1200.0, 560.0] {
            let s = build(crate::quick::Pairing::Idle, w);
            let texts: Vec<&str> = s.texts.iter().map(|t| t.layout.text()).collect();
            assert!(texts.contains(&"Sync and Beam") && texts.contains(&"Desk PC") && texts.contains(&"This one") && texts.contains(&"iPhone"), "{texts:?}");
            assert!(texts.contains(&"3 notes are too big to sync"));
            for t in [q(crate::quick::Hit::Remove(1)), q(crate::quick::Hit::ShowCode), q(crate::quick::Hit::EnterCode)] {
                let r = rect_of(&s, t).unwrap_or_else(|| panic!("{t:?} at {w}"));
                assert!(r.top >= 0.0 && r.bottom <= 800.0, "{t:?} in view at {w}: opened at Devices");
            }
            assert!(rect_of(&s, q(crate::quick::Hit::Remove(0))).is_none(), "this device is not removed from here");
        }
        let s = build(crate::quick::Pairing::Showing { code: "F4FS-2RSA", left: "4:12 left" }, 1200.0);
        assert!(s.texts.iter().any(|t| t.layout.text() == "F4FS-2RSA") && rect_of(&s, q(crate::quick::Hit::Cancel)).is_some());
        let s = build(crate::quick::Pairing::Entering { typed: "F4FS" }, 1200.0);
        assert!(rect_of(&s, q(crate::quick::Hit::CodeField)).is_some() && rect_of(&s, q(crate::quick::Hit::Pair)).is_some());
        // Where pairing does not run, the section has its switch and no devices.
        let none = scene::build(&eng, &fonts, &Input { vault_name: "notes", options_open: true, page_fade: 1.0, ..Default::default() }, 1200.0, 800.0).unwrap();
        assert!(!none.texts.iter().any(|t| t.layout.text() == "Desk PC") && rect_of(&none, Target::Switch(Setting::Beam)).is_some());
    }

    /// Every control of Settings > Devices, clicked where it is drawn (its
    /// word's middle), at 100 and 150%, scrolled to Devices or not: the
    /// click finds that control, and it keeps Settings open (seen: a click
    /// on Pair a device or Enter a code closed Settings instead).
    #[test]
    fn each_devices_control_answers_a_click_where_it_is_drawn() {
        use crate::quick::{Hit as Q, Pairing};
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let devices = [crate::quick::DeviceRow { name: "Desk PC", look: crate::quick::Look::Laptop, this: true }, crate::quick::DeviceRow { name: "iPhone", look: crate::quick::Look::Phone, this: false }];
        let states: [(Pairing, &[(Q, &str)]); 4] = [
            (Pairing::Idle, &[(Q::ShowCode, "Pair a device"), (Q::EnterCode, "Enter a code"), (Q::Remove(1), "Remove")]),
            (Pairing::Showing { code: "F4FS-2RSA", left: "4:12 left" }, &[(Q::Cancel, "Cancel")]),
            (Pairing::Asks { name: "Laptop" }, &[(Q::Allow, "Allow"), (Q::Deny, "No")]),
            (Pairing::Entering { typed: "F4FS" }, &[(Q::Pair, "Pair"), (Q::Cancel, "Cancel"), (Q::CodeField, "F4FS")]),
        ];
        for (pairing, controls) in states {
            for (w, h) in [(1200.0, 800.0), (560.0, 700.0)] {
                for scrolled in [false, true] {
                    let v = scene::DevicesView { devices: &devices, pairing: pairing.clone(), said: None, facts: &[], this: "PC", naming: None };
                    let mut input = Input { vault_name: "notes", options_open: true, devices: Some(v), page_fade: 1.0, ..Default::default() };
                    if scrolled {
                        input.options_scroll = devices_scroll(&input, w);
                    }
                    let s = scene::build(&eng, &fonts, &input, w, h).unwrap();
                    let panel = rect_of(&s, Target::OptionsPanel).expect("the panel");
                    for (hit, word) in controls {
                        let t = s.texts.iter().find(|t| t.layout.text() == *word).unwrap_or_else(|| panic!("{word} drawn ({w}, {scrolled})"));
                        let m = t.layout.measure();
                        let (cx, cy) = (t.x + m.width.min(40.0) / 2.0 + 2.0, t.y + m.height / 2.0);
                        // Below the fold until scrolled there; scrolled to
                        // Devices, every control is in view.
                        let seen = panel.contains(cx, cy) && cy > panel.top + 40.0;
                        assert!(seen || !scrolled, "{word} in view once scrolled to Devices ({w})");
                        if !seen {
                            continue;
                        }
                        for scale in [1.0f32, 1.5] {
                            // As the window finds it: pixels, back to DIPs.
                            let (px, py) = ((cx * scale) as i32, (cy * scale) as i32);
                            let got = scene::hit(&s.hits, px as f32 / scale, py as f32 / scale);
                            assert_eq!(got, Some(Target::Quick(*hit)), "{word} at {scale}x ({w}, scrolled {scrolled})");
                            assert!(keeps_settings(got), "{word}: Settings stays open");
                        }
                    }
                }
            }
        }
        // A click beside the panel still closes it.
        assert!(!keeps_settings(Some(Target::OptionsVeil)) && !keeps_settings(None));
        assert!(!keeps_settings(Some(Target::Quick(Q::Fab))), "the window's Beam button is not Settings'");
    }

    #[test]
    fn an_update_ready_shows_as_a_pill_above_the_side_menus_icons() {
        let scene = with_update(1200.0, 800.0, None);
        let pill = rect_of(&scene, Target::Update).expect("the pill");
        let panel = rect_of(&scene, Target::SidebarPanel).unwrap();
        let icons: Vec<Rect> = [Target::ViewToggle, Target::Trash, Target::ThemeCycle, Target::Options].map(|t| rect_of(&scene, t).unwrap()).to_vec();
        for icon in &icons {
            assert!(pill.bottom <= icon.top - 4.0 && pill.bottom >= icon.top - 16.0, "just above the icons ({pill:?}, {icon:?})");
        }
        assert!(pill.left >= panel.left && pill.right <= panel.right, "inside the side menu");
        // The file tree stops above it.
        for (r, t) in &scene.hits {
            if matches!(t, Target::Row(_)) {
                assert!(r.bottom <= pill.top, "{t:?} {r:?} runs under the pill");
            }
        }
        assert!(pill.height() >= 40.0, "tall enough for two lines and a button");
        // Two lines on the left (what is ready, then its version), and the
        // Restart button at rest, a button's fill, at its right end.
        let inside = |t: &&Text| t.x >= pill.left && t.x < pill.right && t.y >= pill.top - 0.5 && t.y < pill.bottom;
        let texts: Vec<&Text> = scene.texts.iter().filter(inside).collect();
        assert_eq!(texts.len(), 3, "Update ready, the version, Restart");
        assert!(texts.iter().any(|t| t.ink == Ink::Faint) && texts.iter().filter(|t| t.ink != Ink::Faint).count() == 2);
        let button = scene.fills.iter().find(|f| f.ink == Ink::Button && f.rect.left > (pill.left + pill.right) / 2.0 && f.rect.right <= pill.right && f.rect.top >= pill.top && f.rect.bottom <= pill.bottom);
        assert!(button.is_some(), "the Restart button shows without pointing at it");
        assert!(scene.fills.iter().any(|f| f.ink == Ink::Accent), "its teal dot");
        // Narrow, it still fits: "Update" alone, then its version and the button.
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let rows: Vec<SidebarRow> = Vec::new();
        let narrow = Input { sidebar: &rows, update: Some("0.10.0"), sidebar_width: SIDEBAR_LEAST, page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, ..Default::default() };
        let small = scene::build(&eng, &Fonts::new(1.0, 0), &narrow, 1200.0, 800.0).unwrap();
        let tight = rect_of(&small, Target::Update).unwrap();
        for t in small.texts.iter().filter(|t| t.x >= tight.left && t.x < tight.right && t.y >= tight.top - 0.5 && t.y < tight.bottom) {
            assert!(t.x + t.layout.measure().width <= tight.right - 4.0, "text runs out of the narrow pill");
        }
        let pointed = with_update(1200.0, 800.0, Some(Target::Update));
        assert_eq!(rect_of(&pointed, Target::Update), Some(pill));
        // Nothing ready, no pill.
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let none = scene::build(&eng, &Fonts::new(1.0, 0), &Input { page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, ..Default::default() }, 1200.0, 800.0).unwrap();
        assert!(rect_of(&none, Target::Update).is_none());
    }

    #[test]
    fn the_side_menu_is_as_wide_as_dragged_and_leaves_the_note_room() {
        assert_eq!(menu_width(0.0, 1400.0), SIDEBAR_WIDTH);
        assert_eq!(menu_width(320.0, 1400.0), 320.0);
        assert_eq!((menu_width(50.0, 1400.0), menu_width(900.0, 1400.0)), (SIDEBAR_LEAST, SIDEBAR_MOST));
        // Where it only just docks, wide gives way to the note, narrow stays.
        let edge = SIDEBAR_WIDTH + 2.0 * MARGIN + MIN_NOTE_BESIDE_MENU;
        assert_eq!(menu_width(SIDEBAR_MOST, edge), SIDEBAR_WIDTH);
        assert_eq!(menu_width(SIDEBAR_LEAST, edge), SIDEBAR_LEAST);
        assert!(menu_width(SIDEBAR_MOST, 300.0) <= 300.0 - 2.0 * MARGIN, "floating in a tiny window, it fits");
    }

    #[test]
    fn eases_monotonically_between_the_ends() {
        assert_eq!((ease(0.0), ease(1.0)), (0.0, 1.0));
        let samples: Vec<f32> = (0..=20).map(|i| ease(i as f32 / 20.0)).collect();
        assert!(samples.windows(2).all(|w| w[0] <= w[1]));
        assert!(ease(0.5) > 0.5, "ease-out moves fastest at the start");
    }
}
