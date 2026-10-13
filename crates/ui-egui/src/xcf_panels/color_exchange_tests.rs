use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

#[test]
#[ignore = "requires local unpublished original Color Exchange evidence"]
fn original_color_exchange_controls_are_labelled_checked_and_locked() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-color-exchange-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let row = observations["records"].as_array().unwrap().iter().find(|row| row["id"] == "color-exchange-rgb-float_non_linear.small.source").unwrap();
    let path = root.join(row["file"].as_str().unwrap());
    let bytes = std::fs::read(&path).unwrap();
    let original = photocraft_io::import("original.xcf", &bytes).unwrap().document;
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(400.0, 700.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
    harness.run_steps(2);
    for label in ["From color", "To color", "Red threshold", "Green threshold", "Blue threshold"] {
        harness.get_by_label(label);
    }
    assert_eq!(harness.query_all_by_role(egui::accesskit::Role::SpinButton).count(), 3);
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    let reference_row = observations["records"].as_array().unwrap().iter().find(|row| row["id"] == "color-exchange-rgb-float_non_linear.small.edit").unwrap();
    let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join(reference_row["file"].as_str().unwrap())).unwrap()).unwrap().document;
    let reference_node = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let mut changes = 0;
    for (name, label) in [("red-threshold", "Red threshold"), ("green-threshold", "Green threshold"), ("blue-threshold", "Blue threshold")] {
        let value = &reference_node.arguments.iter().find(|argument| argument.name == name).unwrap().value;
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = value else { panic!("float threshold") };
        if original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().find(|argument| argument.name == name).unwrap().value == *value {
            continue;
        }
        harness.get_by_label(label).click();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label(label).type_text(&f32::from_bits(*bits).to_string());
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        changes += 1;
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), changes);
    }
    assert!(changes > 0);
    let edited = harness.state().session.active().unwrap().doc.clone();
    let current = edited.xcf_state.as_ref().unwrap();
    let retained = original.xcf_state.as_ref().unwrap();
    assert_eq!(current.filter_stacks[0].nodes[0].arguments, reference_node.arguments);
    assert_eq!(edited.layers[0].surface(), reference.layers[0].surface());
    assert_eq!(current.filter_masks, retained.filter_masks);
    assert_eq!(current.provenance, retained.provenance);
    assert_eq!(current.opaque, retained.opaque);
    assert_eq!(edited.source_color_context, original.source_color_context);
    assert_eq!(
        photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap().px,
        photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap().px
    );
    let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *edited);
    for _ in 0..changes {
        harness.state_mut().run("edit.undo", serde_json::json!({})).unwrap();
    }
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    harness.state_mut().run("edit.redo", serde_json::json!({})).unwrap();
    harness.state_mut().run("edit.undo", serde_json::json!({})).unwrap();
    let state = harness.state_mut().session.active_mut().unwrap();
    let doc = state.doc.try_resident_mut().unwrap();
    doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(layer)).unwrap().locks.content = true;
    harness.run_steps(2);
    assert!(harness.query_all_by_role(egui::accesskit::Role::SpinButton).all(|node| node.accesskit_node().is_disabled()));
    assert_eq!(harness.query_all_by_role(egui::accesskit::Role::ColorWell).count(), 2);
    assert!(harness.query_all_by_role(egui::accesskit::Role::ColorWell).all(|node| node.accesskit_node().is_disabled()));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn literal_color_labels_have_translations_in_every_catalog() {
    for language in crate::i18n::Lang::all().filter(|language| language.code() != "en") {
        for label in ["From color", "To color", "Red threshold", "Green threshold", "Blue threshold"] {
            assert!(crate::i18n::has(language, label), "{}: {label}", language.code());
        }
    }
}

#[test]
#[ignore = "requires local unpublished original Color Exchange evidence"]
fn original_color_exchange_picker_retains_source_alpha_native_and_history() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-color-exchange-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let row = observations["records"].as_array().unwrap().iter().find(|row| row["id"] == "color-exchange-rgb-float_non_linear.small.source").unwrap();
    let path = root.join(row["file"].as_str().unwrap());
    let bytes = std::fs::read(&path).unwrap();
    let original = photocraft_io::import("original.xcf", &bytes).unwrap().document;
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(650.0, 900.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
    harness.run_steps(2);
    assert_eq!(harness.get_by_label("To color").accesskit_node().role(), egui::accesskit::Role::ColorWell);
    harness.get_by_label("To color").click();
    harness.run_steps(2);
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0, "opening the picker must not edit operands");
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    assert_eq!(harness.query_all_by_role(egui::accesskit::Role::SpinButton).count(), 6);
    harness.get_by_label("Blue").click();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_label("Blue").type_text("0");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    let edited = harness.state().session.active().unwrap().doc.clone();
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
    let current = edited.xcf_state.as_ref().unwrap();
    let retained = original.xcf_state.as_ref().unwrap();
    let color = |state: &photocraft_doc::xcf_state::XcfState| {
        state.filter_stacks[0].nodes[0].arguments.iter().find(|argument| argument.name == "to-color").unwrap().value.clone()
    };
    let FilterValue::Color { components: before, .. } = color(retained) else { panic!("source literal") };
    let FilterValue::Color { components: after, encoding, profile } = color(current) else { panic!("edited literal") };
    assert_eq!(after[3], before[3]);
    assert_eq!(after[2], ExactNumber::Float32Bits(0));
    assert_eq!(encoding, photocraft_doc::SampleEncoding::NonlinearProfile);
    assert_eq!(profile, None);
    assert_eq!(current.filter_masks, retained.filter_masks);
    assert_eq!(current.provenance, retained.provenance);
    assert_eq!(current.opaque, retained.opaque);
    let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *edited);
    harness.state_mut().run("edit.undo", serde_json::json!({})).unwrap();
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    harness.state_mut().run("edit.redo", serde_json::json!({})).unwrap();
    assert_eq!(*harness.state().session.active().unwrap().doc, *edited);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
