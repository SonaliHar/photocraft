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
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 16 << 20, scratch: 0 });
    // Genuine prospective fee before fixture DTO/maps/profile allocation. All
    // native stores also retain their actual backing allocations on this q.
    let fee = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let native_fee = q.reserve(QuotaKind::Resident, 8 << 20).unwrap();
    let mut doc = Document::new("masked.pcraft", Size::new(29, 21), ColorMode::Rgb, sample);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    let profile = Arc::new(vec![0x91; 132]);
    doc.icc_profile = Some(profile.clone());
    doc.source_color_context = Some(crate::SourceColorContext {
        source_precision: match sample {
            SampleType::U16 => 250,
            SampleType::U32 => 350,
            SampleType::F32 => 650,
            _ => 750,
        },
        original_icc: Some(profile),
    });
    doc.working_profile_interpretation = WorkingProfileInterpretation::Icc;
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 12;
    let epoch = if matches!(sample, SampleType::U16 | SampleType::U32) {
        XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH
    } else {
        XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH
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
