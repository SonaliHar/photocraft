//! Authored Color Overlay graph regressions; no execution in this wave.
use super::*;
use photocraft_doc::xcf_filters::{FilterArgument, FilterGraph, GraphEdge, GraphNode};
use photocraft_raster::Surface;
fn domain() -> ExecutionDomain {
    ExecutionDomain { format: photocraft_color::PixelFormat::RGBA32F, encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: Some([6; 32]) }
}
fn node(color: [f32; 4]) -> GraphNode {
    GraphNode {
        id: GraphNodeId(1),
        operation: OperationIdentity { name: "gegl:color-overlay".into(), stored_version: "0:0".into(), schema_epoch: "explicit-overlay-draft".into() },
        version_origin: GraphVersionOrigin::Stored,
        arguments: vec![
            FilterArgument {
                name: "value".into(),
                value: FilterValue::Color {
                    components: color.map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
                    encoding: SampleEncoding::NonlinearProfile,
                    profile: None,
                },
            },
            FilterArgument { name: "srgb".into(), value: FilterValue::Bool(true) },
        ],
    }
}
fn binding<'a>(node: &'a GraphNode, color: [f32; 4], domain: ExecutionDomain) -> Binding<'a> {
    Binding {
        node: node.id,
        identity: &node.operation,
        version_origin: node.version_origin,
        domain,
        full_extent: Rect::new(0, 0, 1, 1),
        operation: PointOperation::ColorOverlay { color },
    }
}
#[test]
fn literal_opacity_and_encoding_flag_are_exact_not_outer_opacity() {
    let color = [0.9f32, 0.2, 0.5, 0.5];
    let mut node = node(color);
    let before = node.clone();
    assert!(validate_operation(&binding(&node, color, domain()), &node).is_ok());
    node.arguments[1].value = FilterValue::Bool(false);
    assert!(validate_operation(&binding(&node, color, domain()), &node).is_err());
    node = before.clone();
    if let FilterValue::Color { components, .. } = &mut node.arguments[0].value {
        components[3] = ExactNumber::Float32Bits(0.25f32.to_bits());
    }
    assert!(validate_operation(&binding(&node, color, domain()), &node).is_err());
    node = before.clone();
    node.arguments[1].name = "value".into();
    assert!(validate_operation(&binding(&node, color, domain()), &node).is_err());
    node = before;
    for d in [ExecutionDomain { profile_fingerprint: None, ..domain() }, ExecutionDomain { encoding: SampleEncoding::Linear, ..domain() }] {
        assert!(validate_operation(&binding(&node, color, d), &node).is_err());
    }
}
#[test]
fn actual_graph_uses_literal_alpha_but_keeps_input_coverage_and_atomic_source() {
    use super::super::super::{
        graph_execute::{ExecutionLimits, execute_graph},
        graph_plan::plan_graph,
    };
    let roi = Rect::new(0, 0, 1, 1);
    let rgba = [0.1f32, 0.3, 0.8, -0.0];
    let raw: Vec<_> = rgba.into_iter().flat_map(f32::to_ne_bytes).collect();
    let input = Surface::try_from_interleaved(domain().format, roi, &raw).unwrap();
    let before = input.clone();
    for alpha in [0f32, 0.5, 1.0] {
        let color = [0.9f32, 0.2, 0.5, alpha];
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node(color)],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), input_port: "input".into(), output_port: "output".into() }],
            output: GraphNodeId(1),
            original_pipeline: None,
        };
        let plan = plan_graph(&graph, 1, &Interrupt::NONE, |_, dir, port| accepts_port(dir, port)).unwrap();
        let mut expected = [0f64; 4];
        PointOperation::ColorOverlay { color }.prepare().unwrap().try_eval_pixel(&rgba.map(f64::from), 3, &mut expected).unwrap();
        let expected: Vec<_> = expected.into_iter().flat_map(|v| (v as f32).to_ne_bytes()).collect();
        let output = execute_graph(&plan, Some(&input), roi, domain(), ExecutionLimits::default(), &Interrupt::NONE, |request, budget| {
            let bound = binding(request.node, color, domain());
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
                let bound = binding(request.node, color, domain());
                evaluate(bound, request, budget)
            },
        );
        assert!(denied.is_err());
        assert_eq!(input, before);
    }
}
