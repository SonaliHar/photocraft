//! UNRUN general CURRENT native workflow, no authored-source geometry claim.
use super::*;
#[path = "canvas_flat_current_fixture.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use fixture::{archive, ctl, fixture, page, payload, reference};
use serde_json::{Value, json};
fn preview(doc: &photocraft_engine::DocumentSnapshot, literals: &[fixture::Literal]) {
    let expected = reference(doc, literals);
    let (image, fee) = backed_snapshot_display_image(doc, 263, 19, None, &Interrupt::NONE).unwrap();
    assert_eq!(image.size, [263, 19]);
    for (actual, p) in image.pixels.iter().zip(expected) {
        let b = p.map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
        let b = egui::Color32::from_rgba_unmultiplied(b[0], b[1], b[2], b[3]).to_array();
        for (c, expected_component) in b.iter().enumerate() {
            assert!((i16::from(actual.to_array()[c]) - i16::from(*expected_component)).abs() <= 1);
        }
    }
    drop((image, fee));
}
#[test]
#[ignore = "UNRUN: genuine sameq synthetic CURRENT five mixed flat native leaves/canvas263x19 negative and tile-crossing extents, hidden/zeroopacity, paid paginated transport, metadata/title/native continuation"]
fn general_flat_current_ui_uses_strict_native_recipe_and_retained_pages() {
    for all_normal in [false, true] {
        let (owner, literals) = if all_normal { fixture::normal_fixture() } else { fixture() };
        let q = owner.quota().clone();
        let input = archive(&owner);
        assert!(q.shares_ledger(input.quota()));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut app, "CURRENT fixture.pcraft", input, &ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        assert!(before.accounted().unwrap().is_readonly_flat_composite_storage());
        fixture::same_quota(before.accounted().unwrap());
        assert!(q.shares_ledger(before.accounted().unwrap().quota()));
        assert!(app.session.active().unwrap().path.is_none());
        assert!(!app.session.active().unwrap().source_read_only);
        payload(owner.document(), &before);
        assert_eq!(owner.document().xcf_state, before.xcf_state);
        preview(&before, &literals);
        for (params, offset, limit) in [
            (Value::Null, 0, 64),
            (json!({"limit":1}), 0, 1),
            (json!({"offset":1,"limit":2}), 1, 2),
            (json!({"offset":4,"limit":128}), 4, 128),
            (json!({"offset":100000,"limit":1}), 100000, 1),
        ] {
            let (request, wake, carrier) = crate::control::ControlRequest::with_retained_query(
                "engine.execute",
                json!({"command":"xcf.inspect","params":params,"wait":false}),
                json!(42),
                1 << 20,
            );
            assert!(matches!(crate::control::handle(&mut app, &egui::Context::default(), &request), crate::control::Outcome::Deferred));
            assert!(wake.try_recv().unwrap().is_null());
            let wire = carrier.try_recv().unwrap();
            let v: Value = serde_json::from_slice(wire.bytes()).unwrap();
            assert_eq!(v["ok"], true);
            page(&v["result"], &before, offset, limit);
            drop(wire);
        }
        let hidden = before.layers[3].id;
        let zero = before.layers[4].id;
        let past = app.session.active().unwrap().history.past_len();
        app.run("layer.setProps", json!({"layer":hidden.0,"name":"Current visible overlay","visible":true,"opacity":0.375,"coalesce":"general"})).unwrap();
        app.run("layer.setProps", json!({"layer":zero.0,"opacity":0.125,"coalesce":"general"})).unwrap();
        assert!(!app.session.has_jobs());
        assert!(app.jobs.last_started.is_none());
        let changed = app.session.active().unwrap().doc.clone();
        payload(&before, &changed);
        preview(&changed, &literals);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let redo = app.session.active().unwrap().history.clone();
        app.run("layer.setProps", json!({"layer":hidden.0,"visible":false,"opacity":0.5})).unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&redo));
        app.run("edit.redo", json!({})).unwrap();
        let titled = changed.accounted().unwrap().try_with_document_name("General CURRENT saved title", &ctl()).unwrap();
        let next = archive(&titled);
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Other filename.pcraft", next, &ctl()).unwrap();
        let current = &reopened.session.active().unwrap().doc;
        assert_eq!(current.name, "General CURRENT saved title");
        assert_eq!(changed.xcf_state, current.xcf_state);
        assert!(q.shares_ledger(current.accounted().unwrap().quota()));
        fixture::same_quota(current.accounted().unwrap());
        payload(&changed, current);
        preview(current, &literals);
        assert!(photocraft_engine::accounted_filter_inspect::prepare(&reopened.session, &json!({"filterId":"1"}), &ctl()).is_err());
    }
    let (bad, _) = fixture::malformed_fixture();
    let input = archive(&bad);
    assert!(photocraft_engine::accounted_native_open::is_current_separable_blend_candidate(input.document(), &ctl()).unwrap());
    let mut refused = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    assert!(crate::owned_native_open::adopt_bytes(&mut refused, "Malformed matching CURRENT.pcraft", input, &ctl()).is_err());
    assert!(refused.session.documents().is_empty());
    // External metadata clone tests routing only, never storage/render admission.
    let (normal, _) = fixture::normal_fixture();
    let mut mixed = normal.document().clone();
    mixed.xcf_state.as_mut().unwrap().items[0].composition.as_mut().unwrap().source_mode = 0;
    assert!(!photocraft_engine::accounted_native_open::is_current_separable_blend_candidate(&mixed, &ctl()).unwrap());
}
