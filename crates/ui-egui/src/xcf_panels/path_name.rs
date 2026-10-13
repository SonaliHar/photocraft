//! Current saved-path naming by canonical ID, never a name-to-identity guess.
use crate::{PhotocraftApp, state::XcfPathNameDraft, theme::Tokens};
use photocraft_doc::{DocId, item_identity::{ItemRef, PathId}};
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Value, json};

const COMMAND: &str = "xcf.path.name.set";
const LIMIT: &str = "xcf.path.name.ui.limit";
const STALE: &str = "xcf.path.name.ui.stale-target";
struct Target { document: DocId, revision: u64, path: PathId, name: String, custom: bool, snapshot: WeakDocumentSnapshot }
fn bounded(name: &str) -> bool { !name.is_empty() && name.len() <= 4096 && !name.contains('\0') }
fn editable_name(name: &str) -> bool { bounded(name) && !name.eq_ignore_ascii_case("work") && name != "Work Path" }
fn owned(name: &str) -> Result<String, String> {
    if !bounded(name) { return Err(LIMIT.into()); }
    let mut text = String::new(); text.try_reserve_exact(name.len()).map_err(|_| LIMIT)?; text.push_str(name); Ok(text)
}
fn capture(app: &PhotocraftApp, path: PathId) -> Result<Option<Target>, String> {
    let Some(active) = app.session.active() else { return Ok(None) };
    let Some(selection) = &app.ui.selected_saved_path else { return Ok(None) };
    if path.0 == 0 || selection.document != active.doc.id.0 || selection.path != path.0 { return Ok(None); }
    let Some(state) = &active.doc.xcf_state else { return Ok(None) };
    if active.doc.paths.len() > 10_000 || state.items.len() > 10_000 { return Ok(None); }
    let mut paths = active.doc.paths.iter().filter(|named| named.id == path);
    let Some(named) = paths.next() else { return Ok(None) };
    if paths.next().is_some() || !bounded(&named.name) { return Ok(None); }
    let mut items = state.items.iter().filter(|item| item.item == ItemRef::Path(path));
    let Some(item) = items.next() else { return Ok(None) };
    if items.next().is_some() || item.name != named.name { return Ok(None); }
    // Legacy view-state consistency is only a guard. It never resolves a PathId.
    if app.ui.selected_path.as_deref() != Some(named.name.as_str()) { return Ok(None); }
    Ok(Some(Target { document: active.doc.id, revision: active.revision, path, name: owned(&named.name)?, custom: item.custom_name, snapshot: active.doc.downgrade() }))
}
fn enabled(app: &PhotocraftApp, target: &Target) -> bool {
    app.ui.transform.is_none() && !app.session.has_jobs() && app.session.active().is_some_and(|active|
        active.floating.is_none() && active.doc.accounted().is_none() && active.revision != u64::MAX
        && active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_none()
            && state.effective_locks(ItemRef::Path(target.path)).is_ok_and(|locks| !locks.content)))
}
fn draft(target: &Target) -> Result<XcfPathNameDraft, String> {
    Ok(XcfPathNameDraft { document: target.document.0, path: target.path.0, revision: target.revision, name: owned(&target.name)?, custom_name: target.custom })
}
fn request(app: &PhotocraftApp, target: &Target, value: &XcfPathNameDraft) -> Result<Value, String> {
    if !editable_name(&value.name) || value.document != target.document.0 || value.path != target.path.0 || value.revision != target.revision || !enabled(app, target) { return Err(STALE.into()); }
    let active = app.session.active().ok_or(STALE)?;
    let original = target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id != target.document || active.revision != target.revision || !original.shares_state(&active.doc) { return Err(STALE.into()); }
    let fresh = capture(app, target.path)?.ok_or(STALE)?;
    if fresh.name != target.name || fresh.custom != target.custom { return Err(STALE.into()); }
    Ok(json!({"documentId":target.document.0.to_string(),"expectedRevision":target.revision.to_string(),"pathId":target.path.0.to_string(),"name":owned(&value.name)?,"customName":value.custom_name}))
}
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, path: PathId) {
    let target = match capture(app, path) { Ok(Some(target)) => target, Ok(None) => return, Err(error) => { app.ui.status = error; app.ui.status_error = true; return } };
    let editable = enabled(app, &target);
    let restored = app.ui.xcf_path_name_draft.take();
    let mut value = match restored {
        Some(value) if value.document == target.document.0 && value.path == path.0 && value.revision == target.revision && value.name.len() <= 4096 && !value.name.contains('\0') => value,
        _ => match draft(&target) { Ok(value) => value, Err(error) => { app.ui.status = error; app.ui.status_error = true; return } },
    };
    ui.push_id(("xcf_saved_path_name", target.document.0, path.0), |ui| {
        let label = ui.label(egui::RichText::new(tl!("Name")).color(Tokens::get(ui.ctx()).text_dim));
        ui.add_enabled(editable, egui::TextEdit::singleline(&mut value.name).char_limit(4096).desired_width(f32::INFINITY)).labelled_by(label.id);
        ui.add_enabled(editable, egui::Checkbox::new(&mut value.custom_name, tl!("Custom")));
    });
    if value.name.len() > 4096 {
        let mut end = 4096;
        while !value.name.is_char_boundary(end) { end = end.saturating_sub(1); }
        value.name.truncate(end);
    }
    let changed = value.name != target.name || value.custom_name != target.custom;
    let mut apply = false;
    let mut reset = false;
    ui.horizontal(|ui| {
        apply = ui.add_enabled(editable && changed && editable_name(&value.name), egui::Button::new(tl!("Apply"))).clicked();
        reset = ui.add_enabled(changed, egui::Button::new(tl!("Reset"))).clicked();
    });
    if reset { value = match draft(&target) { Ok(value) => value, Err(error) => { app.ui.status = error; app.ui.status_error = true; return } }; }
    if apply {
        // Allocate legacy selection synchronization before committing metadata.
        let prepared = owned(&value.name).and_then(|name| request(app, &target, &value).map(|params| (name, params)));
        match prepared.and_then(|(name, params)| app.run(COMMAND, params).map(|_| name)) {
            Ok(name) => {
                if app.ui.selected_saved_path.as_ref().is_some_and(|selected| selected.document == target.document.0 && selected.path == path.0) { app.ui.selected_path = Some(name); }
                app.ui.xcf_path_name_draft = None; return;
            }
            Err(error) => { app.ui.status = error; app.ui.status_error = true; }
        }
    }
    app.ui.xcf_path_name_draft = Some(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use photocraft_doc::{Document, NamedPath, Path, Subpath, XcfState, ClippingPath, xcf_state::ItemState};
    fn fixture() -> (PhotocraftApp, PathId) {
        let mut geometry = Path::default();
        geometry.subpaths.push(Subpath::polygon(&[(1., 1.), (6., 1.), (3., 6.)]));
        let mut first = NamedPath::new("Own saved path", geometry.clone());
        first.psd_raw = Some(std::sync::Arc::new(vec![1, 2, 3, 4]));
        let second = NamedPath::new("Own other path", geometry);
        let id = first.id;
        let item = |path: &NamedPath, tattoo| ItemState { item: ItemRef::Path(path.id), name: path.name.clone(), custom_name: false, visible: true, locks: Default::default(), parent: None, source_tattoo: Some(tattoo), source_flags: 0x1234, raster_bounds: None, mask: None, composition: None };
        let mut document = Document::new("Own path name metadata", photocraft_doc::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        document.xcf_state = Some(XcfState { items: vec![item(&first, 41), item(&second, 42)], ..Default::default() });
        document.clipping_path = Some(ClippingPath { name: first.name.clone(), flatness: 0.75 });
        document.paths = vec![first, second];
        document.validate_xcf_state().unwrap();
        let mut session = photocraft_engine::Session::new();
        session.open_document(document, None).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.selected_path = Some("Own saved path".into());
        app.ui.selected_saved_path = Some(crate::state::XcfSavedPathSelection { document: app.session.active().unwrap().doc.id.0, path: id.0 });
        (app, id)
    }
    #[test]
    fn strict_typed_payload_bounds_aliases_and_legacy_name_are_guards_only() {
        let (mut app, path) = fixture();
        let target = capture(&app, path).unwrap().unwrap();
        let mut value = draft(&target).unwrap();
        value.name = "Own explicit 文🙂 path".into();
        value.custom_name = true;
        let params = request(&app, &target, &value).unwrap();
        assert_eq!(params.as_object().unwrap().len(), 5);
        assert_eq!(params["pathId"], path.0.to_string());
        assert_eq!(params["name"], value.name);
        assert_eq!(params["expectedRevision"], target.revision.to_string());
        for name in ["work".into(), "wOrK".into(), "Work Path".into(), "bad\0name".into(), "x".repeat(4097)] {
            value.name = name;
            assert!(request(&app, &target, &value).is_err());
        }
        app.ui.selected_path = Some("Own other path".into());
        assert!(capture(&app, path).unwrap().is_none());
        app.ui.selected_path = Some("Own saved path".into());
        app.ui.selected_saved_path = None;
        assert!(capture(&app, path).unwrap().is_none(), "name alone never supplies canonical identity");
    }
    #[test]
    fn widgets_stage_reset_then_one_registered_apply_preserving_geometry_clip_and_native_undo() {
        let (app, path) = fixture();
        let before = app.session.active().unwrap().doc.clone();
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let mut harness = Harness::builder().with_size(egui::vec2(500., 260.)).build_ui_state(move |ui, app| properties(app, ui, path), app);
        harness.run_steps(2);
        harness.get_by_label("Name").click();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Name").type_text("Own explicit 文🙂 path");
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        assert!(harness.state().session.active().unwrap().doc.shares_state(&before));
        harness.get_by_label("Reset").click();
        harness.run_steps(2);
        assert_eq!(harness.state().ui.xcf_path_name_draft.as_ref().unwrap().name, "Own saved path");
        harness.get_by_label("Name").click();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Name").type_text("Own explicit 文🙂 path");
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        harness.get_by_label("Custom").click();
        harness.run_steps(2);
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
        harness.get_by_label("Apply").click();
        harness.run_steps(2);
        let after = harness.state().session.active().unwrap().doc.clone();
        let mut expected = (*before).clone();
        expected.paths.iter_mut().find(|entry| entry.id == path).unwrap().name = "Own explicit 文🙂 path".into();
        let item = expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Path(path)).unwrap();
        item.name = "Own explicit 文🙂 path".into(); item.custom_name = true;
        expected.clipping_path.as_mut().unwrap().name = "Own explicit 文🙂 path".into();
        assert_eq!(*after, expected);
        assert_eq!(harness.state().ui.selected_path.as_deref(), Some("Own explicit 文🙂 path"));
        assert_eq!(harness.state().ui.selected_saved_path.as_ref().unwrap().path, path.0);
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
        let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *after);
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(photocraft_format::save_to_bytes(&harness.state().session.active().unwrap().doc, &Default::default()).unwrap(), native);
        harness.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(harness.state().session.active().unwrap().doc, after);
    }
    #[test]
    fn stale_edit_undo_target_does_not_clear_real_redo_or_retarget_same_named_path() {
        let (mut app, path) = fixture();
        let target = capture(&app, path).unwrap().unwrap();
        let mut value = draft(&target).unwrap(); value.custom_name = true;
        let params = request(&app, &target, &value).unwrap();
        app.run(COMMAND, params).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        assert!(request(&app, &target, &value).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert!(app.session.active().unwrap().history.can_redo());
        app.ui.selected_saved_path.as_mut().unwrap().path = path.0.checked_add(1).unwrap();
        assert!(capture(&app, path).unwrap().is_none());
    }
    #[test]
    fn actual_saved_row_click_carries_id_and_work_row_clears_typed_selection() {
        let (mut app, path) = fixture();
        app.ui.selected_saved_path = None;
        app.ui.selected_path = None;
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().work_path = Some(Path::default());
        let mut harness = Harness::builder().with_size(egui::vec2(420., 650.)).build_ui_state(|ui, app| crate::vector_ui::paths_panel(app, ui), app);
        harness.run_steps(2);
        harness.get_by_label("Own saved path").click();
        harness.run_steps(2);
        assert_eq!(harness.state().ui.selected_saved_path.as_ref().unwrap().path, path.0);
        assert!(harness.query_all_by_label("Custom").next().is_some());
        harness.get_by_label("Work Path").click();
        harness.run_steps(2);
        assert!(harness.state().ui.selected_saved_path.is_none());
        assert!(harness.query_all_by_label("Custom").next().is_none());
    }
    #[test]
    fn missing_duplicate_or_hostile_current_metadata_never_exposes_editor() {
        for case in 0..5 {
            let (mut app, path) = fixture();
            let document = app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();
            match case {
                0 => document.xcf_state = None,
                1 => document.xcf_state.as_mut().unwrap().items.clear(),
                2 => { let duplicate = document.paths[0].clone(); document.paths.push(duplicate); }
                3 => { let duplicate = document.xcf_state.as_ref().unwrap().items[0].clone(); document.xcf_state.as_mut().unwrap().items.push(duplicate); }
                _ => document.paths[0].name = "x".repeat(4097),
            }
            assert!(capture(&app, path).unwrap().is_none());
            let mut harness = Harness::builder().with_size(egui::vec2(400., 260.)).build_ui_state(move |ui, app| properties(app, ui, path), app);
            harness.run();
            assert!(harness.query_all_by_label("Apply").next().is_none());
            assert!(harness.query_all_by_label("Custom").next().is_none());
            assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
        }
    }
}
