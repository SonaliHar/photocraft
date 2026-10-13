#![cfg(not(target_arch = "wasm32"))]
//! Genuine full-app Document Properties evidence for authoritative DPI axes.
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::{
    PhotocraftApp,
    control::{ControlRequest, Outcome, handle},
};
use serde_json::json;

fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_text(shape, out)),
        _ => {}
    }
}

#[test]
#[ignore = "explicit real Vulkan full-app Document Properties capture; owned output required"]
fn document_properties_show_canonical_axes_and_legacy_default_without_mutation() {
    let output = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_RESOLUTION_PROPERTIES_VISUALS").unwrap());
    std::fs::create_dir_all(&output).unwrap();
    for canonical in [true, false] {
        for background_selected in [true, false] {
            let name =
                format!("{}-{}", if canonical { "canonical-axes" } else { "legacy-default" }, if background_selected { "background" } else { "no-selection" });
            let mut setup = egui_kittest::wgpu::default_wgpu_setup();
            if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
                create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
            }
            let mut h =
                egui_kittest::Harness::builder().with_size(egui::vec2(1440., 900.)).with_pixels_per_point(1.).wgpu_setup(setup).build_eframe(move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, photocraft_ui_egui::theme::ThemeKind::Pro);
                    if let Some(render) = &cc.wgpu_render_state {
                        println!("OFFSCREEN_ADAPTER={:?}", render.adapter.get_info());
                    }
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                    app.run("file.new", json!({"width":400,"height":300,"background":"white","name":"Physical resolution Properties witness"})).unwrap();
                    let state = app.session.active_mut().unwrap();
                    let doc = state.doc.try_resident_mut().unwrap();
                    // Deliberately stale legacy scalar: the canonical pair must win in the UI.
                    doc.resolution_dpi = 300.;
                    doc.physical_resolution =
                        canonical.then_some(photocraft_doc::PhysicalResolution { x_dpi: 144.25, y_dpi: 287.75, unit: photocraft_doc::PhysicalUnit::Inch });
                    if !background_selected {
                        state.active_layer = None;
                        state.selected_layers.clear();
                    }
                    app.perf.gpu_info.selected = "cpu".into();
                    app.perf.gpu_info.canvas = "cpu".into();
                    app
                });
            h.run_steps(4);
            assert_eq!(h.state().session.active().unwrap().active_layer.is_some(), background_selected);
            let before = h.state().session.active().unwrap().doc.clone();
            let history = h.state().session.active().unwrap().history.past_len();
            let revision = h.state().session.active().unwrap().revision;
            let ctx = h.ctx.clone();
            let (request, _) = ControlRequest::new(
                "ui.set",
                json!({"theme":"pro","zoom":1,"center":[200,150],"fit":false,"dockWidth":440,"dockTabs":{"properties":0},"dock":{"order":["properties","layers"],"heights":{"properties":650},"collapsed":[]},"panels":{"properties":true,"color":false,"history":false,"navigator":false,"character":false,"dock":true}}),
            );
            let Outcome::Done(reply) = handle(h.state_mut(), &ctx, &request) else { panic!("ui.set deferred") };
            assert_eq!(reply["ok"], true);
            h.run_steps(6);
            // Existing UI formatting rounds to one decimal; canonical metadata stays exact.
            let value = if canonical { "144.3 × 287.8" } else { "300" };
            let expected = format!("Resolution: {value} pixels/inch");
            let label_rect = h.get_by_label(&expected).rect();
            assert!(label_rect.is_positive());
            let mut painted = Vec::new();
            for shape in &h.output().shapes {
                collect_text(&shape.shape, &mut painted);
            }
            assert!(painted.iter().any(|text| text == &expected), "{painted:?}");
            assert_eq!(*h.state().session.active().unwrap().doc, *before);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
            assert_eq!(h.state().session.active().unwrap().revision, revision);
            assert_eq!(h.state().session.active().unwrap().active_layer.is_some(), background_selected);
            assert_eq!(h.state().session.active().unwrap().doc.resolution_dpi, 300.);
            if canonical {
                let physical = h.state().session.active().unwrap().doc.physical_resolution.as_ref().unwrap();
                assert_eq!(physical.x_dpi, 144.25);
                assert_eq!(physical.y_dpi, 287.75);
            }
            h.render().unwrap().save(output.join(format!("{name}.png"))).unwrap();
            std::fs::write(output.join(format!("{name}.json")), serde_json::to_vec_pretty(&json!({"expectedAccessibleAndPainted":expected,"canonical":canonical,"backgroundSelected":background_selected,"legacyScalar":300,"history":history,"revision":revision,"labelRect":[label_rect.min.x,label_rect.min.y,label_rect.max.x,label_rect.max.y],"documentUnchanged":true})).unwrap()).unwrap();
        }
    }
}
