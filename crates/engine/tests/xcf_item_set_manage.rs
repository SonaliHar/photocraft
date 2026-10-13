//! Controlled saved-set metadata transactions; drafts intentionally unrun.
use photocraft_doc::{
    Color, ColorMode, Document, SampleType, Size, XcfState,
    item_identity::{ItemRef, ItemSetId},
    xcf_editing::{ItemFamily, ItemSet, ItemSetRule},
    xcf_provenance::OpaqueRecord,
};
use photocraft_engine::{
    Session,
    xcf_editing::{delete_set_with_interrupt, rename_set_with_interrupt, set_members_with_interrupt},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
fn fixture() -> Session {
    let mut document = Document::with_background("Own set metadata", Size::new(2, 2), ColorMode::Rgb, SampleType::U16, Color::WHITE);
    let owner = document.layers[0].id;
    let mut state = XcfState::default();
    state.reconcile_layers(&document).unwrap();
    state.items[0].source_flags = 0x40;
    state.items[0].source_tattoo = Some(41);
    state.selection.selected = vec![ItemRef::Layer(owner)];
    state.selection.active = Some(ItemRef::Layer(owner));
    state.item_sets = vec![
        ItemSet { id: ItemSetId(7), name: "Own named set".into(), family: ItemFamily::Layer, rule: ItemSetRule::Named, members: vec![ItemRef::Layer(owner)] },
        ItemSet { id: ItemSetId(8), name: "Retained query".into(), family: ItemFamily::Layer, rule: ItemSetRule::Regex("[".into()), members: vec![] },
    ];
    let bytes = vec![0, 17, 255];
    state.opaque.push(OpaqueRecord {
        owner: None,
        namespace: "own".into(),
        identity: "inert".into(),
        source_version: 22,
        order: 4,
        flags: 7,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        bytes: bytes.into(),
        rendering_required: false,
    });
    document.xcf_state = Some(state);
    document.validate_xcf_state().unwrap();
    let mut session = Session::new();
    session.add_document(document, None).unwrap();
    session
}
fn target(session: &Session, set: u64) -> Value {
    let active = session.active().unwrap();
    json!({"documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string(),"setId":set.to_string()})
}
#[test]
fn captured_rename_delete_preserve_native_source_selection_and_one_undo() {
    let mut session = fixture();
    let before = session.active().unwrap().doc.clone();
    let mut p = target(&session, 7);
    p["name"] = json!("Own renamed set");
    rename_set_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
    let after = session.active().unwrap().doc.clone();
    let mut expected = before.as_ref().clone();
    expected.xcf_state.as_mut().unwrap().item_sets[0].name = "Own renamed set".into();
    assert_eq!(*after, expected);
    let raw = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&raw).unwrap(), *after);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, before);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, after);
    let p = target(&session, 7);
    delete_set_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
    let deleted = session.active().unwrap().doc.clone();
    let mut expected = after.as_ref().clone();
    expected.xcf_state.as_mut().unwrap().item_sets.remove(0);
    assert_eq!(*deleted, expected);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, after);
}
#[test]
fn legacy_numeric_pattern_metadata_and_named_membership_are_compatible() {
    let mut session = fixture();
    rename_set_with_interrupt(&mut session, &json!({"setId":8,"name":"Query renamed without evaluation"}), &Interrupt::NONE).unwrap();
    assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().item_sets[1].rule, ItemSetRule::Regex("[".into()));
    assert!(set_members_with_interrupt(&mut session, &json!({"setId":8,"members":[]}), &Interrupt::NONE).is_err());
    let owner = session.active().unwrap().doc.layers[0].id;
    set_members_with_interrupt(&mut session, &json!({"setId":7,"members":[]}), &Interrupt::NONE).unwrap();
    let mut p = target(&session, 7);
    p["members"] = json!([{"Layer":owner.0}]);
    set_members_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
    assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().item_sets[0].members, vec![ItemRef::Layer(owner)]);
    delete_set_with_interrupt(&mut session, &json!({"setId":8}), &Interrupt::NONE).unwrap();
}
#[test]
fn stale_partial_pair_float_cancel_and_noop_preserve_real_redo() {
    let mut session = fixture();
    rename_set_with_interrupt(&mut session, &json!({"setId":7,"name":"Seed redo"}), &Interrupt::NONE).unwrap();
    session.execute("edit.undo", json!({})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let mut p = target(&session, 7);
    p["name"] = json!("Own named set");
    assert_eq!(rename_set_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap()["updated"], false);
    assert_eq!(session.active().unwrap().revision, revision);
    for patch in [json!({"expectedRevision":"999999"}), json!({"documentId":1}), json!({"setId":7.5}), json!({"name":"\u{0}"})] {
        let mut p = target(&session, 7);
        p["name"] = json!("Change");
        for (k, v) in patch.as_object().unwrap() {
            p[k] = v.clone();
        }
        assert!(rename_set_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
    }
    assert!(delete_set_with_interrupt(&mut session, &json!({"setId":7,"documentId":before.id.0.to_string()}), &Interrupt::NONE).is_err());
    let p = target(&session, 7);
    let stop = || true;
    assert!(delete_set_with_interrupt(&mut session, &p, &Interrupt::cancel_only(&stop)).is_err());
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().revision, revision);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().item_sets[0].name, "Seed redo");
}
