//! The GPU compositor is built on a worker thread when asked (`GpuCanvas::warm_compositor`): until it is ready a composite
//! stands aside instead of stopping the UI while the driver compiles the shaders, and the renderer stays usable meanwhile.
//! Skips when there is no GPU adapter.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui_wgpu::RenderState;
use photocraft_geom::Rect;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::gpu_canvas::{COMPOSITOR_WARMING, GpuCanvas};
use serde_json::json;

/// A render state from a real adapter (a kittest harness builds one), or `None`.
fn render_state() -> Option<(egui_kittest::Harness<'static, PhotocraftApp>, RenderState)> {
    let slot: Arc<Mutex<Option<RenderState>>> = Arc::default();
    let into = slot.clone();
    let built = std::panic::catch_unwind(move || {
        egui_kittest::Harness::builder().with_size(egui::vec2(320.0, 240.0)).wgpu().build_eframe(move |cc| {
            *into.lock().unwrap() = cc.wgpu_render_state.clone();
            PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
        })
    });
    let harness = match built {
        Ok(h) => h,
        Err(_) => {
            eprintln!("skipping: no GPU adapter");
            return None;
        }
    };
    let rs = slot.lock().unwrap().take()?;
    Some((harness, rs))
}

/// One test, not two, each scenario on a device of its own, one after the other: harnesses started side by side on one
/// adapter disturb each other, and a second canvas on a device that already has one reports a validation error.
#[test]
fn the_compositor_is_built_off_the_ui_thread() {
    {
        let Some((_harness, rs)) = render_state() else { return };
        a_composite_stands_aside_while_the_compositor_is_built_and_the_renderer_stays_usable(&rs);
    }
    {
        let Some((_harness, rs)) = render_state() else { return };
        a_draw_with_nothing_in_flight_still_builds_its_own(&rs);
    }
    {
        let Some((_harness, rs)) = render_state() else { return };
        with_background_work_the_first_draw_starts_the_build_and_does_not_wait(&rs);
    }
    let Some((_harness, rs)) = render_state() else { return };
    a_large_document_redraws_in_the_background_and_a_newer_redraw_replaces_an_older_one(&rs);
}

fn a_composite_stands_aside_while_the_compositor_is_built_and_the_renderer_stays_usable(rs: &RenderState) {
    let canvas = GpuCanvas::new(rs);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48, "background": "white"})).expect("new document");
    let doc = app.session.active().expect("document").doc.clone();
    let region = Rect::new(0, 0, 64, 48);
    assert!(!canvas.compositor_ready() && !canvas.compositor_warming(), "nothing is built before it is asked for");

    // Starting the build does not wait for it, and a second request while it runs starts nothing.
    let began = Instant::now();
    assert!(canvas.warm_compositor(), "a build was started");
    assert!(began.elapsed() < Duration::from_millis(500), "starting it must not wait for the driver: {:?}", began.elapsed());
    assert!(canvas.compositor_warming());
    assert!(!canvas.warm_compositor(), "one at a time");

    // The build is in flight (the driver takes seconds): a composite answers at once, and says why. On a driver that builds
    // it faster than this thread gets here, there is nothing in flight to stand aside for.
    if canvas.compositor_warming() {
        let began = Instant::now();
        let message = canvas.composite(&doc, region, false).expect_err("the compositor is not ready yet").0;
        assert_eq!(message, COMPOSITOR_WARMING);
        assert!(began.elapsed() < Duration::from_millis(500), "it must not wait for the build: {:?}", began.elapsed());
        // And the renderer is not locked while the driver compiles.
        let began = Instant::now();
        let _ = canvas.has(doc.id.0, [64, 48]);
        assert!(began.elapsed() < Duration::from_millis(500), "the renderer was locked for {:?}", began.elapsed());
    }

    let deadline = Instant::now() + Duration::from_secs(300);
    while canvas.compositor_warming() {
        assert!(Instant::now() < deadline, "the compositor never finished building");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(canvas.compositor_ready(), "the build finished and the canvas has the compositor");
    let composited = canvas.composite(&doc, region, false);
    assert!(composited.is_ok(), "ready: the GPU composites the document: {:?}", composited.err());
    assert!(!canvas.warm_compositor(), "nothing to build any more");
}

fn a_draw_with_nothing_in_flight_still_builds_its_own(rs: &RenderState) {
    let canvas = GpuCanvas::new(rs);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32, "background": "white"})).expect("new document");
    let doc = app.session.active().expect("document").doc.clone();
    // Without a warm-up the first composite builds the compositor itself, as it always did.
    let composited = canvas.composite(&doc, Rect::new(0, 0, 32, 32), false);
    assert!(composited.is_ok(), "{:?}", composited.err());
    assert!(canvas.compositor_ready());
    assert!(!canvas.warm_compositor(), "it exists: nothing to warm");
}

fn with_background_work_the_first_draw_starts_the_build_and_does_not_wait(rs: &RenderState) {
    let canvas = GpuCanvas::new(rs);
    canvas.set_background_work(true);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48, "background": "white"})).expect("new document");
    let doc = app.session.active().expect("document").doc.clone();
    let region = Rect::new(0, 0, 64, 48);
    // Nobody asked for a warm-up: the draw that needs the compositor starts it, and answers at once.
    let began = Instant::now();
    let message = canvas.composite(&doc, region, false).expect_err("the first draw has no compositor yet").0;
    assert_eq!(message, COMPOSITOR_WARMING);
    assert!(began.elapsed() < Duration::from_millis(500), "the first draw must not build it: {:?}", began.elapsed());
    assert!(canvas.compositor_warming() || canvas.compositor_ready(), "a build was started");
    let deadline = Instant::now() + Duration::from_secs(300);
    while canvas.compositor_warming() {
        assert!(Instant::now() < deadline, "the compositor never finished building");
        std::thread::sleep(Duration::from_millis(50));
    }
    let composited = canvas.composite(&doc, region, false);
    assert!(composited.is_ok(), "ready: the GPU composites the document: {:?}", composited.err());
}

/// A document of `width` x `height` filled with `color`, as the canvas gets it (the same id across calls on one `app`).
fn filled(app: &mut PhotocraftApp, color: &str) -> Arc<photocraft_doc::Document> {
    app.run("edit.fill", json!({"color": color})).expect("fill");
    app.session.active().expect("document").doc.clone()
}

fn centre_texel(canvas: &GpuCanvas, key: u64) -> [f32; 4] {
    let (_, size, texels) = canvas.read_texels(key).expect("the document has a texture");
    texels[(size[1] / 2 * size[0] + size[0] / 2) as usize]
}

fn wait_until_not_refreshing(canvas: &GpuCanvas, key: u64) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while canvas.background_refreshing(key) {
        assert!(Instant::now() < deadline, "the background redraw never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn a_large_document_redraws_in_the_background_and_a_newer_redraw_replaces_an_older_one(rs: &RenderState) {
    let canvas = GpuCanvas::new(rs);
    let woken = Arc::new(AtomicUsize::new(0));
    canvas.set_wake({
        let woken = woken.clone();
        move || {
            woken.fetch_add(1, Ordering::SeqCst);
        }
    });
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 2400, "height": 1200, "background": "white"})).expect("new document");
    let red = filled(&mut app, "#ff0000");
    let key = red.id.0;

    // Off by default: the caller's own `refresh` is the right call.
    assert!(canvas.refresh_in_background(key, &red, None).is_none(), "background work is off");
    canvas.set_background_work(true);
    // Not this canvas's document, or too small to be worth the hand-over: also `refresh`'s.
    assert!(canvas.refresh_in_background(key + 1, &red, None).is_none(), "another key");
    let mut small = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    small.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).expect("new document");
    let little = small.session.active().expect("document").doc.clone();
    assert!(canvas.refresh_in_background(little.id.0, &little, None).is_none(), "2 MP is the least worth it");

    // A large document: it returns at once, the worker fills the texture, and the UI is woken as it goes and at the end.
    let began = Instant::now();
    let started = canvas.refresh_in_background(key, &red, None).expect("started in the background");
    assert!(began.elapsed() < Duration::from_secs(2), "the calling thread did not do the redraw: {:?}", began.elapsed());
    assert_eq!(started.px, 2400 * 1200);
    wait_until_not_refreshing(&canvas, key);
    assert!(woken.load(Ordering::SeqCst) >= 1, "the UI was woken");
    let [r, g, b, a] = centre_texel(&canvas, key);
    assert!(r > 0.9 && g < 0.1 && b < 0.1 && a > 0.9, "the document is red on the canvas: {r} {g} {b} {a}");
    assert!(canvas.fault().is_none(), "no GPU error");

    // A newer redraw of the same document replaces an older one that has not finished: the newest is what stays.
    let blue = filled(&mut app, "#0000ff");
    let green = filled(&mut app, "#00ff00");
    assert!(canvas.refresh_in_background(key, &blue, None).is_some());
    assert!(canvas.refresh_in_background(key, &green, None).is_some());
    wait_until_not_refreshing(&canvas, key);
    let [r, g, b, _] = centre_texel(&canvas, key);
    assert!(g > 0.9 && r < 0.1 && b < 0.1, "the newer redraw won: {r} {g} {b}");
    assert!(canvas.fault().is_none(), "no GPU error from the restart");

    // A whole-document edit is a full redraw exactly where the CPU does those (large documents on a shared-memory GPU).
    let mut big = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    big.run("file.new", json!({"width": 5000, "height": 4000, "background": "white"})).expect("new document");
    let large = big.session.active().expect("document").doc.clone();
    let expected = photocraft_ui_egui::gpu_canvas::prefers_cpu_full_refresh(
        canvas.adapter_info().device_type,
        5000 * 4000,
        std::env::var("PHOTOCRAFT_FULL_REFRESH").ok().as_deref(),
    );
    assert_eq!(canvas.redraws_whole_document_on_cpu(&large), expected);
    assert!(!canvas.redraws_whole_document_on_cpu(&red), "2.9 MP is drawn in one go on any GPU");
}
