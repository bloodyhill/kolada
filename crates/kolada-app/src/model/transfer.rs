//! Notes in and out from the settings, as on Windows (`kolada-win`
//! `app/share.rs`): files, a folder or the clipboard imported; the note
//! exported as Markdown, a web page or a PDF; another folder opened as the
//! vault. The view asks where (open and save panels) and calls back here.

use super::arrive::{is_document, is_note_file};
use super::settings::Request;
use super::share::Export;
use super::*;

/// What the view is asked to pick a file or a folder for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Notes and documents to import (Markdown, text, DOCX, PDF, HTML, and
    /// other apps' exports).
    Files,
    /// A folder of notes to copy in.
    Folder,
    /// Another folder to be the vault.
    Vault,
}

impl Model {
    /// An import or export button in the settings.
    pub(super) fn transfer(&mut self, what: scene::Transfer) -> bool {
        self.settings.open = false;
        let request = match what {
            scene::Transfer::Files | scene::Transfer::NotesApp => Request::Pick(Pick::Files),
            scene::Transfer::Folder => Request::Pick(Pick::Folder),
            scene::Transfer::Clipboard => Request::ImportClipboard,
            scene::Transfer::Markdown => Request::Export(Export::Markdown),
            scene::Transfer::Page => Request::Export(Export::Html),
            scene::Transfer::Pdf => Request::Export(Export::Pdf),
        };
        if matches!(request, Request::Export(_)) && self.outgoing().is_none() {
            self.whisper("Nothing to export yet");
            return true;
        }
        self.settings.requests.push(request);
        true
    }

    /// Files chosen to import: other apps' exports and documents are read
    /// into notes on a thread, Markdown and text copied in; the first opens.
    pub fn import_files(&mut self, files: Vec<PathBuf>) -> bool {
        let (documents, rest): (Vec<PathBuf>, Vec<PathBuf>) = files.into_iter().partition(|f| is_document(f));
        let notes: Vec<PathBuf> = rest.into_iter().filter(|f| is_note_file(f)).collect();
        let mut changed = false;
        if !documents.is_empty() {
            self.whisper(if documents.len() == 1 { "Reading it\u{2026}" } else { "Reading the files\u{2026}" });
            self.import_documents(documents);
            changed = true;
        }
        if !notes.is_empty() {
            let count = notes.len();
            changed |= self.import_notes(&notes);
            self.whisper(&if count == 1 { "Imported 1 note".to_string() } else { format!("Imported {count} notes") });
        }
        changed
    }

    /// A folder of notes copied into the vault, in a folder named after it
    /// (its Markdown and text, folders kept); the first note opens.
    pub fn import_folder(&mut self, from: &Path) -> bool {
        if from.starts_with(&self.vault) {
            self.whisper("That folder is in this vault already");
            return true;
        }
        let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".to_string());
        let mut top = self.vault.join(&name);
        let mut n = 2;
        while top.exists() {
            top = self.vault.join(format!("{name} {n}"));
            n += 1;
        }
        let (mut count, mut first) = (0, None);
        let mut todo = vec![(from.to_path_buf(), top)];
        while let Some((src, dst)) = todo.pop() {
            let Ok(entries) = std::fs::read_dir(&src) else { continue };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let file_name = e.file_name().to_string_lossy().into_owned();
                if file_name.starts_with('.') {
                    continue;
                }
                let path = e.path();
                if path.is_dir() {
                    todo.push((path, dst.join(&file_name)));
                } else if is_note_file(&path) {
                    let _ = std::fs::create_dir_all(&dst);
                    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let to = note::unique_path(&dst, &stem, None);
                    if std::fs::copy(&path, &to).is_ok() {
                        count += 1;
                        first.get_or_insert(to);
                    }
                }
            }
        }
        if count == 0 {
            self.whisper("That folder has no notes");
            return true;
        }
        if let Some(rel) = first.as_deref().and_then(|p| vault::relative(&self.vault, p)) {
            self.open_note(&rel);
        }
        self.refresh_tree();
        *self.index.lock().unwrap() = None;
        self.index_started = false;
        self.whisper(&if count == 1 { "Imported 1 note".to_string() } else { format!("Imported {count} notes") });
        true
    }

    /// The clipboard's text as a new note, named after its first line.
    pub fn import_clipboard(&mut self, text: Option<String>) -> bool {
        let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
            self.whisper("The clipboard has no text");
            return true;
        };
        self.new_note();
        self.insert(&text);
        self.save();
        true
    }

    /// Another folder becomes the vault: its newest note opens, or a fresh
    /// page. The view starts following its files again.
    pub fn switch_vault(&mut self, dir: PathBuf) -> bool {
        if dir == self.vault || !dir.is_dir() {
            return false;
        }
        self.save_flush();
        self.close_popup();
        self.search = None;
        self.trash = None;
        self.find = None;
        self.settings.open = false;
        self.settings.read_only = false;
        self.vault = dir;
        self.session.recent.clear();
        self.session.vault = self.vault.to_string_lossy().into_owned();
        self.session.sidebar = vault::top_level_rows(&self.vault).unwrap_or_default();
        self.ui.tree_top = 0;
        *self.index.lock().unwrap() = None;
        self.index_started = false;
        self.outside = None;
        match crate::launch::newest(&self.vault).and_then(|p| vault::relative(&self.vault, &p)) {
            Some(rel) => self.open(&rel),
            None => {
                self.session.start_fresh();
                self.show(String::new(), String::new(), None);
            }
        }
        self.persist_session();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kolada-app-transfer-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_settings_import_and_export_buttons_ask_the_view_where() {
        let v = dir("buttons");
        std::fs::write(v.join("Note.md"), "# Note\n\nText\n").unwrap();
        let mut m = Model::new(v.clone(), true);
        // Tall enough for every row of the settings.
        m.frame(2000, 2800, 2.0);
        for (what, asked) in [
            (scene::Transfer::Files, Request::Pick(Pick::Files)),
            (scene::Transfer::Folder, Request::Pick(Pick::Folder)),
            (scene::Transfer::Clipboard, Request::ImportClipboard),
            (scene::Transfer::Pdf, Request::Export(Export::Pdf)),
        ] {
            m.toggle_options();
            m.frame(2000, 2800, 2.0);
            let (r, _) = *m.hits.iter().find(|(_, t)| *t == Target::Transfer(what)).unwrap_or_else(|| panic!("{what:?} is offered"));
            m.pointer_down(r.left + 4.0, r.top + 4.0, 1, false);
            assert_eq!(m.take_requests(), [asked]);
            assert!(!m.settings.open, "the settings close");
        }
        let _ = std::fs::remove_dir_all(v);
    }

    #[test]
    fn files_a_folder_and_the_clipboard_come_in_as_notes() {
        let (v, from) = (dir("in"), dir("in-from"));
        std::fs::write(v.join("Note.md"), "# Note\n").unwrap();
        std::fs::create_dir_all(from.join("Sub")).unwrap();
        std::fs::write(from.join("Ideas.md"), "# Ideas\n").unwrap();
        std::fs::write(from.join("Sub").join("ملاحظة.txt"), "مرحبا\n").unwrap();
        std::fs::write(from.join("skip.png"), b"x").unwrap();
        let mut m = Model::new(v.clone(), true);
        assert!(m.import_files(vec![from.join("Ideas.md"), from.join("skip.png")]));
        assert!(v.join("Ideas.md").is_file() && !v.join("skip.png").exists());
        assert_eq!(m.session.note, "Ideas.md");
        assert!(m.import_folder(&from));
        let name = from.file_name().unwrap().to_string_lossy().into_owned();
        assert!(v.join(&name).join("Sub").join("ملاحظة.md").is_file() || v.join(&name).join("Sub").join("ملاحظة.txt").is_file());
        assert!(m.import_clipboard(Some("From the clipboard\nmore".into())));
        assert!(v.join("From the clipboard.md").is_file());
        assert!(m.import_clipboard(None));
        assert_eq!(m.whispered(), Some("The clipboard has no text"));
        let _ = (std::fs::remove_dir_all(v), std::fs::remove_dir_all(from));
    }

    #[test]
    fn another_folder_becomes_the_vault_and_its_newest_note_opens() {
        let (a, b) = (dir("vault-a"), dir("vault-b"));
        std::fs::write(a.join("A.md"), "# A\n").unwrap();
        std::fs::write(b.join("B.md"), "# B\n\nIn the other vault.\n").unwrap();
        let mut m = Model::new(a.clone(), true);
        assert_eq!(m.session.note, "A.md");
        assert!(m.switch_vault(b.clone()));
        assert_eq!(m.vault(), b.as_path());
        assert_eq!(m.session.note, "B.md");
        assert!(m.session.sidebar.iter().any(|r| r.name == "B") && !m.session.sidebar.iter().any(|r| r.name == "A"));
        let _ = (std::fs::remove_dir_all(a), std::fs::remove_dir_all(b));
    }
}
