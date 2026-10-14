//! A plain mid-gray window over the monitor's work area. Every run launches
//! onto the same static background, so other apps animating behind the
//! target (terminals, spinners, video) cannot disturb the measurement.
//! Mid-gray differs clearly from both dark and light app themes.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    RDW_ERASE, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GW_HWNDPREV, GetWindow, HWND_TOP, MSG, PM_REMOVE, PeekMessageW,
    RegisterClassW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetForegroundWindow,
    SetWindowPos, TranslateMessage, WINDOW_EX_STYLE, WNDCLASSW, WS_POPUP,
};
use windows::core::w;

pub struct Stage {
    hwnd: HWND,
    /// The monitor's work area (screen minus taskbar), physical pixels.
    pub work_area: RECT,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

impl Stage {
    /// Covers the work area of the monitor whose desktop rect is given.
    pub fn new(desktop: RECT) -> Result<Stage, String> {
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandle: {e}"))?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("KoladaBenchStage"),
                hbrBackground: CreateSolidBrush(COLORREF(0x0080_8080)),
                ..Default::default()
            };
            RegisterClassW(&class);

            let center = POINT {
                x: (desktop.left + desktop.right) / 2,
                y: (desktop.top + desktop.bottom) / 2,
            };
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let monitor = MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST);
            let _ = GetMonitorInfoW(monitor, &mut info);
            let r = info.rcWork;

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("KoladaBenchStage"),
                w!("kolada-bench stage"),
                WS_POPUP,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| format!("stage window: {e}"))?;
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_SHOWWINDOW,
            );
            let _ = SetForegroundWindow(hwnd);
            let stage = Stage { hwnd, work_area: r };
            Stage::pump(Duration::from_millis(300));
            Ok(stage)
        }
    }

    /// Takes the foreground and lets the next launched app take it from us,
    /// as Explorer does for a Start-menu launch. Without this, apps started
    /// through a broker (packaged apps like Notepad) open behind the stage.
    pub fn prepare_launch(&self) {
        unsafe {
            let _ = SetForegroundWindow(self.hwnd);
            let _ = AllowSetForegroundWindow(ASFW_ANY);
        }
    }

    /// Brings `hwnd` above the stage without activating it, if it opened
    /// underneath. Returns whether it had to.
    pub fn raise_above(&self, hwnd: HWND) -> bool {
        unsafe {
            let mut above = GetWindow(hwnd, GW_HWNDPREV).ok();
            while let Some(h) = above.filter(|h| !h.is_invalid()) {
                if h == self.hwnd {
                    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
                    let _ = SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, 0, 0, flags);
                    return true;
                }
                above = GetWindow(h, GW_HWNDPREV).ok();
            }
        }
        false
    }

    /// Repaints the stage (same gray) so DWM presents a fresh frame.
    pub fn repaint(&self) {
        unsafe {
            let _ = RedrawWindow(
                Some(self.hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
            );
        }
        Stage::pump(Duration::from_millis(1));
    }

    /// Keeps the stage responsive while waiting. Sleeps in 1 ms steps.
    pub fn pump(duration: Duration) {
        let start = Instant::now();
        loop {
            let mut msg = MSG::default();
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if start.elapsed() >= duration {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}
