//! Bounded metadata only: original identities and recorded current verification
//! are separate. This view neither hashes payloads nor evaluates a linked image.
use crate::{DocState, EngineError, Result};
use photocraft_doc::{
    item_identity::ItemRef,
    xcf_items::EditableSource,
    xcf_resources::{ResourceFidelity, ResourceKind},
};
use serde_json::{Value, json};

const LIMIT: usize = 64;

fn error() -> EngineError {
    EngineError::Other("XCF linked inspection metadata/work limit or invalid identity".into())
}

fn hash(value: Option<&str>) -> Result<Option<&str>> {
    if value.is_some_and(|value| value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())) {
        return Err(error());
    }
    Ok(value)
}

fn reference(item: ItemRef) -> Value {
    let kind = match item {
        ItemRef::Layer(_) => "layer",
        ItemRef::Channel(_) => "channel",
        ItemRef::Mask(_) => "mask",
        ItemRef::Path(_) => "path",
    };
    json!({"kind":kind,"id":item.raw().to_string()})
}

pub(super) fn dependencies(document: &DocState) -> Result<Value> {
    let Some(state) = document.doc.xcf_state.as_ref() else { return Ok(Value::Null) };
    let selected = document.selection_ids();
    // Bounds precede iteration, lookups and JSON copies. Only two fixed-size
    // pages are returned; xcf.inspect provides further paginated source metadata.
    if state.resources.len() > 1000 || state.source_items.len() > 10000 || selected.len() > 10000 {
        return Err(error());
    }
    let mut resources = Vec::new();
    let mut bindings = Vec::new();
    resources.try_reserve(LIMIT).map_err(|_| error())?;
    bindings.try_reserve(LIMIT).map_err(|_| error())?;
    // Always include the operative linked owner and its dependency even when
    // they occur beyond the first page in the retained source ordering.
    let is_active = |owner| matches!(owner, ItemRef::Layer(id) if document.active_layer == Some(id));
    let active_resource = state.source_items.iter().find_map(|source| match &source.source {
        EditableSource::Linked { resource, .. } if is_active(source.owner) => Some(*resource),
        _ => None,
    });
    let resource_total = state.resources.iter().filter(|resource| resource.kind == ResourceKind::LinkedImage).count();
    let resource_order = state
        .resources
        .iter()
        .filter(|resource| Some(resource.id) == active_resource)
        .chain(state.resources.iter().filter(|resource| Some(resource.id) != active_resource));
    for resource in resource_order.filter(|resource| resource.kind == ResourceKind::LinkedImage).take(LIMIT) {
        let original_hash = hash(resource.content_hash.as_deref())?;
        let current_hash = hash(resource.resolution.as_ref().map(|resolution| resolution.payload_hash.as_str()))?;
        let name: String = resource.name.chars().take(256).collect();
        resources.push(json!({
            "resourceId":resource.id.0.to_string(),"kind":resource.kind,"name":name,
            "nameTruncated":resource.name.chars().nth(256).is_some(),
            "original":{"contentHash":original_hash,"internal":resource.internal,"hasLocator":resource.locator.is_some()},
            "current":{"ownedByteLength":resource.owned_bytes.as_ref().map(|bytes|bytes.len()),
                "recordedByteIdentity":resource.resolution.as_ref().map(|resolution|resolution.fidelity),
                "recordedPayloadHash":current_hash,
                "substitutionRequiresReducedAssessment":resource.resolution.as_ref().is_some_and(|resolution|resolution.fidelity==ResourceFidelity::Substituted)},
            "accessGranted":false
        }));
    }
    let binding_total = state.source_items.iter().filter(|source| matches!(source.source, EditableSource::Linked { .. })).count();
    let source_order =
        state.source_items.iter().filter(|source| is_active(source.owner)).chain(state.source_items.iter().filter(|source| !is_active(source.owner)));
    for source in source_order.filter(|source| matches!(source.source, EditableSource::Linked { .. })).take(LIMIT) {
        let EditableSource::Linked { resource, last_verified_hash, current, .. } = &source.source else { return Err(error()) };
        let target = state.resources.iter().find(|target| target.id == *resource).ok_or_else(error)?;
        if target.kind != ResourceKind::LinkedImage {
            return Err(error());
        }
        let verified = hash(last_verified_hash.as_deref())?;
        let active = matches!(source.owner, ItemRef::Layer(id) if document.active_layer==Some(id));
        let selected = matches!(source.owner, ItemRef::Layer(id) if selected.contains(&id));
        bindings.push(json!({
            "owner":reference(source.owner),"resourceId":resource.0.to_string(),"active":active,"selected":selected,
            "original":{"hasTransform":source.transform.is_some(),"cacheHash":hash(source.cache.as_ref().map(|cache|cache.blake3.as_str()))?,
                "cacheByteLength":source.cache.as_ref().map(|cache|cache.bytes.len())},
            "current":{"lastVerifiedHash":verified,"geometry":current.as_ref().map(crate::xcf_link_geometry_cmds::current_geometry_json)}
        }));
    }
    Ok(json!({
        "resourceTotal":resource_total,"bindingTotal":binding_total,"limit":LIMIT,
        "ordering":"active linked owner and resource first, then retained order",
        "resourcesTruncated":resource_total>LIMIT,"bindingsTruncated":binding_total>LIMIT,
        "resources":resources,"bindings":bindings,
        "inspectCommand":"xcf.inspect","assessmentCommand":"xcf.assess",
        "verification":"recorded metadata only; no payload rehash or current semantic qualification",
        "accessGranted":false
    }))
}
