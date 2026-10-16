//! UNRUN source integration cost/control tests. Decoded files/buffers are external harness;
//! real production phase fees are held by the caller Budget, never inferred from these fixtures.
use super::origin_geometry_tests::{INPUTS, budget, expected, file};
use super::*;
use photocraft_cms::xcf_color::prepared_implicit_transfer as transfer;
use photocraft_xcf::BudgetKind;
use std::cell::Cell;
fn facts(f: &XcfFile) -> transfer::Requirements {
    transfer::requirements_with_control(
        if f.model == Model::Gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb },
        photocraft_cms::control::Limits { resident_bytes: 65536, work_units: 65536 * 64 },
        &|| false,
    )
    .unwrap()
}
#[test]
#[ignore = "UNRUN: all13 unchanged source roles validate once perqualification; addedpixels pay actualconversionwork without repeatedprofilevalidation"]
fn once_preparation_and_pixel_growth() {
    for i in 0..INPUTS.len() {
        let f = file(i, 1, 1);
        let required = facts(&f);
        let one = budget();
        assert!(classify(&f, &Interrupt::NONE, Some(&one)).unwrap() == expected(i));
        let f = file(i, 65, 1);
        let many = budget();
        let before = many.used(BudgetKind::Resident);
        assert!(classify(&f, &Interrupt::NONE, Some(&many)).unwrap() == expected(i));
        assert_eq!(many.used(BudgetKind::Resident), before);
        let used = many.used(BudgetKind::Work);
        assert!(used >= transfer::planning_work_ceiling() + required.preparation_work_units + 65 * (required.conversion_work_units + 128));
        let added = used - one.used(BudgetKind::Work);
        assert!(added >= 64 * (required.conversion_work_units + 128));
        let p = if f.model == Model::Gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(p).unwrap();
        assert!(added < 64 * semantic, "source profile validation must not repeat per added pixel");
    }
}
#[test]
#[ignore = "UNRUN: planning/construction/late-mask work and actualsourcephase resident refusal refund localtoken holder without altering sourcebits"]
fn lowered_phase_limits_and_refunds() {
    for i in [0, 1, 9, 10, 11, 12] {
        let f = file(i, 65, 1);
        let required = facts(&f);
        let native = f.layers[0].plane.as_ref().unwrap().bytes.clone();
        let complete = budget();
        assert!(classify(&f, &Interrupt::NONE, Some(&complete)).unwrap() == expected(i));
        let full = complete.used(BudgetKind::Work);
        for limit in [transfer::planning_work_ceiling() - 1, transfer::planning_work_ceiling() + required.preparation_work_units - 1, full - 1] {
            let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", limit)]).unwrap();
            let b = photocraft_xcf::Budget::new_persistent(limits, None).unwrap();
            let before = b.used(BudgetKind::Resident);
            assert!(classify(&f, &Interrupt::NONE, Some(&b)).is_err());
            assert_eq!(b.used(BudgetKind::Resident), before);
            assert_eq!(f.layers[0].plane.as_ref().unwrap().bytes, native);
        }
        let b = budget();
        let before = b.used(BudgetKind::Resident);
        let hold = b.reserve(BudgetKind::Resident, b.limits().get(BudgetKind::Resident) - before - 1, "test other live caller phase").unwrap();
        let held = b.used(BudgetKind::Resident);
        assert!(classify(&f, &Interrupt::NONE, Some(&b)).is_err());
        assert_eq!(b.used(BudgetKind::Resident), held);
        drop(hold);
        assert_eq!(b.used(BudgetKind::Resident), before);
    }
}
#[test]
#[ignore = "UNRUN: sameouterledger across independentqualifiers, late sharedwork cancellation typed and no retainedphase/native mutation or fallback"]
fn cumulative_outer_ledger_and_late_cancel() {
    for i in [0, 1, 9, 10] {
        let f = file(i, 65, 1);
        let native = f.layers[0].plane.as_ref().unwrap().bytes.clone();
        let b = budget();
        let before = b.used(BudgetKind::Resident);
        assert!(classify(&f, &Interrupt::NONE, Some(&b)).unwrap() == expected(i));
        let first = b.used(BudgetKind::Work);
        assert_eq!(b.used(BudgetKind::Resident), before);
        assert!(classify(&f, &Interrupt::NONE, Some(&b)).unwrap() == expected(i));
        assert_eq!(b.used(BudgetKind::Work), 2 * first);
        assert_eq!(b.used(BudgetKind::Resident), before);
        let seen = Cell::new(0u64);
        let observe = |units: u64| {
            let next = seen.get().checked_add(units).unwrap();
            seen.set(next);
            if next >= first {
                Err(photocraft_xcf::XcfError::new(photocraft_xcf::ErrorKind::Cancelled, 0, "test cancellation at final source proof"))
            } else {
                Ok(())
            }
        };
        let interrupted =
            photocraft_xcf::Budget::new_persistent_with_work(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None, Some(&observe))
                .unwrap();
        let before = interrupted.used(BudgetKind::Resident);
        assert!(matches!(classify(&f, &Interrupt::NONE, Some(&interrupted)), Err(IoError::Cancelled)));
        assert_eq!(interrupted.used(BudgetKind::Resident), before);
        assert!(interrupted.used(BudgetKind::Work) > transfer::planning_work_ceiling() + facts(&f).preparation_work_units);
        assert_eq!(f.layers[0].plane.as_ref().unwrap().bytes, native);
    }
    assert!(matches!(prepared_transfer_error(photocraft_cms::control::Error::Cancelled), IoError::Cancelled));
}
