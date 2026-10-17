//! UNRUN original lifecycle only; no GIMP numeric acceptance while ICC/RGB/zero HOLDs remain.
use super::*;
#[path = "canvas_original_flat_fixture.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use fixture::{ctl, page, payload};
use serde_json::{Value, json};
fn image(doc: &photocraft_engine::DocumentSnapshot) -> egui::ColorImage {
    let (image, fee) = backed_snapshot_display_image(doc, doc.size.width, doc.size.height, None, &Interrupt::NONE).unwrap();
    // Return is an external harness copy, never an unleased production output.
    let copy = image.clone();
    drop((image, fee));
    copy
}
#[test]
#[ignore = "UNRUN: six independently saved GIMP originals, literal raw/resolved/native source words, byte adoption, paid pages, CURRENT metadata/composition undo and native display continuity; ICC and signed-zero/RGB HOLDs are not waived"]
fn original_flat_ui_bytes_keep_source_bits_and_current_native_continuity() {
    for case in fixture::cases() {
        let source = fixture::import(&case);
        assert!(matches!(&source, photocraft_io::OwnedXcfImport::SeparableBlend(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "Original research.xcf", &source, &ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        fixture::observed(&case, &before);
        fixture::same_quota(before.accounted().unwrap());
        assert!(app.session.active().unwrap().path.is_none());
        assert!(app.session.active().unwrap().source_read_only);
        let original_image = image(&before);
        for (params, offset, limit) in [(Value::Null, 0, 64), (json!({"offset":1,"limit":1}), 1, 1), (json!({"offset":100000,"limit":128}), 100000, 128)] {
            let (request, wake, carrier) = crate::control::ControlRequest::with_retained_query(
                "engine.execute",
                json!({"command":"xcf.inspect","params":params,"wait":false}),
                json!(19),
                1 << 20,
            );
            assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &request), crate::control::Outcome::Deferred));
            assert!(wake.try_recv().unwrap().is_null());
            let wire = carrier.try_recv().unwrap();
            let value: Value = serde_json::from_slice(wire.bytes()).unwrap();
            assert_eq!(value["ok"], true);
            page(&value["result"], &before, offset, limit);
            drop(wire);
        }
        assert!(photocraft_engine::accounted_filter_inspect::prepare(&app.session, &json!({"filterId":"1"}), &ctl()).is_err());
        let target = before.layers.last().unwrap().id;
        app.run("layer.setProps", json!({"layer":target.0,"name":"Original continued CURRENT","opacity":0.375})).unwrap();
        let metadata = app.session.active().unwrap().doc.clone();
        payload(&before, &metadata);
        assert!(!app.session.has_jobs());
        assert!(app.jobs.last_started.is_none());
        let rev = app.session.active().unwrap().revision;
        let reply=app.run("xcf.composition.set",json!({"layerId":target.0.to_string(),"sourceMode":28,"legacy":false,"blendSpace":"linear-rgb","compositeSpace":"linear-rgb","operator":"union","documentId":metadata.id.0.to_string(),"expectedRevision":rev.to_string()})).unwrap();
        assert_eq!(reply["updated"], true);
        let changed = app.session.active().unwrap().doc.clone();
        payload(&before, &changed);
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&metadata));
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(image(&before), original_image);
        let history = app.session.active().unwrap().history.clone();
        app.run("layer.setProps", json!({"layer":target.0,"name":before.layers.last().unwrap().name})).unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        app.run("edit.redo", Value::Null).unwrap();
        app.run("edit.redo", Value::Null).unwrap();
        let changed_image = image(&changed);
        let titled = changed.accounted().unwrap().try_with_document_name("Independent original continued", &ctl()).unwrap();
        let q = titled.quota().clone();
        let input = fixture::archive(&titled);
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Other picker title.pcraft", input, &ctl()).unwrap();
        let current = &reopened.session.active().unwrap().doc;
        assert_eq!(current.name, "Independent original continued");
        payload(&changed, current);
        assert_eq!(current.xcf_state, changed.xcf_state);
        assert!(q.shares_ledger(current.accounted().unwrap().quota()));
        assert!(!reopened.session.active().unwrap().source_read_only);
        assert!(reopened.session.active().unwrap().path.is_none());
        // Adapter persistence continuity only, not comparison with GIMP's effective ICC.
        assert_eq!(image(current), changed_image);
        fixture::refuses(&fixture::malformed(&case));
        fixture::refuses(&case.input[..case.input.len() / 2]);
        assert_eq!(std::fs::read(&case.path).unwrap(), case.input);
    }
}
