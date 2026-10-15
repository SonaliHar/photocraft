//! UNRUN synthetic native/current/source-preparation controls. Producer source
//! qualification/numerical acceptance remain separately witnessed and deferred.
//! Resident/reference buffers here are external harness allocations.
use super::*;
const CODES: [i32; 4] = [24, 25, 26, 27];
fn fixture_lab(code: i32) -> (AccountedDocument, [Arc<Store>; 2]) {
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
    let blend = GimpSpace::Lab;
    let composite = GimpSpace::LinearRgb;
    doc.layers[1].import_compositing = None;
    let p = doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap();
    p.source_mode = code;
    p.legacy = false;
    p.compatibility_epoch = "xcf-12-authored-blend-auto".into();
    p.stored_blend_space = -3;
    p.stored_composite_space = -1;
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
#[ignore = "UNRUN: paid physicalLab four-mode output matches unchanged resident pipeline, canonicalLab/physicalNone and raw native storage remain intact"]
fn physical_lab_matches_old_resident() {
    for code in CODES {
        assert!(lower::is_supported_lab_mode(code));
        assert!(!lower::is_supported_mode(code));
        let (owner, stores) = fixture_lab(code);
        let doc = resident(&owner, &stores);
        let original = stores.each_ref().map(|s| s.bytes.clone());
        let expected = crate::gimp_render::PreparedRenderer::new(&doc).unwrap().render(doc.bounds(), &mut |_| Ok(())).unwrap();
        let p = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
        let out = p.render_band(p.bounds(), &ctl()).unwrap();
        for (a, b) in out.pixels().iter().zip(&expected.px) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
        assert_eq!(stores.each_ref().map(|s| s.bytes.clone()), original);
        assert!(owner.document().layers[1].import_compositing.is_none());
    }
}

#[test]
#[ignore = "UNRUN: exactLab source prepares full paid physicalCMS bundle without native evaluation; wrong physical/profile/geometry or other source constructors refuse"]
fn exact_source_and_actual_bundle_custody() {
    for code in CODES {
        let (owner, stores) = fixture_lab(code);
        let doc = resident(&owner, &stores);
        let q = owner.quota();
        let before = q.usage().unwrap();
        let mut b = budget(q);
        let held = LeasedSeparableBlendPipeline::new_lab_budgeted(&doc, &Interrupt::NONE, &mut b, owner.clone()).unwrap();
        let requirements = photocraft_cms::owned_lab::requirements_with_control(
            Builtin::Srgb.profile(),
            photocraft_cms::xcf_color::Encoding::NonlinearProfile,
            photocraft_cms::control::Limits::default(),
            &|| false,
        )
        .unwrap();
        assert!(q.usage().unwrap().resident - before.resident >= requirements.peak_resident_bytes as u64);
        assert!(stores.iter().all(|s| s.reads.load(Ordering::Relaxed) == 0));
        drop(held);
        assert_eq!(q.usage().unwrap(), before);
        assert!(LeasedSeparableBlendPipeline::new_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
        assert!(LeasedSeparableBlendPipeline::new_general_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
        for defect in 0..3 {
            let mut bad = doc.clone();
            match defect {
                0 => {
                    bad.layers[1].import_compositing =
                        Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false })
                }
                1 => bad.layers[1].opacity = 0.5,
                _ => bad.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            };
            assert!(LeasedSeparableBlendPipeline::new_lab_budgeted(&bad, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
            assert_eq!(q.usage().unwrap(), before);
        }
    }
}

#[test]
#[ignore = "UNRUN: full hidden/default/offcanvas nativeunit and exactLab physicalNone policy required; modernRGB transition installs ordinary transfer without changing epoch"]
fn current_input_policy_and_transition() {
    for defect in 0..4 {
        let (owner, stores) = fixture_lab(24);
        let q = owner.quota().clone();
        let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
        let mut doc = owner.document().clone();
        doc.layers[1].visible = false;
        doc.layers[1].opacity = 0.;
        doc.xcf_state.as_mut().unwrap().items[1].visible = false;
        let raw: Vec<u8> = [1.25f32, 0.2, 0.3, 0.].into_iter().flat_map(f32::to_ne_bytes).collect();
        match defect {
            0 => {
                let LayerContent::BackedRaster(b) = &doc.layers[1].content else { panic!("fixture") };
                doc.layers[1].content = LayerContent::BackedRaster(b.try_edited_native(Rect::new(11, -1, 12, 0), &raw, &ctl()).unwrap());
            }
            1 => {
                let charge = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
                let descriptor = NativePlaneDescriptor::new(stores[1].descriptor.format(), stores[1].descriptor.bounds(), &raw).unwrap();
                let store = Arc::new(Store { descriptor, bytes: stores[1].bytes.clone(), q: q.clone(), reads: AtomicU64::new(0), _fee: charge });
                let raster = BackedRaster::new(store, q.clone(), 2, &ctl()).unwrap();
                doc.layers[1].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
            }
            2 => {
                doc.layers[1].import_compositing =
                    Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false })
            }
            _ => doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap().compatibility_epoch = "unqualified-Lab".into(),
        }
        let bad = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner, fee)), q, &ctl()).unwrap();
        assert!(PreparedBackedSeparableDocument::new(&bad, Default::default(), &ctl()).is_err());
    }
    let (owner, _) = fixture_lab(26);
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
    assert!(next.document().layers[1].import_compositing.is_some());
    assert_eq!(next.document().xcf_state.as_ref().unwrap().items[1].composition.as_ref().unwrap().compatibility_epoch, "xcf-12-authored-blend-auto");
}

#[test]
#[ignore = "UNRUN: full Lab peak/native two-band fees and prospective actualpipeline bandwork use caller ledger/control; preparation/cancel/quota failure refunds"]
fn aggregate_fees_and_pipeline_work_are_real() {
    let (owner, stores) = fixture_lab(27);
    let doc = resident(&owner, &stores);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let mut b = budget(q);
    drop(LeasedSeparableBlendPipeline::new_lab_budgeted(&doc, &Interrupt::NONE, &mut b, owner.clone()).unwrap());
    assert_eq!(q.usage().unwrap(), before);
    let mut limited = budget(q);
    limited.limit = b.work - 1;
    assert!(LeasedSeparableBlendPipeline::new_lab_budgeted(&doc, &Interrupt::NONE, &mut limited, owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident).unwrap();
    assert!(LeasedSeparableBlendPipeline::new_lab_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
    drop(blocker);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(LeasedSeparableBlendPipeline::new_lab_budgeted(&doc, &interrupt, &mut budget(q), owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap());
    let warm = q.usage().unwrap();
    let p = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
    let prepared = q.usage().unwrap();
    assert!(prepared.resident > warm.resident);
    let out = p.render_band(Rect::new(0, 0, 8, 6), &ctl()).unwrap();
    assert!(q.usage().unwrap().resident > prepared.resident);
    drop(out);
    assert_eq!(q.usage().unwrap(), prepared);
    // These CMS parts are external harness facts, not production catalog claims.
    let facts = photocraft_cms::owned_lab::construct_with_control(
        Builtin::Srgb.profile(),
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        photocraft_cms::control::Limits::default(),
        &|| false,
    )
    .unwrap();
    let expected = facts.per_pixel_work_units + 160;
    assert!(expected > 512);
    let stop_work = |n| if n == expected { Err(BackingError::Work) } else { Ok(()) };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &stop_work };
    assert!(matches!(p.render_band(Rect::new(0, 0, 1, 1), &control), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), prepared);
    drop(p);
    assert_eq!(q.usage().unwrap(), warm);
}
