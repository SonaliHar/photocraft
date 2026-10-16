//! Deferred registered affine edits over independently saved original sources.
use super::*;
#[path = "../accounted_filtered_flat_affine_test_support.rs"]
mod fixture;
fn session(c: &fixture::Case) -> Session {
    let input = fixture::import(c);
    let photocraft_io::OwnedXcfImport::FilteredFlat(input) = &input else { panic!("mixed source") };
    let mut s = Session::new();
    s.add_owned_filtered_flat_xcf_import(input, "affine.xcf", fixture::path(c).to_str(), &fixture::ctl()).unwrap();
    s
}
fn patches(names: &[&str], values: &[f32]) -> Vec<Value> {
    names.iter().zip(values).map(|(name, v)| serde_json::json!({"name":name,"value":{"Number":{"Float32Bits":v.to_bits()}}})).collect()
}
#[test]
#[ignore = "UNRUN actual4 affine sources/10affine nodes each, complete2/4arg compound opacity+activation, one revision/undo/native10/rawsource preservation. No producer numeric oracle."]
fn affine_compound_edits_preserve_native_and_history() {
    for c in &fixture::CASES {
        let mut s = session(c);
        let ids = s
            .active()
            .unwrap()
            .doc
            .xcf_state
            .as_ref()
            .unwrap()
            .filter_stacks
            .iter()
            .flat_map(|st| &st.nodes)
            .filter(|n| fixture::affine(n))
            .map(|n| (n.id, n.operation.name == "gegl:levels"))
            .collect::<Vec<_>>();
        for (id, levels) in ids {
            let before = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            let args = if levels {
                patches(&["out-high", "in-high", "out-low", "in-low"], &[0.75, 1.25, 0.125, -0.125])
            } else {
                patches(&["brightness", "contrast"], &[0.125, 0.875])
            };
            let params = serde_json::json!({"filterId":id.0.to_string(),"arguments":args,"enabled":false,"opacityF64Bits":"0x3fd8000000000001"});
            assert_eq!(s.execute("xcf.filter.set", params.clone()).unwrap()["updated"], true);
            let changed = s.active().unwrap().doc.clone();
            fixture::payload(&before, &changed);
            assert_eq!(s.active().unwrap().revision, revision + 1);
            let old = before.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.nodes.iter().any(|n| n.id == id)).unwrap();
            let new = changed.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == old.owner).unwrap();
            assert_eq!(new.revision, old.revision + 1);
            let node = new.nodes.iter().find(|n| n.id == id).unwrap();
            let original = old.nodes.iter().find(|n| n.id == id).unwrap();
            assert_eq!(node.arguments.iter().map(|a| &a.name).collect::<Vec<_>>(), original.arguments.iter().map(|a| &a.name).collect::<Vec<_>>());
            assert!(!node.enabled);
            assert_eq!(node.opacity_f64_bits, 0x3fd8000000000001);
            if levels {
                assert_eq!(photocraft_compose::gegl_levels_mixed_parameters(&node.arguments).unwrap().values(), [-0.125, 1.25, 0.125, 0.75]);
            } else {
                assert_eq!(photocraft_compose::brightness_contrast_mixed_parameters(&node.arguments).unwrap().values(), [0.125, 0.875]);
            }
            assert_eq!(s.execute("xcf.filter.set", params).unwrap()["updated"], false);
            let mut reopened = Session::new();
            reopened.add_owned_native_filtered_flat_archive(fixture::archive(changed.accounted().unwrap()), "affine.pcraft", None, &fixture::ctl()).unwrap();
            fixture::same(&changed, &reopened.active().unwrap().doc);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &s.active().unwrap().doc);
            s.execute("edit.redo", serde_json::json!({})).unwrap();
            fixture::same(&changed, &s.active().unwrap().doc);
            s.execute("edit.undo", serde_json::json!({})).unwrap();
        }
        assert_eq!(fixture::bytes(c), std::fs::read(fixture::path(c)).unwrap());
    }
}
#[test]
#[ignore = "UNRUN Levels-only four-patch admission, invalid paired endpoints, duplicate/nonfinite/wrongtypes/emptyarray remain atomic and preserve redo; old kinds retain three ceiling. No command execution yet."]
fn affine_outer_limits_and_tuple_refusals_are_atomic() {
    for c in fixture::CASES.iter().step_by(2) {
        let mut s = session(c);
        let before = s.active().unwrap().doc.clone();
        let nodes = before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| &st.nodes).collect::<Vec<_>>();
        let levels = nodes.iter().find(|n| n.operation.name == "gegl:levels").unwrap().id;
        let other = nodes.iter().find(|n| n.operation.name == "gegl:brightness-contrast").unwrap().id;
        let invalid = patches(&["in-low", "in-high", "out-low", "out-high"], &[1., 0., 0., 1.]);
        let four = patches(&["in-low", "in-high", "out-low", "out-high"], &[0., 1., 0., 1.]);
        for p in [
            serde_json::json!({"filterId":levels.0.to_string(),"arguments":invalid}),
            serde_json::json!({"filterId":other.0.to_string(),"arguments":four}),
            serde_json::json!({"filterId":levels.0.to_string(),"arguments":[],"enabled":false}),
            serde_json::json!({"filterId":levels.0.to_string(),"arguments":patches(&["in-low","in-low"], &[0.,0.])}),
            serde_json::json!({"filterId":levels.0.to_string(),"arguments":[{"name":"in-low","value":{"Number":{"Float32Bits":f32::NAN.to_bits()}}}]}),
            serde_json::json!({"filterId":levels.0.to_string(),"arguments":[{"name":"in-low","value":{"Number":{"Float64Bits":0u64}}}]}),
        ] {
            let redo = s.active().unwrap().history.can_redo();
            assert!(s.execute("xcf.filter.set", p).is_err());
            fixture::same(&before, &s.active().unwrap().doc);
            assert_eq!(s.active().unwrap().history.can_redo(), redo);
        }
    }
}
