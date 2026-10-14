//! Authored exact-schema and graph regression drafts; execution is deferred.
use super::*;
use photocraft_doc::xcf_filters::{FilterArgument, FilterGraph, GraphEdge, GraphNode};
use photocraft_raster::Surface;

fn domain() -> ExecutionDomain {
    ExecutionDomain { format: photocraft_color::PixelFormat::RGBA32F, encoding: SampleEncoding::Linear, profile_fingerprint: Some([5; 32]) }
}
fn operation(low: bool, high: bool) -> PointOperation {
    PointOperation::AlphaClipFinitePrecision { clip_low: low, low_limit: 0.875, clip_high: high, high_limit: 0.125 }
}
fn node(low: bool, high: bool) -> GraphNode {
    GraphNode {
        id: GraphNodeId(1),
        operation: OperationIdentity { name: "gegl:alpha-clip".into(), stored_version: "0:0".into(), schema_epoch: "caller-finite-alpha-draft".into() },
        version_origin: GraphVersionOrigin::Stored,
        arguments: vec![
            FilterArgument { name: "clip-low".into(), value: FilterValue::Bool(low) },
            FilterArgument { name: "low-limit".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.875f32.to_bits())) },
            FilterArgument { name: "clip-high".into(), value: FilterValue::Bool(high) },
            FilterArgument { name: "high-limit".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.125f32.to_bits())) },
        ],
    }
}
fn binding<'a>(node: &'a GraphNode, operation: PointOperation, domain: ExecutionDomain) -> Binding<'a> {
    Binding { node: node.id, identity: &node.operation, version_origin: node.version_origin, domain, full_extent: Rect::new(0, 0, 1, 1), operation }
}
#[test]
fn finite_crossed_policy_is_explicit_and_boolean_float_bits_are_exact() {
    let mut node = node(true, true);
    let before = node.clone();
    assert!(validate_operation(&binding(&node, operation(true, true), domain()), &node).is_ok());
    let unit = PointOperation::AlphaClipPrecision { clip_low: true, low_limit: 0.875, clip_high: true, high_limit: 0.125 };
    assert!(validate_operation(&binding(&node, unit, domain()), &node).is_err());
    node.arguments[0].value = FilterValue::Number(ExactNumber::Signed { width: 32, value: 1 });
    assert!(validate_operation(&binding(&node, operation(true, true), domain()), &node).is_err());
    node = before.clone();
    node.arguments[1].value = FilterValue::Number(ExactNumber::Float64Bits(0.875f64.to_bits()));
    assert!(validate_operation(&binding(&node, operation(true, true), domain()), &node).is_err());
    node = before.clone();
    node.arguments[3].name = "low-limit".into();
    assert!(validate_operation(&binding(&node, operation(true, true), domain()), &node).is_err());
    node = before;
    for d in [
        ExecutionDomain { profile_fingerprint: None, ..domain() },
        ExecutionDomain { encoding: SampleEncoding::Perceptual, ..domain() },
        ExecutionDomain { format: photocraft_color::PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), ..domain() },
    ] {
        assert!(validate_operation(&binding(&node, operation(true, true), d), &node).is_err());
    }
}
#[test]
fn actual_graph_crossed_precedence_and_neither_clip_keep_rgb_raw_and_source_atomic() {
    use super::super::super::{
        graph_execute::{ExecutionLimits, execute_graph},
        graph_plan::plan_graph,
    };
    let roi = Rect::new(0, 0, 1, 1);
    let raw: Vec<_> = [-0.0f32, 0.5, 1.0, 0.25].into_iter().flat_map(f32::to_ne_bytes).collect();
    let input = Surface::try_from_interleaved(domain().format, roi, &raw).unwrap();
    let before = input.clone();
    for enabled in [true, false] {
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node(enabled, enabled)],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), input_port: "input".into(), output_port: "output".into() }],
            output: GraphNodeId(1),
            original_pipeline: None,
        };
        let plan = plan_graph(&graph, 1, &Interrupt::NONE, |_, dir, port| accepts_port(dir, port)).unwrap();
        let result = execute_graph(&plan, Some(&input), roi, domain(), ExecutionLimits::default(), &Interrupt::NONE, |request, budget| {
            let bound = binding(request.node, operation(enabled, enabled), domain());
            evaluate(bound, request, budget)
        })
        .unwrap();
        let actual = result.surface().try_read_interleaved(roi).unwrap();
        assert_eq!(&actual[..12], &raw[..12]);
        assert_eq!(&actual[12..], &(if enabled { 0.125f32 } else { 0.25f32 }).to_ne_bytes());
        let denied = execute_graph(
            &plan,
            Some(&input),
            roi,
            domain(),
            ExecutionLimits { work_units: 1, ..ExecutionLimits::default() },
            &Interrupt::NONE,
            |request, budget| {
                let bound = binding(request.node, operation(enabled, enabled), domain());
                evaluate(bound, request, budget)
            },
        );
        assert!(denied.is_err());
        assert_eq!(input, before);
    }
}
