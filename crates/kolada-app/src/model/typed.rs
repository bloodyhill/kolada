//! The system's text replacements and spelling corrections as one types,
//! as the Mac's own text views make them, where the system says to (System
//! Settings › Keyboard: Text Replacements, Correct spelling automatically).
//! When a word ends (a space, a stop, Return), the word just typed becomes
//! its replacement at once (a lookup in the user's list, read after the
//! first frame and whenever the app comes to the front), or is handed to
//! the system's checker on a thread of its own, off the keystroke; its
//! correction comes back through the arrivals and goes in only if the word
//! is still as it was typed. Each is one Cmd+Z step of its own, washed in
//! the accent for a moment as the writing model's fixes are; Esc right
//! after takes it back, and a word taken back is not corrected again in
//! that note. Only prose: never code, links, tags or math. Kolada's own
//! underline and the writing model go on as they were.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use kolada_ui::chrome;

use super::arrive::Arrival;
use super::*;

/// How long a replacement or a correction shows its wash.
const SHOWN: Duration = Duration::from_millis(900);
const SHOWN_ALPHA: f32 = 0.28;

/// What typing does with words as they end.
#[derive(Default)]
pub(super) struct Typed {
    /// The user's text replacements: what is typed, what it becomes.
    replacements: HashMap<String, String>,
    /// Words taken back with Esc in this note: never corrected again.
    rejected: HashSet<String>,
    /// The last replacement or correction, while Esc may take it back.
    fix: Option<Fix>,
    /// The checker's thread, once a word was asked of it.
    ask: Option<Sender<Ask>>,
    /// Which note is open (answers for another are dropped).
    note: u64,
}

struct Fix {
    range: [(usize, usize); 1],
    new: String,
    old: String,
    since: Instant,
}

/// A word for the checker: where it starts, and in which note.
struct Ask {
    at: usize,
    word: String,
    note: u64,
}

/// Whether typing `s` ends the word before it.
fn ends_a_word(s: &str) -> bool {
    let mut chars = s.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if c.is_whitespace() || ".,;:!?)]}\"\u{201d}\u{2019}".contains(c))
}

/// Whether `c` belongs to a word the checker is asked about.
fn in_word(c: char) -> bool {
    c.is_alphabetic() || c == '\'' || c == '\u{2019}'
}

impl Model {
    /// The user's text replacements, as the system has them now (none when
    /// its setting is off): after the first frame, and when the app comes
    /// back to the front.
    pub fn read_replacements(&mut self) {
        self.typed.replacements = self.services.spell.replacements().into_iter().filter(|(from, to)| !from.is_empty() && from != to).collect();
    }

    /// Typing `s` with the selection at `before`, as the editor took it:
    /// if `s` ended a word, the word's replacement or correction.
    pub(super) fn typed_char(&mut self, s: &str, before: Selection) {
        if before.is_caret() && ends_a_word(s) {
            self.word_ended(before.head);
        }
    }

    /// The word before `end` was just ended: replaced from the user's list
    /// at once, or asked of the system's checker.
    fn word_ended(&mut self, end: usize) {
        // Esc takes back only the last fix, before the next word.
        self.typed.fix = None;
        if self.popup.is_some() || self.scrubbing() || self.settings.read_only {
            return;
        }
        let text = self.editor.text();
        if end > text.len() || !text.is_char_boundary(end) {
            return;
        }
        // A replacement's shortcut is what was typed since the last space.
        let token_at = text[..end].char_indices().rev().find(|(_, c)| c.is_whitespace()).map_or(0, |(i, c)| i + c.len_utf8());
        if let Some(to) = self.typed.replacements.get(&text[token_at..end]).cloned()
            && self.in_prose(token_at, end)
        {
            let old = text[token_at..end].to_string();
            self.amend_typed(token_at, end, to, old);
            return;
        }
        let at = text[..end].char_indices().rev().take_while(|&(_, c)| in_word(c)).last().map_or(end, |(i, _)| i);
        let word = &text[at..end];
        if word.chars().count() < 2 || !word.chars().any(char::is_alphabetic) || self.typed.rejected.contains(&word.to_lowercase()) {
            return;
        }
        // Part of a longer run (a number, a name with digits): not a word.
        if text[end..].chars().next().is_some_and(char::is_alphanumeric) || text[..at].chars().next_back().is_some_and(char::is_alphanumeric) {
            return;
        }
        let ask = Ask { at, word: word.to_string(), note: self.typed.note };
        let sender = match &self.typed.ask {
            Some(s) => s.clone(),
            None => {
                let (tx, rx) = channel::<Ask>();
                let (spell, send) = (self.services.spell.clone(), self.arrivals.sender());
                std::thread::spawn(move || {
                    for ask in rx {
                        if let Some(to) = spell.correct(&ask.word).filter(|to| *to != ask.word && !to.is_empty()) {
                            send(Arrival::Corrected { at: ask.at, word: ask.word, to, note: ask.note });
                        }
                    }
                });
                self.typed.ask = Some(tx.clone());
                tx
            }
        };
        let _ = sender.send(ask);
    }

    /// The checker's correction for the word at `at`: in, if that word is
    /// still there as typed, in prose, not being typed into, in this note.
    pub(super) fn corrected(&mut self, at: usize, word: String, to: String, note: u64) -> bool {
        if note != self.typed.note || self.typed.rejected.contains(&word.to_lowercase()) || self.popup.is_some() || self.settings.read_only || self.scrubbing() {
            return false;
        }
        let end = at + word.len();
        let text = self.editor.text();
        if text.get(at..end) != Some(word.as_str()) || text[end..].chars().next().is_none_or(in_word) || text[..at].chars().next_back().is_some_and(in_word) {
            return false;
        }
        let s = self.editor.selection();
        if (at..=end).contains(&s.head) || (at..=end).contains(&s.anchor) || !self.in_prose(at, end) {
            return false;
        }
        self.amend_typed(at, end, to, word)
    }

    /// `old` at `start..end` becomes `new`, one Cmd+Z step of its own, and
    /// the last fix Esc may take back. The view stays where it is.
    fn amend_typed(&mut self, start: usize, end: usize, new: String, old: String) -> bool {
        let top = self.scroll_line;
        if end <= top {
            self.scroll_line = (top as i64 + new.len() as i64 - (end - start) as i64).max(0) as usize;
        }
        let (reveal, before) = (self.reveal, self.editor.version());
        let ranges = self.editor.amend(&[(start, end, new.clone())]);
        self.dirty |= self.editor.version() != before;
        self.reveal = reveal;
        let Some(&range) = ranges.first() else {
            self.scroll_line = top;
            return false;
        };
        self.fix_scroll_line();
        self.typed.fix = Some(Fix { range: [range], new, old, since: Instant::now() });
        true
    }

    /// Esc right after a replacement or a correction: the word as typed
    /// again, and never corrected again in this note.
    pub(super) fn revert_typed(&mut self) -> bool {
        let Some(fix) = self.typed.fix.take() else { return false };
        let (a, b) = fix.range[0];
        if self.editor.text().get(a..b) != Some(fix.new.as_str()) {
            return false;
        }
        self.typed.rejected.insert(fix.old.to_lowercase());
        let changed = self.amend_typed(a, b, fix.old, fix.new);
        self.typed.fix = None;
        changed
    }

    /// Another note is open: what was taken back, and what was asked, are
    /// the last one's.
    pub(super) fn typed_opened(&mut self) {
        self.typed.note += 1;
        self.typed.rejected.clear();
        self.typed.fix = None;
    }

    /// The last fix, washed in the accent while it fades.
    pub(super) fn typed_view(&self) -> Option<(&[(usize, usize)], f32)> {
        let fix = self.typed.fix.as_ref()?;
        let t = fix.since.elapsed().as_secs_f32() / SHOWN.as_secs_f32();
        let (a, b) = fix.range[0];
        if t >= 1.0 || self.editor.text().get(a..b) != Some(fix.new.as_str()) {
            return None;
        }
        let fade = if self.ui.animate { 1.0 - chrome::ease(t) } else { 1.0 };
        Some((&fix.range[..], SHOWN_ALPHA * fade))
    }

    pub(super) fn typed_moving(&self) -> bool {
        self.typed.fix.as_ref().is_some_and(|f| f.since.elapsed() < SHOWN + Duration::from_millis(20))
    }

    /// For a picture of the mark (`--dump --open corrected`): a line typed
    /// at the open note's end, its "teh" just corrected as it ended.
    pub fn demo_correction(&mut self) {
        let end = self.editor.text().len();
        self.editor.set_selection(Selection::caret(end));
        self.insert("\n\nThe cat sat on teh");
        let at = self.editor.text().len() - "teh".len();
        self.insert(" ");
        self.corrected(at, "teh".into(), "the".into(), self.typed.note);
        self.insert("mat");
    }

    /// Whether `a..b` of the note is prose (not code, a link, a tag, math).
    fn in_prose(&self, a: usize, b: usize) -> bool {
        let text = self.editor.text();
        let lines = self.editor.outline();
        let i = lines.partition_point(|l| l.end < a);
        let Some(line) = lines.get(i).filter(|l| l.start <= a && b <= l.end) else { return false };
        let Some((units, offsets)) = kolada_ui::spell::prose(text, line) else { return false };
        units.iter().zip(&offsets).all(|(&u, &off)| {
            let at = line.start + off as usize;
            !(a..b).contains(&at) || u != b' ' as u16 || text[at..].starts_with(' ')
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{Services, Spell};
    use std::sync::Arc;

    /// The system's lists, as a test gives them: one text replacement, and
    /// "teh" corrected to "the".
    struct Lists;

    impl Spell for Lists {
        fn check(&self, _: &[u16]) -> Vec<(usize, usize)> {
            Vec::new()
        }
        fn learn(&self, _: &str) {}
        fn suggest(&self, _: &str) -> Vec<String> {
            Vec::new()
        }
        fn replacements(&self) -> Vec<(String, String)> {
            vec![("omw".into(), "On my way!".into())]
        }
        fn correct(&self, word: &str) -> Option<String> {
            match word {
                "teh" => Some("the".into()),
                "Teh" => Some("The".into()),
                _ => None,
            }
        }
    }

    fn model(test: &str, note: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-app-typed-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Note.md"), note).unwrap();
        let mut m = Model::with_services(Services { spell: Arc::new(Lists), ..Services::none() }, dir.clone(), true);
        m.open("Note.md");
        m.frame(1000, 700, 1.0);
        m.start_arrivals(|| {});
        m.read_replacements();
        m.editor.set_selection(Selection::caret(m.text().len()));
        (m, dir)
    }

    fn typing(m: &mut Model, s: &str) {
        for c in s.chars() {
            m.insert(&c.to_string());
        }
    }

    /// Waits a moment for the checker's thread, then takes what it sent.
    fn settle(m: &mut Model) {
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            m.poll_arrivals();
        }
    }

    /// A text replacement goes in as its word ends, one Cmd+Z step of its
    /// own, washed for a moment; Esc takes it back. Not in code.
    #[test]
    fn a_text_replacement_goes_in_as_its_word_ends() {
        let (mut m, dir) = model("replace", "# Note\n\n");
        typing(&mut m, "omw ");
        assert_eq!(m.text(), "# Note\n\nOn my way! ");
        assert!(m.typed_view().is_some(), "washed in the accent");
        assert!(m.undo());
        assert_eq!(m.text(), "# Note\n\nomw ", "Cmd+Z: what was typed, the space kept");
        typing(&mut m, "omw.");
        assert_eq!(m.text(), "# Note\n\nomw On my way!.");
        assert!(m.command("cancelOperation:"));
        assert_eq!(m.text(), "# Note\n\nomw omw.", "Esc takes it back");
        // In code, a link or a tag: left as typed (the word alone would match).
        let (mut m, dir2) = model("replace-code", "# Note\n\n`x omw` and [[x omw]]\n");
        for at in ["`x omw".len(), "`x omw` and [[x omw".len()] {
            let at = "# Note\n\n".len() + at + if at > 6 { 1 } else { 0 };
            m.editor.set_selection(Selection::caret(at));
            typing(&mut m, " ");
        }
        assert_eq!(m.text(), "# Note\n\n`x omw ` and [[x omw ]]\n");
        let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(dir2));
    }

    /// A misspelled word is corrected off the keystroke when it ends, if it
    /// is still as typed; Esc takes it back, and it is not corrected again
    /// in this note; Cmd+Z takes a correction back as one step.
    #[test]
    fn a_misspelled_word_is_corrected_when_it_ends_and_esc_takes_it_back() {
        let (mut m, dir) = model("correct", "# Note\n\n");
        typing(&mut m, "Teh cat sat on teh ");
        assert_eq!(m.text(), "# Note\n\nTeh cat sat on teh ", "not on the keystroke");
        settle(&mut m);
        assert_eq!(m.text(), "# Note\n\nThe cat sat on the ");
        assert!(m.typed_view().is_some());
        assert!(m.undo());
        assert_eq!(m.text(), "# Note\n\nThe cat sat on teh ", "Cmd+Z: the last correction, as one step");
        // Esc right after: as typed, and not corrected again here.
        typing(&mut m, "mat teh ");
        settle(&mut m);
        assert!(m.text().ends_with("mat the "));
        assert!(m.command("cancelOperation:"));
        assert!(m.text().ends_with("mat teh "), "Esc takes it back");
        typing(&mut m, "and teh ");
        settle(&mut m);
        assert!(m.text().ends_with("and teh "), "a word taken back stays as typed in this note");
        // A word typed on into before the answer came: left alone.
        let (mut m, dir2) = model("correct-moved", "# Note\n\n");
        typing(&mut m, "teh ");
        m.backspace(false);
        typing(&mut m, "e");
        settle(&mut m);
        assert_eq!(m.text(), "# Note\n\ntehe", "the word changed: no correction");
        // In code: never (the word alone would be corrected).
        let (mut m, dir3) = model("correct-code", "# Note\n\n`x teh` y\n");
        m.editor.set_selection(Selection::caret("# Note\n\n`x teh".len()));
        typing(&mut m, " ");
        settle(&mut m);
        assert_eq!(m.text(), "# Note\n\n`x teh ` y\n");
        let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(dir2), std::fs::remove_dir_all(dir3));
    }

    /// Where the system's settings are off (or there is none: Windows,
    /// Linux), typing is as it was.
    #[test]
    fn with_the_systems_settings_off_typing_is_as_it_was() {
        let dir = std::env::temp_dir().join(format!("kolada-app-typed-{}-off", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Note.md"), "# Note\n\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.open("Note.md");
        m.frame(1000, 700, 1.0);
        m.start_arrivals(|| {});
        m.read_replacements();
        m.editor.set_selection(Selection::caret(m.text().len()));
        typing(&mut m, "omw teh ");
        settle(&mut m);
        assert_eq!(m.text(), "# Note\n\nomw teh ");
        let _ = std::fs::remove_dir_all(dir);
    }
}
