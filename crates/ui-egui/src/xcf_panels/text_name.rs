//! Explicit current text naming metadata; source text and cache stay separate.
use crate::{PhotocraftApp, state::XcfTextNameDraft, theme::Tokens};
use photocraft_doc::{DocId, LayerId, item_identity::ItemRef, xcf_items::EditableSource};
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Value, json};

const COMMAND: &str = "xcf.text.name.set";
const LIMIT: &str = "xcf.text.name.ui.limit";
const STALE: &str = "xcf.text.name.stale-target";
struct Target {
    document: DocId, revision: u64, layer: LayerId,
    name: String, custom: bool, snapshot: WeakDocumentSnapshot,
}
fn bounded_name(name: &str) -> bool { !name.is_empty() && name.len() <= 4096 && !name.contains('\0') }
fn owned(name: &str) -> Result<String, String> {
    if !bounded_name(name) { return Err(LIMIT.into()); }
    let mut result = String::new();
    result.try_reserve_exact(name.len()).map_err(|_| LIMIT)?;
    result.push_str(name);
    Ok(result)
}
fn capture(app: &PhotocraftApp, layer: LayerId) -> Result<Option<Target>, String> {
    let Some(active) = app.session.active() else { return Ok(None) };
    let Some(state) = active.doc.xcf_state.as_ref() else { return Ok(None) };
    if layer.0 == 0 || active.doc.id.0 == 0 || state.source_items.len() > 10_000 || state.items.len() > 10_000 { return Ok(None); }
    let mut sources = state.source_items.iter().filter(|source| source.owner == ItemRef::Layer(layer));
    let Some(source) = sources.next() else { return Ok(None) };
    if sources.next().is_some() || !matches!(source.source, EditableSource::Text { .. }) { return Ok(None); }
    let mut items = state.items.iter().filter(|item| item.item == ItemRef::Layer(layer));
    let Some(item) = items.next() else { return Ok(None) };
    if items.next().is_some() || !bounded_name(&item.name) { return Ok(None); }
    Ok(Some(Target { document: active.doc.id, revision: active.revision, layer, name: owned(&item.name)?, custom: item.custom_name, snapshot: active.doc.downgrade() }))
}
fn enabled(app: &PhotocraftApp, target: &Target) -> bool {
    app.ui.transform.is_none() && !app.session.has_jobs() && app.session.active().is_some_and(|active|
        active.floating.is_none() && active.doc.xcf_state.as_ref().is_none_or(|state| state.floating.is_none()) && active.revision != u64::MAX
        && photocraft_engine::xcf_item_locks::effective_layer_locks(&active.doc, target.layer)
            .is_ok_and(|locks| locks.require(photocraft_engine::xcf_item_locks::EditKind::Content).is_ok()))
}
fn draft(target: &Target) -> Result<XcfTextNameDraft, String> {
    Ok(XcfTextNameDraft { document: target.document.0, layer: target.layer.0, revision: target.revision, name: owned(&target.name)?, custom_name: target.custom })
}
fn request(app: &PhotocraftApp, target: &Target, value: &XcfTextNameDraft) -> Result<Value, String> {
    if !bounded_name(&value.name) || value.document != target.document.0 || value.layer != target.layer.0 || value.revision != target.revision { return Err(STALE.into()); }
    if !enabled(app, target) { return Err("xcf.text.name.ui.context".into()); }
    let active = app.session.active().ok_or(STALE)?;
    let original = target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id != target.document || active.revision != target.revision || !original.shares_state(&active.doc) { return Err(STALE.into()); }
    let fresh = capture(app, target.layer)?.ok_or(STALE)?;
    if fresh.name != target.name || fresh.custom != target.custom { return Err(STALE.into()); }
    Ok(json!({"documentId":target.document.0.to_string(),"expectedRevision":target.revision.to_string(),"layerId":target.layer.0.to_string(),"name":owned(&value.name)?,"customName":value.custom_name}))
}

pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId) {
    let target = match capture(app, layer) { Ok(Some(target)) => target, Ok(None) => return, Err(error) => { app.ui.status = error; app.ui.status_error = true; return } };
    let editable = enabled(app, &target);
    let restored = app.ui.xcf_text_name_draft.take();
    let mut value = match restored {
        Some(value) if value.document == target.document.0 && value.layer == layer.0 && value.revision == target.revision && value.name.len() <= 4096 && !value.name.contains('\0') => value,
        _ => match draft(&target) { Ok(value) => value, Err(error) => { app.ui.status = error; app.ui.status_error = true; return } },
    };
    ui.push_id(("xcf_text_name", target.document.0, layer.0), |ui| {
        let label = ui.label(egui::RichText::new(tl!("Name")).color(Tokens::get(ui.ctx()).text_dim));
        ui.add_enabled(editable, egui::TextEdit::singleline(&mut value.name).char_limit(4096).desired_width(f32::INFINITY)).labelled_by(label.id);
        ui.add_enabled(editable, egui::Checkbox::new(&mut value.custom_name, tl!("Custom")));
    });
    // Character-limited widget input is at most 16KiB UTF8. Retain only a valid
    // byte-bounded prefix; restored oversized strings are discarded before cloning.
    if value.name.len() > 4096 {
        let mut end = 4096;
        while !value.name.is_char_boundary(end) { end = end.saturating_sub(1); }
        value.name.truncate(end);
    }
    let changed = value.name != target.name || value.custom_name != target.custom;
    let mut apply = false;
    let mut reset = false;
    ui.horizontal(|ui| {
        apply = ui.add_enabled(editable && changed && bounded_name(&value.name), egui::Button::new(tl!("Apply"))).clicked();
        reset = ui.add_enabled(changed, egui::Button::new(tl!("Reset"))).clicked();
    });
    if reset { value = match draft(&target) { Ok(value) => value, Err(error) => { app.ui.status = error; app.ui.status_error = true; return } }; }
    if apply {
        match request(app, &target, &value).and_then(|params| app.run(COMMAND, params)) {
            Ok(_) => { app.ui.xcf_text_name_draft = None; return; }
            Err(error) => { app.ui.status = error; app.ui.status_error = true; }
        }
    }
    app.ui.xcf_text_name_draft = Some(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use photocraft_doc::{Document, Layer, PixelFormat, XcfState, item_identity::ResourceId, xcf_items::SourceItem, xcf_resources::{ResourceKind, ResourceReference}, xcf_state::ItemState};
    fn fixture(name: &str) -> (PhotocraftApp, LayerId) {
        let mut layer = Layer::raster(name, PixelFormat::RGBA8);
        layer.surface_mut().unwrap().try_write_interleaved_with(photocraft_doc::Rect::new(0, 0, 1, 1), &[11, 22, 33, 255], &photocraft_raster::Interrupt::NONE).unwrap();
        let id = layer.id;
        let mut document = Document::new("Own text naming metadata", photocraft_doc::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        document.layers = vec![layer];
        document.xcf_state = Some(XcfState {
            items: vec![ItemState { item: ItemRef::Layer(id), name: name.into(), custom_name: false, visible: true, locks: Default::default(), parent: None, source_tattoo: Some(42), source_flags: 0x1234, raster_bounds: None, mask: None, composition: None }],
            resources: vec![ResourceReference { id: ResourceId(1), kind: ResourceKind::Font, name: "Own unresolved font".into(), collection: None, internal: false, locator: Some("own:never-read".into()), content_hash: None, owned_bytes: None, resolution: None }],
            source_items: vec![SourceItem { owner: ItemRef::Layer(id), source: EditableSource::Text { content: "Original source I".into(), markup: None, font: ResourceId(1), direction: 0, source_modified: false, configuration: Vec::new(), current_origin: None }, transform: None, cache: None }],
            ..Default::default()
        });
        document.validate_xcf_state().unwrap();
        let mut session = photocraft_engine::Session::new();
        session.open_document(document, None).unwrap();
        (PhotocraftApp::new(session, crate::Services::default()), id)
    }
    #[test]
    fn exact_bounded_payload_retains_full_utf8_name_and_rejects_stale_wire_fields() {
        let name = "Own 文🙂".repeat(24);
        let (app, layer) = fixture(&name);
        let target = capture(&app, layer).unwrap().unwrap();
        let mut value = draft(&target).unwrap();
        value.custom_name = true;
        let params = request(&app, &target, &value).unwrap();
        assert_eq!(params.as_object().unwrap().len(), 5);
        assert_eq!(params["name"], name);
        assert_eq!(params["layerId"], layer.0.to_string());
        assert_eq!(params["documentId"], target.document.0.to_string());
        assert_eq!(params["expectedRevision"], target.revision.to_string());
        assert_eq!(params["customName"], true);
        value.name = "文".repeat(1366);
        assert!(request(&app, &target, &value).is_err());
        value.name = "bad\0name".into();
        assert!(request(&app, &target, &value).is_err());
        value.name = "".into();
        assert!(request(&app, &target, &value).is_err());
        value = draft(&target).unwrap();
        value.revision = value.revision.checked_add(1).unwrap();
        assert!(request(&app, &target, &value).is_err());
    }
    #[test]
    fn editor_and_reset_are_view_only_apply_is_one_native_metadata_undo_redo() {
        let (app, layer) = fixture("Own literal text name");
        let before = app.session.active().unwrap().doc.clone();
        let before_bytes = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let mut harness = Harness::builder().with_size(egui::vec2(500., 240.)).build_ui_state(move |ui, app| properties(app, ui, layer), app);
        harness.run_steps(2);
        harness.get_by_label("Name").click();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Name").type_text("Own changed 文🙂 name");
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        assert!(harness.state().session.active().unwrap().doc.shares_state(&before));
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
        harness.get_by_label("Reset").click();
        harness.run_steps(2);
        assert_eq!(harness.state().ui.xcf_text_name_draft.as_ref().unwrap().name, "Own literal text name");
        harness.get_by_label("Name").click();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Name").type_text("Own changed 文🙂 name");
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        harness.get_by_label("Custom").click();
        harness.run_steps(2);
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
        harness.get_by_label("Apply").click();
        harness.run_steps(2);
        let after = harness.state().session.active().unwrap().doc.clone();
        let mut expected = (*before).clone();
        expected.layer_mut(layer).unwrap().name = "Own changed 文🙂 name".into();
        let item = expected.xcf_state.as_mut().unwrap().items.first_mut().unwrap();
        item.name = "Own changed 文🙂 name".into();
        item.custom_name = true;
        assert_eq!(*after, expected);
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
        let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *after);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(&harness.state().session.active().unwrap().doc, &Default::default()).unwrap(), before_bytes);
        harness.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(harness.state().session.active().unwrap().doc, after);
    }
    #[test]
    fn old_target_after_edit_undo_preserves_real_redo_and_native_source() {
        let (mut app, layer) = fixture("Own current name");
        let target = capture(&app, layer).unwrap().unwrap();
        let mut value = draft(&target).unwrap();
        value.custom_name = true;
        let params = request(&app, &target, &value).unwrap();
        app.run(COMMAND, params).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        assert!(request(&app, &target, &value).is_err());
        let active = app.session.active().unwrap();
        assert_eq!(active.revision, revision);
        assert!(active.doc.shares_state(&before));
        assert!(active.history.can_redo());
        assert_eq!(photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap(), native);
    }
    #[test]
    fn missing_duplicate_and_hostile_name_metadata_render_no_editor() {
        for case in 0..6 {
            let (mut app, layer) = fixture("Own name");
            let document = app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();
            match case {
                0 => document.xcf_state = None,
                1 => document.xcf_state.as_mut().unwrap().source_items.clear(),
                2 => { let duplicate = document.xcf_state.as_ref().unwrap().source_items[0].clone(); document.xcf_state.as_mut().unwrap().source_items.push(duplicate); }
                3 => { let duplicate = document.xcf_state.as_ref().unwrap().items[0].clone(); document.xcf_state.as_mut().unwrap().items.push(duplicate); }
                4 => document.xcf_state.as_mut().unwrap().items[0].name = "x".repeat(4097),
                _ => document.xcf_state.as_mut().unwrap().items[0].name = "bad\0name".into(),
            }
            assert!(capture(&app, layer).unwrap().is_none());
            let mut harness = Harness::builder().with_size(egui::vec2(400., 240.)).build_ui_state(move |ui, app| properties(app, ui, layer), app);
            harness.run();
            assert!(harness.query_all_by_label("Custom").next().is_none());
            assert!(harness.query_all_by_label("Apply").next().is_none());
            assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
        }
    }
    #[test]
    fn restored_oversized_or_other_target_draft_is_replaced_without_dispatch() {
        let (mut app, layer) = fixture("Own safe current name");
        let target = capture(&app, layer).unwrap().unwrap();
        let mut value = draft(&target).unwrap();
        value.name = "x".repeat(4097);
        value.custom_name = true;
        app.ui.xcf_text_name_draft = Some(value);
        let before = app.session.active().unwrap().doc.clone();
        let mut harness = Harness::builder().with_size(egui::vec2(400., 240.)).build_ui_state(move |ui, app| properties(app, ui, layer), app);
        harness.run();
        let current = harness.state().ui.xcf_text_name_draft.as_ref().unwrap();
        assert_eq!(current.name, "Own safe current name");
        assert!(!current.custom_name);
        assert!(harness.state().session.active().unwrap().doc.shares_state(&before));
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
        let restored = harness.state_mut().ui.xcf_text_name_draft.as_mut().unwrap();
        restored.document = restored.document.checked_add(1).unwrap();
        restored.name = "Other document draft".into();
        harness.run();
        assert_eq!(harness.state().ui.xcf_text_name_draft.as_ref().unwrap().name, "Own safe current name");
        assert!(harness.state().session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    #[ignore = "retained qualified text cache/native metadata continuation draft; not executed"]
    fn qualified_text_name_metadata_keeps_nonempty_cache_resources_and_original_fields() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("retained qualified text corpus"));
        let document = photocraft_io::import("source.xcf", &std::fs::read(root.join("text-stem-evidence/simple/source.xcf")).unwrap()).unwrap().document;
        let source = document.xcf_state.as_ref().unwrap().source_items.iter().find(|source| matches!(source.source, EditableSource::Text { .. })).unwrap();
        assert!(!source.cache.as_ref().unwrap().bytes.as_slice().is_empty());
        let ItemRef::Layer(layer) = source.owner else { panic!("qualified text layer") };
        let mut session = photocraft_engine::Session::new();
        session.open_document(document, None).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        let before = app.session.active().unwrap().doc.clone();
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let target = capture(&app, layer).unwrap().unwrap();
        let mut value = draft(&target).unwrap();
        value.name = "Own explicit current text name".into();
        value.custom_name = !target.custom;
        let params = request(&app, &target, &value).unwrap();
        app.run(COMMAND, params).unwrap();
        let after = app.session.active().unwrap().doc.clone();
        assert_eq!(after.xcf_state.as_ref().unwrap().source_items, before.xcf_state.as_ref().unwrap().source_items);
        assert_eq!(after.xcf_state.as_ref().unwrap().resources, before.xcf_state.as_ref().unwrap().resources);
        assert_eq!(after.xcf_state.as_ref().unwrap().provenance, before.xcf_state.as_ref().unwrap().provenance);
        assert_eq!(after.layer(layer).unwrap().surface(), before.layer(layer).unwrap().surface());
        assert_eq!(app.session.active().unwrap().history.past_len(), 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(&app.session.active().unwrap().doc, &Default::default()).unwrap(), native);
    }
}
