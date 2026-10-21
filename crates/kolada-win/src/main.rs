//! Kolada for Windows.
//!
//! The startup path is the product. Before the first frame this process
//! reads one small session snapshot and one note, never the whole vault:
//!
//! 1. claim the single instance (or hand the file to the running one)
//! 2. read the session snapshot, which says where the window goes and
//!    whether to resume the last note or open on a fresh page
//! 3. in parallel: a worker reads the note, lays out the visible lines and
//!    draws them on the CPU (several threads, one band of the window each)
//!    into off-screen bitmaps, while this thread
//!    creates the window (cloaked, no open animation, not activated)
//! 4. join, blit, uncloak; DWM shows the frame at its next composition
//! 5. activate the window (slow the first time on a thread, so it waits)
//!
//! Everything else (listing the vault, theme check, ...) runs after that.
//! No GPU driver is loaded at startup: on the benchmark laptop creating a
//! D3D11 device alone costs 70-90 ms, while this whole path takes ~30 ms.
//!
//! Usage: `kolada [note.md] [--vault <dir>] [--window [--at x,y]] [--new]
//! [--search] [--quit] [--trace] [--exit-after-first-frame]
//! [--dump-frame <file.bmp>] [--no-activate] [--updated] [--hidden]`. `--new` and
//! `--search` go to the running Kolada (the taskbar menu's items); `--quit`
//! ends it, tray or not, with everything saved. `--updated` is how an
//! update restarts Kolada (see `updates`): it waits for the old process to
//! go and reopens the note as it was left. `--hidden` is how Windows starts
//! it at sign-in (the installer's "Start with Windows"): ready, not shown.
//! `--setup` and `--uninstall` install and remove Kolada (see `setup`).
//! `--window` opens the note in a window of its own (at a screen point): a
//! process of its own too, beside the main one. The last two are for
//! automated tests: the frame at start and at close, and a window that
//! shows without taking the keyboard from the user.

#![windows_subsystem = "windows"]

mod app;
mod assist;
mod clipboard;
mod convert;
mod cpu;
mod dialog;
mod drag;
mod ime;
mod fonts;
mod gists;
mod http;
mod images;
mod instance;
mod jump;
mod spell;
mod setup;
mod share;
mod trace;
#[cfg(test)]
mod text_parity;
#[cfg(test)]
mod args_tests {
    use super::parse;
    use std::ffi::OsString;

    #[test]
    fn a_folder_on_the_command_line_opens_as_the_vault() {
        let dir = std::env::temp_dir().join(format!("kolada-args-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let note = dir.join("Plan.md");
        std::fs::write(&note, "# Plan").unwrap();
        let args = parse([OsString::from(&dir)].into_iter());
        assert_eq!(args.vault.as_deref(), Some(dir.as_path()), "the folder is the vault");
        assert!(args.file.is_none(), "not a note in its parent");
        let args = parse([OsString::from(&note)].into_iter());
        assert_eq!(args.file.as_deref(), Some(note.as_path()), "a note is still a note");
        assert!(args.vault.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
mod updates;
use kolada_ui::{chrome, display, note, scene, social, theme, wave};
mod watch;
mod when;
mod window;

use std::ffi::c_void;
use std::io::Write as _;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;

use kolada_core::session::{self, Session};
use kolada_core::vault;
use windows::Win32::Foundation::{E_FAIL, ERROR_SUCCESS, HWND, LPARAM, WPARAM};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PM_REMOVE, PeekMessageW, PostMessageW, TranslateMessage, WM_QUIT,
};
use windows::core::{Error, Result, w};

use app::{App, Ui};
use cpu::Surface;
use scene::{Eng, Fonts, Rect, Scene, Target};
use window::{WM_APP_SIDEBAR, WM_APP_THEME};

/// Moves a value to another thread. Used for COM objects and handles that
/// are safe to use from any thread as long as one thread uses them at a
/// time (DirectWrite, GDI bitmaps, window handles).
pub struct Handoff<T>(pub T);
unsafe impl<T> Send for Handoff<T> {}

#[derive(Default)]
struct Args {
    file: Option<PathBuf>,
    /// Another app's export (see `app::is_export`) or a document (see
    /// `convert::is_document`) to import, absolute.
    import: Option<PathBuf>,
    vault: Option<PathBuf>,
    trace: bool,
    exit_after_first_frame: bool,
    dump_frame: Option<PathBuf>,
    no_activate: bool,
    /// The note in a window of its own, and where on screen.
    window: bool,
    at: Option<(i32, i32)>,
    /// From the taskbar's menu: a fresh page, or search.
    new: bool,
    search: bool,
    quit: bool,
    /// Started by an update in the old process's place.
    updated: bool,
    /// Started with Windows: ready beside the clock, the window not shown
    /// until it is asked for.
    hidden: bool,
}

fn parse_args() -> Args {
    parse(std::env::args_os().skip(1))
}

fn parse(mut it: impl Iterator<Item = std::ffi::OsString>) -> Args {
    let mut args = Args::default();
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--trace") => args.trace = true,
            Some("--exit-after-first-frame") => args.exit_after_first_frame = true,
            Some("--vault") => args.vault = it.next().map(PathBuf::from),
            Some("--dump-frame") => args.dump_frame = it.next().map(PathBuf::from),
            Some("--no-activate") => args.no_activate = true,
            Some("--window") => args.window = true,
            Some("--new") => args.new = true,
            Some("--search") => args.search = true,
            Some("--quit") => args.quit = true,
            Some("--updated") => args.updated = true,
            Some("--hidden") => args.hidden = true,
            Some("--at") => {
                args.at = it.next().and_then(|v| {
                    let (x, y) = v.to_str()?.split_once(',')?;
                    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
                })
            }
            _ => {
                let path = PathBuf::from(a);
                if app::is_export(&path) || convert::is_document(&path) {
                    args.import = Some(std::path::absolute(&path).unwrap_or(path));
                } else if path.is_dir() {
                    // A folder is a vault, never a note in its parent: its
                    // name would be taken for a note's and renamed with it.
                    args.vault = Some(path);
                } else {
                    args.file = Some(path);
                }
            }
        }
    }
    args
}

fn main() {
    // A converting process (see `convert`): no window, no session.
    let raw: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if raw.first().is_some_and(|a| a == convert::FLAG) {
        std::process::exit(convert::child(&raw[1..]));
    }
    // A paste's copied HTML into Markdown (see `convert::paste`).
    if raw.first().is_some_and(|a| a == convert::PASTE_FLAG) {
        std::process::exit(convert::paste_child(&raw[1..]));
    }
    // Installing (from Kolada-Setup.exe) or removing (from Settings > Apps):
    // a window of their own, no session, no instance (see `setup`).
    if raw.first().is_some_and(|a| a == "--setup") {
        std::process::exit(setup::install_main(&raw[1..]));
    }
    if raw.first().is_some_and(|a| a == "--uninstall") {
        std::process::exit(setup::uninstall_main(&raw[1..]));
    }
    trace::mark("main");
    let args = parse_args();
    let result = run(&args);
    if args.trace {
        write_trace(result.err());
    }
}

fn write_trace(error: Option<Error>) {
    let Some(dir) = kolada_core::app_data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let file = std::fs::OpenOptions::new().append(true).create(true).open(dir.join("startup-trace.log"));
    if let Ok(mut f) = file {
        let error = error.map(|e| format!(" error={e}")).unwrap_or_default();
        let _ = writeln!(f, "{}{error}", trace::report());
    }
}

/// Points the session at the note or vault given on the command line.
fn apply_args(session: &mut Session, args: &Args) {
    let absolute = |p: &PathBuf| std::path::absolute(p).unwrap_or_else(|_| p.clone());
    let (vault, note) = if let Some(file) = &args.file {
        let file = absolute(file);
        let root = vault::root_for(&file);
        let Some(rel) = vault::relative(&root, &file) else { return };
        (root, rel)
    } else if let Some(dir) = &args.vault {
        let dir = absolute(dir);
        let note = vault::first_note(&dir).unwrap_or_default();
        (dir, note)
    } else {
        return;
    };
    let vault = vault.to_string_lossy().into_owned();
    // The saved file tree is still right if the vault did not change.
    let sidebar = if session.vault == vault { std::mem::take(&mut session.sidebar) } else { Vec::new() };
    // Every setting stays; where the last note was left does not apply.
    let old = std::mem::take(session);
    *session = Session { vault, note, sidebar, last_active: session::now(), scroll_line: 0, scroll_px: 0.0, caret: 0, captured: false, ..old };
}

/// The first frame, drawn off-screen on a worker thread.
struct FirstFrame {
    eng: Eng,
    fonts: Fonts,
    note: app::Opened,
    /// Where the caret starts.
    caret: u64,
    surface: Surface,
    hits: Vec<(Rect, Target)>,
    column: Rect,
    images: std::cell::RefCell<images::Images>,
}

/// The first frame's scene: the session's note with the caret where it was.
fn first_scene(
    eng: &Eng,
    fonts: &Fonts,
    session: &Session,
    text: &str,
    view: (u32, u32, u32),
    compact: bool,
    images: &std::cell::RefCell<images::Images>,
) -> Result<Scene> {
    let caret = session.caret as usize;
    let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(text).collect();
    let live = app::Live { compact, images: Some(images), ..app::Live::none() };
    let mut scene = app::build_scene(eng, fonts, session, text, &lines, (caret, caret), view, &Ui::resting(session, view.0 as f32 * 96.0 / view.2 as f32), live)?;
    scene.place_caret((0.0, 0.0));
    Ok(scene)
}

/// With `from_file` (a note given to open), the caret starts under the
/// note's title, where writing goes, rather than where the session had it.
fn draw_first_frame(session: &Session, view: (u32, u32, u32), compact: bool, from_file: bool) -> Result<FirstFrame> {
    // The fonts are listed, not searched for: loading them takes a few
    // milliseconds, beside the window being made.
    let set = fonts::font_set(session.font);
    trace::mark("fontset");
    let eng = Eng::new(kolada_text::Engine::new(set));
    trace::mark("engine");
    let theme = theme::for_mode(session.dark);
    // Band threads allocate and clear their bitmaps while the scene is built.
    let pending = cpu::start(view, theme.background);
    let note = app::read_note(session);
    let fonts = Fonts::new(scene::text_scale(session.text_size), session.font);
    let mut session = session.clone();
    if from_file {
        let lines: Vec<kolada_markdown::Line> = kolada_markdown::lines(&note.0).collect();
        session.caret = app::body_start_of(&note.0, &lines) as u64;
    }
    let session = &session;
    let images = std::cell::RefCell::new(images::Images::new());
    let mut scene = first_scene(&eng, &fonts, session, &note.0, view, compact, &images)?;
    let hits = std::mem::take(&mut scene.hits);
    trace::mark("scene");
    let list = display::record(&eng, &scene, theme, view.2 as f32 / 96.0)?;
    trace::mark("record");
    let surface = pending.finish(list)?;
    trace::mark("draw");
    Ok(FirstFrame { eng, fonts, note, caret: session.caret, surface, hits, column: scene.column, images })
}

fn run(args: &Args) -> Result<()> {
    app::NO_ACTIVATE.store(args.no_activate, std::sync::atomic::Ordering::Relaxed);
    // A note in its own window runs beside the main instance.
    let note_window = args.window && args.file.is_some();
    let _instance = if note_window {
        None
    } else if args.quit {
        // Quit the one running, if one is; never start one to quit.
        if instance::claim().is_none() {
            instance::forward_command("quit");
        }
        return Ok(());
    } else {
        // Just updated: the old process is still closing, holding the instance.
        let claimed = if args.updated { instance::claim_within(std::time::Duration::from_secs(10)) } else { instance::claim() };
        let Some(instance) = claimed else {
            // Started with Windows while Kolada already runs: nothing to do.
            if args.hidden {
                return Ok(());
            }
            match (args.new, args.search) {
                _ if args.quit => instance::forward_command("quit"),
                (true, _) => instance::forward_command("new"),
                (_, true) => instance::forward_command("search"),
                _ => instance::forward(args.file.as_deref().or(args.import.as_deref())),
            }
            return Ok(());
        };
        Some(instance)
    };
    trace::mark("instance");
    // An update downloaded last time and never clicked goes in now: two
    // renames, then the new program starts in this one's place.
    if let Some(instance) = &_instance
        && let Some(exe) = updates::install_waiting()
    {
        instance.release();
        updates::start(&exe, std::env::args_os().skip(1));
        return Ok(());
    }
    trace::mark("updates");
    // Just updated from Notsy, still in its install's folder: the program
    // moves house and starts again from there.
    if let Some(instance) = &_instance
        && let Some(exe) = setup::work::rehome()
    {
        instance.release();
        let mut again: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
        if !again.iter().any(|a| a == "--updated") {
            again.push("--updated".into());
        }
        updates::start(&exe, again);
        return Ok(());
    }

    kolada_core::adopt_old_data();
    trace::mark("adopted");
    let mut session_path = kolada_core::app_data_dir().map(|d| d.join("session.bin"));
    let bytes = session_path.as_deref().and_then(|p| std::fs::read(p).ok());
    trace::mark("read");
    let mut session = bytes.as_deref().and_then(session::decode).unwrap_or_else(Session::first_run);
    trace::mark("loaded");
    // Never run before, and nothing given to open: the welcome note, after
    // the first frame.
    let first_run = bytes.is_none() && !note_window && args.file.is_none() && args.import.is_none() && args.vault.is_none();
    apply_args(&mut session, args);
    if note_window {
        // The note alone: no side menu or margin, the toolbar tucked away,
        // and nothing saved over the main window's session.
        session.sidebar_collapsed = true;
        session.toolbar_collapsed = true;
        session.related = 1;
        session.sidebar.clear();
        session.window = window::note_placement(args.at);
        session.minimal = false;
        session_path = None;
    } else {
        // Smart resume: the last note if it was in use within the hour, else
        // a fresh page to type on.
        if args.new || (args.file.is_none() && !args.updated && !session.resumes(session::now())) {
            session.start_fresh();
        }
        session.window = window::placement(&session.window);
        trace::mark("placed");
        // As the options say: the full window, or the minimal one.
        session.minimal = session.window_mode == 1;
        if session.minimal {
            session.minimal_window = window::minimal_placement(&session.minimal_window, &session.window);
        }
    }
    // Where the window opens: the minimal one's place, or the full one's.
    let shown = if session.minimal { session.minimal_window } else { session.window };
    // The note alone: a note's own window, or the minimal one.
    let compact = note_window || session.minimal;
    trace::mark("session");
    let class = if note_window { window::note_class() } else { window::main_class() };

    // Draw the first frame on a worker while this thread creates the window.
    let predicted = window::predicted_view(&shown);
    let worker = {
        let session = session.clone();
        let from_file = args.file.is_some();
        thread::spawn(move || draw_first_frame(&session, predicted, compact, from_file).map(Handoff))
    };
    window::register(class, Some(app::wndproc))?;
    let hwnd = window::create(class, &shown, session.dark)?;
    if session.minimal {
        window::keep_on_top(hwnd, true);
    }
    trace::mark("window");
    let FirstFrame { eng, fonts, note, caret, mut surface, mut hits, mut column, images } =
        worker.join().map_err(|_| Error::from_hresult(E_FAIL))??.0;
    session.caret = caret;
    trace::mark("joined");

    // The window came out a different size than predicted (e.g. it is
    // maximized, or on a monitor with another DPI): draw again at the real size.
    let actual = window::view(hwnd);
    if actual != predicted {
        trace::info("redrawn", &format!("{predicted:?} -> {actual:?}"));
        let mut scene = first_scene(&eng, &fonts, &session, &note.0, actual, compact, &images)?;
        hits = std::mem::take(&mut scene.hits);
        column = scene.column;
        let list = display::record(&eng, &scene, theme::for_mode(session.dark), actual.2 as f32 / 96.0)?;
        surface.draw(actual, &list);
    }
    cpu::blit(&surface, hwnd, None);
    // The copy went through GetDC, outside WM_PAINT, so the window still
    // has a paint pending. Marking it painted made the first frame reach
    // the screen on time in 24 of 30 runs instead of 20.
    // TODO(M1): find what DWM waits for in the rest.
    window::validate(hwnd);
    if args.hidden {
        // Drawn, and waiting: the tray icon or another launch shows it.
        window::hide(hwnd);
    } else {
        window::uncloak(hwnd);
    }
    trace::mark("visible");
    // Activation initializes text input for the thread (~25 ms), so it
    // happens only once the first frame is on its way to the screen.
    if !args.no_activate && !args.hidden {
        window::activate(hwnd);
    }
    trace::mark("active");
    window::set_icon(hwnd);
    trace::mark("icon");

    if let Some(path) = &args.dump_frame {
        let _ = surface.dump(path);
    }
    let vault = session.vault.clone();
    let mut expanded = vault::expanded_folders(&session.sidebar);
    expanded.extend(session.expanded.iter().cloned());
    app::install(App::new(hwnd, surface, eng, fonts, session, note, session_path, hits, column, note_window, images.into_inner()));
    if args.exit_after_first_frame {
        return Ok(());
    }

    // After the first frame.
    if !note_window {
        refresh_sidebar(hwnd, vault.clone(), expanded);
        app::start_tray();
        app::start_hotkey();
        window::accept_drops(hwnd);
        jump::build(vault);
        if first_run {
            app::start_welcome();
        }
        if args.search {
            app::search_now();
        }
        if let Some(export) = &args.import {
            app::import_now(export);
        }
        app::start_updates(args.updated);
    }
    check_theme(hwnd);
    app::start_index();
    app::start_watch();
    app::start_spell();
    // The writing model, a moment later still: the main window starts its
    // process then, a note's own window once it is written in.
    app::start_assist();
    // A running app writes its last frame too, when it closes.
    app::dump_on_close(args.dump_frame.clone());
    message_loop();
    app::leave();
    // The update's pill was clicked: the new program is in place.
    if updates::RELAUNCH.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(instance) = &_instance {
            instance.release();
        }
        if let Ok(exe) = std::env::current_exe() {
            updates::start(&exe, [std::ffi::OsString::from("--updated")]);
        }
    }
    Ok(())
}

/// Waits for messages. When a frame is due (something changed, or a panel
/// is animating), handles every pending message first and then draws once,
/// paced by DWM: at most one frame per display refresh.
fn message_loop() {
    let mut msg = MSG::default();
    loop {
        if app::needs_frame() {
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                if msg.message == WM_QUIT {
                    return;
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            app::frame();
            unsafe {
                let _ = DwmFlush();
            }
        } else {
            if !unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                return;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

/// Moves a file to the Recycle Bin, where it can be restored. Shell32 is
/// delay-loaded: it loads only the first time this runs, never at startup.
pub fn recycle(hwnd: HWND, path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW,
    };
    // A list of paths, each ended by a nul, the list by another.
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut op = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_DELETE,
        pFrom: windows::core::PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT).0 as u16,
        ..Default::default()
    };
    unsafe { SHFileOperationW(&mut op) == 0 && !op.fAnyOperationsAborted.as_bool() }
}

/// Has the window say `text` quietly (a worker finished something).
pub fn post_whisper(hwnd: HWND, text: String) {
    let text = Box::into_raw(Box::new(text));
    if unsafe { PostMessageW(Some(hwnd), window::WM_APP_WHISPER, WPARAM(0), LPARAM(text as isize)) }.is_err() {
        drop(unsafe { Box::from_raw(text) });
    }
}

/// Lowers the calling worker's CPU and I/O priority, for work the user is
/// not waiting on.
pub fn background_priority() {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

/// Lists the vault on a worker, keeping `expanded` folders open, and posts
/// the rows to the window.
pub fn refresh_sidebar(hwnd: HWND, vault: String, expanded: HashSet<String>) {
    if vault.is_empty() {
        return;
    }
    let hwnd = Handoff(hwnd);
    thread::spawn(move || {
        let hwnd = hwnd;
        let Ok(rows) = vault::tree_rows(Path::new(&vault), &expanded) else { return };
        let rows = Box::into_raw(Box::new(rows));
        let posted = unsafe { PostMessageW(Some(hwnd.0), WM_APP_SIDEBAR, WPARAM(0), LPARAM(rows as isize)) };
        if posted.is_err() {
            drop(unsafe { Box::from_raw(rows) });
        }
    });
}

/// Reads the system light/dark setting after the first frame; the first
/// frame uses the setting saved in the snapshot.
fn check_theme(hwnd: HWND) {
    let hwnd = Handoff(hwnd);
    thread::spawn(move || {
        let hwnd = hwnd;
        let mut light = 1u32;
        let mut size = size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                w!("AppsUseLightTheme"),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut light as *mut u32 as *mut c_void),
                Some(&mut size),
            )
        };
        let dark = status == ERROR_SUCCESS && light == 0;
        let _ = unsafe { PostMessageW(Some(hwnd.0), WM_APP_THEME, WPARAM(dark as usize), LPARAM(0)) };
    });
}
