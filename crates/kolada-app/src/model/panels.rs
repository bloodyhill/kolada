//! The side menu and the toolbar, as on Windows (`kolada-win` `app.rs`):
//! docked, or tucked away (focus mode) and revealed while the pointer rests
//! near them, their collapsed pills bending toward it like strings; the side
//! menu's width dragged at its edge; and the minimal window, the note alone.
//! Timers are the frame loop's: a reveal or hide waits by the clock.

use std::time::{Duration, Instant};

use kolada_ui::scene::Panel;
use kolada_ui::wave::{self, SAMPLES, Wave};

use super::*;

/// Docking or undocking a panel.
const DOCK_ANIMATION: Duration = Duration::from_millis(180);
/// Revealing or hiding a panel in focus mode: quicker, it is transient.
const REVEAL_ANIMATION: Duration = Duration::from_millis(140);
/// The pointer must rest in a reveal area this long, so sweeping across the
/// window edge on the way elsewhere opens nothing.
const REVEAL_DELAY: Duration = Duration::from_millis(50);
/// A revealed panel lingers this long after the pointer leaves it.
const HIDE_DELAY: Duration = Duration::from_millis(300);
/// Leaving the window this close to its left edge counts as reaching for the
/// side menu, DIPs.
const OVERSHOOT: f32 = 90.0;

const PANELS: [Panel; 2] = [Panel::Sidebar, Panel::Toolbar];

/// A placement's `dpi` on the Mac: its numbers are points, not pixels.
const MAC_POINTS: u32 = 72;

fn index(panel: Panel) -> usize {
    match panel {
        Panel::Sidebar => 0,
        Panel::Toolbar => 1,
    }
}

struct Animation {
    start: Instant,
    duration: Duration,
    from: [f32; 4],
    to: [f32; 4],
}

pub(super) struct Panels {
    /// How much room each panel takes (0 collapsed, 1 docked) and how
    /// visible it is, by `index`; in between while animating.
    pub dock: [f32; 2],
    pub show: [f32; 2],
    animation: Option<Animation>,
    /// Collapsed panels revealed by the pointer resting near them; a panel
    /// collapsed from inside stays hidden until the pointer leaves it.
    revealed: [bool; 2],
    held_back: [bool; 2],
    /// When a reveal or a hide is due.
    reveal_at: [Option<Instant>; 2],
    hide_at: [Option<Instant>; 2],
    /// Too narrow to dock the side menu: it floats when revealed.
    narrow: bool,
    /// The pointer left through the window's left edge, reaching for the
    /// side menu: it stays out until the pointer comes back.
    exited_left: bool,
    /// Where the collapsed pills were in the last frame, and their strings.
    pub pills: [Option<Rect>; 2],
    waves: [Wave; 2],
    pub wave: [[f32; SAMPLES]; 2],
    magnets_moving: bool,
    /// The side menu's edge being dragged: where the pointer took it and the
    /// menu's width then, DIPs.
    grip: Option<(f32, f32)>,
    /// Laid out once from the session (the first frame's width).
    settled: bool,
}

impl Default for Panels {
    fn default() -> Panels {
        Panels {
            dock: [1.0; 2],
            show: [1.0; 2],
            animation: None,
            revealed: [false; 2],
            held_back: [false; 2],
            reveal_at: [None; 2],
            hide_at: [None; 2],
            narrow: false,
            exited_left: false,
            pills: [None; 2],
            waves: [Wave::default(), Wave::default()],
            wave: [[0.0; SAMPLES]; 2],
            magnets_moving: false,
            grip: None,
            settled: false,
        }
    }
}

impl Model {
    pub(super) fn collapsed(&self, panel: Panel) -> bool {
        let s = &self.session;
        match panel {
            Panel::Sidebar => s.sidebar_collapsed || self.panels.narrow || s.minimal || self.compact,
            Panel::Toolbar => s.toolbar_collapsed || s.minimal,
        }
    }

    /// Every panel where its state puts it, at once.
    fn settle_panels(&mut self) {
        self.panels.animation = None;
        for panel in PANELS {
            let i = index(panel);
            let collapsed = self.collapsed(panel);
            self.panels.dock[i] = if collapsed { 0.0 } else { 1.0 };
            self.panels.show[i] = if !collapsed || self.panels.revealed[i] { 1.0 } else { 0.0 };
        }
    }

    /// The window is `width` DIPs wide: past the width the side menu docks
    /// at, it tucks away or comes back, at once.
    pub(super) fn panels_for_width(&mut self, width: f32) {
        let narrow = !chrome::sidebar_fits(width);
        if narrow != self.panels.narrow || !self.panels.settled {
            self.panels.narrow = narrow;
            self.panels.revealed[0] = false;
            self.panels.settled = true;
            self.settle_panels();
        }
    }

    /// Animates every panel toward where its docked or revealed state puts it.
    fn retarget(&mut self, duration: Duration) {
        let mut to = [0.0; 4];
        for panel in PANELS {
            let i = index(panel);
            let collapsed = self.collapsed(panel);
            to[i] = if collapsed { 0.0 } else { 1.0 };
            to[2 + i] = if !collapsed || self.panels.revealed[i] { 1.0 } else { 0.0 };
        }
        let p = &mut self.panels;
        let from = [p.dock[0], p.dock[1], p.show[0], p.show[1]];
        if from == to {
            // Already there: a slide still running elsewhere (a toggle undone
            // before a frame was drawn) would carry it away again.
            p.animation = None;
            return;
        }
        if self.ui.animate {
            p.animation = Some(Animation { start: Instant::now(), duration, from, to });
        } else {
            // At once, and any slide still running stops where it is going.
            p.animation = None;
            (p.dock, p.show) = ([to[0], to[1]], [to[2], to[3]]);
        }
    }

    /// Docks a collapsed panel, or collapses a docked one into focus mode.
    /// Where the side menu cannot dock, it shows or hides, floating.
    /// A finger's tap at (x, y): a press and a lift, and no pointer stays
    /// behind. A side menu revealed for it stays while it is used (a folder
    /// opened in it) and goes once a note is picked or the tap was elsewhere,
    /// as it goes when a pointer moves off it. Whether the screen changes,
    /// and what the tap asks of the OS.
    pub fn tap(&mut self, x: f32, y: f32, clicks: u32) -> (bool, Vec<super::share::Effect>) {
        let note = self.session.note.clone();
        let in_menu = scene::hit(&self.hits, x, y).and_then(Target::panel) == Some(Panel::Sidebar);
        let mut changed = self.pointer_down(x, y, clicks, false);
        let (up, effects) = self.pointer_up_at(x, y);
        changed |= up;
        if !in_menu || self.session.note != note {
            changed |= self.pointer_left();
        }
        (changed, effects)
    }

    /// The side menu out where it is folded away (an iPhone's swipe in
    /// from the left edge: there is no pointer to rest near it). False if
    /// it shows already.
    pub fn reveal_side_menu(&mut self) -> bool {
        let docked = !self.collapsed(Panel::Sidebar) && !self.panels.narrow;
        if docked || self.panels.revealed[0] || self.session.minimal || self.compact {
            return false;
        }
        self.panels.revealed[0] = true;
        self.panels.held_back[0] = false;
        self.retarget(REVEAL_ANIMATION);
        true
    }

    pub fn toggle_panel(&mut self, panel: Panel) -> bool {
        if self.session.minimal || self.compact {
            return false;
        }
        let i = index(panel);
        if panel == Panel::Sidebar && self.panels.narrow {
            self.panels.revealed[0] = !self.panels.revealed[0];
            self.panels.held_back[0] = !self.panels.revealed[0];
            self.retarget(REVEAL_ANIMATION);
            return true;
        }
        let collapsed = !self.collapsed(panel);
        match panel {
            Panel::Sidebar => self.session.sidebar_collapsed = collapsed,
            Panel::Toolbar => self.session.toolbar_collapsed = collapsed,
        }
        self.panels.revealed[i] = false;
        self.panels.held_back[i] = collapsed;
        self.retarget(DOCK_ANIMATION);
        self.persist_session();
        true
    }

    /// Reveals collapsed panels the pointer rests near, and hides revealed
    /// panels it has left: on every pointer move and leave.
    pub(super) fn update_reveal(&mut self) {
        let now = Instant::now();
        for panel in PANELS {
            let i = index(panel);
            let over = self.hover.and_then(Target::panel) == Some(panel);
            if !over {
                self.panels.held_back[i] = false;
            }
            let wants_reveal = self.collapsed(panel) && over && !self.panels.revealed[i] && !self.panels.held_back[i];
            match (wants_reveal, self.panels.reveal_at[i]) {
                (true, None) => self.panels.reveal_at[i] = Some(now + REVEAL_DELAY),
                (false, Some(_)) => self.panels.reveal_at[i] = None,
                _ => {}
            }
            // Searching or choosing options keeps the menu out.
            let busy = panel == Panel::Sidebar && (self.search.is_some() || self.settings.open || self.panels.exited_left);
            let wants_hide = self.panels.revealed[i] && !over && !busy;
            match (wants_hide, self.panels.hide_at[i]) {
                (true, None) => self.panels.hide_at[i] = Some(now + HIDE_DELAY),
                (false, Some(_)) => self.panels.hide_at[i] = None,
                _ => {}
            }
        }
    }

    /// Something about the panels moves or waits on the clock.
    pub(super) fn panels_moving(&self) -> bool {
        let p = &self.panels;
        p.animation.is_some() || (p.magnets_moving && self.ui.animate)
    }

    /// When a panel is next due to reveal or hide.
    pub(super) fn panels_due(&self) -> Option<Instant> {
        let p = &self.panels;
        p.reveal_at.iter().chain(&p.hide_at).flatten().min().copied()
    }

    /// A frame of the panels: due reveals and hides, the dock animation, the
    /// pills' strings. Whether the screen changes.
    pub(super) fn step_panels(&mut self, dt: f32) -> bool {
        let mut changed = false;
        let now = Instant::now();
        for panel in PANELS {
            let i = index(panel);
            let over = self.hover.and_then(Target::panel) == Some(panel);
            if self.panels.reveal_at[i].is_some_and(|t| now >= t) {
                self.panels.reveal_at[i] = None;
                if over && self.collapsed(panel) {
                    self.panels.revealed[i] = true;
                    self.retarget(REVEAL_ANIMATION);
                }
            }
            if self.panels.hide_at[i].is_some_and(|t| now >= t) {
                self.panels.hide_at[i] = None;
                if !over {
                    self.panels.revealed[i] = false;
                    self.retarget(REVEAL_ANIMATION);
                }
            }
        }
        if let Some(a) = &self.panels.animation {
            let t = (a.start.elapsed().as_secs_f32() / a.duration.as_secs_f32()).min(1.0);
            let v: [f32; 4] = std::array::from_fn(|i| a.from[i] + (a.to[i] - a.from[i]) * t);
            self.panels.dock = [v[0], v[1]];
            self.panels.show = [v[2], v[3]];
            if t >= 1.0 {
                self.panels.animation = None;
            }
            changed = true;
        }
        // With less motion asked for, the pills rest flat: no spring to follow.
        if self.panels.magnets_moving && !self.ui.animate {
            self.panels.magnets_moving = false;
            self.panels.wave = [[0.0; SAMPLES]; 2];
            changed = true;
        }
        if self.panels.magnets_moving {
            let mut moving = false;
            for (i, vertical) in [(0, true), (1, false)] {
                let (target, length) = match self.panels.pills[i] {
                    Some(pill) => (wave::target(pill, vertical, self.ui.pointer), if vertical { pill.height() } else { pill.width() }),
                    None => ([0.0; SAMPLES], 100.0),
                };
                moving |= self.panels.waves[i].step(dt, length, &target);
                self.panels.wave[i] = self.panels.waves[i].d;
            }
            self.panels.magnets_moving = moving;
            changed = true;
        }
        changed
    }

    /// The pointer moved to `p` (from `before`): collapsed pills feel it,
    /// and panels reveal or hide.
    pub(super) fn panels_pointer(&mut self, before: Option<(f32, f32)>, p: (f32, f32)) {
        self.panels.exited_left = false;
        let reach = wave::PROXIMITY + 16.0;
        let near = |(x, y): (f32, f32)| (self.collapsed(Panel::Sidebar) && x < reach) || (self.collapsed(Panel::Toolbar) && y < reach);
        if near(p) || before.is_some_and(near) {
            self.panels.magnets_moving = true;
        }
        self.update_reveal();
    }

    /// The pointer left the window: reaching for the side menu past the
    /// window's left edge still means the menu.
    pub(super) fn panels_left(&mut self, last: Option<(f32, f32)>) {
        if self.collapsed(Panel::Sidebar) && last.is_some_and(|(x, _)| x < OVERSHOOT) && !self.compact && !self.session.minimal {
            self.panels.exited_left = true;
            if !self.panels.revealed[0] {
                self.panels.revealed[0] = true;
                self.panels.held_back[0] = false;
                self.retarget(REVEAL_ANIMATION);
            }
        }
        self.panels.magnets_moving = true;
        self.update_reveal();
    }

    /// The side menu's edge pressed at `x` DIPs: dragged, it sets the
    /// menu's width; double-clicked, it goes back to its own.
    pub(super) fn grab_sidebar_edge(&mut self, x: f32, clicks: u32) -> bool {
        if clicks == 2 {
            self.session.sidebar_width = 0;
            self.panels.grip = None;
            self.persist_session();
            return true;
        }
        let width = chrome::menu_width(self.session.sidebar_width as f32, self.viewport.0);
        self.panels.grip = Some((x, width));
        true
    }

    /// The side menu's edge dragged to `x` DIPs. Near its own width it
    /// settles there. None when the edge is not held.
    pub(super) fn drag_sidebar_edge(&mut self, x: f32) -> Option<bool> {
        let (from, width) = self.panels.grip?;
        let wanted = (width + x - from).clamp(chrome::SIDEBAR_LEAST, chrome::SIDEBAR_MOST);
        let fits = chrome::menu_width(wanted, self.viewport.0);
        let set = if (fits - chrome::SIDEBAR_WIDTH).abs() < 6.0 { 0 } else { fits.round() as u16 };
        Some(std::mem::replace(&mut self.session.sidebar_width, set) != set)
    }

    /// The button let go of the side menu's edge.
    pub(super) fn drop_sidebar_edge(&mut self) -> bool {
        if self.panels.grip.take().is_some() {
            self.persist_session();
            return true;
        }
        false
    }

    /// The edge is held (for the scene's dragging look).
    pub(super) fn holding_edge(&self) -> bool {
        self.panels.grip.is_some()
    }

    /// The window minimal (the note alone, small) or full again. The view
    /// moves the window; a note's own window is neither.
    pub fn set_minimal(&mut self, on: bool) -> bool {
        if on == self.session.minimal || self.compact {
            return false;
        }
        self.session.minimal = on;
        self.panels.revealed = [false; 2];
        self.settle_panels();
        self.persist_session();
        self.settings.requests.push(settings::Request::Minimal(on));
        true
    }

    /// Where the window was, as the session keeps it (points, its bottom
    /// left corner on the screen, and its size): the minimal one's while the
    /// window is minimal. None for a session from before (or from Windows).
    pub fn window_place(&self, minimal: bool) -> Option<(f64, f64, f64, f64)> {
        let p = if minimal { &self.session.minimal_window } else { &self.session.window };
        (p.dpi == MAC_POINTS && p.width > 0 && p.height > 0).then(|| (p.x as f64, p.y as f64, p.width as f64, p.height as f64))
    }

    /// The window is at (x, y), `width` x `height` points: kept for the next
    /// launch, as the minimal one's while it is minimal.
    pub fn place_window(&mut self, rect: (f64, f64, f64, f64)) {
        self.place_window_as(self.session.minimal, rect);
    }

    /// The window was at `rect` as the full one (`minimal` false) or the
    /// minimal one: kept for coming back to it, this run or the next.
    pub fn place_window_as(&mut self, minimal: bool, (x, y, width, height): (f64, f64, f64, f64)) {
        if self.compact {
            return;
        }
        let placed = kolada_core::session::Placement { x: x.round() as i32, y: y.round() as i32, width: width.round() as i32, height: height.round() as i32, dpi: MAC_POINTS, maximized: false };
        if minimal {
            self.session.minimal_window = placed;
        } else {
            self.session.window = placed;
        }
    }

    /// The window starts minimal (the session says so).
    pub fn minimal(&self) -> bool {
        self.session.minimal && !self.compact
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(test: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-app-panels-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Note.md"), "# Note\n\nBody text.\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        (m, dir)
    }

    /// Steps the frame loop until nothing moves, drawing as the app would.
    fn settle(m: &mut Model) {
        for _ in 0..100 {
            // Frames while something moves; else a wait for what is due.
            if !m.wants_frames() {
                match m.next_due() {
                    Some(t) => std::thread::sleep(t.saturating_duration_since(std::time::Instant::now())),
                    None => break,
                }
            }
            m.tick();
            m.frame(2000, 1440, 2.0);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn has(m: &Model, t: Target) -> bool {
        m.hits.iter().any(|(_, h)| *h == t)
    }

    #[test]
    fn the_side_menu_tucks_away_and_is_revealed_by_resting_near_the_left_edge() {
        let (mut m, dir) = model("focus");
        assert!(has(&m, Target::Search), "docked: the side menu shows");
        assert!(m.toggle_panel(Panel::Sidebar));
        assert!(m.session.sidebar_collapsed);
        m.frame(2000, 1440, 2.0);
        assert!(!has(&m, Target::Search), "tucked away");
        assert!(has(&m, Target::RevealSidebar), "with a reveal area at the edge");
        // Collapsed from inside, it stays away until the pointer has left it.
        m.pointer_moved(1200.0, 700.0);
        // The pointer rests at the edge: after a moment the menu floats out.
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::RevealSidebar).unwrap();
        m.pointer_moved(r.left + 2.0, (r.top + r.bottom) / 2.0);
        assert!(m.next_due().is_some(), "a reveal is due");
        settle(&mut m);
        assert!(has(&m, Target::Search), "revealed");
        // The pointer leaves for the page: it hides again.
        m.pointer_moved(1200.0, 700.0);
        settle(&mut m);
        assert!(!has(&m, Target::Search), "hidden again");
        // Docked again with the key.
        m.toggle_panel(Panel::Sidebar);
        m.frame(2000, 1440, 2.0);
        assert!(has(&m, Target::Search) && !m.session.sidebar_collapsed);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_toolbar_tucks_away_too_and_a_narrow_window_floats_the_side_menu() {
        let (mut m, dir) = model("narrow");
        m.toggle_panel(Panel::Toolbar);
        m.frame(2000, 1440, 2.0);
        assert!(!has(&m, Target::Tool(Tool::Bold)), "the toolbar is tucked away");
        m.toggle_panel(Panel::Toolbar);
        m.frame(2000, 1440, 2.0);
        assert!(has(&m, Target::Tool(Tool::Bold)));
        // Too narrow to dock: the side menu is away, and the key floats it out.
        m.frame(900, 1200, 2.0);
        assert!(!has(&m, Target::Search));
        assert!(m.toggle_panel(Panel::Sidebar));
        assert!(!m.session.sidebar_collapsed, "the setting is left as it was");
        m.frame(900, 1200, 2.0);
        assert!(has(&m, Target::Search), "floating over the note");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_side_menus_edge_drags_its_width_and_a_double_click_puts_it_back() {
        let (mut m, dir) = model("edge");
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::SidebarEdge).expect("the edge can be held");
        let x = (r.left + r.right) / 2.0;
        assert!(m.pointer_down(x, (r.top + r.bottom) / 2.0, 1, false));
        assert!(m.pointer_dragged(x + 80.0, 300.0));
        m.pointer_up();
        let wide = m.session.sidebar_width;
        assert!(wide as f32 > chrome::SIDEBAR_WIDTH + 60.0, "{wide}");
        m.frame(2000, 1440, 2.0);
        // Dragged far: no wider than it may be.
        m.pointer_down(x + 80.0, 300.0, 1, false);
        m.pointer_dragged(x + 2000.0, 300.0);
        m.pointer_up();
        assert!(m.session.sidebar_width as f32 <= chrome::SIDEBAR_MOST);
        m.frame(2000, 1440, 2.0);
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::SidebarEdge).unwrap();
        m.pointer_down((r.left + r.right) / 2.0, 300.0, 2, false);
        assert_eq!(m.session.sidebar_width, 0, "its own width again");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_windows_place_is_kept_full_and_minimal_apart() {
        let (mut m, dir) = model("place");
        assert_eq!(m.window_place(false), None, "nothing kept yet");
        m.place_window((100.0, 200.0, 1200.0, 800.0));
        m.set_minimal(true);
        m.place_window((1500.0, 900.0, 420.0, 560.0));
        assert_eq!(m.window_place(false), Some((100.0, 200.0, 1200.0, 800.0)));
        // Leaving one for the other keeps where the one left was.
        m.place_window_as(false, (300.0, 250.0, 1000.0, 700.0));
        assert_eq!(m.window_place(false), Some((300.0, 250.0, 1000.0, 700.0)));
        assert_eq!(m.window_place(true), Some((1500.0, 900.0, 420.0, 560.0)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_minimal_window_is_the_note_alone_and_asks_the_window_to_shrink() {
        let (mut m, dir) = model("minimal");
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::Minimal).expect("the minimal button");
        assert!(m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false));
        assert!(m.session.minimal);
        assert_eq!(m.take_requests(), [settings::Request::Minimal(true)]);
        m.frame(900, 1100, 2.0);
        assert!(!has(&m, Target::Search) && !has(&m, Target::Tool(Tool::Bold)), "no panels");
        assert!(!m.toggle_panel(Panel::Sidebar), "and none to toggle");
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::ToMain).expect("a way back");
        m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false);
        assert!(!m.session.minimal);
        assert_eq!(m.take_requests(), [settings::Request::Minimal(false)]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
