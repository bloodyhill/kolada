//! The installer's one screen, and the uninstaller's: drawn like the rest of
//! Kolada, quiet at rest. The bird, a welcome, a red Install button; Options
//! below it reveal, on intent, where the notes go (a new folder, notes
//! another app keeps, or any folder) and three choices. Installing fills the
//! button; done, it opens Kolada. The shell (Windows: `kolada-win/src/setup.rs`)
//! does the work and the window; this is only what the screen shows.

use kolada_text::{Align, Format};
use windows_core::Result;

use crate::chrome::ease;
use crate::scene::{Eng, Fonts, Scene, Target, Text, VectorMark, layout};
use crate::geom::{Fill, Ink, Rect};
use crate::vector::Logo;

/// The window's size, DIPs.
pub const SIZE: (f32, f32) = (480.0, 580.0);
/// The top strip: drags the window; its buttons at the right.
pub const CAPTION: f32 = 40.0;

/// What can be clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    /// The red button: Install (or Update), Open Kolada, Remove, Close.
    Primary,
    /// The quiet line under it when there is one: making Kolada the default
    /// for `.md` files once installed; keeping Kolada instead of removing it.
    Secondary,
    /// Options, shown or put away.
    Options,
    /// Where the notes go, by index into `View::folders`.
    Folder(usize),
    /// A choice, by index into `CHOICES`.
    Choice(usize),
}

/// The choices, in order.
pub const CHOICES: [&str; 3] = ["Desktop shortcut", "Open .md files with Kolada", "Start with Windows"];

/// Where the screen is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// Ready to install (or to update: see `View::updating`).
    Welcome,
    /// Installing, 0..1.
    Working(f32),
    Done,
    Failed,
    /// Asked to remove Kolada, removing it (0..1), and done.
    Remove,
    Removing(f32),
    Removed,
}

/// A place the notes could go: what it is, and where (or how many notes).
pub struct Folder<'a> {
    pub name: &'a str,
    pub detail: &'a str,
}

pub struct View<'a> {
    pub stage: Stage,
    /// The version installed now, when this updates it.
    pub updating: Option<&'a str>,
    pub version: &'a str,
    /// How far Options are revealed, 0..1 (the shell animates it).
    pub options: f32,
    /// Where the notes can go, the one chosen; none when updating (the notes
    /// stay where they are).
    pub folders: &'a [Folder<'a>],
    pub chosen: usize,
    pub choices: [bool; 3],
    /// What the last step said: where the notes are, or what went wrong.
    pub message: Option<&'a str>,
    /// Offer to make Kolada the default for `.md` files (installed with that
    /// choice).
    pub offer_default: bool,
    pub hover: Option<Target>,
}

const BUTTON: (f32, f32) = (220.0, 46.0);
const FOLDER_ROW: f32 = 36.0;
const CHOICE_ROW: f32 = 30.0;

/// A position at rest (options put away) and with them out.
fn lerp(rest: f32, out: f32, t: f32) -> f32 {
    rest + (out - rest) * t
}

fn centered(f: &Format) -> Format {
    Format { align: Align::Center, center_y: true, wrap: false, ellipsis: true, ..f.clone() }
}

fn left(f: &Format) -> Format {
    Format { align: Align::Start, center_y: true, wrap: false, ellipsis: true, ..f.clone() }
}

fn right(f: &Format) -> Format {
    Format { align: Align::End, center_y: true, wrap: false, ellipsis: true, ..f.clone() }
}

fn put(scene: &mut Scene, eng: &Eng, text: &str, format: &Format, r: Rect, ink: Ink, alpha: f32) {
    if alpha <= 0.01 || text.is_empty() {
        return;
    }
    let l = layout(eng, text, format, r.width(), r.height());
    scene.texts.push(Text { x: r.left, y: r.top, layout: l, ink, alpha });
}

/// The words for each stage: title, the line under it, the button.
fn words<'a>(view: &View<'a>) -> (String, String, &'static str) {
    let message = view.message.unwrap_or("");
    match view.stage {
        Stage::Welcome => match view.updating {
            Some(from) if from != view.version => ("Update Kolada".into(), format!("{from} to {}, your notes stay as they are", view.version), "Update"),
            Some(_) => ("Kolada is installed".into(), format!("Version {}, your notes stay as they are", view.version), "Install again"),
            None => ("Welcome to Kolada".into(), "Notes that open before you blink".into(), "Install"),
        },
        Stage::Working(_) if view.updating.is_some() => ("Update Kolada".into(), "Putting the new Kolada in place".into(), ""),
        Stage::Working(_) => ("Welcome to Kolada".into(), "Installing".into(), ""),
        Stage::Removing(_) => ("Remove Kolada?".into(), "Removing; your notes stay where they are".into(), ""),
        Stage::Done => ("Kolada is ready".into(), message.into(), "Open Kolada"),
        Stage::Failed => ("That did not work".into(), message.into(), "Try again"),
        Stage::Remove => ("Remove Kolada?".into(), "Your notes stay where they are".into(), "Remove"),
        Stage::Removed => ("Kolada is removed".into(), "Your notes are all still there".into(), "Close"),
    }
}

/// The screen, `width` x `height` DIPs (the window's `SIZE`).
pub fn build(eng: &Eng, fonts: &Fonts, view: &View, width: f32, height: f32) -> Result<Scene> {
    let mut scene = Scene::default();
    let s = &mut scene;
    // One layer over the window's background (the theme's page).
    s.in_chrome = true;
    let hot = |h: Hit| view.hover == Some(Target::Setup(h));

    // The window's own buttons, and the strip that drags it.
    let close = Rect::new(width - 46.0, 0.0, width, 34.0);
    let minimize = Rect::new(width - 92.0, 0.0, width - 46.0, 34.0);
    s.hits.push((Rect::new(0.0, 0.0, minimize.left, CAPTION), Target::Caption));
    for (r, t, glyph) in [(minimize, Target::Minimize, "\u{E921}"), (close, Target::Close, "\u{E8BB}")] {
        if view.hover == Some(t) {
            s.fills.push(Fill::solid(r, 0.0, Ink::Hover));
        }
        s.hits.push((r, t));
        put(s, eng, glyph, &centered(&fonts.icon_small), r, if view.hover == Some(t) { Ink::Text } else { Ink::Muted }, 1.0);
    }

    let t = if view.stage == Stage::Welcome { ease(view.options.clamp(0.0, 1.0)) } else { 0.0 };
    let mid = width / 2.0;
    let (title, line, button) = words(view);

    // The bird, red with its white eye.
    let size = lerp(76.0, 40.0, t);
    let top = lerp(96.0, 46.0, t);
    let bird = Rect::new(mid - size / 2.0, top, mid + size / 2.0, top + size);
    for (logo, ink) in [(Logo::Bird, Ink::Accent), (Logo::BirdEye, Ink::OnAccent)] {
        s.vectors.push(VectorMark { layer: s.layer_now(), rect: bird, logo, ink, alpha: 1.0 });
    }
    let title_top = lerp(194.0, 90.0, t);
    let heading = Format { size: 24.0, ..centered(&fonts.title) };
    put(s, eng, &title, &heading, Rect::new(24.0, title_top, width - 24.0, title_top + 34.0), Ink::Text, 1.0);
    let failed = view.stage == Stage::Failed;
    put(s, eng, &line, &centered(&fonts.label), Rect::new(24.0, title_top + 36.0, width - 24.0, title_top + 58.0), if failed { Ink::Danger } else { Ink::Muted }, 1.0 - t);

    // The button: red, its word in white; while working, a track it fills.
    let button_top = lerp(296.0, 132.0, t);
    let b = Rect::new(mid - BUTTON.0 / 2.0, button_top, mid + BUTTON.0 / 2.0, button_top + BUTTON.1);
    let round = BUTTON.1 / 2.0;
    let word = Format { weight: 600, ..centered(&fonts.label) };
    match view.stage {
        Stage::Working(done) | Stage::Removing(done) => {
            s.fills.push(Fill::solid(b, round, Ink::Field));
            let w = (b.width() * done.clamp(0.0, 1.0)).max(BUTTON.1);
            s.fills.push(Fill::solid(Rect::new(b.left, b.top, b.left + w, b.bottom), round, Ink::Accent));
        }
        _ => {
            s.fills.push(Fill::shadow(b.offset(0.0, 3.0), round, 10.0, if hot(Hit::Primary) { 1.2 } else { 0.8 }));
            s.fills.push(Fill::solid(b, round, Ink::Accent));
            if hot(Hit::Primary) {
                s.fills.push(Fill::faded(b, round, Ink::OnAccent, 0.12));
            }
            s.hits.push((b, Target::Setup(Hit::Primary)));
            put(s, eng, button, &word, b, Ink::OnAccent, 1.0);
        }
    }

    // Under the button: Options (before installing), or a quiet second act.
    let under = Rect::new(mid - 160.0, b.bottom + 12.0, mid + 160.0, b.bottom + 38.0);
    let secondary = match view.stage {
        Stage::Done if view.offer_default => Some("Make Kolada the default for .md files"),
        Stage::Remove => Some("Keep Kolada"),
        _ => None,
    };
    if view.stage == Stage::Welcome {
        let chevron = if t > 0.5 { "\u{E70E}" } else { "\u{E70D}" };
        let r = Rect::new(mid - 60.0, under.top, mid + 60.0, under.bottom);
        if hot(Hit::Options) {
            s.fills.push(Fill::solid(r, 8.0, Ink::Hover));
        }
        s.hits.push((r, Target::Setup(Hit::Options)));
        let ink = if hot(Hit::Options) { Ink::Text } else { Ink::Muted };
        put(s, eng, "Options", &right(&fonts.label), Rect::new(r.left, r.top, mid + 18.0, r.bottom), ink, 1.0);
        put(s, eng, chevron, &left(&fonts.icon_small), Rect::new(mid + 24.0, r.top, r.right, r.bottom), ink, 1.0);
    } else if let Some(text) = secondary {
        if hot(Hit::Secondary) {
            s.fills.push(Fill::solid(under, 8.0, Ink::Hover));
        }
        s.hits.push((under, Target::Setup(Hit::Secondary)));
        put(s, eng, text, &centered(&fonts.label), under, if hot(Hit::Secondary) { Ink::Text } else { Ink::Muted }, 1.0);
    }

    // Options, fading in as they slide up into place; clicked once there.
    if t > 0.01 {
        let settled = t > 0.99;
        let (x0, x1) = (44.0, width - 44.0);
        let mut y = lerp(under.bottom + 40.0, under.bottom + 14.0, t);
        let section = &left(&fonts.section);
        if !view.folders.is_empty() {
            put(s, eng, "Where your notes live", section, Rect::new(x0, y, x1, y + 20.0), Ink::Faint, t);
            y += 22.0;
            for (i, f) in view.folders.iter().enumerate() {
                let r = Rect::new(x0 - 10.0, y, x1 + 10.0, y + FOLDER_ROW);
                let on = i == view.chosen;
                if hot(Hit::Folder(i)) || on {
                    s.fills.push(Fill::faded(r, 10.0, if on { Ink::Selection } else { Ink::Hover }, t));
                }
                if settled {
                    s.hits.push((r, Target::Setup(Hit::Folder(i))));
                }
                // A filled dot, red where chosen.
                let dot = Rect::new(x0, y + FOLDER_ROW / 2.0 - 7.0, x0 + 14.0, y + FOLDER_ROW / 2.0 + 7.0);
                s.fills.push(Fill::faded(dot, 7.0, if on { Ink::Accent } else { Ink::Border }, t));
                if on {
                    s.fills.push(Fill::faded(dot.inset(4.5), 2.5, Ink::OnAccent, t));
                }
                put(s, eng, f.name, &left(&fonts.label), Rect::new(x0 + 26.0, y + 2.0, x1, y + 20.0), Ink::Text, t);
                put(s, eng, f.detail, &left(&fonts.small), Rect::new(x0 + 26.0, y + 20.0, x1, y + FOLDER_ROW - 2.0), Ink::Faint, t);
                y += FOLDER_ROW;
            }
            y += 10.0;
        }
        for (i, name) in CHOICES.iter().enumerate() {
            let r = Rect::new(x0 - 10.0, y, x1 + 10.0, y + CHOICE_ROW);
            if hot(Hit::Choice(i)) {
                s.fills.push(Fill::faded(r, 10.0, Ink::Hover, t));
            }
            if settled {
                s.hits.push((r, Target::Setup(Hit::Choice(i))));
            }
            put(s, eng, name, &left(&fonts.label), Rect::new(x0, y, x1 - 48.0, y + CHOICE_ROW), Ink::Text, t);
            // A switch: red track and its knob to the right when on.
            let on = view.choices[i];
            let track = Rect::new(x1 - 34.0, y + CHOICE_ROW / 2.0 - 10.0, x1, y + CHOICE_ROW / 2.0 + 10.0);
            s.fills.push(Fill::faded(track, 10.0, if on { Ink::Accent } else { Ink::Border }, t));
            let kx = if on { track.right - 18.0 } else { track.left + 2.0 };
            s.fills.push(Fill::faded(Rect::new(kx, track.top + 2.0, kx + 16.0, track.bottom - 2.0), 8.0, if on { Ink::OnAccent } else { Ink::Faint }, t));
            y += CHOICE_ROW;
        }
    }

    // The version, faint at the bottom.
    let foot = Rect::new(0.0, height - 30.0, width, height - 8.0);
    put(s, eng, &format!("Kolada {}", view.version), &centered(&fonts.small), foot, Ink::Faint, 1.0 - t);
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders() -> Vec<Folder<'static>> {
        vec![
            Folder { name: "A new folder", detail: "Documents\\Kolada" },
            Folder { name: "Obsidian vault", detail: "Documents\\Work, 214 notes, opens as it is" },
            Folder { name: "Notes export", detail: "Documents\\old notes.nbk, imported into Documents\\Kolada" },
            Folder { name: "Markdown notes", detail: "Documents\\A folder with a rather long name that goes on and on, 9,999 notes" },
            Folder { name: "Another folder\u{2026}", detail: "Choose any folder" },
        ]
    }

    fn view<'a>(stage: Stage, options: f32, folders: &'a [Folder<'a>], updating: Option<&'a str>) -> View<'a> {
        View { stage, updating, version: "0.1.0", options, folders, chosen: 1, choices: [true, true, false], message: Some("Your notes: Documents\\Kolada"), offer_default: true, hover: None }
    }

    fn controls(scene: &Scene) -> Vec<(Rect, Target)> {
        scene.hits.iter().copied().filter(|(_, t)| *t != Target::Caption).collect()
    }

    #[test]
    fn every_stage_fits_the_window_and_nothing_overlaps() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let all = folders();
        let (w, h) = SIZE;
        let stages = [Stage::Welcome, Stage::Working(0.4), Stage::Done, Stage::Failed, Stage::Remove, Stage::Removing(0.7), Stage::Removed];
        let mut checked = 0;
        for stage in stages {
            for options in [0.0, 0.5, 1.0] {
                for (updating, folders) in [(None, &all[..]), (Some("0.0.9"), &all[..0]), (None, &all[..2])] {
                    let v = view(stage, options, folders, updating);
                    let scene = build(&eng, &fonts, &v, w, h).unwrap();
                    let at = format!("{stage:?}, options {options}, updating {updating:?}, {} folders", folders.len());
                    let hits = controls(&scene);
                    for (r, t) in &hits {
                        assert!(r.left >= 0.0 && r.top >= 0.0 && r.right <= w && r.bottom <= h - 30.0 || matches!(t, Target::Close | Target::Minimize), "{t:?} {r:?} leaves the window, {at}");
                    }
                    for (i, (a, ta)) in hits.iter().enumerate() {
                        for (b, tb) in &hits[i + 1..] {
                            let o = (a.right.min(b.right) - a.left.max(b.left)).max(0.0) * (a.bottom.min(b.bottom) - a.top.max(b.top)).max(0.0);
                            assert!(o <= 1.0, "{ta:?} overlaps {tb:?}, {at}");
                        }
                    }
                    for text in &scene.texts {
                        let m = text.layout.measure();
                        assert!(text.x >= 0.0 && text.x + m.width <= w + 0.5 && text.y >= 0.0 && text.y + m.height <= h + 0.5, "a text leaves the window, {at}");
                    }
                    let has = |h: Hit| hits.iter().any(|(_, t)| *t == Target::Setup(h));
                    assert_eq!(has(Hit::Primary), !matches!(stage, Stage::Working(_) | Stage::Removing(_)), "the button takes clicks except while working, {at}");
                    let out = stage == Stage::Welcome && options >= 1.0;
                    assert_eq!(has(Hit::Choice(0)), out, "the choices only once Options are out, {at}");
                    assert_eq!(has(Hit::Folder(0)), out && !folders.is_empty(), "the folders only when there is a choice, {at}");
                    assert_eq!(has(Hit::Options), stage == Stage::Welcome, "{at}");
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 7 * 3 * 3);
    }

    #[test]
    fn at_rest_it_is_the_bird_a_welcome_and_install() {
        let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
        let fonts = Fonts::new(1.0, 0);
        let all = folders();
        let scene = build(&eng, &fonts, &view(Stage::Welcome, 0.0, &all, None), SIZE.0, SIZE.1).unwrap();
        assert_eq!(scene.vectors.iter().map(|v| v.logo).collect::<Vec<_>>(), [Logo::Bird, Logo::BirdEye]);
        let controls: Vec<Target> = controls(&scene).into_iter().map(|(_, t)| t).collect();
        assert_eq!(controls, [Target::Minimize, Target::Close, Target::Setup(Hit::Primary), Target::Setup(Hit::Options)], "nothing else until Options");
        // Red is the button alone, and the bird.
        let red: Vec<&Fill> = scene.fills.iter().filter(|f| f.ink == Ink::Accent).collect();
        assert_eq!(red.len(), 1);
        // Out, the chosen folder's dot and the choices that are on are red.
        let out = build(&eng, &fonts, &view(Stage::Welcome, 1.0, &all, None), SIZE.0, SIZE.1).unwrap();
        assert_eq!(out.fills.iter().filter(|f| f.ink == Ink::Accent).count(), 1 + 1 + 2);
    }
}
