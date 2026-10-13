//! Independent original public-response gate; local unpublished evidence only.
use photocraft_algo::gimp::registry::point_member;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").expect("set PHOTOCRAFT_XCF_FULL_CORPUS")).join("sepia-public-response-evidence")
}
fn read(root: &std::path::Path, path: &str) -> Vec<u8> {
    std::fs::read(root.join(path)).unwrap()
}
fn samples(bytes: &[u8]) -> Vec<[f64; 4]> {
    assert_eq!(bytes.len() % 32, 0);
    bytes.as_chunks::<32>().0.iter().map(|p| std::array::from_fn(|i| f64::from_le_bytes(p[i * 8..i * 8 + 8].try_into().unwrap()))).collect()
}
#[test]
#[ignore = "local unpublished independently authored Sepia public response packet"]
fn independent_rgb_cube_transfer_choices_and_coupled_scales() {
    let root = root();
    let manifest = read(&root, "batch-manifest.json");
    assert_eq!(format!("{:x}", Sha256::digest(&manifest)), "f87f7e8ca4808ffa68e8de8c46ce1b0827c2857dcc54472cbe665f4ce54514e1");
    let binding: Value = serde_json::from_slice(&manifest).unwrap();
    for row in binding["artifacts"].as_array().unwrap() {
        let data = read(&root, row["path"].as_str().unwrap());
        assert_eq!(data.len() as u64, row["bytes"].as_u64().unwrap());
        assert_eq!(format!("{:x}", Sha256::digest(&data)), row["sha256"].as_str().unwrap());
    }
    let proof: Value = serde_json::from_slice(&read(&root, "observations.json")).unwrap();
    let mut count = 0;
    let mut worst = 0.0_f64;
    for row in proof["records"].as_array().unwrap() {
        let srgb = row["srgb"].as_bool().unwrap();
        let scale = row["scale"].as_f64().unwrap();
        let (_, op) = point_member("gegl:sepia", Some("0:0"), &[("scale", scale), ("srgb", if srgb { 1. } else { 0. })]).unwrap();
        let source_key = if srgb { "nonlinear_source" } else { "source" };
        let output_key = if srgb { "nonlinear" } else { "linear" };
        let source = samples(&read(&root, proof[source_key]["path"].as_str().unwrap()));
        let expected = samples(&read(&root, row[output_key]["path"].as_str().unwrap()));
        assert_eq!(source.len(), 125);
        assert_eq!(expected.len(), 125);
        for (input, expected) in source.iter().zip(expected) {
            let mut output = [0.; 4];
            op.try_eval_pixel(input, 3, &mut output).unwrap();
            assert_eq!(output[3].to_bits(), input[3].to_bits());
            for (a, b) in output.iter().zip(expected) {
                worst = worst.max((a - b).abs());
                assert!((a - b).abs() <= 2. / 255., "srgb={srgb} scale={scale}: {a} != {b}");
            }
            count += 1;
        }
    }
    assert_eq!(count, 1250);
    println!("Sepia1250 public response comparisons max {}/255", worst * 255.);
}

#[test]
fn checked_sepia_rejects_bad_descriptor_and_pixels_atomically() {
    for args in [
        vec![("scale", f64::NAN), ("srgb", 1.)],
        vec![("scale", -1.), ("srgb", 1.)],
        vec![("scale", 2.), ("srgb", 1.)],
        vec![("scale", 0.5), ("srgb", 0.5)],
        vec![("scale", 0.5), ("scale", 1.)],
        vec![("scale", 0.5)],
    ] {
        assert!(point_member("gegl:sepia", Some("0:0"), &args).is_err());
    }
    assert!(point_member("gegl:sepia", Some("1:0"), &[("scale", 0.5), ("srgb", 1.)]).is_err());
    let (_, op) = point_member("gegl:sepia", Some("0:0"), &[("scale", 0.5), ("srgb", 1.)]).unwrap();
    for input in [
        [f64::NAN, 0.25, 0.75, 0.5],
        [f64::INFINITY, 0.25, 0.75, 0.5],
        [-0.1, 0.25, 0.75, 0.5],
        [1.1, 0.25, 0.75, 0.5],
        [0.1, 0.25, 0.75, f64::NAN],
        [0.1, 0.25, 0.75, 1.1],
    ] {
        let mut output = [9.; 4];
        assert!(op.try_eval_pixel(&input, 3, &mut output).is_err());
        assert_eq!(output, [9.; 4]);
    }
    let mut gray = [9.; 2];
    assert!(op.try_eval_pixel(&[0.5, 0.5], 1, &mut gray).is_err());
    assert_eq!(gray, [9.; 2]);
    let mut short = [9.; 3];
    assert!(op.try_eval_pixel(&[0.1, 0.2, 0.3, 0.5], 3, &mut short).is_err());
    assert_eq!(short, [9.; 3]);
}

#[test]
#[ignore = "24MP release-only Sepia checked kernel; decode/CMS/storage excluded"]
fn sepia_24mp_release_kernel() {
    assert!(!std::hint::black_box(cfg!(debug_assertions)), "run release");
    let (_, op) = point_member("gegl:sepia", Some("0:0"), &[("scale", 0.5), ("srgb", 0.)]).unwrap();
    let input = std::hint::black_box([0.125, 0.375, 0.75, 0.5]);
    let mut output = [0.; 4];
    let start = std::time::Instant::now();
    for _ in 0..24_000_000 {
        op.try_eval_pixel(std::hint::black_box(&input), 3, &mut output).unwrap();
        std::hint::black_box(output);
    }
    println!("24MP checked Sepia kernel {:?}; decode/CMS/storage/native persistence excluded", start.elapsed());
}
