use super::*;

fn points() -> [[f64; 2]; 4] {
    [[60.0, 45.0], [230.0, 25.0], [250.0, 195.0], [40.0, 220.0]]
}
fn session() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width":320,"height":240,"depth":8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("fixture", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 320, 240), &[0.2, 0.5, 0.8, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    begin(&mut app, &egui::Context::default()).unwrap();
    control(&mut app, &json!({"guides":[{"id":1,"corners":points()}]})).unwrap();
    app
}
fn shift() -> egui::Modifiers {
    egui::Modifiers { shift: true, ..Default::default() }
}
fn down(app: &mut PhotocraftApp, p: [f64; 2], mods: egui::Modifiers) {
    pointer(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, mods);
}
fn moved(app: &mut PhotocraftApp, p: [f64; 2], mods: egui::Modifiers) {
    pointer(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, mods);
}
fn up(app: &mut PhotocraftApp, p: [f64; 2], mods: egui::Modifiers) {
    pointer(app, ToolEvent::Up { x: p[0], y: p[1] }, mods);
}
fn corners(app: &PhotocraftApp) -> [[f64; 2]; 4] {
    app.distort.global_perspective.as_ref().unwrap().guides[0].corners
}
fn on_axis(p: [f64; 2], anchor: [f64; 2], vector: [f64; 2]) {
    let d = [p[0] - anchor[0], p[1] - anchor[1]];
    assert!((d[0] * vector[1] - d[1] * vector[0]).abs() < 1e-8, "{p:?} is not on {anchor:?} + t {vector:?}");
}
fn close(p: [f64; 2], q: [f64; 2]) {
    assert!((p[0] - q[0]).abs() + (p[1] - q[1]).abs() < 1e-8, "{p:?} != {q:?}");
}

#[test]
fn shift_projects_corner_along_perspective_top_edge() {
    let mut app = session();
    down(&mut app, points()[0], shift());
    moved(&mut app, [100.0, 47.0], shift());
    let q = corners(&app);
    on_axis(q[0], points()[0], [170.0, -20.0]);
    assert!(q[0][1] < 45.0 && q[0][0] > 90.0);
    assert_eq!(&q[1..], &points()[1..]);
    let descriptor = app.distort.global_perspective.as_ref().unwrap().describe();
    assert_eq!(descriptor["localAxisConstraint"]["space"], "guide");
    assert_eq!(descriptor["localAxisConstraint"]["frozen"], true);
    up(&mut app, [100.0, 47.0], shift());
    close(corners(&app)[0], q[0]);
    assert!(app.distort.global_perspective.as_ref().unwrap().active_local_axis().is_none());
}
#[test]
fn shift_projects_corner_along_perspective_side_edge() {
    let mut app = session();
    down(&mut app, points()[0], egui::Modifiers::NONE);
    moved(&mut app, [57.0, 85.0], shift());
    let q = corners(&app);
    on_axis(q[0], points()[0], [-20.0, 175.0]);
    assert!(q[0][0] < 57.0 && q[0][1] > 80.0);
    assert_eq!(&q[1..], &points()[1..]);
}
#[test]
fn shift_choice_does_not_follow_diagonal_mouse_noise() {
    let mut app = session();
    down(&mut app, points()[0], shift());
    moved(&mut app, [100.0, 47.0], shift());
    let axis = app.distort.global_perspective.as_ref().unwrap().active_local_axis();
    moved(&mut app, [100.0, 110.0], shift());
    assert_eq!(app.distort.global_perspective.as_ref().unwrap().active_local_axis(), axis);
    on_axis(corners(&app)[0], points()[0], [170.0, -20.0]);
}
#[test]
fn shift_released_returns_free_and_repress_selects_a_new_axis() {
    let mut app = session();
    down(&mut app, points()[0], shift());
    moved(&mut app, [100.0, 47.0], shift());
    moved(&mut app, [103.0, 85.0], egui::Modifiers::NONE);
    assert_eq!(corners(&app)[0], [103.0, 85.0]);
    assert!(app.distort.global_perspective.as_ref().unwrap().active_local_axis().is_none());
    moved(&mut app, [60.0, 95.0], shift());
    on_axis(corners(&app)[0], points()[0], [-20.0, 175.0]);
}
#[test]
fn ctrl_has_priority_over_shift_and_can_switch_back() {
    let mut app = session();
    let both = egui::Modifiers { ctrl: true, shift: true, ..Default::default() };
    down(&mut app, points()[0], both);
    moved(&mut app, [110.0, 53.0], both);
    assert_eq!(corners(&app)[0], [110.0, 45.0]);
    assert!(app.distort.global_perspective.as_ref().unwrap().active_local_axis().is_none());
    moved(&mut app, [60.0, 95.0], shift());
    on_axis(corners(&app)[0], points()[0], [-20.0, 175.0]);
    moved(&mut app, [110.0, 53.0], both);
    assert_eq!(corners(&app)[0], [110.0, 45.0]);
}
#[test]
fn late_shift_uses_original_guide_not_modified_edges() {
    let mut app = session();
    down(&mut app, points()[0], egui::Modifiers::NONE);
    moved(&mut app, [95.0, 100.0], egui::Modifiers::NONE);
    assert_eq!(corners(&app)[0], [95.0, 100.0]);
    moved(&mut app, [120.0, 55.0], shift());
    on_axis(corners(&app)[0], points()[0], [170.0, -20.0]);
}
#[test]
fn shift_corner_moves_outside_photo_without_clamping() {
    let mut app = session();
    down(&mut app, points()[0], shift());
    moved(&mut app, [-40.0, 59.0], shift());
    up(&mut app, [-40.0, 59.0], shift());
    assert!(corners(&app)[0][0] < 0.0);
    on_axis(corners(&app)[0], points()[0], [170.0, -20.0]);
    assert!(validate_guides(&app.distort.global_perspective.as_ref().unwrap().guides).is_ok());
}
#[test]
fn shift_does_not_bypass_invalid_quad_rejection() {
    let mut app = session();
    let old = corners(&app);
    down(&mut app, old[0], shift());
    moved(&mut app, [400.0, 5.0], shift());
    assert_eq!(corners(&app), old);
    up(&mut app, [400.0, 5.0], shift());
    assert_eq!(corners(&app), old);
}
#[test]
fn shift_final_release_sample_is_constrained_even_without_move() {
    let mut app = session();
    down(&mut app, points()[0], shift());
    up(&mut app, [105.0, 50.0], shift());
    on_axis(corners(&app)[0], points()[0], [170.0, -20.0]);
    assert_ne!(corners(&app)[0], points()[0]);
}
#[test]
fn shift_keeps_corner_grab_offset() {
    let mut app = session();
    down(&mut app, [62.0, 46.0], shift());
    moved(&mut app, [102.0, 42.0], shift());
    let raw = [40.0, -4.0];
    let dot = (raw[0] * 170.0 + raw[1] * -20.0) / (170.0 * 170.0 + 20.0 * 20.0);
    close(corners(&app)[0], [60.0 + dot * 170.0, 45.0 - dot * 20.0]);
}
#[test]
fn replacing_guides_cancels_the_frozen_shift_drag() {
    let mut app = session();
    down(&mut app, points()[0], shift());
    moved(&mut app, [100.0, 47.0], shift());
    control(&mut app, &json!({"guides":[{"id":1,"corners":points()}]})).unwrap();
    up(&mut app, [130.0, 50.0], shift());
    assert_eq!(corners(&app), points());
    assert!(app.distort.global_perspective.as_ref().unwrap().active_local_axis().is_none());
}
#[test]
fn shift_does_not_enable_local_warp_in_result_mode() {
    let mut app = session();
    let before = control(&mut app, &json!({"solve":true})).unwrap();
    let g = &before["solution"]["guides"][0]["transformedCorners"][0];
    let p = [g[0].as_f64().unwrap(), g[1].as_f64().unwrap()];
    down(&mut app, p, shift());
    moved(&mut app, [p[0] + 25.0, p[1] + 5.0], shift());
    up(&mut app, [p[0] + 25.0, p[1] + 5.0], shift());
    let after = app.distort.global_perspective.as_ref().unwrap().describe();
    assert_eq!(before["guides"], after["guides"]);
    assert_eq!(before["solution"], after["solution"]);
}
#[test]
fn shift_inside_a_guide_still_creates_a_new_guide() {
    let mut app = session();
    down(&mut app, [100.0, 90.0], shift());
    moved(&mut app, [140.0, 140.0], shift());
    up(&mut app, [140.0, 140.0], shift());
    assert_eq!(app.distort.global_perspective.as_ref().unwrap().guides.len(), 2);
    assert_eq!(corners(&app), points());
}
