//! Actual text events close one ordinary editing group before structural metadata editing.
use super::*;
use egui_kittest::Harness;
use photocraft_doc::LayerContent;
use serde_json::json;
fn editing() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("type.create", json!({"text": "Original", "x": 10, "y": 40})).unwrap();
    edit_active(&mut app).unwrap();
    let mut h = Harness::builder().with_max_steps(64).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            handle_keys(app, ui.ctx());
        },
        app,
    );
    h.run_steps(2);
    h
}
fn typed(h: &mut Harness<'_, PhotocraftApp>) {
    h.event(egui::Event::Text("Committed".into()));
    h.run_steps(2);
    let state = h.state().session.active().unwrap();
    let LayerContent::Text(text) = &state.doc.layer(state.active_layer.unwrap()).unwrap().content else { panic!("type") };
    assert_eq!(text.text, "Committed");
    assert_eq!(state.coalesce.as_deref(), Some(h.state().ui.text_edit.as_ref().unwrap().session.as_str()));
}
#[test]
fn real_text_commit_then_mask_preserves_group_history_and_undo_redo() {
    let mut h = editing();
    let original = h.state().session.active().unwrap().doc.clone();
    let initial = h.state().session.active().unwrap().history.past_len();
    typed(&mut h);
    let state = h.state().session.active().unwrap();
    let edited = state.doc.clone();
    let revision = state.revision;
    let past = state.history.past_len();
    assert_eq!(past, initial + 1);
    commit(h.state_mut());
    let state = h.state().session.active().unwrap();
    assert!(h.state().ui.text_edit.is_none() && state.coalesce.is_none());
    assert!(state.doc.shares_state(&edited));
    assert_eq!((state.revision, state.history.past_len()), (revision, past));
    h.state_mut().run("layer.layerMask.revealAll", json!({})).unwrap();
    let masked = h.state().session.active().unwrap().doc.clone();
    let layer = masked.layer(h.state().session.active().unwrap().active_layer.unwrap()).unwrap();
    assert!(matches!(layer.content, LayerContent::Text(_)) && layer.mask.is_some());
    assert_eq!(h.state().session.active().unwrap().history.past_len(), past + 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(&*h.state().session.active().unwrap().doc, &*edited);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(&*h.state().session.active().unwrap().doc, &*original);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(&*h.state().session.active().unwrap().doc, &*masked);
}
#[test]
fn cancel_uses_existing_undo_and_restored_or_other_document_editor_cannot_finish_a_group() {
    let mut h = editing();
    let original = h.state().session.active().unwrap().doc.clone();
    typed(&mut h);
    cancel(h.state_mut());
    let state = h.state().session.active().unwrap();
    assert!(h.state().ui.text_edit.is_none() && state.coalesce.is_none());
    assert_eq!(&*state.doc, &*original);
    assert!(state.history.can_redo());
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert!(h.state().session.active().unwrap().doc.layers.iter().any(|l| matches!(&l.content, LayerContent::Text(t) if t.text == "Committed")));
    for restored in [false, true] {
        let mut h = editing();
        typed(&mut h);
        let editor = h.state().ui.text_edit.clone().unwrap();
        if restored {
            let json = serde_json::to_value(&editor).unwrap();
            assert!(json.get("document").is_none(), "runtime origin is never persisted");
            let editor: crate::state::TextEdit = serde_json::from_value(json).unwrap();
            assert!(editor.document.is_none());
            h.state_mut().ui.text_edit = Some(editor);
        } else {
            let new = photocraft_doc::Document::with_background(
                "Other",
                photocraft_geom::Size::new(2, 2),
                photocraft_color::ColorMode::Rgb,
                photocraft_color::SampleType::U8,
                photocraft_color::Color::WHITE,
            );
            h.state_mut().session.add_document(new, None).unwrap();
        }
        let origin = h.state().session.documents().iter().find(|state| Some(state.doc.id) == editor.document).unwrap().try_clone_resident().unwrap();
        let before = h.state().session.active().unwrap().try_clone_resident().unwrap();
        commit(h.state_mut());
        assert!(h.state().ui.status_error);
        assert_eq!(h.state().ui.status, "type.ui.stale-editor");
        let state = h.state().session.active().unwrap();
        assert!(state.doc.shares_state(&before.doc));
        assert_eq!(
            (state.revision, state.history.past_len(), state.coalesce.as_deref()),
            (before.revision, before.history.past_len(), before.coalesce.as_deref())
        );
        cancel(h.state_mut());
        assert!(h.state().session.active().unwrap().doc.shares_state(&before.doc));
        assert_eq!(h.state().session.active().unwrap().coalesce, before.coalesce);
        let original = h.state().session.documents().iter().find(|state| state.doc.id == origin.doc.id).unwrap();
        assert!(original.doc.shares_state(&origin.doc));
        assert_eq!(
            (original.revision, original.history.past_len(), original.coalesce.as_deref()),
            (origin.revision, origin.history.past_len(), origin.coalesce.as_deref())
        );
    }
}
