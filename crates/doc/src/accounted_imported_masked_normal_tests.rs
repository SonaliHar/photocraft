//! UNRUN synthetic structural custody only. The bounded ICC bytes below are
//! deliberately not an analytic-profile/source/evaluator qualification witness.
use super::*;
use crate::{BackedLayer, Layer, LayerMask, Size, import_compositing::*, item_identity::MaskId};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        out.fill(0);
        Ok(())
    }
}
fn fixture(sample: SampleType) -> AccountedDocument {
    fixture_encoding(sample, SampleEncoding::NonlinearProfile)
}
pub(super) fn fixture_encoding(sample: SampleType, encoding: SampleEncoding) -> AccountedDocument {
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 16 << 20, scratch: 0 });
    // Genuine prospective fee before fixture DTO/maps/profile allocation. All
    // native stores also retain their actual backing allocations on this q.
    let fee = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let native_fee = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let mut doc = Document::new("masked.pcraft", Size::new(29, 21), ColorMode::Rgb, sample);
    doc.sample_encoding = Some(encoding);
    let profile = Arc::new(vec![0x91; 132]);
    doc.icc_profile = Some(if encoding == SampleEncoding::NonlinearProfile {
        profile.clone()
    } else {
        // Structural-only independent current bytes, not a CMS witness.
        Arc::new(vec![0x92; 133])
    });
    doc.source_color_context = Some(crate::SourceColorContext {
        source_precision: match (sample, encoding) {
            (SampleType::U16, _) => 250,
            (SampleType::U32, _) => 350,
            (SampleType::F32, SampleEncoding::Linear) => 600,
            (SampleType::F64, SampleEncoding::Linear) => 700,
            (SampleType::F32, SampleEncoding::Perceptual) => 675,
            (SampleType::F64, SampleEncoding::Perceptual) => 775,
            (SampleType::F32, _) => 650,
            _ => 750,
        },
        original_icc: Some(profile),
    });
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 12;
    let epoch = match encoding {
        SampleEncoding::Linear => XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH,
        SampleEncoding::Perceptual => XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH,
        _ if matches!(sample, SampleType::U16 | SampleType::U32) => XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH,
        _ => XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH,
    };
    for (i, extent) in [doc.bounds(), Rect::new(-3, -4, 4, 9)].into_iter().enumerate() {
        let format = PixelFormat::new(doc.mode, sample, true);
        let descriptor = NativePlaneDescriptor::new(format, extent, &vec![0; format.bytes_per_pixel()]).unwrap();
        let backing = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &ctl()).unwrap();
        let mut layer = Layer::new(format!("layer{i}"), LayerContent::BackedRaster(BackedLayer::new(backing, descriptor, &ctl()).unwrap()));
        layer.import_compositing =
            Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
        if i == 1 {
            layer.opacity = 0.625;
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
            surface.try_write_pixel_f64(-3, -4, &[0.25]).unwrap();
            layer.mask = Some(LayerMask { surface: Arc::new(surface), enabled: true, linked: false, density: 1., feather: 0. });
            state.items.push(ItemState {
                item: ItemRef::Mask(MaskId(901)),
                name: "mask".into(),
                custom_name: false,
                visible: true,
                locks: Default::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(extent),
                mask: Some(MaskState {
                    owner: ItemRef::Layer(layer.id),
                    bounds: extent,
                    offset: [extent.x0, extent.y0],
                    outside_f64_bits: 0f64.to_bits(),
                    enabled: true,
                    shown: false,
                    editing: false,
                    linked: false,
                }),
                composition: None,
            });
        }
        state.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: layer.name.clone(),
            custom_name: false,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(extent),
            mask: None,
            composition: Some(GimpComposition {
                compatibility_epoch: epoch.into(),
                source_mode: 28,
                legacy: false,
                stored_blend_space: 0,
                stored_composite_space: if i == 0 { -1 } else { 1 },
                stored_operator: if i == 0 { -1 } else { 1 },
                resolved: Some(ResolvedGimpComposition {
                    blend_space: GimpSpace::LinearRgb,
                    composite_space: GimpSpace::LinearRgb,
                    operator: GimpOperator::Union,
                }),
                pass_through: false,
            }),
        });
        doc.layers.push(layer);
    }
    state.selection.active = Some(ItemRef::Layer(doc.layers[1].id));
    state.selection.selected.push(ItemRef::Layer(doc.layers[1].id));
    doc.xcf_state = Some(state);
    AccountedDocument::from_owned(doc, Arc::new((fee, native_fee)), q, &ctl()).unwrap()
}

#[test]
fn title_copy_preserves_actual_mask_native_profiles_and_original_geometry_epochs() {
    for sample in [SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let source = fixture(sample);
        source.validate_masked_analytic_normal_storage(&ctl()).unwrap();
        let copy = source.try_with_document_name("continued.pcraft", &ctl()).unwrap();
        let mut expected = source.document().clone(); // external expected-model oracle
        expected.name = "continued.pcraft".into();
        assert_eq!(copy.document(), &expected);
        assert!(Arc::ptr_eq(&source.document().layers[1].mask.as_ref().unwrap().surface, &copy.document().layers[1].mask.as_ref().unwrap().surface));
        assert!(Arc::ptr_eq(source.document().icc_profile.as_ref().unwrap(), copy.document().icc_profile.as_ref().unwrap()));
        assert!(Arc::ptr_eq(&source.owner.retention, &copy.owner.retention));
        assert!(copy.quota().shares_ledger(source.quota()));
        assert!(copy.try_edit(AccountedEdit::LayerName { layer: copy.document().layers[0].id, name: "generic" }, &ctl()).is_err());
        assert!(copy.try_replace_selection(None, &ctl()).is_err());
    }
}

#[test]
fn noop_at_full_quota_and_failed_copies_preserve_owner_and_refund() {
    let source = fixture(SampleType::F64);
    let q = source.quota();
    let baseline = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - baseline.index).unwrap();
    assert!(source.try_with_document_name(&source.document().name, &ctl()).unwrap().shares_model(&source));
    assert!(source.try_with_document_name("changed", &ctl()).is_err());
    drop(blocker);
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(source.try_with_document_name("changed", &deny).is_err());
    let stop = || q.usage().unwrap().index > baseline.index;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(source.try_with_document_name("changed", &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    let first = source.try_with_document_name("a", &ctl()).unwrap();
    let second = first.try_with_document_name("b", &ctl()).unwrap();
    assert!(Arc::ptr_eq(&source.owner.retention, &second.owner.retention));
    drop(first);
    drop(second);
    assert_eq!(q.usage().unwrap(), baseline);
}

#[test]
fn contradictory_profiles_epochs_mask_geometry_or_operator_refuse_before_title_copy() {
    for case in 0..6 {
        let source = fixture(SampleType::F32);
        let q = source.quota().clone();
        let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
        let mut doc = source.document().clone();
        let state = doc.xcf_state.as_mut().unwrap();
        match case {
            0 => doc.icc_profile = Some(Arc::new(vec![0x92; 132])),
            1 => state.items.iter_mut().find_map(|i| i.composition.as_mut()).unwrap().compatibility_epoch = XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH.into(),
            2 => state.items.iter_mut().find_map(|i| i.mask.as_mut()).unwrap().offset[0] += 1,
            3 => state.items.iter_mut().find_map(|i| i.composition.as_mut()).unwrap().resolved.as_mut().unwrap().operator = GimpOperator::Intersection,
            4 => doc.sample_encoding = Some(SampleEncoding::Linear),
            _ => state.items.push(state.items[0].clone()),
        }
        let malformed = AccountedDocument::from_owned(doc, Arc::new((source, fee)), q.clone(), &ctl()).unwrap();
        let baseline = q.usage().unwrap();
        assert!(malformed.try_with_document_name("refused", &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
}

#[test]
fn assigned_transfer_storage_preserves_independent_profiles_and_shared_payloads() {
    for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
        for sample in [SampleType::F32, SampleType::F64] {
            let source = fixture_encoding(sample, encoding);
            source.validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
            assert!(source.validate_masked_analytic_normal_storage(&ctl()).is_err());
            let original = source.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap();
            let current = source.document().icc_profile.as_ref().unwrap();
            assert_ne!(original.as_slice(), current.as_slice());
            let copy = source.try_with_document_name("transfer-continued.pcraft", &ctl()).unwrap();
            copy.validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
            assert_eq!(copy.document().sample_encoding, Some(encoding));
            assert_eq!(copy.document().depth, sample);
            assert!(Arc::ptr_eq(original, copy.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()));
            assert!(Arc::ptr_eq(current, copy.document().icc_profile.as_ref().unwrap()));
            assert!(Arc::ptr_eq(&source.document().layers[1].mask.as_ref().unwrap().surface, &copy.document().layers[1].mask.as_ref().unwrap().surface));
            assert!(Arc::ptr_eq(&source.owner.retention, &copy.owner.retention));
            assert!(source.quota().shares_ledger(copy.quota()));
            let mut expected = source.document().clone(); // external structural oracle
            expected.name = "transfer-continued.pcraft".into();
            assert_eq!(copy.document(), &expected);
            assert!(copy.try_replace_selection(None, &ctl()).is_err());
            assert!(copy.try_edit(AccountedEdit::LayerName { layer: copy.document().layers[0].id, name: "generic" }, &ctl()).is_err());
        }
    }
}

#[test]
fn assigned_transfer_wrong_precision_epoch_or_independent_buffer_bounds_refuse() {
    for case in 0..6 {
        let source = fixture_encoding(SampleType::F32, SampleEncoding::Linear);
        let q = source.quota().clone();
        let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
        let mut doc = source.document().clone();
        match case {
            0 => doc.source_color_context.as_mut().unwrap().source_precision = 675,
            1 => doc.sample_encoding = Some(SampleEncoding::Perceptual),
            2 => {
                doc.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|i| i.composition.as_mut()).unwrap().compatibility_epoch =
                    XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH.into()
            }
            3 => doc.icc_profile = Some(Arc::new(vec![0; 131])),
            4 => doc.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(vec![0; 65537])),
            _ => doc.icc_profile = Some(Arc::new(vec![0; 65537])),
        }
        let invalid = AccountedDocument::from_owned(doc, Arc::new((source, fee)), q.clone(), &ctl()).unwrap();
        let baseline = q.usage().unwrap();
        assert!(invalid.validate_assigned_analytic_geometry_storage(&ctl()).is_err());
        assert!(invalid.try_with_document_name("refused", &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
}

#[test]
fn assigned_transfer_title_failures_and_full_quota_noop_keep_exact_owners() {
    let source = fixture_encoding(SampleType::F64, SampleEncoding::Perceptual);
    let q = source.quota();
    let baseline = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - baseline.index).unwrap();
    assert!(source.try_with_document_name(&source.document().name, &ctl()).unwrap().shares_model(&source));
    assert!(source.try_with_document_name("changed", &ctl()).is_err());
    drop(blocker);
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(source.validate_assigned_analytic_geometry_storage(&deny).is_err());
    let stop = || q.usage().unwrap().index > baseline.index;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(source.try_with_document_name("changed", &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
}

// Synthetic metadata-only operator custody. checked_resolved, not a guessed
// enum alias, binds each explicit native word. No source/render proof here.
fn operator_owner(sample: SampleType, encoding: SampleEncoding, raw: i32, resolved: GimpOperator) -> AccountedDocument {
    let source = fixture_encoding(sample, encoding);
    let q = source.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = source.document().clone();
    let top = doc.layers[1].id;
    let p = doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(top)).unwrap().composition.as_mut().unwrap();
    p.stored_operator = raw;
    p.resolved.as_mut().unwrap().operator = resolved;
    AccountedDocument::from_owned(doc, Arc::new((source, fee)), q, &ctl()).unwrap()
}
#[test]
fn broad_operator_custody_preserves_raw_policy_while_old_nonlinear_gate_stays_union_only() {
    let operators = [(1, GimpOperator::Union), (2, GimpOperator::ClipToBackdrop), (3, GimpOperator::ClipToLayer), (4, GimpOperator::Intersection)];
    for (sample, encoding) in [
        (SampleType::U16, SampleEncoding::NonlinearProfile),
        (SampleType::U32, SampleEncoding::NonlinearProfile),
        (SampleType::F32, SampleEncoding::NonlinearProfile),
        (SampleType::F64, SampleEncoding::NonlinearProfile),
        (SampleType::F32, SampleEncoding::Linear),
        (SampleType::F64, SampleEncoding::Linear),
        (SampleType::F32, SampleEncoding::Perceptual),
        (SampleType::F64, SampleEncoding::Perceptual),
    ] {
        for (raw, resolved) in operators {
            let source = operator_owner(sample, encoding, raw, resolved);
            source.validate_assigned_analytic_geometry_storage(&ctl()).unwrap();
            assert_eq!(source.validate_masked_analytic_normal_storage(&ctl()).is_ok(), encoding == SampleEncoding::NonlinearProfile && raw == 1);
            let renamed = source.try_with_document_name("operator.pcraft", &ctl()).unwrap();
            assert_eq!(renamed.document().xcf_state, source.document().xcf_state);
            assert!(Arc::ptr_eq(&renamed.document().layers[1].mask.as_ref().unwrap().surface, &source.document().layers[1].mask.as_ref().unwrap().surface));
            assert_eq!(renamed.document().icc_profile, source.document().icc_profile);
            assert_eq!(renamed.document().source_color_context, source.document().source_color_context);
            assert!(renamed.try_replace_selection(None, &ctl()).is_err());
        }
    }
}
#[test]
fn operator_mismatch_out_of_domain_and_backdrop_nonunion_fail_without_fee_change() {
    for (raw, resolved) in [(0, GimpOperator::Union), (5, GimpOperator::Union), (2, GimpOperator::Intersection)] {
        let source = operator_owner(SampleType::F32, SampleEncoding::NonlinearProfile, raw, resolved);
        let baseline = source.quota().usage().unwrap();
        assert!(source.validate_assigned_analytic_geometry_storage(&ctl()).is_err());
        assert!(source.try_with_document_name("refused", &ctl()).is_err());
        assert_eq!(source.quota().usage().unwrap(), baseline);
    }
    let source = fixture(SampleType::F64);
    let q = source.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = source.document().clone();
    let p = doc.xcf_state.as_mut().unwrap().items.iter_mut().find_map(|i| i.composition.as_mut()).unwrap();
    p.stored_operator = 2;
    p.resolved.as_mut().unwrap().operator = GimpOperator::ClipToBackdrop;
    let invalid = AccountedDocument::from_owned(doc, Arc::new((source, fee)), q, &ctl()).unwrap();
    assert!(invalid.validate_assigned_analytic_geometry_storage(&ctl()).is_err());
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(invalid.validate_assigned_analytic_geometry_storage(&denied).is_err());
}
