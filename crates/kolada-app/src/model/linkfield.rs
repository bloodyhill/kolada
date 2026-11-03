//! Links without seeing addresses, as on Windows (`kolada-win` `app/link.rs`;
//! the shared behaviour is `kolada_ui::linkfield`): Cmd+K, the toolbar's
//! Link and the selection bar's open a small field under the selection. A
//! note's name (its suggestions under it, Up and Down to pick) or a web
//! address, then Return, links the selected words; on a link it opens on
//! where it goes and what can be done with it: Open (Return), Copy link,
//! Edit (that becomes the field: Return keeps it, Escape goes back) and
//! Remove link. Escape, Cmd+K again or a click elsewhere puts it away. Resting the pointer on a link for 400 ms shows where it goes in a
//! faint card, gone on a key, a click or leaving it.

use std::time::{Duration, Instant};

use kolada_ui::linkfield::{self, LinkField};

use super::*;

/// Notes suggested at most.
const SHOWN: usize = 5;

/// How long the pointer rests on a link before its address shows.
const REST: Duration = Duration::from_millis(linkfield::ADDRESS_DELAY_MS as u64);

pub(super) struct LinkState {
    pub field: LinkField,
    /// Notes whose names fit what is typed: name and folder.
    pub names: Vec<(String, String)>,
}

/// Where the pointer rests on a link: where it leads, where the pointer is,
/// and when the card shows (None once it does).
struct Hover {
    address: String,
    at: (f32, f32),
    due: Option<Instant>,
}

#[derive(Default)]
pub(super) struct Linking {
    pub open: Option<LinkState>,
    hover: Option<Hover>,
}

impl Model {
    /// Opens the link field for the selection (Cmd+K, the toolbar's and the
    /// selection bar's Link); open, Cmd+K puts it away. Whether the screen
    /// changes.
    pub fn open_link_field(&mut self) -> bool {
        if self.linking.open.is_some() {
            return self.close_link_field();
        }
        if self.settings.read_only || self.scrubbing() || self.popup.is_some() {
            return false;
        }
        self.close_find();
        self.search = None;
        self.suggest.now = None;
        self.hide_link_card();
        self.formatting.selbar.dismiss();
        self.linking.open = Some(LinkState { field: LinkField::open(&self.editor), names: Vec::new() });
        self.refresh_link_names();
        true
    }

    /// The link at text offset `at` as the field would open on it (where
    /// it goes, its range), the selection left as it was. None off a link.
    fn field_on(&mut self, at: usize) -> Option<LinkField> {
        let was = self.editor.selection();
        self.editor.set_selection(Selection::caret(at));
        let field = LinkField::open(&self.editor);
        self.editor.set_selection(was);
        field.existing.then_some(field)
    }

    /// The text offset under (x, y) DIPs when a link is there.
    fn link_point(&self, x: f32, y: f32) -> Option<usize> {
        if self.popup.is_some() || self.linking.open.is_some() || kolada_ui::scene::hit(&self.hits, x, y) != Some(Target::Note) || self.scrub_page().is_some() {
            return None;
        }
        let at = self.offset_at(x, y);
        linkfield::link_under(self.editor.text(), self.editor.outline(), Selection::caret(at)).map(|_| at)
    }

    /// A link is under (x, y) DIPs: a phone's long press there gives its
    /// menu at once.
    pub fn on_link(&self, x: f32, y: f32) -> bool {
        self.link_point(x, y).is_some()
    }

    /// A finger's tap on a link opens it, as a tap on a link does on a
    /// phone, unless the caret is already in that link: then it places the
    /// caret, to change its words. None for an ordinary tap.
    pub fn link_tap(&mut self, x: f32, y: f32) -> Option<(bool, Vec<super::share::Effect>)> {
        let at = self.link_point(x, y)?;
        let field = self.field_on(at)?;
        let s = self.editor.selection();
        if s.is_caret() && (field.range.0..=field.range.1).contains(&s.head) {
            return None;
        }
        Some(self.follow_link(at))
    }

    /// The menu's Copy Link: where the link at `at` goes, on the clipboard.
    pub(super) fn link_copy_at(&mut self, at: usize) -> Vec<super::share::Effect> {
        self.field_on(at).map(|f| super::share::Effect::Clipboard(f.address, "Link copied".into())).into_iter().collect()
    }

    /// The menu's Edit Link: the field opens on the link at `at`, editing
    /// where it goes (Return keeps it as one undo step, Escape goes back).
    pub(super) fn link_edit_at(&mut self, at: usize) -> bool {
        if self.settings.read_only || self.field_on(at).is_none() {
            return false;
        }
        self.linking.open = None;
        self.editor.set_selection(Selection::caret(at));
        self.caret_moved(false);
        if !self.open_link_field() {
            return false;
        }
        if let Some(l) = &mut self.linking.open {
            l.field.start_editing();
        }
        self.refresh_link_names();
        true
    }

    /// The menu's Remove Link: the words stay, the link goes, one undo step.
    pub(super) fn link_remove_at(&mut self, at: usize) -> bool {
        if self.settings.read_only {
            return false;
        }
        let Some(field) = self.field_on(at) else { return false };
        self.edit(|e| {
            field.remove(e);
        })
    }

    /// The field goes, the note as it was. Whether it was open.
    pub fn close_link_field(&mut self) -> bool {
        self.linking.open.take().is_some()
    }

    /// The link field is open (it has the keys).
    pub fn linking(&self) -> bool {
        self.linking.open.is_some()
    }

    /// The field as the scene draws it.
    pub(super) fn link_field_view(&self) -> Option<scene::LinkFieldView<'_>> {
        let l = self.linking.open.as_ref()?;
        Some(scene::LinkFieldView { query: l.field.query.text(), caret: l.field.query.selection().head, names: &l.names, picked: l.field.picked, existing: l.field.existing, editing: l.field.editing })
    }

    /// The address card as the scene draws it: what the link leads to, and
    /// where the pointer rested.
    pub(super) fn link_card_view(&self) -> Option<(&str, f32, f32)> {
        self.linking.hover.as_ref().filter(|h| h.due.is_none()).map(|h| (h.address.as_str(), h.at.0, h.at.1))
    }

    /// The notes that fit what is typed (none for a web address, nor while
    /// a link's actions show).
    fn refresh_link_names(&mut self) {
        let Some(typed) = self.linking.open.as_ref().filter(|l| l.field.editing).and_then(|l| l.field.note_typed()).map(str::to_string) else {
            if let Some(l) = &mut self.linking.open {
                l.names.clear();
                l.field.picked = None;
            }
            return;
        };
        let names: Vec<(String, String)> = if typed.is_empty() {
            Vec::new()
        } else {
            self.start_index();
            match self.index.lock().unwrap().as_ref() {
                Some(all) => search::complete(all, &typed, SHOWN)
                    .into_iter()
                    .map(|n| (n.name.clone(), n.path.rsplit_once('/').map_or(String::new(), |(f, _)| f.to_string())))
                    .collect(),
                None => Vec::new(),
            }
        };
        if let Some(l) = &mut self.linking.open {
            l.field.picked = l.field.picked.filter(|&i| i < names.len());
            l.names = names;
        }
    }

    /// An edit of what is typed: a pick made goes, and the names follow.
    /// On a link's actions, typing starts editing where it goes.
    pub(super) fn link_query(&mut self, f: impl FnOnce(&mut Editor)) -> bool {
        let Some(l) = &mut self.linking.open else { return false };
        l.field.start_editing();
        let before = l.field.query.version();
        f(&mut l.field.query);
        if l.field.query.version() != before {
            l.field.picked = None;
            self.refresh_link_names();
        }
        true
    }

    /// Return: on a link's actions, it opens; editing, what is typed is
    /// kept.
    pub(super) fn link_return(&mut self) -> bool {
        if self.linking.open.as_ref().is_some_and(|l| !l.field.editing) {
            return self.link_open();
        }
        self.link_commit()
    }

    /// Escape: editing a link goes back to its actions; else the field goes.
    pub(super) fn link_escape(&mut self) -> bool {
        if let Some(l) = &mut self.linking.open
            && l.field.stop_editing()
        {
            self.refresh_link_names();
            return true;
        }
        self.close_link_field()
    }

    /// Open: the link is followed (a note over the page, a web page in the
    /// browser), and the field goes.
    fn link_open(&mut self) -> bool {
        let Some(l) = self.linking.open.take() else { return false };
        let at = l.field.inside(self.editor.text());
        let (changed, effects) = self.follow_link(at);
        self.menus.effects.extend(effects);
        changed || true
    }

    /// Return, or a suggestion clicked: the link is written, one undo step,
    /// the caret after it.
    pub(super) fn link_commit(&mut self) -> bool {
        let Some(l) = self.linking.open.take() else { return false };
        let to = match l.field.picked.and_then(|i| l.names.get(i)) {
            Some((name, _)) => name.clone(),
            None => l.field.query.text().to_string(),
        };
        if to.trim().is_empty() {
            // Nothing typed: on a link, Return leaves it as it is.
            return true;
        }
        self.edit(|e| {
            l.field.apply(e, &to);
        })
    }

    /// Up and Down through the suggestions; other moves are the field's.
    pub(super) fn link_go(&mut self, motion: Motion, extend: bool) -> bool {
        let Some(l) = &mut self.linking.open else { return false };
        let rows = l.names.len();
        match motion {
            Motion::Up | Motion::Down if rows > 0 => {
                let down = motion == Motion::Down;
                l.field.picked = Some(match (l.field.picked, down) {
                    (None, true) => 0,
                    (None, false) => rows - 1,
                    (Some(i), true) => (i + 1) % rows,
                    (Some(i), false) => (i + rows - 1) % rows,
                });
            }
            Motion::Up | Motion::Down => {}
            m => {
                let q = &mut l.field.query;
                let plan = q.plan(m, extend, &OneLine(q.text()));
                q.apply(plan);
            }
        }
        true
    }

    /// Tab takes the suggestion picked; with none, the field keeps it.
    pub(super) fn link_tab(&mut self) -> bool {
        if self.linking.open.as_ref().is_some_and(|l| l.field.picked.is_some()) {
            return self.link_commit();
        }
        self.linking.open.is_some()
    }

    /// A press while the field is open: its own (Remove, a suggestion), or
    /// elsewhere, which puts it away. None when it is not open.
    pub(super) fn link_field_click(&mut self, hit: Option<Target>) -> Option<bool> {
        self.linking.open.as_ref()?;
        match hit {
            Some(Target::LinkRemove) => {
                let l = self.linking.open.take()?;
                Some(self.edit(|e| {
                    l.field.remove(e);
                }))
            }
            Some(Target::LinkOpen) => Some(self.link_open()),
            Some(Target::LinkCopy) => {
                let l = self.linking.open.take()?;
                self.menus.effects.push(super::share::Effect::Clipboard(l.field.address, "Link copied".into()));
                Some(true)
            }
            Some(Target::LinkEdit) => {
                let started = self.linking.open.as_mut().is_some_and(|l| l.field.start_editing());
                self.refresh_link_names();
                Some(started)
            }
            Some(Target::LinkSave) => Some(self.link_commit()),
            Some(Target::LinkCancel) => Some(self.link_escape()),
            Some(Target::LinkSuggestion(i)) => {
                if let Some(l) = &mut self.linking.open {
                    l.field.picked = Some(i);
                }
                Some(self.link_commit())
            }
            Some(Target::LinkPanel | Target::LinkField) => Some(false),
            // Elsewhere: it goes, and the press does what it does.
            _ => {
                self.linking.open = None;
                None
            }
        }
    }

    /// The pointer moved to `p`, over the note's text or not: resting on a
    /// link starts the wait for its address card; leaving it puts the card
    /// away. Whether the screen changes.
    pub(super) fn link_hover_moved(&mut self, over_note: bool, p: (f32, f32)) -> bool {
        let address = if over_note && self.linking.open.is_none() && self.popup.is_none() && !self.scrubbing() && !self.settings.read_only {
            let at = self.offset_at(p.0, p.1);
            // Past the end of a line that ends in a link is not on it: the
            // offset there stays put as the pointer moves left.
            let line_end = self.editor.line_at(at).end;
            let past = at == line_end && self.offset_at(p.0 - 6.0, p.1) == at;
            if past { None } else { linkfield::address_at(self.editor.text(), at) }
        } else {
            None
        };
        if self.linking.hover.as_ref().map(|h| &h.address) == address.as_ref() {
            return false;
        }
        let shown = self.link_card_view().is_some();
        self.linking.hover = address.map(|address| Hover { address, at: p, due: Some(Instant::now() + REST) });
        shown
    }

    /// When the address card is due.
    pub(super) fn link_card_due(&self) -> Option<Instant> {
        self.linking.hover.as_ref().and_then(|h| h.due)
    }

    /// A frame: a card due shows, where the pointer is now. Whether the
    /// screen changes.
    pub(super) fn step_link_card(&mut self) -> bool {
        let pointer = self.ui.pointer;
        match &mut self.linking.hover {
            Some(h) if h.due.is_some_and(|t| Instant::now() >= t) => {
                h.due = None;
                if let Some(p) = pointer {
                    h.at = p;
                }
                true
            }
            _ => false,
        }
    }

    /// A key, a press or the pointer leaving puts the card away. Whether
    /// it showed.
    pub(super) fn hide_link_card(&mut self) -> bool {
        self.linking.hover.take().is_some_and(|h| h.due.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vault with a note and another to link to, the first open, a frame drawn.
    fn open(text: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-linkfield-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), text).unwrap();
        std::fs::write(dir.join("Roadmap.md"), "# Roadmap\n\nLater.\n").unwrap();
        let mut m = Model::new(dir.clone(), false);
        m.open("Plan.md");
        m.frame(1000, 700, 1.0);
        (m, dir)
    }

    fn select(m: &mut Model, word: &str) {
        let a = m.text().find(word).unwrap();
        m.editor.set_selection(Selection { anchor: a, head: a + word.len() });
    }

    fn wait_for_index(m: &mut Model) {
        m.start_index();
        for _ in 0..400 {
            if m.corpus_ready() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn cmd_k_links_the_selected_words_to_a_web_address_or_a_note() {
        let (mut m, dir) = open("# Plan\n\nRead the guide and the roadmap.\n");
        wait_for_index(&mut m);
        select(&mut m, "guide");
        assert!(m.tool(Tool::Link) && m.linking(), "Cmd+K opens the field, the words left as they are");
        assert_eq!(m.text(), "# Plan\n\nRead the guide and the roadmap.\n");
        m.frame(1000, 700, 1.0);
        assert!(m.hits.iter().any(|(_, t)| *t == Target::LinkField), "drawn under the selection");
        m.insert("example.com/guide");
        assert!(m.newline(false));
        assert!(!m.linking());
        assert_eq!(m.text(), "# Plan\n\nRead the [guide](https://example.com/guide) and the roadmap.\n");
        assert_eq!(m.selection().head, m.text().find(" and").unwrap(), "the caret after the link");
        // A note's name: its suggestions, Down picks, Return links it.
        select(&mut m, "roadmap");
        m.open_link_field();
        m.insert("Road");
        assert_eq!(m.linking.open.as_ref().unwrap().names.first().map(|n| n.0.as_str()), Some("Roadmap"));
        assert!(m.go(Motion::Down, false) && m.newline(false));
        assert_eq!(m.text(), "# Plan\n\nRead the [guide](https://example.com/guide) and the [[Roadmap|roadmap]].\n");
        // One undo step each.
        m.undo();
        assert_eq!(m.text(), "# Plan\n\nRead the [guide](https://example.com/guide) and the roadmap.\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn on_a_link_it_opens_filled_and_remove_keeps_the_words() {
        let (mut m, dir) = open("# Plan\n\nRead the [guide](https://example.com/guide) today.\n");
        let at = m.text().find("guide]").unwrap() + 2;
        m.editor.set_selection(Selection::caret(at));
        m.open_link_field();
        assert_eq!(m.linking.open.as_ref().map(|l| l.field.query.text()), Some("https://example.com/guide"));
        m.frame(1000, 700, 1.0);
        let remove = m.hits.iter().rev().find(|(_, t)| *t == Target::LinkRemove).map(|(r, _)| *r).expect("Remove is offered");
        assert!(m.pointer_down(remove.left + 2.0, remove.top + 2.0, 1, false));
        assert!(!m.linking());
        assert_eq!(m.text(), "# Plan\n\nRead the guide today.\n");
        assert_eq!(m.selected_text(), "guide");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Presses the field's `target` where the last frame drew it.
    fn press(m: &mut Model, target: Target) -> bool {
        m.frame(1000, 700, 1.0);
        let r = m.hits.iter().rev().find(|(_, t)| *t == target).map(|(r, _)| *r).unwrap_or_else(|| panic!("{target:?} is offered"));
        m.pointer_down(r.left + 2.0, r.top + 2.0, 1, false)
    }

    /// On a link, the field opens on what can be done with it: Copy link
    /// puts where it goes on the clipboard, Open follows it, and Edit
    /// makes it the field: Return keeps a change (one undo step), Escape
    /// goes back to the actions. The same for an address written out.
    #[test]
    fn a_links_field_copies_opens_and_edits_where_it_goes() {
        use super::super::share::Effect;
        let text = "# Plan\n\nRead the [guide](https://example.com/guide) and www.example.org today.\n";
        let (mut m, dir) = open(text);
        let on = |m: &mut Model, s: &str| {
            let at = m.text().find(s).unwrap() + 2;
            m.editor.set_selection(Selection::caret(at));
            m.open_link_field();
        };
        // Copy link: where it goes, said quietly; the field goes.
        on(&mut m, "guide]");
        assert!(m.linking.open.as_ref().is_some_and(|l| !l.field.editing), "its actions first");
        m.frame(1000, 700, 1.0);
        assert!(!m.hits.iter().any(|(_, t)| *t == Target::LinkField), "nothing to type in yet");
        assert!(press(&mut m, Target::LinkCopy) && !m.linking());
        assert_eq!(m.take_effects(), [Effect::Clipboard("https://example.com/guide".into(), "Link copied".into())]);
        // Open: the browser, by the shell.
        on(&mut m, "guide]");
        press(&mut m, Target::LinkOpen);
        assert!(!m.linking());
        assert_eq!(m.take_effects(), [Effect::Url("https://example.com/guide".into())]);
        // Return on the actions opens it too.
        on(&mut m, "guide]");
        assert!(m.newline(false) && !m.linking());
        assert_eq!(m.take_effects(), [Effect::Url("https://example.com/guide".into())]);
        // Edit, then Escape: back to the actions, nothing changed.
        on(&mut m, "guide]");
        assert!(press(&mut m, Target::LinkEdit));
        m.insert("example.com/other");
        assert!(m.escape() && m.linking(), "Escape goes back, the field stays");
        assert_eq!(m.linking.open.as_ref().map(|l| (l.field.editing, l.field.query.text())), Some((false, "https://example.com/guide")));
        assert!(m.escape() && !m.linking(), "and again puts it away");
        assert_eq!(m.text(), text);
        // Edit and Return: the words stay, where they go changes; one undo.
        on(&mut m, "guide]");
        press(&mut m, Target::LinkEdit);
        m.insert("example.com/new");
        assert!(m.newline(false) && !m.linking());
        assert_eq!(m.text(), "# Plan\n\nRead the [guide](https://example.com/new) and www.example.org today.\n");
        assert!(m.undo() && m.text() == text, "one undo step");
        // Typing on the actions starts editing; Save keeps it. An address
        // written out stays one.
        on(&mut m, "example.org");
        m.insert("example.org/b");
        assert!(m.linking.open.as_ref().is_some_and(|l| l.field.editing));
        assert!(press(&mut m, Target::LinkSave) && !m.linking());
        assert_eq!(m.text(), "# Plan\n\nRead the [guide](https://example.com/guide) and https://example.org/b today.\n");
        assert!(m.undo() && m.text() == text, "one undo step");
        // Cancel goes back to the actions, as Escape does.
        on(&mut m, "example.org");
        press(&mut m, Target::LinkEdit);
        assert!(press(&mut m, Target::LinkCancel) && m.linking.open.as_ref().is_some_and(|l| !l.field.editing));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_field_has_the_keys_until_escape_or_a_click_elsewhere() {
        let (mut m, dir) = open("# Plan\n\nRead the guide.\n");
        select(&mut m, "guide");
        m.open_link_field();
        m.insert("abc");
        m.backspace(false);
        m.go(Motion::Left, false);
        m.insert("x");
        assert_eq!(m.linking.open.as_ref().map(|l| l.field.query.text()), Some("axb"));
        assert!(!m.undo(), "the page's undo waits: it would move the words being linked");
        assert_eq!(m.text(), "# Plan\n\nRead the guide.\n", "nothing typed reached the note");
        assert!(m.escape() && !m.linking());
        m.open_link_field();
        assert!(m.tool(Tool::Link) && !m.linking(), "Cmd+K again puts it away");
        m.open_link_field();
        m.frame(1000, 700, 1.0);
        let (x, y) = (m.lines[0].x + 2.0, m.lines[0].y + 2.0);
        m.pointer_down(x, y, 1, false);
        assert!(!m.linking(), "a click elsewhere puts it away");
        assert_eq!(m.text(), "# Plan\n\nRead the guide.\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_selection_bars_link_opens_the_field() {
        let (mut m, dir) = open("# Plan\n\nRead the guide.\n");
        select(&mut m, "guide");
        assert!(m.selbar_tool(Tool::Link) && m.linking());
        assert_eq!(m.text(), "# Plan\n\nRead the guide.\n", "no [[ ]] wrapped around the words");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resting_on_a_link_shows_where_it_goes_and_a_key_puts_it_away() {
        let (mut m, dir) = open("# Plan\n\nRead the [guide](https://example.com/guide) today.\n");
        m.frame(1000, 700, 1.0);
        let at = m.text().find("guide]").unwrap() + 2;
        let line = m.lines.iter().find(|l| l.para.start <= at && at <= l.para.end).unwrap();
        let nt = m.note_text();
        let [l, t, r, b] = nt.range(&line.para, at, at + 1).unwrap()[0];
        let (x, y) = (line.x + (l + r) / 2.0, line.y + (t + b) / 2.0);
        m.pointer_moved(x, y);
        assert!(m.link_card_view().is_none(), "not at once");
        let due = m.next_due().expect("the frame loop wakes for it");
        assert!(due <= Instant::now() + REST);
        std::thread::sleep(REST + Duration::from_millis(20));
        assert!(m.tick());
        assert_eq!(m.link_card_view().map(|c| c.0), Some("https://example.com/guide"));
        m.frame(1000, 700, 1.0);
        m.insert("x");
        assert!(m.link_card_view().is_none(), "typing puts it away");
        // Off a link, nothing waits.
        m.pointer_moved(5.0, 5.0);
        assert!(m.link_card_due().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
