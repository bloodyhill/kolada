//! Smart paste: text copied with its formatting (a web page, Word, Google
//! Docs, Mail) comes in as Markdown, its headings, lists, links and tables
//! kept; plain text, code, and a table's cells stay plain. Cmd+Shift+V pastes
//! the plain text whatever the pasteboard also holds.

use std::sync::mpsc;
use std::time::Duration;

use super::Model;

/// How long a paste waits for its Markdown before taking the plain text.
const WAIT: Duration = Duration::from_millis(1500);

/// The pasteboard's HTML (`public.html`, as UTF-8; `base` the page it came
/// from when the source says) as Markdown with its pictures, converted on a
/// thread of its own so a page that hangs the converter or panics it costs
/// only the wait: None when the plain text says as much, or when converting
/// fails or takes too long. (Windows runs the same conversion in a child
/// process.) The model saves the pictures (`pictures::save_pasted`).
pub fn markdown_from_html(html: &str, base: Option<&str>) -> Option<kolada_convert::Pasted> {
    let (html, base) = (html.to_string(), base.map(str::to_string));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(kolada_convert::paste_html(&html, base.as_deref()));
    });
    let pasted = rx.recv_timeout(WAIT).ok().flatten()?;
    let pictures = !pasted.pictures.is_empty() || !pasted.fetch.is_empty();
    (!pasted.markdown.trim().is_empty() || pictures).then_some(pasted)
}

impl Model {
    /// Cmd+V: `plain` is the pasteboard's text; `html` its `public.html`, if
    /// it has one (read only when it matters, so a plain paste costs nothing
    /// more). Whether the screen changes.
    pub fn paste(&mut self, plain: &str, html: impl FnOnce() -> Option<(String, Option<String>)>) -> bool {
        self.paste_with(plain, |page, base| markdown_from_html(page, base), html)
    }

    /// `paste` with the converter named, so tests need no thread.
    fn paste_with(
        &mut self,
        plain: &str,
        convert: impl FnOnce(&str, Option<&str>) -> Option<kolada_convert::Pasted>,
        html: impl FnOnce() -> Option<(String, Option<String>)>,
    ) -> bool {
        self.paste_stamped(plain, convert, html, &super::pictures::stamp(&*self.services.calendar))
    }

    /// `paste_with` with the time the pictures are named for given.
    fn paste_stamped(
        &mut self,
        plain: &str,
        convert: impl FnOnce(&str, Option<&str>) -> Option<kolada_convert::Pasted>,
        html: impl FnOnce() -> Option<(String, Option<String>)>,
        stamp: &str,
    ) -> bool {
        // The search box and code, and a table's cells, take text as it is.
        if self.search.is_none() && self.editor.takes_markdown() && !self.settings.read_only && !self.scrubbing() {
            if let Some((page, base)) = html() {
                if let Some(pasted) = convert(&page, base.as_deref()) {
                    // Its pictures are saved with the note's attachments at
                    // once; those on the web are fetched after.
                    let brings = !pasted.pictures.is_empty() || !pasted.fetch.is_empty();
                    let dir = brings.then(|| self.attachment_path());
                    let (markdown, fetch) = super::pictures::save_pasted(pasted, dir.as_deref(), stamp);
                    let changed = self.edit(|e| e.paste_markdown(&markdown));
                    self.pictures.borrow_mut().forget_places();
                    self.fetch_pictures(fetch);
                    return changed;
                }
            }
        }
        self.insert(plain)
    }

    /// Cmd+Shift+V: the text alone.
    pub fn paste_plain(&mut self, plain: &str) -> bool {
        self.insert(plain)
    }

    /// Cmd+C (and Cut, before it deletes): what goes on the pasteboard for
    /// the selection, None with nothing selected. The view writes all three:
    /// `plain` as `public.utf8-plain-text`, `html` as `public.html`, and
    /// `markdown` as Kolada's own type (`app.kolada.markdown`), which
    /// `paste_kolada` takes back exactly. With "Show Markdown" on, the plain
    /// text is the Markdown.
    pub fn copy(&self) -> Option<kolada_editor::slice::Copied> {
        if self.in_field() {
            let text = self.selected_text();
            return (!text.is_empty()).then(|| kolada_editor::slice::Copied { markdown: text.clone(), html: String::new(), plain: text });
        }
        self.active().copied(self.session.show_marks)
    }

    /// Cmd+V when the pasteboard holds Kolada's own type (a copy made in
    /// Kolada): its Markdown, exactly, over the selection. Into code or a
    /// table's cell it goes as typed text.
    pub fn paste_kolada(&mut self, markdown: &str) -> bool {
        if self.search.is_some() || self.in_field() || !self.active().takes_markdown() {
            return self.insert(markdown);
        }
        if self.popup.is_none() {
            self.leave_picture_for_text();
        }
        self.edit(|e| e.paste_markdown(markdown))
    }

    /// Cmd+V of `plain` over selected words: a web address links them
    /// (`[words](address)`). False when the paste is not that; then the
    /// view pastes as ever.
    pub fn paste_address(&mut self, plain: &str) -> bool {
        if self.in_field() || self.search.is_some() || kolada_ui::linkfield::address_over(self.active(), plain).is_none() {
            return false;
        }
        self.edit(|e| {
            kolada_ui::linkfield::paste_address(e, plain);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn model(test: &str, note: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-app-paste-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Named after its first line, as a note written in Kolada is (else
        // it would open under its file's name).
        let stem = kolada_core::note::file_stem_for(&kolada_editor::title_of(note));
        let stem = if stem.is_empty() { "Note".to_string() } else { stem };
        std::fs::write(dir.join(format!("{stem}.md")), note).unwrap();
        let mut m = Model::new(dir.clone(), true);
        // No text: a fresh page, which has no name line over it.
        if note.is_empty() {
            m.new_note();
        }
        m.frame(2000, 1440, 2.0);
        (m, dir)
    }

    fn page() -> Option<(String, Option<String>)> {
        Some(("<h2>Plan</h2><ul><li>one</li><li>two</li></ul>".to_string(), None))
    }

    #[test]
    fn copied_html_pastes_as_markdown_and_plain_text_stays_plain() {
        let (mut m, dir) = model("html", "");
        assert!(m.paste("Plan one two", page));
        let text = m.text().to_string();
        assert!(text.contains("## Plan"), "{text:?}");
        assert!(text.contains("one") && text.lines().filter(|l| l.starts_with("- ") || l.starts_with("* ")).count() == 2, "{text:?}");
        // No HTML on the pasteboard: the text as it is.
        let (mut m2, dir2) = model("plain", "");
        m2.paste("just text", || None);
        assert_eq!(m2.text(), "just text");
        let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(dir2));
    }

    #[test]
    fn pasted_pictures_are_saved_in_the_vault_and_embedded() {
        let (mut m, dir) = model("pictures", "");
        let pasted = || {
            Some(kolada_convert::Pasted {
                markdown: "A\n\n![[one.png]]\n\nB".into(),
                pictures: vec![kolada_convert::Picture { name: "one.png".into(), data: b"\x89PNG\r\n\x1a\nabc".to_vec() }],
                fetch: Vec::new(),
            })
        };
        assert!(m.paste_stamped("A B", |_, _| pasted(), page, "20261005183012"));
        assert!(m.text().contains("![[Pasted image 20261005183012.png]]"), "{:?}", m.text());
        assert_eq!(std::fs::read(dir.join("Pasted image 20261005183012.png")).unwrap(), b"\x89PNG\r\n\x1a\nabc");
        // One undo takes the whole paste back.
        assert!(m.undo());
        assert_eq!(m.text(), "");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pasted_picture_goes_where_the_vault_keeps_attachments() {
        let (mut m, dir) = model("attach", "");
        std::fs::create_dir_all(dir.join(".obsidian")).unwrap();
        std::fs::write(dir.join(".obsidian").join("app.json"), r#"{"attachmentFolderPath":"Media"}"#).unwrap();
        let pasted = kolada_convert::Pasted {
            markdown: "![[one.png]]".into(),
            pictures: vec![kolada_convert::Picture { name: "one.png".into(), data: b"x".to_vec() }],
            fetch: Vec::new(),
        };
        m.paste_stamped("", |_, _| Some(pasted), page, "1");
        assert!(dir.join("Media").join("Pasted image 1.png").is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_screenshot_on_the_pasteboard_is_saved_and_embedded_on_a_line_of_its_own() {
        let (mut m, dir) = model("shot", "Words here");
        m.editor.set_selection(kolada_editor::Selection::caret(3));
        assert!(m.paste_picture_stamped(b"\x89PNG\r\n\x1a\nabc", "20261005183012"));
        assert_eq!(std::fs::read(dir.join("Pasted image 20261005183012.png")).unwrap(), b"\x89PNG\r\n\x1a\nabc");
        assert_eq!(m.text(), "Words here\n![[Pasted image 20261005183012.png]]\n");
        // Another in the same second keeps its own name.
        assert!(m.paste_picture_stamped(b"\x89PNG\r\n\x1a\ndef", "20261005183012"));
        assert!(dir.join("Pasted image 20261005183012 1.png").is_file());
        assert!(m.undo() && m.text().matches("![[").count() == 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn paste_plain_ignores_the_formatting() {
        let (mut m, dir) = model("plainkey", "");
        assert!(m.paste_plain("Plan one two"));
        assert_eq!(m.text(), "Plan one two");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn code_and_a_tables_cells_take_the_plain_text() {
        let (mut m, dir) = model("code", "Code\n```\nx\n```\n");
        let x = m.text().find("x\n").unwrap();
        m.editor.set_selection(kolada_editor::Selection::caret(x + 1));
        assert!(!m.editor.takes_markdown());
        m.paste_with("raw", |_, _| panic!("converted inside code"), || panic!("read the HTML inside code"));
        assert!(m.text().contains("raw"));
        let _ = std::fs::remove_dir_all(dir);

        let (mut t, dir) = model("cell", "| a | b |\n| - | - |\n| 1 | 2 |\n");
        let at = t.text().find('1').unwrap();
        t.editor.set_selection(kolada_editor::Selection::caret(at));
        assert!(!t.editor.takes_markdown());
        t.paste_with("cell text", |_, _| panic!("converted in a cell"), || panic!("read the HTML in a cell"));
        assert!(t.text().contains("cell text"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_converter_that_gives_nothing_falls_back_to_the_plain_text() {
        let (mut m, dir) = model("fallback", "");
        m.paste_with("as typed", |_, _| None, page);
        assert_eq!(m.text(), "as typed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_paste_is_one_undo_step() {
        let (mut m, dir) = model("undo", "");
        m.paste("Plan one two", page);
        assert!(m.undo());
        assert_eq!(m.text(), "");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_copy_carries_plain_text_html_and_markdown_and_pastes_back_exactly() {
        let note = "Plan\n- one **two** [site](https://x.io)\n- [x] done\n";
        let (mut m, dir) = model("copy", note);
        assert!(m.copy().is_none(), "nothing selected, nothing copied");
        let a = m.text().find("ne **").unwrap();
        m.editor.set_selection(kolada_editor::Selection { anchor: a, head: m.text().len() });
        let c = m.copy().unwrap();
        assert_eq!(c.plain, "\u{2022} ne two site (https://x.io)\n\u{2611} done\n", "plain text without marks");
        assert!(c.html.contains("<strong>two</strong>"), "{}", c.html);
        assert_eq!(c.markdown, "- ne **two** [site](https://x.io)\n- [x] done\n");
        let (mut into, dir2) = model("copy-into", "");
        assert!(into.paste_kolada(&c.markdown));
        assert_eq!(into.text(), c.markdown, "Kolada's own type pastes back exactly");
        assert!(into.undo() && into.text().is_empty(), "one undo step");
        let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(dir2));
    }

    #[test]
    fn pasting_an_address_over_words_links_them() {
        let (mut m, dir) = model("address", "Our site is here");
        let a = m.text().find("site").unwrap();
        m.editor.set_selection(kolada_editor::Selection { anchor: a, head: a + 4 });
        assert!(!m.paste_address("not an address"));
        assert!(m.paste_address("https://kolada.app"));
        assert_eq!(m.text(), "Our [site](https://kolada.app) is here");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_converter_thread_answers_with_markdown() {
        let md = markdown_from_html("<h2>Plan</h2><p>one</p>", None).expect("a heading is structure").markdown;
        assert!(md.contains("## Plan"), "{md:?}");
    }
}
