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

// These are CURRENT-only producer/consumer integration drafts; they do not
// manufacture original GIMP source qualification or compare normalized profiles.
fn assigned_model(sample: SampleType, stored: SampleEncoding) -> (AccountedDocument, BackingQuota) {
    let (mut doc, q, guard, basis) = model(sample);
    let cms_encoding = match stored {
        SampleEncoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
        SampleEncoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
        SampleEncoding::NonlinearProfile => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
    };
    // Existing fixed generated fixture reserve precedes all replacement model
    // strings/profile bytes/mask allocations; the actual returned owner retains it.
    let effective = photocraft_cms::xcf_color::effective_profile_from_definition(basis, cms_encoding).unwrap();
    doc.icc_profile = Some(effective.to_bytes());
    doc.sample_encoding = Some(stored);
    doc.source_color_context.as_mut().unwrap().source_precision = match (sample, stored) {
        (SampleType::F32, SampleEncoding::Linear) => 600,
        (SampleType::F64, SampleEncoding::Linear) => 700,
        (SampleType::F32, SampleEncoding::Perceptual) => 675,
        (SampleType::F64, SampleEncoding::Perceptual) => 775,
        _ => panic!("closed generated assigned geometry fixture"),
    };
    let epoch = match stored {
        SampleEncoding::Linear => XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
        SampleEncoding::Perceptual => XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
        _ => panic!("L/P integration fixture"),
    };
    for item in &mut doc.xcf_state.as_mut().unwrap().items {
        if let Some(policy) = &mut item.composition {
            policy.compatibility_epoch = epoch.into();
        }
    }
    let mask = doc.layers[1].mask.as_mut().unwrap();
    mask.surface = Arc::new(Surface::with_default(mask.surface.format(), &[0.5]));
    (adopt(doc, q.clone(), guard), q)
}
fn native_storage_hash(owner: &AccountedDocument) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let c = ctl();
    for layer in &owner.document().layers {
        let LayerContent::BackedRaster(backed) = &layer.content else { panic!("backed fixture") };
        let view = backed.checked_view();
        view.default_bytes().hash(&mut hash);
        for coord in backed.operative_bounds().tiles() {
            let pin = view.pin_tile(coord, &c).unwrap();
            let r = coord.rect();
            for y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    let mut raw = [0; 40];
                    let pixel = &mut raw[..view.format().bytes_per_pixel()];
                    pin.read_pixel_native_into(x, y, pixel, &c).unwrap();
                    hash.write(pixel);
                }
            }
        }
        if let Some(mask) = &layer.mask {
            mask.surface.default_bytes().hash(&mut hash);
            // This generated fixture has sparse default-only coverage; no hidden
            // mask tile is omitted from the identity assertion.
            assert_eq!(mask.surface.tile_count(), 0);
        }
    }
    hash.finish()
}
#[test]
fn typed_assigned_factory_all_linear_perceptual_depth_pairs_matches_existing_resident_math() {
    use crate::{
        backed_assigned_current_profile::{AssignedCurrentProfileLimits, PreparedAssignedCurrentProfile},
        backed_document::PreparedBackedDocument,
        backed_normal::NormalBandLimits,
    };
    for sample in [SampleType::F32, SampleType::F64] {
        for stored in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
            let (owner, q) = assigned_model(sample, stored);
            let raw = native_storage_hash(&owner);
            let binding = PreparedAssignedCurrentProfile::prepare(&owner, AssignedCurrentProfileLimits::default(), &ctl()).unwrap();
            assert!(qualify(&owner, binding.profile(), &ctl()).is_err(), "old bare-profile endpoint stays NL-only");
            assert_eq!(qualify_assigned(&owner, &binding, &ctl()).unwrap(), GimpSpace::LinearRgb);
            let prepared = PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &binding, NormalBandLimits::default(), &ctl()).unwrap();
            let actual = prepared.render_band(owner.document().bounds(), &ctl()).unwrap();
            // This independent existing renderer consumes ordinary resident layers.
            // Its generated model/materialization/output buffers are test-owned and
            // prepaid on the real fixture ledger before any clone or allocation.
            let _reference_fee = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
            let mut resident = owner.document().clone();
            for layer in &mut resident.layers {
                let LayerContent::BackedRaster(backed) = &layer.content else { panic!("backed fixture") };
                let rect = backed.operative_bounds();
                let mut values = Vec::new();
                for y in rect.y0..rect.y1 {
                    for x in rect.x0..rect.x1 {
                        let mut pixel = [0.; 4];
                        backed.checked_view().read_logical_pixel_f64_into(x, y, &mut pixel, &ctl()).unwrap();
                        values.extend_from_slice(&pixel);
                    }
                }
                let mut surface = Surface::new(backed.format());
                surface.try_write_region_f64_with(rect, &values, &Interrupt::NONE).unwrap();
                layer.content = LayerContent::Raster(surface);
            }
            let reference = crate::gimp_render::PreparedRenderer::new(&resident).unwrap().render(resident.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(actual.rect(), reference.rect);
            assert_eq!(actual.pixels().len(), reference.px.len());
            for (a, b) in actual.pixels().iter().zip(&reference.px) {
                for (a, b) in a.iter().zip(b) {
                    assert!((a - b).abs() < 1e-6, "{sample:?}/{stored:?}: {a} != {b}");
                }
            }
            assert_eq!(native_storage_hash(&owner), raw);
        }
    }
}
#[test]
fn assigned_factory_refuses_other_model_binding_and_control_without_mutation() {
    use crate::{
        backed_assigned_current_profile::{AssignedCurrentProfileLimits, PreparedAssignedCurrentProfile},
        backed_document::PreparedBackedDocument,
        backed_normal::NormalBandLimits,
    };
    let (owner, q) = assigned_model(SampleType::F64, SampleEncoding::Perceptual);
    let binding = PreparedAssignedCurrentProfile::prepare(&owner, AssignedCurrentProfileLimits::default(), &ctl()).unwrap();
    let (other, _) = assigned_model(SampleType::F64, SampleEncoding::Perceptual);
    assert!(!binding.matches(&other, SampleEncoding::Perceptual));
    assert!(!binding.matches(&owner, SampleEncoding::Linear));
    assert!(PreparedBackedDocument::new_accounted_with_assigned_profile(&other, &binding, NormalBandLimits::default(), &ctl()).is_err());
    qualify_assigned(&owner, &binding, &ctl()).unwrap();
    let before = q.usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(
        PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &binding, NormalBandLimits::default(), &denied),
        Err(E::Backing(BackingError::Work))
    ));
    assert_eq!(q.usage().unwrap(), before);
}

// CURRENT gate compatibility only. Per-operator compositing equations are
// exercised separately by the leaf/host groups; this is not a source witness.
fn assigned_operator(sample: SampleType, stored: SampleEncoding, raw: i32, operator: GimpOperator) -> AccountedDocument {
    let owner = if stored == SampleEncoding::NonlinearProfile {
        let (doc, q, guard, _) = model(sample);
        adopt(doc, q, guard)
    } else {
        assigned_model(sample, stored).0
    };
    let q = owner.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mut doc = owner.document().clone();
    let id = doc.layers[1].id;
    let policy = doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().composition.as_mut().unwrap();
    policy.stored_operator = raw;
    policy.resolved.as_mut().unwrap().operator = operator;
    adopt(doc, q, Arc::new((owner, fee)))
}
#[test]
fn typed_nonunion_gates_preserve_bare_profile_compatibility_and_native_storage() {
    use crate::backed_assigned_current_profile::PreparedAssignedCurrentProfile;
    for (sample, stored) in [
        (SampleType::U16, SampleEncoding::NonlinearProfile),
        (SampleType::U32, SampleEncoding::NonlinearProfile),
        (SampleType::F32, SampleEncoding::NonlinearProfile),
        (SampleType::F64, SampleEncoding::NonlinearProfile),
        (SampleType::F32, SampleEncoding::Linear),
        (SampleType::F64, SampleEncoding::Linear),
        (SampleType::F32, SampleEncoding::Perceptual),
        (SampleType::F64, SampleEncoding::Perceptual),
    ] {
        for (word, op) in [(1, GimpOperator::Union), (2, GimpOperator::ClipToBackdrop), (3, GimpOperator::ClipToLayer), (4, GimpOperator::Intersection)] {
            let owner = assigned_operator(sample, stored, word, op);
            let raw = native_storage_hash(&owner);
            let binding = PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).unwrap();
            assert_eq!(qualify_assigned(&owner, &binding, &ctl()).unwrap(), GimpSpace::LinearRgb);
            assert_eq!(qualify(&owner, binding.profile(), &ctl()).is_ok(), stored == SampleEncoding::NonlinearProfile && word == 1);
            let baseline = owner.quota().usage().unwrap();
            let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
            assert!(qualify_assigned(&owner, &binding, &denied).is_err());
            let stop = Interrupt::cancel_only(&|| true);
            assert!(qualify_assigned(&owner, &binding, &ReadControl { interrupt: &stop, work: &|_| Ok(()) }).is_err());
            assert_eq!(owner.quota().usage().unwrap(), baseline);
            assert_eq!(native_storage_hash(&owner), raw);
        }
    }
}
#[test]
fn typed_geometry_rejects_raw_resolved_operator_contradictions_and_nonunion_backdrop() {
    use crate::backed_assigned_current_profile::PreparedAssignedCurrentProfile;
    for (word, op) in [(0, GimpOperator::Union), (5, GimpOperator::Union), (2, GimpOperator::Intersection)] {
        let owner = assigned_operator(SampleType::F32, SampleEncoding::Linear, word, op);
        let baseline = owner.quota().usage().unwrap();
        assert!(PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).is_err());
        assert_eq!(owner.quota().usage().unwrap(), baseline);
    }
    let source = assigned_operator(SampleType::F64, SampleEncoding::Perceptual, 4, GimpOperator::Intersection);
    let q = source.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mut doc = source.document().clone();
    let id = doc.layers[0].id;
    let p = doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().composition.as_mut().unwrap();
    p.stored_operator = 2;
    p.resolved.as_mut().unwrap().operator = GimpOperator::ClipToBackdrop;
    let invalid = adopt(doc, q, Arc::new((source, fee)));
    assert!(PreparedAssignedCurrentProfile::prepare(&invalid, Default::default(), &ctl()).is_err());
}

// Activity belongs to CURRENT metadata only. These generated fixtures retain
// their genuine paid native/source owners; no hidden-source eligibility follows.
fn activity_owner(visible: [bool; 2], opacity: [f32; 2], defect: u8) -> AccountedDocument {
    let source = assigned_operator(SampleType::F64, SampleEncoding::NonlinearProfile, 4, GimpOperator::Intersection);
    let q = source.quota().clone();
    let fee = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
    let mut doc = source.document().clone();
    for (index, layer) in doc.layers.iter_mut().enumerate() {
        layer.visible = visible[index];
        layer.opacity = opacity[index];
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap().visible = visible[index];
    }
    match defect {
        1 | 2 => {
            let layer = doc.layers.get_mut(if defect == 1 { 0 } else { 1 }).unwrap();
            let LayerContent::BackedRaster(backed) = &layer.content else { panic!("generated backed fixture") };
            let r = backed.operative_bounds();
            let last = photocraft_geom::Rect::new(r.x1 - 1, r.y1 - 1, r.x1, r.y1);
            let mut raw = [0; 32];
            backed.checked_view().read_logical_pixel_native_into(last.x0, last.y0, &mut raw, &ctl()).unwrap();
            photocraft_color::try_write_sample_f64(&mut raw, SampleType::F64, 0, 1.5).unwrap();
            layer.content = LayerContent::BackedRaster(backed.try_edited_native(last, &raw, &ctl()).unwrap());
        }
        3 => {
            let r = doc.bounds();
            let layer = &mut doc.layers[1];
            let mask = layer.mask.as_mut().unwrap();
            let mut surface = Surface::new(mask.surface.format());
            surface.try_write_pixel_f64(r.x1 - 1, r.y1 - 1, &[1.5]).unwrap();
            mask.surface = Arc::new(surface);
        }
        4 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(doc.layers[1].id)).unwrap().visible = !visible[1],
        5 => {
            let mut bytes = doc.icc_profile.as_ref().unwrap().as_ref().clone();
            bytes[0] = 255;
            doc.icc_profile = Some(Arc::new(bytes));
        }
        _ => (),
    }
    adopt(doc, q, Arc::new((source, fee)))
}
#[test]
fn typed_current_activity_allows_hidden_and_both_layer_opacities_without_narrow_route_enrollment() {
    use crate::{backed_assigned_current_profile::PreparedAssignedCurrentProfile, backed_document::PreparedBackedDocument};
    for (visible, opacity) in [
        ([false, false], [0.25, 0.375]),
        ([false, true], [0.0, 0.375]),
        ([true, false], [0.25, 0.0]),
        ([true, true], [0.0, 0.0]),
        ([true, true], [0.25, 0.375]),
    ] {
        let owner = activity_owner(visible, opacity, 0);
        let raw = native_storage_hash(&owner);
        let binding = PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).unwrap();
        assert_eq!(qualify_assigned(&owner, &binding, &ctl()).unwrap(), GimpSpace::LinearRgb);
        assert!(qualify(&owner, binding.profile(), &ctl()).is_err(), "old narrow source-visible/full-backdrop path remains closed");
        let renderer = PreparedBackedDocument::new_accounted_with_assigned_profile(&owner, &binding, Default::default(), &ctl()).unwrap();
        let band = renderer.render_band(owner.document().bounds(), &ctl()).unwrap();
        assert!(band.pixels().iter().flatten().all(|v| v.is_finite()));
        if !visible[0] || opacity[0] == 0.0 || (visible[1] && opacity[1] == 0.0) {
            assert!(band.pixels().iter().all(|v| *v == [0.; 4]), "Intersection with hidden/zero source or backdrop is transparent");
        }
        assert_eq!(native_storage_hash(&owner), raw);
        // Isolate the activity gate from the independent old non-Union refusal.
        let q = owner.quota().clone();
        let fee = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
        let mut union = owner.document().clone();
        let id = union.layers[1].id;
        let p = union.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().composition.as_mut().unwrap();
        p.stored_operator = 1;
        p.resolved.as_mut().unwrap().operator = GimpOperator::Union;
        let union = adopt(union, q, Arc::new((owner.clone(), fee)));
        let union_binding = PreparedAssignedCurrentProfile::prepare(&union, Default::default(), &ctl()).unwrap();
        qualify_assigned(&union, &union_binding, &ctl()).unwrap();
        assert!(qualify(&union, union_binding.profile(), &ctl()).is_err(), "old Union path still rejects changed source activity");
    }
}
#[test]
fn hidden_current_layers_and_mask_still_validate_last_native_word_and_profile() {
    use crate::backed_assigned_current_profile::PreparedAssignedCurrentProfile;
    for defect in [1, 2, 3] {
        let owner = activity_owner([false, false], [0.0, 0.0], defect);
        let binding = PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).unwrap();
        // Descriptor/profile preparation is insufficient: the hidden malformed
        // last pixel in EACH layer or mask must fail full operative validation.
        assert!(qualify_assigned(&owner, &binding, &ctl()).is_err(), "hidden malformed tail {defect}");
    }
    for defect in [4, 5] {
        let owner = activity_owner([false, false], [0.0, 0.0], defect);
        assert!(PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).is_err(), "hidden metadata/profile defect {defect}");
    }
    for opacity in [[f32::NAN, 0.0], [0.0, f32::INFINITY], [-0.25, 0.0], [0.0, 1.25]] {
        let owner = activity_owner([false, false], opacity, 0);
        assert!(PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).is_err());
    }
}
#[test]
fn hidden_current_full_validation_preserves_control_and_refunds_private_scratch() {
    use crate::backed_assigned_current_profile::PreparedAssignedCurrentProfile;
    let owner = activity_owner([false, false], [0.0, 0.0], 0);
    let binding = PreparedAssignedCurrentProfile::prepare(&owner, Default::default(), &ctl()).unwrap();
    qualify_assigned(&owner, &binding, &ctl()).unwrap(); // warm genuine native cache before baseline
    let before = owner.quota().usage().unwrap();
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(qualify_assigned(&owner, &binding, &denied), Err(E::Backing(BackingError::Work))));
    let cancelled = || true;
    let stop = Interrupt::cancel_only(&cancelled);
    assert!(matches!(qualify_assigned(&owner, &binding, &ReadControl { interrupt: &stop, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(owner.quota().usage().unwrap(), before);
}

#[path = "backed_analytic_geometry_mask_tests.rs"]
mod mask_activity;
