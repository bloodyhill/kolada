//! HTML into Markdown. A page is read as a browser forgives it (unclosed
//! paragraphs and list items, stray end tags), then only its article is
//! written: `<main>`, or its largest `<article>`, or else the body without
//! navigation, asides, footers, forms, scripts and hidden things.
//!
//! Rules as markdownify has them, with these changes: a first table row is
//! the header (never an empty one above it), tables that only lay a page
//! out are read as text, task lists stay tasks, relative links resolve
//! against the page's own address when it says it, and pictures inside the
//! page (`data:`) come out as files.

use std::cell::Cell;
use std::rc::Rc;

use crate::md::{self, Block, Span};
use crate::xml::{Element, Node};
use crate::{Error, Note, Pasted, Picture};

const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
/// Elements whose content is text, not markup.
const RAW: &[&str] = &["script", "style", "textarea", "title", "xmp", "noscript", "template", "iframe", "noembed", "noframes"];
/// Elements that end an open paragraph when they start.
const ENDS_P: &[&str] = &[
    "address", "article", "aside", "blockquote", "details", "div", "dl", "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2", "h3",
    "h4", "h5", "h6", "header", "hr", "main", "menu", "nav", "ol", "p", "pre", "section", "table", "ul",
];
/// Never written: page furniture and what is not text.
const SKIP: &[&str] = &[
    "head", "title", "script", "style", "noscript", "template", "svg", "math", "iframe", "object", "embed", "canvas", "video", "audio", "nav", "aside",
    "footer", "form", "button", "select", "textarea", "dialog", "noembed", "noframes",
];
/// Never written when pasting: what is not text. All else that was copied
/// is wanted, navigation and footers included.
const PASTE_SKIP: &[&str] = &[
    "head", "title", "script", "style", "noscript", "template", "svg", "math", "iframe", "object", "embed", "canvas", "video", "audio", "button",
    "select", "textarea", "dialog", "noembed", "noframes",
];
/// Blocks: they start and end a paragraph.
const BLOCKS: &[&str] = &[
    "html", "body", "main", "article", "section", "div", "p", "header", "figure", "figcaption", "address", "center", "details", "summary", "dl",
    "dt", "dd", "fieldset", "legend", "hgroup", "picture",
];

pub fn convert(bytes: &[u8]) -> Result<Note, Error> {
    let text = decode(bytes);
    let saved_from = text.find("saved from url=(").and_then(|i| {
        let rest = &text[i..];
        let start = rest.find(')')? + 1;
        let end = rest[start..].find(|c: char| c.is_whitespace() || c == '-').map_or(rest.len(), |e| start + e);
        Some(rest[start..end].to_string())
    });
    let doc = parse(&text);
    let mut base = None;
    let mut title = None;
    walk_head(&doc, &mut base, &mut title);
    let base = base.or(saved_from).filter(|b| b.starts_with("http"));
    let root = article(&doc);
    let mut out = Out { base, ..Out::default() };
    out.walk(root, &Look::default());
    out.flush();
    // The first heading of the article is its title, before the page's.
    let first_heading = out.blocks.iter().find_map(|b| match b {
        Block::Heading(_, spans) => Some(md::inline(spans)),
        _ => None,
    });
    let title = first_heading.filter(|h| !h.is_empty()).or(title);
    if out.blocks.is_empty() {
        return Err(Error("the page has no text".to_string()));
    }
    Ok(Note { title, markdown: md::write(&out.blocks), pictures: out.pictures, beside: out.beside })
}

/// HTML copied from a page, a document or an email (`base`: the page's
/// address, for its relative links), as Markdown to paste: all that was
/// copied, no article picked out of it, its pictures embedded by names to
/// save them under (see `Pasted`). None when it says nothing its plain text
/// does not (no headings, lists, tables, quotes, code, links, emphasis or
/// pictures), or when it is code from an editor (laid out with its spaces
/// kept), so the copied text pastes as it is.
pub fn paste(html: &str, base: Option<&str>) -> Option<Pasted> {
    let doc = parse(html);
    let root = doc.find("body").unwrap_or(&doc);
    if kept_spaces(root) {
        return None;
    }
    let mut out = Out { base: base.filter(|b| b.starts_with("http")).map(str::to_string), paste: true, ..Out::default() };
    out.walk(root, &Look::default());
    out.flush();
    let structured = out.blocks.iter().any(|b| match b {
        Block::Paragraph(spans) => spans.iter().any(|s| s.bold || s.italic || s.strike || s.code || s.link.is_some()),
        _ => true,
    });
    let markdown = md::write(&out.blocks).trim_end().to_string();
    (structured && !markdown.trim().is_empty()).then_some(Pasted { markdown, pictures: out.pictures, fetch: out.fetch })
}

/// Copied from a code editor: what was copied sits in an element that keeps
/// its spaces (`white-space: pre`), outside a `<pre>`.
fn kept_spaces(root: &Element) -> bool {
    let mut e = root;
    loop {
        if e.name != "pre" && style_has(e, "white-space:pre") {
            return true;
        }
        let mut inner = e.elements().filter(|c| !matches!(c.name.as_str(), "meta" | "style" | "script" | "link"));
        match (inner.next(), inner.next()) {
            (Some(only), None) if e.children.iter().all(|n| matches!(n, Node::Element(_)) || matches!(n, Node::Text(t) if t.trim().is_empty())) => e = only,
            _ => return false,
        }
    }
}

/// Whether `e`'s inline style says `what` (spaces left out, any case).
fn style_has(e: &Element, what: &str) -> bool {
    e.attr("style").is_some_and(|s| s.to_ascii_lowercase().replace([' ', '\t', '\n'], "").contains(what))
}

/// The value of property `name` in `e`'s inline style, lower case.
fn style_of(e: &Element, name: &str) -> Option<String> {
    let style = e.attr("style")?.to_ascii_lowercase();
    style.split(';').find_map(|d| {
        let (k, v) = d.split_once(':')?;
        (k.trim() == name).then(|| v.trim().trim_end_matches("!important").trim().to_string())
    })
}

/// Bold or not, as `e`'s style says (Google Docs sets weights so, and wraps
/// what is copied in a `<b>` it says is not bold).
fn weight(e: &Element) -> Option<bool> {
    let w = style_of(e, "font-weight")?;
    match w.as_str() {
        "bold" | "bolder" => Some(true),
        "normal" | "lighter" => Some(false),
        n => n.parse::<u32>().ok().map(|n| n >= 600),
    }
}

/// A Word or Outlook list paragraph (`mso-list: l0 level2 lfo1`): how deep
/// it is (0 outermost).
fn word_list(e: &Element) -> Option<usize> {
    if e.name != "p" {
        return None;
    }
    let list = style_of(e, "mso-list")?;
    let level = list.split_whitespace().find_map(|w| w.strip_prefix("level")?.parse::<usize>().ok())?;
    Some(level.clamp(1, 9) - 1)
}

/// A list item's own number or bullet as Word writes it before its text:
/// `·`, `o`, `§`, `1.`, `a)`, `iv.`
fn is_marker(word: &str) -> bool {
    let bullets = ["·", "•", "o", "§", "▪", "◦", "-", "–", "*", "Ø", "ü", "\u{F0B7}", "\u{F0A7}"];
    if bullets.contains(&word) {
        return true;
    }
    let Some(body) = word.strip_suffix(['.', ')']) else { return false };
    !body.is_empty() && body.len() <= 4 && body.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_numbered(marker: &str) -> bool {
    marker.ends_with(['.', ')']) && is_marker(marker)
}

/// The text of the first element in `e` styled `mso-list: Ignore` (Word's
/// own number or bullet for a list paragraph).
fn word_marker(e: &Element) -> Option<String> {
    for c in e.elements() {
        if style_has(c, "mso-list:ignore") {
            return Some(collapse(&c.text()).trim().to_string());
        }
        if let Some(m) = word_marker(c) {
            return Some(m);
        }
    }
    None
}

/// The page's own address (to resolve its links against) and title.
fn walk_head(e: &Element, base: &mut Option<String>, title: &mut Option<String>) {
    for c in e.elements() {
        match c.name.as_str() {
            "base" if base.is_none() => *base = c.attr("href").map(str::to_string),
            "link" if c.attr("rel").is_some_and(|r| r.eq_ignore_ascii_case("canonical")) && base.is_none() => {
                *base = c.attr("href").map(str::to_string)
            }
            "meta" if c.attr("property") == Some("og:url") && base.is_none() => *base = c.attr("content").map(str::to_string),
            "title" if title.is_none() => {
                let t = collapse(&c.text());
                if !t.trim().is_empty() {
                    *title = Some(t.trim().to_string());
                }
            }
            "body" => {}
            _ => walk_head(c, base, title),
        }
    }
}

/// What to write: `<main>`, or the article with the most text, or the body.
fn article(doc: &Element) -> &Element {
    // An empty one is filled in by the page's script, which is not run:
    // what the page has without it is in its body.
    let mut mains = Vec::new();
    doc.find_all("main", &mut mains);
    if let Some(m) = mains.into_iter().find(|m| !m.text().trim().is_empty()) {
        return m;
    }
    let mut articles = Vec::new();
    doc.find_all("article", &mut articles);
    if let Some(a) = articles.into_iter().filter(|a| !a.text().trim().is_empty()).max_by_key(|a| a.text().len()) {
        return a;
    }
    doc.find("body").unwrap_or(doc)
}

/// How text looks where it is.
#[derive(Clone, Default)]
struct Look {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

#[derive(Default)]
struct Out {
    blocks: Vec<Block>,
    spans: Vec<Span>,
    pictures: Vec<Picture>,
    beside: Vec<(String, String)>,
    base: Option<String>,
    /// Inside a heading or a cell: blocks are line breaks.
    inline: bool,
    /// A checkbox before any text: the item is a task, done or not.
    task: Option<bool>,
    /// Pasting (see `paste`): all that was copied, looks from styles too.
    paste: bool,
    /// Pictures a paste brings from elsewhere: the names the note embeds
    /// them by, and where they are (`file:` and a path on this machine, or
    /// a web address).
    fetch: Vec<(String, String)>,
    /// Pictures found so far, for their names: one count for every writer
    /// of a page, so no two share one.
    count: Rc<Cell<usize>>,
}

impl Out {
    fn flush(&mut self) {
        if self.inline {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        if spans.iter().any(|s| !s.text.trim().is_empty()) {
            self.blocks.push(Block::Paragraph(spans));
        }
    }

    fn text(&mut self, raw: &str, look: &Look) {
        let mut t = collapse(raw);
        // One space where two runs meet.
        let ends_spaced = self.spans.last().is_none_or(|s| s.text.ends_with([' ', '\n']));
        if ends_spaced {
            t = t.trim_start().to_string();
        }
        if t.is_empty() {
            return;
        }
        self.spans.push(Span { text: t, bold: look.bold, italic: look.italic, strike: look.strike, code: look.code, link: look.link.clone(), raw: false });
    }

    fn line_break(&mut self) {
        self.spans.push(Span::plain("\n"));
    }

    /// A fresh writer for what is inside `e` (a heading, a cell, an item).
    fn sub(&self, inline: bool) -> Out {
        Out { base: self.base.clone(), inline, paste: self.paste, count: self.count.clone(), ..Out::default() }
    }

    /// Takes the pictures a sub-writer found.
    fn adopt(&mut self, sub: &mut Out) {
        self.pictures.append(&mut sub.pictures);
        self.beside.append(&mut sub.beside);
        self.fetch.append(&mut sub.fetch);
    }

    /// A name for the next picture found, with extension `ext`.
    fn picture_name(&self, ext: &str) -> String {
        self.count.set(self.count.get() + 1);
        format!("picture {}.{ext}", self.count.get())
    }

    fn walk(&mut self, e: &Element, look: &Look) {
        for n in &e.children {
            match n {
                Node::Text(t) => self.text(t, look),
                Node::Element(c) => self.element(c, look),
            }
        }
    }

    fn element(&mut self, e: &Element, look: &Look) {
        let name = e.name.as_str();
        if self.paste {
            // What was copied is wanted, but for what is not text, what is
            // hidden, and Word's own list bullets (see `word_item`).
            if PASTE_SKIP.contains(&name) || e.attr("hidden").is_some() || style_has(e, "display:none") || style_has(e, "mso-list:ignore") {
                return;
            }
        } else if SKIP.contains(&name) || hidden(e) {
            return;
        }
        // A page header that holds its navigation is furniture too.
        if !self.paste && name == "header" && e.find("nav").is_some() {
            return;
        }
        let mut look = look.clone();
        if self.paste {
            // Copied text keeps its looks in styles as often as in tags.
            if let Some(bold) = weight(e) {
                look.bold = bold;
            }
            if let Some(style) = style_of(e, "font-style") {
                look.italic = style.contains("italic") || style.contains("oblique");
            }
            if style_of(e, "text-decoration").or_else(|| style_of(e, "text-decoration-line")).is_some_and(|d| d.contains("line-through")) {
                look.strike = true;
            }
            if let Some(depth) = word_list(e).filter(|_| !self.inline) {
                self.flush();
                return self.word_item(e, depth, &look);
            }
        }
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if !self.inline => {
                self.flush();
                let mut sub = self.sub(true);
                sub.walk(e, &look);
                self.adopt(&mut sub);
                if sub.spans.iter().any(|s| !s.text.trim().is_empty()) {
                    self.blocks.push(Block::Heading(name.as_bytes()[1] - b'0', sub.spans));
                }
            }
            "ul" | "ol" if !self.inline => {
                self.flush();
                self.list(e, name == "ol", 0);
            }
            "table" if !self.inline => {
                self.flush();
                self.table(e, &look);
            }
            "blockquote" if !self.inline => {
                self.flush();
                let mut sub = self.sub(true);
                sub.walk(e, &look);
                self.adopt(&mut sub);
                if sub.spans.iter().any(|s| !s.text.trim().is_empty()) {
                    self.blocks.push(Block::Quote(sub.spans));
                }
            }
            "pre" if !self.inline => {
                self.flush();
                let text = raw_text(e);
                if !text.trim().is_empty() {
                    self.blocks.push(Block::Code(text));
                }
            }
            "hr" if !self.inline => {
                self.flush();
                self.blocks.push(Block::Rule);
            }
            "br" => self.line_break(),
            "img" => self.picture(e),
            "input" if e.attr("type").is_some_and(|t| t.eq_ignore_ascii_case("checkbox")) => {
                if self.task.is_none() && self.spans.iter().all(|s| s.text.trim().is_empty()) {
                    self.task = Some(e.attr("checked").is_some());
                }
            }
            "strong" | "b" => {
                look.bold = weight(e).unwrap_or(true);
                self.walk(e, &look);
            }
            "em" | "i" | "cite" | "dfn" | "var" => {
                look.italic = true;
                self.walk(e, &look);
            }
            "s" | "strike" | "del" => {
                look.strike = true;
                self.walk(e, &look);
            }
            "code" | "kbd" | "samp" | "tt" => {
                look.code = true;
                self.walk(e, &look);
            }
            "a" => {
                look.link = e.attr("href").and_then(|h| self.resolve(h));
                self.walk(e, &look);
            }
            _ if BLOCKS.contains(&name) || ENDS_P.contains(&name) || name.starts_with('h') && name.len() == 2 => {
                if self.inline {
                    if !self.spans.is_empty() {
                        self.line_break();
                    }
                    self.walk(e, &look);
                } else {
                    self.flush();
                    self.walk(e, &look);
                    self.flush();
                }
            }
            _ => self.walk(e, &look),
        }
    }

    fn list(&mut self, e: &Element, ordered: bool, depth: usize) {
        for li in e.elements() {
            match li.name.as_str() {
                "li" => {}
                // A list put straight inside a list nests under the last item.
                "ul" | "ol" => {
                    self.list(li, li.name == "ol", depth + 1);
                    continue;
                }
                _ => continue,
            }
            // The item's own text, then the lists inside it, one deeper.
            let mut sub = self.sub(true);
            let mut nested = Vec::new();
            for n in &li.children {
                match n {
                    Node::Element(c) if c.name == "ul" || c.name == "ol" => nested.push(c),
                    Node::Element(c) => sub.element(c, &Look::default()),
                    Node::Text(t) => sub.text(t, &Look::default()),
                }
            }
            self.adopt(&mut sub);
            let task = sub.task;
            let spans = trim_breaks(sub.spans);
            self.blocks.push(Block::Item { ordered, depth, task, spans });
            for n in nested {
                self.list(n, n.name == "ol", depth + 1);
            }
        }
    }

    /// A Word or Outlook list paragraph as a list item: numbered or not as
    /// its own marker is, which is left out of its text.
    fn word_item(&mut self, e: &Element, depth: usize, look: &Look) {
        let marker = word_marker(e);
        let mut sub = self.sub(true);
        sub.walk(e, look);
        let mut spans = trim_breaks(sub.spans);
        let ordered = match marker {
            Some(m) => is_numbered(&m),
            // Older Word: the marker is the item's first word.
            None => {
                let first = spans.first().map(|s| s.text.trim_start().to_string()).unwrap_or_default();
                let word = first.split_whitespace().next().unwrap_or("").to_string();
                if is_marker(&word) {
                    if let Some(s) = spans.first_mut() {
                        s.text = first[word.len()..].trim_start().to_string();
                    }
                    is_numbered(&word)
                } else {
                    false
                }
            }
        };
        if spans.iter().any(|s| !s.text.trim().is_empty()) {
            self.blocks.push(Block::Item { ordered, depth, task: None, spans });
        }
    }

    fn table(&mut self, e: &Element, look: &Look) {
        let mut rows = Vec::new();
        e.find_all("tr", &mut rows);
        let cells = |r: &Element| r.elements().filter(|c| c.name == "td" || c.name == "th").count();
        let widest = rows.iter().map(|r| cells(r)).max().unwrap_or(0);
        let mut nested = Vec::new();
        for r in &rows {
            r.find_all("table", &mut nested);
        }
        // A table that only lays a page out is read as its text.
        if rows.len() < 2 || widest < 2 || !nested.is_empty() {
            self.walk(e, look);
            self.flush();
            return;
        }
        let mut table = Vec::new();
        for r in rows {
            let mut row = Vec::new();
            for c in r.elements().filter(|c| c.name == "td" || c.name == "th") {
                let mut sub = self.sub(true);
                sub.walk(c, &Look { bold: false, ..look.clone() });
                self.adopt(&mut sub);
                row.push(trim_breaks(sub.spans));
                let span = c.attr("colspan").and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 50);
                row.extend(std::iter::repeat_n(Vec::new(), span - 1));
            }
            table.push(row);
        }
        self.blocks.push(Block::Table(table));
    }

    fn picture(&mut self, e: &Element) {
        let src = e.attr("src").filter(|s| !s.trim().is_empty() && !s.starts_with("data:") || e.attr("data-src").is_none());
        let src = src.or_else(|| e.attr("data-src")).unwrap_or("").trim();
        let alt = collapse(e.attr("alt").unwrap_or("")).trim().to_string();
        // Pasted: tracking pixels and spacers are not pictures.
        let tiny = |a: &str| e.attr(a).and_then(|v| v.trim().trim_end_matches("px").parse::<u32>().ok()).is_some_and(|n| n <= 2);
        if self.paste && (tiny("width") || tiny("height")) {
            return;
        }
        let name = if let Some(data) = src.strip_prefix("data:") {
            let Some((mime, b64)) = data.split_once(";base64,") else { return };
            let Some(bytes) = base64(b64) else { return };
            let ext = mime.strip_prefix("image/").map(|m| m.split('+').next().unwrap_or(m)).unwrap_or("png").replace("jpeg", "jpg");
            let name = self.picture_name(&ext);
            self.pictures.push(Picture { name: name.clone(), data: bytes });
            name
        } else if self.paste {
            // Copied from Word or Outlook: a file they keep for the moment;
            // from a page: its address. Saved by whoever pastes.
            let source = match src.strip_prefix("file:///").or_else(|| src.strip_prefix("file://")) {
                Some(path) => format!("file:{}", percent_decode(path)),
                None => match self.resolve(src) {
                    Some(url) if url.starts_with("http") => url,
                    _ => return,
                },
            };
            let ext = picture_ext(&source);
            let name = self.picture_name(ext);
            self.fetch.push((name.clone(), source));
            name
        } else if src.contains("://") || src.starts_with("//") {
            match self.resolve(src) {
                Some(url) => url,
                None => return,
            }
        } else if !src.is_empty() {
            // Beside the page (a saved page's folder): resolved by who
            // reads the file, or else on the web where the page came from.
            let path = percent_decode(src.split(['?', '#']).next().unwrap_or(src));
            let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
            if name.is_empty() {
                return;
            }
            self.beside.push((name.clone(), path.clone()));
            name
        } else {
            return;
        };
        if self.inline {
            // Inside a heading or cell a picture is its words.
            if !alt.is_empty() {
                self.text(&alt, &Look::default());
            }
            return;
        }
        self.flush();
        self.blocks.push(Block::Picture { name, alt });
    }

    /// A link's address as it works outside the page: absolute, or made so
    /// from the page's own address; none for scripts and in-page anchors.
    fn resolve(&self, href: &str) -> Option<String> {
        let href = href.trim();
        let lower = href.to_ascii_lowercase();
        if href.is_empty() || href.starts_with('#') || lower.starts_with("javascript:") || lower.starts_with("data:") {
            return None;
        }
        if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
            return Some(href.to_string());
        }
        let base = self.base.as_deref()?;
        let scheme_end = base.find("://")?;
        if let Some(rest) = href.strip_prefix("//") {
            return Some(format!("{}://{rest}", &base[..scheme_end]));
        }
        let host_end = base[scheme_end + 3..].find('/').map_or(base.len(), |i| scheme_end + 3 + i);
        let origin = &base[..host_end];
        let path = if href.starts_with('/') {
            href.to_string()
        } else {
            let dir = base[host_end..].split(['?', '#']).next().unwrap_or("");
            let dir = &dir[..dir.rfind('/').map_or(0, |i| i + 1)];
            format!("{}{href}", if dir.is_empty() { "/" } else { dir })
        };
        // Dot segments resolved.
        let mut parts: Vec<&str> = Vec::new();
        for seg in path.split('/') {
            match seg {
                "." => {}
                ".." => {
                    if parts.len() > 1 {
                        parts.pop();
                    }
                }
                s => parts.push(s),
            }
        }
        Some(format!("{origin}{}", parts.join("/")))
    }
}

/// A picture's file extension, from its address or path: as written when
/// it names a picture, else `png`.
fn picture_ext(source: &str) -> &'static str {
    let path = source.split(['?', '#']).next().unwrap_or(source).to_ascii_lowercase();
    let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "jpg" | "jpeg" => "jpg",
        "gif" => "gif",
        "webp" => "webp",
        "bmp" => "bmp",
        "svg" => "svg",
        _ => "png",
    }
}

fn hidden(e: &Element) -> bool {
    let style = e.attr("style").unwrap_or("").to_ascii_lowercase().replace(' ', "");
    e.attr("hidden").is_some()
        || e.attr("aria-hidden") == Some("true")
        || style.contains("display:none")
        || style.contains("visibility:hidden")
        || matches!(e.attr("role"), Some("navigation" | "banner" | "contentinfo" | "complementary" | "search"))
}

/// Line breaks at the ends of a run of spans go.
fn trim_breaks(mut spans: Vec<Span>) -> Vec<Span> {
    while spans.last().is_some_and(|s| s.text.trim().is_empty()) {
        spans.pop();
    }
    while spans.first().is_some_and(|s| s.text.trim().is_empty()) {
        spans.remove(0);
    }
    spans
}

/// Whitespace as a browser shows it: any run of it, one space.
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

/// A `<pre>`'s text as written, line breaks and all.
fn raw_text(e: &Element) -> String {
    let mut out = String::new();
    fn walk(e: &Element, out: &mut String) {
        for n in &e.children {
            match n {
                Node::Text(t) => out.push_str(t),
                Node::Element(c) if c.name == "br" => out.push('\n'),
                Node::Element(c) => walk(c, out),
            }
        }
    }
    walk(e, &mut out);
    out.trim_start_matches('\n').trim_end().to_string()
}

/// The page's text, in the encoding it is written in.
fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], le: bool| {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) }).collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return utf16(rest, false);
    }
    if std::str::from_utf8(bytes).is_ok() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    // Not UTF-8: as its <meta> says, if it says a Western one.
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    let western = ["windows-1252", "iso-8859-1", "latin1", "cp1252", "us-ascii", "iso-8859-15"].iter().any(|c| head.contains(c));
    if western || !head.contains("charset") {
        return bytes.iter().map(|&b| crate::cp1252(b)).collect();
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// The page as a tree, as a browser forgives it.
pub(crate) fn parse(s: &str) -> Element {
    let mut stack: Vec<Element> = vec![Element { name: "#root".into(), ..Element::default() }];
    let lower = s.to_ascii_lowercase();
    let mut i = 0;
    let close_to = |stack: &mut Vec<Element>, depth: usize| {
        while stack.len() > depth {
            let done = stack.pop().expect("deeper than the root");
            stack.last_mut().expect("the root").children.push(Node::Element(done));
        }
    };
    while i < s.len() {
        let Some(lt) = s[i..].find('<').map(|k| i + k) else {
            text_into(&mut stack, &s[i..]);
            break;
        };
        if lt > i {
            text_into(&mut stack, &s[i..lt]);
        }
        let rest = &s[lt..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map_or(s.len(), |k| lt + k + 3);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            i = rest.find('>').map_or(s.len(), |k| lt + k + 1);
            continue;
        }
        if let Some(close) = rest.strip_prefix("</") {
            let end = close.find('>').unwrap_or(close.len());
            let name = close[..end].trim().to_ascii_lowercase();
            i = lt + 2 + end + 1;
            if name == "br" {
                stack.last_mut().expect("the root").children.push(Node::Element(Element { name, ..Element::default() }));
                continue;
            }
            if let Some(depth) = stack.iter().rposition(|e| e.name == name).filter(|&d| d > 0) {
                close_to(&mut stack, depth);
            }
            continue;
        }
        let starts_tag = rest[1..].chars().next().is_some_and(|c| c.is_ascii_alphabetic());
        let Some(end) = starts_tag.then(|| tag_end(rest)).flatten() else {
            text_into(&mut stack, "<");
            i = lt + 1;
            continue;
        };
        let inner = rest[1..end].trim_end_matches('/');
        let (name, attrs) = name_and_attrs(inner);
        i = lt + end + 1;
        // What a start tag ends.
        let open = |stack: &Vec<Element>, names: &[&str], fence: &[&str]| -> Option<usize> {
            for (d, e) in stack.iter().enumerate().rev() {
                if names.contains(&e.name.as_str()) {
                    return Some(d);
                }
                if fence.contains(&e.name.as_str()) {
                    return None;
                }
            }
            None
        };
        let ends: Option<usize> = match name.as_str() {
            n if ENDS_P.contains(&n) && open(&stack, &["p"], &["table", "td", "th", "li", "button"]).is_some() => {
                open(&stack, &["p"], &["table", "td", "th", "li", "button"])
            }
            "li" => open(&stack, &["li"], &["ul", "ol", "table"]),
            "dt" | "dd" => open(&stack, &["dt", "dd"], &["dl", "table"]),
            "tr" => open(&stack, &["tr"], &["table"]),
            "td" | "th" => open(&stack, &["td", "th"], &["tr", "table"]),
            "thead" | "tbody" | "tfoot" => open(&stack, &["thead", "tbody", "tfoot"], &["table"]),
            "option" => open(&stack, &["option"], &["select"]),
            _ => None,
        };
        if let Some(depth) = ends {
            close_to(&mut stack, depth);
        }
        // A heading inside a heading ends the first.
        if name.len() == 2
            && name.starts_with('h')
            && name.as_bytes()[1].is_ascii_digit()
            && let Some(depth) = open(&stack, &["h1", "h2", "h3", "h4", "h5", "h6"], &["table", "div", "section", "article"])
        {
            close_to(&mut stack, depth);
        }
        let mut element = Element { name: name.clone(), attrs, children: Vec::new() };
        if VOID.contains(&name.as_str()) {
            stack.last_mut().expect("the root").children.push(Node::Element(element));
        } else if RAW.contains(&name.as_str()) {
            let close = format!("</{name}");
            let end = lower[i..].find(&close).map_or(s.len(), |k| i + k);
            let content = &s[i..end];
            let content = if name == "title" || name == "textarea" { entities(content) } else { content.to_string() };
            element.children.push(Node::Text(content));
            stack.last_mut().expect("the root").children.push(Node::Element(element));
            i = lower[end..].find('>').map_or(s.len(), |k| end + k + 1);
        } else {
            stack.push(element);
        }
    }
    close_to(&mut stack, 1);
    stack.pop().expect("the root")
}

fn text_into(stack: &mut [Element], raw: &str) {
    if let Some(top) = stack.last_mut() {
        top.children.push(Node::Text(entities(raw)));
    }
}

/// Where a tag ends: its `>`, outside quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices().skip(1) {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            (None, '<') => return None,
            _ => {}
        }
    }
    None
}

fn name_and_attrs(inner: &str) -> (String, Vec<(String, String)>) {
    let name_end = inner.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(inner.len());
    let name = inner[..name_end].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let chars: Vec<char> = inner[name_end..].chars().collect();
    let mut k = 0;
    while k < chars.len() {
        while k < chars.len() && (chars[k].is_whitespace() || chars[k] == '/') {
            k += 1;
        }
        let start = k;
        while k < chars.len() && !chars[k].is_whitespace() && chars[k] != '=' && chars[k] != '/' {
            k += 1;
        }
        if k == start {
            break;
        }
        let key: String = chars[start..k].iter().collect::<String>().to_ascii_lowercase();
        while k < chars.len() && chars[k].is_whitespace() {
            k += 1;
        }
        let mut value = String::new();
        if k < chars.len() && chars[k] == '=' {
            k += 1;
            while k < chars.len() && chars[k].is_whitespace() {
                k += 1;
            }
            if k < chars.len() && (chars[k] == '"' || chars[k] == '\'') {
                let q = chars[k];
                k += 1;
                let vs = k;
                while k < chars.len() && chars[k] != q {
                    k += 1;
                }
                value = chars[vs..k.min(chars.len())].iter().collect();
                k += 1;
            } else {
                let vs = k;
                while k < chars.len() && !chars[k].is_whitespace() {
                    k += 1;
                }
                value = chars[vs..k].iter().collect();
            }
        }
        attrs.push((key, entities(&value)));
    }
    (name, attrs)
}

/// Replaces character references: numbers, and the named ones pages use.
pub(crate) fn entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let end = after.find(|c: char| !c.is_ascii_alphanumeric() && c != '#').unwrap_or(after.len());
        let name = &after[..end];
        let c = if let Some(num) = name.strip_prefix('#') {
            let n = if let Some(h) = num.strip_prefix(['x', 'X']) { u32::from_str_radix(h, 16).ok() } else { num.parse().ok() };
            n.map(|n| match n {
                // Windows-1252's characters, as browsers read these numbers.
                0x80..=0x9F => crate::cp1252(n as u8),
                _ => char::from_u32(n).unwrap_or('\u{FFFD}'),
            })
        } else {
            named(name)
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = after[end..].strip_prefix(';').unwrap_or(&after[end..]);
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn named(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "sbquo" => '‚',
        "ldquo" => '“',
        "rdquo" => '”',
        "bdquo" => '„',
        "laquo" => '«',
        "raquo" => '»',
        "lsaquo" => '‹',
        "rsaquo" => '›',
        "bull" => '•',
        "middot" => '·',
        "deg" => '°',
        "plusmn" => '±',
        "times" => '×',
        "divide" => '÷',
        "frac12" => '½',
        "frac14" => '¼',
        "frac34" => '¾',
        "sup2" => '²',
        "sup3" => '³',
        "micro" => 'µ',
        "para" => '¶',
        "sect" => '§',
        "euro" => '€',
        "pound" => '£',
        "yen" => '¥',
        "cent" => '¢',
        "curren" => '¤',
        "iexcl" => '¡',
        "iquest" => '¿',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        "harr" => '↔',
        "check" => '✓',
        "dagger" => '†',
        "Dagger" => '‡',
        "permil" => '‰',
        "prime" => '′',
        "Prime" => '″',
        "ne" => '≠',
        "le" => '≤',
        "ge" => '≥',
        "infin" => '∞',
        "shy" => '\u{AD}',
        "zwj" => '\u{200D}',
        "zwnj" => '\u{200C}',
        "lrm" => '\u{200E}',
        "rlm" => '\u{200F}',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "thinsp" => '\u{2009}',
        "eacute" => 'é',
        "egrave" => 'è',
        "ecirc" => 'ê',
        "aacute" => 'á',
        "agrave" => 'à',
        "acirc" => 'â',
        "auml" => 'ä',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "Auml" => 'Ä',
        "Ouml" => 'Ö',
        "Uuml" => 'Ü',
        "szlig" => 'ß',
        "ccedil" => 'ç',
        "ntilde" => 'ñ',
        "iacute" => 'í',
        "oacute" => 'ó',
        "uacute" => 'ú',
        _ => return None,
    })
}

/// Decodes base64, ignoring whitespace; None if it is not base64.
pub(crate) fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(html: &str) -> String {
        convert(html.as_bytes()).unwrap().markdown
    }

    #[test]
    fn forgives_markup_as_a_browser_does() {
        let m = md("<p>one<p>two<ul><li>a<li>b</ul><table><tr><td>x<td>y<tr><td>1<td>2</table>");
        assert_eq!(m, "one\n\ntwo\n\n- a\n- b\n\n| x | y |\n| --- | --- |\n| 1 | 2 |\n");
        assert_eq!(md("<p>a < b &amp;&amp; c &gt; d &unknown; &#169; &#x2014; &#150;</p>"), "a < b && c > d &unknown; © — –\n");
    }

    #[test]
    fn keeps_the_article_not_the_page() {
        let page = "<body><header><nav><a href='/'>Home</a></nav></header><main><h1>Title</h1><p>Text <a href='/x'>rel</a>.</p></main><footer>Foot</footer></body>";
        assert_eq!(md(page), "# Title\n\nText rel.\n");
        let based = "<head><link rel=canonical href='https://site.com/blog/post'></head><body><p><a href='../about'>About</a> <a href='/x?y=1'>X</a> <a href='#top'>Top</a> <a href='javascript:go()'>Go</a></p></body>";
        assert_eq!(md(based), "[About](https://site.com/about) [X](https://site.com/x?y=1) Top Go\n");
        // A main its script fills: the page as it is without it.
        let scripted = "<body><h1>Index</h1><p>12 notes</p><main></main><script>render()</script></body>";
        assert_eq!(md(scripted), "# Index\n\n12 notes\n");
    }

    #[test]
    fn tasks_layout_tables_and_pictures() {
        assert_eq!(md("<ul><li><input type=checkbox checked> done</li><li><label><input type=checkbox> todo</label></li></ul>"), "- [x] done\n- [ ] todo\n");
        assert_eq!(md("<table><tr><td><p>Just a</p><p>layout</p></td></tr></table>"), "Just a\n\nlayout\n");
        let n = convert(b"<p>Look: <img src='data:image/png;base64,iVBORw0KGgo=' alt='A dot'> and <img src='pics/cover%201.png'></p>").unwrap();
        assert_eq!(n.markdown, "Look:\n\n![[picture 1.png]]\n\nand\n\n![[cover 1.png]]\n");
        assert_eq!(n.pictures[0].data, b"\x89PNG\r\n\x1a\n");
        assert_eq!(n.beside, [("cover 1.png".to_string(), "pics/cover 1.png".to_string())]);
    }

    #[test]
    fn reads_the_sample_page() {
        let n = convert(include_bytes!("../testdata/sample.html")).unwrap();
        assert_eq!(n.title.as_deref(), Some("Launch plan"));
        let m = n.markdown;
        for want in [
            "# Launch plan\n\nShip the **public beta** on *October 12*, see [the post](https://notsy.app/beta).",
            "## What ships\n\n- Live Markdown\n- Pictures\n    - pasted\n    - dropped\n1. Write the notes\n2. Record the video",
            "| Item | Owner | Due |\n| --- | --- | --- |\n| Press kit | Sam | Oct 5 |",
            "![[cover.png]]",
            "> Speed is a feature.",
            "```\ncargo build --release\nnotsy --trace\n```",
            "Entities: & \\<tags> © € — and a line\nbreak.",
        ] {
            assert!(m.contains(want), "missing {want:?} in:\n{m}");
        }
        assert!(!m.contains("Home") && !m.contains("never") && !m.contains("tracking") && !m.contains("Footer"), "page furniture in:\n{m}");
    }
}

#[cfg(test)]
mod paste_tests {
    use super::paste;

    #[test]
    fn google_docs_pastes_with_its_looks_not_its_wrapper() {
        // Docs wraps what is copied in a <b> it says is not bold, and sets
        // weights and slants in styles.
        let html = r#"<meta charset="utf-8"><b style="font-weight:normal;" id="docs-internal-guid-1"><h2 dir="ltr"><span style="font-weight:400">Launch</span></h2><p dir="ltr"><span style="font-weight:700">Ship</span><span style="font-weight:400"> on </span><span style="font-style:italic;font-weight:400">Friday</span></p><ul><li dir="ltr"><p dir="ltr"><span style="font-weight:400">milk</span></p></li><li><p><span style="text-decoration:line-through">eggs</span></p></li></ul></b>"#;
        assert_eq!(paste(html, None).unwrap().markdown, "## Launch\n\n**Ship** on *Friday*\n\n- milk\n- ~~eggs~~");
    }

    #[test]
    fn word_and_outlook_list_paragraphs_paste_as_lists() {
        let item = |level: u8, marker: &str, text: &str| {
            format!(
                "<p class=MsoListParagraphCxSpMiddle style='margin-left:.5in;text-indent:-.25in;mso-list:l0 level{level} lfo1'><![if !supportLists]><span style='font-family:Symbol'><span style='mso-list:Ignore'>{marker}<span style='font:7.0pt \"Times New Roman\"'>&nbsp;&nbsp;&nbsp;&nbsp; </span></span></span><![endif]>{text}<o:p></o:p></p>"
            )
        };
        let html = format!(
            "<html xmlns:o=\"urn:schemas-microsoft-com:office:office\"><head><style>p.MsoNormal {{margin:0in}}</style></head><body lang=EN-US><div class=WordSection1><p class=MsoNormal><b>Plan</b><o:p></o:p></p>{}{}{}</div></body></html>",
            item(1, "·", "Pricing page"),
            item(2, "o", "FAQ"),
            item(1, "1.", "Press kit")
        );
        let m = paste(&html, None).unwrap().markdown;
        let lines: Vec<&str> = m.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines[0], "**Plan**");
        assert_eq!(lines[1], "- Pricing page");
        assert!(lines[2].trim_start() == "- FAQ" && lines[2].starts_with([' ', '\t']), "{m}");
        assert_eq!(lines[3], "1. Press kit");
        // Older Word: the bullet is the paragraph's first word, unmarked.
        let old = "<p style='mso-list:l1 level1 lfo2'><span style='font-family:Symbol'>·<span>&nbsp;&nbsp; </span></span>Call Sam</p><p style='mso-list:l1 level1 lfo2'><span>2)&nbsp;&nbsp;</span>Book room</p>";
        assert_eq!(paste(old, None).unwrap().markdown, "- Call Sam\n1. Book room");
    }

    #[test]
    fn a_web_pages_table_links_and_headings_paste_as_markdown() {
        let html = r#"<html><body><!--StartFragment--><h3>Costs</h3><table><tr><th>Item</th><th>Cost</th></tr><tr><td>Kit</td><td>120</td></tr></table><p>See the <a href="/docs/pricing">pricing docs</a>.</p><nav><a href="https://example.com/">Home</a></nav><img src="https://example.com/logo.png" alt="logo"><!--EndFragment--></body></html>"#;
        let p = paste(html, Some("https://example.com/blog/post")).unwrap();
        let m = &p.markdown;
        assert!(m.starts_with("### Costs\n\n| Item | Cost |"), "{m}");
        assert!(m.contains("| Kit  | 120  |") || m.contains("| Kit | 120 |"), "{m}");
        assert!(m.contains("[pricing docs](https://example.com/docs/pricing)"), "{m}");
        // What was copied is kept, navigation too; its picture is embedded,
        // to be fetched from the page's address.
        assert!(m.contains("[Home](https://example.com/)") && m.ends_with("![[picture 1.png]]"), "{m}");
        assert_eq!(p.fetch, [("picture 1.png".to_string(), "https://example.com/logo.png".to_string())]);
    }

    #[test]
    fn pictures_come_along_from_word_pages_and_inside_them() {
        // Word and Outlook: a file of their own for the moment; a page: its
        // address (relative ones made whole); inside: the picture itself.
        // Tracking pixels stay out, and no two pictures share a name, in
        // lists or not.
        let html = r#"<p>Chart:</p><p><img width=320 height=200 src="file:///C:/Users/me/AppData/Local/Temp/msohtmlclip1/01/clip_image001.png" alt="chart"></p><ul><li>Logo <img src="/img/logo.JPG?v=2"></li></ul><p><img src="data:image/png;base64,iVBORw0KGgo="><img src="https://t.example/pixel.gif" width="1" height="1"></p>"#;
        let p = paste(html, Some("https://example.com/a/b")).unwrap();
        assert_eq!(
            p.fetch,
            [
                ("picture 1.png".to_string(), "file:C:/Users/me/AppData/Local/Temp/msohtmlclip1/01/clip_image001.png".to_string()),
                ("picture 2.jpg".to_string(), "https://example.com/img/logo.JPG?v=2".to_string()),
            ]
        );
        assert_eq!(p.pictures.len(), 1);
        assert_eq!(p.pictures[0].name, "picture 3.png");
        assert!(p.markdown.contains("![[picture 1.png]]") && p.markdown.contains("![[picture 3.png]]"), "{}", p.markdown);
        // A picture alone is worth pasting.
        assert!(paste(r#"<img src="https://example.com/cat.webp">"#, None).is_some());
    }

    #[test]
    fn plain_text_and_code_from_an_editor_paste_as_they_are() {
        // Nothing plain text lacks: the copied text pastes, not Markdown.
        assert!(paste("<p>Just words.</p><p>More words.</p>", None).is_none());
        assert!(paste("<span style=\"color:#333\">one line</span>", None).is_none());
        // Code from VS Code: spans of colour (some bold) in an element that
        // keeps its spaces.
        let code = r#"<meta charset='utf-8'><div style="color: #d4d4d4;background-color: #1e1e1e;font-family: Consolas, 'Courier New', monospace;white-space: pre;"><div><span style="color: #569cd6;font-weight: bold;">fn</span><span> main() {</span></div><div><span>    run();</span></div></div>"#;
        assert!(paste(code, None).is_none());
        // A page's own code block is code.
        assert_eq!(paste("<p>Run:</p><pre><code>cargo test\n  --release</code></pre>", None).unwrap().markdown, "Run:\n\n```\ncargo test\n  --release\n```");
    }
}
