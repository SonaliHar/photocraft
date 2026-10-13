#![cfg(feature = "corpus")]
//! Independent public saved opaque basis distinguishes float legacy math.
use photocraft_compose::{gimp_blend, gimp_composite::WorkingSpace};
use photocraft_doc::{
    SampleEncoding,
    import_compositing::{GimpOperator, GimpSpace},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn checked(root: &Path, blob: &Value) -> Vec<u8> {
    let raw = fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
    let hash: String = Sha256::digest(&raw).iter().map(|v| format!("{v:02x}")).collect();
    assert_eq!(hash, blob["sha256"].as_str().unwrap());
    assert_eq!(raw.len(), blob["bytes"].as_u64().unwrap() as usize);
    raw
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn independent_saved_legacy_opaque_basis_matches_checked_float_products() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("legacy-basis-evidence");
    let hashes: Value = serde_json::from_str(include_str!("gimp_legacy_basis_hashes.json")).unwrap();
    let profile = WorkingSpace::new(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
    let mut worst = 0.0_f64;
    for folder in ["", "fresh/"] {
        let bytes = fs::read(root.join(format!("{folder}observations.json"))).unwrap();
        let hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(hash, hashes[format!("{folder}observations.json")].as_str().unwrap());
        let data: Value = serde_json::from_slice(&bytes).unwrap();
        for row in data["records"].as_array().unwrap() {
            if row["unsupported"] == true {
                assert_eq!(row["requested"], 22);
                if folder.is_empty() {
                    assert_eq!(row["public_set_mode_result"], false);
                }
                continue;
            }
            let mode = row["actual"].as_i64().unwrap() as i32;
            if row["requested"] == 5 {
                assert_eq!(mode, 19);
            }
            let source = fs::read(root.join(row["source_file"].as_str().unwrap())).unwrap();
            let hash: String = Sha256::digest(&source).iter().map(|v| format!("{v:02x}")).collect();
            assert_eq!(hash, row["source_sha256"]);
            let dir = root.join(folder);
            let back = checked(&dir, &row["layers"][1]["native"]);
            let upper = checked(&dir, &row["layers"][0]["native"]);
            let projection = checked(&dir, &row["projection"]);
            assert_eq!(back.len(), 64 * 16);
            assert_eq!(upper.len(), 64 * 16);
            assert_eq!(projection.len(), 64 * 32);
            for i in 0..64 {
                let read = |raw: &[u8]| std::array::from_fn(|c| f64::from(f32::from_le_bytes(raw[i * 16 + c * 4..i * 16 + c * 4 + 4].try_into().unwrap())));
                let out = profile.separable(&profile, read(&back), read(&upper), 1.0, GimpOperator::ClipToBackdrop, gimp_blend::mode(mode).unwrap()).unwrap();
                for c in 0..4 {
                    let expected = f64::from_le_bytes(projection[i * 32 + c * 8..i * 32 + c * 8 + 8].try_into().unwrap());
                    let error = (out[c] - expected).abs();
                    worst = worst.max(error);
                    assert!(error <= 1.0 / 65535.0, "mode{mode}pixel{i}channel{c}: {error}");
                }
            }
        }
    }
    println!("Legacy independent basis max {worst}");
}
