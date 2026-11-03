//! The command palette (Ctrl+K at a bare caret, Ctrl+P): a panel that drops
//! from the top of the note column, with a field to type a command in and
//! the commands that match below it. The chosen one, what Enter runs, has a
//! soft grey fill across its row that glides from row to row; every row is
//! in the text's ink, its keys faint at the right. Opened with nothing
//! typed, it shows the few commands run last, or the field alone. The
//! commands are the shared list (`commands`); each shell runs them.

use kolada_text::Format;
use windows_core::Result;

use crate::geom::{Fill, Ink, Rect};
use crate::scene::{self, Eng, Fonts, MenuItem, Scene, Target, Text};

/// The panel's width at most, its padding, the field's height and the
/// gap under it, a row's height, and the rows shown at once.
pub const WIDTH: f32 = 520.0;
pub const PAD: f32 = 8.0;
pub const FIELD: f32 = 40.0;
pub const GAP: f32 = 6.0;
pub const ROW: f32 = 36.0;
pub const SHOWN: usize = 8;
/// How far it drops in from, and how far below the window's top it rests.
const DROP: f32 = 18.0;
const TOP: f32 = 10.0;
/// What shows in the field before anything is typed.
pub const PLACEHOLDER: &str = "Type a command";

/// The palette as the view shows it. `drop`, `height` and `fill` are
/// springs the shell steps, so each can be caught mid-way and still land.
pub struct PaletteView<'a> {
    pub query: &'a str,
    pub caret: usize,
    /// The commands that match, best first: name and keys.
    pub rows: &'a [MenuItem],
    /// The first row shown (the list scrolls to keep the chosen one in
    /// view), and the chosen one.
    pub first: usize,
    pub chosen: usize,
    /// How far it has dropped in, 0..1.
    pub drop: f32,
    /// The rows' room, in rows (it grows and shrinks as they change).
    pub height: f32,
    /// Where the chosen row's fill is, in rows below the first shown.
    pub fill: f32,
}

/// The rows' room for `n` commands matching: up to `SHOWN`.
pub fn rows_for(n: usize) -> f32 {
    n.min(SHOWN) as f32
}

/// The panel at rest, for a window `width` wide whose note column is
/// `column`: as wide as the column (at most `WIDTH`), centred on it, 16
/// DIPs from each edge in a narrow window; `rows` tall.
pub fn card(width: f32, column: Rect, rows: f32) -> Rect {
    let w = WIDTH.min(column.width().max(WIDTH.min(width - 32.0))).min(width - 32.0).max(1.0);
    let mid = ((column.left + column.right) / 2.0).clamp(16.0 + w / 2.0, (width - 16.0 - w / 2.0).max(16.0 + w / 2.0));
    let rows_room = if rows > 0.0 { GAP + rows * ROW } else { 0.0 };
    Rect::new(mid - w / 2.0, TOP, mid + w / 2.0, TOP + PAD + FIELD + rows_room + PAD)
}

pub fn draw(eng: &Eng, fonts: &Fonts, v: &PaletteView, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    let a = v.drop.clamp(0.0, 1.0);
    if a <= 0.01 {
        return Ok(());
    }
    scene.overlay(None);
    // A click anywhere else closes it.
    scene.hits.push((Rect::new(0.0, 0.0, width, height), Target::PaletteVeil));
    let at_rest = card(width, scene.column, v.height.max(0.0));
    let card = at_rest.offset(0.0, -DROP * (1.0 - v.drop));
    let card = Rect::new(card.left, card.top, card.right, card.bottom.min(height - 8.0));
    scene.fills.push(Fill::shadow(card.offset(0.0, 1.0), 14.0, 3.0, 0.5 * a));
    scene.fills.push(Fill::shadow(card.offset(0.0, 7.0), 14.0, 22.0, a));
    scene.fills.push(Fill::faded(card, 14.0, Ink::Panel, a));
    scene.hits.push((card, Target::PalettePanel));
    // The field: a filled bar, the caret the only teal.
    let field = Rect::new(card.left + PAD, card.top + PAD, card.right - PAD, card.top + PAD + FIELD);
    scene.fills.push(Fill::faded(field, 10.0, Ink::Field, a));
    scene.hits.push((field, Target::PaletteField));
    let inner = Rect::new(field.left + 14.0, field.top, field.right - 12.0, field.bottom);
    let format = fonts.label;
    let shown = if v.query.is_empty() { PLACEHOLDER } else { v.query };
    let l = scene::kept_layout(eng, shown, &format, inner.width(), inner.height(), 0, || {
        scene::layout_styled(eng, shown, &Format { rtl: Some(scene::is_rtl(shown)), ..format }, inner.width(), inner.height(), &[])
    });
    let caret_x = if v.query.is_empty() { l.caret(0).x } else { l.caret(v.caret.min(v.query.len())).x };
    scene.texts.push(Text { x: inner.left, y: inner.top, layout: l, ink: if v.query.is_empty() { Ink::Faint } else { Ink::Text }, alpha: a });
    let c = Rect::new(inner.left + caret_x - 0.75, field.top + 10.0, inner.left + caret_x + 0.75, field.bottom - 10.0);
    scene.fills.push(Fill::faded(c, 0.75, Ink::Accent, a));
    if v.rows.is_empty() || v.height <= 0.01 {
        return Ok(());
    }
    // The rows, clipped to the room the panel has grown to.
    let top = field.bottom + GAP;
    let clip = Rect::new(card.left, top, card.right, card.bottom - PAD + 1.0);
    scene.overlay(Some(clip));
    let fill = Rect::new(card.left + PAD, top + v.fill * ROW, card.right - PAD, top + v.fill * ROW + ROW);
    scene.fills.push(Fill::faded(fill, 8.0, Ink::Hover, a));
    let small = fonts.small;
    for (k, item) in v.rows.iter().enumerate().skip(v.first).take(SHOWN + 1) {
        let y = top + (k - v.first) as f32 * ROW;
        if y > clip.bottom {
            break;
        }
        let r = Rect::new(card.left + PAD, y, card.right - PAD, y + ROW);
        scene.hits.push((Rect::new(r.left, r.top.max(clip.top), r.right, r.bottom.min(clip.bottom)), Target::PaletteRow(k)));
        let keys_w = if item.keys.is_empty() {
            0.0
        } else {
            let keys = scene::single_line(eng, item.keys, &small, 160.0, ROW);
            let w = keys.measure().width;
            scene.texts.push(Text { x: r.right - 12.0 - w, y, layout: keys, ink: Ink::Faint, alpha: a });
            w + 16.0
        };
        let name = scene::single_line(eng, &item.label, &format, (r.width() - 24.0 - keys_w).max(1.0), ROW);
        scene.texts.push(Text { x: r.left + 12.0, y, layout: name, ink: Ink::Text, alpha: a });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Input, build};

    #[test]
    fn it_sits_on_the_note_column_and_keeps_off_a_narrow_windows_edges() {
        let column = Rect::new(300.0, 80.0, 1000.0, 900.0);
        let r = card(1400.0, column, 3.0);
        assert_eq!((r.width(), (r.left + r.right) / 2.0), (WIDTH, 650.0), "as wide as it may be, over the column's middle");
        assert_eq!(r.height(), PAD + FIELD + GAP + 3.0 * ROW + PAD);
        assert_eq!(card(1400.0, column, 0.0).height(), PAD + FIELD + PAD, "nothing to list: the field alone");
        let snap = card(380.0, Rect::new(24.0, 80.0, 356.0, 600.0), 2.0);
        assert_eq!((snap.left, snap.right), (16.0, 364.0), "16 DIPs from each edge of a side-by-side snap");
    }

    #[test]
    fn a_filled_field_the_chosen_row_on_grey_and_teal_only_for_the_caret() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let rows = [MenuItem { label: "Move line up".into(), keys: "Alt+\u{2191}", ..MenuItem::default() }, MenuItem { label: "Move line down".into(), keys: "Alt+\u{2193}", ..MenuItem::default() }];
        let view = PaletteView { query: "move", caret: 4, rows: &rows, first: 0, chosen: 1, drop: 1.0, height: 2.0, fill: 1.0 };
        let scene = build(&eng, &fonts, &Input { palette: Some(view), page_fade: 1.0, ..Default::default() }, 1200.0, 800.0).unwrap();
        let panel = scene.hits.iter().find(|(_, t)| *t == Target::PalettePanel).map(|(r, _)| *r).unwrap();
        let inside: Vec<&Fill> = scene.fills.iter().filter(|f| f.rect.left >= panel.left && f.rect.right <= panel.right && f.rect.top >= panel.top && f.rect.bottom <= panel.bottom && f.blur == 0.0).collect();
        assert!(inside.iter().all(|f| !f.ring && f.ink != Ink::Border), "no outlines");
        assert_eq!(inside.iter().filter(|f| f.ink == Ink::Field).count(), 1, "the field is a filled bar");
        assert_eq!(inside.iter().filter(|f| f.ink == Ink::Accent).count(), 1, "the caret is the only teal");
        let chosen = scene.hits.iter().find(|(_, t)| *t == Target::PaletteRow(1)).map(|(r, _)| *r).unwrap();
        assert!(inside.iter().any(|f| f.ink == Ink::Hover && (f.rect.top - chosen.top).abs() < 0.5), "the chosen row on a soft grey fill");
        let names: Vec<(&str, Ink)> = scene.texts.iter().map(|t| (t.layout.text(), t.ink)).filter(|(s, _)| s.starts_with("Move")).collect();
        assert!(names.len() == 2 && names.iter().all(|&(_, ink)| ink == Ink::Text), "every row in the text's ink: {names:?}");
        assert!(scene.texts.iter().any(|t| t.layout.text() == "Alt+\u{2191}" && t.ink == Ink::Faint), "keys faint at the right");
        let empty = PaletteView { query: "", caret: 0, rows: &[], first: 0, chosen: 0, drop: 1.0, height: 0.0, fill: 0.0 };
        let scene = build(&eng, &fonts, &Input { palette: Some(empty), page_fade: 1.0, ..Default::default() }, 1200.0, 800.0).unwrap();
        assert!(scene.texts.iter().any(|t| t.layout.text() == PLACEHOLDER && t.ink == Ink::Faint));
        assert!(!scene.hits.iter().any(|(_, t)| matches!(t, Target::PaletteRow(_))));
    }
}
