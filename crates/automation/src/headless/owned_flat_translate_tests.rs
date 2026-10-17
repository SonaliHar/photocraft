//! UNRUN explicit trusted-local workflows; selected/effectreference/lock states
//! are synthetic CURRENT native controls derived from pinned original inputs.
use super::*;
#[path = "../../../ui-egui/src/canvas_flat_translate_fixture.rs"]
#[allow(dead_code)]
#[allow(clippy::duplicate_mod, reason = "This workflow deliberately instantiates its shared test helper under its own consumer parent")]
mod fixture;
use fixture::ctl;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-flat-move-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn seed_bytes(owner: &photocraft_doc::accounted_document::AccountedDocument, locked: bool) -> Vec<u8> {
    let archive = fixture::seed(owner, locked);
    photocraft_format::owned_document::save_current_document_archive(
        archive.document(),
        std::io::Cursor::new(Vec::new()),
        Default::default(),
        archive.quota(),
        &ctl(),
    )
    .unwrap()
    .into_inner()
}
#[test]
#[ignore = "UNRUN: six actual pinned original flat inputs, CURRENT native multi-selection/effectreference, registered Move explicit+selected/units/gesture/undo/noopredo/auth/wait, exact native/profile/source and native save-reopen; no original GIMP movement/numerical claim"]
fn flat_current_headless_move_keeps_wait_authority_and_native_custody() {
    for case in fixture::cases() {
        let dir = Directory::new();
        let original_path = dir.0.join("Original.xcf");
        std::fs::write(&original_path, &case.input).unwrap();
        let mut original = Headless::trusted_local();
        drop(original.open_owned_xcf_keep_current(&original_path, &ctl()).unwrap());
        let authored = original.session.active().unwrap().doc.clone();
        fixture::observed(&case, &authored);
        assert!(original.save_owned_native_active(Some(&original_path), Some("pcraft"), &ctl()).is_err());
        let native_path = dir.0.join("Current selection.pcraft");
        std::fs::write(&native_path, seed_bytes(authored.accounted().unwrap(), false)).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current(&native_path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let bottom = before.layers[0].id;
        let top = before.layers.last().unwrap().id;
        assert_eq!(h.session.active().unwrap().selection_ids(), vec![top, bottom]);
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.clone();
        assert!(matches!(h.command_start("layer.translate",json!({"dx":1}),false),Err(AutomationError::BadRequest(ref e))if e.contains("synchronous")));
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal, journal);
        assert!(!h.session.has_jobs());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("flat move denied".into())));
        assert!(h.command_run("layer.translate", json!({"dx":1})).unwrap_err().to_string().contains("flat move denied"));
        h.session.authorize = None;
        assert!(h.command_run("layer.translate", json!({"layer":bottom.0,"dx":1.49,"dy":-0.5,"units":"px","coalesce":"flat\0move"})).unwrap().is_null());
        let first = h.session.active().unwrap().doc.clone();
        fixture::exact(&before, &first, &[(bottom, 1, -1)]);
        h.command_run("layer.setProps", json!({"layer":bottom.0,"name":"Moved CURRENT bottom","coalesce":"flat\0move"})).unwrap();
        h.command_run("layer.translate", json!({"layer":null,"dx":-2,"dy":1,"coalesce":"flat\0move"})).unwrap();
        let moved = h.session.active().unwrap().doc.clone();
        fixture::exact(&before, &moved, &[(bottom, -1, 0), (top, -2, 1)]);
        assert_eq!(h.session.active().unwrap().history.past_len(), history.past_len() + 1);
        assert_eq!(h.session.active().unwrap().history.entry_labels().last(), Some("Move"));
        h.command_run("edit.undo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let redo = h.session.active().unwrap().history.clone();
        h.command_run("layer.translate", json!({"layer":"invalid ignored on rounded zero","dx":0.49,"dy":-0.49})).unwrap();
        assert!(h.session.active().unwrap().history.shares_accounted_history(&redo));
        h.command_run("edit.redo", Value::Null).unwrap();
        let dpi = f64::from(moved.resolution_dpi);
        h.command_run("layer.translate", json!({"layer":"Moved CURRENT bottom","dx":72.0/dpi,"dy":0,"units":"pt"})).unwrap();
        fixture::exact(&moved, &h.session.active().unwrap().doc, &[(bottom, 1, 0)]);
        h.command_run("edit.undo", Value::Null).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&moved));
        let history = h.session.active().unwrap().history.clone();
        for params in [json!({"dx":1,"target":"mask"}), json!({"dx":1,"target":{"channel":1}}), json!({"dx":2147483647})] {
            assert!(h.command_run("layer.translate", params).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&moved));
            assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        }
        let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &json!({"offset":1,"limit":2}), false, &ctl()).unwrap();
        assert_eq!(reply.value()["items"]["total"], moved.layers.len());
        assert_eq!(reply.value()["filters"]["total"], 0);
        drop(reply);
        let source = h.session.active().unwrap().source.clone();
        let out = dir.0.join("Moved CURRENT.pcraft");
        h.save_owned_native_active(Some(&out), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, source);
        let saved = h.session.active().unwrap().doc.clone();
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&out, &ctl()).unwrap());
        let current = reopened.session.active().unwrap().doc.clone();
        fixture::exact(&saved, &current, &[]);
        assert_eq!(saved.name, current.name);
        assert!(!reopened.session.active().unwrap().source_read_only);
        assert!(!reopened.has_owned_filter_inspection());
        reopened.command_run("layer.translate", json!({"layer":bottom.0,"dx":-1})).unwrap();
        fixture::exact(&current, &reopened.session.active().unwrap().doc, &[(bottom, -1, 0)]);
        let locked_path = dir.0.join("Locked CURRENT.pcraft");
        std::fs::write(&locked_path, seed_bytes(authored.accounted().unwrap(), true)).unwrap();
        let mut locked = Headless::trusted_local();
        drop(locked.open_owned_native_keep_current(&locked_path, &ctl()).unwrap());
        let old = locked.session.active().unwrap().doc.clone();
        assert!(locked.command_run("layer.translate", json!({"dx":1})).is_err());
        assert!(locked.session.active().unwrap().doc.shares_state(&old));
        assert_eq!(std::fs::read(&original_path).unwrap(), case.input);
        assert_eq!(std::fs::read(&case.path).unwrap(), case.input);
    }
}
