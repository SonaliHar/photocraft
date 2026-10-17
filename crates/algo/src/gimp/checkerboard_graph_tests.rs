// Draft controls only; UNRUN in this implementation cohort.
use super::super::{
    generator_checkerboard::{Argument, Checkerboard},
    graph_execute::ExecutionLimits,
    graph_pipeline::execute_pipeline,
};
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    SampleEncoding,
    item_identity::GraphNodeId,
    xcf_filters::{ExactColorStorage, FilterArgument, FilterGraph, GraphVersionOrigin, OperationIdentity},
};
fn format() -> PixelFormat {
    PixelFormat::new(ColorMode::Rgb, SampleType::F64, true)
}
fn context() -> WorkingContext {
    WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [9; 32] }
}
fn domain() -> ExecutionDomain {
    ExecutionDomain { format: format(), encoding: context().encoding, profile_fingerprint: Some(context().profile_identity) }
}
fn colors() -> ([u8; 32], [u8; 32]) {
    let mut a = [0; 32];
    let mut b = [0; 32];
    for (out, words) in [
        (&mut a, [(-0f64).to_bits(), 0.25f64.to_bits(), 0.5f64.to_bits(), 1f64.to_bits()]),
        (&mut b, [1f64.to_bits(), 0.5f64.to_bits(), 0.25f64.to_bits(), 1f64.to_bits()]),
    ] {
        for (chunk, word) in out.as_chunks_mut::<8>().0.iter_mut().zip(words) {
            chunk.copy_from_slice(&word.to_ne_bytes());
        }
    }
    (a, b)
}
fn configuration() -> Checkerboard {
    let (a, b) = colors();
    Checkerboard::from_named(
        "gegl:checkerboard",
        "0:0",
        format(),
        context().encoding,
        &[
            Argument::Integer { name: "x", value: 16 },
            Argument::Integer { name: "y", value: 16 },
            Argument::Integer { name: "x-offset", value: 0 },
            Argument::Integer { name: "y-offset", value: 0 },
            Argument::Color { name: "color1", encoding: context().encoding, pixel: &a },
            Argument::Color { name: "color2", encoding: context().encoding, pixel: &b },
        ],
    )
    .unwrap()
}
fn fixture_graph() -> FilterGraph {
    let (a, b) = colors();
    let mut arguments: Vec<_> = [("x", 16), ("y", 16), ("x-offset", 0), ("y-offset", 0)]
        .into_iter()
        .map(|(name, value)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value }) })
        .collect();
    for (name, bytes) in [("color1", a), ("color2", b)] {
        arguments.push(FilterArgument {
            name: name.into(),
            value: FilterValue::StoredColor(ExactColorStorage { layout: "own-native-rgba64".into(), samples: bytes.to_vec(), opaque_tail: vec![] }),
        });
    }
    FilterGraph {
        input: None,
        nodes: vec![GraphNode {
            id: GraphNodeId(1),
            operation: OperationIdentity { name: "gegl:checkerboard".into(), stored_version: "0:0".into(), schema_epoch: "own-explicit-test".into() },
            version_origin: GraphVersionOrigin::Stored,
            arguments,
        }],
        edges: vec![],
        output: GraphNodeId(1),
        original_pipeline: None,
    }
}
// This fixture establishes its own native byte contract, not a source parser.
fn validate(node: &GraphNode, c: &Checkerboard) -> Result<()> {
    if node.arguments.len() != 6
        || c.cell_width() != 16
        || c.cell_height() != 16
        || c.offsets() != [0, 0]
        || c.color1_bytes() != colors().0
        || c.color2_bytes() != colors().1
    {
        return Err(E::Operation("own exact fixture"));
    }
    for (a, (name, value)) in node.arguments.iter().take(4).zip([("x", 16), ("y", 16), ("x-offset", 0), ("y-offset", 0)]) {
        if a.name != name || a.value != FilterValue::Number(ExactNumber::Signed { width: 32, value }) {
            return Err(E::InvalidPlan);
        }
    }
    for (a, (name, bytes)) in node.arguments.iter().skip(4).zip([("color1", colors().0), ("color2", colors().1)]) {
        let FilterValue::StoredColor(v) = &a.value else { return Err(E::InvalidPlan) };
        if a.name != name || v.layout != "own-native-rgba64" || v.samples != bytes || !v.opaque_tail.is_empty() {
            return Err(E::InvalidPlan);
        }
    }
    Ok(())
}
#[test]
fn partial_graph_absolute_origin_matches_kernel_and_retains_raw_zero_default() {
    let config = configuration();
    let graph = fixture_graph();
    let full = Rect::new(0, 0, 32, 2);
    let prepared = PreparedCheckerboard::prepare(&graph.nodes[0], &config, full, context(), domain(), &validate, Limits::default(), &Interrupt::NONE).unwrap();
    let q = kernel::requirements(&config, full).unwrap();
    let whole =
        kernel::try_generate(&config, full, kernel::Limits { resident_bytes: q.peak_resident_bytes, work_units: q.work_units }, &Interrupt::NONE).unwrap();
    let before = prepared.remaining_work();
    for roi in [Rect::new(0, 0, 16, 2), Rect::new(16, 0, 32, 2)] {
        let part = execute_pipeline(
            &graph,
            1,
            None,
            roi,
            domain(),
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, d, p| accepts_port(d, p),
            |r, b| evaluate(&prepared, r, b),
        )
        .unwrap();
        assert_eq!(part.surface().try_read_interleaved(roi).unwrap(), whole.try_read_interleaved(roi).unwrap());
        assert_eq!(part.surface().default_bytes(), &[0; 32]);
        assert_eq!(part.surface().try_read_interleaved(Rect::new(0, 3, 1, 4)).unwrap(), vec![0; 32]);
    }
    assert!(prepared.remaining_work() < before);
    assert_eq!(&whole.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap()[..8], &(-0f64).to_ne_bytes());
}
#[test]
fn exact_node_nonzero_domain_and_closed_kernel_limits() {
    let config = configuration();
    let graph = fixture_graph();
    let full = Rect::new(0, 0, 1, 1);
    let zero = ExecutionDomain { profile_fingerprint: Some([0; 32]), ..domain() };
    assert!(PreparedCheckerboard::prepare(&graph.nodes[0], &config, full, context(), zero, &validate, Limits::default(), &Interrupt::NONE).is_err());
    assert!(kernel::requirements(&config, Rect::new(-1, 0, 1, 1)).is_err());
    let q = kernel::requirements(&config, full).unwrap();
    assert!(q.work_units >= 2 * 65536 * 32);
    assert!(
        kernel::try_generate(&config, full, kernel::Limits { resident_bytes: q.peak_resident_bytes, work_units: q.work_units - 1 }, &Interrupt::NONE).is_err()
    );
    let prepared = PreparedCheckerboard::prepare(&graph.nodes[0], &config, full, context(), domain(), &validate, Limits::default(), &Interrupt::NONE).unwrap();
    let equal_but_other = fixture_graph();
    assert!(
        execute_pipeline(
            &equal_but_other,
            1,
            None,
            full,
            domain(),
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, d, p| accepts_port(d, p),
            |r, b| evaluate(&prepared, r, b)
        )
        .is_err()
    );
}
#[test]
fn cancellation_after_attempt_debit_does_not_reset_prepared_quota() {
    let config = configuration();
    let graph = fixture_graph();
    let full = Rect::new(0, 0, 1, 1);
    let armed = std::sync::atomic::AtomicBool::new(false);
    let validator = |n: &GraphNode, c: &Checkerboard| {
        validate(n, c)?;
        armed.store(true, Ordering::Relaxed);
        Ok(())
    };
    let prepared = PreparedCheckerboard::prepare(&graph.nodes[0], &config, full, context(), domain(), &validator, Limits::default(), &Interrupt::NONE).unwrap();
    armed.store(false, Ordering::Relaxed);
    let initial = prepared.remaining_work();
    let cancel = || armed.load(Ordering::Relaxed);
    assert!(
        execute_pipeline(
            &graph,
            1,
            None,
            full,
            domain(),
            ExecutionLimits::default(),
            &Interrupt::cancel_only(&cancel),
            |_, d, p| accepts_port(d, p),
            |r, b| evaluate(&prepared, r, b)
        )
        .is_err()
    );
    let after_cancel = prepared.remaining_work();
    assert!(after_cancel < initial);
    execute_pipeline(
        &graph,
        1,
        None,
        full,
        domain(),
        ExecutionLimits::default(),
        &Interrupt::NONE,
        |_, d, p| accepts_port(d, p),
        |r, b| evaluate(&prepared, r, b),
    )
    .unwrap();
    assert!(prepared.remaining_work() < after_cancel); // Fresh scheduler limits do not reset it.
}
