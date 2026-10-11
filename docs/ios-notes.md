# iOS: what is reusable, what the shell needs

Notes for the iOS shell. Read from `crates/notsy-mac` as of 2026-10-05, updated 2026-10-09 for the lift of the app model into `notsy-app`.

**Built since (2026-10-09, branch `ios/shell`):** `crates/notsy-ios` (the model behind a C interface, with iOS's calendar and fonts; tested end to end through the C calls) and `apps/ios/Notsy` (a thin Swift UIKit shell: drawing on change, taps and pans with momentum, the iPad pointer, `UITextInput` with marked text, the hardware keyboard, cut/copy/paste and the edit menu, selection by long press and pointer drag, VoiceOver line by line, Reduce Motion, autosave 0.6 s after an edit and on leaving the front, the vault from the Files app as a security-scoped bookmark, iCloud placeholders fetched, import and export through the Files picker and the share sheet, the icon and a paper or graphite launch screen). `tools/ios-build.sh --run` builds it with no Xcode project and opens it in the iPad simulator; `tools/ios-launch.sh` measures launch against an empty UIKit app (Notsy adds about 6 ms). Still to do: fonts on a device (the simulator reads the Mac's), signing and TestFlight, the writing model in process, a `Watcher` (the vault is read again on coming forward instead). See [mac-ios-status.md](mac-ios-status.md).

## Already shared, compiles for iOS as is

`notsy-core`, `notsy-markdown`, `notsy-editor`, `notsy-text`, `notsy-ui`, `notsy-convert`, `notsy-assist`, `notsy-app` have no OS code (pure Rust, no `cfg(windows)` outside tests; `notsy-ui` pulls `windows-core` only for its `Result` type). Not yet checked with `cargo check --target aarch64-apple-ios` (rustup target not installed on the build host: ask first).

Things to know before they run on a phone:

- `notsy_core::data_root()` treats everything that is not Windows or macOS as Linux (`XDG_DATA_HOME`, else `~/.local/share`). On iOS there is no such home: the app container's Application Support directory comes from `NSFileManager`. Give `notsy-core` an override the shell sets at start (a `set_data_root`), or add an `ios` branch fed by the shell; do not read the OS API inside core.
- `default_vault_dir()` (`~/Documents/Notsy`) means nothing on iOS: the vault is wherever the user picked it (below).
- `notsy-ui`'s `testing.rs` fonts (what `Services::none()` uses) and the Mac's `fonts.rs` name system font files (`/System/Library/Fonts/...`). iOS keeps its system fonts outside the app's reach: implement `services::Fonts` over the core set shipped inside the app bundle (`FontSet.data` takes bytes) and load fallbacks (emoji, Arabic, CJK) from the bundle too. Never scan.
- `std::thread` and `std::fs` work. File watching (`FSEvents`) is not a std feature: it is the `Watcher` service.

## `notsy-app`: the headless app model, shared (lifted out of `notsy-mac` on 2026-10-09)

`crates/notsy-app` holds the whole app state and everything that is not window, input or blit: `Model` (`model.rs` and `model/*.rs`: the session, the vault, the `Editor`, the scene `Eng`, the `Surface` it draws into, search, the trash, the time scrubber, settings, find, pictures, the agenda, Favorites and Recent, spelling, the writing model's requests, menus, drags...), `gists.rs`, `helper.rs`, and `launch.rs` (which vault to open, the newest note). It has no `objc2`, no AppKit and no `cfg(target_os)` dependency: it builds and tests on Windows, macOS and Linux (CI runs it on all three) and compiles as is for iOS. The files moved with `git mv`, so `git log --follow` reaches their Mac history.

What stayed in `notsy-mac`: `view.rs`, `app.rs`, `status.rs`, `hotkey.rs`, `view/*` (AppKit), `trace.rs`, the `--dump` entry in `mod.rs`, and the Mac's implementations of the services below (`services.rs` puts them together).

### Services: what the shell hands the model

The model never calls an OS API. Everything it needs from the OS is a small `Send + Sync` trait in `notsy_app::services`, collected in a `Services` (a struct of `Arc<dyn Trait>`, cheap to clone) that the shell builds and passes in. `Services::none()` (the stand-in `NoServices`) answers every one harmlessly and deterministically, which is what tests and dumps run on; a shell starts with it and replaces fields as it implements them (`Services { calendar: Arc::new(Mine), ..Services::none() }`).

| Field / trait | The shell says | The Mac does it with | `none()` |
|---|---|---|---|
| `calendar: Calendar` | `local(t) -> Option<Local>` (year, month, day, weekday, hour, minute, second on the local calendar), `clock(t)` (the time as the user's 12 or 24 hour setting writes it) | `localtime_r`, `NSDateFormatter` (`when.rs`) | UTC, 24 hour |
| `spell: Spell` | `check(&[u16]) -> ranges`, `learn(word)`, `suggest(word)` | `NSSpellChecker` (`spell.rs`); on iOS, `UITextChecker` | nothing misspelled |
| `ocr: Ocr` | `read(path) -> Read::Words / Missing` (called from the index's thread) | Vision (`vision.rs`) | `Missing` |
| `web: Web` | `get(url, most) -> Option<Vec<u8>>` (http and https only, from a thread) | `NSData` (`web.rs`); on iOS, `URLSession` | no network |
| `bin: Bin` | `recycle(path) -> bool`: the OS's own trash (used only once the app, not a test or dump, runs) | `trashItemAtURL` (`bin.rs`); iOS has no Trash: return false and the note stays in Notsy's `.trash` | refuses |
| `watcher: Watcher` | `watch(vault, wake) -> Receiver<Change>`; `Change { path, flags }` with `ITEM_CREATED`, `ITEM_REMOVED`, `ITEM_RENAMED` (a move is two changes, both renamed, old path first); `wake` is called from the watcher's thread after each batch | FSEvents (`watch.rs`); on iOS, `NSFilePresenter` or a re-read on foreground | never reports |
| `priority: Priority` | `lower_thread()` (the index's thread), `lower_process(pid)` (the writing helper) | QoS background, `setpriority` (`services.rs`) | nothing |
| `fonts: Fonts` | `font_set(option) -> notsy_text::FontSet` (core faces for the first screen, fallbacks loaded later, `data` for bundled bytes), `note_family(option) -> (family, files)` for the Font option (sans 0, serif 1, mono 2) | named system font files (`fonts.rs`); on iOS, the bundled set | `notsy_ui::testing::fonts()` |
| `images: Images` | `decoder() -> Option<Box<dyn notsy_ui::pictures::Decoder>>`: a picture's size, and its pixels scaled, on the pictures' worker thread | ImageIO (`images.rs`) | no pictures |
| `installed: Installed` | `app(name) -> bool`, `tool(name) -> Option<PathBuf>`: which AI apps and command-line tools the send menu offers | `/Applications`, PATH and the usual tool folders (`installed.rs`); on iOS, none (the web fallbacks show) | nothing installed (ChatGPT and Perplexity on the web) |

What the model wants *done* (open a URL or a file, show it in the Finder, copy to the pasteboard, a save or open panel, a window of its own, the menu bar icon, the hotkey, the Dock menu) is not a service: it comes back as values the shell polls and performs: `take_effects()` (`Effect::Url`, `Window`, `Rich`, `Clipboard`, `Export`, `Terminal`...), `take_requests()` (settings: `Request::Pick`, `ShowVault`, tray and hotkey changes), `take_menu_request()`, `whispered()`. Pasting goes the other way: the shell reads its pasteboard and calls `paste`, `paste_plain` or `paste_picture`.

Not yet a service: `helper.rs` still starts the writing model as a child process (`notsy-ai` beside the binary, over stdin and stdout). Where there is no such file (iOS, or the model not downloaded) it ends at once and writing help stays off, silently, so an iOS shell compiles and runs unchanged; the in-process model (below) will replace `Helper::start`: make it a service then.

### Building the model

- `Model::launch(services, vault, dark)`: the app's launch (session restored and kept, the OS's trash and the data folder in use, animations on). `Model::note_window(services, vault, rel, dark)`: a note's own window. `Model::with_services(services, vault, dark)`: nothing kept and nothing written outside the vault: for dumps. `Model::new(vault, dark)` is `with_services(Services::none(), ..)`: for tests.
- `notsy_app::launch::{vault, launch_vault, newest}`: which vault opens (`NOTSY_VAULT`, else the last one open, else the default) and its newest note. On iOS the vault is the one the picker gave (below).

### The call list (what a shell drives)

All `pub` on `Model`; they return `bool` ("the screen changes: draw again") unless said.

- Draw: `frame(w, h, scale)` then `surface()` (`0x00RRGGBB` pixels; the view blits it); `wants_frames()` and `next_due()` say whether and when to draw again (0 fps idle); `tick()`; `set_dark` and `dark_to_tell` (the system's appearance).
- Edit: `insert`, `newline`, `tab`, `backspace`, `delete`, `delete_to`, `undo`, `redo`, `tool(Tool)`, `block`, `toggle_task_here`, `go(Motion, extend)`, `page`, `select_all`, `selected_text`, `selection`, `caret_rect`, `text`.
- Text input in UTF-16 units, as UIKit and AppKit ask for it: `selection_utf16`, `marked_utf16`, `has_marked`, `select_utf16`, `set_marked`, `set_marked_converting`, `commit_marked`, `unmark`, `substring_utf16`, `index_at`.
- Pointer: `pointer_moved`, `pointer_left`, `pointer_down(x, y, clicks, shift)`, `pointer_dragged`, `pointer_up`, `pointer_up_at`, `scroll_by`, `wheel`. A touch shell maps tap, long-press and drag onto them and adds what a finger needs (inertial scroll, selection handles, which `notsy-ui` does not have yet).
- Notes and chrome: `open`, `open_path`, `save`, `save_flush`, `new_note`, `open_today`, `open_search`, `close_search`, `open_palette`, `open_trash`, `trash_pick`, `escape`, `open_find`, `close_find`, `find_again`, `toggle_scrub`, `scrub_step`, `restore_version`, `toggle_options`, `choose(Setting, value)`, `toggle_panel`, `set_minimal`, `open_note_over`, `close_popup`, `open_recent`, `open_favorite`, `toggle_favorite`, `open_agenda`, `follow_link`, `context_menu`, `row_menu`, `send_menu`, `run_menu`, `spell_menu_items`, `spell_pick`, `show_social`.
- Background work the shell wakes and polls (each `start_*` takes a `wake` called from another thread; each `poll_*` says whether to draw): `start_pictures` and `poll_pictures`, `start_watching` and `poll_outside`, `start_arrivals` and `poll_arrivals`, `start_assist` and `assist_events`, `start_index`, `on_index` and `index_arrived`, `spell_step` (a few lines at a time while idle), `update_related`.
- Files arriving: `drop_files`, `drag_enter`, `drag_over`, `drag_leave`, `drag_drop`, `import_files`, `import_folder`, `import_clipboard`, `switch_vault`.

The `--dump` path in `notsy-mac` (`mac/mod.rs`) draws one frame to a BMP with no window, now on `Model::with_services`: the way to test an iOS frame on a Mac with no simulator.

Two Mac files are AppKit and do not carry over: `mac/view.rs` (`NotsyView`: `draw_rect`, `NSTextInputClient`, key and mouse events, `NSPasteboard`, `present`, the CGImage blit) and `mac/app.rs` (delegate, window, menu bar).

Known gaps for other OSes in the model itself: a note moved by another app is followed by its file id (device and inode: unix only; elsewhere by name alone, and the two tests that need an id are `cfg(unix)`); the created date of imported notes is set only on macOS and Windows; a few labels and effects say "Finder" (a shell may relabel).

Tests: `crates/notsy-app` holds the model's tests (181 passing and 5 ignored speed and diagnostic tests on Windows). Timing tests that need the Mac's own fonts (`time_a_*_on_the_macs_fonts`) and the Mac's service tests stay in `notsy-mac`. A shell's own tests should build the model on its `Services` and drive it through the calls above.

## What an iOS shell has to provide

- **Services**: implement the traits above that the phone has an answer for (fonts and the calendar first; then spell, web, images, installed) and start the rest from `Services::none()`.
- **View**: a `UIView` subclass whose `drawRect:` (or a `CALayer`'s `contents` set from a `CGImage`) blits `Model::surface()`, the same CGImage code as `present` in `view.rs` with the flip removed (UIKit views are not flipped). Re-run `frame` on `layoutSubviews` with `UIScreen`'s scale. Draw only on demand (0 fps idle: no `CADisplayLink` running while nothing changes).
- **Keyboard and IME**: `UITextInput` (not `NSTextInputClient`), on a view that `canBecomeFirstResponder`: `insertText`, `deleteBackward`, `setMarkedText:selectedRange:`, `unmarkText`, `markedTextRange`, `selectedTextRange`, `text(in:)`, `firstRect(for:)`, `caretRect(for:)`, `closestPosition(to:)`, plus `UITextPosition` and `UITextRange` subclasses wrapping UTF-16 offsets. Everything it asks maps to the model's `*_utf16` calls. Hardware keyboards arrive as `pressesBegan` (arrows, Cmd shortcuts); `UIKeyCommand` gives the shortcuts and the Cmd-hold help.
- **Touch**: gesture recognizers to `pointer_*` and `scroll_by`; a native `UIEditMenuInteraction` for cut/copy/paste; `UIPasteboard` in place of `NSPasteboard`. Safe areas and the on-screen keyboard's height change the viewport.
- **Vault via Files / iCloud**: the vault is a folder the user picks with `UIDocumentPickerViewController` (folder mode), which returns a security-scoped URL. Keep a bookmark (`bookmarkData` with security scope, not the path) in the session, resolve it at launch, call `startAccessingSecurityScopedResource()` before any `std::fs` call and `stop...` when done. After that the shared crates work on plain paths. iCloud Drive files may be placeholders that are not downloaded yet: coordinate reads with `NSFileCoordinator` (or `startDownloadingUbiquitousItem`), or the first read of a note can fail or block. Vault-wide scans (`search::index`, trash) will meet that: they need to tolerate and report a missing file rather than stop. Obsidian's own iOS vaults live in the iCloud container the same way.
- **No helper processes**: iOS cannot spawn them. Today `notsy-ai` runs as a child process speaking over stdin/stdout, and `notsy-convert` runs in a child on Windows (`--paste-markdown`). On iOS both must run in-process, on a thread: `notsy-convert` is pure Rust, so call it directly; `notsy-ai` needs a library entry point (link llama.cpp's static libs plus the C shim into the app, call the same request/response code `notsy-ai/src/main.rs` runs over a pipe, from a background thread at low QoS). Memory is the limit: a phone kills an app past its memory budget, so use a small quantised model, `mmap` it, and drop it when the note is not being worked on. The model file is not bundled: it is downloaded after install (ask the user first: that is a download).
- **Build**: a Rust static library (`crate-type = ["staticlib"]`) linked into a thin Swift or Objective-C app target, or a pure `objc2` app as `notsy-mac` is; either way an Xcode project (or `xcodebuild` on the Mac host) is needed for signing, the Info.plist and the launch screen. A launch screen must show the paper or graphite background at once, so the 100 ms promise is met by the first frame being the session snapshot and one note, as everywhere else.
- **Lifecycle**: iOS suspends and kills without warning. Save on `applicationWillResignActive` / `didEnterBackground` (the model's `save` and the session snapshot), not only on a quit.
- **Sandbox of app data**: history, trash list and session live under `app_data_dir()`, which on iOS is the app container, outside the vault: fine, same rule as other platforms.

## Open questions for the user

- Does the iOS build target the iPad first (hardware keyboard, closest to the desktop UI) or the iPhone?
- Where may the model be downloaded from, and how large (this needs approval before any download)?
- Settled 2026-10-09: the shared model lift happened before the iOS shell, as `notsy-app`.
