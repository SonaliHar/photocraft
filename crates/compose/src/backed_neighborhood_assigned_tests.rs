//! UNRUN assigned Box CURRENT and preparation regressions. Source producer
//! native/profile evidence is separate; external legacy oracle buffers are test
//! harness allocations, never production catalog receipts.
use super::*;
use crate::gimp_render::PreparationBudget;
use photocraft_doc::WorkingProfileInterpretation;

#[derive(Debug)]
struct Raw {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    q: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for Raw {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.q.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let start = usize::try_from(at).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn embedded(edit: bool) -> Vec<u8> {
    let name = if edit { "assigned-unit-f32.edit.xcf" } else { "assigned-unit-f32.source.xcf" };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full/box-raster-color-evidence").join(name);
    let raw = std::fs::read(path).expect("fetch the pinned Box corpus; no ICC capture is synthesized");
    assert_eq!(raw.len(), 4817);
    let bytes = raw.get(111..751).unwrap().to_vec();
    assert_eq!(bytes.get(36..40), Some(b"acsp".as_slice()));
    bytes
}
fn assigned(radius: i64, hidden: bool, defect: u8) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, Profile, Surface) {
    let (mut doc, q, guard, _, _) = model(4);
    // Genuine additional source/model/ICC fixture reservation before test-owned
    // replacement metadata is installed. This is not an importer allocation claim.
    let fee = q.reserve(QuotaKind::Resident, 2 << 20).unwrap();
    let bytes = embedded(radius == 5);
    assert_eq!(bytes, embedded(false));
    let profile = Profile::parse(&bytes).unwrap();
    let curves = profile.trc.as_ref().unwrap();
    assert_ne!(curves[0], curves[1]);
    assert_ne!(curves[1], curves[2]);
    doc.depth = SampleType::F32;
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    doc.icc_profile = Some(Arc::new(bytes.clone()));
    let source = doc.source_color_context.as_mut().unwrap();
    source.original_icc = Some(Arc::new(bytes));
    source.source_precision = 650;
    let bounds = doc.bounds();
    let format = doc.pixel_format();
    let mut raw = Vec::with_capacity(16 * 12 * 16);
    for y in 0..12 {
        for x in 0..16 {
            for value in [(x + 1) as f32 / 17., (y + 1) as f32 / 13., (x + y + 1) as f32 / 29., if (x + y) % 3 == 0 { 0.25f32 } else { 0.75f32 }] {
                raw.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    // Actual saved Box native out-of-unit words independently decoded in
    // evidence-v2-box-native-check.json (28443513). Their locations/bit words
    // are ingress controls here, not a producer filter-output oracle.
    for (word, bits) in [(209, 0x3f80160bu32), (250, 0xba15c5a6), (290, 0x3f8024b8), (294, 0xbb93e671), (630, 0xb9477e7b)] {
        raw[word * 4..word * 4 + 4].copy_from_slice(&bits.to_le_bytes());
    }
    if defect == 1 {
        let n = raw.len();
        raw[n - 16..n - 12].copy_from_slice(&f32::NAN.to_le_bytes());
    }
    if defect == 2 {
        let n = raw.len();
        raw[n - 16..n - 12].copy_from_slice(&f32::INFINITY.to_le_bytes());
    }
    if defect == 3 {
        let n = raw.len();
        raw[n - 4..n].copy_from_slice(&1.125f32.to_le_bytes());
    }
    let source_surface = Surface::try_from_interleaved(format, bounds, &raw).unwrap();
    let descriptor = NativePlaneDescriptor::new(format, bounds, &[0; 16]).unwrap();
    let payload = q.reserve(QuotaKind::Resident, (raw.capacity() + std::mem::size_of::<Raw>() + 128) as u64).unwrap();
    let store = Arc::new(Raw { descriptor, bytes: raw, q: q.clone(), _fee: payload });
    let backed = BackedRaster::new(store, q.clone(), 1, &ctl()).unwrap();
    let layer = doc.layers[0].id;
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(backed, descriptor, &ctl()).unwrap());
    doc.layers[0].visible = !hidden;
    doc.layers[0].opacity = if hidden { 0.375 } else { 1. };
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_masks[0].surface = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    for item in &mut state.items {
        if item.item == ItemRef::Layer(layer) {
            item.visible = !hidden;
        }
    }
    state.filter_stacks[0].nodes[0].arguments = vec![i("radius", radius)];
    (doc, q, Arc::new((guard, fee)), profile, source_surface)
}
fn adopt_assigned(doc: Document, q: BackingQuota, guard: Arc<dyn std::any::Any + Send + Sync>) -> AccountedDocument {
    AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q, &ctl()).unwrap()
}
struct Budget<'a> {
    q: &'a BackingQuota,
    c: &'a ReadControl<'a>,
}
impl PreparationBudget for Budget<'_> {
    type Lease = QuotaLease;
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.q.reserve(QuotaKind::Resident, n).map_err(|_| CompositionError::Raster(photocraft_raster::RasterInputError::LimitExceeded))
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
        self.c.check(n).map_err(|_| CompositionError::Raster(photocraft_raster::RasterInputError::LimitExceeded))
    }
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn assigned_distinct_trcs_and_premultiplied_box_match_existing_independent_ordered_evaluator() {
    for radius in [4, 5] {
        let (doc, q, guard, profile, source) = assigned(radius, false, 0);
        let owner = adopt_assigned(doc, q, guard);
        let current = PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &ctl()).unwrap();
        current.validate_operative(&ctl()).unwrap();
        let stack = &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0];
        let old = OrderedStack::new_with_interrupt(
            stack,
            &profile,
            SampleEncoding::NonlinearProfile,
            22,
            owner.document().bounds(),
            [true, false, false],
            &Interrupt::NONE,
        )
        .unwrap();
        let expected = old.evaluate(&source, current.bounds(), &mut |_| Ok(()), &Interrupt::NONE).unwrap();
        let output = current.render_wide_band(current.bounds(), &ctl()).unwrap();
        let mut n = 0;
        for y in current.bounds().y0..current.bounds().y1 {
            for x in current.bounds().x0..current.bounds().x1 {
                let mut p = [0.; 4];
                expected.try_read_pixel_f64(x, y, &mut p).unwrap();
                assert_eq!(output.pixels()[n].map(f64::to_bits), p.map(f64::to_bits));
                n += 1;
            }
        }
        assert_eq!(owner.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap().as_slice(), embedded(radius == 5));
    }
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn assigned_role_mismatch_foreign_profile_and_implicit_lane_refuse_without_hash_or_name_authority() {
    for case in 0..5 {
        let (mut doc, q, guard, _, _) = assigned(4, false, 0);
        match case {
            0 => doc.icc_profile = Some(Arc::new(photocraft_cms::Builtin::Srgb.profile().to_bytes())),
            1 => {
                let mut bytes = embedded(false);
                bytes[36] = b'x';
                doc.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(bytes));
            }
            2 => doc.sample_encoding = Some(SampleEncoding::Linear),
            3 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments = vec![i("radius", 6)],
            _ => doc.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "stale".into() },
        }
        let before = q.usage().unwrap();
        assert!(prepare_mode(&doc, None, 512, &Interrupt::NONE, &mut NativeBudget { q: &q, c: &ctl() }, true, true).is_err());
        assert_eq!(q.usage().unwrap(), before);
        drop(guard);
    }
    let (doc, q, guard, _, _) = assigned(4, false, 0);
    assert!(AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn hidden_disabled_still_refuse_nonfinite_and_nonunit_alpha_while_finite_hdr_identity_has_no_halo() {
    for defect in [1, 2, 3] {
        let (mut doc, q, guard, _, _) = assigned(4, true, defect);
        doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
        let owner = adopt_assigned(doc, q, guard);
        assert!(PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &ctl()).is_err());
    }
    let (mut doc, q, guard, _, source) = assigned(4, true, 0);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    let owner = adopt_assigned(doc, q, guard);
    let context = PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &ctl()).unwrap();
    assert_eq!(context.bounds(), owner.document().bounds());
    let out = context.render_wide_band(context.bounds(), &ctl()).unwrap();
    let mut n = 0;
    for y in context.bounds().y0..context.bounds().y1 {
        for x in context.bounds().x0..context.bounds().x1 {
            let mut p = [0.; 4];
            source.try_read_pixel_f64(x, y, &mut p).unwrap();
            assert_eq!(out.pixels()[n].map(f64::to_bits), p.map(f64::to_bits));
            n += 1;
        }
    }
    let (mut doc, q, guard, _, _) = assigned(4, true, 0);
    let state = doc.xcf_state.as_mut().unwrap();
    state.filter_stacks[0].nodes[0].enabled = false;
    let mut mask = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    mask.try_write_pixel_f64(15, 11, &[1.]).unwrap();
    state.filter_masks[0].surface = mask;
    let owner = adopt_assigned(doc, q, guard);
    assert!(PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN: PhotoCraft test compilation/runtime deferred"]
fn original_effective_and_image_fees_overlap_and_source_current_failure_refunds_real_ledger() {
    let (doc, q, guard, _, _) = assigned(4, false, 0);
    let owner = adopt_assigned(doc, q.clone(), guard);
    {
        let warm = PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &ctl()).unwrap();
        warm.validate_operative(&ctl()).unwrap();
    }
    let baseline = q.usage().unwrap();
    {
        let context = PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &ctl()).unwrap();
        let held = q.usage().unwrap();
        assert!(held.resident > baseline.resident);
        let band = context.render_wide_band(context.bounds(), &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > held.resident);
        drop(band);
        assert_eq!(q.usage().unwrap(), held);
    }
    assert_eq!(q.usage().unwrap(), baseline);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &denied), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let cancelled = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &cancelled), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), baseline);
    // Fill the actual aggregate Resident ledger after source caches are warm.
    let blocked = q.reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    assert!(matches!(PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &ctl()), Err(E::Quota(_))));
    drop(blocked);
    assert_eq!(q.usage().unwrap(), baseline);
    let visits = std::sync::atomic::AtomicU64::new(0);
    let stop_late = |units| {
        if units == 4 && visits.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 190 { Err(BackingError::Work) } else { Ok(()) }
    };
    let late = ReadControl { interrupt: &Interrupt::NONE, work: &stop_late };
    assert!(matches!(PreparedBackedNeighborhoodContext::new_assigned_box(&owner, Default::default(), &late), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let c = ctl();
    let mut budget = Budget { q: &q, c: &c };
    {
        let pipeline = LeasedStockNeighborhoodPipeline::new_assigned_box_budgeted(owner.document(), &Interrupt::NONE, &mut budget, ()).unwrap();
        assert!(q.usage().unwrap().resident > baseline.resident);
        drop(pipeline);
    }
    assert_eq!(q.usage().unwrap(), baseline);
    let (mut staged, source_q, source_guard, _, source) = assigned(4, false, 0);
    staged.layers[0].content = LayerContent::Raster(source);
    let before = source_q.usage().unwrap();
    let c = ctl();
    let mut budget = Budget { q: &source_q, c: &c };
    {
        let prepared = LeasedStockNeighborhoodPipeline::new_assigned_box_budgeted(&staged, &Interrupt::NONE, &mut budget, &source_guard).unwrap();
        assert!(source_q.usage().unwrap().resident > before.resident);
        drop(prepared);
    }
    assert_eq!(source_q.usage().unwrap(), before);
    let (mut doc, q, guard, _, _) = assigned(4, false, 0);
    doc.layers[0].visible = false;
    let c = ctl();
    let mut budget = Budget { q: &q, c: &c };
    let before = q.usage().unwrap();
    assert!(LeasedStockNeighborhoodPipeline::new_assigned_box_budgeted(&doc, &Interrupt::NONE, &mut budget, ()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(guard);
}
