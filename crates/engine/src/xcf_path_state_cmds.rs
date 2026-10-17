//! Current saved-path names/flags, without speculative visibility behavior.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document,
    item_identity::{ItemRef, PathId},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};

const ID: &str = "xcf.path.name.set";
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
    path: PathId,
    name: &'a str,
    custom: bool,
}
fn decimal(params: &Value, key: &str, zero: bool) -> Result<u64> {
    let text = params.get(key).and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return Err(bad("parameters"));
    }
    let number = text.parse::<u64>().map_err(|_| bad("parameters"))?;
    if number == 0 && !zero {
        return Err(bad("parameters"));
    }
    Ok(number)
}
fn request(params: &Value) -> Result<Request<'_>> {
    let fields = params.as_object().ok_or_else(|| bad("parameters"))?;
    const KEYS: [&str; 5] = ["documentId", "expectedRevision", "pathId", "name", "customName"];
    if fields.len() != KEYS.len() || fields.keys().any(|k| !KEYS.contains(&k.as_str())) {
        return Err(bad("parameters"));
    }
    let name = params.get("name").and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if name.is_empty() || name.len() > MAX_NAME || name.contains('\0') || crate::vector_cmds::is_work(Some(name)) {
        return Err(bad("name"));
    }
    Ok(Request {
        document: decimal(params, "documentId", false)?,
        revision: decimal(params, "expectedRevision", true)?,
        path: PathId(decimal(params, "pathId", false)?),
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
/// Saved path geometry is immutable here, but will be cloned by the transaction.
/// Charge the aggregate native path collection before any cloning/validation.
fn path_bounds(document: &Document, control: &Interrupt<'_>) -> Result<u64> {
    if document.paths.len() > 10_000 {
        return Err(bad("limit"));
    }
    let mut visits = document.paths.len();
    let mut bytes = (document.paths.len() as u64).checked_mul(std::mem::size_of::<photocraft_doc::NamedPath>() as u64).ok_or_else(|| bad("limit"))?;
    for named in &document.paths {
        if named.name.len() > 1 << 20 {
            return Err(bad("limit"));
        }
        bytes = bytes.checked_add(named.name.len() as u64).ok_or_else(|| bad("limit"))?;
    }
    if let Some(clipping) = &document.clipping_path {
        if clipping.name.len() > 1 << 20 {
            return Err(bad("limit"));
        }
        bytes = bytes.checked_add(clipping.name.len() as u64).ok_or_else(|| bad("limit"))?;
    }
    for path in document.paths.iter().map(|named| &named.path).chain(document.work_path.iter()) {
        poll(control)?;
        visits = visits.checked_add(path.subpaths.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad("limit"))?;
        bytes = bytes
            .checked_add((path.subpaths.len() as u64).checked_mul(std::mem::size_of::<photocraft_doc::Subpath>() as u64).ok_or_else(|| bad("limit"))?)
            .ok_or_else(|| bad("limit"))?;
        for subpath in &path.subpaths {
            visits = visits.checked_add(subpath.knots.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad("limit"))?;
            bytes = bytes
                .checked_add((subpath.knots.len() as u64).checked_mul(std::mem::size_of::<photocraft_doc::Knot>() as u64).ok_or_else(|| bad("limit"))?)
                .ok_or_else(|| bad("limit"))?;
        }
    }
    if bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(bad("limit"));
    }
    Ok(bytes)
}
struct Prepared {
    path: usize,
    item: usize,
    changed: bool,
    clipping: bool,
    path_clone_bytes: u64,
}
fn prepare(document: &Document, req: &Request<'_>, control: &Interrupt<'_>) -> Result<Prepared> {
    let path_clone_bytes = path_bounds(document, control)?;
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("missing-state"))?;
    if state.items.len() > 10_000 {
        return Err(bad("limit"));
    }
    document.validate_xcf_state_with(control).map_err(|_| controlled(control, "invalid-state"))?;
    let mut paths = document.paths.iter().enumerate().filter(|(_, path)| path.id == req.path);
    let (path_index, path) = paths.next().ok_or_else(|| bad("missing-path"))?;
    if paths.next().is_some() {
        return Err(bad("invalid-path-identity"));
    }
    let mut items = state.items.iter().enumerate().filter(|(_, item)| item.item == ItemRef::Path(req.path));
    let (item_index, item) = items.next().ok_or_else(|| bad("missing-item"))?;
    if items.next().is_some() || item.name != path.name {
        return Err(bad("invalid-current-name"));
    }
    if state.effective_locks(ItemRef::Path(req.path)).map_err(|_| bad("invalid-locks"))?.content {
        return Err(bad("locked"));
    }
    if document.paths.iter().any(|other| other.id != req.path && other.name == req.name) {
        return Err(bad("name-conflict"));
    }
    let clipping = document.clipping_path.as_ref().is_some_and(|clip| clip.name == path.name) && path.name != req.name;
    if clipping && document.paths.iter().filter(|other| other.name == path.name).count() != 1 {
        return Err(bad("ambiguous-clipping-path"));
    }
    Ok(Prepared { path: path_index, item: item_index, changed: path.name != req.name || item.custom_name != req.custom, clipping, path_clone_bytes })
}
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
fn owned(value: &str) -> Result<String> {
    let mut result = String::new();
    result.try_reserve_exact(value.len()).map_err(|_| bad("allocation"))?;
    result.push_str(value);
    Ok(result)
}
/// Update both operative names before reconciliation; explicit false is not
/// silently replaced by generic rename's custom=true. No automatic name is
/// inferred from geometry/source metadata and no visibility semantics are added.
pub fn set_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    poll(control)?;
    let req = request(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let prepared = prepare(&state.doc, &req, control)?;
    let response =
        json!({"documentId":req.document.to_string(),"pathId":req.path.0.to_string(),"name":req.name,"customName":req.custom,"updated":prepared.changed});
    if !prepared.changed {
        poll(control)?;
        return Ok(response);
    }
    let weak = state.doc.downgrade();
    let footprint = state.doc.resident_raster_footprint_with(control).map_err(|_| controlled(control, "limit"))?;
    let peak = footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes.checked_mul(2).ok_or_else(|| bad("limit"))?)
        .and_then(|n| n.checked_add(prepared.path_clone_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add((req.name.len() as u64).checked_mul(4)?))
        .ok_or_else(|| bad("limit"))?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(bad("limit"));
    }
    let path_name = owned(req.name)?;
    let item_name = owned(req.name)?;
    let clipping_name = if prepared.clipping { Some(owned(req.name)?) } else { None };
    poll(control)?;
    context(session, false)?;
    let current = session.active().ok_or(EngineError::NoDocument)?;
    let original = weak.upgrade().ok_or_else(|| bad("stale-target"))?;
    if current.doc.id.0 != req.document || current.revision != req.revision || !original.shares_state(&current.doc) {
        return Err(bad("stale-target"));
    }
    prepare(&current.doc, &req, control)?;
    session.edit_with_interrupt("Set Current XCF Saved Path Name", control, move |document, _| {
        if document.id.0 != req.document {
            return Err(bad("stale-target"));
        }
        let now = prepare(document, &req, control)?;
        if now.clipping != prepared.clipping {
            return Err(bad("stale-target"));
        }
        document.paths.get_mut(now.path).filter(|path| path.id == req.path).ok_or_else(|| bad("stale-target"))?.name = path_name;
        let item = document
            .xcf_state
            .as_mut()
            .and_then(|state| state.items.get_mut(now.item))
            .filter(|item| item.item == ItemRef::Path(req.path))
            .ok_or_else(|| bad("stale-target"))?;
        item.name = item_name;
        item.custom_name = req.custom;
        if let Some(name) = clipping_name {
            document.clipping_path.as_mut().ok_or_else(|| bad("stale-target"))?.name = name;
        }
        document.validate_xcf_state_with(control).map_err(|_| controlled(control, "invalid-state"))?;
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
    session.active().and_then(|state| state.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no canonical saved path state".into())
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Set Current XCF Saved Path Name",
        menu: &[],
        shortcut: None,
        params: r#"Exact {documentId:canonical nonzero U64 string,expectedRevision:canonical U64 string,pathId:canonical nonzero U64 string,name:exact nonempty UTF8<=4096 without NUL or reserved work-path alias,customName:bool}. Synchronize current saved-path/item names and clipping association; preserve geometry/raw/provenance/flags. No visibility or auto-name inference."#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params() -> Value {
        json!({"documentId":"1","expectedRevision":"0","pathId":"2","name":"literal saved name","customName":false})
    }
    #[test]
    fn exact_saved_path_envelope_rejects_work_aliases_and_hostile_ids() {
        assert!(request(&params()).is_ok());
        for (key, value) in [
            ("pathId", json!(2)),
            ("pathId", json!("02")),
            ("customName", json!("false")),
            ("name", json!("Work Path")),
            ("name", json!("wOrK")),
            ("name", json!("bad\0name")),
            ("name", json!("x".repeat(MAX_NAME + 1))),
        ] {
            let mut p = params();
            p[key] = value;
            assert!(request(&p).is_err());
        }
        let mut p = params();
        p["extra"] = json!(true);
        assert!(request(&p).is_err());
    }
    #[test]
    fn empty_dispatch_refuses_without_install_or_history() {
        let session = Session::new();
        assert!(dispatch_preflight(&session, &params(), false).is_err());
        assert!(dispatch_preflight(&session, &params(), true).is_err());
        assert!(session.documents().is_empty());
        assert!(session.journal.is_empty());
    }
}
