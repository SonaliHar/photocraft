//! UNRUN translated CURRENT inputs; external reference allocations are harness
//! memory. This proves no GIMP producer or numerical/performance acceptance.
use super::*;
fn translated_reference(
    format: photocraft_color::PixelFormat,
    old: Rect,
    shifted: Rect,
    radius: u32,
    tile: photocraft_geom::TileCoord,
    profile: &Profile,
) -> (Rect, Surface) {
    let source = resident(format, old);
    let mut pixels = source.try_read_region_f64(old).unwrap();
    let space = WorkingSpace::new_with_interrupt(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE).unwrap();
    for pixel in pixels.chunks_exact_mut(4) {
        let rgb = space.convert([pixel[0], pixel[1], pixel[2]], true).unwrap();
        pixel[..3].copy_from_slice(&rgb);
    }
    // Move raw input samples first. Regenerating values at shifted coordinates
    // or shifting a previously filtered output would be a different oracle.
    let mut linear = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    linear.try_write_region_f64(shifted, &pixels).unwrap();
    let t = tile.rect().intersect(&n::box_blur_extent(shifted, radius).unwrap());
    let req = n::box_blur_requirements(&linear, t, shifted, radius, &Interrupt::NONE).unwrap();
    let filtered = n::try_box_blur(
        &linear,
        t,
        shifted,
        radius,
        n::GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units },
        &Interrupt::NONE,
    )
    .unwrap();
    let mut pixels = filtered.try_read_region_f64(t).unwrap();
    for pixel in pixels.chunks_exact_mut(4) {
        let rgb = space.convert([pixel[0], pixel[1], pixel[2]], false).unwrap();
        pixel[..3].copy_from_slice(&rgb);
    }
    let mut output = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    output.try_write_region_f64(t, &pixels).unwrap();
    (t, output)
}
#[test]
#[ignore = "UNRUN translated raw native input/global tile reset reference for three CURRENT roles"]
fn translated_inputs_recompute_canonical_tiles_without_moving_filter_masks() {
    // All four original alpha words are exactly .5; the reference isolates Move
    // coordinate and tile-reset behavior from derived-alpha refusal controls.
    let s = Rect::new(-3, 5, -1, 7);
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        for radius in [0, 4, 127, 1000] {
            let (d, q, g, p) = observed_role(sample, assigned, s, radius);
            let original = owner(d, q, g, assigned);
            let layer = original.document().layers[0].id;
            for (dx, dy) in [(-257, 255), (13, -17), (256, -256)] {
                let moved = original.try_with_readonly_point_layer_translation(layer, dx, dy, &ctl()).unwrap();
                let shifted = Rect::new(s.x0 + dx, s.y0 + dy, s.x1 + dx, s.y1 + dy);
                let context = PreparedBackedBoxContext::new(&moved, NormalBandLimits { max_rows: 2, max_pixels: 16 }, &ctl()).unwrap();
                assert_eq!(context.bounds(), n::box_blur_extent(shifted, radius as u32).unwrap());
                let tile = photocraft_geom::TileCoord::containing(shifted.x0, shifted.y0);
                let (t, expected) = translated_reference(original.document().pixel_format(), s, shifted, radius as u32, tile, &p);
                for roi in [Rect::new(t.x0, t.y0, t.x0 + 1, t.y0 + 2), Rect::new(t.x0, t.y0 + 1, t.x0 + 2, t.y0 + 2), Rect::new(t.x0, t.y0, t.x0 + 1, t.y0 + 1)]
                {
                    compare(&context, roi, &expected);
                }
                let a = original.document().xcf_state.as_ref().unwrap();
                let b = moved.document().xcf_state.as_ref().unwrap();
                assert_eq!(a.filter_masks, b.filter_masks);
                assert_eq!(a.filter_stacks, b.filter_stacks);
                assert_eq!(original.document().source_color_context, moved.document().source_color_context);
                assert_eq!(original.document().icc_profile, moved.document().icc_profile);
                // Native fixture values still come from ORIGINAL world coordinates.
                let LayerContent::BackedRaster(native) = &moved.document().layers[0].content else { panic!("native") };
                let mut actual = [0.; 4];
                native.checked_view().read_pixel_f64_into(shifted.x0, shifted.y0, &mut actual, &ctl()).unwrap();
                let mut expected_native = [0.; 4];
                resident(original.document().pixel_format(), s).try_read_pixel_f64(s.x0, s.y0, &mut expected_native).unwrap();
                assert_eq!(actual.map(f64::to_bits), expected_native.map(f64::to_bits));
            }
        }
    }
}
#[test]
#[ignore = "UNRUN translated hidden/disabled malformed input proof and identity view"]
fn translated_disabled_identity_is_raw_and_hidden_inputs_still_require_full_proof() {
    let s = Rect::new(-3, 5, -1, 7);
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        let (mut d, q, g, _) = observed_role(sample, assigned, s, 1000);
        d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let original = owner(d, q, g, assigned);
        let layer = original.document().layers[0].id;
        let moved = original.try_with_readonly_point_layer_translation(layer, 257, -259, &ctl()).unwrap();
        let shifted = Rect::new(254, -254, 256, -252);
        let context = PreparedBackedBoxContext::new(&moved, Default::default(), &ctl()).unwrap();
        assert_eq!(context.bounds(), shifted);
        let raw = resident(original.document().pixel_format(), s).try_read_region_f64(s).unwrap();
        let mut expected = Surface::new(photocraft_color::PixelFormat::RGBA64F);
        expected.try_write_region_f64(shifted, &raw).unwrap();
        compare(&context, shifted, &expected);
        let (mut d, q, g, _, counts) = fixture(sample, assigned, s, Rect::new(0, 0, 283, 281), 4, false);
        d.layers[0].visible = false;
        let layer = d.layers[0].id;
        let state = d.xcf_state.as_mut().unwrap();
        state.items.iter_mut().find(|i| i.item == ItemRef::Layer(layer)).unwrap().visible = false;
        let node = &mut state.filter_stacks[0].nodes[0];
        node.operation.schema_epoch = BOX_GEOMETRY_EPOCH.into();
        node.composition.as_mut().unwrap().compatibility_epoch = BOX_GEOMETRY_EPOCH.into();
        counts.bad.store(true, Ordering::Relaxed);
        let original = owner(d, q, g, assigned);
        let moved = original.try_with_readonly_point_layer_translation(layer, 257, -259, &ctl()).unwrap();
        assert!(PreparedBackedBoxContext::new(&moved, Default::default(), &ctl()).is_err());
    }
}
