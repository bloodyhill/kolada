//! Kolada for the Mac: the AppKit shell around the shared engine. It owns the
//! window, the menu bar, input and putting pixels on screen; what is drawn
//! and what typing does come from the shared crates.

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
fn main() {
    mac::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("kolada-mac runs on macOS only");
}
