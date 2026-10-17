//! Assigned CURRENT foreground with a coherent linked physical layer mask.
//! Selection only chooses this stricter lane; refusal never retries plain Move.
use super::*;
use photocraft_doc::{LayerContent, accounted_document::AccountedDocument};

pub(super) fn candidate(owner: &AccountedDocument) -> bool {
    let doc = owner.document();
    !owner.is_readonly_point_storage()
        && !owner.is_readonly_threshold_storage()
        && !owner.is_readonly_threshold_alpha_storage()
        && doc.layers.len() == 2
        && doc.layers.get(1).is_some_and(|layer| layer.mask.is_some() && matches!(&layer.content, LayerContent::BackedRaster(_)))
        && doc.xcf_state.as_ref().is_some_and(|state| state.items.len() == 3)
}
pub(super) fn validate(owner: &AccountedDocument, layer: LayerId, dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<()> {
    owner.validate_assigned_layer_translation(layer, dx, dy, c)?;
    Ok(())
}
pub(super) fn stage(owner: &AccountedDocument, layer: LayerId, dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<AccountedDocument> {
    Ok(owner.try_with_assigned_layer_translation(layer, dx, dy, c)?)
}
pub(super) fn current(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<()> {
    photocraft_compose::backed_assigned_current::validate_current_assigned_geometry_storage(owner, c)
        .map_err(|error| super::imported_translate::current_error(error, c, "owned assigned Move current recipe"))
}

#[cfg(all(test, feature = "corpus", not(target_arch = "wasm32")))]
#[path = "assigned_translate_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
