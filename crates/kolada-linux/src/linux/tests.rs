//! The Linux shell without a display: what it hands the model, where it
//! keeps its data, and that a window's frame is drawn (the same model,
//! scene and surface the window shows, through `--dump`).

use super::*;
use std::time::Duration;

fn vault(name: &str, notes: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kolada-linux-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (rel, text) in notes {
        std::fs::write(dir.join(rel), text).unwrap();
    }
    dir
}

#[test]
fn the_command_line_names_the_vault_a_note_or_a_dump() {
    let os = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
    let a = parse(os(&["--vault", "/home/a/Notes"]));
    assert_eq!(a.vault, Some(PathBuf::from("/home/a/Notes")));
    let a = parse(os(&["/home/a/Notes/Plan.md", "--x11"]));
    assert_eq!(a.vault, Some(PathBuf::from("/home/a/Notes/Plan.md")), "a path alone is a vault or a note (launch::given says which)");
    assert!(a.x11);
    let a = parse(os(&["--dump", "/tmp/f.png", "--size", "800x600", "--scale", "1.5", "--dark", "--note", "Plan.md"]));
    assert_eq!((a.dump, a.size, a.scale, a.dark, a.note.as_deref()), (Some(PathBuf::from("/tmp/f.png")), (800, 600), 1.5, true, Some("Plan.md")));
    assert_eq!(parse(os(&[])).size, (2000, 1440));
    // The app list's actions on Kolada's icon (the .desktop file's).
    let (n, s) = (parse(os(&["--new"])), parse(os(&["--search"])));
    assert!(n.new && !n.search && s.search && !s.new && n.vault.is_none());
    assert!(parse(os(&["--updated"])).vault.is_none(), "an update's restart names nothing to open");
}

#[test]
fn a_frame_is_drawn_for_a_window_with_no_display() {
    let v = vault("frame", &[("Plan.md", "# Plan\n\nThe quick brown fox jumps over the lazy dog.\n")]);
    let mut m = linux_model(Model::with_services(services::services(), v.clone(), true));
    let (w, h, scale) = (1600, 1000, 2.0);
    m.frame(w, h, scale);
    let px = m.surface().pixels();
    assert_eq!(px.len(), (w * h) as usize);
    assert_eq!(px[0], 0x1A1A1A, "graphite, top left");
    assert!(px.iter().filter(|&&p| p != px[0]).count() > 5_000, "the note and the chrome are drawn");
    // The window has no title bar: the chrome draws its buttons, top right.
    let (cx, cy) = ((w as f32 / scale) - 20.0, 16.0);
    let button = (0..120).map(|i| m.window_button(cx - i as f32, cy)).find(Option::is_some).flatten();
    assert_eq!(button, Some(kolada_app::model::WindowButton::Close), "the close button is the rightmost");
    // Typing repaints only what changed: the line typed on.
    m.command("moveToEndOfDocument:");
    m.frame(w, h, scale);
    assert!(m.commit_marked("x"));
    m.frame(w, h, scale);
    let damage = m.surface().last_damage.clone().expect("a partial frame");
    let area: i64 = damage.iter().map(|&(l, t, r, b)| ((r - l) * (b - t)) as i64).sum();
    assert!(area > 0 && area * 20 < (w * h) as i64, "a keystroke repaints a sliver of the window: {area} px of {}", w * h);
    // Nothing changed: nothing is painted.
    m.frame(w, h, scale);
    assert_eq!(m.surface().last_damage.as_deref(), Some(&[][..]));
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn menus_are_drawn_by_the_chrome_with_linuxs_keys() {
    let v = vault("menus", &[("Plan.md", "# Plan\n\nThe quick brown fox.\n")]);
    let mut m = linux_model(Model::with_services(services::services(), v.clone(), false));
    m.frame(1600, 1000, 2.0);
    let r = m.place_of(kolada_ui::scene::Target::Note).expect("the note");
    assert!(m.open_context_menu((r.left + r.right) / 2.0, r.top + 40.0, false));
    assert!(m.menu_open(), "a right-click opens a menu the chrome draws");
    assert!(m.take_menu_request().is_none());
    m.frame(1600, 1000, 2.0);
    assert!(m.place_of(kolada_ui::scene::Target::MenuPanel).is_some());
    assert!(m.command("cancelOperation:") && !m.menu_open(), "Escape closes it");
    let _ = std::fs::remove_dir_all(v);
}

/// Every size a Linux window can be, from its least (360 x 320) to a 4K
/// screen, the halves and thirds of common screens side by side among them,
/// at the scales Linux desktops use (1, 1.25, 1.5, 2), in the full window
/// and the minimal one, with the options open and not: the window buttons
/// the chrome draws are all there, inside the window and apart, Close at its
/// right edge; Send and Minimal beside them and clear of them; the note has
/// room; and a right-click's menu near each corner of the note, the send
/// menu and a row's menu open inside the window.
#[test]
fn holds_together_at_every_linux_window_size() {
    use kolada_ui::scene::{Rect, Target};
    let v = vault("sizes", &[("Plan.md", "# Plan\n\nShip the Linux build.\n\n- [ ] One\n- [ ] Two\n"), ("Other.md", "# Other\n\nIts text.\n")]);
    let mut m = linux_model(Model::with_services(services::services(), v.clone(), true));
    let inside = |r: Rect, w: f32, h: f32| r.left >= -0.5 && r.top >= -0.5 && r.right <= w + 0.5 && r.bottom <= h + 0.5;
    let apart = |a: Rect, b: Rect| (a.right.min(b.right) - a.left.max(b.left)).max(0.0) * (a.bottom.min(b.bottom) - a.top.max(b.top)).max(0.0) <= 1.0;
    let mut sizes: Vec<(f32, f32)> = Vec::new();
    for h in [320.0, 480.0, 720.0, 1000.0, 1400.0, 2100.0] {
        for w in (360..=3840).step_by(120) {
            sizes.push((w as f32, h));
        }
    }
    // Side by side: halves and thirds of 1366, 1920, 2560 and 3840 wide screens.
    for (sw, sh) in [(1366.0, 768.0), (1920.0, 1080.0), (2560.0, 1440.0), (3840.0, 2160.0)] {
        for part in [2.0, 3.0] {
            sizes.push(((sw / part as f32).floor(), sh - 40.0));
        }
    }
    let mut checked = 0;
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        for &(w, h) in &sizes {
            for (minimal, options) in [(false, false), (false, true), (true, false)] {
                let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
                m.set_minimal(minimal);
                m.close_options();
                if options {
                    m.toggle_options();
                }
                m.frame(pw, ph, scale);
                let (w, h) = (pw as f32 / scale, ph as f32 / scale);
                let at = format!("{w} x {h} at {scale}x, minimal {minimal}, options {options}");
                let place = |m: &Model, t: Target| m.place_of(t).unwrap_or_else(|| panic!("{t:?} missing, {at}"));
                let buttons = [Target::Minimize, Target::Maximize, Target::Close].map(|t| place(&m, t));
                for (i, b) in buttons.iter().enumerate() {
                    assert!(inside(*b, w, h) && b.width() >= 20.0, "window button {i} {b:?} out of the window, {at}");
                    assert!(buttons[i + 1..].iter().all(|o| apart(*b, *o)), "window buttons overlap, {at}");
                }
                assert!((buttons[2].right - w).abs() <= 1.0, "Close at the right edge, {at}");
                if !options {
                    let send = place(&m, Target::SendTo);
                    let size = place(&m, if minimal { Target::ToMain } else { Target::Minimal });
                    for r in [send, size] {
                        assert!(inside(r, w, h) && buttons.iter().all(|b| apart(r, *b)), "{r:?} out of the window or under its buttons, {at}");
                    }
                    assert!(apart(send, size), "Send and the window's size overlap, {at}");
                    let note = place(&m, Target::Note);
                    assert!(note.width() >= 200.0 && note.height() >= 100.0, "the note has no room {note:?}, {at}");
                    // A right-click near each corner of the note: its menu in the window.
                    let corners = [(note.left + 4.0, note.top + 4.0), (note.right - 4.0, note.top + 4.0), (note.left + 4.0, note.bottom - 4.0), (note.right - 4.0, note.bottom - 4.0)];
                    for (x, y) in corners {
                        if m.target_at(x, y) != Some(Target::Note) {
                            continue;
                        }
                        assert!(m.open_context_menu(x, y, true), "no menu at ({x}, {y}), {at}");
                        m.frame(pw, ph, scale);
                        let menu = place(&m, Target::MenuPanel);
                        // Taller than the window, it starts at its top and shows what fits.
                        let fits = menu.height() + 16.0 <= h;
                        assert!(menu.left >= 7.5 && menu.right <= w - 7.5 && menu.top >= 7.5 && (!fits || menu.bottom <= h - 7.5), "the menu {menu:?} leaves the window, {at}");
                        m.close_menu();
                        m.frame(pw, ph, scale);
                    }
                    // The send menu, hanging from Send.
                    let (sx, sy) = ((send.left + send.right) / 2.0, (send.top + send.bottom) / 2.0);
                    m.pointer_down(sx, sy, 1, false);
                    m.pointer_up_at(sx, sy);
                    m.frame(pw, ph, scale);
                    let menu = place(&m, Target::MenuPanel);
                    assert!(menu.left >= 7.5 && menu.right <= w - 7.5 && menu.top >= send.bottom, "the send menu {menu:?}, {at}");
                    m.close_menu();
                    m.frame(pw, ph, scale);
                }
                checked += 1;
            }
        }
    }
    m.set_minimal(false);
    assert!(checked > 2000, "{checked} layouts");
    let _ = std::fs::remove_dir_all(v);
}

#[test]
fn a_dump_writes_the_frame_as_a_png() {
    let v = vault("dump", &[("Plan.md", "# Plan\n\n- [ ] Ship the Linux build\n")]);
    let out = v.join("frame.png");
    let args = Args { vault: Some(v.clone()), size: (900, 600), scale: 1.0, ..Args::default() };
    dump(&args, &out).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(image::image_dimensions(&out).unwrap(), (900, 600));
    let _ = std::fs::remove_dir_all(v);
}

/// One test for everything that moves the process's environment (HOME and
/// the XDG folders), so no other test sees it half changed.
#[test]
fn data_follows_xdg_and_a_first_run_opens_on_the_welcome_note_in_documents() {
    let root = vault("home", &[]);
    let (home, data, config) = (root.join("home"), root.join("data"), root.join("config"));
    for d in [&home, &data, &config] {
        std::fs::create_dir_all(d).unwrap();
    }
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", &data);
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::remove_var("KOLADA_VAULT");
    }
    // The app's data: in XDG_DATA_HOME, a test instance's apart.
    assert_eq!(kolada_core::data_root(), Some(data.clone()));
    let own = if kolada_core::testing() { "Kolada.test" } else { "Kolada" };
    assert_eq!(kolada_core::app_data_dir(), Some(data.join(own)));
    unsafe { std::env::set_var("XDG_DATA_HOME", "relative/data") };
    assert_eq!(kolada_core::data_root(), Some(home.join(".local/share")), "a relative XDG_DATA_HOME is ignored");
    unsafe { std::env::set_var("XDG_DATA_HOME", &data) };
    // Documents as the desktop names it.
    std::fs::write(config.join("user-dirs.dirs"), "XDG_DOCUMENTS_DIR=\"$HOME/Papers\"\n").unwrap();
    let documents = home.join("Papers");
    assert_eq!(kolada_core::default_vault_dir(), Some(documents.join("Kolada")));
    // A first run: no session, no vault yet. The window opens on a fresh
    // page; the welcome note is written after the first frame, and opens.
    let vault_dir = kolada_app::launch::launch_vault(None);
    assert_eq!(vault_dir, documents.join("Kolada"));
    let mut m = linux_model(Model::launch(services::services(), vault_dir.clone(), true));
    m.frame(1600, 1000, 2.0);
    assert!(!vault_dir.exists(), "nothing is written before the first frame");
    let (tx, rx) = std::sync::mpsc::channel();
    m.start_arrivals(move || {
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(10)).expect("the welcome note arrives");
    assert!(m.poll_arrivals());
    let notes: Vec<_> = std::fs::read_dir(&vault_dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".md")).collect();
    assert_eq!(notes.len(), 1, "the welcome note");
    let welcome = std::fs::read_to_string(notes[0].path()).unwrap();
    assert!(welcome.contains("Ctrl"), "its keys are Linux's (Ctrl), not the Mac's");
    // The session is kept in the app's data, for the next launch.
    m.save_flush();
    assert!(data.join(own).join("session.bin").is_file(), "the session is in XDG_DATA_HOME");
    let again = kolada_app::launch::launch_vault(None);
    assert_eq!(again, vault_dir, "the next launch opens the same vault");
    let _ = std::fs::remove_dir_all(root);
}
