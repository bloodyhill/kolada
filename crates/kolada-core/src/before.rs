//! What the app was called before it was Kolada, kept so that what earlier
//! installs left behind goes on working: their data folder and vault are
//! taken over (`adopt_old_data`), a copy of the old app still running is
//! seen, an install from the website moves house, and old installs still
//! get updates. This file is never renamed.

/// The old name: the data folder, the vault folder in Documents.
pub const NAME: &str = "Notsy";

/// The old program and its writing helper.
pub const EXE: &str = "notsy.exe";
pub const HELPER: &str = "notsy-ai.exe";
/// What an old program left beside it: itself replaced by an update, or a
/// download cut short.
pub const LEFTOVERS: [&str; 2] = ["notsy.old.exe", "notsy.part"];

/// The writing model trained for the app keeps its file name, and the
/// helper knows it by its first part.
pub const MODEL: &str = "notsy-writer-1.2b-Q4_K_M.gguf";
pub const MODEL_PREFIX: &str = "notsy-writer";

/// An old copy's single-instance mutex and main window class.
pub const INSTANCE: &str = r"Local\Notsy.Instance";
pub const MAIN_CLASS: &str = "Notsy.Main";

/// The ProgID `.md` files open through. Kept for good: Windows protects the
/// user's choice of it, and an app cannot make itself the default again.
pub const PROG_ID: &str = "Notsy.md";

/// Where the website's installer put an old install, under
/// `%LOCALAPPDATA%\Programs`.
pub const INSTALL_DIR: &str = "Notsy";

/// An old install's registry names (all under HKCU).
pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Notsy";
pub const SOFTWARE_KEY: &str = r"Software\Notsy";
pub const APPLICATIONS_KEY: &str = r"Software\Classes\Applications\notsy.exe";
/// Its value under Run and under RegisteredApplications, and its shortcuts'
/// name.
pub const VALUE: &str = "Notsy";
/// The choices it kept in its uninstall key: desktop shortcut, `.md` files,
/// starting with Windows.
pub const CHOICES: [&str; 3] = ["NotsyDesktop", "NotsyOpensMarkdown", "NotsyStartsWithWindows"];

/// The update feed installs from before ask. The website keeps serving it,
/// with a download on the same site.
pub const FEED: &str = "https://notsy.co/download/latest.json";
