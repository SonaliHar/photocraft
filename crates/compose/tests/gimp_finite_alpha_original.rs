#![cfg(feature = "corpus")]
use photocraft_compose::gimp_composite::WorkingSpace;
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
fn read(root: &Path, value: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(value["file"].as_str().unwrap())).unwrap();
    let hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
    assert_eq!(hash, value["sha256"]);
    assert_eq!(bytes.len() as u64, value["bytes"]);
    bytes
}
#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn original_finite_alpha_normal_transparent_and_opaque() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("float-alpha-composition-evidence");
    let manifest = fs::read(root.join("batch-manifest.json")).unwrap();
    let hash: String = Sha256::digest(&manifest).iter().map(|v| format!("{v:02x}")).collect();
    assert_eq!(hash, "adecf73e948e7c62db8ecee561cd440d1eefcbd0f76ef57e5d4c5614619bac37");
    let bytes = fs::read(root.join("observations.json")).unwrap();
    let observed_hash: String = Sha256::digest(&bytes).iter().map(|v| format!("{v:02x}")).collect();
    assert_eq!(observed_hash, "78802d7c9ff63f535481f0ae46e38e8461ebbb33c6c23164dcf0ffae04f24997");
    let observations: Value = serde_json::from_slice(&bytes).unwrap();
    let mut count = 0;
    let mut nonunit = 0;
    for row in observations["records"].as_array().unwrap().iter().filter(|r| r.get("source_mode").is_some()) {
        assert_eq!(row["source_mode"], 28);
        assert_eq!(row["source_opacity"], 100.);
        let gray = row["source"]["native_format"].as_str().unwrap().starts_with("Y'");
        let channels = if gray { 2 } else { 4 };
        let profile = if gray { photocraft_cms::Builtin::SGray.profile() } else { photocraft_cms::Builtin::Srgb.profile() };
        let space = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        let source = read(&root, &row["source"]["native"]);
        let projection = read(&root, &row["projection"]["native"]);
        let width = row["image_size"][0].as_u64().unwrap() as usize;
        let source_width = row["source"]["size"][0].as_u64().unwrap() as usize;
        let source_height = row["source"]["size"][1].as_u64().unwrap() as usize;
        let offset = [row["source"]["offset"][0].as_i64().unwrap(), row["source"]["offset"][1].as_i64().unwrap()];
        let opaque = row["backdrop"] == "opaque";
        let backdrop = if opaque { if gray { [0.4, 0.4, 0.4, 1.] } else { [0.2, 0.4, 0.6, 1.] } } else { [0.; 4] };
        let mut maximum = 0.0_f64;
        for (index, bytes) in projection.chunks_exact(channels * 8).enumerate() {
            let x = (index % width) as i64 - offset[0];
            let y = (index / width) as i64 - offset[1];
            let mut pixel = [0.; 4];
            if x >= 0 && y >= 0 && x < source_width as i64 && y < source_height as i64 {
                let start = (y as usize * source_width + x as usize) * channels * 8;
                let raw = &source[start..start + channels * 8];
                for c in 0..channels - 1 {
                    pixel[c] = f64::from_le_bytes(raw[c * 8..c * 8 + 8].try_into().unwrap());
                }
                pixel[3] = f64::from_le_bytes(raw[(channels - 1) * 8..channels * 8].try_into().unwrap());
                if gray {
                    pixel[1] = pixel[0];
                    pixel[2] = pixel[0];
                }
            }
            nonunit += usize::from(!(0.0..=1.0).contains(&pixel[3]));
            let result = space.normal_finite_source(backdrop, pixel, 1., GimpOperator::Union).unwrap();
            for c in 0..channels {
                let reference = f64::from_le_bytes(bytes[c * 8..c * 8 + 8].try_into().unwrap());
                let actual = if c == channels - 1 { result[3] } else { result[c] };
                maximum = maximum.max((actual - reference).abs());
            }
        }
        println!("{} {} finite-alpha native {maximum}", row["id"], row["backdrop"]);
        assert!(maximum <= if opaque { 1e-6 } else { 0.0 });
        assert!(space.normal_finite_source([0.2, 0.4, 0.6, 0.5], [0.2, 0.4, 0.6, 1.03], 1., GimpOperator::Union).is_err());
        assert!(space.normal_finite_source(backdrop, [0.2, 0.4, 0.6, f64::NAN], 1., GimpOperator::Union).is_err());
        assert!(space.normal_finite_source(backdrop, [0.2, 0.4, 0.6, 1.03], 0.5, GimpOperator::Union).is_err());
        count += 1;
    }
    assert_eq!(count, 12);
    assert!(nonunit > 0);
}
