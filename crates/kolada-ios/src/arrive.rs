//! Files coming in, the iPad way: dragged in from the Files app or another
//! app in Split View (the card says what each becomes as they come over,
//! as on the Mac, and pictures land in the gap it opens), and files opened
//! with Kolada from the share sheet or the Files app (copied in: a note
//! from elsewhere becomes a note of this vault, a document or another
//! app's export is imported; one of this vault's own just opens).

use std::ffi::c_char;
use std::path::PathBuf;

use crate::{KoladaApp, app, paths, text};

/// Files dragged in at (x, y) DIPs, named one a line (iOS gives their
/// names before their contents): whether any can be taken.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_drag_enter(h: *mut KoladaApp, names: *const c_char, x: f32, y: f32) -> bool {
    app(h).model.drag_enter(&paths(names), x, y)
}

#[unsafe(no_mangle)]
pub extern "C" fn kolada_drag_over(h: *mut KoladaApp, x: f32, y: f32) -> bool {
    app(h).model.drag_over(x, y)
}

#[unsafe(no_mangle)]
pub extern "C" fn kolada_drag_leave(h: *mut KoladaApp) -> bool {
    app(h).model.drag_leave()
}

/// What the card over the note says while files are dragged in: its two
/// lines, tab-separated (a C string to free); null when it is not up.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_drag_card(h: *mut KoladaApp) -> *mut c_char {
    match app(h).model.drag_card() {
        Some((title, change)) => std::ffi::CString::new(format!("{title}\t{change}").replace('\0', "")).map_or(std::ptr::null_mut(), std::ffi::CString::into_raw),
        None => std::ptr::null_mut(),
    }
}

/// Let go: where pictures land (the end of the line the gap opened after),
/// or -1 for where the finger is. The files follow, once iOS has copied
/// them, through `kolada_drop_files`.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_drop(h: *mut KoladaApp) -> i64 {
    app(h).model.drag_drop().map_or(-1, |end| end as i64)
}

/// The files of a drop, copied to paths one a line: pictures saved and
/// embedded after `end` (or at (x, y) when it is -1), notes copied in,
/// documents made notes. Whether the screen changes.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_drop_files(h: *mut KoladaApp, list: *const c_char, x: f32, y: f32, end: i64) -> bool {
    let files = paths(list);
    let m = &mut app(h).model;
    if end >= 0 { m.drop_files_after(files, end as usize) } else { m.drop_files(files, Some((x, y))) }
}

/// A file opened with Kolada: one of this vault's own opens; any other is
/// brought in (iOS hands the app a copy).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_open_file(h: *mut KoladaApp, path: *const c_char) -> bool {
    let path = PathBuf::from(text(path));
    let m = &mut app(h).model;
    if path.starts_with(m.vault()) {
        m.open_path(&path);
        return true;
    }
    m.import_files(vec![path])
}
