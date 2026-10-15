//! UNRUN real authored sources through explicit path/native lifecycle APIs.
use super::*;
#[path = "../../../ui-egui/src/observed_box_source_activity_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-box-source-activity-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: fifteen actual independent GIMP activity sources through bounded held-path/typed native/paidquery, exact raw6/33/native/profile/provenance, source fingerprint protection, disabled original/native/enable/undo and CURRENT metadata/Move/auth/sync; no derived numerical acceptance"]
fn observed_source_activity_headless_preserves_source_and_current_authority() {
    for case in fixture::cases() {
        let directory = Directory::new();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&case.path).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        fixture::observed(&case, &original);
        assert!(h.session.active().unwrap().source_read_only);
        let identity = h.session.active().unwrap().source.clone();
        let sourcecopy = directory.0.join("source-copy.pcraft");
        std::fs::write(&sourcecopy, &case.bytes).unwrap();
        // Correct extension forces real fingerprint protection, not an XCF
        // destination-extension refusal. Copied source bytes confer no rights.
        assert!(h.save_owned_native_active(Some(&sourcecopy), Some("pcraft"), &fixture::ctl()).is_err());
        assert_eq!(std::fs::read(&sourcecopy).unwrap(), case.bytes);
        let initial = directory.0.join("original.pcraft");
        h.save_owned_native_active(Some(&initial), Some("pcraft"), &fixture::ctl()).unwrap();
        assert_eq!(std::fs::read(&case.path).unwrap(), case.bytes);
        let mut continued = Headless::trusted_local();
        drop(continued.open_owned_native_keep_current_bounded(&initial).unwrap());
        let native = continued.session.active().unwrap().doc.clone();
        fixture::observed(&case, &native);
        fixture::exact(&original, &native, 0, 0);
        let layer = native.layers[0].id;
        let filter = native.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        if !case.enabled {
            assert_eq!(continued.command_run("xcf.filter.enable", json!({"filterId":filter,"enabled":true})).unwrap()["updated"], true);
            continued.command_run("edit.undo", Value::Null).unwrap();
            assert!(continued.session.active().unwrap().doc.shares_state(&native));
        }
        for wait in [false, true] {
            let query = continued.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, wait, &fixture::ctl()).unwrap();
            assert_eq!(query.value()["filters"]["total"], 1);
            assert_eq!(query.value()["items"]["total"], 2);
            drop(query);
        }
        let p = json!({"layer":layer.0,"name":"Source CURRENT editable","visible":true,"opacity":0.5,"coalesce":"sourceactivity"});
        assert!(continued.command_start("layer.setProps", p.clone(), false).unwrap_err().to_string().contains("synchronous"));
        continued.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("activity edit denied".into())));
        assert!(continued.command_start("layer.setProps", p.clone(), true).unwrap_err().to_string().contains("activity edit denied"));
        continued.session.authorize = None;
        assert!(continued.command_start("layer.setProps", p, true).unwrap().is_null());
        continued.command_run("layer.translate", json!({"layer":"Source CURRENT editable","dx":1,"dy":-1,"coalesce":"sourceactivity"})).unwrap();
        let changed = continued.session.active().unwrap().doc.clone();
        fixture::exact(&native, &changed, 1, -1);
        assert_eq!(continued.session.active().unwrap().history.undo_label(), Some("Layer Properties"));
        continued.command_run("edit.undo", Value::Null).unwrap();
        assert!(continued.session.active().unwrap().doc.shares_state(&native));
        let history = continued.session.active().unwrap().history.clone();
        let zero = continued.command_run("layer.translate", json!({"dx":0,"coalesce":"sourceactivity"}));
        // Existing hidden dispatch guard precedes the handler's zero shortcut.
        if case.visible {
            assert!(zero.unwrap().is_null());
        } else {
            assert!(zero.is_err());
        }
        assert!(continued.session.active().unwrap().history.shares_accounted_history(&history));
        continued.command_run("edit.redo", Value::Null).unwrap();
        assert!(continued.session.active().unwrap().doc.shares_state(&changed));
        if !case.visible || case.opacity == 0 {
            let malformed = directory.0.join("malformed.xcf");
            std::fs::write(&malformed, fixture::malformed_inactive(&case)).unwrap();
            let count = continued.session.documents().len();
            assert!(continued.open_owned_xcf_keep_current_bounded(&malformed).is_err());
            assert_eq!(continued.session.documents().len(), count);
            assert!(continued.session.active().unwrap().doc.shares_state(&changed));
        }
        assert_eq!(h.session.active().unwrap().source, identity);
        let output = directory.0.join("continued.pcraft");
        continued.save_owned_native_active(Some(&output), Some("pcraft"), &fixture::ctl()).unwrap();
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&output).unwrap());
        fixture::exact(&changed, &reopened.session.active().unwrap().doc, 0, 0);
        reopened.command_run("layer.translate", json!({"dx":-1,"dy":1})).unwrap();
        fixture::exact(&native, &reopened.session.active().unwrap().doc, 0, 0);
        assert!(!reopened.session.has_jobs());
        assert_eq!(std::fs::read(&case.path).unwrap(), case.bytes);
    }
}
