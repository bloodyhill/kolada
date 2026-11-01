//! Quick Send on the Mac (docs/specs/2026-10-26-sync-quick-send.md, section
//! 7): a small panel of its own under the menu bar's right end, on the
//! screen with the pointer, never part of the window. It is made once after
//! the first frame and kept hidden, drawn once, so the keys
//! (Ctrl+Option+Shift+Z) only show it. It takes the keys without bringing
//! Kolada forward, as Spotlight does, and goes when another window takes
//! them. What it shows and does is the app model's (`Model::quick_*`); this
//! makes the panel, hands it the pointer, the keys and the clipboard, and
//! does what the model asks (`QuickDo`).

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearanceCustomization, NSBackingStoreType, NSDragOperation, NSDraggingInfo, NSImage, NSModalResponseOK, NSOpenPanel, NSSavePanel, NSColor, NSCursor, NSEvent, NSEventModifierFlags, NSGraphicsContext, NSPanel, NSPasteboard, NSPasteboardTypeString, NSScreen, NSTrackingArea,
    NSTextInputClient, NSTrackingAreaOptions, NSView, NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSData, NSNotFound, NSObject, NSObjectNSDelayedPerforming, NSObjectNSThreadPerformAdditions, NSPoint, NSRange, NSRangePointer, NSRect, NSSize, NSString, NSUInteger, NSURL,
};

use kolada_app::model::quick::{Clip, QuickDo, QuickKey};
use kolada_ui::draft::Key as FieldKey;
use kolada_ui::quick as q;

use super::{KoladaView, blit};
use crate::mac::{hotkey, status};

/// The panel keeps this far from the screen's edges (under the menu bar),
/// points.
const EDGE: f64 = 10.0;
/// What concealed copies are marked with (nspasteboard.org), so clipboard
/// managers keep no copy.
const CONCEALED: &str = "org.nspasteboard.ConcealedType";
/// What a copy meant to pass through is marked with (nspasteboard.org): a
/// password manager's, too.
const TRANSIENT: &str = "org.nspasteboard.TransientType";
/// A wheel notch, points.
const NOTCH: f32 = 24.0;
/// The panel's level: over windows, like the menu bar's own panels.
const LEVEL: isize = 25;

pub struct Ivars {
    /// The main window's view: its model is Quick Send's.
    main: Retained<KoladaView>,
    /// When the next tick is scheduled for.
    ticking: Cell<Option<Instant>>,
    /// When the keys were pressed, until the panel is drawn (test instances
    /// write key to visible).
    pressed: Cell<Option<Instant>>,
    /// The side menu's count as the main window last drew it.
    badge: Cell<Option<u32>>,
}

define_class!(
    /// Takes the keys without bringing Kolada forward, and says when another
    /// window took them.
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "KoladaQuickPanel"]
    pub struct QuickPanel;

    impl QuickPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }

        #[unsafe(method(resignKeyWindow))]
        fn resign_key(&self) {
            unsafe {
                let _: () = msg_send![super(self), resignKeyWindow];
            }
            // Its own open or save panel takes the keys: it stays.
            if PICKING.with(|p| p.get()) {
                return;
            }
            if let Some(view) = view() {
                let dos = view.with(|m| m.quick_deactivated()).unwrap_or_default();
                view.perform(dos);
            }
        }
    }
);

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "KoladaQuickView"]
    #[ivars = Ivars]
    pub struct QuickView;

    impl QuickView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.paint();
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let p = self.point(event);
            let (changed, typing) = self.with(|m| (m.quick_pointer_moved(p.x as f32, p.y as f32), m.quick_takes_typing())).unwrap_or((false, false));
            if changed {
                self.setNeedsDisplay(true);
            }
            // The I-beam over the send area and the code field.
            if typing { NSCursor::IBeamCursor() } else { NSCursor::arrowCursor() }.set();
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            NSCursor::arrowCursor().set();
            if self.with(|m| m.quick_pointer_left()).unwrap_or(false) {
                self.setNeedsDisplay(true);
            }
        }

        /// In the send field's words: the caret there, a double click's
        /// word, a triple click's all; Shift takes the selection there.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.point(event);
            let shift = event.modifierFlags().contains(NSEventModifierFlags::Shift);
            let clicks = event.clickCount().max(1) as u32;
            if self.with(|m| m.quick_field_down(p.x as f32, p.y as f32, clicks, shift)).unwrap_or(false) {
                self.setNeedsDisplay(true);
            }
        }

        /// A drag in the send field selects.
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let p = self.point(event);
            if self.with(|m| m.quick_field_drag(p.x as f32, p.y as f32)).unwrap_or(false) {
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let p = self.point(event);
            let dos = self.with(|m| m.quick_click(p.x as f32, p.y as f32)).unwrap_or_default();
            self.perform(dos);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let step = if event.hasPreciseScrollingDeltas() { 1.0 } else { NOTCH };
            let dy = -event.scrollingDeltaY() as f32 * step;
            if self.with(|m| m.quick_wheel(dy)).unwrap_or(false) {
                self.setNeedsDisplay(true);
            }
        }

        /// A picture dragged over the panel: it is taken.
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, _info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            NSDragOperation::Copy
        }

        /// A picture dropped on the panel, ready to beam.
        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            let files = super::arrive::dragged_files(&info.draggingPasteboard());
            let dos = self.with(|m| m.quick_attach(&files)).unwrap_or_default();
            self.perform(dos);
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            // While an input method composes, every key is its own.
            if self.with(|m| m.quick_has_marked()).unwrap_or(false) {
                self.interpretKeyEvents(&NSArray::from_slice(&[event]));
                return;
            }
            let command = event.modifierFlags().contains(NSEventModifierFlags::Command);
            let shift = event.modifierFlags().contains(NSEventModifierFlags::Shift);
            let plain = event.charactersIgnoringModifiers().map(|s| s.to_string()).unwrap_or_default();
            let key = match event.keyCode() {
                53 => Some(QuickKey::Escape),
                36 | 76 if shift => Some(QuickKey::NewLine),
                36 | 76 => Some(QuickKey::Enter),
                // Option-Delete: a word at a time, as in any Mac field.
                51 if event.modifierFlags().contains(NSEventModifierFlags::Option) => return self.field_key(FieldKey::WordBackspace, false),
                51 => Some(QuickKey::Backspace),
                // Forward Delete, Command's editing keys: the send field's.
                117 => return self.field_key(FieldKey::Delete, false),
                _ if command => {
                    match plain.to_lowercase().as_str() {
                        "a" => self.field_key(FieldKey::SelectAll, false),
                        "z" if shift => self.field_key(FieldKey::Redo, false),
                        "z" => self.field_key(FieldKey::Undo, false),
                        "c" => self.field_clipboard(false),
                        "x" => self.field_clipboard(true),
                        "v" => {
                            let dos = self.with(|m| m.quick_key(QuickKey::Paste, &clipboard)).unwrap_or_default();
                            self.perform(dos);
                        }
                        _ => {}
                    }
                    return;
                }
                _ => None,
            };
            if let Some(key) = key {
                let dos = self.with(|m| m.quick_key(key, &clipboard)).unwrap_or_default();
                return self.perform(dos);
            }
            if command {
                return;
            }
            // Text, through AppKit's text input: input methods (Japanese,
            // Chinese, Korean), dead keys and the emoji panel too.
            self.interpretKeyEvents(&NSArray::from_slice(&[event]));
        }

        /// The engine has news (from Quick Send's thread).
        #[unsafe(method(quickReady))]
        fn quick_ready(&self) {
            let dos = self.with(|m| m.quick_events()).unwrap_or_default();
            self.perform(dos);
            // A beam sent with no panel: its check by the window's button.
            self.ivars().main.redraw_if(true);
        }

        /// Beam's keys were read from the Keychain (on a thread of its own):
        /// Beam starts with them.
        #[unsafe(method(keysRead))]
        fn keys_read(&self) {
            let Some(kept) = KEPT.lock().ok().and_then(|mut k| k.take()) else { return };
            started(self, kept);
        }

        /// Something is due by the clock (`quick_next_due`).
        #[unsafe(method(quickTick))]
        fn quick_tick(&self) {
            self.ivars().ticking.set(None);
            let dos = self.with(|m| m.quick_tick()).unwrap_or_default();
            self.perform(dos);
        }
    }

    /// The field's text input: what is typed, and what an input method
    /// composes, shown in the field until it settles.
    unsafe impl NSTextInputClient for QuickView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, _replacement: NSRange) {
            let text = typed(&super::plain(string));
            if self.with(|m| m.quick_commit(&text)).unwrap_or(false) {
                self.setNeedsDisplay(true);
            }
        }

        // The field's keys (Return, Delete, Esc) are the panel's own
        // (`keyDown:`); the arrows move and, with Shift, select in it.
        #[unsafe(method(doCommandBySelector:))]
        fn do_command(&self, selector: Sel) {
            if let Some((key, extend)) = field_command(selector.name().to_str().unwrap_or("")) {
                self.field_key(key, extend);
            }
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, _selected: NSRange, _replacement: NSRange) {
            let text = super::plain(string);
            if self.with(|m| m.quick_marked(&text)).unwrap_or(false) {
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            self.with(|m| m.quick_unmark());
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            let ((at, length), _) = self.with(|m| m.quick_marked_utf16()).unwrap_or(((0, 0), None));
            NSRange::new(at, length)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            match self.with(|m| m.quick_marked_utf16()).and_then(|(_, marked)| marked) {
                Some((at, length)) => NSRange::new(at, length),
                None => NSRange::new(NSNotFound as NSUInteger, 0),
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            self.with(|m| m.quick_has_marked()).unwrap_or(false)
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring(&self, _range: NSRange, _actual: NSRangePointer) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        // The candidates go under the field, at the panel's foot.
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect(&self, _range: NSRange, _actual: NSRangePointer) -> NSRect {
            let bounds = self.bounds();
            let r = NSRect::new(NSPoint::new(24.0, bounds.size.height - 44.0), NSSize::new(1.0, 20.0));
            match self.window() {
                Some(window) => window.convertRectToScreen(self.convertRect_toView(r, None)),
                None => r,
            }
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index(&self, _point: NSPoint) -> NSUInteger {
            NSNotFound as NSUInteger
        }
    }
);

/// Beam's keys as the Keychain gave them, on their way to the main thread.
static KEPT: std::sync::Mutex<Option<Result<Option<Vec<u8>>, String>>> = std::sync::Mutex::new(None);

thread_local! {
    static PANEL: RefCell<Option<(Retained<QuickPanel>, Retained<QuickView>)>> = const { RefCell::new(None) };
    /// The pasteboard's change count as Beam left it with a password on it.
    static COPIED: Cell<Option<isize>> = const { Cell::new(None) };
    /// The panel's own open or save panel is up.
    static PICKING: Cell<bool> = const { Cell::new(false) };
}

fn panel() -> Option<Retained<QuickPanel>> {
    PANEL.with(|p| p.borrow().as_ref().map(|(panel, _)| panel.clone()))
}

fn view() -> Option<Retained<QuickView>> {
    PANEL.with(|p| p.borrow().as_ref().map(|(_, view)| view.clone()))
}

/// The view, as Quick Send's thread holds it to wake the main thread. Only
/// `performSelectorOnMainThread` is sent to it from there, which is safe
/// from any thread.
struct Wake(Retained<QuickView>, Sel);

unsafe impl Send for Wake {}
unsafe impl Sync for Wake {}

impl Wake {
    fn ready(&self) {
        unsafe { self.0.performSelectorOnMainThread_withObject_waitUntilDone(self.1, None, false) };
    }
}

/// What AppKit's text commands are in the send field: a key, and whether
/// it extends the selection (Shift).
fn field_command(name: &str) -> Option<(FieldKey, bool)> {
    let (name, extend) = match name.strip_suffix("AndModifySelection:") {
        Some(base) => (base, true),
        None => (name.strip_suffix(':').unwrap_or(name), false),
    };
    let key = match name {
        "moveLeft" | "moveBackward" => FieldKey::Left,
        "moveRight" | "moveForward" => FieldKey::Right,
        "moveWordLeft" | "moveWordBackward" => FieldKey::WordLeft,
        "moveWordRight" | "moveWordForward" => FieldKey::WordRight,
        "moveUp" => FieldKey::Up,
        "moveDown" => FieldKey::Down,
        "moveToLeftEndOfLine" | "moveToBeginningOfLine" | "moveToBeginningOfParagraph" => FieldKey::LineStart,
        "moveToRightEndOfLine" | "moveToEndOfLine" | "moveToEndOfParagraph" => FieldKey::LineEnd,
        "moveToBeginningOfDocument" | "scrollToBeginningOfDocument" => FieldKey::Start,
        "moveToEndOfDocument" | "scrollToEndOfDocument" => FieldKey::End,
        "deleteForward" => FieldKey::Delete,
        "deleteWordBackward" => FieldKey::WordBackspace,
        "deleteWordForward" => FieldKey::WordDelete,
        "selectAll" => FieldKey::SelectAll,
        _ => return None,
    };
    Some((key, extend))
}

/// What a key typed into the panel's field: AppKit names the arrows,
/// Home, End, Page Up and Down and the F keys by characters of its own
/// (U+F700 to U+F74F), which are not text and must never be beamed.
fn typed(characters: &str) -> String {
    characters.chars().filter(|c| !('\u{F700}'..='\u{F74F}').contains(c)).collect()
}

/// Whether a pasteboard type marks the copy a password (concealed or
/// transient).
fn sensitive(kind: &str) -> bool {
    kind == CONCEALED || kind == TRANSIENT
}

/// What is on the clipboard: its text, and whether the app that copied it
/// marked it concealed or transient (a password manager); or with no text, a picture (a
/// screenshot), as a PNG; and files copied in the Finder (whose text is
/// only their names). Read only when the model asks.
fn clipboard() -> Option<Clip> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let files = super::arrive::dragged_files(&pasteboard);
    let Some(text) = pasteboard.stringForType(unsafe { NSPasteboardTypeString }).map(|s| s.to_string()) else {
        let Some(png) = super::arrive::picture_on_pasteboard(&pasteboard) else {
            return (!files.is_empty()).then(|| Clip { files, ..Default::default() });
        };
        return Some(Clip { picture: Some(("image/png".into(), png)), files, ..Default::default() });
    };
    let concealed = pasteboard.types().is_some_and(|types| types.iter().any(|t| sensitive(&t.to_string())));
    Some(Clip { text, concealed, files, ..Default::default() })
}

/// Runs one of the panel's own open or save panels: losing the keys to it
/// does not put the panel away.
fn picking<R>(f: impl FnOnce() -> R) -> R {
    PICKING.with(|p| p.set(true));
    let r = f();
    PICKING.with(|p| p.set(false));
    if let Some(panel) = panel() {
        panel.makeKeyAndOrderFront(None);
    }
    r
}

/// The screen the pointer is on (the main one when none says so).
fn pointer_screen(mtm: MainThreadMarker) -> Option<Retained<NSScreen>> {
    let at = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let inside = |f: NSRect| at.x >= f.origin.x && at.x < f.origin.x + f.size.width && at.y >= f.origin.y && at.y < f.origin.y + f.size.height;
    screens.iter().find(|s| inside(s.frame())).or_else(|| NSScreen::mainScreen(mtm))
}

/// After the first frame: Quick Send's engine, and its panel made and drawn
/// once, hidden. Its keys are taken once Settings has them on.
pub fn start(main: &KoladaView) {
    let mtm = MainThreadMarker::from(main);
    if PANEL.with(|p| p.borrow().is_some()) {
        return;
    }
    let size = NSSize::new(q::W as f64, q::H as f64);
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel: Retained<QuickPanel> = unsafe {
        msg_send![mtm.alloc::<QuickPanel>(), initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), size), styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setLevel(LEVEL);
    panel.setHidesOnDeactivate(false);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary);
    let ivars = Ivars { main: main.retain(), ticking: Cell::new(None), pressed: Cell::new(None), badge: Cell::new(None) };
    let view: Retained<QuickView> = unsafe { msg_send![super(mtm.alloc::<QuickView>().set_ivars(ivars)), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), size)] };
    // Pictures dropped on it are beamed.
    let kinds = NSArray::from_retained_slice(&[NSString::from_str("public.file-url")]);
    view.registerForDraggedTypes(&kinds);
    // Rounded as the system's own panels are; the shadow follows.
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setCornerRadius(12.0);
        layer.setMasksToBounds(true);
    }
    let options = NSTrackingAreaOptions::MouseMoved | NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways | NSTrackingAreaOptions::InVisibleRect;
    let area = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(mtm.alloc(), view.bounds(), options, Some(&view), None) };
    view.addTrackingArea(&area);
    panel.setContentView(Some(&view));
    PANEL.with(|p| *p.borrow_mut() = Some((panel, view.clone())));
    // The keys are read on a thread of their own: a Keychain that asks the
    // person (after an update re-signed the app) must not hold up the window.
    let read = Wake(view.clone(), sel!(keysRead));
    std::thread::spawn(move || {
        let kept = kolada_app::services::KeyStore::load(&crate::mac::keychain::MacKeys);
        if let Ok(mut k) = KEPT.lock() {
            *k = Some(kept);
        }
        read.ready();
    });
}

/// The keys read: Beam starts, its panel drawn once and its keys held.
fn started(view: &QuickView, kept: Result<Option<Vec<u8>>, String>) {
    let mtm = MainThreadMarker::from(view);
    let base = std::env::var("KOLADA_SYNC_BASE").unwrap_or_else(|_| kolada_app::quick::SERVER.to_string());
    let model = kolada_app::quick::names::desktop_model(is_laptop());
    let name = std::env::var("KOLADA_SYNC_NAME").unwrap_or_else(|_| kolada_app::quick::names::default_name(computer_name().as_deref(), model));
    let wake = Wake(view.retain(), sel!(quickReady));
    let started = view.with(|m| {
        m.start_quick_from(kept, &base, "desktop", &name, Arc::new(move || wake.ready()));
        m.set_quick_model(model);
        m.quick_on()
    });
    if started != Some(true) {
        return;
    }
    // Drawn once now, so the first press rasterizes nothing.
    let scale = pointer_screen(mtm).map_or(2.0, |s| s.backingScaleFactor()) as f32;
    view.with(|m| m.quick_frame((q::W * scale).round() as u32, (q::H * scale).round() as u32, scale));
    update_keys(mtm);
    // Another keyboard layout: Beam's keys are held where its Z is.
    hotkey::follow_layout(move || update_keys(mtm));
    // The window's Beam button and side-menu row show now.
    view.ivars().main.redraw_if(true);
}

/// Whether this Mac is a laptop: its model is a MacBook's ("MacBookAir10,1").
fn is_laptop() -> bool {
    let mut buf = [0u8; 64];
    let mut len = buf.len();
    let ok = unsafe { libc::sysctlbyname(c"hw.model".as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) } == 0;
    ok && buf[..len.min(buf.len())].starts_with(b"MacBook")
}

/// Files on the general pasteboard, as Finder puts them there: their URLs.
fn copy_files(paths: &[std::path::PathBuf]) -> bool {
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::NSPasteboardWriting;
    let urls: Vec<objc2::rc::Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
        paths.iter().map(|p| ProtocolObject::from_retained(NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy())))).collect();
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.writeObjects(&objc2_foundation::NSArray::from_retained_slice(&urls))
}

/// The Mac's name as Sharing shows it ("Sam's Mac mini"), which is how
/// the other devices list it. NSHost is the one call that has it.
#[allow(deprecated)]
fn computer_name() -> Option<String> {
    objc2_foundation::NSHost::currentHost().localizedName().map(|n| n.to_string()).filter(|n| !n.is_empty())
}

/// Takes Beam's keys from the system, or gives them back, as Settings has
/// them: Option+Z, or where another app holds it, the old
/// Ctrl+Option+Shift+Z, which Settings and the popup then say. A test
/// instance takes them only when asked (`KOLADA_TEST_HOTKEY`), so the
/// user's own Kolada keeps them.
pub fn update_keys(_mtm: MainThreadMarker) {
    let Some(view) = view() else { return };
    let want = view.with(|m| m.quick_wants_keys()).unwrap_or(false) && (!kolada_core::testing() || std::env::var_os("KOLADA_TEST_HOTKEY").is_some());
    let choice = view.with(|m| m.quick_keys_choice()).unwrap_or(0);
    let main = view.ivars().main.clone();
    let in_use = |keys: Option<u8>| main.ivars().state.borrow_mut().model.set_quick_keys_in_use(keys);
    if !want {
        hotkey::set_quick(2);
        let changed = in_use(None);
        main.redraw_if(changed);
        return;
    }
    // Where Option+Z types a letter of the layout's own (Polish Pro's ż),
    // Beam leaves it to typing and takes its other keys.
    let letter = choice == 0 && hotkey::option_z_types_a_letter();
    if letter && hotkey::set_quick(1) {
        let changed = in_use(Some(1));
        main.redraw_if(changed);
        return;
    }
    if !letter && hotkey::set_quick(choice) {
        let changed = in_use(Some(choice));
        main.redraw_if(changed);
        return;
    }
    if choice == 0 && hotkey::set_quick(1) {
        // Option+Z is another app's: the old keys for now (the choice is
        // kept, so the next start tries Option+Z again), said where the
        // keys show.
        in_use(Some(1));
        main.ivars().state.borrow_mut().model.whisper(&format!("{} is in use by another app, so Beam is on {}", hotkey::QUICK_KEYS, hotkey::OLD_KEYS));
    } else {
        in_use(None);
        let keys = if choice == 1 { hotkey::OLD_KEYS } else { hotkey::QUICK_KEYS };
        main.ivars().state.borrow_mut().model.whisper(&format!("{keys} is already in use by another app: choose other keys in Settings"));
    }
    main.redraw_if(true);
}

/// The keys were pressed (`hotkey.rs`): what was copied, shown first, or
/// beamed at once. `secret`: the password keys.
pub fn keys(mtm: MainThreadMarker, secret: bool) {
    keys_with(mtm, secret, &clipboard);
}

/// The keys, with `clip` reading what was copied (debug scripts give text
/// of their own rather than the clipboard's).
pub fn keys_with(_mtm: MainThreadMarker, secret: bool, clip: &dyn Fn() -> Option<Clip>) {
    let Some(view) = view() else { return };
    view.ivars().pressed.set(Some(Instant::now()));
    let dos = view.with(|m| m.quick_keys(secret, clip)).unwrap_or_default();
    view.perform(dos);
}

/// Debug builds' scripts (`script.rs`): runs `f` on the model, and does
/// what it asks.
#[cfg(debug_assertions)]
pub fn run(f: impl FnOnce(&mut kolada_app::Model) -> Vec<QuickDo>) {
    let Some(view) = view() else { return };
    let dos = view.with(f).unwrap_or_default();
    view.perform(dos);
}

/// The same for reading the model; None before Quick Send starts.
#[cfg(debug_assertions)]
pub fn read<R>(f: impl FnOnce(&kolada_app::Model) -> R) -> Option<R> {
    view()?.with(|m| f(m))
}

/// A click on `hit` where the panel last drew it.
#[cfg(debug_assertions)]
pub fn press(hit: q::Hit) -> bool {
    let Some(view) = view() else { return false };
    let Some(r) = view.with(|m| m.quick_place_of(hit)).flatten() else { return false };
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let dos = view.with(|m| m.quick_click(x, y)).unwrap_or_default();
    view.perform(dos);
    true
}

/// Beam runs here (the Dock menu offers it then).
pub fn runs() -> bool {
    view().and_then(|v| v.with(|m| m.quick_badge().is_some())).unwrap_or(false)
}

/// Whether the panel shows.
#[cfg(debug_assertions)]
pub fn visible() -> bool {
    panel().is_some_and(|p| p.isVisible())
}

/// The Beam button's "Beam this note": the open note goes, and its check
/// shows by the button once the server has it.
pub fn beam_note(_mtm: MainThreadMarker) {
    let Some(view) = view() else { return };
    let dos = view.with(|m| m.quick_beam_note()).unwrap_or_default();
    view.perform(dos);
}

/// Opens the panel: from the menus, Settings' Devices… (`devices`) and a
/// click on a notification.
pub fn open(_mtm: MainThreadMarker, devices: bool) {
    let Some(view) = view() else { return };
    let dos = view.with(|m| m.quick_open(devices)).unwrap_or_default();
    view.perform(dos);
}

impl QuickView {
    /// The model, unless it is busy (asked from inside one of its calls).
    /// A key in the send field: drawn again if it changed.
    fn field_key(&self, key: FieldKey, extend: bool) {
        if self.with(|m| m.quick_field_key(key, extend)).unwrap_or(false) {
            self.setNeedsDisplay(true);
        }
    }

    /// Copy (or Cut) in the send field: its selected words to the
    /// clipboard, as plain text.
    fn field_clipboard(&self, cut: bool) {
        let words = self.with(|m| if cut { m.quick_field_cut() } else { m.quick_field_copy() }).flatten();
        let Some(words) = words else { return };
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        unsafe { pasteboard.setString_forType(&NSString::from_str(&words), NSPasteboardTypeString) };
        if cut {
            self.setNeedsDisplay(true);
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut kolada_app::Model) -> R) -> Option<R> {
        let main = &self.ivars().main;
        let mut st = main.ivars().state.try_borrow_mut().ok()?;
        Some(f(&mut st.model))
    }

    fn point(&self, event: &NSEvent) -> NSPoint {
        self.convertPoint_fromView(event.locationInWindow(), None)
    }

    /// Does what the model asked, then draws, and wakes when it is next due.
    fn perform(&self, dos: Vec<QuickDo>) {
        let mtm = MainThreadMarker::from(self);
        for d in dos {
            match d {
                QuickDo::Show { height } => {
                    self.place(height, false);
                    if let Some(panel) = panel() {
                        panel.makeKeyAndOrderFront(None);
                    }
                }
                // "Beamed": shown a moment, without taking the keys, from
                // wherever the beam started (the panel, or the window's button).
                QuickDo::Shrink { height } => {
                    self.place(height, true);
                    if let Some(panel) = panel() {
                        panel.orderFrontRegardless();
                    }
                }
                QuickDo::Hide => {
                    // The input method lets go of what it was composing.
                    if let Some(context) = self.inputContext() {
                        context.discardMarkedText();
                    }
                    if let Some(panel) = panel() {
                        panel.orderOut(None);
                    }
                }
                QuickDo::Copy { text, secret } => {
                    let pasteboard = NSPasteboard::generalPasteboard();
                    pasteboard.clearContents();
                    unsafe {
                        pasteboard.setString_forType(&NSString::from_str(&text), NSPasteboardTypeString);
                        if secret {
                            pasteboard.setString_forType(&NSString::from_str(""), &NSString::from_str(CONCEALED));
                        }
                    }
                    // The pasteboard as this copy left it: only that is cleared.
                    COPIED.with(|c| c.set(secret.then(|| pasteboard.changeCount())));
                }
                QuickDo::Clear(text) => {
                    // Only the password Beam put there, untouched since: a
                    // copy made after it, even of the same text, stays.
                    let pasteboard = NSPasteboard::generalPasteboard();
                    let ours = COPIED.with(|c| c.take()) == Some(pasteboard.changeCount());
                    if ours && pasteboard.stringForType(unsafe { NSPasteboardTypeString }).is_some_and(|s| s.to_string() == text) {
                        pasteboard.clearContents();
                    }
                }
                QuickDo::Url(url) => {
                    if let Some(u) = NSURL::URLWithString(&NSString::from_str(&url)) {
                        NSWorkspace::sharedWorkspace().openURL(&u);
                    }
                }
                QuickDo::BringUp => {
                    status::show_window(mtm);
                    self.ivars().main.redraw_if(true);
                }
                // While Kolada is in front, the side menu's count says it.
                QuickDo::PickFile => {
                    let picked = picking(|| {
                        let open = NSOpenPanel::openPanel(mtm);
                        open.setTitle(Some(&NSString::from_str("Beam pictures")));
                        open.setCanChooseFiles(true);
                        open.setCanChooseDirectories(false);
                        open.setAllowsMultipleSelection(true);
                        if open.runModal() != NSModalResponseOK {
                            return Vec::new();
                        }
                        open.URLs().iter().filter_map(|u| u.path()).map(|p| std::path::PathBuf::from(p.to_string())).collect::<Vec<_>>()
                    });
                    let more = self.with(|m| m.quick_attach(&picked)).unwrap_or_default();
                    return self.perform(more);
                }
                QuickDo::SaveFile { name, bytes } => {
                    let to = picking(|| {
                        let save = NSSavePanel::savePanel(mtm);
                        save.setTitle(Some(&NSString::from_str("Save the picture")));
                        save.setNameFieldStringValue(&NSString::from_str(&name));
                        save.setCanCreateDirectories(true);
                        if save.runModal() != NSModalResponseOK {
                            return None;
                        }
                        save.URL().and_then(|u| u.path()).map(|p| std::path::PathBuf::from(p.to_string()))
                    });
                    if let Some(path) = to
                        && std::fs::write(&path, &bytes).is_err()
                    {
                        self.with(|m| m.whisper("The picture couldn't be saved there."));
                    }
                }
                QuickDo::SaveAll(files) => {
                    // A folder, and each picture there, never over a file.
                    let to = picking(|| {
                        let open = NSOpenPanel::openPanel(mtm);
                        open.setTitle(Some(&NSString::from_str("Save the pictures")));
                        open.setPrompt(Some(&NSString::from_str("Save")));
                        open.setCanChooseFiles(false);
                        open.setCanChooseDirectories(true);
                        open.setCanCreateDirectories(true);
                        open.setAllowsMultipleSelection(false);
                        if open.runModal() != NSModalResponseOK {
                            return None;
                        }
                        open.URL().and_then(|u| u.path()).map(|p| std::path::PathBuf::from(p.to_string()))
                    });
                    if let Some(dir) = to
                        && kolada_core::files::save_all(&dir, &files).is_err()
                    {
                        self.with(|m| m.whisper("The pictures couldn't be saved there."));
                    }
                }
                QuickDo::CopyFiles(files) => {
                    // As files (Finder, Messages and Mail paste them all),
                    // from copies among the app's own files.
                    if let Some(dir) = kolada_core::app_data_dir().map(|d| d.join("sync").join("copied"))
                        && let Ok(paths) = kolada_core::files::copies(&dir, &files)
                    {
                        copy_files(&paths);
                    }
                }
                QuickDo::OpenFile { name, bytes } => {
                    // A picture full size, in Preview, from a copy among the
                    // app's own files (never the vault).
                    if let Some(dir) = kolada_core::app_data_dir().map(|d| d.join("sync").join("open")) {
                        let _ = std::fs::create_dir_all(&dir);
                        let file = dir.join(std::path::Path::new(&name).file_name().unwrap_or(std::ffi::OsStr::new("Picture.png")));
                        if std::fs::write(&file, &bytes).is_ok() {
                            let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
                            NSWorkspace::sharedWorkspace().openURL(&url);
                        }
                    }
                }
                QuickDo::CopyPicture(bytes) => {
                    // As the picture, which every app takes, and as a PNG
                    // beside it when it is one.
                    let pasteboard = NSPasteboard::generalPasteboard();
                    pasteboard.clearContents();
                    let data = NSData::with_bytes(&bytes);
                    if let Some(image) = NSImage::initWithData(mtm.alloc(), &data)
                        && let Some(tiff) = image.TIFFRepresentation()
                    {
                        unsafe { pasteboard.setData_forType(Some(&tiff), objc2_app_kit::NSPasteboardTypeTIFF) };
                    }
                    if bytes.starts_with(b"\x89PNG") {
                        unsafe { pasteboard.setData_forType(Some(&data), objc2_app_kit::NSPasteboardTypePNG) };
                    }
                }
                QuickDo::Notify { title, text } => {
                    if !objc2_app_kit::NSApplication::sharedApplication(mtm).isActive() {
                        crate::mac::notify::quick(&title, &text);
                    }
                }
                QuickDo::Keys => update_keys(mtm),
            }
        }
        if panel().is_some_and(|p| p.isVisible()) {
            self.setNeedsDisplay(true);
        }
        // The side menu's count, when it changed.
        let badge = self.with(|m| m.quick_badge()).flatten();
        if badge != self.ivars().badge.replace(badge) {
            self.ivars().main.redraw_if(true);
        }
        self.schedule();
    }

    /// Puts the panel under the menu bar's right end on the screen with the
    /// pointer, `height` points tall; `keep` keeps its top where it is.
    fn place(&self, height: f32, keep: bool) {
        let Some(panel) = panel() else { return };
        let mtm = MainThreadMarker::from(self);
        let (w, h) = (q::W as f64, height as f64);
        let frame = if keep && panel.isVisible() {
            let now = panel.frame();
            NSRect::new(NSPoint::new(now.origin.x, now.origin.y + now.size.height - h), NSSize::new(w, h))
        } else {
            let Some(screen) = pointer_screen(mtm) else { return };
            let area = screen.visibleFrame();
            NSRect::new(NSPoint::new(area.origin.x + area.size.width - EDGE - w, area.origin.y + area.size.height - EDGE - h), NSSize::new(w, h))
        };
        panel.setFrame_display(frame, false);
        panel.invalidateShadow();
    }

    /// The next tick, when the model says something is due.
    fn schedule(&self) {
        let now = Instant::now();
        let Some(at) = self.with(|m| m.quick_next_due()).flatten().map(|t| t.max(now)) else { return };
        if let Some(scheduled) = self.ivars().ticking.get() {
            if scheduled <= at {
                return;
            }
            unsafe { NSObject::cancelPreviousPerformRequestsWithTarget_selector_object(self, sel!(quickTick), None) };
        }
        self.ivars().ticking.set(Some(at));
        let wait = at.duration_since(now).max(Duration::from_millis(1));
        unsafe { self.performSelector_withObject_afterDelay(sel!(quickTick), None, wait.as_secs_f64()) };
    }

    /// Draws the panel's frame and puts all of it on the screen (it is small).
    fn paint(&self) {
        let bounds = self.bounds();
        let scale = self.window().map_or(2.0, |w| w.backingScaleFactor()) as f32;
        let size = blit::Size { w: (bounds.size.width as f32 * scale).round().max(1.0) as u32, h: (bounds.size.height as f32 * scale).round().max(1.0) as u32, scale };
        let dark = self.effectiveAppearance().name().to_string().contains("Dark");
        let main = &self.ivars().main;
        let Ok(mut st) = main.ivars().state.try_borrow_mut() else { return };
        st.model.set_dark(dark);
        st.model.quick_frame(size.w, size.h, size.scale);
        let Some(surface) = st.model.quick_surface() else { return };
        if let Some(ctx) = NSGraphicsContext::currentContext().map(|c| c.CGContext()) {
            let space = self.window().and_then(|w| w.colorSpace()).and_then(|s| s.CGColorSpace());
            blit::present(&ctx, surface.pixels(), size, (0.0, 0.0, bounds.size.width, bounds.size.height), space.as_deref());
        }
        drop(st);
        // Key to visible, for test instances: appended beside their data
        // (Quick Send's budget is under 50 ms).
        if let Some(t) = self.ivars().pressed.take()
            && kolada_core::testing()
            && let Some(dir) = kolada_core::app_data_dir()
        {
            use std::io::Write;
            // The folder is made when the session is first kept: maybe not yet.
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("quick-open-ms.txt")) {
                let _ = writeln!(f, "{:.2}", t.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    /// AppKit's text commands move and select in the send field as in any
    /// Mac text field (Option a word, Command the line, Shift selects).
    #[test]
    fn the_text_commands_move_and_select_in_the_send_field() {
        use kolada_ui::draft::Key;
        assert_eq!(super::field_command("moveLeft:"), Some((Key::Left, false)));
        assert_eq!(super::field_command("moveWordLeftAndModifySelection:"), Some((Key::WordLeft, true)));
        assert_eq!(super::field_command("moveToRightEndOfLineAndModifySelection:"), Some((Key::LineEnd, true)));
        assert_eq!(super::field_command("moveToEndOfDocument:"), Some((Key::End, false)));
        assert_eq!(super::field_command("deleteWordBackward:"), Some((Key::WordBackspace, false)));
        assert_eq!(super::field_command("insertTab:"), None);
    }

    #[test]
    fn the_arrows_and_function_keys_type_nothing_into_the_field() {
        assert_eq!(super::typed("\u{F700}\u{F703}\u{F729}\u{F704}"), "", "up, right, Home, F1");
        assert_eq!(super::typed("a\u{F702}é"), "aé");
        assert_eq!(super::typed("\u{F8FF}"), "\u{F8FF}", "the Apple logo (Option-Shift-K) is text");
    }

    use kolada_app::Model;
    use kolada_sync::testing::{T, deliver_at, deliver_picture, pair};
    use kolada_sync::{Engine, Outgoing};

    use super::*;

    /// The panel in each state, drawn by the model with this Mac's fonts, as
    /// BMPs in KOLADA_DUMP_DIR (`quick-<state>-<graphite|paper>.bmp`): what
    /// the screen would show, where SSH gives no window captures.
    #[test]
    #[ignore = "writes pictures: KOLADA_DUMP_DIR=<dir> cargo test -p kolada-mac dump_quick_send -- --ignored"]
    fn dump_quick_send() {
        let out = PathBuf::from(std::env::var("KOLADA_DUMP_DIR").expect("KOLADA_DUMP_DIR"));
        for (theme, dark) in [("graphite", true), ("paper", false)] {
            let vault = std::env::temp_dir().join(format!("kolada-mac-quick-{}-{theme}", std::process::id()));
            let _ = std::fs::remove_dir_all(&vault);
            std::fs::create_dir_all(&vault).unwrap();
            let mut a = Engine::open("https://sync.kolada.app", "desktop", "Mac mini", None, None);
            let mut b = Engine::open("https://sync.kolada.app", "ios", "iPhone", None, None);
            pair(&mut a, "desktop", &mut b, "ios");
            for (seq, what) in [Outgoing::Text("Gate B, row 14. Boarding at 19:05.".into()), Outgoing::Text("https://developers.cloudflare.com/durable-objects/".into()), Outgoing::Secret("hunter2!Xq9".into())].into_iter().enumerate() {
                b.send(what, T);
                // Sent a few minutes ago, an hour ago, three hours ago.
                let at = kolada_app::quick::now() - [180_000, 3_600_000, 10_800_000][seq];
                assert!(deliver_at(&mut b, &mut a, seq as u64 + 1, at));
            }
            // A picture from the iPhone: made here, a PNG, with its JPEG
            // thumbnail as a sender makes it (ImageIO).
            let recode = crate::mac::services::services().images.recode().expect("ImageIO");
            let (pw, ph) = (1200u32, 800u32);
            let bgra: Vec<u8> = (0..pw * ph)
                .flat_map(|i| {
                    let (x, y) = ((i % pw) as f32, (i / pw) as f32);
                    let hill = y > 500.0 + 90.0 * (x / 200.0).sin();
                    let sun = (x - 900.0).powi(2) + (y - 200.0).powi(2) < 10_000.0;
                    let (r, g, b) = if sun { (250u8, 220u8, 150u8) } else if hill { (40, 120, 70) } else { (150, 175, 225) };
                    [b, g, r, 255]
                })
                .collect();
            let raster = kolada_core::share::Raster { width: pw, height: ph, bgra };
            let png = recode.encode(&raster, 100, true).expect("a PNG");
            let small = recode.decode(&png, 360).expect("decoded");
            let thumb = recode.encode(&small, 72, false).expect("a JPEG");
            let at = kolada_app::quick::now() - 60_000;
            deliver_picture(&mut b, &mut a, 4, Outgoing::Picture { bytes: png.clone(), mime: "image/png".into(), name: "Whiteboard.png".into(), width: small.width, height: small.height, thumb }, at).expect("a picture went");
            let mut m = Model::with_services(crate::mac::services::services(), vault.clone(), dark);
            m.start_quick_on(a, Arc::new(|| {}));
            let scale = 2.0;
            let shot = |m: &mut Model, name: &str, h: f32| {
                m.quick_frame((q::W * scale) as u32, (h * scale) as u32, scale);
                m.quick_surface().unwrap().dump(&out.join(format!("quick-{name}-{theme}.bmp"))).unwrap();
            };
            m.quick_open(false);
            shot(&mut m, "first-use", q::H);
            // Not now, at the foot's right.
            m.quick_click(q::W - 16.0 - 50.0, q::H - 16.0 - 19.0);
            shot(&mut m, "clips", q::H);
            m.quick_keys(false, &|| Some(Clip { text: "https://developers.cloudflare.com/durable-objects/platform/pricing/".into(), ..Default::default() }));
            shot(&mut m, "ready", q::H);
            shot(&mut m, "short", 330.0);
            m.quick_key(QuickKey::Enter, &|| None);
            shot(&mut m, "sent", q::H);
            m.quick_open(true);
            shot(&mut m, "devices", q::H);
            // The picture card, once its thumbnail is decoded (off the UI
            // thread: waited for here).
            m.quick_open(false);
            for _ in 0..200 {
                m.quick_events();
                m.quick_frame((q::W * scale) as u32, (q::H * scale) as u32, scale);
                if m.quick_place_of(kolada_ui::quick::Hit::SaveFile(0)).is_some() && m.quick_surface().is_some() {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            shot(&mut m, "pictures", q::H);
            // Typing a quick note.
            for c in "Gate B, row 14".chars() {
                m.quick_char(c);
            }
            m.quick_key(QuickKey::NewLine, &|| None);
            for c in "Boarding at 19:05".chars() {
                m.quick_char(c);
            }
            shot(&mut m, "typing", q::H);
            // A screenshot pasted: ready to beam, with its thumbnail.
            m.quick_key(QuickKey::Escape, &|| None);
            m.quick_open(false);
            m.quick_key(QuickKey::Paste, &|| Some(Clip { picture: Some(("image/png".into(), png.clone())), ..Default::default() }));
            for _ in 0..100 {
                m.quick_events();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            shot(&mut m, "picture-ready", q::H);
            let _ = std::fs::remove_dir_all(&vault);
        }
    }

    /// Pictures copied as files are on the general pasteboard as Finder
    /// puts files there: one item each, its file URL.
    #[test]
    fn pictures_go_on_the_pasteboard_as_files() {
        let dir = std::env::temp_dir().join(format!("kolada-copied-{}", std::process::id()));
        let paths = kolada_core::files::copies(&dir, &[("one.png".into(), vec![1]), ("two.png".into(), vec![2])]).unwrap();
        assert!(copy_files(&paths));
        let items = objc2_app_kit::NSPasteboard::generalPasteboard().pasteboardItems().expect("items");
        let urls: Vec<String> = items.iter().filter_map(|i| i.stringForType(unsafe { objc2_app_kit::NSPasteboardTypeFileURL })).map(|s| s.to_string()).collect();
        assert_eq!(urls.len(), 2, "{urls:?}");
        assert!(urls[0].ends_with("one.png") && urls[1].ends_with("two.png"), "{urls:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A password manager's copy is marked concealed or transient
    /// (nspasteboard.org): either makes it a password.
    #[test]
    fn concealed_and_transient_copies_are_passwords() {
        assert!(sensitive("org.nspasteboard.ConcealedType"));
        assert!(sensitive("org.nspasteboard.TransientType"));
        assert!(!sensitive("public.utf8-plain-text"));
        assert!(!sensitive("org.nspasteboard.AutoGeneratedType"));
    }
}
