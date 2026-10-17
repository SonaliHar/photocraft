//! Real importer behind interactive byte, renamed-file and dropped-file opens.
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug)]
struct Drop(PathBuf, Vec<u8>);
impl egui::DroppedFile for Drop {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.1.clone())
    }
}

#[test]
fn open_and_drop_detect_content_and_preserve_work_on_failure() {
    let services = Services {
        import: Some(Box::new(|name, bytes, depth| photocraft_io::import_with_svg_group_depth(name, bytes, depth).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let bytes = xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]);
    for name in ["project.xcf", "project.XCF", "renamed.psd", "renamed.pcraft", "no-extension"] {
        app.open_file(name, &bytes).unwrap();
        let state = app.session.active().unwrap();
        assert!(state.path.is_none());
        assert!(state.source.as_ref().unwrap().origin.is_read_only());
        assert_eq!(state.doc.layers[0].surface().unwrap().rgba(0, 0), [31.0 / 255.0, 61.0 / 255.0, 91.0 / 255.0, 1.0]);
    }
    app.open_dropped(&egui::Context::default(), vec![Arc::new(Drop("dropped.XCF".into(), bytes))], None);
    assert_eq!(app.session.documents().len(), 6);
    let original = app.session.active().unwrap().doc.clone();
    app.run("layer.setProps", json!({"name":"Edited"})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, original);
    app.run("layer.setProps", json!({"name":"Unsaved"})).unwrap();
    let dirty = app.session.active().unwrap().try_clone_resident().unwrap();
    assert!(app.open_file("broken.xcf", b"gimp xcf bad").is_err());
    let after = app.session.active().unwrap();
    assert_eq!(after.doc, dirty.doc);
    assert_eq!(after.revision, dirty.revision);
    assert_eq!(after.history.past_len(), dirty.history.past_len());
    assert_eq!(after.source, dirty.source);
    assert_eq!(after.path, dirty.path);
}
