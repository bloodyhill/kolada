//! The note a first run opens on: a short welcome with a picture
//! (`assets/welcome`). It goes only into a vault folder this run makes
//! itself, so nothing is ever written among notes that are already there.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The note's file name.
pub const NOTE: &str = "Welcome to Kolada.md";
/// Its picture's file name, embedded by the note (`![[…]]`).
pub const PICTURE: &str = "Welcome to Kolada.png";

const TEXT: &str = include_str!("../../../assets/welcome/Welcome to Kolada.md");
const IMAGE: &[u8] = include_bytes!("../../../assets/welcome/Welcome to Kolada.png");

/// The note's text, with the platform's shortcut key: `Ctrl`, or `⌘`.
pub fn text(modifier: &str) -> String {
    TEXT.replace("{mod}", modifier)
}

/// Makes `vault` and writes the welcome note and its picture into it.
/// Nothing happens when the folder already exists, empty or not: the
/// note's path when written, else None.
pub fn seed(vault: &Path, modifier: &str) -> io::Result<Option<PathBuf>> {
    if let Some(parent) = vault.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::create_dir(vault) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
        Err(e) => return Err(e),
    }
    fs::write(vault.join(PICTURE), IMAGE)?;
    let note = vault.join(NOTE);
    fs::write(&note, text(modifier))?;
    Ok(Some(note))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-welcome-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_new_vault_gets_the_note_and_its_picture() {
        let root = scratch("new");
        let vault = root.join("Documents").join("Kolada");
        let note = seed(&vault, "Ctrl").unwrap().expect("a note in a new vault");
        let text = fs::read_to_string(&note).unwrap();
        // Titled after its file, so no name line is added over it.
        assert_eq!(text.lines().next(), Some("# Welcome to Kolada"));
        assert!(text.contains("**Ctrl+P**") && !text.contains("{mod}"));
        // The picture it embeds is beside it, and is a PNG.
        assert!(text.contains(&format!("![[{PICTURE}]]")));
        assert!(fs::read(vault.join(PICTURE)).unwrap().starts_with(b"\x89PNG"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_that_exists_is_left_alone() {
        // Even empty: it was made by someone, maybe for other notes.
        let vault = scratch("existing");
        fs::create_dir_all(&vault).unwrap();
        assert_eq!(seed(&vault, "Ctrl").unwrap(), None);
        assert_eq!(fs::read_dir(&vault).unwrap().count(), 0);
        fs::write(vault.join("Mine.md"), "kept").unwrap();
        assert_eq!(seed(&vault, "Ctrl").unwrap(), None);
        assert_eq!(fs::read_dir(&vault).unwrap().count(), 1);
        assert_eq!(fs::read_to_string(vault.join("Mine.md")).unwrap(), "kept");
        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn the_shortcut_follows_the_platform() {
        assert!(text("⌘").contains("**⌘+P**"));
    }
}
