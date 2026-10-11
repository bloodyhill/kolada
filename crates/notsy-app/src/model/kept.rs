//! Favorites and Recent in the side menu, as on Windows (`notsy-win`
//! `app.rs` `open_favorite`, `open_recent`, `toggle_favorite`): the notes
//! opened last, and notes and folders kept at hand, both in the session so
//! the first frame has them with no look at the vault. A row's menu (right
//! click, the three dots) adds a note or folder to the favorites.

use notsy_core::session::{FOLD_FAVORITES, FOLD_RECENT};
use notsy_ui::scene::Target;

use super::menu::{MenuAction, MenuItem};
use super::*;

impl Model {
    /// The note open now is recent too, however it was left: called as
    /// another is opened. A note of another vault is not.
    pub(super) fn remember_left(&mut self) {
        if self.path.as_ref().is_some_and(|p| p.starts_with(&self.vault)) {
            let left = self.session.note.clone();
            self.session.remember(&left);
        }
    }

    /// The note now open is the most recent.
    pub(super) fn remember_opened(&mut self) {
        let rel = self.session.note.clone();
        self.session.remember(&rel);
    }

    /// A note under Recent: opened, or let go if it is not there any more.
    pub fn open_recent(&mut self, i: usize) -> bool {
        let Some(rel) = self.session.recent.get(i).cloned() else { return false };
        if self.vault.join(&rel).is_file() {
            self.close_overlays();
            self.open(&rel);
        } else {
            self.session.recent.retain(|n| *n != rel);
            self.whisper("That note is not there any more");
        }
        true
    }

    /// A favorite: its note opened, or its folder opened out in the tree and
    /// scrolled to; one no longer there is let go.
    pub fn open_favorite(&mut self, i: usize) -> bool {
        let Some(fav) = self.session.favorites.get(i).cloned() else { return false };
        let Some(folder) = fav.strip_suffix('/') else {
            if self.vault.join(&fav).is_file() {
                self.close_overlays();
                self.open(&fav);
            } else {
                self.session.favorites.retain(|f| *f != fav);
                self.whisper("That note is not there any more");
            }
            return true;
        };
        if !self.vault.join(folder).is_dir() {
            self.session.favorites.retain(|f| *f != fav);
            self.whisper("That folder is not there any more");
            return true;
        }
        self.close_overlays();
        // Each folder on the way, opened out in turn.
        let (mut at, mut path) = (None, String::new());
        for part in folder.split('/') {
            path = if path.is_empty() { part.to_string() } else { format!("{path}/{part}") };
            let Some(i) = vault::row_paths(&self.session.sidebar).iter().position(|p| *p == path) else { break };
            if matches!(self.session.sidebar[i].kind, RowKind::Folder { expanded: false }) {
                self.click_row(i);
            }
            at = Some(i);
        }
        if let Some(i) = at {
            let max = self.session.sidebar.len().saturating_sub(self.ui.tree_rows.max(1));
            self.ui.tree_top = i.min(max);
        }
        true
    }

    /// For `notsy-mac --dump --open kept`: the vault's first notes opened
    /// one after another, so Recent has them, and the first folder and the
    /// first note among the favorites.
    pub fn demo_kept(&mut self) {
        let rows = vault::row_paths(&self.session.sidebar);
        let notes: Vec<String> = rows.iter().zip(&self.session.sidebar).filter(|(_, r)| r.kind == RowKind::Note).map(|(p, _)| format!("{p}.md")).collect();
        for rel in notes.iter().take(4) {
            self.open(rel);
        }
        if let Some((p, _)) = rows.iter().zip(&self.session.sidebar).find(|(_, r)| matches!(r.kind, RowKind::Folder { .. })) {
            self.session.toggle_favorite(&format!("{p}/"));
        }
        if let Some(first) = notes.first() {
            self.session.toggle_favorite(first);
        }
    }

    /// The search and the trash give way to a note opened from the menu.
    fn close_overlays(&mut self) {
        self.trash = None;
        self.search = None;
    }

    /// Adds a note (with `.md`) or folder (ending in `/`) to the favorites,
    /// or takes it off them.
    pub fn toggle_favorite(&mut self, path: &str) -> bool {
        let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path).trim_end_matches(".md").to_string();
        let kept = self.session.toggle_favorite(path);
        let full = !kept && self.session.favorites.iter().all(|f| f != path);
        self.whisper(&if full {
            "Favorites are full".to_string()
        } else if kept {
            format!("{name} is in Favorites")
        } else {
            format!("{name} is out of Favorites")
        });
        true
    }

    /// The menu's words for a note or folder: into the favorites, or out.
    fn favorite_label(&self, path: &str) -> &'static str {
        if self.session.favorites.iter().any(|f| f == path) { "Remove from Favorites" } else { "Add to Favorites" }
    }

    /// The menu item that puts `path` in the favorites or takes it out.
    pub(super) fn favorite_item(&self, path: String) -> MenuItem {
        MenuItem { label: self.favorite_label(&path).to_string(), key: "", action: MenuAction::Favorite(path), divider: false }
    }

    /// The menu of a row under Favorites or Recent: the note or folder it
    /// stands for, and the favorites.
    pub(super) fn kept_menu(&self, target: Target) -> Vec<MenuItem> {
        let item = |label: &str, action| MenuItem { label: label.to_string(), key: "", action, divider: false };
        match target {
            Target::Favorite(i) => match self.session.favorites.get(i) {
                Some(f) if f.ends_with('/') => vec![item("New Note Here", MenuAction::NewNoteIn(f.trim_end_matches('/').to_string())), self.favorite_item(f.clone())],
                Some(f) => vec![item("Open", MenuAction::Open(f.clone())), self.favorite_item(f.clone())],
                None => Vec::new(),
            },
            Target::Recent(i) => match self.session.recent.get(i) {
                Some(n) => {
                    let mut items = vec![item("Open", MenuAction::Open(n.clone())), self.favorite_item(n.clone())];
                    items.extend(self.recent_toggle().map(|mut t| {
                        t.divider = true;
                        t
                    }));
                    items
                }
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// "Hide Recent" or "Show Recent", where the vault is big enough to have
    /// it (`session::shows_recent`), as on Windows.
    pub(super) fn recent_toggle(&self) -> Option<MenuItem> {
        if self.session.folded & session::MANY_NOTES == 0 {
            return None;
        }
        let label = if self.session.folded & session::HIDE_RECENT == 0 { "Hide Recent" } else { "Show Recent" };
        Some(MenuItem { label: label.to_string(), key: "", action: MenuAction::ToggleRecent, divider: false })
    }

    /// A click on the side menu's Favorites or Recent: a row, or a heading
    /// that folds its section. None when the click is not about them.
    pub(super) fn kept_click(&mut self, target: Option<Target>) -> Option<bool> {
        match target? {
            Target::Recent(i) => Some(self.open_recent(i)),
            Target::Favorite(i) => Some(self.open_favorite(i)),
            Target::FoldFavorites => {
                self.session.folded ^= FOLD_FAVORITES;
                Some(true)
            }
            Target::FoldRecent => {
                self.session.folded ^= FOLD_RECENT;
                Some(true)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notsy_ui::scene::Rect;

    fn vault(test: &str, notes: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("notsy-app-kept-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in notes {
            let file = dir.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        dir
    }

    const SIZE: (u32, u32, f32) = (2000, 1600, 2.0);

    fn drawn(v: &Path) -> Model {
        let mut m = Model::new(v.to_path_buf(), true);
        // A vault counted big enough for Recent to show (`session::RECENT_FROM`).
        m.session.counted(notsy_core::session::RECENT_FROM + 5);
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        m
    }

    fn on_screen(m: &Model, pick: &dyn Fn(&Target) -> bool) -> Option<Rect> {
        m.hits.iter().find(|(_, t)| pick(t)).map(|(r, _)| *r)
    }

    fn click(m: &mut Model, target: Target) -> bool {
        let r = on_screen(m, &|t| *t == target).unwrap_or_else(|| panic!("{target:?} is not on screen"));
        let changed = m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false);
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        changed
    }

    fn recent_targets(m: &Model) -> usize {
        m.hits.iter().filter(|(_, t)| matches!(t, Target::Recent(_))).count()
    }

    #[test]
    fn the_notes_opened_last_are_recent_and_a_click_opens_one() {
        let v = vault("recent", &[("A.md", "# A\n\none\n"), ("B.md", "# B\n\ntwo\n"), ("C.md", "# C\n\nthree\n")]);
        let mut m = drawn(&v);
        m.open("A.md");
        m.open("B.md");
        m.open("C.md");
        assert_eq!(&m.session.recent[..3], ["C.md", "B.md", "A.md"], "the open note is the newest");
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        // The open note is not listed under Recent; the others are.
        assert!(on_screen(&m, &|t| matches!(t, Target::Recent(0))).is_none(), "not the note open now");
        assert!(recent_targets(&m) >= 2);
        assert!(click(&mut m, Target::Recent(1)));
        assert_eq!(m.session.note, "B.md");
        assert_eq!(m.session.recent[0], "B.md");
        // A note no longer there is let go.
        std::fs::remove_file(v.join("A.md")).unwrap();
        let i = m.session.recent.iter().position(|n| n == "A.md").unwrap();
        m.open_recent(i);
        assert!(!m.session.recent.iter().any(|n| n == "A.md"));
        assert_eq!(m.session.note, "B.md");
        assert!(m.whispered().is_some_and(|w| w.contains("not there")));
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn a_note_whose_file_follows_its_title_stays_recent_under_the_new_name() {
        let v = vault("renamed", &[("Old name.md", "# Old name\n\nwords\n"), ("Other.md", "# Other\n\nmore\n")]);
        let mut m = drawn(&v);
        m.open("Old name.md");
        m.open("Other.md");
        // Retitled and left: the file takes the new title, and so does Recent.
        m.open("Old name.md");
        m.editor.set_selection(notsy_editor::Selection::caret(10));
        m.insert(" plan");
        m.open("Other.md");
        assert!(v.join("Old name plan.md").is_file(), "the file follows its title");
        assert!(!m.session.recent.iter().any(|n| n == "Old name.md"), "{:?}", m.session.recent);
        assert!(m.session.recent.iter().any(|n| n == "Old name plan.md"), "{:?}", m.session.recent);
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn a_note_or_folder_is_added_to_favorites_from_its_menu_and_opens_from_there() {
        let v = vault("favorites", &[("Plan.md", "# Plan\n\nx\n"), ("Home.md", "# Home\n\ny\n"), ("Work/Deep/Task.md", "# Task\n\nz\n")]);
        let mut m = drawn(&v);
        m.open("Home.md");
        // The tree row's menu offers it.
        let plan = m.session.sidebar.iter().position(|r| r.name == "Plan").unwrap();
        let items = m.row_menu(plan);
        let add = items.iter().find(|i| i.label == "Add to Favorites").expect("a note's menu has it");
        assert_eq!(add.action, MenuAction::Favorite("Plan.md".into()));
        let work = m.session.sidebar.iter().position(|r| r.name == "Work").unwrap();
        assert!(m.row_menu(work).iter().any(|i| i.action == MenuAction::Favorite("Work/".into())), "a folder's too");
        m.run_menu(MenuAction::Favorite("Plan.md".into()));
        m.run_menu(MenuAction::Favorite("Work/Deep/".into()));
        assert_eq!(m.session.favorites, ["Plan.md", "Work/Deep/"]);
        let plan = m.session.sidebar.iter().position(|r| r.name == "Plan").unwrap();
        assert!(m.row_menu(plan).iter().any(|i| i.label == "Remove from Favorites"));
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        // The side menu lists them first, above Recent and the tree.
        let fav = on_screen(&m, &|t| *t == Target::Favorite(0)).expect("a favorite is listed");
        let tree = on_screen(&m, &|t| matches!(t, Target::Row(_))).unwrap();
        assert!(fav.top < tree.top);
        // Their menu takes them off again.
        assert!(m.kept_menu(Target::Favorite(0)).iter().any(|i| i.label == "Remove from Favorites"));
        // A note opens; a folder opens out in the tree.
        assert!(click(&mut m, Target::Favorite(0)));
        assert_eq!(m.session.note, "Plan.md");
        assert!(click(&mut m, Target::Favorite(1)));
        let rows = vault::row_paths(&m.session.sidebar);
        assert!(rows.iter().any(|p| p == "Work/Deep"), "the way to it is opened out: {rows:?}");
        // Taken off, and one that is gone is let go.
        m.run_menu(MenuAction::Favorite("Plan.md".into()));
        assert_eq!(m.session.favorites, ["Work/Deep/"]);
        std::fs::remove_dir_all(v.join("Work")).unwrap();
        m.open_favorite(0);
        assert!(m.session.favorites.is_empty());
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn a_heading_folds_its_section_and_the_lists_are_kept_in_the_session() {
        let v = vault("fold", &[("A.md", "# A\n\n1\n"), ("B.md", "# B\n\n2\n"), ("C.md", "# C\n\n3\n")]);
        let mut m = drawn(&v);
        m.open("A.md");
        m.open("B.md");
        m.open("C.md");
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        assert!(recent_targets(&m) > 0);
        assert!(click(&mut m, Target::FoldRecent));
        assert_eq!(recent_targets(&m), 0, "folded shut, its heading stays");
        assert!(on_screen(&m, &|t| *t == Target::FoldRecent).is_some());
        assert!(click(&mut m, Target::FoldRecent));
        assert!(recent_targets(&m) > 0);
        m.toggle_favorite("A.md");
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        assert!(click(&mut m, Target::FoldFavorites));
        // The fold bits (the vault's count rides along in `folded` too).
        let folds = |f: u8| f & (FOLD_FAVORITES | FOLD_RECENT);
        assert_eq!(folds(m.session.folded), FOLD_FAVORITES);
        // What the next launch finds.
        let back = notsy_core::session::decode(&notsy_core::session::encode(&m.session)).unwrap();
        assert_eq!((back.recent, back.favorites, folds(back.folded)), (m.session.recent.clone(), vec!["A.md".to_string()], FOLD_FAVORITES));
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn another_vault_starts_recent_over() {
        let (a, b) = (vault("va", &[("A.md", "# A\n\n1\n"), ("A2.md", "# A2\n\n1\n")]), vault("vb", &[("B.md", "# B\n\n2\n")]));
        let mut m = drawn(&a);
        m.open("A.md");
        m.open("A2.md");
        assert!(m.switch_vault(b.clone()));
        assert_eq!(m.session.recent, ["B.md"], "none of the old vault's notes");
        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }

    #[test]
    fn recent_shows_in_a_vault_counted_past_25_notes_and_the_side_menu_hides_it() {
        let v = vault("recentcount", &[("Plan.md", "# Plan\n"), ("Other.md", "# Other\n")]);
        let mut m = Model::new(v.clone(), true);
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        m.open("Other.md");
        m.open("Plan.md");
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        assert_eq!(recent_targets(&m), 0, "a small vault: the tree, newest first, is enough");
        assert!(m.recent_toggle().is_none(), "nothing to hide");
        // The index counts the vault: past 25 notes, Recent shows.
        *m.index.lock().unwrap() = Some((0..30).map(|i| Indexed::new(format!("N{i}.md"), String::new())).collect());
        assert!(m.index_arrived(), "counting it changes the side menu");
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        assert!(recent_targets(&m) > 0);
        // A right-click on the side menu's background hides it, and shows it again.
        let bg = on_screen(&m, &|t| *t == Target::SidebarPanel).unwrap();
        let menu = m.context_menu(bg.left + 4.0, bg.bottom - 60.0, false);
        assert_eq!(menu.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["Hide Recent"]);
        m.run_menu(MenuAction::ToggleRecent);
        m.frame(SIZE.0, SIZE.1, SIZE.2);
        assert_eq!(recent_targets(&m), 0, "hidden");
        assert_eq!(m.recent_toggle().map(|i| i.label), Some("Show Recent".to_string()));
        let _ = std::fs::remove_dir_all(v);
    }
}
