//! Explicit current layer-lock edits; source flags/tattoos and payloads stay inert.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{Document, LayerId, Locks, item_identity::ItemRef, xcf_state::ItemLocks};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

pub const SET: &str = "xcf.item.locks.set";
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: SET.into(), msg: message.into() }
}
#[derive(Clone, Copy)]
struct Patch {
    layer: LayerId,
    content: Option<bool>,
    position: Option<bool>,
    alpha: Option<bool>,
    visibility: Option<bool>,
}
fn parse(params: &Value) -> Result<Patch> {
    let object = params.as_object().ok_or_else(|| bad("expected an object"))?;
    if object.len() > 5 || object.keys().any(|key| !matches!(key.as_str(), "layerId" | "content" | "position" | "alpha" | "visibility")) {
        return Err(bad("unknown lock parameter"));
    }
    let text = object.get("layerId").and_then(Value::as_str).ok_or_else(|| bad("layerId must be a canonical nonzero decimal string"))?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("layerId must be a canonical nonzero decimal string"));
    }
    let raw = text.parse::<u64>().map_err(|_| bad("layerId overflow"))?;
    let flag = |key: &str| -> Result<Option<bool>> {
        object.get(key).map(|value| value.as_bool().ok_or_else(|| bad(format!("{key} must be an explicit boolean")))).transpose()
    };
    let patch = Patch { layer: LayerId(raw), content: flag("content")?, position: flag("position")?, alpha: flag("alpha")?, visibility: flag("visibility")? };
    if patch.content.is_none() && patch.position.is_none() && patch.alpha.is_none() && patch.visibility.is_none() {
        return Err(bad("at least one lock flag is required"));
    }
    Ok(patch)
}
fn context(session: &Session, background: bool) -> Result<()> {
    if session.active().ok_or(EngineError::NoDocument)?.floating.is_some() {
        return Err(bad("transient floating selection is unsupported"));
    }
    if background || session.command_is_background() {
        return Err(bad("lock metadata edits require synchronous dispatch"));
    }
    if session.coalesce_request.is_some() || session.color_restrict.is_some() {
        return Err(bad("inherited edit context is unsupported"));
    }
    Ok(())
}
/// Root dispatch calls this before transient floating normalization or context setup.
/// Cheap strict envelope validation also precedes generic parameter cloning.
pub fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    context(session, background)?;
    parse(params)?;
    Ok(())
}
#[derive(Clone, Copy)]
struct Prepared {
    layer: LayerId,
    item_index: usize,
    imported: ItemLocks,
    native: Locks,
    changed: bool,
}
fn prepare(document: &Document, patch: Patch, control: &Interrupt<'_>) -> Result<Prepared> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    document.validate_xcf_state_with(control).map_err(|error| bad(error.to_string()))?;
    // Query only validates ancestry; locking/unlocking one's own flags does not
    // require content permission and never clears an inherited parent's flag.
    crate::xcf_item_locks::effective_layer_locks(document, patch.layer).map_err(|error| bad(error.to_string()))?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("missing canonical imported state"))?;
    if state.items.len() > 10_000 {
        return Err(bad("item limit"));
    }
    let mut target = None;
    for (index, item) in state.items.iter().enumerate() {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if item.item == ItemRef::Layer(patch.layer) {
            if target.is_some() {
                return Err(bad("duplicate canonical layer metadata"));
            }
            target = Some((index, item.locks));
        }
    }
    let (item_index, previous) = target.ok_or_else(|| bad("missing canonical layer metadata"))?;
    let layer = document.layer(patch.layer).ok_or(EngineError::NoLayer(patch.layer))?;
    let mut imported = previous;
    let mut native = layer.locks;
    if let Some(value) = patch.content {
        imported.content = value;
        native.pixels = value;
    }
    if let Some(value) = patch.position {
        imported.position = value;
        native.position = value;
    }
    if let Some(value) = patch.alpha {
        imported.alpha = value;
        native.transparency = value;
    }
    if let Some(value) = patch.visibility {
        imported.visibility = value;
    }
    // native.all/artboard and every unrequested component remain unchanged.
    Ok(Prepared { layer: patch.layer, item_index, imported, native, changed: imported != previous || native != layer.locks })
}

/// Current metadata transaction with cancellation before staging and publication.
/// Call through registered dispatch for authorization/journal/float safeguards.
pub fn set_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    context(session, false)?;
    let patch = parse(params)?;
    let document = &session.active().ok_or(EngineError::NoDocument)?.doc;
    document.validate_resident().map_err(|error| bad(format!("resident metadata editing required: {error}")))?;
    let planned = prepare(document, patch, control)?;
    // Prepare the bounded response before a commit; no fallible response work follows it.
    let response = json!({"layerId":planned.layer.0.to_string(),"updated":planned.changed,"locks":planned.imported});
    if !planned.changed {
        control.check().map_err(|_| EngineError::Cancelled)?;
        return Ok(response);
    }
    // Raster-only admission for retained backing plus the COW metadata clone.
    // Config/resource/text budgets remain separate; this is not an RSS guarantee.
    let footprint = document
        .resident_raster_footprint_with(control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(format!("retained raster admission: {error}")) })?;
    let peak = footprint.retained_bytes.checked_add(footprint.clone_metadata_bytes).ok_or_else(|| bad("retained raster/COW limit"))?;
    if peak > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(bad("retained raster/COW limit"));
    }
    control.check().map_err(|_| EngineError::Cancelled)?;
    session.edit_with_interrupt("Set Imported Layer Locks", control, |document, _| {
        let current = prepare(document, patch, control)?;
        control.check().map_err(|_| EngineError::Cancelled)?;
        let layer = document.layer_mut(current.layer).ok_or(EngineError::NoLayer(current.layer))?;
        layer.locks = current.native;
        let state = document.xcf_state.as_mut().ok_or_else(|| bad("missing canonical imported state"))?;
        let item = state
            .items
            .get_mut(current.item_index)
            .filter(|item| item.item == ItemRef::Layer(current.layer))
            .ok_or_else(|| bad("canonical layer target changed"))?;
        item.locks = current.imported;
        document.validate_xcf_state_with(control).map_err(|error| bad(error.to_string()))?;
        crate::xcf_item_locks::effective_layer_locks(document, current.layer).map_err(|error| bad(error.to_string()))?;
        control.check().map_err(|_| EngineError::Cancelled)?;
        Ok(())
    })?;
    Ok(response)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    context(session, false).map_err(|error| error.to_string())?;
    session.active().and_then(|active| active.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no canonical imported state".into())
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    set_with_interrupt(session, params, &Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: SET,
        label: "Set Imported Layer Locks",
        menu: &[],
        shortcut: None,
        params: r#"{layerId:canonical nonzero decimal string,content?:bool,position?:bool,alpha?:bool,visibility?:bool} -> {layerId:string,updated:bool,locks:{content,position,alpha,visibility}} (at least one explicit flag; own current layer flags only; preserve inherited parents and source metadata)"#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        ColorMode, Layer, PixelFormat, SampleType, Size, XcfState,
        item_identity::ResourceId,
        xcf_items::{EditableSource, SourceCache, SourceItem},
        xcf_resources::{ResourceKind, ResourceReference},
        xcf_state::ItemState,
    };
    fn fixture() -> (Session, LayerId, LayerId) {
        let mut child = Layer::raster("Own locked child", PixelFormat::RGBA8);
        child.locks.pixels = true;
        child.locks.all = true;
        let child_id = child.id;
        let group = Layer::group("Own parent", vec![child]);
        let group_id = group.id;
        let item = |id, name: &str, parent, locks, tattoo, flags| ItemState {
            item: ItemRef::Layer(id),
            name: name.into(),
            custom_name: true,
            visible: true,
            locks,
            parent,
            source_tattoo: Some(tattoo),
            source_flags: flags,
            raster_bounds: None,
            mask: None,
            composition: None,
        };
        let bytes = vec![1, 2, 3, 4];
        let cache = SourceCache { blake3: blake3::hash(&bytes).to_hex().to_string(), bytes: bytes.into(), width: 1, height: 1, format: PixelFormat::RGBA8 };
        let mut document = Document::new("Own lock metadata", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        document.layers = vec![group];
        document.xcf_state = Some(XcfState {
            items: vec![
                item(group_id, "Own parent", None, ItemLocks { content: true, ..Default::default() }, 41, 0x40),
                item(child_id, "Own locked child", Some(ItemRef::Layer(group_id)), ItemLocks { content: true, ..Default::default() }, 42, 0x80),
            ],
            resources: vec![ResourceReference {
                id: ResourceId(1),
                kind: ResourceKind::LinkedImage,
                name: "Own inert reference".into(),
                collection: None,
                internal: false,
                locator: Some("own:never-read".into()),
                content_hash: None,
                owned_bytes: None,
                resolution: None,
            }],
            source_items: vec![SourceItem {
                owner: ItemRef::Layer(child_id),
                source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: Vec::new(), current: None },
                transform: None,
                cache: Some(cache),
            }],
            ..Default::default()
        });
        document.validate_xcf_state().unwrap();
        let mut session = Session::new();
        session.open_document(document, None).unwrap();
        (session, child_id, group_id)
    }
    #[test]
    fn strict_ids_boolean_subset_and_no_inherited_unlock_parameter() {
        let patch = parse(&json!({"layerId":"18446744073709551615","content":false,"visibility":true})).unwrap();
        assert_eq!(patch.layer, LayerId(u64::MAX));
        assert_eq!(patch.content, Some(false));
        assert_eq!(patch.visibility, Some(true));
        assert_eq!(patch.position, None);
        for params in [
            json!(null),
            json!({"layerId":"7"}),
            json!({"layerId":7,"content":true}),
            json!({"layerId":"07","content":true}),
            json!({"layerId":"0","content":true}),
            json!({"layerId":"18446744073709551616","content":true}),
            json!({"layerId":"7","content":null}),
            json!({"layerId":"7","alpha":1}),
            json!({"layerId":"7","content":false,"parentUnlock":true}),
            json!({"layerId":"7","visibility":true,"coalesce":"locks"}),
        ] {
            assert!(parse(&params).is_err());
        }
    }
    #[test]
    fn own_lock_edits_preserve_parent_source_cache_native_all_and_one_undo_redo() {
        let (mut session, id, parent) = fixture();
        let before = session.active().unwrap().doc.clone();
        let result =
            set_with_interrupt(&mut session, &json!({"layerId":id.0.to_string(),"content":false,"alpha":true,"visibility":true}), &Interrupt::NONE).unwrap();
        assert_eq!(result["updated"], true);
        let after = session.active().unwrap().doc.clone();
        let state = after.xcf_state.as_ref().unwrap();
        let old = before.xcf_state.as_ref().unwrap();
        let current = state.items.iter().find(|item| item.item == ItemRef::Layer(id)).unwrap();
        assert!(!current.locks.content && current.locks.alpha && current.locks.visibility);
        assert_eq!(current.source_flags, 0x80);
        assert_eq!(current.source_tattoo, Some(42));
        assert_eq!(state.source_items, old.source_items);
        assert_eq!(state.resources, old.resources);
        assert_eq!(state.provenance, old.provenance);
        assert_eq!(state.items.iter().find(|item| item.item == ItemRef::Layer(parent)), old.items.iter().find(|item| item.item == ItemRef::Layer(parent)));
        let layer = after.layer(id).unwrap();
        assert!(layer.locks.all && layer.locks.transparency);
        assert!(!layer.locks.pixels);
        assert_eq!(layer.surface(), before.layer(id).unwrap().surface());
        assert!(crate::xcf_item_locks::effective_layer_locks(&after, id).unwrap().imported.content);
        assert_eq!(session.active().unwrap().history.entry_labels().count(), 1);
        let labels = session.active().unwrap().history.entry_labels().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(
            set_with_interrupt(&mut session, &json!({"layerId":id.0.to_string(),"content":false,"alpha":true,"visibility":true}), &Interrupt::NONE).unwrap()["updated"],
            false
        );
        assert!(session.active().unwrap().doc.shares_state(&after));
        assert_eq!(session.active().unwrap().history.entry_labels().map(str::to_string).collect::<Vec<_>>(), labels);
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&before));
        session.execute("edit.redo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&after));
    }
    #[test]
    fn malformed_and_final_cancel_preserve_existing_redo_and_native_state() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let seed = |session: &mut Session, id: LayerId| {
            set_with_interrupt(session, &json!({"layerId":id.0.to_string(),"content":false}), &Interrupt::NONE).unwrap();
            session.execute("edit.undo", json!({})).unwrap();
            assert!(session.active().unwrap().history.can_redo());
        };
        let (mut probe, id, _) = fixture();
        seed(&mut probe, id);
        let polls = AtomicUsize::new(0);
        let count = || {
            polls.fetch_add(1, Ordering::Relaxed);
            false
        };
        set_with_interrupt(&mut probe, &json!({"layerId":id.0.to_string(),"alpha":true}), &Interrupt::cancel_only(&count)).unwrap();
        let final_poll = polls.load(Ordering::Relaxed).checked_sub(1).unwrap();
        let (mut session, id, _) = fixture();
        seed(&mut session, id);
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let labels = session.active().unwrap().history.entry_labels().map(str::to_string).collect::<Vec<_>>();
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        assert!(set_with_interrupt(&mut session, &json!({"layerId":id.0.to_string(),"visibility":"false"}), &Interrupt::NONE).is_err());
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= final_poll;
        assert!(set_with_interrupt(&mut session, &json!({"layerId":id.0.to_string(),"alpha":true}), &Interrupt::cancel_only(&cancel)).is_err());
        let active = session.active().unwrap();
        assert!(active.doc.shares_state(&before));
        assert_eq!(active.revision, revision);
        assert!(active.history.can_redo());
        assert_eq!(active.history.entry_labels().map(str::to_string).collect::<Vec<_>>(), labels);
        assert_eq!(photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap(), native);
        session.execute("edit.redo", json!({})).unwrap();
        assert!(!session.active().unwrap().doc.xcf_state.as_ref().unwrap().items.iter().find(|item| item.item == ItemRef::Layer(id)).unwrap().locks.content);
    }
}
