#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::json;
fn root() -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-stem-evidence");
    let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), "ba7b36146b20976846cd334a16f2abda9407894970697fd2d9f6aba56c127bef");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes).unwrap();
    for (name, expected) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(root.join(&name)).unwrap())), expected, "{name}");
    }
    root
}
fn source() -> photocraft_doc::Document {
    photocraft_io::import("text.xcf", &std::fs::read(root().join("simple/source.xcf")).unwrap()).unwrap().document
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
#[ignore = "requires independently authored exact-font source and public text edit"]
fn registered_content_edit_native_undo_cache_and_font_identity() {
    let doc = source();
    let mut s = Session::new();
    s.add_document(doc.clone(), None).unwrap();
    let p = json!({"layerId":doc.layers[0].id.0.to_string(),"content":"II"});
    s.execute("xcf.text.content.set", p).unwrap();
    let edited = (*s.active().unwrap().doc).clone();
    let expected = std::fs::read(root().join("simple/edit-reopened.projection.bin")).unwrap();
    assert_eq!(projection(&edited), expected);
    assert_eq!(edited.layers, doc.layers);
    let before = doc.xcf_state.as_ref().unwrap();
    let after = edited.xcf_state.as_ref().unwrap();
    assert_eq!(before.resources, after.resources);
    assert_eq!(before.opaque, after.opaque);
    assert_eq!(before.source_items[0].cache, after.source_items[0].cache);
    assert_ne!(before.source_items[0].source, after.source_items[0].source);
    let encoded = photocraft_io::export(&edited, "text.pcraft", &Default::default()).unwrap();
    let reopened = photocraft_io::import("text.pcraft", &encoded.bytes).unwrap().document;
    let mut expected_native = edited.clone();
    expected_native.name = "text.pcraft".into();
    assert_eq!(reopened, expected_native);
    assert_eq!(projection(&reopened), expected);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, doc);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, edited);
}
#[test]
fn malformed_text_params_never_create_history() {
    let mut s = Session::new();
    s.add_document(
        photocraft_doc::Document::new("bad", photocraft_geom::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8),
        None,
    )
    .unwrap();
    for p in [
        json!(null),
        json!({}),
        json!({"layerId":1,"content":"II"}),
        json!({"layerId":"18446744073709551616","content":"II"}),
        json!({"layerId":"1","content":"fallback"}),
    ] {
        assert!(s.execute("xcf.text.content.set", p).is_err());
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}

#[test]
#[ignore = "requires independent exact-font original for locks and cancellation"]
fn text_locks_unresolved_font_and_cancel_preserve_history_and_source() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for case in 0..4 {
        let mut doc = source();
        let owner = doc.layers[0].id;
        if case == 0 {
            doc.layers[0].locks.pixels = true;
        }
        if case == 1 {
            doc.xcf_state.as_mut().unwrap().resources[0].name = "A different unqualified font".into();
        }
        let mut s = Session::new();
        s.add_document(doc.clone(), None).unwrap();
        let cancel = AtomicBool::new(case == 2);
        let check = || cancel.load(Ordering::SeqCst);
        let progress = |value: f32| {
            if value == 1. && case == 3 {
                cancel.store(true, Ordering::SeqCst)
            }
        };
        let ctl = photocraft_raster::Interrupt::new(&check, &progress);
        let result = photocraft_engine::xcf_text_cmds::set_content_with_interrupt(&mut s, &json!({"layerId":owner.0.to_string(),"content":"II"}), &ctl);
        assert!(result.is_err());
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}

#[test]
#[ignore = "requires original exact-font live source for retained-raster authority guard"]
fn ordinary_raster_mutation_cannot_be_silently_ignored_by_live_text() {
    let doc = source();
    let mut s = Session::new();
    s.add_document(doc.clone(), None).unwrap();
    let result = s.edit_with_interrupt("Paint live text without an explicit source transition", &photocraft_raster::Interrupt::NONE, |doc, _| {
        doc.layers[0].surface_mut().unwrap().write_region(photocraft_geom::Rect::new(12, 14, 13, 15), &[1., 0., 0., 1.]);
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(*s.active().unwrap().doc, doc);
    assert_eq!(s.active().unwrap().history.past_len(), 0);
}
