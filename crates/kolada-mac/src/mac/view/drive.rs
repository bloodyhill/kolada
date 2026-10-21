//! Debug builds only: typing with no keyboard, to see over SSH what
//! reaches the window server. `KOLADA_DRIVE_KEYS=abc` types each character
//! as AppKit hands typing over (`insertText:replacementRange:`), lets the
//! frame reach the window server, and reads the window back (an app may
//! read its own windows with no permission); then the whole view is drawn
//! again and read again. The two must be the same, pixel for pixel (what
//! the drawn rectangles left on the screen is the whole frame), and the
//! frame's own pixels, as they are (outside the traffic lights and the
//! window's corners). Prints what each key drew and whether it matched,
//! then quits.
//!
//!     KOLADA_TEST_INSTANCE=1 KOLADA_DRIVE_KEYS="Hi there" target/debug/kolada-mac <a generated vault>

use std::cell::{Cell, RefCell};

use objc2::{DefinedClass, MainThreadMarker, msg_send, sel};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGDataProvider, CGImage};
use objc2_foundation::{NSNotFound, NSObjectNSDelayedPerforming, NSRange, NSRect, NSString};

use super::KoladaView;

thread_local! {
    static DRAWN: RefCell<Vec<NSRect>> = const { RefCell::new(Vec::new()) };
    static STEP: Cell<usize> = const { Cell::new(0) };
}

const GAP: f64 = 0.15;

fn keys() -> Option<Vec<char>> {
    std::env::var("KOLADA_DRIVE_KEYS").ok().filter(|k| !k.is_empty()).map(|k| k.chars().collect())
}

/// `drawRect:` drew `dirty`.
pub fn drew(dirty: NSRect) {
    DRAWN.with(|d| d.borrow_mut().push(dirty));
}

/// After the first frame: starts typing when asked to.
pub fn start(view: &KoladaView) {
    if keys().is_some() {
        unsafe { view.performSelector_withObject_afterDelay(sel!(driveStep), None, 0.5) };
    }
}

unsafe extern "C" {
    fn CGWindowListCreateImage(bounds: CGRect, list: u32, window: u32, options: u32) -> *mut CGImage;
}

/// The window as the window server shows it: its pixels.
fn capture(view: &KoladaView) -> Option<Vec<u8>> {
    let window = view.window()?;
    // Only this window, its content as drawn at the screen's resolution.
    let null = CGRect::new(CGPoint::new(f64::INFINITY, f64::INFINITY), CGSize::new(0.0, 0.0));
    let image = unsafe { CGWindowListCreateImage(null, 1 << 3, window.windowNumber() as u32, 1 | 8) };
    let image = unsafe { objc2_core_foundation::CFRetained::from_raw(std::ptr::NonNull::new(image)?) };
    let data = CGDataProvider::data(CGImage::data_provider(Some(&image)).as_deref())?;
    ROW.with(|r| r.set(CGImage::bytes_per_row(Some(&image)) / 4));
    Some(data.to_vec())
}

thread_local! {
    /// Pixels a row of the last window read holds.
    static ROW: Cell<usize> = const { Cell::new(1) };
}

/// The window as the window server shows it, written to `path` as a
/// 32-bit BMP: its size in pixels.
pub fn capture_bmp(view: &KoladaView, path: &std::path::Path) -> std::io::Result<(usize, usize)> {
    let pixels = capture(view).ok_or_else(|| std::io::Error::other("the window could not be read"))?;
    let row = ROW.with(|r| r.get()).max(1);
    let scale = view.window().map_or(1.0, |w| w.backingScaleFactor());
    let w = ((view.bounds().size.width * scale) as usize).min(row);
    let h = pixels.len() / 4 / row;
    let mut b = Vec::with_capacity(54 + w * h * 4);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&((54 + w * h * 4) as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(-(h as i32)).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(&[0u8; 24]);
    for y in 0..h {
        b.extend_from_slice(&pixels[y * row * 4..(y * row + w) * 4]);
    }
    std::fs::write(path, b)?;
    Ok((w, h))
}

/// Pixels that differ in `a` and `b`, 4 bytes each: by more than the
/// window server's rounding (a level or two of a channel, where it
/// matches colours to the display), and at all.
fn differ(a: &[u8], b: &[u8]) -> (usize, usize) {
    let (mut much, mut any) = (0, 0);
    for (x, y) in a.chunks(4).zip(b.chunks(4)) {
        if x != y {
            any += 1;
            if x.iter().zip(y).any(|(p, q)| p.abs_diff(*q) > 3) {
                much += 1;
            }
        }
    }
    (much, any)
}

/// While the screen is compared, no frame is drawn (`refresh` waits).
pub fn frozen() -> bool {
    FROZEN.with(|f| f.get())
}

thread_local! {
    static FROZEN: Cell<bool> = const { Cell::new(false) };
    static SHOT: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    static DIFFERED: Cell<usize> = const { Cell::new(0) };
}

/// One step: a key typed; the screen held still and read; the same frame
/// put up whole and read again.
pub fn step(view: &KoladaView) {
    let Some(keys) = keys() else { return };
    let n = STEP.with(|s| s.get());
    STEP.with(|s| s.set(n + 1));
    let key = n / 4;
    let next = |view: &KoladaView| unsafe { view.performSelector_withObject_afterDelay(sel!(driveStep), None, GAP) };
    // Something still moving (the caret gliding, the chrome fading as
    // typing starts): the screen is held still once it is at rest.
    let moving = view.ivars().state.try_borrow().map_or(true, |st| st.model.wants_frames());
    if n % 4 == 1 && moving {
        STEP.with(|s| s.set(n));
        return next(view);
    }
    if key >= keys.len() {
        let bad = DIFFERED.with(|d| d.get());
        eprintln!("drive: {} keys typed; {}", keys.len(), if bad == 0 { "each left the screen as the whole frame".to_string() } else { format!("{bad} left the screen unlike the whole frame") });
        objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::from(view)).terminate(None);
        return;
    }
    let verbose = std::env::var_os("KOLADA_DRIVE_WHERE").is_some();
    match n % 4 {
        0 => {
            DRAWN.with(|d| d.borrow_mut().clear());
            let text = NSString::from_str(&keys[key].to_string());
            let _: () = unsafe { msg_send![view, insertText: &*text, replacementRange: NSRange::new(NSNotFound as usize, 0)] };
            view.displayIfNeeded();
            next(view);
        }
        1 => {
            // At rest: what is asked for is drawn, and then nothing more
            // until the screen has been read twice.
            view.displayIfNeeded();
            FROZEN.with(|f| f.set(true));
            let bounds = view.bounds();
            let rects = DRAWN.with(|d| std::mem::take(&mut *d.borrow_mut()));
            let area: f64 = rects.iter().map(|r| r.size.width * r.size.height).sum();
            let whole = bounds.size.width * bounds.size.height;
            let scale = view.window().map_or(0.0, |w| w.backingScaleFactor());
            eprintln!("drive: key {}: drawRect {} time(s), {:.1}% of the window ({:.0} x {:.0} points at {scale}x)", key + 1, rects.len(), 100.0 * area / whole.max(1.0), bounds.size.width, bounds.size.height);
            if verbose {
                let list: Vec<_> = rects.iter().map(|r| (r.origin.x, r.origin.y, r.size.width, r.size.height)).collect();
                eprintln!("drive:   drew {list:?}");
            }
            next(view);
        }
        2 => {
            SHOT.with(|s| *s.borrow_mut() = capture(view));
            if verbose {
                if let Ok(st) = view.ivars().state.try_borrow() {
                    eprintln!("drive:   read; the frame's sum {}", st.model.surface().pixels().iter().map(|&c| c as u64).sum::<u64>());
                }
            }
            // The same frame put up whole (no new frame is drawn).
            view.setNeedsDisplay(true);
            view.displayIfNeeded();
            next(view);
        }
        _ => {
            let parts = SHOT.with(|s| s.borrow_mut().take());
            let whole = capture(view);
            match (parts, whole) {
                (Some(a), Some(b)) if a.len() == b.len() => {
                    let (much, any) = differ(&a, &b);
                    eprintln!("drive: key {}: of {} pixels, {much} differ from the frame put up whole ({any} by the window server's rounding or more)", key + 1, a.len() / 4);
                    // And the screen against the frame itself: its pixels as they are.
                    if let Ok(st) = view.ivars().state.try_borrow() {
                        let size = view.pixel_size();
                        let row = ROW.with(|r| r.get());
                        let p = st.model.surface().pixels();
                        let mut off = 0;
                        let mut bbox = (usize::MAX, usize::MAX, 0, 0);
                        // The traffic lights are AppKit's, over the frame, and the
                        // window's rounded corners cut it.
                        let (w, h, s) = (size.w as f32, size.h as f32, size.scale);
                        let lights = |x: usize, y: usize| {
                            let (x, y) = (x as f32, y as f32);
                            let corner = (x < 32.0 * s || x >= w - 32.0 * s) && (y < 32.0 * s || y >= h - 32.0 * s);
                            corner || (x < 90.0 * s && y < 40.0 * s)
                        };
                        for y in 0..(size.h as usize).min(b.len() / 4 / row.max(1)) {
                            for x in (0..size.w as usize).filter(|&x| !lights(x, y)) {
                                let o = (y * row + x) * 4;
                                let c = u32::from_le_bytes([b[o], b[o + 1], b[o + 2], 0]);
                                if c != p[y * size.w as usize + x] & 0x00FF_FFFF {
                                    off += 1;
                                    bbox = (bbox.0.min(x), bbox.1.min(y), bbox.2.max(x), bbox.3.max(y));
                                }
                            }
                        }
                        eprintln!("drive: key {}: {off} pixels on the screen are not the frame's own{}", key + 1, if off > 0 { format!(" (in {bbox:?})") } else { String::new() });
                        if off > 0 {
                            DIFFERED.with(|d| d.set(d.get() + 1));
                        }
                    }
                    if any > 0 && verbose {
                        let w = ROW.with(|r| r.get()).max(1);
                        let (mut l, mut t, mut r, mut bt) = (usize::MAX, usize::MAX, 0, 0);
                        let mut first = None;
                        for (i, (x, y)) in a.chunks(4).zip(b.chunks(4)).enumerate() {
                            if x.iter().zip(y).any(|(p, q)| p.abs_diff(*q) > if much > 0 { 3 } else { 0 }) {
                                let (px, py) = (i % w, i / w);
                                (l, t, r, bt) = (l.min(px), t.min(py), r.max(px), bt.max(py));
                                first.get_or_insert((px, py, x.to_vec(), y.to_vec()));
                            }
                        }
                        eprintln!("drive:   rows of {w} pixels; they differ in {l},{t} to {r},{bt}; first {first:?}");
                        if let (Some((x, y, _, _)), Ok(st)) = (&first, view.ivars().state.try_borrow()) {
                            let size = view.pixel_size();
                            let p = st.model.surface().pixels();
                            let at = |x: usize, y: usize| p.get(y * size.w as usize + x).copied().unwrap_or(0);
                            eprintln!("drive:   the frame there: {:06x} {:06x} {:06x}, sum {}", at(x - 1, *y), at(*x, *y), at(x + 1, *y), p.iter().map(|&c| c as u64).sum::<u64>());
                        }
                    }
                    if much > 0 {
                        DIFFERED.with(|d| d.set(d.get() + 1));
                    }
                }
                _ => eprintln!("drive: key {}: the window could not be read back", key + 1),
            }
            FROZEN.with(|f| f.set(false));
            view.refresh();
            next(view);
        }
    }
}
