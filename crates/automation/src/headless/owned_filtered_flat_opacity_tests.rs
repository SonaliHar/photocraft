//! Deferred genuine mixed invert sources/native writer/control lifecycle.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_opacity_test_support.rs"]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-mixed-opacity-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "UNRUN genuine +1 source→derived mixedCURRENT compound F64opacity/args/enable, authwait/nativewriter/query/undo/noopredo/rawsource continuity, sourcepartial refusal. No partial-opacity GIMP Source or numerical oracle."]
fn mixed_opacity_public_lifecycle_remains_atomic() {
    for case in &fixture::CASES {
        let dir = Directory::new();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&fixture::workspace().join(case.source)).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let node = original.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.operation.name == "gegl:exposure").unwrap();
        let id = node.id;
        let p = serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0x3fd8000000000001","enabled":false,"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0.75f32.to_bits()}}}]});
        assert!(h.command_start("xcf.filter.set", p.clone(), false).is_err());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("denied".into())));
        assert!(h.command_run("xcf.filter.set", p.clone()).is_err());
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&original));
        let rev = h.session.active().unwrap().revision;
        assert_eq!(h.command_start("xcf.filter.set", p.clone(), true).unwrap()["updated"], true);
        let changed = h.session.active().unwrap().doc.clone();
        assert_eq!(h.session.active().unwrap().revision, rev + 1);
        fixture::payload(&original, &changed);
        assert_eq!(h.command_run("xcf.filter.set", p).unwrap()["updated"], false);
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&original, &h.session.active().unwrap().doc);
        assert!(h.session.active().unwrap().history.can_redo());
        assert_eq!(
            h.command_run("xcf.filter.set", serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":"0x3ff0000000000000"})).unwrap()["updated"],
            false
        );
        assert!(h.session.active().unwrap().history.can_redo());
        h.command_run("edit.redo", serde_json::json!({})).unwrap();
        let q =
            h.inspect_owned_command_with_read_control("xcf.filter.inspect", &serde_json::json!({"filterId":id.0.to_string()}), false, &fixture::ctl()).unwrap();
        assert_eq!(q.value()["opacityF64Hex"], "0x3fd8000000000001");
        drop(q);
        let path = dir.0.join("partial-current.pcraft");
        h.save_owned_native_active(Some(&path), Some("pcraft"), &fixture::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "partial-current.pcraft");
        fixture::storage(&changed, &saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&path).unwrap());
        fixture::same(&saved, &reopened.session.active().unwrap().doc);
        for bits in ["0x0000000000000000", "0x8000000000000000"] {
            reopened.command_run("xcf.filter.set", serde_json::json!({"filterId":id.0.to_string(),"opacityF64Bits":bits})).unwrap();
            let changed = reopened.session.active().unwrap().doc.clone();
            let node = changed.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.id == id).unwrap();
            assert_eq!(format!("0x{:016x}", node.opacity_f64_bits), bits);
            reopened.command_run("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&saved, &reopened.session.active().unwrap().doc);
        }
        assert!(
            reopened
                .command_run("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":true,"opacityF64Bits":"0x3ff0000000000000"}))
                .is_err()
        );
        assert!(reopened.session.active().unwrap().doc.shares_state(&saved));
        fixture::rejects_source(case);
        assert_eq!(fixture::bytes(case), std::fs::read(fixture::workspace().join(case.source)).unwrap());
    }
}
