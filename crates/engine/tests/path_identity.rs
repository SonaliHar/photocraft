//! Point edits select saved paths by exact identity, including duplicate names.
use photocraft_doc::{ColorMode, Document, NamedPath, Path, SampleType, Size, Subpath, item_identity::PathId};
use photocraft_engine::Session;
use serde_json::json;
fn session() -> Session {
    let mut doc = Document::new("paths", Size::new(16, 16), ColorMode::Rgb, SampleType::U8);
    let geometry = Path::new(vec![Subpath::polygon(&[(1.0, 1.0), (6.0, 1.0), (6.0, 6.0)])]);
    for id in [1, 9_007_199_254_740_993] {
        let mut path = NamedPath::new("duplicate", geometry.clone());
        path.id = PathId(id);
        doc.paths.push(path);
    }
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    session
}
#[test]
fn point_edits_choose_large_saved_identity_over_duplicate_name() {
    let mut session = session();
    let before = (*session.active().unwrap().doc).clone();
    session.execute("path.moveAnchors", json!({"pathId":"9007199254740993","name":"duplicate","anchors":[[0,0]],"move":[2,-1]})).unwrap();
    let edited = (*session.active().unwrap().doc).clone();
    assert_eq!(edited.paths[0], before.paths[0]);
    assert_eq!(edited.paths[1].path.subpaths[0].knots[0].anchor.x, 3.0);
    assert_eq!(edited.paths[1].path.subpaths[0].knots[0].anchor.y, 0.0);
    assert_eq!(edited.paths[1].id, before.paths[1].id);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, before);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, edited);
}
#[test]
fn invalid_saved_identity_never_falls_back_to_name() {
    let mut session = session();
    let before = session.active().unwrap().try_clone_resident().unwrap();
    for id in [json!("0"), json!("01"), json!("18446744073709551616"), json!("2"), json!(true), json!(-1), json!(null)] {
        assert!(session.execute("path.moveAnchors", json!({"pathId":id,"name":"duplicate","anchors":[[0,0]],"move":[2,-1]})).is_err());
        assert_eq!(session.active().unwrap().doc, before.doc);
        assert_eq!(session.active().unwrap().revision, before.revision);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
}
