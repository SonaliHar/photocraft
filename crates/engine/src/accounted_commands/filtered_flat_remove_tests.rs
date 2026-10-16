//! Deferred topology-safe removal with genuine source/native custody.
use super::*;
#[path = "../accounted_filtered_flat_remove_test_support.rs"]
mod fixture;
fn session(case: &fixture::data::Case) -> Session {
    let input = fixture::import(case);
    let photocraft_io::OwnedXcfImport::FilteredFlat(input) = &input else { panic!("mixed") };
    let mut s = Session::new();
    s.add_owned_filtered_flat_xcf_import(input, "ordered.xcf", Some(case.source.path), &fixture::ctl()).unwrap();
    s
}
#[test]
#[ignore = "UNRUN genuine2 ordered source remove first/middle/last exacttarget→retained2, whole sibling/raw/profile/native/mask/canonical/inert archive custody, onehistory+label/undo/redo/query/native; single-root3→2 only, no producer deleted numeric oracle."]
fn safe_remove_keeps_exact_custody_and_native() {
    for case in &fixture::data::CASES {
        let mut s = session(case);
        let original = s.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let targets = original.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| st.nodes.iter().map(|n| n.id)).collect::<Vec<_>>();
        for id in targets {
            let before = s.active().unwrap().doc.clone();
            let rev = s.active().unwrap().revision;
            let p = serde_json::json!({"filterId":id.0.to_string()});
            assert_eq!(s.execute("xcf.filter.remove", p.clone()).unwrap(), serde_json::json!({"filterId":id.0.to_string(),"updated":true}));
            let removed = s.active().unwrap().doc.clone();
            fixture::preserved(&before, &removed, id);
            assert_eq!(s.active().unwrap().revision, rev + 1);
            assert_eq!(s.active().unwrap().history.entry_labels().last(), Some("Remove GIMP Stock Filter"));
            assert!(s.execute("xcf.filter.remove", p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&removed));
            let query = crate::accounted_xcf_inspect::prepare(&s, &serde_json::json!({"limit":128}), &fixture::ctl()).unwrap();
            assert_eq!(query.value()["filters"]["total"], 8);
            assert!(!query.value()["filters"]["entries"].as_array().unwrap().iter().any(|v| v["id"] == id.0.to_string()));
            drop(query);
            let mut native = Session::new();
            native.add_owned_native_filtered_flat_archive(fixture::archive(removed.accounted().unwrap()), "removed.pcraft", None, &fixture::ctl()).unwrap();
            fixture::same(&removed, &native.active().unwrap().doc);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &s.active().unwrap().doc);
            assert!(s.execute("xcf.filter.remove", serde_json::json!({"filterId":"0"})).is_err());
            assert!(s.active().unwrap().history.can_redo());
            s.execute("edit.redo", serde_json::json!({})).unwrap();
            fixture::same(&removed, &s.active().unwrap().doc);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
        }
        let single = fixture::single_root_three(original.accounted().unwrap());
        let mut s = Session::new();
        s.add_owned_native_filtered_flat_archive(fixture::archive(&single), "single.pcraft", None, &fixture::ctl()).unwrap();
        let id = single.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[2].id;
        s.execute("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
        let two = s.active().unwrap().doc.clone();
        fixture::preserved(single.document(), &two, id);
        let other = two.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        assert!(s.execute("xcf.filter.remove", serde_json::json!({"filterId":other.0.to_string(),"index":0})).is_err());
        assert!(s.active().unwrap().doc.shares_state(&two));
        let mut native = Session::new();
        native.add_owned_native_filtered_flat_archive(fixture::archive(two.accounted().unwrap()), "two.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&two, &native.active().unwrap().doc);
        assert_eq!(fixture::bytes(case.source), std::fs::read(fixture::workspace().join(case.source.path)).unwrap());
    }
}
#[test]
#[ignore = "UNRUN remove exactwire/unknownID/unsupported wire/context/MAX/locks/history-view stale/earlyworkcancel and inactive malformed target proof rollback; no late-render cancellation or emptycohort grant."]
fn safe_remove_refuses_edges_and_stale_context() {
    let case = &fixture::data::CASES[0];
    let mut s = session(case);
    let before = s.active().unwrap().doc.clone();
    let st = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
    let id = st.nodes[1].id;
    let p = serde_json::json!({"filterId":id.0.to_string()});
    for bad in [
        serde_json::json!({"filterId":id.0.to_string(),"index":0}),
        serde_json::json!({"filterId":"0"}),
        serde_json::json!({"filterId":"0001"}),
        serde_json::json!({"filterId":"18446744073709551615"}),
    ] {
        assert!(s.execute("xcf.filter.remove", bad).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    s.active_mut().unwrap().coalesce = Some("pending".into());
    assert!(Request::parse(&s, "xcf.filter.remove", &p, &fixture::ctl()).is_err());
    s.active_mut().unwrap().coalesce = None;
    let prepared = Request::parse(&s, "xcf.filter.remove", &p, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let ItemRef::Layer(root) = st.owner else { panic!("layer") };
    for max in [false, true] {
        let model = fixture::current(before.accounted().unwrap(), |d| {
            if max {
                d.xcf_state.as_mut().unwrap().filter_stacks.iter_mut().find(|s| s.owner == st.owner).unwrap().revision = u64::MAX;
            } else {
                d.layer_mut(root).unwrap().locks.pixels = true;
            }
        });
        let mut s = Session::new();
        s.add_owned_native_filtered_flat_archive(fixture::archive(&model), "locked.pcraft", None, &fixture::ctl()).unwrap();
        let old = s.active().unwrap().doc.clone();
        assert!(s.execute("xcf.filter.remove", p.clone()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&old));
    }
    let mut s = session(case);
    s.active_mut().unwrap().revision = u64::MAX;
    assert!(s.execute("xcf.filter.remove", p.clone()).is_err());
    let s = session(case);
    let work = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &work };
    assert!(Request::parse(&s, "xcf.filter.remove", &p, &c).is_err());
    let cancel = || true;
    let interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(Request::parse(&s, "xcf.filter.remove", &p, &c).is_err());
    let mut mismatch = session(case);
    let active = mismatch.active().unwrap();
    let paid = active.doc.accounted().unwrap().try_with_document_name("paid mismatched history", &fixture::ctl()).unwrap();
    let view = active
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&paid, active.active_layer, active.accounted_view.as_ref().unwrap().selected(), active.layer_anchor, &fixture::ctl())
        .unwrap();
    mismatch.active_mut().unwrap().doc = DocumentSnapshot::from_accounted(paid).unwrap();
    mismatch.active_mut().unwrap().accounted_view = Some(view);
    assert!(Request::parse(&mismatch, "xcf.filter.remove", &p, &fixture::ctl()).unwrap().prepare(&mismatch, &fixture::ctl()).is_err());
    // Paid target-disabled malformed argument cannot be deleted to bypass OLD proof.
    let bad = fixture::current(before.accounted().unwrap(), |d| {
        let n = d.xcf_state.as_mut().unwrap().filter_stacks.iter_mut().flat_map(|s| s.nodes.iter_mut()).find(|n| n.id == id).unwrap();
        n.enabled = false;
        n.arguments[0].value = photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(f32::NAN.to_bits()));
    });
    let mut s = session(case);
    let active = s.active().unwrap();
    let view = active
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&bad, active.active_layer, active.accounted_view.as_ref().unwrap().selected(), active.layer_anchor, &fixture::ctl())
        .unwrap();
    let history = DocumentHistory::new_accounted(bad.clone(), 64, "external malformed current", view.target(), &fixture::ctl()).unwrap();
    let active = s.active_mut().unwrap();
    active.doc = DocumentSnapshot::from_accounted(bad).unwrap();
    active.accounted_view = Some(view);
    active.history = history;
    assert!(Request::parse(&s, "xcf.filter.remove", &p, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).is_err());
}
