//! Note files: reading and writing one note, and naming new ones.
//!
//! In memory a note is UTF-8 with `\n` line breaks. The file keeps what it
//! had: a byte-order mark and CRLF line breaks are remembered on load and
//! written back on save, so opening and editing a note never rewrites the
//! parts the user did not touch.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// How a note's file encodes its text, beyond the text itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Format {
    pub bom: bool,
    pub crlf: bool,
}

/// A note's text and file format from its bytes. Invalid UTF-8 is replaced
/// rather than refused, so a damaged file still opens.
pub fn decode(bytes: &[u8]) -> (String, Format) {
    let (bom, bytes) = match bytes.strip_prefix(b"\xEF\xBB\xBF") {
        Some(rest) => (true, rest),
        None => (false, bytes),
    };
    let text = String::from_utf8_lossy(bytes);
    // The first line break decides; mixed files are normalized to it.
    let crlf = text.find('\n').is_some_and(|i| text[..i].ends_with('\r'));
    let text = if text.contains('\r') { text.replace("\r\n", "\n") } else { text.into_owned() };
    (text, Format { bom, crlf })
}

pub fn encode(text: &str, format: Format) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 3);
    if format.bom {
        out.extend_from_slice(b"\xEF\xBB\xBF");
    }
    if format.crlf {
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(line.as_bytes());
        }
    } else {
        out.extend_from_slice(text.as_bytes());
    }
    out
}

pub fn load(path: &Path) -> io::Result<(String, Format)> {
    Ok(decode(&fs::read(path)?))
}

/// Writes the note atomically: a hidden temporary file beside it, then a
/// rename over the old file, so a crash mid-save leaves the old note or the
/// new one, never half of each. The note keeps the time it was created (the
/// new file would otherwise be "created just now" on every save).
pub fn save(path: &Path, text: &str, format: Format) -> io::Result<()> {
    let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let tmp = path.with_file_name(format!(".{}.kolada-tmp", name.to_string_lossy()));
    let created = fs::metadata(path).and_then(|m| m.created()).ok();
    fs::write(&tmp, encode(text, format))?;
    if let Some(t) = created {
        keep_created(&tmp, t);
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Gives `file` the creation time `t`, where the OS lets a program set it
/// (Windows, macOS, iOS; Linux keeps no settable birth time). Best effort:
/// the note is saved either way.
fn keep_created(file: &Path, t: std::time::SystemTime) {
    #[cfg(windows)]
    use std::os::windows::fs::FileTimesExt;
    #[cfg(target_os = "macos")]
    use std::os::macos::fs::FileTimesExt;
    #[cfg(target_os = "ios")]
    use std::os::ios::fs::FileTimesExt;
    #[cfg(any(windows, target_os = "macos", target_os = "ios"))]
    if let Ok(f) = fs::OpenOptions::new().write(true).open(file) {
        let _ = f.set_times(fs::FileTimes::new().set_created(t));
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "ios")))]
    let _ = (file, t);
}

/// A file name (without `.md`) for a note titled `title`: characters that
/// Windows or wiki links cannot hold are dropped, and an empty title
/// becomes "Untitled".
pub fn file_stem_for(title: &str) -> String {
    const MAX_CHARS: usize = 100;
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']'))
        .collect();
    let mut stem = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((cut, _)) = stem.char_indices().nth(MAX_CHARS) {
        stem.truncate(cut);
    }
    // Windows drops trailing dots and spaces, and a leading dot hides a note.
    let stem = stem.trim_end_matches(['.', ' ']).trim_start_matches(['.', ' ']);
    if stem.is_empty() {
        return "Untitled".to_string();
    }
    let device = stem.split('.').next().unwrap_or(stem).to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device.len() == 4
            && (device.starts_with("COM") || device.starts_with("LPT"))
            && device.as_bytes()[3].is_ascii_digit());
    if reserved { format!("{stem} note") } else { stem.to_string() }
}

/// `dir/stem.md`, or `dir/stem 2.md`, `dir/stem 3.md`, ... if taken.
/// `current`, the note being renamed, never counts as taken.
pub fn unique_path(dir: &Path, stem: &str, current: Option<&Path>) -> PathBuf {
    (1..)
        .map(|n| dir.join(if n == 1 { format!("{stem}.md") } else { format!("{stem} {n}.md") }))
        .find(|p| Some(p.as_path()) == current || !p.exists())
        .expect("a free name exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_bom_and_line_endings() {
        for bytes in [&b"a\r\nb\r\n"[..], b"\xEF\xBB\xBFa\nb", b"a\nb\n", b"", b"\xEF\xBB\xBF# t\r\n\r\nx"] {
            let (text, format) = decode(bytes);
            assert!(!text.contains('\r'));
            assert_eq!(encode(&text, format), bytes, "roundtrip of {bytes:?}");
        }
        assert_eq!(decode(b"a\r\nb").1, Format { bom: false, crlf: true });
        assert_eq!(decode(b"\xEF\xBB\xBFa").1, Format { bom: true, crlf: false });
    }

    #[test]
    fn opens_damaged_files() {
        let (text, _) = decode(b"ok \xFF\xFE end");
        assert!(text.starts_with("ok ") && text.ends_with(" end"));
    }

    /// A save writes a new file and renames it over the note; the note
    /// still says when it was first made (its caption's "Created"), where
    /// the OS lets that time be kept.
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn a_save_keeps_when_the_note_was_created() {
        let dir = std::env::temp_dir().join(format!("kolada-note-created-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Plan.md");
        fs::write(&path, "# Plan
").unwrap();
        let made = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_760_000_000);
        keep_created(&path, made);
        assert_eq!(fs::metadata(&path).unwrap().created().unwrap(), made, "the test's own starting point");
        save(&path, "# Plan

more", Format::default()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Plan

more");
        assert_eq!(fs::metadata(&path).unwrap().created().unwrap(), made, "still created when it was");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn saves_atomically() {
        let dir = std::env::temp_dir().join(format!("kolada-note-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("n.md");
        let format = Format { bom: false, crlf: true };
        save(&path, "one\ntwo", format).unwrap();
        save(&path, "one\nthree", format).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one\r\nthree");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no temporary file left behind");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_files_after_titles() {
        assert_eq!(file_stem_for("Groceries"), "Groceries");
        assert_eq!(file_stem_for("  What: now?  "), "What now");
        assert_eq!(file_stem_for("a/b\\c [[link]] #tag ^block"), "abc link tag block");
        assert_eq!(file_stem_for("..."), "Untitled");
        assert_eq!(file_stem_for(""), "Untitled");
        assert_eq!(file_stem_for(".hidden"), "hidden");
        assert_eq!(file_stem_for("end."), "end");
        assert_eq!(file_stem_for("con"), "con note");
        assert_eq!(file_stem_for("COM1.txt"), "COM1.txt note");
        assert_eq!(file_stem_for("Console"), "Console");
        assert_eq!(file_stem_for("ملاحظة 👍🏽"), "ملاحظة 👍🏽");
        assert_eq!(file_stem_for(&"x".repeat(150)).len(), 100);
    }

    #[test]
    fn finds_a_free_name() {
        let dir = std::env::temp_dir().join(format!("kolada-note-names-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "Plan", None), dir.join("Plan.md"));
        fs::write(dir.join("Plan.md"), "").unwrap();
        fs::write(dir.join("Plan 2.md"), "").unwrap();
        assert_eq!(unique_path(&dir, "Plan", None), dir.join("Plan 3.md"));
        let current = dir.join("Plan 2.md");
        assert_eq!(unique_path(&dir, "Plan", Some(&current)), current, "renaming to itself");
        fs::remove_dir_all(&dir).unwrap();
    }
}
