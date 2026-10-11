//! The open note going out, as on Windows (`notsy-win` `app/share.rs`): to
//! an AI app found on this Mac as the start of a new chat, to a file
//! (Markdown, PDF, a web page), or copied with its formatting. The model
//! says what to do; the view does it (`view/share.rs`).

use std::path::{Path, PathBuf};

use notsy_markdown::html;

use super::*;

/// An AI app to start a chat in, found on this Mac (or, with none, on the web).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    /// The Claude app: a new chat with the note in its prompt box.
    Claude,
    /// ChatGPT, in the browser (its app takes no text through a link).
    ChatGpt,
    /// Command-line agents, in Terminal in the note's folder.
    Codex,
    Gemini,
    /// On the web, when nothing is installed.
    Perplexity,
}

/// What a link can carry, in characters of the note: the apps' own limits,
/// and a URL's for the browser.
const CLAUDE_CHARS: usize = 14_000;
const URL_BYTES: usize = 24_000;

impl Agent {
    /// The agents found, most capable first: `app` says whether a chat app
    /// is installed, `tool` whether a command-line one is.
    pub fn detect_with(app: impl Fn(&str) -> bool, tool: impl Fn(&str) -> bool) -> Vec<Agent> {
        let mut found = Vec::new();
        if app("Claude") {
            found.push(Agent::Claude);
        }
        if app("ChatGPT") {
            found.push(Agent::ChatGpt);
        }
        if tool("codex") {
            found.push(Agent::Codex);
        }
        if tool("gemini") {
            found.push(Agent::Gemini);
        }
        if found.is_empty() {
            found = vec![Agent::ChatGpt, Agent::Perplexity];
        }
        found
    }

    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "New Chat in Claude",
            Agent::ChatGpt => "New Chat in ChatGPT",
            Agent::Codex => "New Session in Codex",
            Agent::Gemini => "New Session in Gemini CLI",
            Agent::Perplexity => "Ask Perplexity",
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

/// What a note exports as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Export {
    Markdown,
    Pdf,
    Html,
}

impl Export {
    /// The kinds the save panel offers, Markdown first.
    pub const ALL: [Export; 3] = [Export::Markdown, Export::Pdf, Export::Html];

    pub fn ext(self) -> &'static str {
        match self {
            Export::Markdown => "md",
            Export::Pdf => "pdf",
            Export::Html => "html",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Export::Markdown => "Markdown",
            Export::Pdf => "PDF",
            Export::Html => "Web Page",
        }
    }
}

/// What the view does for the model, outside it: the OS's clipboard, its
/// panels, its browser, its Finder and Terminal.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Text on the clipboard, and what to say.
    Clipboard(String, String),
    /// The note on the clipboard formatted (HTML) and as Markdown.
    Rich { text: String, html: String },
    /// A save panel for the note named `name`, then `export` it there.
    Export { name: String },
    /// A web page (or a mail) in the browser, or an app's link.
    Url(String),
    /// A file or folder shown in the Finder.
    Reveal(PathBuf),
    /// A command started in Terminal, in a folder.
    Terminal { dir: PathBuf, command: String },
    /// The note at this path in a window of its own.
    Window(PathBuf),
}

impl Model {
    /// The open note as it reads, with its title, and its name. None on a
    /// page with nothing written yet.
    pub fn outgoing(&self) -> Option<(String, String)> {
        let text = self.editor.text();
        if text.trim().is_empty() {
            return None;
        }
        let name = match &self.path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            None => note::file_stem_for(&self.editor.title()),
        };
        Some((text.to_string(), name))
    }

    /// The note copied: formatted for mail and documents, its Markdown for
    /// anything that takes plain text.
    pub fn copy_rich(&mut self) -> Option<Effect> {
        let (text, _) = self.outgoing()?;
        let html = html::to_html(&text, true);
        self.whisper("Copied the note");
        Some(Effect::Rich { text, html })
    }

    /// Export Note: a save panel for it (the view asks where and which kind).
    pub fn export_note(&mut self) -> Option<Effect> {
        match self.outgoing() {
            Some((_, name)) => Some(Effect::Export { name }),
            None => {
                self.whisper("Nothing to export yet");
                None
            }
        }
    }

    /// The bytes of the note exported as `kind` (a PDF is printed from the
    /// web page by the view): its Markdown, or a page of its own.
    pub fn export_page(&self, kind: Export) -> Option<String> {
        let (text, name) = self.outgoing()?;
        Some(match kind {
            Export::Markdown => text,
            Export::Html | Export::Pdf => self.page_of(&name, &text),
        })
    }

    /// The note was exported to `path` (or could not be).
    pub fn exported(&mut self, path: &Path, ok: bool) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.whisper(&if ok { format!("Exported {name}") } else { format!("Could not export {name}") });
    }

    /// Starts a new chat in `agent` with the open note in it. A note too long
    /// for the link goes in cut, and whole on the clipboard too.
    pub fn send_to(&mut self, agent: Agent) -> Vec<Effect> {
        let Some((text, name)) = self.outgoing() else {
            self.whisper("Write something to send first");
            return Vec::new();
        };
        self.save();
        let dir = self.path.as_deref().and_then(Path::parent).map_or_else(|| self.vault.clone(), Path::to_path_buf);
        let (effect, cut_short) = match agent {
            Agent::Claude => {
                let (q, cut_short) = cut(&text, CLAUDE_CHARS);
                (Effect::Url(format!("claude://claude.ai/new?q={}", encode(q))), cut_short)
            }
            Agent::ChatGpt | Agent::Perplexity => {
                let (q, cut_short) = cut_encoded(&text, URL_BYTES);
                let base = if agent == Agent::ChatGpt { "https://chatgpt.com/?q=" } else { "https://www.perplexity.ai/search?q=" };
                (Effect::Url(format!("{base}{}", encode(q))), cut_short)
            }
            Agent::Codex | Agent::Gemini => {
                let prompt = if self.path.is_none() { "Help me with a note.".to_string() } else { format!("Read my note '{name}.md' in this folder.") };
                let prompt = prompt.replace(['"', '\'', ';', '$', '`', '\\'], " ");
                let tool = if agent == Agent::Codex { "codex" } else { "gemini" };
                let tool = self.services.installed.tool(tool).map_or(tool.to_string(), |p| p.to_string_lossy().into_owned());
                let flag = if agent == Agent::Gemini { " -i" } else { "" };
                (Effect::Terminal { dir, command: format!("'{tool}'{flag} \"{prompt}\"") }, false)
            }
        };
        let mut effects = vec![effect];
        if cut_short {
            effects.push(Effect::Clipboard(text, "A long note: all of it is on the clipboard too".to_string()));
        }
        effects
    }
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
        let (s, cut_short) = cut(text, 25);
        assert!(cut_short && s == "first line\nsecond line", "{s:?}");
        assert_eq!(cut(text, 14).0, "first line");
        assert_eq!(cut(text, 1000), (text, false));
        let arabic = "كلمة ".repeat(100);
        let (s, cut_short) = cut_encoded(&arabic, 300);
        assert!(cut_short && encode(s).len() <= 300 && !s.is_empty());
    }

    #[test]
    fn the_apps_found_come_first_and_the_web_when_there_are_none() {
        assert_eq!(Agent::detect_with(|a| a == "Claude", |t| t == "codex"), [Agent::Claude, Agent::Codex]);
        assert_eq!(Agent::detect_with(|_| false, |_| false), [Agent::ChatGpt, Agent::Perplexity]);
        assert_eq!(Agent::detect_with(|a| a == "ChatGPT", |t| t == "gemini"), [Agent::ChatGpt, Agent::Gemini]);
    }

    fn vault(test: &str, note: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("notsy-app-share-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), note).unwrap();
        let mut m = Model::new(dir.clone(), true);
        m.frame(1600, 1000, 2.0);
        (m, dir)
    }

    #[test]
    fn a_note_goes_to_claude_whole_and_to_the_web_cut_with_all_of_it_on_the_clipboard() {
        let (mut m, dir) = vault("send", "# Plan\n\nمرحبا and hello\n");
        let sent = m.send_to(Agent::Claude);
        assert_eq!(sent.len(), 1);
        let Effect::Url(url) = &sent[0] else { panic!("{sent:?}") };
        assert!(url.starts_with("claude://claude.ai/new?q=%23%20Plan"), "{url}");
        assert!(url.contains(&encode("مرحبا")));
        // Too long for a link: cut, and the whole note on the clipboard.
        let long = format!("# Long\n\n{}", "word ".repeat(10_000));
        m.insert(&long);
        let sent = m.send_to(Agent::ChatGpt);
        assert!(matches!(&sent[0], Effect::Url(u) if u.starts_with("https://chatgpt.com/?q=") && u.len() < 24_100));
        assert!(matches!(&sent[1], Effect::Clipboard(t, _) if t.len() > 50_000));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_terminal_agent_starts_in_the_notes_folder_naming_the_note() {
        let (mut m, dir) = vault("terminal", "# Plan\n\nText\n");
        let sent = m.send_to(Agent::Codex);
        let Effect::Terminal { dir: at, command } = &sent[0] else { panic!("{sent:?}") };
        assert_eq!(at, &dir);
        assert!(command.contains("codex") && command.ends_with("\"Read my note  Plan.md  in this folder.\""), "{command}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_note_exports_as_markdown_or_a_page_and_copies_with_its_formatting() {
        let (mut m, dir) = vault("export", "# Plan\n\n**Bold** and عربي\n");
        assert_eq!(m.export_note(), Some(Effect::Export { name: "Plan".into() }));
        assert_eq!(m.export_page(Export::Markdown).unwrap(), "# Plan\n\n**Bold** and عربي\n");
        let page = m.export_page(Export::Html).unwrap();
        assert!(page.contains("<strong>Bold</strong>") && page.contains("عربي") && page.contains("<title>Plan</title>"), "{page}");
        let Some(Effect::Rich { text, html }) = m.copy_rich() else { panic!() };
        assert!(text.starts_with("# Plan") && html.contains("<strong>"));
        // A blank page has nothing to send.
        m.new_note();
        assert_eq!(m.export_note(), None);
        assert!(m.copy_rich().is_none());
        assert!(m.send_to(Agent::Claude).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
