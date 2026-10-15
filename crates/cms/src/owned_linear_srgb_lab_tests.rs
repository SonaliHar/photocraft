//! Ignored CMS owner-contract drafts, not GIMP or outer-quota acceptance.
//! Builtins are fixed process-global inputs; caller receipts belong to L3.
use super::*;
use std::cell::Cell;

#[test]
#[ignore = "UNRUN specialized actual LinearSrgb physical Lab directions, finite HDR, distinct NL basis; no producer oracle"]
fn physical_coordinates_use_the_explicit_linear_basis_without_clamping() {
    let limits = Limits::default();
    let parts = construct_with_control(limits, &|| false).unwrap();
    let basis = Builtin::LinearSrgb.profile();
    let matrix = basis.matrix.unwrap();
    let inverse = crate::math::invert(&matrix).unwrap();
    for input in [[0.125, 0.5, 0.875], [0., 0., 0.], [1., 1., 1.], [-0.125, 0.5, 1.25]] {
        let expected = crate::math::xyz_to_lab(crate::math::apply(&matrix, input), crate::math::D50);
        let mut lab = [0.; 3];
        parts.physical_forward.try_eval_f64(&input, &mut lab).unwrap();
        for (a, b) in lab.into_iter().zip(expected) {
            assert!((a - b).abs() < 1e-10);
        }
        let expected = crate::math::apply(&inverse, crate::math::lab_to_xyz(lab, crate::math::D50));
        let mut back = [0.; 3];
        parts.physical_reverse.try_eval_f64(&lab, &mut back).unwrap();
        for (a, b) in back.into_iter().zip(expected) {
            assert!((a - b).abs() < 1e-10);
        }
        if input[0] < 0. {
            assert!(back[0] < 0. && back[2] > 1.);
        }
    }
    let nl = crate::owned_lab::construct_with_control(Builtin::Srgb.profile(), crate::xcf_color::Encoding::NonlinearProfile, limits, &|| false).unwrap();
    let mut linear_lab = [0.; 3];
    let mut nonlinear_lab = [0.; 3];
    parts.physical_forward.try_eval_f64(&[0.5; 3], &mut linear_lab).unwrap();
    nl.physical_forward.try_eval_f64(&[0.5; 3], &mut nonlinear_lab).unwrap();
    assert!((linear_lab[0] - nonlinear_lab[0]).abs() > 10.);
}

#[test]
#[ignore = "UNRUN prospective full owner peak/work facts, lowered limits and unchanged composite Parts work-field meaning"]
fn complete_peak_and_work_are_facts_for_all_retained_parts() {
    const CEILING: u64 = planning_work_ceiling();
    assert!(CEILING > 0); // Const evaluation cannot initialize builtin profiles.
    let r = requirements_with_control(Limits::default(), &|| false).unwrap();
    let exact = Limits { resident_bytes: r.peak_resident_bytes, work_units: r.work_units };
    let parts = construct_with_control(exact, &|| false).unwrap();
    let retained = parts.retained_bytes_with_control(exact, &|| false).unwrap();
    assert!(retained <= r.peak_resident_bytes && r.work_units > CEILING);
    let physical = parts.physical_forward.retained_bytes_with_control(exact, &|| false).unwrap()
        + parts.physical_reverse.retained_bytes_with_control(exact, &|| false).unwrap();
    assert!(retained > physical); // Normalized pair remains an actual owner too.
    let forward = parts.physical_forward.evaluation_work_units_with_control(exact, &|| false).unwrap();
    let reverse = parts.physical_reverse.evaluation_work_units_with_control(exact, &|| false).unwrap();
    let transfer: u64 = Builtin::LinearSrgb.profile().trc.as_ref().unwrap().iter().map(|c| c.controlled_work().max(128)).sum();
    assert_eq!(parts.per_pixel_work_units, forward * 2 + reverse + transfer * 3);
    for limits in [
        Limits { resident_bytes: r.peak_resident_bytes - 1, ..exact },
        Limits { work_units: r.work_units - 1, ..exact },
        Limits { resident_bytes: 0, work_units: 0 },
    ] {
        assert!(matches!(requirements_with_control(limits, &|| false), Err(Error::Limit(_))));
        assert!(matches!(construct_with_control(limits, &|| false), Err(Error::Limit(_))));
    }
}

#[test]
#[ignore = "UNRUN early/mid/final cooperative cancellation exposes no partial Parts; input builtins stay exact"]
fn cancellation_keeps_fixed_profile_inputs_immutable() {
    let basis = Builtin::LinearSrgb.profile();
    let encoded = basis.encoded_bytes().unwrap().to_vec(); // External test harness.
    assert!(matches!(requirements_with_control(Limits::default(), &|| true), Err(Error::Cancelled)));
    let calls = Cell::new(0usize);
    let parts = construct_with_control(Limits::default(), &|| {
        calls.set(calls.get() + 1);
        false
    })
    .unwrap();
    let total = calls.get();
    drop(parts);
    assert!(total > 8);
    for stop in [1, total / 2, total] {
        let calls = Cell::new(0usize);
        assert!(matches!(
            construct_with_control(Limits::default(), &|| {
                calls.set(calls.get() + 1);
                calls.get() >= stop
            }),
            Err(Error::Cancelled)
        ));
        assert_eq!(basis.encoded_bytes().unwrap(), encoded);
    }
}
