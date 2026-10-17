//! Distinct actual-class publication bypass/readonly controls, not numeric tests.
use super::*;
#[path = "../../ui-egui/src/filtered_flat_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[test]
#[ignore = "UNRUN:authentic mixedsource→paid larger syntheticCURRENT/native, genericSession bypass and directMovezero closures, remaining unsupported mutations refused; US2 admitted hints corrected, retained pagedquery/control/title/native custody; no producergeometry/numericalacceptance"]
fn filtered_flat_publication_is_typed_and_mutations_are_closed() {
    for index in 0..2 {
        let source = Arc::new(fixture::import(index));
        let owner = fixture::larger(source);
        let owner = fixture::activity(&owner);
        let mut generic = Session::new();
        assert!(generic.add_accounted_document(owner.clone(), None, &fixture::ctl()).is_err());
        assert!(generic.active().is_none());
        // L1 is deliberately data-only: an unknown-operation structural owner cannot
        // gain publication through the generic API when old semantic hints are false.
        let unknown = fixture::current(&owner, |d| {
            for stack in &mut d.xcf_state.as_mut().unwrap().filter_stacks {
                let n = &mut stack.nodes[0];
                n.operation.name = "unsupported:fixture".into();
                n.operation.schema_epoch = "unsupported-v1".into();
                n.composition.as_mut().unwrap().compatibility_epoch = "unsupported-v1".into();
            }
        });
        assert!(generic.add_accounted_document(unknown, None, &fixture::ctl()).is_err());
        assert!(generic.active().is_none());
        assert!(
            generic
                .add_owned_native_archive(
                    fixture::archive(&owner),
                    AccountedNativeOpenPolicy::KeepCurrentInterpretation,
                    "generic.pcraft",
                    None,
                    &fixture::ctl()
                )
                .is_err()
        );
        assert!(generic.active().is_none());
        let mut s = Session::new();
        s.add_owned_native_filtered_flat_archive(fixture::archive(&owner), "typed.pcraft", None, &fixture::ctl()).unwrap();
        let before = s.active().unwrap().doc.clone();
        fixture::same(owner.document(), &before);
        assert!(before.accounted().unwrap().quota().shares_ledger(owner.quota()));
        let history = s.active().unwrap().history.clone();
        let layer = before.layers[0].id;
        let filter = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        assert!(s.translate_accounted_imported_layer(layer, 0, 0, None, &fixture::ctl()).is_err());
        assert!(s.translate_accounted_imported_selection(0, 0, None, &fixture::ctl()).is_err());
        assert!(
            s.edit_accounted("denied", photocraft_doc::accounted_document::AccountedEdit::LayerVisibility { layer, visible: true }, &fixture::ctl()).is_err()
        );
        for (id, p) in [
            ("xcf.filter.enable", serde_json::json!({"filterId":filter,"enabled":false,"unsupported":true})),
            ("xcf.filter.set", serde_json::json!({"filterId":filter,"enabled":true,"unsupported":true})),
            ("layer.select", serde_json::json!({"index":0})),
            ("layer.setProps", serde_json::json!({"blend":"denied"})),
            ("layer.translate", serde_json::json!({"dx":0,"dy":0})),
            ("xcf.filter.remove", serde_json::json!({"filterId":filter,"index":0})),
        ] {
            assert!(s.execute(id, p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert!(s.active().unwrap().history.shares_accounted_history(&history));
        }
        let reply = crate::accounted_xcf_inspect::prepare(&s, &serde_json::json!({"offset":1,"limit":2}), &fixture::ctl()).unwrap();
        assert_eq!(reply.value()["items"]["total"], 7);
        assert_eq!(reply.value()["filters"]["total"], 3);
        drop(reply);
        for stack in &before.xcf_state.as_ref().unwrap().filter_stacks {
            let id = stack.nodes[0].id.0.to_string();
            let reply = crate::accounted_filter_inspect::prepare(&s, &serde_json::json!({"filterId":id}), &fixture::ctl()).unwrap();
            assert_eq!(reply.value()["filterId"], id);
            assert_eq!(reply.value()["scalarEditCandidate"], true);
            drop(reply);
        }
        let calls = AtomicU64::new(0);
        let deny = |_: u64| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err(BackingError::Work)
        };
        let c = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &deny };
        let mut failed = Session::new();
        assert!(failed.add_owned_native_filtered_flat_archive(fixture::archive(&owner), "work.pcraft", None, &c).is_err());
        assert!(calls.load(Ordering::Relaxed) > 0);
        assert!(failed.active().is_none());
        let invalid = fixture::current(&owner, |d| d.xcf_state.as_mut().unwrap().filter_stacks[1].nodes[0].operation.name = "unsupported:fixture".into());
        assert!(is_current_filtered_flat_candidate(invalid.document(), &fixture::ctl()).unwrap());
        assert!(failed.add_owned_native_filtered_flat_archive(fixture::archive(&invalid), "invalid.pcraft", None, &fixture::ctl()).is_err());
        assert!(failed.active().is_none());
        let titled = before.accounted().unwrap().try_with_document_name("Filtered flat title", &fixture::ctl()).unwrap();
        let mut next = Session::new();
        next.add_owned_native_filtered_flat_archive(fixture::archive(&titled), "continued.pcraft", None, &fixture::ctl()).unwrap();
        fixture::same(titled.document(), &next.active().unwrap().doc);
    }
}
