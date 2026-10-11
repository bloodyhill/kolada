//! Writing what a document says as Markdown, the way Notsy writes notes:
//! blocks (headings, paragraphs, lists, tables...) made of runs of styled
//! text. Every converter fills these; this writes them once, the same way.

/// A run of text and how it looks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
    /// Markdown already (a footnote mark): written as it is.
    pub raw: bool,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Span {
        Span { text: text.into(), ..Span::default() }
    }

    pub fn raw(text: impl Into<String>) -> Span {
        Span { text: text.into(), raw: true, ..Span::default() }
    }

    fn same_look(&self, o: &Span) -> bool {
        (self.bold, self.italic, self.strike, self.code, &self.link, self.raw) == (o.bold, o.italic, o.strike, o.code, &o.link, o.raw)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(u8, Vec<Span>),
    Paragraph(Vec<Span>),
    /// A list item: numbered or not, how deep (0 is outermost), and, for
    /// a task, whether it is done.
    Item { ordered: bool, depth: usize, task: Option<bool>, spans: Vec<Span> },
    /// Rows of cells; the first row is the header.
    Table(Vec<Vec<Vec<Span>>>),
    Quote(Vec<Span>),
    Code(String),
    /// A picture, by its name among the note's pictures (or an address).
    Picture { name: String, alt: String },
    Rule,
}

/// The Markdown for `blocks`.
pub fn write(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut previous: Option<&Block> = None;
    // Each list's kind and numbering, by depth.
    let mut numbers: Vec<(bool, usize)> = Vec::new();
    for b in blocks {
        let in_list = |b: Option<&Block>| matches!(b, Some(Block::Item { .. }));
        if !out.is_empty() {
            // Items of one list sit together; everything else is apart.
            out.push_str(if in_list(previous) && matches!(b, Block::Item { .. }) { "\n" } else { "\n\n" });
        }
        if !matches!(b, Block::Item { .. }) {
            numbers.clear();
        }
        match b {
            Block::Heading(level, spans) => {
                let text = inline(spans);
                out.push_str(&"#".repeat((*level).clamp(1, 6) as usize));
                out.push(' ');
                out.push_str(&text.replace('\n', " "));
            }
            Block::Paragraph(spans) => out.push_str(&start_safe(&inline(spans))),
            Block::Item { ordered, depth, task, spans } => {
                numbers.truncate(depth + 1);
                while numbers.len() <= *depth {
                    numbers.push((*ordered, 0));
                }
                // A list of the other kind starts counting again.
                if numbers[*depth].0 != *ordered {
                    numbers[*depth] = (*ordered, 0);
                }
                numbers[*depth].1 += 1;
                out.push_str(&"    ".repeat(*depth));
                if *ordered {
                    out.push_str(&format!("{}. ", numbers[*depth].1));
                } else {
                    out.push_str("- ");
                }
                match task {
                    Some(true) => out.push_str("[x] "),
                    Some(false) => out.push_str("[ ] "),
                    None => {}
                }
                let text = inline(spans);
                let pad = format!("\n{}", "    ".repeat(depth + 1));
                out.push_str(&text.replace('\n', &pad));
            }
            Block::Table(rows) => out.push_str(&table(rows)),
            Block::Quote(spans) => {
                let text = inline(spans);
                out.push_str("> ");
                out.push_str(&text.replace('\n', "\n> "));
            }
            Block::Code(text) => {
                let fence = if text.contains("```") { "~~~~" } else { "```" };
                out.push_str(fence);
                out.push('\n');
                out.push_str(text.trim_end_matches('\n'));
                out.push('\n');
                out.push_str(fence);
            }
            Block::Picture { name, alt } => {
                if name.contains("://") || name.starts_with("data:") {
                    out.push_str(&format!("![{}]({})", escape(alt), name.replace(' ', "%20")));
                } else {
                    out.push_str(&format!("![[{name}]]"));
                }
            }
            Block::Rule => out.push_str("---"),
        }
        previous = Some(b);
    }
    out.push('\n');
    out
}

/// Runs of text as Markdown: neighbors that look alike join, marks go
/// around the words (spaces stay outside them), and only characters that
/// would mean something are escaped.
pub fn inline(spans: &[Span]) -> String {
    let mut merged: Vec<Span> = Vec::new();
    for s in spans.iter().filter(|s| !s.text.is_empty()) {
        match merged.last_mut() {
            Some(last) if last.same_look(s) => last.text.push_str(&s.text),
            _ => merged.push(s.clone()),
        }
    }
    let mut out = String::new();
    for s in &merged {
        if s.raw {
            out.push_str(&s.text);
            continue;
        }
        let body = s.text.trim();
        if body.is_empty() {
            out.push_str(&s.text.replace('\n', " "));
            continue;
        }
        let lead = &s.text[..s.text.len() - s.text.trim_start().len()];
        let trail = &s.text[s.text.trim_end().len()..];
        out.push_str(lead);
        let mut piece = if s.code {
            let ticks = if body.contains('`') { "``" } else { "`" };
            format!("{ticks}{body}{ticks}")
        } else {
            escape(body)
        };
        if !s.code {
            if s.strike {
                piece = format!("~~{piece}~~");
            }
            if s.italic {
                piece = format!("*{piece}*");
            }
            if s.bold {
                piece = format!("**{piece}**");
            }
        }
        if let Some(url) = &s.link {
            // A link that is its own address reads as the address.
            if url == body {
                piece = if url.starts_with("http") { url.clone() } else { format!("<{url}>") };
            } else {
                piece = format!("[{piece}]({})", url.replace(' ', "%20").replace(')', "%29"));
            }
        }
        out.push_str(&piece);
        out.push_str(trail);
    }
    out.trim().to_string()
}

/// Escapes what Markdown (and Obsidian's additions) would read as marks.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let before = i.checked_sub(1).map(|j| chars[j]);
        let after = chars.get(i + 1).copied();
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        let special = match c {
            '*' | '`' | '[' | ']' | '\\' => true,
            // Inside a word (snake_case) an underscore is plain.
            '_' => !(word(before) && word(after)),
            // Obsidian's highlight and comment marks.
            '=' => after == Some('=') || before == Some('='),
            '%' => after == Some('%') || before == Some('%'),
            '<' => after.is_some_and(|a| a.is_ascii_alphabetic() || a == '/' || a == '!'),
            _ => false,
        };
        if special {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A paragraph that would read as another block where it starts (a
/// heading, list or quote) gets a backslash.
fn start_safe(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let t = line.trim_start();
            let ordered = t.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()));
            let marks = ["# ", "> ", "- ", "+ ", "* ", "| ", "---"];
            if ordered {
                let dot = t.find('.').expect("an ordered start has a dot");
                format!("{}\\{}", &t[..dot], &t[dot..])
            } else if marks.iter().any(|m| t.starts_with(m)) || t == "#" || t == "-" {
                format!("\\{t}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn table(rows: &[Vec<Vec<Span>>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let cell = |spans: &Vec<Span>| inline(spans).replace('\n', "<br>").replace('|', "\\|");
    let mut lines = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut cells: Vec<String> = row.iter().map(cell).collect();
        cells.resize(cols, String::new());
        lines.push(format!("| {} |", cells.join(" | ")));
        if r == 0 {
            lines.push(format!("|{}|", vec![" --- "; cols].join("|")));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold(t: &str) -> Span {
        Span { bold: true, ..Span::plain(t) }
    }

    #[test]
    fn marks_go_around_words_and_runs_join() {
        let spans = vec![Span::plain("Ship the "), bold("public "), bold("beta "), Span::plain("now.")];
        assert_eq!(inline(&spans), "Ship the **public beta** now.");
        let link = Span { link: Some("https://notsy.app".into()), ..Span::plain("the post") };
        assert_eq!(inline(&[Span::plain("see "), link]), "see [the post](https://notsy.app)");
        let bare = Span { link: Some("https://notsy.app".into()), ..Span::plain("https://notsy.app") };
        assert_eq!(inline(&[bare]), "https://notsy.app");
        assert_eq!(inline(&[Span { code: true, ..Span::plain("a`b") }]), "``a`b``");
    }

    #[test]
    fn escapes_only_what_means_something() {
        assert_eq!(escape("2 * 3 = 6, snake_case, [x], a == b, 50%%, <b>"), "2 \\* 3 = 6, snake_case, \\[x\\], a \\=\\= b, 50\\%\\%, \\<b>");
        assert_eq!(write(&[Block::Paragraph(vec![Span::plain("# not a heading")])]), "\\# not a heading\n");
        assert_eq!(write(&[Block::Paragraph(vec![Span::plain("1. not a list")])]), "1\\. not a list\n");
    }

    #[test]
    fn lists_nest_and_number_tables_line_up() {
        let item = |ordered, depth, t: &str| Block::Item { ordered, depth, task: None, spans: vec![Span::plain(t)] };
        let blocks = vec![
            Block::Heading(2, vec![Span::plain("What ships")]),
            item(false, 0, "Pictures"),
            item(false, 1, "pasted"),
            item(true, 0, "one"),
            item(true, 0, "two"),
            Block::Table(vec![vec![vec![Span::plain("A")], vec![Span::plain("B|C")]], vec![vec![Span::plain("1")]]]),
        ];
        assert_eq!(write(&blocks), "## What ships\n\n- Pictures\n    - pasted\n1. one\n2. two\n\n| A | B\\|C |\n| --- | --- |\n| 1 |  |\n");
    }
}
