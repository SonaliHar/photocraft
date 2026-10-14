//! Shared after-callback edit validation; document/result ownership remains in Session.
use crate::{DocumentSnapshot, EngineError, Result, channel_cmds, retained_surface_validation, xcf_editing, xcf_vector_guard};
use photocraft_doc::{Document, LayerId};
use photocraft_raster::Interrupt;

#[inline(never)]
pub(crate) fn validate(before: &DocumentSnapshot, doc: &mut Document, active: Option<LayerId>, restrict: Option<usize>, control: &Interrupt<'_>) -> Result<()> {
    validate_font_input(before, doc, active, restrict, None, control)
}
/// A per-call borrowed input, retained only through validation. This grants no
/// font registration, source qualification, or persistent document authority.
pub(crate) fn validate_with_authorized_font(
    before: &DocumentSnapshot,
    doc: &mut Document,
    active: Option<LayerId>,
    restrict: Option<usize>,
    font: &photocraft_text::xcf::runtime_font_input::PreparedRuntimeFont<'_>,
    control: &Interrupt<'_>,
) -> Result<()> {
    validate_font_input(before, doc, active, restrict, Some(font), control)
}
fn validate_font_input(
    before: &DocumentSnapshot,
    doc: &mut Document,
    active: Option<LayerId>,
    restrict: Option<usize>,
    font: Option<&photocraft_text::xcf::runtime_font_input::PreparedRuntimeFont<'_>>,
    control: &Interrupt<'_>,
) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    retained_surface_validation::validate_document(doc, control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("invalid edited raster: {error}")) })?;
    // Current interpretation follows the active profile. Original import
    // provenance is retained independently across explicit color edits.
    if doc.icc_profile != before.icc_profile || doc.mode != before.mode || doc.sample_encoding != before.sample_encoding {
        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc;
    }
    doc.working_profile_interpretation.validate_with_interrupt(doc.icc_profile.as_deref().map(Vec::as_slice), doc.mode, control)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
    if let (Some(k), Some(id)) = (restrict, active) {
        channel_cmds::restrict_to_color(before, doc, id, k);
    }
    xcf_editing::synchronize_indexed_with_interrupt(before, doc, control)?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    xcf_vector_guard::validate_path_edits_with_interrupt(before, doc, control)?;
    xcf_editing::synchronize_layer_locks_with_interrupt(before, doc, control)?;
    if before.xcf_state.is_some() {
        let previous = visibility_layers(before, control)?;
        let current = visibility_layers(doc, control)?;
        let mut changed = Vec::new();
        changed.try_reserve_exact(previous.len()).map_err(|_| EngineError::Other("visibility validation allocation".into()))?;
        for (id, layer) in previous {
            control.check().map_err(|_| EngineError::Cancelled)?;
            // Creation and removal do not invent a visibility transition.
            if let Some(edited) = current.get(&id)
                && layer.visible != edited.visible
            {
                changed.push(id);
            }
        }
        crate::xcf_item_locks::require_visibility_changes(before, &changed, control)?;
    }
    if let Some(mut state) = doc.xcf_state.take() {
        state.reconcile_layers_with_interrupt(doc, control).map_err(|e| {
            if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("cannot preserve imported editing state: {e}")) }
        })?;
        doc.xcf_state = Some(state);
    }
    if xcf_vector_guard::has_live_sources(before) {
        let previous_qualified = match font {
            Some(font) => photocraft_compose::gimp_render::PreparedRenderer::new_with_authorized_font(before, font, control),
            None => photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(before, control),
        }
        .is_ok();
        if previous_qualified {
            match font {
                Some(font) => photocraft_compose::gimp_render::PreparedRenderer::new_with_authorized_font(doc, font, control),
                None => photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control),
            }
            .map_err(|error| {
                if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("cannot preserve live imported rendering: {error}")) }
            })?;
        }
    }
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok(())
}

// Independent of imported item membership: an ordinary child still inherits
// its imported group's visibility lock. Borrow data; never inspect pixels.
fn visibility_layers<'a>(doc: &'a Document, control: &Interrupt<'_>) -> Result<std::collections::HashMap<LayerId, &'a photocraft_doc::Layer>> {
    const MAX_LAYERS: usize = 10_000;
    if doc.layers.len() > MAX_LAYERS {
        return Err(EngineError::Other("visibility validation layer limit".into()));
    }
    let mut result = std::collections::HashMap::new();
    let mut pending = Vec::new();
    result.try_reserve(MAX_LAYERS).map_err(|_| EngineError::Other("visibility validation allocation".into()))?;
    pending.try_reserve_exact(MAX_LAYERS).map_err(|_| EngineError::Other("visibility validation allocation".into()))?;
    pending.extend(doc.layers.iter().map(|layer| (layer, 0usize)));
    while let Some((layer, depth)) = pending.pop() {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if depth > photocraft_doc::MAX_GROUP_DEPTH || result.len() >= MAX_LAYERS || layer.id.0 == 0 {
            return Err(EngineError::Other("visibility validation hierarchy limit".into()));
        }
        if result.insert(layer.id, layer).is_some() {
            return Err(EngineError::Other("visibility validation duplicate layer identity".into()));
        }
        if let photocraft_doc::LayerContent::Group(group) = &layer.content {
            if pending.len().checked_add(result.len()).and_then(|n| n.checked_add(group.children.len())).is_none_or(|n| n > MAX_LAYERS) {
                return Err(EngineError::Other("visibility validation layer limit".into()));
            }
            let depth = depth.checked_add(1).ok_or_else(|| EngineError::Other("visibility validation hierarchy limit".into()))?;
            pending.extend(group.children.iter().map(|child| (child, depth)));
        }
    }
    Ok(result)
}
