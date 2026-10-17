#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
fn root() -> std::path::PathBuf {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let manifest = std::fs::read(p.join("SHA256SUMS.json")).unwrap();
    assert_eq!(
        Sha256::digest(&manifest).iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "127bdde60da33ab18380057a42c7ea602c8ac48e55d0f2b3d2dee96bd97f4af7"
    );
    let rows: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    for (name, hash) in rows {
        assert_eq!(Sha256::digest(std::fs::read(p.join(name)).unwrap()).iter().map(|b| format!("{b:02x}")).collect::<String>(), hash)
    }
    p
}
fn imported(root: &std::path::Path, state: &Value) -> photocraft_doc::Document {
    photocraft_io::import("text.xcf", &std::fs::read(root.join(state["xcf_file"].as_str().unwrap())).unwrap()).unwrap().document
}
fn pixels(doc: &photocraft_doc::Document) -> Vec<u8> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8))
        .collect()
}
#[test]
#[ignore = "private independently authored underline source/edit controls"]
fn registered_underline_ranges_render_native_undo_and_source_preservation() {
    let root = root();
    let rows: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in rows["rows"].as_array().unwrap().iter().filter(|r| r["negative"] == false && !(r["size_px"] == 16 && r["antialias"] == false)) {
        let states = row["states"].as_array().unwrap();
        let doc = imported(&root, &states[0]);
        let owner = doc.layers[0].id;
        let original = doc.xcf_state.clone().unwrap();
        let layers = doc.layers.clone();
        let name = row["case"].as_str().unwrap();
        let ranges = if name.ends_with("whole") {
            json!([[0, 2]])
        } else if name.ends_with("middle") {
            json!([[2, 4]])
        } else {
            json!([])
        };
        let mut s = Session::new();
        s.add_document(doc.clone(), None).unwrap();
        assert_eq!(s.execute("xcf.text.underline.set", json!({"layerId":owner.0.to_string(),"ranges":ranges})).unwrap()["updated"], true);
        let current = (*s.active().unwrap().doc).clone();
        let expected = std::fs::read(root.join(states[1]["state"]["projection"]["file"].as_str().unwrap())).unwrap();
        assert!(pixels(&current).iter().zip(expected).map(|(a, b)| a.abs_diff(b)).max().unwrap() <= 2, "{name}");
        assert_eq!(current.layers, layers);
        let state = current.xcf_state.as_ref().unwrap();
        assert_eq!(state.resources, original.resources);
        assert_eq!(state.items, original.items);
        assert_eq!(state.opaque, original.opaque);
        assert_eq!(state.source_items[0].cache, original.source_items[0].cache);
        let saved = photocraft_io::export(&current, "text.pcraft", &Default::default()).unwrap();
        let mut expected_native = current.clone();
        expected_native.name = "text.pcraft".into();
        assert_eq!(photocraft_io::import("text.pcraft", &saved.bytes).unwrap().document, expected_native);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, doc);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, current);
        count += 1;
    }
    assert_eq!(count, 9);
}
#[test]
fn underline_command_registered_exact_byte_range_schema() {
    let specs = photocraft_engine::xcf_text_cmds::specs();
    let c = specs.iter().find(|c| c.id == "xcf.text.underline.set").unwrap();
    assert!(c.params.contains("ranges"));
    assert!(c.params.contains("UTF8"));
    assert!(c.params.contains("nonoverlapping"));
}

#[test]
#[ignore = "private independently authored underline source/edit controls"]
fn underline_params_locks_cancellation_and_legacy_closed_are_atomic() {
    let root = root();
    let rows: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let row = rows["rows"].as_array().unwrap().iter().find(|r| r["case"] == "aa-32-whole").unwrap();
    let doc = imported(&root, &row["states"][0]);
    let owner = doc.layers[0].id;
    let mut s = Session::new();
    s.add_document(doc.clone(), None).unwrap();
    for p in [
        json!({}),
        json!({"layerId":owner.0.to_string(),"ranges":[[0,5]]}),
        json!({"layerId":owner.0.to_string(),"ranges":[[1,1]]}),
        json!({"layerId":owner.0.to_string(),"ranges":[[2,4],[0,2]]}),
        json!({"layerId":owner.0.to_string(),"ranges":[[0,3],[2,4]]}),
        json!({"layerId":owner.0.to_string(),"ranges":[[0.0,2]]}),
        json!({"layerId":owner.0.to_string(),"ranges":[[-1,2]]}),
        json!({"layerId":owner.0.to_string(),"ranges":[[0,2]],"extra":true}),
    ] {
        assert!(s.execute("xcf.text.underline.set", p).is_err());
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
    assert!(!photocraft_engine::xcf_text_cmds::content_edit_candidate(&doc, owner));
    assert!(!photocraft_engine::xcf_text_cmds::font_size_edit_candidate(&doc, owner));
    let mut locked = doc.clone();
    locked.layers[0].locks.pixels = true;
    let mut ls = Session::new();
    ls.add_document(locked.clone(), None).unwrap();
    assert!(ls.execute("xcf.text.underline.set", json!({"layerId":owner.0.to_string(),"ranges":[]})).is_err());
    assert_eq!(*ls.active().unwrap().doc, locked);
    use std::sync::atomic::{AtomicBool, Ordering};
    for late in [false, true] {
        let cancelled = AtomicBool::new(!late);
        let check = || cancelled.load(Ordering::Relaxed);
        let progress = |_: f32| {
            cancelled.store(true, Ordering::Relaxed);
        };
        let ctl = photocraft_raster::Interrupt::new(&check, &progress);
        assert!(matches!(
            photocraft_engine::xcf_text_cmds::set_underline_with_interrupt(&mut s, &json!({"layerId":owner.0.to_string(),"ranges":[]}), &ctl),
            Err(photocraft_engine::EngineError::Cancelled)
        ));
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}

#[test]
#[ignore = "private independently authored underline controls"]
fn underline_readonly_inspection_retains_locked_ranges() {
    let root = root();
    let doc = photocraft_io::import("text.xcf", &std::fs::read(root.join("aa-32-whole.source.xcf")).unwrap()).unwrap().document;
    let owner = doc.layers[0].id;
    let mut s = Session::new();
    s.add_document(doc, None).unwrap();
    assert_eq!(photocraft_engine::xcf_text_cmds::current_underline_ranges(&s.active().unwrap().doc, owner), Some(vec![[0, 4]]));
    assert!(photocraft_engine::xcf_text_cmds::underline_edit_candidate(&s.active().unwrap().doc, owner));
    s.execute("layer.setProps", json!({"layer":owner.0,"locks":{"position":true}})).unwrap();
    assert_eq!(photocraft_engine::xcf_text_cmds::current_underline_ranges(&s.active().unwrap().doc, owner), Some(vec![[0, 4]]));
    assert!(!photocraft_engine::xcf_text_cmds::underline_edit_candidate(&s.active().unwrap().doc, owner));
}
