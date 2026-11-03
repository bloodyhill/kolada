//! Apple Notes' notes brought into the vault, on the Mac: the welcome
//! screen's "Apple Notes" tile (`offers_apple_notes`, `import_apple_notes`).
//! Notes is asked for its notes through the shell's automation (macOS asks
//! the user once whether Kolada may control Notes), on a thread; they are
//! written into "From Apple Notes" at the vault's top, in Notes' folders,
//! dated as Notes has them (`kolada_convert::applenotes`). Locked notes stay
//! in Notes and are counted, as are attachments other than pictures.

use std::path::PathBuf;

use kolada_convert::applenotes;

use super::arrive::Arrival;
use super::*;

/// Where bringing Apple Notes in has got, for the welcome screen's tile.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum AppleNotes {
    #[default]
    Idle,
    /// Notes is being asked (the user may be answering macOS's question).
    Running,
    Done(Brought),
    /// The user (or macOS) did not let Kolada control Notes.
    Refused,
    Failed(String),
}

/// What came in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Brought {
    /// The notes written, in Notes' order.
    pub notes: Vec<PathBuf>,
    /// Left in Notes: locked notes, notes Notes would not give, and
    /// attachments other than pictures.
    pub locked: usize,
    pub unreadable: usize,
    pub attachments_left: usize,
    /// Notes stopped part way (it stopped answering, or quit): the notes
    /// that came are kept, and `not_reached` stayed in Notes.
    pub stopped: bool,
    pub not_reached: usize,
}

impl Brought {
    /// In a few words, as the whisper says it.
    pub fn words(&self) -> String {
        let n = self.notes.len();
        let mut said = match n {
            0 => "No notes came from Apple Notes".to_string(),
            1 => "Brought in 1 note from Apple Notes".to_string(),
            n => format!("Brought in {n} notes from Apple Notes"),
        };
        if self.stopped {
            said.push_str(" before Notes stopped answering");
        }
        let mut left = Vec::new();
        if self.not_reached > 0 {
            left.push(if self.not_reached == 1 { "1 more note".to_string() } else { format!("{} more notes", self.not_reached) });
        }
        if self.locked > 0 {
            left.push(if self.locked == 1 { "1 locked note".to_string() } else { format!("{} locked notes", self.locked) });
        }
        if self.unreadable > 0 {
            left.push(if self.unreadable == 1 { "1 note Notes would not give".to_string() } else { format!("{} notes Notes would not give", self.unreadable) });
        }
        if self.attachments_left > 0 {
            left.push(if self.attachments_left == 1 { "1 attachment".to_string() } else { format!("{} attachments", self.attachments_left) });
        }
        if !left.is_empty() {
            said.push_str("; ");
            said.push_str(&left.join(", "));
            said.push_str(" stayed in Notes");
        }
        said
    }
}

/// Asks Notes for its notes through `automation` and writes them into
/// `vault`, their pictures into `pictures`.
pub(super) fn bring(automation: &dyn crate::services::Automation, vault: &Path, pictures: &Path) -> AppleNotes {
    let dir = std::env::temp_dir().join(format!("kolada-apple-notes-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())));
    if std::fs::create_dir_all(&dir).is_err() {
        return AppleNotes::Failed("Kolada could not make a folder to read into".to_string());
    }
    // However the script ended (Notes may stop answering, or quit, part
    // way), the notes that came are brought in; only a refusal brings none.
    let ran = automation.run(applenotes::SCRIPT, &[&dir]);
    if ran.as_ref().is_err_and(|why| applenotes::refused(why)) {
        let _ = std::fs::remove_dir_all(&dir);
        return AppleNotes::Refused;
    }
    // Each note read, converted and written before the next is read: a
    // library of photos is never held at once.
    let mut writer = kolada_convert::import::Writer::new(applenotes::FOLDER, vault, pictures);
    let (mut notes, mut unsaved) = (Vec::new(), 0);
    let read = applenotes::each(&dir, |e| match writer.write(&e) {
        Some(at) => notes.push(at),
        None => unsaved += 1,
    });
    let state = match (ran, read) {
        (Err(_), Err(_)) => AppleNotes::Failed("Notes did not answer".to_string()),
        (Ok(()), Err(e)) => AppleNotes::Failed(e.to_string()),
        (ran, Ok(t)) => AppleNotes::Done(Brought {
            notes,
            locked: t.locked,
            unreadable: t.unreadable + unsaved,
            attachments_left: t.attachments_left,
            // Ended early and yet whole on paper: say it stopped all the same.
            stopped: t.stopped || ran.is_err(),
            not_reached: t.not_reached,
        }),
    };
    // What Notes gave is the user's: not left behind in the temporary folder.
    let _ = std::fs::remove_dir_all(&dir);
    state
}

impl Model {
    /// Whether the welcome screen offers Apple Notes: where the shell can
    /// ask other apps (the Mac).
    pub fn offers_apple_notes(&self) -> bool {
        self.services.automation.available()
    }

    /// Where bringing Apple Notes in has got.
    pub fn apple_notes(&self) -> &AppleNotes {
        &self.arrivals.apple_notes
    }

    /// The "Apple Notes" tile: Notes' notes into "From Apple Notes", on a
    /// thread. False where it is not offered, or already running.
    pub fn import_apple_notes(&mut self) -> bool {
        if !self.offers_apple_notes() || self.arrivals.apple_notes == AppleNotes::Running {
            return false;
        }
        self.arrivals.apple_notes = AppleNotes::Running;
        let (vault, pictures) = (self.vault.clone(), self.attachment_path());
        let automation = self.services.automation.clone();
        let send = self.arrivals.sender();
        std::thread::spawn(move || send(Arrival::AppleNotes(bring(&*automation, &vault, &pictures))));
        true
    }

    /// Notes answered: the tree shows what came, the first opens, and the
    /// whisper says what came and what stayed.
    pub(super) fn apple_notes_in(&mut self, state: AppleNotes) -> bool {
        match &state {
            AppleNotes::Done(b) => {
                self.imported(b.notes.first().cloned());
                self.whisper(&b.words());
            }
            AppleNotes::Refused => self.whisper("Kolada may not read Notes: allow it in System Settings, Privacy & Security, Automation"),
            AppleNotes::Failed(why) => self.whisper(&format!("Could not bring in Apple Notes: {why}")),
            AppleNotes::Idle | AppleNotes::Running => {}
        }
        self.apple_notes_on_welcome(&state);
        self.arrivals.apple_notes = state;
        true
    }
}

#[cfg(test)]
mod tests {
    /// Apple Notes runs one at a time here: each run's temporary folder is
    /// named for this process, so a test that checks none is left sees
    /// only its own.
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    use super::*;
    use crate::services::{Automation, Services};
    use std::sync::{Arc, Mutex};

    const US: &str = "\u{1F}";

    /// Notes as a test has it: canned replies written where the script
    /// would write them, never the real app.
    struct Canned {
        notes: Vec<String>,
        locked: usize,
        refuse: bool,
        asked: Mutex<usize>,
        /// Notes stops answering after this many notes, of all it has.
        stalls: Option<usize>,
    }

    impl Automation for Canned {
        fn available(&self) -> bool {
            true
        }
        fn run(&self, script: &str, args: &[&Path]) -> Result<(), String> {
            *self.asked.lock().unwrap() += 1;
            assert_eq!(script, applenotes::SCRIPT);
            if self.refuse {
                return Err("execution error: Not authorized to send Apple events to Notes. (-1743)".to_string());
            }
            std::fs::write(args[0].join("total.txt"), ["2", &self.notes.len().to_string(), &self.locked.to_string(), "1"].join(US)).unwrap();
            for (i, n) in self.notes.iter().enumerate().take(self.stalls.unwrap_or(usize::MAX)) {
                std::fs::write(args[0].join(format!("note-{}.txt", i + 1)), n).unwrap();
            }
            if self.stalls.is_some() {
                return Err("Notes stopped answering".to_string());
            }
            std::fs::write(args[0].join("done.txt"), ["2", &self.notes.len().to_string(), &self.locked.to_string(), "0"].join(US)).unwrap();
            Ok(())
        }
    }

    fn record(folders: &str, name: &str, attachments: usize, body: &str) -> String {
        ["2", "iCloud", folders, name, "1600000000000", "1700000000000", &attachments.to_string(), body].join(US)
    }

    fn vault(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kolada-app-applenotes-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Welcome.md"), "# Welcome\n").unwrap();
        dir
    }

    fn model(dir: &Path, canned: Canned) -> (Model, Arc<Canned>) {
        let canned = Arc::new(canned);
        let m = Model::with_services(Services { automation: canned.clone(), ..Services::none() }, dir.to_path_buf(), true);
        (m, canned)
    }

    #[test]
    fn apple_notes_come_in_to_their_folders_and_the_whisper_says_what_stayed() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let dir = vault("in");
        let notes = vec![
            record("Travel", "Trip to Lisbon", 2, "<div>Trip to Lisbon</div><div>Flights at 09:40</div>"),
            record("", "Ideas", 0, "<div>Ideas</div><ul class=\"checklist\"><li class=\"checked\">one</li><li>two</li></ul>"),
        ];
        let (mut m, _) = model(&dir, Canned { notes, locked: 2, refuse: false, asked: Mutex::new(0), stalls: None });
        assert!(m.offers_apple_notes());
        assert!(m.import_apple_notes());
        assert_eq!(*m.apple_notes(), AppleNotes::Running);
        assert!(!m.import_apple_notes(), "once at a time");
        m.wait_for_arrivals();
        let AppleNotes::Done(b) = m.apple_notes().clone() else { panic!("{:?}", m.apple_notes()) };
        let rel: Vec<String> = b.notes.iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(rel, ["From Apple Notes/Travel/Trip to Lisbon.md", "From Apple Notes/Ideas.md"]);
        assert_eq!(std::fs::read_to_string(dir.join("From Apple Notes/Ideas.md")).unwrap(), "# Ideas\n\n- [x] one\n- [ ] two\n");
        assert_eq!(b.words(), "Brought in 2 notes from Apple Notes; 2 locked notes, 2 attachments stayed in Notes");
        assert_eq!(m.session.note, "From Apple Notes/Travel/Trip to Lisbon.md", "the first opens");
        let leftover = std::fs::read_dir(std::env::temp_dir()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(&format!("kolada-apple-notes-{}-", std::process::id()))).count();
        assert_eq!(leftover, 0, "what Notes gave is not left in the temporary folder");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refusal_says_where_to_allow_it_and_nothing_is_written() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let dir = vault("refused");
        let (mut m, canned) = model(&dir, Canned { notes: vec![record("", "Secret", 0, "<div>x</div>")], locked: 0, refuse: true, asked: Mutex::new(0), stalls: None });
        m.import_apple_notes();
        m.wait_for_arrivals();
        assert_eq!(*m.apple_notes(), AppleNotes::Refused);
        assert!(!dir.join("From Apple Notes").exists());
        assert_eq!(*canned.asked.lock().unwrap(), 1);
        // Asked again, it asks Notes again (the user may have allowed it).
        assert!(m.import_apple_notes());
        m.wait_for_arrivals();
        assert_eq!(*canned.asked.lock().unwrap(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn when_notes_stops_part_way_the_notes_that_came_are_kept_and_the_rest_counted() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let dir = vault("stalled");
        let notes = (1..=5).map(|i| record("Work", &format!("Note {i}"), 0, &format!("<div>Note {i}</div><div>Body</div>"))).collect();
        let (mut m, _) = model(&dir, Canned { notes, locked: 0, refuse: false, asked: Mutex::new(0), stalls: Some(2) });
        m.import_apple_notes();
        m.wait_for_arrivals();
        let AppleNotes::Done(b) = m.apple_notes().clone() else { panic!("{:?}", m.apple_notes()) };
        assert_eq!(b.notes.len(), 2, "the two that came are brought in");
        assert!(b.stopped);
        assert_eq!(b.not_reached, 3);
        assert_eq!(b.words(), "Brought in 2 notes from Apple Notes before Notes stopped answering; 3 more notes stayed in Notes");
        assert!(dir.join("From Apple Notes/Work/Note 2.md").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn when_notes_stops_before_any_note_it_says_so() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let dir = vault("silent");
        let (mut m, _) = model(&dir, Canned { notes: vec![record("", "One", 0, "<div>One</div>")], locked: 0, refuse: false, asked: Mutex::new(0), stalls: Some(0) });
        m.import_apple_notes();
        m.wait_for_arrivals();
        assert_eq!(*m.apple_notes(), AppleNotes::Failed("Notes did not answer".to_string()));
        assert!(!dir.join("From Apple Notes").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_offered_where_the_shell_cannot_ask_other_apps() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let dir = vault("none");
        let mut m = Model::with_services(Services::none(), dir.clone(), true);
        assert!(!m.offers_apple_notes());
        assert!(!m.import_apple_notes());
        assert_eq!(*m.apple_notes(), AppleNotes::Idle);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_words_count_what_stayed() {
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let b = |n: usize, locked, unreadable, att| Brought { notes: vec![PathBuf::new(); n], locked, unreadable, attachments_left: att, ..Brought::default() };
        assert_eq!(b(1, 0, 0, 0).words(), "Brought in 1 note from Apple Notes");
        assert_eq!(b(0, 1, 0, 0).words(), "No notes came from Apple Notes; 1 locked note stayed in Notes");
        assert_eq!(b(3, 0, 1, 1).words(), "Brought in 3 notes from Apple Notes; 1 note Notes would not give, 1 attachment stayed in Notes");
    }
}
