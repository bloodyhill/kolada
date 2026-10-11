//! Today's note, as on Windows (`notsy-win` `app/daily.rs`): where the vault
//! keeps daily notes (Obsidian's settings: its folder, how days are named,
//! the template each starts from), opened if it is there, else made; the
//! caret at its end, to write on.

use notsy_core::obsidian::{self, When};

use crate::services::{self, Calendar};

use super::*;

/// Now, in local time.
pub fn now(calendar: &dyn Calendar) -> When {
    let t = services::now(calendar);
    When { year: t.year as u16, month: t.month as u8, day: t.day as u8, weekday: t.weekday as u8, hour: t.hour as u8, minute: t.minute as u8 }
}

impl Model {
    /// Opens today's note, made from the vault's template if it is not there.
    pub fn open_today(&mut self) -> bool {
        self.open_day(now(&*self.services.calendar))
    }

    fn open_day(&mut self, when: When) -> bool {
        let daily = obsidian::daily(&self.vault);
        // A format with slashes in it (YYYY/MM/DD) names folders too.
        let path = self.vault.join(&daily.folder).join(format!("{}.md", obsidian::format(&daily.format, when)));
        if !path.exists() {
            let Some(dir) = path.parent() else { return false };
            let template = (!daily.template.is_empty())
                .then(|| std::fs::read_to_string(self.vault.join(format!("{}.md", daily.template.trim_end_matches(".md")))).ok())
                .flatten();
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let text = template.map(|t| obsidian::fill(&t, &stem, when)).unwrap_or_default();
            if std::fs::create_dir_all(dir).is_err() || note::save(&path, &text, note::Format::default()).is_err() {
                self.whisper("Could not make today's note");
                return true;
            }
            self.refresh_tree();
        }
        let Some(rel) = vault::relative(&self.vault, &path) else { return false };
        self.open_note(&rel);
        let end = self.editor.text().len();
        self.editor.set_selection(Selection::caret(end));
        self.caret_moved(false);
        self.reveal = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todays_note_is_made_from_the_vaults_daily_settings_and_opened_again_after() {
        let dir = std::env::temp_dir().join(format!("notsy-app-daily-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".obsidian")).unwrap();
        std::fs::write(dir.join(".obsidian").join("daily-notes.json"), r#"{"folder":"Journal","format":"YYYY-MM-DD","template":"Templates/Day"}"#).unwrap();
        std::fs::create_dir_all(dir.join("Templates")).unwrap();
        std::fs::write(dir.join("Templates").join("Day.md"), "# {{title}}\n\n").unwrap();
        let mut m = Model::new(dir.clone(), true);
        let day = When { year: 2026, month: 10, day: 6, weekday: 2, hour: 9, minute: 30 };
        assert!(m.open_day(day));
        assert_eq!(m.session.note, "Journal/2026-10-06.md");
        assert_eq!(m.text(), "# 2026-10-06\n\n");
        assert_eq!(m.selection().head, m.text().len(), "the caret at its end");
        m.insert("Wrote today.");
        m.save_flush();
        m.new_note();
        assert!(m.open_day(day));
        assert!(m.text().contains("Wrote today."), "the same note, not a new one");
        let _ = std::fs::remove_dir_all(dir);
    }
}
