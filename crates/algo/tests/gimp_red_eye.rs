use photocraft_algo::gimp::point_red_eye::RedEyeRemoval;

#[test]
#[ignore = "24MP release pure arithmetic timing; excludes CMS and storage"]
fn red_eye_24mp_working_kernel_timing() {
    let operation = RedEyeRemoval { threshold: 0.4 };
    let mut output = [0.0; 4];
    let start = std::time::Instant::now();
    let mut checksum = 0.0f64;
    for pixel in 0..24_000_000u32 {
        let input = [((pixel % 17) as f32) / 16.0, ((pixel % 7) as f32) / 6.0, ((pixel % 5) as f32) / 4.0, 0.5];
        operation.try_eval_nonlinear_float(std::hint::black_box(&input), &mut output).unwrap();
        checksum += f64::from(std::hint::black_box(output[0]));
    }
    assert!(checksum.is_finite() && checksum > 0.0);
    println!("RedEye explicit nonlinearFloat24MP pure kernel: {:.6}s checksum={checksum}", start.elapsed().as_secs_f64());
}

#[test]
fn invalid_operands_are_atomic() {
    for threshold in [f64::NAN, f64::INFINITY, -0.001, 0.801] {
        let mut out = [17.0; 4];
        assert!(RedEyeRemoval { threshold }.try_eval_nonlinear_float(&[0.5, 0.125, 0.0, 1.0], &mut out).is_err());
        assert_eq!(out, [17.0; 4]);
    }
    for channel in 0..4 {
        let mut input = [0.5, 0.125, 0.0, 1.0];
        input[channel] = f32::NAN;
        let mut out = [17.0; 4];
        assert!(RedEyeRemoval { threshold: 0.4 }.try_eval_nonlinear_float(&input, &mut out).is_err());
        assert_eq!(out, [17.0; 4]);
    }
    for input in [vec![], vec![0.0; 3], vec![0.0; 5], vec![0.5, 0.125, 0.0, -0.1], vec![0.5, 0.125, 0.0, 1.1], vec![f32::MAX, -f32::MAX, -f32::MAX, 1.0]] {
        let mut out = [17.0; 4];
        assert!(RedEyeRemoval { threshold: 0.4 }.try_eval_nonlinear_float(&input, &mut out).is_err());
        assert_eq!(out, [17.0; 4]);
    }
    let mut short = [17.0; 3];
    assert!(RedEyeRemoval { threshold: 0.4 }.try_eval_nonlinear_float(&[0.5, 0.125, 0.0, 1.0], &mut short).is_err());
    assert_eq!(short, [17.0; 3]);
}

#[test]
fn hidden_color_and_unchanged_channels_retain_bits() {
    for alpha in [0.0f32, -0.0, 0.5, 1.0] {
        let input = [0.375, 0.125, -0.0, alpha];
        let mut out = [17.0; 4];
        RedEyeRemoval { threshold: 0.4 }.try_eval_nonlinear_float(&input, &mut out).unwrap();
        assert_eq!(out[0].to_bits(), 0x3df9_59c5);
        for channel in 1..4 {
            assert_eq!(out[channel].to_bits(), input[channel].to_bits());
        }
    }
    let mut out = [17.0; 4];
    RedEyeRemoval { threshold: 0.4 }.try_eval_nonlinear_float(&[-0.0, -0.0, -0.0, -0.0], &mut out).unwrap();
    assert!(out.iter().all(|v| v.to_bits() == (-0.0f32).to_bits()));
}

#[test]
#[ignore = "frozen independent working-domain evidence; no source admission"]
fn original_response_boundary_decimal_and_signed_zero() {
    use sha2::{Digest, Sha256};
    let hash = |b: &[u8]| Sha256::digest(b).iter().map(|n| format!("{n:02x}")).collect::<String>();
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full");
    let packets = [
        ("red-eye-removal-response-evidence", "11402e9f1282383642378f00e3fb960a0121847e4eb1b43053103c2c7394794d"),
        ("red-eye-removal-boundary-evidence", "8496cb0a0d08ea6f7f7d7beffb3b9f5be778bcd8e3a7263a4298328d1c08d5ea"),
        ("red-eye-removal-decimal-holdout-evidence", "fffbb8bf7d17d9f572cfd9be4867e747339a18c0a9722a5718f0f6c33c79a55b"),
        ("red-eye-removal-signed-zero-evidence", "db10cc366c61acf1100d9750516b3757644f645266c013d51d84e2ed1fc93373"),
    ];
    let mut pixels = 0usize;
    for (name, expected_manifest) in packets {
        let root = base.join(name);
        let manifest_bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
        assert_eq!(hash(&manifest_bytes), expected_manifest);
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
        let entry = manifest["artifacts"].as_array().unwrap().iter().find(|e| e["path"] == "observations.json").unwrap();
        let bytes = std::fs::read(root.join("observations.json")).unwrap();
        assert_eq!(hash(&bytes), entry["sha256"].as_str().unwrap());
        let observations: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for row in observations["records"].as_array().unwrap() {
            if name == "red-eye-removal-response-evidence" && (row["model"] != "RGB" || !row["precision"].as_str().unwrap().ends_with("NON_LINEAR")) {
                continue;
            }
            let current = &row["current"]["layers"][0];
            let read = |descriptor: &serde_json::Value| {
                let bytes = std::fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
                assert_eq!(bytes.len() as u64, descriptor["bytes"].as_u64().unwrap());
                assert_eq!(hash(&bytes), descriptor["sha256"].as_str().unwrap());
                assert_eq!(bytes.len() % 16, 0);
                bytes.as_chunks::<4>().0.iter().map(|v| f32::from_le_bytes(*v)).collect::<Vec<_>>()
            };
            let input = read(&current["working"]["R'G'B'A float"]);
            let expected = read(&row["baked"]["layers"][0]["working"]["R'G'B'A float"]);
            assert_eq!(input.len(), expected.len());
            let threshold = current["filters"][0]["config"]["threshold"].as_f64().unwrap();
            for (index, (input, expected)) in input.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()).enumerate() {
                let mut output = [17.0; 4];
                if row["enabled"] == false {
                    output.copy_from_slice(input);
                } else {
                    RedEyeRemoval { threshold }.try_eval_nonlinear_float(input, &mut output).unwrap();
                }
                for (actual, expected) in output.iter().zip(expected) {
                    if name == "red-eye-removal-signed-zero-evidence" {
                        assert_eq!(actual.to_bits(), expected.to_bits(), "{} pixel {index}", row["id"]);
                    } else {
                        assert!((actual - expected).abs() <= 1e-6, "{} pixel {index}: {output:?} vs {expected:?}", row["id"]);
                    }
                }
                for (actual, original) in output.iter().skip(1).zip(input.iter().skip(1)) {
                    assert_eq!(actual.to_bits(), original.to_bits());
                }
                pixels += 1;
            }
        }
    }
    assert_eq!(pixels, 15_584);
}
