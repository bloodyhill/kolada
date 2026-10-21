//! Where X loses what the window showed. Without a compositor, a window
//! that covers ours (a dialog, a menu, a dragged file's icon) takes our
//! pixels with it, and X asks for them again with an Expose. winit turns that
//! into a redraw like any other, so nothing says that all of the window must
//! go up again rather than what changed (seen under Xvfb: a dialog's place
//! stayed black after it closed). This watches for exposures on a connection
//! of its own and says so. Wayland always composites: nothing to watch.

use std::os::raw::c_ulong;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// The window's X id, when it is an X window.
pub fn x_window(window: &Window) -> Option<c_ulong> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => Some(h.window),
        RawWindowHandle::Xcb(h) => Some(h.window.get() as c_ulong),
        _ => None,
    }
}

/// Calls `exposed` (from a thread of its own) each time X has lost part of
/// `window`'s pixels.
pub fn watch(window: c_ulong, exposed: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let Ok(xlib) = x11_dl::xlib::Xlib::open() else { return };
        unsafe {
            let display = (xlib.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return;
            }
            // Events go to every connection that asks: ours, and winit's.
            (xlib.XSelectInput)(display, window, x11_dl::xlib::ExposureMask);
            let mut event: x11_dl::xlib::XEvent = std::mem::zeroed();
            loop {
                (xlib.XNextEvent)(display, &mut event);
                if event.get_type() == x11_dl::xlib::Expose && event.expose.count == 0 {
                    exposed();
                }
                if event.get_type() == x11_dl::xlib::DestroyNotify {
                    break;
                }
            }
        }
    });
}
