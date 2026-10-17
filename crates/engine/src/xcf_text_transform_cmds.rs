//! Explicit current text naming metadata; no unqualified text transform authoring.
//! Scale/rotation authoring is not inferred from the existing font-size candidate.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{Document, LayerId, item_identity::ItemRef, xcf_items::EditableSource};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};

const ID: &str = "xcf.text.name.set";
const MAX_NAME: usize = 4096;
fn bad(message: &str) -> EngineError {
    EngineError::Other(format!("{ID}.{message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: &str) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(message) }
}
struct Request<'a> {
    document: u64,
    revision: u64,
    owner: LayerId,
    name: &'a str,
    custom: bool,
}
fn decimal(params: &Value, key: &str, allow_zero: bool) -> Result<u64> {
    let text = params.get(key).and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return Err(bad("parameters"));
    }
    let value = text.parse::<u64>().map_err(|_| bad("parameters"))?;
    if value == 0 && !allow_zero {
        return Err(bad("parameters"));
    }
    Ok(value)
}
fn request(params: &Value) -> Result<Request<'_>> {
    let fields = params.as_object().ok_or_else(|| bad("parameters"))?;
    const KEYS: [&str; 5] = ["documentId", "expectedRevision", "layerId", "name", "customName"];
    if fields.len() != KEYS.len() || fields.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(bad("parameters"));
    }
    let name = params.get("name").and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if name.is_empty() || name.len() > MAX_NAME || name.contains('\0') {
        return Err(bad("name"));
    }
    Ok(Request {
        document: decimal(params, "documentId", false)?,
        revision: decimal(params, "expectedRevision", true)?,
        owner: LayerId(decimal(params, "layerId", false)?),
        name,
        custom: params.get("customName").and_then(Value::as_bool).ok_or_else(|| bad("parameters"))?,
    })
}
fn context(session: &Session, background: bool) -> Result<()> {
    if background
        || session.command_is_background()
        || session.command_has_pending_job()
        || session.has_jobs()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
    {
        return Err(bad("context"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.floating.is_some() || state.doc.accounted().is_some() || state.revision == u64::MAX {
        return Err(bad("context"));
    }
    Ok(())
}
fn prepare(document: &Document, request: &Request<'_>, control: &Interrupt<'_>) -> Result<(usize, bool)> {
    poll(control)?;
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("missing-source"))?;
    if state.items.len() > 10_000 || state.source_items.len() > 10_000 {
        return Err(bad("limit"));
    }
    // This existing checked ancestry census bounds the native tree before layer()
    // access and preserves both imported and native inherited content locks.
    crate::xcf_item_locks::effective_layer_locks(document, request.owner)?.require(crate::xcf_item_locks::EditKind::Content)?;
    document.validate_xcf_state_with(control).map_err(|_| controlled(control, "invalid-source"))?;
    let mut sources = state.source_items.iter().filter(|source| source.owner == ItemRef::Layer(request.owner));
    let source = sources.next().ok_or_else(|| bad("missing-source"))?;
    if sources.next().is_some() || !matches!(source.source, EditableSource::Text { .. }) {
        return Err(bad("unsupported"));
    }
    let mut items = state.items.iter().enumerate().filter(|(_, item)| item.item == ItemRef::Layer(request.owner));
    let (index, item) = items.next().ok_or_else(|| bad("missing-item"))?;
    if items.next().is_some() {
        return Err(bad("invalid-source"));
    }
    let layer = document.layer(request.owner).ok_or(EngineError::NoLayer(request.owner))?;
    if layer.is_group() || item.name != layer.name {
        return Err(bad("invalid-current-name"));
    }
    Ok((index, layer.name != request.name || item.custom_name != request.custom))
}
/// Dedicated dispatcher guard before parameter cloning, float normalization,
/// job routing, coalescing, or transient command context installation.
pub fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    let req = request(params)?;
    context(session, background)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.id.0 != req.document || state.revision != req.revision {
        return Err(bad("stale-target"));
    }
    prepare(&state.doc, &req, &Interrupt::NONE)?;
    Ok(())
}
fn owned(text: &str) -> Result<String> {
    let mut output = String::new();
    output.try_reserve_exact(text.len()).map_err(|_| bad("allocation"))?;
    output.push_str(text);
    Ok(output)
}
/// Synchronize current literal names and explicit custom-name metadata only.
/// customName=false does not infer a name from the original AST or regenerate
/// pixels. Metadata editing needs no font activation or profile replacement;
/// existing resource/fidelity/render eligibility remains independently assessed.
pub fn set_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    poll(control)?;
    let req = request(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let (_, changed) = prepare(&state.doc, &req, control)?;
    let response = json!({"documentId":req.document.to_string(),"layerId":req.owner.0.to_string(),"name":req.name,"customName":req.custom,"updated":changed});
    if !changed {
        poll(control)?;
        return Ok(response);
    }
    let snapshot = state.doc.downgrade();
    let footprint = state.doc.resident_raster_footprint_with(control).map_err(|_| controlled(control, "limit"))?;
    let peak = footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes.checked_mul(2).ok_or_else(|| bad("limit"))?)
        .and_then(|n| n.checked_add((req.name.len() as u64).checked_mul(3)?))
        .ok_or_else(|| bad("limit"))?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(bad("limit"));
    }
    let layer_name = owned(req.name)?;
    let item_name = owned(req.name)?;
    poll(control)?;
    context(session, false)?;
    let current = session.active().ok_or(EngineError::NoDocument)?;
    let original = snapshot.upgrade().ok_or_else(|| bad("stale-target"))?;
    if current.doc.id.0 != req.document || current.revision != req.revision || !original.shares_state(&current.doc) {
        return Err(bad("stale-target"));
    }
    prepare(&current.doc, &req, control)?;
    session.edit_with_interrupt("Set Current XCF Text Name", control, move |document, _| {
        if document.id.0 != req.document {
            return Err(bad("stale-target"));
        }
        let (index, _) = prepare(document, &req, control)?;
        document.layer_mut(req.owner).ok_or(EngineError::NoLayer(req.owner))?.name = layer_name;
        let item = document
            .xcf_state
            .as_mut()
            .and_then(|state| state.items.get_mut(index))
            .filter(|item| item.item == ItemRef::Layer(req.owner))
            .ok_or_else(|| bad("stale-target"))?;
        // Set both current names before reconciliation so it cannot override an
        // explicitly requested false flag merely because native name differs.
        item.name = item_name;
        item.custom_name = req.custom;
        document.validate_xcf_state_with(control).map_err(|_| controlled(control, "invalid-source"))?;
        poll(control)?;
        Ok(())
    })?;
    Ok(response)
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    set_with_interrupt(session, params, &Interrupt::NONE)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    context(session, false).map_err(|e| e.to_string())?;
    session.active().and_then(|s| s.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no canonical imported text state".into())
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Set Current XCF Text Name",
        menu: &[],
        shortcut: None,
        params: r#"Exact {documentId:canonical nonzero U64 string,expectedRevision:canonical U64 string,layerId:canonical nonzero U64 string,name:nonempty exact UTF8 <=4096 bytes without NUL,customName:bool}. One current metadata undo; no source AST/cache/font/profile or auto-name inference."#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params() -> Value {
        json!({"documentId":"1","expectedRevision":"0","layerId":"2","name":"literal name","customName":false})
    }
    #[test]
    fn bounded_canonical_metadata_envelope() {
        assert!(request(&params()).is_ok());
        for (field, value) in [
            ("layerId", json!(2)),
            ("layerId", json!("02")),
            ("customName", json!("false")),
            ("name", json!("bad\0name")),
            ("name", json!("x".repeat(MAX_NAME + 1))),
        ] {
            let mut p = params();
            p[field] = value;
            assert!(request(&p).is_err());
        }
        let mut p = params();
        p["extra"] = json!(true);
        assert!(request(&p).is_err());
    }
    #[test]
    fn empty_and_background_refuse_without_install() {
        let session = Session::new();
        assert!(dispatch_preflight(&session, &params(), true).is_err());
        assert!(dispatch_preflight(&session, &params(), false).is_err());
        assert!(session.documents().is_empty());
        assert!(session.journal.is_empty());
    }
}
