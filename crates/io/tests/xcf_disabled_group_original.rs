#![cfg(feature = "corpus")]
//! Disabled stock filters retain their schema and render current child pixels.
use photocraft_doc::{Rect, item_identity::ItemRef};
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

#[test]
#[ignore = "requires reviewed unpublished GIMP disabled-group evidence"]
fn disabled_gaussian_group_import_native_planes_and_render() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("group-disabled-evidence");
    let observations = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&observations), "06c29caff71c0da5c64ed8fbc59f7ff5de4a30d7a59d992d344b062f59779bcc");
    let packet: Value = serde_json::from_slice(&observations).unwrap();
    for record in packet["records"].as_array().unwrap() {
        let bytes = read(&root, &record["saved"]);
        let original_hash = hash(&bytes);
        let doc = photocraft_io::import("disabled.xcf", &bytes).unwrap().document;
        let before = doc.clone();
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!(state.filter_stacks.len(), 1);
        let node = &state.filter_stacks[0].nodes[0];
        assert!(!node.enabled);
        let policy = node.composition.as_ref().unwrap();
        assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (-1474592175, -1474592175, -1474592175));
        let captured = &record["reopened"];
        assert_eq!(captured["filter_visible"], false);
        for child in captured["children"].as_array().unwrap() {
            let layer = doc.walk().into_iter().map(|(_, _, layer)| layer).find(|layer| layer.name == child["name"].as_str().unwrap()).unwrap();
            let bounds = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).unwrap().raster_bounds.unwrap();
            let expected_bounds = Rect::from_xywh(
                child["offset"][0].as_i64().unwrap() as i32,
                child["offset"][1].as_i64().unwrap() as i32,
                child["size"][0].as_u64().unwrap() as u32,
                child["size"][1].as_u64().unwrap() as u32,
            );
            assert_eq!(bounds, expected_bounds);
            let native = child["reads"].as_array().unwrap().iter().find(|d| d["region"] == "buffer-extent" && d["requested_format"].is_null()).unwrap();
            let expected: Vec<u8> = read(&root, native)
                .as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| chunk.as_slice())
                .flat_map(|p| u32::from_le_bytes(p.try_into().unwrap()).to_ne_bytes())
                .collect();
            assert_eq!(layer.surface().unwrap().try_read_interleaved(bounds).unwrap(), expected);
        }
        let saved = photocraft_io::export(&doc, "disabled.pcraft", &Default::default()).unwrap();
        assert!(saved.warnings.is_empty());
        let reopened = photocraft_io::import("disabled.pcraft", &saved.bytes).unwrap().document;
        assert_eq!(*photocraft_io::named_for_file(&doc, "disabled.pcraft"), reopened);
        let descriptor = captured["projection"]["reads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["region"] == "buffer-extent" && d["requested_format"] == "R'G'B'A double")
            .unwrap();
        let expected = read(&root, descriptor);
        assert_eq!(expected.len(), doc.size.width as usize * doc.size.height as usize * 32);
        for candidate in [&doc, &reopened] {
            let actual = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            assert_eq!(actual.px.len() * 32, expected.len());
            let mut worst = 0.0_f64;
            for (pixel, raw) in actual.px.iter().zip(expected.as_chunks::<32>().0.iter().map(|chunk| chunk.as_slice())) {
                let alpha = f64::from_le_bytes(raw[24..32].try_into().unwrap());
                for (channel, value) in pixel.iter().enumerate() {
                    if channel < 3 && pixel[3] == 0.0 && alpha == 0.0 {
                        continue;
                    }
                    worst = worst.max((value - f64::from_le_bytes(raw[channel * 8..channel * 8 + 8].try_into().unwrap())).abs());
                }
            }
            assert!(worst < 1e-6, "{}: {worst}", record["phase"]);
        }
        assert_eq!(doc, before);
        assert_eq!(hash(&bytes), original_hash);
        assert_eq!(hash(&fs::read(root.join(record["saved"]["file"].as_str().unwrap())).unwrap()), original_hash);
    }
}
