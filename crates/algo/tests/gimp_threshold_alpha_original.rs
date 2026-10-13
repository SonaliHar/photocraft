#![cfg(feature = "corpus")]
//! Original saved native alpha-only outputs, independent of the compositor.
use photocraft_algo::gimp::threshold_alpha::ThresholdAlpha;
use serde_json::Value;
use sha2::{Digest, Sha256};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}
fn samples(bytes: &[u8], precision: &str) -> Vec<f64> {
    if precision.contains("U16") {
        bytes.as_chunks::<2>().0.iter().map(|b| f64::from(u16::from_le_bytes(*b)) / 65535.).collect()
    } else if precision.contains("FLOAT") {
        bytes.as_chunks::<4>().0.iter().map(|b| f64::from(f32::from_le_bytes(*b))).collect()
    } else {
        bytes.as_chunks::<8>().0.iter().map(|b| f64::from_le_bytes(*b)).collect()
    }
}
#[test]
#[ignore = "requires local unpublished Alpha Threshold originals"]
fn original_alpha_threshold_native_boundaries_and_disabled_states() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alpha-threshold-precision-evidence");
    let manifest = std::fs::read(root.join("SHA256SUMS.json")).unwrap();
    assert_eq!(hash(&manifest), "37eb14b73c0b873bc418781310b2bd384d873d9ebade486932bcdde0c4d27263");
    let hashes: Value = serde_json::from_slice(&manifest).unwrap();
    let read = |name: &str| {
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(hash(&bytes), hashes[name]);
        bytes
    };
    let proof: Value = serde_json::from_slice(&read("observations.json")).unwrap();
    let mut count = 0;
    let mut worst = 0_f64;
    for row in proof["records"].as_array().unwrap() {
        let precision = row["precision"].as_str().unwrap();
        let channels = if row["model"] == "GRAY" { 1 } else { 3 };
        let load = |role: &str| samples(&read(row[role]["layers"][0]["native"]["file"].as_str().unwrap()), precision);
        let input = load("reopened");
        let expected = load("baked");
        assert_eq!(input.len(), 48 * (channels + 1));
        assert_eq!(expected.len(), input.len());
        let operation = ThresholdAlpha { value: row["requested"]["value"].as_f64().unwrap() };
        for (source, reference) in input.chunks_exact(channels + 1).zip(expected.chunks_exact(channels + 1)) {
            let mut actual = vec![0.; channels + 1];
            if row["requested"]["enabled"].as_bool().unwrap() {
                operation.try_eval_pixel(source, channels, &mut actual).unwrap();
            } else {
                actual.copy_from_slice(source);
            }
            assert_eq!(actual[channels].to_bits(), reference[channels].to_bits(), "{} alpha", row["id"]);
            for index in 0..channels {
                assert_eq!(actual[index].to_bits(), source[index].to_bits());
                let delta = (actual[index] - reference[index]).abs();
                worst = worst.max(delta);
                assert!(delta <= 1. / 65535., "{} color delta {delta}", row["id"]);
            }
        }
        count += 1;
    }
    assert_eq!(count, 60);
    println!("60 Alpha Threshold native alpha exact/source colors untouched; worst color {}/255", worst * 255.);
}
