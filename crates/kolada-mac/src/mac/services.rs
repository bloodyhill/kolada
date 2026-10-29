//! What the Mac gives the app model (`kolada_app::Services`): the local
//! calendar, the system's spell checker, Vision, Foundation's download, the
//! Trash, FSEvents, the system's fonts and ImageIO, each in its own file;
//! this puts them together, and sets the priorities (below).

use std::sync::Arc;

use kolada_app::Services;
use kolada_app::services::Priority;

/// Where the OS lets a thread or a child process run below the window's.
pub struct MacPriority;

impl Priority for MacPriority {
    fn lower_thread(&self) {
        unsafe {
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0);
        }
    }

    fn lower_process(&self, pid: u32) {
        // Below the app, as on Windows: typing comes first.
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, pid, 10);
        }
    }
}

/// All of them, for the app and for dumps.
pub fn services() -> Services {
    Services {
        calendar: Arc::new(super::when::Mac),
        spell: Arc::new(super::spell::MacSpell),
        ocr: Arc::new(super::vision::MacOcr),
        web: Arc::new(super::web::MacWeb),
        bin: Arc::new(super::bin::MacBin),
        watcher: Arc::new(super::watch::MacWatcher),
        priority: Arc::new(MacPriority),
        fonts: Arc::new(super::fonts::MacFonts),
        images: Arc::new(super::images::MacImages),
        installed: Arc::new(super::installed::MacInstalled),
        notify: Arc::new(super::notify::MacNotify),
        socket: Arc::new(super::socket::MacSocket),
        keys: Arc::new(super::keychain::MacKeys),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_app::Model;
    use kolada_ui::scene::Setting;

    #[test]
    fn the_font_option_changes_the_note_typeface_and_what_the_frame_draws() {
        let dir = std::env::temp_dir().join(format!("kolada-mac-test-{}-font", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), "# Plan\n\nThe quick brown fox jumps over the lazy dog.\n").unwrap();
        let mut m = Model::with_services(services(), dir.clone(), true);
        m.frame(2000, 1440, 2.0);
        let sans = m.surface().pixels().to_vec();
        let mut seen = vec![sans.clone()];
        for option in [1, 2] {
            assert!(m.choose(Setting::Font, option));
            m.frame(2000, 1440, 2.0);
            let now = m.surface().pixels().to_vec();
            assert!(seen.iter().all(|p| *p != now), "option {option} draws differently from the ones before (Georgia and Menlo on this Mac)");
            seen.push(now);
        }
        assert!(m.choose(Setting::Font, 0));
        m.frame(2000, 1440, 2.0);
        assert_eq!(m.surface().pixels(), &sans[..], "back to sans, the same frame as at first");
        let _ = std::fs::remove_dir_all(dir);
    }
}
