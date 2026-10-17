//! Controlled ordinary Reveal Selection: exact retained mask storage, no arithmetic.
use crate::{EngineError, Result, Session};
use photocraft_doc::{Document, LayerId, LayerMask};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::Value;
use std::sync::Arc;
const CMD: &str = "layer.layerMask.revealSelection";
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.into() }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn error(control: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(error.to_string()) }
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok()
}
fn request(session: &Session, params: &Value) -> Result<Vec<LayerId>> {
    if !params.is_null() {
        let object = params.as_object().filter(|map| map.len() <= 3).ok_or_else(|| bad("bounded mask parameters required"))?;
        if !object.keys().all(|key| matches!(key.as_str(), "layer" | "documentId" | "expectedRevision")) {
            return Err(bad("unknown mask parameter"));
        }
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.selected_layers.len() > 10_000 {
        return Err(bad("mask target count limit"));
    }
    if let Some(value) = params.get("layer")
        && value.as_u64().filter(|id| *id != 0).is_none()
    {
        return Err(bad("layer must be a nonzero unsigned integer"));
    }
    // Use the upstream multi-selected mask eligibility and ordering policy.
    let ids = crate::layer_multi_cmds::mask_targets(session, params)?;
    if ids.is_empty() || ids.len() > 10_000 {
        return Err(bad("mask target count limit"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if state.doc.id.0 != 0 && decimal(id) == Some(state.doc.id.0) && decimal(revision) == Some(state.revision) => {}
        _ => return Err(bad("captured target changed or incomplete")),
    }
    if state.revision == u64::MAX
        || state.doc.accounted().is_some()
        || state.floating.is_some()
        || state.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
    {
        return Err(bad("finish transient work before mask creation"));
    }
    if state.doc.xcf_state.is_some() || state.doc.color_table.is_some() || state.doc.mode == photocraft_doc::ColorMode::Indexed {
        return Err(bad("source/indexed mask replacement is not qualified"));
    }
    let mut sorted = Vec::new();
    sorted.try_reserve_exact(ids.len()).map_err(|_| bad("mask target allocation"))?;
    sorted.extend_from_slice(&ids);
    sorted.sort_unstable();
    if sorted.windows(2).any(|pair| pair.first() == pair.get(1)) {
        return Err(bad("ambiguous mask owner"));
    }
    let mut found = Vec::new();
    found.try_reserve_exact(ids.len()).map_err(|_| bad("mask target allocation"))?;
    found.resize(ids.len(), 0usize);
    photocraft_doc::resident_structure::visit_layers_with(&state.doc, &Interrupt::NONE, |layer| {
        if let Ok(index) = sorted.binary_search(&layer.id) {
            let count = found.get_mut(index).ok_or(photocraft_raster::RasterInputError::InvalidBounds)?;
            *count = count.checked_add(1).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
            if layer.indexed_source.is_some() || layer.video.is_some() {
                return Err(photocraft_raster::RasterInputError::UnsupportedConversion);
            }
        }
        Ok(true)
    })?;
    if found.iter().any(|count| *count != 1) {
        return Err(bad("missing or ambiguous mask owner"));
    }
    for id in &ids {
        crate::xcf_item_locks::require_layer_mask_content(&state.doc, *id)?;
    }
    let selection = state.doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
    if selection.format().mode != photocraft_doc::ColorMode::Grayscale || selection.format().alpha {
        return Err(bad("selection must be native Gray without alpha"));
    }
    if selection.format().sample != state.doc.depth
        && (matches!(selection.format().sample, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64)
            || matches!(state.doc.depth, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64))
    {
        return Err(bad("wide selection mask depth conversion requires explicit qualified policy"));
    }
    Ok(ids)
}
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    request(session, params).map(|_| ())
}
fn admission(doc: &Document, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let selection = doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
    // Validates every raw tile/origin, including default-only allocated tiles,
    // without converting sample values or discarding off-canvas storage.
    selection.try_content_bounds_with_interrupt(control).map_err(|e| error(control, e))?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let map = (selection.tile_count() as u64).checked_mul(2048).ok_or_else(|| bad("mask map accounting overflow"))?;
    let peak = payload
        .checked_add(model.checked_mul(6).ok_or_else(|| bad("model accounting overflow"))?)
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(map))
        .and_then(|n| n.checked_add(1 << 20));
    if peak.is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES) {
        return Err(bad("mask retained/model/COW/map limit"));
    }
    let work = (selection.tile_count() as u64)
        .checked_mul(photocraft_geom::TILE_SIZE as u64)
        .and_then(|n| n.checked_mul(photocraft_geom::TILE_SIZE as u64))
        .and_then(|n| n.checked_mul(32))
        .and_then(|n| n.checked_add(payload.checked_add(model)?.checked_mul(16)?));
    if work.is_none_or(|n| n > 8_000_000_000) {
        return Err(bad("mask preparation work limit"));
    }
    poll(control)
}
fn copy_storage(source: &Surface, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    let mut output = Surface::try_with_default_bytes(source.format(), source.default_bytes()).map_err(|e| error(control, e))?;
    for (position, tile) in source.tiles() {
        poll(control)?;
        output.put_tiles(std::iter::once((*position, Arc::clone(tile))));
    }
    poll(control)?;
    Ok(output)
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    let ids = request(session, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let snapshot = state.doc.clone();
    let revision = state.revision;
    let active = state.active_layer;
    let selected = state.selected_layers.clone();
    let weak = snapshot.downgrade();
    let worker = snapshot.clone();
    crate::jobs::run(
        session,
        "Add Layer Mask",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Add Layer Mask", |control| {
                admission(&worker, control)?;
                poll(control)?;
                for id in &ids {
                    poll(control)?;
                    crate::xcf_item_locks::require_layer_mask_content(&worker, *id)?;
                }
                let selection = worker.selection.as_ref().ok_or_else(|| bad("selection disappeared"))?;
                let storage = if selection.format().sample == worker.depth {
                    copy_storage(selection, control)?
                } else {
                    poll(control)?;
                    let converted = selection.try_convert_legacy(selection.format().with_sample(worker.depth)).map_err(|e| error(control, e))?;
                    poll(control)?;
                    converted
                };
                let mask = LayerMask { surface: Arc::new(storage), enabled: true, linked: true, density: 1., feather: 0. };
                poll(control)?;
                Ok((mask, ids, ctx.clone()))
            })
        },
        move |session, (mask, ids, ctx)| {
            ctx.stage(0.95, 1., "Add Layer Mask", |control| {
                poll(control)?;
                let state = session.active().ok_or(EngineError::NoDocument)?;
                if state.doc.id != snapshot.id
                    || state.revision != revision
                    || !state.doc.shares_state(&snapshot)
                    || !weak.is_current(&state.doc)
                    || state.active_layer != active
                    || state.selected_layers != selected
                    || state.floating.is_some()
                    || state.coalesce.is_some()
                    || session.command_has_job_on(snapshot.id)
                    || session.command_has_pending_job()
                    || session.coalesce_request.is_some()
                    || session.color_restrict.is_some()
                {
                    return Err(bad("captured mask state changed"));
                }
                session.edit_with_interrupt("Add Layer Mask", control, move |doc, _| {
                    poll(control)?;
                    for id in ids {
                        poll(control)?;
                        crate::xcf_item_locks::require_layer_mask_content(doc, id)?;
                        crate::extra_cmds::background_to_layer_for_mask(doc, id);
                        doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.mask = Some(mask.clone());
                    }
                    // Upstream Reveal Selection consumes the selection in the same undo step.
                    doc.selection = None;
                    poll(control)?;
                    Ok(Value::Null)
                })
            })
        },
    )
}

// Draft actual storage/history/locks/cancellation regressions; intentionally unrun.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Layer, SampleType, Size};
    use photocraft_geom::Rect;
    use serde_json::json;
    fn fixture(sample: SampleType) -> (Session, LayerId, Vec<u8>) {
        let mut doc = Document::new("Own mask", Size::new(2, 1), ColorMode::Rgb, sample);
        let mut layer = Layer::raster("Background", doc.pixel_format());
        layer.locks.transparency = true;
        layer.locks.position = true;
        let id = layer.id;
        doc.layers.push(layer);
        let raw = match sample {
            SampleType::U32 => [1u32, u32::MAX - 1].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>(),
            SampleType::F64 => [0.5f64.to_bits() + 1, (-0f64).to_bits()].into_iter().flat_map(u64::to_le_bytes).collect(),
            _ => vec![1, 254],
        };
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, false);
        doc.selection = Some(Surface::try_from_interleaved_with(format, Rect::new(-1, 0, 1, 1), &raw, &Interrupt::NONE).unwrap());
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        (session, id, raw)
    }
    #[test]
    fn native_selection_default_offcanvas_words_and_shared_tiles_survive_mask_native_history() {
        for sample in [SampleType::U32, SampleType::F64] {
            let (mut session, id, raw) = fixture(sample);
            let before = session.active().unwrap().doc.clone();
            run(&mut session, &json!({"layer":id.0})).unwrap();
            let current = &session.active().unwrap().doc;
            let layer = current.layer(id).unwrap();
            let mask = layer.mask.as_ref().unwrap();
            assert_eq!(mask.surface.to_interleaved(Rect::new(-1, 0, 1, 1)), raw);
            assert_eq!(mask.surface.default_bytes(), before.selection.as_ref().unwrap().default_bytes());
            assert!(current.selection.is_none());
            assert_eq!(layer.name, "Layer 0");
            assert_eq!(layer.locks, Default::default());
            assert!(mask.enabled && mask.linked);
            assert_eq!(mask.density, 1.);
            assert_eq!(mask.feather, 0.);
            for (position, tile) in before.selection.as_ref().unwrap().tiles() {
                assert!(Arc::ptr_eq(tile, mask.surface.tile(*position).unwrap()));
            }
            let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(current, &Default::default()).unwrap()).unwrap();
            assert_eq!(reopened.layer(id).unwrap().mask, layer.mask);
            session.execute("edit.undo", Value::Null).unwrap();
            assert_eq!(session.active().unwrap().doc.layers, before.layers);
            assert_eq!(session.active().unwrap().doc.selection, before.selection);
            session.execute("edit.redo", Value::Null).unwrap();
            assert_eq!(session.active().unwrap().doc.layers, reopened.layers);
            assert!(session.active().unwrap().doc.selection.is_none());
        }
    }
    #[test]
    fn stale_wrong_type_and_inherited_all_lock_refuse_without_losing_real_redo() {
        let (mut session, id, _) = fixture(SampleType::U32);
        session
            .edit("Seed", |doc, _| {
                doc.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let redo = session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
        for params in [
            json!({"layer":"wrong"}),
            json!({"documentId":before.id.0.to_string()}),
            json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
        ] {
            assert!(run(&mut session, &params).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        }
        let mut locked = (*before).clone();
        locked.layers[0].locks.all = true;
        let mut blocked = Session::new();
        blocked.add_document(locked, None).unwrap();
        assert!(run(&mut blocked, &json!({"layer":id.0})).is_err());
        assert!(blocked.active().unwrap().doc.layer(id).unwrap().mask.is_none());
    }
    #[test]
    fn cancellation_while_copying_map_preserves_selection_and_does_not_publish_mask() {
        let (session, _, _) = fixture(SampleType::F64);
        let before = session.active().unwrap().doc.clone();
        let cancel = Interrupt::cancel_only(&|| true);
        assert!(matches!(admission(&before, &cancel), Err(EngineError::Cancelled)));
        assert!(matches!(copy_storage(before.selection.as_ref().unwrap(), &cancel), Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().doc.layers[0].mask.is_none());
        let mut source = before.selection.as_ref().unwrap().clone();
        for x in [256, 512, 768] {
            source.try_write_interleaved_with(Rect::new(x, 0, x + 1, 1), &0.5f64.to_le_bytes(), &Interrupt::NONE).unwrap();
        }
        let preserved = source.clone();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let late = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 2;
        assert!(matches!(copy_storage(&source, &Interrupt::cancel_only(&late)), Err(EngineError::Cancelled)));
        assert_eq!(source, preserved);
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
}

mod actions;
pub(crate) fn reveal_all(session: &mut Session, params: &Value) -> Result<Value> {
    actions::run(session, params, actions::Action::Reveal)
}
pub(crate) fn hide_all(session: &mut Session, params: &Value) -> Result<Value> {
    actions::run(session, params, actions::Action::Hide)
}
pub(crate) fn delete(session: &mut Session, params: &Value) -> Result<Value> {
    actions::run(session, params, actions::Action::Delete)
}
pub(crate) fn apply(session: &mut Session, params: &Value) -> Result<Value> {
    actions::run(session, params, actions::Action::Apply)
}
pub(crate) fn actions_dispatch_preflight(session: &Session, id: &str, params: &Value, _background: bool) -> Result<()> {
    let action = match id {
        "layer.layerMask.revealAll" => actions::Action::Reveal,
        "layer.layerMask.hideAll" => actions::Action::Hide,
        "layer.layerMask.delete" => actions::Action::Delete,
        "layer.layerMask.apply" => actions::Action::Apply,
        "layer.layerMask.hideSelection" => actions::Action::HideSelection,
        "layer.layerMask.fromTransparency" => actions::Action::FromTransparency,
        _ => return Ok(()),
    };
    actions::dispatch_preflight(session, params, action)
}

pub(crate) fn hide_selection(session: &mut Session, params: &Value) -> Result<Value> {
    actions::run(session, params, actions::Action::HideSelection)
}
pub(crate) fn from_transparency(session: &mut Session, params: &Value) -> Result<Value> {
    actions::run(session, params, actions::Action::FromTransparency)
}
