//! The notes as a grid of boxes under the note (see `kolada_ui::grid`):
//! the side menu's view choice, kept in the session. The list of notes and
//! each box's preview are read on workers after the first frame, only
//! the boxes in sight; the split's spring runs with the other motion.

use kolada_core::vault::Listed;
use kolada_ui::grid::{self, BoxView, Grid, GridSearch, GridView, Notes, Phase};

use super::*;

/// The grid while it is the view: what is being done, and its notes.
pub(super) struct GridState {
    pub grid: Grid,
    pub notes: Notes,
}

/// What a worker read: the notes (newest first), or boxes.
pub(super) enum GridRead {
    Listed(Vec<Listed>),
    Boxes(Vec<grid::Read>),
}

impl GridState {
    pub fn new(fresh: bool) -> GridState {
        GridState { grid: Grid::new(fresh), notes: Notes::default() }
    }

    /// The boxes in sight, for the scene: the first's place, the boxes.
    pub fn boxes(&self, width: f32, height: f32) -> (usize, Vec<BoxView<'_>>) {
        self.notes.boxes(&self.grid, width, height)
    }

    /// The grid as the scene draws it, with the search typed in its bar.
    pub fn view<'a>(&'a self, first: usize, boxes: &'a [BoxView<'a>], search: Option<&'a SearchState>) -> GridView<'a> {
        let search_view = search.map(|s| GridSearch { query: s.query.text(), caret: s.focused.then(|| s.query.selection().head) });
        let chosen = search.filter(|_| self.notes.searching() && self.notes.count() > 0).map(|s| s.selected.min(self.notes.count() - 1));
        GridView { grid: &self.grid, count: self.notes.count(), first, boxes, search: search_view, chosen }
    }
}

impl App {
    /// The window's size in DIPs.
    fn window_dips(&self) -> (f32, f32) {
        let (w, h, dpi) = window::view(self.hwnd);
        let scale = dpi as f32 / 96.0;
        (w as f32 / scale, h as f32 / scale)
    }

    /// After the first frame: the grid's notes, when it is the view.
    pub(super) fn start_grid(&mut self) {
        if self.grid.is_some() {
            self.list_grid();
        }
    }

    /// The side menu's view icon, or the grid's: the list or the grid. Out
    /// of a note into the grid, the boxes come up under it to choose from.
    pub(super) fn toggle_view(&mut self) {
        // A search in either view's field ends: the other view has its own.
        if self.search.is_some() {
            self.close_search();
        }
        if self.grid.take().is_some() {
            self.session.view = 0;
        } else {
            let fresh = self.session.note.is_empty() && self.editor.text().is_empty();
            let mut g = GridState::new(fresh);
            if !fresh {
                g.grid.phase = Phase::Browse;
            }
            self.grid = Some(g);
            self.session.view = 1;
            self.list_grid();
        }
        self.request_frame();
    }

    /// Lists the vault's notes on a worker.
    pub(super) fn list_grid(&self) {
        if self.grid.is_none() || self.session.vault.is_empty() {
            return;
        }
        let (vault, hwnd) = (PathBuf::from(&self.session.vault), crate::Handoff(self.hwnd));
        std::thread::spawn(move || {
            let hwnd = hwnd;
            post_grid(hwnd.0, GridRead::Listed(vault::every_note(&vault)));
        });
    }

    /// Reads the boxes in sight not read yet, on a worker.
    fn read_grid(&mut self) {
        let (w, h) = self.window_dips();
        let Some(g) = &mut self.grid else { return };
        let wanted = g.notes.wanted(&g.grid, w, h);
        if wanted.is_empty() {
            return;
        }
        let (vault, hwnd) = (PathBuf::from(&self.session.vault), crate::Handoff(self.hwnd));
        std::thread::spawn(move || {
            let hwnd = hwnd;
            post_grid(hwnd.0, GridRead::Boxes(grid::read(&vault, wanted)));
        });
    }

    /// A worker's reading came in.
    pub(super) fn grid_read(&mut self, read: GridRead) {
        let Some(g) = &mut self.grid else { return };
        match read {
            GridRead::Listed(list) => g.notes.listed(list),
            GridRead::Boxes(boxes) => g.notes.read(boxes),
        }
        self.read_grid();
        self.request_frame();
    }

    /// The open note was saved: it is the newest now, its box read again.
    pub(super) fn grid_saved(&mut self) {
        let rel = self.session.note.clone();
        if let Some(g) = &mut self.grid {
            g.notes.saved(&rel);
            self.read_grid();
        }
    }

    /// Typed into the note: it takes the window.
    pub(super) fn grid_typed(&mut self) {
        if self.grid.as_mut().is_some_and(|g| g.grid.typed()) {
            self.request_frame();
        }
    }

    /// A note opened (or a fresh page): writing it, or at rest.
    pub(super) fn grid_opened(&mut self) {
        let fresh = self.session.note.is_empty() && self.editor.text().is_empty();
        if let Some(g) = &mut self.grid
            && if fresh { g.grid.fresh() } else { g.grid.back_to_note() }
        {
            self.request_frame();
        }
    }

    /// The pointer at `y` DIPs, or gone: at the window's foot the boxes peek.
    pub(super) fn grid_pointer(&mut self, y: Option<f32>) {
        let (_, h) = self.window_dips();
        if self.grid.as_mut().is_some_and(|g| g.grid.pointer(y, h)) {
            self.read_grid();
            self.request_frame();
        }
    }

    /// A click on the grid: true if it was the grid's.
    pub(super) fn grid_click(&mut self, target: Target) -> bool {
        let Some(g) = &mut self.grid else { return false };
        match target {
            Target::GridSheet => {
                g.grid.clicked();
            }
            // Peeking, a click brings the boxes up; browsing, it opens one.
            Target::GridBox(_) if g.grid.phase == Phase::Peek => {
                g.grid.clicked();
            }
            Target::GridBox(i) => {
                let Some(rel) = g.notes.note(i).map(str::to_string) else { return true };
                let path = Path::new(&self.session.vault).join(rel);
                // A search's card chosen: the search is done, the note opens
                // on what it found.
                let query = self.grid_query();
                if self.search.is_some() {
                    self.close_search();
                }
                self.open(&path);
                if let Some(q) = query {
                    self.opened_from_search(&q);
                }
            }
            Target::GridNote => {
                g.grid.back_to_note();
                if self.search.is_some() {
                    self.close_search();
                }
            }
            // The pill at the foot: the boxes come up (riding their edge, down).
            Target::GridPill => {
                g.grid.pill_clicked();
            }
            _ => return false,
        }
        self.read_grid();
        self.request_frame();
        true
    }

    /// Escape while browsing: back to the note.
    pub(super) fn grid_escape(&mut self) -> bool {
        let browsing = self.grid.as_mut().is_some_and(|g| g.grid.phase == Phase::Browse && g.grid.back_to_note());
        if browsing {
            self.request_frame();
        }
        browsing
    }

    /// The search typed in the grid's bar came back: its notes are the
    /// boxes, from the top; nothing typed (or the index not read yet: no
    /// "No notes match" before it is), the notes again. The boxes came up
    /// as the field opened (`open_search`); hits that refresh later (the
    /// index read again, its PDFs) never pull them up over the note.
    pub(super) fn grid_found(&mut self) {
        let Some(g) = &mut self.grid else { return };
        let ready = !self.index.is_empty();
        let found = self.search.as_ref().filter(|s| ready && !s.query.text().trim().is_empty()).map(|s| grid::matches(&s.hits));
        g.notes.found(found);
        g.grid.scroll = 0.0;
        self.read_grid();
        self.request_frame();
    }

    /// Up or Down in the grid's search: the box above or below the chosen
    /// one, kept in sight.
    pub(super) fn grid_search_step(&mut self, down: bool) {
        let (w, h) = self.window_dips();
        let (Some(g), Some(s)) = (&mut self.grid, &mut self.search) else { return };
        let count = g.notes.count();
        if !g.notes.searching() || count == 0 {
            return;
        }
        let at = grid::step(Some(s.selected.min(count - 1)), if down { grid::Step::Down } else { grid::Step::Up }, grid::columns(w), count, false);
        s.selected = at.unwrap_or(0);
        g.grid.reveal(s.selected, count, w, h);
        self.read_grid();
        self.request_frame();
    }

    /// Enter in the grid's search: the chosen box's note opens, the search
    /// ends.
    pub(super) fn grid_search_open(&mut self) {
        let Some(g) = &self.grid else { return };
        let i = self.search.as_ref().map_or(0, |s| s.selected);
        let Some(rel) = g.notes.searching().then(|| g.notes.note(i)).flatten().map(str::to_string) else { return };
        let query = self.grid_query();
        self.close_search();
        self.open(&Path::new(&self.session.vault).join(rel));
        if let Some(q) = query {
            self.opened_from_search(&q);
        }
    }

    /// The side menu's key or button while the grid is the view: the grid's
    /// own way, the boxes up to choose from, or back to the note.
    pub(super) fn grid_menu_key(&mut self) {
        if let Some(g) = &mut self.grid {
            if g.grid.phase == Phase::Browse {
                g.grid.back_to_note();
            } else {
                g.grid.browse();
            }
            self.read_grid();
            self.request_frame();
        }
    }

    /// The wheel over the boxes scrolls them.
    pub(super) fn grid_wheel(&mut self, notches: f32) -> bool {
        if !matches!(self.ui.hover, Some(Target::GridSheet | Target::GridBox(_))) {
            return false;
        }
        let (w, h) = self.window_dips();
        if let Some(g) = &mut self.grid
            && g.grid.scroll_by(-notches * WHEEL_STEP * 3.0, g.notes.count(), w, h)
        {
            self.read_grid();
            self.request_frame();
        }
        true
    }

    pub(super) fn grid_moving(&self) -> bool {
        self.grid.as_ref().is_some_and(|g| g.grid.moving())
    }

    /// The split's spring, `dt` seconds on; boxes coming into sight are read.
    pub(super) fn step_grid(&mut self, dt: f32) {
        if let Some(g) = &mut self.grid {
            g.grid.step(dt);
        }
        self.read_grid();
    }
}

fn post_grid(hwnd: HWND, read: GridRead) {
    let read = Box::into_raw(Box::new(read));
    if unsafe { PostMessageW(Some(hwnd), window::WM_APP_GRID, WPARAM(0), LPARAM(read as isize)) }.is_err() {
        drop(unsafe { Box::from_raw(read) });
    }
}
