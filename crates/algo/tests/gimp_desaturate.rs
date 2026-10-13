use photocraft_algo::gimp::registry::{WorkingEncoding, point_member};
#[test]
fn public_desaturate_modes_are_closed_typed_and_atomic() {
    let expected = [0.4, 0.351436095, 0.4, 0.351436095, 0.7];
    for mode in 0..5 {
        let (member, operation) = point_member("gimp:desaturate", Some("0:0"), &[("mode", f64::from(mode))]).unwrap();
        assert_eq!(member.encoding, if mode == 3 { WorkingEncoding::Linear } else { WorkingEncoding::Nonlinear });
        let mut output = [9.; 4];
        operation.try_eval_pixel(&[0.1, 0.4, 0.7, -0.0], 3, &mut output).unwrap();
        assert!((output[0] - expected[mode as usize]).abs() < 1e-6);
        assert_eq!(output[0], output[1]);
        assert_eq!(output[1], output[2]);
        assert_eq!(output[3].to_bits(), (-0.0f64).to_bits());
        for bad in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            let mut result = [9.; 4];
            assert!(operation.try_eval_pixel(&[bad, 0.4, 0.7, 1.], 3, &mut result).is_err());
            assert_eq!(result, [9.; 4]);
        }
        assert!(operation.try_eval_pixel(&[0.4, 1.], 1, &mut [9.; 2]).is_err());
    }
    for mode in [-1., 5., 0.5, f64::NAN] {
        assert!(point_member("gimp:desaturate", Some("0:0"), &[("mode", mode)]).is_err());
    }
}

#[test]
#[ignore = "manual release-only24MP F64 checked desaturate kernel benchmark"]
fn desaturate_24mp_release() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let pixel: Vec<u8> = [0.25_f64, 0.5, 0.75, 1.0].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let source = photocraft_raster::Surface::try_with_default_bytes(photocraft_color::PixelFormat::RGBA64F, &pixel).unwrap();
    let operation = photocraft_algo::gimp::point::PointOperation::Desaturate { mode: 3 };
    let start = std::time::Instant::now();
    let out = photocraft_algo::gimp::point::try_apply(
        &source,
        photocraft_geom::Rect::from_xywh(0, 0, 6000, 4000),
        operation,
        2_u64 << 30,
        6000 * 4000 * 32 * 16,
        &photocraft_raster::Interrupt::NONE,
    )
    .unwrap();
    println!("24MP F64 checkedDesaturate: {:.3}s", start.elapsed().as_secs_f64());
    let mut expected = [0.0; 4];
    operation.try_eval_pixel(&[0.25, 0.5, 0.75, 1.0], 3, &mut expected).unwrap();
    let mut actual = [0.0; 4];
    out.try_read_pixel_f64(5999, 3999, &mut actual).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(source.default_bytes(), pixel);
    Ok(())
}
