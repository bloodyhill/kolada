//! Launching, finding, closing and killing the app under test.

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateProcessW, OpenProcess, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, STARTUPINFOW, TerminateProcess,
    WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
    IsWindowVisible, PostMessageW, SW_SHOWNORMAL, WM_CLOSE, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PCWSTR, PWSTR, w};

use crate::capture::wide_to_string;

/// Windows smaller than this (in physical pixels) are splash screens,
/// tooltips or helpers, not the app's main window.
const MIN_WINDOW_SIDE: i32 = 200;

/// Matches processes by image file name, e.g. `obsidian.exe`.
pub struct Target {
    names: Vec<String>,
    by_pid: HashMap<u32, bool>,
}

impl Target {
    pub fn new(names: &[String]) -> Target {
        Target {
            names: names.iter().map(|n| n.to_ascii_lowercase()).collect(),
            by_pid: HashMap::new(),
        }
    }

    fn matches_pid(&mut self, pid: u32) -> bool {
        if let Some(&hit) = self.by_pid.get(&pid) {
            return hit;
        }
        let hit = image_name(pid)
            .map(|n| self.names.contains(&n.to_ascii_lowercase()))
            .unwrap_or(false);
        self.by_pid.insert(pid, hit);
        hit
    }

    /// Visible, uncloaked, top-level main windows of the target, with their
    /// visible bounds in physical screen pixels.
    pub fn windows(&mut self) -> Vec<(HWND, RECT)> {
        let mut all: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(collect_window), LPARAM(&mut all as *mut _ as isize));
        }
        all.into_iter()
            .filter(|&hwnd| is_main_window(hwnd))
            .filter(|&hwnd| {
                let mut pid = 0;
                unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
                pid != 0 && self.matches_pid(pid)
            })
            .filter_map(|hwnd| {
                let r = visible_bounds(hwnd)?;
                let big = r.right - r.left >= MIN_WINDOW_SIDE && r.bottom - r.top >= MIN_WINDOW_SIDE;
                big.then_some((hwnd, r))
            })
            .collect()
    }

    /// PIDs of all running processes with a matching image name.
    pub fn processes(&self) -> Vec<u32> {
        let mut pids = Vec::new();
        let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return pids;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = unsafe { Process32FirstW(snap, &mut entry) }.is_ok();
        while ok {
            let name = wide_to_string(&entry.szExeFile).to_ascii_lowercase();
            if self.names.contains(&name) {
                pids.push(entry.th32ProcessID);
            }
            ok = unsafe { Process32NextW(snap, &mut entry) }.is_ok();
        }
        let _ = unsafe { CloseHandle(snap) };
        pids
    }

    /// Asks every target window to close and waits until none is visible.
    /// Apps that live in the tray stay running, which is the resident case.
    pub fn close_windows(&mut self, grace: Duration) {
        for (hwnd, _) in self.windows() {
            let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
        let start = Instant::now();
        while !self.windows().is_empty() && start.elapsed() < grace {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Closes the app gracefully, then terminates anything still running,
    /// so the next launch is a true process start.
    pub fn quit(&mut self, grace: Duration) {
        self.close_windows(grace);
        let start = Instant::now();
        while !self.processes().is_empty() && start.elapsed() < grace {
            std::thread::sleep(Duration::from_millis(20));
        }
        for pid in self.processes() {
            let access = PROCESS_TERMINATE | PROCESS_SYNCHRONIZE;
            if let Ok(h) = unsafe { OpenProcess(access, false, pid) } {
                unsafe {
                    let _ = TerminateProcess(h, 1);
                    WaitForSingleObject(h, 5000);
                    let _ = CloseHandle(h);
                }
            }
        }
        // PIDs can be reused by the next launch.
        self.by_pid.clear();
    }
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let all = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    all.push(hwnd);
    true.into()
}

fn is_main_window(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return false;
        }
        if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        );
        cloaked == 0
    }
}

/// Visible bounds without the invisible resize border. DWM reports these in
/// physical pixels regardless of DPI awareness.
fn visible_bounds(hwnd: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    Some(r)
}

fn image_name(pid: u32) -> Option<String> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
    }
    .is_ok();
    let _ = unsafe { CloseHandle(h) };
    if !ok {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned())
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    format!("\"{}\"", arg.replace('"', "\\\""))
}

/// Starts the app. `shell` goes through ShellExecuteEx like a Start-menu
/// click or double-click (resolves shortcuts and app execution aliases);
/// otherwise CreateProcess is called directly.
pub fn launch(exe: &str, args: &[String], shell: bool) -> Result<(), String> {
    let params: Vec<String> = args.iter().map(|a| quote_arg(a)).collect();
    let params = params.join(" ");
    if shell {
        let file = to_wide(exe);
        let params_w = to_wide(&params);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI,
            lpVerb: w!("open"),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(params_w.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe { ShellExecuteExW(&mut info) }.map_err(|e| format!("ShellExecuteEx {exe}: {e}"))
    } else {
        let mut cmdline = to_wide(&format!("{} {params}", quote_arg(exe)));
        let si = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut pi = PROCESS_INFORMATION::default();
        unsafe {
            CreateProcessW(
                PCWSTR::null(),
                Some(PWSTR(cmdline.as_mut_ptr())),
                None,
                None,
                false,
                PROCESS_CREATION_FLAGS(0),
                None,
                PCWSTR::null(),
                &si,
                &mut pi,
            )
        }
        .map_err(|e| format!("CreateProcess {exe}: {e}"))?;
        for h in [pi.hProcess, pi.hThread] {
            if h != HANDLE::default() {
                let _ = unsafe { CloseHandle(h) };
            }
        }
        Ok(())
    }
}
