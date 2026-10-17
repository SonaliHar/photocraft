//! UNRUN mapper-shaped preparation and new-epoch CURRENT regressions. These
//! generated models and resident buffers are external harness data, not GIMP
//! evidence, source enrollment or profile/projection acceptance.
use super::*;
use photocraft_doc::import_compositing::XCF_FLAT_MODERN_IMPLICIT_SOURCE_EPOCH as EPOCH;

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
fn general() -> (AccountedDocument, Document, [Arc<Store>; 2]) {
    let (owner, stores) = fixture(30, 0);
    let mut doc = owner.document().clone();
    doc.size = Size::new(259, 257);
    let state = doc.xcf_state.as_mut().unwrap();
    state.provenance.format_version = 23;
    for item in &mut state.items {
        item.composition.as_mut().unwrap().compatibility_epoch = EPOCH.into();
    }
    for (layer, store) in doc.layers.iter_mut().zip(&stores) {
        layer.content = LayerContent::Raster(Surface::try_from_interleaved(store.descriptor.format(), store.descriptor.bounds(), &store.bytes).unwrap());
    }
    (owner, doc, stores)
}
fn budget(q: &BackingQuota) -> Budget {
    Budget { q: q.clone(), work: 0, limit: u64::MAX }
}

#[test]
#[ignore = "UNRUN: arbitrary mapped activity/dynamic Raster recipe preparation is paid, does not pin/evaluate native, and old50 source remains strict"]
fn generic_source_preparation_retains_source_and_fees() {
    let (owner, mut doc, stores) = general();
    doc.layers[0].visible = false;
    doc.layers[0].opacity = 0.375;
    doc.xcf_state.as_mut().unwrap().items[0].visible = false;
    doc.layers[1].opacity = 0.;
    // Three actual flat rows, reordered canonical metadata and negative bounds.
    let bounds = Rect::new(-3, 257, 7, 266);
    let mut third = Layer::new("third", LayerContent::Raster(Surface::try_from_interleaved(stores[1].descriptor.format(), bounds, &stores[1].bytes).unwrap()));
    third.import_compositing = doc.layers[1].import_compositing;
    let mut item = doc.xcf_state.as_ref().unwrap().items[1].clone();
    item.item = ItemRef::Layer(third.id);
    item.name = third.name.clone();
    item.raster_bounds = Some(bounds);
    doc.layers.push(third);
    doc.xcf_state.as_mut().unwrap().items.insert(0, item);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let mut b = budget(q);
    let held = LeasedSeparableBlendPipeline::new_general_budgeted(&doc, &Interrupt::NONE, &mut b, owner.clone()).unwrap();
    assert!(b.work > 0);
    assert!(q.usage().unwrap().resident > before.resident);
    assert!(stores.iter().all(|s| s.reads.load(Ordering::Relaxed) == 0));
    drop(held);
    assert_eq!(q.usage().unwrap(), before);
    assert!(LeasedSeparableBlendPipeline::new_budgeted(&doc, &Interrupt::NONE, &mut b, owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
}

#[test]
#[ignore = "UNRUN: exact shared raw rule is mandatory in generic source and CURRENT before explicit fallback, with genuine native ownership retained"]
fn new_epoch_checks_raw_rule_in_source_and_current() {
    let (owner, mapped, _) = general();
    let q = owner.quota().clone();
    for defect in 0..5 {
        let mut doc = mapped.clone();
        let item = &mut doc.xcf_state.as_mut().unwrap().items[1];
        let policy = item.composition.as_mut().unwrap();
        match defect {
            0 => policy.stored_blend_space = -3,
            1 => policy.resolved.as_mut().unwrap().operator = GimpOperator::Union,
            2 => policy.stored_operator = 1, // partial explicit is not a rule
            3 => policy.source_mode = 61,
            _ => policy.compatibility_epoch = "unrecognized-source-epoch".into(),
        }
        let before = q.usage().unwrap();
        assert!(LeasedSeparableBlendPipeline::new_general_budgeted(&doc, &Interrupt::NONE, &mut budget(&q), owner.clone()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut current = owner.document().clone();
    current.size = mapped.size;
    let state = current.xcf_state.as_mut().unwrap();
    state.provenance.format_version = 23;
    for row in &mut state.items {
        row.composition.as_mut().unwrap().compatibility_epoch = EPOCH.into();
    }
    // The observed partial Normal tuple is not a blanket positive-current rule.
    let p = state.items[0].composition.as_mut().unwrap();
    p.stored_composite_space = 1;
    p.stored_operator = 1;
    let current = AccountedDocument::from_owned_readonly_flat_composite(current, Arc::new((owner.clone(), fee)), q.clone(), &ctl()).unwrap();
    let prepared = PreparedBackedSeparableDocument::new(&current, Default::default(), &ctl()).unwrap();
    assert!(prepared.render_band(Rect::new(0, 0, 8, 6), &ctl()).is_ok());
    drop(prepared);
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut bad = current.document().clone();
    // Structurally consistent mixed Auto/explicit tuple, but not an original
    // rule: only Normal (0,1,1), not (0,2,1), was independently witnessed.
    let policy = bad.xcf_state.as_mut().unwrap().items[0].composition.as_mut().unwrap();
    policy.stored_composite_space = 2;
    policy.resolved.as_mut().unwrap().composite_space = GimpSpace::ProfileRgb;
    bad.layers[0].import_compositing.as_mut().unwrap().composite_encoding = SampleEncoding::NonlinearProfile;
    let bad = AccountedDocument::from_owned_readonly_flat_composite(bad, Arc::new((current, fee)), q, &ctl()).unwrap();
    assert!(PreparedBackedSeparableDocument::new(&bad, Default::default(), &ctl()).is_err());
}

#[test]
#[ignore = "UNRUN: bounded dynamic source preparation refuses malformed profile/hidden metadata, duplicate identities and over-limit rows without leaks"]
fn profile_hidden_and_count_refusals() {
    let (owner, mapped, _) = general();
    let q = owner.quota();
    for defect in 0..5 {
        let mut doc = mapped.clone();
        match defect {
            0 => doc.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            1 => {
                doc.layers[1].visible = false;
                doc.xcf_state.as_mut().unwrap().items[1].visible = false;
                doc.layers[1].excluded_channels = 1;
            }
            2 => doc.layers[1].id = doc.layers[0].id,
            3 => {
                let layer = doc.layers[1].clone();
                while doc.layers.len() <= 4096 {
                    doc.layers.push(layer.clone());
                }
            }
            _ => doc.xcf_state.as_mut().unwrap().items[1].raster_bounds = Some(Rect::new(i32::MIN, 0, i32::MAX, 2)),
        }
        let before = q.usage().unwrap();
        assert!(LeasedSeparableBlendPipeline::new_general_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
}

#[test]
#[ignore = "UNRUN: cumulative mapper Budget failure, actual quota refusal and cancellation refund preparation without native reads"]
fn controlled_failures_refund_paid_preparation() {
    let (owner, mapped, stores) = general();
    let q = owner.quota();
    let before = q.usage().unwrap();
    let mut count = budget(q);
    drop(LeasedSeparableBlendPipeline::new_general_budgeted(&mapped, &Interrupt::NONE, &mut count, owner.clone()).unwrap());
    assert_eq!(q.usage().unwrap(), before);
    let mut limited = budget(q);
    limited.limit = count.work - 1;
    assert!(LeasedSeparableBlendPipeline::new_general_budgeted(&mapped, &Interrupt::NONE, &mut limited, owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident).unwrap();
    assert!(LeasedSeparableBlendPipeline::new_general_budgeted(&mapped, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
    drop(blocker);
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert!(matches!(
        LeasedSeparableBlendPipeline::new_general_budgeted(&mapped, &interrupt, &mut budget(q), owner.clone()),
        Err(CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), before);
    assert!(stores.iter().all(|s| s.reads.load(Ordering::Relaxed) == 0));
}
