#![cfg(feature = "corpus")]
//! Disabled arithmetic is unused, including during an idempotent filter edit.
use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn disabled_overflow() -> Document {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("point-unpremultiply-evidence");
    let report_bytes = std::fs::read(root.join("observations.json")).unwrap();
    let hash = |bytes: &[u8]| Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hash(&report_bytes), "f40f522c4e8b7e7bab1e00b4e2d5fd2729c90062fbc36cb93172f362cd406f3d");
    let report: Value = serde_json::from_slice(&report_bytes).unwrap();
    let row = report["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["model"] == "RGB" && r["precision"] == "FLOAT_LINEAR" && r["id"].as_str().unwrap().ends_with(".disabled"))
        .unwrap();
    let bytes = std::fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    let mut doc = photocraft_io::import("unpremultiply.xcf", &bytes).unwrap().document;
    assert!(!doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    let bounds = doc.bounds();
    let mut bits = doc.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
    bits[..4].copy_from_slice(&f32::MAX.to_ne_bytes());
    bits[12..16].copy_from_slice(&0.5f32.to_ne_bytes());
    doc.layers[0].surface_mut().unwrap().try_write_interleaved(bounds, &bits).unwrap();
    assert!(photocraft_compose::gimp_render::PreparedRenderer::new(&doc).is_ok());
    doc
}

fn check_noop(command: &str) {
    let original = disabled_overflow();
    let bounds = original.bounds();
    let native = original.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap();
    let id = original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
    let mut session = Session::new();
    session.add_document(original.clone(), None).unwrap();
    // Leave a real redo snapshot to catch accidental accepted transactions.
    session
        .edit("Retain no-op redo witness", |doc, _| {
            doc.layers[0].name.push_str(" redo witness");
            Ok(())
        })
        .unwrap();
    let redo_document = (*session.active().unwrap().doc).clone();
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, original);
    let before = session.active().unwrap();
    let revision = before.revision;
    let saved_revision = before.saved_revision;
    let past = before.history.past_len();
    let entries: Vec<String> = before.history.entry_labels().map(str::to_owned).collect();
    let redo: Vec<String> = before.history.redo_labels().map(str::to_owned).collect();
    assert_eq!(redo.len(), 1);
    let unchanged = |session: &Session| {
        let now = session.active().unwrap();
        assert_eq!(*now.doc, original);
        assert_eq!(now.revision, revision);
        assert_eq!(now.saved_revision, saved_revision);
        assert_eq!(now.history.past_len(), past);
        assert_eq!(now.history.entry_labels().map(str::to_owned).collect::<Vec<_>>(), entries);
        assert_eq!(now.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        assert_eq!(now.doc.layers[0].surface().unwrap().try_read_interleaved(bounds).unwrap(), native);
    };
    let params = json!({"filterId":id,"enabled":false});
    let cancelled = Interrupt::cancel_only(&|| true);
    let result = if command == "xcf.filter.enable" {
        photocraft_engine::xcf_filter_cmds::enable_with_interrupt(&mut session, &params, &cancelled)
    } else {
        photocraft_engine::xcf_filter_cmds::set_with_interrupt(&mut session, &params, &cancelled)
    };
    assert!(matches!(result, Err(photocraft_engine::EngineError::Cancelled)));
    unchanged(&session);
    assert!(session.execute("xcf.filter.enable", json!({"filterId":id,"enabled":true})).is_err());
    unchanged(&session);
    let result = session.execute(command, params);
    unchanged(&session);
    // Redo must still install the exact pre-existing witness, not a new no-op.
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, redo_document);
    let result = result.unwrap_or_else(|error| panic!("valid disabled no-op {command} failed: {error}"));
    assert_eq!(result["updated"], false);
}

#[test]
#[ignore = "hash-bound independent disabled source; unused overflow arithmetic must not run"]
fn disabled_overflow_enable_noop_preserves_document_revision_history_redo_and_native() {
    check_noop("xcf.filter.enable");
}

#[test]
#[ignore = "hash-bound independent disabled source; unused overflow arithmetic must not run"]
fn disabled_overflow_set_noop_preserves_document_revision_history_redo_and_native() {
    check_noop("xcf.filter.set");
}
