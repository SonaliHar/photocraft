//! UNRUN native CURRENT metadata lifecycle, separate from authored source proof.
use super::*;
#[path = "../../../ui-egui/src/observed_box_metadata_fixture.rs"]
#[allow(dead_code)]
#[allow(clippy::duplicate_mod, reason = "This workflow deliberately instantiates its shared test helper under its own consumer parent")]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-box-metadata-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: observed Box all three native CURRENT roles, synchronous authorization/firstlabel opaque metadata gesture/noopredo, canonical/native/profile/source protection, paid query/title/write/reopen and malformed rollback; no GIMP hidden/opacity proof"]
fn observed_box_metadata_headless_continues_native_and_preserves_permissions() {
    for (sample, assigned) in [(photocraft_doc::SampleType::F32, false), (photocraft_doc::SampleType::F64, false), (photocraft_doc::SampleType::F32, true)] {
        let dir = Directory::new();
        let owner = fixture::owner(sample, assigned);
        // External harness encoded capture; the actual native reader supplies
        // provider/payload/header receipts on import, without cloned model authority.
        let bytes = fixture::encoded(&owner);
        let input = dir.0.join("input.pcraft");
        std::fs::write(&input, &bytes).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current_bounded(&input).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let layer = before.layers[0].id;
        let identity = h.session.active().unwrap().source.clone();
        let history = h.session.active().unwrap().history.clone();
        let p = json!({"layer":layer.0,"name":"Metadata current","visible":false,"opacity":-0.0,"coalesce":"Box\u{0}gesture"});
        assert!(h.command_start("layer.setProps", p.clone(), false).unwrap_err().to_string().contains("synchronous"));
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("Box metadata denied".into())));
        assert!(h.command_start("layer.setProps", p.clone(), true).unwrap_err().to_string().contains("Box metadata denied"));
        h.session.authorize = None;
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        let past = h.session.active().unwrap().history.past_len();
        assert!(h.command_start("layer.setProps", p, true).unwrap().is_null());
        let hidden = h.session.active().unwrap().doc.clone();
        assert!(!hidden.layers[0].visible);
        assert_eq!(hidden.layers[0].opacity.to_bits(), (-0f32).to_bits());
        fixture::preserved(&before, &hidden);
        h.command_run("layer.setProps", json!({"layer":"Metadata current","opacity":0.375,"coalesce":"Box\u{0}gesture"})).unwrap();
        let changed = h.session.active().unwrap().doc.clone();
        assert_eq!(changed.layers[0].opacity.to_bits(), 0.375f32.to_bits());
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(h.session.active().unwrap().history.undo_label(), Some("Layer Properties"));
        assert_eq!(h.session.active().unwrap().source, identity);
        h.command_run("edit.undo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        h.command_run("layer.setProps", json!({"layer":layer.0,"opacity":1.0,"coalesce":"Box\u{0}gesture"})).unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        h.command_run("edit.redo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&changed));
        for (id, p) in [
            ("layer.setProps", json!({"blend":"multiply"})),
            ("layer.setProps", json!({"layer":u64::MAX,"visible":true})),
            ("layer.setProps", json!({"name":"refuse","coalesce":"a".repeat(129)})),
            ("layer.translate", json!({"target":"mask","dx":1})),
            ("select.all", Value::Null),
            ("xcf.mask.enabled.set", json!({"maskId":"1","enabled":false})),
        ] {
            assert!(h.command_run(id, p).is_err(), "{id}");
        }
        assert!(h.session.active().unwrap().doc.shares_state(&changed));
        assert!(!h.session.has_jobs());
        for wait in [false, true] {
            let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, wait, &fixture::ctl()).unwrap();
            assert_eq!(reply.value()["filters"]["total"], 1);
            assert_eq!(reply.value()["items"]["total"], 2);
            drop(reply);
        }
        // This explicitly writable native input is not a read-only XCF source;
        // Save As below leaves it exact without inventing a same-path refusal.
        let output = dir.0.join("continued.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &fixture::ctl()).unwrap();
        assert_eq!(std::fs::read(&input).unwrap(), bytes);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&output).unwrap());
        fixture::preserved(&changed, &reopened.session.active().unwrap().doc);
        assert_eq!(reopened.session.active().unwrap().doc.layers[0].opacity.to_bits(), 0.375f32.to_bits());
        assert!(!reopened.session.active().unwrap().doc.layers[0].visible);
        reopened.command_run("layer.setProps", json!({"layer":layer.0,"visible":true})).unwrap();
        assert_eq!(reopened.session.active().unwrap().history.undo_label(), Some("Layer Visibility"));
    }
}
