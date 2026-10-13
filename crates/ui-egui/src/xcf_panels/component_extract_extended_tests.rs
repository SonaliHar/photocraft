use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn packet(family: &str) -> std::path::PathBuf {
    std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join(format!("point-component-extract-{family}-evidence"))
}
fn load_original(root: &std::path::Path, stem: &str) -> photocraft_doc::Document {
    use sha2::Digest;
    let bytes = std::fs::read(root.join("states").join(format!("{stem}.xcf"))).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("batch-manifest.json")).unwrap()).unwrap();
    let case = manifest["cases"].as_array().unwrap().iter().find(|case| case["id"] == stem).unwrap();
    let digest: String = sha2::Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(case["sha256"], digest);
    photocraft_io::import("original.xcf", &bytes).unwrap().document
}
#[test]
#[ignore = "requires qualified independent local CMYK and YCbCr Component originals"]
fn original_extended_component_controls_current_native_history_locks_and_projection() {
    for (family, first, choices) in [
        ("cmyk", "cmyk-c", vec![("cmyk-c", "Cyan"), ("cmyk-m", "Magenta"), ("cmyk-y", "Yellow"), ("cmyk-k", "Black")]),
        ("ycbcr", "ycbcr-y", vec![("ycbcr-y", "Y'CbCr Y'"), ("ycbcr-cb", "Y'CbCr Cb"), ("ycbcr-cr", "Y'CbCr Cr")]),
    ] {
        let root = packet(family);
        let source = format!("rgb.float_non_linear.{first}.source");
        let source_path = root.join("states").join(format!("{source}.xcf"));
        let immutable_source = std::fs::read(&source_path).unwrap();
        let original = load_original(&root, &source);
        let layer = original.layers[0].id;
        let node = &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert_eq!(node.operation.name, "gegl:component-extract");
        assert_eq!(node.operation.stored_version, "0:0");
        assert_eq!(node.operation.schema_epoch, format!("xcf-22-stock-component-extract-{family}-precision-v1"));
        let mut session = photocraft_engine::Session::new();
        session.add_document(original.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut h = Harness::builder().with_size(egui::vec2(450., 750.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        h.get_by_label("Component");
        h.get_by_label("Invert");
        h.get_by_label("Linear");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
        h.get_by_label("Component").click();
        h.run_steps(2);
        for (_, label) in &choices {
            h.get_by_label(label);
        }
        assert!(h.query_all_by_label("Red").next().is_none());
        h.get_by_label(choices[0].1).click();
        h.run_steps(2);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 0, "unchanged selector is read-only");
        let mut edits: Vec<_> =
            choices.iter().skip(1).map(|(nick, label)| (format!("rgb.float_non_linear.{nick}.source"), "Component", Some(*label))).collect();
        edits.extend([(format!("rgb.float_non_linear.{first}.invert"), "Invert", None), (format!("rgb.float_non_linear.{first}.linear"), "Linear", None)]);
        for (target, control, choice) in edits {
            h.get_by_label(control).click();
            h.run_steps(2);
            if let Some(choice) = choice {
                assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
                h.get_by_label(choice).click();
                h.run_steps(2);
            }
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
            let reference = load_original(&root, &target);
            let current = h.state().session.active().unwrap().doc.clone();
            let state = current.xcf_state.as_ref().unwrap();
            let retained = original.xcf_state.as_ref().unwrap();
            assert_eq!(state.filter_stacks[0].nodes[0].arguments, reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
            assert_eq!(current.layers, original.layers);
            assert_eq!(current.source_color_context, original.source_color_context);
            assert_eq!(state.provenance, retained.provenance);
            assert_eq!(state.opaque, retained.opaque);
            assert_eq!(state.filter_masks, retained.filter_masks);
            component_extract_tests::independent_projection(&root.join("states"), &target, &current);
            assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&current, &Default::default()).unwrap()).unwrap(), *current);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(*h.state().session.active().unwrap().doc, original);
            h.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(*h.state().session.active().unwrap().doc, *current);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            h.run_steps(2);
        }
        let st = h.state_mut().session.active_mut().unwrap();
        let doc = st.doc.try_resident_mut().unwrap();
        doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(layer)).unwrap().locks.content = true;
        h.run_steps(2);
        for label in ["Component", "Invert", "Linear"] {
            assert!(h.get_by_label(label).accesskit_node().is_disabled());
        }
        assert_eq!(std::fs::read(source_path).unwrap(), immutable_source);
    }
}

#[test]
fn extended_component_labels_translated_in_every_catalog() {
    for language in crate::i18n::Lang::all().filter(|language| language.code() != "en") {
        for label in ["Cyan", "Magenta", "Yellow", "Black", "Y'CbCr Y'", "Y'CbCr Cb", "Y'CbCr Cr"] {
            assert!(crate::i18n::has(language, label), "{}: {label}", language.code());
        }
    }
}
