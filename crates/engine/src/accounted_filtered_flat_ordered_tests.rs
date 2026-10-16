//! UNRUN storage/target/order continuity; no producer arithmetic oracle.
use super::*;
#[path = "accounted_filtered_flat_ordered_test_support.rs"]
mod fixture;
use photocraft_doc::{
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};
use serde_json::{Value, json};
fn patch(node: &FilterNode) -> Value {
    let argument = if node.operation.name == "gimp:semi-flatten" {
        let FilterValue::Color { components, encoding, .. } = &node.arguments[0].value else { panic!("color") };
        let ExactNumber::Float32Bits(alpha) = components[3] else { panic!("alpha") };
        let tag = if *encoding == photocraft_doc::SampleEncoding::Linear { "nonlinear_profile" } else { "linear" };
        json!({"name":"color","value":{"Color":{"components":[{"Float32Bits":0.125f32.to_bits()},{"Float32Bits":0.25f32.to_bits()},{"Float32Bits":0.375f32.to_bits()},{"Float32Bits":alpha}],"encoding":tag,"profile":null}}})
    } else {
        let name = if node.operation.name == "gegl:hue-chroma" { "hue" } else { "vibrance" };
        let a = node.arguments.iter().find(|a| a.name == name).unwrap();
        let FilterValue::Number(ExactNumber::Float32Bits(old)) = a.value else { panic!("F32") };
        let value = if old == 17f32.to_bits() { 0f32 } else { 17f32 };
        json!({"name":name,"value":{"Number":{"Float32Bits":value.to_bits()}}})
    };
    json!({"filterId":node.id.0.to_string(),"arguments":[argument],"enabled":!node.enabled})
}
#[test]
#[ignore = "UNRUN actual2 ordered source imports→typed owner/native; exactraw/public list/IDs/typedargs/masks, paged9node inspect and every middle/last target atomic edit+undo/noopredo; paid synthetic1root2native/control and strictmatching failures. Operativeorder/format/numerics remain lower authority/HOLD."]
fn ordered_storage_and_indexed_transactions_continue_without_aliases() {
    for case in &fixture::data::CASES {
        let input = fixture::import(case);
        let photocraft_io::OwnedXcfImport::FilteredFlat(imported) = &input else { panic!("FilteredFlat") };
        let mut s = Session::new();
        s.add_owned_filtered_flat_xcf_import(imported, "ordered.xcf", Some(case.source.path), &fixture::ctl()).unwrap();
        let original = s.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let mut native = Session::new();
        native.add_owned_native_filtered_flat_archive(fixture::archive(original.accounted().unwrap()), "authored.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(&original, &native.active().unwrap().doc);
        let flat = original.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| s.nodes.iter().map(|n| n.id)).collect::<Vec<_>>();
        assert_eq!(flat.len(), 9);
        for (i, id) in flat.iter().enumerate() {
            assert!(flat.iter().take(i).all(|old| old != id));
            let reply = crate::accounted_filter_inspect::prepare(&s, &json!({"filterId":id.0.to_string()}), &fixture::ctl()).unwrap();
            assert_eq!(reply.value()["filterId"], id.0.to_string());
            assert_eq!(reply.value()["scalarEditCandidate"], true);
            drop(reply);
        }
        for offset in [0usize, 1, 4, 8, 9] {
            let reply = crate::accounted_xcf_inspect::prepare(&s, &json!({"offset":offset,"limit":2}), &fixture::ctl()).unwrap();
            assert_eq!(reply.value()["filters"]["total"], 9);
            let ids = reply.value()["filters"]["entries"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
            assert_eq!(ids, flat.iter().skip(offset).take(2).map(|id| id.0.to_string()).collect::<Vec<_>>());
            drop(reply);
        }
        let targets = original
            .xcf_state
            .as_ref()
            .unwrap()
            .filter_stacks
            .iter()
            .flat_map(|st| st.nodes.iter().enumerate().skip(1).map(move |(index, n)| (st.owner, index, n.clone())))
            .collect::<Vec<_>>();
        for (owner, index, node) in targets {
            let before = s.active().unwrap().doc.clone();
            let before_revision = s.active().unwrap().revision;
            let old_stack = before.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == owner).unwrap();
            let p = patch(&node);
            assert_eq!(s.execute("xcf.filter.set", p.clone()).unwrap()["updated"], true);
            assert_eq!(s.active().unwrap().revision, before_revision + 1);
            let after = s.active().unwrap().doc.clone();
            fixture::payload(&before, &after);
            for old in &before.xcf_state.as_ref().unwrap().filter_stacks {
                let next = after.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == old.owner).unwrap();
                if old.owner != owner {
                    assert_eq!(old, next);
                    continue;
                }
                assert_eq!(next.revision, old_stack.revision + 1);
                for (at, (a, b)) in old.nodes.iter().zip(&next.nodes).enumerate() {
                    assert_eq!(a.id, b.id);
                    if at == index {
                        assert_eq!(b.id, node.id);
                        assert_eq!(b.enabled, !node.enabled);
                    } else {
                        assert_eq!(a, b);
                    }
                }
            }
            assert_eq!(s.execute("xcf.filter.set", p).unwrap()["updated"], false);
            assert!(s.active().unwrap().doc.shares_state(&after));
            s.execute("edit.undo", json!({})).unwrap();
            fixture::same(&before, &s.active().unwrap().doc);
            assert_eq!(s.execute("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":node.enabled})).unwrap()["updated"], false);
            assert!(s.active().unwrap().history.can_redo());
            s.execute("edit.redo", json!({})).unwrap();
            fixture::same(&after, &s.active().unwrap().doc);
            s.execute("edit.undo", json!({})).unwrap();
            fixture::same(&original, &s.active().unwrap().doc);
        }
        let single = fixture::single_root_two(original.accounted().unwrap());
        assert_eq!(single.document().layers.len(), 1);
        assert_eq!(single.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), 2);
        assert!(is_current_filtered_flat_candidate(single.document(), &fixture::ctl()).unwrap());
        let mut single_session = Session::new();
        single_session.add_owned_native_filtered_flat_archive(fixture::archive(&single), "single.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(single.document(), &single_session.active().unwrap().doc);
        let target = single_session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[1].clone();
        let p = patch(&target);
        let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&single_session, "xcf.filter.set", &p, &fixture::ctl()).unwrap();
        let before = single_session.active().unwrap().doc.clone();
        single_session.active_mut().unwrap().view_revision += 1;
        assert!(prepared.execute(&mut single_session, &fixture::ctl()).is_err());
        assert!(single_session.active().unwrap().doc.shares_state(&before));
        let bad = fixture::malformed(&single);
        assert!(is_current_filtered_flat_candidate(bad.document(), &fixture::ctl()).unwrap());
        let mut failed = Session::new();
        assert!(failed.add_owned_native_filtered_flat_archive(fixture::archive(&bad), "bad.pcraft", None, &fixture::ctl()).is_err());
        assert!(failed.active().is_none());
        assert!(failed.add_owned_native_point_archive(fixture::archive(&bad), "old-point.pcraft", None, &fixture::ctl()).is_err());
        assert!(failed.active().is_none());
        // External raw hint-only boundary, not a new source/constructor claim.
        let mut one = single.document().clone();
        one.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.truncate(1);
        assert!(!is_current_filtered_flat_candidate(&one, &fixture::ctl()).unwrap());
        for (id, p) in [
            ("layer.translate", json!({"dx":0})),
            ("xcf.filter.remove", json!({"filterId":target.id.0.to_string()})),
            ("layer.select", json!({"index":0})),
        ] {
            assert!(single_session.execute(id, p).is_err());
            assert!(single_session.active().unwrap().doc.shares_state(&before));
        }
        let ItemRef::Layer(root) = single.document().xcf_state.as_ref().unwrap().filter_stacks[0].owner else { panic!("root") };
        single_session.execute("layer.setProps", json!({"layer":root.0,"name":"Single CURRENT","opacity":0.5})).unwrap();
        fixture::payload(&before, &single_session.active().unwrap().doc);
        assert_eq!(std::fs::read(fixture::workspace().join(case.source.path)).unwrap(), fixture::bytes(case.source));
    }
}
