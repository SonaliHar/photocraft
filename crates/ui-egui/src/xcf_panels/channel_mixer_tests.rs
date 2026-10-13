use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
fn original_context(context: &str, stage: &str) -> photocraft_doc::Document {
    use sha2::Digest;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/channel-mixer-precision-evidence");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    let hash = |bytes: &[u8]| sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hash(&bytes), "91be8128c1d351336cd7ef304dd81d42b010aedba1610384aed9526a7ca04f20");
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let bytes = std::fs::read(root.join(format!("{context}.{stage}.xcf"))).unwrap();
    let file = format!("{context}.{stage}.xcf");
    assert_eq!(hash(&bytes), manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == file).unwrap()["sha256"]);
    photocraft_io::import("original.xcf", &bytes).unwrap().document
}
#[test]
#[ignore = "requires independent local Channel Mixer references"]
fn channel_mixer_actual_widgets_have_exact_signed_gain_labels() {
    let doc = original_context("rgb.float_non_linear", "source");
    let layer = doc.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut h = Harness::builder().with_size(egui::vec2(450., 950.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    for output in ["Red", "Green", "Blue"] {
        for input in ["Red", "Green", "Blue"] {
            assert!(!h.get_by_label(&format!("{output} / {input}")).accesskit_node().is_disabled());
        }
    }
    assert!(!h.get_by_label("Preserve luminosity").accesskit_node().is_disabled());
}

const GAINS: [(&str, &str); 9] = [
    ("rr-gain", "Red / Red"),
    ("rg-gain", "Red / Green"),
    ("rb-gain", "Red / Blue"),
    ("gr-gain", "Green / Red"),
    ("gg-gain", "Green / Green"),
    ("gb-gain", "Green / Blue"),
    ("br-gain", "Blue / Red"),
    ("bg-gain", "Blue / Green"),
    ("bb-gain", "Blue / Blue"),
];
fn value(doc: &photocraft_doc::Document, name: &str) -> FilterValue {
    doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.iter().find(|a| a.name == name).unwrap().value.clone()
}
fn native(doc: &photocraft_doc::Document) {
    let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *doc);
    let mut bytes = std::io::Cursor::new(Vec::new());
    photocraft_io::save_native_stream(doc, &mut bytes, Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(photocraft_format::load_from_stream(bytes, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap(), *doc);
}
#[test]
#[ignore = "requires independent local Channel Mixer references"]
fn channel_mixer_widgets_edit_representative_originals_native_history_and_half_storage() {
    for context in ["rgb.float_non_linear", "gray.double_linear", "rgb.half_non_linear"] {
        let source = original_context(context, "source");
        let layer = source.layers[0].id;
        let caption = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].source_display_name.clone().unwrap();
        let mut session = photocraft_engine::Session::new();
        session.add_document(source.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, Default::default());
        let visual = context == "rgb.float_non_linear" && std::env::var_os("PHOTOCRAFT_CHANNEL_UI_VISUALS").is_some();
        let mut builder = Harness::builder().with_size(egui::vec2(460., 620.));
        if visual {
            let mut setup = egui_kittest::wgpu::default_wgpu_setup();
            if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
                create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
            }
            builder = builder.wgpu_setup(setup);
        }
        let mut h = builder.build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        h.get_by_label("Red / Red").click();
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert_eq!(*h.state().session.active().unwrap().doc, source);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
        for stage in ["signed", "preserve", "disabled"] {
            let target = original_context(context, stage);
            for (name, label) in GAINS {
                if value(&h.state().session.active().unwrap().doc, name) == value(&target, name) {
                    continue;
                }
                let FilterValue::Number(ExactNumber::Float32Bits(bits)) = value(&target, name) else { panic!("original wire F32") };
                h.get_by_label(label).click();
                h.run_steps(2);
                h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
                h.get_by_label(label).type_text(&f32::from_bits(bits).to_string());
                h.key_press(egui::Key::Enter);
                h.run_steps(2);
                assert_eq!(value(&h.state().session.active().unwrap().doc, name), value(&target, name));
            }
            if value(&h.state().session.active().unwrap().doc, "preserve-luminosity") != value(&target, "preserve-luminosity") {
                h.get_by_label("Preserve luminosity").click();
                h.run_steps(2);
            }
            let target_enabled = target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled;
            if h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled != target_enabled {
                h.get_by_label(&caption).click();
                h.run_steps(2);
            }
            let current = h.state().session.active().unwrap().doc.clone();
            assert_eq!(current.layers, source.layers);
            assert_eq!(current.xcf_state.as_ref().unwrap().provenance, source.xcf_state.as_ref().unwrap().provenance);
            let render = |doc: &photocraft_doc::Document| photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
            let actual = render(&current);
            let expected = render(&target);
            for (a, b) in actual.px.iter().zip(&expected.px) {
                assert!((a[3] - b[3]).abs() <= 1.0 / 65535.0);
                for c in 0..3 {
                    assert!((a[c] * a[3] - b[c] * b[3]).abs() <= 2.0 / 255.0, "{context}/{stage}/{c}");
                }
            }
            native(&current);
            if visual && stage == "preserve" {
                let directory = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_CHANNEL_UI_VISUALS").unwrap());
                std::fs::create_dir_all(&directory).unwrap();
                h.render().unwrap().save(directory.join("channel-mixer-preserve.png")).unwrap();
            }
            h.state_mut().run("edit.undo", json!({})).unwrap();
            h.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(*h.state().session.active().unwrap().doc, *current);
        }
    }
}

#[test]
#[ignore = "requires independent Channel Mixer source and current Double authority"]
fn channel_mixer_double_widget_keeps_sub_float_bits_focus_noop_and_history() {
    let source = original_context("rgb.float_non_linear", "source");
    let layer = source.layers[0].id;
    let id = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    let mut session = photocraft_engine::Session::new();
    session.add_document(source.clone(), None).unwrap();
    let exact = 0.75000000001_f64;
    session
        .execute(
            "xcf.filter.set",
            json!({"filterId":id,"arguments":[{"name":"rr-gain","value":{"Number":{"Float64Bits":format!("0x{:016x}",exact.to_bits())}}}]}),
        )
        .unwrap();
    let seeded = session.active().unwrap().doc.clone();
    let app = PhotocraftApp::new(session, Default::default());
    let mut h = Harness::builder().with_size(egui::vec2(460., 950.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    h.get_by_label("Red / Red").click();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(*h.state().session.active().unwrap().doc, *seeded);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
    for invalid in ["2.0000001", "NaN", "inf"] {
        h.get_by_label("Red / Red").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Red / Red").type_text(invalid);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert_eq!(*h.state().session.active().unwrap().doc, *seeded);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
    }
    h.get_by_label("Red / Red").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Red / Red").type_text("-0.50000000001");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let current = h.state().session.active().unwrap().doc.clone();
    assert_eq!(value(&current, "rr-gain"), FilterValue::Number(ExactNumber::Float64Bits((-0.50000000001_f64).to_bits())));
    assert_eq!(current.layers, source.layers);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 2);
    native(&current);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, *seeded);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, *current);
}

#[test]
#[ignore = "requires independent Channel Mixer Half promotion reference"]
fn channel_mixer_promoted_half_widget_retains_original_samples_and_float_current_authority() {
    use sha2::Digest;
    let source = original_context("rgb.half_non_linear", "signed");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/channel-mixer-half-promotion-evidence");
    let hash = |bytes: &[u8]| sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "d18d1a9bf42df4caf7ac5946d3252561c6c87c3492a1db28a62134f2b1d68109");
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let checked = |file: &str| {
        let bytes = std::fs::read(root.join(file)).unwrap();
        assert_eq!(hash(&bytes), manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == file).unwrap()["sha256"]);
        bytes
    };
    let obs: serde_json::Value = serde_json::from_slice(&checked("observations.json")).unwrap();
    let row = obs["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb.half_non_linear.signed").unwrap();
    let _promoted_xcf_bytes = checked(row["file"].as_str().unwrap());
    let layer = source.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(source.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, Default::default());
    let mut h = Harness::builder().with_size(egui::vec2(460., 620.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    h.state_mut().run("image.mode.bits32", json!({})).unwrap();
    h.run_steps(2);
    let promoted = h.state().session.active().unwrap().doc.clone();
    assert_eq!(promoted.xcf_state.as_ref().unwrap().current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
    assert_eq!(promoted.layers, source.layers);

    let fresh: serde_json::Value = serde_json::from_slice(&checked("fresh-observations.json")).unwrap();
    let row = fresh["records"].as_array().unwrap().iter().find(|r| r["id"] == "rgb.half_non_linear.signed").unwrap();
    assert_eq!(
        promoted.layers[0].surface().unwrap().try_read_interleaved(promoted.bounds()).unwrap(),
        checked(row["current"]["layers"][0]["native"]["file"].as_str().unwrap())
    );
    let bytes = checked(row["current"]["projection"]["file"].as_str().unwrap());
    let render = photocraft_compose::gimp_render::try_render(&promoted, promoted.bounds(), &mut |_| Ok(())).unwrap();
    for (a, raw) in render.px.iter().zip(bytes.as_chunks::<32>().0) {
        let mut expected = [0.; 4];
        for (c, v) in expected.iter_mut().enumerate() {
            *v = f64::from_le_bytes(raw[c * 8..c * 8 + 8].try_into().unwrap());
        }
        assert!((a[3] - expected[3]).abs() <= 1. / 65535.);
        for c in 0..3 {
            assert!((a[c] * a[3] - expected[c] * expected[3]).abs() <= 2. / 255.);
        }
    }
    native(&promoted);
    h.get_by_label("Red / Red").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Red / Red").type_text("0.5");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let edited = h.state().session.active().unwrap().doc.clone();
    assert_eq!(value(&edited, "rr-gain"), FilterValue::Number(ExactNumber::Float32Bits(0.5_f32.to_bits())));
    assert_eq!(edited.layers, source.layers);
    native(&edited);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, *promoted);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, source);
}
