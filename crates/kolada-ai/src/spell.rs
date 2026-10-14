//! The dictionary's guesses for the words it does not know, handed to the
//! model with the paragraph: `whaere → where`. The model still decides from
//! the sentence what was meant; the guard still checks what it changed.
//! Windows' own spell checkers, the ones every app shares.

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

#[cfg(not(windows))]
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
        let first = text.split_whitespace().next().unwrap_or("");
        let mut out: Vec<String> = Vec::new();
        for (word, guesses) in self.unknown(tags, text) {
            let named = word.starts_with(char::is_uppercase) && !first.starts_with(word.as_str());
            if guesses.is_empty() || named || word.chars().any(|c| c.is_numeric() || c == '⟦' || c == '⟧') {
                continue;
            }
            // In words, not marks: the model copies an arrow into its answer.
            out.push(format!("{word} (maybe {})", guesses.join(" or ")));
            if out.len() == MOST {
                break;
            }
        }
        out.join(", ")
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
