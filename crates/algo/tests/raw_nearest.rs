use photocraft_algo::{
    transform::{Homography, try_warp_nearest_surface_with_limits},
    wide_geometry::GeometryLimits,
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
#[test]
fn nearest_copies_hidden_samples_and_negative_zero_at_every_depth() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
        let fmt = PixelFormat::new(ColorMode::Grayscale, sample, true);
        let raw = match sample {
            SampleType::U8 => vec![213, 0, 127, 128],
            SampleType::U16 => [54321u16, 0, 32767, 32768].into_iter().flat_map(u16::to_ne_bytes).collect(),
            SampleType::U32 => [0x87654321u32, 0, 0x80000001, 0x80000002].into_iter().flat_map(u32::to_ne_bytes).collect(),
            SampleType::F32 => [0.83f32, -0., 0.5 + f32::EPSILON, 0.5].into_iter().flat_map(f32::to_ne_bytes).collect(),
            SampleType::F64 => [0.83f64, -0., 0.5 + 2f64.powi(-48), 0.5].into_iter().flat_map(f64::to_ne_bytes).collect(),
        };
        let bounds = Rect::new(-2, -1, 0, 0);
        let source = Surface::try_from_interleaved(fmt, bounds, &raw).unwrap();
        let before = source.clone();
        let matrix = Homography([2., 0., 0., 0., 1., 0., 0., 0., 1.]);
        let out = try_warp_nearest_surface_with_limits(&source, bounds, &matrix, Default::default(), &Interrupt::NONE).unwrap();
        let pixel = fmt.bytes_per_pixel();
        let expected = [&raw[..pixel], &raw[..pixel], &raw[pixel..], &raw[pixel..]].concat();
        assert_eq!(out.try_read_interleaved(Rect::new(-4, -1, 0, 0)).unwrap(), expected);
        assert_eq!(out.try_read_interleaved(Rect::new(0, -1, 1, 0)).unwrap(), vec![0; pixel]);
        assert_eq!(source, before);
    }
}
#[test]
fn nearest_limit_and_cancel_fail_without_changing_source() {
    let bounds = Rect::new(-2, -1, 0, 0);
    let source = Surface::try_from_interleaved(
        PixelFormat::new(ColorMode::Rgb, SampleType::F32, true),
        bounds,
        &[0.83f32, 0.19, 0.61, -0., 0.25, 0.5, 0.75, 1.].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>(),
    )
    .unwrap();
    let before = source.clone();
    let matrix = Homography([1.25, 0., 0.25, 0., 0.75, 0.375, 0., 0., 1.]);
    assert!(matches!(
        try_warp_nearest_surface_with_limits(&source, bounds, &matrix, GeometryLimits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE),
        Err(RasterInputError::LimitExceeded)
    ));
    assert!(matches!(
        try_warp_nearest_surface_with_limits(&source, bounds, &matrix, Default::default(), &Interrupt::cancel_only(&|| true)),
        Err(RasterInputError::Cancelled)
    ));
    assert_eq!(source, before);
}
