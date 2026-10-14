//! A note open over the page, as on Windows (`kolada-win` `app.rs`, the
//! popup): a linked note read, scrolled and edited without leaving the note
//! being written. While it is open, typing, the keys and input methods are
//! its; a click outside it closes it, its button opens it as the page.

use std::time::{Duration, Instant};

use kolada_ui::scene::{PopupDrawn, PopupView};

use super::*;

/// A popup fades in over this long.
const POPUP_FADE: Duration = Duration::from_millis(140);

pub(super) struct Popup {
    /// Vault-relative, with `.md`.
    rel: String,
    path: PathBuf,
    name: String,
    pub editor: Editor,
    format: note::Format,
    saved_version: u64,
    /// The name line shown over a text that does not start with it.
    added: Option<String>,
    /// The title its file is named after.
    named: String,
    /// How far its text is scrolled; the height it opened beside; when.
    scroll: f32,
    anchor: f32,
    opened: Instant,
    /// Where it was dragged to, and where the pointer holds it while it is
    /// dragged (DIPs from its offset).
    offset: (f32, f32),
    held: Option<(f32, f32)>,
    /// Where the last frame put it.
    pub drawn: Option<PopupDrawn>,
    /// Bring its caret into view in the next frame; a selection is being
    /// dragged in it.
    reveal: bool,
    selecting: bool,
    animate: bool,
}

/// The popup's own hit targets.
fn on_popup(t: Option<Target>) -> bool {
    matches!(
        t,
        Some(Target::PopupCard | Target::PopupHeader | Target::PopupText | Target::PopupOpen | Target::PopupClose | Target::PopupCheckbox(_) | Target::PopupScrollbar)
    )
}

impl Model {
    /// Opens the note at `rel` over the page, beside the caret's height. The
    /// page's own note opens as itself.
    pub(super) fn open_over_page(&mut self, rel: &str) -> bool {
        if rel == self.session.note {
            return false;
        }
        self.close_popup();
        self.search = None;
        let path = self.vault.join(rel);
        let Ok((text, format)) = note::load(&path) else { return false };
        let (text, added) = super::opened(&path, text);
        let mut editor = Editor::new(text);
        editor.set_snippets(super::pictures::snippets(&self.services.calendar));
        let at = super::body_start(editor.text(), editor.outline());
        editor.set_selection(Selection::caret(at));
        let anchor = self.caret.map_or(self.ui.column.top + 80.0, |c| c.top);
        self.popup = Some(Popup {
            rel: rel.to_string(),
            name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            path,
            saved_version: editor.version(),
            added,
            named: editor.title(),
            editor,
            format,
            scroll: 0.0,
            anchor,
            opened: Instant::now(),
            offset: (0.0, 0.0),
            held: None,
            drawn: None,
            reveal: false,
            selecting: false,
            animate: self.ui.animate,
        });
        true
    }

    /// `open_over_page`, for a dump.
    pub fn open_note_over(&mut self, rel: &str) -> bool {
        self.open_over_page(rel)
    }

    /// The note open over the page, if one is.
    #[cfg(test)]
    pub fn popup_note(&self) -> Option<&str> {
        self.popup.as_ref().map(|p| p.rel.as_str())
    }

    /// Writes the popup's note if it changed; its file follows its title
    /// once the caret is off that line, or it closes (`flush`).
    pub(super) fn save_popup(&mut self, flush: bool) {
        let Some(p) = &mut self.popup else { return };
        // Left: stray spaces and runs of empty lines go, as the page's do.
        if flush && p.editor.version() != p.saved_version {
            p.editor.tidy();
        }
        if p.editor.version() != p.saved_version && p.editor.marked().is_none() {
            let text = super::file_text(p.editor.text(), &mut p.added).into_owned();
            if note::save(&p.path, &text, p.format).is_ok() {
                p.saved_version = p.editor.version();
            }
        }
        let title = p.editor.title();
        let on_title = {
            let lines = p.editor.outline();
            let head = p.editor.selection().head;
            title_line(p.editor.text(), lines).is_some_and(|t| lines[t].start <= head && head <= lines[t].end)
        };
        if title == p.named || title.is_empty() || !(flush || !on_title) || p.editor.version() != p.saved_version {
            return;
        }
        p.named = title.clone();
        let Some(dir) = p.path.parent() else { return };
        let wanted = note::unique_path(dir, &note::file_stem_for(&title), Some(&p.path));
        if wanted == p.path || std::fs::rename(&p.path, &wanted).is_err() {
            return;
        }
        let stem = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let (old, new) = (stem(&p.path), stem(&wanted));
        if let Some(rel) = vault::relative(&self.vault, &wanted) {
            self.session.renamed(&p.rel, &rel);
            p.rel = rel;
        }
        p.name = new.clone();
        p.path = wanted;
        let to = p.rel.clone();
        // Links to it follow: the page's as edits (undo takes them back),
        // other notes' on a thread.
        let own = kolada_core::links::renamed(self.editor.text(), &old, &new);
        if !own.is_empty() {
            self.editor.replace_ranges(&own);
            self.dirty = true;
        }
        self.links_follow(old, new, to);
        self.refresh_tree();
    }

    pub fn close_popup(&mut self) -> bool {
        self.save_popup(true);
        self.popup.take().is_some()
    }

    /// The popup's note becomes the page.
    fn expand_popup(&mut self) -> bool {
        let Some(rel) = self.popup.as_ref().map(|p| p.rel.clone()) else { return false };
        self.close_popup();
        self.open(&rel);
        true
    }

    /// The popup as the scene draws it.
    pub(super) fn popup_view(&self) -> Option<PopupView<'_>> {
        let p = self.popup.as_ref()?;
        let s = p.editor.selection();
        let alpha = if p.animate { chrome::ease(p.opened.elapsed().as_secs_f32() / POPUP_FADE.as_secs_f32()) } else { 1.0 };
        let over = matches!(self.hover, Some(t) if on_popup(Some(t)));
        Some(PopupView {
            marked: p.editor.marked().map(|(a, b)| ((a, b), self.ime_target.map(|(x, y)| (a + x, (a + y).min(b))))),
            name: &p.name,
            text: p.editor.text(),
            lines: p.editor.outline(),
            selection: (s.anchor, s.head),
            scroll: p.scroll,
            anchor: p.anchor,
            alpha,
            offset: p.offset,
            hover: self.hover,
            scrolling: self.ui.scroll_drag.is_some_and(|d| d.bar == Target::PopupScrollbar),
            bar_alpha: if over { 1.0 } else { 0.0 },
            spelling: None,
            note: p.rel.strip_suffix(".md"),
        })
    }

    /// Still fading in.
    pub(super) fn popup_fading(&self) -> bool {
        self.popup.as_ref().is_some_and(|p| p.animate && p.opened.elapsed() < POPUP_FADE + Duration::from_millis(20))
    }

    /// Where the frame put the popup; its caret kept in its card. True if
    /// it scrolled (the frame is drawn again).
    pub(super) fn popup_drawn(&mut self, drawn: Option<PopupDrawn>) -> bool {
        let Some(p) = &mut self.popup else { return false };
        p.drawn = drawn;
        if !std::mem::take(&mut p.reveal) {
            return false;
        }
        let (Some(d), Some(c)) = (&p.drawn, p.drawn.as_ref().and_then(|d| d.caret)) else { return false };
        let delta = if c.top < d.content.top + 8.0 {
            c.top - d.content.top - 8.0
        } else if c.bottom > d.content.bottom - 24.0 {
            c.bottom - (d.content.bottom - 24.0)
        } else {
            return false;
        };
        let max = (d.height - d.content.height()).max(0.0);
        let scroll = (p.scroll + delta).clamp(0.0, max);
        let moved = scroll != p.scroll;
        p.scroll = scroll;
        moved
    }

    /// The popup's text laid out as the last frame drew it.
    fn popup_text<'a>(&'a self, p: &'a Popup) -> NoteText<'a> {
        let width = p.drawn.as_ref().map_or(452.0, |d| d.content.width());
        NoteText {
            eng: &self.eng,
            fonts: &self.fonts,
            text: p.editor.text(),
            lines: p.editor.outline(),
            width,
            title: title_line(p.editor.text(), p.editor.outline()),
            caret: Some(p.editor.selection().head),
            pictures: None,
            tables: Default::default(),
        }
    }

    /// Runs an editing command in the popup.
    pub(super) fn popup_edit(&mut self, f: impl FnOnce(&mut Editor)) -> bool {
        let Some(p) = &mut self.popup else { return false };
        let before = p.editor.version();
        f(&mut p.editor);
        p.reveal = true;
        if p.editor.version() != before {
            self.ime_target = None;
        }
        true
    }

    /// Moves the popup's caret by a motion, or by `lines` lines.
    pub(super) fn popup_go(&mut self, motion: Option<Motion>, lines: isize, extend: bool) -> bool {
        let Some(p) = &self.popup else { return false };
        let plan = {
            let layout = self.popup_text(p);
            match motion {
                Some(m) => p.editor.plan(m, extend, &layout),
                None => p.editor.plan_lines(lines, extend, &layout),
            }
        };
        if let Some(p) = &mut self.popup {
            p.editor.apply(plan);
            p.reveal = true;
        }
        true
    }

    /// A page's worth of lines in the popup.
    pub(super) fn popup_page(&self) -> isize {
        self.popup.as_ref().and_then(|p| p.drawn.as_ref()).map_or(10, |d| (d.content.height() / 24.0) as isize)
    }

    /// The text offset in the popup under (x, y) DIPs.
    fn popup_offset(&self, x: f32, y: f32) -> Option<usize> {
        let p = self.popup.as_ref()?;
        let d = p.drawn.as_ref()?;
        let line = d.lines.iter().find(|l| y < l.y + l.height).or(d.lines.last())?;
        let ly = (y - line.y).clamp(0.0, (line.height - 0.5).max(0.0));
        self.popup_text(p).offset_at(&line.para, x - line.x, ly).ok()
    }

    /// A click while a note is open over the page: on it, its own; elsewhere
    /// it closes (and the click goes on to what is under it). None when the
    /// click is not the popup's to take.
    pub(super) fn popup_down(&mut self, x: f32, y: f32, clicks: u32, shift: bool) -> Option<bool> {
        self.popup.as_ref()?;
        let target = scene::hit(&self.hits, x, y);
        if !on_popup(target) {
            self.close_popup();
            return None;
        }
        Some(match target {
            Some(Target::PopupClose) => self.close_popup(),
            Some(Target::PopupOpen) => self.expand_popup(),
            Some(Target::PopupCheckbox(at)) => self.popup_edit(|e| {
                e.toggle_task(at);
            }),
            Some(Target::PopupScrollbar) => {
                self.ui.scroll_drag = None;
                self.popup_scroll_to(y);
                if let Some(p) = &mut self.popup {
                    p.held = None;
                }
                self.ui.scroll_drag = Some(super::bars::Drag::new(Target::PopupScrollbar));
                true
            }
            Some(Target::PopupHeader) => {
                if let Some(p) = &mut self.popup {
                    p.held = Some((x - p.offset.0, y - p.offset.1));
                }
                true
            }
            Some(Target::PopupText) => {
                let Some(at) = self.popup_offset(x, y) else { return Some(false) };
                let Some(p) = &mut self.popup else { return Some(false) };
                match clicks {
                    2 => p.editor.select_word_at(at),
                    n if n >= 3 => p.editor.select_line_at(at),
                    _ => {
                        let anchor = if shift { p.editor.selection().anchor } else { at };
                        p.editor.set_selection(Selection { anchor, head: at });
                    }
                }
                p.selecting = clicks == 1;
                true
            }
            _ => true,
        })
    }

    /// The button held and the pointer at (x, y): the popup's selection, its
    /// header (it moves) or its scroll bar follow. None if not the popup's.
    pub(super) fn popup_dragged(&mut self, x: f32, y: f32) -> Option<bool> {
        if self.ui.scroll_drag.is_some_and(|d| d.bar == Target::PopupScrollbar) {
            return Some(self.popup_scroll_to(y));
        }
        let p = self.popup.as_mut()?;
        if let Some((gx, gy)) = p.held {
            p.offset = (x - gx, y - gy);
            return Some(true);
        }
        if p.selecting {
            let at = self.popup_offset(x, y)?;
            let p = self.popup.as_mut()?;
            let s = p.editor.selection();
            if at != s.head {
                p.editor.set_selection(Selection { anchor: s.anchor, head: at });
                p.reveal = true;
                return Some(true);
            }
            return Some(false);
        }
        None
    }

    /// The button let go at (x, y) with the popup held by its header: out of
    /// the window, it becomes a window of its own there. None if it was not held.
    pub(super) fn popup_dropped(&mut self, x: f32, y: f32) -> Option<(bool, Vec<super::share::Effect>)> {
        self.popup.as_ref()?.held?;
        let (w, h) = self.viewport;
        if x >= 0.0 && y >= 0.0 && x < w && y < h {
            return None;
        }
        let path = self.popup.as_ref()?.path.clone();
        self.close_popup();
        Some((true, vec![super::share::Effect::Window(path)]))
    }

    /// The button let go: the popup stops being dragged or selected in.
    pub(super) fn popup_up(&mut self) -> bool {
        match &mut self.popup {
            Some(p) if p.held.is_some() || p.selecting => {
                p.held = None;
                p.selecting = false;
                true
            }
            _ => false,
        }
    }

    /// Scrolls the popup so its scroll bar's thumb is at `y` DIPs.
    fn popup_scroll_to(&mut self, y: f32) -> bool {
        let Some(p) = &mut self.popup else { return false };
        let Some(d) = &p.drawn else { return false };
        let max = (d.height - d.content.height()).max(0.0);
        if max <= 1.0 {
            return false;
        }
        let track = scene::popup_track(d.card, d.content);
        let (_, h) = chrome::thumb(track, p.scroll / max, d.content.height() / d.height);
        let f = ((y - h / 2.0 - track.top) / (track.height() - h).max(1.0)).clamp(0.0, 1.0);
        p.scroll = f * max;
        true
    }

    /// The wheel over the popup scrolls its text. None if it is elsewhere.
    pub(super) fn popup_wheel(&mut self, dy: f32) -> Option<bool> {
        if !on_popup(self.hover) {
            return None;
        }
        let p = self.popup.as_mut()?;
        let d = p.drawn.as_ref()?;
        let max = (d.height - d.content.height()).max(0.0);
        p.scroll = (p.scroll + dy).clamp(0.0, max);
        Some(true)
    }

    /// The popup's editor, while one is open (it has the keys).
    pub(super) fn popup_editor(&self) -> Option<&Editor> {
        self.popup.as_ref().map(|p| &p.editor)
    }

    pub(super) fn popup_editor_mut(&mut self) -> Option<&mut Editor> {
        self.popup.as_mut().map(|p| &mut p.editor)
    }

    /// The popup's caret, in the last frame, for input methods' windows.
    pub(super) fn popup_caret(&self) -> Option<Rect> {
        self.popup.as_ref()?.drawn.as_ref()?.caret
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-app-popup-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Page.md"), "# Page\n\nSee [[Other]] here.\n").unwrap();
        std::fs::write(dir.join("Other.md"), "# Other\n\nIts first line.\nمرحبا\n").unwrap();
        dir
    }

    fn page(dir: &Path) -> Model {
        let mut m = Model::new(dir.to_path_buf(), true);
        m.open("Page.md");
        m.frame(2000, 1440, 2.0);
        m
    }

    #[test]
    fn a_linked_note_opens_over_the_page_and_typing_goes_into_it() {
        let dir = vault("type");
        let mut m = page(&dir);
        assert!(m.open_over_page("Other.md"));
        m.frame(2000, 1440, 2.0);
        assert!(m.popup.as_ref().unwrap().drawn.is_some(), "it is drawn over the page");
        assert!(m.hits.iter().any(|(_, t)| *t == Target::PopupText));
        // The caret starts on the line under its title; typing is the popup's.
        m.insert("Added");
        assert_eq!(m.text(), "# Page\n\nSee [[Other]] here.\n", "the page is untouched");
        m.save();
        assert_eq!(std::fs::read_to_string(dir.join("Other.md")).unwrap(), "# Other\nAdded\nIts first line.\nمرحبا\n");
        // Arrows and undo too.
        m.go(Motion::DocEnd, false);
        m.insert("!");
        assert!(m.undo());
        assert!(m.close_popup());
        assert_eq!(std::fs::read_to_string(dir.join("Other.md")).unwrap(), "# Other\nAdded\nIts first line.\nمرحبا\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_click_outside_closes_it_and_its_button_opens_it_as_the_page() {
        let dir = vault("click");
        let mut m = page(&dir);
        m.open_over_page("Other.md");
        m.frame(2000, 1440, 2.0);
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::PopupOpen).unwrap();
        assert!(m.pointer_down(r.left + 2.0, r.top + 2.0, 1, false));
        assert_eq!(m.session.note, "Other.md", "it is the page now");
        assert!(m.popup.is_none());
        // Open again from the page, then click the page beside it: closed.
        m.open("Page.md");
        m.frame(2000, 1440, 2.0);
        m.open_over_page("Other.md");
        m.frame(2000, 1440, 2.0);
        let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::Note).unwrap();
        m.pointer_down(r.left + 5.0, r.top + 5.0, 1, false);
        assert!(m.popup.is_none());
        assert!(m.escape() || m.popup.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shortcuts_expand_in_it_and_it_is_tidied_as_it_closes() {
        let dir = vault("tidy");
        let mut m = page(&dir);
        m.open_over_page("Other.md");
        m.go(Motion::DocEnd, false);
        for c in ";;alpha ".chars() {
            m.insert(&c.to_string());
        }
        assert!(m.popup.as_ref().unwrap().editor.text().contains('\u{3b1}'), "{:?}", m.popup.as_ref().unwrap().editor.text());
        m.insert("end   \n\n\n\n");
        m.close_popup();
        let saved = std::fs::read_to_string(dir.join("Other.md")).unwrap();
        assert!(!saved.contains("end   ") && !saved.contains("\n\n\n"), "{saved:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dragged_out_of_the_window_by_its_header_it_becomes_a_window_of_its_own() {
        let dir = vault("out");
        let mut m = page(&dir);
        m.open_over_page("Other.md");
        m.frame(2000, 1440, 2.0);
        let (h, _) = *m.hits.iter().find(|(_, t)| *t == Target::PopupHeader).unwrap();
        m.pointer_down(h.left + 20.0, h.top + 10.0, 1, false);
        m.pointer_dragged(1500.0, 300.0);
        let (_, effects) = m.pointer_up_at(1200.0, -40.0);
        assert!(matches!(effects.as_slice(), [super::super::share::Effect::Window(p)] if p.ends_with("Other.md")));
        assert!(m.popup.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn escape_closes_it_and_its_title_names_its_file() {
        let dir = vault("rename");
        let mut m = page(&dir);
        m.open_over_page("Other.md");
        m.frame(2000, 1440, 2.0);
        m.go(Motion::DocStart, false);
        m.go(Motion::LineEnd, false);
        m.insert(" renamed");
        assert!(m.escape());
        assert!(m.popup.is_none());
        assert!(dir.join("Other renamed.md").is_file() && !dir.join("Other.md").exists());
        assert_eq!(m.text(), "# Page\n\nSee [[Other renamed]] here.\n", "the page's link follows");
        let _ = std::fs::remove_dir_all(dir);
    }
}
