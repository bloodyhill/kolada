//! Kolada on iPad and iPhone: the shared app model (`kolada-app`) behind a C
//! interface, for the UIKit shell in `apps/ios` (Swift, kept thin: what the
//! app does lives here and in the shared crates, tested in Rust). The shell
//! makes one `KoladaApp` per window and calls these on the main thread; the
//! `wake` it passes to `kolada_start` may be called from any thread, and asks
//! it to call `kolada_poll` on the main one. Text positions are UTF-16 units,
//! as UIKit counts them; points are DIPs from the view's top left. The C
//! declarations are in `apps/ios/Kolada/kolada.h`.

use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;
use std::time::Instant;

use kolada_app::Model;
use kolada_app::model::settings::Request;
use kolada_app::model::share::{Effect, Export};
use kolada_app::model::transfer::Pick;
use kolada_ui::scene::Panel;

pub mod services;

/// One window's app: the model, and the size it last drew at.
pub struct KoladaApp {
    model: Model,
    size: (u32, u32),
    /// What the model asked the OS to do, for the shell (`kolada_take_event`).
    events: VecDeque<String>,
}

impl KoladaApp {
    /// What the model asks of the OS, as tab-separated events: `url	<address>`,
    /// `copy	<text>`, `rich	<text>	<html>`, `pick	vault|files|folder`,
    /// `paste-new`, `export	markdown|html|pdf|choose	<name>`. What iOS has no use for (the Finder, Terminal, the menu
    /// bar icon) is left out.
    fn queue(&mut self, effects: Vec<Effect>) {
        for e in effects {
            match e {
                Effect::Url(u) => self.events.push_back(format!("url\t{u}")),
                Effect::Clipboard(text, _) => self.events.push_back(format!("copy\t{text}")),
                Effect::Rich { text, html } => self.events.push_back(format!("rich\t{text}\t{html}")),
                Effect::Export { name } => self.events.push_back(format!("export\tchoose\t{name}")),
                _ => {}
            }
        }
        for r in self.model.take_requests() {
            match r {
                Request::Pick(Pick::Vault) => self.events.push_back("pick\tvault".into()),
                Request::Pick(Pick::Files) => self.events.push_back("pick\tfiles".into()),
                Request::Pick(Pick::Folder) => self.events.push_back("pick\tfolder".into()),
                Request::ImportClipboard => self.events.push_back("paste-new".into()),
                Request::Export(kind) => {
                    if let Some((_, name)) = self.model.outgoing() {
                        self.events.push_back(format!("export\t{}\t{name}", kind_name(kind)));
                    }
                }
                _ => {}
            }
        }
    }
}

/// A string argument: UTF-8, or empty.
fn text<'a>(s: *const c_char) -> &'a str {
    if s.is_null() { "" } else { unsafe { CStr::from_ptr(s) }.to_str().unwrap_or("") }
}

fn app<'a>(h: *mut KoladaApp) -> &'a mut KoladaApp {
    unsafe { &mut *h }
}

/// Opens the vault at `vault` (empty: `Documents/Kolada` in the app's
/// container, made on a first run) as the app's own window: the session is
/// restored and kept.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_open(vault: *const c_char, dark: bool) -> *mut KoladaApp {
    let given = Some(text(vault)).filter(|v| !v.is_empty()).map(PathBuf::from);
    // Not made here: a first run's welcome is written into the vault it makes
    // (after the first frame), and a note's first save makes it otherwise.
    let vault = kolada_app::launch::launch_vault(given);
    let mut model = Model::launch(services::ios(), vault, dark);
    model.set_platform(kolada_ui::scene::Platform::Ios);
    Box::into_raw(Box::new(KoladaApp { model, size: (0, 0), events: VecDeque::new() }))
}

/// Writes the note and the session, and frees the app.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_close(h: *mut KoladaApp) {
    if h.is_null() {
        return;
    }
    let mut a = unsafe { Box::from_raw(h) };
    a.model.save_flush();
}

/// A thread-safe handle on the shell's wake call.
struct Wake(extern "C" fn(*mut c_void), usize);

impl Wake {
    fn call(&self) {
        (self.0)(self.1 as *mut c_void)
    }
}

/// Starts the work that waits for the first frame: pictures, the vault's
/// changes from other apps, the search index. `wake(context)` is called
/// from their threads when there is something to show.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_start(h: *mut KoladaApp, wake: extern "C" fn(*mut c_void), context: *mut c_void) {
    let m = &mut app(h).model;
    let ctx = context as usize;
    m.start_pictures(move || Wake(wake, ctx).call());
    m.start_arrivals(move || Wake(wake, ctx).call());
    m.start_watching(move || Wake(wake, ctx).call());
    m.on_index(move || Wake(wake, ctx).call());
    m.start_index();
}

/// Takes in what the threads brought back; whether to draw again.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_poll(h: *mut KoladaApp) -> bool {
    let m = &mut app(h).model;
    let pictures = m.poll_pictures();
    let arrivals = m.poll_arrivals();
    let outside = m.poll_outside();
    let index = m.index_arrived();
    pictures || arrivals || outside || index || m.pictures_fading()
}

/// Draws a frame of `w` x `h` pixels at `scale` pixels a point; the pixels
/// are then at `kolada_pixels`, 0x00RRGGBB in rows from the top
/// (little-endian: B, G, R, unused in memory).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_frame(h: *mut KoladaApp, w: u32, height: u32, scale: f32) {
    let a = app(h);
    a.model.frame(w, height, scale);
    a.size = (w, height);
}

#[unsafe(no_mangle)]
pub extern "C" fn kolada_pixels(h: *mut KoladaApp) -> *const u32 {
    app(h).model.surface().pixels().as_ptr()
}

/// Whether frames should follow at the display's rate (something moves).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_wants_frames(h: *mut KoladaApp) -> bool {
    app(h).model.wants_frames()
}

/// Seconds until something is next due by the clock while nothing moves
/// (a reveal, a fade starting); negative when nothing is.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_next_due(h: *mut KoladaApp) -> f64 {
    match app(h).model.next_due() {
        Some(t) => t.saturating_duration_since(Instant::now()).as_secs_f64(),
        None => -1.0,
    }
}

/// A step of whatever is due; whether to draw.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_tick(h: *mut KoladaApp) -> bool {
    app(h).model.tick()
}

/// The system's appearance (light or dark), for the System theme.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_set_dark(h: *mut KoladaApp, dark: bool) {
    app(h).model.set_dark(dark);
}

#[unsafe(no_mangle)]
pub extern "C" fn kolada_reduce_motion(h: *mut KoladaApp, reduce: bool) {
    app(h).model.reduce_motion(reduce);
}

/// Text typed (or settled by an input method) at the caret.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_insert(h: *mut KoladaApp, s: *const c_char) -> bool {
    app(h).model.commit_marked(text(s))
}

/// A text-system command by AppKit's name (`deleteBackward:`,
/// `moveWordLeftAndModifySelection:`...): the hardware keyboard's keys.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_command(h: *mut KoladaApp, name: *const c_char) -> bool {
    app(h).model.command(text(name))
}

/// An app command by name, for the keyboard shortcuts and menus: `new`,
/// `search`, `palette`, `today`, `past`, `find`, `replace`, `menu`,
/// `toolbar`, `undo`, `redo`, `escape`, `save` (the note written and named
/// for its title, and the session: as the app leaves the front), `autosave`
/// (the note written: a moment after an edit), `refresh` (the vault read
/// again).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_do(h: *mut KoladaApp, what: *const c_char) -> bool {
    let m = &mut app(h).model;
    match text(what) {
        "new" => m.new_note(),
        "search" => m.open_search(),
        "palette" => m.open_palette(),
        "today" => m.open_today(),
        "past" => m.toggle_scrub(),
        "find" => m.open_find(false),
        "replace" => m.open_find(true),
        "menu" => m.toggle_panel(Panel::Sidebar),
        "reveal-menu" => m.reveal_side_menu(),
        "toolbar" => m.toggle_panel(Panel::Toolbar),
        "undo" => m.undo(),
        "redo" => m.redo(),
        "escape" => m.escape(),
        "save" => m.save_flush(),
        // A moment after an edit, as the Mac does: the note only, no rename.
        "autosave" => m.save(),
        // The vault changed with no watcher to say so (iCloud downloads).
        "refresh" => m.refresh(),
        _ => false,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kolada_pointer_down(h: *mut KoladaApp, x: f32, y: f32, clicks: u32, shift: bool) -> bool {
    app(h).model.pointer_down(x, y, clicks, shift)
}

#[unsafe(no_mangle)]
pub extern "C" fn kolada_pointer_dragged(h: *mut KoladaApp, x: f32, y: f32) -> bool {
    app(h).model.pointer_dragged(x, y)
}

/// The press ends at (x, y). What it asks the OS to do (open a link...) is
/// not carried out yet on iOS.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_pointer_up(h: *mut KoladaApp, x: f32, y: f32) -> bool {
    let a = app(h);
    let (changed, effects) = a.model.pointer_up_at(x, y);
    a.queue(effects);
    changed
}

/// A finger's tap (see `Model::tap`: a revealed side menu stays while used).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_tap(h: *mut KoladaApp, x: f32, y: f32, clicks: u32) -> bool {
    let a = app(h);
    let (changed, effects) = a.model.tap(x, y, clicks);
    a.queue(effects);
    changed
}

/// The next thing the model asks the OS to do (see `KoladaApp::queue`), as a
/// C string to free with `kolada_free_string`; null when there is none. The
/// shell drains these after each call that may ask (clicks, commands).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_take_event(h: *mut KoladaApp) -> *mut c_char {
    let a = app(h);
    let effects = a.model.take_effects();
    a.queue(effects);
    match a.events.pop_front() {
        Some(e) => CString::new(e.replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw),
        None => std::ptr::null_mut(),
    }
}

fn kind_name(kind: Export) -> &'static str {
    match kind {
        Export::Markdown => "markdown",
        Export::Html => "html",
        Export::Pdf => "pdf",
    }
}

/// Paths, one a line.
fn paths(list: *const c_char) -> Vec<PathBuf> {
    text(list).lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

/// Notes and documents brought in (Markdown, text, DOCX, PDF, HTML, other
/// apps' exports), their paths one a line.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_import_files(h: *mut KoladaApp, list: *const c_char) -> bool {
    app(h).model.import_files(paths(list))
}

/// A folder of notes copied into the vault.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_import_folder(h: *mut KoladaApp, folder: *const c_char) -> bool {
    app(h).model.import_folder(std::path::Path::new(text(folder)))
}

/// The pasteboard's text as a new note.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_import_clipboard(h: *mut KoladaApp, s: *const c_char) -> bool {
    app(h).model.import_clipboard(Some(text(s).to_string()))
}

/// Another folder as the vault (one the shell holds access to).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_switch_vault(h: *mut KoladaApp, folder: *const c_char) -> bool {
    app(h).model.switch_vault(PathBuf::from(text(folder)))
}

/// The open note exported as `kind` (`markdown`, `html`, or `pdf`, which
/// is the web page for the shell to print): the file's text, a C string to
/// free; null when there is nothing to export.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_export(h: *mut KoladaApp, kind: *const c_char) -> *mut c_char {
    let kind = match text(kind) {
        "markdown" => Export::Markdown,
        "pdf" => Export::Pdf,
        _ => Export::Html,
    };
    match app(h).model.export_page(kind) {
        Some(page) => CString::new(page.replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw),
        None => std::ptr::null_mut(),
    }
}

/// The selected text (for Copy and Cut), as a C string to free.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_selected_text(h: *mut KoladaApp) -> *mut c_char {
    CString::new(app(h).model.selected_text().replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Paste: `plain` is the pasteboard's text, `html` its HTML or null; HTML
/// comes in as Markdown (headings, lists, links, tables, pictures).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_paste(h: *mut KoladaApp, plain: *const c_char, html: *const c_char) -> bool {
    let page = (!html.is_null()).then(|| text(html).to_string()).filter(|p| !p.is_empty());
    app(h).model.paste(text(plain), || page.map(|p| (p, None)))
}

/// Paste the text alone, its formatting left behind.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_paste_plain(h: *mut KoladaApp, plain: *const c_char) -> bool {
    app(h).model.paste_plain(text(plain))
}

/// The pointer (a trackpad's or a mouse's on iPad) hovers at (x, y).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_pointer_moved(h: *mut KoladaApp, x: f32, y: f32) -> bool {
    app(h).model.pointer_moved(x, y)
}

#[unsafe(no_mangle)]
pub extern "C" fn kolada_pointer_left(h: *mut KoladaApp) -> bool {
    app(h).model.pointer_left()
}

/// Scrolls by `dy` points (positive: further down the note), at once.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_scroll(h: *mut KoladaApp, dy: f32) -> bool {
    app(h).model.wheel(dy, true)
}

/// The selection, UTF-16 start and length.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_selection(h: *mut KoladaApp, location: *mut usize, length: *mut usize) {
    let (l, n) = app(h).model.selection_utf16();
    unsafe {
        *location = l;
        *length = n;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kolada_select(h: *mut KoladaApp, location: usize, length: usize) {
    app(h).model.select_utf16(location, length);
}

/// The text being composed by an input method, if any: false when none.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_marked(h: *mut KoladaApp, location: *mut usize, length: *mut usize) -> bool {
    match app(h).model.marked_utf16() {
        Some((l, n)) => {
            unsafe {
                *location = l;
                *length = n;
            }
            true
        }
        None => false,
    }
}

/// An input method's text in progress, with its selection inside it.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_set_marked(h: *mut KoladaApp, s: *const c_char, cursor: usize, converting: usize) -> bool {
    app(h).model.set_marked_converting(text(s), cursor, converting)
}

#[unsafe(no_mangle)]
pub extern "C" fn kolada_unmark(h: *mut KoladaApp) -> bool {
    app(h).model.unmark()
}

/// The text's length, UTF-16 units.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_length(h: *mut KoladaApp) -> usize {
    app(h).model.access_len()
}

/// The text between two UTF-16 positions, as a C string to free with
/// `kolada_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_substring(h: *mut KoladaApp, location: usize, length: usize) -> *mut c_char {
    let m = &app(h).model;
    let end = (location + length).min(m.access_len());
    let s = m.substring_utf16(location.min(end), end - location.min(end));
    CString::new(s.replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw)
}

#[unsafe(no_mangle)]
pub extern "C" fn kolada_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// The caret's box, points (left, top, right, bottom): false when it is
/// not drawn.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_caret(h: *mut KoladaApp, rect: *mut f32) -> bool {
    let Some(r) = app(h).model.caret_rect() else { return false };
    unsafe { std::ptr::copy_nonoverlapping([r.left, r.top, r.right, r.bottom].as_ptr(), rect, 4) };
    true
}

/// Where the range is drawn, points (left, top, right, bottom): the caret's
/// box for an empty range at the caret, else across the paragraphs it is in;
/// false when none of it is in view.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_rect(h: *mut KoladaApp, location: usize, length: usize, rect: *mut f32) -> bool {
    let Some(r) = app(h).model.access_rect(location, length) else { return false };
    unsafe { std::ptr::copy_nonoverlapping([r.left, r.top, r.right, r.bottom].as_ptr(), rect, 4) };
    true
}

/// For VoiceOver: the line (from 0) that holds UTF-16 position `at`.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_line_of(h: *mut KoladaApp, at: usize) -> usize {
    app(h).model.access_line_of(at)
}

/// For VoiceOver: line `line`'s UTF-16 start and length; false past the last.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_line_range(h: *mut KoladaApp, line: usize, location: *mut usize, length: *mut usize) -> bool {
    let Some((l, n)) = app(h).model.access_line_range(line) else { return false };
    unsafe {
        *location = l;
        *length = n;
    }
    true
}

/// For VoiceOver: what the view shows of the note, UTF-16 start and length.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_visible(h: *mut KoladaApp, location: *mut usize, length: *mut usize) {
    let (l, n) = app(h).model.access_visible();
    unsafe {
        *location = l;
        *length = n;
    }
}

/// What VoiceOver calls the note: its name.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_label(h: *mut KoladaApp) -> *mut c_char {
    CString::new(app(h).model.access_label().replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// The UTF-16 position nearest (x, y).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_index_at(h: *mut KoladaApp, x: f32, y: f32) -> usize {
    app(h).model.index_at(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C interface end to end, as the Swift shell drives it: open a
    /// vault, draw, type, move, compose, read back, close.
    #[test]
    fn the_shell_opens_draws_types_and_reads_the_note_through_the_c_interface() {
        // `kolada_open` is the app's launch, which keeps its session: in a
        // home of the test's own, so neither the user's data nor a test
        // instance's (which a Mac being driven by hand uses) is touched.
        let home = std::env::temp_dir().join(format!("kolada-ios-test-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        unsafe {
            std::env::set_var("KOLADA_TEST_INSTANCE", "1");
            std::env::set_var("HOME", &home);
        }
        let dir = std::env::temp_dir().join(format!("kolada-ios-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), "# Plan\n").unwrap();
        let path = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        let h = kolada_open(path.as_ptr(), false);
        kolada_frame(h, 1640, 2360, 2.0);
        let pixels = unsafe { std::slice::from_raw_parts(kolada_pixels(h), 1640 * 2360) };
        assert!(pixels.iter().filter(|&&p| p != pixels[0]).count() > 10_000, "a frame with something on it");
        kolada_command(h, c"moveToEndOfDocument:".as_ptr());
        assert!(kolada_insert(h, c"Ship it".as_ptr()));
        kolada_command(h, c"moveWordLeftAndModifySelection:".as_ptr());
        let (mut l, mut n) = (0, 0);
        kolada_selection(h, &mut l, &mut n);
        assert_eq!((l, n), (12, 2), "the word just typed is selected");
        let s = kolada_substring(h, l, n);
        assert_eq!(unsafe { CStr::from_ptr(s) }.to_str().unwrap(), "it");
        kolada_free_string(s);
        // A Japanese input method composes, then settles.
        assert!(kolada_set_marked(h, c"\u{306B}\u{307B}".as_ptr(), 2, 0));
        assert!(kolada_marked(h, &mut l, &mut n));
        assert_eq!((l, n), (12, 2));
        assert!(kolada_insert(h, c"\u{65E5}\u{672C}".as_ptr()));
        assert!(!kolada_marked(h, &mut l, &mut n));
        assert_eq!(kolada_length(h), 14);
        kolada_frame(h, 1640, 2360, 2.0);
        let mut caret = [0.0f32; 4];
        assert!(kolada_caret(h, caret.as_mut_ptr()), "the caret is drawn");
        let mut at = [0.0f32; 4];
        assert!(kolada_rect(h, 14, 0, at.as_mut_ptr()));
        assert_eq!(at, caret, "an empty range at the caret is the caret's box");
        assert!(kolada_rect(h, 0, 6, at.as_mut_ptr()) && at[2] > at[0] + 100.0, "the title's line spans the column: {at:?}");
        assert!(kolada_do(h, c"undo".as_ptr()));
        // Copy, and paste HTML as Markdown.
        kolada_command(h, c"selectAll:".as_ptr());
        let copied = kolada_selected_text(h);
        assert!(unsafe { CStr::from_ptr(copied) }.to_str().unwrap().starts_with("# Plan"));
        kolada_free_string(copied);
        kolada_command(h, c"moveToEndOfDocument:".as_ptr());
        kolada_command(h, c"insertNewline:".as_ptr());
        assert!(kolada_paste(h, c"Big".as_ptr(), c"<h2>Big</h2>".as_ptr()));
        let all = kolada_substring(h, 0, kolada_length(h));
        assert!(unsafe { CStr::from_ptr(all) }.to_str().unwrap().contains("## Big"), "the HTML came in as a heading");
        kolada_free_string(all);
        assert!(kolada_take_event(h).is_null(), "nothing asked of the OS");
        // Export the note, import a file.
        let md = kolada_export(h, c"markdown".as_ptr());
        assert!(unsafe { CStr::from_ptr(md) }.to_str().unwrap().contains("## Big"));
        kolada_free_string(md);
        let page = kolada_export(h, c"html".as_ptr());
        assert!(unsafe { CStr::from_ptr(page) }.to_str().unwrap().contains("<h2"), "a web page of the note");
        kolada_free_string(page);
        // VoiceOver reads it by lines.
        assert_eq!(kolada_line_of(h, 0), 0);
        assert!(kolada_line_range(h, 0, &mut l, &mut n) && (l, n) == (0, 7), "the title's line, its newline included: {l} {n}");
        assert!(!kolada_line_range(h, 99, &mut l, &mut n));
        kolada_visible(h, &mut l, &mut n);
        assert_eq!(l, 0);
        let label = kolada_label(h);
        assert_eq!(unsafe { CStr::from_ptr(label) }.to_str().unwrap(), "Plan");
        kolada_free_string(label);
        // A file brought in (it opens, as the first imported note does).
        let outside = std::env::temp_dir().join(format!("kolada-ios-test-{}-in.md", std::process::id()));
        std::fs::write(&outside, "# Brought in\n").unwrap();
        let list = CString::new(outside.to_string_lossy().as_bytes()).unwrap();
        let notes = |d: &std::path::Path| std::fs::read_dir(d).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".md")).count();
        let before = notes(&dir);
        assert!(kolada_import_files(h, list.as_ptr()));
        assert_eq!(notes(&dir), before + 1, "the file came into the vault");
        let _ = std::fs::remove_file(outside);
        kolada_close(h);
        assert!(home.join("Library/Application Support/Kolada.test/session.bin").is_file(), "the session went to the test's own home");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(home);
    }
}
