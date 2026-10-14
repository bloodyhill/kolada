//! What the model asks the shell for (`model/settings.rs`, `Request`): the
//! vault in Finder, the menu bar icon, the quick note hotkey.

use objc2::{DefinedClass, MainThreadMarker, sel};
use objc2_foundation::NSObjectNSDelayedPerforming;

use super::KoladaView;
use crate::mac::model::settings::Request;
use crate::mac::model::transfer::Pick;
use crate::mac::{hotkey, status};

impl KoladaView {
    /// Does what the model asked for since the last look.
    pub(super) fn serve(&self) {
        let effects = self.ivars().state.borrow_mut().model.take_effects();
        if !effects.is_empty() {
            self.run_effects(effects);
        }
        let requests = self.ivars().state.borrow_mut().model.take_requests();
        for request in requests {
            match request {
                Request::ShowVault(path) => {
                    let _ = std::process::Command::new("open").arg(path).spawn();
                }
                Request::Tray(mode) => status::set_mode(MainThreadMarker::from(self), mode),
                Request::Hotkey(on) => {
                    hotkey::set(on);
                }
                Request::Minimal(on) => self.set_window_minimal(on),
                Request::Quit => {
                    let app = objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(self));
                    app.terminate(None);
                }
                Request::Pick(pick) => {
                    let chosen = super::share::choose(MainThreadMarker::from(self), pick);
                    if chosen.is_empty() {
                        continue;
                    }
                    let changed = {
                        let mut st = self.ivars().state.borrow_mut();
                        match pick {
                            Pick::Files => st.model.import_files(chosen),
                            Pick::Folder => st.model.import_folder(&chosen[0]),
                            Pick::Vault => st.model.switch_vault(chosen[0].clone()),
                        }
                    };
                    // Another vault: its files are followed from now on.
                    if pick == Pick::Vault && changed {
                        unsafe { self.performSelector_withObject_afterDelay(sel!(startWatching), None, 0.0) };
                    }
                    self.edited(changed);
                    self.redraw_if(true);
                }
                Request::ImportClipboard => {
                    let changed = self.ivars().state.borrow_mut().model.import_clipboard(super::clipboard());
                    self.edited(changed);
                    self.redraw_if(true);
                }
                Request::Export(kind) => {
                    let name = self.ivars().state.borrow().model.outgoing().map(|(_, n)| n);
                    if let Some(name) = name {
                        self.export_panel(&name, Some(kind));
                    }
                    self.redraw_if(true);
                }
                Request::ToMain(rel) => {
                    if let Some(main) = status::main_view() {
                        let changed = main.ivars().state.borrow_mut().model.open_note(&rel);
                        main.redraw_if(changed);
                    }
                    status::show_window(MainThreadMarker::from(self));
                    if let Some(window) = self.window() {
                        window.close();
                    }
                }
            }
        }
    }

    /// The window small with the note alone (where it was last minimal, or
    /// at the screen's top right), or full again where it was.
    pub(super) fn set_window_minimal(&self, on: bool) {
        use objc2_foundation::{NSPoint, NSRect, NSSize};
        thread_local! {
            static FULL: std::cell::Cell<Option<NSRect>> = const { std::cell::Cell::new(None) };
            static SMALL: std::cell::Cell<Option<NSRect>> = const { std::cell::Cell::new(None) };
        }
        let Some(window) = self.window() else { return };
        let now = window.frame();
        // The window being left is kept for the next launch too: the full
        // one's place as it goes minimal, the minimal one's as it goes full.
        self.ivars().state.borrow_mut().model.place_window_as(!on, (now.origin.x, now.origin.y, now.size.width, now.size.height));
        // Where each was, as the session kept it, unless moved this run.
        let kept = |minimal: bool| self.ivars().state.borrow().model.window_place(minimal).map(|(x, y, w, h)| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
        if SMALL.with(|f| f.get()).is_none() {
            SMALL.with(|f| f.set(kept(true)));
        }
        if FULL.with(|f| f.get()).is_none() {
            FULL.with(|f| f.set(kept(false)));
        }
        let to = if on {
            FULL.with(|f| f.set(Some(now)));
            SMALL.with(|f| f.get()).unwrap_or_else(|| {
                let screen = window.screen().map(|s| s.visibleFrame()).unwrap_or(now);
                let size = NSSize::new(420.0, 560.0);
                NSRect::new(NSPoint::new(screen.origin.x + screen.size.width - size.width - 24.0, screen.origin.y + screen.size.height - size.height - 24.0), size)
            })
        } else {
            SMALL.with(|f| f.set(Some(now)));
            FULL.with(|f| f.get()).unwrap_or_else(|| NSRect::new(now.origin, NSSize::new(1000.0, 720.0)))
        };
        window.setFrame_display_animate(to, true, true);
        self.setNeedsDisplay(true);
    }

    /// The settings as launch left them: the menu bar icon and the hotkey.
    /// Called after the first frame, never before it.
    pub fn start_extras(&self) {
        let (mode, hotkey_on) = {
            let st = self.ivars().state.borrow();
            (st.model.tray_mode(), st.model.hotkey_on())
        };
        // The icon is off until it is turned on (0 on, 1 off, 2 stay running);
        // a first launch has the session's defaults.
        status::set_mode(MainThreadMarker::from(self), mode);
        hotkey::set(hotkey_on);
    }
}
