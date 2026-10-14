//! What is installed on this Mac, for the model's `Installed` service (the
//! send menu offers the AI apps and tools found): apps where Mac apps go,
//! command-line tools where they usually live.

use std::path::PathBuf;

/// Where command-line tools live on a Mac: an app opened from the Finder
/// does not get the shell's PATH, so the usual places are looked in too.
fn tool_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    for d in ["/opt/homebrew/bin", "/usr/local/bin"] {
        dirs.push(PathBuf::from(d));
    }
    for d in [".local/bin", ".npm-global/bin", ".bun/bin", ".volta/bin", ".cargo/bin"] {
        dirs.push(home.join(d));
    }
    dirs
}

/// The command `name`, where it is found.
fn find_tool(name: &str) -> Option<PathBuf> {
    tool_dirs().into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// Whether the app `name` is installed where Mac apps go.
fn has_app(name: &str) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    [PathBuf::from("/Applications"), home.join("Applications")].iter().any(|d| d.join(format!("{name}.app")).is_dir())
}

/// The Mac's apps and tools.
pub struct MacInstalled;

impl kolada_app::services::Installed for MacInstalled {
    fn app(&self, name: &str) -> bool {
        has_app(name)
    }

    fn tool(&self, name: &str) -> Option<PathBuf> {
        find_tool(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kolada_app::services::Installed;

    #[test]
    fn finds_a_tool_every_mac_has_and_not_one_it_does_not() {
        assert!(MacInstalled.tool("ls").is_some_and(|p| p.is_file()));
        assert!(MacInstalled.tool("no-such-tool-kolada").is_none());
        assert!(!MacInstalled.app("No Such App Kolada"));
    }
}
