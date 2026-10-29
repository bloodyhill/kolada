//! The windows and their event loop: winit gives the windows (X11 or
//! Wayland) and their events, each window's model draws its frames, and
//! softbuffer puts the parts that changed on the screen. Nothing runs
//! between events: the loop sleeps until the next event or the next thing
//! due by the clock (a step of motion, a note's save), so an idle window
//! costs nothing.
//!
//! The main window shows the vault; a note opened in a window of its own
//! ("Open in New Window") gets one beside it, with its own model, as on
//! Windows and the Mac. A window has no title bar of its own: the chrome
//! draws the window buttons (as on Windows), the empty strip along the top
//! moves the window, and its edges resize it.
//!
//! Beam's popup is one more window, the main model's: made hidden once
//! Beam starts after the first frame, shown on Alt+Z (where the desktop
//! lets Kolada have keys), the window's Beam button, or `--beam`.

use std::cell::RefCell;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kolada_app::Model;
use kolada_app::model::WindowButton;
use kolada_app::model::menu::MenuAction;
use kolada_app::model::settings::Request;
use kolada_app::model::share::Effect;
use kolada_app::model::quick::{QuickDo, QuickKey};
use kolada_app::model::transfer::Pick;
use kolada_editor::BlockKind;
use kolada_ui::scene::Panel;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{CursorIcon, ResizeDirection, Theme, Window, WindowId};

use super::keys::{self, Action, Do, Mods, Named};
use super::{Args, arrive, beam, blit, clipboard, instance, outside, trace};
use kolada_ui::quick as q;

/// How long after the last edit the note is written.
const SAVE_DELAY: Duration = Duration::from_millis(600);
/// Frames while something moves, as often as a display refreshes.
const FRAME: Duration = Duration::from_millis(16);
/// How close to an edge, DIPs, the pointer resizes the window.
const EDGE: f64 = 5.0;
/// A wheel's notch: three lines of the page (24 DIPs each), as on Windows.
const LINES_A_NOTCH: f32 = 3.0;
const LINE: f32 = 24.0;
/// Two presses this close in time and place are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Spelling is checked this long after the last change, a few lines at a
/// time, and the next few this soon after.
const SPELL_DELAY: Duration = Duration::from_millis(400);
const SPELL_MORE: Duration = Duration::from_millis(20);
/// A note's own window, as it first opens (DIPs).
const NOTE_WINDOW: (f64, f64) = (560.0, 680.0);

/// What threads have brought, to be taken in on the UI's: for one window
/// (by its id), or for the app.
#[derive(Debug, Clone)]
pub enum Wake {
    Pictures(WindowId),
    Arrivals(WindowId),
    Index(WindowId),
    Outside(WindowId),
    /// A note was exported to this path (a PDF printed), or why not (empty:
    /// it could not be written).
    Printed(WindowId, PathBuf, Result<(), String>),
    /// A chooser closed: what was chosen (None: there is no chooser).
    Picked(WindowId, Pick, Option<Vec<PathBuf>>),
    /// A word to pass on.
    Whisper(WindowId, &'static str),
    /// X lost what part of the window showed (`expose.rs`).
    Exposed(WindowId),
    /// The writing model answered, its download moved on, or the gists were read.
    Assist(WindowId),
    /// The desktop said whether it is dark.
    Appearance(bool),
    /// The dictionaries are read: spelling can be checked.
    Spelling,
    /// Kolada was launched again, with this to open.
    Handed(instance::Handed),
    /// An update is downloaded and checked (`updates::take_ready`).
    UpdateReady,
    /// Beam's engine has news (`Model::quick_events`).
    Beam,
    /// Beam's keys were pressed.
    BeamKeys,
    /// The chooser for a picture to beam closed: what was chosen.
    BeamPicked(Option<Vec<PathBuf>>),
}

/// What the left button went down on.
enum Press {
    Nothing,
    /// The page or the chrome: drags go to the model.
    Model,
    Button(WindowButton),
}

struct Shown {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
}

/// What one window asks of the app, once its event is handled.
#[derive(Default)]
struct Asks {
    /// Notes to open in windows of their own.
    windows: Vec<PathBuf>,
    /// This (a note's) window goes, its note opening in the main one.
    to_main: Option<String>,
    /// This window goes; the app, if it is the main one.
    close: bool,
    /// The app goes.
    quit: bool,
    /// The app goes and starts again as this program (an update put in).
    restart: Option<PathBuf>,
    /// Beam's popup, on its clips or (true) on the devices.
    beam: Option<bool>,
    /// Beam this note (the Beam button's menu).
    beam_note: bool,
    /// Beam's keys were turned on or off in Settings.
    beam_keys: bool,
}

/// Beam's popup: a small window of its own, the main model's.
struct Popup {
    shown: Shown,
    size: (u32, u32),
    pointer: Option<PhysicalPosition<f64>>,
    mods: ModifiersState,
    /// The chooser for a picture is open: losing the keys to it keeps the
    /// popup.
    picking: bool,
    /// It has had the keys since it was shown: only then does losing them
    /// put it away (a desktop that gives a new window no keys leaves it).
    focused: bool,
}

/// A window and what is in it.
struct Win {
    model: Model,
    /// The vault's own window (else a note's own).
    main: bool,
    proxy: EventLoopProxy<Wake>,
    shown: Option<Shown>,
    history: blit::History,
    size: (u32, u32),
    first: bool,
    mods: ModifiersState,
    /// The pointer, physical pixels in the window.
    pointer: Option<PhysicalPosition<f64>>,
    press: Press,
    /// The last press: when, where, and how many in a row.
    clicks: (Option<Instant>, PhysicalPosition<f64>, u32),
    /// An input method is composing.
    composing: bool,
    ime_area: Option<(i32, i32, u32, u32)>,
    tick_at: Option<Instant>,
    save_at: Option<Instant>,
    key_at: Option<Instant>,
    /// The desktop's clipboard, shared by the windows (so a copy in one
    /// pastes exactly in another).
    clipboard: Rc<RefCell<clipboard::Clipboard>>,
    cursor: CursorIcon,
    /// The window's size before it went minimal.
    full_size: Option<PhysicalSize<u32>>,
    dark: bool,
    /// The dictionaries are in; when the next lines are checked.
    spelling: bool,
    spell_at: Option<Instant>,
    /// Files dragged over the window.
    drag: arrive::Drag,
    asks: Asks,
}

pub struct App {
    wins: Vec<Win>,
    proxy: EventLoopProxy<Wake>,
    clipboard: Rc<RefCell<clipboard::Clipboard>>,
    dark: bool,
    spelling: bool,
    /// Where later launches hand over what they were asked to open.
    listener: Option<std::os::unix::net::UnixListener>,
    /// Once the loop ends, the app becomes this program (an update).
    restart: Option<PathBuf>,
    /// Beam's popup, made once Beam starts; its keys while held; when it
    /// is next due by the clock; and whether this launch asked for it.
    popup: Option<Popup>,
    keys: Option<beam::Keys>,
    beam_at: Option<Instant>,
    beam_asked: bool,
}

/// Runs the app until it quits.
pub fn run(args: Args) {
    let (vault, note) = match args.vault.clone().and_then(kolada_app::launch::given) {
        Some((vault, note)) => (Some(vault), note),
        None => (None, None),
    };
    // Kolada is running already: it opens what this launch was asked to,
    // and this one goes.
    let listener = match instance::claim(&instance::name(kolada_core::app_data_dir().as_deref()), &instance::Handed::asked_beam(args.beam, args.new, args.search, args.vault.as_deref())) {
        instance::Claim::Handed => {
            trace::mark("handed to the one running");
            return;
        }
        instance::Claim::First(l) => Some(l),
        instance::Claim::Alone => None,
    };
    // An update downloaded last time and never clicked goes in now, and
    // the new program starts in this one's place (one look at a file when
    // nothing waits).
    if let Some(exe) = super::updates::install_waiting() {
        drop(listener);
        super::updates::restart(&exe);
        return;
    }
    let mut builder = EventLoop::<Wake>::with_user_event();
    if args.x11 {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11();
    }
    let event_loop = match builder.build() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("kolada: no display to open a window on ({e})");
            std::process::exit(1);
        }
    };
    trace::mark("display connected");
    // Dark or light as the desktop was last seen (asked again after the first frame).
    let dark = super::appearance::kept(kolada_core::app_data_dir().as_deref()).unwrap_or(true);
    let mut model = super::linux_model(Model::launch(super::services::services(), kolada_app::launch::launch_vault(vault), dark));
    if let Some(rel) = &note {
        model.open(rel);
    }
    // The app list's actions on Kolada's icon.
    if args.new {
        model.new_note();
    } else if args.search {
        model.open_search();
    }
    trace::mark("model ready");
    let proxy = event_loop.create_proxy();
    let clipboard = Rc::new(RefCell::new(clipboard::Clipboard::default()));
    let main = Win::new(model, true, proxy.clone(), clipboard.clone(), dark);
    let mut app = App { wins: vec![main], proxy, clipboard, dark, spelling: false, listener, restart: None, popup: None, keys: None, beam_at: None, beam_asked: args.beam };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("kolada: {e}");
    }
    for w in &mut app.wins {
        w.model.save_flush();
    }
    // Updated: the windows go, then this process becomes the new program.
    if let Some(exe) = app.restart.take() {
        drop(app);
        super::updates::restart(&exe);
    }
}

impl Win {
    fn new(model: Model, main: bool, proxy: EventLoopProxy<Wake>, clipboard: Rc<RefCell<clipboard::Clipboard>>, dark: bool) -> Win {
        Win {
            model,
            main,
            proxy,
            shown: None,
            history: blit::History::default(),
            size: (0, 0),
            first: true,
            mods: ModifiersState::empty(),
            pointer: None,
            press: Press::Nothing,
            clicks: (None, PhysicalPosition::new(0.0, 0.0), 0),
            composing: false,
            ime_area: None,
            tick_at: None,
            save_at: None,
            key_at: None,
            clipboard,
            cursor: CursorIcon::Default,
            full_size: None,
            dark,
            spelling: false,
            spell_at: None,
            drag: arrive::Drag::default(),
            asks: Asks::default(),
        }
    }

    fn window(&self) -> Option<&Rc<Window>> {
        self.shown.as_ref().map(|s| &s.window)
    }

    fn id(&self) -> Option<WindowId> {
        self.window().map(|w| w.id())
    }

    fn scale(&self) -> f64 {
        self.window().map_or(1.0, |w| w.scale_factor())
    }

    /// The pointer in DIPs.
    fn dip(&self, p: PhysicalPosition<f64>) -> (f32, f32) {
        let s = self.scale();
        ((p.x / s) as f32, (p.y / s) as f32)
    }

    fn redraw(&self) {
        if let Some(w) = self.window() {
            w.request_redraw();
        }
    }

    /// The screen changed: draw it, and step what moves.
    fn changed(&mut self, changed: bool) {
        if changed {
            self.redraw();
            self.schedule_tick();
            // Another note, or lines scrolled in: their spelling, soon.
            if self.spelling && self.spell_at.is_none() {
                self.spell_at = Some(Instant::now() + SPELL_DELAY);
            }
        }
    }

    /// The note changed: draw it, write it soon, and check its spelling
    /// once typing pauses.
    fn edited(&mut self, changed: bool) {
        if changed {
            self.changed(true);
            let now = Instant::now();
            self.save_at = Some(now + SAVE_DELAY);
            if self.spelling {
                self.spell_at = Some(now + SPELL_DELAY);
            }
        }
    }

    /// Frames follow while anything moves; with nothing moving, one step
    /// when something is next due by the clock.
    fn schedule_tick(&mut self) {
        let now = Instant::now();
        // While files are dragged over, the pointer is followed a frame at a time.
        self.tick_at = if self.model.wants_frames() || self.drag.active() { Some(now + FRAME) } else { self.model.next_due().map(|t| t.max(now)) };
    }

    /// When this window next has something due by the clock.
    fn next(&self) -> Option<Instant> {
        [self.tick_at, self.save_at, self.spell_at].into_iter().flatten().min()
    }

    /// Where a drag is over the window, DIPs: the pointer as X says it is,
    /// else the window's middle.
    fn drag_point(&self) -> (f32, f32) {
        let Some(w) = self.window() else { return (0.0, 0.0) };
        match arrive::pointer(w) {
            Some((x, y)) => self.dip(PhysicalPosition::new(x, y)),
            None => {
                let size = w.inner_size();
                self.dip(PhysicalPosition::new(size.width as f64 / 2.0, size.height as f64 / 3.0))
            }
        }
    }

    /// Files dragged over the window: the card says what they become.
    fn drag_hover(&mut self, path: PathBuf) {
        self.drag.hover(path);
        let (x, y) = self.drag_point();
        let files = self.drag.hovering.clone();
        self.model.drag_enter(&files, x, y);
        self.changed(true);
    }

    /// The drag was let go: its files come in, pictures where the gap
    /// opened, else where it was let go (notes and documents become notes).
    fn take_drop(&mut self) {
        let files = self.drag.end();
        if files.is_empty() {
            return;
        }
        let at = self.window().and_then(|w| arrive::pointer(w)).map(|(x, y)| self.dip(PhysicalPosition::new(x, y)));
        let changed = match self.model.drag_drop() {
            Some(end) => self.model.drop_files_after(files, end),
            None => self.model.drop_files(files, at),
        };
        self.edited(changed);
        self.changed(true);
    }

    /// Makes the window: the main one where it last was, a note's own beside it.
    fn create(&mut self, el: &ActiveEventLoop) -> bool {
        let minimal = self.model.minimal();
        let mut size = if minimal { LogicalSize::new(420.0, 560.0) } else { LogicalSize::new(1000.0, 720.0) };
        let mut attrs = Window::default_attributes().with_title("Kolada").with_min_inner_size(LogicalSize::new(360.0, 320.0)).with_decorations(false);
        if self.main {
            if let Some((x, y, w, h)) = self.model.window_place(minimal) {
                size = LogicalSize::new(w.max(360.0), h.max(320.0));
                attrs = attrs.with_position(winit::dpi::LogicalPosition::new(x, y));
            }
        } else {
            size = LogicalSize::new(NOTE_WINDOW.0, NOTE_WINDOW.1);
        }
        attrs = attrs.with_inner_size(size);
        attrs = winit::platform::x11::WindowAttributesExtX11::with_name(attrs, "kolada", "Kolada");
        attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, "kolada", "Kolada");
        let window = match el.create_window(attrs) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("kolada: could not open a window ({e})");
                return false;
            }
        };
        trace::mark("window made");
        let surface = softbuffer::Context::new(window.clone()).and_then(|c| softbuffer::Surface::new(&c, window.clone()));
        let surface = match surface {
            Ok(s) => s,
            Err(e) => {
                eprintln!("kolada: could not draw in the window ({e})");
                return false;
            }
        };
        window.set_ime_allowed(true);
        self.dark = window.theme().map_or(self.dark, |t| t == Theme::Dark);
        self.model.set_dark(self.dark);
        self.model.set_maximized(window.is_maximized());
        self.shown = Some(Shown { window: window.clone(), surface });
        window.request_redraw();
        true
    }

    /// Draws the frame and puts what changed on the screen.
    fn paint(&mut self, el: &ActiveEventLoop) {
        let Some(shown) = self.shown.as_mut() else { return };
        let size = shown.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if (size.width, size.height) != self.size {
            if shown.surface.resize(w, h).is_err() {
                return;
            }
            self.size = (size.width, size.height);
            self.history.forget();
        }
        let scale = shown.window.scale_factor() as f32;
        self.model.frame(size.width, size.height, scale);
        self.history.drew(self.model.surface().last_damage.clone());
        let Ok(mut buffer) = shown.surface.buffer_mut() else { return };
        let parts = self.history.lacking(buffer.age(), size.width, size.height);
        if !parts.is_empty() {
            blit::copy(self.model.surface().pixels(), &mut buffer, size.width, &parts);
            let rects: Vec<softbuffer::Rect> = parts
                .iter()
                .filter_map(|&(x, y, pw, ph)| Some(softbuffer::Rect { x, y, width: NonZeroU32::new(pw)?, height: NonZeroU32::new(ph)? }))
                .collect();
            let _ = buffer.present_with_damage(&rects);
        }
        if let Some(t) = self.key_at.take() {
            self.model.key_cost(t.elapsed().as_secs_f32() * 1000.0);
        }
        self.place_ime();
        if std::mem::take(&mut self.first) {
            if self.main {
                trace::mark("first frame presented");
                self.model.opened_in(Some(trace::age_ms()));
                if trace::exit_after_first_frame() {
                    el.exit();
                    return;
                }
            }
            self.start();
        }
    }

    /// After the first frame: pictures, the vault's changes from other apps,
    /// the first run's welcome, the index's news, X's exposures, the icon.
    fn start(&mut self) {
        let Some(id) = self.id() else { return };
        let p = self.proxy.clone();
        self.model.start_pictures(move || {
            let _ = p.send_event(Wake::Pictures(id));
        });
        let p = self.proxy.clone();
        self.model.start_arrivals(move || {
            let _ = p.send_event(Wake::Arrivals(id));
        });
        // Before the index starts (start_watching): its thread takes the
        // wake as it starts, and says nothing to a window that had none.
        let p = std::sync::Mutex::new(self.proxy.clone());
        self.model.on_index(move || {
            if let Ok(p) = p.lock() {
                let _ = p.send_event(Wake::Index(id));
            }
        });
        self.start_watching();
        // The window's own icon, for X11's task switchers (Wayland's come
        // from the .desktop file); after the first frame, as it costs a decode.
        if let Some(w) = self.window()
            && let Ok(picture) = image::load_from_memory(include_bytes!("../../../../assets/icon-256.png"))
        {
            let rgba = picture.resize(128, 128, image::imageops::FilterType::Triangle).to_rgba8();
            w.set_window_icon(winit::window::Icon::from_rgba(rgba.into_raw(), 128, 128).ok());
        }
        if let Some(x) = self.window().and_then(|w| super::expose::x_window(w)) {
            let p = self.proxy.clone();
            super::expose::watch(x, move || {
                let _ = p.send_event(Wake::Exposed(id));
            });
        }
        // Writing help: a moment later the gists are read and the helper
        // starts (a note's own window starts none); the first run offers a
        // model where one can be downloaded.
        let p = std::sync::Mutex::new(self.proxy.clone());
        self.model.start_assist(move || {
            if let Ok(p) = p.lock() {
                let _ = p.send_event(Wake::Assist(id));
            }
        });
    }

    fn start_watching(&mut self) {
        let Some(id) = self.id() else { return };
        let p = self.proxy.clone();
        self.model.start_watching(move || {
            let _ = p.send_event(Wake::Outside(id));
        });
    }

    /// The input method's candidates go under the caret.
    fn place_ime(&mut self) {
        let Some(c) = self.model.caret_rect() else { return };
        let s = self.scale();
        let area = ((c.left as f64 * s) as i32, (c.top as f64 * s) as i32, ((c.right - c.left).max(1.0) as f64 * s) as u32, ((c.bottom - c.top) as f64 * s) as u32);
        if self.ime_area != Some(area) {
            self.ime_area = Some(area);
            if let Some(w) = self.window() {
                w.set_ime_cursor_area(PhysicalPosition::new(area.0, area.1), PhysicalSize::new(area.2, area.3));
            }
        }
    }

    /// Which edge of the window the pointer is on, for resizing it.
    fn edge(&self, p: PhysicalPosition<f64>) -> Option<ResizeDirection> {
        let w = self.window()?;
        if w.is_maximized() {
            return None;
        }
        let size = w.inner_size();
        let e = EDGE * w.scale_factor();
        let (left, right) = (p.x < e, p.x > size.width as f64 - e);
        let (top, bottom) = (p.y < e, p.y > size.height as f64 - e);
        use ResizeDirection::*;
        Some(match (left, right, top, bottom) {
            (true, _, true, _) => NorthWest,
            (_, true, true, _) => NorthEast,
            (true, _, _, true) => SouthWest,
            (_, true, _, true) => SouthEast,
            (true, ..) => West,
            (_, true, ..) => East,
            (_, _, true, _) => North,
            (_, _, _, true) => South,
            _ => return None,
        })
    }

    fn set_cursor(&mut self, cursor: CursorIcon) {
        if self.cursor != cursor {
            self.cursor = cursor;
            if let Some(w) = self.window() {
                w.set_cursor(cursor);
            }
        }
    }

    fn mods(&self) -> Mods {
        Mods { ctrl: self.mods.control_key(), shift: self.mods.shift_key(), alt: self.mods.alt_key() }
    }

    fn pointer_moved(&mut self, p: PhysicalPosition<f64>) {
        self.pointer = Some(p);
        let (x, y) = self.dip(p);
        let cursor = match self.edge(p) {
            Some(d) => CursorIcon::from(d),
            None => CursorIcon::Default,
        };
        self.set_cursor(cursor);
        let mut changed = self.model.pointer_moved(x, y);
        if matches!(self.press, Press::Model) {
            changed |= self.model.pointer_dragged(x, y);
        }
        self.changed(changed);
        // A panel to reveal, a chip to peek at: due by the clock.
        self.schedule_tick();
    }

    fn left_down(&mut self) {
        let Some(p) = self.pointer else { return };
        let (x, y) = self.dip(p);
        // Its edges resize the window.
        if let (Some(d), Some(w)) = (self.edge(p), self.window()) {
            let _ = w.drag_resize_window(d);
            return;
        }
        // A click in the middle of a composition settles it first.
        let settled = self.composing && self.model.unmark();
        if settled {
            self.composing = false;
            if let Some(w) = self.window() {
                // Turning the input method off and on drops what it was composing.
                w.set_ime_allowed(false);
                w.set_ime_allowed(true);
            }
        }
        let now = Instant::now();
        let near = (p.x - self.clicks.1.x).abs() < 6.0 && (p.y - self.clicks.1.y).abs() < 6.0;
        let clicks = if self.clicks.0.is_some_and(|t| now - t < DOUBLE_CLICK) && near { self.clicks.2 % 3 + 1 } else { 1 };
        self.clicks = (Some(now), p, clicks);
        if let Some(button) = self.model.window_button(x, y) {
            self.press = Press::Button(button);
            return;
        }
        // The empty strip along the top moves the window; twice maximizes it.
        if !settled && self.model.drags_window(x, y) {
            self.press = Press::Nothing;
            if let Some(w) = self.window().cloned() {
                if clicks == 2 {
                    self.toggle_maximized(&w);
                } else {
                    let _ = w.drag_window();
                }
            }
            return;
        }
        // Ctrl+click follows a link.
        if self.mods.control_key()
            && let Some((changed, effects)) = self.model.link_click(x, y)
        {
            self.changed(changed || settled);
            self.run_effects(effects);
            return;
        }
        self.press = Press::Model;
        let changed = self.model.pointer_down(x, y, clicks, self.mods.shift_key());
        self.edited(changed || settled);
        self.serve();
    }

    fn left_up(&mut self) {
        let p = self.pointer.unwrap_or(self.clicks.1);
        let (x, y) = self.dip(p);
        match std::mem::replace(&mut self.press, Press::Nothing) {
            Press::Button(b) if self.model.window_button(x, y) == Some(b) => {
                let Some(w) = self.window().cloned() else { return };
                match b {
                    WindowButton::Minimize => w.set_minimized(true),
                    WindowButton::Maximize => self.toggle_maximized(&w),
                    WindowButton::Close => self.asks.close = true,
                }
            }
            Press::Model => {
                let (changed, effects) = self.model.pointer_up_at(x, y);
                self.edited(changed);
                self.changed(changed);
                self.run_effects(effects);
                self.serve();
            }
            _ => {}
        }
    }

    /// A right-click: the menu for what is under the pointer, drawn by the
    /// chrome (a misspelled word's fixes first).
    fn right_down(&mut self) {
        let Some(p) = self.pointer else { return };
        let (x, y) = self.dip(p);
        let html = self.clipboard.borrow_mut().has_html();
        let changed = self.model.open_context_menu(x, y, html);
        self.changed(changed);
    }

    fn toggle_maximized(&mut self, w: &Window) {
        let on = !w.is_maximized();
        w.set_maximized(on);
        let changed = self.model.set_maximized(on);
        self.changed(changed);
    }

    fn wheel(&mut self, delta: MouseScrollDelta) {
        // A wheel's notches are lines, of three body lines a notch, eased in
        // by the model; a touchpad's pixels move at once.
        let moved = match delta {
            MouseScrollDelta::LineDelta(_, dy) => self.model.wheel(-dy * LINES_A_NOTCH * LINE, false),
            MouseScrollDelta::PixelDelta(p) => self.model.wheel(-(p.y / self.scale()) as f32, true),
        };
        self.changed(moved);
    }

    /// A key that is not text.
    fn key(&mut self, key: keys::Key) -> bool {
        let Some(action) = keys::action(key, self.mods()) else { return false };
        if self.key_at.is_none() {
            self.key_at = Some(Instant::now());
        }
        // An open menu takes the text system's keys (it walks, picks,
        // closes); any other key puts it away and goes on as usual.
        let closed = !matches!(action, Action::Command(_)) && self.model.close_menu();
        let changed = match action {
            Action::Command(name) => self.model.command(name),
            Action::Tool(tool) => self.model.tool(tool),
            Action::Heading(0) => self.model.block(BlockKind::Body),
            Action::Heading(n) => self.model.block(BlockKind::Heading(n)),
            Action::Do(Do::Copy) => {
                self.copy();
                false
            }
            Action::Do(Do::Cut) => {
                self.copy();
                self.model.insert("")
            }
            Action::Do(Do::Paste) => self.paste(false),
            Action::Do(Do::PastePlain) => self.paste(true),
            Action::Do(Do::Quit) => {
                self.asks.quit = true;
                return true;
            }
            Action::Do(d) => {
                let m = &mut self.model;
                match d {
                    Do::Undo => m.undo(),
                    Do::Redo => m.redo(),
                    Do::NewNote => m.new_note(),
                    Do::Search => m.open_search(),
                    Do::Palette => m.open_palette(),
                    Do::Find => m.open_find(false),
                    Do::Replace => m.open_find(true),
                    Do::FindNext => m.find_again(false),
                    Do::FindPrevious => m.find_again(true),
                    Do::Today => m.toggle_agenda(),
                    Do::Past => m.toggle_scrub(),
                    Do::SideMenu => m.toggle_panel(Panel::Sidebar),
                    Do::Toolbar => m.toggle_panel(Panel::Toolbar),
                    Do::Settings => m.toggle_options(),
                    Do::Back => m.step(false),
                    Do::Forward => m.step(true),
                    Do::Task => m.toggle_task_here(),
                    Do::Save => {
                        m.save_flush();
                        false
                    }
                    Do::Menu => {
                        let html = self.clipboard.borrow_mut().has_html();
                        m.open_menu_at_caret(html)
                    }
                    Do::Copy | Do::Cut | Do::Paste | Do::PastePlain | Do::Quit => false,
                }
            }
        };
        self.edited(changed || closed);
        // Return on a command in the palette may ask for more.
        self.serve();
        true
    }

    fn copy(&mut self) {
        if let Some(c) = self.model.copy() {
            self.clipboard.borrow_mut().copy(c);
        }
    }

    fn paste(&mut self, plain: bool) -> bool {
        let Some(pasted) = self.clipboard.borrow_mut().paste() else { return false };
        match pasted {
            clipboard::Pasted::Kolada(markdown) if !plain => self.model.paste_kolada(&markdown),
            clipboard::Pasted::Kolada(markdown) => self.model.paste_plain(&markdown),
            clipboard::Pasted::Text { plain: text, .. } if plain => self.model.paste_plain(&text),
            clipboard::Pasted::Text { plain: text, html } => self.model.paste_address(&text) || self.model.paste(&text, || html.map(|h| (h, None))),
            // A picture and no text (a screenshot): saved beside the note and embedded.
            clipboard::Pasted::Picture(png) if !plain => self.model.paste_picture(&png),
            clipboard::Pasted::Picture(_) => false,
        }
    }

    /// Where the window is, for the next launch (the main window's only).
    fn save_place(&mut self) {
        if !self.main {
            return;
        }
        let Some(w) = self.window() else { return };
        let s = w.scale_factor();
        let size = w.inner_size().to_logical::<f64>(s);
        // Wayland does not say where a window is (nor let it be put there):
        // its size is kept, beside where it was last known to be.
        let known = self.model.window_place(self.model.minimal()).map_or((0.0, 0.0), |(x, y, _, _)| (x, y));
        let at = w.outer_position().map_or(known, |p| {
            let p = p.to_logical::<f64>(s);
            (p.x, p.y)
        });
        if !w.is_maximized() {
            self.model.place_window((at.0, at.1, size.width, size.height));
        }
    }

    /// Does what the model asked for since the last look.
    fn serve(&mut self) {
        // A menu's clipboard items: ours to do, with the desktop's clipboard.
        for pick in self.model.take_menu_picks() {
            let changed = match pick {
                MenuAction::Cut => {
                    self.copy();
                    self.model.insert("")
                }
                MenuAction::Copy => {
                    self.copy();
                    false
                }
                MenuAction::Paste => self.paste(false),
                MenuAction::PastePlain => self.paste(true),
                _ => false,
            };
            self.edited(changed);
            self.changed(true);
        }
        let effects = self.model.take_effects();
        if !effects.is_empty() {
            self.run_effects(effects);
        }
        for request in self.model.take_requests() {
            match request {
                Request::ShowVault(path) => {
                    outside::open(&path.to_string_lossy());
                }
                Request::Minimal(on) => self.set_minimal(on),
                Request::Quit => self.asks.quit = true,
                // The chooser on a thread of its own: the window keeps
                // drawing (and answering) while it is open.
                Request::Pick(pick) => {
                    let (p, parent) = (self.proxy.clone(), self.parent_window());
                    let Some(id) = self.id() else { continue };
                    std::thread::spawn(move || {
                        let _ = p.send_event(Wake::Picked(id, pick, outside::pick(pick, &parent)));
                    });
                }
                Request::ImportClipboard => {
                    let text = self.clipboard.borrow_mut().text();
                    let changed = self.model.import_clipboard(text);
                    self.edited(changed);
                    self.changed(true);
                }
                Request::Export(kind) => {
                    if let Some((_, name)) = self.model.outgoing() {
                        self.export(&name, kind);
                    }
                }
                // A note's window: its note back in the main one.
                Request::ToMain(rel) => self.asks.to_main = Some(rel),
                Request::Updates(on) => self.watch_updates(on),
                // The pill (everything saved): the update in place, then
                // the app starts again as it.
                Request::InstallUpdate => match super::updates::put_in_place() {
                    Some(exe) => {
                        self.asks.restart = Some(exe);
                        self.asks.quit = true;
                    }
                    None => {
                        self.model.whisper("Could not update: try again later");
                        self.changed(true);
                    }
                },
                // Beam's: the app's (its popup is the main model's).
                Request::QuickOpen => self.asks.beam = Some(false),
                Request::QuickDevices => self.asks.beam = Some(true),
                Request::BeamNote => self.asks.beam_note = true,
                Request::QuickKeys(_) => self.asks.beam_keys = true,
                // No tray icon or quick note keys here yet.
                Request::Tray(_) | Request::Hotkey(_) => {}
            }
        }
    }

    /// The daily check for updates, on (or off: it stops looking).
    fn watch_updates(&self, on: bool) {
        let p = self.proxy.clone();
        super::updates::watch(on, move || {
            let _ = p.send_event(Wake::UpdateReady);
        });
    }

    fn run_effects(&mut self, effects: Vec<Effect>) {
        for e in effects {
            match e {
                Effect::Clipboard(text, said) => {
                    self.clipboard.borrow_mut().set_text(&text, None);
                    if !said.is_empty() {
                        self.model.whisper(&said);
                    }
                }
                Effect::Rich { text, html } => self.clipboard.borrow_mut().set_text(&text, Some(&html)),
                Effect::Export { name } => self.export(&name, kolada_app::model::share::Export::Markdown),
                Effect::Url(url) => {
                    if !outside::open(&url) {
                        self.model.whisper("Could not open it");
                    }
                }
                Effect::Reveal(path) => {
                    let dir = if path.is_dir() { path } else { path.parent().map(PathBuf::from).unwrap_or(path) };
                    outside::open(&dir.to_string_lossy());
                }
                Effect::Terminal { dir, command } => {
                    if !outside::in_terminal(&dir, &command) {
                        self.model.whisper("Could not open a terminal");
                    }
                }
                // A note in a window of its own: the app makes it.
                Effect::Window(path) => self.asks.windows.push(path),
            }
            self.changed(true);
        }
    }

    /// The note exported as `kind`: the page as it is now, written where
    /// the desktop's chooser says, on a thread (the chooser, and printing a
    /// PDF with what the desktop has, take their time); it says when done.
    fn export(&mut self, name: &str, kind: kolada_app::model::share::Export) {
        use kolada_app::model::share::Export;
        let Some(id) = self.id() else { return };
        let Some(page) = self.model.export_page(kind) else { return };
        let (p, name, parent) = (self.proxy.clone(), name.to_string(), self.parent_window());
        std::thread::spawn(move || {
            let Some(place) = outside::save_place(&name, kind, &parent) else {
                let _ = p.send_event(Wake::Whisper(id, "Exporting needs the desktop's file chooser"));
                return;
            };
            let Some(path) = place else { return };
            let done = if kind == Export::Pdf {
                let _ = p.send_event(Wake::Whisper(id, "Making the PDF\u{2026}"));
                super::print::print(&page, &path).map_err(str::to_string)
            } else {
                std::fs::write(&path, page).map_err(|_| String::new())
            };
            let _ = p.send_event(Wake::Printed(id, path, done));
        });
    }

    /// The window, as the portal names a dialog's parent (X11's id; on
    /// Wayland none, which needs a handle exported first).
    fn parent_window(&self) -> String {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match self.window().and_then(|w| w.window_handle().ok()).map(|h| h.as_raw()) {
            Some(RawWindowHandle::Xlib(h)) => format!("x11:{:x}", h.window),
            Some(RawWindowHandle::Xcb(h)) => format!("x11:{:x}", h.window),
            _ => String::new(),
        }
    }

    /// What a chooser came back with.
    fn picked(&mut self, pick: Pick, chosen: Option<Vec<PathBuf>>) -> bool {
        let Some(chosen) = chosen else {
            self.model.whisper("Choosing files needs the desktop's file chooser");
            return true;
        };
        if chosen.is_empty() {
            return false;
        }
        let changed = match pick {
            Pick::Files => self.model.import_files(chosen),
            Pick::Folder => self.model.import_folder(&chosen[0]),
            Pick::Vault => self.model.switch_vault(chosen[0].clone()),
        };
        if pick == Pick::Vault && changed {
            self.start_watching();
        }
        self.edited(changed);
        true
    }

    /// The window small with the note alone, or full again as it was.
    fn set_minimal(&mut self, on: bool) {
        let Some(w) = self.window().cloned() else { return };
        if on {
            self.full_size = Some(w.inner_size());
            let _ = w.request_inner_size(LogicalSize::new(420.0, 560.0));
        } else {
            let size = self.full_size.take().unwrap_or_else(|| LogicalSize::new(1000.0, 720.0).to_physical(w.scale_factor()));
            let _ = w.request_inner_size(size);
        }
        self.changed(true);
    }

    /// Something came for this window from a thread.
    fn woken(&mut self, wake: Wake) {
        let changed = match wake {
            Wake::Pictures(_) => {
                let changed = self.model.poll_pictures();
                changed || self.model.pictures_fading()
            }
            Wake::Arrivals(_) => {
                let changed = self.model.poll_arrivals();
                self.edited(changed);
                changed
            }
            Wake::Index(_) => self.model.due_ready() | self.model.update_related() | self.model.index_arrived(),
            Wake::Outside(_) => self.model.poll_outside(),
            Wake::Printed(_, path, Ok(())) => {
                self.model.exported(&path, true);
                true
            }
            Wake::Printed(_, path, Err(why)) if why.is_empty() => {
                self.model.exported(&path, false);
                true
            }
            Wake::Printed(_, _, Err(why)) => {
                self.model.whisper(&why);
                true
            }
            Wake::Picked(_, pick, chosen) => self.picked(pick, chosen),
            Wake::Whisper(_, said) => {
                self.model.whisper(said);
                true
            }
            // All of it goes up again, not only what changed.
            Wake::Assist(_) => {
                let changed = self.model.assist_events();
                self.edited(changed);
                changed
            }
            Wake::Exposed(_) => {
                self.history.forget();
                true
            }
            Wake::Appearance(dark) => {
                let changed = dark != self.dark;
                self.dark = dark;
                self.model.set_dark(dark);
                changed
            }
            Wake::Spelling => {
                self.spelling = true;
                self.spell_at = Some(Instant::now());
                false
            }
            Wake::Handed(_) | Wake::UpdateReady | Wake::Beam | Wake::BeamKeys | Wake::BeamPicked(_) => false,
        };
        self.changed(changed);
    }

    /// The window comes to the front.
    fn raise(&self) {
        if let Some(w) = self.window() {
            w.set_minimized(false);
            w.focus_window();
            // Wayland lets no app take the focus: it asks for attention.
            if matches!(raw_window_handle::HasDisplayHandle::display_handle(&**w).map(|d| d.as_raw()), Ok(raw_window_handle::RawDisplayHandle::Wayland(_))) {
                w.request_user_attention(Some(winit::window::UserAttentionType::Informational));
            }
        }
    }

    /// What is due by the clock: a step of motion, the note written.
    fn due(&mut self) {
        let now = Instant::now();
        if self.tick_at.is_some_and(|t| t <= now) {
            self.tick_at = None;
            let mut changed = false;
            if self.drag.active() {
                let (x, y) = self.drag_point();
                changed |= self.model.drag_over(x, y);
            }
            let changed = self.model.tick() || changed;
            if changed {
                self.redraw();
            }
            self.schedule_tick();
        }
        if self.save_at.is_some_and(|t| t <= now) {
            self.save_at = None;
            self.model.save();
        }
        if self.spell_at.is_some_and(|t| t <= now) {
            let (changed, more) = self.model.spell_step();
            self.spell_at = more.then(|| now + SPELL_MORE);
            if changed {
                self.redraw();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => self.paint(el),
            WindowEvent::CloseRequested => self.asks.close = true,
            WindowEvent::Resized(_) => {
                let maximized = self.window().is_some_and(|w| w.is_maximized());
                self.model.set_maximized(maximized);
                self.redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => self.redraw(),
            WindowEvent::ThemeChanged(theme) => {
                self.dark = theme == Theme::Dark;
                self.model.set_dark(self.dark);
                self.redraw();
            }
            WindowEvent::Focused(false) => {
                self.model.save();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => self.pointer_moved(position),
            WindowEvent::CursorLeft { .. } => {
                self.pointer = None;
                let changed = self.model.pointer_left();
                self.changed(changed);
                self.schedule_tick();
            }
            WindowEvent::MouseInput { state, button, .. } => match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => self.left_down(),
                (MouseButton::Left, ElementState::Released) => self.left_up(),
                (MouseButton::Right, ElementState::Pressed) => self.right_down(),
                (MouseButton::Back, ElementState::Pressed) => {
                    let changed = self.model.step(false);
                    self.changed(changed);
                }
                (MouseButton::Forward, ElementState::Pressed) => {
                    let changed = self.model.step(true);
                    self.changed(changed);
                }
                _ => {}
            },
            WindowEvent::MouseWheel { delta, .. } => self.wheel(delta),
            WindowEvent::HoveredFile(path) => self.drag_hover(path),
            WindowEvent::HoveredFileCancelled => {
                self.drag.end();
                let changed = self.model.drag_leave();
                self.changed(changed);
            }
            WindowEvent::DroppedFile(path) => self.drag.drop(path),
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(text, cursor) => {
                    self.composing = !text.is_empty();
                    // Cleared: winit clears what was shown before every
                    // commit (and on Escape in the input method), so the
                    // composition is taken back, not kept.
                    let changed = if text.is_empty() {
                        self.model.cancel_marked()
                    } else {
                        let (at, end) = cursor.unwrap_or((text.len(), text.len()));
                        let units = |b: usize| text[..b.min(text.len())].encode_utf16().count();
                        self.model.set_marked_converting(&text, units(at), units(end).saturating_sub(units(at)))
                    };
                    self.edited(changed);
                }
                Ime::Commit(text) => {
                    self.composing = false;
                    let changed = self.model.commit_marked(&text);
                    self.edited(changed);
                }
                Ime::Enabled | Ime::Disabled => {}
            },
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed || self.composing {
                    return;
                }
                let named = match event.logical_key {
                    Key::Named(n) => match n {
                        NamedKey::ArrowLeft => Some(Named::Left),
                        NamedKey::ArrowRight => Some(Named::Right),
                        NamedKey::ArrowUp => Some(Named::Up),
                        NamedKey::ArrowDown => Some(Named::Down),
                        NamedKey::Home => Some(Named::Home),
                        NamedKey::End => Some(Named::End),
                        NamedKey::PageUp => Some(Named::PageUp),
                        NamedKey::PageDown => Some(Named::PageDown),
                        NamedKey::Backspace => Some(Named::Backspace),
                        NamedKey::Delete => Some(Named::Delete),
                        NamedKey::Enter => Some(Named::Enter),
                        NamedKey::Tab => Some(Named::Tab),
                        NamedKey::Escape => Some(Named::Escape),
                        NamedKey::Insert => Some(Named::Insert),
                        NamedKey::F3 => Some(Named::F3),
                        NamedKey::F10 => Some(Named::F10),
                        NamedKey::ContextMenu => Some(Named::Menu),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(n) = named {
                    self.key(keys::Key::Named(n));
                    return;
                }
                if let Key::Character(c) = event.key_without_modifiers()
                    && self.key(keys::Key::Char(c.as_str()))
                {
                    return;
                }
                if let Some(text) = event.text.as_ref().filter(|t| keys::types(t, self.mods())) {
                    if self.key_at.is_none() {
                        self.key_at = Some(Instant::now());
                    }
                    let changed = self.model.commit_marked(text);
                    self.edited(changed);
                }
            }
            _ => {}
        }
    }
}

impl App {
    fn win(&mut self, id: WindowId) -> Option<&mut Win> {
        self.wins.iter_mut().find(|w| w.id() == Some(id))
    }

    /// After the main window's first frame: what the app does once, for all
    /// its windows.
    fn start(&mut self, el: &ActiveEventLoop) {
        super::services::warm();
        self.start_beam(el);
        if let Some(l) = self.listener.take() {
            let p = self.proxy.clone();
            instance::listen(l, move |h| {
                let _ = p.send_event(Wake::Handed(h));
            });
        }
        // The dictionaries, read on a thread of their own.
        let p = self.proxy.clone();
        std::thread::spawn(move || {
            kolada_app::services::Priority::lower_thread(&super::services::LinuxPriority);
            if super::spell::load(kolada_core::app_data_dir().as_deref()) {
                let _ = p.send_event(Wake::Spelling);
            }
        });
        // Dark or light as the desktop says, now and whenever it changes.
        let p = self.proxy.clone();
        super::appearance::follow(kolada_core::app_data_dir(), move |dark| {
            let _ = p.send_event(Wake::Appearance(dark));
        });
        // The per-user install updates itself: Settings offers the switch,
        // and the daily check starts while it is on.
        if super::updates::offered()
            && let Some(main) = self.wins.first_mut()
        {
            main.model.set_updater(true);
            let on = main.model.checks_for_updates();
            main.watch_updates(on);
        }
    }

    /// A note in a window of its own, beside the main one: its own model on
    /// the same vault. A note already in a window of its own comes forward.
    fn open_window(&mut self, el: &ActiveEventLoop, path: PathBuf) {
        let Some(main) = self.wins.first() else { return };
        let vault = main.model.vault().to_path_buf();
        let Some(rel) = kolada_core::vault::relative(&vault, &path) else { return };
        if let Some(w) = self.wins.iter().skip(1).find(|w| w.model.note_rel().as_deref() == Some(rel.as_str())) {
            w.raise();
            return;
        }
        let model = super::linux_model(Model::note_window(super::services::services(), vault, &rel, self.dark));
        let mut win = Win::new(model, false, self.proxy.clone(), self.clipboard.clone(), self.dark);
        win.spelling = self.spelling;
        if win.create(el) {
            self.wins.push(win);
        }
    }

    /// Does what the window `i` asked for in its last event.
    fn settle(&mut self, el: &ActiveEventLoop, i: usize) {
        let asks = std::mem::take(&mut self.wins[i].asks);
        for path in asks.windows {
            self.open_window(el, path);
        }
        if let Some(exe) = asks.restart {
            self.restart = Some(exe);
        }
        if let Some(devices) = asks.beam {
            let dos = self.wins[0].model.quick_open(devices);
            self.beam_do(dos);
        }
        if asks.beam_note {
            let dos = self.wins[0].model.quick_beam_note();
            self.beam_do(dos);
            self.wins[0].changed(true);
        }
        if asks.beam_keys {
            self.follow_keys();
        }
        if asks.quit {
            return self.quit(el);
        }
        if let Some(rel) = asks.to_main {
            self.close(el, i);
            if let Some(main) = self.wins.first_mut() {
                main.model.open(&rel);
                main.raise();
                main.changed(true);
            }
            return;
        }
        if asks.close {
            if self.wins[i].main { self.quit(el) } else { self.close(el, i) }
        }
    }

    /// A note's window goes, its note written first.
    fn close(&mut self, _el: &ActiveEventLoop, i: usize) {
        if i == 0 || i >= self.wins.len() {
            return;
        }
        let mut w = self.wins.remove(i);
        w.model.save_flush();
        // The main window shows the note as written in the other.
        if let Some(main) = self.wins.first_mut() {
            let changed = main.model.poll_outside();
            main.changed(changed);
        }
    }

    fn quit(&mut self, el: &ActiveEventLoop) {
        for w in &mut self.wins {
            w.save_place();
            w.model.save_flush();
        }
        el.exit();
    }

    /// Launched again: the main window comes to the front, with what that
    /// launch was asked to open.
    fn handed(&mut self, h: instance::Handed) {
        let Some(main) = self.wins.first_mut() else { return };
        main.raise();
        match h {
            instance::Handed::Show => {}
            instance::Handed::Open(path) => {
                if main.model.open_path(&path) {
                    main.start_watching();
                }
            }
            instance::Handed::Vault(dir) => {
                if main.model.switch_vault(dir) {
                    main.start_watching();
                }
            }
            instance::Handed::New => {
                let changed = main.model.new_note();
                main.edited(changed);
            }
            instance::Handed::Search => {
                main.model.open_search();
            }
            instance::Handed::Beam => {
                let dos = main.model.quick_open(false);
                self.beam_do(dos);
                return;
            }
        }
        main.serve();
        main.changed(true);
    }
}

/// The computer's name, as the other devices list it.
fn host_name() -> Option<String> {
    let mut buf = [0u8; 256];
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    ok.then(|| String::from_utf8_lossy(&buf[..end]).into_owned()).filter(|n| !n.is_empty())
}

impl App {
    /// Beam, after the first frame: its engine (no connection until paired),
    /// its popup made and kept hidden, and its keys where the desktop allows.
    fn start_beam(&mut self, el: &ActiveEventLoop) {
        let base = std::env::var("KOLADA_SYNC_BASE").unwrap_or_else(|_| kolada_app::quick::SERVER.to_string());
        let name = std::env::var("KOLADA_SYNC_NAME").ok().or_else(host_name).unwrap_or_else(|| "Linux".into());
        let p = Mutex::new(self.proxy.clone());
        self.wins[0].model.start_quick(&base, "desktop", &name, Arc::new(move || {
            if let Ok(p) = p.lock() {
                let _ = p.send_event(Wake::Beam);
            }
        }));
        if !self.wins[0].model.quick_on() {
            return;
        }
        self.make_popup(el);
        self.follow_keys();
        if std::mem::take(&mut self.beam_asked) {
            let dos = self.wins[0].model.quick_open(false);
            self.beam_do(dos);
        }
    }

    /// The popup's window, hidden until shown: no title bar, kept above,
    /// off the task bar.
    fn make_popup(&mut self, el: &ActiveEventLoop) {
        let mut attrs = Window::default_attributes()
            .with_title("Beam")
            .with_decorations(false)
            .with_resizable(false)
            .with_visible(false)
            .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
            .with_inner_size(LogicalSize::new(q::W as f64, q::H as f64));
        attrs = winit::platform::x11::WindowAttributesExtX11::with_name(attrs, "kolada", "Kolada");
        attrs = winit::platform::x11::WindowAttributesExtX11::with_x11_window_type(attrs, vec![winit::platform::x11::WindowType::Utility]);
        attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, "kolada", "Kolada");
        let Ok(window) = el.create_window(attrs) else { return };
        let window = Rc::new(window);
        let Ok(surface) = softbuffer::Context::new(window.clone()).and_then(|c| softbuffer::Surface::new(&c, window.clone())) else { return };
        window.set_ime_allowed(true);
        self.popup = Some(Popup { shown: Shown { window, surface }, size: (0, 0), pointer: None, mods: ModifiersState::empty(), picking: false, focused: false });
    }

    /// Takes Beam's keys while Beam wants them (paired, and on in
    /// Settings), and lets them go otherwise; the model says whether held.
    fn follow_keys(&mut self) {
        let wanted = self.wins[0].model.quick_wants_keys();
        if wanted && self.keys.is_none() {
            let p = Mutex::new(self.proxy.clone());
            self.keys = beam::take_keys(move || {
                if let Ok(p) = p.lock() {
                    let _ = p.send_event(Wake::BeamKeys);
                }
            });
        } else if !wanted {
            self.keys = None;
        }
        let held = self.keys.is_some();
        if self.wins[0].model.set_quick_keys_held(held) {
            self.wins[0].changed(true);
        }
    }

    /// What Beam's engine said: news for the popup and the window's button.
    fn beam_events(&mut self) {
        let dos = self.wins[0].model.quick_events();
        self.beam_do(dos);
        self.wins[0].changed(true);
    }

    /// Does what Beam's model asked, then draws the popup and sets its clock.
    fn beam_do(&mut self, dos: Vec<QuickDo>) {
        for d in dos {
            match d {
                QuickDo::Show { height } => self.popup_show(height, true),
                QuickDo::Shrink { height } => self.popup_show(height, false),
                QuickDo::Hide => {
                    if let Some(p) = &self.popup {
                        p.shown.window.set_visible(false);
                    }
                }
                QuickDo::Copy { text, secret } => {
                    let mut c = self.clipboard.borrow_mut();
                    if secret { c.set_secret(&text) } else { c.set_text(&text, None) }
                }
                QuickDo::Clear(text) => {
                    let mut c = self.clipboard.borrow_mut();
                    if c.text().as_deref() == Some(text.as_str()) {
                        c.set_text("", None);
                    }
                }
                QuickDo::Url(url) => {
                    outside::open(&url);
                }
                QuickDo::BringUp => self.wins[0].raise(),
                QuickDo::Notify { title, text } => kolada_app::services::Notify::notify(&super::notify::LinuxNotify, &title, &text),
                QuickDo::Keys => self.follow_keys(),
                QuickDo::PickFile => {
                    if let Some(p) = &mut self.popup {
                        p.picking = true;
                    }
                    let p = self.proxy.clone();
                    std::thread::spawn(move || {
                        let ask = super::portal::Ask::Open { title: "Beam a picture".into(), multiple: false, directory: false };
                        let _ = p.send_event(Wake::BeamPicked(super::portal::choose(&ask, "").ok()));
                    });
                }
                QuickDo::SaveFile { name, bytes } => {
                    let (p, id) = (self.proxy.clone(), self.wins[0].id());
                    std::thread::spawn(move || {
                        let folder = kolada_core::default_vault_dir().and_then(|v| v.parent().map(std::path::Path::to_path_buf)).unwrap_or_else(std::env::temp_dir);
                        let ask = super::portal::Ask::Save { title: "Save the picture".into(), name, folder };
                        if let Ok(chosen) = super::portal::choose(&ask, "")
                            && let Some(path) = chosen.into_iter().next()
                            && std::fs::write(&path, &bytes).is_err()
                            && let Some(id) = id
                        {
                            let _ = p.send_event(Wake::Whisper(id, "The picture couldn't be saved there."));
                        }
                    });
                }
                QuickDo::CopyPicture(bytes) => {
                    self.clipboard.borrow_mut().set_picture(&bytes);
                }
                // A picture full size: a copy among Kolada's own files (never
                // the vault), in the desktop's viewer.
                QuickDo::OpenFile { name, bytes } => {
                    let Some(dir) = kolada_core::app_data_dir().map(|d| d.join("sync").join("open")) else { continue };
                    let name: String = name.chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
                    let file = dir.join(if name.is_empty() { "Picture".into() } else { name });
                    if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&file, &bytes).is_ok() {
                        outside::open(&file.to_string_lossy());
                    }
                }
            }
        }
        if let Some(p) = &self.popup {
            p.shown.window.request_redraw();
        }
        self.beam_at = self.wins[0].model.quick_next_due();
    }

    /// The popup shown at `height` DIPs, at the top right of the screen the
    /// main window is on (where the desktop lets a window say where it
    /// goes); with `focus`, taking the keys.
    fn popup_show(&mut self, height: f32, focus: bool) {
        let Some(p) = &mut self.popup else { return };
        if !p.shown.window.is_visible().unwrap_or(false) {
            p.focused = false;
        }
        let w = &p.shown.window;
        let _ = w.request_inner_size(LogicalSize::new(q::W as f64, height as f64));
        if let Some(m) = self.wins[0].window().and_then(|m| m.current_monitor()).or_else(|| w.primary_monitor()) {
            let s = m.scale_factor();
            let (mx, my, mw) = (m.position().x as f64, m.position().y as f64, m.size().width as f64);
            w.set_outer_position(PhysicalPosition::new(mx + mw - (q::W as f64 + 16.0) * s, my + 48.0 * s));
        }
        w.set_visible(true);
        if focus {
            w.focus_window();
        }
        w.request_redraw();
    }

    /// The popup's events: drawn by the main model, its input passed on.
    fn popup_event(&mut self, event: WindowEvent) {
        let Some(p) = self.popup.as_mut() else { return };
        let scale = p.shown.window.scale_factor();
        let dip = |at: PhysicalPosition<f64>| ((at.x / scale) as f32, (at.y / scale) as f32);
        let model = &mut self.wins[0].model;
        let mut dos = Vec::new();
        let mut redraw = false;
        match event {
            WindowEvent::RedrawRequested => {
                let size = p.shown.window.inner_size();
                let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
                if (size.width, size.height) != p.size {
                    if p.shown.surface.resize(w, h).is_err() {
                        return;
                    }
                    p.size = (size.width, size.height);
                }
                model.quick_frame(size.width, size.height, scale as f32);
                let Some(surface) = model.quick_surface() else { return };
                let Ok(mut buffer) = p.shown.surface.buffer_mut() else { return };
                if surface.pixels().len() == buffer.len() {
                    blit::copy(surface.pixels(), &mut buffer, size.width, &[(0, 0, size.width, size.height)]);
                }
                let _ = buffer.present();
                return;
            }
            WindowEvent::CloseRequested => dos = model.quick_deactivated(),
            WindowEvent::Focused(true) => p.focused = true,
            WindowEvent::Focused(false) if !p.picking && p.focused => dos = model.quick_deactivated(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => redraw = true,
            WindowEvent::ModifiersChanged(m) => p.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                p.pointer = Some(position);
                let (x, y) = dip(position);
                redraw = model.quick_pointer_moved(x, y);
                p.shown.window.set_cursor(if model.quick_takes_typing() { CursorIcon::Text } else { CursorIcon::Default });
            }
            WindowEvent::CursorLeft { .. } => {
                p.pointer = None;
                redraw = model.quick_pointer_left();
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some(at) = p.pointer {
                    let (x, y) = dip(at);
                    dos = model.quick_click(x, y);
                    redraw = true;
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, dy) => -dy * Model::quick_notch(),
                    MouseScrollDelta::PixelDelta(d) => -(d.y / scale) as f32,
                };
                redraw = model.quick_wheel(dy);
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                for c in text.chars() {
                    redraw |= model.quick_char(c);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let ctrl = p.mods.control_key();
                let clipboard = self.clipboard.clone();
                let clip = || beam::clip(&mut clipboard.borrow_mut());
                let key = match &event.logical_key {
                    Key::Named(NamedKey::Escape) => Some(QuickKey::Escape),
                    Key::Named(NamedKey::Enter) if p.mods.shift_key() => Some(QuickKey::NewLine),
                    Key::Named(NamedKey::Enter) => Some(QuickKey::Enter),
                    Key::Named(NamedKey::Backspace) => Some(QuickKey::Backspace),
                    Key::Character(c) if ctrl && c.eq_ignore_ascii_case("v") => Some(QuickKey::Paste),
                    _ => None,
                };
                match key {
                    Some(k) => {
                        dos = model.quick_key(k, &clip);
                        redraw = true;
                    }
                    None if !ctrl => {
                        if let Some(text) = &event.text {
                            for c in text.chars().filter(|c| !c.is_control()) {
                                redraw |= model.quick_char(c);
                            }
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
        if redraw {
            p.shown.window.request_redraw();
        }
        if !dos.is_empty() || redraw {
            self.beam_do(dos);
        }
    }
}

impl ApplicationHandler<Wake> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if let Some(main) = self.wins.first_mut()
            && main.shown.is_none()
            && !main.create(el)
        {
            el.exit();
        }
    }

    fn new_events(&mut self, _el: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. } | StartCause::WaitCancelled { .. } | StartCause::Poll) {
            for w in &mut self.wins {
                w.due();
            }
            if self.beam_at.is_some_and(|t| t <= Instant::now()) {
                self.beam_at = None;
                let dos = self.wins[0].model.quick_tick();
                self.beam_do(dos);
                self.wins[0].changed(true);
            }
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, wake: Wake) {
        let to = match &wake {
            Wake::Pictures(id) | Wake::Arrivals(id) | Wake::Index(id) | Wake::Outside(id) | Wake::Printed(id, ..) | Wake::Picked(id, ..) | Wake::Whisper(id, _) | Wake::Exposed(id) | Wake::Assist(id) => Some(*id),
            Wake::Appearance(dark) => {
                self.dark = *dark;
                None
            }
            Wake::Spelling => {
                self.spelling = true;
                None
            }
            Wake::Handed(h) => return self.handed(h.clone()),
            Wake::Beam => return self.beam_events(),
            Wake::BeamKeys => {
                let clipboard = self.clipboard.clone();
                let dos = self.wins[0].model.quick_keys(false, &|| beam::clip(&mut clipboard.borrow_mut()));
                return self.beam_do(dos);
            }
            Wake::BeamPicked(chosen) => {
                if let Some(p) = &mut self.popup {
                    p.picking = false;
                }
                let dos = self.wins[0].model.quick_attach(&chosen.clone().unwrap_or_default());
                return self.beam_do(dos);
            }
            // The pill, in the main window (the side menu is there).
            Wake::UpdateReady => {
                if let (Some(version), Some(main)) = (super::updates::take_ready(), self.wins.first_mut()) {
                    let changed = main.model.update_ready(&version);
                    main.changed(changed);
                }
                return;
            }
        };
        match to {
            Some(id) => {
                if let Some(w) = self.win(id) {
                    w.woken(wake);
                }
            }
            None => {
                for w in &mut self.wins {
                    w.woken(wake.clone());
                }
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.popup.as_ref().is_some_and(|p| p.shown.window.id() == id) {
            return self.popup_event(event);
        }
        let Some(i) = self.wins.iter().position(|w| w.id() == Some(id)) else { return };
        let first = self.wins[i].first && self.wins[i].main;
        self.wins[i].window_event(el, event);
        if first && !self.wins[i].first {
            self.start(el);
        }
        self.settle(el, i);
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // A drop's files have all been named: take them in.
        for w in &mut self.wins {
            if !w.drag.dropped.is_empty() {
                w.take_drop();
            }
        }
        let next = self.wins.iter().filter_map(Win::next).chain(self.beam_at).min();
        el.set_control_flow(match next {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        for w in &mut self.wins {
            w.model.save_flush();
        }
    }
}
