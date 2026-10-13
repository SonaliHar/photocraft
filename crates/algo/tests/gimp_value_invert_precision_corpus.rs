#![cfg(feature = "corpus")]
use photocraft_algo::gimp::point::PointOperation;
use photocraft_cms::curve::srgb_trc;
#[test]
#[ignore = "local unpublished independently authored Value Invert precision and alpha"]
fn independent_value_invert_precision_alpha_native_baked_outputs() {
    use sha2::Digest;
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("value-invert-precision-evidence");
    let bytes = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(format!("{:x}", sha2::Sha256::digest(&bytes)), "8a75ccbb88e61611d3c3523db8145f922b63676601986c124d2d8493a27e73a1");
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(
            format!("{:x}", sha2::Sha256::digest(std::fs::read(root.join(artifact["path"].as_str().unwrap())).unwrap())),
            artifact["sha256"].as_str().unwrap()
        );
    }
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("fresh-verification.json")).unwrap()).unwrap();
    // Source and operation share the same implicit RGB primaries: only the
    // explicitly declared transfer changes, with no intervening ICC matrix round trip.
    let curve = srgb_trc();
    for record in proof["checks"].as_array().unwrap() {
        let state = &record["fresh"];
        let precision = state["precision"].as_str().unwrap();
        let source = std::fs::read(root.join(state["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        let reference = std::fs::read(root.join(record["baked"]["layers"][0]["native"]["file"].as_str().unwrap())).unwrap();
        let width = if precision.contains("U16") {
            2
        } else if precision.contains("FLOAT") {
            4
        } else {
            8
        };
        let decode = |raw: &[u8]| -> f64 {
            match width {
                2 => f64::from(u16::from_le_bytes(raw.try_into().unwrap())) / 65535.0,
                4 => f64::from(f32::from_le_bytes(raw.try_into().unwrap())),
                _ => f64::from_le_bytes(raw.try_into().unwrap()),
            }
        };
        assert_eq!(source.len(), 48 * 4 * width);
        assert_eq!(reference.len(), source.len());
        let enabled = state["layers"][0]["filters"][0]["visible"].as_bool().unwrap();
        let linear = precision == "GIMP_PRECISION_FLOAT_LINEAR";
        let mut worst = 0.0_f64;
        for (source, expected) in source.chunks_exact(width * 4).zip(reference.chunks_exact(width * 4)) {
            assert_eq!(&source[width * 3..], &expected[width * 3..]);
            let input: [f64; 4] = std::array::from_fn(|i| decode(&source[i * width..(i + 1) * width]));
            let mut output = input;
            if enabled {
                let mut color = [0.0; 3];
                if linear {
                    color = std::array::from_fn(|channel| curve.eval_inverse64(input[channel]));
                } else {
                    color.copy_from_slice(&input[..3]);
                }
                PointOperation::ValueInvert.try_eval_pixel(&[color[0], color[1], color[2], input[3]], 3, &mut output).unwrap();
                if linear {
                    let color: [f64; 3] = std::array::from_fn(|channel| curve.eval64(output[channel]));
                    output[..3].copy_from_slice(&color);
                }
            }
            for (actual, expected) in output.iter().zip(expected.chunks_exact(width)) {
                worst = worst.max((actual - decode(expected)).abs());
            }
        }
        println!("{} native baked error{}/255", record["id"], worst * 255.0);
        assert!(worst <= 2.0 / 255.0);
    }
}
