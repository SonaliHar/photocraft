#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::json;
fn root() -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-rectilinear-aa-evidence");
    let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), "5b76f7e300ceae7f7051338dfd796701047928d1d2b8b3d00b18863e1533501e");
    let files: std::collections::BTreeMap<String, String> = serde_json::from_slice(&bytes).unwrap();
    for (name, expected) in files {
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(root.join(&name)).unwrap())), expected, "{name}");
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
#[ignore = "requires independently authored exact-font source and public text edit"]
fn registered_aa_shapes_and_kerning_native_history() {
    for (case, content) in [("shape", "H E"), ("kern", "TL")] {
        let doc = photocraft_io::import("text.xcf", &std::fs::read(root().join(format!("{case}-source.xcf"))).unwrap()).unwrap().document;
        let mut s = Session::new();
        s.add_document(doc.clone(), None).unwrap();

        assert!(!photocraft_engine::xcf_text_cmds::content_edit_candidate(&doc, photocraft_doc::LayerId(0)));
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        let p = json!({"layerId":doc.layers[0].id.0.to_string(),"content":content});
        s.execute("xcf.text.content.set", p).unwrap();
        assert!(photocraft_engine::xcf_text_cmds::content_edit_candidate(&doc, doc.layers[0].id));
        let edited = (*s.active().unwrap().doc).clone();
        let expected = std::fs::read(root().join(format!("{case}-edit.projection.bin"))).unwrap();
        let rendered = projection(&edited);
        assert!(rendered.iter().zip(&expected).all(|(a, b)| a.abs_diff(*b) <= 2));
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
        assert_eq!(projection(&reopened), rendered);
        for unqualified in ["O", "A", "iiiiiiiii", "        "] {
            let before = (*s.active().unwrap().doc).clone();
            let history = s.active().unwrap().history.past_len();
            assert!(s.execute("xcf.text.content.set", json!({"layerId":doc.layers[0].id.0.to_string(),"content":unqualified})).is_err());
            assert_eq!(*s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().history.past_len(), history);
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, doc);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
    }
}

#[test]
#[ignore = "requires independent exact-font original for locks and cancellation"]
fn aa_text_locks_unresolved_font_and_cancel_preserve_history_and_source() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for case in 0..4 {
        let mut doc = photocraft_io::import("text.xcf", &std::fs::read(root().join("shape-source.xcf")).unwrap()).unwrap().document;
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
        let result = photocraft_engine::xcf_text_cmds::set_content_with_interrupt(&mut s, &json!({"layerId":owner.0.to_string(),"content":"H E"}), &ctl);
        assert!(result.is_err());
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}
