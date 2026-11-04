//! The machine's own fonts, for Settings › Font › Other: read on a worker
//! when the finder first opens, never at launch. A family chosen is kept
//! by its files, so a launch reads those and looks through nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cosmic_text::fontdb;

/// A family of the machine's: its name, and the files that hold it,
/// regular first, then bold, italic and bold italic, then the rest.
#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    pub name: String,
    pub files: Vec<PathBuf>,
}

/// The most files a family is kept by.
pub const FILES: usize = 8;

/// The font files in `dirs` and the folders in them.
pub fn files_in(dirs: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let path = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if depth < 4 {
                    walk(&path, depth + 1, out);
                }
            } else if path.extension().and_then(|x| x.to_str()).is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc")) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        walk(d, 0, &mut out);
    }
    out
}

/// The families `files` hold, by name (A to Z, whatever the case), each
/// with its files. Names a font keeps for vertical writing (`@`) and
/// families of the app's own icons are left out.
pub fn families(files: &[PathBuf]) -> Vec<Family> {
    let mut db = fontdb::Database::new();
    for f in files {
        let _ = db.load_font_file(f);
    }
    let mut by_name: BTreeMap<String, (String, Vec<(u32, PathBuf)>)> = BTreeMap::new();
    for face in db.faces() {
        let Some((name, _)) = face.families.first() else { continue };
        if name.is_empty() || name.starts_with('@') || name.contains("Icons") || name.contains("MDL2") {
            continue;
        }
        let path = match &face.source {
            fontdb::Source::File(p) => p.clone(),
            fontdb::Source::SharedFile(p, _) => p.clone(),
            fontdb::Source::Binary(_) => continue,
        };
        let bold = face.weight.0 >= 600;
        let italic = face.style != fontdb::Style::Normal;
        let rank = match (face.weight.0, bold, italic) {
            (400, false, false) => 0,
            (_, true, false) if face.weight.0 == 700 => 1,
            (400, false, true) => 2,
            (700, true, true) => 3,
            (w, ..) => 4 + u32::from(w) / 100,
        };
        let entry = by_name.entry(name.to_lowercase()).or_insert_with(|| (name.clone(), Vec::new()));
        entry.1.push((rank, path));
    }
    by_name
        .into_values()
        .map(|(name, mut files)| {
            files.sort();
            let mut kept: Vec<PathBuf> = Vec::new();
            for (_, f) in files {
                if !kept.contains(&f) && kept.len() < FILES {
                    kept.push(f);
                }
            }
            Family { name, files: kept }
        })
        .collect()
}

/// The places of `names` that match what is typed: those that start with
/// it first, then those with a word that does, then any that hold it; in
/// their own order within each. Empty, every name.
pub fn matching(names: &[&str], typed: &str) -> Vec<usize> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return (0..names.len()).collect();
    }
    let mut ranked: Vec<(u8, usize)> = names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| {
            let n = n.to_lowercase();
            let rank = if n.starts_with(&typed) {
                0
            } else if n.split([' ', '-']).any(|w| w.starts_with(&typed)) {
                1
            } else if n.contains(&typed) {
                2
            } else {
                return None;
            };
            Some((rank, i))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_typed_finds_names_that_start_with_it_first() {
        let names = ["Book Antiqua", "Cambria", "Candara", "Courier New", "Palatino Linotype", "Segoe UI", "Sitka Text"];
        let found = |q: &str| matching(&names, q).into_iter().map(|i| names[i]).collect::<Vec<_>>();
        assert_eq!(found("ca"), ["Cambria", "Candara"]);
        assert_eq!(found("t"), ["Sitka Text", "Book Antiqua", "Palatino Linotype"], "a word that starts with it, then anywhere");
        assert_eq!(found("NEW"), ["Courier New"], "whatever the case");
        assert_eq!(found("ia"), ["Cambria"], "anywhere in the name last");
        assert_eq!(found("zz"), Vec::<&str>::new());
        assert_eq!(found("  ").len(), names.len(), "nothing typed: every name");
    }

    /// A folder of the test's own, from the repo's fonts (the public ones):
    /// fonts are found in the folders under it, a family's files are kept
    /// together, the app's icons are no font to write in, and anything else
    /// is passed over.
    #[test]
    fn the_test_fonts_read_as_families_with_their_files() {
        let fonts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
        let dir = std::env::temp_dir().join(format!("kolada-text-installed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("More/Deeper")).unwrap();
        std::fs::copy(fonts.join("Prata-Regular.ttf"), dir.join("Prata-Regular.ttf")).unwrap();
        // The same family again, further down: one family, two files.
        std::fs::copy(fonts.join("Prata-Regular.ttf"), dir.join("More/Deeper/Prata-Copy.ttf")).unwrap();
        std::fs::copy(fonts.join("KoladaIcons.ttf"), dir.join("More/KoladaIcons.ttf")).unwrap();
        std::fs::write(dir.join("notes.txt"), "not a font").unwrap();
        let files = files_in(std::slice::from_ref(&dir));
        assert_eq!(files.len(), 3, "the fonts, not the text file");
        let found = families(&files);
        let names: Vec<String> = found.iter().map(|f| f.name.to_lowercase()).collect();
        assert_eq!(names, ["prata"], "one family; the app's icons are not a font to write in");
        assert_eq!(found[0].files.len(), 2, "both its files, as one family");
        assert!(found[0].files.len() <= FILES && !found[0].name.starts_with('@'));
        let _ = std::fs::remove_dir_all(dir);
    }
}
