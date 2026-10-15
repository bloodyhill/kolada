//! Kolada updating itself from the website (the rules are in
//! `kolada_core::update`). Never in the Microsoft Store's build: the Store
//! updates it, and its policy forbids an app doing so itself.
//!
//! - After the first frame and a while longer, a worker at background
//!   priority asks the feed, at most once a day, whether there is a newer
//!   version. If so it downloads it into the app's data folder and checks
//!   its SHA-256; only then does the window get its pill. Nothing that fails
//!   the check is kept.
//! - Clicked, the pill saves everything, renames the running `kolada.exe`
//!   to `kolada.old.exe` (Windows lets a running program be renamed), moves
//!   the new one into its place, and the app quits through its usual path
//!   and starts the new one with `--updated`, on the same note.
//! - Never clicked, the update goes in the same way at the next start,
//!   before the window: two renames, then the new program starts in this
//!   one's place.
//! - The next start deletes `kolada.old.exe`.
//! - The writing helper (`kolada-ai.exe`) comes with the program when the
//!   feed lists it, checked the same way, and goes in beside it (a running
//!   one steps aside to `kolada-ai.old.exe`). A copy without one (updated
//!   by a version that knew only the program) fetches it on its own, once
//!   a start, whatever the daily rule says, and writing help starts on it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kolada_core::update::{self, Feed, Plan, Version};
use windows::Win32::Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_SUCCESS, HWND, LPARAM, WPARAM};
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::window::{WM_APP_HELPER, WM_APP_UPDATE};

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
/// `KOLADA_UPDATE_FEED` (a server on this machine), else none.
fn feed_url() -> Option<String> {
    if kolada_core::testing() {
        std::env::var("KOLADA_UPDATE_FEED").ok().filter(|u| !u.is_empty())
    } else {
        Some(update::FEED.to_string())
    }
}

fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

fn staging() -> Option<PathBuf> {
    kolada_core::app_data_dir().map(|d| update::staging(&d))
}

/// Whether this copy of Kolada updates itself: not the Store's, not a dev
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
            put_helper(&exe, &dir);
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
    if swapped.is_ok() {
        put_helper(&exe, &dir);
    }
    update::clear(&dir);
    swapped.map(|()| exe)
}

/// The helper staged with an update goes in beside the program: in place
/// of the one there (which, running, steps aside like the program), or
/// where there was none. A helper that cannot go in is fetched again later.
fn put_helper(exe: &Path, dir: &Path) {
    let Some(new) = update::staged_helper_file(dir) else { return };
    let to = exe.with_file_name(crate::assist::HELPER);
    let _ = if to.exists() { update::swap(&to, &new) } else { std::fs::rename(&new, &to) };
}

/// Whether the writing helper is beside this program.
fn helper_here() -> bool {
    exe().is_some_and(|e| e.with_file_name(crate::assist::HELPER).is_file())
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
        std::thread::sleep(if kolada_core::testing() { Duration::from_secs(2) } else { Duration::from_secs(20) });
        crate::background_priority();
        let Some(dir) = staging() else { return };
        // A copy without its helper asks once this start, due or not.
        let mut helper_asked = false;
        loop {
            let now = kolada_core::session::now();
            let last = update::last_check(&dir);
            let missing = !helper_asked && !helper_here();
            if ON.load(Ordering::Relaxed) && (update::due(last, now) || missing) {
                helper_asked |= missing;
                match fetch(&url, &dir, now) {
                    Some(Got::Update(version)) => {
                        let version = Box::into_raw(Box::new(version.to_string()));
                        if unsafe { PostMessageW(Some(hwnd.0), WM_APP_UPDATE, WPARAM(0), LPARAM(version as isize)) }.is_err() {
                            drop(unsafe { Box::from_raw(version) });
                        }
                        return;
                    }
                    Some(Got::Helper) => {
                        let _ = unsafe { PostMessageW(Some(hwnd.0), WM_APP_HELPER, WPARAM(0), LPARAM(0)) };
                    }
                    None => {}
                }
            }
            // Look again in an hour (the rule says whether a check is due).
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

/// What a check brought.
enum Got {
    /// A newer version, ready to go in.
    Update(Version),
    /// This version's missing writing helper, now in place.
    Helper,
}

/// Asks the feed; a newer version (and its helper) is downloaded and
/// checked, and kept in `dir` until it goes in. This version's helper, when
/// it is missing, is fetched and put beside the program at once. A feed
/// that did not answer (offline) is asked again within the hour; one that
/// did, in a day.
fn fetch(url: &str, dir: &Path, now: u64) -> Option<Got> {
    let json = crate::http::get(url, 16 * 1024)?;
    let _ = update::checked(dir, now);
    let feed = Feed::parse(std::str::from_utf8(&json).ok()?, url, kolada_core::testing())?;
    let current = Version::current();
    match feed.plan(&current, helper_here()) {
        Plan::Nothing => None,
        Plan::Helper => {
            let helper = feed.helper.as_ref()?;
            let bytes = crate::http::get(&helper.url, update::MOST)?;
            if !helper.matches(&bytes) {
                return None;
            }
            update::put(&exe()?.with_file_name(crate::assist::HELPER), &bytes).ok()?;
            Some(Got::Helper)
        }
        Plan::Update => {
            if let Some((waiting, _)) = update::staged(dir, &current)
                && waiting == feed.version
            {
                return Some(Got::Update(waiting));
            }
            let bytes = crate::http::get(&feed.url, update::MOST)?;
            // Only what the feed promised is kept, and nothing else is written.
            if !feed.matches(&bytes) {
                return None;
            }
            // A helper that does not come (or is not what was promised) is
            // left out: the new version fetches it itself.
            let helper = feed.helper.as_ref().and_then(|h| crate::http::get(&h.url, update::MOST).filter(|b| h.matches(b)));
            update::stage(dir, &feed.version, &bytes, helper.as_deref()).ok()?;
            Some(Got::Update(feed.version))
        }
    }
}

/// After the first frame of a start that followed an update: the program
/// replaced goes (once it has stopped running), and Settings > Apps learns
/// the new version.
pub fn tidy() {
    let Some(exe) = exe() else { return };
    let old = update::old_exe(&exe);
    // The helper that stepped aside goes too, whenever it can.
    let _ = std::fs::remove_file(update::old_exe(&exe.with_file_name(crate::assist::HELPER)));
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
