//! Kolada-Setup.exe: the Kolada it carries installs itself, for the user
//! running it, with no admin. This program only unpacks it to the temp
//! folder and runs it with `--setup`, which shows the installer's screen and
//! does the work (`kolada-win/src/setup.rs`); `--quiet` installs with the
//! usual choices and no screen.
//!
//! The files travel inside this one, appended to it (`kolada_core::pack`):
//! `kolada-setup --pack <out.exe> <file>...` makes an installer from the
//! stub and the files (`tools/setup.ps1` does it for a release).

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    std::process::exit(win::run(std::env::args_os().skip(1).collect()));
}

#[cfg(not(windows))]
fn main() {
    eprintln!("kolada-setup runs on Windows only");
}
