//! Documents dropped or imported into notes: a DOCX, PDF or HTML file
//! becomes a Markdown note (see `kolada_convert`), its pictures saved with
//! the vault's attachments, dated as the document was.
//!
//! The work happens in a process of its own (`kolada --convert-into`), at
//! low priority: the window's process never runs the converter, so a large
//! or damaged document can neither slow typing nor take the app down, and
//! its memory is given back the moment it is done.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use kolada_convert::Kind;
use kolada_convert::import::Source;
use kolada_core::note;

/// The flag that starts a converting process.
pub const FLAG: &str = "--convert-into";
/// The flag that starts a process converting copied HTML for a paste.
pub const PASTE_FLAG: &str = "--paste-markdown";
/// How long a paste waits for its Markdown before taking the plain text.
const PASTE_WAIT: Duration = Duration::from_millis(1500);

/// Between a paste's Markdown and the pictures to fetch, in what the
/// pasting process answers.
const FETCH_AFTER: &str = "\n\u{1E}\n";

/// What a paste brings: its Markdown, its pictures embedded by the names
/// they were saved under, and those still to fetch from the web (each: the
/// file it goes to, made empty to hold the name, and its address).
pub struct Paste {
    pub markdown: String,
    pub fetch: Vec<(PathBuf, String)>,
}

/// What was copied with its formatting (the clipboard's HTML, see
/// `clipboard::cf_html_page`) as Markdown to paste, its pictures saved in
/// `pictures` (without it, left out), converted in a process of its own as
/// documents are: None when its plain text is as good, or when converting
/// it fails or takes too long.
pub fn paste(cf_html: &[u8], pictures: Option<&Path>) -> Option<Paste> {
    let exe = std::env::current_exe().ok()?;
    let mut c = Command::new(exe);
    c.arg(PASTE_FLAG).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    if let Some(dir) = pictures {
        c.arg(dir);
    }
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = c.spawn().ok()?;
    let (mut stdin, mut stdout) = (child.stdin.take()?, child.stdout.take()?);
    let bytes = cf_html.to_vec();
    // Written and read beside the wait, so a large paste fills no pipe.
    std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(&bytes);
    });
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        out
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() < PASTE_WAIT => std::thread::sleep(Duration::from_millis(1)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = reader.join().ok()?;
    if !status?.success() || out.trim().is_empty() {
        return None;
    }
    let (markdown, list) = out.split_once(FETCH_AFTER).unwrap_or((&out, ""));
    let fetch = list.lines().filter_map(|l| l.split_once('\t')).map(|(path, url)| (PathBuf::from(path), url.to_string())).collect();
    Some(Paste { markdown: markdown.to_string(), fetch })
}

/// The pasting process: `kolada --paste-markdown [pictures folder]`, the
/// clipboard's HTML on stdin. Its Markdown on stdout, then the pictures to
/// fetch, a line each (`file<TAB>address`), after `FETCH_AFTER` (exit 0);
/// or nothing (exit 1) when its plain text is as good.
pub fn paste_child(args: &[std::ffi::OsString]) -> i32 {
    let pictures = args.first().map(PathBuf::from);
    let mut bytes = Vec::new();
    if std::io::stdin().read_to_end(&mut bytes).is_err() {
        return 2;
    }
    let Some((page, source)) = crate::clipboard::cf_html_page(&bytes) else { return 1 };
    let Some(pasted) = kolada_convert::paste_html(&page, source.as_deref()) else { return 1 };
    let (markdown, fetch) = save_pasted(pasted, pictures.as_deref(), &stamp());
    let mut answer = markdown;
    if !fetch.is_empty() {
        answer.push_str(FETCH_AFTER);
        for (path, url) in &fetch {
            answer.push_str(&format!("{}\t{url}\n", path.display()));
        }
    }
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    if out.write_all(answer.as_bytes()).and_then(|_| out.flush()).is_err() {
        return 2;
    }
    0
}

/// The moment, as Obsidian names pasted pictures by it: `20261005183012`.
fn stamp() -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}{:02}{:02}{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

/// Saves a paste's pictures in `dir`, named as Obsidian names pasted
/// pictures (`Pasted image <stamp>.png`): those inside it, and those Word
/// or Outlook keep on this machine, at once; for each on the web, an empty
/// file holds its name until it is fetched. The Markdown with each picture
/// embedded by its saved name (one that cannot be kept, or without `dir`,
/// left out), and the pictures to fetch.
fn save_pasted(p: kolada_convert::Pasted, dir: Option<&Path>, stamp: &str) -> (String, Vec<(PathBuf, String)>) {
    let mut text = p.markdown;
    let mut fetch = Vec::new();
    let ready = dir.is_some_and(|d| std::fs::create_dir_all(d).is_ok());
    let place = |name: &str, save: &mut dyn FnMut(&Path) -> bool| -> Option<PathBuf> {
        let dir = dir.filter(|_| ready)?;
        let ext = name.rsplit_once('.').map_or("png", |(_, e)| e);
        let to = free_name(dir, &format!("Pasted image {stamp}.{ext}"));
        save(&to).then_some(to)
    };
    /// Where a picture of the paste is.
    enum Source<'a> {
        Inside(&'a [u8]),
        File(&'a str),
        Web(&'a str),
    }
    let mut all: Vec<(&str, Source)> = p.pictures.iter().map(|pic| (pic.name.as_str(), Source::Inside(&pic.data))).collect();
    all.extend(p.fetch.iter().map(|(name, s)| (name.as_str(), s.strip_prefix("file:").map_or(Source::Web(s), Source::File))));
    // Named in the order the note shows them, the first the plain name.
    all.sort_by_key(|(name, _)| text.find(&format!("![[{name}]]")).unwrap_or(usize::MAX));
    let mut kept: Vec<(String, Option<PathBuf>)> = Vec::new();
    for (name, source) in all {
        let saved = match source {
            Source::Inside(data) => place(name, &mut |to| std::fs::write(to, data).is_ok()),
            Source::File(path) => place(name, &mut |to| std::fs::copy(path, to).is_ok()),
            Source::Web(url) => {
                let saved = place(name, &mut |to| std::fs::File::create(to).is_ok());
                if let Some(to) = &saved {
                    fetch.push((to.clone(), url.to_string()));
                }
                saved
            }
        };
        kept.push((name.to_string(), saved));
    }
    for (name, saved) in kept {
        let now = saved.as_deref().map_or_else(String::new, |to| format!("![[{}]]", name_of(to)));
        text = text.replace(&format!("![[{name}]]"), &now);
    }
    // Where a picture was left out, one blank line, not more.
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    (text.trim().to_string(), fetch)
}

/// Whether `path` is a document that converts into a note, or another
/// app's export that the converter reads into many (Evernote, Notion).
pub fn is_document(path: &Path) -> bool {
    Kind::of(path).is_some() || Source::of(path).is_some()
}

/// What converting found: the notes written, and the files that could
/// not be read (their names, and why).
#[derive(Debug, Default)]
pub struct Converted {
    pub notes: Vec<PathBuf>,
    pub failed: Vec<(String, String)>,
}

/// Converts `files` into notes in `dir`, their pictures into `pictures`,
/// in a process of its own; waits for it (on a worker, never the window's
/// thread).
pub fn run(files: &[PathBuf], dir: &Path, pictures: &Path) -> Converted {
    let mut done = Converted::default();
    let mut args: Vec<std::ffi::OsString> = vec![dir.into(), pictures.into()];
    args.extend(files.iter().map(|f| f.into()));
    let (out, finished) = match converter(&args, files) {
        Ok(got) => got,
        Err(why) => {
            done.failed.push((String::new(), why));
            return done;
        }
    };
    let out = String::from_utf8_lossy(&out);
    for line in out.lines() {
        let mut parts = line.splitn(3, '\t');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("ok"), Some(path), _) => done.notes.push(PathBuf::from(path)),
            (Some("fail"), Some(name), Some(why)) => done.failed.push((name.to_string(), why.to_string())),
            _ => {}
        }
    }
    // Files the converter never answered for (it stopped, or broke).
    let answered = done.notes.len() + done.failed.len();
    if answered < files.len() {
        let why = if finished { "it could not be read" } else { "it took too long to read" };
        for f in &files[answered..] {
            done.failed.push((name_of(f), why.to_string()));
        }
    }
    done
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The converting process: `kolada --convert-into <dir> <pictures> <files>`.
/// One line per file on stdout: `ok<TAB>note path`, or
/// `fail<TAB>file name<TAB>why`. Its exit code.
pub fn child(args: &[std::ffi::OsString]) -> i32 {
    if args.first().is_some_and(|a| a == TEXT_FLAG) {
        return text_child(&args[1..]);
    }
    let [dir, pictures, files @ ..] = args else { return 2 };
    let (dir, pictures) = (PathBuf::from(dir), PathBuf::from(pictures));
    for f in files {
        let f = PathBuf::from(f);
        let done = match Source::of(&f) {
            Some(source) => import(&f, source, &dir, &pictures),
            None => one(&f, &dir, &pictures).map(|p| vec![p]),
        };
        match done {
            Ok(paths) => paths.iter().for_each(|p| println!("ok\t{}", p.display())),
            Err(why) => println!("fail\t{}\t{}", name_of(&f), why.replace(['\t', '\n'], " ")),
        }
    }
    0
}

/// Runs the converting process with `args` after its flag, waiting for it:
/// a minute, and more for big `files`; past that it is stopped. What it
/// wrote, and whether it finished.
fn converter(args: &[std::ffi::OsString], files: &[PathBuf]) -> Result<(Vec<u8>, bool), String> {
    let exe = std::env::current_exe().map_err(|_| "Kolada could not start its converter".to_string())?;
    let mut c = Command::new(exe);
    c.arg(FLAG).args(args).stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
    {
        use std::os::windows::process::CommandExt;
        // No console window; below normal priority, so typing stays quick.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        c.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    let mut child = c.spawn().map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let bytes: u64 = files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum();
    let limit = Duration::from_secs(60 + bytes / (1024 * 1024) * 2);
    let start = Instant::now();
    let finished = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    Ok((reader.join().unwrap_or_default(), finished))
}

/// After the flag, asks the converting process for files' text only.
const TEXT_FLAG: &str = "--text";

/// The text of each of `files` (PDFs), for search, read by the converting
/// process: empty for one with no text it can read, None for one it did not
/// reach (it was stopped, or broke on an earlier file).
pub fn text_of(files: &[PathBuf]) -> Vec<Option<String>> {
    let mut args: Vec<std::ffi::OsString> = vec![TEXT_FLAG.into()];
    args.extend(files.iter().map(|f| f.into()));
    match converter(&args, files) {
        Ok((out, _)) => read_texts(&out, files.len()),
        Err(_) => vec![None; files.len()],
    }
}

/// What `text_child` wrote, back as the text of each of `n` files.
fn read_texts(out: &[u8], n: usize) -> Vec<Option<String>> {
    let mut texts = vec![None; n];
    // Each: `<index><TAB><length><LF>` then the text.
    let mut at = 0;
    while let Some(end) = out[at..].iter().position(|&b| b == b'\n').map(|e| at + e) {
        let head = String::from_utf8_lossy(&out[at..end]);
        let Some((i, len)) = head.split_once('\t').and_then(|(i, l)| Some((i.parse::<usize>().ok()?, l.parse::<usize>().ok()?))) else { break };
        let Some(text) = out.get(end + 1..end + 1 + len) else { break };
        if let Some(slot) = texts.get_mut(i) {
            *slot = Some(String::from_utf8_lossy(text).into_owned());
        }
        at = end + 1 + len;
    }
    texts
}

/// The vault's PDFs for search, by their names and text. Text is read once
/// per file (by `text_of`) and kept in the app's data; one that stops the
/// reader is kept as having none, so the next time goes on past it.
pub fn index_files(vault: &Path) -> Vec<kolada_core::search::Indexed> {
    use kolada_core::search::{self, Indexed, TextCache};
    let files = search::files(vault);
    let at = kolada_core::app_data_dir().map(|d| TextCache::path(&d, vault));
    let mut cache = at.as_deref().map(TextCache::load).unwrap_or_default();
    // Pictures take a while each: so many a time, the rest the next.
    let mut pictures = 0;
    let todo: Vec<&search::File> = files
        .iter()
        .filter(|f| cache.get(f).is_none())
        .filter(|f| !search::is_picture(&f.rel) || { pictures += 1; pictures <= PICTURES_AT_ONCE })
        .collect();
    if !todo.is_empty() {
        let texts = text_of(&todo.iter().map(|f| f.path.clone()).collect::<Vec<_>>());
        let stopped = texts.iter().position(Option::is_none);
        for (k, (f, text)) in todo.iter().zip(texts).enumerate() {
            match text {
                Some(text) => cache.put(f, text),
                None if stopped == Some(k) => cache.put(f, String::new()),
                None => {}
            }
        }
    }
    cache.keep_only(&files);
    if let Some(at) = &at {
        let _ = cache.save(at);
    }
    // PDFs by their names too; pictures only by words found in them.
    files
        .iter()
        .filter_map(|f| {
            let text = cache.get(f).unwrap_or_default();
            (!search::is_picture(&f.rel) || !text.trim().is_empty()).then(|| Indexed::new(f.rel.clone(), text.to_string()))
        })
        .collect()
}

/// The converting process asked for text: `kolada --convert-into --text
/// <files>`. Each file read is written as `<index><TAB><length><LF>` and its
/// text (empty when it has none it can read), as soon as it is read.
fn text_child(files: &[std::ffi::OsString]) -> i32 {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let mut ocr = None;
    for (i, f) in files.iter().enumerate() {
        let f = PathBuf::from(f);
        let text = if kolada_core::search::is_picture(&f.to_string_lossy()) {
            ocr.get_or_insert_with(Ocr::new).read(&f).unwrap_or_default()
        } else {
            Kind::of(&f)
                .and_then(|kind| std::fs::read(&f).ok().and_then(|b| kolada_convert::convert(kind, &b).ok()))
                .map(|n| n.markdown)
                .unwrap_or_default()
        };
        let _ = write!(out, "{i}\t{}\n", text.len());
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
    0
}

/// Pictures read for words in one go (see `index_files`).
const PICTURES_AT_ONCE: usize = 200;

/// Windows' own text recognition, in the languages the user has set: it
/// comes with Windows, runs on this machine, and needs nothing downloaded.
struct Ocr {
    engine: Option<windows::Media::Ocr::OcrEngine>,
}

impl Ocr {
    fn new() -> Ocr {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Ocr { engine: windows::Media::Ocr::OcrEngine::TryCreateFromUserProfileLanguages().ok() }
    }

    /// The words in the picture at `path`, a line each; None when it cannot
    /// be read.
    fn read(&self, path: &Path) -> Option<String> {
        use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode, ExifOrientationMode};
        use windows::Media::Ocr::OcrEngine;
        use windows::Storage::{FileAccessMode, StorageFile};
        let engine = self.engine.as_ref()?;
        let file = StorageFile::GetFileFromPathAsync(&windows_core::HSTRING::from(path.as_os_str())).ok()?.join().ok()?;
        let stream = file.OpenAsync(FileAccessMode::Read).ok()?.join().ok()?;
        let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.join().ok()?;
        // Larger than it reads, it is made smaller first.
        let (w, h) = (decoder.OrientedPixelWidth().ok()?, decoder.OrientedPixelHeight().ok()?);
        let most = OcrEngine::MaxImageDimension().ok()?;
        let transform = BitmapTransform::new().ok()?;
        if w.max(h) > most {
            let scale = most as f64 / w.max(h) as f64;
            transform.SetScaledWidth(((w as f64 * scale) as u32).max(1)).ok()?;
            transform.SetScaledHeight(((h as f64 * scale) as u32).max(1)).ok()?;
        }
        let bitmap = decoder
            .GetSoftwareBitmapTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Premultiplied,
                &transform,
                ExifOrientationMode::RespectExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )
            .ok()?
            .join()
            .ok()?;
        let result = engine.RecognizeAsync(&bitmap).ok()?.join().ok()?;
        let lines: Vec<String> = result.Lines().ok()?.into_iter().filter_map(|l| l.Text().ok().map(|t| t.to_string())).collect();
        Some(lines.join("\n"))
    }
}

/// Converts one document into a note in `dir`; its path.
fn one(file: &Path, dir: &Path, pictures: &Path) -> Result<PathBuf, String> {
    let kind = Kind::of(file).ok_or("not a document Kolada reads")?;
    let bytes = std::fs::read(file).map_err(|e| said(&e))?;
    let converted = kolada_convert::convert(kind, &bytes).map_err(|e| e.to_string())?;
    let mut text = converted.markdown;
    // Pictures inside the document, saved beside the vault's others; the
    // note names them as saved.
    let mut saved = Vec::new();
    for p in &converted.pictures {
        let _ = std::fs::create_dir_all(pictures);
        let to = free_name(pictures, &p.name);
        if std::fs::write(&to, &p.data).is_ok() {
            saved.push((p.name.clone(), name_of(&to)));
        }
    }
    // Pictures beside an HTML page, copied in; ones not there, left out.
    let from_dir = file.parent().unwrap_or(Path::new("."));
    let mut missing = Vec::new();
    for (name, rel) in &converted.beside {
        let src = from_dir.join(rel);
        if src.is_file() {
            let _ = std::fs::create_dir_all(pictures);
            let to = free_name(pictures, name);
            if std::fs::copy(&src, &to).is_ok() {
                saved.push((name.clone(), name_of(&to)));
                continue;
            }
        }
        missing.push(name.clone());
    }
    for (was, now) in &saved {
        if was != now {
            text = text.replace(&format!("![[{was}]]"), &format!("![[{now}]]"));
        }
    }
    if !missing.is_empty() {
        let gone: Vec<String> = missing.iter().map(|n| format!("![[{n}]]")).collect();
        let kept: Vec<&str> = text.lines().filter(|l| !gone.iter().any(|g| l.trim() == g)).collect();
        // Blank lines left where they were, one.
        let mut joined = String::new();
        for line in kept {
            if line.is_empty() && (joined.is_empty() || joined.ends_with("\n\n")) {
                continue;
            }
            joined.push_str(line);
            joined.push('\n');
        }
        text = format!("{}\n", joined.trim_end());
    }
    // The note's first line is its title: the document's own heading, or
    // else the title it gives itself, or its file's name.
    if !text.trim_start().starts_with('#') {
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let title = converted.title.filter(|t| !t.trim().is_empty()).unwrap_or(stem);
        text = format!("# {}\n\n{}", title.trim(), text.trim_start());
    }
    let stem = note::file_stem_for(&kolada_editor::title_of(&text));
    std::fs::create_dir_all(dir).map_err(|e| said(&e))?;
    let to = note::unique_path(dir, &stem, None);
    std::fs::write(&to, &text).map_err(|e| said(&e))?;
    // Dated as the document was, so notes sort as their sources did.
    if let Ok(meta) = std::fs::metadata(file) {
        let mut times = std::fs::FileTimes::new();
        if let Ok(m) = meta.modified() {
            times = times.set_modified(m).set_accessed(m);
        }
        {
            use std::os::windows::fs::FileTimesExt;
            if let Ok(c) = meta.created() {
                times = times.set_created(c);
            }
        }
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&to) {
            let _ = f.set_times(times);
        }
    }
    Ok(to)
}

/// Another app's export read into notes, in a folder of `dir` named after
/// it, their attachments into `pictures`; the notes' paths.
fn import(file: &Path, source: Source, dir: &Path, pictures: &Path) -> Result<Vec<PathBuf>, String> {
    let bytes = std::fs::read(file).map_err(|e| said(&e))?;
    let export = kolada_convert::import::read(source, &bytes).map_err(|e| e.to_string())?;
    if export.notes.is_empty() {
        return Err("it has no notes".to_string());
    }
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // Notion names its exports `Export-<id>`.
    let name = if source == Source::Notion && (stem.starts_with("Export-") || stem.is_empty()) { "Notion".to_string() } else { stem };
    kolada_convert::import::write(&export, &name, dir, pictures).map_err(|e| e.to_string())
}

/// A file error in words a person would use.
fn said(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "it is not there any more".to_string(),
        std::io::ErrorKind::PermissionDenied => "Windows did not let Kolada open it".to_string(),
        _ if e.raw_os_error() == Some(32) => "another app has it open".to_string(),
        _ => "it could not be opened".to_string(),
    }
}

/// `dir/name`, or `dir/name 1.ext`, `name 2.ext`... if taken.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = name.rsplit_once('.').map_or((name, ""), |(s, e)| (s, e));
    (0..)
        .map(|n| match (n, ext) {
            (0, _) => dir.join(name),
            (n, "") => dir.join(format!("{stem} {n}")),
            (n, ext) => dir.join(format!("{stem} {n}.{ext}")),
        })
        .find(|p| !p.exists())
        .expect("a free name exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_come_back_framed_and_a_cut_off_one_is_not_reached() {
        let out = "0\t10\nline\none 22\t0\n1\t3\nabc3\t10\ncut".as_bytes();
        assert_eq!(read_texts(out, 4), [Some("line\none 2".to_string()), Some("abc".to_string()), Some(String::new()), None]);
        assert_eq!(read_texts(b"", 2), [None, None]);
        assert_eq!(read_texts(b"9\t1\nx", 1), [None], "an index out of range is ignored");
    }

    #[test]
    fn a_pastes_pictures_are_saved_and_web_ones_wait_to_be_fetched() {
        let dir = std::env::temp_dir().join(format!("kolada-paste-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pictures = dir.join("attachments");
        std::fs::create_dir_all(&dir).unwrap();
        // Word keeps a copied picture in a file of its own for a while.
        let word = dir.join("clip_image001.png");
        std::fs::write(&word, b"\x89PNG\r\n\x1a\nword").unwrap();
        let pasted = kolada_convert::Pasted {
            markdown: "Intro\n\n![[picture 1.png]]\n\n![[picture 2.jpg]]\n\n![[picture 3.png]]\n\n![[picture 4.png]]\n\nEnd".to_string(),
            pictures: vec![kolada_convert::Picture { name: "picture 3.png".into(), data: b"\x89PNG\r\n\x1a\ninside".to_vec() }],
            fetch: vec![
                ("picture 1.png".into(), format!("file:{}", word.display())),
                ("picture 2.jpg".into(), "https://example.com/cat.jpg".into()),
                ("picture 4.png".into(), "file:C:/no/such/clip.png".into()),
            ],
        };
        let (text, fetch) = save_pasted(pasted, Some(&pictures), "20261005183012");
        // Saved as Obsidian names pasted pictures, none taking another's
        // name; the one gone missing left out.
        assert_eq!(
            text,
            "Intro\n\n![[Pasted image 20261005183012.png]]\n\n![[Pasted image 20261005183012.jpg]]\n\n![[Pasted image 20261005183012 1.png]]\n\nEnd"
        );
        assert_eq!(std::fs::read(pictures.join("Pasted image 20261005183012.png")).unwrap(), b"\x89PNG\r\n\x1a\nword");
        assert_eq!(std::fs::read(pictures.join("Pasted image 20261005183012 1.png")).unwrap(), b"\x89PNG\r\n\x1a\ninside");
        // The web one: its name held by an empty file, to be fetched.
        assert_eq!(fetch, [(pictures.join("Pasted image 20261005183012.jpg"), "https://example.com/cat.jpg".to_string())]);
        assert_eq!(std::fs::metadata(&fetch[0].0).unwrap().len(), 0);
        // Nowhere to save them: the text alone.
        let pasted = kolada_convert::Pasted { markdown: "A\n\n![[picture 1.png]]\n\nB".into(), pictures: Vec::new(), fetch: vec![("picture 1.png".into(), "https://x/y.png".into())] };
        assert_eq!(save_pasted(pasted, None, "1"), ("A\n\nB".to_string(), Vec::new()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_document_becomes_a_dated_note_with_its_pictures() {
        let base = std::env::temp_dir().join(format!("kolada-convert-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (vault, pictures) = (base.join("vault"), base.join("vault").join("attachments"));
        std::fs::create_dir_all(&vault).unwrap();
        let doc = base.join("Plan.docx");
        std::fs::write(&doc, include_bytes!("../../kolada-convert/testdata/sample.docx")).unwrap();
        // An existing picture of the same name: the note's gets another.
        std::fs::create_dir_all(&pictures).unwrap();
        std::fs::write(pictures.join("Picture 1.png"), b"taken").unwrap();
        let path = one(&doc, &vault, &pictures).unwrap();
        assert_eq!(path, vault.join("Launch plan.md"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Launch plan\n\nShip the **public beta**"), "{text}");
        assert!(text.contains("![[Picture 1 1.png]]"), "{text}");
        assert!(std::fs::read(pictures.join("Picture 1 1.png")).unwrap().starts_with(b"\x89PNG"));
        let (src, dst) = (std::fs::metadata(&doc).unwrap(), std::fs::metadata(&path).unwrap());
        assert_eq!(dst.modified().unwrap(), src.modified().unwrap(), "dated as the document");
        // A page with no heading takes its title; a missing picture goes.
        let page = base.join("clip.html");
        std::fs::write(&page, "<title>Saved page</title><p>Some text <img src='gone.png'></p>").unwrap();
        let path = one(&page, &vault, &pictures).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Saved page\n\nSome text\n");
        assert_eq!(one(&base.join("missing.pdf"), &vault, &pictures), Err("it is not there any more".to_string()));
        let _ = std::fs::remove_dir_all(&base);
    }
}
