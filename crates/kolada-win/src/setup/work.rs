//! What installing and removing do, with no window: files, shortcuts, the
//! Settings > Apps entry, `.md` files, starting with Windows.
//!
//! Everything is per user (HKCU, `%LOCALAPPDATA%`): no admin. A test run
//! (`KOLADA_TEST_INSTANCE`) keeps apart from a real install: its own names in
//! the registry, folders from the environment (point `LOCALAPPDATA`,
//! `APPDATA` and `USERPROFILE` elsewhere), and never the per-program key
//! `Applications\kolada.exe`, which a real install shares.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kolada_core::before;
use kolada_core::found::{Found, Kind};
use kolada_core::pack;
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

pub const APP: &str = "kolada.exe";

fn testing() -> bool {
    kolada_core::testing()
}

/// What Settings > Apps and the shortcuts call it.
pub fn name() -> &'static str {
    if testing() { "Kolada (test)" } else { "Kolada" }
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

/// `%LOCALAPPDATA%\Programs\Kolada`: per user, no admin.
pub fn install_dir() -> Option<PathBuf> {
    Some(env_dir("LOCALAPPDATA")?.join("Programs").join("Kolada"))
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
/// the profile's, where Kolada makes its own folder.
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

/// Start's Beam: its popup alone, pinnable on its own.
fn beam_shortcut_name() -> &'static str {
    if testing() { "Beam (test).lnk" } else { "Beam.lnk" }
}

/// The Beam shortcut's arguments and icon (`kolada.rc`'s 5, by resource id).
const BEAM_ARGS: &str = "--beam";
const BEAM_ICON: i32 = 5;

/// The three choices: a desktop shortcut, `.md` files opening in Kolada, and
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

/// This app's install, or else one from before the rename (whose choices a
/// new install keeps).
pub fn installed() -> Option<Installed> {
    let own = Key::open(kolada_core::update::uninstall_key()).and_then(|k| read_install(&k, APP, ["KoladaDesktop", "KoladaOpensMarkdown", "KoladaStartsWithWindows"]));
    own.or_else(|| old_install().map(|(_, i)| i))
}

fn read_install(key: &Key, exe: &str, choices: [&str; 3]) -> Option<Installed> {
    let dir = PathBuf::from(key.get_str("InstallLocation")?);
    if !dir.join(exe).is_file() {
        return None;
    }
    let on = |name: &str| key.get_dword(name).is_some_and(|v| v != 0);
    let choices = Choices { desktop: on(choices[0]), files: on(choices[1]), startup: on(choices[2]) };
    Some(Installed { version: key.get_str("DisplayVersion").unwrap_or_default(), choices })
}

/// An install the website's installer made before the rename, and its
/// folder; never seen by a test instance.
fn old_install() -> Option<(PathBuf, Installed)> {
    if testing() || before::UNINSTALL_KEY == kolada_core::update::uninstall_key() {
        return None;
    }
    let key = Key::open(before::UNINSTALL_KEY)?;
    let dir = PathBuf::from(key.get_str("InstallLocation")?);
    read_install(&key, before::EXE, before::CHOICES).map(|i| (dir, i))
}

/// Where the notes go, as chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// A new folder (Documents\Kolada), the welcome note in it.
    New(PathBuf),
    /// A folder of Markdown, opened as it is (Kolada's own from before too).
    Open(PathBuf),
    /// An export, imported into a new Documents\Kolada.
    Import(PathBuf),
    /// Any folder, once chosen.
    Other(Option<PathBuf>),
}

impl Pick {
    pub fn from_found(f: &Found) -> Pick {
        if f.kind.opens_in_place() { Pick::Open(f.path.clone()) } else { Pick::Import(f.path.clone()) }
    }
}

/// What the installed Kolada is started with, the first time: nothing (a
/// first run: the welcome note in a new Documents\Kolada), the vault to open,
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
    let count = if n >= kolada_core::found::MOST_COUNTED { format!("{n}+") } else { n.to_string() };
    if n == 1 { "1 note".into() } else { format!("{count} notes") }
}

/// A place found, as the installer lists it: its name and the line under it.
pub fn describe(f: &Found, home: Option<&Path>, new_folder: &Path) -> (String, String) {
    let at = short(&f.path, home);
    let detail = match f.kind {
        Kind::NotesExport => format!("{at} \u{00B7} imported into {}", short(new_folder, home)),
        Kind::Kolada => format!("{at} \u{00B7} {}", notes_phrase(f.notes)),
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

/// A registry value Kolada sets so `.md` files can open in it: the key (under
/// HKCU), the value's name (None: the key's default), and its text (None:
/// an empty value of no type, as `OpenWithProgids` wants).
#[derive(Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub name: Option<String>,
    pub value: Option<String>,
}

/// The ProgID `.md` files open through, and where Default apps finds Kolada.
/// It keeps the old name (see `before::PROG_ID`).
fn prog_id() -> &'static str {
    if testing() { "Kolada.Test.md" } else { before::PROG_ID }
}

fn capabilities() -> &'static str {
    if testing() { r"Software\Kolada.Test\Capabilities" } else { r"Software\Kolada\Capabilities" }
}

/// Every value that makes `.md` and `.markdown` files open in Kolada: its own
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
        out.push(e(r"Software\Classes\Applications\kolada.exe\shell\open\command".into(), None, Some(open)));
        out.push(e(r"Software\Classes\Applications\kolada.exe".into(), Some("FriendlyAppName"), Some("Kolada".into())));
        for ext in [".md", ".markdown"] {
            out.push(e(r"Software\Classes\Applications\kolada.exe\SupportedTypes".into(), Some(ext), Some(String::new())));
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
        delete_tree(r"Software\Classes\Applications\kolada.exe");
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn run_value() -> &'static str {
    if testing() { "Kolada.Test" } else { "Kolada" }
}

/// Starting with Windows: Kolada starts hidden beside the clock (`--hidden`).
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
    shortcut_to(app, at, "", "Notes that open before you blink", 0)
}

/// A shortcut to `app` with `args`, and the icon resource `icon` (0 the
/// app's own).
fn shortcut_to(app: &Path, at: &Path, args: &str, tip: &str, icon: i32) -> windows::core::Result<()> {
    if let Some(dir) = at.parent() {
        let _ = fs::create_dir_all(dir);
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(app.as_os_str()))?;
        if !args.is_empty() {
            link.SetArguments(&HSTRING::from(args))?;
        }
        link.SetDescription(&HSTRING::from(tip))?;
        if let Some(dir) = app.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        // A negative index names an icon by its resource id.
        link.SetIconLocation(&HSTRING::from(app.as_os_str()), -icon)?;
        link.cast::<IPersistFile>()?.Save(&HSTRING::from(at.as_os_str()), true)
    }
}

/// Start's Beam beside Kolada: a second entry, opening only its popup
/// (`--beam`). Kolada is installed without it if it cannot be made.
fn beam_shortcut(app: &Path, menu: &Path) {
    let _ = shortcut_to(app, &menu.join(beam_shortcut_name()), BEAM_ARGS, "What you copied, to your other devices", BEAM_ICON);
}

/// The writing helper, set aside while it was still running.
const OLD_HELPER: &str = "kolada-ai.old.exe";

/// Every file an install puts in its folder (`tools/setup.ps1` packs
/// them), with what each leaves behind: a copy cut short, a program set
/// aside while it ran. Removing Kolada removes them all.
const INSTALLED: [&str; 10] = [
    APP,
    "kolada.old.exe",
    "kolada.part",
    crate::assist::HELPER,
    OLD_HELPER,
    "kolada-ai.part",
    "THIRD-PARTY.md",
    "THIRD-PARTY.part",
    "LICENSE-llama.cpp.txt",
    "LICENSE-llama.cpp.part",
];

/// Where a program of the install steps aside to while it is still running
/// (a running program can be renamed, not written over): Kolada to
/// `kolada.old.exe`, which it deletes at its next start; its writing helper,
/// on its way out with it, to `kolada-ai.old.exe`. Nothing else is a
/// program.
fn aside(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?;
    if name == APP {
        Some(kolada_core::update::old_exe(path))
    } else if name == crate::assist::HELPER {
        Some(path.with_file_name(OLD_HELPER))
    } else {
        None
    }
}

/// Writes `bytes` to `path` whole or not at all. A program still running
/// there steps aside (see `aside`).
fn put(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let part = path.with_extension("part");
    fs::write(&part, bytes)?;
    if fs::rename(&part, path).is_ok() {
        return Ok(());
    }
    let Some(old) = aside(path) else {
        let _ = fs::remove_file(&part);
        return Err(std::io::ErrorKind::PermissionDenied.into());
    };
    let _ = fs::remove_file(&old);
    let moved = fs::rename(path, &old).and_then(|()| fs::rename(&part, path));
    if moved.is_err() {
        let _ = fs::remove_file(&part);
    }
    moved
}

/// Asks a running Kolada to save and quit, through its own `--quit`, and
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
    let files = pack::unpack(&bytes).ok_or("The installer has no Kolada inside it")?;
    progress(0.1);
    let dir = install_dir().ok_or("LOCALAPPDATA is not set")?;
    fs::create_dir_all(&dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
    let app = dir.join(APP);
    quit_running();
    progress(0.25);
    for f in &files {
        put(&dir.join(&f.name), &f.bytes).map_err(|e| format!("Could not write {}: {e}", f.name))?;
    }
    // The uninstaller of the first installer: Kolada removes itself now.
    let _ = fs::remove_file(dir.join("uninstall.exe"));
    // A helper set aside has ended with the Kolada that ran it, by now or
    // by the next install or removal.
    let _ = fs::remove_file(dir.join(OLD_HELPER));
    progress(0.6);
    let menu = start_menu().ok_or("No Start menu folder")?;
    shortcut(&app, &menu.join(shortcut_name())).map_err(|e| format!("Could not add Kolada to Start: {e}"))?;
    beam_shortcut(&app, &menu);
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
    settle(&dir, size, choices)?;
    // An install from before the rename: this one takes its place.
    if let Some((old, _)) = old_install() {
        forget_old_install(&old, &app);
    }
    progress(1.0);
    Ok(app)
}

/// Settings > Apps, `.md` files and starting with Windows, for the program
/// in `dir`, as `choices` say.
fn settle(dir: &Path, size: u64, choices: Choices) -> Result<(), String> {
    let app = dir.join(APP);
    register(dir, size, choices).ok_or("Could not add Kolada to Settings > Apps")?;
    if choices.files {
        associate(&app);
    } else {
        dissociate();
    }
    set_startup(&app, choices.startup);
    Ok(())
}

/// The files an install from before the rename has in `old`, and what each
/// is called in the new install: the program, its writing helper and
/// model, the notices. What is not there is left out.
fn old_files(old: &Path) -> Vec<(PathBuf, &'static str)> {
    [(before::EXE, APP), (before::HELPER, crate::assist::HELPER), ("models", "models"), ("THIRD-PARTY.md", "THIRD-PARTY.md")]
        .into_iter()
        .map(|(from, to)| (old.join(from), to))
        .filter(|(from, _)| from.exists())
        .collect()
}

/// Puts an old install's files in `new`: the program copied (it is the
/// one running), the rest moved. On a failure what was done is undone, and
/// nothing changed.
fn move_files(old: &Path, new: &Path) -> std::io::Result<()> {
    fs::create_dir_all(new)?;
    let mut done: Vec<(PathBuf, PathBuf, bool)> = Vec::new();
    let mut result = Ok(());
    for (from, to) in old_files(old) {
        let to = new.join(to);
        let copy = from.file_name().is_some_and(|n| n.eq_ignore_ascii_case(before::EXE));
        let step = if copy {
            let part = to.with_extension("part");
            fs::copy(&from, &part).and_then(|_| fs::rename(&part, &to))
        } else {
            fs::rename(&from, &to)
        };
        if let Err(e) = step {
            result = Err(e);
            break;
        }
        done.push((from, to, copy));
    }
    if result.is_err() {
        for (from, to, copy) in done.into_iter().rev() {
            let _ = if copy { fs::remove_file(&to) } else { fs::rename(&to, &from) };
        }
    }
    result
}

/// After an update from Notsy, the program is still the old install's
/// (`Programs\Notsy\notsy.exe`): before the window it moves house to
/// `install_dir`, under the new name, with the choices it was installed
/// with. Returns the moved program, to be started in this one's place;
/// None when there is nothing to move (or it could not be).
pub fn rehome() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let (old, installed) = old_install()?;
    let new = install_dir()?;
    let at_old = exe.file_name().is_some_and(|n| n.eq_ignore_ascii_case(before::EXE)) && exe.parent() == Some(old.as_path());
    if !at_old || old == new || crate::updates::packaged() {
        return None;
    }
    move_files(&old, &new).ok()?;
    let app = new.join(APP);
    let size = fs::metadata(&app).map_or(0, |m| m.len());
    if settle(&new, size, installed.choices).is_err() {
        return None;
    }
    if let Some(menu) = start_menu() {
        let _ = shortcut(&app, &menu.join(shortcut_name()));
        beam_shortcut(&app, &menu);
    }
    if installed.choices.desktop
        && let Some(desk) = desktop()
    {
        let _ = shortcut(&app, &desk.join(shortcut_name()));
    }
    forget_old_install(&old, &app);
    Some(app)
}

/// Takes back what an install from before the rename set: its shortcuts
/// (a taskbar pin is pointed at `app` instead), starting with Windows, its
/// place in Default apps and Settings > Apps, and its files once it has
/// stopped (now, or at a later start, see `tidy_old_install`). `.md` files
/// keep opening through the same ProgID, now in `app`.
fn forget_old_install(old: &Path, app: &Path) {
    let link = format!("{}.lnk", before::VALUE);
    for folder in [start_menu(), desktop()].into_iter().flatten() {
        let _ = fs::remove_file(folder.join(&link));
    }
    if let Some(pins) = env_dir("APPDATA").map(|a| a.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar")) {
        let pin = pins.join(&link);
        if pin.is_file() {
            let _ = shortcut(app, &pin);
        }
    }
    if let Some(k) = Key::open(RUN) {
        k.delete_value(before::VALUE);
    }
    if let Some(k) = Key::open(r"Software\RegisteredApplications") {
        k.delete_value(before::VALUE);
    }
    delete_tree(before::SOFTWARE_KEY);
    delete_tree(before::APPLICATIONS_KEY);
    delete_tree(before::UNINSTALL_KEY);
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    remove_old_files(old);
}

/// An old install's own files, by name (never the folder whole: it is
/// only ever removed once empty).
fn remove_old_files(old: &Path) -> bool {
    for name in [before::EXE, before::HELPER, before::LEFTOVERS[0], before::LEFTOVERS[1], "THIRD-PARTY.md", "THIRD-PARTY.part", "uninstall.exe"] {
        let _ = fs::remove_file(old.join(name));
    }
    let _ = fs::remove_dir(old);
    !old.exists()
}

/// After the first frame of a start that followed a move from an install
/// from before the rename: its folder goes, once the old program has
/// stopped running.
pub fn tidy_old_install() {
    if testing() || before::INSTALL_DIR == install_dir().and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default() {
        return;
    }
    let Some(old) = env_dir("LOCALAPPDATA").map(|l| l.join("Programs").join(before::INSTALL_DIR)) else { return };
    if !old.join(before::EXE).is_file() || Key::open(before::UNINSTALL_KEY).is_some() {
        return;
    }
    std::thread::spawn(move || {
        crate::background_priority();
        for _ in 0..40 {
            if remove_old_files(&old) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}

/// The entry in Settings > Apps, which runs `kolada.exe --uninstall`; it
/// keeps the choices too, for the next update.
fn register(dir: &Path, size: u64, choices: Choices) -> Option<()> {
    let key = Key::create(kolada_core::update::uninstall_key())?;
    let app = dir.join(APP);
    let strings = [
        ("DisplayName", name().to_string()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
        ("Publisher", "Kolada".to_string()),
        ("DisplayIcon", format!("{},0", app.display())),
        ("InstallLocation", dir.display().to_string()),
        ("UninstallString", format!("\"{}\" --uninstall", app.display())),
        ("QuietUninstallString", format!("\"{}\" --uninstall --quiet", app.display())),
        ("URLInfoAbout", "https://kolada.app".to_string()),
        ("InstallDate", today()),
    ];
    let mut ok = strings.iter().all(|(n, v)| key.set_str(Some(n), v));
    for (n, v) in [
        ("EstimatedSize", size.div_ceil(1024) as u32),
        ("NoModify", 1),
        ("NoRepair", 1),
        ("KoladaDesktop", choices.desktop as u32),
        ("KoladaOpensMarkdown", choices.files as u32),
        ("KoladaStartsWithWindows", choices.startup as u32),
    ] {
        ok &= key.set_dword(n, v);
    }
    ok.then_some(())
}

/// Today as Windows' InstallDate wants it, `YYYYMMDD` (UTC).
pub fn today() -> String {
    let days = (kolada_core::session::now() / 86_400) as i64;
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

/// Removes Kolada: the program, its shortcuts, `.md` files opening in it,
/// starting with Windows, its Settings > Apps entry, an update waiting.
/// Never a note, nor the settings (a later install keeps them).
pub fn uninstall(progress: &dyn Fn(f32)) -> Result<(), String> {
    let dir = install_dir().ok_or("LOCALAPPDATA is not set")?;
    quit_running();
    progress(0.3);
    let me = std::env::current_exe().ok();
    for name in INSTALLED.into_iter().chain(["uninstall.exe"]) {
        let path = dir.join(name);
        // This program, if it is the install's, goes last (see `leave`).
        if !path.exists() || me.as_deref() == Some(path.as_path()) {
            continue;
        }
        fs::remove_file(&path).map_err(|e| format!("Could not remove {name}: {e}. Is Kolada still open?"))?;
    }
    // A model an install from before the rename kept beside the program
    // (`old_files`); a model the person downloaded is in the app's data.
    let _ = fs::remove_dir_all(dir.join("models"));
    progress(0.6);
    for folder in [start_menu(), desktop()].into_iter().flatten() {
        let _ = fs::remove_file(folder.join(shortcut_name()));
        let _ = fs::remove_file(folder.join(beam_shortcut_name()));
    }
    dissociate();
    if let Some(k) = Key::open(RUN) {
        k.delete_value(run_value());
    }
    delete_tree(kolada_core::update::uninstall_key());
    if let Some(data) = kolada_core::app_data_dir() {
        let staging = kolada_core::update::staging(&data);
        kolada_core::update::clear(&staging);
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
        let away = std::env::temp_dir().join(format!("kolada-uninstalled-{}.exe", std::process::id()));
        if fs::rename(&me, &away).is_ok() {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let line = format!("ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"", away.display());
            let _ = std::process::Command::new("cmd.exe").arg("/c").raw_arg(line).creation_flags(CREATE_NO_WINDOW).spawn();
        }
    }
    let _ = fs::remove_dir(&dir);
}

/// Settings > Default apps, open at Kolada, where `.md` files can be given
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
        let dir = std::env::temp_dir().join(format!("kolada-launch-{}", std::process::id()));
        let missing = dir.join("Kolada");
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
        let new = docs.join("Kolada");
        let vault = Found { kind: Kind::Obsidian, path: docs.join("Work"), notes: 214 };
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(describe(&vault, Some(&home), &new), ("Obsidian vault \u{00B7} Work".to_string(), format!("Documents{sep}Work \u{00B7} 214 notes \u{00B7} opens as it is")));
        let export = Found { kind: Kind::NotesExport, path: docs.join("old.nbk"), notes: 0 };
        assert_eq!(describe(&export, Some(&home), &new).1, format!("Documents{sep}old.nbk \u{00B7} imported into Documents{sep}Kolada"));
        let one = Found { kind: Kind::Kolada, path: new.clone(), notes: 1 };
        assert_eq!(describe(&one, Some(&home), &new), ("Your Kolada notes".to_string(), format!("Documents{sep}Kolada \u{00B7} 1 note")));
        assert_eq!(short(&new, None), new.display().to_string());
    }

    #[test]
    fn md_files_open_in_kolada_through_its_own_type() {
        let exe = PathBuf::from_iter(["C:\\", "Kolada", "kolada.exe"]);
        let all = association_entries(&exe, "Kolada.md", r"Software\Kolada\Capabilities", "Kolada", true);
        let find = |key: &str, name: Option<&str>| all.iter().find(|e| e.key == key && e.name.as_deref() == name).unwrap_or_else(|| panic!("{key} {name:?}"));
        assert_eq!(find(r"Software\Classes\Kolada.md\shell\open\command", None).value.as_deref(), Some(format!("\"{}\" \"%1\"", exe.display()).as_str()));
        assert_eq!(find(r"Software\Classes\.md\OpenWithProgids", Some("Kolada.md")).value, None, "an empty value, as Windows wants");
        assert_eq!(find(r"Software\Classes\.markdown\OpenWithProgids", Some("Kolada.md")).value, None);
        assert_eq!(find(r"Software\Kolada\Capabilities\FileAssociations", Some(".md")).value.as_deref(), Some("Kolada.md"));
        assert_eq!(find(r"Software\RegisteredApplications", Some("Kolada")).value.as_deref(), Some(r"Software\Kolada\Capabilities"));
        // Nothing takes `.md` itself: the user's default stays theirs.
        assert!(all.iter().all(|e| !(e.key == r"Software\Classes\.md" && e.name.is_none())));
        // A test install never touches the key a real one shares.
        let test = association_entries(&exe, "Kolada.Test.md", r"Software\Kolada.Test\Capabilities", "Kolada (test)", false);
        assert!(test.iter().all(|e| !e.key.contains("Applications\\kolada.exe")));
    }

    fn old_install_folder(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("notsy-rehome-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let old = root.join(before::INSTALL_DIR);
        fs::create_dir_all(old.join("models")).unwrap();
        fs::write(old.join(before::EXE), b"exe").unwrap();
        fs::write(old.join(before::HELPER), b"ai").unwrap();
        fs::write(old.join("models").join("m.gguf"), b"model").unwrap();
        fs::write(old.join("THIRD-PARTY.md"), b"notices").unwrap();
        (old, root.join("New"))
    }

    #[test]
    fn an_old_install_moves_house_under_the_new_names() {
        let (old, new) = old_install_folder("moves");
        move_files(&old, &new).unwrap();
        assert_eq!(fs::read(new.join(APP)).unwrap(), b"exe");
        assert_eq!(fs::read(new.join(crate::assist::HELPER)).unwrap(), b"ai");
        assert_eq!(fs::read(new.join("models").join("m.gguf")).unwrap(), b"model");
        assert!(new.join("THIRD-PARTY.md").is_file());
        // The program is the one running: copied, and removed by name later.
        assert!(old.join(before::EXE).is_file());
        assert!(!old.join("models").exists());
        assert!(remove_old_files(&old), "the emptied folder goes");
        let _ = fs::remove_dir_all(old.parent().unwrap());
    }

    #[test]
    fn a_move_that_fails_halfway_is_undone() {
        let (old, new) = old_install_folder("undone");
        // Something in the way of the models.
        fs::create_dir_all(new.join("models").join("other")).unwrap();
        assert!(move_files(&old, &new).is_err());
        assert!(old.join(before::HELPER).is_file() && old.join("models").join("m.gguf").is_file() && old.join(before::EXE).is_file());
        assert!(!new.join(APP).exists() && !new.join(crate::assist::HELPER).exists());
        let _ = fs::remove_dir_all(old.parent().unwrap());
    }

    #[test]
    fn a_running_helper_steps_aside_for_the_new_one() {
        let dir = std::env::temp_dir().join(format!("kolada-put-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // A program running as the helper: Windows' ping, waiting.
        let helper = dir.join(crate::assist::HELPER);
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        fs::copy(system.join("System32").join("PING.EXE"), &helper).unwrap();
        let mut running = std::process::Command::new(&helper).args(["-n", "30", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        assert!(fs::write(&helper, b"new").is_err(), "a running program cannot be written over");
        let put_it = put(&helper, b"new helper");
        let _ = running.kill();
        let _ = running.wait();
        put_it.unwrap();
        assert_eq!(fs::read(&helper).unwrap(), b"new helper");
        assert!(dir.join(OLD_HELPER).is_file() && !dir.join("kolada-ai.part").exists());
        // Anything else is no program: it fails whole, leaving nothing.
        let notices = dir.join("LICENSE-llama.cpp.txt");
        assert_eq!(aside(&notices), None);
        put(&notices, b"MIT").unwrap();
        assert_eq!(fs::read(&notices).unwrap(), b"MIT");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_kolada_knows_every_file_an_install_puts() {
        // What tools/setup.ps1 packs, and what each leaves while written.
        for name in [APP, crate::assist::HELPER, "THIRD-PARTY.md", "LICENSE-llama.cpp.txt"] {
            assert!(INSTALLED.contains(&name), "{name}");
            let part = Path::new(name).with_extension("part");
            assert!(INSTALLED.contains(&part.to_str().unwrap()), "{}", part.display());
            if let Some(old) = aside(Path::new(name)) {
                assert!(INSTALLED.contains(&old.to_str().unwrap()), "{}", old.display());
            }
        }
    }

    #[test]
    fn install_date_is_year_month_day() {
        let t = today();
        assert!(t.len() == 8 && t.starts_with("20") && t.bytes().all(|b| b.is_ascii_digit()));
    }
}
