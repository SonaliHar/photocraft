//! Deferred actual source-first masked Point transaction contracts; no producer RGB oracle.
use super::*;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "../accounted_exposure_masked_test_support.rs"]
mod fixture;
fn session(case: &serde_json::Value) -> Session {
    let input = fixture::import(case);
    let photocraft_io::OwnedXcfImport::Point(input) = &input else { panic!("Point") };
    let mut s = Session::new();
    s.add_owned_point_xcf_import(input, "masked.xcf", Some(case["source"].as_str().unwrap()), &fixture::ctl()).unwrap();
    s
}
#[test]
#[ignore = "UNRUN six authentic nonempty sources/Pointclass/hint11/native, atomic stop+enable/rawmask retention/undo/reorder0/finalremove+emptyhistory; Source proof real not fakeCurrent"]
fn masked_source_edits_preserve_raw_mask() {
    for case in fixture::cases() {
        let mut s = session(&case);
        let original = s.active().unwrap().doc.clone();
        let id = original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        assert!(crate::xcf_filter_cmds::masked_exposure_point_edit_candidate(
            original.accounted().unwrap(),
            &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
        ));
        assert!(crate::accounted_native_open::is_current_pure_point_work_candidate(original.accounted().unwrap(), &fixture::ctl()).unwrap());
        let archive = fixture::archive(original.accounted().unwrap());
        assert_eq!(archive.current_storage_hint(), Some(photocraft_format::NativeCurrentStorageHint::PointF32Coverage));
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(archive, "masked.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&original, &reopened.active().unwrap().doc);
        let revision = s.active().unwrap().revision;
        s.execute("xcf.filter.set",serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0.75f32.to_bits()}}}],"enabled":false})).unwrap();
        assert_eq!(s.active().unwrap().revision, revision + 1);
        fixture::payload(&original, &s.active().unwrap().doc);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&original, &s.active().unwrap().doc);
        s.execute("xcf.filter.move", serde_json::json!({"filterId":id.0.to_string(),"index":0})).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&original));
        s.execute("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
        assert!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
        let empty = s.active().unwrap().doc.clone();
        let mut restored = Session::new();
        restored.add_owned_native_point_archive(fixture::archive(empty.accounted().unwrap()), "empty.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&empty, &restored.active().unwrap().doc);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&original, &s.active().unwrap().doc);
        assert_eq!(fixture::bytes(&case).len() as u64, case["bytes"].as_u64().unwrap());
    }
}
#[test]
#[ignore = "UNRUN strict new black endpoint and staged scale overflow refusal, no-op retained state, early Work/cancel and invalidopacity/selection authority; full proof held by unchanged common transaction"]
fn masked_invalid_requests_are_atomic() {
    for case in fixture::cases() {
        let mut s = session(&case);
        let original = s.active().unwrap().doc.clone();
        let id = original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        for (name, bits) in [("black-level", 0x3dcc_cccd), ("black-level", 0xbdcc_cccd), ("exposure", f32::MAX.to_bits())] {
            assert!(
                s.execute(
                    "xcf.filter.set",
                    serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":name,"value":{"Number":{"Float32Bits":bits}}}]})
                )
                .is_err()
            );
            assert!(s.active().unwrap().doc.shares_state(&original));
        }
        assert!(s.execute("xcf.filter.set", serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0x3fe0000000000000"})).is_err());
        let params = serde_json::json!({"filterId":id.0.to_string(),"enabled":true});
        let deny = |_| Err(BackingError::Work);
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
        assert!(Request::parse(&s, "xcf.filter.enable", &params, &c).is_err());
        let cancel = || true;
        let i = photocraft_raster::Interrupt::cancel_only(&cancel);
        let c = ReadControl { interrupt: &i, work: &|_| Ok(()) };
        assert!(Request::parse(&s, "xcf.filter.enable", &params, &c).is_err());
        s.execute("xcf.filter.enable", params).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&original));
    }
}
