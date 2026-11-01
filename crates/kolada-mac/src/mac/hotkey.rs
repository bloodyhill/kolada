//! Keys from any app: Ctrl+Option+N brings Kolada forward with a fresh page
//! (the quick note), and Option+Z shows Beam with what was copied (with
//! Shift too: as a password), as Alt+Z does on Windows. Where another app
//! holds Option+Z, Ctrl+Option+Shift+Z (and Command for a password), as
//! before. Option+Z types Ω elsewhere; Kolada takes it while it runs. The
//! keys are the letters, wherever the keyboard layout puts them (Z is where
//! Y is on a German keyboard, W on a French one), held again when the
//! layout changes; where Option+Z types a letter of the layout's own
//! (Polish Pro's ż), Beam is on its other keys. Carbon's hotkey registration is
//! the one the system offers apps for this: it needs no Accessibility or
//! Input Monitoring permission and changes no system setting. Only these
//! keys are registered, and only while their settings are on.

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::Once;

use objc2::MainThreadMarker;

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> *mut c_void;
    fn TISGetInputSourceProperty(source: *mut c_void, key: *const c_void) -> *const c_void;
    static kTISPropertyUnicodeKeyLayoutData: *const c_void;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(layout: *const c_void, code: u16, action: u16, modifiers: u32, keyboard: u32, options: u32, dead: *mut u32, most: usize, length: *mut usize, chars: *mut u16) -> i32;
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(target: *mut c_void, handler: Handler, count: u32, types: *const EventTypeSpec, data: *mut c_void, out: *mut *mut c_void) -> i32;
    fn RegisterEventHotKey(code: u32, modifiers: u32, id: EventHotKeyId, target: *mut c_void, options: u32, out: *mut *mut c_void) -> i32;
    fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
    fn GetEventParameter(event: *mut c_void, name: u32, kind: u32, actual: *mut u32, size: usize, out_size: *mut usize, data: *mut c_void) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFRelease(cf: *const c_void);
}

/// 'keyb', and kEventHotKeyPressed.
const KEYBOARD: u32 = 0x6b65_7962;
const HOT_KEY_PRESSED: u32 = 5;
/// kEventParamDirectObject ('----') as typeEventHotKeyID ('hkid').
const DIRECT: u32 = 0x2d2d_2d2d;
const HOT_KEY_ID: u32 = 0x686b_6964;
/// The N and Z keys where a US keyboard has them (where no key of the
/// layout types the letter: Russian, Arabic); Control, Option, Shift and
/// Command.
const KEY_N: u32 = 45;
const KEY_Z: u32 = 6;
const OPTION: u32 = 0x0800;
const CONTROL_OPTION: u32 = 0x1000 | OPTION;
const SHIFT: u32 = 0x0200;
const COMMAND: u32 = 0x0100;
/// 'Nsty'.
const SIGNATURE: u32 = 0x4e73_7479;
/// The ids: the quick note, Quick Send, Quick Send as a password.
const NOTE: u32 = 1;
const QUICK: u32 = 2;
const SECRET: u32 = 3;

/// How the menus and Settings write Beam's keys (`Session::quick_keys`:
/// 0 Option+Z, 1 the old Control+Option+Shift+Z, 2 off).
pub const QUICK_KEYS: &str = "\u{2325}Z";
pub const OLD_KEYS: &str = "\u{2303}\u{2325}\u{21E7}Z";

thread_local! {
    /// Each id's registration (at id - 1), while held.
    static REGISTERED: [Cell<*mut c_void>; 3] = const { [Cell::new(std::ptr::null_mut()), Cell::new(std::ptr::null_mut()), Cell::new(std::ptr::null_mut())] };
    /// The quick note's keys are wanted (`set`), to hold them again on a
    /// new layout.
    static NOTE_ON: Cell<bool> = const { Cell::new(false) };
}

/// What key `code` types on the current layout with `modifiers` (Carbon's,
/// shifted down as UCKeyTranslate takes them), as one character.
fn typed(layout: *const u8, code: u16, modifiers: u32) -> Option<char> {
    let (mut dead, mut length, mut chars) = (0u32, 0usize, [0u16; 4]);
    // kUCKeyActionDisplay, and no dead keys.
    let done = unsafe { UCKeyTranslate(layout as *const c_void, code, 3, (modifiers >> 8) & 0xFF, LMGetKbdType() as u32, 1, &mut dead, chars.len(), &mut length, chars.as_mut_ptr()) };
    if done != 0 || length != 1 {
        return None;
    }
    char::from_u32(chars[0] as u32)
}

/// The key of the first code (0 to 127) whose character is `letter`.
fn code_for(letter: char, typed: impl Fn(u16) -> Option<char>) -> Option<u32> {
    (0..128u16).find(|&c| typed(c).is_some_and(|t| t.to_lowercase().eq(letter.to_lowercase()))).map(u32::from)
}

/// A letter of a language written in Latin letters (ż, ñ, å), which a
/// layout types with Option for its own words: not Option+Z's Ω, a symbol.
fn latin_letter(c: char) -> bool {
    c.is_alphabetic() && matches!(c as u32, 0x00C0..=0x024F | 0x1E00..=0x1EFF)
}

/// The current layout's keys for N and Z, and whether Option+Z types a
/// letter there; the US positions where the layout cannot be read.
fn layout_keys() -> (u32, u32, bool) {
    unsafe {
        let source = TISCopyCurrentASCIICapableKeyboardLayoutInputSource();
        if source.is_null() {
            return (KEY_N, KEY_Z, false);
        }
        let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
        let keys = if data.is_null() {
            (KEY_N, KEY_Z, false)
        } else {
            let layout = CFDataGetBytePtr(data);
            let plain = |c: u16| typed(layout, c, 0);
            let z = code_for('z', plain).unwrap_or(KEY_Z);
            let letter = typed(layout, z as u16, OPTION).is_some_and(latin_letter);
            (code_for('n', plain).unwrap_or(KEY_N), z, letter)
        };
        CFRelease(source);
        keys
    }
}

/// Whether Option+Z types a letter of the current layout (Polish Pro's ż):
/// Beam is then on its other keys, so typing keeps it.
pub fn option_z_types_a_letter() -> bool {
    layout_keys().2
}

/// The keyboard layout changed: the keys held are held again where its
/// letters are, and `then` is told (Beam's, which its view holds again).
pub fn follow_layout(then: impl Fn() + 'static) {
    use block2::RcBlock;
    let center = objc2_foundation::NSDistributedNotificationCenter::defaultCenter();
    let name = objc2_foundation::NSString::from_str("com.apple.Carbon.TISNotifySelectedKeyboardInputSourceChanged");
    let changed = RcBlock::new(move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
        if NOTE_ON.with(Cell::get) {
            hold(NOTE, 0, 0, false);
            set(true);
        }
        then();
    });
    let main = objc2_foundation::NSOperationQueue::mainQueue();
    let observer = unsafe { center.addObserverForName_object_queue_usingBlock(Some(&name), None, Some(&main), &changed) };
    // Kept for as long as the app runs.
    std::mem::forget(observer);
}

static INSTALLED: Once = Once::new();

/// A key was pressed (on the main thread, where the event loop is).
extern "C" fn pressed(_next: *mut c_void, event: *mut c_void, _data: *mut c_void) -> i32 {
    let mut id = EventHotKeyId { signature: 0, id: NOTE };
    unsafe { GetEventParameter(event, DIRECT, HOT_KEY_ID, std::ptr::null_mut(), std::mem::size_of::<EventHotKeyId>(), std::ptr::null_mut(), &mut id as *mut _ as *mut c_void) };
    if let Some(mtm) = MainThreadMarker::new() {
        match id.id {
            QUICK => super::view::quick::keys(mtm, false),
            SECRET => super::view::quick::keys(mtm, true),
            _ => super::status::quick_note(mtm),
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The letters, wherever the layout puts them: Z where Y is on a
    /// German keyboard, where W is on a French one; the US key where no key
    /// types it.
    #[test]
    fn the_keys_are_the_letters_wherever_the_layout_puts_them() {
        let us = |c: u16| match c { 6 => Some('z'), 16 => Some('y'), 13 => Some('w'), 45 => Some('n'), _ => None };
        let german = |c: u16| match c { 6 => Some('y'), 16 => Some('z'), 45 => Some('n'), _ => None };
        let french = |c: u16| match c { 6 => Some('w'), 13 => Some('z'), 45 => Some('n'), _ => None };
        let russian = |c: u16| match c { 6 => Some('\u{044F}'), _ => None };
        assert_eq!(code_for('z', us), Some(6));
        assert_eq!(code_for('z', german), Some(16));
        assert_eq!(code_for('z', french), Some(13));
        assert_eq!(code_for('z', russian), None, "the US key, then");
        assert!(latin_letter('\u{017C}'), "Polish Pro's Option+Z: z with a dot");
        assert!(!latin_letter('\u{03A9}'), "the US Option+Z: an omega, a symbol here");
        assert!(!latin_letter('\u{00A5}'), "a yen sign");
    }

    #[test]
    #[ignore = "registers real global hotkeys: try it by hand in a logged-in session (cargo test -- --ignored)"]
    fn the_hotkeys_register_and_unregister() {
        assert!(set(true), "the system accepted Ctrl+Option+N");
        assert!(set(true), "turning it on twice is fine");
        assert!(!set(false));
        assert!(set(true) && !set(false), "and it can come back");
        assert!(set_quick(0), "the system accepted Option+Z");
        assert!(set_quick(1), "and, chosen, Ctrl+Option+Shift+Z in its place");
        assert!(set(true), "beside the quick note's");
        assert!(!set_quick(2) && !set(false));
    }
}

/// Holds `id` on `code` with `modifiers`, or lets it go: whether it is held.
fn hold(id: u32, code: u32, modifiers: u32, on: bool) -> bool {
    REGISTERED.with(|r| {
        let r = &r[id as usize - 1];
        if !on {
            let key = r.replace(std::ptr::null_mut());
            if !key.is_null() {
                unsafe { UnregisterEventHotKey(key) };
            }
            return false;
        }
        if !r.get().is_null() {
            return true;
        }
        unsafe {
            INSTALLED.call_once(|| {
                let spec = EventTypeSpec { class: KEYBOARD, kind: HOT_KEY_PRESSED };
                InstallEventHandler(GetApplicationEventTarget(), pressed, 1, &spec, std::ptr::null_mut(), std::ptr::null_mut());
            });
            let mut key: *mut c_void = std::ptr::null_mut();
            let made = RegisterEventHotKey(code, modifiers, EventHotKeyId { signature: SIGNATURE, id }, GetApplicationEventTarget(), 0, &mut key);
            if made == 0 && !key.is_null() {
                r.set(key);
            }
            made == 0
        }
    })
}

/// Turns the quick note's hotkey on or off. Returns whether it is on (the
/// system refuses a key another app already took).
pub fn set(on: bool) -> bool {
    NOTE_ON.with(|n| n.set(on));
    let code = if on { layout_keys().0 } else { KEY_N };
    hold(NOTE, code, CONTROL_OPTION, on)
}

/// Beam's keys as Settings has them (`Session::quick_keys`): 0 Option+Z
/// and Option+Shift+Z for a password; 1 Ctrl+Option+Shift+Z and Command
/// with it for a password; 2 none. Whether Beam's are held (another app
/// may have them). What was held before is let go first.
pub fn set_quick(keys: u8) -> bool {
    hold(SECRET, 0, 0, false);
    hold(QUICK, 0, 0, false);
    let (beam, secret) = match keys {
        0 => (OPTION, OPTION | SHIFT),
        1 => (CONTROL_OPTION | SHIFT, CONTROL_OPTION | SHIFT | COMMAND),
        _ => return false,
    };
    let z = layout_keys().1;
    if !hold(QUICK, z, beam, true) {
        return false;
    }
    hold(SECRET, z, secret, true);
    true
}
