//! Genuine-source masked-mixed transaction contracts, all UNRUN; no producer pixel oracle.
use super::*;
// Shared fixture is intentionally instantiated in this test namespace.
#[allow(clippy::duplicate_mod)]
#[path = "../accounted_masked_filtered_flat_test_support.rs"]
mod fixture;
fn session(c: &serde_json::Value) -> Session {
    let i = fixture::import(c);
    let photocraft_io::OwnedXcfImport::MaskedFilteredFlat(i) = &i else { panic!("sealed masked mixed") };
    let mut s = Session::new();
    s.add_owned_masked_filtered_flat_xcf_import(i, "masked.xcf", fixture::path(c).to_str(), &fixture::ctl()).unwrap();
    s
}
fn targets(d: &photocraft_doc::Document) -> Vec<(LayerId, FilterId)> {
    d.xcf_state
        .as_ref()
        .unwrap()
        .filter_stacks
        .iter()
        .flat_map(|s| {
            let ItemRef::Layer(l) = s.owner else { panic!("root") };
            s.nodes.iter().map(move |n| (l, n.id))
        })
        .collect()
}
fn patch(n: &photocraft_doc::xcf_filters::FilterNode) -> Option<serde_json::Value> {
    let (name, value) = match n.operation.name.as_str() {
        "gegl:exposure" => ("exposure", 0.5f32),
        "gegl:brightness-contrast" => ("brightness", 0.125),
        "gegl:levels" => ("out-high", 0.875),
        _ => return None,
    };
    Some(serde_json::json!({"name":name,"value":{"Number":{"Float32Bits":value.to_bits()}}}))
}
#[test]
#[ignore = "UNRUN genuine8 sealed Sources/all indexed targets, combined args+enable/raw native-mask-source custody/undo, whole-node ordered Move and compound metadata. No numerical producer equivalence."]
fn masked_mixed_indexed_edits_and_metadata_preserve_custody() {
    for c in fixture::cases() {
        let mut s = session(&c);
        let before = s.active().unwrap().doc.clone();
        assert!(before.accounted().unwrap().is_readonly_masked_filtered_flat_point_storage());
        assert!(!before.accounted().unwrap().is_readonly_filtered_flat_point_storage());
        for (layer, id) in targets(&before) {
            let n = before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.id == id).unwrap();
            assert!(crate::masked_filtered_flat_filter_edit_candidate(before.accounted().unwrap(), layer, n));
            assert!(!crate::filtered_flat_filter_edit_candidate(before.accounted().unwrap(), layer, n));
            let revision = s.active().unwrap().revision;
            let params = if let Some(a) = patch(n) {
                serde_json::json!({"filterId":id.0.to_string(),"arguments":[a],"enabled":false})
            } else {
                serde_json::json!({"filterId":id.0.to_string(),"enabled":false})
            };
            s.execute("xcf.filter.set", params).unwrap();
            assert_eq!(s.active().unwrap().revision, revision + 1);
            fixture::payload(&before, &s.active().unwrap().doc);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &s.active().unwrap().doc);
            let st = before.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == ItemRef::Layer(layer)).unwrap();
            if st.nodes.len() > 1 {
                let from = st.nodes.iter().position(|n| n.id == id).unwrap();
                let to = usize::from(from == 0);
                s.execute("xcf.filter.move", serde_json::json!({"filterId":id.0.to_string(),"index":to})).unwrap();
                let now = &s.active().unwrap().doc;
                assert_eq!(now.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == st.owner).unwrap().nodes[to].id, id);
                fixture::payload(&before, now);
                s.execute("edit.undo", serde_json::json!({})).unwrap();
                fixture::same(&before, &s.active().unwrap().doc);
            }
        }
        let root = before.layers[0].id;
        s.execute("layer.setProps", serde_json::json!({"layerId":root.0.to_string(),"name":"Masked continuation","visible":false,"opacity":0.375})).unwrap();
        let now = &s.active().unwrap().doc;
        assert_eq!(now.layers[0].name, "Masked continuation");
        assert!(!now.layers[0].visible);
        fixture::payload(&before, now);
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&before, &s.active().unwrap().doc);
        fixture::observed(&s.active().unwrap().doc, &c);
    }
}
fn install(s: &mut Session, owner: AccountedDocument) {
    let st = s.active().unwrap();
    let view = st
        .accounted_view
        .as_ref()
        .unwrap()
        .try_replace(&owner, st.active_layer, st.accounted_view.as_ref().unwrap().selected(), st.layer_anchor, &fixture::ctl())
        .unwrap();
    let history = DocumentHistory::new_accounted(owner.clone(), 64, "external CURRENT", view.target(), &fixture::ctl()).unwrap();
    let st = s.active_mut().unwrap();
    st.doc = DocumentSnapshot::from_accounted(owner).unwrap();
    st.accounted_view = Some(view);
    st.history = history;
}
#[test]
#[ignore = "UNRUN new nonempty exact+1 versus zero-sibling partial opacity, full OLD disabled malformed OTHER node before equal-noop, target locks/MAX/history/view and early Work/cancel atomic controls"]
fn masked_mixed_refusals_keep_current_atomic() {
    for c in fixture::cases() {
        let mut s = session(&c);
        let before = s.active().unwrap().doc.clone();
        let all = targets(&before);
        let (layer, id) = all[0];
        let n = before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.id == id).unwrap();
        let equal = serde_json::json!({"filterId":id.0.to_string(),"enabled":n.enabled});
        for (st, node) in before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| st.nodes.iter().map(move |n| (st, n))) {
            let is_masked = masked_kind(node).is_some();
            for word in ["0x0000000000000000", "0x8000000000000000", "0x3fd8000000000000"] {
                let p = serde_json::json!({"filterId":node.id.0.to_string(),"opacityF64Bits":word});
                if is_masked {
                    assert!(s.execute("xcf.filter.set", p).is_err());
                    assert!(s.active().unwrap().doc.shares_state(&before));
                } else {
                    s.execute("xcf.filter.set", p).unwrap();
                    s.execute("edit.undo", serde_json::json!({})).unwrap();
                    fixture::same(&before, &s.active().unwrap().doc);
                }
                let _ = st;
            }
        }
        for p in [
            serde_json::json!({"filterId":id.0.to_string(),"arguments":[],"enabled":false}),
            serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"unknown","value":{"Number":{"Float32Bits":0}}}]}),
        ] {
            assert!(s.execute("xcf.filter.set", p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
        }
        let bad = fixture::current(before.accounted().unwrap(), |d| {
            let other = all[1].1;
            let x = d.xcf_state.as_mut().unwrap();
            let n = x.filter_stacks.iter_mut().flat_map(|s| &mut s.nodes).find(|n| n.id == other).unwrap();
            n.enabled = false;
            n.operation.name = "unqualified:other".into();
        });
        let mut poisoned = session(&c);
        install(&mut poisoned, bad);
        let old = poisoned.active().unwrap().doc.clone();
        assert!(poisoned.execute("xcf.filter.enable", equal.clone()).is_err());
        assert!(poisoned.active().unwrap().doc.shares_state(&old));
        let locked = fixture::current(before.accounted().unwrap(), |d| d.layer_mut(layer).unwrap().locks.pixels = true);
        let mut blocked = session(&c);
        install(&mut blocked, locked);
        assert!(blocked.execute("xcf.filter.enable", equal.clone()).is_err());
        let mut mismatch = session(&c);
        let titled = before.accounted().unwrap().try_with_document_name("external title", &fixture::ctl()).unwrap();
        mismatch.active_mut().unwrap().doc = DocumentSnapshot::from_accounted(titled).unwrap();
        assert!(mismatch.execute("xcf.filter.enable", equal.clone()).is_err());
        let deny = |_| Err(BackingError::Work);
        let ctl = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
        assert!(Request::parse(&s, "xcf.filter.enable", &equal, &ctl).is_err());
        let cancelled = || true;
        let interrupt = photocraft_raster::Interrupt::cancel_only(&cancelled);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(Request::parse(&s, "xcf.filter.enable", &equal, &ctl).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        s.active_mut().unwrap().view_revision += 1;
        let prepared = Request::parse(&s, "xcf.filter.enable", &equal, &fixture::ctl()).unwrap().prepare(&s, &fixture::ctl()).unwrap();
        s.active_mut().unwrap().view_revision += 1;
        assert!(prepared.execute(&mut s, &fixture::ctl()).is_err());
        s.active_mut().unwrap().revision = u64::MAX;
        assert!(s.execute("xcf.filter.enable", equal).is_err());
    }
}
