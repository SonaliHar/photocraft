//! Current imported layer controls use the registered metadata commands.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{LayerId, item_identity::ItemRef};
use serde_json::json;

fn context(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or_else(||tl!("Open a document before editing layer properties.").to_owned())?;
    if active.revision==u64::MAX || active.floating.is_some() || active.coalesce.is_some() || app.ui.transform.is_some() || app.session.has_jobs() {
        return Err(tl!("Finish pending edits before changing layer properties.").into());
    }
    Ok(())
}
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let Some(active) = app.session.active() else { return };
    let Some(imported) = active.doc.xcf_state.as_ref() else { return };
    if imported.items.len() > 10_000 { return; }
    let mut matches = imported.items.iter().filter(|item| item.item == ItemRef::Layer(layer));
    let Some(item) = matches.next() else { return };
    if matches.next().is_some() { return; }
    let identity = active.doc.id;
    let revision = active.revision;
    let snapshot = active.doc.downgrade();
    let locks = item.locks;
    let visible = item.visible;
    let can_change_visibility = enabled_visibility(&active.doc, layer);
    let permission=context(app);
    let enabled=permission.is_ok();
    let mut change = None;
    let mut visibility_change = None;
    ui.separator();
    let mut showing = visible;
    let visibility=ui.add_enabled(enabled && can_change_visibility, egui::Checkbox::new(&mut showing, tl!("Visible")));
    if let Err(error)=&permission {visibility.clone().on_disabled_hover_text(error);}
    if visibility.changed() {visibility_change=Some(showing);}
    ui.label(egui::RichText::new(tl!("Lock:")).color(Tokens::get(ui.ctx()).text));
    if let Err(error)=&permission {ui.label(egui::RichText::new(error).color(Tokens::get(ui.ctx()).text_dim));}
    ui.add_enabled_ui(enabled, |ui| {
        for (key, label, current) in [
            ("content", tl!("Lock image pixels"), locks.content),
            ("position", tl!("Lock position"), locks.position),
            ("alpha", tl!("Lock transparent pixels"), locks.alpha),
            ("visibility", tl!("Visibility"), locks.visibility),
        ] {
            let mut value = current;
            if ui.checkbox(&mut value, label).changed() { change = Some((key, value)); }
        }
    });
    if change.is_none() && visibility_change.is_none() { return; }
    let current = app.session.active();
    let valid = snapshot.upgrade().is_some_and(|captured| current.is_some_and(|state|
        state.doc.id == identity && state.revision == revision && captured.shares_state(&state.doc) && state.floating.is_none()));
    if !valid {app.ui.status=tl!("The layer changed. Select it again before editing.").into();app.ui.status_error=true;return;}
    if let Err(error)=context(app) {app.ui.status=error;app.ui.status_error=true;return;}
    let (command, params) = if let Some((key, value)) = change {
        let mut params = json!({"layerId":layer.0.to_string()});
        if let Some(object) = params.as_object_mut() { object.insert(key.into(), json!(value)); }
        ("xcf.item.locks.set", params)
    } else if let Some(visible) = visibility_change {
        ("xcf.items.visibility.set", json!({"items":[{"kind":"layer","id":layer.0.to_string()}],"visible":visible}))
    } else { return };
    if let Err(error) = app.run(command, params) { app.ui.status = error; app.ui.status_error = true; }
}

fn enabled_visibility(document: &photocraft_doc::Document, layer: LayerId) -> bool {
    photocraft_engine::xcf_item_locks::require_visibility_changes(document, &[layer], &photocraft_raster::Interrupt::NONE).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::{NodeT,Queryable}};
    fn app() -> (PhotocraftApp, LayerId) {
        let mut document = photocraft_doc::Document::new("Current controls", photocraft_doc::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        let layer = photocraft_doc::Layer::raster("Current layer", document.pixel_format());
        let identity = layer.id;
        document.layers.push(layer);
        let mut imported = photocraft_doc::XcfState::default();
        imported.reconcile_layers(&document).unwrap();
        imported.items[0].source_flags = 0x1234;
        document.xcf_state = Some(imported);
        let mut session = photocraft_engine::Session::new();
        session.open_document(document, None).unwrap();
        (PhotocraftApp::new(session, crate::Services::default()), identity)
    }
    #[test]
    fn checkbox_uses_one_registered_lock_transaction_with_undo() {
        let (app, layer) = app();
        let before = app.session.active().unwrap().doc.clone();
        let count = app.session.active().unwrap().history.entry_labels().count();
        let mut harness = Harness::builder().with_size(egui::vec2(440., 260.)).build_ui_state(move |ui, app| properties(app, ui, layer), app);
        harness.run();
        harness.get_by_label("Lock image pixels").click();
        harness.run();
        let current = harness.state().session.active().unwrap();
        assert!(current.doc.layer(layer).unwrap().locks.pixels);
        assert!(current.doc.xcf_state.as_ref().unwrap().items[0].locks.content);
        assert_eq!(current.doc.xcf_state.as_ref().unwrap().items[0].source_flags, 0x1234);
        assert_eq!(current.history.entry_labels().count(), count + 1);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(harness.state().session.active().unwrap().doc, before);
    }
    #[test]
    fn inherited_visibility_lock_disables_control_without_mutation() {
        let (mut app, layer) = app();
        app.session.edit("Visibility lock", |document, _| {
            document.xcf_state.as_mut().unwrap().items[0].locks.visibility = true;
            Ok(())
        }).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        assert!(!enabled_visibility(&before, layer));
        let mut harness = Harness::builder().with_size(egui::vec2(440., 260.)).build_ui_state(move |ui, app| properties(app, ui, layer), app);
        harness.run();
        assert_eq!(harness.state().session.active().unwrap().doc, before);
    }
    #[test]
    fn retained_coalesce_disables_current_layer_controls_without_changing_source_or_history() {
        let (mut app,layer)=app();
        let before=app.session.active().unwrap().doc.clone();
        let history=app.session.active().unwrap().history.entry_labels().count();
        app.session.active_mut().unwrap().coalesce=Some("Own in-progress property gesture".into());
        assert!(context(&app).is_err());
        let mut harness=Harness::builder().with_size(egui::vec2(440.,300.)).build_ui_state(move|ui,app|properties(app,ui,layer),app);
        harness.run();
        assert!(harness.get_by_label("Lock image pixels").accesskit_node().is_disabled());
        let current=harness.state().session.active().unwrap();
        assert!(current.doc.shares_state(&before));
        assert_eq!(current.history.entry_labels().count(),history);
        assert!(current.coalesce.is_some());
    }

}
