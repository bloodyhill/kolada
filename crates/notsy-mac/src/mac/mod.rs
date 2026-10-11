//! The Mac shell: application, window, view, and what the Mac gives the app
//! model. The model itself (the whole state of the app) is `notsy-app`; the
//! files here that the model's `Services` need (`bin`, `fonts`, `images`,
//! `spell`, `vision`, `watch`, `web`, `when`) are the Mac's half of it.

mod app;
mod bin;
mod fonts;
mod hotkey;
mod images;
mod installed;
mod later;
mod services;
mod spell;
#[cfg(test)]
mod speed;
mod status;
mod trace;
mod watch;
mod web;
mod view;
mod vision;
mod when;

// Where the shell's files have always found the model.
pub(crate) use notsy_app::launch as note;
pub(crate) use notsy_app::model;

use std::path::PathBuf;

/// What the command line asks for.
pub struct Args {
    /// A vault folder or a note to open (else `NOTSY_VAULT`, else
    /// `~/Documents/Notsy`): `note::given` says which.
    pub vault: Option<PathBuf>,
    /// Draw one frame to this BMP and exit, with no window (to see what a
    /// frame looks like without a screen).
    pub dump: Option<PathBuf>,
    pub dark: bool,
    /// With `--dump`: open the note's past, a made-up one, as well.
    pub scrub: bool,
    pub size: (u32, u32),
    pub scale: f32,
    /// With `--dump`: what to open before the frame (`options`, `highlights`).
    pub open: Option<String>,
    /// With `--dump`: the note to open (vault-relative, with `.md`).
    pub note: Option<String>,
}

fn args() -> Args {
    let mut a = Args { vault: None, dump: None, dark: false, scrub: false, size: (2000, 1440), scale: 2.0, open: None, note: None };
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        match arg.to_string_lossy().as_ref() {
            "--dump" => a.dump = it.next().map(PathBuf::from),
            "--dark" => a.dark = true,
            "--scrub" => a.scrub = true,
            "--size" => {
                let s = it.next().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if let Some((w, h)) = s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
                    a.size = (w, h);
                }
            }
            "--note" => a.note = it.next().map(|s| s.to_string_lossy().into_owned()),
            "--open" => a.open = it.next().map(|s| s.to_string_lossy().into_owned()),
            "--scale" => a.scale = it.next().and_then(|s| s.to_string_lossy().parse().ok()).unwrap_or(2.0),
            // macOS adds a few of its own (-psn_..., -NS...): not ours.
            s if s.starts_with('-') => {}
            _ => a.vault = Some(PathBuf::from(arg)),
        }
    }
    a
}

/// Runs the app until it quits.
pub fn run() {
    let args = args();
    match &args.dump {
        Some(path) => {
            let mut model = model::Model::with_services(services::services(), note::vault(args.vault.clone()), args.dark);
            if let Some(rel) = &args.note {
                model.open(rel);
            }
            if args.scrub {
                model.demo_past();
            }
            model.frame(args.size.0, args.size.1, args.scale);
            match args.open.as_deref() {
                Some("options") => {
                    model.toggle_options();
                }
                Some("highlights") => {
                    model.open_highlights();
                }
                // A picture and a PDF dragged over the note, before they are let go.
                Some("drag") => {
                    let files = [PathBuf::from("/tmp/cat.png"), PathBuf::from("/tmp/Invoice.pdf")];
                    model.drag_enter(&files, 900.0, 420.0);
                }
                // Related notes in the margin, the first peeked at.
                Some("related") => {
                    model.start_index();
                    for _ in 0..400 {
                        if model.corpus_ready() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    model.update_related();
                    model.frame(args.size.0, args.size.1, args.scale);
                    model.peek_first();
                }
                // The command palette, its commands for "th".
                Some("palette") => {
                    model.open_palette();
                    model.insert("th");
                    model.frame(args.size.0, args.size.1, args.scale);
                }
                // Favorites and Recent in the side menu.
                Some("kept") => {
                    model.demo_kept();
                    model.frame(args.size.0, args.size.1, args.scale);
                }
                // The Today card, open at the red dot, over what is due in the vault.
                Some("due") => {
                    model.start_index();
                    for _ in 0..400 {
                        if model.corpus_ready() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    model.due_ready();
                    model.frame(args.size.0, args.size.1, args.scale);
                    model.open_agenda();
                    model.settle_agenda();
                }
                // The minimal window: the note alone.
                Some("minimal") => {
                    model.set_minimal(true);
                }
                // The social preview for X.
                Some("social") => {
                    model.show_social(Some(notsy_core::social::Platform::X));
                }
                // A note open over the page.
                Some(o) if o.starts_with("popup:") => {
                    model.open_note_over(&o["popup:".len()..]);
                }
                // Scrolled down, the pointer at the right edge: the scroll bar and its map.
                Some("scrolled") => {
                    model.wheel(900.0, true);
                    model.frame(args.size.0, args.size.1, args.scale);
                    model.pointer_moved(args.size.0 as f32 / args.scale - 8.0, 400.0);
                }
                _ => {}
            }
            // Pictures decode on their worker: give it a moment, then draw again.
            model.start_pictures(|| {});
            model.frame(args.size.0, args.size.1, args.scale);
            for _ in 0..100 {
                std::thread::sleep(std::time::Duration::from_millis(10));
                if model.poll_pictures() {
                    // Let it fade in.
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    break;
                }
            }
            model.frame(args.size.0, args.size.1, args.scale);
            model.surface().dump(path).expect("the frame is written");
        }
        None => app::run(args),
    }
}
