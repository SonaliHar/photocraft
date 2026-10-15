//! UNRUN trusted source/native model-specific lifecycle; Gray producer ICC-byte
//! HOLD is preserved. No generated-builtin/absent-producer equality is claimed.
use super::*;
#[path = "../../../ui-egui/src/canvas_alpha_flat_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::ctl;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-alpha-model-host-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN:16 RGB+Gray alpha57..60 source/edit files, exact raw/native/profile and source protection, paid queries/auth/wait, metadata+Move+title/native continuation; RGBmodern20 only, Gray unsupported transition atomic, missing8ICC HOLD not equality"]
fn alpha_rgb_gray_headless_keeps_exact_storage_and_closed_gray_policy() {
    for case in fixture::cases() {
        let dir = Directory::new();
        let bytes = fixture::input(&case);
        let path = dir.0.join("Original alpha.xcf");
        std::fs::write(&path, &bytes).unwrap();
        let mut source = Headless::trusted_local();
        drop(source.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let original = source.session.active().unwrap().doc.clone();
        fixture::observed(&case, &original);
        fixture::same_quota(original.accounted().unwrap());
        assert!(source.session.active().unwrap().source_read_only);
        assert!(!source.has_owned_filter_inspection());
        assert!(source.has_owned_inspection_command("xcf.inspect"));
        assert!(source.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        for wait in [false, true] {
            let reply = source.inspect_owned_command_with_read_control("xcf.inspect", &json!({"offset":1,"limit":1}), wait, &ctl()).unwrap();
            fixture::page(reply.value(), &original, 1, 1);
            drop(reply);
        }
        source.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("alpha query denied".into())));
        match source.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()) {
            Err(e) => assert!(e.to_string().contains("alpha query denied")),
            Ok(_) => panic!("supported query honors authorizer"),
        };
        source.session.authorize = None;
        let protected = source.session.active().unwrap().source.clone();
        let first = dir.0.join("Original alpha.pcraft");
        source.save_owned_native_active(Some(&first), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(source.session.active().unwrap().source, protected);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current(&first, &ctl()).unwrap());
        let initial = h.session.active().unwrap().doc.clone();
        fixture::observed(&case, &initial);
        fixture::raw(&original, &initial);
        assert_eq!(original.xcf_state, initial.xcf_state);
        assert!(!h.session.active().unwrap().source_read_only);
        let top = initial.layers[1].id;
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.clone();
        for (id, params) in [
            ("layer.translate", json!({"dx":1})),
            ("layer.setProps", json!({"layer":top.0,"name":"current"})),
            ("xcf.composition.set", fixture::transition(&initial, revision)),
        ] {
            assert!(matches!(h.command_start(id,params,false),Err(AutomationError::BadRequest(ref e))if e.contains("synchronous")));
            assert!(h.session.active().unwrap().doc.shares_state(&initial));
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(h.session.active().unwrap().revision, revision);
            assert_eq!(h.session.journal, journal);
            assert!(!h.session.has_jobs());
        }
        h.command_run("layer.translate", json!({"layer":initial.layers[1].name,"dx":-1,"dy":2,"coalesce":"alpha"})).unwrap();
        let moved = h.session.active().unwrap().doc.clone();
        fixture::moved(&initial, &moved, top, -1, 2);
        h.command_run("layer.setProps", json!({"layer":top.0,"name":"Alpha CURRENT foreground","opacity":0.375,"coalesce":"alpha"})).unwrap();
        let metadata = h.session.active().unwrap().doc.clone();
        fixture::raw(&moved, &metadata);
        assert_eq!(h.session.active().unwrap().history.past_len(), history.past_len() + 1);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Move"));
        h.command_run("edit.undo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&initial));
        let redo = h.session.active().unwrap().history.clone();
        h.command_run("layer.translate", json!({"layer":"ignored rounded-zero","dx":0.49})).unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&redo));
        h.command_run("edit.redo", Value::Null).unwrap();
        let revision = h.session.active().unwrap().revision;
        let transition = fixture::transition(&metadata, revision);
        let changed = if case.gray {
            let history = h.session.active().unwrap().history.clone();
            let journal = h.session.journal.clone();
            assert!(h.command_run("xcf.composition.set", transition).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&metadata));
            assert_eq!(h.session.active().unwrap().revision, revision);
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(h.session.journal, journal);
            metadata.clone()
        } else {
            let reply = h.command_run("xcf.composition.set", transition).unwrap();
            assert_eq!(reply["updated"], true);
            let changed = h.session.active().unwrap().doc.clone();
            fixture::raw(&metadata, &changed);
            h.command_run("edit.undo", Value::Null).unwrap();
            assert!(h.session.active().unwrap().doc.shares_state(&metadata));
            h.command_run("edit.redo", Value::Null).unwrap();
            changed
        };
        let before_save = h.session.active().unwrap().source.clone();
        let out = dir.0.join("Alpha current model.pcraft");
        h.save_owned_native_active(Some(&out), Some("pcraft"), &ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(h.session.active().unwrap().source, before_save);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&out, &ctl()).unwrap());
        let current = reopened.session.active().unwrap().doc.clone();
        fixture::raw(&changed, &current);
        fixture::same_quota(current.accounted().unwrap());
        assert_eq!(saved.xcf_state, current.xcf_state);
        assert_eq!(saved.name, current.name);
        assert!(!reopened.session.active().unwrap().source_read_only);
        let reply = reopened.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, false, &ctl()).unwrap();
        fixture::page(reply.value(), &current, 0, 64);
        drop(reply);
        reopened.command_run("layer.translate", json!({"layer":top.0,"dx":1})).unwrap();
        fixture::moved(&current, &reopened.session.active().unwrap().doc, top, 1, 0);
        assert!(reopened.command_run("select.all", Value::Null).is_err());
        assert!(reopened.inspect_owned_filter_bounded(&json!({"filterId":"1"}), true).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(case.root.join(&case.file)).unwrap(), bytes);
    }
}
