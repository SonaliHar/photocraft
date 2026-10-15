//! UNRUN synthetic CURRENT/mapper controls. Missing producer Gray ICC captures
//! remain HOLD; matching the existing SGray basis is no byte-equivalence claim.
//! Resident/reference buffers and expected clones are external harness data.
use super::*;
fn alpha(mode: ColorMode, code: i32) -> (AccountedDocument, [Arc<Store>; 2]) {
    let (old, originals) = fixture(30, 0);
    let q = old.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = old.document().clone();
    doc.mode = mode;
    let profile = if mode == ColorMode::Rgb { Builtin::Srgb.profile() } else { Builtin::SGray.profile() };
    doc.icc_profile = Some(Arc::new(profile.to_bytes()));
    doc.working_profile_interpretation = if mode == ColorMode::Rgb {
        WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap())
    } else {
        WorkingProfileInterpretation::implicit_sgray(doc.icc_profile.as_ref().unwrap())
    };
    let f = PixelFormat::new(mode, SampleType::F32, true);
    let mut stores = Vec::new();
    for (index, original) in originals.iter().enumerate() {
        let charge = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
        let values: Vec<f32> = if mode == ColorMode::Rgb {
            if index == 0 { vec![0.2, 0.5, 0.35, 0.7] } else { vec![0.65, 0.25, 0.4, 0.625] }
        } else if index == 0 {
            vec![0.35, 0.7]
        } else {
            vec![0.65, 0.625]
        };
        let pixel: Vec<u8> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
        let bounds = original.descriptor.bounds();
        let bytes = pixel.repeat(bounds.width() as usize * bounds.height() as usize);
        let descriptor = NativePlaneDescriptor::new(f, bounds, &vec![0; f.bytes_per_pixel()]).unwrap();
        let store = Arc::new(Store { descriptor, bytes, q: q.clone(), reads: AtomicU64::new(0), _fee: charge });
        let raster = BackedRaster::new(store.clone(), q.clone(), 2, &ctl()).unwrap();
        doc.layers[index].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
        stores.push(store);
    }
    let (br, cr, op, blend, composite, operator) = match code {
        57 => (-1, -1, -2, GimpSpace::LinearRgb, GimpSpace::LinearRgb, GimpOperator::ClipToBackdrop),
        58 => (0, -1, -2, GimpSpace::ProfileRgb, GimpSpace::LinearRgb, GimpOperator::ClipToBackdrop),
        59 => (0, -1, -1, GimpSpace::ProfileRgb, GimpSpace::LinearRgb, GimpOperator::Union),
        60 => (0, 0, -2, GimpSpace::ProfileRgb, GimpSpace::ProfileRgb, GimpOperator::ClipToBackdrop),
        _ => panic!("fixture"),
    };
    let p = doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap();
    p.source_mode = code;
    p.legacy = false;
    p.compatibility_epoch = "xcf-12-authored-blend-auto".into();
    p.stored_blend_space = br;
    p.stored_composite_space = cr;
    p.stored_operator = op;
    p.resolved = Some(ResolvedGimpComposition { blend_space: blend, composite_space: composite, operator });
    doc.layers[1].import_compositing = Some(ImportCompositing { blend_encoding: enc(blend), composite_encoding: enc(composite), pass_through: false });
    (AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((old, fee)), q, &ctl()).unwrap(), stores.try_into().unwrap())
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
#[ignore = "UNRUN: all8 RGB/Gray native alpha recipes match old resident math while native channels/defaults and Gray display replication remain distinct"]
fn resident_and_native_layout_reference() {
    for model in [ColorMode::Rgb, ColorMode::Grayscale] {
        for code in 57..=60 {
            assert!(lower::is_supported_alpha_mode(code));
            assert!(!lower::is_supported_mode(code));
            let (owner, stores) = alpha(model, code);
            let doc = resident(&owner, &stores);
            let bytes = stores.each_ref().map(|s| s.bytes.clone());
            let expected = crate::gimp_render::PreparedRenderer::new(&doc).unwrap().render(doc.bounds(), &mut |_| Ok(())).unwrap();
            let p = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
            let actual = p.render_band(p.bounds(), &ctl()).unwrap();
            for (a, b) in actual.pixels().iter().zip(&expected.px) {
                assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
                if model == ColorMode::Grayscale {
                    assert_eq!(a[0].to_bits(), a[1].to_bits());
                    assert_eq!(a[0].to_bits(), a[2].to_bits());
                }
            }
            assert_eq!(stores.each_ref().map(|s| s.bytes.clone()), bytes);
            assert_eq!(stores[1].descriptor.format().channels(), if model == ColorMode::Rgb { 4 } else { 2 });
        }
    }
}

#[test]
#[ignore = "UNRUN: exactalpha source preparation uses matching implicit RGB/Gray basis with real fees and no native evaluation; other source constructors stay closed"]
fn source_basis_and_route_separation() {
    for model in [ColorMode::Rgb, ColorMode::Grayscale] {
        for code in 57..=60 {
            let (owner, stores) = alpha(model, code);
            let doc = resident(&owner, &stores);
            let q = owner.quota();
            let before = q.usage().unwrap();
            let held = LeasedSeparableBlendPipeline::new_alpha_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).unwrap();
            assert!(q.usage().unwrap().resident > before.resident);
            assert!(stores.iter().all(|s| s.reads.load(Ordering::Relaxed) == 0));
            drop(held);
            assert_eq!(q.usage().unwrap(), before);
            assert!(LeasedSeparableBlendPipeline::new_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
            assert!(LeasedSeparableBlendPipeline::new_general_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
            assert!(LeasedSeparableBlendPipeline::new_component_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
            let mut wrong = doc.clone();
            wrong.icc_profile = Some(Arc::new(Builtin::GrayGamma22.profile().to_bytes()));
            wrong.working_profile_interpretation = WorkingProfileInterpretation::implicit_sgray(wrong.icc_profile.as_ref().unwrap());
            assert!(LeasedSeparableBlendPipeline::new_alpha_budgeted(&wrong, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
            assert_eq!(q.usage().unwrap(), before);
        }
    }
}

#[test]
#[ignore = "UNRUN: hidden native/default alpha inputs remain unitchecked and Gray CURRENT cannot gain Screen or lose last alpha through modern wire"]
fn current_unit_and_gray_policy_boundaries() {
    for model in [ColorMode::Rgb, ColorMode::Grayscale] {
        for default_bad in [false, true] {
            let (owner, stores) = alpha(model, 58);
            let q = owner.quota().clone();
            let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
            let mut doc = owner.document().clone();
            doc.layers[1].visible = false;
            doc.layers[1].opacity = 0.;
            doc.xcf_state.as_mut().unwrap().items[1].visible = false;
            let values = if model == ColorMode::Rgb { vec![1.25f32, 0.2, 0.3, 0.] } else { vec![1.25f32, 0.] };
            let raw: Vec<u8> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
            if default_bad {
                let charge = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
                let descriptor = NativePlaneDescriptor::new(stores[1].descriptor.format(), stores[1].descriptor.bounds(), &raw).unwrap();
                let store = Arc::new(Store { descriptor, bytes: stores[1].bytes.clone(), q: q.clone(), reads: AtomicU64::new(0), _fee: charge });
                let raster = BackedRaster::new(store, q.clone(), 2, &ctl()).unwrap();
                doc.layers[1].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap());
            } else {
                let LayerContent::BackedRaster(b) = &doc.layers[1].content else { panic!("fixture") };
                doc.layers[1].content = LayerContent::BackedRaster(b.try_edited_native(Rect::new(11, -1, 12, 0), &raw, &ctl()).unwrap());
            }
            let bad = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner, fee)), q, &ctl()).unwrap();
            assert!(PreparedBackedSeparableDocument::new(&bad, Default::default(), &ctl()).is_err());
        }
    }
    let (owner, _) = alpha(ColorMode::Grayscale, 59);
    for source_mode in [28, 31] {
        assert!(
            owner
                .try_with_flat_composite_composition_patch(
                    photocraft_doc::accounted_document::FlatCompositeCompositionPatch {
                        layer: owner.document().layers[1].id,
                        source_mode,
                        blend_space: GimpSpace::LinearRgb,
                        composite_space: GimpSpace::LinearRgb,
                        operator: GimpOperator::Union,
                    },
                    &ctl(),
                )
                .is_err()
        );
        // Independent prepaid CURRENT model construction tests the renderer
        // boundary, without inferring any Gray composition mutation authority.
        let q = owner.quota().clone();
        let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
        let mut doc = owner.document().clone();
        let p = doc.xcf_state.as_mut().unwrap().items[1].composition.as_mut().unwrap();
        p.source_mode = source_mode;
        p.stored_blend_space = 1;
        p.stored_composite_space = 1;
        p.stored_operator = 1;
        p.resolved = Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union });
        doc.layers[1].import_compositing =
            Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
        let next = AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner.clone(), fee)), q, &ctl()).unwrap();
        assert!(PreparedBackedSeparableDocument::new(&next, Default::default(), &ctl()).is_err());
    }
}

#[test]
#[ignore = "UNRUN: alpha rawzero/positivezero branches and same-ledger source/CURRENT work quota cancellation fees are preserved"]
fn alpha_branches_and_actual_fees() {
    for model in [ColorMode::Rgb, ColorMode::Grayscale] {
        let profile = if model == ColorMode::Rgb { Builtin::Srgb.profile() } else { Builtin::SGray.profile() };
        let linear = WorkingSpace::new(profile, SampleEncoding::Linear, GimpSpace::LinearRgb).unwrap();
        let encoded = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let bg = [0.25, 0.25, 0.25, 0.75];
        for code in 57..=60 {
            let ws = if code == 60 { &encoded } else { &linear };
            let blend = if code == 57 { &linear } else { &encoded };
            let op = if code == 59 { GimpOperator::Union } else { GimpOperator::ClipToBackdrop };
            assert_eq!(ws.separable(blend, bg, [0.5, 0.5, 0.5, 0.], 1., op, gimp_blend::mode(code).unwrap()).unwrap().map(f64::to_bits), bg.map(f64::to_bits));
        }
        let erased = linear.separable(&encoded, bg, [0.5, 0.5, 0.5, 0.5], 1., GimpOperator::ClipToBackdrop, gimp_blend::mode(58).unwrap()).unwrap();
        assert_eq!(erased[3], 0.375);
        assert_eq!(&erased[..3], &bg[..3]);
        let split = encoded.separable(&encoded, bg, [0.5, 0.5, 0.5, 1.], 1., GimpOperator::ClipToBackdrop, gimp_blend::mode(60).unwrap()).unwrap();
        assert_eq!(split.map(f64::to_bits), [0f64; 4].map(f64::to_bits));
    }
    let (owner, stores) = alpha(ColorMode::Grayscale, 59);
    let doc = resident(&owner, &stores);
    let q = owner.quota();
    let before = q.usage().unwrap();
    let mut b = budget(q);
    drop(LeasedSeparableBlendPipeline::new_alpha_budgeted(&doc, &Interrupt::NONE, &mut b, owner.clone()).unwrap());
    assert_eq!(q.usage().unwrap(), before);
    let mut limited = budget(q);
    limited.limit = b.work - 1;
    assert!(LeasedSeparableBlendPipeline::new_alpha_budgeted(&doc, &Interrupt::NONE, &mut limited, owner.clone()).is_err());
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident).unwrap();
    assert!(LeasedSeparableBlendPipeline::new_alpha_budgeted(&doc, &Interrupt::NONE, &mut budget(q), owner.clone()).is_err());
    drop(blocker);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(LeasedSeparableBlendPipeline::new_alpha_budgeted(&doc, &interrupt, &mut budget(q), owner.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap());
    let warm = q.usage().unwrap();
    let p = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
    drop(p.render_band(Rect::new(0, 0, 8, 6), &ctl()).unwrap());
    drop(p);
    assert_eq!(q.usage().unwrap(), warm);
}
