//! The icon in the notification area, beside the clock: a click brings
//! Kolada up; its menu starts a note, finds one, or quits. With the Tray
//! option on "Stay running", closing the window only hides it, and it comes
//! back at once. Only the main window has one (not a note's own window).

use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_USER, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_BALLOONHIDE,
    NIN_BALLOONTIMEOUT, NIN_BALLOONUSERCLICK, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, IsWindowVisible, MF_SEPARATOR, MF_STRING, SW_SHOW, TPM_BOTTOMALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_CONTEXTMENU, WM_NULL,
};
use windows::core::{PCWSTR, w};

use super::*;
use crate::window::WM_APP_TRAY;

const ICON_ID: u32 = 1;
/// The icon chosen with the keyboard (NIN_SELECT | NINF_KEY).
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

/// The tray menu's items.
const NEW: usize = 1;
const SEARCH: usize = 2;
const SHOW: usize = 3;
const TODAY: usize = 4;
const QUIT: usize = 9;

/// The quick note hotkey's id.
const QUICK_NOTE: i32 = 1;

impl App {
    fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_APP_TRAY,
            hIcon: window::small_icon(self.hwnd),
            ..Default::default()
        };
        for (d, s) in data.szTip.iter_mut().zip("Kolada".encode_utf16()) {
            *d = s;
        }
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        data
    }

    /// Puts the icon in the tray, or takes it out, as the option says.
    pub(super) fn update_tray(&mut self) {
        if self.compact {
            return;
        }
        let want = self.session.tray != 1;
        let data = self.tray_data();
        unsafe {
            if want && !self.tray_shown {
                if Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                    let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
                    self.tray_shown = true;
                }
            } else if !want && self.tray_shown {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
                self.tray_shown = false;
            }
        }
    }

    /// A notification from the tray icon (Windows shows it with the others,
    /// and keeps it in the notification center). With the icon off, it is
    /// there only while the notification is.
    pub(super) fn notify(&mut self, title: &str, text: &str) {
        if self.compact {
            return;
        }
        let mut data = self.tray_data();
        unsafe {
            if !self.tray_shown {
                if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                    return;
                }
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
                self.tray_shown = true;
                self.agenda.icon_for_now = true;
            }
            data.uFlags |= NIF_INFO;
            data.dwInfoFlags = NIIF_USER;
            data.hBalloonIcon = window::small_icon(self.hwnd);
            for (d, s) in data.szInfoTitle.iter_mut().take(63).zip(title.encode_utf16()) {
                *d = s;
            }
            for (d, s) in data.szInfo.iter_mut().take(255).zip(text.encode_utf16()) {
                *d = s;
            }
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    /// A notification is gone: an icon put there only for it goes too.
    fn notified(&mut self) {
        if std::mem::take(&mut self.agenda.icon_for_now) && self.session.tray == 1 {
            self.remove_tray();
        }
    }

    /// Explorer restarted: the tray is new and empty.
    pub(super) fn tray_again(&mut self) {
        self.tray_shown = false;
        self.update_tray();
    }

    pub(super) fn remove_tray(&mut self) {
        if self.tray_shown {
            let data = self.tray_data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            self.tray_shown = false;
        }
    }

    /// Shows the window (hidden in the tray, minimized, or behind others).
    pub(super) fn bring_up(&mut self) {
        unsafe {
            if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOW);
            }
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            }
            // A test run never takes the keyboard from the user.
            if !NO_ACTIVATE.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
        self.request_frame();
    }

    /// Something happened at the tray icon: `event` (as version 4 of the
    /// notification area sends it), at the screen point `x`, `y`.
    pub(super) fn tray_event(&mut self, event: u32, x: i32, y: i32) {
        match event {
            e if e == NIN_SELECT || e == NIN_KEYSELECT || e == WM_LBUTTONUP => self.bring_up(),
            e if e == WM_CONTEXTMENU => self.tray_menu(x, y),
            // A reminder chosen: Kolada, with what is due.
            e if e == NIN_BALLOONUSERCLICK => {
                self.bring_up();
                self.open_agenda();
                self.notified();
            }
            e if e == NIN_BALLOONTIMEOUT || e == NIN_BALLOONHIDE => self.notified(),
            _ => {}
        }
    }

    fn tray_menu(&mut self, x: i32, y: i32) {
        let chosen = unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let new = if self.hotkey_held { w!("New note\tWin+Alt+N") } else { w!("New note") };
            let _ = AppendMenuW(menu, MF_STRING, NEW, new);
            let _ = AppendMenuW(menu, MF_STRING, TODAY, w!("Today's note"));
            let _ = AppendMenuW(menu, MF_STRING, SEARCH, w!("Search notes"));
            let _ = AppendMenuW(menu, MF_STRING, SHOW, w!("Show Kolada"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, QUIT, w!("Quit Kolada"));
            // The menu closes when the pointer goes elsewhere only if this
            // window is in front.
            let _ = SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0, x, y, self.hwnd, None);
            let _ = DestroyMenu(menu);
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            cmd.0 as usize
        };
        match chosen {
            NEW => {
                self.bring_up();
                self.fresh_page();
            }
            SEARCH => {
                self.bring_up();
                self.open_search();
            }
            TODAY => {
                self.bring_up();
                self.open_today();
            }
            SHOW => self.bring_up(),
            QUIT => self.quit(),
            _ => {}
        }
    }

    /// Takes Win+Alt+N from the system (or gives it back), as the options
    /// say. `said`: tell when another app has it.
    pub(super) fn update_hotkey(&mut self, said: bool) {
        use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_NOREPEAT, MOD_WIN, RegisterHotKey, UnregisterHotKey};
        // A note window leaves it to the main one; a test run leaves it to
        // the user's Kolada.
        if self.compact || kolada_core::testing() {
            return;
        }
        let want = self.session.hotkey == 0;
        unsafe {
            if want && !self.hotkey_held {
                self.hotkey_held = RegisterHotKey(Some(self.hwnd), QUICK_NOTE, MOD_WIN | MOD_ALT | MOD_NOREPEAT, b'N' as u32).is_ok();
                if !self.hotkey_held && said {
                    self.whisper("Another app has Win+Alt+N".to_string());
                }
            } else if !want && self.hotkey_held {
                let _ = UnregisterHotKey(Some(self.hwnd), QUICK_NOTE);
                self.hotkey_held = false;
            }
        }
    }

    /// Win+Alt+N, from any app: Kolada in front, on a fresh page (or the one
    /// already fresh).
    pub(super) fn quick_note(&mut self) {
        self.bring_up();
        if !(self.session.note.is_empty() && self.editor.text().trim().is_empty()) {
            self.fresh_page();
        }
    }

    /// Quits for real, even when closing would stay in the tray.
    pub(super) fn quit(&mut self) {
        self.quitting = true;
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    /// Whether closing the window should only hide it in the tray.
    pub(super) fn close_to_tray(&self) -> bool {
        !self.compact && !self.quitting && self.session.tray == 2 && self.tray_shown
    }
}
