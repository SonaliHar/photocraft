#[path = "common/xcf.rs"]
mod common;
use photocraft_doc::{LayerContent, Rect, SampleType};
#[test]
fn renamed_content_exact_samples_and_origin() {
    let bytes = common::image(12, 250, 2, 1, 1, &[0x12, 0x34, 0, 1, 0xff, 0xfe, 0xff, 0xff]);
    let r = photocraft_io::import("renamed.pcraft", &bytes).unwrap();
    assert!(r.origin.is_read_only());
    assert_eq!(r.document.depth, SampleType::U16);
    assert_eq!(r.document.layers.len(), 1);
    let LayerContent::Raster(s) = &r.document.layers[0].content else { panic!("raster") };
    assert_eq!(s.to_interleaved(Rect::new(0, 0, 1, 1)), [0x1234u16, 1, 0xfffe, 0xffff].iter().flat_map(|v| v.to_ne_bytes()).collect::<Vec<_>>());
}
#[test]
fn unsupported_blend_in_hidden_layer_is_rejected() {
    let mut bytes = common::image(12, 150, 0, 1, 1, &[1, 2, 3, 255]);
    // Property list starts immediately after the layer name. Insert mode/visibility
    // by replacing the fixed-length layer name with an equal-size properties record.
    let marker = bytes.windows(6).position(|w| w == b"Layer\0").unwrap() + 6;
    // Existing END property becomes mode with deliberately wrong payload: error required.
    bytes[marker..marker + 4].copy_from_slice(&7u32.to_be_bytes());
    assert!(photocraft_io::import("hidden.xcf", &bytes).is_err());
}
#[test]
fn duplicate_palette_indices_survive_native_edit_provenance() {
    let mut palette = 2u32.to_be_bytes().to_vec();
    palette.extend([255, 0, 0, 255, 0, 0]);
    let options = common::FixtureOptions { model: 2, image_properties: vec![(1, palette)], ..Default::default() };
    let bytes = common::image_custom(12, 150, 2, 2, 1, &[0, 255, 1, 127], &options);
    let mut doc = photocraft_io::import("indexed.xcf", &bytes).unwrap().document;
    let layer = &mut doc.layers[0];
    let side = layer.indexed_source.as_ref().unwrap();
    assert_eq!(side.indices().as_slice(), [0, 1]);
    assert_eq!(side.alpha().as_ref().unwrap().as_slice(), [255, 127]);
    let LayerContent::Raster(s) = &mut layer.content else { panic!("raster") };
    let original = s.clone();
    assert!(side.current_matches(s, &doc.color_table.as_ref().unwrap().colors, Rect::new(0, 0, 2, 1)));
    s.write_region(Rect::new(0, 0, 1, 1), &[0.0, 1.0, 0.0, 1.0]);
    assert!(!side.current_matches(s, &doc.color_table.as_ref().unwrap().colors, Rect::new(0, 0, 2, 1)));
    *s = original;
    assert!(side.current_matches(s, &doc.color_table.as_ref().unwrap().colors, Rect::new(0, 0, 2, 1)));
    let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(doc.layers, reopened.layers);
}
#[test]
fn empty_stack_unicode_negative_offsets_and_exact_opaque_samples() {
    let empty = photocraft_io::import("empty.XCF", &common::empty(26)).unwrap();
    assert!(empty.document.layers.is_empty());
    let options = common::FixtureOptions {
        name: Some("レイヤー 🎨".into()),
        layer_type: Some(0),
        layer_properties: vec![(15, [-3i32, -7].iter().flat_map(|v| v.to_be_bytes()).collect())],
        ..Default::default()
    };
    let bytes = common::image_custom(12, 150, 1, 1, 1, &[2, 3, 4], &options);
    let r = photocraft_io::import("unicode", &bytes).unwrap();
    assert_eq!(r.document.layers[0].name, "レイヤー 🎨");
    let LayerContent::Raster(s) = &r.document.layers[0].content else { panic!("raster") };
    assert_eq!(s.to_interleaved(Rect::new(-3, -7, -2, -6)), [2, 3, 4, 255]);
}
