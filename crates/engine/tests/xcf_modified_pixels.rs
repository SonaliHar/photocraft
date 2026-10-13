//! Painted source pixels are editable; immutable regeneration evidence is not a renderer.
use photocraft_doc::{Document, item_identity::ItemRef};
use photocraft_engine::Session;
use serde_json::json;

fn evidence_unchanged(before: &Document, after: &Document) {
    let before = before.xcf_state.as_ref().unwrap();
    let after = after.xcf_state.as_ref().unwrap();
    assert_eq!(before.source_items, after.source_items);
    assert_eq!(before.resources, after.resources);
    assert_eq!(before.opaque, after.opaque);
}

#[test]
#[ignore = "requires reviewed local modified-source originals"]
fn original_modified_pixels_rename_undo_and_native_preserve_inert_sources() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    for name in ["text-painted.xcf", "vector-modified.xcf", "vector-modified.edit.xcf"] {
        let source = std::fs::read(base.join(name)).unwrap();
        let source_hash = blake3::hash(&source);
        if name.starts_with("vector-") {
            assert!(photocraft_io::import(name, &source).err().unwrap().to_string().contains("regeneration"), "{name}: vector cache is not a renderer");
            assert_eq!(blake3::hash(&std::fs::read(base.join(name)).unwrap()), source_hash);
            continue;
        }
        let imported = photocraft_io::import(name, &source).unwrap();
        assert!(imported.warnings.iter().any(|warning| warning.contains("regeneration is unavailable")), "{name}");
        let mut session = Session::new();
        session.open_document(imported.document, None).unwrap();
        let before = session.active().unwrap().doc.clone();
        let state = before.xcf_state.as_ref().unwrap();
        assert_eq!(state.source_items.len(), 1);
        let ItemRef::Layer(id) = state.source_items[0].owner else { panic!("source layer owner") };
        assert!(state.source_items[0].cache.is_some());
        assert!(state.resources.iter().all(|resource| resource.locator.is_none()));
        session.active_mut().unwrap().active_layer = Some(id);
        session.execute("image.adjustments.invert", json!({})).unwrap();
        let edited = session.active().unwrap().doc.clone();
        assert_ne!(before.layer(id).unwrap().surface(), edited.layer(id).unwrap().surface(), "{name}");
        evidence_unchanged(&before, &edited);
        assert_eq!(before.paths, edited.paths);
        let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(edited.layer(id).unwrap().surface(), reopened.layer(id).unwrap().surface());
        evidence_unchanged(&edited, &reopened);
        assert_eq!(edited.paths, reopened.paths);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, before);
        session.execute("layer.renameLayer", json!({"layer": id.0, "name": "Own renamed painted source"})).unwrap();
        let renamed = &session.active().unwrap().doc;
        assert_eq!(renamed.layer(id).unwrap().name, "Own renamed painted source");
        assert_eq!(renamed.layer(id).unwrap().surface(), before.layer(id).unwrap().surface());
        evidence_unchanged(&before, renamed);
        assert_eq!(renamed.paths, before.paths);
        let native = photocraft_format::save_to_bytes(renamed, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.layer(id).unwrap().name, "Own renamed painted source");
        evidence_unchanged(renamed, &reopened);
        assert_eq!(reopened.paths, before.paths);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(blake3::hash(&std::fs::read(base.join(name)).unwrap()), source_hash);
    }
}
