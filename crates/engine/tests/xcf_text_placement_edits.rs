#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::json;
use sha2::{Digest, Sha256};
fn root() -> std::path::PathBuf {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-spacing-evidence");
    let bytes = std::fs::read(path.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), "2c987cfa774b38d9004717f68a657c6f7c2b3ff00004b43fcf03c8f61a934687");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes).unwrap();
    for (name, hash) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(path.join(name)).unwrap())), hash);
    }
    path
}
fn projection(doc: &photocraft_doc::Document) -> Vec<u8> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8))
        .collect()
}

fn imported(root: &std::path::Path, name: &str) -> photocraft_doc::Document {
    photocraft_io::import("text.xcf", &std::fs::read(root.join(format!("{name}.xcf"))).unwrap()).unwrap().document
}
#[test]
#[ignore = "requires hash-bound independent text placement controls"]
fn registered_placement_eight_moved_controls_native_history_and_immutable_source() {
    let root = root();
    let evidence = root.parent().unwrap().join("text-placement-evidence");
    let manifest = std::fs::read(evidence.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "b3a3bfc82a7c58d4da8c86698b1c71a74c2afae57d5a2438133906e35888f316");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (path, hash) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(evidence.join(path)).unwrap())), hash);
    }
    for name in ["none-32-HEFT-control", "aa-16-HEFT-source", "aa-32-H-E-edit", "aa-40-II-source"] {
        for (suffix, origin) in [("shift", [11, 2]), ("negative", [-5, 7])] {
            let original = imported(&root, name);
            let owner = original.layers[0].id;
            let original_state = original.xcf_state.clone().unwrap();
            let mut session = Session::new();
            session.add_document(original.clone(), None).unwrap();
            let params = json!({"layerId":owner.0.to_string(),"originPx":origin});
            assert_eq!(session.execute("xcf.text.position.set", params.clone()).unwrap()["updated"], true);
            let edited = (*session.active().unwrap().doc).clone();
            let state = edited.xcf_state.as_ref().unwrap();
            assert_eq!(edited.layers, original.layers);
            assert_eq!(state.items, original_state.items);
            assert_eq!(state.resources, original_state.resources);
            assert_eq!(state.opaque, original_state.opaque);
            assert_eq!(state.source_items[0].cache, original_state.source_items[0].cache);
            let mut expected_source = original_state.source_items[0].clone();
            let photocraft_doc::xcf_items::EditableSource::Text { current_origin, .. } = &mut expected_source.source else { panic!() };
            *current_origin = Some(origin);
            assert_eq!(state.source_items[0], expected_source);
            let expected = std::fs::read(evidence.join(format!("{name}-{suffix}.projection.bin"))).unwrap();
            let actual = projection(&edited);
            assert_eq!(actual.len(), expected.len());
            let delta = actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            assert!(delta <= 2, "{name}-{suffix}: {delta}/255");
            let native = photocraft_io::export(&edited, "placed.pcraft", &Default::default()).unwrap();
            let mut expected_native = edited.clone();
            expected_native.name = "placed.pcraft".into();
            assert_eq!(photocraft_io::import("placed.pcraft", &native.bytes).unwrap().document, expected_native);
            assert_eq!(session.execute("xcf.text.position.set", params).unwrap()["updated"], false);
            assert_eq!(session.active().unwrap().history.past_len(), 1);
            session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, original);
            session.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*session.active().unwrap().doc, edited);
            session.execute("xcf.text.position.set", json!({"layerId":owner.0.to_string(),"originPx":[8,4]})).unwrap();
            assert_eq!(*session.active().unwrap().doc, original);
        }
    }
}

#[test]
#[ignore = "requires independent original text placement source"]
fn placement_lock_cancel_overflow_and_exact_parameter_failures_are_atomic() {
    use photocraft_engine::xcf_text_cmds::{current_text_origin, position_edit_candidate, set_position_with_interrupt};
    use std::sync::atomic::{AtomicBool, Ordering};
    let doc = imported(&root(), "aa-16-HEFT-source");
    let owner = doc.layers[0].id;
    let mut locked = doc.clone();
    locked.layers[0].locks.position = true;
    assert_eq!(current_text_origin(&locked, owner), Some([8, 4]));
    assert!(!position_edit_candidate(&locked, owner));
    let mut session = Session::new();
    session.add_document(locked.clone(), None).unwrap();
    assert!(session.execute("xcf.text.position.set", json!({"layerId":owner.0.to_string(),"originPx":[11,2]})).is_err());
    assert_eq!(*session.active().unwrap().doc, locked);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    for bad in [
        json!({}),
        json!({"layerId":owner.0.to_string(),"originPx":[]}),
        json!({"layerId":owner.0.to_string(),"originPx":[11,2,0]}),
        json!({"layerId":owner.0.to_string(),"originPx":[11.0,2]}),
        json!({"layerId":owner.0.to_string(),"originPx":["11",2]}),
        json!({"layerId":owner.0.to_string(),"originPx":[2147483647,0]}),
        json!({"layerId":owner.0.to_string(),"originPx":[536870912,0]}),
    ] {
        assert!(session.execute("xcf.text.position.set", bad).is_err());
        assert_eq!(*session.active().unwrap().doc, doc);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
    for late in [false, true] {
        let flag = AtomicBool::new(!late);
        let check = || flag.load(Ordering::Relaxed);
        let progress = |v: f32| {
            if v == 1. {
                flag.store(true, Ordering::Relaxed)
            }
        };
        let ctl = photocraft_raster::Interrupt::new(&check, &progress);
        assert!(matches!(
            set_position_with_interrupt(&mut session, &json!({"layerId":owner.0.to_string(),"originPx":[11,2]}), &ctl),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*session.active().unwrap().doc, doc);
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
}
#[test]
#[ignore = "requires independent current-placement text original"]
fn content_size_color_spacing_preserve_current_origin_and_immutable_cache() {
    let doc = imported(&root(), "aa-16-HEFT-source");
    let owner = doc.layers[0].id;
    let source = doc.xcf_state.clone().unwrap();
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    session.execute("xcf.text.position.set", json!({"layerId":owner.0.to_string(),"originPx":[11,2]})).unwrap();
    for (command, params) in [
        ("xcf.text.content.set", json!({"layerId":owner.0.to_string(),"content":"II"})),
        ("xcf.text.fontSize.set", json!({"layerId":owner.0.to_string(),"sizePx":32})),
        ("xcf.text.letterSpacing.set", json!({"layerId":owner.0.to_string(),"letterSpacingPx":4})),
        ("xcf.text.color.set", json!({"layerId":owner.0.to_string(),"encoding":"linear","rgbaF32Bits":["3f800000","00000000","00000000","3f000000"]})),
    ] {
        session.execute(command, params).unwrap();
        let current = &session.active().unwrap().doc;
        let state = current.xcf_state.as_ref().unwrap();
        assert_eq!(photocraft_engine::xcf_text_cmds::current_text_origin(current, owner), Some([11, 2]));
        assert_eq!(state.source_items[0].cache, source.source_items[0].cache);
        assert_eq!(state.items, source.items);
        assert_eq!(state.opaque, source.opaque);
        assert_eq!(state.resources, source.resources);
        assert_eq!(current.layers, doc.layers);
        let saved = photocraft_io::export(current, "moved.pcraft", &Default::default()).unwrap();
        let mut expected_native = (**current).clone();
        expected_native.name = "moved.pcraft".into();
        assert_eq!(photocraft_io::import("moved.pcraft", &saved.bytes).unwrap().document, expected_native);
    }
    for _ in 0..5 {
        session.execute("edit.undo", json!({})).unwrap();
    }
    assert_eq!(*session.active().unwrap().doc, doc);
}
