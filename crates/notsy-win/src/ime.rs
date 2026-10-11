//! Input methods (Japanese, Chinese, Korean; the emoji panel) through IMM32:
//! what is being composed, read as it changes, and the candidate list placed
//! at the caret. The composition shows in the text itself, as marked text
//! (see `notsy_editor::Editor::set_marked`), so the input method's own
//! floating composition window is turned off. imm32.dll is delay-loaded:
//! nothing here runs before the first frame.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::UI::Input::Ime::{
    ATTR_TARGET_CONVERTED, ATTR_TARGET_NOTCONVERTED, CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM, CPS_CANCEL, GCS_COMPATTR, GCS_COMPSTR,
    GCS_CURSORPOS, GCS_RESULTSTR, IME_COMPOSITION_STRING, ISC_SHOWUICOMPOSITIONWINDOW, ImmGetCompositionStringW, ImmGetContext, ImmNotifyIME,
    ImmReleaseContext, ImmSetCandidateWindow, ImmSetCompositionWindow, NI_COMPOSITIONSTR,
};

/// Text still being composed: the caret's byte in it, and the bytes being
/// converted now (the clause the candidate list is for), if any.
#[derive(Debug, PartialEq)]
pub struct Composing {
    pub text: String,
    pub cursor: usize,
    pub target: Option<(usize, usize)>,
}

/// What a WM_IME_COMPOSITION carries (`flags` is its lParam): text the
/// input method settled on, and text still being composed. Either may come
/// alone or both together (a word settled as the next one starts).
pub fn read(hwnd: HWND, flags: u32) -> (Option<String>, Option<Composing>) {
    let has = |f: IME_COMPOSITION_STRING| flags & f.0 != 0;
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return (None, None);
        }
        let result = has(GCS_RESULTSTR).then(|| String::from_utf16_lossy(&units(himc, GCS_RESULTSTR)));
        let composing = has(GCS_COMPSTR).then(|| {
            let text = units(himc, GCS_COMPSTR);
            let cursor = if has(GCS_CURSORPOS) { ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0).max(0) as usize } else { text.len() };
            let attrs = if has(GCS_COMPATTR) { bytes(himc, GCS_COMPATTR) } else { Vec::new() };
            composing(&text, cursor, &attrs)
        });
        let _ = ImmReleaseContext(hwnd, himc);
        (result, composing)
    }
}

/// A composition from its UTF-16 text, caret (in units) and per-unit
/// attributes, in UTF-8 bytes.
pub fn composing(text: &[u16], cursor: usize, attrs: &[u8]) -> Composing {
    let s = String::from_utf16_lossy(text);
    // UTF-8 byte of each UTF-16 unit's start (and the end).
    let mut at = Vec::with_capacity(text.len() + 1);
    let mut b = 0;
    for c in s.chars() {
        at.push(b);
        if c.len_utf16() == 2 {
            at.push(b);
        }
        b += c.len_utf8();
    }
    at.push(b);
    let byte = |unit: usize| at[unit.min(at.len() - 1)];
    let target = attrs.iter().position(|&a| a as u32 == ATTR_TARGET_CONVERTED || a as u32 == ATTR_TARGET_NOTCONVERTED).map(|first| {
        let last = attrs.iter().rposition(|&a| a as u32 == ATTR_TARGET_CONVERTED || a as u32 == ATTR_TARGET_NOTCONVERTED).unwrap_or(first);
        (byte(first), byte(last + 1))
    });
    Composing { text: s, cursor: byte(cursor), target }
}

unsafe fn bytes(himc: windows::Win32::UI::Input::Ime::HIMC, what: IME_COMPOSITION_STRING) -> Vec<u8> {
    let n = unsafe { ImmGetCompositionStringW(himc, what, None, 0) };
    if n <= 0 {
        return Vec::new();
    }
    let mut buf = vec![0u8; n as usize];
    let got = unsafe { ImmGetCompositionStringW(himc, what, Some(buf.as_mut_ptr().cast()), n as u32) };
    buf.truncate(got.max(0) as usize);
    buf
}

unsafe fn units(himc: windows::Win32::UI::Input::Ime::HIMC, what: IME_COMPOSITION_STRING) -> Vec<u16> {
    let raw = unsafe { bytes(himc, what) };
    raw.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect()
}

/// Puts the candidate list just below `caret` (client pixels), never over
/// it, and the composition point at it for input methods that use that.
pub fn place(hwnd: HWND, caret: RECT) {
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return;
        }
        let at = POINT { x: caret.left, y: caret.top };
        let _ = ImmSetCompositionWindow(himc, &COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: at, rcArea: caret });
        let below = POINT { x: caret.left, y: caret.bottom };
        let _ = ImmSetCandidateWindow(himc, &CANDIDATEFORM { dwIndex: 0, dwStyle: CFS_EXCLUDE, ptCurrentPos: below, rcArea: caret });
        let _ = ImmReleaseContext(hwnd, himc);
    }
}

/// Drops the input method's composition (its text was settled here
/// already; see `App::settle_composition`).
pub fn cancel(hwnd: HWND) {
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return;
        }
        let _ = ImmNotifyIME(himc, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
        let _ = ImmReleaseContext(hwnd, himc);
    }
}

/// WM_IME_SETCONTEXT's lParam without the input method's own composition
/// window: the text shows in the note instead.
pub fn without_composition_window(lp: LPARAM) -> LPARAM {
    LPARAM(lp.0 & !(ISC_SHOWUICOMPOSITIONWINDOW as isize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_composition_is_read_in_utf8_bytes() {
        // "かんじ" with the caret after the second kana, the last two being
        // converted.
        let text: Vec<u16> = "\u{304B}\u{3093}\u{3058}".encode_utf16().collect();
        let c = composing(&text, 2, &[0, ATTR_TARGET_CONVERTED as u8, ATTR_TARGET_CONVERTED as u8]);
        assert_eq!(c, Composing { text: "\u{304B}\u{3093}\u{3058}".into(), cursor: 6, target: Some((3, 9)) });
        // A surrogate pair counts as one character; no target, the caret at
        // the end.
        let text: Vec<u16> = "a\u{1F600}".encode_utf16().collect();
        let c = composing(&text, 3, &[]);
        assert_eq!(c, Composing { text: "a\u{1F600}".into(), cursor: 5, target: None });
        assert_eq!(composing(&[], 0, &[]), Composing { text: String::new(), cursor: 0, target: None });
    }
}
