#![cfg(feature = "corpus")]
//! Independent floating sources through the actual import/native/appearance path.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn checked(root: &std::path::Path, file: &str, expected: &str) -> Vec<u8> {
    let raw = fs::read(root.join(file)).unwrap();
    let hash: String = Sha256::digest(&raw).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, expected, "{file}");
    raw
}

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn floating_sources_import_reopen_and_match_native_projection() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("floating-anchor-evidence");
    let packet: Value =
        serde_json::from_slice(&checked(&root, "observations.json", "e1893a1601c94f745f9f445db5d865a90ab1f2f0cfae7fbf0cee8f2696e7f523")).unwrap();
    let mut count = 0;
    for record in packet["records"].as_array().unwrap() {
        let id = record["id"].as_str().unwrap();
        let input = checked(&root, record["input"].as_str().unwrap(), record["input_sha256"].as_str().unwrap());
        let doc = photocraft_io::import("floating.xcf", &input).unwrap().document;
        let before = doc.clone();
        let attachment = doc.xcf_state.as_ref().unwrap().floating.as_ref().unwrap();
        assert_ne!(photocraft_doc::item_identity::ItemRef::Layer(attachment.layer), attachment.target);
        for item in record["source_reopened"]["layers"].as_array().unwrap() {
            let layer = doc.layers.iter().find(|layer| layer.name == item["name"].as_str().unwrap()).unwrap();
            let raw = checked(&root, item["native"]["file"].as_str().unwrap(), item["native"]["sha256"].as_str().unwrap());
            let expected: Vec<u8> = match doc.depth {
                photocraft_doc::SampleType::U8 => raw,
                photocraft_doc::SampleType::U16 => {
                    raw.as_chunks::<2>().0.iter().map(|chunk| chunk.as_slice()).flat_map(|p| u16::from_le_bytes(p.try_into().unwrap()).to_ne_bytes()).collect()
                }
                photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F32 => {
                    raw.as_chunks::<4>().0.iter().map(|chunk| chunk.as_slice()).flat_map(|p| u32::from_le_bytes(p.try_into().unwrap()).to_ne_bytes()).collect()
                }
                photocraft_doc::SampleType::F64 => {
                    raw.as_chunks::<8>().0.iter().map(|chunk| chunk.as_slice()).flat_map(|p| u64::from_le_bytes(p.try_into().unwrap()).to_ne_bytes()).collect()
                }
            };
            let bounds = photocraft_doc::Rect::from_xywh(
                item["offset"][0].as_i64().unwrap() as i32,
                item["offset"][1].as_i64().unwrap() as i32,
                item["size"][0].as_u64().unwrap() as u32,
                item["size"][1].as_u64().unwrap() as u32,
            );
            assert_eq!(layer.surface().unwrap().to_interleaved(bounds), expected, "{id}");
        }
        let native = photocraft_io::export(&doc, "floating.pcraft", &Default::default()).unwrap();
        assert!(native.warnings.is_empty());
        let reopened = photocraft_io::import("floating.pcraft", &native.bytes).unwrap().document;
        assert_eq!(*photocraft_io::named_for_file(&doc, "floating.pcraft"), reopened);
        let descriptor = &record["source_reopened"]["projection"];
        let reference = checked(&root, descriptor["file"].as_str().unwrap(), descriptor["sha256"].as_str().unwrap());
        let tolerance = match doc.depth {
            photocraft_doc::SampleType::U8 => 0.501 / 255.0 + 1e-6,
            photocraft_doc::SampleType::U16 => 0.501 / 65535.0 + 1e-6,
            _ => 1e-6,
        };
        for source in [&doc, &reopened] {
            let actual = photocraft_compose::gimp_render::try_render(source, source.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(actual.px.len() * 32, reference.len());
            for (pixel, bytes) in actual.px.iter().zip(reference.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())) {
                let alpha = f64::from_le_bytes(bytes[24..32].try_into().unwrap());
                for (channel, value) in pixel.iter().enumerate() {
                    if channel < 3 && alpha == 0.0 && pixel[3] == 0.0 {
                        continue;
                    }
                    let expected = f64::from_le_bytes(bytes[channel * 8..channel * 8 + 8].try_into().unwrap());
                    assert!((value - expected).abs() <= tolerance, "{id}/{channel}: {value} vs {expected}");
                }
            }
        }
        assert_eq!(doc, before);
        count += 1;
    }
    assert_eq!(count, 10);
}
