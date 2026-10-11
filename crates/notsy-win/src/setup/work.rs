//! What installing and removing do, with no window: files, shortcuts, the
//! Settings > Apps entry, `.md` files, starting with Windows.
//!
//! Everything is per user (HKCU, `%LOCALAPPDATA%`): no admin. A test run
//! (`NOTSY_TEST_INSTANCE`) keeps apart from a real install: its own names in
//! the registry, folders from the environment (point `LOCALAPPDATA`,
//! `APPDATA` and `USERPROFILE` elsewhere), and never the per-program key
//! `Applications\notsy.exe`, which a real install shares.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use notsy_core::found::{Found, Kind};
use notsy_core::pack;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_NONE, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Programs, IShellLinkW, KF_FLAG_DEFAULT, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify, SHGetKnownFolderPath, ShellLink,
};
use windows::core::{GUID, HSTRING, Interface, PCWSTR};

pub const APP: &str = "notsy.exe";

fn testing() -> bool {
    notsy_core::testing()
}

/// What Settings > Apps and the shortcuts call it.
pub fn name() -> &'static str {
    if testing() { "Notsy (test)" } else { "Notsy" }
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn known(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = p.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(p.0 as *const _));
        path
    }
}

/// `%LOCALAPPDATA%\Programs\Notsy`: per user, no admin.
pub fn install_dir() -> Option<PathBuf> {
    Some(env_dir("LOCALAPPDATA")?.join("Programs").join("Notsy"))
}

fn start_menu() -> Option<PathBuf> {
    if testing() {
        return Some(env_dir("APPDATA")?.join("Microsoft").join("Windows").join("Start Menu").join("Programs"));
    }
    known(&FOLDERID_Programs)
}

fn desktop() -> Option<PathBuf> {
    if testing() { Some(env_dir("USERPROFILE")?.join("Desktop")) } else { known(&FOLDERID_Desktop) }
}

/// The folders called Documents: the shell's (moved to OneDrive, say) and
/// the profile's, where Notsy makes its own folder.
pub fn documents() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let profile = env_dir("USERPROFILE").map(|h| h.join("Documents"));
    let shell = if testing() { None } else { known(&FOLDERID_Documents) };
    for d in [shell, profile].into_iter().flatten() {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

fn shortcut_name() -> String {
    format!("{}.lnk", name())
}

/// The three choices: a desktop shortcut, `.md` files opening in Notsy, and
/// starting with Windows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Choices {
    pub desktop: bool,
    pub files: bool,
    pub startup: bool,
}

impl Choices {
    /// What a first install offers: everything on.
    pub const FIRST: Choices = Choices { desktop: true, files: true, startup: true };

    pub fn as_array(self) -> [bool; 3] {
        [self.desktop, self.files, self.startup]
    }

    pub fn from_array(a: [bool; 3]) -> Choices {
        Choices { desktop: a[0], files: a[1], startup: a[2] }
    }
}

/// An install found on this machine (by its Settings > Apps entry).
pub struct Installed {
    pub version: String,
    pub choices: Choices,
}

pub fn installed() -> Option<Installed> {
    let key = Key::open(notsy_core::update::uninstall_key())?;
    let dir = PathBuf::from(key.get_str("InstallLocation")?);
    if !dir.join(APP).is_file() {
        return None;
    }
    let on = |name: &str, default: bool| key.get_dword(name).map_or(default, |v| v != 0);
    let choices = Choices { desktop: on("NotsyDesktop", false), files: on("NotsyOpensMarkdown", false), startup: on("NotsyStartsWithWindows", false) };
    Some(Installed { version: key.get_str("DisplayVersion").unwrap_or_default(), choices })
}

/// Where the notes go, as chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// A new folder (Documents\Notsy), the welcome note in it.
    New(PathBuf),
    /// A folder of Markdown, opened as it is (Notsy's own from before too).
    Open(PathBuf),
    /// An export, imported into a new Documents\Notsy.
    Import(PathBuf),
    /// Any folder, once chosen.
    Other(Option<PathBuf>),
}

impl Pick {
    pub fn from_found(f: &Found) -> Pick {
        if f.kind.opens_in_place() { Pick::Open(f.path.clone()) } else { Pick::Import(f.path.clone()) }
    }
}

/// What the installed Notsy is started with, the first time: nothing (a
/// first run: the welcome note in a new Documents\Notsy), the vault to open,
/// or the export to import.
pub fn launch_args(pick: &Pick) -> Vec<OsString> {
    match pick {
        Pick::New(dir) if !dir.exists() => Vec::new(),
        Pick::New(dir) | Pick::Open(dir) | Pick::Other(Some(dir)) => vec!["--vault".into(), dir.into()],
        Pick::Import(file) => vec![file.into()],
        Pick::Other(None) => Vec::new(),
    }
}

/// A path as the user knows it: under their profile, from there
/// (`Documents\Work`).
pub fn short(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if !rest.as_os_str().is_empty() => rest.display().to_string(),
        _ => path.display().to_string(),
    }
}

fn notes_phrase(n: usize) -> String {
    let count = if n >= notsy_core::found::MOST_COUNTED { format!("{n}+") } else { n.to_string() };
    if n == 1 { "1 note".into() } else { format!("{count} notes") }
}

/// A place found, as the installer lists it: its name and the line under it.
pub fn describe(f: &Found, home: Option<&Path>, new_folder: &Path) -> (String, String) {
    let at = short(&f.path, home);
    let detail = match f.kind {
        Kind::NotesExport => format!("{at} \u{00B7} imported into {}", short(new_folder, home)),
        Kind::Notsy => format!("{at} \u{00B7} {}", notes_phrase(f.notes)),
        _ => format!("{at} \u{00B7} {} \u{00B7} opens as it is", notes_phrase(f.notes)),
    };
    let name = match f.kind {
        Kind::Obsidian | Kind::Logseq | Kind::Markdown => format!("{} \u{00B7} {}", f.kind.name(), f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        _ => f.kind.name().to_string(),
    };
    (name, detail)
}

// The registry, per user.

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn wide(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

impl Key {
    fn create(path: &str) -> Option<Key> {
        let mut k = HKEY::default();
        let s = unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE | KEY_READ, None, &mut k, None) };
        (s == ERROR_SUCCESS).then_some(Key(k))
    }

    fn open(path: &str) -> Option<Key> {
        let mut k = HKEY::default();
        let s = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, KEY_READ | KEY_WRITE, &mut k) };
        (s == ERROR_SUCCESS).then_some(Key(k))
    }

    fn set(&self, name: Option<&str>, kind: REG_VALUE_TYPE, data: &[u8]) -> bool {
        let name = name.map(HSTRING::from);
        let pname = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
        unsafe { RegSetValueExW(self.0, pname, None, kind, Some(data)) == ERROR_SUCCESS }
    }

    fn set_str(&self, name: Option<&str>, value: &str) -> bool {
        self.set(name, REG_SZ, &wide(value))
    }

    fn set_dword(&self, name: &str, value: u32) -> bool {
        self.set(Some(name), REG_DWORD, &value.to_le_bytes())
    }

    fn get_str(&self, name: &str) -> Option<String> {
        let mut buf = vec![0u16; 2048];
        let mut size = (buf.len() * 2) as u32;
        let s = unsafe { RegGetValueW(self.0, None, &HSTRING::from(name), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size)) };
        (s == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }

    fn get_dword(&self, name: &str) -> Option<u32> {
        let mut v = 0u32;
        let mut size = 4u32;
        let s = unsafe { RegGetValueW(self.0, None, &HSTRING::from(name), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut size)) };
        (s == ERROR_SUCCESS).then_some(v)
    }

    fn delete_value(&self, name: &str) {
        unsafe {
            let _ = RegDeleteValueW(self.0, &HSTRING::from(name));
        }
    }
}

fn delete_tree(path: &str) {
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(path));
    }
}

/// A registry value Notsy sets so `.md` files can open in it: the key (under
/// HKCU), the value's name (None: the key's default), and its text (None:
/// an empty value of no type, as `OpenWithProgids` wants).
#[derive(Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub name: Option<String>,
    pub value: Option<String>,
}

/// The ProgID `.md` files open through, and where Default apps finds Notsy.
fn prog_id() -> &'static str {
    if testing() { "Notsy.Test.md" } else { "Notsy.md" }
}

fn capabilities() -> &'static str {
    if testing() { r"Software\Notsy.Test\Capabilities" } else { r"Software\Notsy\Capabilities" }
}

/// Every value that makes `.md` and `.markdown` files open in Notsy: its own
/// file type, Open with, and a place in Settings > Default apps (where the
/// user makes it the default: Windows lets no app do that itself).
pub fn association_entries(exe: &Path, prog: &str, caps: &str, app_name: &str, shared: bool) -> Vec<Entry> {
    let open = format!("\"{}\" \"%1\"", exe.display());
    let e = |key: String, name: Option<&str>, value: Option<String>| Entry { key, name: name.map(str::to_string), value };
    let mut out = vec![
        e(format!(r"Software\Classes\{prog}"), None, Some("Markdown note".into())),
        e(format!(r"Software\Classes\{prog}\DefaultIcon"), None, Some(format!("\"{}\",0", exe.display()))),
        e(format!(r"Software\Classes\{prog}\shell\open\command"), None, Some(open.clone())),
    ];
    for ext in [".md", ".markdown"] {
        out.push(e(format!(r"Software\Classes\{ext}\OpenWithProgids"), Some(prog), None));
        out.push(e(format!(r"{caps}\FileAssociations"), Some(ext), Some(prog.into())));
    }
    out.push(e(caps.into(), Some("ApplicationName"), Some(app_name.into())));
    out.push(e(caps.into(), Some("ApplicationDescription"), Some("Notes that open before you blink".into())));
    out.push(e(r"Software\RegisteredApplications".into(), Some(app_name), Some(caps.into())));
    if shared {
        out.push(e(r"Software\Classes\Applications\notsy.exe\shell\open\command".into(), None, Some(open)));
        out.push(e(r"Software\Classes\Applications\notsy.exe".into(), Some("FriendlyAppName"), Some("Notsy".into())));
        for ext in [".md", ".markdown"] {
            out.push(e(r"Software\Classes\Applications\notsy.exe\SupportedTypes".into(), Some(ext), Some(String::new())));
        }
    }
    out
}

fn associate(exe: &Path) -> bool {
    let mut ok = true;
    for entry in association_entries(exe, prog_id(), capabilities(), name(), !testing()) {
        let Some(key) = Key::create(&entry.key) else {
            ok = false;
            continue;
        };
        ok &= match &entry.value {
            Some(v) => key.set_str(entry.name.as_deref(), v),
            None => key.set(entry.name.as_deref(), REG_NONE, &[]),
        };
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    ok
}

/// Takes back what `associate` set, and only that: `.md` keeps whatever
/// else opens it.
fn dissociate() {
    delete_tree(&format!(r"Software\Classes\{}", prog_id()));
    for ext in [".md", ".markdown"] {
        if let Some(k) = Key::open(&format!(r"Software\Classes\{ext}\OpenWithProgids")) {
            k.delete_value(prog_id());
        }
    }
    if let Some(k) = Key::open(r"Software\RegisteredApplications") {
        k.delete_value(name());
    }
    delete_tree(capabilities().trim_end_matches(r"\Capabilities"));
    if !testing() {
        delete_tree(r"Software\Classes\Applications\notsy.exe");
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn run_value() -> &'static str {
    if testing() { "Notsy.Test" } else { "Notsy" }
}

/// Starting with Windows: Notsy starts hidden beside the clock (`--hidden`).
fn set_startup(exe: &Path, on: bool) -> bool {
    let Some(k) = Key::create(RUN) else { return false };
    if on {
        k.set_str(Some(run_value()), &format!("\"{}\" --hidden", exe.display()))
    } else {
        k.delete_value(run_value());
        true
    }
}

fn shortcut(app: &Path, at: &Path) -> windows::core::Result<()> {
    if let Some(dir) = at.parent() {
        let _ = fs::create_dir_all(dir);
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(app.as_os_str()))?;
        link.SetDescription(&HSTRING::from("Notes that open before you blink"))?;
        if let Some(dir) = app.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        link.SetIconLocation(&HSTRING::from(app.as_os_str()), 0)?;
        link.cast::<IPersistFile>()?.Save(&HSTRING::from(at.as_os_str()), true)
    }
}

/// Writes `bytes` to `path` whole or not at all. A Notsy still running
/// there steps aside to `notsy.old.exe` (a running program can be renamed),
/// which Notsy deletes at its next start.
fn put(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let part = path.with_extension("part");
    fs::write(&part, bytes)?;
    if fs::rename(&part, path).is_ok() {
        return Ok(());
    }
    if path.file_name().is_none_or(|n| n != APP) {
        let _ = fs::remove_file(&part);
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    let old = notsy_core::update::old_exe(path);
    let _ = fs::remove_file(&old);
    let moved = fs::rename(path, &old).and_then(|()| fs::rename(&part, path));
    if moved.is_err() {
        let _ = fs::remove_file(&part);
    }
    moved
}

/// Asks a running Notsy to save and quit, through its own `--quit`, and
/// waits a while for it to go. Never by force.
pub fn quit_running() {
    if !crate::instance::running() {
        return;
    }
    if let Ok(exe) = std::env::current_exe()
        && let Ok(mut child) = std::process::Command::new(exe).arg("--quit").spawn()
    {
        let _ = child.wait();
    }
    let until = Instant::now() + Duration::from_secs(10);
    while crate::instance::running() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Installs what `stub` carries, as `choices` say; `progress` hears how far
/// it is (0..1). Returns the installed program.
pub fn install(stub: &Path, choices: Choices, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    let bytes = fs::read(stub).map_err(|e| format!("Could not read the installer: {e}"))?;
    let files = pack::unpack(&bytes).ok_or("The installer has no Notsy inside it")?;
    progress(0.1);
    let dir = install_dir().ok_or("LOCALAPPDATA is not set")?;
    fs::create_dir_all(&dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
    let app = dir.join(APP);
    quit_running();
    progress(0.25);
    for f in &files {
        put(&dir.join(&f.name), &f.bytes).map_err(|e| format!("Could not write {}: {e}", f.name))?;
    }
    // The uninstaller of the first installer: Notsy removes itself now.
    let _ = fs::remove_file(dir.join("uninstall.exe"));
    progress(0.6);
    let menu = start_menu().ok_or("No Start menu folder")?;
    shortcut(&app, &menu.join(shortcut_name())).map_err(|e| format!("Could not add Notsy to Start: {e}"))?;
    if let Some(desk) = desktop() {
        let at = desk.join(shortcut_name());
        if choices.desktop {
            shortcut(&app, &at).map_err(|e| format!("Could not add the desktop shortcut: {e}"))?;
        } else {
            let _ = fs::remove_file(at);
        }
    }
    progress(0.75);
    let size: u64 = files.iter().map(|f| f.bytes.len() as u64).sum();
    register(&dir, size, choices).ok_or("Could not add Notsy to Settings > Apps")?;
    if choices.files {
        associate(&app);
    } else {
        dissociate();
    }
    set_startup(&app, choices.startup);
    progress(1.0);
    Ok(app)
}

/// The entry in Settings > Apps, which runs `notsy.exe --uninstall`; it
/// keeps the choices too, for the next update.
fn register(dir: &Path, size: u64, choices: Choices) -> Option<()> {
    let key = Key::create(notsy_core::update::uninstall_key())?;
    let app = dir.join(APP);
    let strings = [
        ("DisplayName", name().to_string()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
        ("Publisher", "Notsy".to_string()),
        ("DisplayIcon", format!("{},0", app.display())),
        ("InstallLocation", dir.display().to_string()),
        ("UninstallString", format!("\"{}\" --uninstall", app.display())),
        ("QuietUninstallString", format!("\"{}\" --uninstall --quiet", app.display())),
        ("URLInfoAbout", "https://notsy.co".to_string()),
        ("InstallDate", today()),
    ];
    let mut ok = strings.iter().all(|(n, v)| key.set_str(Some(n), v));
    for (n, v) in [
        ("EstimatedSize", size.div_ceil(1024) as u32),
        ("NoModify", 1),
        ("NoRepair", 1),
        ("NotsyDesktop", choices.desktop as u32),
        ("NotsyOpensMarkdown", choices.files as u32),
        ("NotsyStartsWithWindows", choices.startup as u32),
    ] {
        ok &= key.set_dword(n, v);
    }
    ok.then_some(())
}

/// Today as Windows' InstallDate wants it, `YYYYMMDD` (UTC).
pub fn today() -> String {
    let days = (notsy_core::session::now() / 86_400) as i64;
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}")
}

/// Removes Notsy: the program, its shortcuts, `.md` files opening in it,
/// starting with Windows, its Settings > Apps entry, an update waiting.
/// Never a note, nor the settings (a later install keeps them).
pub fn uninstall(progress: &dyn Fn(f32)) -> Result<(), String> {
    let dir = install_dir().ok_or("LOCALAPPDATA is not set")?;
    quit_running();
    progress(0.3);
    let me = std::env::current_exe().ok();
    for name in [APP, "notsy.old.exe", "notsy.part", "THIRD-PARTY.md", "THIRD-PARTY.part", "uninstall.exe"] {
        let path = dir.join(name);
        // This program, if it is the install's, goes last (see `leave`).
        if !path.exists() || me.as_deref() == Some(path.as_path()) {
            continue;
        }
        fs::remove_file(&path).map_err(|e| format!("Could not remove {name}: {e}. Is Notsy still open?"))?;
    }
    progress(0.6);
    for folder in [start_menu(), desktop()].into_iter().flatten() {
        let _ = fs::remove_file(folder.join(shortcut_name()));
    }
    dissociate();
    if let Some(k) = Key::open(RUN) {
        k.delete_value(run_value());
    }
    delete_tree(notsy_core::update::uninstall_key());
    if let Some(data) = notsy_core::app_data_dir() {
        let staging = notsy_core::update::staging(&data);
        notsy_core::update::clear(&staging);
        let _ = fs::remove_file(staging.join("checked"));
        let _ = fs::remove_dir(staging);
    }
    progress(1.0);
    Ok(())
}

/// The last step of removing: this program, if it is the install's, moves
/// out to the temp folder (a running program cannot be deleted, but it can
/// be moved), the empty install folder goes, and a moment after this
/// process ends, the moved program is deleted too.
pub fn leave() {
    let (Some(dir), Ok(me)) = (install_dir(), std::env::current_exe()) else { return };
    if me.parent() == Some(dir.as_path()) {
        let away = std::env::temp_dir().join(format!("notsy-uninstalled-{}.exe", std::process::id()));
        if fs::rename(&me, &away).is_ok() {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let line = format!("ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"", away.display());
            let _ = std::process::Command::new("cmd.exe").arg("/c").raw_arg(line).creation_flags(CREATE_NO_WINDOW).spawn();
        }
    }
    let _ = fs::remove_dir(&dir);
}

/// Settings > Default apps, open at Notsy, where `.md` files can be given
/// to it with a click.
pub fn open_default_apps() {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let uri = HSTRING::from(format!("ms-settings:defaultapps?registeredAppUser={}", name()));
    unsafe {
        ShellExecuteW(None, &HSTRING::from("open"), &uri, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_start_opens_what_was_chosen() {
        let dir = std::env::temp_dir().join(format!("notsy-launch-{}", std::process::id()));
        let missing = dir.join("Notsy");
        assert!(launch_args(&Pick::New(missing.clone())).is_empty(), "a new folder: a first run, the welcome note in it");
        fs::create_dir_all(&missing).unwrap();
        assert_eq!(launch_args(&Pick::New(missing.clone())), [OsString::from("--vault"), missing.clone().into()], "a folder there already opens as it is");
        let vault = dir.join("Work");
        assert_eq!(launch_args(&Pick::Open(vault.clone())), [OsString::from("--vault"), vault.clone().into()]);
        assert_eq!(launch_args(&Pick::Other(Some(vault.clone()))), [OsString::from("--vault"), vault.into()]);
        let export = dir.join("old.nbk");
        assert_eq!(launch_args(&Pick::Import(export.clone())), [OsString::from(export)], "an export is imported");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn places_read_as_the_user_knows_them() {
        let home = PathBuf::from_iter(["C:\\", "Users", "a"]);
        let docs = home.join("Documents");
        let new = docs.join("Notsy");
        let vault = Found { kind: Kind::Obsidian, path: docs.join("Work"), notes: 214 };
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(describe(&vault, Some(&home), &new), ("Obsidian vault \u{00B7} Work".to_string(), format!("Documents{sep}Work \u{00B7} 214 notes \u{00B7} opens as it is")));
        let export = Found { kind: Kind::NotesExport, path: docs.join("old.nbk"), notes: 0 };
        assert_eq!(describe(&export, Some(&home), &new).1, format!("Documents{sep}old.nbk \u{00B7} imported into Documents{sep}Notsy"));
        let one = Found { kind: Kind::Notsy, path: new.clone(), notes: 1 };
        assert_eq!(describe(&one, Some(&home), &new), ("Your Notsy notes".to_string(), format!("Documents{sep}Notsy \u{00B7} 1 note")));
        assert_eq!(short(&new, None), new.display().to_string());
    }

    #[test]
    fn md_files_open_in_notsy_through_its_own_type() {
        let exe = PathBuf::from_iter(["C:\\", "Notsy", "notsy.exe"]);
        let all = association_entries(&exe, "Notsy.md", r"Software\Notsy\Capabilities", "Notsy", true);
        let find = |key: &str, name: Option<&str>| all.iter().find(|e| e.key == key && e.name.as_deref() == name).unwrap_or_else(|| panic!("{key} {name:?}"));
        assert_eq!(find(r"Software\Classes\Notsy.md\shell\open\command", None).value.as_deref(), Some(format!("\"{}\" \"%1\"", exe.display()).as_str()));
        assert_eq!(find(r"Software\Classes\.md\OpenWithProgids", Some("Notsy.md")).value, None, "an empty value, as Windows wants");
        assert_eq!(find(r"Software\Classes\.markdown\OpenWithProgids", Some("Notsy.md")).value, None);
        assert_eq!(find(r"Software\Notsy\Capabilities\FileAssociations", Some(".md")).value.as_deref(), Some("Notsy.md"));
        assert_eq!(find(r"Software\RegisteredApplications", Some("Notsy")).value.as_deref(), Some(r"Software\Notsy\Capabilities"));
        // Nothing takes `.md` itself: the user's default stays theirs.
        assert!(all.iter().all(|e| !(e.key == r"Software\Classes\.md" && e.name.is_none())));
        // A test install never touches the key a real one shares.
        let test = association_entries(&exe, "Notsy.Test.md", r"Software\Notsy.Test\Capabilities", "Notsy (test)", false);
        assert!(test.iter().all(|e| !e.key.contains("Applications\\notsy.exe")));
    }

    #[test]
    fn install_date_is_year_month_day() {
        let t = today();
        assert!(t.len() == 8 && t.starts_with("20") && t.bytes().all(|b| b.is_ascii_digit()));
    }
}
