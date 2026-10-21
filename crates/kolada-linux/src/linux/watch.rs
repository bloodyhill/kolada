//! Following what other apps do to the vault: inotify tells a thread when
//! files change in the vault's folders (each folder watched, new ones as they
//! come), which hands the news to the app (a redraw is asked for with `wake`).

use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use kolada_app::services::{Change, ITEM_CREATED, ITEM_REMOVED, ITEM_RENAMED};

/// A file written and closed: FSEvents' "modified", which the model reads as
/// any change that is neither made, removed nor renamed.
pub const ITEM_MODIFIED: u32 = 0x1000;

const MASK: u32 = libc::IN_CLOSE_WRITE | libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO;

/// inotify, for the model's `Watcher` service.
pub struct LinuxWatcher;

impl kolada_app::services::Watcher for LinuxWatcher {
    fn watch(&self, vault: &Path, wake: Box<dyn Fn() + Send>) -> Receiver<Change> {
        start(vault, wake)
    }
}

/// What an inotify event's mask says happened, as the model's flags.
pub fn flags(mask: u32) -> u32 {
    if mask & (libc::IN_MOVED_FROM | libc::IN_MOVED_TO) != 0 {
        ITEM_RENAMED
    } else if mask & libc::IN_CREATE != 0 {
        ITEM_CREATED
    } else if mask & libc::IN_DELETE != 0 {
        ITEM_REMOVED
    } else {
        ITEM_MODIFIED
    }
}

/// The folders watched, by watch descriptor.
struct Watches {
    fd: i32,
    dirs: HashMap<i32, PathBuf>,
}

impl Watches {
    /// Watches `dir` and every folder under it (hidden ones, `.git` and the
    /// like, are not notes and are left out).
    fn add_tree(&mut self, dir: &Path) {
        let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else { return };
        let wd = unsafe { libc::inotify_add_watch(self.fd, c.as_ptr(), MASK | libc::IN_ONLYDIR) };
        if wd < 0 {
            return;
        }
        self.dirs.insert(wd, dir.to_path_buf());
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let hidden = e.file_name().as_bytes().first() == Some(&b'.');
            if !hidden && e.file_type().is_ok_and(|t| t.is_dir()) {
                self.add_tree(&e.path());
            }
        }
    }
}

/// Watches `vault` on a thread of its own: the paths that change arrive in
/// the receiver, with what happened to them, and `wake` is called after each batch.
pub fn start(vault: &Path, wake: impl Fn() + Send + 'static) -> Receiver<Change> {
    let (out, rx) = channel();
    let vault = vault.to_path_buf();
    std::thread::spawn(move || {
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 {
            return;
        }
        let mut w = Watches { fd, dirs: HashMap::new() };
        w.add_tree(&vault);
        follow(&mut w, &out, &wake);
        unsafe { libc::close(fd) };
    });
    rx
}

/// Reads events until the app stops listening.
fn follow(w: &mut Watches, out: &Sender<Change>, wake: &dyn Fn()) {
    // Room for many events at once: each is 16 bytes and a name.
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = unsafe { libc::read(w.fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return;
        }
        let mut at = 0usize;
        let n = n as usize;
        while at + 16 <= n {
            let word = |i: usize| u32::from_ne_bytes(buf[at + i..at + i + 4].try_into().unwrap());
            let (wd, mask, len) = (word(0) as i32, word(4), word(12) as usize);
            let name = &buf[at + 16..(at + 16 + len).min(n)];
            let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
            at += 16 + len;
            if mask & libc::IN_IGNORED != 0 {
                w.dirs.remove(&wd);
                continue;
            }
            let Some(dir) = w.dirs.get(&wd).cloned() else { continue };
            let path = if name.is_empty() { dir } else { dir.join(OsStr::from_bytes(name)) };
            // A folder made or moved in: its own folders are watched too.
            if mask & libc::IN_ISDIR != 0 && mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0 && name.first() != Some(&b'.') {
                w.add_tree(&path);
            }
            if out.send(Change { path, flags: flags(mask) }).is_err() {
                return;
            }
        }
        wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-linux-watch-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn until(rx: &Receiver<Change>, mut done: impl FnMut(&Change) -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if rx.recv_timeout(Duration::from_millis(100)).is_ok_and(|c| done(&c)) {
                return true;
            }
        }
        false
    }

    #[test]
    fn a_file_written_under_the_vault_is_reported() {
        let dir = scratch("write");
        std::fs::create_dir_all(dir.join("Work")).unwrap();
        let rx = start(&dir, || {});
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(dir.join("Outside.md"), "written by another app").unwrap();
        assert!(until(&rx, |c| c.path == dir.join("Outside.md")), "the change was reported");
        std::fs::write(dir.join("Work/Deep.md"), "x").unwrap();
        assert!(until(&rx, |c| c.path == dir.join("Work/Deep.md")), "in a folder too");
        // A folder made after the watch began is followed as well.
        std::fs::create_dir_all(dir.join("Later")).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(dir.join("Later/New.md"), "x").unwrap();
        assert!(until(&rx, |c| c.path == dir.join("Later/New.md")), "and in a new folder");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_renamed_by_another_app_is_reported_as_renamed_at_both_names() {
        let dir = scratch("rename");
        std::fs::write(dir.join("Before.md"), "x").unwrap();
        let rx = start(&dir, || {});
        std::thread::sleep(Duration::from_millis(200));
        std::fs::rename(dir.join("Before.md"), dir.join("After.md")).unwrap();
        let (mut old, mut new) = (false, false);
        assert!(until(&rx, |c| {
            old |= c.path.ends_with("Before.md") && c.flags & ITEM_RENAMED != 0;
            new |= c.path.ends_with("After.md") && c.flags & ITEM_RENAMED != 0;
            old && new
        }), "both names were reported, as renamed (old {old}, new {new})");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inotifys_events_read_as_the_models_flags() {
        assert_eq!(flags(libc::IN_CREATE), ITEM_CREATED);
        assert_eq!(flags(libc::IN_DELETE), ITEM_REMOVED);
        assert_eq!(flags(libc::IN_MOVED_FROM), ITEM_RENAMED);
        assert_eq!(flags(libc::IN_MOVED_TO | libc::IN_ISDIR), ITEM_RENAMED);
        assert_eq!(flags(libc::IN_CLOSE_WRITE), ITEM_MODIFIED);
    }
}
