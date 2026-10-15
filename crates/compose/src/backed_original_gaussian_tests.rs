//! UNRUN generated CURRENT original-Gaussian context. Real source fixtures and
//! GIMP output comparisons belong to the independently pinned IO/host workflows.
use super::*;
#[derive(Debug)]
struct Bytes {
    descriptor: NativePlaneDescriptor,
    raw: Vec<u8>,
    q: BackingQuota,
    _fee: QuotaLease,
}
impl NativePlaneStore for Bytes {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if q.shares_ledger(&self.q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, at: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        let start = usize::try_from(at).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(out.len()).ok_or(BackingError::Bounds)?;
        out.copy_from_slice(self.raw.get(start..end).ok_or(BackingError::Bounds)?);
        Ok(())
    }
}
fn original(x: f32, enabled: bool, defect: u8) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, &'static Profile, Surface) {
    let (mut doc, q, guard, profile, _) = model(0);
    let fee = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let bounds = Rect::new(0, 0, 96, 80);
    doc.size = Size::new(96, 80);
    let mut raw = Vec::with_capacity(96 * 80 * 4);
    for y in 0..80 {
        for x in 0..96 {
            raw.extend_from_slice(&[(x * 2) as u8, (y * 3) as u8, ((x + y) % 256) as u8, 255]);
        }
    }
    if defect == 1 {
        *raw.last_mut().unwrap() = 254;
    }
    // Independent old resident oracle buffer is external harness storage.
    let source = Surface::try_from_interleaved(doc.pixel_format(), bounds, &raw).unwrap();
    let descriptor = NativePlaneDescriptor::new(doc.pixel_format(), bounds, &[0; 4]).unwrap();
    let payload = q.reserve(QuotaKind::Resident, (raw.capacity() + std::mem::size_of::<Bytes>() + 128) as u64).unwrap();
    let bytes = Arc::new(Bytes { descriptor, raw, q: q.clone(), _fee: payload });
    let raster = BackedRaster::new(bytes, q.clone(), 1, &ctl()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
    let state = doc.xcf_state.as_mut().unwrap();
    for item in &mut state.items {
        item.raster_bounds = Some(bounds);
        if let Some(mask) = &mut item.mask {
            mask.bounds = bounds;
        }
    }
    state.filter_masks[0].bounds = bounds;
    let node = &mut state.filter_stacks[0].nodes[0];
    node.operation.name = "gegl:gaussian-blur".into();
    node.operation.schema_epoch = "xcf-22-stock-gaussian-replace".into();
    node.enabled = enabled;
    node.arguments = vec![
        f("std-dev-x", x),
        f("std-dev-y", 1.5),
        e("filter", "gegl:gaussian-blur::filter", 0),
        e("abyss-policy", "gegl:gaussian-blur::abyss-policy", 1),
        arg("clip-extent", FilterValue::Bool(true)),
    ];
    let composition = node.composition.as_mut().unwrap();
    composition.compatibility_epoch = "xcf-22-stock-gaussian-replace".into();
    composition.stored_blend_space = -856260015;
    composition.stored_composite_space = -856260015;
    composition.stored_operator = -856260015;
    if defect == 2 {
        composition.stored_operator = -1474592175;
    }
    if defect == 3 {
        node.operation.schema_epoch = "xcf22-rgb-alpha-finite-gaussian-v1".into();
    }
    (doc, q, Arc::new((guard, fee)), profile, source)
}
#[test]
#[ignore = "UNRUN: enabled original Gaussian2.25/3.75 uses exact existing OrderedStack recurrence/working-space/opaque alpha without native U8 intermediate"]
fn original_matches_old_ordered_stack_on_varying_opaque_input() {
    for x in [2.25, 3.75] {
        let (doc, q, guard, profile, source) = original(x, true, 0);
        let owner = adopt(doc, q, guard);
        let context = PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).unwrap();
        let old = OrderedStack::new_with_interrupt(
            &owner.document().xcf_state.as_ref().unwrap().filter_stacks[0],
            profile,
            SampleEncoding::NonlinearProfile,
            22,
            owner.document().bounds(),
            [true, false, false],
            &Interrupt::NONE,
        )
        .unwrap();
        let expected = old.evaluate(&source, context.bounds(), &mut |_| Ok(()), &Interrupt::NONE).unwrap();
        let band = context.render_wide_band(context.bounds(), &ctl()).unwrap();
        let mut n = 0;
        for y in 0..80 {
            for x in 0..96 {
                let mut p = [0.; 4];
                expected.try_read_pixel_f64(x, y, &mut p).unwrap();
                assert_eq!(band.pixels()[n].map(f64::to_bits), p.map(f64::to_bits));
                assert_eq!(band.pixels()[n][3], 1.0);
                n += 1;
            }
        }
    }
}
#[test]
#[ignore = "UNRUN: CURRENT disabled original validates typed descriptor/native before identity; opaque/raw/schema/parameter negatives refuse"]
fn disabled_identity_and_exact_original_refusals() {
    let (doc, q, guard, profile, source) = original(2.25, false, 0);
    let owner = adopt(doc, q, guard);
    let context = PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).unwrap();
    let band = context.render_wide_band(context.bounds(), &ctl()).unwrap();
    let mut n = 0;
    for y in 0..80 {
        for x in 0..96 {
            let mut p = [0.; 4];
            source.try_read_pixel_f64(x, y, &mut p).unwrap();
            assert_eq!(band.pixels()[n].map(f64::to_bits), p.map(f64::to_bits));
            n += 1;
        }
    }
    for (enabled, defect, x) in [(false, 1, 2.25), (true, 2, 2.25), (false, 3, 2.25), (false, 0, 2.5)] {
        let (doc, q, guard, profile, _) = original(x, enabled, defect);
        let owner = adopt(doc, q, guard);
        assert!(PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN: actual output/profile/maps retain quota; caller work and cancel refuse without owner changes"]
fn original_retention_and_cumulative_control() {
    let (doc, q, guard, profile, _) = original(3.75, true, 0);
    let owner = adopt(doc, q.clone(), guard);
    drop(PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).unwrap());
    let before = q.usage().unwrap();
    {
        let context = PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ctl()).unwrap();
        let held = q.usage().unwrap();
        assert!(held.resident > before.resident);
        let band = context.render_wide_band(context.bounds(), &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > held.resident);
        drop(band);
        assert_eq!(q.usage().unwrap(), held);
    }
    assert_eq!(q.usage().unwrap(), before);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|n| if n > 1 << 20 { Err(BackingError::Work) } else { Ok(()) } };
    assert!(matches!(PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &denied), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), before);
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(matches!(
        PreparedBackedNeighborhoodContext::new(&owner, profile, Default::default(), &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }),
        Err(E::Backing(BackingError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), before);
}
