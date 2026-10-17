//! UNRUN fixed-implicit-basis transfer contract regressions. Harness Vecs and
//! legacy reference profiles are external test data; CMS Limits are prospective
//! component facts, not outer quota receipts, RSS or elapsed-time claims.
use super::{PreparedImplicitTransfer, construct_with_control, planning_work_ceiling, requirements_with_control};
use crate::{
    Builtin, ColorSpace,
    control::{Error, Limits},
    xcf_color::{Encoding, convert_encoding_f64},
};
use std::{cell::Cell, sync::Arc};
const ENCODINGS: [Encoding; 3] = [Encoding::Linear, Encoding::NonlinearProfile, Encoding::Perceptual];
const SPACES: [ColorSpace; 2] = [ColorSpace::Rgb, ColorSpace::Gray];
fn basis(space: ColorSpace) -> &'static crate::Profile {
    match space {
        ColorSpace::Rgb => Builtin::Srgb.profile(),
        ColorSpace::Gray => Builtin::SGray.profile(),
        _ => panic!("test basis"),
    }
}
fn bits(values: &[f64]) -> Vec<u64> {
    values.iter().map(|v| v.to_bits()).collect()
}
fn input(space: ColorSpace, value: f64) -> Vec<f64> {
    if space == ColorSpace::Rgb { vec![value, 0.125, 0.875] } else { vec![value] }
}
fn sentinel(len: usize) -> Vec<f64> {
    (0..len).map(|i| f64::from_bits(0x7ff8_0000_0000_0010 + i as u64)).collect()
}
fn conversion_limits(token: &PreparedImplicitTransfer) -> Limits {
    let r = token.requirements();
    Limits { resident_bytes: r.conversion_scratch_bytes, work_units: r.conversion_work_units }
}
#[test]
#[ignore = "UNRUN: RGB/Gray all nine encoding pairs match exact legacy bits including identity, signed zero, subnormal, finite HDR and overflow errors"]
fn fixed_bases_preserve_legacy_numeric_schedule_and_identity_bits() {
    let values = [
        -4.0,
        -1.0,
        -0.04045,
        -0.0031308,
        -0.0,
        0.0,
        f64::from_bits(1),
        f64::MIN_POSITIVE,
        0.0031308,
        0.04045,
        0.5,
        f64::from_bits(1f64.to_bits() - 1),
        1.0,
        4.0,
        f64::MAX,
    ];
    let mut saw_legacy_error = false;
    for space in SPACES {
        let _warm_builtin_bytes = basis(space).to_bytes();
        for from in ENCODINGS {
            for to in ENCODINGS {
                let token = construct_with_control(space, from, to, Limits::default(), &|| false).unwrap();
                for value in values {
                    let source = input(space, value);
                    let mut expected = sentinel(source.len());
                    let mut actual = expected.clone();
                    let untouched = bits(&actual);
                    let old = convert_encoding_f64(basis(space), from, to, &source, &mut expected);
                    let new = token.convert_with_control(&source, &mut actual, conversion_limits(&token), &|| false);
                    match (old, new) {
                        (Ok(()), Ok(())) => {
                            assert_eq!(bits(&actual), bits(&expected), "{space:?} {from:?}->{to:?} input {value:?}");
                            if from == to {
                                assert_eq!(bits(&actual), bits(&source));
                            }
                        }
                        (Err(_), Err(Error::Cms(_))) => {
                            saw_legacy_error = true;
                            assert_eq!(bits(&expected), untouched);
                            assert_eq!(bits(&actual), untouched);
                        }
                        (old, new) => panic!("legacy/prepared disposition differs: {space:?} {from:?}->{to:?} {value:?}: {old:?} {new:?}"),
                    }
                }
            }
        }
    }
    assert!(saw_legacy_error, "finite input overflow is refused, not clamped or partially published");
}
#[test]
#[ignore = "UNRUN: invalid lengths/nonfinite inputs and every conversion cancellation checkpoint preserve caller output atomically"]
fn validation_and_cancellation_never_partially_publish_output() {
    for space in SPACES {
        let _warm_builtin_bytes = basis(space).to_bytes();
        for from in ENCODINGS {
            for to in ENCODINGS {
                let token = construct_with_control(space, from, to, Limits::default(), &|| false).unwrap();
                let channels = basis(space).channels();
                let limits = conversion_limits(&token);
                for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                    let source = input(space, value);
                    let mut output = sentinel(channels);
                    let before = bits(&output);
                    assert!(matches!(token.convert_with_control(&source, &mut output, limits, &|| false), Err(Error::Cms(_))));
                    assert_eq!(bits(&output), before);
                }
                for (source_len, output_len) in [(channels - 1, channels), (channels + 1, channels), (channels, channels - 1), (channels, channels + 1)] {
                    let source = vec![0.25; source_len];
                    let mut output = sentinel(output_len);
                    let before = bits(&output);
                    assert!(matches!(token.convert_with_control(&source, &mut output, limits, &|| false), Err(Error::Cms(_))));
                    assert_eq!(bits(&output), before);
                }
                let source = input(space, 0.375);
                let polls = Cell::new(0usize);
                let mut success = sentinel(channels);
                token
                    .convert_with_control(&source, &mut success, limits, &|| {
                        polls.set(polls.get() + 1);
                        false
                    })
                    .unwrap();
                let total = polls.get();
                assert!(total > 0);
                // Calibrate from the public successful call, not hardcoded private phase
                // indices. Includes the last checkpoint immediately before output copy.
                for allowed in 0..total {
                    let count = Cell::new(0usize);
                    let mut output = sentinel(channels);
                    let before = bits(&output);
                    let result = token.convert_with_control(&source, &mut output, limits, &|| {
                        let n = count.get();
                        count.set(n + 1);
                        n >= allowed
                    });
                    assert!(matches!(result, Err(Error::Cancelled)), "allowed {allowed}/{total}");
                    assert_eq!(bits(&output), before);
                    assert_eq!(bits(&source), bits(&input(space, 0.375)));
                }
            }
        }
    }
}
#[test]
#[ignore = "UNRUN: exact/lowered/raised component and scratch/work limits plus unsupported models fail closed without arbitrary profile API"]
fn prospective_limits_are_component_aware_and_lower_only() {
    // The type accepts a fixed model identity, not a caller Profile/ICC/LUT.
    type Constructor = fn(ColorSpace, Encoding, Encoding, Limits, &dyn Fn() -> bool) -> crate::control::Result<PreparedImplicitTransfer>;
    let _: Constructor = construct_with_control;
    assert!(planning_work_ceiling() > 0);
    for space in SPACES {
        let _warm_builtin_bytes = basis(space).to_bytes();
        let r = requirements_with_control(space, Limits::default(), &|| false).unwrap();
        assert!(r.preparation_work_units >= planning_work_ceiling());
        assert!(r.conversion_work_units > 0);
        assert!(r.borrowed_profile_bytes > 0);
        assert_eq!(r.inline_state_bytes, std::mem::size_of::<PreparedImplicitTransfer>());
        assert_eq!(r.component_resident_bytes, r.borrowed_profile_bytes.checked_add(r.peak_resident_bytes).unwrap());
        assert!(r.peak_resident_bytes >= r.inline_state_bytes + r.conversion_scratch_bytes);
        let exact = Limits { resident_bytes: r.component_resident_bytes, work_units: r.preparation_work_units };
        let again = requirements_with_control(space, exact, &|| false).unwrap();
        assert_eq!(again.component_resident_bytes, r.component_resident_bytes);
        let token = construct_with_control(space, Encoding::Linear, Encoding::NonlinearProfile, exact, &|| false).unwrap();
        let polls = Cell::new(0usize);
        let _complete = construct_with_control(space, Encoding::Linear, Encoding::NonlinearProfile, exact, &|| {
            polls.set(polls.get() + 1);
            false
        })
        .unwrap();
        let total = polls.get();
        assert!(total > 2);
        for allowed in [1, total / 2, total - 1] {
            let count = Cell::new(0usize);
            assert!(matches!(
                construct_with_control(space, Encoding::Linear, Encoding::NonlinearProfile, exact, &|| {
                    let n = count.get();
                    count.set(n + 1);
                    n >= allowed
                }),
                Err(Error::Cancelled)
            ));
        }
        for lower in [Limits { resident_bytes: r.component_resident_bytes - 1, ..exact }, Limits { work_units: r.preparation_work_units - 1, ..exact }] {
            assert!(matches!(requirements_with_control(space, lower, &|| false), Err(Error::Limit(_))));
            assert!(matches!(construct_with_control(space, Encoding::Linear, Encoding::NonlinearProfile, lower, &|| false), Err(Error::Limit(_))));
        }
        let conversion = conversion_limits(&token);
        let source = input(space, 0.25);
        for lower in [Limits { resident_bytes: conversion.resident_bytes - 1, ..conversion }, Limits { work_units: conversion.work_units - 1, ..conversion }] {
            let mut output = sentinel(source.len());
            let before = bits(&output);
            assert!(matches!(token.convert_with_control(&source, &mut output, lower, &|| false), Err(Error::Limit(_))));
            assert_eq!(bits(&output), before);
        }
        // Identity is a bit-copy after admission, not a work/control bypass.
        let identity = construct_with_control(space, Encoding::Linear, Encoding::Linear, exact, &|| false).unwrap();
        let source = input(space, -0.0);
        let mut output = sentinel(source.len());
        let before = bits(&output);
        assert!(matches!(
            identity.convert_with_control(&source, &mut output, Limits { work_units: 0, ..conversion_limits(&identity) }, &|| false),
            Err(Error::Limit(_))
        ));
        assert_eq!(bits(&output), before);
        assert!(matches!(requirements_with_control(space, Limits::default(), &|| true), Err(Error::Cancelled)));
        assert!(matches!(construct_with_control(space, Encoding::Linear, Encoding::Linear, exact, &|| true), Err(Error::Cancelled)));
        let max = Limits::default();
        for raised in [Limits { resident_bytes: max.resident_bytes + 1, ..max }, Limits { work_units: max.work_units + 1, ..max }] {
            assert!(matches!(requirements_with_control(space, raised, &|| false), Err(Error::Limit(_))));
            assert!(matches!(construct_with_control(space, Encoding::Linear, Encoding::Linear, raised, &|| false), Err(Error::Limit(_))));
            let mut output = sentinel(source.len());
            let before = bits(&output);
            assert!(matches!(token.convert_with_control(&source, &mut output, raised, &|| false), Err(Error::Limit(_))));
            assert_eq!(bits(&output), before);
        }
    }
    for space in [ColorSpace::Xyz, ColorSpace::Lab, ColorSpace::Cmyk, ColorSpace::Color(3), ColorSpace::Unknown(0)] {
        assert!(matches!(requirements_with_control(space, Limits::default(), &|| false), Err(Error::Cms(crate::CmsError::Unsupported(_)))));
        assert!(matches!(
            construct_with_control(space, Encoding::Linear, Encoding::Linear, Limits::default(), &|| false),
            Err(Error::Cms(crate::CmsError::Unsupported(_)))
        ));
    }
}
#[test]
#[ignore = "UNRUN: one controlled constructor supports repeated conversion-only allowances while immutable builtin bytes remain shared; no elapsed performance or quota receipt claim"]
fn repeated_conversions_exclude_repeated_profile_preparation() {
    for space in SPACES {
        let profile = basis(space);
        let original = profile.to_bytes();
        let hash = profile.content_hash();
        let r = requirements_with_control(space, Limits::default(), &|| false).unwrap();
        let constructor = Limits { resident_bytes: r.component_resident_bytes, work_units: r.preparation_work_units };
        let token = construct_with_control(space, Encoding::NonlinearProfile, Encoding::Linear, constructor, &|| false).unwrap();
        let conversion = conversion_limits(&token);
        assert!(conversion.work_units < r.preparation_work_units);
        // The cooperative equivalent of legacy's mandatory validation really
        // refuses this small work allowance, while prepared conversion succeeds.
        crate::xcf_color::validate_profile_with_control(profile, constructor, &|| false).unwrap();
        assert!(matches!(
            crate::xcf_color::validate_profile_with_control(
                profile,
                Limits { resident_bytes: r.borrowed_profile_bytes, work_units: conversion.work_units },
                &|| false
            ),
            Err(Error::Limit(_))
        ));
        assert!(matches!(construct_with_control(space, Encoding::NonlinearProfile, Encoding::Linear, conversion, &|| false), Err(Error::Limit(_))));
        for value in [-2.0, -0.0, f64::from_bits(1), 0.125, 0.5, 1.0, 2.0] {
            let source = input(space, value);
            let mut expected = sentinel(source.len());
            let mut output = expected.clone();
            convert_encoding_f64(profile, Encoding::NonlinearProfile, Encoding::Linear, &source, &mut expected).unwrap();
            token.convert_with_control(&source, &mut output, conversion, &|| false).unwrap();
            assert_eq!(bits(&output), bits(&expected));
        }
        // These are prospective public work facts, not a timer or a claim that
        // an external caller has already debited any operation ledger.
        let count = 7u64;
        let prepared = r.preparation_work_units.checked_add(count.checked_mul(r.conversion_work_units).unwrap()).unwrap();
        let repeated = count.checked_mul(r.preparation_work_units.checked_add(r.conversion_work_units).unwrap()).unwrap();
        assert!(prepared < repeated);
        assert!(Arc::ptr_eq(&original, &profile.to_bytes()));
        assert_eq!(profile.content_hash(), hash);
        assert_eq!(profile.to_bytes(), original);
    }
}
