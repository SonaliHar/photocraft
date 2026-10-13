//! Immediate insertion from an explicit scalar template; command-owned qualification.
use crate::PhotocraftApp;
use photocraft_doc::{DocId, LayerId, item_identity::FilterId};
use photocraft_engine::{
    document_snapshot::WeakDocumentSnapshot,
    xcf_filter_insert_cmds::{current_templates, template_candidate, template_wire, unused_filter_id},
};
use serde_json::{Value, json};

const STALE: &str = "xcf.filter.insert.ui.stale-target";
struct Target {
    document: DocId,
    revision: u64,
    owner: LayerId,
    stack_revision: u64,
    snapshot: WeakDocumentSnapshot,
}
fn context(app: &PhotocraftApp) -> bool {
    app.ui.transform.is_none()
        && !app.session.has_jobs()
        && app.session.active().is_some_and(|active| {
            active.floating.is_none()
                && active.doc.xcf_state.as_ref().is_none_or(|state| state.floating.is_none())
                && active.revision != u64::MAX
                && active.doc.accounted().is_none()
        })
}
fn apply(app: &mut PhotocraftApp, target: &Target, template_id: FilterId) -> Result<Value, String> {
    if !context(app) {
        return Err("xcf.filter.insert.ui.context".into());
    }
    let active = app.session.active().ok_or(STALE)?;
    let original = target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id != target.document || active.revision != target.revision || !original.shares_state(&active.doc) {
        return Err(STALE.into());
    }
    let stack = current_templates(&active.doc, target.owner).ok_or(STALE)?;
    if stack.revision != target.stack_revision {
        return Err(STALE.into());
    }
    let index = stack.nodes.iter().position(|node| node.id == template_id).ok_or(STALE)?;
    let template = stack.nodes.get(index).filter(|node| template_candidate(node)).ok_or(STALE)?;
    let insert_index = index.checked_add(1).filter(|index| *index <= stack.nodes.len() && *index < 1024).ok_or(STALE)?;
    let (operation, arguments) = template_wire(template)?;
    let new_id = unused_filter_id(&active.doc).ok_or(STALE)?;
    let params = json!({"layerId":target.owner.0.to_string(),"filterId":new_id.0.to_string(),"templateFilterId":template_id.0.to_string(),
        "index":insert_index,"operation":operation,"arguments":arguments,"enabled":template.enabled});
    // No await/focus change/direct edit between the guarded source and dispatch.
    app.run("xcf.filter.insert", params)
}

/// Existing localized Add control; choosing a caption inserts the complete
/// closed current configuration immediately after that explicit template.
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, owner: LayerId) {
    let Some(active) = app.session.active() else { return };
    let Some(stack) = current_templates(&active.doc, owner) else { return };
    let target = Target { document: active.doc.id, revision: active.revision, owner, stack_revision: stack.revision, snapshot: active.doc.downgrade() };
    let mut chosen = None;
    let editable = context(app);
    ui.push_id(("xcf_filter_insert", owner.0), |ui| {
        ui.add_enabled_ui(editable, |ui| {
            ui.menu_button(tl!("Add"), |ui| {
                for node in stack.nodes.iter().take(64).filter(|node| template_candidate(node)) {
                    let caption: String =
                        node.source_display_name.as_deref().filter(|text| !text.is_empty()).unwrap_or(&node.operation.name).chars().take(128).collect();
                    if ui.button(caption).clicked() {
                        chosen = Some(node.id);
                        ui.close();
                    }
                }
            });
        });
    });
    if let Some(template) = chosen
        && let Err(error) = apply(app, &target, template)
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_imported_stack_creates_no_insertion_controls() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut document = photocraft_doc::Document::new(
            "Own absent template",
            photocraft_doc::Size::new(1, 1),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
        );
        let layer = photocraft_doc::Layer::raster("Own ordinary layer", document.pixel_format());
        let owner = layer.id;
        document.layers.push(layer);
        let mut session = photocraft_engine::Session::new();
        session.open_document(document, None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(320., 200.)).build_ui_state(move |ui, app| properties(app, ui, owner), app);
        harness.run();
        assert!(harness.query_all_by_label("Add").next().is_none());
    }
}
