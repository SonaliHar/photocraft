//! Deferred genuine mixed Exposure sources/native writer/control lifecycle.
use super::*;
#[path = "../../../engine/src/accounted_exposure_loaded_test_support.rs"]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-loaded-exposure-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN four actual ordinaryPoint source/native/query/strictincoming/stops-enable/rawendpoint/undo lifecycle and prepaid synthetic mixedCurrent loadedV2 native10 continuation; authwait rollback, explicitzero rawedit. No genuine mixed endpoint source or numerical oracle."]
fn loaded_endpoints_continue_both_native_classes() {
    for case in &fixture::CASES {
        let dir = Directory::new();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&fixture::workspace().join(case.source)).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let id = original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        let p = serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":0.75f32.to_bits()}}}]});
        assert!(h.command_start("xcf.filter.set", p.clone(), false).is_err());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("denied".into())));
        assert!(h.command_run("xcf.filter.set", p.clone()).is_err());
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&original));
        h.command_run("xcf.filter.set", p).unwrap();
        let changed = h.session.active().unwrap().doc.clone();
        assert_eq!(fixture::words(&changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]), [0.75f32.to_bits(), case.black]);
        for black in [0x3dcc_cccdu32, 0xbdcc_cccd] {
            assert!(
                h.command_run(
                    "xcf.filter.set",
                    serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"black-level","value":{"Number":{"Float32Bits":black}}}]})
                )
                .is_err()
            );
            assert!(h.session.active().unwrap().doc.shares_state(&changed));
        }
        let q =
            h.inspect_owned_command_with_read_control("xcf.filter.inspect", &serde_json::json!({"filterId":id.0.to_string()}), false, &fixture::ctl()).unwrap();
        assert_eq!(q.value()["scalarEditCandidate"], true);
        drop(q);
        let path = dir.0.join("point.pcraft");
        h.save_owned_native_active(Some(&path), Some("pcraft"), &fixture::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "point.pcraft");
        fixture::storage(&changed, &saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&path).unwrap());
        fixture::same(&saved, &reopened.session.active().unwrap().doc);
        assert!(reopened.session.active().unwrap().doc.accounted().unwrap().is_readonly_point_storage());
        reopened
            .command_run(
                "xcf.filter.set",
                serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"black-level","value":{"Number":{"Float32Bits":0}}}]}),
            )
            .unwrap();
        reopened.command_run("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&saved, &reopened.session.active().unwrap().doc);
        let mixed = fixture::mixed_current(if case.precision == 650 { 0 } else { 1 }, case.black);
        let mixedpath = dir.0.join("mixed.pcraft");
        let mut h = Headless::trusted_local();
        h.session.add_owned_native_filtered_flat_archive(fixture::archive(&mixed), "mixed.pcraft", None, &fixture::ctl()).unwrap();
        let node = h
            .session
            .active()
            .unwrap()
            .doc
            .xcf_state
            .as_ref()
            .unwrap()
            .filter_stacks
            .iter()
            .flat_map(|s| &s.nodes)
            .find(|n| n.operation.name == "gegl:exposure")
            .unwrap();
        let id = node.id;
        h.command_run("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":false})).unwrap();
        h.save_owned_native_active(Some(&mixedpath), Some("pcraft"), &fixture::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&mixedpath).unwrap());
        fixture::same(&saved, &reopened.session.active().unwrap().doc);
        assert!(reopened.session.active().unwrap().doc.accounted().unwrap().is_readonly_filtered_flat_point_storage());
        assert_eq!(
            fixture::words(
                &reopened.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.id == id).unwrap()
            )[1],
            case.black
        );
        assert_eq!(fixture::bytes(case), std::fs::read(fixture::workspace().join(case.source)).unwrap());
    }
}
