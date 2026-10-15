//! UNRUN requirements/legacy admission boundary drafts. Input buffers here are
//! harness-owned; reports are not asserted to be aggregate quota receipts.
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};

fn source(sample: SampleType) -> Surface {
    let mut source = Surface::new(PixelFormat { mode: ColorMode::Rgb, sample, alpha: true });
    for (x, pixel) in [[0.25, 0.0, 1.0, 1.0], [0.5, 1.0, 0.0, 0.5], [1.0, 0.25, 0.5, 1.0]].iter().enumerate() {
        source.try_write_pixel_f64(x as i32, 0, pixel).unwrap();
    }
    source.try_write_pixel_f64(200, 200, &[-0.0, 0.75, 1.0, 0.0]).unwrap();
    source
}
fn median(high_precision: bool) -> MedianParams {
    MedianParams { radius: -1, neighborhood: MedianNeighborhood::Square, percentile: 25.0, alpha_percentile: 75.0, high_precision }
}

#[test]
fn reported_box_boundary_admits_exact_old_result_and_refuses_one_unit_less() {
    let roi = Rect::new(0, 0, 3, 1);
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        let source = source(sample);
        let before = source.clone();
        let required = box_blur_requirements(&source, roi, roi, 1, &Interrupt::NONE).unwrap();
        assert!(required.peak_resident_bytes > 0);
        assert!(required.work_units > 0);
        let exact = GaussianLimits { resident_bytes: required.peak_resident_bytes, work_units: required.work_units };
        let actual = try_box_blur(&source, roi, roi, 1, exact, &Interrupt::NONE).unwrap();
        assert_eq!(actual, try_box_blur(&source, roi, roi, 1, Default::default(), &Interrupt::NONE).unwrap());
        for lower in [GaussianLimits { resident_bytes: exact.resident_bytes - 1, ..exact }, GaussianLimits { work_units: exact.work_units - 1, ..exact }] {
            assert_eq!(try_box_blur(&source, roi, roi, 1, lower, &Interrupt::NONE), Err(RasterInputError::LimitExceeded));
        }
        assert_eq!(source, before);
        assert_eq!(actual.default_bytes(), source.default_bytes());
        let untouched = Rect::new(200, 200, 201, 201);
        assert_eq!(actual.try_read_interleaved(untouched).unwrap(), source.try_read_interleaved(untouched).unwrap());
    }
}

#[test]
fn reported_median_boundary_covers_both_modes_without_changing_encoded_or_native_math() {
    let roi = Rect::new(0, 0, 3, 1);
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        let source = source(sample);
        let before = source.clone();
        for high in [false, true] {
            let p = median(high);
            let required = median_requirements(&source, roi, roi, p, &Interrupt::NONE).unwrap();
            let evaluate = if high { try_median_blur } else { try_median_blur_quantized_encoded };
            let limits = GaussianLimits { resident_bytes: required.peak_resident_bytes, work_units: required.work_units };
            assert_eq!(
                evaluate(&source, roi, roi, p, limits, &Interrupt::NONE).unwrap(),
                evaluate(&source, roi, roi, p, Default::default(), &Interrupt::NONE).unwrap()
            );
            for lower in
                [GaussianLimits { resident_bytes: limits.resident_bytes - 1, ..limits }, GaussianLimits { work_units: limits.work_units - 1, ..limits }]
            {
                assert_eq!(evaluate(&source, roi, roi, p, lower, &Interrupt::NONE), Err(RasterInputError::LimitExceeded));
            }
        }
        assert_eq!(source, before);
    }
}

#[test]
fn public_noop_reports_clone_cost_while_legacy_limit_bypass_and_raw_storage_stay_exact() {
    let source = source(SampleType::F64);
    let roi = Rect::new(0, 0, 3, 1);
    let empty = Rect::new(0, 0, 0, 0);
    let zero = GaussianLimits { resident_bytes: 0, work_units: 0 };
    let report = box_blur_requirements(&source, roi, roi, 0, &Interrupt::NONE).unwrap();
    assert!(report.peak_resident_bytes >= 2 * source.resident_payload_bytes().unwrap());
    assert!(report.work_units > 0);
    assert_eq!(try_box_blur(&source, roi, roi, 0, zero, &Interrupt::NONE).unwrap(), source);
    let report = median_requirements(&source, empty, roi, median(true), &Interrupt::NONE).unwrap();
    assert!(report.peak_resident_bytes >= 2 * source.resident_payload_bytes().unwrap());
    assert!(report.work_units > 0);
    assert_eq!(try_median_blur(&source, empty, roi, median(true), zero, &Interrupt::NONE).unwrap(), source);
}

#[test]
fn requirements_reject_invalid_geometry_params_and_cancellation_without_mutating_source() {
    let source = source(SampleType::F32);
    let before = source.clone();
    let roi = Rect::new(0, 0, 3, 1);
    assert_eq!(box_blur_requirements(&source, roi, roi, 1001, &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion));
    assert_eq!(
        median_requirements(&source, roi, roi, MedianParams { percentile: f64::NAN, ..median(true) }, &Interrupt::NONE),
        Err(RasterInputError::InvalidSample)
    );
    assert_eq!(median_requirements(&source, Rect::new(-1, 0, 3, 1), roi, median(true), &Interrupt::NONE), Err(RasterInputError::InvalidBounds));
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert_eq!(box_blur_requirements(&source, roi, roi, 1, &interrupt), Err(RasterInputError::Cancelled));
    assert_eq!(median_requirements(&source, roi, roi, median(true), &interrupt), Err(RasterInputError::Cancelled));
    assert_eq!(source, before);
}

#[test]
fn capacity_admission_rejects_oversize_and_overflow_before_any_initialization() {
    assert_eq!(checked_allocation_capacity::<f64>(10, 10), Ok(()));
    assert_eq!(checked_allocation_capacity::<f64>(20, 10), Ok(()));
    assert_eq!(checked_allocation_capacity::<f64>(21, 10), Err(RasterInputError::LimitExceeded));
    assert_eq!(checked_allocation_capacity::<f64>(9, 10), Err(RasterInputError::LimitExceeded));
    assert_eq!(checked_allocation_capacity::<u8>(0, 0), Ok(()));
    assert_eq!(checked_allocation_capacity::<u8>(1, 0), Err(RasterInputError::LimitExceeded));
    assert_eq!(checked_allocation_capacity::<u8>(usize::MAX, usize::MAX), Err(RasterInputError::LimitExceeded));
    assert_eq!(checked_allocation_capacity::<[u64; 8]>(usize::MAX / 32, usize::MAX / 32), Err(RasterInputError::LimitExceeded));
    let over_cap = usize::try_from(MAX_CHECKED_RASTER_BYTES / 8 + 1).unwrap();
    assert_eq!(checked_allocation_capacity::<u64>(over_cap, over_cap), Err(RasterInputError::LimitExceeded));
}
