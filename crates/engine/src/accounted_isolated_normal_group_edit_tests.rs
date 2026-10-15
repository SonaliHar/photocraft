//! UNRUN current-only transactions over the 24 genuine isolated-group sources.
//! Source/archives/fingerprints are external harness data. Synthetic CURRENT
//! locks have a real prospective metadata fee and retain the original owner;
//! they do not claim a modified source oracle or independent numeric reference.
use super::*;
use crate::{DocumentSnapshot, accounted_commands::PreparedAccountedCommand};
use photocraft_doc::{
    LayerId,
    accounted_document::{AccountedDocument, AccountedEdit},
    item_identity::{ItemRef, MaskId},
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
fn session(case: usize) -> Session {
    let imported = imported(case, &quota());
    let mut s = Session::new();
    s.add_owned_isolated_normal_group_xcf_import(&imported, CASES[case].0, Some(CASES[case].0), &ctl()).unwrap();
    drop(imported);
    s
}
fn ids(s: &Session) -> [LayerId; 4] {
    let layers = &s.active().unwrap().doc.layers;
    let LayerContent::Group(group) = &layers[1].content else { panic!("genuine group") };
    [layers[0].id, layers[1].id, group.children[0].id, group.children[1].id]
}
fn layer(s: &Session, id: LayerId) -> &Layer {
    s.active().unwrap().doc.layer(id).unwrap()
}
fn mask_id(s: &Session) -> Option<MaskId> {
    s.active().unwrap().doc.xcf_state.as_ref().unwrap().items.iter().find_map(|i| if let ItemRef::Mask(id) = i.item { Some(id) } else { None })
}
fn mask_enabled(s: &Session) -> bool {
    s.active().unwrap().doc.layers[1].mask.as_ref().unwrap().enabled
}
fn mask_run(s: &mut Session, value: bool) -> Value {
    let id = mask_id(s).unwrap();
    s.execute("xcf.mask.enabled.set", json!({"maskId":id.0.to_string(),"enabled":value})).unwrap()
}
fn reopen(s: &Session) -> Session {
    let q = s.active().unwrap().doc.quota().unwrap();
    let bytes =
        photocraft_format::owned_document::save_current_document_archive(&s.active().unwrap().doc, Cursor::new(Vec::new()), Default::default(), q, &ctl())
            .unwrap()
            .into_inner();
    let mut out = Session::new();
    out.add_owned_native_isolated_normal_group_archive(archive(&bytes, q), "current.pcraft", Some("actual/current.pcraft"), &ctl()).unwrap();
    out
}
#[test]
#[ignore = "UNRUN: all24 actual groups, four-target coalescing/first-label/raw no-op redo and name lookup reverse DFS"]
fn all_four_layers_share_one_upstream_key_without_losing_original_owners() {
    for case in 0..CASES.len() {
        let mut s = session(case);
        let ids = ids(&s);
        let initial = s.active().unwrap().doc.clone();
        let words = native(&s);
        let source = s.active().unwrap().source.clone();
        let first = !layer(&s, ids[3]).visible;
        s.execute("layer.setProps", json!({"layer":ids[3].0,"visible":first,"coalesce":"\0group"})).unwrap();
        for (i, id) in ids.iter().enumerate() {
            s.execute("layer.setProps", json!({"layer":id.0,"name":format!("Current {i}"),"opacity":0.375,"coalesce":"\0group"})).unwrap();
        }
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Layer Visibility"));
        assert_eq!(native(&s), words);
        assert_eq!(s.active().unwrap().source, source);
        assert_eq!(s.active().unwrap().doc.icc_profile, initial.icc_profile);
        assert_eq!(s.active().unwrap().doc.source_color_context, initial.source_color_context);
        assert!(s.undo_with_control(&ctl()).unwrap());
        let history = s.active().unwrap().history.clone();
        let rev = s.active().unwrap().revision;
        s.execute(
            "layer.setProps",
            json!({"layer":ids[0].0,"name":layer(&s,ids[0]).name,"visible":layer(&s,ids[0]).visible,"opacity":layer(&s,ids[0]).opacity,"coalesce":"other"}),
        )
        .unwrap();
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, rev);
        assert!(s.active().unwrap().history.can_redo());
        assert!(s.redo_with_control(&ctl()).unwrap());
        for id in ids {
            s.execute("layer.setProps", json!({"layer":id.0,"name":"Same name"})).unwrap();
        }
        s.execute("layer.setProps", json!({"layer":"Same name","name":"Topmost child"})).unwrap();
        assert_eq!(layer(&s, ids[3]).name, "Topmost child");
        for id in &ids[..3] {
            assert_eq!(layer(&s, *id).name, "Same name");
        }
        let before = s.active().unwrap().doc.clone();
        let rev = s.active().unwrap().revision;
        // Recognized wrong types and all-None preserve the original adapter.
        s.execute("layer.setProps", json!({"name":false,"visible":"wrong","opacity":null,"coalesce":42})).unwrap();
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().revision, rev);
        s.execute("layer.setProps", json!({"layer":ids[0].0,"opacity":-0.0})).unwrap();
        assert_eq!(layer(&s, ids[0]).opacity.to_bits(), (-0.0f32).to_bits());
        let current = s.active().unwrap().doc.clone();
        let reopened = reopen(&s);
        assert_eq!(reopened.active().unwrap().doc.xcf_state, current.xcf_state);
        assert_eq!(native(&reopened), words);
        assert_eq!(reopened.active().unwrap().doc.icc_profile, current.icc_profile);
    }
}
#[test]
#[ignore = "UNRUN: optional group mask, exact four-field journal:false reply, direct label and undo/noop-redo/native continuation"]
fn group_mask_uses_the_existing_no_journal_transaction() {
    for case in 0..CASES.len() {
        let mut s = session(case);
        let group = ids(&s)[1];
        let Some(mask) = mask_id(&s) else {
            let before = s.active().unwrap().doc.clone();
            assert!(s.edit_accounted("No mask", AccountedEdit::MaskEnabled { layer: group, enabled: false }, &ctl()).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            continue;
        };
        let original = mask_enabled(&s);
        let words = native(&s);
        s.execute("layer.setProps", json!({"layer":group.0,"name":"Mask key","coalesce":"key"})).unwrap();
        let journal = s.journal.clone();
        let source = s.active().unwrap().source.clone();
        assert_eq!(mask_run(&mut s, !original), json!({"maskId":mask.0.to_string(),"layerId":group.0.to_string(),"enabled":!original,"changed":true}));
        assert_eq!(s.journal, journal);
        assert!(s.active().unwrap()._accounted_metadata_gesture.is_none());
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Set XCF Layer Mask Enabled"));
        assert_eq!(native(&s), words);
        assert_eq!(s.active().unwrap().source, source);
        assert!(s.undo_with_control(&ctl()).unwrap());
        let history = s.active().unwrap().history.clone();
        let rev = s.active().unwrap().revision;
        assert_eq!(mask_run(&mut s, original)["changed"], false);
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, rev);
        assert!(s.redo_with_control(&ctl()).unwrap());
        let mut reopened = reopen(&s);
        assert_eq!(mask_enabled(&reopened), !original);
        assert_eq!(native(&reopened), words);
        reopened.edit_accounted("Caller mask label", AccountedEdit::MaskEnabled { layer: group, enabled: original }, &ctl()).unwrap();
        assert_eq!(reopened.active().unwrap().history.undo_label(), Some("Caller mask label"));
        assert_eq!(s.journal, journal);
        s.coalesce_request = Some("forbidden".into());
        assert!(s.edit_accounted("No coalesce", AccountedEdit::MaskEnabled { layer: group, enabled: original }, &ctl()).is_err());
    }
}
fn locked(kind: usize) -> Session {
    let s = session(0);
    let q = s.active().unwrap().doc.quota().unwrap().clone();
    let source = s.active().unwrap().doc.accounted().unwrap().clone();
    let fee = q.reserve(QuotaKind::Index, 1 << 20).unwrap();
    let mut doc = source.document().clone();
    let group = doc.layers[1].id;
    match kind {
        0 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(group)).unwrap().locks.visibility = true,
        1 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(group)).unwrap().locks.content = true,
        2 => doc.layers[1].locks.all = true,
        3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| matches!(i.item, ItemRef::Mask(_))).unwrap().locks.content = true,
        _ => doc.layers[1].locks.pixels = true,
    }
    let owner = AccountedDocument::from_owned(doc, Arc::new((source, fee)), q.clone(), &ctl()).unwrap();
    let snapshot = DocumentSnapshot::from_accounted(owner).unwrap();
    let bytes = photocraft_format::owned_document::save_current_document_archive(&snapshot, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
        .unwrap()
        .into_inner();
    let mut current = Session::new();
    current.add_owned_native_isolated_normal_group_archive(archive(&bytes, &q), "locked.pcraft", None, &ctl()).unwrap();
    current
}
#[test]
#[ignore = "UNRUN: genuine native CURRENT inherited visibility/content/all locks, mask no-op lock ordering and pixel-only distinction"]
fn inherited_group_locks_follow_the_actual_owner_tree() {
    for kind in 0..5 {
        let mut s = locked(kind);
        let ids = ids(&s);
        let child = ids[3];
        let old = layer(&s, child).visible;
        let result = s.execute("layer.setProps", json!({"layer":child.0,"visible":!old}));
        if kind == 0 {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        // Name/opacity do not acquire a new content or native-all ban.
        s.execute("layer.setProps", json!({"layer":child.0,"name":"Locked child","opacity":0.5})).unwrap();
        let original = mask_enabled(&s);
        let before = s.active().unwrap().doc.clone();
        let journal = s.journal.clone();
        for enabled in [original, !original] {
            let mask = mask_id(&s).unwrap();
            let result = s.execute("xcf.mask.enabled.set", json!({"maskId":mask.0.to_string(),"enabled":enabled}));
            if matches!(kind, 1 | 2 | 3) {
                assert!(result.is_err());
                assert!(s.active().unwrap().doc.shares_state(&before));
            } else {
                result.unwrap();
            }
            assert_eq!(s.journal, journal);
        }
    }
}
#[test]
#[ignore = "UNRUN: prepared group source/history/preferences/channel/journal fences and quota/late cumulative cancellation rollback"]
fn group_transactions_fail_privately_under_stale_or_exhausted_resources() {
    for mask in [false, true] {
        for change in 0..6 {
            let mut s = session(0);
            let params = if mask { json!({"maskId":mask_id(&s).unwrap().0.to_string(),"enabled":false}) } else { json!({"layer":ids(&s)[3].0,"name":"Stale"}) };
            let command = if mask { "xcf.mask.enabled.set" } else { "layer.setProps" };
            let prepared = PreparedAccountedCommand::prepare(&s, command, &params, &ctl()).unwrap();
            match change {
                0 => s.active_mut().unwrap().source.as_mut().unwrap().label = "Changed source".into(),
                1 => s.active_mut().unwrap().path = Some("changed.pcraft".into()),
                2 => s.active_mut().unwrap().history.max_states += 1,
                3 => s.prefs.edit(|p| p.performance.history_states = p.performance.history_states.saturating_add(1)),
                4 => s.active_mut().unwrap().channel_view.alpha_visible.push(false),
                _ => s.journal.push(("harness-only".into(), Value::Null)),
            }
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.clone();
            let journal = s.journal.clone();
            let rev = s.active().unwrap().revision;
            assert!(prepared.execute(&mut s, &ctl()).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert!(s.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(s.journal, journal);
            assert_eq!(s.active().unwrap().revision, rev);
        }
    }
    let mut s = session(0);
    let id = ids(&s)[3];
    let q = s.active().unwrap().doc.quota().unwrap().clone();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let spent = AtomicU64::new(0);
    let work = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    crate::accounted_xcf_open::isolated_normal_group::validate_current_isolated_normal_group_storage(
        before.accounted().unwrap(),
        &ReadControl { interrupt: &Interrupt::NONE, work: &work },
    )
    .unwrap();
    let cost = spent.load(Ordering::Relaxed);
    let spent = AtomicU64::new(0);
    let work = |n| if spent.fetch_add(n, Ordering::Relaxed).saturating_add(n) > cost + cost / 4 { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(
        s.edit_accounted(
            "Cancelled group",
            AccountedEdit::LayerMetadata { layer: id, name: Some("private"), visible: None, opacity: None },
            &ReadControl { interrupt: &Interrupt::NONE, work: &work }
        )
        .is_err()
    );
    assert!(spent.load(Ordering::Relaxed) > cost);
    assert!(s.active().unwrap().doc.shares_state(&before));
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute("layer.setProps", json!({"layer":id.0,"name":"Denied"})).is_err());
    drop(hold);
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
}
