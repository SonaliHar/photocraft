//! UNRUN CURRENT/native Move consumers; no live resident drag/source oracle.
use super::*;
#[path = "observed_box_move_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use serde_json::{Value, json};
fn preview(snapshot: &photocraft_engine::DocumentSnapshot) {
    let expected = display_image(None, &fixture::reference(snapshot));
    let (actual, fee) = backed_snapshot_display_image(snapshot, snapshot.size.width, snapshot.size.height, None, &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(actual.pixels.len(), expected.pixels.len());
    for (a, b) in actual.pixels.iter().zip(&expected.pixels) {
        for (a, b) in a.to_array().into_iter().zip(b.to_array()) {
            assert!((i16::from(a) - i16::from(b)).abs() <= 1);
        }
    }
    drop((actual, fee));
}
#[test]
#[ignore = "UNRUN: actual Move release registered action on observed Box three native CURRENT roles/bgsettings, unaligned full-kernel reference, mixedmetadata gesture/firstlabel/noopredo, native/mask/profile custody and continued access; no live preview or GIMP translation claim"]
fn observed_box_move_release_and_metadata_gesture_continue_native() {
    for (sample, assigned) in [(photocraft_doc::SampleType::F32, false), (photocraft_doc::SampleType::F64, false), (photocraft_doc::SampleType::F32, true)] {
        for background in [false, true] {
            let owner = fixture::owner(sample, assigned, false);
            let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            crate::owned_native_open::adopt_bytes(&mut app, "move.pcraft", fixture::archive(&owner), &fixture::ctl()).unwrap();
            app.background_jobs = background;
            let before = app.session.active().unwrap().doc.clone();
            let source = app.session.active().unwrap().source.clone();
            let layer = before.layers[0].id;
            let past = app.session.active().unwrap().history.past_len();
            assert!(app.move_preview.is_none());
            // Existing release handler sends upstream float deltas; the shared
            // command rounds them. Accounted resident preview intentionally absent.
            crate::move_ui::finish(&mut app, 1.6, -1.5);
            let moved = app.session.active().unwrap().doc.clone();
            fixture::exact(&before, &moved, 2, -2);
            assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
            assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Move"));
            assert!(!app.session.has_jobs());
            preview(&moved);
            app.run("edit.undo", Value::Null).unwrap();
            let history = app.session.active().unwrap().history.clone();
            assert!(app.run("layer.translate", json!({"layer":u64::MAX,"dx":0.2,"dy":-0.2})).unwrap().is_null());
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
            app.run("edit.redo", Value::Null).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&moved));
            let past = app.session.active().unwrap().history.past_len();
            app.run("layer.setProps", json!({"layer":layer.0,"name":"Moved CURRENT Box","opacity":0.375,"coalesce":"Box\u{0}mixed"})).unwrap();
            assert!(app.run("layer.translate", json!({"layer":"Moved CURRENT Box","dx":-3,"dy":1,"coalesce":"Box\u{0}mixed"})).unwrap().is_null());
            let changed = app.session.active().unwrap().doc.clone();
            fixture::exact(&before, &changed, -1, -1);
            assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
            assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Layer Properties"));
            assert_eq!(app.session.active().unwrap().source, source);
            preview(&changed);
            app.run("edit.undo", Value::Null).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&moved));
            let history = app.session.active().unwrap().history.clone();
            app.run("layer.translate", json!({"dx":0,"coalesce":"Box\u{0}mixed"})).unwrap();
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
            app.run("edit.redo", Value::Null).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&changed));
            app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("Box Move denied".into())));
            assert!(app.run("layer.translate", json!({"dx":1})).unwrap_err().contains("Box Move denied"));
            app.session.authorize = None;
            for p in [json!({"target":"mask","dx":1}), json!({"layer":u64::MAX,"dx":1}), json!({"dx":i32::MAX}), json!({"units":"cm","dx":1})] {
                assert!(app.run("layer.translate", p).is_err());
            }
            assert!(app.session.active().unwrap().doc.shares_state(&changed));
            let titled = changed.accounted().unwrap().try_with_document_name("Moved native title", &fixture::ctl()).unwrap();
            let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", fixture::archive(&titled), &fixture::ctl()).unwrap();
            fixture::exact(&changed, &reopened.session.active().unwrap().doc, 0, 0);
            preview(&reopened.session.active().unwrap().doc);
            reopened.run("layer.translate", json!({"layer":layer.0,"dx":1,"dy":1})).unwrap();
            fixture::exact(&changed, &reopened.session.active().unwrap().doc, 1, 1);
            assert_eq!(reopened.session.active().unwrap().doc.name, "Moved native title");
        }
    }
    let owner = fixture::owner(photocraft_doc::SampleType::F32, false, true);
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_native_open::adopt_bytes(&mut app, "locked.pcraft", fixture::archive(&owner), &fixture::ctl()).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    assert!(app.run("layer.translate", json!({"dx":1})).is_err());
    assert!(app.run("layer.translate", json!({"dx":0})).unwrap().is_null());
    assert!(app.session.active().unwrap().doc.shares_state(&before));
}
