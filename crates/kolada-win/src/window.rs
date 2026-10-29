//! The main window. It is created cloaked, so nothing shows until the first
//! frame is presented, and with the open animation disabled, so the first
//! frame appears at full size immediately.

use std::ffi::c_void;

use kolada_core::session::Placement;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAK, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromRect, ValidateRect,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetClientRect, IDC_ARROW, LoadCursorW, RegisterClassExW, SM_CXPADDEDBORDER,
    ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CXSMICON,
    SM_CYFRAME, SPI_GETWORKAREA, SendMessageW, WM_SETICON,
    HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetWindowPos,
    SW_HIDE, SW_SHOWNA, SW_SHOWNOACTIVATE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, ShowWindow,
    SystemParametersInfoW, WINDOW_EX_STYLE, WM_APP, WNDCLASSEXW, WNDPROC, WS_MAXIMIZE, WS_OVERLAPPEDWINDOW,
};
use windows::core::{BOOL, PCWSTR, Result, w};

/// The main window's class, and a note's own window's (see `--window`):
/// another, so the main window stays the one a second launch hands its file
/// to. A test run has classes of its own (see `kolada_core::testing`).
pub fn main_class() -> PCWSTR {
    if kolada_core::testing() { w!("Kolada.Test.Main") } else { w!("Kolada.Main") }
}

pub fn note_class() -> PCWSTR {
    if kolada_core::testing() { w!("Kolada.Test.Note") } else { w!("Kolada.Note") }
}

/// A note window's size, DIPs.
const NOTE_SIZE: (i32, i32) = (580, 720);

/// Posted with a boxed `Vec<SidebarRow>` in LPARAM once the vault is listed.
pub const WM_APP_SIDEBAR: u32 = WM_APP + 1;
/// Posted with the system light/dark setting (WPARAM 1 for dark).
pub const WM_APP_THEME: u32 = WM_APP + 2;
/// Posted with a boxed `Vec<search::Indexed>` once the vault is read.
pub const WM_APP_INDEX: u32 = WM_APP + 3;
/// Posted with a boxed `String` a worker wants said (see `whisper`).
pub const WM_APP_WHISPER: u32 = WM_APP + 4;
/// Posted with boxed spelling marks, by line (see `spell`).
pub const WM_APP_SPELL: u32 = WM_APP + 5;
/// Sent by the notification area about the tray icon (see `app::tray`).
pub const WM_APP_TRAY: u32 = WM_APP + 6;
/// Posted with a boxed `images::Found` (see `images`).
pub const WM_APP_IMAGE: u32 = WM_APP + 7;
/// Posted with boxed `watch::Changes` in the vault.
pub const WM_APP_VAULT: u32 = WM_APP + 8;
/// Posted with a boxed `app::share::ExportsIn` once other apps' exports
/// are written into the vault.
pub const WM_APP_IMPORTED: u32 = WM_APP + 9;
/// Posted with a boxed `convert::Converted` once documents are notes.
pub const WM_APP_CONVERTED: u32 = WM_APP + 11;
/// Posted with a boxed `app::arrive::Dropped` by a drop, to take the files
/// once the app they were dragged from is no longer waiting.
pub const WM_APP_DROPPED: u32 = WM_APP + 12;
/// Posted with a boxed `app::paste::Fetched` as each picture of a paste
/// comes from the web (or does not).
pub const WM_APP_FETCHED: u32 = WM_APP + 13;
/// Posted with a boxed `PathBuf`: a first run's welcome note, written.
pub const WM_APP_WELCOME: u32 = WM_APP + 14;
/// An update downloaded and checked (lparam: Box<String>, its version): the
/// side menu's pill.
pub const WM_APP_UPDATE: u32 = WM_APP + 15;
/// Posted with a boxed `share::Answer`: a link made, listed or deleted.
pub const WM_APP_SHARE: u32 = WM_APP + 24;
/// The writing helper this copy lacked is now beside it (see `updates`).
pub const WM_APP_HELPER: u32 = WM_APP + 25;
/// How far Settings' "Check now" has got (lparam: Box<CheckState>).
pub const WM_APP_CHECK: u32 = WM_APP + 27;
/// Posted when the writing model's process has news: what it sent waits in
/// a channel (see `assist`), so a post lost while the app is busy loses
/// nothing.
pub const WM_APP_ASSIST: u32 = WM_APP + 10;

/// Default size for a first launch, in DIPs.
const DEFAULT_SIZE: (i32, i32) = (1400, 900);

pub fn set_attribute(hwnd: HWND, attribute: DWMWINDOWATTRIBUTE, on: bool) {
    let value = BOOL::from(on);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            attribute,
            &value as *const BOOL as *const c_void,
            size_of::<BOOL>() as u32,
        );
    }
}

fn to_rect(p: &Placement) -> RECT {
    RECT { left: p.x, top: p.y, right: p.x + p.width, bottom: p.y + p.height }
}

/// The saved placement if it is still on a connected monitor, otherwise a
/// centered default on the primary monitor.
pub fn placement(saved: &Placement) -> Placement {
    let on_screen = saved.width > 0
        && saved.height > 0
        && !unsafe { MonitorFromRect(&to_rect(saved), MONITOR_DEFAULTTONULL) }.is_invalid();
    if on_screen {
        return *saved;
    }
    let dpi = unsafe { GetDpiForSystem() };
    let (w, h) = (DEFAULT_SIZE.0 * dpi as i32 / 96, DEFAULT_SIZE.1 * dpi as i32 / 96);
    let mut work = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    let (w, h) = (w.min(work.right - work.left), h.min(work.bottom - work.top));
    Placement {
        x: work.left + (work.right - work.left - w) / 2,
        y: work.top + (work.bottom - work.top - h) / 2,
        width: w,
        height: h,
        dpi,
        maximized: false,
    }
}

/// Where a note's own window goes: with its top left near the screen point
/// `at` (pixels), else centered, inside that monitor's work area.
/// Where the minimal window goes: where it was last, or, the first time, a
/// note window's size inside the full window's top right corner (`full`).
pub fn minimal_placement(saved: &Placement, full: &Placement) -> Placement {
    let on_screen = saved.width > 0 && saved.height > 0 && !unsafe { MonitorFromRect(&to_rect(saved), MONITOR_DEFAULTTONULL) }.is_invalid();
    if on_screen {
        return Placement { maximized: false, ..*saved };
    }
    if full.width <= 0 {
        return note_placement(None);
    }
    let dpi = if full.dpi > 0 { full.dpi as i32 } else { 96 };
    let inset = 24 * dpi / 96;
    note_placement(Some((full.x + full.width - NOTE_SIZE.0 * dpi / 96 - inset, full.y + inset)))
}

/// Moves and sizes a shown window to `p`, maximized or not, without
/// activating it.
pub fn place(hwnd: HWND, p: &Placement) {
    use windows::Win32::UI::WindowsAndMessaging::{SW_SHOWMAXIMIZED, SetWindowPlacement, WINDOWPLACEMENT};
    let wp = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        showCmd: if p.maximized { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNOACTIVATE.0 as u32 },
        ptMinPosition: windows::Win32::Foundation::POINT { x: -1, y: -1 },
        ptMaxPosition: windows::Win32::Foundation::POINT { x: -1, y: -1 },
        rcNormalPosition: RECT { left: p.x, top: p.y, right: p.x + p.width, bottom: p.y + p.height },
        ..Default::default()
    };
    unsafe {
        let _ = SetWindowPlacement(hwnd, &wp);
    }
}

/// Keeps the window above every other (the minimal window, so that clicking
/// another app's window does not bury the note), or lets it sit among them.
pub fn keep_on_top(hwnd: HWND, on: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{HWND_NOTOPMOST, HWND_TOPMOST};
    let at = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
    unsafe {
        let _ = SetWindowPos(hwnd, Some(at), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Whether the window is kept above every other.
pub fn is_on_top(hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongW, WS_EX_TOPMOST};
    unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 }
}

pub fn note_placement(at: Option<(i32, i32)>) -> Placement {
    let dpi = unsafe { GetDpiForSystem() };
    let (w, h) = (NOTE_SIZE.0 * dpi as i32 / 96, NOTE_SIZE.1 * dpi as i32 / 96);
    let point = windows::Win32::Foundation::POINT { x: at.map_or(0, |a| a.0), y: at.map_or(0, |a| a.1) };
    let monitor = unsafe { windows::Win32::Graphics::Gdi::MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let work = if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() { info.rcWork } else { RECT { left: 0, top: 0, right: w, bottom: h } };
    let (w, h) = (w.min(work.right - work.left), h.min(work.bottom - work.top));
    let (x, y) = match at {
        Some((x, y)) => (x, y),
        None => (work.left + (work.right - work.left - w) / 2, work.top + (work.bottom - work.top - h) / 2),
    };
    Placement {
        x: x.clamp(work.left, work.right - w),
        y: y.clamp(work.top, work.bottom - h),
        width: w,
        height: h,
        dpi,
        maximized: false,
    }
}

pub fn register(class: PCWSTR, wndproc: WNDPROC) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: wndproc,
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&class);
    }
    Ok(())
}

/// Creates the window cloaked and shows it (still invisible), so it has its
/// final size and the first frame can be copied into it.
pub fn create(class: PCWSTR, p: &Placement, dark: bool) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        // A maximized window is created maximized: showing it maximized
        // would also activate it, the slow part this path leaves for later.
        let style = if p.maximized { WS_OVERLAPPEDWINDOW | WS_MAXIMIZE } else { WS_OVERLAPPEDWINDOW };
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("Kolada"),
            style,
            p.x,
            p.y,
            p.width,
            p.height,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        set_attribute(hwnd, DWMWA_CLOAK, true);
        set_attribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, true);
        set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, dark);
        let _ = ShowWindow(hwnd, if p.maximized { SW_SHOWNA } else { SW_SHOWNOACTIVATE });
        // Shown without activation, the window can end up behind whatever
        // app is in the foreground until `activate` runs. A launched app must
        // appear on top at once, so raise it now; moving in the z-order
        // does not trigger the slow activation work.
        let _ = SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        Ok(hwnd)
    }
}

pub fn uncloak(hwnd: HWND) {
    set_attribute(hwnd, DWMWA_CLOAK, false);
}

/// Hides the window (it was created cloaked) so that showing it later shows
/// it as usual.
pub fn hide(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    uncloak(hwnd);
}

/// Takes the foreground. The window was shown without activation because
/// the first activation on a thread sets up text input, which is slow.
pub fn activate(hwnd: HWND) {
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn dpi(hwnd: HWND) -> u32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96)
}

/// Client width and height in physical pixels, and the window's DPI.
pub fn view(hwnd: HWND) -> (u32, u32, u32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    ((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32, dpi(hwnd))
}

/// What `view` will return for a window created at `p`, so the first frame
/// can be laid out and drawn before the window exists. A maximized window's
/// client area is its monitor's work area (its frame hangs off-screen, and
/// the caption is ours). The caller redraws if the prediction was wrong.
pub fn predicted_view(p: &Placement) -> (u32, u32, u32) {
    if p.maximized {
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let monitor = unsafe { MonitorFromRect(&to_rect(p), MONITOR_DEFAULTTONEAREST) };
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let w = info.rcWork;
            return ((w.right - w.left).max(1) as u32, (w.bottom - w.top).max(1) as u32, p.dpi.max(96));
        }
    }
    let mut frame = RECT::default();
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut frame, WS_OVERLAPPEDWINDOW, false, WINDOW_EX_STYLE::default(), p.dpi);
    }
    // No caption (see WM_NCCALCSIZE in app.rs): only the side and bottom
    // resize borders are outside the client area.
    let w = p.width - (frame.right - frame.left);
    let h = p.height - frame.bottom;
    (w.max(1) as u32, h.max(1) as u32, p.dpi.max(96))
}

/// The resize border's thickness in pixels at the window's DPI.
pub fn frame_thickness(hwnd: HWND) -> i32 {
    let dpi = dpi(hwnd);
    unsafe { GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) }
}

/// Marks the whole client area as painted.
pub fn validate(hwnd: HWND) {
    unsafe {
        let _ = ValidateRect(Some(hwnd), None);
    }
}


/// Gives the window the app icon (resource 1, from kolada.rc) at its DPI,
/// for the taskbar, Alt+Tab and the window itself. Called after the first
/// frame: loading an icon has no business on the startup path.
/// Takes files dropped from Explorer (after the first frame: shell32).
pub fn accept_drops(hwnd: HWND) {
    // Seen as they come, when OLE starts; else just taken when let go.
    if !crate::drag::register(hwnd) {
        unsafe { windows::Win32::UI::Shell::DragAcceptFiles(hwnd, true) };
    }
}

/// The app icon at the small size, for the tray.
pub fn small_icon(hwnd: HWND) -> windows::Win32::UI::WindowsAndMessaging::HICON {
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi(hwnd)) };
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return Default::default() };
        #[allow(clippy::manual_dangling_ptr)]
        let icon = LoadImageW(Some(instance.into()), PCWSTR(1 as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
        icon.map(|h| windows::Win32::UI::WindowsAndMessaging::HICON(h.0)).unwrap_or_default()
    }
}

pub fn set_icon(hwnd: HWND) {
    let dpi = dpi(hwnd);
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let size = GetSystemMetricsForDpi(metric, dpi);
            // MAKEINTRESOURCE(1): an integer resource id in place of a name.
            #[allow(clippy::manual_dangling_ptr)]
            let icon = LoadImageW(Some(instance.into()), PCWSTR(1 as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
            if let Ok(icon) = icon {
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(kind as usize)), Some(LPARAM(icon.0 as isize)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, DestroyWindow};

    unsafe extern "system" fn plain(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> windows::Win32::Foundation::LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, w, l) }
    }

    #[test]
    fn the_minimal_window_stays_above_other_windows_until_it_is_full_again() {
        let class = w!("Kolada.Test.OnTop");
        register(class, Some(plain)).unwrap();
        let p = Placement { x: 10, y: 10, width: 400, height: 300, dpi: 96, maximized: false };
        let hwnd = create(class, &p, true).unwrap();
        assert!(!is_on_top(hwnd), "a window opens among the others");
        keep_on_top(hwnd, true);
        assert!(is_on_top(hwnd), "the minimal window is kept on top");
        keep_on_top(hwnd, false);
        assert!(!is_on_top(hwnd), "the full window sits among the others again");
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}
