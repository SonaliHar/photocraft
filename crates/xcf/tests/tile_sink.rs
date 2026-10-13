mod common;
use photocraft_xcf::*;

#[derive(Default)]
struct Recording {
    collector: CollectingPlaneSink,
    descriptors: Vec<PlaneDescriptor>,
    tiles: Vec<(u32, u32, u32, u32)>,
    aborted: usize,
    fail_tile: Option<usize>,
}
impl PlaneSink for Recording {
    type Plane = Plane;
    fn begin(&mut self, d: &PlaneDescriptor, c: &mut PlaneSinkControl<'_, '_, '_>) -> Result<(), XcfError> {
        c.charge_retained(1024)?;
        self.descriptors.push(d.clone());
        self.collector.begin(d, c)
    }
    fn tile(&mut self, x: u32, y: u32, w: u32, h: u32, bytes: &[u8], c: &mut PlaneSinkControl<'_, '_, '_>) -> Result<(), XcfError> {
        if self.fail_tile == Some(self.tiles.len()) {
            return Err(XcfError { kind: ErrorKind::Allocation, offset: 123, item: "sink".into(), message: "injected sink failure".into() });
        }
        self.tiles.push((x, y, w, h));
        self.collector.tile(x, y, w, h, bytes, c)
    }
    fn finish(&mut self, c: &mut PlaneSinkControl<'_, '_, '_>) -> Result<Plane, XcfError> {
        self.collector.finish(c)
    }
    fn abort(&mut self) {
        self.aborted += 1;
        self.collector.abort();
    }
}
#[test]
fn native_delivery_matches_eager_precisions_and_transports_with_grid_edges() {
    for (precision, pixel) in [
        (150, vec![1, 2, 3, 255]),
        (250, [1u16, 2, 3, u16::MAX].into_iter().flat_map(u16::to_be_bytes).collect()),
        (350, [1u32, 2, 3, u32::MAX].into_iter().flat_map(u32::to_be_bytes).collect()),
        (550, [0x3400u16, 0x8000, 0x3800, 0x3c00].into_iter().flat_map(u16::to_be_bytes).collect()),
        (650, [0.25f32, -0.0, 0.75, 1.0].into_iter().flat_map(f32::to_be_bytes).collect()),
        (750, [f64::from_bits(0x3fd0000000000001), -0.0, 0.75, 1.0].into_iter().flat_map(f64::to_be_bytes).collect()),
    ] {
        for compression in 0..=2 {
            let bytes = common::image(22, precision, compression, 65, 66, &pixel.repeat(65 * 66));
            let eager = decode(&bytes, &DecodeOptions::default()).unwrap();
            let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
            let mut sink = Recording::default();
            let result = decode_plain_source_into_sink_with_budget(
                &mut ByteSource::new(&bytes),
                &DecodeOptions::default(),
                &budget,
                PlainSourceAccounting::BorrowedMemory,
                &mut sink,
            )
            .unwrap();
            assert_eq!(result.decoded.file.layers[0].plane.as_ref().unwrap().bytes, eager.layers[0].plane.as_ref().unwrap().bytes);
            assert_eq!(sink.tiles, [(0, 0, 64, 64), (64, 0, 1, 64), (0, 64, 64, 2), (64, 64, 1, 2)]);
            assert_eq!(sink.descriptors[0].role, PlaneRole::Raster);
            assert_eq!(sink.descriptors[0].sample, if precision == 550 { Sample::F32 } else { eager.sample });
            assert_eq!(sink.aborted, 0);
            drop(result);
            assert_eq!(budget.used(BudgetKind::Resident), 0);
        }
    }
}
#[test]
fn invalid_native_or_coverage_never_reaches_tile_callback() {
    for value in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
        let bytes = common::image(22, 750, 0, 1, 1, &[0.2f64, 0.3, 0.4, value].into_iter().flat_map(f64::to_be_bytes).collect::<Vec<_>>());
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
        let mut sink = Recording::default();
        let error = decode_plain_source_into_sink_with_budget(
            &mut ByteSource::new(&bytes),
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::BorrowedMemory,
            &mut sink,
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unsupported);
        assert!(sink.tiles.is_empty());
        assert_eq!(sink.aborted, 1);
        assert_eq!(budget.used(BudgetKind::Resident), 0);
    }
}
#[test]
fn finite_drawable_override_is_explicit_and_preserves_bits() {
    let native: Vec<u8> = [0.2f64, -0.0, 0.4, 1.01].into_iter().flat_map(f64::to_ne_bytes).collect();
    let bytes = common::image(22, 750, 0, 1, 1, &[0.2f64, -0.0, 0.4, 1.01].into_iter().flat_map(f64::to_be_bytes).collect::<Vec<_>>());
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
    let mut sink = Recording::default();
    let result = decode_plain_source_into_sink_with_budget(
        &mut ByteSource::new(&bytes),
        &DecodeOptions { allow_finite_drawable_alpha: true, ..Default::default() },
        &budget,
        PlainSourceAccounting::BorrowedMemory,
        &mut sink,
    )
    .unwrap();
    assert_eq!(result.decoded.file.layers[0].plane.as_ref().unwrap().bytes, native);
}
#[test]
fn late_callback_error_discards_staging_and_keeps_original_error() {
    let bytes = common::image(22, 150, 0, 65, 1, &[7, 8, 9, 255].repeat(65));
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
    let mut sink = Recording { fail_tile: Some(1), ..Default::default() };
    let error = decode_plain_source_into_sink_with_budget(
        &mut ByteSource::new(&bytes),
        &DecodeOptions::default(),
        &budget,
        PlainSourceAccounting::BorrowedMemory,
        &mut sink,
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Allocation);
    assert_eq!(error.offset, 123);
    assert_eq!(error.message, "injected sink failure");
    assert_eq!(sink.tiles.len(), 1);
    assert_eq!(sink.aborted, 1);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
    // Cleanup reset the collector, so a new transaction can safely begin.
    sink.tiles.clear();
    sink.fail_tile = None;
    assert!(
        decode_plain_source_into_sink_with_budget(
            &mut ByteSource::new(&bytes),
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::BorrowedMemory,
            &mut sink
        )
        .is_ok()
    );
}

fn append_mask(bytes: &mut Vec<u8>, value: f64) {
    let layer = u64::from_be_bytes(bytes[47..55].try_into().unwrap()) as usize;
    let mask = bytes.len();
    common::patch(bytes, layer + 34, mask, 22); // fixture name X and empty properties
    common::u32be(bytes, 1);
    common::u32be(bytes, 1);
    common::u32be(bytes, 2);
    bytes.extend_from_slice(b"M\0");
    common::prop(bytes, 0, &[]);
    common::pointer(bytes, bytes.len() + 8, 22);
    common::u32be(bytes, 1);
    common::u32be(bytes, 1);
    common::u32be(bytes, 8);
    common::pointer(bytes, bytes.len() + 16, 22);
    common::pointer(bytes, 0, 22);
    common::u32be(bytes, 1);
    common::u32be(bytes, 1);
    common::pointer(bytes, bytes.len() + 16, 22);
    common::pointer(bytes, 0, 22);
    bytes.extend_from_slice(&value.to_be_bytes());
}
#[test]
fn mask_role_remains_unit_even_with_drawable_override() {
    for value in [0.5f64, -0.1, 1.1] {
        let mut bytes = common::image_custom(
            22,
            750,
            0,
            1,
            1,
            &[0.0f64, 0.0, 0.0, 1.1].into_iter().flat_map(f64::to_be_bytes).collect::<Vec<_>>(),
            &common::FixtureOptions { name: Some("X".into()), ..Default::default() },
        );
        append_mask(&mut bytes, value);
        let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
        let mut sink = Recording::default();
        let result = decode_plain_source_into_sink_with_budget(
            &mut ByteSource::new(&bytes),
            &DecodeOptions { allow_finite_drawable_alpha: true, ..Default::default() },
            &budget,
            PlainSourceAccounting::BorrowedMemory,
            &mut sink,
        );
        assert_eq!(sink.descriptors[1].role, PlaneRole::Mask);
        if value == 0.5 {
            assert!(result.is_ok());
            assert_eq!(sink.tiles.len(), 2);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Unsupported);
            assert_eq!(sink.tiles.len(), 1);
            assert_eq!(sink.aborted, 1);
        }
    }
}

#[test]
fn resident_exact_and_one_under_enforce_actual_collector_peak() {
    let bytes = common::image(22, 550, 0, 65, 1, &[0x3800u16, 0x3400, 0x8000, 0x3c00].into_iter().flat_map(u16::to_be_bytes).collect::<Vec<_>>().repeat(65));
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    let budget = Budget::new(limits.clone(), None);
    let result = decode_plain_source_into_sink_with_budget(
        &mut ByteSource::new(&bytes),
        &DecodeOptions::default(),
        &budget,
        PlainSourceAccounting::BorrowedMemory,
        &mut CollectingPlaneSink::default(),
    )
    .unwrap();
    let peak = budget.peak(BudgetKind::Resident);
    drop(result);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    for (cap, succeeds) in [(peak, true), (peak - 1, false)] {
        let budget = Budget::new(limits.lowered(&[("residentBytes", cap)]).unwrap(), None);
        let result = decode_plain_source_into_sink_with_budget(
            &mut ByteSource::new(&bytes),
            &DecodeOptions::default(),
            &budget,
            PlainSourceAccounting::BorrowedMemory,
            &mut CollectingPlaneSink::default(),
        );
        assert_eq!(result.is_ok(), succeeds);
        if !succeeds {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Limit);
        } else {
            drop(result);
        }
        assert_eq!(budget.used(BudgetKind::Resident), 0);
        assert_eq!(budget.used(BudgetKind::Encoded), 0);
    }
}

struct StopDuringCopy<'a> {
    inner: Recording,
    stop: &'a std::sync::atomic::AtomicBool,
}
impl PlaneSink for StopDuringCopy<'_> {
    type Plane = Plane;
    fn begin(&mut self, d: &PlaneDescriptor, c: &mut PlaneSinkControl<'_, '_, '_>) -> Result<(), XcfError> {
        self.inner.begin(d, c)
    }
    fn tile(&mut self, x: u32, y: u32, w: u32, h: u32, b: &[u8], c: &mut PlaneSinkControl<'_, '_, '_>) -> Result<(), XcfError> {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.inner.tile(x, y, w, h, b, c)
    }
    fn finish(&mut self, c: &mut PlaneSinkControl<'_, '_, '_>) -> Result<Plane, XcfError> {
        self.inner.finish(c)
    }
    fn abort(&mut self) {
        self.inner.abort();
    }
}
#[test]
fn cancellation_inside_collecting_copy_aborts_before_handoff_and_refunds() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let bytes = common::image(22, 150, 0, 64, 64, &[1, 2, 3, 255].repeat(64 * 64));
    let stopped = AtomicBool::new(false);
    let cancelled = || stopped.load(Ordering::Relaxed);
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None);
    let mut sink = StopDuringCopy { inner: Recording::default(), stop: &stopped };
    let error = decode_plain_source_into_sink_with_budget(
        &mut ByteSource::new(&bytes),
        &DecodeOptions { cancelled: Some(&cancelled), ..Default::default() },
        &budget,
        PlainSourceAccounting::BorrowedMemory,
        &mut sink,
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert_eq!(sink.inner.aborted, 1);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
}
