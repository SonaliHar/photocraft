//! UNRUN real source/native Headless continuation. Files are external harness
//! captures; only actual importer/reader/provider owners establish paid custody.
use super::*;
#[path = "../../../ui-egui/src/canvas_separable_blend_fixture.rs"]
mod fixture;
use fixture::{cases, ctl, observed, retained};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-separable-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: all50 typed separable path opens, paid project authorization, mutation closure/source protection and writable native reopen; Lab remains refused"]
fn separable_headless_explicit_paths_have_no_filter_or_edit_grant() {
    let (root, cases) = cases();
    for case in cases {
        let dir = Directory::new();
        let input = std::fs::read(root.join(&case.file)).unwrap();
        let path = dir.0.join("Authored.xcf");
        std::fs::write(&path, &input).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let source = h.session.active().unwrap().source.clone();
        assert!(before.accounted().unwrap().is_readonly_flat_composite_storage());
        assert!(h.session.active().unwrap().source_read_only);
        observed(&root, &before, &case);
        assert!(!h.has_owned_filter_inspection());
        assert!(h.has_owned_inspection_command("xcf.inspect"));
        assert!(!h.has_owned_inspection_command("xcf.filter.inspect"));
        for wait in [true, false] {
            let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, wait, &ctl()).unwrap();
            assert_eq!(reply.value()["items"]["total"], 2);
            assert_eq!(reply.value()["filters"]["total"], 0);
            assert_eq!(reply.value()["filterStacks"]["total"], 0);
            drop(reply);
        }
        assert!(!h.session.has_jobs());
        assert!(h.inspect_owned_filter_bounded(&json!({"filterId":"1"}), true).is_err());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("separable query denied".into())));
        match h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()) {
            Err(e) => assert!(e.to_string().contains("separable query denied")),
            Ok(_) => panic!("supported project query honors authorizer"),
        };
        h.session.authorize = None;
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.clone();
        for (id, p) in [
            ("layer.setProps", json!({"blend":"multiply"})),
            ("layer.translate", json!({"dx":1})),
            ("select.all", Value::Null),
            ("xcf.filter.set", json!({"filterId":"1","enabled":false})),
        ] {
            assert!(h.command_run(id, p).is_err());
        }
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal, journal);
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        let output = dir.0.join("Saved separable.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        retained(&before, &saved);
        assert_eq!(h.session.active().unwrap().source, source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = &reopened.session.active().unwrap().doc;
        assert_eq!(current.name, saved.name);
        retained(&before, current);
        observed(&root, current, &case);
        assert!(!reopened.session.active().unwrap().source_read_only);
        assert!(current.accounted().unwrap().is_readonly_flat_composite_storage());
        let query = reopened.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 2);
        drop(query);
        assert_eq!(std::fs::read(&path).unwrap(), input);
        assert_eq!(std::fs::read(root.join(&case.file)).unwrap(), input);
    }
    for file in ["composite-space-lab.xcf", "composite-space-lab.edit.xcf"] {
        let dir = Directory::new();
        let bytes = std::fs::read(root.join(file)).unwrap();
        let path = dir.0.join("Excluded Lab.xcf");
        std::fs::write(&path, &bytes).unwrap();
        let mut h = Headless::trusted_local();
        assert!(h.open_owned_xcf_keep_current(&path, &ctl()).is_err());
        assert!(h.session.documents().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
