//! UNRUN existing compound metadata semantics on genuine readonly flat sources.
//! Native/source captures and separately paid CURRENT controls are harness data.
use super::*;
use photocraft_doc::{
    accounted_document::{AccountedDocument, AccountedEdit},
    item_identity::ItemRef,
};
use photocraft_raster::backing::quota::QuotaKind;
fn session(i: usize) -> (Session, BackingQuota) {
    let q = quota();
    let imported = source(i, &q);
    let mut s = Session::new();
    s.add_owned_separable_blend_xcf_import(&imported, "flat", None, &ctl()).unwrap();
    drop(imported);
    (s, q)
}
fn ids(s: &Session) -> [photocraft_doc::LayerId; 2] {
    let l = &s.active().unwrap().doc.layers;
    [l[0].id, l[1].id]
}
fn current_control(lock: usize) -> (Session, BackingQuota) {
    let (s, q) = session(0);
    let original = s.active().unwrap().doc.accounted().unwrap().clone();
    let measured = photocraft_io::retained_native_model_census(original.document(), Default::default(), &Interrupt::NONE).unwrap();
    let index = q.reserve(QuotaKind::Index, (measured.retained_bytes() as u64) * 2 + 8192).unwrap();
    let resident = q.reserve(QuotaKind::Resident, (original.document().icc_profile.as_ref().map_or(0, Vec::capacity) as u64) * 2 + 8192).unwrap();
    let mut doc = original.document().clone();
    let id = doc.layers[1].id;
    match lock {
        1 => doc.layers[1].locks.all = true,
        2 => doc.layers[1].locks.pixels = true,
        3 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().locks.content = true,
        4 => doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(id)).unwrap().locks.visibility = true,
        _ => {}
    }
    let actual = photocraft_io::retained_native_model_census(&doc, Default::default(), &Interrupt::NONE).unwrap();
    assert!(actual.retained_bytes() as u64 <= index.bytes());
    assert!(doc.icc_profile.as_ref().map_or(0, Vec::capacity) as u64 <= resident.bytes());
    let owner = AccountedDocument::from_owned_readonly_flat_composite(doc, std::sync::Arc::new((original, index, resident)), q.clone(), &ctl()).unwrap();
    crate::accounted_xcf_open::separable_blend::validate_current_separable_blend_storage(&owner, &ctl()).unwrap();
    let mut current = Session::new();
    let states = current.prefs().performance.history_states.max(1) as usize;
    let state = crate::DocState::new_accounted(owner, None, states, &ctl()).unwrap();
    current.publish_document_state(state, &ctl()).unwrap();
    drop(s);
    (current, q)
}
#[test]
#[ignore = "UNRUN: genuine50 packets both IDs compound metadata onehistory rawzero/native/profile/source retention and native continuation"]
fn genuine_metadata_all_modes_and_native() {
    for case in 0..CASES.len() {
        for target in 0..2 {
            let (mut s, q) = session(case);
            let id = ids(&s)[target];
            let before = s.active().unwrap().doc.clone();
            let raw = native(&s);
            let source = s.active().unwrap().source.clone();
            let rev = s.active().unwrap().revision;
            assert!(crate::owned_layer_metadata_candidate(before.accounted().unwrap()));
            assert!(!crate::assigned_geometry_metadata_candidate(before.accounted().unwrap()));
            assert_eq!(
                s.execute("layer.setProps", serde_json::json!({"layer":id.0,"name":"Metadata","visible":false,"opacity":-0.0})).unwrap(),
                serde_json::Value::Null
            );
            let state = s.active().unwrap();
            assert_eq!(state.revision, rev + 1);
            assert_eq!(state.history.past_len(), 1);
            assert_eq!(state.history.undo_label(), Some("Layer Properties"));
            let layer = state.doc.layers.iter().find(|l| l.id == id).unwrap();
            assert_eq!(layer.opacity.to_bits(), (-0.0f32).to_bits());
            assert!(!layer.visible);
            assert_eq!(layer.name, "Metadata");
            assert_eq!(native(&s), raw);
            assert_eq!(state.source, source);
            assert_eq!(state.doc.icc_profile, before.icc_profile);
            assert_eq!(state.doc.source_color_context, before.source_color_context);
            assert!(s.undo_with_control(&ctl()).unwrap());
            assert!(s.redo_with_control(&ctl()).unwrap());
            let bytes = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "FlatMetadata.pcraft", &ctl())
                .unwrap()
                .write_and_publish(&mut s, &ctl(), |doc, _| {
                    photocraft_format::owned_document::save_current_document_archive(doc, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                        .map(|w| w.into_inner())
                        .map_err(|e| EngineError::Other(e.to_string()))
                })
                .unwrap();
            let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(
                &mut Cursor::new(&bytes),
                bytes.len() as u64,
                Default::default(),
                q.clone(),
                &ctl(),
            )
            .unwrap();
            let mut reopened = Session::new();
            reopened.add_owned_native_separable_blend_archive(archive, "native", None, &ctl()).unwrap();
            assert_eq!(reopened.active().unwrap().doc.xcf_state, s.active().unwrap().doc.xcf_state);
            assert_eq!(native(&reopened), raw);
        }
    }
}
#[test]
#[ignore = "UNRUN: opaque crosslayer key/firstlabel, wrongtyped omission, reverse duplicate name selection, direct caller label and noopredo"]
fn metadata_keys_names_and_direct_route() {
    let (mut s, _) = session(0);
    let [base, top] = ids(&s);
    let before = s.active().unwrap().doc.clone();
    s.execute("layer.setProps", serde_json::json!({"layer":top.0,"visible":false,"coalesce":""})).unwrap();
    s.execute("layer.setProps", serde_json::json!({"layer":base.0,"name":"Same","opacity":0.375,"coalesce":""})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Layer Visibility"));
    assert!(s.undo_with_control(&ctl()).unwrap());
    let h = s.active().unwrap().history.clone();
    assert_eq!(
        s.execute("layer.setProps", serde_json::json!({"layer":top.0,"name":false,"visible":"ignored","opacity":null,"coalesce":false})).unwrap(),
        serde_json::Value::Null
    );
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    assert!(s.active().unwrap().history.can_redo());
    assert!(s.redo_with_control(&ctl()).unwrap());
    s.execute("layer.setProps", serde_json::json!({"layer":top.0,"name":"Same"})).unwrap();
    s.execute("layer.setProps", serde_json::json!({"layer":"Same","opacity":0.625})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[1].opacity, 0.625);
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.375);
    s.edit_accounted("Direct label", AccountedEdit::LayerMetadata { layer: base, name: Some("Direct"), visible: None, opacity: None }, &ctl()).unwrap();
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Direct label"));
    assert_eq!(s.active().unwrap().doc.icc_profile, before.icc_profile);
}
#[test]
#[ignore = "UNRUN: paid CURRENT native/content locks do not prohibit name/opacity; visibility lock only changed visibility; unrelated grants refused"]
fn locks_and_no_other_edit_grants() {
    for lock in 1..=4 {
        let (mut s, _) = current_control(lock);
        let top = ids(&s)[1];
        s.execute("layer.setProps", serde_json::json!({"layer":top.0,"name":"Allowed","opacity":0.5,"visible":true})).unwrap();
        let before = s.active().unwrap().doc.clone();
        if lock == 4 {
            assert!(s.execute("layer.setProps", serde_json::json!({"layer":top.0,"visible":false})).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
        }
        for (cmd, p) in [
            ("layer.translate", serde_json::json!({"layer":top.0,"dx":1})),
            ("xcf.mask.enabled.set", serde_json::json!({"maskId":"1","enabled":false})),
            ("xcf.filter.enable", serde_json::json!({"filterId":"1","enabled":false})),
            ("layer.setProps", serde_json::json!({"blend":"multiply"})),
        ] {
            assert!(s.execute(cmd, p).is_err());
            assert!(s.active().unwrap().doc.shares_state(&before));
        }
    }
}
#[test]
#[ignore = "UNRUN: prepared full history/source/view/channel/journal fences and cumulativework/fullquota rollback without prefix"]
fn full_fences_and_resource_atomicity() {
    for fault in 0..5 {
        let (mut s, _) = session(0);
        let before = s.active().unwrap().doc.clone();
        let p = serde_json::json!({"layer":ids(&s)[1].0,"name":"Staged"});
        let prepared = crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &ctl()).unwrap();
        match fault {
            0 => s.active_mut().unwrap().history.max_states += 1,
            1 => s.active_mut().unwrap().source_read_only = false,
            2 => s.active_mut().unwrap().channel_view.quick_mask_hidden = true,
            3 => s.active_mut().unwrap().view_revision += 1,
            _ => s.journal.push(("external".into(), serde_json::Value::Null)),
        };
        assert!(prepared.execute(&mut s, &ctl()).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
    }
    let (mut s, q) = session(0);
    let before = s.active().unwrap().doc.clone();
    let h = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let p = serde_json::json!({"name":"Denied"});
    let work = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(crate::accounted_commands::PreparedAccountedCommand::prepare(&s, "layer.setProps", &p, &c).is_err());
    let hold = q.reserve(QuotaKind::Index, (32u64 << 20) - q.usage().unwrap().index).unwrap();
    assert!(s.execute("layer.setProps", p).is_err());
    drop(hold);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&h));
    assert_eq!(s.journal, journal);
}
