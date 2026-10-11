//! What the model may not change is not shown to it: code, links, embeds,
//! addresses and tags become numbered markers, and each must come back
//! exactly once, in order, for an answer to count. Emphasis marks stay as
//! they are (a model reads `**` well); the guard checks they survive.

use notsy_markdown::{self as md, Style};

pub fn marker(n: usize) -> String {
    format!("⟦{n}⟧")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Masked {
    /// The text as the model sees it.
    pub text: String,
    /// What each marker stands for, in order.
    pub pieces: Vec<String>,
}

/// A paragraph's text (after its block mark) with its protected spans as
/// markers.
pub fn mask(content: &str) -> Masked {
    let mut text = String::with_capacity(content.len());
    let mut pieces = Vec::new();
    let mut at = 0;
    for s in md::spans(content) {
        let atom = matches!(s.style, Style::Code | Style::WikiLink | Style::Link | Style::Url | Style::Tag);
        if !atom || s.start < at {
            continue;
        }
        text.push_str(&content[at..s.start]);
        pieces.push(content[s.start..s.end].to_string());
        text.push_str(&marker(pieces.len()));
        at = s.end;
    }
    text.push_str(&content[at..]);
    Masked { text, pieces }
}

impl Masked {
    /// The model's answer with its markers put back; None unless each came
    /// back once, in order, and nothing else looks like one.
    pub fn unmask(&self, answer: &str) -> Option<String> {
        let mut out = String::with_capacity(answer.len() + self.pieces.iter().map(String::len).sum::<usize>());
        let mut rest = answer;
        for (i, piece) in self.pieces.iter().enumerate() {
            let m = marker(i + 1);
            let at = rest.find(&m)?;
            let before = &rest[..at];
            if before.contains('⟦') || before.contains('⟧') {
                return None;
            }
            out.push_str(before);
            out.push_str(piece);
            rest = &rest[at + m.len()..];
        }
        if rest.contains('⟦') || rest.contains('⟧') {
            return None;
        }
        out.push_str(rest);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_spans_become_markers_and_come_back() {
        let m = mask("see [[Plans|the plan]] and `code` at https://x.io #todo, **really**");
        assert_eq!(m.text, "see ⟦1⟧ and ⟦2⟧ at ⟦3⟧ ⟦4⟧, **really**");
        assert_eq!(m.pieces, ["[[Plans|the plan]]", "`code`", "https://x.io", "#todo"]);
        assert_eq!(
            m.unmask("See ⟦1⟧ and ⟦2⟧ at ⟦3⟧ ⟦4⟧, **really**.").as_deref(),
            Some("See [[Plans|the plan]] and `code` at https://x.io #todo, **really**.")
        );
    }

    #[test]
    fn markers_lost_moved_or_doubled_fail() {
        let m = mask("a [[b]] c `d`");
        assert_eq!(m.unmask("a c ⟦2⟧"), None);
        assert_eq!(m.unmask("a ⟦2⟧ c ⟦1⟧"), None);
        assert_eq!(m.unmask("a ⟦1⟧ ⟦1⟧ c ⟦2⟧"), None);
        assert_eq!(m.unmask("a ⟦1⟧ c ⟦2⟧ ⟦3⟧"), None);
        assert_eq!(mask("plain words").unmask("Plain words.").as_deref(), Some("Plain words."));
    }
}
