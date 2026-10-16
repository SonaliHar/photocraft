//! Ignored UNRUN checked-HSL typed adapter contracts, no producer equivalence.
use super::*;
fn parameters(keep: [bool; 3]) -> Parameters {
    Parameters::AlienHsl(AlienHslParameters::from_words([1f32, 2.5, 0.].map(f32::to_bits), [0f32, 91., 360.].map(f32::to_bits), keep).unwrap())
}
fn args() -> Vec<FilterArgument> {
    let mut a = alien_args();
    a[0].value = FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 1 };
    a
}
fn old_unit(p: Parameters, input: [f64; 4]) -> [f64; 4] {
    let mut out = [0.; 4];
    p.operation().prepare().unwrap().try_eval_pixel(&input, 3, &mut out).unwrap();
    out
}
#[test]
#[ignore = "UNRUN separateHSLcheapword API/model1 exactten fields/order, RGB/HSL crossedmodels andepochs refuse; no tuple normalization or hiddenkernelprepare"]
fn exact_hsl_typed_identity_and_raw_words() {
    let args = args();
    let before = args.clone();
    let p = alien_map_hsl_parameters(&args).unwrap();
    assert_eq!(args, before);
    assert_eq!(p.raw_words(), ([1f32, 2.5, 0.].map(f32::to_bits), [0f32, 91., 360.].map(f32::to_bits)));
    assert_eq!(p.keep(), [false, true, false]);
    assert!(alien_map_rgb_parameters(&args).is_err());
    assert!(alien_map_hsl_parameters(&alien_args()).is_err());
    assert_ne!(Kind::AlienHsl.epoch(), Kind::AlienRgb.epoch());
    assert_eq!(Kind::AlienHsl.operation_name(), Kind::AlienRgb.operation_name());
    assert_eq!(Kind::AlienHsl.version(), "0:0");
    for fault in 0..6 {
        let mut a = args.clone();
        match fault {
            0 => a[0].value = FilterValue::Enum { domain: "foreign".into(), value: 1 },
            1 => a[1].value = FilterValue::Number(ExactNumber::Float64Bits(1f64.to_bits())),
            2 => a[7] = arg("cpn-1-keep", 1.),
            3 => a[3] = arg("cpn-3-frequency", 21.),
            4 => a[6] = arg("cpn-3-phaseshift", 361.),
            _ => a[2].name = a[1].name.clone(),
        }
        assert!(alien_map_hsl_parameters(&a).is_err());
    }
    assert!(alien_map_hsl_parameters(&args[..9]).is_err());
}
#[test]
#[ignore = "UNRUN oldunit HSL arithmetic/FloatRGB widealpha versus explicitlydifferentdefaultgraph, derivedpartialdomain/singularatomicrefusal and allkeep roundtrip notidentity"]
fn old_strict_schedule_and_checked_partial_domain() {
    let q = q();
    for keep in [[true; 3], [false; 3], [false, true, false]] {
        let p = parameters(keep);
        let h = held(&q, p);
        for alpha in [-0., 0., 0.375 + f64::EPSILON, f64::from_bits(1f64.to_bits() - 1), 1.] {
            let input = [0.125, 0.5 + 2f64.powi(-45), 0.875, alpha];
            for derived in [false, true] {
                let out = h.value.evaluate(input, true, derived, &q, &ctl()).unwrap();
                assert_eq!(out.map(f64::to_bits), old_unit(p, input).map(f64::to_bits));
                assert_eq!(out[3].to_bits(), input[3].to_bits());
            }
            assert_eq!(h.value.evaluate(input, false, false, &q, &ctl()).unwrap().map(f64::to_bits), input.map(f64::to_bits));
        }
        for input in [[-1., 0., 1., 1.], [0., 1., 2., 1.], [f64::MAX, 0., 0., -0.], [f64::from(f32::MAX), f64::from(f32::MAX), f64::from(f32::MAX), 1.]] {
            assert!(h.value.evaluate(input, true, true, &q, &ctl()).is_err());
        }
    }
    let p = Parameters::AlienHsl(AlienHslParameters::from_words([1f32.to_bits(); 3], [0; 3], [true; 3]).unwrap());
    let h = held(&q, p);
    let input = [-0.25, 0.5 + 2f64.powi(-45), 1.25, -0.];
    let out = h.value.evaluate(input, true, true, &q, &ctl()).unwrap();
    assert!(out.iter().all(|v| v.is_finite()));
    assert_eq!(out[3].to_bits(), input[3].to_bits());
    assert_ne!(out[1].to_bits(), input[1].to_bits());
    assert!(h.value.evaluate(input, true, false, &q, &ctl()).is_err());
    // Disabled keeps finite incoming wide values even outside Float range, after proof.
    let p = [f64::MAX, -f64::MAX, 0.0, -0.];
    assert_eq!(h.value.evaluate(p, false, true, &q, &ctl()).unwrap().map(f64::to_bits), p.map(f64::to_bits));
}
#[test]
#[ignore = "UNRUN independent1024HSL runtime/concurrent1024mix versusretainedprep, full4480allkeep/256disabled, sameq work/cancel/quota/dropcleanup"]
fn actual_hsl_runtime_and_work_overlap() {
    let q = q();
    let base = q.usage().unwrap();
    let p = parameters([true; 3]);
    let h = held(&q, p);
    assert_eq!(p.scratch_bytes(), 1024);
    let retained = q.usage().unwrap();
    for (enabled, expected) in [(true, 4480), (false, 256)] {
        let n = AtomicU64::new(0);
        let count = |v| {
            n.fetch_add(v, Ordering::Relaxed);
            Ok(())
        };
        h.value.evaluate([0.125, 0.5, 0.875, -0.], enabled, false, &q, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
        assert_eq!(n.load(Ordering::Relaxed), expected);
        assert_eq!(q.usage().unwrap(), retained);
        let used = AtomicU64::new(0);
        let deny = |v| if used.fetch_add(v, Ordering::Relaxed).saturating_add(v) > expected - 1 { Err(BackingError::Work) } else { Ok(()) };
        assert!(matches!(
            h.value.evaluate([0.125, 0.5, 0.875, 1.], enabled, false, &q, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }),
            Err(E::Backing(BackingError::Work))
        ));
        assert_eq!(q.usage().unwrap(), retained);
    }
    let fill = q.reserve(QuotaKind::Resident, (1 << 20) - q.usage().unwrap().resident - 768).unwrap();
    assert!(matches!(h.value.evaluate([0.125, 0.5, 0.875, 1.], true, false, &q, &ctl()), Err(E::Quota(_))));
    drop(fill);
    assert_eq!(q.usage().unwrap(), retained);
    struct MixHeld {
        value: crate::gimp_mixed_filter_opacity::PreparedMix,
        _fee: QuotaLease,
    }
    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
    let (value, fee) = crate::gimp_mixed_filter_opacity::PreparedMix::prepare(128, &Interrupt::NONE, &mut b).unwrap();
    let mix = MixHeld { value, _fee: fee };
    let before = q.usage().unwrap();
    let runtime = mix.value.reserve_runtime(&q, &ctl()).unwrap();
    let during = q.usage().unwrap();
    assert_eq!(during.resident - before.resident, 1024);
    let input = [0.125, 0.5, 0.875, 1.];
    let peak = AtomicU64::new(0);
    let observe = |_| {
        peak.fetch_max(q.usage().unwrap().resident, Ordering::Relaxed);
        Ok(())
    };
    let after = h.value.evaluate(input, true, false, &q, &ReadControl { interrupt: &Interrupt::NONE, work: &observe }).unwrap();
    assert!(peak.load(Ordering::Relaxed) >= during.resident + 1024);
    mix.value.apply(input, after, 0.375f64.to_bits(), &ctl()).unwrap();
    assert_eq!(q.usage().unwrap(), during);
    drop(runtime);
    assert_eq!(q.usage().unwrap(), before);
    drop(mix);
    assert_eq!(q.usage().unwrap(), retained);
    let cancel = || true;
    let i = Interrupt::cancel_only(&cancel);
    assert!(matches!(h.value.evaluate(input, true, false, &q, &ReadControl { interrupt: &i, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), retained);
    drop(h);
    assert_eq!(q.usage().unwrap(), base);
}
