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
const PAD: f32 = 10.0;
const ROW: f32 = 30.0;
const INDENT: f32 = 14.0;
pub const TOOLBAR_HEIGHT: f32 = 40.0;
const TOOL: f32 = 32.0;
const TOOL_GAP: f32 = 2.0;
const SEPARATOR: f32 = 13.0;
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

mod icon {
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
}

/// Toolbar contents, left to right; None is a separator.
const TOOLS: [Option<Tool>; 9] = [
    Some(Tool::Bold),
    Some(Tool::Italic),
    None,
    Some(Tool::List),
    Some(Tool::Checklist),
    Some(Tool::Quote),
    None,
    Some(Tool::Code),
    Some(Tool::Link),
];

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

struct Painter<'a> {
    eng: &'a Eng,
    fonts: &'a Fonts,
    hover: Option<Target>,
    scene: &'a mut Scene,
}

impl Painter<'_> {
    fn text(&mut self, text: &str, format: &Format, r: Rect, ink: Ink) -> Result<()> {
        let eng = self.eng;
        let layout = scene::kept_layout(eng, text, format, r.width(), r.height(), 0, || {
            scene::layout_styled(eng, text, &Format { rtl: Some(scene::is_rtl(text)), ..*format }, r.width(), r.height(), &[])
        });
        self.scene.texts.push(Text { x: r.left, y: r.top, layout, ink, alpha: 1.0 });
        Ok(())
    }

    fn icon(&mut self, glyph: &str, r: Rect, ink: Ink) -> Result<()> {
        let format = self.fonts.icon.clone();
        self.text(glyph, &format, r, ink)
    }

    /// A floating panel: soft shadow, 1-DIP border, fill. `lift` deepens the
    /// shadow for panels floating over the note.
    fn panel(&mut self, r: Rect, lift: f32) {
        let fills = &mut self.scene.fills;
        fills.push(Fill::shadow(r.offset(0.0, 2.0 + 4.0 * lift), PANEL_RADIUS, SHADOW_BLUR * (1.0 + lift), 1.0 + lift));
        fills.push(Fill::solid(r, PANEL_RADIUS, Ink::Border));
        fills.push(Fill::solid(r.inset(1.0), PANEL_RADIUS - 1.0, Ink::Panel));
    }

    /// A button's hover highlight, and its click target.
    fn button(&mut self, r: Rect, target: Target, radius: f32) {
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

/// The space that keeps the quick actions apart from the window buttons.
const ACTIONS_GAP: f32 = 16.0;

/// Where the OS draws its window buttons when it does (the Mac's traffic
/// lights, at the top left), DIPs: nothing of ours goes under them.
pub const TRAFFIC_LIGHTS: Rect = Rect { left: 12.0, top: 6.0, right: 80.0, bottom: 30.0 };
/// How far right of them, and below them, our own controls keep.
const LIGHTS_GAP: f32 = 8.0;

/// The left edge, on the top row, our controls keep to: right of the OS's
/// window buttons, if it draws them there.
fn top_row_left(input: &Input) -> f32 {
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
    } else {
        &[(Target::SendTo, icon::SEND), (Target::Minimal, icon::MINIMAL)]
    }
}

/// Where the quick actions start, from the left.
fn actions_left(input: &Input, width: f32) -> f32 {
    // Left of the window's buttons; when the OS draws them (the Mac's, on
    // the left), at the window's right edge.
    let buttons = if input.native_caption { 0.0 } else { 3.0 * CAPTION_WIDTH + ACTIONS_GAP };
    width - buttons - actions(input).len() as f32 * CAPTION_WIDTH
}

/// Room kept right of the note column for related notes: the gap, a chip,
/// and the window's edge.
const MARGIN_ROOM: f32 = 44.0 + 232.0 + 24.0;
/// Narrowest the column gets to keep that room.
const MIN_COLUMN_BESIDE_MARGIN: f32 = 520.0;

/// Room the docked side menu takes on the left, DIPs.
fn content_left(input: &Input, width: f32) -> f32 {
    (side_width(input, width) + 2.0 * MARGIN) * ease(input.sidebar_dock)
}

/// The side menu's width as it is now: docked, as dragged; revealed from
/// focus mode, its own; on the way between, between.
fn side_width(input: &Input, width: f32) -> f32 {
    let (own, set) = (menu_width(0.0, width), menu_width(input.sidebar_width, width));
    own + (set - own) * ease(input.sidebar_dock)
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
pub fn sidebar_fits(width: f32) -> bool {
    width >= SIDEBAR_WIDTH + 2.0 * MARGIN + MIN_NOTE_BESIDE_MENU
}

/// The toolbar's width with every button, and the least it takes (the
/// text size and line width buttons, then others, give way before that).
const TOOLBAR_FULL: f32 = 460.0;
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
    let mut column_width = (content_width - 2.0 * pad).clamp(80.0, column_max(input.options[2]));
    let mut column_left = content_left + (content_width - column_width) / 2.0;
    // Keep room on the right for related notes when the window allows:
    // move the column left of center, narrowing it a little if need be.
    // It depends on the window alone, so the text never jumps as notes
    // come and go.
    if !input.no_margin && (content_width - column_width) / 2.0 < MARGIN_ROOM {
        let fit = (content_width - COLUMN_PAD - MARGIN_ROOM).min(column_width);
        if fit >= MIN_COLUMN_BESIDE_MARGIN {
            column_width = fit;
            column_left = content_left + ((content_width - column_width - MARGIN_ROOM) / 2.0).max(COLUMN_PAD);
        }
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
    Rect::new(column_left, column_top, column_left + column_width, height)
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
    let (s_dock, s_show) = (ease(input.sidebar_dock), ease(input.sidebar_show));
    let (t_dock, t_show) = (ease(input.toolbar_dock), ease(input.toolbar_show));

    // The top strip drags the window wherever nothing else is on top.
    p.scene.hits.push((Rect::new(0.0, 0.0, width, MARGIN + TOOLBAR_HEIGHT), Target::Caption));
    let column = column(input, width, height);
    p.scene.hits.push((Rect::new(content_left(input, width), MARGIN + TOOLBAR_HEIGHT, width, height), Target::Note));

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
        toolbar(&mut p, input, column, width, t_dock, t_show)?;
        side_menu(&mut p, input, width, height, s_dock, s_show)?;
    }

    // Quick actions, then the window buttons: one row, all alike.
    let open = input.menu.is_some().then_some(input.menu_anchor).flatten();
    for (i, &(target, glyph)) in actions(input).iter().enumerate() {
        caption_button(&mut p, target, glyph, actions_left(input, width) + i as f32 * CAPTION_WIDTH, open == Some(target))?;
    }
    let buttons = [
        (Target::Minimize, icon::MINIMIZE),
        (Target::Maximize, if input.maximized { icon::RESTORE } else { icon::MAXIMIZE }),
        (Target::Close, icon::CLOSE),
    ];
    if !input.native_caption {
        for (i, (target, glyph)) in buttons.into_iter().enumerate() {
            caption_button(&mut p, target, glyph, caption_left + i as f32 * CAPTION_WIDTH, false)?;
        }
    }
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
const CAPTION_BOTTOM: f32 = 37.0;

/// A button in the top right row (a quick action or a window button): a
/// rounded highlight under the pointer (red for Close) or while its menu is
/// open. It answers from the window's top edge down, so throwing the
/// pointer into the corner still hits it.
fn caption_button(p: &mut Painter, target: Target, glyph: &str, left: f32, open: bool) -> Result<()> {
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
    let small = p.fonts.icon_small.clone();
    p.text(glyph, &small, r, ink)
}

/// The toolbar's buttons, left to right, that fit in `room` DIPs (the bar
/// less its ends and the toggle): all, or without the text size and line
/// width buttons, then without the least used, dropped in `DROPPED` order.
/// Separators stay only between groups that are left.
fn tools_that_fit(room: f32) -> (Vec<Option<Tool>>, bool) {
    const DROPPED: [Tool; 5] = [Tool::Quote, Tool::Code, Tool::Link, Tool::Heading, Tool::List];
    let reading = 3.0 * (TOOL + TOOL_GAP) + SEPARATOR;
    let build = |dropped: usize| {
        let mut out: Vec<Option<Tool>> = Vec::new();
        for t in TOOLS {
            match t {
                Some(t) if DROPPED[..dropped].contains(&t) => {}
                None if out.last().is_none_or(Option::is_none) => {}
                t => out.push(t),
            }
        }
        if out.last() == Some(&None) {
            out.pop();
        }
        out
    };
    let width = |tools: &[Option<Tool>]| tools.iter().map(|t| if t.is_some() { TOOL + TOOL_GAP } else { SEPARATOR }).sum::<f32>();
    let all = build(0);
    if width(&all) + reading <= room {
        return (all, true);
    }
    for dropped in 0..=DROPPED.len() {
        let tools = build(dropped);
        if width(&tools) <= room || dropped == DROPPED.len() {
            return (tools, false);
        }
    }
    unreachable!()
}

fn toolbar(p: &mut Painter, input: &Input, column: Rect, width: f32, dock: f32, show: f32) -> Result<()> {
    let collapsed = dock < 0.5;
    let caption_left = actions_left(input, width);

    // Focus mode: the pill, and the invisible area that reveals the bar.
    // Centered over the note, but between the side menu and the quick
    // actions, shorter where that is narrow.
    let (lo, hi) = ((content_left(input, width) + MARGIN + 8.0).max(top_row_left(input)), caption_left - 16.0);
    let pill_len = (column.width() * 0.26).clamp(120.0, 260.0).min((hi - lo).max(40.0));
    let mid = ((column.left + column.right) / 2.0).min(hi - pill_len / 2.0).max(lo + pill_len / 2.0);
    let pill_box = Rect::new(mid - pill_len / 2.0, 5.0, mid + pill_len / 2.0, 5.0 + PILL);
    if collapsed {
        let zone = Rect::new((pill_box.left - TOP_ZONE_REACH).max(top_row_left(input)), 0.0, pill_box.right + TOP_ZONE_REACH, TOP_ZONE_HEIGHT);
        p.scene.hits.push((zone, Target::RevealToolbar));
        p.scene.pills[1] = Some(pill_box);
        p.pill(pill_box, false, input.toolbar_wave, pill_alpha(show));
    }
    if show <= 0.0 {
        return Ok(());
    }

    // The bar spans the note column. Beside the window buttons it stops
    // short of them; on a row of its own it may reach past the column to
    // hold its buttons.
    let row = toolbar_top(input, width, column.left, column.right);
    let (left, right) = if row == MARGIN {
        ((column.left - 4.0).max(top_row_left(input)), (column.right + 4.0).min(caption_left - MARGIN))
    } else {
        let want = (column.width() + 8.0).max(TOOLBAR_FULL.min(width - 2.0 * MARGIN));
        let mid = (column.left + column.right) / 2.0;
        let left = (mid - want / 2.0).max(content_left(input, width) + MARGIN).max(MARGIN);
        (left, (left + want).min(width - MARGIN))
    };
    let right = right.max(left + TOOL + 8.0);
    let top = row - (TOOLBAR_HEIGHT + row + SHADOW_BLUR) * (1.0 - show);
    let bar = Rect::new(left, top, right, top + TOOLBAR_HEIGHT);
    let floating = (show - dock).clamp(0.0, 1.0);
    if collapsed {
        // Where it is going, not where it is on the way: a pointer that
        // outruns it sliding in is over it all the same.
        let settled = Rect::new(left, row, right, row + TOOLBAR_HEIGHT);
        p.scene.hits.push((settled.inset(-KEEP_OPEN), Target::ToolbarPanel));
    }
    p.panel(bar, floating);
    p.scene.hits.push((bar, Target::ToolbarPanel));

    let y = bar.top + (TOOLBAR_HEIGHT - TOOL) / 2.0;
    let toggle = Rect::new(bar.right - 4.0 - TOOL, y, bar.right - 4.0, y + TOOL);
    // How the page reads, at the right: text size and line width.
    let width = toggle.offset(-(TOOL + TOOL_GAP), 0.0);
    let larger = width.offset(-(TOOL + TOOL_GAP), 0.0);
    let smaller = larger.offset(-(TOOL + TOOL_GAP), 0.0);
    let (tools, reading) = tools_that_fit(toggle.left - TOOL_GAP - (bar.left + 4.0));
    let mut x = bar.left + 4.0;
    for tool in tools {
        let Some(tool) = tool else {
            let m = x + SEPARATOR / 2.0;
            p.scene.fills.push(Fill::solid(Rect::new(m - 0.5, y + 8.0, m + 0.5, y + TOOL - 8.0), 0.0, Ink::Border));
            x += SEPARATOR;
            continue;
        };
        let r = Rect::new(x, y, x + TOOL, y + TOOL);
        // Lit while the caret is in formatting the button makes.
        let active = input.active_tools.contains(&tool);
        if active {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Selection));
        }
        p.button(r, Target::Tool(tool), 6.0);
        let ink = if active || p.hover == Some(Target::Tool(tool)) { Ink::Text } else { Ink::Muted };
        let glyph = match tool {
            Tool::Heading => {
                let format = p.fonts.glyph.clone();
                p.text("H", &format, r, ink)?;
                x += TOOL + TOOL_GAP;
                continue;
            }
            Tool::Bold => icon::BOLD,
            Tool::Italic => icon::ITALIC,
            // Shortcuts only (Ctrl+Shift+X).
            Tool::Strike => icon::STRIKE,
            Tool::List => icon::LIST,
            Tool::Checklist => icon::CHECKLIST,
            Tool::Quote => icon::QUOTE,
            Tool::Code => icon::CODE,
            Tool::Link => icon::LINK,
        };
        p.icon(glyph, r, ink)?;
        x += TOOL + TOOL_GAP;
    }
    if reading {
        // Text size: dimmed at its smallest and largest.
        let size = input.options[1];
        for (r, up, glyph, limit) in [(smaller, false, icon::SMALLER, 1), (larger, true, icon::LARGER, 2)] {
            let target = Target::TextSize(up);
            p.button(r, target, 6.0);
            let ink = if size == limit { Ink::Faint } else if p.hover == Some(target) { Ink::Text } else { Ink::Muted };
            p.icon(glyph, r, ink)?;
        }
        // Line width: a ruler as long as the column is wide.
        p.button(width, Target::LineWidth, 6.0);
        let ink = if p.hover == Some(Target::LineWidth) { Ink::Text } else { Ink::Muted };
        let half = match input.options[2] {
            1 => 4.0,
            2 => 8.5,
            _ => 6.0,
        };
        let (cx, cy) = ((width.left + width.right) / 2.0, (width.top + width.bottom) / 2.0);
        let fills = &mut p.scene.fills;
        fills.push(Fill::solid(Rect::new(cx - half, cy - 0.6, cx + half, cy + 0.6), 0.6, ink));
        for end in [cx - half, cx + half] {
            fills.push(Fill::solid(Rect::new(end - 0.7, cy - 4.5, end + 0.7, cy + 4.5), 0.7, ink));
        }
    }
    // Docked: collapse into focus mode. Revealed: pin it open.
    p.button(toggle, Target::ToggleToolbar, 6.0);
    let ink = if p.hover == Some(Target::ToggleToolbar) { Ink::Text } else { Ink::Faint };
    p.icon(if collapsed { icon::PIN } else { icon::CHEVRON_UP }, toggle, ink)?;
    Ok(())
}

/// New note's keys, as the platform's keyboard writes them: the Command key
/// on Apple's (the iPad's too, seen there as Ctrl+N), Ctrl elsewhere.
fn new_note_keys(platform: Platform) -> &'static str {
    if matches!(platform, Platform::Mac | Platform::Ios) { "\u{2318}N" } else { "Ctrl+N" }
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
    p.panel(panel, floating);
    p.scene.hits.push((panel, Target::SidebarPanel));

    let (x0, x1) = (panel.left + PAD, panel.right - PAD);
    let mut y = side_content_top(input, panel.top);

    // Search field, and the button that collapses (or, revealed, pins) it.
    let toggle = Rect::new(x1 - CONTROL, y, x1, y + CONTROL);
    let search = Rect::new(x0, y, toggle.left - 6.0, y + CONTROL);
    let searching = input.search.as_ref();
    if searching.is_some() {
        p.scene.fills.push(Fill::solid(search, CONTROL_RADIUS, Ink::Border));
        p.scene.fills.push(Fill::solid(search.inset(1.0), CONTROL_RADIUS - 1.0, Ink::Field));
    } else {
        p.scene.fills.push(Fill::solid(search, CONTROL_RADIUS, Ink::Field));
        if p.hover == Some(Target::Search) {
            p.scene.fills.push(Fill::faded(search, CONTROL_RADIUS, Ink::Hover, 0.6));
        }
    }
    p.scene.hits.push((search, Target::Search));
    p.icon(icon::SEARCH, Rect::new(search.left + 4.0, y, search.left + 32.0, y + CONTROL), Ink::Faint)?;
    let label = p.fonts.label.clone();
    let field = Rect::new(search.left + 32.0, y, search.right - 8.0, y + CONTROL);
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
            let caret = Rect::new(field.left + caret_x - 0.75, y + 8.0, field.left + caret_x + 0.75, y + CONTROL - 8.0);
            p.scene.fills.push(Fill::solid(caret, 0.75, Ink::Accent));
        }
        None => p.text("Search", &label, field, Ink::Faint)?,
    }
    p.button(toggle, Target::ToggleSidebar, CONTROL_RADIUS);
    let ink = if p.hover == Some(Target::ToggleSidebar) { Ink::Text } else { Ink::Faint };
    // Revealed where it cannot dock, the button puts it away.
    p.icon(if collapsed && sidebar_fits(width) { icon::PIN } else { icon::COLLAPSE_LEFT }, toggle, ink)?;
    y += CONTROL + 8.0;

    // New note: the menu's first act, a soft button the width of the
    // search above it, its key at its end.
    let new_note = Rect::new(x0, y, x1, y + CONTROL);
    let hot = p.hover == Some(Target::NewNote);
    p.scene.fills.push(Fill::solid(new_note, CONTROL_RADIUS, Ink::Field));
    p.button(new_note, Target::NewNote, CONTROL_RADIUS);
    p.icon(icon::ADD, Rect::new(x0 + 4.0, y, x0 + 32.0, y + CONTROL), if hot { Ink::Text } else { Ink::Muted })?;
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

    // Social: a drawer of the platforms a note previews for.
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
                p.scene.fills.push(Fill::solid(cell, 7.0, Ink::Selection));
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

    y += 12.0;

    // At the bottom, icons only: the trash, appearance, settings.
    let options = Rect::new(x0, panel.bottom - PAD - CONTROL, x1, panel.bottom - PAD);
    let slot = options.width() / 3.0;
    let theme = match input.options[0] {
        1 => icon::SUN,
        2 => icon::MOON,
        _ => icon::SYSTEM,
    };
    for (k, (target, glyph)) in [(Target::Trash, icon::DELETE), (Target::ThemeCycle, theme), (Target::Options, icon::SETTINGS)]
        .into_iter()
        .enumerate()
    {
        let mid = options.left + slot * (k as f32 + 0.5);
        let r = Rect::new(mid - CONTROL / 2.0 - 6.0, options.top, mid + CONTROL / 2.0 + 6.0, options.bottom);
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
    let mut bottom = options.top - 8.0;
    if let Some(version) = input.update {
        let pill = update_pill_rect(x0, x1, options.top);
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

/// Where the update pill goes: the side menu's width, just above its icons
/// (whose top is `icons`).
fn update_pill_rect(x0: f32, x1: f32, icons: f32) -> Rect {
    Rect::new(x0, icons - 8.0 - UPDATE_PILL, x1, icons - 8.0)
}

/// The update pill: a soft field with a small teal dot, "Update ready" over
/// the version, and a Restart button at its end. A click anywhere on it
/// restarts into the update.
fn update_pill(p: &mut Painter, r: Rect, version: &str) -> Result<()> {
    p.scene.fills.push(Fill::solid(r, 14.0, Ink::Field));
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
    p.scene.fills.push(Fill::shadow(button.offset(0.0, 1.0), 15.0, 4.0, 0.8));
    p.scene.fills.push(Fill::solid(button, 15.0, Ink::Raised));
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
    let label = p.fonts.label.clone();
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
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS - 2.0, Ink::Selection));
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
            p.scene.fills.push(Fill::solid(r, CONTROL_RADIUS - 2.0, Ink::Selection));
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
fn trash(p: &mut Painter, entries: &[TrashEntry], x0: f32, x1: f32, top: f32, bottom: f32) -> Result<()> {
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
    /// A row every platform has but one.
    NotOn(Platform, &'static Row),
}

/// The options, in two columns of sections.
const LEFT: &[Section] = &[
    (
        "Appearance",
        &[
            Row::Choice("Theme", Setting::Theme, &[("System", 0), ("Light", 1), ("Dark", 2)]),
            Row::Choice("Font", Setting::Font, &[("Sans", 0), ("Serif", 1), ("Mono", 2)]),
            Row::Choice("Text size", Setting::TextSize, &[("Small", 1), ("Default", 0), ("Large", 2)]),
            Row::Choice("Line width", Setting::LineWidth, &[("Narrow", 1), ("Default", 0), ("Wide", 2)]),
        ],
    ),
    (
        "Writing",
        &[
            Row::Choice("Spelling", Setting::Spell, &[("Marked", 0), ("Not marked", 1)]),
            Row::Choice("Writing help", Setting::Assist, &[("Automatic", 0), ("Off", 1)]),
            Row::Choice("Related notes", Setting::Related, &[("Show", 0), ("Hide", 1)]),
            Row::On(Platform::Windows, &Row::Choice("Reminders", Setting::Remind, &[("On", 0), ("Off", 1)])),
        ],
    ),
    ("Updates", &[Row::Updater(&Row::Choice("Check for updates", Setting::Updates, &[("On", 0), ("Off", 1)]))]),
];

const RIGHT: &[Section] = &[
    (
        "Opening and closing",
        &[
            Row::Choice("At launch", Setting::Launch, &[("Smart", 0), ("Last note", 1), ("New page", 2)]),
            // iOS sizes its windows itself.
            Row::NotOn(Platform::Ios, &Row::Choice("Window", Setting::Window, &[("Full", 0), ("Minimal", 1)])),
            Row::On(Platform::Windows, &Row::Choice("Tray icon", Setting::Tray, &[("Off", 1), ("On", 0), ("Stay running", 2)])),
            Row::On(Platform::Mac, &Row::Choice("Menu bar icon", Setting::Tray, &[("Off", 1), ("On", 0), ("Stay running", 2)])),
            Row::On(Platform::Windows, &Row::Choice("Quick note", Setting::Hotkey, &[("Win+Alt+N", 0), ("Off", 1)])),
            Row::On(Platform::Mac, &Row::Choice("Quick note", Setting::Hotkey, &[("Ctrl+Opt+N", 0), ("Off", 1)])),
        ],
    ),
    (
        "Vault",
        &[
            Row::Vault,
            Row::NotOn(Platform::Ios, &Row::Buttons("", &[("Open another\u{2026}", Target::VaultOpen), ("Show in folder", Target::VaultShow)])),
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
const LABEL_W: f32 = 112.0;

/// The rows of a section this platform has, and the sections left with any.
fn shown(sections: &'static [Section], input: &Input) -> Vec<(&'static str, Vec<&'static Row>)> {
    sections
        .iter()
        .map(|(name, rows)| {
            let rows: Vec<&'static Row> = rows
                .iter()
                .filter_map(|r| match r {
                    Row::On(only, row) => (*only == input.platform).then_some(*row),
                    Row::Updater(row) => input.updater.then_some(*row),
                    Row::NotOn(not, row) => (*not != input.platform).then_some(*row),
                    row => Some(row),
                })
                .collect();
            (*name, rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

fn column_height(sections: &'static [Section], input: &Input) -> f32 {
    shown(sections, input)
        .iter()
        .map(|(_, rows)| SECTION_H + SECTION_GAP + rows.len() as f32 * ROW_H)
        .sum()
}

/// A row of buttons (or, with `setting`, a choice of one) in `bar`.
fn segments(p: &mut Painter, input: &Input, bar: Rect, items: &[(&str, Target)], chosen: Option<Target>) -> Result<()> {
    let choice = p.fonts.choice.clone();
    let choosing = chosen.is_some();
    if choosing {
        p.scene.fills.push(Fill::solid(bar, 8.0, Ink::Field));
    }
    let w = bar.width() / items.len() as f32;
    for (k, &(label, target)) in items.iter().enumerate() {
        let r = Rect::new(bar.left + k as f32 * w, bar.top, bar.left + (k + 1) as f32 * w, bar.bottom).inset(2.0);
        let on = chosen == Some(target);
        let hover = input.hover == Some(target);
        if on {
            p.scene.fills.push(Fill::shadow(r.offset(0.0, 1.0), 6.0, 4.0, 0.8));
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Raised));
        } else if hover {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Hover));
        } else if !choosing {
            p.scene.fills.push(Fill::solid(r, 6.0, Ink::Field));
        }
        p.scene.hits.push((r, target));
        p.text(label, &choice, r, if on || hover { Ink::Text } else { Ink::Muted })?;
    }
    Ok(())
}

fn options_column(p: &mut Painter, input: &Input, sections: &'static [Section], left: f32, right: f32, top: f32) -> Result<()> {
    let section = p.fonts.section.clone();
    let label = p.fonts.label.clone();
    let mut y = top;
    for (name, rows) in shown(sections, input) {
        p.text(name, &section, Rect::new(left, y, right, y + SECTION_H), Ink::Faint)?;
        y += SECTION_H;
        for row in rows {
            let bar = Rect::new(left + LABEL_W, y + 5.0, right, y + ROW_H - 5.0);
            match row {
                Row::Choice(name, setting, choices) => {
                    p.text(name, &label, Rect::new(left, y, left + LABEL_W - 8.0, y + ROW_H), Ink::Muted)?;
                    let items: Vec<(&str, Target)> = choices.iter().map(|&(l, v)| (l, Target::Choice(*setting, v))).collect();
                    let value = input.options[setting.index()];
                    segments(p, input, bar, &items, Some(Target::Choice(*setting, value)))?;
                    y += ROW_H;
                }
                Row::Buttons(name, items) => {
                    p.text(name, &label, Rect::new(left, y, left + LABEL_W - 8.0, y + ROW_H), Ink::Muted)?;
                    segments(p, input, bar, items, None)?;
                    y += ROW_H;
                }
                Row::Vault => {
                    p.text("Folder", &label, Rect::new(left, y, left + LABEL_W - 8.0, y + ROW_H), Ink::Muted)?;
                    let l = scene::single_line(p.eng, input.vault_path, &label, right - left - LABEL_W, ROW_H);
                    p.scene.texts.push(Text { x: left + LABEL_W + 4.0, y, layout: l, ink: Ink::Text, alpha: 1.0 });
                    y += ROW_H;
                }
                // Resolved by `shown`.
                Row::On(..) | Row::Updater(_) | Row::NotOn(..) => {}
            }
        }
        y += SECTION_GAP;
    }
    Ok(())
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
    let header = 62.0;
    let footer = 44.0;
    let h = (header + body + footer).min(height - 32.0);
    let card = Rect::new((width - w) / 2.0, ((height - h) / 2.0).max(16.0), (width + w) / 2.0, ((height - h) / 2.0).max(16.0) + h);
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
    let scroll = input.options_scroll.clamp(0.0, max);
    p.scene.overlay(Some(view));
    let hits_from = p.scene.hits.len();
    let top = view.top - scroll;
    if two {
        let mid = (card.left + card.right) / 2.0;
        options_column(&mut p, input, LEFT, card.left + 28.0, mid - 20.0, top)?;
        options_column(&mut p, input, RIGHT, mid + 20.0, card.right - 28.0, top)?;
    } else {
        options_column(&mut p, input, LEFT, card.left + 20.0, card.right - 20.0, top)?;
        options_column(&mut p, input, RIGHT, card.left + 20.0, card.right - 20.0, top + column_height(LEFT, input))?;
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
    let mut about = format!("Kolada {}", env!("CARGO_PKG_VERSION"));
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
    let y = if m.y + h > height - 8.0 { (m.y - h).max(8.0) } else { m.y };
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
        p.button(r, Target::MenuItem(i), 6.0);
        let (icon_ink, ink) = if item.danger { (Ink::Danger, Ink::Danger) } else { (Ink::Muted, Ink::Text) };
        if !item.icon.is_empty() {
            p.icon(item.icon, Rect::new(r.left + 2.0, y, r.left + 30.0, y + ITEM), icon_ink)?;
        }
        p.text(&item.label, &label, Rect::new(r.left + 34.0, y, r.right - 12.0 - keys_room, y + ITEM), ink)?;
        if !item.keys.is_empty() {
            let l = scene::layout(eng, item.keys, &Format { align: Align::End, ..small }, keys + 4.0, ITEM);
            p.scene.texts.push(Text { x: r.right - 12.0 - keys - 4.0, y, layout: l, ink: Ink::Faint, alpha: 1.0 });
        }
        y += ITEM;
    }
    let a = m.alpha.clamp(0.0, 1.0);
    if a < 1.0 {
        scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= a);
        scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= a);
    }
    Ok(())
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
            options: [0; 13],
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
        for (width, side) in [(1268.0, (1.0, 1.0)), (1268.0, (0.0, 0.0)), (1400.0, (1.0, 1.0)), (2400.0, (1.0, 1.0))] {
            let (_, column) = scene_for(width, side, (1.0, 1.0), &[]);
            let (left, w) = scene::margin(column, width).expect("room for the margin");
            assert!(left + w <= width - 24.0 + 0.5, "chips fit inside the window at {width}");
            assert!(column.width() >= MIN_COLUMN_BESIDE_MARGIN, "the note stays readable at {width}");
        }
        // Too narrow: the column keeps its width and the margin is dropped.
        let (_, narrow) = scene_for(900.0, (1.0, 1.0), (1.0, 1.0), &[]);
        assert!(scene::margin(narrow, 900.0).is_none());
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
        assert!(scene.fills.iter().any(|f| f.rect == new_note && f.ink == Ink::Field), "a filled button");
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
                        compact: state == "minimal",
                        no_margin: state == "minimal",
                        platform: if state.ends_with(" mac") { Platform::Mac } else if state.ends_with(" ios") { Platform::Ios } else { Platform::Windows },
                        native_caption: state.ends_with(" mac"),
                        // An update ready: its pill above the side menu's
                        // icons, and the switch in the options.
                        updater: !state.ends_with(" mac") && !state.ends_with(" ios"),
                        update: Some("0.10.0"),
                        ..Default::default()
                    };
                    let input = make(None);
                    let menu = menu_width(set, width);
                    if state == "minimal" {
                        let scene = scene::build(&eng, &fonts, &input, width, height).unwrap();
                        assert!(rect_of(&scene, Target::SidebarPanel).is_none(), "the minimal window has no side menu, {state} at {width} x {height}");
                        assert!(scene.pills[1].is_none() && rect_of(&scene, Target::RevealToolbar).is_none() && rect_of(&scene, Target::ToggleToolbar).is_none(), "the minimal window has no toolbar, {state} at {width} x {height}");
                        let (grow, send) = (rect_of(&scene, Target::ToMain).expect("its way back"), rect_of(&scene, Target::SendTo).unwrap());
                        assert!(grow.left >= send.right - 0.5 && rect_of(&scene, Target::Minimal).is_none(), "it grows back beside Send, {state} at {width} x {height}");
                    } else if !options_open {
                        let scene = scene::build(&eng, &fonts, &input, width, height).unwrap();
                        let (shrink, send) = (rect_of(&scene, Target::Minimal).expect("the minimal button"), rect_of(&scene, Target::SendTo).unwrap());
                        assert!(shrink.left >= send.right - 0.5 && shrink.right <= width, "Minimal beside Send, {state} at {width} x {height}");
                    }
                    let at = format!("{state} at {width} x {height}");
                    let scene = scene::build(&eng, &fonts, &input, width, height).unwrap();
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
                    } else if let Some(pill) = scene.pills[1] {
                        let minimize = rect_of(&scene, Target::SendTo).unwrap();
                        assert!(pill.right <= minimize.left - 8.0, "the toolbar's pill under the window buttons, {at}");
                        if side.0 > 0.0 {
                            assert!(pill.left >= MARGIN + menu, "the toolbar's pill under the side menu, {at}");
                        }
                    }
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 141 * 5 * 18, "every size and state");
    }

    /// The settings (and buttons) the options card offers on a platform.
    fn offered(platform: Platform, options: [u8; scene::SETTINGS], updater: bool) -> (Vec<Setting>, Vec<Target>) {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let input = Input { vault_name: "notes", options_open: true, options, platform, updater, page_fade: 1.0, sidebar_dock: 1.0, sidebar_show: 1.0, toolbar_dock: 1.0, toolbar_show: 1.0, ..Default::default() };
        let scene = scene::build(&eng, &fonts, &input, 1000.0, 1600.0).unwrap();
        assert_eq!(scene.options_scroll_max, 0.0, "everything fits a tall window");
        let mut settings: Vec<Setting> = Vec::new();
        for (_, t) in &scene.hits {
            if let Target::Choice(s, _) = t {
                if !settings.contains(s) {
                    settings.push(*s);
                }
            }
        }
        let buttons = scene.hits.iter().map(|(_, t)| *t).filter(|t| matches!(t, Target::Transfer(_) | Target::VaultOpen | Target::VaultShow)).collect();
        (settings, buttons)
    }

    #[test]
    fn each_platform_offers_only_the_options_it_has() {
        let (win, buttons) = offered(Platform::Windows, [0; scene::SETTINGS], true);
        for s in [Setting::Theme, Setting::Font, Setting::TextSize, Setting::LineWidth, Setting::Spell, Setting::Assist, Setting::Related, Setting::Launch, Setting::Window, Setting::Tray, Setting::Hotkey, Setting::Remind] {
            assert!(win.contains(&s), "Windows offers {s:?}");
        }
        assert!(buttons.contains(&Target::VaultOpen) && buttons.contains(&Target::VaultShow) && buttons.contains(&Target::Transfer(Transfer::Pdf)));
        // The Mac offers its own menu bar icon and quick note keys in their place.
        let (mac, mac_buttons) = offered(Platform::Mac, [0; scene::SETTINGS], false);
        for s in [Setting::Theme, Setting::Font, Setting::TextSize, Setting::LineWidth, Setting::Spell, Setting::Assist, Setting::Related, Setting::Launch, Setting::Window, Setting::Tray, Setting::Hotkey] {
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
        for s in [Setting::Theme, Setting::Font, Setting::TextSize, Setting::LineWidth, Setting::Spell, Setting::Assist, Setting::Related, Setting::Launch] {
            assert!(ios.contains(&s), "iOS offers {s:?}");
        }
        for s in [Setting::Tray, Setting::Hotkey, Setting::Window, Setting::Remind, Setting::Updates] {
            assert!(!ios.contains(&s), "iOS has no {s:?}");
        }
        assert!(ios_buttons.contains(&Target::VaultOpen) && !ios_buttons.contains(&Target::VaultShow));
    }

    #[test]
    fn new_note_shows_the_keys_of_the_platforms_keyboard() {
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
                    // The update switch shows only in Windows' website build,
                    // so only Windows' fonts measure it (in the Mac's
                    // Helvetica Neue it is 3 DIPs past the column).
                    Row::Updater(_) if !cfg!(windows) => continue,
                    Row::On(_, r) | Row::Updater(r) | Row::NotOn(_, r) => *r,
                    r => r,
                };
                if let Row::Choice(name, ..) | Row::Buttons(name, _) = row {
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

    #[test]
    fn an_update_ready_shows_as_a_pill_above_the_side_menus_icons() {
        let scene = with_update(1200.0, 800.0, None);
        let pill = rect_of(&scene, Target::Update).expect("the pill");
        let panel = rect_of(&scene, Target::SidebarPanel).unwrap();
        let icons: Vec<Rect> = [Target::Trash, Target::ThemeCycle, Target::Options].map(|t| rect_of(&scene, t).unwrap()).to_vec();
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
        // Restart button at rest, raised, at its right end.
        let inside = |t: &&Text| t.x >= pill.left && t.x < pill.right && t.y >= pill.top - 0.5 && t.y < pill.bottom;
        let texts: Vec<&Text> = scene.texts.iter().filter(inside).collect();
        assert_eq!(texts.len(), 3, "Update ready, the version, Restart");
        assert!(texts.iter().any(|t| t.ink == Ink::Faint) && texts.iter().filter(|t| t.ink != Ink::Faint).count() == 2);
        let button = scene.fills.iter().find(|f| f.ink == Ink::Raised && f.rect.left > (pill.left + pill.right) / 2.0 && f.rect.right <= pill.right && f.rect.top >= pill.top && f.rect.bottom <= pill.bottom);
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
