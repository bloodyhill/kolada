//! Keys from any app: Ctrl+Option+N brings Kolada forward with a fresh page
//! (the quick note), and Option+Z shows Beam with what was copied (with
//! Shift too: as a password), as Alt+Z does on Windows. Where another app
//! holds Option+Z, Ctrl+Option+Shift+Z (and Command for a password), as
//! before. Option+Z types Ω elsewhere; Kolada takes it while it runs. Carbon's hotkey registration is
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
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(target: *mut c_void, handler: Handler, count: u32, types: *const EventTypeSpec, data: *mut c_void, out: *mut *mut c_void) -> i32;
    fn RegisterEventHotKey(code: u32, modifiers: u32, id: EventHotKeyId, target: *mut c_void, options: u32, out: *mut *mut c_void) -> i32;
    fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
    fn GetEventParameter(event: *mut c_void, name: u32, kind: u32, actual: *mut u32, size: usize, out_size: *mut usize, data: *mut c_void) -> i32;
}

/// 'keyb', and kEventHotKeyPressed.
const KEYBOARD: u32 = 0x6b65_7962;
const HOT_KEY_PRESSED: u32 = 5;
/// kEventParamDirectObject ('----') as typeEventHotKeyID ('hkid').
const DIRECT: u32 = 0x2d2d_2d2d;
const HOT_KEY_ID: u32 = 0x686b_6964;
/// The N and Z keys; Control, Option, Shift and Command.
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
    hold(NOTE, KEY_N, CONTROL_OPTION, on)
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
    if !hold(QUICK, KEY_Z, beam, true) {
        return false;
    }
    hold(SECRET, KEY_Z, secret, true);
    true
}
