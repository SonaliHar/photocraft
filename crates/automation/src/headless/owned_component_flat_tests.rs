//! UNRUN source/edit original component/luma packets and explicit CURRENT continuations.
//! Native/source zero tolerance is distinct from derived display test bounds.
use super::*;
#[path = "../../../ui-egui/src/canvas_component_flat_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::ctl;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-component-luma11-host-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN:11 component/luma modes and22 actual source/edit XCFs through trusted explicit source/native routes, exact policy/native/profile/source protection, paid queries/auth/wait, metadata+Move/noopredo/modern20 transition and native continuation; no alias/producer CURRENT-edit claim"]
fn component_luma11_headless_preserves_raw_source_and_allows_explicit_current_continuation() {
    let (root, cases) = fixture::cases();
    for case in cases {
        let dir = Directory::new();
        let bytes = fixture::input(&root, &case);
        let path = dir.0.join("Original component.xcf");
        std::fs::write(&path, &bytes).unwrap();
        let mut source = Headless::trusted_local();
        drop(source.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let original = source.session.active().unwrap().doc.clone();
        fixture::observed(&root, &case, &original);
        assert!(source.session.active().unwrap().source_read_only);
        assert!(!source.has_owned_filter_inspection());
        assert!(source.has_owned_inspection_command("xcf.inspect"));
        assert!(source.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        for wait in [false, true] {
            let reply = source.inspect_owned_command_with_read_control("xcf.inspect", &json!({"offset":1,"limit":1}), wait, &ctl()).unwrap();
            assert_eq!(reply.value()["items"]["total"], 2);
            assert_eq!(reply.value()["items"]["entries"].as_array().unwrap().len(), 1);
            assert_eq!(reply.value()["filters"]["total"], 0);
            drop(reply);
        }
        source.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("component query denied".into())));
        match source.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()) {
            Err(e) => assert!(e.to_string().contains("component query denied")),
            Ok(_) => panic!("supported query must honor authorizer"),
        };
        source.session.authorize = None;
        let saved_source = source.session.active().unwrap().source.clone();
        let first_native = dir.0.join("Component current.pcraft");
        source.save_owned_native_active(Some(&first_native), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(source.session.active().unwrap().source, saved_source);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current(&first_native, &ctl()).unwrap());
        let initial = h.session.active().unwrap().doc.clone();
        fixture::observed(&root, &case, &initial);
        fixture::raw(&original, &initial);
        assert_eq!(original.xcf_state, initial.xcf_state);
        assert!(!h.session.active().unwrap().source_read_only);
        // Synthetic CURRENT native zero-Y Luminance refuses typed publication.
        // Equal-Y/differentRGB and later HDR restrictions remain lower-kernel gates.
        if case.mode == 56 {
            let bad = fixture::zero_y_archive(initial.accounted().unwrap());
            let bytes = photocraft_format::owned_document::save_current_document_archive(
                bad.document(),
                std::io::Cursor::new(Vec::new()),
                Default::default(),
                bad.quota(),
                &ctl(),
            )
            .unwrap()
            .into_inner();
            let badpath = dir.0.join("Zero Y CURRENT.pcraft");
            std::fs::write(&badpath, bytes).unwrap();
            let mut refused = Headless::trusted_local();
            assert!(refused.open_owned_native_keep_current(&badpath, &ctl()).is_err());
            assert!(refused.session.documents().is_empty());
        }
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
        h.command_run("layer.translate", json!({"layer":initial.layers[1].name,"dx":-1,"dy":2,"coalesce":"component"})).unwrap();
        let moved = h.session.active().unwrap().doc.clone();
        fixture::moved(&initial, &moved, top, -1, 2);
        h.command_run("layer.setProps", json!({"layer":top.0,"name":"Component CURRENT foreground","opacity":0.375,"coalesce":"component"})).unwrap();
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
        let rev = h.session.active().unwrap().revision;
        let mut unsupported = fixture::transition(&metadata, rev);
        unsupported["sourceMode"] = json!(case.mode);
        unsupported["legacy"] = json!(case.mode <= 14);
        assert!(h.command_run("xcf.composition.set", unsupported).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&metadata));
        let reply = h.command_run("xcf.composition.set", fixture::transition(&metadata, rev)).unwrap();
        assert_eq!(reply["updated"], true);
        let changed = h.session.active().unwrap().doc.clone();
        fixture::transitioned(&changed);
        fixture::raw(&metadata, &changed);
        h.command_run("edit.undo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&metadata));
        h.command_run("edit.redo", Value::Null).unwrap();
        let before_save_source = h.session.active().unwrap().source.clone();
        let output = dir.0.join("Component to modern.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(h.session.active().unwrap().source, before_save_source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = reopened.session.active().unwrap().doc.clone();
        fixture::raw(&saved, &current);
        assert_eq!(saved.xcf_state, current.xcf_state);
        assert_eq!(saved.name, current.name);
        fixture::transitioned(&current);
        assert!(!reopened.session.active().unwrap().source_read_only);
        let reply = reopened.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, false, &ctl()).unwrap();
        assert_eq!(reply.value()["items"]["total"], 2);
        drop(reply);
        assert!(reopened.command_run("select.all", Value::Null).is_err());
        assert!(reopened.inspect_owned_filter_bounded(&json!({"filterId":"1"}), true).is_err());
        reopened.command_run("layer.translate", json!({"layer":top.0,"dx":1})).unwrap();
        fixture::moved(&current, &reopened.session.active().unwrap().doc, top, 1, 0);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(root.join(&case.file)).unwrap(), bytes);
    }
}
