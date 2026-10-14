//! Evernote's export, `.enex`: XML holding each note's title, dates, tags,
//! its body as ENML (HTML with Evernote's own `<en-media>` for attachments
//! and `<en-todo>` for checkboxes) and its attachments in base64, which the
//! body points at by the MD5 of their bytes.

use crate::import::{Collection, Entry, file_name_for, millis, unique_in};
use crate::{Error, Picture, html, xml};

/// Stand-ins written into the body for what HTML has no word for, swapped
/// for Markdown once it is converted: an attachment (by its number) and a
/// checkbox. Private-use characters, so no note's own text holds them.
const MEDIA: char = '\u{E000}';
const MEDIA_END: char = '\u{E001}';
const TODO: char = '\u{E002}';
const DONE: char = '\u{E003}';

pub fn read(bytes: &[u8]) -> Result<Collection, Error> {
    let root = xml::parse(bytes);
    let mut notes = Vec::new();
    root.find_all("note", &mut notes);
    if notes.is_empty() && root.find("en-export").is_none() {
        return Err(Error("not an Evernote export".to_string()));
    }
    let mut c = Collection::default();
    for n in notes {
        match note(n) {
            Some(e) => c.notes.push(e),
            None => c.skipped += 1,
        }
    }
    Ok(c)
}

fn note(n: &xml::Element) -> Option<Entry> {
    let field = |name: &str| n.child(name).map(|e| e.text().trim().to_string()).unwrap_or_default();
    let title = field("title");
    // Attachments, and the hash the body knows each by.
    let mut files: Vec<Picture> = Vec::new();
    let mut hashes = Vec::new();
    for r in n.elements().filter(|e| e.name == "resource") {
        let Some(data) = r.child("data").and_then(|d| html::base64(&d.text())) else { continue };
        let mime = r.child("mime").map(|m| m.text().trim().to_string()).unwrap_or_default();
        let given = r.child("resource-attributes").and_then(|a| a.child("file-name")).map(|f| f.text().trim().to_string()).unwrap_or_default();
        let name = if given.is_empty() { format!("attachment.{}", ext_for(&mime)) } else { file_name_for(&given, ext_for(&mime)) };
        let name = unique_in(&files, &name);
        hashes.push(hex(&md5(&data)));
        files.push(Picture { name, data });
    }
    let body = enml(&field("content"), &hashes);
    let mut markdown = html::convert(body.as_bytes()).map(|n| n.markdown).unwrap_or_default();
    markdown = swap_stand_ins(&markdown, &files);
    // Attachments the body never shows still come along, at its end.
    let shown: Vec<bool> = (0..files.len()).map(|i| body.contains(&format!("{MEDIA}{i}{MEDIA_END}"))).collect();
    for (f, _) in files.iter().zip(&shown).filter(|(_, s)| !**s) {
        markdown = format!("{}\n\n![[{}]]\n", markdown.trim_end(), f.name);
    }
    let tags: Vec<String> = n.elements().filter(|e| e.name == "tag").map(|t| tag(&t.text())).filter(|t| !t.is_empty()).collect();
    if !tags.is_empty() {
        markdown = format!("{}\n\n{}\n", markdown.trim_end(), tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "));
    }
    if title.is_empty() && markdown.trim().is_empty() {
        return None;
    }
    let title = if title.is_empty() { "Untitled".to_string() } else { title };
    let heading = format!("# {title}");
    let markdown = if markdown.trim_start().starts_with(&heading) {
        format!("{}\n", markdown.trim())
    } else if markdown.trim().is_empty() {
        format!("{heading}\n")
    } else {
        format!("{heading}\n\n{}\n", markdown.trim())
    };
    let created = when(&field("created"));
    let modified = when(&field("updated"));
    Some(Entry { folders: Vec::new(), title, markdown, created, modified: if modified > 0 { modified } else { created }, files })
}

/// The ENML body as HTML the converter reads, `<en-media>` and `<en-todo>`
/// swapped for stand-ins.
fn enml(body: &str, hashes: &[String]) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(i) = rest.find("<en-") {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..=end];
        rest = &rest[end + 1..];
        let name = tag[1..].split(|c: char| c.is_whitespace() || c == '/' || c == '>').next().unwrap_or("");
        match name {
            "en-media" => {
                let hash = attr(tag, "hash").unwrap_or_default().to_ascii_lowercase();
                if let Some(k) = hashes.iter().position(|h| *h == hash) {
                    out.push_str(&format!("<span>{MEDIA}{k}{MEDIA_END}</span>"));
                }
                if !tag.ends_with("/>") && let Some(close) = rest.find("</en-media>") {
                    rest = &rest[close + "</en-media>".len()..];
                }
            }
            "en-todo" => {
                out.push(if attr(tag, "checked").is_some_and(|v| v == "true") { DONE } else { TODO });
                if !tag.ends_with("/>") && let Some(close) = rest.find("</en-todo>") {
                    rest = &rest[close + "</en-todo>".len()..];
                }
            }
            "en-note" => out.push_str("<div>"),
            "en-crypt" => {
                out.push_str("<p>(encrypted in Evernote)</p>");
                if let Some(close) = rest.find("</en-crypt>") {
                    rest = &rest[close + "</en-crypt>".len()..];
                }
            }
            _ => {}
        }
    }
    out.push_str(rest);
    out.replace("</en-note>", "</div>")
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!(" {name}="))? + name.len() + 2;
    let quote = tag[at..].chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let end = tag[at + 1..].find(quote)?;
    Some(tag[at + 1..at + 1 + end].to_string())
}

/// The converted Markdown with attachments embedded and checkboxes as tasks.
fn swap_stand_ins(md: &str, files: &[Picture]) -> String {
    let mut out = String::with_capacity(md.len());
    for line in md.split_inclusive('\n') {
        let indent = line.len() - line.trim_start().len();
        let body = &line[indent..];
        let line = match body.chars().next() {
            Some(c @ (TODO | DONE)) => format!("{}- [{}] {}", &line[..indent], if c == DONE { 'x' } else { ' ' }, body[c.len_utf8()..].trim_start()),
            _ => line.to_string(),
        };
        out.push_str(&line.replace(TODO, "[ ] ").replace(DONE, "[x] "));
    }
    // Each checkbox was a paragraph of its own: as tasks they make one list.
    let task = |l: &str| l.trim_start().starts_with("- [ ] ") || l.trim_start().starts_with("- [x] ");
    let lines: Vec<&str> = out.split('\n').collect();
    let kept: Vec<&str> = (0..lines.len())
        .filter(|&i| !(lines[i].is_empty() && i > 0 && i + 1 < lines.len() && task(lines[i - 1]) && task(lines[i + 1])))
        .map(|i| lines[i])
        .collect();
    let mut md = kept.join("\n");
    for (k, f) in files.iter().enumerate() {
        md = md.replace(&format!("{MEDIA}{k}{MEDIA_END}"), &format!("![[{}]]", f.name));
    }
    md
}

/// A tag as Obsidian reads one: no spaces or `#`.
fn tag(t: &str) -> String {
    t.trim().split_whitespace().collect::<Vec<_>>().join("-").replace(['#', ',', '.'], "")
}

/// `20201231T235959Z` as milliseconds since 1970; 0 if it is not that.
fn when(s: &str) -> i64 {
    let n = |a: usize, b: usize| s.get(a..b).and_then(|v| v.parse::<u32>().ok());
    if s.len() < 15 || s.as_bytes()[8] != b'T' {
        return 0;
    }
    (|| millis(n(0, 4)? as i32, n(4, 6)?, n(6, 8)?, n(9, 11)?, n(11, 13)?, n(13, 15)?))().unwrap_or(0)
}

fn ext_for(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mp4" | "audio/m4a" => "m4a",
        "video/mp4" => "mp4",
        "text/plain" => "txt",
        _ => "bin",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// MD5, which Evernote names attachments by (not for anything secret).
fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16,
        23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x6745_2301u32, 0xefcd_ab89u32, 0x98ba_dcfeu32, 0x1032_5476u32);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(data: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in data.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for k in 0..4 {
                out.push(if k <= c.len() { T[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
            }
        }
        out
    }

    fn export(notes: &str) -> String {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE en-export SYSTEM \"http://xml.evernote.com/pub/evernote-export4.dtd\">\n<en-export export-date=\"20261005T120000Z\" application=\"Evernote\">{notes}</en-export>")
    }

    fn body(enml: &str) -> String {
        format!("<content><![CDATA[<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE en-note SYSTEM \"http://xml.evernote.com/pub/enml2.dtd\"><en-note>{enml}</en-note>]]></content>")
    }

    #[test]
    fn md5_matches_the_known_answers() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"The quick brown fox jumps over the lazy dog")), "9e107d9d372bb6826bd81d3542a419d6");
        assert_eq!(hex(&md5(&[b'a'; 1000])), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    }

    #[test]
    fn a_note_keeps_its_title_text_dates_and_tags() {
        let xml = export(&format!(
            "<note><title>Shopping &amp; errands</title>{}<created>20201231T235959Z</created><updated>20210102T030405Z</updated><tag>home</tag><tag>to do</tag></note>",
            body("<div><b>Milk</b> and <a href=\"https://example.com\">eggs</a></div><ul><li>one</li><li>two</li></ul>")
        ));
        let c = read(xml.as_bytes()).unwrap();
        assert_eq!(c.notes.len(), 1);
        let n = &c.notes[0];
        assert_eq!(n.title, "Shopping & errands");
        assert_eq!(n.markdown, "# Shopping & errands\n\n**Milk** and [eggs](https://example.com)\n\n- one\n- two\n\n#home #to-do\n");
        assert_eq!((n.created, n.modified), (1_609_459_199_000, 1_609_556_645_000));
    }

    #[test]
    fn checkboxes_become_tasks() {
        let xml = export(&format!("<note><title>List</title>{}</note>", body("<div><en-todo checked=\"true\"/>Call mum</div><div><en-todo checked=\"false\"/>Pay rent</div><div><en-todo/>Water plants</div>")));
        let md = &read(xml.as_bytes()).unwrap().notes[0].markdown;
        assert_eq!(md, "# List\n\n- [x] Call mum\n- [ ] Pay rent\n- [ ] Water plants\n");
    }

    #[test]
    fn attachments_come_with_their_names_where_the_body_shows_them() {
        let (png, pdf) = (b"\x89PNG fake picture".as_slice(), b"%PDF-1.4 fake".as_slice());
        let res = |data: &[u8], mime: &str, name: &str| {
            let attrs = if name.is_empty() { String::new() } else { format!("<resource-attributes><file-name>{name}</file-name></resource-attributes>") };
            format!("<resource><data encoding=\"base64\">\n{}\n</data><mime>{mime}</mime>{attrs}</resource>", b64(data))
        };
        let enml = format!("<div>Before</div><en-media type=\"image/png\" hash=\"{}\"/><div>After</div>", hex(&md5(png)));
        let xml = export(&format!("<note><title>Scan</title>{}{}{}</note>", body(&enml), res(png, "image/png", ""), res(pdf, "application/pdf", "Lease: final.pdf")));
        let n = &read(xml.as_bytes()).unwrap().notes[0];
        assert_eq!(n.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["attachment.png", "Lease final.pdf"]);
        assert_eq!(n.files[0].data, png);
        assert_eq!(n.markdown, "# Scan\n\nBefore\n\n![[attachment.png]]\n\nAfter\n\n![[Lease final.pdf]]\n", "the PDF the body never showed goes at the end");
    }

    #[test]
    fn empty_notes_are_skipped_and_other_files_are_refused() {
        let xml = export(&format!("<note><title></title>{}</note><note><title>Kept</title>{}</note>", body(""), body("")));
        let c = read(xml.as_bytes()).unwrap();
        assert_eq!((c.notes.len(), c.skipped), (1, 1));
        assert_eq!(c.notes[0].markdown, "# Kept\n");
        assert!(read(b"<html><body>hello</body></html>").is_err());
    }

    #[test]
    fn a_heading_that_is_the_title_is_not_said_twice() {
        let xml = export(&format!("<note><title>Plan</title>{}</note>", body("<h1>Plan</h1><p>Go.</p>")));
        assert_eq!(read(xml.as_bytes()).unwrap().notes[0].markdown, "# Plan\n\nGo.\n");
    }

    #[test]
    fn arabic_and_encrypted_text() {
        let xml = export(&format!("<note><title>ملاحظة</title>{}</note>", body("<div>مرحبا</div><en-crypt hint=\"x\">AbCd</en-crypt>")));
        assert_eq!(read(xml.as_bytes()).unwrap().notes[0].markdown, "# ملاحظة\n\nمرحبا\n\n(encrypted in Evernote)\n");
    }
}
