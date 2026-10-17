//! Deferred full-removal CURRENT class/native intent, no empty original-source grant.
use super::*;
#[path = "../accounted_filtered_flat_current_test_support.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
fn session(case: &fixture::data::Case) -> Session {
    let input = fixture::import(case);
    let photocraft_io::OwnedXcfImport::FilteredFlat(input) = &input else { panic!("mixed") };
    let mut s = Session::new();
    s.add_owned_filtered_flat_xcf_import(input, "ordered.xcf", Some(case.source.path), &fixture::ctl()).unwrap();
    s
}
fn all(s: &Session) -> Vec<FilterId> {
    s.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|st| st.nodes.iter().map(|n| n.id)).collect()
}
#[test]
#[ignore = "UNRUN both genuine ordered sources→all9removed fullOLD/history/STAGED, retainedemptystacks/class/profile/source/native/masks+inertarchives, zero/single query/workpolicy, markednative10 roundtrip and unmarked/wronghandoff refusal; no empty producer oracle."]
fn complete_remove_retains_class_and_marked_native() {
    for case in &fixture::data::CASES {
        let mut s = session(case);
        let original = s.active().unwrap().doc.clone();
        let ids = all(&s);
        for (at, id) in ids.iter().enumerate() {
            let before = s.active().unwrap().doc.clone();
            s.execute("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
            let after = s.active().unwrap().doc.clone();
            fixture::preserved(&before, &after, *id);
            assert!(after.accounted().unwrap().is_readonly_filtered_flat_point_storage());
            let q = crate::accounted_xcf_inspect::prepare(&s, &serde_json::json!({"limit":128}), &fixture::ctl()).unwrap();
            assert_eq!(q.value()["filters"]["total"], ids.len() - at - 1);
            assert_eq!(q.value()["filterStacks"]["total"], 3);
            drop(q);
            let archive = fixture::archive(after.accounted().unwrap());
            assert!(crate::accounted_native_open::is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
            let mut reopened = Session::new();
            reopened.add_owned_native_filtered_flat_archive(archive, "current.pcraft", None, &fixture::ctl()).unwrap();
            fixture::same(&after, &reopened.active().unwrap().doc);
        }
        let empty = s.active().unwrap().doc.clone();
        assert_eq!(empty.xcf_state.as_ref().unwrap().filter_stacks.len(), 3);
        assert!(empty.xcf_state.as_ref().unwrap().filter_stacks.iter().all(|st| st.nodes.is_empty()));
        assert!(empty.xcf_state.as_ref().unwrap().filter_masks.is_empty());
        assert!(crate::accounted_native_open::is_current_standard_work_candidate(empty.accounted().unwrap(), &fixture::ctl()).unwrap());
        assert!(crate::accounted_native_open::is_current_pure_point_work_candidate(empty.accounted().unwrap(), &fixture::ctl()).unwrap());
        assert!(crate::accounted_filter_inspect::prepare(&s, &serde_json::json!({"filterId":ids[0].0.to_string()}), &fixture::ctl()).is_err());
        let history = s.active().unwrap().history.clone();
        assert!(s.execute("xcf.filter.remove", serde_json::json!({"filterId":ids[0].0.to_string()})).is_err());
        assert!(s.active().unwrap().doc.shares_state(&empty));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        assert_eq!(all(&s).len(), 1);
        s.execute("edit.redo", serde_json::json!({})).unwrap();
        fixture::same(&empty, &s.active().unwrap().doc);
        let raw = fixture::encoded(empty.accounted().unwrap());
        assert!(photocraft_format::load_from_bytes(&raw).is_err());
        let mut wrong = Session::new();
        assert!(wrong.add_owned_native_point_archive(fixture::archive(empty.accounted().unwrap()), "wrong.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.active().is_none());
        assert!(
            wrong
                .add_owned_native_archive(
                    fixture::archive(empty.accounted().unwrap()),
                    crate::accounted_native_open::AccountedNativeOpenPolicy::KeepCurrentInterpretation,
                    "wrong.pcraft",
                    None,
                    &fixture::ctl()
                )
                .is_err()
        );
        let unmarked = fixture::unmarked_archive(empty.accounted().unwrap());
        assert_eq!(unmarked.current_storage_hint(), None);
        assert!(wrong.add_owned_native_filtered_flat_archive(unmarked, "unmarked.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.active().is_none());
        assert_eq!(fixture::bytes(case.source), std::fs::read(fixture::workspace().join(case.source.path)).unwrap());
        let bad = fixture::current(original.accounted().unwrap(), |d| {
            d.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.name = "unsupported:marked".into()
        });
        let marked = fixture::archive(&bad);
        assert!(crate::accounted_native_open::is_owned_current_filtered_flat_candidate(&marked, &fixture::ctl()).unwrap());
        assert!(wrong.add_owned_native_filtered_flat_archive(marked, "bad.pcraft", None, &fixture::ctl()).is_err());
        assert!(wrong.active().is_none());
        assert_eq!(empty.layers.len(), original.layers.len());
    }
}
#[test]
#[ignore = "UNRUN paid single-root3→2→1→0 Current eachstage markednative proof, zero stackmetadata/compoundprops/history/query and closed generic/oldsource handoff; no source topology widening."]
fn single_root_zero_remains_mixed() {
    for case in &fixture::data::CASES {
        let original = session(case);
        let single = fixture::single_root_three(original.active().unwrap().doc.accounted().unwrap());
        let mut s = Session::new();
        s.add_owned_native_filtered_flat_archive(fixture::archive(&single), "single.pcraft", None, &fixture::ctl()).unwrap();
        let ids = all(&s);
        let root = s.active().unwrap().doc.layers[0].id;
        for id in ids {
            let before = s.active().unwrap().doc.clone();
            s.execute("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
            let after = s.active().unwrap().doc.clone();
            fixture::preserved(&before, &after, id);
            let mut reopened = Session::new();
            reopened.add_owned_native_filtered_flat_archive(fixture::archive(after.accounted().unwrap()), "one.pcraft", None, &fixture::ctl()).unwrap();
            fixture::same(&after, &reopened.active().unwrap().doc);
        }
        let before = s.active().unwrap().doc.clone();
        assert_eq!(before.xcf_state.as_ref().unwrap().filter_stacks.len(), 1);
        assert!(before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
        s.execute("layer.setProps", serde_json::json!({"layer":root.0,"name":"Empty retained root","opacity":0.5})).unwrap();
        let updated = s.active().unwrap().doc.clone();
        assert!(updated.accounted().unwrap().is_readonly_filtered_flat_point_storage());
        s.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&before, &s.active().unwrap().doc);
        s.execute("edit.redo", serde_json::json!({})).unwrap();
        fixture::same(&updated, &s.active().unwrap().doc);
        let query = crate::accounted_xcf_inspect::prepare(&s, &serde_json::json!({"offset":0,"limit":1}), &fixture::ctl()).unwrap();
        assert_eq!(query.value()["filters"]["total"], 0);
        assert_eq!(query.value()["items"]["total"], 1);
        assert_eq!(query.value()["filterStacks"]["total"], 1);
        drop(query);
        let mut wrong = Session::new();
        assert!(wrong.add_accounted_document(updated.accounted().unwrap().clone(), None, &fixture::ctl()).is_err());
    }
}
