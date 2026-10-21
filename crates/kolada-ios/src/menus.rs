//! The menus, as the Mac has them (`kolada-mac` `view/context.rs`), shown
//! the iOS way: a long press (or a trackpad's secondary click) on the note
//! gives its menu in the edit menu over the selection, on a row of the side
//! menu the row's menu; a row's three dots and the send button open theirs
//! as the model asks (an event, `menu	x	y`). The model says what a menu
//! holds; the shell shows the lines `kolada_menu` gives and sends back the
//! index picked. What iOS cannot do (a note's own window, the Finder) is
//! left out; the pasteboard's items come back to the shell as events.

use std::ffi::{CString, c_char};

use kolada_app::model::menu::{MenuAction, MenuItem};
use kolada_ui::scene::Target;

use crate::{KoladaApp, app};

/// What an item picked does.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// The `i`th fix of a misspelled word, Ignore or Learn.
    Spell(usize),
    Model(MenuAction),
}

/// Items iOS has no use for: a note's window of its own (one window
/// here), the Finder.
fn on_ios(item: &MenuItem) -> bool {
    !matches!(item.action, MenuAction::OpenWindow(_) | MenuAction::Reveal(_))
}

/// A menu's lines (`label	key	divider`, divider 1 where a group starts) and
/// what each does; the spelling's first.
pub fn build(spelling: &[kolada_ui::scene::MenuItem], items: &[MenuItem]) -> (String, Vec<Pick>) {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut lines = String::new();
    let mut picks = Vec::new();
    for (i, it) in spelling.iter().enumerate() {
        lines.push_str(&format!("{}\t\t{}\n", clean(&it.label), u8::from(it.divider && i > 0)));
        picks.push(Pick::Spell(i));
    }
    let kept: Vec<&MenuItem> = items.iter().filter(|i| on_ios(i)).collect();
    for (i, it) in kept.iter().enumerate() {
        let divider = (it.divider && (i > 0 || !spelling.is_empty())) || (i == 0 && !spelling.is_empty());
        lines.push_str(&format!("{}\t{}\t{}\n", clean(&it.label), it.key, u8::from(divider)));
        picks.push(Pick::Model(it.action.clone()));
    }
    (lines, picks)
}

fn give(a: &mut KoladaApp, lines: String, picks: Vec<Pick>) -> *mut c_char {
    a.menu = picks;
    if a.menu.is_empty() {
        return std::ptr::null_mut();
    }
    CString::new(lines).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// The menu for what is at (x, y) DIPs (a long press, a secondary click):
/// its lines (see `build`), a C string to free; null when there is none.
/// On the note, the caret moves to the press first unless it is in the
/// selection, so the items are for the place pressed. `has_html`: the
/// pasteboard holds formatted text.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_menu(h: *mut KoladaApp, x: f32, y: f32, has_html: bool) -> *mut c_char {
    let a = app(h);
    let spelling = a.model.spell_menu_items(x, y).unwrap_or_default();
    let mut items = a.model.context_menu(x, y, has_html);
    // The selection bar is up over the words with their formatting: the
    // menu under them keeps to the pasteboard and the note, not the same twice.
    if a.model.selection_bar_shown() {
        items.retain(|i| !matches!(i.action, MenuAction::Format(_) | MenuAction::Highlight));
    }
    let (lines, picks) = build(&spelling, &items);
    give(a, lines, picks)
}

/// The menu the model opened from one of its buttons (a row's three dots,
/// the send button), after a `menu` event: its lines, as `kolada_menu`.
#[unsafe(no_mangle)]
pub extern "C" fn kolada_menu_requested(h: *mut KoladaApp) -> *mut c_char {
    let a = app(h);
    let items = a.requested.take().unwrap_or_default();
    let (lines, picks) = build(&[], &items);
    give(a, lines, picks)
}

/// The `index`th item of the menu last given was picked; whether the
/// screen changes. The pasteboard's items come back as events for the
/// shell (`edit	cut|copy|paste|paste-plain|select-all`).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_menu_pick(h: *mut KoladaApp, index: usize) -> bool {
    let a = app(h);
    let Some(pick) = a.menu.get(index).cloned() else { return false };
    a.menu.clear();
    match pick {
        Pick::Spell(i) => a.model.spell_pick(i),
        Pick::Model(edit @ (MenuAction::Cut | MenuAction::Copy | MenuAction::Paste | MenuAction::PastePlain)) => {
            let name = match edit {
                MenuAction::Cut => "cut",
                MenuAction::Copy => "copy",
                MenuAction::Paste => "paste",
                _ => "paste-plain",
            };
            a.events.push_back(format!("edit\t{name}"));
            false
        }
        Pick::Model(MenuAction::SelectAll) => a.model.select_all(),
        Pick::Model(action) => {
            let (changed, effects) = a.model.run_menu(action);
            a.queue(effects);
            changed
        }
    }
}

/// What a long press at (x, y) DIPs means: 1 on the note's text (it
/// selects there, then its menu), 2 where a menu of its own opens at once
/// (a row of the side menu, a deleted note), 0 elsewhere (a tap's place).
#[unsafe(no_mangle)]
pub extern "C" fn kolada_press_kind(h: *mut KoladaApp, x: f32, y: f32) -> u8 {
    match app(h).model.target_at(x, y) {
        Some(Target::Note) => 1,
        Some(Target::Row(_) | Target::RowMenu(_) | Target::Recent(_) | Target::Favorite(_) | Target::TrashRow(_) | Target::TrashRestore(_)) => 2,
        _ => 0,
    }
}
