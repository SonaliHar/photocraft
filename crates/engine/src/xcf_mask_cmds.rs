//! Current canonical layer-mask enablement; filter/channel masks remain unsupported.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{Document, LayerId, item_identity::{ItemRef, MaskId}};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

fn bad(code: &str) -> EngineError { EngineError::Other(format!("xcf.mask.enabled.{code}")) }
pub(crate) fn envelope(params: &Value) -> Result<(MaskId, bool)> {
    let object = params.as_object().ok_or_else(|| bad("params"))?;
    if object.len() != 2 || object.keys().any(|key| !matches!(key.as_str(), "maskId" | "enabled")) { return Err(bad("params")); }
    let text = object.get("maskId").and_then(Value::as_str).ok_or_else(|| bad("identity"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) { return Err(bad("identity")); }
    let raw = text.parse::<u64>().map_err(|_| bad("identity"))?;
    if raw == 0 || text.starts_with('0') { return Err(bad("identity")); }
    let enabled = object.get("enabled").and_then(Value::as_bool).ok_or_else(|| bad("params"))?;
    Ok((MaskId(raw), enabled))
}
/// Root calls this before implicit preview normalization or background dispatch.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if params.as_object().is_some_and(|object| object.contains_key("coalesce")) || session.coalesce_request.is_some() { return Err(bad("coalescing")); }
    envelope(params)?;
    if background || session.command_is_background() || session.command_has_pending_job() || session.has_jobs() || session.color_restrict.is_some() { return Err(bad("context")); }
    if active.floating.is_some() { return Err(bad("floating")); }
    Ok(())
}
fn legacy_target(session: &Session, params: &Value) -> Result<Option<(LayerId, Option<MaskId>)>> {
    let active = session.active().ok_or(EngineError::NoDocument)?;
    let Some(state) = &active.doc.xcf_state else { return Ok(None); };
    match params {
        Value::Null => {},
        Value::Object(object) => {
            if object.len() > 2 || object.keys().any(|key| !matches!(key.as_str(), "layer" | "enabled")) { return Err(bad("params")); }
            if object.get("layer").is_some_and(|value| value.as_u64().is_none_or(|id| id == 0))
                || object.get("enabled").is_some_and(|value| !value.is_boolean()) { return Err(bad("params")); }
        },
        _ => return Err(bad("params")),
    }
    let owner = crate::commands::layer_param(session, params)?;
    // Checked actual ancestry bounds the native lookup before association scans.
    crate::xcf_item_locks::effective_layer_locks(&active.doc, owner)?;
    if state.items.len() > 10_000 || state.filter_masks.len() > 10_000 { return Err(bad("limit")); }
    let mut filter_ids = std::collections::HashSet::new();
    filter_ids.try_reserve(state.filter_masks.len()).map_err(|_| bad("allocation"))?;
    for mask in &state.filter_masks { filter_ids.insert(mask.id); }
    let mut found = None;
    for item in &state.items {
        if let Some(metadata) = &item.mask
            && metadata.owner == ItemRef::Layer(owner)
            && !matches!(item.item, ItemRef::Mask(id) if filter_ids.contains(&id)) {
            let ItemRef::Mask(id) = item.item else { return Err(bad("identity")); };
            if id.0 == 0 || found.replace(id).is_some() { return Err(bad("ambiguous-owner")); }
        }
    }
    if found.is_some() && active.doc.layer(owner).is_none_or(|layer| layer.mask.is_none()) { return Err(bad("missing-native-mask")); }
    Ok(Some((owner, found)))
}
/// Source-document legacy toggles refuse before dispatcher preview/async/coalesce normalization.
/// False leaves non-XCF legacy semantics untouched; true includes ordinary new masks in a source doc.
pub(crate) fn legacy_enabled_preflight(session: &Session, params: &Value, background: bool) -> Result<bool> {
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.doc.xcf_state.is_none() { return Ok(false); }
    if params.as_object().is_some_and(|object| object.contains_key("coalesce")) || session.coalesce_request.is_some() { return Err(bad("coalescing")); }
    if active.floating.is_some() { return Err(bad("floating")); }
    if background || session.command_is_background() || session.command_has_pending_job() || session.has_jobs() || session.color_restrict.is_some() { return Err(bad("context")); }
    legacy_target(session, params)?;
    Ok(true)
}
/// Preserve the established legacy response while routing canonical masks through checked enablement.
pub(crate) fn legacy_enabled(session: &mut Session, params: &Value) -> Result<Option<Value>> {
    if !legacy_enabled_preflight(session, params, false)? { return Ok(None); }
    let Some((owner, target)) = legacy_target(session, params)? else { return Ok(None); };
    let Some(mask) = target else {
        // A newly created ordinary mask has no imported mask identity; inherit
        // the checked owner's imported content/native All without inventing one.
        let locks=crate::xcf_item_locks::effective_layer_locks(&session.active().ok_or(EngineError::NoDocument)?.doc, owner)?;
        if locks.native.all || locks.imported.content { return Err(bad("locked")); }
        return Ok(None);
    };
    let current = session.active().ok_or(EngineError::NoDocument)?.doc.layer(owner).and_then(|layer| layer.mask.as_ref())
        .ok_or_else(|| bad("missing-native-mask"))?.enabled;
    let enabled = params.get("enabled").and_then(Value::as_bool).unwrap_or(!current);
    set_enabled_with_interrupt(session, &json!({"maskId":mask.0.to_string(),"enabled":enabled}), &Interrupt::NONE)?;
    Ok(Some(json!({"enabled":enabled})))
}
fn relation(doc: &Document, id: MaskId, control: &Interrupt<'_>) -> Result<(LayerId, bool)> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    doc.validate_xcf_state_with(control).map_err(|error| if error.code == "cancelled" { EngineError::Cancelled } else { bad("invalid-state") })?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("unsupported"))?;
    if state.items.len() > 10_000 || state.filter_masks.len() > 10_000 { return Err(bad("limit")); }
    let mut filter_ids = std::collections::HashSet::new();
    filter_ids.try_reserve(state.filter_masks.len()).map_err(|_| bad("allocation"))?;
    for mask in &state.filter_masks {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if mask.id == id { return Err(bad("unsupported-filter-mask")); }
        filter_ids.insert(mask.id);
    }
    let mut target = None;
    for item in &state.items {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if item.item == ItemRef::Mask(id) {
            if target.is_some() { return Err(bad("identity")); }
            target = Some(item);
        }
    }
    let item = target.ok_or_else(|| bad("missing-mask"))?;
    let metadata = item.mask.as_ref().ok_or_else(|| bad("unsupported"))?;
    let ItemRef::Layer(owner) = metadata.owner else { return Err(bad("unsupported-owner")); };
    for other in &state.items {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if other.item != ItemRef::Mask(id)
            && !matches!(other.item, ItemRef::Mask(other_id) if filter_ids.contains(&other_id))
            && other.mask.as_ref().is_some_and(|mask| mask.owner == ItemRef::Layer(owner)) { return Err(bad("ambiguous-owner")); }
    }
    let layer = doc.layer(owner).ok_or_else(|| bad("missing-owner"))?;
    let native = layer.mask.as_ref().ok_or_else(|| bad("missing-native-mask"))?;
    let format = native.surface.format();
    if metadata.bounds.is_empty() || metadata.offset != [metadata.bounds.x0, metadata.bounds.y0]
        || item.raster_bounds.is_some_and(|bounds| bounds != metadata.bounds)
        || format.mode != photocraft_doc::ColorMode::Grayscale || format.alpha || format.sample != doc.depth
        || native.enabled != metadata.enabled || native.linked != metadata.linked
        || !native.density.is_finite() || native.density != 1.0 || native.feather != 0.0 {
        return Err(bad("unsupported-mask-domain"));
    }
    let outside = photocraft_color::try_read_sample_f64(native.surface.default_bytes(), format.sample, 0).map_err(|_| bad("invalid-default"))?;
    if outside.to_bits() != metadata.outside_f64_bits || !(0.0..=1.0).contains(&outside) { return Err(bad("invalid-default")); }
    native.surface.try_validate_tile_origins_with(control)?;
    crate::xcf_item_locks::require_layer_mask_content(doc, owner)?;
    let locks = crate::xcf_item_locks::effective_item_locks(doc, ItemRef::Mask(id))?;
    if locks.native.all || locks.imported.content { return Err(bad("locked")); }
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok((owner, metadata.enabled))
}
fn install(doc: &mut Document, id: MaskId, owner: LayerId, enabled: bool, control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    let state = doc.xcf_state.as_mut().ok_or_else(|| bad("missing-mask"))?;
    let mut target = None;
    for item in &mut state.items {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if item.item == ItemRef::Mask(id) { target = Some(item); break; }
    }
    target.and_then(|item| item.mask.as_mut()).ok_or_else(|| bad("missing-mask"))?.enabled = enabled;
    doc.layer_mut(owner).and_then(|layer| layer.mask.as_mut()).ok_or_else(|| bad("missing-native-mask"))?.enabled = enabled;
    Ok(())
}
pub fn set_enabled_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    let (mask, enabled) = envelope(params)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    active.doc.validate_resident().map_err(|_| bad("unsupported-accounted"))?;
    let (owner, previous) = relation(&active.doc, mask, control)?;
    photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(&active.doc, control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("xcf.mask.enabled.unsupported: {error}")) })?;
    let response = json!({"maskId":mask.0.to_string(),"layerId":owner.0.to_string(),"enabled":enabled,"changed":previous != enabled});
    if previous == enabled { return Ok(response); }
    let baseline = active.doc.clone();
    let revision = active.revision;
    let footprint = baseline.resident_raster_footprint_with(control)?;
    if footprint.clone_metadata_bytes.checked_mul(2).and_then(|n| n.checked_add(footprint.retained_bytes))
        .is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES) { return Err(bad("cow-limit")); }
    let mut staged = baseline.clone();
    let doc = staged.try_resident_mut_with_raster_limit(photocraft_raster::MAX_CHECKED_RASTER_BYTES, control)?;
    install(doc, mask, owner, enabled, control)?;
    relation(doc, mask, control)?;
    photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("xcf.mask.enabled.unsupported: {error}")) })?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    dispatch_preflight(session, params, false)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.revision != revision || !active.doc.shares_state(&baseline) { return Err(bad("stale-target")); }
    session.edit_with_interrupt("Set XCF Layer Mask Enabled", control, move |doc, _| {
        let (current_owner, current_enabled) = relation(doc, mask, control)?;
        if current_owner != owner || current_enabled != previous { return Err(bad("stale-target")); }
        install(doc, mask, owner, enabled, control)
    })?;
    Ok(response)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    session.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn run(session: &mut Session, params: &Value) -> Result<Value> { set_enabled_with_interrupt(session, params, &Interrupt::NONE) }
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec { id: "xcf.mask.enabled.set", label: "Set XCF Layer Mask Enabled", menu: &[], shortcut: None,
        params: r#"{"maskId":"canonical decimal U64","enabled":bool} → current qualified layer mask only; synchronize native/current flags without altering retained samples or source; synchronous, no preview/coalescing"#,
        enabled, run, journal: false }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{LayerMask, XcfState, SampleEncoding, xcf_state::{ItemState, MaskState}};
    fn fixture() -> (Session, LayerId, MaskId) {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":4,"height":3})).unwrap();
        let owner = session.active().unwrap().doc.layers.first().unwrap().id;
        let mask = MaskId(1);
        let doc = session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        let bounds = doc.bounds();
        let layer = doc.layer_mut(owner).unwrap();
        let mut surface = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::GRAY8);
        surface.try_write_interleaved(bounds, &[0,17,128,255,9,31,127,254,3,40,155,200]).unwrap();
        layer.mask = Some(LayerMask { surface: std::sync::Arc::new(surface), enabled:true, linked:true, density:1.0, feather:0.0 });
        let item = |reference, name: &str| ItemState { item:reference, name:name.into(), custom_name:true, visible:true, locks:Default::default(),
            parent:None, source_tattoo:None, source_flags:0, raster_bounds:Some(bounds), mask:None, composition:None };
        let mut mask_item = item(ItemRef::Mask(mask), "Own byte-exact mask");
        mask_item.mask = Some(MaskState { owner:ItemRef::Layer(owner), bounds, offset:[bounds.x0,bounds.y0], outside_f64_bits:0.0_f64.to_bits(),
            enabled:true, shown:false, editing:false, linked:true });
        doc.xcf_state = Some(XcfState { items:vec![item(ItemRef::Layer(owner), "Own raster"),mask_item], ..Default::default() });
        (session, owner, mask)
    }
    #[test]
    fn layer_mask_flags_are_one_undo_step_native_and_sample_exact() {
        let (mut session, owner, mask) = fixture();
        let before = session.active().unwrap().doc.clone();
        let surface = before.layer(owner).unwrap().mask.as_ref().unwrap().surface.clone();
        let p = json!({"maskId":mask.0.to_string(),"enabled":false});
        assert_eq!(set_enabled_with_interrupt(&mut session,&p,&Interrupt::NONE).unwrap()["changed"],true);
        let mut expected = before.as_ref().clone();
        install(&mut expected,mask,owner,false,&Interrupt::NONE).unwrap();
        assert_eq!(session.active().unwrap().doc.as_ref(),&expected);
        assert!(std::sync::Arc::ptr_eq(&surface,&session.active().unwrap().doc.layer(owner).unwrap().mask.as_ref().unwrap().surface));
        let native = photocraft_format::save_to_bytes(&session.active().unwrap().doc,&Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(),expected);
        let revision = session.active().unwrap().revision;
        assert_eq!(set_enabled_with_interrupt(&mut session,&p,&Interrupt::NONE).unwrap()["changed"],false);
        assert_eq!(session.active().unwrap().revision,revision);
        session.execute("edit.undo",json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc,before);
        session.execute("edit.redo",json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.as_ref(),&expected);
        let p = json!({"maskId":mask.0.to_string(),"enabled":true});
        set_enabled_with_interrupt(&mut session,&p,&Interrupt::NONE).unwrap();
        assert_eq!(session.active().unwrap().doc,before);
    }
    #[test]
    fn strict_identity_lock_missing_and_cancellation_refuse_without_edit() {
        let (mut session, owner, mask) = fixture();
        for p in [json!({"maskId":mask.0,"enabled":false}),json!({"maskId":"01","enabled":false}),
            json!({"maskId":mask.0.to_string(),"enabled":1}),json!({"maskId":"99","enabled":false}),
            json!({"maskId":mask.0.to_string(),"enabled":false,"unknown":true})] {
            let before = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            assert!(set_enabled_with_interrupt(&mut session,&p,&Interrupt::NONE).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision,revision);
        }
        let p=json!({"maskId":mask.0.to_string(),"enabled":false});
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().layer_mut(owner).unwrap().locks.all=true;
        let before=session.active().unwrap().doc.clone();
        assert!(set_enabled_with_interrupt(&mut session,&p,&Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().layer_mut(owner).unwrap().locks.all=false;
        let before=session.active().unwrap().doc.clone();
        let (mut probe, _, probe_mask) = fixture();
        let calls=std::sync::atomic::AtomicUsize::new(0);
        let observe=|| { calls.fetch_add(1,std::sync::atomic::Ordering::Relaxed); false };
        set_enabled_with_interrupt(&mut probe,&json!({"maskId":probe_mask.0.to_string(),"enabled":false}),&Interrupt::cancel_only(&observe)).unwrap();
        let final_poll=calls.load(std::sync::atomic::Ordering::Relaxed);
        assert!(final_poll>1);
        calls.store(0,std::sync::atomic::Ordering::Relaxed);
        let cancel=|| calls.fetch_add(1,std::sync::atomic::Ordering::Relaxed)+1>=final_poll;
        assert!(matches!(set_enabled_with_interrupt(&mut session,&p,&Interrupt::cancel_only(&cancel)),Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn pixels_only_lock_allows_mask_toggle_but_imported_content_refuses() {
        let (mut session,owner,mask)=fixture();
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().layer_mut(owner).unwrap().locks.pixels=true;
        let params=json!({"maskId":mask.0.to_string(),"enabled":false});
        set_enabled_with_interrupt(&mut session,&params,&Interrupt::NONE).unwrap();
        session.execute("edit.undo",json!({})).unwrap();
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state.as_mut().unwrap().items.iter_mut()
            .find(|item|item.item==ItemRef::Mask(mask)).unwrap().locks.content=true;
        let before=session.active().unwrap().doc.clone();
        assert!(set_enabled_with_interrupt(&mut session,&params,&Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        {
            let doc=session.active_mut().unwrap().doc.try_resident_mut().unwrap();
            doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item|item.item==ItemRef::Mask(mask)).unwrap().locks.content=false;
            let child=doc.layers.pop().unwrap();
            let group=photocraft_doc::Layer::group("Own locked ancestor",vec![child]);
            let group_id=group.id;
            doc.layers.push(group);
            let state=doc.xcf_state.as_mut().unwrap();
            let child_item=state.items.iter_mut().find(|item|item.item==ItemRef::Layer(owner)).unwrap();
            child_item.parent=Some(ItemRef::Layer(group_id));
            let mut group_item=child_item.clone();
            group_item.item=ItemRef::Layer(group_id);
            group_item.name="Own locked ancestor".into();
            group_item.parent=None;
            group_item.raster_bounds=None;
            group_item.locks.content=true;
            state.items.push(group_item);
        }
        let before=session.active().unwrap().doc.clone();
        assert!(set_enabled_with_interrupt(&mut session,&params,&Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }

    #[test]
    fn legacy_registered_toggle_preserves_source_and_refuses_stale_preview() {
        let (mut session,owner,mask)=fixture();
        let before=session.active().unwrap().doc.clone();
        let result=session.execute("layer.layerMask.enabled",json!({"layer":owner.0})).unwrap();
        assert_eq!(result,json!({"enabled":false}));
        let mut expected=before.as_ref().clone();
        install(&mut expected,mask,owner,false,&Interrupt::NONE).unwrap();
        assert_eq!(session.active().unwrap().doc.as_ref(),&expected);
        session.execute("edit.undo",json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc,before);
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().selection=Some(photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::GRAY8,&[1.0]));
        session.execute("select.float",json!({"layer":owner.0,"copy":true,"dx":1,"dy":0})).unwrap();
        session.active_mut().unwrap().floating.as_mut().unwrap().revision=0;
        let snapshot=session.active().unwrap().doc.clone();
        let floating=session.active().unwrap().floating.clone();
        let revision=session.active().unwrap().revision;
        assert!(session.execute("layer.layerMask.enabled",json!({"layer":owner.0,"enabled":false})).is_err());
        assert!(session.active().unwrap().doc.shares_state(&snapshot));
        assert_eq!(session.active().unwrap().revision,revision);
        assert!(std::sync::Arc::ptr_eq(&session.active().unwrap().floating.as_ref().unwrap().parts,&floating.unwrap().parts));
    }

    #[test]
    fn legacy_null_is_default_toggle_but_new_command_stays_strict() {
        let (mut session,owner,mask)=fixture();
        session.active_mut().unwrap().active_layer=Some(owner);
        let before=session.active().unwrap().doc.clone();
        assert_eq!(session.execute("layer.layerMask.enabled",Value::Null).unwrap(),json!({"enabled":false}));
        let mut expected=before.as_ref().clone();
        install(&mut expected,mask,owner,false,&Interrupt::NONE).unwrap();
        assert_eq!(session.active().unwrap().doc.as_ref(),&expected);
        session.execute("edit.undo",json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc,before);
        assert!(set_enabled_with_interrupt(&mut session,&Value::Null,&Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }

}
