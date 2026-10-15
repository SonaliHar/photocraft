//! UNRUN planning/legacy constructor boundary drafts, no aggregate quota claim.
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
fn params() -> GaussianParams {
    GaussianParams { std_dev_x: 2.25, std_dev_y: 1.5, abyss: GaussianAbyss::Clamp, clip_extent: true }
}
fn source(sample: SampleType) -> Surface {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
    for y in -1..2 {
        for x in -2..3 {
            s.try_write_pixel_f64(x, y, &[0.25 + (x + 2) as f64 * 0.125, 0.5, 0.75, if x % 2 == 0 { 1.0 } else { 0.5 }]).unwrap();
        }
    }
    s.try_write_pixel_f64(800, 800, &[-0.0, 0.75, 0.125, 1.0]).unwrap();
    s
}
#[test]
#[ignore = "UNRUN: prospective recursive requirements cover real sparse native output and retain exact legacy recurrence/default behavior"]
fn recursive_report_admits_unchanged_old_constructor_output() {
    let roi = Rect::new(-2, -1, 3, 2);
    for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
        let source = source(sample);
        let before = source.clone();
        let req = gaussian_young_recursive_requirements(&source, roi, roi, params(), &Interrupt::NONE).unwrap();
        let actual = try_gaussian_young_recursive(
            &source,
            roi,
            roi,
            params(),
            GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units },
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(actual, try_gaussian_young_recursive(&source, roi, roi, params(), Default::default(), &Interrupt::NONE).unwrap());
        assert_eq!(actual.default_bytes(), source.default_bytes());
        let untouched = Rect::new(800, 800, 801, 801);
        assert_eq!(actual.try_read_interleaved(untouched).unwrap(), source.try_read_interleaved(untouched).unwrap());
        assert_eq!(source, before);
    }
}
#[test]
#[ignore = "UNRUN: growing sparse output and all retained input map entries contribute to prospective work/peak without kernel execution"]
fn sparse_map_and_multiple_output_tiles_are_included() {
    let mut s = source(SampleType::F64);
    let roi = Rect::new(-2, -1, 3, 2);
    let base = gaussian_young_recursive_requirements(&s, roi, roi, params(), &Interrupt::NONE).unwrap();
    for i in 0..20 {
        s.try_write_pixel_f64(1024 + i * 256, 1024, &[0.25, 0.5, 0.75, 1.0]).unwrap();
    }
    let sparse = gaussian_young_recursive_requirements(&s, roi, roi, params(), &Interrupt::NONE).unwrap();
    assert!(sparse.work_units > base.work_units);
    assert!(sparse.peak_resident_bytes > base.peak_resident_bytes);
    let multiple = Rect::new(-1, 0, 257, 1);
    let req = gaussian_young_recursive_requirements(&s, multiple, multiple, params(), &Interrupt::NONE).unwrap();
    let out = try_gaussian_young_recursive(
        &s,
        multiple,
        multiple,
        params(),
        GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units },
        &Interrupt::NONE,
    )
    .unwrap();
    assert!(out.tile_count() >= s.tile_count());
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let stop = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 5;
    let interrupt = Interrupt::cancel_only(&stop);
    assert_eq!(gaussian_young_recursive_requirements(&s, roi, roi, params(), &interrupt), Err(RasterInputError::Cancelled));
}
#[test]
#[ignore = "UNRUN: truthful no-op clone envelope does not redefine legacy zero-limit refusal or malformed parameter behavior"]
fn noop_and_invalid_parameters_keep_legacy_policy() {
    let s = source(SampleType::F64);
    let roi = Rect::new(-2, -1, 3, 2);
    let zero = GaussianParams { std_dev_x: 0.0, std_dev_y: 0.0, ..params() };
    let req = gaussian_young_recursive_requirements(&s, roi, roi, zero, &Interrupt::NONE).unwrap();
    assert!(req.peak_resident_bytes >= 2 * s.resident_payload_bytes().unwrap());
    assert!(req.work_units > 0);
    assert_eq!(
        try_gaussian_young_recursive(&s, roi, roi, zero, GaussianLimits { resident_bytes: 0, work_units: 0 }, &Interrupt::NONE),
        Err(RasterInputError::LimitExceeded)
    );
    assert_eq!(
        try_gaussian_young_recursive(
            &s,
            roi,
            roi,
            zero,
            GaussianLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units },
            &Interrupt::NONE
        )
        .unwrap(),
        s
    );
    for sigma in [f64::NAN, f64::INFINITY, -1.0, 0.25] {
        let bad = GaussianParams { std_dev_x: sigma, ..params() };
        assert!(gaussian_young_recursive_requirements(&s, roi, roi, bad, &Interrupt::NONE).is_err());
    }
}
