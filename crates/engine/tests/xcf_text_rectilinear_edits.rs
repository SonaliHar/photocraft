#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::json;
fn root() -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-rectilinear-evidence");
    let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), "855fd8b448b8190b0cd035e76ce1c503cb98aafa08f634f5ae4002893eca0a4a");
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
fn registered_rectilinear_shapes_and_kerning_native_history() {
    for (case, content) in [("shape", "H E"), ("kern", "TL")] {
        let doc = photocraft_io::import("text.xcf", &std::fs::read(root().join(format!("rectilinear-pairs/{case}-source.xcf"))).unwrap()).unwrap().document;
        let mut s = Session::new();
        s.add_document(doc.clone(), None).unwrap();
        assert!(photocraft_engine::xcf_text_cmds::content_edit_candidate(&doc, doc.layers[0].id));
        assert!(!photocraft_engine::xcf_text_cmds::content_edit_candidate(&doc, photocraft_doc::LayerId(0)));
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        let p = json!({"layerId":doc.layers[0].id.0.to_string(),"content":content});
        s.execute("xcf.text.content.set", p).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        let expected = std::fs::read(root().join(format!("rectilinear-pairs/{case}-edit.projection.bin"))).unwrap();
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
