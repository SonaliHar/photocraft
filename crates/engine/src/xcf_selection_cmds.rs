//! Explicit persisted semantic selection, independent of runtime editing focus.
use crate::{DocumentSnapshot, EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document, LayerId, MAX_GROUP_DEPTH,
    item_identity::{ChannelId, ItemRef, MaskId, PathId},
    xcf_editing::EditingSelection,
};
use photocraft_raster::{Interrupt, backing::ReadControl};
use serde_json::{Map, Value, json};
use std::{collections::HashSet, sync::Arc};

const ID: &str = "xcf.items.select";
const VISIBILITY: &str = "xcf.items.visibility.set";
const MAX_REFS: usize = 1000;
const MAX_ITEMS: usize = 10_000;

fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}
fn check(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn validation(control: &Interrupt<'_>, message: &str) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(message) }
}

fn reference(value: &Value) -> Result<ItemRef> {
    let object = value.as_object().ok_or_else(|| bad("typed reference must be an object"))?;
    if object.len() != 2 || !object.contains_key("kind") || !object.contains_key("id") {
        return Err(bad("typed reference requires exactly kind and id"));
    }
    let text = object.get("id").and_then(Value::as_str).ok_or_else(|| bad("id must be a canonical nonzero decimal string"))?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("id must be a canonical nonzero decimal string"));
    }
    let raw = text.parse::<u64>().map_err(|_| bad("identity overflow"))?;
    match object.get("kind").and_then(Value::as_str) {
        Some("layer") => Ok(ItemRef::Layer(LayerId(raw))),
        Some("channel") => Ok(ItemRef::Channel(ChannelId(raw))),
        Some("mask") => Ok(ItemRef::Mask(MaskId(raw))),
        Some("path") => Ok(ItemRef::Path(PathId(raw))),
        _ => Err(bad("unknown typed reference family")),
    }
}
fn array<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a [Value]> {
    object.get(key).and_then(Value::as_array).map(Vec::as_slice).ok_or_else(|| bad("selection arrays are required"))
}
fn optional(object: &Map<String, Value>, key: &str) -> Result<Option<ItemRef>> {
    match object.get(key) {
        Some(Value::Null) => Ok(None),
        Some(value) => reference(value).map(Some),
        None => Err(bad("all four selection fields are required")),
    }
}
/// Borrow and bound all request fields before dispatcher copying/normalization.
fn envelope(params: &Value) -> Result<&Map<String, Value>> {
    let object = params.as_object().ok_or_else(|| bad("all four selection fields are required"))?;
    if object.len() != 4 || object.keys().any(|key| !matches!(key.as_str(), "selected" | "active" | "selectedPaths" | "maskTarget")) {
        return Err(bad("selection requires exactly selected, active, selectedPaths and maskTarget"));
    }
    let selected = array(object, "selected")?;
    let paths = array(object, "selectedPaths")?;
    let active = optional(object, "active")?;
    let mask = optional(object, "maskTarget")?;
    let count = selected
        .len()
        .checked_add(paths.len())
        .and_then(|n| n.checked_add(usize::from(active.is_some()) + usize::from(mask.is_some())))
        .ok_or_else(|| bad("selection reference limit"))?;
    if count > MAX_REFS {
        return Err(bad("at most 1000 explicit references are supported"));
    }
    for value in selected {
        reference(value)?;
    }
    for value in paths {
        if !matches!(reference(value)?, ItemRef::Path(_)) {
            return Err(bad("selectedPaths accepts paths only"));
        }
    }
    if mask.is_some_and(|r| !matches!(r, ItemRef::Mask(_))) {
        return Err(bad("maskTarget accepts masks only"));
    }
    Ok(object)
}
fn context(session: &Session, background: bool) -> Result<()> {
    if background || session.command_is_background() || session.command_has_pending_job() || session.has_jobs() || session.color_restrict.is_some() {
        return Err(bad("selection requires synchronous idle dispatch"));
    }
    if session.coalesce_request.is_some() {
        return Err(bad("selection cannot coalesce"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.floating.is_some() {
        return Err(bad("complete or cancel every pixel-move preview first"));
    }
    if state.coalesce.is_some() {
        return Err(bad("selection cannot join a coalesced gesture"));
    }
    if state.revision == u64::MAX || state.doc.resident_arc().is_none() {
        return Err(bad("selection requires a resident revision carrier"));
    }
    let xcf = state.doc.xcf_state.as_ref().ok_or_else(|| bad("no retained XCF editing state"))?;
    if xcf.items.len() > MAX_ITEMS || state.selected_layers.len() > MAX_ITEMS {
        return Err(bad("current item limit"));
    }
    Ok(())
}
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    context(session, background)?;
    envelope(params)?;
    Ok(())
}
fn references(values: &[Value], control: &Interrupt<'_>) -> Result<Vec<ItemRef>> {
    let mut output = Vec::new();
    output.try_reserve_exact(values.len()).map_err(|_| bad("selection allocation"))?;
    let mut seen = HashSet::new();
    seen.try_reserve(values.len()).map_err(|_| bad("selection allocation"))?;
    for value in values {
        check(control)?;
        let item = reference(value)?;
        if !seen.insert(item) {
            return Err(bad("duplicate selection identity"));
        }
        output.push(item);
    }
    Ok(output)
}
fn parse(params: &Value, control: &Interrupt<'_>) -> Result<EditingSelection> {
    let object = envelope(params)?;
    let selection = EditingSelection {
        selected: references(array(object, "selected")?, control)?,
        active: optional(object, "active")?,
        selected_paths: references(array(object, "selectedPaths")?, control)?,
        mask_target: optional(object, "maskTarget")?,
    };
    if selection.active.is_some_and(|active| !selection.selected.contains(&active)) {
        return Err(bad("active must be a selected typed identity"));
    }
    Ok(selection)
}
fn current_items(doc: &Document, requested: &EditingSelection, control: &Interrupt<'_>) -> Result<()> {
    let xcf = doc.xcf_state.as_ref().ok_or_else(|| bad("missing retained state"))?;
    if xcf.items.len() > MAX_ITEMS || xcf.item_sets.len() > MAX_ITEMS || doc.channels.len() > MAX_ITEMS || doc.paths.len() > MAX_ITEMS {
        return Err(bad("current item limit"));
    }
    let mut retained_refs = xcf
        .selection
        .selected
        .len()
        .checked_add(xcf.selection.selected_paths.len())
        .filter(|n| *n <= 100_000)
        .ok_or_else(|| bad("retained selection work limit"))?;
    for set in &xcf.item_sets {
        check(control)?;
        retained_refs = retained_refs.checked_add(set.members.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad("retained selection work limit"))?;
    }
    let mut available = HashSet::new();
    available.try_reserve(xcf.items.len()).map_err(|_| bad("item allocation"))?;
    for item in &xcf.items {
        check(control)?;
        if item.item.raw() == 0 || !available.insert(item.item) {
            return Err(bad("zero or duplicate current item identity"));
        }
    }
    for item in requested.selected.iter().chain(&requested.selected_paths).chain(requested.active.iter()).chain(requested.mask_target.iter()) {
        check(control)?;
        if !available.contains(item) {
            return Err(bad("requested typed item is not current"));
        }
    }
    // Fixed frame storage prevents hostile breadth/depth before document clone,
    // resident validation or reconciliation. No recursive tree walk here.
    let mut frames: [Option<std::slice::Iter<'_, photocraft_doc::Layer>>; MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    let mut depth = 0usize;
    *frames.first_mut().ok_or_else(|| bad("hierarchy limit"))? = Some(doc.layers.iter());
    let mut count = 0usize;
    loop {
        check(control)?;
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth = depth.checked_sub(1).ok_or_else(|| bad("hierarchy limit"))?;
            continue;
        };
        count = count.checked_add(1).filter(|n| *n <= MAX_ITEMS).ok_or_else(|| bad("hierarchy limit"))?;
        if let Some(children) = layer.children() {
            depth = depth.checked_add(1).filter(|n| *n <= MAX_GROUP_DEPTH).ok_or_else(|| bad("hierarchy limit"))?;
            *frames.get_mut(depth).ok_or_else(|| bad("hierarchy limit"))? = Some(children.iter());
        }
    }
    doc.validate_xcf_state_with(control).map_err(|_| validation(control, "invalid current XCF state"))
}
fn runtime_target(state: &crate::DocState) -> Result<photocraft_ops::LayerTarget> {
    if state.selected_layers.len() > MAX_ITEMS {
        return Err(bad("runtime focus limit"));
    }
    let mut selected = Vec::new();
    selected.try_reserve_exact(state.selected_layers.len()).map_err(|_| bad("runtime focus allocation"))?;
    selected.extend_from_slice(&state.selected_layers);
    Ok(photocraft_ops::LayerTarget { active: state.active_layer, selected })
}
fn copy_selection(value: &EditingSelection, control: &Interrupt<'_>) -> Result<EditingSelection> {
    let copy = |values: &[ItemRef]| -> Result<Vec<ItemRef>> {
        if values.len() > MAX_REFS {
            return Err(bad("selection copy limit"));
        }
        let mut output = Vec::new();
        output.try_reserve_exact(values.len()).map_err(|_| bad("selection allocation"))?;
        for item in values {
            check(control)?;
            output.push(*item);
        }
        Ok(output)
    };
    Ok(EditingSelection {
        selected: copy(&value.selected)?,
        active: value.active,
        selected_paths: copy(&value.selected_paths)?,
        mask_target: value.mask_target,
    })
}
/// Borrow all retained fields. Exhaustive destructuring makes new state fields
/// require an explicit preservation decision; no cache/resource clone is made.
fn same_nonselection(before: &photocraft_doc::XcfState, after: &photocraft_doc::XcfState) -> bool {
    before.items == after.items && same_nonselection_except_items(before, after)
}
fn same_nonselection_except_items(before: &photocraft_doc::XcfState, after: &photocraft_doc::XcfState) -> bool {
    let photocraft_doc::XcfState {
        provenance,
        items: _,
        resources,
        source_items,
        editable_indexed,
        filter_stacks,
        current_filter_output_precision,
        filter_masks,
        floating,
        selection: _,
        item_sets,
        opaque,
    } = before;
    provenance == &after.provenance
        && resources == &after.resources
        && source_items == &after.source_items
        && editable_indexed == &after.editable_indexed
        && filter_stacks == &after.filter_stacks
        && current_filter_output_precision == &after.current_filter_output_precision
        && filter_masks == &after.filter_masks
        && floating == &after.floating
        && item_sets == &after.item_sets
        && opaque == &after.opaque
}

fn visibility_error(error: EngineError) -> EngineError {
    match error {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: VISIBILITY.into(), msg },
        other => other,
    }
}
fn visibility_bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: VISIBILITY.into(), msg: message.into() }
}
fn visibility_envelope(params: &Value) -> Result<(&[Value], bool)> {
    let object = params.as_object().ok_or_else(|| visibility_bad("items and visible are required"))?;
    if object.len() != 2 || !object.contains_key("items") || !object.contains_key("visible") {
        return Err(visibility_bad("requires exactly items and visible"));
    }
    let items = object
        .get("items")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= MAX_REFS)
        .ok_or_else(|| visibility_bad("items must contain 1..1000 typed targets"))?;
    let visible = object.get("visible").and_then(Value::as_bool).ok_or_else(|| visibility_bad("visible must be a boolean"))?;
    for value in items {
        if !matches!(reference(value).map_err(visibility_error)?, ItemRef::Layer(_) | ItemRef::Channel(_)) {
            return Err(visibility_bad("visibility targets must be current layers or channels"));
        }
    }
    Ok((items, visible))
}
pub(crate) fn visibility_dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    context(session, background).map_err(visibility_error)?;
    visibility_envelope(params)?;
    Ok(())
}
fn same_visibility_state(before: &photocraft_doc::XcfState, after: &photocraft_doc::XcfState, targets: &HashSet<ItemRef>, visible: bool) -> bool {
    if !same_nonselection_except_items(before, after) || before.selection != after.selection || before.items.len() != after.items.len() {
        return false;
    }
    before.items.iter().zip(&after.items).all(|(old, new)| {
        let photocraft_doc::xcf_state::ItemState {
            item,
            name,
            custom_name,
            visible: _,
            locks,
            parent,
            source_tattoo,
            source_flags,
            raster_bounds,
            mask,
            composition,
        } = old;
        let expected = if targets.contains(item) { visible } else { old.visible };
        new.visible == expected
            && item == &new.item
            && name == &new.name
            && custom_name == &new.custom_name
            && locks == &new.locks
            && parent == &new.parent
            && source_tattoo == &new.source_tattoo
            && source_flags == &new.source_flags
            && raster_bounds == &new.raster_bounds
            && mask == &new.mask
            && composition == &new.composition
    })
}

/// Persisted channel visibility is metadata, not a Channels-panel view change.
/// Actual layer visibility and its canonical metadata are changed together.
pub fn visibility_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    check(control)?;
    visibility_dispatch_preflight(session, params, false)?;
    let (values, visible) = visibility_envelope(params)?;
    let items = references(values, control).map_err(visibility_error)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    // Reuse exact current typed membership and complete validator admission.
    let requested = EditingSelection { selected: items, ..EditingSelection::default() };
    current_items(&state.doc, &requested, control).map_err(visibility_error)?;
    let baseline = state.doc.xcf_state.as_ref().ok_or_else(|| visibility_bad("missing retained state"))?;
    let mut targets = HashSet::new();
    targets.try_reserve(requested.selected.len()).map_err(|_| visibility_bad("target allocation"))?;
    targets.extend(requested.selected.iter().copied());
    let mut changed_layers = Vec::new();
    changed_layers.try_reserve_exact(requested.selected.len()).map_err(|_| visibility_bad("target allocation"))?;
    let mut changed = 0usize;
    for target in &requested.selected {
        check(control)?;
        let item = baseline.items.iter().find(|item| item.item == *target).ok_or_else(|| visibility_bad("missing current typed target"))?;
        if let ItemRef::Layer(id) = target {
            let layer = state.doc.layer(*id).ok_or_else(|| visibility_bad("missing current native layer"))?;
            if layer.visible != item.visible {
                return Err(visibility_bad("native and imported layer visibility disagree"));
            }
        }
        if item.visible != visible {
            changed += 1;
            match target {
                ItemRef::Layer(id) => changed_layers.push(*id),
                ItemRef::Channel(_) => {
                    crate::xcf_item_locks::effective_item_locks(&state.doc, *target)?.require(crate::xcf_item_locks::EditKind::Visibility)?
                }
                _ => return Err(visibility_bad("unsupported visibility target")),
            }
        }
    }
    crate::xcf_item_locks::require_visibility_changes(&state.doc, &changed_layers, control)?;
    if changed == 0 {
        check(control)?;
        return Ok(json!({"updated":false,"changed":0}));
    }
    let footprint = state.doc.resident_raster_footprint_with(control).map_err(|_| visibility_error(validation(control, "retained raster limit")))?;
    footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes)
        .filter(|n| *n <= photocraft_raster::MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| visibility_bad("retained raster and COW peak limit"))?;
    let before = state.doc.clone();
    let mut candidate = (*before).clone();
    for id in changed_layers {
        check(control)?;
        candidate.layer_mut(id).ok_or_else(|| visibility_bad("missing staged layer"))?.visible = visible;
    }
    let mut xcf = candidate.xcf_state.take().ok_or_else(|| visibility_bad("missing staged state"))?;
    for item in &mut xcf.items {
        check(control)?;
        if targets.contains(&item.item) {
            item.visible = visible;
        }
    }
    xcf.reconcile_layers(&candidate).map_err(|_| visibility_error(validation(control, "cannot preserve current visibility state")))?;
    if !same_visibility_state(baseline, &xcf, &targets, visible) {
        return Err(visibility_bad("visibility reconciliation would change other retained metadata"));
    }
    candidate.xcf_state = Some(xcf);
    candidate.validate_xcf_state_with(control).map_err(|_| visibility_error(validation(control, "invalid staged visibility state")))?;
    let snapshot = DocumentSnapshot::from_resident(Arc::new(candidate))?;
    let prior = runtime_target(state).map_err(visibility_error)?;
    let layers = runtime_target(state).map_err(visibility_error)?;
    let read = ReadControl { interrupt: control, work: &|_| Ok(()) };
    let mut history = state.history.clone();
    history.set_current_layers(prior, &read)?;
    history.record("Set Saved XCF Item Visibility", before, layers, &read)?;
    history.trim(&snapshot, &read)?;
    check(control)?;
    let state = session.active_mut().ok_or(EngineError::NoDocument)?;
    state.doc = snapshot;
    state.history = history;
    state.revision += 1;
    state.last_damage = None;
    Ok(json!({"updated":true,"changed":changed,"visible":visible,"revisionString":state.revision.to_string()}))
}

/// Stage metadata and history together. Runtime focus fields are deliberately
/// not passed through Session::edit's layer/channel focus normalization.
pub fn select_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    check(control)?;
    dispatch_preflight(session, params, false)?;
    let requested = parse(params, control)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    current_items(&state.doc, &requested, control)?;
    if state.doc.xcf_state.as_ref().is_some_and(|xcf| xcf.selection == requested) {
        check(control)?;
        return Ok(json!({"updated":false}));
    }
    let footprint = state.doc.resident_raster_footprint_with(control).map_err(|_| validation(control, "retained raster limit"))?;
    footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes)
        .filter(|peak| *peak <= photocraft_raster::MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad("retained raster and COW metadata peak limit"))?;
    let before = state.doc.clone();
    let baseline = before.xcf_state.as_ref().ok_or_else(|| bad("missing retained state"))?;
    let mut candidate = (*before).clone();
    let mut xcf = candidate.xcf_state.take().ok_or_else(|| bad("missing retained state"))?;
    xcf.selection = copy_selection(&requested, control)?;
    check(control)?;
    xcf.reconcile_layers(&candidate).map_err(|_| validation(control, "cannot preserve current typed selection"))?;
    if xcf.selection != requested || !same_nonselection(baseline, &xcf) {
        return Err(bad("reconciliation would alter retained state beyond semantic selection"));
    }
    candidate.xcf_state = Some(xcf);
    candidate.validate_xcf_state_with(control).map_err(|_| validation(control, "invalid requested semantic selection"))?;
    let snapshot = DocumentSnapshot::from_resident(Arc::new(candidate))?;
    let prior = runtime_target(state)?;
    let layers = runtime_target(state)?;
    let read = ReadControl { interrupt: control, work: &|_| Ok(()) };
    let mut history = state.history.clone();
    history.set_current_layers(prior, &read)?;
    history.record("Select Saved XCF Items", before, layers, &read)?;
    history.trim(&snapshot, &read)?;
    // All fallible work and cancellation precede both assignments. No UI focus,
    // source identity, locator, raster/cache or grant is changed by publication.
    check(control)?;
    let state = session.active_mut().ok_or(EngineError::NoDocument)?;
    state.doc = snapshot;
    state.history = history;
    state.revision += 1;
    state.last_damage = None;
    Ok(json!({"updated":true,"revisionString":state.revision.to_string()}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: ID,
            label: "Select Saved XCF Items",
            menu: &[],
            shortcut: None,
            params: r#"{selected:[{kind:layer|channel|mask|path,id:canonical nonzero U64 string}],active:{kind,id}|null,selectedPaths:[{kind:path,id}],maskTarget:{kind:mask,id}|null}; all four fields, max1000 refs, one undoable semantic selection; runtime focus and grants unchanged"#,
            enabled: |s| s.active().and_then(|d| d.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no retained XCF editing state".into()),
            journal: true,
            run: |s, p| select_with_interrupt(s, p, &Interrupt::NONE),
        },
        CommandSpec {
            id: VISIBILITY,
            label: "Set Saved XCF Item Visibility",
            menu: &[],
            shortcut: None,
            params: r#"{items:[{kind:layer|channel,id:canonical nonzero U64 string}],visible:bool}; 1..1000 unique current typed targets, inherited visibility locks, one atomic undo; source flags and runtime focus unchanged"#,
            enabled: |s| s.active().and_then(|d| d.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no retained XCF editing state".into()),
            journal: true,
            run: |s, p| visibility_with_interrupt(s, p, &Interrupt::NONE),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, Layer, SampleType, Size, XcfState};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn setup() -> (Session, LayerId, LayerId) {
        let mut doc = Document::with_background("saved selection", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let first = doc.layers.first().unwrap().id;
        let second = Layer::raster("other", photocraft_color::PixelFormat::RGBA8);
        let second_id = second.id;
        doc.layers.push(second);
        let mut xcf = XcfState::default();
        xcf.reconcile_layers(&doc).unwrap();
        doc.xcf_state = Some(xcf);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session.select_layer(first).unwrap();
        (session, first, second_id)
    }
    fn request(id: LayerId) -> Value {
        json!({"selected":[{"kind":"layer","id":id.0.to_string()}],"active":{"kind":"layer","id":id.0.to_string()},"selectedPaths":[],"maskTarget":null})
    }

    #[test]
    fn semantic_selection_history_native_roundtrip_and_runtime_focus_are_independent() {
        let (mut session, first, second) = setup();
        let before = session.active().unwrap().doc.clone();
        let view = session.active().unwrap().channel_view.clone();
        let focus = session.active().unwrap().selected_layers.clone();
        assert_eq!(select_with_interrupt(&mut session, &request(second), &Interrupt::NONE).unwrap()["updated"], true);
        let current = session.active().unwrap();
        assert_eq!(current.active_layer, Some(first));
        assert_eq!(current.selected_layers, focus);
        assert_eq!(current.channel_view, view);
        let mut expected = (*before).clone();
        expected.xcf_state.as_mut().unwrap().selection = parse(&request(second), &Interrupt::NONE).unwrap();
        assert_eq!(*current.doc, expected);
        let bytes = photocraft_format::save_to_bytes(&current.doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), expected);
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc, before);
        assert!(session.redo());
        assert_eq!(*session.active().unwrap().doc, expected);
    }

    #[test]
    fn malformed_and_late_unknown_reference_preserve_native_source_and_real_redo() {
        let (mut session, _, second) = setup();
        session
            .edit("Real redo", |doc, _| {
                doc.name = "redo name".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let mut missing = request(second);
        missing["selected"].as_array_mut().unwrap().push(json!({"kind":"path","id":"18446744073709551615"}));
        let mut duplicate = request(second);
        duplicate["selected"].as_array_mut().unwrap().push(json!({"kind":"layer","id":second.0.to_string()}));
        let mut wrong = request(second);
        wrong["maskTarget"] = json!({"kind":"layer","id":second.0.to_string()});
        for value in [missing, duplicate, wrong, Value::Null, json!({"selected":[],"active":null,"selectedPaths":[]})] {
            assert!(select_with_interrupt(&mut session, &value, &Interrupt::NONE).is_err());
            assert_eq!(session.active().unwrap().doc, before);
            assert_eq!(session.active().unwrap().revision, revision);
            assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), native);
        }
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc.name, "redo name");
    }

    #[test]
    fn every_observed_cancellation_fence_is_atomic_through_history_publication() {
        let (mut completed, _, second) = setup();
        let count = AtomicUsize::new(0);
        let measure = || {
            count.fetch_add(1, Ordering::Relaxed);
            false
        };
        select_with_interrupt(&mut completed, &request(second), &Interrupt::cancel_only(&measure)).unwrap();
        let checks = count.load(Ordering::Relaxed);
        assert!(checks > 1);
        for threshold in 0..checks {
            let (mut session, _, second) = setup();
            session
                .edit("Retained redo", |doc, _| {
                    doc.name = "real redo".into();
                    Ok(())
                })
                .unwrap();
            assert!(session.undo());
            let before = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            let calls = AtomicUsize::new(0);
            let stop = || calls.fetch_add(1, Ordering::Relaxed) >= threshold;
            let result = select_with_interrupt(&mut session, &request(second), &Interrupt::cancel_only(&stop));
            if matches!(result, Err(EngineError::Cancelled)) {
                assert_eq!(session.active().unwrap().doc, before);
                assert_eq!(session.active().unwrap().revision, revision);
                assert!(session.redo());
                assert_eq!(session.active().unwrap().doc.name, "real redo");
            } else {
                assert!(result.is_ok());
            }
        }
    }

    #[test]
    fn typed_layer_and_saved_channel_visibility_preserve_source_flags_pixels_and_view() {
        let (initial, first, _) = setup();
        let mut doc = (*initial.active().unwrap().doc).clone();
        let channel = photocraft_doc::AlphaChannel::new("saved alpha", photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8));
        let channel_id = channel.id;
        doc.channels.push(channel);
        let mut xcf = doc.xcf_state.take().unwrap();
        xcf.reconcile_layers(&doc).unwrap();
        for item in &mut xcf.items {
            item.source_flags = 0x12345678;
            if item.item == ItemRef::Channel(channel_id) {
                item.visible = true;
            }
        }
        doc.xcf_state = Some(xcf);
        doc.layer_mut(first).unwrap().locks.all = true;
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        let before = session.active().unwrap().doc.clone();
        let view = session.active().unwrap().channel_view.clone();
        let active = session.active().unwrap().active_layer;
        let request = json!({"items":[{"kind":"layer","id":first.0.to_string()},{"kind":"channel","id":channel_id.0.to_string()}],"visible":false});
        assert_eq!(visibility_with_interrupt(&mut session, &request, &Interrupt::NONE).unwrap()["changed"], 2);
        let mut expected = (*before).clone();
        expected.layer_mut(first).unwrap().visible = false;
        for item in &mut expected.xcf_state.as_mut().unwrap().items {
            if item.item == ItemRef::Layer(first) || item.item == ItemRef::Channel(channel_id) {
                item.visible = false;
            }
        }
        assert_eq!(*session.active().unwrap().doc, expected);
        assert_eq!(session.active().unwrap().channel_view, view);
        assert_eq!(session.active().unwrap().active_layer, active);
        let bytes = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), expected);
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc, before);
        assert!(session.redo());
        assert_eq!(*session.active().unwrap().doc, expected);
    }

    #[test]
    fn late_locked_or_wrong_family_visibility_target_refuses_entire_batch_and_preserves_redo() {
        let (mut session, first, second) = setup();
        session
            .edit("Lock saved visibility", |doc, _| {
                doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(second)).unwrap().locks.visibility = true;
                Ok(())
            })
            .unwrap();
        session
            .edit("Real redo", |doc, _| {
                doc.name = "redo".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        for request in [
            json!({"items":[{"kind":"layer","id":first.0.to_string()},{"kind":"layer","id":second.0.to_string()}],"visible":false}),
            json!({"items":[{"kind":"mask","id":"1"}],"visible":false}),
            json!({"items":[{"kind":"layer","id":first.0.to_string()}],"visible":"false"}),
            json!({"items":[{"kind":"layer","id":first.0.to_string()},{"kind":"layer","id":first.0.to_string()}],"visible":false}),
        ] {
            assert!(visibility_with_interrupt(&mut session, &request, &Interrupt::NONE).is_err());
            assert_eq!(session.active().unwrap().doc, before);
            assert_eq!(session.active().unwrap().revision, revision);
        }
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc.name, "redo");
    }

    #[test]
    fn visibility_cancellation_before_install_preserves_native_bytes_and_history() {
        let (mut session, first, _) = setup();
        session
            .edit("Real redo", |doc, _| {
                doc.name = "redo".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let request = json!({"items":[{"kind":"layer","id":first.0.to_string()}],"visible":false});
        let count = AtomicUsize::new(0);
        let stop = || count.fetch_add(1, Ordering::Relaxed) >= 12;
        assert!(matches!(visibility_with_interrupt(&mut session, &request, &Interrupt::cancel_only(&stop)), Err(EngineError::Cancelled)));
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc.name, "redo");
    }

    #[test]
    fn imported_parent_visibility_lock_is_enforced_for_explicit_child_target() {
        let (initial, child, _) = setup();
        let mut doc = (*initial.active().unwrap().doc).clone();
        let group = Layer::group("locked ancestor", std::mem::take(&mut doc.layers));
        let parent = group.id;
        doc.layers.push(group);
        let mut xcf = doc.xcf_state.take().unwrap();
        xcf.reconcile_layers(&doc).unwrap();
        xcf.items.iter_mut().find(|item| item.item == ItemRef::Layer(parent)).unwrap().locks.visibility = true;
        doc.xcf_state = Some(xcf);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        let before = session.active().unwrap().doc.clone();
        assert!(
            visibility_with_interrupt(&mut session, &json!({"items":[{"kind":"layer","id":child.0.to_string()}],"visible":false}), &Interrupt::NONE).is_err()
        );
        assert_eq!(session.active().unwrap().doc, before);
    }
}
