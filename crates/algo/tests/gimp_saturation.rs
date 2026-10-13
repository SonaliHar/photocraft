use photocraft_algo::gimp::registry::{WorkingEncoding, point_member};
#[test]
fn native_saturation_is_typed_linear_and_atomic() {
    let (member, operation) = point_member("gegl:saturation", Some("0:0"), &[("scale", 0.5), ("colorspace", 0.0)]).unwrap();
    assert_eq!(member.encoding, WorkingEncoding::Linear);
    let mut output = [9.; 4];
    operation.try_eval_pixel(&[1., 0., 0., -0.0], 3, &mut output).unwrap();
    assert!((output[0] - 0.6112439632415771).abs() < 1e-7);
    assert!((output[1] - 0.11124397069215775).abs() < 1e-7);
    assert_eq!(output[3].to_bits(), (-0.0f64).to_bits());
    for scale in [-0.1, 10.1, f64::NAN] {
        assert!(point_member("gegl:saturation", Some("0:0"), &[("scale", scale), ("colorspace", 0.0)]).is_err());
    }
    for choice in [-1., 0.5, 3.] {
        assert!(point_member("gegl:saturation", Some("0:0"), &[("scale", 0.5), ("colorspace", choice)]).is_err());
    }
    for bad in [-0.1, 1.1, f64::NAN] {
        let mut result = [9.; 4];
        assert!(operation.try_eval_pixel(&[bad, 0.4, 0.7, 1.], 3, &mut result).is_err());
        assert_eq!(result, [9.; 4]);
    }
}

#[test]
fn public_cie_coordinate_choices_match_independent_primary_observations() {
    for (choice, expected) in
        [(1., [0.5865739583969116, 0.1228514090180397, 0.06450749933719635]), (2., [0.6339406744761142, 0.10246295615755352, 0.13179227345395134])]
    {
        let (_, op) = point_member("gegl:saturation", Some("0:0"), &[("scale", 0.5), ("colorspace", choice)]).unwrap();
        let mut out = [0.; 4];
        op.try_eval_pixel(&[1., 0., 0., 1.], 3, &mut out).unwrap();
        for (a, b) in out[..3].iter().zip(expected) {
            assert!((a - b).abs() < 0.00002);
        }
    }
}
#[test]
#[ignore = "local unpublished independent Saturation125-point RGB cube public holdouts"]
fn independent_public_saturation_cube_continuous_scale_holdouts() {
    use sha2::Digest;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("saturation-range-evidence");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(
        sha2::Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(),
        "da3ed2adf822c5e264d986a242258a36d2226f84b75dd6e8489abe2a03ddbb80"
    );
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for a in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(
            sha2::Sha256::digest(std::fs::read(root.join(a["path"].as_str().unwrap())).unwrap()).iter().map(|v| format!("{v:02x}")).collect::<String>(),
            a["sha256"]
        );
    }
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let source = std::fs::read(root.join("source-rgba-double.bin")).unwrap();
    let decode = |b: &[u8]| std::array::from_fn::<_, 4, _>(|i| f64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap()));
    let mut count = 0;
    let mut rejected = 0;
    let mut worst = 0.0f64;
    for row in proof["rows"].as_array().unwrap() {
        let choice = match row["colorspace"].as_str().unwrap() {
            "Native" => 0,
            "CIE-Lab" => 1,
            "CIE-Yuv" => 2,
            _ => panic!("unexpected public choice"),
        };
        let scale = row["config"]["scale"].as_f64().unwrap();
        let (_, op) = point_member("gegl:saturation", Some("0:0"), &[("scale", scale), ("colorspace", f64::from(choice))]).unwrap();
        let reference = std::fs::read(root.join(row["output"]["file"].as_str().unwrap())).unwrap();
        for (pixel, refpixel) in source.as_chunks::<32>().0.iter().zip(reference.as_chunks::<32>().0.iter()) {
            let pixel = decode(pixel);
            let expected = decode(refpixel);
            let mut actual = [9.; 4];
            if op.try_eval_pixel(&pixel, 3, &mut actual).is_err() {
                assert_eq!(actual, [9.; 4]);
                assert!(expected[..3].iter().any(|v| !v.is_finite() || v.abs() > 256.0), "rejected public output {:?}", expected);
                rejected += 1;
                continue;
            }
            assert_eq!(actual[3].to_bits(), pixel[3].to_bits());
            for (a, b) in actual.iter().zip(expected) {
                let delta = (a - b).abs();
                worst = worst.max(delta);
                assert!(delta <= 2.0 / 255.0, "{} {} {:?} -> {:?} expected {:?}: {}/255", row["colorspace"], scale, pixel, actual, expected, delta * 255.0);
            }
            count += 1;
        }
    }
    println!("Saturation125×30 cube admitted{count}/rejected{rejected} max{}/255", worst * 255.0);
    assert!(count > 3000);
}
#[test]
#[ignore = "manual release-only24MP F64 Saturation hot-path benchmark"]
fn saturation_24mp_release() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run with --release");
    }
    let pixel: Vec<u8> = [0.25_f64, 0.5, 0.75, 1.0].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let source = photocraft_raster::Surface::try_with_default_bytes(photocraft_color::PixelFormat::RGBA64F, &pixel).unwrap();
    for colorspace in 0..=2 {
        let operation = photocraft_algo::gimp::point::PointOperation::Saturation { scale: 0.5, colorspace };
        let start = std::time::Instant::now();
        let out = photocraft_algo::gimp::point::try_apply(
            &source,
            photocraft_geom::Rect::from_xywh(0, 0, 6000, 4000),
            operation,
            2u64 << 30,
            6000 * 4000 * 32 * 16,
            &photocraft_raster::Interrupt::NONE,
        )
        .unwrap();
        println!("24MP F64 Saturationchoice{colorspace}: {:.3}s", start.elapsed().as_secs_f64());
        let mut expected = [0.; 4];
        operation.try_eval_pixel(&[0.25, 0.5, 0.75, 1.], 3, &mut expected).unwrap();
        let mut actual = [0.; 4];
        out.try_read_pixel_f64(5999, 3999, &mut actual).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(source.default_bytes(), pixel);
    }
    Ok(())
}

#[test]
fn checked_gray_saturation_keeps_current_samples_and_alpha_bits() {
    for colorspace in 0..=2 {
        for scale in [0.0, 0.5, 1.0, 1.5, 10.0] {
            let (_, op) = point_member("gegl:saturation", Some("0:0"), &[("scale", scale), ("colorspace", f64::from(colorspace))]).unwrap();
            for input in [[-0.0, -0.0], [0.03125, 0.0], [0.75, 0.5], [1.0, 1.0]] {
                let mut out = [9.0; 2];
                op.try_eval_pixel(&input, 1, &mut out).unwrap();
                assert_eq!(out.map(f64::to_bits), input.map(f64::to_bits));
            }
            for value in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
                let mut out = [9.0; 2];
                assert!(op.try_eval_pixel(&[value, 0.5], 1, &mut out).is_err());
                assert_eq!(out, [9.0; 2]);
            }
        }
    }
}

#[test]
#[ignore = "local unpublished Gray Saturation live response proof"]
fn independent_gray_live_ramps_all_choices_and_scales() {
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("saturation-gray-native-range-evidence");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(
        Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect::<String>(),
        "42da99678ab0ed82ff963e14982060ac8378ed95fd700102d577908ee339019e"
    );
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for a in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(
            Sha256::digest(std::fs::read(root.join(a["path"].as_str().unwrap())).unwrap()).iter().map(|v| format!("{v:02x}")).collect::<String>(),
            a["sha256"]
        );
    }
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut worst = 0.0f64;
    let mut count = 0;
    for row in proof["rows"].as_array().unwrap() {
        let choice = match row["colorspace"].as_str().unwrap() {
            "Native" => 0.0,
            "CIE-Lab" => 1.0,
            "CIE-Yuv" => 2.0,
            _ => panic!("unexpected choice"),
        };
        let (_, op) = point_member("gegl:saturation", Some("0:0"), &[("scale", row["scale"].as_f64().unwrap()), ("colorspace", choice)]).unwrap();
        let source = std::fs::read(root.join(row["source"].as_str().unwrap())).unwrap();
        let live = std::fs::read(root.join(row["live"].as_str().unwrap())).unwrap();
        let precision = row["precision"].as_u64().unwrap();
        let width = if precision < 300 {
            2
        } else if precision < 700 {
            4
        } else {
            8
        };
        assert_eq!(source.len(), 33 * width * 2);
        assert_eq!(live.len(), 33 * 16);
        let decode = |b: &[u8]| match width {
            2 => f64::from(u16::from_le_bytes(b.try_into().unwrap())) / 65535.0,
            4 => f64::from(f32::from_le_bytes(b.try_into().unwrap())),
            _ => f64::from_le_bytes(b.try_into().unwrap()),
        };
        for (input, expected) in source.chunks_exact(width * 2).zip(live.as_chunks::<16>().0.iter()) {
            let input = [decode(&input[..width]), decode(&input[width..])];
            let expected = [f64::from_le_bytes(expected[..8].try_into().unwrap()), f64::from_le_bytes(expected[8..].try_into().unwrap())];
            let mut out = [9.0; 2];
            op.try_eval_pixel(&input, 1, &mut out).unwrap();
            assert_eq!(out.map(f64::to_bits), input.map(f64::to_bits));
            assert!((out[1] - expected[1]).abs() <= 1.0 / 65535.0);
            worst = worst.max((out[0] * out[1] - expected[0] * expected[1]).abs());
        }
        count += 1;
    }
    assert_eq!(count, 90);
    assert!(worst <= 2.0 / 255.0);
    println!("90 independent Gray Saturation live ramps max {}/255", worst * 255.0);
}
