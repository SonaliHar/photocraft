//! Native reader floors protect operative text placement from older readers.
mod common;
use photocraft_doc::{
    Color, ColorMode, Document, PixelFormat, SampleType, Size,
    item_identity::{ItemRef, ResourceId},
    xcf_items::{EditableSource, SourceCache, SourceItem, TransformState},
    xcf_resources::{ResourceKind, ResourceReference},
    xcf_state::{ItemLocks, ItemState, XcfState},
};
use photocraft_format::{PcraftWriter, SaveOptions, load_from_bytes, load_path, save_to_bytes, zip::ZipReader};
use serde_json::json;

fn document(origin: Option<[i32; 2]>) -> Document {
    let mut doc = Document::with_background("Own native text placement", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let owner = ItemRef::Layer(doc.layers[0].id);
    let mut source = json!({"Text": {
        "content":"HEFT", "markup":null, "font":1, "direction":0,
        "source_modified":false, "configuration":[]
    }});
    if let Some(origin) = origin {
        source["Text"]["current_origin"] = json!(origin);
    }
    let source: EditableSource = serde_json::from_value(source).unwrap();
    let pixels = vec![255; 16];
    doc.xcf_state = Some(XcfState {
        items: vec![ItemState {
            item: owner,
            name: "Own source".into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: Some(37),
            source_flags: 0,
            raster_bounds: Some(photocraft_doc::Rect::new(8, 4, 10, 6)),
            mask: None,
            composition: None,
        }],
        resources: vec![ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::Font,
            name: "Own inert font".into(),
            collection: None,
            internal: false,
            locator: None,
            content_hash: None,
            owned_bytes: None,
            resolution: None,
        }],
        source_items: vec![SourceItem {
            owner,
            source,
            transform: Some(TransformState { matrix_f64_bits: [1., 0., 0., 0., 1., 0., 0., 0., 1.].map(f64::to_bits), interpolation: 0, pre_offset: [8, 4] }),
            cache: Some(SourceCache { blake3: blake3::hash(&pixels).to_hex().to_string(), bytes: pixels.into(), width: 2, height: 2, format: PixelFormat::RGBA8 }),
        }],
        ..Default::default()
    });
    doc
}
fn manifest(bytes: &[u8]) -> serde_json::Value {
    let zip = ZipReader::new(bytes).unwrap();
    serde_json::from_slice(&zip.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap()
}
#[test]
fn operative_text_placement_requires_reader_four_and_preserves_all_source_state() {
    let original = document(None);
    for origin in [[11, 2], [-5, 7], [8, 4]] {
        let mut doc = original.clone();
        let source = &mut doc.xcf_state.as_mut().unwrap().source_items[0].source;
        let mut value = serde_json::to_value(&*source).unwrap();
        value["Text"]["current_origin"] = json!(origin);
        *source = serde_json::from_value(value).unwrap();
        let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(manifest(&bytes)["minimum_reader_version"], 4);
        assert_eq!(manifest(&bytes)["format_version"], 4);
        let loaded = load_from_bytes(&bytes).unwrap();
        assert_eq!(loaded, doc);
        let state = loaded.xcf_state.as_ref().unwrap();
        let expected = original.xcf_state.as_ref().unwrap();
        assert_eq!(serde_json::to_value(&state.source_items[0].source).unwrap()["Text"]["current_origin"], json!(origin));
        assert_eq!(state.source_items[0].cache, expected.source_items[0].cache);
        assert_eq!(state.source_items[0].transform, expected.source_items[0].transform);
        assert_eq!(state.items, expected.items);
        assert_eq!(state.resources, expected.resources);
    }
}
#[test]
fn absent_current_placement_keeps_reader_three_and_legacy_defaults() {
    let doc = document(None);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(manifest(&bytes)["minimum_reader_version"], 3);
    assert_eq!(manifest(&bytes)["format_version"], 3);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}
#[test]
fn forged_reader_three_for_current_placement_fails_after_state_decode() {
    let doc = document(Some([-5, 7]));
    let path = common::temp_dir("text-placement-reader");
    PcraftWriter::new().save_dir(&doc, &path, &SaveOptions::default()).unwrap();
    let file = path.join("manifest.json");
    let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    value["minimum_reader_version"] = 3.into();
    std::fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
    let result = load_path(&path);
    std::fs::remove_dir_all(path).unwrap();
    assert!(result.is_err(), "lowering the advertised reader must not hide operative placement");
}
#[test]
fn absurd_current_origin_is_rejected_before_native_output() {
    for origin in [[i32::MIN, 0], [0, i32::MAX]] {
        let doc = document(Some(origin));
        assert!(doc.validate_xcf_state().is_err());
        assert!(save_to_bytes(&doc, &SaveOptions::default()).is_err());
    }
}
#[test]
fn modified_saved_pixels_cannot_silently_ignore_current_text_placement() {
    let mut doc = document(None);
    let source = &mut doc.xcf_state.as_mut().unwrap().source_items[0].source;
    let EditableSource::Text { source_modified, .. } = source else { panic!("text fixture") };
    *source_modified = true;
    assert!(doc.validate_xcf_state().is_ok(), "legacy modified text retains its saved pixels");
    let EditableSource::Text { current_origin, .. } = &mut doc.xcf_state.as_mut().unwrap().source_items[0].source else { panic!("text fixture") };
    *current_origin = Some([11, 2]);
    assert!(doc.validate_xcf_state().is_err(), "modified pixels have no qualified current-origin evaluator");
    assert!(save_to_bytes(&doc, &SaveOptions::default()).is_err());
}
