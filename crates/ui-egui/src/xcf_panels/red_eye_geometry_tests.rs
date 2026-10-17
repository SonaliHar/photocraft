//! Fresh six-control owner geometry witness; distinct from precision108.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
#[path = "../../../xcf/tests/common/red_eye_geometry_evidence.rs"]
mod evidence;
use evidence::hash;

fn activation_checkbox<'a>(h: &'a Harness<'_, PhotocraftApp>, caption: &'a str) -> egui_kittest::Node<'a> {
    let mut matches = h.query_all_by_label(caption).filter(|node| node.accesskit_node().role() == egui::accesskit::Role::CheckBox);
    let node = matches.next().expect("the actual filter activation checkbox");
    assert!(matches.next().is_none(), "activation checkbox must be unique");
    node
}
fn packet() -> std::path::PathBuf {
    evidence::selected(env!("CARGO_MANIFEST_DIR")).unwrap().packet
}
fn state(root: &std::path::Path, stem: &str) -> serde_json::Value {
    let witness = evidence::selected(env!("CARGO_MANIFEST_DIR")).unwrap();
    assert_eq!(witness.packet.canonicalize().unwrap(), root.canonicalize().unwrap());
    let state = witness.state(stem, "reopened").unwrap();
    witness.assert_implicit_profile(&state).unwrap();
    state
}

fn verify(root: &std::path::Path, stem: &str, doc: &photocraft_doc::Document, source: &photocraft_doc::Document) {
    use photocraft_cms::xcf_color::{Encoding, convert_encoding_f64, implicit_source_profile};
    let witness = state(root, stem);
    assert_eq!(doc.icc_profile, Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()));
    assert_eq!(source.icc_profile, Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()));
    let width = witness["owner"]["size"][0].as_i64().unwrap() as i32;
    let height = witness["owner"]["size"][1].as_i64().unwrap() as i32;
    let x = witness["owner"]["offset"][0].as_i64().unwrap() as i32;
    let y = witness["owner"]["offset"][1].as_i64().unwrap() as i32;
    let owner = photocraft_geom::Rect::new(x, y, x + width, y + height);
    // Current typed edits never bake the original full owner, including hidden RGB.
    let native = evidence::artifact(root, &witness["owner"]["native"]).unwrap();
    assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(owner).unwrap(), native);
    assert_eq!(doc.layers[0].surface(), source.layers[0].surface());
    assert_eq!(doc.bounds().width() as i64, witness["canvas"][0].as_i64().unwrap());
    assert_eq!(doc.bounds().height() as i64, witness["canvas"][1].as_i64().unwrap());
    let rendered = photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(())).unwrap();
    let descriptor = &witness["projection"]["formats"]["R'G'B'A double"];
    let projection = evidence::artifact(root, descriptor).unwrap();
    assert_eq!(hash(&projection), descriptor["sha256"]);
    let (pixels, tail) = projection.as_chunks::<32>();
    assert!(tail.is_empty());
    assert_eq!(pixels.len(), rendered.px.len());
    let profile = implicit_source_profile(doc.icc_profile.as_ref().unwrap(), photocraft_cms::ColorSpace::Rgb, Encoding::NonlinearProfile).unwrap();
    let mut color_max = 0f64;
    let mut alpha_max = 0f64;
    for (actual, expected) in rendered.px.iter().zip(pixels) {
        let expected: [f64; 4] = expected.as_chunks::<8>().0.iter().map(|v| f64::from_le_bytes(*v)).collect::<Vec<_>>().try_into().unwrap();
        let mut color = [0.; 3];
        convert_encoding_f64(&profile, Encoding::NonlinearProfile, Encoding::NonlinearProfile, &actual[..3], &mut color).unwrap();
        alpha_max = alpha_max.max((actual[3] - expected[3]).abs());
        for c in 0..3 {
            color_max = color_max.max((color[c] * actual[3] - expected[c] * expected[3]).abs());
        }
    }
    println!("{stem} admitted canvas DISPLAY max={color_max}, alpha={alpha_max}");
    assert!(color_max <= 2. / 255.);
    assert!(alpha_max <= 1. / 65535.);
    let reopened = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(doc, &Default::default()).unwrap()).unwrap();
    assert_eq!(reopened, *doc);
    let mut archive = std::io::Cursor::new(Vec::new());
    photocraft_io::save_native_stream(doc, &mut archive, Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(photocraft_format::load_from_stream(archive, &Default::default(), Default::default(), &photocraft_raster::Interrupt::NONE).unwrap(), *doc);
}

#[test]
#[ignore = "fresh V2 six actual owner references; optional real Vulkan Properties capture"]
fn fresh_red_eye_geometry_properties_typed_edit_native_history_and_canvas() {
    let root = packet();
    for geometry in ["negative-owner", "tile-crossing-owner"] {
        let source_path = root.join(format!("{geometry}.source.xcf"));
        let original_bytes = std::fs::read(&source_path).unwrap();
        let source = photocraft_io::import("red-eye-geometry.xcf", &original_bytes).unwrap().document;
        let layer = source.layers[0].id;
        let caption = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].source_display_name.clone().unwrap();
        verify(&root, &format!("{geometry}.source"), &source, &source);
        for control in ["edit", "disabled"] {
            let target =
                photocraft_io::import("red-eye-geometry.xcf", &std::fs::read(root.join(format!("{geometry}.{control}.xcf"))).unwrap()).unwrap().document;
            let mut session = photocraft_engine::Session::new();
            session.add_document(source.clone(), None).unwrap();
            let mut builder = Harness::builder().with_size(egui::vec2(520., 360.));
            let output = std::env::var_os("PHOTOCRAFT_RED_EYE_GEOMETRY_VISUALS").map(std::path::PathBuf::from);
            if output.is_some() {
                let mut setup = egui_kittest::wgpu::default_wgpu_setup();
                if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
                    create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
                }
                builder = builder.wgpu_setup(setup);
            }
            let mut h = builder.build_ui_state(move |ui, app| filter_properties(app, ui, layer), PhotocraftApp::new(session, Default::default()));
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
            h.run_steps(2);
            assert!(!h.get_by_label("Threshold").accesskit_node().is_disabled());
            h.get_by_label("Threshold").click();
            h.run_steps(2);
            h.key_press(egui::Key::Escape);
            h.run_steps(2);
            assert_eq!(*h.state().session.active().unwrap().doc, source);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
            if control == "edit" {
                h.get_by_label("Threshold").click();
                h.run_steps(2);
                h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
                h.get_by_label("Threshold").type_text("0.675");
                h.key_press(egui::Key::Enter);
                h.run_steps(2);
            } else {
                activation_checkbox(&h, &caption).click();
                h.run_steps(2);
            }
            let current = h.state().session.active().unwrap().doc.clone();
            let actual = &current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let expected = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            assert_eq!(actual.arguments, expected.arguments);
            assert_eq!(actual.enabled, expected.enabled);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), 1);
            verify(&root, &format!("{geometry}.{control}"), &current, &source);
            if let Some(output) = output {
                std::fs::create_dir_all(&output).unwrap();
                h.render().unwrap().save(output.join(format!("{geometry}-{control}-properties.png"))).unwrap();
            }
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(*h.state().session.active().unwrap().doc, source);
            h.state_mut().run("edit.redo", json!({})).unwrap();
            assert_eq!(*h.state().session.active().unwrap().doc, *current);
        }
        assert_eq!(std::fs::read(source_path).unwrap(), original_bytes);
    }
}
