//! UNRUN generated CURRENT-only assigned-profile relation drafts. The fixture's
//! native/model allocations retain actual prepaid owners; it makes no XCF source claim.
use super::*;
use crate::backed_stock_point_context::tests::{ctl, fixture};
use photocraft_color::SampleType;
use photocraft_doc::{
    Document, Layer, LayerMask,
    import_compositing::{
        GimpComposition, GimpOperator, GimpSpace, ImportCompositing, ResolvedGimpComposition, XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
        XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH, XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
    },
    item_identity::{ItemRef, MaskId},
    xcf_state::MaskState,
};
use photocraft_raster::{Interrupt, backing::quota::BackingQuota};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
fn model(stored: SampleEncoding, defect: u8) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, &'static Profile) {
    let sample = SampleType::F64;
    let (mut doc, q, guard, _, profile, _) = fixture(2, ColorMode::Rgb, sample, stored);
    // Fixed generated model: two flat items, one zero-tile mask, <=64KiB ICC.
    // Reserve before duplicate names/map/default/profile allocations; not a codec proof.
    let extra = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let mut top = doc.layers.remove(0);
    let base = Layer::new("Backdrop", top.content.clone());
    top.opacity = 0.375;
    let bounds = doc.bounds();
    let state = doc.xcf_state.as_mut().unwrap();
    let mask_surface = state.filter_masks[0].surface.clone();
    let mask_id = MaskId(9000);
    top.mask = Some(LayerMask { surface: Arc::new(mask_surface), enabled: true, linked: true, density: 1., feather: 0. });
    let template = state.items.iter().find(|i| i.item == ItemRef::Layer(top.id)).unwrap().clone();
    let epoch = match stored {
        SampleEncoding::Linear => XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
        SampleEncoding::Perceptual => XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
        SampleEncoding::NonlinearProfile => XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH,
    };
    let policy = |foreground| GimpComposition {
        compatibility_epoch: epoch.into(),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 0,
        stored_composite_space: if foreground { 1 } else { -1 },
        stored_operator: if foreground { 1 } else { -1 },
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        pass_through: false,
    };
    let mut backdrop = template.clone();
    backdrop.item = ItemRef::Layer(base.id);
    backdrop.name = base.name.clone();
    backdrop.composition = Some(policy(false));
    backdrop.raster_bounds = Some(bounds);
    let mut foreground = template.clone();
    foreground.name = top.name.clone();
    foreground.composition = Some(policy(true));
    foreground.raster_bounds = Some(bounds);
    let mut mask = template;
    mask.item = ItemRef::Mask(mask_id);
    mask.name = "Mask".into();
    mask.composition = None;
    mask.raster_bounds = Some(bounds);
    mask.mask = Some(MaskState {
        owner: ItemRef::Layer(top.id),
        bounds,
        offset: [bounds.x0, bounds.y0],
        outside_f64_bits: 0f64.to_bits(),
        enabled: true,
        shown: false,
        editing: false,
        linked: true,
    });
    state.items = vec![backdrop, foreground, mask];
    state.filter_stacks.clear();
    state.filter_masks.clear();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 12;
    top.import_compositing =
        Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    let mut base = base;
    base.import_compositing = top.import_compositing;
    doc.layers = vec![base, top];
    let icc = Arc::new(profile.encoded_bytes().unwrap().to_vec());
    let parsed = Profile::parse(icc.as_slice()).unwrap();
    let effective = xcf_color::effective_profile_from_definition(
        &parsed,
        match stored {
            SampleEncoding::Linear => Encoding::Linear,
            SampleEncoding::Perceptual => Encoding::Perceptual,
            SampleEncoding::NonlinearProfile => Encoding::NonlinearProfile,
        },
    )
    .unwrap();
    let mut current = effective.encoded_bytes().unwrap().to_vec();
    // CURRENT-only negative fixture; never a decoded source assertion.
    if defect == 1 {
        current[24] ^= 1;
    }
    if defect == 2 {
        current[64..68].copy_from_slice(&4u32.to_be_bytes());
    }
    doc.icc_profile = Some(Arc::new(current));
    doc.source_color_context.as_mut().unwrap().original_icc = Some(icc);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    (doc, q, Arc::new((guard, extra)), profile)
}
fn adopt(doc: Document, q: BackingQuota, guard: Arc<dyn std::any::Any + Send + Sync>) -> AccountedDocument {
    AccountedDocument::from_owned(doc, guard, q, &ctl()).unwrap()
}

#[test]
fn exact_current_relation_retains_generated_math_and_model_identity() {
    for encoding in [SampleEncoding::NonlinearProfile, SampleEncoding::Linear, SampleEncoding::Perceptual] {
        let (doc, q, guard, _) = model(encoding, 0);
        let owner = adopt(doc, q.clone(), guard);
        let before = q.usage().unwrap();
        let binding = PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).unwrap();
        assert!(binding.matches(&owner, encoding));
        assert!(!binding.matches(&owner, if encoding == SampleEncoding::Linear { SampleEncoding::Perceptual } else { SampleEncoding::Linear }));
        assert_eq!(binding.profile().encoded_bytes().unwrap(), owner.document().icc_profile.as_deref().unwrap().as_slice());
        assert!(q.usage().unwrap().resident > before.resident);
        if encoding == SampleEncoding::Perceptual {
            let reparsed = Profile::parse(binding.profile().encoded_bytes().unwrap()).unwrap();
            let generated = &binding.profile().trc.as_ref().unwrap()[0];
            let decoded = &reparsed.trc.as_ref().unwrap()[0];
            assert_ne!(generated, decoded);
            assert_eq!(generated.eval64(0.25).to_bits(), photocraft_cms::curve::srgb_trc().eval64(0.25).to_bits());
        }
        let renamed = owner.try_with_document_name("Fresh model", &ctl()).unwrap();
        assert!(!binding.matches(&renamed, encoding));
        drop(renamed);
        drop(binding);
        assert_eq!(q.usage().unwrap(), before);
    }
}
#[test]
fn wrong_current_relation_and_strict_invalid_intent_are_not_normalized() {
    for defect in [1, 2] {
        let (doc, q, guard, _) = model(SampleEncoding::Perceptual, defect);
        let owner = adopt(doc, q.clone(), guard);
        let before = q.usage().unwrap();
        assert!(PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
}
#[test]
fn cumulative_work_late_cancel_and_quota_failure_refund_private_profiles() {
    let (doc, q, guard, _) = model(SampleEncoding::Linear, 0);
    let owner = adopt(doc, q.clone(), guard);
    let before = q.usage().unwrap();
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let ready = PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &control).unwrap();
    let total = spent.load(Ordering::Relaxed);
    assert!(total > 100_000);
    drop(ready);
    assert_eq!(q.usage().unwrap(), before);
    let spent = AtomicU64::new(0);
    let work = |n| {
        if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > total / 2 { Err(BackingError::Cancelled) } else { Ok(()) }
    };
    assert!(matches!(
        PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ReadControl { interrupt: &Interrupt::NONE, work: &work }),
        Err(Error::Backing(BackingError::Cancelled))
    ));
    assert_eq!(q.usage().unwrap(), before);
    let mut limits = AssignedCurrentProfileLimits::default();
    limits.cms.work_units = 1;
    assert!(PreparedAssignedCurrentProfile::prepare(&owner, limits, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    // The shared native fixture declares a 64MiB real resident cap.
    let free = (64u64 << 20) - before.resident;
    let hold = q.reserve(QuotaKind::Resident, free).unwrap();
    let held = q.usage().unwrap();
    assert!(PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), held);
    drop(hold);
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn lower_only_limits_and_encoded_bounds_do_not_allocate() {
    let (doc, q, guard, _) = model(SampleEncoding::NonlinearProfile, 0);
    let owner = adopt(doc, q.clone(), guard);
    let before = q.usage().unwrap();
    for bytes in [0, 131, 65537] {
        let limits = AssignedCurrentProfileLimits { max_encoded_bytes: bytes, ..Default::default() };
        assert!(PreparedAssignedCurrentProfile::prepare(&owner, limits, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let mut lowered = AssignedCurrentProfileLimits::default();
    lowered.cms.resident_bytes = 0;
    assert!(matches!(PreparedAssignedCurrentProfile::prepare(&owner, lowered, &ctl()), Err(Error::Backing(BackingError::Quota))));
    assert_eq!(q.usage().unwrap(), before);
    let mut limits = AssignedCurrentProfileLimits::default();
    limits.cms.resident_bytes = limits.cms.resident_bytes.checked_add(1).unwrap();
    assert!(PreparedAssignedCurrentProfile::prepare(&owner, limits, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
