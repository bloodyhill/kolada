//! The open note as a web page to export (and print to PDF), as on Windows
//! (`kolada-win` `app/share.rs` `write_export`): tables are tables, and the
//! pictures the note shows go inside the page, so it carries them wherever
//! it is sent.

use kolada_markdown::html;

use super::*;

/// Pictures larger than this stay out of an exported page.
const LARGEST_EXPORTED_PICTURE: usize = 25 * 1024 * 1024;

impl Model {
    /// `text`, the open note's Markdown, as a page named `name`, with the
    /// note's pictures inside it.
    pub(super) fn page_of(&self, name: &str, text: &str) -> String {
        let from = match &self.path {
            Some(p) => p.parent().map_or(self.vault.clone(), Path::to_path_buf),
            None => self.vault.clone(),
        };
        let known = std::cell::RefCell::new(None);
        let picture = |embed: &str| {
            let file = kolada_ui::pictures::find(&self.vault, &from, embed, &mut known.borrow_mut())?;
            let bytes = std::fs::read(&file).ok().filter(|b| b.len() <= LARGEST_EXPORTED_PICTURE)?;
            Some(html::data_uri(&file.to_string_lossy(), &bytes))
        };
        html::document(name, &html::to_html_with(text, true, &picture))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::share::Export;

    /// A 1 x 1 PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C,
        0x63, 0xF8, 0xFF, 0xFF, 0x3F, 0, 5, 0xFE, 2, 0xFE, 0xA7, 0x35, 0x81, 0x84, 0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn an_exported_page_shows_a_table_as_a_table_and_carries_the_notes_pictures() {
        let dir = std::env::temp_dir().join(format!("kolada-app-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Notes")).unwrap();
        std::fs::write(dir.join("Notes").join("dot.png"), PNG).unwrap();
        let note = "# Plan\n\n| name | phone |\n| --- | --- |\n| Sam | 555 |\n\n![[dot.png]]\n\n![gone](missing.png)\n";
        std::fs::write(dir.join("Notes").join("Plan.md"), note).unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.open("Notes/Plan.md");
        for kind in [Export::Html, Export::Pdf] {
            let page = m.export_page(kind).unwrap();
            assert!(page.contains("<table") && page.contains("<th") && page.contains("Sam"), "a real table");
            assert!(!page.contains("| Sam |"), "no pipes left");
            assert!(page.contains("src=\"data:image/png;base64,"), "the picture goes inside the page");
            assert!(page.contains("<title>Plan</title>"));
        }
        // Markdown is the note as it is.
        assert_eq!(m.export_page(Export::Markdown).unwrap(), note);
        let _ = std::fs::remove_dir_all(dir);
    }
}
