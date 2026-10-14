//! Watches the vault for what other apps (and Kolada's other windows) do to
//! it: notes edited, added, removed or renamed. Changes are gathered until
//! the folder is quiet for a moment, then posted to the window together,
//! so a sync or a git checkout touching hundreds of files is one update.
//! Hidden folders (`.obsidian`, `.trash`, `.git`) are left out.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{HSTRING, Result};

use crate::window::WM_APP_VAULT;

/// How long the folder stays quiet before its changes are sent, ms; and
/// the longest changes wait while it never is (a long sync).
const QUIET: u32 = 150;
const AT_MOST: std::time::Duration = std::time::Duration::from_secs(1);

/// The watch that is current: a new vault starts a new one, and the old
/// one stops.
static CURRENT: AtomicU64 = AtomicU64::new(0);

/// What happened to a path (as the file system says).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Added,
    Removed,
    Modified,
    /// Renamed: the old name, then the new.
    RenamedFrom,
    RenamedTo,
}

/// Changes in a vault: paths from its top with `/` between folders, in the
/// order they happened. `overflow`: too many to list; look at everything.
pub struct Changes {
    pub vault: PathBuf,
    pub list: Vec<(Action, String)>,
    pub overflow: bool,
}

/// Watches `vault` on a worker, posting `Changes` to `hwnd`; any watch
/// before stops.
pub fn start(hwnd: HWND, vault: PathBuf) {
    let id = CURRENT.fetch_add(1, Ordering::SeqCst) + 1;
    if vault.as_os_str().is_empty() {
        return;
    }
    let hwnd = crate::Handoff(hwnd);
    std::thread::spawn(move || {
        let hwnd = hwnd;
        crate::background_priority();
        let _ = watch(hwnd.0, vault, id);
    });
}

/// A handle closed when dropped.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn watch(hwnd: HWND, vault: PathBuf, id: u64) -> Result<()> {
    unsafe {
        let dir = Owned(CreateFileW(
            &HSTRING::from(vault.as_os_str()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        )?);
        let event = Owned(CreateEventW(None, true, false, None)?);
        // DWORD-aligned, as the records in it are.
        let mut buffer = vec![0u32; 16 * 1024];
        let mut pending = Changes { vault: vault.clone(), list: Vec::new(), overflow: false };
        let mut since = std::time::Instant::now();
        loop {
            let mut overlapped = OVERLAPPED { hEvent: event.0, ..Default::default() };
            ResetEvent(event.0)?;
            ReadDirectoryChangesW(
                dir.0,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 4) as u32,
                true,
                FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE,
                None,
                Some(&mut overlapped),
                None,
            )?;
            // Wait for the read; with changes in hand, a quiet moment sends
            // them. Now and then, look whether this watch is still wanted.
            loop {
                let waiting = pending.list.is_empty() && !pending.overflow;
                let woke = WaitForSingleObject(event.0, if waiting { 2000 } else { QUIET });
                if woke == WAIT_OBJECT_0 {
                    let mut bytes = 0;
                    GetOverlappedResult(dir.0, &overlapped, &mut bytes, false)?;
                    let bytes: &[u8] = std::slice::from_raw_parts(buffer.as_ptr().cast(), bytes as usize);
                    if waiting {
                        since = std::time::Instant::now();
                    }
                    if bytes.is_empty() {
                        pending.overflow = true;
                    } else {
                        pending.list.extend(parse(bytes).into_iter().filter(|(_, p)| !hidden(p)));
                    }
                    if since.elapsed() >= AT_MOST && (!pending.list.is_empty() || pending.overflow) {
                        let changes = std::mem::replace(&mut pending, Changes { vault: vault.clone(), list: Vec::new(), overflow: false });
                        post(hwnd, changes);
                    }
                    break;
                }
                if woke != WAIT_TIMEOUT {
                    return Ok(());
                }
                if CURRENT.load(Ordering::SeqCst) != id {
                    // The read still points at the buffer: end it first.
                    let _ = CancelIoEx(dir.0, Some(&overlapped));
                    let mut bytes = 0;
                    let _ = GetOverlappedResult(dir.0, &overlapped, &mut bytes, true);
                    return Ok(());
                }
                if !waiting {
                    let changes = std::mem::replace(&mut pending, Changes { vault: vault.clone(), list: Vec::new(), overflow: false });
                    post(hwnd, changes);
                }
            }
        }
    }
}

fn post(hwnd: HWND, changes: Changes) {
    let changes = Box::into_raw(Box::new(changes));
    if unsafe { PostMessageW(Some(hwnd), WM_APP_VAULT, WPARAM(0), LPARAM(changes as isize)) }.is_err() {
        drop(unsafe { Box::from_raw(changes) });
    }
}

/// The records of a `FILE_NOTIFY_INFORMATION` list: each an offset to the
/// next (0 for the last), an action, a name's length in bytes, the name.
fn parse(bytes: &[u8]) -> Vec<(Action, String)> {
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let mut out = Vec::new();
    let mut at = 0;
    while let (Some(next), Some(action), Some(len)) = (u32_at(at), u32_at(at + 4), u32_at(at + 8)) {
        let Some(name) = bytes.get(at + 12..at + 12 + len as usize) else { break };
        let units: Vec<u16> = name.as_chunks::<2>().0.iter().map(|u| u16::from_le_bytes(*u)).collect();
        let action = match action {
            1 => Some(Action::Added),
            2 => Some(Action::Removed),
            3 => Some(Action::Modified),
            4 => Some(Action::RenamedFrom),
            5 => Some(Action::RenamedTo),
            _ => None,
        };
        if let Some(action) = action {
            out.push((action, String::from_utf16_lossy(&units).replace('\\', "/")));
        }
        if next == 0 {
            break;
        }
        at += next as usize;
    }
    out
}

/// In a hidden folder, or hidden itself (a name starting with a dot).
fn hidden(path: &str) -> bool {
    path.split('/').any(|part| part.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(next: u32, action: u32, name: &str) -> Vec<u8> {
        let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut r = [next.to_le_bytes(), action.to_le_bytes(), (units.len() as u32).to_le_bytes()].concat();
        r.extend(units);
        // Records start DWORD-aligned.
        r.resize(if next > 0 { next as usize } else { r.len() }, 0);
        r
    }

    #[test]
    fn reads_what_changed() {
        let first = record(0, 4, r"Projects\Plan.md");
        let size = first.len().div_ceil(4) as u32 * 4;
        let mut bytes = record(size, 4, r"Projects\Plan.md");
        bytes.extend(record(0, 5, r"Projects\Launch plan.md"));
        assert_eq!(parse(&bytes), [(Action::RenamedFrom, "Projects/Plan.md".to_string()), (Action::RenamedTo, "Projects/Launch plan.md".to_string())]);
        assert!(hidden(".obsidian/workspace.json") && hidden("Notes/.trash/a.md") && !hidden("Notes/a.md"));
    }
}
