//! Deleting notes for good, and taking a delete back. A deleted note waits in
//! the vault's `.trash` (see `kolada_core::trash`); removing it from there
//! moves it to the Mac's own Trash (never beyond), through `bin`, which the
//! app sets to the AppKit call (`mac/bin.rs`) and tests set to a fake.

use super::*;

/// Moves a file to the Mac's Trash: whether it went.
pub type Bin = Box<dyn Fn(&Path) -> bool>;

impl Model {
    /// The deleted note at `i` in the trash.
    fn trashed(&self, i: usize) -> Option<Trashed> {
        self.trash.as_ref().and_then(|(items, _)| items.get(i)).cloned()
    }

    /// Puts a deleted note back and opens it: whether it came back.
    pub(super) fn bring_back(&mut self, item: &Trashed) -> bool {
        let Ok(path) = trash::restore(&self.vault, item, &self.trash_log()) else { return false };
        self.undo_delete.retain(|(t, _)| t.file != item.file);
        self.refresh_tree();
        if let Some(rel) = vault::relative(&self.vault, &path) {
            self.open(&rel);
        }
        true
    }

    /// Cmd+Z right after deleting a note (nothing typed since): it comes
    /// back, and opens.
    pub(super) fn undo_last_delete(&mut self) -> bool {
        let Some((item, version)) = self.undo_delete.last().cloned() else { return false };
        if version != self.editor.version() {
            // Typing since: Cmd+Z is the text's again.
            self.undo_delete.clear();
            return false;
        }
        let back = self.bring_back(&item);
        if back && self.trash.is_some() {
            self.open_trash();
        }
        back
    }

    /// An item of a trash row's menu picked.
    pub fn trash_pick(&mut self, action: super::menu::MenuAction, i: usize) -> bool {
        use super::menu::MenuAction;
        match action {
            MenuAction::Restore(_) => self.restore(i),
            MenuAction::DeleteForGood(_) => self.delete_for_good(i),
            _ => false,
        }
    }

    /// The `i`th note in the trash goes to the Mac's Trash, and is forgotten.
    pub(super) fn delete_for_good(&mut self, i: usize) -> bool {
        let Some(item) = self.trashed(i) else { return false };
        let Some(bin) = &self.bin else { return false };
        if !bin(&item.file) {
            return false;
        }
        trash::forget(&self.vault, &item, &self.trash_log());
        self.undo_delete.retain(|(t, _)| t.file != item.file);
        self.open_trash();
        true
    }

    /// Every note in the trash goes to the Mac's Trash: whether any did.
    pub(super) fn empty_trash(&mut self) -> bool {
        let Some((items, _)) = self.trash.take() else { return false };
        let mut moved = 0;
        let log = self.trash_log();
        if let Some(bin) = &self.bin {
            for item in &items {
                if bin(&item.file) {
                    trash::forget(&self.vault, item, &log);
                    moved += 1;
                }
            }
        }
        self.undo_delete.clear();
        self.open_trash();
        moved > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn model(test: &str, notes: &[(&str, &str)]) -> (Model, PathBuf, Rc<RefCell<Vec<PathBuf>>>) {
        let dir = std::env::temp_dir().join(format!("kolada-app-trash-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in notes {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        // A fake bin: it takes the file off the disk, as the Trash would.
        let taken = Rc::new(RefCell::new(Vec::new()));
        let seen = taken.clone();
        m.bin = Some(Box::new(move |p| {
            seen.borrow_mut().push(p.to_path_buf());
            std::fs::remove_file(p).is_ok()
        }));
        (m, dir, taken)
    }

    #[test]
    fn cmd_z_right_after_deleting_a_note_brings_it_back_and_opens_it() {
        let (mut m, dir, _) = model("undo", &[("Old.md", "# Old\n\nbye\n"), ("Keep.md", "# Keep\n")]);
        m.open("Old.md");
        assert!(m.trash_open_note());
        assert!(!dir.join("Old.md").exists());
        assert_eq!(m.session.note, "");
        assert!(m.undo(), "Cmd+Z undoes the delete");
        assert!(dir.join("Old.md").exists());
        assert_eq!(m.session.note, "Old.md");
        assert!(m.text().starts_with("# Old"));
        assert!(!m.undo_last_delete(), "only once");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn once_typing_starts_cmd_z_is_the_texts_not_the_deletes() {
        let (mut m, dir, _) = model("typed", &[("Old.md", "# Old\n")]);
        m.open("Old.md");
        assert!(m.trash_open_note());
        m.insert("typed on the fresh page");
        assert!(m.undo(), "the typing is undone");
        assert!(!dir.join("Old.md").exists(), "the note stays deleted");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_note_removed_for_good_goes_to_the_macs_trash_and_cannot_be_undone() {
        let (mut m, dir, taken) = model("forever", &[("Old.md", "# Old\n"), ("Other.md", "# Other\n")]);
        m.open("Old.md");
        assert!(m.trash_open_note());
        assert!(m.open_trash());
        let i = m.trash.as_ref().unwrap().0.iter().position(|t| t.name() == "Old").unwrap();
        let file = m.trash.as_ref().unwrap().0[i].file.clone();
        assert!(m.delete_for_good(i));
        assert_eq!(*taken.borrow(), vec![file.clone()], "handed to the Mac's Trash");
        assert!(!file.exists());
        assert!(m.trash.as_ref().unwrap().0.iter().all(|t| t.name() != "Old"), "gone from the list");
        assert!(!m.undo_last_delete(), "nothing left to bring back");
        // Without a bin (a dump), nothing is removed.
        m.bin = None;
        m.open("Other.md");
        assert!(m.trash_open_note() && m.open_trash());
        assert!(!m.delete_for_good(0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_right_click_on_a_deleted_note_offers_to_bring_it_back_or_remove_it() {
        use super::super::menu::MenuAction;
        let (mut m, dir, taken) = model("menu", &[("Old.md", "# Old\n")]);
        m.open("Old.md");
        assert!(m.trash_open_note() && m.open_trash());
        m.frame(2000, 1440, 2.0);
        let (r, _) = *m.hits.iter().find(|(_, t)| matches!(t, Target::TrashRow(0))).expect("the deleted note's row");
        let items = m.context_menu((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, false);
        assert_eq!(items.iter().map(|i| (i.label.as_str(), i.action.clone())).collect::<Vec<_>>(), [("Bring Back", MenuAction::Restore(0)), ("Move to Trash", MenuAction::DeleteForGood(0))]);
        assert!(m.trash_pick(MenuAction::DeleteForGood(0), 0));
        assert_eq!(taken.borrow().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn emptying_the_trash_moves_every_note_to_the_macs_trash() {
        let (mut m, dir, taken) = model("empty", &[("A.md", "# A\n"), ("B.md", "# B\n"), ("C.md", "# C\n")]);
        for n in ["A.md", "B.md"] {
            m.open(n);
            assert!(m.trash_open_note());
        }
        assert!(m.open_trash());
        let listed = m.trash.as_ref().unwrap().0.len();
        assert!(listed >= 2);
        assert!(m.empty_trash());
        assert_eq!(taken.borrow().len(), listed);
        assert!(m.trash.as_ref().unwrap().0.is_empty(), "the trash is empty");
        assert!(dir.join("C.md").exists(), "the other notes are not touched");
        assert!(!m.undo_last_delete());
        // It is the Empty button's work (the click lands on it).
        m.open("C.md");
        assert!(m.trash_open_note() && m.open_trash());
        m.frame(2000, 1440, 2.0);
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::TrashEmpty).expect("the Empty button");
        assert!(m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false));
        assert!(m.trash.as_ref().unwrap().0.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
