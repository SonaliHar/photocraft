//! UNRUN original native/policy evidence vs generated CURRENT display lifecycle.
//! Eight absent Gray producer ICCs remain HOLD; no equality is inferred.
use super::*;
#[path = "canvas_alpha_flat_fixture.rs"]
mod fixture;
use fixture::ctl;
use serde_json::{Value, json};
fn preview(case: &fixture::Case, current: &photocraft_engine::DocumentSnapshot) {
    let expected = display_image(None, &fixture::reference(case, current));
    let (actual, fee) = backed_snapshot_display_image(current, 16, 12, None, &Interrupt::NONE).unwrap();
    for (a, b) in actual.pixels.iter().zip(&expected.pixels) {
        for c in 0..4 {
            assert!((i16::from(a.to_array()[c]) - i16::from(b.to_array()[c])).abs() <= 1);
        }
    }
    drop((actual, fee));
}
fn original_rgb_png(case: &fixture::Case, current: &photocraft_engine::DocumentSnapshot) {
    assert!(!case.gray);
    let png = photocraft_codecs::decode(&std::fs::read(case.root.join(&case.png)).unwrap()).unwrap();
    let (actual, fee) = backed_snapshot_display_image(current, 16, 12, None, &Interrupt::NONE).unwrap();
    for y in 0..12 {
        for x in 0..16 {
            let p: [u8; 4] = std::array::from_fn(|c| (png.get(x, y, c).clamp(0., 1.) * 255.).round() as u8);
            let expected = egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]).to_array();
            for (c, expected_component) in expected.iter().enumerate() {
                assert!((i16::from(actual.pixels[(y * 16 + x) as usize].to_array()[c]) - i16::from(*expected_component)).abs() <= 2);
            }
        }
    }
    drop((actual, fee));
}
#[test]
#[ignore = "UNRUN: alpha57..60 RGB+Gray16 actual source/edit files, exact native/raw/profile roles, paid query/metadata/Move/title/native/auth; RGBmodern20 transition, Gray unsupported transition atomic and panel readonly; missing8producerICC HOLD no equality/numerical acceptance"]
fn alpha_rgb_gray_ui_preserves_native_and_model_specific_current_authority() {
    for case in fixture::cases() {
        let bytes = fixture::input(&case);
        let source = fixture::import(&bytes);
        assert!(matches!(&source, photocraft_io::OwnedXcfImport::SeparableBlend(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, &case.file, &source, &ctl()).unwrap();
        app.background_jobs = true;
        let original = app.session.active().unwrap().doc.clone();
        fixture::observed(&case, &original);
        fixture::same_quota(original.accounted().unwrap());
        assert!(app.session.active().unwrap().source_read_only);
        assert!(app.session.active().unwrap().path.is_none());
        // Gray CURRENT reference uses actual SGray only; its producer ICC bytes are absent.
        if case.gray {
            preview(&case, &original);
            for layer in &original.layers {
                assert!(crate::xcf_panels::composition::capture_alpha_composition_test(&app, layer.id).is_err());
            }
        } else {
            original_rgb_png(&case, &original);
            assert!(crate::xcf_panels::composition::capture_alpha_composition_test(&app, original.layers[0].id).is_ok());
        }
        let mut native = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut native, "Original alpha.pcraft", fixture::archive(original.accounted().unwrap()), &ctl()).unwrap();
        fixture::observed(&case, &native.session.active().unwrap().doc);
        assert!(!native.session.active().unwrap().source_read_only);
        let (request, wake, carrier) = crate::control::ControlRequest::with_retained_query(
            "engine.execute",
            json!({"command":"xcf.inspect","params":{"offset":1,"limit":1},"wait":false}),
            json!(60),
            1 << 20,
        );
        assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &request), crate::control::Outcome::Deferred));
        assert!(wake.try_recv().unwrap().is_null());
        let wire = carrier.try_recv().unwrap();
        let reply: Value = serde_json::from_slice(wire.bytes()).unwrap();
        assert_eq!(reply["ok"], true);
        fixture::page(&reply["result"], &original, 1, 1);
        drop(wire);
        app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("alpha move denied".into())));
        assert!(app.run("layer.translate", json!({"dx":1})).unwrap_err().contains("alpha move denied"));
        app.session.authorize = None;
        let top = original.layers[1].id;
        let past = app.session.active().unwrap().history.past_len();
        app.run("layer.translate", json!({"layer":top.0,"dx":-1,"dy":2,"coalesce":"alpha"})).unwrap();
        let moved = app.session.active().unwrap().doc.clone();
        fixture::moved(&original, &moved, top, -1, 2);
        preview(&case, &moved);
        app.run("layer.setProps", json!({"layer":top.0,"name":"Alpha CURRENT foreground","opacity":0.375,"coalesce":"alpha"})).unwrap();
        let metadata = app.session.active().unwrap().doc.clone();
        fixture::raw(&moved, &metadata);
        preview(&case, &metadata);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.entry_labels().last(), Some("Move"));
        assert!(app.jobs.last_started.is_none());
        assert!(!app.session.has_jobs());
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&original));
        let redo = app.session.active().unwrap().history.clone();
        app.run("layer.translate", json!({"layer":"ignored rounded-zero","dx":0.49})).unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&redo));
        app.run("edit.redo", Value::Null).unwrap();
        let rev = app.session.active().unwrap().revision;
        let transition = fixture::transition(&metadata, rev);
        let changed = if case.gray {
            let history = app.session.active().unwrap().history.clone();
            let journal = app.session.journal.clone();
            assert!(app.run("xcf.composition.set", transition).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&metadata));
            assert_eq!(app.session.active().unwrap().revision, rev);
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(app.session.journal, journal);
            metadata.clone()
        } else {
            let reply = app.run("xcf.composition.set", transition).unwrap();
            assert_eq!(reply["updated"], true);
            let changed = app.session.active().unwrap().doc.clone();
            fixture::raw(&metadata, &changed);
            app.run("edit.undo", Value::Null).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&metadata));
            app.run("edit.redo", Value::Null).unwrap();
            changed
        };
        preview(&case, &changed);
        let named = changed.accounted().unwrap().try_with_document_name("Alpha model CURRENT", &ctl()).unwrap();
        let q = named.quota().clone();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Other name.pcraft", fixture::archive(&named), &ctl()).unwrap();
        let current = reopened.session.active().unwrap().doc.clone();
        assert_eq!(current.name, "Alpha model CURRENT");
        fixture::raw(&changed, &current);
        fixture::same_quota(current.accounted().unwrap());
        assert_eq!(changed.xcf_state, current.xcf_state);
        assert!(q.shares_ledger(current.accounted().unwrap().quota()));
        preview(&case, &current);
        assert_eq!(std::fs::read(case.root.join(&case.file)).unwrap(), bytes);
    }
}
