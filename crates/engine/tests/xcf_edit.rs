//! Imported documents remain editable without losing immutable source provenance.
#![cfg(feature = "corpus")]
use photocraft_engine::Session;
use serde_json::json;
use std::path::PathBuf;
fn opened(name: &str) -> Session {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf").join(name);
    let imported = photocraft_io::import(name, &std::fs::read(path).expect("run cargo xtask corpus --xcf")).unwrap();
    let mut session = Session::new();
    session.open_document(imported.document, None).unwrap();
    session
}
#[test]
fn imported_mask_edit_and_undo_restore_exact_document_and_encoding() {
    let mut s = opened("rgb-u8-mask.xcf");
    let id = s.active().unwrap().doc.layers.iter().find(|l| l.mask.is_some()).unwrap().id;
    s.active_mut().unwrap().active_layer = Some(id);
    let before = s.active().unwrap().doc.clone();
    s.execute("image.adjustments.invert", json!({"target":"mask"})).unwrap();
    let after = &s.active().unwrap().doc;
    assert_ne!(after.layer(id).unwrap().mask, before.layer(id).unwrap().mask);
    assert_eq!(after.layer(id).unwrap().surface(), before.layer(id).unwrap().surface());
    assert_eq!(after.sample_encoding, before.sample_encoding);
    assert_eq!(after.source_color_context, before.source_color_context);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, before);
}
fn current(s: &Session) -> bool {
    let d = &s.active().unwrap().doc;
    let layer = d.layers.iter().find(|l| l.indexed_source.is_some()).unwrap();
    let source = layer.indexed_source.as_ref().unwrap();
    let canonical = &d.xcf_state.as_ref().unwrap().editable_indexed.iter().find(|r| r.owner == layer.id).unwrap().raster;
    // Expanded RGB cannot distinguish duplicate palette ordinals. Current identity must
    // also compare the editable index/alpha planes and their operative bounds.
    canonical.indices() == source.indices()
        && canonical.alpha() == source.alpha().as_ref()
        && canonical.palette() == source.palette()
        && source.current_matches(layer.surface().unwrap(), &d.color_table.as_ref().unwrap().colors, canonical.bounds())
}
#[test]
fn indexed_pixels_palette_and_position_edits_invalidate_then_undo_restores_identity() {
    let mut s = opened("indexed-u8-duplicate-indices.xcf");
    let id = s.active().unwrap().doc.layers.iter().find(|l| l.indexed_source.is_some()).unwrap().id;
    s.active_mut().unwrap().active_layer = Some(id);
    let before = s.active().unwrap().doc.clone();
    assert!(current(&s));
    for (command, params) in [
        ("image.adjustments.invert", json!({})),
        ("image.mode.colorTable", json!({"entries":{"0":"#ff00ff"}})),
        ("layer.translate", json!({"layer":id.0,"dx":1,"dy":2})),
    ] {
        s.execute(command, params).unwrap();
        assert!(!current(&s), "{command}: edited content cannot claim original index identity");
        if command == "image.adjustments.invert" {
            let after = &s.active().unwrap().doc;
            let canonical = &after.xcf_state.as_ref().unwrap().editable_indexed.iter().find(|r| r.owner == id).unwrap().raster;
            assert!(before.layer(id).unwrap().indexed_source.as_ref().unwrap().indices().contains(&1));
            assert!(canonical.indices().iter().all(|&index| index == 0), "equal palette entries use the first nearest-color ordinal");
            assert_eq!(after.layer(id).unwrap().surface(), before.layer(id).unwrap().surface(), "duplicate palette colors expand identically");
        }
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().indexed_source, before.layer(id).unwrap().indexed_source);
        assert_eq!(s.active().unwrap().doc.sample_encoding, before.sample_encoding);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, before);
        assert!(current(&s));
    }
}

#[test]
fn layer_export_and_native_smart_conversion_preserve_imported_interpretation() {
    let s = opened("rgb-u16_linear.xcf");
    let d = &s.active().unwrap().doc;
    let layer = &d.layers[0];
    let exported = photocraft_engine::layer_menu_cmds::layer_document(d, layer.id).unwrap();
    assert_eq!(exported.sample_encoding, d.sample_encoding);
    assert_eq!(exported.source_color_context, d.source_color_context);
    assert_eq!(exported.icc_profile, d.icc_profile);
    let smart = photocraft_engine::smart_cmds::layer_to_smart(d, layer).unwrap();
    let photocraft_doc::LayerContent::Smart(content) = &smart.content else { panic!("expected smart layer") };
    let photocraft_doc::SmartSource::Embedded { file_name, bytes } = &content.source else { panic!("expected native embedded source") };
    let child = photocraft_engine::smart_cmds::decode_source(file_name, bytes).unwrap();
    assert_eq!(child.sample_encoding, d.sample_encoding);
    assert_eq!(child.source_color_context, d.source_color_context);
    assert_eq!(child.icc_profile, d.icc_profile);
    let mut after = (**d).clone();
    after.layers = vec![smart];
    let expected = photocraft_compose::render(d, d.bounds()).unwrap();
    let actual = photocraft_compose::render(&after, after.bounds()).unwrap();
    for (a, b) in actual.px.iter().zip(expected.px) {
        for (a, b) in a.iter().zip(b) {
            assert!((a - b).abs() <= 2.0 / 65535.0);
        }
    }
}

#[test]
fn placed_xcf_refresh_keeps_destination_interpretation_and_original_embedded_bytes() {
    for name in ["rgb-u16_linear.xcf", "rgb-float_non_linear.xcf", "rgb-u8-wide-profile.xcf"] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf").join(name);
        let bytes = std::fs::read(path).unwrap();
        let source = photocraft_io::import(name, &bytes).unwrap().document;
        for linear in [false, true] {
            let mut session = Session::new();
            let mut target = photocraft_doc::Document::new("target", source.size, photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U16);
            target.icc_profile = Some(if linear { photocraft_cms::Builtin::LinearSrgb } else { photocraft_cms::Builtin::Srgb }.profile().to_bytes());
            if linear {
                target.sample_encoding = Some(photocraft_doc::SampleEncoding::Linear);
            }
            session.open_document(target, None).unwrap();
            photocraft_engine::file_cmds::place_bytes(&mut session, name, bytes.clone(), None, &json!({"fit":false})).unwrap();
            let id = session.active().unwrap().active_layer.unwrap();
            let layer = session.active().unwrap().doc.layer(id).unwrap();
            let photocraft_doc::LayerContent::Smart(smart) = &layer.content else { panic!("expected smart object") };
            let expected = smart.cache.as_ref().unwrap().clone();
            // Explicit refresh must work after the initial placement cache is gone.
            let refreshed = photocraft_engine::smart_cmds::render(&session.active().unwrap().doc, smart).unwrap().unwrap();
            assert_eq!(refreshed, expected);
            session.execute("layer.translate", json!({"layer":id.0,"dx":1,"dy":2})).unwrap();
            let layer = session.active().unwrap().doc.layer(id).unwrap();
            let photocraft_doc::LayerContent::Smart(smart) = &layer.content else { panic!("expected smart object") };
            assert_eq!(smart.cache.as_ref().unwrap(), &photocraft_algo::resample::translate_surface(&expected, 1, 2));
            let photocraft_doc::SmartSource::Embedded { bytes: original, .. } = &smart.source else { panic!("expected embedded source") };
            assert_eq!(original.as_slice(), bytes.as_slice());
            let before = session.active().unwrap().doc.clone();
            assert!(photocraft_engine::smart_cmds::stack_image(name, &bytes, before.pixel_format(), photocraft_doc::StackMode::Mean).is_err());
            assert_eq!(session.active().unwrap().doc, before);
        }
    }
}
