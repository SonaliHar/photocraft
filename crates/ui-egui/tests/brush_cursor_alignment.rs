//! A scaled native Hyprland cursor must share the painted dab's centre and diameter.
//! Drive screen-space input rather than the document-space automation shortcut.

use egui::{Event, Modifiers, PointerButton, vec2};
use photocraft_ui_egui::{PhotocraftApp, Services, Tool};
use serde_json::json;

#[test]
fn brush_outline_and_paint_share_the_pointer_at_160_percent() {
    let _gpu = photocraft_testkit::gpu_lock();
    let built = std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(vec2(1000.0, 700.0)).with_pixels_per_point(1.6).with_max_steps(64).wgpu().build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services { cursor_bitmap_ignores_viewport: true, ..Default::default() });
            if let Some(rs) = &cc.wgpu_render_state {
                app.set_wgpu(rs.clone());
            }
            app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
            app.run("tools.setBrush", json!({"brush": {"size": 64, "hardness": 1.0, "pressureSize": false}})).unwrap();
            app.run("prefs.set", json!({"values": {"cursors.painting": "fullSizeTip", "cursors.showCrosshairInBrushTip": true}})).unwrap();
            app.ui.tool = Tool::Brush;
            app.sync_views();
            app
        })
    });
    let Ok(mut h) = built else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    h.run_steps(6);
    h.state_mut().ui.views[0].zoom = 2.0;
    h.run_steps(2);
    let pointer = h.state().last_canvas_rect.center() + vec2(30.0, 20.0);
    h.event(Event::PointerMoved(pointer));
    h.run_steps(3);
    assert!(h.output().platform_output.cursor_image.is_none(), "the compositor must not get the oversized, offset bitmap");
    let circle = h
        .output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Circle(c) if c.center.distance(pointer) < 0.01 && (c.radius - 40.0).abs() < 0.01 => Some(*c),
            _ => None,
        })
        .expect("64 px brush at 200% zoom / 160% display scale: a 40 point circle at the pointer");
    let button = |pressed| Event::PointerButton { pos: pointer, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.event(button(true));
    h.run_steps(2);
    h.event(button(false));
    h.run_steps(3);
    let image = h.render().expect("render the painted dab and cursor");
    // At 200% zoom the physical diameter is 128 pixels. Measure the black dab's bounds,
    // excluding the cursor's small centre cross and its light outlined edge.
    let ppp = h.ctx.pixels_per_point();
    let centre = circle.center * ppp;
    let mut bounds = egui::Rect::NOTHING;
    for y in (centre.y - 72.0) as u32..(centre.y + 72.0) as u32 {
        for x in (centre.x - 72.0) as u32..(centre.x + 72.0) as u32 {
            let px = image.get_pixel(x, y).0;
            if px[0] < 12 && px[1] < 12 && px[2] < 12 {
                bounds.extend_with(egui::pos2(x as f32 + 0.5, y as f32 + 0.5));
            }
        }
    }
    assert!(bounds.center().distance(centre) < 2.0, "paint {bounds:?} must be centred at {centre:?}");
    assert!((bounds.width() - 128.0).abs() < 6.0 && (bounds.height() - 128.0).abs() < 6.0, "the paint fills the cursor diameter: {bounds:?}");
    if let Some(path) = std::env::var_os("PHOTOCRAFT_CURSOR_EVIDENCE") {
        image.save(path).expect("save cursor alignment evidence");
    }
}
