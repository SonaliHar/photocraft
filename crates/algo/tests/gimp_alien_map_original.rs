//! Independent original public-response gate; local unpublished evidence only.
use photocraft_algo::gimp::registry::point_member;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[test]
#[ignore = "24MP release-only checked RGB Alien Map kernel, decode/CMS/storage excluded"]
fn alien_map_24mp_release_kernel() {
    use photocraft_algo::gimp::point::PointOperation;
    assert!(!std::hint::black_box(cfg!(debug_assertions)), "run release");
    let op = PointOperation::AlienMap { frequency: [3.25, 3.375, 3.5], phase: [35., 42.5, 50.], keep: [false; 3] };
    let input = std::hint::black_box([0.125, 0.375, 0.75, 0.5]);
    let mut output = [0.; 4];
    let start = std::time::Instant::now();
    for _ in 0..24_000_000 {
        op.try_eval_pixel(std::hint::black_box(&input), 3, &mut output).unwrap();
        std::hint::black_box(output);
    }
    println!("24MP checked RGB Alien Map kernel {:?}; decode/CMS/storage/native excluded", start.elapsed());
}

#[test]
fn checked_rgb_alien_map_errors_are_atomic() {
    use photocraft_algo::gimp::point::PointOperation;
    let valid = PointOperation::AlienMap { frequency: [1., 3.25, 20.], phase: [0., 90., 359.], keep: [true, false, false] };
    for input in [[f64::NAN, 0.5, 0.5, 0.5], [f64::INFINITY, 0.5, 0.5, 0.5], [-0.01, 0.5, 0.5, 0.5], [1.01, 0.5, 0.5, 0.5], [0.5, 0.5, 0.5, 2.]] {
        let mut output = [9.; 4];
        assert!(valid.try_eval_pixel(&input, 3, &mut output).is_err());
        assert_eq!(output, [9.; 4]);
    }
    let mut gray = [9.; 2];
    assert!(valid.try_eval_pixel(&[0.5, 0.5], 1, &mut gray).is_err());
    assert_eq!(gray, [9.; 2]);
    for operation in [
        PointOperation::AlienMap { frequency: [f64::NAN, 1., 1.], phase: [0.; 3], keep: [false; 3] },
        PointOperation::AlienMap { frequency: [21., 1., 1.], phase: [0.; 3], keep: [false; 3] },
        PointOperation::AlienMap { frequency: [1.; 3], phase: [-1., 0., 0.], keep: [false; 3] },
        PointOperation::AlienMap { frequency: [1.; 3], phase: [361., 0., 0.], keep: [false; 3] },
    ] {
        let mut output = [9.; 4];
        assert!(operation.try_eval_pixel(&[0.5; 4], 3, &mut output).is_err());
        assert_eq!(output, [9.; 4]);
    }
}

#[test]
#[ignore = "local unpublished independently authored Alien Map public response"]
fn independent_rgb_frequency_phase_and_keep_match_public_response() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set corpus")).join("alien-map-public-response-evidence");
    let manifest = std::fs::read(root.join("batch-manifest.json")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "6388fd167ce730cb89ea8f371c4e62f9789e26a1729beb76453fcf3e7f390382");
    let binding: Value = serde_json::from_slice(&manifest).unwrap();
    for a in binding["artifacts"].as_array().unwrap() {
        let data = std::fs::read(root.join(a["path"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&data)), a["sha256"].as_str().unwrap());
        assert_eq!(data.len() as u64, a["bytes"].as_u64().unwrap());
    }
    let proof: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut worst = 0.0_f64;
    for row in proof["records"].as_array().unwrap() {
        let f = row["frequency"].as_f64().unwrap();
        let p = row["phase"].as_f64().unwrap();
        let k = if row["keep"].as_bool().unwrap() { 1. } else { 0. };
        let (_, op) = point_member(
            "gegl:alien-map",
            Some("0:0"),
            &[
                ("color-model", 0.),
                ("cpn-1-frequency", f),
                ("cpn-2-frequency", f),
                ("cpn-3-frequency", f),
                ("cpn-1-phaseshift", p),
                ("cpn-2-phaseshift", p),
                ("cpn-3-phaseshift", p),
                ("cpn-1-keep", k),
                ("cpn-2-keep", k),
                ("cpn-3-keep", k),
            ],
        )
        .unwrap();
        let raw = std::fs::read(root.join(row["merged_native"]["path"].as_str().unwrap())).unwrap();
        for (x, bytes) in proof["values"].as_array().unwrap().iter().zip(raw.as_chunks::<32>().0.iter()) {
            let x = x.as_f64().unwrap();
            let expected: [f64; 4] = std::array::from_fn(|i| f64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap()));
            let mut actual = [9.; 4];
            op.try_eval_pixel(&[x, x, x, 0.5], 3, &mut actual).unwrap();
            assert_eq!(actual[3].to_bits(), 0.5_f64.to_bits());
            for (a, b) in actual.iter().zip(expected) {
                worst = worst.max((a - b).abs());
                assert!((a - b).abs() <= 2. / 255., "f={f} p={p} k={k}: {a} != {b}");
            }
        }
    }
    println!("Alien Map independent650 ramp samples max {}/255", worst * 255.);
}
