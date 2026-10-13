use photocraft_algo::gimp::point::PointOperation;
#[test]
fn temperature_checked_atomic_domains_and_exact_alpha() {
    let operation = PointOperation::ColorTemperature { original: 6500.0, intended: 4000.0 };
    let alpha = f64::from_bits(0x8000000000000000);
    let mut output = [9.0; 4];
    operation.try_eval_pixel(&[1.0, 1.0, 1.0, alpha], 3, &mut output).unwrap();
    for (value, expected) in output.iter().zip([0.7380187511444092, 1.0645488500595093, 1.9414352178573608]) {
        assert!((value - expected).abs() < 0.00001);
    }
    assert_eq!(output[3].to_bits(), alpha.to_bits());
    for invalid in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        output = [9.0; 4];
        assert!(operation.try_eval_pixel(&[invalid, 0.5, 0.25, alpha], 3, &mut output).is_err());
        assert_eq!(output, [9.0; 4]);
    }
    for (original, intended) in [(999.0, 6500.0), (6500.0, 12001.0), (f64::NAN, 6500.0), (6500.0, f64::INFINITY), (10661.9482421875, 1906.62060546875)] {
        assert!(PointOperation::ColorTemperature { original, intended }.validate().is_err());
    }
}

#[test]
#[ignore = "manual release-only24MP F64 checked temperature kernel benchmark"]
fn temperature_24mp_release() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let pixel: Vec<u8> = [0.25_f64, 0.5, 0.75, 1.0].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let source = photocraft_raster::Surface::try_with_default_bytes(photocraft_color::PixelFormat::RGBA64F, &pixel).unwrap();
    let operation = PointOperation::ColorTemperature { original: 7031.125, intended: 5299.375 };
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
    println!("24MP F64 checkedTemperature: {:.3}s", start.elapsed().as_secs_f64());
    let mut expected = [0.0; 4];
    operation.try_eval_pixel(&[0.25, 0.5, 0.75, 1.0], 3, &mut expected).unwrap();
    let mut actual = [0.0; 4];
    out.try_read_pixel_f64(5999, 3999, &mut actual).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(source.default_bytes(), pixel);
    Ok(())
}
