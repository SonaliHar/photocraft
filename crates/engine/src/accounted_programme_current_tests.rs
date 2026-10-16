//! Deferred dispatcher contracts; fixtures retain actual paid source/native owners.
//! Hierarchy is synthetic CURRENT, not a newly qualified hierarchy Source.
use super::*;
#[path = "accounted_normal_hierarchy_test_support.rs"]
mod hierarchy;
#[path = "accounted_masked_scalar_test_support.rs"]
mod point;
use crate::EngineError;

fn check(owner: &AccountedDocument, expected: ProgrammeKind) {
    assert_eq!(kind(owner), Some(expected));
    validate(owner, expected, &hierarchy::ctl()).unwrap();
    // Warmed success establishes the actual provider cache baseline. No cache
    // eviction or previously paid recipe is assumed to waive a subsequent walk.
    let baseline = owner.quota().usage().unwrap();
    for wrong in [
        ProgrammeKind::ThresholdAlpha,
        ProgrammeKind::HistogramThreshold,
        ProgrammeKind::Point,
        ProgrammeKind::AssignedPoint,
        ProgrammeKind::GaussianGroup,
        ProgrammeKind::FlatComposite,
        ProgrammeKind::FilteredFlatPoint,
        ProgrammeKind::MaskedFilteredFlatPoint,
        ProgrammeKind::NormalHierarchy,
    ] {
        if wrong == expected {
            continue;
        }
        let error = validate(owner, wrong, &hierarchy::ctl()).unwrap_err();
        assert_eq!(error.to_string(), BackingError::Unsupported.to_string());
        assert_eq!(owner.quota().usage().unwrap(), baseline);
    }
    let work = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work };
    assert_eq!(validate(owner, expected, &c).unwrap_err().to_string(), BackingError::Work.to_string());
    assert_eq!(owner.quota().usage().unwrap(), baseline);
    // Original typed cancellation must not become a recipe fallback or success.
    let cancelled = |_| Err(BackingError::Cancelled);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &cancelled };
    assert!(matches!(validate(owner, expected, &c), Err(EngineError::Cancelled)));
    assert_eq!(owner.quota().usage().unwrap(), baseline);
    validate(owner, expected, &hierarchy::ctl()).unwrap();
    assert_eq!(owner.quota().usage().unwrap(), baseline);
}

#[test]
#[ignore = "UNRUN/uncompiled: actual paid Point sources and synthetic retained-native filtered hierarchy CURRENT; exact nine-way class refusal, full Current success and original Work/cancel/refund. Not all-class positive coverage or Source authority."]
fn actual_class_is_required_before_complete_current_delegation() {
    for depth in 0..2 {
        let owner = hierarchy::build(depth);
        check(&owner, ProgrammeKind::NormalHierarchy);
        crate::accounted_xcf_open::normal_hierarchy::validate_current_normal_hierarchy_storage(&owner, &hierarchy::ctl()).unwrap();
    }
    for case in point::cases() {
        let imported = point::import(&case);
        let owner = imported.document();
        check(owner, ProgrammeKind::Point);
        accounted_native_open::validate_current_owned_filter_storage(owner, &hierarchy::ctl()).unwrap();
    }
}
