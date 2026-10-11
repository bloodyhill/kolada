//! What other apps (and Notsy's other windows) did to the vault, as the
//! watcher (`crate::watch`) reports it: the side menu and search follow,
//! and a note open here shows their edits at once, unless it has unsaved
//! edits of its own, which are then kept beside theirs.

use crate::watch::{Action, Changes};

use super::*;

/// A file Notsy reads as a note.
fn is_note(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| ["md", "markdown", "txt"].contains(&ext.to_ascii_lowercase().as_str()))
}

/// Where a caret at `caret` in `old` is in `new`: where it was, before
/// what changed; as far from the end, after it; at the end of the new
/// text, inside it.
pub(super) fn map_caret(old: &str, new: &str, caret: usize) -> usize {
    let caret = caret.min(old.len());
    let prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - prefix;
    let suffix = old.bytes().rev().zip(new.bytes().rev()).take(room).take_while(|(a, b)| a == b).count();
    let at = if caret <= prefix {
        caret
    } else if caret >= old.len() - suffix {
        caret + new.len() - old.len()
    } else {
        new.len() - suffix
    };
    // On a character's first byte.
    (0..=at.min(new.len())).rev().find(|&i| new.is_char_boundary(i)).unwrap_or(0)
}

/// What a batch of changes means for the open note: changed (or made
/// again), removed, or renamed to a new path.
#[derive(Debug, Default, PartialEq)]
struct ForNote {
    changed: bool,
    removed: bool,
    renamed: Option<String>,
}

/// What `changes` did to the note at `rel`.
fn for_note(changes: &[(Action, String)], rel: &str) -> ForNote {
    let mut out = ForNote::default();
    let mut from = None;
    for (action, path) in changes {
        match action {
            Action::RenamedTo if from == Some(rel) => {
                out.renamed = Some(path.clone());
                out.removed = false;
            }
            _ if path != rel => {}
            // A file renamed over it is its new text (a safe save).
            Action::Modified | Action::Added | Action::RenamedTo => {
                out.changed = true;
                out.removed = false;
            }
            // Renamed where the watch does not see (out of the vault, or
            // into its trash) is gone too.
            Action::Removed | Action::RenamedFrom => out.removed = true,
        }
        from = (*action == Action::RenamedFrom).then_some(path.as_str());
    }
    out
}

impl App {
    /// Starts watching the vault (after the first frame, and when another
    /// vault opens).
    pub(super) fn watch_vault(&self) {
        crate::watch::start(self.hwnd, PathBuf::from(&self.session.vault));
    }

    pub(super) fn vault_changed(&mut self, changes: Changes) {
        if self.session.vault.is_empty() || changes.vault != Path::new(&self.session.vault) {
            return;
        }
        let everything = changes.overflow;
        let list = &changes.list;
        let tree = everything || list.iter().any(|(a, _)| *a != Action::Modified);
        let mut index = everything || list.iter().any(|(_, p)| is_note(p) && *p != self.session.note);
        if everything || list.iter().any(|(_, p)| crate::images::is_image(p)) {
            self.images.borrow_mut().forget_places();
            self.request_frame();
        }

        // The page's note.
        if !self.session.note.is_empty() {
            let page = if everything {
                ForNote { changed: true, ..ForNote::default() }
            } else {
                for_note(list, &self.session.note)
            };
            if let Some(to) = page.renamed {
                // Renamed elsewhere: it is followed, and shown by its new name.
                self.session.note = to;
                if self.editor.version() == self.saved_version {
                    let opened = read_note(&self.session);
                    let caret = map_caret(self.editor.text(), &opened.0, self.editor.selection().head);
                    self.load(opened, caret, false);
                } else {
                    self.disk = stamp(&self.note_path());
                }
            } else if page.removed && !self.note_path().exists() {
                self.whisper("Deleted in another app. Typing here keeps it".to_string());
            } else if page.changed && stamp(&self.note_path()) != self.disk {
                // Not this window's own save.
                index = true;
                if self.editor.version() == self.saved_version {
                    self.check_disk();
                } else {
                    // Both changed it: theirs stays, and these edits are
                    // saved beside it now (see `write`).
                    self.save_note(false);
                    self.whisper("Edited in another app too. Yours is kept as a copy".to_string());
                }
            }
        }

        // The note open over the page.
        if let Some(p) = &self.popup
            && let Some(rel) = vault::relative(Path::new(&self.session.vault), &p.path)
            && (everything || for_note(list, &rel).changed)
            && stamp(&p.path) != p.disk
            && p.editor.version() == p.saved_version
            && let Some((text, format, added)) = read_file(&p.path)
            && let Some(p) = &mut self.popup
        {
            let caret = map_caret(p.editor.text(), &text, p.editor.selection().head);
            p.editor = Editor::new(text);
            p.editor.set_snippets(super::daily::snippets());
            p.editor.set_selection(Selection::caret(caret));
            p.saved_version = p.editor.version();
            p.format = format;
            p.added = added;
            p.named = p.editor.title();
            p.disk = stamp(&p.path);
            index = true;
            self.request_frame();
        }

        if tree {
            self.refresh_tree();
        }
        if index {
            self.start_index();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(list: &[(Action, &str)]) -> Vec<(Action, String)> {
        list.iter().map(|(a, p)| (*a, p.to_string())).collect()
    }

    #[test]
    fn follows_the_open_note() {
        let note = "Projects/Plan.md";
        assert_eq!(for_note(&changes(&[(Action::Modified, note)]), note), ForNote { changed: true, ..ForNote::default() });
        assert_eq!(for_note(&changes(&[(Action::Modified, "Other.md")]), note), ForNote::default());
        // Saved the way many apps save: to a new file, then over the old.
        assert_eq!(
            for_note(&changes(&[(Action::Removed, note), (Action::Added, note), (Action::Modified, note)]), note),
            ForNote { changed: true, ..ForNote::default() }
        );
        assert_eq!(
            for_note(&changes(&[(Action::RenamedFrom, note), (Action::RenamedTo, "Projects/Launch plan.md")]), note),
            ForNote { renamed: Some("Projects/Launch plan.md".to_string()), ..ForNote::default() }
        );
        assert_eq!(for_note(&changes(&[(Action::Removed, note)]), note), ForNote { removed: true, ..ForNote::default() });
        // A safe save: written beside it, then renamed over it.
        assert_eq!(
            for_note(&changes(&[(Action::Added, "Projects/Plan.md.tmp"), (Action::RenamedFrom, "Projects/Plan.md.tmp"), (Action::RenamedTo, note)]), note),
            ForNote { changed: true, ..ForNote::default() }
        );
        // Moved to a hidden trash: the watch sees only it leaving.
        assert_eq!(for_note(&changes(&[(Action::RenamedFrom, note)]), note), ForNote { removed: true, ..ForNote::default() });
        assert!(is_note("a/b.MD") && !is_note("a/b.png"));
    }

    #[test]
    fn keeps_the_caret_where_it_was() {
        let old = "Plan\nShip it.";
        // A line added above: the caret moves down with its words.
        assert_eq!(map_caret(old, "Launch plan\nPlan\nShip it.", 7), 7 + 12);
        // Words added after it: it stays.
        assert_eq!(map_caret(old, "Plan\nShip it. Soon.", 7), 7);
        // Inside what was replaced: after the new words.
        assert_eq!(map_caret(old, "Plan\nShip nothing.", 11), "Plan\nShip nothing".len());
        assert_eq!(map_caret(old, "Plan\nShip nothing.", 8), 8, "before the change");
        assert_eq!(map_caret("é", "", 2), 0);
        assert_eq!(map_caret("aé", "aéé", 3), 3);
    }
}
