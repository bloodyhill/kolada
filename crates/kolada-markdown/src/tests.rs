use super::*;

fn blocks(text: &str) -> Vec<(Block, usize, u8)> {
    lines(text).map(|l| (l.block, l.mark, l.depth)).collect()
}

/// (style, whole span, shown text) for each span of `line`.
fn shown(line: &str) -> Vec<(Style, &str, &str)> {
    spans(line).into_iter().map(|s| (s.style, &line[s.start..s.end], &line[s.text_start..s.text_end])).collect()
}

#[test]
fn reads_block_marks() {
    use Block::*;
    assert_eq!(block("# Title"), (Heading(1), 2, 0));
    assert_eq!(block("###   Deep"), (Heading(3), 6, 0));
    assert_eq!(block("#tag"), (Body, 0, 0));
    assert_eq!(block("####### seven"), (Body, 0, 0));
    assert_eq!(block("- milk"), (Bullet, 2, 0));
    assert_eq!(block("\t* nested"), (Bullet, 3, 1));
    assert_eq!(block("    + deeper"), (Bullet, 6, 2));
    assert_eq!(block("- [ ] call"), (Task { done: false }, 6, 0));
    assert_eq!(block("- [x] done"), (Task { done: true }, 6, 0));
    assert_eq!(block("- [ ]"), (Task { done: false }, 5, 0));
    assert_eq!(block("- [link](x)"), (Bullet, 2, 0));
    assert_eq!(block("12. twelve"), (Ordered, 4, 0));
    assert_eq!(block("1) one"), (Ordered, 3, 0));
    assert_eq!(block("2024. not a list? it is"), (Ordered, 6, 0));
    assert_eq!(block("> quote"), (Quote, 2, 1));
    assert_eq!(block(">> nested"), (Quote, 3, 2));
    assert_eq!(block("---"), (Rule, 0, 0));
    assert_eq!(block("* * *"), (Rule, 0, 0));
    assert_eq!(block("-- two"), (Body, 0, 0));
    assert_eq!(block("-not a bullet"), (Body, 0, 0));
    assert_eq!(block("plain"), (Body, 0, 0));
}

#[test]
fn tracks_frontmatter_and_fences_across_lines() {
    use Block::*;
    let text = "---\ntags: [a]\n---\n# Title\n```rust\n# not a heading\n- not a bullet\n```\n- item\n";
    let got: Vec<Block> = blocks(text).into_iter().map(|b| b.0).collect();
    assert_eq!(got, [Frontmatter, Frontmatter, Frontmatter, Heading(1), Fence, Code, Code, Fence, Bullet, Body]);
    // A fence closes only with the same character, at least as long.
    let got: Vec<Block> = lines("````\n```\n~~~\n````").map(|l| l.block).collect();
    assert_eq!(got, [Fence, Code, Code, Fence]);
    let starts: Vec<usize> = lines("a\nbc\n").map(|l| l.start).collect();
    assert_eq!(starts, [0, 2, 5]);
    assert_eq!(lines("").count(), 1);
}

#[test]
fn continues_lists_and_quotes() {
    let next = |line: &str| {
        let (b, m, _) = block(line);
        continuation(line, b, m)
    };
    assert_eq!(next("- milk").as_deref(), Some("- "));
    assert_eq!(next("\t* x").as_deref(), Some("\t* "));
    assert_eq!(next("- [x] done").as_deref(), Some("- [ ] "));
    assert_eq!(next("9. nine").as_deref(), Some("10. "));
    assert_eq!(next("3) three").as_deref(), Some("4) "));
    assert_eq!(next("> said").as_deref(), Some("> "));
    assert_eq!(next("# Title"), None);
    assert_eq!(next("text"), None);
}

#[test]
fn finds_emphasis_and_its_marks() {
    use Style::*;
    assert_eq!(shown("a **bold** b"), [(Strong, "**bold**", "bold")]);
    assert_eq!(shown("*it* and _it_"), [(Emphasis, "*it*", "it"), (Emphasis, "_it_", "it")]);
    assert_eq!(shown("~~gone~~ ==marked=="), [(Strike, "~~gone~~", "gone"), (Highlight, "==marked==", "marked")]);
    assert_eq!(shown("***both***"), [(Strong, "***both***", "both"), (Emphasis, "both", "both")]);
    assert_eq!(shown("*outer **inner** outer*"), [(Emphasis, "*outer **inner** outer*", "outer **inner** outer"), (Strong, "**inner**", "inner")]);
}

#[test]
fn leaves_what_is_not_emphasis() {
    assert_eq!(shown("2 * 3 * 4"), []);
    assert_eq!(shown("snake_case_name"), []);
    assert_eq!(shown("**unclosed"), []);
    assert_eq!(shown("\\*not\\*"), []);
    assert_eq!(shown("** spaced **"), []);
}

#[test]
fn keeps_code_and_links_whole() {
    use Style::*;
    assert_eq!(shown("run `a *b* c` now"), [(Code, "`a *b* c`", "a *b* c")]);
    assert_eq!(shown("``a ` b``"), [(Code, "``a ` b``", "a ` b")]);
    assert_eq!(shown("see [[Plan]]"), [(WikiLink, "[[Plan]]", "Plan")]);
    assert_eq!(shown("see [[Plan|the plan]]"), [(WikiLink, "[[Plan|the plan]]", "the plan")]);
    assert_eq!(shown("![[image.png]]"), [(WikiLink, "![[image.png]]", "image.png")]);
    assert_eq!(shown("[site](https://a.b/c_d_e)"), [(Link, "[site](https://a.b/c_d_e)", "site")]);
    assert_eq!(shown("**see [[x]]**"), [(Strong, "**see [[x]]**", "see [[x]]"), (WikiLink, "[[x]]", "x")]);
    assert_eq!(shown("[not a link] here"), []);
}

#[test]
fn finds_tags_but_not_numbers_or_anchors() {
    use Style::*;
    assert_eq!(shown("#idea and #work/deep"), [(Tag, "#idea", "#idea"), (Tag, "#work/deep", "#work/deep")]);
    assert_eq!(shown("issue #42"), []);
    assert_eq!(shown("a#b"), []);
    assert_eq!(shown("#تجربة"), [(Tag, "#تجربة", "#تجربة")]);
}

#[test]
fn reads_lines_without_their_marks() {
    assert_eq!(plain("# **Big** plan"), "Big plan");
    assert_eq!(plain("- [ ] call [[Mom|mom]]"), "call mom");
    assert_eq!(plain("> see [site](http://x)"), "see site");
    assert_eq!(plain("`code` #tag"), "code #tag");
}

#[test]
fn places_line_spans_in_the_whole_text() {
    let text = "intro\n- **x**";
    let line = lines(text).nth(1).unwrap();
    let s = line_spans(text, &line);
    assert_eq!(s.len(), 1);
    assert_eq!(&text[s[0].start..s[0].end], "**x**");
    assert_eq!(line.content(), 8);
}

#[test]
fn finds_web_addresses_written_out() {
    use Style::*;
    assert_eq!(shown("see https://x.io/a_b_c."), [(Url, "https://x.io/a_b_c", "https://x.io/a_b_c")]);
    assert_eq!(shown("(www.x.io) and http://a.b/c?d=e#f"), [(Url, "www.x.io", "www.x.io"), (Url, "http://a.b/c?d=e#f", "http://a.b/c?d=e#f")]);
    assert_eq!(shown("wiki https://en.wikipedia.org/wiki/Rust_(language) ok"), [(Url, "https://en.wikipedia.org/wiki/Rust_(language)", "https://en.wikipedia.org/wiki/Rust_(language)")]);
    // Not inside words, links or code; and emphasis around one still works.
    assert_eq!(shown("xhttps://a.b"), []);
    assert_eq!(shown("[site](https://a.b)"), [(Link, "[site](https://a.b)", "site")]);
    assert_eq!(shown("`https://a.b`"), [(Code, "`https://a.b`", "https://a.b")]);
    assert_eq!(shown("**https://a.b**"), [(Strong, "**https://a.b**", "https://a.b"), (Url, "https://a.b", "https://a.b")]);
    assert_eq!(shown("www. not"), []);
}


#[test]
fn reading_again_after_an_edit_matches_reading_it_all() {
    let base = "---\ntags: a\n---\n# Title\nSome *text* here.\n```rust\nfn main() {}\n```\n- [ ] task\n> quote\n\n~~~\ncode ~~~\n~~~\nend";
    // A small, fixed pseudo-random walk of edits: inserts, deletes and
    // replacements of text with and without line breaks and fence marks.
    let pieces = ["a", "\n", "```", "---\n", "# ", "\n\n", "~~~\n", "- ", "x\ny", ""];
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut text = base.to_string();
    let (mut lines, mut states): (Vec<Line>, Vec<LexState>) = lines_with_states(&text).unzip();
    for step in 0..3000 {
        let at = next(text.len() + 1);
        let at = (0..=at).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
        let removed = next(4).min(text.len() - at);
        let removed = (0..=removed).rev().find(|&r| text.is_char_boundary(at + r)).unwrap_or(0);
        let inserted = pieces[next(pieces.len())];
        text.replace_range(at..at + removed, inserted);
        relex(&text, &mut lines, &mut states, at, removed, inserted.len());
        let (want_lines, want_states): (Vec<Line>, Vec<LexState>) = lines_with_states(&text).unzip();
        assert_eq!(lines, want_lines, "lines after edit {step} at {at} ({removed} removed, {inserted:?} in): {text:?}");
        assert_eq!(states, want_states, "states after edit {step}");
        if text.len() > 400 {
            text = base.to_string();
            (lines, states) = lines_with_states(&text).unzip();
        }
    }
}

#[test]
fn tables_are_a_header_an_alignment_row_and_rows() {
    let text = "Intro\n| Item | Owner | Cost |\n| --- | :-: | --: |\n| Press kit | Sam | 120 |\n| Video | Omar |\nAfter\n";
    let lines: Vec<Line> = lines(text).collect();
    let blocks: Vec<Block> = lines.iter().map(|l| l.block).collect();
    assert_eq!(blocks, [Block::Body, Block::Body, Block::TableDelimiter, Block::TableRow, Block::TableRow, Block::Body, Block::Body]);
    assert!(is_table_header(&lines, 1) && !is_table_header(&lines, 0));
    let delimiter = &text[lines[2].start..lines[2].end];
    assert_eq!(alignments(delimiter), Some(vec![Align::Left, Align::Center, Align::Right]));
    let row = &text[lines[3].start..lines[3].end];
    let words: Vec<&str> = cells(row).into_iter().map(|(a, z)| &row[a..z]).collect();
    assert_eq!(words, ["Press kit", "Sam", "120"]);
}

#[test]
fn a_table_needs_its_alignment_row_and_matching_cells() {
    // No alignment row: plain lines with pipes.
    let blocks: Vec<Block> = lines("a | b\nc | d\n").map(|l| l.block).collect();
    assert_eq!(blocks, [Block::Body, Block::Body, Block::Body]);
    // An alignment row with fewer cells than the header is no table.
    let blocks: Vec<Block> = lines("| a | b | c |\n| --- | --- |\n").map(|l| l.block).collect();
    assert_eq!(blocks[1], Block::Body);
    // Pipes escaped, outer pipes optional, cells empty.
    let row = r"a \| b | | c";
    let words: Vec<&str> = cells(row).into_iter().map(|(x, z)| &row[x..z]).collect();
    assert_eq!(words, [r"a \| b", "", "c"]);
    let row = "| one |";
    assert_eq!(cells(row).len(), 1);
}

#[test]
fn editing_a_table_reads_again_as_reading_it_all() {
    let before = "| a | b |\n| - | - |\n| 1 | 2 |\nend\n";
    let mut lines_ = Vec::new();
    let mut states = Vec::new();
    for (l, s) in lines_with_states(before) {
        lines_.push(l);
        states.push(s);
    }
    // A third cell in the header: the alignment row no longer matches.
    let at = before.find(" |").unwrap() + 2;
    let after = format!("{} c |{}", &before[..at], &before[at..]);
    relex(&after, &mut lines_, &mut states, at, 0, 4);
    let fresh: Vec<Line> = lines(&after).collect();
    assert_eq!(lines_, fresh);
    assert_eq!(fresh[1].block, Block::Body);
}

#[test]
fn frontmatter_is_properties_and_a_rule_first_is_a_rule() {
    // Properties: keys, lists, indented values, blank lines.
    let props = "---\ntitle: Launch plan\ntags:\n  - work\n- beta\naliases: [plan]\ncreated at: 2026-10-01\n\n---\n# Launch\n";
    assert!(has_frontmatter(props));
    let blocks: Vec<Block> = lines(props).map(|l| l.block).collect();
    assert!(blocks[..9].iter().all(|b| *b == Block::Frontmatter), "{blocks:?}");
    assert_eq!(blocks[9], Block::Heading(1));
    assert!(has_frontmatter("---\n---\nBody") && has_frontmatter("---"));
    // A rule over headings and paragraphs: the note, not properties.
    let ruled = "---\n\n## 11. Funding: who pays\n\nSome text: with a colon. And more.\n\n---\n\n## 12. Next\n";
    assert!(!has_frontmatter(ruled));
    let blocks: Vec<Block> = lines(ruled).map(|l| l.block).collect();
    assert_eq!(blocks[0], Block::Rule);
    assert_eq!(blocks[2], Block::Heading(2));
    assert_eq!(blocks[4], Block::Body);
    // Never closed: text.
    assert!(!has_frontmatter("---\ntitle: x\nno end"));
    // A sentence before a colon is no key.
    assert!(!has_frontmatter("---\nThis is a sentence, really. It goes: on\n---\n"));
}

#[test]
fn an_edit_that_makes_or_unmakes_frontmatter_reads_the_note_again() {
    let before = "---\ntitle: x\n---\nBody\n";
    let mut lines_: Vec<Line> = Vec::new();
    let mut states = Vec::new();
    for (l, s) in lines_with_states(before) {
        lines_.push(l);
        states.push(s);
    }
    // A heading typed into it: no longer properties.
    let at = before.find("title").unwrap();
    let after = format!("{}## {}", &before[..at], &before[at..]);
    relex(&after, &mut lines_, &mut states, at, 0, 3);
    assert_eq!(lines_, lines(&after).collect::<Vec<_>>());
    assert_eq!(lines_[0].block, Block::Rule);
}

/// What `hidden` leaves of each line: the text as clean editing shows it.
fn clean(text: &str) -> Vec<String> {
    lines(text)
        .map(|l| {
            let hide = hidden(text, &l);
            text[l.start..l.end].char_indices().filter(|&(i, _)| !hide.iter().any(|&(a, b)| a <= l.start + i && l.start + i < b)).map(|(_, c)| c).collect()
        })
        .collect()
}

#[test]
fn hidden_ranges_cover_every_mark_kind() {
    let text = "## Plan\n- **bold** and *it* ~~gone~~ ==marked== `code`\n> [site](https://a.b) and [[Note|alias]]\n---\n```rust\nlet x = 1;\n```\n![a photo](https://a.b/p.png)\n1. [ ] not a task";
    assert_eq!(clean(text), ["Plan", "bold and it gone marked code", "site and alias", "", "", "let x = 1;", "", "a photo", "[ ] not a task"]);
    // A heading's #s are kept apart from the inline marks after them, so
    // the caret can tell where its text starts.
    let line = lines("# **x**").next().unwrap();
    assert_eq!(hidden("# **x**", &line), [(0, 2), (2, 4), (5, 7)]);
    // Marks that touch hide as one run.
    let text = "==a==**b**";
    let line = lines(text).next().unwrap();
    assert_eq!(hidden(text, &line), [(0, 2), (3, 7), (8, 10)]);
    // Unclosed marks are text, and so is a tag or an address.
    assert_eq!(clean("**unclosed #tag https://a.b"), ["**unclosed #tag https://a.b"]);
}

#[test]
fn escapes_hide_their_backslash() {
    assert_eq!(escapes(r"\*not\* a \ b"), [0, 5]);
    assert_eq!(clean(r"\*not emphasis\* and \# not a tag"), ["*not emphasis* and # not a tag"]);
    assert_eq!(clean(r"a \\ b"), [r"a \ b"], "an escaped backslash shows one");
    // In code a backslash is a backslash.
    assert_eq!(clean(r"`a\*b` \_"), [r"a\*b _"]);
    // An escaped ! is no picture's.
    assert_eq!(clean(r"\![a](b)"), ["!a"]);
}
