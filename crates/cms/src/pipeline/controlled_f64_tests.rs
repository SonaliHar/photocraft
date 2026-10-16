//! Ignored UNRUN/uncompiled controlled F64 contracts. Legacy evaluator is the
//! independent numeric reference; no external producer or perf acceptance.
use super::*;
use std::cell::Cell;
fn compare(p: &Pipeline, input: &[f64]) {
    let mut old = vec![0.; p.outputs];
    p.try_eval_f64(input, &mut old).unwrap();
    let req = p.eval_f64_requirements_with_control(Limits::default(), &|| false).unwrap();
    assert!(req.planning_work_units <= Pipeline::eval_f64_planning_work_ceiling());
    let mut controlled = vec![91.; p.outputs];
    p.try_eval_f64_with_control(input, &mut controlled, Limits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units }, &|| false).unwrap();
    assert_eq!(old.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), controlled.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
}
#[test]
#[ignore = "UNRUN/uncompiled: inverse table plateau edge walks poll original dynamic cancel after bounded planning; output atomic and legacy bits retained"]
fn inverse_plateau_has_dynamic_inner_cancellation() {
    for curve in [Curve::Table(vec![0.; 65536]), Curve::Table64(vec![0.; 65536])] {
        let p = Pipeline::new(1, vec![Stage::InvCurves(vec![curve])]);
        compare(&p, &[0.]);
        let req = p.eval_f64_requirements_with_control(Limits::default(), &|| false).unwrap();
        assert_eq!(req.work_units, req.planning_work_units + 64 + 65536 + 128 + 1);
        let calls = Cell::new(0usize);
        let cancel = || {
            let n = calls.get() + 1;
            calls.set(n);
            n > 32
        };
        let mut out = [91.];
        assert!(matches!(
            p.try_eval_f64_with_control(&[0.], &mut out, Limits { resident_bytes: 4096, work_units: req.work_units }, &cancel),
            Err(Error::Cancelled)
        ));
        assert_eq!(out, [91.]);
        assert_eq!(calls.get(), 33);
        // No cancellation in a separate invocation cannot consume an earlier budget.
        let mut full = [91.];
        p.try_eval_f64_with_control(&[0.], &mut full, Limits { resident_bytes: 4096, work_units: req.work_units }, &|| false).unwrap();
        assert_eq!(full[0].to_bits(), 1f64.to_bits());
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: prospective exact wide schedule, lower work/scratch, malformed/nonfinite/overflow and final output remain atomic"]
fn lower_control_and_invalid_samples_do_not_publish() {
    let p = Pipeline::new(3, vec![Stage::Matrix { rows: 3, cols: 3, m: vec![1., 0., 0., 0., 2., 0., 0., 0., 3.], offset: vec![0.; 3] }]);
    let req = p.eval_f64_requirements_with_control(Limits::default(), &|| false).unwrap();
    assert_eq!(req.work_units, req.planning_work_units + 64 + 9 + 128 + 1);
    for limits in [Limits { resident_bytes: 4095, work_units: req.work_units }, Limits { resident_bytes: 4096, work_units: req.work_units - 1 }] {
        let mut out = [91.; 3];
        assert!(p.try_eval_f64_with_control(&[0.25; 3], &mut out, limits, &|| false).is_err());
        assert_eq!(out, [91.; 3]);
    }
    for input in [vec![f64::NAN, 0., 0.], vec![f64::INFINITY, 0., 0.], vec![0.; 2], vec![0., f64::MAX, 0.]] {
        let mut out = [91.; 3];
        assert!(p.try_eval_f64_with_control(&input, &mut out, Limits::default(), &|| false).is_err());
        assert_eq!(out, [91.; 3]);
    }
    let mut wrong = [91.; 2];
    assert!(p.try_eval_f64_with_control(&[0.; 3], &mut wrong, Limits::default(), &|| false).is_err());
    assert_eq!(wrong, [91.; 2]);
    let bad = Pipeline { inputs: 3, outputs: 3, stages: vec![Stage::Curves(vec![Curve::Identity; 2])] };
    let mut out = [91.; 3];
    assert!(bad.try_eval_f64_with_control(&[0.; 3], &mut out, Limits::default(), &|| false).is_err());
    assert_eq!(out, [91.; 3]);
    let too_many = Pipeline { inputs: 3, outputs: 3, stages: vec![Stage::Clamp01; 4097] };
    assert!(too_many.eval_f64_requirements_with_control(Limits::default(), &|| false).is_err());
    assert!(p.eval_f64_requirements_with_control(Limits::default(), &|| true).is_err());
}
#[test]
#[ignore = "UNRUN/uncompiled: legacy bit-exact wide curves/matrix/PCS and CLUT corner schedules plus Transform forwarding without F32 narrowing"]
fn exact_legacy_wide_math_and_transform_forwarding() {
    let identity = Pipeline::new(3, vec![]);
    compare(&identity, &[-0., f64::from_bits(1), 0.5 + 2f64.powi(-44)]);
    for curve in [
        Curve::Identity,
        Curve::Gamma(2.2),
        Curve::Parametric { kind: 4, p: [2.4, 1. / 1.055, 0.055 / 1.055, 1. / 12.92, 0.04045, 0., 0.] },
        Curve::Table(vec![0., 0., 0.25, 0.75, 1., 1.]),
        Curve::Table64(vec![1., 1., 0.75, 0.25, 0., 0.]),
    ] {
        for inverse in [false, true] {
            let stage = if inverse { Stage::InvCurves(vec![curve.clone()]) } else { Stage::Curves(vec![curve.clone()]) };
            let p = Pipeline::new(1, vec![stage]);
            for x in [-2., -0., f64::from_bits(1), 0.25 + 2f64.powi(-45), 0.75, 1., 2.] {
                compare(&p, &[x]);
            }
        }
    }
    let pcs = Pipeline::new(
        3,
        vec![
            Stage::Matrix { rows: 3, cols: 3, m: vec![0.7, 0.2, 0.1, 0.1, 0.8, 0.1, 0.1, 0.1, 0.8], offset: vec![0.01, 0., 0.] },
            Stage::XyzToLab,
            Stage::EncodeLabV4,
            Stage::DecodeLabV4,
            Stage::LabToXyz,
        ],
    );
    compare(&pcs, &[0.5 + 2f64.powi(-44), 0.25, 0.75]);
    for channels in [1usize, 3, 4, 5] {
        let entries = (1usize << channels) * 3;
        let data = (0..entries).map(|n| (n % 17) as f32 / 16.).collect();
        let clut = Clut::new(vec![2; channels], 3, data);
        let p = Pipeline::new(channels, vec![Stage::Clut(clut)]);
        let input = (0..channels).map(|n| 0.125 + (n as f64) * 0.15).collect::<Vec<_>>();
        compare(&p, &input);
        let req = p.eval_f64_requirements_with_control(Limits::default(), &|| false).unwrap();
        let visits = match channels {
            3 => 4,
            4 => 8,
            n => 1u64 << n,
        };
        assert_eq!(req.work_units, req.planning_work_units + 64 + visits * (channels as u64 + 3 + 1) + 1);
    }
    let t = crate::Transform::from_pipeline(pcs, crate::transform::TransformOptions::default());
    let input = [0.5 + 2f64.powi(-44), 0.25, 0.75];
    let mut old = [0.; 3];
    let mut new = [91.; 3];
    t.try_eval_f64(&input, &mut old).unwrap();
    let req = t.eval_f64_requirements_with_control(Limits::default(), &|| false).unwrap();
    assert_eq!(crate::Transform::eval_f64_planning_work_ceiling(), Pipeline::eval_f64_planning_work_ceiling());
    t.try_eval_f64_with_control(&input, &mut new, Limits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units }, &|| false).unwrap();
    assert_eq!(old.map(f64::to_bits), new.map(f64::to_bits));
}
