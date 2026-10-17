//! UNRUN/uncompiled bounded-core regressions. Surfaces, packed arrays and
//! comparison Vecs here are external harness storage, not catalog quota proof.
//! No source/profile/producer or numerical GIMP qualification is asserted.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
fn limits() -> GaussianLimits {
    GaussianLimits::default()
}
fn pack(source: &Surface, plan: BoxBlurTilePlan) -> Vec<[f64; 4]> {
    let mut input = Vec::new();
    for y in plan.source_read().y0..plan.source_read().y1 {
        for x in plan.source_read().x0..plan.source_read().x1 {
            let mut p = [0.; 4];
            source.try_read_pixel_f64(x, y, &mut p).unwrap();
            input.push(p);
        }
    }
    assert_eq!(input.len(), plan.packed_input_pixels().unwrap());
    input
}
fn procedural(bounds: Rect) -> Surface {
    let mut source = Surface::new(format());
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            let i = (x - bounds.x0) as u32;
            let j = (y - bounds.y0) as u32;
            let alpha = if (i + j).is_multiple_of(11) { 0. } else { ((i * 3 + j * 7) % 31) as f64 / 30. };
            source.try_write_pixel_f64(x, y, &[i as f64 / 137. - 0.25, j as f64 / 113., ((i * 7 + j * 13) % 97) as f64 / 61., alpha]).unwrap();
        }
    }
    source
}
#[test]
#[ignore = "UNRUN shared-core old full-output exact-word oracle"]
fn packed_tiles_match_old_full_operation_across_negative_and_tile_crossing_extents() {
    for source_bounds in [Rect::new(0, 0, 16, 12), Rect::new(-3, -2, 263, 259)] {
        let source = procedural(source_bounds);
        let before = source.clone();
        for radius in [4, 5] {
            let output = box_blur_extent(source_bounds, radius).unwrap();
            let whole = try_box_blur(&source, output, source_bounds, radius, limits(), &Interrupt::NONE).unwrap();
            for coord in output.tiles() {
                let plan = BoxBlurTilePlan::new(source_bounds, coord, radius, &Interrupt::NONE).unwrap();
                let packed = pack(&source, plan);
                let required = plan.requirements(&Interrupt::NONE).unwrap();
                let raw = try_box_blur_packed_tile(
                    &plan,
                    &packed,
                    GaussianLimits { resident_bytes: required.peak_resident_bytes, work_units: required.work_units },
                    &Interrupt::NONE,
                )
                .unwrap();
                assert_eq!(raw, whole.try_read_interleaved(plan.tile_output()).unwrap());
                assert_eq!(raw.len(), plan.output_bytes().unwrap());
                assert!(raw.capacity() <= raw.len() * 2);
            }
            assert_eq!(source, before);
            assert_eq!(whole.default_bytes(), source.default_bytes());
        }
    }
}
fn crop(raw: &[u8], full: Rect, roi: Rect) -> Vec<u8> {
    assert!(full.contains_rect(&roi));
    let mut out = Vec::new();
    for y in roi.y0..roi.y1 {
        let start = ((y - full.y0) as usize * full.width() as usize + (roi.x0 - full.x0) as usize) * 32;
        out.extend_from_slice(&raw[start..start + roi.width() as usize * 32]);
    }
    out
}
#[test]
#[ignore = "UNRUN full canonical reset then arbitrary thin crop"]
fn lower_band_limits_and_reordered_crops_use_full_output_canonical_tile_oracle() {
    let bounds = Rect::new(-7, -5, 270, 19);
    let source = procedural(bounds);
    let output = box_blur_extent(bounds, 5).unwrap();
    let whole = try_box_blur(&source, output, bounds, 5, limits(), &Interrupt::NONE).unwrap();
    let mut coords: Vec<_> = output.tiles().collect();
    coords.reverse();
    for coord in coords {
        let plan = BoxBlurTilePlan::new(bounds, coord, 5, &Interrupt::NONE).unwrap();
        let packed = pack(&source, plan);
        let raw = try_box_blur_packed_tile(&plan, &packed, limits(), &Interrupt::NONE).unwrap();
        let t = plan.tile_output();
        for y in (t.y0..t.y1).rev() {
            let roi = Rect::new(t.x0, y, t.x1, y + 1);
            assert_eq!(crop(&raw, t, roi), whole.try_read_interleaved(roi).unwrap());
        }
        assert_eq!(raw, try_box_blur_packed_tile(&plan, &packed, limits(), &Interrupt::NONE).unwrap());
    }
    // An old kernel invoked directly on each thin ROI resets its sums there;
    // it is deliberately NOT the canonical full-output bitwise oracle.
}
#[test]
#[ignore = "UNRUN large logical geometry without full-image allocation"]
fn logical_36mp_and_negative_coordinates_keep_support_bounded_and_reject_bad_geometry() {
    for bounds in [Rect::new(-13, 17, 5987, 6017), Rect::new(0, 0, 6000, 4000)] {
        // No full native/linear source is allocated. Logical F64 bytes exceed
        // wasm512MiB even while this plan's actual support/output is bounded.
        assert!(u64::from(bounds.width()) * u64::from(bounds.height()) * 32 > 512 << 20);
        for coord in [TileCoord::containing(bounds.x0, bounds.y0), TileCoord::containing(bounds.x1 - 1, bounds.y1 - 1)] {
            let plan = BoxBlurTilePlan::new(bounds, coord, 5, &Interrupt::NONE).unwrap();
            assert!(bounds.contains_rect(&plan.source_read()));
            assert!(plan.source_read().width() <= 266 && plan.source_read().height() <= 266);
            assert!(plan.packed_input_pixels().unwrap() <= 266 * 266);
            assert!(plan.output_bytes().unwrap() <= 256 * 256 * 32);
            assert!(plan.requirements(&Interrupt::NONE).unwrap().peak_resident_bytes <= 2 * (256 * 266 * 32 + 256 * 256 * 32) + 4096);
        }
    }
    let edge = Rect::new(-(1 << 29) + 5, 0, -(1 << 29) + 21, 12);
    let plan = BoxBlurTilePlan::new(edge, TileCoord::containing(edge.x0, edge.y0), 5, &Interrupt::NONE).unwrap();
    assert!(plan.support().x0 < -(1 << 29));
    assert!(edge.contains_rect(&plan.source_read()));
    for (source, tile, radius) in [
        (Rect::EMPTY, TileCoord::new(0, 0), 5),
        (Rect::new(0, 0, 16, 12), TileCoord::new(i32::MAX, i32::MIN), 5),
        (Rect::new(0, 0, 16, 12), TileCoord::new(0, 0), 1001),
        (Rect::new(0, 0, 16, 12), TileCoord::new(99, 99), 5),
        (Rect::new(-(1 << 29), 0, -(1 << 29) + 16, 12), TileCoord::new(0, 0), 5),
    ] {
        assert!(BoxBlurTilePlan::new(source, tile, radius, &Interrupt::NONE).is_err());
    }
}
#[test]
#[ignore = "UNRUN bounds, honest requirements and early/during-kernel cancellation"]
fn requirements_limits_and_cancel_refuse_privately_without_modifying_borrowed_input() {
    let bounds = Rect::new(0, 0, 512, 512);
    let plan = BoxBlurTilePlan::new(bounds, TileCoord::new(0, 0), 5, &Interrupt::NONE).unwrap();
    let input = vec![[0.25, -0.125, 2., 0.75]; plan.packed_input_pixels().unwrap()];
    let expected = input.clone();
    let req = plan.requirements(&Interrupt::NONE).unwrap();
    for limit in [
        GaussianLimits { resident_bytes: req.peak_resident_bytes - 1, work_units: req.work_units },
        GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units - 1 },
    ] {
        assert_eq!(try_box_blur_packed_tile(&plan, &input, limit, &Interrupt::NONE), Err(RasterInputError::LimitExceeded));
    }
    assert_eq!(try_box_blur_packed_tile(&plan, &input[..input.len() - 1], limits(), &Interrupt::NONE), Err(RasterInputError::LengthMismatch));
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert_eq!(try_box_blur_packed_tile(&plan, &input, limits(), &interrupt), Err(RasterInputError::Cancelled));
    let polls = AtomicUsize::new(0);
    let after_validation = input.len().div_ceil(64) + 3;
    let stop = || polls.fetch_add(1, Ordering::Relaxed) >= after_validation;
    let interrupt = Interrupt::cancel_only(&stop);
    assert_eq!(try_box_blur_packed_tile(&plan, &input, limits(), &interrupt), Err(RasterInputError::Cancelled));
    assert!(polls.load(Ordering::Relaxed) > after_validation);
    assert_eq!(input, expected);
    for bad in [[f64::NAN, 0., 0., 1.], [0., 0., 0., 1.001]] {
        let mut malformed = input.clone();
        malformed[0] = bad;
        assert_eq!(try_box_blur_packed_tile(&plan, &malformed, limits(), &Interrupt::NONE), Err(RasterInputError::InvalidSample));
    }
}
#[test]
#[ignore = "UNRUN original-S Clamp versus None and radius0 raw identity"]
fn expanded_halo_repeats_original_endpoint_and_zero_identity_keeps_hidden_raw_words() {
    let bounds = Rect::new(0, 0, 3, 2);
    let source = Surface::try_with_default_bytes(format(), &[0.25f64, -0.5, 2., 1.].iter().flat_map(|v| v.to_ne_bytes()).collect::<Vec<_>>()).unwrap();
    let output = box_blur_extent(bounds, 4).unwrap();
    for coord in output.tiles() {
        let plan = BoxBlurTilePlan::new(bounds, coord, 4, &Interrupt::NONE).unwrap();
        let packed = pack(&source, plan);
        let raw = try_box_blur_packed_tile(&plan, &packed, limits(), &Interrupt::NONE).unwrap();
        for p in raw.as_chunks::<32>().0 {
            let expected: Vec<_> = [0.25f64, -0.5, 2., 1.].iter().flat_map(|v| v.to_ne_bytes()).collect();
            assert_eq!(p, expected.as_slice());
        }
    }
    // At the expanded edge None would lose opaque taps. This asserts old
    // Clamp-to-original-S only; it is not independent GIMP evidence.
    let plan = BoxBlurTilePlan::new(bounds, TileCoord::new(0, 0), 0, &Interrupt::NONE).unwrap();
    let input = vec![[-0.0, 2., 0.5, -0.0]; plan.packed_input_pixels().unwrap()];
    let raw = try_box_blur_packed_tile(&plan, &input, limits(), &Interrupt::NONE).unwrap();
    let expected: Vec<u8> = input.iter().flat_map(|p| p.iter().flat_map(|v| v.to_ne_bytes())).collect();
    assert_eq!(raw, expected);
    let source = Surface::try_with_default_bytes(format(), &expected[..32]).unwrap();
    let legacy = try_box_blur(&source, bounds, bounds, 0, GaussianLimits { resident_bytes: 0, work_units: 0 }, &Interrupt::NONE).unwrap();
    assert_eq!(legacy.default_bytes(), source.default_bytes());
    assert_eq!(legacy, source);
    let nonzero = BoxBlurTilePlan::new(bounds, TileCoord::new(0, 0), 4, &Interrupt::NONE).unwrap();
    let input = vec![[-0.0, 2., 0.5, -0.0]; nonzero.packed_input_pixels().unwrap()];
    assert!(try_box_blur_packed_tile(&nonzero, &input, limits(), &Interrupt::NONE).unwrap().iter().all(|b| *b == 0));
}
