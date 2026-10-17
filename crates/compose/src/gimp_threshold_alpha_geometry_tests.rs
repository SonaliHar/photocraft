//! UNREGISTERED / UNRUN future resident Threshold Alpha geometry drafts.
//! Intended as a child of backed_threshold_alpha_context_tests, reusing its
//! complete CURRENT singleton fixture. These synthetic changes are neither
//! independent GIMP evidence nor source/default enrollment. Root owns activation.
use super::*;

struct ResidentFixture {
    doc: Document,
    raw: Vec<u8>,
    _original: Arc<dyn Any + Send + Sync>,
    _fee: QuotaLease,
}
fn resident(size: Size, mode: ColorMode, sample: SampleType, stored: SampleEncoding, origin: [i32; 2]) -> ResidentFixture {
    let (mut doc, q, original, _, _, _) = fixture(mode, sample, stored);
    // Bound these synthetic fixtures before allocating raw input/native tiles.
    assert!(size.width <= 257 && size.height <= 257);
    let fee = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    // This resident generic-renderer fixture uses the current effective encoding,
    // unlike the separate exact-Builtin backed Threshold Alpha class fixture.
    let effective = photocraft_cms::xcf_color::effective_profile(
        None,
        if mode == ColorMode::Rgb { photocraft_cms::ColorSpace::Rgb } else { photocraft_cms::ColorSpace::Gray },
        crate::gimp_composite::encoding(stored),
    )
    .unwrap();
    doc.icc_profile = Some(effective.to_bytes());
    doc.working_profile_interpretation = if mode == ColorMode::Rgb {
        photocraft_doc::WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap())
    } else {
        photocraft_doc::WorkingProfileInterpretation::implicit_sgray(doc.icc_profile.as_ref().unwrap())
    };
    doc.size = size;
    let extent = Rect::from_xywh(origin[0], origin[1], size.width, size.height);
    let format = doc.pixel_format();
    let mut raw = vec![0u8; size.width as usize * size.height as usize * format.bytes_per_pixel()];
    for (i, pixel) in raw.chunks_exact_mut(format.bytes_per_pixel()).enumerate() {
        for channel in 0..mode.color_channels() {
            let color = if (i + channel) % 3 == 0 { -0.0 } else { 0.5 + 2f64.powi(-45) };
            photocraft_color::try_write_sample_f64(pixel, sample, channel, color).unwrap();
        }
        let alpha = [0.0, 0.25, 0.5, 0.5 + 2f64.powi(-45), 0.75, 1.0][i % 6];
        photocraft_color::try_write_sample_f64(pixel, sample, mode.color_channels(), alpha).unwrap();
    }
    let mut surface = Surface::new(format);
    surface.try_write_interleaved(extent, &raw).unwrap();
    doc.layers[0].content = LayerContent::Raster(surface);
    let state = doc.xcf_state.as_mut().unwrap();
    for item in &mut state.items {
        item.raster_bounds = Some(extent);
        if let Some(mask) = &mut item.mask {
            mask.bounds = extent;
            mask.offset = origin;
        }
    }
    state.filter_masks[0].bounds = extent;
    ResidentFixture { doc, raw, _original: original, _fee: fee }
}
fn render(doc: &Document) -> crate::gimp_render::WideBuffer {
    crate::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap()
}

#[test]
fn wide_and_tall_threshold_matches_old_singleton_pixels_and_keeps_raw_source_words() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            for stored in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile] {
                let old = resident(Size::new(8, 6), mode, sample, stored, [0, 0]);
                let reference = render(&old.doc);
                for size in [Size::new(257, 3), Size::new(3, 257)] {
                    let current = resident(size, mode, sample, stored, [0, 0]);
                    let before = current.doc.clone();
                    let output = render(&current.doc);
                    assert_eq!(output.px.len(), size.width as usize * size.height as usize);
                    for (i, pixel) in output.px.iter().enumerate() {
                        assert_eq!(pixel.map(f64::to_bits), reference.px[i % 48].map(f64::to_bits));
                        let expected_alpha = if i % 6 >= 4 || sample == SampleType::U16 && matches!(i % 6, 2 | 3) { 1.0 } else { 0.0 };
                        assert_eq!(pixel[3], expected_alpha, "native quantization then strict Float alpha boundary");
                    }
                    assert_eq!(current.doc.layers[0].surface().unwrap().try_read_interleaved(current.doc.bounds()).unwrap(), current.raw);
                    assert_eq!(current.doc, before, "rendering does not rewrite native colors, alpha, masks or original metadata");
                }
            }
        }
    }
}

#[test]
fn tile_boundary_subregions_equal_full_render_without_enrolling_raw_padding() {
    for size in [Size::new(257, 3), Size::new(3, 257)] {
        let current = resident(size, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile, [0, 0]);
        let full = render(&current.doc);
        let prepared = crate::gimp_render::PreparedRenderer::new(&current.doc).unwrap();
        let rects = if size.width == 257 { [Rect::new(0, 0, 256, 3), Rect::new(256, 0, 257, 3)] } else { [Rect::new(0, 0, 3, 256), Rect::new(0, 256, 3, 257)] };
        for rect in rects {
            let part = prepared.render(rect, &mut |_| Ok(())).unwrap();
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    let global = y as usize * size.width as usize + x as usize;
                    let local = (y - rect.y0) as usize * rect.width() as usize + (x - rect.x0) as usize;
                    assert_eq!(part.px[local].map(f64::to_bits), full.px[global].map(f64::to_bits));
                }
            }
        }
    }
}

#[test]
fn shifted_nonunit_and_profile_mismatch_remain_refused_after_geometry_extension() {
    for size in [Size::new(257, 3), Size::new(3, 257)] {
        for origin in [[1, 0], [0, -1]] {
            let current = resident(size, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile, origin);
            assert!(crate::gimp_render::PreparedRenderer::new(&current.doc).is_err());
        }
        for bad in [f64::NAN, f64::INFINITY, -0.001, 1.001] {
            let mut current = resident(size, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile, [0, 0]);
            for alpha in [false, true] {
                let LayerContent::Raster(surface) = &mut current.doc.layers[0].content else { panic!() };
                let bad_pixel = if alpha { [0.5, 0.5, 0.5, bad] } else { [bad, 0.5, 0.5, 0.75] };
                surface.try_write_pixel_f64(0, 0, &bad_pixel).unwrap();
                assert!(crate::gimp_render::PreparedRenderer::new(&current.doc).is_err());
            }
        }
        let mut current = resident(size, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile, [0, 0]);
        current.doc.icc_profile = Some(Builtin::AdobeRgbCompat.profile().to_bytes());
        assert!(crate::gimp_render::PreparedRenderer::new(&current.doc).is_err());
    }
}
fn exposure(fixture: &mut ResidentFixture) {
    let node = &mut fixture.doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.name = "gegl:exposure".into();
    node.operation.stored_version = "1:0".into();
    node.operation.schema_epoch = crate::gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH.into();
    node.arguments = ["black-level", "exposure"]
        .map(|name| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(0f32.to_bits())) })
        .to_vec();
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = crate::gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH.into();
    policy.stored_blend_space = 706183761;
    policy.stored_composite_space = 706183761;
    policy.stored_operator = 706183761;
}
#[test]
fn another_precision_operation_keeps_old_geometry_admission() {
    let mut old = resident(Size::new(8, 6), ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile, [0, 0]);
    exposure(&mut old);
    assert!(crate::gimp_render::PreparedRenderer::new(&old.doc).is_ok(), "positive old Exposure control");
    for size in [Size::new(257, 3), Size::new(3, 257)] {
        let mut current = resident(size, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile, [0, 0]);
        exposure(&mut current);
        assert!(crate::gimp_render::PreparedRenderer::new(&current.doc).is_err(), "Threshold-only geometry must not widen Exposure");
    }
}

fn retain_zero_mask_tile(current: &mut ResidentFixture, x: i32, y: i32) {
    let mask = &mut current.doc.xcf_state.as_mut().unwrap().filter_masks[0];
    let zero = vec![0u8; mask.surface.format().bytes_per_pixel()];
    // The checked raw writer retains the initialized full tile, even when the
    // logical pixel equals the default. Do not prune away this regression.
    mask.surface.try_write_interleaved(Rect::from_xywh(x, y, 1, 1), &zero).unwrap();
}
#[test]
fn old_f64_zero_tile_mask_remains_admitted_but_invisible_mask_does_not() {
    let mut old = resident(Size::new(8, 6), ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile, [0, 0]);
    let sparse = render(&old.doc);
    retain_zero_mask_tile(&mut old, 0, 0);
    let state = old.doc.xcf_state.as_ref().unwrap();
    assert_eq!(state.filter_masks[0].surface.tile_count(), 1);
    assert_eq!(state.filter_masks[0].surface.resident_payload_bytes().unwrap(), 8 * 65_536 + 8);
    assert_eq!(render(&old.doc).px, sparse.px, "old F64 full-zero physical tile exceeds the former Float-only byte cap");
    let state = old.doc.xcf_state.as_mut().unwrap();
    let mask = state.filter_masks[0].id;
    state.items.iter_mut().find(|i| i.item == ItemRef::Mask(mask)).unwrap().visible = false;
    assert!(crate::gimp_render::PreparedRenderer::new(&old.doc).is_err(), "extracting the Threshold predicate must retain visibility refusal");
    assert_eq!(old.doc.layers[0].surface().unwrap().try_read_interleaved(old.doc.bounds()).unwrap(), old.raw);
}

#[test]
fn wide_and_tall_two_zero_tiles_are_admitted_but_outside_grid_tiles_are_refused() {
    for size in [Size::new(257, 3), Size::new(3, 257)] {
        let mut current = resident(size, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile, [0, 0]);
        let sparse = render(&current.doc);
        retain_zero_mask_tile(&mut current, 0, 0);
        let second = if size.width == 257 { (256, 0) } else { (0, 256) };
        retain_zero_mask_tile(&mut current, second.0, second.1);
        let mask = &current.doc.xcf_state.as_ref().unwrap().filter_masks[0];
        assert_eq!(mask.surface.tile_count(), 2);
        assert_eq!(mask.surface.resident_payload_bytes().unwrap(), 2 * 8 * 65_536 + 8);
        assert_eq!(render(&current.doc).px, sparse.px);
        // All added raw samples are still zero, so only the genuine physical
        // coordinate guard can reject the out-of-canvas retained tile.
        let outside = if size.width == 257 { (512, 0) } else { (0, 512) };
        retain_zero_mask_tile(&mut current, outside.0, outside.1);
        assert!(crate::gimp_render::PreparedRenderer::new(&current.doc).is_err());
        let mask = &mut current.doc.xcf_state.as_mut().unwrap().filter_masks[0];
        drop(mask.surface.try_take_tiles(Rect::from_xywh(outside.0, outside.1, 1, 1)).unwrap());
        assert_eq!(mask.surface.tile_count(), 2, "isolate the negative-coordinate refusal");
        retain_zero_mask_tile(&mut current, -256, 0);
        assert!(crate::gimp_render::PreparedRenderer::new(&current.doc).is_err());
        assert_eq!(current.doc.layers[0].surface().unwrap().try_read_interleaved(current.doc.bounds()).unwrap(), current.raw);
    }
}
