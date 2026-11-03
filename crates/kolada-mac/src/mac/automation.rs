//! Other apps asked for what they hold, for the model's `Automation`
//! service: a script (JavaScript for Automation) run by the system's
//! `osascript`. The first time Kolada asks Notes, macOS asks the user
//! whether it may: the bundle's `NSAppleEventsUsageDescription` says why,
//! and a signed build carries the `com.apple.security.automation.apple-events`
//! entitlement (see `tools/mac-bundle.sh`, `tools/mac-release.sh`).

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const OSASCRIPT: &str = "/usr/bin/osascript";
/// How long a script may go without writing anything into its folder
/// before it is ended: at first, room for the user to answer macOS's
/// question; after, between one note and the next. A large library takes
/// as long as it takes; only an app that stops answering (an iCloud sign-in
/// sheet in Notes) ends it, and what it wrote by then is kept.
const FIRST: Duration = Duration::from_secs(3 * 60);
const IDLE: Duration = Duration::from_secs(60);

pub struct MacAutomation;

impl kolada_app::services::Automation for MacAutomation {
    fn available(&self) -> bool {
        Path::new(OSASCRIPT).is_file()
    }

    /// Runs `script` until it ends, or until it has written nothing into
    /// its folder (the first argument) for a while. What `osascript`
    /// complains of comes back as it is: the model tells a refusal (-1743)
    /// from a fault.
    fn run(&self, script: &str, args: &[&Path]) -> Result<(), String> {
        run_within(script, args, FIRST, IDLE)
    }
}

/// How much is in `dir`: a script's progress.
fn entries(dir: Option<&&Path>) -> usize {
    dir.and_then(|d| std::fs::read_dir(d).ok()).map_or(0, |d| d.count())
}

fn run_within(script: &str, args: &[&Path], first: Duration, idle: Duration) -> Result<(), String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let file = std::env::temp_dir().join(format!("kolada-script-{}-{nanos}.js", std::process::id()));
    std::fs::write(&file, script).map_err(|e| e.to_string())?;
    let child = Command::new(OSASCRIPT).arg("-l").arg("JavaScript").arg(&file).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&file);
            return Err(e.to_string());
        }
    };
    // Its complaint, read as it comes so a long one never stalls it.
    let mut stderr = child.stderr.take();
    let said = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    // The clock starts again whenever the folder gains something.
    let (mut seen, mut since, mut wait) = (entries(args.first()), Instant::now(), first);
    let mut polls = 0u32;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                std::thread::sleep(Duration::from_millis(100));
                polls += 1;
                if polls % 5 == 0 {
                    let now = entries(args.first());
                    if now != seen {
                        (seen, since, wait) = (now, Instant::now(), idle);
                    }
                }
                if since.elapsed() < wait {
                    continue;
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let _ = std::fs::remove_file(&file);
    let said = said.join().unwrap_or_default();
    match status {
        Some(s) if s.success() => Ok(()),
        Some(_) => Err(said.trim().to_string()),
        None => Err("it stopped answering".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_app::services::Automation;

    /// A script that asks no app anything: it only writes into the folder
    /// it is given, as the Notes script does. Notes is never touched.
    #[test]
    fn a_script_runs_with_its_folder_and_a_failure_says_why() {
        let dir = std::env::temp_dir().join(format!("kolada-mac-automation-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = MacAutomation;
        assert!(a.available());
        let write = "function run(argv) { ObjC.import('Foundation'); $('written').writeToFileAtomicallyEncodingError(argv[0] + '/done.txt', true, $.NSUTF8StringEncoding, null); }";
        assert_eq!(a.run(write, &[&dir]), Ok(()));
        assert_eq!(std::fs::read_to_string(dir.join("done.txt")).unwrap(), "written");
        let fails = "function run(argv) { throw new Error('no such thing'); }";
        assert!(matches!(a.run(fails, &[&dir]), Err(why) if why.contains("no such thing")));
        // One that stops writing is ended a while after its last file,
        // what it wrote kept.
        let stalls = "function run(argv) { ObjC.import('Foundation'); for (var i = 1; i <= 2; i++) { $('n').writeToFileAtomicallyEncodingError(argv[0] + '/note-' + i + '.txt', true, $.NSUTF8StringEncoding, null); delay(0.5); } delay(60); }";
        let start = Instant::now();
        assert_eq!(run_within(stalls, &[&dir], Duration::from_secs(2), Duration::from_secs(2)), Err("it stopped answering".to_string()));
        assert!(start.elapsed() < Duration::from_secs(15));
        assert!(dir.join("note-2.txt").is_file());
        // One that keeps writing runs past the quiet spell it is allowed.
        let slow = "function run(argv) { ObjC.import('Foundation'); for (var i = 1; i <= 4; i++) { delay(1.2); $('n').writeToFileAtomicallyEncodingError(argv[0] + '/slow-' + i + '.txt', true, $.NSUTF8StringEncoding, null); } }";
        let start = Instant::now();
        assert_eq!(run_within(slow, &[&dir], Duration::from_secs(2), Duration::from_secs(2)), Ok(()));
        assert!(start.elapsed() > Duration::from_secs(4), "it ran longer than any one quiet spell");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
