//! What Linux gives the app model (`kolada_app::Services`): the local
//! calendar, the web (curl), the desktop's trash, inotify, the fonts,
//! picture decoding, the spell checker and the desktop's notifications,
//! each in its own file; this puts
//! them together, with priorities and what is installed (below). No text
//! recognition yet: that stays the model's harmless stand-in.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use kolada_app::Services;
use kolada_app::services::{Installed, Local, Priority};

/// Where the OS lets a thread or a child process run below the window's.
pub struct LinuxPriority;

impl Priority for LinuxPriority {
    fn lower_thread(&self) {
        // On Linux a thread's nice value is its own (the id is the thread's).
        unsafe {
            let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
            libc::setpriority(libc::PRIO_PROCESS, tid, 10);
        }
    }

    fn lower_process(&self, pid: u32) {
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, 10);
        }
    }
}

/// What is installed, for the send menu's AI apps: tools found on `PATH`
/// (and in `~/.local/bin`, where installers put them).
pub struct LinuxInstalled;

pub fn find_tool(name: &str, path: Option<std::ffi::OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    let mut dirs: Vec<PathBuf> = path.map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(home.map(|h| h.join(".local").join("bin")));
    dirs.into_iter().map(|d| d.join(name)).find(|p| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

/// What `PATH` was found to hold, by name: looked up once, off the UI's
/// thread (`warm`). A `PATH` of many folders, or of slow ones (WSL's
/// Windows folders took 1.5 s), held up the palette and the send menu,
/// which ask as they open.
fn found() -> &'static Mutex<HashMap<String, Option<PathBuf>>> {
    static FOUND: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    FOUND.get_or_init(Default::default)
}

impl Installed for LinuxInstalled {
    fn app(&self, name: &str) -> bool {
        // Desktop apps are commands too ("claude-desktop"), by their lower-case name.
        let lower = name.to_lowercase();
        [lower.clone(), format!("{lower}-desktop")].iter().any(|n| self.tool(n).is_some())
    }

    fn tool(&self, name: &str) -> Option<PathBuf> {
        if let Some(hit) = found().lock().ok().and_then(|f| f.get(name).cloned()) {
            return hit;
        }
        let hit = find_tool(name, std::env::var_os("PATH"), std::env::var_os("HOME").map(PathBuf::from));
        if let Ok(mut f) = found().lock() {
            f.insert(name.to_string(), hit.clone());
        }
        hit
    }
}

/// Looks up the AI apps and tools the send menu and the palette offer, on a
/// thread of its own (after the first frame), so they open at once.
pub fn warm() {
    std::thread::spawn(|| {
        LinuxPriority.lower_thread();
        kolada_app::model::share::Agent::detect_with(|a| LinuxInstalled.app(a), |t| LinuxInstalled.tool(t).is_some());
    });
}

/// Now, on the local calendar.
pub fn now() -> Local {
    kolada_app::services::now(&super::when::LinuxCalendar)
}

/// All of them, for the app and for dumps.
pub fn services() -> Services {
    Services {
        calendar: Arc::new(super::when::LinuxCalendar),
        web: Arc::new(super::web::LinuxWeb),
        bin: Arc::new(super::bin::LinuxBin),
        watcher: Arc::new(super::watch::LinuxWatcher),
        priority: Arc::new(LinuxPriority),
        fonts: Arc::new(super::fonts::LinuxFonts),
        images: Arc::new(super::images::LinuxImages),
        installed: Arc::new(LinuxInstalled),
        spell: Arc::new(super::spell::LinuxSpell),
        notify: Arc::new(super::notify::LinuxNotify),
        ..Services::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_found_on_the_path_and_in_local_bin_and_only_if_they_run() {
        let dir = std::env::temp_dir().join(format!("kolada-linux-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (bin, home) = (dir.join("bin"), dir.join("home"));
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let make = |p: PathBuf, mode: u32| {
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        make(bin.join("codex"), 0o755);
        make(bin.join("notes.txt"), 0o644);
        make(home.join(".local/bin/gemini"), 0o755);
        let path = Some(std::env::join_paths([&bin]).unwrap());
        assert_eq!(find_tool("codex", path.clone(), Some(home.clone())), Some(bin.join("codex")));
        assert_eq!(find_tool("gemini", path.clone(), Some(home.clone())), Some(home.join(".local/bin/gemini")));
        assert_eq!(find_tool("notes.txt", path.clone(), Some(home.clone())), None, "not a program");
        assert_eq!(find_tool("../bin/codex", path, Some(home)), None, "a name, not a path");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_background_thread_can_lower_itself() {
        std::thread::spawn(|| {
            LinuxPriority.lower_thread();
            let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, libc::syscall(libc::SYS_gettid) as libc::id_t) };
            assert!(nice >= 10, "{nice}");
        })
        .join()
        .unwrap();
        let main = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
        assert!(main < 10, "the rest of the app keeps its priority: {main}");
    }
}
