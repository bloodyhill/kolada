//! Following what other apps do to the vault: FSEvents tells a thread when
//! files change under it, which hands the news to the app (a redraw is asked
//! for with `wake`).

use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

type Ref = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(allocator: Ref, c_str: *const c_char, encoding: u32) -> Ref;
    fn CFArrayCreate(allocator: Ref, values: *const Ref, count: isize, callbacks: Ref) -> Ref;
    fn CFRelease(object: Ref);
    fn CFRunLoopGetCurrent() -> Ref;
    fn CFRunLoopRun();
    static kCFRunLoopDefaultMode: Ref;
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn FSEventStreamCreate(
        allocator: Ref,
        callback: extern "C" fn(Ref, *mut c_void, usize, *const *const c_char, *const u32, *const u64),
        context: *const FSContext,
        paths: Ref,
        since: u64,
        latency: f64,
        flags: u32,
    ) -> Ref;
    fn FSEventStreamScheduleWithRunLoop(stream: Ref, run_loop: Ref, mode: Ref);
    fn FSEventStreamStart(stream: Ref) -> u8;
}

#[repr(C)]
struct FSContext {
    version: isize,
    info: *mut c_void,
    retain: Ref,
    release: Ref,
    copy_description: Ref,
}

const UTF8: u32 = 0x0800_0100;
/// Events for files, not just their folders; no waiting for the first.
const FILE_EVENTS: u32 = 0x10;
const NO_DEFER: u32 = 0x02;
const SINCE_NOW: u64 = u64::MAX;

// The flags the model reads are FSEvents' own (`notsy_app::services` has them
// at the same values), so they pass through as they come.
use notsy_app::services::Change;

/// FSEvents, for the model's `Watcher` service.
pub struct MacWatcher;

impl notsy_app::services::Watcher for MacWatcher {
    fn watch(&self, vault: &Path, wake: Box<dyn Fn() + Send>) -> Receiver<Change> {
        start(vault, wake)
    }
}

struct Sink {
    out: Sender<Change>,
    wake: Box<dyn Fn() + Send>,
}

extern "C" fn changed(_: Ref, info: *mut c_void, count: usize, paths: *const *const c_char, flags: *const u32, _: *const u64) {
    let sink = unsafe { &*(info as *const Sink) };
    for i in 0..count {
        let path = unsafe { CStr::from_ptr(*paths.add(i)) };
        let flags = unsafe { *flags.add(i) };
        let _ = sink.out.send(Change { path: PathBuf::from(path.to_string_lossy().into_owned()), flags });
    }
    (sink.wake)();
}

/// Watches `vault` on a thread of its own: the paths that change arrive in
/// the receiver, with what happened to them, and `wake` is called after each batch.
pub fn start(vault: &Path, wake: impl Fn() + Send + 'static) -> Receiver<Change> {
    let (out, rx) = channel();
    let vault = vault.to_path_buf();
    std::thread::spawn(move || {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c_path) = std::ffi::CString::new(vault.as_os_str().as_bytes()) else { return };
        // The sink lives as long as the stream, which is as long as the thread.
        let sink = Box::new(Sink { out, wake: Box::new(wake) });
        unsafe {
            let string = CFStringCreateWithCString(std::ptr::null(), c_path.as_ptr(), UTF8);
            let array = CFArrayCreate(std::ptr::null(), &string, 1, std::ptr::null());
            let context = FSContext { version: 0, info: &*sink as *const Sink as *mut c_void, retain: std::ptr::null(), release: std::ptr::null(), copy_description: std::ptr::null() };
            let stream = FSEventStreamCreate(std::ptr::null(), changed, &context, array, SINCE_NOW, 0.25, FILE_EVENTS | NO_DEFER);
            CFRelease(array);
            CFRelease(string);
            if stream.is_null() {
                return;
            }
            FSEventStreamScheduleWithRunLoop(stream, CFRunLoopGetCurrent(), kCFRunLoopDefaultMode);
            if FSEventStreamStart(stream) != 0 {
                CFRunLoopRun();
            }
        }
        drop(sink);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use notsy_app::services::ITEM_RENAMED;
    use std::time::{Duration, Instant};

    #[test]
    fn a_file_written_under_the_vault_is_reported() {
        let dir = std::env::temp_dir().join(format!("notsy-mac-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let rx = start(&dir, || {});
        std::thread::sleep(Duration::from_millis(500));
        std::fs::write(dir.join("Outside.md"), "written by another app").unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        let mut seen = false;
        while Instant::now() < until && !seen {
            seen = rx.recv_timeout(Duration::from_millis(100)).is_ok_and(|c| c.path.ends_with("Outside.md"));
        }
        assert!(seen, "the change was reported");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_renamed_by_another_app_is_reported_as_renamed_at_both_names() {
        let dir = std::env::temp_dir().join(format!("notsy-mac-watch-{}-rename", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        std::fs::write(dir.join("Before.md"), "x").unwrap();
        let rx = start(&dir, || {});
        std::thread::sleep(Duration::from_millis(500));
        std::fs::rename(dir.join("Before.md"), dir.join("After.md")).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        let (mut old, mut new) = (false, false);
        while Instant::now() < until && !(old && new) {
            if let Ok(c) = rx.recv_timeout(Duration::from_millis(100)) {
                old |= c.path.ends_with("Before.md") && c.flags & ITEM_RENAMED != 0;
                new |= c.path.ends_with("After.md") && c.flags & ITEM_RENAMED != 0;
            }
        }
        assert!(old && new, "both names were reported, as renamed (old {old}, new {new})");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
