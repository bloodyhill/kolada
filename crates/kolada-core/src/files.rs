//! Files written where the user picks, or put on the clipboard as files:
//! never over one already there.

use std::path::{Path, PathBuf};

/// Where `name` can go in `dir` without overwriting anything: as it is,
/// else with " (2)", " (3)"… before its extension.
pub fn unique(dir: &Path, name: &str) -> PathBuf {
    let name = clean(name);
    let first = dir.join(&name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Each of `files` (a name and its bytes) written into `dir`, none over
/// another: where each went, in order.
pub fn save_all(dir: &Path, files: &[(String, Vec<u8>)]) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut out = Vec::with_capacity(files.len());
    for (name, bytes) in files {
        let to = unique(dir, name);
        std::fs::write(&to, bytes)?;
        out.push(to);
    }
    Ok(out)
}

/// Pictures to put on the clipboard as files: written fresh into `dir`
/// (Kolada's own folder for copies, never the vault), last time's gone.
/// Where each is, in order.
pub fn copies(dir: &Path, files: &[(String, Vec<u8>)]) -> std::io::Result<Vec<PathBuf>> {
    let _ = std::fs::remove_dir_all(dir);
    save_all(dir, files)
}

/// A file name as it may be written: no folders, nothing a file system
/// refuses; "Picture" if nothing is left.
fn clean(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let s: String = base.chars().map(|c| if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "Picture".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saving all never overwrites: a name already there, or twice among
    /// them, gets " (2)", " (3)"; folders in a name never lead out.
    #[test]
    fn saving_all_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("kolada-save-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("IMG_1.jpg"), b"old").unwrap();
        let files = vec![("IMG_1.jpg".to_string(), b"a".to_vec()), ("IMG_1.jpg".to_string(), b"b".to_vec()), ("../up.png".to_string(), b"c".to_vec()), ("".to_string(), b"d".to_vec())];
        let went = save_all(&dir, &files).unwrap();
        let names: Vec<String> = went.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["IMG_1 (2).jpg", "IMG_1 (3).jpg", "up.png", "Picture"]);
        assert_eq!(std::fs::read(dir.join("IMG_1.jpg")).unwrap(), b"old", "never overwritten");
        assert!(went.iter().all(|p| p.parent() == Some(dir.as_path())));
        // Copies: only these, each once, last time's gone.
        let copied = dir.join("copied");
        copies(&copied, &[("a.png".into(), b"1".to_vec())]).unwrap();
        let now = copies(&copied, &[("b.png".into(), b"2".to_vec()), ("b.png".into(), b"3".to_vec())]).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(&copied).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec!["b (2).png", "b.png"]);
        assert_eq!(now.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
