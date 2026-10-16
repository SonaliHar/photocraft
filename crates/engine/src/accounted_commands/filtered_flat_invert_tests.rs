//! Deferred real mixed invert transport/native lifecycle; no producer numeric oracle.
use super::*;
#[path = "../accounted_filtered_flat_invert_test_support.rs"]
mod fixture;
fn session(case: &fixture::Case) -> Session {
    let input = fixture::import(case);
    let photocraft_io::OwnedXcfImport::FilteredFlat(input) = &input else { panic!("mixed") };
    let mut s = Session::new();
    s.add_owned_filtered_flat_xcf_import(input, "inverts.xcf", Some(case.source), &fixture::ctl()).unwrap();
    s
}
#[test]
#[ignore = "UNRUN all four authentic source packets/eight indexed invert targets, zero-args wire boundaries, fullproof activation/noopredo/query/move/remove and marker10 native continuation; no producer numeric/profile oracle."]
fn invert_targets_and_empty_wire_continue_native() {
    for case in &fixture::CASES {
        let mut s = session(case);
        let original = s.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let targets = original
            .xcf_state
            .as_ref()
            .unwrap()
            .filter_stacks
            .iter()
            .flat_map(|st| st.nodes.iter().filter(|n| matches!(n.operation.name.as_str(), "gegl:invert-linear" | "gegl:invert-gamma")).map(|n| n.id))
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 8);
        for id in &targets {
            let before = s.active().unwrap().doc.clone();
            let p = serde_json::json!({"filterId":id.0.to_string(),"enabled":false});
            for bad in [
                serde_json::json!({"filterId":id.0.to_string()}),
                serde_json::json!({"filterId":id.0.to_string(),"arguments":[]}),
                serde_json::json!({"filterId":id.0.to_string(),"arguments":[],"enabled":false}),
                serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0x3ff0000000000001"}),
                serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"value","value":{"Number":{"Float32Bits":0}}}]}),
                serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0xbff0000000000000"}),
            ] {
                assert!(s.execute("xcf.filter.set", bad).is_err());
                assert!(s.active().unwrap().doc.shares_state(&before));
            }
            // Inner wire broader than registered mixed outer parser; no authority alias.
            assert!(wire_with_literal("xcf.filter.set", &serde_json::json!({"filterId":id.0.to_string(),"arguments":[],"enabled":false}), true).is_some());
            assert!(
                wire_with_literal("xcf.filter.set", &serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0x3ff0000000000000"}), true).is_some()
            );
            assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], true);
            let after = s.active().unwrap().doc.clone();
            fixture::payload(&before, &after);
            for (a, b) in before.xcf_state.as_ref().unwrap().filter_stacks.iter().zip(&after.xcf_state.as_ref().unwrap().filter_stacks) {
                for (old, new) in a.nodes.iter().zip(&b.nodes) {
                    if old.id == *id {
                        let mut expected = old.clone();
                        expected.enabled = false;
                        assert_eq!(&expected, new);
                    } else {
                        assert_eq!(old, new);
                    }
                }
            }
            assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], false);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &s.active().unwrap().doc);
            assert_eq!(s.execute("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":true})).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.can_redo());
            s.execute("edit.redo", serde_json::json!({})).unwrap();
            let q = crate::accounted_filter_inspect::prepare(&s, &serde_json::json!({"filterId":id.0.to_string()}), &fixture::ctl()).unwrap();
            assert_eq!(q.value()["scalarEditCandidate"], true);
            drop(q);
            let mut reopened = Session::new();
            reopened.add_owned_native_filtered_flat_archive(fixture::archive(after.accounted().unwrap()), "changed.pcraft", None, &fixture::ctl()).unwrap();
            fixture::same(&after, &reopened.active().unwrap().doc);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
        }
        let root = original.layers[7].id;
        let stack = original.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == ItemRef::Layer(root)).unwrap();
        let id = stack.nodes.last().unwrap().id;
        s.execute("xcf.filter.move", serde_json::json!({"filterId":id.0.to_string(),"index":0})).unwrap();
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&original, &s.active().unwrap().doc);
        // Actual command removals retain one invert, then zero, under marked CURRENT.
        let keep = targets[0];
        let all = original.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| st.nodes.iter().map(|n| n.id)).collect::<Vec<_>>();
        for id in all {
            if id != keep {
                s.execute("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
            }
        }
        let one = s.active().unwrap().doc.clone();
        let mut native = Session::new();
        native.add_owned_native_filtered_flat_archive(fixture::archive(one.accounted().unwrap()), "one.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&one, &native.active().unwrap().doc);
        s.execute("xcf.filter.remove", serde_json::json!({"filterId":keep.0.to_string()})).unwrap();
        let empty = s.active().unwrap().doc.clone();
        let mut native = Session::new();
        native.add_owned_native_filtered_flat_archive(fixture::archive(empty.accounted().unwrap()), "empty.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&empty, &native.active().unwrap().doc);
        assert_eq!(fixture::bytes(case), std::fs::read(fixture::workspace().join(case.source)).unwrap());
    }
}
#[test]
#[ignore = "UNRUN exact mixed versions/epochs/zeroargs, paid sole-invert marked/native strict refusal, old singleton epoch priority, inactive malformed other-node fullOLD equal-noop refusal, view/history/context and cumulative postparse Work and early cancellation atomicity; no source or numerical theorem."]
fn invert_guards_and_fullproof_refuse_atomically() {
    for index in 0..2 {
        let input = fixture::old_point_baseline(index);
        assert!(input.document().is_readonly_point_storage());
        let archive = fixture::unmarked_archive(input.document());
        assert!(!crate::accounted_native_open::is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
        let mut old = Session::new();
        old.add_owned_native_point_archive(archive, "old-invert.pcraft", None, &fixture::ctl()).unwrap();
        assert!(old.active().unwrap().doc.accounted().unwrap().is_readonly_point_storage());
    }
    let case = &fixture::CASES[0];
    let mut s = session(case);
    let before = s.active().unwrap().doc.clone();
    let node = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
    let id = node.id;
    assert_eq!(family(&node), Some(4));
    for edit in 0..3 {
        let mut wrong = node.clone();
        match edit {
            0 => wrong.operation.stored_version = "1:0".into(),
            1 => wrong.operation.schema_epoch = photocraft_compose::INVERT_GAMMA_MIXED_EPOCH.into(),
            _ => wrong.composition.as_mut().unwrap().compatibility_epoch = photocraft_compose::INVERT_GAMMA_MIXED_EPOCH.into(),
        }
        assert!(!valid_node(&wrong));
    }
    // Paid synthetic CURRENT retains real original provider/fees, not a Source witness.
    let sole = fixture::derived_current(before.accounted().unwrap(), |d| {
        let root = d.layers[1].id;
        d.layers.retain(|l| l.id == root);
        let x = d.xcf_state.as_mut().unwrap();
        x.filter_stacks.retain(|st| st.owner == ItemRef::Layer(root));
        let masks = x.filter_stacks[0].nodes.iter().filter_map(|n| n.mask).collect::<Vec<_>>();
        x.filter_masks.retain(|m| masks.contains(&ItemRef::Mask(m.id)));
        x.items.retain(|i| i.item == ItemRef::Layer(root) || masks.contains(&i.item));
        x.opaque.retain(|r| r.owner.is_none() || r.owner == Some(ItemRef::Layer(root)) || r.owner.is_some_and(|i| masks.contains(&i)));
        x.selection.selected.clear();
        x.selection.selected.push(ItemRef::Layer(root));
        x.selection.active = Some(ItemRef::Layer(root));
    });
    let raw = fixture::unmarked_archive(&sole);
    assert!(crate::accounted_native_open::is_owned_current_filtered_flat_candidate(&raw, &fixture::ctl()).unwrap());
    let mut strict = Session::new();
    assert!(strict.add_owned_native_filtered_flat_archive(raw, "unmarked.pcraft", None, &fixture::ctl()).is_err());
    assert!(strict.active().is_none());
    strict.add_owned_native_filtered_flat_archive(fixture::archive(&sole), "marked.pcraft", None, &fixture::ctl()).unwrap();
    let old = fixture::derived_current(&sole, |d| {
        let n = &mut d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        n.operation.schema_epoch = "xcf-22-stock-invert-linear-precision-v1".into();
        n.composition.as_mut().unwrap().compatibility_epoch = "xcf-22-stock-invert-linear-precision-v1".into();
    });
    assert!(!crate::accounted_native_open::is_current_filtered_flat_candidate(old.document(), &fixture::ctl()).unwrap());
    let p = serde_json::json!({"filterId":id.0.to_string(),"enabled":true});
    let prepared = Request::parse(&s, "xcf.filter.enable", &p, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let bad = fixture::current(before.accounted().unwrap(), |d| {
        let n = d
            .xcf_state
            .as_mut()
            .unwrap()
            .filter_stacks
            .iter_mut()
            .flat_map(|st| st.nodes.iter_mut())
            .find(|n| n.operation.name == "gegl:invert-gamma")
            .unwrap();
        n.enabled = false;
        n.operation.stored_version = "1:0".into();
    });
    let mut invalid = session(case);
    let state = invalid.active().unwrap();
    let view = state
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&bad, state.active_layer, state.accounted_view.as_ref().unwrap().selected(), state.layer_anchor, &fixture::ctl())
        .unwrap();
    let history = DocumentHistory::new_accounted(bad.clone(), 64, "malformed CURRENT", view.target(), &fixture::ctl()).unwrap();
    let state = invalid.active_mut().unwrap();
    state.doc = DocumentSnapshot::from_accounted(bad).unwrap();
    state.accounted_view = Some(view);
    state.history = history;
    assert!(Request::parse(&invalid, "xcf.filter.enable", &p, &fixture::ctl()).unwrap().prepare(&invalid, &fixture::ctl()).is_err());
    let mut mismatch = session(case);
    let state = mismatch.active().unwrap();
    let paid = state.doc.accounted().unwrap().try_with_document_name("paid mismatch", &fixture::ctl()).unwrap();
    let view = state
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&paid, state.active_layer, state.accounted_view.as_ref().unwrap().selected(), state.layer_anchor, &fixture::ctl())
        .unwrap();
    mismatch.active_mut().unwrap().doc = DocumentSnapshot::from_accounted(paid).unwrap();
    mismatch.active_mut().unwrap().accounted_view = Some(view);
    assert!(Request::parse(&mismatch, "xcf.filter.enable", &p, &fixture::ctl()).unwrap().prepare(&mismatch, &fixture::ctl()).is_err());
    let locked = fixture::current(before.accounted().unwrap(), |d| d.layers[1].locks.pixels = true);
    let mut locked_session = Session::new();
    locked_session.add_owned_native_filtered_flat_archive(fixture::archive(&locked), "locked.pcraft", None, &fixture::ctl()).unwrap();
    let locked_before = locked_session.active().unwrap().doc.clone();
    assert!(locked_session.execute("xcf.filter.enable", p.clone()).is_err());
    assert!(locked_session.active().unwrap().doc.shares_state(&locked_before));
    let mut max = session(case);
    max.active_mut().unwrap().revision = u64::MAX;
    assert!(max.execute("xcf.filter.enable", p.clone()).is_err());
    let s = session(case);
    let cumulative_before = s.active().unwrap().doc.clone();
    let work =
        photocraft_io::accounted_operation_work::OperationWork::standard(&photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap()).unwrap();
    let debit = |n| work.check(n);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &debit };
    let request = Request::parse(&s, "xcf.filter.enable", &p, &c).unwrap();
    work.check((1u64 << 40) - work.diagnostics().used).unwrap();
    assert!(request.prepare(&s, &c).is_err());
    assert!(s.active().unwrap().doc.shares_state(&cumulative_before));
    let cancel = || true;
    let interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(Request::parse(&s, "xcf.filter.enable", &p, &c).is_err());
}
