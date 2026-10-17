#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::json;
fn root() -> std::path::PathBuf {
    let root =
        std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("local original corpus required")).join("vector-monotone-arch-evidence");
    let bytes = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(blake3::hash(&bytes).to_hex().as_str(), "abf6f3617c0c6ce93987101cb1f0ce2e287f96c66f42992ca9aa0f72efc92743");
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
#[ignore = "requires independent stock arch translation source/edit references"]
fn registered_arch_translation_native_undo_preserves_original_evidence() {
    for name in ["basic", "asymmetric-fractional", "wide", "reversed"] {
        let d = root().join(name);
        let source = std::fs::read(d.join("source.xcf")).unwrap();
        let doc = photocraft_io::import("arch.xcf", &source).unwrap().document;
        let expected = std::fs::read(d.join("edit.ref.rgba-u8.bin")).unwrap();
        let mut s = Session::new();
        s.add_document(doc.clone(), None).unwrap();
        s.execute("path.transform", json!({"pathId":doc.paths[0].id.0.to_string(),"translateX":2,"translateY":-1})).unwrap();
        let edited = (*s.active().unwrap().doc).clone();
        assert_eq!(projection(&edited), expected, "{name}");
        assert_eq!(edited.layers, doc.layers);
        assert_eq!(edited.xcf_state, doc.xcf_state);
        assert_ne!(edited.paths[0].path, doc.paths[0].path);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        let encoded = photocraft_io::export(&edited, "arch.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("arch.pcraft", &encoded.bytes).unwrap().document;
        let mut expected_native = edited.clone();
        expected_native.name = "arch.pcraft".into();
        assert_eq!(expected_native, reopened);
        assert_eq!(projection(&reopened), expected);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, doc);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
    }
}
#[test]
#[ignore = "requires independent arch source for locked and unsupported current geometry"]
fn arch_current_geometry_lock_and_cancel_are_atomic() {
    let doc = photocraft_io::import("arch.xcf", &std::fs::read(root().join("basic/source.xcf")).unwrap()).unwrap().document;
    for variant in 0..4 {
        let mut input = doc.clone();
        if variant == 0 {
            input
                .xcf_state
                .as_mut()
                .unwrap()
                .items
                .iter_mut()
                .find(|i| i.item == photocraft_doc::item_identity::ItemRef::Layer(input.layers[0].id))
                .unwrap()
                .locks
                .position = true;
        }
        let mut s = Session::new();
        s.add_document(input.clone(), None).unwrap();
        let stop = std::sync::atomic::AtomicBool::new(false);
        let cancelled = || stop.load(std::sync::atomic::Ordering::Relaxed);
        let result = match variant {
            0 => s.execute("path.transform", json!({"pathId":input.paths[0].id.0.to_string(),"translateX":2,"translateY":-1})).map(|_| ()),
            1 => s.execute("path.transform", json!({"pathId":input.paths[0].id.0.to_string(),"translateX":0.001})).map(|_| ()),
            2 => s.execute("path.transform", json!({"pathId":"18446744073709551615","translateX":2})).map(|_| ()),
            _ => s.edit_with_interrupt("cancelled arch", &photocraft_raster::Interrupt::cancel_only(&cancelled), |d, _| {
                d.paths[0].path.subpaths[0].knots[0].out_ctrl.x += 1.;
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }),
        };
        assert!(result.is_err(), "variant{variant}");
        assert_eq!(*s.active().unwrap().doc, input);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}
