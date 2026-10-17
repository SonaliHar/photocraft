//! UNRUN synthetic mixed Exposure declared policy; no GIMP projection oracle.
use super::*;
fn set_exposure(n: &mut photocraft_doc::xcf_filters::FilterNode, stops: f32, black: f32) {
    n.operation = OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: crate::EXPOSURE_MIXED_EPOCH.into() };
    n.arguments = vec![number("exposure", stops), number("black-level", black)];
    n.composition.as_mut().unwrap().compatibility_epoch = crate::EXPOSURE_MIXED_EPOCH.into();
}
fn exposed(sample: SampleType, sequence: &[u8], enabled: bool) -> Fixture {
    let mut f = match sequence.len() {
        1 => ordered(sample, &[&[0], &[]], enabled),
        2 => ordered(sample, &[&[0, 0], &[]], enabled),
        4 => ordered(sample, &[&[0, 0, 0, 0], &[]], enabled),
        _ => panic!("closed fixture"),
    };
    // The inherited real Index8MiB/Resident8MiB holders precede all new arguments,
    // names and literals. Native store payloads remain independently paid.
    for (n, kind) in f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.iter_mut().zip(sequence) {
        if *kind == 3 {
            set_exposure(n, 0.5, 0.0625)
        } else {
            configure(n, *kind)
        }
        n.enabled = enabled;
    }
    f
}
fn exposure(p: [f64; 4]) -> [f64; 4] {
    let curve = photocraft_cms::curve::srgb_trc();
    let mut out = p;
    for i in 0..3 {
        out[i] = curve.eval_inverse64((curve.eval64(p[i]) - 0.0625) / ((-0.5f64).exp2() - 0.0625));
    }
    out
}
#[test]
#[ignore = "UNRUN noncommuting Exposure before/after all three existing leaves and fixed fourstage sequences, real ordered root/Normal reference and finite derivedHDR; no source/numeric acceptance"]
fn actual_ordered_exposure_is_not_a_family_batch() {
    let (nl, linear) = parts();
    let w = WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
        .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        for kind in 0..3 {
            let (a, _, _, _) = adopt(exposed(sample, &[3, kind], true));
            let (b, _, _, _) = adopt(exposed(sample, &[kind, 3], true));
            let ca = Context::new(&a, Default::default(), &ctl()).unwrap();
            let cb = Context::new(&b, Default::default(), &ctl()).unwrap();
            ca.validate_operative(&ctl()).unwrap();
            cb.validate_operative(&ctl()).unwrap();
            for x in [0u32, 2, 4, 7] {
                let p = input(0, x, 0, sample);
                let expected_a = exposure(bridge(step(kind, p, false, &nl, &linear)));
                let expected_b = step(kind, bridge(exposure(p)), true, &nl, &linear);
                assert_eq!(root_value(&ca, 0, x as i32, 0).map(f64::to_bits), expected_a.map(f64::to_bits));
                assert_eq!(root_value(&cb, 0, x as i32, 0).map(f64::to_bits), expected_b.map(f64::to_bits));
                if x == 2 {
                    assert_ne!(expected_a.map(f64::to_bits), expected_b.map(f64::to_bits));
                }
                let expected =
                    w.normal(w.normal([0.; 4], expected_a, 1., GimpOperator::Union).unwrap(), input(1, x, 0, sample), 0.375, GimpOperator::Union).unwrap();
                assert_eq!(ca.render_band(Rect::new(x as i32, 0, x as i32 + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
        let (o, _, _, _) = adopt(exposed(sample, &[3, 0, 1, 2], true));
        let c = Context::new(&o, Default::default(), &ctl()).unwrap();
        c.validate_operative(&ctl()).unwrap();
        let p = input(0, 2, 0, sample);
        let expected = exposure(bridge(step(0, bridge(step(1, bridge(step(2, p, false, &nl, &linear)), true, &nl, &linear)), true, &nl, &linear)));
        assert_eq!(root_value(&c, 0, 2, 0).map(f64::to_bits), expected.map(f64::to_bits));
    }
}
#[test]
#[ignore = "UNRUN paid sole Exposure without oldsingleton fallback, exact disabled/neutral wide bits, source/current130x65 origin and canonical/native bounds boundary and full inactive envelope/profile/mask proof"]
fn sole_leaf_and_closed_roles_keep_their_own_policy() {
    for sample in [SampleType::F32, SampleType::F64] {
        for mode in 0..3 {
            let mut f = exposed(sample, &[3], mode != 1);
            if mode == 2 {
                set_exposure(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], -0., 0.);
            }
            let (o, _, _, _) = adopt(f);
            let c = Context::new(&o, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            for x in [0u32, 2, 4, 7] {
                let p = input(0, x, 0, sample);
                let expected = if mode == 0 { exposure(p) } else { p };
                assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
                assert_eq!(expected[3].to_bits(), p[3].to_bits());
            }
        }
    }
    let mut f = fixture(SampleType::F64, Size::new(130, 65), &[Some(0), None], None);
    set_exposure(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], 0.5, 0.0625);
    let o = AccountedDocument::new_readonly_filtered_flat_point_current_storage(f.document, f.retention, f.quota, &ctl()).unwrap();
    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
    c.validate_operative(&ctl()).unwrap();
    // Genuine paid 130x65 native stores; synthetic mathematical fixture, not
    // a substitute for the independently captured original-XCF IO evidence.
    let before = o.quota().usage().unwrap();
    let mut b = Budget { q: o.quota().clone(), used: 0, limit: u64::MAX };
    let prepared = LeasedFilteredFlatPointPipeline::new_budgeted(o.document(), &Interrupt::NONE, &mut b, o.clone()).unwrap();
    assert!(o.quota().usage().unwrap().resident > before.resident);
    drop(prepared);
    assert_eq!(o.quota().usage().unwrap(), before);
    let mut invalid = fixture(SampleType::F64, Size::new(130, 65), &[Some(0), None], None);
    set_exposure(&mut invalid.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], 0.5, 0.0625);
    let id = invalid.document.layers[0].id;
    invalid.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().raster_bounds =
        Some(Rect::new(1, 0, 131, 65));
    let mut b = Budget { q: invalid.quota.clone(), used: 0, limit: u64::MAX };
    assert!(LeasedFilteredFlatPointPipeline::new_budgeted(&invalid.document, &Interrupt::NONE, &mut b, invalid.retention.clone()).is_err());
    for fault in 0..5 {
        let mut f = exposed(SampleType::F64, &[3], false);
        f.document.layers[0].visible = false;
        f.document.layers[0].opacity = 0.;
        let id = f.document.layers[0].id;
        let s = f.document.xcf_state.as_mut().unwrap();
        s.items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().visible = false;
        match fault {
            0 => s.filter_stacks[0].nodes[0].operation.stored_version = "0:0".into(),
            1 => s.filter_stacks[0].nodes[0].arguments[1] = number("black-level", 0.1),
            2 => s.filter_stacks[0].nodes[0].composition.as_mut().unwrap().compatibility_epoch = "foreign".into(),
            3 => {
                s.filter_masks[0].surface.try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes()).unwrap();
            }
            _ => f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
        }
        if let Ok(o) = AccountedDocument::new_readonly_filtered_flat_point_current_storage(f.document, f.retention, f.quota, &ctl()) {
            assert!(Context::new(&o, Default::default(), &ctl()).is_err());
        }
    }
}
#[test]
#[ignore = "UNRUN every prepared Space/Parts stage and same-ledger current/source fees, cumulative lateWork/cancel and actual finalroot provider rollback without duplicate Exposure scratch charge"]
fn whole_root_custody_and_control() {
    let (o, rasters, fails, _) = adopt(exposed(SampleType::F64, &[3, 0, 1, 2], true));
    let q = o.quota().clone();
    let before = q.usage().unwrap();
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    let p = LeasedFilteredFlatPointPipeline::new_budgeted(o.document(), &Interrupt::NONE, &mut b, o.clone()).unwrap();
    assert!(q.usage().unwrap().resident > before.resident);
    let work = b.used;
    drop(p);
    assert_eq!(q.usage().unwrap(), before);
    let mut b = Budget { q: q.clone(), used: 0, limit: work - 1 };
    assert!(LeasedFilteredFlatPointPipeline::new_budgeted(o.document(), &Interrupt::NONE, &mut b, o.clone()).is_err());
    assert_eq!(q.usage().unwrap(), before);
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
    let baseline = q.usage().unwrap();
    assert!(c.render_band(Rect::new(0, 0, 1, 1), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    fails[1].store(false, Ordering::Relaxed);
}
