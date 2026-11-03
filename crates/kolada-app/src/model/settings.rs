//! The settings panel, the page of highlights, and what the shell is asked
//! to do for them (show the vault in Finder, the menu bar icon, the quick
//! note hotkey). The panel is drawn by the shared chrome; this is what its
//! clicks do, as `choose` does on Windows, minus the options the Mac does
//! not have (the scene hides those, see `Platform`).

use std::path::PathBuf;

use kolada_core::related;
use kolada_core::search;
use kolada_core::vault;
use kolada_text::Role;
use kolada_ui::scene::{self, Fonts, Setting, Target};

use super::Model;

/// What the model asks the shell to do: it has no AppKit.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Show the vault's folder in Finder.
    ShowVault(PathBuf),
    /// The menu bar icon setting changed (0 on, 1 off, 2 on and stay running).
    Tray(u8),
    /// The quick note hotkey was turned on or off.
    Hotkey(bool),
    /// A note's own window hands its note (vault-relative) to the main
    /// window, and closes.
    ToMain(String),
    /// The window made minimal (the note alone, small) or full again.
    Minimal(bool),
    /// A panel to choose files or a folder (to import, or as the vault).
    Pick(super::transfer::Pick),
    /// The clipboard's text, for a new note.
    ImportClipboard,
    /// A save panel for the note as this kind.
    Export(super::share::Export),
    /// Quit the app.
    Quit,
    /// "Check for updates" was turned on or off.
    Updates(bool),
    /// Settings' "Check now": ask the feed at once, whatever the daily
    /// rule says; the shell says how far it got (`Model::update_checked`).
    CheckUpdates,
    /// The update pill was clicked: put the update in place and start it,
    /// on the same note (everything is saved first).
    InstallUpdate,
    /// Quick Send's keys changed (`Setting::QuickKeys`: 0 the default keys,
    /// 2 off).
    QuickKeys(u8),
    /// Settings' Quick Send › Devices…: open the popup on its devices
    /// (`Model::quick_open(true)`).
    QuickDevices,
    /// Beam's row or button: open the popup on its beams
    /// (`Model::quick_open(false)`).
    QuickOpen,
    /// The Beam button's "Beam this note" (`Model::quick_beam_note`).
    BeamNote,
    /// Settings' Beam turned on or off: off, the shell gives its keys back,
    /// puts its popup away and drops what it shows of Beam (a tray or menu
    /// bar item); on, it takes the keys again (`quick_wants_keys`).
    Beam(bool),
}

/// The panel's state, and the page of highlights being read-only.
#[derive(Default)]
pub struct State {
    pub open: bool,
    /// Opened for Devices: the next frame scrolls there.
    pub devices_pending: bool,
    /// How far the panel's rows are scrolled, and how far they can be.
    pub scroll: f32,
    pub scroll_max: f32,
    /// The page shown is the gathered highlights: it is read, not typed into.
    pub read_only: bool,
    pub requests: Vec<Request>,
    /// This copy updates itself from the website (the shell says), so
    /// Settings offers "Check for updates".
    pub updater: bool,
    /// The version downloaded and waiting to go in: the pill shows.
    pub update: Option<String>,
    /// Settings' "Check now", as far as it has got.
    pub update_check: scene::CheckState,
}

/// The `[[note]]` the text has around byte `at`, as the name it points to
/// (without a `|alias` or `#heading`).
pub fn wiki_link_at(text: &str, at: usize) -> Option<&str> {
    let at = at.min(text.len());
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    let line = &text[start..end];
    let mut from = 0;
    while let Some(open) = line[from..].find("[[") {
        let open = from + open;
        let Some(close) = line[open + 2..].find("]]") else { return None };
        let close = open + 2 + close;
        if (start + open..=start + close + 2).contains(&at) {
            let name = line[open + 2..close].split(['|', '#']).next().unwrap_or("").trim();
            return (!name.is_empty()).then_some(name);
        }
        from = close + 2;
    }
    None
}

impl Model {
    /// The settings button: opens the panel, or closes it.
    pub fn toggle_options(&mut self) -> bool {
        self.welcome_away();
        self.settings.open = !self.settings.open;
        self.settings.scroll = 0.0;
        self.search = None;
        true
    }

    pub fn close_options(&mut self) -> bool {
        std::mem::replace(&mut self.settings.open, false)
    }

    /// The wheel over the panel scrolls its rows.
    pub fn scroll_options(&mut self, dy: f32) -> bool {
        let to = (self.settings.scroll + dy).clamp(0.0, self.settings.scroll_max);
        std::mem::replace(&mut self.settings.scroll, to) != to
    }

    /// The menu bar icon's setting (0 on, 1 off, 2 stay running) and whether the quick note hotkey is on.
    pub fn tray_mode(&self) -> u8 {
        self.session.tray
    }

    pub fn hotkey_on(&self) -> bool {
        self.session.hotkey == 0
    }

    /// What the shell is asked to do since it last looked.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.settings.requests)
    }

    /// What a shell's own settings page shows (the iPhone's): each setting
    /// offered here, with its value now.
    pub fn settings_offered(&self) -> Vec<(kolada_ui::chrome::Offered, u8)> {
        let values = self.option_values();
        kolada_ui::chrome::offered_settings(self.platform, self.settings.updater).into_iter().map(|o| {
            let v = values[o.setting.index()];
            (o, v)
        }).collect()
    }

    /// A button on a shell's own settings page: another vault folder, or
    /// an import, as the card's buttons do. Whether the screen changes.
    pub fn settings_button(&mut self, target: Target) -> bool {
        match target {
            Target::Transfer(what) => self.transfer(what),
            Target::VaultOpen => {
                self.settings.requests.push(Request::Pick(super::transfer::Pick::Vault));
                false
            }
            _ => false,
        }
    }

    /// An option was chosen: the same values Windows keeps, so a session
    /// moves between them.
    pub fn choose(&mut self, setting: Setting, value: u8) -> bool {
        match setting {
            Setting::Theme => {
                self.session.theme = value;
                self.apply_theme();
            }
            Setting::TextSize => {
                self.session.text_size = value;
                self.fonts = Fonts::new(scene::text_scale(value), self.session.font);
            }
            Setting::Font => {
                self.session.font = value;
                self.apply_font();
            }
            Setting::LineWidth => self.session.line_width = value,
            Setting::Launch => self.session.launch = value,
            Setting::Related => self.session.related = value,
            Setting::Spell => self.session.spell = value,
            Setting::Tray => {
                self.session.tray = value;
                self.settings.requests.push(Request::Tray(value));
            }
            Setting::Hotkey => {
                self.session.hotkey = value;
                self.settings.requests.push(Request::Hotkey(value == 0));
            }
            Setting::Window => self.session.window_mode = value,
            Setting::Assist => self.choose_assist(value),
            Setting::Remind => self.session.remind = value,
            // Offered where the shell said this copy updates itself.
            Setting::Updates => {
                self.session.updates = value;
                self.settings.requests.push(Request::Updates(value == 0));
            }
            Setting::QuickKeys => {
                self.session.quick_keys = value;
                self.settings.requests.push(Request::QuickKeys(value));
            }
            Setting::QuickBeam => self.session.quick_beam = value,
            Setting::Beam => {
                self.session.beam = value;
                self.beam_switched();
            }
            Setting::Motion => {
                self.session.motion = value;
                self.ui.animate = value == 0 && !self.os_reduces_motion;
            }
            Setting::ShowMarks => {
                self.session.show_marks = value == 1;
                self.editor.set_clean(value == 0);
                if let Some(p) = &mut self.popup {
                    p.editor.set_clean(value == 0);
                }
            }
        }
        self.persist_session();
        true
    }

    /// A switch of Settings' Features pressed: its feature off, or on again.
    pub(super) fn switch(&mut self, setting: Setting) -> bool {
        let now = match setting {
            Setting::Assist => super::assist::option(&self.session),
            Setting::Spell => self.session.spell,
            Setting::Beam => self.session.beam,
            Setting::Motion => self.session.motion,
            _ => return false,
        };
        let value = scene::switched(setting, now, super::assist::chosen_index(&self.session));
        let changed = self.choose(setting, value);
        // Its rows fold out from under it, or away, as it turns (at once
        // with motion off, this switch's own included).
        self.ui.fold = self.ui.animate.then(|| (setting, std::time::Instant::now(), scene::switch_on(setting, value)));
        changed
    }

    /// The note's typeface follows the font option: serif and mono are set
    /// in their own families, loaded when first chosen; a font of the
    /// machine's in its own, from the files it was chosen by.
    pub(super) fn apply_font(&mut self) {
        let (family, files) = match super::fontpick::installed(&self.session) {
            Some((family, files)) => (family, files),
            None => self.services.fonts.note_family(self.session.font),
        };
        {
            let mut eng = self.eng.borrow_mut();
            eng.load_files(&files);
            eng.set_family(Role::Text, &family);
        }
        self.fonts = Fonts::new(scene::text_scale(self.session.text_size), self.session.font);
    }

    /// What a click on the panel (or the button that opens it, or the
    /// highlights) does: None when the click is not about them.
    pub(super) fn settings_click(&mut self, x: f32, y: f32) -> Option<bool> {
        let target = scene::hit(&self.hits, x, y);
        if self.settings.open {
            return match target {
                Some(Target::Choice(setting, value)) => {
                    self.font_finder_close();
                    Some(self.choose(setting, value))
                }
                Some(Target::Switch(setting)) => Some(self.switch(setting)),
                Some(Target::FontOther) => Some(self.font_other()),
                Some(Target::FontName(row)) => Some(self.font_clicked(row)),
                Some(Target::FontField) => Some(false),
                Some(Target::Transfer(what)) => Some(self.transfer(what)),
                Some(Target::VaultOpen) => {
                    self.settings.requests.push(Request::Pick(super::transfer::Pick::Vault));
                    Some(false)
                }
                Some(Target::VaultShow) => {
                    self.settings.requests.push(Request::ShowVault(self.vault.clone()));
                    Some(false)
                }
                Some(Target::Quick(_)) => {
                    self.settings.requests.push(Request::QuickDevices);
                    Some(false)
                }
                Some(Target::CheckUpdates) => Some(self.check_updates_now()),
                Some(Target::OptionsClose | Target::OptionsVeil | Target::Options) => Some(self.close_options()),
                // The card and anything under the veil take no clicks.
                _ => Some(false),
            };
        }
        match target {
            Some(Target::Options) => Some(self.toggle_options()),
            Some(Target::Highlights) => Some(self.open_highlights()),
            Some(Target::Note) if self.settings.read_only => {
                let at = self.offset_at(x, y);
                let name = wiki_link_at(self.editor.text(), at).map(str::to_string)?;
                Some(self.open_linked(&name))
            }
            _ => None,
        }
    }

    /// The shell says whether this copy updates itself from the website
    /// (not a store's build, not a dev build): Settings then offers "Check
    /// for updates".
    pub fn set_updater(&mut self, on: bool) {
        self.settings.updater = on;
    }

    /// "Check for updates" is on (the shell checks only then).
    pub fn checks_for_updates(&self) -> bool {
        self.settings.updater && self.session.updates == 0
    }

    /// A newer version is downloaded, checked and waiting: the pill shows
    /// above the side menu's icons. Whether the screen changes.
    pub fn update_ready(&mut self, version: &str) -> bool {
        let new = Some(version.to_string());
        if self.settings.update == new {
            // Found again by "Check now": the pill is there, so the check
            // is done rather than left saying it downloads.
            return std::mem::replace(&mut self.settings.update_check, scene::CheckState::Idle) != scene::CheckState::Idle;
        }
        self.settings.update = new;
        self.settings.update_check = scene::CheckState::Idle;
        true
    }

    /// Settings' "Check now": the shell is asked to check at once (not
    /// while a check is under way). Whether the screen changes.
    fn check_updates_now(&mut self) -> bool {
        if !self.settings.updater || self.settings.update_check.busy() {
            return false;
        }
        self.settings.update_check = scene::CheckState::Checking;
        self.settings.requests.push(Request::CheckUpdates);
        true
    }

    /// The shell says how far "Check now" got: a newer version on its way,
    /// up to date, or it could not check. Whether the screen changes.
    pub fn update_checked(&mut self, state: scene::CheckState) -> bool {
        if self.settings.update_check == state {
            return false;
        }
        self.settings.update_check = state;
        true
    }

    /// The pill was clicked: the shell puts the update in place.
    pub(super) fn install_update(&mut self) -> bool {
        if self.settings.update.is_none() {
            return false;
        }
        self.save_flush();
        self.persist_session();
        self.settings.requests.push(Request::InstallUpdate);
        true
    }

    /// Opens the note a `[[link]]` names, if it exists.
    fn open_linked(&mut self, name: &str) -> bool {
        let Some(path) = vault::find_note(&self.vault, name) else { return false };
        let Some(rel) = vault::relative(&self.vault, &path) else { return false };
        self.open(&rel);
        true
    }

    /// `text` on the page as a note reads it, read and not typed into, and
    /// never written anywhere: a shell's reader (Beam's Read more). Saving it
    /// as a note is the shell's to ask for.
    pub fn read_only_page(&mut self, text: &str) {
        self.save_flush();
        self.session.start_fresh();
        self.show(text.to_string(), String::new(), None);
        self.settings.read_only = true;
    }

    /// The page of every `==marked==` passage in the vault, gathered by
    /// note, each under a link to it. Read, not typed into.
    pub fn open_highlights(&mut self) -> bool {
        let marks = {
            let mut slot = self.index.lock().unwrap();
            // The index is read on a thread of its own once search opens; asked
            // for before, it is read here: this is the user asking for it.
            let notes = slot.get_or_insert_with(|| search::index(&self.vault));
            related::gather_highlights(notes)
        };
        self.index_started = true;
        let text = related::highlights_page(&marks);
        self.save_flush();
        self.search = None;
        self.trash = None;
        self.settings.open = false;
        self.session.start_fresh();
        self.show(text, String::new(), None);
        self.settings.read_only = true;
        true
    }
}

#[cfg(test)]
mod tests;
