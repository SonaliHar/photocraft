//! Read-only inspection of retained, inert XCF resource identities.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::xcf_resources::{ResourceKind, ResourceReference};
use serde_json::{Value, json};

const MAX_RESOURCES: usize = 1000;
const MAX_FIELD: usize = 16 << 10;
const MAX_OUTPUT: usize = 1 << 20;
fn error(message: &str) -> EngineError {
    EngineError::Other(message.into())
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    session.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn resource_id(value: &Value) -> Result<u64> {
    let text = value.as_str().ok_or_else(|| error("resource must be an exact decimal-string U64 identity"))?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(error("resource must be a nonzero canonical decimal-string U64 identity"));
    }
    text.parse::<u64>().map_err(|_| error("resource identity exceeds U64"))
}
fn output_cost(resource: &ResourceReference) -> Result<usize> {
    let mut bytes = 512usize;
    for text in
        [Some(resource.name.as_str()), resource.collection.as_deref(), resource.locator.as_deref(), resource.content_hash.as_deref()].into_iter().flatten()
    {
        if text.len() > MAX_FIELD || text.contains('\0') {
            return Err(error("XCF resource inspection field exceeds bounded UTF-8 output policy"));
        }
        // JSON can escape each UTF-8 byte to at most six bytes. Never truncate identifiers.
        bytes = bytes
            .checked_add(text.len().checked_mul(6).ok_or_else(|| error("XCF resource output limit"))?)
            .ok_or_else(|| error("XCF resource output limit"))?;
    }
    Ok(bytes)
}
fn row(resource: &ResourceReference, activated: Option<bool>) -> Value {
    let kind = match resource.kind {
        ResourceKind::Font => "font",
        ResourceKind::Pattern => "pattern",
        ResourceKind::Gradient => "gradient",
        ResourceKind::Brush => "brush",
        ResourceKind::Palette => "palette",
        ResourceKind::ColorProfile => "color-profile",
        ResourceKind::LinkedImage => "linked-image",
    };
    let status = match &resource.resolution {
        Some(resolution) => match resolution.fidelity {
            photocraft_doc::xcf_resources::ResourceFidelity::Exact => "exact-resolved-bytes",
            photocraft_doc::xcf_resources::ResourceFidelity::Substituted => "substituted-resolved-bytes",
        },
        None => match (&resource.owned_bytes, &resource.content_hash) {
            (Some(_), Some(_)) => "exact-hash-bound",
            (Some(_), None) => "owned-unverified",
            (None, _) => "unresolved",
        },
    };
    json!({"id":resource.id.0.to_string(), "kind":kind, "name":resource.name, "collection":resource.collection,
        "internal":resource.internal, "locator":resource.locator, "contentHash":resource.content_hash,
        "ownedByteLength":resource.owned_bytes.as_ref().map_or(0,|bytes|bytes.len()), "status":status, "inert":activated != Some(true),
        "activated":activated, "identityResolved":resource.resolution.is_some(),
        "resolution":resource.resolution})
}
fn activation(document: &photocraft_doc::Document, resource: &ResourceReference, work: &mut u64) -> Option<bool> {
    use photocraft_doc::xcf_items::EditableSource;
    if resource.kind != ResourceKind::LinkedImage || resource.resolution.is_none() {
        return Some(false);
    }
    let state = document.xcf_state.as_ref()?;
    let mut found = false;
    for source in &state.source_items {
        let EditableSource::Linked { resource: id, last_verified_hash, .. } = &source.source else {
            continue;
        };
        if *id != resource.id {
            continue;
        }
        found = true;
        if last_verified_hash.as_deref() != resource.resolution.as_ref().map(|r| r.payload_hash.as_str()) {
            return Some(false);
        }
        let pixels = state
            .items
            .iter()
            .find(|item| item.item == source.owner)
            .and_then(|item| item.raster_bounds)
            .and_then(|bounds| u64::from(bounds.width()).checked_mul(u64::from(bounds.height())))?;
        let remaining = work.checked_sub(pixels)?;
        *work = remaining;
        if photocraft_compose::gimp_link_items::qualify_current_source(document, source, &photocraft_raster::Interrupt::NONE).is_err() {
            return Some(false);
        }
    }
    Some(found)
}
fn inspect(session: &mut Session, params: &Value) -> Result<Value> {
    let params = params.as_object().ok_or_else(|| error("XCF resource inspection parameters must be an object"))?;
    if params.keys().any(|key| key != "resource") {
        return Err(error("unknown XCF resource inspection parameter"));
    }
    let selected = params.get("resource").map(resource_id).transpose()?;
    let document = &session.active().ok_or(EngineError::NoDocument)?.doc;
    document.validate_xcf_state().map_err(|e| error(&format!("invalid retained XCF source state: {e}")))?;
    let Some(state) = &document.xcf_state else {
        return if selected.is_some() { Err(error("unknown XCF resource identity")) } else { Ok(json!({"resources":[],"count":0})) };
    };
    let mut count = 0usize;
    let mut bytes = 64usize;
    for resource in state.resources.iter().filter(|r| selected.is_none_or(|id| r.id.0 == id)) {
        count = count.checked_add(1).ok_or_else(|| error("XCF resource output limit"))?;
        bytes = bytes.checked_add(output_cost(resource)?).ok_or_else(|| error("XCF resource output limit"))?;
        if count > MAX_RESOURCES || bytes > MAX_OUTPUT {
            return Err(error("XCF resource inspection output limit exceeded"));
        }
    }
    if selected.is_some() && count != 1 {
        return Err(error("unknown or duplicate XCF resource identity"));
    }
    let mut rows = Vec::new();
    let mut activation_work = 4_000_000u64;
    rows.try_reserve_exact(count).map_err(|_| error("XCF resource inspection allocation denied"))?;
    for resource in state.resources.iter().filter(|r| selected.is_none_or(|id| r.id.0 == id)) {
        rows.push(row(resource, activation(document, resource, &mut activation_work)));
    }
    Ok(json!({"resources":rows,"count":count}))
}
fn upload(session: &mut Session, params: &Value) -> Result<Value> {
    use crate::xcf_resources::{RecoveryPolicy, ServiceResourceBytes, recover_authorized_bytes};
    let object = params.as_object().ok_or_else(|| error("resource upload requires an object"))?;
    if object.keys().any(|key| !["resource", "payloadHex", "policy"].contains(&key.as_str())) {
        return Err(error("unknown XCF resource upload parameter"));
    }
    let id = resource_id(params.get("resource").ok_or_else(|| error("resource upload requires an identity"))?)?;
    let policy = match params.get("policy").and_then(Value::as_str) {
        Some("exact") => RecoveryPolicy::Exact,
        Some("substitute") => RecoveryPolicy::AllowSubstitution,
        _ => return Err(error("resource upload requires exact or substitute policy")),
    };
    let payload = params.get("payloadHex").and_then(Value::as_str).ok_or_else(|| error("resource upload requires hexadecimal supplied bytes"))?;
    if payload.len() > 16 << 20 || !payload.len().is_multiple_of(2) || !payload.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error("resource upload exceeds its 8 MiB decoded bound or contains invalid hexadecimal"));
    }
    let kind = session
        .active()
        .and_then(|active| active.doc.xcf_state.as_ref())
        .and_then(|state| state.resources.iter().find(|entry| entry.id.0 == id))
        .map(|entry| entry.kind.clone())
        .ok_or_else(|| error("unknown XCF resource identity"))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(payload.len() / 2).map_err(|_| error("resource upload allocation denied"))?;
    for pair in payload.as_bytes().as_chunks::<2>().0 {
        let nibble = |byte: u8| -> Result<u8> {
            match byte {
                b'0'..=b'9' => Ok(byte - b'0'),
                b'a'..=b'f' => Ok(byte - b'a' + 10),
                b'A'..=b'F' => Ok(byte - b'A' + 10),
                _ => Err(error("invalid hexadecimal")),
            }
        };
        let high = pair.first().copied().ok_or_else(|| error("invalid hexadecimal pair"))?;
        let low = pair.get(1).copied().ok_or_else(|| error("invalid hexadecimal pair"))?;
        bytes.push(nibble(high)? * 16 + nibble(low)?);
    }
    let input = ServiceResourceBytes::from_authorized_service(kind, bytes, true).map_err(|e| error(&e.to_string()))?;
    let fidelity = recover_authorized_bytes(session, photocraft_doc::item_identity::ResourceId(id), input, policy, &photocraft_raster::Interrupt::NONE)?;
    Ok(json!({"resource":id.to_string(),"byteIdentity":fidelity,"activated":false}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "xcf.resource.inspect",
            label: "Inspect XCF Resources",
            menu: &[],
            shortcut: None,
            params: r#"{"resource":"nonzero canonical decimal U64"?} → bounded retained resource metadata; read-only, no resource loading or byte payload. Hash-bound owned bytes do not imply a resolved font or activated locator."#,
            enabled,
            run: inspect,
            journal: false,
        },
        CommandSpec {
            id: "xcf.resource.upload",
            label: "Upload XCF Resource Bytes",
            menu: &[],
            shortcut: None,
            params: r#"{"resource":"canonical decimal U64","payloadHex":"explicit supplied bytes, max 8 MiB","policy":"exact|substitute"} → undoable byte-identity recovery; no locator read or render activation"#,
            enabled,
            run: upload,
            journal: false,
        },
    ]
}
