//! The Windows shell's part of drawing: the pixels `kolada_ui::raster` blends,
//! copied to the window with GDI, and the first frame's surface made on a
//! worker thread while the scene is still being built.
//!
//! Worker threads never call GDI or USER: a thread that does becomes a
//! kernel "GUI thread", and those contend with window creation on the main
//! thread (CreateWindowEx measured 2-3 ms alone, 7 ms next to GDI workers).

use std::sync::mpsc;
use std::thread;

use windows::Win32::Foundation::{E_FAIL, HWND};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GdiFlush, GetDC, HDC, ReleaseDC, SetDIBitsToDevice};
use windows::core::{Error, Result};

use crate::Handoff;
use crate::display::DisplayList;
pub use kolada_ui::raster::{PxRect, Surface, THREADS};

/// A surface whose buffer is being allocated and cleared on another thread
/// while the display list is still being built.
pub struct PendingSurface {
    list: mpsc::Sender<Handoff<DisplayList>>,
    worker: thread::JoinHandle<Handoff<Surface>>,
}

impl PendingSurface {
    pub fn finish(self, list: DisplayList) -> Result<Surface> {
        // If the worker already failed, the error is the join's.
        let _ = self.list.send(Handoff(list));
        Ok(self.worker.join().map_err(|_| Error::from_hresult(E_FAIL))?.0)
    }
}

/// Allocates and clears the buffer on worker threads right away, then draws
/// the list passed to `PendingSurface::finish`.
pub fn start(view: (u32, u32, u32), background: u32) -> PendingSurface {
    let (tx, rx) = mpsc::channel::<Handoff<DisplayList>>();
    let worker = thread::spawn(move || {
        let mut surface = Surface::new(view);
        surface.clear(background, THREADS);
        let list = rx.recv().expect("the first frame's list comes").0;
        surface.paint(&list, list.background != background, THREADS, false);
        Handoff(surface)
    });
    PendingSurface { list: tx, worker }
}

/// Copies the pixels the last draw changed to the window (all of them, if
/// it changed everything).
pub fn blit_changed(surface: &Surface, hwnd: HWND) {
    match &surface.last_damage {
        None => blit(surface, hwnd, None),
        Some(rects) if rects.is_empty() => {}
        Some(rects) => unsafe {
            let dc = GetDC(Some(hwnd));
            for &r in rects {
                blit_rect(surface, dc, r);
            }
            // Once, for all of them.
            let _ = GdiFlush();
            ReleaseDC(Some(hwnd), dc);
        },
    }
}

fn bitmap_info(width: u32, rows: u32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(rows as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Copies one rectangle of the buffer to the same place in `dc`.
fn blit_rect(surface: &Surface, dc: HDC, (l, t, r, b): PxRect) {
    let rows = (b - t) as u32;
    let info = bitmap_info(surface.width, rows);
    let start = &surface.pixels[(t as u32 * surface.width) as usize..];
    unsafe {
        SetDIBitsToDevice(dc, l, t, (r - l) as u32, rows, l, 0, 0, rows, start.as_ptr().cast(), &info, DIB_RGB_COLORS);
    }
}

/// Copies rows `top..top + rows` of the buffer to the same rows of `dc`.
fn blit_rows(surface: &Surface, dc: HDC, top: u32, rows: u32) {
    let info = bitmap_info(surface.width, rows);
    let start = &surface.pixels[(top * surface.width) as usize..];
    unsafe {
        SetDIBitsToDevice(dc, 0, top as i32, surface.width, rows, 0, 0, 0, rows, start.as_ptr().cast(), &info, DIB_RGB_COLORS);
        // The copy is complete when this returns, so the window can be
        // uncloaked immediately: DWM's next frame already has it.
        let _ = GdiFlush();
    }
}

/// Copies the buffer to `dc` (during WM_PAINT), or to the window's DC. One
/// call on this thread: splitting the copy across threads measured slower,
/// because GDI serializes it and every thread that calls GDI first has to
/// become a GUI thread.
pub fn blit(surface: &Surface, hwnd: HWND, dc: Option<HDC>) {
    match dc {
        Some(dc) => blit_rows(surface, dc, 0, surface.height),
        None => unsafe {
            let dc = GetDC(Some(hwnd));
            blit_rows(surface, dc, 0, surface.height);
            ReleaseDC(Some(hwnd), dc);
        },
    }
}

#[cfg(test)]
mod tests;
