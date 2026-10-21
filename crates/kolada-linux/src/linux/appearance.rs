//! Whether the desktop is dark, for the System theme, and when that
//! changes. X11 does not say, and asking the desktop costs a process, so it
//! is asked after the first frame and what it said is kept in the app's data
//! for the next launch's first frame (one small file read). Then the
//! desktop is followed: the XDG desktop portal's `color-scheme` (GNOME, KDE
//! and the desktops with a portal), watched through `gdbus monitor`; where
//! there is no portal, GNOME's own setting through `gsettings monitor`.
//! `GTK_THEME=Name:dark` counts too, and is not followed (it cannot change).

use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

const FILE: &str = "appearance";

const PORTAL: &[&str] = &["--session", "--dest", "org.freedesktop.portal.Desktop", "--object-path", "/org/freedesktop/portal/desktop"];

/// What `gsettings get org.gnome.desktop.interface color-scheme` printed.
pub fn read_scheme(out: &str) -> Option<bool> {
    match out.trim().trim_matches('\'') {
        "prefer-dark" => Some(true),
        "prefer-light" | "default" => Some(false),
        _ => None,
    }
}

/// A line `gsettings monitor` printed (`color-scheme: 'prefer-dark'`).
pub fn read_monitored(line: &str) -> Option<bool> {
    read_scheme(line.strip_prefix("color-scheme:")?)
}

/// The portal's `color-scheme` in what `gdbus` printed for it (`(<uint32 1>,)`,
/// or `<<uint32 1>>` from portals that only have `Read`): 1 dark, 2 light,
/// 0 no preference (light, as GNOME's "default").
pub fn read_portal(out: &str) -> Option<bool> {
    let at = out.find("uint32 ")? + "uint32 ".len();
    match out[at..].chars().next()? {
        '1' => Some(true),
        '0' | '2' => Some(false),
        _ => None,
    }
}

/// A line `gdbus monitor` printed: the portal's color scheme changed.
pub fn portal_changed(line: &str) -> Option<bool> {
    let at = line.find("'org.freedesktop.appearance', 'color-scheme'")?;
    read_portal(&line[at..])
}

/// `GTK_THEME`'s say, when it is set (`Adwaita:dark`).
pub fn from_gtk_theme(value: &str) -> Option<bool> {
    let lower = value.to_ascii_lowercase();
    if lower.is_empty() { None } else { Some(lower.ends_with(":dark") || lower.ends_with("-dark")) }
}

fn gtk_theme() -> Option<bool> {
    std::env::var("GTK_THEME").ok().as_deref().and_then(from_gtk_theme)
}

/// What the last launch learned, at once.
pub fn kept(data: Option<&Path>) -> Option<bool> {
    if let Some(dark) = gtk_theme() {
        return Some(dark);
    }
    match std::fs::read(data?.join(FILE)).ok()?.first() {
        Some(b'd') => Some(true),
        Some(b'l') => Some(false),
        _ => None,
    }
}

fn keep(data: Option<&Path>, dark: bool) {
    if let Some(dir) = data {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(dir.join(FILE), if dark { "dark" } else { "light" });
    }
}

/// A desktop program's output, read quietly; None if it is missing or fails.
fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The portal's say now, if there is a portal.
fn ask_portal() -> Option<bool> {
    let mut read_one: Vec<&str> = vec!["call"];
    read_one.extend_from_slice(PORTAL);
    read_one.extend_from_slice(&["--method", "org.freedesktop.portal.Settings.ReadOne", "org.freedesktop.appearance", "color-scheme"]);
    let mut read = read_one.clone();
    let method = read.len() - 3;
    read[method] = "org.freedesktop.portal.Settings.Read";
    output("gdbus", &read_one).or_else(|| output("gdbus", &read)).as_deref().and_then(read_portal)
}

/// GNOME's own setting now.
fn ask_gnome() -> Option<bool> {
    output("gsettings", &["get", "org.gnome.desktop.interface", "color-scheme"]).as_deref().and_then(read_scheme)
}

/// A watcher started for the life of the app: it dies with it.
fn watcher(program: &str, args: &[&str]) -> Option<std::process::Child> {
    let mut c = Command::new(program);
    c.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    unsafe {
        c.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    c.spawn().ok()
}

/// Asks the desktop, then follows it: `told` hears the scheme now and at
/// every change, from a thread of its own (started after the first frame).
pub fn follow(data: Option<std::path::PathBuf>, told: impl Fn(bool) + Send + 'static) {
    std::thread::spawn(move || {
        if let Some(dark) = gtk_theme() {
            told(dark);
            return;
        }
        // The portal, where there is one; else GNOME's setting.
        let from_portal = ask_portal();
        let portal = from_portal.is_some();
        let first = from_portal.or_else(ask_gnome);
        if let Some(dark) = first {
            keep(data.as_deref(), dark);
            told(dark);
        }
        let child = if portal {
            let mut monitor: Vec<&str> = vec!["monitor"];
            monitor.extend_from_slice(PORTAL);
            watcher("gdbus", &monitor)
        } else {
            watcher("gsettings", &["monitor", "org.gnome.desktop.interface", "color-scheme"])
        };
        let Some(mut child) = child else { return };
        let Some(out) = child.stdout.take() else { return };
        let mut last = first;
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let seen = if portal { portal_changed(&line) } else { read_monitored(&line) };
            if let Some(dark) = seen
                && Some(dark) != last
            {
                last = Some(dark);
                keep(data.as_deref(), dark);
                told(dark);
            }
        }
        let _ = child.wait();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktops_colour_scheme_is_read_and_kept_for_the_next_launch() {
        assert_eq!(read_scheme("'prefer-dark'\n"), Some(true));
        assert_eq!(read_scheme("'default'\n"), Some(false));
        assert_eq!(read_scheme("'prefer-light'"), Some(false));
        assert_eq!(read_scheme("No such schema"), None);
        assert_eq!(from_gtk_theme("Adwaita:dark"), Some(true));
        assert_eq!(from_gtk_theme("Arc-Dark"), Some(true));
        assert_eq!(from_gtk_theme("Adwaita"), Some(false));
        assert_eq!(from_gtk_theme(""), None);
        let dir = std::env::temp_dir().join(format!("kolada-linux-appearance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        if std::env::var_os("GTK_THEME").is_none() {
            assert_eq!(kept(Some(&dir)), None, "nothing learned yet");
            keep(Some(&dir), false);
            assert_eq!(kept(Some(&dir)), Some(false));
            keep(Some(&dir), true);
            assert_eq!(kept(Some(&dir)), Some(true));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_portal_and_gnome_are_followed_as_they_change() {
        assert_eq!(read_portal("(<uint32 1>,)\n"), Some(true));
        assert_eq!(read_portal("(<<uint32 2>>,)\n"), Some(false), "an older portal's Read");
        assert_eq!(read_portal("(<uint32 0>,)"), Some(false), "no preference reads light");
        assert_eq!(read_portal("Error: GDBus.Error"), None);
        let changed = "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.freedesktop.appearance', 'color-scheme', <uint32 1>)";
        assert_eq!(portal_changed(changed), Some(true));
        assert_eq!(portal_changed("/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.gnome.desktop.interface', 'font-name', <'Cantarell 11'>)"), None, "another setting");
        assert_eq!(read_monitored("color-scheme: 'prefer-dark'"), Some(true));
        assert_eq!(read_monitored("color-scheme: 'default'"), Some(false));
        assert_eq!(read_monitored("gtk-theme: 'Adwaita'"), None);
    }
}
