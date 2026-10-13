use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity},
};
use photocraft_io::xcf_graphs::{GraphMappingLimits, ResolvedOperation, map_graph};
use photocraft_raster::Interrupt;
use photocraft_xcf::{GraphEntry, GraphValue, StoredGraph};
fn operation(source: &str, name: &str) -> GraphEntry {
    let start = source.find(name).unwrap();
    GraphEntry::Operation { identity: name.into(), start: start as u64, end: (start + name.len()) as u64 }
}
fn property(source: &str, name: &str, value: &str) -> GraphEntry {
    let literal = format!("{name}={value}");
    let start = source.find(&literal).unwrap();
    GraphEntry::Property {
        name: name.into(),
        value: GraphValue::Literal { source: value.into(), numeric_bits: value.parse::<f64>().ok().map(f64::to_bits) },
        start: start as u64,
        end: (start + literal.len()) as u64,
    }
}
#[test]
fn typed_chain_has_external_input_exact_parameters_and_original_source() {
    let source = "exposure exposure=-0.0";
    let graph = StoredGraph { entries: vec![operation(source, "exposure"), property(source, "exposure", "-0.0")], nodes: 1, edges: 1 };
    let mut id = 100;
    let result = map_graph(
        &graph,
        source,
        GraphMappingLimits::default(),
        &Interrupt::NONE,
        &mut || {
            id += 1;
            GraphNodeId(id)
        },
        &mut |name, props| {
            assert_eq!(name, "exposure");
            assert_eq!(props[0].name, "exposure");
            Ok(ResolvedOperation {
                operation: OperationIdentity {
                    name: "gegl:exposure".into(),
                    stored_version: "synthetic-v1".into(),
                    schema_epoch: "synthetic-graph-test".into(),
                },
                arguments: vec![FilterArgument {
                    name: "exposure".into(),
                    value: FilterValue::Number(ExactNumber::Float64Bits(props[0].numeric_bits.unwrap())),
                }],
            })
        },
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(result.input, Some(GraphNodeId(101)));
    assert_eq!(result.output, GraphNodeId(102));
    assert_eq!(result.edges[0].from, GraphNodeId(101));
    assert_eq!(result.edges[0].input_port, "input");
    assert_eq!(result.original_pipeline.as_deref(), Some(source));
    assert_eq!(result.nodes[0].arguments[0].value, FilterValue::Number(ExactNumber::Float64Bits((-0.0f64).to_bits())));
}

#[path = "../../xcf/tests/common/mod.rs"]
mod common;
fn parsed(source: &str) -> StoredGraph {
    fn string(v: &mut Vec<u8>, s: &str) {
        common::u32be(v, s.len() as u32 + 1);
        v.extend_from_slice(s.as_bytes());
        v.push(0);
    }
    let mut effect = Vec::new();
    for s in ["Owned graph", "icon", "gegl:gegl", "0:0"] {
        string(&mut effect, s);
    }
    let mut arg = Vec::new();
    string(&mut arg, "string");
    common::u32be(&mut arg, 4);
    string(&mut arg, source);
    common::prop(&mut effect, 45, &arg);
    common::prop(&mut effect, 0, &[]);
    common::pointer(&mut effect, 0, 22);
    let options = common::FixtureOptions { effect_records: vec![effect], ..Default::default() };
    let bytes = common::image_custom(22, 150, 0, 1, 1, &[1, 2, 3, 255], &options);
    photocraft_xcf::decode(&bytes, &Default::default()).unwrap().layers.remove(0).effects.remove(0).arguments.remove(0).graph.unwrap()
}
fn synthetic_schema(name: &str, props: &[photocraft_io::xcf_graphs::GraphLiteral<'_>]) -> Result<ResolvedOperation, photocraft_io::IoError> {
    let arguments = props
        .iter()
        .map(|p| FilterArgument {
            name: p.name.into(),
            value: p.numeric_bits.map_or_else(|| FilterValue::String(p.source.into()), |bits| FilterValue::Number(ExactNumber::Float64Bits(bits))),
        })
        .collect();
    Ok(ResolvedOperation {
        operation: OperationIdentity {
            name: format!("gegl:{name}"),
            stored_version: "synthetic-v1".into(),
            schema_epoch: "synthetic-test-not-qualified".into(),
        },
        arguments,
    })
}
fn mapped(graph: &StoredGraph, source: &str) -> Result<photocraft_doc::xcf_filters::FilterGraph, photocraft_io::IoError> {
    let mut id = 100;
    map_graph(
        graph,
        source,
        GraphMappingLimits::default(),
        &Interrupt::NONE,
        &mut || {
            id += 1;
            GraphNodeId(id)
        },
        &mut synthetic_schema,
        &mut |_| Ok(()),
    )
}
#[test]
fn nested_aux_references_preserve_shared_input_and_order() {
    let source = "id=original exposure exposure=0.5 over aux=[ ref=original gaussian-blur std-dev-x=1.5 ]";
    let result = mapped(&parsed(source), source).unwrap();
    assert_eq!(result.nodes.len(), 3);
    assert_eq!(result.edges.len(), 4);
    assert_eq!(result.nodes.iter().map(|n| n.operation.name.as_str()).collect::<Vec<_>>(), ["gegl:exposure", "gegl:over", "gegl:gaussian-blur"]);
    let blur = result.nodes[2].id;
    let over = result.nodes[1].id;
    assert!(result.edges.iter().any(|e| e.from == result.input.unwrap() && e.to == blur && e.input_port == "input"));
    assert!(result.edges.iter().any(|e| e.from == blur && e.to == over && e.input_port == "aux"));
    assert_eq!(result.output, over);
    assert_eq!(result.nodes[2].source_version(), None);
}
#[test]
fn unknown_required_properties_and_inert_resource_references() {
    let source = "load path='https://invalid.example/no-read'";
    let graph = parsed(source);
    let mut id = 0;
    let result = map_graph(
        &graph,
        source,
        GraphMappingLimits::default(),
        &Interrupt::NONE,
        &mut || {
            id += 1;
            GraphNodeId(id)
        },
        &mut |name, props| {
            assert_eq!(name, "load");
            assert_eq!(props[0].source, "'https://invalid.example/no-read'");
            Ok(ResolvedOperation {
                operation: OperationIdentity { name: "gegl:load".into(), stored_version: "synthetic-v1".into(), schema_epoch: "inert-resource-test".into() },
                arguments: vec![FilterArgument { name: "path".into(), value: FilterValue::Resource(photocraft_doc::item_identity::ResourceId(99)) }],
            })
        },
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(result.nodes[0].arguments[0].value, FilterValue::Resource(photocraft_doc::item_identity::ResourceId(99)));
    assert_eq!(result.original_pipeline.as_deref(), Some(source));
    let error = map_graph(
        &graph,
        source,
        GraphMappingLimits::default(),
        &Interrupt::NONE,
        &mut || {
            id += 1;
            GraphNodeId(id)
        },
        &mut |_, _| Err(photocraft_io::IoError::Unsupported("unqualified resource schema".into())),
        &mut |_| Ok(()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("unqualified resource schema"));
    let error = map_graph(
        &graph,
        source,
        GraphMappingLimits::default(),
        &Interrupt::NONE,
        &mut || {
            id += 1;
            GraphNodeId(id)
        },
        &mut |name, _| synthetic_schema(name, &[]),
        &mut |_| Ok(()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("omitted"));
}
#[test]
fn duplicate_properties_ids_invalid_spans_and_numeric_bits_fail_atomically() {
    for source in ["exposure exposure=0 exposure=1", "exposure"] {
        let mut graph = parsed(source);
        if source == "exposure" {
            graph.entries[0] = GraphEntry::Operation { identity: "exposure".into(), start: 1, end: 8 };
        }
        assert!(mapped(&graph, source).is_err());
    }
    let source = "exposure exposure=-0.0";
    let mut graph = parsed(source);
    if let GraphEntry::Property { value: GraphValue::Literal { numeric_bits, .. }, .. } = &mut graph.entries[1] {
        *numeric_bits = Some(0.0f64.to_bits());
    }
    assert!(mapped(&graph, source).unwrap_err().to_string().contains("bits mismatch"));
    let graph = parsed("exposure");
    assert!(
        map_graph(&graph, "exposure", GraphMappingLimits::default(), &Interrupt::NONE, &mut || GraphNodeId(1), &mut synthetic_schema, &mut |_| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("duplicated")
    );
}
fn raw_reference_graph(source: &str, id: &str) -> StoredGraph {
    let aux_start = source.find("aux=").unwrap();
    StoredGraph {
        entries: vec![
            operation(source, "over"),
            property(source, "id", "a"),
            GraphEntry::Property {
                name: "aux".into(),
                value: GraphValue::Chain(vec![property(source, "ref", id), operation(source, "exposure")]),
                start: aux_start as u64,
                end: source.len() as u64,
            },
        ],
        nodes: 2,
        edges: 3,
    }
}
#[test]
fn manually_constructed_cycle_dangling_and_deep_graphs_reject() {
    for (source, id, error) in [("over id=a aux=[ ref=a exposure ]", "a", "cycle"), ("over id=a aux=[ ref=missing exposure ]", "missing", "dangling")] {
        assert!(mapped(&raw_reference_graph(source, id), source).unwrap_err().to_string().contains(error));
    }
    let deep = format!("{}exposure{}", "over aux=[ ".repeat(25), "] ".repeat(25));
    assert!(mapped(&parsed(&deep), &deep).unwrap_err().to_string().contains("depth"));
}
#[test]
fn counts_budgets_and_midmapping_cancellation_leave_no_partial_graph() {
    let source = "exposure exposure=0.5";
    let graph = parsed(source);
    for limits in [
        GraphMappingLimits { nodes: 0, ..Default::default() },
        GraphMappingLimits { edges: 0, ..Default::default() },
        GraphMappingLimits { entries: 0, ..Default::default() },
        GraphMappingLimits { resident_bytes: 0, ..Default::default() },
    ] {
        let mut id = 0;
        assert!(
            map_graph(
                &graph,
                source,
                limits,
                &Interrupt::NONE,
                &mut || {
                    id += 1;
                    GraphNodeId(id)
                },
                &mut synthetic_schema,
                &mut |_| Ok(())
            )
            .is_err()
        );
    }
    let mut bad = graph.clone();
    bad.edges += 1;
    assert!(mapped(&bad, source).unwrap_err().to_string().contains("counts mismatch"));
    let mut id = 0;
    assert!(
        map_graph(
            &graph,
            source,
            GraphMappingLimits::default(),
            &Interrupt::NONE,
            &mut || {
                id += 1;
                GraphNodeId(id)
            },
            &mut synthetic_schema,
            &mut |_| Err(photocraft_io::IoError::Unsupported("caller budget denied".into()))
        )
        .unwrap_err()
        .to_string()
        .contains("caller budget denied")
    );
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 6;
    let mut id = 0;
    assert!(matches!(
        map_graph(
            &graph,
            source,
            GraphMappingLimits::default(),
            &Interrupt::cancel_only(&cancel),
            &mut || {
                id += 1;
                GraphNodeId(id)
            },
            &mut synthetic_schema,
            &mut |_| Ok(())
        ),
        Err(photocraft_io::IoError::Cancelled)
    ));
}
#[test]
#[ignore = "requires reviewed independently authored exceptional graph corpus"]
fn authored_graphs_keep_source_spelling_and_require_explicit_nested_schemas() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/graph-enrollment");
    for name in ["own-exceptional-graph.xcf", "own-exceptional-graph.edit.xcf"] {
        let bytes = std::fs::read(root.join(name)).unwrap();
        use sha2::{Digest, Sha256};
        let expected = if name.ends_with(".edit.xcf") {
            "686a216c1590f0cadc456bc046bca102e9ad574a12263a20b356bb381ad6635b"
        } else {
            "6ba4feeb9eba93790dde6cb31a41354bffdef208d12ec1d4c91981841562bed7"
        };
        assert_eq!(Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), expected);
        let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        let effect = file.layers.iter().flat_map(|d| &d.effects).find(|e| e.operation == "gegl:gegl").unwrap();
        let arg = effect.arguments.iter().find(|a| a.name == "string").unwrap();
        let graph = arg.graph.as_ref().unwrap();
        let photocraft_xcf::ArgumentValue::String(source) = &arg.value else { panic!("string") };
        assert_eq!(graph.nodes, 2);
        let mut id = 0;
        let error = map_graph(
            graph,
            source,
            GraphMappingLimits::default(),
            &Interrupt::NONE,
            &mut || {
                id += 1;
                GraphNodeId(id)
            },
            &mut |_, _| Err(photocraft_io::IoError::Unsupported("nested operation schema is not qualified".into())),
            &mut |_| Ok(()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("not qualified"));
        assert!(file.layers.iter().any(|d| d.unsupported.iter().any(|s| s.contains("effects"))));
    }
}

#[test]
fn omitted_required_tokens_and_forged_literal_spans_are_not_published() {
    let source = "exposure omitted-operation";
    let graph = StoredGraph { entries: vec![operation(source, "exposure")], nodes: 1, edges: 1 };
    assert!(mapped(&graph, source).is_err());
    let source = "exposure exposure=ignored other=0.5";
    let start = source.find("exposure=").unwrap();
    let graph = StoredGraph {
        entries: vec![
            operation(source, "exposure"),
            GraphEntry::Property {
                name: "exposure".into(),
                value: GraphValue::Literal { source: "0.5".into(), numeric_bits: Some(0.5f64.to_bits()) },
                start: start as u64,
                end: source.len() as u64,
            },
        ],
        nodes: 1,
        edges: 1,
    };
    assert!(mapped(&graph, source).is_err());
}

#[test]
fn identity_input_and_numeric_aliases_are_explicit() {
    let result = mapped(&parsed("id=123"), "id=123").unwrap();
    assert!(result.nodes.is_empty());
    assert!(result.edges.is_empty());
    assert_eq!(result.output, result.input.unwrap());
    let source = "id=123 over aux=[ref=123 exposure]";
    assert_eq!(mapped(&parsed(source), source).unwrap().nodes.len(), 2);
}
#[test]
fn invalid_resolved_parameter_shapes_reject_before_publication() {
    let source = "exposure exposure=0.5";
    let graph = parsed(source);
    let mut deep = FilterValue::Bool(true);
    for _ in 0..25 {
        deep = FilterValue::Config(vec![FilterArgument { name: "nested".into(), value: deep }]);
    }
    let values = vec![
        FilterValue::Resource(photocraft_doc::item_identity::ResourceId(0)),
        FilterValue::Number(ExactNumber::Signed { width: 8, value: 128 }),
        FilterValue::Enum { domain: String::new(), value: 0 },
        FilterValue::Curve { free: true, points_f64_bits: vec![[0, 0]], point_types: vec![0], samples_f64_bits: vec![] },
        deep,
    ];
    for value in values {
        let mut id = 0;
        let result = map_graph(
            &graph,
            source,
            GraphMappingLimits::default(),
            &Interrupt::NONE,
            &mut || {
                id += 1;
                GraphNodeId(id)
            },
            &mut |name, _| {
                Ok(ResolvedOperation {
                    operation: OperationIdentity { name: format!("gegl:{name}"), stored_version: "synthetic-v1".into(), schema_epoch: "synthetic-test".into() },
                    arguments: vec![FilterArgument { name: "exposure".into(), value: value.clone() }],
                })
            },
            &mut |_| Ok(()),
        );
        assert!(result.is_err());
    }
}
#[test]
fn native_graph_preserves_external_input_resolved_version_origin_and_exact_bits() {
    use photocraft_doc::{
        ColorMode, PixelFormat, SampleType,
        item_identity::{FilterId, ItemRef},
        xcf_filters::{FilterNode, FilterStack},
        xcf_state::{ItemState, XcfState},
    };
    let source = "exposure exposure=-0.0";
    let graph = mapped(&parsed(source), source).unwrap();
    let layer = photocraft_doc::Layer::raster("Own graph", PixelFormat::RGBA8);
    let owner = ItemRef::Layer(layer.id);
    let mut doc = photocraft_doc::Document::new("Own", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(layer);
    doc.xcf_state = Some(XcfState {
        items: vec![ItemState {
            item: owner,
            name: "Own graph".into(),
            custom_name: true,
            visible: true,
            locks: Default::default(),
            parent: None,
            source_tattoo: Some(2),
            source_flags: 0,
            raster_bounds: None,
            mask: None,
            composition: None,
        }],
        filter_stacks: vec![FilterStack {
            owner,
            nodes: vec![FilterNode {
                id: FilterId(77),
                operation: OperationIdentity {
                    name: "gegl:gegl".into(),
                    stored_version: "synthetic-envelope-v1".into(),
                    schema_epoch: "synthetic-test".into(),
                },
                source_display_name: None,
                source_icon: None,
                arguments: vec![FilterArgument { name: "string".into(), value: FilterValue::Graph(graph.clone()) }],
                enabled: true,
                opacity_f64_bits: 1.0f64.to_bits(),
                source_clipping: Some(0),
                source_region: Some(0),
                mask: None,
                auxiliary: vec![],
                composition: None,
            }],
            revision: 0,
        }],
        ..Default::default()
    });
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
    let FilterValue::Graph(saved) = &loaded.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments[0].value else { panic!("graph") };
    assert_eq!(saved, &graph);
    assert_eq!(saved.nodes[0].source_version(), None);
    assert_eq!(saved.nodes[0].resolved_version(), "synthetic-v1");
}
