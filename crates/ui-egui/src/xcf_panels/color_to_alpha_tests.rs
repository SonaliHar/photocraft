use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn assert_independent_projection(root: &std::path::Path, row: &serde_json::Value, doc: &photocraft_doc::Document) {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    use std::io::Read;
    let descriptor = &row["reopened"]["projection"];
    let compressed = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    let mut projection = Vec::new();
    flate2::read::ZlibDecoder::new(compressed.as_slice()).take(4097).read_to_end(&mut projection).unwrap();
    assert_eq!(projection.len(), descriptor["decoded_bytes"].as_u64().unwrap() as usize);
    assert_eq!(projection.len(), 8 * 6 * 32);
    assert_eq!(doc.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
    let profile = implicit_source_profile(doc.icc_profile.as_ref().unwrap(), photocraft_cms::ColorSpace::Rgb, Encoding::NonlinearProfile).unwrap();
    let rendered = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
    for (actual, expected) in rendered.px.iter().zip(projection.as_chunks::<32>().0) {
        let expected: [f64; 4] = std::array::from_fn(|i| f64::from_le_bytes(expected[i * 8..i * 8 + 8].try_into().unwrap()));
        let mut encoded = [0.; 3];
        convert_encoding_f64(&profile, Encoding::NonlinearProfile, Encoding::NonlinearProfile, &actual[..3], &mut encoded).unwrap();
        assert!((actual[3] - expected[3]).abs() <= 1. / 65535.);
        for channel in 0..3 {
            assert!((encoded[channel] * actual[3] - expected[channel] * expected[3]).abs() <= 2. / 255.);
            if expected[3] > 0. {
                assert!((encoded[channel] - expected[channel]).abs() <= 2. / 255.);
            }
        }
    }
}

#[test]
#[ignore = "requires local unpublished original Color to Alpha evidence"]
fn original_color_to_alpha_controls_are_labelled_checked_and_locked() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("point-color-to-alpha-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let row = observations["records"].as_array().unwrap().iter().find(|row| row["id"] == "color-to-alpha-rgb-float_non_linear.small.source").unwrap();
    let path = root.join(row["file"].as_str().unwrap());
    let bytes = std::fs::read(&path).unwrap();
    let original = photocraft_io::import("original.xcf", &bytes).unwrap().document;
    let node = &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.name, "gegl:color-to-alpha");
    assert_eq!(node.operation.stored_version, "0:0");
    assert_eq!(original.depth, photocraft_color::SampleType::F32);
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(400.0, 700.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
    harness.run_steps(2);
    for label in ["Color", "Transparency threshold", "Opacity threshold"] {
        harness.get_by_label(label);
    }
    assert_eq!(harness.query_all_by_role(egui::accesskit::Role::SpinButton).count(), 2);
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    let reference_row = observations["records"].as_array().unwrap().iter().find(|row| row["id"] == "color-to-alpha-rgb-float_non_linear.small.edit").unwrap();
    let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join(reference_row["file"].as_str().unwrap())).unwrap()).unwrap().document;
    let reference_node = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let mut changes = 0;
    for (name, label) in [("transparency-threshold", "Transparency threshold"), ("opacity-threshold", "Opacity threshold")] {
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
    assert_independent_projection(&root, reference_row, &edited);
    let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *edited);
    for _ in 0..changes {
        harness.state_mut().run("edit.undo", serde_json::json!({})).unwrap();
    }
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    for _ in 0..changes {
        harness.state_mut().run("edit.redo", serde_json::json!({})).unwrap();
    }
    assert_eq!(*harness.state().session.active().unwrap().doc, *edited);
    for _ in 0..changes {
        harness.state_mut().run("edit.undo", serde_json::json!({})).unwrap();
    }
    let state = harness.state_mut().session.active_mut().unwrap();
    let doc = state.doc.try_resident_mut().unwrap();
    doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(layer)).unwrap().locks.content = true;
    harness.run_steps(2);
    assert!(harness.query_all_by_role(egui::accesskit::Role::SpinButton).all(|node| node.accesskit_node().is_disabled()));
    assert_eq!(harness.query_all_by_role(egui::accesskit::Role::ColorWell).count(), 1);
    assert!(harness.query_all_by_role(egui::accesskit::Role::ColorWell).all(|node| node.accesskit_node().is_disabled()));
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn color_to_alpha_labels_have_translations_in_every_catalog() {
    for language in crate::i18n::Lang::all().filter(|language| language.code() != "en") {
        for label in ["Color", "Transparency threshold", "Opacity threshold"] {
            assert!(crate::i18n::has(language, label), "{}: {label}", language.code());
        }
    }
}

#[test]
#[ignore = "requires local unpublished original Color to Alpha evidence"]
fn original_color_to_alpha_picker_retains_source_alpha_native_and_history() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("point-color-to-alpha-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let row = observations["records"].as_array().unwrap().iter().find(|row| row["id"] == "color-to-alpha-rgb-float_non_linear.small.source").unwrap();
    let path = root.join(row["file"].as_str().unwrap());
    let bytes = std::fs::read(&path).unwrap();
    let original = photocraft_io::import("original.xcf", &bytes).unwrap().document;
    let node = &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.name, "gegl:color-to-alpha");
    assert_eq!(node.operation.stored_version, "0:0");
    assert_eq!(original.depth, photocraft_color::SampleType::F32);
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(650.0, 900.0)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
    harness.run_steps(2);
    assert_eq!(harness.get_by_label("Color").accesskit_node().role(), egui::accesskit::Role::ColorWell);
    harness.get_by_label("Color").click();
    harness.run_steps(2);
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0, "opening the picker must not edit operands");
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    assert_eq!(harness.query_all_by_role(egui::accesskit::Role::SpinButton).count(), 5);
    let reference_row = observations["records"].as_array().unwrap().iter().find(|row| row["id"] == "color-to-alpha-rgb-float_non_linear.small.colors").unwrap();
    let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join(reference_row["file"].as_str().unwrap())).unwrap()).unwrap().document;
    let reference_color =
        &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().find(|argument| argument.name == "color").unwrap().value;
    let FilterValue::Color { components: target, .. } = reference_color else { panic!("reference literal") };
    for (label, component) in ["Red", "Green", "Blue"].into_iter().zip(target) {
        let ExactNumber::Float32Bits(bits) = component else { panic!("F32 literal") };
        harness.get_by_label(label).click();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label(label).type_text(&f32::from_bits(*bits).to_string());
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
    }
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    let edited = harness.state().session.active().unwrap().doc.clone();
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 3);
    let current = edited.xcf_state.as_ref().unwrap();
    let retained = original.xcf_state.as_ref().unwrap();
    let color = |state: &photocraft_doc::xcf_state::XcfState| {
        state.filter_stacks[0].nodes[0].arguments.iter().find(|argument| argument.name == "color").unwrap().value.clone()
    };
    let FilterValue::Color { components: before, .. } = color(retained) else { panic!("source literal") };
    let FilterValue::Color { components: after, encoding, profile } = color(current) else { panic!("edited literal") };
    assert_eq!(after[3], before[3]);
    assert_eq!(after[..3], target[..3]);
    assert_ne!(after[3], target[3], "RGB controls retain the inert original alpha operand");
    assert_eq!(encoding, photocraft_doc::SampleEncoding::NonlinearProfile);
    assert_eq!(profile, None);
    assert_eq!(current.filter_masks, retained.filter_masks);
    assert_eq!(current.provenance, retained.provenance);
    assert_eq!(current.opaque, retained.opaque);
    assert_eq!(
        photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap().px,
        photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap().px
    );
    assert_eq!(edited.layers, original.layers);
    assert_independent_projection(&root, reference_row, &edited);
    let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *edited);
    for _ in 0..3 {
        harness.state_mut().run("edit.undo", serde_json::json!({})).unwrap();
    }
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
    for _ in 0..3 {
        harness.state_mut().run("edit.redo", serde_json::json!({})).unwrap();
    }
    assert_eq!(*harness.state().session.active().unwrap().doc, *edited);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
