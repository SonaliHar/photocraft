use photocraft_algo::gimp::{
    point::{PointOperation, try_apply},
    registry::red_eye_member,
};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

#[test]
fn typed_red_eye_dispatch_writes_color_and_float_coverage_atomically() {
    let (_, operation) = red_eye_member(Some("0:0"), 0.625).unwrap();
    assert!(matches!(operation, PointOperation::RedEyeRemoval { threshold: 0.625 }));
    assert!(red_eye_member(Some("future"), 0.625).is_err());
    assert!(red_eye_member(Some("0:0"), f64::from(0.8f32)).is_err());
    let source = Surface::new(PixelFormat::RGBA32F);
    let bounds = Rect::new(0, 0, 1, 1);
    let mut source = source;
    source.try_write_pixel_f64(0, 0, &[0.875, 0.125, 0.25, 0.333333333333]).unwrap();
    let original = source.try_read_interleaved(bounds).unwrap();
    let result = try_apply(&source, bounds, operation, 64 << 20, 10000, &Interrupt::NONE).unwrap();
    let actual = result.try_read_interleaved(bounds).unwrap();
    let values: Vec<f32> = actual.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)).collect();
    let expected_red = ((f64::from(0.125f32 + (0.25f64 * 0.1933333) as f32) / (2. * 0.5133333)) as f32).clamp(0., 1.);
    assert_eq!(values.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), [expected_red, 0.125, 0.25, 0.333_333_34_f32].map(f32::to_bits));
    assert_eq!(source.try_read_interleaved(bounds).unwrap(), original);
    assert!(try_apply(&source, bounds, operation, 64 << 20, 10000, &Interrupt::cancel_only(&|| true)).is_err());
    assert_eq!(source.try_read_interleaved(bounds).unwrap(), original);
}
