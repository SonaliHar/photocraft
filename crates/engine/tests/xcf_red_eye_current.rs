#![cfg(feature = "corpus")]
//! Registered Red Eye edits against independently authored, immutable originals.
use photocraft_doc::{
    Document,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterValue},
};
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn packet() -> (PathBuf, Vec<Value>) {
    let base = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"));
    let root = base.join("red-eye-removal-precision-evidence");
    let bytes = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "821fad4b046cae0174291934fa61b01f5fc966cd4d612e4ec77eeb0cb6fb6e5f");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    let bytes = fs::read(root.join("observations.json")).unwrap();
    let entry = manifest["artifacts"].as_array().unwrap().iter().find(|e| e["path"] == "observations.json").unwrap();
    assert_eq!(hash(&bytes), entry["sha256"]);
    let observations: Value = serde_json::from_slice(&bytes).unwrap();
    let rows = observations["records"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 108);
    (root, rows)
}
fn import(root: &Path, row: &Value) -> Document {
    let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    photocraft_io::import("red-eye.xcf", &bytes).unwrap().document
}
fn source_unchanged(current: &Document, source: &Document) {
    assert_eq!(current.layers, source.layers);
    assert_eq!(current.source_color_context, source.source_color_context);
    let a = current.xcf_state.as_ref().unwrap();
    let b = source.xcf_state.as_ref().unwrap();
    assert_eq!(a.opaque, b.opaque);
    assert_eq!(a.provenance, b.provenance);
    assert_eq!(a.filter_masks, b.filter_masks);
    assert_eq!(a.resources, b.resources);
}
fn native_and_history(session: &mut Session, source: &Document) {
    let current = (*session.active().unwrap().doc).clone();
    let bytes = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(reopened, current);
    let node = &reopened.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    assert!(photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(node));
    let mut continued = Session::new();
    continued.add_document(reopened.clone(), None).unwrap();
    continued.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":node.arguments})).unwrap();
    assert_eq!(continued.active().unwrap().history.past_len(), 0);
    let mut arguments = node.arguments.clone();
    let next = if arguments[0].value == FilterValue::Number(ExactNumber::Float32Bits(0.625f32.to_bits())) { 0.25f32 } else { 0.625f32 };
    arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(next.to_bits()));
    continued.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":arguments})).unwrap();
    let second = (*continued.active().unwrap().doc).clone();
    source_unchanged(&second, &reopened);
    assert_eq!(continued.active().unwrap().history.past_len(), 1);
    let saved = photocraft_format::save_to_bytes(&second, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), second);
    continued.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*continued.active().unwrap().doc, reopened);
    assert_eq!(session.active().unwrap().history.past_len(), 1);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, *source);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*session.active().unwrap().doc, current);
}

#[test]
#[ignore = "local frozen108 Red Eye originals and independently authored edits"]
fn registered_threshold_disable_activation_native_and_history() {
    let (root, rows) = packet();
    let mut count = 0;
    for row in rows.iter().filter(|r| r["action"] == "source") {
        let source = import(&root, row);
        let family = row["id"].as_str().unwrap().strip_suffix(".source").unwrap();
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(node));
        assert_eq!(node.operation.schema_epoch, "xcf-22-stock-red-eye-removal-precision-v1");
        assert_eq!(node.arguments[0].value, FilterValue::Number(ExactNumber::Float32Bits(0.25f32.to_bits())));
        let fid = node.id.0.to_string();
        for stage in ["edit", "disabled"] {
            let target = import(&root, rows.iter().find(|r| r["id"] == format!("{family}.{stage}")).unwrap());
            let desired = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":node.arguments})).unwrap();
            assert_eq!(session.active().unwrap().history.past_len(), 0);
            assert_eq!(*session.active().unwrap().doc, source);
            if stage == "edit" {
                session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":desired.arguments})).unwrap();
            } else {
                session.execute("xcf.filter.enable", json!({"filterId":fid,"enabled":false})).unwrap();
            }
            let current = &session.active().unwrap().doc;
            source_unchanged(current, &source);
            let actual = &current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            assert_eq!(actual.arguments, desired.arguments);
            assert_eq!(actual.enabled, desired.enabled);
            assert_eq!(
                photocraft_compose::gimp_render::try_render(current, current.bounds(), &mut |_| Ok(())).unwrap().px,
                photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px,
                "{family}/{stage}"
            );
            native_and_history(&mut session, &source);
            count += 1;
        }
        let disabled = import(&root, rows.iter().find(|r| r["id"] == format!("{family}.disabled")).unwrap());
        let mut session = Session::new();
        session.add_document(disabled.clone(), None).unwrap();
        let disabled_node = &disabled.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        session.execute("xcf.filter.enable", json!({"filterId":disabled_node.id.0.to_string(),"enabled":true})).unwrap();
        source_unchanged(&session.active().unwrap().doc, &disabled);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(&session.active().unwrap().doc, source.bounds(), &mut |_| Ok(())).unwrap().px,
            photocraft_compose::gimp_render::try_render(&source, source.bounds(), &mut |_| Ok(())).unwrap().px
        );
        native_and_history(&mut session, &disabled);
        count += 1;
    }
    assert_eq!(count, 108);
}

#[test]
#[ignore = "local Red Eye typed thresholds and atomic refusal controls"]
fn wrong_type_invalid_threshold_locks_and_cancel_are_atomic() {
    let (root, rows) = packet();
    let mut refusals = 0;
    for row in rows.iter().filter(|r| r["action"] == "source") {
        let source = import(&root, row);
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let fid = node.id.0.to_string();
        let values = [
            FilterValue::Bool(true),
            FilterValue::Number(ExactNumber::Float64Bits(0.625f64.to_bits())),
            FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
            FilterValue::Number(ExactNumber::Float32Bits(f32::INFINITY.to_bits())),
            FilterValue::Number(ExactNumber::Float32Bits((-0.001f32).to_bits())),
            FilterValue::Number(ExactNumber::Float32Bits(0.8f32.to_bits())),
        ];
        for value in values {
            let mut args = node.arguments.clone();
            args[0].value = value;
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            assert!(session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":args})).is_err());
            assert_eq!(*session.active().unwrap().doc, source);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
            refusals += 1;
        }
        for params in [
            json!({"filterId":fid,"arguments":[]}),
            json!({"filterId":fid,"arguments":node.arguments,"unknown":true}),
            json!({"filterId":0,"arguments":node.arguments}),
        ] {
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            assert!(session.execute("xcf.filter.set", params).is_err());
            assert_eq!(*session.active().unwrap().doc, source);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
            refusals += 1;
        }
        let mut locked = source.clone();
        let owner = locked.layers[0].id;
        locked.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == ItemRef::Layer(owner)).unwrap().locks.content = true;
        let mut session = Session::new();
        session.add_document(locked.clone(), None).unwrap();
        let mut args = node.arguments.clone();
        args[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.625f32.to_bits()));
        assert!(session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":args})).is_err());
        assert_eq!(*session.active().unwrap().doc, locked);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
        refusals += 1;
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        assert!(matches!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut session,
                &json!({"filterId":fid,"arguments":args}),
                &photocraft_raster::Interrupt::cancel_only(&|| true)
            ),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
        refusals += 1;
    }
    assert_eq!(refusals, 396);
}

#[test]
#[ignore = "six frozen Half Red Eye originals"]
fn half_removal_reverts_current_precision_and_preserves_source_native_history() {
    let (root, rows) = packet();
    let mut count = 0;
    for row in rows.iter().filter(|r| r["action"] == "source" && r["precision"].as_str().unwrap().starts_with("HALF_")) {
        let source = import(&root, row);
        let fid = source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute("xcf.filter.remove", json!({"filterId":fid})).unwrap();
        let current = (*session.active().unwrap().doc).clone();
        let state = current.xcf_state.as_ref().unwrap();
        assert_eq!(state.current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
        assert!(state.filter_masks.is_empty());
        assert!(state.filter_stacks.iter().all(|s| s.nodes.is_empty()));
        assert_eq!(current.layers, source.layers);
        assert_eq!(current.source_color_context, source.source_color_context);
        let mut archived = source.xcf_state.as_ref().unwrap().opaque.clone();
        for record in &mut archived {
            if let Some(ItemRef::Mask(mask)) = record.owner {
                assert!(source.xcf_state.as_ref().unwrap().filter_masks.iter().any(|m| m.id == mask));
                record.owner = None;
                record.identity = format!("deleted-filter-mask/{}/{}", mask.0, record.identity);
            }
        }
        assert_eq!(state.opaque, archived);
        assert_eq!(state.provenance, source.xcf_state.as_ref().unwrap().provenance);
        assert_eq!(state.resources, source.xcf_state.as_ref().unwrap().resources);
        let bytes = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), current);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, current);
        count += 1;
    }
    assert_eq!(count, 6);
}
