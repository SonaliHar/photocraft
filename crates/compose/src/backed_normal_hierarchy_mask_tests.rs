//! UNRUN synthetic paid hierarchy contracts; no original-XCF or producer-pixel claim.
//! Expected arithmetic buffers/parameters use the independent public-TRC/scalar
//! reference from the existing masked numeric harness, outside quota authority.
use super::*;
use crate::gimp_masked_mixed::{self as masked, Kind};
fn args(kind: Kind) -> Vec<FilterArgument> {
    match kind {
        Kind::Exposure => vec![number("exposure", 0.5), number("black-level", 0.0625)],
        Kind::BrightnessContrast => vec![number("contrast", 0.75), number("brightness", -0.125)],
        Kind::GeglLevels => vec![number("in-low", 0.125), number("in-high", 0.875), number("out-low", 0.0625), number("out-high", 0.9375)],
        _ => vec![],
    }
}
fn coverage(x: u32, sample: SampleType) -> f64 {
    match x % 4 {
        0 => 0.,
        1 => 0.375,
        2 => 1.,
        _ => {
            if sample == SampleType::F64 {
                f64::from_bits(0x3ff0000009b25010)
            } else {
                1.
            }
        }
    }
}
fn masked_fixture(sample: SampleType, kind: Kind) -> Fixture {
    let mut f = fixture(sample, Size::new(8, 6), true);
    // Existing prospective model/Index8MiB/Resident8MiB preholds precede
    // bounded argument/String edits and the actual native-depth mask COW tile.
    let s = f.doc.xcf_state.as_mut().unwrap();
    let n = &mut s.filter_stacks[0].nodes[0];
    n.operation = OperationIdentity { name: kind.operation_name().into(), stored_version: kind.version().into(), schema_epoch: kind.epoch().into() };
    n.arguments = args(kind);
    n.composition.as_mut().unwrap().compatibility_epoch = kind.epoch().into();
    let id = n.mask.unwrap();
    let m = s.filter_masks.iter_mut().find(|m| id == ItemRef::Mask(m.id)).unwrap();
    for y in 0..6 {
        for x in 0..8 {
            m.surface.try_write_pixel_f64(x, y, &[coverage(x as u32, sample)]).unwrap();
        }
    }
    f
}
fn adopt_coverage(f: Fixture) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>) {
    drop(f.template);
    let owner = AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(f.doc, f.retention, f.q, &ctl()).unwrap();
    (owner, f.rasters, f.fails)
}
fn expected(kind: Kind, sample: SampleType, x: u32) -> [f64; 4] {
    let normal =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    let plain = normal.normal([0.; 4], input(0, x, 0, sample), 0.5, GimpOperator::Union).unwrap();
    let leaf = masked::tests::reference(masked::parameters(kind, &args(kind)).unwrap(), input(1, x, 0, sample), coverage(x, sample));
    let first = normal.normal([0.; 4], leaf, 0.75, GimpOperator::Union).unwrap();
    let inner = normal.normal([0.; 4], semi(input(2, x, 0, sample)), 1., GimpOperator::Union).unwrap();
    let outer = normal.normal(first, inner, 0.5, GimpOperator::Union).unwrap();
    normal.normal(plain, outer, 0.375, GimpOperator::Union).unwrap()
}
#[test]
#[ignore = "UNRUN actual nested five masked kernels plus literal zero Semi sibling; fixed isolated Normal reference, local0/1/fullmix, wide alpha and Float coverage preimage"]
fn actual_nested_masked_leaves_keep_isolation_and_full_endpoint_mix() {
    for sample in [SampleType::F32, SampleType::F64] {
        for kind in [Kind::Exposure, Kind::InvertLinear, Kind::InvertGamma, Kind::BrightnessContrast, Kind::GeglLevels] {
            let (o, _, _) = adopt_coverage(masked_fixture(sample, kind));
            assert!(o.is_readonly_normal_hierarchy_storage());
            assert!(o.checked_readonly_normal_hierarchy_index(&ctl()).is_err());
            let ctx = Context::new_current(&o, Default::default(), &ctl()).unwrap();
            ctx.validate_operative(&ctl()).unwrap();
            for x in 0..4 {
                let band = ctx.render_band(Rect::new(x, 0, x + 1, 1), &ctl()).unwrap();
                assert_eq!(band.pixels()[0].map(f64::to_bits), expected(kind, sample, x as u32).map(f64::to_bits));
            }
            assert!(crate::backed_normal_hierarchy_document::PreparedBackedNormalHierarchyDocument::new_byte_zero(&o, Default::default(), &ctl()).is_err());
            let facade = crate::backed_normal_hierarchy_document::PreparedBackedNormalHierarchyDocument::new(&o, Default::default(), &ctl()).unwrap();
            assert_eq!(facade.bounds(), o.document().bounds());
            assert_eq!(facade.current_profile(), photocraft_cms::Builtin::Srgb.profile());
        }
    }
}
struct SourceBudget {
    q: BackingQuota,
    used: u64,
    limit: u64,
}
impl PreparationBudget for SourceBudget {
    type Lease = QuotaLease;
    fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.q.reserve(QuotaKind::Resident, n).map_err(|_| CompositionError::Unsupported("test resident"))
    }
    fn consume_preparation_work(&mut self, n: u64) -> Result<(), CompositionError> {
        self.used = self.used.checked_add(n).ok_or(CompositionError::Unsupported("test work"))?;
        if self.used > self.limit { Err(CompositionError::Unsupported("test work")) } else { Ok(()) }
    }
}
impl HierarchyPreparationBudget for SourceBudget {
    fn reserve_hierarchy_index(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.q.reserve(QuotaKind::Index, n).map_err(|_| CompositionError::Unsupported("test index"))
    }
}
#[test]
#[ignore = "UNRUN explicit synthetic Source22 masked entry and actual Current purpose; old zero Source remains closed, Source+1/allraw proof, same original Index/work refund"]
fn separate_source_roles_use_one_actual_tree_and_original_budget() {
    let f = masked_fixture(SampleType::F64, Kind::InvertGamma);
    let q = f.q.clone();
    let base = q.usage().unwrap();
    let mut b = SourceBudget { q: q.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedNormalHierarchyPipeline::new_budgeted(&f.doc, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).is_err());
    assert_eq!(q.usage().unwrap(), base);
    let source = LeasedNormalHierarchyPipeline::new_masked_source_budgeted(&f.doc, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).unwrap();
    assert!(b.used > 0);
    drop(source);
    assert_eq!(q.usage().unwrap(), base);
    let mut limited = SourceBudget { q: q.clone(), used: 0, limit: 1 };
    assert!(LeasedNormalHierarchyPipeline::new_masked_source_budgeted(&f.doc, &Interrupt::NONE, &mut limited, Arc::clone(&f.retention)).is_err());
    assert_eq!(q.usage().unwrap(), base);
    for fault in 0..3 {
        let mut f = masked_fixture(SampleType::F64, Kind::Exposure);
        match fault {
            0 => f.doc.xcf_state.as_mut().unwrap().filter_stacks[1].nodes[0].opacity_f64_bits = 0.5f64.to_bits(),
            1 => {
                let s = f.doc.xcf_state.as_mut().unwrap();
                s.provenance.format_version = 13;
            }
            _ => {
                let s = f.doc.xcf_state.as_mut().unwrap();
                let format = s.filter_masks[0].surface.format();
                s.filter_masks[0].surface = Surface::new(format);
            }
        }
        let mut b = SourceBudget { q: f.q.clone(), used: 0, limit: u64::MAX };
        assert!(LeasedNormalHierarchyPipeline::new_masked_source_budgeted(&f.doc, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).is_err());
    }
}
#[test]
#[ignore = "UNRUN complete inactive nonempty/zero mask proof, signedzero third state and nonempty-opacity closure; warmed same-ledger Work/cancel and last native cold-Io atomic refunds"]
fn inactive_masks_and_failed_render_never_become_transparent_shortcuts() {
    for fault in 0..4 {
        let mut f = masked_fixture(SampleType::F64, Kind::Exposure);
        f.doc.layers[1].visible = false;
        reindex(&mut f);
        let s = f.doc.xcf_state.as_mut().unwrap();
        s.filter_stacks[0].nodes[0].enabled = false;
        match fault {
            0 => s.filter_stacks[0].nodes[0].opacity_f64_bits = 0.5f64.to_bits(),
            1 => s.filter_masks[0].surface.try_write_pixel_f64(255, 255, &[f64::NAN]).unwrap(),
            2 => {
                let format = s.filter_masks[0].surface.format();
                s.filter_masks[0].surface = Surface::new(format);
                s.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[-0.]).unwrap();
            }
            _ => s.filter_stacks[0].nodes[0].operation.schema_epoch = crate::EXPOSURE_MASKED_EPOCH.into(),
        }
        drop(f.template);
        let owner = AccountedDocument::new_readonly_masked_normal_hierarchy_current_storage(f.doc, f.retention, f.q, &ctl());
        if let Ok(o) = owner {
            assert!(Context::new_current(&o, Default::default(), &ctl()).is_err());
        }
    }
    let (o, rasters, fails) = adopt_coverage(masked_fixture(SampleType::F64, Kind::InvertLinear));
    let q = o.quota().clone();
    let ctx = Context::new_current(&o, Default::default(), &ctl()).unwrap();
    ctx.validate_operative(&ctl()).unwrap();
    let r = Rect::new(0, 0, 2, 1);
    drop(ctx.render_band(r, &ctl()).unwrap());
    let base = q.usage().unwrap();
    let total = AtomicU64::new(0);
    drop(
        ctx.render_band(
            r,
            &ReadControl {
                interrupt: &Interrupt::NONE,
                work: &|n| {
                    total.fetch_add(n, Ordering::Relaxed);
                    Ok(())
                },
            },
        )
        .unwrap(),
    );
    let used = AtomicU64::new(0);
    assert!(
        ctx.render_band(
            r,
            &ReadControl {
                interrupt: &Interrupt::NONE,
                work: &|n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > total.load(Ordering::Relaxed) / 2 {
                    Err(BackingError::Work)
                } else {
                    Ok(())
                }
            }
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap(), base);
    let cancel = || true;
    let interrupt = Interrupt::cancel_only(&cancel);
    assert!(ctx.render_band(r, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), base);
    rasters[2].clear_cache().unwrap();
    let base = q.usage().unwrap();
    fails[2].store(true, Ordering::Relaxed);
    assert!(ctx.render_band(r, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), base);
    fails[2].store(false, Ordering::Relaxed);
}
