#![cfg(feature = "corpus")]
//! Editable XCF geometry uses existing path commands and atomic history.
use photocraft_engine::Session;
use serde_json::json;
fn source() -> photocraft_doc::Document {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    photocraft_io::import("vector.xcf", &std::fs::read(root.join("vector-rectangle-evidence/source.xcf")).unwrap()).unwrap().document
}
fn projection(doc: &photocraft_doc::Document) -> Vec<u8> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8))
        .collect()
}
#[test]
#[ignore = "requires reviewed unpublished editable vector rectangle source/edit"]
fn current_path_command_matches_gimp_edit_and_undo_native() {
    let doc = source();
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let expected = std::fs::read(root.join("vector-rectangle-evidence/edit.ref.rgba-u8.bin")).unwrap();
    let id = doc.paths[0].id.0;
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    session.execute("path.transform", json!({"pathId":id,"translateX":-2,"translateY":3})).unwrap();
    let edited = (*session.active().unwrap().doc).clone();
    assert_eq!(projection(&edited), expected);
    assert_eq!(edited.layers, doc.layers);
    assert_eq!(edited.xcf_state.as_ref().unwrap().source_items, doc.xcf_state.as_ref().unwrap().source_items);
    let saved = photocraft_io::export(&edited, "edited.pcraft", &Default::default()).unwrap();
    let mut native_expected = edited.clone();
    native_expected.name = "edited.pcraft".into();
    assert_eq!(photocraft_io::import("edited.pcraft", &saved.bytes).unwrap().document, native_expected);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, doc);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, edited);
}
#[test]
#[ignore = "requires reviewed unpublished editable vector source"]
fn unsupported_vector_geometry_and_owner_locks_reject_atomically() {
    for variant in 0..3 {
        let mut doc = source();
        if variant > 0 {
            let layer = &mut doc.layers[0];
            let owner = photocraft_doc::item_identity::ItemRef::Layer(layer.id);
            let locks = &mut doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == owner).unwrap().locks;
            if variant == 1 {
                locks.content = true;
                layer.locks.pixels = true;
            } else {
                locks.position = true;
                layer.locks.position = true;
            }
        }
        let id = doc.paths[0].id.0;
        let params = if variant == 0 { json!({"pathId":id,"angle":15}) } else { json!({"pathId":id,"translateX":-2,"translateY":3}) };
        let mut session = Session::new();
        session.add_document(doc.clone(), None).unwrap();
        assert!(session.execute("path.transform", params).is_err(), "variant {variant}");
        assert_eq!(*session.active().unwrap().doc, doc);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
}
#[test]
#[ignore = "requires reviewed unpublished compound vector source/edit"]
fn current_hole_command_matches_gimp_edit_and_undo_native() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("vector-compound-evidence");
    let raw = std::fs::read(root.join("source.xcf")).unwrap();
    assert_eq!(blake3::hash(&raw).to_hex().as_str(), "a2a97bd684b770a5e5edff834ac378abb0682e192b7563547746b1eddc38d294");
    let expected = std::fs::read(root.join("edit.ref.rgba-u8.bin")).unwrap();
    assert_eq!(blake3::hash(&expected).to_hex().as_str(), "c90abfe436890dd91c29c89c0025a54514fb9112b3364f9a902098be8919d09d");
    let doc = photocraft_io::import("compound.xcf", &raw).unwrap().document;
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    session.execute("path.moveAnchors", json!({"pathId":doc.paths[0].id.0.to_string(),"anchors":[[1,0],[1,1],[1,2],[1,3]],"move":[2,-1]})).unwrap();
    let edited = (*session.active().unwrap().doc).clone();
    assert_eq!(projection(&edited), expected);
    assert_eq!(edited.layers, doc.layers);
    assert_eq!(edited.xcf_state, doc.xcf_state);
    let saved = photocraft_io::export(&edited, "hole.pcraft", &Default::default()).unwrap();
    let mut native_expected = edited.clone();
    native_expected.name = "hole.pcraft".into();
    assert_eq!(photocraft_io::import("hole.pcraft", &saved.bytes).unwrap().document, native_expected);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, doc);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, edited);
}

#[test]
#[ignore = "requires reviewed unpublished same-winding compound source/edit"]
fn same_winding_hole_command_matches_gimp_edit_and_undo_native() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("vector-compound-winding-control");
    let raw = std::fs::read(root.join("source.xcf")).unwrap();
    assert_eq!(blake3::hash(&raw).to_hex().as_str(), "25df1be2cbf8a9e36e60cfb5ea21aa22a5a1e1bd65f65bf7cbdd25c396c09a99");
    let expected = std::fs::read(root.join("edit-reopened.projection.native.bin")).unwrap();
    assert_eq!(blake3::hash(&expected).to_hex().as_str(), "c90abfe436890dd91c29c89c0025a54514fb9112b3364f9a902098be8919d09d");
    let doc = photocraft_io::import("same-winding.xcf", &raw).unwrap().document;
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    session.execute("path.moveAnchors", json!({"pathId":doc.paths[0].id.0.to_string(),"anchors":[[1,0],[1,1],[1,2],[1,3]],"move":[2,-1]})).unwrap();
    let edited = (*session.active().unwrap().doc).clone();
    assert_eq!(projection(&edited), expected);
    assert_eq!(edited.layers, doc.layers);
    assert_eq!(edited.xcf_state, doc.xcf_state);
    let saved = photocraft_io::export(&edited, "same-winding.pcraft", &Default::default()).unwrap();
    let reopened = photocraft_io::import("same-winding.pcraft", &saved.bytes).unwrap().document;
    let mut native_expected = edited.clone();
    native_expected.name = "same-winding.pcraft".into();
    assert_eq!(reopened, native_expected);
    assert_eq!(projection(&reopened), expected);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, doc);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, edited);
}
