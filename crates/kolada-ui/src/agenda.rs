//! The Today card: what is due, late first, then today, then the coming
//! week, in a small card that opens at the teal dot beside the note's dates
//! (or below the toolbar when the dot is out of sight) and closes like a
//! menu. Every shell draws it from an `AgendaView`; what is due comes from
//! `kolada_core::tasks`.

use kolada_text::{Align, Format};
use windows_core::Result;

use crate::geom::{Fill, Ink, Rect};
use crate::scene::{self, Eng, Fonts, Scene, Target, Text};

/// Which part of the sheet a row is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Late,
    Today,
    Coming,
}

/// A task as the sheet shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub part: Part,
    pub text: String,
    /// The note it is in.
    pub note: String,
    /// When, in a few words: `2 days late`, `4pm`, `Fri`.
    pub when: String,
}

/// The sheet, as the view shows it.
pub struct AgendaView<'a> {
    /// How far open it is: 0 gone, 1 open (a spring may take it a little
    /// past); it fades and drops into place as it opens.
    pub open: f32,
    pub rows: &'a [Row],
    /// Late or due today.
    pub now: usize,
    /// The row the keys are on.
    pub selected: Option<usize>,
}

const ROW: f32 = 34.0;
const HEAD: f32 = 38.0;
const PART: f32 = 28.0;
const PAD: f32 = 12.0;
const BOX: f32 = 14.0;
/// The widest the card gets, DIPs.
const WIDEST: f32 = 440.0;
/// Where it opens when the dot is out of sight: below the toolbar.
const TOP: f32 = 64.0;
/// Its space below the dot, and from the window's edges.
const GAP: f32 = 8.0;
const EDGE: f32 = 12.0;

/// Where each row goes, and the card's height, for `rows` in at most `most`
/// DIPs: rows that do not fit are left off, the last line saying how many
/// more.
fn plan(rows: &[Row], most: f32) -> (Vec<(usize, f32, bool)>, f32, usize) {
    let most = most.max(HEAD + ROW + PAD);
    let mut placed = Vec::new();
    let mut y = HEAD;
    let mut part = None;
    for (i, r) in rows.iter().enumerate() {
        // A heading above the coming week (late and today read as one).
        let heading = r.part == Part::Coming && part != Some(Part::Coming) && i > 0;
        let need = ROW + if heading { PART } else { 0.0 };
        if y + need + ROW + PAD > most && i + 1 < rows.len() {
            return (placed, y + ROW + PAD, rows.len() - i);
        }
        if heading {
            y += PART;
        }
        placed.push((i, y, heading));
        part = Some(r.part);
        y += ROW;
    }
    let h = if rows.is_empty() { HEAD + ROW + PAD } else { y + PAD };
    (placed, h, 0)
}

/// The card's rectangle in a window: below the teal dot (`dot`) when it is
/// in sight, else at the top of the note's column; inside the window.
pub fn card_rect(dot: Option<Rect>, column: Rect, width: f32, height: f32, rows: &[Row]) -> (Rect, f32) {
    let w = column.width().clamp(260.0, WIDEST).min(width - 2.0 * EDGE);
    let (left, top) = match dot {
        Some(d) => (d.left - 24.0, d.bottom + GAP),
        None => (column.left, TOP),
    };
    let left = left.clamp(EDGE, (width - EDGE - w).max(EDGE));
    let top = top.min(height * 0.5);
    let most = height - top - EDGE;
    let (_, h, _) = plan(rows, most);
    (Rect::new(left, top, left + w, top + h), most)
}

pub fn draw(eng: &Eng, fonts: &Fonts, view: &AgendaView, hover: Option<Target>, width: f32, height: f32, scene: &mut Scene) -> Result<()> {
    if view.open <= 0.001 {
        return Ok(());
    }
    scene.overlay(None);
    let open = view.open.clamp(0.0, 1.0);
    let (fills_from, texts_from) = (scene.fills.len(), scene.texts.len());
    // A click anywhere else lets it go, as a menu's does.
    scene.hits.push((Rect::new(0.0, 0.0, width, height), Target::AgendaShade));
    let (r, most) = card_rect(scene.due_dot, scene.column, width, height, view.rows);
    // It drops a few DIPs into place as it opens.
    let r = r.offset(0.0, -6.0 * (1.0 - view.open));
    scene.fills.push(Fill::shadow(r.offset(0.0, 6.0), 12.0, 24.0, 1.4));
    scene.fills.push(Fill::solid(r, 12.0, Ink::Raised));
    scene.hits.push((r, Target::AgendaPanel));
    let small = fonts.small.clone();
    let label = fonts.label.clone();
    let head = if view.now == 0 { "Today".to_string() } else { format!("Today \u{00B7} {}", view.now) };
    let l = scene::single_line(eng, &head, &small, r.width() - 2.0 * PAD, 20.0);
    scene.texts.push(Text { x: r.left + PAD + 4.0, y: r.top + 12.0, layout: l, ink: if view.now > 0 { Ink::Accent } else { Ink::Muted }, alpha: 1.0 });
    let (placed, _, more) = plan(view.rows, most);
    if view.rows.is_empty() {
        let l = scene::single_line(eng, "Nothing due. Tasks with an @date show up here.", &Format { ellipsis: true, ..small.clone() }, r.width() - 2.0 * PAD - 4.0, ROW);
        scene.texts.push(Text { x: r.left + PAD + 4.0, y: r.top + HEAD, layout: l, ink: Ink::Faint, alpha: 1.0 });
    }
    for &(i, y, heading) in &placed {
        let row = &view.rows[i];
        let top = r.top + y;
        if heading {
            let l = scene::single_line(eng, "Coming up", &small, r.width() - 2.0 * PAD, 20.0);
            scene.texts.push(Text { x: r.left + PAD + 4.0, y: top - PART + 8.0, layout: l, ink: Ink::Faint, alpha: 1.0 });
        }
        let line = Rect::new(r.left + 6.0, top, r.right - 6.0, top + ROW);
        let open_target = Target::AgendaOpen(i);
        if hover == Some(open_target) || view.selected == Some(i) {
            scene.fills.push(Fill::solid(line, 8.0, Ink::Hover));
        }
        scene.hits.push((line, open_target));
        // The box: ticked here, ticked in its note.
        let b = Rect::new(line.left + PAD - 4.0, top + (ROW - BOX) / 2.0, line.left + PAD - 4.0 + BOX, top + (ROW + BOX) / 2.0);
        let tick = Target::AgendaTick(i);
        scene.fills.push(Fill::solid(b, 4.0, if hover == Some(tick) { Ink::Accent } else { Ink::Field }));
        scene.hits.push((b.inset(-6.0), tick));
        let coming = row.part == Part::Coming;
        let text_left = b.right + 12.0;
        // The note's name at the end, then when, then the task in the room left.
        let note_w = if line.width() > 380.0 { (line.width() * 0.24).min(110.0) } else { 0.0 };
        let when_w = 84.0;
        let text_w = (line.right - PAD - note_w - when_w - text_left - 8.0).max(40.0);
        let t = scene::single_line(eng, &row.text, &Format { ellipsis: true, ..label.clone() }, text_w, ROW);
        scene.texts.push(Text { x: text_left, y: top, layout: t, ink: if coming { Ink::Muted } else { Ink::Text }, alpha: 1.0 });
        let w = scene::single_line(eng, &row.when, &Format { align: Align::End, ..small.clone() }, when_w, ROW);
        let when_ink = if row.part == Part::Late { Ink::Accent } else { Ink::Faint };
        scene.texts.push(Text { x: line.right - PAD - note_w - when_w, y: top, layout: w, ink: when_ink, alpha: 1.0 });
        if note_w > 0.0 {
            let n = scene::single_line(eng, &row.note, &Format { align: Align::End, ellipsis: true, ..small.clone() }, note_w, ROW);
            scene.texts.push(Text { x: line.right - PAD - note_w, y: top, layout: n, ink: Ink::Faint, alpha: 1.0 });
        }
    }
    if more > 0 {
        let y = placed.last().map_or(HEAD, |p| p.1 + ROW);
        let l = scene::single_line(eng, &format!("and {more} more"), &small, r.width() - 2.0 * PAD, ROW);
        scene.texts.push(Text { x: r.left + PAD + 4.0, y: r.top + y, layout: l, ink: Ink::Faint, alpha: 1.0 });
    }
    if open < 1.0 {
        scene.fills[fills_from..].iter_mut().for_each(|f| f.alpha *= open);
        scene.texts[texts_from..].iter_mut().for_each(|t| t.alpha *= open);
    }
    Ok(())
}

/// When a task is due, in a few words, from today: `3 days late`,
/// `Yesterday`, `4pm` (today), `Tomorrow`, `Fri`.
pub fn when_words(day: kolada_core::when::Day, time: Option<(u32, u32)>, today: kolada_core::when::Day) -> String {
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let days = day.number() - today.number();
    let clock = time.map(|(h, m)| {
        let (h12, half) = (if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "am" } else { "pm" });
        if m == 0 { format!("{h12}{half}") } else { format!("{h12}:{m:02}{half}") }
    });
    match days {
        d if d < -1 => format!("{} days late", -d),
        -1 => "Yesterday".to_string(),
        0 => clock.unwrap_or_default(),
        1 => clock.map_or("Tomorrow".to_string(), |c| format!("Tomorrow {c}")),
        _ => {
            let name = NAMES[day.weekday() as usize];
            clock.map_or(name.to_string(), |c| format!("{name} {c}"))
        }
    }
}

/// The agenda as the sheet's rows.
pub fn rows(a: &kolada_core::tasks::Agenda, today: kolada_core::when::Day) -> Vec<Row> {
    let row = |t: &kolada_core::tasks::Task, part| {
        let file = t.path.rsplit('/').next().unwrap_or(&t.path);
        Row { part, text: t.text.clone(), note: file.strip_suffix(".md").unwrap_or(file).to_string(), when: when_words(t.day, t.time, today) }
    };
    a.late.iter().map(|t| row(t, Part::Late)).chain(a.today.iter().map(|t| row(t, Part::Today))).chain(a.coming.iter().map(|t| row(t, Part::Coming))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_core::when::Day;

    const TODAY: Day = Day { year: 2026, month: 10, day: 5 };

    fn row(part: Part, text: &str) -> Row {
        Row { part, text: text.into(), note: "Note".into(), when: String::new() }
    }

    #[test]
    fn when_is_said_in_a_few_words() {
        assert_eq!(when_words(Day { day: 2, ..TODAY }, None, TODAY), "3 days late");
        assert_eq!(when_words(Day { day: 4, ..TODAY }, None, TODAY), "Yesterday");
        assert_eq!(when_words(TODAY, Some((16, 0)), TODAY), "4pm");
        assert_eq!(when_words(TODAY, None, TODAY), "");
        assert_eq!(when_words(Day { day: 6, ..TODAY }, Some((9, 30)), TODAY), "Tomorrow 9:30am");
        assert_eq!(when_words(Day { day: 9, ..TODAY }, None, TODAY), "Fri");
    }

    #[test]
    fn rows_that_do_not_fit_are_counted_not_squeezed() {
        let many: Vec<Row> = (0..40).map(|i| row(if i < 30 { Part::Today } else { Part::Coming }, "x")).collect();
        let (placed, h, more) = plan(&many, 500.0);
        assert!(more > 0 && placed.len() + more == 40);
        assert!(h <= 500.0, "{h}");
        let few = [row(Part::Late, "a"), row(Part::Today, "b"), row(Part::Coming, "c")];
        let (placed, _, more) = plan(&few, 900.0);
        assert_eq!(more, 0);
        assert_eq!(placed.iter().map(|p| p.2).collect::<Vec<_>>(), [false, false, true], "a heading above the coming week only");
    }

    #[test]
    fn the_card_opens_at_the_dot_and_holds_at_every_window_size() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let rows = vec![row(Part::Late, "Send the invoice to the client before the weekend"), row(Part::Today, "Pay deposit"), row(Part::Coming, "Call")];
        for width in (360..=2600).step_by(80).map(|w| w as f32) {
            for height in [320.0, 600.0, 1400.0] {
                let column = Rect::new((width - 640.0).max(0.0) / 2.0, 0.0, (width + 640.0).min(width * 2.0) / 2.0, height);
                for dot in [Some(Rect::new(column.left + 180.0, 98.0, column.left + 187.0, 105.0)), Some(Rect::new(width - 20.0, 98.0, width - 13.0, 105.0)), None] {
                    let mut scene = Scene { column, due_dot: dot, ..Scene::default() };
                    let view = AgendaView { open: 1.0, rows: &rows, now: 2, selected: Some(1) };
                    draw(&eng, &fonts, &view, None, width, height, &mut scene).unwrap();
                    let card = scene.hits.iter().find(|(_, t)| *t == Target::AgendaPanel).unwrap().0;
                    assert!(card.left >= 0.0 && card.right <= width && card.top >= 0.0 && card.bottom <= height, "{width}x{height}: {card:?}");
                    if let Some(d) = dot.filter(|_| height >= 600.0) {
                        assert!(card.top >= d.bottom, "below the dot, {width}x{height}");
                    }
                    let boxes = scene.hits.iter().filter(|(_, t)| matches!(t, Target::AgendaTick(_))).count();
                    assert!(boxes > 0, "{width}x{height}");
                    for t in &scene.texts {
                        assert!(t.x >= card.left && t.x + t.layout.measure().width <= card.right + 1.0, "{width}x{height}: text past the card");
                    }
                    assert!(!scene.fills.iter().any(|f| f.ink == Ink::Page), "the page is not dimmed");
                }
            }
        }
        let mut scene = Scene::default();
        draw(&eng, &fonts, &AgendaView { open: 0.0, rows: &rows, now: 2, selected: None }, None, 800.0, 600.0, &mut scene).unwrap();
        assert!(scene.hits.is_empty() && scene.fills.is_empty(), "nothing at all while it is closed");
    }
}
