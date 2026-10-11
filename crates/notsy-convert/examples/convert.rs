//! Converts documents and prints their Markdown, for looking at by eye:
//!
//!     cargo run --release -p notsy-convert --example convert -- file.pdf [more files]
//!
//! With `--summary` first, one line per file and no text: how long it
//! took, and how many headings, list items, table rows and characters.
//! With `--masked` first, the Markdown with every letter an x (Arabic a ب)
//! and every digit a 9: the shape of someone's own document, not its words.

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let summary = args.first().is_some_and(|a| a == "--summary");
    if summary {
        args.remove(0);
    }
    let masked = args.first().is_some_and(|a| a == "--masked");
    if masked {
        args.remove(0);
    }
    let shown = |t: &str| -> String {
        if !masked {
            return t.to_string();
        }
        t.chars()
            .map(|c| match c {
                '\u{0600}'..='\u{06FF}' | '\u{FB50}'..='\u{FEFF}' if c.is_alphabetic() => 'ب',
                c if c.is_alphabetic() => 'x',
                c if c.is_numeric() => '9',
                c => c,
            })
            .collect()
    };
    for path in args {
        let path = std::path::PathBuf::from(path);
        let Some(kind) = notsy_convert::Kind::of(&path) else {
            eprintln!("{}: not a DOCX, PDF or HTML file", path.display());
            continue;
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                println!("error\t{}\t{e}", path.display());
                continue;
            }
        };
        let start = std::time::Instant::now();
        let result = notsy_convert::convert(kind, &bytes);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        match result {
            Ok(note) if summary => {
                let m = &note.markdown;
                let lines = |p: &dyn Fn(&str) -> bool| m.lines().filter(|l| p(l)).count();
                println!(
                    "ok\t{}\t{ms:.1} ms\t{} KB\theadings {}\titems {}\ttable rows {}\tchars {}\tpictures {}",
                    path.display(),
                    bytes.len() / 1024,
                    lines(&|l| l.starts_with('#')),
                    lines(&|l| l.trim_start().starts_with("- ") || l.trim_start().split_once(". ").is_some_and(|(n, _)| n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty())),
                    lines(&|l| l.starts_with('|')),
                    m.chars().count(),
                    note.pictures.len()
                );
            }
            Ok(note) => {
                println!("=== {} ({ms:.1} ms, {} KB, title {:?}, {} pictures)", path.display(), bytes.len() / 1024, note.title.as_deref().map(shown), note.pictures.len());
                print!("{}", shown(&note.markdown));
            }
            Err(e) => println!("error\t{}\t{ms:.1} ms\t{e}", path.display()),
        }
    }
}
