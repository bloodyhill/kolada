//! The desktop's own trash (the freedesktop.org Trash spec, which GNOME's
//! Files, KDE's Dolphin and the rest share), where a note removed for good
//! from Kolada's trash goes, so the desktop can still put it back.

use std::path::{Path, PathBuf};

use kolada_app::services::{Bin, Local};

pub struct LinuxBin;

impl Bin for LinuxBin {
    fn recycle(&self, path: &Path) -> bool {
        // The home trash, in the XDG data folder: a rename when the vault is
        // on the same disk; else the desktop's own tool knows the disk's.
        let home_trash = kolada_core::data_root().map(|d| d.join("Trash"));
        if home_trash.is_some_and(|t| recycle_in(&t, path, super::services::now()).is_some()) {
            return true;
        }
        std::process::Command::new("gio").arg("trash").arg("--").arg(path).status().is_ok_and(|s| s.success())
    }
}

/// Moves `path` into the trash at `trash` (its `files` and `info`), under a
/// name not taken there, with the `.trashinfo` that says where it was and
/// when it went. Where it went, or None.
pub fn recycle_in(trash: &Path, path: &Path, now: Local) -> Option<PathBuf> {
    let (files, info) = (trash.join("files"), trash.join("info"));
    std::fs::create_dir_all(&files).ok()?;
    std::fs::create_dir_all(&info).ok()?;
    let absolute = std::path::absolute(path).ok()?;
    let name = absolute.file_name()?.to_string_lossy().into_owned();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let date = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", now.year, now.month, now.day, now.hour, now.minute, now.second);
    for n in 1..1000 {
        let candidate = if n == 1 { name.clone() } else { format!("{stem} {n}{ext}") };
        // The info file is made first, and only if new: it claims the name.
        let info_path = info.join(format!("{candidate}.trashinfo"));
        let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).open(&info_path) else { continue };
        if files.join(&candidate).exists() {
            let _ = std::fs::remove_file(&info_path);
            continue;
        }
        use std::io::Write;
        let body = format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", escaped(&absolute.to_string_lossy()));
        if f.write_all(body.as_bytes()).is_err() {
            let _ = std::fs::remove_file(&info_path);
            return None;
        }
        return match std::fs::rename(&absolute, files.join(&candidate)) {
            Ok(()) => Some(files.join(&candidate)),
            Err(_) => {
                let _ = std::fs::remove_file(&info_path);
                None
            }
        };
    }
    None
}

/// A path as the spec writes it: percent-escaped, but for `/` and the
/// characters URLs leave as they are.
fn escaped(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_goes_to_the_desktops_trash_with_where_it_was_and_when() {
        let dir = std::env::temp_dir().join(format!("kolada-linux-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (vault, trash) = (dir.join("My Notes"), dir.join("Trash"));
        std::fs::create_dir_all(&vault).unwrap();
        let now = Local { year: 2026, month: 10, day: 15, weekday: 4, hour: 9, minute: 5, second: 7 };
        std::fs::write(vault.join("Plan.md"), "one").unwrap();
        let went = recycle_in(&trash, &vault.join("Plan.md"), now).expect("it went");
        assert_eq!(went, trash.join("files").join("Plan.md"));
        assert!(!vault.join("Plan.md").exists());
        let info = std::fs::read_to_string(trash.join("info").join("Plan.md.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\n"));
        assert!(info.contains("/My%20Notes/Plan.md\n"), "{info}");
        assert!(info.contains("DeletionDate=2026-10-15T09:05:07\n"));
        // Another of the same name does not overwrite the first.
        std::fs::write(vault.join("Plan.md"), "two").unwrap();
        let second = recycle_in(&trash, &vault.join("Plan.md"), now).expect("it went too");
        assert_eq!(second, trash.join("files").join("Plan 2.md"));
        assert_eq!(std::fs::read_to_string(&went).unwrap(), "one");
        assert!(recycle_in(&trash, &vault.join("Gone.md"), now).is_none(), "nothing to move");
        assert!(!trash.join("info").join("Gone.md.trashinfo").exists(), "and no info left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
