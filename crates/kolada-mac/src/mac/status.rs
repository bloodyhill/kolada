//! The menu bar icon: the bird, with New Note, Show Kolada and Quit. It is
//! made after the first frame (never before it), and only when the setting
//! asks for it. With "Stay running" closing the window leaves Kolada in the
//! menu bar, and the icon (or the Dock) brings the window back.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength, NSWindow};
use objc2_foundation::{NSData, NSObject, NSSize, NSString};

/// The user's bird, red with its white eye (the one the app's icon is made from).
const BIRD: &[u8] = include_bytes!("../../../../assets/logo.png");

/// Closing the last window leaves the app running.
static STAY: AtomicBool = AtomicBool::new(false);

thread_local! {
    static ITEM: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
    static TARGET: RefCell<Option<Retained<StatusTarget>>> = const { RefCell::new(None) };
    static WINDOW: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
}

define_class!(
    /// What the menu's items are sent to.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KoladaStatusTarget"]
    struct StatusTarget;

    impl StatusTarget {
        #[unsafe(method(newNote:))]
        fn new_note(&self, _sender: Option<&AnyObject>) {
            quick_note(self.mtm());
        }

        #[unsafe(method(quickSend:))]
        fn quick_send(&self, _sender: Option<&AnyObject>) {
            super::view::quick::open(self.mtm(), false);
        }

        #[unsafe(method(showKolada:))]
        fn show_kolada(&self, _sender: Option<&AnyObject>) {
            show_window(self.mtm());
        }

        #[unsafe(method(quitKolada:))]
        fn quit_kolada(&self, _sender: Option<&AnyObject>) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

/// The window the menu brings back.
pub fn set_window(window: Retained<NSWindow>) {
    WINDOW.with(|w| *w.borrow_mut() = Some(window));
}

/// The main window's view, which holds the vault's session.
pub fn main_view() -> Option<Retained<super::view::KoladaView>> {
    WINDOW.with(|w| w.borrow().as_ref().and_then(|w| w.contentView()).and_then(|v| v.downcast::<super::view::KoladaView>().ok()))
}

/// Closing the last window does not quit (the "Stay running" setting).
pub fn stay_running() -> bool {
    STAY.load(Ordering::Relaxed)
}

/// Brings Kolada's window to the front, from the menu bar, the Dock or a hotkey.
pub fn show_window(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    app.activate();
    WINDOW.with(|w| {
        if let Some(window) = w.borrow().as_ref() {
            if window.isMiniaturized() {
                window.deminiaturize(None);
            }
            window.makeKeyAndOrderFront(None);
        }
    });
}

/// A fresh page, from anywhere: the window comes forward and takes it.
pub fn quick_note(mtm: MainThreadMarker) {
    show_window(mtm);
    // To the note's view itself: sent to "the first responder", it reached
    // none, since the app becomes active only after this returns (seen on
    // the real screen: the hotkey brought Kolada forward on the old note).
    let Some(view) = main_view() else { return };
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.sendAction_to_from(sel!(newNote:), Some(&view), None) };
}

/// Applies the setting (0 on, 1 off, 2 on and stay running): makes the icon
/// or takes it away.
pub fn set_mode(mtm: MainThreadMarker, mode: u8) {
    STAY.store(mode == 2, Ordering::Relaxed);
    let have = ITEM.with(|i| i.borrow().is_some());
    match (mode == 1, have) {
        (true, true) => {
            if let Some(item) = ITEM.with(|i| i.borrow_mut().take()) {
                NSStatusBar::systemStatusBar().removeStatusItem(&item);
            }
        }
        (false, false) => install(mtm),
        _ => {}
    }
}

fn entry(mtm: MainThreadMarker, target: &StatusTarget, title: &str, action: objc2::runtime::Sel, key: &str) -> Retained<NSMenuItem> {
    let item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), &NSString::from_str(title), Some(action), &NSString::from_str(key)) };
    unsafe { item.setTarget(Some(target)) };
    item
}

fn install(mtm: MainThreadMarker) {
    let target: Retained<StatusTarget> = unsafe { msg_send![mtm.alloc::<StatusTarget>(), init] };
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        let image = NSImage::initWithData(mtm.alloc(), &NSData::with_bytes(BIRD));
        if let Some(image) = image {
            image.setSize(NSSize::new(18.0, 18.0));
            button.setImage(Some(&image));
        } else {
            button.setTitle(&NSString::from_str("Kolada"));
        }
    }
    let menu = NSMenu::new(mtm);
    menu.addItem(&entry(mtm, &target, "New Note", sel!(newNote:), ""));
    menu.addItem(&entry(mtm, &target, "Beam", sel!(quickSend:), ""));
    menu.addItem(&entry(mtm, &target, "Show Kolada", sel!(showKolada:), ""));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&entry(mtm, &target, "Quit Kolada", sel!(quitKolada:), ""));
    item.setMenu(Some(&menu));
    ITEM.with(|i| *i.borrow_mut() = Some(item));
    TARGET.with(|t| *t.borrow_mut() = Some(target));
}
