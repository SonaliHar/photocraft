#![cfg(not(target_arch = "wasm32"))]
//! Actual app Properties/canvas snapshots from the fresh owner geometry packet.
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_ui_egui::{
    PhotocraftApp,
    control::{ControlRequest, Outcome, handle},
};
use serde_json::json;
#[path = "../../xcf/tests/common/red_eye_geometry_evidence.rs"]
mod evidence;

#[test]
#[ignore = "explicit Vulkan full-app geometry witness; requires packet and owned PNG output"]
fn fresh_red_eye_owner_canvas_and_properties_are_visible() {
    let witness = evidence::selected(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = &witness.root;
    let output = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_RED_EYE_GEOMETRY_VISUALS").unwrap());
    std::fs::create_dir_all(&output).unwrap();
    for theme in ["studio", "pro"] {
        for geometry in ["negative-owner", "tile-crossing-owner"] {
            for control in ["source", "edit"] {
                let name = format!("{geometry}.{control}");
                let path = format!("packet/{name}.xcf");
                let row = witness.record(&name).unwrap();
                let bytes = evidence::artifact(&witness.packet, &row["saved"]).unwrap();
                let imported = photocraft_io::import("red-eye-geometry.xcf", &bytes).unwrap();
                assert_eq!(imported.document.icc_profile, Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()));
                let layer = imported.document.layers[0].id;
                let bounds = imported.document.bounds();
                let zoom = if geometry == "negative-owner" { 10. } else { 3. };
                let mut setup = egui_kittest::wgpu::default_wgpu_setup();
                if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
                    create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
                }
                let mut h =
                    egui_kittest::Harness::builder().with_size(egui::vec2(1440., 900.)).with_pixels_per_point(1.).wgpu_setup(setup).build_eframe(move |cc| {
                        PhotocraftApp::setup_context(&cc.egui_ctx, photocraft_ui_egui::theme::ThemeKind::from_name(theme).unwrap());
                        if let Some(render) = &cc.wgpu_render_state {
                            println!("OFFSCREEN_ADAPTER={:?}", render.adapter.get_info());
                        }
                        let mut session = photocraft_engine::Session::new();
                        session.add_document(imported.document, None).unwrap();
                        let mut app = PhotocraftApp::new(session, Default::default());
                        app.run("layer.select", json!({"layer":layer.0})).unwrap();
                        app.perf.gpu_info.selected = "cpu".into();
                        app.perf.gpu_info.canvas = "cpu".into();
                        app
                    });
                h.run_steps(4);
                let (request, _) = ControlRequest::new(
                    "ui.set",
                    json!({"theme":theme,"zoom":zoom,"center":[bounds.width() as f32/2.,bounds.height() as f32/2.],"fit":false,"dockWidth":440,"dockTabs":{"properties":0},"dock":{"order":["properties","layers"],"heights":{"properties":650},"collapsed":[]},"panels":{"properties":true,"color":false,"history":false,"navigator":false,"character":false,"dock":true}}),
                );
                let ctx = h.ctx.clone();
                let Outcome::Done(reply) = handle(h.state_mut(), &ctx, &request) else { panic!("ui.set deferred") };
                assert_eq!(reply["ok"], true);
                h.run_steps(6);
                h.get_by_label("Threshold").focus();
                h.run_steps(2);
                h.get_by_label("Threshold").scroll_to_me();
                h.run_steps(4);
                assert!(!h.get_by_label("Threshold").accesskit_node().is_disabled());
                h.render().unwrap().save(output.join(format!("{theme}-{name}-full-app.png"))).unwrap();
                let (request, _) = ControlRequest::new("ui.inspect", json!({}));
                let Outcome::Done(inspect) = handle(h.state_mut(), &ctx, &request) else { panic!("ui.inspect deferred") };
                assert_eq!(inspect["ok"], true);
                std::fs::write(output.join(format!("{theme}-{name}-full-app.json")), serde_json::to_vec_pretty(&inspect).unwrap()).unwrap();
                if theme == "studio" && control == "source" {
                    let before = h.state().session.active().unwrap().doc.clone();
                    let history = h.state().session.active().unwrap().history.past_len();
                    h.get_by_label("Threshold").focus();
                    h.run_steps(2);
                    assert!(h.get_by_label("Threshold").is_focused());
                    h.key_press(egui::Key::Escape);
                    h.run_steps(2);
                    assert_eq!(*h.state().session.active().unwrap().doc, *before);
                    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
                    h.get_by_label("Threshold").focus();
                    h.run_steps(2);
                    assert!(h.get_by_label("Threshold").is_focused());
                    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
                    h.get_by_label("Threshold").type_text("0.675");
                    h.run_steps(2);
                    assert_eq!(*h.state().session.active().unwrap().doc, *before);
                    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
                    h.key_press(egui::Key::Enter);
                    h.run_steps(2);
                    let edited = h.state().session.active().unwrap().doc.clone();
                    let target = photocraft_io::import(
                        "red-eye-geometry.xcf",
                        &evidence::artifact(&witness.packet, &witness.record(&format!("{geometry}.edit")).unwrap()["saved"]).unwrap(),
                    )
                    .unwrap()
                    .document;
                    assert_eq!(
                        edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments,
                        target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments
                    );
                    assert_eq!(edited.layers[0].surface(), before.layers[0].surface());
                    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
                    h.render().unwrap().save(output.join(format!("studio-{geometry}-widget-edit.png"))).unwrap();
                    h.state_mut().run("edit.undo", json!({})).unwrap();
                    assert_eq!(*h.state().session.active().unwrap().doc, *before);
                    h.state_mut().run("edit.redo", json!({})).unwrap();
                    assert_eq!(*h.state().session.active().unwrap().doc, *edited);
                    h.run_steps(2);
                    let caption = edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].source_display_name.clone().unwrap();
                    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, &caption).scroll_to_me();
                    h.run_steps(4);
                    let activation = h.get_by_role_and_label(egui::accesskit::Role::CheckBox, &caption);
                    assert!(!activation.accesskit_node().is_disabled());
                    let viewport = h.ctx.input(|input| input.viewport_rect());
                    let checkbox = activation.rect();
                    assert!(
                        viewport.contains(checkbox.min) && viewport.contains(checkbox.max),
                        "activation checkbox must be visible: {checkbox:?} in {viewport:?}"
                    );
                    activation.click();
                    h.run_steps(2);
                    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
                    let disabled = h.state().session.active().unwrap().doc.clone();
                    assert!(!disabled.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
                    assert_eq!(disabled.layers[0].surface(), before.layers[0].surface());
                    h.render().unwrap().save(output.join(format!("studio-{geometry}-widget-disabled.png"))).unwrap();
                    h.state_mut().run("edit.undo", json!({})).unwrap();
                    assert_eq!(*h.state().session.active().unwrap().doc, *edited);
                    assert_eq!(
                        photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&disabled, &Default::default()).unwrap()).unwrap(),
                        *disabled
                    );
                }
                assert_eq!(std::fs::read(root.join(path)).unwrap(), bytes);
            }
        }
    }
}
