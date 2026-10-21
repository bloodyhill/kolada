//! Kolada for Linux: a thin shell around the shared app model. It owns the
//! window (winit: X11 or Wayland), puts the model's pixels on screen
//! (softbuffer), and turns keys, input methods, the pointer and the wheel
//! into the model's calls; what is drawn and what typing does come from the
//! shared crates.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() {
    linux::run();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("kolada-linux runs on Linux only");
}
