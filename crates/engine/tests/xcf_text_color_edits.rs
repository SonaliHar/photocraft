#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
fn root() -> std::path::PathBuf {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-color-evidence");
    let bytes = std::fs::read(path.join("SHA256SUMS.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), "deef5cdada65df4ffc05e9e2e84deaa75f1ed61cee11e71fd5111e26f52cc086");
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
fn params(doc: &photocraft_doc::Document, owner: photocraft_doc::LayerId) -> Value {
    let state = doc.xcf_state.as_ref().unwrap();
    let (_, _, color) =
        photocraft_text::xcf::qualify_colored_rectilinear_source(&state.source_items[0], &state.resources[0], [8, 4], &photocraft_raster::Interrupt::NONE)
            .unwrap();
    json!({"layerId":owner.0.to_string(),"encoding":match color.encoding {photocraft_text::xcf::TextColorEncoding::Linear=>"linear",photocraft_text::xcf::TextColorEncoding::Nonlinear=>"nonlinear"},"rgbaF32Bits":color.rgba_f32_bits.map(|b|format!("{b:08x}"))})
}
#[test]
#[ignore = "requires independently bound exact-font color originals"]
fn registered_color_edit_native_projection_history_and_immutable_cache() {
    let root = root();
    for name in ["none-32-primary", "aa-16-gray", "aa-32-fractional", "alpha-only/aa-40-alpha-only"] {
        let source = std::fs::read(root.join(format!("{name}-source.xcf"))).unwrap();
        let doc = photocraft_io::import("text.xcf", &source).unwrap().document;
        let expected = photocraft_io::import("text.xcf", &std::fs::read(root.join(format!("{name}-edit.xcf"))).unwrap()).unwrap().document;
        let before = doc.xcf_state.clone().unwrap();
        let owner = doc.layers[0].id;
        let layers = doc.layers.clone();
        let mut s = Session::new();
        s.add_document(doc.clone(), None).unwrap();
        let params = params(&expected, owner);
        assert_eq!(s.execute("xcf.text.color.set", params.clone()).unwrap()["changed"], true);
        let edited = (*s.active().unwrap().doc).clone();
        assert_eq!(edited.layers, layers);
        let after = edited.xcf_state.as_ref().unwrap();
        assert_eq!(after.resources, before.resources);
        assert_eq!(after.opaque, before.opaque);
        assert_eq!(after.source_items[0].cache, before.source_items[0].cache);
        let photocraft_doc::xcf_items::EditableSource::Text { configuration, .. } = &after.source_items[0].source else { panic!("not text") };
        let photocraft_doc::xcf_items::EditableSource::Text { configuration: expected_config, .. } =
            &expected.xcf_state.as_ref().unwrap().source_items[0].source
        else {
            panic!("not text")
        };
        assert_eq!(configuration, expected_config);
        let actual = projection(&edited);
        let reference = std::fs::read(root.join(format!("{name}-edit.projection.bin"))).unwrap();
        assert!(actual.iter().zip(reference).map(|(a, b)| a.abs_diff(b)).max().unwrap() <= 2);
        let saved = photocraft_io::export(&edited, "text.pcraft", &Default::default()).unwrap();
        let mut native_expected = edited.clone();
        native_expected.name = "text.pcraft".into();
        assert_eq!(photocraft_io::import("text.pcraft", &saved.bytes).unwrap().document, native_expected);
        assert_eq!(s.execute("xcf.text.color.set", params).unwrap()["changed"], false);
        assert_eq!(s.active().unwrap().history.past_len(), 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, doc);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, edited);
        assert_eq!(std::fs::read(root.join(format!("{name}-source.xcf"))).unwrap(), source);
    }
}
#[test]
#[ignore = "requires independently bound color source for atomic denial"]
fn color_lock_font_cancel_and_bad_numeric_bits_are_atomic() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = root();
    let original = photocraft_io::import("text.xcf", &std::fs::read(root.join("aa-16-primary-source.xcf")).unwrap()).unwrap().document;
    let edit = photocraft_io::import("text.xcf", &std::fs::read(root.join("aa-16-primary-edit.xcf")).unwrap()).unwrap().document;
    for case in 0..5 {
        let mut doc = original.clone();
        let owner = doc.layers[0].id;
        if case == 0 {
            doc.layers[0].locks.pixels = true;
        }
        if case == 1 {
            doc.xcf_state.as_mut().unwrap().resources[0].name = "Unqualified font".into();
        }
        if case == 0 {
            assert_eq!(
                photocraft_engine::xcf_text_cmds::current_text_color(&doc, owner),
                photocraft_engine::xcf_text_cmds::current_text_color(&original, owner)
            );
            assert!(photocraft_engine::xcf_text_cmds::current_text_color(&doc, owner).is_some());
            assert!(!photocraft_engine::xcf_text_cmds::color_edit_candidate(&doc, owner));
        }
        if case == 1 {
            assert!(photocraft_engine::xcf_text_cmds::current_text_color(&doc, owner).is_none());
            assert!(!photocraft_engine::xcf_text_cmds::color_edit_candidate(&doc, owner));
        }
        let mut p = params(&edit, owner);
        if case == 4 {
            p["rgbaF32Bits"][0] = json!("7fc00000");
        }
        let mut s = Session::new();
        s.add_document(doc.clone(), None).unwrap();
        let cancelled = AtomicBool::new(case == 2);
        let check = || cancelled.load(Ordering::SeqCst);
        let progress = |v: f32| {
            if case == 3 && v == 1. {
                cancelled.store(true, Ordering::SeqCst);
            }
        };
        let ctl = photocraft_raster::Interrupt::new(&check, &progress);
        assert!(photocraft_engine::xcf_text_cmds::set_color_with_interrupt(&mut s, &p, &ctl).is_err());
        assert_eq!(*s.active().unwrap().doc, doc);
        assert_eq!(s.active().unwrap().history.past_len(), 0);
    }
}
#[test]
fn malformed_color_params_never_create_history() {
    let mut s = Session::new();
    for p in [
        json!({}),
        json!([]),
        json!({"layerId":1,"encoding":"linear","rgbaF32Bits":[]}),
        json!({"layerId":"1","encoding":"auto","rgbaF32Bits":["0","0","0","0"]}),
        json!({"layerId":"1","encoding":"linear","rgbaF32Bits":["3F800000","00000000","00000000","3f800000"]}),
    ] {
        assert!(s.execute("xcf.text.color.set", p).is_err());
        assert!(s.active().is_none());
    }
}
