//! UNRUN real upstream action producers over synthetic paid CURRENT native owners.
use super::*;
#[path = "observed_box_metadata_fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[test]
#[ignore = "UNRUN: observed Box three-role layer metadata via actual rename/opacity/eye-sweep actions, bg settings, opaque gestures/firstlabel/noopredo, source/native/profile/mask custody and writable continuation; not GIMP metadata evidence"]
fn observed_box_metadata_uses_existing_layer_controls_and_gestures() {
    for (sample, assigned) in [(photocraft_doc::SampleType::F32, false), (photocraft_doc::SampleType::F64, false), (photocraft_doc::SampleType::F32, true)] {
        for background in [false, true] {
            let owner = fixture::owner(sample, assigned);
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            crate::owned_native_open::adopt_bytes(&mut app, "observed.pcraft", fixture::archive(&owner), &fixture::ctl()).unwrap();
            app.background_jobs = background;
            let before = app.session.active().unwrap().doc.clone();
            let identity = app.session.active().unwrap().source.clone();
            let layer = &before.layers[0];
            let rename = crate::layer_row_ui::Rename {
                layer: layer.id.0,
                original: layer.name.clone(),
                text: "Observed CURRENT renamed".into(),
                focused: true,
                pass: 0,
            };
            let (command, params) = rename.commit_action().unwrap();
            assert!(app.run(&command, params).unwrap().is_null());
            let named = app.session.active().unwrap().doc.clone();
            let past = app.session.active().unwrap().history.past_len();
            let (command, params) = pct_action(&json!({"layer": named.layers[0].id.0}), "opacity", 37.5, Some(42));
            assert_eq!(params["coalesce"], "layer-opacity-slider:42");
            assert!(app.run(&command, params).unwrap().is_null());
            let (command, params) = pct_action(&json!({"layer": app.session.active().unwrap().doc.layers[0].id.0}), "opacity", -0.0, Some(42));
            app.run(&command, params).unwrap();
            app.run("layer.setProps", json!({"layer":layer.id.0,"visible":false,"name":"Same gesture hidden","coalesce":"layer-opacity-slider:42"})).unwrap();
            let changed = app.session.active().unwrap().doc.clone();
            assert!(!changed.layers[0].visible);
            assert_eq!(changed.layers[0].opacity.to_bits(), (-0f32).to_bits());
            assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
            assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Layer Properties"));
            fixture::preserved(&before, &changed);
            assert_eq!(app.session.active().unwrap().source, identity);
            app.run("edit.undo", Value::Null).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&named));
            let history = app.session.active().unwrap().history.clone();
            app.run("layer.setProps", json!({"layer":layer.id.0,"opacity":1.0,"coalesce":"layer-opacity-slider:42"})).unwrap();
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
            app.run("edit.redo", Value::Null).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&changed));
            // Existing eye-sweep consumer receives a genuine pointer-down frame.
            // Its opaque key is preserved, rather than stripped for this class.
            let ctx = egui::Context::default();
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("layer-eye-sweep"), EyeSweep { visible: true, key: 77, swept: vec![] }));
            let mut actions = Vec::new();
            let position = egui::pos2(10., 10.);
            let input = egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
                ],
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let response = ui.allocate_response(egui::vec2(20., 20.), egui::Sense::click_and_drag());
                    eye_sweep(ui.ctx(), &changed.layers[0], egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200., 30.)), &response, &mut actions);
                });
            });
            assert_eq!(actions.len(), 1);
            let (command, params) = actions.remove(0);
            assert_eq!(params["coalesce"], "layer-eye-sweep:77");
            app.run(&command, params).unwrap();
            assert!(app.session.active().unwrap().doc.layers[0].visible);
            assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Layer Visibility"));
            // Opacity remains raw negative zero: the current display is transparent.
            let snapshot = app.session.active().unwrap().doc.clone();
            let (image, fee) = crate::canvas::test_backed_snapshot_display_image(
                &snapshot,
                snapshot.size.width,
                snapshot.size.height,
                None,
                &photocraft_raster::Interrupt::NONE,
            )
            .unwrap();
            assert!(image.pixels.iter().all(|pixel| pixel.a() == 0));
            drop((image, fee));
            let state = app.session.active().unwrap().doc.clone();
            app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("Box metadata denied".into())));
            assert!(app.run("layer.setProps", json!({"name":"denied"})).unwrap_err().contains("Box metadata denied"));
            app.session.authorize = None;
            for (command, params) in [
                ("layer.setProps", json!({"blend":"multiply"})),
                ("xcf.items.visibility.set", json!({"items":[{"kind":"layer","id":layer.id.0.to_string()}],"visible":false})),
                ("layer.translate", json!({"target":"mask","dx":1})),
            ] {
                assert!(app.run(command, params).is_err());
            }
            assert!(app.session.active().unwrap().doc.shares_state(&state));
            assert!(!app.session.has_jobs());
            let titled = state.accounted().unwrap().try_with_document_name("Metadata continued title", &fixture::ctl()).unwrap();
            let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", fixture::archive(&titled), &fixture::ctl()).unwrap();
            assert_eq!(reopened.session.active().unwrap().doc.name, "Metadata continued title");
            fixture::preserved(&state, &reopened.session.active().unwrap().doc);
            reopened.run("layer.setProps", json!({"layer":layer.id.0,"opacity":0.5})).unwrap();
        }
    }
}
