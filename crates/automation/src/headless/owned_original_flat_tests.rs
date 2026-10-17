//! UNRUN independently saved original lifecycle. Profile-byte/RGB/signed-zero
//! HOLDs remain; no GIMP mathematical acceptance is asserted by these workflows.
use super::*;
#[path = "../../../ui-egui/src/canvas_original_flat_fixture.rs"]
#[allow(dead_code)]
#[allow(clippy::duplicate_mod, reason = "This workflow deliberately instantiates its shared test helper under its own consumer parent")]
mod fixture;
use fixture::{ctl, page, payload};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-original-flat-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: six hash-pinned GIMP originals through explicit path, retained paid pages/auth/wait, metadata/composition undo/noopredo, source protection and real native writer/reopen; not numerical GIMP validation"]
fn original_flat_headless_keeps_exact_source_and_native_access_after_current_edits() {
    for case in fixture::cases() {
        let dir = Directory::new();
        let path = dir.0.join("Original.xcf");
        std::fs::write(&path, &case.input).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let source = h.session.active().unwrap().source.clone();
        fixture::observed(&case, &before);
        fixture::same_quota(before.accounted().unwrap());
        assert!(h.session.active().unwrap().source_read_only);
        assert!(!h.has_owned_filter_inspection());
        assert!(h.has_owned_inspection_command("xcf.inspect"));
        for (params, offset, limit) in
            [(Value::Null, 0, 64), (json!({"limit":1}), 0, 1), (json!({"offset":1,"limit":2}), 1, 2), (json!({"offset":100000,"limit":128}), 100000, 128)]
        {
            for wait in [false, true] {
                let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &params, wait, &ctl()).unwrap();
                page(reply.value(), &before, offset, limit);
                drop(reply);
            }
        }
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("original flat denied".into())));
        match h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()) {
            Err(e) => assert!(e.to_string().contains("original flat denied")),
            Ok(_) => panic!("supported query must honor authorizer"),
        };
        h.session.authorize = None;
        let target = before.layers.last().unwrap().id;
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        assert!(matches!(h.command_start("layer.setProps", json!({"layer":target.0,"name":"current"}), false), Err(AutomationError::BadRequest(_))));
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert!(!h.session.has_jobs());
        h.command_run("layer.setProps", json!({"layer":target.0,"name":"Original continued CURRENT","opacity":0.375})).unwrap();
        let metadata = h.session.active().unwrap().doc.clone();
        payload(&before, &metadata);
        let rev = h.session.active().unwrap().revision;
        let params = json!({"layerId":target.0.to_string(),"sourceMode":28,"legacy":false,"blendSpace":"linear-rgb","compositeSpace":"linear-rgb","operator":"union","documentId":metadata.id.0.to_string(),"expectedRevision":rev.to_string()});
        assert!(matches!(h.command_start("xcf.composition.set", params.clone(), false), Err(AutomationError::BadRequest(_))));
        let reply = h.command_run("xcf.composition.set", params).unwrap();
        assert_eq!(reply["updated"], true);
        let changed = h.session.active().unwrap().doc.clone();
        payload(&before, &changed);
        h.command_run("edit.undo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&metadata));
        h.command_run("edit.undo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        h.command_run("layer.setProps", json!({"layer":target.0,"name":before.layers.last().unwrap().name})).unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        h.command_run("edit.redo", Value::Null).unwrap();
        h.command_run("edit.redo", Value::Null).unwrap();
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        let out = dir.0.join("Original continued.pcraft");
        h.save_owned_native_active(Some(&out), Some("pcraft"), &ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(h.session.active().unwrap().source, source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&out, &ctl()).unwrap());
        let native = reopened.session.active().unwrap().doc.clone();
        payload(&saved, &native);
        assert_eq!(saved.xcf_state, native.xcf_state);
        assert_eq!(saved.name, native.name);
        assert!(!reopened.session.active().unwrap().source_read_only);
        fixture::same_quota(native.accounted().unwrap());
        let reply = reopened.inspect_owned_command_with_read_control("xcf.inspect", &json!({"offset":1,"limit":1}), false, &ctl()).unwrap();
        page(reply.value(), &native, 1, 1);
        drop(reply);
        assert!(reopened.command_run("layer.translate", json!({"dx":1})).is_err());
        assert!(reopened.command_run("select.all", Value::Null).is_err());
        let malformed = dir.0.join("Malformed raw policy.xcf");
        std::fs::write(&malformed, fixture::malformed(&case)).unwrap();
        let mut refused = Headless::trusted_local();
        assert!(refused.open_owned_xcf_keep_current(&malformed, &ctl()).is_err());
        assert!(refused.session.documents().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), case.input);
        assert_eq!(std::fs::read(&case.path).unwrap(), case.input);
    }
}
