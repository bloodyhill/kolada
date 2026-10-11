//! Spelling, checked by the Mac's own checker (NSSpellChecker): the system's
//! dictionaries and the words the user taught them, with the language found
//! from the text, so English and Arabic sit side by side.

use objc2_app_kit::NSSpellChecker;
use objc2_foundation::{NSNotFound, NSString};

/// The system's checker, for the model's `Spell` service.
pub struct MacSpell;

impl notsy_app::services::Spell for MacSpell {
    fn check(&self, units: &[u16]) -> Vec<(usize, usize)> {
        check(units)
    }
    fn learn(&self, word: &str) {
        learn(word)
    }
    fn suggest(&self, word: &str) -> Vec<String> {
        suggest(word)
    }
}

/// The misspelled words of `units` (a line's prose as UTF-16, with what is
/// not prose blanked: see `notsy_ui::spell::prose`), as UTF-16 ranges.
pub fn check(units: &[u16]) -> Vec<(usize, usize)> {
    let checker = NSSpellChecker::sharedSpellChecker();
    checker.setAutomaticallyIdentifiesLanguages(true);
    let string = NSString::from_str(&String::from_utf16_lossy(units));
    let mut out = Vec::new();
    let mut start = 0isize;
    while (start as usize) < units.len() {
        // Without wrapping: the short form wraps to the line's start after
        // its last misspelling, and this loop then went round for ever
        // (seen on the real screen: any line with a misspelling hung the app).
        let range = unsafe { checker.checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(&string, start, None, false, 0, std::ptr::null_mut()) };
        if range.location == NSNotFound as usize || range.length == 0 || (range.location as isize) < start {
            break;
        }
        out.push((range.location, range.location + range.length));
        start = (range.location + range.length) as isize;
    }
    out
}

/// Teaches the system's dictionary `word` (Learn Spelling): no app marks it again.
pub fn learn(word: &str) {
    NSSpellChecker::sharedSpellChecker().learnWord(&NSString::from_str(word));
}

/// Up to five words the dictionary offers for `word`.
pub fn suggest(word: &str) -> Vec<String> {
    let checker = NSSpellChecker::sharedSpellChecker();
    let string = NSString::from_str(word);
    let range = objc2_foundation::NSRange::new(0, string.length());
    checker
        .guessesForWordRange_inString_language_inSpellDocumentWithTag(range, &string, None, 0)
        .map(|list| list.iter().take(5).map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs a logged-in GUI session: NSSpellChecker waits on the spelling service, which a bare SSH session has none of"]
    fn a_line_with_misspellings_is_checked_once_through_not_round_and_round() {
        // Two misspellings, the line ending after the last: the checker must
        // not wrap back to the first. Run on a thread with a deadline, so a
        // hang fails the test instead of stopping it.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let units: Vec<u16> = "I recieve realy good notes".encode_utf16().collect();
            let _ = tx.send(check(&units).into_iter().map(|(a, b)| String::from_utf16_lossy(&units[a..b])).collect::<Vec<_>>());
        });
        let words = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the check returns: it does not go round for ever");
        assert_eq!(words, ["recieve", "realy"]);
    }

    #[test]
    #[ignore = "needs a logged-in GUI session: NSSpellChecker waits on the spelling service, which a bare SSH session has none of"]
    fn the_system_checker_marks_misspelled_words_and_offers_fixes() {
        let text = "Teh plan is realy good";
        let units: Vec<u16> = text.encode_utf16().collect();
        let marks = check(&units);
        let words: Vec<String> = marks.iter().map(|&(a, b)| String::from_utf16_lossy(&units[a..b])).collect();
        assert!(words.contains(&"realy".to_string()), "{words:?}");
        assert!(suggest("realy").iter().any(|s| s == "really"));
    }
}
