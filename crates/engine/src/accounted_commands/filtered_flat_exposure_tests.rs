//! Deferred exact Exposure1:0 mixed transport/transactions, no producer RGB oracle.
use super::*;
#[path = "../accounted_filtered_flat_exposure_test_support.rs"]
mod fixture;
fn session(case: &fixture::Case) -> Session {
    let input = fixture::import(case);
    let photocraft_io::OwnedXcfImport::FilteredFlat(input) = &input else { panic!("mixed") };
    let mut s = Session::new();
    s.add_owned_filtered_flat_xcf_import(input, "exposure.xcf", Some(case.source), &fixture::ctl()).unwrap();
    s
}
#[test]
#[ignore = "UNRUN bothactual8x6 mixed sources/threeExposurepositions, exact1:0+newEPOCH/twoF32/rawnative/source/masks, atomicpartial+activation/history/noopredo/nativequery, derivedExposure-only unmarkedstrict hint and emptyCurrent10 continuation. No producer numerical/profile/internalformat oracle."]
fn exposure_mixed_targets_use_exact_family_and_native_custody() {
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
            .flat_map(|st| st.nodes.iter().enumerate().filter(|(_, n)| n.operation.name == "gegl:exposure").map(move |(at, n)| (st.owner, at, n.id)))
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 3);
        for (owner, at, id) in &targets {
            let before = s.active().unwrap().doc.clone();
            let rev = s.active().unwrap().revision;
            let p = serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"black-level","value":{"Number":{"Float32Bits":(-0.03125f32).to_bits()}}},{"name":"exposure","value":{"Number":{"Float32Bits":0.75f32.to_bits()}}}],"enabled":false});
            assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], true);
            let after = s.active().unwrap().doc.clone();
            fixture::payload(&before, &after);
            assert_eq!(s.active().unwrap().revision, rev + 1);
            for (a, b) in before.xcf_state.as_ref().unwrap().filter_stacks.iter().zip(&after.xcf_state.as_ref().unwrap().filter_stacks) {
                if a.owner == *owner {
                    assert_eq!(b.revision, a.revision + 1);
                    for (index, (old, new)) in a.nodes.iter().zip(&b.nodes).enumerate() {
                        if index == *at {
                            assert_eq!(old.id, new.id);
                            assert!(!new.enabled);
                            assert_eq!(
                                old.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
                                new.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>()
                            );
                            assert_eq!(family(new), Some(3));
                        } else {
                            assert_eq!(old, new);
                        }
                    }
                } else {
                    assert_eq!(a, b);
                }
            }
            let query = crate::accounted_filter_inspect::prepare(&s, &serde_json::json!({"filterId":id.0.to_string()}), &fixture::ctl()).unwrap();
            assert_eq!(query.value()["scalarEditCandidate"], true);
            assert_eq!(query.value()["enabled"], false);
            drop(query);
            assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], false);
            assert!(s.active().unwrap().doc.shares_state(&after));
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &s.active().unwrap().doc);
            assert_eq!(s.execute("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":true})).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.can_redo());
            s.execute("edit.redo", serde_json::json!({})).unwrap();
            fixture::same(&after, &s.active().unwrap().doc);
            s.execute("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":true})).unwrap();
            let active = s.active().unwrap().doc.clone();
            let mut native = Session::new();
            native.add_owned_native_filtered_flat_archive(fixture::archive(active.accounted().unwrap()), "changed.pcraft", None, &fixture::ctl()).unwrap();
            fixture::same(&active, &native.active().unwrap().doc);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &s.active().unwrap().doc);
        }
        // Actual command-derived CURRENT (not fabricated original): retain only root1 Exposure.
        let keep = targets[0].2;
        let all = original.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| st.nodes.iter().map(|n| n.id)).collect::<Vec<_>>();
        for id in &all {
            if *id != keep {
                s.execute("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
            }
        }
        let only = s.active().unwrap().doc.clone();
        let unmarked = fixture::unmarked_archive(only.accounted().unwrap());
        assert!(crate::accounted_native_open::is_owned_current_filtered_flat_candidate(&unmarked, &fixture::ctl()).unwrap());
        let mut strict = Session::new();
        assert!(strict.add_owned_native_filtered_flat_archive(unmarked, "unmarked.pcraft", None, &fixture::ctl()).is_err());
        assert!(strict.active().is_none());
        s.execute("xcf.filter.remove", serde_json::json!({"filterId":keep.0.to_string()})).unwrap();
        let empty = s.active().unwrap().doc.clone();
        let mut native = Session::new();
        native.add_owned_native_filtered_flat_archive(fixture::archive(empty.accounted().unwrap()), "empty.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&empty, &native.active().unwrap().doc);
        let pair = fixture::exposure_pair(original.accounted().unwrap());
        let mut singleton = Session::new();
        singleton.add_owned_native_filtered_flat_archive(fixture::archive(&pair), "pair.pcraft", None, &fixture::ctl()).unwrap();
        let semi = pair.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[1].id;
        singleton.execute("xcf.filter.remove", serde_json::json!({"filterId":semi.0.to_string()})).unwrap();
        let one = singleton.active().unwrap().doc.clone();
        assert_eq!(one.layers.len(), 1);
        assert_eq!(one.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), 1);
        let unmarked = fixture::unmarked_archive(one.accounted().unwrap());
        assert!(crate::accounted_native_open::is_owned_current_filtered_flat_candidate(&unmarked, &fixture::ctl()).unwrap());
        let mut strict = Session::new();
        assert!(strict.add_owned_native_filtered_flat_archive(unmarked, "unmarked-single.pcraft", None, &fixture::ctl()).is_err());
        assert!(strict.active().is_none());
        strict.add_owned_native_filtered_flat_archive(fixture::archive(one.accounted().unwrap()), "marked-single.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&one, &strict.active().unwrap().doc);
        assert_eq!(fixture::bytes(case), std::fs::read(fixture::workspace().join(case.source)).unwrap());
    }
}
#[test]
#[ignore = "UNRUN promoted-f64 blackendpoint inward/outward bits, finiteExposure noarbitrarycap but fullkernel overflow/denominator refusal, signedzeroidentity, exactversion/epoch/types/duplicates, actualviewstale/MAX andearlyWork/cancel atomicity. No .1sourceword waiver or latekernel cancellationclaim."]
fn exposure_domains_and_family_guards_are_exact() {
    assert!(number_in_domain(3, "black-level", 0x3dcc_cccc));
    assert!(number_in_domain(3, "black-level", 0xbdcc_cccc));
    assert!(!number_in_domain(3, "black-level", 0.1f32.to_bits()));
    assert!(!number_in_domain(3, "black-level", (-0.1f32).to_bits()));
    assert!(number_in_domain(3, "exposure", f32::MAX.to_bits()));
    assert!(!number_in_domain(3, "exposure", f32::NAN.to_bits()));
    for index in 0..2 {
        let old = fixture::old_point_baseline(index);
        let owner = old.document();
        assert!(owner.is_readonly_point_storage());
        let archive = fixture::unmarked_archive(owner);
        assert!(!crate::accounted_native_open::is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
        let mut original = Session::new();
        original.add_owned_native_point_archive(archive, "old-point.pcraft", None, &fixture::ctl()).unwrap();
        assert!(original.active().unwrap().doc.accounted().unwrap().is_readonly_point_storage());
    }
    let case = &fixture::CASES[0];
    let mut s = session(case);
    let before = s.active().unwrap().doc.clone();
    let node = before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| &st.nodes).find(|n| n.operation.name == "gegl:exposure").unwrap();
    let id = node.id;
    for (name, bits) in
        [("black-level", 0.1f32.to_bits()), ("black-level", (-0.1f32).to_bits()), ("black-level", f32::NAN.to_bits()), ("exposure", f32::MAX.to_bits())]
    {
        let p = serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":name,"value":{"Number":{"Float32Bits":bits}}}]});
        assert!(s.execute("xcf.filter.set", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    for p in [
        serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"exposure","value":{"Number":{"Float64Bits":0}}}]}),
        serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0}}},{"name":"exposure","value":{"Number":{"Float32Bits":0}}}]}),
        serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"unknown","value":{"Number":{"Float32Bits":0}}}]}),
    ] {
        assert!(s.execute("xcf.filter.set", p).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    // A disabled but fully qualified finite scale above an invented +/-20 UI
    // clamp remains a legal CURRENT transaction; no HDR output oracle follows.
    s.execute("xcf.filter.set",serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":64f32.to_bits()}}},{"name":"black-level","value":{"Number":{"Float32Bits":0f32.to_bits()}}}],"enabled":false})).unwrap();
    s.execute("edit.undo", serde_json::json!({})).unwrap();
    fixture::same(&before, &s.active().unwrap().doc);
    let mut wrong = node.clone();
    wrong.operation.stored_version = "0:0".into();
    assert_eq!(family(&wrong), None);
    wrong = node.clone();
    wrong.operation.schema_epoch = "xcf-22-stock-exposure-black-level-precision-v1".into();
    assert_eq!(family(&wrong), None);
    for old in before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).filter(|n| n.operation.name != "gegl:exposure") {
        assert!(family(old).is_some());
        let mut wrong = old.clone();
        wrong.operation.stored_version = "1:0".into();
        assert_eq!(family(&wrong), None);
    }
    let p = serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":(-0.0f32).to_bits()}}},{"name":"black-level","value":{"Number":{"Float32Bits":(-0.0f32).to_bits()}}}]});
    let prepared = Request::parse(&s, "xcf.filter.set", &p, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let mut s = session(case);
    s.execute("xcf.filter.set", p.clone()).unwrap();
    let n = s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| &st.nodes).find(|n| n.id == id).unwrap();
    assert!(
        n.arguments
            .iter()
            .all(|a| a.value == photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits((-0.0f32).to_bits())))
    );
    let old = s.active().unwrap().doc.clone();
    assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], false);
    assert!(s.active().unwrap().doc.shares_state(&old));
    s.active_mut().unwrap().revision = u64::MAX;
    assert!(s.execute("xcf.filter.set", p.clone()).is_err());
    let s = session(case);
    let deny = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
    assert!(Request::parse(&s, "xcf.filter.set", &p, &c).is_err());
    let cancel = || true;
    let interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(Request::parse(&s, "xcf.filter.set", &p, &c).is_err());
}
