use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use sha2::{Digest, Sha256};

fn activation_checkbox<'a>(h: &'a Harness<'_, PhotocraftApp>, caption: &'a str) -> egui_kittest::Node<'a> {
    let mut matches = h.query_all_by_label(caption).filter(|node| node.accesskit_node().role() == egui::accesskit::Role::CheckBox);
    let node = matches.next().expect("the actual filter activation checkbox");
    assert!(matches.next().is_none(), "activation checkbox must be unique");
    node
}
fn packet() -> std::path::PathBuf {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("red-eye-removal-precision-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "821fad4b046cae0174291934fa61b01f5fc966cd4d612e4ec77eeb0cb6fb6e5f");
    root
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
#[ignore = "independently authored original Red Eye source and actual accessible widget"]
fn original_red_eye_threshold_widget_is_labelled_and_retains_source_on_focus() {
    let root = packet();
    let bytes = std::fs::read(root.join("rgb.float_non_linear.source.xcf")).unwrap();
    assert_eq!(hash(&bytes), "3d8afbab52b1f245a0ce78fc2e3ea63ab03180d2c35942191f75827b11ce8ae1");
    let source = photocraft_io::import("red-eye.xcf", &bytes).unwrap().document;
    let layer = source.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(source.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, Default::default());
    let mut harness = Harness::builder().with_size(egui::vec2(440., 340.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
    harness.run_steps(2);
    assert!(!harness.get_by_label("Threshold").accesskit_node().is_disabled());
    harness.get_by_label("Threshold").click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
    assert_eq!(*harness.state().session.active().unwrap().doc, source);
    harness.get_by_label("Threshold").click();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_label("Threshold").type_text("0.79999995");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    let current = harness.state().session.active().unwrap().doc.clone();
    assert_eq!(current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].value, FilterValue::Number(ExactNumber::Float32Bits(0x3f4ccccc)));
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
    harness.get_by_label("Threshold").click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(*harness.state().session.active().unwrap().doc, *current);
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
    harness.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(*harness.state().session.active().unwrap().doc, source);
    assert_eq!(std::fs::read(root.join("rgb.float_non_linear.source.xcf")).unwrap(), bytes);
}

fn original(root: &std::path::Path, rows: &[serde_json::Value], id: &str) -> (photocraft_doc::Document, serde_json::Value) {
    let row = rows.iter().find(|row| row["id"] == id).unwrap().clone();
    let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    (photocraft_io::import("original-red-eye.xcf", &bytes).unwrap().document, row)
}
fn projection(root: &std::path::Path, row: &serde_json::Value, doc: &photocraft_doc::Document) {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    let descriptor = &row["current"]["projection"];
    let bytes = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), descriptor["sha256"]);
    assert_eq!(bytes.len() as u64, descriptor["bytes"].as_u64().unwrap());
    let gray = doc.mode == photocraft_doc::ColorMode::Grayscale;
    let channels = if gray { 1 } else { 3 };
    let encoding = match doc.sample_encoding.unwrap() {
        photocraft_doc::SampleEncoding::Linear => Encoding::Linear,
        photocraft_doc::SampleEncoding::NonlinearProfile => Encoding::NonlinearProfile,
        photocraft_doc::SampleEncoding::Perceptual => Encoding::Perceptual,
    };
    let profile = implicit_source_profile(
        doc.icc_profile.as_ref().unwrap(),
        if gray { photocraft_cms::ColorSpace::Gray } else { photocraft_cms::ColorSpace::Rgb },
        encoding,
    )
    .unwrap();
    let rendered = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(rendered.px.len() * 32, bytes.len());
    for (pixel, raw) in rendered.px.iter().zip(bytes.as_chunks::<32>().0) {
        let mut rgb = [0.; 3];
        convert_encoding_f64(&profile, encoding, Encoding::NonlinearProfile, &pixel[..channels], &mut rgb[..channels]).unwrap();
        if gray {
            rgb = [rgb[0]; 3];
        }
        let alpha = f64::from_le_bytes(raw[24..32].try_into().unwrap());
        assert!((pixel[3] - alpha).abs() <= 1. / 65535.);
        for (channel, value) in rgb.iter().enumerate() {
            let expected = f64::from_le_bytes(raw[channel * 8..channel * 8 + 8].try_into().unwrap());
            assert!((value * pixel[3] - expected * alpha).abs() <= 2. / 255., "{}", row["id"]);
        }
    }
}
fn verify(root: &std::path::Path, row: &serde_json::Value, current: &photocraft_doc::Document, source: &photocraft_doc::Document) {
    assert_eq!(current.layers, source.layers);
    assert_eq!(current.source_color_context, source.source_color_context);
    let state = current.xcf_state.as_ref().unwrap();
    let before = source.xcf_state.as_ref().unwrap();
    assert_eq!(state.provenance, before.provenance);
    assert_eq!(state.opaque, before.opaque);
    assert_eq!(state.filter_masks, before.filter_masks);
    projection(root, row, current);
    assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(current, &Default::default()).unwrap()).unwrap(), *current);
    let mut archive = std::io::Cursor::new(Vec::new());
    photocraft_io::save_native_stream(current, &mut archive, Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(photocraft_format::load_from_stream(archive, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap(), *current);
}
#[test]
#[ignore = "representative independent Red Eye widget/native/history references, optional Vulkan capture"]
fn original_red_eye_representative_edits_activation_native_history_and_boundaries() {
    let root = packet();
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("batch-manifest.json")).unwrap()).unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), manifest["artifacts"].as_array().unwrap().iter().find(|entry| entry["path"] == "observations.json").unwrap()["sha256"]);
    let observations: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let rows = observations["records"].as_array().unwrap();
    for family in ["rgb.float_non_linear", "gray.double_linear", "rgb.half_non_linear"] {
        let (source, _) = original(&root, rows, &format!("{family}.source"));
        let layer = source.layers[0].id;
        let caption = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].source_display_name.clone().unwrap();
        for stage in ["edit", "disabled"] {
            let (target, row) = original(&root, rows, &format!("{family}.{stage}"));
            let visual = family == "rgb.float_non_linear" && stage == "edit" && std::env::var_os("PHOTOCRAFT_RED_EYE_UI_VISUALS").is_some();
            let mut builder = Harness::builder().with_size(egui::vec2(440., 340.));
            if visual {
                let mut setup = egui_kittest::wgpu::default_wgpu_setup();
                if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
                    create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
                }
                builder = builder.wgpu_setup(setup);
            }
            let mut session = photocraft_engine::Session::new();
            session.add_document(source.clone(), None).unwrap();
            let mut harness = builder.build_ui_state(move |ui, app| filter_properties(app, ui, layer), PhotocraftApp::new(session, Default::default()));
            PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
            harness.run_steps(2);
            if stage == "edit" {
                harness.get_by_label("Threshold").click();
                harness.run_steps(2);
                harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
                harness.get_by_label("Threshold").type_text("0.8");
                harness.key_press(egui::Key::Enter);
                harness.run_steps(2);
                assert_eq!(*harness.state().session.active().unwrap().doc, source);
                assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
                harness.get_by_label("Threshold").click();
                harness.run_steps(2);
                harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
                harness.get_by_label("Threshold").type_text("0.625");
                harness.key_press(egui::Key::Enter);
                harness.run_steps(2);
            } else {
                activation_checkbox(&harness, &caption).click();
                harness.run_steps(2);
            }
            let current = harness.state().session.active().unwrap().doc.clone();
            assert_eq!(harness.state().session.active().unwrap().history.past_len(), 1);
            let actual = &current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let expected = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            assert_eq!(actual.arguments, expected.arguments);
            assert_eq!(actual.enabled, expected.enabled);
            verify(&root, &row, &current, &source);
            if visual {
                let directory = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_RED_EYE_UI_VISUALS").unwrap());
                std::fs::create_dir_all(&directory).unwrap();
                harness.render().unwrap().save(directory.join("red-eye-threshold.png")).unwrap();
            }
            harness.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(*harness.state().session.active().unwrap().doc, source);
            harness.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(*harness.state().session.active().unwrap().doc, *current);
            harness.state_mut().session.active_mut().unwrap().doc.try_resident_mut().unwrap().layers[0].locks.pixels = true;
            harness.run_steps(2);
            for label in ["Threshold", &caption] {
                if label == caption {
                    assert!(activation_checkbox(&harness, label).accesskit_node().is_disabled());
                } else {
                    assert!(harness.get_by_label(label).accesskit_node().is_disabled());
                }
            }
        }
    }
}
