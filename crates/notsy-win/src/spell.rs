//! Spelling, checked by Windows: the same dictionaries every app uses, the
//! words added to them, English and Arabic side by side. A worker checks a
//! line at a time and marks come back keyed by the line's text, so lines
//! that did not change keep their marks while others are being typed.
//! What is not prose is left alone: code, links, addresses, tags, marks.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};

use notsy_markdown::{self as md, Line};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Globalization::{GetUserDefaultLocaleName, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{HSTRING, PWSTR};

use crate::window::WM_APP_SPELL;

pub use notsy_ui::spell::{Marks, line_key, prose};

/// Marks kept at most before those of lines no longer in the note go.
const KEEP: usize = 4000;

fn arabic(u: u16) -> bool {
    matches!(u, 0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)
}

/// The dictionaries this machine has: one for Latin-script words (the
/// user's language if it is one, else English) and one for Arabic.
struct Checkers {
    latin: Option<ISpellChecker>,
    arabic: Option<ISpellChecker>,
}

impl Checkers {
    fn new() -> Checkers {
        let pick = |factory: &ISpellCheckerFactory, tags: &[String]| {
            tags.iter().find_map(|t| unsafe {
                let tag = HSTRING::from(t.as_str());
                (factory.IsSupported(&tag).ok()?.as_bool()).then(|| factory.CreateSpellChecker(&tag).ok()).flatten()
            })
        };
        let Ok(factory) = (unsafe { CoCreateInstance::<_, ISpellCheckerFactory>(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER) }) else {
            return Checkers { latin: None, arabic: None };
        };
        let mut user = [0u16; 85];
        let n = unsafe { GetUserDefaultLocaleName(&mut user) } as usize;
        let user = String::from_utf16_lossy(&user[..n.saturating_sub(1)]);
        let mut latin_tags = vec![];
        if !user.starts_with("ar") && !user.is_empty() {
            latin_tags.push(user.clone());
        }
        latin_tags.extend(["en-US".to_string(), "en-GB".to_string()]);
        let mut arabic_tags = vec![];
        if user.starts_with("ar") {
            arabic_tags.push(user);
        }
        arabic_tags.extend(["ar-SA".to_string(), "ar".to_string()]);
        let latin = pick(&factory, &latin_tags);
        // For this run only: Ignore, unlike Add, leaves the user's dictionary alone.
        for word in notsy_ui::spell::OWN_WORDS {
            if let Some(c) = &latin {
                let _ = unsafe { c.Ignore(&HSTRING::from(*word)) };
            }
        }
        Checkers { latin, arabic: pick(&factory, &arabic_tags) }
    }

    /// Misspelled words of a line's prose (see `prose`), as byte ranges.
    fn check(&self, units: &[u16], offsets: &[u32]) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for (checker, keep_arabic) in [(&self.latin, false), (&self.arabic, true)] {
            let Some(checker) = checker else { continue };
            // Each dictionary sees only its own script.
            let mut own: Vec<u16> = units.iter().map(|&u| if arabic(u) == keep_arabic { u } else { b' ' as u16 }).collect();
            if !own.iter().any(|&u| u != b' ' as u16) {
                continue;
            }
            own.push(0);
            let Ok(errors) = (unsafe { checker.Check(windows::core::PCWSTR(own.as_ptr())) }) else { continue };
            loop {
                let mut e = None;
                if unsafe { errors.Next(&mut e) }.0 != 0 {
                    break;
                }
                let Some(e) = e else { break };
                let (Ok(start), Ok(len)) = (unsafe { e.StartIndex() }, unsafe { e.Length() }) else { continue };
                let (a, b) = (start as usize, (start + len) as usize);
                if b <= units.len() && a < b {
                    out.push((offsets[a], offsets[b]));
                }
            }
        }
        out.sort_unstable();
        out
    }

    fn for_word(&self, word: &str) -> Option<&ISpellChecker> {
        let is_arabic = word.encode_utf16().any(arabic);
        if is_arabic { self.arabic.as_ref() } else { self.latin.as_ref() }
    }
}

/// Work for the checker's thread.
enum Job {
    Check(Vec<(u64, Vec<u16>, Vec<u32>)>),
    Ignore(String),
}

pub struct Spell {
    jobs: Sender<Job>,
    pub marks: Marks,
    pending: HashSet<u64>,
    /// The text versions (the page's, the popup's) and the scroll position
    /// last looked through for lines to check.
    pub seen: (u64, u64, u64),
    /// The UI thread's own checkers, for suggestions (made when first
    /// asked).
    here: Option<Checkers>,
}

impl Spell {
    /// Starts the checker's thread; marks are posted to `hwnd`.
    pub fn start(hwnd: HWND) -> Spell {
        let (jobs, rx) = channel();
        let hwnd = crate::Handoff(hwnd);
        std::thread::spawn(move || {
            let hwnd = hwnd;
            crate::background_priority();
            work(hwnd.0, rx);
        });
        Spell { jobs, marks: HashMap::new(), pending: HashSet::new(), seen: (u64::MAX, u64::MAX, u64::MAX), here: None }
    }

    /// Sends the prose lines of `text` not checked yet.
    pub fn look_through(&mut self, text: &str, lines: &[Line]) {
        let mut batch = Vec::new();
        for l in lines {
            let key = line_key(&text[l.start..l.end]);
            if self.marks.contains_key(&key) || self.pending.contains(&key) {
                continue;
            }
            match prose(text, l) {
                Some((units, offsets)) => {
                    self.pending.insert(key);
                    batch.push((key, units, offsets));
                }
                None => {
                    self.marks.insert(key, Vec::new());
                }
            }
        }
        if !batch.is_empty() {
            let _ = self.jobs.send(Job::Check(batch));
        }
        if self.marks.len() > KEEP {
            let keys: HashSet<u64> = lines.iter().map(|l| line_key(&text[l.start..l.end])).collect();
            self.marks.retain(|k, _| keys.contains(k));
        }
    }

    /// Marks back from the worker; true if any shows: a line checked clean
    /// looks as it did while it was being checked, and the word being typed
    /// (in line `typing.0`, at byte `typing.1` of it) is not marked.
    pub fn arrived(&mut self, found: Vec<(u64, Vec<(u32, u32)>)>, typing: Option<(u64, u32)>) -> bool {
        let mut shows = false;
        for (key, marks) in found {
            self.pending.remove(&key);
            shows |= marks.iter().any(|&(a, b)| typing.is_none_or(|(line, at)| line != key || at < a || at > b));
            self.marks.insert(key, marks);
        }
        shows
    }

    /// The misspelled word at byte `at` of `text` (in `line`): its range
    /// and the word.
    pub fn word_at(&self, text: &str, line: &Line, at: usize) -> Option<(usize, usize, String)> {
        let marks = self.marks.get(&line_key(&text[line.start..line.end]))?;
        let &(a, b) = marks.iter().find(|&&(a, b)| line.start + a as usize <= at && at <= line.start + b as usize)?;
        let (a, b) = (line.start + a as usize, line.start + b as usize);
        Some((a, b, text[a..b].to_string()))
    }

    fn here(&mut self) -> &Checkers {
        self.here.get_or_insert_with(|| {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            Checkers::new()
        })
    }

    /// Up to five words the dictionary offers for `word`.
    pub fn suggest(&mut self, word: &str) -> Vec<String> {
        let Some(checker) = self.here().for_word(word) else { return Vec::new() };
        let Ok(list) = (unsafe { checker.Suggest(&HSTRING::from(word)) }) else { return Vec::new() };
        let mut out = Vec::new();
        while out.len() < 5 {
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

    /// Adds `word` to the user's dictionary (every app shares it).
    pub fn add(&mut self, word: &str) {
        if let Some(checker) = self.here().for_word(word) {
            let _ = unsafe { checker.Add(&HSTRING::from(word)) };
        }
        self.recheck();
    }

    /// Leaves `word` unmarked for as long as the app runs.
    pub fn ignore(&mut self, word: &str) {
        if let Some(checker) = self.here().for_word(word) {
            let _ = unsafe { checker.Ignore(&HSTRING::from(word)) };
        }
        let _ = self.jobs.send(Job::Ignore(word.to_string()));
        self.recheck();
    }

    fn recheck(&mut self) {
        self.marks.clear();
        self.pending.clear();
        self.seen = (u64::MAX, u64::MAX, u64::MAX);
    }
}

fn work(hwnd: HWND, jobs: Receiver<Job>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let checkers = Checkers::new();
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Ignore(word) => {
                if let Some(c) = checkers.for_word(&word) {
                    let _ = unsafe { c.Ignore(&HSTRING::from(word.as_str())) };
                }
            }
            Job::Check(lines) => {
                let found: Vec<(u64, Vec<(u32, u32)>)> =
                    lines.into_iter().map(|(key, units, offsets)| (key, checkers.check(&units, &offsets))).collect();
                let found = Box::into_raw(Box::new(found));
                if unsafe { PostMessageW(Some(hwnd), WM_APP_SPELL, WPARAM(0), LPARAM(found as isize)) }.is_err() {
                    drop(unsafe { Box::from_raw(found) });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs the Windows spell checker and its English dictionary"]
    fn marks_misspelled_words() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let c = Checkers::new();
        let text = "Teh plan is realy good in Notsy, see `cdoe`.";
        let (units, offsets) = prose(text, &md::lines(text).next().unwrap()).unwrap();
        let marks = c.check(&units, &offsets);
        let words: Vec<&str> = marks.iter().map(|&(a, b)| &text[a as usize..b as usize]).collect();
        assert_eq!(words, ["Teh", "realy"]);
        assert!(!{
            let mut s = Spell { jobs: channel().0, marks: HashMap::new(), pending: HashSet::new(), seen: (0, 0, 0), here: Some(c) };
            s.suggest("realy")
        }
        .is_empty());
    }
}
