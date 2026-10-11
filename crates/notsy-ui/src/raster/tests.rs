use super::*;

#[test]
fn blends_exactly_at_the_ends_and_halfway() {
    assert_eq!(blend(0x000000, 0xFFFFFF, 255), 0xFFFFFF);
    assert_eq!(blend(0xFFFFFF, 0x000000, 0), 0xFFFFFF);
    assert_eq!(blend(0x102030, 0x102030, 77), 0x102030);
    let half = blend(0x000000, 0xFFFFFF, 128);
    assert!((0x7F7F7F..=0x808080).contains(&half), "{half:06x}");
    assert_eq!(blend(0x00FF00, 0xFF0000, 255), 0xFF0000);
}

#[test]
fn contrast_table_keeps_the_ends_and_only_increases() {
    let t = contrast_table(CONTRAST);
    assert_eq!((t[0], t[255]), (0, 255));
    assert!(t.windows(2).all(|w| w[0] <= w[1]));
    assert!(t[128] >= 128);
}

#[test]
fn splits_into_bands_covering_every_row() {
    for h in [1, 2, 3, 7, 100, 2177] {
        for n in [1, 2, 4, 8] {
            let bands = split(h, n);
            assert!(!bands.is_empty() && bands.len() <= n);
            assert_eq!(bands[0].0, 0);
            assert_eq!(bands.iter().map(|b| b.1).sum::<u32>(), h, "height {h}: {bands:?}");
            for pair in bands.windows(2) {
                assert_eq!(pair[0].0 + pair[0].1, pair[1].0);
            }
        }
    }
}

#[test]
fn blob_melts_into_its_shape_through_a_neck() {
    use crate::display::Shape;
    use crate::geom::{Blob, Rect};
    // A thin vertical pill with a circle a little to its right.
    let pill = Rect::new(10.0, 10.0, 16.0, 110.0);
    let base = Shape { rect: pill, radius: 3.0, blur: 0.0, color: 0xFFFFFF, alpha: 1.0, ring: false, blob: None, wave: None };
    let melted = Shape { blob: Some(Blob { x: 24.0, y: 60.0, radius: 5.0, smooth: 7.0 }), ..base };
    let (a, b) = (PxShape::new(&base, 1.0), PxShape::new(&melted, 1.0));
    // Inside the pill both are solid; inside the circle only the melted one.
    assert_eq!((a.coverage(13.0, 60.0), b.coverage(13.0, 60.0)), (1.0, 1.0));
    assert_eq!(a.coverage(24.0, 60.0), 0.0);
    assert_eq!(b.coverage(24.0, 60.0), 1.0);
    // The 3-pixel gap between them (x = 16..19) is bridged by the neck:
    // one shape, not two.
    assert!(a.coverage(17.5, 60.0) == 0.0 && b.coverage(17.5, 60.0) > 0.6, "neck missing");
    // Far from the circle the pill keeps its edge.
    assert_eq!(b.coverage(18.5, 15.0), 0.0);
    // Bounds grow to include the circle and its neck.
    assert!(b.bounds().2 >= 24.0 + 5.0);
}
