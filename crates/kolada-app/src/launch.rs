//! Finding the note to show at launch, without reading the whole vault: the
//! vault's top level, newest note first.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What a path on the command line asks for, as on Windows: a note opens in
/// its vault (the nearest folder with `.obsidian`, else its own folder);
/// a folder is the vault; a file that is no note asks for nothing (the
/// vault opens as if no path were given).
pub fn given(path: PathBuf) -> Option<(PathBuf, Option<String>)> {
    if !path.is_file() {
        return Some((path, None));
    }
    if !crate::model::arrive::is_note_file(&path) {
        return None;
    }
    let file = std::path::absolute(&path).unwrap_or(path);
    let root = kolada_core::vault::root_for(&file);
    let rel = kolada_core::vault::relative(&root, &file);
    Some((root, rel))
}

/// The vault: the one given, `KOLADA_VAULT`, or `~/Documents/Kolada` (an earlier `~/Documents/Notsy` when there is one).
pub fn vault(given: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = given {
        return dir;
    }
    if let Some(dir) = std::env::var_os("KOLADA_VAULT") {
        return PathBuf::from(dir);
    }
    kolada_core::default_vault_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// The vault the app opens: the one given or `KOLADA_VAULT`, else the one
/// last open (another chosen in the settings), else the default (`vault`).
pub fn launch_vault(given: Option<PathBuf>) -> PathBuf {
    // The app was called Notsy: its data folder is taken over first.
    kolada_core::adopt_old_data();
    if given.is_none() && std::env::var_os("KOLADA_VAULT").is_none() {
        let last = kolada_core::app_data_dir().map(|d| d.join("session.bin")).and_then(|p| kolada_core::session::load(&p)).map(|s| PathBuf::from(s.vault));
        if let Some(dir) = last.filter(|d| !d.as_os_str().is_empty() && d.is_dir()) {
            return dir;
        }
    }
    vault(given)
}

/// The most recently written Markdown note at the top level of `vault`.
pub fn newest(vault: &Path) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(vault).ok()?.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !name.ends_with(".md") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else { continue };
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_given_on_the_command_line_opens_in_its_vault_and_a_folder_is_the_vault() {
        let dir = std::env::temp_dir().join(format!("kolada-given-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Vault/.obsidian")).unwrap();
        std::fs::create_dir_all(dir.join("Vault/Work")).unwrap();
        std::fs::create_dir_all(dir.join("Loose")).unwrap();
        std::fs::write(dir.join("Vault/Work/Plan.md"), "# Plan\n").unwrap();
        std::fs::write(dir.join("Loose/One.md"), "# One\n").unwrap();
        std::fs::write(dir.join("Loose/Plan.rtf"), "{\\rtf1}").unwrap();
        assert_eq!(given(dir.join("Vault/Work/Plan.md")), Some((dir.join("Vault"), Some("Work/Plan.md".to_string()))));
        assert_eq!(given(dir.join("Loose/One.md")), Some((dir.join("Loose"), Some("One.md".to_string()))));
        assert_eq!(given(dir.join("Vault")), Some((dir.join("Vault"), None)));
        assert_eq!(given(dir.join("Not there")), Some((dir.join("Not there"), None)), "a folder to be made");
        assert_eq!(given(dir.join("Loose/Plan.rtf")), None, "no note: not a vault either");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Where every Notsy beta user is: a session in the old data folder,
    /// pointing at `Documents/Notsy` with a note open. After the rename the
    /// same vault and note open, from the folder they always were in.
    #[test]
    fn a_notsy_users_session_opens_the_same_vault_and_note() {
        let dir = std::env::temp_dir().join(format!("notsy-beta-user-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (data, vault) = (dir.join("Application Support"), dir.join("Documents").join(kolada_core::before::NAME));
        std::fs::create_dir_all(vault.join("Work")).unwrap();
        std::fs::write(vault.join("Work/Plan.md"), "# Plan\n").unwrap();
        let mut s = kolada_core::session::Session::first_run();
        s.vault = vault.to_string_lossy().into_owned();
        s.note = "Work/Plan.md".into();
        let old = data.join(kolada_core::before::NAME);
        std::fs::create_dir_all(&old).unwrap();
        kolada_core::session::save(&old.join("session.bin"), &s).unwrap();
        kolada_core::adopt_old_data_in(&data);
        let moved = data.read_dir().unwrap().flatten().map(|e| e.path()).collect::<Vec<_>>();
        assert_eq!(moved.len(), 1, "one folder, the old one taken over");
        assert_ne!(moved[0], old);
        let back = kolada_core::session::load(&moved[0].join("session.bin")).expect("the session came along");
        assert_eq!((PathBuf::from(&back.vault), back.note.as_str()), (vault.clone(), "Work/Plan.md"));
        assert!(vault.join("Work/Plan.md").is_file(), "the notes never move");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
