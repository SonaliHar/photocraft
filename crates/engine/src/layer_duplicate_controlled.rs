//! Frozen layer duplication, preserving the legacy single/multiple copy policies.
use crate::{EngineError, Result, Session};
use photocraft_doc::{Document, Layer, LayerId};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};
use std::collections::HashSet;
const CMD: &str = "layer.duplicate";
const MAX_LAYERS: usize = 100000;
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(format!("{CMD}: {}", message.into()))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn map_error(control: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(error.to_string()) }
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok()
}
fn explicit(params: &Value) -> Result<Option<LayerId>> {
    params
        .get("layer")
        .map(|value| value.as_u64().filter(|id| *id != 0).map(LayerId).ok_or_else(|| bad("layer must be a nonzero unsigned integer")))
        .transpose()
}
fn census(doc: &Document, control: &Interrupt<'_>) -> Result<usize> {
    let mut count = 0usize;
    photocraft_doc::resident_structure::visit_layers_with(doc, control, |layer| {
        count = count.checked_add(1).filter(|count| *count <= MAX_LAYERS).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        if layer.name.len() > 1 << 20 || layer.name.contains('\0') {
            return Err(photocraft_raster::RasterInputError::LimitExceeded);
        }
        Ok(true)
    })
    .map_err(|error| map_error(control, error))?;
    poll(control)?;
    Ok(count)
}
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    if !params.is_null() {
        let object = params.as_object().filter(|map| map.len() <= 7).ok_or_else(|| bad("expected bounded duplicate parameters"))?;
        if !object.keys().all(|key| matches!(key.as_str(), "layer" | "name" | "inPlace" | "documentId" | "expectedRevision" | "document" | "documentName")) {
            return Err(bad("unknown duplicate parameter"));
        }
    }
    explicit(params)?;
    for key in ["name", "documentName"] {
        if params.get(key).is_some_and(|value| value.as_str().is_none_or(|name| name.len() > 2048 || name.contains('\0'))) {
            return Err(bad(format!("{key} must be a bounded string without NUL")));
        }
    }
    if let Some(destination) = params.get("document") {
        let same = destination.as_u64().is_some_and(|index| session.active_index().is_some_and(|active| index == active as u64));
        if destination.as_str() != Some("new") && !same {
            return Err(bad(if destination.as_u64().is_some() {
                "the destination is another open document (use layer.copyToDocument)"
            } else {
                "document must be the current document index or new"
            }));
        }
    }
    if let Some(value) = params.get("inPlace")
        && !value.is_boolean()
    {
        return Err(bad("inPlace must be boolean"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX
        || state.doc.accounted().is_some()
        || state.selection_ids().len() > 10000
        || state.selected_layers.capacity() > MAX_LAYERS * 2
        || state.channel_view.color_hidden.capacity().checked_add(state.channel_view.alpha_visible.capacity()).is_none_or(|n| n > 1 << 20)
    {
        return Err(bad("revision/resident/view admission"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if state.doc.id.0 != 0 && decimal(id) == Some(state.doc.id.0) && decimal(revision) == Some(state.revision) => {}
        _ => return Err(bad("captured target changed or incomplete")),
    }
    if state.floating.is_some()
        || state.coalesce.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
    {
        return Err(bad("finish jobs or gestures before duplicating layers"));
    }
    census(&state.doc, &Interrupt::NONE)?;
    let id = explicit(params)?.or(state.active_layer).ok_or_else(|| bad("no active layer"))?;
    let mut matches = 0usize;
    photocraft_doc::resident_structure::visit_layers_with(&state.doc, &Interrupt::NONE, |layer| {
        if layer.id == id {
            matches += 1;
        }
        Ok(true)
    })?;
    if matches != 1 {
        return Err(bad("missing or ambiguous selected layer"));
    }
    Ok(())
}
fn top_selected(doc: &Document, selection: &[LayerId], active: Option<LayerId>, control: &Interrupt<'_>) -> Result<Vec<LayerId>> {
    let mut output = Vec::new();
    output.try_reserve_exact(selection.len().max(1)).map_err(|_| bad("selection allocation"))?;
    let Some(active) = active else {
        return Ok(output);
    };
    let selected = if selection.contains(&active) { selection } else { std::slice::from_ref(&active) };
    let mut selected_set = HashSet::new();
    selected_set.try_reserve(selected.len()).map_err(|_| bad("selection inventory allocation"))?;
    for id in selected {
        poll(control)?;
        selected_set.insert(*id);
    }
    fn walk(layers: &[Layer], selected: &HashSet<LayerId>, depth: usize, output: &mut Vec<LayerId>, control: &Interrupt<'_>) -> Result<()> {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(bad("layer depth limit"));
        }
        for layer in layers {
            poll(control)?;
            if selected.contains(&layer.id) {
                output.push(layer.id);
            } else if let Some(children) = layer.children() {
                walk(children, selected, depth + 1, output, control)?;
            }
        }
        Ok(())
    }
    walk(&doc.layers, &selected_set, 0, &mut output, control)?;
    poll(control)?;
    Ok(output)
}
fn subtree_count(layer: &Layer, depth: usize, control: &Interrupt<'_>) -> Result<usize> {
    poll(control)?;
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("layer depth limit"));
    }
    let mut count = 1usize;
    if let Some(children) = layer.children() {
        for child in children {
            count = count.checked_add(subtree_count(child, depth + 1, control)?).filter(|n| *n <= MAX_LAYERS).ok_or_else(|| bad("subtree limit"))?;
        }
    }
    Ok(count)
}
pub(crate) fn admission(doc: &Document, targets: &[LayerId], in_place: bool, control: &Interrupt<'_>) -> Result<usize> {
    poll(control)?;
    let count = census(doc, control)?;
    let mut copies = 0usize;
    for id in targets {
        poll(control)?;
        let layer = doc.layer(*id).ok_or(EngineError::NoLayer(*id))?;
        copies = copies.checked_add(subtree_count(layer, 0, control)?).ok_or_else(|| bad("copy count overflow"))?;
        if !in_place {
            crate::artboard_cmds::place_copy_preflight_with_interrupt(doc, *id, control)?;
        }
    }
    let output = count.checked_add(copies).filter(|count| *count <= MAX_LAYERS).ok_or_else(|| bad("duplicated layer count limit"))?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| map_error(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| map_error(control, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let retained = payload
        .checked_mul(3)
        .and_then(|n| n.checked_add(model.checked_mul(12)?))
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(12)?))
        .and_then(|n| n.checked_add((output as u64).checked_mul(4096)?))
        .and_then(|n| n.checked_add(64 << 20));
    if retained.is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES) {
        return Err(bad("duplicate aggregate model/COW/output admission"));
    }
    let metadata = match &doc.xcf_state {
        Some(state) => (state.items.len() as u64)
            .checked_add(state.filter_masks.len() as u64)
            .and_then(|n| n.checked_add(state.filter_stacks.len() as u64))
            .ok_or_else(|| bad("metadata count overflow"))?,
        None => 0,
    };
    let work = (output as u64)
        .checked_add(metadata)
        .and_then(|n| n.checked_pow(2))
        .and_then(|n| n.checked_mul(targets.len().max(1) as u64))
        .and_then(|n| n.checked_mul(64))
        .and_then(|n| n.checked_add(payload.checked_add(model)?.checked_mul(16)?.checked_mul((targets.len() as u64).checked_add(1)?)?));
    if work.is_none_or(|n| n > 8_000_000_000) {
        return Err(bad("duplicate aggregate work admission"));
    }
    poll(control)?;
    Ok(output)
}
fn unique_copy_ids(layer: &mut Layer, ids: &mut HashSet<LayerId>, limit: usize, depth: usize, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("copy depth limit"));
    }
    if layer.id.0 == 0 || ids.contains(&layer.id) {
        let mut found = None;
        for _ in 0..=limit {
            poll(control)?;
            let id = LayerId::fresh();
            if id.0 != 0 && !ids.contains(&id) {
                found = Some(id);
                break;
            }
        }
        layer.id = found.ok_or_else(|| bad("layer identities exhausted"))?;
    }
    if !ids.insert(layer.id) {
        return Err(bad("duplicate generated layer identity"));
    }
    if let Some(children) = layer.children_mut() {
        for child in children {
            unique_copy_ids(child, ids, limit, depth + 1, control)?;
        }
    }
    poll(control)
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let snapshot = state.doc.clone();
    let revision = state.revision;
    let weak = snapshot.downgrade();
    let active = state.active_layer;
    let anchor = state.layer_anchor;
    let mut selection = Vec::new();
    selection.try_reserve_exact(state.selection_ids().len()).map_err(|_| bad("captured selection allocation"))?;
    selection.extend_from_slice(state.selection_ids());
    let view = state.channel_view.clone();
    let multiple = params.get("layer").is_none() && selection.len() > 1;
    let copy_name = if multiple {
        None
    } else if let Some(name) = params.get("name").and_then(Value::as_str) {
        let mut owned = String::new();
        owned.try_reserve_exact(name.len()).map_err(|_| bad("copy name allocation"))?;
        owned.push_str(name);
        Some(owned)
    } else {
        None
    };
    let requested = explicit(params)?.or(active).ok_or_else(|| bad("no active layer"))?;
    let in_place = params.get("inPlace").and_then(Value::as_bool).unwrap_or(false);
    let worker = snapshot.clone();
    let mut work_selection = Vec::new();
    work_selection.try_reserve_exact(selection.len()).map_err(|_| bad("worker selection allocation"))?;
    work_selection.extend_from_slice(&selection);
    let label = if multiple { "Duplicate Layers" } else { "Duplicate Layer" };
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| {
            ctx.stage(0., 0.95, label, |control| {
                let targets = if multiple { top_selected(&worker, &work_selection, active, control)? } else { vec![requested] };
                if targets.is_empty() {
                    return Err(bad("no selected layers"));
                }
                let output = admission(&worker, &targets, in_place, control)?;
                let mut identities = HashSet::new();
                identities.try_reserve(output).map_err(|_| bad("identity inventory allocation"))?;
                photocraft_doc::resident_structure::visit_layers_with(&worker, control, |layer| {
                    if layer.id.0 == 0 || !identities.insert(layer.id) {
                        return Err(photocraft_raster::RasterInputError::InvalidBounds);
                    }
                    Ok(true)
                })?;
                poll(control)?;
                let mut document = (*worker).clone();
                let mut copies = Vec::new();
                copies.try_reserve_exact(targets.len()).map_err(|_| bad("copy selection allocation"))?;
                let mut chosen = None;
                for id in targets {
                    poll(control)?;
                    // Existing duplicate/name/artboard implementations are bounded by
                    // admission above and polled at their boundaries, not inside them.
                    // The shared helper makes Background copies ordinary/unlocked;
                    // every other selected layer retains its existing lock policy.
                    let mut copy = crate::commands::layer_copy(&document, id)?;
                    if let Some(name) = &copy_name {
                        copy.name.clear();
                        copy.name.try_reserve_exact(name.len()).map_err(|_| bad("copy name allocation"))?;
                        copy.name.push_str(name);
                    }
                    poll(control)?;
                    unique_copy_ids(&mut copy, &mut identities, output, 0, control)?;
                    let new_id = document.insert_above(Some(id), copy);
                    poll(control)?;
                    crate::xcf_editing::clone_duplicate_with_interrupt(&mut document, id, new_id, control)?;
                    if !in_place {
                        poll(control)?;
                        crate::artboard_cmds::place_copy_with_interrupt(&mut document, new_id, control)?;
                        poll(control)?;
                    }
                    if Some(id) == active {
                        chosen = Some(new_id);
                    }
                    copies.push(new_id);
                }
                let chosen = if multiple { chosen.or(copies.last().copied()) } else { copies.first().copied() };
                let response = if multiple {
                    json!({"layers":copies.iter().map(|id|id.0).collect::<Vec<_>>()})
                } else {
                    json!({"layer":chosen.ok_or_else(||bad("prepared copy missing"))?.0})
                };
                poll(control)?;
                Ok((document, copies, chosen, response, ctx.clone()))
            })
        },
        move |session, (document, copies, chosen, response, ctx)| {
            ctx.stage(0.95, 1., label, |control| {
                poll(control)?;
                let state = session.active().ok_or(EngineError::NoDocument)?;
                if state.doc.id != snapshot.id
                    || state.revision != revision
                    || !state.doc.shares_state(&snapshot)
                    || !weak.is_current(&state.doc)
                    || state.active_layer != active
                    || state.layer_anchor != anchor
                    || state.selection_ids() != selection
                    || state.channel_view != view
                    || state.floating.is_some()
                    || state.coalesce.is_some()
                    || session.command_has_job_on(snapshot.id)
                    || session.command_has_pending_job()
                    || session.coalesce_request.is_some()
                    || session.color_restrict.is_some()
                {
                    return Err(bad("captured layer/model/selection/view changed"));
                }
                if multiple {
                    session.edit_with_layer_target_with_interrupt(
                        label,
                        control,
                        photocraft_ops::LayerTarget { active: chosen, selected: copies },
                        chosen,
                        move |doc, _| {
                            poll(control)?;
                            *doc = document;
                            Ok(())
                        },
                    )?;
                } else {
                    session.edit_with_interrupt(label, control, move |doc, active| {
                        poll(control)?;
                        *doc = document;
                        *active = chosen;
                        Ok(())
                    })?;
                }
                Ok(response)
            })
        },
    )
}

// Actual command/history/native drafts; deliberately unrun during implementation.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, LayerContent, SampleType, Size};
    fn fixture(sample: SampleType) -> Session {
        let mut doc = Document::new("Own duplicate", Size::new(1, 1), ColorMode::Rgb, sample);
        let mut layer = Layer::raster("Background", doc.pixel_format());
        layer.locks.transparency = true;
        layer.locks.position = true;
        let bytes = match sample {
            SampleType::U32 => [1u32, 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>(),
            SampleType::F64 => [0.5f64.to_bits() + 1, (-0f64).to_bits(), 1f64.to_bits() + 1, 1f64.to_bits()].into_iter().flat_map(u64::to_le_bytes).collect(),
            _ => vec![1, 2, 3, 255],
        };
        layer.surface_mut().unwrap().try_write_interleaved_with(doc.bounds(), &bytes, &Interrupt::NONE).unwrap();
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }
    #[test]
    fn single_background_copy_unlocks_without_narrowing_and_roundtrips_one_undo() {
        for sample in [SampleType::U32, SampleType::F64] {
            let mut session = fixture(sample);
            let before = session.active().unwrap().doc.clone();
            let source = before.layers[0].id;
            let result = run(&mut session, &json!({"layer":source.0})).unwrap();
            let current = &session.active().unwrap().doc;
            let copy = current.layer(LayerId(result["layer"].as_u64().unwrap())).unwrap();
            assert_ne!(copy.id, source);
            assert_eq!(copy.name, "Background copy");
            assert_eq!(copy.locks, Default::default());
            assert_eq!(copy.surface(), before.layers[0].surface());
            assert_eq!(current.layers[0], before.layers[0]);
            let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(current, &Default::default()).unwrap()).unwrap();
            assert_eq!(reopened.layers, current.layers);
            session.execute("edit.undo", Value::Null).unwrap();
            assert_eq!(session.active().unwrap().doc.layers, before.layers);
            session.execute("edit.redo", Value::Null).unwrap();
            assert_eq!(session.active().unwrap().doc.layers, reopened.layers);
        }
    }
    #[test]
    fn multiple_selection_unlocks_only_background_copy_and_retargets_atomically() {
        let mut session = fixture(SampleType::U32);
        session
            .edit("Setup", |doc, active| {
                let mut second = Layer::raster("Second", doc.pixel_format());
                second.locks.position = true;
                *active = Some(second.id);
                doc.layers.push(second);
                Ok(())
            })
            .unwrap();
        let ids = session.active().unwrap().doc.layers.iter().map(|layer| layer.id).collect::<Vec<_>>();
        session.active_mut().unwrap().selected_layers = ids.clone();
        let before = session.active().unwrap().doc.clone();
        let result = run(&mut session, &Value::Null).unwrap();
        let copies = result["layers"].as_array().unwrap().iter().map(|value| LayerId(value.as_u64().unwrap())).collect::<Vec<_>>();
        assert_eq!(copies.len(), 2);
        let state = session.active().unwrap();
        assert_eq!(state.selected_layers, copies);
        assert_eq!(state.active_layer, copies.last().copied());
        assert_eq!(state.layer_anchor, state.active_layer);
        assert_eq!(state.doc.layer(copies[0]).unwrap().locks, Default::default());
        assert_eq!(state.doc.layer(copies[1]).unwrap().locks, before.layers[1].locks);
        assert!(state.doc.layer(copies[1]).unwrap().locks.position);
        assert_eq!(state.doc.layer(ids[0]).unwrap(), &before.layers[0]);
        assert_eq!(state.doc.layer(ids[1]).unwrap(), &before.layers[1]);
        assert_eq!(state.doc.layer(copies[0]).unwrap().surface(), before.layers[0].surface());
        session.execute("edit.undo", Value::Null).unwrap();
        assert_eq!(session.active().unwrap().doc.layers, before.layers);
        assert_eq!(session.active().unwrap().selected_layers, ids);
        session.execute("edit.redo", Value::Null).unwrap();
        assert_eq!(session.active().unwrap().selected_layers, copies);
    }
    #[test]
    fn selected_group_suppresses_child_copy_and_generated_collisions_are_repaired() {
        let mut doc = Document::new("Own group", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let child = Layer::raster("Child", doc.pixel_format());
        let child_id = child.id;
        let group = Layer::group("Group", vec![child]);
        let group_id = group.id;
        doc.layers.push(group);
        assert_eq!(top_selected(&doc, &[group_id, child_id], Some(child_id), &Interrupt::NONE).unwrap(), vec![group_id]);
        let mut copy = doc.layers[0].duplicate();
        copy.id = group_id;
        copy.children_mut().unwrap()[0].id = child_id;
        let mut ids = HashSet::from([group_id, child_id]);
        ids.try_reserve(2).unwrap();
        unique_copy_ids(&mut copy, &mut ids, 4, 0, &Interrupt::NONE).unwrap();
        assert_ne!(copy.id, group_id);
        assert_ne!(copy.children().unwrap()[0].id, child_id);
        assert_ne!(copy.children().unwrap()[0].id, copy.id);
        assert_eq!(doc.layers[0].id, group_id);
    }
    #[test]
    fn stale_bad_params_and_cancelled_preparation_preserve_model_and_real_redo() {
        let mut session = fixture(SampleType::U32);
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
            json!({"inPlace":0}),
            json!({"layer":-1}),
            json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
            json!({"documentId":before.id.0.to_string()}),
        ] {
            assert!(run(&mut session, &params).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(session.active().unwrap().history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        }
        let cancelled = Interrupt::cancel_only(&|| true);
        assert!(matches!(admission(&before, &[before.layers[0].id], true, &cancelled), Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(matches!(before.layers[0].content, LayerContent::Raster(_)));
    }
    #[test]
    fn upstream_new_destination_and_same_index_are_bounded_before_dispatch() {
        let session = fixture(SampleType::U32);
        dispatch_preflight(&session, &json!({"document":"new", "documentName":"Owned copy"}), false).unwrap();
        dispatch_preflight(&session, &json!({"document":session.active_index().unwrap()}), false).unwrap();
        for params in [
            json!({"document":true}),
            json!({"document":1.5}),
            json!({"document":"other"}),
            json!({"documentName":3}),
            json!({"documentName":"x".repeat(2049)}),
        ] {
            assert!(dispatch_preflight(&session, &params, false).is_err());
        }
    }
}
