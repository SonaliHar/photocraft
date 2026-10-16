//! Deferred declared mixed CURRENT opacity transport/lifecycle, not GIMP arithmetic acceptance.
use super::*;
#[path = "../accounted_filtered_flat_opacity_test_support.rs"]
mod fixture;
fn session(case: &fixture::Case) -> Session {
    let input = fixture::import(case);
    let photocraft_io::OwnedXcfImport::FilteredFlat(input) = &input else { panic!("mixed") };
    let mut s = Session::new();
    s.add_owned_filtered_flat_xcf_import(input, "unit-source.xcf", Some(case.source), &fixture::ctl()).unwrap();
    s
}
fn opacity(id: FilterId, bits: u64) -> Value {
    serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":format!("0x{bits:016x}")})
}
#[test]
#[ignore = "UNRUN four genuine unit-opacity source packets/all6mixedfamilies derivedCURRENT F64opacity compound enable/args/rawsignedzero/native10/noopredo/sourcepreservation; originalsource+1 only, no producer partial-opacity arithmetic oracle."]
fn opacity_compound_and_native_preserve_raw_words() {
    for case in &fixture::CASES {
        let mut s = session(case);
        let original = s.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let ids = original.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| st.nodes.iter().map(|n| n.id)).collect::<Vec<_>>();
        assert_eq!(ids.len(), 15);
        for id in ids {
            let before = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            let bits = 0x3fd8000000000001u64;
            let p = opacity(id, bits);
            assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], true);
            let changed = s.active().unwrap().doc.clone();
            fixture::payload(&before, &changed);
            assert_eq!(s.active().unwrap().revision, revision + 1);
            for (a, b) in before.xcf_state.as_ref().unwrap().filter_stacks.iter().zip(&changed.xcf_state.as_ref().unwrap().filter_stacks) {
                for (old, new) in a.nodes.iter().zip(&b.nodes) {
                    if old.id == id {
                        let mut expected = old.clone();
                        expected.opacity_f64_bits = bits;
                        assert_eq!(&expected, new);
                        assert_eq!(b.revision, a.revision + 1);
                    } else {
                        assert_eq!(old, new);
                    }
                }
            }
            assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], false);
            let mut reopened = Session::new();
            reopened
                .add_owned_native_filtered_flat_archive(fixture::archive(changed.accounted().unwrap()), "partial-current.pcraft", None, &fixture::ctl())
                .unwrap();
            fixture::same(&changed, &reopened.active().unwrap().doc);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &s.active().unwrap().doc);
            assert!(s.active().unwrap().history.can_redo());
            assert_eq!(s.execute("xcf.filter.set", opacity(id, 1f64.to_bits())).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.can_redo());
            for zero in [0u64, (-0.0f64).to_bits()] {
                s.execute("xcf.filter.set", opacity(id, zero)).unwrap();
                let now = s.active().unwrap().doc.clone();
                assert_eq!(now.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.id == id).unwrap().opacity_f64_bits, zero);
                s.execute("edit.undo", serde_json::json!({})).unwrap();
                fixture::same(&before, &s.active().unwrap().doc);
            }
        }
        fixture::rejects_source(case);
        assert_eq!(fixture::bytes(case), std::fs::read(fixture::workspace().join(case.source)).unwrap());
    }
    for rawblack in [0x3dcc_cccd, 0xbdcc_cccd] {
        let model = fixture::loaded_mixed_current(0, rawblack);
        let mut s = Session::new();
        s.add_owned_native_filtered_flat_archive(fixture::archive(&model), "loaded-current.pcraft", None, &fixture::ctl()).unwrap();
        let before = s.active().unwrap().doc.clone();
        let node = before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.operation.name == "gegl:exposure").unwrap();
        let id = node.id;
        let rev = s.active().unwrap().revision;
        let p = serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0x3fd8000000000001","enabled":false,"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0.75f32.to_bits()}}}]});
        s.execute("xcf.filter.set", p.clone()).unwrap();
        let after = s.active().unwrap().doc.clone();
        let new = after.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.id == id).unwrap();
        assert_eq!(new.opacity_f64_bits, 0x3fd8000000000001);
        assert!(!new.enabled);
        assert_eq!(new.arguments.iter().find(|a| a.name == "black-level").unwrap(), node.arguments.iter().find(|a| a.name == "black-level").unwrap());
        assert_eq!(s.active().unwrap().revision, rev + 1);
        assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], false);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&before, &s.active().unwrap().doc);
    }
}
#[test]
#[ignore = "UNRUN canonical opacity finiteunit F64 boundary and parser refusals, ordinaryPoint/source+1 remainsclosed, locked equal/noop and view/history/preparedcommitWork rollback; malformed disabled opacityzero OLD cannot bypass fullleaf proof. No late-render cancellation or opacity math claim."]
fn opacity_refusals_are_atomic_and_fullproof() {
    for text in ["0x0000000000000000", "0x8000000000000000", "0x0000000000000001", "0x3ff0000000000000"] {
        assert!(opacity_bits(&serde_json::json!(text)).is_some());
    }
    for text in [
        "0x3ff0000000000001",
        "0xbff0000000000000",
        "0x7ff0000000000000",
        "0x7ff8000000000000",
        "0X3ff0000000000000",
        "0x3FF0000000000000",
        "0x0",
        "0x00000000000000000",
    ] {
        assert!(opacity_bits(&serde_json::json!(text)).is_none());
    }
    let case = &fixture::CASES[0];
    let mut s = session(case);
    let before = s.active().unwrap().doc.clone();
    let stack = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
    let id = stack.nodes[0].id;
    for bad in [
        serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":0.5}),
        serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0x3fe0000000000000","arguments":[]}),
    ] {
        assert!(s.execute("xcf.filter.set", bad).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    assert!(s.execute("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":true,"opacityF64Bits":"0x3fe0000000000000"})).is_err());
    let point = fixture::ordinary_point();
    let photocraft_io::OwnedXcfImport::Point(point) = &point else { panic!("point") };
    let mut p = Session::new();
    p.add_owned_point_xcf_import(point, "point.xcf", None, &fixture::ctl()).unwrap();
    let initial = p.active().unwrap().doc.clone();
    let pointid = initial.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
    assert!(p.execute("xcf.filter.set", opacity(pointid, 0.5f64.to_bits())).is_err());
    assert!(p.active().unwrap().doc.shares_state(&initial));
    let request = opacity(id, 0.5f64.to_bits());
    let prepared = Request::parse(&s, "xcf.filter.set", &request, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).unwrap();
    s.active_mut().unwrap().view_revision += 1;
    assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    let ItemRef::Layer(root) = stack.owner else { panic!("root") };
    let locked = fixture::current(before.accounted().unwrap(), |d| d.layer_mut(root).unwrap().locks.pixels = true);
    let mut s = Session::new();
    s.add_owned_native_filtered_flat_archive(fixture::archive(&locked), "locked.pcraft", None, &fixture::ctl()).unwrap();
    let old = s.active().unwrap().doc.clone();
    assert!(s.execute("xcf.filter.set", opacity(id, 1f64.to_bits())).is_err());
    assert!(s.active().unwrap().doc.shares_state(&old));
    let mut mismatch = session(case);
    let state = mismatch.active().unwrap();
    let paid = state.doc.accounted().unwrap().try_with_document_name("paid history mismatch", &fixture::ctl()).unwrap();
    let view = state
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&paid, state.active_layer, state.accounted_view.as_ref().unwrap().selected(), state.layer_anchor, &fixture::ctl())
        .unwrap();
    mismatch.active_mut().unwrap().doc = DocumentSnapshot::from_accounted(paid).unwrap();
    mismatch.active_mut().unwrap().accounted_view = Some(view);
    assert!(Request::parse(&mismatch, "xcf.filter.set", &request, &fixture::ctl()).unwrap().prepare(&mismatch, &fixture::ctl()).is_err());
    let visible_lock = fixture::current(before.accounted().unwrap(), |d| {
        d.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(root)).unwrap().locks.visibility = true
    });
    let mut v = Session::new();
    v.add_owned_native_filtered_flat_archive(fixture::archive(&visible_lock), "visibility-locked.pcraft", None, &fixture::ctl()).unwrap();
    v.execute("xcf.filter.set", opacity(id, 0.5f64.to_bits())).unwrap();
    let valid = v.active().unwrap().doc.clone();
    assert!(v.execute("xcf.filter.set", serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0x3fe0000000000000","enabled":false})).is_err());
    assert!(v.active().unwrap().doc.shares_state(&valid));
    // Valid OLD, invalid staged full leaf: zero/disabled cannot hide overflow.
    let mut late = session(case);
    let exposure = before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.operation.name == "gegl:exposure").unwrap().id;
    let late_before = late.active().unwrap().doc.clone();
    for enabled in [false, true] {
        assert!(late.execute("xcf.filter.set",serde_json::json!({"filterId":exposure.0.to_string(),"opacityF64Bits":"0x0000000000000000","enabled":enabled,"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":f32::MAX.to_bits()}}}]})).is_err());
        assert!(late.active().unwrap().doc.shares_state(&late_before));
    }
    let good = opacity(id, 0.5f64.to_bits());
    let prepared = Request::parse(&late, "xcf.filter.set", &good, &fixture::ctl()).unwrap().prepare(&late, &fixture::ctl()).unwrap();
    let deny = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
    assert!(prepared.execute(&mut late, &c).is_err());
    assert!(late.active().unwrap().doc.shares_state(&late_before));
    // Synthetic invalid CURRENT metadata, real prepaid retained native/source owner.
    let bad = fixture::current(before.accounted().unwrap(), |d| {
        let n = d.xcf_state.as_mut().unwrap().filter_stacks.iter_mut().flat_map(|s| s.nodes.iter_mut()).find(|n| n.operation.name == "gegl:exposure").unwrap();
        n.enabled = false;
        n.opacity_f64_bits = 0;
        n.arguments.iter_mut().find(|a| a.name == "exposure").unwrap().value =
            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(f32::MAX.to_bits()));
    });
    let mut invalid = session(case);
    let state = invalid.active().unwrap();
    let view = state
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&bad, state.active_layer, state.accounted_view.as_ref().unwrap().selected(), state.layer_anchor, &fixture::ctl())
        .unwrap();
    let history = DocumentHistory::new_accounted(bad.clone(), 64, "malformed zero CURRENT", view.target(), &fixture::ctl()).unwrap();
    let state = invalid.active_mut().unwrap();
    state.doc = DocumentSnapshot::from_accounted(bad).unwrap();
    state.accounted_view = Some(view);
    state.history = history;
    let unchanged = invalid.active().unwrap().doc.clone();
    assert!(Request::parse(&invalid, "xcf.filter.set", &opacity(id, 1f64.to_bits()), &fixture::ctl()).unwrap().prepare(&invalid, &fixture::ctl()).is_err());
    assert!(invalid.active().unwrap().doc.shares_state(&unchanged));
}
