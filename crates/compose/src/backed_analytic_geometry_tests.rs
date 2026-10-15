//! UNRUN generated CURRENT-only geometry. Native source observation and codec
//! custody are independently tested by the IO/engine workflows.
use super::*;
use crate::backed_stock_point_context::tests::{ctl, fixture};
use photocraft_doc::{
    Document, Layer, LayerMask,
    import_compositing::{GimpComposition, ImportCompositing, ResolvedGimpComposition},
    item_identity::MaskId,
    xcf_state::MaskState,
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::quota::{BackingQuota, QuotaLease},
};
use std::sync::Arc;
fn model(sample: SampleType) -> (Document, BackingQuota, Arc<dyn std::any::Any + Send + Sync>, &'static Profile) {
    let (mut doc, q, guard, _, profile, _) = fixture(2, ColorMode::Rgb, sample, SampleEncoding::NonlinearProfile);
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
    let epoch = if matches!(sample, SampleType::U16 | SampleType::U32) {
        XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH
    } else {
        XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH
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
    doc.icc_profile = Some(icc.clone());
    doc.source_color_context.as_mut().unwrap().original_icc = Some(icc);
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    (doc, q, Arc::new((guard, extra)), profile)
}
fn adopt(doc: Document, q: BackingQuota, guard: Arc<dyn std::any::Any + Send + Sync>) -> AccountedDocument {
    AccountedDocument::from_owned(doc, guard, q, &ctl()).unwrap()
}
#[test]
fn current_geometry_preserves_distinct_epoch_and_reuses_linear_union_resolution() {
    for sample in [SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let (doc, q, guard, profile) = model(sample);
        let owner = adopt(doc, q, guard);
        assert!(is_candidate(owner.document(), &ctl()).unwrap());
        assert_eq!(qualify(&owner, profile, &ctl()).unwrap(), GimpSpace::LinearRgb);
        let epoch = &owner.document().xcf_state.as_ref().unwrap().items[0].composition.as_ref().unwrap().compatibility_epoch;
        assert_eq!(
            epoch,
            if matches!(sample, SampleType::U16 | SampleType::U32) {
                XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH
            } else {
                XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH
            }
        );
    }
}
#[test]
fn current_geometry_rejects_profile_tuple_and_epoch_alias_without_mutation() {
    for case in 0..4 {
        let (mut doc, q, guard, profile) = model(SampleType::F64);
        match case {
            0 => doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap().stored_operator = 2,
            1 => {
                doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap().compatibility_epoch =
                    photocraft_doc::import_compositing::XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH.into()
            }
            2 => doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap()),
            _ => doc.source_color_context.as_mut().unwrap().source_precision = 700,
        };
        let owner = adopt(doc, q, guard);
        assert!(qualify(&owner, profile, &ctl()).is_err());
    }
    let (doc, q, guard, profile) = model(SampleType::F64);
    let owner = adopt(doc, q.clone(), guard);
    qualify(&owner, profile, &ctl()).unwrap();
    let before = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(qualify(&owner, profile, &denied), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), before);
    assert!(qualify(&owner, photocraft_cms::Builtin::SGray.profile(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn full_operative_scanner_rejects_nonunit_and_cancel_before_native_access() {
    let q = BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits { resident: 16 << 20, index: 4 << 20, scratch: 0 });
    let _fee: QuotaLease = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let mut surface = Surface::new(photocraft_color::PixelFormat::RGBA64F);
    let rect = photocraft_geom::Rect::new(0, 0, 1, 1);
    surface.try_write_region_f64_with(rect, &[0.25, 0.5, 1.5, 1.0], &Interrupt::NONE).unwrap();
    let view = CheckedRasterView::resident_with_control(&surface, rect, &ctl()).unwrap();
    assert!(scan(view, &ctl()).is_err());
    let stop = Interrupt::cancel_only(&|| true);
    let control = ReadControl { interrupt: &stop, work: &|_| Ok(()) };
    let view = CheckedRasterView::resident_with_control(&surface, rect, &ctl()).unwrap();
    assert!(matches!(scan(view, &control), Err(E::Backing(BackingError::Cancelled))));
}
