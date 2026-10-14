//! Structure: lists written in a run, and list marks typed by hand.
//!
//! Only marks and line breaks are added: every item keeps the words it was
//! written with, and an answer that would drop, change or move any of them
//! is refused.

use crate::text::{self, Kind};
use kolada_markdown::{self as md, Block};

/// Marks people type for a bullet, a space after them. Not dashes: a line of
/// dialogue opens with one in Spanish, French and more.
const BULLETS: [char; 7] = ['•', '◦', '▪', '●', '‣', '▸', '·'];
/// What separates items written in a run.
const SEPARATORS: [char; 6] = [',', ';', '،', '、', '；', '，'];
/// What ends a sentence.
const ENDS: [char; 7] = ['.', '!', '?', '。', '！', '？', '؟'];
/// Words that join the last item on (`milk, eggs and bread`), dropped with
/// the separators. Only these: any other word between items (`but not`) is
/// part of what was written.
const JOINS: [&str; 24] = [
    "and", "or", "&", "+", "et", "ou", "y", "e", "o", "u", "und", "oder", "و", "أو", "او", "和", "与", "及", "或", "や", "と", "및", "또는", "그리고",
];
/// Longer runs between separators are clauses, not items.
const MAX_ITEM_WORDS: usize = 8;

/// A Body line (its whole text) typed with a hand-made list mark: the line
/// with a Markdown one. `• x`/`◦ x`/`▪ x` → `- x`; `[] x`/`[ ] x`
/// → `- [ ] x`; `[x] x`/`[X] x` → `- [x] x`; `todo: x` (any case, colon
/// required) and `TODO x` (upper case) → `- [ ] x`. Keeps leading
/// indentation. None otherwise (including lines that already are Markdown
/// items, `-5 degrees`, `Todo list for today`).
pub fn rules(line: &str) -> Option<String> {
    if md::block(line).0 != Block::Body {
        return None;
    }
    let rest = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - rest.len()];
    let item = |mark: &str, after: &str| {
        let words = after.trim_start();
        (after.starts_with([' ', '\t']) && !words.is_empty()).then(|| format!("{indent}{mark}{words}"))
    };
    if let Some(c) = rest.chars().next().filter(|c| BULLETS.contains(c)) {
        return item("- ", &rest[c.len_utf8()..]);
    }
    for (task, mark) in [("[]", "- [ ] "), ("[ ]", "- [ ] "), ("[x]", "- [x] "), ("[X]", "- [x] ")] {
        if let Some(after) = rest.strip_prefix(task) {
            return item(mark, after);
        }
    }
    if rest.get(..5).is_some_and(|h| h.eq_ignore_ascii_case("todo:")) {
        return item("- [ ] ", &format!(" {}", &rest[5..]));
    }
    item("- [ ] ", rest.strip_prefix("TODO")?)
}

/// Cheap: whether a Body paragraph's content looks like a run of items
/// worth asking the model about — ≥ 2 separators (`,` `;` `،` `、` `；` `，`)
/// with ≥ 3 short segments (avg ≤ 6 words, none over 8) and no sentence end
/// (`.!?。！？؟`) inside other than at the very end. A lead-in up to a colon
/// is not a segment. Most prose → false.
pub fn worth_asking(content: &str) -> bool {
    let run = content.trim().trim_end_matches(ENDS);
    if run.contains(ENDS) {
        return false;
    }
    let first = run.find(SEPARATORS).unwrap_or(run.len());
    let run = match run[..first].rfind([':', '：']) {
        Some(colon) => &run[colon + run[colon..].chars().next().map_or(1, char::len_utf8)..],
        None => run,
    };
    let (mut segments, mut words, mut longest) = (0, 0, 0);
    for part in run.split(SEPARATORS) {
        let n = text::words(part);
        if n > 0 {
            segments += 1;
            words += n;
            longest = longest.max(n);
        }
    }
    run.matches(SEPARATORS).count() >= 2 && segments >= 3 && words <= 6 * segments && longest <= MAX_ITEM_WORDS
}

/// The model's answer to Shape (`KEEP`, or `LIST` followed by `\n- item`
/// lines) as the paragraph's new text, or None.
///
/// Accepted only if there are ≥ 3 items, each (trimmed, trailing `,;.،、`
/// removed) found verbatim in `content`, in order, at word edges; between
/// items only separators, space and at most one joining word; after the
/// last only punctuation; before the first nothing, or a lead-in ending in
/// a colon, which becomes the first line.
pub fn list(content: &str, answer: &str) -> Option<String> {
    let answer = answer.trim();
    let mut lines = answer.strip_suffix("<|im_end|>").unwrap_or(answer).lines().map(str::trim).filter(|l| !l.is_empty());
    if !matches!(lines.next()?, "LIST" | "LIST:") {
        return None;
    }
    let items: Vec<&str> = lines.map(|l| Some(l.strip_prefix("- ")?.trim().trim_end_matches([',', ';', '.', '،', '、', '；', '，']).trim_end())).collect::<Option<_>>()?;
    if items.len() < 3 || items.iter().any(|i| i.is_empty()) {
        return None;
    }
    let cuts: Vec<usize> = text::tokens(content).iter().map(|t| t.start).chain([content.len()]).collect();
    let cut = |at: usize| cuts.binary_search(&at).is_ok();
    let (mut at, mut lead) = (0, None);
    for (k, item) in items.iter().enumerate() {
        let found = content[at..].match_indices(item).map(|(i, _)| at + i).find_map(|start| {
            let end = start + item.len();
            if !cut(end) {
                return None;
            }
            let gap = &content[at..start];
            // Arabic joins the last item with a `و` written onto it.
            let (gap, joined) = if cut(start) { (gap, false) } else { (gap.strip_suffix('و').filter(|g| cut(at + g.len()))?, true) };
            if k > 0 {
                return joins(gap, joined).then_some((start, end, None));
            }
            let l = gap.trim_end();
            match (joined, l.is_empty()) {
                (false, true) => Some((start, end, None)),
                (false, false) if l.ends_with([':', '：']) => Some((start, end, Some(l))),
                _ => None,
            }
        });
        let (_, end, l) = found?;
        lead = lead.or(l);
        at = end;
    }
    if !content[at..].chars().all(|c| c.is_whitespace() || ENDS.contains(&c) || SEPARATORS.contains(&c) || c == '…') {
        return None;
    }
    let mut out: Vec<String> = lead.map(str::to_string).into_iter().collect();
    out.extend(items.iter().map(|i| format!("- {i}")));
    Some(out.join("\n"))
}

/// Whether the text between two items is only separators, space and at most
/// one joining word, with at least one separator or joining word.
fn joins(gap: &str, mut joined: bool) -> bool {
    let mut apart = joined;
    for t in text::tokens(gap) {
        if t.kind == Kind::Space {
            continue;
        }
        if t.text.chars().all(|c| SEPARATORS.contains(&c)) {
            apart = true;
        } else if !joined && JOINS.contains(&t.text.to_lowercase().as_str()) {
            (joined, apart) = (true, true);
        } else {
            return false;
        }
    }
    apart
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hand_made_marks_become_markdown() {
        for (line, want) in [
            ("• milk", "- milk"),
            ("◦ milk", "- milk"),
            ("▪ milk", "- milk"),
            ("•   spaced", "- spaced"),
            ("  • nested", "  - nested"),
            ("\t• tabbed", "\t- tabbed"),
            ("[] call mum", "- [ ] call mum"),
            ("[ ] call mum", "- [ ] call mum"),
            ("[x] done", "- [x] done"),
            ("[X] done", "- [x] done"),
            ("todo: call mum", "- [ ] call mum"),
            ("TODO: call mum", "- [ ] call mum"),
            ("Todo:call mum", "- [ ] call mum"),
            ("TODO call mum", "- [ ] call mum"),
            ("  todo: nested", "  - [ ] nested"),
            ("• حليب", "- حليب"),
        ] {
            assert_eq!(rules(line).as_deref(), Some(want), "{line:?}");
        }
        for line in [
            "- milk", "* milk", "1. milk", "- [ ] milk", "-5 degrees", "–5 degrees", "—no space", "— Bonjour, dit-il.", "– milk", "Todo list for today", "todo list", "TODOs are due", "TODO", "todo:",
            "todo:  ", "•", "• ", "[x]", "[x]done", "[link](https://x.io) here", "plain text", "# heading", "> quote", "",
        ] {
            assert_eq!(rules(line), None, "{line:?}");
        }
    }

    #[test]
    fn runs_of_items_are_worth_asking_about() {
        for run in [
            "milk, eggs, bread",
            "Groceries: milk, eggs, bread and butter",
            "Things to pack for the long weekend away: socks, charger, passport",
            "milk; eggs; bread.",
            "pommes, poires et bananes, raisins",
            "leche, huevos, pan y queso",
            "حليب، بيض، خبز",
            "苹果、香蕉、牛奶",
            "call the plumber, pay rent, book flights to Lisbon",
        ] {
            assert!(worth_asking(run), "{run:?}");
        }
        for prose in [
            "I went to the store and bought milk.",
            "When I got home, I made dinner, and then I watched a film with my sister.",
            "Yes. I know, I know, fine.",
            "It weighs 1.5 kg, 2 kg, 3 kg",
            "milk, eggs",
            "Things went well, mostly.",
            "",
        ] {
            assert!(!worth_asking(prose), "{prose:?}");
        }
    }

    #[test]
    fn a_run_becomes_a_list_with_its_own_words() {
        for (content, answer, want) in [
            ("Groceries: milk, eggs, bread and butter", "LIST\n- milk\n- eggs\n- bread\n- butter", "Groceries:\n- milk\n- eggs\n- bread\n- butter"),
            ("milk, eggs, bread.", "LIST\n- milk,\n- eggs,\n- bread.<|im_end|>", "- milk\n- eggs\n- bread"),
            ("Buy milk: milk, oat milk, eggs", "LIST\n- milk\n- oat milk\n- eggs", "Buy milk:\n- milk\n- oat milk\n- eggs"),
            ("pommes, poires et bananes", "LIST\n- pommes\n- poires\n- bananes", "- pommes\n- poires\n- bananes"),
            ("leche, huevos y pan", "LIST\n- leche\n- huevos\n- pan", "- leche\n- huevos\n- pan"),
            ("Milch, Eier, Brot und Käse", "LIST\n- Milch\n- Eier\n- Brot\n- Käse", "- Milch\n- Eier\n- Brot\n- Käse"),
            ("arroz, feijão e salada", "LIST\n- arroz\n- feijão\n- salada", "- arroz\n- feijão\n- salada"),
            ("حليب، بيض، خبز، وجبن", "LIST\n- حليب\n- بيض\n- خبز\n- جبن", "- حليب\n- بيض\n- خبز\n- جبن"),
            ("حليب، بيض، وخبز", "LIST\n- حليب\n- بيض\n- وخبز", "- حليب\n- بيض\n- وخبز"),
            ("买：苹果、香蕉、牛奶和面包", "LIST\n- 苹果\n- 香蕉\n- 牛奶\n- 面包", "买：\n- 苹果\n- 香蕉\n- 牛奶\n- 面包"),
            ("りんご、バナナ、みかん", "LIST\n- りんご\n- バナナ\n- みかん", "- りんご\n- バナナ\n- みかん"),
            ("사과, 배, 그리고 포도", "LIST\n- 사과\n- 배\n- 포도", "- 사과\n- 배\n- 포도"),
            ("call the plumber; pay **rent**; book [[Lisbon]] flights", "LIST\n- call the plumber\n- pay **rent**\n- book [[Lisbon]] flights", "- call the plumber\n- pay **rent**\n- book [[Lisbon]] flights"),
        ] {
            assert_eq!(list(content, answer).as_deref(), Some(want), "{content:?}");
        }
    }

    #[test]
    fn answers_that_change_the_words_are_refused() {
        for (content, answer) in [
            ("milk, eggs, bread", "KEEP"),
            ("milk, eggs, bread", "Here is the list:\n- milk\n- eggs\n- bread"),
            ("milk, eggs, bread", "LIST\nmilk\neggs\nbread"),
            ("milk and eggs", "LIST\n- milk\n- eggs"),
            ("milk, eggs, bread", "LIST\n- eggs\n- milk\n- bread"),
            ("milk, eggs, bread", "LIST\n- Milk\n- Eggs\n- Bread"),
            ("milk, eggs, bread", "LIST\n- milk\n- eggs\n- bread\n- butter"),
            ("milk, eggs, bread", "LIST\n- milk\n- \n- bread"),
            ("milk, eggs, but not bread", "LIST\n- milk\n- eggs\n- bread"),
            ("milk, eggs, then bread", "LIST\n- milk\n- eggs\n- bread"),
            ("fresh milk, brown eggs, bread", "LIST\n- fresh\n- milk\n- brown eggs\n- bread"),
            ("milk, eggs, bread. Then go home", "LIST\n- milk\n- eggs\n- bread"),
            ("milk, eggs, bread 🙂", "LIST\n- milk\n- eggs\n- bread"),
            ("I need milk, eggs, bread", "LIST\n- milk\n- eggs\n- bread"),
            ("apples, bananas, cherries", "LIST\n- apple\n- bananas\n- cherries"),
            ("apples, bananas, cherries", "LIST\n- pples\n- bananas\n- cherries"),
            ("milk, eggs and and bread", "LIST\n- milk\n- eggs\n- bread"),
            ("حليب، بيض، خبز", "LIST\n- حليب\n- يض\n- خبز"),
        ] {
            assert_eq!(list(content, answer), None, "{content:?} -> {answer:?}");
        }
    }
}
