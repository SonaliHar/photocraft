#![cfg(feature = "corpus")]
//! Box Blur goes through ordinary IO; references include radius-expanded bounds.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn read(root: &Path, descriptor: &Value) -> Vec<u8> {
    let bytes = fs::read(root.join(descriptor["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), descriptor["sha256"]);
    bytes
}
fn native(drawable: &Value) -> &Value {
    drawable["reads"].as_array().unwrap().iter().find(|r| r["region"] == "buffer-extent" && r["requested_format"].is_null()).unwrap()
}
fn reopen(doc: &photocraft_doc::Document) -> photocraft_doc::Document {
    let saved = photocraft_io::export(doc, "box.pcraft", &Default::default()).unwrap();
    assert!(saved.warnings.is_empty());
    let reopened = photocraft_io::import("box.pcraft", &saved.bytes).unwrap().document;
    assert_eq!(&reopened, photocraft_io::named_for_file(doc, "box.pcraft").as_ref());
    reopened
}
#[test]
#[ignore = "requires reviewed unpublished horizontal Motion Blur source/edit witnesses"]
fn motion_import_native_and_radius_aligned_projection() {
    original_pipeline("gegl-motion-blur-linear", include_str!("../../compose/tests/gimp_motion_hashes.json"), [5, 6]);
}
#[test]
#[ignore = "requires reviewed unpublished square Pixelize source/edit witnesses"]
fn pixelize_import_native_and_independent_projection() {
    original_pipeline("gegl-pixelize", include_str!("../../compose/tests/gimp_pixelize_hashes.json"), [0, 0]);
}
fn original_pipeline(operation: &str, manifest: &str, margins: [u32; 2]) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: Value = serde_json::from_str(manifest).unwrap();
    let name = format!("operations/{operation}/operation-{operation}");
    for (suffix, ref_suffix, margin) in [("xcf", "ref.png", margins[0]), ("edit.xcf", "edit.ref.png", margins[1])] {
        let path = format!("{name}.{suffix}");
        let bytes = fs::read(root.join(&path)).unwrap();
        assert_eq!(hash(&bytes), hashes[&path]);
        let doc = photocraft_io::import("motion.xcf", &bytes).unwrap().document;
        if operation == "gegl-pixelize" {
            let state = doc.xcf_state.as_ref().unwrap();
            let color = state.opaque.iter().find(|v| v.namespace == "gimp/xcf/source-filter-color" && v.identity == "0/background").unwrap();
            let mut expected = Vec::new();
            for part in [b"R'G'B'A float".as_slice(), &[0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63], &[]] {
                expected.extend_from_slice(&(part.len() as u32).to_be_bytes());
                expected.extend_from_slice(part);
            }
            assert_eq!(color.bytes.as_slice(), expected.as_slice());
            assert_eq!(color.source_version, 22);
            assert!(!color.rendering_required);
        }
        let raw_path = format!("{name}.source.0.native.bin");
        let raw = fs::read(root.join(&raw_path)).unwrap();
        assert_eq!(hash(&raw), hashes[&raw_path]);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), raw);
        let ref_path = format!("{name}.{ref_suffix}");
        let png = fs::read(root.join(&ref_path)).unwrap();
        assert_eq!(hash(&png), hashes[&ref_path]);
        let reference = photocraft_codecs::decode(&png).unwrap();
        assert_eq!((reference.width(), reference.height()), (32 + margin * 2, 32));
        for candidate in [&doc, &reopen(&doc)] {
            let output = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            for (index, pixel) in output.px.iter().enumerate() {
                for (channel, value) in pixel.iter().enumerate() {
                    let expected = f64::from(reference.get(index as u32 % 32 + margin, index as u32 / 32, channel));
                    assert!((value - expected).abs() <= 2.0 / 255.0, "{suffix}/{index}/{channel}");
                }
            }
        }
        assert_eq!(fs::read(root.join(path)).unwrap(), bytes);
    }
}
#[test]
#[ignore = "requires reviewed unpublished Box Blur original/color source/edit witnesses"]
fn box_import_native_profile_and_radius_aligned_projection() {
    let base = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_box_hashes.json")).unwrap();
    let name = "operations/gegl-box-blur/operation-gegl-box-blur";
    for (suffix, ref_suffix, radius) in [("xcf", "ref.png", 4), ("edit.xcf", "edit.ref.png", 5)] {
        let path = format!("{name}.{suffix}");
        let bytes = fs::read(base.join(&path)).unwrap();
        assert_eq!(hash(&bytes), hashes[&path]);
        let doc = photocraft_io::import("box.xcf", &bytes).unwrap().document;
        let raw_path = format!("{name}.source.0.native.bin");
        let raw = fs::read(base.join(&raw_path)).unwrap();
        assert_eq!(hash(&raw), hashes[&raw_path]);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), raw);
        let ref_path = format!("{name}.{ref_suffix}");
        let png = fs::read(base.join(&ref_path)).unwrap();
        assert_eq!(hash(&png), hashes[&ref_path]);
        let reference = photocraft_codecs::decode(&png).unwrap();
        for candidate in [&doc, &reopen(&doc)] {
            let output = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            for (index, pixel) in output.px.iter().enumerate() {
                for (channel, value) in pixel.iter().enumerate() {
                    let expected = f64::from(reference.get(index as u32 % 32 + radius, index as u32 / 32 + radius, channel));
                    assert!((value - expected).abs() <= 2.0 / 255.0);
                }
            }
        }
        assert_eq!(fs::read(base.join(path)).unwrap(), bytes);
    }
    let root = base.join("box-raster-color-evidence");
    let observations = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&observations), "9d93881469841ad87128d6bc7aebc7e2ed2c69812dd82d92eb36966752fb15b6");
    let packet: Value = serde_json::from_slice(&observations).unwrap();
    assert!(packet["errors"].as_array().unwrap().is_empty());
    assert_eq!(packet["records"].as_array().unwrap().len(), 10);
    for record in packet["records"].as_array().unwrap() {
        let bytes = read(&root, &record["source"]);
        let doc = photocraft_io::import("box.xcf", &bytes).unwrap().document;
        let source = &record["reopened"];
        let raw = read(&root, native(&source["layer"]));
        let double = doc.depth == photocraft_doc::SampleType::F64;
        let raw: Vec<u8> = if double {
            raw.as_chunks::<8>().0.iter().map(|chunk| chunk.as_slice()).flat_map(|v| u64::from_le_bytes(v.try_into().unwrap()).to_ne_bytes()).collect()
        } else {
            raw.as_chunks::<4>().0.iter().map(|chunk| chunk.as_slice()).flat_map(|v| u32::from_le_bytes(v.try_into().unwrap()).to_ne_bytes()).collect()
        };
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), raw);
        let profile = if source["profiles"]["assigned"].is_null() { None } else { Some(read(&root, &source["profiles"]["assigned"])) };
        assert_eq!(doc.source_color_context.as_ref().unwrap().original_icc.as_deref(), profile.as_ref());
        let merged = &record["merged_layer"];
        let descriptor = native(merged);
        let raw = read(&root, descriptor);
        let expected: Vec<f64> = if double {
            raw.as_chunks::<8>().0.iter().map(|chunk| chunk.as_slice()).map(|v| f64::from_le_bytes(v.try_into().unwrap())).collect()
        } else {
            raw.as_chunks::<4>().0.iter().map(|chunk| chunk.as_slice()).map(|v| f64::from(f32::from_le_bytes(v.try_into().unwrap()))).collect()
        };
        let width = descriptor["rect"][2].as_u64().unwrap() as usize;
        let radius = source["config"]["radius"].as_u64().unwrap() as usize;
        for candidate in [&doc, &reopen(&doc)] {
            let output = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            for (index, pixel) in output.px.iter().enumerate() {
                let position = ((index / 16 + radius) * width + index % 16 + radius) * 4;
                for (channel, value) in pixel.iter().enumerate() {
                    assert!(value.is_finite());
                    assert!((value - expected[position + channel]).abs() <= 2.0 / 255.0, "{}", record["id"]);
                }
            }
        }
        assert_eq!(read(&root, &record["source"]), bytes);
    }
}
#[test]
#[ignore = "requires reviewed unpublished quantized Median Blur original source/edit witnesses"]
fn median_import_native_and_exact_eight_bit_reference() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let hashes: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_median_hashes.json")).unwrap();
    let name = "operations/gegl-median-blur/operation-gegl-median-blur";
    for (suffix, ref_suffix) in [("xcf", "ref.png"), ("edit.xcf", "edit.ref.png")] {
        let path = format!("{name}.{suffix}");
        let bytes = fs::read(root.join(&path)).unwrap();
        assert_eq!(hash(&bytes), hashes[&path]);
        let doc = photocraft_io::import("median.xcf", &bytes).unwrap().document;
        let raw_path = format!("{name}.source.0.native.bin");
        let raw = fs::read(root.join(&raw_path)).unwrap();
        assert_eq!(hash(&raw), hashes[&raw_path]);
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), raw);
        let ref_path = format!("{name}.{ref_suffix}");
        let png = fs::read(root.join(&ref_path)).unwrap();
        assert_eq!(hash(&png), hashes[&ref_path]);
        let reference = photocraft_codecs::decode(&png).unwrap();
        for candidate in [&doc, &reopen(&doc)] {
            let output = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            for (index, pixel) in output.px.iter().enumerate() {
                for (channel, value) in pixel.iter().enumerate() {
                    let actual = (value * 255.0).round().clamp(0.0, 255.0) as u8;
                    let expected = (reference.get(index as u32 % 32, index as u32 / 32, channel) * 255.0).round() as u8;
                    assert_eq!(actual, expected, "{suffix}/{index}/{channel}");
                }
            }
        }
        assert_eq!(fs::read(root.join(path)).unwrap(), bytes);
    }
}
