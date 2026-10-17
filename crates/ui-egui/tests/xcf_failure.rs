//! Background XCF failure/cancellation leaves the visible document and recents intact.
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
use photocraft_ui_egui::{PhotocraftApp, Services, jobs_ui};
use serde_json::json;
use std::time::{Duration, Instant};

#[test]
fn cancelled_and_failed_open_preserve_dirty_ui_document() {
    let services = Services {
        import: Some(Box::new(|name, bytes, depth| photocraft_io::import_with_svg_group_depth(name, bytes, depth).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let bytes = xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255]);
    app.open_file("original.xcf", &bytes).unwrap();
    app.run("layer.setProps", json!({"name":"Unsaved"})).unwrap();
    let before = app.session.active().unwrap().try_clone_resident().unwrap();
    let recents = app.ui.recent_files.clone();
    app.background_jobs = true;
    app.open_file("cancelled.xcf", &bytes).unwrap();
    let job = app.session.jobs()[0].id;
    jobs_ui::cancel(&mut app, job);
    let ctx = egui::Context::default();
    jobs_ui::tick(&mut app, &ctx);
    app.open_file("broken.xcf", b"gimp xcf bad").unwrap();
    let start = Instant::now();
    while app.session.has_jobs() {
        jobs_ui::tick(&mut app, &ctx);
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(app.session.documents().len(), 1);
    let after = app.session.active().unwrap();
    assert_eq!(after.doc, before.doc);
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.history.past_len(), before.history.past_len());
    assert_eq!(after.source, before.source);
    assert_eq!(after.path, before.path);
    assert_eq!(app.ui.recent_files, recents);
    assert!(app.ui.notices.iter().any(|notice| notice.error));
}
