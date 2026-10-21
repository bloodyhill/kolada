//! The Linux shell: the window and its event loop (`app`), what keys mean
//! (`keys`), putting the frame on screen (`blit`), and what Linux gives the
//! app model (`services`: `fonts`, `images`, `web`, `watch`, `bin`, `when`).
//! The model itself is `kolada-app`.

mod app;
mod appearance;
mod arrive;
mod bin;
mod blit;
mod clipboard;
mod expose;
mod fonts;
mod images;
mod instance;
mod keys;
mod notify;
mod outside;
mod portal;
mod print;
mod services;
mod spell;
mod trace;
mod updates;
mod watch;
mod web;
mod when;

use std::path::{Path, PathBuf};

use kolada_app::Model;
use kolada_app::launch;

/// What the command line asks for.
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    /// A vault folder or a note to open (`--vault <dir>`, or a path on its
    /// own; else `KOLADA_VAULT`, else the one last open, else
    /// `~/Documents/Kolada`).
    pub vault: Option<PathBuf>,
    /// Draw one frame to this PNG and exit, with no window.
    pub dump: Option<PathBuf>,
    pub dark: bool,
    /// With `--dump`: the frame's size in pixels, and pixels per DIP.
    pub size: (u32, u32),
    pub scale: f32,
    /// With `--dump`: the note to open (vault-relative, with `.md`), and
    /// what to open over it (`options`, `palette`).
    pub note: Option<String>,
    pub open: Option<String>,
    /// Use X11 even where Wayland is offered.
    pub x11: bool,
    /// A new note, or the search (`--new`, `--search`: the app list's
    /// actions on Kolada's icon), in the Kolada running or this one.
    pub new: bool,
    pub search: bool,
}

pub fn parse(args: impl IntoIterator<Item = std::ffi::OsString>) -> Args {
    let mut a = Args { size: (2000, 1440), scale: 2.0, ..Args::default() };
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.to_string_lossy().as_ref() {
            "--vault" => a.vault = it.next().map(PathBuf::from),
            "--dump" => a.dump = it.next().map(PathBuf::from),
            "--dark" => a.dark = true,
            "--x11" => a.x11 = true,
            "--new" => a.new = true,
            "--search" => a.search = true,
            "--size" => {
                let s = it.next().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if let Some((w, h)) = s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
                    a.size = (w, h);
                }
            }
            "--scale" => a.scale = it.next().and_then(|s| s.to_string_lossy().parse().ok()).unwrap_or(2.0),
            "--note" => a.note = it.next().map(|s| s.to_string_lossy().into_owned()),
            "--open" => a.open = it.next().map(|s| s.to_string_lossy().into_owned()),
            s if s.starts_with('-') => {}
            _ => a.vault = Some(PathBuf::from(arg)),
        }
    }
    a
}

/// Runs the app until it quits.
pub fn run() {
    trace::mark("main");
    let args = parse(std::env::args_os().skip(1));
    match &args.dump {
        Some(path) => {
            if let Err(e) = dump(&args, path) {
                eprintln!("kolada: could not write {}: {e}", path.display());
                std::process::exit(1);
            }
        }
        None => app::run(args),
    }
}

/// The model as the Linux shell sets it up: its OS, and the window buttons
/// drawn by the chrome (the window has no title bar of its own).
pub fn linux_model(mut model: Model) -> Model {
    model.set_platform(kolada_ui::scene::Platform::Linux);
    model.set_os_buttons(false);
    // No toolkit to show menus in: the chrome draws them.
    model.set_drawn_menus(true);
    model
}

/// One frame of the vault, as the window would show it, written as a PNG.
fn dump(args: &Args, path: &Path) -> std::io::Result<()> {
    let mut model = linux_model(Model::with_services(services::services(), launch::vault(args.vault.clone()), args.dark));
    if let Some(rel) = &args.note {
        model.open(rel);
    }
    let (w, h, scale) = (args.size.0, args.size.1, args.scale);
    model.frame(w, h, scale);
    match args.open.as_deref() {
        Some("options") => {
            model.toggle_options();
        }
        Some("palette") => {
            model.open_palette();
            model.insert("th");
        }
        _ => {}
    }
    // Pictures decode on their worker: give it a moment, then draw again.
    model.start_pictures(|| {});
    model.frame(w, h, scale);
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        if model.poll_pictures() {
            std::thread::sleep(std::time::Duration::from_millis(250));
            break;
        }
    }
    model.frame(w, h, scale);
    write_png(path, model.surface().pixels(), w, h)
}

/// `pixels` (0x00RRGGBB, rows from the top) as a PNG.
pub fn write_png(path: &Path, pixels: &[u32], w: u32, h: u32) -> std::io::Result<()> {
    let mut rgb = Vec::with_capacity(pixels.len() * 3);
    for p in pixels {
        rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    image::save_buffer(path, &rgb, w, h, image::ExtendedColorType::Rgb8).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests;
