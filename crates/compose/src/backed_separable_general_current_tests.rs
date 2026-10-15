//! UNRUN generalized CURRENT fixtures. These genuinely paid native models are
//! independent test-harness constructions, not additional GIMP source evidence.
use super::*;

fn append(doc: &mut Document, q: &BackingQuota, mode: i32, bounds: Rect, explicit: bool, operator: GimpOperator) {
    let (backed, _) = leaf(q, ColorMode::Rgb, SampleType::F32, bounds);
    let mut layer = Layer::new(format!("mixed-{mode}"), LayerContent::BackedRaster(backed));
    layer.opacity = 0.375;
    let blend = if explicit { GimpSpace::LinearRgb } else { GimpSpace::PerceptualRgb };
    layer.import_compositing = Some(ImportCompositing { blend_encoding: enc(blend), composite_encoding: SampleEncoding::Linear, pass_through: false });
    let state = doc.xcf_state.as_mut().unwrap();
    state.items.insert(
        0,
        ItemState {
            item: ItemRef::Layer(layer.id),
            name: layer.name.clone(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(bounds),
            mask: None,
            composition: Some(GimpComposition {
                compatibility_epoch: if explicit { "xcf-23-normal" } else { "xcf-23-authored-blend-auto" }.into(),
                source_mode: mode,
                legacy: false,
                stored_blend_space: if explicit { 1 } else { -4 },
                stored_composite_space: if explicit { 1 } else { -1 },
                stored_operator: if explicit {
                    match operator {
                        GimpOperator::Union => 1,
                        GimpOperator::ClipToBackdrop => 2,
                        GimpOperator::ClipToLayer => 3,
                        GimpOperator::Intersection => 4,
                    }
                } else {
                    -2
                },
                resolved: Some(ResolvedGimpComposition { blend_space: blend, composite_space: GimpSpace::LinearRgb, operator }),
                pass_through: false,
            }),
        },
    );
    doc.layers.push(layer);
}
fn extended(size: Size) -> AccountedDocument {
    let (owner, _) = fixture(31, 0);
    let q = owner.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = owner.document().clone();
    doc.size = size;
    append(&mut doc, &q, 47, Rect::new(-3, 2, 21, 19), false, GimpOperator::ClipToBackdrop);
    append(&mut doc, &q, 33, Rect::new(255, 255, 260, 258), true, GimpOperator::Union);
    AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner, fee)), q, &ctl()).unwrap()
}
fn resident_model(owner: &AccountedDocument) -> Document {
    let mut doc = owner.document().clone();
    for layer in &mut doc.layers {
        let LayerContent::BackedRaster(backed) = &layer.content else { panic!("fixture") };
        let view = backed.checked_view();
        let rect = view.logical_bounds();
        let f = view.format();
        let mut raw = vec![0; rect.width() as usize * rect.height() as usize * f.bytes_per_pixel()];
        let mut at = 0;
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                view.read_pixel_native_into(x, y, &mut raw[at..at + f.bytes_per_pixel()], &ctl()).unwrap();
                at += f.bytes_per_pixel();
            }
        }
        layer.content = LayerContent::Raster(Surface::try_from_interleaved(f, rect, &raw).unwrap());
    }
    doc
}
fn changed(owner: AccountedDocument, edit: impl FnOnce(&mut Document)) -> AccountedDocument {
    let q = owner.quota().clone();
    let fee = q.reserve(QuotaKind::Index, 2 << 20).unwrap();
    let mut doc = owner.document().clone();
    edit(&mut doc);
    AccountedDocument::from_owned_readonly_flat_composite(doc, Arc::new((owner, fee)), q, &ctl()).unwrap()
}

#[test]
#[ignore = "UNRUN: four mixed arbitrary CURRENT leaves with negative/tile-crossing/offcanvas extents match existing resident order and preserve native storage"]
fn mixed_dynamic_geometry_matches_resident_reference() {
    let owner = extended(Size::new(513, 289));
    let reference = resident_model(&owner);
    let renderer = crate::gimp_render::PreparedRenderer::new(&reference).unwrap();
    let prepared = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
    for roi in [Rect::new(0, 0, 23, 21), Rect::new(254, 254, 262, 260)] {
        let actual = prepared.render_band(roi, &ctl()).unwrap();
        let expected = renderer.render(roi, &mut |_| Ok(())).unwrap();
        assert_eq!(actual.pixels().len(), expected.px.len());
        for (a, b) in actual.pixels().iter().zip(&expected.px) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
    }
    assert_eq!(owner.document().layers.len(), 4);
    for (layer, resident) in owner.document().layers.iter().zip(&reference.layers) {
        let LayerContent::BackedRaster(backed) = &layer.content else { panic!("fixture") };
        let rect = backed.operative_bounds();
        let mut raw = [0; 16];
        backed.checked_view().read_pixel_native_into(rect.x0, rect.y0, &mut raw, &ctl()).unwrap();
        let mut expected = [0.; 4];
        resident.surface().unwrap().try_read_pixel_f64(rect.x0, rect.y0, &mut expected).unwrap();
        assert_eq!(lower::decode(&raw, backed.format()).unwrap().map(f64::to_bits), expected.map(f64::to_bits));
    }
}

#[test]
#[ignore = "UNRUN: tiled fullcanvas validation handles max_pixels smaller than one row and max_rows independently with positive bounded progress"]
fn tiny_bands_and_hostile_limits() {
    let owner = extended(Size::new(19, 17));
    for limits in
        [NormalBandLimits { max_rows: 1, max_pixels: 1 }, NormalBandLimits { max_rows: 3, max_pixels: 7 }, NormalBandLimits { max_rows: 1, max_pixels: 32 }]
    {
        let prepared = PreparedBackedSeparableDocument::new(&owner, limits, &ctl()).unwrap();
        prepared.validate_operative(&ctl()).unwrap();
        assert_eq!(prepared.render_band(Rect::new(0, 0, 1, 1), &ctl()).unwrap().pixels().len(), 1);
        if limits.max_pixels < 19 {
            assert!(prepared.render_band(Rect::new(0, 0, 19, 1), &ctl()).is_err());
        }
    }
    for limits in [
        NormalBandLimits { max_rows: 0, max_pixels: 1 },
        NormalBandLimits { max_rows: 1, max_pixels: 0 },
        NormalBandLimits { max_rows: 257, max_pixels: 65536 },
    ] {
        assert!(PreparedBackedSeparableDocument::new(&owner, limits, &ctl()).is_err());
    }
}

#[test]
#[ignore = "UNRUN: arbitrary first visible Multiply has no isolated seed and explicit zero/outside coverage retains all four operator equations"]
fn first_visible_and_zero_coverage_are_not_normalized() {
    let base = extended(Size::new(19, 17));
    let first = changed(base, |doc| {
        // All rows remain stored and validated, but only the Multiply row is visible.
        for layer in &mut doc.layers {
            layer.visible = false;
        }
        let id = {
            let layer = &mut doc.layers[1];
            layer.visible = true;
            layer.opacity = 1.;
            layer.import_compositing =
                Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
            layer.id
        };
        let state = doc.xcf_state.as_mut().unwrap();
        for row in &mut state.items {
            if let ItemRef::Layer(id) = row.item {
                row.visible = doc.layers.iter().find(|l| l.id == id).unwrap().visible;
            }
        }
        let row = state.items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap();
        let p = row.composition.as_mut().unwrap();
        p.source_mode = 30;
        p.stored_blend_space = 1;
        p.stored_composite_space = 1;
        p.stored_operator = 2;
        p.resolved =
            Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::ClipToBackdrop });
    });
    let prepared = PreparedBackedSeparableDocument::new(&first, Default::default(), &ctl()).unwrap();
    assert!(prepared.render_band(prepared.bounds(), &ctl()).unwrap().pixels().iter().all(|p| p.map(f64::to_bits) == [0.; 4].map(f64::to_bits)));
    for operator in [GimpOperator::Union, GimpOperator::ClipToBackdrop, GimpOperator::ClipToLayer, GimpOperator::Intersection] {
        let owner = changed(extended(Size::new(19, 17)), |doc| {
            let layer = doc.layers.last_mut().unwrap();
            layer.opacity = 0.;
            let id = layer.id;
            let p = doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().composition.as_mut().unwrap();
            p.stored_operator = match operator {
                GimpOperator::Union => 1,
                GimpOperator::ClipToBackdrop => 2,
                GimpOperator::ClipToLayer => 3,
                GimpOperator::Intersection => 4,
            };
            p.resolved.as_mut().unwrap().operator = operator;
        });
        let reference = resident_model(&owner);
        let expected = crate::gimp_render::PreparedRenderer::new(&reference).unwrap().render(reference.bounds(), &mut |_| Ok(())).unwrap();
        let prepared = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
        let band = prepared.render_band(prepared.bounds(), &ctl()).unwrap();
        for (a, b) in band.pixels().iter().zip(&expected.px) {
            assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
        }
        if matches!(operator, GimpOperator::ClipToLayer | GimpOperator::Intersection) {
            assert!(band.pixels().iter().all(|p| p[3] == 0.));
        }
    }
}

#[test]
#[ignore = "UNRUN: hidden entirely offcanvas native tail/default defects are refused and dynamic explicit policy never aliases unknown Auto"]
fn complete_hidden_native_and_auto_identity_checks() {
    for defect in 0..9 {
        let owner = changed(extended(Size::new(19, 17)), |doc| {
            let layer = doc.layers.last_mut().unwrap();
            layer.visible = false;
            let id = layer.id;
            doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
            if defect >= 4 {
                match defect {
                    5 => layer.excluded_channels = 1,
                    6 => layer.advanced.knockout = photocraft_doc::Knockout::Deep,
                    _ => {
                        let p = doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().composition.as_mut().unwrap();
                        p.source_mode = if defect == 4 { 47 } else { 28 };
                        p.stored_blend_space = 1;
                        p.stored_composite_space = 1;
                        p.stored_operator = 1;
                        p.resolved = Some(ResolvedGimpComposition {
                            blend_space: GimpSpace::LinearRgb,
                            composite_space: GimpSpace::LinearRgb,
                            operator: GimpOperator::Union,
                        });
                        p.compatibility_epoch = match defect {
                            7 => photocraft_doc::import_compositing::XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH,
                            8 => photocraft_doc::import_compositing::XCF12_FINITE_NORMAL_SOURCE_EPOCH,
                            _ => "xcf-23-authored-blend-auto",
                        }
                        .into();
                    }
                }
                return;
            }
            let LayerContent::BackedRaster(backed) = &layer.content else { panic!("fixture") };
            let r = backed.operative_bounds();
            let values = [f32::NAN, 0.25, 0.25, 0.625];
            let raw: Vec<_> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
            if defect == 0 {
                layer.content = LayerContent::BackedRaster(backed.try_edited_native(Rect::new(r.x1 - 1, r.y1 - 1, r.x1, r.y1), &raw, &ctl()).unwrap());
            } else if defect == 3 {
                let q = backed.quota().clone();
                let fee = q.reserve(QuotaKind::Resident, 1 << 20).unwrap();
                let raw: Vec<u8> = [0.25f32, 0.25, 0.25, 0.625].into_iter().flat_map(f32::to_ne_bytes).collect();
                let bytes = raw.repeat(r.width() as usize * r.height() as usize);
                let descriptor = NativePlaneDescriptor::new(backed.format(), r, &values.into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>()).unwrap();
                let store = Arc::new(Store { descriptor, bytes, q: q.clone(), reads: AtomicU64::new(0), _fee: fee });
                layer.content = LayerContent::BackedRaster(BackedLayer::new(BackedRaster::new(store, q, 2, &ctl()).unwrap(), descriptor, &ctl()).unwrap());
            } else {
                if defect == 1 {
                    layer.import_compositing =
                        Some(ImportCompositing { blend_encoding: SampleEncoding::Perceptual, composite_encoding: SampleEncoding::Linear, pass_through: false });
                }
                let p = doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().composition.as_mut().unwrap();
                p.stored_blend_space = -1;
                p.stored_composite_space = -1;
                p.stored_operator = -2;
                if defect == 1 {
                    p.source_mode = 31;
                    p.stored_blend_space = -4;
                    p.compatibility_epoch = "unknown-Auto-epoch".into();
                } else {
                    p.source_mode = 33;
                    p.compatibility_epoch = "xcf-23-authored-blend-auto".into();
                }
                p.resolved = Some(ResolvedGimpComposition {
                    blend_space: if defect == 1 { GimpSpace::PerceptualRgb } else { GimpSpace::LinearRgb },
                    composite_space: GimpSpace::LinearRgb,
                    operator: GimpOperator::ClipToBackdrop,
                });
            }
        });
        assert!(PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).is_err());
    }
}

#[test]
#[ignore = "UNRUN: dynamic metadata/lower entries/WorkingSpaces coexist under actual quota and late band Work/Cancelled errors refund on warm native caches"]
fn dynamic_descriptor_and_band_resource_rollback() {
    let owner = extended(Size::new(19, 17));
    drop(PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap());
    let q = owner.quota();
    let baseline = q.usage().unwrap();
    {
        let prepared = PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).unwrap();
        let held = q.usage().unwrap();
        assert!(held.index > baseline.index);
        assert!(held.resident > baseline.resident);
        let output = prepared.render_band(prepared.bounds(), &ctl()).unwrap();
        assert!(q.usage().unwrap().resident > held.resident);
        drop(output);
        assert_eq!(q.usage().unwrap(), held);
        let spent = AtomicU64::new(0);
        let work = |n| if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > 100_000 { Err(BackingError::Work) } else { Ok(()) };
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        assert!(matches!(prepared.render_band(prepared.bounds(), &control), Err(E::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), held);
        let cancelled = || true;
        let interrupt = Interrupt::cancel_only(&cancelled);
        let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(matches!(prepared.render_band(prepared.bounds(), &control), Err(E::Backing(BackingError::Cancelled))));
        assert_eq!(q.usage().unwrap(), held);
    }
    assert_eq!(q.usage().unwrap(), baseline);
}

#[test]
#[ignore = "UNRUN: generalized CURRENT eligibility does not enroll arbitrary geometry/layercount in the unchanged original source pipeline"]
fn strict_source_recipe_stays_separate() {
    struct Budget {
        q: BackingQuota,
    }
    impl crate::gimp_render::PreparationBudget for Budget {
        type Lease = QuotaLease;
        fn reserve_prepared_bytes(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
            self.q.reserve(QuotaKind::Resident, n).map_err(|_| CompositionError::Invalid("fixture quota"))
        }
        fn consume_preparation_work(&mut self, _: u64) -> Result<(), CompositionError> {
            Ok(())
        }
    }
    let owner = extended(Size::new(19, 17));
    let mapped = resident_model(&owner);
    let mut budget = Budget { q: owner.quota().clone() };
    let before = budget.q.usage().unwrap();
    assert!(LeasedSeparableBlendPipeline::new_budgeted(&mapped, &Interrupt::NONE, &mut budget, owner.clone()).is_err());
    assert_eq!(budget.q.usage().unwrap(), before);
    assert!(PreparedBackedSeparableDocument::new(&owner, Default::default(), &ctl()).is_ok());
}
