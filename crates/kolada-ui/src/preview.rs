//! A note's box in the notes grid (a phone's or a tablet's list of notes,
//! shown as boxes): its title, then its words, as many lines as the box
//! holds; or, when the note has a picture, a table or a list, the first of
//! those in their place.
//! Every box is the same size, so a picture is cropped and a table or a
//! list shows its first `ROWS` rows. Each shell draws the box its own way;
//! what goes in it is decided here, so every OS shows the same.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kolada_markdown::{Block, Line, cells, is_table_header, lines, readable as plain};

/// The rows a box shows of a table (its header among them) or a list.
pub const ROWS: usize = 3;

/// The columns a box shows of a table: more would be too narrow to read.
pub const COLUMNS: usize = 3;

/// The lines of words a box is given at most (each a line of the note,
/// which a box may wrap): more than any box shows, and each shell shows as
/// many as fit its own height, the last cut with an ellipsis.
pub const LINES: usize = 8;

/// The characters a line of words keeps at most.
const LINE_MOST: usize = 200;

/// How much of a note is read for its box: its start is what the box shows.
const READ: usize = 64 * 1024;

/// What a box shows under its title.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    /// The note's name, on one line.
    pub title: String,
    /// The first line of its words after the title, as it reads (no marks);
    /// empty when there is none.
    pub line: String,
    /// Its lines of words after the title, as they read, `line` first: at
    /// most `LINES`. A box with no picture, table or list shows as many as
    /// fit.
    pub lines: Vec<String>,
    /// The first picture, table or list in it, shown in place of the line.
    pub rich: Option<Rich>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rich {
    /// A picture on a line of its own: its file as the note names it.
    Picture(String),
    /// A table's first rows, its header first, each as its cells read.
    Table(Vec<Vec<String>>),
    /// A list's first items.
    List(Vec<Item>),
}

/// A list's item as a box shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub mark: Mark,
    /// Indented under the item above: 0 at the list's edge.
    pub depth: u8,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    Bullet,
    /// Its number as written: `1.`, `2)`.
    Number(String),
    Task { done: bool },
}

impl Rich {
    /// Its kind, as the shells name it: `picture`, `table`, `list`.
    pub fn kind(&self) -> &'static str {
        match self {
            Rich::Picture(_) => "picture",
            Rich::Table(_) => "table",
            Rich::List(_) => "list",
        }
    }
}

/// The box of the note named `name` (its file's name) whose text is `md`.
pub fn preview(name: &str, md: &str) -> Preview {
    let all: Vec<Line> = lines(md).collect();
    let title = if name.trim().is_empty() { kolada_markdown::plain::title(md) } else { name.trim().to_string() };
    let mut said: Vec<String> = Vec::new();
    let mut rich = None;
    // The first line with words names the note (its file follows it): it
    // is the title already, unless it says something else.
    let mut first = true;
    let mut i = 0;
    while i < all.len() && rich.is_none() {
        let l = all[i];
        let text = &md[l.start..l.end];
        match l.block {
            Block::Frontmatter | Block::Fence | Block::Rule | Block::TableDelimiter => {}
            _ if is_table_header(&all, i) => {
                let mut rows = vec![row(text)];
                let mut j = i + 2;
                while rows.len() < ROWS && all.get(j).is_some_and(|l| l.block == Block::TableRow) {
                    rows.push(row(&md[all[j].start..all[j].end]));
                    j += 1;
                }
                rich = Some(Rich::Table(rows));
            }
            Block::Bullet | Block::Ordered | Block::Task { .. } => {
                let mut items = Vec::new();
                let mut j = i;
                while items.len() < ROWS && all.get(j).is_some_and(|l| l.block.is_item()) {
                    let l = all[j];
                    let words = plain(&md[l.start..l.end]).trim().to_string();
                    let mark = match l.block {
                        Block::Ordered => Mark::Number(md[l.start..l.content()].trim().to_string()),
                        Block::Task { done } => Mark::Task { done },
                        _ => Mark::Bullet,
                    };
                    items.push(Item { mark, depth: l.depth, text: words });
                    j += 1;
                }
                rich = Some(Rich::List(items));
            }
            Block::Body if crate::images::embed_in(text).is_some() => {
                rich = crate::images::embed_in(text).map(|p| Rich::Picture(p.to_string()));
            }
            // A block of math between `$$`s: one formula, read as text.
            Block::Math { .. } => {
                if let Some((from, to)) = kolada_markdown::math_block(&all, i) {
                    let (a, z) = kolada_markdown::math_source(md, &all, from, to);
                    let formula = kolada_markdown::math::linear(md.get(a..z.max(a)).unwrap_or(""));
                    if !formula.is_empty() && said.len() < LINES {
                        said.push(formula.chars().take(LINE_MOST).collect());
                        first = false;
                    }
                    i = to;
                }
            }
            _ => {
                let words = plain(text).trim().to_string();
                if !words.is_empty() {
                    if !(first && same(&words, &title)) && said.len() < LINES {
                        said.push(words.chars().take(LINE_MOST).collect());
                    }
                    first = false;
                }
            }
        }
        i += 1;
    }
    // A picture is what a box shows best: one further on (past a list or
    // a table, or in a list item or a quote) is shown over them.
    if !matches!(rich, Some(Rich::Picture(_)))
        && let Some(picture) = all
            .iter()
            .filter(|l| !matches!(l.block, Block::Fence | Block::Code | Block::Frontmatter))
            .find_map(|l| crate::images::embed_in(&md[l.content().min(l.end)..l.end]))
    {
        rich = Some(Rich::Picture(picture.to_string()));
    }
    let line = said.first().cloned().unwrap_or_default();
    Preview { title, line, lines: said, rich }
}

/// Whether a line says what the note's name does, the name's file-safe
/// changes aside (`a/b` is saved as `a-b`).
fn same(line: &str, name: &str) -> bool {
    let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    key(line) == key(name)
}

/// A table line's first `COLUMNS` cells as they read.
fn row(line: &str) -> Vec<String> {
    cells(line).into_iter().take(COLUMNS).map(|(a, z)| plain(&line[a..z]).trim().to_string()).collect()
}

/// The box of the note at `rel` in `vault`, read from its start, and its
/// picture's file when it shows one (found as the note finds it; the
/// vault's pictures by name are kept in `known` between boxes). None when
/// the note cannot be read.
pub fn of_note(vault: &Path, rel: &str, known: &mut Option<(PathBuf, HashMap<String, PathBuf>)>) -> Option<(Preview, Option<PathBuf>)> {
    use std::io::Read;
    let path = vault.join(rel);
    let mut bytes = Vec::new();
    std::fs::File::open(&path).ok()?.take(READ as u64).read_to_end(&mut bytes).ok()?;
    // Cut short: its last line may be cut in a letter, so it goes.
    if bytes.len() == READ
        && let Some(end) = bytes.iter().rposition(|&b| b == b'\n')
    {
        bytes.truncate(end);
    }
    let md = String::from_utf8_lossy(&bytes);
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let name = name.strip_suffix(".md").unwrap_or(name);
    let preview = preview(name, &md);
    let from = path.parent().unwrap_or(vault);
    let file = match &preview.rich {
        Some(Rich::Picture(target)) => picture_file(vault, from, target, known),
        _ => None,
    };
    Some((preview, file))
}

/// Where the picture `name`, embedded in a note in `from`, is: beside the
/// note, at the vault's top, in its attachments folder (Obsidian's
/// setting), or anywhere in the vault by its file's name.
pub fn picture_file(vault: &Path, from: &Path, name: &str, known: &mut Option<(PathBuf, HashMap<String, PathBuf>)>) -> Option<PathBuf> {
    use crate::pictures::{attachment_dir, attachment_setting, find};
    let setting = std::fs::read_to_string(vault.join(".obsidian").join("app.json")).ok().and_then(|j| attachment_setting(&j));
    if let Some(dir) = setting.map(|s| attachment_dir(vault, from, &s)) {
        let p = dir.join(name.replace("%20", " "));
        if p.is_file() {
            return Some(p);
        }
    }
    find(vault, from, name, known)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block of math reads as one formula in a box, its `$$`s gone.
    #[test]
    fn a_block_of_math_reads_as_one_formula() {
        let p = preview("Circle", "# Circle\nThe area of a circle:\n$$\nA = \\pi r^2\n$$\nAfter.\n");
        assert_eq!(p.lines, ["The area of a circle:", "A = \u{03C0}r\u{00B2}", "After."]);
        let one = preview("Sum", "# Sum\n$$\\frac{1}{2}$$\n");
        assert_eq!(one.lines, ["1/2"]);
    }

    /// Math reads as text in a box: it cannot be drawn there.
    #[test]
    fn math_in_a_box_reads_as_text() {
        let p = preview("Circle", "# Circle\nThe area is $A = \\pi r^2$ and $\\frac{1}{2}$ of $\\sqrt{x+1}$.\n");
        assert_eq!(p.line, "The area is A = \u{03C0}r\u{00B2} and 1/2 of \u{221A}(x + 1).");
        let sum = preview("Sums", "# Sums\n$\\sum_{k=1}^{n} k$ and $x_{ab}$ and $y^{2q}$.\n");
        assert_eq!(sum.line, "\u{2211}\u{2096}\u{208C}\u{2081}\u{207F} k and x\u{2090}_b and y\u{00B2}q.".replace("\u{2090}_b", "_(ab)").replace("\u{00B2}q", "^(2q)"));
    }

    /// A note with a picture shows it in its box, wherever it comes in the
    /// note's start: after a list or a table, in a list item or a quote.
    #[test]
    fn a_picture_further_on_is_what_the_box_shows() {
        let picture = |md: &str| preview("Trip", md).rich;
        let shot = Some(Rich::Picture("coast.png".into()));
        assert_eq!(picture("# Trip\n- bags\n- tickets\n\n![[coast.png]]\n"), shot, "past a list");
        assert_eq!(picture("# Trip\n| a | b |\n| - | - |\n| 1 | 2 |\n\nWords.\n![](coast.png)\n"), shot, "past a table");
        assert_eq!(picture("# Trip\n- ![[coast.png]]\n"), shot, "in a list item");
        assert_eq!(picture("# Trip\n> ![[coast.png]]\n"), shot, "in a quote");
        assert!(matches!(picture("# Trip\n- bags\n- tickets\n"), Some(Rich::List(_))), "no picture: the list");
        assert!(matches!(picture("# Trip\n- bags\n```\n![[coast.png]]\n```\n"), Some(Rich::List(_))), "one in a code block is code");
    }

    fn item(mark: Mark, depth: u8, text: &str) -> Item {
        Item { mark, depth, text: text.to_string() }
    }

    #[test]
    fn a_plain_note_shows_its_first_line_after_the_title() {
        let p = preview("Groceries run", "# Groceries run\n\nBuy **oat** milk and [[Bread|bread]].\nAnd eggs.\n");
        assert_eq!(p.title, "Groceries run");
        assert_eq!(p.line, "Buy oat milk and bread.");
        assert_eq!(p.rich, None);
        // A first line that is not the name is the note's words.
        assert_eq!(preview("Trip", "Leave at nine\nPack light").line, "Leave at nine");
        // A name saved file-safe still matches its first line.
        assert_eq!(preview("Q3-Q4 plan", "Q3/Q4 plan\nStart here").line, "Start here");
        // Frontmatter and rules are not words; an empty note has none.
        assert_eq!(preview("Log", "---\ntags: a\n---\nLog\n---\nFirst entry").line, "First entry");
        assert_eq!(preview("Empty", ""), Preview { title: "Empty".into(), line: String::new(), lines: Vec::new(), rich: None });
        // No name: the title is its first line.
        assert_eq!(preview("", "## Hello there\nworld").title, "Hello there");
    }

    #[test]
    fn a_plain_note_gives_its_lines_of_words_for_the_box_to_fill() {
        let p = preview("Walk", "# Walk\n\nThe cliff path.\n\n## Later\nTea at **four**.\n---\n> A quote\n");
        assert_eq!(p.lines, ["The cliff path.", "Later", "Tea at four.", "A quote"]);
        assert_eq!(p.line, p.lines[0], "the first line is the line");
        // At most LINES, each at most LINE_MOST characters.
        let long = format!("Many\n{}", (0..20).map(|i| format!("{i} {}", "word ".repeat(80))).collect::<Vec<_>>().join("\n"));
        let p = preview("Many", &long);
        assert_eq!(p.lines.len(), LINES);
        assert!(p.lines.iter().all(|l| l.chars().count() <= LINE_MOST));
        assert!(p.lines[0].starts_with("0 ") && p.lines[LINES - 1].starts_with(&format!("{} ", LINES - 1)));
        // A note of a title alone has none.
        assert!(preview("Alone", "# Alone\n").lines.is_empty());
    }

    #[test]
    fn the_first_picture_table_or_list_shows_in_place_of_the_line() {
        let p = preview("Cats", "Cats\nSome words first.\n![[tabby 1.png|300]]\n- a\n");
        assert_eq!(p.rich, Some(Rich::Picture("tabby 1.png".into())));
        assert_eq!(p.line, "Some words first.");
        assert_eq!(preview("A", "A\n![a cat](cats/tabby.webp)\n").rich, Some(Rich::Picture("cats/tabby.webp".into())));
        // A picture inside a line, or on the web, is not the box's.
        assert_eq!(preview("A", "A\nsee ![[x.png]] here\n![](https://e.com/a.png)\n").rich, None);

        let list = preview("Plan", "Plan\n- [x] **Book** rooms\n  - [ ] pay\n1. three\n2. four\n");
        assert_eq!(list.rich, Some(Rich::List(vec![item(Mark::Task { done: true }, 0, "Book rooms"), item(Mark::Task { done: false }, 1, "pay"), item(Mark::Number("1.".into()), 0, "three")])));
        assert_eq!(list.rich.as_ref().map(Rich::kind), Some("list"));
        // A list shorter than the rows ends where it ends.
        assert_eq!(preview("B", "B\n* one\n\nafter").rich, Some(Rich::List(vec![item(Mark::Bullet, 0, "one")])));
    }

    #[test]
    fn a_table_shows_its_header_and_first_rows_and_columns() {
        let md = "Costs\n| Item | **Cost** | Qty | Note |\n| --- | --: | :-: | --- |\n| Tea | 3 | 1 | x |\n| Milk | 2 | 2 | y |\n| Rice | 9 | 1 | z |\n";
        let p = preview("Costs", md);
        assert_eq!(p.rich, Some(Rich::Table(vec![vec!["Item".into(), "Cost".into(), "Qty".into()], vec!["Tea".into(), "3".into(), "1".into()], vec!["Milk".into(), "2".into(), "2".into()]])));
        assert_eq!(p.rich.as_ref().map(Rich::kind), Some("table"));
        assert_eq!(p.rich.map(|r| if let Rich::Table(t) = r { t.len() } else { 0 }), Some(ROWS));
        // Whichever comes first wins: here the list.
        assert_eq!(preview("C", "C\n- first\n\n| a | b |\n| - | - |\n| 1 | 2 |\n").rich.as_ref().map(Rich::kind), Some("list"));
    }

    #[test]
    fn a_note_is_read_from_its_file_with_its_picture_found() {
        let vault = std::env::temp_dir().join(format!("kolada-ui-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        std::fs::create_dir_all(vault.join("Trips/assets")).unwrap();
        std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
        std::fs::write(vault.join(".obsidian/app.json"), r#"{"attachmentFolderPath":"./assets"}"#).unwrap();
        std::fs::write(vault.join("Trips/assets/sea.png"), b"\x89PNG").unwrap();
        std::fs::write(vault.join("Trips/Coast.md"), "Coast\n![[sea.png]]\n").unwrap();
        std::fs::write(vault.join("Words.md"), "Words\nhello\n").unwrap();
        let mut known = None;
        let (p, file) = of_note(&vault, "Trips/Coast.md", &mut known).unwrap();
        assert_eq!(p.title, "Coast");
        assert_eq!(file, Some(vault.join("Trips/assets/sea.png")));
        let (p, file) = of_note(&vault, "Words.md", &mut known).unwrap();
        assert_eq!((p.line.as_str(), file), ("hello", None));
        assert!(of_note(&vault, "Gone.md", &mut known).is_none());
        let _ = std::fs::remove_dir_all(&vault);
    }
}
