//! Explicit exact recovery or Reduced replacement through registered linked-image commands.
use crate::PhotocraftApp;
use photocraft_doc::{LayerId, item_identity::ItemRef, xcf_items::EditableSource, xcf_resources::ResourceKind};

/// Retained identity is view data; the click and completed picker recheck the target.
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let Some((captured, revision)) = app.session.active().map(|active| (active.doc.clone(), active.revision)) else { return };
    let Some((resource, exact)) = retained_resource(&captured, layer) else { return };
    ui.separator();
    ui.label(tl!("GIMP Linked Image"));
    if exact {
        ui.label(tl!("The selected PNG must match the retained original byte identity (8 MiB limit). Exact recovery applies to this linked image only."));
    } else {
        ui.label(tl!("Original byte identity is unavailable. The selected PNG is a Reduced substitution (8 MiB limit)."));
    }
    let available = app.services.file_dialog.is_some() && !app.file_dialog_open() && live_context(app, layer).is_ok();
    if app.services.file_dialog.is_none() {
        ui.label(tl!("Resource picker unavailable."));
    }
    let owned = has_owned_material(&captured, resource);
    let can_refresh = owned && live_context(app, layer).is_ok();
    if ui.add_enabled(can_refresh, egui::Button::new(tl!("Refresh"))).clicked() {
        let result = run_frame(app, &captured, revision, layer, "xcf.link.refresh", serde_json::json!({"layerId":layer.0.to_string()}));
        show_error(app, result);
    }
    geometry_properties(app, ui, &captured, revision, layer, owned);
    let label = if exact { tl!("Recover Linked Image (Exact)…") } else { tl!("Replace Linked Image (Reduced)…") };
    if ui.add_enabled(available, egui::Button::new(label)).clicked() {
        let result = validate_frame(app, &captured, revision, layer).and_then(|()| if exact {
            photocraft_engine::xcf_link_substitution_cmds::capture_recovery_target(&app.session, layer, resource).map_err(|error| error.to_string()).and_then(
                |target| {
                    app.pick_xcf_link_bytes(move |app, _name, bytes| {
                        live_context(app, layer)?;
                        let request = photocraft_engine::xcf_link_substitution_cmds::request_for_recovery_target(&app.session, &target, &bytes)
                            .map_err(|error| error.to_string())?;
                        app.run("xcf.link.recover", request)
                    })
                },
            )
        } else {
            photocraft_engine::xcf_link_substitution_cmds::capture_target(&app.session, layer, resource).map_err(|error| error.to_string()).and_then(|target| {
                app.pick_xcf_link_bytes(move |app, _name, bytes| {
                    live_context(app, layer)?;
                    let request =
                        photocraft_engine::xcf_link_substitution_cmds::request_for_target(&app.session, &target, &bytes).map_err(|error| error.to_string())?;
                    app.run("xcf.link.substitute", request)
                })
            })
        });
        if let Err(error) = result {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
}

fn has_owned_material(doc: &photocraft_doc::Document, resource: photocraft_doc::item_identity::ResourceId) -> bool {
    doc.xcf_state.as_ref().filter(|state| state.resources.len() <= 10_000).is_some_and(|state|
        state.resources.iter().find(|entry| entry.id == resource).is_some_and(|entry| entry.resolution.is_some() && entry.owned_bytes.as_ref().is_some_and(|bytes| !bytes.is_empty() && bytes.len() <= 8 << 20)))
}
struct GeometryHints { origin: [i32;2], transform: Option<photocraft_doc::xcf_items::TransformState> }
fn geometry_hints(doc: &photocraft_doc::Document, layer: LayerId) -> Option<GeometryHints> {
    let state = doc.xcf_state.as_ref()?;
    if state.items.len() > 10_000 || state.source_items.len() > 10_000 { return None; }
    let source = state.source_items.iter().find(|source| source.owner == ItemRef::Layer(layer))?;
    let EditableSource::Linked { current, .. } = &source.source else { return None };
    let bounds = current.as_ref().map(|current|current.bounds).or_else(|| state.items.iter().find(|item| item.item == ItemRef::Layer(layer)).and_then(|item| item.raster_bounds))?;
    if bounds.is_empty() { return None; }
    let transform = current.as_ref().map_or(source.transform.as_ref(), |current|current.transform.as_ref()).cloned();
    if transform.as_ref().is_some_and(|transform| !(0..=3).contains(&transform.interpolation) || transform.matrix_f64_bits.iter().any(|bits| !f64::from_bits(*bits).is_finite())) { return None; }
    Some(GeometryHints { origin: [bounds.x0,bounds.y0], transform })
}
fn geometry_context(app: &PhotocraftApp, layer: LayerId) -> Result<(),String> {
    live_context(app,layer)?;
    let active = app.session.active().ok_or("xcf.link.ui.stale-target")?;
    photocraft_engine::xcf_item_locks::effective_layer_locks(&active.doc,layer)
        .and_then(|locks|locks.require(photocraft_engine::xcf_item_locks::EditKind::Position)).map_err(|error|error.to_string())
}
fn geometry_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, captured: &photocraft_engine::document_snapshot::DocumentSnapshot, revision: u64, layer: LayerId, owned: bool) {
    let Some(hints) = geometry_hints(captured,layer).filter(|_| owned) else { return; };
    let initial_interpolation = hints.transform.as_ref().map_or(0,|transform|transform.interpolation);
    let restored = app.ui.xcf_link_geometry_draft.take().filter(|draft| draft.document == captured.id.0 && draft.layer == layer.0 && draft.revision == revision);
    let mut draft = restored.unwrap_or(crate::state::XcfLinkedGeometryDraft { document: captured.id.0, layer: layer.0, revision, origin: hints.origin, interpolation: initial_interpolation });
    let enabled = geometry_context(app,layer).is_ok();
    ui.label(tl!("Position"));
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            let label = ui.label(tl!("X"));
            ui.add(egui::DragValue::new(&mut draft.origin[0]).range(-(1 << 29)..=(1 << 29))).labelled_by(label.id);
            let label = ui.label(tl!("Y"));
            ui.add(egui::DragValue::new(&mut draft.origin[1]).range(-(1 << 29)..=(1 << 29))).labelled_by(label.id);
        });
        if hints.transform.is_some() {
            egui::ComboBox::from_id_salt(("xcf_link_interpolation",layer.0)).selected_text(interpolation_label(draft.interpolation)).show_ui(ui, |ui| {
                for id in 0..=3 { ui.selectable_value(&mut draft.interpolation,id,interpolation_label(id)); }
            });
        }
    });
    let valid = draft.origin.iter().all(|value| (-(1 << 29)..=(1 << 29)).contains(value)) && (0..=3).contains(&draft.interpolation);
    let interpolation_changed = hints.transform.is_some() && draft.interpolation != initial_interpolation;
    let changed = draft.origin != hints.origin || interpolation_changed;
    let mut apply = false;
    let mut reset = false;
    ui.horizontal(|ui| {
        apply = ui.add_enabled(enabled && valid && changed,egui::Button::new(tl!("Apply"))).clicked();
        reset = ui.button(tl!("Reset")).clicked();
    });
    if reset { draft.origin = hints.origin; draft.interpolation = initial_interpolation; }
    let request = if apply && interpolation_changed {
        hints.transform.as_ref().map(|transform|("xcf.link.setTransform",serde_json::json!({"layerId":layer.0.to_string(),"matrixF64Bits":transform.matrix_f64_bits.map(|bits|format!("{bits:016x}")),"interpolation":interpolation_name(draft.interpolation),"originPx":draft.origin})))
    } else if apply { Some(("xcf.link.setPlacement",serde_json::json!({"layerId":layer.0.to_string(),"originPx":draft.origin}))) } else { None };
    app.ui.xcf_link_geometry_draft = Some(draft);
    if let Some((command,params)) = request {
        let result = geometry_context(app,layer).and_then(|()|run_frame(app,captured,revision,layer,command,params));
        show_error(app,result);
    }
}
fn interpolation_name(id: i32) -> &'static str { match id { 1=>"linear",2=>"cubic",3=>"nohalo",_=>"none" } }
fn interpolation_label(id: i32) -> &'static str { match id { 0=>tl!("Nearest Neighbor"),1=>tl!("Bilinear"),2=>tl!("Bicubic"),3=>tl!("NoHalo"),_=>tl!("Unsupported") } }
fn run_frame(app: &mut PhotocraftApp, captured: &photocraft_engine::document_snapshot::DocumentSnapshot, revision: u64, layer: LayerId, command: &str, params: serde_json::Value) -> Result<serde_json::Value,String> {
    validate_frame(app,captured,revision,layer)?;
    app.run(command,params)
}
fn show_error(app:&mut PhotocraftApp,result:Result<serde_json::Value,String>) {
    if let Err(error)=result { app.ui.status=error; app.ui.status_error=true; }
}

fn retained_resource(doc: &photocraft_doc::Document, layer: LayerId) -> Option<(photocraft_doc::item_identity::ResourceId, bool)> {
    let state = doc.xcf_state.as_ref()?;
    if layer.0 == 0 || state.items.len() > 10_000 || state.source_items.len() > 10_000 || state.resources.len() > 10_000 { return None; }
    let mut sources = state.source_items.iter().filter(|source| source.owner == ItemRef::Layer(layer));
    let source = sources.next()?;
    if sources.next().is_some() { return None; }
    let EditableSource::Linked { resource, .. } = source.source else { return None };
    let mut entries = state.resources.iter().filter(|entry| entry.id == resource);
    let entry = entries.next()?;
    if entries.next().is_some() || entry.kind != ResourceKind::LinkedImage { return None; }
    Some((resource, entry.content_hash.is_some()))
}
fn live_context(app: &PhotocraftApp, layer: LayerId) -> Result<(), String> {
    let active = app.session.active().ok_or("xcf.link.ui.stale-target")?;
    if app.ui.transform.is_some() || app.session.has_jobs() || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some()) {
        return Err("xcf.link.ui.context".into());
    }
    photocraft_engine::xcf_item_locks::effective_layer_locks(&active.doc, layer)
        .and_then(|locks| locks.require(photocraft_engine::xcf_item_locks::EditKind::Content))
        .map_err(|error| error.to_string())
}
fn validate_frame(app: &PhotocraftApp, captured: &photocraft_engine::document_snapshot::DocumentSnapshot, revision: u64, layer: LayerId) -> Result<(), String> {
    let active = app.session.active().ok_or("xcf.link.ui.stale-target")?;
    if active.doc.id != captured.id || active.revision != revision || !active.doc.shares_state(captured) { return Err("xcf.link.ui.stale-target".into()); }
    live_context(app, layer)
}

#[cfg(test)]
#[path = "linked_image_tests.rs"]
mod tests;
