//! UNRUN paid native CURRENT workflow; no authored translation or source grant.
use super::*;
#[path = "../../../ui-egui/src/observed_box_move_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-observed-box-move-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "UNRUN: observed Box three-role native CURRENT synchronous Move units/target/gesture/firstlabel/noopredo/query/native continuation and rollback, fixed effectmask/source/profile custody; no GIMP translation or resident preview claim"]
fn observed_box_move_headless_preserves_native_and_command_boundaries() {
    for (sample, assigned) in [(photocraft_doc::SampleType::F32, false), (photocraft_doc::SampleType::F64, false), (photocraft_doc::SampleType::F32, true)] {
        let directory = Directory::new();
        let owner = fixture::owner(sample, assigned, false);
        let input = directory.0.join("input.pcraft");
        // External encoded capture; native opener supplies actual paid custody.
        let bytes = fixture::encoded(&owner);
        std::fs::write(&input, &bytes).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current_bounded(&input).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let layer = before.layers[0].id;
        let source = h.session.active().unwrap().source.clone();
        let params = json!({"layer":layer.0,"dx":1.5,"dy":-2.5,"coalesce":"Move\u{0}metadata"});
        assert!(h.command_start("layer.translate", params.clone(), false).unwrap_err().to_string().contains("synchronous"));
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("Box Move denied".into())));
        assert!(h.command_start("layer.translate", params.clone(), true).unwrap_err().to_string().contains("Box Move denied"));
        h.session.authorize = None;
        let past = h.session.active().unwrap().history.past_len();
        assert!(h.command_start("layer.translate", params, true).unwrap().is_null());
        let moved = h.session.active().unwrap().doc.clone();
        fixture::exact(&before, &moved, 2, -3);
        h.command_run("layer.setProps", json!({"layer":layer.0,"name":"Mixed current native","opacity":0.375,"coalesce":"Move\u{0}metadata"})).unwrap();
        let mixed = h.session.active().unwrap().doc.clone();
        fixture::exact(&before, &mixed, 2, -3);
        assert_eq!(mixed.layers[0].opacity.to_bits(), 0.375f32.to_bits());
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.undo_label(), Some("Move"));
        h.command_run("edit.undo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        h.command_run("layer.translate", json!({"dx":null,"dy":"ignored","units":null,"layer":u64::MAX,"coalesce":"Move\u{0}metadata"})).unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        h.command_run("edit.redo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&mixed));
        // Named target + points use the actual document DPI and existing parser.
        let dpi = mixed.resolution_dpi;
        let dx = f64::from(dpi).round() as i32;
        h.command_run("layer.translate", json!({"layer":"Mixed current native","units":"pt","dx":72.0,"dy":0.0})).unwrap();
        let points = h.session.active().unwrap().doc.clone();
        fixture::exact(&mixed, &points, dx, 0);
        for params in [
            json!({"target":"mask","dx":1}),
            json!({"target":"quickMask","dx":1}),
            json!({"layer":u64::MAX,"dx":1}),
            json!({"dx":i32::MAX}),
            json!({"units":"pt","dx":null}),
            json!({"dx":1,"coalesce":"x".repeat(129)}),
        ] {
            assert!(h.command_run("layer.translate", params).is_err());
        }
        assert!(h.session.active().unwrap().doc.shares_state(&points));
        assert_eq!(h.session.active().unwrap().source, source);
        assert!(!h.session.has_jobs());
        let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, false, &fixture::ctl()).unwrap();
        assert_eq!(reply.value()["items"]["total"], 2);
        assert_eq!(reply.value()["filters"]["total"], 1);
        drop(reply);
        let output = directory.0.join("continued.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &fixture::ctl()).unwrap();
        assert_eq!(std::fs::read(&input).unwrap(), bytes);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&output).unwrap());
        fixture::exact(&points, &reopened.session.active().unwrap().doc, 0, 0);
        reopened.command_run("layer.translate", json!({"layer":layer.0,"dx":-1,"dy":1})).unwrap();
        fixture::exact(&points, &reopened.session.active().unwrap().doc, -1, 1);
        let state = reopened.session.active().unwrap().doc.clone();
        reopened.command_run("layer.setProps", json!({"visible":false})).unwrap();
        let hidden = reopened.session.active().unwrap().doc.clone();
        assert!(reopened.command_run("layer.translate", json!({"dx":1})).is_err());
        assert!(reopened.session.active().unwrap().doc.shares_state(&hidden));
        reopened.command_run("edit.undo", Value::Null).unwrap();
        assert!(reopened.session.active().unwrap().doc.shares_state(&state));
    }
    let directory = Directory::new();
    let path = directory.0.join("locked.pcraft");
    let owner = fixture::owner(photocraft_doc::SampleType::F32, false, true);
    std::fs::write(&path, fixture::encoded(&owner)).unwrap();
    let mut h = Headless::trusted_local();
    drop(h.open_owned_native_keep_current_bounded(&path).unwrap());
    let before = h.session.active().unwrap().doc.clone();
    assert!(h.command_run("layer.translate", json!({"dx":1})).is_err());
    assert!(h.command_run("layer.translate", json!({"dx":0})).unwrap().is_null());
    assert!(h.session.active().unwrap().doc.shares_state(&before));
}
