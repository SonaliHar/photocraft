#![cfg(feature = "corpus")]
//! Frozen Perceptual Color Rotate current edits; immutable original state remains separate.
use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
use photocraft_engine::Session;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|n| format!("{n:02x}")).collect()
}
fn packet() -> (PathBuf, Value) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/color-rotate-perceptual-evidence");
    let bytes = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "8b338d140af0add90f7390e7fc2db4f0adef9dac24f45151ff50140dd66bb59c");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    let bytes = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), manifest["files"].as_array().unwrap().iter().find(|n| n["file"] == "observations.json").unwrap()["sha256"]);
    (root, serde_json::from_slice(&bytes).unwrap())
}
fn import(root: &Path, row: &Value) -> photocraft_doc::Document {
    let bytes = fs::read(root.join(row["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    photocraft_io::import("color-rotate.xcf", &bytes).unwrap().document
}
#[test]
#[ignore = "frozen source/edit/disabled Perceptual originals, registered history/native"]
fn registered_color_rotate_current_native_history() {
    let (root, obs) = packet();
    let rows = obs["records"].as_array().unwrap();
    let mut count = 0;
    for family in ["rgb", "gray"] {
        let source = import(&root, rows.iter().find(|r| r["id"] == format!("{family}.source")).unwrap());
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(node));
        for stage in ["edit", "disabled"] {
            let target = import(&root, rows.iter().find(|r| r["id"] == format!("{family}.{stage}")).unwrap());
            let desired = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            session.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":node.arguments})).unwrap();
            assert_eq!(session.active().unwrap().history.past_len(), 0);
            if stage == "edit" {
                session.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":desired.arguments})).unwrap();
            } else {
                session.execute("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":false})).unwrap();
            }
            let current = (*session.active().unwrap().doc).clone();
            assert_eq!(current.layers, source.layers);
            assert_eq!(current.source_color_context, source.source_color_context);
            let a = current.xcf_state.as_ref().unwrap();
            let b = source.xcf_state.as_ref().unwrap();
            assert_eq!(a.opaque, b.opaque);
            assert_eq!(a.resources, b.resources);
            assert_eq!(a.filter_masks, b.filter_masks);
            assert_eq!(a.provenance, b.provenance);
            assert_eq!(
                photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap().px,
                photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px
            );
            let saved = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened, current);
            let restored = &reopened.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let mut continued = Session::new();
            continued.add_document(reopened.clone(), None).unwrap();
            continued.execute("xcf.filter.set", json!({"filterId":restored.id.0.to_string(),"arguments":restored.arguments})).unwrap();
            assert_eq!(continued.active().unwrap().history.past_len(), 0);
            assert_eq!(*continued.active().unwrap().doc, reopened);
            session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, source);
            session.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, current);
            count += 1;
        }
    }
    assert_eq!(count, 4);
}
#[test]
#[ignore = "Perceptual Color Rotate hostile current schema is atomic"]
fn color_rotate_wrong_type_unproved_geometry_and_locks_are_atomic() {
    let (root, obs) = packet();
    let source = import(&root, &obs["records"][0]);
    let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    let mut count = 0;
    for mutation in 0..18 {
        let mut args = node.arguments.clone();
        match mutation {
            0 => args[0].value = FilterValue::Bool(true),
            1 => args[1].value = FilterValue::Number(ExactNumber::Float32Bits(200f32.to_bits())),
            2 => args[1].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
            3 => args[1].value = FilterValue::Number(ExactNumber::Float64Bits(15f64.to_bits())),
            4 => args[7].value = FilterValue::Enum { domain: "wrong".into(), value: 0 },
            5 => args[7].value = FilterValue::Enum { domain: "gegl:color-rotate/gray-mode".into(), value: 2 },
            6 => args.push(args[0].clone()),
            7 => args[6].value = FilterValue::Number(ExactNumber::Float32Bits((-0.1f32).to_bits())),
            8 => args[1].value = FilterValue::Number(ExactNumber::Float32Bits(120f32.to_bits())),
            9 => args[4].value = args[5].value.clone(),
            10 => args[8].name = "unknown".into(),
            11 => args[9].value = FilterValue::Number(ExactNumber::Float32Bits(f32::INFINITY.to_bits())),
            12 => args[1].value = FilterValue::Number(ExactNumber::Float32Bits(16f32.to_bits())),
            13 => args[2].value = FilterValue::Number(ExactNumber::Float32Bits(121f32.to_bits())),
            14 => args[6].value = FilterValue::Number(ExactNumber::Float32Bits(0.25f32.to_bits())),
            15 => args[7].value = FilterValue::Enum { domain: "gegl:color-rotate/gray-mode".into(), value: 1 },
            16 => args[8].value = FilterValue::Number(ExactNumber::Float32Bits(46f32.to_bits())),
            _ => args[9].value = FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())),
        }
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        assert!(session.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":args})).is_err());
        assert_eq!(*session.active().unwrap().doc, source);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
        count += 1;
    }
    assert_eq!(count, 18);
    let mut args = node.arguments.clone();
    args[4].value = FilterValue::Number(ExactNumber::Float32Bits(210f32.to_bits()));
    args[5].value = FilterValue::Number(ExactNumber::Float32Bits(330f32.to_bits()));
    let mut locked = source.clone();
    let owner = locked.layers[0].id;
    locked.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(owner)).unwrap().locks.content = true;
    let mut session = Session::new();
    session.add_document(locked.clone(), None).unwrap();
    assert!(session.execute("xcf.filter.set", json!({"filterId":node.id.0.to_string(),"arguments":args})).is_err());
    assert_eq!(*session.active().unwrap().doc, locked);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    let mut session = Session::new();
    session.add_document(source.clone(), None).unwrap();
    assert!(matches!(
        photocraft_engine::xcf_filter_cmds::set_with_interrupt(
            &mut session,
            &json!({"filterId":node.id.0.to_string(),"arguments":args}),
            &photocraft_raster::Interrupt::cancel_only(&|| true)
        ),
        Err(photocraft_engine::EngineError::Cancelled)
    ));
    assert_eq!(*session.active().unwrap().doc, source);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
}

#[test]
#[ignore = "disabled original Color Rotate activation uses current authoritative pipeline"]
fn color_rotate_disabled_activation_native_and_undo() {
    let (root, obs) = packet();
    let rows = obs["records"].as_array().unwrap();
    for family in ["rgb", "gray"] {
        let disabled = import(&root, rows.iter().find(|r| r["id"] == format!("{family}.disabled")).unwrap());
        let source = import(&root, rows.iter().find(|r| r["id"] == format!("{family}.source")).unwrap());
        let node = &disabled.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let mut session = Session::new();
        session.add_document(disabled.clone(), None).unwrap();
        session.execute("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":false})).unwrap();
        assert_eq!(session.active().unwrap().history.past_len(), 0);
        session.execute("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":true})).unwrap();
        let current = &session.active().unwrap().doc;
        assert_eq!(current.layers, disabled.layers);
        assert_eq!(current.xcf_state.as_ref().unwrap().opaque, disabled.xcf_state.as_ref().unwrap().opaque);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(current, current.bounds(), &mut |_| Ok(())).unwrap().px,
            photocraft_compose::gimp_render::try_render(&source, source.bounds(), &mut |_| Ok(())).unwrap().px
        );
        let saved = photocraft_format::save_to_bytes(current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), **current);
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, disabled);
    }
}

#[test]
#[ignore = "Color Rotate removal/native/undo retains immutable source evidence"]
fn color_rotate_remove_current_native_and_undo() {
    let (root, obs) = packet();
    let rows = obs["records"].as_array().unwrap();
    for family in ["rgb", "gray"] {
        let source = import(&root, rows.iter().find(|r| r["id"] == format!("{family}.source")).unwrap());
        let disabled = import(&root, rows.iter().find(|r| r["id"] == format!("{family}.disabled")).unwrap());
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute("xcf.filter.remove", json!({"filterId":node.id.0.to_string()})).unwrap();
        let current = &session.active().unwrap().doc;
        assert_eq!(current.layers, source.layers);
        let current_records = &current.xcf_state.as_ref().unwrap().opaque;
        let original_records = &source.xcf_state.as_ref().unwrap().opaque;
        assert_eq!(current_records.len(), original_records.len());
        for (after, before) in current_records.iter().zip(original_records) {
            assert_eq!(after.bytes, before.bytes);
            assert_eq!(after.blake3, before.blake3);
            if matches!(before.owner, Some(photocraft_doc::item_identity::ItemRef::Mask(_))) {
                assert_eq!(after.owner, None);
                assert!(after.identity.ends_with(&before.identity));
                assert!(after.identity.starts_with("deleted-filter-mask/"));
            } else {
                assert_eq!(after, before);
            }
        }
        assert!(current.xcf_state.as_ref().unwrap().filter_stacks.iter().all(|s| s.nodes.is_empty()));
        assert_eq!(
            photocraft_compose::gimp_render::try_render(current, current.bounds(), &mut |_| Ok(())).unwrap().px,
            photocraft_compose::gimp_render::try_render(&disabled, disabled.bounds(), &mut |_| Ok(())).unwrap().px
        );
        let saved = photocraft_format::save_to_bytes(current, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap(), **current);
        assert_eq!(session.active().unwrap().history.past_len(), 1);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
    }
}

#[test]
#[ignore = "four independently saved fractional source/edit pairs, current/native/history"]
fn registered_color_rotate_fractional_pairs_native_history() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/color-rotate-perceptual-boundary-evidence");
    let bytes = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "72e93210100f51ce988929d0ba9010f817b7ca5c1e69cb4f66caaae0eeacc9ac");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    let bytes = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), manifest["files"].as_array().unwrap().iter().find(|r| r["file"] == "observations.json").unwrap()["sha256"]);
    let observed: Value = serde_json::from_slice(&bytes).unwrap();
    let rows = observed["records"].as_array().unwrap();
    let mut count = 0;
    for family in ["rgb.arc-a", "rgb.arc-b", "gray.arc-a", "gray.arc-b"] {
        let source = import(&root, rows.iter().find(|r| r["id"] == format!("{family}-source")).unwrap());
        let target = import(&root, rows.iter().find(|r| r["id"] == format!("{family}-edit")).unwrap());
        let original = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let desired = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(original));
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute("xcf.filter.set", json!({"filterId":original.id.0.to_string(),"arguments":desired.arguments})).unwrap();
        let current = (*session.active().unwrap().doc).clone();
        assert_eq!(current.layers, source.layers);
        assert_eq!(current.source_color_context, source.source_color_context);
        let a = current.xcf_state.as_ref().unwrap();
        let b = source.xcf_state.as_ref().unwrap();
        assert_eq!(a.opaque, b.opaque);
        assert_eq!(a.resources, b.resources);
        assert_eq!(a.filter_masks, b.filter_masks);
        assert_eq!(a.provenance, b.provenance);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(&current, current.bounds(), &mut |_| Ok(())).unwrap().px,
            photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px
        );
        let bytes = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened, current);
        let mut continued = Session::new();
        continued.add_document(reopened.clone(), None).unwrap();
        continued.execute("xcf.filter.set", json!({"filterId":original.id.0.to_string(),"arguments":desired.arguments})).unwrap();
        assert_eq!(continued.active().unwrap().history.past_len(), 0);
        assert_eq!(*continued.active().unwrap().doc, reopened);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, current);
        for mutation in 0..6 {
            let mut args = desired.arguments.clone();
            match mutation {
                0 => args[0].value = FilterValue::Bool(true),
                1 => args[4].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits())),
                2 => args[4].value = FilterValue::Number(ExactNumber::Float64Bits(207.5f64.to_bits())),
                3 => args[4].value = args[5].value.clone(),
                4 => args[1].value = FilterValue::Number(ExactNumber::Float32Bits(18f32.to_bits())),
                _ => args[9].value = FilterValue::Number(ExactNumber::Float32Bits(0.3f32.to_bits())),
            }
            let mut denied = Session::new();
            denied.add_document(source.clone(), None).unwrap();
            assert!(denied.execute("xcf.filter.set", json!({"filterId":original.id.0.to_string(),"arguments":args})).is_err());
            assert_eq!(*denied.active().unwrap().doc, source);
            assert_eq!(denied.active().unwrap().history.past_len(), 0);
        }
        let mut locked = source.clone();
        let owner = locked.layers[0].id;
        locked.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(owner)).unwrap().locks.content =
            true;
        let mut denied = Session::new();
        denied.add_document(locked.clone(), None).unwrap();
        assert!(denied.execute("xcf.filter.set", json!({"filterId":original.id.0.to_string(),"arguments":desired.arguments})).is_err());
        assert_eq!(*denied.active().unwrap().doc, locked);
        assert_eq!(denied.active().unwrap().history.past_len(), 0);
        let mut cancelled = Session::new();
        cancelled.add_document(source.clone(), None).unwrap();
        assert!(matches!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut cancelled,
                &json!({"filterId":original.id.0.to_string(),"arguments":desired.arguments}),
                &photocraft_raster::Interrupt::cancel_only(&|| true)
            ),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*cancelled.active().unwrap().doc, source);
        assert_eq!(cancelled.active().unwrap().history.past_len(), 0);
        count += 1;
    }
    assert_eq!(count, 4);
}
