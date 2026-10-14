//! The native shell learns document roles before immediate viewport creation.
use super::*;
use std::{cell::RefCell, rc::Rc};

struct ImmediateIntegration;
impl Drop for ImmediateIntegration {
    fn drop(&mut self) {
        egui::Context::set_immediate_viewport_renderer(|_, _| {});
    }
}

#[test]
fn document_roles_precede_creation_and_follow_close_reopen() {
    let roles = Rc::new(RefCell::new(Vec::<egui::ViewportId>::new()));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let native_roles = roles.clone();
    let native_calls = calls.clone();
    egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
        native_calls.borrow_mut().push((viewport.ids.this, native_roles.borrow().contains(&viewport.ids.this)));
        let size = viewport.builder.inner_size.unwrap_or(egui::vec2(800.0, 600.0));
        let mut input = egui::RawInput {
            viewport_id: viewport.ids.this,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            focused: false,
            ..Default::default()
        };
        input.viewports.entry(viewport.ids.this).or_default().parent = Some(viewport.ids.parent);
        let mut output = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        output.textures_delta.clear();
    });
    let _integration = ImmediateIntegration;
    let reported = roles.clone();
    let services = Services { document_viewports: Some(Box::new(move |ids| *reported.borrow_mut() = ids.to_vec())), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", serde_json::json!({"width":16,"height":16})).unwrap();
    let window = |id, document, primary, open| state::DocWindow { id, document, primary, open, view: Default::default(), position: None };
    app.ui.windows = vec![window(11, 0, true, true), window(12, 0, false, true), window(13, 0, false, false), window(14, usize::MAX, false, true)];
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let unrelated = egui::ViewportId::from_hash_of("unrelated-native-role");
    let frame = |app: &mut PhotocraftApp| {
        calls.borrow_mut().clear();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            canvas::extra_windows(app, ui.ctx(), false);
            ui.ctx().show_viewport_immediate(unrelated, egui::ViewportBuilder::default(), |_, _| {});
        });
        // This fixture observes registration, with no renderer consuming font uploads.
        output.textures_delta.clear();
    };
    let primary = document_windows::viewport(11);
    let additional = document_windows::viewport(12);
    frame(&mut app);
    assert_eq!(*roles.borrow(), [primary, additional]);
    assert_eq!(*calls.borrow(), [(primary, true), (additional, true), (unrelated, false)], "register the real document ids before native creation");
    app.ui.windows.iter_mut().find(|w| w.id == 11).unwrap().open = false;
    frame(&mut app);
    assert_eq!(*roles.borrow(), [additional]);
    assert_eq!(*calls.borrow(), [(additional, true), (unrelated, false)]);
    app.ui.windows.push(window(11, 0, true, true));
    frame(&mut app);
    assert_eq!(*roles.borrow(), [additional, primary]);
    assert_eq!(*calls.borrow(), [(additional, true), (primary, true), (unrelated, false)]);
    app.ui.windows.clear();
    frame(&mut app);
    assert!(roles.borrow().is_empty(), "clear document ownership roles when all views close");
    assert_eq!(*calls.borrow(), [(unrelated, false)], "an unrelated native viewport never acquires a document role");
}
