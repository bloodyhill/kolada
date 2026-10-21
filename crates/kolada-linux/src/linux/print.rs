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

/// The printers found by `find` (a command's path by its name), best
/// first: one browser, then LibreOffice, WeasyPrint, wkhtmltopdf.
pub fn printers(find: impl Fn(&str) -> Option<PathBuf>) -> Vec<Printer> {
    [
        BROWSERS.iter().find_map(|b| find(b).map(Printer::Browser)),
        ["soffice", "libreoffice"].iter().find_map(|n| find(n).map(Printer::LibreOffice)),
        find("weasyprint").map(Printer::Weasy),
        find("wkhtmltopdf").map(Printer::Wkhtml),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// How many ways `p` is tried: a browser in its sandbox, then without it.
/// Its sandbox cannot start where unprivileged user namespaces are
/// restricted (Ubuntu 24.04 does so for a browser with no AppArmor profile
/// of its own: one unpacked by hand, or put in /usr/local), nor as root;
/// the page is the note's own, and scripts are off then.
fn tries(p: &Printer) -> usize {
    if matches!(p, Printer::Browser(_)) { 2 } else { 1 }
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

/// The command that prints `html` (in `dir`) to `pdf` (in `dir` too);
/// `unsandboxed`: a browser's second try (see `tries`).
pub fn command(p: &Printer, dir: &Path, html: &Path, pdf: &Path, unsandboxed: bool) -> Command {
    let mut c;
    match p {
        Printer::Browser(b) => {
            c = Command::new(b);
            c.args(["--headless", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--no-pdf-header-footer"]);
            if unsandboxed {
                c.args(["--no-sandbox", "--blink-settings=scriptEnabled=false"]);
            }
            c.arg(format!("--user-data-dir={}", dir.join("profile").display()))
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
    print_with(&printers(find), page, out)
}

/// Prints `page` to `out` with the first of `printers` that makes a PDF
/// (one that cannot run here passes it on to the next).
pub fn print_with(printers: &[Printer], page: &str, out: &Path) -> Result<(), &'static str> {
    if printers.is_empty() {
        return Err("PDF needs Chromium, Chrome or LibreOffice");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    for printer in printers {
        let dir = work_dir(printer, home.as_deref(), std::process::id());
        let _ = std::fs::remove_dir_all(&dir);
        if std::fs::create_dir_all(&dir).is_err() {
            continue;
        }
        let (html, pdf) = (dir.join("note.html"), dir.join("note.pdf"));
        let written = std::fs::write(&html, page).is_ok();
        let done = written
            && (0..tries(printer)).any(|t| {
                let _ = std::fs::remove_file(&pdf);
                run(command(printer, &dir, &html, &pdf, t > 0), Duration::from_secs(90)) && pdf.is_file()
            })
            && std::fs::copy(&pdf, out).is_ok();
        let _ = std::fs::remove_dir_all(&dir);
        if done {
            return Ok(());
        }
    }
    Err("Could not make the PDF")
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
        let first = |names: &'static [&'static str]| printers(only(names)).into_iter().next();
        assert_eq!(first(&["soffice", "chromium"]), Some(Printer::Browser("/usr/bin/chromium".into())));
        assert_eq!(first(&["soffice", "weasyprint"]), Some(Printer::LibreOffice("/usr/bin/soffice".into())));
        assert_eq!(first(&["wkhtmltopdf", "weasyprint"]), Some(Printer::Weasy("/usr/bin/weasyprint".into())));
        assert_eq!(first(&[]), None);
        assert_eq!(printers(only(&["weasyprint", "google-chrome", "soffice"])), [Printer::Browser("/usr/bin/google-chrome".into()), Printer::LibreOffice("/usr/bin/soffice".into()), Printer::Weasy("/usr/bin/weasyprint".into())], "each one found, best first");
    }

    #[test]
    fn a_snaps_browser_works_in_its_own_folder() {
        let home = Path::new("/home/a");
        let snap = Printer::Browser("/snap/bin/chromium".into());
        assert_eq!(work_dir(&snap, Some(home), 7), PathBuf::from("/home/a/snap/chromium/common/kolada-print"));
        let plain = Printer::Browser("/usr/bin/chromium".into());
        assert_eq!(work_dir(&plain, Some(home), 7), std::env::temp_dir().join("kolada-print-7"));
        let args = |c: Command| -> Vec<String> { c.get_args().map(|a| a.to_string_lossy().into_owned()).collect() };
        let a = args(command(&plain, Path::new("/t"), Path::new("/t/note.html"), Path::new("/t/note.pdf"), false));
        assert!(a.contains(&"--print-to-pdf=/t/note.pdf".to_string()) && a.contains(&"file:///t/note.html".to_string()), "{a:?}");
        assert!(a.iter().any(|a| a.starts_with("--user-data-dir=/t/")), "a profile of its own, not the person's");
        assert!(!a.iter().any(|a| a == "--no-sandbox"), "sandboxed first");
        let b = args(command(&plain, Path::new("/t"), Path::new("/t/note.html"), Path::new("/t/note.pdf"), true));
        assert!(b.contains(&"--no-sandbox".to_string()) && b.contains(&"--blink-settings=scriptEnabled=false".to_string()), "{b:?}");
    }

    /// A program standing in for a printer: it makes `%PDF-` at the path it
    /// is given (after `--print-to-pdf=`, or its last argument) only when
    /// its arguments hold `needs` ("" for always), else fails.
    fn fake(dir: &Path, name: &str, needs: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        let script = format!(
            "#!/bin/sh\nok=0; out=\"\"\nfor a in \"$@\"; do case \"$a\" in --print-to-pdf=*) out=\"${{a#--print-to-pdf=}}\";; esac; [ -z \"{needs}\" ] || [ \"$a\" = \"{needs}\" ] && ok=1; last=\"$a\"; done\n[ $ok = 1 ] || exit 1\n[ -n \"$out\" ] || out=\"$last\"\nprintf '%%PDF-1.4 {name}' > \"$out\"\n"
        );
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn a_browser_whose_sandbox_cannot_start_prints_without_it_and_one_that_cannot_run_passes_it_on() {
        let dir = std::env::temp_dir().join(format!("kolada-print-fakes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out.pdf");
        let page = "<!doctype html><h1>Plan</h1>";
        // Ubuntu 24.04's restricted namespaces, or root: only --no-sandbox prints.
        let browser = Printer::Browser(fake(&dir, "chromium", "--no-sandbox"));
        assert_eq!(print_with(&[browser], page, &out), Ok(()));
        assert_eq!(std::fs::read(&out).unwrap(), b"%PDF-1.4 chromium");
        // A browser that cannot print at all: the next printer does.
        let broken = Printer::Browser(fake(&dir, "google-chrome", "--never"));
        let weasy = Printer::Weasy(fake(&dir, "weasyprint", ""));
        assert_eq!(print_with(&[broken.clone(), weasy], page, &out), Ok(()));
        assert_eq!(std::fs::read(&out).unwrap(), b"%PDF-1.4 weasyprint");
        assert_eq!(print_with(&[broken], page, &out), Err("Could not make the PDF"));
        assert_eq!(print_with(&[], page, &out), Err("PDF needs Chromium, Chrome or LibreOffice"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Whether the program behind `p` runs here at all (its version said).
    fn runs(p: &Printer) -> bool {
        let (Printer::Browser(x) | Printer::LibreOffice(x) | Printer::Weasy(x) | Printer::Wkhtml(x)) = p;
        Command::new(x).arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }

    /// Prints a page with what this machine has: skipped where it has no
    /// printer, or none that runs at all; one that runs must print.
    #[test]
    fn a_page_is_printed_to_a_pdf_with_what_the_desktop_has() {
        let find = |n: &str| super::super::services::find_tool(n, std::env::var_os("PATH"), None);
        let found: Vec<Printer> = printers(find).into_iter().filter(runs).collect();
        if found.is_empty() {
            eprintln!("no printer that runs here: skipped");
            return;
        }
        let out = std::env::temp_dir().join(format!("kolada-print-test-{}.pdf", std::process::id()));
        let page = "<!doctype html><html><head><meta charset=utf-8><title>Plan</title></head><body><h1>Plan</h1><p>The quick brown fox.</p></body></html>";
        print_with(&found, page, &out).unwrap_or_else(|e| panic!("{e}, with {found:?}"));
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"%PDF-"), "a PDF");
        let _ = std::fs::remove_file(out);
    }
}
