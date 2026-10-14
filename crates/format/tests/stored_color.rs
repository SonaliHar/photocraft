//! Raw storage identity only; these drafts do not qualify a color evaluator.
mod common;
use photocraft_doc::{item_identity::*, xcf_filters::*, xcf_state::*, *};
use photocraft_format::*;

fn raw() -> FilterValue {
    FilterValue::StoredColor(ExactColorStorage {
        layout: "HSL float".into(),
        // Signed zero and a NaN payload are uninterpreted source words here.
        samples: [0x80000000u32, 0x7fc00042, 0x3f800000].into_iter().flat_map(u32::to_le_bytes).collect(),
        opaque_tail: vec![0, 255, 17],
    })
}
fn document(value: FilterValue) -> Document {
    let mut doc = Document::with_background("Raw storage", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let owner = ItemRef::Layer(doc.layers[0].id);
    let mut state = XcfState::default();
    state.items.push(ItemState {
        item: owner,
        name: "Raw storage".into(),
        custom_name: true,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(37),
        source_flags: 5,
        raster_bounds: None,
        mask: None,
        composition: None,
    });
    state.filter_stacks.push(FilterStack {
        owner,
        revision: 0,
        nodes: vec![FilterNode {
            id: FilterId(21),
            operation: OperationIdentity { name: "own:raw-storage".into(), stored_version: "0:0".into(), schema_epoch: "own-storage-only".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![FilterArgument { name: "raw".into(), value }],
            enabled: false,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }],
    });
    doc.xcf_state = Some(state);
    doc
}
fn floor(bytes: &[u8]) -> u64 {
    let archive = zip::ZipReader::new(bytes).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&archive.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
    manifest["minimum_reader_version"].as_u64().unwrap()
}
#[test]
fn raw_roundtrip_requires_nine_but_existing_color_keeps_legacy_floor() {
    let doc = document(raw());
    let before = doc.clone();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(floor(&bytes), 9);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    assert_eq!(doc, before);
    let legacy =
        document(FilterValue::Color { components: vec![ExactNumber::Float32Bits(0x80000000)], encoding: SampleEncoding::NonlinearProfile, profile: None });
    let bytes = save_to_bytes(&legacy, &SaveOptions::default()).unwrap();
    assert_eq!(floor(&bytes), 2);
    assert_eq!(load_from_bytes(&bytes).unwrap(), legacy);
}
#[test]
fn nested_graph_and_config_are_not_hidden_from_reader_floor() {
    let graph = FilterGraph {
        input: None,
        nodes: vec![GraphNode {
            id: GraphNodeId(31),
            operation: OperationIdentity { name: "own:raw".into(), stored_version: "0:0".into(), schema_epoch: "own".into() },
            version_origin: GraphVersionOrigin::Stored,
            arguments: vec![FilterArgument { name: "nested".into(), value: FilterValue::Config(vec![FilterArgument { name: "raw".into(), value: raw() }]) }],
        }],
        edges: vec![],
        output: GraphNodeId(31),
        original_pipeline: None,
    };
    let doc = document(FilterValue::Graph(graph));
    let bytes = save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(floor(&bytes), 9);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}
#[test]
fn forged_lower_floor_cannot_publish_new_raw_variant() {
    let doc = document(raw());
    let path = common::temp_dir("stored-color-reader-floor");
    PcraftWriter::new().save_dir(&doc, &path, &SaveOptions::default()).unwrap();
    let file = path.join("manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    manifest["minimum_reader_version"] = 8.into();
    std::fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(load_path(&path).is_err());
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn cancelled_load_preserves_raw_source_and_malformed_storage_is_rejected() {
    let doc = document(raw());
    let before = doc.clone();
    let bytes = save_to_bytes(&doc, &Default::default()).unwrap();
    assert!(load_from_bytes_with_interrupt(&bytes, &Default::default(), &photocraft_raster::Interrupt::cancel_only(&|| true)).is_err());
    assert_eq!(doc, before);
    for value in [
        FilterValue::StoredColor(ExactColorStorage { layout: "bad\0layout".into(), samples: vec![1], opaque_tail: vec![] }),
        FilterValue::StoredColor(ExactColorStorage { layout: "HSL float".into(), samples: vec![], opaque_tail: vec![] }),
        FilterValue::StoredColor(ExactColorStorage { layout: "HSL float".into(), samples: vec![1; 1025], opaque_tail: vec![] }),
        FilterValue::StoredColor(ExactColorStorage { layout: "HSL float".into(), samples: vec![1], opaque_tail: vec![0; (1 << 20) + 1] }),
    ] {
        assert!(document(value).validate_xcf_state().is_err());
    }
}

#[test]
fn unknown_new_storage_fields_are_not_silently_discarded() {
    let mut value = serde_json::to_value(raw()).unwrap();
    value["StoredColor"]["future_profile"] = "unresolved-required-identity".into();
    assert!(serde_json::from_value::<FilterValue>(value).is_err());
    let exact = serde_json::json!({"StoredColor":{"layout":"HSL float","samples":[0,1,255],"opaque_tail":[4,0]}});
    assert_eq!(serde_json::to_value(serde_json::from_value::<FilterValue>(exact.clone()).unwrap()).unwrap(), exact);
}
