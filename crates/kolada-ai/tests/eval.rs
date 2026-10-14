//! The writing model against fixtures, through the real helper process:
//! mask, prompt, answer, then the guards the app applies.
//!
//! `cargo test -p kolada-ai --release --target-dir target\a-ai --test eval -- --ignored --nocapture`
//!
//! EVAL_ONLY=fix|shape|gist picks one part, EVAL_LANG=xx one language,
//! EVAL_FIX=<path> fixes other lines than the fixtures (lang, input,
//! expected, and the paragraph before if it has one), EVAL_SHOW=1 prints
//! every case, EVAL_CONTEXT=1 hands each fix the line before it (or its own
//! paragraph before) as context; EVAL_THREADS, EVAL_BATCH_THREADS, EVAL_CTX,
//! EVAL_NEAR and EVAL_FAR are handed to the helper; KOLADA_MODEL overrides
//! the model. Notes for the gists are read from vaults/my-notes-copy and
//! only their sizes are printed.

use std::collections::BTreeMap;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use kolada_assist::mask::{self, Masked};
use kolada_assist::wire::{self, Kind, Reply, Request, Status};
use kolada_assist::{gist as gists, guard, shape as shapes, text};

fn model() -> PathBuf {
    if let Some(m) = std::env::var_os("KOLADA_MODEL") {
        return m.into();
    }
    PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA")).join(r"Kolada\models\notsy-writer-1.2b-Q4_K_M.gguf")
}

fn var(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Helper {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    log: PathBuf,
    next: u64,
    started: Instant,
}

impl Helper {
    fn start() -> Helper {
        let log = Path::new(env!("CARGO_TARGET_TMPDIR")).join("eval-ai.log");
        let _ = std::fs::remove_file(&log);
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_kolada-ai"));
        cmd.arg("--model").arg(model());
        for (flag, k) in [("--threads", "EVAL_THREADS"), ("--batch-threads", "EVAL_BATCH_THREADS"), ("--ctx", "EVAL_CTX"), ("--near", "EVAL_NEAR"), ("--far", "EVAL_FAR")] {
            if let Some(v) = var(k) {
                cmd.arg(flag).arg(v);
            }
        }
        let started = Instant::now();
        let mut child = cmd.env("KOLADA_AI_LOG", &log).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().expect("spawn kolada-ai");
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Helper { child, stdin, stdout, log, next: 1, started }
    }

    fn ask(&mut self, kind: Kind, context: &str, text: &str) -> (Reply, Duration) {
        let id = self.next;
        self.next += 1;
        let r = Request { kind, id, note: 1, slot: id, context: context.into(), text: text.into() };
        let t = Instant::now();
        wire::write_frame(&mut self.stdin, &r.frame()).expect("write");
        let body = wire::read_frame(&mut self.stdout).expect("read").expect("the helper closed its output");
        let reply = Reply::parse(&body).expect("a reply");
        assert_eq!(reply.id, id);
        (reply, t.elapsed())
    }

    /// Peak working set and private bytes, in MB.
    #[cfg(windows)]
    fn memory(&self) -> (f64, f64) {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
        let mut m = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
        let h = HANDLE(self.child.as_raw_handle());
        unsafe { GetProcessMemoryInfo(h, (&raw mut m).cast::<PROCESS_MEMORY_COUNTERS>(), m.cb) }.expect("memory info");
        (m.PeakWorkingSetSize as f64 / 1048576.0, m.PrivateUsage as f64 / 1048576.0)
    }

    #[cfg(not(windows))]
    fn memory(&self) -> (f64, f64) {
        (0.0, 0.0)
    }

    fn close(mut self) -> String {
        drop(self.stdin);
        let status = self.child.wait().expect("wait");
        assert!(status.success(), "the helper exited with {status}");
        // llama.cpp's own lines may cut a character in two.
        String::from_utf8_lossy(&std::fs::read(&self.log).unwrap_or_default()).into_owned()
    }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn lines(name: &str) -> Vec<Vec<String>> {
    // A path of its own (EVAL_FIX), or one of the fixtures.
    let path = if Path::new(name).is_absolute() { PathBuf::from(name) } else { root().join("crates/kolada-assist/testdata").join(name) };
    let s = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let lang = var("EVAL_LANG");
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect::<Vec<_>>())
        .filter(|f| lang.as_ref().is_none_or(|g| &f[0] == g))
        .collect()
}

#[derive(Default)]
struct Tally {
    /// Lines already right: changed after the guard, changed as the model
    /// answered, and of those the guard held back.
    correct: usize,
    false_edits: usize,
    raw_false: usize,
    held: usize,
    /// Lines with a mistake.
    wrong: usize,
    exact: usize,
    partly: usize,
    missed: usize,
    worse: usize,
    dropped: usize,
    raw_exact: usize,
    /// Answered exactly right, dropped by the guard all the same.
    lost: usize,
    /// Word by word: the changes a line needed, of those the ones made as
    /// expected, and every change made (on right lines too).
    needed: usize,
    fixed: usize,
    made: usize,
    ms: Vec<f64>,
}

impl Tally {
    fn add(&mut self, t: &Tally) {
        self.correct += t.correct;
        self.false_edits += t.false_edits;
        self.raw_false += t.raw_false;
        self.held += t.held;
        self.wrong += t.wrong;
        self.exact += t.exact;
        self.partly += t.partly;
        self.missed += t.missed;
        self.worse += t.worse;
        self.dropped += t.dropped;
        self.raw_exact += t.raw_exact;
        self.lost += t.lost;
        self.needed += t.needed;
        self.fixed += t.fixed;
        self.made += t.made;
        self.ms.extend(&t.ms);
    }

    fn row(&self, name: &str) {
        let mut ms = self.ms.clone();
        println!(
            "{name:4}  {:7}  {:10}  {:9}  {:4} | {:5}  {:5}  {:6}  {:6}  {:5}  {:7}  {:9}  {:4} | {:7.0} {:7.0}",
            self.correct,
            self.false_edits,
            self.raw_false,
            self.held,
            self.wrong,
            self.exact,
            self.partly,
            self.missed,
            self.worse,
            self.dropped,
            self.raw_exact,
            self.lost,
            pct(&mut ms, 0.5),
            pct(&mut ms, 0.95)
        );
    }
}

/// What the note holds after the app applies `answer` to `input`: None when
/// the guard drops it.
fn applied(input: &str, masked: &Masked, answer: &str) -> Option<String> {
    let edits = guard::fix(input, masked, answer)?;
    let mut s = input.to_string();
    for e in edits.iter().rev() {
        s.replace_range(e.start..e.end, &e.text);
    }
    Some(s)
}

/// A line's words and marks, spaces left out.
fn units(s: &str) -> Vec<&str> {
    text::tokens(s).iter().filter(|t| t.kind != text::Kind::Space).map(|t| t.text).collect()
}

/// The changes that make `a` into `b`, word by word: which words go (a
/// range of `a`) and what is put for them.
fn changes(a: &str, b: &str) -> Vec<(usize, usize, String)> {
    let (x, y) = (units(a), units(b));
    let (n, m) = (x.len(), y.len());
    let mut lcs = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if x[i] == y[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    let mut open: Option<(usize, Vec<&str>)> = None;
    while i < n || j < m {
        if i < n && j < m && x[i] == y[j] {
            if let Some((from, put)) = open.take() {
                out.push((from, i, put.join(" ")));
            }
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            open.get_or_insert((i, Vec::new())).1.push(y[j]);
            j += 1;
        } else {
            open.get_or_insert((i, Vec::new()));
            i += 1;
        }
    }
    if let Some((from, put)) = open {
        out.push((from, n, put.join(" ")));
    }
    out
}

#[test]
fn changes_are_counted_word_by_word() {
    assert_eq!(changes("isthis real life", "Is this real life"), vec![(0, 1, "Is this".to_string())]);
    assert_eq!(changes("whaere is it nut why", "Where is it but why"), vec![(0, 1, "Where".to_string()), (3, 4, "but".to_string())]);
    assert_eq!(changes("the the cat", "the cat"), vec![(1, 2, String::new())]);
    assert!(changes("Right as it is.", "Right as it is.").is_empty());
}

/// What a fix is sent as context, as the app sends it: nothing, the
/// paragraph before, or the title and the paragraph before.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Context {
    None,
    Para,
    Title,
}

fn title(lang: &str) -> &'static str {
    match lang {
        "ar" => "ملاحظات الأسبوع",
        "zh" => "本周笔记",
        "ja" => "今週のメモ",
        "ko" => "이번 주 메모",
        "fr" => "Notes de la semaine",
        "de" => "Wochennotizen",
        "pt" => "Notas da semana",
        "es" => "Notas de la semana",
        _ => "Weekly notes",
    }
}

/// Asks for one fix: (lang, context, masked text) to the answer.
type Ask<'a> = dyn FnMut(&str, &str, &str) -> (Reply, Duration) + 'a;

fn fix(ask: &mut Ask, show: bool, context: Context) -> (Vec<f64>, BTreeMap<String, Tally>) {
    let mut by: BTreeMap<String, Tally> = BTreeMap::new();
    let mut short = Vec::new();
    // EVAL_FIX: another set of lines (lang, input, expected) in place of the
    // fixtures, the held-out pairs of tools/train for one.
    let cases: Vec<Vec<String>> = match var("EVAL_FIX") {
        Some(file) => lines(&file),
        None => lines("fix.tsv").into_iter().chain(lines("keep.tsv").into_iter().map(|f| vec![f[0].clone(), f[1].clone(), f[1].clone()])).collect(),
    };
    let mut before: BTreeMap<String, String> = BTreeMap::new();
    for f in cases {
        let (lang, input, expected) = (&f[0], &f[1], &f[2]);
        let masked = mask::mask(input);
        // A line's own paragraph before (a fourth column), or the line before it.
        let own = f.get(3).filter(|c| !c.is_empty());
        let ctx = match (context, own.or(before.get(lang))) {
            (Context::Para, Some(p)) => p.clone(),
            // The app sends the title only with a paragraph before.
            (Context::Title, Some(p)) => format!("{} \u{2014} {p}", title(lang)),
            _ => String::new(),
        };
        before.insert(lang.clone(), expected.clone());
        let (reply, took) = ask(lang, &ctx, &masked.text);
        let ms = took.as_secs_f64() * 1000.0;
        let t = by.entry(lang.clone()).or_default();
        t.ms.push(ms);
        if text::words(input) <= 40 {
            short.push(ms);
        }
        let done = reply.status == Status::Done;
        let answer = if done { reply.text.clone() } else { String::new() };
        let raw = if done { masked.unmask(&answer) } else { None };
        let after = if done { applied(input, &masked, &answer) } else { None };
        let now = after.clone().unwrap_or_else(|| input.clone());
        let (need, did) = (changes(input, expected), changes(input, &now));
        t.needed += need.len();
        t.made += did.len();
        t.fixed += need.iter().filter(|c| did.contains(c)).count();
        let mark = if input == expected {
            t.correct += 1;
            let raw_changed = raw.as_deref().is_some_and(|r| r != input);
            t.raw_false += usize::from(raw_changed);
            if now != *input {
                t.false_edits += 1;
                "FALSE"
            } else if raw_changed {
                t.held += 1;
                "held"
            } else {
                "ok"
            }
        } else {
            t.wrong += 1;
            let raw_right = raw.as_deref() == Some(expected.as_str());
            t.raw_exact += usize::from(raw_right);
            if after.is_none() {
                t.dropped += 1;
                t.lost += usize::from(raw_right);
                if raw_right { "LOST" } else { "dropped" }
            } else if now == *expected {
                t.exact += 1;
                "EXACT"
            } else if now == *input {
                t.missed += 1;
                "missed"
            } else if text::distance(&now, expected, 99) < text::distance(input, expected, 99) {
                t.partly += 1;
                "partly"
            } else {
                t.worse += 1;
                "worse"
            }
        };
        if show {
            println!("[{lang}] {mark:7} {ms:6.0} ms  {input}\n                   -> {}", answer.replace('\n', "\\n"));
            if now != *input && now != answer {
                println!("                   => {now}");
            }
        }
    }
    (short, by)
}

fn fix_table(short: &mut [f64], by: &BTreeMap<String, Tally>, context: Context) -> f64 {
    println!("\nFix with context {context:?}, per language. Right lines: changed after the guard (false-edit), as answered (raw-false), held back by the guard (held).");
    println!("Wrong lines: after the guard; raw-exact = answered exactly right, lost = of those, dropped by the guard.");
    println!("lang  correct  false-edit  raw-false  held | wrong  exact  partly  missed  worse  dropped  raw-exact  lost |  p50 ms  p95 ms");
    let mut all = Tally::default();
    for (lang, t) in by {
        t.row(lang);
        all.add(t);
    }
    all.row("all");
    let rate = all.false_edits as f64 * 100.0 / all.correct.max(1) as f64;
    println!(
        "false-edit rate {rate:.1}% (gate <= 2%), wrong made worse {}, recall exact {:.0}% + partly {:.0}%, p50 for <= 40 words {:.0} ms (gate <= 1000)",
        all.worse,
        all.exact as f64 * 100.0 / all.wrong.max(1) as f64,
        all.partly as f64 * 100.0 / all.wrong.max(1) as f64,
        pct(short, 0.5)
    );
    println!("By goal:");
    for (lang, t) in by.iter().chain([(&"all".to_string(), &all)]) {
        println!(
            "  {lang:4} G1 words fixed {:5.1}% of {:4} (>= 90)   G4 right lines changed {:4.1}% (<= 0.5)   G5 changes right {:5.1}% of {:4} (>= 97)",
            t.fixed as f64 * 100.0 / t.needed.max(1) as f64,
            t.needed,
            t.false_edits as f64 * 100.0 / t.correct.max(1) as f64,
            t.fixed as f64 * 100.0 / t.made.max(1) as f64,
            t.made
        );
    }
    rate
}

/// Answers no fix should be, put to the guard alone: the helper's decoding
/// already refuses to write them, but the guard is the last word.
fn probe_guard() {
    let probes = [
        ("a word dropped for a mark", "Meet the landlord on friday and ask about the heating.", "Meet the landlord on Friday; ask about the heating."),
        ("the title put in front", "I recieve mail every day.", "Plan I receive mail every day."),
        ("a synonym", "La reunión de mañana empieza a las diez.", "La reunión de mañana comienza a las diez."),
        ("a tense", "電車が遅れたので、会社に遅刻しまた。", "電車が遅れたので、会社に遅刻します。"),
        ("a question made a statement", "大家さんから返事は来ましたか？", "大家さんから返事は来ました。"),
        ("a politer ending", "오늘 4장을 다 읽었다.", "오늘 4장을 다 읽었습니다."),
        ("a full stop after a fragment", "Call the dentist about moving Thursday's appointment", "Call the dentist about moving Thursday's appointment."),
        ("another verb form", "Perguntar à Carla se o orçamento inclui viagens.", "Pergunte à Carla se o orçamento inclui viagens."),
        ("a number", "The flight lands at 6:40 pm.", "The flight lands at 6:45 pm."),
        ("a number spelled out", "We need 3 boxes for the move.", "We need three boxes for the move."),
        ("another pronoun", "我对这个结果非常满意。", "你对这个结果非常满意。"),
        ("a question reworded", "Hat sich der Vermieter schon gemeldet?", "Hast du den Vermieter schon gemeldet?"),
        ("an answer", "Est-ce que le propriétaire nous a répondu ?", "Oui, le propriétaire nous a répondu."),
        ("vowel marks added", "ذهبت إلى السوق في الصباح.", "ذهبتُ إلى السوقِ في الصباحِ."),
    ];
    let mut let_through = 0;
    println!("\nGuard probes (answers that are not fixes):");
    for (what, input, answer) in probes {
        let masked = mask::mask(input);
        if let Some(after) = applied(input, &masked, answer).filter(|a| a != input) {
            let_through += 1;
            println!("  accepted: {what}: {input} => {after}");
        }
    }
    println!("Guard: {} of {} refused", probes.len() - let_through, probes.len());
}

fn shape(h: &mut Helper, show: bool) {
    let (mut pass, mut n, mut asked, mut ms) = (0, 0, 0, Vec::new());
    for f in lines("shape.tsv") {
        let (lang, input, expected) = (&f[0], &f[1], &f[2]);
        n += 1;
        // As the app does: only what looks like a run is asked about.
        let got = if shapes::worth_asking(input) {
            asked += 1;
            let (reply, took) = h.ask(Kind::Shape, "", input);
            ms.push(took.as_secs_f64() * 1000.0);
            let answer = if reply.status == Status::Done { reply.text.clone() } else { String::new() };
            match shapes::list(input, &answer) {
                Some(list) => format!("LIST {}", list.lines().filter_map(|l| l.strip_prefix("- ")).collect::<Vec<_>>().join(" | ")),
                None if answer.trim() == "KEEP" => "KEEP".to_string(),
                None => format!("KEEP (refused: {})", answer.replace('\n', "\\n")),
            }
        } else {
            "KEEP (not asked)".to_string()
        };
        let ok = got == *expected || (expected == "KEEP" && got.starts_with("KEEP"));
        pass += usize::from(ok);
        if show || !ok {
            println!("[{lang}] {} {input}\n       -> {got}", if ok { "ok  " } else { "MISS" });
        }
    }
    println!("\nShape: {pass}/{n} as expected ({asked} asked), p50 {:.0} ms, p95 {:.0} ms", pct(&mut ms, 0.5), pct(&mut ms, 0.95));
}

/// Up to 8 notes of 150 words or more, with their titles.
fn notes() -> Vec<(String, String)> {
    let mut paths = Vec::new();
    let mut dirs = vec![root().join("vaults/my-notes-copy")];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                paths.push(p);
            }
        }
    }
    paths.sort();
    paths
        .into_iter()
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), std::fs::read_to_string(&p).ok()?)))
        .filter(|(_, s)| text::words(s) >= gists::MIN_WORDS)
        .take(8)
        .collect()
}

fn gist(h: &mut Helper) {
    let (mut ms, mut pass, mut n) = (Vec::new(), 0, 0);
    let mut samples = Vec::new();
    for (i, (title, note)) in notes().iter().enumerate() {
        let source = gists::source(note);
        let (reply, took) = h.ask(Kind::Gist, title, &source);
        ms.push(took.as_secs_f64() * 1000.0);
        let got = (reply.status == Status::Done).then(|| gists::accept(&reply.text, title, note)).flatten();
        n += 1;
        pass += usize::from(got.is_some());
        let same_script = text::dominant(&reply.text) == text::dominant(&source);
        println!(
            "gist {i}: note {} words, source {} chars, answer {} words / {} chars, {}, {:?}, {}, {:.0} ms",
            text::words(note),
            source.chars().count(),
            text::words(&reply.text),
            reply.text.chars().count(),
            if same_script { "note's script".to_string() } else { format!("{:?} for a {:?} note", text::dominant(&reply.text), text::dominant(&source)) },
            reply.status,
            if got.is_some() { "accepted" } else { "refused" },
            took.as_secs_f64() * 1000.0
        );
        if let Some(g) = got
            && samples.len() < 3
        {
            samples.push(g);
        }
    }
    for s in samples {
        println!("  sample: {s}");
    }
    println!("Gist: {pass}/{n} accepted, p50 {:.0} ms, p95 {:.0} ms", pct(&mut ms, 0.5), pct(&mut ms, 0.95));
}

fn speeds(log: &str) {
    let (mut np, mut nc, mut no, mut pm, mut dm, mut kept, mut left) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for l in log.lines() {
        if let Some(rest) = l.strip_prefix("kolada-ai: ") {
            if rest.starts_with("loaded ") {
                println!("{rest}");
            }
            let field = |k: &str| rest.split(' ').find_map(|w| w.strip_prefix(k)).map(|v| v.trim_end_matches("ms").parse::<f64>().unwrap_or(0.0));
            if let (Some(p), Some(o), Some(a), Some(b)) = (field("prompt="), field("out="), field("prefill="), field("decode=")) {
                np += p;
                nc += field("cached=").unwrap_or(0.0);
                no += o;
                pm += a;
                dm += b;
                kept += field("kept=").unwrap_or(0.0);
                left += field("left=").unwrap_or(0.0);
            }
        }
    }
    println!(
        "prefill {:.0} tokens/s ({} read, {nc} restored), decode {:.0} tokens/s ({no} tokens; anchor held {kept}, left {left})",
        (np - nc) * 1000.0 / pm.max(1.0),
        np - nc,
        no * 1000.0 / dm.max(1.0)
    );
}

/// The helper's side of the protocol, with the real model: a fix put before
/// a running gist, a newer job for the same place, another note opened, and
/// exactly one reply for each request.
#[test]
#[ignore = "runs the real model; see the top of this file"]
fn protocol() {
    let missing = Command::new(env!("CARGO_BIN_EXE_kolada-ai")).args(["--model", "no-such.gguf"]).stdin(Stdio::null()).output().expect("spawn");
    assert_eq!(missing.status.code(), Some(2), "a model that does not load");
    assert!(missing.stdout.is_empty(), "nothing but frames on stdout");
    assert_eq!(Command::new(env!("CARGO_BIN_EXE_kolada-ai")).output().expect("spawn").status.code(), Some(1), "no arguments");

    let mut h = Helper::start();
    h.ask(Kind::Fix, "", "Warm up.");
    let long = "The quarterly plan covers hiring, the new office and the launch in spring. ".repeat(40);
    let send = |h: &mut Helper, kind: Kind, id: u64, note: u64, slot: u64, text: &str| {
        let r = Request { kind, id, note, slot, context: String::new(), text: text.into() };
        wire::write_frame(&mut h.stdin, &r.frame()).expect("write");
    };
    let replies = |h: &mut Helper, n: usize| -> Vec<Reply> {
        (0..n).map(|_| Reply::parse(&wire::read_frame(&mut h.stdout).unwrap().unwrap()).unwrap()).collect()
    };

    send(&mut h, Kind::Gist, 100, 1, 0, &long);
    std::thread::sleep(Duration::from_millis(150));
    send(&mut h, Kind::Fix, 101, 1, 7, "She dont like it.");
    let r = replies(&mut h, 2);
    assert_eq!((r[0].id, r[0].status), (101, Status::Done), "the fix goes first");
    assert_eq!((r[1].id, r[1].status), (100, Status::Done), "the gist is done after it");

    send(&mut h, Kind::Fix, 102, 1, 9, "The first try at this line.");
    send(&mut h, Kind::Fix, 103, 1, 9, "The second try at this line.");
    let r = replies(&mut h, 2);
    let status = |id: u64| r.iter().find(|x| x.id == id).map(|x| x.status);
    assert_eq!(status(102), Some(Status::Cancelled), "replaced");
    assert_eq!(status(103), Some(Status::Done));

    send(&mut h, Kind::Gist, 104, 1, 0, &long);
    send(&mut h, Kind::Fix, 105, 1, 3, "Another line of the old note.");
    std::thread::sleep(Duration::from_millis(100));
    send(&mut h, Kind::Cancel, 0, 2, 0, "");
    send(&mut h, Kind::Fix, 106, 1, 4, "A late line of the old note.");
    send(&mut h, Kind::Fix, 107, 2, 4, "A line of the new note.");
    let r = replies(&mut h, 4);
    for id in [104, 105, 106] {
        let x = r.iter().find(|x| x.id == id).unwrap_or_else(|| panic!("no reply for {id}"));
        assert!(x.status == Status::Cancelled || (id == 105 && x.status == Status::Done), "{id}: {:?}", x.status);
    }
    assert_eq!(r.iter().find(|x| x.id == 107).map(|x| (x.note, x.status)), Some((2, Status::Done)));
    let log = h.close();
    assert!(log.contains("cancel, keeping note 2"));
}

// The helper's own dictionaries; the eval uses only what the baseline needs.
#[allow(dead_code)]
#[path = "../src/spell.rs"]
mod spell;

/// The fixes a spell checker alone makes, each unknown word put for its
/// first guess, scored as the model is: what the model must beat.
#[test]
#[ignore = "reads EVAL_FIX; see the top of this file"]
fn spell_baseline() {
    let mut dict = spell::Dictionaries::new();
    let mut ask = |_: &str, _: &str, text: &str| {
        let t = Instant::now();
        let text = dict.corrected(kolada_assist::prompt::language(text), text);
        (Reply { id: 0, note: 0, status: Status::Done, text, ms: 0 }, t.elapsed())
    };
    let (mut short, by) = fix(&mut ask, var("EVAL_SHOW").is_some(), Context::None);
    fix_table(&mut short, &by, Context::None);
}

#[test]
#[ignore = "runs the real model; see the top of this file"]
fn eval() {
    assert!(model().is_file(), "no model at {}", model().display());
    let only = var("EVAL_ONLY");
    let show = var("EVAL_SHOW").is_some();
    let mut h = Helper::start();
    let (warm, took) = h.ask(Kind::Fix, "", "Hello there.");
    println!("first answer {:.0} ms after the start ({:.0} ms for it): {:?}", h.started.elapsed().as_secs_f64() * 1000.0, took.as_secs_f64() * 1000.0, warm.text);
    let mut rate: f64 = 0.0;
    if only.as_deref().is_none_or(|o| o == "fix") {
        let modes = var("EVAL_CONTEXT").unwrap_or_else(|| "none,title".into());
        for mode in modes.split(',') {
            let context = match mode.trim() {
                "para" => Context::Para,
                "title" => Context::Title,
                _ => Context::None,
            };
            let (mut short, by) = fix(&mut |_: &str, c: &str, t: &str| h.ask(Kind::Fix, c, t), show, context);
            rate = rate.max(fix_table(&mut short, &by, context));
        }
        probe_guard();
    }
    if only.as_deref().is_none_or(|o| o == "shape") {
        shape(&mut h, show);
    }
    if only.as_deref().is_none_or(|o| o == "gist") {
        gist(&mut h);
    }
    let (peak, private) = h.memory();
    println!("helper peak working set {peak:.0} MB, private bytes {private:.0} MB");
    let log = h.close();
    speeds(&log);
    if let Some(t) = log.lines().find_map(|l| l.strip_prefix("kolada-ai: template ")) {
        println!("chat template: {}…", t.chars().take(60).collect::<String>());
    }
    assert!(rate <= 2.0, "false-edit rate {rate:.1}% is over 2%");
}
