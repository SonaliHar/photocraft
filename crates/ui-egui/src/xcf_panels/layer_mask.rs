//! Current canonical layer-mask flag, dispatched without changing shell selection.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{
    DocId, LayerId,
    item_identity::{ItemRef, MaskId},
};
use photocraft_engine::WeakDocumentSnapshot;
use serde_json::json;
fn stale() -> String {
    tl!("The layer mask changed. Select it again before editing.").into()
}
struct Target {
    document: DocId,
    revision: u64,
    layer: LayerId,
    mask: MaskId,
    enabled: bool,
    snapshot: WeakDocumentSnapshot,
}
fn association(app: &PhotocraftApp, layer: LayerId) -> Option<(MaskId, bool)> {
    let active = app.session.active()?;
    let state = active.doc.xcf_state.as_ref()?;
    if state.items.len() > 10_000 || state.filter_masks.len() > 10_000 {
        return None;
    }
    let native = active.doc.layer(layer)?.mask.as_ref()?;
    let mut filters = std::collections::HashSet::new();
    filters.try_reserve(state.filter_masks.len()).ok()?;
    for mask in &state.filter_masks {
        filters.insert(mask.id);
    }
    let mut target = None;
    for item in &state.items {
        let Some(metadata) = &item.mask else {
            continue;
        };
        if metadata.owner != ItemRef::Layer(layer) {
            continue;
        }
        let ItemRef::Mask(id) = item.item else {
            return None;
        };
        if filters.contains(&id) {
            continue;
        }
        if id.0 == 0
            || target.is_some()
            || metadata.enabled != native.enabled
            || metadata.linked != native.linked
            || metadata.bounds.is_empty()
            || metadata.offset != [metadata.bounds.x0, metadata.bounds.y0]
            || item.raster_bounds.is_some_and(|bounds| bounds != metadata.bounds)
        {
            return None;
        }
        target = Some((id, metadata.enabled));
    }
    target
}
fn capture(app: &PhotocraftApp, layer: LayerId) -> Option<Target> {
    let (mask, enabled) = association(app, layer)?;
    let active = app.session.active()?;
    Some(Target { document: active.doc.id, revision: active.revision, layer, mask, enabled, snapshot: active.doc.downgrade() })
}
fn permitted(app: &PhotocraftApp, target: &Target) -> bool {
    let Some(active) = app.session.active() else {
        return false;
    };
    if app.ui.transform.is_some()
        || active.floating.is_some()
        || active.coalesce.is_some()
        || app.session.has_jobs()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return false;
    }
    let Ok(locks) = photocraft_engine::xcf_item_locks::effective_item_locks(&active.doc, ItemRef::Mask(target.mask)) else {
        return false;
    };
    // Pixels-only native locks do not prohibit the separate mask operation.
    !locks.native.all && !locks.imported.content && photocraft_engine::xcf_item_locks::require_layer_mask_content(&active.doc, target.layer).is_ok()
}
fn apply(app: &mut PhotocraftApp, target: &Target, enabled: bool) -> Result<serde_json::Value, String> {
    let active = app.session.active().ok_or_else(stale)?;
    let snapshot = target.snapshot.upgrade().ok_or_else(stale)?;
    if active.doc.id != target.document
        || active.revision != target.revision
        || !active.doc.shares_state(&snapshot)
        || association(app, target.layer) != Some((target.mask, target.enabled))
    {
        return Err(stale());
    }
    if !permitted(app, target) {
        return Err(tl!("Finish pending edits and unlock the layer mask before editing.").into());
    }
    // Immediate registered dispatch after revalidation: no refocus, preview drop or direct edit.
    app.run("xcf.mask.enabled.set", json!({"maskId":target.mask.0.to_string(),"enabled":enabled}))
}
/// Prepare queued storage parameters before any queued focus change. No source authority is granted.
pub(crate) fn queued_params(app: &PhotocraftApp, layer: LayerId, on_set: bool, command: &str) -> Result<serde_json::Value, String> {
    if !matches!(
        command,
        "layer.layerMask.revealAll"
            | "layer.layerMask.hideAll"
            | "layer.layerMask.delete"
            | "layer.layerMask.apply"
            | "layer.layerMask.hideSelection"
            | "layer.layerMask.fromTransparency"
            | "layer.layerMask.revealSelection"
    ) {
        return Err(tl!("This layer mask action is unavailable.").into());
    }
    let active = app.session.active().ok_or_else(|| tl!("Open a document before editing a layer mask.").to_owned())?;
    if active.doc.id.0 == 0 || active.revision == u64::MAX || active.doc.layer(layer).is_none() {
        return Err(stale());
    }
    if app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.coalesce.is_some()
        || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(tl!("Finish pending edits before editing a layer mask.").into());
    }
    if on_set && active.selected_layers.len() > 10_000 {
        return Err(tl!("Too many selected layers for this mask action.").into());
    }
    // The controlled engine resolves the selected owner order and qualifies each operation.
    // Omit an explicit layer for a selected-set action so its established eligibility is retained.
    let mut params = json!({"documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string()});
    if !on_set {
        photocraft_engine::xcf_item_locks::require_layer_mask_content(&active.doc, layer).map_err(|error| error.to_string())?;
        if let Some(object) = params.as_object_mut() {
            object.insert("layer".into(), json!(layer.0));
        }
    }
    Ok(params)
}
/// A menu captures this witness before any focus or selection change. It is never persisted.
#[derive(Clone)]
pub(crate) struct ActionTarget {
    snapshot: photocraft_engine::document_snapshot::DocumentSnapshot,
    revision: u64,
    layer: LayerId,
    mask: Option<(MaskId, bool)>,
}
pub(crate) fn capture_action(app: &PhotocraftApp, layer: LayerId) -> Result<ActionTarget, String> {
    let active = app.session.active().ok_or_else(|| tl!("Open a document before editing a layer mask.").to_owned())?;
    if layer.0 == 0 || active.doc.layer(layer).is_none() {
        return Err(stale());
    }
    Ok(ActionTarget { snapshot: active.doc.clone(), revision: active.revision, layer, mask: association(app, layer) })
}
/// Dispatch only the existing storage operations. The engine owns qualification and allocation.
pub(crate) fn run_action(app: &mut PhotocraftApp, target: &ActionTarget, command: &str) -> Result<serde_json::Value, String> {
    if !matches!(
        command,
        "layer.layerMask.revealAll"
            | "layer.layerMask.hideAll"
            | "layer.layerMask.delete"
            | "layer.layerMask.apply"
            | "layer.layerMask.hideSelection"
            | "layer.layerMask.fromTransparency"
            | "layer.layerMask.revealSelection"
    ) {
        return Err(tl!("This layer mask action is unavailable.").into());
    }
    let active = app.session.active().ok_or_else(stale)?;
    if active.doc.id != target.snapshot.id
        || active.revision != target.revision
        || !active.doc.shares_state(&target.snapshot)
        || active.doc.layer(target.layer).is_none()
        || association(app, target.layer) != target.mask
    {
        return Err(stale());
    }
    if app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.coalesce.is_some()
        || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(tl!("Finish pending edits before editing a layer mask.").into());
    }
    photocraft_engine::xcf_item_locks::require_layer_mask_content(&active.doc, target.layer).map_err(|error| error.to_string())?;
    app.run(command, json!({"layer":target.layer.0,"documentId":target.snapshot.id.0.to_string(),"expectedRevision":target.revision.to_string()}))
}
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let Some(target) = capture(app, layer) else {
        return;
    };
    super::source_inspect::owner_properties(app, ui, photocraft_doc::item_identity::ItemRef::Mask(target.mask));
    let tokens = Tokens::get(ui.ctx());
    let editable = permitted(app, &target);
    let mut enabled = target.enabled;
    ui.separator();
    ui.label(egui::RichText::new(tl!("Layer Mask")).color(tokens.text));
    let changed = ui
        .push_id(("xcf-current-layer-mask", target.document.0, target.mask.0), |ui| {
            ui.add_enabled_ui(editable, |ui| crate::widgets::checkbox(ui, &mut enabled, tl!("Enable Layer Mask")).changed()).inner
        })
        .inner;
    let action = capture_action(app, layer);
    ui.horizontal(|ui| {
        for (label, command) in [(tl!("Apply Layer Mask"), "layer.layerMask.apply"), (tl!("Delete Layer Mask"), "layer.layerMask.delete")] {
            if ui.add_enabled(editable, egui::Button::new(label)).clicked() {
                let result = action.as_ref().map_err(Clone::clone).and_then(|captured| run_action(app, captured, command));
                if let Err(error) = result {
                    app.ui.status = error;
                    app.ui.status_error = true;
                }
            }
        }
    });
    if changed && let Err(error) = apply(app, &target, enabled) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "layer_mask/assigned_geometry_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::unwrap_used)]
mod assigned_geometry_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PhotocraftApp, LayerId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let doc = app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        let layer = doc.layers.first().unwrap().id;
        doc.layer_mut(layer).unwrap().mask = Some(photocraft_doc::LayerMask::hide_all());
        doc.sample_encoding = Some(photocraft_doc::SampleEncoding::NonlinearProfile);
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        let mut state = photocraft_doc::XcfState::default();
        state.reconcile_layers(doc).unwrap();
        let bounds = doc.bounds();
        state.items.push(photocraft_doc::xcf_state::ItemState {
            item: ItemRef::Mask(MaskId(1)),
            name: "Own mask".into(),
            custom_name: true,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(bounds),
            composition: None,
            mask: Some(photocraft_doc::xcf_state::MaskState {
                owner: ItemRef::Layer(layer),
                bounds,
                offset: [bounds.x0, bounds.y0],
                outside_f64_bits: 0.0f64.to_bits(),
                enabled: true,
                shown: false,
                editing: false,
                linked: true,
            }),
        });
        doc.xcf_state = Some(state);
        (app, layer)
    }
    #[test]
    fn guarded_mask_control_changes_only_flags_and_preserves_focus_undo() {
        let (mut app, layer) = fixture();
        let target = capture(&app, layer).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let focus = app.session.active().unwrap().active_layer;
        apply(&mut app, &target, false).unwrap();
        let mut expected = before.as_ref().clone();
        expected.layer_mut(layer).unwrap().mask.as_mut().unwrap().enabled = false;
        expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Mask(target.mask)).unwrap().mask.as_mut().unwrap().enabled =
            false;
        assert_eq!(app.session.active().unwrap().doc.as_ref(), &expected);
        assert_eq!(app.session.active().unwrap().active_layer, focus);
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, before);
    }
    #[test]
    fn stale_and_lock_refusal_never_dispatches() {
        let (mut app, layer) = fixture();
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().layer_mut(layer).unwrap().locks.pixels = true;
        let target = capture(&app, layer).unwrap();
        assert!(permitted(&app, &target));
        app.session.active_mut().unwrap().revision += 1;
        let before = app.session.active().unwrap().doc.clone();
        assert_eq!(apply(&mut app, &target, false).unwrap_err(), stale());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().layer_mut(layer).unwrap().locks.all = true;
        let target = capture(&app, layer).unwrap();
        assert!(!permitted(&app, &target));
        let before = app.session.active().unwrap().doc.clone();
        assert!(apply(&mut app, &target, false).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn captured_storage_action_refuses_revision_transform_and_restored_mask_owner() {
        let (mut app, layer) = fixture();
        let target = capture_action(&app, layer).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        app.session.active_mut().unwrap().revision += 1;
        assert_eq!(run_action(&mut app, &target, "layer.layerMask.delete").unwrap_err(), stale());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let target = capture_action(&app, layer).unwrap();
        app.session.active_mut().unwrap().coalesce = Some("Own mask gesture".into());
        assert!(run_action(&mut app, &target, "layer.layerMask.apply").is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert!(run_action(&mut app, &target, "layer.vectorMask.delete").is_err());
    }

    #[test]
    fn queued_storage_keeps_single_owner_and_selected_set_parameter_policy() {
        let (mut app, layer) = fixture();
        let revision = app.session.active().unwrap().revision;
        let single = queued_params(&app, layer, false, "layer.layerMask.delete").unwrap();
        assert_eq!(single["layer"], json!(layer.0));
        assert_eq!(single["expectedRevision"], json!(revision.to_string()));
        let selected = queued_params(&app, layer, true, "layer.layerMask.hideAll").unwrap();
        assert!(selected.get("layer").is_none());
        assert_eq!(single["documentId"], selected["documentId"]);
        for command in ["layer.layerMask.hideSelection", "layer.layerMask.fromTransparency"] {
            let captured = queued_params(&app, layer, false, command).unwrap();
            assert_eq!(captured, single);
            let captured_set = queued_params(&app, layer, true, command).unwrap();
            assert_eq!(captured_set, selected);
        }
        app.session.active_mut().unwrap().coalesce = Some("Own menu gesture".into());
        assert!(queued_params(&app, layer, true, "layer.layerMask.hideAll").is_err());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "layer_mask/isolated_group_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod isolated_group_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "layer_mask/gaussian_group_metadata_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod gaussian_group_metadata_tests;
