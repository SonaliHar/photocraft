use photocraft_algo::gimp::{point::PointOperation, registry::point_member};
#[test]
fn black_white_endpoints_and_original_zero_black_bits() {
    let operation = PointOperation::Exposure { stops: 0.5, black_level: 0.0625 };
    let mut output = [0.0; 4];
    operation.try_eval_pixel(&[0.0625, (-0.5_f64).exp2(), 0.5, -0.0], 3, &mut output).unwrap();
    assert_eq!(output[0], 0.0);
    assert_eq!(output[1], 1.0);
    assert_eq!(output[3].to_bits(), (-0.0_f64).to_bits());
    let legacy = PointOperation::Exposure { stops: 0.5, black_level: 0.0 };
    legacy.try_eval_pixel(&[0.125, 0.5, 0.875, 0.25], 3, &mut output).unwrap();
    for (value, actual) in [0.125_f64, 0.5, 0.875].into_iter().zip(output) {
        assert_eq!(actual.to_bits(), (value * 0.5_f64.exp2()).to_bits());
    }
    assert!(point_member("gegl:exposure", Some("1:0"), &[("exposure", 0.5), ("black-level", 0.0625)]).is_ok());
}
#[test]
fn unqualified_or_nonfinite_white_minus_black_rejects_without_output_mutation() {
    for (stops, black) in [(4.0, 0.0625), (5.0, 0.0625), (0.0, 0.125), (f64::NAN, 0.0625), (0.5, f64::INFINITY), (f64::NEG_INFINITY, 0.0625)] {
        let mut output = [1.0, 2.0, 3.0, 4.0];
        assert!(PointOperation::Exposure { stops, black_level: black }.try_eval_pixel(&[0.25, 0.5, 0.75, 0.5], 3, &mut output).is_err());
        assert_eq!(output, [1.0, 2.0, 3.0, 4.0]);
    }
}

#[test]
#[ignore = "manual release-only 24MP F64 checked point kernel benchmark"]
fn exposure_black_24mp_release() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let pixel: Vec<u8> = [0.25_f64, 0.5, 0.75, 1.0].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let source = photocraft_raster::Surface::try_with_default_bytes(photocraft_color::PixelFormat::RGBA64F, &pixel).unwrap();
    let bounds = photocraft_geom::Rect::from_xywh(0, 0, 6000, 4000);
    for black_level in [0.0, 0.0625] {
        let operation = PointOperation::Exposure { stops: 0.5, black_level };
        let start = std::time::Instant::now();
        let out = photocraft_algo::gimp::point::try_apply(&source, bounds, operation, 2_u64 << 30, 6000 * 4000 * 32 * 16, &photocraft_raster::Interrupt::NONE)
            .unwrap();
        println!("24MP F64 checkedExposure black={black_level}: {:.3}s", start.elapsed().as_secs_f64());
        let mut expected = [0.0; 4];
        operation.try_eval_pixel(&[0.25, 0.5, 0.75, 1.0], 3, &mut expected).unwrap();
        let mut actual = [0.0; 4];
        out.try_read_pixel_f64(5999, 3999, &mut actual).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(source.default_bytes(), pixel);
    }
    Ok(())
}
