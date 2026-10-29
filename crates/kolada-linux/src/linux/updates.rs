//! Kolada for Linux updating itself from the website, as Windows and the Mac
//! do (`kolada-win` `updates.rs`, `kolada-mac` `updates.rs`; the rules are
//! `kolada_core::update`). Only the per-user install from the .tar.gz
//! updates itself (`./install.sh` puts it in `~/.local/lib/kolada`, or it
//! runs where it was unpacked): never a package's (`/usr`, which its
//! package manager updates), an AppImage, a dev build, nor a test instance
//! without a feed of its own (`KOLADA_UPDATE_FEED`).
//!
//! - Well after the first frame, a thread at background priority asks the
//!   Linux feed (`update::FEED_LINUX`), at most once a day. A newer version:
//!   the tarball is downloaded (curl, over HTTPS, from the feed's own site)
//!   and checked against the feed's SHA-256 and size; `kolada` (and
//!   `kolada-ai`, when it has one) is read out of it and must be a program
//!   for this machine. Only then is it staged and the window gets its pill.
//! - Clicked, the pill saves everything, puts the new program in place of
//!   the running one (a rename over it: Linux keeps running the old file
//!   until it ends), and the app starts again as the new one, on the same
//!   note (`exec`, the same process, `--updated`).
//! - Never clicked, it goes in the same way at the next start, before the
//!   window.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
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

/// The program, and the writing helper beside it.
const PROGRAM: &str = "kolada";
const HELPER: &str = "kolada-ai";

/// The feed to ask: the website's; under test only one named by
/// `KOLADA_UPDATE_FEED` (a server on this machine), else none.
fn feed_url() -> Option<String> {
    if kolada_core::testing() {
        std::env::var("KOLADA_UPDATE_FEED").ok().filter(|u| !u.is_empty())
    } else {
        Some(update::feed_linux(std::env::consts::ARCH))
    }
}

fn staging() -> Option<PathBuf> {
    kolada_core::app_data_dir().map(|d| update::staging(&d))
}

/// The program that runs (links followed: `~/.local/bin/kolada` is one).
fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Whether the copy of Kolada at `exe` updates itself: not a dev build, not
/// a package's (under `/usr`), not an AppImage (`appimage`: it runs from
/// its own image, read only), and in a folder this person may write.
pub fn updates_itself(exe: &Path, appimage: bool) -> bool {
    if update::is_dev_build(exe) || exe.starts_with("/usr") || appimage || exe.file_name().is_none_or(|n| n != PROGRAM) {
        return false;
    }
    exe.parent().is_some_and(writable)
}

fn writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return false };
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// Whether this copy updates itself (Settings then offers the switch).
/// Costs a look at the program's folder: after the first frame.
pub fn offered() -> bool {
    feed_url().is_some() && exe().is_some_and(|e| updates_itself(&e, std::env::var_os("APPIMAGE").is_some()))
}

/// Whether `bytes` are a program this machine runs: 64-bit little-endian
/// ELF for its processor.
pub fn runs_here(bytes: &[u8]) -> bool {
    let machine: u16 = match std::env::consts::ARCH {
        "x86_64" => 62,
        "aarch64" => 183,
        "riscv64" => 243,
        _ => return false,
    };
    bytes.len() > 64 && bytes.starts_with(b"\x7fELF") && bytes[4] == 2 && bytes[5] == 1 && u16::from_le_bytes([bytes[18], bytes[19]]) == machine
}

/// The files of a .tar.gz that sit in its one top folder under `names`
/// (`kolada-0.5.0-linux-x86_64/kolada`), each at most `most` bytes, in the
/// order asked. Plain files only (no links), read in Rust: nothing is
/// unpacked onto the disk.
pub fn members(targz: &[u8], names: &[&str], most: usize) -> io::Result<Vec<Option<Vec<u8>>>> {
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(targz).take(update::MOST as u64 * 2).read_to_end(&mut tar)?;
    let mut found: Vec<Option<Vec<u8>>> = vec![None; names.len()];
    let mut at = 0;
    let mut long_name: Option<String> = None;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let field = |from: usize, len: usize| {
            let f = &header[from..from + len];
            let end = f.iter().position(|&b| b == 0).unwrap_or(len);
            String::from_utf8_lossy(&f[..end]).into_owned()
        };
        let size = usize::from_str_radix(field(124, 12).trim(), 8).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        let kind = header[156];
        let body = at + 512;
        let next = body + size.div_ceil(512) * 512;
        if body + size > tar.len() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let data = &tar[body..body + size];
        // GNU tar's long name, for the entry after it.
        if kind == b'L' {
            long_name = Some(String::from_utf8_lossy(data).trim_end_matches('\0').to_string());
            at = next;
            continue;
        }
        let name = long_name.take().unwrap_or_else(|| {
            let (prefix, name) = (field(345, 155), field(0, 100));
            if header[257..262] == *b"ustar" && !prefix.is_empty() { format!("{prefix}/{name}") } else { name }
        });
        let name = name.trim_start_matches("./");
        if matches!(kind, b'0' | 0)
            && let Some((top, file)) = name.split_once('/')
            && !top.is_empty()
            && !top.starts_with('.')
            && let Some(i) = names.iter().position(|n| *n == file)
        {
            if size > most {
                return Err(io::ErrorKind::InvalidData.into());
            }
            found[i] = Some(data.to_vec());
        }
        at = next;
    }
    Ok(found)
}

/// Out of a verified tarball: the program, and its helper if it has one,
/// both programs for this machine. None if it holds no program.
pub fn programs(targz: &[u8]) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    let mut found = members(targz, &[PROGRAM, HELPER], update::MOST).ok()?;
    let helper = found.pop()?.filter(|h| runs_here(h));
    let program = found.pop()??;
    runs_here(&program).then_some((program, helper))
}

/// Writes `from` (a staged file) over `to`, whole or not at all: copied
/// beside it, made a program, then renamed over it. A running program can
/// be replaced so: it keeps running from the file it opened.
fn put(to: &Path, from: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let name = to.file_name().ok_or(io::ErrorKind::InvalidInput)?.to_string_lossy();
    let part = to.with_file_name(format!(".{name}.part"));
    let done = (|| {
        std::fs::copy(from, &part)?;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))?;
        std::fs::File::open(&part)?.sync_all()?;
        std::fs::rename(&part, to)
    })();
    if done.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    done
}

/// Puts the update staged in `dir` in place of the program at `exe` (and
/// its helper beside it, when it came with one), then forgets it. Whether
/// the program went in.
pub fn install(exe: &Path, dir: &Path) -> bool {
    let Some((_, next)) = update::staged(dir, &Version::current()) else { return false };
    if let Some(helper) = update::staged_helper_file(dir) {
        let _ = put(&exe.with_file_name(HELPER), &helper);
    }
    let done = put(exe, &next).is_ok();
    update::clear(dir);
    done
}

/// Before the window: an update downloaded before and never clicked goes
/// in, and the program to start in this one's place is said (`restart`).
/// Costs one look at a file when nothing waits.
pub fn install_waiting() -> Option<PathBuf> {
    let dir = staging()?;
    if !dir.join("ready").exists() {
        return None;
    }
    let exe = exe()?;
    if !offered() {
        update::clear(&dir);
        return None;
    }
    install(&exe, &dir).then_some(exe)
}

/// The pill was clicked (everything saved): the update goes in place of the
/// running program. The program to start again, or None when it could not
/// go in (it is dropped, and the next check fetches it again).
pub fn put_in_place() -> Option<PathBuf> {
    let (exe, dir) = (exe()?, staging()?);
    install(&exe, &dir).then_some(exe)
}

/// Becomes the program at `exe`, with this launch's arguments and
/// `--updated`: the same process, so the next launch finds it as before.
/// Returns only if it could not.
pub fn restart(exe: &Path) {
    use std::os::unix::process::CommandExt;
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if !args.iter().any(|a| a == "--updated") {
        args.push("--updated".into());
    }
    let e = std::process::Command::new(exe).args(args).exec();
    eprintln!("kolada: could not start the update ({e})");
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
        match fetch(&url, &dir, kolada_core::session::now(), kolada_core::testing(), &found) {
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
        kolada_app::services::Priority::lower_thread(&super::services::LinuxPriority);
        let Some(dir) = staging() else { return };
        // Tidy what a broken-off update left beside the program.
        if let Some(exe) = exe() {
            for name in [PROGRAM, HELPER] {
                let _ = std::fs::remove_file(exe.with_file_name(format!(".{name}.part")));
            }
        }
        // One downloaded before, never clicked, and still newer: the pill at once.
        if let Some((version, _)) = update::staged(&dir, &Version::current()) {
            *READY.lock().unwrap() = Some(version.to_string());
            return wake();
        }
        loop {
            let now = kolada_core::session::now();
            if ON.load(Ordering::Relaxed) && update::due(update::last_check(&dir), now) {
                if let Some(Got::Update(version)) = fetch(&url, &dir, now, kolada_core::testing(), &|_| {}) {
                    *READY.lock().unwrap() = Some(version.to_string());
                    return wake();
                }
            }
            // Look again in an hour (the rule says whether a check is due).
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

/// Asks the feed at `url`; a newer version is downloaded, checked, its
/// programs read out and staged in `dir`. A feed that did not answer
/// (offline) is asked again within the hour; one that did, in a day.
/// `local`: a server on this machine may serve it over plain HTTP (tests).
/// `found` hears of a newer version before it downloads. None: the feed
/// did not answer, or what came failed a check.
pub fn fetch(url: &str, dir: &Path, now: u64, local: bool, found: &dyn Fn(&Version)) -> Option<Got> {
    use kolada_app::services::Web;
    let _one = FETCHING.lock().unwrap_or_else(|e| e.into_inner());
    let json = super::web::LinuxWeb.get(url, 16 * 1024)?;
    let _ = update::checked(dir, now);
    let feed = Feed::parse(std::str::from_utf8(&json).ok()?, url, local)?;
    if feed.plan(&Version::current(), true) != Plan::Update {
        return Some(Got::Nothing);
    }
    found(&feed.version);
    let tarball = super::web::get_big(&feed.url, feed.size)?;
    if !feed.matches(&tarball) {
        return None;
    }
    let (program, helper) = programs(&tarball)?;
    update::stage(dir, &feed.version, &program, helper.as_deref()).ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-linux-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A tarball as `tools/linux-release.sh` makes it: one top folder with
    /// the program (and here a helper and the rest beside it).
    fn tarball(dir: &Path, version: &str, program: &[u8], helper: Option<&[u8]>) -> Vec<u8> {
        let top = format!("kolada-{version}-linux-{}", std::env::consts::ARCH);
        let src = dir.join("src");
        std::fs::create_dir_all(src.join(&top).join("icons")).unwrap();
        std::fs::write(src.join(&top).join("kolada"), program).unwrap();
        std::fs::write(src.join(&top).join("install.sh"), "#!/bin/sh\n").unwrap();
        std::fs::write(src.join(&top).join("icons").join("kolada"), "not this one").unwrap();
        if let Some(h) = helper {
            std::fs::write(src.join(&top).join("kolada-ai"), h).unwrap();
        }
        let out = dir.join("kolada.tar.gz");
        assert!(Command::new("tar").arg("-C").arg(&src).args(["--owner=0", "--group=0", "-czf"]).arg(&out).arg(&top).status().unwrap().success());
        std::fs::read(out).unwrap()
    }

    fn a_program() -> Vec<u8> {
        std::fs::read("/bin/true").unwrap()
    }

    #[test]
    fn only_the_per_user_install_updates_itself() {
        let dir = scratch("which");
        let lib = dir.join("home/.local/lib/kolada");
        std::fs::create_dir_all(&lib).unwrap();
        assert!(updates_itself(&lib.join("kolada"), false), "the tarball's install");
        assert!(!updates_itself(&lib.join("kolada"), true), "an AppImage");
        assert!(!updates_itself(Path::new("/usr/lib/kolada/kolada"), false), "the .deb's: its package manager's");
        assert!(!updates_itself(Path::new("/home/me/kolada/target/release/kolada"), false), "a dev build");
        assert!(!updates_itself(&lib.join("kolada-ai"), false), "not the program");
        assert!(!updates_itself(Path::new("/proc/kolada"), false), "a folder it may not write");
        // `cargo test` runs a binary under target/: never offered.
        assert!(!offered());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_programs_are_read_out_of_the_tarball_and_must_run_here() {
        let dir = scratch("tar");
        let program = a_program();
        assert!(runs_here(&program), "this machine's own");
        assert!(!runs_here(b"#!/bin/sh\necho hi\n"));
        let mut other = program.clone();
        other[18] = 40; // ARM's 32-bit
        assert!(!runs_here(&other));
        let t = tarball(&dir, "9.9.9", &program, Some(&program));
        assert_eq!(programs(&t), Some((program.clone(), Some(program.clone()))));
        let t = tarball(&scratch("tar-alone"), "9.9.9", &program, None);
        assert_eq!(programs(&t), Some((program.clone(), None)), "no helper yet");
        let t = tarball(&scratch("tar-script"), "9.9.9", b"#!/bin/sh\nrm -rf ~\n", None);
        assert_eq!(programs(&t), None, "not a program for this machine");
        assert_eq!(programs(b"not a tarball"), None);
        assert_eq!(programs(&t[..t.len() / 2]), None, "cut short");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_update_comes_from_a_feed_checked_and_goes_in_over_the_running_program() {
        let dir = scratch("end-to-end");
        let program = a_program();
        let tar = tarball(&dir, "9.9.9", &program, Some(&program));
        let files = super::super::web::loopback::Files::default();
        let site = super::super::web::loopback::serve(files.clone());
        let feed = |version: &str, sha: &str, size: usize| format!("{{\"version\": \"{version}\", \"url\": \"{site}/kolada-{version}-linux.tar.gz\", \"sha256\": \"{sha}\", \"size\": {size}}}");
        let sha = update::hex(&update::sha256(&tar));
        files.lock().unwrap().insert("/kolada-9.9.9-linux.tar.gz".into(), tar.clone());
        let staging = dir.join("update");
        let url = format!("{site}/linux.json");

        // What Settings' "Check now" hears before the download.
        let heard = std::cell::RefCell::new(Vec::new());
        let found = |v: &Version| heard.borrow_mut().push(v.to_string());
        // Not newer: nothing fetched ("Up to date"), but the check is noted.
        files.lock().unwrap().insert("/linux.json".into(), feed("0.0.1", &sha, tar.len()).into_bytes());
        assert_eq!(fetch(&url, &staging, 100, true, &found), Some(Got::Nothing));
        assert_eq!(update::last_check(&staging), Some(100));
        assert!(heard.borrow().is_empty());
        // Over plain HTTP from this machine only under test ("Couldn't check").
        files.lock().unwrap().insert("/linux.json".into(), feed("9.9.9", &sha, tar.len()).into_bytes());
        assert_eq!(fetch(&url, &staging, 200, false, &found), None);
        // The wrong hash: refused, nothing staged, after "Downloading 9.9.9".
        let wrong = update::hex(&update::sha256(b"another"));
        files.lock().unwrap().insert("/linux.json".into(), feed("9.9.9", &wrong, tar.len()).into_bytes());
        assert_eq!(fetch(&url, &staging, 300, true, &found), None);
        assert_eq!(update::staged(&staging, &Version::current()), None);
        assert_eq!(*heard.borrow(), ["9.9.9"]);
        // The right one: staged, the program and its helper.
        files.lock().unwrap().insert("/linux.json".into(), feed("9.9.9", &sha, tar.len()).into_bytes());
        assert_eq!(fetch(&url, &staging, 400, true, &found), Some(Got::Update(Version::parse("9.9.9").unwrap())));
        assert!(update::staged(&staging, &Version::current()).is_some() && update::staged_helper_file(&staging).is_some());

        // In place of the "running" program, beside which the helper goes.
        let lib = dir.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        let exe = lib.join("kolada");
        std::fs::write(&exe, b"the old one").unwrap();
        let running = std::fs::File::open(&exe).unwrap();
        assert!(install(&exe, &staging));
        assert_eq!(std::fs::read(&exe).unwrap(), program);
        assert_eq!(std::fs::read(lib.join("kolada-ai")).unwrap(), program);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755);
        let mut old = String::new();
        let _ = (&running).read_to_string(&mut old);
        assert_eq!(old, "the old one", "the running program keeps its file");
        assert!(Command::new(&exe).status().unwrap().success(), "and the new one runs");
        assert_eq!(update::staged(&staging, &Version::current()), None, "nothing waits after");
        assert!(!install(&exe, &staging), "nor goes in twice");
        assert_eq!(std::fs::read_dir(&lib).unwrap().count(), 2, "no part file left: {:?}", std::fs::read_dir(&lib).unwrap().collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(dir);
    }
}
