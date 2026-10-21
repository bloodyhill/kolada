//! What the model asks of the desktop outside the app: a page or a folder
//! opened (`xdg-open`), files chosen and a place to save (the desktop's own
//! chooser through its portal, `portal.rs`; else `zenity`, else KDE's
//! `kdialog`), a command started in a terminal. Each is the desktop's own;
//! none is needed at launch, and the choosers are asked from a thread.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kolada_app::model::share::Export;
use kolada_app::model::transfer::Pick;

/// Opens `what` (a web address, a mail link, a file or a folder) with the
/// desktop's own choice of app. Whether it could be started.
pub fn open(what: &str) -> bool {
    Command::new("xdg-open").arg(what).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

/// The arguments `zenity` gets to choose files, a folder, or the vault.
pub fn pick_args(pick: Pick) -> Vec<String> {
    let mut a = vec!["--file-selection".to_string()];
    match pick {
        Pick::Files => a.extend(["--multiple", "--separator=\n", "--title=Import Notes"].map(String::from)),
        Pick::Folder => a.extend(["--directory", "--title=Import a Folder of Notes"].map(String::from)),
        Pick::Vault => a.extend(["--directory", "--title=Open a Folder of Notes"].map(String::from)),
    }
    a
}

/// The arguments `zenity` gets to ask where the note goes, as `kind`.
pub fn save_args(name: &str, kind: Export, dir: &Path) -> Vec<String> {
    vec![
        "--file-selection".into(),
        "--save".into(),
        "--confirm-overwrite".into(),
        "--title=Export Note".into(),
        format!("--filename={}", dir.join(format!("{name}.{}", kind.ext())).display()),
    ]
}

/// What a chooser said: the paths, one a line; none when it was cancelled.
pub fn chosen(out: &str) -> Vec<PathBuf> {
    out.lines().map(str::trim_end).filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

/// The arguments `kdialog` gets for the same.
pub fn kdialog_pick_args(pick: Pick) -> Vec<String> {
    match pick {
        Pick::Files => ["--getopenfilename", ".", "--multiple", "--separate-output", "--title", "Import Notes"].map(String::from).to_vec(),
        Pick::Folder => ["--getexistingdirectory", ".", "--title", "Import a Folder of Notes"].map(String::from).to_vec(),
        Pick::Vault => ["--getexistingdirectory", ".", "--title", "Open a Folder of Notes"].map(String::from).to_vec(),
    }
}

/// A chooser program run with `args`, and what was chosen. None when the
/// desktop has no such program, empty when the person cancelled.
fn chooser(program: &str, args: &[String]) -> Option<Vec<PathBuf>> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    Some(if out.status.success() { chosen(&String::from_utf8_lossy(&out.stdout)) } else { Vec::new() })
}

/// The chooser's title for `pick`.
fn pick_title(pick: Pick) -> &'static str {
    match pick {
        Pick::Files => "Import Notes",
        Pick::Folder => "Import a Folder of Notes",
        Pick::Vault => "Open a Folder of Notes",
    }
}

/// Files or a folder chosen, for an import or the vault; `parent` is the
/// window it belongs to, for the portal. None when the desktop has no
/// chooser at all.
pub fn pick(what: Pick, parent: &str) -> Option<Vec<PathBuf>> {
    let ask = super::portal::Ask::Open { title: pick_title(what).to_string(), multiple: what == Pick::Files, directory: what != Pick::Files };
    if let Ok(chosen) = super::portal::choose(&ask, parent) {
        return Some(chosen);
    }
    chooser("zenity", &pick_args(what)).or_else(|| chooser("kdialog", &kdialog_pick_args(what)))
}

/// Where exports start: the desktop's Documents folder.
fn documents() -> PathBuf {
    kolada_core::default_vault_dir().and_then(|v| v.parent().map(Path::to_path_buf)).filter(|d| d.is_dir()).unwrap_or_else(std::env::temp_dir)
}

/// Where to save the note exported as `kind`, its extension the kind's own.
pub fn save_place(name: &str, kind: Export, parent: &str) -> Option<Option<PathBuf>> {
    let dir = documents();
    let file = format!("{name}.{}", kind.ext());
    let ask = super::portal::Ask::Save { title: "Export Note".to_string(), name: file.clone(), folder: dir.clone() };
    let place = match super::portal::choose(&ask, parent) {
        Ok(chosen) => chosen.into_iter().next(),
        Err(_) => chooser("zenity", &save_args(name, kind, &dir))
            .or_else(|| chooser("kdialog", &["--getsavefilename".to_string(), dir.join(&file).display().to_string(), "--title".to_string(), "Export Note".to_string()]))?
            .into_iter()
            .next(),
    };
    Some(place.map(|p| if p.extension().is_some_and(|e| e.eq_ignore_ascii_case(kind.ext())) { p } else { p.with_extension(kind.ext()) }))
}

/// `s` in single quotes for the shell.
pub fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `command` run in `dir` in the desktop's terminal (Debian's and Ubuntu's
/// `x-terminal-emulator`, else GNOME's or KDE's). Whether one started.
pub fn in_terminal(dir: &Path, command: &str) -> bool {
    let script = format!("cd {} && {command}; exec \"${{SHELL:-sh}}\"", quoted(&dir.to_string_lossy()));
    for (term, run) in [("x-terminal-emulator", "-e"), ("gnome-terminal", "--"), ("konsole", "-e"), ("xterm", "-e")] {
        if Command::new(term).arg(run).arg("sh").arg("-c").arg(&script).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choosers_are_asked_for_what_the_model_wants_and_read_back() {
        assert!(pick_args(Pick::Files).contains(&"--multiple".to_string()));
        assert!(pick_args(Pick::Vault).contains(&"--directory".to_string()));
        assert!(!pick_args(Pick::Files).contains(&"--directory".to_string()));
        assert!(kdialog_pick_args(Pick::Files).contains(&"--multiple".to_string()));
        assert_eq!(kdialog_pick_args(Pick::Vault)[0], "--getexistingdirectory");
        let save = save_args("Plan", Export::Markdown, Path::new("/home/a/Documents"));
        assert!(save.contains(&"--save".to_string()) && save.contains(&"--filename=/home/a/Documents/Plan.md".to_string()), "{save:?}");
        assert_eq!(chosen("/a/One.md\n/a/Two words.md\n"), vec![PathBuf::from("/a/One.md"), PathBuf::from("/a/Two words.md")]);
        assert!(chosen("").is_empty(), "cancelled: nothing");
    }

    #[test]
    fn folders_are_quoted_for_the_shell_whatever_their_names() {
        assert_eq!(quoted("/home/a/My Notes"), "'/home/a/My Notes'");
        assert_eq!(quoted("it's"), "'it'\\''s'");
        assert_eq!(quoted("\u{645}\u{644}\u{627}\u{62D}\u{638}\u{627}\u{62A}"), "'\u{645}\u{644}\u{627}\u{62D}\u{638}\u{627}\u{62A}'");
    }
}
