use super::text_properties;
use crate::PhotocraftApp;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;

#[test]
#[ignore = "requires local unpublished independent multiline originals"]
fn line_spacing_control_preserves_source_native_and_history() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("text-multiline-evidence");
    let path = root.join("aa-32-H-E-edit-mixed-lines-line0.xcf");
    let bytes = std::fs::read(&path).unwrap();
    let original = photocraft_io::import("multiline.xcf", &bytes).unwrap().document;
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(420.0, 620.0)).build_ui_state(move |ui, app| text_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
    harness.run_steps(2);
    for spacing in [2, 4, 0] {
        harness.get_by_role(egui::accesskit::Role::ComboBox).click();
        harness.run_steps(2);
        harness.get_by_label(&format!("{spacing} px")).click();
        harness.run_steps(2);
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join(format!("aa-32-H-E-edit-mixed-lines-line{spacing}.xcf"))).unwrap())
            .unwrap()
            .document;
        let edited = &harness.state().session.active().unwrap().doc;
        let current = edited.xcf_state.as_ref().unwrap();
        let retained = original.xcf_state.as_ref().unwrap();
        assert_eq!(current.source_items[0].source, reference.xcf_state.as_ref().unwrap().source_items[0].source);
        assert_eq!(edited.layers, original.layers);
        assert_eq!(current.source_items[0].cache, retained.source_items[0].cache);
        assert_eq!(current.items, retained.items);
        assert_eq!(current.resources, retained.resources);
        assert_eq!(current.opaque, retained.opaque);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
            photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
        );
        let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
        let expected = photocraft_engine::transform_cmds::transform_bounds(&reference, &reference.layers[0]);
        assert_eq!(harness.state_mut().cached_layer_bounds(&original.layers[0]), Some(expected));
    }
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 3);
    for _ in 0..3 {
        harness.state_mut().run("edit.undo", json!({})).unwrap();
    }
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    harness.state_mut().run("layer.setProps", json!({"layer":layer.0,"locks":{"position":true}})).unwrap();
    harness.run_steps(2);
    assert!(harness.get_by_role(egui::accesskit::Role::ComboBox).accesskit_node().is_disabled());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
#[ignore = "requires local unpublished independent multiline originals"]
fn content_editor_keeps_two_lines_and_preserves_original_cache() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("text-multiline-evidence");
    let path = root.join("aa-32-H-E-edit-mixed-lines-line0.xcf");
    let bytes = std::fs::read(&path).unwrap();
    let original = photocraft_io::import("multiline.xcf", &bytes).unwrap().document;
    let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join("aa-32-H-E-edit-two-stems-line0.xcf")).unwrap()).unwrap().document;
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(420.0, 620.0)).build_ui_state(move |ui, app| text_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
    harness.run_steps(2);
    harness.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_role(egui::accesskit::Role::MultilineTextInput).type_text("I\nI");
    harness.run_steps(2);
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
    harness.get_by_label("Apply Text").click();
    harness.run_steps(2);
    let edited = &harness.state().session.active().unwrap().doc;
    let current = edited.xcf_state.as_ref().unwrap();
    let retained = original.xcf_state.as_ref().unwrap();
    assert_eq!(current.source_items[0].source, reference.xcf_state.as_ref().unwrap().source_items[0].source);
    assert_eq!(edited.layers, original.layers);
    assert_eq!(current.source_items[0].cache, retained.source_items[0].cache);
    assert_eq!(current.resources, retained.resources);
    assert_eq!(current.opaque, retained.opaque);
    assert_eq!(
        photocraft_compose::gimp_render::try_render(edited, edited.bounds(), &mut |_| Ok(())).unwrap(),
        photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap()
    );
    let native = photocraft_format::save_to_bytes(edited, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **edited);
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
    harness.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
