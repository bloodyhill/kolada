//! Debug builds only: a short script of steps the app runs on itself, for
//! checking a flow over SSH where the Mac takes no input (the writing
//! model's first run, say). `KOLADA_DRIVE="wait-card; shot:/tmp/card.bmp;
//! click-download; wait-model:600; wait-helper:60; type:A line.; wait-fixed:60;
//! undo; quit"`. Each step says what it saw on stderr ("drive: ..."); the
//! note's text is never printed, only whether it changed and its masked
//! shape. Steps:
//!
//! - `wait-card[:secs]`: the first run's model card is up;
//!   `wait-no-card[:secs]`: it went.
//! - `click-download`, `click-notnow`, `click-cancel`: a click on that
//!   control of the card, where the last frame drew it.
//! - `shot:path`: the window as the window server shows it, as a BMP.
//! - `sleep:secs`.
//! - `wait-model[:secs]`: a catalogue model is in the models folder, whole.
//! - `wait-helper[:secs]`: the writing helper runs, and on which model.
//! - `new-note`: Cmd+N.
//! - `type:text`: typed as AppKit hands typing over; `enter`: a new line.
//! - `wait-fixed[:secs]`: the note's text changed since the last typing,
//!   with no key pressed (writing help put a fix in).
//! - `undo`: Cmd+Z; says whether the text is again as it was typed.
//! - `paste`: Cmd+V on the note; says what it added, counted.
//! - `quit`.
//!
//! VoiceOver's view of the window (run it on a generated vault: the side
//! menu's rows are named by their notes):
//! - `access`: the window's elements as AppKit hands them to an assistive
//!   app, each role and name (a text area's name masked, its text only
//!   counted), which one has the keys, and what is under the middle of the
//!   first control.
//! - `access-quick`: the same for Beam's panel (`quick-open` first).
//! - `access-press:<name>`: the control of that name pressed, as VoiceOver
//!   presses it (in the window, else in Beam's panel).
//!
//! Quick Send's panel (`quick.rs`), never reading the Mac's clipboard:
//! - `quick-open`, `quick-devices`: the panel, on its clips or devices.
//! - `quick-press:<hit>`: a click on `pair-now`, `show-code`, `allow`, `not-now`,
//!   `beam`, `close` where the panel drew it.
//! - `wait-quick-code[:secs]`: a pairing code shows, and is said (it is a
//!   test instance's, good for 5 minutes).
//! - `wait-quick-asks[:secs]`, `wait-quick-paired[:secs]`.
//! - `wait-quick-clips:<n>[:secs]`: at least n clips.
//! - `quick-stage:text`: the keys, with `text` as what was copied.
//! - `quick-key:enter|escape`.
//! - `quick-shot:path`: the panel's last frame, as a BMP.
//! - `quick-visible`: says whether the panel shows.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use kolada_ui::modelcard::Hit;
use kolada_ui::scene::Target;
use objc2::{DefinedClass, MainThreadMarker, msg_send, sel};
use objc2_foundation::{NSNotFound, NSObjectNSDelayedPerforming, NSRange, NSString};

use super::KoladaView;

struct Run {
    steps: VecDeque<String>,
    /// When the step at the front began.
    began: Option<Instant>,
    /// The note's text after the last typing.
    typed: Option<String>,
}

thread_local! {
    static RUN: RefCell<Option<Run>> = const { RefCell::new(None) };
}

fn say(what: &str) {
    eprintln!("drive: {what}");
}

/// The text's shape, letters as x and digits as 9.
fn masked(s: &str) -> String {
    s.chars().map(|c| if c.is_alphabetic() { 'x' } else if c.is_numeric() { '9' } else { c }).collect()
}

type Element = objc2::rc::Retained<objc2::runtime::AnyObject>;

fn window_of(view: &KoladaView) -> Option<Element> {
    view.window().map(|w| objc2::rc::Retained::into_super(objc2::rc::Retained::into_super(objc2::rc::Retained::into_super(w))).into())
}

fn children(e: &objc2::runtime::AnyObject) -> Vec<Element> {
    let kids: Option<objc2::rc::Retained<objc2_foundation::NSArray<objc2::runtime::AnyObject>>> = unsafe { msg_send![e, accessibilityChildren] };
    kids.map(|k| k.to_vec()).unwrap_or_default()
}

fn text(e: &objc2::runtime::AnyObject, sel: objc2::runtime::Sel) -> String {
    let responds: bool = unsafe { msg_send![e, respondsToSelector: sel] };
    if !responds {
        return String::new();
    }
    let s: Option<objc2::rc::Retained<objc2::runtime::AnyObject>> = unsafe { msg_send![e, performSelector: sel] };
    s.and_then(|s| s.downcast::<NSString>().ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// The first element under `from` named `name`, depth first.
fn find(from: &Option<Element>, name: &str) -> Option<Element> {
    let mut stack: Vec<Element> = from.iter().cloned().collect();
    while let Some(e) = stack.pop() {
        if text(&e, sel!(accessibilityLabel)) == name {
            return Some(e);
        }
        stack.extend(children(&e).into_iter().rev());
    }
    None
}

/// Says the window's elements, two levels down, and which has the keys.
fn access_tree(view: &KoladaView) {
    let Some(window) = window_of(view) else { return say("access: no window") };
    walk_from(view, &window);
}

/// Says the elements under `window`, which has the keys, and what is under
/// the middle of the first control.
fn walk_from(view: &KoladaView, window: &Element) {
    let mut first_control: Option<Element> = None;
    let mut count = 0;
    fn walk(e: &Element, depth: usize, count: &mut usize, first: &mut Option<Element>) {
        for c in children(e) {
            let role = text(&c, sel!(accessibilityRole));
            let label = text(&c, sel!(accessibilityLabel));
            *count += 1;
            if role == "AXTextArea" {
                let value = text(&c, sel!(accessibilityValue));
                say(&format!("access: {}{role} {:?}, {} characters", "  ".repeat(depth), masked(&label), value.chars().count()));
            } else {
                // A box, a folder, a choice: its value too.
                let value: Option<objc2::rc::Retained<objc2::runtime::AnyObject>> = if matches!(role.as_str(), "AXCheckBox" | "AXDisclosureTriangle" | "AXRadioButton") { unsafe { msg_send![&*c, accessibilityValue] } } else { None };
                let value = value.and_then(|v| v.downcast::<objc2_foundation::NSNumber>().ok()).map_or(String::new(), |n| format!(" = {}", n.as_i64()));
                say(&format!("access: {}{role} {label:?}{value}", "  ".repeat(depth)));
            }
            if first.is_none() && role == "AXButton" && !label.is_empty() && depth == 0 {
                *first = Some(c.clone());
            }
            if depth < 1 {
                walk(&c, depth + 1, count, first);
            }
        }
    }
    walk(window, 0, &mut count, &mut first_control);
    say(&format!("access: {count} elements"));
    let app = objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(view));
    let focused: Option<Element> = unsafe { msg_send![&*app, accessibilityFocusedUIElement] };
    say(&format!("access: the keys are with {}", focused.map_or("nothing".into(), |f| text(&f, sel!(accessibilityRole)))));
    if let Some(c) = first_control {
        let frame: objc2_foundation::NSRect = unsafe { msg_send![&*c, accessibilityFrame] };
        let middle = objc2_foundation::NSPoint::new(frame.origin.x + frame.size.width / 2.0, frame.origin.y + frame.size.height / 2.0);
        let hit: Option<Element> = unsafe { msg_send![&*app, accessibilityHitTest: middle] };
        let same = hit.as_ref().is_some_and(|h| std::ptr::eq(&**h, &*c));
        say(&format!("access: the middle of {:?} hits {}", text(&c, sel!(accessibilityLabel)), if same { "it" } else { "something else" }));
    }
}

/// After the first frame: starts the script when there is one.
pub fn start(view: &KoladaView) {
    let Ok(script) = std::env::var("KOLADA_DRIVE") else { return };
    let steps: VecDeque<String> = script.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    say(&format!("{} steps", steps.len()));
    RUN.with(|r| *r.borrow_mut() = Some(Run { steps, began: None, typed: None }));
    again(view, 0.3);
}

fn again(view: &KoladaView, after: f64) {
    unsafe { view.performSelector_withObject_afterDelay(sel!(driveScript), None, after) };
}

/// What a step waiting on something saw: done, or not yet.
enum Wait {
    Done,
    NotYet,
}

/// Runs the step at the front; a step that waits looks again shortly, and
/// one that waits too long ends the script.
pub fn step(view: &KoladaView) {
    let Some(step) = RUN.with(|r| r.borrow().as_ref().and_then(|r| r.steps.front().cloned())) else {
        say("done");
        return;
    };
    let began = RUN.with(|r| *r.borrow_mut().as_mut().unwrap().began.get_or_insert_with(Instant::now));
    let (name, arg) = step.split_once(':').map_or((step.as_str(), ""), |(n, a)| (n, a));
    let limit = Duration::from_secs_f64(arg.parse().unwrap_or(30.0));
    let waited = began.elapsed();
    let outcome = match name {
        "wait-card" => match view.ivars().state.borrow().model.model_card_shown() {
            true => {
                say(&format!("the model card is up ({:.1} s)", waited.as_secs_f64()));
                Wait::Done
            }
            false => Wait::NotYet,
        },
        "wait-no-card" => match view.ivars().state.borrow().model.model_card_shown() {
            false => {
                say(&format!("the model card is gone ({:.1} s)", waited.as_secs_f64()));
                Wait::Done
            }
            true => Wait::NotYet,
        },
        "click-download" | "click-notnow" | "click-cancel" => {
            let hit = match name {
                "click-download" => Hit::Download,
                "click-notnow" => Hit::NotNow,
                _ => Hit::Cancel,
            };
            click(view, Target::Model(hit), name);
            Wait::Done
        }
        "shot" => {
            match super::drive::capture_bmp(view, std::path::Path::new(arg)) {
                Ok((w, h)) => say(&format!("the window, {w} x {h} pixels, in {arg}")),
                Err(e) => say(&format!("no picture of the window: {e}")),
            }
            Wait::Done
        }
        "sleep" => match waited >= Duration::from_secs_f64(arg.parse().unwrap_or(1.0)) {
            true => Wait::Done,
            false => Wait::NotYet,
        },
        "wait-model" => match model_in_place() {
            Some(file) => {
                say(&format!("{file} is in the models folder, whole ({:.1} s)", waited.as_secs_f64()));
                Wait::Done
            }
            None => Wait::NotYet,
        },
        "wait-helper" => match helper_running() {
            Some(on) => {
                say(&format!("the writing helper runs, on {on} ({:.1} s)", waited.as_secs_f64()));
                Wait::Done
            }
            None => Wait::NotYet,
        },
        "type" => {
            for c in arg.chars() {
                let text = NSString::from_str(&c.to_string());
                let _: () = unsafe { msg_send![view, insertText: &*text, replacementRange: NSRange::new(NSNotFound as usize, 0)] };
            }
            let text = view.ivars().state.borrow().model.access_text().to_string();
            say(&format!("typed {} characters", arg.chars().count()));
            RUN.with(|r| r.borrow_mut().as_mut().unwrap().typed = Some(text));
            Wait::Done
        }
        "new-note" => {
            let _: () = unsafe { msg_send![view, newNote: None::<&objc2::runtime::AnyObject>] };
            say("a new note");
            Wait::Done
        }
        "enter" => {
            let _: () = unsafe { msg_send![view, doCommandBySelector: sel!(insertNewline:)] };
            let text = view.ivars().state.borrow().model.access_text().to_string();
            RUN.with(|r| r.borrow_mut().as_mut().unwrap().typed = Some(text));
            Wait::Done
        }
        "wait-fixed" => {
            let typed = RUN.with(|r| r.borrow().as_ref().unwrap().typed.clone()).unwrap_or_default();
            let now = view.ivars().state.borrow().model.access_text().to_string();
            if now != typed {
                let (a, b): (Vec<char>, Vec<char>) = (typed.chars().collect(), now.chars().collect());
                let lead = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
                let tail = a[lead..].iter().rev().zip(b[lead..].iter().rev()).take_while(|(x, y)| x == y).count();
                let was: String = a[lead..a.len() - tail].iter().collect();
                let is: String = b[lead..b.len() - tail].iter().collect();
                say(&format!("writing help changed the note ({:.1} s after typing): \"{}\" became \"{}\" (masked)", waited.as_secs_f64(), masked(&was), masked(&is)));
                Wait::Done
            } else {
                Wait::NotYet
            }
        }
        "undo" => {
            let _: () = unsafe { msg_send![view, undo: None::<&objc2::runtime::AnyObject>] };
            let typed = RUN.with(|r| r.borrow().as_ref().unwrap().typed.clone()).unwrap_or_default();
            let same = view.ivars().state.borrow().model.access_text() == typed;
            say(&format!("undo: the note is {}as it was typed", if same { "" } else { "not " }));
            Wait::Done
        }
        "quick-open" | "quick-devices" => {
            super::quick::open(MainThreadMarker::from(view), name == "quick-devices");
            say(&format!("{name}: the panel {}", if super::quick::visible() { "shows" } else { "does not show" }));
            Wait::Done
        }
        "quick-press" => {
            use kolada_ui::quick::Hit as Q;
            let hit = match arg {
                "show-code" => Some(Q::ShowCode),
                "pair-now" => Some(Q::PairNow),
                "allow" => Some(Q::Allow),
                "not-now" => Some(Q::NotNow),
                "beam" => Some(Q::Beam),
                "close" => Some(Q::Close),
                _ => None,
            };
            let pressed = hit.is_some_and(super::quick::press);
            say(&format!("quick-press {arg}: {}", if pressed { "pressed" } else { "not drawn" }));
            Wait::Done
        }
        "wait-quick-code" | "wait-quick-asks" | "wait-quick-paired" | "wait-quick-clips" => {
            let (want, limit_arg) = match name {
                "wait-quick-clips" => arg.split_once(':').map_or((arg.parse().unwrap_or(1), None), |(n, s)| (n.parse().unwrap_or(1), s.parse::<f64>().ok())),
                _ => (0, None),
            };
            if let Some(secs) = limit_arg
                && waited > Duration::from_secs_f64(secs)
            {
                say(&format!("{name}: nothing after {secs:.0} s; the script ends"));
                RUN.with(|r| *r.borrow_mut() = None);
                objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(view)).terminate(None);
                return;
            }
            let state = super::quick::read(|m| m.quick_state());
            let seen = match (name, state) {
                ("wait-quick-code", Some((Some(code), ..))) => Some(format!("code {code}")),
                ("wait-quick-asks", Some((_, true, ..))) => Some("a device asks to pair".into()),
                ("wait-quick-paired", Some((_, _, true, _))) => Some("paired".into()),
                ("wait-quick-clips", Some((.., n))) if n >= want => Some(format!("{n} clips")),
                _ => None,
            };
            match seen {
                Some(s) => {
                    say(&format!("{s} ({:.1} s)", waited.as_secs_f64()));
                    Wait::Done
                }
                None => Wait::NotYet,
            }
        }
        "quick-stage" => {
            let text = arg.to_string();
            super::quick::keys_with(MainThreadMarker::from(view), false, &|| Some(kolada_app::model::quick::Clip { text: text.clone(), ..Default::default() }));
            say(&format!("quick-stage: {} characters; the panel {}", arg.chars().count(), if super::quick::visible() { "shows" } else { "does not show" }));
            Wait::Done
        }
        "quick-key" => {
            use kolada_app::model::quick::QuickKey;
            let key = if arg == "escape" { QuickKey::Escape } else { QuickKey::Enter };
            super::quick::run(|m| m.quick_key(key, &|| None));
            say(&format!("quick-key {arg}"));
            Wait::Done
        }
        "quick-shot" => {
            let done = super::quick::read(|m| m.quick_surface().map(|s| s.dump(std::path::Path::new(arg)).is_ok())).flatten();
            say(&format!("quick-shot: {}", if done == Some(true) { arg } else { "no frame" }));
            Wait::Done
        }
        "quick-visible" => {
            say(&format!("the panel {}", if super::quick::visible() { "shows" } else { "does not show" }));
            Wait::Done
        }
        // Cmd+V on the note, as the Edit menu sends it.
        "paste" => {
            let before = view.ivars().state.borrow().model.access_text().len();
            view.paste_clipboard(false);
            let st = view.ivars().state.borrow();
            let text = st.model.access_text();
            say(&format!("paste: {} characters added, {} embeds", text.len() as isize - before as isize, text.matches("![[").count()));
            Wait::Done
        }
        "access" => {
            access_tree(view);
            Wait::Done
        }
        "access-quick" => {
            match super::quick::panel_object() {
                Some(panel) => walk_from(view, &panel),
                None => say("access-quick: no panel"),
            }
            Wait::Done
        }
        "access-press" => {
            let found = find(&window_of(view), arg).or_else(|| find(&super::quick::panel_object(), arg));
            let pressed = found.is_some_and(|e| unsafe { msg_send![&*e, accessibilityPerformPress] });
            say(&format!("access-press {arg}: {}", if pressed { "pressed" } else { "not found" }));
            Wait::Done
        }
        "quit" => {
            say("quit");
            objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(view)).terminate(None);
            Wait::Done
        }
        other => {
            say(&format!("no such step: {other}"));
            Wait::Done
        }
    };
    match outcome {
        Wait::Done => {
            RUN.with(|r| {
                let mut r = r.borrow_mut();
                let run = r.as_mut().unwrap();
                run.steps.pop_front();
                run.began = None;
            });
            again(view, 0.05);
        }
        Wait::NotYet if waited > limit => {
            say(&format!("{name}: nothing after {:.0} s; the script ends", limit.as_secs_f64()));
            RUN.with(|r| *r.borrow_mut() = None);
            objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(view)).terminate(None);
        }
        Wait::NotYet => again(view, 0.2),
    }
}

/// A click where the last frame drew `target`, as the pointer's would be.
fn click(view: &KoladaView, target: Target, name: &str) {
    let place = view.ivars().state.borrow().model.place_of(target);
    let Some(r) = place else {
        say(&format!("{name}: not drawn"));
        return;
    };
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let changed = view.ivars().state.borrow_mut().model.pointer_down(x, y, 1, false);
    view.edited(changed);
    let (changed, effects) = view.ivars().state.borrow_mut().model.pointer_up_at(x, y);
    view.edited(changed);
    view.redraw_if(changed);
    view.run_effects(effects);
    view.serve();
    say(&format!("{name} at {x:.0}, {y:.0}"));
}

/// The catalogue model in the (test instance's) models folder, whole.
fn model_in_place() -> Option<&'static str> {
    let dir = kolada_app::helper::models_dir()?;
    kolada_assist::models::MODELS.iter().find(|m| std::fs::metadata(dir.join(m.file)).is_ok_and(|f| f.len() == m.bytes)).map(|m| m.file)
}

/// The writing helper started by this app, and the model it was given.
fn helper_running() -> Option<String> {
    let out = std::process::Command::new("pgrep").args(["-P", &std::process::id().to_string(), "kolada-ai"]).output().ok()?;
    let pid = String::from_utf8_lossy(&out.stdout).lines().next()?.trim().to_string();
    let args = std::process::Command::new("ps").args(["-o", "args=", "-p", &pid]).output().ok()?;
    let args = String::from_utf8_lossy(&args.stdout).to_string();
    // The model's file name, not where it is (the path has spaces: it is
    // the last argument).
    let model = args.split_once("--model ").and_then(|(_, p)| std::path::Path::new(p.trim()).file_name()).map(|n| n.to_string_lossy().into_owned());
    Some(model.unwrap_or_else(|| "no model named".into()))
}
