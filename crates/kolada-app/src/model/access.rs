//! What a screen reader asks of the note: its text, its lines, what is in
//! view and where a range is drawn, in UTF-16 units as the OSes count them.
//! The Mac's NSAccessibility asks this (VoiceOver); iOS's UIAccessibility and
//! Windows' UI Automation ask the same. Lines are the note's own lines, as
//! typed: VoiceOver reads a wrapped paragraph as one line, as it does in
//! TextEdit with wrapping off.

use super::*;

impl Model {
    /// What a screen reader calls the note: its name.
    pub fn access_label(&self) -> String {
        let note = &self.session.note;
        let name = note.rsplit(['/', '\\']).next().unwrap_or(note);
        name.strip_suffix(".md").unwrap_or(name).to_string()
    }

    /// The text a screen reader reads: the note, or the one open over it.
    pub fn access_text(&self) -> &str {
        self.active().text()
    }

    /// The text's length in UTF-16 units.
    pub fn access_len(&self) -> usize {
        let text = self.access_text();
        utf16_at(text, text.len())
    }

    /// The line (from 0) that holds UTF-16 position `at`.
    pub fn access_line_of(&self, at: usize) -> usize {
        let text = self.access_text();
        let b = byte_at(text, at.min(self.access_len()));
        text.as_bytes()[..b].iter().filter(|&&c| c == b'\n').count()
    }

    /// Line `line`'s range, UTF-16 start and length, its newline included;
    /// None past the last line.
    pub fn access_line_range(&self, line: usize) -> Option<(usize, usize)> {
        let text = self.access_text();
        let mut start = 0;
        for _ in 0..line {
            start += text[start..].find('\n')? + 1;
        }
        let end = text[start..].find('\n').map_or(text.len(), |i| start + i + 1);
        let s = utf16_at(text, start);
        Some((s, utf16_at(text, end) - s))
    }

    /// The character at UTF-16 position `at`, start and length: two units
    /// for one outside the Basic Multilingual Plane (most emoji), so a
    /// screen reader never splits one.
    pub fn access_char_at(&self, at: usize) -> (usize, usize) {
        let text = self.access_text();
        let mut unit = 0;
        for c in text.chars() {
            let n = c.len_utf16();
            if at < unit + n {
                return (unit, n);
            }
            unit += n;
        }
        (unit, 0)
    }

    /// What the window shows of the note, UTF-16 start and length: the
    /// paragraphs the last frame drew.
    pub fn access_visible(&self) -> (usize, usize) {
        if self.popup.is_some() {
            return (0, self.access_len());
        }
        let text = self.editor.text();
        let (Some(first), Some(last)) = (self.lines.first(), self.lines.last()) else { return (0, 0) };
        let s = utf16_at(text, first.para.start.min(text.len()));
        (s, utf16_at(text, last.para.end.min(text.len())) - s)
    }

    /// Where a range is drawn, DIPs from the view's top left: across the
    /// column, over the paragraphs it is in (the caret's box for an empty
    /// one at the caret); None when none of it is in view.
    pub fn access_rect(&self, start: usize, len: usize) -> Option<Rect> {
        if self.popup.is_some() {
            return self.caret_rect();
        }
        let text = self.editor.text();
        let (a, b) = (byte_at(text, start), byte_at(text, start + len));
        if a == b && Some(a) == Some(self.editor.selection().head) {
            if let Some(caret) = self.caret {
                return Some(caret);
            }
        }
        self.lines
            .iter()
            .filter(|l| l.para.start <= b && a <= l.para.end)
            .map(|l| Rect::new(l.x, l.y, l.x + self.column_width, l.y + l.height))
            .reduce(|r, s| Rect::new(r.left.min(s.left), r.top.min(s.top), r.right.max(s.right), r.bottom.max(s.bottom)))
    }
}

impl Model {
    /// Where the caret goes at UTF-16 position `at`, in the last frame
    /// (DIPs): for a text system's own caret, loupe and selection handles.
    /// None where that paragraph is not drawn.
    pub fn caret_box_at(&self, at: usize) -> Option<Rect> {
        let text = self.editor.text();
        let a = byte_at(text, at);
        let nt = self.note_text();
        let l = self.lines.iter().find(|l| l.para.start <= a && a <= l.para.end)?;
        let (x, y, h) = nt.caret(&l.para, a).ok()?;
        // Two DIPs wide, on the position, as the frame draws it.
        Some(Rect::new(l.x + x - 1.0, l.y + y, l.x + x + 1.0, l.y + y + h))
    }

    /// The boxes the text from UTF-16 `start`, `len` long, covers in the
    /// last frame (DIPs): one for each visual line of it that is drawn, top
    /// to bottom.
    pub fn range_boxes(&self, start: usize, len: usize) -> Vec<Rect> {
        let text = self.editor.text();
        let (a, b) = (byte_at(text, start), byte_at(text, start + len));
        let nt = self.note_text();
        self.lines
            .iter()
            .filter(|l| l.para.start < b && a < l.para.end)
            .flat_map(|l| {
                let boxes = nt.range(&l.para, a.max(l.para.start), b.min(l.para.end)).unwrap_or_default();
                boxes.into_iter().map(|[x0, y0, x1, y1]| Rect::new(l.x + x0, l.y + y0, l.x + x1, l.y + y1))
            })
            .collect()
    }
}

/// A control a screen reader can name and press: where it is (DIPs), what
/// it is called, and what kind it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub rect: Rect,
    pub label: String,
    pub kind: ControlKind,
    /// What it is on the screen (a press finds it again by this).
    pub target: Target,
    /// Scrolled out of sight: how far its list scrolls to show it
    /// (`access_scroll_to`); its `rect` is where it shows then.
    pub away: Option<Away>,
}

/// How far a list scrolls to show a control scrolled out of sight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Away {
    /// Settings, scrolled this far (DIPs).
    Settings(f32),
    /// The side menu's rows, from this one.
    Tree(usize),
    /// Beam's clips, scrolled this far (DIPs).
    Clips(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    Button,
    /// A button for the choice in force (a setting's value, the line width).
    Selected,
    /// A section's heading, which folds it.
    Heading,
    /// A folder in the side menu, or a section that folds (Favorites,
    /// Recent): open or closed.
    Folder(bool),
    /// A text field (find, replace, the link).
    Field,
    /// A task's box, ticked or not.
    Task(bool),
}

/// What an icon's button is called, where it shows no words of its own.
fn icon_name(t: Target) -> Option<&'static str> {
    use kolada_ui::share::Hit as S;
    Some(match t {
        Target::ToggleSidebar => "Side menu",
        Target::RevealSidebar => "Show the side menu",
        Target::RevealToolbar => "Show the toolbar",
        Target::Search => "Search",
        Target::SearchClear => "Clear search",
        Target::NewNote => "New note",
        Target::Highlights => "Highlights",
        Target::Options => "Settings",
        Target::Trash => "Trash",
        Target::GridSheet => "Notes",
        Target::GridBox(_) => "Note",
        Target::GridNote => "Back to the note",
        Target::GridPill => "Show the notes",
        Target::ThemeCycle => "Appearance",
        Target::Quick(kolada_ui::quick::Hit::Show | kolada_ui::quick::Hit::Fab) => "Beam",
        Target::Quick(kolada_ui::quick::Hit::BeamNote) => "Beam this note",
        Target::Quick(kolada_ui::quick::Hit::NewBeam) => "New beam",
        Target::TextSize(true) => "Larger text",
        Target::TextSize(false) => "Smaller text",
        Target::LineWidth => "Line width",
        Target::Width(0) => "Standard width",
        Target::Width(1) => "Narrow",
        Target::Width(_) => "Wide",
        Target::RowMenu(_) => "More",
        Target::TrashRestore(_) => "Restore",
        Target::TrashBack => "Back to the notes",
        Target::TrashEmpty => "Empty the trash",
        Target::SendTo => "Send",
        Target::ToMain => "Back to the main window",
        Target::FindField => "Find",
        Target::ReplaceField => "Replace with",
        Target::FindPrev => "Previous match",
        Target::FindNext => "Next match",
        Target::FindClose => "Close find",
        Target::ReplaceOne => "Replace",
        Target::ReplaceAll => "Replace all",
        Target::LinkField => "Link",
        Target::LinkRemove => "Remove the link",
        Target::LinkOpen => "Open the link",
        Target::LinkCopy => "Copy the link",
        Target::LinkEdit => "Edit the link",
        Target::LinkSave => "Save the link",
        Target::LinkCancel => "Cancel",
        Target::Checkbox(_) | Target::PopupCheckbox(_) => "Task",
        Target::Picture(_) => "Picture",
        Target::Source => "Open the page this came from",
        Target::Lightbox => "Put the picture back",
        Target::PopupOpen => "Open",
        Target::PopupClose => "Close",
        Target::OptionsClose => "Close settings",
        Target::VaultOpen => "Open another folder",
        Target::VaultShow => "Show the folder",
        Target::CheckUpdates => "Check for updates now",
        Target::PageDate => "Note history",
        Target::FoldFavorites => "Favorites",
        Target::FoldRecent => "Recent",
        Target::DueDot => "Due",
        Target::AgendaTick(_) => "Done",
        Target::ScrubRestore => "Restore this version",
        Target::ScrubClose => "Close history",
        Target::BringBack(_) => "Bring back this line",
        Target::Update => "Update",
        Target::Minimal => "Minimal window",
        Target::SocialDrawer => "Social",
        Target::SocialClose => "Close",
        Target::Share(S::CopyNew) => "Copy",
        Target::Share(S::Copy(_)) => "Copy the link",
        Target::Share(S::Delete(_)) => "Delete the link",
        _ => return None,
    })
}

/// What a screen reader leaves out: backgrounds, veils, scroll bars, the
/// note (it is read as text), edges dragged with a pointer.
fn unexposed(t: Target) -> bool {
    use kolada_ui::share::Hit as S;
    matches!(
        t,
        Target::Caption
            | Target::Welcome(kolada_ui::welcome::Hit::Page)
            | Target::Minimize
            | Target::Maximize
            | Target::Close
            | Target::SidebarPanel
            | Target::ToolbarPanel
            | Target::ReadingPanel
            | Target::SelBarPanel
            | Target::MenuPanel
            | Target::FindPanel
            | Target::LinkPanel
            | Target::Heading(_)
            | Target::Scrollbar
            | Target::PopupScrollbar
            | Target::TreeScrollbar
            | Target::Note
            | Target::PictureEdge(_)
            | Target::SidebarEdge
            | Target::SocialCard
            | Target::SocialHeader
            | Target::PopupCard
            | Target::PopupHeader
            | Target::PopupText
            | Target::OptionsVeil
            | Target::OptionsPanel
            | Target::AgendaPanel
            | Target::AgendaShade
            | Target::ScrubPanel
            | Target::ScrubTrack
            | Target::Setup(_)
            | Target::Share(S::Shade | S::Card)
    )
}

impl Model {
    /// Keeps each frame's scene for `access_controls` (a screen reader is
    /// on), or stops. Off, nothing is kept and frames cost what they did.
    pub fn keep_for_access(&mut self, on: bool) {
        self.access_kept = on;
        if !on {
            self.kept_scene = None;
            self.quick_forget_kept();
        }
    }

    /// The settings' values, in `Setting` order (as the scene shows them).
    pub(super) fn option_values(&self) -> [u8; scene::SETTINGS] {
        let s = &self.session;
        [s.theme, s.text_size, s.line_width, s.font, s.launch, s.related, s.spell, s.tray, s.hotkey, s.window_mode, assist::option(s), s.remind, s.updates, s.show_marks as u8, self.quick_keys_shown(), s.quick_beam, s.beam, s.motion]
    }

    /// The controls the last frame drew, for a screen reader: each named by
    /// the words drawn on it, or by what its icon means, in the order they
    /// were drawn. One covered by another (a menu over the side menu) is
    /// left out. Empty unless `keep_for_access` is on.
    pub fn access_controls(&self) -> Vec<Control> {
        let Some(scene) = &self.kept_scene else { return Vec::new() };
        self.controls_in(scene, &self.hits)
    }

    /// The controls in a scrolled list that the screen does not show now
    /// (Settings' lower rows, the side menu's rows past its end): each with
    /// how far its list scrolls to show it. Settings are drawn again at
    /// each place, not shown; the side menu's rows are known by name.
    /// Empty unless `keep_for_access` is on.
    pub fn access_away(&mut self) -> Vec<Control> {
        if !self.access_kept {
            return Vec::new();
        }
        let shown: Vec<Target> = self.access_controls().into_iter().map(|c| c.target).collect();
        let mut out: Vec<Control> = Vec::new();
        if self.settings.open && self.settings.scroll_max > 0.0 {
            let (w, h) = ((self.viewport.0 * self.scale).round() as u32, (self.viewport.1 * self.scale).round() as u32);
            let (was, max) = (self.settings.scroll, self.settings.scroll_max);
            let step = (self.viewport.1 / 2.0).max(80.0);
            let mut at = 0.0f32;
            loop {
                self.settings.scroll = at.min(max);
                let built = self.build(w, h, self.scale);
                for mut c in self.controls_in(&built, &built.hits) {
                    if !shown.contains(&c.target) && !out.iter().any(|o| o.target == c.target) {
                        c.away = Some(Away::Settings(self.settings.scroll));
                        out.push(c);
                    }
                }
                if at >= max {
                    break;
                }
                at += step;
            }
            self.settings.scroll = was;
        }
        // The side menu's rows past what it shows, at the place of its first.
        let (top, fit, rows) = (self.ui.tree_top, self.ui.tree_rows, self.session.sidebar.len());
        let first = self.hits.iter().find(|(_, t)| matches!(t, Target::Row(_))).map(|(r, _)| *r);
        if let Some(first) = first.filter(|_| fit > 0 && rows > fit && self.search.is_none() && self.trash.is_none() && !self.settings.open) {
            for (i, row) in self.session.sidebar.iter().enumerate().filter(|(i, _)| *i < top || *i >= top + fit) {
                out.push(Control { rect: first, label: row.name.clone(), kind: self.row_kind(i), target: Target::Row(i), away: Some(Away::Tree(i.min(rows - fit))) });
            }
        }
        out
    }

    /// Scrolls the list a control scrolled out of sight is in so it shows
    /// (`Control::away`): the next frame draws it.
    pub fn access_scroll_to(&mut self, away: Away) -> bool {
        match away {
            Away::Settings(at) => self.settings.scroll = at.clamp(0.0, self.settings.scroll_max),
            Away::Tree(top) => self.ui.tree_top = top.min(self.session.sidebar.len().saturating_sub(self.ui.tree_rows)),
            Away::Clips(at) => self.quick_scroll_to(at),
        }
        true
    }

    /// A side menu row as a screen reader says it: a folder open or
    /// closed, else a button.
    fn row_kind(&self, i: usize) -> ControlKind {
        match self.session.sidebar.get(i).map(|r| &r.kind) {
            Some(kolada_core::session::RowKind::Folder { expanded }) => ControlKind::Folder(*expanded),
            _ => ControlKind::Button,
        }
    }

    /// The controls `hits` holds in `scene`, named (`access_controls`).
    fn controls_in(&self, scene: &scene::Scene, hits: &[(Rect, Target)]) -> Vec<Control> {
        let texts = &scene.texts[scene.chrome_from.1.min(scene.texts.len())..];
        let options = self.option_values();
        let mut out: Vec<Control> = Vec::new();
        let mut seen: Vec<Target> = Vec::new();
        for &(r, t) in hits {
            if unexposed(t) || seen.contains(&t) || r.right - r.left < 1.0 || r.bottom - r.top < 1.0 {
                continue;
            }
            // The side menu's and the toolbar's invisible bands: only their pills are named.
            let r = match t {
                Target::RevealSidebar => match self.panels.pills[0] {
                    Some(p) => p,
                    None => continue,
                },
                Target::RevealToolbar => match self.panels.pills[1] {
                    Some(p) => p,
                    None => continue,
                },
                _ => r,
            };
            let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            let on_top = scene::hit(hits, cx, cy);
            if on_top != Some(t) && !matches!(t, Target::RevealSidebar | Target::RevealToolbar) {
                continue;
            }
            seen.push(t);
            // The words drawn on it, but not its shortcut ("⌘N") or its icons.
            let words: Vec<&str> = texts
                .iter()
                .filter(|x| x.alpha > 0.1 && x.layout.role() != kolada_text::Role::Icon)
                .filter(|x| {
                    let m = x.layout.measure();
                    let (mx, my) = (x.x + m.width.min(x.layout.width()) / 2.0, x.y + m.height / 2.0);
                    r.contains(mx, my)
                })
                .map(|x| x.layout.text().trim())
                .filter(|w| !w.is_empty() && !w.contains('\u{2318}') && !w.starts_with("Ctrl+"))
                .collect();
            // The text size's two buttons are icons: smaller and larger than now.
            let step = match t {
                Target::Choice(scene::Setting::TextSize, v) => Some(if scene::step_text_size(options[scene::Setting::TextSize.index()], true) == Some(v) { "Larger" } else { "Smaller" }),
                _ => None,
            };
            // A switch is named by its row ("Beam"); Other, by what it opens.
            let switch = match t {
                Target::Switch(setting) => chrome::setting_title(setting, self.platform),
                Target::FontOther if words.is_empty() => Some("Other font"),
                _ => None,
            };
            // The view's button says what a press does.
            let view = (t == Target::ViewToggle).then_some(if self.grid_shown() { "Show as list" } else { "Show as grid" });
            let named = chrome::button_name(t, self.platform, self.panels.show[1] >= 0.5).map(|(n, _)| n).or(view).or_else(|| icon_name(t)).or(step).or(switch);
            let label = match (words.is_empty(), named) {
                (false, Some(n)) if matches!(t, Target::NewNote | Target::Search) => n.to_string(),
                (false, _) => words.join(", "),
                (true, Some(n)) => n.to_string(),
                (true, None) => continue,
            };
            // A choice says whose it is ("Theme: Light"); import and export say which.
            let label = match t {
                Target::Choice(setting, _) => match chrome::setting_title(setting, self.platform) {
                    Some(title) => format!("{title}: {label}"),
                    None => label,
                },
                Target::Transfer(kolada_ui::scene::Transfer::Files | kolada_ui::scene::Transfer::Folder | kolada_ui::scene::Transfer::Clipboard | kolada_ui::scene::Transfer::NotesApp) => format!("Import: {label}"),
                Target::Transfer(_) => format!("Export: {label}"),
                // The welcome's tiles say what they bring in, not their mark.
                Target::Welcome(_) => kolada_ui::welcome::name(t).unwrap_or(label),
                _ => label,
            };
            let kind = match t {
                Target::Choice(setting, v) if options[setting.index()] == v => ControlKind::Selected,
                Target::Width(w) if self.session.line_width == w => ControlKind::Selected,
                Target::FoldFavorites => ControlKind::Folder(self.session.folded & kolada_core::session::FOLD_FAVORITES == 0),
                Target::FoldRecent => ControlKind::Folder(self.session.folded & kolada_core::session::FOLD_RECENT == 0),
                Target::Row(i) if self.search.is_none() && self.trash.is_none() => self.row_kind(i),
                Target::FindField | Target::ReplaceField | Target::LinkField | Target::FontField => ControlKind::Field,
                // A switch reads as a box ticked while its feature is on.
                Target::Switch(setting) => ControlKind::Task(scene::switch_on(setting, options[setting.index()])),
                Target::FontOther if options[scene::Setting::Font.index()] == scene::INSTALLED_FONT => ControlKind::Selected,
                Target::Checkbox(at) => ControlKind::Task(self.editor.text().get(at..).is_some_and(|l| l.trim_start().get(2..5).is_some_and(|b| b.eq_ignore_ascii_case("[x]")))),
                _ => ControlKind::Button,
            };
            out.push(Control { rect: r, label, kind, target: t, away: None });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(test: &str, text: &str) -> (PathBuf, Model) {
        let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-access-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), text).unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        (dir, m)
    }

    #[test]
    fn a_screen_reader_names_every_control_drawn_and_leaves_covered_ones_out() {
        let (dir, mut m) = note("controls", "# Plan\n\n- [x] Book the train\n- [ ] Pack\n");
        std::fs::write(dir.join("Other.md"), "# Other\n").unwrap();
        m.refresh();
        assert!(m.access_controls().is_empty(), "nothing kept until a screen reader asks");
        m.keep_for_access(true);
        m.frame(2000, 1440, 2.0);
        let controls = m.access_controls();
        let labels: Vec<&str> = controls.iter().map(|c| c.label.as_str()).collect();
        for want in ["Search", "New note", "Plan", "Other", "Trash", "Settings"] {
            assert!(labels.contains(&want), "{want} in {labels:?}");
        }
        let tasks: Vec<ControlKind> = controls.iter().filter(|c| c.label == "Task").map(|c| c.kind).collect();
        assert_eq!(tasks, [ControlKind::Task(true), ControlKind::Task(false)]);
        assert!(controls.iter().all(|c| !c.label.contains('\u{2318}')), "shortcuts are not read out: {labels:?}");
        // Settings over the page: their choices are named, the one in force selected,
        // and the side menu under the veil is left out.
        m.toggle_options();
        m.frame(2000, 1440, 2.0);
        let controls = m.access_controls();
        assert!(controls.iter().any(|c| c.kind == ControlKind::Selected && c.label == "Theme: System"), "{controls:?}");
        assert!(controls.iter().any(|c| c.label == "Close settings"));
        assert!(controls.iter().any(|c| c.label == "Text size: Larger") && controls.iter().any(|c| c.label == "Text size: Smaller"), "the text size's icon buttons are named");
        assert!(!controls.iter().any(|c| c.label == "Other"), "covered by the settings");
        m.keep_for_access(false);
        assert!(m.access_controls().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A folder in the side menu says whether it is open, and opens on a
    /// press; a section that folds says the same.
    #[test]
    fn a_screen_reader_hears_a_folder_open_or_closed() {
        let (dir, mut m) = note("folders", "# Plan\n");
        std::fs::create_dir_all(dir.join("Work")).unwrap();
        std::fs::write(dir.join("Work").join("Ideas.md"), "# Ideas\n").unwrap();
        m.refresh();
        m.keep_for_access(true);
        m.frame(2000, 1440, 2.0);
        let work = |m: &Model| m.access_controls().into_iter().find(|c| c.label == "Work").expect("the folder");
        let closed = work(&m);
        assert_eq!(closed.kind, ControlKind::Folder(false));
        let (x, y) = ((closed.rect.left + closed.rect.right) / 2.0, (closed.rect.top + closed.rect.bottom) / 2.0);
        m.pointer_down(x, y, 1, false);
        m.pointer_up_at(x, y);
        m.frame(2000, 1440, 2.0);
        assert_eq!(work(&m).kind, ControlKind::Folder(true), "pressed, it opens");
        assert!(m.access_controls().iter().any(|c| c.label == "Ideas"), "and shows its note");
        // Recent, once notes were opened: a section that folds.
        for name in ["B", "C"] {
            std::fs::write(dir.join(format!("{name}.md")), format!("# {name}\n")).unwrap();
        }
        for rel in ["Work/Ideas.md", "B.md", "C.md"] {
            m.open(rel);
        }
        // Recent shows in a vault of many notes.
        m.session.folded = (m.session.folded | kolada_core::session::MANY_NOTES) & !kolada_core::session::HIDE_RECENT;
        m.frame(2000, 1440, 2.0);
        let recent = |m: &Model| m.access_controls().into_iter().find(|c| c.target == Target::FoldRecent).unwrap_or_else(|| panic!("Recent's heading: {:?} recent {:?}", m.access_controls().iter().map(|c| &c.label).collect::<Vec<_>>(), m.session.recent));
        assert_eq!(recent(&m).kind, ControlKind::Folder(true), "open until folded");
        m.session.folded |= kolada_core::session::FOLD_RECENT;
        m.frame(2000, 1440, 2.0);
        assert_eq!(recent(&m).kind, ControlKind::Folder(false));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Controls scrolled out of sight are a screen reader's too: Settings'
    /// lower rows in a short window and the side menu's rows past its end,
    /// each with how far its list scrolls; scrolled there, it is drawn.
    #[test]
    fn a_screen_reader_reaches_controls_scrolled_out_of_sight() {
        let (dir, mut m) = note("away", "# Plan\n");
        for i in 0..60 {
            std::fs::write(dir.join(format!("Note {i:02}.md")), "# x\n").unwrap();
        }
        m.refresh();
        m.keep_for_access(true);
        let (w, h) = (1600, 700);
        m.frame(w, h, 2.0);
        let away = m.access_away();
        let last = m.session.sidebar.last().map(|r| r.name.clone()).unwrap();
        let far = away.iter().find(|c| c.label == last).expect("the side menu's last row, past its end");
        assert!(matches!(far.away, Some(Away::Tree(_))));
        assert!(!m.access_controls().iter().any(|c| c.label == last));
        assert!(m.access_scroll_to(far.away.unwrap()));
        m.frame(w, h, 2.0);
        assert!(m.access_controls().iter().any(|c| c.label == last), "scrolled there, it is drawn");
        // Settings in a window too short for them.
        m.toggle_options();
        m.frame(w, h, 2.0);
        assert!(m.settings.scroll_max > 0.0, "they scroll in this window");
        let away = m.access_away();
        let export = away.iter().find(|c| c.label.starts_with("Export: ")).unwrap_or_else(|| panic!("{:?}", away.iter().map(|c| &c.label).collect::<Vec<_>>()));
        assert!(matches!(export.away, Some(Away::Settings(at)) if at > 0.0));
        assert_eq!(m.settings.scroll, 0.0, "looking does not scroll");
        assert!(!away.iter().any(|c| matches!(c.away, Some(Away::Tree(_)))), "the side menu under the settings is not theirs");
        let target = export.target;
        m.access_scroll_to(export.away.unwrap());
        m.frame(w, h, 2.0);
        assert!(m.access_controls().iter().any(|c| c.target == target), "scrolled there, it is drawn");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A folder in the side menu says whether it is open, and opens on a
    /// press; a section that folds says the same.

    #[test]
    fn a_screen_reader_reads_the_note_by_its_lines_in_utf16() {
        // "é" is one unit, the emoji two: the OS counts UTF-16, as the input methods do.
        let (dir, m) = note("lines", "# Plan\nCaf\u{E9} \u{1F426}\nLast");
        assert_eq!(m.access_label(), "Plan");
        assert_eq!(m.access_len(), 7 + 8 + 4);
        assert_eq!(m.access_line_range(0), Some((0, 7)));
        assert_eq!(m.access_line_range(1), Some((7, 8)), "the newline is the line's, the emoji two units");
        assert_eq!(m.access_line_range(2), Some((15, 4)));
        assert_eq!(m.access_line_range(3), None);
        assert_eq!((m.access_line_of(0), m.access_line_of(7), m.access_line_of(14), m.access_line_of(18), m.access_line_of(99)), (0, 1, 1, 2, 2));
        assert_eq!(m.access_text(), "# Plan\nCaf\u{E9} \u{1F426}\nLast");
        assert_eq!((m.access_char_at(11), m.access_char_at(12), m.access_char_at(13), m.access_char_at(99)), ((11, 1), (12, 2), (12, 2), (19, 0)), "the bird is one character of two units");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_screen_reader_knows_what_is_in_view_and_where_a_line_is_drawn() {
        let long: String = (0..400).map(|i| format!("Line {i}\n")).collect();
        let (dir, m) = note("view", &format!("# Plan\n{long}"));
        let (start, len) = m.access_visible();
        assert_eq!(start, 0, "the top of the note is in view");
        assert!(len > 0 && len < m.access_len(), "and not all of a long note: {len} of {}", m.access_len());
        let (l1, n1) = m.access_line_range(1).unwrap();
        let r = m.access_rect(l1, n1).expect("the first line is drawn");
        assert!(r.top >= 0.0 && r.bottom <= 720.0 && r.right > r.left, "{r:?}");
        let (far, n) = m.access_line_range(390).unwrap();
        assert_eq!(m.access_rect(far, n), None, "a line out of view has no place");
        let caret = m.access_rect(m.selection_utf16().0, 0).expect("the caret is drawn");
        assert!(caret.right - caret.left < 10.0, "the caret's own box: {caret:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
