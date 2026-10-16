//! Deferred real Layers row focus for the existing mixed document classes.
use crate::PhotocraftApp;
use egui::{Event, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use photocraft_engine::Session;
#[path = "../../engine/src/accounted_commands/programme_select_test_support.rs"]
mod f;
#[test]
#[ignore = "UNRUN/uncompiled real Layers panel primary row clicks select the second genuine mixed/masked F32/F64 root through registered replace-focus, preserve model/source/programmes/native and use that active raster for existing brush. Internal UI only, no visual/producer oracle."]
fn real_mixed_layer_rows_focus_the_actual_second_root() {
    for mask in [false, true] {
        for depth in 0..2 {
            let mut session = f::session(mask, depth);
            let (first, second) = f::roots(&session.active().unwrap().doc);
            session.execute("layer.select", serde_json::json!({"layer":first.0})).unwrap();
            session.tools.brush.pressure_size = false;
            let mut app = PhotocraftApp::new(session, Default::default());
            app.background_jobs = true;
            let before = app.session.active().unwrap().doc.clone();
            let witness = f::Witness::capture(before.accounted().unwrap(), &f::ctl());
            let raw = f::programmes::raw(&before, second);
            let revision = app.session.active().unwrap().revision;
            let past = app.session.active().unwrap().history.past_len();
            let source = app.session.active().unwrap().source.clone();
            let mut h = Harness::builder().with_size(vec2(560.0, 700.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        return;
                    }
                    crate::i18n::with_language(crate::i18n::Lang::EN, || {
                        app.ppp = ui.ctx().pixels_per_point();
                        super::layers(app, ui);
                    });
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, Default::default());
            h.run_steps(4);
            let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == second.0).unwrap();
            let at = row.name.unwrap().center();
            h.hover_at(at);
            h.step();
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                h.step();
            }
            h.run_steps(3);
            let a = h.state_mut();
            assert!(!a.ui.status_error, "{}", a.ui.status);
            let state = a.session.active().unwrap();
            assert!(before.shares_state(&state.doc));
            assert_eq!(state.active_layer, Some(second));
            assert_eq!(state.layer_anchor, Some(second));
            assert_eq!(state.revision, revision + 1);
            assert_eq!(state.history.past_len(), past);
            assert_eq!(state.source, source);
            witness.preserved(&state.doc, second, &f::ctl());
            assert_eq!(f::programmes::raw(&state.doc, second), raw);
            let mut patch = f::programmes::patch(&state.doc, second);
            patch.as_object_mut().unwrap().remove("layer");
            crate::jobs_ui::run(a, "paint.stroke", patch).unwrap();
            let after = a.session.active().unwrap().doc.clone();
            assert_ne!(f::programmes::raw(&after, second), raw);
            witness.preserved(&after, second, &f::ctl());
            assert_eq!(a.session.active().unwrap().source, source);
        }
    }
}
