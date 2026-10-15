//! UNRUN explicit authored sources with synthetic CURRENT metadata transactions.
//! Harness files/model copies do not assert producer-edited source evidence.
use super::*;
#[path = "../../../ui-egui/src/canvas_separable_metadata_fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "../../../ui-egui/src/canvas_separable_blend_fixture.rs"]
#[allow(dead_code)]
mod original;
use fixture::payload;
use original::{cases, ctl};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-separable-metadata-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN:50 actual flat source domains Headless compoundmetadata/bothIDs/coalesce/firstlabel/noopredo/auth/sync/paidquery/native/source/raw preservation; CURRENT lock control"]
fn separable_headless_metadata_preserves_source_and_refuses_start() {
    let (root, cases) = cases();
    for case in cases {
        let dir = Directory::new();
        let bytes = std::fs::read(root.join(&case.file)).unwrap();
        let path = dir.0.join("Original.xcf");
        std::fs::write(&path, &bytes).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let source = h.session.active().unwrap().source.clone();
        let base = before.layers[0].id;
        let top = before.layers[1].id;
        let past = h.session.active().unwrap().history.past_len();
        assert!(
            matches!(h.command_start("layer.setProps",json!({"layer":top.0,"visible":false}),false),Err(AutomationError::BadRequest(ref why))if why.contains("synchronous"))
        );
        assert!(!h.session.has_jobs());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("flat metadata denied".into())));
        assert!(h.command_run("layer.setProps", json!({"layer":top.0,"visible":false})).unwrap_err().to_string().contains("flat metadata denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        h.command_start("layer.setProps", json!({"layer":top.0,"visible":false,"coalesce":""}), true).unwrap();
        h.command_run("layer.setProps", json!({"layer":base.0,"name":"Current base","visible":true,"opacity":0.375,"coalesce":""})).unwrap();
        h.command_run("layer.setProps", json!({"layer":top.0,"name":"Current top","visible":true,"opacity":-0.0,"coalesce":""})).unwrap();
        let changed = h.session.active().unwrap().doc.clone();
        assert_eq!(changed.layer(top).unwrap().opacity.to_bits(), (-0.0f32).to_bits());
        payload(&before, &changed);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Layer Visibility"));
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let redo = h.session.active().unwrap().history.clone();
        let old = before.layer(top).unwrap();
        h.command_run("layer.setProps", json!({"layer":top.0,"name":old.name,"visible":old.visible,"opacity":f64::from(old.opacity),"coalesce":"noop"}))
            .unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&redo));
        h.command_run("edit.redo", json!({})).unwrap();
        let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(reply.value()["items"]["total"], 2);
        assert_eq!(reply.value()["filters"]["total"], 0);
        drop(reply);
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        let output = dir.0.join("Current flat.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        payload(&before, &saved);
        assert_eq!(h.session.active().unwrap().source, source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = reopened.session.active().unwrap().doc.clone();
        assert_eq!(current.name, saved.name);
        assert_eq!(current.xcf_state, saved.xcf_state);
        payload(&saved, &current);
        assert!(!reopened.session.active().unwrap().source_read_only);
        for id in [base, top] {
            reopened.command_run("layer.setProps", json!({"layer":id.0,"name":"Duplicate"})).unwrap();
        }
        reopened.command_run("layer.setProps", json!({"layer":"Duplicate","opacity":0.25})).unwrap();
        assert_eq!(reopened.session.active().unwrap().doc.layer(top).unwrap().opacity.to_bits(), 0.25f32.to_bits());
        assert_eq!(reopened.session.active().unwrap().doc.layer(base).unwrap().opacity.to_bits(), 0.375f32.to_bits());
        for (id, p) in [("layer.translate", json!({"dx":1})), ("select.all", Value::Null), ("xcf.filter.set", json!({"filterId":"1","enabled":false}))] {
            assert!(reopened.command_run(id, p).is_err());
        }
        if case.file == "mode-screen.xcf" {
            // External harness metadata capture, explicitly synthetic CURRENT.
            // The original paid owner remains live through real native capture
            // and strict typed reopen. No locked GIMP-source claim follows.
            let mut locked = current.accounted().unwrap().document().clone();
            locked.layer_mut(top).unwrap().locks.all = true;
            locked.layer_mut(top).unwrap().locks.pixels = true;
            let item = locked.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(top)).unwrap();
            item.locks.content = true;
            item.locks.visibility = true;
            let captured = photocraft_format::owned_document::save_current_document_archive(
                &locked,
                std::io::Cursor::new(Vec::new()),
                Default::default(),
                current.accounted().unwrap().quota(),
                &ctl(),
            )
            .unwrap()
            .into_inner();
            let locked_path = dir.0.join("Synthetic current locks.pcraft");
            std::fs::write(&locked_path, captured).unwrap();
            let mut control = Headless::trusted_local();
            drop(control.open_owned_native_keep_current(&locked_path, &ctl()).unwrap());
            let prior = control.session.active().unwrap().doc.clone();
            let history = control.session.active().unwrap().history.clone();
            let journal = control.session.journal.clone();
            assert!(control.command_run("layer.setProps", json!({"layer":top.0,"visible":false})).is_err());
            assert!(control.session.active().unwrap().doc.shares_state(&prior));
            assert!(control.session.active().unwrap().history.shares_accounted_history(&history));
            assert_eq!(control.session.journal, journal);
            control.command_run("layer.setProps", json!({"layer":top.0,"visible":true})).unwrap();
            assert!(control.session.active().unwrap().doc.shares_state(&prior));
            control.command_run("layer.setProps", json!({"layer":top.0,"name":"Allowed locked metadata","opacity":0.5})).unwrap();
            assert_eq!(control.session.active().unwrap().doc.layer(top).unwrap().opacity.to_bits(), 0.5f32.to_bits());
            assert_eq!(control.session.active().unwrap().doc.layer(top).unwrap().name, "Allowed locked metadata");
        }
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(root.join(&case.file)).unwrap(), bytes);
    }
}
