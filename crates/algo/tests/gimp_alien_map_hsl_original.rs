//! Independent public HSL response; local unpublished originals only.
use photocraft_algo::gimp::registry::point_member;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
#[test]
#[ignore = "local unpublished independently authored HSL response"]
fn independent_hsl_cube_wrap_and_all_keep_combinations_match_stock() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set corpus")).join("alien-map-hsl-public-response-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "54adbfc11f3950cfc0f7d629a7f9360573d1532f48170df98a271def0b212b20");
    let binding: Value = serde_json::from_slice(&manifest).unwrap();
    for a in binding["artifacts"].as_array().unwrap() {
        let data = std::fs::read(root.join(a["path"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&data)), a["sha256"].as_str().unwrap());
        assert_eq!(data.len() as u64, a["bytes"].as_u64().unwrap());
    }
    let proof: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut worst = 0.0_f64;
    let mut samples = 0;
    for row in proof["records"].as_array().unwrap() {
        let f = row["frequency"].as_f64().unwrap();
        let p = row["phase"].as_f64().unwrap();
        let keep: [f64; 3] = std::array::from_fn(|i| if row["keep"].as_bool().unwrap_or_else(|| row["keep"][i].as_bool().unwrap()) { 1. } else { 0. });
        let (_, op) = point_member(
            "gegl:alien-map",
            Some("0:0"),
            &[
                ("color-model", 1.),
                ("cpn-1-frequency", f),
                ("cpn-2-frequency", f),
                ("cpn-3-frequency", f),
                ("cpn-1-phaseshift", p),
                ("cpn-2-phaseshift", p),
                ("cpn-3-phaseshift", p),
                ("cpn-1-keep", keep[0]),
                ("cpn-2-keep", keep[1]),
                ("cpn-3-keep", keep[2]),
            ],
        )
        .unwrap();
        let raw = std::fs::read(root.join(row["merged_native"]["path"].as_str().unwrap())).unwrap();
        for (pixel, bytes) in proof["pixels"].as_array().unwrap().iter().zip(raw.as_chunks::<32>().0.iter()) {
            let input: [f64; 4] = std::array::from_fn(|i| pixel[i].as_f64().unwrap());
            let expected: [f64; 4] = std::array::from_fn(|i| f64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap()));
            let mut actual = [9.; 4];
            op.try_eval_pixel(&input, 3, &mut actual).unwrap();
            assert_eq!(actual[3].to_bits(), input[3].to_bits());
            for (a, b) in actual.iter().zip(expected) {
                worst = worst.max((a - b).abs());
                assert!((a - b).abs() <= 2. / 255., "f={f} p={p} keep={keep:?}: {a} != {b}");
            }
            samples += 1;
        }
    }
    println!("Independent HSL {samples} cube/achromatic/wrap samples max {}/255", worst * 255.);
}

#[test]
fn hsl_invalid_inputs_and_parameters_are_atomic() {
    use photocraft_algo::gimp::point::PointOperation;
    let op = PointOperation::AlienMapHsl { frequency: [1., 3.125, 20.], phase: [0., 27.5, 359.], keep: [true, false, true] };
    for input in [[f64::NAN, 0.5, 0.5, 0.5], [-0.01, 0.5, 0.5, 0.5], [1.01, 0.5, 0.5, 0.5], [0.5, 0.5, 0.5, 2.]] {
        let mut output = [9.; 4];
        assert!(op.try_eval_pixel(&input, 3, &mut output).is_err());
        assert_eq!(output, [9.; 4]);
    }
    let mut gray = [9.; 2];
    assert!(op.try_eval_pixel(&[0.5, 0.5], 1, &mut gray).is_err());
    assert_eq!(gray, [9.; 2]);
    let mut output = [9.; 4];
    let invalid = PointOperation::AlienMapHsl { frequency: [21.; 3], phase: [0.; 3], keep: [false; 3] };
    assert!(invalid.try_eval_pixel(&[0.5; 4], 3, &mut output).is_err());
    assert_eq!(output, [9.; 4]);
}

#[test]
#[ignore = "24MP release-only HSL kernel, decode/CMS/storage excluded"]
fn alien_map_hsl_24mp_release_kernel() {
    use photocraft_algo::gimp::point::PointOperation;
    assert!(!std::hint::black_box(cfg!(debug_assertions)), "run release");
    let op = PointOperation::AlienMapHsl { frequency: [3.25, 3.375, 3.5], phase: [35., 42.5, 50.], keep: [false; 3] };
    let input = std::hint::black_box([0.125, 0.375, 0.75, 0.5]);
    let mut output = [0.; 4];
    let start = std::time::Instant::now();
    for _ in 0..24_000_000 {
        op.try_eval_pixel(std::hint::black_box(&input), 3, &mut output).unwrap();
        std::hint::black_box(output);
    }
    println!("24MP checked HSL Alien Map kernel {:?}; decode/CMS/storage/native excluded", start.elapsed());
}
