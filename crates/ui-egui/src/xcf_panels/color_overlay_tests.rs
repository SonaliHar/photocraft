use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use sha2::{Digest, Sha256};

#[test]
#[ignore = "requires local unpublished original Color Overlay evidence"]
fn original_color_overlay_has_checked_rgba_and_srgb_widgets() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("point-color-overlay-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    let hash: String = Sha256::digest(&manifest).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, "828c1da757d0706cd6eb502699c74c0c2856745d3603c6ab6d1dd56d70df9831");
    let file = "rgb.float_non_linear.srgb0.source.xcf";
    let bytes = std::fs::read(root.join(file)).unwrap();
    let values: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let artifact = values["artifacts"].as_array().unwrap().iter().find(|row| row["path"] == file).unwrap();
    let source_hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(artifact["sha256"], source_hash);
    let original = photocraft_io::import("original.xcf", &bytes).unwrap().document;
    let layer = original.layers[0].id;
    let node = &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.schema_epoch, "xcf-22-stock-color-overlay-six-precision-v1");
    assert!(photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(node));
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(480., 760.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
    harness.run_steps(2);
    assert!(!harness.get_by_label("Color").accesskit_node().is_disabled());
    assert!(!harness.get_by_label("Process in sRGB").accesskit_node().is_disabled());
    harness.get_by_label("Color").click();
    harness.run_steps(2);
    for name in ["Red", "Green", "Blue", "Alpha"] {
        assert!(!harness.get_by_label(name).accesskit_node().is_disabled());
    }
    assert_eq!(harness.state().session.active().unwrap().history.past_len(), 0);
    assert_eq!(*harness.state().session.active().unwrap().doc, original);
}
fn packet() -> (std::path::PathBuf, serde_json::Value) {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("point-color-overlay-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest), "828c1da757d0706cd6eb502699c74c0c2856745d3603c6ab6d1dd56d70df9831");
    let entries: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), entries["artifacts"].as_array().unwrap().iter().find(|v| v["path"] == "observations.json").unwrap()["sha256"]);
    (root, serde_json::from_slice(&bytes).unwrap())
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn original(root: &std::path::Path, row: &serde_json::Value) -> photocraft_doc::Document {
    let bytes = std::fs::read(root.join(row["source"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    photocraft_io::import("overlay.xcf", &bytes).unwrap().document
}
fn projection(root: &std::path::Path, row: &serde_json::Value, doc: &photocraft_doc::Document) {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    use std::io::Read;
    let descriptor = &row["reopened"]["projection"];
    let compressed = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&compressed), descriptor["sha256"]);
    let mut values = Vec::new();
    flate2::read::ZlibDecoder::new(compressed.as_slice()).take(16385).read_to_end(&mut values).unwrap();
    assert_eq!(values.len() as u64, descriptor["decoded_bytes"].as_u64().unwrap());
    assert_eq!(hash(&values), descriptor["decoded_sha256"]);
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
    assert_eq!(rendered.px.len() * 32, values.len());
    for (pixel, raw) in rendered.px.iter().zip(values.as_chunks::<32>().0) {
        let mut color = [0.; 3];
        convert_encoding_f64(&profile, encoding, Encoding::NonlinearProfile, &pixel[..channels], &mut color[..channels]).unwrap();
        if gray {
            color = [color[0]; 3];
        }
        let alpha = f64::from_le_bytes(raw[24..32].try_into().unwrap());
        assert!((pixel[3] - alpha).abs() <= 1. / 65535.);
        for (i, value) in color.iter().enumerate() {
            let expected = f64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap());
            assert!((value * pixel[3] - expected * alpha).abs() <= 2. / 255., "{}", row["id"]);
        }
    }
}
fn verify(root: &std::path::Path, row: &serde_json::Value, current: &photocraft_doc::Document, source: &photocraft_doc::Document) {
    let reference = original(root, row);
    assert_eq!(current.layers, source.layers);
    assert_eq!(current.source_color_context, source.source_color_context);
    let state = current.xcf_state.as_ref().unwrap();
    let before = source.xcf_state.as_ref().unwrap();
    assert_eq!(state.provenance, before.provenance);
    assert_eq!(state.opaque, before.opaque);
    assert_eq!(state.filter_masks, before.filter_masks);
    assert_eq!(state.filter_stacks[0].nodes[0].arguments, reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments);
    assert_eq!(state.filter_stacks[0].nodes[0].enabled, reference.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    projection(root, row, current);
    assert_eq!(photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(current, &Default::default()).unwrap()).unwrap(), *current);
    let mut archive = std::io::Cursor::new(Vec::new());
    photocraft_io::save_native_stream(current, &mut archive, Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    let reopened = photocraft_format::load_from_stream(archive, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(reopened, *current);
    projection(root, row, &reopened);
}
#[test]
#[ignore = "requires independent Color Overlay originals and actual headless widgets"]
fn original_color_overlay_rgba_srgb_activation_native_history_and_locks() {
    let (root, observations) = packet();
    let rows = observations["records"].as_array().unwrap();
    let mut checked = 0;
    for model in ["rgb", "gray"] {
        for precision in ["u8_non_linear", "u16_non_linear", "u32_non_linear", "half_non_linear", "float_non_linear", "double_non_linear"] {
            let family = format!("{model}.{precision}.srgb0");
            let row = rows.iter().find(|row| row["id"] == format!("{family}.source")).unwrap();
            let source = original(&root, row);
            let layer = source.layers[0].id;
            let sourcebytes = std::fs::read(root.join(row["source"].as_str().unwrap())).unwrap();
            for stage in ["edit", "transparent", "opaque", "toggle", "disabled"] {
                let targetid = if stage == "toggle" { format!("{model}.{precision}.srgb1.source") } else { format!("{family}.{stage}") };
                let targetrow = rows.iter().find(|r| r["id"] == targetid).unwrap();
                let target = original(&root, targetrow);
                let targetnode = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
                let caption = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].source_display_name.clone().unwrap();
                let mut session = photocraft_engine::Session::new();
                session.add_document(source.clone(), None).unwrap();
                let app = PhotocraftApp::new(session, crate::Services::default());
                let visual = stage == "edit"
                    && matches!(precision, "half_non_linear" | "float_non_linear")
                    && std::env::var_os("PHOTOCRAFT_COLOR_OVERLAY_UI_VISUALS").is_some();
                let builder = Harness::builder().with_size(egui::vec2(480., 760.));
                let builder = if visual {
                    let mut setup = egui_kittest::wgpu::default_wgpu_setup();
                    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
                        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
                    }
                    builder.wgpu_setup(setup)
                } else {
                    builder
                };
                let mut h = builder.build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
                PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
                h.run_steps(2);
                let mut edits = 0;
                if matches!(stage, "toggle" | "disabled") {
                    h.get_by_label(if stage == "toggle" { "Process in sRGB" } else { &caption }).click();
                    h.run_steps(2);
                    edits = 1;
                } else {
                    h.get_by_label("Color").click();
                    h.run_steps(2);
                    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
                    assert_eq!(*h.state().session.active().unwrap().doc, source);
                    let FilterValue::Color { components, .. } = &targetnode.arguments.iter().find(|a| a.name == "value").unwrap().value else {
                        panic!("literal")
                    };
                    for (i, label) in ["Red", "Green", "Blue", "Alpha"].into_iter().enumerate() {
                        let current = h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
                            .arguments
                            .iter()
                            .find(|a| a.name == "value")
                            .unwrap()
                            .value
                            .clone();
                        let FilterValue::Color { components: before, .. } = current else { panic!("literal") };
                        if before[i] == components[i] {
                            continue;
                        }
                        let ExactNumber::Float32Bits(bits) = components[i] else { panic!("float literal") };
                        h.get_by_label(label).click();
                        h.run_steps(2);
                        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
                        h.get_by_label(label).type_text(&f32::from_bits(bits).to_string());
                        h.key_press(egui::Key::Enter);
                        h.run_steps(2);
                        edits += 1;
                        assert_eq!(h.state().session.active().unwrap().history.past_len(), edits);
                    }
                    if visual {
                        let out = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_COLOR_OVERLAY_UI_VISUALS").unwrap());
                        std::fs::create_dir_all(&out).unwrap();
                        h.render().unwrap().save(out.join(format!("{model}-{precision}.png"))).unwrap();
                    }
                    h.key_press(egui::Key::Escape);
                    h.run_steps(2);
                }
                assert!(edits > 0);
                let edited = h.state().session.active().unwrap().doc.clone();
                verify(&root, targetrow, &edited, &source);
                for _ in 0..edits {
                    h.state_mut().run("edit.undo", serde_json::json!({})).unwrap();
                }
                assert_eq!(*h.state().session.active().unwrap().doc, source);
                for _ in 0..edits {
                    h.state_mut().run("edit.redo", serde_json::json!({})).unwrap();
                }
                assert_eq!(*h.state().session.active().unwrap().doc, *edited);
                let active = h.state_mut().session.active_mut().unwrap();
                active
                    .doc
                    .try_resident_mut()
                    .unwrap()
                    .xcf_state
                    .as_mut()
                    .unwrap()
                    .items
                    .iter_mut()
                    .find(|v| v.item == photocraft_doc::item_identity::ItemRef::Layer(layer))
                    .unwrap()
                    .locks
                    .content = true;
                h.run_steps(2);
                for label in ["Color", "Process in sRGB", &caption] {
                    assert!(h.get_by_label(label).accesskit_node().is_disabled());
                }
                assert_eq!(std::fs::read(root.join(row["source"].as_str().unwrap())).unwrap(), sourcebytes);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 60);
}
#[test]
#[ignore = "requires independently authored Color Overlay source for malformed current UI controls"]
fn original_color_overlay_invalid_current_schema_widgets_fail_closed() {
    let (root, observations) = packet();
    let row = observations["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb.float_non_linear.srgb0.source").unwrap();
    let source = original(&root, row);
    for which in 0..8 {
        let mut document = source.clone();
        let node = &mut document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        match which {
            0 => node.operation.schema_epoch = "unknown".into(),
            1 => node.operation.stored_version = "future".into(),
            2 => node.arguments.push(node.arguments.iter().find(|a| a.name == "srgb").unwrap().clone()),
            _ => {
                let value = &mut node.arguments.iter_mut().find(|a| a.name == "value").unwrap().value;
                if which == 7 {
                    *value = FilterValue::Number(ExactNumber::Float32Bits(0));
                } else {
                    let FilterValue::Color { components, encoding, profile } = value else { panic!("color") };
                    match which {
                        3 => components[3] = ExactNumber::Float32Bits(f32::NAN.to_bits()),
                        4 => *encoding = photocraft_doc::SampleEncoding::Linear,
                        5 => *profile = Some(photocraft_doc::item_identity::ResourceId(1)),
                        _ => components.push(ExactNumber::Float32Bits(0)),
                    }
                }
            }
        }
        let layer = document.layers[0].id;
        let mut session = photocraft_engine::Session::new();
        session.add_document(document.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut h = Harness::builder().with_size(egui::vec2(480., 760.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        let controls: Vec<_> = h.query_all_by_label("Process in sRGB").collect();
        assert_eq!(controls.len(), if which == 2 { 2 } else { 1 }, "{which}");
        assert!(controls.iter().all(|control| control.accesskit_node().is_disabled()), "{which}");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
        assert!(*h.state().session.active().unwrap().doc == document, "UI must retain malformed current state without mutation");
    }
}

#[test]
#[ignore = "requires qualified Overlay source for exact current literal no-op controls"]
fn original_color_overlay_focused_noop_preserves_adjacent_float_and_signed_zero_bits() {
    let (root, observations) = packet();
    let row = observations["records"].as_array().unwrap().iter().find(|row| row["id"] == "rgb.float_non_linear.srgb0.source").unwrap();
    let source = original(&root, row);
    let layer = source.layers[0].id;
    let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let value = FilterValue::Color {
        components: [0.25_f32.to_bits() + 1, 0.5_f32.to_bits() + 1, 0.75_f32.to_bits() + 1, (-0.0_f32).to_bits()].map(ExactNumber::Float32Bits).to_vec(),
        encoding: photocraft_doc::SampleEncoding::NonlinearProfile,
        profile: None,
    };
    let mut session = photocraft_engine::Session::new();
    session.add_document(source.clone(), None).unwrap();
    session.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"value","value":value}]})).unwrap();
    let seeded = (*session.active().unwrap().doc).clone();
    let history = session.active().unwrap().history.past_len();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut h = Harness::builder().with_size(egui::vec2(480., 760.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    for label in ["Red", "Green", "Blue", "Alpha"] {
        h.get_by_label("Color").click();
        h.run_steps(2);
        h.get_by_label(label).click();
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().session.active().unwrap().doc.as_ref() == &seeded, "{label} focus/Escape must preserve literal bits");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    }
    assert_eq!(seeded.layers, source.layers);
    assert_eq!(seeded.source_color_context, source.source_color_context);
}
