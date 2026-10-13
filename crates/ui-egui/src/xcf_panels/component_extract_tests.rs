use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn evidence() -> std::path::PathBuf {
    std::env::var_os("PHOTOCRAFT_COMPONENT_EXTRACT_EVIDENCE").map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
            .join("point-component-extract-evidence/enabled")
    })
}

#[test]
fn component_extract_labels_are_translated_in_every_catalog() {
    for language in crate::i18n::Lang::all().filter(|language| language.code() != "en") {
        for label in ["Component", "Invert", "Linear", "Red", "Green", "Blue", "Alpha"] {
            assert!(crate::i18n::has(language, label), "{}: {label}", language.code());
        }
    }
}

pub(super) fn independent_projection(root: &std::path::Path, stem: &str, doc: &photocraft_doc::Document) {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    use std::io::Read;
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(format!("{stem}.fresh.state.json"))).unwrap()).unwrap();
    assert_eq!(state["size"], json!([8, 6]));
    assert_eq!(state["precision"], "GIMP_PRECISION_FLOAT_NON_LINEAR");
    let descriptor = &state["projection"];
    let compressed = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    let mut expected = Vec::new();
    flate2::read::ZlibDecoder::new(compressed.as_slice()).take(1537).read_to_end(&mut expected).unwrap();
    assert_eq!(expected.len(), 8 * 6 * 32);
    assert_eq!(descriptor["decoded_bytes"], expected.len());
    let profile = implicit_source_profile(doc.icc_profile.as_ref().unwrap(), photocraft_cms::ColorSpace::Rgb, Encoding::NonlinearProfile).unwrap();
    let rendered = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
    for (actual, expected) in rendered.px.iter().zip(expected.as_chunks::<32>().0) {
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
#[ignore = "requires qualified independent local Component Extract originals"]
fn original_component_controls_current_native_history_locks_and_projection() {
    let root = evidence();
    let source = "rgb.float_non_linear.rgb-r.invert0.linear0";
    let path = root.join(format!("{source}.xcf"));
    let bytes = std::fs::read(&path).unwrap();
    let original = photocraft_io::import("original.xcf", &bytes).unwrap().document;
    assert_eq!(original.depth, photocraft_color::SampleType::F32);
    assert_eq!(original.sample_encoding, Some(photocraft_doc::SampleEncoding::NonlinearProfile));
    let original_node = &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(original_node.operation.name, "gegl:component-extract");
    assert_eq!(original_node.operation.stored_version, "0:0");
    assert_eq!(original_node.operation.schema_epoch, "xcf-22-stock-component-extract-four-precision-v1");
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut h = Harness::builder().with_size(egui::vec2(450., 750.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    h.get_by_label("Component");
    h.get_by_label("Invert");
    h.get_by_label("Linear");
    assert_eq!(*h.state().session.active().unwrap().doc, original);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    for (target, control, choice) in [
        ("rgb.float_non_linear.rgb-g.invert0.linear0", "Component", Some("Green")),
        ("rgb.float_non_linear.rgb-b.invert0.linear0", "Component", Some("Blue")),
        ("rgb.float_non_linear.alpha.invert0.linear0", "Component", Some("Alpha")),
        ("rgb.float_non_linear.rgb-r.invert1.linear0", "Invert", None),
        ("rgb.float_non_linear.rgb-r.invert0.linear1", "Linear", None),
    ] {
        h.get_by_label(control).click();
        h.run_steps(2);
        if let Some(choice) = choice {
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 0, "opening the selector is read-only");
            for caption in ["Red", "Green", "Blue", "Alpha"] {
                assert!(h.query_all_by_label(caption).next().is_some(), "{caption} choice");
            }
            h.get_by_label(choice).click();
            h.run_steps(2);
        }
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join(format!("{target}.xcf"))).unwrap()).unwrap().document;
        let current = h.state().session.active().unwrap().doc.clone();
        let state = current.xcf_state.as_ref().unwrap();
        let retained = original.xcf_state.as_ref().unwrap();
        assert_eq!(state.filter_stacks[0].nodes[0].arguments, reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
        assert_eq!(current.layers, original.layers);
        assert_eq!(current.source_color_context, original.source_color_context);
        assert_eq!(state.provenance, retained.provenance);
        assert_eq!(state.opaque, retained.opaque);
        assert_eq!(state.filter_masks, retained.filter_masks);
        independent_projection(&root, target, &current);
        let native = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *current);
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
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label}");
    }
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
