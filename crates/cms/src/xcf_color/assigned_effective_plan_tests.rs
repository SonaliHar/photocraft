//! UNRUN drafts: prospective planning is not profile/rendering authority.
use super::*;
use crate::{
    Builtin,
    profile::{Lut, LutKind},
};
use std::sync::Arc;

#[test]
fn admitted_assigned_definitions_preserve_existing_effective_recipe_and_source_bytes() {
    for builtin in [Builtin::Srgb, Builtin::DisplayP3, Builtin::SGray] {
        let original = builtin.profile().to_bytes();
        let source = Profile::parse(&original).unwrap();
        let cached = source.to_bytes();
        for encoding in [Encoding::NonlinearProfile, Encoding::Linear, Encoding::Perceptual] {
            let planning = assigned_effective_planning_work_ceiling(&source, encoding).unwrap();
            assert!(planning > 0 && planning <= Limits::default().work_units);
            // Future operation limits remain independent of the planning debit.
            let requirements = super::super::effective_profile_requirements_with_control(&source, encoding, Limits::default(), &|| false).unwrap();
            let future = requirements.work_units.checked_mul(4).unwrap();
            assert!(future <= Limits::default().work_units);
            let actual = super::super::effective_profile_from_definition_with_control(&source, encoding, Limits::default(), &|| false).unwrap();
            let legacy = super::super::effective_profile_from_definition(&source, encoding).unwrap();
            assert_eq!(actual.to_bytes(), legacy.to_bytes());
            assert_eq!(actual.description, legacy.description);
            assert!(Arc::ptr_eq(&cached, &source.to_bytes()));
            assert_eq!(source.to_bytes(), original);
        }
    }
}

#[test]
fn long_unicode_text_capacity_and_inactive_analytic_tags_are_planned_without_substitution() {
    let mut source = Builtin::Srgb.profile().clone();
    source.description = "assigned \u{1f642} \u{03bb} ".repeat(1024);
    source.copyright = "copyright \u{1f642}".repeat(1024);
    source.gray_trc = Some(Curve::Gamma(2.2));
    let before = source.description.clone();
    let first = assigned_effective_planning_work_ceiling(&source, Encoding::Perceptual).unwrap();
    source.description.reserve_exact(16384);
    let second = assigned_effective_planning_work_ceiling(&source, Encoding::Perceptual).unwrap();
    assert!(second > first, "actual spare text capacity is retained");
    let requirements = super::super::effective_profile_requirements_with_control(&source, Encoding::Perceptual, Limits::default(), &|| false).unwrap();
    assert!(requirements.work_units >= second);
    let actual = super::super::effective_profile_from_definition_with_control(&source, Encoding::Perceptual, Limits::default(), &|| false).unwrap();
    let legacy = super::super::effective_profile_from_definition(&source, Encoding::Perceptual).unwrap();
    assert_eq!(actual.to_bytes(), legacy.to_bytes());
    assert_eq!(source.description, before);
    assert!(matches!(
        super::super::effective_profile_requirements_with_control(&source, Encoding::Perceptual, Limits::default(), &|| true,),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        super::super::effective_profile_requirements_with_control(&source, Encoding::Perceptual, Limits { resident_bytes: 1, ..Limits::default() }, &|| false,),
        Err(Error::Limit(_))
    ));
}

#[test]
fn tables_luts_and_unhandled_models_are_refused_but_planning_does_not_claim_validity() {
    let mut source = Builtin::Srgb.profile().clone();
    // Even an inactive table would enter clone/writer traversal; refuse it.
    source.gray_trc = Some(Curve::Table64(vec![0.0, 1.0]));
    assert!(assigned_effective_planning_work_ceiling(&source, Encoding::Linear).is_err());
    source.gray_trc = None;
    source.trc = Some([Curve::Table(vec![0.0, 1.0]), Curve::Identity, Curve::Identity]);
    assert!(assigned_effective_planning_work_ceiling(&source, Encoding::NonlinearProfile).is_err());
    source.trc = Some([Curve::Identity, Curve::Identity, Curve::Identity]);
    source.a2b[0] = Some(Lut { kind: LutKind::Ab, inputs: 3, outputs: 3, stages: Vec::new() });
    assert!(assigned_effective_planning_work_ceiling(&source, Encoding::Linear).is_err());
    source.a2b[0] = None;
    source.color_space = ColorSpace::Lab;
    assert!(assigned_effective_planning_work_ceiling(&source, Encoding::Linear).is_err());
    source.color_space = ColorSpace::Rgb;
    source.trc = Some([Curve::Gamma(f64::NAN), Curve::Identity, Curve::Identity]);
    assert!(assigned_effective_planning_work_ceiling(&source, Encoding::Linear).is_ok());
    assert!(
        super::super::effective_profile_requirements_with_control(&source, Encoding::Linear, Limits::default(), &|| false,).is_err(),
        "semantic validation remains authoritative"
    );
}

#[test]
fn semantic_only_ceiling_preserves_legacy_inactive_table_acceptance() {
    for builtin in [Builtin::Srgb, Builtin::SGray] {
        let mut source = builtin.profile().clone();
        if source.color_space == ColorSpace::Rgb {
            source.gray_trc = Some(Curve::Table64(vec![0.0, 0.5, 1.0]));
        } else {
            source.trc = Some([Curve::Table(vec![0.0, 1.0]), Curve::Identity, Curve::Identity]);
        }
        super::super::qualify_analytic_profile(&source).unwrap();
        assert!(semantic_analytic_profile_work_ceiling(&source).unwrap() > 0);
        assert!(assigned_effective_planning_work_ceiling(&source, Encoding::Linear).is_err());
        if source.color_space == ColorSpace::Rgb {
            source.trc = Some([Curve::Table64(vec![0.0, 1.0]), Curve::Identity, Curve::Identity]);
        } else {
            source.gray_trc = Some(Curve::Table(vec![0.0, 1.0]));
        }
        assert!(super::super::qualify_analytic_profile(&source).is_err());
        assert!(semantic_analytic_profile_work_ceiling(&source).is_err());
    }
}
