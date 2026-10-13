use photocraft_algo::{
    paint::GradientShape,
    wide_paint::{GradientOptions, NativeStop, PaintLimits, try_paint_gradient, try_paint_region},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
fn options() -> GradientOptions {
    GradientOptions { from: (0.0, 0.0), to: (4.0, 0.0), shape: GradientShape::Linear, reverse: false, opacity: 1.0, lock_transparency: false }
}

#[test]
fn malformed_retained_tile_keys_reject_solid_and_gradient_atomically() {
    use photocraft_algo::wide_paint::try_paint_solid;
    use photocraft_raster::RasterInputError;
    let mut valid = Surface::new(PixelFormat::RGBA64F);
    valid.try_write_pixel_f64(0, 0, &[0.0, 0.0, 0.0, 1.0]).unwrap();
    let tile = valid.tiles().next().unwrap().1.clone();
    for gradient in [false, true] {
        let mut target = Surface::new(PixelFormat::RGBA64F);
        target.put_tiles([(photocraft_geom::TileCoord::new(i32::MIN, i32::MAX), tile.clone())]);
        let before = target.clone();
        let result = if gradient {
            try_paint_gradient(
                &mut target,
                Rect::new(0, 0, 4, 1),
                options(),
                &[NativeStop { position: 0.0, color: [0.0, 0.0, 0.0, 1.0] }],
                None,
                Default::default(),
                &Interrupt::NONE,
            )
        } else {
            try_paint_solid(&mut target, Rect::new(0, 0, 4, 1), &[0.0, 0.0, 0.0, 1.0], 1.0, None, false, Default::default(), &Interrupt::NONE)
        };
        assert_eq!(result, Err(RasterInputError::InvalidBounds));
        assert_eq!(target, before);
    }
}
#[test]
fn gradient_f64_stops_and_unselected_raw_history_default_bits_survive() {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let raw = [-0.0_f64, 1.125, 0.5 + 2.0_f64.powi(-40), 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let mut surface = Surface::try_with_default_bytes(format, &raw).unwrap();
    let before = surface.clone();
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut selection = Surface::new(gray);
    selection.try_write_pixel_f64(1, 0, &[1.0]).unwrap();
    let color = [0.5 + 2.0_f64.powi(-40), -0.0, 3.0, 1.0];
    let stops = [NativeStop { position: 0.0, color }, NativeStop { position: 1.0, color }];
    try_paint_gradient(&mut surface, Rect::new(0, 0, 4, 1), options(), &stops, Some(&selection), PaintLimits::default(), &Interrupt::NONE).unwrap();
    let painted = surface.try_read_region_f64(Rect::new(1, 0, 2, 1)).unwrap();
    assert_eq!(painted[0].to_bits(), color[0].to_bits());
    assert_eq!(painted[2], 3.0);
    assert_eq!(painted[1].to_bits(), color[1].to_bits());
    assert_eq!(surface.try_read_interleaved(Rect::new(2, 0, 4, 1)).unwrap(), before.try_read_interleaved(Rect::new(2, 0, 4, 1)).unwrap());
    assert_eq!(surface.default_bytes(), before.default_bytes());
    assert_eq!(before.try_read_interleaved(Rect::new(1, 0, 2, 1)).unwrap(), raw);
}
#[test]
fn native_u32_region_color_rounds_once_after_coverage_and_keeps_adjacent_unselected_codes() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
    let bytes = [0_u32, 0x8000_0001].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
    let mut surface = Surface::try_from_interleaved(format, Rect::new(0, 0, 2, 1), &bytes).unwrap();
    let before = surface.clone();
    try_paint_region(
        &mut surface,
        Rect::new(0, 0, 2, 1),
        &[0.5, 0.0],
        &[0.6 / f64::from(u32::MAX)],
        1.0,
        None,
        false,
        PaintLimits::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    assert_eq!(surface, before); // Premature U32 source rounding would turn0.6 into1 before weighting.
    try_paint_region(
        &mut surface,
        Rect::new(0, 0, 2, 1),
        &[1.0, 0.0],
        &[0.8 / f64::from(u32::MAX)],
        1.0,
        None,
        false,
        PaintLimits::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    let result = surface.try_read_interleaved(Rect::new(0, 0, 2, 1)).unwrap();
    assert_eq!(&result[..4], &1_u32.to_ne_bytes());
    assert_eq!(&result[4..], &bytes[4..]);
}
#[test]
fn linear_gradient_uses_f64_coordinates_reverse_and_ordered_duplicate_edges() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut surface = Surface::new(format);
    let stops = [NativeStop { position: 0.0, color: [0.0; 4] }, NativeStop { position: 1.0, color: [1.0; 4] }];
    try_paint_gradient(&mut surface, Rect::new(0, 0, 4, 1), options(), &stops, None, PaintLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(surface.try_read_region_f64(Rect::new(0, 0, 4, 1)).unwrap(), [0.125, 0.375, 0.625, 0.875]);
    let mut reversed = options();
    reversed.reverse = true;
    try_paint_gradient(&mut surface, Rect::new(0, 0, 4, 1), reversed, &stops, None, PaintLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(surface.try_read_region_f64(Rect::new(0, 0, 4, 1)).unwrap(), [0.875, 0.625, 0.375, 0.125]);
    let edge = [stops[0], NativeStop { position: 0.5, color: [0.0; 4] }, NativeStop { position: 0.5, color: [1.0; 4] }, stops[1]];
    try_paint_gradient(&mut surface, Rect::new(0, 0, 4, 1), options(), &edge, None, PaintLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(surface.try_read_region_f64(Rect::new(0, 0, 4, 1)).unwrap(), [0.0, 0.0, 1.0, 1.0]);
}
#[test]
fn alpha_lock_zero_coverage_and_cancellation_are_atomic() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
    let raw = [(2.0 + 2.0_f64.powi(-40)).to_ne_bytes(), 0.25_f64.to_ne_bytes()].concat();
    let mut surface = Surface::try_with_default_bytes(format, &raw).unwrap();
    let before = surface.clone();
    try_paint_region(&mut surface, Rect::new(0, 0, 1, 1), &[0.5], &[0.0, 1.0], 0.5, None, true, PaintLimits::default(), &Interrupt::NONE).unwrap();
    let values = surface.try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap();
    assert_eq!(values[1].to_bits(), 0.25_f64.to_bits());
    assert!((values[0] - (2.0 + 2.0_f64.powi(-40)) * 0.25 * 0.75 / (0.25 + 0.25 * 0.75)).abs() < 1e-15);
    let changed = surface.clone();
    assert!(
        try_paint_region(&mut surface, Rect::new(0, 0, 1, 1), &[1.0], &[0.0, 1.0], 1.0, None, false, PaintLimits::default(), &Interrupt::cancel_only(&|| true))
            .is_err()
    );
    assert_eq!(surface, changed);
    try_paint_region(&mut surface, Rect::new(0, 0, 1, 1), &[0.0], &[0.0, 1.0], 1.0, None, false, PaintLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(surface, changed);
    assert_eq!(before.default_bytes(), raw);
}
#[test]
fn malformed_inputs_and_budget_rejection_never_publish() {
    let mut surface = Surface::new(PixelFormat::RGBA64F);
    let before = surface.clone();
    let area = Rect::new(0, 0, 1, 1);
    let stop = NativeStop { position: 0.0, color: [1.0; 4] };
    for stops in [
        vec![],
        vec![NativeStop { position: f64::NAN, ..stop }],
        vec![NativeStop { position: 0.8, ..stop }, stop],
        vec![NativeStop { color: [0.0, 0.0, 0.0, 2.0], ..stop }],
    ] {
        assert!(try_paint_gradient(&mut surface, area, options(), &stops, None, PaintLimits::default(), &Interrupt::NONE).is_err());
        assert_eq!(surface, before);
    }
    assert!(
        try_paint_gradient(&mut surface, area, options(), &[stop], None, PaintLimits { resident_bytes: 1, work_units: u64::MAX }, &Interrupt::NONE).is_err()
    );
    assert_eq!(surface, before);
    assert!(try_paint_region(&mut surface, area, &[f64::NAN], &[0.0; 4], 1.0, None, false, PaintLimits::default(), &Interrupt::NONE).is_err());
    assert_eq!(surface, before);
    let mut degenerate = options();
    degenerate.to = degenerate.from;
    assert!(try_paint_gradient(&mut surface, area, degenerate, &[stop], None, PaintLimits::default(), &Interrupt::NONE).is_err());
    assert_eq!(surface, before);
}

#[test]
fn locked_transparent_nonfinite_hidden_color_is_untouched_and_midwork_cancel_is_atomic() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
    let raw = [0x7ff8_1234_5678_abcd_u64.to_ne_bytes(), 0.0_f64.to_ne_bytes()].concat();
    let mut target = Surface::try_with_default_bytes(format, &raw).unwrap();
    let before = target.clone();
    try_paint_region(&mut target, Rect::new(0, 0, 1, 1), &[1.0], &[0.0, 1.0], 1.0, None, true, PaintLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(target, before);
    try_paint_region(&mut target, Rect::new(0, 0, 1, 1), &[1.0], &[0.0, 0.0], 1.0, None, false, PaintLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(target, before);
    let mut target = Surface::new(PixelFormat::RGBA64F);
    let before = target.clone();
    let count = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 8;
    assert!(
        try_paint_region(
            &mut target,
            Rect::new(0, 0, 513, 1),
            &vec![1.0; 513],
            &[1.0; 4],
            1.0,
            None,
            false,
            PaintLimits::default(),
            &Interrupt::cancel_only(&cancel)
        )
        .is_err()
    );
    assert_eq!(target, before);
}

#[test]
fn solid_fill_retains_raw_default_and_checks_budget_before_coverage_scan() {
    use photocraft_algo::wide_paint::try_paint_solid;
    let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut target = Surface::try_with_default_bytes(fmt, &(-0.0_f64).to_ne_bytes()).unwrap();
    let original = target.clone();
    let value = 0.5 + 2.0_f64.powi(-40);
    try_paint_solid(&mut target, Rect::new(0, 0, 1, 1), &[value], 1.0, None, false, Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(target.try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap()[0].to_bits(), value.to_bits());
    assert_eq!(target.default_bytes(), original.default_bytes());
    assert_eq!(target.try_read_interleaved(Rect::new(1, 0, 2, 1)).unwrap(), original.default_bytes());
    let before = target.clone();
    let error = try_paint_region(
        &mut target,
        Rect::new(0, 0, 1, 1),
        &[f64::NAN],
        &[value],
        1.0,
        None,
        false,
        PaintLimits { resident_bytes: 1, work_units: 1 },
        &Interrupt::NONE,
    )
    .unwrap_err();
    assert_eq!(error, photocraft_raster::RasterInputError::LimitExceeded);
    assert_eq!(target, before);
}
