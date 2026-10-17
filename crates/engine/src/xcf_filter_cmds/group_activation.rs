//! Activation only for the existing isolated-small-auto GroupGaussian consumer.
//! Source policy/configuration/cache/resource records are not rewritten.
use super::*;
use photocraft_doc::{LayerId, xcf_filters::FilterStack};
const EPOCH: &str = "xcf22-isolated-small-auto-v1";
fn target<'a>(doc: &'a Document, id: FilterId, control: &Interrupt<'_>) -> Result<(LayerId, &'a FilterStack, &'a FilterNode)> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad(SET, "missing group filter state"))?;
    if state.filter_stacks.len() > 10_000 || state.items.len() > 10_000 {
        return Err(bad(SET, "group filter inventory limit"));
    }
    let mut total = 0usize;
    for stack in &state.filter_stacks {
        control.check().map_err(|_| EngineError::Cancelled)?;
        total = total.checked_add(stack.nodes.len()).filter(|count| *count <= 10_000).ok_or_else(|| bad(SET, "group filter node limit"))?;
    }
    let mut found = None;
    for stack in &state.filter_stacks {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if stack.nodes.len() > 1024 {
            return Err(bad(SET, "group filter stack limit"));
        }
        for node in &stack.nodes {
            control.check().map_err(|_| EngineError::Cancelled)?;
            if node.id == id {
                if found.is_some() {
                    return Err(bad(SET, "duplicate group filter identity"));
                }
                let ItemRef::Layer(owner) = stack.owner else {
                    return Err(bad(SET, "group filter owner"));
                };
                found = Some((owner, stack, node));
            }
        }
    }
    let (owner, stack, node) = found.ok_or_else(|| bad(SET, "missing group filter"))?;
    if !doc.layer(owner).is_some_and(|layer| matches!(layer.content, LayerContent::Group(_)))
        || stack.nodes.len() != 1
        || node.operation.name != "gegl:gaussian-blur"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != EPOCH
    {
        return Err(bad(SET, "group activation requires exact isolated-small-auto Gaussian singleton"));
    }
    let locks = crate::xcf_item_locks::effective_layer_locks(doc, owner).map_err(|e| bad(SET, e.to_string()))?;
    if locks.native.all || locks.native.pixels || locks.imported.content {
        return Err(bad(SET, "group filter inherited content is locked"));
    }
    Ok((owner, stack, node))
}
fn install(doc: &mut Document, owner: LayerId, id: FilterId, previous: bool, enabled: bool, revision: u64) -> Result<()> {
    let state = doc.xcf_state.as_mut().ok_or_else(|| bad(SET, "missing staged group filter"))?;
    let stack = state.filter_stacks.iter_mut().find(|stack| stack.owner == ItemRef::Layer(owner)).ok_or_else(|| bad(SET, "missing staged group stack"))?;
    if stack.revision != revision || stack.nodes.len() != 1 {
        return Err(bad(SET, "stale group filter stack"));
    }
    let node = stack.nodes.first_mut().filter(|node| node.id == id && node.enabled == previous).ok_or_else(|| bad(SET, "stale group filter"))?;
    let next = revision.checked_add(1).ok_or_else(|| bad(SET, "group filter revision overflow"))?;
    node.enabled = enabled;
    stack.revision = next;
    Ok(())
}
pub(super) fn set(
    session: &mut Session,
    params: &Value,
    snapshot: &crate::DocumentSnapshot,
    revision: u64,
    id: FilterId,
    control: &Interrupt<'_>,
) -> Result<Value> {
    // A supplied opacity/argument field is refused even if equal/empty: the group
    // lane does not grant generic scalar/configuration authoring authority.
    let fields = object(SET, params, &["filterId", "enabled"])?;
    if fields.len() != 2 {
        return Err(bad(SET, "group activation requires filterId and enabled only"));
    }
    let enabled = params.get("enabled").and_then(Value::as_bool).ok_or_else(|| bad(SET, "group enabled must be boolean"))?;
    let (owner, stack, node) = target(snapshot, id, control)?;
    let previous = node.enabled;
    let stack_revision = stack.revision;
    let locks = crate::xcf_item_locks::effective_layer_locks(snapshot, owner).map_err(|e| bad(SET, e.to_string()))?;
    if previous != enabled && locks.imported.visibility {
        return Err(bad(SET, "group filter inherited activation is locked"));
    }
    // Validate the actual state, not an invented enabled descriptor. In particular
    // disabled-only opaque tuples may stay disabled but cannot silently be normalized.
    validate_current_pipeline_with_interrupt(snapshot, control)?;
    let response = json!({"filterId":id.0.to_string(),"updated":previous!=enabled});
    if previous == enabled {
        final_set_context(session, snapshot, revision, params, control)?;
        return Ok(response);
    }
    let mut staged: Document = (**snapshot).clone();
    install(&mut staged, owner, id, previous, enabled, stack_revision)?;
    staged.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(SET, e.to_string()) })?;
    validate_current_pipeline_with_interrupt(&staged, control)?;
    final_set_context(session, snapshot, revision, params, control)?;
    let mut staged = Some(staged);
    session.edit_with_interrupt("Set GIMP Group Filter Enabled", control, move |document, _| {
        control.check().map_err(|_| EngineError::Cancelled)?;
        let (current_owner, current_stack, current) = target(document, id, control)?;
        if crate::xcf_item_locks::effective_layer_locks(document, current_owner).map_err(|e| bad(SET, e.to_string()))?.imported.visibility {
            return Err(bad(SET, "group filter inherited activation is locked"));
        }
        if current_owner != owner || current_stack.revision != stack_revision || current.enabled != previous {
            return Err(bad(SET, "stale group filter target"));
        }
        let replacement = staged.take().ok_or_else(|| bad(SET, "group filter stage already consumed"))?;
        control.check().map_err(|_| EngineError::Cancelled)?;
        *document = replacement;
        Ok(())
    })?;
    Ok(response)
}
