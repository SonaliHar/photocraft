#![cfg(feature = "corpus")]
//! Noncommuting point stacks from independently reopened GIMP projects.
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
    let bytes = fs::read(root.join(blob["file"].as_str().unwrap())).unwrap();
    assert_eq!(hash(&bytes), blob["sha256"]);
    assert_eq!(bytes.len(), blob["bytes"].as_u64().unwrap() as usize);
    bytes
}
#[test]
#[ignore = "requires reviewed unpublished GIMP filter stack edit witnesses"]
fn saved_stack_order_enable_remove_opacity_import_and_native_projection() {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("filter-stack-edits-evidence");
    let raw = fs::read(root.join("observations.json")).unwrap();
    assert_eq!(hash(&raw), "104ba73e04806be7547efae7f6362b0ecad66dbb02e7973eda26d499b0d63386");
    let observations: Value = serde_json::from_slice(&raw).unwrap();
    let raw = fs::read(root.join("fresh-verification.json")).unwrap();
    assert_eq!(hash(&raw), "71b69b90e4ae17d21a8879cd3017f9cf7ae3770a4ef1c179c174baeee8275ec6");
    let verification: Value = serde_json::from_slice(&raw).unwrap();
    let mut count = 0;
    for row in observations["records"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        assert!(matches!(id, "stack.source" | "stack.disable" | "stack.remove" | "stack.reverse" | "stack.opacity-half"));
        let path = root.join(row["file"].as_str().unwrap());
        let bytes = fs::read(&path).unwrap();
        assert_eq!(hash(&bytes), row["sha256"]);
        let fresh = verification["checks"].as_array().unwrap().iter().find(|check| check["id"] == id).unwrap();
        assert_eq!(fresh["source_sha256"], row["sha256"]);
        assert_eq!(fresh["native_exact"], true);
        assert_eq!(fresh["projection_exact"], true);
        assert_eq!(fresh["filters_exact"], true);
        let doc = photocraft_io::import("stack.xcf", &bytes).unwrap().document;
        let original = &fresh["fresh"]["layers"][0];
        assert_eq!(doc.layers[0].surface().unwrap().try_read_interleaved(doc.bounds()).unwrap(), read(&root, &original["native"]));
        let stack = &doc.xcf_state.as_ref().unwrap().filter_stacks[0];
        assert_eq!(stack.nodes.len(), original["filters"].as_array().unwrap().len());
        for (node, expected) in stack.nodes.iter().zip(original["filters"].as_array().unwrap()) {
            assert_eq!(node.operation.name, expected["operation"]);
            assert_eq!(node.operation.schema_epoch, "xcf-22-stock-point-stack-edits-v1");
            assert_eq!(node.enabled, expected["visible"].as_bool().unwrap());
            assert_eq!(node.opacity_f64_bits, expected["opacity"].as_f64().unwrap().to_bits());
            let policy = node.composition.as_ref().unwrap();
            assert_eq!(
                (policy.source_mode, policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator),
                (62, -1178762671, -1178762671, -1178762671)
            );
            assert!(policy.resolved.is_none());
        }
        let before = doc.clone();
        let saved = photocraft_io::export(&doc, "stack.pcraft", &Default::default()).unwrap();
        let reopened = photocraft_io::import("stack.pcraft", &saved.bytes).unwrap().document;
        assert_eq!(*photocraft_io::named_for_file(&doc, "stack.pcraft"), reopened);
        let expected = read(&root, &fresh["fresh"]["projection"]);
        assert_eq!(expected.len(), 8 * 6 * 4 * 8);
        for candidate in [&doc, &reopened] {
            let rendered = photocraft_compose::gimp_render::try_render(candidate, candidate.bounds(), &mut |_| Ok(())).unwrap();
            for (actual, expected) in rendered.px.iter().flatten().zip(expected.as_chunks::<8>().0.iter().map(|chunk| chunk.as_slice())) {
                let expected = f64::from_le_bytes(expected.try_into().unwrap());
                assert!((actual.clamp(0.0, 1.0) - expected).abs() <= 2.0 / 255.0, "{id} {actual} {expected}");
            }
        }
        assert_eq!(doc, before);
        assert_eq!(fs::read(path).unwrap(), bytes);
        count += 1;
    }
    assert_eq!(count, 5);
}
