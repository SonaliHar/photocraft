//! Lossless canonical layer selector drafts; intentionally unrun during implementation.
use photocraft_doc::{Color, ColorMode, Document, LayerId, SampleType, Size};
use photocraft_engine::{Session, layer_multi_cmds::rename_layer_with_interrupt};
use photocraft_raster::Interrupt;
use serde_json::json;
fn fixture(id: u64) -> Session {
    let mut doc = Document::with_background("Canonical rename", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    doc.layers[0].id = LayerId(id);
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    session
}
#[test]
fn canonical_high_id_and_max_id_rename_native_and_history_exact() {
    for id in [(1u64 << 53) + 1, u64::MAX] {
        let mut session = fixture(id);
        let before = session.active().unwrap().doc.clone();
        let active = session.active().unwrap();
        let p = json!({"layerId":id.to_string(),"name":"Exact high identity","documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string()});
        rename_layer_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
        let after = session.active().unwrap().doc.clone();
        assert_eq!(after.layers[0].id, LayerId(id));
        assert_eq!(after.layers[0].name, "Exact high identity");
        let bytes = photocraft_format::save_to_bytes(&after, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), *after);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, before);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, after);
    }
}
#[test]
fn stale_duplicate_and_malformed_selectors_preserve_real_redo() {
    let id = (1u64 << 53) + 1;
    let mut session = fixture(id);
    rename_layer_with_interrupt(&mut session, &json!({"layer":id,"name":"Seed"}), &Interrupt::NONE).unwrap();
    session.execute("edit.undo", json!({})).unwrap();
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    for p in [
        json!({"layer":id,"layerId":id.to_string(),"name":"Ambiguous"}),
        json!({"layerId":id.to_string(),"name":"Stale","documentId":before.id.0.to_string(),"expectedRevision":revision.saturating_sub(1).to_string()}),
        json!({"layerId":"01","name":"Bad"}),
        json!({"layerId":"0","name":"Bad"}),
        json!({"layerId":"18446744073709551616","name":"Overflow"}),
        json!({"layerId":id,"name":"Wrong type"}),
        json!({"layer":id,"name":"Unknown","unknown":true}),
    ] {
        assert!(rename_layer_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
    }
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.layers[0].name, "Seed");
}
