//! The desktop's clipboard (arboard: X11's selections, or Wayland's through
//! XWayland), opened on first use, never before the first frame. A copy
//! writes plain text and HTML; Kolada's own exact Markdown is kept in the
//! app beside them, and a paste of that same text gets it back exactly (the
//! clipboard has no type of Kolada's own here yet).

use kolada_editor::slice::Copied;

#[derive(Default)]
pub struct Clipboard {
    board: Option<arboard::Clipboard>,
    /// The last copy made in Kolada: its plain text and its Markdown.
    ours: Option<(String, String)>,
}

/// What a paste brings: Kolada's own copy, or the clipboard's text and HTML.
#[derive(Debug, PartialEq)]
pub enum Pasted {
    Kolada(String),
    Text { plain: String, html: Option<String> },
    /// A picture and no text, as PNG bytes.
    Picture(Vec<u8>),
}

impl Clipboard {
    fn board(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.board.is_none() {
            self.board = arboard::Clipboard::new().ok();
        }
        self.board.as_mut()
    }

    /// A copy made in Kolada: plain text, HTML when there is some, and the
    /// Markdown remembered for a paste here.
    pub fn copy(&mut self, c: Copied) {
        let wrote = match self.board() {
            Some(b) if !c.html.is_empty() => b.set_html(c.html.as_str(), Some(c.plain.as_str())).is_ok(),
            Some(b) => b.set_text(c.plain.as_str()).is_ok(),
            None => false,
        };
        self.ours = wrote.then_some((c.plain, c.markdown));
    }

    /// Text on the clipboard (a link, a note's words) with no Markdown of ours.
    pub fn set_text(&mut self, text: &str, html: Option<&str>) {
        self.ours = None;
        if let Some(b) = self.board() {
            let _ = match html {
                Some(h) => b.set_html(h, Some(text)),
                None => b.set_text(text),
            };
        }
    }

    /// What a paste brings now.
    pub fn paste(&mut self) -> Option<Pasted> {
        let ours = self.ours.clone();
        let b = self.board()?;
        let Some(plain) = b.get_text().ok().filter(|t| !t.is_empty()) else {
            // No text: a picture, if there is one (as PNG).
            let image = b.get_image().ok()?;
            return super::arrive::png(image.width, image.height, &image.bytes).map(Pasted::Picture);
        };
        Some(match ours {
            Some((p, markdown)) if p == plain => Pasted::Kolada(markdown),
            _ => Pasted::Text { html: b.get().html().ok().filter(|h| !h.is_empty()), plain },
        })
    }

    /// Whether the clipboard holds formatted text (for a menu's "Paste as
    /// Plain Text").
    pub fn has_html(&mut self) -> bool {
        self.board().is_some_and(|b| b.get().html().is_ok_and(|h| !h.is_empty()))
    }

    /// The clipboard's text alone.
    pub fn text(&mut self) -> Option<String> {
        self.board()?.get_text().ok()
    }
}
