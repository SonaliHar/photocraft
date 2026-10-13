#![cfg(not(target_arch = "wasm32"))]
//! Full-app headless evidence for physical grid drawing and marquee snapping.
use photocraft_ui_egui::{
    PhotocraftApp,
    canvas::ViewXform,
    control::{ControlRequest, Outcome, handle},
};
use serde_json::{Value, json};

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (request, _) = ControlRequest::new(method, params);
    let Outcome::Done(reply) = handle(app, ctx, &request) else { panic!("unexpected deferred {method}") };
    assert_eq!(reply["ok"], true, "{method}: {reply}");
    reply
}

#[test]
#[ignore = "explicit full-app Vulkan offscreen visual verification; PHOTOCRAFT_GRID_VISUALS required"]
fn physical_grid_and_control_marquee_align_on_both_axes() {
    let directory = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_GRID_VISUALS").expect("owned visual output directory"));
    std::fs::create_dir_all(&directory).unwrap();
    let mut setup = egui_kittest::wgpu::default_wgpu_setup();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
    }
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 760.0)).with_pixels_per_point(1.0).wgpu_setup(setup).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, photocraft_ui_egui::theme::ThemeKind::Studio);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":400,"height":300,"background":"white","name":"Physical grid 144 × 288 DPI"})).unwrap();
        app.session
            .active_mut()
            .unwrap()
            .doc
            .try_resident_mut()
            .unwrap()
            .set_physical_resolution(photocraft_doc::PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: photocraft_doc::PhysicalUnit::Inch })
            .unwrap();
        if let Some(render) = &cc.wgpu_render_state {
            println!("OFFSCREEN_ADAPTER={:?}", render.adapter.get_info());
        }
        // Real app UI on Vulkan, CPU canvas as the existing --safe-gpu path.
        app.perf.gpu_info.selected = "cpu".into();
        app.perf.gpu_info.canvas = "cpu".into();
        app.ui.extras.grid = true;
        app.ui.extras.snap = true;
        app.ui.view.extras = true;
        app.ui.view.snap_to.guides = false;
        app.ui.view.snap_to.layers = false;
        app.ui.view.snap_to.document_bounds = false;
        app
    });
    h.run_steps(4);
    let ctx = h.ctx.clone();
    call(
        h.state_mut(),
        &ctx,
        "engine.execute",
        json!({"command":"prefs.set","params":{"values":{
            "guidesGridAndSlices.gridlineEvery":1,"guidesGridAndSlices.gridUnit":"inches",
            "guidesGridAndSlices.subdivisions":4,"guidesGridAndSlices.gridColor":"#ff0000"
        }}}),
    );
    call(
        h.state_mut(),
        &ctx,
        "ui.set",
        json!({"tool":"RectMarquee","zoom":1,"center":[200,150],"fit":false,"theme":"studio","panels":{"properties":false,"color":false}}),
    );
    h.run_steps(4);
    let before = h.render().unwrap();
    before.save(directory.join("physical-grid-before.png")).unwrap();
    call(
        h.state_mut(),
        &ctx,
        "ui.pointer",
        json!({"tool":"RectMarquee","events":[
            {"kind":"down","x":106,"y":34},{"kind":"move","x":214,"y":142},{"kind":"up","x":214,"y":142}
        ]}),
    );
    h.run_steps(4);
    let bounds = h.state().session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
    assert_eq!((bounds.x0, bounds.y0, bounds.x1, bounds.y1), (108, 34, 216, 144));
    assert_eq!(h.state().current_zoom(), 1.0);
    let xf = ViewXform::active(h.state()).unwrap();
    let image = h.render().unwrap();
    image.save(directory.join("physical-grid-marquee.png")).unwrap();
    let red_strength = |x: f32, y: f32| {
        let p = xf.to_screen(x, y);
        let mut maximum = 0i32;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let sx = p.x.round() as i32 + dx;
                let sy = p.y.round() as i32 + dy;
                assert!(sx >= 0 && sy >= 0 && sx < image.width() as i32 && sy < image.height() as i32);
                let [r, g, b, _] = image.get_pixel(sx as u32, sy as u32).0;
                maximum = maximum.max(i32::from(r) - (i32::from(g) + i32::from(b)) / 2);
            }
        }
        maximum
    };
    let probes = [
        ((36., 20.), red_strength(36., 20.)),
        ((72., 20.), red_strength(72., 20.)),
        ((20., 72.), red_strength(20., 72.)),
        ((20., 144.), red_strength(20., 144.)),
        ((20., 36.), red_strength(20., 36.)),
    ];
    for (_, strength) in probes.iter().take(4) {
        assert!(*strength > 40, "visible grid probe missing: {probes:?}");
    }
    assert!(probes[4].1 < 10, "phantom36pxY line: {probes:?}");
    let report = json!({"documentPixels":[400,300],"dpi":[144,288],"inchMajorSteps":[144,288],"subdivisionSteps":[36,72],
        "input":[106,34,214,142],"selection":[bounds.x0,bounds.y0,bounds.x1,bounds.y1],"zoom":xf.zoom,
        "canvasPoints":[xf.rect.min.x,xf.rect.min.y,xf.rect.max.x,xf.rect.max.y],"redGridProbes":probes,
        "mode":"real Vulkan UI, safe-gpu CPU canvas; no native window","controlInspect":call(h.state_mut(),&ctx,"ui.inspect",json!({}))});
    std::fs::write(directory.join("physical-grid-proof.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("GRID_VISUAL_PROOF={report}");
}
