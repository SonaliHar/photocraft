//! Deferred loaded endpoint engine lifecycle, ordinary producer sources versus mixed synthetic CURRENT.
use super::*;
#[path = "../accounted_exposure_loaded_test_support.rs"]
mod fixture;
fn point(case: &fixture::Case) -> Session {
    let input = fixture::import(case);
    let photocraft_io::OwnedXcfImport::Point(input) = &input else { panic!("Point") };
    let mut s = Session::new();
    s.add_owned_point_xcf_import(input, "endpoint.xcf", Some(case.source), &fixture::ctl()).unwrap();
    s
}
fn node(s: &Session, id: photocraft_doc::item_identity::FilterId) -> &photocraft_doc::xcf_filters::FilterNode {
    s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| &st.nodes).find(|n| n.id == id).unwrap()
}
fn black(id: photocraft_doc::item_identity::FilterId, bits: u32) -> serde_json::Value {
    serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"black-level","value":{"Number":{"Float32Bits":bits}}}]})
}
#[test]
#[ignore = "UNRUN four authentic ordinary endpoint source imports, strict incoming endpoint refusal/stop-only preservedraw/explicitzero rawedit and undo/native Point identity/workhint/query; no producer RGB/profile oracle."]
fn loaded_point_preserves_raw_until_explicit_edit() {
    for case in &fixture::CASES {
        let mut s = point(case);
        let original = s.active().unwrap().doc.clone();
        let id = original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        assert!(photocraft_compose::ExposureParameters::from_words(0.5f32.to_bits(), case.black).is_err());
        assert_eq!(photocraft_compose::loaded_exposure_parameters(&node(&s, id).arguments).unwrap().effective().values(), [0.5, 0.0]);
        assert!(crate::accounted_native_open::is_current_pure_point_work_candidate(original.accounted().unwrap(), &fixture::ctl()).unwrap());
        let archive = fixture::archive(original.accounted().unwrap());
        assert!(!crate::accounted_native_open::is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
        let mut reopened = Session::new();
        reopened.add_owned_native_point_archive(archive, "endpoint.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&original, &reopened.active().unwrap().doc);
        for bits in [0x3dcc_cccd, 0xbdcc_cccd] {
            assert!(s.execute("xcf.filter.set", black(id, bits)).is_err());
            assert!(s.active().unwrap().doc.shares_state(&original));
        }
        let stop = serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0.75f32.to_bits()}}}]});
        s.execute("xcf.filter.set", stop).unwrap();
        assert_eq!(fixture::words(node(&s, id)), [0.75f32.to_bits(), case.black]);
        let changed = s.active().unwrap().doc.clone();
        fixture::payload(&original, &changed);
        s.execute("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":false})).unwrap();
        assert_eq!(fixture::words(node(&s, id))[1], case.black);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        let before = s.active().unwrap().doc.clone();
        assert_eq!(s.execute("xcf.filter.move", serde_json::json!({"filterId":id.0.to_string(),"index":0})).unwrap()["updated"], false);
        assert_eq!(fixture::words(node(&s, id))[1], case.black);
        let rev = s.active().unwrap().revision;
        let stackrev = before.xcf_state.as_ref().unwrap().filter_stacks[0].revision;
        s.execute("xcf.filter.set", black(id, 0)).unwrap();
        assert_eq!(fixture::words(node(&s, id))[1], 0);
        assert_eq!(s.active().unwrap().revision, rev + 1);
        assert_eq!(s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].revision, stackrev + 1);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&before, &s.active().unwrap().doc);
        assert!(s.active().unwrap().history.can_redo());
        assert_eq!(s.execute("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":true})).unwrap()["updated"], false);
        assert!(s.active().unwrap().history.can_redo());
        let q = crate::accounted_filter_inspect::prepare(&s, &serde_json::json!({"filterId":id.0.to_string()}), &fixture::ctl()).unwrap();
        assert_eq!(q.value()["scalarEditCandidate"], true);
        drop(q);
        assert_eq!(fixture::bytes(case), std::fs::read(fixture::workspace().join(case.source)).unwrap());
        s.execute("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
        let empty = s.active().unwrap().doc.clone();
        assert!(empty.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
        let mut native = Session::new();
        native.add_owned_native_point_archive(fixture::archive(empty.accounted().unwrap()), "empty-point.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&empty, &native.active().unwrap().doc);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&before, &s.active().unwrap().doc);
    }
}
#[test]
#[ignore = "UNRUN retained genuine mixed packet plus prepaid synthetic loadedV2 CURRENT endpoint nodes, exact class/strictincoming/effectivezero/rawstop/enable/native10/undo; not an authentic mixed endpoint Source witness."]
fn loaded_mixed_is_distinct_and_strict_on_requests() {
    for index in 0..2 {
        for endpoint in [0x3dcc_cccd, 0xbdcc_cccd] {
            let owner = fixture::mixed_current(index, endpoint);
            let mut s = Session::new();
            s.add_owned_native_filtered_flat_archive(fixture::archive(&owner), "mixed-current.pcraft", None, &fixture::ctl()).unwrap();
            let original = s.active().unwrap().doc.clone();
            let ids = original
                .xcf_state
                .as_ref()
                .unwrap()
                .filter_stacks
                .iter()
                .flat_map(|st| st.nodes.iter().filter(|n| n.operation.name == "gegl:exposure").map(|n| n.id))
                .collect::<Vec<_>>();
            assert_eq!(ids.len(), 3);
            for id in ids {
                let before = s.active().unwrap().doc.clone();
                for bits in [0x3dcc_cccd, 0xbdcc_cccd] {
                    assert!(s.execute("xcf.filter.set", black(id, bits)).is_err());
                    assert!(s.active().unwrap().doc.shares_state(&before));
                }
                assert!(s.execute("xcf.filter.set", serde_json::json!({"filterId":id.0.to_string(),"arguments":[],"enabled":false})).is_err());
                s.execute("xcf.filter.set",serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0.75f32.to_bits()}}}],"enabled":false})).unwrap();
                assert_eq!(fixture::words(node(&s, id)), [0.75f32.to_bits(), endpoint]);
                let changed = s.active().unwrap().doc.clone();
                fixture::payload(&before, &changed);
                let mut reopened = Session::new();
                reopened
                    .add_owned_native_filtered_flat_archive(fixture::archive(changed.accounted().unwrap()), "changed.pcraft", None, &fixture::ctl())
                    .unwrap();
                fixture::same(&changed, &reopened.active().unwrap().doc);
                s.execute("xcf.filter.set", black(id, 0)).unwrap();
                assert_eq!(fixture::words(node(&s, id))[1], 0);
                s.execute("edit.undo", serde_json::json!({})).unwrap();
                fixture::same(&changed, &s.active().unwrap().doc);
                s.execute("edit.undo", serde_json::json!({})).unwrap();
                fixture::same(&before, &s.active().unwrap().doc);
            }
        }
    }
}
