//! Deferred move transport/custody; no producer arithmetic oracle.
use super::*;
#[path = "../accounted_filtered_flat_ordered_test_support.rs"]
mod fixture;
fn session(case: &fixture::data::Case) -> Session {
    let input = fixture::import(case);
    let photocraft_io::OwnedXcfImport::FilteredFlat(input) = &input else { panic!("mixed") };
    let mut s = Session::new();
    s.add_owned_filtered_flat_xcf_import(input, "ordered.xcf", Some(case.source.path), &fixture::ctl()).unwrap();
    s
}
#[test]
#[ignore = "UNRUN genuine ordered2 inputs exact whole-node owner-local permutation, allroots/siblings/raw/masks/profiles/source, onehistory+label/noopredo/native query continuity. Declared lower compatibility only; no producer reordered numeric oracle."]
fn mixed_move_preserves_whole_nodes_and_native_continuity() {
    for case in &fixture::data::CASES {
        let mut s = session(case);
        let original = s.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let targets = original.xcf_state.as_ref().unwrap().filter_stacks.iter().map(|st| (st.owner, st.nodes[2].id)).collect::<Vec<_>>();
        for (owner, id) in targets {
            let before = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            let old = before.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == owner).unwrap();
            let params = serde_json::json!({"filterId":id.0.to_string(),"index":0});
            assert_eq!(s.execute("xcf.filter.move", params.clone()).unwrap(), serde_json::json!({"filterId":id.0.to_string(),"updated":true}));
            let moved = s.active().unwrap().doc.clone();
            assert_eq!(s.active().unwrap().history.entry_labels().last(), Some("Move GIMP Stock Filter"));
            assert_eq!(s.active().unwrap().revision, revision + 1);
            fixture::payload(&before, &moved);
            for (a, b) in before.xcf_state.as_ref().unwrap().filter_stacks.iter().zip(&moved.xcf_state.as_ref().unwrap().filter_stacks) {
                if a.owner == owner {
                    assert_eq!(b.revision, a.revision + 1);
                    assert_eq!(b.nodes, vec![a.nodes[2].clone(), a.nodes[0].clone(), a.nodes[1].clone()]);
                } else {
                    assert_eq!(a, b)
                }
            }
            let query = crate::accounted_xcf_inspect::prepare(&s, &serde_json::json!({"limit":128}), &fixture::ctl()).unwrap();
            let expected =
                moved.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| st.nodes.iter().map(|n| n.id.0.to_string())).collect::<Vec<_>>();
            let actual = query.value()["filters"]["entries"].as_array().unwrap().iter().map(|n| n["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
            assert_eq!(actual, expected);
            drop(query);
            let mut reopened = Session::new();
            reopened.add_owned_native_filtered_flat_archive(fixture::archive(moved.accounted().unwrap()), "moved.pcraft", None, &fixture::ctl()).unwrap();
            fixture::same(&moved, &reopened.active().unwrap().doc);
            assert_eq!(s.execute("xcf.filter.move", params).unwrap()["updated"], false);
            assert!(s.active().unwrap().doc.shares_state(&moved));
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &s.active().unwrap().doc);
            assert_eq!(s.execute("xcf.filter.move", serde_json::json!({"filterId":id.0.to_string(),"index":2})).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.can_redo());
            s.execute("edit.redo", serde_json::json!({})).unwrap();
            fixture::same(&moved, &s.active().unwrap().doc);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            assert_eq!(old.nodes[2].id, id);
        }
        assert_eq!(fixture::bytes(case.source), std::fs::read(fixture::workspace().join(case.source.path)).unwrap());
    }
}
#[test]
#[ignore = "UNRUN mixed move exactwire1024 refusal/out-of-stack, pendingcoalesce/locks/MAX/history+view stale and earlyWork/cancel beforeequal noop; full otherrootinactive malformed CURRENT oldproof refusal. No late-render cancellation claim."]
fn mixed_move_equal_requests_keep_all_fences() {
    let case = &fixture::data::CASES[0];
    let mut s = session(case);
    let before = s.active().unwrap().doc.clone();
    let stack = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
    let id = stack.nodes[0].id;
    let equal = serde_json::json!({"filterId":id.0.to_string(),"index":0});
    for bad in [
        serde_json::json!({"filterId":id.0.to_string(),"index":3}),
        serde_json::json!({"filterId":id.0.to_string(),"index":1024}),
        serde_json::json!({"filterId":id.0.to_string(),"index":-1}),
        serde_json::json!({"filterId":id.0.to_string(),"index":0,"enabled":true}),
    ] {
        assert!(s.execute("xcf.filter.move", bad).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    s.active_mut().unwrap().coalesce = Some("pending gesture".into());
    assert!(Request::parse(&s, "xcf.filter.move", &equal, &fixture::ctl()).is_err());
    s.active_mut().unwrap().coalesce = None;
    let prepared = Request::parse(&s, "xcf.filter.move", &equal, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let mut s = session(case);
    s.active_mut().unwrap().revision = u64::MAX;
    assert!(s.execute("xcf.filter.move", equal.clone()).is_err());
    let s = session(case);
    let denied = |_: u64| Err(BackingError::Work);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &denied };
    assert!(Request::parse(&s, "xcf.filter.move", &equal, &c).is_err());
    let cancelled = || true;
    let interrupt = photocraft_raster::Interrupt::cancel_only(&cancelled);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(Request::parse(&s, "xcf.filter.move", &equal, &c).is_err());
    // This paid native CURRENT variant changes only an inherited native owner lock.
    #[path = "../../../ui-egui/src/filtered_flat_fixture.rs"]
    mod native;
    let ItemRef::Layer(root) = stack.owner else { panic!("root") };
    let locked = native::current(before.accounted().unwrap(), |d| d.layer_mut(root).unwrap().locks.pixels = true);
    let mut s = Session::new();
    s.add_owned_native_filtered_flat_archive(fixture::archive(&locked), "locked.pcraft", None, &fixture::ctl()).unwrap();
    let locked_before = s.active().unwrap().doc.clone();
    assert!(s.execute("xcf.filter.move", equal.clone()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&locked_before));
    // Actual paid title copy with coherent view but original history: equal move refuses.
    let mut s = session(case);
    let st = s.active().unwrap();
    let title = st.doc.accounted().unwrap().try_with_document_name("history mismatch", &fixture::ctl()).unwrap();
    let view = st
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&title, st.active_layer, st.accounted_view.as_ref().unwrap().selected(), st.layer_anchor, &fixture::ctl())
        .unwrap();
    s.active_mut().unwrap().accounted_view = Some(view);
    s.active_mut().unwrap().doc = DocumentSnapshot::from_accounted(title).unwrap();
    assert!(Request::parse(&s, "xcf.filter.move", &equal, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).is_err());
    let mut s = session(case);
    let source = s.active().unwrap().doc.clone();
    let bad = native::current(source.accounted().unwrap(), |d| {
        let other = d.layers[3].id;
        d.layers[3].visible = false;
        let x = d.xcf_state.as_mut().unwrap();
        x.items.iter_mut().find(|i| i.item == ItemRef::Layer(other)).unwrap().visible = false;
        let st = x.filter_stacks.iter_mut().find(|st| st.owner == ItemRef::Layer(other)).unwrap();
        st.nodes[0].enabled = false;
        st.nodes[0].arguments[0].value =
            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(f32::NAN.to_bits()));
    });
    let st = s.active().unwrap();
    let view = st
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&bad, st.active_layer, st.accounted_view.as_ref().unwrap().selected(), st.layer_anchor, &fixture::ctl())
        .unwrap();
    let history = DocumentHistory::new_accounted(bad.clone(), 64, "external invalid current", view.target(), &fixture::ctl()).unwrap();
    let st = s.active_mut().unwrap();
    st.doc = DocumentSnapshot::from_accounted(bad).unwrap();
    st.accounted_view = Some(view);
    st.history = history;
    assert!(Request::parse(&s, "xcf.filter.move", &equal, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).is_err());
    let max = native::current(before.accounted().unwrap(), |d| {
        d.xcf_state.as_mut().unwrap().filter_stacks.iter_mut().find(|st| st.owner == stack.owner).unwrap().revision = u64::MAX
    });
    let mut s = Session::new();
    s.add_owned_native_filtered_flat_archive(fixture::archive(&max), "max.pcraft", None, &fixture::ctl()).unwrap();
    let max_before = s.active().unwrap().doc.clone();
    assert_ne!(s.active().unwrap().revision, u64::MAX);
    for index in [0, 1] {
        assert!(s.execute("xcf.filter.move", serde_json::json!({"filterId":id.0.to_string(),"index":index})).is_err());
        assert!(s.active().unwrap().doc.shares_state(&max_before));
    }
}
