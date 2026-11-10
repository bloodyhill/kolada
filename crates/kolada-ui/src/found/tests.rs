use super::*;
use crate::scene::{self, Eng};
use kolada_text::{Format, Role};

fn words<'a>(text: &'a str, r: &[(usize, usize)]) -> Vec<&'a str> {
    r.iter().map(|&(a, b)| &text[a..b]).collect()
}

#[test]
fn every_match_is_found_as_search_finds_it_case_and_accents_folded() {
    let text = "Plan the launch. The PLAN holds; a plán with an accent, and planning too.";
    assert_eq!(words(text, &ranges(text, "plan")), ["Plan", "PLAN", "plán", "plan"], "every one, as written");
    assert!(ranges(text, "  ").is_empty() && ranges(text, "").is_empty(), "nothing typed finds nothing");
    // A query that is nowhere whole: each of its words, as search finds a
    // note by all its words.
    let r = ranges(text, "launch accent");
    assert_eq!(words(text, &r), ["launch", "accent"]);
    // Words that meet are one tint.
    assert_eq!(words("planplan", &ranges("planplan", "plan an")), ["planplan"]);
}

#[test]
fn arabic_is_found_however_it_was_typed_vowel_marks_tatweel_and_letter_forms_too() {
    // Made-up words: a book with its vowel marks, one drawn out with a
    // tatweel, a name with its hamza, and the same word in an English line.
    let text = "\u{643}\u{650}\u{62A}\u{64E}\u{627}\u{628}\u{64C} \u{648} \u{643}\u{640}\u{62A}\u{627}\u{628} \u{648} \u{623}\u{62D}\u{645}\u{62F} \u{2014} the \u{643}\u{62A}\u{627}\u{628} is on the table";
    let book = "\u{643}\u{62A}\u{627}\u{628}";
    let found = ranges(text, book);
    assert_eq!(found.len(), 3, "with its marks, drawn out, and plain");
    // The marks after the last letter are part of the word: the tint covers
    // the whole cluster, never half of it.
    assert_eq!(&text[found[0].0..found[0].1], "\u{643}\u{650}\u{62A}\u{64E}\u{627}\u{628}\u{64C}");
    assert_eq!(&text[found[1].0..found[1].1], "\u{643}\u{640}\u{62A}\u{627}\u{628}");
    // Typed with its marks, it finds the plain word too.
    assert_eq!(ranges(text, "\u{643}\u{650}\u{62A}\u{64E}\u{627}\u{628}").len(), 3);
    // Alif with a hamza is the alif it is typed as.
    assert_eq!(words(text, &ranges(text, "\u{627}\u{62D}\u{645}\u{62F}")), ["\u{623}\u{62D}\u{645}\u{62F}"]);
    // Mixed: an English word and an Arabic one, each found.
    assert_eq!(words(text, &ranges(text, "table")), ["table"]);
    assert_eq!(ranges(text, &format!("the {book}")).len(), 1, "the whole query, across scripts");
}

fn lines(text: &str) -> Vec<kolada_markdown::Line> {
    kolada_markdown::lines(text).collect()
}

#[test]
fn a_note_opened_from_search_shows_its_matches_until_its_text_changes() {
    let text = "# Garden\n\nRaised beds by the fence.\n\nThe fence needs paint.\n";
    let f = Found::opened(text, &lines(text), "fence", 7).expect("found");
    assert_eq!(f.query(), "fence");
    assert_eq!(words(text, f.ranges(7)), ["fence", "fence"], "every match");
    let first = text.find("fence").unwrap();
    assert_eq!(f.caret(), Some(first), "the caret at the first match");
    assert!(f.holds(7) && !f.holds(8));
    assert!(f.ranges(8).is_empty(), "an edit (a new version): no tint, and nothing to find again");
    let mut f = f;
    assert_eq!(f.take_center(), Some(first), "the view goes to it once");
    assert_eq!(f.take_center(), None, "and is the user's after that");
    assert!(Found::opened(text, &lines(text), "hedge", 7).is_none(), "not in the text: nothing to show");
}

#[test]
fn a_match_only_in_the_title_tints_the_title_and_the_caret_stays_under_it() {
    let text = "# Plum jam\n\nSugar, lemons, a slow afternoon.\n";
    let f = Found::opened(text, &lines(text), "plum", 1).expect("the title has it");
    assert_eq!(words(text, f.ranges(1)), ["Plum"]);
    assert_eq!(f.caret(), None, "typing at once adds to the note, not to its name");
    // A title with no heading mark, as most notes have.
    let plain = "Plum jam\nSugar, lemons.\n";
    let f = Found::opened(plain, &lines(plain), "jam", 1).unwrap();
    assert_eq!((words(plain, f.ranges(1)), f.caret()), (vec!["jam"], None));
}

fn eng() -> Eng {
    Eng::new(kolada_text::Engine::new(crate::testing::fonts()))
}

/// The tints a layout of `text` gets for `query`, and where its letters are.
fn tinted(text: &str, query: &str, width: f32) -> (Vec<Rect>, Vec<Vec<[f32; 4]>>) {
    let eng = eng();
    let l = scene::layout(&eng, text, &Format::new(Role::Ui, 13.0), width, 200.0);
    let mut s = Scene::default();
    tint(&mut s, &l, 10.0, 20.0, query, (0.0, 20.0 + width));
    assert!(s.fills.iter().all(|f| f.ink == Ink::Mark && f.blur == 0.0 && !f.ring), "the marks' teal, filled");
    let at = ranges(text, query).iter().map(|&(a, b)| l.range(a, b)).collect();
    (s.fills.iter().map(|f| f.rect).collect(), at)
}

#[test]
fn the_tint_sits_on_the_words_it_found_in_latin_and_arabic() {
    for (text, query) in [
        ("Leave Friday at nine; the ferry leaves at ten.", "leave"),
        ("\u{645}\u{648}\u{639}\u{62F} \u{627}\u{644}\u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639} \u{63A}\u{62F}\u{627}\u{64B} \u{645}\u{639} Kolada team", "\u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639}"),
        ("\u{645}\u{648}\u{639}\u{62F} \u{627}\u{644}\u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639} \u{645}\u{639} Kolada team", "kolada"),
    ] {
        let (fills, letters) = tinted(text, query, 400.0);
        let pieces: Vec<[f32; 4]> = letters.into_iter().flatten().collect();
        assert!(!pieces.is_empty(), "{query:?} is laid out");
        assert_eq!(fills.len(), pieces.len(), "a tint per piece of a match, {query:?}");
        for (f, [x, y, w, h]) in fills.iter().zip(pieces) {
            // A pixel past each side of the letters, a pixel inside the line.
            assert!((f.left - (10.0 + x - 1.0)).abs() < 0.01 && (f.right - (10.0 + x + w + 1.0)).abs() < 0.01, "{query:?}: {f:?} across the letters at {x}..{}", x + w);
            assert!((f.top - (20.0 + y + 1.0)).abs() < 0.01 && (f.bottom - (20.0 + y + h - 1.0)).abs() < 0.01, "{query:?}: {f:?} on their line");
        }
    }
}

#[test]
fn no_tint_past_what_shows() {
    let eng = eng();
    let text = "A line far too long for its place, with the word lemon at its very end";
    let f = Format { wrap: false, ellipsis: true, ..Format::new(Role::Ui, 13.0) };
    let l = scene::layout(&eng, text, &f, 120.0, 20.0);
    let mut s = Scene::default();
    tint(&mut s, &l, 0.0, 0.0, "lemon", (0.0, 120.0));
    assert!(s.fills.is_empty(), "the word is cut away: nothing tinted where it is not");
    tint(&mut s, &l, 0.0, 0.0, "line", (0.0, 120.0));
    assert_eq!(s.fills.len(), 1);
    assert!(s.fills.iter().all(|f| f.rect.left >= 0.0 && f.rect.right <= 120.0));
}

#[test]
fn a_one_line_snippet_brings_its_match_near_the_start() {
    let s = "Oat milk, eggs, a loaf from the corner bakery and the lemons for Sunday";
    let led = lead(s, "lemons", 12);
    assert!(led.starts_with('\u{2026}') && led.contains("lemons"), "{led:?}");
    let at = led.find("lemons").unwrap();
    assert!(led[..at].chars().count() <= 14, "a few words before it at most: {led:?}");
    assert!(!led[3..at].starts_with(' '), "from a word's start: {led:?}");
    assert_eq!(lead(s, "milk", 12), s, "near the start already: whole");
    assert_eq!(lead(s, "hedge", 12), s, "no match: as it was");
    assert_eq!(lead(s, "lemons", 0).find("lemons"), Some(3), "nothing kept before it");
}

#[test]
fn a_one_letter_word_is_tinted_only_where_it_stands_alone() {
    // "plan a" is nowhere whole: each word, and the "a" only as a word.
    let text = "# Garden\n\nA plan for a banana and a pear.\n";
    let f = Found::opened(text, &lines(text), "plan a", 3).expect("found");
    assert_eq!(words(text, f.ranges(3)), ["A", "plan", "a", "a"], "not the a's inside banana, and or pear");
    // Just "a": the same, never every a in the note.
    assert_eq!(words(text, &ranges(text, "a")), ["A", "a", "a"]);
    assert!(ranges("banana", "a").is_empty() && Found::opened("banana\n", &lines("banana\n"), "a", 1).is_none(), "nothing to tint");
    // Two letters and more: found inside words, as search finds them.
    assert_eq!(words(text, &ranges(text, "an")), ["an", "an", "an", "an"], "plan, banana twice, and");
    // Arabic's one-letter "and", alone, not inside a word that has it.
    let ar = "\u{643}\u{62A}\u{627}\u{628} \u{648} \u{645}\u{648}\u{639}\u{62F}";
    assert_eq!(words(ar, &ranges(ar, "\u{648}")), ["\u{648}"]);
    // A whole query that ends (or starts) with one: "plan a" is not in
    // "plan already".
    assert_eq!(words("the plan already", &ranges("the plan already", "plan a")), ["plan"], "its words then, the a nowhere alone");
    assert_eq!(words("a plan a day", &ranges("a plan a day", "plan a")), ["plan a"]);
    // Scripts without spaces between words: one character is found where
    // it is, and the note opens on it.
    assert_eq!(words("我爱中国人", &ranges("我爱中国人", "中")), ["中"]);
    assert_eq!(words("東京に行く", &ranges("東京に行く", "京")), ["京"]);
    assert!(Found::opened("東京に行く\n", &lines("東京に行く\n"), "京", 1).is_some());
}

#[test]
fn a_one_line_snippet_is_cut_at_a_word_with_an_ellipsis_at_its_end_either_way() {
    let eng = eng();
    let format = Format { wrap: false, ellipsis: true, ..Format::new(Role::Ui, 13.0) };
    let latin = "Oat milk, eggs, a loaf from the corner bakery and two lemons, then a slow afternoon of baking";
    let arabic = "\u{645}\u{648}\u{639}\u{62F} \u{627}\u{644}\u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639} \u{63A}\u{62F}\u{627}\u{64B} \u{645}\u{639} \u{627}\u{644}\u{641}\u{631}\u{64A}\u{642} \u{641}\u{64A} \u{627}\u{644}\u{645}\u{643}\u{62A}\u{628} \u{627}\u{644}\u{62C}\u{62F}\u{64A}\u{62F} \u{642}\u{628}\u{644} \u{627}\u{644}\u{638}\u{647}\u{631}";
    for (text, rtl) in [(latin, false), (arabic, true)] {
        for width in [90.0, 140.0, 200.0, 260.0] {
            let l = snippet_line(&eng, text, &format, width, 20.0);
            let shown = l.cut_text();
            assert!(shown.ends_with('\u{2026}'), "rtl {rtl} at {width}: cut, with an ellipsis");
            let kept = shown.trim_end_matches(['\u{2026}', '\u{200E}', '\u{200F}']);
            assert!(text.starts_with(kept) && text[kept.len()..].starts_with([' ', ',']), "rtl {rtl} at {width}: whole words ({} of {} bytes)", kept.len(), text.len());
            assert!(l.measure().width <= width + 0.5, "rtl {rtl} at {width}: it fits");
            // The ellipsis at the end the line reads to: the right, or the
            // left for Arabic.
            let dots = shown.len() - '\u{2026}'.len_utf8();
            let at = l.range(dots, shown.len());
            let mid = at.first().map(|[x, _, w, _]| x + w / 2.0).expect("the ellipsis is drawn");
            assert!(if rtl { mid < l.measure().width / 2.0 } else { mid > l.measure().width / 2.0 }, "rtl {rtl} at {width}: the ellipsis at {mid}");
        }
        // Room for it all: as it is.
        assert_eq!(snippet_line(&eng, text, &format, 2000.0, 20.0).cut_text(), text);
    }
    assert_eq!(cut_at_word("Sunday lunch with everyone", "Sunday lunc\u{200E}\u{2026}").as_deref(), Some("Sunday\u{200E}\u{2026}"));
    assert_eq!(cut_at_word("Sunday lunch with", "Sunday lunch\u{200E}\u{2026}"), None, "at a word's end already");
}

#[test]
fn a_snippet_led_to_its_match_starts_at_a_word_in_either_direction() {
    // Arabic, its words with their vowel marks: the cut is at a word's
    // start, never between a letter and its mark.
    let s = "\u{643}\u{650}\u{62A}\u{64E}\u{627}\u{628}\u{64C} \u{645}\u{64E}\u{648}\u{652}\u{639}\u{650}\u{62F} \u{62C}\u{64E}\u{62F}\u{650}\u{64A}\u{62F} \u{643}\u{650}\u{62A}\u{64E}\u{627}\u{628}\u{64C} \u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639}";
    let led = lead(s, "\u{627}\u{62C}\u{62A}\u{645}\u{627}\u{639}", 10);
    assert!(led.starts_with('\u{2026}'));
    let rest = &led['\u{2026}'.len_utf8()..];
    assert!(s.ends_with(rest) && s[..s.len() - rest.len()].ends_with(' '), "from a word's start");
    // A word too long to keep whole ahead of the match: cut between letters.
    let long = format!("{}lemons", "e\u{301}".repeat(60));
    let led = lead(&long, "lemons", 12);
    assert!(led.starts_with('\u{2026}') && led.chars().nth(1) == Some('e') && led.ends_with("lemons"), "{} chars", led.chars().count());
}
