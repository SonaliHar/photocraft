//! Directed-hue graph regression drafts, execution deferred.
use super::super::super::point_color_rotate::HueArc;
use super::*;
use photocraft_doc::xcf_filters::{FilterArgument, FilterGraph, GraphEdge, GraphNode};
use photocraft_raster::Surface;
fn operation() -> ColorRotate {
    ColorRotate {
        source: HueArc { from_degrees: 15., to_degrees: 120., clockwise: false },
        destination: HueArc { from_degrees: 30., to_degrees: 180., clockwise: true },
        gray_threshold: 0.5,
        gray_mode: GrayMode::TreatAs,
        gray_hue_degrees: 45.,
        gray_saturation: 0.75,
    }
}
fn domain() -> ExecutionDomain {
    ExecutionDomain { format: photocraft_color::PixelFormat::RGBA32F, encoding: SampleEncoding::Perceptual, profile_fingerprint: Some([7; 32]) }
}
fn node() -> GraphNode {
    let op = operation();
    let values = [
        ("src-clockwise", FilterValue::Bool(op.source.clockwise)),
        ("src-from", FilterValue::Number(ExactNumber::Float32Bits(op.source.from_degrees.to_bits()))),
        ("src-to", FilterValue::Number(ExactNumber::Float32Bits(op.source.to_degrees.to_bits()))),
        ("dest-clockwise", FilterValue::Bool(op.destination.clockwise)),
        ("dest-from", FilterValue::Number(ExactNumber::Float32Bits(op.destination.from_degrees.to_bits()))),
        ("dest-to", FilterValue::Number(ExactNumber::Float32Bits(op.destination.to_degrees.to_bits()))),
        ("threshold", FilterValue::Number(ExactNumber::Float32Bits(op.gray_threshold.to_bits()))),
        ("gray-mode", FilterValue::Enum { domain: "gegl:color-rotate/gray-mode".into(), value: 0 }),
        ("hue", FilterValue::Number(ExactNumber::Float32Bits(op.gray_hue_degrees.to_bits()))),
        ("saturation", FilterValue::Number(ExactNumber::Float32Bits(op.gray_saturation.to_bits()))),
    ];
    GraphNode {
        id: GraphNodeId(1),
        operation: OperationIdentity { name: "gegl:color-rotate".into(), stored_version: "0:0".into(), schema_epoch: "caller-color-rotate-draft".into() },
        version_origin: GraphVersionOrigin::Stored,
        arguments: values.into_iter().map(|(name, value)| FilterArgument { name: name.into(), value }).collect(),
    }
}
fn binding<'a>(node: &'a GraphNode, op: ColorRotate, domain: ExecutionDomain) -> Binding<'a> {
    Binding {
        node: node.id,
        identity: &node.operation,
        version_origin: node.version_origin,
        domain,
        full_extent: Rect::new(0, 0, 1, 1),
        operation: PointOperation::ColorRotate { operation: op },
    }
}
#[test]
fn exact_angles_and_enum_domain_cannot_be_normalized_or_aliased() {
    let mut node = node();
    let before = node.clone();
    assert!(validate_operation(&binding(&node, operation(), domain()), &node).is_ok());
    node.arguments[1].value = FilterValue::Number(ExactNumber::Float64Bits(15f64.to_bits()));
    assert!(validate_operation(&binding(&node, operation(), domain()), &node).is_err());
    node = before.clone();
    node.arguments[7].value = FilterValue::Enum { domain: "alias".into(), value: 0 };
    assert!(validate_operation(&binding(&node, operation(), domain()), &node).is_err());
    node = before.clone();
    node.arguments[9].name = "threshold".into();
    assert!(validate_operation(&binding(&node, operation(), domain()), &node).is_err());
    node = before;
    let mut singular = operation();
    singular.source.to_degrees = singular.source.from_degrees;
    assert!(validate_operation(&binding(&node, singular, domain()), &node).is_err());
    for d in [
        ExecutionDomain { profile_fingerprint: None, ..domain() },
        ExecutionDomain { encoding: SampleEncoding::NonlinearProfile, ..domain() },
        ExecutionDomain { format: photocraft_color::PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), ..domain() },
    ] {
        assert!(validate_operation(&binding(&node, operation(), d), &node).is_err());
    }
}
#[test]
fn actual_graph_matches_existing_kernel_keeps_alpha_and_source_on_denial() {
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
    let rgba = [0.9f32, 0.4, 0.2, -0.0];
    let raw: Vec<_> = rgba.into_iter().flat_map(f32::to_ne_bytes).collect();
    let source = Surface::try_from_interleaved(domain().format, roi, &raw).unwrap();
    let before = source.clone();
    let plan = plan_graph(&graph, 1, &Interrupt::NONE, |_, direction, port| accepts_port(direction, port)).unwrap();
    let mut expected = [0f32; 4];
    operation().try_eval_perceptual_float(&rgba, &mut expected).unwrap();
    let expected: Vec<_> = expected.into_iter().flat_map(f32::to_ne_bytes).collect();
    let output = execute_graph(&plan, Some(&source), roi, domain(), ExecutionLimits::default(), &Interrupt::NONE, |request, budget| {
        let bound = binding(request.node, operation(), domain());
        evaluate(bound, request, budget)
    })
    .unwrap();
    let actual = output.surface().try_read_interleaved(roi).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(&actual[12..], &raw[12..]);
    let denied = execute_graph(
        &plan,
        Some(&source),
        roi,
        domain(),
        ExecutionLimits { work_units: 1, ..ExecutionLimits::default() },
        &Interrupt::NONE,
        |request, budget| {
            let bound = binding(request.node, operation(), domain());
            evaluate(bound, request, budget)
        },
    );
    assert!(denied.is_err());
    assert_eq!(source, before);
}
