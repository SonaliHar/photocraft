//! Explicit byte-owned linked source refresh; stored locators remain inert.
use crate::{DocumentSnapshot, EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{LayerContent, LayerId, item_identity::ItemRef, xcf_items::EditableSource};
use photocraft_raster::{Interrupt, backing::ReadControl};
use serde_json::{Value, json};
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    context(s, false).map_err(|error| error.to_string())
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn context(s: &Session, background: bool) -> Result<()> {
    if background || s.command_is_background() || s.command_has_pending_job() || s.has_jobs() || s.coalesce_request.is_some() || s.color_restrict.is_some() {
        return Err(bad("linked refresh requires synchronous idle uncoalesced context"));
    }
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if active.doc.resident_arc().is_none() || active.revision == u64::MAX {
        return Err(bad("linked refresh requires a resident revision carrier"));
    }
    if s.documents().len() > 4096 {
        return Err(bad("linked refresh document count limit"));
    }
    if s.documents()
        .iter()
        .any(|state| state.floating.is_some() || state.coalesce.is_some() || state.doc.xcf_state.as_ref().is_some_and(|xcf| xcf.floating.is_some()))
    {
        return Err(bad("complete every pixel-move/attachment/coalesced context before linked refresh"));
    }
    Ok(())
}
fn owner(params: &Value) -> Result<LayerId> {
    let object = params.as_object().ok_or_else(|| bad("linked source parameters require an object"))?;
    if object.keys().any(|key| key != "layerId") {
        return Err(bad("unknown linked source parameter"));
    }
    let text = params.get("layerId").and_then(Value::as_str).ok_or_else(|| bad("layerId requires a canonical decimal string"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad("invalid layer identity"));
    }
    let id = text.parse::<u64>().map_err(|_| bad("invalid layer identity"))?;
    if id == 0 || id.to_string() != text {
        return Err(bad("invalid layer identity"));
    }
    Ok(LayerId(id))
}
/// Returns true for a linked material command after strict borrowed admission.
/// Root dispatch must call this before its implicit floating-selection handling.
pub(crate) fn dispatch_preflight(s: &Session, id: &str, params: &Value) -> Result<bool> {
    dispatch_preflight_with_background(s, id, params, false)
}
/// Also admit explicit background intent before generic dispatcher setup.
pub(crate) fn dispatch_preflight_with_background(s: &Session, id: &str, params: &Value, background: bool) -> Result<bool> {
    if id != "xcf.link.refresh" {
        return crate::xcf_link_geometry_cmds::dispatch_preflight(s, id, params);
    }
    owner(params)?;
    context(s, background)?;
    Ok(true)
}
fn headroom(doc: &DocumentSnapshot, owner: LayerId, control: &Interrupt<'_>) -> Result<()> {
    let xcf = doc.xcf_state.as_ref().ok_or_else(|| bad("missing linked source state"))?;
    if [xcf.items.len(), xcf.resources.len(), xcf.source_items.len(), xcf.filter_stacks.len(), xcf.filter_masks.len(), xcf.item_sets.len()]
        .into_iter()
        .any(|count| count > 10_000)
    {
        return Err(bad("linked retained metadata count limit"));
    }
    let bounds = xcf
        .items
        .iter()
        .find(|item| item.item == ItemRef::Layer(owner))
        .and_then(|item| item.raster_bounds)
        .ok_or_else(|| bad("missing linked current bounds"))?;
    if bounds.is_empty() || bounds.width() > 8192 || bounds.height() > 8192 {
        return Err(bad("linked current extent limit"));
    }
    let pixels = u64::from(bounds.width())
        .checked_mul(u64::from(bounds.height()))
        .filter(|count| *count <= 4_000_000)
        .ok_or_else(|| bad("linked output pixel limit"))?;
    let tile = i64::from(photocraft_geom::TILE_SIZE);
    let axis = |lo: i32, hi: i32| -> Result<u64> {
        u64::try_from(
            (i64::from(hi) - 1)
                .div_euclid(tile)
                .checked_sub(i64::from(lo).div_euclid(tile))
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| bad("linked tile count overflow"))?,
        )
        .map_err(|_| bad("linked tile count overflow"))
    };
    let bpp = doc.pixel_format().bytes_per_pixel() as u64;
    let output = axis(bounds.x0, bounds.x1)?
        .checked_mul(axis(bounds.y0, bounds.y1)?)
        .and_then(|n| n.checked_mul((tile as u64).checked_mul(tile as u64)?.checked_mul(bpp)?.checked_add(256)?))
        .and_then(|n| n.checked_add(bpp))
        .ok_or_else(|| bad("linked output envelope overflow"))?;
    let footprint = doc.resident_raster_footprint_with(control)?;
    if footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes.checked_mul(2).ok_or_else(|| bad("linked COW overflow"))?)
        .and_then(|n| n.checked_add(output.checked_mul(2)?))
        .and_then(|n| n.checked_add(pixels.checked_mul(bpp)?))
        .is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad("linked retained/COW/output staging limit"));
    }
    let mut limits = photocraft_io::NativeSaveLimits::default().preparation;
    limits.resident_bytes = limits.resident_bytes.min(128 << 20);
    limits.object_bytes = limits.object_bytes.min(128 << 20);
    limits.index_bytes = limits.index_bytes.min(4 << 20);
    limits.manifest_bytes = limits.manifest_bytes.min(16 << 20);
    let retained = photocraft_io::retained_native_input_bytes(doc, limits, control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(error.to_string()) })?;
    if (retained as u64).checked_mul(2).and_then(|n| n.checked_add(output)).is_none_or(|n| n > 256 << 20) {
        return Err(bad("linked measured retained metadata staging limit"));
    }
    poll(control)
}
fn fence(s: &Session, baseline: &DocumentSnapshot, revision: u64, params: &Value, control: &Interrupt<'_>) -> Result<()> {
    dispatch_preflight(s, "xcf.link.refresh", params)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    if active.revision != revision || !active.doc.shares_state(baseline) {
        return Err(bad("linked refresh captured document changed while preparing"));
    }
    poll(control)
}
/// Regenerate from explicitly owned material; cancellation never commits partial raster or identity.
pub fn refresh_with_interrupt(s: &mut Session, params: &Value, control: &photocraft_raster::Interrupt<'_>) -> Result<Value> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    dispatch_preflight(s, "xcf.link.refresh", params)?;
    let owner = owner(params)?;
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let baseline = active.doc.clone();
    let revision = active.revision;
    let doc = &baseline;
    headroom(doc, owner, control)?;
    doc.validate_resident().map_err(|_| bad("linked refresh requires a resident document; accounted edit adoption is not available"))?;
    doc.validate_xcf_state_with(control).map_err(|e| if e.code == "cancelled" { EngineError::Cancelled } else { bad(e.to_string()) })?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing linked source state"))?;
    let source = state.source_items.iter().find(|source| source.owner == ItemRef::Layer(owner)).ok_or_else(|| bad("missing linked source owner"))?;
    let EditableSource::Linked { resource, last_verified_hash, .. } = &source.source else {
        return Err(bad("owner is not a linked image"));
    };
    crate::xcf_item_locks::effective_layer_locks(doc, owner)?.require(crate::xcf_item_locks::EditKind::Content)?;
    let resolved = state
        .resources
        .iter()
        .find(|entry| entry.id == *resource)
        .and_then(|entry| entry.resolution.as_ref())
        .ok_or_else(|| bad("linked image requires explicit resource recovery"))?;
    let digest = resolved.payload_hash.clone();
    let fidelity = resolved.fidelity;
    let (_, current) = photocraft_compose::gimp_link_items::prepare_current_source(doc, source, control)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    if last_verified_hash.as_deref() == Some(digest.as_str()) && doc.layer(owner).and_then(|layer| layer.surface()) == Some(&current) {
        fence(s, &baseline, revision, params, control)?;
        return Ok(json!({"layerId":owner.0.to_string(),"byteIdentity":fidelity,"refreshed":false}));
    }
    let mut candidate = baseline.clone();
    {
        let doc = candidate.try_resident_mut_with_raster_limit(photocraft_raster::MAX_CHECKED_RASTER_BYTES, control)?;
        let layer = doc.layer_mut(owner).ok_or_else(|| bad("unknown linked owner"))?;
        if !matches!(layer.content, LayerContent::Raster(_)) {
            return Err(bad("linked refresh requires a resident raster owner"));
        }
        layer.content = LayerContent::Raster(current);
        let source = doc
            .xcf_state
            .as_mut()
            .and_then(|state| state.source_items.iter_mut().find(|source| source.owner == ItemRef::Layer(owner)))
            .ok_or_else(|| bad("missing linked source"))?;
        let EditableSource::Linked { last_verified_hash, .. } = &mut source.source else {
            return Err(bad("invalid linked source"));
        };
        *last_verified_hash = Some(digest);
        photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control)
            .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) })?;
    }
    headroom(&candidate, owner, control)?;
    let response = json!({"layerId":owner.0.to_string(),"byteIdentity":fidelity,"refreshed":true});
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let target = || -> Result<photocraft_ops::LayerTarget> {
        if active.selected_layers.len() > 10_000 {
            return Err(bad("linked runtime selection limit"));
        }
        let mut selected = Vec::new();
        selected.try_reserve_exact(active.selected_layers.len()).map_err(|error| bad(error.to_string()))?;
        selected.extend_from_slice(&active.selected_layers);
        Ok(photocraft_ops::LayerTarget { active: active.active_layer, selected })
    };
    let prior = target()?;
    let layers = target()?;
    let read = ReadControl { interrupt: control, work: &|_| Ok(()) };
    let mut history = active.history.clone();
    history.set_current_layers(prior, &read)?;
    history.record("Refresh XCF Linked Image", baseline.clone(), layers, &read)?;
    history.trim(&candidate, &read)?;
    fence(s, &baseline, revision, params, control)?;
    let active = s.active_mut().ok_or(EngineError::NoDocument)?;
    active.doc = candidate;
    active.history = history;
    active.revision += 1;
    active.last_damage = None;
    Ok(response)
}
fn refresh(s: &mut Session, params: &Value) -> Result<Value> {
    refresh_with_interrupt(s, params, &photocraft_raster::Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "xcf.link.refresh",
        label: "Refresh XCF Linked Image",
        menu: &[],
        shortcut: None,
        params: r#"{"layerId":"canonical decimal U64"} → refresh current raster from qualified compatible PNG owned bytes; no locator reads; retains exact or substituted byte identity"#,
        enabled,
        run: refresh,
        journal: false,
    }]
}

// Implementation-first transaction regression drafts; execution is deferred.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        Color, ColorMode, Document, SampleEncoding, SampleType, Size, XcfState,
        item_identity::ResourceId,
        xcf_items::{SourceCache, SourceItem},
        xcf_resources::{ResourceFidelity, ResourceKind, ResourceReference, ResourceResolution},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn fixture() -> (Session, LayerId) {
        let mut doc = Document::with_background("Own refresh source", Size::new(2, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let owner = doc.layers.first().unwrap().id;
        doc.layers.first_mut().unwrap().locks = Default::default();
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(photocraft_cms::builtin::srgb().to_bytes());
        let original = doc.layers.first().unwrap().surface().unwrap().try_read_interleaved(doc.bounds()).unwrap();
        let mut image = photocraft_codecs::Image::from_raw(
            2,
            1,
            photocraft_codecs::ChannelLayout::Rgba,
            photocraft_codecs::SampleType::U8,
            vec![17, 29, 31, 255, 2, 3, 4, 64],
        )
        .unwrap();
        image.icc = doc.icc_profile.as_deref().cloned();
        let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let digest = blake3::hash(&png).to_hex().to_string();
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.items.iter_mut().find(|item| item.item == ItemRef::Layer(owner)).unwrap().raster_bounds = Some(doc.bounds());
        state.resources.push(ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::LinkedImage,
            name: "Own material".into(),
            collection: None,
            internal: false,
            locator: Some("inert:never-read".into()),
            content_hash: Some(digest.clone()),
            owned_bytes: Some(png.into()),
            resolution: Some(ResourceResolution { fidelity: ResourceFidelity::Exact, payload_hash: digest }),
        });
        state.source_items.push(SourceItem {
            owner: ItemRef::Layer(owner),
            source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
            transform: None,
            cache: Some(SourceCache {
                blake3: blake3::hash(&original).to_hex().to_string(),
                bytes: original.into(),
                width: 2,
                height: 1,
                format: doc.pixel_format(),
            }),
        });
        doc.xcf_state = Some(state);
        let mut session = Session::new();
        session.open_document(doc, None).unwrap();
        session.active_mut().unwrap().set_import_origin(photocraft_io::ImportOrigin::xcf(), "Own synthetic linked source", None);
        (session, owner)
    }
    fn params(owner: LayerId) -> Value {
        json!({"layerId":owner.0.to_string()})
    }
    #[test]
    fn refresh_changes_only_current_pixels_and_verified_hash_with_real_undo() {
        let (mut session, owner) = fixture();
        let before = session.active().unwrap().doc.clone();
        let source = session.active().unwrap().source.clone();
        let focus = session.active().unwrap().selected_layers.clone();
        assert_eq!(refresh_with_interrupt(&mut session, &params(owner), &Interrupt::NONE).unwrap()["refreshed"], true);
        let active = session.active().unwrap();
        let mut expected = (*before).clone();
        expected.layer_mut(owner).unwrap().content = active.doc.layer(owner).unwrap().content.clone();
        let digest = expected.xcf_state.as_ref().unwrap().resources.first().unwrap().resolution.as_ref().unwrap().payload_hash.clone();
        let EditableSource::Linked { last_verified_hash, .. } = &mut expected.xcf_state.as_mut().unwrap().source_items.first_mut().unwrap().source else {
            panic!("synthetic linked source required")
        };
        *last_verified_hash = Some(digest);
        assert_eq!(*active.doc, expected);
        assert_eq!(active.source, source);
        assert_eq!(active.selected_layers, focus);
        let native = photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), expected);
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc, before);
        assert!(session.redo());
        assert_eq!(*session.active().unwrap().doc, expected);
    }
    #[test]
    fn refreshed_noop_keeps_real_redo_and_checks_every_cancellation_fence() {
        let (mut session, owner) = fixture();
        refresh_with_interrupt(&mut session, &params(owner), &Interrupt::NONE).unwrap();
        session
            .edit("Real redo", |doc, _| {
                doc.name = "redo sentinel".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let count = AtomicUsize::new(0);
        let measure = || {
            count.fetch_add(1, Ordering::Relaxed);
            false
        };
        assert_eq!(refresh_with_interrupt(&mut session, &params(owner), &Interrupt::cancel_only(&measure)).unwrap()["refreshed"], false);
        let checks = count.load(Ordering::Relaxed);
        assert!(checks > 1);
        for threshold in 0..checks {
            let calls = AtomicUsize::new(0);
            let stop = || calls.fetch_add(1, Ordering::Relaxed) >= threshold;
            assert!(matches!(refresh_with_interrupt(&mut session, &params(owner), &Interrupt::cancel_only(&stop)), Err(EngineError::Cancelled)));
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision, revision);
        }
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), native);
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc.name, "redo sentinel");
    }
    #[test]
    fn nonactive_preview_background_coalesce_and_stale_snapshot_refuse_before_refresh() {
        let (mut session, owner) = fixture();
        let baseline = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        assert!(dispatch_preflight_with_background(&session, "xcf.link.refresh", &params(owner), true).is_err());
        session.coalesce_request = Some("own current interaction".into());
        assert!(refresh_with_interrupt(&mut session, &params(owner), &Interrupt::NONE).is_err());
        session.coalesce_request = None;
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().name = "changed without revision".into();
        assert!(fence(&session, &baseline, revision, &params(owner), &Interrupt::NONE).is_err());
        session.open_document(Document::with_background("Own other preview", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE), None).unwrap();
        session.execute("select.all", json!({})).unwrap();
        session.execute("select.float", json!({"dx":1,"dy":0,"copy":true})).unwrap();
        session.set_active(0);
        let before = session.active().unwrap().doc.clone();
        let preview = session.documents().get(1).unwrap().floating.clone().unwrap();
        assert!(refresh_with_interrupt(&mut session, &params(owner), &Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        let retained = session.documents().get(1).unwrap().floating.as_ref().unwrap();
        assert!(std::sync::Arc::ptr_eq(&preview.parts, &retained.parts));
        assert_eq!((preview.offset, preview.revision), (retained.offset, retained.revision));
    }
}
