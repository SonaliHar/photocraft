//! UNRUN actual private composition capture/request + registered UI dispatch.
//! Synthetic CURRENT references reuse old kernels; UI visual testing deferred.
use super::*;
#[path = "../../canvas_separable_blend_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod authored;
#[path = "../../canvas_flat_composition_fixture.rs"]
#[allow(dead_code)]
mod common;
#[path = "../../canvas_flat_current_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use fixture::{archive, ctl};
fn preview(doc: &photocraft_engine::DocumentSnapshot, literals: &[fixture::Literal]) {
    let expected = fixture::reference(doc, literals);
    let (image, fee) = crate::canvas::assigned_geometry_test_support::display(doc).unwrap();
    for (actual, p) in image.pixels.iter().zip(expected) {
        let p = p.map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
        let p = egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]).to_array();
        for (c, expected) in p.iter().enumerate() {
            assert!((i16::from(actual.to_array()[c]) - i16::from(*expected)).abs() <= 1);
        }
    }
    drop((image, fee));
}
#[test]
#[ignore = "UNRUN: owned-only20mode composition private UI capture/request + bg=true registered command, explicit spaces/operators/grain transition/stale/noopredo/native; no physical UI visual claim"]
fn owned_composition_selector_preserves_source_modes_until_explicit_choice() {
    assert_eq!(EDITABLE_MODES, common::MODES);
    assert!(mode_label(46).is_none() && mode_label(47).is_none());
    // Topmost positive-opacity target has no downstream contrast-domain ambiguity.
    for (mode, blend, composite, operator) in common::MODES.into_iter().map(|m| (m, 1, 1, 1)).chain([(30, 2, 1, 2), (30, 4, 2, 3), (30, 1, 4, 4)]) {
        let (owner, literals) = fixture::fixture();
        let q = owner.quota().clone();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut app, "CURRENT composition.pcraft", archive(&owner), &ctl()).unwrap();
        app.background_jobs = true;
        let layer = app.session.active().unwrap().doc.layers[4].id;
        app.run("layer.setProps", json!({"layer":layer.0,"opacity":0.125})).unwrap();
        // The synthetic renderer fixture deliberately permits an inert physical
        // Normal surrogate. Establish the ordinary Add mapping through the real
        // command before baselining exact raw+physical no-op behavior.
        let setup = capture(&app, layer).unwrap();
        let setup_draft = initial(&setup, &current(&app, layer).unwrap());
        app.run("xcf.composition.set", request(&app, &setup, &setup_draft).unwrap()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.clone();
        let past = history.past_len();
        let revision = app.session.active().unwrap().revision;
        let origin = capture(&app, layer).unwrap();
        let mut draft = initial(&origin, &current(&app, layer).unwrap());
        draft.source_mode = mode;
        draft.blend_space = Some(blend);
        draft.composite_space = Some(composite);
        draft.operator = Some(operator);
        let params = request(&app, &origin, &draft).unwrap();
        assert_eq!(params["documentId"], before.id.0.to_string());
        assert_eq!(params["expectedRevision"], revision.to_string());
        assert_eq!(app.run("xcf.composition.set", params.clone()).unwrap(), json!({"layerId":layer.0.to_string(),"sourceMode":mode,"updated":true}));
        assert!(!app.session.has_jobs());
        assert!(app.jobs.last_started.is_none());
        let changed = app.session.active().unwrap().doc.clone();
        common::tuple(&changed, layer, mode, i32::from(blend), i32::from(composite), i32::from(operator));
        common::preserved(&before, &changed);
        preview(&changed, &literals);
        let did_change = !changed.shares_state(&before);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + usize::from(did_change));
        if did_change {
            assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Set GIMP Composition Policy"));
            assert!(request(&app, &origin, &draft).is_err());
            assert!(app.run("xcf.composition.set", params).is_err());
            app.run("edit.undo", json!({})).unwrap();
            assert!(app.session.active().unwrap().doc.shares_state(&before));
            let redo = app.session.active().unwrap().history.clone();
            let fresh = capture(&app, layer).unwrap();
            let noop = initial(&fresh, &current(&app, layer).unwrap());
            assert_eq!(app.run("xcf.composition.set", request(&app, &fresh, &noop).unwrap()).unwrap()["updated"], true);
            assert!(app.session.active().unwrap().history.shares_accounted_history(&redo));
            app.run("edit.redo", json!({})).unwrap();
        } else {
            assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        }
        let live = capture(&app, layer).unwrap();
        let mut invalid = initial(&live, &current(&app, layer).unwrap());
        invalid.source_mode = 46;
        assert!(request(&app, &live, &invalid).is_err());
        let old_target = app.session.active().unwrap().active_layer;
        let stable_draft = initial(&live, &current(&app, layer).unwrap());
        assert!(request(&app, &live, &stable_draft).is_ok());
        let other = app.session.active().unwrap().doc.layers[0].id;
        app.session.active_mut().unwrap().active_layer = Some(other);
        assert!(request(&app, &live, &stable_draft).is_err());
        app.session.active_mut().unwrap().active_layer = old_target;
        let title = changed.accounted().unwrap().try_with_document_name("Composition native", &ctl()).unwrap();
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "Different filename.pcraft", archive(&title), &ctl()).unwrap();
        let native = &reopened.session.active().unwrap().doc;
        assert_eq!(native.name, "Composition native");
        assert_eq!(native.xcf_state, changed.xcf_state);
        assert!(q.shares_ledger(native.accounted().unwrap().quota()));
        common::preserved(&changed, native);
        preview(native, &literals);
        let reply = photocraft_engine::accounted_xcf_inspect::prepare(&reopened.session, &json!({"limit":128}), &ctl()).unwrap();
        assert_eq!(reply.value()["items"]["total"], 5);
        drop(reply);
    }
    // Actual authored grain source remains grain in the draft; no silent alias.
    let (root, cases) = authored::cases();
    for case in cases.into_iter().filter(|c| c.file == "mode-grain-merge.xcf" || c.file == "mode-grain-extract.xcf") {
        let bytes = std::fs::read(root.join(&case.file)).unwrap();
        let q = photocraft_raster::backing::quota::BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits {
            resident: 256 << 20,
            index: 32 << 20,
            scratch: 0,
        });
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
            .unwrap()
            .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
            .unwrap();
        let source = photocraft_io::import_owned_xcf_from_reader_in_quota(
            &case.file,
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q,
            &ctl(),
        )
        .unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, &case.file, &source, &ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let layer = before.layers[1].id;
        let origin = capture(&app, layer).unwrap();
        let mut draft = initial(&origin, &current(&app, layer).unwrap());
        assert!(matches!(draft.source_mode, 46 | 47));
        draft.blend_space = Some(1);
        draft.composite_space = Some(1);
        draft.operator = Some(1);
        assert!(request(&app, &origin, &draft).is_err());
        draft.source_mode = 28;
        app.run("xcf.composition.set", request(&app, &origin, &draft).unwrap()).unwrap();
        assert_eq!(
            app.session
                .active()
                .unwrap()
                .doc
                .xcf_state
                .as_ref()
                .unwrap()
                .items
                .iter()
                .find(|i| i.item == ItemRef::Layer(layer))
                .unwrap()
                .composition
                .as_ref()
                .unwrap()
                .source_mode,
            28
        );
        authored::observed(&root, &before, &case);
        common::preserved(&before, &app.session.active().unwrap().doc);
        assert_eq!(std::fs::read(root.join(&case.file)).unwrap(), bytes);
    }
}
