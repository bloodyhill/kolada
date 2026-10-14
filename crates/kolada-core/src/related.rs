//! Magnetic memory: which notes relate to what is being written.
//!
//! Every note becomes a weighted bag of its words (TF-IDF: a word counts
//! for more the more a note uses it and the fewer other notes do). What is
//! being written is weighed the same way, the paragraph at the caret twice
//! over, and the notes whose words overlap most with it are related. Notes
//! that link here count too. Notes this one already links to are left out:
//! the margin is for what has not been connected yet. Marked passages
//! (`==highlights==`) from other notes resurface when they speak to the
//! paragraph at hand.
//!
//! All of it runs on the device from the search index, in well under a
//! millisecond per query for thousands of notes. Nothing leaves the machine.

use std::collections::HashMap;

use crate::search::Indexed;

/// Common words that say nothing about what a note is about.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "your", "all", "any", "can", "had", "her", "was", "one", "our",
    "out", "has", "have", "him", "his", "how", "its", "may", "new", "now", "old", "see", "two", "way", "who", "did",
    "get", "got", "let", "put", "say", "she", "too", "use", "used", "using", "that", "with", "this", "from", "they",
    "will", "would", "there", "their", "what", "about", "which", "when", "make", "like", "time", "just", "know",
    "take", "into", "year", "good", "some", "could", "them", "than", "then", "look", "only", "come", "over",
    "think", "also", "back", "after", "work", "first", "well", "even", "want", "because", "these", "give", "most",
    "very", "been", "were", "said", "each", "does", "done", "doing", "should", "here", "where", "more", "much",
    "many", "such", "those", "other", "being", "while", "still", "into", "onto", "upon", "yes", "yet", "off",
    "own", "same", "every", "both", "few", "again", "once", "why", "whom", "whose", "itself", "myself", "http",
    "https", "www", "com", "org", "net", "html", "png", "jpg", "jpeg", "gif", "pdf", "true", "false", "null",
    "في", "من", "على", "إلى", "عن", "مع", "هذا", "هذه", "التي", "الذي", "كان", "لكن", "أيضا", "ذلك", "هناك",
];

fn is_stopword(w: &str) -> bool {
    STOPWORDS.contains(&w)
}

/// A word's stem: close enough forms (plan, plans, planned) count as one.
fn stem(word: &str) -> String {
    let w = word;
    if !w.is_ascii() {
        return w.to_string();
    }
    let n = w.len();
    let cut = |k: usize| w[..n - k].to_string();
    if n > 5 && w.ends_with("ies") {
        return format!("{}y", &w[..n - 3]);
    }
    if n > 6 && w.ends_with("ing") {
        return trim_double(&cut(3));
    }
    if n > 5 && w.ends_with("ed") {
        return trim_double(&cut(2));
    }
    if n > 4 && w.ends_with("es") && (w.ends_with("ches") || w.ends_with("shes") || w.ends_with("sses") || w.ends_with("xes")) {
        return cut(2);
    }
    if n > 3 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") && !w.ends_with("is") {
        return cut(1);
    }
    w.to_string()
}

/// `plann` (from planning) back to `plan`.
fn trim_double(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() > 3 && b[b.len() - 1] == b[b.len() - 2] && !matches!(b[b.len() - 1], b'l' | b's' | b'z') {
        s[..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// The words of a text that carry meaning, as (stem, the word as written).
pub fn words(text: &str) -> impl Iterator<Item = (String, &str)> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !w.chars().all(|c| c.is_ascii_digit()))
        .filter_map(|w| {
            let lower = w.to_lowercase();
            (!is_stopword(&lower)).then(|| (stem(&lower), w))
        })
}

/// `[[link]]` targets in a text, lowercased, without folders, headings or
/// aliases.
fn links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("[[") {
        rest = &rest[i + 2..];
        let Some(j) = rest.find("]]") else { break };
        let target = rest[..j].split(['|', '#', '^']).next().unwrap_or("");
        let name = target.rsplit('/').next().unwrap_or(target).trim().to_lowercase();
        if !name.is_empty() {
            out.push(name);
        }
        rest = &rest[j + 2..];
    }
    out
}

/// `==marked==` passages in a text.
fn marks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        while let Some(i) = rest.find("==") {
            rest = &rest[i + 2..];
            let Some(j) = rest.find("==") else { break };
            let passage = rest[..j].trim();
            if passage.chars().count() >= 3 {
                out.push(passage.to_string());
            }
            rest = &rest[j + 2..];
        }
    }
    out
}

/// Every `==marked==` passage in `notes`, as (note name, passage), grouped
/// by note in the order of their names: what `Corpus::highlights` says, for
/// a shell that has no corpus (reading the marks is far cheaper than
/// weighing every word of the vault).
pub fn gather_highlights(notes: &[crate::search::Indexed]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = notes.iter().flat_map(|n| marks(n.text()).into_iter().map(|m| (n.name.clone(), m))).collect();
    out.sort_by_key(|m| m.0.to_lowercase());
    out
}

/// The page of highlights as Markdown: a count, then each note's passages
/// as quotes under a link to it. `marks` come grouped by note.
pub fn highlights_page<A: AsRef<str>, B: AsRef<str>>(marks: &[(A, B)]) -> String {
    let mut text = String::from("# Highlights\n");
    if marks.is_empty() {
        text.push_str("Mark a passage with ==two equals signs on each side== and it gathers here.\n");
        return text;
    }
    let mut names: Vec<&str> = marks.iter().map(|m| m.0.as_ref()).collect();
    names.dedup();
    let (p, n) = (marks.len(), names.len());
    text.push_str(&format!("{p} passage{} from {n} note{}\n", if p == 1 { "" } else { "s" }, if n == 1 { "" } else { "s" }));
    let mut last = "";
    for (name, passage) in marks {
        let name = name.as_ref();
        if name != last {
            text.push_str(&format!("### [[{name}]]\n"));
            last = name;
        }
        text.push_str(&format!("> {}\n", passage.as_ref()));
    }
    text
}

/// A text's words as normalized TF-IDF weights (the most telling first is
/// not guaranteed; it is a bag).
fn weigh(counts: &HashMap<String, f32>, idf: &HashMap<String, f32>) -> Vec<(String, f32)> {
    let mut v: Vec<(String, f32)> = counts
        .iter()
        .filter_map(|(t, &c)| idf.get(t).map(|&i| (t.clone(), (1.0 + c.ln()) * i)))
        .filter(|(_, w)| *w > 0.0)
        .collect();
    let norm = v.iter().map(|(_, w)| w * w).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|(_, w)| *w /= norm);
    }
    v
}

struct Doc {
    path: String,
    name: String,
    name_lower: String,
    links: Vec<String>,
}

struct Mark {
    doc: usize,
    text: String,
    weights: Vec<(String, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Says similar things.
    Similar,
    /// It links to this note.
    Backlink,
    /// A marked passage from it.
    Highlight,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Related {
    /// Vault-relative path, with `.md`.
    pub path: String,
    pub name: String,
    pub kind: Kind,
    /// Why, in a few words: the words in common, or the marked passage.
    pub detail: String,
    pub score: f32,
}

impl Related {
    /// Stands for the same thing across queries, for animating it.
    pub fn key(&self) -> String {
        match self.kind {
            Kind::Highlight => format!("{}\u{1}{}", self.path, self.detail),
            _ => self.path.clone(),
        }
    }
}

/// What every note is about, for finding related ones.
#[derive(Default)]
pub struct Corpus {
    docs: Vec<Doc>,
    idf: HashMap<String, f32>,
    /// Each word's notes and weight in them.
    postings: HashMap<String, Vec<(u32, f32)>>,
    marks: Vec<Mark>,
}

/// Words in a note's name count this many times more than in its text.
const NAME_WEIGHT: f32 = 3.0;
/// How similar a note must be to show as related (cosine, 0..1).
const MIN_SIMILAR: f32 = 0.1;
/// And a marked passage, to resurface.
const MIN_MARK: f32 = 0.3;

impl Corpus {
    pub fn build(notes: &[Indexed]) -> Corpus {
        let mut counts_per_doc = Vec::with_capacity(notes.len());
        let mut df: HashMap<String, f32> = HashMap::new();
        let mut docs = Vec::with_capacity(notes.len());
        for n in notes {
            let mut counts: HashMap<String, f32> = HashMap::new();
            for (s, _) in words(n.text()) {
                *counts.entry(s).or_default() += 1.0;
            }
            for (s, _) in words(&n.name) {
                *counts.entry(s).or_default() += NAME_WEIGHT;
            }
            for t in counts.keys() {
                *df.entry(t.clone()).or_default() += 1.0;
            }
            counts_per_doc.push(counts);
            docs.push(Doc { path: n.path.clone(), name: n.name.clone(), name_lower: n.name.to_lowercase(), links: links(n.text()) });
        }
        let total = notes.len().max(1) as f32;
        // Words in (nearly) every note tell nothing apart.
        let idf: HashMap<String, f32> = df.into_iter().map(|(t, d)| (t, (1.0 + total / d).ln())).collect();
        let mut postings: HashMap<String, Vec<(u32, f32)>> = HashMap::new();
        for (i, counts) in counts_per_doc.iter().enumerate() {
            for (t, w) in weigh(counts, &idf) {
                postings.entry(t).or_default().push((i as u32, w));
            }
        }
        let mut marks_out = Vec::new();
        for (i, n) in notes.iter().enumerate() {
            for text in marks(n.text()) {
                let mut counts: HashMap<String, f32> = HashMap::new();
                for (s, _) in words(&text) {
                    *counts.entry(s).or_default() += 1.0;
                }
                let weights = weigh(&counts, &idf);
                if !weights.is_empty() {
                    marks_out.push(Mark { doc: i, text, weights });
                }
            }
        }
        Corpus { docs, idf, postings, marks: marks_out }
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Every marked passage in the vault, as (note name, passage), grouped by
    /// note in the order of their names.
    pub fn highlights(&self) -> Vec<(&str, &str)> {
        let mut out: Vec<(&str, &str)> = self.marks.iter().map(|m| (self.docs[m.doc].name.as_str(), m.text.as_str())).collect();
        out.sort_by_key(|m| m.0.to_lowercase());
        out
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Notes related to `text` (the note being written, at vault-relative
    /// `this`), most to `focus` (the paragraph at the caret), best first,
    /// with at most one resurfaced highlight among them.
    pub fn related(&self, text: &str, focus: &str, this: &str, limit: usize) -> Vec<Related> {
        let this_name = this.rsplit('/').next().unwrap_or(this).trim_end_matches(".md").to_lowercase();
        let mut counts: HashMap<String, f32> = HashMap::new();
        let mut shown: HashMap<String, String> = HashMap::new();
        for (s, w) in words(text) {
            shown.entry(s.clone()).or_insert_with(|| w.to_lowercase());
            *counts.entry(s).or_default() += 1.0;
        }
        let mut focus_counts: HashMap<String, f32> = HashMap::new();
        for (s, w) in words(focus) {
            shown.entry(s.clone()).or_insert_with(|| w.to_lowercase());
            *focus_counts.entry(s.clone()).or_default() += 1.0;
            *counts.entry(s).or_default() += 2.0;
        }
        let query = weigh(&counts, &self.idf);

        // Similarity through shared words, keeping what each word added.
        let mut scores: HashMap<u32, (f32, Vec<(f32, String)>)> = HashMap::new();
        for (t, q) in &query {
            for &(doc, w) in self.postings.get(t).map(Vec::as_slice).unwrap_or(&[]) {
                let e = scores.entry(doc).or_default();
                e.0 += q * w;
                e.1.push((q * w, t.clone()));
            }
        }
        let linked = links(text);
        let mut found: Vec<Related> = Vec::new();
        for (i, d) in self.docs.iter().enumerate() {
            if d.path == this || d.name_lower == this_name {
                continue;
            }
            if linked.contains(&d.name_lower) {
                continue;
            }
            let (similar, mut why) = scores.remove(&(i as u32)).unwrap_or_default();
            let (kind, bonus) = if !this_name.is_empty() && d.links.contains(&this_name) {
                (Kind::Backlink, 0.15)
            } else {
                (Kind::Similar, 0.0)
            };
            if kind == Kind::Similar && similar < MIN_SIMILAR {
                continue;
            }
            why.sort_by(|a, b| b.0.total_cmp(&a.0));
            let detail = match kind {
                Kind::Backlink => "links to this note".to_string(),
                _ => why.iter().take(3).map(|(_, t)| shown.get(t).cloned().unwrap_or_else(|| t.clone())).collect::<Vec<_>>().join(" · "),
            };
            found.push(Related { path: d.path.clone(), name: d.name.clone(), kind, detail, score: similar + bonus });
        }
        found.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.name.cmp(&b.name)));

        // One marked passage from another note, if it speaks to this paragraph.
        let focus_query = weigh(&focus_counts, &self.idf);
        let focus_weight: HashMap<&str, f32> = focus_query.iter().map(|(t, w)| (t.as_str(), *w)).collect();
        let best_mark = self
            .marks
            .iter()
            .filter(|m| self.docs[m.doc].path != this)
            .map(|m| (m.weights.iter().map(|(t, w)| w * focus_weight.get(t.as_str()).unwrap_or(&0.0)).sum::<f32>(), m))
            .filter(|(s, _)| *s >= MIN_MARK)
            .max_by(|a, b| a.0.total_cmp(&b.0));
        found.truncate(if best_mark.is_some() { limit.saturating_sub(1) } else { limit });
        if let Some((score, m)) = best_mark {
            let d = &self.docs[m.doc];
            let at = found.iter().position(|r| r.score < score).unwrap_or(found.len());
            found.insert(at, Related { path: d.path.clone(), name: d.name.clone(), kind: Kind::Highlight, detail: m.text.clone(), score });
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Corpus {
        let notes: Vec<Indexed> = [
            ("Rocket engines.md", "Liquid fuel rocket engines burn kerosene and oxygen. Engine thrust and nozzles."),
            ("Launch plan.md", "Plan the launch window. The rocket needs fuel checks before launch. See [[Weekly]]."),
            ("Sourdough.md", "Flour, water and a starter. Bake the bread at high heat. ==Patience makes the crumb=="),
            ("Bread notes.md", "Starter feeding schedule for sourdough bread and ==a long cold proof builds flavor=="),
            ("Weekly.md", "Groceries and errands for the week."),
        ]
        .into_iter()
        .map(|(p, t)| Indexed::new(p.into(), t.into()))
        .collect();
        Corpus::build(&notes)
    }

    #[test]
    fn stems_close_forms_together() {
        assert_eq!(stem("plans"), "plan");
        assert_eq!(stem("planning"), "plan");
        assert_eq!(stem("planned"), "plan");
        assert_eq!(stem("stories"), "story");
        assert_eq!(stem("boxes"), "box");
        assert_eq!(stem("glass"), "glass");
        assert_eq!(stem("focus"), "focus");
        // Short words, numbers, common words and web addresses say nothing.
        let got: Vec<String> = words("The Rockets, and 42 of them: https://x.com").map(|(s, _)| s).collect();
        assert_eq!(got, ["rocket"]);
    }

    #[test]
    fn finds_notes_about_the_same_thing() {
        let c = corpus();
        let r = c.related("Notes on the rocket's fuel and engine thrust.", "rocket engine thrust", "Draft.md", 4);
        let names: Vec<&str> = r.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names.first(), Some(&"Rocket engines"), "{r:?}");
        assert!(names.contains(&"Launch plan"));
        assert!(!names.contains(&"Sourdough") && !names.contains(&"Weekly"));
        assert!(r[0].detail.contains("engine") || r[0].detail.contains("thrust"), "{}", r[0].detail);
    }

    #[test]
    fn finds_backlinks_and_leaves_out_what_is_already_linked() {
        let c = corpus();
        let r = c.related("Errands for the week: see [[Weekly]]", "errands week", "Launch plan.md", 5);
        assert!(r.iter().all(|r| r.name != "Launch plan"), "this note is not related to itself");
        assert!(r.iter().all(|r| r.name != "Weekly"), "already linked: {r:?}");
        let from_weekly = c.related("Groceries", "groceries", "Weekly.md", 5);
        let back = from_weekly.iter().find(|r| r.name == "Launch plan").expect("backlink");
        assert_eq!(back.kind, Kind::Backlink);
    }

    #[test]
    fn resurfaces_a_marked_passage() {
        let c = corpus();
        let r = c.related("Tried a new sourdough loaf today.", "the crumb needs patience", "Today.md", 4);
        let mark = r.iter().find(|r| r.kind == Kind::Highlight).expect("a highlight");
        assert_eq!(mark.detail, "Patience makes the crumb");
        assert_eq!(mark.name, "Sourdough");
        assert!(r.len() <= 4);
    }

    #[test]
    fn gathers_every_highlight_by_note() {
        let c = corpus();
        assert_eq!(c.highlights(), [("Bread notes", "a long cold proof builds flavor"), ("Sourdough", "Patience makes the crumb")]);
    }

    #[test]
    fn gathers_highlights_without_a_corpus_and_writes_their_page() {
        let notes = vec![
            crate::search::Indexed::new("Sourdough.md".into(), "Bake ==Patience makes the crumb== and ==ok==\n".into()),
            crate::search::Indexed::new("a/Bread notes.md".into(), "x ==a long cold proof builds flavor== y ==tw==\n==second one here==".into()),
        ];
        let marks = gather_highlights(&notes);
        let names: Vec<&str> = marks.iter().map(|m| m.0.as_str()).collect();
        assert_eq!(names, ["Bread notes", "Bread notes", "Sourdough"]);
        assert_eq!(marks[0].1, "a long cold proof builds flavor");
        assert_eq!(
            highlights_page(&marks),
            "# Highlights\n3 passages from 2 notes\n### [[Bread notes]]\n> a long cold proof builds flavor\n> second one here\n### [[Sourdough]]\n> Patience makes the crumb\n"
        );
        assert_eq!(highlights_page::<&str, &str>(&[]), "# Highlights\nMark a passage with ==two equals signs on each side== and it gathers here.\n");
        assert!(highlights_page(&[("One", "just this")]).contains("1 passage from 1 note\n"));
    }

    #[test]
    fn says_nothing_for_unrelated_text() {
        let c = corpus();
        assert!(c.related("Quantum chromodynamics lecture", "gluons", "Physics.md", 5).is_empty());
        assert!(Corpus::default().related("rocket", "rocket", "x.md", 5).is_empty());
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    #[test]
    #[ignore = "timing on a real vault: KOLADA_VAULT=<dir>"]
    fn time_on_a_vault() {
        let Some(dir) = std::env::var_os("KOLADA_VAULT") else { return };
        let t = std::time::Instant::now();
        let notes = crate::search::index(std::path::Path::new(&dir));
        let read = t.elapsed();
        let corpus = Corpus::build(&notes);
        let built = t.elapsed() - read;
        let mut worst = std::time::Duration::ZERO;
        let mut found = 0;
        for n in notes.iter().take(200) {
            let q = std::time::Instant::now();
            let focus = n.text().lines().find(|l| l.len() > 20).unwrap_or("");
            found += corpus.related(n.text(), focus, &n.path, 5).len();
            worst = worst.max(q.elapsed());
        }
        println!(
            "{} notes: read {:.0} ms, corpus {:.0} ms, slowest query {:.2} ms, {:.1} related per note",
            notes.len(),
            read.as_secs_f64() * 1e3,
            built.as_secs_f64() * 1e3,
            worst.as_secs_f64() * 1e3,
            found as f64 / notes.len().min(200) as f64
        );
    }
}
