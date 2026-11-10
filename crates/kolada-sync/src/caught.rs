//! Pages caught with Catcher for Kolada, saved: each clip that comes here goes
//! into the vault's Caught folder as the note it is (the extension writes
//! its Markdown whole: frontmatter, title, the user's line, the page), and
//! each of its pictures beside it, at the place the note embeds it from.
//!
//! Every paired device saves the same clip under the same name with the
//! same bytes, so one that a synced vault already brought is the same file,
//! not a second one.

use std::path::{Path, PathBuf};

use kolada_core::note;

use crate::engine::{Engine, ItemId};

/// The folder clips go into, at the vault's root.
pub const FOLDER: &str = "Caught";

/// Saves each clip that came here and isn't saved yet, and the pictures of
/// every saved clip whose bytes are here and whose file isn't (while its
/// note is still there: one the user deleted stays deleted, pictures and
/// all). The clips saved now, with where.
pub fn catch(e: &mut Engine, vault: &Path) -> Vec<(ItemId, PathBuf)> {
    let clips: Vec<(ItemId, String, String, Option<String>)> = e.cards().iter().filter(|c| c.source.is_some() && !c.mine).map(|c| (c.id, c.title.clone(), c.text.clone(), c.saved.clone())).collect();
    let mut now = Vec::new();
    for (id, title, text, saved) in clips {
        let stem = note::file_stem_for(&title);
        let note = match saved {
            Some(rel) => vault.join(rel),
            None => {
                let Ok(path) = save_note(vault, &stem, &text) else { continue };
                e.caught_saved(id, &relative(vault, &path));
                now.push((id, path.clone()));
                path
            }
        };
        if !note.exists() {
            continue;
        }
        let (Some(dir), Some(own)) = (note.parent(), note.file_stem()) else { continue };
        let own = own.to_string_lossy();
        let pictures: Vec<(ItemId, String)> = e.note_pictures(id).iter().map(|c| (c.id, c.text.clone())).collect();
        for (picture, name) in pictures {
            let Some(to) = picture_path(dir, &renamed(&name, &stem, &own)) else { continue };
            // Its bytes are read only for a file still to write: this runs
            // on every event, for every clip saved.
            if to.exists() {
                continue;
            }
            let Some(bytes) = e.picture(picture) else { continue };
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&to, bytes);
        }
    }
    now
}

/// The clip `text` saved under its title's `stem` in Caught: where. A note
/// already there under that name with the same text is this clip (another
/// device saved it, and the vault synced); one with other text keeps its
/// name, and this one takes the next, its pictures with it (see
/// [`renamed`]), so they never meet another clip's of the same title.
fn save_note(vault: &Path, stem: &str, text: &str) -> std::io::Result<PathBuf> {
    let dir = vault.join(FOLDER);
    std::fs::create_dir_all(&dir)?;
    for n in 1..1000 {
        let own = if n == 1 { stem.to_string() } else { format!("{stem} {n}") };
        let path = dir.join(format!("{own}.md"));
        let text = text.replace(&format!("![[{stem} "), &format!("![[{own} "));
        if !path.exists() {
            note::save(&path, &text, note::Format::default())?;
            return Ok(path);
        }
        if std::fs::read_to_string(&path).is_ok_and(|t| t == text) {
            return Ok(path);
        }
    }
    Err(std::io::ErrorKind::AlreadyExists.into())
}

/// A clip's picture `name` (the extension names them after the title's
/// `stem`, `{stem} 1.jpg`) for its note saved as `own`: after the note's
/// own name, as its text embeds it. Others keep theirs.
fn renamed(name: &str, stem: &str, own: &str) -> String {
    match name.strip_prefix(stem).and_then(|rest| rest.strip_prefix(' ')) {
        Some(rest) if own != stem => format!("{own} {rest}"),
        _ => name.to_string(),
    }
}

/// `path` from the vault's root, `/` between folders.
fn relative(vault: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(vault).unwrap_or(path);
    rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Where a picture a clip embeds as `target` goes, beside the note in
/// `dir`, so the note reads the same. None for one that would leave `dir`.
fn picture_path(dir: &Path, target: &str) -> Option<PathBuf> {
    let t = target.replace("%20", " ").replace('\\', "/");
    if t.starts_with('/') || t.contains(':') {
        return None;
    }
    let mut p = dir.to_path_buf();
    for part in t.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            s => p.push(s),
        }
    }
    (p != dir).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Picture names stay inside the note's folder.
    #[test]
    fn a_picture_never_leaves_the_notes_folder() {
        let dir = Path::new("vault/Caught");
        assert_eq!(picture_path(dir, "walking-1.jpg"), Some(dir.join("walking-1.jpg")));
        assert_eq!(picture_path(dir, "img/a%20b.png"), Some(dir.join("img").join("a b.png")));
        for bad in ["../x.png", "/etc/x.png", "C:/x.png", "a/../../x.png", ""] {
            assert_eq!(picture_path(dir, bad), None, "{bad}");
        }
    }
}
