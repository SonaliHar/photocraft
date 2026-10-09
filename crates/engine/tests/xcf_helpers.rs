//! Source protection remains effective through file helper commands.
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
use photocraft_engine::{Session, file_cmds};
use serde_json::json;

#[test]
fn native_revert_and_quick_export_retain_xcf_source_protection() {
    let dir = std::env::temp_dir().join(format!("photocraft-xcf-helpers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.png");
    let native = dir.join("working.pcraft");
    let bytes = xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]);
    std::fs::write(&source, &bytes).unwrap();
    let mut s = Session::new();
    file_cmds::open_bytes_as(&mut s, "source.png", &bytes, None, Some(source.to_string_lossy().into_owned())).unwrap();
    let identity = s.active().unwrap().source.clone();
    let saved = photocraft_io::export(&s.active().unwrap().doc, "working.pcraft", &Default::default()).unwrap();
    std::fs::write(&native, saved.bytes).unwrap();
    s.active_mut().unwrap().path = Some(native.to_string_lossy().into_owned());
    s.execute("file.revert", json!({})).unwrap();
    assert_eq!(s.active().unwrap().source, identity);
    // Protection must survive even when the source is replaced externally.
    let replacement = b"externally replaced source";
    std::fs::write(&source, replacement).unwrap();
    assert!(s.execute("file.export.quickExport", json!({"path":source.to_string_lossy()})).is_err());
    assert_eq!(std::fs::read(&source).unwrap(), replacement);
    std::fs::remove_dir_all(&dir).unwrap();
}
