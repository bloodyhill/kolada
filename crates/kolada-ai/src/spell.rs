//! The dictionary's guesses for the words it does not know, handed to the
//! model with the paragraph: `whaere → where`. The model still decides from
//! the sentence what was meant; the guard still checks what it changed.
//! The OS's own spell checkers, the ones every app shares: Windows'
//! ISpellChecker, the Mac's NSSpellChecker. Elsewhere no hints are sent, and
//! the model works without them.

use kolada_assist::prompt::Lang;

/// Words hinted at most for one paragraph.
const MOST: usize = 6;

#[cfg(windows)]
mod os {
    use std::collections::HashMap;

    use windows::Win32::Globalization::{ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};
    use windows::core::{HSTRING, PCWSTR, PWSTR};

    pub struct Dictionaries {
        factory: Option<ISpellCheckerFactory>,
        by_tag: HashMap<&'static str, Option<ISpellChecker>>,
    }

    impl Dictionaries {
        /// On the thread that will use it: COM objects stay on theirs.
        pub fn new() -> Dictionaries {
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let factory = unsafe { CoCreateInstance::<_, ISpellCheckerFactory>(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER) }.ok();
            Dictionaries { factory, by_tag: HashMap::new() }
        }

        fn checker(&mut self, tags: &[&'static str]) -> Option<ISpellChecker> {
            let factory = self.factory.as_ref()?;
            tags.iter()
                .find_map(|&tag| {
                    self.by_tag
                        .entry(tag)
                        .or_insert_with(|| unsafe {
                            let t = HSTRING::from(tag);
                            factory.IsSupported(&t).ok().filter(|s| s.as_bool()).and_then(|_| factory.CreateSpellChecker(&t).ok())
                        })
                        .clone()
                })
        }

        /// The words of `text` the dictionary for `tags` does not know, as
        /// UTF-16 ranges, with its first guesses for each.
        pub fn unknown(&mut self, tags: &[&'static str], text: &str) -> Vec<(String, Vec<String>)> {
            let Some(checker) = self.checker(tags) else { return Vec::new() };
            let mut units: Vec<u16> = text.encode_utf16().collect();
            let n = units.len();
            units.push(0);
            let Ok(errors) = (unsafe { checker.Check(PCWSTR(units.as_ptr())) }) else { return Vec::new() };
            let mut out = Vec::new();
            loop {
                let mut e = None;
                if unsafe { errors.Next(&mut e) }.0 != 0 {
                    break;
                }
                let Some(e) = e else { break };
                let (Ok(start), Ok(len)) = (unsafe { e.StartIndex() }, unsafe { e.Length() }) else { continue };
                let (a, b) = (start as usize, (start + len) as usize);
                if b > n || a >= b {
                    continue;
                }
                let word = String::from_utf16_lossy(&units[a..b]);
                let guesses = suggest(&checker, &word);
                out.push((word, guesses));
            }
            out
        }
    }

    fn suggest(checker: &ISpellChecker, word: &str) -> Vec<String> {
        let Ok(list) = (unsafe { checker.Suggest(&HSTRING::from(word)) }) else { return Vec::new() };
        let mut out = Vec::new();
        while out.len() < 2 {
            let mut item = [PWSTR::null()];
            let mut fetched = 0;
            if unsafe { list.Next(&mut item, Some(&mut fetched)) }.0 != 0 || fetched == 0 {
                break;
            }
            if let Ok(s) = unsafe { item[0].to_string() } {
                out.push(s);
            }
            unsafe { CoTaskMemFree(Some(item[0].0 as *const _)) };
        }
        out
    }
}

#[cfg(target_os = "macos")]
mod os {
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSNotFound, NSRange, NSString};

    pub struct Dictionaries {
        /// The Mac's names for its dictionaries (`en`, `en_GB`, `pt_BR`...).
        available: Option<Vec<String>>,
    }

    impl Dictionaries {
        pub fn new() -> Dictionaries {
            Dictionaries { available: None }
        }

        /// The Mac's dictionary for the first of `tags` it has.
        fn language(&mut self, tags: &[&'static str]) -> Option<String> {
            let available = self.available.get_or_insert_with(|| NSSpellChecker::sharedSpellChecker().availableLanguages().iter().map(|l| l.to_string()).collect());
            super::mac_language(tags, available)
        }

        /// The words of `text` the dictionary for `tags` does not know,
        /// with its first two guesses for each, read in the sentence.
        pub fn unknown(&mut self, tags: &[&'static str], text: &str) -> Vec<(String, Vec<String>)> {
            let Some(language) = self.language(tags) else { return Vec::new() };
            let language = NSString::from_str(&language);
            let checker = NSSpellChecker::sharedSpellChecker();
            let units: Vec<u16> = text.encode_utf16().collect();
            let string = NSString::from_str(text);
            let mut out = Vec::new();
            let mut start = 0isize;
            while (start as usize) < units.len() {
                // Without wrapping, or it comes back round to the first one.
                let range = unsafe { checker.checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(&string, start, Some(&language), false, 0, std::ptr::null_mut()) };
                let (a, b) = (range.location, range.location + range.length);
                if range.location == NSNotFound as usize || range.length == 0 || (a as isize) < start || b > units.len() {
                    break;
                }
                let word = String::from_utf16_lossy(&units[a..b]);
                let guesses = checker
                    .guessesForWordRange_inString_language_inSpellDocumentWithTag(NSRange::new(a, b - a), &string, Some(&language), 0)
                    .map(|list| list.iter().take(2).map(|g| g.to_string()).collect())
                    .unwrap_or_default();
                out.push((word, guesses));
                start = b as isize;
            }
            out
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod os {
    pub struct Dictionaries;

    impl Dictionaries {
        pub fn new() -> Dictionaries {
            Dictionaries
        }

        pub fn unknown(&mut self, _: &[&'static str], _: &str) -> Vec<(String, Vec<String>)> {
            Vec::new()
        }
    }
}

pub use os::Dictionaries;

/// The Mac's name for the first of `tags` (BCP 47, `en-US`) among its
/// dictionaries: the tag itself (`en_US`), else its language alone (`en`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mac_language(tags: &[&str], available: &[String]) -> Option<String> {
    tags.iter().find_map(|tag| {
        let exact = tag.replace('-', "_");
        let alone = tag.split('-').next().unwrap_or(tag);
        [exact.as_str(), alone].into_iter().find(|t| available.iter().any(|a| a == t)).map(str::to_string)
    })
}

/// The hint line for `text` from the dictionary's `unknown` words and their
/// guesses: `whaere (maybe where), tosdat (maybe toasted or today)`, at most
/// six, two guesses each. Not a word with a capital after the first (it may
/// be a name the dictionary lacks), a number, a protected marker, or one
/// with no guess.
fn hint_line(text: &str, unknown: Vec<(String, Vec<String>)>) -> String {
    let first = text.split_whitespace().next().unwrap_or("");
    let mut out: Vec<String> = Vec::new();
    for (word, guesses) in unknown {
        let named = word.starts_with(char::is_uppercase) && !first.starts_with(word.as_str());
        if guesses.is_empty() || named || word.chars().any(|c| c.is_numeric() || c == '\u{27E6}' || c == '\u{27E7}') {
            continue;
        }
        // In words, not marks: the model copies an arrow into its answer.
        let guesses: Vec<&str> = guesses.iter().take(2).map(String::as_str).collect();
        out.push(format!("{word} (maybe {})", guesses.join(" or ")));
        if out.len() == MOST {
            break;
        }
    }
    out.join(", ")
}

fn tags(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::En => &["en-US", "en-GB"],
        Lang::Ar => &["ar-SA", "ar"],
        Lang::Fr => &["fr-FR"],
        Lang::De => &["de-DE"],
        Lang::Es => &["es-ES", "es-MX"],
        Lang::Pt => &["pt-BR", "pt-PT"],
        // Written without spaces, or with no dictionary on most machines.
        Lang::Zh | Lang::Ja | Lang::Ko => &[],
    }
}

impl Dictionaries {
    /// The hints for a paragraph in `lang`: `whaere (maybe where), tosdat
    /// (maybe toasted or today)`. Not a word with a capital after the first (it may be
    /// a name the dictionary lacks), a number, or a protected marker.
    pub fn hints(&mut self, lang: Option<Lang>, text: &str) -> String {
        let Some(lang) = lang else { return String::new() };
        let tags = tags(lang);
        if tags.is_empty() {
            return String::new();
        }
        let unknown = self.unknown(tags, text);
        hint_line(text, unknown)
    }

    /// `text` with each word the dictionary lacks put for its first guess,
    /// by the same rule as the hints: what a spell checker alone would do.
    /// The eval's baseline (tests/eval.rs, EVAL_SPELL), not the app's.
    #[allow(dead_code)]
    pub fn corrected(&mut self, lang: Option<Lang>, text: &str) -> String {
        let Some(lang) = lang else { return text.to_string() };
        let first = text.split_whitespace().next().unwrap_or("");
        let mut out = text.to_string();
        let mut from = 0;
        for (word, guesses) in self.unknown(tags(lang), text) {
            let named = word.starts_with(char::is_uppercase) && !first.starts_with(word.as_str());
            let Some(guess) = guesses.first().filter(|_| !named && !word.chars().any(|c| c.is_numeric() || c == '⟦' || c == '⟧')) else { continue };
            if let Some(at) = out[from..].find(word.as_str()).map(|i| i + from) {
                out.replace_range(at..at + word.len(), guess);
                from = at + guess.len();
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(words: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        words.iter().map(|(w, g)| (w.to_string(), g.iter().map(|g| g.to_string()).collect())).collect()
    }

    #[test]
    fn hints_name_two_guesses_for_at_most_six_words_and_skip_names_numbers_and_markers() {
        let text = "Teh cat sat whaere Bxob put 3rd \u{27E6}1\u{27E7}x";
        let unknown = said(&[("Teh", &["The", "Ten", "Tea"]), ("whaere", &["where"]), ("Bxob", &["Bob"]), ("3rd", &["3d"]), ("\u{27E6}1\u{27E7}x", &["x"]), ("zqz", &[])]);
        assert_eq!(hint_line(text, unknown), "Teh (maybe The or Ten), whaere (maybe where)", "the first word may be capitalised; a name, a number, a marker or no guess is left out");
        let many: Vec<(String, Vec<String>)> = (0..9).map(|i| (format!("wrod{}", "x".repeat(i)), vec!["word".to_string()])).collect();
        assert_eq!(hint_line("a b c", many).matches(" (maybe ").count(), 6);
        assert_eq!(hint_line("fine", Vec::new()), "");
    }

    #[test]
    fn the_macs_dictionary_is_the_tag_or_its_language_alone() {
        let mac: Vec<String> = ["en", "en_GB", "ar", "fr", "de", "es", "pt_BR", "pt_PT"].iter().map(|s| s.to_string()).collect();
        assert_eq!(mac_language(tags(Lang::En), &mac).as_deref(), Some("en"), "en-US is the Mac's en");
        assert_eq!(mac_language(&["en-GB"], &mac).as_deref(), Some("en_GB"));
        assert_eq!(mac_language(tags(Lang::Pt), &mac).as_deref(), Some("pt_BR"));
        assert_eq!(mac_language(tags(Lang::Ar), &mac).as_deref(), Some("ar"));
        assert_eq!(mac_language(tags(Lang::Ja), &mac), None, "no dictionary asked for");
        assert_eq!(mac_language(&["fi-FI"], &mac), None, "one the Mac lacks");
    }

    /// The Mac's own checker gives the model its guesses.
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "needs a logged-in GUI session: NSSpellChecker waits on the spelling service, which a bare SSH session has none of"]
    fn the_macs_spell_checker_hints_the_words_it_does_not_know() {
        let hints = Dictionaries::new().hints(Some(Lang::En), "We recieve the mail evry morning, Bxob said.");
        assert!(hints.contains("recieve (maybe receive"), "{hints}");
        assert!(hints.contains("evry (maybe "), "{hints}");
        assert!(!hints.contains("Bxob"), "a name is left alone: {hints}");
    }
}
