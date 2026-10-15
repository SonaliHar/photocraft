//! UNRUN three-leaf CURRENT consumer. Engine transaction/locks/history are
//! independent; this uses literal raw source bytes and the existing Normal kernel.
use super::*;
use crate::backed_normal_stack::{NormalStackLayer, PreparedBackedNormalStack};

#[test]
#[ignore = "UNRUN: two independently translated native leaves retain COW words and compose in saved order; unrelated third leaf and source remain exact"]
fn two_moves_compose_independently_without_changing_unrelated_leaf() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in SampleType::ALL {
            let format = PixelFormat::new(mode, sample, true);
            let (root, store, raw, quota) = fixture(format);
            let original = root.descriptor();
            let replacement = pixel(format, 0.875, 0.625);
            let edited_root = root.try_edited_native(Rect::new(-257, -1, -256, 0), &replacement, &ctl()).unwrap();
            let unrelated = root.try_translated(257, 255, &ctl()).unwrap();
            let unchanged = unrelated.clone();
            let first = root.try_translated(258, 255, &ctl()).unwrap();
            let second = edited_root.try_translated(259, 255, &ctl()).unwrap();
            let profile = if mode == ColorMode::Rgb { Builtin::Srgb.profile() } else { Builtin::SGray.profile() };
            let working = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, space(SampleEncoding::NonlinearProfile)).unwrap();
            let inputs = [
                NormalStackLayer { layer: &first, visible: true, opacity: 0.375, mask: None },
                NormalStackLayer { layer: &second, visible: true, opacity: 0.625, mask: None },
                NormalStackLayer { layer: &unrelated, visible: true, opacity: 0.5, mask: None },
            ];
            let prepared = PreparedBackedNormalStack::new(
                &inputs,
                &quota,
                profile,
                SampleEncoding::NonlinearProfile,
                space(SampleEncoding::NonlinearProfile),
                Default::default(),
                &ctl(),
            )
            .unwrap();
            let roi = Rect::new(-2, 253, 9, 258);
            let band = prepared.render_band(roi, [0.; 4], &ctl()).unwrap();
            for y in roi.y0..roi.y1 {
                for x in roi.x0..roi.x1 {
                    let mut expected = [0.; 4];
                    for (index, (dx, opacity)) in [(258, 0.375), (259, 0.625), (257, 0.5)].into_iter().enumerate() {
                        let (sx, sy) = (x - dx, y - 255);
                        if !original.bounds().contains(sx, sy) {
                            continue;
                        }
                        let native =
                            if index == 1 && (sx, sy) == (-257, -1) { replacement.as_slice() } else { expected_pixel(&raw, format, original.bounds(), sx, sy) };
                        expected = working.normal(expected, rgba(native, format), opacity, GimpOperator::Union).unwrap();
                    }
                    let at = ((y - roi.y0) as usize * roi.width() as usize) + (x - roi.x0) as usize;
                    assert_eq!(band.pixels()[at].map(f64::to_bits), expected.map(f64::to_bits));
                }
            }
            assert_eq!(unrelated, unchanged);
            assert_eq!(unrelated.default_bytes(), root.default_bytes());
            assert_eq!(root.descriptor(), original);
            assert_eq!(store.bytes, raw);
            assert_eq!(exact_pixel(&second, 2, 254), replacement);
            assert_eq!(exact_pixel(&first, 1, 254), expected_pixel(&raw, format, original.bounds(), -257, -1));
            // Inspect every logical raw word of the unrelated leaf using each
            // genuine target pin once; no display value can hide native changes.
            for coord in unrelated.operative_bounds().tiles() {
                let pin = unrelated.checked_view().pin_tile(coord, &ctl()).unwrap();
                let part = coord.rect().intersect(&unrelated.operative_bounds());
                let mut actual = vec![0; format.bytes_per_pixel()];
                for y in part.y0..part.y1 {
                    for x in part.x0..part.x1 {
                        pin.read_pixel_native_into(x, y, &mut actual, &ctl()).unwrap();
                        assert_eq!(actual, expected_pixel(&raw, format, original.bounds(), x - 257, y - 255));
                    }
                }
            }
        }
    }
}
