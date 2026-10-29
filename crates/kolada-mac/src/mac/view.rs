//! The view that shows the window's pixels: it asks the model for a frame
//! and puts the frame on screen, and turns the pointer, the wheel and the
//! keyboard into what the model understands. Text comes through AppKit's
//! text input protocol, so input methods (Japanese, Chinese, Korean, dead
//! keys, the emoji panel, dictation) work as they do in any Mac app.

use std::cell::RefCell;

use kolada_ui::scene::Tool;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_foundation::NSObjectNSThreadPerformAdditions;
use objc2_app_kit::{
    NSAppearanceCustomization, NSColor, NSDragOperation, NSDraggingInfo, NSEvent, NSEventModifierFlags, NSGraphicsContext, NSPasteboard, NSPasteboardTypeString, NSTextInputClient,
    NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSObject, NSObjectNSDelayedPerforming, NSPoint, NSRange, NSRangePointer, NSRect, NSSize, NSString,
    NSUInteger,
};

use super::model::Model;

/// What the view shows.
pub struct State {
    pub model: Model,
    pub first_frame: bool,
    /// The size the model last drew a frame at: one asked for at another
    /// size (a resize, the first frame) is drawn whole in `drawRect:`.
    drawn: Option<blit::Size>,
}

impl State {
    pub fn new(model: Model) -> State {
        State { model, first_frame: true, drawn: None }
    }
}

/// How far the traffic lights are set in from where AppKit puts them,
/// points right and down: inside the side menu's corner (its card starts
/// at 10 points with a 12-point radius; AppKit's close button is at 10 to 22).
/// `kolada_ui::chrome::TRAFFIC_LIGHTS` holds the room they then take.
pub const LIGHTS_IN: (f64, f64) = (4.0, 3.0);

pub struct Ivars {
    state: RefCell<State>,
    /// When the next step is scheduled for (see `kick`).
    ticking: std::cell::Cell<Option<std::time::Instant>>,
    /// When the key being handled arrived, until its frame is drawn.
    key_at: std::cell::Cell<Option<std::time::Instant>>,
    /// What the open right-click menu's items do, by tag.
    menu: RefCell<Vec<super::model::menu::MenuAction>>,
    /// Where the traffic lights were last put (`place_lights`).
    lights: std::cell::Cell<[(f64, f64); 3]>,
    /// Spelling's next step, and writing the note, after the last edit.
    spell: super::later::Later,
    save: super::later::Later,
    /// What was drawn no longer looks as the view should (the appearance
    /// changed, or a frame could not be drawn when asked): `drawRect:`
    /// draws a frame first.
    stale: std::cell::Cell<bool>,
}

mod arrive;
pub(super) mod blit;
mod context;
#[cfg(debug_assertions)]
mod drive;
#[cfg(debug_assertions)]
mod script;
pub(super) mod quick;
mod serve;
mod share;

/// How long after the last edit the note is written.
const SAVE_DELAY: f64 = 0.6;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "KoladaView"]
    #[ivars = Ivars]
    pub struct KoladaView;

    impl KoladaView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            #[cfg(debug_assertions)]
            drive::drew(dirty);
            self.place_lights();
            self.paint(dirty);
        }

        // Rows count from the top, as the engine's do.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        // VoiceOver: the note is a text area it reads by character, word
        // and line, and follows as the caret moves (`model/access.rs`).
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            true
        }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Option<Retained<NSString>> {
            Some(NSString::from_str("AXTextArea"))
        }

        #[unsafe(method_id(accessibilityLabel))]
        fn accessibility_label(&self) -> Option<Retained<NSString>> {
            Some(NSString::from_str(&self.ivars().state.borrow().model.access_label()))
        }

        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<NSString>> {
            Some(NSString::from_str(self.ivars().state.borrow().model.access_text()))
        }

        #[unsafe(method(accessibilityNumberOfCharacters))]
        fn accessibility_number_of_characters(&self) -> isize {
            self.ivars().state.borrow().model.access_len() as isize
        }

        #[unsafe(method(accessibilitySelectedTextRange))]
        fn accessibility_selected_text_range(&self) -> NSRange {
            let (location, length) = self.ivars().state.borrow().model.selection_utf16();
            NSRange::new(location, length)
        }

        #[unsafe(method(setAccessibilitySelectedTextRange:))]
        fn set_accessibility_selected_text_range(&self, range: NSRange) {
            self.ivars().state.borrow_mut().model.select_utf16(range.location, range.length);
            self.redraw_if(true);
        }

        #[unsafe(method_id(accessibilitySelectedText))]
        fn accessibility_selected_text(&self) -> Option<Retained<NSString>> {
            let st = self.ivars().state.borrow();
            let (location, length) = st.model.selection_utf16();
            Some(NSString::from_str(&st.model.substring_utf16(location, length)))
        }

        #[unsafe(method(accessibilityVisibleCharacterRange))]
        fn accessibility_visible_character_range(&self) -> NSRange {
            let (location, length) = self.ivars().state.borrow().model.access_visible();
            NSRange::new(location, length)
        }

        #[unsafe(method(accessibilityInsertionPointLineNumber))]
        fn accessibility_insertion_point_line_number(&self) -> isize {
            let st = self.ivars().state.borrow();
            let (location, length) = st.model.selection_utf16();
            st.model.access_line_of(location + length) as isize
        }

        #[unsafe(method(accessibilityLineForIndex:))]
        fn accessibility_line_for_index(&self, index: isize) -> isize {
            self.ivars().state.borrow().model.access_line_of(index.max(0) as usize) as isize
        }

        #[unsafe(method(accessibilityRangeForLine:))]
        fn accessibility_range_for_line(&self, line: isize) -> NSRange {
            match self.ivars().state.borrow().model.access_line_range(line.max(0) as usize) {
                Some((location, length)) => NSRange::new(location, length),
                None => NSRange::new(NSNotFound as NSUInteger, 0),
            }
        }

        #[unsafe(method(accessibilityStyleRangeForIndex:))]
        fn accessibility_style_range_for_index(&self, index: isize) -> NSRange {
            let st = self.ivars().state.borrow();
            let line = st.model.access_line_of(index.max(0) as usize);
            let (location, length) = st.model.access_line_range(line).unwrap_or((0, 0));
            NSRange::new(location, length)
        }

        #[unsafe(method(accessibilityRangeForIndex:))]
        fn accessibility_range_for_index(&self, index: isize) -> NSRange {
            let (location, length) = self.ivars().state.borrow().model.access_char_at(index.max(0) as usize);
            NSRange::new(location, length)
        }

        #[unsafe(method_id(accessibilityStringForRange:))]
        fn accessibility_string_for_range(&self, range: NSRange) -> Option<Retained<NSString>> {
            let st = self.ivars().state.borrow();
            let end = (range.location + range.length).min(st.model.access_len());
            let start = range.location.min(end);
            Some(NSString::from_str(&st.model.substring_utf16(start, end - start)))
        }

        #[unsafe(method(accessibilityFrameForRange:))]
        fn accessibility_frame_for_range(&self, range: NSRange) -> NSRect {
            let r = self.ivars().state.borrow().model.access_rect(range.location, range.length);
            let Some(r) = r else { return NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)) };
            let local = NSRect::new(NSPoint::new(r.left as f64, r.top as f64), NSSize::new((r.right - r.left) as f64, (r.bottom - r.top) as f64));
            match self.window() {
                Some(window) => window.convertRectToScreen(self.convertRect_toView(local, None)),
                None => local,
            }
        }

        #[unsafe(method(accessibilityRangeForPosition:))]
        fn accessibility_range_for_position(&self, point: NSPoint) -> NSRange {
            let Some(window) = self.window() else { return NSRange::new(NSNotFound as NSUInteger, 0) };
            let p = self.convertPoint_fromView(window.convertPointFromScreen(point), None);
            let st = self.ivars().state.borrow();
            let (location, length) = st.model.access_char_at(st.model.index_at(p.x as f32, p.y as f32));
            NSRange::new(location, length)
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            // A trackpad's deltas are points; a mouse wheel's are lines, of
            // three body lines a notch as on Windows (eased in by the model).
            let precise = event.hasPreciseScrollingDeltas();
            let step = if precise { 1.0 } else { 24.0 };
            let moved = self.ivars().state.borrow_mut().model.wheel(-(event.scrollingDeltaY() as f32) * step, precise);
            self.redraw_if(moved);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let p = self.point(event);
            let changed = self.ivars().state.borrow_mut().model.pointer_moved(p.x as f32, p.y as f32);
            self.redraw_if(changed);
            // A panel to reveal, a chip to peek at: due by the clock.
            self.kick();
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            let changed = self.ivars().state.borrow_mut().model.pointer_left();
            self.redraw_if(changed);
            self.kick();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            // A click in the middle of a composition settles it first.
            let settled = self.ivars().state.borrow_mut().model.unmark();
            if settled {
                if let Some(context) = self.inputContext() {
                    context.discardMarkedText();
                }
            }
            let p = self.point(event);
            let shift = event.modifierFlags().contains(NSEventModifierFlags::Shift);
            let clicks = event.clickCount().max(1) as u32;
            // Control-click is the Mac's right-click (one-button mice, trackpads).
            if event.modifierFlags().contains(NSEventModifierFlags::Control) {
                self.show_context_menu(event);
                return;
            }
            // The empty strip along the top is the title bar: it moves the
            // window, and a double-click zooms or minimizes, as the user set.
            if !settled && self.ivars().state.borrow().model.drags_window(p.x as f32, p.y as f32) {
                if let Some(window) = self.window() {
                    if clicks >= 2 {
                        title_double_click(&window);
                    } else {
                        window.performWindowDragWithEvent(event);
                    }
                }
                return;
            }
            // Command-click follows a link.
            if event.modifierFlags().contains(NSEventModifierFlags::Command) {
                let followed = self.ivars().state.borrow_mut().model.link_click(p.x as f32, p.y as f32);
                if let Some((changed, effects)) = followed {
                    self.redraw_if(changed || settled);
                    self.run_effects(effects);
                    return;
                }
            }
            let changed = self.ivars().state.borrow_mut().model.pointer_down(p.x as f32, p.y as f32, clicks, shift);
            self.edited(changed || settled);
            self.serve();
            // A button that opens a menu (a row's three dots, send).
            self.show_requested_menu();
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let p = self.point(event);
            let changed = self.ivars().state.borrow_mut().model.pointer_dragged(p.x as f32, p.y as f32);
            self.redraw_if(changed);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let p = self.point(event);
            let (changed, effects) = self.ivars().state.borrow_mut().model.pointer_up_at(p.x as f32, p.y as f32);
            self.edited(changed);
            self.redraw_if(changed);
            self.run_effects(effects);
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            self.ivars().stale.set(true);
            self.setNeedsDisplay(true);
        }

        // The keyboard: AppKit turns key events into text and commands and
        // calls the text input methods below.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if self.ivars().key_at.get().is_none() {
                self.ivars().key_at.set(Some(std::time::Instant::now()));
            }
            let events = NSArray::from_slice(&[event]);
            self.interpretKeyEvents(&events);
        }

        // The menus' actions, sent to the view as the first responder.
        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            self.copy_selection();
        }

        #[unsafe(method(cut:))]
        fn cut(&self, _sender: Option<&AnyObject>) {
            if self.copy_selection() {
                let changed = self.ivars().state.borrow_mut().model.insert("");
                self.edited(changed);
            }
        }

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            self.paste_clipboard(false);
        }

        /// Cmd+Shift+V: the text alone, its formatting left behind.
        #[unsafe(method(pastePlain:))]
        fn paste_plain(&self, _sender: Option<&AnyObject>) {
            self.paste_clipboard(true);
        }

        // Files dragged onto the window (NSDraggingDestination).
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.drag_enter(info)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _info: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            self.drag_exit();
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.drag_over(info)
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.take_drop(info)
        }

        /// The mouse's side buttons: back and forward through the notes visited.
        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            let forward = match event.buttonNumber() {
                3 => false,
                4 => true,
                _ => return,
            };
            let changed = self.ivars().state.borrow_mut().model.step(forward);
            self.redraw_if(changed);
        }

        #[unsafe(method(goBack:))]
        fn go_back(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.step(false);
            self.redraw_if(changed);
        }

        #[unsafe(method(goForward:))]
        fn go_forward(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.step(true);
            self.redraw_if(changed);
        }

        #[unsafe(method(exportNote:))]
        fn export_note(&self, _sender: Option<&AnyObject>) {
            self.menu_action(crate::mac::model::menu::MenuAction::ExportNote);
        }

        #[unsafe(method(copyNote:))]
        fn copy_note(&self, _sender: Option<&AnyObject>) {
            self.menu_action(crate::mac::model::menu::MenuAction::CopyRich);
        }

        #[unsafe(method(openInWindow:))]
        fn open_in_window(&self, _sender: Option<&AnyObject>) {
            let rel = self.ivars().state.borrow().model.note_rel();
            if let Some(rel) = rel {
                self.menu_action(crate::mac::model::menu::MenuAction::OpenWindow(rel));
            }
        }

        #[unsafe(method(toggleHighlight:))]
        fn toggle_highlight(&self, _sender: Option<&AnyObject>) {
            self.menu_action(crate::mac::model::menu::MenuAction::Highlight);
        }

        #[unsafe(method(heading1:))]
        fn heading1(&self, _sender: Option<&AnyObject>) {
            self.block(kolada_editor::BlockKind::Heading(1));
        }

        #[unsafe(method(heading2:))]
        fn heading2(&self, _sender: Option<&AnyObject>) {
            self.block(kolada_editor::BlockKind::Heading(2));
        }

        #[unsafe(method(heading3:))]
        fn heading3(&self, _sender: Option<&AnyObject>) {
            self.block(kolada_editor::BlockKind::Heading(3));
        }

        #[unsafe(method(bodyText:))]
        fn body_text(&self, _sender: Option<&AnyObject>) {
            self.block(kolada_editor::BlockKind::Body);
        }

        #[unsafe(method(toggleTask:))]
        fn toggle_task(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.toggle_task_here();
            self.edited(changed);
        }

        #[unsafe(method(openPalette:))]
        fn open_palette(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.open_palette();
            self.redraw_if(changed);
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.toggle_options();
            self.redraw_if(changed);
        }

        /// Cmd+G and Cmd+Shift+G: the next match, or the one before.
        #[unsafe(method(findNext:))]
        fn find_next(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.find_again(false);
            self.redraw_if(changed);
        }

        #[unsafe(method(findPrevious:))]
        fn find_previous(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.find_again(true);
            self.redraw_if(changed);
        }

        #[unsafe(method(openToday:))]
        fn open_today(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.open_today();
            self.redraw_if(changed);
        }

        #[unsafe(method(toggleSideMenu:))]
        fn toggle_side_menu(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.toggle_panel(kolada_ui::scene::Panel::Sidebar);
            self.redraw_if(changed);
        }

        #[unsafe(method(toggleToolbarPanel:))]
        fn toggle_toolbar_panel(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.toggle_panel(kolada_ui::scene::Panel::Toolbar);
            self.redraw_if(changed);
        }

        #[unsafe(method(toggleMinimal:))]
        fn toggle_minimal(&self, _sender: Option<&AnyObject>) {
            let changed = {
                let mut st = self.ivars().state.borrow_mut();
                let on = !st.model.minimal();
                st.model.set_minimal(on)
            };
            self.redraw_if(changed);
            self.serve();
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.show_context_menu(event);
        }

        #[unsafe(method(contextItem:))]
        fn context_item(&self, sender: Option<&AnyObject>) {
            self.pick_context_item(sender);
        }

        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.select_all();
            self.redraw_if(changed);
        }

        #[unsafe(method(undo:))]
        fn undo(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.undo();
            self.edited(changed);
        }

        #[unsafe(method(redo:))]
        fn redo(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.redo();
            self.edited(changed);
        }

        #[unsafe(method(newNote:))]
        fn new_note(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.new_note();
            self.edited(changed);
        }

        #[unsafe(method(openSearch:))]
        fn open_search(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.open_search();
            self.redraw_if(changed);
        }

        #[unsafe(method(openFind:))]
        fn open_find(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.open_find(false);
            self.redraw_if(changed);
        }

        /// Quick Send's panel, from the File menu.
        #[unsafe(method(openQuickSend:))]
        fn open_quick_send(&self, _sender: Option<&AnyObject>) {
            quick::open(self.mtm(), false);
        }

        /// The first frame is up: the menu bar is made (5 ms that no
        /// longer come before the window's first frame).
        #[unsafe(method(startMenus))]
        fn start_menus(&self) {
            super::app::set_menu_bar(self.mtm());
        }

        /// The first frame is up: pictures start decoding.
        #[unsafe(method(startPictures))]
        fn start_pictures(&self) {
            let wake = Wake(self.retain(), sel!(picturesReady));
            self.ivars().state.borrow_mut().model.start_pictures(move || wake.ready());
            let wake = Wake(self.retain(), sel!(arrivalsReady));
            self.ivars().state.borrow_mut().model.start_arrivals(move || wake.ready());
        }

        /// Pictures fetched for a paste, or dropped documents read, are in.
        #[unsafe(method(arrivalsReady))]
        fn arrivals_ready(&self) {
            let changed = self.ivars().state.borrow_mut().model.poll_arrivals();
            self.edited(changed);
        }

        /// The menu bar icon and the quick note hotkey, as the settings have
        /// them; Quick Send's engine, panel and keys.
        #[unsafe(method(startExtras))]
        fn start_extras_now(&self) {
            self.start_extras();
            quick::start(self);
            self.follow_display_options();
            let wake = Wake(self.retain(), sel!(updaterKnown));
            super::updates::find_out(move || wake.ready());
        }

        /// This copy updates itself: Settings offers the switch, and the
        /// daily check starts while it is on.
        #[unsafe(method(updaterKnown))]
        fn updater_known(&self) {
            let on = {
                let mut st = self.ivars().state.borrow_mut();
                st.model.set_updater(super::updates::is_offered());
                st.model.checks_for_updates()
            };
            self.watch_updates(on);
        }

        /// An update is downloaded and checked: the pill shows.
        #[unsafe(method(updateReady))]
        fn update_ready(&self) {
            let Some(version) = super::updates::take_ready() else { return };
            let changed = self.ivars().state.borrow_mut().model.update_ready(&version);
            self.redraw_if(changed);
        }

        /// Settings' "Check now" got further: Settings says how far.
        #[unsafe(method(updateChecked))]
        fn update_checked(&self) {
            let Some(state) = super::updates::take_checked() else { return };
            let changed = self.ivars().state.borrow_mut().model.update_checked(state);
            self.redraw_if(changed);
        }

        /// notsy.co answered the share card.
        #[unsafe(method(shareReady))]
        fn share_ready(&self) {
            let changed = self.ivars().state.borrow_mut().model.share_ready();
            self.redraw_if(changed);
            self.serve();
        }

        /// Reduce Motion (or another display option) changed in System Settings.
        #[unsafe(method(displayOptionsChanged:))]
        fn display_options_changed(&self, _note: &objc2_foundation::NSNotification) {
            self.read_display_options();
        }

        /// A picture arrived (or is fading in): draw it.
        #[unsafe(method(picturesReady))]
        fn pictures_ready(&self) {
            let (changed, fading) = {
                let mut st = self.ivars().state.borrow_mut();
                (st.model.poll_pictures(), st.model.pictures_fading())
            };
            self.redraw_if(changed || fading);
            if fading {
                unsafe { self.performSelector_withObject_afterDelay(sel!(picturesReady), None, 1.0 / 60.0) };
            }
        }

        /// The writing model answered (or its gists were read).
        #[unsafe(method(assistReady))]
        fn assist_ready(&self) {
            let changed = self.ivars().state.borrow_mut().model.assist_events();
            self.edited(changed);
            self.redraw_if(changed);
        }

        /// Every note is read: the margin finds the notes related to this one.
        #[unsafe(method(indexReady))]
        fn index_ready(&self) {
            let changed = {
                let mut st = self.ivars().state.borrow_mut();
                st.model.due_ready() | st.model.update_related() | st.model.index_arrived()
            };
            self.redraw_if(changed);
        }

        /// What is due opens as a card at the teal dot (Cmd+T).
        #[unsafe(method(openDue:))]
        fn open_due(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.toggle_agenda();
            self.redraw_if(changed);
        }

        #[unsafe(method(startWatching))]
        fn start_watching(&self) {
            let assist = Wake(self.retain(), sel!(assistReady));
            self.ivars().state.borrow_mut().model.start_assist(move || assist.ready());
            let share = Wake(self.retain(), sel!(shareReady));
            self.ivars().state.borrow_mut().model.on_share(move || share.ready());
            let index = Wake(self.retain(), sel!(indexReady));
            self.ivars().state.borrow_mut().model.on_index(move || index.ready());
            let wake = Wake(self.retain(), sel!(vaultChanged));
            self.ivars().state.borrow_mut().model.start_watching(move || wake.ready());
        }

        /// Another app changed files in the vault.
        #[unsafe(method(vaultChanged))]
        fn vault_changed(&self) {
            let changed = self.ivars().state.borrow_mut().model.poll_outside();
            self.redraw_if(changed);
        }

        /// The next few lines are checked for spelling.
        #[unsafe(method(spellStep))]
        fn spell_step(&self) {
            match self.ivars().spell.fire(std::time::Instant::now()) {
                super::later::Fire::Now => {}
                super::later::Fire::Again(s) => return unsafe { self.performSelector_withObject_afterDelay(sel!(spellStep), None, s) },
                super::later::Fire::Nothing => return,
            }
            let (changed, more) = self.ivars().state.borrow_mut().model.spell_step();
            self.redraw_if(changed);
            if more {
                self.schedule_spell(0.02);
            }
        }

        /// The note's past opens or closes (Cmd+Shift+H).
        #[unsafe(method(togglePast:))]
        fn toggle_past(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.toggle_scrub();
            self.redraw_if(changed);
        }

        /// One step of whatever moves: a scroll easing in, the caret
        /// gliding, a page fading in, the scrubber's playhead.
        #[unsafe(method(tick))]
        fn tick(&self) {
            self.ivars().ticking.set(None);
            let changed = self.ivars().state.borrow_mut().model.tick();
            if changed {
                self.refresh();
            }
            self.kick();
        }

        /// Debug builds: the next step of typing with no keyboard (`view/drive.rs`).
        #[unsafe(method(driveStep))]
        fn drive_step(&self) {
            #[cfg(debug_assertions)]
            drive::step(self);
        }

        /// Debug builds: the next step of a script the app runs on itself (`view/script.rs`).
        #[unsafe(method(driveScript))]
        fn drive_script(&self) {
            #[cfg(debug_assertions)]
            script::step(self);
        }

        #[unsafe(method(trashNote:))]
        fn trash_note(&self, _sender: Option<&AnyObject>) {
            let changed = self.ivars().state.borrow_mut().model.trash_open_note();
            self.redraw_if(changed);
        }

        #[unsafe(method(toggleBold:))]
        fn toggle_bold(&self, _sender: Option<&AnyObject>) {
            self.tool(Tool::Bold);
        }

        #[unsafe(method(toggleItalic:))]
        fn toggle_italic(&self, _sender: Option<&AnyObject>) {
            self.tool(Tool::Italic);
        }

        #[unsafe(method(toggleStrike:))]
        fn toggle_strike(&self, _sender: Option<&AnyObject>) {
            self.tool(Tool::Strike);
        }

        #[unsafe(method(toggleCode:))]
        fn toggle_code(&self, _sender: Option<&AnyObject>) {
            self.tool(Tool::Code);
        }

        #[unsafe(method(toggleLink:))]
        fn toggle_link(&self, _sender: Option<&AnyObject>) {
            self.tool(Tool::Link);
        }

        #[unsafe(method(toggleList:))]
        fn toggle_list(&self, _sender: Option<&AnyObject>) {
            self.tool(Tool::List);
        }

        #[unsafe(method(toggleChecklist:))]
        fn toggle_checklist(&self, _sender: Option<&AnyObject>) {
            self.tool(Tool::Checklist);
        }

        #[unsafe(method(cycleHeading:))]
        fn cycle_heading(&self, _sender: Option<&AnyObject>) {
            self.tool(Tool::Heading);
        }

        /// The note is written, a moment after the last edit.
        #[unsafe(method(autosave))]
        fn autosave(&self) {
            match self.ivars().save.fire(std::time::Instant::now()) {
                super::later::Fire::Now => {}
                super::later::Fire::Again(s) => return unsafe { self.performSelector_withObject_afterDelay(sel!(autosave), None, s) },
                super::later::Fire::Nothing => return,
            }
            self.ivars().state.borrow_mut().model.save();
        }
    }

    unsafe impl NSTextInputClient for KoladaView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, replacement: NSRange) {
            let text = plain(string);
            let changed = {
                let mut st = self.ivars().state.borrow_mut();
                if replacement.location != NSNotFound as NSUInteger && !st.model.has_marked() {
                    st.model.select_utf16(replacement.location, replacement.length);
                }
                // What an input method settled on, or plain typing.
                st.model.commit_marked(&text)
            };
            self.edited(changed);
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command(&self, selector: Sel) {
            let changed = self.command(selector.name().to_str().unwrap_or(""));
            self.edited(changed);
            // Return on a command in the palette may ask for more.
            self.serve();
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, selected: NSRange, replacement: NSRange) {
            let text = plain(string);
            let changed = {
                let mut st = self.ivars().state.borrow_mut();
                if replacement.location != NSNotFound as NSUInteger && !st.model.has_marked() {
                    st.model.select_utf16(replacement.location, replacement.length);
                }
                // The selected range is the caret, or the clause being converted.
                st.model.set_marked_converting(&text, selected.location, selected.length)
            };
            self.edited(changed);
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            let changed = self.ivars().state.borrow_mut().model.unmark();
            self.edited(changed);
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            let (location, length) = self.ivars().state.borrow().model.selection_utf16();
            NSRange::new(location, length)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            match self.ivars().state.borrow().model.marked_utf16() {
                Some((location, length)) => NSRange::new(location, length),
                None => NSRange::new(NSNotFound as NSUInteger, 0),
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            self.ivars().state.borrow().model.has_marked()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring(&self, range: NSRange, _actual: NSRangePointer) -> Option<Retained<NSAttributedString>> {
            let text = self.ivars().state.borrow().model.substring_utf16(range.location, range.length);
            Some(NSAttributedString::from_nsstring(&NSString::from_str(&text)))
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        // Where the candidate window goes: under the caret, in screen points.
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect(&self, _range: NSRange, _actual: NSRangePointer) -> NSRect {
            let caret = self.ivars().state.borrow().model.caret_rect();
            let r = caret.map_or(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 16.0)), |c| {
                NSRect::new(NSPoint::new(c.left as f64, c.top as f64), NSSize::new((c.right - c.left).max(1.0) as f64, (c.bottom - c.top) as f64))
            });
            match self.window() {
                Some(window) => window.convertRectToScreen(self.convertRect_toView(r, None)),
                None => r,
            }
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index(&self, point: NSPoint) -> NSUInteger {
            let Some(window) = self.window() else { return NSNotFound as NSUInteger };
            let in_window = window.convertPointFromScreen(point);
            let p = self.convertPoint_fromView(in_window, None);
            self.ivars().state.borrow().model.index_at(p.x as f32, p.y as f32)
        }
    }
);

impl KoladaView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect, state: State) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(Ivars { state: RefCell::new(state), ticking: std::cell::Cell::new(None), key_at: std::cell::Cell::new(None), menu: RefCell::new(Vec::new()), lights: std::cell::Cell::new([(f64::NAN, f64::NAN); 3]), spell: Default::default(), save: Default::default(), stale: std::cell::Cell::new(true) });
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        // The pointer is followed while it is over the view, so rows can answer it.
        let options = NSTrackingAreaOptions::MouseMoved | NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveInKeyWindow | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(mtm.alloc(), frame, options, Some(&view), None) };
        view.addTrackingArea(&area);
        view.accept_drags();
        view
    }

    /// Sets the traffic lights in by `LIGHTS_IN`, inside the side menu's
    /// rounded corner rather than over its edge. AppKit lays the title bar
    /// out again on resizing and leaving full screen, before the content is
    /// drawn: so this runs as each frame is drawn, and moves them only when
    /// AppKit has put them back (three frame reads, nothing at rest).
    fn place_lights(&self) {
        use objc2_app_kit::NSWindowButton;
        let Some(window) = self.window() else { return };
        let mut placed = self.ivars().lights.get();
        for (i, kind) in [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton].into_iter().enumerate() {
            let Some(button) = window.standardWindowButton(kind) else { continue };
            let at = button.frame().origin;
            if (at.x, at.y) == placed[i] {
                continue;
            }
            // The title bar counts up from its bottom: in and down is +x, -y.
            let to = NSPoint::new(at.x + LIGHTS_IN.0, at.y - LIGHTS_IN.1);
            button.setFrameOrigin(to);
            placed[i] = (to.x, to.y);
        }
        self.ivars().lights.set(placed);
    }

    /// Motion as System Settings has it (Accessibility > Display > Reduce
    /// motion), now and whenever it changes: after the first frame, which
    /// it does not touch.
    fn follow_display_options(&self) {
        self.read_display_options();
        let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
        unsafe {
            workspace.notificationCenter().addObserver_selector_name_object(
                self,
                sel!(displayOptionsChanged:),
                Some(objc2_app_kit::NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                None,
            );
        }
    }

    fn read_display_options(&self) {
        let reduce = objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        self.ivars().state.borrow_mut().model.reduce_motion(reduce);
    }

    pub fn redraw_if(&self, changed: bool) {
        if changed {
            self.refresh();
            self.kick();
        }
    }

    /// Frames follow, once a display refresh, while anything moves; with
    /// nothing moving, one step when something is next due by the clock.
    fn kick(&self) {
        let now = std::time::Instant::now();
        let at = match self.ivars().state.try_borrow() {
            Ok(st) if st.model.wants_frames() => Some(now + std::time::Duration::from_secs_f64(1.0 / 60.0)),
            Ok(st) => st.model.next_due().map(|t| t.max(now)),
            Err(_) => None,
        };
        let Some(at) = at else { return };
        // A step sooner than the one scheduled replaces it.
        if let Some(scheduled) = self.ivars().ticking.get() {
            if scheduled <= at {
                return;
            }
            unsafe { NSObject::cancelPreviousPerformRequestsWithTarget_selector_object(self, sel!(tick), None) };
        }
        self.ivars().ticking.set(Some(at));
        unsafe { self.performSelector_withObject_afterDelay(sel!(tick), None, at.duration_since(now).as_secs_f64().max(0.001)) };
    }

    /// The screen changed because of an edit: draw it, and write the note soon.
    fn edited(&self, changed: bool) {
        if !changed {
            return;
        }
        // VoiceOver hears what was typed and where the caret went (nothing
        // is sent when no assistive app listens).
        unsafe {
            objc2_app_kit::NSAccessibilityPostNotification(self, objc2_app_kit::NSAccessibilityValueChangedNotification);
            objc2_app_kit::NSAccessibilityPostNotification(self, objc2_app_kit::NSAccessibilitySelectedTextChangedNotification);
        }
        self.refresh();
        self.kick();
        self.schedule_spell(0.4);
        if let Some(s) = self.ivars().save.push(std::time::Instant::now(), SAVE_DELAY) {
            unsafe { self.performSelector_withObject_afterDelay(sel!(autosave), None, s) };
        }
    }

    /// Checking spelling resumes after `delay` seconds without edits.
    fn schedule_spell(&self, delay: f64) {
        if let Some(s) = self.ivars().spell.push(std::time::Instant::now(), delay) {
            unsafe { self.performSelector_withObject_afterDelay(sel!(spellStep), None, s) };
        }
    }

    /// The daily check for updates, on (or off: it stops looking).
    pub(super) fn watch_updates(&self, on: bool) {
        let wake = Wake(self.retain(), sel!(updateReady));
        super::updates::watch(on, move || wake.ready());
    }

    /// Settings' "Check now": the feed asked at once.
    pub(super) fn check_updates_now(&self) {
        let (said, ready) = (Wake(self.retain(), sel!(updateChecked)), Wake(self.retain(), sel!(updateReady)));
        super::updates::check_now(move || said.ready(), move || ready.ready());
    }

    /// A menu item's action, as if picked from a menu.
    fn menu_action(&self, action: crate::mac::model::menu::MenuAction) {
        let (changed, effects) = self.ivars().state.borrow_mut().model.run_menu(action);
        self.edited(changed);
        self.redraw_if(true);
        self.run_effects(effects);
    }

    fn block(&self, kind: kolada_editor::BlockKind) {
        let changed = self.ivars().state.borrow_mut().model.block(kind);
        self.edited(changed);
    }

    /// Cmd+C: the selection as plain text, HTML and Kolada's Markdown
    /// (`Model::copy`). Whether anything was selected.
    fn copy_selection(&self) -> bool {
        let copied = self.ivars().state.borrow().model.copy();
        let Some(c) = copied else { return false };
        share::set_copied(&c.plain, Some(&c.html), Some(&c.markdown));
        true
    }

    fn tool(&self, tool: Tool) {
        let changed = self.ivars().state.borrow_mut().model.tool(tool);
        self.edited(changed);
    }

    /// A file opened with Kolada: the model opens or imports it, and follows
    /// a new vault's files.
    pub fn open_path(&self, path: &std::path::Path) {
        let moved = self.ivars().state.borrow_mut().model.open_path(path);
        if moved {
            unsafe { self.performSelector_withObject_afterDelay(sel!(startWatching), None, 0.0) };
        }
        self.edited(true);
        self.redraw_if(true);
    }

    /// The notes written last, for the Dock's menu.
    pub fn recent(&self, limit: usize) -> Vec<(String, std::path::PathBuf)> {
        self.ivars().state.borrow().model.recent(limit)
    }

    /// A fresh page, or search, from the Dock's menu.
    pub fn dock_new(&self) {
        let changed = self.ivars().state.borrow_mut().model.new_note();
        self.edited(changed);
    }

    pub fn dock_search(&self) {
        let changed = self.ivars().state.borrow_mut().model.open_search();
        self.redraw_if(changed);
    }

    /// Where the window was left, as the session keeps it (the minimal one's
    /// when it opens minimal).
    pub fn window_place(&self) -> Option<NSRect> {
        let st = self.ivars().state.borrow();
        let minimal = st.model.minimal();
        st.model.window_place(minimal).map(|(x, y, w, h)| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)))
    }

    /// The vault the view's model shows.
    pub fn ivars_vault(&self) -> std::path::PathBuf {
        self.ivars().state.borrow().model.vault().to_path_buf()
    }

    /// Writes the note now (as it quits, or after the last edit), and where
    /// the window is.
    pub fn save_now(&self) {
        if let Some(window) = self.window() {
            let f = window.frame();
            self.ivars().state.borrow_mut().model.place_window((f.origin.x, f.origin.y, f.size.width, f.size.height));
        }
        self.ivars().state.borrow_mut().model.save_flush();
    }

    /// What a command key does: AppKit names it by selector.
    fn command(&self, name: &str) -> bool {
        self.ivars().state.borrow_mut().model.command(name)
    }

    /// Where an event is in this view, DIPs from its top left.
    fn point(&self, event: &NSEvent) -> NSPoint {
        self.convertPoint_fromView(event.locationInWindow(), None)
    }

    fn dark(&self) -> bool {
        self.effectiveAppearance().name().to_string().contains("Dark")
    }

    /// The view's size in pixels, and its scale.
    fn pixel_size(&self) -> blit::Size {
        let bounds = self.bounds();
        let scale = self.window().map_or(2.0, |w| w.backingScaleFactor()) as f32;
        let (w, h) = ((bounds.size.width as f32 * scale).round().max(1.0) as u32, (bounds.size.height as f32 * scale).round().max(1.0) as u32);
        blit::Size { w, h, scale }
    }

    /// Has the model draw a frame of how things are now, at `size`.
    fn draw_frame(&self, st: &mut State, size: blit::Size) {
        let dark = self.dark();
        // In full screen the traffic lights hide: the room kept for them goes back.
        let full = self.window().is_some_and(|w| w.styleMask().contains(objc2_app_kit::NSWindowStyleMask::FullScreen));
        st.model.set_os_buttons(!full);
        st.model.set_dark(dark);
        if let Some(dark) = st.model.dark_to_tell() {
            if let Some(window) = self.window() {
                window.setBackgroundColor(Some(&background(dark)));
            }
        }
        st.model.frame(size.w, size.h, size.scale);
        st.drawn = Some(size);
        self.ivars().stale.set(false);
    }

    /// Something on the screen changed: the frame is drawn now, and AppKit
    /// is asked to put on the screen only the rectangles it changed (a
    /// keystroke: its line; the caret: its few points). The first frame is
    /// left to `drawRect:`, where launch measures it.
    pub fn refresh(&self) {
        #[cfg(debug_assertions)]
        if drive::frozen() {
            return;
        }
        let size = self.pixel_size();
        let rects = match self.ivars().state.try_borrow_mut() {
            Ok(mut st) if !st.first_frame => {
                self.draw_frame(&mut st, size);
                blit::points(st.model.surface().last_damage.as_deref(), size)
            }
            // The first frame, or the model busy (asked from inside one of
            // its calls): the whole view is drawn, from a new frame.
            _ => {
                self.ivars().stale.set(true);
                None
            }
        };
        self.invalidate(rects);
    }

    /// Asks AppKit to draw these rectangles again (None: all of the view).
    fn invalidate(&self, rects: Option<Vec<blit::Points>>) {
        match rects {
            None => self.setNeedsDisplay(true),
            Some(rects) => {
                for (x, y, w, h) in rects {
                    self.setNeedsDisplayInRect(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
                }
            }
        }
    }

    /// Puts the frame on the screen in the rectangles AppKit is drawing,
    /// drawing one first when none was drawn at this size or it looks out
    /// of date.
    fn paint(&self, dirty: NSRect) {
        let bounds = self.bounds();
        let size = self.pixel_size();
        let mut st = self.ivars().state.borrow_mut();
        if st.first_frame {
            super::trace::mark("first frame asked");
        }
        if self.ivars().stale.get() || st.drawn != Some(size) {
            self.draw_frame(&mut st, size);
            // A frame drawn here may change more than AppKit is drawing now:
            // the rest follows in the next pass.
            let whole = dirty.origin.x <= 0.0 && dirty.origin.y <= 0.0 && dirty.size.width >= bounds.size.width && dirty.size.height >= bounds.size.height;
            if !whole {
                self.invalidate(blit::points(st.model.surface().last_damage.as_deref(), size));
            }
        }
        let first = st.first_frame;
        if first {
            super::trace::mark("first frame drawn");
        }
        if let Some(ctx) = NSGraphicsContext::currentContext().map(|c| c.CGContext()) {
            // The display's own colour space: the pixels go up as they are.
            let space = self.window().and_then(|w| w.colorSpace()).and_then(|s| s.CGColorSpace());
            blit::present(&ctx, st.model.surface().pixels(), size, (dirty.origin.x, dirty.origin.y, dirty.size.width, dirty.size.height), space.as_deref());
        }
        // A key's frame is on the screen: what it cost, from its event.
        if let Some(t) = self.ivars().key_at.take() {
            st.model.key_cost(t.elapsed().as_secs_f32() * 1000.0);
        }
        if first {
            super::trace::mark("first frame presented");
            st.model.opened_in(super::trace::age_ms());
            super::trace::exit_if_asked(self.mtm());
            drop(st);
            unsafe {
                self.performSelector_withObject_afterDelay(sel!(startMenus), None, 0.0);
                self.performSelector_withObject_afterDelay(sel!(startPictures), None, 0.0);
                self.performSelector_withObject_afterDelay(sel!(startWatching), None, 0.0);
                self.performSelector_withObject_afterDelay(sel!(startExtras), None, 0.1);
            }
            self.schedule_spell(0.05);
            #[cfg(debug_assertions)]
            {
                drive::start(self);
                script::start(self);
            }
            st = self.ivars().state.borrow_mut();
        }
        st.first_frame = false;
    }
}

/// The view, as the pictures' worker thread holds it to wake the main thread.
/// Only `performSelectorOnMainThread` is sent to it from there, which is safe
/// from any thread.
struct Wake(Retained<KoladaView>, Sel);

unsafe impl Send for Wake {}
unsafe impl Sync for Wake {}

impl Wake {
    fn ready(&self) {
        unsafe { self.0.performSelectorOnMainThread_withObject_waitUntilDone(self.1, None, false) };
    }
}

/// The text an input method hands over: a string or an attributed string.
fn plain(string: &AnyObject) -> String {
    if let Some(attributed) = string.downcast_ref::<NSAttributedString>() {
        attributed.string().to_string()
    } else if let Some(s) = string.downcast_ref::<NSString>() {
        s.to_string()
    } else {
        String::new()
    }
}

fn clipboard() -> Option<String> {
    NSPasteboard::generalPasteboard().stringForType(unsafe { NSPasteboardTypeString }).map(|s| s.to_string())
}

/// The window's own color, behind the title bar: graphite or paper.
pub fn background(dark: bool) -> Retained<NSColor> {
    let (r, g, b) = if dark { (0x1A, 0x1A, 0x1A) } else { (0xF7, 0xF7, 0xF8) };
    NSColor::colorWithSRGBRed_green_blue_alpha(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 1.0)
}

pub fn frame(w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h))
}

/// What a double-click on a title bar does, as System Settings says
/// (Desktop & Dock: "Double-click a window's title bar to"): zoom by default.
fn title_double_click(window: &objc2_app_kit::NSWindow) {
    let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
    let action = defaults.stringForKey(&NSString::from_str("AppleActionOnDoubleClick")).map(|s| s.to_string());
    match action.as_deref() {
        Some("Minimize") => window.performMiniaturize(None),
        Some("None") => {}
        _ => window.performZoom(None),
    }
}
