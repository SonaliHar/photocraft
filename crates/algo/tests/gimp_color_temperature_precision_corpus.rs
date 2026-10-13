#![cfg(feature = "corpus")]
use photocraft_algo::gimp::point::PointOperation;
#[test]
#[ignore = "local unpublished independent public Color Temperature off-grid observations"]
fn independent_off_grid_kelvin_pairs() {
    use sha2::Digest;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("color-temperature-range-evidence");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(format!("{:x}", sha2::Sha256::digest(&bytes)), "b50d5832dd0ce64bdc7faace52e7c60c6b6faa015d5d54ad08234c067b2aea67");
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(
            format!("{:x}", sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())),
            artifact["sha256"].as_str().unwrap()
        );
    }
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("temperature-range-observations.json")).unwrap()).unwrap();
    let mut worst = 0.0_f64;
    for (model, channels) in [("RGB", 3), ("GRAY", 1)] {
        let mut accepted = 0;
        let mut rejected = 0;
        for row in proof["independent_off_grid_pairs"][model].as_array().unwrap() {
            let operation = PointOperation::ColorTemperature { original: row["original"].as_f64().unwrap(), intended: row["intended"].as_f64().unwrap() };
            let mut output = [0.0; 4];
            if operation.try_eval_pixel(&[1.0; 4][..channels + 1], channels, &mut output[..channels + 1]).is_err() {
                rejected += 1;
                continue;
            }
            accepted += 1;
            for (value, expected) in output[..channels].iter().zip(row["samples"].as_array().unwrap()) {
                worst = worst.max((value - expected.as_f64().unwrap()).abs());
            }
            assert_eq!(output[channels].to_bits(), 1.0_f64.to_bits());
        }
        assert_eq!((accepted, rejected), (2002, 1));
    }
    println!("independent off-grid temperature pairs worst {}/255", worst * 255.0);
    assert!(worst <= 2.0 / 255.0);
}
#[test]
#[ignore = "local unpublished independent GEGL Color Temperature precision originals"]
fn independent_color_temperature_precision_alpha_baked_native() {
    use sha2::Digest;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("color-temperature-precision-evidence");
    let raw = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(format!("{:x}", sha2::Sha256::digest(&raw)), "f727e979471d42eff4535cda98f5adcef9f43cf0f47578dae99a43f2fb104b3c");
    let manifest: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(
            format!("{:x}", sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())),
            artifact["sha256"].as_str().unwrap()
        );
    }
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    let curve = photocraft_cms::curve::srgb_trc();
    for row in proof["checks"].as_array().unwrap() {
        let state = &row["fresh"];
        let layer = &state["layers"][0];
        let precision = state["precision"].as_str().unwrap();
        let width = if precision.contains("U16") {
            2
        } else if precision.contains("FLOAT") {
            4
        } else {
            8
        };
        let channels = if layer["native_format"].as_str().unwrap().starts_with('Y') { 1 } else { 3 };
        let count = channels + 1;
        let linear = precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR");
        let config = &layer["filters"][0]["config"];
        let operation = PointOperation::ColorTemperature {
            original: config["original-temperature"].as_f64().unwrap(),
            intended: config["intended-temperature"].as_f64().unwrap(),
        };
        let enabled = layer["filters"][0]["visible"].as_bool().unwrap();
        let source = std::fs::read(root.join(layer["native"]["file"].as_str().unwrap())).unwrap();
        let reference = std::fs::read(root.join(row["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        let decode = |bytes: &[u8]| match width {
            2 => f64::from(u16::from_le_bytes(bytes.try_into().unwrap())) / 65535.0,
            4 => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
            _ => f64::from_le_bytes(bytes.try_into().unwrap()),
        };
        assert_eq!(source.len(), 48 * count * width);
        assert_eq!(reference.len(), source.len());
        let mut worst = 0.0_f64;
        for (pixel, expected) in source.chunks_exact(count * width).zip(reference.chunks_exact(count * width)) {
            assert_eq!(&pixel[channels * width..], &expected[channels * width..]);
            let mut input: Vec<f64> = pixel.chunks_exact(width).map(decode).collect();
            let mut output = input.clone();
            if enabled {
                if !linear {
                    for value in &mut input[..channels] {
                        *value = curve.eval64(*value);
                    }
                }
                operation.try_eval_pixel(&input, channels, &mut output).unwrap();
                if !linear {
                    for value in &mut output[..channels] {
                        *value = curve.eval_inverse64(*value);
                    }
                }
            }
            for (actual, expected) in output.iter().zip(expected.chunks_exact(width)) {
                // Integer source storage cannot represent out-of-range filtered samples.
                let actual = if width == 2 { actual.clamp(0.0, 1.0) } else { *actual };
                worst = worst.max((actual - decode(expected)).abs());
            }
        }
        println!("{} GEGL Color Temperature baked native {}/255", row["id"], worst * 255.0);
        assert!(worst <= 2.0 / 255.0);
    }
    assert_eq!(proof["checks"].as_array().unwrap().len(), 48);
}
