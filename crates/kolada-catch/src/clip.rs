//! A page as the note it is caught as: the page's article, what was
//! selected, or its link, as Markdown (kolada-convert), then the note whole:
//! Obsidian frontmatter (`source`, `caught`), the title, the user's line,
//! and the page, its pictures named after the note so two clips never share
//! a picture file in Caught.


use kolada_core::note;
use kolada_sync::engine::PICTURE_MOST;
use kolada_sync::{PICTURES_AT_ONCE, PICTURE_BYTES_AT_ONCE};

/// What is caught.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The page's article.
    Page,
    /// What was selected on it.
    Selection,
    /// Its address, under its title.
    Link,
}

impl Mode {
    pub fn of(word: &str) -> Mode {
        match word {
            "selection" => Mode::Selection,
            "link" => Mode::Link,
            _ => Mode::Page,
        }
    }
}

/// A page read as a note, before it is written whole (`compose`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clipped {
    /// The page's title (its article's first heading, else what the page
    /// calls itself, else its address).
    pub title: String,
    /// Pictures embedded as `![[name]]`, by the names below.
    pub markdown: String,
    /// Pictures inside the page (`data:`), by name.
    pub inside: Vec<(String, Vec<u8>)>,
    /// Pictures from elsewhere, by name: their addresses, to fetch.
    pub fetch: Vec<(String, String)>,
}

/// `html` (the page, or for a selection what was selected, with `text` its
/// plain words) read as a note. `url` resolves relative links and pictures.
pub fn extract(mode: Mode, html_text: &str, text: &str, url: &str, page_title: &str) -> Clipped {
    let fallback_title = || if page_title.trim().is_empty() { url.to_string() } else { page_title.trim().to_string() };
    match mode {
        Mode::Link => Clipped { title: fallback_title(), markdown: format!("[{}]({url})\n", escape_link(&fallback_title())), ..Clipped::default() },
        Mode::Selection => match kolada_convert::paste_html(html_text, Some(url)) {
            Some(p) => Clipped { title: fallback_title(), markdown: ensure_newline(p.markdown), inside: p.pictures.into_iter().map(|p| (p.name, p.data)).collect(), fetch: p.fetch },
            // Plain words (no structure to keep): as they were selected.
            None => Clipped { title: fallback_title(), markdown: ensure_newline(text.trim().to_string()), ..Clipped::default() },
        },
        Mode::Page => match kolada_convert::catch_html(html_text, url) {
            Some(c) => Clipped {
                title: c.title.filter(|t| !t.trim().is_empty()).map(|t| t.trim().to_string()).unwrap_or_else(fallback_title),
                markdown: ensure_newline(c.pasted.markdown),
                inside: c.pasted.pictures.into_iter().map(|p| (p.name, p.data)).collect(),
                fetch: c.pasted.fetch,
            },
            // A page with no text of its own: its link.
            None => extract(Mode::Link, html_text, text, url, page_title),
        },
    }
}

/// Which pictures ride with the clip, of those whose bytes are here (by
/// name and size, in the page's order): as many as Beam takes at once, each
/// no bigger than Beam takes. The rest stay links.
pub fn riding(pictures: &[(String, usize)]) -> Vec<String> {
    let (mut count, mut bytes) = (0usize, 0usize);
    let mut out = Vec::new();
    for (name, size) in pictures {
        if *size == 0 || *size > PICTURE_MOST || count + 1 > PICTURES_AT_ONCE || bytes + size > PICTURE_BYTES_AT_ONCE {
            continue;
        }
        count += 1;
        bytes += size;
        out.push(name.clone());
    }
    out
}

/// The note whole, as it is saved on every device: frontmatter (`source`,
/// `caught`, the time as the browser has it), the title (unless the page's
/// own first heading is it), the user's line, and the page. Pictures in
/// `rides` are renamed after the note (`{title} 1.jpg`); the others become
/// links to where they are (`links`), or go. The text, and each riding
/// picture's old and new names.
pub fn compose(title: &str, url: &str, line: &str, caught: &str, markdown: &str, rides: &[String], links: &[(String, String)]) -> (String, Vec<(String, String)>) {
    let stem = note::file_stem_for(title);
    let mut renames = Vec::new();
    let mut body = markdown.to_string();
    for (k, old) in rides.iter().enumerate() {
        let ext = old.rsplit_once('.').map_or("png", |(_, e)| e);
        let new = format!("{stem} {}.{ext}", k + 1);
        body = body.replace(&format!("![[{old}]]"), &format!("![[{new}]]"));
        renames.push((old.clone(), new));
    }
    for (name, link) in links {
        body = body.replace(&format!("![[{name}]]"), &format!("![]({link})"));
    }
    // Any picture that neither rides nor has an address goes.
    body = drop_embeds(&body, &renames.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>());
    let mut out = format!("---\nsource: {}\ncaught: {}\n---\n", yaml(url), yaml(caught));
    // The title: the page's own first heading when it is the title, else
    // one of ours.
    let body = body.trim_matches('\n');
    let (first, rest) = body.split_once('\n').unwrap_or((body, ""));
    let rest = if first.strip_prefix("# ").is_some_and(|h| h.trim() == title.trim()) { rest } else { body };
    out.push_str(&format!("# {}\n\n", title.trim()));
    let line = line.trim();
    if !line.is_empty() {
        out.push_str(line);
        out.push_str("\n\n");
    }
    out.push_str(rest.trim_matches('\n'));
    (ensure_newline(out.trim_end().to_string()), renames)
}

/// `body` without `![[name]]` embeds of pictures other than `keep`.
fn drop_embeds(body: &str, keep: &[&str]) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find("![[") {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        match tail.find("]]") {
            Some(end) => {
                let name = &tail[3..end];
                if keep.contains(&name) {
                    out.push_str(&tail[..end + 2]);
                }
                rest = &tail[end + 2..];
            }
            None => {
                out.push_str(tail);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    // A line left empty by a picture that went is one blank line, not two.
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out
}

/// A frontmatter value: plain where YAML reads it as written, else quoted.
fn yaml(v: &str) -> String {
    let v = v.replace(['\r', '\n'], " ");
    let plain = !v.is_empty()
        && !v.contains(": ")
        && !v.contains(" #")
        && !v.starts_with(['[', ']', '{', '}', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', '#', ',', '?', '-', ' '])
        && !v.ends_with([' ', ':']);
    if plain { v } else { format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"")) }
}

fn escape_link(t: &str) -> String {
    t.replace('[', "\\[").replace(']', "\\]")
}

fn ensure_newline(mut s: String) -> String {
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A generated article page: navigation, an aside and a footer around
    /// it, a picture inside (`data:`) and two from elsewhere, one relative.
    fn page() -> String {
        let dot = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGD4DwABBAEAX+XDSwAAAABJRU5ErkJggg==";
        format!(
            "<html><head><title>Walking | Longreads</title></head><body><nav><a href=\"/\">Home</a> <a href=\"/about\">About</a></nav>\
             <article><h1>The quiet power of walking</h1><p>An hour a day on foot changes how we <em>think</em>.</p>\
             <img src=\"/img/walk.jpg\" alt=\"A path\"><p>Read <a href=\"/more\">more</a>.</p><img src=\"https://cdn.example/b.webp\"><img src=\"{dot}\"></article>\
             <aside>Subscribe now</aside><footer>© Longreads</footer></body></html>"
        )
    }

    /// The whole page: its article as Markdown, without the page's
    /// furniture, its title its first heading, its links and pictures
    /// resolved against its address; pictures inside it come with it, the
    /// rest are listed to fetch.
    #[test]
    fn a_page_is_caught_as_its_article() {
        let c = extract(Mode::Page, &page(), "", "https://longreads.example/walking", "Walking | Longreads");
        assert_eq!(c.title, "The quiet power of walking");
        assert!(c.markdown.contains("An hour a day on foot changes how we *think*."), "{}", c.markdown.len());
        assert!(c.markdown.contains("[more](https://longreads.example/more)"));
        for junk in ["Home", "About", "Subscribe", "Longreads\n", "©"] {
            assert!(!c.markdown.contains(junk), "{junk}");
        }
        assert_eq!(c.fetch.iter().map(|(_, u)| u.as_str()).collect::<Vec<_>>(), vec!["https://longreads.example/img/walk.jpg", "https://cdn.example/b.webp"]);
        assert_eq!(c.inside.len(), 1);
        let names = c.fetch.iter().map(|(n, _)| n).chain(c.inside.iter().map(|(n, _)| n));
        for name in names {
            assert!(c.markdown.contains(&format!("![[{name}]]")), "{name}");
        }
    }

    /// A selection keeps its structure (or its words, when it has none);
    /// a link is its title and address; a page with no text is its link.
    #[test]
    fn a_selection_and_a_link_are_caught_as_they_are() {
        let sel = extract(Mode::Selection, "<p>Walking is <strong>the</strong> exercise.</p><ul><li>Daily</li><li>Slowly</li></ul>", "", "https://longreads.example/walking", "Walking");
        assert_eq!(sel.markdown, "Walking is **the** exercise.\n\n- Daily\n- Slowly\n");
        let plain = extract(Mode::Selection, "<span>just words</span>", "just words", "https://x.example/", "X");
        assert_eq!(plain.markdown, "just words\n");
        let link = extract(Mode::Link, "", "", "https://longreads.example/walking", "Walking [long]");
        assert_eq!((link.title.as_str(), link.markdown.as_str()), ("Walking [long]", "[Walking \\[long\\]](https://longreads.example/walking)\n"));
        let empty = extract(Mode::Page, "<html><body><script>app()</script></body></html>", "", "https://app.example/", "");
        assert_eq!((empty.title.as_str(), empty.markdown.as_str()), ("https://app.example/", "[https://app.example/](https://app.example/)\n"));
    }

    /// The note whole: frontmatter Obsidian reads, the title once, the
    /// user's line, the page; riding pictures renamed after the note, the
    /// rest links to where they are, or gone.
    #[test]
    fn a_clip_is_written_whole_with_its_pictures_named_after_it() {
        let md = "# The quiet power of walking\n\nAn hour.\n\n![[picture 1.jpg]]\n\n![[picture 2.webp]]\n\n![[picture 3.png]]\n";
        let (text, renames) = compose("The quiet power of walking", "https://longreads.example/walking", " For the talk ", "2026-11-08T14:03", md, &["picture 1.jpg".to_string()], &[("picture 2.webp".to_string(), "https://cdn.example/b.webp".to_string())]);
        assert_eq!(text, "---\nsource: https://longreads.example/walking\ncaught: 2026-11-08T14:03\n---\n# The quiet power of walking\n\nFor the talk\n\nAn hour.\n\n![[The quiet power of walking 1.jpg]]\n\n![](https://cdn.example/b.webp)\n");
        assert_eq!(renames, vec![("picture 1.jpg".to_string(), "The quiet power of walking 1.jpg".to_string())]);
        // Without a heading of its own, the title comes first; a source that
        // YAML would misread is quoted.
        let (text, _) = compose("Notes: day one", "https://x.example/a?b=c #frag", "", "2026-11-08T14:03", "Words.\n", &[], &[]);
        assert!(text.starts_with("---\nsource: \"https://x.example/a?b=c #frag\"\ncaught: 2026-11-08T14:03\n---\n# Notes: day one\n\nWords.\n"), "{text:?}");
    }

    /// As many pictures ride as Beam takes at once, none too big; the
    /// rest stay links.
    #[test]
    fn pictures_ride_within_beams_caps() {
        let small: Vec<(String, usize)> = (0..14).map(|k| (format!("p{k}.jpg"), 1000)).collect();
        assert_eq!(riding(&small).len(), PICTURES_AT_ONCE);
        let big = vec![("huge.png".to_string(), PICTURE_MOST + 1), ("ok.png".to_string(), 10), ("none.png".to_string(), 0)];
        assert_eq!(riding(&big), vec!["ok.png".to_string()]);
    }
}
