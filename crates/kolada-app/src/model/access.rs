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

/// A control a screen reader can name and press: where it is (DIPs), what
/// it is called, and what kind it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub rect: Rect,
    pub label: String,
    pub kind: ControlKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    Button,
    /// A button for the choice in force (a setting's value, the line width).
    Selected,
    /// A section's heading, which folds it.
    Heading,
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
        Target::NewNote => "New note",
        Target::Highlights => "Highlights",
        Target::Options => "Settings",
        Target::Trash => "Trash",
        Target::ThemeCycle => "Appearance",
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
        Target::Checkbox(_) | Target::PopupCheckbox(_) => "Task",
        Target::Picture(_) => "Picture",
        Target::PopupOpen => "Open",
        Target::PopupClose => "Close",
        Target::OptionsClose => "Close settings",
        Target::VaultOpen => "Open another folder",
        Target::VaultShow => "Show the folder",
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
        }
    }

    /// The settings' values, in `Setting` order (as the scene shows them).
    pub(super) fn option_values(&self) -> [u8; scene::SETTINGS] {
        let s = &self.session;
        [s.theme, s.text_size, s.line_width, s.font, s.launch, s.related, s.spell, s.tray, s.hotkey, s.window_mode, assist::option(s), s.remind, s.updates, s.show_marks as u8]
    }

    /// The controls the last frame drew, for a screen reader: each named by
    /// the words drawn on it, or by what its icon means, in the order they
    /// were drawn. One covered by another (a menu over the side menu) is
    /// left out. Empty unless `keep_for_access` is on.
    pub fn access_controls(&self) -> Vec<Control> {
        let Some(scene) = &self.kept_scene else { return Vec::new() };
        let texts = &scene.texts[scene.chrome_from.1.min(scene.texts.len())..];
        let options = self.option_values();
        let mut out: Vec<Control> = Vec::new();
        let mut seen: Vec<Target> = Vec::new();
        for &(r, t) in &self.hits {
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
            let on_top = scene::hit(&self.hits, cx, cy);
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
            let named = chrome::button_name(t, self.platform, self.panels.show[1] >= 0.5).map(|(n, _)| n).or_else(|| icon_name(t));
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
                _ => label,
            };
            let kind = match t {
                Target::Choice(setting, v) if options[setting.index()] == v => ControlKind::Selected,
                Target::Width(w) if self.session.line_width == w => ControlKind::Selected,
                Target::FoldFavorites | Target::FoldRecent => ControlKind::Heading,
                Target::FindField | Target::ReplaceField | Target::LinkField => ControlKind::Field,
                Target::Checkbox(at) => ControlKind::Task(self.editor.text().get(at..).is_some_and(|l| l.trim_start().get(2..5).is_some_and(|b| b.eq_ignore_ascii_case("[x]")))),
                _ => ControlKind::Button,
            };
            out.push(Control { rect: r, label, kind });
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
        assert!(!controls.iter().any(|c| c.label == "Other"), "covered by the settings");
        m.keep_for_access(false);
        assert!(m.access_controls().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

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
