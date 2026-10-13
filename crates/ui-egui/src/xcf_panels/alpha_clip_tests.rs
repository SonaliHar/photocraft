use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
fn evidence() -> std::path::PathBuf {
    std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("point-alpha-clip-evidence")
}
pub(super) fn projection(root: &std::path::Path, row: &serde_json::Value, doc: &photocraft_doc::Document) {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    use std::io::Read;
    let descriptor = &row["reopened"]["projection"];
    let compressed = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    let mut expected = Vec::new();
    flate2::read::ZlibDecoder::new(compressed.as_slice()).take(1537).read_to_end(&mut expected).unwrap();
    assert_eq!(expected.len(), 8 * 6 * 32);
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
#[ignore = "requires qualified independent local Alpha Clip originals"]
fn original_alpha_clip_controls_current_native_history_locks_and_noop() {
    let root = evidence();
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let rows = observations["records"].as_array().unwrap();
    let row = |role: &str| rows.iter().find(|row| row["id"] == format!("rgb-float_non_linear.{role}")).unwrap();
    let source = root.join(row("source")["file"].as_str().unwrap());
    let bytes = std::fs::read(&source).unwrap();
    let original = photocraft_io::import("original.xcf", &bytes).unwrap().document;
    assert_eq!(original.depth, photocraft_color::SampleType::F32);
    let source_node = &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(source_node.operation.name, "gegl:alpha-clip");
    assert_eq!(source_node.operation.stored_version, "0:0");
    assert_eq!(source_node.operation.schema_epoch, "xcf22-stock-alpha-clip-precision-v1");
    let effect_label = source_node.source_display_name.as_ref().unwrap().clone();
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut h = Harness::builder().with_size(egui::vec2(450., 750.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    for label in ["Clip low", "Clip high", "Lower limit", "Upper limit"] {
        h.get_by_label(label);
    }
    assert_eq!(h.query_all_by_role(egui::accesskit::Role::SpinButton).count(), 2);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    let mut noop_pixels = None;
    for (role, controls) in [
        ("low-only", vec!["Clip high"]),
        ("high-only", vec!["Clip low"]),
        ("neither", vec!["Clip low", "Clip high"]),
        ("disabled", vec![effect_label.as_str()]),
        ("edit", vec!["Lower limit", "Upper limit"]),
    ] {
        let reference = photocraft_io::import("reference.xcf", &std::fs::read(root.join(row(role)["file"].as_str().unwrap())).unwrap()).unwrap().document;
        let target = &reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        for label in &controls {
            h.get_by_label(label).click();
            h.run_steps(2);
            if matches!(*label, "Lower limit" | "Upper limit") {
                let name = if *label == "Lower limit" { "low-limit" } else { "high-limit" };
                let FilterValue::Number(ExactNumber::Float32Bits(bits)) = target.arguments.iter().find(|a| a.name == name).unwrap().value else {
                    panic!("F32 limit")
                };
                h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
                h.get_by_label(label).type_text(&f32::from_bits(bits).to_string());
                h.key_press(egui::Key::Enter);
                h.run_steps(2);
            }
        }
        let current = h.state().session.active().unwrap().doc.clone();
        let state = current.xcf_state.as_ref().unwrap();
        let retained = original.xcf_state.as_ref().unwrap();
        assert_eq!(state.filter_stacks[0].nodes[0].arguments, target.arguments);
        assert_eq!(state.filter_stacks[0].nodes[0].enabled, target.enabled);
        assert_eq!(current.layers, original.layers);
        assert_eq!(current.source_color_context, original.source_color_context);
        assert_eq!(state.provenance, retained.provenance);
        assert_eq!(state.opaque, retained.opaque);
        assert_eq!(state.filter_masks, retained.filter_masks);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), controls.len());
        projection(&root, row(role), &current);
        if matches!(role, "neither" | "disabled") {
            let pixels = photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap().px;
            if let Some(expected) = &noop_pixels {
                assert_eq!(&pixels, expected, "both flags false and disabled preserve exactly the same input pixels");
            } else {
                noop_pixels = Some(pixels);
            }
        }
        let native = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *current);
        for _ in &controls {
            h.state_mut().run("edit.undo", json!({})).unwrap();
        }
        assert_eq!(*h.state().session.active().unwrap().doc, original);
        for _ in &controls {
            h.state_mut().run("edit.redo", json!({})).unwrap();
        }
        assert_eq!(*h.state().session.active().unwrap().doc, *current);
        for _ in &controls {
            h.state_mut().run("edit.undo", json!({})).unwrap();
        }
        h.run_steps(2);
    }
    h.get_by_label("Lower limit").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Lower limit").type_text("0.25");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0, "focus and unchanged limit create no history");
    assert_eq!(*h.state().session.active().unwrap().doc, original);
    let st = h.state_mut().session.active_mut().unwrap();
    let doc = st.doc.try_resident_mut().unwrap();
    doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Layer(layer)).unwrap().locks.content = true;
    h.run_steps(2);
    for label in ["Clip low", "Clip high", "Lower limit", "Upper limit"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
    }
    assert_eq!(std::fs::read(source).unwrap(), bytes);
}

#[test]
fn alpha_clip_labels_are_translated_in_every_catalog() {
    for language in crate::i18n::Lang::all().filter(|language| language.code() != "en") {
        for label in ["Clip low", "Clip high", "Lower limit", "Upper limit"] {
            assert!(crate::i18n::has(language, label), "{}: {label}", language.code());
        }
    }
}
