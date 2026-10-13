use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn hash(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn evidence_document(stage: &str) -> photocraft_doc::Document {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/mono-mixer-signed-precision-evidence");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "e63618b1a99e38852cd35207e9120e2c6e3a2700433b2a7e281683650155e194");
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "observations.json").unwrap()["sha256"]);
    let observations: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let row = observations["records"].as_array().unwrap().iter().find(|row| row["id"] == format!("rgb.float_non_linear.{stage}")).unwrap();
    let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    photocraft_io::import("original.xcf", &bytes).unwrap().document
}

#[test]
#[ignore = "requires independent local Mono Mixer source and current Double authority"]
fn current_double_mono_controls_preserve_sub_float_bits_noop_and_hostile_guards() {
    let original = evidence_document("source");
    let reference = evidence_document("signed");
    let layer = original.layers[0].id;
    let id = original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let weights = [-0.50000000001_f64, 1.5, 0.25];
    let arguments: Vec<_> = ["red", "green", "blue"]
        .into_iter()
        .zip(weights)
        .map(|(name, value)| json!({"name":name,"value":{"Number":{"Float64Bits":format!("0x{:016x}",value.to_bits())}}}))
        .collect();
    session.execute("xcf.filter.set", json!({"filterId":id,"arguments":arguments})).unwrap();
    let seeded = (*session.active().unwrap().doc).clone();
    assert_eq!(seeded.layers, original.layers);
    let actual = photocraft_compose::gimp_render::try_render(&seeded, seeded.bounds(), &mut |_| Ok(())).unwrap();
    let expected = photocraft_compose::gimp_render::try_render(&reference, reference.bounds(), &mut |_| Ok(())).unwrap();
    assert_eq!(actual, expected, "different retained Double bits narrow to the independently observed working Float weights");
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut h = Harness::builder().with_size(egui::vec2(450., 750.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(3);
    for label in ["Red weight", "Green weight", "Blue weight"] {
        h.get_by_label(label);
    }
    assert_eq!(*h.state().session.active().unwrap().doc, seeded);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
    h.get_by_label("Red weight").click();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(*h.state().session.active().unwrap().doc, seeded);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
    h.get_by_label("Red weight").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Red weight").type_text("-0.5");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let edited = h.state().session.active().unwrap().doc.clone();
    let node = &edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.arguments.iter().find(|a| a.name == "red").unwrap().value, FilterValue::Number(ExactNumber::Float64Bits((-0.5_f64).to_bits())));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 2);
    assert_eq!(edited.layers, original.layers);
    assert_eq!(photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap(), expected);
    let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *edited);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, seeded);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(*h.state().session.active().unwrap().doc, *edited);
    for mutation in 0..3 {
        let mut invalid = seeded.clone();
        let node = &mut invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        match mutation {
            0 => node.arguments.push(node.arguments[0].clone()),
            1 => node.arguments[0].value = FilterValue::String("hostile".into()),
            _ => node.operation.schema_epoch = "unqualified".into(),
        }
        let mut session = photocraft_engine::Session::new();
        session.add_document(invalid.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut bad = Harness::builder().with_size(egui::vec2(450., 750.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
        PhotocraftApp::setup_context(&bad.ctx, crate::theme::ThemeKind::Studio);
        bad.run_steps(2);
        assert!(bad.query_all_by_role(egui::accesskit::Role::SpinButton).all(|node| node.accesskit_node().is_disabled()));
        assert_eq!(*bad.state().session.active().unwrap().doc, invalid);
        assert_eq!(bad.state().session.active().unwrap().history.past_len(), 0);
    }
}

#[test]
#[ignore = "requires independent local expanded Mono Mixer references"]
fn original_signed_mono_controls_preserve_exact_source_native_history_and_locks() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("mono-mixer-signed-precision-evidence");
    let manifest_bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&manifest_bytes), "e63618b1a99e38852cd35207e9120e2c6e3a2700433b2a7e281683650155e194");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let observation_bytes = std::fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&observation_bytes), manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "observations.json").unwrap()["sha256"]);
    let observations: serde_json::Value = serde_json::from_slice(&observation_bytes).unwrap();
    let rows = observations["records"].as_array().unwrap();
    let row = |stage: &str| rows.iter().find(|row| row["id"] == format!("rgb.float_non_linear.{stage}")).unwrap();
    let load = |stage: &str| {
        let bytes = std::fs::read(root.join(row(stage)["file"].as_str().unwrap())).unwrap();
        assert_eq!(hash(&bytes), row(stage)["sha256"]);
        photocraft_io::import("original.xcf", &bytes).unwrap().document
    };
    let original = load("source");
    let node = &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert_eq!(node.operation.name, "gegl:mono-mixer");
    assert_eq!(node.operation.stored_version, "0:0");
    assert_eq!(node.operation.schema_epoch, "xcf-22-stock-mono-mixer-signed-precision-v1");
    assert!(photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(node));
    let effect_label = node.source_display_name.as_ref().unwrap().clone();
    let layer = original.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(original.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let visuals = std::env::var_os("PHOTOCRAFT_MONO_UI_VISUALS").map(std::path::PathBuf::from);
    let builder = Harness::builder().with_size(egui::vec2(450., 750.));
    let builder = if visuals.is_some() {
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
    for label in ["Red weight", "Green weight", "Blue weight", "Preserve luminosity"] {
        assert!(!h.get_by_label(label).accesskit_node().is_disabled(), "qualified control: {label}");
    }
    assert_eq!(h.query_all_by_role(egui::accesskit::Role::SpinButton).count(), 3);
    assert_eq!(*h.state().session.active().unwrap().doc, original);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    let mut edits = 0;
    for stage in ["signed", "preserve"] {
        let target = load(stage);
        let target_node = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        for (name, label) in [("red", "Red weight"), ("green", "Green weight"), ("blue", "Blue weight")] {
            let argument = target_node.arguments.iter().find(|argument| argument.name == name).unwrap();
            let current = &h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            if current.arguments.iter().find(|a| a.name == name).unwrap().value == argument.value {
                continue;
            }
            let value = match argument.value {
                FilterValue::Number(ExactNumber::Float32Bits(bits)) => f64::from(f32::from_bits(bits)),
                FilterValue::Number(ExactNumber::Float64Bits(bits)) => f64::from_bits(bits),
                _ => panic!("independent exact weight"),
            };
            h.get_by_label(label).click();
            h.run_steps(2);
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.get_by_label(label).type_text(&value.to_string());
            h.key_press(egui::Key::Enter);
            h.run_steps(2);
            edits += 1;
            assert_eq!(h.state().session.active().unwrap().history.past_len(), edits);
        }
        let current = &h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let preserve = target_node.arguments.iter().find(|a| a.name == "preserve-luminosity").unwrap();
        if current.arguments.iter().find(|a| a.name == "preserve-luminosity").unwrap().value != preserve.value {
            h.get_by_label("Preserve luminosity").click();
            h.run_steps(2);
            edits += 1;
        }
        let current = h.state().session.active().unwrap().doc.clone();
        assert_eq!(current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, target_node.arguments);
        assert_eq!(current.layers, original.layers);
        assert_eq!(current.source_color_context, original.source_color_context);
        assert_eq!(current.xcf_state.as_ref().unwrap().opaque, original.xcf_state.as_ref().unwrap().opaque);
        assert_eq!(current.xcf_state.as_ref().unwrap().filter_masks, original.xcf_state.as_ref().unwrap().filter_masks);
        let actual = photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap();
        let expected = photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap();
        assert_eq!(actual, expected);
        if stage == "preserve"
            && let Some(directory) = &visuals
        {
            std::fs::create_dir_all(directory).unwrap();
            h.render().unwrap().save(directory.join("mono-signed-preserve.png")).unwrap();
        }
        let native = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *current);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(*h.state().session.active().unwrap().doc, *current);
    }
    assert_eq!(h.state().session.active().unwrap().history.past_len(), edits);
    // Disabled source retains the independent native input without running the mixer.
    h.get_by_label(&effect_label).click();
    h.run_steps(2);
    let disabled = load("disabled");
    assert_eq!(
        photocraft_compose::gimp_render::try_render(&h.state().session.active().unwrap().doc, original.bounds(), &mut |_| Ok(())).unwrap(),
        photocraft_compose::gimp_render::try_render(&disabled, disabled.bounds(), &mut |_| Ok(())).unwrap()
    );
    let state = h.state_mut().session.active_mut().unwrap();
    state.doc.try_resident_mut().unwrap().layers[0].locks.pixels = true;
    h.run_steps(2);
    for label in ["Red weight", "Green weight", "Blue weight", "Preserve luminosity"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
    }
    let bytes = std::fs::read(root.join(row("source")["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row("source")["sha256"]);
}
