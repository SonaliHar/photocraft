//! Controlled CURRENT metadata copies for genuine readonly Point custody.
//! This preserves structural storage only; the engine must validate the full
//! operative Point recipe before and after a prepared publication.
use super::*;

pub(super) fn layer_named(owner: &AccountedDocument, patch: ReadonlyLayerNamePatch<'_>, control: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    imported_edits::validate_name(patch.name, control)?;
    let logical = measured(owner.document(), owner.quota(), control)?;
    if !readonly_layer_name::changed(owner.document(), patch, control)? {
        control.check(1)?;
        return Ok(owner.clone());
    }
    current_copy(owner, logical, CopyKind::LayerName(patch), control)
}

pub(super) fn layer_properties(
    owner: &AccountedDocument,
    patch: ReadonlyLayerPropertiesPatch<'_>,
    control: &ReadControl<'_>,
) -> Result<AccountedDocument, BackingError> {
    readonly_layer_properties::validate(patch, control)?;
    let logical = measured(owner.document(), owner.quota(), control)?;
    let Some(plan) = readonly_layer_properties::plan(owner.document(), patch, control)? else {
        control.check(1)?;
        return Ok(owner.clone());
    };
    current_copy(owner, logical, CopyKind::LayerProperties(plan), control)
}

#[cfg(test)]
#[path = "properties_tests.rs"]
mod tests;
