//! VoiceOver on the Mac: the view is a group holding the note, a text area
//! VoiceOver reads by character, word and line and follows as the caret
//! moves, and a button for each control the last frame drew (the side
//! menu's rows and buttons, the toolbar, settings, menus and cards), named
//! by the model (`model/access.rs`), as the iPad's are. A control pressed is
//! clicked at its middle, as the pointer would. Nothing is kept for this
//! until an assistive app first asks.

use std::cell::{Cell, RefCell};

use kolada_app::model::access::{Away, Control, ControlKind};
use kolada_ui::scene::Target;
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::NSAccessibilityElement;
use objc2_foundation::{NSNotFound, NSNumber, NSObject, NSObjectNSDelayedPerforming, NSPoint, NSRange, NSRect, NSSize, NSString, NSUInteger};

use super::KoladaView;

/// How VoiceOver is told what a control is: its role, and its value (a
/// task's box ticked or not; the choice in force; a folder open or not).
pub(super) fn role_of(kind: ControlKind) -> (&'static str, Option<bool>) {
    match kind {
        ControlKind::Task(done) => ("AXCheckBox", Some(done)),
        ControlKind::Selected => ("AXRadioButton", Some(true)),
        // A folder or a section that folds: VoiceOver says expanded or collapsed.
        ControlKind::Folder(open) => ("AXDisclosureTriangle", Some(open)),
        ControlKind::Button | ControlKind::Heading | ControlKind::Field => ("AXButton", None),
    }
}

/// For each control drawn now, the element made before that it keeps (one
/// standing for the same, taken in order), so VoiceOver keeps its place
/// when the screen is drawn again; None: a new one.
pub(super) fn reuse<T: PartialEq>(old: &[T], new: &[T]) -> Vec<Option<usize>> {
    let mut taken = vec![false; old.len()];
    new.iter()
        .map(|name| {
            let i = old.iter().enumerate().position(|(i, o)| !taken[i] && o == name)?;
            taken[i] = true;
            Some(i)
        })
        .collect()
}

/// A rectangle in a view (DIPs, from its top left) on the screen.
fn on_screen(view: &objc2_app_kit::NSView, r: [f32; 4]) -> NSRect {
    let local = NSRect::new(NSPoint::new(r[0] as f64, r[1] as f64), NSSize::new((r[2] - r[0]) as f64, (r[3] - r[1]) as f64));
    match view.window() {
        Some(window) => window.convertRectToScreen(view.convertRect_toView(local, None)),
        None => local,
    }
}

/// An element's parent for VoiceOver: the view is a group it does not
/// show, so the window (its nearest shown ancestor).
fn unignored(view: Option<Retained<KoladaView>>) -> Option<Retained<AnyObject>> {
    let view: &AnyObject = view.as_deref()?.as_ref();
    unsafe { objc2_app_kit::NSAccessibilityUnignoredAncestor(view) }
}

fn none_range() -> NSRange {
    NSRange::new(NSNotFound as NSUInteger, 0)
}

pub struct NoteIvars {
    view: Weak<KoladaView>,
}

define_class!(
    /// The note, a text area.
    #[unsafe(super(NSAccessibilityElement, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KoladaNoteElement"]
    #[ivars = NoteIvars]
    pub struct NoteElement;

    impl NoteElement {
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            true
        }

        #[unsafe(method_id(accessibilityParent))]
        fn accessibility_parent(&self) -> Option<Retained<AnyObject>> {
            unignored(self.ivars().view.load())
        }

        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame(&self) -> NSRect {
            let Some(view) = self.view() else { return NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)) };
            let b = view.bounds();
            on_screen(&view, [0.0, 0.0, b.size.width as f32, b.size.height as f32])
        }

        /// It has the keys while the view does.
        #[unsafe(method(isAccessibilityFocused))]
        fn is_accessibility_focused(&self) -> bool {
            self.view().is_some_and(|v| v.window().and_then(|w| w.firstResponder()).is_some_and(|r| std::ptr::eq(Retained::as_ptr(&r).cast::<u8>(), Retained::as_ptr(&v).cast::<u8>())))
        }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Option<Retained<NSString>> {
            Some(NSString::from_str("AXTextArea"))
        }

        #[unsafe(method_id(accessibilityLabel))]
        fn accessibility_label(&self) -> Option<Retained<NSString>> {
            self.with(|m| NSString::from_str(&m.access_label()))
        }

        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<NSString>> {
            self.with(|m| NSString::from_str(m.access_text()))
        }

        #[unsafe(method(accessibilityNumberOfCharacters))]
        fn accessibility_number_of_characters(&self) -> isize {
            self.with(|m| m.access_len() as isize).unwrap_or(0)
        }

        #[unsafe(method(accessibilitySelectedTextRange))]
        fn accessibility_selected_text_range(&self) -> NSRange {
            self.with(|m| m.selection_utf16()).map_or(none_range(), |(location, length)| NSRange::new(location, length))
        }

        #[unsafe(method(setAccessibilitySelectedTextRange:))]
        fn set_accessibility_selected_text_range(&self, range: NSRange) {
            let Some(view) = self.view() else { return };
            view.ivars().state.borrow_mut().model.select_utf16(range.location, range.length);
            view.redraw_if(true);
        }

        #[unsafe(method_id(accessibilitySelectedText))]
        fn accessibility_selected_text(&self) -> Option<Retained<NSString>> {
            self.with(|m| {
                let (location, length) = m.selection_utf16();
                NSString::from_str(&m.substring_utf16(location, length))
            })
        }

        #[unsafe(method(accessibilityVisibleCharacterRange))]
        fn accessibility_visible_character_range(&self) -> NSRange {
            self.with(|m| m.access_visible()).map_or(none_range(), |(location, length)| NSRange::new(location, length))
        }

        #[unsafe(method(accessibilityInsertionPointLineNumber))]
        fn accessibility_insertion_point_line_number(&self) -> isize {
            self.with(|m| {
                let (location, length) = m.selection_utf16();
                m.access_line_of(location + length) as isize
            })
            .unwrap_or(0)
        }

        #[unsafe(method(accessibilityLineForIndex:))]
        fn accessibility_line_for_index(&self, index: isize) -> isize {
            self.with(|m| m.access_line_of(index.max(0) as usize) as isize).unwrap_or(0)
        }

        #[unsafe(method(accessibilityRangeForLine:))]
        fn accessibility_range_for_line(&self, line: isize) -> NSRange {
            self.with(|m| m.access_line_range(line.max(0) as usize)).flatten().map_or(none_range(), |(location, length)| NSRange::new(location, length))
        }

        #[unsafe(method(accessibilityStyleRangeForIndex:))]
        fn accessibility_style_range_for_index(&self, index: isize) -> NSRange {
            self.with(|m| {
                let line = m.access_line_of(index.max(0) as usize);
                m.access_line_range(line).unwrap_or((0, 0))
            })
            .map_or(none_range(), |(location, length)| NSRange::new(location, length))
        }

        #[unsafe(method(accessibilityRangeForIndex:))]
        fn accessibility_range_for_index(&self, index: isize) -> NSRange {
            self.with(|m| m.access_char_at(index.max(0) as usize)).map_or(none_range(), |(location, length)| NSRange::new(location, length))
        }

        #[unsafe(method_id(accessibilityStringForRange:))]
        fn accessibility_string_for_range(&self, range: NSRange) -> Option<Retained<NSString>> {
            self.with(|m| {
                let end = (range.location + range.length).min(m.access_len());
                let start = range.location.min(end);
                NSString::from_str(&m.substring_utf16(start, end - start))
            })
        }

        #[unsafe(method(accessibilityFrameForRange:))]
        fn accessibility_frame_for_range(&self, range: NSRange) -> NSRect {
            let empty = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
            let Some(view) = self.view() else { return empty };
            let r = view.ivars().state.borrow().model.access_rect(range.location, range.length);
            r.map_or(empty, |r| on_screen(&view, [r.left, r.top, r.right, r.bottom]))
        }

        #[unsafe(method(accessibilityRangeForPosition:))]
        fn accessibility_range_for_position(&self, point: NSPoint) -> NSRange {
            let Some(view) = self.view() else { return none_range() };
            let Some(window) = view.window() else { return none_range() };
            let p = view.convertPoint_fromView(window.convertPointFromScreen(point), None);
            let st = view.ivars().state.borrow();
            let (location, length) = st.model.access_char_at(st.model.index_at(p.x as f32, p.y as f32));
            NSRange::new(location, length)
        }
    }
);

impl NoteElement {
    pub(super) fn new(view: &KoladaView) -> Retained<NoteElement> {
        let this = MainThreadMarker::from(view).alloc::<NoteElement>().set_ivars(NoteIvars { view: Weak::from(view) });
        unsafe { msg_send![super(this), init] }
    }

    fn view(&self) -> Option<Retained<KoladaView>> {
        self.ivars().view.load()
    }

    fn with<R>(&self, f: impl FnOnce(&kolada_app::Model) -> R) -> Option<R> {
        let view = self.view()?;
        let st = view.ivars().state.try_borrow().ok()?;
        Some(f(&st.model))
    }
}

/// Whose control an element is: the window's, or Beam's panel's.
#[derive(Clone)]
pub(super) enum Owner {
    Main(Weak<KoladaView>),
    Quick(Weak<super::quick::QuickView>),
}

pub struct ControlIvars {
    owner: Owner,
    /// Where it was drawn last (DIPs), or None once it is no longer drawn.
    rect: Cell<Option<[f32; 4]>>,
    label: RefCell<String>,
    kind: Cell<ControlKind>,
    target: Cell<Option<Target>>,
    /// Scrolled out of sight: how far its list scrolls to show it.
    away: Cell<Option<Away>>,
}

define_class!(
    /// One of the chrome's controls, a button VoiceOver can press.
    #[unsafe(super(NSAccessibilityElement, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KoladaControlElement"]
    #[ivars = ControlIvars]
    pub struct ControlElement;

    impl ControlElement {
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            true
        }

        #[unsafe(method_id(accessibilityParent))]
        fn accessibility_parent(&self) -> Option<Retained<AnyObject>> {
            self.owner_view().and_then(|view| {
                let view: &AnyObject = view.as_ref();
                unsafe { objc2_app_kit::NSAccessibilityUnignoredAncestor(view) }
            })
        }

        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame(&self) -> NSRect {
            match (self.owner_view(), self.ivars().rect.get()) {
                (Some(view), Some(r)) => on_screen(&view, r),
                _ => NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
            }
        }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Option<Retained<NSString>> {
            Some(NSString::from_str(role_of(self.ivars().kind.get()).0))
        }

        #[unsafe(method_id(accessibilityLabel))]
        fn accessibility_label(&self) -> Option<Retained<NSString>> {
            Some(NSString::from_str(&self.ivars().label.borrow()))
        }

        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<NSNumber>> {
            role_of(self.ivars().kind.get()).1.map(NSNumber::new_bool)
        }

        /// Pressed: its list scrolled to it first if it is out of sight,
        /// then clicked at its middle, as the pointer would.
        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> objc2::runtime::Bool {
            let Some(target) = self.ivars().target.get() else { return objc2::runtime::Bool::NO };
            let away = self.ivars().away.get();
            let pressed = match &self.ivars().owner {
                Owner::Main(v) => v.load().is_some_and(|v| {
                    if let Some(a) = away {
                        v.access_scroll(a);
                    }
                    let Some(r) = v.access_rect(target) else { return false };
                    v.access_press((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
                    v.control_elements();
                    true
                }),
                Owner::Quick(q) => q.load().is_some_and(|q| q.access_press(target, away)),
            };
            if pressed {
                // A task ticked, a choice made: said at once.
                unsafe { objc2_app_kit::NSAccessibilityPostNotification(self, objc2_app_kit::NSAccessibilityValueChangedNotification) };
            }
            pressed.into()
        }
    }
);

impl ControlElement {
    fn new(owner: Owner, mtm: MainThreadMarker) -> Retained<ControlElement> {
        let ivars = ControlIvars { owner, rect: Cell::new(None), label: RefCell::new(String::new()), kind: Cell::new(ControlKind::Button), target: Cell::new(None), away: Cell::new(None) };
        let this = mtm.alloc::<ControlElement>().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }

    fn owner_view(&self) -> Option<Retained<objc2_app_kit::NSView>> {
        match &self.ivars().owner {
            Owner::Main(v) => v.load().map(|v| Retained::into_super(v)),
            Owner::Quick(q) => q.load().map(|q| Retained::into_super(q)),
        }
    }

    fn set(&self, c: &Control) {
        self.ivars().rect.set(Some([c.rect.left, c.rect.top, c.rect.right, c.rect.bottom]));
        *self.ivars().label.borrow_mut() = c.label.clone();
        self.ivars().kind.set(c.kind);
        self.ivars().target.set(Some(c.target));
        self.ivars().away.set(c.away);
    }

    /// Whether (x, y) in its view is on it; never for one out of sight.
    fn contains(&self, x: f32, y: f32) -> bool {
        self.ivars().away.get().is_none() && self.ivars().rect.get().is_some_and(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3])
    }
}

/// The elements for `controls`, those made before kept by what they stand
/// for (so VoiceOver keeps its place across frames), into `kept`.
pub(super) fn elements(owner: Owner, mtm: MainThreadMarker, kept: &RefCell<Vec<Retained<ControlElement>>>, controls: &[Control]) -> Vec<Retained<ControlElement>> {
    let before = kept.borrow().clone();
    let old: Vec<Option<Target>> = before.iter().map(|c| c.ivars().target.get()).collect();
    let new: Vec<Option<Target>> = controls.iter().map(|c| Some(c.target)).collect();
    let reused = reuse(&old, &new);
    // Those no longer drawn press nothing.
    for (i, e) in before.iter().enumerate() {
        if !reused.contains(&Some(i)) {
            e.ivars().rect.set(None);
            e.ivars().target.set(None);
        }
    }
    let made: Vec<Retained<ControlElement>> = controls
        .iter()
        .zip(&reused)
        .map(|(c, k)| {
            let e = k.map(|i| before[i].clone()).unwrap_or_else(|| ControlElement::new(owner.clone(), mtm));
            e.set(c);
            e
        })
        .collect();
    *kept.borrow_mut() = made.clone();
    made
}

/// The element at (x, y) in a view among `controls`, the topmost.
pub(super) fn hit(controls: &[Retained<ControlElement>], x: f32, y: f32) -> Option<Retained<NSObject>> {
    controls.iter().rev().find(|c| c.contains(x, y)).map(|c| Retained::into_super(Retained::into_super(c.clone())))
}

/// Elements as an array AppKit takes.
pub(super) fn array(all: Vec<Retained<NSObject>>) -> Retained<objc2_foundation::NSArray<NSObject>> {
    objc2_foundation::NSArray::from_retained_slice(&all)
}

/// The view's elements: the note's, and the controls' as last asked for.
#[derive(Default)]
pub struct Access {
    note: RefCell<Option<Retained<NoteElement>>>,
    controls: RefCell<Vec<Retained<ControlElement>>>,
    /// The controls' names the last frame drew, to tell VoiceOver when
    /// they change.
    names: RefCell<Vec<String>>,
}

impl KoladaView {
    /// The note's element (made once).
    pub(super) fn note_element(&self) -> Retained<NoteElement> {
        self.ivars().access.note.borrow_mut().get_or_insert_with(|| NoteElement::new(self)).clone()
    }

    /// The controls drawn now and those scrolled out of sight, as
    /// elements. An assistive app asking for the first time has the model
    /// keep what each frame draws from then on.
    pub(super) fn control_elements(&self) -> Vec<Retained<ControlElement>> {
        let controls = {
            let Ok(mut st) = self.ivars().state.try_borrow_mut() else { return self.ivars().access.controls.borrow().clone() };
            if st.model.access_controls().is_empty() {
                st.model.keep_for_access(true);
                drop(st);
                self.refresh();
                st = match self.ivars().state.try_borrow_mut() {
                    Ok(st) => st,
                    Err(_) => return Vec::new(),
                };
            }
            let mut controls = st.model.access_controls();
            controls.extend(st.model.access_away());
            controls
        };
        elements(Owner::Main(Weak::from(self)), MainThreadMarker::from(self), &self.ivars().access.controls, &controls)
    }

    /// The view's children for VoiceOver: the note, then the controls.
    pub(super) fn access_children(&self) -> Retained<objc2_foundation::NSArray<NSObject>> {
        let mut all: Vec<Retained<NSObject>> = vec![Retained::into_super(Retained::into_super(self.note_element()))];
        all.extend(self.control_elements().into_iter().map(|c| Retained::into_super(Retained::into_super(c))));
        array(all)
    }

    /// The element at a point on the screen: a control, or else the note.
    pub(super) fn access_hit(&self, point: NSPoint) -> Retained<NSObject> {
        if let Some(window) = self.window() {
            let p = self.convertPoint_fromView(window.convertPointFromScreen(point), None);
            if let Some(c) = hit(&self.ivars().access.controls.borrow(), p.x as f32, p.y as f32) {
                return c;
            }
        }
        Retained::into_super(Retained::into_super(self.note_element()))
    }

    /// A frame was drawn: when an assistive app listens and the controls
    /// drawn changed, it is told (after the frame, with nothing borrowed).
    pub(super) fn access_drawn(&self, model: &kolada_app::Model) {
        let controls = model.access_controls();
        if controls.is_empty() && self.ivars().access.names.borrow().is_empty() {
            return;
        }
        let names: Vec<String> = controls.into_iter().map(|c| c.label).collect();
        if *self.ivars().access.names.borrow() != names {
            *self.ivars().access.names.borrow_mut() = names;
            unsafe { self.performSelector_withObject_afterDelay(objc2::sel!(accessMoved), None, 0.0) };
        }
    }

    /// A list scrolled to show a control out of sight, drawn at once.
    pub(super) fn access_scroll(&self, away: Away) {
        self.ivars().state.borrow_mut().model.access_scroll_to(away);
        self.refresh();
    }

    /// Where `target` is drawn now (DIPs), if it is.
    pub(super) fn access_rect(&self, target: Target) -> Option<[f32; 4]> {
        let st = self.ivars().state.try_borrow().ok()?;
        st.model.access_controls().into_iter().find(|c| c.target == target).map(|c| [c.rect.left, c.rect.top, c.rect.right, c.rect.bottom])
    }

    /// A control pressed by VoiceOver: a click at (x, y), as the pointer's.
    pub(super) fn access_press(&self, x: f32, y: f32) {
        let changed = self.ivars().state.borrow_mut().model.pointer_down(x, y, 1, false);
        self.edited(changed);
        self.serve();
        self.show_requested_menu();
        let (changed, effects) = self.ivars().state.borrow_mut().model.pointer_up_at(x, y);
        self.edited(changed);
        self.redraw_if(changed);
        self.run_effects(effects);
        self.serve();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_is_a_checkbox_the_choice_in_force_selected_a_folder_open_or_closed() {
        assert_eq!(role_of(ControlKind::Task(true)), ("AXCheckBox", Some(true)));
        assert_eq!(role_of(ControlKind::Task(false)), ("AXCheckBox", Some(false)));
        assert_eq!(role_of(ControlKind::Selected), ("AXRadioButton", Some(true)));
        assert_eq!(role_of(ControlKind::Folder(true)), ("AXDisclosureTriangle", Some(true)));
        assert_eq!(role_of(ControlKind::Folder(false)), ("AXDisclosureTriangle", Some(false)));
        for kind in [ControlKind::Button, ControlKind::Heading, ControlKind::Field] {
            assert_eq!(role_of(kind), ("AXButton", None));
        }
    }

    /// Drawn again, the controls keep the elements VoiceOver knows, by
    /// what they stand for and in order: its place is not lost to a redraw.
    #[test]
    fn controls_drawn_again_keep_their_elements() {
        let old = ["Search", "New note", "Task", "Task", "Settings"];
        assert_eq!(reuse(&old, &["Search", "New note", "Task", "Task", "Settings"]), [Some(0), Some(1), Some(2), Some(3), Some(4)]);
        assert_eq!(reuse(&old, &["New note", "Task", "Plan", "Settings"]), [Some(1), Some(2), None, Some(4)], "one gone, one new");
        assert_eq!(reuse(&[], &["Search"]), [None]);
    }
}
