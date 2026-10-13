//! Independently authored histogram channels and bounded threshold references.
#![cfg(feature = "corpus")]
use serde_json::Value;
use sha2::{Digest, Sha256};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn samples(bytes: &[u8], precision: &str) -> Vec<f64> {
    if precision.contains("U16") {
        bytes.as_chunks::<2>().0.iter().map(|b| f64::from(u16::from_le_bytes(*b)) / 65535.0).collect()
    } else if precision.contains("FLOAT") {
        bytes.as_chunks::<4>().0.iter().map(|b| f64::from(f32::from_le_bytes(*b))).collect()
    } else {
        bytes.as_chunks::<8>().0.iter().map(|b| f64::from_le_bytes(*b)).collect()
    }
}
fn encode(value: f64) -> f64 {
    if value <= 0.0031308 { value * 12.92 } else { 1.055 * value.powf(1.0 / 2.4) - 0.055 }
}
#[test]
#[ignore = "requires local unpublished histogram Threshold references"]
fn independent_channels_endpoints_disabled_and_alpha_match_saved_gimp() {
    run(false);
}
#[test]
#[ignore = "requires local unpublished RGB Threshold references"]
fn independent_rgb_channels_endpoints_disabled_and_alpha_match_saved_gimp() {
    run(true);
}
fn run(rgb_only: bool) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "23015e327f45c490ed6dbeaa6f4a9541b13dbe8526e9abfff26da13d38c56142");
    let hashes: Value = serde_json::from_slice(&manifest).unwrap();
    let proof: Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in proof["records"].as_array().unwrap().iter().filter(|row| !rgb_only || row["model"] == "RGB") {
        let precision = row["precision"].as_str().unwrap();
        let linear = precision.ends_with("_LINEAR") && !precision.ends_with("_NON_LINEAR");
        let channels = if row["model"] == "GRAY" { 1 } else { 3 };
        let load = |role: &str| {
            let file = row[role]["layers"][0]["native"]["file"].as_str().unwrap();
            let bytes = std::fs::read(root.join(file)).unwrap();
            assert_eq!(hash(&bytes), hashes[file]);
            samples(&bytes, precision)
        };
        let input = load("reopened");
        let reference = load("baked");
        assert_eq!(input.len(), 48 * (channels + 1));
        assert_eq!(reference.len(), input.len());
        let (_, operation) = photocraft_algo::gimp::registry::point_member(
            "gimp:threshold",
            Some("0:0"),
            &[
                ("channel", row["requested"]["channel"].as_f64().unwrap()),
                ("low", row["requested"]["low"].as_f64().unwrap()),
                ("high", row["requested"]["high"].as_f64().unwrap()),
            ],
        )
        .unwrap();
        for (source, expected) in input.chunks_exact(channels + 1).zip(reference.chunks_exact(channels + 1)) {
            let mut working = source.to_vec();
            if linear {
                for value in &mut working[..channels] {
                    *value = encode(*value);
                }
            }
            let mut actual = vec![0.0; channels + 1];
            if row["requested"]["enabled"].as_bool().unwrap() {
                operation.try_eval_pixel(&working, channels, &mut actual).unwrap();
            } else {
                actual.copy_from_slice(&working);
            }
            let expected: Vec<_> = expected.iter().enumerate().map(|(i, v)| if linear && i < channels { encode(*v) } else { *v }).collect();
            for (a, b) in actual.iter().zip(&expected) {
                assert!((a - b).abs() <= 1.0 / 65535.0, "{}: {a} vs {b}", row["id"]);
            }
            assert_eq!(actual[channels].to_bits(), source[channels].to_bits());
        }
        count += 1;
    }
    assert_eq!(count, if rgb_only { 48 } else { 78 });
}
