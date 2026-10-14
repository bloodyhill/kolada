//! Generates synthetic Obsidian-style vaults for startup-scaling benchmarks.
//!
//! Usage: vaultgen <note-count> <out-dir> [--seed <n>]
//!
//! The note size distribution mirrors a real vault (about 1 KB on average, a
//! long tail up to about 100 KB) and uses the Obsidian syntax Kolada must
//! handle: wikilinks with headings/aliases, tags, frontmatter, tasks,
//! callouts, code, tables and Arabic paragraphs.
//!
//! Output is deterministic for a given seed. The tool only writes into a
//! directory that is missing, empty, or was created by vaultgen (it drops a
//! `.vaultgen` marker), so it can never overwrite a real vault.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const MARKER: &str = ".vaultgen";

const WORDS: &[&str] = &[
    "system", "note", "idea", "draft", "project", "meeting", "review", "design", "market",
    "launch", "budget", "client", "research", "summary", "question", "answer", "problem",
    "method", "result", "insight", "process", "team", "goal", "plan", "task", "update",
    "detail", "feature", "release", "sprint", "metric", "customer", "product", "vision",
    "memo", "journal", "reading", "book", "chapter", "quote", "principle", "habit", "health",
    "travel", "recipe", "finance", "invoice", "contract", "hiring", "interview", "feedback",
    "strategy", "roadmap", "priority", "risk", "decision", "outcome", "lesson", "archive",
    "inbox", "weekly", "daily", "monthly", "quarter", "report", "analysis", "data", "model",
    "server", "client", "network", "storage", "cache", "index", "search", "editor", "layout",
    "render", "frame", "window", "thread", "queue", "latency", "memory", "startup", "sync",
];

const ARABIC: &[&str] = &[
    "ملاحظة", "فكرة", "مشروع", "اجتماع", "مراجعة", "تصميم", "سوق", "إطلاق", "ميزانية",
    "عميل", "بحث", "ملخص", "سؤال", "جواب", "مشكلة", "طريقة", "نتيجة", "فريق", "هدف",
    "خطة", "مهمة", "تحديث", "تفاصيل", "ميزة", "إصدار", "منتج", "رؤية", "كتاب", "قراءة",
    "صحة", "سفر", "تقرير", "تحليل", "بيانات", "قرار", "درس", "أسبوع", "يوم", "شهر",
];

const TAGS: &[&str] = &[
    "work", "personal", "idea", "todo", "reading", "project/alpha", "project/beta",
    "meeting", "journal", "reference", "archive", "health", "finance", "writing",
];

const CALLOUTS: &[&str] = &["note", "tip", "warning", "info", "todo", "quote"];

/// SplitMix64: tiny, fast and deterministic.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }

    fn chance(&mut self, p: f64) -> bool {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) < p
    }

    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

struct Note {
    /// Path relative to the vault root, with forward slashes, without `.md`.
    rel: String,
    title: String,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("vaultgen: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: vaultgen <note-count> <out-dir> [--seed <n>]";
    let count: usize = args
        .next()
        .ok_or(usage)?
        .replace('_', "")
        .parse()
        .map_err(|_| usage)?;
    let out = PathBuf::from(args.next().ok_or(usage)?);
    // A fixed seed, so generated vaults are the same every time.
    let mut seed = 0x006E_6175_7479_u64;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).ok_or(usage)?,
            _ => return Err(usage.into()),
        }
    }
    if count == 0 {
        return Err("note-count must be at least 1".into());
    }

    check_out_dir(&out)?;
    fs::create_dir_all(&out).map_err(|e| format!("creating {}: {e}", out.display()))?;
    fs::write(out.join(MARKER), format!("count={count}\nseed={seed}\n"))
        .map_err(|e| format!("writing marker: {e}"))?;

    let mut rng = Rng(seed);
    let folders = make_folders(&mut rng, count);
    let notes = make_notes(&mut rng, count, &folders);

    for folder in &folders {
        fs::create_dir_all(out.join(folder)).map_err(|e| format!("creating folder: {e}"))?;
    }

    let mut total_bytes = 0usize;
    let mut body = String::new();
    for (i, note) in notes.iter().enumerate() {
        body.clear();
        write_note(&mut rng, &notes, note, &mut body);
        let path = out.join(format!("{}.md", note.rel));
        fs::write(&path, &body).map_err(|e| format!("writing {}: {e}", path.display()))?;
        total_bytes += body.len();
        if (i + 1) % 10_000 == 0 {
            eprintln!("  {} / {count} notes", i + 1);
        }
    }

    println!(
        "wrote {count} notes in {} folders, {:.1} MB, to {}",
        folders.len(),
        total_bytes as f64 / (1024.0 * 1024.0),
        out.display()
    );
    Ok(())
}

/// Refuses any directory that holds files but was not created by vaultgen.
fn check_out_dir(out: &Path) -> Result<(), String> {
    let Ok(mut entries) = fs::read_dir(out) else {
        return Ok(()); // Missing: we create it.
    };
    if entries.next().is_none() || out.join(MARKER).is_file() {
        return Ok(());
    }
    Err(format!(
        "{} is not empty and was not created by vaultgen; refusing to write into it",
        out.display()
    ))
}

/// About sqrt(n)/2 folders, up to three levels deep, like a real vault.
fn make_folders(rng: &mut Rng, count: usize) -> Vec<String> {
    let n = ((count as f64).sqrt() / 2.0).ceil() as usize;
    let mut folders: Vec<String> = Vec::with_capacity(n);
    for i in 0..n {
        let name = format!("{} {}", capitalize(rng.pick(WORDS)), i + 1);
        let parent_depth_ok: Vec<usize> = (0..folders.len())
            .filter(|&j| folders[j].matches('/').count() < 2)
            .collect();
        if !parent_depth_ok.is_empty() && rng.chance(0.4) {
            let parent = &folders[parent_depth_ok[rng.below(parent_depth_ok.len())]];
            let path = format!("{parent}/{name}");
            folders.push(path);
        } else {
            folders.push(name);
        }
    }
    folders
}

fn make_notes(rng: &mut Rng, count: usize, folders: &[String]) -> Vec<Note> {
    (0..count)
        .map(|i| {
            let title = format!(
                "{} {} {}",
                capitalize(rng.pick(WORDS)),
                rng.pick(WORDS),
                i + 1
            );
            // A fifth of notes live at the root, the rest in folders.
            let rel = if folders.is_empty() || rng.chance(0.2) {
                title.clone()
            } else {
                format!("{}/{title}", folders[rng.below(folders.len())])
            };
            Note { rel, title }
        })
        .collect()
}

fn write_note(rng: &mut Rng, notes: &[Note], note: &Note, out: &mut String) {
    if rng.chance(0.3) {
        out.push_str("---\n");
        let _ = writeln!(out, "tags: [{}, {}]", rng.pick(TAGS), rng.pick(TAGS));
        if rng.chance(0.3) {
            let _ = writeln!(out, "aliases: [{} {}]", rng.pick(WORDS), rng.pick(WORDS));
        }
        let _ = writeln!(
            out,
            "created: 2025-{:02}-{:02}",
            rng.range(1, 12),
            rng.range(1, 28)
        );
        out.push_str("---\n\n");
    }
    if rng.chance(0.6) {
        let _ = writeln!(out, "# {}\n", note.title);
    }

    // Geometric block count: most notes are short, a few are long, and a
    // rare handful are very large.
    let mut blocks = 1;
    while blocks < 60 && rng.chance(0.65) {
        blocks += 1;
    }
    if rng.chance(0.005) {
        blocks = rng.range(150, 400);
    }

    for _ in 0..blocks {
        match rng.below(100) {
            0..=54 => paragraph(rng, notes, out),
            55..=64 => {
                let _ = writeln!(
                    out,
                    "{} {}\n",
                    "#".repeat(rng.range(2, 4)),
                    capitalize(&sentence(rng, 2, 5))
                );
            }
            65..=74 => list(rng, notes, out),
            75..=81 => tasks(rng, out),
            82..=86 => arabic_paragraph(rng, out),
            87..=90 => code_block(rng, out),
            91..=94 => callout(rng, out),
            95..=97 => table(rng, out),
            _ => out.push_str("---\n\n"),
        }
    }
}

fn paragraph(rng: &mut Rng, notes: &[Note], out: &mut String) {
    for _ in 0..rng.range(1, 5) {
        let mut s = capitalize(&sentence(rng, 5, 18));
        match rng.below(12) {
            0..=3 => s.push_str(&format!(" See {}.", wikilink(rng, notes))),
            4 => s.push_str(&format!(" #{}", rng.pick(TAGS))),
            5 => s.push_str(&format!(" **{}**.", rng.pick(WORDS))),
            6 => s.push_str(&format!(" *{}*.", rng.pick(WORDS))),
            7 => s.push_str(&format!(" `{}()`.", rng.pick(WORDS))),
            8 => s.push_str(&format!(" =={}==.", rng.pick(WORDS))),
            9 => s.push_str(&format!(" [{}](https://example.com/{}).", rng.pick(WORDS), rng.pick(WORDS))),
            10 => s.push_str(&format!(" %%{}%%", sentence(rng, 2, 4))),
            _ => s.push('.'),
        }
        out.push_str(&s);
        out.push(' ');
    }
    out.push_str("\n\n");
}

fn wikilink(rng: &mut Rng, notes: &[Note]) -> String {
    let target = &notes[rng.below(notes.len())].title;
    match rng.below(20) {
        0..=1 => format!("[[{target}|{}]]", rng.pick(WORDS)),
        2 => format!("[[{target}#{}]]", capitalize(rng.pick(WORDS))),
        3 => format!("[[{target}#^{:06x}]]", rng.next() & 0xFF_FFFF),
        4 => format!("[[Missing {} {}]]", rng.pick(WORDS), rng.below(1000)),
        _ => format!("[[{target}]]"),
    }
}

fn list(rng: &mut Rng, notes: &[Note], out: &mut String) {
    let ordered = rng.chance(0.3);
    for i in 0..rng.range(2, 8) {
        let indent = if i > 0 && rng.chance(0.25) { "  " } else { "" };
        let bullet = if ordered { format!("{}.", i + 1) } else { "-".into() };
        let mut item = capitalize(&sentence(rng, 3, 10));
        if rng.chance(0.15) {
            item.push_str(&format!(" {}", wikilink(rng, notes)));
        }
        let _ = writeln!(out, "{indent}{bullet} {item}");
    }
    out.push('\n');
}

fn tasks(rng: &mut Rng, out: &mut String) {
    for _ in 0..rng.range(1, 6) {
        let done = if rng.chance(0.4) { 'x' } else { ' ' };
        let _ = writeln!(out, "- [{done}] {}", capitalize(&sentence(rng, 3, 8)));
    }
    out.push('\n');
}

fn arabic_paragraph(rng: &mut Rng, out: &mut String) {
    let n = rng.range(6, 24);
    let words: Vec<&str> = (0..n).map(|_| rng.pick(ARABIC)).collect();
    out.push_str(&words.join(" "));
    // Mixed-direction text is the hard case for caret movement.
    if rng.chance(0.5) {
        let _ = write!(out, " {} {}", rng.pick(WORDS), rng.range(1, 2030));
    }
    out.push_str(".\n\n");
}

fn code_block(rng: &mut Rng, out: &mut String) {
    let lang = ["rust", "ts", "python", "sql", ""][rng.below(5)];
    let _ = writeln!(out, "```{lang}");
    for _ in 0..rng.range(2, 12) {
        let _ = writeln!(
            out,
            "let {} = {}({});",
            rng.pick(WORDS),
            rng.pick(WORDS),
            rng.below(100)
        );
    }
    out.push_str("```\n\n");
}

fn callout(rng: &mut Rng, out: &mut String) {
    let _ = writeln!(
        out,
        "> [!{}] {}",
        rng.pick(CALLOUTS),
        capitalize(&sentence(rng, 2, 4))
    );
    for _ in 0..rng.range(1, 3) {
        let _ = writeln!(out, "> {}.", capitalize(&sentence(rng, 5, 14)));
    }
    out.push('\n');
}

fn table(rng: &mut Rng, out: &mut String) {
    let cols = rng.range(2, 4);
    let header: Vec<String> = (0..cols).map(|_| capitalize(rng.pick(WORDS))).collect();
    let _ = writeln!(out, "| {} |", header.join(" | "));
    let _ = writeln!(out, "|{}", "---|".repeat(cols));
    for _ in 0..rng.range(1, 5) {
        let row: Vec<String> = (0..cols).map(|_| rng.pick(WORDS).to_string()).collect();
        let _ = writeln!(out, "| {} |", row.join(" | "));
    }
    out.push('\n');
}

fn sentence(rng: &mut Rng, min: usize, max: usize) -> String {
    let n = rng.range(min, max);
    let words: Vec<&str> = (0..n).map(|_| rng.pick(WORDS)).collect();
    words.join(" ")
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
