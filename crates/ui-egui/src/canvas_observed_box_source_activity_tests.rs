//! UNRUN genuine GIMP source activity through byte/typed/native consumers.
use super::*;
#[path = "observed_box_source_activity_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use serde_json::{Value, json};
#[test]
#[ignore = "UNRUN: fifteen independently GIMP-authored activity originals, exact raw6/33/native/ICC source roles, typed byte/native adoption and CURRENT activation/metadata/Move/history/canvas; derived adapter display is not GIMP numerical acceptance"]
fn observed_source_activity_ui_keeps_original_records_and_current_edits_distinct() {
    for case in fixture::cases() {
        let imported = fixture::import(&case);
        assert!(matches!(&imported, photocraft_io::OwnedXcfImport::Point(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, &case.id, &imported, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let original = app.session.active().unwrap().doc.clone();
        fixture::observed(&case, &original);
        assert!(app.session.active().unwrap().source_read_only);
        assert_eq!(original.accounted().unwrap().is_readonly_assigned_point_storage(), case.assigned);
        let expected = display_image(None, &fixture::reference(&original));
        let (actual, fee) =
            backed_snapshot_display_image(&original, original.size.width, original.size.height, None, &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(actual.pixels.len(), expected.pixels.len());
        for (a, b) in actual.pixels.iter().zip(&expected.pixels) {
            for (a, b) in a.to_array().into_iter().zip(b.to_array()) {
                assert!((i16::from(a) - i16::from(b)).abs() <= 1);
            }
        }
        if !case.visible || case.opacity == 0 {
            assert!(actual.pixels.iter().all(|pixel| pixel.a() == 0));
        }
        drop((actual, fee));
        // Preserve actual authored state through native before changing it.
        let mut continued = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut continued, "original.pcraft", fixture::archive(original.accounted().unwrap()), &fixture::ctl()).unwrap();
        continued.background_jobs = true;
        let native = continued.session.active().unwrap().doc.clone();
        fixture::observed(&case, &native);
        fixture::exact(&original, &native, 0, 0);
        let layer = native.layers[0].id;
        let filter = native.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        if !case.enabled {
            continued.run("xcf.filter.enable", json!({"filterId":filter,"enabled":true})).unwrap();
            assert!(continued.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
            continued.run("edit.undo", Value::Null).unwrap();
            assert!(continued.session.active().unwrap().doc.shares_state(&native));
        }
        let past = continued.session.active().unwrap().history.past_len();
        continued
            .run("layer.setProps", json!({"layer":layer.0,"name":"Current source continuation","visible":true,"opacity":0.5,"coalesce":"activity"}))
            .unwrap();
        continued.run("layer.translate", json!({"dx":1,"dy":-1,"coalesce":"activity"})).unwrap();
        let changed = continued.session.active().unwrap().doc.clone();
        fixture::exact(&native, &changed, 1, -1);
        assert_eq!(continued.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(continued.session.active().unwrap().history.undo_label(), Some("Layer Properties"));
        continued.run("edit.undo", Value::Null).unwrap();
        assert!(continued.session.active().unwrap().doc.shares_state(&native));
        let history = continued.session.active().unwrap().history.clone();
        continued
            .run(
                "layer.setProps",
                json!({"layer":layer.0,"name":native.layers[0].name,"visible":case.visible,"opacity":f32::from_bits(case.opacity),"coalesce":"activity"}),
            )
            .unwrap();
        assert!(continued.session.active().unwrap().history.shares_accounted_history(&history));
        continued.run("edit.redo", Value::Null).unwrap();
        assert!(continued.session.active().unwrap().doc.shares_state(&changed));
        if !case.visible || case.opacity == 0 {
            // Synthetic malformed copies of inactive actual originals: not GIMP
            // witnesses. Neither a transparent view nor old tag6 repairs NaN33.
            assert!(fixture::import_bytes("malformed.xcf", fixture::malformed_inactive(&case)).is_err());
            assert!(continued.session.active().unwrap().doc.shares_state(&changed));
        }
        let titled = changed.accounted().unwrap().try_with_document_name("Activity continued title", &fixture::ctl()).unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", fixture::archive(&titled), &fixture::ctl()).unwrap();
        fixture::exact(&changed, &reopened.session.active().unwrap().doc, 0, 0);
        assert_eq!(reopened.session.active().unwrap().doc.name, "Activity continued title");
        assert!(!reopened.session.has_jobs());
        assert_eq!(std::fs::read(&case.path).unwrap(), case.bytes);
    }
}
