//! A note exported as PDF: its web page (`Model::export_page`, pictures
//! inlined) printed by a program the desktop has, as Windows prints with
//! Edge: a Chromium-family browser headless (the best match for the page),
//! else LibreOffice, else WeasyPrint or wkhtmltopdf. Each works in a folder
//! of its own (a browser in a snap only sees its own), and the PDF is copied
//! to where the person chose. On a thread: printing takes a second or two.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The browsers that print a page to PDF headless, as their commands are named.
const BROWSERS: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome-stable",
    "google-chrome",
    "microsoft-edge-stable",
    "microsoft-edge",
    "brave-browser",
    "vivaldi-stable",
];

/// How a page becomes a PDF here.
#[derive(Clone, Debug, PartialEq)]
pub enum Printer {
    Browser(PathBuf),
    LibreOffice(PathBuf),
    Weasy(PathBuf),
    Wkhtml(PathBuf),
}

/// The first printer found by `find` (a command's path by its name).
pub fn printer(find: impl Fn(&str) -> Option<PathBuf>) -> Option<Printer> {
    BROWSERS
        .iter()
        .find_map(|b| find(b).map(Printer::Browser))
        .or_else(|| ["soffice", "libreoffice"].iter().find_map(|n| find(n).map(Printer::LibreOffice)))
        .or_else(|| find("weasyprint").map(Printer::Weasy))
        .or_else(|| find("wkhtmltopdf").map(Printer::Wkhtml))
}

/// Where a printer works: a browser from a snap sees only its own folder.
pub fn work_dir(p: &Printer, home: Option<&Path>, pid: u32) -> PathBuf {
    if let Printer::Browser(path) = p
        && let Ok(rest) = path.strip_prefix("/snap/bin")
        && let Some(home) = home
    {
        return home.join("snap").join(rest).join("common").join("kolada-print");
    }
    std::env::temp_dir().join(format!("kolada-print-{pid}"))
}

/// The command that prints `html` (in `dir`) to `pdf` (in `dir` too).
pub fn command(p: &Printer, dir: &Path, html: &Path, pdf: &Path) -> Command {
    let mut c;
    match p {
        Printer::Browser(b) => {
            c = Command::new(b);
            c.args(["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--no-pdf-header-footer"])
                .arg(format!("--user-data-dir={}", dir.join("profile").display()))
                .arg(format!("--print-to-pdf={}", pdf.display()))
                .arg(format!("file://{}", html.display()));
        }
        Printer::LibreOffice(s) => {
            c = Command::new(s);
            // A profile of its own, so a LibreOffice already open is left be.
            c.arg(format!("-env:UserInstallation=file://{}", dir.join("lo").display()))
                .args(["--headless", "--convert-to", "pdf:writer_web_pdf_Export", "--outdir"])
                .arg(dir)
                .arg(html);
        }
        Printer::Weasy(w) => {
            c = Command::new(w);
            c.arg(html).arg(pdf);
        }
        Printer::Wkhtml(w) => {
            c = Command::new(w);
            c.args(["--quiet", "--enable-local-file-access"]).arg(html).arg(pdf);
        }
    }
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c
}

/// Prints `page` to `out` with what the desktop has. Err: what to tell the
/// person.
pub fn print(page: &str, out: &Path) -> Result<(), &'static str> {
    let find = |n: &str| super::services::find_tool(n, std::env::var_os("PATH"), std::env::var_os("HOME").map(PathBuf::from)).or_else(|| Some(PathBuf::from("/snap/bin").join(n)).filter(|p| p.is_file()));
    let printer = printer(find).ok_or("PDF needs Chromium, Chrome or LibreOffice")?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = work_dir(&printer, home.as_deref(), std::process::id());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|_| "Could not make the PDF")?;
    let (html, pdf) = (dir.join("note.html"), dir.join("note.pdf"));
    let done = std::fs::write(&html, page).is_ok() && run(command(&printer, &dir, &html, &pdf), Duration::from_secs(90)) && pdf.is_file() && std::fs::copy(&pdf, out).is_ok();
    let _ = std::fs::remove_dir_all(&dir);
    if done { Ok(()) } else { Err("Could not make the PDF") }
}

/// Runs `c` until it ends or `most` has passed (then it is stopped);
/// whether it ended well.
fn run(mut c: Command, most: Duration) -> bool {
    let Ok(mut child) = c.spawn() else { return false };
    let until = Instant::now() + most;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browser_prints_first_then_libreoffice_then_the_rest() {
        let only = |names: &'static [&'static str]| move |n: &str| names.contains(&n).then(|| PathBuf::from(format!("/usr/bin/{n}")));
        assert_eq!(printer(only(&["soffice", "chromium"])), Some(Printer::Browser("/usr/bin/chromium".into())));
        assert_eq!(printer(only(&["soffice", "weasyprint"])), Some(Printer::LibreOffice("/usr/bin/soffice".into())));
        assert_eq!(printer(only(&["wkhtmltopdf", "weasyprint"])), Some(Printer::Weasy("/usr/bin/weasyprint".into())));
        assert_eq!(printer(only(&[])), None);
    }

    #[test]
    fn a_snaps_browser_works_in_its_own_folder() {
        let home = Path::new("/home/a");
        let snap = Printer::Browser("/snap/bin/chromium".into());
        assert_eq!(work_dir(&snap, Some(home), 7), PathBuf::from("/home/a/snap/chromium/common/kolada-print"));
        let plain = Printer::Browser("/usr/bin/chromium".into());
        assert_eq!(work_dir(&plain, Some(home), 7), std::env::temp_dir().join("kolada-print-7"));
        let c = command(&plain, Path::new("/t"), Path::new("/t/note.html"), Path::new("/t/note.pdf"));
        let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&"--print-to-pdf=/t/note.pdf".to_string()) && args.contains(&"file:///t/note.html".to_string()), "{args:?}");
        assert!(args.iter().any(|a| a.starts_with("--user-data-dir=/t/")), "a profile of its own, not the person's");
    }

    /// Prints a page with what this machine has (skipped where it has none).
    #[test]
    fn a_page_is_printed_to_a_pdf_with_what_the_desktop_has() {
        let find = |n: &str| super::super::services::find_tool(n, std::env::var_os("PATH"), None);
        if printer(find).is_none() {
            return;
        }
        let out = std::env::temp_dir().join(format!("kolada-print-test-{}.pdf", std::process::id()));
        let page = "<!doctype html><html><head><meta charset=utf-8><title>Plan</title></head><body><h1>Plan</h1><p>The quick brown fox.</p></body></html>";
        print(page, &out).expect("printed");
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"%PDF-"), "a PDF");
        let _ = std::fs::remove_file(out);
    }
}
