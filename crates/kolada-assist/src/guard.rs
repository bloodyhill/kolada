//! Whether a fix is a fix: the checks an answer passes before it may land.
//!
//! Fixes apply on their own, so the model (small, and apt to rewrite,
//! translate or carry on writing) is trusted with no more than a few small
//! changes to the words: a slip of a letter, an ending, a small word put in
//! or taken out, punctuation. Anything more drops the answer.

use crate::mask::{self, Masked};
use crate::text::{self, Kind, Script};
use kolada_markdown as md;

/// Bytes `start..end` of the paragraph become `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Longer paragraphs are not diffed: the table grows with the square.
const MAX_TOKENS: usize = 600;
/// More changed words in a row than this is a rewrite.
const MAX_RUN: usize = 3;

/// What a model puts before its answer though told not to. One the
/// paragraph itself starts with stays.
const LABELS: [&str; 15] = [
    "here is the corrected text",
    "here's the corrected text",
    "here’s the corrected text",
    "here is the corrected sentence",
    "here's the corrected sentence",
    "corrected text",
    "corrected sentence",
    "corrected version",
    "corrected",
    "correction",
    "fixed text",
    "fixed",
    "text",
    "output",
    "answer",
];

const QUOTES: [(&str, &str); 10] = [("```", "```"), ("`", "`"), ("\"", "\""), ("'", "'"), ("“", "”"), ("‘", "’"), ("„", "“"), ("«", "»"), ("「", "」"), ("『", "』")];

/// Forms of one short word that a fix swaps (agreement, tense) though they
/// share no stem. Words that merely look alike (`big`, `bad`) do not count.
const FORMS: [&str; 31] = [
    "vou vais vai vamos vão ia iam",
    "voy vas va vamos vais van iba iban",
    "am is are was were be been",
    "isn't aren't wasn't weren't isnt arent wasnt werent",
    "has have had",
    "hasn't haven't hadn't hasnt havent hadnt",
    "do does did don't doesn't didn't dont doesnt didnt",
    "this these",
    "that those",
    "a an",
    "suis es est sommes êtes sont étais était étions étiez étaient",
    "ai as a avons avez ont",
    "le la les",
    "un une des",
    "ce cet cette ces",
    "mon ma mes",
    "ton ta tes",
    "son sa ses",
    "soy eres es somos sois son era eras éramos erais eran fui fue fueron",
    "he has ha hemos habéis han",
    "el la los las",
    "un una unos unas",
    "bin bist ist sind seid war warst waren wart",
    "habe hast hat haben habt hatte hatten",
    "der die das den dem des",
    "ein eine einen einem einer eines",
    "sou é és somos são era eram foi foram",
    "o a os as",
    "um uma uns umas",
    "do da dos das",
    "no na nos nas",
];

/// Words that turn a sentence around. A fix never adds or drops one: a
/// small word put in is otherwise cheap.
const NEGATIONS: [&str; 74] = [
    "not", "no", "never", "nothing", "nobody", "none", "nowhere", "neither", "nor", "cannot", "dont", "doesnt", "didnt", "cant", "wont", "isnt", "arent", "wasnt",
    "werent", "havent", "hasnt", "hadnt", "shouldnt", "wouldnt", "couldnt", "mustnt", "neednt", "ne", "pas", "jamais", "rien", "aucun", "aucune", "non", "ni", "nunca",
    "nada", "nadie", "ningún", "ninguno", "ninguna", "jamás", "tampoco", "não", "ninguém", "nenhum", "nenhuma", "nem", "nicht", "kein", "keine", "keinen", "keinem",
    "keiner", "keines", "nie", "niemals", "nichts", "niemand", "nein", "weder", "لا", "لم", "لن", "ليس", "ليست", "لست", "غير", "不", "没", "别", "未", "안", "못",
];

/// Pronouns by person, each with its own forms: a fix may change a form
/// (`I` / `me`), never who it is (`I` / `you`, `他` / `她`). Words that are
/// pronouns only sometimes (`sie`, `nos`, `le`) are left out.
const PERSONS: [&[&str]; 37] = [
    &["i", "me", "my", "mine", "myself"],
    &["you", "your", "yours", "yourself", "yourselves"],
    &["he", "him", "his", "himself"],
    &["she", "her", "hers", "herself"],
    &["we", "us", "our", "ours", "ourselves"],
    &["they", "them", "their", "theirs", "themselves"],
    &["je", "moi"],
    &["tu", "toi"],
    &["il", "lui"],
    &["elle"],
    &["ils", "eux"],
    &["elles"],
    &["yo", "mí", "conmigo"],
    &["tú", "ti", "contigo"],
    &["él"],
    &["ella"],
    &["nosotros", "nosotras"],
    &["ellos"],
    &["ellas"],
    &["eu", "mim", "comigo"],
    &["ele"],
    &["ela"],
    &["eles"],
    &["elas"],
    &["ich", "mich", "mir"],
    &["du", "dich", "dir"],
    &["er", "ihn", "ihm"],
    &["أنا"],
    &["أنت", "أنتِ", "أنتَ"],
    &["هو"],
    &["هي"],
    &["نحن"],
    &["我", "私", "僕", "俺"],
    &["你", "您", "君"],
    &["他", "彼"],
    &["她"],
    &["它"],
];

/// Arabic writes a possessive or object pronoun onto its word (مساعدتك
/// "your help", مساعدتي "my help"): the same word with another of those
/// endings is another person. Ta marbuta (ة) is no such ending, so ه/ة
/// stays a spelling fix.
fn arabic_person(x: &str, y: &str) -> bool {
    const ENDS: [&str; 12] = ["كما", "هما", "كم", "كن", "هم", "هن", "ها", "نا", "ني", "ك", "ه", "ي"];
    let split = |w: &'_ str| ENDS.iter().find_map(|e| w.strip_suffix(e).filter(|s| s.chars().count() >= 2).map(|s| (s.to_string(), *e)));
    matches!((split(x), split(y)), (Some((a, e)), Some((b, f))) if a == b && e != f)
}

/// `w` as its letters read, whatever their case or the Arabic forms people
/// write for one another (alif with or without hamza, ta marbuta and ha,
/// alif maqsura and ya).
fn letters(w: &str) -> String {
    w.to_lowercase()
        .chars()
        .map(|c| match c {
            'أ' | 'إ' | 'آ' => 'ا',
            'ة' => 'ه',
            'ى' => 'ي',
            c => c,
        })
        .collect()
}

/// `w` without Arabic vowel marks (harakat, shadda, sukun) or tatweel.
fn unvowelled(w: &str) -> String {
    w.chars().filter(|c| !matches!(*c as u32, 0x064B..=0x0652 | 0x0670 | 0x0640)).collect()
}

fn person(w: &str) -> Option<usize> {
    let w = w.to_lowercase();
    PERSONS.iter().position(|forms| forms.contains(&w.as_str()))
}

fn questions(s: &str) -> usize {
    s.chars().filter(|c| matches!(c, '?' | '？' | '؟')).count()
}

/// `content`: a paragraph's text after its block mark, as it is in the
/// note. `masked`: `mask(content)`, what the model was shown. `answer`: the
/// model's raw reply.
///
/// None: rejected (not a fix). Some(empty): the paragraph was already
/// right. Some(edits): minimal, non-overlapping, in order, byte offsets
/// into `content`.
pub fn fix(content: &str, masked: &Masked, answer: &str) -> Option<Vec<Edit>> {
    fix_spelled(content, masked, answer, &[])
}

/// As `fix`, knowing which words of `content` the spell checker marked
/// (byte ranges): one of those may become another word altogether (`goed`
/// → `went`), since it was no word to begin with. Not one written with a
/// capital, which may be a name the dictionary lacks.
pub fn fix_spelled(content: &str, masked: &Masked, answer: &str, misspelled: &[(usize, usize)]) -> Option<Vec<Edit>> {
    let shown = masked.text.trim();
    let answer = clean(answer, shown)?;
    if answer.contains(['\n', '\r']) || (answer.is_empty() && !shown.is_empty()) {
        return None;
    }
    let body = masked.unmask(answer)?;
    if let Some(script) = text::dominant(shown)
        && text::dominant(answer) != Some(script)
    {
        return None;
    }
    let (was, now) = (shown.chars().count(), answer.chars().count());
    if was.abs_diff(now) > (was / 5).max(6) {
        return None;
    }
    // The model never sees the paragraph's outer spaces; they stay as typed.
    let from = content.len() - content.trim_start().len();
    let to = from + content[from..].trim_end().len();
    let fixed = format!("{}{body}{}", &content[..from], &content[to..]);
    if fixed == content {
        return Some(Vec::new());
    }
    let again = mask::mask(&fixed);
    if again.pieces != masked.pieces || echoes(content, &fixed) {
        return None;
    }
    let pieces = pieces_at(content, masked)?;
    let (a, b) = (tokenize(content, &pieces), tokenize(&fixed, &pieces_at(&fixed, &again)?));
    if a.len() > MAX_TOKENS || b.len() > MAX_TOKENS {
        return None;
    }
    let hunks = diff(&a, &b);
    let runs: Vec<&[Hunk]> = hunks.chunk_by(|x, y| a[x.a1..y.a0].iter().all(|t| t.kind == Kind::Space)).collect();
    // A word taken out in one place and put back in another: the answer
    // reordered the paragraph, and none of it is a fix.
    let mut seen = Price::default();
    for h in &hunks {
        if let Some(p) = price(std::slice::from_ref(h), &a, &b, misspelled) {
            seen.gone.extend(p.gone.iter().copied());
            seen.came.extend(p.came.iter().copied());
        }
    }
    if seen.gone.iter().any(|x| x.chars().count() > 3 && seen.came.iter().any(|y| typo(x, y))) {
        return None;
    }
    let guarded = guarded(content, &pieces);
    let acronyms = acronyms(content);
    // Each run of changes is judged on its own: a fix goes in, and the rest
    // (a word put for another, another person, a word added or dropped, a
    // number, a protected piece) stays as written, so one bad change does
    // not cost the good ones beside it. First with a run that is no fix
    // read change by change (`god i` → `no, I` keeps the `I`); if what
    // that leaves reads wrong as a whole (half of `doesnt dix` → `does not
    // display` drops the "not"), with such runs left out.
    for apart in [true, false] {
        let mut kept: Vec<Hunk> = Vec::new();
        let mut total = Price::default();
        for run in &runs {
            match price(run, &a, &b, misspelled) {
                Some(p) if p.keeps() => {
                    total.add(p);
                    kept.extend_from_slice(run);
                }
                _ if apart => {
                    for h in *run {
                        if let Some(p) = price(std::slice::from_ref(h), &a, &b, misspelled).filter(Price::keeps) {
                            total.add(p);
                            kept.push(*h);
                        }
                    }
                }
                _ => {}
            }
        }
        if total.tenths > budget(&a) {
            return None;
        }
        let edits: Vec<Edit> = edits(&kept, &a, &b, content.len(), &fixed)
            .into_iter()
            .filter(|e| !guarded.iter().chain(&acronyms).any(|&g| touches(e, g)) && !pieces.iter().any(|&p| strips(content, e, p)))
            .collect();
        // What the paragraph would say, read as a whole: no negation comes
        // or goes, no question becomes a statement, its Markdown stays.
        let landed = applied(content, &edits);
        if negations(content) != negations(&landed) || questions(&landed) < questions(content) || marks(content) != marks(&landed) {
            continue;
        }
        // Only a full stop after a line that had none (a task, a note to
        // self): the line was right as it was.
        let end = content.trim_end().len();
        if let [e] = edits.as_slice()
            && (e.start, e.end) == (end, end)
            && matches!(e.text.as_str(), "." | "。")
        {
            return Some(Vec::new());
        }
        return Some(edits);
    }
    None
}

/// `content` with `edits` (in order, apart) made.
fn applied(content: &str, edits: &[Edit]) -> String {
    let mut out = content.to_string();
    for e in edits.iter().rev() {
        out.replace_range(e.start..e.end, &e.text);
    }
    out
}

/// The answer without what models wrap one in: an end-of-turn mark, a
/// label, quotes the paragraph did not have. None if it opens a quote it
/// never closes.
fn clean<'a>(answer: &'a str, shown: &str) -> Option<&'a str> {
    let answer = answer.trim();
    let labelled = after_label(shown).is_some();
    let unlabel = |s: &'a str| if labelled { s } else { after_label(s).unwrap_or(s) };
    let mut s = unlabel(answer.strip_suffix("<|im_end|>").unwrap_or(answer).trim_end());
    for (open, close) in QUOTES {
        if s.starts_with(open) && !shown.starts_with(open) {
            s = unlabel(s[open.len()..].strip_suffix(close)?.trim());
            break;
        }
    }
    Some(s)
}

/// The text after a leading label such as `Corrected:`.
fn after_label(s: &str) -> Option<&str> {
    LABELS.iter().find_map(|l| {
        let rest = s.get(l.len()..)?.strip_prefix(':')?;
        s[..l.len()].eq_ignore_ascii_case(l).then(|| rest.trim_start())
    })
}

/// A paragraph's Markdown: each span's style, with its marks as written.
fn marks(s: &str) -> Vec<(md::Style, &str, &str)> {
    md::spans(s).into_iter().map(|p| (p.style, &s[p.start..p.text_start], &s[p.text_end..p.end])).collect()
}

/// The paragraph as it was with words added around it: carrying on
/// writing, or a preamble, not a fix.
fn echoes(content: &str, fixed: &str) -> bool {
    let (c, f) = (content.trim().to_lowercase(), fixed.trim().to_lowercase());
    let Some(at) = f.find(&c).filter(|_| f.len() > c.len()) else { return false };
    let cuts: Vec<usize> = text::tokens(&f).iter().map(|t| t.start).chain([f.len()]).collect();
    let edge = |i: usize| cuts.binary_search(&i).is_ok();
    edge(at) && edge(at + c.len()) && text::words(&f[..at]) + text::words(&f[at + c.len()..]) > 0
}

fn negations(s: &str) -> usize {
    let tokens = text::tokens(s);
    let words = tokens.iter().filter(|t| t.kind == Kind::Word).filter(|t| {
        let w = t.text.to_lowercase();
        NEGATIONS.contains(&w.as_str()) || ["n't", "n’t"].iter().any(|e| w.ends_with(e)) || ["n'", "n’", "않", "없", "아니"].iter().any(|p| w.starts_with(p))
    });
    // `doesn t`, `can t`: the apostrophe typed as a space.
    let apart = tokens.windows(3).filter(|w| w[0].text.to_lowercase().ends_with('n') && w[1].kind == Kind::Space && w[2].text.eq_ignore_ascii_case("t")).count();
    words.count() + apart + s.matches("ない").count() + s.matches("ません").count()
}

/// Where each protected piece is in `s`, `m` being its masked form.
fn pieces_at(s: &str, m: &Masked) -> Option<Vec<(usize, usize)>> {
    let mut out = Vec::with_capacity(m.pieces.len());
    let (mut from, mut shift) = (0, 0isize);
    for (i, piece) in m.pieces.iter().enumerate() {
        let marker = mask::marker(i + 1);
        let found = from + m.text[from..].find(&marker)?;
        let start = found.checked_add_signed(shift)?;
        let end = start + piece.len();
        if s.get(start..end)? != piece {
            return None;
        }
        out.push((start, end));
        shift += piece.len() as isize - marker.len() as isize;
        from = found + marker.len();
    }
    Some(out)
}

/// What no edit may touch: the protected pieces and the marks of emphasis.
fn guarded(content: &str, pieces: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out = pieces.to_vec();
    for s in md::spans(content) {
        out.extend(s.marks().into_iter().filter(|(x, y)| x < y));
    }
    out
}

/// Words in capitals (BVE, NASA): an abbreviation, which a fix cannot know
/// and never respells.
fn acronyms(content: &str) -> Vec<(usize, usize)> {
    text::tokens(content)
        .iter()
        .filter(|t| t.kind == Kind::Word && t.text.chars().filter(|c| c.is_uppercase()).count() >= 2 && t.text.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase))
        .map(|t| (t.start, t.start + t.text.len()))
        .collect()
}

/// Whether `e` only takes away what is written right beside a protected
/// piece `x..y` (the particle after a link: `[[計画]]を見て`), which ties the
/// piece to the sentence.
fn strips(content: &str, e: &Edit, (x, y): (usize, usize)) -> bool {
    (e.end == x || e.start == y) && e.text.trim().is_empty() && !content[e.start..e.end].trim().is_empty()
}

/// Whether `e` changes a byte of `x..y`, or puts text inside it.
fn touches(e: &Edit, (x, y): (usize, usize)) -> bool {
    if e.start == e.end { x < e.start && e.start < y } else { e.start < y && x < e.end }
}

/// A token of a paragraph; a protected piece is one token, an `atom`.
#[derive(Clone, Copy, Debug)]
struct Tok<'a> {
    kind: Kind,
    text: &'a str,
    start: usize,
    atom: bool,
}

fn tokenize<'a>(s: &'a str, pieces: &[(usize, usize)]) -> Vec<Tok<'a>> {
    let mut out = Vec::new();
    let mut at = 0;
    let end = (s.len(), s.len());
    for &(x, y) in pieces.iter().chain([&end]) {
        out.extend(text::tokens(&s[at..x]).into_iter().map(|t| Tok { kind: t.kind, text: t.text, start: at + t.start, atom: false }));
        if x < y {
            out.push(Tok { kind: Kind::Word, text: &s[x..y], start: x, atom: true });
        }
        at = y;
    }
    out
}

/// Tokens `a0..a1` of the paragraph became `b0..b1` of the answer.
#[derive(Clone, Copy, Debug)]
struct Hunk {
    a0: usize,
    a1: usize,
    b0: usize,
    b1: usize,
}

/// What changed between `a` and `b`: everything outside a longest common
/// subsequence of their tokens.
fn diff(a: &[Tok], b: &[Tok]) -> Vec<Hunk> {
    let (n, m) = (a.len(), b.len());
    let w = m + 1;
    let same = |i: usize, j: usize| a[i].atom == b[j].atom && a[i].text == b[j].text;
    let mut kept = vec![0u16; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            kept[i * w + j] = if same(i, j) { kept[(i + 1) * w + j + 1] + 1 } else { kept[(i + 1) * w + j].max(kept[i * w + j + 1]) };
        }
    }
    let (mut out, mut open) = (Vec::new(), None);
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && same(i, j) {
            if let Some((a0, b0)) = open.take() {
                out.push(Hunk { a0, a1: i, b0, b1: j });
            }
            i += 1;
            j += 1;
            continue;
        }
        open.get_or_insert((i, j));
        if j == m || (i < n && kept[(i + 1) * w + j] >= kept[i * w + j + 1]) {
            i += 1;
        } else {
            j += 1;
        }
    }
    if let Some((a0, b0)) = open {
        out.push(Hunk { a0, a1: n, b0, b1: m });
    }
    out
}

fn start_of(t: &[Tok], i: usize, len: usize) -> usize {
    t.get(i).map_or(len, |t| t.start)
}

/// The hunks as edits of the paragraph; hunks a space apart make one.
fn edits(hunks: &[Hunk], a: &[Tok], b: &[Tok], len: usize, fixed: &str) -> Vec<Edit> {
    hunks
        .chunk_by(|x, y| y.a0 == x.a1 + 1 && a[x.a1].kind == Kind::Space)
        .map(|run| {
            let (first, last) = (run[0], run[run.len() - 1]);
            let text = &fixed[start_of(b, first.b0, fixed.len())..start_of(b, last.b1, fixed.len())];
            Edit { start: start_of(a, first.a0, len), end: start_of(a, last.a1, len), text: text.to_string() }
        })
        .collect()
}

/// What a run of changes costs, in tenths of a changed word, and what it
/// looks like.
#[derive(Default)]
struct Price<'a> {
    tenths: usize,
    /// Words swapped for unrelated ones: allowed only beside as many fixes.
    swaps: usize,
    /// Changes that look like fixing: a slip, an ending, a small word.
    fixes: usize,
    /// Words taken out, and put in, with nothing in their place.
    gone: Vec<&'a str>,
    came: Vec<&'a str>,
    /// Pronouns made another person's.
    turned: usize,
    /// Longer words put in or taken out alone.
    odd: usize,
}

impl<'a> Price<'a> {
    fn add(&mut self, p: Price<'a>) {
        self.tenths += p.tenths;
        self.swaps += p.swaps;
        self.fixes += p.fixes;
        self.gone.extend(p.gone);
        self.came.extend(p.came);
        self.turned += p.turned;
        self.odd += p.odd;
    }

    fn fixing(&mut self, tenths: usize) {
        self.tenths += tenths;
        self.fixes += 1;
    }

    fn swapping(&mut self) {
        self.tenths += 15;
        self.swaps += 1;
    }

    /// `x` became `y`. `cjk_run`: more than one character of CJK changed
    /// side by side, a word swapped rather than one mistyped. `wrong`: the
    /// spell checker marked `x`.
    fn pair(&mut self, x: &str, y: &str, cjk_run: bool, wrong: bool) {
        if x == y {
            return;
        }
        if x.to_lowercase() == y.to_lowercase() {
            self.tenths += 2;
        } else if unvowelled(x) == unvowelled(y) {
            // Arabic vowel marks put in or taken out: how it is read aloud,
            // not a mistake.
            self.odd += 1;
        } else if let (Some(p), Some(q)) = (person(x), person(y)) {
            if p == q { self.fixing(8) } else { self.turned += 1 }
        } else if arabic_person(x, y) {
            self.turned += 1;
        } else if single_cjk(x) && single_cjk(y) {
            if cjk_run { self.swapping() } else { self.fixing(8) }
        } else if typo(x, y) {
            self.fixing(5);
        } else if stem(x, y) {
            self.fixing(8);
        } else if wrong && x.chars().count().abs_diff(y.chars().count()) <= 2 {
            // Not a word, and about as long as the one put for it: `goed` /
            // `went`, not a guess at another (`dix` / `display`).
            self.fixing(10);
        } else {
            self.swapping();
        }
    }

    /// A word taken out or put in with nothing in its place: an article or
    /// a particle is a fix; a longer word is not.
    fn alone(&mut self, w: &str) {
        if w.chars().count() <= 3 {
            self.fixing(6);
        } else {
            self.tenths += 12;
            self.odd += 1;
        }
    }

    /// Whether this run of changes is a fix to keep: no word put for an
    /// unrelated one, no person changed, no longer word put in or taken out.
    fn keeps(&self) -> bool {
        self.swaps == 0 && self.turned == 0 && self.odd == 0
    }
}

/// A run of hunks with only spaces between them, priced; None if it changes
/// what may not change (a protected piece, a number) or too many words.
fn price<'a>(run: &[Hunk], a: &[Tok<'a>], b: &[Tok<'a>], misspelled: &[(usize, usize)]) -> Option<Price<'a>> {
    let gone: Vec<Tok<'a>> = run.iter().flat_map(|h| a[h.a0..h.a1].iter().copied()).collect();
    let marked = |t: &Tok| misspelled.iter().any(|&(s, e)| s <= t.start && t.start + t.text.len() <= e);
    let wrong: Vec<&'a str> = gone.iter().filter(|t| t.kind == Kind::Word && !t.atom && marked(t) && !t.text.starts_with(char::is_uppercase)).map(|t| t.text).collect();
    let came: Vec<Tok<'a>> = run.iter().flat_map(|h| b[h.b0..h.b1].iter().copied()).collect();
    if gone.iter().chain(&came).any(|t| t.atom || t.text.chars().any(char::is_numeric)) {
        return None;
    }
    let words = |ts: &[Tok<'a>]| -> Vec<&'a str> { ts.iter().filter(|t| t.kind == Kind::Word).map(|t| t.text).collect() };
    let (x, y) = (words(&gone), words(&came));
    if x.len().max(y.len()) > MAX_RUN {
        return None;
    }
    // A word put out for a mark (`friday and` → `Friday;`) drops what was
    // said.
    let marks = |ts: &[Tok<'a>]| ts.iter().filter(|t| t.kind == Kind::Punct).count();
    if x.len() > y.len() && marks(&came) > marks(&gone) {
        return None;
    }
    // The same letters spaced otherwise: `alot` / `a lot`.
    let respaced = x.len() != y.len() && !x.is_empty() && !y.is_empty() && letters(&x.concat()) == letters(&y.concat());
    let mut p = if respaced {
        Price { tenths: 3, fixes: 1, ..Price::default() }
    } else {
        let (left, right) = (pairs(&x, &y, false, &wrong), pairs(&x, &y, true, &wrong));
        if right.tenths < left.tenths { right } else { left }
    };
    p.tenths += gone.len() + came.len() - x.len() - y.len();
    Some(p)
}

/// The words of a run paired in order from its start (or its end), the
/// rest left alone.
fn pairs<'a>(x: &[&'a str], y: &[&'a str], from_end: bool, wrong: &[&str]) -> Price<'a> {
    let n = x.len().min(y.len());
    let (i, j) = if from_end { (x.len() - n, y.len() - n) } else { (0, 0) };
    let both = x[i..i + n].iter().zip(&y[j..j + n]);
    let cjk_run = both.clone().filter(|(u, v)| u != v && single_cjk(u) && single_cjk(v)).count() > 1;
    let mut p = Price::default();
    for (u, v) in both {
        p.pair(u, v, cjk_run, wrong.contains(u));
    }
    for &w in x[..i].iter().chain(&x[i + n..]) {
        p.alone(w);
        p.gone.push(w);
    }
    for &w in y[..j].iter().chain(&y[j + n..]) {
        p.alone(w);
        p.came.push(w);
    }
    p
}

/// What the fixes kept may cost together, in tenths: 0.4 of a word for each
/// of its words (a CJK character counting half), and at least one. Each is
/// a fix on its own, so a line typed in a hurry may need many.
fn budget(a: &[Tok]) -> usize {
    let halves: usize = a.iter().filter(|t| t.kind == Kind::Word).map(|t| if single_cjk(t.text) { 1 } else { 2 }).sum();
    (halves * 2).max(10)
}

fn single_cjk(w: &str) -> bool {
    let mut c = w.chars();
    matches!((c.next(), c.next()), (Some(ch), None) if text::script(ch) == Some(Script::Cjk))
}

/// A slip of a letter or two, case aside: `recieve`, `teh`, `a`/`an`.
fn typo(x: &str, y: &str) -> bool {
    let (x, y) = (x.to_lowercase(), y.to_lowercase());
    let limit = (x.chars().count().max(y.chars().count()) / 3).max(1);
    text::distance(&x, &y, limit) <= limit || transposed(&x, &y)
}

/// Two neighbouring letters typed the wrong way round.
fn transposed(x: &str, y: &str) -> bool {
    let (x, y): (Vec<char>, Vec<char>) = (x.chars().collect(), y.chars().collect());
    let d: Vec<usize> = (0..x.len().min(y.len())).filter(|&i| x[i] != y[i]).collect();
    x.len() == y.len() && d.len() == 2 && d[1] == d[0] + 1 && x[d[0]] == y[d[1]] && x[d[1]] == y[d[0]]
}

/// The same word with another ending (`go`/`goes`, `spielen`/`spielt`):
/// a shared start of at least two letters and 60% of the shorter word. Or
/// another form of a short one (`was`/`were`).
fn stem(x: &str, y: &str) -> bool {
    let (x, y) = (x.to_lowercase(), y.to_lowercase());
    let shared = x.chars().zip(y.chars()).take_while(|(p, q)| p == q).count();
    let (n, m) = (x.chars().count(), y.chars().count());
    let forms = |f: &&str| f.split(' ').any(|w| w == x) && f.split(' ').any(|w| w == y);
    // Most of the shorter word, and half the longer: `dix` and `display`
    // only start alike.
    (shared >= 2 && shared * 5 >= n.min(m) * 3 && shared * 2 >= n.max(m)) || FORMS.iter().any(forms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mask::{mask, marker};

    fn run(content: &str, answer: &str) -> Option<Vec<Edit>> {
        fix(content, &mask(content), answer)
    }

    fn apply(content: &str, edits: &[Edit]) -> String {
        applied(content, edits)
    }

    /// `answer` passes, and its edits (in order, apart) make `content` into
    /// `expect`.
    #[track_caller]
    fn passes(content: &str, answer: &str, expect: &str) -> Vec<Edit> {
        let edits = run(content, answer).unwrap_or_else(|| panic!("refused: {content:?} -> {answer:?}"));
        assert!(!edits.is_empty(), "{content:?} -> {answer:?}");
        assert!(edits.windows(2).all(|w| w[0].end < w[1].start), "{edits:?}");
        assert_eq!(apply(content, &edits), expect);
        edits
    }

    /// What `answer` makes of `content`: the paragraph as the fixes kept
    /// leave it, or None if refused whole.
    fn lands(content: &str, answer: &str) -> Option<String> {
        run(content, answer).map(|edits| apply(content, &edits))
    }

    fn words(s: &str) -> Vec<String> {
        text::tokens(s).iter().filter(|t| t.kind != Kind::Space).map(|t| t.text.to_lowercase()).collect()
    }

    #[test]
    fn real_fixes_pass_in_every_language() {
        for (content, answer) in [
            ("i has went to the store yesterday", "I went to the store yesterday."),
            ("recieve", "receive"),
            ("He go to school every day", "He goes to school every day."),
            ("I have a apple and two banana", "I have an apple and two bananas."),
            ("teh cat sat on the mat", "The cat sat on the mat."),
            ("they was late for the train again", "They were late for the train again."),
            ("there is alot of snow today", "There is a lot of snow today."),
            ("Les filles sont allé au marché", "Les filles sont allées au marché."),
            ("Yo tengo dos gato negro", "Yo tengo dos gatos negros."),
            ("Der Hund spielen im Garten", "Der Hund spielt im Garten."),
            ("ich habe einen hund gesehen", "Ich habe einen Hund gesehen."),
            ("Nós vai para a praia amanhã", "Nós vamos para a praia amanhã."),
            ("ذهبت الى المدرسه", "ذهبت إلى المدرسة"),
            ("我昨天去商店买牛奶", "我昨天去商店买了牛奶。"),
            ("我在也不去那家店了", "我再也不去那家店了。"),
            ("私は学校行きます", "私は学校に行きます。"),
            ("저는 학교 갑니다", "저는 학교에 갑니다."),
        ] {
            passes(content, answer, answer);
        }
    }

    #[test]
    fn edits_are_minimal_and_those_a_space_apart_merge() {
        let content = "i has went to the store yesterday and buyed milk";
        let answer = "I went to the store yesterday and bought milk.";
        let edits = fix_spelled(content, &mask(content), answer, &[(38, 43)]).unwrap();
        let want = [Edit { start: 0, end: 5, text: "I".into() }, Edit { start: 38, end: 43, text: "bought".into() }, Edit { start: 48, end: 48, text: ".".into() }];
        assert_eq!(edits, want);
        assert_eq!(apply(content, &edits), answer);
        let content = "The childs was playing outside in the garden all day";
        let edits = passes(content, "The children were playing outside in the garden all day.", "The children were playing outside in the garden all day.");
        assert_eq!(edits[0], Edit { start: 4, end: 14, text: "children were".into() });
        let edits = passes("ذهبت الى المدرسه", "ذهبت إلى المدرسة", "ذهبت إلى المدرسة");
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn answers_that_rewrite_are_refused_whole() {
        for (content, answer) in [
            ("After lunch we went to the park", "We went to the park after lunch."),
            ("ذهبت إلى المدرسة", "I went to school."),
            ("I went home", "I went home. Then I slept."),
            ("Buy milk", "Buy milk now."),
            ("went to the market", "I went to the market."),
            ("i went home and slept", "I went home.\nAnd slept."),
            ("ok", "Okay, that sounds good to me."),
            ("the report is ready", ""),
            ("i went home", "\"I went home\"."),
            ("大家さんから返事は来ましたか？", "大家さんから返事は来ました。"),
            ("did we ever hear back from the landlord?", "We heard back from the landlord."),
            // A note taken for a question to the model, and answered.
            ("whaere is the ai corrector when you need it", "wha are you asking?"),
        ] {
            assert_eq!(run(content, answer), None, "{content:?} -> {answer:?}");
        }
    }

    #[test]
    fn changes_that_are_no_fix_never_land_and_the_fixes_beside_them_do() {
        for (content, answer, never) in [
            ("I bought fresh bread at the shop this morning", "I bought fresh bread at the store this morning.", "store"),
            ("i bought fresh bread at the shop", "I bought fresh bread at the store.", "store"),
            ("I had a big day", "I had a bad day.", "bad"),
            ("It was a hard week for everyone at home", "It was a happy week for everyone at home.", "happy"),
            ("我昨天去商店买牛奶", "我昨天去超市买牛奶", "超"),
            ("the meeting was good and everyone liked it", "The meeting went well and everybody enjoyed it.", "enjoyed"),
            ("we should probably go now", "We must leave right away.", "must"),
            ("Je suis allé au marché ce matin", "I went to the market this morning.", "market"),
            ("meet me at 5 on friday", "Meet me at 6 on Friday.", "6"),
            ("send the forms on friday and pay the rent", "Send the forms on Friday; pay the rent.", ";"),
            ("tomates y cebolla para la cena de hoy", "Tomates, cebolla para la cena de hoy.", ","),
            ("我对这个结果非常满意。", "你对这个结果非常满意。", "你"),
            ("他明天会来开会。", "她明天会来开会。", "她"),
            ("he said he would call me tomorow", "She said she would call me tomorrow.", "she"),
            ("شكرا على مساعدتك، لقد كانت مفيده جدا.", "شكرا على مساعدتي، لقد كانت مفيدة جدا.", "مساعدتي"),
            ("I buyed milk", "I bought milk.", "bought"),
            ("ذهبت إلى السوق في الصباح.", "ذهبتُ إلى السوقِ في الصباحِ.", "السوقِ"),
            ("this seems like it doesnt work and doesnt dix gibberish text", "This seems like it does not work and does not display gibberish text", "display"),
        ] {
            let landed = lands(content, answer).unwrap_or_else(|| content.to_string());
            assert!(!words(&landed).contains(&never.to_lowercase()), "{content:?} -> {answer:?} landed {landed:?}");
        }
        passes("he said he would call me tomorow", "She said she would call me tomorrow.", "he said he would call me tomorrow.");
        passes("شكرا على مساعدتك، لقد كانت مفيده جدا.", "شكرا على مساعدتي، لقد كانت مفيدة جدا.", "شكرا على مساعدتك، لقد كانت مفيدة جدا.");
        passes("meet me at 5 on friday", "Meet me at 6 on Friday.", "Meet me at 5 on Friday.");
    }

    #[test]
    fn words_run_together_are_split_whatever_their_alif() {
        passes("لا يبدو انهناك اي تصحيح", "لا يبدو أن هناك أي تصحيح", "لا يبدو أن هناك أي تصحيح");
        passes("هذا الكتاب انهناك", "هذا الكتاب أن هناك", "هذا الكتاب أن هناك");
    }

    #[test]
    fn a_bad_change_does_not_cost_the_good_ones() {
        passes("Oh god i wish there was an ai assistant to help me here", "Oh no, I wish there was an AI assistant to help me here", "Oh god I wish there was an AI assistant to help me here");
        passes(
            "this seems like it doesnt work all the time and doesnt dix gibberish text",
            "This seems like it does not work all the time and does not display gibberish text",
            // The second `doesnt` came tangled with the guess at `dix`: it waits.
            "This seems like it does not work all the time and doesnt dix gibberish text",
        );
        passes("whyu doesn t it always work??", "Why doesn't it always work??", "Why doesn't it always work??");
    }

    #[test]
    fn protected_pieces_and_marks_survive() {
        let content = "see [[Plans]] for detials";
        assert_eq!(run(content, "See Plans for details."), None);
        assert_eq!(run(content, "See [[Plans]] for details."), None);
        passes(content, &format!("See {} for details.", marker(1)), "See [[Plans]] for details.");
        passes("check https://x.io/recieve now", &format!("Check {} now.", marker(1)), "Check https://x.io/recieve now.");
        assert_eq!(run("see #todo list", &format!("See {}s list.", marker(1))), None);
        assert_eq!(run("a [[b]] c `d`", &format!("A {} c {}", marker(2), marker(1))), None);
        // Emphasis dropped, swapped or moved off a word: whatever else lands,
        // the marks stay as written.
        for (content, answer) in [
            ("this is **very** importent", "This is very important."),
            ("this is **very** importent", "This is __very__ important."),
            ("this is **very importent** for the whole team at work today and tomorrow", "This is **very** important for the whole team at work today and tomorrow."),
        ] {
            if let Some(landed) = lands(content, answer) {
                assert_eq!(marks(&landed).iter().map(|m| (m.1, m.2)).collect::<Vec<_>>(), marks(content).iter().map(|m| (m.1, m.2)).collect::<Vec<_>>(), "{landed:?}");
            }
        }
        passes("this is **realy** importent for the whole team", "This is **really** important for the whole team.", "This is **really** important for the whole team.");
        // What ties a piece to its sentence stays; a space too many beside it goes.
        assert_eq!(lands("詳しくは[[旅行の計画]]を見てください。", &format!("詳しくは{}見てください。", marker(1))).as_deref(), Some("詳しくは[[旅行の計画]]を見てください。"));
        passes("see  [[Plans]] for detials", &format!("See {} for details.", marker(1)), "See [[Plans]] for details.");
    }

    #[test]
    fn abbreviations_are_never_respelled() {
        assert_eq!(lands("BVE Bundesgesetz über die verdeckte Ermittlung", "BGE Bundesgesetz über die verdeckte Ermittlung").as_deref(), Some("BVE Bundesgesetz über die verdeckte Ermittlung"));
        passes("the NASA reprot is out", "The NASA report is out.", "The NASA report is out.");
    }

    #[test]
    fn answers_are_cleaned_first() {
        for answer in ["Corrected: I went home.", "corrected text: I went home.", "\"I went home.\"", "“I went home.”", "`I went home.`", "I went home.<|im_end|>", "  Fixed: \"I went home.\"  "] {
            passes("i went home", answer, "I went home.");
        }
        passes("Text: helo world", "Text: hello world.", "Text: hello world.");
        passes("\"hello\" she said", "\"Hello\" she said.", "\"Hello\" she said.");
    }

    #[test]
    fn a_pronouns_form_may_change_its_person_not() {
        passes("between you and I it went fine", "Between you and me it went fine.", "Between you and me it went fine.");
        passes("Kannst du mich morgen beim Umzug helfen", "Kannst du mir morgen beim Umzug helfen?", "Kannst du mir morgen beim Umzug helfen?");
    }

    #[test]
    fn a_word_marked_misspelled_may_become_another() {
        let content = "Yesterday I goed to the library";
        let answer = "Yesterday I went to the library.";
        assert_eq!(run(content, answer), Some(vec![]), "unmarked, `went` is another word");
        let at = content.find("goed").unwrap();
        let edits = fix_spelled(content, &mask(content), answer, &[(at, at + 4)]).unwrap();
        assert_eq!(apply(content, &edits), answer);
        // A name the dictionary lacks is not swapped for a word.
        let content = "Ask Tavrek about the new plan";
        assert_eq!(fix_spelled(content, &mask(content), "Ask Tofu about the new plan.", &[(4, 10)]), Some(vec![]));
        // Nor is a word the checker had no quarrel with.
        let content = "Yesterday I goed to the shop";
        let edits = fix_spelled(content, &mask(content), "Yesterday I went to the store.", &[(12, 16)]).unwrap();
        assert_eq!(apply(content, &edits), "Yesterday I went to the shop");
        // Nor a guess at another word, far from what was written.
        let content = "please dix the sink today";
        let edits = fix_spelled(content, &mask(content), "Please display the sink today.", &[(7, 10)]).unwrap();
        assert_eq!(apply(content, &edits), "Please dix the sink today.");
    }

    #[test]
    fn a_full_stop_alone_is_no_fix() {
        assert_eq!(run("Call the dentist about moving Thursday's appointment", "Call the dentist about moving Thursday's appointment."), Some(vec![]));
        assert_eq!(run("下班以后去超市", "下班以后去超市。"), Some(vec![]));
        passes("call the dentist about moving thursday's appointment", "Call the dentist about moving Thursday's appointment.", "Call the dentist about moving Thursday's appointment.");
        passes("did we hear back from the landlord", "Did we hear back from the landlord?", "Did we hear back from the landlord?");
    }

    #[test]
    fn right_text_gives_no_edits() {
        assert_eq!(run("I went to the store.", "I went to the store."), Some(vec![]));
        assert_eq!(run("I went to the store.", "\"I went to the store.\""), Some(vec![]));
        assert_eq!(run("ذهبت إلى المدرسة", "ذهبت إلى المدرسة"), Some(vec![]));
        let content = "See [[Plans]] and `code` today.";
        assert_eq!(run(content, &mask(content).text), Some(vec![]));
        assert_eq!(run("ends in a space ", "ends in a space"), Some(vec![]));
    }

    #[test]
    fn the_space_around_a_paragraph_stays() {
        passes("i went home ", "I went home.", "I went home. ");
        passes("  i went home", "I went home.", "  I went home.");
    }

    #[test]
    fn a_negation_never_comes_or_goes() {
        for (content, answer) in [
            ("I think we should go to the beach this weekend", "I think we should not go to the beach this weekend."),
            ("I do like the new office a lot", "I don't like the new office a lot."),
            ("we can finish the report by tomorrow", "We can't finish the report by tomorrow."),
            ("Je sais pourquoi il est parti hier soir", "Je ne sais pas pourquoi il est parti hier soir."),
            ("Ich habe heute Zeit für das Treffen am Abend", "Ich habe heute keine Zeit für das Treffen am Abend."),
            ("我明天去公司开会讨论新的计划", "我明天不去公司开会讨论新的计划"),
        ] {
            if let Some(landed) = lands(content, answer) {
                assert_eq!(negations(&landed), negations(content), "{content:?} -> {answer:?} landed {landed:?}");
            }
        }
        passes("he don't like the new office at all", "He doesn't like the new office at all.", "He doesn't like the new office at all.");
        passes("I dont like the new office very much", "I don't like the new office very much.", "I don't like the new office very much.");
        passes("the shops aren't open on sunday mornings here", "The shops aren't open on Sunday mornings here.", "The shops aren't open on Sunday mornings here.");
    }

    #[test]
    fn long_paragraphs_are_not_diffed() {
        let long = format!("{}recieve", "word ".repeat(350));
        assert_eq!(run(&long, &long.replace("recieve", "receive")), None);
        let fine = format!("{}recieve", "word ".repeat(250));
        passes(&fine, &fine.replace("recieve", "receive"), &fine.replace("recieve", "receive"));
    }
}
