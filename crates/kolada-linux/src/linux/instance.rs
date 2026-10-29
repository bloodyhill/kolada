//! One Kolada per user (per data folder, so a test instance keeps apart). A
//! second launch hands what it was asked to open to the one running, which
//! comes to the front, and exits at once. They meet at a socket in Linux's
//! abstract namespace (no file to leave behind), named for the data folder;
//! finding it costs one `connect`, before anything else is done.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// What a second launch asks of the first.
#[derive(Clone, Debug, PartialEq)]
pub enum Handed {
    /// Launched again with nothing named: come to the front.
    Show,
    /// A note or a document to open.
    Open(PathBuf),
    /// A folder: the vault to open.
    Vault(PathBuf),
    /// A new note (`--new`).
    New,
    /// The search (`--search`).
    Search,
    /// Beam's popup (`--beam`).
    Beam,
}

impl Handed {
    /// What a launch asks of the one running: a new note or the search,
    /// else what its command line named.
    pub fn asked(new: bool, search: bool, given: Option<&Path>) -> Handed {
        match (new, search) {
            (true, _) => Handed::New,
            (_, true) => Handed::Search,
            _ => Handed::of(given),
        }
    }

    /// The same, with Beam's popup asked for (`--beam`).
    pub fn asked_beam(beam: bool, new: bool, search: bool, given: Option<&Path>) -> Handed {
        if beam { Handed::Beam } else { Handed::asked(new, search, given) }
    }

    /// What the command line named, made absolute (the second launch's
    /// folder is not the first's).
    pub fn of(given: Option<&Path>) -> Handed {
        let Some(p) = given else { return Handed::Show };
        let p = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
        if p.is_dir() { Handed::Vault(p) } else { Handed::Open(p) }
    }

    fn line(&self) -> String {
        match self {
            Handed::Show => "show\n".to_string(),
            Handed::Open(p) => format!("open\t{}\n", p.display()),
            Handed::Vault(p) => format!("vault\t{}\n", p.display()),
            Handed::New => "new\n".to_string(),
            Handed::Search => "search\n".to_string(),
            Handed::Beam => "beam\n".to_string(),
        }
    }

    fn parse(line: &str) -> Option<Handed> {
        let line = line.trim_end_matches(['\r', '\n']);
        match line.split_once('\t') {
            None if line == "show" => Some(Handed::Show),
            None if line == "new" => Some(Handed::New),
            None if line == "search" => Some(Handed::Search),
            None if line == "beam" => Some(Handed::Beam),
            Some(("open", p)) if !p.is_empty() => Some(Handed::Open(PathBuf::from(p))),
            Some(("vault", p)) if !p.is_empty() => Some(Handed::Vault(PathBuf::from(p))),
            _ => None,
        }
    }
}

/// The socket's name for a data folder.
pub fn name(data: Option<&Path>) -> String {
    // FNV-1a of the folder: short, and the same for every launch.
    let key = data.map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    format!("kolada-{hash:016x}")
}

/// The outcome of looking for a running Kolada.
pub enum Claim {
    /// None was running: this is the one, and listens here.
    First(UnixListener),
    /// One was, and has been told.
    Handed,
    /// Could not tell (no socket): run alone.
    Alone,
}

fn address(name: &str) -> Option<SocketAddr> {
    SocketAddr::from_abstract_name(name.as_bytes()).ok()
}

fn tell(name: &str, what: &Handed) -> bool {
    let Some(addr) = address(name) else { return false };
    let Ok(mut s) = UnixStream::connect_addr(&addr) else { return false };
    s.write_all(what.line().as_bytes()).is_ok()
}

/// Tells a running Kolada `what`, or becomes the one that runs.
pub fn claim(name: &str, what: &Handed) -> Claim {
    if tell(name, what) {
        return Claim::Handed;
    }
    let Some(addr) = address(name) else { return Claim::Alone };
    match UnixListener::bind_addr(&addr) {
        Ok(l) => Claim::First(l),
        // Two launched at once, and the other bound first: tell it.
        Err(_) if tell(name, what) => Claim::Handed,
        Err(_) => Claim::Alone,
    }
}

/// Takes what later launches hand over, on a thread of its own, and passes
/// each to `got`.
pub fn listen(listener: UnixListener, got: impl Fn(Handed) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            // One line, from one of our own; a stray peer is not waited on.
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new(stream).take(8192).read_line(&mut line).is_ok()
                && let Some(h) = Handed::parse(&line)
            {
                got(h);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn a_second_launch_hands_its_file_to_the_first() {
        let name = format!("kolada-test-{}", std::process::id());
        let Claim::First(listener) = claim(&name, &Handed::Show) else { panic!("the first launch runs") };
        let (tx, rx) = mpsc::channel();
        listen(listener, move |h| {
            let _ = tx.send(h);
        });
        let note = PathBuf::from("/home/a/Notes/Plan.md");
        assert!(matches!(claim(&name, &Handed::Open(note.clone())), Claim::Handed), "a second launch finds the first");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Handed::Open(note));
        assert!(matches!(claim(&name, &Handed::Show), Claim::Handed));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Handed::Show);
    }

    #[test]
    fn each_data_folder_has_its_own_instance() {
        let a = name(Some(Path::new("/home/a/.local/share/Kolada")));
        assert_eq!(a, name(Some(Path::new("/home/a/.local/share/Kolada"))));
        assert_ne!(a, name(Some(Path::new("/home/a/.local/share/Kolada.test"))), "a test instance keeps apart");
        assert!(a.len() < 100, "fits a socket's name");
    }

    #[test]
    fn what_is_handed_is_read_back_and_nothing_else() {
        for h in [Handed::Show, Handed::Open("/a/b c.md".into()), Handed::Vault("/a/Notes".into()), Handed::New, Handed::Search] {
            assert_eq!(Handed::parse(&h.line()), Some(h));
        }
        assert_eq!(Handed::asked(true, false, Some(Path::new("/a/b.md"))), Handed::New, "the app list's New Note");
        assert_eq!(Handed::asked(false, true, None), Handed::Search);
        assert_eq!(Handed::asked(false, false, None), Handed::Show);
        assert_eq!(Handed::parse("open\t\n"), None);
        assert_eq!(Handed::parse("rm -rf\n"), None);
        let dir = std::env::temp_dir();
        assert_eq!(Handed::of(Some(&dir)), Handed::Vault(dir.clone()), "a folder is a vault");
        assert_eq!(Handed::of(None), Handed::Show);
        assert!(matches!(Handed::of(Some(Path::new("Plan.md"))), Handed::Open(p) if p.is_absolute()), "made absolute");
    }
}
