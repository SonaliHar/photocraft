//! Complete CURRENT validation for closed, actual imported storage classes.
//! This module grants neither native-write nor original-Source authority.
use crate::{Result, accounted_native_open, accounted_xcf_open};
use photocraft_doc::accounted_document::AccountedDocument;
use photocraft_raster::backing::{BackingError, ReadControl};

// Share the exact closed L1 target identity; no parallel class registry.
pub(crate) use photocraft_doc::accounted_document::ProgrammeNativeStorageKind as ProgrammeKind;

/// Cheap class routing only. Complete CURRENT proof remains mandatory.
pub(crate) fn kind(owner: &AccountedDocument) -> Option<ProgrammeKind> {
    if owner.is_readonly_threshold_alpha_storage() {
        Some(ProgrammeKind::ThresholdAlpha)
    } else if owner.is_readonly_threshold_storage() {
        Some(ProgrammeKind::HistogramThreshold)
    } else if owner.is_readonly_point_storage() {
        Some(ProgrammeKind::Point)
    } else if owner.is_readonly_assigned_point_storage() {
        Some(ProgrammeKind::AssignedPoint)
    } else if owner.is_readonly_gaussian_group_storage() {
        Some(ProgrammeKind::GaussianGroup)
    } else if owner.is_readonly_flat_composite_storage() {
        Some(ProgrammeKind::FlatComposite)
    } else if owner.is_readonly_filtered_flat_point_storage() {
        Some(ProgrammeKind::FilteredFlatPoint)
    } else if owner.is_readonly_masked_filtered_flat_point_storage() {
        Some(ProgrammeKind::MaskedFilteredFlatPoint)
    } else if owner.is_readonly_normal_hierarchy_storage() {
        Some(ProgrammeKind::NormalHierarchy)
    } else {
        None
    }
}

/// Prove the complete actual CURRENT recipe, including inactive native input.
/// A selected error is final; callers must use the same cumulative control for
/// OLD, history and STAGED proof and independently enforce publication fences.
pub(crate) fn validate(owner: &AccountedDocument, expected: ProgrammeKind, control: &ReadControl<'_>) -> Result<()> {
    control.check(1)?;
    if kind(owner) != Some(expected) {
        return Err(BackingError::Unsupported.into());
    }
    match expected {
        ProgrammeKind::ThresholdAlpha => accounted_native_open::validate_current_threshold_alpha_storage(owner, control),
        ProgrammeKind::HistogramThreshold => accounted_native_open::validate_current_threshold_storage(owner, control),
        ProgrammeKind::Point | ProgrammeKind::AssignedPoint => accounted_native_open::validate_current_owned_filter_storage(owner, control),
        ProgrammeKind::GaussianGroup => accounted_xcf_open::gaussian_group::validate_current_isolated_gaussian_group_storage(owner, control),
        ProgrammeKind::FlatComposite => accounted_xcf_open::separable_blend::validate_current_separable_blend_storage(owner, control),
        ProgrammeKind::FilteredFlatPoint => accounted_xcf_open::filtered_flat::validate_current_filtered_flat_storage(owner, control),
        ProgrammeKind::MaskedFilteredFlatPoint => accounted_xcf_open::masked_filtered_flat::validate_current_masked_filtered_flat_storage(owner, control),
        ProgrammeKind::NormalHierarchy => accounted_xcf_open::normal_hierarchy::validate_current_normal_hierarchy_storage(owner, control),
    }
}

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "accounted_programme_current_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod tests;
