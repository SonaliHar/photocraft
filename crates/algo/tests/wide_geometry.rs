use photocraft_algo::wide_geometry::{GeometryLimits, OrthogonalTransform as T, try_transform_orthogonal};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn raw_flip_and_quarter_turns_preserve_exact_samples_defaults_and_history() {
    for sample in [SampleType::U32, SampleType::F64] {
        let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
        let values = [0x7ff8123456789abc_u64, 0x8000000000000000, 0x3ff0000000000001, 0, 0xffffffffffffffff, 0x1234567812345678];
        let raw: Vec<u8> = values.into_iter().flat_map(|v| v.to_ne_bytes().into_iter().take(sample.bytes())).collect();
        let default = &raw[..sample.bytes()];
        let bounds = Rect::new(255, -1, 258, 1);
        let mut source = Surface::try_with_default_bytes(fmt, default).unwrap();
        source.try_write_interleaved(bounds, &raw).unwrap();
        let history = source.clone();
        for (transform, indices) in [
            (T::FlipHorizontal, vec![2, 1, 0, 5, 4, 3]),
            (T::FlipVertical, vec![3, 4, 5, 0, 1, 2]),
            (T::RotateClockwise, vec![3, 0, 4, 1, 5, 2]),
            (T::RotateCounterclockwise, vec![2, 5, 1, 4, 0, 3]),
            (T::Rotate180, vec![5, 4, 3, 2, 1, 0]),
        ] {
            let (out, area) = try_transform_orthogonal(&source, bounds, (-258, 255), transform, GeometryLimits::default(), &Interrupt::NONE).unwrap();
            let expected: Vec<u8> = indices.into_iter().flat_map(|i| raw[i * sample.bytes()..(i + 1) * sample.bytes()].iter().copied()).collect();
            assert_eq!(out.try_read_interleaved(area).unwrap(), expected);
            assert_eq!(out.default_bytes(), default);
            assert_eq!(source, history);
            assert_eq!(source.try_read_interleaved(bounds).unwrap(), raw);
        }
        let (cw, area) = try_transform_orthogonal(&source, bounds, (0, 0), T::RotateClockwise, GeometryLimits::default(), &Interrupt::NONE).unwrap();
        let (back, area) =
            try_transform_orthogonal(&cw, area, (bounds.x0, bounds.y0), T::RotateCounterclockwise, GeometryLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(back.try_read_interleaved(area).unwrap(), raw);
    }
}

#[test]
fn hostile_geometry_limits_and_mid_mapping_cancel_leave_source_exact() {
    let source = Surface::new(PixelFormat::RGBA64F);
    let area = Rect::new(-1, -1, 270, 20);
    let history = source.clone();
    let count = AtomicUsize::new(0);
    let poll = || count.fetch_add(1, Ordering::Relaxed) > 4;
    let interrupt = Interrupt::cancel_only(&poll);
    assert_eq!(
        try_transform_orthogonal(&source, area, (0, 0), T::FlipHorizontal, GeometryLimits::default(), &interrupt).unwrap_err(),
        RasterInputError::Cancelled
    );
    assert!(count.load(Ordering::Relaxed) > 4);
    assert_eq!(source, history);
    assert_eq!(
        try_transform_orthogonal(&source, area, (0, 0), T::Rotate180, GeometryLimits { resident_bytes: 1, work_units: u64::MAX }, &Interrupt::NONE)
            .unwrap_err(),
        RasterInputError::LimitExceeded
    );
    assert_eq!(
        try_transform_orthogonal(&source, area, (0, 0), T::Rotate180, GeometryLimits { resident_bytes: u64::MAX, work_units: 1 }, &Interrupt::NONE)
            .unwrap_err(),
        RasterInputError::LimitExceeded
    );
    assert_eq!(
        try_transform_orthogonal(&source, area, (i32::MAX, 0), T::Rotate180, GeometryLimits::default(), &Interrupt::NONE).unwrap_err(),
        RasterInputError::InvalidBounds
    );
    assert_eq!(
        try_transform_orthogonal(&source, Rect::new(3, 0, 2, 1), (0, 0), T::FlipVertical, GeometryLimits::default(), &Interrupt::NONE).unwrap_err(),
        RasterInputError::InvalidBounds
    );
}
