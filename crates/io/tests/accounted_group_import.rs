#![cfg(feature = "corpus")]
//! Actual XCF source admission is independent of synthetic accounted renderer fixtures.
use photocraft_doc::{LayerContent, item_identity::ItemRef};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

#[test]
#[ignore = "requires reviewed unpublished full XCF corpus"]
fn actual_authored_normal_groups_import_without_flattening_and_keep_current_bounds() {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("accounted-normal-group-evidence");
    let observation_bytes = fs::read(root.join("observations.json")).unwrap();
    let observation_hash: String = Sha256::digest(&observation_bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    let pins: Value = serde_json::from_str(include_str!("../../compose/tests/gimp_accounted_group_hashes.json")).unwrap();
    assert_eq!(observation_hash, pins["observations.json"].as_str().unwrap());
    let observations: Value = serde_json::from_slice(&observation_bytes).unwrap();
    let records = observations["records"].as_array().unwrap();
    assert_eq!(records.len(), 24);
    let mut failures = Vec::new();
    for record in records {
        let source = &record["source"];
        let bytes = fs::read(root.join(source["file"].as_str().unwrap())).unwrap();
        let digest: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(digest, source["sha256"].as_str().unwrap());
        match photocraft_io::import(source["file"].as_str().unwrap(), &bytes) {
            Err(error) => failures.push(format!("{}: {error}", record["id"])),
            Ok(imported) => {
                let document = &imported.document;
                assert_eq!(document.layers.len(), 2, "{}", record["id"]);
                let group = &document.layers[1];
                let LayerContent::Group(content) = &group.content else { panic!("group flattened: {}", record["id"]) };
                assert_eq!(content.children.len(), 2);
                assert_eq!(group.opacity.to_bits(), 0.625f32.to_bits());
                assert_eq!(group.visible, record["layers"][0]["visible"].as_bool().unwrap());
                let state = document.xcf_state.as_ref().unwrap();
                let item = state.items.iter().find(|item| item.item == ItemRef::Layer(group.id)).unwrap();
                assert_eq!(item.raster_bounds, Some(photocraft_doc::Rect::new(-3, -2, 13, 10)), "operative group extent: {}", record["id"]);
                for layer in std::iter::once(&document.layers[0]).chain(std::iter::once(group)).chain(content.children.iter()) {
                    let item = state.items.iter().find(|item| item.item == ItemRef::Layer(layer.id)).unwrap();
                    let policy = item.composition.as_ref().unwrap();
                    assert_eq!(policy.compatibility_epoch, "own-accounted-isolated-normal-group-v1", "{}", record["id"]);
                    assert_eq!((policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator), (0, 1, 1));
                    assert!(layer.import_compositing.is_none(), "current typed policy is authoritative");
                }
                for child in &content.children {
                    assert!(matches!(child.content, LayerContent::Raster(_)));
                    assert_eq!(state.items.iter().find(|item| item.item == ItemRef::Layer(child.id)).unwrap().parent, Some(ItemRef::Layer(group.id)));
                }
            }
        }
    }
    assert!(failures.is_empty(), "actual importer refusals:\n{}", failures.join("\n"));
}
