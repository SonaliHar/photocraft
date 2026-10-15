//! UNRUN assigned Median controls. Existing lower/OrderedStack references are
//! independent CURRENT kernel oracles, not producer projection acceptance.
//! Harness clones/parsed profiles are external to production quota claims.
use super::*;
use photocraft_algo::gimp::neighborhood as n;
fn median(edit: bool, hidden: bool, defect: u8) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Profile, Surface) {
    let (mut doc, q, guard, _, source) = assigned(4, hidden, defect);
    let filename = if edit { "assigned-unit-f32.edit.xcf" } else { "assigned-unit-f32.source.xcf" };
    let raw = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full/median-raster-color-evidence").join(filename))
        .expect("fetch pinned Median source/edit corpus");
    assert_eq!(raw.len(), 4998);
    let bytes = raw.get(111..751).unwrap().to_vec();
    assert_eq!(bytes, embedded(false)); // independently extracted common640byte role
    let profile = Profile::parse(&bytes).unwrap();
    doc.icc_profile = Some(Arc::new(bytes.clone()));
    doc.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(bytes));
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation.name = "gegl:median-blur".into();
    node.operation.schema_epoch = "xcf22-rgb-alpha-median-v1".into();
    node.composition.as_mut().unwrap().compatibility_epoch = "xcf22-rgb-alpha-median-v1".into();
    node.arguments = vec![
        i("radius", if edit { 4 } else { 3 }),
        e("neighborhood", "gegl:median-blur::neighborhood", 1),
        f("percentile", 50.),
        f("alpha-percentile", 50.),
        e("abyss-policy", "gegl:median-blur::abyss-policy", 1),
        arg("high-precision", FilterValue::Bool(edit)),
    ];
    (doc, q, guard, profile, source)
}
fn params(edit: bool) -> n::MedianParams {
    n::MedianParams {
        radius: if edit { 4 } else { 3 },
        neighborhood: n::MedianNeighborhood::Circle,
        percentile: 50.,
        alpha_percentile: 50.,
        high_precision: edit,
    }
}
fn lower(source: &Surface, bounds: Rect, edit: bool) -> Surface {
    // Existing OrderedStack prefix0 decodes native input into RGBA64 before
    // Median, retaining wide derived output. Do not insert an F32 write here.
    let mut wide = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    let decoded = source.try_read_region_f64(bounds).unwrap();
    wide.try_write_region_f64(bounds, &decoded).unwrap();
    let source = &wide;
    let req = n::median_requirements(source, bounds, bounds, params(edit), &Interrupt::NONE).unwrap();
    let limits = n::GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units };
    if edit {
        n::try_median_blur(source, bounds, bounds, params(edit), limits, &Interrupt::NONE).unwrap()
    } else {
        n::try_median_blur_quantized_encoded(source, bounds, bounds, params(edit), limits, &Interrupt::NONE).unwrap()
    }
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn assigned_pair_matches_encoded_quantized_and_highprecision_without_pixel_cms() {
    for edit in [false, true] {
        let (doc, q, guard, profile, source) = median(edit, false, 0);
        let owner = adopt_assigned(doc, q, guard);
        let current = PreparedBackedNeighborhoodContext::new_assigned_median(&owner, Default::default(), &ctl()).unwrap();
        current.validate_operative(&ctl()).unwrap();
        assert_eq!(current.bounds(), owner.document().bounds());
        let expected = lower(&source, current.bounds(), edit);
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
        // Generic assigned route selects the exact same recipe, with no Box fallback.
        let dispatched = PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).unwrap();
        assert_eq!(dispatched.bounds(), current.bounds());
        assert!(PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &ctl()).is_err());
    }
    // Pure lower contract: ingress remains finite HDR, while only the existing
    // quantized derived kernel clamps/rounds; high precision keeps rank values.
    let bounds = Rect::new(0, 0, 16, 12);
    let mut raw = Vec::new();
    for _ in 0..192 {
        for v in [-0.25f32, 1.25, 0.5, 1.] {
            raw.extend_from_slice(&v.to_le_bytes());
        }
    }
    let source = Surface::try_from_interleaved(photocraft_color::PixelFormat::RGBA32F, bounds, &raw).unwrap();
    let high = lower(&source, bounds, true);
    let low = lower(&source, bounds, false);
    let mut h = [0.; 4];
    let mut l = [0.; 4];
    high.try_read_pixel_f64(8, 6, &mut h).unwrap();
    low.try_read_pixel_f64(8, 6, &mut l).unwrap();
    assert_eq!(h, [-0.25, 1.25, 0.5, 1.]);
    assert_eq!(l[0], 0.);
    assert_eq!(l[1], 1.);
    assert_ne!(h[2].to_bits(), l[2].to_bits());
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn dedicated_median_exact_arguments_profile_and_dispatch_fail_authoritatively() {
    for case in 0..8 {
        let (mut doc, q, guard, _, _) = median(false, false, 0);
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        match case {
            0 => node.arguments[0] = f("radius", 3.),
            1 => node.arguments[5] = arg("high-precision", FilterValue::Bool(true)),
            2 => node.arguments[1] = e("neighborhood", "foreign-domain", 1),
            3 => node.arguments[3] = node.arguments[2].clone(),
            4 => node.composition.as_mut().unwrap().compatibility_epoch = "unqualified-current".into(),
            5 => node.operation.name = "gegl:gaussian-blur".into(),
            6 => doc.icc_profile = Some(Arc::new(photocraft_cms::Builtin::Srgb.profile().to_bytes())),
            _ => node.composition.as_mut().unwrap().stored_operator = -261651887,
        }
        let owner = adopt_assigned(doc, q.clone(), guard);
        let before = q.usage().unwrap();
        assert!(PreparedBackedNeighborhoodContext::new_assigned_median(&owner, Default::default(), &ctl()).is_err());
        assert!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let (doc, q, guard, _, _) = assigned(4, false, 0);
    let box_owner = adopt_assigned(doc, q, guard);
    assert!(PreparedBackedNeighborhoodContext::new_assigned_median(&box_owner, Default::default(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn median_hidden_disabled_native_default_and_mask_checks_precede_encoded_identity() {
    for defect in [1, 2, 3] {
        let (mut doc, q, guard, _, _) = median(true, true, defect);
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let owner = adopt_assigned(doc, q, guard);
        assert!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).is_err());
    }
    let (mut doc, q, guard, _, source) = median(true, true, 0);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    let owner = adopt_assigned(doc, q, guard);
    let current = PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).unwrap();
    let band = current.render_wide_band(current.bounds(), &ctl()).unwrap();
    let mut index = 0;
    for y in current.bounds().y0..current.bounds().y1 {
        for x in current.bounds().x0..current.bounds().x1 {
            let mut p = [0.; 4];
            source.try_read_pixel_f64(x, y, &mut p).unwrap();
            assert_eq!(band.pixels()[index].map(f64::to_bits), p.map(f64::to_bits));
            index += 1;
        }
    }
    // A late allocated physical mask word remains semantically active for
    // qualification even while layer/filter visibility cannot contribute output.
    let (mut doc, q, guard, _, _) = median(false, true, 0);
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_stacks[0].nodes[0].enabled = false;
    state.filter_masks[0].surface.try_write_pixel_f64(15, 11, &[1.]).unwrap();
    let owner = adopt_assigned(doc, q, guard);
    assert!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).is_err());
    // Default bytes are canonical storage data, not a free display normalization.
    let (mut doc, q, guard, _, source) = median(true, true, 0);
    let mut pixel = [0; 16];
    pixel[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    let descriptor = NativePlaneDescriptor::new(doc.pixel_format(), doc.bounds(), &pixel).unwrap();
    let raw = source.try_read_interleaved(doc.bounds()).unwrap();
    let payload = q.reserve(QuotaKind::Resident, (raw.capacity() + std::mem::size_of::<Raw>() + 128) as u64).unwrap();
    let store = Arc::new(Raw { descriptor, bytes: raw, q: q.clone(), _fee: payload });
    let raster = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    let owner = adopt_assigned(doc, q, guard);
    assert!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn retained_assigned_profiles_overlap_median_scratch_and_output_on_actual_ledger() {
    for edit in [false, true] {
        let (doc, q, guard, _, _) = median(edit, false, 0);
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
            let band = context.render_wide_band(context.bounds(), &ctl()).unwrap();
            assert!(q.usage().unwrap().resident > held.resident);
            drop(band);
            assert_eq!(q.usage().unwrap(), held);
        }
        assert_eq!(q.usage().unwrap(), before);
        let blocked = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident).unwrap();
        assert!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &ctl()).is_err());
        drop(blocked);
        assert_eq!(q.usage().unwrap(), before);
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &denied), Err(E::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), before);
        let visits = std::sync::atomic::AtomicU64::new(0);
        let late_work = |units| {
            if units == 4 && visits.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 190 { Err(BackingError::Work) } else { Ok(()) }
        };
        let late = ReadControl { interrupt: &Interrupt::NONE, work: &late_work };
        assert!(matches!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &late), Err(E::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), before);
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        let cancelled = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(matches!(PreparedBackedNeighborhoodContext::new_assigned(&owner, Default::default(), &cancelled), Err(E::Backing(BackingError::Cancelled))));
        assert_eq!(q.usage().unwrap(), before);
    }
    let (mut doc, q, guard, _, source) = median(false, false, 0);
    doc.layers[0].content = LayerContent::Raster(source);
    let before = q.usage().unwrap();
    let c = ctl();
    let mut budget = Budget { q: &q, c: &c };
    {
        let prepared = LeasedStockNeighborhoodPipeline::new_assigned_median_budgeted(&doc, &Interrupt::NONE, &mut budget, &guard).unwrap();
        assert!(q.usage().unwrap().resident > before.resident);
        drop(prepared);
    }
    assert_eq!(q.usage().unwrap(), before);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    assert!(LeasedStockNeighborhoodPipeline::new_assigned_median_budgeted(&doc, &Interrupt::NONE, &mut budget, &guard).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
