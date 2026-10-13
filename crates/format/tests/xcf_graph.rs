//! Pure canonical graph structure, source-version absence, and native compatibility.
//! These synthetic identities do not qualify a GEGL evaluator or stock schema.
use photocraft_doc::item_identity::{FilterId, GraphNodeId, ItemRef};
use photocraft_doc::xcf_filters::*;
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size, XcfState};
use photocraft_format::{SaveOptions, load_from_bytes, save_to_bytes, zip::ZipReader};
mod common;
fn identity() -> OperationIdentity {
    OperationIdentity { name: "own:typed-model".into(), stored_version: "own-resolved-version".into(), schema_epoch: "own-synthetic-model-only".into() }
}
fn graph(input: bool, origin: GraphVersionOrigin) -> FilterGraph {
    FilterGraph {
        input: input.then_some(GraphNodeId(1)),
        nodes: vec![GraphNode { id: GraphNodeId(2), operation: identity(), version_origin: origin, arguments: vec![] }],
        edges: if input {
            vec![GraphEdge { from: GraphNodeId(1), to: GraphNodeId(2), output_port: "output".into(), input_port: "input".into() }]
        } else {
            vec![]
        },
        output: GraphNodeId(2),
        original_pipeline: Some("own original literal pipeline".into()),
    }
}
fn document(value: FilterValue) -> Document {
    let mut doc = Document::with_background("Own graph state", Size::new(2, 2), ColorMode::Rgb, SampleType::U16, Color::WHITE);
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(doc.layers[0].id),
        revision: 4,
        nodes: vec![FilterNode {
            id: FilterId(71),
            operation: identity(),
            source_display_name: None,
            source_icon: None,
            arguments: vec![FilterArgument { name: "graph".into(), value }],
            enabled: true,
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
fn minimum(bytes: &[u8]) -> u64 {
    let zip = ZipReader::new(bytes).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&zip.read_by_name("manifest.json", 1 << 20).unwrap()).unwrap();
    json["minimum_reader_version"].as_u64().unwrap()
}
#[test]
fn source_version_absence_and_external_input_round_trip_without_fabrication() {
    let graph = graph(true, GraphVersionOrigin::SchemaResolved);
    assert_eq!(graph.nodes[0].source_version(), None);
    assert_eq!(graph.nodes[0].resolved_version(), "own-resolved-version");
    let doc = document(FilterValue::Graph(graph));
    doc.validate_xcf_state().unwrap();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(minimum(&bytes), 3);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}
#[test]
fn legacy_graph_defaults_omit_new_fields_and_keep_reader_two() {
    let graph = graph(false, GraphVersionOrigin::Stored);
    let value = serde_json::to_value(&graph).unwrap();
    assert!(value.get("input").is_none());
    assert!(value["nodes"][0].get("version_origin").is_none());
    let decoded: FilterGraph = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.input, None);
    assert_eq!(decoded.nodes[0].source_version(), Some("own-resolved-version"));
    let doc = document(FilterValue::Graph(decoded));
    let bytes = save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(minimum(&bytes), 2);
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}
#[test]
fn each_new_graph_semantic_and_nested_origin_requires_reader_three() {
    for value in [
        FilterValue::Graph(graph(true, GraphVersionOrigin::Stored)),
        FilterValue::Graph(graph(false, GraphVersionOrigin::SchemaResolved)),
        FilterValue::Config(vec![FilterArgument { name: "nested".into(), value: FilterValue::Graph(graph(false, GraphVersionOrigin::SchemaResolved)) }]),
    ] {
        let doc = document(value);
        let bytes = save_to_bytes(&doc, &Default::default()).unwrap();
        assert_eq!(minimum(&bytes), 3);
        let path = common::temp_dir("graph-reader");
        photocraft_format::PcraftWriter::new().save_dir(&doc, &path, &Default::default()).unwrap();
        let file = path.join("manifest.json");
        let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        json["minimum_reader_version"] = 2.into();
        std::fs::write(file, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(photocraft_format::load_path(&path).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }
}
#[test]
fn identity_external_input_is_valid_but_zero_collision_incoming_and_cycles_fail() {
    let mut identity_graph = graph(true, GraphVersionOrigin::SchemaResolved);
    identity_graph.nodes.clear();
    identity_graph.edges.clear();
    identity_graph.output = GraphNodeId(1);
    document(FilterValue::Graph(identity_graph)).validate_xcf_state().unwrap();
    for mode in 0..5 {
        let mut graph = graph(true, GraphVersionOrigin::SchemaResolved);
        match mode {
            0 => graph.input = Some(GraphNodeId(0)),
            1 => graph.input = Some(GraphNodeId(2)),
            2 => graph.edges.push(GraphEdge { from: GraphNodeId(2), to: GraphNodeId(1), output_port: "output".into(), input_port: "input".into() }),
            3 => graph.edges.push(GraphEdge { from: GraphNodeId(2), to: GraphNodeId(2), output_port: "output".into(), input_port: "aux".into() }),
            _ => graph.output = GraphNodeId(999),
        }
        assert!(document(FilterValue::Graph(graph)).validate_xcf_state().is_err());
    }
}
#[test]
fn seed_epoch_requires_reader_three_after_active_profile_becomes_explicit_icc() {
    use photocraft_doc::import_compositing::{GimpComposition, XCF13_ISOLATED_MULTIPLY_SEED_EPOCH};
    let mut doc = Document::new("Own structural epoch", Size::new(2, 2), ColorMode::Rgb, SampleType::U16);
    doc.layers.push(photocraft_doc::Layer::group("Own group", vec![]));
    let mut state = XcfState::default();
    state.reconcile_layers(&doc).unwrap();
    state.items[0].composition = Some(GimpComposition {
        compatibility_epoch: XCF13_ISOLATED_MULTIPLY_SEED_EPOCH.into(),
        source_mode: 30,
        legacy: false,
        stored_blend_space: 1,
        stored_composite_space: 1,
        stored_operator: 2,
        resolved: None,
        pass_through: false,
    });
    let original = state.items[0].composition.clone();
    doc.xcf_state = Some(state);
    doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::Icc;
    let bytes = save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(minimum(&bytes), 3);
    let loaded = load_from_bytes(&bytes).unwrap();
    assert_eq!(loaded.xcf_state.as_ref().unwrap().items[0].composition, original);
    assert_eq!(loaded.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc);
    let path = common::temp_dir("seed-reader");
    photocraft_format::PcraftWriter::new().save_dir(&doc, &path, &Default::default()).unwrap();
    let file = path.join("manifest.json");
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    json["minimum_reader_version"] = 2.into();
    std::fs::write(file, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(photocraft_format::load_path(&path).is_err());
    std::fs::remove_dir_all(path).unwrap();
}
