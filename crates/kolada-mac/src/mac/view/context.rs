//! The view's smart paste and its right-click menu: thin AppKit calls around
//! what the model decides (`model/paste.rs`, `model/menu.rs`). Everything
//! that can be tested without a screen is tested there.

use objc2::{DefinedClass, MainThreadMarker};
use objc2::runtime::AnyObject;
use objc2::sel;
use objc2_app_kit::{NSEvent, NSMenu, NSMenuItem, NSPasteboard};
use objc2_foundation::{NSPoint, NSString};

use super::{KoladaView, clipboard};
use crate::mac::model::menu::{MenuAction, MenuItem};

/// The pasteboard's HTML, if it has any (as Safari, Chrome, Pages, Mail and
/// Word put it), and the page it was copied from when the source says (the
/// address Chrome names).
fn html() -> Option<(String, Option<String>)> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let page = pasteboard.stringForType(&NSString::from_str("public.html"))?.to_string();
    let source = pasteboard.stringForType(&NSString::from_str("org.chromium.source-url")).map(|s| s.to_string()).filter(|s| !s.is_empty());
    Some((page, source))
}

/// Whether the pasteboard holds formatted text (for the menu's "Paste as
/// Plain Text").
fn has_html() -> bool {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.availableTypeFromArray(&objc2_foundation::NSArray::from_retained_slice(&[NSString::from_str("public.html")])).is_some()
}

impl KoladaView {
    /// Cmd+V, or Cmd+Shift+V (`plain`): formatted text comes in as Markdown
    /// where the caret can take it, else the text as it is.
    pub(super) fn paste_clipboard(&self, plain: bool) {
        // A picture and no text (a screenshot): saved and embedded.
        let Some(text) = clipboard() else {
            if !plain {
                if let Some(png) = super::arrive::picture_on_pasteboard(&NSPasteboard::generalPasteboard()) {
                    let changed = self.ivars().state.borrow_mut().model.paste_picture(&png);
                    self.edited(changed);
                }
            }
            return;
        };
        let changed = {
            let mut st = self.ivars().state.borrow_mut();
            if plain { st.model.paste_plain(&text) } else { st.model.paste(&text, html) }
        };
        self.edited(changed);
    }

    /// A right-click: the model says what the menu holds (a misspelled
    /// word's fixes first, then a link, a table's rows and columns, the
    /// clipboard, formatting and the note; or a file tree row's, or a
    /// deleted note's), and it is shown at the pointer.
    pub(super) fn show_context_menu(&self, event: &NSEvent) {
        let p = self.point(event);
        let (spelling, items) = {
            let mut st = self.ivars().state.borrow_mut();
            // The menu's selection is the word, so Cut and Copy are for it.
            let spelling = st.model.spell_menu_items(p.x as f32, p.y as f32).unwrap_or_default();
            (spelling, st.model.context_menu(p.x as f32, p.y as f32, has_html()))
        };
        // The caret may have moved to where the click was.
        self.setNeedsDisplay(true);
        if items.is_empty() && spelling.is_empty() {
            return;
        }
        let menu = self.build_menu(&spelling, &items);
        // Returns once an item has been picked (its action is sent first) or the menu is dismissed.
        NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self);
    }

    /// A menu the model opened from a button (a row's three dots, the send
    /// button): shown under it.
    pub(super) fn show_requested_menu(&self) {
        let Some(req) = self.ivars().state.borrow_mut().model.take_menu_request() else { return };
        let menu = self.build_menu(&[], &req.items);
        // A menu to the left of the window's right edge goes leftward.
        let width = menu.size().width;
        let right_edge = self.bounds().size.width;
        let x = (req.x as f64).min(right_edge - width - 8.0).max(4.0);
        menu.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(x, req.y as f64), Some(self));
    }

    /// An NSMenu of the model's items, each sending `contextItem:` with its
    /// index; spelling's fixes first.
    fn build_menu(&self, spelling: &[kolada_ui::scene::MenuItem], items: &[MenuItem]) -> objc2::rc::Retained<NSMenu> {
        let mtm = MainThreadMarker::from(self);
        let menu = NSMenu::new(mtm);
        let mut actions = Vec::new();
        let mut add = |label: &str, key: &str, divider: bool, action: MenuAction| {
            if divider {
                menu.addItem(&NSMenuItem::separatorItem(mtm));
            }
            let entry = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), &NSString::from_str(label), Some(sel!(contextItem:)), &NSString::from_str(key)) };
            entry.setTag(actions.len() as isize);
            unsafe { entry.setTarget(Some(self)) };
            menu.addItem(&entry);
            actions.push(action);
        };
        for (i, it) in spelling.iter().enumerate() {
            add(&it.label, "", it.divider, MenuAction::Spell(i));
        }
        for (i, it) in items.iter().enumerate() {
            add(&it.label, it.key, it.divider || (i == 0 && !spelling.is_empty()), it.action.clone());
        }
        *self.ivars().menu.borrow_mut() = actions;
        menu
    }

    /// An item of a menu was picked.
    pub(super) fn pick_context_item(&self, sender: Option<&AnyObject>) {
        let Some(tag) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()).map(|i| i.tag()) else { return };
        let Some(action) = usize::try_from(tag).ok().and_then(|i| self.ivars().menu.borrow().get(i).cloned()) else { return };
        match action {
            MenuAction::Cut => self.cut(sel!(cut:), None),
            MenuAction::Copy => self.copy(sel!(copy:), None),
            MenuAction::Paste => self.paste_clipboard(false),
            MenuAction::PastePlain => self.paste_clipboard(true),
            MenuAction::SelectAll => self.select_all(sel!(selectAll:), None),
            MenuAction::Spell(i) => {
                let changed = self.ivars().state.borrow_mut().model.spell_pick(i);
                self.edited(changed);
            }
            action => {
                let (changed, effects) = self.ivars().state.borrow_mut().model.run_menu(action);
                self.edited(changed);
                self.redraw_if(true);
                self.run_effects(effects);
            }
        }
    }
}
