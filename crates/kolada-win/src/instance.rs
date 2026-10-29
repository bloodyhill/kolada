//! One Kolada process per user session. A second launch hands its file (if
//! any) to the running instance, which comes to the front, and exits.

use std::path::Path;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use kolada_core::before;
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, WM_COPYDATA,
};
use windows::core::{HSTRING, PCWSTR};

use crate::window::main_class;

/// The instance's name, then an old Notsy's: either one held means the app
/// is running, so the two never run side by side, an update from Notsy
/// waits for it to go, and the installer can ask it to quit.
/// Test instances named apart (`KOLADA_TEST_INSTANCE=a`, `=b`) run side by
/// side, each with its own; Quick Send's pairing is tried that way.
fn names(testing: bool) -> &'static [&'static str] {
    static TEST: std::sync::OnceLock<[&'static str; 1]> = std::sync::OnceLock::new();
    if testing {
        TEST.get_or_init(|| {
            let name = std::env::var("KOLADA_TEST_INSTANCE").ok().filter(|v| v != "1" && v.chars().all(|c| c.is_ascii_alphanumeric()) && v.len() <= 16);
            [match name {
                Some(n) => Box::leak(format!(r"Local\Kolada.Test.Instance.{n}").into_boxed_str()),
                None => r"Local\Kolada.Test.Instance",
            }]
        })
    } else {
        &[r"Local\Kolada.Instance", before::INSTANCE]
    }
}

/// Whether some process holds the mutex `name`.
fn held(name: &str) -> bool {
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &HSTRING::from(name)) } {
        Ok(h) => {
            unsafe {
                let _ = CloseHandle(h);
            }
            true
        }
        Err(_) => false,
    }
}

/// Held for the life of the process.
pub struct Instance(HANDLE);

impl Instance {
    /// Lets the next process claim it (an update starting in this one's
    /// place).
    pub fn release(&self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Returns None if another instance is already running.
pub fn claim() -> Option<Instance> {
    unsafe {
        let names = names(kolada_core::testing());
        match CreateMutexW(None, false, &HSTRING::from(names[0])) {
            Ok(h) if GetLastError() == ERROR_ALREADY_EXISTS || names[1..].iter().any(|n| *n != names[0] && held(n)) => {
                let _ = CloseHandle(h);
                None
            }
            Ok(h) => Some(Instance(h)),
            // Without a mutex we cannot coordinate; run normally.
            Err(_) => Some(Instance(HANDLE::default())),
        }
    }
}

/// Whether a Kolada holds the instance (one is running).
pub fn running() -> bool {
    names(kolada_core::testing()).iter().any(|n| held(n))
}

/// Claims the instance once the process holding it has gone, waiting at
/// most `wait`.
pub fn claim_within(wait: Duration) -> Option<Instance> {
    let until = std::time::Instant::now() + wait;
    loop {
        if let Some(instance) = claim() {
            return Some(instance);
        }
        if std::time::Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Sends `file` to the running instance and lets it take the foreground.
pub fn forward(file: Option<&Path>) {
    let wide: Vec<u16> = file.map(|f| f.to_string_lossy().encode_utf16().collect()).unwrap_or_default();
    send(1, &wide);
}

/// Sends a command (`new`, `search`) to the running instance.
pub fn forward_command(command: &str) {
    let wide: Vec<u16> = command.encode_utf16().collect();
    send(2, &wide);
}

/// What a second launch hands over: 1 a file (or nothing), 2 a command.
fn send(kind: usize, wide: &[u16]) {
    // The other instance may still be creating its window.
    let old = HSTRING::from(before::MAIN_CLASS);
    for _ in 0..100 {
        let found = unsafe { FindWindowW(main_class(), PCWSTR::null()) }
            .or_else(|e| if kolada_core::testing() { Err(e) } else { unsafe { FindWindowW(&old, PCWSTR::null()) } });
        if let Ok(hwnd) = found {
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
            }
            let data = COPYDATASTRUCT {
                dwData: kind,
                cbData: (wide.len() * 2) as u32,
                lpData: wide.as_ptr() as *mut _,
            };
            unsafe {
                SendMessageTimeoutW(
                    hwnd,
                    WM_COPYDATA,
                    WPARAM(0),
                    LPARAM(&data as *const COPYDATASTRUCT as isize),
                    SMTO_ABORTIFHUNG,
                    2000,
                    None,
                );
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_notsy_running_counts_as_running() {
        assert!(names(false).contains(&before::INSTANCE));
        assert!(!names(true).contains(&before::INSTANCE), "a test instance keeps apart");
        let name = format!(r"Local\Kolada.Test.Probe.{}", std::process::id());
        assert!(!held(&name));
        let h = unsafe { CreateMutexW(None, false, &HSTRING::from(name.as_str())) }.unwrap();
        assert!(held(&name));
        unsafe {
            let _ = CloseHandle(h);
        }
        assert!(!held(&name));
    }
}
