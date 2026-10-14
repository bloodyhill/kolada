//! The Font option: sans, serif or monospace for the note (`choose` in
//! settings.rs does it; this holds what it must do, as Windows'
//! `choose(Setting::Font)` does). Which faces each option means is the
//! shell's (`services::Fonts`); the Mac checks its own list in its own tests.

use super::*;

#[test]
fn the_font_option_changes_the_note_typeface_and_what_the_frame_draws() {
    let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-font", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Plan.md"), "# Plan\n\nThe quick brown fox jumps over the lazy dog.\n").unwrap();
    let mut m = Model::new(dir.clone(), true);
    m.frame(2000, 1440, 2.0);
    let sans = m.surface().pixels().to_vec();
    assert!(m.choose(scene::Setting::Font, 2));
    assert_eq!(m.session.font, 2);
    m.frame(2000, 1440, 2.0);
    let mono = m.surface().pixels().to_vec();
    assert!(mono != sans, "monospace draws differently from sans");
    assert!(m.choose(scene::Setting::Font, 0));
    m.frame(2000, 1440, 2.0);
    assert_eq!(m.surface().pixels(), &sans[..], "back to sans, the same frame as at first");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_font_option_asks_the_shell_which_family_and_files_it_means() {
    use crate::services::{Fonts, NoServices};
    use std::sync::Mutex;
    struct Asked(Mutex<Vec<u8>>);
    impl Fonts for Asked {
        fn font_set(&self, option: u8) -> kolada_text::FontSet {
            NoServices.font_set(option)
        }
        fn note_family(&self, option: u8) -> (String, Vec<std::path::PathBuf>) {
            self.0.lock().unwrap().push(option);
            NoServices.note_family(option)
        }
    }
    let dir = std::env::temp_dir().join(format!("kolada-app-test-{}-font-asked", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Plan.md"), "# Plan\n").unwrap();
    let asked = std::sync::Arc::new(Asked(Mutex::default()));
    let mut m = Model::with_services(Services { fonts: asked.clone(), ..Services::none() }, dir.clone(), true);
    m.frame(1000, 700, 1.0);
    assert!(asked.0.lock().unwrap().is_empty(), "launch on sans asks for nothing more");
    let generation = m.eng.borrow().generation();
    m.choose(scene::Setting::Font, 2);
    assert_eq!(*asked.0.lock().unwrap(), vec![2]);
    assert!(m.eng.borrow().generation() > generation, "the notes are laid out again");
    let _ = std::fs::remove_dir_all(dir);
}
