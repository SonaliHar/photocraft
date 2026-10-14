//! Synthetic native arithmetic drafts, UNRUN; no source/engine qualification.
use super::*;
fn f64_surface(default: f64, roi: Rect, values: &[u64]) -> Surface {
    let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
    let mut s = Surface::try_with_default_bytes(format, &default.to_bits().to_ne_bytes()).unwrap();
    if !roi.is_empty() {
        s.try_write_interleaved_with(roi, &values.iter().flat_map(|v| v.to_ne_bytes()).collect::<Vec<_>>(), &Interrupt::NONE).unwrap();
    }
    s
}
fn run<'a>(a: &'a Surface, b: &'a Surface, roi: Rect, operation: Operation) -> Surface {
    try_combine(Request { a, b, roi, operation, coverage: CoveragePolicy::FiniteUnit }, Limits::default(), &Interrupt::NONE).unwrap()
}
#[test]
fn minimum_maximum_preserve_low_words_and_left_signed_zero_ties() {
    let roi = Rect::new(0, 0, 2, 1);
    let low = 0.5f64.to_bits() + 1;
    let a = f64_surface(0., roi, &[low, (-0f64).to_bits()]);
    let b = f64_surface(0., roi, &[0.5f64.to_bits(), 0]);
    assert_eq!(
        run(&a, &b, roi, Operation::Maximum).try_read_interleaved(roi).unwrap(),
        [low, (-0f64).to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()
    );
    assert_eq!(
        run(&a, &b, roi, Operation::Minimum).try_read_interleaved(roi).unwrap(),
        [0.5f64.to_bits(), (-0f64).to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()
    );
    for op in [Operation::SubtractCoverage, Operation::SubtractClamped] {
        let zeros = f64_surface(0., roi, &[0, (-0f64).to_bits()]);
        assert_eq!(run(&a, &zeros, roi, op).try_read_interleaved(roi).unwrap(), a.try_read_interleaved(roi).unwrap());
    }
}
#[test]
fn coverage_subtraction_is_distinct_and_u32_uses_native_nearest_product() {
    let roi = Rect::new(0, 0, 1, 1);
    let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::U32, alpha: false };
    let a = Surface::try_with_default_bytes(format, &3u32.to_ne_bytes()).unwrap();
    let b = Surface::try_with_default_bytes(format, &(u32::MAX / 2).to_ne_bytes()).unwrap();
    assert_eq!(run(&a, &b, roi, Operation::SubtractClamped).try_read_interleaved(roi).unwrap(), 0u32.to_ne_bytes());
    assert_eq!(run(&a, &b, roi, Operation::SubtractCoverage).try_read_interleaved(roi).unwrap(), 2u32.to_ne_bytes());
    let a = f64_surface(0.75, Rect::EMPTY, &[]);
    let b = f64_surface(0.5, Rect::EMPTY, &[]);
    assert_eq!(run(&a, &b, roi, Operation::SubtractClamped).try_read_interleaved(roi).unwrap(), 0.25f64.to_ne_bytes());
    assert_eq!(run(&a, &b, roi, Operation::SubtractCoverage).try_read_interleaved(roi).unwrap(), 0.375f64.to_ne_bytes());
}
#[test]
fn defaults_are_used_inside_negative_roi_and_unselected_raw_state_is_preserved() {
    let roi = Rect::new(-1, -1, 1, 1);
    let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
    let outside = 0x7ff8_0000_0000_0042u64;
    let mut a = Surface::try_with_default_bytes(format, &outside.to_ne_bytes()).unwrap();
    let words = [0.25f64.to_bits(); 4];
    a.try_write_interleaved_with(roi, &words.into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>(), &Interrupt::NONE).unwrap();
    let b = f64_surface(0.5, Rect::EMPTY, &[]);
    let before = a.clone();
    let output = run(&a, &b, roi, Operation::Maximum);
    assert_eq!(output.try_read_interleaved(roi).unwrap(), [0.5f64.to_bits(); 4].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>());
    assert_eq!(output.default_bytes(), outside.to_ne_bytes());
    assert_eq!(output.try_read_interleaved(Rect::new(4, 4, 5, 5)).unwrap(), outside.to_ne_bytes());
    assert_eq!(a, before);
}
#[test]
fn selected_nonfinite_outside_unit_and_hostile_roi_refuse_without_output() {
    let roi = Rect::new(0, 0, 1, 1);
    let a = f64_surface(0.5, Rect::EMPTY, &[]);
    for (value, error) in [(f64::NAN, Error::NonFiniteCoverage), (1.0000000001, Error::OutsideUnitCoverage)] {
        let b = f64_surface(value, Rect::EMPTY, &[]);
        assert_eq!(
            try_combine(
                Request { a: &a, b: &b, roi, operation: Operation::Maximum, coverage: CoveragePolicy::FiniteUnit },
                Limits::default(),
                &Interrupt::NONE
            )
            .unwrap_err(),
            error
        );
    }
    let before = a.clone();
    assert!(
        try_combine(
            Request { a: &a, b: &a, roi: Rect::new(i32::MIN, 0, 1, 1), operation: Operation::Maximum, coverage: CoveragePolicy::FiniteUnit },
            Limits::default(),
            &Interrupt::NONE
        )
        .is_err()
    );
    assert_eq!(a, before);
}
#[test]
fn complete_requirements_and_cancellation_precede_private_publication() {
    let roi = Rect::new(0, 0, 1, 1);
    let a = f64_surface(0.5, Rect::EMPTY, &[]);
    let b = f64_surface(0.25, Rect::EMPTY, &[]);
    let r = Request { a: &a, b: &b, roi, operation: Operation::SubtractCoverage, coverage: CoveragePolicy::FiniteUnit };
    let needs = requirements(r, Limits::default(), &Interrupt::NONE).unwrap();
    assert!(needs.work_units >= 2 * 65536 * 8);
    assert!(needs.source_retained_bytes >= 2 * std::mem::size_of::<Surface>() as u64 + 16);
    for limits in
        [Limits { work_units: needs.work_units - 1, ..Limits::default() }, Limits { resident_bytes: needs.peak_resident_bytes - 1, ..Limits::default() }]
    {
        assert_eq!(try_combine(r, limits, &Interrupt::NONE).unwrap_err(), Error::Raster(RasterInputError::LimitExceeded));
    }
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 7;
    assert_eq!(try_combine(r, Limits::default(), &Interrupt::cancel_only(&cancelled)).unwrap_err(), Error::Raster(RasterInputError::Cancelled));
    assert_eq!(a.default_bytes(), 0.5f64.to_ne_bytes());
}
