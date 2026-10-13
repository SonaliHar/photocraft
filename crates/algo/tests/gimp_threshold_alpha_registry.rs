use photocraft_algo::gimp::registry::{WorkingEncoding, point_member};
#[test]
fn checked_surface_alpha_threshold_preserves_raw_colors_and_outside_roi() {
    use photocraft_algo::gimp::point::{PointOperation, try_apply};
    use photocraft_color::PixelFormat;
    use photocraft_geom::Rect;
    use photocraft_raster::{Interrupt, Surface};
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_region_f64(Rect::from_xywh(0, 0, 2, 1), &[0.173, -0.0, 0.719, 0.6, 0.2, 0.4, 0.8, 0.6]).unwrap();
    let before = source.try_read_interleaved(Rect::from_xywh(0, 0, 2, 1)).unwrap();
    let output = try_apply(&source, Rect::from_xywh(0, 0, 1, 1), PointOperation::ThresholdAlpha { value: 0.5 }, 128 << 20, 1 << 20, &Interrupt::NONE).unwrap();
    let after = output.try_read_interleaved(Rect::from_xywh(0, 0, 2, 1)).unwrap();
    assert_eq!(before[..24], after[..24]);
    assert_eq!(f64::from_ne_bytes(after[24..32].try_into().unwrap()), 1.0);
    assert_eq!(before[32..], after[32..]);
    assert_eq!(source.try_read_interleaved(Rect::from_xywh(0, 0, 2, 1)).unwrap(), before);
}

#[test]
fn alpha_threshold_registry_preserves_color_and_checks_exact_boundaries() {
    let (member, operation) = point_member("gimp:threshold-alpha", Some("0:0"), &[("value", 0.5)]).unwrap();
    assert_eq!(member.encoding, WorkingEncoding::Linear);
    for channels in [1, 3] {
        for (alpha, expected) in [(0.5, 0.), (0.5 + 2_f64.powi(-25), 0.), (0.5 + 2_f64.powi(-24), 1.)] {
            let mut input = vec![0.173; channels + 1];
            input[0] = -0.0;
            input[channels] = alpha;
            let mut output = vec![9.; input.len()];
            operation.try_eval_pixel(&input, channels, &mut output).unwrap();
            assert_eq!(output[channels], expected);
            assert_eq!(output[..channels].iter().map(|v| v.to_bits()).collect::<Vec<_>>(), input[..channels].iter().map(|v| v.to_bits()).collect::<Vec<_>>());
        }
    }
    for (version, args) in [
        (None, vec![("value", 0.5)]),
        (Some("1:0"), vec![("value", 0.5)]),
        (Some("0:0"), vec![("value", 1.1)]),
        (Some("0:0"), vec![("value", f64::NAN)]),
        (Some("0:0"), vec![("value", 0.5), ("value", 0.5)]),
    ] {
        assert!(point_member("gimp:threshold-alpha", version, &args).is_err());
    }
    let mut output = [9.; 4];
    assert!(operation.try_eval_pixel(&[0.1, 0.2, 0.3, f64::INFINITY], 3, &mut output).is_err());
    assert_eq!(output, [9.; 4]);
}
