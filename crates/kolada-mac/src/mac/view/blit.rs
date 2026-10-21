//! Putting a frame on the screen: only what changed. The model's surface
//! says where its last frame differs from the one before (`last_damage`).
//! The view draws a frame as soon as something changes and asks AppKit to
//! draw again only those rectangles (`setNeedsDisplayInRect:`); the layer
//! keeps the rest of what it showed. `drawRect:` then puts the frame's
//! pixels in the rectangles AppKit asks for, clipped to them.

use std::ffi::c_void;

use kolada_ui::raster::PxRect;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGInterpolationQuality};

/// A frame's size, pixels, and how many pixels a point is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub w: u32,
    pub h: u32,
    pub scale: f32,
}

/// A rectangle of the view, points from its top left: x, y, width, height.
pub type Points = (f64, f64, f64, f64);

/// Where AppKit must draw again after a frame of `size` whose damage is
/// `damage`, in the view's points, rounded out to whole points and kept
/// inside the view: None when it is everything (a new size, a new look),
/// empty when nothing changed.
pub fn points(damage: Option<&[PxRect]>, size: Size) -> Option<Vec<Points>> {
    let damage = damage?;
    let s = size.scale.max(0.01) as f64;
    let (vw, vh) = (size.w as f64 / s, size.h as f64 / s);
    let mut out = Vec::with_capacity(damage.len());
    for &(l, t, r, b) in damage {
        let (l, t) = ((l as f64 / s).floor().max(0.0), (t as f64 / s).floor().max(0.0));
        let (r, b) = ((r as f64 / s).ceil().min(vw), (b as f64 / s).ceil().min(vh));
        if r > l && b > t {
            out.push((l, t, r - l, b - t));
        }
    }
    Some(out)
}

/// The smallest rectangle holding all of `rects`: AppKit's `dirtyRect`.
#[cfg(test)]
pub fn union(rects: &[Points]) -> Option<Points> {
    let (mut l, mut t, mut r, mut b) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y, w, h) in rects {
        (l, t, r, b) = (l.min(x), t.min(y), r.max(x + w), b.max(y + h));
    }
    (r > l && b > t).then_some((l, t, r - l, b - t))
}

/// Puts the part `area` (points from the top left) of a `size` pixel
/// buffer (0x00RRGGBB, top-down rows) at the same place on a flipped view
/// in `ctx`, which AppKit has clipped to the rectangles it is drawing.
/// Only that part's pixels are read, while this call draws them, with no
/// copy: a keystroke reads its line, not the window.
///
/// `space` is the window's colour space (its display's), so the pixels
/// reach the screen as they are, as on Windows, however they get there.
/// Tagged as anything else they are matched to the display, but only when
/// they are drawn in part: the window server puts a whole new layer of a
/// big window up as it is (seen on the Mac mini's display: the caret's
/// #3EA9A9 came up as #039F9E in a typed line's rectangle and as itself
/// around it), and the line would show as a patch.
pub fn present(ctx: &CGContext, buf: &[u32], size: Size, area: Points, space: Option<&CGColorSpace>) {
    let (w, h) = (size.w as usize, size.h as usize);
    if buf.len() < w * h {
        return;
    }
    // The area in whole pixels, inside the buffer.
    let s = size.scale as f64;
    let (l, t) = (((area.0 * s).floor().max(0.0) as usize).min(w), ((area.1 * s).floor().max(0.0) as usize).min(h));
    let (r, b) = ((((area.0 + area.2) * s).ceil().max(0.0) as usize).min(w), (((area.1 + area.3) * s).ceil().max(0.0) as usize).min(h));
    if r <= l || b <= t {
        return;
    }
    let (pw, ph) = (r - l, b - t);
    // Rows of `w` pixels from the area's first, of which `pw` are read each.
    let start = t * w + l;
    let len = (ph - 1) * w + pw;
    let part = &buf[start..start + len];
    let provider = unsafe { CGDataProvider::with_data(std::ptr::null_mut(), part.as_ptr() as *const c_void, len * 4, None) };
    let device = if space.is_none() { CGColorSpace::new_device_rgb() } else { None };
    let space = space.or(device.as_deref());
    let info = CGBitmapInfo(CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::NoneSkipFirst.0);
    let image = unsafe { CGImage::new(pw, ph, 8, 32, w * 4, space.as_deref(), info, provider.as_deref(), std::ptr::null(), false, CGColorRenderingIntent::RenderingIntentDefault) };
    let Some(image) = image else { return };
    // Where it goes, points; the view is flipped, so the image is drawn
    // upside down unless the context is flipped back for it.
    let (x, y, pw_pt, ph_pt) = (l as f64 / s, t as f64 / s, pw as f64 / s, ph as f64 / s);
    CGContext::save_g_state(Some(ctx));
    CGContext::set_interpolation_quality(Some(ctx), CGInterpolationQuality::None);
    CGContext::translate_ctm(Some(ctx), x, y + ph_pt);
    CGContext::scale_ctm(Some(ctx), 1.0, -1.0);
    CGContext::draw_image(Some(ctx), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(pw_pt, ph_pt)), Some(&image));
    CGContext::restore_g_state(Some(ctx));
}

/// A context over `pixels` set up as AppKit sets up a flipped view's on a
/// screen of `size.scale` (points from the top left), for tests and the
/// speed gate: what `present` draws lands there as on the screen.
#[cfg(test)]
pub fn flipped_context(pixels: &mut [u32], size: Size) -> objc2_core_foundation::CFRetained<CGContext> {
    let space = CGColorSpace::new_device_rgb();
    let info = CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::NoneSkipFirst.0;
    let ctx = unsafe { objc2_core_graphics::CGBitmapContextCreate(pixels.as_mut_ptr() as *mut c_void, size.w as usize, size.h as usize, 8, size.w as usize * 4, space.as_deref(), info) }.expect("a bitmap context");
    CGContext::translate_ctm(Some(&ctx), 0.0, size.h as f64);
    CGContext::scale_ctm(Some(&ctx), size.scale as f64, -(size.scale as f64));
    ctx
}

/// What AppKit does before `drawRect:`: `present` into `ctx` clipped to
/// `rects` (None: the whole view), for tests and the speed gate.
#[cfg(test)]
pub fn present_in(ctx: &CGContext, buf: &[u32], size: Size, rects: Option<&[Points]>) {
    let whole = (0.0, 0.0, size.w as f64 / size.scale as f64, size.h as f64 / size.scale as f64);
    CGContext::save_g_state(Some(ctx));
    let area = match rects {
        None => Some(whole),
        Some(rects) => {
            let cg: Vec<CGRect> = rects.iter().map(|r| CGRect::new(CGPoint::new(r.0, r.1), CGSize::new(r.2, r.3))).collect();
            if !cg.is_empty() {
                unsafe { CGContext::clip_to_rects(Some(ctx), std::ptr::NonNull::new(cg.as_ptr() as *mut CGRect).unwrap(), cg.len()) };
            }
            // AppKit's `dirtyRect`: what the rectangles cover together.
            union(rects)
        }
    };
    if let Some(area) = area {
        present(ctx, buf, size, area, None);
    }
    CGContext::restore_g_state(Some(ctx));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size { w: 2000, h: 1200, scale: 2.0 };

    #[test]
    fn damage_becomes_whole_points_inside_the_view() {
        // Odd pixels round out to the points that hold them.
        assert_eq!(points(Some(&[(3, 5, 9, 11)]), SIZE), Some(vec![(1.0, 2.0, 4.0, 4.0)]));
        // Past the edges is cut off; nothing left is no rectangle.
        assert_eq!(points(Some(&[(-4, -4, 4, 4), (1990, 1190, 2100, 1300)]), SIZE), Some(vec![(0.0, 0.0, 2.0, 2.0), (995.0, 595.0, 5.0, 5.0)]));
        assert_eq!(points(Some(&[(2100, 0, 2200, 10)]), SIZE), Some(vec![]));
        // Nothing changed: nothing to draw. Everything: the whole view.
        assert_eq!(points(Some(&[]), SIZE), Some(vec![]));
        assert_eq!(points(None, SIZE), None);
    }

    #[test]
    fn a_frame_lands_on_the_screen_pixel_for_pixel_and_the_right_way_up() {
        let size = Size { w: 64, h: 40, scale: 2.0 };
        let frame: Vec<u32> = (0..size.w * size.h).map(|i| 0x0010_0000 * (i % 7) + 0x0000_0100 * (i / size.w) + (i % size.w)).collect();
        let mut screen = vec![0u32; frame.len()];
        let ctx = flipped_context(&mut screen, size);
        // A part first, then the rest: each lands where it is in the frame.
        present(&ctx, &frame, size, (5.0, 3.0, 9.0, 4.0), None);
        let colours = |p: &[u32]| p.iter().map(|c| c & 0x00FF_FFFF).collect::<Vec<_>>();
        for y in 0..40 {
            for x in 0..64 {
                let i = y * 64 + x;
                let inside = (10..28).contains(&x) && (6..14).contains(&y);
                assert_eq!(screen[i] & 0x00FF_FFFF, if inside { frame[i] & 0x00FF_FFFF } else { 0 }, "pixel {x}, {y}");
            }
        }
        present(&ctx, &frame, size, (0.0, 0.0, 32.0, 20.0), None);
        drop(ctx);
        // The x byte is not drawn: only the colours count.
        assert_eq!(colours(&screen), colours(&frame));
        assert_eq!(union(&[(1.0, 2.0, 3.0, 4.0), (10.0, 0.0, 1.0, 1.0)]), Some((1.0, 0.0, 10.0, 6.0)));
        assert_eq!(union(&[]), None);
    }

    #[test]
    fn drawing_only_the_damage_leaves_the_rest_as_it_was() {
        let size = Size { w: 40, h: 40, scale: 2.0 };
        let old = vec![0x0011_2233u32; 1600];
        let mut new = old.clone();
        for y in 10..14 {
            for x in 6..20 {
                new[y * 40 + x] = 0x00AA_BBCC;
            }
        }
        let mut screen = old.clone();
        let ctx = flipped_context(&mut screen, size);
        // As AppKit does: the context clipped to the rectangles asked for.
        let rects = points(Some(&[(6, 10, 20, 14), (30, 30, 31, 31)]), size).unwrap();
        present_in(&ctx, &new, size, Some(&rects));
        drop(ctx);
        assert_eq!(screen.iter().map(|c| c & 0x00FF_FFFF).collect::<Vec<_>>(), new);
    }

    #[test]
    fn a_keystroke_asks_appkit_for_a_sliver_and_the_screen_ends_as_a_whole_frame() {
        let dir = std::env::temp_dir().join(format!("kolada-mac-blit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let body: String = (0..40).map(|i| format!("Line {i} of a note with words enough to fill a row.\n\n")).collect();
        std::fs::write(dir.join("Plan.md"), format!("# Plan\n\n{body}")).unwrap();
        let mut m = kolada_app::Model::new(dir.clone(), false);
        let size = Size { w: 2400, h: 1600, scale: 2.0 };
        m.frame(size.w, size.h, size.scale);
        let mut screen = vec![0u32; (size.w * size.h) as usize];
        let ctx = flipped_context(&mut screen, size);
        present_in(&ctx, m.surface().pixels(), size, None);
        for key in ["a", "b", "c"] {
            m.insert(key);
            m.frame(size.w, size.h, size.scale);
            let rects = points(m.surface().last_damage.as_deref(), size).expect("a keystroke changes part of the window");
            let area: f64 = rects.iter().map(|r| r.2 * r.3).sum();
            let whole = (size.w * size.h) as f64 / 4.0;
            assert!(area > 0.0 && area < whole * 0.05, "a keystroke redraws {:.1}% of the window", 100.0 * area / whole);
            present_in(&ctx, m.surface().pixels(), size, Some(&rects));
        }
        drop(ctx);
        let colours = |p: &[u32]| p.iter().map(|c| c & 0x00FF_FFFF).collect::<Vec<_>>();
        assert!(colours(&screen) == colours(m.surface().pixels()), "what the parts drew is the whole frame");
        let _ = std::fs::remove_dir_all(dir);
    }
}
