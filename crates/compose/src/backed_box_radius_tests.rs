//! UNRUN full-radius consumers. External reference Surfaces are harness memory;
//! no numerical producer acceptance, deadline or large-image throughput claim.
#[path = "backed_box_assigned_f64_tests.rs"]
mod assigned_f64;
#[path = "backed_box_activity_tests.rs"]
mod layer_activity;
#[path = "backed_box_move_tests.rs"]
mod translated_input;
use super::*;
fn canonical_reference(sample: SampleType, s: Rect, r: u32, tile: photocraft_geom::TileCoord, p: &Profile) -> (Rect, Surface) {
    let source = resident(photocraft_color::PixelFormat::new(ColorMode::Rgb, sample, true), s);
    let o = n::box_blur_extent(s, r).unwrap();
    let t = tile.rect().intersect(&o);
    let space = WorkingSpace::new_with_interrupt(p, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE).unwrap();
    let mut samples = source.try_read_region_f64(s).unwrap();
    for pixel in samples.chunks_exact_mut(4) {
        let rgb = space.convert([pixel[0], pixel[1], pixel[2]], true).unwrap();
        pixel[..3].copy_from_slice(&rgb);
    }
    let mut linear = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    linear.try_write_region_f64(s, &samples).unwrap();
    // T is exactly the global tile intersect full O, so this old lower call
    // has the same reset points as full-O evaluation before an arbitrary crop.
    let req = n::box_blur_requirements(&linear, t, s, r, &Interrupt::NONE).unwrap();
    let filtered =
        n::try_box_blur(&linear, t, s, r, n::GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units }, &Interrupt::NONE).unwrap();
    let mut samples = filtered.try_read_region_f64(t).unwrap();
    for pixel in samples.chunks_exact_mut(4) {
        let rgb = space.convert([pixel[0], pixel[1], pixel[2]], false).unwrap();
        pixel[..3].copy_from_slice(&rgb);
    }
    let mut out = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    out.try_write_region_f64(t, &samples).unwrap();
    (t, out)
}
#[test]
#[ignore = "UNRUN radius1/127/1000 exact canonical old-loop reference"]
fn positive_row_radii_preserve_full_tile_original_source_clamp_and_profile_order() {
    let s = Rect::new(-3, 5, 2, 8);
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        for radius in [1, 127, 1000] {
            let (d, q, g, p) = observed_role(sample, assigned, s, radius);
            let owned = owner(d, q, g, assigned);
            let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits { max_rows: 2, max_pixels: 17 }, &ctl()).unwrap();
            let coordinate = photocraft_geom::TileCoord::containing(-1, 6);
            let (t, expected) = canonical_reference(sample, s, radius as u32, coordinate, &p);
            let x = t.x1 - 2;
            let y = t.y0 + 1;
            for roi in [Rect::new(x, y, x + 2, y + 1), Rect::new(t.x0, t.y0, t.x0 + 1, t.y0 + 2), Rect::new(x, y, x + 1, y + 1)] {
                compare(&context, roi, &expected);
            }
            assert!(context.render_wide_band(Rect::new(t.x0, t.y0, t.x0 + 18, t.y0 + 1), &ctl()).is_err());
        }
    }
}
#[test]
#[ignore = "UNRUN active radius0 CMS identity versus disabled native identity"]
fn active_zero_keeps_cms_roundtrip_and_disabled_identity_has_no_halo() {
    let s = Rect::new(-3, 5, 2, 8);
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        let (d, q, g, p) = observed_role(sample, assigned, s, 0);
        let owned = owner(d, q, g, assigned);
        let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits::default(), &ctl()).unwrap();
        assert_eq!(context.bounds(), s);
        for coordinate in s.tiles() {
            let (t, expected) = canonical_reference(sample, s, 0, coordinate, &p);
            compare(&context, t, &expected);
        }
        let (mut d, q, g, _) = observed_role(sample, assigned, s, 1000);
        d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let owned = owner(d, q, g, assigned);
        let context = PreparedBackedBoxContext::new(&owned, NormalBandLimits::default(), &ctl()).unwrap();
        assert_eq!(context.bounds(), s);
        let expected = resident(owned.document().pixel_format(), s);
        compare(&context, s, &expected);
    }
}
#[test]
#[ignore = "UNRUN observed source radius/activity and old epoch separation"]
fn observed_source_range_accepts_disabled_without_expanding_old_epochs() {
    let s = Rect::new(-3, 5, 2, 8);
    for radius in [0, 1, 127, 1000] {
        let (d, q, g, _) = observed(s, radius);
        let mut input = resident_source(d, &q, g, s);
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        drop(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap());
        input.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        drop(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap());
    }
    for radius in [-1, 1001] {
        let (mut d, q, g, _) = observed(s, radius);
        d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let owned = owner(d, q, g, false);
        assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
    }
    let (d, q, g, _, _) = fixture(SampleType::F32, false, s, Rect::new(0, 0, 283, 281), 127, true);
    let owned = owner(d, q, g, false);
    assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN row callback typed provider/work/quota/cancel cleanup"]
fn row_failures_keep_original_errors_and_publish_no_partial_band() {
    let s = Rect::new(-7, 11, 250, 270);
    let (mut d, q, g, _, counts) = fixture(SampleType::F32, false, s, Rect::new(0, 0, 283, 281), 127, true);
    let node = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.schema_epoch = BOX_GEOMETRY_EPOCH.into();
    node.composition.as_mut().unwrap().compatibility_epoch = BOX_GEOMETRY_EPOCH.into();
    let owned = owner(d, q.clone(), g, false);
    let context = PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).unwrap();
    let roi = Rect::new(-2, 12, 2, 14);
    drop(context.render_wide_band(roi, &ctl()).unwrap()); // Warm actual cache before refund comparisons.
    let baseline = q.usage().unwrap();
    let plan = n::BoxBlurRowPlan::new(s, photocraft_geom::TileCoord::containing(roi.x0, roi.y0), 127, &Interrupt::NONE).unwrap();
    let req = plan.requirements(&Interrupt::NONE).unwrap();
    let conversion = req.row_requests * req.row_pixels as u64 * (4 + 3 * 128);
    let refuse = |units| if units == conversion { Err(BackingError::Work) } else { Ok(()) };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &refuse };
    assert!(matches!(context.render_wide_band(roi, &control), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let armed = AtomicBool::new(false);
    let refuse_inverse = |units| {
        if units == req.work_units {
            armed.store(true, Ordering::Relaxed);
        }
        if armed.load(Ordering::Relaxed) && units == 388 { Err(BackingError::Work) } else { Ok(()) }
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &refuse_inverse };
    assert!(matches!(context.render_wide_band(roi, &control), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let blocked = q.reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    assert!(context.render_wide_band(roi, &ctl()).is_err());
    drop(blocked);
    assert_eq!(q.usage().unwrap(), baseline);
    let armed = AtomicBool::new(false);
    let polls = AtomicU64::new(0);
    let cancel = || armed.load(Ordering::Relaxed) && polls.fetch_add(1, Ordering::Relaxed) >= 32;
    let interrupt = Interrupt::cancel_only(&cancel);
    let charge = |units| {
        if units == req.work_units {
            armed.store(true, Ordering::Relaxed);
        }
        Ok(())
    };
    let control = ReadControl { interrupt: &interrupt, work: &charge };
    assert!(matches!(context.render_wide_band(roi, &control), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), baseline);
    counts.fail.store(true, Ordering::Relaxed);
    assert!(matches!(context.render_wide_band(roi, &ctl()), Err(E::View(photocraft_raster::checked_view::RasterViewError::Backed(BackingError::Io)))));
    counts.fail.store(false, Ordering::Relaxed);
}
#[test]
#[ignore = "UNRUN disabled full native/default/mask/profile validation"]
fn disabled_radius_never_waives_complete_native_mask_or_profile_proof() {
    let s = Rect::new(13, 9, 270, 268);
    for defect in 0..3 {
        let (mut d, q, g, _, counts) = fixture(SampleType::F32, false, s, Rect::new(0, 0, 283, 281), 1000, false);
        let node = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        node.operation.schema_epoch = BOX_GEOMETRY_EPOCH.into();
        node.composition.as_mut().unwrap().compatibility_epoch = BOX_GEOMETRY_EPOCH.into();
        match defect {
            0 => counts.bad.store(true, Ordering::Relaxed),
            1 => d.xcf_state.as_mut().unwrap().filter_masks[0]
                .surface
                .try_write_interleaved(Rect::new(511, 511, 512, 512), &f32::from_bits(1).to_ne_bytes())
                .unwrap(),
            _ => d.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc,
        }
        let owned = owner(d, q, g, false);
        assert!(PreparedBackedBoxContext::new(&owned, Default::default(), &ctl()).is_err());
    }
}
