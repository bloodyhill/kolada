//! Short words that turn into longer text as you type. `;;alpha` and a space
//! becomes `α `, `;;date` becomes today's date. The `;;` makes a trigger
//! rare enough that ordinary writing never sets one off, and Ctrl+Z puts the
//! typed trigger back.
//!
//! The built-in set is a static table, so having it costs nothing at launch.
//! A person's own are lines of `name = text` (see [`Snippets::parse`]).

use crate::{Block, Editor, Kind};

/// Written before a name to make it a trigger.
const MARK: &str = ";;";
/// The longest a name may be.
const LONGEST: usize = 24;
/// Where the caret goes in an expansion that says so.
const CURSOR: &str = "{cursor}";

const BUILT_IN: &[(&str, &str)] = &[
    ("alpha", "α"), ("beta", "β"), ("gamma", "γ"), ("delta", "δ"), ("epsilon", "ε"), ("theta", "θ"),
    ("lambda", "λ"), ("mu", "μ"), ("pi", "π"), ("rho", "ρ"), ("sigma", "σ"), ("tau", "τ"), ("phi", "φ"),
    ("omega", "ω"), ("Delta", "Δ"), ("Sigma", "Σ"), ("Omega", "Ω"),
    ("to", "→"), ("from", "←"), ("up", "↑"), ("down", "↓"), ("both", "↔"), ("implies", "⇒"),
    ("le", "≤"), ("ge", "≥"), ("ne", "≠"), ("approx", "≈"), ("pm", "±"), ("times", "×"), ("div", "÷"),
    ("inf", "∞"), ("sqrt", "√"), ("sum", "∑"), ("deg", "°"),
    ("check", "✓"), ("cross", "✗"), ("star", "★"), ("heart", "♥"),
    ("dash", "—"), ("dots", "…"), ("copy", "©"), ("tm", "™"), ("euro", "€"), ("pound", "£"),
];

/// What triggers expand to.
#[derive(Clone, Debug)]
pub struct Snippets {
    /// A person's own, which win over the built-in ones.
    custom: Vec<(String, String)>,
    /// Says today as `2026-10-05`, for `;;date`: the shell's own clock, read
    /// when the trigger is typed. Without one `;;date` does nothing.
    today: Option<Today>,
    on: bool,
}

/// The shell's clock, shared (a closure may hold the calendar it reads).
#[derive(Clone)]
struct Today(std::sync::Arc<dyn Fn() -> String + Send + Sync>);

impl std::fmt::Debug for Today {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Today")
    }
}

impl Default for Snippets {
    fn default() -> Snippets {
        Snippets { custom: Vec::new(), today: None, on: false }
    }
}

impl Snippets {
    /// Reads a person's own: one `name = text` per line, `#` for comments.
    /// `\n` in the text is a line break, `\\` a backslash and `{cursor}`
    /// where the caret ends up. A line that is not one of these is skipped.
    pub fn parse(text: &str) -> Snippets {
        let custom = text
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| {
                let (name, body) = l.split_once('=')?;
                let name = name.trim();
                (is_name(name) && !body.trim().is_empty()).then(|| (name.to_string(), unescape(body.trim())))
            })
            .collect();
        Snippets { custom, on: true, ..Snippets::default() }
    }

    /// The built-in set, on. A new editor has none, so the fields around a
    /// note (search, find) never expand what is typed in them.
    pub fn standard() -> Snippets {
        Snippets { on: true, ..Snippets::default() }
    }

    pub fn with_today(mut self, today: impl Fn() -> String + Send + Sync + 'static) -> Snippets {
        self.today = Some(Today(std::sync::Arc::new(today)));
        self
    }

    pub fn turned(mut self, on: bool) -> Snippets {
        self.on = on;
        self
    }

    fn get(&self, name: &str) -> Option<String> {
        if let Some((_, body)) = self.custom.iter().rev().find(|(n, _)| n == name) {
            return Some(body.clone());
        }
        if name == "date" {
            return self.today.as_ref().map(|today| (today.0)());
        }
        BUILT_IN.iter().find(|(n, _)| *n == name).map(|(_, body)| body.to_string())
    }
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= LONGEST && s.bytes().all(is_name_byte)
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some('n')) => {
                out.push('\n');
                chars.next();
            }
            ('\\', Some('\\')) => {
                out.push('\\');
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// Whether typing `s` ends a trigger: one space or punctuation mark.
fn ends_trigger(s: &str) -> bool {
    let mut c = s.chars();
    match (c.next(), c.next()) {
        (Some(ch), None) => (ch.is_whitespace() && ch != '\n') || (ch.is_ascii_punctuation() && !matches!(ch, ';' | '_' | '-')),
        _ => false,
    }
}

impl Editor {
    pub fn set_snippets(&mut self, snippets: Snippets) {
        self.snippets = snippets;
    }

    /// If `s`, typed at the caret, ends a `;;name` that is a known snippet,
    /// swaps the trigger for its text in one undo step and says so.
    pub(crate) fn expand_snippet(&mut self, s: &str) -> bool {
        let at = self.selection.start();
        if !self.snippets.on || !self.selection.is_caret() || self.marked.is_some() || !ends_trigger(s) || at < MARK.len() + 1 {
            return false;
        }
        let before = &self.text[..at];
        let name_len = before.bytes().rev().take(LONGEST).take_while(|b| is_name_byte(*b)).count();
        if name_len == 0 {
            return false;
        }
        let head = at - name_len;
        if !before[..head].ends_with(MARK) {
            return false;
        }
        let start = head - MARK.len();
        if !self.text[..start].chars().next_back().is_none_or(|c| c.is_whitespace() || "([{\"'".contains(c)) {
            return false;
        }
        if matches!(self.line_at(at).block, Block::Fence | Block::Code | Block::Frontmatter) || self.cell_at(at).is_some() {
            return false;
        }
        let Some(body) = self.snippets.get(&self.text[head..at]) else { return false };
        let (text, caret) = match body.find(CURSOR) {
            // the typed character is spent on ending the trigger
            Some(i) => (body.replace(CURSOR, ""), start + i),
            None => (format!("{body}{s}"), start + body.len() + s.len()),
        };
        self.replace(Kind::Other, start, at - start, &text, caret);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Selection;

    fn typed(ed: &mut Editor, s: &str) {
        for g in s.chars() {
            ed.insert(&g.to_string());
        }
    }

    fn after(typed_text: &str) -> String {
        let mut ed = Editor::new("");
        ed.set_snippets(Snippets::standard());
        typed(&mut ed, typed_text);
        ed.text().to_string()
    }

    #[test]
    fn a_trigger_becomes_its_text_when_a_space_ends_it() {
        assert_eq!(after(";;alpha "), "α ");
        assert_eq!(after("x ;;le 3, y ;;ge 1."), "x ≤ 3, y ≥ 1.");
        assert_eq!(after("(;;to)"), "(→)");
    }

    #[test]
    fn it_waits_for_the_space_so_a_longer_name_can_be_typed() {
        assert_eq!(after(";;pi"), ";;pi");
        assert_eq!(after(";;pi "), "π ");
        assert_eq!(after(";;pix "), ";;pix ");
    }

    #[test]
    fn ordinary_writing_is_left_alone() {
        assert_eq!(after("alpha "), "alpha ");
        assert_eq!(after("a;;alpha "), "a;;alpha ", "must start a word");
        assert_eq!(after("http://x;;to "), "http://x;;to ");
        assert_eq!(after("; ;alpha "), "; ;alpha ");
        assert_eq!(after(";;nothing "), ";;nothing ");
    }

    #[test]
    fn undo_puts_the_typed_trigger_back_in_one_step() {
        let mut ed = Editor::new("");
        ed.set_snippets(Snippets::standard());
        typed(&mut ed, "hi ;;alpha ");
        assert_eq!(ed.text(), "hi α ");
        assert!(ed.undo());
        assert_eq!(ed.text(), "hi ;;alpha");
        assert_eq!(ed.selection().head, ed.text().len());
    }

    #[test]
    fn the_caret_lands_after_the_expansion() {
        let mut ed = Editor::new("");
        ed.set_snippets(Snippets::standard());
        typed(&mut ed, ";;to ");
        assert_eq!(ed.selection().head, ed.text().len());
        typed(&mut ed, "b");
        assert_eq!(ed.text(), "→ b");
    }

    #[test]
    fn code_and_table_cells_keep_what_is_typed() {
        let mut ed = Editor::new("```\n");
        ed.set_snippets(Snippets::standard());
        ed.set_selection(Selection::caret(4));
        typed(&mut ed, ";;alpha ");
        assert_eq!(ed.text(), "```\n;;alpha ");
        let mut ed = Editor::new("| a |\n| - |\n| b ");
        ed.set_snippets(Snippets::standard());
        ed.set_selection(Selection::caret(ed.text().len()));
        typed(&mut ed, ";;ge ");
        assert!(ed.text().contains(";;ge") && !ed.text().contains('≥'), "{}", ed.text());
    }

    #[test]
    fn date_needs_today_to_be_known() {
        assert_eq!(after(";;date "), ";;date ");
        let mut ed = Editor::new("");
        ed.set_snippets(Snippets::standard().with_today(|| "2026-10-05".into()));
        typed(&mut ed, "on ;;date.");
        assert_eq!(ed.text(), "on 2026-10-05.");
    }

    #[test]
    fn a_persons_own_snippets_win_and_can_set_the_caret() {
        let own = Snippets::parse("# mine\nsig = Best,\\nMo\nalpha = A\nmeet = ## Meeting {cursor}\nbroken line\n = x\n");
        let mut ed = Editor::new("");
        ed.set_snippets(own);
        typed(&mut ed, ";;sig ;;alpha ;;meet ");
        assert_eq!(ed.text(), "Best,\nMo A ## Meeting ");
        assert_eq!(ed.selection().head, ed.text().len());
    }

    #[test]
    fn a_cursor_marker_puts_the_caret_inside() {
        let mut ed = Editor::new("");
        ed.set_snippets(Snippets::parse("link = [[{cursor}]]"));
        typed(&mut ed, ";;link ");
        assert_eq!(ed.text(), "[[]]");
        assert_eq!(ed.selection().head, 2);
    }

    #[test]
    fn a_new_editor_does_not_expand_so_search_and_find_fields_stay_plain() {
        let mut ed = Editor::new("");
        typed(&mut ed, ";;alpha ");
        assert_eq!(ed.text(), ";;alpha ");
    }

    #[test]
    fn they_can_be_turned_off() {
        let mut ed = Editor::new("");
        ed.set_snippets(Snippets::standard().turned(false));
        typed(&mut ed, ";;alpha ");
        assert_eq!(ed.text(), ";;alpha ");
    }

    #[test]
    fn a_selection_is_typed_over_not_expanded() {
        let mut ed = Editor::new(";;alpha");
        ed.set_snippets(Snippets::standard());
        ed.set_selection(Selection { anchor: 0, head: 7 });
        ed.insert(" ");
        assert_eq!(ed.text(), " ");
    }
}
