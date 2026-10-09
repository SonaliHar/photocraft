//! Existing command workflows with a bounded synthetic XCF.
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
use photocraft_engine::{Session, file_cmds};
use serde_json::json;
fn input() -> Vec<u8> {
    xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])
}

#[test]
fn renamed_xcf_is_editable_and_undo_retains_source() {
    let mut s = Session::new();
    file_cmds::open_bytes_as(&mut s, "source.psd", &input(), None, Some("source.psd".into())).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.active().unwrap().path.is_none());
    assert!(s.active().unwrap().source.as_ref().unwrap().origin.is_read_only());
    s.execute("layer.setProps", json!({"name":"Edited"})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].name, "Edited");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, *before);
    assert_eq!(s.active().unwrap().source.as_ref().unwrap().path.as_deref(), Some("source.psd"));
}

#[test]
fn failed_open_retains_dirty_document_and_policy() {
    let mut s = Session::new();
    file_cmds::open_bytes_as(&mut s, "source.xcf", &input(), None, Some("source.xcf".into())).unwrap();
    s.execute("layer.setProps", json!({"name":"Dirty"})).unwrap();
    let before = s.active().unwrap().clone();
    assert!(file_cmds::open_bytes_as(&mut s, "broken.xcf", b"gimp xcf broken", None, Some("broken.xcf".into())).is_err());
    let after = s.active().unwrap();
    assert_eq!(s.documents().len(), 1);
    assert_eq!(after.doc, before.doc);
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.source, before.source);
    assert_eq!(after.path, before.path);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn save_copy_cannot_replace_source_or_create_xcf() {
    let dir = std::env::temp_dir().join(format!("photocraft-xcf-workflow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("renamed.psd");
    let denied = dir.join("new.XCF");
    let bytes = input();
    std::fs::write(&source, &bytes).unwrap();
    let source_name = source.to_string_lossy().into_owned();
    let mut s = Session::new();
    file_cmds::open_bytes_as(&mut s, "renamed.psd", &bytes, None, Some(source_name.clone())).unwrap();
    assert!(s.execute("file.saveACopy", json!({"path":source_name})).is_err());
    assert!(s.execute("file.saveACopy", json!({"path":denied.to_string_lossy()})).is_err());
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert!(!denied.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
