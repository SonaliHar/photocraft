//! Authored schema and actual graph regression drafts; execution deferred.
use super::*;
use photocraft_doc::xcf_filters::{FilterArgument, FilterGraph, GraphEdge, GraphNode};
use photocraft_raster::Surface;

const FROM: [f32; 4] = [0.9, 0.1, 0.2, -0.0];
const TO: [f32; 4] = [0.1, 0.7, 0.8, 0.5];
fn domain() -> ExecutionDomain {
    ExecutionDomain { format: photocraft_color::PixelFormat::RGBA32F, encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: Some([4; 32]) }
}
fn operation() -> PointOperation {
    PointOperation::ColorExchange { from: FROM, to: TO, thresholds: [0.; 3] }
}
fn node() -> GraphNode {
    let color = |name: &str, components: [f32; 4]| FilterArgument {
        name: name.into(),
        value: FilterValue::Color {
            components: components.map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
            encoding: SampleEncoding::NonlinearProfile,
            profile: None,
        },
    };
    let mut arguments = vec![color("from-color", FROM), color("to-color", TO)];
    arguments.extend(
        ["red-threshold", "green-threshold", "blue-threshold"]
            .into_iter()
            .map(|name| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(0)) }),
    );
    GraphNode {
        id: GraphNodeId(1),
        operation: OperationIdentity { name: "gegl:color-exchange".into(), stored_version: "0:0".into(), schema_epoch: "caller-color-exchange-draft".into() },
        version_origin: GraphVersionOrigin::Stored,
        arguments,
    }
}
fn binding<'a>(node: &'a GraphNode, domain: ExecutionDomain) -> Binding<'a> {
    Binding {
        node: node.id,
        identity: &node.operation,
        version_origin: node.version_origin,
        domain,
        full_extent: Rect::new(0, 0, 1, 1),
        operation: operation(),
    }
}
#[test]
fn unused_literal_alpha_bits_still_bind_and_types_duplicates_domains_refuse() {
    let mut node = node();
    let original = node.clone();
    assert!(validate_operation(&binding(&node, domain()), &node).is_ok());
    if let FilterValue::Color { components, .. } = &mut node.arguments[0].value {
        components[3] = ExactNumber::Float32Bits(0);
    }
    assert!(validate_operation(&binding(&node, domain()), &node).is_err());
    node = original.clone();
    node.arguments[2].value = FilterValue::Number(ExactNumber::Float64Bits(0));
    assert!(validate_operation(&binding(&node, domain()), &node).is_err());
    node = original.clone();
    node.arguments[4].name = "red-threshold".into();
    assert!(validate_operation(&binding(&node, domain()), &node).is_err());
    node = original;
    for d in [
        ExecutionDomain { profile_fingerprint: None, ..domain() },
        ExecutionDomain { encoding: SampleEncoding::Linear, ..domain() },
        ExecutionDomain { format: photocraft_color::PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), ..domain() },
    ] {
        assert!(validate_operation(&binding(&node, d), &node).is_err());
    }
}
#[test]
fn actual_graph_reuses_exchange_kernel_and_preserves_raw_alpha_and_source_on_denial() {
    use super::super::super::{
        graph_execute::{ExecutionLimits, execute_graph},
        graph_plan::plan_graph,
    };
    let graph = FilterGraph {
        input: Some(GraphNodeId(9)),
        nodes: vec![node()],
        edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), input_port: "input".into(), output_port: "output".into() }],
        output: GraphNodeId(1),
        original_pipeline: None,
    };
    let roi = Rect::new(0, 0, 1, 1);
    let raw: Vec<_> = FROM.into_iter().flat_map(f32::to_ne_bytes).collect();
    let input = Surface::try_from_interleaved(domain().format, roi, &raw).unwrap();
    let before = input.clone();
    let plan = plan_graph(&graph, 1, &Interrupt::NONE, |_, dir, port| accepts_port(dir, port)).unwrap();
    let mut expected = [0f64; 4];
    operation().prepare().unwrap().try_eval_pixel(&FROM.map(f64::from), 3, &mut expected).unwrap();
    let expected: Vec<_> = expected.into_iter().flat_map(|v| (v as f32).to_ne_bytes()).collect();
    let output = execute_graph(&plan, Some(&input), roi, domain(), ExecutionLimits::default(), &Interrupt::NONE, |request, budget| {
        let bound = binding(request.node, domain());
        evaluate(bound, request, budget)
    })
    .unwrap();
    let actual = output.surface().try_read_interleaved(roi).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(&actual[12..], &raw[12..]);
    let denied = execute_graph(
        &plan,
        Some(&input),
        roi,
        domain(),
        ExecutionLimits { work_units: 1, ..ExecutionLimits::default() },
        &Interrupt::NONE,
        |request, budget| {
            let bound = binding(request.node, domain());
            evaluate(bound, request, budget)
        },
    );
    assert!(denied.is_err());
    assert_eq!(input, before);
}
