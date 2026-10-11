//! What iOS gives the model (`notsy_app::services`): the calendar and the
//! fonts so far; the rest start from `Services::none()` (no spell checker,
//! no OCR, no network, no OS trash: a note deleted stays in Notsy's own
//! trash, as iOS has no Trash) and are added as the shell grows.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use notsy_app::services::{Calendar, Fonts, Local, NoServices, Services};
use notsy_text::FontSet;

/// The services an iOS window's model gets.
pub fn ios() -> Services {
    Services { calendar: Arc::new(IosCalendar), fonts: Arc::new(IosFonts), ..Services::none() }
}

/// The local calendar, from the C library (iOS keeps the zone in `TZ` and
/// `/etc/localtime` as the Mac does). Clock times are 24-hour until the
/// shell passes the user's setting.
pub struct IosCalendar;

impl Calendar for IosCalendar {
    fn local(&self, t: SystemTime) -> Option<Local> {
        let secs = match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        };
        #[cfg(unix)]
        {
            let at = secs as libc::time_t;
            let mut tm: libc::tm = unsafe { std::mem::zeroed() };
            if unsafe { libc::localtime_r(&at, &mut tm) }.is_null() {
                return None;
            }
            Some(Local {
                year: tm.tm_year + 1900,
                month: tm.tm_mon as u32 + 1,
                day: tm.tm_mday as u32,
                weekday: tm.tm_wday as u32,
                hour: tm.tm_hour as u32,
                minute: tm.tm_min as u32,
                second: tm.tm_sec as u32,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = secs;
            NoServices.local(t)
        }
    }

    fn clock(&self, t: SystemTime) -> String {
        match self.local(t) {
            Some(l) => format!("{:02}:{:02}", l.hour, l.minute),
            None => NoServices.clock(t),
        }
    }
}

/// The fonts, named file by file (never a scan): the system's own where the
/// app may read them, the icons from inside the program. The simulator
/// reads the Mac's font files at the Mac's paths; a device has its own
/// under `/System/Library/Fonts/Core`. Missing files are skipped.
pub struct IosFonts;

/// Where the system's fonts are: the device's folders first, then the Mac's
/// (the simulator).
const DIRS: [&str; 4] = ["/System/Library/Fonts/Core", "/System/Library/Fonts/CoreAddition", "/System/Library/Fonts", "/System/Library/Fonts/Supplemental"];

fn find(names: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for name in names {
        if let Some(p) = DIRS.iter().map(|d| Path::new(d).join(name)).find(|p| p.is_file()) {
            found.push(p);
        }
    }
    found
}

impl Fonts for IosFonts {
    fn font_set(&self, option: u8) -> FontSet {
        let mut now = find(&["HelveticaNeue.ttc", "Menlo.ttc"]);
        let georgia = find(&["Georgia.ttf", "Georgia Bold.ttf", "Georgia Italic.ttf", "Georgia Bold Italic.ttf"]);
        if option == 1 {
            now.extend(georgia.clone());
        }
        let mut later = find(&["Apple Color Emoji.ttc", "AppleColorEmoji@2x.ttc", "Apple Symbols.ttf", "GeezaPro.ttc", "AppleSDGothicNeo.ttc", "PingFang.ttc", "Hiragino Sans GB.ttc", "NewPeninimMT.ttc", "Arial Unicode.ttf"]);
        if option != 1 {
            later.extend(georgia);
        }
        FontSet {
            files: now,
            fallback: later,
            data: vec![include_bytes!("../../../assets/fonts/NotsyIcons.ttf")],
            ui: "Helvetica Neue".into(),
            text: match option {
                1 => "Georgia",
                2 => "Menlo",
                _ => "Helvetica Neue",
            }
            .into(),
            code: "Menlo".into(),
            icon: "Notsy Icons".into(),
        }
    }

    fn note_family(&self, option: u8) -> (String, Vec<PathBuf>) {
        match option {
            1 => ("Georgia".into(), find(&["Georgia.ttf", "Georgia Bold.ttf", "Georgia Italic.ttf", "Georgia Bold Italic.ttf"])),
            2 => ("Menlo".into(), Vec::new()),
            _ => ("Helvetica Neue".into(), Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_local_calendar_and_writes_a_clock() {
        let l = IosCalendar.local(SystemTime::now()).unwrap();
        assert!(l.year >= 2026 && (1..=12).contains(&l.month) && l.weekday < 7 && l.hour < 24);
        assert_eq!(IosCalendar.clock(SystemTime::now()).len(), 5);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn the_simulators_fonts_are_the_macs_and_the_icons_come_inside() {
        let set = IosFonts.font_set(0);
        assert!(set.files.iter().any(|f| f.ends_with("HelveticaNeue.ttc")), "{:?}", set.files);
        let e = notsy_text::Engine::new(set);
        for family in ["Helvetica Neue", "Menlo", "Notsy Icons"] {
            assert!(e.families().iter().any(|f| f == family), "{family} in {:?}", e.families());
        }
    }
}
