//! An export's files, wherever they are: inside its zip, or in the folder
//! it was unpacked into. Each by its path with `/` between folders, read
//! when asked for.

use std::path::{Path, PathBuf};

use crate::{Error, zip::Zip};

pub struct Tree<'a> {
    /// Every file's path, and where it is: the zip's entry, or the file.
    files: Vec<(String, Place)>,
    zip: Option<Zip<'a>>,
}

enum Place {
    Entry(usize),
    Disk(PathBuf),
}

/// A folder holding more files than this is not an export to walk whole.
const MOST_FILES: usize = 200_000;
/// Nor one nested deeper.
const MOST_DEPTH: usize = 32;

impl<'a> Tree<'a> {
    /// The files of a zip archive.
    pub fn zip(bytes: &'a [u8]) -> Result<Tree<'a>, Error> {
        let zip = Zip::open(bytes)?;
        let files = zip.names().enumerate().filter(|(_, n)| !n.ends_with('/')).map(|(i, n)| (n.to_string(), Place::Entry(i))).collect();
        Ok(Tree { files, zip: Some(zip) })
    }

    /// The files under a folder (links not followed).
    pub fn dir(root: &Path) -> Result<Tree<'static>, Error> {
        let mut files = Vec::new();
        let mut stack = vec![(root.to_path_buf(), String::new(), 0)];
        while let Some((dir, prefix, depth)) = stack.pop() {
            let entries = std::fs::read_dir(&dir).map_err(|_| Error("the folder could not be read".to_string()))?;
            for e in entries.flatten() {
                let Ok(kind) = e.file_type() else { continue };
                let name = e.file_name().to_string_lossy().into_owned();
                let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
                if kind.is_dir() && depth < MOST_DEPTH {
                    stack.push((e.path(), path, depth + 1));
                } else if kind.is_file() {
                    files.push((path, Place::Disk(e.path())));
                    if files.len() > MOST_FILES {
                        return Err(Error("the folder holds too many files to be an export".to_string()));
                    }
                }
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Tree { files, zip: None })
    }

    /// Every file's path.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|(n, _)| n.as_str())
    }

    /// Whether there is a file at `path` (in any case).
    pub fn has(&self, path: &str) -> bool {
        let path = path.trim_start_matches('/');
        self.files.iter().any(|(n, _)| n.eq_ignore_ascii_case(path))
    }

    /// The file at `path` (case as written, else in any case), if there is one
    /// that reads.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let path = path.trim_start_matches('/');
        let (_, place) = self.files.iter().find(|(n, _)| n == path).or_else(|| self.files.iter().find(|(n, _)| n.eq_ignore_ascii_case(path)))?;
        match place {
            Place::Entry(i) => self.zip.as_ref()?.read_at(*i)?.ok(),
            Place::Disk(p) => std::fs::read(p).ok(),
        }
    }

    /// When the file at `path` last changed, milliseconds since 1970, where
    /// the tree knows (a folder's files; a zip's entries do not say in UTC).
    pub fn modified(&self, path: &str) -> i64 {
        match self.files.iter().find(|(n, _)| n == path) {
            Some((_, Place::Disk(p))) => std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64),
            _ => 0,
        }
    }
}

/// `dir/target`, with `..` and `.` worked out.
pub fn join(dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in target.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// The folder a path is in.
pub fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// A path's last part.
pub fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A file's name without its extension.
pub fn stem(path: &str) -> &str {
    let name = name_of(path);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}
