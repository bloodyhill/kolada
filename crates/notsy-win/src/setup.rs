//! Notsy installing and removing itself, with a window of its own drawn by
//! the same engine as the app (the screen is `notsy_ui::setup`; the work is
//! `work`). `Notsy-Setup.exe` unpacks this program to the temp folder and
//! runs `notsy --setup <Notsy-Setup.exe>`; Settings > Apps runs `notsy
//! --uninstall`. With `--quiet` there is no window: install with the usual
//! choices (or remove), and say how it went in the exit code.
//!
//! Neither takes the app's instance, reads its session or writes a note.

mod work;

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use notsy_core::found;
use notsy_ui::scene::{self, Eng, Fonts, Target};
use notsy_ui::setup::{self as screen, Folder, Hit, Stage, View};
use notsy_ui::{display, theme};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT, ScreenToClient};
/// Sent when the pointer leaves the window (TrackMouseEvent).
const WM_MOUSELEAVE: u32 = 0x02A3;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HTCAPTION, HTCLIENT, IDC_ARROW, KillTimer, LoadCursorW,
    LoadIconW, MSG, PostMessageW, PostQuitMessage, RegisterClassExW, SPI_GETWORKAREA, SW_MINIMIZE, SW_SHOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, SystemParametersInfoW, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE,
    WM_DESTROY, WM_DPICHANGED, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_CAPTION,
    WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
};
use windows::core::{PCWSTR, w};

use crate::cpu::{self, Surface};
use work::{Choices, Pick};

/// Progress from the worker (wparam: per mille), and its end (lparam:
/// `Box<Result<Option<PathBuf>, String>>`, the installed program).
const WM_APP_STEP: u32 = WM_APP + 40;
const WM_APP_END: u32 = WM_APP + 41;
const TICK: usize = 1;
/// Options take this long to come out or go back.
const REVEAL: Duration = Duration::from_millis(260);

/// What this window is for.
enum Job {
    Install { stub: PathBuf },
    Remove,
}

/// A place the notes could go, as listed.
struct Place {
    name: String,
    detail: String,
    pick: Pick,
}

struct Ui {
    hwnd: HWND,
    job: Job,
    eng: Eng,
    fonts: Fonts,
    surface: Option<Surface>,
    hits: Vec<(scene::Rect, Target)>,
    hover: Option<Target>,
    tracking: bool,
    dark: bool,
    stage: Stage,
    /// The version installed now, when this updates it.
    updating: Option<String>,
    places: Vec<Place>,
    chosen: usize,
    choices: [bool; 3],
    /// Options: shown or not, and the reveal under way (from, since).
    options: bool,
    reveal: (f32, Instant),
    /// How far the work is, as said and as shown (the fill eases after it).
    done: f32,
    shown: f32,
    message: Option<String>,
    installed: Option<PathBuf>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|u| u.try_borrow_mut().ok()?.as_mut().map(f))
}

fn has(args: &[OsString], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// `notsy --setup <stub> [--quiet] [--no-launch]`.
pub fn install_main(args: &[OsString]) -> i32 {
    if crate::updates::packaged() {
        return 2;
    }
    let Some(stub) = args.first().map(PathBuf::from) else { return 2 };
    if has(args, "--quiet") {
        let choices = work::installed().map_or(Choices::FIRST, |i| i.choices);
        return match work::install(&stub, choices, &|_| {}) {
            Ok(app) => {
                if !has(args, "--no-launch") {
                    let pick = places().into_iter().next().map(|p| p.pick).unwrap_or(Pick::Other(None));
                    start(&app, &pick);
                }
                0
            }
            Err(_) => 1,
        };
    }
    open(Job::Install { stub }).0
}

/// `notsy --uninstall [--quiet]`, as Settings > Apps runs it.
pub fn uninstall_main(args: &[OsString]) -> i32 {
    if crate::updates::packaged() {
        return 2;
    }
    if has(args, "--quiet") {
        let ok = work::uninstall(&|_| {}).is_ok();
        if ok {
            work::leave();
        }
        return if ok { 0 } else { 1 };
    }
    let (code, end) = open(Job::Remove);
    // Removed (not kept): the program itself goes last.
    if end == Stage::Removed {
        work::leave();
    }
    code
}

/// Where the notes can go: a new Documents\Notsy (or the one there from
/// before), what other apps keep in Documents, any folder.
fn places() -> Vec<Place> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let new = notsy_core::default_vault_dir().unwrap_or_else(|| PathBuf::from("Notsy"));
    let mut found: Vec<found::Found> = Vec::new();
    for docs in work::documents() {
        for f in found::in_documents(&docs) {
            if !found.iter().any(|g| g.path == f.path) {
                found.push(f);
            }
        }
    }
    found.sort_by(|a, b| a.kind.cmp(&b.kind).then(b.notes.cmp(&a.notes)));
    let mut out = Vec::new();
    match found.iter().position(|f| f.kind == found::Kind::Notsy && f.path == new) {
        Some(i) => {
            let own = found.remove(i);
            let (name, detail) = work::describe(&own, home.as_deref(), &new);
            out.push(Place { name, detail, pick: Pick::Open(own.path) });
        }
        None => out.push(Place { name: "A new folder".into(), detail: work::short(&new, home.as_deref()), pick: Pick::New(new.clone()) }),
    }
    for f in found.iter().filter(|f| f.kind != found::Kind::Notsy).take(found::MOST) {
        let (name, detail) = work::describe(f, home.as_deref(), &new);
        out.push(Place { name, detail, pick: Pick::from_found(f) });
    }
    out.push(Place { name: "Another folder\u{2026}".into(), detail: "Choose any folder".into(), pick: Pick::Other(None) });
    out
}

/// Starts the installed Notsy on what was chosen.
fn start(app: &Path, pick: &Pick) {
    let _ = std::process::Command::new(app).args(work::launch_args(pick)).current_dir(app.parent().unwrap_or(Path::new("."))).spawn();
}

fn system_dark() -> bool {
    let mut light = 1u32;
    let mut size = 4u32;
    let s = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut light as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    s == ERROR_SUCCESS && light == 0
}

fn class() -> PCWSTR {
    if notsy_core::testing() { w!("Notsy.Test.Setup") } else { w!("Notsy.Setup") }
}

/// Opens the window and runs it until it closes: the exit code, and where
/// it ended.
fn open(job: Job) -> (i32, Stage) {
    let (updating, places, choices) = match &job {
        Job::Install { .. } => match work::installed() {
            Some(i) => (Some(i.version), Vec::new(), i.choices.as_array()),
            None => (None, places(), Choices::FIRST.as_array()),
        },
        Job::Remove => (None, Vec::new(), [false; 3]),
    };
    let stage = if matches!(job, Job::Remove) { Stage::Remove } else { Stage::Welcome };
    let dark = system_dark();
    let Some(hwnd) = create_window(dark) else { return (1, stage) };
    let eng = Eng::new(notsy_text::Engine::new(crate::fonts::font_set(0)));
    let ui = Ui {
        hwnd,
        job,
        eng,
        fonts: Fonts::new(1.0, 0),
        surface: None,
        hits: Vec::new(),
        hover: None,
        tracking: false,
        dark,
        stage,
        updating,
        places,
        chosen: 0,
        choices,
        options: false,
        reveal: (0.0, Instant::now() - REVEAL),
        done: 0.0,
        shown: 0.0,
        message: None,
        installed: None,
    };
    UI.with(|u| *u.borrow_mut() = Some(ui));
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    let end = with_ui(|u| u.stage).unwrap_or(Stage::Failed);
    UI.with(|u| std::mem::forget(u.borrow_mut().take()));
    (if end == Stage::Failed { 1 } else { 0 }, end)
}

fn create_window(dark: bool) -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1 as _)).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: class(),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let dpi = GetDpiForSystem().max(96);
        let scale = dpi as f32 / 96.0;
        let (w, h) = ((screen::SIZE.0 * scale) as i32, (screen::SIZE.1 * scale) as i32);
        let mut area = RECT::default();
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut area as *mut RECT as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let (x, y) = (area.left + (area.right - area.left - w) / 2, area.top + ((area.bottom - area.top - h) / 2).max(0));
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE::default(), class(), w!("Notsy"), WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, x, y, w, h, None, None, Some(instance.into()), None).ok()?;
        let on: i32 = dark as i32;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &on as *const i32 as *const _, 4);
        let round: i32 = 2;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const i32 as *const _, 4);
        // The frame went with NCCALCSIZE: make the window recompute it.
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        Some(hwnd)
    }
}

impl Ui {
    fn scale(&self) -> f32 {
        crate::window::dpi(self.hwnd) as f32 / 96.0
    }

    /// How far Options are out, 0..1.
    fn options_now(&self) -> f32 {
        let (from, since) = self.reveal;
        let to = if self.options { 1.0 } else { 0.0 };
        let k = (since.elapsed().as_secs_f32() / REVEAL.as_secs_f32()).min(1.0);
        from + (to - from) * k
    }

    fn moving(&self) -> bool {
        self.reveal.1.elapsed() < REVEAL || (matches!(self.stage, Stage::Working(_) | Stage::Removing(_)) && self.shown < self.done - 0.001)
    }

    fn tick(&mut self) {
        if self.moving() {
            unsafe {
                SetTimer(Some(self.hwnd), TICK, 15, None);
            }
        }
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// The screen as it is now; its folders go in by the caller (they are
    /// borrowed from `places`).
    fn view(&self) -> (Vec<Folder<'_>>, View<'_>) {
        let folders: Vec<Folder> = self.places.iter().map(|p| Folder { name: &p.name, detail: &p.detail }).collect();
        let stage = match self.stage {
            Stage::Working(_) => Stage::Working(self.shown),
            Stage::Removing(_) => Stage::Removing(self.shown),
            s => s,
        };
        let view = View {
            stage,
            updating: self.updating.as_deref(),
            version: env!("CARGO_PKG_VERSION"),
            options: self.options_now(),
            folders: &[],
            chosen: self.chosen,
            choices: self.choices,
            message: self.message.as_deref(),
            offer_default: self.choices[1],
            hover: self.hover,
        };
        (folders, view)
    }

    fn paint(&mut self) {
        // The fill eases after the work, never ahead of it.
        if self.shown < self.done {
            self.shown = (self.shown + (self.done - self.shown).max(0.02) * 0.18).min(self.done);
        }
        let scale = self.scale();
        let (w, h, dpi) = crate::window::view(self.hwnd);
        let scene = {
            let (folders, mut view) = self.view();
            view.folders = &folders;
            screen::build(&self.eng, &self.fonts, &view, w as f32 / scale, h as f32 / scale)
        };
        let Ok(scene) = scene else { return };
        self.hits = scene.hits.clone();
        let Ok(list) = display::record(&self.eng, &scene, theme::for_mode(self.dark), scale) else { return };
        let surface = self.surface.get_or_insert_with(|| Surface::new((w, h, dpi)));
        surface.draw((w, h, dpi), &list);
        cpu::blit(surface, self.hwnd, None);
        if !self.moving() {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TICK);
            }
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Target> {
        let s = self.scale();
        scene::hit(&self.hits, x as f32 / s, y as f32 / s)
    }

    fn click(&mut self, target: Target) {
        match target {
            Target::Close => self.close(),
            Target::Minimize => unsafe {
                let _ = ShowWindow(self.hwnd, SW_MINIMIZE);
            },
            Target::Setup(Hit::Options) => {
                self.reveal = (self.options_now(), Instant::now());
                self.options = !self.options;
            }
            Target::Setup(Hit::Choice(i)) => self.choices[i] = !self.choices[i],
            Target::Setup(Hit::Folder(i)) => {
                if matches!(self.places.get(i).map(|p| &p.pick), Some(Pick::Other(_))) {
                    let Some(dir) = crate::dialog::pick_folder(self.hwnd, "Where your notes live") else { return self.tick() };
                    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
                    let notes = found::count_notes(&dir, 4);
                    let place = &mut self.places[i];
                    place.detail = format!("{} \u{00B7} {}", work::short(&dir, home.as_deref()), if notes == 1 { "1 note".into() } else { format!("{notes} notes") });
                    place.pick = Pick::Other(Some(dir));
                }
                self.chosen = i;
            }
            Target::Setup(Hit::Primary) => self.primary(),
            Target::Setup(Hit::Secondary) => match self.stage {
                Stage::Done => work::open_default_apps(),
                Stage::Remove => self.close(),
                _ => {}
            },
            _ => {}
        }
        self.tick();
    }

    fn primary(&mut self) {
        match self.stage {
            Stage::Welcome | Stage::Failed => {
                if self.stage == Stage::Failed {
                    self.stage = Stage::Welcome;
                    self.message = None;
                    return;
                }
                let Job::Install { stub } = &self.job else { return };
                let (stub, choices) = (stub.clone(), Choices::from_array(self.choices));
                self.begin(Stage::Working(0.0), move |step| work::install(&stub, choices, step).map(Some));
            }
            Stage::Remove => self.begin(Stage::Removing(0.0), |step| work::uninstall(step).map(|()| None)),
            Stage::Done => {
                if let Some(app) = &self.installed {
                    let pick = self.places.get(self.chosen).map_or(Pick::Other(None), |p| p.pick.clone());
                    start(app, &pick);
                }
                self.close();
            }
            Stage::Removed => self.close(),
            Stage::Working(_) | Stage::Removing(_) => {}
        }
    }

    /// The work on a worker thread; the window hears each step and the end.
    fn begin(&mut self, stage: Stage, work: impl FnOnce(&dyn Fn(f32)) -> Result<Option<PathBuf>, String> + Send + 'static) {
        self.stage = stage;
        self.options = false;
        self.reveal = (self.options_now(), Instant::now());
        self.done = 0.05;
        self.shown = 0.0;
        let hwnd = crate::Handoff(self.hwnd);
        std::thread::spawn(move || {
            let hwnd = hwnd;
            let h = hwnd.0;
            let step = move |p: f32| unsafe {
                let _ = PostMessageW(Some(h), WM_APP_STEP, WPARAM((p * 1000.0) as usize), LPARAM(0));
            };
            let result = Box::into_raw(Box::new(work(&step)));
            if unsafe { PostMessageW(Some(hwnd.0), WM_APP_END, WPARAM(0), LPARAM(result as isize)) }.is_err() {
                drop(unsafe { Box::from_raw(result) });
            }
        });
    }

    fn ended(&mut self, result: Result<Option<PathBuf>, String>) {
        let removing = matches!(self.stage, Stage::Removing(_));
        match result {
            Ok(app) => {
                self.done = 1.0;
                self.shown = 1.0;
                self.installed = app;
                self.stage = if removing { Stage::Removed } else { Stage::Done };
                self.message = Some(self.where_notes_are());
            }
            Err(e) => {
                self.stage = Stage::Failed;
                self.message = Some(e);
            }
        }
    }

    /// What the done screen says about the notes.
    fn where_notes_are(&self) -> String {
        if self.updating.is_some() {
            return "Your notes stay where they are".into();
        }
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
        match self.places.get(self.chosen).map(|p| &p.pick) {
            Some(Pick::New(dir) | Pick::Open(dir) | Pick::Other(Some(dir))) => format!("Your notes are in {}", work::short(dir, home.as_deref())),
            Some(Pick::Import(file)) => format!("{} comes in when Notsy opens", work::short(file, home.as_deref())),
            _ => "Your notes are in Documents\\Notsy".into(),
        }
    }

    fn close(&mut self) {
        if !matches!(self.stage, Stage::Working(_) | Stage::Removing(_)) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

fn point(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32)
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // No frame: the whole window is the client, drawn by the app (DWM
        // keeps its shadow and round corners).
        WM_NCCALCSIZE => return LRESULT(0),
        WM_NCHITTEST => {
            let (sx, sy) = point(lp);
            let mut p = POINT { x: sx, y: sy };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut p);
            }
            let caption = with_ui(|u| u.hit(p.x, p.y) == Some(Target::Caption)).unwrap_or(false);
            return LRESULT(if caption { HTCAPTION } else { HTCLIENT } as isize);
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe {
                BeginPaint(hwnd, &mut ps);
            }
            with_ui(|u| u.paint());
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            return LRESULT(0);
        }
        WM_TIMER => {
            with_ui(|u| u.tick());
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lp);
            with_ui(|u| {
                if !u.tracking {
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: u.hwnd, dwHoverTime: 0 };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                    u.tracking = true;
                }
                let hover = u.hit(x, y).filter(|t| *t != Target::Caption);
                if hover != u.hover {
                    u.hover = hover;
                    u.tick();
                }
            });
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            with_ui(|u| {
                u.tracking = false;
                u.hover = None;
                u.tick();
            });
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lp);
            if let Some(t) = with_ui(|u| u.hit(x, y)).flatten() {
                with_ui(|u| u.click(t));
            }
            return LRESULT(0);
        }
        WM_KEYDOWN => {
            match wp.0 as u16 {
                k if k == VK_RETURN.0 => {
                    with_ui(|u| u.click(Target::Setup(Hit::Primary)));
                }
                k if k == VK_ESCAPE.0 => {
                    with_ui(|u| u.close());
                }
                _ => {}
            }
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let r = unsafe { &*(lp.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            with_ui(|u| {
                u.surface = None;
                u.tick();
            });
            return LRESULT(0);
        }
        WM_APP_STEP => {
            with_ui(|u| {
                u.done = u.done.max(wp.0 as f32 / 1000.0);
                u.tick();
            });
            return LRESULT(0);
        }
        WM_APP_END => {
            let result = unsafe { Box::from_raw(lp.0 as *mut Result<Option<PathBuf>, String>) };
            with_ui(|u| {
                u.ended(*result);
                u.tick();
            });
            return LRESULT(0);
        }
        WM_CLOSE => {
            with_ui(|u| u.close());
            return LRESULT(0);
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}
