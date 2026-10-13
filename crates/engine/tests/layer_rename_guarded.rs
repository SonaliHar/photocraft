//! Compatible bounded ordinary naming and imported metadata preservation.
use photocraft_doc::{
    Color, ColorMode, Document, SampleType, Size, XcfState,
    item_identity::{ItemRef, ResourceId},
    xcf_items::{EditableSource, SourceCache, SourceItem},
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_engine::{Session, layer_multi_cmds::rename_layer_with_interrupt};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
fn fixture(source: bool) -> Session {
    let mut doc = Document::with_background("Own rename", Size::new(2, 2), ColorMode::Rgb, SampleType::U16, Color::WHITE);
    if source {
        let mut state = XcfState::default();
        state.reconcile_layers(&doc).unwrap();
        state.items[0].custom_name = false;
        state.items[0].source_flags = 0x40;
        state.items[0].source_tattoo = Some(41);
        state.resources.push(ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::LinkedImage,
            name: "Inert source".into(),
            collection: None,
            internal: false,
            locator: Some("own:never-read".into()),
            content_hash: None,
            owned_bytes: None,
            resolution: None,
        });
        let bytes = vec![1, 0, 2, 0, 3, 0, 255, 255];
        state.source_items.push(SourceItem {
            owner: ItemRef::Layer(doc.layers[0].id),
            source: EditableSource::Linked { resource: ResourceId(1), last_verified_hash: None, configuration: vec![], current: None },
            transform: None,
            cache: Some(SourceCache {
                blake3: blake3::hash(&bytes).to_hex().to_string(),
                bytes: bytes.into(),
                width: 1,
                height: 1,
                format: photocraft_doc::PixelFormat::RGBA16,
            }),
        });
        doc.xcf_state = Some(state);
        doc.validate_xcf_state().unwrap();
    }
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    session
}
fn request(s: &Session, name: &str) -> Value {
    let active = s.active().unwrap();
    json!({"layer":active.doc.layers[0].id.0,"name":name,"documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string()})
}
#[test]
fn imported_trimmed_name_updates_only_name_custom_flag_native_and_history() {
    let mut session = fixture(true);
    let before = session.active().unwrap().doc.clone();
    let p = request(&session, "  Own renamed layer  ");
    rename_layer_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
    let after = session.active().unwrap().doc.clone();
    let mut expected = before.as_ref().clone();
    expected.layers[0].name = "Own renamed layer".into();
    let item = &mut expected.xcf_state.as_mut().unwrap().items[0];
    item.name = "Own renamed layer".into();
    item.custom_name = true;
    assert_eq!(*after, expected);
    let native = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), *after);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, before);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, after);
}
#[test]
fn ordinary_native_locks_do_not_invent_name_content_policy() {
    let mut session = fixture(false);
    session.active_mut().unwrap().doc.try_resident_mut().unwrap().layers[0].locks.all = true;
    rename_layer_with_interrupt(&mut session, &json!({"name":"  Ordinary locked rename  "}), &Interrupt::NONE).unwrap();
    assert_eq!(session.active().unwrap().doc.layers[0].name, "Ordinary locked rename");
}
#[test]
fn source_content_lock_stale_bad_names_and_cancel_preserve_snapshot() {
    let mut session = fixture(true);
    session.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state.as_mut().unwrap().items[0].locks.content = true;
    let p = request(&session, "Locked source");
    let before = session.active().unwrap().doc.clone();
    assert!(rename_layer_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
    assert_eq!(session.active().unwrap().doc, before);
    let mut session = fixture(true);
    let before = session.active().unwrap().doc.clone();
    for name in ["\u{0}".to_string(), " ".to_string(), "x".repeat((1 << 20) + 1)] {
        let p = request(&session, &name);
        assert!(rename_layer_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
    }
    let mut p = request(&session, "Change");
    p["expectedRevision"] = json!("999999");
    assert!(rename_layer_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
    let p = request(&session, "Change");
    let stop = || true;
    assert!(rename_layer_with_interrupt(&mut session, &p, &Interrupt::cancel_only(&stop)).is_err());
    assert_eq!(session.active().unwrap().doc, before);
}
#[test]
fn trim_noop_preserves_revision_and_real_redo() {
    let mut session = fixture(false);
    let old = session.active().unwrap().doc.layers[0].name.clone();
    rename_layer_with_interrupt(&mut session, &json!({"name":"Seed redo"}), &Interrupt::NONE).unwrap();
    session.execute("edit.undo", json!({})).unwrap();
    let revision = session.active().unwrap().revision;
    let p = request(&session, &format!("  {old}  "));
    rename_layer_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
    assert_eq!(session.active().unwrap().revision, revision);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.layers[0].name, "Seed redo");
}
