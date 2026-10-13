//! Private pending widget RED; independent old-six packet only.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
fn original(model: &str, stage: &str) -> photocraft_doc::Document {
    use sha2::Digest;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/color-rotate-perceptual-evidence");
    let manifest_bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    let hash = |bytes: &[u8]| sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hash(&manifest_bytes), "8b338d140af0add90f7390e7fc2db4f0adef9dac24f45151ff50140dd66bb59c");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let name = format!("{model}.{stage}.xcf");
    let bytes = std::fs::read(root.join(&name)).unwrap();
    assert_eq!(hash(&bytes), manifest["files"].as_array().unwrap().iter().find(|a| a["file"] == name).unwrap()["sha256"]);
    photocraft_io::import("original.xcf", &bytes).unwrap().document
}
#[test]
#[ignore = "requires independently frozen old-six Color Rotate packet"]
fn color_rotate_destination_widgets_have_accessible_exact_angle_labels() {
    let doc = original("rgb", "source");
    let layer = doc.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, None).unwrap();
    let app = PhotocraftApp::new(session, Default::default());
    let mut h = Harness::builder().with_size(egui::vec2(460., 900.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    for label in ["Destination start angle", "Destination end angle"] {
        assert!(!h.get_by_label(label).accesskit_node().is_disabled());
    }
}

fn number(doc: &photocraft_doc::Document, name: &str) -> u32 {
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let FilterValue::Number(ExactNumber::Float32Bits(bits)) = node.arguments.iter().find(|a| a.name == name).unwrap().value else { panic!("F32 original") };
    bits
}
fn set_angle(h: &mut Harness<PhotocraftApp>, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label(label).type_text(text);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}
fn native(doc: &photocraft_doc::Document) {
    let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *doc);
    let mut bytes = std::io::Cursor::new(Vec::new());
    photocraft_io::save_native_stream(doc, &mut bytes, Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(photocraft_format::load_from_stream(bytes, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap(), *doc);
}
#[test]
#[ignore = "requires independently frozen old-six Color Rotate references"]
fn color_rotate_paired_widget_apply_keeps_source_and_native_history() {
    for model in ["rgb", "gray"] {
        let source = original(model, "source");
        let target = original(model, "edit");
        let disabled = original(model, "disabled");
        let layer = source.layers[0].id;
        let caption = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].source_display_name.clone().unwrap();
        let mut session = photocraft_engine::Session::new();
        session.add_document(source.clone(), None).unwrap();
        let app = PhotocraftApp::new(session, Default::default());
        let visual = std::env::var_os("PHOTOCRAFT_COLOR_ROTATE_UI_VISUALS").is_some();
        let mut builder = Harness::builder().with_size(egui::vec2(460., 450.));
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
        h.get_by_label("Destination start angle").click();
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert_eq!(*h.state().session.active().unwrap().doc, source);
        set_angle(&mut h, "Destination start angle", "210");
        assert_eq!(*h.state().session.active().unwrap().doc, source);
        assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
        set_angle(&mut h, "Destination end angle", "330");
        assert_eq!(*h.state().session.active().unwrap().doc, source);
        h.get_by_label("Apply").click();
        h.run_steps(2);
        let current = h.state().session.active().unwrap().doc.clone();
        assert_eq!(number(&current, "dest-from"), number(&target, "dest-from"));
        assert_eq!(number(&current, "dest-to"), number(&target, "dest-to"));
        assert_eq!(current.layers, source.layers);
        assert_eq!(current.xcf_state.as_ref().unwrap().provenance, source.xcf_state.as_ref().unwrap().provenance);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
        let render = |doc: &photocraft_doc::Document| photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
        for (a, b) in render(&current).px.iter().zip(render(&target).px.iter()) {
            assert!((a[3] - b[3]).abs() <= 1.0 / 65535.0);
            for c in 0..3 {
                assert!((a[c] * a[3] - b[c] * b[3]).abs() <= 2.0 / 255.0);
            }
        }
        native(&current);
        if visual {
            let directory = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_COLOR_ROTATE_UI_VISUALS").unwrap());
            std::fs::create_dir_all(&directory).unwrap();
            h.render().unwrap().save(directory.join(format!("color-rotate-{model}-destination.png"))).unwrap();
        }
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*h.state().session.active().unwrap().doc, source);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(*h.state().session.active().unwrap().doc, *current);
        h.get_by_label(&caption).click();
        h.run_steps(2);
        let current = h.state().session.active().unwrap().doc.clone();
        assert!(!current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        assert_eq!(current.layers, source.layers);
        assert_eq!(render(&current).px, render(&disabled).px);
        native(&current);
    }
}

#[test]
#[ignore = "requires independently frozen old-six Color Rotate references"]
fn color_rotate_locked_stale_and_hostile_drafts_cannot_apply() {
    let source = original("rgb", "source");
    let original_node = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
    assert!(color_rotate::valid_node(&original_node));
    for case in 0..7 {
        let mut node = original_node.clone();
        match case {
            0 => node.operation.schema_epoch.clear(),
            1 => node.operation.stored_version = "0:1".into(),
            2 => node.composition.as_mut().unwrap().stored_blend_space = 0,
            3 => node.arguments[1] = node.arguments[0].clone(),
            4 => {
                node.arguments.pop();
            }
            5 => node.arguments.iter_mut().find(|a| a.name == "dest-from").unwrap().value = FilterValue::Number(ExactNumber::Float64Bits(30f64.to_bits())),
            _ => node.arguments.iter_mut().find(|a| a.name == "src-from").unwrap().value = FilterValue::Number(ExactNumber::Float32Bits(17.25f32.to_bits())),
        }
        assert!(!color_rotate::valid_node(&node));
    }
    let layer = source.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(source.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, Default::default());
    let mut h = Harness::builder().with_size(egui::vec2(460., 700.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    h.state_mut().ui.xcf_color_rotate_draft.as_mut().unwrap().revision = u64::MAX;
    h.state_mut().ui.xcf_color_rotate_draft.as_mut().unwrap().from = 210.;
    h.state_mut().ui.xcf_color_rotate_draft.as_mut().unwrap().to = 330.;
    h.run_steps(2);
    assert_eq!(h.state().ui.xcf_color_rotate_draft.as_ref().unwrap().from.to_bits(), 30f32.to_bits());
    assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
    assert_eq!(*h.state().session.active().unwrap().doc, source);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    for text in ["30.00000001", "NaN", "inf", "360"] {
        set_angle(&mut h, "Destination start angle", text);
        assert_eq!(h.state().ui.xcf_color_rotate_draft.as_ref().unwrap().from.to_bits(), 30f32.to_bits());
        assert_eq!(*h.state().session.active().unwrap().doc, source);
    }
    let mut locked = source.clone();
    locked.layers[0].locks.pixels = true;
    let mut session = photocraft_engine::Session::new();
    session.add_document(locked.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, Default::default());
    let mut h = Harness::builder().with_size(egui::vec2(460., 700.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    assert!(h.get_by_label("Destination start angle").accesskit_node().is_disabled());
    assert!(h.get_by_label("Destination end angle").accesskit_node().is_disabled());
    assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
    assert_eq!(*h.state().session.active().unwrap().doc, locked);
}
