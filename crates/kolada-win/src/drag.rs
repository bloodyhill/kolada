//! Files dragged over the window, seen as they come (OLE drag and drop):
//! what they are before they are let go, so the note can show what they
//! become and where they land (see `app::arrive`). Registered after the
//! first frame, which OLE stays off; until then, and if OLE cannot start,
//! `WM_DROPFILES` takes drops without showing them coming.

use std::path::PathBuf;

use windows::Win32::Foundation::{HWND, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop, ReleaseStgMedium,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_core::{Ref, Result, implement};

/// Starts taking drags over `hwnd`; false if OLE could not, and drops come
/// as `WM_DROPFILES` instead.
pub fn register(hwnd: HWND) -> bool {
    let target: IDropTarget = Target { hwnd }.into();
    unsafe { OleInitialize(None).is_ok() && RegisterDragDrop(hwnd, &target).is_ok() }
}

#[implement(IDropTarget)]
struct Target {
    hwnd: HWND,
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let files = data.as_ref().map(files_of).unwrap_or_default();
        let (x, y) = client(self.hwnd, pt);
        answer(effect, crate::app::drag_enter(files, x, y));
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let (x, y) = client(self.hwnd, pt);
        answer(effect, crate::app::drag_over(x, y));
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        crate::app::drag_leave();
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let files = data.as_ref().map(files_of).unwrap_or_default();
        let (x, y) = client(self.hwnd, pt);
        answer(effect, crate::app::drag_drop(files, x, y));
        Ok(())
    }
}

/// Copy if the files are taken; none if not (the pointer says so).
fn answer(effect: *mut DROPEFFECT, takes: bool) {
    if !effect.is_null() {
        unsafe { *effect = if takes { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
    }
}

/// A screen point in the window's client pixels.
fn client(hwnd: HWND, pt: &POINTL) -> (i32, i32) {
    let mut p = POINT { x: pt.x, y: pt.y };
    unsafe {
        let _ = ScreenToClient(hwnd, &mut p);
    }
    (p.x, p.y)
}

/// The files being dragged (Explorer's list of paths); none for text or
/// anything else.
fn files_of(data: &IDataObject) -> Vec<PathBuf> {
    let format = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    let Ok(mut medium) = (unsafe { data.GetData(&format) }) else { return Vec::new() };
    let drop = HDROP(unsafe { medium.u.hGlobal.0 });
    let files = unsafe {
        let count = DragQueryFileW(drop, u32::MAX, None);
        (0..count.min(10_000))
            .filter_map(|i| {
                let mut buf = vec![0u16; DragQueryFileW(drop, i, None) as usize + 1];
                let n = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
                (n > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
            })
            .collect()
    };
    unsafe { ReleaseStgMedium(&mut medium) };
    files
}
