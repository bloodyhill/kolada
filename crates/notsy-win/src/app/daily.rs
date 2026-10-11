//! Today's note, where the vault keeps daily notes (Obsidian's settings:
//! its folder, how days are named, the template each starts from): opened
//! if it is there, else made; the caret at its end, to write on.

use notsy_core::obsidian::{self, When};

use super::*;

/// Now, in local time.
pub(super) fn now() -> When {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    When { year: t.wYear, month: t.wMonth as u8, day: t.wDay as u8, weekday: t.wDayOfWeek as u8, hour: t.wHour as u8, minute: t.wMinute as u8 }
}

/// Today, in local time.
pub(super) fn today() -> notsy_core::when::Day {
    let w = now();
    notsy_core::when::Day::new(i32::from(w.year), u32::from(w.month), u32::from(w.day)).unwrap_or(notsy_core::when::Day { year: 1970, month: 1, day: 1 })
}

/// What `;;name` expands to in a note: the built-in set, and `;;date` as today.
pub(super) fn snippets() -> notsy_editor::Snippets {
    notsy_editor::Snippets::standard().with_today(|| {
        let w = now();
        format!("{:04}-{:02}-{:02}", w.year, w.month, w.day)
    })
}

impl App {
    pub(super) fn open_today(&mut self) {
        let Some(vault) = self.ensure_vault() else { return };
        let daily = obsidian::daily(&vault);
        let when = now();
        // A format with slashes in it (YYYY/MM/DD) names folders too.
        let path = vault.join(&daily.folder).join(format!("{}.md", obsidian::format(&daily.format, when)));
        if !path.exists() {
            let Some(dir) = path.parent() else { return };
            let template = (!daily.template.is_empty())
                .then(|| std::fs::read_to_string(vault.join(format!("{}.md", daily.template.trim_end_matches(".md")))).ok())
                .flatten();
            let text = template.map(|t| obsidian::fill(&t, &stem(&path), when)).unwrap_or_default();
            if std::fs::create_dir_all(dir).is_err() || note::save(&path, &text, Format::default()).is_err() {
                return self.whisper("Could not make today's note".to_string());
            }
            self.refresh_tree();
        }
        self.open(&path);
        let end = self.editor.text().len();
        self.editor.set_selection(Selection::caret(end));
        self.caret_moved(false);
    }
}
