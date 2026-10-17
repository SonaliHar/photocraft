//! UNRUN synthetic CURRENT invert roots; independent declared-math references.
//! Paid synthetic Source contracts remain distinct from original-XCF IO evidence.
//! No producer pixel/format qualification follows.
use super::*;
fn set_invert(n: &mut photocraft_doc::xcf_filters::FilterNode, kind: crate::InvertKind) {
    n.operation = OperationIdentity { name: kind.operation_name().into(), stored_version: kind.version().into(), schema_epoch: kind.epoch().into() };
    n.arguments.clear();
    n.composition.as_mut().unwrap().compatibility_epoch = kind.epoch().into();
}
fn inverted(sample: SampleType, kinds: &[crate::InvertKind], enabled: bool) -> Fixture {
    let placeholders: &[u8] = match kinds.len() {
        1 => &[0],
        2 => &[0, 0],
        3 => &[0, 0, 0],
        _ => panic!("closed fixture"),
    };
    let mut f = ordered(sample, &[placeholders, &[]], enabled);
    // The inherited prospective Index8MiB/Resident8MiB model envelope precedes
    // these bounded metadata replacements; actual native stores are paid too.
    for (n, kind) in f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.iter_mut().zip(kinds) {
        set_invert(n, *kind);
    }
    f
}
fn current(f: Fixture) -> (AccountedDocument, Vec<BackedRaster>, Vec<Arc<AtomicBool>>, Arc<AtomicU64>) {
    let o = AccountedDocument::new_readonly_filtered_flat_point_current_storage(f.document, f.retention, f.quota, &ctl()).unwrap();
    (o, f.rasters, f.fails, f.reads)
}
fn invert(kind: crate::InvertKind, p: [f64; 4]) -> [f64; 4] {
    let curve = photocraft_cms::curve::srgb_trc();
    let mut out = p;
    for i in 0..3 {
        out[i] = match kind {
            crate::InvertKind::Linear => curve.eval_inverse64(1.0 - curve.eval64(p[i])),
            crate::InvertKind::Gamma => 1.0 - p[i],
        };
    }
    out
}
#[test]
#[ignore = "UNRUN distinct twoinvert reversecanonical noncommuting references plus alloldleaf neighbors; enabled NLFloat bridge, rawwide alpha and Normal composition; no producer oracle"]
fn actual_order_preserves_distinct_working_interpretations() {
    use crate::InvertKind::{Gamma, Linear};
    let (nl, linear) = parts(); // External public reference allocations, not receipts.
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        let (a, _, _, _) = current(inverted(sample, &[Linear, Gamma], true));
        let (b, _, _, _) = current(inverted(sample, &[Gamma, Linear], true));
        let ca = Context::new(&a, Default::default(), &ctl()).unwrap();
        let cb = Context::new(&b, Default::default(), &ctl()).unwrap();
        ca.validate_operative(&ctl()).unwrap();
        cb.validate_operative(&ctl()).unwrap();
        for x in [0u32, 2, 4, 7] {
            let p = input(0, x, 0, sample);
            // Named nested equations are independent of container traversal.
            let gamma_then_linear = invert(Linear, bridge(invert(Gamma, p)));
            let linear_then_gamma = invert(Gamma, bridge(invert(Linear, p)));
            assert_eq!(root_value(&ca, 0, x as i32, 0).map(f64::to_bits), gamma_then_linear.map(f64::to_bits));
            assert_eq!(root_value(&cb, 0, x as i32, 0).map(f64::to_bits), linear_then_gamma.map(f64::to_bits));
            assert_ne!(gamma_then_linear[0].to_bits(), linear_then_gamma[0].to_bits());
            let expected = working
                .normal(working.normal([0.; 4], gamma_then_linear, 1., GimpOperator::Union).unwrap(), input(1, x, 0, sample), 0.375, GimpOperator::Union)
                .unwrap();
            assert_eq!(ca.render_band(Rect::new(x as i32, 0, x as i32 + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
        }
        for kind in [Linear, Gamma] {
            for old in 0..4 {
                let mut f = inverted(sample, &[kind, kind], true);
                let node = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[1];
                if old == 3 {
                    node.operation =
                        OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: crate::EXPOSURE_MIXED_EPOCH.into() };
                    node.arguments = vec![number("exposure", 0.5), number("black-level", 0.0625)];
                    node.composition.as_mut().unwrap().compatibility_epoch = crate::EXPOSURE_MIXED_EPOCH.into();
                } else {
                    configure(node, old);
                }
                let (o, _, _, _) = current(f);
                let c = Context::new(&o, Default::default(), &ctl()).unwrap();
                c.validate_operative(&ctl()).unwrap();
                let p = input(0, 2, 0, sample);
                let prior = if old == 3 {
                    {
                        let curve = photocraft_cms::curve::srgb_trc();
                        let mut v = p;
                        for i in 0..3 {
                            v[i] = curve.eval_inverse64((curve.eval64(p[i]) - 0.0625) / ((-0.5f64).exp2() - 0.0625));
                        }
                        v
                    }
                } else {
                    step(old, p, false, &nl, &linear)
                };
                assert_eq!(root_value(&c, 0, 2, 0).map(f64::to_bits), invert(kind, bridge(prior)).map(f64::to_bits));
            }
        }
    }
    // Disabled nodes before and after the sole enabled kernel never trigger a
    // Float bridge or a final cast, even when the incoming source is wide F64.
    let mut f = inverted(SampleType::F64, &[Gamma, Linear, Gamma], true);
    let nodes = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
    nodes[0].enabled = false;
    nodes[2].enabled = false;
    let (o, _, _, _) = current(f);
    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
    c.validate_operative(&ctl()).unwrap();
    let p = input(0, 4, 0, SampleType::F64);
    assert_eq!(root_value(&c, 0, 4, 0).map(f64::to_bits), invert(Linear, p).map(f64::to_bits));
    let (o, _, _, _) = current(inverted(SampleType::F64, &[Gamma, Gamma], true));
    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
    let p = input(0, 4, 0, SampleType::F64);
    let expected = invert(Gamma, bridge(invert(Gamma, p)));
    assert_eq!(root_value(&c, 0, 4, 0).map(f64::to_bits), expected.map(f64::to_bits));
    assert_ne!(expected[1].to_bits(), p[1].to_bits()); // No double-invert cancellation.
}
#[test]
#[ignore = "UNRUN paid soleinvert CURRENT route, paid Source, disabled exactwide bits and full inactive native/profile/mask/zeroarg/version/epoch refusal"]
fn sole_current_source_and_inactive_proof_keep_closed_roles() {
    use crate::InvertKind::{Gamma, Linear};
    for sample in [SampleType::F32, SampleType::F64] {
        for kind in [Linear, Gamma] {
            for enabled in [true, false] {
                let (o, _, _, _) = current(inverted(sample, &[kind], enabled));
                let c = Context::new(&o, Default::default(), &ctl()).unwrap();
                c.validate_operative(&ctl()).unwrap();
                for x in [0u32, 2, 4, 7] {
                    let p = input(0, x, 0, sample);
                    let expected = if enabled { invert(kind, p) } else { p };
                    assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
                    assert_eq!(expected[3].to_bits(), p[3].to_bits());
                }
                let q = o.quota().clone();
                let before = q.usage().unwrap();
                let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
                let paid = LeasedFilteredFlatPointPipeline::new_budgeted(o.document(), &Interrupt::NONE, &mut b, o.clone()).unwrap();
                assert!(q.usage().unwrap().resident > before.resident);
                drop(paid);
                assert_eq!(q.usage().unwrap(), before);
            }
        }
    }
    // Real paid 130x65 native stores exercise checked origin geometry. This is
    // a synthetic contract, separate from independently reviewed original XCFs.
    for sample in [SampleType::F32, SampleType::F64] {
        for kind in [Linear, Gamma] {
            let mut f = fixture(sample, Size::new(130, 65), &[Some(0), None], None);
            set_invert(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], kind);
            let (o, _, _, _) = current(f);
            let c = Context::new(&o, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            let q = o.quota().clone();
            let before = q.usage().unwrap();
            let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
            let paid = LeasedFilteredFlatPointPipeline::new_budgeted(o.document(), &Interrupt::NONE, &mut b, o.clone()).unwrap();
            assert!(q.usage().unwrap().resident > before.resident);
            drop(paid);
            assert_eq!(q.usage().unwrap(), before);
        }
    }
    for fault in 0..3 {
        let mut f = inverted(SampleType::F64, &[Linear], true);
        let id = f.document.layers[0].id;
        let state = f.document.xcf_state.as_mut().unwrap();
        match fault {
            0 => state.filter_stacks[0].nodes[0].operation.schema_epoch = crate::INVERT_GAMMA_MIXED_EPOCH.into(),
            1 => state.items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().raster_bounds = Some(Rect::new(1, 0, 9, 6)),
            _ => state.filter_stacks[0].nodes[0].composition.as_mut().unwrap().source_mode = 28,
        }
        let before = f.quota.usage().unwrap();
        let mut b = Budget { q: f.quota.clone(), used: 0, limit: u64::MAX };
        assert!(LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
        assert_eq!(f.quota.usage().unwrap(), before);
    }
    for fault in 0..6 {
        let mut f = if fault == 5 {
            let mut f = fixture(SampleType::F64, Size::new(8, 6), &[Some(0), None], Some(0));
            set_invert(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], Linear);
            f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
            f
        } else {
            inverted(SampleType::F64, &[Linear], false)
        };
        f.document.layers[0].visible = false;
        f.document.layers[0].opacity = 0.;
        let id = f.document.layers[0].id;
        let state = f.document.xcf_state.as_mut().unwrap();
        state.items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
        match fault {
            0 => state.filter_stacks[0].nodes[0].arguments.push(number("unexpected", 0.)),
            1 => state.filter_stacks[0].nodes[0].operation.stored_version = "1:0".into(),
            2 => state.filter_stacks[0].nodes[0].composition.as_mut().unwrap().compatibility_epoch = crate::INVERT_GAMMA_MIXED_EPOCH.into(),
            3 => {
                state.filter_masks[0].surface.try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes()).unwrap();
            }
            4 => f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            _ => {} // Actual provider tail NaN was generated under its prospective fee.
        }
        if let Ok(o) = AccountedDocument::new_readonly_filtered_flat_point_current_storage(f.document, f.retention, f.quota, &ctl()) {
            if fault == 5 {
                let c = Context::new(&o, Default::default(), &ctl()).unwrap();
                assert!(c.validate_operative(&ctl()).is_err());
            } else {
                assert!(Context::new(&o, Default::default(), &ctl()).is_err());
            }
        }
    }
}
#[test]
#[ignore = "UNRUN cumulative CURRENT preparation and operative Work/cancel, samequota retainedstage cleanup, cold actual lastroot provider failure; same Source topology and proof"]
fn current_preparation_and_runtime_keep_the_original_control() {
    use crate::InvertKind::{Gamma, Linear};
    let (o, rasters, fails, _) = current(inverted(SampleType::F64, &[Linear, Gamma], true));
    let q = o.quota().clone();
    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
    c.validate_operative(&ctl()).unwrap();
    let baseline = q.usage().unwrap();
    let used = AtomicU64::new(0);
    let count = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    c.validate_operative(&ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    let total = used.load(Ordering::Relaxed);
    assert!(total > 1);
    let used = AtomicU64::new(0);
    let deny = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) >= total { Err(BackingError::Work) } else { Ok(()) };
    assert!(matches!(c.validate_operative(&ReadControl { interrupt: &Interrupt::NONE, work: &deny }), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let polls = AtomicU64::new(0);
    let count = || {
        polls.fetch_add(1, Ordering::Relaxed);
        false
    };
    let i = Interrupt::cancel_only(&count);
    c.validate_operative(&ReadControl { interrupt: &i, work: &|_| Ok(()) }).unwrap();
    let total = polls.load(Ordering::Relaxed);
    assert!(total > 1);
    let polls = AtomicU64::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= total - 1;
    let i = Interrupt::cancel_only(&cancel);
    assert!(matches!(c.validate_operative(&ReadControl { interrupt: &i, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), baseline);
    rasters[1].clear_cache().unwrap();
    fails[1].store(true, Ordering::Relaxed);
    let before = q.usage().unwrap();
    assert!(c.render_band(Rect::new(0, 0, 1, 1), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), before);
    fails[1].store(false, Ordering::Relaxed);
    drop(c);
    let before = q.usage().unwrap();
    let used = AtomicU64::new(0);
    let count = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let prepared = Context::new(&o, Default::default(), &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    let work = used.load(Ordering::Relaxed);
    assert!(q.usage().unwrap().resident > before.resident);
    drop(prepared);
    assert_eq!(q.usage().unwrap(), before);
    let used = AtomicU64::new(0);
    let deny = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) >= work { Err(BackingError::Work) } else { Ok(()) };
    assert!(Context::new(&o, Default::default(), &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
