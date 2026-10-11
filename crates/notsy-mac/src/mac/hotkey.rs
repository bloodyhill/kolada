//! A quick note from any app: Ctrl+Option+N brings Notsy forward with a
//! fresh page. Carbon's hotkey registration is the one the system offers
//! apps for this: it needs no Accessibility or Input Monitoring permission
//! and changes no system setting. Only this key is registered, and only
//! while the setting is on.

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
}

/// 'keyb', and kEventHotKeyPressed.
const KEYBOARD: u32 = 0x6b65_7962;
const HOT_KEY_PRESSED: u32 = 5;
/// The N key; Control and Option.
const KEY_N: u32 = 45;
const CONTROL_OPTION: u32 = 0x1000 | 0x0800;
/// 'Nsty'.
const SIGNATURE: u32 = 0x4e73_7479;

thread_local! {
    static REGISTERED: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
}

static INSTALLED: Once = Once::new();

/// The key was pressed (on the main thread, where the event loop is).
extern "C" fn pressed(_next: *mut c_void, _event: *mut c_void, _data: *mut c_void) -> i32 {
    if let Some(mtm) = MainThreadMarker::new() {
        super::status::quick_note(mtm);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "registers a real global hotkey: try it by hand in a logged-in session (cargo test -- --ignored)"]
    fn the_hotkey_registers_and_unregisters() {
        assert!(set(true), "the system accepted Ctrl+Option+N");
        assert!(set(true), "turning it on twice is fine");
        assert!(!set(false));
        assert!(set(true) && !set(false), "and it can come back");
    }
}

/// Turns the hotkey on or off. Returns whether it is on (the system
/// refuses a key another app already took).
pub fn set(on: bool) -> bool {
    REGISTERED.with(|r| {
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
            let made = RegisterEventHotKey(KEY_N, CONTROL_OPTION, EventHotKeyId { signature: SIGNATURE, id: 1 }, GetApplicationEventTarget(), 0, &mut key);
            if made == 0 && !key.is_null() {
                r.set(key);
            }
            made == 0
        }
    })
}
