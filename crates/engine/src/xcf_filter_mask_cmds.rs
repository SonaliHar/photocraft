//! Enabled transition for a current, operative filter's qualified empty-selection mask.
//! Nonempty coverage and disabled/unknown filters are deliberately unsupported.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document, LayerId,
    item_identity::{ItemRef, MaskId},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};

pub const SET: &str = "xcf.filter.mask.enabled.set";
const MAX_ITEMS: usize = 10_000;
const MAX_WORK: u64 = 1 << 30;
fn bad(code: &str) -> EngineError {
    EngineError::Other(format!("{SET}.{code}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn failure(control: &Interrupt<'_>, code: &str) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(code) }
}
fn envelope(params: &Value) -> Result<(MaskId, bool)> {
    let object = params.as_object().ok_or_else(|| bad("parameters"))?;
    if object.len() != 2 || object.keys().any(|key| !matches!(key.as_str(), "maskId" | "enabled")) {
        return Err(bad("parameters"));
    }
    let text = object.get("maskId").and_then(Value::as_str).ok_or_else(|| bad("identity"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) || text.len() > 1 && text.starts_with('0') {
        return Err(bad("identity"));
    }
    let raw = text.parse::<u64>().map_err(|_| bad("identity"))?;
    if raw == 0 {
        return Err(bad("identity"));
    }
    Ok((MaskId(raw), object.get("enabled").and_then(Value::as_bool).ok_or_else(|| bad("parameters"))?))
}
/// Called by dispatch before preview, floating, jobs or coalesce normalization.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    envelope(params)?;
    if background || session.command_is_background() || session.command_has_pending_job() || session.has_jobs() || session.color_restrict.is_some() {
        return Err(bad("context"));
    }
    if session.coalesce_request.is_some() || params.as_object().is_some_and(|object| object.contains_key("coalesce")) {
        return Err(bad("coalescing"));
    }
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some()) {
        return Err(bad("floating"));
    }
    if active.revision == u64::MAX {
        return Err(bad("revision"));
    }
    active.doc.validate_resident().map_err(|_| bad("unsupported-accounted"))?;
    Ok(())
}
/// Charge retained rasters and private COW metadata before validation or cloning.
/// This is a raster/structure admission, not a total heap/RSS guarantee.
fn admission(doc: &Document, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("unsupported"))?;
    if state.items.len() > MAX_ITEMS || state.filter_masks.len() > MAX_ITEMS || state.filter_stacks.len() > MAX_ITEMS {
        return Err(bad("limit"));
    }
    let mut count = 0usize;
    let mut work = 0u64;
    for stack in &state.filter_stacks {
        poll(control)?;
        count = count.checked_add(stack.nodes.len()).filter(|n| *n <= MAX_ITEMS).ok_or_else(|| bad("limit"))?;
    }
    for mask in &state.filter_masks {
        poll(control)?;
        let samples = (mask.surface.tile_count() as u64).checked_mul(65536).and_then(|n| n.checked_add(1)).ok_or_else(|| bad("work-limit"))?;
        work = work.checked_add(samples).filter(|n| *n <= MAX_WORK).ok_or_else(|| bad("work-limit"))?;
    }
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let inventory = (state.items.len() as u64).checked_add(count as u64).and_then(|n| n.checked_mul(256)).ok_or_else(|| bad("cow-limit"))?;
    if footprint
        .clone_metadata_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(footprint.retained_bytes))
        .and_then(|n| n.checked_add(inventory))
        .is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad("cow-limit"));
    }
    poll(control)
}
fn relation(doc: &Document, id: MaskId, control: &Interrupt<'_>) -> Result<(LayerId, bool)> {
    admission(doc, control)?;
    doc.validate_xcf_state_with(control).map_err(|_| failure(control, "invalid-state"))?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("unsupported"))?;
    let mut masks = state.filter_masks.iter().filter(|mask| mask.id == id);
    let mask = masks.next().ok_or_else(|| bad("missing-mask"))?;
    if masks.next().is_some() {
        return Err(bad("identity"));
    }
    let mut target = None;
    for stack in &state.filter_stacks {
        poll(control)?;
        for node in &stack.nodes {
            poll(control)?;
            if node.id == mask.owner {
                let ItemRef::Layer(owner) = stack.owner else {
                    return Err(bad("unsupported-owner"));
                };
                if !node.enabled || node.mask != Some(ItemRef::Mask(id)) || target.replace((owner, node)).is_some() {
                    return Err(bad("operative-filter"));
                }
            } else if node.mask == Some(ItemRef::Mask(id)) {
                return Err(bad("ambiguous-mask"));
            }
        }
    }
    let (owner, node) = target.ok_or_else(|| bad("missing-filter"))?;
    let locks = crate::xcf_item_locks::effective_layer_locks(doc, owner)?;
    if locks.native.all || locks.imported.content {
        return Err(bad("locked"));
    }
    let locks = crate::xcf_item_locks::effective_item_locks(doc, ItemRef::Mask(id))?;
    if locks.native.all || locks.imported.content {
        return Err(bad("locked"));
    }
    let prepared = photocraft_compose::gimp_filter_mask_state::prepare_empty_selection(
        state,
        node,
        owner,
        mask.bounds,
        mask.surface.format(),
        photocraft_compose::gimp_filter_mask_state::MaskStateLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: MAX_WORK },
        control,
    )
    .map_err(|_| failure(control, "unsupported-mask"))?;
    if prepared.id() != id || prepared.filter() != node.id || !prepared.permits_unrestricted_support() {
        return Err(bad("unsupported-mask"));
    }
    let item = state.items.iter().find(|item| item.item == ItemRef::Mask(id)).ok_or_else(|| bad("missing-mask"))?;
    let enabled = item.mask.as_ref().ok_or_else(|| bad("missing-mask"))?.enabled;
    // The complete renderer retains operation/version/epoch/profile/raw tuple,
    // independent owner/canvas geometry and per-lane mask format authority.
    photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control).map_err(|_| failure(control, "unsupported-current"))?;
    poll(control)?;
    Ok((owner, enabled))
}
/// Conservative metadata hint for three already closed mask lanes. It performs
/// no sample/profile/render qualification and never reads tiles. A tile-free,
/// declared zero-default mask is required; other candidates remain inspection-only.
pub fn zero_selection_candidate(doc: &Document, owner: LayerId, id: MaskId) -> Option<bool> {
    let state = doc.xcf_state.as_ref()?;
    if owner.0 == 0 || id.0 == 0 || state.items.len() > MAX_ITEMS || state.filter_masks.len() > MAX_ITEMS || state.filter_stacks.len() > MAX_ITEMS {
        return None;
    }
    let mut masks = state.filter_masks.iter().filter(|mask| mask.id == id);
    let mask = masks.next()?;
    if masks.next().is_some() || mask.surface.tile_count() != 0 {
        return None;
    }
    let mut count = 0usize;
    let mut found = None;
    for stack in &state.filter_stacks {
        count = count.checked_add(stack.nodes.len()).filter(|count| *count <= MAX_ITEMS)?;
        for node in &stack.nodes {
            if node.mask == Some(ItemRef::Mask(id)) {
                if found.is_some() || stack.owner != ItemRef::Layer(owner) || node.id != mask.owner {
                    return None;
                }
                found = Some(node);
            }
        }
    }
    let node = found?;
    if node.operation.stored_version != "0:0" {
        return None;
    }
    let format = mask.surface.format();
    let extent = photocraft_doc::Rect::from_xywh(0, 0, 8, 6);
    let closed = match node.operation.name.as_str() {
        "gegl:rgb-clip" => {
            node.operation.schema_epoch == photocraft_compose::gimp_filter_mask_state::RGB_CLIP_HDR_EPOCH
                && format == photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Grayscale, photocraft_doc::SampleType::F32, false)
        }
        "gegl:value-invert" => {
            node.operation.schema_epoch == photocraft_compose::gimp_filter_mask_state::VALUE_INVERT_PRECISION_EPOCH
                && matches!(format.sample, photocraft_doc::SampleType::U16 | photocraft_doc::SampleType::F32 | photocraft_doc::SampleType::F64)
        }
        "gegl:brightness-contrast" => {
            node.operation.schema_epoch == photocraft_compose::gimp_filter_mask_state::BRIGHTNESS_CONTRAST_PRECISION_EPOCH
                && matches!(format.sample, photocraft_doc::SampleType::U16 | photocraft_doc::SampleType::F32 | photocraft_doc::SampleType::F64)
        }
        _ => false,
    };
    if !closed
        || mask.bounds != extent
        || doc.bounds() != extent
        || !photocraft_compose::gimp_filter_mask_state::declared_empty_selection_candidate(state, node, owner)
    {
        return None;
    }
    let mut items = state.items.iter().filter(|item| item.item == ItemRef::Mask(id));
    let enabled = items.next()?.mask.as_ref()?.enabled;
    items.next().is_none().then_some(enabled)
}
fn install(doc: &mut Document, id: MaskId, enabled: bool, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let state = doc.xcf_state.as_mut().ok_or_else(|| bad("missing-mask"))?;
    for item in &mut state.items {
        poll(control)?;
        if item.item == ItemRef::Mask(id) {
            item.mask.as_mut().ok_or_else(|| bad("missing-mask"))?.enabled = enabled;
            return poll(control);
        }
    }
    Err(bad("missing-mask"))
}
pub fn set_enabled_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    poll(control)?;
    let (mask, enabled) = envelope(params)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    let (owner, previous) = relation(&active.doc, mask, control)?;
    let response = json!({"maskId":mask.0.to_string(), "layerId":owner.0.to_string(), "enabled":enabled, "changed":previous != enabled});
    if previous == enabled {
        poll(control)?;
        return Ok(response);
    }
    let baseline = active.doc.clone();
    let revision = active.revision;
    let mut staged = baseline.clone();
    let doc = staged.try_resident_mut_with_raster_limit(MAX_CHECKED_RASTER_BYTES, control)?;
    install(doc, mask, enabled, control)?;
    let (staged_owner, staged_enabled) = relation(doc, mask, control)?;
    if staged_owner != owner || staged_enabled != enabled {
        return Err(bad("stale-target"));
    }
    poll(control)?;
    dispatch_preflight(session, params, false)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.revision != revision || !active.doc.shares_state(&baseline) {
        return Err(bad("stale-target"));
    }
    session.edit_with_interrupt("Set XCF Filter Mask Enabled", control, move |doc, _| {
        let (current_owner, current_enabled) = relation(doc, mask, control)?;
        if current_owner != owner || current_enabled != previous {
            return Err(bad("stale-target"));
        }
        install(doc, mask, enabled, control)?;
        let (current_owner, current_enabled) = relation(doc, mask, control)?;
        if current_owner != owner || current_enabled != enabled {
            return Err(bad("stale-target"));
        }
        poll(control)
    })?;
    Ok(response)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    session.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    set_enabled_with_interrupt(session, params, &Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: SET,
        label: "Set XCF Filter Mask Enabled",
        menu: &[],
        shortcut: None,
        params: r#"{"maskId":"canonical decimal U64","enabled":bool} → qualified current operative filter empty-selection mask; synchronous, no preview/coalescing"#,
        enabled,
        run,
        journal: false,
    }]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_envelope_preserves_canonical_identity() {
        assert_eq!(envelope(&json!({"maskId":"7", "enabled":false})).unwrap(), (MaskId(7), false));
        for params in [
            json!({"maskId":"07", "enabled":true}),
            json!({"maskId":7, "enabled":true}),
            json!({"maskId":"0", "enabled":true}),
            json!({"maskId":"7", "enabled":0}),
            json!({"maskId":"7", "enabled":true,"coalesce":false}),
        ] {
            assert!(envelope(&params).is_err());
        }
    }
    #[test]
    fn missing_document_cannot_create_history() {
        let mut session = Session::new();
        assert!(set_enabled_with_interrupt(&mut session, &json!({"maskId":"1","enabled":false}), &Interrupt::NONE).is_err());
        assert!(session.active().is_none());
    }
}
