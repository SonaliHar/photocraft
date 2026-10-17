#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::json;
fn root() -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-size-evidence");
    let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), "caf94a273f103f9ddafdf0090c5a839f1a97d2a24e3f44519913a6bf19d07eb4");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes).unwrap();
    for (name, hash) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(root.join(name)).unwrap())), hash);
    }
    root
}
fn projection(doc: &photocraft_doc::Document) -> Vec<u8> {
    photocraft_compose::gimp_render::try_render(doc, doc.bounds(), &mut |_| Ok(()))
        .unwrap()
        .px
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.).round().clamp(0., 255.) as u8))
        .collect()
}
#[test]
#[ignore = "requires independently authored exact-font AA size originals"]
fn registered_size_edit_independent_projection_native_undo_and_atomic_denial() {
    let original = std::fs::read(root().join("aa-size-source.xcf")).unwrap();
    let doc = photocraft_io::import("text.xcf", &original).unwrap().document;
    let owner = doc.layers[0].id;
    let mut s = Session::new();
    s.add_document(doc.clone(), None).unwrap();
    let result = s.execute("xcf.text.fontSize.set", json!({"layerId":owner.0.to_string(),"sizePx":24})).unwrap();
    assert_eq!(result["updated"], true);
    let edited = (*s.active().unwrap().doc).clone();
    let expected = std::fs::read(root().join("aa-size-edit.projection.bin")).unwrap();
    assert!(projection(&edited).iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2));
    assert_eq!(edited.layers, doc.layers);
    let before = doc.xcf_state.as_ref().unwrap();
    let after = edited.xcf_state.as_ref().unwrap();
    assert_eq!(before.resources, after.resources);
    assert_eq!(before.opaque, after.opaque);
    assert_eq!(before.source_items[0].cache, after.source_items[0].cache);
    assert_ne!(before.source_items[0].source, after.source_items[0].source);
    assert_eq!(before.source_items[0].cache.as_ref().unwrap().height, 20);
    let bytes = photocraft_io::export(&edited, "text.pcraft", &Default::default()).unwrap().bytes;
    let reopened = photocraft_io::import("text.pcraft", &bytes).unwrap().document;
    let mut expected_native = edited.clone();
    expected_native.name = "text.pcraft".into();
    assert_eq!(reopened, expected_native);
    assert_eq!(projection(&reopened), projection(&edited));
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    assert_eq!(s.execute("xcf.text.fontSize.set", json!({"layerId":owner.0.to_string(),"sizePx":24})).unwrap()["updated"], false);
    assert_eq!(s.active().unwrap().history.past_len(), 1);
    for value in [json!(0), json!(-24), json!(24.5), json!("24"), json!(17), json!(1e100), json!(null)] {
        assert!(s.execute("xcf.text.fontSize.set", json!({"layerId":owner.0.to_string(),"sizePx":value})).is_err());
        assert_eq!(*s.active().unwrap().doc, edited);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
    }
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, doc);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, edited);
    assert_eq!(photocraft_engine::xcf_text_cmds::current_font_size(&edited, owner), Some(24));
    assert!(photocraft_engine::xcf_text_cmds::font_size_edit_candidate(&edited, owner));
    s.execute("xcf.text.content.set", json!({"layerId":owner.0.to_string(),"content":"H E"})).unwrap();
    let content_edited = (*s.active().unwrap().doc).clone();
    let expected = std::fs::read(root().join("aa-24-H-E.projection.bin")).unwrap();
    assert!(projection(&content_edited).iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2));
    assert_eq!(photocraft_engine::xcf_text_cmds::current_font_size(&content_edited, owner), Some(24));
    assert_eq!(content_edited.layers, doc.layers);
    assert_eq!(content_edited.xcf_state.as_ref().unwrap().source_items[0].cache, before.source_items[0].cache);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, edited);
    assert_eq!(std::fs::read(root().join("aa-size-source.xcf")).unwrap(), original);
}
#[test]
#[ignore = "requires exact-font originals for size locks, authority and cancellation"]
fn size_lock_font_and_cancel_fail_atomically() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for case in 0..4 {
        let mut doc = photocraft_io::import("text.xcf", &std::fs::read(root().join("aa-size-source.xcf")).unwrap()).unwrap().document;
        let owner = doc.layers[0].id;
        if case == 0 {
            doc.layers[0].locks.pixels = true;
        }
        if case == 1 {
            doc.xcf_state.as_mut().unwrap().resources[0].name = "Unqualified face".into();
        }
        let mut s = Session::new();
        s.add_document(doc.clone(), None).unwrap();
        let cancelled = AtomicBool::new(case == 2);
        let check = || cancelled.load(Ordering::SeqCst);
        let progress = |v: f32| {
            if v == 1. && case == 3 {
                cancelled.store(true, Ordering::SeqCst);
            }
        };
        let ctl = photocraft_raster::Interrupt::new(&check, &progress);
        assert!(photocraft_engine::xcf_text_cmds::set_font_size_with_interrupt(&mut s, &json!({"layerId":owner.0.to_string(),"sizePx":24}), &ctl).is_err());
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}

#[test]
#[ignore = "requires independently authored no-AA original for mode denial"]
fn valid_no_aa_size_edit_is_denied_atomically() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-rectilinear-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "855fd8b448b8190b0cd035e76ce1c503cb98aafa08f634f5ae4002893eca0a4a");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest).unwrap();
    let file = "rectilinear-pairs/shape-source.xcf";
    let bytes = std::fs::read(root.join(file)).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), files[file]);
    let doc = photocraft_io::import("text.xcf", &bytes).unwrap().document;
    let owner = doc.layers[0].id;
    assert!(photocraft_engine::xcf_text_cmds::content_edit_candidate(&doc, owner));
    assert!(!photocraft_engine::xcf_text_cmds::font_size_edit_candidate(&doc, owner));
    let mut s = Session::new();
    s.add_document(doc.clone(), None).unwrap();
    assert!(s.execute("xcf.text.fontSize.set", json!({"layerId":owner.0.to_string(),"sizePx":40})).is_err());
    assert_eq!(*s.active().unwrap().doc, doc);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
}
#[test]
fn malformed_size_parameters_never_create_history() {
    let mut s = Session::new();
    for params in [
        json!({}),
        json!([]),
        json!({"layerId":1,"sizePx":24}),
        json!({"layerId":"01","sizePx":24}),
        json!({"layerId":"18446744073709551616","sizePx":24}),
        json!({"layerId":"1","sizePx":24,"extra":true}),
        json!({"layerId":"1","sizePx":null}),
    ] {
        assert!(s.execute("xcf.text.fontSize.set", params).is_err());
        assert!(s.active().is_none());
    }
}
#[test]
#[ignore = "requires independently bound fresh locked text source"]
fn fresh_locked_size_inspection_does_not_grant_edit_authority() {
    for source_lock in [false, true] {
        let mut doc = photocraft_io::import("text.xcf", &std::fs::read(root().join("aa-size-source.xcf")).unwrap()).unwrap().document;
        let owner = doc.layers[0].id;
        if source_lock {
            doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(owner)).unwrap().locks.content =
                true;
        } else {
            doc.layers[0].locks.pixels = true;
        }
        let before = doc.clone();
        assert_eq!(photocraft_engine::xcf_text_cmds::current_font_size(&doc, owner), Some(16));
        assert!(!photocraft_engine::xcf_text_cmds::font_size_edit_candidate(&doc, owner));
        assert!(!photocraft_engine::xcf_text_cmds::content_edit_candidate(&doc, owner));
        assert_eq!(doc, before);
        let mut s = Session::new();
        s.add_document(doc.clone(), None).unwrap();
        assert!(s.execute("xcf.text.fontSize.set", json!({"layerId":owner.0.to_string(),"sizePx":24})).is_err());
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        let state = doc.xcf_state.as_mut().unwrap();
        state.resources[0].name = "Unqualified face".into();
        let unqualified = doc.clone();
        assert_eq!(photocraft_engine::xcf_text_cmds::current_font_size(&doc, owner), None);
        assert_eq!(doc, unqualified);
    }
}
