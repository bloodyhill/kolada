//! Linking as you write: after `[[`, the notes whose names fit what follows
//! show under the caret. Up and Down pick, Enter or Tab links it, Esc lets
//! the brackets be. With no note by that name, the last row makes one when
//! the link is followed.

use notsy_core::search;

use super::*;

/// Names shown at most.
const SHOWN: usize = 6;

pub struct Suggest {
    /// Where the name being typed starts (just after `[[`).
    start: usize,
    pub names: Vec<(String, String)>,
    pub selected: usize,
    /// The name as typed, when no note has it.
    pub create: Option<String>,
}

/// The `[[link]]` being typed at `caret`: where its name starts and what
/// it is so far. None outside one, or past its name (in an alias or a
/// heading).
pub fn link_at_caret(text: &str, caret: usize) -> Option<(usize, &str)> {
    let caret = caret.min(text.len());
    let line_start = text[..caret].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line_start..caret];
    let open = before.rfind("[[")?;
    let typed = &before[open + 2..];
    if typed.contains("]]") || typed.contains(['|', '#', '^', '[', ']']) || typed.chars().count() > 80 {
        return None;
    }
    Some((line_start + open + 2, typed))
}

impl App {
    /// Finds suggestions for the link at the caret, or puts them away.
    pub(super) fn update_suggestions(&mut self) {
        let head = self.editor.selection().head;
        let found = if self.read_only || !self.editor.selection().is_caret() || self.popup.is_some() {
            None
        } else {
            link_at_caret(self.editor.text(), head)
        };
        let Some((start, typed)) = found else {
            self.suggest_dismissed = None;
            if self.suggest.take().is_some() {
                self.request_frame();
            }
            return;
        };
        // Put away with Esc for this link: stays away until it is left.
        if self.suggest_dismissed == Some(start) {
            return;
        }
        let names: Vec<(String, String)> = search::complete(&self.index, typed, SHOWN)
            .into_iter()
            .map(|n| {
                let folder = n.path.rsplit_once('/').map_or(String::new(), |(f, _)| f.to_string());
                (n.name.clone(), folder)
            })
            .collect();
        let exact = names.iter().any(|(n, _)| n.to_lowercase() == typed.trim().to_lowercase());
        let create = (!typed.trim().is_empty() && !exact).then(|| typed.trim().to_string());
        let selected = self.suggest.as_ref().filter(|s| s.start == start).map_or(0, |s| s.selected);
        let rows = names.len() + create.is_some() as usize;
        self.suggest = (rows > 0).then(|| Suggest { start, names, selected: selected.min(rows.saturating_sub(1)), create });
        self.request_frame();
    }

    /// Links the `i`th suggestion: the name typed so far becomes it, and
    /// the link closes.
    pub(super) fn accept_suggestion(&mut self, i: usize) {
        let Some(s) = self.suggest.take() else { return };
        let name = match s.names.get(i) {
            Some((name, _)) => name.clone(),
            None => match s.create {
                Some(name) => name,
                None => return,
            },
        };
        let head = self.editor.selection().head;
        let text = self.editor.text();
        // Brackets typed already stay; otherwise they close the link.
        let closed = text[head..].starts_with("]]");
        let insert = if closed { name } else { format!("{name}]]") };
        let start = s.start;
        self.editor.set_selection(Selection { anchor: start, head });
        self.edit(|e| {
            e.insert(&insert);
            if closed {
                let at = e.selection().head + 2;
                e.set_selection(Selection::caret(at));
            }
        });
        self.suggest_dismissed = None;
    }

    /// Keys while suggestions show; false for keys that are not theirs.
    pub(super) fn suggest_key(&mut self, vk: u16) -> bool {
        let Some(s) = &mut self.suggest else { return false };
        let rows = s.names.len() + s.create.is_some() as usize;
        match vk {
            v if v == VK_DOWN.0 => s.selected = (s.selected + 1) % rows,
            v if v == VK_UP.0 => s.selected = (s.selected + rows - 1) % rows,
            v if v == VK_RETURN.0 || v == VK_TAB.0 => {
                let i = s.selected;
                self.accept_suggestion(i);
            }
            v if v == VK_ESCAPE.0 => {
                self.suggest_dismissed = Some(s.start);
                self.suggest = None;
            }
            _ => return false,
        }
        self.request_frame();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::link_at_caret;

    #[test]
    fn knows_the_link_being_typed() {
        let text = "See [[Pric";
        assert_eq!(link_at_caret(text, text.len()), Some((6, "Pric")));
        assert_eq!(link_at_caret("See [[", 6), Some((6, "")));
        assert_eq!(link_at_caret("![[Img", 6), Some((3, "Img")), "embeds too");
        assert_eq!(link_at_caret("[[Note|ali", 10), None, "past the name");
        assert_eq!(link_at_caret("[[Done]] and", 12), None, "closed");
        assert_eq!(link_at_caret("[[a\nb", 5), None, "on another line");
        let arabic = "انظر [[ملا";
        assert_eq!(link_at_caret(arabic, arabic.len()).map(|(_, t)| t), Some("ملا"));
    }
}
