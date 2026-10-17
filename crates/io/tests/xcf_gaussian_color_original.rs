#![cfg(feature = "corpus")]
//! Stock native/profile/alpha/HDR witnesses through the actual import and save path.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn read(root: &Path, descriptor: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), descriptor["sha256"].as_str().unwrap());
    bytes
}
fn native_descriptor(drawable: &Value) -> &Value {
    drawable["reads"].as_array().unwrap().iter().find(|r| r["region"] == "buffer-extent" && r["requested_format"].is_null()).unwrap()
}
#[test]
#[ignore = "requires reviewed unpublished literal Gaussian color/profile evidence"]
fn gaussian_color_import_native_source_bits_and_independent_evaluation() {
    color_import_native_source_bits_and_independent_evaluation(
        "gaussian-raster-color-evidence",
        "b5cc9fa07219b68436dc8821dc7ff902aef835a31e751e96eb36d706b50eee5a",
    );
}
#[test]
#[ignore = "requires reviewed unpublished literal Median color/profile evidence"]
fn median_color_import_native_source_bits_and_independent_evaluation() {
    color_import_native_source_bits_and_independent_evaluation(
        "median-raster-color-evidence",
        "f5f9965d5f5ac36dfd0d015701f24efa64776a7fcc2183b1acbdb03a0412d1b1",
    );
}
#[test]
#[ignore = "requires reviewed unpublished literal Gray Median evidence"]
fn median_gray_import_native_source_bits_and_independent_evaluation() {
    color_pipeline("median-gray-color-evidence", "2069220e399ef00cdc12bb57cb4ec574bdc59f47f42abe0d5cb78e217a1b2012", 4, true);
}
fn color_import_native_source_bits_and_independent_evaluation(subtree: &str, observations_hash: &str) {
    color_pipeline(subtree, observations_hash, 10, false);
}
fn color_pipeline(subtree: &str, observations_hash: &str, count: usize, gray: bool) {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join(subtree);
    let bytes = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&bytes), observations_hash);
    let records: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(records["errors"].as_array().unwrap().is_empty());
    assert_eq!(records["records"].as_array().unwrap().len(), count);
    for record in records["records"].as_array().unwrap() {
        let bytes = read(&root, &record["source"]);
        let before_hash = hash(&bytes);
        let doc = photocraft_io::import("literal-gaussian.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        assert_eq!(doc.layers.len(), 1);
        let source = &record["reopened"];
        let native = read(&root, native_descriptor(&source["layer"]));
        let raw: Vec<u8> = if doc.depth == photocraft_doc::SampleType::F64 {
            native.as_chunks::<8>().0.iter().map(|chunk| chunk.as_slice()).flat_map(|v| u64::from_le_bytes(v.try_into().unwrap()).to_ne_bytes()).collect()
        } else {
            native.as_chunks::<4>().0.iter().map(|chunk| chunk.as_slice()).flat_map(|v| u32::from_le_bytes(v.try_into().unwrap()).to_ne_bytes()).collect()
        };
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), raw);
        let expected_profile = if source["profiles"]["assigned"].is_null() { None } else { Some(read(&root, &source["profiles"]["assigned"])) };
        assert_eq!(doc.source_color_context.as_ref().unwrap().original_icc.as_deref(), expected_profile.as_ref());
        let saved = photocraft_io::export(&doc, "literal-gaussian.pcraft", &Default::default()).unwrap();
        assert!(saved.warnings.is_empty());
        let reopened = photocraft_io::import("literal-gaussian.pcraft", &saved.bytes).unwrap().document;
        assert_eq!(*photocraft_io::named_for_file(&doc, "literal-gaussian.pcraft"), reopened);
        let reference = if gray {
            record["merged_layer"]["reads"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["region"] == "buffer-extent" && r["requested_format"] == "R'G'B'A double")
                .unwrap()
        } else {
            native_descriptor(&record["merged_layer"])
        };
        let native = read(&root, reference);
        let expected: Vec<f64> = if gray || doc.depth == photocraft_doc::SampleType::F64 {
            native.as_chunks::<8>().0.iter().map(|chunk| chunk.as_slice()).map(|v| f64::from_le_bytes(v.try_into().unwrap())).collect()
        } else {
            native.as_chunks::<4>().0.iter().map(|chunk| chunk.as_slice()).map(|v| f64::from(f32::from_le_bytes(v.try_into().unwrap()))).collect()
        };
        assert_eq!(expected.len(), 16 * 12 * 4);
        for candidate in [&doc, &reopened] {
            let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            let mut worst = 0.0_f64;
            for (pixel, expected) in rendered.px.iter().zip(expected.as_chunks::<4>().0.iter().map(|chunk| chunk.as_slice())) {
                for (channel, value) in pixel.iter().enumerate() {
                    if channel < 3 && pixel[3] == 0.0 && expected[3] == 0.0 {
                        continue;
                    }
                    assert!(value.is_finite());
                    worst = worst.max((value - expected[channel]).abs());
                }
            }
            assert!(worst <= 2.0 / 255.0, "{}: {}/255", record["id"], worst * 255.0);
        }
        assert_eq!(doc, before);
        assert_eq!(hash(&bytes), before_hash);
        assert_eq!(hash(&fs::read(root.join(record["source"]["file"].as_str().unwrap())).unwrap()), before_hash);
    }
}
