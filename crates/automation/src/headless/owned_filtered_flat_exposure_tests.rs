//! Deferred genuine mixed Exposure sources/native writer/control lifecycle.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_exposure_test_support.rs"]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-mixed-exposure-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN bothauthentic8x6 packets exact1:0 Exposure targets/twoF32 newepoch, partialset+atomicenabled/authwait/refusalrollback/history/noopredo/nativewriter/query/sourcewordcontinuity; fullkernelinvalidpromotedblackendpoint/overflow refuses. No producerprojection/profile/Floatoracle."]
fn mixed_exposure_public_commands_continue_native() {
    for case in &fixture::CASES {
        let dir = Directory::new();
        let source = fixture::bytes(case);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&fixture::workspace().join(case.source)).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let targets = original
            .xcf_state
            .as_ref()
            .unwrap()
            .filter_stacks
            .iter()
            .flat_map(|s| s.nodes.iter().filter(|n| n.operation.name == "gegl:exposure").map(|n| n.id))
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 3);
        for id in targets {
            let before = h.session.active().unwrap().doc.clone();
            let p = serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0.875f32.to_bits()}}},{"name":"black-level","value":{"Number":{"Float32Bits":(-0.0625f32).to_bits()}}}],"enabled":false});
            assert!(h.command_start("xcf.filter.set", p.clone(), false).is_err());
            h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("denied".into())));
            assert!(h.command_run("xcf.filter.set", p.clone()).is_err());
            h.session.authorize = None;
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            assert_eq!(h.command_start("xcf.filter.set", p.clone(), true).unwrap()["updated"], true);
            let after = h.session.active().unwrap().doc.clone();
            fixture::payload(&before, &after);
            assert_eq!(h.command_run("xcf.filter.set", p).unwrap()["updated"], false);
            h.command_run("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &h.session.active().unwrap().doc);
            assert_eq!(h.command_run("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":true})).unwrap()["updated"], false);
            assert!(h.session.active().unwrap().history.can_redo());
            h.command_run("edit.redo", serde_json::json!({})).unwrap();
            fixture::same(&after, &h.session.active().unwrap().doc);
            let q = h
                .inspect_owned_command_with_read_control("xcf.filter.inspect", &serde_json::json!({"filterId":id.0.to_string()}), false, &fixture::ctl())
                .unwrap();
            assert_eq!(q.value()["filterId"], id.0.to_string());
            assert_eq!(q.value()["scalarEditCandidate"], true);
            drop(q);
            h.command_run("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":true})).unwrap();
            let active = h.session.active().unwrap().doc.clone();
            let path = dir.0.join(format!("target-{}.pcraft", id.0));
            h.save_owned_native_active(Some(&path), Some("pcraft"), &fixture::ctl()).unwrap();
            let saved = h.session.active().unwrap().doc.clone();
            assert_eq!(saved.name, path.file_name().unwrap().to_str().unwrap());
            fixture::storage(&active, &saved);
            let mut reopened = Headless::trusted_local();
            drop(reopened.open_owned_native_keep_current_bounded(&path).unwrap());
            fixture::same(&saved, &reopened.session.active().unwrap().doc);
            for (name, bits) in [("black-level", 0.1f32.to_bits()), ("exposure", f32::MAX.to_bits())] {
                let bad = serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":name,"value":{"Number":{"Float32Bits":bits}}}]});
                let before = reopened.session.active().unwrap().doc.clone();
                assert!(reopened.command_run("xcf.filter.set", bad).is_err());
                assert!(reopened.session.active().unwrap().doc.shares_state(&before));
            }
            // Start next target from authentic source state rather than filename-title edited history.
            let mut next = Headless::trusted_local();
            drop(next.open_owned_xcf_keep_current_bounded(&fixture::workspace().join(case.source)).unwrap());
            h = next;
        }
        assert_eq!(std::fs::read(fixture::workspace().join(case.source)).unwrap(), source);
        assert!(!h.session.has_jobs());
    }
}
