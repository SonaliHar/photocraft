//! Optional imported foundations must never be lost or treated as runtime authority.
mod common;
use photocraft_doc::*;
use photocraft_doc::{item_identity::*, xcf_filters::*, xcf_provenance::*, xcf_resources::*, xcf_state::*};
use photocraft_format::*;
fn document() -> Document {
    let mut d = Document::with_background("own", Size::new(2, 2), ColorMode::Rgb, SampleType::F32, Color::WHITE);
    let id = d.layers[0].id;
    let mut s = XcfState::default();
    s.items.push(ItemState {
        item: ItemRef::Layer(id),
        name: "own".into(),
        custom_name: true,
        visible: true,
        locks: ItemLocks { visibility: true, ..Default::default() },
        parent: None,
        source_tattoo: Some(53),
        source_flags: 7,
        raster_bounds: None,
        mask: None,
        composition: None,
    });
    s.provenance = ProjectProvenance {
        format: "xcf".into(),
        format_version: 26,
        container: "plain".into(),
        source_label: "Own.xcf".into(),
        original_locator: Some("inert/Own.xcf".into()),
        ..Default::default()
    };
    s.resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::LinkedImage,
        name: "Own image".into(),
        collection: None,
        internal: false,
        locator: Some("inert/source.png".into()),
        content_hash: None,
        owned_bytes: None,
        resolution: None,
    });
    s.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(id),
        revision: 9,
        nodes: vec![FilterNode {
            id: FilterId(2),
            operation: OperationIdentity { name: "gegl:exposure".into(), stored_version: "1:0".into(), schema_epoch: "own observed".into() },
            arguments: vec![FilterArgument { name: "exact".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0x7ff8_0000_0000_0042)) }],
            enabled: false,
            opacity_f64_bits: 1.0f64.to_bits(),
            source_clipping: None,
            source_region: None,
            source_display_name: None,
            source_icon: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }],
    });
    let raw = vec![1, 2, 3];
    s.opaque.push(OpaqueRecord {
        owner: Some(ItemRef::Layer(id)),
        namespace: "own".into(),
        identity: "note".into(),
        source_version: 26,
        order: 3,
        flags: 9,
        blake3: blake3::hash(&raw).to_hex().to_string(),
        bytes: raw.into(),
        rendering_required: false,
    });
    d.xcf_state = Some(s);
    d.set_physical_resolution(PhysicalResolution {
        x_dpi: 137.5,
        y_dpi: 92.25,
        unit: PhysicalUnit::Custom(CustomUnit {
            name: "Own unit".into(),
            factor: 2.5399999618530273,
            digits: 3,
            symbol: "ou".into(),
            abbreviation: "own".into(),
        }),
    })
    .unwrap();
    let state = d.xcf_state.as_mut().unwrap();
    state.current_filter_output_precision = FilterOutputPrecision::Half;
    let node = &mut state.filter_stacks[0].nodes[0];
    node.operation =
        OperationIdentity { name: "gegl:color-to-alpha".into(), stored_version: "0:0".into(), schema_epoch: "xcf-22-stock-color-to-alpha-precision-v1".into() };
    node.arguments = vec![
        FilterArgument {
            name: "color".into(),
            value: FilterValue::Color {
                components: [0.375f32, 0.5, 0.625, 0.75].into_iter().map(|value| ExactNumber::Float32Bits(value.to_bits())).collect(),
                encoding: SampleEncoding::NonlinearProfile,
                profile: None,
            },
        },
        FilterArgument { name: "transparency-threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0f32.to_bits())) },
        FilterArgument { name: "opacity-threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits())) },
    ];
    node.source_clipping = Some(0);
    node.source_region = Some(0);
    node.composition = Some(import_compositing::GimpComposition {
        compatibility_epoch: "xcf-22-stock-point-replace-fields-v1".into(),
        source_mode: 62,
        legacy: false,
        stored_blend_space: 1758364241,
        stored_composite_space: 1758364241,
        stored_operator: 1758364241,
        resolved: None,
        pass_through: false,
    });
    d
}

#[test]
fn mutable_half_precision_roundtrip_requires_reader_three_and_preserves_source() {
    let doc = document();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let zip = zip::ZipReader::new(&bytes).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&zip.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
    assert_eq!(manifest["minimum_reader_version"], 3);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    let mut current = doc.clone();
    current.xcf_state.as_mut().unwrap().current_filter_output_precision = FilterOutputPrecision::Canonical;
    let bytes = save_to_bytes(&current, &SaveOptions::default()).unwrap();
    let zip = zip::ZipReader::new(&bytes).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&zip.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
    assert_eq!(manifest["minimum_reader_version"], 2);
    let state_hash = manifest["document"]["xcf_state"].as_str().unwrap();
    use std::io::Read;
    let compressed = zip.read_by_name(&format!("blobs/{state_hash}.zst"), 1 << 20).unwrap();
    let decoder = ruzstd::decoding::StreamingDecoder::new(compressed.as_slice()).unwrap();
    let mut decoded = Vec::new();
    decoder.take((1 << 20) + 1).read_to_end(&mut decoded).unwrap();
    assert!(decoded.len() <= 1 << 20);
    assert_eq!(blake3::hash(&decoded).to_hex().as_str(), state_hash);
    let state: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
    assert_eq!(state, serde_json::to_value(current.xcf_state.as_ref().unwrap()).unwrap());
    assert!(state.get("current_filter_output_precision").is_none());
    assert_eq!(load_from_bytes(&bytes).unwrap(), current);
    assert_eq!(current.layers, doc.layers);
    assert_eq!(current.xcf_state.as_ref().unwrap().provenance, doc.xcf_state.as_ref().unwrap().provenance);
}

#[test]
fn component_half_policy_is_typed_mutable_and_native_reader_three() {
    let mut doc = document();
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation = OperationIdentity {
        name: "gegl:component-extract".into(),
        stored_version: "0:0".into(),
        schema_epoch: "xcf-22-stock-component-extract-four-precision-v1".into(),
    };
    node.arguments = vec![
        FilterArgument { name: "component".into(), value: FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: 20 } },
        FilterArgument { name: "invert".into(), value: FilterValue::Bool(true) },
        FilterArgument { name: "linear".into(), value: FilterValue::Bool(false) },
    ];
    doc.validate_xcf_state().unwrap();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let zip = zip::ZipReader::new(&bytes).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&zip.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
    assert_eq!(manifest["minimum_reader_version"], 3);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
    for variant in 0..8 {
        let mut invalid = doc.clone();
        let node = &mut invalid.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        match variant {
            0 => node.operation.schema_epoch = "other".into(),
            1 => node.operation.stored_version = "0:1".into(),
            2 => node.arguments[0].value = FilterValue::Enum { domain: "gegl:component-extract/component".into(), value: 3 },
            3 => node.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(0)),
            4 => node.arguments[2].name = "invert".into(),
            5 => node.opacity_f64_bits = 0.5f64.to_bits(),
            6 => node.source_region = Some(1),
            _ => invalid.depth = SampleType::F64,
        }
        assert!(invalid.validate_xcf_state().is_err(), "{variant}");
        assert!(save_to_bytes(&invalid, &SaveOptions::default()).is_err());
    }
    let mut promoted = doc.clone();
    promoted.xcf_state.as_mut().unwrap().current_filter_output_precision = FilterOutputPrecision::Canonical;
    assert_eq!(promoted.layers, doc.layers);
    assert_eq!(load_from_bytes(&save_to_bytes(&promoted, &Default::default()).unwrap()).unwrap(), promoted);
}
#[test]
fn half_policy_rejects_forged_context_and_only_last_owner_removal_clears_it() {
    let original = document();
    for variant in 0..8 {
        let mut doc = original.clone();
        let state = doc.xcf_state.as_mut().unwrap();
        match variant {
            0 => doc.depth = SampleType::F64,
            1 => state.filter_stacks.clear(),
            2 => state.filter_stacks[0].nodes[0].operation.name = "gegl:exposure".into(),
            3 => state.filter_stacks[0].nodes[0].source_region = None,
            4 => state.filter_stacks[0].nodes[0].opacity_f64_bits = 0.5f64.to_bits(),
            6 => doc.mode = ColorMode::Cmyk,
            7 => doc.layers[0].content = Layer::group("unqualified", vec![]).content,
            _ => state.filter_stacks[0].nodes[0].composition.as_mut().unwrap().compatibility_epoch = "unknown".into(),
        }
        assert!(doc.validate_xcf_state().is_err());
        assert!(save_to_bytes(&doc, &SaveOptions::default()).is_err());
    }
    let mut state = original.xcf_state.clone().unwrap();
    state.reconcile_filter_output_precision();
    assert_eq!(state.current_filter_output_precision, FilterOutputPrecision::Half);
    state.filter_stacks.clear();
    state.reconcile_filter_output_precision();
    assert_eq!(state.current_filter_output_precision, FilterOutputPrecision::Canonical);
}
#[test]
fn absent_native_policy_defaults_canonical_and_unknown_policy_is_rejected() {
    let mut value = serde_json::to_value(document().xcf_state.unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("current_filter_output_precision");
    let old: XcfState = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(old.current_filter_output_precision, FilterOutputPrecision::Canonical);
    value["current_filter_output_precision"] = "future-unqualified".into();
    assert!(serde_json::from_value::<XcfState>(value).is_err());
}

#[test]
fn native_half_policy_cannot_be_downgraded_to_older_reader() {
    let doc = document();
    let path = common::temp_dir("filter-output-precision");
    PcraftWriter::new().save_dir(&doc, &path, &SaveOptions::default()).unwrap();
    let file = path.join("manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    manifest["minimum_reader_version"] = 2.into();
    std::fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(load_path(&path).is_err());
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn disabled_buffer_retains_policy_and_deleted_last_layer_resets_it() {
    let mut doc = document();
    let mut state = doc.xcf_state.take().unwrap();
    state.filter_stacks[0].nodes[0].enabled = true;
    state.validate(&doc).unwrap();
    state.filter_stacks[0].nodes[0].enabled = false;
    state.validate(&doc).unwrap();
    assert_eq!(state.current_filter_output_precision, FilterOutputPrecision::Half);
    let before = state.clone();
    doc.layers.clear();
    state.reconcile_layers(&doc).unwrap();
    assert_eq!(state.current_filter_output_precision, FilterOutputPrecision::Canonical);
    assert!(state.filter_stacks.is_empty());
    assert_eq!(state.provenance, before.provenance);
    doc.xcf_state = Some(state);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}
