//! Notsy updating itself from the website (the rules are in
//! `notsy_core::update`). Never in the Microsoft Store's build: the Store
//! updates it, and its policy forbids an app doing so itself.
//!
//! - After the first frame and a while longer, a worker at background
//!   priority asks the feed, at most once a day, whether there is a newer
//!   version. If so it downloads it into the app's data folder and checks
//!   its SHA-256; only then does the window get its pill. Nothing that fails
//!   the check is kept.
//! - Clicked, the pill saves everything, renames the running `notsy.exe`
//!   to `notsy.old.exe` (Windows lets a running program be renamed), moves
//!   the new one into its place, and the app quits through its usual path
//!   and starts the new one with `--updated`, on the same note.
//! - Never clicked, the update goes in the same way at the next start,
//!   before the window: two renames, then the new program starts in this
//!   one's place.
//! - The next start deletes `notsy.old.exe`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use notsy_core::update::{self, Feed, Version};
use windows::Win32::Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_SUCCESS, HWND, LPARAM, WPARAM};
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::window::WM_APP_UPDATE;

/// The switch in the options, as last set.
static ON: AtomicBool = AtomicBool::new(true);
static STARTED: AtomicBool = AtomicBool::new(false);
/// The new program is in place: start it once this one has closed.
pub static RELAUNCH: AtomicBool = AtomicBool::new(false);

/// Whether this process runs from an MSIX package (the Store's build).
pub fn packaged() -> bool {
    static PACKAGED: OnceLock<bool> = OnceLock::new();
    *PACKAGED.get_or_init(|| {
        let mut len = 0u32;
        unsafe { GetCurrentPackageFullName(&mut len, None) != APPMODEL_ERROR_NO_PACKAGE }
    })
}

/// The feed to ask: the website's; under test only one named by
/// `NOTSY_UPDATE_FEED` (a server on this machine), else none.
fn feed_url() -> Option<String> {
    if notsy_core::testing() {
        std::env::var("NOTSY_UPDATE_FEED").ok().filter(|u| !u.is_empty())
    } else {
        Some(update::FEED.to_string())
    }
}

fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

fn staging() -> Option<PathBuf> {
    notsy_core::app_data_dir().map(|d| update::staging(&d))
}

/// Whether this copy of Notsy updates itself: not the Store's, not a dev
/// build, and under test only with a feed of its own.
pub fn offered() -> bool {
    !packaged() && exe().is_some_and(|e| !update::is_dev_build(&e)) && feed_url().is_some()
}

/// Before the window: an update downloaded before and never clicked goes
/// in. Returns the program now in place, to start in this one's stead.
/// Costs one look at a file when there is none.
pub fn install_waiting() -> Option<PathBuf> {
    let dir = staging()?;
    if !dir.join("ready").exists() {
        return None;
    }
    let (exe, (_, new)) = (exe()?, update::staged(&dir, &Version::current())?);
    if packaged() || update::is_dev_build(&exe) {
        update::clear(&dir);
        return None;
    }
    match update::swap(&exe, &new) {
        Ok(()) => {
            update::clear(&dir);
            Some(exe)
        }
        // It cannot go in here (the folder is read-only, say): not again.
        Err(_) => {
            update::clear(&dir);
            None
        }
    }
}

/// The pill was clicked: the update goes in place of the running program.
pub fn put_in_place() -> std::io::Result<PathBuf> {
    let (exe, dir) = (exe().ok_or(std::io::ErrorKind::NotFound)?, staging().ok_or(std::io::ErrorKind::NotFound)?);
    let (_, new) = update::staged(&dir, &Version::current()).ok_or(std::io::ErrorKind::NotFound)?;
    let swapped = update::swap(&exe, &new);
    update::clear(&dir);
    swapped.map(|()| exe)
}

/// Starts `exe` with `args`, apart from this process.
pub fn start(exe: &Path, args: impl IntoIterator<Item = std::ffi::OsString>) {
    let _ = std::process::Command::new(exe).args(args).spawn();
}

/// After the first frame: the daily check, on a worker, while the switch
/// is on. A change of the switch comes here too.
pub fn watch(hwnd: HWND, on: bool) {
    ON.store(on, Ordering::Relaxed);
    if !on || STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(url) = feed_url() else { return };
    let hwnd = crate::Handoff(hwnd);
    std::thread::spawn(move || {
        let hwnd = hwnd;
        // Well after the first frame, and never in the way of the launch.
        std::thread::sleep(if notsy_core::testing() { Duration::from_secs(2) } else { Duration::from_secs(20) });
        crate::background_priority();
        let Some(dir) = staging() else { return };
        loop {
            let now = notsy_core::session::now();
            let last = update::last_check(&dir);
            if ON.load(Ordering::Relaxed) && update::due(last, now) {
                if let Some(version) = fetch(&url, &dir, now) {
                    let version = Box::into_raw(Box::new(version.to_string()));
                    if unsafe { PostMessageW(Some(hwnd.0), WM_APP_UPDATE, WPARAM(0), LPARAM(version as isize)) }.is_err() {
                        drop(unsafe { Box::from_raw(version) });
                    }
                    return;
                }
            }
            // Look again in an hour (the rule says whether a check is due).
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

/// Asks the feed; a newer version is downloaded and checked, and kept in
/// `dir` until it goes in. Returns it once it is ready. A feed that did not
/// answer (offline) is asked again within the hour; one that did, in a day.
fn fetch(url: &str, dir: &Path, now: u64) -> Option<Version> {
    let json = crate::http::get(url, 16 * 1024)?;
    let _ = update::checked(dir, now);
    let feed = Feed::parse(std::str::from_utf8(&json).ok()?, url, notsy_core::testing())?;
    let current = Version::current();
    if feed.version <= current {
        return None;
    }
    if let Some((waiting, _)) = update::staged(dir, &current)
        && waiting == feed.version
    {
        return Some(waiting);
    }
    let bytes = crate::http::get(&feed.url, update::MOST)?;
    // Only what the feed promised is kept, and nothing else is written.
    if !feed.matches(&bytes) {
        return None;
    }
    update::stage(dir, &feed.version, &bytes).ok()?;
    Some(feed.version)
}

/// After the first frame of a start that followed an update: the program
/// replaced goes (once it has stopped running), and Settings > Apps learns
/// the new version.
pub fn tidy() {
    let Some(exe) = exe() else { return };
    let old = update::old_exe(&exe);
    if !old.exists() {
        return;
    }
    std::thread::spawn(move || {
        crate::background_priority();
        // The old process may take a moment to end; a note's own window
        // may keep it running longer (then the next start tries again).
        for _ in 0..40 {
            if std::fs::remove_file(&old).is_ok() || !old.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Some(dir) = exe.parent() {
            set_installed_version(dir);
        }
    });
}

/// The installer's entry in Settings > Apps, if it installed this copy
/// (its folder is `dir`): its version becomes this one.
fn set_installed_version(dir: &Path) {
    use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegGetValueW, RegOpenKeyExW, RegSetValueExW};
    use windows::core::{HSTRING, w};
    let key_name = HSTRING::from(update::uninstall_key());
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &key_name, None, KEY_QUERY_VALUE | KEY_SET_VALUE, &mut key) } != ERROR_SUCCESS {
        return;
    }
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let read = unsafe { RegGetValueW(key, None, w!("InstallLocation"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size)) };
    let at = String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]);
    if read == ERROR_SUCCESS && Path::new(at.trim_end_matches('\\')) == dir {
        let version: Vec<u8> = env!("CARGO_PKG_VERSION").encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        unsafe {
            let _ = RegSetValueExW(key, w!("DisplayVersion"), None, REG_SZ, Some(&version));
        }
    }
    unsafe {
        let _ = RegCloseKey(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_run_is_not_packaged() {
        assert!(!packaged(), "cargo test runs outside any package");
    }
}
