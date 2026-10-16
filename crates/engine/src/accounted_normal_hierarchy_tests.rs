//! Deferred synthetic CURRENT hierarchy integration. Genuine native leaves are
//! retained; topology is a declared fixture, never a producer Source oracle.
use super::*;
#[path = "accounted_normal_hierarchy_test_support.rs"]
mod f;
use photocraft_doc::item_identity::ItemRef;
use serde_json::json;
#[test]
#[ignore = "UNRUN/uncompiled: actual nested filter/metadata/current native13 and closed generic routes"]
fn hierarchy_nested_transactions_keep_tree_and_native_purpose() {
    // Source entry remains conditional on the separately released sealed IO
    // programme. These are real packets, never the synthetic CURRENT builder.
    for case in f::cases() {
        let source = f::import(&case);
        let photocraft_io::OwnedXcfImport::NormalHierarchy(imported) = &source else { panic!("hierarchy carrier") };
        let mut session = Session::new();
        session.add_owned_normal_hierarchy_xcf_import(imported, "source.xcf", Some(f::path(&case).to_str().unwrap()), &f::ctl()).unwrap();
        let before = session.active().unwrap().doc.clone();
        f::observed(&before, &case);
        let mut reopened = Session::new();
        reopened.add_owned_native_normal_hierarchy_archive(f::archive(before.accounted().unwrap()), "source.pcraft", None, &f::ctl()).unwrap();
        f::same(&before, &reopened.active().unwrap().doc);
    }

    for depth in 0..2 {
        let owner = f::build(depth);
        let mut s = Session::new();
        assert!(s.add_accounted_document(owner.clone(), None, &f::ctl()).is_err());
        s.add_owned_native_normal_hierarchy_archive(f::archive(&owner), "hierarchy.pcraft", None, &f::ctl()).unwrap();
        let layer = f::layer(&owner);
        let old = s.active().unwrap().doc.clone();
        let stack = old.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == ItemRef::Layer(layer)).unwrap();
        let target = stack.nodes.last().unwrap().id;
        s.execute("xcf.filter.enable", json!({"filterId":target.0.to_string(),"enabled":false})).unwrap();
        let after = s.active().unwrap().doc.clone();
        assert!(after.accounted().unwrap().is_readonly_normal_hierarchy_storage());
        f::payload(&old, &after);
        let changed = after.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == stack.owner).unwrap();
        assert_eq!(changed.revision, stack.revision + 1);
        assert!(!changed.nodes.last().unwrap().enabled);
        assert_eq!(&changed.nodes[..changed.nodes.len() - 1], &stack.nodes[..stack.nodes.len() - 1]);
        s.execute("xcf.filter.move", json!({"filterId":target.0.to_string(),"index":0})).unwrap();
        s.execute("xcf.filter.remove", json!({"filterId":target.0.to_string()})).unwrap();
        s.execute("layer.setProps", json!({"layer":f::OUTER.0,"name":"Renamed outer","opacity":0.375})).unwrap();
        let edited = s.active().unwrap().doc.clone();
        assert_eq!(edited.layer(f::OUTER).unwrap().opacity.to_bits(), 0.375f32.to_bits());
        f::payload(&old, &edited);
        let mut reopened = Session::new();
        reopened.add_owned_native_normal_hierarchy_archive(f::archive(edited.accounted().unwrap()), "continued.pcraft", None, &f::ctl()).unwrap();
        f::same(&edited, &reopened.active().unwrap().doc);
        for (id, p) in [("layer.translate", json!({"layer":layer.0,"dx":1,"dy":0})), ("xcf.filter.insert", json!({})), ("select.all", json!({}))] {
            let before = s.active().unwrap().doc.clone();
            assert!(s.execute(id, p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before))
        }
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.redo", json!({})).unwrap();
        f::same(&edited, &s.active().unwrap().doc);
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: hierarchy focus/disclosure upstream bookkeeping, stale source/view and full inactive proof"]
fn hierarchy_view_and_disclosure_are_paid_captured_routes() {
    let owner = f::build(1);
    let mut s = Session::new();
    s.add_owned_native_normal_hierarchy_archive(f::archive(&owner), "hierarchy.pcraft", None, &f::ctl()).unwrap();
    let layer = f::layer(&owner);
    let before = s.active().unwrap();
    let revision = before.revision;
    let count = before.history.past_len();
    s.execute("layer.select", json!({"layer":layer.0})).unwrap();
    let state = s.active().unwrap();
    assert_eq!(state.active_layer, Some(layer));
    assert_eq!(state.revision, revision + 1);
    assert_eq!(state.history.past_len(), count);
    let params = json!({"layer":f::OUTER.0,"expanded":false});
    let revision = state.revision;
    let journal = s.journal.len();
    s.execute("layer.setExpanded", params.clone()).unwrap();
    let state = s.active().unwrap();
    assert_eq!(state.revision, revision);
    assert_eq!(state.history.past_len(), count);
    assert_eq!(s.journal.len(), journal);
    let photocraft_doc::LayerContent::Group(g) = &state.doc.layer(f::OUTER).unwrap().content else { panic!("group") };
    assert!(!g.expanded);
    let reply = s.execute("layer.setExpanded", json!({"all":true,"expanded":true})).unwrap();
    assert_eq!(reply["groups"], 3);
    assert_eq!(reply["expanded"], true);
    let params = json!({"layer":f::INNER.0});
    let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "layer.select", &params, &f::ctl()).unwrap();
    s.active_mut().unwrap().source.as_mut().unwrap().label.push_str(" changed without revision");
    assert!(prepared.execute(&mut s, &f::ctl()).is_err());
    let poisoned = f::changed(&owner, |d| {
        let x = d.xcf_state.as_mut().unwrap();
        let n = &mut x.filter_stacks.last_mut().unwrap().nodes[0];
        n.enabled = false;
        n.operation.schema_epoch = "unsupported-hidden".into();
    });
    assert!(crate::accounted_native_open::is_owned_current_normal_hierarchy_candidate(&f::archive(&owner), &f::ctl()).unwrap());
    let raw = photocraft_format::owned_document::save_accounted_normal_hierarchy_document_archive(
        &poisoned,
        std::io::Cursor::new(Vec::new()),
        Default::default(),
        poisoned.quota(),
        &f::ctl(),
    )
    .unwrap()
    .into_inner();
    let a = photocraft_io::accounted_native::open_accounted_native_from_reader(
        &mut std::io::Cursor::new(&raw),
        raw.len() as u64,
        Default::default(),
        poisoned.quota().clone(),
        &f::ctl(),
    )
    .unwrap();
    let mut refused = Session::new();
    assert!(refused.add_owned_native_normal_hierarchy_archive(a, "poisoned", None, &f::ctl()).is_err());
    assert!(refused.active().is_none());
    let _ = journal;
}
