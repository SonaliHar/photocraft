//! UNRUN drafts: no test execution was authorized for this cohort.
use super::*;
use crate::xcf_color::{Encoding, effective_implicit_profile_requirements_with_control, effective_implicit_profile_with_control};

#[test]
fn planner_matches_actual_nonlinear_requirements_including_mismatch_length() {
    let bytes = Builtin::Srgb.profile().to_bytes();
    for bound in [bytes.as_slice(), &[1, 2, 3][..], &[][..]] {
        let planned = effective_implicit_rgb_nonlinear_planner_work_with_control(bound.len(), Limits::default(), &|| false).unwrap();
        let actual =
            effective_implicit_profile_requirements_with_control(bound, ColorSpace::Rgb, Encoding::NonlinearProfile, Limits::default(), &|| false).unwrap();
        assert_eq!(planned, actual.work_units);
    }
    assert!(effective_implicit_profile_with_control(&[1, 2, 3], ColorSpace::Rgb, Encoding::NonlinearProfile, Limits::default(), &|| false).is_err());
}

#[test]
fn insufficient_planning_work_and_cancel_refuse() {
    assert!(matches!(
        effective_implicit_rgb_nonlinear_planner_work_with_control(0, Limits { work_units: 4095, ..Limits::default() }, &|| false),
        Err(Error::Limit(_))
    ));
    assert!(matches!(effective_implicit_rgb_nonlinear_planner_work_with_control(0, Limits::default(), &|| true), Err(Error::Cancelled)));
    let required = effective_implicit_rgb_nonlinear_planner_work_with_control(0, Limits { work_units: 4096, ..Limits::default() }, &|| false).unwrap();
    assert!(required > 4096);
}

#[test]
fn length_refusal_precedes_builtin_provider_boundary() {
    let calls = std::cell::Cell::new(0);
    let result = effective_implicit_rgb_nonlinear_planner_work_with_control((16 << 20) + 1, Limits::default(), &|| {
        calls.set(calls.get() + 1);
        false
    });
    assert!(matches!(result, Err(Error::Cms(_))));
    assert_eq!(calls.get(), 1); // Budget entry only; no provider/metadata polls.
}

#[test]
fn signature_exposes_neither_encoding_override_nor_profile_assertion() {
    let planner: fn(usize, Limits, &dyn Fn() -> bool) -> Result<u64> = effective_implicit_rgb_nonlinear_planner_work_with_control;
    let source = Builtin::Srgb.profile();
    let work = planner(0, Limits::default(), &|| false).unwrap();
    assert_eq!(
        work,
        crate::xcf_color::effective_profile_requirements_with_control(source, Encoding::NonlinearProfile, Limits::default(), &|| false).unwrap().work_units
    );
    // Linear/perceptual callers must use their real transfer planner; this API
    // has no encoding parameter and cannot admit a transfer writer recipe.
}
