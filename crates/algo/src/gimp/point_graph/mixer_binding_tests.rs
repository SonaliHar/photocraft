//! Authored regression drafts; execution is deferred by the implementation wave.
use super::*;
use photocraft_doc::xcf_filters::{FilterArgument, FilterGraph, GraphEdge, GraphNode};
use photocraft_raster::Surface;

fn domain() -> ExecutionDomain {
    ExecutionDomain { format: photocraft_color::PixelFormat::RGBA32F, encoding: SampleEncoding::Linear, profile_fingerprint: Some([1; 32]) }
}
fn channel_node() -> GraphNode {
    let mut arguments = Vec::new();
    for (name, value) in [
        ("rr-gain", 1.0),
        ("rg-gain", 0.0),
        ("rb-gain", 0.0),
        ("gr-gain", 0.0),
        ("gg-gain", 1.0),
        ("gb-gain", 0.0),
        ("br-gain", 0.0),
        ("bg-gain", 0.0),
        ("bb-gain", 1.0),
    ] {
        arguments.push(FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(f64::to_bits(value))) });
    }
    arguments.push(FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(true) });
    GraphNode {
        id: GraphNodeId(1),
        operation: OperationIdentity { name: "gegl:channel-mixer".into(), stored_version: "0:0".into(), schema_epoch: "explicit-mixer-draft".into() },
        version_origin: GraphVersionOrigin::Stored,
        arguments,
    }
}
fn binding<'a>(node: &'a GraphNode, domain: ExecutionDomain, operation: PointOperation) -> Binding<'a> {
    Binding { node: node.id, identity: &node.operation, version_origin: node.version_origin, domain, full_extent: Rect::new(0, 0, 1, 1), operation }
}
fn channel_operation() -> PointOperation {
    PointOperation::ChannelMixer { gains: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], preserve_luminosity: true }
}
#[test]
fn ten_retained_arguments_bind_without_mutating_bits_and_refuse_aliases() {
    let mut node = channel_node();
    let before = node.clone();
    assert!(validate_operation(&binding(&node, domain(), channel_operation()), &node).is_ok());
    assert_eq!(node, before);
    // Same approximate value is not an exact Double authority match.
    node.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(1f64.to_bits() + 1));
    assert!(validate_operation(&binding(&node, domain(), channel_operation()), &node).is_err());
    node = before.clone();
    node.arguments[9].name = "rr-gain".into();
    assert!(validate_operation(&binding(&node, domain(), channel_operation()), &node).is_err());
    node = before.clone();
    node.arguments[9].value = FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits()));
    assert!(validate_operation(&binding(&node, domain(), channel_operation()), &node).is_err());
    node = before;
    for rejected in [
        ExecutionDomain { profile_fingerprint: None, ..domain() },
        ExecutionDomain { profile_fingerprint: Some([0; 32]), ..domain() },
        ExecutionDomain { encoding: SampleEncoding::NonlinearProfile, ..domain() },
        ExecutionDomain { format: photocraft_color::PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), ..domain() },
        ExecutionDomain { format: photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true), ..domain() },
    ] {
        assert!(validate_operation(&binding(&node, rejected, channel_operation()), &node).is_err());
    }
}
#[test]
fn signed_mono_preservation_flag_and_float_authority_are_exact() {
    let mut node = channel_node();
    node.operation.name = "gegl:mono-mixer".into();
    node.arguments = [
        ("red", FilterValue::Number(ExactNumber::Float32Bits((-1f32).to_bits()))),
        ("green", FilterValue::Number(ExactNumber::Float64Bits(0.5f64.to_bits()))),
        ("blue", FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()))),
        ("preserve-luminosity", FilterValue::Bool(true)),
    ]
    .into_iter()
    .map(|(name, value)| FilterArgument { name: name.into(), value })
    .collect();
    let op = PointOperation::MonoMixerSigned { red: -1., green: 0.5, blue: 0.25, preserve_luminosity: true };
    assert!(validate_operation(&binding(&node, domain(), op), &node).is_ok());
    node.arguments[3].value = FilterValue::Bool(false);
    assert!(validate_operation(&binding(&node, domain(), op), &node).is_err());
}
#[test]
fn actual_channel_graph_identity_preserves_raw_alpha_and_low_budget_source() {
    use super::super::{
        graph_execute::{ExecutionLimits, execute_graph},
        graph_plan::plan_graph,
    };
    let node = channel_node();
    let graph = FilterGraph {
        input: Some(GraphNodeId(9)),
        nodes: vec![node],
        edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), input_port: "input".into(), output_port: "output".into() }],
        output: GraphNodeId(1),
        original_pipeline: None,
    };
    let roi = Rect::new(0, 0, 1, 1);
    let raw: Vec<_> = [0.25f32, 0.5, 0.75, -0.0].into_iter().flat_map(f32::to_ne_bytes).collect();
    let input = Surface::try_from_interleaved(domain().format, roi, &raw).unwrap();
    let before = input.clone();
    let plan = plan_graph(&graph, 1, &Interrupt::NONE, |_, direction, port| accepts_port(direction, port)).unwrap();
    for limits in [ExecutionLimits::default(), ExecutionLimits { work_units: 1, ..ExecutionLimits::default() }] {
        let result = execute_graph(&plan, Some(&input), roi, domain(), limits, &Interrupt::NONE, |request, budget| {
            let bound = binding(request.node, domain(), channel_operation());
            evaluate(bound, request, budget)
        });
        if limits.work_units == 1 {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap().surface().try_read_interleaved(roi).unwrap(), raw);
        }
        assert_eq!(input, before);
    }
}
