//! Startup timing, printed when `NOTSY_TRACE` is set: each stage's time since
//! the process was created (not since `main`, which comes after the loader).

use std::sync::OnceLock;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NOTSY_TRACE").is_some())
}

/// Milliseconds since this process was created.
pub fn age_ms() -> Option<f64> {
    let pid = std::process::id() as i32;
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, &mut info as *mut _ as *mut libc::c_void, size) };
    if got != size {
        return None;
    }
    let mut now = libc::timeval { tv_sec: 0, tv_usec: 0 };
    unsafe { libc::gettimeofday(&mut now, std::ptr::null_mut()) };
    Some((now.tv_sec - info.pbi_start_tvsec as libc::time_t) as f64 * 1000.0 + (now.tv_usec as f64 - info.pbi_start_tvusec as f64) / 1000.0)
}

/// Says that `what` has just happened.
pub fn mark(what: &str) {
    if enabled() {
        match age_ms() {
            Some(ms) => eprintln!("{what}: {ms:.1} ms"),
            None => eprintln!("{what}"),
        }
    }
}

/// With `NOTSY_EXIT_AFTER_FIRST_FRAME` set (the speed gate does), the app
/// quits, by its normal quit (so the session is saved as a user's quit
/// saves it), once the first frame is on the screen: the run loop does it,
/// after the frame's own code has finished.
pub fn exit_if_asked(mtm: objc2::MainThreadMarker) {
    use objc2::sel;
    use objc2_foundation::NSObjectNSDelayedPerforming;
    if std::env::var_os("NOTSY_EXIT_AFTER_FIRST_FRAME").is_some() {
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        unsafe { app.performSelector_withObject_afterDelay(sel!(terminate:), None, 0.0) };
    }
}
