//! What the model asks of the Mac outside the app (`model/share.rs`
//! `Effect`): the clipboard, formatted or plain; a save panel and the file
//! written (a PDF printed from the note's web page by AppKit's own text
//! system, no browser needed); a page in the browser or an app's link; a
//! file shown in the Finder; a command started in Terminal.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, msg_send};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString, NSPopUpButton, NSPrintInfo, NSPrintOperation, NSSavePanel, NSTextView, NSWorkspace};
use objc2_foundation::{NSArray, NSAttributedString, NSData, NSPoint, NSRect, NSSize, NSString, NSURL};

use super::NotsyView;
use crate::mac::model::share::{Effect, Export};

/// `s` in single quotes for the shell.
fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

/// Opens a URL with whatever the Mac opens it with.
fn open_url(url: &str) -> bool {
    match NSURL::URLWithString(&NSString::from_str(url)) {
        Some(u) => NSWorkspace::sharedWorkspace().openURL(&u),
        None => false,
    }
}

/// Puts `text` on the clipboard, and `html` with it when given (apps that
/// take formatting read that, the rest the text).
pub fn set_clipboard(text: &str, html: Option<&str>) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    unsafe {
        if let Some(html) = html {
            pasteboard.setString_forType(&NSString::from_str(html), NSPasteboardTypeHTML);
        }
        pasteboard.setString_forType(&NSString::from_str(text), NSPasteboardTypeString);
    }
}

/// A Terminal window in `dir` running `command`: a `.command` file, which
/// Terminal opens and runs (no permission to control Terminal is needed).
fn in_terminal(dir: &Path, command: &str) -> bool {
    let script = std::env::temp_dir().join(format!("notsy-{}-{}.command", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())));
    let body = format!("#!/bin/zsh -l\ncd {} || exit 1\nrm -f {}\n{command}\n", quoted(&dir.to_string_lossy()), quoted(&script.to_string_lossy()));
    if std::fs::write(&script, body).is_err() || std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).is_err() {
        return false;
    }
    NSWorkspace::sharedWorkspace().openURL(&file_url(&script))
}

/// Prints the web page `html` to a PDF at `out`, as AppKit lays out HTML:
/// paged, with margins, right-to-left text kept.
fn print_pdf(mtm: MainThreadMarker, html: &str, out: &Path) -> bool {
    let data = NSData::with_bytes(html.as_bytes());
    let text: Option<Retained<NSAttributedString>> = unsafe { msg_send![NSAttributedString::alloc(), initWithHTML: &*data, documentAttributes: std::ptr::null_mut::<AnyObject>()] };
    let Some(text) = text else { return false };
    let info = NSPrintInfo::sharedPrintInfo();
    let info: Retained<NSPrintInfo> = unsafe { msg_send![&*info, copy] };
    let margin = 54.0;
    info.setLeftMargin(margin);
    info.setRightMargin(margin);
    info.setTopMargin(margin);
    info.setBottomMargin(margin);
    let paper = info.paperSize();
    let width = paper.width - 2.0 * margin;
    let view = NSTextView::initWithFrame(mtm.alloc(), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, paper.height)));
    // TextKit 1, which asking for the layout manager switches the view to:
    // TextKit 2 (the default) stacks a table's cells one under another and
    // draws each list item's bullet twice (seen in a PDF exported on the Mac).
    let _ = unsafe { view.layoutManager() };
    unsafe {
        if let Some(storage) = view.textStorage() {
            let _: () = msg_send![&*storage, setAttributedString: &*text];
        }
    }
    view.setVerticallyResizable(true);
    view.sizeToFit();
    info.setHorizontalPagination(objc2_app_kit::NSPrintingPaginationMode::Fit);
    info.setVerticalPagination(objc2_app_kit::NSPrintingPaginationMode::Automatic);
    unsafe {
        info.setJobDisposition(objc2_app_kit::NSPrintSaveJob);
        let dict = info.dictionary();
        let _: () = msg_send![&*dict, setObject: &*file_url(out), forKey: objc2_app_kit::NSPrintJobSavingURL];
    }
    let op = NSPrintOperation::printOperationWithView_printInfo(&view, &info);
    op.setShowsPrintPanel(false);
    op.setShowsProgressPanel(false);
    op.runOperation() && out.is_file()
}

/// Files or a folder chosen in an open panel: none if it was cancelled.
pub fn choose(mtm: MainThreadMarker, pick: crate::mac::model::transfer::Pick) -> Vec<PathBuf> {
    use crate::mac::model::transfer::Pick;
    let panel = objc2_app_kit::NSOpenPanel::openPanel(mtm);
    let (title, files, many) = match pick {
        Pick::Files => ("Import Notes", true, true),
        Pick::Folder => ("Import a Folder of Notes", false, false),
        Pick::Vault => ("Open a Folder of Notes", false, false),
    };
    panel.setTitle(Some(&NSString::from_str(title)));
    panel.setCanChooseFiles(files);
    panel.setCanChooseDirectories(!files);
    panel.setAllowsMultipleSelection(many);
    if pick == Pick::Vault {
        panel.setCanCreateDirectories(true);
        panel.setPrompt(Some(&NSString::from_str("Open")));
    }
    if panel.runModal() != objc2_app_kit::NSModalResponseOK {
        return Vec::new();
    }
    panel.URLs().iter().filter_map(|u| u.path()).map(|p| PathBuf::from(p.to_string())).collect()
}

impl NotsyView {
    /// Does what the model asked, in order.
    pub(super) fn run_effects(&self, effects: Vec<Effect>) {
        for e in effects {
            self.run_effect(e);
        }
    }

    fn run_effect(&self, effect: Effect) {
        match effect {
            Effect::Clipboard(text, said) => {
                set_clipboard(&text, None);
                if !said.is_empty() {
                    self.ivars().state.borrow_mut().model.whisper(&said);
                }
            }
            Effect::Rich { text, html } => set_clipboard(&text, Some(&html)),
            Effect::Export { name } => self.export_panel(&name, None),
            Effect::Url(url) => {
                if !open_url(&url) {
                    self.ivars().state.borrow_mut().model.whisper("Could not open it");
                }
            }
            Effect::Reveal(path) => NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[file_url(&path)])),
            Effect::Terminal { dir, command } => {
                if !in_terminal(&dir, &command) {
                    self.ivars().state.borrow_mut().model.whisper("Could not open Terminal");
                }
            }
            Effect::Window(path) => crate::mac::app::open_note_window(MainThreadMarker::from(self), &path),
        }
        self.redraw_if(true);
    }

    /// Export Note: a save panel, its kind chosen in it (Markdown first, a
    /// PDF, a web page), then the file written.
    pub(super) fn export_panel(&self, name: &str, preset: Option<Export>) {
        let mtm = MainThreadMarker::from(self);
        let panel = NSSavePanel::savePanel(mtm);
        panel.setTitle(Some(&NSString::from_str("Export Note")));
        panel.setNameFieldStringValue(&NSString::from_str(&format!("{name}.md")));
        panel.setCanCreateDirectories(true);
        panel.setAllowsOtherFileTypes(true);
        let kinds = NSPopUpButton::initWithFrame_pullsDown(mtm.alloc(), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(180.0, 26.0)), false);
        for k in Export::ALL {
            kinds.addItemWithTitle(&NSString::from_str(k.name()));
        }
        if let Some(i) = preset.and_then(|p| Export::ALL.iter().position(|k| *k == p)) {
            kinds.selectItemAtIndex(i as isize);
            panel.setNameFieldStringValue(&NSString::from_str(&format!("{name}.{}", Export::ALL[i].ext())));
        }
        panel.setAccessoryView(Some(&kinds));
        if panel.runModal() != objc2_app_kit::NSModalResponseOK {
            return;
        }
        let Some(path) = panel.URL().and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string())) else { return };
        let kind = Export::ALL[(kinds.indexOfSelectedItem().max(0) as usize).min(Export::ALL.len() - 1)];
        // The name the kind's own, whatever was typed.
        let path = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(kind.ext())) { path } else { path.with_extension(kind.ext()) };
        let page = self.ivars().state.borrow().model.export_page(kind);
        let Some(page) = page else { return };
        let ok = match kind {
            Export::Pdf => print_pdf(mtm, &page, &path),
            _ => std::fs::write(&path, page).is_ok(),
        };
        self.ivars().state.borrow_mut().model.exported(&path, ok);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_quoted_for_the_shell_whatever_their_names() {
        assert_eq!(quoted("/Users/a/My Notes"), "'/Users/a/My Notes'");
        assert_eq!(quoted("it's"), "'it'\\''s'");
        assert_eq!(quoted("ملاحظات"), "'ملاحظات'");
    }
}
