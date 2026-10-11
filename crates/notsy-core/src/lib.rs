//! Platform-independent Notsy core.
//!
//! Nothing here may touch the vault as a whole on the startup path: the
//! first frame reads only the session snapshot and one note.

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
pub mod social;
pub mod sqlite;
pub mod trash;
pub mod update;
pub mod vault;
pub mod welcome;
pub mod when;

use std::path::PathBuf;

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Where per-user app data folders live: `%LOCALAPPDATA%` on Windows,
/// `~/Library/Application Support` on macOS and iOS (where `HOME` is the
/// app's container), `$XDG_DATA_HOME` or `~/.local/share` elsewhere. Read from the environment rather than the
/// shell API so shell32 stays off the startup path.
pub fn data_root() -> Option<PathBuf> {
    if cfg!(windows) {
        env_path("LOCALAPPDATA")
    } else if cfg!(any(target_os = "macos", target_os = "ios")) {
        env_path("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        xdg_data_root(env_path("XDG_DATA_HOME"), env_path("HOME"))
    }
}

/// The XDG rule: `$XDG_DATA_HOME` counts only when it is an absolute path
/// (a relative one is to be ignored), else `~/.local/share`.
fn xdg_data_root(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    xdg.filter(|p| p.is_absolute()).or_else(|| home.map(|h| h.join(".local").join("share")))
}

/// The folder a first run puts the vault in: `Documents\Notsy` under the
/// user's profile. On Linux the user's Documents folder may be named or
/// placed otherwise (`XDG_DOCUMENTS_DIR` in `~/.config/user-dirs.dirs`).
pub fn default_vault_dir() -> Option<PathBuf> {
    let home = if cfg!(windows) { env_path("USERPROFILE") } else { env_path("HOME") }?;
    let documents = if cfg!(any(windows, target_os = "macos", target_os = "ios")) {
        None
    } else {
        let config = env_path("XDG_CONFIG_HOME").filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".config"));
        std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|t| xdg_documents_dir(&t, &home))
    };
    Some(documents.unwrap_or_else(|| home.join("Documents")).join("Notsy"))
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

/// `Notsy` inside `data_root`. Under test (see `testing`) a folder of its
/// own inside it.
pub fn app_data_dir() -> Option<PathBuf> {
    let dir = data_root()?.join("Notsy");
    Some(if testing() { dir.join("test") } else { dir })
}

/// The app was called Nauty before: the first time Notsy runs, it takes
/// the old app's session and trash list (copies, so an old Nauty still
/// running keeps its own). Costs one look at a folder on later launches.
pub fn adopt_old_data() {
    let Some(local) = data_root() else { return };
    let (old, new) = (local.join("Nauty"), local.join("Notsy"));
    if testing() || new.exists() || !old.is_dir() {
        return;
    }
    if std::fs::create_dir_all(&new).is_ok() {
        for file in ["session.bin", "trash.tsv"] {
            let _ = std::fs::copy(old.join(file), new.join(file));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

/// Whether this is a test run (`NOTSY_TEST_INSTANCE` set): it keeps apart
/// from a Notsy in use, with its own data, windows and instance.
pub fn testing() -> bool {
    static TESTING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TESTING.get_or_init(|| std::env::var_os("NOTSY_TEST_INSTANCE").is_some())
}
