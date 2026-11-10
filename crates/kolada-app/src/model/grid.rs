//! The notes as a grid of boxes under the note (see `kolada_ui::grid`),
//! when that is the view chosen (`Session::view`), as Windows' `app/grid.rs`
//! has it: the list of notes and each box in sight are read on workers
//! after the first frame, and come back through the same wake as the
//! vault's changes (`poll_outside`); the split's spring steps with the
//! other motion.

use std::sync::mpsc::{Receiver, Sender, channel};

use kolada_core::vault::Listed;
use kolada_ui::grid::{self, BoxView, Grid, GridSearch, GridView, Notes, Phase};

use super::*;

/// The grid while it is the view: what is being done, and its notes.
pub(super) struct GridState {
    pub grid: Grid,
    pub notes: Notes,
}

/// What a worker read: the notes (newest first), or boxes.
enum GridRead {
    Listed(Vec<Listed>),
    Boxes(Vec<grid::Read>),
}

/// The grid, and the way its workers' readings come back.
#[derive(Default)]
pub(super) struct Grids {
    pub state: Option<GridState>,
    channel: Option<(Sender<GridRead>, Receiver<GridRead>)>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Grids {
    fn sender(&mut self) -> Sender<GridRead> {
        self.channel.get_or_insert_with(channel).0.clone()
    }
}

impl Model {
    /// At launch: the grid, when it is the view (not in a note's own
    /// window); its boxes come after the first frame.
    pub(super) fn grid_at_launch(&mut self) {
        if self.session.view == 1 && !self.compact {
            self.grid.state = Some(GridState { grid: Grid::new(self.session.note.is_empty()), notes: Notes::default() });
        }
    }

    /// After the first frame (`start_watching`): the grid's notes are
    /// listed, and `wake` brings back what its workers read.
    pub(super) fn start_grid(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.grid.wake = Some(wake);
        self.list_grid();
    }

    /// The grid is the view.
    pub fn grid_shown(&self) -> bool {
        self.grid.state.is_some()
    }

    /// The side menu's view icon, the grid's, or the View menu: the list or
    /// the grid, kept in the session. Out of a note into the grid, the boxes
    /// come up under it to choose from.
    pub fn toggle_view(&mut self) -> bool {
        // A search in either view's field ends: the other view has its own.
        if self.search.is_some() {
            self.close_search();
        }
        if self.grid.state.take().is_some() {
            self.session.view = 0;
        } else {
            let fresh = self.session.note.is_empty() && self.editor.text().is_empty();
            let mut grid = Grid::new(fresh);
            if !fresh {
                grid.phase = Phase::Browse;
            }
            self.grid.state = Some(GridState { grid, notes: Notes::default() });
            self.session.view = 1;
            self.list_grid();
        }
        self.persist_session();
        true
    }

    /// Lists the vault's notes on a worker.
    fn list_grid(&mut self) {
        if self.grid.state.is_none() {
            return;
        }
        let (tx, vault, wake) = (self.grid.sender(), self.vault.clone(), self.grid.wake.clone());
        std::thread::spawn(move || {
            let _ = tx.send(GridRead::Listed(vault::every_note(&vault)));
            if let Some(wake) = wake {
                wake();
            }
        });
    }

    /// Reads the boxes in sight not read yet, on a worker.
    fn read_grid(&mut self) {
        let (w, h) = self.viewport;
        let Some(g) = &mut self.grid.state else { return };
        let wanted = g.notes.wanted(&g.grid, w, h);
        if wanted.is_empty() {
            return;
        }
        let (tx, vault, wake) = (self.grid.sender(), self.vault.clone(), self.grid.wake.clone());
        std::thread::spawn(move || {
            let _ = tx.send(GridRead::Boxes(grid::read(&vault, wanted)));
            if let Some(wake) = wake {
                wake();
            }
        });
    }

    /// What the grid's workers read since the last look: whether the
    /// screen changes.
    pub(super) fn grid_arrived(&mut self) -> bool {
        let mut came = Vec::new();
        if let Some((_, rx)) = &self.grid.channel {
            came.extend(rx.try_iter());
        }
        let Some(g) = &mut self.grid.state else { return false };
        if came.is_empty() {
            return false;
        }
        for read in came {
            match read {
                GridRead::Listed(list) => g.notes.listed(list),
                GridRead::Boxes(boxes) => g.notes.read(boxes),
            }
        }
        self.read_grid();
        true
    }

    /// The open note was saved: it is the newest now, its box read again.
    pub(super) fn grid_saved(&mut self) {
        let rel = self.session.note.clone();
        if let Some(g) = &mut self.grid.state {
            g.notes.saved(&rel);
            self.read_grid();
        }
    }

    /// Typed into the note: it takes the window.
    pub(super) fn grid_typed(&mut self) {
        if let Some(g) = &mut self.grid.state {
            g.grid.typed();
        }
    }

    /// A note opened (or a fresh page): writing it, or at rest.
    pub(super) fn grid_opened(&mut self) {
        let fresh = self.session.note.is_empty() && self.editor.text().is_empty();
        if let Some(g) = &mut self.grid.state {
            if fresh {
                g.grid.fresh();
            } else {
                g.grid.back_to_note();
            }
        }
    }

    /// The pointer at `y` DIPs, or gone: at the window's foot the boxes
    /// peek. Whether the screen changes.
    pub(super) fn grid_pointer(&mut self, y: Option<f32>) -> bool {
        let h = self.viewport.1;
        let changed = self.grid.state.as_mut().is_some_and(|g| g.grid.pointer(y, h));
        if changed {
            self.read_grid();
        }
        changed
    }

    /// A click on the grid: None if it was not the grid's, else whether the
    /// screen changes.
    pub(super) fn grid_click(&mut self, target: Target) -> Option<bool> {
        let g = self.grid.state.as_mut()?;
        match target {
            Target::GridSheet => {
                g.grid.clicked();
            }
            // Peeking, a click brings the boxes up; browsing, it opens one.
            Target::GridBox(_) if g.grid.phase == Phase::Peek => {
                g.grid.clicked();
            }
            Target::GridBox(i) => {
                let Some(rel) = g.notes.note(i).map(str::to_string) else { return Some(false) };
                // A search's card chosen: the search is done, the note opens
                // on what it found.
                let query = self.grid_query();
                self.close_search();
                self.open(&rel);
                if let Some(q) = query {
                    self.opened_from_search(&q);
                }
            }
            Target::GridNote => {
                g.grid.back_to_note();
                self.close_search();
            }
            // The pill at the foot: the boxes come up (riding their edge, down).
            Target::GridPill => {
                g.grid.pill_clicked();
            }
            _ => return None,
        }
        self.read_grid();
        Some(true)
    }

    /// The search typed in the grid's bar, a frame on: its notes are the
    /// boxes, from the top (the boxes come up to choose from); nothing
    /// typed, or no search, the notes again. With ">" typed, the palette
    /// drops in over the boxes instead (`grid_palette`).
    pub(super) fn grid_follow_search(&mut self) {
        if self.grid.state.is_none() {
            return;
        }
        // While the index is read (at first, or again after a change from
        // outside): the matches there were stay; before the first answer,
        // the notes (never "No notes match" before it is known).
        let typed = self.search.as_ref().is_some_and(|s| s.commands.is_none() && !s.query.text().trim().is_empty());
        if typed && self.search.as_ref().is_some_and(|s| !s.fresh) {
            return;
        }
        let found = self.search.as_ref().filter(|_| typed).map(|s| grid::matches(&s.hits));
        let Some(g) = &mut self.grid.state else { return };
        let same = match (&found, g.notes.searching()) {
            (None, false) => true,
            (Some(m), true) => g.notes.count() == m.len() && (0..m.len()).all(|i| g.notes.note(i) == Some(m[i].0.as_str())),
            _ => false,
        };
        if same {
            return;
        }
        g.notes.found(found);
        g.grid.scroll = 0.0;
        self.read_grid();
    }

    /// The search's field opened while the grid is the view: it is the
    /// grid bar's, and the boxes come up under it.
    pub(super) fn grid_search_opened(&mut self) {
        if let Some(g) = &mut self.grid.state {
            g.grid.browse();
        }
    }

    /// Up or Down in the grid's search: the box above or below the chosen
    /// one, kept in sight. None when the grid does not take it.
    pub(super) fn grid_search_step(&mut self, down: bool) -> Option<bool> {
        let (w, h) = self.viewport;
        let g = self.grid.state.as_mut()?;
        let s = self.search.as_mut().filter(|s| s.focused && s.commands.is_none())?;
        let count = g.notes.count();
        if !g.notes.searching() || count == 0 {
            return Some(false);
        }
        let at = grid::step(Some(s.selected.min(count - 1)), if down { grid::Step::Down } else { grid::Step::Up }, grid::columns(w), count, false);
        s.selected = at.unwrap_or(0);
        g.grid.reveal(s.selected, count, w, h);
        self.read_grid();
        Some(true)
    }

    /// Return in the grid's search: the chosen box's note opens, the search
    /// ends. None when the grid does not take it.
    pub(super) fn grid_search_open(&mut self) -> Option<bool> {
        let g = self.grid.state.as_ref()?;
        let s = self.search.as_ref().filter(|s| s.focused && s.commands.is_none())?;
        let count = g.notes.count();
        let Some(rel) = (g.notes.searching() && count > 0).then(|| g.notes.note(s.selected.min(count - 1))).flatten().map(str::to_string) else { return Some(false) };
        let query = self.grid_query();
        self.close_search();
        self.open(&rel);
        if let Some(q) = query {
            self.opened_from_search(&q);
        }
        Some(true)
    }

    /// The search the grid's boxes are the matches of, if they are.
    fn grid_query(&self) -> Option<String> {
        let searching = self.grid.state.as_ref().is_some_and(|g| g.notes.searching());
        self.search.as_ref().filter(|s| searching && s.commands.is_none()).map(|s| s.query.text().to_string())
    }

    /// The side menu's key while the grid is the view: the grid's own way,
    /// the boxes up to choose from, or back to the note.
    pub(super) fn grid_menu_key(&mut self) -> bool {
        let Some(g) = &mut self.grid.state else { return false };
        if g.grid.phase == Phase::Browse {
            g.grid.back_to_note();
        } else {
            g.grid.browse();
        }
        self.read_grid();
        true
    }

    /// Whether the boxes are browsed (for "/" and Cmd+F to search them).
    pub(super) fn grid_browsing(&self) -> bool {
        self.grid.state.as_ref().is_some_and(|g| g.grid.phase == Phase::Browse)
    }

    /// With ">" typed in the grid's field: the commands, in the palette's
    /// panel over the boxes (the side menu, where they list otherwise, does
    /// not show with the grid).
    pub(super) fn grid_palette(&self) -> Option<kolada_ui::palette::PaletteView<'_>> {
        self.grid.state.as_ref()?;
        let s = self.search.as_ref().filter(|s| s.commands.is_some())?;
        let query = s.query.text().get(1..).unwrap_or("");
        let shown = kolada_ui::palette::SHOWN;
        let first = s.selected.saturating_sub(shown.saturating_sub(1));
        Some(kolada_ui::palette::PaletteView {
            query,
            caret: s.query.selection().head.saturating_sub(1).min(query.len()),
            rows: &s.shown,
            first,
            chosen: s.selected,
            drop: 1.0,
            height: kolada_ui::palette::rows_for(s.shown.len()),
            fill: (s.selected - first) as f32,
        })
    }

    /// Escape while browsing: back to the note.
    pub(super) fn grid_escape(&mut self) -> bool {
        self.grid.state.as_mut().is_some_and(|g| g.grid.phase == Phase::Browse && g.grid.back_to_note())
    }

    /// The wheel over the boxes scrolls them (`dy` DIPs, down positive):
    /// whether it was theirs.
    pub(super) fn grid_wheel(&mut self, dy: f32) -> bool {
        if !matches!(self.hover, Some(Target::GridSheet | Target::GridBox(_))) {
            return false;
        }
        let (w, h) = self.viewport;
        if let Some(g) = &mut self.grid.state
            && g.grid.scroll_by(dy, g.notes.count(), w, h)
        {
            self.read_grid();
        }
        true
    }

    pub(super) fn grid_moving(&self) -> bool {
        self.grid.state.as_ref().is_some_and(|g| g.grid.moving())
    }

    /// The split's spring, `dt` seconds on; boxes coming into sight are read.
    pub(super) fn step_grid(&mut self, dt: f32) {
        if let Some(g) = &mut self.grid.state {
            g.grid.step(dt);
        }
        self.read_grid();
    }

    /// The boxes in sight, for the scene: the first's place, the boxes.
    pub(super) fn grid_boxes(&self, width: f32, height: f32) -> Option<(usize, Vec<BoxView<'_>>)> {
        self.grid.state.as_ref().map(|g| g.notes.boxes(&g.grid, width, height))
    }

    pub(super) fn grid_view<'a>(&'a self, boxes: &'a Option<(usize, Vec<BoxView<'a>>)>) -> Option<GridView<'a>> {
        let g = self.grid.state.as_ref()?;
        let (first, boxes) = boxes.as_ref()?;
        let s = self.search.as_ref();
        let search = s.map(|s| GridSearch { query: s.query.text(), caret: s.focused.then(|| s.query.selection().head) });
        let chosen = s.filter(|_| g.notes.searching() && g.notes.count() > 0).map(|s| s.selected.min(g.notes.count() - 1));
        Some(GridView { grid: &g.grid, count: g.notes.count(), first: *first, boxes, search, chosen })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-app-grid-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Work")).unwrap();
        for (rel, text) in [("Plan.md", "# Plan\n\nShip the beta.\n"), ("Groceries.md", "# Groceries\n\n- Eggs\n- Bread\n"), ("Work/Ideas.md", "# Ideas\n\nA grid of notes.\n")] {
            std::fs::write(dir.join(rel), text).unwrap();
        }
        dir
    }

    /// Until the workers' readings came in (the notes listed, each box in
    /// sight read) and the sheet has settled.
    fn settle(m: &mut Model) {
        for _ in 0..300 {
            m.poll_outside();
            // The sheet rises on its spring.
            m.tick();
            m.frame(2000, 1440, 2.0);
            let read = m.grid_boxes(1000.0, 720.0).is_some_and(|(_, boxes)| !boxes.is_empty() && boxes.iter().all(|b| b.preview.is_some()));
            // And the sheet settled where it goes, every box in sight.
            let still = m.grid.state.as_ref().is_some_and(|g| !g.grid.moving());
            if read && still {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let g = m.grid.state.as_ref().unwrap();
        let boxes = m.grid_boxes(1000.0, 720.0).map(|(f, b)| (f, b.iter().map(|b| (b.name.to_string(), b.preview.is_some())).collect::<Vec<_>>()));
        panic!("the grid's readings did not come in: listed {} phase {:?} boxes {boxes:?} channel {}", g.notes.list.len(), g.grid.phase, m.grid.channel.is_some());
    }

    /// The view icon turns the list into the grid, kept in the session; its
    /// notes and boxes are read on workers and come back through the
    /// vault's wake; the boxes are named for a screen reader; a click opens
    /// one; typing gives the note the window; Esc while browsing goes back.
    #[test]
    fn the_grid_lists_the_notes_as_boxes_and_opens_one() {
        let v = vault("boxes");
        let mut m = Model::new(v.clone(), true);
        m.frame(2000, 1440, 2.0);
        assert!(!m.grid_shown());
        m.start_watching(|| {});
        assert!(m.toggle_view());
        assert!(m.grid_shown() && m.session.view == 1, "kept in the session");
        m.keep_for_access(true);
        settle(&mut m);
        m.frame(2000, 1440, 2.0);
        let boxes: Vec<Target> = m.hits.iter().filter_map(|(_, t)| matches!(t, Target::GridBox(_)).then_some(*t)).collect();
        assert_eq!(boxes.len(), 3, "a box for each note: {:?}", m.hits.iter().map(|(_, t)| t).collect::<Vec<_>>());
        let labels: Vec<String> = m.access_controls().into_iter().filter(|c| matches!(c.target, Target::GridBox(_))).map(|c| c.label).collect();
        assert!(labels.iter().any(|l| l.starts_with("Groceries")), "named by its title: {labels:?}");
        // The view's one button says what a press does: back to the list.
        assert!(m.access_controls().iter().any(|c| c.target == Target::ViewToggle && c.label == "Show as list"), "the view's button named for the list");
        // A click on a box opens its note.
        let (r, t) = *m.hits.iter().find(|(_, t)| matches!(t, Target::GridBox(_))).unwrap();
        let Target::GridBox(i) = t else { unreachable!() };
        let rel = m.grid.state.as_ref().unwrap().notes.note(i).unwrap().to_string();
        assert_eq!(m.grid_click(t), Some(true));
        assert_eq!(m.session.note, rel, "{r:?}");
        // Browsing again, Esc goes back to the note; typing takes the window.
        m.grid.state.as_mut().unwrap().grid.phase = Phase::Browse;
        assert!(m.command("cancelOperation:"));
        assert_ne!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Browse);
        m.insert("More.");
        assert_eq!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Writing);
        // The list again.
        assert!(m.toggle_view());
        assert!(!m.grid_shown() && m.session.view == 0);
        let _ = std::fs::remove_dir_all(v);
    }

    /// Writing in the grid, a pill at the window's foot says where the boxes
    /// come up from: on the window's middle, under the note's text, named
    /// for a screen reader; the pointer over it peeks them, a click on it
    /// brings them up (a tap does, on a touch screen), and once they are up
    /// it is gone under them. The list has none.
    #[test]
    fn the_pill_at_the_foot_peeks_the_boxes_and_a_click_brings_them_up() {
        let v = vault("pill");
        let mut m = Model::new(v.clone(), true);
        m.open("Plan.md");
        m.frame(2000, 1440, 2.0);
        let pill_hit = |m: &Model| m.hits.iter().find(|(_, t)| *t == Target::GridPill).map(|(r, _)| *r);
        assert!(m.panels.pills[2].is_none() && pill_hit(&m).is_none(), "no pill at the foot in the list");
        m.start_watching(|| {});
        assert!(m.toggle_view());
        // Writing: the boxes down, the pill at the foot.
        m.grid.state.as_mut().unwrap().grid = Grid::at(Phase::Writing);
        m.keep_for_access(true);
        m.frame(2000, 1440, 2.0);
        let pill = m.panels.pills[2].expect("the pill at the foot");
        assert!(((pill.left + pill.right) / 2.0 - 500.0).abs() <= 0.5 && pill.bottom <= 720.0 && pill.top >= 700.0, "centred at the foot: {pill:?}");
        assert!(pill.top >= m.ui.column.bottom, "under the note's text: {pill:?}, {:?}", m.ui.column);
        assert!(m.access_controls().iter().any(|c| c.target == Target::GridPill && c.label == "Show the notes"), "named");
        // Over it, the boxes peek.
        let (x, y) = ((pill.left + pill.right) / 2.0, (pill.top + pill.bottom) / 2.0);
        assert_eq!(scene::hit(&m.hits, x, y), Some(Target::GridPill));
        assert!(m.pointer_moved(x, y));
        assert_eq!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Peek, "over the pill, the boxes peek");
        // Clicked, they come up.
        m.pointer_down(x, y, 1, false);
        assert_eq!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Browse, "clicked, the boxes come up");
        // Up, the pill is under them.
        settle(&mut m);
        m.frame(2000, 1440, 2.0);
        assert!(m.panels.pills[2].is_none() && pill_hit(&m).is_none(), "the boxes up over it");
        // Back to the note: the pill again; a tap on it (no hover first, as
        // on a touch screen) brings the boxes up too.
        m.pointer_left();
        assert!(m.grid_escape());
        for _ in 0..300 {
            m.tick();
            m.frame(2000, 1440, 2.0);
            if !m.grid_moving() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let pill = m.panels.pills[2].expect("the pill again");
        m.pointer_down((pill.left + pill.right) / 2.0, pill.top + 1.0, 1, false);
        assert_eq!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Browse, "tapped, the boxes come up");
        let _ = std::fs::remove_dir_all(v);
    }

    /// Whether anything of the side menu's is on screen.
    fn side_menu(m: &Model) -> bool {
        m.hits.iter().any(|(_, t)| matches!(t, Target::SidebarPanel | Target::Row(_) | Target::Result(_) | Target::Recent(_) | Target::Favorite(_)))
    }

    /// Frames until `done`, the index and the workers given time.
    fn until(m: &mut Model, done: impl Fn(&Model) -> bool) {
        for _ in 0..300 {
            m.poll_outside();
            m.tick();
            m.frame(2000, 1440, 2.0);
            if done(m) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("never came");
    }

    /// The grid's bar has the search: typed there, the boxes are its
    /// notes, with the words around the match; Down and Return open the
    /// chosen; Esc clears it, a second Esc goes back to the note; ">" drops
    /// the palette in over the boxes. The side menu shows at no point.
    #[test]
    fn the_grid_searches_in_its_own_field_and_the_side_menu_never_shows() {
        let v = vault("search");
        let mut m = Model::new(v.clone(), true);
        m.frame(2000, 1440, 2.0);
        m.start_watching(|| {});
        m.toggle_view();
        settle(&mut m);
        assert!(!side_menu(&m));
        // Writing; the side menu's key brings the boxes up instead.
        m.grid.state.as_mut().unwrap().grid.phase = Phase::Writing;
        assert!(m.toggle_panel(kolada_ui::scene::Panel::Sidebar));
        assert_eq!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Browse);
        // The pointer at the left edge, a while: nothing comes out.
        m.pointer_moved(2.0, 400.0);
        std::thread::sleep(std::time::Duration::from_millis(400));
        m.tick();
        m.frame(2000, 1440, 2.0);
        assert!(!side_menu(&m), "the left edge in grid view");
        // The grid's field, typed into.
        assert!(m.open_search());
        m.insert("grid");
        until(&mut m, |m| m.grid.state.as_ref().is_some_and(|g| g.notes.searching() && g.notes.count() == 1));
        assert!(!side_menu(&m), "searching in grid view");
        let g = m.grid.state.as_ref().unwrap();
        assert_eq!(g.grid.phase, Phase::Browse);
        let (_, boxes) = m.grid_boxes(1000.0, 720.0).unwrap();
        assert!(boxes[0].snippet.is_some_and(|s| s.contains("grid")), "the words around the match");
        let none = Some((0, Vec::new()));
        let view = m.grid_view(&none).unwrap();
        assert_eq!((view.chosen, view.search.map(|s| s.query)), (Some(0), Some("grid")));
        // Return opens it, and the search is done.
        assert!(m.command("insertNewline:"));
        assert_eq!(m.session.note, "Work/Ideas.md");
        assert!(m.search.is_none() && !m.grid.state.as_ref().unwrap().notes.searching());
        // Esc takes the tint of what the search found in that note, then
        // clears a search; the next goes back to the note.
        m.grid.state.as_mut().unwrap().grid.browse();
        m.open_search();
        m.insert("plan");
        until(&mut m, |m| m.grid.state.as_ref().is_some_and(|g| g.notes.searching()));
        assert!(!m.found_view().is_empty());
        assert!(m.command("cancelOperation:"));
        assert!(m.found.is_none() && m.search.is_some(), "the tint first");
        assert!(m.command("cancelOperation:"));
        assert!(m.search.is_none());
        m.frame(2000, 1440, 2.0);
        assert!(!m.grid.state.as_ref().unwrap().notes.searching(), "the notes again");
        assert_eq!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Browse);
        assert!(m.command("cancelOperation:"));
        assert_eq!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Writing);
        // ">" and the commands drop in over the boxes; the side menu stays in.
        m.grid.state.as_mut().unwrap().grid.browse();
        m.open_search();
        m.insert(">list");
        m.frame(2000, 1440, 2.0);
        assert!(m.hits.iter().any(|(_, t)| matches!(t, Target::PaletteRow(_))), "the palette over the grid");
        assert!(!side_menu(&m), "the palette in grid view");
        // The palette names the view a press goes to.
        assert!(m.commands().iter().any(|c| c.item.label == "Show as list"));
        assert!(!m.commands().iter().any(|c| c.item.label.contains("side menu")), "no side menu to tuck away");
        m.close_search();
        // A new note, a restored session, any size: still no side menu.
        m.new_note();
        for (w, h) in [(720, 640), (1400, 1800), (3840, 2160)] {
            m.frame(w, h, 2.0);
            assert!(!side_menu(&m), "{w}x{h}");
        }
        // (A test model keeps no session on disk: the view is set as one read back.)
        let mut again = Model::new(v.clone(), true);
        again.session.view = 1;
        again.grid_at_launch();
        again.frame(2000, 1440, 2.0);
        assert!(again.grid_shown() && !side_menu(&again), "restored in grid view");
        // Only the view button leaves it.
        assert!(m.toggle_view());
        assert!(!m.grid_shown());
        assert!(m.commands().iter().any(|c| c.item.label == "Show as grid"));
        let _ = std::fs::remove_dir_all(v);
    }

    /// A search's card clicked (not chosen with Return) ends the search:
    /// typing goes to the note, the boxes stay down. The note above the
    /// boxes clicked ends it too; so does the view's button. The trash,
    /// opened from the bar, shows on the grid's panel. Before the index has
    /// answered, a search says nothing of "no notes".
    #[test]
    fn a_search_ends_however_it_is_left_and_the_trash_shows_in_the_grid() {
        let v = vault("leave");
        let mut m = Model::new(v.clone(), true);
        m.frame(2000, 1440, 2.0);
        m.start_watching(|| {});
        m.toggle_view();
        settle(&mut m);
        let search = |m: &mut Model, q: &str| {
            m.grid.state.as_mut().unwrap().grid.browse();
            m.open_search();
            m.insert(q);
            until(m, |m| m.grid.state.as_ref().is_some_and(|g| g.notes.searching() && g.notes.count() > 0));
        };
        // A click on a card.
        search(&mut m, "grid");
        assert_eq!(m.grid_click(Target::GridBox(0)), Some(true));
        assert!(m.search.is_none(), "the search is done");
        m.insert("Typed.");
        m.frame(2000, 1440, 2.0);
        assert!(m.text().contains("Typed."), "the keys are the note's");
        assert_eq!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Writing, "the boxes stay down");
        // The vault changed from outside: while it is read again, the
        // matches stay.
        search(&mut m, "grid");
        let before = m.grid.state.as_ref().unwrap().notes.count();
        *m.index.lock().unwrap() = None;
        m.search.as_mut().unwrap().fresh = false;
        m.frame(2000, 1440, 2.0);
        let g = m.grid.state.as_ref().unwrap();
        assert!(g.notes.searching() && g.notes.count() == before, "the matches stay while the index is read again");
        // (Read again, as `apply_outside` has it.)
        m.index_started = false;
        m.start_index();
        // The note above the boxes.
        assert_eq!(m.grid_click(Target::GridNote), Some(true));
        assert!(m.search.is_none());
        // The view's button, mid-search.
        search(&mut m, "grid");
        assert!(m.toggle_view());
        assert!(m.search.is_none() && !m.grid_shown(), "the list, with no search left typing unseen");
        assert!(m.toggle_view());
        settle(&mut m);
        // Return with the chosen box past the matches left: the last.
        search(&mut m, "e");
        if let Some(s) = &mut m.search {
            s.selected = 99;
        }
        assert_eq!(m.command("insertNewline:"), true);
        assert!(m.search.is_none(), "Return opened a box");
        // The trash, on the grid's panel.
        m.grid.state.as_mut().unwrap().grid.browse();
        assert!(m.open_trash());
        m.frame(2000, 1440, 2.0);
        assert!(m.hits.iter().any(|(_, t)| *t == Target::TrashBack), "the trash shows in grid view");
        assert!(!m.hits.iter().any(|(_, t)| matches!(t, Target::GridBox(_))), "in place of the boxes");
        m.trash = None;
        // Before the index answers: no "no notes".
        let v2 = vault("unread");
        let mut fresh = Model::new(v2.clone(), true);
        fresh.frame(2000, 1440, 2.0);
        fresh.toggle_view();
        fresh.open_search();
        fresh.insert("grid");
        fresh.frame(2000, 1440, 2.0);
        let g = fresh.grid.state.as_ref().unwrap();
        assert!(fresh.index.lock().unwrap().is_some() || !g.notes.searching(), "no matches shown before the index is read");
        let _ = std::fs::remove_dir_all(v);
        let _ = std::fs::remove_dir_all(v2);
    }

    /// While the grid's search or the palette has the keys, no key reaches
    /// the open note: letters (Arabic too), Backspace, Delete, the arrows,
    /// a paste-sized insert; in the grid and in the list. The note's text
    /// is as it was, byte for byte.
    #[test]
    fn keys_for_the_search_or_the_palette_never_reach_the_note() {
        let v = vault("keys");
        let mut m = Model::new(v.clone(), true);
        m.frame(2000, 1440, 2.0);
        m.start_watching(|| {});
        m.open("Plan.md");
        m.frame(2000, 1440, 2.0);
        let before = m.text().to_string();
        let keys = ["deleteBackward:", "deleteForward:", "deleteWordBackward:", "moveLeft:", "moveRight:", "moveUp:", "moveDown:", "moveToBeginningOfLine:", "selectAll:", "deleteToEndOfLine:"];
        let type_all = |m: &mut Model, what: &str| {
            for q in ["harbor", "\u{0645}\u{0635}\u{0628}\u{0627}\u{062D}", ">bold", ">list", "a whole pasted line of words"] {
                m.insert(q);
                m.frame(2000, 1440, 2.0);
                for k in keys {
                    m.command(k);
                }
                assert_eq!(m.text(), before, "{what}: {q:?} reached the note");
            }
        };
        // The grid's own field.
        m.toggle_view();
        settle(&mut m);
        m.open("Plan.md");
        m.grid.state.as_mut().unwrap().grid.browse();
        m.open_search();
        type_all(&mut m, "grid search");
        m.command("cancelOperation:");
        // The palette over the grid.
        m.grid.state.as_mut().unwrap().grid.browse();
        m.open_palette();
        type_all(&mut m, "palette over the grid");
        m.command("cancelOperation:");
        // The list's search and palette.
        m.toggle_view();
        m.open_search();
        type_all(&mut m, "list search");
        m.command("cancelOperation:");
        m.open_palette();
        type_all(&mut m, "list palette");
        assert_eq!(m.session.note, "Plan.md", "the note open all along");
        let _ = std::fs::remove_dir_all(v);
    }

    /// Keys typed as the Mac, Linux and iOS hand them over (`commit_marked`,
    /// nothing composed): "/" while the boxes are browsed opens the grid's
    /// search, not a "/" in the note, and what follows goes into its field;
    /// Cmd+F does the same. Writing, "/" is typed; a composition still
    /// settles as composed.
    #[test]
    fn slash_and_cmd_f_typed_on_a_mac_search_the_grid() {
        let v = vault("slash");
        let mut m = Model::new(v.clone(), true);
        m.frame(2000, 1440, 2.0);
        m.start_watching(|| {});
        m.open("Plan.md");
        m.toggle_view();
        settle(&mut m);
        assert_eq!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Browse);
        let before = m.text().to_string();
        assert!(m.commit_marked("/"));
        assert!(m.search.is_some(), "the grid's search");
        assert_eq!(m.text(), before, "no \"/\" in the note");
        assert!(m.commit_marked("g"));
        assert_eq!(m.search.as_ref().unwrap().query.text(), "g");
        assert!(m.command("cancelOperation:") && m.search.is_none());
        assert!(m.open_find(false) && m.search.is_some(), "Cmd+F while browsing");
        assert!(m.find.is_none());
        m.close_search();
        // Writing: "/" is the note's.
        m.grid.state.as_mut().unwrap().grid.phase = Phase::Writing;
        assert!(m.commit_marked("/"));
        assert!(m.search.is_none() && m.text().contains('/'));
        // A composition settles as before.
        m.set_marked("に", 1);
        assert!(m.commit_marked("日"));
        assert!(m.text().contains('日') && !m.has_marked());
        let _ = std::fs::remove_dir_all(v);
    }

    /// Esc with Beam's menu open puts the menu away, and only that: the
    /// grid stays as it was.
    #[test]
    fn esc_puts_beams_menu_away_before_leaving_the_grid() {
        let v = vault("beam-esc");
        let mut m = Model::new(v.clone(), true);
        m.frame(2000, 1440, 2.0);
        m.toggle_view();
        m.grid.state.as_mut().unwrap().grid.browse();
        m.quick.menu = Some("Plan".into());
        assert!(m.command("cancelOperation:"));
        assert!(m.quick.menu.is_none(), "the menu goes");
        assert_eq!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Browse, "the grid stays");
        assert!(m.command("cancelOperation:"));
        assert_ne!(m.grid.state.as_ref().unwrap().grid.phase, Phase::Browse, "then Esc goes back to the note");
        let _ = std::fs::remove_dir_all(v);
    }
}
