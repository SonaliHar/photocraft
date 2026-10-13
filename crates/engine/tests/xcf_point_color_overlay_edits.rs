#![cfg(feature = "corpus")]
//! Independently authored Color Overlay current edits, native state and history.
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn packet() -> (PathBuf, Value) {
    let base = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full"));
    let root = base.join("point-color-overlay-evidence");
    let bytes = fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(hash(&bytes), "828c1da757d0706cd6eb502699c74c0c2856745d3603c6ab6d1dd56d70df9831");
    let manifest: Value = serde_json::from_slice(&bytes).unwrap();
    let obs = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&obs), manifest["artifacts"].as_array().unwrap().iter().find(|a| a["path"] == "observations.json").unwrap()["sha256"]);
    (root, serde_json::from_slice(&obs).unwrap())
}
fn import(root: &std::path::Path, row: &Value) -> photocraft_doc::Document {
    let bytes = fs::read(root.join(row["source"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), row["sha256"]);
    photocraft_io::import("source.xcf", &bytes).unwrap().document
}
#[test]
#[ignore = "local independent Color Overlay360 frozen source references"]
fn color_overlay_registered_current_references_native_history_and_atomic_guards() {
    let (root, obs) = packet();
    let rows = obs["records"].as_array().unwrap();
    let mut edits = 0;
    let mut negatives = 0;
    for row in rows.iter().filter(|r| r["variant"] == "source") {
        let source = import(&root, row);
        let id = row["id"].as_str().unwrap();
        let family = id.strip_suffix(".source").unwrap();
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(node));
        let fid = node.id.0.to_string();
        let before = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
        for stage in ["edit", "transparent", "opaque", "disabled", "toggle"] {
            let targetid = if stage == "toggle" {
                format!("{}.srgb{}.source", family.rsplit_once(".srgb").unwrap().0, if row["srgb"] == true { 0 } else { 1 })
            } else {
                format!("{family}.{stage}")
            };
            let target = import(&root, rows.iter().find(|r| r["id"] == targetid).unwrap());
            let targetnode = &target.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            if stage == "disabled" {
                session.execute("xcf.filter.enable", json!({"filterId":fid,"enabled":false})).unwrap();
            } else {
                session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":targetnode.arguments})).unwrap();
            }
            let edited = (*session.active().unwrap().doc).clone();
            let current = edited.xcf_state.as_ref().unwrap();
            let original = source.xcf_state.as_ref().unwrap();
            assert_eq!(edited.layers, source.layers);
            assert_eq!(edited.source_color_context, source.source_color_context);
            assert_eq!(current.opaque, original.opaque);
            assert_eq!(current.provenance, original.provenance);
            assert_eq!(current.filter_masks, original.filter_masks);
            assert_eq!(current.filter_stacks[0].nodes[0].arguments, targetnode.arguments);
            assert_eq!(current.filter_stacks[0].nodes[0].enabled, targetnode.enabled);
            let rendered = photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap();
            let expected = photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(rendered.px, expected.px, "{id}/{stage}");
            let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), edited);
            assert_eq!(session.active().unwrap().history.past_len(), 1);
            session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, source);
            session.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, edited);
            edits += 1;
        }
        let mut arguments = node.arguments.clone();
        arguments.iter_mut().find(|argument| argument.name == "value").unwrap().value = photocraft_doc::xcf_filters::FilterValue::Bool(true);
        for params in [
            json!({"filterId":fid,"arguments":arguments}),
            json!({"filterId":fid,"arguments":[{"name":"value","value":{"Color":{"components":[{"Float32Bits":0},{"Float32Bits":0},{"Float32Bits":0},{"Float32Bits":f32::NAN.to_bits()}],"encoding":"nonlinear_profile","profile":null}}}]}),
            json!({"filterId":fid,"arguments":[{"name":"srgb","value":{"Number":{"Float32Bits":0}}}]}),
            json!({"filterId":fid,"arguments":[{"name":"unknown","value":{"Bool":true}}]}),
        ] {
            let mut session = Session::new();
            session.add_document(source.clone(), None).unwrap();
            assert!(session.execute("xcf.filter.set", params).is_err());
            assert_eq!(*session.active().unwrap().doc, source);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
            negatives += 1;
        }
        let mut cancelled = Session::new();
        cancelled.add_document(source.clone(), None).unwrap();
        assert!(matches!(
            photocraft_engine::xcf_filter_cmds::set_with_interrupt(
                &mut cancelled,
                &json!({"filterId":fid,"arguments":node.arguments}),
                &photocraft_raster::Interrupt::cancel_only(&|| true)
            ),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*cancelled.active().unwrap().doc, source);
        assert_eq!(cancelled.active().unwrap().history.past_len(), 0);
        assert_eq!(photocraft_format::save_to_bytes(&cancelled.active().unwrap().doc, &Default::default()).unwrap(), before);
        for mutation in 0..4 {
            let mut bad = source.clone();
            let state = bad.xcf_state.as_mut().unwrap();
            match mutation {
                0 => state.filter_stacks[0].nodes[0].operation.schema_epoch = "unknown".into(),
                1 => state.filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = 0,
                2 => state.items.iter_mut().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(bad.layers[0].id)).unwrap().locks.content = true,
                _ => state.filter_stacks[0].nodes[0].operation.stored_version = "future".into(),
            }
            let mut session = Session::new();
            session.add_document(bad.clone(), None).unwrap();
            assert!(session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":node.arguments})).is_err());
            assert_eq!(*session.active().unwrap().doc, bad);
            assert_eq!(session.active().unwrap().history.past_len(), 0);
            negatives += 1;
        }
    }
    assert_eq!(edits, 360);
    assert_eq!(negatives, 576);
}

#[test]
#[ignore = "local independent Color Overlay disabled/source activation controls"]
fn color_overlay_disabled_activation_and_same_argument_noop_preserve_native_source() {
    let (root, obs) = packet();
    let rows = obs["records"].as_array().unwrap();
    let mut count = 0;
    for row in rows.iter().filter(|r| r["variant"] == "disabled") {
        let source = import(&root, row);
        let targetid = row["id"].as_str().unwrap().strip_suffix(".disabled").unwrap().to_owned() + ".source";
        let target = import(&root, rows.iter().find(|r| r["id"] == targetid).unwrap());
        let node = &source.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let fid = node.id.0.to_string();
        let mut session = Session::new();
        session.add_document(source.clone(), None).unwrap();
        session.execute("xcf.filter.set", json!({"filterId":fid,"arguments":node.arguments})).unwrap();
        assert_eq!(session.active().unwrap().history.past_len(), 0);
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("xcf.filter.enable", json!({"filterId":fid,"enabled":true})).unwrap();
        let edited = (*session.active().unwrap().doc).clone();
        assert_eq!(edited.layers, source.layers);
        assert_eq!(edited.source_color_context, source.source_color_context);
        assert_eq!(edited.xcf_state.as_ref().unwrap().opaque, source.xcf_state.as_ref().unwrap().opaque);
        assert_eq!(edited.xcf_state.as_ref().unwrap().filter_masks, source.xcf_state.as_ref().unwrap().filter_masks);
        assert_eq!(
            photocraft_compose::gimp_render::try_render(&edited, edited.bounds(), &mut |_| Ok(())).unwrap().px,
            photocraft_compose::gimp_render::try_render(&target, target.bounds(), &mut |_| Ok(())).unwrap().px
        );
        let native = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), edited);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, source);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*session.active().unwrap().doc, edited);
        count += 1;
    }
    assert_eq!(count, 72);
}
