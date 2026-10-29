//! Kolada for the Mac updating itself from the website, as Windows does
//! (`kolada-win` `updates.rs`; the rules are `kolada_core::update`). Only
//! the website's build updates itself: an app bundle signed with Kolada's
//! Developer ID, not a dev build, and under test only with a feed of its
//! own (`KOLADA_UPDATE_FEED`).
//!
//! - Well after the first frame, a thread at background priority asks the
//!   Mac feed, at most once a day, whether there is a newer version. If so
//!   it downloads the zip of the new `Kolada.app` and checks its SHA-256,
//!   unpacks it (`ditto`), and checks its signature: Kolada's Developer ID
//!   and notarized (`codesign`, `spctl`). Only then is it staged and the
//!   window gets its pill. Nothing that fails a check is kept.
//! - Clicked, the pill saves everything, renames the running bundle aside
//!   (`.Kolada.old.app`, macOS lets a running app's folder be renamed), puts
//!   the new one in its place, quits, and the new one opens once this one
//!   has gone.
//! - Never clicked, the update goes in the same way at the next start,
//!   before the window.
//! - The next start removes the old bundle.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kolada_core::update::{self, Feed, Plan, Version};
use kolada_ui::scene::CheckState;

/// The switch in the settings, as last set.
static ON: AtomicBool = AtomicBool::new(true);
static STARTED: AtomicBool = AtomicBool::new(false);
/// The version waiting, for the window once it is woken.
static READY: Mutex<Option<String>> = Mutex::new(None);
/// How far Settings' "Check now" has got, for the window once it is woken.
static CHECKED: Mutex<Option<CheckState>> = Mutex::new(None);
/// One fetch at a time: the daily check and "Check now" never download
/// into the staging folder together.
static FETCHING: Mutex<()> = Mutex::new(());
/// This copy updates itself (`offered`, found off the main thread).
static OFFERED: AtomicBool = AtomicBool::new(false);

/// The requirement a new bundle's signature must meet: Apple's chain to
/// Kolada's Developer ID certificate (its team).
const TEAM: &str = "5GQ2248AFB";

/// The feed to ask: the website's; under test only one named by
/// `KOLADA_UPDATE_FEED` (a server on this machine), else none.
fn feed_url() -> Option<String> {
    if kolada_core::testing() {
        std::env::var("KOLADA_UPDATE_FEED").ok().filter(|u| !u.is_empty())
    } else {
        Some(update::FEED_MAC.to_string())
    }
}

/// The app bundle this runs from (`…/Kolada.app`), if it runs from one.
pub fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension().is_some_and(|e| e == "app") && exe.parent()?.ends_with("Contents/MacOS")).then(|| app.to_path_buf())
}

fn staging() -> Option<PathBuf> {
    kolada_core::app_data_dir().map(|d| update::staging(&d))
}

/// Whether a bundle's signature is Kolada's Developer ID (and, `notarized`,
/// that Gatekeeper takes it: notarized and stapled).
fn signed_by_us(app: &Path, notarized: bool) -> bool {
    let requirement = format!("=anchor apple generic and certificate leaf[subject.OU] = \"{TEAM}\"");
    let signed = Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict", "-R"]).arg(&requirement).arg(app).output().is_ok_and(|o| o.status.success());
    signed && (!notarized || Command::new("/usr/sbin/spctl").args(["--assess", "--type", "execute"]).arg(app).output().is_ok_and(|o| o.status.success()))
}

/// Whether this copy of Kolada updates itself: the website's (an app bundle
/// signed with Kolada's ID, in a folder it may write), not a dev build, and
/// under test only with a feed of its own.
pub fn offered() -> bool {
    let Some(app) = bundle() else { return false };
    if update::is_dev_build(&app) || feed_url().is_none() {
        return false;
    }
    let writable = app.parent().is_some_and(|d| unsafe { libc::access(path_c(d).as_ptr(), libc::W_OK) } == 0);
    writable && signed_by_us(&app, false)
}

fn path_c(p: &Path) -> std::ffi::CString {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap_or_default()
}

/// After the first frame, on a thread (checking the signature runs
/// `codesign`): whether this copy updates itself, then `wake`.
pub fn find_out(wake: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        unsafe {
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
        }
        tidy();
        if offered() {
            OFFERED.store(true, Ordering::Relaxed);
            wake();
        }
    });
}

/// What `find_out` found.
pub fn is_offered() -> bool {
    OFFERED.load(Ordering::Relaxed)
}

/// Before the window: an update downloaded before and never clicked goes
/// in, and the new app opens in this one's place (true: this one quits at
/// once). A bundle left by the last update is removed. Costs one look at a
/// file when there is nothing.
pub fn install_waiting() -> bool {
    let Some(dir) = staging() else { return false };
    if !dir.join("ready").exists() {
        return false;
    }
    let Some(app) = bundle() else { return false };
    let Some((_, new)) = update::staged_bundle(&dir, &Version::current()) else { return false };
    if update::is_dev_build(&app) {
        update::clear_bundle(&dir);
        return false;
    }
    let swapped = put(&app, &new).is_ok();
    update::clear_bundle(&dir);
    if swapped {
        open_when_gone(&app);
    }
    swapped
}

/// After the first frame: the bundle the last update set aside goes.
fn tidy() {
    if let Some(app) = bundle() {
        let old = update::old_bundle(&app);
        if old.exists() {
            let _ = std::fs::remove_dir_all(old);
        }
    }
}

/// `new` in place of the bundle at `app`: a rename, or, when they are on
/// different volumes, a copy beside the app first (`ditto`, which keeps the
/// signature whole), then the rename.
fn put(app: &Path, new: &Path) -> std::io::Result<()> {
    if update::swap_bundle(app, new).is_ok() {
        return Ok(());
    }
    let beside = app.with_file_name(".Kolada.new.app");
    let _ = std::fs::remove_dir_all(&beside);
    let copied = Command::new("/usr/bin/ditto").arg(new).arg(&beside).status().is_ok_and(|s| s.success());
    if !copied {
        let _ = std::fs::remove_dir_all(&beside);
        return Err(std::io::ErrorKind::Other.into());
    }
    let swapped = update::swap_bundle(app, &beside);
    if swapped.is_err() {
        let _ = std::fs::remove_dir_all(&beside);
    }
    swapped
}

/// Opens the app at `app` once this process has ended (a shell waits for
/// it), on the same note: the session says which. A test instance's new
/// app is one too, with its own data (`open` passes no environment on).
fn open_when_gone(app: &Path) {
    let pid = std::process::id();
    let script = format!("while kill -0 {pid} 2>/dev/null; do sleep 0.1; done; exec /usr/bin/open -n \"$@\"");
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    if kolada_core::testing() {
        for var in ["KOLADA_TEST_INSTANCE", "KOLADA_UPDATE_FEED", "KOLADA_VAULT", "HOME"] {
            if let Some(value) = std::env::var_os(var) {
                let mut pair = std::ffi::OsString::from(format!("{var}="));
                pair.push(value);
                args.extend(["--env".into(), pair]);
            }
        }
    }
    args.push(app.into());
    args.extend(["--args".into(), "--updated".into()]);
    let _ = Command::new("/bin/sh").arg("-c").arg(script).arg("kolada-update").args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

/// The pill was clicked (everything saved): the update goes in place of the
/// running app, which then quits and opens again. False when it could not
/// go in (it is dropped, and the next check fetches it again).
pub fn put_in_place() -> bool {
    let (Some(app), Some(dir)) = (bundle(), staging()) else { return false };
    let Some((_, new)) = update::staged_bundle(&dir, &Version::current()) else { return false };
    let swapped = put(&app, &new).is_ok();
    update::clear_bundle(&dir);
    if swapped {
        open_when_gone(&app);
    }
    swapped
}

/// The version that waits, once the check found and staged it (taken once).
pub fn take_ready() -> Option<String> {
    READY.lock().ok()?.take()
}
/// How far "Check now" has got, once (`check_now`'s `said`).
pub fn take_checked() -> Option<CheckState> {
    CHECKED.lock().ok()?.take()
}

/// Settings' "Check now": the feed is asked at once, whatever the daily
/// rule and the switch say. `said` is called as it gets further (on its
/// way, up to date, could not check: `take_checked`), `ready` once a newer
/// version waits (`take_ready`: the pill, as from the daily check).
pub fn check_now(said: impl Fn() + Send + 'static, ready: impl Fn() + Send + 'static) {
    let Some(url) = feed_url() else { return };
    std::thread::spawn(move || {
        let say = |state: CheckState| {
            *CHECKED.lock().unwrap() = Some(state);
            said();
        };
        let Some(dir) = staging() else { return say(CheckState::Failed) };
        let found = |v: &Version| say(CheckState::Downloading(v.to_string()));
        match fetch(&url, &dir, kolada_core::session::now(), &found) {
            Some(Got::Update(version)) => {
                *READY.lock().unwrap() = Some(version.to_string());
                ready();
            }
            Some(Got::Nothing) => say(CheckState::UpToDate),
            None => say(CheckState::Failed),
        }
    });
}

/// After the first frame: the daily check, on a thread, while the switch is
/// on; `wake` is called once an update waits. A change of the switch comes
/// here too.
pub fn watch(on: bool, wake: impl Fn() + Send + 'static) {
    ON.store(on, Ordering::Relaxed);
    if !on || STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(url) = feed_url() else { return };
    std::thread::spawn(move || {
        // Well after the first frame, and never in the way of the launch.
        std::thread::sleep(if kolada_core::testing() { Duration::from_secs(2) } else { Duration::from_secs(20) });
        unsafe {
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
        }
        let Some(dir) = staging() else { return };
        // One downloaded before, never clicked, and still newer: the pill at once.
        if let Some((version, _)) = update::staged_bundle(&dir, &Version::current()) {
            *READY.lock().unwrap() = Some(version.to_string());
            return wake();
        }
        loop {
            let now = kolada_core::session::now();
            if ON.load(Ordering::Relaxed) && update::due(update::last_check(&dir), now) {
                if let Some(Got::Update(version)) = fetch(&url, &dir, now, &|_| {}) {
                    *READY.lock().unwrap() = Some(version.to_string());
                    return wake();
                }
            }
            // Look again in an hour (the rule says whether a check is due).
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

/// Asks the feed; a newer version is downloaded, checked, unpacked, its
/// signature checked, and staged in `dir`. A feed that did not answer
/// (offline) is asked again within the hour; one that did, in a day.
/// `found` hears of a newer version before it downloads. None: the feed
/// did not answer, or what came failed a check.
fn fetch(url: &str, dir: &Path, now: u64, found: &dyn Fn(&Version)) -> Option<Got> {
    let _one = FETCHING.lock().unwrap_or_else(|e| e.into_inner());
    let json = super::web::get(url, 16 * 1024)?;
    let _ = update::checked(dir, now);
    let feed = Feed::parse(std::str::from_utf8(&json).ok()?, url, kolada_core::testing())?;
    if feed.plan(&Version::current(), true) != Plan::Update {
        return Some(Got::Nothing);
    }
    found(&feed.version);
    let zip = super::web::get(&feed.url, update::MOST)?;
    if !feed.matches(&zip) {
        return None;
    }
    let app = unpack(dir, &zip, !kolada_core::testing())?;
    let staged = update::stage_bundle(dir, &feed.version, &app);
    let _ = std::fs::remove_dir_all(dir.join("unpacked"));
    staged.ok()?;
    Some(Got::Update(feed.version))
}

/// What a check brought.
#[derive(Debug, PartialEq)]
pub enum Got {
    /// A newer version, staged.
    Update(Version),
    /// The feed answered: nothing newer.
    Nothing,
}

/// The zip's `Kolada.app`, unpacked into `dir/unpacked` and its signature
/// checked (`notarized` too, outside tests); None, and nothing kept, if it
/// is not Kolada's.
fn unpack(dir: &Path, zip: &[u8], notarized: bool) -> Option<PathBuf> {
    let (file, out) = (dir.join("next.zip"), dir.join("unpacked"));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).ok()?;
    std::fs::write(&file, zip).ok()?;
    let unzipped = Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(&file).arg(&out).status().is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&file);
    let app = out.join("Kolada.app");
    if unzipped && app.join("Contents").is_dir() && signed_by_us(&app, notarized) {
        return Some(app);
    }
    let _ = std::fs::remove_dir_all(&out);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_binary_is_no_bundle_and_never_updates_itself() {
        // `cargo test` runs a bare binary under target/: no bundle, no updater.
        assert_eq!(bundle(), None);
        assert!(!offered());
        assert!(!install_waiting() || bundle().is_some());
    }

    #[test]
    fn a_bundle_not_signed_by_kolada_is_not_unpacked() {
        let dir = std::env::temp_dir().join(format!("kolada-mac-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A zip of an app signed ad hoc (anyone's): refused, nothing kept.
        let app = dir.join("src").join("Kolada.app");
        std::fs::create_dir_all(app.join("Contents").join("MacOS")).unwrap();
        std::fs::copy("/usr/bin/true", app.join("Contents").join("MacOS").join("Kolada")).unwrap();
        std::fs::write(app.join("Contents").join("Info.plist"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>Kolada</string><key>CFBundleIdentifier</key><string>app.kolada.test</string></dict></plist>").unwrap();
        assert!(Command::new("codesign").args(["--force", "--sign", "-"]).arg(&app).status().unwrap().success());
        let zip = dir.join("Kolada.zip");
        assert!(Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(&app).arg(&zip).status().unwrap().success());
        let staging = dir.join("update");
        std::fs::create_dir_all(&staging).unwrap();
        assert_eq!(unpack(&staging, &std::fs::read(&zip).unwrap(), false), None);
        assert!(!staging.join("unpacked").exists() && !staging.join("next.zip").exists());
        assert!(!signed_by_us(&app, false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
