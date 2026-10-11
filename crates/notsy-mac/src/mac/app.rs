//! The application: its menu bar, and the window it opens.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType, NSMenu, NSMenuItem, NSWindow, NSWindowStyleMask,
    NSWindowTitleVisibility,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSString};

use super::{Args, model::Model, view};

pub struct Ivars {
    window: RefCell<Option<Retained<NSWindow>>>,
    /// The Dock menu's recent notes, by tag.
    recent: RefCell<Vec<std::path::PathBuf>>,
}

thread_local! {
    /// Notes' own windows, kept while the app runs.
    static NOTE_WINDOWS: RefCell<Vec<Retained<NSWindow>>> = const { RefCell::new(Vec::new()) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NotsyAppDelegate"]
    #[ivars = Ivars]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    impl AppDelegate {
        /// A recent note from the Dock's menu.
        #[unsafe(method(dockOpen:))]
        fn dock_open(&self, sender: Option<&objc2::runtime::AnyObject>) {
            let Some(tag) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()).map(|i| i.tag()) else { return };
            let Some(path) = usize::try_from(tag).ok().and_then(|i| self.ivars().recent.borrow().get(i).cloned()) else { return };
            super::status::show_window(self.mtm());
            if let Some(view) = super::status::main_view() {
                view.open_path(&path);
            }
        }

        #[unsafe(method(dockNew:))]
        fn dock_new(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            super::status::show_window(self.mtm());
            if let Some(view) = super::status::main_view() {
                view.dock_new();
            }
        }

        #[unsafe(method(dockSearch:))]
        fn dock_search(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            super::status::show_window(self.mtm());
            if let Some(view) = super::status::main_view() {
                view.dock_search();
            }
        }
    }

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            super::trace::mark("launched");
            NSApplication::sharedApplication(self.mtm()).activate();
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _notification: &NSNotification) {
            // The note is written before the app goes, whatever the last edit was.
            let window = self.ivars().window.borrow();
            if let Some(view) = window.as_ref().and_then(|w| w.contentView()).and_then(|v| v.downcast::<view::NotsyView>().ok()) {
                view.save_now();
            }
            NOTE_WINDOWS.with(|ws| {
                for w in ws.borrow().iter() {
                    if let Some(view) = w.contentView().and_then(|v| v.downcast::<view::NotsyView>().ok()) {
                        view.save_now();
                    }
                }
            });
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn terminate_after_last_window(&self, _app: &NSApplication) -> bool {
            // Unless the menu bar icon is set to keep Notsy running.
            !super::status::stay_running()
        }

        /// Files opened with Notsy (the Finder's Open With, another app): a
        /// note opens, an export or a document is imported.
        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _app: &NSApplication, urls: &objc2_foundation::NSArray<objc2_foundation::NSURL>) {
            super::status::show_window(self.mtm());
            let Some(view) = super::status::main_view() else { return };
            for url in urls.iter() {
                if let Some(path) = url.path() {
                    view.open_path(std::path::Path::new(&path.to_string()));
                }
            }
        }

        /// The Dock icon's menu: a new note, search, and the notes written last.
        #[unsafe(method_id(applicationDockMenu:))]
        fn dock_menu(&self, _app: &NSApplication) -> Option<Retained<NSMenu>> {
            self.make_dock_menu()
        }

        /// The Dock icon was clicked with no window open: it comes back.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, visible: bool) -> bool {
            if !visible {
                super::status::show_window(self.mtm());
            }
            true
        }
    }
);

impl AppDelegate {
    /// The Dock icon's menu: the notes written last, a new note, search.
    fn make_dock_menu(&self) -> Option<Retained<NSMenu>> {
            let mtm = self.mtm();
            let view = super::status::main_view()?;
            let menu = NSMenu::new(mtm);
            let recent = view.recent(7);
            for (i, (name, _)) in recent.iter().enumerate() {
                let entry = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), &NSString::from_str(name), Some(sel!(dockOpen:)), &NSString::from_str("")) };
                entry.setTag(i as isize);
                unsafe { entry.setTarget(Some(self)) };
                menu.addItem(&entry);
            }
            if !recent.is_empty() {
                menu.addItem(&NSMenuItem::separatorItem(mtm));
            }
            for (title, action) in [("New Note", sel!(dockNew:)), ("Search Notes", sel!(dockSearch:))] {
                let entry = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), &NSString::from_str(title), Some(action), &NSString::from_str("")) };
                unsafe { entry.setTarget(Some(self)) };
                menu.addItem(&entry);
            }
            *self.ivars().recent.borrow_mut() = recent.into_iter().map(|(_, p)| p).collect();
            Some(menu)
        }

    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(Ivars { window: RefCell::new(None), recent: RefCell::new(Vec::new()) });
        unsafe { msg_send![super(this), init] }
    }
}

/// Every window's style: titled (the traffic lights) with its content the
/// full size of the window, so the note runs under the transparent title bar
/// and the scene's top strip is the title bar (`Model::drags_window`).
fn window_style() -> NSWindowStyleMask {
    NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable | NSWindowStyleMask::FullSizeContentView
}

/// The window: the note under a transparent title bar, so the content runs
/// to the top and the traffic lights float over it.
fn open_window(mtm: MainThreadMarker, args: &Args) -> Retained<NSWindow> {
    let (vault, note) = match args.vault.clone().and_then(super::note::given) {
        Some((vault, note)) => (Some(vault), note),
        None => (None, None),
    };
    let mut model = Model::launch(super::services::services(), super::note::launch_vault(vault), true);
    if let Some(rel) = &note {
        model.open(rel);
    }
    super::trace::mark("model ready");
    let state = view::State { model, first_frame: true };

    let size = if state.model.minimal() { (420.0, 560.0) } else { (1000.0, 720.0) };
    let content = view::NotsyView::new(mtm, view::frame(size.0, size.1), state);
    // A title bar the window's own color, with the buttons in it, over the
    // note: the title is hidden.
    let style = window_style();
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(mtm.alloc(), view::frame(size.0, size.1), style, NSBackingStoreType::Buffered, false)
    };
    window.setTitlebarAppearsTransparent(true);
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setBackgroundColor(Some(&view::background(true)));
    window.setAcceptsMouseMovedEvents(true);
    window.setMinSize(objc2_foundation::NSSize::new(360.0, 320.0));
    window.setContentView(Some(&content));
    // Where it was left, if that is still on a screen; else the middle.
    let kept = content.window_place();
    let on_screen = |r: objc2_foundation::NSRect| {
        objc2_app_kit::NSScreen::screens(mtm).iter().any(|s| {
            let v = s.visibleFrame();
            r.origin.x < v.origin.x + v.size.width - 80.0 && r.origin.x + r.size.width > v.origin.x + 80.0 && r.origin.y < v.origin.y + v.size.height - 40.0 && r.origin.y + r.size.height > v.origin.y + 40.0
        })
    };
    match kept.filter(|r| on_screen(*r)) {
        Some(r) => window.setFrame_display(r, false),
        None => window.center(),
    }
    super::trace::mark("window made");
    window.makeKeyAndOrderFront(None);
    super::trace::mark("window shown");
    window.makeFirstResponder(Some(&content));
    unsafe { window.setReleasedWhenClosed(false) };
    super::status::set_window(window.clone());
    window
}

/// A note in a window of its own, beside the main one: the note alone, with
/// a button that takes it back to the main window.
pub fn open_note_window(mtm: MainThreadMarker, path: &std::path::Path) {
    let Some(main) = super::status::main_view() else { return };
    let vault = main.ivars_vault();
    let Some(rel) = notsy_core::vault::relative(&vault, path) else { return };
    let model = Model::note_window(super::services::services(), vault, &rel, true);
    let size = (560.0, 680.0);
    let content = view::NotsyView::new(mtm, view::frame(size.0, size.1), view::State { model, first_frame: true });
    let style = window_style();
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(mtm.alloc(), view::frame(size.0, size.1), style, NSBackingStoreType::Buffered, false) };
    window.setTitlebarAppearsTransparent(true);
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setBackgroundColor(Some(&view::background(true)));
    window.setAcceptsMouseMovedEvents(true);
    window.setMinSize(objc2_foundation::NSSize::new(360.0, 320.0));
    window.setContentView(Some(&content));
    unsafe { window.setReleasedWhenClosed(false) };
    // Beside the main window, a little in from its left.
    if let Some(main_window) = main.window() {
        let f = main_window.frame();
        window.setFrameTopLeftPoint(objc2_foundation::NSPoint::new(f.origin.x + 300.0, f.origin.y + f.size.height - 40.0));
    } else {
        window.center();
    }
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&content));
    NOTE_WINDOWS.with(|ws| {
        let mut ws = ws.borrow_mut();
        // Windows closed since are let go.
        ws.retain(|w| w.isVisible() || w.isMiniaturized());
        ws.push(window);
    });
}

/// A menu item for the first responder, with a Command key equivalent (an
/// upper-case letter adds Shift).
fn item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str) -> Retained<NSMenuItem> {
    unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), &NSString::from_str(title), action, &NSString::from_str(key)) }
}

fn menu(mtm: MainThreadMarker, title: &str, items: Vec<Retained<NSMenuItem>>) -> Retained<NSMenuItem> {
    let holder = NSMenuItem::new(mtm);
    let submenu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str(title));
    for i in &items {
        submenu.addItem(i);
    }
    holder.setSubmenu(Some(&submenu));
    holder
}

/// The menu bar: Notsy, Edit and Format. Their actions go to whatever has
/// the keyboard, which is the note's view.
fn menu_bar(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let bar = NSMenu::new(mtm);
    bar.addItem(&menu(
        mtm,
        "Notsy",
        vec![item(mtm, "Settings\u{2026}", Some(sel!(openSettings:)), ","), NSMenuItem::separatorItem(mtm), item(mtm, "Quit Notsy", Some(sel!(terminate:)), "q")],
    ));
    bar.addItem(&menu(
        mtm,
        "File",
        vec![
            item(mtm, "New Note", Some(sel!(newNote:)), "n"),
            item(mtm, "Today's Note", Some(sel!(openToday:)), "D"),
            item(mtm, "What's Due", Some(sel!(openDue:)), "t"),
            item(mtm, "Search Notes", Some(sel!(openSearch:)), "o"),
            item(mtm, "Command Palette", Some(sel!(openPalette:)), "p"),
            item(mtm, "Find", Some(sel!(openFind:)), "f"),
            item(mtm, "Find Next", Some(sel!(findNext:)), "g"),
            item(mtm, "Find Previous", Some(sel!(findPrevious:)), "G"),
            item(mtm, "Note History", Some(sel!(togglePast:)), "H"),
            item(mtm, "Open in New Window", Some(sel!(openInWindow:)), ""),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Export Note\u{2026}", Some(sel!(exportNote:)), "E"),
            item(mtm, "Copy Note", Some(sel!(copyNote:)), "C"),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Move to Trash", Some(sel!(trashNote:)), "\u{8}"),
        ],
    ));
    bar.addItem(&menu(
        mtm,
        "View",
        vec![
            item(mtm, "Side Menu", Some(sel!(toggleSideMenu:)), "\\"),
            item(mtm, "Toolbar", Some(sel!(toggleToolbarPanel:)), "|"),
            item(mtm, "Minimal Window", Some(sel!(toggleMinimal:)), "M"),
        ],
    ));
    bar.addItem(&menu(
        mtm,
        "Go",
        vec![item(mtm, "Back", Some(sel!(goBack:)), "["), item(mtm, "Forward", Some(sel!(goForward:)), "]")],
    ));
    bar.addItem(&menu(
        mtm,
        "Edit",
        vec![
            item(mtm, "Undo", Some(sel!(undo:)), "z"),
            item(mtm, "Redo", Some(sel!(redo:)), "Z"),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Cut", Some(sel!(cut:)), "x"),
            item(mtm, "Copy", Some(sel!(copy:)), "c"),
            item(mtm, "Paste", Some(sel!(paste:)), "v"),
            item(mtm, "Paste as Plain Text", Some(sel!(pastePlain:)), "V"),
            item(mtm, "Select All", Some(sel!(selectAll:)), "a"),
        ],
    ));
    bar.addItem(&menu(
        mtm,
        "Format",
        vec![
            item(mtm, "Bold", Some(sel!(toggleBold:)), "b"),
            item(mtm, "Italic", Some(sel!(toggleItalic:)), "i"),
            item(mtm, "Strikethrough", Some(sel!(toggleStrike:)), "X"),
            item(mtm, "Code", Some(sel!(toggleCode:)), "e"),
            item(mtm, "Link", Some(sel!(toggleLink:)), "k"),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Bulleted List", Some(sel!(toggleList:)), "8"),
            item(mtm, "Checklist", Some(sel!(toggleChecklist:)), "l"),
            item(mtm, "Check Off Task", Some(sel!(toggleTask:)), "\r"),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Heading", Some(sel!(cycleHeading:)), "h"),
            item(mtm, "Heading 1", Some(sel!(heading1:)), "1"),
            item(mtm, "Heading 2", Some(sel!(heading2:)), "2"),
            item(mtm, "Heading 3", Some(sel!(heading3:)), "3"),
            item(mtm, "Body Text", Some(sel!(bodyText:)), "0"),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Highlight", Some(sel!(toggleHighlight:)), "U"),
        ],
    ));
    bar
}

pub fn run(args: Args) {
    super::trace::mark("main");
    let mtm = MainThreadMarker::new().expect("the app starts on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    super::trace::mark("application created");
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    super::trace::mark("activation policy set");
    app.setMainMenu(Some(&menu_bar(mtm)));
    super::trace::mark("menu set");
    let delegate = AppDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The window is made and ordered in front before the run loop starts, so
    // it does not wait for the app to finish launching.
    *delegate.ivars().window.borrow_mut() = Some(open_window(mtm, &args));
    super::trace::mark("running");
    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_runs_under_the_title_bar() {
        // Without it the content view started below a 32-point title bar of
        // its own and the side menu sat that much low (seen on the real screen).
        assert!(window_style().contains(NSWindowStyleMask::FullSizeContentView));
        assert!(window_style().contains(NSWindowStyleMask::Titled | NSWindowStyleMask::Resizable));
    }
}
