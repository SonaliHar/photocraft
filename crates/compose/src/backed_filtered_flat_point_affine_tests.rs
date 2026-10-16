//! Ignored UNRUN mixed affine declared arithmetic/custody, never a producer projection oracle.
use super::*;
fn set_affine(n: &mut photocraft_doc::xcf_filters::FilterNode, kind: crate::AffineKind) {
    n.operation = OperationIdentity { name: kind.operation_name().into(), stored_version: kind.version().into(), schema_epoch: kind.epoch().into() };
    n.composition.as_mut().unwrap().compatibility_epoch = kind.epoch().into();
    n.arguments = match kind {
        crate::AffineKind::BrightnessContrast => vec![number("contrast", 0.75), number("brightness", -0.125)],
        crate::AffineKind::GeglLevels => vec![number("in-low", 0.125), number("in-high", 0.875), number("out-low", 0.0625), number("out-high", 0.9375)],
    };
}
fn affine_fixture(sample: SampleType, kind: crate::AffineKind, amount: f64, size: Size) -> Fixture {
    let mut f = if size == Size::new(8, 6) {
        ordered(sample, &[&[0], &[]], true)
    } else {
        let mut f = fixture(sample, size, &[Some(0), None], None);
        let index = f.quota.reserve(QuotaKind::Index, 8 << 20).unwrap();
        let resident = f.quota.reserve(QuotaKind::Resident, 8 << 20).unwrap();
        f.retention = Arc::new(Extra { _prior: f.retention, _index: index, _resident: resident });
        f
    };
    let n = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    set_affine(n, kind);
    n.opacity_f64_bits = amount.to_bits();
    f
}
fn affine(kind: crate::AffineKind, p: [f64; 4]) -> [f64; 4] {
    let curve = photocraft_cms::srgb_trc();
    let mut out = p;
    for c in 0..3 {
        let x = curve.eval64(p[c]);
        let y = match kind {
            crate::AffineKind::BrightnessContrast => (x - 0.5) * 0.75 + 0.5 - 0.125,
            crate::AffineKind::GeglLevels => (x - 0.125) / (0.875 - 0.125) * (0.9375 - 0.0625) + 0.0625,
        };
        out[c] = curve.eval_inverse64(y);
    }
    out
}
#[test]
#[ignore = "UNRUN bothaffine kinds/precisions/fullorigin8x6+130x65 solepaidStage, oldSourceunit andexplicitSourceOpacitycrossproduct, independentrawroot/Normal equations and exacttypedword/rawgroup custody"]
fn source_and_current_are_real_paid_roots() {
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap(); // external harness reference
    for sample in [SampleType::F32, SampleType::F64] {
        for kind in [crate::AffineKind::BrightnessContrast, crate::AffineKind::GeglLevels] {
            for size in [Size::new(8, 6), Size::new(130, 65)] {
                for amount in [1., 0., -0., 0.375] {
                    let mut f = affine_fixture(sample, kind, amount, size);
                    groups(&mut f);
                    let nodes = f.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.clone();
                    let raw = f.document.xcf_state.as_ref().unwrap().opaque.clone();
                    let q = f.quota.clone();
                    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
                    if amount == 1. {
                        drop(LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap());
                        assert!(
                            LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err()
                        );
                    } else {
                        drop(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap());
                        assert!(LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
                    }
                    let (o, _, _, _) = adopt(f);
                    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
                    c.validate_operative(&ctl()).unwrap();
                    for x in 0u32..5 {
                        let p = input(0, x, 0, sample);
                        let expected = mix_reference(p, affine(kind, p), amount);
                        assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
                        let normal = working
                            .normal(working.normal([0.; 4], expected, 1., GimpOperator::Union).unwrap(), input(1, x, 0, sample), 0.375, GimpOperator::Union)
                            .unwrap();
                        assert_eq!(
                            c.render_band(Rect::new(x as i32, 0, x as i32 + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits),
                            normal.map(f64::to_bits)
                        );
                    }
                    assert_eq!(o.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes, nodes);
                    assert_eq!(o.document().xcf_state.as_ref().unwrap().opaque, raw);
                }
            }
        }
    }
}
#[test]
#[ignore = "UNRUN namednoncommutingLevels-thenBC vsBC-thenLevels, reversecanonical order, existing32workNLFloat bridges andExposure neighbor, enabledzero/fullleaf versusdisabled, no familybatch or ownleafFloat"]
fn named_sequences_and_opacity_do_not_change_leaf_boundaries() {
    use crate::AffineKind::{BrightnessContrast as B, GeglLevels as L};
    let (nl, linear) = parts();
    for sample in [SampleType::F32, SampleType::F64] {
        for order in [[B, L], [L, B]] {
            let mut f = ordered(sample, &[&[0, 0], &[]], true);
            for (n, k) in f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.iter_mut().zip(order) {
                set_affine(n, k);
            }
            let (o, _, _, _) = adopt(f);
            let c = Context::new(&o, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            for x in 0u32..5 {
                let p = input(0, x, 0, sample);
                let levels_then_bc = affine(B, bridge(affine(L, p)));
                let bc_then_levels = affine(L, bridge(affine(B, p)));
                assert_ne!(levels_then_bc[..3], bc_then_levels[..3]);
                let expected = if order == [B, L] { levels_then_bc } else { bc_then_levels };
                assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
        for enabled in [true, false] {
            let mut f = ordered(sample, &[&[0, 0, 0], &[]], true);
            let nodes = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
            set_affine(&mut nodes[0], B);
            nodes[0].opacity_f64_bits = 0.375f64.to_bits();
            set_affine(&mut nodes[1], L);
            nodes[1].opacity_f64_bits = (-0f64).to_bits();
            nodes[1].enabled = enabled;
            set_kind(&mut nodes[2], 3);
            let (o, _, _, _) = current(f);
            let c = Context::new(&o, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            for x in 0u32..5 {
                let p = input(0, x, 0, sample);
                let exposure = old_leaf(3, p, false, &nl, &linear);
                let before = if enabled {
                    let p = bridge(exposure);
                    let _full_leaf = affine(L, p);
                    bridge(p.map(|v| f64::from(v as f32)))
                } else {
                    bridge(exposure)
                };
                let expected = mix_reference(before, affine(B, before), 0.375);
                assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
    }
}
#[test]
#[ignore = "UNRUN oldsingleton epoch/invalidtuple/schema finalrefusal, inactivephysicalproof andnativeunit retained, realWork/cancel/coldprovider+sameq cleanup forbothkindspreparedholders"]
fn strict_roles_and_inactive_control_proof() {
    for fault in 0..5 {
        let mut f = affine_fixture(SampleType::F64, crate::AffineKind::GeglLevels, 0.375, Size::new(8, 6));
        let n = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        match fault {
            0 => {
                n.operation.schema_epoch = "xcf-22-stock-gegl-levels-precision-v1".into();
                n.composition.as_mut().unwrap().compatibility_epoch = n.operation.schema_epoch.clone();
            }
            1 => {
                n.operation.stored_version = "1:0".into();
            }
            2 => {
                n.arguments = vec![number("in-low", 1.), number("in-high", 0.), number("out-low", 0.), number("out-high", 1.)];
            }
            3 => {
                n.arguments.pop();
            }
            _ => {
                n.arguments.push(number("unknown", 0.));
            }
        }
        groups(&mut f);
        let q = f.quota.clone();
        let before = q.usage().unwrap();
        let reads = f.reads.load(Ordering::Relaxed);
        let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
        assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(f.reads.load(Ordering::Relaxed), reads);
    }
    for fault in 0..4 {
        let mut f = affine_fixture(SampleType::F64, crate::AffineKind::BrightnessContrast, -0., Size::new(8, 6));
        if fault == 3 {
            f = fixture(SampleType::F64, Size::new(8, 6), &[Some(0), None], Some(0));
            let index = f.quota.reserve(QuotaKind::Index, 8 << 20).unwrap();
            let resident = f.quota.reserve(QuotaKind::Resident, 8 << 20).unwrap();
            f.retention = Arc::new(Extra { _prior: f.retention, _index: index, _resident: resident });
            let node = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
            set_affine(node, crate::AffineKind::BrightnessContrast);
            node.opacity_f64_bits = (-0f64).to_bits();
        }
        f.document.layers[0].visible = false;
        f.document.layers[0].opacity = 0.;
        let id = f.document.layers[0].id;
        let state = f.document.xcf_state.as_mut().unwrap();
        state.items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
        state.filter_stacks[0].nodes[0].enabled = false;
        match fault {
            0 => {
                state.filter_masks[0].surface.try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes()).unwrap();
            }
            1 => {
                f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) };
            }
            2 => {
                state.filter_stacks[0].nodes[0].source_region = Some(1);
            }
            _ => {}
        }
        groups(&mut f);
        let q = f.quota.clone();
        let before = q.usage().unwrap();
        let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
        assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    for kind in [crate::AffineKind::BrightnessContrast, crate::AffineKind::GeglLevels] {
        let (o, rasters, fails, _) = current(affine_fixture(SampleType::F64, kind, 0.375, Size::new(8, 6)));
        let q = o.quota().clone();
        let c = Context::new(&o, Default::default(), &ctl()).unwrap();
        c.validate_operative(&ctl()).unwrap();
        let r = Rect::new(0, 0, 2, 1);
        drop(c.render_band(r, &ctl()).unwrap());
        let baseline = q.usage().unwrap();
        let total = AtomicU64::new(0);
        let count = |n| {
            total.fetch_add(n, Ordering::Relaxed);
            Ok(())
        };
        drop(c.render_band(r, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap());
        let limit = total.load(Ordering::Relaxed) - 1;
        let used = AtomicU64::new(0);
        let deny = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > limit { Err(BackingError::Work) } else { Ok(()) };
        assert!(matches!(c.render_band(r, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }), Err(E::Backing(BackingError::Work))));
        assert_eq!(q.usage().unwrap(), baseline);
        let polls = AtomicU64::new(0);
        let observe = || {
            polls.fetch_add(1, Ordering::Relaxed);
            false
        };
        let i = Interrupt::cancel_only(&observe);
        drop(c.render_band(r, &ReadControl { interrupt: &i, work: &|_| Ok(()) }).unwrap());
        let accepted = polls.load(Ordering::Relaxed);
        let polls = AtomicU64::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= accepted - 1;
        let i = Interrupt::cancel_only(&cancel);
        assert!(matches!(c.render_band(r, &ReadControl { interrupt: &i, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
        assert_eq!(q.usage().unwrap(), baseline);
        rasters[1].clear_cache().unwrap();
        let before = q.usage().unwrap();
        fails[1].store(true, Ordering::Relaxed);
        assert!(matches!(c.render_band(r, &ctl()), Err(E::Backing(BackingError::Io))));
        assert_eq!(q.usage().unwrap(), before);
        fails[1].store(false, Ordering::Relaxed);
    }
}
