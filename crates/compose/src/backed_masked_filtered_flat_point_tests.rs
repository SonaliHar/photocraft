//! UNRUN synthetic paid actual-owner programme contracts, not original GIMP
//! import or projection/format oracles. Genuine eight-source IO coverage is separate.
use super::*;
use crate::{
    backed_masked_filtered_flat_point_document::PreparedBackedMaskedFilteredFlatPointDocument,
    gimp_masked_mixed::{self as masked, Kind},
};
impl crate::backed_masked_filtered_flat_point_context::MaskedFilterPreparationBudget for Budget {
    fn reserve_filter_mask_proof_index(&mut self, n: u64) -> Result<QuotaLease, CompositionError> {
        self.q.reserve(QuotaKind::Index, n).map_err(|_| photocraft_raster::RasterInputError::LimitExceeded.into())
    }
}
fn masked_args(k: Kind) -> Vec<FilterArgument> {
    match k {
        Kind::Exposure => vec![number("exposure", 0.5), number("black-level", 0.0625)],
        Kind::BrightnessContrast => vec![number("contrast", 0.75), number("brightness", -0.125)],
        Kind::GeglLevels => vec![number("in-low", 0.125), number("in-high", 0.875), number("out-low", 0.0625), number("out-high", 0.9375)],
        _ => vec![],
    }
}
fn set_masked(n: &mut photocraft_doc::xcf_filters::FilterNode, k: Kind) {
    n.operation = OperationIdentity { name: k.operation_name().into(), stored_version: k.version().into(), schema_epoch: k.epoch().into() };
    n.arguments = masked_args(k);
    n.composition.as_mut().unwrap().compatibility_epoch = k.epoch().into();
}
fn masked_fixture(sample: SampleType, kind: Kind) -> Fixture {
    let mut f = ordered(sample, &[&[0, 0], &[]], true);
    // Inherited prospective Index8MiB/Resident8MiB precedes each bounded node,
    // native and mask COW edit below; actual store buffers have their own fees.
    let s = f.document.xcf_state.as_mut().unwrap();
    set_masked(&mut s.filter_stacks[0].nodes[0], kind);
    let id = s.filter_stacks[0].nodes[0].mask.unwrap();
    let mask = s.filter_masks.iter_mut().find(|m| id == ItemRef::Mask(m.id)).unwrap();
    for y in 0..6 {
        for x in 0..8 {
            let v = match x % 4 {
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
            };
            mask.surface.try_write_pixel_f64(x, y, &[v]).unwrap();
        }
    }
    f
}
fn adopt_masked(f: Fixture) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>, Arc<AtomicU64>) {
    let o = AccountedDocument::new_readonly_masked_filtered_flat_point_current_storage(f.document, f.retention, f.quota, &ctl()).unwrap();
    (o, f.rasters, f.fails, f.reads)
}
fn coverage_at(x: i32, sample: SampleType) -> f64 {
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
fn masked_reference(k: Kind, before: [f64; 4], raw: f64) -> [f64; 4] {
    crate::gimp_masked_mixed::tests::reference(masked::parameters(k, &masked_args(k)).unwrap(), before, raw)
}
#[test]
#[ignore = "UNRUN actual two-root ordered five masked leaves plus literal zero Semi sibling, independent fixed order/Normal math and derived finite crossing"]
fn real_ordered_masks_use_actual_previous_wide_values() {
    let (nl, linear) = parts(); // Independent public reference buffers/Parts are external harness.
    let normal =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        for kind in [Kind::Exposure, Kind::InvertLinear, Kind::InvertGamma, Kind::BrightnessContrast, Kind::GeglLevels] {
            let (o, _, _, _) = adopt_masked(masked_fixture(sample, kind));
            assert!(o.is_readonly_masked_filtered_flat_point_storage());
            assert!(!o.is_readonly_filtered_flat_point_storage());
            assert!(!o.is_readonly_point_storage());
            let ctx = Context::new_masked(&o, Default::default(), &ctl()).unwrap();
            ctx.validate_operative(&ctl()).unwrap();
            for x in [0u32, 1, 2, 3, 4] {
                let source = input(0, x, 0, sample);
                let semi_first = step(0, source, false, &nl, &linear);
                let expected = masked_reference(kind, bridge(semi_first), coverage_at(x as i32, sample));
                assert_eq!(root_value(&ctx, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
                let expected = normal
                    .normal(normal.normal([0.; 4], expected, 1., GimpOperator::Union).unwrap(), input(1, x, 0, sample), 0.375, GimpOperator::Union)
                    .unwrap();
                assert_eq!(ctx.render_band(Rect::new(x as i32, 0, x as i32 + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
            }
            let facade = PreparedBackedMaskedFilteredFlatPointDocument::new(&o, Default::default(), &ctl()).unwrap();
            assert_eq!(facade.bounds(), o.document().bounds());
            assert!(PreparedBackedFilteredFlatPointDocument::new(&o, Default::default(), &ctl()).is_err());
            drop(facade);
        }
        let mut f = masked_fixture(sample, Kind::InvertGamma);
        let s = f.document.xcf_state.as_mut().unwrap();
        let n = &mut s.filter_stacks[0].nodes[1];
        n.operation = OperationIdentity {
            name: "gegl:brightness-contrast".into(),
            stored_version: "0:0".into(),
            schema_epoch: crate::BRIGHTNESS_CONTRAST_MIXED_EPOCH.into(),
        };
        n.composition.as_mut().unwrap().compatibility_epoch = crate::BRIGHTNESS_CONTRAST_MIXED_EPOCH.into();
        n.arguments = vec![number("contrast", 2.), number("brightness", 1.)];
        let (o, _, _, _) = adopt_masked(f);
        let ctx = Context::new_masked(&o, Default::default(), &ctl()).unwrap();
        let p = input(0, 1, 0, sample);
        let trc = photocraft_cms::srgb_trc();
        let mut hdr = p;
        for v in &mut hdr[..3] {
            *v = trc.eval_inverse64((trc.eval64(*v) - 0.5) * 2. + 0.5 + 1.);
        }
        assert!(hdr[..3].iter().any(|v| *v > 1.));
        assert_eq!(root_value(&ctx, 0, 1, 0).map(f64::to_bits), masked_reference(Kind::InvertGamma, bridge(hdr), 0.375).map(f64::to_bits));
    }
}
#[test]
#[ignore = "UNRUN distinct proof classifications, mask/epoch mismatch, signed-zero-only refusal, Source+1 closure and pre-scan aggregate cap"]
fn selected_role_and_all_inactive_proofs_are_not_epoch_aliases() {
    for fault in 0..7 {
        let mut f = masked_fixture(SampleType::F64, Kind::Exposure);
        let s = f.document.xcf_state.as_mut().unwrap();
        let n = &mut s.filter_stacks[0].nodes[0];
        n.enabled = false;
        let id = n.mask.unwrap();
        match fault {
            0 => n.opacity_f64_bits = 0.375f64.to_bits(),
            1 => n.operation.schema_epoch = crate::EXPOSURE_MASKED_EPOCH.into(),
            2 => {
                let m = s.filter_masks.iter_mut().find(|m| id == ItemRef::Mask(m.id)).unwrap();
                m.surface = Surface::new(m.surface.format());
                m.surface.try_write_pixel_f64(0, 0, &[-0.]).unwrap();
            }
            3 => {
                let m = s.filter_masks.iter_mut().find(|m| id == ItemRef::Mask(m.id)).unwrap();
                m.surface.try_write_pixel_f64(255, 255, &[0.375]).unwrap();
            }
            4 => {
                let m = s.filter_masks.iter_mut().find(|m| id == ItemRef::Mask(m.id)).unwrap();
                m.surface.try_write_pixel_f64(1, 0, &[f64::NAN]).unwrap();
            }
            5 => n.composition.as_mut().unwrap().source_mode = 28,
            _ => n.source_region = Some(1),
        }
        let Fixture { document, retention, quota, .. } = f;
        match AccountedDocument::new_readonly_masked_filtered_flat_point_current_storage(document, retention, quota, &ctl()) {
            Err(_) => {}
            Ok(o) => assert!(Context::new_masked(&o, Default::default(), &ctl()).is_err()),
        }
    }
    let mut f = masked_fixture(SampleType::F64, Kind::InvertGamma);
    let s = f.document.xcf_state.as_mut().unwrap();
    s.filter_stacks[0].nodes[1].opacity_f64_bits = 0.375f64.to_bits();
    let q = f.quota.clone();
    let base = q.usage().unwrap();
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedMaskedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).is_err());
    assert_eq!(q.usage().unwrap(), base);
    let (o, _, _, _) = adopt_masked(f);
    Context::new_masked(&o, Default::default(), &ctl()).unwrap().validate_operative(&ctl()).unwrap();
    // Actual descriptor-only cap fault precedes census/sample reads. Map entries
    // alias an already preheld physical tile; no producer/geometric claim follows.
    let mut f = masked_fixture(SampleType::F64, Kind::Exposure);
    let q = f.quota.clone();
    let extra = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    f.retention = Arc::new((f.retention, extra));
    let s = f.document.xcf_state.as_mut().unwrap();
    let m = &mut s.filter_masks[0];
    let tile = Arc::clone(m.surface.tiles().next().unwrap().1);
    for tx in 1..33 {
        m.surface.put_tiles(std::iter::once((photocraft_geom::TileCoord { tx, ty: 0 }, Arc::clone(&tile))));
    }
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|n| if n > 16 { Err(BackingError::Work) } else { Ok(()) } };
    assert_eq!(
        f.document.checked_f32_filter_mask_coverage_set_requirements(photocraft_doc::xcf_filter_masks::MaskCoverageSetLimits::STANDARD, &denied).err(),
        Some(BackingError::Length)
    );
    let mut f = ordered(SampleType::F64, &[&[0, 0, 0, 0, 0], &[]], false);
    let extra = f.quota.reserve(QuotaKind::Index, 1 << 20).unwrap();
    f.retention = Arc::new((f.retention, extra));
    let s = f.document.xcf_state.as_mut().unwrap();
    // Five individually eligible31-tile F64 masks exceed aggregate64MiB.
    // Shared payload and outgrid coordinates are intentionally downstream-invalid;
    // this descriptor-only refusal claims no successful structural publication.
    s.filter_masks[0].surface.try_write_pixel_f64(0, 0, &[0.375]).unwrap();
    let tile = Arc::clone(s.filter_masks[0].surface.tiles().next().unwrap().1);
    for mask in &mut s.filter_masks {
        mask.surface = Surface::new(mask.surface.format());
        for tx in 0..31 {
            mask.surface.put_tiles(std::iter::once((photocraft_geom::TileCoord { tx, ty: 0 }, Arc::clone(&tile))));
        }
    }
    assert_eq!(
        f.document.checked_f32_filter_mask_coverage_set_requirements(photocraft_doc::xcf_filter_masks::MaskCoverageSetLimits::STANDARD, &denied).err(),
        Some(BackingError::Length)
    );
}
#[test]
#[ignore = "UNRUN same-ledger Source preparation/Current new purpose, disabled exact wide, retained empty roots, late Work/cancel/cold provider cleanup"]
fn owners_controls_and_empty_continuation_keep_full_proof() {
    let f = masked_fixture(SampleType::F64, Kind::BrightnessContrast);
    let q = f.quota.clone();
    let base = q.usage().unwrap();
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    let source = LeasedMaskedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, Arc::clone(&f.retention)).unwrap();
    drop(source);
    assert_eq!(q.usage().unwrap(), base);
    let (o, rasters, fails, _) = adopt_masked(f);
    let ctx = Context::new_masked(&o, Default::default(), &ctl()).unwrap();
    ctx.validate_operative(&ctl()).unwrap();
    let rect = Rect::new(0, 0, 1, 1);
    drop(ctx.render_band(rect, &ctl()).unwrap());
    let base = q.usage().unwrap();
    let total = AtomicU64::new(0);
    ctx.render_band(
        rect,
        &ReadControl {
            interrupt: &Interrupt::NONE,
            work: &|n| {
                total.fetch_add(n, Ordering::Relaxed);
                Ok(())
            },
        },
    )
    .unwrap();
    let used = AtomicU64::new(0);
    let work = ReadControl {
        interrupt: &Interrupt::NONE,
        work: &|n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > total.load(Ordering::Relaxed) / 2 { Err(BackingError::Work) } else { Ok(()) },
    };
    assert!(ctx.render_band(rect, &work).is_err());
    assert_eq!(q.usage().unwrap(), base);
    let stop = || true;
    let i = Interrupt::cancel_only(&stop);
    assert!(ctx.render_band(rect, &ReadControl { interrupt: &i, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), base);
    rasters[1].clear_cache().unwrap();
    let base = q.usage().unwrap();
    fails[1].store(true, Ordering::Relaxed);
    assert!(ctx.render_band(rect, &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), base);
    fails[1].store(false, Ordering::Relaxed);
    drop(ctx);
    let mut f = masked_fixture(SampleType::F64, Kind::GeglLevels);
    for n in &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes {
        n.enabled = false;
    }
    let (o, _, _, _) = adopt_masked(f);
    let ctx = Context::new_masked(&o, Default::default(), &ctl()).unwrap();
    assert_eq!(root_value(&ctx, 0, 4, 0).map(f64::to_bits), input(0, 4, 0, SampleType::F64).map(f64::to_bits));
    let mut f = masked_fixture(SampleType::F64, Kind::InvertLinear);
    let s = f.document.xcf_state.as_mut().unwrap();
    s.filter_stacks[0].nodes.clear();
    s.filter_masks.clear();
    s.items.retain(|r| matches!(r.item, ItemRef::Layer(_)));
    let (o, _, _, _) = adopt_masked(f);
    let ctx = Context::new_masked(&o, Default::default(), &ctl()).unwrap();
    ctx.validate_operative(&ctl()).unwrap();
    assert!(ctx.plan.entries[0].point.is_none());
    assert!(o.is_readonly_masked_filtered_flat_point_storage());
    assert!(Context::new(&o, Default::default(), &ctl()).is_err());
}
