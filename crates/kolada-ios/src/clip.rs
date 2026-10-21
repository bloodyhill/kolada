//! The pasteboard, as the Mac has it (`model/paste.rs`): a copy is three
//! kinds at once (plain text without marks, HTML, and Kolada's own
//! `app.kolada.markdown`, the exact slice), and a paste takes Kolada's own
//! first, then a web address over selected words (it links them), then
//! formatted text as Markdown, then plain text; a picture alone (a
//! screenshot) is saved with the attachments and embedded.

use std::ffi::{CString, c_char};

use crate::{KoladaApp, app, text};

fn out(s: &str) -> *mut c_char {
    CString::new(s.replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Copy (and Cut, before it deletes): the selection as plain text, HTML and
/// Kolada's Markdown, three C strings to free; false with nothing selected.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_copy(h: *mut KoladaApp, plain: *mut *mut c_char, html: *mut *mut c_char, markdown: *mut *mut c_char) -> bool {
    let Some(c) = app(h).model.copy() else { return false };
    unsafe {
        *plain = out(&c.plain);
        *html = out(&c.html);
        *markdown = out(&c.markdown);
    }
    true
}

/// Cut: what `kolada_copy` gives, then the selection goes (one undo step).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_cut(h: *mut KoladaApp, plain: *mut *mut c_char, html: *mut *mut c_char, markdown: *mut *mut c_char) -> bool {
    kolada_copy(h, plain, html, markdown) && app(h).model.insert("")
}

/// Paste: the pasteboard's `markdown` (Kolada's own, or null), `plain`
/// text and `html` (or null). Whether the screen changes.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_paste(h: *mut KoladaApp, plain: *const c_char, html: *const c_char, markdown: *const c_char) -> bool {
    let m = &mut app(h).model;
    if !markdown.is_null() && !text(markdown).is_empty() {
        return m.paste_kolada(text(markdown));
    }
    let plain = text(plain);
    if m.paste_address(plain) {
        return true;
    }
    let page = (!html.is_null()).then(|| text(html).to_string()).filter(|p| !p.is_empty());
    m.paste(plain, || page.map(|p| (p, None)))
}

/// Paste the text alone, its formatting left behind.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_paste_plain(h: *mut KoladaApp, plain: *const c_char) -> bool {
    app(h).model.paste_plain(text(plain))
}

/// A picture pasted with no text (a screenshot): its PNG bytes.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_paste_picture(h: *mut KoladaApp, png: *const u8, length: usize) -> bool {
    if png.is_null() || length == 0 {
        return false;
    }
    app(h).model.paste_picture(unsafe { std::slice::from_raw_parts(png, length) })
}
