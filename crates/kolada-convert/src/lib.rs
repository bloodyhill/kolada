//! Documents into notes: a DOCX, PDF or HTML file read into Markdown, the
//! way Kolada writes notes, with its pictures.
//!
//! Pure Rust with no dependencies: its own zip, DEFLATE, XML, HTML and PDF
//! readers. Nothing here runs at startup; a dropped file is converted on a
//! worker. What it keeps, by format:
//! - DOCX: headings, bold, italic, strikethrough, links, nested lists
//!   (bulleted or numbered, as the document's numbering says), tables,
//!   pictures, line breaks.
//! - HTML: the article (a page's `<main>` or `<article>` when it has one,
//!   never its navigation, scripts or styles), headings, emphasis, links,
//!   nested lists, tables, quotes, code, pictures.
//! - PDF: the text in reading order, paragraphs joined across the lines
//!   they were wrapped into, headings by their size, bold, bulleted and
//!   numbered lists, pages in order. Pictures and tables come out as text.
//!
//! Other note apps' exports (Evernote's `.enex`, Notion's `.zip`) are read
//! into many notes at once: see [`import`].

mod docx;
mod enex;
mod html;
pub mod import;
pub mod inflate;
mod md;
mod notion;
mod pdf;
mod xml;
mod zip;

use std::path::Path;

/// Why a document could not be read.
#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The kinds of document read here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Docx,
    Pdf,
    Html,
}

impl Kind {
    /// The kind a file is, by its extension.
    pub fn of(path: &Path) -> Option<Kind> {
        let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
        match ext.as_str() {
            "docx" => Some(Kind::Docx),
            "pdf" => Some(Kind::Pdf),
            "html" | "htm" | "xhtml" => Some(Kind::Html),
            _ => None,
        }
    }
}

/// A picture from inside a document, to keep with the vault's attachments.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    /// A file name for it (with its extension), as the note embeds it.
    pub name: String,
    pub data: Vec<u8>,
}

/// A document read as a note.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Note {
    /// The title the document gives itself, if it gives one.
    pub title: Option<String>,
    /// The note's Markdown. Pictures are embedded as `![[name]]`.
    pub markdown: String,
    /// Pictures inside the document, by the names the note uses.
    pub pictures: Vec<Picture>,
    /// Pictures the document points to beside it (an HTML page's local
    /// images): the name the note uses, and the path as written.
    pub beside: Vec<(String, String)>,
}

/// Copied HTML as Markdown to paste, with its pictures.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pasted {
    /// Pictures are embedded as `![[name]]`, by the names below: saved
    /// under other names, the embeds change with them.
    pub markdown: String,
    /// Pictures inside what was copied.
    pub pictures: Vec<Picture>,
    /// Pictures from elsewhere: their names, and where they are (`file:`
    /// and a path on this machine, as Word and Outlook keep them for a
    /// while, or a web address to fetch).
    pub fetch: Vec<(String, String)>,
}

/// HTML copied from a page, a document or an email (`base`: the page's
/// address), as Markdown to paste; None when its plain text says as much
/// (see `html::paste`).
pub fn paste_html(html: &str, base: Option<&str>) -> Option<Pasted> {
    html::paste(html, base)
}

/// Reads a document of `kind` from its bytes.
pub fn convert(kind: Kind, bytes: &[u8]) -> Result<Note, Error> {
    match kind {
        Kind::Docx => docx::convert(bytes),
        Kind::Pdf => pdf::convert(bytes),
        Kind::Html => html::convert(bytes),
    }
}

/// A byte as Windows-1252 (and PDF's WinAnsi) has it: Latin-1, but for
/// the punctuation and letters at 0x80 to 0x9F.
pub(crate) fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™',
        'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}
