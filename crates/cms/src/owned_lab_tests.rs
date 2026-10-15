//! UNRUN additive physical-Lab CMS contract regressions. Requirements and local
//! CMS Limits are facts/admission, not a catalog quota receipt. Vec/Profile
//! reference fixtures below are external harness data; outer same-ledger fee
//! lifetime tests belong to the compose PaidLabWorkingSpace owner.
use super::{construct_with_control, planning_work_ceiling, requirements_with_control};
use crate::{
    Builtin, Curve, Intent, Pcs,
    control::{Error, Limits},
    pipeline::{Pipeline, Stage},
    xcf_color::Encoding,
};
use std::cell::Cell;

#[test]
#[ignore = "UNRUN: prospective exact/lower/raised limits, closed basis/encoding and early/mid/final cancellation expose no partial Parts; no quota/RSS claim"]
fn whole_owner_limits_are_lowerable_and_prospective() {
    let basis = Builtin::Srgb.profile();
    let stored = Encoding::NonlinearProfile;
    let planning = planning_work_ceiling(basis, stored).unwrap();
    let r = requirements_with_control(basis, stored, Limits::default(), &|| false).unwrap();
    assert!(r.peak_resident_bytes > std::mem::size_of::<super::Parts>());
    assert!(r.work_units > planning && planning > 0);
    let exact = Limits { resident_bytes: r.peak_resident_bytes, work_units: r.work_units };
    let repeated = requirements_with_control(basis, stored, exact, &|| false).unwrap();
    assert_eq!((repeated.peak_resident_bytes, repeated.work_units), (r.peak_resident_bytes, r.work_units));
    let parts = construct_with_control(basis, stored, exact, &|| false).unwrap();
    assert!(parts.retained_bytes_with_control(exact, &|| false).unwrap() <= r.peak_resident_bytes);
    drop(parts);
    for lower in [
        Limits { resident_bytes: r.peak_resident_bytes - 1, work_units: r.work_units },
        Limits { resident_bytes: r.peak_resident_bytes, work_units: r.work_units - 1 },
        Limits { resident_bytes: 0, work_units: 0 },
    ] {
        assert!(matches!(requirements_with_control(basis, stored, lower, &|| false), Err(Error::Limit(_))));
        assert!(matches!(construct_with_control(basis, stored, lower, &|| false), Err(Error::Limit(_))));
    }
    let max = Limits::default();
    for raised in [
        Limits { resident_bytes: max.resident_bytes + 1, work_units: max.work_units },
        Limits { resident_bytes: max.resident_bytes, work_units: max.work_units + 1 },
        Limits { resident_bytes: usize::MAX, work_units: u64::MAX },
    ] {
        assert!(matches!(construct_with_control(basis, stored, raised, &|| false), Err(Error::Limit(_))));
    }
    cancellation_at_multiple_construction_boundaries_is_closed();
    unsupported_profile_shapes_and_encodings_stay_closed();
}

fn cancellation_at_multiple_construction_boundaries_is_closed() {
    let basis = Builtin::Srgb.profile();
    let encoded = basis.encoded_bytes().unwrap().to_vec();
    assert!(matches!(requirements_with_control(basis, Encoding::NonlinearProfile, Limits::default(), &|| true), Err(Error::Cancelled)));
    let calls = Cell::new(0usize);
    let parts = construct_with_control(basis, Encoding::NonlinearProfile, Limits::default(), &|| {
        calls.set(calls.get() + 1);
        false
    })
    .unwrap();
    let total = calls.get();
    assert!(total > 8);
    drop(parts);
    // The warm builtin and immutable input keep the poll schedule stable; using
    // the successful count exercises the final checkpoint without guessing a
    // particular internal phase index. These are cancellation, not timing tests.
    for allowed in [0, 1, total / 2, total - 1] {
        let count = Cell::new(0usize);
        let result = construct_with_control(basis, Encoding::NonlinearProfile, Limits::default(), &|| {
            let next = count.get();
            count.set(next + 1);
            next >= allowed
        });
        assert!(matches!(result, Err(Error::Cancelled)), "allowed polls {allowed}");
        assert!(count.get() > allowed);
        assert_eq!(basis.encoded_bytes().unwrap(), encoded);
    }
}

#[test]
#[ignore = "UNRUN: actual nested retained facts include normalized and physical pairs; per-pixel schedule covers two physical forwards plus reverse and transfer work"]
fn retained_and_evaluation_facts_cover_all_retained_parts() {
    let basis = Builtin::Srgb.profile();
    let r = requirements_with_control(basis, Encoding::NonlinearProfile, Limits::default(), &|| false).unwrap();
    let parts = construct_with_control(basis, Encoding::NonlinearProfile, Limits::default(), &|| false).unwrap();
    let retained = parts.retained_bytes_with_control(Limits::default(), &|| false).unwrap();
    let physical = parts.physical_forward.retained_bytes_with_control(Limits::default(), &|| false).unwrap()
        + parts.physical_reverse.retained_bytes_with_control(Limits::default(), &|| false).unwrap();
    let normalized = parts.forward.retained_bytes_with_control(Limits::default(), &|| false).unwrap()
        + parts.reverse.retained_bytes_with_control(Limits::default(), &|| false).unwrap();
    assert!(retained > physical && retained > normalized);
    assert!(retained <= r.peak_resident_bytes);
    assert!(matches!(parts.retained_bytes_with_control(Limits { resident_bytes: retained - 1, ..Limits::default() }, &|| false), Err(Error::Limit(_))));
    assert!(matches!(parts.retained_bytes_with_control(Limits::default(), &|| true), Err(Error::Cancelled)));
    let forward = parts.physical_forward.evaluation_work_units_with_control(Limits::default(), &|| false).unwrap();
    let reverse = parts.physical_reverse.evaluation_work_units_with_control(Limits::default(), &|| false).unwrap();
    assert!(forward > 0 && reverse > 0);
    // A forward-only or one-forward/one-reverse estimate is insufficient for
    // evaluating both backdrop and source before the physical inverse.
    assert!(parts.per_pixel_work_units > forward.checked_mul(2).unwrap().checked_add(reverse).unwrap());
    let one = Pipeline::new(3, vec![Stage::XyzToLab]);
    let two = Pipeline::new(3, vec![Stage::XyzToLab, Stage::LabToXyz]);
    let a = one.evaluation_work_units_with_control(Limits::default(), &|| false).unwrap();
    let b = two.evaluation_work_units_with_control(Limits::default(), &|| false).unwrap();
    assert!(b > a);
    assert!(two.evaluation_work_units_with_control(Limits { work_units: 0, ..Limits::default() }, &|| false).is_err());
    assert!(matches!(two.evaluation_work_units_with_control(Limits::default(), &|| true), Err(Error::Cancelled)));
}

// Assemble the prior public physical recipe independently of owned_lab. It is
// an external reference allocation, never passed off as an accounted owner.
fn reference_sides() -> (Pipeline, Pipeline) {
    let basis = Builtin::Srgb.profile();
    let (mut forward, fp) = basis.device_to_pcs(Intent::RelativeColorimetric).unwrap();
    let (reverse, rp) = basis.pcs_to_device(Intent::RelativeColorimetric).unwrap();
    assert_eq!(fp, Pcs::Xyz);
    assert_eq!(rp, Pcs::Xyz);
    forward.push(Stage::XyzToLab);
    let mut backward = vec![Stage::LabToXyz];
    backward.extend(reverse);
    (Pipeline::new(3, forward), Pipeline::new(3, backward))
}
fn evaluate(p: &Pipeline, input: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0; 3];
    p.try_eval_f64(&input, &mut out).unwrap();
    out
}
#[test]
#[ignore = "UNRUN: physical CIE Lab D50 sides preserve old wide signed/unclipped f64 recipe; no normalized ICC Lab reference or producer claim"]
fn physical_d50_wide_sides_keep_signed_out_of_gamut_words() {
    let parts = construct_with_control(Builtin::Srgb.profile(), Encoding::NonlinearProfile, Limits::default(), &|| false).unwrap();
    let (old_forward, old_reverse) = reference_sides();
    // These two normalized ICC transforms coexist in Parts, but are NOT the
    // physical-Lab component path. Check their retained recipe separately so
    // merely constructing unrelated identity placeholders would not pass.
    let source = Builtin::Srgb.profile();
    let target = Builtin::LabD50.profile();
    let normalized_forward = crate::Transform::new(source, target, Intent::RelativeColorimetric, false).unwrap();
    let normalized_reverse = crate::Transform::new(target, source, Intent::RelativeColorimetric, false).unwrap();
    for (input, actual, reference) in [([0.25, 0.5, 0.75], &parts.forward, &normalized_forward), ([0.53, 0.6, 0.4], &parts.reverse, &normalized_reverse)] {
        let mut a = [0.0; 3];
        let mut b = [0.0; 3];
        actual.try_eval_f64(&input, &mut a).unwrap();
        reference.try_eval_f64(&input, &mut b).unwrap();
        assert_eq!(a.map(f64::to_bits), b.map(f64::to_bits));
    }

    for input in [[0.0; 3], [1.0; 3], [0.15, 0.6, 0.85], [0.25, 0.5, 0.75]] {
        assert_eq!(evaluate(&parts.physical_forward, input).map(f64::to_bits), evaluate(&old_forward, input).map(f64::to_bits));
    }
    let white = evaluate(&parts.physical_forward, [1.0; 3]);
    assert!(white[0] > 90.0); // physical L*, not an ICC normalized 0..1 code
    let a = [53.0, 200.0, -300.0];
    let signed = evaluate(&parts.physical_reverse, a);
    assert!(signed.iter().all(|v| v.is_finite()));
    assert!(signed.iter().any(|v| *v < 0.0) && signed.iter().any(|v| *v > 1.0));
    for input in [a, [53.0, 30.0, -20.0], [53.0 + 2.0_f64.powi(-35), 30.0, -20.0]] {
        assert_eq!(evaluate(&parts.physical_reverse, input).map(f64::to_bits), evaluate(&old_reverse, input).map(f64::to_bits));
    }
    assert_ne!(evaluate(&parts.physical_reverse, [53.0, 30.0, -20.0]), evaluate(&parts.physical_reverse, [53.0 + 2.0_f64.powi(-35), 30.0, -20.0]));
    let mut unchanged = [17.0; 3];
    assert!(parts.physical_reverse.try_eval_f64(&[f64::NAN, 0.0, 0.0], &mut unchanged).is_err());
    assert_eq!(unchanged, [17.0; 3]);
}

fn unsupported_profile_shapes_and_encodings_stay_closed() {
    let basis = Builtin::Srgb.profile();
    for stored in [Encoding::Linear, Encoding::Perceptual] {
        assert!(planning_work_ceiling(basis, stored).is_err());
        assert!(construct_with_control(basis, stored, Limits::default(), &|| false).is_err());
    }
    for builtin in [Builtin::SGray, Builtin::CoatedCmyk, Builtin::LabD50] {
        assert!(planning_work_ceiling(builtin.profile(), Encoding::NonlinearProfile).is_err());
        assert!(requirements_with_control(builtin.profile(), Encoding::NonlinearProfile, Limits::default(), &|| false).is_err());
    }
    let mut table = basis.try_clone_with_control(Limits::default(), &|| false).unwrap();
    table.trc = Some([Curve::Table(vec![0.0, 1.0]), Curve::Identity, Curve::Identity]);
    assert!(requirements_with_control(&table, Encoding::NonlinearProfile, Limits::default(), &|| false).is_err());
    let mut singular = basis.try_clone_with_control(Limits::default(), &|| false).unwrap();
    singular.matrix = Some([[0.0; 3]; 3]);
    assert!(construct_with_control(&singular, Encoding::NonlinearProfile, Limits::default(), &|| false).is_err());
}
