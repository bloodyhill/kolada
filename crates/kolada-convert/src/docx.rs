//! Word documents (DOCX) into Markdown. A DOCX is a zip of XML parts: the
//! document, its styles (which say what is a heading), its numbering (which
//! says what is a bulleted or numbered list), its relationships (where links
//! and pictures point) and its footnotes.
//!
//! As mammoth (behind MarkItDown) reads it, with these changes: the Title
//! style and outline levels are headings, styles inherit heading levels
//! through `basedOn`, nested lists keep their depth, checkboxes make tasks,
//! footnotes become Markdown footnotes, pictures become files, and a first
//! table row is the header (never an empty row above it).

use std::collections::HashMap;

use crate::md::{self, Block, Span};
use crate::xml::{self, Element, Node};
use crate::zip::Zip;
use crate::{Error, Note, Picture};

/// A style: its name, the style it is based on, its outline level, its
/// list numbering, and whether its runs are bold or italic.
#[derive(Default, Clone)]
struct Style {
    name: String,
    based_on: Option<String>,
    outline: Option<u8>,
    num: Option<(String, u8)>,
    /// Its left indent, twips.
    indent: Option<i32>,
    bold: bool,
    italic: bool,
}

struct Doc<'a> {
    zip: &'a Zip<'a>,
    styles: HashMap<String, Style>,
    /// numId → abstractNumId, and abstractNumId → each level's format and
    /// left indent (twips).
    nums: HashMap<String, String>,
    formats: HashMap<String, Vec<(String, Option<i32>)>>,
    /// The indents of the list being read, outermost first: how deep an
    /// item is (lists made with Word's List Bullet 2 styles are at level 0
    /// of their own numbering, and only their indent says they are nested).
    indents: Vec<i32>,
    /// Relationship ids of the part being read → (target, external).
    rels: HashMap<String, (String, bool)>,
    /// The document part's folder, which relationship targets are under.
    dir: String,
    pictures: Vec<Picture>,
    /// Pictures already read, by their part: the name they were given.
    read: HashMap<String, String>,
    footnotes: HashMap<String, Element>,
    /// Footnotes in the order the text refers to them: (id, number).
    noted: Vec<String>,
}

pub fn convert(bytes: &[u8]) -> Result<Note, Error> {
    let zip = Zip::open(bytes)?;
    // The main part, as the package's relationships name it.
    let main = zip
        .read("_rels/.rels")
        .and_then(Result::ok)
        .and_then(|b| {
            let rels = xml::parse(&b);
            let main = rels.elements().find(|e| e.name == "Relationship" && e.attr("Type").is_some_and(|t| t.ends_with("/officeDocument")))?;
            Some(main.attr("Target")?.trim_start_matches('/').to_string())
        })
        .unwrap_or_else(|| "word/document.xml".to_string());
    let document = zip.read(&main).ok_or_else(|| Error("not a Word document: it has no document part".to_string()))??;
    let dir = main.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
    let part = |name: &str| zip.read(&format!("{dir}{name}")).and_then(Result::ok);
    let rels = part(&format!("_rels/{}.rels", main.rsplit('/').next().unwrap_or("document.xml"))).map(|b| relationships(&b)).unwrap_or_default();
    let mut doc = Doc {
        zip: &zip,
        styles: part("styles.xml").map(|b| styles(&xml::parse(&b))).unwrap_or_default(),
        nums: HashMap::new(),
        formats: HashMap::new(),
        rels,
        dir: dir.clone(),
        pictures: Vec::new(),
        read: HashMap::new(),
        footnotes: HashMap::new(),
        noted: Vec::new(),
        indents: Vec::new(),
    };
    if let Some(b) = part("numbering.xml") {
        numbering(&xml::parse(&b), &mut doc);
    }
    if let Some(b) = part("footnotes.xml") {
        for f in xml::parse(&b).elements().filter(|e| e.name == "w:footnote") {
            if !matches!(f.attr("w:type"), Some("separator" | "continuationSeparator" | "continuationNotice"))
                && let Some(id) = f.attr("w:id")
            {
                doc.footnotes.insert(id.to_string(), f.clone());
            }
        }
    }
    let root = xml::parse(&document);
    let body = root.child("w:body").ok_or_else(|| Error("not a Word document: it has no body".to_string()))?;
    let mut blocks = Vec::new();
    doc.blocks(body, &mut blocks);
    // Footnotes, as Markdown footnotes, at the end.
    let noted = std::mem::take(&mut doc.noted);
    for (n, id) in noted.iter().enumerate() {
        let Some(f) = doc.footnotes.get(id).cloned() else { continue };
        let mut inner = Vec::new();
        doc.blocks(&f, &mut inner);
        let mut spans = vec![Span::raw(format!("[^{}]: ", n + 1))];
        for (k, b) in inner.into_iter().enumerate() {
            if let Block::Paragraph(s) | Block::Heading(_, s) | Block::Quote(s) | Block::Item { spans: s, .. } = b {
                if k > 0 {
                    spans.push(Span::plain(" "));
                }
                spans.extend(s);
            }
        }
        blocks.push(Block::Paragraph(spans));
    }
    let title = zip
        .read("docProps/core.xml")
        .and_then(Result::ok)
        .map(|b| xml::parse(&b))
        .and_then(|c| c.find("dc:title").map(|t| t.text().trim().to_string()))
        .filter(|t| !t.is_empty())
        .or_else(|| {
            blocks.iter().find_map(|b| match b {
                Block::Heading(_, s) => Some(md::inline(s)),
                _ => None,
            })
        });
    if blocks.is_empty() {
        return Err(Error("the document has no text".to_string()));
    }
    Ok(Note { title, markdown: md::write(&blocks), pictures: doc.pictures, beside: Vec::new() })
}

/// A relationships part: id → (target, external).
fn relationships(bytes: &[u8]) -> HashMap<String, (String, bool)> {
    xml::parse(bytes)
        .elements()
        .filter(|e| e.name == "Relationship")
        .filter_map(|e| Some((e.attr("Id")?.to_string(), (e.attr("Target")?.to_string(), e.attr("TargetMode") == Some("External")))))
        .collect()
}

/// Whether an on/off property is on: present, and not set to false.
fn on(e: Option<&Element>) -> bool {
    e.is_some_and(|e| !matches!(e.attr("w:val"), Some("false" | "0" | "off" | "none")))
}

fn styles(root: &Element) -> HashMap<String, Style> {
    let mut out = HashMap::new();
    for s in root.elements().filter(|e| e.name == "w:style") {
        let Some(id) = s.attr("w:styleId") else { continue };
        if out.contains_key(id) {
            continue;
        }
        let ppr = s.child("w:pPr");
        let num = ppr.and_then(|p| p.child("w:numPr")).and_then(|n| {
            let id = n.child("w:numId")?.attr("w:val")?.to_string();
            let level = n.child("w:ilvl").and_then(|l| l.attr("w:val")).and_then(|v| v.parse().ok()).unwrap_or(0);
            Some((id, level))
        });
        let rpr = s.child("w:rPr");
        let indent = ppr.and_then(indent_of);
        out.insert(
            id.to_string(),
            Style {
                name: s.child("w:name").and_then(|n| n.attr("w:val")).unwrap_or(id).to_string(),
                based_on: s.child("w:basedOn").and_then(|b| b.attr("w:val")).map(str::to_string),
                outline: ppr.and_then(|p| p.child("w:outlineLvl")).and_then(|o| o.attr("w:val")).and_then(|v| v.parse().ok()),
                num,
                indent,
                bold: on(rpr.and_then(|r| r.child("w:b"))),
                italic: on(rpr.and_then(|r| r.child("w:i"))),
            },
        );
    }
    out
}

fn numbering(root: &Element, doc: &mut Doc) {
    for a in root.elements().filter(|e| e.name == "w:abstractNum") {
        let Some(id) = a.attr("w:abstractNumId") else { continue };
        let mut levels = vec![(String::new(), None); 9];
        for l in a.elements().filter(|e| e.name == "w:lvl") {
            if let Some(i) = l.attr("w:ilvl").and_then(|v| v.parse::<usize>().ok()).filter(|&i| i < 9) {
                let format = l.child("w:numFmt").and_then(|f| f.attr("w:val")).unwrap_or("decimal").to_string();
                levels[i] = (format, l.child("w:pPr").and_then(indent_of));
            }
        }
        // A list defined by a numbering style: its levels are that style's.
        if let Some(link) = a.child("w:numStyleLink").and_then(|l| l.attr("w:val")) {
            doc.formats.insert(format!("link:{id}"), vec![(link.to_string(), None)]);
        }
        doc.formats.insert(id.to_string(), levels);
    }
    for n in root.elements().filter(|e| e.name == "w:num") {
        if let (Some(id), Some(abs)) = (n.attr("w:numId"), n.child("w:abstractNumId").and_then(|a| a.attr("w:val"))) {
            doc.nums.insert(id.to_string(), abs.to_string());
        }
    }
}

/// A paragraph properties' left indent, twips.
fn indent_of(ppr: &Element) -> Option<i32> {
    let ind = ppr.child("w:ind")?;
    ind.attr("w:left").or(ind.attr("w:start")).and_then(|v| v.parse().ok())
}

/// How a paragraph is written: its kind, and for a list, whether numbered,
/// its level, and its left indent (twips) if anything gives one.
enum Kind {
    Heading(u8),
    Item { ordered: bool, level: usize, indent: Option<i32> },
    Quote,
    Code,
    Plain,
}

impl Doc<'_> {
    /// The style's chain, from `id` up through what it is based on.
    fn chain(&self, id: &str) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut at = Some(id.to_string());
        while let Some(i) = at.take() {
            let Some(s) = self.styles.get(&i) else { break };
            out.push(s);
            if out.len() > 8 {
                break;
            }
            at = s.based_on.clone();
        }
        out
    }

    fn heading_level(&self, style: Option<&str>, ppr: Option<&Element>) -> Option<u8> {
        if let Some(l) = ppr.and_then(|p| p.child("w:outlineLvl")).and_then(|o| o.attr("w:val")).and_then(|v| v.parse::<u8>().ok()) {
            return (l < 6).then_some(l + 1);
        }
        let id = style?;
        for s in self.chain(id) {
            let name = s.name.to_ascii_lowercase();
            if let Some(n) = name.strip_prefix("heading").map(str::trim).and_then(|n| n.parse::<u8>().ok()) {
                return Some(n.clamp(1, 6));
            }
            if name == "title" || name == "heading" {
                return Some(1);
            }
            if let Some(l) = s.outline {
                return (l < 6).then_some(l + 1);
            }
        }
        None
    }

    /// The list a paragraph is in (its own numbering, or its style's):
    /// whether numbered, its level, and its indent.
    fn list(&self, style: Option<&str>, ppr: Option<&Element>) -> Option<(bool, usize, Option<i32>)> {
        let own = ppr.and_then(|p| p.child("w:numPr")).map(|n| {
            let id = n.child("w:numId").and_then(|i| i.attr("w:val")).map(str::to_string);
            let level = n.child("w:ilvl").and_then(|l| l.attr("w:val")).and_then(|v| v.parse().ok());
            (id, level)
        });
        let styled = style.and_then(|s| self.chain(s).into_iter().find_map(|s| s.num.clone()));
        let (id, level) = match (own, styled) {
            (Some((Some(id), level)), styled) => (id, level.or(styled.map(|s| s.1)).unwrap_or(0)),
            (Some((None, Some(level))), Some((id, _))) => (id, level),
            (_, Some((id, level))) => (id, level),
            _ => return None,
        };
        // numId 0 takes a paragraph out of a list.
        if id == "0" {
            return None;
        }
        let mut abs = self.nums.get(&id)?.clone();
        // A list defined by a numbering style.
        if let Some((link, _)) = self.formats.get(&format!("link:{abs}")).and_then(|l| l.first())
            && let Some((num, _)) = self.chain(link).into_iter().find_map(|s| s.num.clone())
            && let Some(a) = self.nums.get(&num)
        {
            abs = a.clone();
        }
        let (format, level_indent) = self.formats.get(&abs).and_then(|l| l.get(level as usize)).cloned().unwrap_or(("bullet".to_string(), None));
        if format == "none" {
            return None;
        }
        // The paragraph's own indent, else its style's, else its level's.
        let styled = style.and_then(|s| self.chain(s).into_iter().find_map(|s| s.indent));
        let indent = ppr.and_then(indent_of).or(styled).or(level_indent);
        Some((format != "bullet", level as usize, indent))
    }

    fn kind(&self, p: &Element) -> Kind {
        let ppr = p.child("w:pPr");
        let style = ppr.and_then(|pp| pp.child("w:pStyle")).and_then(|s| s.attr("w:val"));
        if let Some(level) = self.heading_level(style, ppr) {
            return Kind::Heading(level);
        }
        if let Some((ordered, level, indent)) = self.list(style, ppr) {
            return Kind::Item { ordered, level, indent };
        }
        let names: Vec<String> = style.map(|s| self.chain(s).iter().map(|s| s.name.to_ascii_lowercase()).collect()).unwrap_or_default();
        if names.iter().any(|n| n.contains("quote")) {
            Kind::Quote
        } else if names.iter().any(|n| n.contains("code") || n.contains("preformatted") || n == "plain text") {
            Kind::Code
        } else {
            Kind::Plain
        }
    }

    /// The blocks of a body, cell, footnote or text box.
    fn blocks(&mut self, parent: &Element, out: &mut Vec<Block>) {
        for e in parent.elements() {
            match e.name.as_str() {
                "w:p" => self.paragraph(e, out),
                "w:tbl" => self.table(e, out),
                // Content controls, tracked insertions and custom markup
                // hold blocks too.
                "w:sdt" => {
                    if let Some(c) = e.child("w:sdtContent") {
                        self.blocks(c, out);
                    }
                }
                "w:ins" | "w:customXml" | "w:smartTag" | "w:moveTo" => self.blocks(e, out),
                _ => {}
            }
        }
    }

    fn paragraph(&mut self, p: &Element, out: &mut Vec<Block>) {
        let kind = self.kind(p);
        let mut spans = Vec::new();
        let mut later = Vec::new();
        let mut task = None;
        let mut field = Field::default();
        self.runs(p, &Look::default(), &mut spans, &mut later, out, &mut task, &mut field);
        let has_text = spans.iter().any(|s: &Span| !s.text.trim().is_empty());
        if has_text {
            let spans = trim(spans);
            if !matches!(kind, Kind::Item { .. }) {
                self.indents.clear();
            }
            out.push(match kind {
                Kind::Heading(level) => Block::Heading(level, spans),
                Kind::Item { ordered, level, indent } => {
                    let depth = match indent {
                        // Deeper than the item before it by its indent: one
                        // level in; back to an indent seen: that level.
                        Some(i) => match self.indents.iter().position(|&x| x >= i) {
                            Some(k) => {
                                self.indents.truncate(k);
                                self.indents.push(i);
                                k
                            }
                            None => {
                                self.indents.push(i);
                                self.indents.len() - 1
                            }
                        },
                        None => level,
                    };
                    Block::Item { ordered, depth, task, spans }
                }
                _ if task.is_some() => Block::Item { ordered: false, depth: 0, task, spans },
                Kind::Quote => Block::Quote(spans),
                Kind::Code => {
                    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
                    // Code paragraphs in a row are one block.
                    if let Some(Block::Code(prev)) = out.last_mut() {
                        prev.push('\n');
                        prev.push_str(&text);
                    } else {
                        out.push(Block::Code(text));
                    }
                    out.append(&mut later);
                    return;
                }
                Kind::Plain => Block::Paragraph(spans),
            });
        }
        out.append(&mut later);
    }

    /// The runs of a paragraph (or of what is inside one), as spans.
    /// Pictures and text boxes are blocks of their own, after the text so
    /// far: what came before them is ended into `out`.
    #[allow(clippy::too_many_arguments)]
    fn runs(&mut self, e: &Element, look: &Look, spans: &mut Vec<Span>, later: &mut Vec<Block>, out: &mut Vec<Block>, task: &mut Option<bool>, field: &mut Field) {
        for c in e.elements() {
            match c.name.as_str() {
                "w:r" => self.run(c, look, spans, later, out, field),
                "w:hyperlink" => {
                    let link = c.attr("r:id").and_then(|id| self.rels.get(id)).map(|(t, _)| t.clone()).map(|t| match c.attr("w:anchor") {
                        Some(a) => format!("{t}#{a}"),
                        None => t,
                    });
                    let look = Look { link: link.or(look.link.clone()), ..look.clone() };
                    self.runs(c, &look, spans, later, out, task, field);
                }
                "w:fldSimple" => {
                    let link = hyperlink(c.attr("w:instr").unwrap_or(""));
                    let look = Look { link: link.or(look.link.clone()), ..look.clone() };
                    self.runs(c, &look, spans, later, out, task, field);
                }
                "w:sdt" => {
                    let checkbox = c.child("w:sdtPr").and_then(|p| p.child("w14:checkbox"));
                    if let Some(cb) = checkbox {
                        let checked = cb.child("w14:checked").and_then(|v| v.attr("w14:val")).is_some_and(|v| v == "1" || v == "true");
                        if task.is_none() && spans.iter().all(|s| s.text.trim().is_empty()) {
                            *task = Some(checked);
                        }
                        // Its box glyph is not text.
                        continue;
                    }
                    if let Some(content) = c.child("w:sdtContent") {
                        self.runs(content, look, spans, later, out, task, field);
                    }
                }
                "w:ins" | "w:smartTag" | "w:customXml" | "w:moveTo" | "w:dir" | "w:bdo" => self.runs(c, look, spans, later, out, task, field),
                _ => {}
            }
        }
    }

    fn run(&mut self, r: &Element, look: &Look, spans: &mut Vec<Span>, later: &mut Vec<Block>, out: &mut Vec<Block>, field: &mut Field) {
        let rpr = r.child("w:rPr");
        let mut look = look.clone();
        if let Some(style) = rpr.and_then(|p| p.child("w:rStyle")).and_then(|s| s.attr("w:val")) {
            for s in self.chain(style) {
                look.bold |= s.bold || s.name.eq_ignore_ascii_case("strong");
                look.italic |= s.italic || s.name.eq_ignore_ascii_case("emphasis");
            }
        }
        if let Some(rpr) = rpr {
            if let Some(b) = rpr.child("w:b") {
                look.bold = on(Some(b));
            }
            if let Some(i) = rpr.child("w:i") {
                look.italic = on(Some(i));
            }
            look.strike |= on(rpr.child("w:strike")) || on(rpr.child("w:dstrike"));
        }
        let mut text = String::new();
        let flush = |text: &mut String, spans: &mut Vec<Span>, look: &Look| {
            if !text.is_empty() {
                spans.push(Span { text: std::mem::take(text), bold: look.bold, italic: look.italic, strike: look.strike, link: look.link.clone(), ..Span::default() });
            }
        };
        for c in &r.children {
            let Node::Element(c) = c else { continue };
            match c.name.as_str() {
                "w:fldChar" => match c.attr("w:fldCharType") {
                    Some("begin") => {
                        flush(&mut text, spans, &look);
                        *field = Field { open: true, ..Field::default() };
                    }
                    Some("separate") => {
                        field.link = hyperlink(&field.instr);
                        field.result = true;
                    }
                    Some("end") => {
                        flush(&mut text, spans, &look);
                        *field = Field::default();
                    }
                    _ => {}
                },
                "w:instrText" if field.open && !field.result => field.instr.push_str(&c.text()),
                // A field's instructions are not text.
                _ if field.open && !field.result => {}
                "w:t" => {
                    if field.link.is_some() && look.link.is_none() {
                        flush(&mut text, spans, &look);
                        look.link = field.link.clone();
                    }
                    text.push_str(&c.text());
                }
                "w:tab" | "w:ptab" => text.push(' '),
                "w:br" | "w:cr" => {
                    if !matches!(c.attr("w:type"), Some("page" | "column")) {
                        text.push('\n');
                    }
                }
                "w:noBreakHyphen" => text.push('-'),
                "w:sym" => {
                    if let Some(ch) = c.attr("w:char").and_then(|h| u32::from_str_radix(h, 16).ok()) {
                        // Symbol-font bullets and checks, in the private use area.
                        let ch = match ch {
                            0xF0B7 | 0xF0A7 => '•',
                            0xF0FC => '✓',
                            0xF0FB => '✗',
                            n => char::from_u32(n).filter(|c| !('\u{E000}'..='\u{F8FF}').contains(c)).unwrap_or(' '),
                        };
                        text.push(ch);
                    }
                }
                "w:footnoteReference" => {
                    if let Some(id) = c.attr("w:id") {
                        flush(&mut text, spans, &look);
                        let n = match self.noted.iter().position(|x| x == id) {
                            Some(i) => i + 1,
                            None => {
                                self.noted.push(id.to_string());
                                self.noted.len()
                            }
                        };
                        spans.push(Span::raw(format!("[^{n}]")));
                    }
                }
                "w:drawing" | "w:pict" | "mc:AlternateContent" | "w:object" => {
                    flush(&mut text, spans, &look);
                    self.drawing(c, spans, later, out);
                }
                _ => {}
            }
        }
        flush(&mut text, spans, &look);
    }

    /// A drawing: its pictures, and the text of any text boxes in it.
    fn drawing(&mut self, d: &Element, spans: &mut Vec<Span>, later: &mut Vec<Block>, out: &mut Vec<Block>) {
        // Word's own choice first; the fallback only when it has nothing.
        let d = match d.name.as_str() {
            "mc:AlternateContent" => match d.child("mc:Choice") {
                Some(c) if c.find("a:blip").is_some() || c.find("w:txbxContent").is_some() => c,
                _ => d.child("mc:Fallback").unwrap_or(d),
            },
            _ => d,
        };
        let mut blips = Vec::new();
        d.find_all("a:blip", &mut blips);
        d.find_all("v:imagedata", &mut blips);
        let alt = d.find("wp:docPr").and_then(|p| p.attr("descr").filter(|a| !a.trim().is_empty()).or(p.attr("title"))).unwrap_or("").to_string();
        for b in blips {
            let id = b.attr("r:embed").or(b.attr("r:id")).or(b.attr("r:link"));
            if let Some(name) = id.and_then(|id| self.picture(id)) {
                // Text before the picture is its own paragraph.
                if spans.iter().any(|s| !s.text.trim().is_empty()) {
                    out.push(Block::Paragraph(trim(std::mem::take(spans))));
                } else {
                    spans.clear();
                }
                out.push(Block::Picture { name, alt: alt.clone() });
            }
        }
        let mut boxes = Vec::new();
        d.find_all("w:txbxContent", &mut boxes);
        for b in boxes {
            self.blocks(b, later);
        }
    }

    /// The picture a relationship points to, read into the note's
    /// pictures once: its name.
    fn picture(&mut self, id: &str) -> Option<String> {
        let (target, external) = self.rels.get(id)?.clone();
        if external {
            return None;
        }
        let path = if let Some(abs) = target.strip_prefix('/') { abs.to_string() } else { normalize(&format!("{}{target}", self.dir)) };
        if let Some(name) = self.read.get(&path) {
            return Some(name.clone());
        }
        let data = self.zip.read(&path)?.ok()?;
        let ext = path.rsplit_once('.').map_or("png", |(_, e)| e).to_ascii_lowercase();
        // Word names them image1.png and so on, in every document: numbered
        // here as the note's pictures.
        let name = format!("Picture {}.{ext}", self.pictures.len() + 1);
        self.pictures.push(Picture { name: name.clone(), data });
        self.read.insert(path, name.clone());
        Some(name)
    }

    fn table(&mut self, t: &Element, out: &mut Vec<Block>) {
        let rows: Vec<&Element> = t.elements().filter(|e| e.name == "w:tr").collect();
        let cells = |r: &Element| r.elements().filter(|e| e.name == "w:tc").count();
        let widest = rows.iter().map(|r| cells(r)).max().unwrap_or(0);
        // A table that only lays a page out is read as its text.
        if rows.len() < 2 || widest < 2 {
            for r in rows {
                for c in r.elements().filter(|e| e.name == "w:tc") {
                    self.blocks(c, out);
                }
            }
            return;
        }
        let mut table = Vec::new();
        for r in rows {
            // A deleted row is gone.
            if r.child("w:trPr").and_then(|p| p.child("w:del")).is_some() {
                continue;
            }
            let mut row = Vec::new();
            for c in r.elements().filter(|e| e.name == "w:tc") {
                let props = c.child("w:tcPr");
                let span = props.and_then(|p| p.child("w:gridSpan")).and_then(|g| g.attr("w:val")).and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).clamp(1, 60);
                let merged = props.and_then(|p| p.child("w:vMerge")).is_some_and(|m| m.attr("w:val") != Some("restart"));
                let mut inner = Vec::new();
                if !merged {
                    self.blocks(c, &mut inner);
                }
                let mut spans = Vec::new();
                for (k, b) in inner.into_iter().enumerate() {
                    if k > 0 {
                        spans.push(Span::plain("\n"));
                    }
                    match b {
                        Block::Paragraph(s) | Block::Heading(_, s) | Block::Quote(s) | Block::Item { spans: s, .. } => spans.extend(s),
                        Block::Code(text) => spans.push(Span { code: true, ..Span::plain(text) }),
                        Block::Picture { alt, .. } => spans.push(Span::plain(alt)),
                        _ => {}
                    }
                }
                row.push(spans);
                row.extend(std::iter::repeat_n(Vec::new(), span - 1));
            }
            table.push(row);
        }
        out.push(Block::Table(table));
    }
}

/// How a run looks, inherited from what holds it.
#[derive(Clone, Default)]
struct Look {
    bold: bool,
    italic: bool,
    strike: bool,
    link: Option<String>,
}

/// A complex field being read: its instructions, then its result (a link,
/// when it is a HYPERLINK field).
#[derive(Default)]
struct Field {
    open: bool,
    result: bool,
    instr: String,
    link: Option<String>,
}

/// A HYPERLINK field's address: `HYPERLINK "https://..."`, or an anchor
/// (`\l "name"`), which links nowhere outside the document.
fn hyperlink(instr: &str) -> Option<String> {
    let rest = instr.trim().strip_prefix("HYPERLINK")?.trim();
    if rest.starts_with("\\l") {
        return None;
    }
    let url = rest.trim_start_matches('"').split('"').next()?.trim();
    (!url.is_empty()).then(|| url.to_string())
}

fn trim(mut spans: Vec<Span>) -> Vec<Span> {
    while spans.last().is_some_and(|s| s.text.trim().is_empty() && !s.raw) {
        spans.pop();
    }
    while spans.first().is_some_and(|s| s.text.trim().is_empty() && !s.raw) {
        spans.remove(0);
    }
    spans
}

/// A part's path with `..` resolved.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_sample_document() {
        let n = convert(include_bytes!("../testdata/sample.docx")).unwrap();
        assert_eq!(n.title.as_deref(), Some("Launch plan"));
        let m = n.markdown;
        for want in [
            "# Launch plan\n\nShip the **public beta** on *October 12*, see [the post](https://notsy.app/beta).",
            "## What ships\n\n- Live Markdown\n- Pictures\n    - pasted\n    - dropped\n1. Write the notes\n2. Record the video",
            "| Item | Owner | Due |\n| --- | --- | --- |\n| Press kit | Sam | Oct 5 |\n| Pricing page | Lena | Oct 8 |",
            "A picture of the cover:\n\n![[Picture 1.png]]",
            "\u{0645}\u{0644}\u{0627}\u{062d}\u{0638}\u{0629} \u{0642}\u{0635}\u{064a}\u{0631}\u{0629}",
            "Tabs and line\nbreaks stay.",
        ] {
            assert!(m.contains(want), "missing {want:?} in:\n{m}");
        }
        assert_eq!(n.pictures.len(), 1);
        let zip = Zip::open(include_bytes!("../testdata/sample.docx")).unwrap();
        assert!(zip.names().any(|n| n == "word/document.xml"));
        assert!(n.pictures[0].data.starts_with(b"\x89PNG"));
    }

    #[test]
    fn fields_and_links() {
        assert_eq!(hyperlink(r#" HYPERLINK "https://notsy.app/a b" \o "tip""#).as_deref(), Some("https://notsy.app/a b"));
        assert_eq!(hyperlink(r#"HYPERLINK \l "_Toc1""#), None);
        assert_eq!(hyperlink("PAGE"), None);
        assert_eq!(normalize("word/../media/x.png"), "media/x.png");
    }
}
