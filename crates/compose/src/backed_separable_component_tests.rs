//! UNRUN synthetic native/current/source-preparation controls. Producer source
//! qualification/numerical acceptance remain separately witnessed and deferred.
//! Resident/reference buffers here are external harness allocations.
use super::*;
const CODES: [i32; 11] = [11, 12, 13, 14, 37, 38, 39, 40, 54, 55, 56];
fn fixture_component(code: i32) -> (AccountedDocument, [Arc<Store>; 2]) {
    let (old, original) = fixture(30, 0);
    let q = old.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = old.document().clone();
    let mut stores = Vec::new();
    for (index, prior) in original.iter().enumerate() {
        let charge = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
        let color = if index == 0 { [0.2f32, 0.5, 0.35, 0.7] } else { [0.65f32, 0.25, 0.4, 0.625] };
        let pixel: Vec<u8> = color.into_iter().flat_map(f32::to_ne_bytes).collect();
        let bytes = pixel.repeat(prior.bytes.len() / pixel.len());
        let descriptor = prior.descriptor;
        let store = Arc::new(Store { descriptor, bytes, q: q.clone(), reads: AtomicU64::new(0), _fee: charge });
        let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
        doc.layers[index].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
        stores.push(store);
    }
    let legacy = code < 20;
    let blend = if code >= 54 { GimpSpace::LinearRgb } else { GimpSpace::ProfileRgb };
    let composite = if legacy { GimpSpace::ProfileRgb } else { GimpSpace::LinearRgb };
    doc.layers[1].import_compositing = Some(ImportCompositing { blend_encoding: enc(blend), composite_encoding: enc(composite), pass_through: false });
    let p = doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap();
    p.source_mode = code;
    p.legacy = legacy;
    p.compatibility_epoch = "xcf-12-authored-blend-auto".into();
    p.stored_blend_space = if code >= 54 { -1 } else { -2 };
    p.stored_composite_space = if legacy { -2 } else { -1 };
    p.stored_operator = -2;
    p.resolved = Some(ResolvedGimpComposition { blend_space: blend, composite_space: composite, operator: GimpOperator::ClipToBackdrop });
    let stores: [Arc<Store>; 2] = stores.try_into().unwrap();
    (AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((old, fee)), q, &ctl()).unwrap(), stores)
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
#[ignore = "UNRUN: chromatic native-backed legacyHSV/modernHSV/luma match existing resident pipeline and preserve distinct coverage/space policy"]
fn exact_existing_resident_math_and_inventories() {
    for code in CODES {
        assert!(!lower::is_supported_mode(code));
        assert_eq!(lower::is_supported_legacy_mode(code), code < 20);
        assert_eq!(lower::is_supported_component_mode(code), code >= 37);
        let (owner, stores) = fixture_component(code);
        let doc = resident(&owner, &stores);
        let expected = crate::gimp_render::PreparedRenderer::new(&doc).unwrap().render(doc.bounds(), &mut |_| Ok(())).unwrap();
        let prepared = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
        let actual = prepared.render_band(prepared.bounds(), &ctl()).unwrap();
        for (a, b) in actual.pixels().iter().zip(&expected.px) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
        assert!(owner.document().xcf_state.as_ref().unwrap().items[1].composition.as_ref().unwrap().legacy == (code < 20));
    }
}

#[test]
#[ignore = "UNRUN: dedicated exactold2 source preparation retains real fees/source without native reads and refuses wrong policy/constructor/geometry"]
fn source_constructors_are_separate() {
    for code in CODES {
        let (owner, stores) = fixture_component(code);
        let doc = resident(&owner, &stores);
        let q = owner.quota();
        let before = q.usage().unwrap();
        let prepared = if code < 20 {
            LeasedSeparableBlendPipeline::new_legacy_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone())
        } else {
            LeasedSeparableBlendPipeline::new_component_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone())
        }
        .unwrap();
        assert!(q.usage().unwrap().resident > before.resident);
        assert!(stores.iter().all(|s| s.reads.load(Ordering::Relaxed) == 0));
        drop(prepared);
        assert_eq!(q.usage().unwrap(), before);
        assert!(LeasedSeparableBlendPipeline::new_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
        assert!(LeasedSeparableBlendPipeline::new_general_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
        let wrong = if code < 20 {
            LeasedSeparableBlendPipeline::new_component_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone())
        } else {
            LeasedSeparableBlendPipeline::new_legacy_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone())
        };
        assert!(wrong.is_err());
        let mut bad = doc.clone();
        bad.layers[1].opacity = 0.5;
        let bad = if code < 20 {
            LeasedSeparableBlendPipeline::new_legacy_budgeted(&bad, &Interrupt::NONE, &mut budget(q), owner.clone())
        } else {
            LeasedSeparableBlendPipeline::new_component_budgeted(&bad, &Interrupt::NONE, &mut budget(q), owner.clone())
        };
        assert!(bad.is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
}

#[test]
#[ignore = "UNRUN: hidden/offcanvas component native/default unit and exact policy required; luma ties/zeroY remain errors and finiteHDR output is not clamped"]
fn full_current_input_and_luma_domains() {
    for code in CODES {
        for default_defect in [false, true] {
            let (owner, stores) = fixture_component(code);
            let q = owner.quota().clone();
            let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
            let mut doc = owner.document().clone();
            doc.layers[1].visible = false;
            doc.layers[1].opacity = 0.;
            doc.xcf_state.as_mut().unwrap().items[1].visible = false;
            let raw: Vec<u8> = [1.25f32, 0.2, 0.3, 0.].into_iter().flat_map(f32::to_ne_bytes).collect();
            if default_defect {
                let charge = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
                let descriptor = NativePlaneDescriptor::new(stores[1].descriptor.format(), stores[1].descriptor.bounds(), &raw).unwrap();
                let store = Arc::new(Store { descriptor, bytes: stores[1].bytes.clone(), q: q.clone(), reads: AtomicU64::new(0), _fee: charge });
                let raster = BackedRaster::new(store, q.clone(), 2, &ctl()).unwrap();
                doc.layers[1].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
            } else {
                let LayerContent::BackedRaster(backed) = &doc.layers[1].content else { panic!("fixture") };
                doc.layers[1].content = LayerContent::BackedRaster(backed.try_edited_native(Rect::new(11, -1, 12, 0), &raw, &ctl()).unwrap());
            }
            let bad = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner, fee)), q, &ctl()).unwrap();
            assert!(PreparedBackedSeparableDocument::new(&bad, Default::default(), &ctl()).is_err());
        }
    }
    let profile = Builtin::Srgb.profile();
    let ws = WorkingSpace::new(profile, SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
    let w = profile.matrix.as_ref().unwrap()[1];
    let b = [w[1], 0., 0., 1.];
    let s = [0., w[0], 0., 1.];
    for code in [54, 55] {
        assert!(ws.separable(&ws, b, s, 1., GimpOperator::ClipToBackdrop, gimp_blend::mode(code).unwrap()).is_err());
    }
    assert!(ws.separable(&ws, [0., 0., 0., 1.], [0.5, 0.5, 0.5, 1.], 1., GimpOperator::ClipToBackdrop, gimp_blend::mode(56).unwrap()).is_err());
    let hdr = ws.separable(&ws, [1., 0., 0., 1.], [0., 1., 0., 1.], 1., GimpOperator::ClipToBackdrop, gimp_blend::mode(56).unwrap()).unwrap();
    assert!(hdr[0] > 1.);
    assert_eq!(hdr[1], 0.);
    assert!(ws.separable(&ws, hdr, [0.5; 4], 1., GimpOperator::ClipToBackdrop, gimp_blend::mode(54).unwrap()).is_err());
}

#[test]
#[ignore = "UNRUN: source work/quota/cancel and warm CURRENT Spaces/output fees use same ledger and refund; modern20 transition retains old epoch"]
fn fees_controls_and_modern_transition() {
    let (owner, stores) = fixture_component(56);
    let doc = resident(&owner, &stores);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let mut b = budget(q);
    drop(LeasedSeparableBlendPipeline::new_component_budgeted(&doc, &Interrupt::NONE, &mut b, owner.clone()).unwrap());
    assert_eq!(q.usage().unwrap(), before);
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident).unwrap();
    assert!(LeasedSeparableBlendPipeline::new_component_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let mut limited = budget(q);
    limited.limit = b.work - 1;
    assert!(LeasedSeparableBlendPipeline::new_component_budgeted(&doc, &Interrupt::NONE, &mut limited, owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(LeasedSeparableBlendPipeline::new_component_budgeted(&doc, &interrupt, &mut budget(q), owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap());
    let warm = q.usage().unwrap();
    let p = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
    assert!(q.usage().unwrap().resident > warm.resident);
    drop(p.render_band(Rect::new(0, 0, 8, 6), &ctl()).unwrap());
    drop(p);
    assert_eq!(q.usage().unwrap(), warm);
    let next = owner
        .try_with_flat_composite_composition_patch(
            photocraft_doc::accounted_document::FlatCompositeCompositionPatch {
                layer: owner.document().layers[1].id,
                source_mode: 31,
                blend_space: GimpSpace::LinearRgb,
                composite_space: GimpSpace::LinearRgb,
                operator: GimpOperator::Union,
            },
            &ctl(),
        )
        .unwrap();
    assert!(PreparedBackedSeparableDocument::new(&next, Default::default(), &ctl()).is_ok());
    assert_eq!(next.document().xcf_state.as_ref().unwrap().items[1].composition.as_ref().unwrap().compatibility_epoch, "xcf-12-authored-blend-auto");
}
