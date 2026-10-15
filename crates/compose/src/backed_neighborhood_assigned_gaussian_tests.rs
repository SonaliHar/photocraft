//! UNRUN assigned Gaussian CURRENT references. Public native captures are
//! producer input evidence only; no producer projection/profile-role equivalence
//! is inferred. Existing lower/OrderedStack numerical references are external
//! harness allocations, not production accounting receipts.
use super::*;
use crate::gimp_composite::WorkingSpace;
use photocraft_algo::gimp::neighborhood as n;
fn gaussian(edit: bool, hdr: bool, hidden: bool, defect: u8) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Profile, Surface) {
    let (mut doc, q, guard, _, _) = assigned(4, hidden, 0);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full/gaussian-raster-color-evidence");
    let prefix = if hdr { "assigned-hdr-f32" } else { "assigned-unit-f32" };
    let phase = if edit { "edit" } else { "source" };
    let xcf = std::fs::read(root.join(format!("{prefix}.{phase}.xcf"))).expect("fetch pinned assigned Gaussian corpus");
    let bytes = xcf.get(111..751).unwrap().to_vec();
    assert_eq!(bytes, embedded(false));
    let profile = Profile::parse(&bytes).unwrap();
    doc.icc_profile = Some(Arc::new(bytes.clone()));
    doc.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(bytes));
    let mut raw = std::fs::read(root.join(format!("{prefix}.{phase}.reopened.layer.buffer-extent.native.none.bin"))).unwrap();
    assert_eq!(raw.len(), 3072);
    assert_eq!(raw, std::fs::read(root.join(format!("{prefix}.source.reopened.layer.buffer-extent.native.none.bin"))).unwrap());
    // Actual captured words are finiteRGB/unitAlpha, not filename-unit. All3072
    // bytes have independently recorded SHA3102cfa5(namedunit) or77cacaa1(HDR).
    assert!(raw.chunks_exact(4).all(|word| f32::from_le_bytes(word.try_into().unwrap()).is_finite()));
    assert!(raw.chunks_exact(16).all(|pixel| (0.0..=1.0).contains(&f32::from_le_bytes(pixel[12..16].try_into().unwrap()))));
    assert!(raw.chunks_exact(16).any(|pixel| pixel[..12].chunks_exact(4).any(|word| !(0.0..=1.0).contains(&f32::from_le_bytes(word.try_into().unwrap())))));
    let len = raw.len();
    match defect {
        1 => raw[len - 16..len - 12].copy_from_slice(&f32::NAN.to_le_bytes()),
        2 => raw[len - 16..len - 12].copy_from_slice(&f32::INFINITY.to_le_bytes()),
        3 => raw[len - 4..].copy_from_slice(&1.125f32.to_le_bytes()),
        _ => {}
    }
    let format = doc.pixel_format();
    let bounds = doc.bounds();
    let source = Surface::try_from_interleaved(format, bounds, &raw).unwrap();
    let descriptor = NativePlaneDescriptor::new(format, bounds, &[0; 16]).unwrap();
    let fee = q.reserve(QuotaKind::Resident, (raw.capacity() + std::mem::size_of::<Raw>() + 128) as u64).unwrap();
    let store = Arc::new(Raw { descriptor, bytes: raw, q: q.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.name = "gegl:gaussian-blur".into();
    node.operation.schema_epoch = "xcf22-rgb-alpha-finite-gaussian-v1".into();
    node.composition.as_mut().unwrap().compatibility_epoch = "xcf22-rgb-alpha-finite-gaussian-v1".into();
    node.arguments = vec![
        f("std-dev-x", if edit { 3.75 } else { 2.25 }),
        f("std-dev-y", if edit { 0.8 } else { 1.3 }),
        e("filter", "gegl:gaussian-blur::filter", if edit { 1 } else { 0 }),
        e("abyss-policy", "gegl:gaussian-blur::abyss-policy", 1),
        arg("clip-extent", FilterValue::Bool(true)),
    ];
    (doc, q, guard, profile, source)
}
fn reference(source: &Surface, bounds: Rect, profile: &Profile, edit: bool) -> Surface {
    // Independent existing public transfer + finite kernel schedule. All maps
    // staywide; an F32 native intermediate would change the established result.
    let space = WorkingSpace::new_with_interrupt(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE).unwrap();
    let mut linear = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            let mut p = [0.; 4];
            source.try_read_pixel_f64(x, y, &mut p).unwrap();
            let rgb = space.convert([p[0], p[1], p[2]], true).unwrap();
            linear.try_write_pixel_f64(x, y, &[rgb[0], rgb[1], rgb[2], p[3]]).unwrap();
        }
    }
    let params = n::GaussianParams {
        std_dev_x: if edit { 3.75 } else { 2.25 },
        std_dev_y: f64::from(if edit { 0.8f32 } else { 1.3f32 }),
        abyss: n::GaussianAbyss::Clamp,
        clip_extent: true,
    };
    let filter = if edit { n::GaussianFilter::Fir } else { n::GaussianFilter::Auto };
    let req = n::gaussian_finite_requirements(&linear, bounds, bounds, params, filter, &Interrupt::NONE).unwrap();
    let filtered = n::try_gaussian_finite_clamp(
        &linear,
        bounds,
        bounds,
        params,
        filter,
        n::GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units },
        &Interrupt::NONE,
    )
    .unwrap();
    let mut out = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            let mut p = [0.; 4];
            filtered.try_read_pixel_f64(x, y, &mut p).unwrap();
            let rgb = space.convert([p[0], p[1], p[2]], false).unwrap();
            out.try_write_pixel_f64(x, y, &[rgb[0], rgb[1], rgb[2], p[3]]).unwrap();
        }
    }
    out
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn four_actual_native_inputs_match_existing_per_channel_whole_axis_auto_fir_schedule() {
    for hdr in [false, true] {
        for edit in [false, true] {
            let (doc, q, guard, profile, source) = gaussian(edit, hdr, false, 0);
            let owner = adopt_assigned(doc, q, guard);
            let current = PreparedBackedNeighborhoodContext::new_assigned_gaussian(&owner, Default::default(), &ctl()).unwrap();
            current.validate_operative(&ctl()).unwrap();
            assert_eq!(current.bounds(), owner.document().bounds());
            let expected = reference(&source, current.bounds(), &profile, edit);
            let state = owner.document().xcf_state.as_ref().unwrap();
            let old = OrderedStack::new_with_interrupt(
                &state.filter_stacks[0],
                &profile,
                SampleEncoding::NonlinearProfile,
                22,
                current.bounds(),
                [true, false, false],
                &Interrupt::NONE,
            )
            .unwrap();
            let legacy = old.evaluate(&source, current.bounds(), &mut |_| Ok(()), &Interrupt::NONE).unwrap();
            let band = current.render_wide_band(current.bounds(), &ctl()).unwrap();
            let mut index = 0;
            for y in current.bounds().y0..current.bounds().y1 {
                for x in current.bounds().x0..current.bounds().x1 {
                    let mut a = [0.; 4];
                    let mut b = [0.; 4];
                    expected.try_read_pixel_f64(x, y, &mut a).unwrap();
                    legacy.try_read_pixel_f64(x, y, &mut b).unwrap();
                    assert_eq!(band.pixels()[index].map(f64::to_bits), a.map(f64::to_bits));
                    assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
                    index += 1;
                }
            }
            assert!(PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &ctl()).is_err());
            assert!(PreparedBackedNeighborhoodContext::new_assigned_median(&owner, Default::default(), &ctl()).is_err());
        }
    }
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn gaussian_exact_five_field_algorithm_pair_and_profile_rule_refuse_without_retry() {
    for case in 0..8 {
        let (mut doc, q, guard, _, _) = gaussian(false, false, false, 0);
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        match case {
            0 => node.arguments[0] = arg("std-dev-x", FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float64Bits(2.25f64.to_bits()))),
            1 => node.arguments[1] = f("std-dev-y", 0.8),
            2 => node.arguments[2] = e("filter", "gegl:gaussian-blur::filter", 1),
            3 => node.arguments[3] = e("abyss-policy", "foreign", 1),
            4 => node.arguments[4] = arg("clip-extent", FilterValue::Bool(false)),
            5 => node.composition.as_mut().unwrap().compatibility_epoch = "group-small-auto".into(),
            6 => node.arguments[1] = node.arguments[0].clone(),
            _ => doc.icc_profile = Some(Arc::new(photocraft_cms::Builtin::Srgb.profile().to_bytes())),
        }
        let owner = adopt_assigned(doc, q.clone(), guard);
        let before = q.usage().unwrap();
        assert!(PreparedBackedNeighborhoodContext::new_assigned_gaussian(&owner, Default::default(), &ctl()).is_err());
        assert!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let (doc, q, guard, _, _) = assigned(4, false, 0);
    let owner = adopt_assigned(doc, q, guard);
    assert!(PreparedBackedNeighborhoodContext::new_assigned_gaussian(&owner, Default::default(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn disabled_hidden_gaussian_retains_complete_input_profile_mask_default_checks() {
    for defect in [1, 2, 3] {
        let (mut doc, q, guard, _, _) = gaussian(true, true, true, defect);
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let owner = adopt_assigned(doc, q, guard);
        assert!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).is_err());
    }
    let (mut doc, q, guard, _, source) = gaussian(true, true, true, 0);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    let owner = adopt_assigned(doc, q, guard);
    let context = PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).unwrap();
    let band = context.render_wide_band(context.bounds(), &ctl()).unwrap();
    let mut index = 0;
    for y in context.bounds().y0..context.bounds().y1 {
        for x in context.bounds().x0..context.bounds().x1 {
            let mut p = [0.; 4];
            source.try_read_pixel_f64(x, y, &mut p).unwrap();
            assert_eq!(band.pixels()[index].map(f64::to_bits), p.map(f64::to_bits));
            index += 1;
        }
    }
    let (mut doc, q, guard, _, _) = gaussian(false, true, true, 0);
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_stacks[0].nodes[0].enabled = false;
    state.filter_masks[0].surface.try_write_pixel_f64(15, 11, &[1.]).unwrap();
    let owner = adopt_assigned(doc, q, guard);
    assert!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).is_err());
    let (mut doc, q, guard, _, source) = gaussian(false, true, true, 0);
    let mut default = [0; 16];
    default[12..16].copy_from_slice(&1.25f32.to_le_bytes());
    let descriptor = NativePlaneDescriptor::new(doc.pixel_format(), doc.bounds(), &default).unwrap();
    let raw = source.try_read_interleaved(doc.bounds()).unwrap();
    let fee = q.reserve(QuotaKind::Resident, (raw.capacity() + std::mem::size_of::<Raw>() + 128) as u64).unwrap();
    let store = Arc::new(Raw { descriptor, bytes: raw, q: q.clone(), _fee: fee });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    let owner = adopt_assigned(doc, q, guard);
    assert!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn profile_working_space_kernel_maps_and_output_hold_real_cumulative_fees() {
    for edit in [false, true] {
        let (doc, q, guard, _, _) = gaussian(edit, true, false, 0);
        let owner = adopt_assigned(doc, q.clone(), guard);
        {
            let warm = PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).unwrap();
            warm.validate_operative(&ctl()).unwrap();
        }
        let before = q.usage().unwrap();
        {
            let context = PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).unwrap();
            let held = q.usage().unwrap();
            assert!(held.resident > before.resident);
            let out = context.render_wide_band(context.bounds(), &ctl()).unwrap();
            assert!(q.usage().unwrap().resident > held.resident);
            drop(out);
            assert_eq!(q.usage().unwrap(), held);
        }
        assert_eq!(q.usage().unwrap(), before);
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &denied), Err(E::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), before);
        let visits = std::sync::atomic::AtomicU64::new(0);
        let fail_late = |units| {
            if units == 4 && visits.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 190 { Err(BackingError::Work) } else { Ok(()) }
        };
        let late = ReadControl { interrupt: &Interrupt::NONE, work: &fail_late };
        assert!(matches!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &late), Err(E::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), before);
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        let cancelled = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(matches!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &cancelled), Err(E::Backing(BackingError::Cancelled))));
        assert_eq!(q.usage().unwrap(), before);
        let blocked = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident).unwrap();
        assert!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).is_err());
        drop(blocked);
        assert_eq!(q.usage().unwrap(), before);
    }
    for hdr in [false, true] {
        for edit in [false, true] {
            let (mut doc, q, guard, _, source) = gaussian(edit, hdr, false, 0);
            doc.layers[0].content = LayerContent::Raster(source);
            let c = ctl();
            let mut budget = Budget { q: &q, c: &c };
            let before = q.usage().unwrap();
            {
                let pipeline = LeasedStockNeighborhoodPipeline::new_assigned_gaussian_budgeted(&doc, &Interrupt::NONE, &mut budget, &guard).unwrap();
                assert!(q.usage().unwrap().resident > before.resident);
                drop(pipeline);
            }
            assert_eq!(q.usage().unwrap(), before);
            doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
            assert!(LeasedStockNeighborhoodPipeline::new_assigned_gaussian_budgeted(&doc, &Interrupt::NONE, &mut budget, &guard).is_err());
            assert_eq!(q.usage().unwrap(), before);
        }
    }
}
