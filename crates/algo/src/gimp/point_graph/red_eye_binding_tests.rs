//! Authored graph regressions; execution is deferred.
use super::*;
use photocraft_doc::xcf_filters::{FilterArgument, FilterGraph, GraphEdge, GraphNode};
use photocraft_raster::Surface;

fn domain() -> ExecutionDomain {
    ExecutionDomain { format: photocraft_color::PixelFormat::RGBA32F, encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: Some([3; 32]) }
}
fn node() -> GraphNode {
    GraphNode {
        id: GraphNodeId(1),
        operation: OperationIdentity {
            name: "gegl:red-eye-removal".into(),
            stored_version: "0:0".into(),
            schema_epoch: "explicit-working-red-eye-draft".into(),
        },
        version_origin: GraphVersionOrigin::Stored,
        arguments: vec![FilterArgument { name: "threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.4f32.to_bits())) }],
    }
}
fn binding<'a>(node: &'a GraphNode, domain: ExecutionDomain) -> Binding<'a> {
    Binding {
        node: node.id,
        identity: &node.operation,
        version_origin: node.version_origin,
        domain,
        full_extent: Rect::new(0, 0, 1, 1),
        operation: PointOperation::RedEyeRemoval { threshold: f64::from(0.4f32) },
    }
}
#[test]
fn original_float_threshold_is_exact_and_domains_are_not_inferred() {
    let mut node = node();
    let before = node.clone();
    assert!(validate_operation(&binding(&node, domain()), &node).is_ok());
    assert_eq!(node, before);
    node.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(0.4f64.to_bits()));
    assert!(validate_operation(&binding(&node, domain()), &node).is_err());
    node = before.clone();
    node.arguments.push(node.arguments[0].clone());
    assert!(validate_operation(&binding(&node, domain()), &node).is_err());
    node = before.clone();
    node.arguments[0].name = "unknown".into();
    assert!(validate_operation(&binding(&node, domain()), &node).is_err());
    node = before;
    for d in [
        ExecutionDomain { profile_fingerprint: None, ..domain() },
        ExecutionDomain { profile_fingerprint: Some([0; 32]), ..domain() },
        ExecutionDomain { encoding: SampleEncoding::Linear, ..domain() },
        ExecutionDomain { format: photocraft_color::PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), ..domain() },
    ] {
        assert!(validate_operation(&binding(&node, d), &node).is_err());
    }
}
#[test]
fn graph_red_eye_uses_rgba_kernel_and_preserves_other_channels_and_source() {
    use super::super::super::{
        graph_execute::{ExecutionLimits, execute_graph},
        graph_plan::plan_graph,
    };
    let node = node();
    let graph = FilterGraph {
        input: Some(GraphNodeId(9)),
        nodes: vec![node],
        edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), input_port: "input".into(), output_port: "output".into() }],
        output: GraphNodeId(1),
        original_pipeline: None,
    };
    let roi = Rect::new(0, 0, 1, 1);
    let rgba = [0.9f32, 0.1, 0.2, -0.0];
    let raw: Vec<_> = rgba.into_iter().flat_map(f32::to_ne_bytes).collect();
    let input = Surface::try_from_interleaved(domain().format, roi, &raw).unwrap();
    let before = input.clone();
    let plan = plan_graph(&graph, 1, &Interrupt::NONE, |_, dir, port| accepts_port(dir, port)).unwrap();
    let mut expected = [0f32; 4];
    super::super::super::point_red_eye::RedEyeRemoval { threshold: f64::from(0.4f32) }.try_eval_nonlinear_float(&rgba, &mut expected).unwrap();
    let expected: Vec<_> = expected.into_iter().flat_map(f32::to_ne_bytes).collect();
    let output = execute_graph(&plan, Some(&input), roi, domain(), ExecutionLimits::default(), &Interrupt::NONE, |request, budget| {
        let bound = binding(request.node, domain());
        evaluate(bound, request, budget)
    })
    .unwrap();
    assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), expected);
    assert_eq!(output.surface().try_read_interleaved(roi).unwrap()[4..], raw[4..]);
    assert_eq!(input, before);
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
