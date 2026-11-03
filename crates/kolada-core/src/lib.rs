//! Platform-independent Kolada core.
//!
//! Nothing here may touch the vault as a whole on the startup path: the
//! first frame reads only the session snapshot and one note.

pub mod before;
pub mod fetch;
pub mod files;
pub mod found;
pub mod history;
pub mod links;
pub mod nbk;
pub mod note;
pub mod obsidian;
pub mod pack;
pub mod related;
pub mod search;
pub mod tasks;
pub mod session;
pub mod share;
pub mod social;
pub mod sqlite;
pub mod trash;
pub mod update;
pub mod vault;
pub mod welcome;
pub mod when;

use std::path::{Path, PathBuf};

/// The app's folder, in the data root and in Documents.
pub(crate) const OWN: &str = "Kolada";
/// What the app was called before; its folders are taken over (see
/// `adopt_old_data`).
pub(crate) const OLD: &str = before::NAME;

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Where per-user app data folders live: `%LOCALAPPDATA%` on Windows,
/// `~/Library/Application Support` on macOS and iOS (where `HOME` is the
/// app's container), the app's files folder on Android (see
/// `set_data_root`), `$XDG_DATA_HOME` or `~/.local/share` elsewhere. Read from the environment rather than the
/// shell API so shell32 stays off the startup path.
pub fn data_root() -> Option<PathBuf> {
    if cfg!(windows) {
        env_path("LOCALAPPDATA")
    } else if cfg!(target_os = "android") {
        given_root()
    } else if cfg!(any(target_os = "macos", target_os = "ios")) {
        env_path("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        xdg_data_root(env_path("XDG_DATA_HOME"), env_path("HOME"))
    }
}

static GIVEN_ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Android's data root: the app's own files folder (`Context.getFilesDir()`),
/// which the shell gives once at start, before anything reads `data_root`.
/// Android sets no `XDG_DATA_HOME`, and its `HOME` is not the app's. Only an
/// absolute path counts, and only the first; whether it was taken.
pub fn set_data_root(dir: PathBuf) -> bool {
    dir.is_absolute() && GIVEN_ROOT.set(dir).is_ok()
}

fn given_root() -> Option<PathBuf> {
    GIVEN_ROOT.get().cloned()
}

static GIVEN_DOCUMENTS: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Android's Documents folder in shared storage, given by the shell once at
/// start: a first run makes `Kolada` in it (`default_vault_dir`). Only an
/// absolute path counts, and only the first; whether it was taken.
pub fn set_documents_dir(dir: PathBuf) -> bool {
    dir.is_absolute() && GIVEN_DOCUMENTS.set(dir).is_ok()
}

fn given_documents() -> Option<PathBuf> {
    GIVEN_DOCUMENTS.get().cloned()
}

/// The XDG rule: `$XDG_DATA_HOME` counts only when it is an absolute path
/// (a relative one is to be ignored), else `~/.local/share`.
fn xdg_data_root(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    xdg.filter(|p| p.is_absolute()).or_else(|| home.map(|h| h.join(".local").join("share")))
}

/// The folder a first run puts the vault in: `Documents\Kolada` under the
/// user's profile, or the `Documents\Notsy` an earlier install made (the
/// user's notes are never moved). On Linux the user's Documents folder may
/// be named or placed otherwise (`XDG_DOCUMENTS_DIR` in
/// `~/.config/user-dirs.dirs`).
pub fn default_vault_dir() -> Option<PathBuf> {
    if cfg!(target_os = "android") {
        return given_documents().map(|d| own_vault(&d));
    }
    let home = if cfg!(windows) { env_path("USERPROFILE") } else { env_path("HOME") }?;
    let documents = if cfg!(any(windows, target_os = "macos", target_os = "ios")) {
        None
    } else {
        let config = env_path("XDG_CONFIG_HOME").filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".config"));
        std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|t| xdg_documents_dir(&t, &home))
    };
    Some(own_vault(&documents.unwrap_or_else(|| home.join("Documents"))))
}

/// `Kolada` in `documents`, or `Notsy` there when only that one exists.
fn own_vault(documents: &Path) -> PathBuf {
    let dir = documents.join(OWN);
    let old = documents.join(OLD);
    if !dir.exists() && old.is_dir() { old } else { dir }
}

/// `XDG_DOCUMENTS_DIR="$HOME/Docs"` out of a `user-dirs.dirs` file. None
/// when it is missing, not under `$HOME` or `/` form, or points at the home
/// folder itself (how the desktop says "no Documents folder").
fn xdg_documents_dir(config: &str, home: &std::path::Path) -> Option<PathBuf> {
    let line = config.lines().map(str::trim).find(|l| l.starts_with("XDG_DOCUMENTS_DIR="))?;
    let value = line["XDG_DOCUMENTS_DIR=".len()..].trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some("") => home.to_path_buf(),
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None if value.starts_with('/') => PathBuf::from(value),
        None => return None,
    };
    (path != home).then_some(path)
}

/// `Kolada` inside `data_root`, or the `Notsy` from before while that has
/// not moved yet (see `adopt_old_data`). Under test (see `testing`) a
/// folder of its own beside it, `Kolada.test`: never inside, where it
/// would make `Kolada` before the real app has taken Notsy's over.
pub fn app_data_dir() -> Option<PathBuf> {
    let root = data_root()?;
    Some(if testing() { root.join(format!("{OWN}.test")) } else { own_dir(&root) })
}

/// The app's own data folder, a test instance's never: where the writing
/// model is, shared by every copy of the app.
pub fn real_data_dir() -> Option<PathBuf> {
    data_root().map(|root| own_dir(&root))
}

fn own_dir(root: &Path) -> PathBuf {
    let dir = root.join(OWN);
    if dir.exists() {
        return dir;
    }
    let old = root.join(OLD);
    if old.is_dir() { old } else { dir }
}

/// The app was called Notsy, and Nauty before that. The first start takes
/// Notsy's folder over whole, with one rename; when that fails (a file in
/// it held open) the old folder stays in use and the next start tries
/// again. From Nauty it takes copies of the session and trash list, so an
/// old Nauty still running keeps its own. Costs one look at a folder on
/// later launches. A test instance never touches the real folders.
///
/// The old names that stay are in `before`; the Store package keeps its
/// identity too (tools/msix.ps1).
pub fn adopt_old_data() {
    if testing() {
        return;
    }
    if let Some(root) = data_root() {
        adopt_old_data_in(&root);
    }
}

/// `adopt_old_data` in the data root `root`.
pub fn adopt_old_data_in(root: &Path) {
    let new = root.join(OWN);
    if new.exists() {
        return;
    }
    let notsy = root.join(OLD);
    if notsy.is_dir() {
        let _ = std::fs::rename(&notsy, &new);
        return;
    }
    let nauty = root.join("Nauty");
    if nauty.is_dir() && std::fs::create_dir_all(&new).is_ok() {
        for file in ["session.bin", "trash.tsv"] {
            let _ = std::fs::copy(nauty.join(file), new.join(file));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shells (and the repos iOS and Android live in) build on the
    /// shared crates, never the other way: a shared crate that needed a
    /// shell would tie every OS to it. Every crate here but the shells is
    /// shared.
    #[test]
    fn no_shared_crate_depends_on_a_shell() {
        // Each shell crate is `kolada-` and its OS.
        let shells: Vec<String> = ["win", "mac", "ios", "linux", "android"].iter().map(|os| format!("kolada-{os}")).collect();
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut checked = 0;
        for entry in std::fs::read_dir(crates).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(manifest) = std::fs::read_to_string(entry.path().join("Cargo.toml")) else { continue };
            if shells.contains(&name) {
                continue;
            }
            checked += 1;
            // Each dependency's name, at the start of its line, in any table.
            for line in manifest.lines().map(str::trim_start) {
                for shell in &shells {
                    let named = line.strip_prefix(shell.as_str()).is_some_and(|rest| rest.trim_start().starts_with('=') || rest.starts_with('.'));
                    assert!(!named, "{name} depends on the shell {shell}: move what it needs into a shared crate");
                }
            }
        }
        assert!(checked >= 8, "the shared crates were found ({checked})");
    }

    #[test]
    fn xdg_data_home_must_be_absolute() {
        let abs = std::env::temp_dir().join("xdg-data");
        let home = std::env::temp_dir().join("home");
        assert_eq!(xdg_data_root(Some(abs.clone()), Some(home.clone())), Some(abs));
        let fallback = Some(home.join(".local").join("share"));
        assert_eq!(xdg_data_root(Some(PathBuf::from("relative/dir")), Some(home.clone())), fallback);
        assert_eq!(xdg_data_root(None, Some(home)), fallback);
        assert_eq!(xdg_data_root(None, None), None);
    }

    #[test]
    fn android_keeps_its_data_in_the_folder_the_shell_gives() {
        // Android's HOME and XDG are not the app's: the shell's files folder is.
        let files = std::env::temp_dir().join("app.kolada").join("files");
        assert!(!set_data_root(PathBuf::from("relative/files")), "a relative folder is not taken");
        assert!(set_data_root(files.clone()));
        assert!(!set_data_root(std::env::temp_dir().join("other")), "only the first is taken");
        assert_eq!(given_root(), Some(files.clone()));
        // The notes go in Documents/Kolada in shared storage, which the Files app shows.
        let documents = std::env::temp_dir().join("storage").join("Documents");
        assert!(!set_documents_dir(PathBuf::from("Documents")));
        assert!(set_documents_dir(documents.clone()));
        assert_eq!(given_documents(), Some(documents.clone()));
        if cfg!(target_os = "android") {
            assert_eq!(data_root(), Some(files));
            assert!(app_data_dir().is_some_and(|d| d.starts_with(data_root().unwrap())));
            assert_eq!(default_vault_dir(), Some(documents.join("Kolada")));
        } else {
            assert_ne!(data_root(), Some(files), "elsewhere the OS's own rule holds");
            assert_ne!(default_vault_dir(), Some(documents.join("Kolada")));
        }
    }

    #[test]
    fn documents_folder_follows_user_dirs() {
        let home = std::env::temp_dir().join("home");
        let cfg = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOCUMENTS_DIR=\"$HOME/Docs/Mine\"\n";
        assert_eq!(xdg_documents_dir(cfg, &home), Some(home.join("Docs").join("Mine")));
        // Home itself means "no Documents folder"; a missing line, nothing to follow.
        assert_eq!(xdg_documents_dir("XDG_DOCUMENTS_DIR=\"$HOME/\"\n", &home), None);
        assert_eq!(xdg_documents_dir("XDG_DOCUMENTS_DIR=\"$HOME\"\n", &home), None);
        assert_eq!(xdg_documents_dir("XDG_DESKTOP_DIR=\"$HOME/Desktop\"\n", &home), None);
        assert_eq!(xdg_documents_dir("XDG_DOCUMENTS_DIR=\"relative\"\n", &home), None);
        assert_eq!(xdg_documents_dir("XDG_DOCUMENTS_DIR=\"/srv/docs\"\n", &home), Some(PathBuf::from("/srv/docs")));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn kolada_takes_over_notsy_data() {
        let root = scratch("adopt-notsy");
        std::fs::create_dir_all(root.join(OLD).join("history")).unwrap();
        std::fs::write(root.join(OLD).join("session.bin"), b"s").unwrap();
        std::fs::write(root.join(OLD).join("history").join("h"), b"h").unwrap();
        // Before the move the data is used where it is.
        assert_eq!(own_dir(&root), root.join(OLD));
        adopt_old_data_in(&root);
        // The whole folder moved, history and all.
        assert!(!root.join(OLD).exists());
        assert_eq!(std::fs::read(root.join(OWN).join("session.bin")).unwrap(), b"s");
        assert!(root.join(OWN).join("history").join("h").is_file());
        assert_eq!(own_dir(&root), root.join(OWN));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn both_folders_are_left_as_they_are() {
        let root = scratch("adopt-both");
        for name in [OWN, OLD] {
            std::fs::create_dir_all(root.join(name)).unwrap();
            std::fs::write(root.join(name).join("session.bin"), name).unwrap();
        }
        adopt_old_data_in(&root);
        assert_eq!(std::fs::read_to_string(root.join(OLD).join("session.bin")).unwrap(), OLD);
        assert_eq!(std::fs::read_to_string(root.join(OWN).join("session.bin")).unwrap(), OWN);
        assert_eq!(own_dir(&root), root.join(OWN));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nauty_data_is_copied_once() {
        let root = scratch("adopt-nauty");
        std::fs::create_dir_all(root.join("Nauty")).unwrap();
        std::fs::write(root.join("Nauty").join("session.bin"), b"n").unwrap();
        std::fs::write(root.join("Nauty").join("gists.bin"), b"g").unwrap();
        adopt_old_data_in(&root);
        assert_eq!(std::fs::read(root.join(OWN).join("session.bin")).unwrap(), b"n");
        assert!(!root.join(OWN).join("gists.bin").exists(), "only the session and trash list");
        assert!(root.join("Nauty").join("session.bin").is_file(), "an old Nauty keeps its own");
        // Nothing from before: a first run, in the new folder.
        let empty = scratch("adopt-none");
        adopt_old_data_in(&empty);
        assert!(!empty.join(OWN).exists());
        assert_eq!(own_dir(&empty), empty.join(OWN));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn default_vault_keeps_an_existing_notsy_folder() {
        let docs = scratch("vault-notsy");
        assert_eq!(own_vault(&docs), docs.join(OWN));
        std::fs::create_dir_all(docs.join(OLD)).unwrap();
        assert_eq!(own_vault(&docs), docs.join(OLD), "the notes from before stay where they are");
        std::fs::create_dir_all(docs.join(OWN)).unwrap();
        assert_eq!(own_vault(&docs), docs.join(OWN));
        let _ = std::fs::remove_dir_all(&docs);
    }
}

/// Whether this is a test run (`KOLADA_TEST_INSTANCE` set): it keeps apart
/// from a Kolada in use, with its own data, windows and instance.
pub fn testing() -> bool {
    static TESTING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TESTING.get_or_init(|| std::env::var_os("KOLADA_TEST_INSTANCE").is_some())
}
