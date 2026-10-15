//! UNRUN generated CURRENT/mapper fixtures. No new GIMP source or numerical
//! acceptance is claimed; reference surfaces and clones are external harness.
use super::*;
use photocraft_doc::import_compositing::XCF12_FLAT_LEGACY_SOURCE_EPOCH as EPOCH;
const LEGACY: [i32; 14] = [3, 4, 6, 7, 8, 9, 10, 15, 16, 17, 18, 19, 20, 21];
fn legacy(mode: i32) -> (AccountedDocument, [Arc<Store>; 2]) {
    let (original, stores) = fixture(30, 0);
    let q = original.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = original.document().clone();
    let p = doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap();
    p.compatibility_epoch = EPOCH.into();
    p.source_mode = mode;
    p.legacy = true;
    p.stored_blend_space = -2;
    p.stored_composite_space = -2;
    p.stored_operator = -2;
    p.resolved =
        Some(ResolvedGimpComposition { blend_space: GimpSpace::ProfileRgb, composite_space: GimpSpace::ProfileRgb, operator: GimpOperator::ClipToBackdrop });
    doc.layers[1].import_compositing =
        Some(ImportCompositing { blend_encoding: SampleEncoding::NonlinearProfile, composite_encoding: SampleEncoding::NonlinearProfile, pass_through: false });
    (AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((original, fee)), q, &ctl()).unwrap(), stores)
}
fn resident(owner: &AccountedDocument, stores: &[Arc<Store>; 2]) -> Document {
    let mut doc = owner.document().clone();
    for (layer, store) in doc.layers.iter_mut().zip(stores) {
        layer.content = LayerContent::Raster(Surface::try_from_interleaved(store.descriptor.format(), store.descriptor.bounds(), &store.bytes).unwrap());
    }
    doc
}
struct Budget {
    q: BackingQuota,
    work: u64,
    limit: u64,
}
impl crate::gimp_render::PreparationBudget for Budget {
    type Lease = QuotaLease;
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.q.reserve(QuotaKind::Resident, n).map_err(|_| CompositionError::Invalid("harness quota"))
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
        self.work = self.work.checked_add(n).ok_or(CompositionError::Invalid("harness work"))?;
        if self.work > self.limit {
            return Err(CompositionError::Invalid("harness work limit"));
        }
        Ok(())
    }
}
fn budget(q: &BackingQuota) -> Budget {
    Budget { q: q.clone(), work: 0, limit: u64::MAX }
}

#[test]
#[ignore = "UNRUN: all14 paid legacy modes exactly match existing resident renderer and retain backdrop alpha/native custody; modern inventory stays separate"]
fn legacy_kernel_and_resident_reference() {
    for mode in LEGACY {
        assert!(lower::is_supported_legacy_mode(mode));
        assert!(!lower::is_supported_mode(mode));
        let (owner, stores) = legacy(mode);
        let original_bytes = stores.each_ref().map(|s| s.bytes.clone());
        let reference = resident(&owner, &stores);
        let expected = crate::gimp_render::PreparedRenderer::new(&reference).unwrap().render(reference.bounds(), &mut |_| Ok(())).unwrap();
        let prepared = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
        let output = prepared.render_band(prepared.bounds(), &ctl()).unwrap();
        for (index, (a, b)) in output.pixels().iter().zip(&expected.px).enumerate() {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
            let LayerContent::BackedRaster(base) = &owner.document().layers[0].content else { panic!("fixture") };
            assert_eq!(a[3].to_bits(), native_pixel(base, (index % 16) as i32, (index / 16) as i32)[3].to_bits());
        }
        assert_eq!(stores.each_ref().map(|s| s.bytes.clone()), original_bytes);
        if matches!(mode, 15..=17) {
            let q = owner.quota().clone();
            let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
            let mut doc = owner.document().clone();
            let LayerContent::BackedRaster(backed) = &doc.layers[1].content else { panic!("fixture") };
            let endpoint = if mode == 16 { 1f32 } else { 0f32 };
            let raw: Vec<u8> = [endpoint, endpoint, endpoint, 1f32].into_iter().flat_map(f32::to_ne_bytes).collect();
            doc.layers[1].content = LayerContent::BackedRaster(backed.try_edited_native(Rect::new(3, 1, 4, 2), &raw, &ctl()).unwrap());
            let endpoint_owner = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner.clone(), fee)), q, &ctl()).unwrap();
            assert!(PreparedBackedSeparableDocument::new(&endpoint_owner, Default::default(), &ctl()).is_ok());
        }
    }
    for mode in [0, 1, 5, 22, 61] {
        assert!(!lower::is_supported_legacy_mode(mode));
    }
}

#[test]
#[ignore = "UNRUN: exactold2 legacy source prepares paid Profile spaces without native reads; old modern constructors and altered source envelopes refuse"]
fn source_preparation_is_separate_and_strict() {
    for mode in LEGACY {
        let (owner, stores) = legacy(mode);
        let mut doc = resident(&owner, &stores);
        let q = owner.quota();
        let before = q.usage().unwrap();
        let held = LeasedSeparableBlendPipeline::new_legacy_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).unwrap();
        assert!(q.usage().unwrap().resident > before.resident);
        assert!(stores.iter().all(|s| s.reads.load(Ordering::Relaxed) == 0));
        drop(held);
        assert_eq!(q.usage().unwrap(), before);
        assert!(LeasedSeparableBlendPipeline::new_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
        assert!(LeasedSeparableBlendPipeline::new_general_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
        for defect in 0..4 {
            let mut bad = doc.clone();
            match defect {
                0 => bad.layers[1].visible = false,
                1 => bad.layers[1].opacity = 0.5,
                2 => bad.xcf_state.as_mut().unwrap().provenance.format_version = 23,
                _ => bad.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap().stored_blend_space = -1,
            }
            assert!(LeasedSeparableBlendPipeline::new_legacy_budgeted(&bad, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
            assert_eq!(q.usage().unwrap(), before);
        }
        doc.layers.clear(); // external harness surfaces only
    }
}

#[test]
#[ignore = "UNRUN: dynamic CURRENT hidden/offcanvas legacy native/default unit and exactpolicy gates remain mandatory; modernexplicit transition preserves epoch"]
fn current_unit_and_policy_refusals() {
    for defect in 0..5 {
        let (owner, stores) = legacy(3);
        let q = owner.quota().clone();
        let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
        let mut doc = owner.document().clone();
        doc.layers[1].visible = false;
        doc.layers[1].opacity = 0.;
        doc.xcf_state.as_mut().unwrap().items[1].visible = false;
        match defect {
            0 => {
                let LayerContent::BackedRaster(backed) = &doc.layers[1].content else { panic!("fixture") };
                let raw: Vec<u8> = [1.25f32, 0.25, 0.5, 0.].into_iter().flat_map(f32::to_ne_bytes).collect();
                doc.layers[1].content = LayerContent::BackedRaster(backed.try_edited_native(Rect::new(11, -1, 12, 0), &raw, &ctl()).unwrap());
            }
            1 => doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap().legacy = false,
            2 => doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap().compatibility_epoch = "unknown-legacy".into(),
            3 => {
                let p = doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap();
                p.stored_blend_space = 2;
                p.stored_composite_space = 2;
                p.stored_operator = 2;
            }
            _ => {
                let charge = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
                let defaults: Vec<u8> = [1.25f32, 0.25, 0.5, 0.].into_iter().flat_map(f32::to_ne_bytes).collect();
                let descriptor = NativePlaneDescriptor::new(stores[1].descriptor.format(), stores[1].descriptor.bounds(), &defaults).unwrap();
                let store = Arc::new(Store { descriptor, bytes: stores[1].bytes.clone(), q: q.clone(), reads: AtomicU64::new(0), _fee: charge });
                let raster = BackedRaster::new(store, q.clone(), 2, &ctl()).unwrap();
                doc.layers[1].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
            }
        }
        let invalid = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner, fee)), q, &ctl()).unwrap();
        assert!(PreparedBackedSeparableDocument::new(&invalid, Default::default(), &ctl()).is_err());
    }
    let (owner, _) = legacy(3);
    let id = owner.document().layers[1].id;
    let moved = owner.try_with_flat_composite_layers_translation(&[id], -257, 259, &ctl()).unwrap();
    assert!(PreparedBackedSeparableDocument::new(&moved, Default::default(), &ctl()).is_ok());
    let next = owner
        .try_with_flat_composite_composition_patch(
            photocraft_doc::accounted_document::FlatCompositeCompositionPatch {
                layer: id,
                source_mode: 31,
                blend_space: GimpSpace::LinearRgb,
                composite_space: GimpSpace::LinearRgb,
                operator: GimpOperator::Union,
            },
            &ctl(),
        )
        .unwrap();
    assert_eq!(next.document().xcf_state.as_ref().unwrap().items[1].composition.as_ref().unwrap().compatibility_epoch, EPOCH);
    assert!(PreparedBackedSeparableDocument::new(&next, Default::default(), &ctl()).is_ok());
}

#[test]
#[ignore = "UNRUN: retained source preparation and warm CURRENT real fees refund under aggregate quota/cumulative work/caller cancellation"]
fn actual_fees_and_controls() {
    let (owner, stores) = legacy(21);
    let doc = resident(&owner, &stores);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let mut b = budget(q);
    drop(LeasedSeparableBlendPipeline::new_legacy_budgeted(&doc, &Interrupt::NONE, &mut b, owner.clone()).unwrap());
    assert_eq!(q.usage().unwrap(), before);
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident).unwrap();
    assert!(LeasedSeparableBlendPipeline::new_legacy_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let mut limited = budget(q);
    limited.limit = b.work - 1;
    assert!(LeasedSeparableBlendPipeline::new_legacy_budgeted(&doc, &Interrupt::NONE, &mut limited, owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(LeasedSeparableBlendPipeline::new_legacy_budgeted(&doc, &interrupt, &mut budget(q), owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap());
    let warm = q.usage().unwrap();
    let prepared = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
    assert!(q.usage().unwrap().resident > warm.resident);
    let band = prepared.render_band(Rect::new(0, 0, 8, 6), &ctl()).unwrap();
    drop(band);
    drop(prepared);
    assert_eq!(q.usage().unwrap(), warm);
}
