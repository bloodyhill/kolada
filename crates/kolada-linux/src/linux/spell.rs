//! Spelling, checked against the desktop's Hunspell dictionaries (the
//! `.aff` and `.dic` files Debian, Ubuntu, Fedora and Arch keep under
//! `/usr/share/hunspell`), read by a pure-Rust reader (`spellbook`), so
//! nothing is linked and nothing runs outside the app. The dictionaries are
//! read on a thread after the first frame (`load`, some 30 ms for English);
//! until then nothing is marked, and the shell starts checking once they are
//! in. Words taught with "Learn Spelling" are kept in the app's data
//! (`words.txt`, one a line).
//!
//! Which dictionaries: the languages the desktop asks for (`LANGUAGE`,
//! `LC_ALL`, `LC_MESSAGES`, `LANG`), each by its exact name (`de_DE`) or
//! another of its language (`de_AT`), and English besides; a word is spelled
//! right when any of them knows it, so notes in two languages read clean.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use spellbook::Dictionary;

/// Where the desktop keeps its dictionaries.
const DIRS: &[&str] = &["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell", "/app/share/hunspell"];

/// No more than this many dictionaries are read (each costs memory).
const MOST: usize = 3;

/// The checker once read.
pub struct Checker {
    dicts: Vec<Dictionary>,
    /// Words taught here, lower-cased; and where they are kept.
    own: Mutex<HashSet<String>>,
    file: Option<PathBuf>,
}

fn checker() -> &'static OnceLock<Option<Checker>> {
    static CHECKER: OnceLock<Option<Checker>> = OnceLock::new();
    &CHECKER
}

/// The languages the desktop asks for, best first, as dictionary names
/// (`en_US`), from the locale's variables as given.
pub fn wanted(language: Option<&str>, all: Option<&str>, messages: Option<&str>, lang: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        // "de_DE.UTF-8@euro" is de_DE; "C" and "POSIX" name no language.
        let name = name.split(['.', '@']).next().unwrap_or("").trim();
        if name.is_empty() || name == "C" || name == "POSIX" || !name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
            return;
        }
        if !out.iter().any(|o| o == name) {
            out.push(name.to_string());
        }
    };
    for list in [language, all, messages, lang].into_iter().flatten() {
        for name in list.split(':') {
            add(name);
        }
    }
    add("en_US");
    out
}

/// The dictionaries to read for `wanted` among the `.dic` files found
/// (their stems: `en_US`, `de_AT`...): each language's exact name, else
/// another of its language, at most `MOST`.
pub fn chosen(wanted: &[String], found: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in wanted {
        let language = w.split('_').next().unwrap_or(w);
        let pick = found.iter().find(|f| *f == w).or_else(|| found.iter().find(|f| f.split('_').next() == Some(language) || *f == language));
        if let Some(p) = pick
            && !out.contains(p)
            && out.len() < MOST
        {
            out.push(p.clone());
        }
    }
    out
}

/// A dictionary file's text: UTF-8, or the Latin-1 that older ones are in
/// (as their `.aff` says with `SET`); None for other encodings.
pub fn decode(bytes: &[u8], set: &str) -> Option<String> {
    match set.to_ascii_uppercase().replace('-', "").as_str() {
        "" | "UTF8" => Some(String::from_utf8_lossy(bytes).into_owned()),
        "ISO88591" | "ISO885915" | "LATIN1" => Some(bytes.iter().map(|&b| b as char).collect()),
        _ => None,
    }
}

/// The `SET` an `.aff` names.
fn set_of(aff: &[u8]) -> String {
    aff.split(|&b| b == b'\n')
        .find_map(|l| l.strip_prefix(b"SET "))
        .map(|s| String::from_utf8_lossy(s).trim().to_string())
        .unwrap_or_default()
}

fn read_dictionary(aff: &Path, dic: &Path) -> Option<Dictionary> {
    let aff = std::fs::read(aff).ok()?;
    let dic = std::fs::read(dic).ok()?;
    let set = set_of(&aff);
    Dictionary::new(&decode(&aff, &set)?, &decode(&dic, &set)?).ok()
}

/// Reads the dictionaries and the words taught (from `data`, the app's
/// folder): slow, so on a thread of its own, once. Whether there is a checker.
pub fn load(data: Option<&Path>) -> bool {
    checker()
        .get_or_init(|| {
            let env = |k: &str| std::env::var(k).ok();
            let wanted = wanted(env("LANGUAGE").as_deref(), env("LC_ALL").as_deref(), env("LC_MESSAGES").as_deref(), env("LANG").as_deref());
            let mut found: Vec<(String, PathBuf)> = Vec::new();
            for dir in DIRS {
                let Ok(entries) = std::fs::read_dir(dir) else { continue };
                for e in entries.flatten() {
                    let path = e.path();
                    if path.extension().is_some_and(|x| x == "dic")
                        && path.with_extension("aff").is_file()
                        && let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned())
                        && !found.iter().any(|(s, _)| *s == stem)
                    {
                        found.push((stem, path));
                    }
                }
            }
            let names: Vec<String> = found.iter().map(|(s, _)| s.clone()).collect();
            let dicts: Vec<Dictionary> = chosen(&wanted, &names)
                .iter()
                .filter_map(|name| {
                    let dic = &found.iter().find(|(s, _)| s == name)?.1;
                    read_dictionary(&dic.with_extension("aff"), dic)
                })
                .collect();
            if dicts.is_empty() {
                return None;
            }
            let file = data.map(|d| d.join("words.txt"));
            let own = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).map(|t| t.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect()).unwrap_or_default();
            Some(Checker { dicts, own: Mutex::new(own), file })
        })
        .is_some()
}

/// The words of `units`: runs of letters (and the marks that go on them),
/// with an apostrophe inside a word kept in it ("don't"), as UTF-16 ranges.
pub fn words(units: &[u16]) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = {
        let mut at = 0;
        char::decode_utf16(units.iter().copied())
            .map(|c| {
                let c = c.unwrap_or('\u{FFFD}');
                let here = at;
                at += c.len_utf16();
                (here, c)
            })
            .collect()
    };
    let letter = |c: char| c.is_alphabetic() || is_mark(c);
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !letter(chars[i].1) {
            i += 1;
            continue;
        }
        let start = chars[i].0;
        let mut j = i + 1;
        while j < chars.len() && (letter(chars[j].1) || (matches!(chars[j].1, '\'' | '\u{2019}') && chars.get(j + 1).is_some_and(|c| letter(c.1)))) {
            j += 1;
        }
        // Letters run into digits or an underscore: a code, a name, not a word.
        let joined = |k: Option<&(usize, char)>| k.is_some_and(|c| c.1.is_ascii_digit() || c.1 == '_');
        if !joined(chars.get(j)) && !(i > 0 && joined(chars.get(i - 1))) {
            let end = chars.get(j).map_or(units.len(), |c| c.0);
            out.push((start, end));
        }
        i = j;
    }
    out
}

/// Combining marks (Arabic's vowels, accents typed apart): part of a word.
fn is_mark(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED | 0x1AB0..=0x1AFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

impl Checker {
    /// Whether `word` is spelled right by any dictionary, or taught here.
    pub fn knows(&self, word: &str) -> bool {
        let word = word.replace('\u{2019}', "'");
        // A letter alone, and words all in capitals (names, acronyms), pass.
        if word.chars().count() < 2 || (word.chars().all(|c| !c.is_lowercase()) && word.chars().any(char::is_uppercase)) {
            return true;
        }
        if self.own.lock().is_ok_and(|o| o.contains(&word.to_lowercase())) {
            return true;
        }
        self.dicts.iter().any(|d| d.check(&word))
    }

    pub fn misspelled(&self, units: &[u16]) -> Vec<(usize, usize)> {
        words(units).into_iter().filter(|&(a, b)| !self.knows(&String::from_utf16_lossy(&units[a..b]))).collect()
    }

    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        for d in &self.dicts {
            let mut some = Vec::new();
            d.suggest(&word.replace('\u{2019}', "'"), &mut some);
            for s in some {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
            if out.len() >= 5 {
                break;
            }
        }
        out.truncate(5);
        out
    }

    pub fn learn(&self, word: &str) {
        let word = word.trim().to_lowercase();
        if word.is_empty() || word.contains('\n') {
            return;
        }
        let Ok(mut own) = self.own.lock() else { return };
        if !own.insert(word.clone()) {
            return;
        }
        if let Some(file) = &self.file {
            use std::io::Write;
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
                let _ = writeln!(f, "{word}");
            }
        }
    }
}

/// For the model's `Spell` service: nothing is marked until `load` is done.
pub struct LinuxSpell;

impl kolada_app::services::Spell for LinuxSpell {
    fn check(&self, units: &[u16]) -> Vec<(usize, usize)> {
        match checker().get() {
            Some(Some(c)) => c.misspelled(units),
            _ => Vec::new(),
        }
    }

    fn learn(&self, word: &str) {
        if let Some(Some(c)) = checker().get() {
            c.learn(word);
        }
    }

    fn suggest(&self, word: &str) -> Vec<String> {
        match checker().get() {
            Some(Some(c)) => c.suggest(word),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_desktops_languages_choose_the_dictionaries_with_english_besides() {
        assert_eq!(wanted(None, None, None, Some("de_DE.UTF-8")), s(&["de_DE", "en_US"]));
        assert_eq!(wanted(Some("ar:fr_FR"), None, None, Some("C.UTF-8")), s(&["ar", "fr_FR", "en_US"]));
        assert_eq!(wanted(None, None, None, Some("C")), s(&["en_US"]), "no language: English");
        assert_eq!(wanted(None, Some("en_GB.UTF-8@x"), None, Some("en_GB")), s(&["en_GB", "en_US"]));
        let found = s(&["en_US", "en_GB", "de_AT", "ar", "fr_FR", "es_ES"]);
        assert_eq!(chosen(&s(&["de_DE", "en_US"]), &found), s(&["de_AT", "en_US"]), "another of the language when the exact one is missing");
        assert_eq!(chosen(&s(&["en_GB", "en_US"]), &found), s(&["en_GB", "en_US"]));
        assert_eq!(chosen(&s(&["ar_SA", "fr_FR", "es_ES", "en_US"]), &found).len(), MOST, "no more than three");
        assert!(chosen(&s(&["ja_JP"]), &found).is_empty());
    }

    #[test]
    fn words_are_runs_of_letters_with_apostrophes_inside_and_codes_left_alone() {
        let units: Vec<u16> = "Don't stop, teh x2 abc_def caf\u{E9} \u{645}\u{64E}\u{631}\u{62D}\u{628}\u{627} 'quoted'".encode_utf16().collect();
        let got: Vec<String> = words(&units).into_iter().map(|(a, b)| String::from_utf16_lossy(&units[a..b])).collect();
        assert_eq!(got, ["Don't", "stop", "teh", "caf\u{E9}", "\u{645}\u{64E}\u{631}\u{62D}\u{628}\u{627}", "quoted"]);
    }

    #[test]
    fn latin1_dictionaries_are_read_as_their_aff_says() {
        assert_eq!(decode(b"caf\xE9", "ISO8859-1").as_deref(), Some("caf\u{E9}"));
        assert_eq!(decode("caf\u{E9}".as_bytes(), "UTF-8").as_deref(), Some("caf\u{E9}"));
        assert_eq!(decode(b"x", "KOI8-R"), None);
        assert_eq!(set_of(b"# x\nSET ISO8859-1\nTRY abc\n"), "ISO8859-1");
    }

    #[test]
    fn misspellings_are_found_suggested_and_learned() {
        let aff = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz'\n\nSFX S Y 1\nSFX S 0 s .\n";
        let dic = "4\nquick\nbrown\nfox/S\nnote/S\n";
        let dir = std::env::temp_dir().join(format!("kolada-linux-spell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let c = Checker { dicts: vec![Dictionary::new(aff, dic).unwrap()], own: Mutex::new(HashSet::new()), file: Some(dir.join("words.txt")) };
        let units: Vec<u16> = "quick brwon foxs, NASA notes".encode_utf16().collect();
        let marked: Vec<String> = c.misspelled(&units).into_iter().map(|(a, b)| String::from_utf16_lossy(&units[a..b])).collect();
        assert_eq!(marked, ["brwon"], "capitals and known words pass");
        assert!(c.suggest("brwon").contains(&"brown".to_string()));
        c.learn("Brwon");
        assert!(c.knows("brwon"));
        assert_eq!(std::fs::read_to_string(dir.join("words.txt")).unwrap(), "brwon\n", "kept for the next launch");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_desktops_english_dictionary_is_read_when_there_is_one() {
        let Some(dir) = DIRS.iter().map(Path::new).find(|d| d.join("en_US.dic").is_file()) else { return };
        let started = std::time::Instant::now();
        let d = read_dictionary(&dir.join("en_US.aff"), &dir.join("en_US.dic")).expect("it reads");
        eprintln!("en_US read in {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
        assert!(d.check("receive") && !d.check("recieve"));
    }
}
