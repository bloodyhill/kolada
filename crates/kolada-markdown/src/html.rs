//! A note as HTML: for export (a web page, or a PDF made from one) and for
//! copying as rich text. Reads the Markdown the same line-level way the
//! editor shows it.

use crate::{Align, Block, Line, Span, Style, alignments, block, cells, is_table_header, lines, spans};

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // An escaped mark reads as the mark itself.
            '\\' => {}
            c => out.push(c),
        }
    }
}

fn escape_url(s: &str, out: &mut String) {
    let safe = !s.trim_start().to_ascii_lowercase().starts_with("javascript:");
    if safe {
        escape(s, out);
    }
}

/// An attribute's value as it is: every character that could end it or
/// start markup escaped, backslashes kept.
fn attribute(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

/// Whether a page anyone can open may link to `url`: the web and mail
/// only (an allowlist, so no `javascript:`, `data:` or `file:` in any
/// spelling).
fn public_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let scheme_ok = lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:");
    scheme_ok && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// How a note is turned into HTML: for the user (export, copy), or for a
/// page anyone can open (a shared note): links to the web and mail only,
/// marked as the writer's and not the site's, other notes' names as plain
/// text, and pictures from the share alone.
#[derive(Clone, Copy)]
enum Mode<'a> {
    Own,
    /// The address the share's pictures are under (`/{id}/i/`).
    Public(&'a str),
}

impl Mode<'_> {
    fn public(self) -> bool {
        matches!(self, Mode::Public(_))
    }
}

const REL: &str = "nofollow noopener noreferrer ugc";

/// A link anyone can open, around `inner` (HTML already).
fn public_link(url: &str, inner: &str, out: &mut String) {
    out.push_str("<a href=\"");
    attribute(url, out);
    out.push_str("\" rel=\"");
    out.push_str(REL);
    out.push_str("\">");
    out.push_str(inner);
    out.push_str("</a>");
}

/// The name a shared note's pictures are uploaded under: `3.jpg`.
fn shared_picture(name: &str) -> bool {
    let Some((n, ext)) = name.split_once('.') else { return false };
    !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && ["jpg", "png", "webp", "gif"].contains(&ext)
}

/// `s[from..to]` with its inline spans (in `s`'s offsets, nested, in order)
/// as HTML.
fn inline(s: &str, spans: &[Span], from: usize, to: usize, mode: Mode, out: &mut String) {
    let mut at = from;
    let mut i = 0;
    while i < spans.len() {
        let sp = spans[i];
        if sp.start < at || sp.end > to {
            i += 1;
            continue;
        }
        let mut before = &s[at..sp.start];
        // A picture inside a line (`![alt](address)`) on a public page is
        // never loaded from elsewhere: its `!` goes, and it is a link.
        if mode.public() && sp.style == Style::Link && before.ends_with('!') {
            before = &before[..before.len() - 1];
        }
        escape(before, out);
        let inside: Vec<Span> = spans[i + 1..].iter().filter(|c| c.start >= sp.text_start && c.end <= sp.text_end).copied().collect();
        let text = &s[sp.text_start..sp.text_end];
        match sp.style {
            Style::Code => {
                out.push_str("<code>");
                escape(text, out);
                out.push_str("</code>");
            }
            Style::Link if mode.public() => {
                let target = &s[(sp.text_end + 2).min(sp.end - 1)..sp.end - 1];
                let url = target.split_whitespace().next().unwrap_or("").trim_start_matches('<').trim_end_matches('>');
                let mut shown = String::new();
                inline(s, &inside, sp.text_start, sp.text_end, mode, &mut shown);
                if public_url(url) {
                    public_link(url, &shown, out);
                } else {
                    out.push_str(&shown);
                }
            }
            Style::Link => {
                out.push_str("<a href=\"");
                escape_url(&s[(sp.text_end + 2).min(sp.end - 1)..sp.end - 1], out);
                out.push_str("\">");
                inline(s, &inside, sp.text_start, sp.text_end, mode, out);
                out.push_str("</a>");
            }
            Style::Url => {
                let href = if text.to_ascii_lowercase().starts_with("www.") { format!("https://{text}") } else { text.to_string() };
                if mode.public() {
                    let mut shown = String::new();
                    escape(text, &mut shown);
                    if public_url(&href) { public_link(&href, &shown, out) } else { out.push_str(&shown) }
                } else {
                    out.push_str("<a href=\"");
                    escape_url(&href, out);
                    out.push_str("\">");
                    escape(text, out);
                    out.push_str("</a>");
                }
            }
            // Another note is not on the web: its name reads as text.
            Style::WikiLink if mode.public() => escape(text, out),
            Style::WikiLink => {
                out.push_str("<span class=\"wikilink\">");
                escape(text, out);
                out.push_str("</span>");
            }
            Style::Tag => {
                out.push_str("<span class=\"tag\">");
                escape(text, out);
                out.push_str("</span>");
            }
            style => {
                let tag = match style {
                    Style::Strong => "strong",
                    Style::Emphasis => "em",
                    Style::Strike => "del",
                    _ => "mark",
                };
                out.push_str(&format!("<{tag}>"));
                inline(s, &inside, sp.text_start, sp.text_end, mode, out);
                out.push_str(&format!("</{tag}>"));
            }
        }
        at = sp.end;
        i += 1;
        while i < spans.len() && spans[i].start < sp.end {
            i += 1;
        }
    }
    escape(&s[at..to], out);
}

/// A line's text after its block mark, as HTML.
fn content(line: &str, mode: Mode) -> String {
    let (_, mark, _) = block(line);
    let text = &line[mark..];
    let mut out = String::new();
    inline(text, &spans(text), 0, text.len(), mode, &mut out);
    out
}

/// What is open while lines go by.
#[derive(PartialEq)]
enum Open {
    Nothing,
    Paragraph,
    Quote,
    Code,
}

/// The note `text` as HTML (the body only). With `title`, its first line
/// with words is the note's title and becomes its top heading. Pictures it
/// embeds are left out: see `to_html_with`.
pub fn to_html(text: &str, title: bool) -> String {
    to_html_with(text, title, &|_| None)
}

/// As `to_html`, with the pictures the note embeds on lines of their own
/// (`![[photo.png]]`, `![alt](photo.png)`): `picture` gives the `src` for
/// the file an embed names (a `data:` address from `data_uri`, so the page
/// carries it), None to leave it out. Pictures on the web keep their address.
pub fn to_html_with(text: &str, title: bool, picture: &dyn Fn(&str) -> Option<String>) -> String {
    render(text, title, picture, Mode::Own)
}

/// A shared note as a page anyone can open shows it (the body only, its
/// first line its title): links only to the web and mail, each marked
/// `rel="nofollow noopener noreferrer ugc"`; other notes' names as text;
/// frontmatter and raw HTML never through. Its pictures are the share's
/// own, named `3.jpg` (`![](3.jpg)`, `![[3.jpg]]`), found under
/// `picture_prefix`; any other picture becomes a link to it (on the web)
/// or its words, never a picture loaded from elsewhere.
pub fn to_public_html(text: &str, picture_prefix: &str) -> String {
    render(text, true, &|_| None, Mode::Public(picture_prefix))
}

fn render(text: &str, title: bool, picture: &dyn Fn(&str) -> Option<String>, mode: Mode) -> String {
    let mut out = String::new();
    let mut open = Open::Nothing;
    // Open lists, outermost first: numbered or not.
    let mut lists: Vec<bool> = Vec::new();
    let mut titled = !title;
    let close_lists = |lists: &mut Vec<bool>, to: usize, out: &mut String| {
        while lists.len() > to {
            out.push_str(if lists.pop() == Some(true) { "</ol>\n" } else { "</ul>\n" });
        }
    };
    let close = |open: &mut Open, out: &mut String| {
        match open {
            Open::Paragraph => out.push_str("</p>\n"),
            Open::Quote => out.push_str("</p></blockquote>\n"),
            Open::Code => out.push_str("</code></pre>\n"),
            Open::Nothing => {}
        }
        *open = Open::Nothing;
    };
    let all: Vec<Line> = lines(text).collect();
    let mut i = 0;
    while i < all.len() {
        let l = all[i];
        i += 1;
        let line = &text[l.start..l.end];
        match l.block {
            Block::Frontmatter => continue,
            Block::Fence => {
                if open == Open::Code {
                    close(&mut open, &mut out);
                } else {
                    close(&mut open, &mut out);
                    close_lists(&mut lists, 0, &mut out);
                    out.push_str("<pre><code>");
                    open = Open::Code;
                }
                continue;
            }
            Block::Code => {
                if open != Open::Code {
                    close(&mut open, &mut out);
                    out.push_str("<pre><code>");
                    open = Open::Code;
                }
                let mut s = String::new();
                for c in line.chars() {
                    match c {
                        '&' => s.push_str("&amp;"),
                        '<' => s.push_str("&lt;"),
                        '>' => s.push_str("&gt;"),
                        c => s.push(c),
                    }
                }
                out.push_str(&s);
                out.push('\n');
                continue;
            }
            _ => {}
        }
        if line.trim().is_empty() {
            close(&mut open, &mut out);
            close_lists(&mut lists, 0, &mut out);
            continue;
        }
        // A picture on a line of its own.
        if let Some(embed) = Embed::of(line) {
            close(&mut open, &mut out);
            close_lists(&mut lists, 0, &mut out);
            if let Mode::Public(prefix) = mode {
                public_picture(&embed, prefix, &mut out);
                continue;
            }
            let src =if embed.target.contains("://") { Some(embed.target.to_string()) } else { picture(embed.target) };
            if let Some(src) = src {
                out.push_str("<figure><img src=\"");
                escape_url(&src, &mut out);
                out.push_str("\" alt=\"");
                escape(embed.alt, &mut out);
                out.push('"');
                if let Some(w) = embed.width {
                    out.push_str(&format!(" width=\"{w}\""));
                }
                out.push_str("></figure>\n");
            }
            continue;
        }
        // A table: its header, its alignment row and the rows below.
        if is_table_header(&all, i - 1) {
            close(&mut open, &mut out);
            close_lists(&mut lists, 0, &mut out);
            titled = true;
            table(text, &all, &mut i, mode, &mut out);
            continue;
        }
        if !titled {
            titled = true;
            if matches!(l.block, Block::Body | Block::Heading(_)) {
                close(&mut open, &mut out);
                out.push_str(&format!("<h1 dir=\"auto\">{}</h1>\n", content(line, mode)));
                continue;
            }
        }
        match l.block {
            Block::Heading(n) => {
                close(&mut open, &mut out);
                close_lists(&mut lists, 0, &mut out);
                out.push_str(&format!("<h{n} dir=\"auto\">{}</h{n}>\n", content(line, mode)));
            }
            Block::Rule => {
                close(&mut open, &mut out);
                close_lists(&mut lists, 0, &mut out);
                out.push_str("<hr>\n");
            }
            Block::Quote => {
                close_lists(&mut lists, 0, &mut out);
                if open == Open::Quote {
                    out.push_str("<br>");
                } else {
                    close(&mut open, &mut out);
                    out.push_str("<blockquote dir=\"auto\"><p>");
                    open = Open::Quote;
                }
                out.push_str(&content(line, mode));
            }
            Block::Bullet | Block::Ordered | Block::Task { .. } => {
                close(&mut open, &mut out);
                let numbered = l.block == Block::Ordered;
                let depth = l.depth as usize;
                close_lists(&mut lists, depth + 1, &mut out);
                if lists.len() == depth + 1 && lists[depth] != numbered {
                    close_lists(&mut lists, depth, &mut out);
                }
                while lists.len() < depth + 1 {
                    out.push_str(if numbered { "<ol dir=\"auto\">\n" } else { "<ul dir=\"auto\">\n" });
                    lists.push(numbered);
                }
                match l.block {
                    // A box drawn as the app draws it (a form's checkbox
                    // prints grey): red and ticked when done.
                    Block::Task { done } => out.push_str(&format!(
                        "<li class=\"task{}\"><span class=\"box\">{}</span>{}</li>\n",
                        if done { " done" } else { "" },
                        if done { "\u{2713}" } else { "" },
                        content(line, mode)
                    )),
                    _ => out.push_str(&format!("<li>{}</li>\n", content(line, mode))),
                }
            }
            _ => {
                close_lists(&mut lists, 0, &mut out);
                if open == Open::Paragraph {
                    out.push_str("<br>\n");
                } else {
                    close(&mut open, &mut out);
                    out.push_str("<p dir=\"auto\">");
                    open = Open::Paragraph;
                }
                out.push_str(&content(line, mode));
            }
        }
    }
    close(&mut open, &mut out);
    close_lists(&mut lists, 0, &mut out);
    out
}

/// A picture a line embeds, when the line is only that.
struct Embed<'a> {
    /// The file (or web address) it names.
    target: &'a str,
    alt: &'a str,
    width: Option<u32>,
}

impl<'a> Embed<'a> {
    fn of(line: &'a str) -> Option<Embed<'a>> {
        let s = line.trim();
        let width = |size: &str| size.trim().split('x').next().and_then(|w| w.parse::<u32>().ok()).filter(|w| *w > 0);
        let embed = if let Some(inner) = s.strip_prefix("![[").and_then(|r| r.strip_suffix("]]")) {
            // ![[photo.png|300]]: a size after the bar.
            let mut parts = inner.split('|');
            let target = parts.next()?.trim();
            let size = parts.next_back().and_then(width);
            Embed { target, alt: target, width: size }
        } else {
            let (alt, url) = s.strip_prefix("![")?.split_once("](")?;
            let target = url.strip_suffix(')')?.split_whitespace().next()?;
            let (alt, size) = alt.rsplit_once('|').map_or((alt, None), |(a, w)| (a, width(w)));
            Embed { target, alt, width: size }
        };
        let file = embed.target.split(['?', '#']).next().unwrap_or("");
        let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        ["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "tif", "tiff", "ico", "avif"].contains(&ext.as_str()).then_some(embed)
    }
}

/// A picture on a line of its own on a public page: the share's own
/// picture, else a link to it on the web, else its words.
fn public_picture(embed: &Embed, prefix: &str, out: &mut String) {
    if shared_picture(embed.target) {
        out.push_str("<figure><img src=\"");
        attribute(prefix, out);
        attribute(embed.target, out);
        out.push_str("\" alt=\"");
        if embed.alt != embed.target {
            escape(embed.alt, out);
        }
        out.push('"');
        if let Some(w) = embed.width {
            out.push_str(&format!(" width=\"{w}\""));
        }
        out.push_str("></figure>\n");
        return;
    }
    let named = !embed.alt.is_empty() && embed.alt != embed.target;
    let mut words = String::new();
    escape(if named { embed.alt } else { embed.target }, &mut words);
    if public_url(embed.target) {
        out.push_str("<p dir=\"auto\">");
        public_link(embed.target, &words, out);
        out.push_str("</p>\n");
    } else if named {
        out.push_str(&format!("<p dir=\"auto\">{words}</p>\n"));
    }
}

/// The table whose header is line `*i - 1` (its alignment row at `*i`), as
/// HTML; `*i` moves past its last row.
fn table(text: &str, all: &[Line], i: &mut usize, mode: Mode, out: &mut String) {
    let line = |k: usize| &text[all[k].start..all[k].end];
    let head = line(*i - 1);
    let aligns = alignments(line(*i)).unwrap_or_default();
    *i += 1;
    let mut rows = vec![head];
    while *i < all.len() && all[*i].block == Block::TableRow {
        rows.push(line(*i));
        *i += 1;
    }
    let found: Vec<Vec<&str>> = rows.iter().map(|r| cells(r).into_iter().map(|(a, z)| &r[a..z]).collect()).collect();
    let width = found.iter().map(Vec::len).max().unwrap_or(0);
    // A column of short cells (numbers, phones, dates) is never broken
    // across lines: the long ones around it wrap instead.
    let short: Vec<bool> = (0..width).map(|k| found.iter().all(|r| r.get(k).is_none_or(|c| c.chars().count() <= SHORT_CELL))).collect();
    out.push_str("<table dir=\"auto\">\n");
    for (n, row) in found.iter().enumerate() {
        let tag = if n == 0 { "th" } else { "td" };
        out.push_str(if n == 0 { "<thead><tr>" } else { "<tr>" });
        for k in 0..width {
            out.push('<');
            out.push_str(tag);
            if short[k] {
                out.push_str(" class=\"n\"");
            }
            match aligns.get(k) {
                Some(Align::Center) => out.push_str(" style=\"text-align:center\""),
                Some(Align::Right) => out.push_str(" style=\"text-align:end\""),
                _ => {}
            }
            out.push('>');
            if let Some(cell) = row.get(k) {
                inline(cell, &spans(cell), 0, cell.len(), mode, out);
            }
            out.push_str(&format!("</{tag}>"));
        }
        out.push_str(if n == 0 { "</tr></thead>\n<tbody>\n" } else { "</tr>\n" });
    }
    out.push_str("</tbody>\n</table>\n");
}

/// Cells up to this many characters, in a column of only such, keep to one
/// line.
const SHORT_CELL: usize = 20;

/// A file's bytes as a `data:` address for a page to carry it, its kind
/// from its name.
pub fn data_uri(name: &str, bytes: &[u8]) -> String {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        _ => "image/png",
    };
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 32);
    out.push_str("data:");
    out.push_str(mime);
    out.push_str(";base64,");
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            out.push(if k <= c.len() { T[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// A whole web page around a note's HTML, in the app's paper theme:
/// readable on screen and in print, the logo's teal its only colour.
pub fn document(title: &str, body: &str) -> String {
    let mut t = String::new();
    escape(title, &mut t);
    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{t}</title>
<style>
body {{ font: 16px/1.65 "Segoe UI", system-ui, -apple-system, sans-serif; color: #1d1c1a; background: #fff; max-width: 46rem; margin: 3rem auto; padding: 0 1.5rem; }}
h1 {{ font-size: 1.85em; line-height: 1.25; margin: 0 0 .7em; letter-spacing: -.01em; }}
h2 {{ font-size: 1.4em; margin: 1.5em 0 .45em; }}
h3, h4, h5, h6 {{ margin: 1.25em 0 .35em; }}
p {{ margin: 0 0 .9em; }}
a, .wikilink, .tag {{ color: #0B7A79; text-decoration: none; }}
mark {{ background: #C4FFF9; color: inherit; padding: 0 .12em; border-radius: 3px; }}
code {{ font-family: "Cascadia Mono", Consolas, ui-monospace, monospace; font-size: .88em; background: #f3f2ef; padding: .1em .32em; border-radius: 4px; }}
pre {{ background: #f3f2ef; padding: 1em 1.2em; border-radius: 10px; overflow: auto; }}
pre code {{ background: none; padding: 0; }}
blockquote {{ margin: 1em 0; padding: .6em 1em; background: #f7f6f4; border-radius: 8px; color: #55534f; }}
blockquote p {{ margin: 0; }}
hr {{ border: 0; height: 1px; background: #e8e6e2; margin: 2em 0; }}
ul, ol {{ padding-inline-start: 1.6em; margin: 0 0 .9em; }}
li.task {{ list-style: none; margin-inline-start: -1.5em; }}
li.task .box {{ display: inline-block; width: 1em; height: 1em; line-height: 1em; margin-inline-end: .55em; border-radius: .25em; background: #e9e7e3; color: #fff; font-size: .9em; font-weight: 700; text-align: center; vertical-align: -.1em; }}
li.task.done .box {{ background: #0E9594; }}
li.task.done {{ color: #8a8780; }}
del {{ color: #8a8780; }}
table {{ border-collapse: separate; border-spacing: 0; width: 100%; margin: 1.2em 0 1.4em; font-size: .94em; line-height: 1.5; }}
th, td {{ padding: .6em .85em; text-align: start; vertical-align: top; overflow-wrap: break-word; }}
th {{ font-weight: 600; background: #f3f2ef; }}
th:first-child {{ border-start-start-radius: 8px; border-end-start-radius: 8px; }}
th:last-child {{ border-start-end-radius: 8px; border-end-end-radius: 8px; }}
tbody tr + tr td {{ border-top: 1px solid #ecebe7; }}
th.n, td.n {{ white-space: nowrap; }}
figure {{ margin: 1.2em 0; }}
img {{ max-width: 100%; height: auto; border-radius: 6px; }}
@page {{ margin: 16mm; }}
@media print {{
  body {{ margin: 0 auto; max-width: none; }}
  thead {{ display: table-header-group; }}
  tr, figure, pre, blockquote {{ break-inside: avoid; }}
  * {{ -webkit-print-color-adjust: exact; print-color-adjust: exact; }}
}}
</style>
</head>
<body>
{body}</body>
</html>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_become_their_elements() {
        let html = to_html("Plan\nShip **it** on *Monday*.\nThen rest.\n\n## Steps\n- one\n\t- nested\n- [x] done\n1. first\n> quoted\n---\n```\nlet x = 1 < 2;\n```", true);
        assert!(html.starts_with("<h1 dir=\"auto\">Plan</h1>\n"), "{html}");
        assert!(html.contains("<p dir=\"auto\">Ship <strong>it</strong> on <em>Monday</em>.<br>\nThen rest.</p>"), "{html}");
        assert!(html.contains("<h2 dir=\"auto\">Steps</h2>"));
        assert!(html.contains("<ul dir=\"auto\">\n<li>one</li>\n<ul dir=\"auto\">\n<li>nested</li>\n</ul>\n"), "{html}");
        assert!(html.contains("<li class=\"task done\"><span class=\"box\">\u{2713}</span>done</li>"), "{html}");
        assert!(html.contains("</ul>\n<ol dir=\"auto\">\n<li>first</li>\n</ol>"), "{html}");
        assert!(html.contains("<blockquote dir=\"auto\"><p>quoted</p></blockquote>"));
        assert!(html.contains("<hr>"));
        assert!(html.contains("<pre><code>let x = 1 &lt; 2;\n</code></pre>"), "{html}");
    }

    #[test]
    fn inline_marks_links_and_escapes() {
        let html = to_html("a ==marked== `c<d>` [site](https://x.io/?a=1&b=2) [[Note|alias]] #tag ~~old~~ <b>", false);
        assert_eq!(
            html,
            "<p dir=\"auto\">a <mark>marked</mark> <code>c&lt;d&gt;</code> <a href=\"https://x.io/?a=1&amp;b=2\">site</a> <span class=\"wikilink\">alias</span> <span class=\"tag\">#tag</span> <del>old</del> &lt;b&gt;</p>\n"
        );
        assert!(!to_html("[x](javascript:alert(1))", false).contains("javascript"));
        assert_eq!(to_html("go to www.x.io.", false), "<p dir=\"auto\">go to <a href=\"https://www.x.io\">www.x.io</a>.</p>\n");
    }

    #[test]
    fn a_table_becomes_a_table_and_is_not_taken_for_the_title() {
        let html = to_html("| # | Name | Cost |\n| --- | :-: | --: |\n| 1 | **Tea** from the hills of Darjeeling | 4 |\n| 2 | Pipe \\| bar |\n\nAfter", true);
        assert!(html.starts_with("<table dir=\"auto\">\n<thead><tr><th class=\"n\">#</th><th style=\"text-align:center\">Name</th><th class=\"n\" style=\"text-align:end\">Cost</th></tr></thead>\n<tbody>\n"), "{html}");
        assert!(html.contains("<tr><td class=\"n\">1</td><td style=\"text-align:center\"><strong>Tea</strong> from the hills of Darjeeling</td><td class=\"n\" style=\"text-align:end\">4</td></tr>"), "{html}");
        assert!(html.contains("<tr><td class=\"n\">2</td><td style=\"text-align:center\">Pipe | bar</td><td class=\"n\" style=\"text-align:end\"></td></tr>\n</tbody>\n</table>\n"), "a short row is filled out: {html}");
        assert!(html.ends_with("<p dir=\"auto\">After</p>\n") && !html.contains("<h1"), "{html}");
        let titled = to_html("Team\n\n| a | b |\n| - | - |\n| 1 | 2 |", true);
        assert!(titled.starts_with("<h1 dir=\"auto\">Team</h1>\n<table"), "{titled}");
    }

    #[test]
    fn pictures_come_with_the_page() {
        let found = |name: &str| (name == "shot.png").then(|| data_uri(name, b"PNG!"));
        let html = to_html_with("Trip\n\n![[shot.png|300]]\n![a cat](cats/cat.jpg)\n![[Other note]]\n![x](https://example.com/a.png)", true, &found);
        assert!(html.contains("<figure><img src=\"data:image/png;base64,UE5HIQ==\" alt=\"shot.png\" width=\"300\"></figure>"), "{html}");
        assert!(!html.contains("cats/cat.jpg"), "a picture not found is left out: {html}");
        assert!(html.contains("<span class=\"wikilink\">Other note</span>"), "an embedded note stays a link: {html}");
        assert!(html.contains("<img src=\"https://example.com/a.png\" alt=\"x\">"), "{html}");
        assert!(!to_html("![[icon.png]]\n![[logo.png]]", true).contains("<h1"), "pictures are not a title");
        assert_eq!(data_uri("a.JPG", b"ab"), "data:image/jpeg;base64,YWI=");
    }

    #[test]
    fn the_page_uses_only_the_logos_red() {
        let page = document("x", "");
        assert!(page.contains("#0B7A79") && page.contains("#0E9594"));
        for colour in ["#3b26b0", "#dad2ff", "#2a0f9e", "yellow"] {
            assert!(!page.contains(colour), "{colour}");
        }
    }

    #[test]
    fn public_html_drops_unsafe_links() {
        let md = "Shared\n[web](https://x.io/?a=1&b=2) [mail](mailto:a@b.co) [js](javascript:void) [js2]( JavaScript:void) [data](data:text/html,x) [file](file:///c:/x) [note](Plan.md) [[Secret plan|the plan]] [[Other]] www.y.io https://z.io";
        let html = to_public_html(md, "/Ab3d/i/");
        let rel = "rel=\"nofollow noopener noreferrer ugc\"";
        assert!(html.contains(&format!("<a href=\"https://x.io/?a=1&amp;b=2\" {rel}>web</a>")), "{html}");
        assert!(html.contains(&format!("<a href=\"mailto:a@b.co\" {rel}>mail</a>")), "{html}");
        assert!(html.contains(&format!("<a href=\"https://www.y.io\" {rel}>www.y.io</a>")), "{html}");
        assert!(html.contains(&format!("<a href=\"https://z.io\" {rel}>https://z.io</a>")), "{html}");
        for gone in ["javascript", "JavaScript", "data:", "file:", "Plan.md", "Secret plan", "wikilink"] {
            assert!(!html.contains(gone), "{gone} got through: {html}");
        }
        assert!(html.contains(" js js2 data file note the plan Other "), "their words stay: {html}");
        assert_eq!(html.matches("<a ").count(), html.matches(rel).count(), "every link is marked: {html}");
        assert!(html.starts_with("<h1 dir=\"auto\">Shared</h1>"), "{html}");
        assert!(!to_public_html("---\nsecret: yes\n---\nHi", "/").contains("secret"), "frontmatter stays home");
    }

    #[test]
    fn public_pictures_point_at_the_share() {
        let md = "Trip\n\n![](0.jpg)\n![[12.png|300]]\n![a cat](cat.jpg)\n![[photo.png]]\n![remote](https://evil.example/a.png)\n![](3.jpg?x=1)\n![](../1.jpg)\ninline ![x](https://evil.example/b.png) here";
        let html = to_public_html(md, "/Ab3d/i/");
        assert!(html.contains("<figure><img src=\"/Ab3d/i/0.jpg\" alt=\"\"></figure>"), "{html}");
        assert!(html.contains("<figure><img src=\"/Ab3d/i/12.png\" alt=\"\" width=\"300\"></figure>"), "{html}");
        assert_eq!(html.matches("<img").count(), 2, "only the share's own pictures: {html}");
        assert!(html.contains("<p dir=\"auto\">a cat</p>"), "a picture not shared leaves its words: {html}");
        assert!(!html.contains("photo.png") && !html.contains("../1.jpg") && !html.contains("3.jpg?"), "{html}");
        assert!(html.contains("<a href=\"https://evil.example/a.png\" rel=\"nofollow noopener noreferrer ugc\">remote</a>"), "{html}");
        assert!(html.contains("inline <a href=\"https://evil.example/b.png\""), "an inline picture is a link, its ! gone: {html}");
        let quoted = to_public_html("![](1.jpg)", "/a\"b/");
        assert!(quoted.contains("src=\"/a&quot;b/1.jpg\""), "{quoted}");
    }

    #[test]
    fn raw_html_never_passes() {
        let md = "<script>alert(1)</script>\n**<img src=x onerror=alert(1)>**\n[<b>x</b>](https://x.io\"onmouseover=\"alert(1))\n| <i>a</i> | b |\n| - | - |\n| <svg onload=alert(1)> | c |\n```\n</code><script>x</script>\n```\n`<b>`\n- <iframe src=x>\n> <style>body{}</style>";
        let html = to_public_html(md, "/i/");
        for tag in ["<script", "<img", "<b>", "<i>", "<svg", "<iframe", "<style"] {
            assert!(!html.contains(tag), "{tag} got through: {html}");
        }
        assert!(!html.contains("\"onmouseover"), "a quote never ends an attribute: {html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn right_to_left_text_keeps_its_direction() {
        let html = to_html("ملاحظة\nنص **مهم**", true);
        assert!(html.contains("<h1 dir=\"auto\">ملاحظة</h1>") && html.contains("<strong>مهم</strong>"));
        let page = document("A <title>", &html);
        assert!(page.contains("<title>A &lt;title&gt;</title>") && page.contains("<meta charset=\"utf-8\">"));
    }
}
