//! One Notsy process per user session. A second launch hands its file (if
//! any) to the running instance, which comes to the front, and exits.

use std::path::Path;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, WM_COPYDATA,
};
use windows::core::{PCWSTR, w};

use crate::window::main_class;

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
        let name = if notsy_core::testing() { w!("Local\\Notsy.Test.Instance") } else { w!("Local\\Notsy.Instance") };
        match CreateMutexW(None, false, name) {
            Ok(h) if GetLastError() == ERROR_ALREADY_EXISTS => {
                let _ = CloseHandle(h);
                None
            }
            Ok(h) => Some(Instance(h)),
            // Without a mutex we cannot coordinate; run normally.
            Err(_) => Some(Instance(HANDLE::default())),
        }
    }
}

/// Whether a Notsy holds the instance (one is running).
pub fn running() -> bool {
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    let name = if notsy_core::testing() { w!(r"Local\Notsy.Test.Instance") } else { w!(r"Local\Notsy.Instance") };
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, name) } {
        Ok(h) => {
            unsafe {
                let _ = CloseHandle(h);
            }
            true
        }
        Err(_) => false,
    }
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
    for _ in 0..100 {
        if let Ok(hwnd) = unsafe { FindWindowW(main_class(), PCWSTR::null()) } {
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
