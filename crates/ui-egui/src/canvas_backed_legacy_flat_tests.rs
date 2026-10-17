//! UNRUN legacy source/native lifecycle. Independent original PNG checks and
//! changed CURRENT public-kernel references have separate evidence roles.
use super::*;
#[path = "canvas_legacy_flat_fixture.rs"]
mod fixture;
use fixture::ctl;
use serde_json::{Value, json};
fn original_png(snapshot: &photocraft_engine::DocumentSnapshot, root: &std::path::Path, case: &fixture::Case) {
    let png = photocraft_codecs::decode(&std::fs::read(root.join(&case.png)).unwrap()).unwrap();
    let (actual, fee) = backed_snapshot_display_image(snapshot, 16, 12, None, &Interrupt::NONE).unwrap();
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
fn current_preview(original: &photocraft_doc::Document, current: &photocraft_engine::DocumentSnapshot) {
    let expected = display_image(None, &fixture::reference(original, current));
    let (actual, fee) = backed_snapshot_display_image(current, 16, 12, None, &Interrupt::NONE).unwrap();
    for (a, b) in actual.pixels.iter().zip(&expected.pixels) {
        for c in 0..4 {
            assert!((i16::from(a.to_array()[c]) - i16::from(b.to_array()[c])).abs() <= 1);
        }
    }
    drop((actual, fee));
}
#[test]
#[ignore = "UNRUN: all14 legacy modes/source+edit28 actual GIMP inputs, literal raw/legacy epochs and native source identity, original PNG vs CURRENT kernel roles, byte/native adoption, metadata/Move/modern20 transition/undo and native persistence; no alias/default enrollment"]
fn legacy14_ui_keeps_original_policies_then_explicitly_transitions_current() {
    let (root, cases) = fixture::cases();
    for case in cases {
        let bytes = fixture::input(&root, &case);
        let source = fixture::import(&bytes);
        assert!(matches!(&source, photocraft_io::OwnedXcfImport::SeparableBlend(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, &case.file, &source, &ctl()).unwrap();
        app.background_jobs = true;
        let original = app.session.active().unwrap().doc.clone();
        fixture::observed(&root, &case, &original);
        assert!(app.session.active().unwrap().path.is_none());
        assert!(app.session.active().unwrap().source_read_only);
        original_png(&original, &root, &case);
        // Actual legacy CURRENT archive before changing any policy, no modern hint shortcut.
        let original_archive = fixture::archive(original.accounted().unwrap());
        let mut native = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut native, "Legacy native.pcraft", original_archive, &ctl()).unwrap();
        fixture::observed(&root, &case, &native.session.active().unwrap().doc);
        assert!(!native.session.active().unwrap().source_read_only);
        let (request, wake, carrier) = crate::control::ControlRequest::with_retained_query(
            "engine.execute",
            json!({"command":"xcf.inspect","params":{"offset":1,"limit":1},"wait":false}),
            json!(14),
            1 << 20,
        );
        assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &request), crate::control::Outcome::Deferred));
        assert!(wake.try_recv().unwrap().is_null());
        let wire = carrier.try_recv().unwrap();
        let v: Value = serde_json::from_slice(wire.bytes()).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["result"]["items"]["total"], 2);
        assert_eq!(v["result"]["filters"]["total"], 0);
        drop(wire);
        app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("legacy move denied".into())));
        assert!(app.run("layer.translate", json!({"dx":1})).unwrap_err().contains("legacy move denied"));
        app.session.authorize = None;
        let top = original.layers[1].id;
        let past = app.session.active().unwrap().history.past_len();
        app.run("layer.translate", json!({"layer":top.0,"dx":-1,"dy":2,"coalesce":"legacy"})).unwrap();
        let moved = app.session.active().unwrap().doc.clone();
        fixture::moved(&original, &moved, top, -1, 2);
        current_preview(&original, &moved);
        app.run("layer.setProps", json!({"layer":top.0,"name":"Legacy CURRENT foreground","opacity":0.375,"coalesce":"legacy"})).unwrap();
        let metadata = app.session.active().unwrap().doc.clone();
        fixture::raw(&moved, &metadata);
        current_preview(&original, &metadata);
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
        let revision = app.session.active().unwrap().revision;
        let mut unsupported = fixture::transition(&metadata, revision);
        unsupported["sourceMode"] = json!(case.mode);
        unsupported["legacy"] = json!(true);
        assert!(app.run("xcf.composition.set", unsupported).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&metadata));
        // Existing modern20 wire only: Screen31 is explicit, no legacy-mode chooser/alias.
        let reply = app.run("xcf.composition.set", fixture::transition(&metadata, revision)).unwrap();
        assert_eq!(reply["updated"], true);
        let changed = app.session.active().unwrap().doc.clone();
        fixture::transitioned(&changed);
        fixture::raw(&metadata, &changed);
        current_preview(&original, &changed);
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&metadata));
        app.run("edit.redo", Value::Null).unwrap();
        let titled = changed.accounted().unwrap().try_with_document_name("Legacy to modern CURRENT", &ctl()).unwrap();
        let q = titled.quota().clone();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Other title.pcraft", fixture::archive(&titled), &ctl()).unwrap();
        let current = reopened.session.active().unwrap().doc.clone();
        assert_eq!(current.name, "Legacy to modern CURRENT");
        fixture::raw(&changed, &current);
        assert_eq!(changed.xcf_state, current.xcf_state);
        assert!(q.shares_ledger(current.accounted().unwrap().quota()));
        current_preview(&original, &current);
        assert_eq!(std::fs::read(root.join(&case.file)).unwrap(), bytes);
    }
}
