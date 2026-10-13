use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_paint::wide_erase::{EraseColorPolicy as Policy, EraseLimits, try_erase_coverage};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn erasing_alpha_keeps_adjacent_u32_color_codes_that_legacy_roundtrip_loses() {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::U32, true);
    let area = Rect::new(0, 0, 1, 1);
    let original = [16_777_216_u32, 16_777_217, u32::MAX, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
    let mut source = Surface::try_from_interleaved(format, area, &original).unwrap();
    let history = source.clone();
    let mut legacy = source.clone();
    legacy.write_region(area, &source.read_region(area));
    assert_ne!(legacy.try_read_interleaved(area).unwrap(), original);
    try_erase_coverage(&mut source, area, &[1.0], 0.5, None, false, Policy::Preserve, EraseLimits::default(), &Interrupt::NONE).unwrap();
    let erased = source.try_read_interleaved(area).unwrap();
    assert_eq!(&erased[..12], &original[..12]);
    assert_eq!(u32::from_ne_bytes(erased[12..].try_into().unwrap()), 2_147_483_648);
    assert_eq!(history.try_read_interleaved(area).unwrap(), original);
}

#[test]
fn partial_f64_erasing_preserves_nan_color_payload_and_unselected_alpha_bits() {
    let area = Rect::new(255, -1, 257, 0);
    let values = [f64::from_bits(0x7ff8123456789abc), -0.0, 1.0 + f64::EPSILON, 1.0 - 2.0_f64.powi(-40), 0.0, 0.0, 0.0, f64::from_bits(0x7ff8abcdef123456)];
    let raw = values.into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let mut source = Surface::try_from_interleaved(PixelFormat::RGBA64F, area, &raw).unwrap();
    let history = source.clone();
    let selection = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &0.25_f64.to_ne_bytes()).unwrap();
    try_erase_coverage(&mut source, area, &[0.25, 0.0], 0.5, Some(&selection), false, Policy::Preserve, EraseLimits::default(), &Interrupt::NONE).unwrap();
    let erased = source.try_read_interleaved(area).unwrap();
    assert_eq!(&erased[..24], &raw[..24]);
    assert_eq!(&erased[32..], &raw[32..]);
    assert_eq!(f64::from_ne_bytes(erased[24..32].try_into().unwrap()).to_bits(), (values[3] * 0.96875).to_bits());
    assert_eq!(history.try_read_interleaved(area).unwrap(), raw);
}

#[test]
fn full_erase_policies_lock_and_zero_weight_keep_explicit_semantics() {
    let area = Rect::new(0, 0, 1, 1);
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_pixel_f64(0, 0, &[4.0, -0.0, 2.0, 1e-15]).unwrap();
    let history = source.clone();
    try_erase_coverage(&mut source, area, &[1.0], 1.0, None, true, Policy::Preserve, EraseLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(source, history);
    try_erase_coverage(&mut source, area, &[1.0], 1.0, None, false, Policy::Preserve, EraseLimits::default(), &Interrupt::NONE).unwrap();
    let erased = source.try_read_region_f64(area).unwrap();
    assert_eq!(erased[0], 4.0);
    assert_eq!(erased[1].to_bits(), (-0.0_f64).to_bits());
    assert_eq!(erased[3], 0.0);
    try_erase_coverage(&mut source, area, &[1.0], 1.0, None, false, Policy::ClearAtZero, EraseLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(source.try_read_interleaved(area).unwrap(), [0; 32]);
    let mut empty = Surface::new(PixelFormat::RGBA64F);
    let original = empty.clone();
    try_erase_coverage(&mut empty, area, &[1.0], 1.0, None, false, Policy::Preserve, EraseLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(empty, original);
    let zero = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false));
    try_erase_coverage(&mut empty, area, &[1.0], 1.0, Some(&zero), false, Policy::Preserve, EraseLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(empty, original);
}

#[test]
fn erased_input_validation_limits_and_cancellation_are_atomic() {
    let area = Rect::new(0, 0, 270, 20);
    let mut source = Surface::new(PixelFormat::RGBA64F);
    let history = source.clone();
    let coverage = vec![1.0; 270 * 20];
    let count = AtomicUsize::new(0);
    let poll = || count.fetch_add(1, Ordering::Relaxed) > 7;
    let interrupt = Interrupt::cancel_only(&poll);
    assert_eq!(
        try_erase_coverage(&mut source, area, &coverage, 1.0, None, false, Policy::Preserve, EraseLimits::default(), &interrupt).unwrap_err(),
        RasterInputError::Cancelled
    );
    assert_eq!(source, history);
    for limits in [EraseLimits { resident_bytes: 1, work_units: u64::MAX }, EraseLimits { resident_bytes: u64::MAX, work_units: 1 }] {
        assert_eq!(
            try_erase_coverage(&mut source, area, &coverage, 1.0, None, false, Policy::Preserve, limits, &Interrupt::NONE).unwrap_err(),
            RasterInputError::LimitExceeded
        );
        assert_eq!(source, history);
    }
    for (coverage, opacity) in [(vec![f64::NAN; 270 * 20], 1.0), (vec![1.0; 270 * 20], f64::INFINITY), (vec![1.0], 1.0)] {
        assert!(try_erase_coverage(&mut source, area, &coverage, opacity, None, false, Policy::Preserve, EraseLimits::default(), &Interrupt::NONE).is_err());
        assert_eq!(source, history);
    }
    let mut background = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F64, false));
    assert_eq!(
        try_erase_coverage(&mut background, Rect::new(0, 0, 1, 1), &[1.0], 1.0, None, false, Policy::Preserve, EraseLimits::default(), &Interrupt::NONE)
            .unwrap_err(),
        RasterInputError::UnsupportedConversion
    );
}
