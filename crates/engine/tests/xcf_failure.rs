//! Pending XCF imports cannot replace existing work on failure or cancellation.
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
use photocraft_engine::{
    Session,
    jobs::{OpenSource, Started},
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn cancelled_or_failed_background_open_preserves_dirty_session() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":2,"height":2})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let before = s.active().unwrap().try_clone_resident().unwrap();
    let bytes = xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]);
    let Started::Job(job) = s.start_open("project.xcf", OpenSource::Bytes(Arc::new(bytes))).unwrap() else {
        panic!("expected native job");
    };
    assert!(s.cancel_job(job));
    assert!(s.wait_job(job).is_err());
    let Started::Job(job) = s.start_open("broken.xcf", OpenSource::Bytes(Arc::new(b"gimp xcf bad".to_vec()))).unwrap() else {
        panic!("expected native job");
    };
    assert!(s.wait_job(job).is_err());
    assert_eq!(s.documents().len(), 1);
    let after = s.active().unwrap();
    assert_eq!(after.doc, before.doc);
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.saved_revision, before.saved_revision);
    assert_eq!(after.history.past_len(), before.history.past_len());
    assert_eq!(after.source, before.source);
    assert_eq!(after.path, before.path);
}
