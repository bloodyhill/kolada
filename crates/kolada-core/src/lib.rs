//! Platform-independent Kolada core.
//!
//! Nothing here may touch the vault as a whole on the startup path: the
//! first frame reads only the session snapshot and one note.

pub mod before;
pub mod fetch;
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

/// The folder a first run puts the vault in: `Documents\Kolada` under the
/// user's profile, or the `Documents\Notsy` an earlier install made (the
/// user's notes are never moved). On Linux the user's Documents folder may
/// be named or placed otherwise (`XDG_DOCUMENTS_DIR` in
/// `~/.config/user-dirs.dirs`).
pub fn default_vault_dir() -> Option<PathBuf> {
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
