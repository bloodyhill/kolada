//! Quick actions from the title bar: notes coming in (import), the open note
//! going out (a file, a web page, a PDF, rich text), and the note handed to
//! an AI app installed here, as the start of a new chat. Also a note's own
//! window: opening one, and taking its note back to the main window.

use std::process::Command;

use kolada_markdown::html;
use windows::Win32::System::Registry::{HKEY, HKEY_CLASSES_ROOT, KEY_READ, RegCloseKey, RegOpenKeyExW};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, FindWindowW, GetWindowRect, SW_SHOWNORMAL};
use windows::core::{HSTRING, PCWSTR, w};

use super::menu::Action;
use super::*;
use crate::dialog;
use crate::scene::MenuItem;

/// An AI app to start a chat in, found installed on this machine (or, with
/// none, on the web).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Agent {
    /// The Claude app: a new chat with the note in its prompt box.
    Claude,
    /// ChatGPT, in the browser (its app takes no text through a link).
    ChatGpt,
    /// Command-line agents, in a terminal in the note's folder.
    Codex,
    Gemini,
    /// On the web, when nothing is installed.
    Perplexity,
}

/// What a link can carry, in characters of the note: the apps' own limits,
/// and a command line's for the browser.
const CLAUDE_CHARS: usize = 14_000;
const URL_BYTES: usize = 24_000;

fn has_key(path: PCWSTR) -> bool {
    let mut key = HKEY::default();
    unsafe {
        let found = RegOpenKeyExW(HKEY_CLASSES_ROOT, path, None, KEY_READ, &mut key).is_ok();
        if found {
            let _ = RegCloseKey(key);
        }
        found
    }
}

/// Whether a command is on the PATH (as an .exe or a .cmd script).
fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|dir| ["exe", "cmd"].iter().any(|ext| dir.join(format!("{name}.{ext}")).is_file()))
}

impl Agent {
    /// The agents installed here, most capable first.
    pub fn detect() -> Vec<Agent> {
        let mut found = Vec::new();
        if has_key(w!("claude\\shell\\open\\command")) {
            found.push(Agent::Claude);
        }
        if has_key(w!("chatgpt")) {
            found.push(Agent::ChatGpt);
        }
        if on_path("codex") {
            found.push(Agent::Codex);
        }
        if on_path("gemini") {
            found.push(Agent::Gemini);
        }
        if found.is_empty() {
            found = vec![Agent::ChatGpt, Agent::Perplexity];
        }
        found
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Agent::Claude => "New chat in Claude",
            Agent::ChatGpt => "New chat in ChatGPT",
            Agent::Codex => "New session in Codex",
            Agent::Gemini => "New session in Gemini CLI",
            Agent::Perplexity => "Ask Perplexity",
        }
    }

    /// Where it opens.
    pub(super) fn place(self) -> &'static str {
        match self {
            Agent::Claude => "App",
            Agent::Codex | Agent::Gemini => "Terminal",
            Agent::ChatGpt | Agent::Perplexity => "Browser",
        }
    }

    pub(super) fn icon(self) -> &'static str {
        match self {
            Agent::Codex | Agent::Gemini => "\u{E756}",
            _ => "\u{E8BD}",
        }
    }
}

/// `s` for a URL's query: everything but unreserved characters as UTF-8
/// percent escapes.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// At most `chars` characters of `s`, cut at a line or word if one is near.
fn cut(s: &str, chars: usize) -> (&str, bool) {
    match s.char_indices().nth(chars) {
        None => (s, false),
        Some((end, _)) => {
            let at = s[..end].rfind('\n').filter(|&i| i > end / 2).or_else(|| s[..end].rfind(' ')).unwrap_or(end);
            (&s[..at], true)
        }
    }
}

/// `s` cut so that encoded it takes at most `bytes`.
fn cut_encoded(s: &str, bytes: usize) -> (&str, bool) {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        used += if c.is_ascii_alphanumeric() { 1 } else { 3 * c.len_utf8() };
        if used > bytes {
            return cut(s, s[..i].chars().count());
        }
    }
    (s, false)
}

/// Opens a web page (or an email) from a note in the browser; nothing
/// else a link could name.
pub fn open_web(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p)) && open_url(url)
}

/// Opens a file of the vault that is not a note (a PDF) in its own app.
pub fn open_file(path: &Path) -> bool {
    path.is_file() && open_url(&path.to_string_lossy())
}

fn open_url(url: &str) -> bool {
    let r = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    r.0 as isize > 32
}

/// Microsoft Edge, which comes with Windows and prints a page to PDF.
fn edge() -> Option<PathBuf> {
    ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|d| PathBuf::from(d).join("Microsoft\\Edge\\Application\\msedge.exe"))
        .find(|p| p.is_file())
}

impl App {
    /// The open note as it reads, with its title, and its name. None on a
    /// page with nothing written yet.
    fn outgoing(&self) -> Option<(String, String)> {
        let text = self.editor.text();
        if text.trim().is_empty() {
            return None;
        }
        let name = if self.session.note.is_empty() { note::file_stem_for(&self.editor.title()) } else { stem(&self.note_path()) };
        Some((text.to_string(), name))
    }

    /// The menu of the title bar's send button, below it: the note shared
    /// as a link; the AI apps found here; then the note exported to a file,
    /// or copied.
    pub(super) fn quick_menu(&mut self, target: Target) {
        let Some(button) = self.hits.iter().find(|(_, t)| *t == target).map(|(r, _)| *r) else { return };
        let at = (button.right, button.bottom + 6.0);
        let item = |icon: &'static str, label: &str, keys: &'static str| MenuItem { icon, label: label.to_string(), keys, ..MenuItem::default() };
        let mut entries: Vec<(MenuItem, Action)> = vec![(item("\u{E71B}", "Share link\u{2026}", ""), Action::ShareLink)];
        for (k, a) in Agent::detect().into_iter().enumerate() {
            entries.push((MenuItem { divider: k == 0, ..item(a.icon(), a.label(), a.place()) }, Action::SendTo(a)));
        }
        entries.push((MenuItem { divider: true, ..item("\u{E72D}", "Export note", "") }, Action::ExportNote));
        entries.push((item("\u{E8C8}", "Copy", ""), Action::CopyRich));
        self.show_menu(at, entries, None, false);
        if let Some(m) = &mut self.menu {
            m.anchor = Some(target);
        }
    }

    /// An import or export button in the options.
    pub(super) fn transfer(&mut self, what: scene::Transfer) {
        self.ui.options_open = false;
        self.request_frame();
        match what {
            scene::Transfer::Files => self.import_files(),
            scene::Transfer::Folder => self.import_folder(),
            scene::Transfer::Clipboard => self.import_clipboard(),
            scene::Transfer::NotesApp => self.import_notes_app(),
            scene::Transfer::Markdown => self.export(Export::Markdown),
            scene::Transfer::Page => self.export(Export::Html),
            scene::Transfer::Pdf => self.export(Export::Pdf),
        }
    }

    // Import.

    /// Where imported notes go: the vault's top.
    fn import_dir(&mut self) -> Option<PathBuf> {
        self.ensure_vault()
    }

    pub(super) fn import_files(&mut self) {
        let files = dialog::open_files(
            self.hwnd,
            "Import notes",
            &[("Notes, documents and exports", "*.md;*.markdown;*.txt;*.nbk;*.enex;*.zip;*.docx;*.pdf;*.html;*.htm"), ("All files", "*.*")],
        );
        let Some(dir) = self.import_dir() else { return };
        let (exports, files): (Vec<PathBuf>, Vec<PathBuf>) = files.into_iter().partition(|f| is_export(f));
        if !exports.is_empty() {
            return self.import_exports(&exports, &dir);
        }
        let (documents, files): (Vec<PathBuf>, Vec<PathBuf>) = files.into_iter().partition(|f| crate::convert::is_document(f));
        if !documents.is_empty() {
            return self.import_documents(&documents, &dir);
        }
        let mut first = None;
        let mut count = 0;
        for file in &files {
            let to = note::unique_path(&dir, &stem(file), None);
            if std::fs::copy(file, &to).is_ok() {
                count += 1;
                first.get_or_insert(to);
            }
        }
        self.imported(count, first);
    }

    /// An export of Notes, chosen: its notes, dated, in a folder of their
    /// own.
    pub(super) fn import_notes_app(&mut self) {
        let files = dialog::open_files(self.hwnd, "Import from Notes", &[("Notes export", "*.nbk")]);
        if files.is_empty() {
            return;
        }
        let Some(dir) = self.import_dir() else { return };
        self.import_exports(&files, &dir);
    }

    pub(super) fn import_folder(&mut self) {
        let Some(from) = dialog::pick_folder(self.hwnd, "Import a folder of notes") else { return };
        let Some(dir) = self.import_dir() else { return };
        if from.starts_with(&dir) {
            return self.whisper("That folder is in this vault already".to_string());
        }
        let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".to_string());
        let mut top = dir.join(&name);
        let mut n = 2;
        while top.exists() {
            top = dir.join(format!("{name} {n}"));
            n += 1;
        }
        let (mut count, mut first) = (0, None);
        let mut todo = vec![(from.clone(), top.clone())];
        while let Some((src, dst)) = todo.pop() {
            let Ok(entries) = std::fs::read_dir(&src) else { continue };
            for e in entries.flatten() {
                let file_name = e.file_name().to_string_lossy().into_owned();
                if file_name.starts_with('.') {
                    continue;
                }
                let path = e.path();
                if path.is_dir() {
                    todo.push((path, dst.join(&file_name)));
                } else if path.extension().is_some_and(|x| ["md", "markdown", "txt"].contains(&x.to_string_lossy().to_lowercase().as_str())) {
                    let _ = std::fs::create_dir_all(&dst);
                    let to = note::unique_path(&dst, &stem(&path), None);
                    if std::fs::copy(&path, &to).is_ok() {
                        count += 1;
                        first.get_or_insert(to);
                    }
                }
            }
        }
        self.imported(count, first);
    }

    pub(super) fn import_clipboard(&mut self) {
        let Some(text) = clipboard::get(self.hwnd).filter(|t| !t.trim().is_empty()) else {
            return self.whisper("The clipboard has no text".to_string());
        };
        // A fresh page with it: it names itself after its first line.
        self.fresh_page();
        self.edit(|e| e.insert(&text));
        self.save_note(false);
    }

    /// Documents (DOCX, PDF, HTML) made notes in `dir`, by a converting
    /// process of their own (see `convert`); the first opens when done.
    pub(super) fn import_documents(&mut self, files: &[PathBuf], dir: &Path) {
        let Some(pictures) = self.attachments_for(dir) else { return };
        let work = crate::Handoff((self.hwnd, files.to_vec(), dir.to_path_buf(), pictures));
        self.whisper(if files.len() == 1 { "Reading it\u{2026}".to_string() } else { format!("Reading {} files\u{2026}", files.len()) });
        std::thread::spawn(move || {
            let work = work;
            let (hwnd, files, dir, pictures) = work.0;
            let done = Box::into_raw(Box::new(crate::convert::run(&files, &dir, &pictures)));
            let posted = unsafe {
                windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(hwnd),
                    crate::window::WM_APP_CONVERTED,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(done as isize),
                )
            };
            if posted.is_err() {
                drop(unsafe { Box::from_raw(done) });
            }
        });
    }

    /// Documents converted (see `import_documents`): the tree shows their
    /// notes, the first opens, and what could not be read is said.
    pub(super) fn documents_in(&mut self, done: crate::convert::Converted) {
        let count = done.notes.len();
        let first = done.notes.first().cloned();
        if count > 0 {
            self.imported(count, first.clone());
        }
        let said = match (count, done.failed.as_slice()) {
            (0, [(name, why)]) => format!("Could not read {name}: {why}"),
            (0, failed) => format!("Could not read {} files", failed.len()),
            (1, []) => format!("Made a note of it: {}", first.as_deref().map(stem).unwrap_or_default()),
            (n, []) => format!("Made {n} notes"),
            (n, failed) => format!("Made {n} notes; {} could not be read", failed.len()),
        };
        self.whisper(said);
    }

    /// Another app's export, or a document, opened with Kolada: imported
    /// into the vault.
    pub(super) fn import_arg(&mut self, file: &Path) {
        let Some(vault) = self.ensure_vault() else { return };
        if crate::convert::is_document(file) {
            self.import_documents(&[file.to_path_buf()], &vault);
        } else {
            self.import_exports(&[file.to_path_buf()], &vault);
        }
    }

    /// Other apps' exports (see `is_export`), each into a folder of `dir`
    /// named after it, on a worker: a thousand files take a while to write.
    /// The newest note opens once they are in (see `exports_in`).
    pub(super) fn import_exports(&mut self, exports: &[PathBuf], dir: &Path) {
        let work = crate::Handoff((self.hwnd, exports.to_vec(), dir.to_path_buf()));
        self.whisper("Importing\u{2026}".to_string());
        std::thread::spawn(move || {
            let work = work;
            let (hwnd, exports, dir) = work.0;
            let done = Box::into_raw(Box::new(write_exports(&exports, &dir)));
            let posted = unsafe {
                windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(hwnd),
                    crate::window::WM_APP_IMPORTED,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(done as isize),
                )
            };
            if posted.is_err() {
                drop(unsafe { Box::from_raw(done) });
            }
        });
    }

    /// Exports written (see `import_exports`): the tree shows them, and the
    /// note changed last opens.
    pub(super) fn exports_in(&mut self, done: ExportsIn) {
        match done {
            Err(why) => self.whisper(why),
            Ok(w) if w.count == 0 => self.whisper("That export has no notes".to_string()),
            Ok(w) => {
                self.imported(w.count, w.newest.map(|n| n.1));
                let left = if w.trashed > 0 { format!(", not the {} in its trash", thousands(w.trashed)) } else { String::new() };
                self.whisper(format!("Imported {} notes with their dates{left}", thousands(w.count)));
            }
        }
    }

    pub(super) fn imported(&mut self, count: usize, first: Option<PathBuf>) {
        if count == 0 {
            return;
        }
        if let Some(first) = first {
            self.open(&first);
        }
        self.refresh_tree();
        self.start_index();
        self.whisper(if count == 1 { "Imported 1 note".to_string() } else { format!("Imported {count} notes") });
    }

    // Export.

    pub(super) fn export(&mut self, kind: Export) {
        let Some((text, name)) = self.outgoing() else { return self.whisper("Nothing to export yet".to_string()) };
        let (filter, ext) = match kind {
            Export::Markdown => (("Markdown", "*.md"), "md"),
            Export::Html => (("Web page", "*.html"), "html"),
            Export::Pdf => (("PDF", "*.pdf"), "pdf"),
        };
        let Some(path) = dialog::save_file(self.hwnd, "Export note", &format!("{name}.{ext}"), filter, ext) else { return };
        self.write_export(kind, &text, &name, path);
    }

    /// Export note, from the send menu: one dialog, the kind chosen in it
    /// (Markdown first, a PDF, a web page).
    pub(super) fn export_note(&mut self) {
        let Some((text, name)) = self.outgoing() else { return self.whisper("Nothing to export yet".to_string()) };
        let kinds = [(Export::Markdown, ("Markdown", "*.md", "md")), (Export::Pdf, ("PDF", "*.pdf", "pdf")), (Export::Html, ("Web page", "*.html", "html"))];
        let types: Vec<(&str, &str, &str)> = kinds.iter().map(|(_, t)| *t).collect();
        let Some((path, i)) = dialog::save_file_as(self.hwnd, "Export note", &name, &types) else { return };
        let (kind, (_, _, ext)) = kinds[i.min(kinds.len() - 1)];
        // The name the kind's own, whatever was typed.
        let path = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) { path } else { path.with_extension(ext) };
        self.write_export(kind, &text, &name, path);
    }

    fn write_export(&mut self, kind: Export, text: &str, name: &str, path: PathBuf) {
        // Pictures the note shows go inside the page, so it carries them
        // wherever it is sent (and into the PDF printed from it).
        let vault = PathBuf::from(&self.session.vault);
        let from = if self.session.note.is_empty() { vault.clone() } else { self.note_path().parent().map_or(vault.clone(), Path::to_path_buf) };
        let known = std::cell::RefCell::new(None);
        let picture = |name: &str| {
            if vault.as_os_str().is_empty() {
                return None;
            }
            let file = kolada_ui::pictures::find(&vault, &from, name, &mut known.borrow_mut())?;
            let bytes = std::fs::read(&file).ok().filter(|b| b.len() <= LARGEST_EXPORTED_PICTURE)?;
            Some(html::data_uri(&file.to_string_lossy(), &bytes))
        };
        let page = || html::document(name, &html::to_html_with(text, true, &picture));
        let written = match kind {
            Export::Markdown => std::fs::write(&path, text.replace('\n', "\r\n")).is_ok(),
            Export::Html => std::fs::write(&path, page()).is_ok(),
            Export::Pdf => return self.print_pdf(page(), path),
        };
        if written {
            self.whisper(format!("Exported {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        }
    }

    /// Prints the page to a PDF with Edge, headless, on a worker; says so
    /// when it is done.
    fn print_pdf(&mut self, page: String, out: PathBuf) {
        let Some(edge) = edge() else { return self.whisper("PDF needs Microsoft Edge".to_string()) };
        let dir = std::env::temp_dir().join(format!("kolada-pdf-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let html = dir.join("note.html");
        if std::fs::write(&html, page).is_err() {
            return;
        }
        let hwnd = crate::Handoff(self.hwnd);
        self.whisper("Making the PDF\u{2026}".to_string());
        std::thread::spawn(move || {
            let hwnd = hwnd;
            let url = format!("file:///{}", html.to_string_lossy().replace('\\', "/"));
            let status = Command::new(edge)
                .args(["--headless", "--disable-gpu", "--no-first-run", "--no-pdf-header-footer"])
                .arg(format!("--user-data-dir={}", dir.join("profile").display()))
                .arg(format!("--print-to-pdf={}", out.display()))
                .arg(url)
                .status();
            let done = status.is_ok_and(|s| s.success()) && out.is_file();
            let _ = std::fs::remove_dir_all(&dir);
            let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            crate::post_whisper(hwnd.0, if done { format!("Exported {name}") } else { "Could not make the PDF".to_string() });
        });
    }

    /// The note copied: formatted for mail and documents, its Markdown for
    /// anything that takes plain text (an AI's chat among them).
    pub(super) fn copy_rich(&mut self) {
        let Some((text, _)) = self.outgoing() else { return };
        if clipboard::set_rich(self.hwnd, &text, Some(&html::to_html(&text, true))) {
            self.whisper("Copied the note".to_string());
        }
    }

    pub(super) fn copy_markdown(&mut self) {
        let Some((text, _)) = self.outgoing() else { return };
        if clipboard::set(self.hwnd, &text) {
            self.whisper("Copied as Markdown".to_string());
        }
    }

    // Send to an AI.

    /// Starts a new chat in `agent` with the open note in it. A note too long
    /// for the link goes in cut, and whole on the clipboard.
    pub(super) fn send_to(&mut self, agent: Agent) {
        let Some((text, name)) = self.outgoing() else { return self.whisper("Write something to send first".to_string()) };
        self.save_note(false);
        let dir = if self.session.note.is_empty() { PathBuf::from(&self.session.vault) } else { self.note_path().parent().map(Path::to_path_buf).unwrap_or_default() };
        let file = if self.session.note.is_empty() { String::new() } else { format!("{name}.md") };
        let (sent, cut_short) = match agent {
            Agent::Claude => {
                let (q, cut_short) = cut(&text, CLAUDE_CHARS);
                (open_url(&format!("claude://claude.ai/new?q={}", encode(q))), cut_short)
            }
            Agent::ChatGpt | Agent::Perplexity => {
                let (q, cut_short) = cut_encoded(&text, URL_BYTES);
                let base = if agent == Agent::ChatGpt { "https://chatgpt.com/?q=" } else { "https://www.perplexity.ai/search?q=" };
                (open_url(&format!("{base}{}", encode(q))), cut_short)
            }
            Agent::Codex | Agent::Gemini => {
                let prompt = if file.is_empty() { "Help me with a note.".to_string() } else { format!("Read my note '{file}' in this folder.") };
                let prompt = prompt.replace(['"', ';'], " ");
                let (cmd, flag) = if agent == Agent::Codex { ("codex", None) } else { ("gemini", Some("-i")) };
                let mut c = Command::new("wt.exe");
                c.arg("-d").arg(&dir).arg(cmd);
                if let Some(flag) = flag {
                    c.arg(flag);
                }
                c.arg(&prompt);
                (c.spawn().is_ok(), false)
            }
        };
        if !sent {
            return self.whisper("Could not open it".to_string());
        }
        if cut_short {
            clipboard::set(self.hwnd, &text);
            self.whisper("A long note: all of it is on the clipboard too".to_string());
        }
    }

    // A note's own window.

    /// Opens the note at `path` in a window of its own, a process of its own
    /// (it starts in a few frames): at the screen point `at`, or beside the
    /// side menu.
    pub(super) fn open_window(&mut self, path: &Path, at: Option<(i32, i32)>) {
        let Ok(exe) = std::env::current_exe() else { return };
        self.save_note(true);
        let at = at.or_else(|| {
            let mut r = RECT::default();
            unsafe { GetWindowRect(self.hwnd, &mut r).ok()? };
            let scale = window::dpi(self.hwnd) as f32 / 96.0;
            Some((r.left + (300.0 * scale) as i32, r.top + (56.0 * scale) as i32))
        });
        let mut c = Command::new(exe);
        c.arg("--window").arg(path);
        if NO_ACTIVATE.load(std::sync::atomic::Ordering::Relaxed) {
            c.arg("--no-activate");
        }
        if let Some((x, y)) = at {
            c.arg("--at").arg(format!("{x},{y}"));
        }
        match c.spawn() {
            Ok(child) => unsafe {
                let _ = AllowSetForegroundWindow(child.id());
            },
            Err(_) => self.whisper("Could not open a window".to_string()),
        }
    }

    /// A note's own window hands its note to the main window (starting it if
    /// it is not running), and closes.
    pub(super) fn back_to_main(&mut self) {
        if self.session.note.is_empty() {
            return;
        }
        self.save_note(true);
        let path = self.note_path();
        let running = unsafe { FindWindowW(window::main_class(), PCWSTR::null()) }.is_ok();
        if running {
            crate::instance::forward(Some(&path));
        } else if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe).arg(&path).spawn();
        }
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Pictures larger than this stay out of an exported page.
const LARGEST_EXPORTED_PICTURE: usize = 25 * 1024 * 1024;

/// What a note exports as.
#[derive(Clone, Copy, Debug)]
pub enum Export {
    Markdown,
    Html,
    Pdf,
}

/// Whether `path` is another notes app's export that Kolada reads: Notes'
/// `.nbk` (see `kolada_core::nbk`).
pub fn is_export(path: &Path) -> bool {
    path.extension().is_some_and(|x| x.eq_ignore_ascii_case("nbk"))
}

/// What writing exports did, or why one could not be.
pub(super) type ExportsIn = std::result::Result<Written, String>;

/// Writes every export in `exports` (see `write_export`).
fn write_exports(exports: &[PathBuf], dir: &Path) -> ExportsIn {
    let mut all = Written { count: 0, newest: None, trashed: 0 };
    for file in exports {
        let done = write_export(file, dir).map_err(|why| format!("Could not import {}: {why}", file.file_name().unwrap_or_default().to_string_lossy()))?;
        all.count += done.count;
        all.trashed += done.trashed;
        if let Some(n) = done.newest.filter(|n| all.newest.as_ref().is_none_or(|m| n.0 > m.0)) {
            all.newest = Some(n);
        }
    }
    Ok(all)
}

/// What writing an export did: how many notes, the one changed last (when,
/// where), and how many were in its trash.
pub(super) struct Written {
    count: usize,
    newest: Option<(i64, PathBuf)>,
    trashed: usize,
}

/// Writes the notes of the export at `path` as Markdown into a new folder
/// of `dir` named after it, in their folders, each file dated when its note
/// was made and last changed, so they sort as they did.
fn write_export(path: &Path, dir: &Path) -> std::result::Result<Written, String> {
    use std::io::Write as _;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let export = kolada_core::nbk::read(&bytes).map_err(|e| e.to_string())?;
    let name = note::file_stem_for(&stem(path));
    let top = (1..).map(|n| if n == 1 { dir.join(&name) } else { dir.join(format!("{name} {n}")) }).find(|p| !p.exists()).expect("a free name");
    let mut done = Written { count: 0, newest: None, trashed: export.trashed };
    // The folder is new: its names are known without asking the disk.
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut made: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for n in &export.notes {
        let folder = n.folders.iter().fold(top.clone(), |p, f| p.join(note::file_stem_for(f)));
        if !made.contains(&folder) {
            if std::fs::create_dir_all(&folder).is_err() {
                continue;
            }
            made.insert(folder.clone());
        }
        let stem = note::file_stem_for(&kolada_editor::title_of(&n.text));
        let to = (1..)
            .map(|k| folder.join(if k == 1 { format!("{stem}.md") } else { format!("{stem} {k}.md") }))
            .find(|p| !taken.contains(&p.to_string_lossy().to_lowercase()))
            .expect("a free name");
        taken.insert(to.to_string_lossy().to_lowercase());
        let Ok(mut file) = std::fs::File::create_new(&to) else { continue };
        if file.write_all(n.text.as_bytes()).is_err() {
            continue;
        }
        let _ = file.set_times(times(n.created, n.modified));
        done.count += 1;
        if done.newest.as_ref().is_none_or(|m| n.modified > m.0) {
            done.newest = Some((n.modified, to));
        }
    }
    Ok(done)
}

/// A file's times for one made at `created` and last changed at `modified`
/// (milliseconds since 1970; 0 leaves one as it is).
fn times(created: i64, modified: i64) -> std::fs::FileTimes {
    use std::os::windows::fs::FileTimesExt;
    let at = |ms: i64| (ms > 0).then(|| std::time::UNIX_EPOCH + Duration::from_millis(ms as u64));
    let mut times = std::fs::FileTimes::new();
    if let Some(m) = at(modified) {
        times = times.set_modified(m).set_accessed(m);
    }
    if let Some(c) = at(created) {
        times = times.set_created(c);
    }
    times
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_carry_any_text() {
        assert_eq!(encode("a b\n#c&d=é"), "a%20b%0A%23c%26d%3D%C3%A9");
        assert_eq!(encode("ملاحظة").len(), 6 * 2 * 3);
    }

    #[test]
    fn long_notes_are_cut_at_a_line() {
        let text = "first line\nsecond line that goes on\nthird";
        // At a line when one is near the end, else at a word.
        let (s, cut_short) = cut(text, 25);
        assert!(cut_short && s == "first line
second line", "{s:?}");
        assert_eq!(cut(text, 14).0, "first line");
        assert_eq!(cut(text, 1000), (text, false));
        let arabic = "كلمة ".repeat(100);
        let (s, cut_short) = cut_encoded(&arabic, 300);
        assert!(cut_short && encode(s).len() <= 300 && !s.is_empty());
    }

    #[test]
    fn an_export_becomes_dated_notes_in_their_folders() {
        let dir = std::env::temp_dir().join(format!("kolada-export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("From phone.nbk");
        std::fs::write(&file, include_bytes!("../../../kolada-core/testdata/notes.nbk")).unwrap();
        let vault = dir.join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let done = write_export(&file, &vault).unwrap();
        assert_eq!((done.count, done.trashed), (6, 1));
        let top = vault.join("From phone");
        let mut found: Vec<String> = Vec::new();
        let mut todo = vec![top.clone()];
        while let Some(d) = todo.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                if e.path().is_dir() {
                    todo.push(e.path());
                } else {
                    found.push(e.path().strip_prefix(&top).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        found.sort();
        assert_eq!(
            found,
            ["Groceries 2.md", "Groceries.md", "Only a title, nothing written under it.md", "Tagged.md", "Work/Plans 2026/Launch.md", "Work/What now.md"]
        );
        // Dated as they were: the first groceries made on 10 October 2022,
        // changed a day later.
        let meta = std::fs::metadata(top.join("Groceries.md")).unwrap();
        let ms = |t: std::time::SystemTime| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        assert_eq!(ms(meta.created().unwrap()), 1_665_419_890_626);
        assert_eq!(ms(meta.modified().unwrap()), 1_665_419_890_626 + 86_400_000);
        assert_eq!(std::fs::read_to_string(top.join("Work/What now.md")).unwrap(), "# What: now?\n**Soon**");
        assert_eq!(done.newest.map(|n| n.1), Some(top.join("Work/What now.md")));
        // Again: a folder of its own, beside the first.
        assert_eq!(write_export(&file, &vault).unwrap().count, 6);
        assert!(vault.join("From phone 2").join("Tagged.md").exists());
        assert!(write_export(&dir.join("missing.nbk"), &vault).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Imports a real export into a scratch folder and says how it went
    /// (KOLADA_NBK=file).
    #[test]
    #[ignore = "reads the export named by KOLADA_NBK"]
    fn imports_a_real_export() {
        let file = PathBuf::from(std::env::var("KOLADA_NBK").unwrap());
        let vault = std::env::temp_dir().join(format!("kolada-real-export-{}", std::process::id()));
        std::fs::create_dir_all(&vault).unwrap();
        let start = std::time::Instant::now();
        let done = write_export(&file, &vault).unwrap();
        let took = start.elapsed();
        let mut dates = std::collections::HashSet::new();
        for e in std::fs::read_dir(vault.join(note::file_stem_for(&stem(&file)))).unwrap().flatten() {
            dates.insert(e.metadata().unwrap().modified().unwrap());
        }
        println!("imported {} notes ({} in the trash left out) in {took:?}; {} distinct file dates", done.count, done.trashed, dates.len());
        let _ = std::fs::remove_dir_all(&vault);
    }
}
