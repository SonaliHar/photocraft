use photocraft_algo::gimp::point::{PointOperation, try_apply};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

#[test]
fn component_extraction_writes_gray_rgb_and_opaque_alpha_together() {
    let area = Rect::new(0, 0, 1, 1);
    let mut source = Surface::new(PixelFormat::RGBA8);
    source.try_write_interleaved(area, &[64, 128, 192, 128]).unwrap();
    let operation = PointOperation::ComponentExtract { component: 0, invert: false, linear: false };
    let output = try_apply(&source, area, operation, 8 << 20, 1 << 20, &Interrupt::NONE).unwrap();
    assert_eq!(output.try_read_interleaved(area).unwrap(), [64, 64, 64, 255]);
    assert_eq!(source.try_read_interleaved(area).unwrap(), [64, 128, 192, 128]);
}

#[test]
fn color_to_alpha_writes_unmixed_color_and_reduced_alpha_together() {
    let area = Rect::new(0, 0, 1, 1);
    let input = [0.75_f64, 0.5, 0.25, 0.5];
    let raw: Vec<u8> = input.into_iter().flat_map(f64::to_le_bytes).collect();
    let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true));
    source.try_write_interleaved(area, &raw).unwrap();
    let operation = PointOperation::ColorToAlpha { color: [0.5, 0.5, 0.5, 0.0], transparency: 0.0, opacity: 1.0 };
    let output = try_apply(&source, area, operation, 8 << 20, 1 << 20, &Interrupt::NONE).unwrap();
    let expected: Vec<u8> = [1.0_f64, 0.5, 0.0, 0.25].into_iter().flat_map(f64::to_le_bytes).collect();
    assert_eq!(output.try_read_interleaved(area).unwrap(), expected);
    assert_eq!(source.try_read_interleaved(area).unwrap(), raw);
}
