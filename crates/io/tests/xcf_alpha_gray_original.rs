#![cfg(feature = "corpus")]
//! Real Gray alpha mode imports retain exact source bytes and native semantic state.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn read(root: &Path, blob: &Value) -> Vec<u8> {
    let raw = fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&raw), blob["sha256"]);
    assert_eq!(raw.len(), blob["bytes"].as_u64().unwrap() as usize);
    raw
}
#[test]
#[ignore = "requires reviewed unpublished independent Gray alpha source/edit witnesses"]
fn gray_alpha_import_retains_source_native_state_and_independent_projection() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("alpha-gray-evidence");
    let observations = fs::read(root.join("observations.json")).unwrap();
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_alpha_gray_hashes.json")).unwrap();
    assert_eq!(hash(&observations), hashes["observations.json"]);
    let observations: Value = serde_json::from_slice(&observations).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap() {
        let source_path = root.join(row["source_file"].as_str().unwrap());
        let bytes = fs::read(&source_path).unwrap();
        assert_eq!(hash(&bytes), row["source_sha256"]);
        let doc = photocraft_io::import("gray.xcf", &bytes).unwrap().document;
        assert_eq!(doc.mode, photocraft_doc::ColorMode::Grayscale);
        assert_eq!(doc.depth, photocraft_doc::SampleType::F32);
        assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
        for (layer, original) in doc.layers.iter().rev().zip(row["layers"].as_array().unwrap()) {
            let raw = read(&root, &original["native"]);
            let raw: Vec<u8> =
                raw.as_chunks::<4>().0.iter().map(|chunk| chunk.as_slice()).flat_map(|v| u32::from_le_bytes(v.try_into().unwrap()).to_ne_bytes()).collect();
            let bounds = photocraft_doc::Rect::from_xywh(
                original["offset"][0].as_i64().unwrap() as i32,
                original["offset"][1].as_i64().unwrap() as i32,
                original["size"][0].as_u64().unwrap() as u32,
                original["size"][1].as_u64().unwrap() as u32,
            );
            assert_eq!(layer.surface().unwrap().try_read_interleaved(bounds).unwrap(), raw);
            let state = doc.xcf_state.as_ref().unwrap();
            let policy =
                state.items.iter().find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(layer.id)).unwrap().composition.as_ref().unwrap();
            assert_eq!(policy.source_mode, original["mode"].as_i64().unwrap() as i32);
        }
        let before = doc.clone();
        let saved = photocraft_io::export(&doc, "gray.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("gray.pcraft", &saved.bytes).unwrap().document;
        assert_eq!(*photocraft_io::named_for_file(&doc, "gray.pcraft"), reopened);
        let reference = photocraft_codecs::decode(&read(&root, &row["reference"])).unwrap();
        let native = read(&root, &row["projection"]["nonlinear_double"]);
        for candidate in [&doc, &reopened] {
            let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            for (index, (pixel, original)) in rendered.px.iter().zip(native.as_chunks::<16>().0.iter().map(|chunk| chunk.as_slice())).enumerate() {
                let y = f64::from_le_bytes(original[..8].try_into().unwrap());
                let alpha = f64::from_le_bytes(original[8..].try_into().unwrap());
                assert!((pixel[3] - alpha).abs() <= 1.0 / 65535.0);
                for color in pixel.iter().take(3) {
                    // Alpha-zero hidden projection color is explicitly distinct in GIMP;
                    // its immutable saved source samples remain checked exactly above.
                    if alpha != 0.0 {
                        assert!((color - y).abs() <= 1.0 / 65535.0);
                    }
                    assert!((color * pixel[3] - y * alpha).abs() <= 1.0 / 65535.0);
                }
                let x = index as u32 % 16;
                let y = index as u32 / 16;
                assert!((pixel[0] - f64::from(reference.get(x, y, 0))).abs() <= 1.0 / 65535.0);
                assert!((pixel[3] - f64::from(reference.get(x, y, 1))).abs() <= 1.0 / 65535.0);
            }
        }
        assert_eq!(doc, before);
        assert_eq!(fs::read(source_path).unwrap(), bytes);
        count += 1;
    }
    assert_eq!(count, 8);
}
