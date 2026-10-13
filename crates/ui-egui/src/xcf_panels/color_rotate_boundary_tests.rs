//! Exact paired destination widgets for the separately witnessed boundary packet.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
fn original(model: &str, arc: &str, stage: &str) -> photocraft_doc::Document {
    use sha2::Digest;
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"))
        .join("color-rotate-perceptual-boundary-evidence");
    let manifest_bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    let hash = |bytes: &[u8]| sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hash(&manifest_bytes), "72e93210100f51ce988929d0ba9010f817b7ca5c1e69cb4f66caaae0eeacc9ac");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let name = format!("{model}.{arc}-{stage}.xcf");
    let bytes = std::fs::read(root.join(&name)).unwrap();
    assert_eq!(hash(&bytes), manifest["files"].as_array().unwrap().iter().find(|a| a["file"] == name).unwrap()["sha256"]);
    photocraft_io::import("original.xcf", &bytes).unwrap().document
}
fn angle(h: &mut Harness<PhotocraftApp>, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label(label).type_text(text);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}
#[test]
#[ignore = "exact independent four Color Rotate boundary source/edit pairs"]
fn color_rotate_boundary_paired_widgets_match_four_native_references() {
    for model in ["rgb", "gray"] {
        for arc in ["arc-a", "arc-b"] {
            let source = original(model, arc, "source");
            let target = original(model, arc, "edit");
            let layer = source.layers[0].id;
            let mut session = photocraft_engine::Session::new();
            session.add_document(source.clone(), None).unwrap();
            let app = PhotocraftApp::new(session, Default::default());
            let visual = std::env::var_os("PHOTOCRAFT_COLOR_ROTATE_UI_VISUALS");
            let mut builder = Harness::builder().with_size(egui::vec2(460., 450.));
            if visual.is_some() {
                let mut setup = egui_kittest::wgpu::default_wgpu_setup();
                if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
                    create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
                }
                builder = builder.wgpu_setup(setup);
            }
            let mut h = builder.build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
            h.run_steps(2);
            assert!(!h.get_by_label("Destination start angle").accesskit_node().is_disabled());
            h.get_by_label("Destination start angle").click();
            h.run_steps(2);
            h.key_press(egui::Key::Escape);
            h.run_steps(2);
            assert_eq!(*h.state().session.active().unwrap().doc, source);
            angle(&mut h, "Destination start angle", "207.5");
            assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
            assert_eq!(*h.state().session.active().unwrap().doc, source);
            angle(&mut h, "Destination end angle", "337.25");
            h.get_by_label("Apply").click();
            h.run_steps(2);
            let current = h.state().session.active().unwrap().doc.clone();
            let nodes = |d: &photocraft_doc::Document| d.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.clone();
            assert_eq!(nodes(&current), nodes(&target));
            let mut retained_node = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
            retained_node.arguments = nodes(&target);
            assert_eq!(current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0], retained_node);
            assert_eq!(current.layers, source.layers);
            assert_eq!(current.xcf_state.as_ref().unwrap().provenance, source.xcf_state.as_ref().unwrap().provenance);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
            assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
            let render = |d: &photocraft_doc::Document| photocraft_compose::gimp_render::try_render(d, d.bounds(), &mut |_| Ok(())).unwrap();
            let (actual, reference) = (render(&current), render(&target));
            for (a, b) in actual.px.iter().zip(&reference.px) {
                assert!((a[3] - b[3]).abs() <= 1. / 65535.);
                for c in 0..3 {
                    assert!((a[c] * a[3] - b[c] * b[3]).abs() <= 2. / 255.);
                }
            }
            let mut native = std::io::Cursor::new(Vec::new());
            photocraft_io::save_native_stream(&current, &mut native, Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
            assert_eq!(
                photocraft_format::load_from_stream(native, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap(),
                *current
            );
            if let Some(directory) = &visual {
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                h.render().unwrap().save(directory.join(format!("color-rotate-{model}-{arc}-boundary.png"))).unwrap();
            }
            h.get_by_label("Destination end angle").click();
            h.run_steps(2);
            h.key_press(egui::Key::Escape);
            h.run_steps(2);
            assert_eq!(*h.state().session.active().unwrap().doc, *current);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(*h.state().session.active().unwrap().doc, source);
            h.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(*h.state().session.active().unwrap().doc, *current);
        }
    }
}

#[test]
#[ignore = "exact independent Color Rotate boundary source and locked/stale controls"]
fn color_rotate_boundary_stale_noop_and_unqualified_drafts_refuse() {
    let source = original("rgb", "arc-a", "source");
    let layer = source.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(source.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, Default::default());
    let mut h = Harness::builder().with_size(egui::vec2(460., 450.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
    h.state_mut().ui.xcf_color_rotate_draft.as_mut().unwrap().revision = u64::MAX;
    h.state_mut().ui.xcf_color_rotate_draft.as_mut().unwrap().from = 207.5;
    h.state_mut().ui.xcf_color_rotate_draft.as_mut().unwrap().to = 337.25;
    h.run_steps(2);
    assert_eq!(h.state().ui.xcf_color_rotate_draft.as_ref().unwrap().from.to_bits(), 33.125f32.to_bits());
    assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
    for text in ["33.125000001", "NaN", "inf", "30", "360"] {
        angle(&mut h, "Destination start angle", text);
        assert_eq!(h.state().ui.xcf_color_rotate_draft.as_ref().unwrap().from.to_bits(), 33.125f32.to_bits());
        assert_eq!(*h.state().session.active().unwrap().doc, source);
    }
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
    let mut locked = source;
    locked.layers[0].locks.pixels = true;
    let mut session = photocraft_engine::Session::new();
    session.add_document(locked.clone(), None).unwrap();
    let app = PhotocraftApp::new(session, Default::default());
    let mut h = Harness::builder().with_size(egui::vec2(460., 450.)).build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    for label in ["Destination start angle", "Destination end angle", "Apply"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
    }
    assert_eq!(*h.state().session.active().unwrap().doc, locked);
}

/// Offscreen view inspection only; it adds no source/evaluator qualification.
#[test]
#[ignore = "real Vulkan Color Rotate Properties theme/width inspection"]
fn color_rotate_boundary_visual_theme_width_matrix() {
    use crate::theme::ThemeKind;
    let directory = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_COLOR_ROTATE_MATRIX_VISUALS").expect("explicit temp visual output"));
    std::fs::create_dir_all(&directory).unwrap();
    for (theme, theme_name) in
        [(ThemeKind::Pro, "pro"), (ThemeKind::Studio, "studio"), (ThemeKind::StudioLight, "studio-light"), (ThemeKind::Classic, "classic")]
    {
        for width in [340u16, 520] {
            for (model, arc) in [("rgb", "arc-a"), ("gray", "arc-b")] {
                let source = original(model, arc, "source");
                let layer = source.layers[0].id;
                let mut session = photocraft_engine::Session::new();
                session.add_document(source.clone(), None).unwrap();
                let app = PhotocraftApp::new(session, Default::default());
                let mut setup = egui_kittest::wgpu::default_wgpu_setup();
                if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
                    create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
                }
                let mut h = Harness::builder()
                    .with_size(egui::vec2(f32::from(width), 360.))
                    .wgpu_setup(setup)
                    .build_ui_state(move |ui, app| filter_properties(app, ui, layer), app);
                PhotocraftApp::setup_context(&h.ctx, theme);
                h.run_steps(2);
                angle(&mut h, "Destination start angle", "207.5");
                angle(&mut h, "Destination end angle", "337.25");
                assert!(!h.get_by_label("Apply").accesskit_node().is_disabled());
                assert_eq!(*h.state().session.active().unwrap().doc, source);
                h.event(egui::Event::PointerGone);
                h.run_steps(2);
                // Capture the complete valid draft while Apply remains enabled.
                h.render().unwrap().save(directory.join(format!("color-rotate-{model}-{arc}-{theme_name}-{width}.png"))).unwrap();
            }
        }
    }
}
