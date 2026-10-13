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
    let before = s.active().unwrap().try_clone_resident().unwrap();
    assert!(file_cmds::open_bytes_as(&mut s, "broken.xcf", b"gimp xcf broken", None, Some("broken.xcf".into())).is_err());
    let after = s.active().unwrap();
    assert_eq!(s.documents().len(), 1);
    assert_eq!(after.doc, before.doc);
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.source, before.source);
    assert_eq!(after.path, before.path);
}

#[test]
fn cancelled_open_preserves_dirty_session_even_after_decode_completion() {
    use photocraft_raster::Interrupt;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    // Count completion notifications without cancellation, then cancel on the
    // final IO notification, after its last internal cancellation check.
    let completions = AtomicUsize::new(0);
    let count = |fraction: f32| {
        if fraction == 1.0 {
            completions.fetch_add(1, Ordering::Relaxed);
        }
    };
    photocraft_io::import_with("new.xcf", &input(), &Interrupt::new(&|| false, &count)).unwrap();
    let last_completion = completions.load(Ordering::Relaxed);
    assert!(last_completion > 0);
    for cancelled_at_start in [true, false] {
        let mut s = Session::new();
        file_cmds::open_bytes_as(&mut s, "source.xcf", &input(), None, Some("source.xcf".into())).unwrap();
        let original = s.active().unwrap().doc.clone();
        s.execute("layer.setProps", json!({"name":"Dirty"})).unwrap();
        let before = s.active().unwrap().try_clone_resident().unwrap();
        let cancelled = AtomicBool::new(cancelled_at_start);
        let observed = AtomicUsize::new(0);
        let check = || cancelled.load(Ordering::Relaxed);
        let progress = |fraction: f32| {
            if fraction == 1.0 && observed.fetch_add(1, Ordering::Relaxed) + 1 == last_completion {
                cancelled.store(true, Ordering::Relaxed);
            }
        };
        let result = file_cmds::open_bytes_as_with(&mut s, "new.xcf", &input(), None, Some("new.xcf".into()), &Interrupt::new(&check, &progress));
        assert!(matches!(result, Err(photocraft_engine::EngineError::Cancelled)));
        assert_eq!(s.documents().len(), 1);
        let after = s.active().unwrap();
        assert_eq!(after.doc, before.doc);
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.path, before.path);
        assert_eq!(after.source, before.source);
        assert_eq!(after.active_layer, before.active_layer);
        assert_eq!(after.selected_layers, before.selected_layers);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, original);
    }
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

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn wrapped_xcf_destination_is_protected_even_when_not_the_open_source() {
    use std::io::Write;
    let dir = std::env::temp_dir().join(format!("photocraft-xcf-wrapped-destination-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let destination = dir.join("unrelated.png");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&input()).unwrap();
    let bytes = encoder.finish().unwrap();
    std::fs::write(&destination, &bytes).unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width":1,"height":1})).unwrap();
    assert!(s.execute("file.saveACopy", json!({"path":destination.to_string_lossy()})).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), bytes);
    let denied = dir.join("new.xcf.gz");
    assert!(s.execute("file.saveACopy", json!({"path":denied.to_string_lossy()})).is_err());
    assert!(!denied.exists());
    std::fs::remove_dir_all(dir).unwrap();
}
